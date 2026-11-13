#define PY_SSIZE_T_CLEAN
#include <Python.h>
#include "window_owned_child_darwin.h"
#include <limits.h>
#include <string.h>

#if PY_VERSION_HEX < 0x03090000
#error "Child candidate has no supported source lifecycle below CPython 3.9"
#endif

/* 3.9-3.12 depend on the private declaration supplied by Python.h. Exact
 * installed header/export/ABI compatibility remains UNQUALIFIED. No copied
 * declaration or dynamically guessed symbol is used. */
#if PY_VERSION_HEX >= 0x030D0000
#define WOC_PY_FINALIZING() Py_IsFinalizing()
#else
#define WOC_PY_FINALIZING() _Py_IsFinalizing()
#endif

#ifdef Py_GIL_DISABLED
#error "This unqualified candidate requires a separately reviewed GIL lifecycle"
#endif

_Static_assert(WOC_NATIVE_QUALIFIED == 0, "Native qualification must remain disabled");
_Static_assert(sizeof(long long) >= sizeof(int64_t), "Public integer conversion required");

typedef struct {
    PyObject_HEAD
} Bridge;

typedef struct {
    PyObject_HEAD
    Bridge *bridge;
    woc_owner *native;
    int published;
    int busy;
    int stopped;
    int signal_failed;
    int latch_started;
    int latched;
    int reap_started;
    int reaped;
    int release_requested;
} Owner;

static PyTypeObject BridgeType;
static PyTypeObject OwnerType;
static Owner *retained_owner;
static int module_busy;
static int module_initialization_started;
static PyInterpreterState *module_interpreter;

static int module_enter(void);

static int put(PyObject *mapping, const char *name, PyObject *value)
{
    if (value == NULL) {
        return -1;
    }
    int result = PyDict_SetItemString(mapping, name, value);
    Py_DECREF(value);
    return result;
}

static int put_reference(PyObject *mapping, const char *name, PyObject *value)
{
    return PyDict_SetItemString(mapping, name, value == NULL ? Py_None : value);
}

static PyObject *description_mapping(const woc_description *out)
{
    PyObject *mapping = PyDict_New();
    if (mapping == NULL) {
        return NULL;
    }
#define DESCRIPTION_UNSIGNED(name) \
    if (put(mapping, #name, PyLong_FromUnsignedLong(out->name)) < 0) { goto failed; }
#define DESCRIPTION_SIGNED(name) \
    if (put(mapping, #name, PyLong_FromLong(out->name)) < 0) { goto failed; }
    DESCRIPTION_UNSIGNED(bytes)
    DESCRIPTION_UNSIGNED(version)
    DESCRIPTION_SIGNED(error)
    DESCRIPTION_SIGNED(system_errno)
    DESCRIPTION_UNSIGNED(record_bytes)
    DESCRIPTION_UNSIGNED(pid_bytes)
    DESCRIPTION_UNSIGNED(platform)
    DESCRIPTION_UNSIGNED(enabled)
#undef DESCRIPTION_UNSIGNED
#undef DESCRIPTION_SIGNED
    return mapping;
failed:
    Py_DECREF(mapping);
    return NULL;
}

static PyObject *record_mapping(const woc_record *out)
{
    PyObject *mapping = PyDict_New();
    if (mapping == NULL) {
        return NULL;
    }
#define RECORD_UNSIGNED(name) \
    if (put(mapping, #name, PyLong_FromUnsignedLong(out->name)) < 0) { goto failed; }
#define RECORD_HEADER(name) \
    if (put(mapping, #name, PyLong_FromLong(out->name)) < 0) { goto failed; }
#define RECORD_SIGNED(name) \
    if (put(mapping, #name, PyLong_FromLongLong((long long)out->name)) < 0) { goto failed; }
    RECORD_UNSIGNED(bytes)
    RECORD_UNSIGNED(version)
    RECORD_HEADER(error)
    RECORD_HEADER(system_errno)
    RECORD_SIGNED(pid)
    RECORD_SIGNED(pgid)
    RECORD_SIGNED(uid)
    RECORD_SIGNED(ppid)
    RECORD_SIGNED(sid)
    RECORD_SIGNED(state)
    RECORD_SIGNED(exit_kind)
    RECORD_SIGNED(exit_status)
    RECORD_SIGNED(wait_uid_available)
    RECORD_SIGNED(uid_evidence)
    RECORD_SIGNED(sigchld_default)
    RECORD_SIGNED(no_cldwait)
    RECORD_SIGNED(raw_bytes)
    RECORD_SIGNED(pid_bytes)
    RECORD_SIGNED(capacity)
    RECORD_SIGNED(count)
    RECORD_SIGNED(member0)
    RECORD_SIGNED(member1)
    RECORD_SIGNED(no_signals)
    RECORD_SIGNED(reap_attempted)
    RECORD_SIGNED(reaped)
    RECORD_SIGNED(returncode)
    RECORD_SIGNED(signal_attempted)
#undef RECORD_UNSIGNED
#undef RECORD_HEADER
#undef RECORD_SIGNED
    return mapping;
failed:
    Py_DECREF(mapping);
    return NULL;
}

static PyObject *reply(const char *operation, int rc, int initialized,
                       PyObject *description, PyObject *record, Owner *owner)
{
    PyObject *mapping = PyDict_New();
    if (mapping == NULL) {
        return NULL;
    }
    if (put(mapping, "operation", PyUnicode_FromString(operation)) < 0 ||
        put(mapping, "rc", PyLong_FromLong(rc)) < 0 ||
        put(mapping, "initialized", PyBool_FromLong(initialized)) < 0 ||
        put_reference(mapping, "description", description) < 0 ||
        put_reference(mapping, "record", record) < 0 ||
        put_reference(mapping, "owner", (PyObject *)owner) < 0) {
        Py_DECREF(mapping);
        return NULL;
    }
    return mapping;
}

static int consistent(int rc, const woc_record *out)
{
    return rc >= WOC_OK && rc <= WOC_INCONCLUSIVE && rc == out->error &&
           out->system_errno >= 0 && (rc == WOC_SYSTEM || out->system_errno == 0);
}

static PyObject *record_reply(const char *operation, int rc, woc_record *out,
                              Owner *owner, int opening)
{
    int initialized = out->bytes == sizeof(*out) && out->version == WOC_VERSION;
    int signal_error = initialized && consistent(rc, out) &&
                       strcmp(operation, "signal") == 0 && rc == WOC_SYSTEM &&
                       out->system_errno > 0 && out->signal_attempted == 1;
    if (!initialized || !consistent(rc, out) || (rc != WOC_OK && !signal_error) ||
        (opening && owner->native == NULL) ||
        (strcmp(operation, "signal") == 0 && out->signal_attempted != 1)) {
        owner->stopped = 1;
    }
    if (signal_error) {
        owner->signal_failed = 1;
    }
    if (initialized && out->reaped == 1) {
        owner->reaped = 1;
    }
    PyObject *record = initialized ? record_mapping(out) : NULL;
    if (initialized && record == NULL) {
        owner->stopped = 1;
        return NULL;
    }
    PyObject *result = reply(operation, rc, initialized, NULL, record,
                             opening && owner->native != NULL ? owner : NULL);
    Py_XDECREF(record);
    if (result == NULL) {
        owner->stopped = 1;
        return NULL;
    }
    owner->busy = 0;
    if (opening) {
        owner->published = 1;
    }
    return result;
}

static int integer64(PyObject *value, int64_t minimum, int64_t maximum, int64_t *out)
{
    if (!PyLong_CheckExact(value)) {
        PyErr_SetString(PyExc_TypeError, "Exact integer required, not bool or an owner address");
        return 0;
    }
    long long number = PyLong_AsLongLong(value);
    if (number == -1 && PyErr_Occurred()) {
        return 0;
    }
    if (number < minimum || number > maximum) {
        PyErr_SetString(PyExc_ValueError, "Integer outside public argument range");
        return 0;
    }
    *out = (int64_t)number;
    return 1;
}

static Owner *checked_owner(Bridge *bridge, PyObject *value, int releasing)
{
    if (Py_TYPE(value) != &OwnerType) {
        PyErr_SetString(PyExc_TypeError, "Exact strong bridge owner required");
        return NULL;
    }
    Owner *owner = (Owner *)value;
    if (owner != retained_owner || owner->bridge != bridge || owner->native == NULL ||
        !owner->published || owner->busy || owner->release_requested ||
        (!releasing && (owner->stopped || owner->reap_started || owner->reaped))) {
        PyErr_SetString(PyExc_RuntimeError, "Foreign, unresolved, reaped or released owner");
        return NULL;
    }
    return owner;
}

static PyObject *bridge_describe(Bridge *bridge, PyObject *unused)
{
    (void)bridge;
    (void)unused;
    woc_description out;
    memset(&out, 0, sizeof(out));
    int rc = woc_describe(&out, sizeof(out), WOC_VERSION);
    int initialized = out.bytes == sizeof(out) && out.version == WOC_VERSION;
    PyObject *description = initialized ? description_mapping(&out) : NULL;
    if (initialized && description == NULL) {
        return NULL;
    }
    PyObject *result = reply("describe", rc, initialized, description, NULL, NULL);
    Py_XDECREF(description);
    return result;
}

static PyObject *bridge_open(Bridge *bridge, PyObject *arguments)
{
    PyObject *pid_value, *pgid_value, *uid_value;
    int64_t pid, pgid, uid;
    if (!PyArg_ParseTuple(arguments, "OOO:open", &pid_value, &pgid_value, &uid_value) ||
        !integer64(pid_value, 2, INT32_MAX, &pid) ||
        !integer64(pgid_value, 2, INT32_MAX, &pgid) ||
        !integer64(uid_value, 0, UINT32_MAX, &uid)) {
        return NULL;
    }
    if (pid != pgid || retained_owner != NULL || woc_child_registry_used()) {
        PyErr_SetString(PyExc_RuntimeError, "Retained attempt blocks open; no adoption/reconciliation API");
        return NULL;
    }
    Owner *owner = PyObject_New(Owner, &OwnerType);
    if (owner == NULL) {
        return NULL;
    }
    owner->bridge = bridge;
    Py_INCREF((PyObject *)bridge);
    owner->native = NULL;
    owner->published = 0;
    owner->busy = 1;
    owner->stopped = 0;
    owner->signal_failed = 0;
    owner->latch_started = 0;
    owner->latched = 0;
    owner->reap_started = 0;
    owner->reaped = 0;
    owner->release_requested = 0;
    retained_owner = owner;
    Py_INCREF((PyObject *)owner);
    woc_record out;
    memset(&out, 0, sizeof(out));
    int rc = woc_open(pid, pgid, uid, &owner->native, &out, sizeof(out), WOC_VERSION);
    PyObject *result = record_reply("open", rc, &out, owner, 1);
    Py_DECREF((PyObject *)owner);
    return result;
}

static PyObject *bridge_operation(Bridge *bridge, PyObject *arguments, const char *operation)
{
    PyObject *value, *first_value = NULL, *second_value = NULL;
    int64_t first = 0, second = 0;
    int binary = strcmp(operation, "signal") == 0 || strcmp(operation, "latch") == 0;
    if (binary) {
        if (!PyArg_ParseTuple(arguments, "OOO", &value, &first_value, &second_value) ||
            !integer64(first_value, 1, strcmp(operation, "signal") == 0 ? WOC_KILL : WOC_EXIT_CORE, &first) ||
            !integer64(second_value, 0, INT32_MAX, &second)) {
            return NULL;
        }
    } else if (strcmp(operation, "member") == 0) {
        if (!PyArg_ParseTuple(arguments, "OO", &value, &first_value) ||
            !integer64(first_value, 2, INT32_MAX, &first)) {
            return NULL;
        }
    } else if (!PyArg_ParseTuple(arguments, "O", &value)) {
        return NULL;
    }
    Owner *owner = checked_owner(bridge, value, 0);
    if (owner == NULL) {
        return NULL;
    }
    if ((owner->latch_started && strcmp(operation, "reap") != 0) ||
        (strcmp(operation, "signal") == 0 && owner->signal_failed) ||
        (strcmp(operation, "reap") == 0 && !owner->latched)) {
        PyErr_SetString(PyExc_RuntimeError, "No repeat latch/reap or post-latch/error signal");
        return NULL;
    }
    owner->busy = 1;
    woc_record out;
    memset(&out, 0, sizeof(out));
    int rc;
    if (strcmp(operation, "observe") == 0) {
        rc = woc_observe(owner->native, &out, sizeof(out), WOC_VERSION);
    } else if (strcmp(operation, "members") == 0) {
        rc = woc_members(owner->native, &out, sizeof(out), WOC_VERSION);
    } else if (strcmp(operation, "member") == 0) {
        rc = woc_member(owner->native, first, &out, sizeof(out), WOC_VERSION);
    } else if (strcmp(operation, "signal") == 0) {
        rc = woc_signal(owner->native, (int)first, second, &out, sizeof(out), WOC_VERSION);
    } else if (strcmp(operation, "latch") == 0) {
        owner->latch_started = 1;
        rc = woc_latch(owner->native, first, second, &out, sizeof(out), WOC_VERSION);
        owner->latched = out.bytes == sizeof(out) && out.version == WOC_VERSION &&
                         rc == WOC_OK && consistent(rc, &out) &&
                         out.no_signals == 1;
    } else {
        owner->reap_started = 1;
        rc = woc_reap(owner->native, &out, sizeof(out), WOC_VERSION);
    }
    return record_reply(operation, rc, &out, owner, 0);
}

#define OWNER_METHOD(name) \
    static PyObject *bridge_##name(Bridge *bridge, PyObject *arguments) \
    { return bridge_operation(bridge, arguments, #name); }
OWNER_METHOD(observe)
OWNER_METHOD(members)
OWNER_METHOD(member)
OWNER_METHOD(signal)
OWNER_METHOD(latch)
OWNER_METHOD(reap)
#undef OWNER_METHOD

static PyObject *bridge_release(Bridge *bridge, PyObject *value)
{
    Owner *owner = checked_owner(bridge, value, 1);
    if (owner == NULL) {
        return NULL;
    }
    owner->release_requested = 1;
    owner->stopped = 1;
    PyErr_SetString(PyExc_NotImplementedError,
                    "woc_release is allocation-only; Layer B reconciliation handoff is unimplemented; retained");
    return NULL;
}

static PyObject *forbidden_new(PyTypeObject *type, PyObject *arguments, PyObject *keywords)
{
    (void)type;
    (void)arguments;
    (void)keywords;
    PyErr_SetString(PyExc_TypeError, "Opaque nonconstructible bridge/owner");
    return NULL;
}

static PyObject *forbidden_copy(PyObject *value, PyObject *arguments)
{
    (void)value;
    (void)arguments;
    PyErr_SetString(PyExc_TypeError, "Bridge owners cannot be copied or serialized");
    return NULL;
}

static PyObject *owner_repr(PyObject *value)
{
    (void)value;
    return PyUnicode_FromString("<opaque owned-child owner; unqualified, retained>");
}

static void owner_dealloc(Owner *owner)
{
    if (owner->native != NULL) {
        Py_INCREF((PyObject *)owner);
        owner->stopped = 1;
        return;
    }
    Py_XDECREF((PyObject *)owner->bridge);
    PyObject_Del(owner);
}

static PyMethodDef owner_methods[] = {
    {"__copy__", forbidden_copy, METH_VARARGS, NULL},
    {"__deepcopy__", forbidden_copy, METH_VARARGS, NULL},
    {"__reduce_ex__", forbidden_copy, METH_VARARGS, NULL},
    {NULL, NULL, 0, NULL}
};


/* Additive Layer B SOURCE ONLY. Python wrappers AND native attempt/terminal roots
 * are allocated before spawn. No GIL release, callbacks or Python allocation at
 * the irreversible native boundaries. All result mappings are recoverable reads.
 * The static roots intentionally survive module removal; no reinitialization or
 * reset is supported. Whole-process wait/SIGCHLD/FD exclusion remains unqualified.
 * Child-v1 mappings preserve raw/scalar wait UID diagnostics without identity
 * authority. Native records retain registered BSD uid, unavailable wait UID and
 * operation-specific uid_evidence; Python mappings do not derive owner identity.
 */
_Static_assert(WOC_CHILD_NATIVE_QUALIFIED == 0, "Child qualification stays disabled");

typedef struct {
    PyObject_HEAD
    Bridge *bridge;
    woc_child *native;
    uint64_t generation;
    int invalid;
} Child;

typedef struct {
    PyObject_HEAD
    Bridge *bridge;
    woc_terminal *native;
    uint64_t generation;
} TerminalSink;

static PyTypeObject ChildType;
static PyTypeObject TerminalSinkType;
static Child *retained_child;
static Bridge *retained_bridge;
static TerminalSink *sink_roots[WOC_CHILD_HISTORY];
static size_t sink_count;

static int lifecycle(void)
{
    /* Python calls these methods with an attached thread. PyGILState_Check alone
     * is insufficient for subinterpreters; compare the actual interpreter too.
     * These guards express a supported candidate lifecycle, not its qualification.
     */
    if (WOC_PY_FINALIZING() || !PyGILState_Check()) {
        PyErr_SetString(PyExc_RuntimeError, "Finalizing/non-GIL lifecycle is unsupported");
        return 0;
    }
    PyInterpreterState *current = PyInterpreterState_Get();
    if (current != PyInterpreterState_Main() ||
        (module_interpreter != NULL && current != module_interpreter)) {
        PyErr_SetString(PyExc_RuntimeError, "Main interpreter lifecycle required");
        return 0;
    }
    return 1;
}

static int module_enter(void)
{
    if (!lifecycle()) return 0;
    if (module_busy) {
        PyErr_SetString(PyExc_RuntimeError, "Racing/reentrant module operation retained and rejected");
        return 0;
    }
    module_busy = 1;
    return 1;
}

static PyObject *module_done(PyObject *result)
{
    module_busy = 0;
    return result;
}


static PyObject *abort_mapping(const woc_launch_abort_report *a)
{
    PyObject *m = PyDict_New();
    if (!m) return NULL;
#define ABORT_INT(name) if (put(m, #name, PyLong_FromLong(a->name)) < 0) goto failed;
#define ABORT_U64(name) if (put(m, #name, PyLong_FromUnsignedLongLong(a->name)) < 0) goto failed;
#define ABORT_I64(name) if (put(m, #name, PyLong_FromLongLong(a->name)) < 0) goto failed;
    ABORT_INT(last_operation) ABORT_INT(last_rc) ABORT_INT(last_system_errno)
    ABORT_INT(phase) ABORT_INT(started) ABORT_INT(entries) ABORT_INT(max_entries)
    ABORT_U64(deadline_ns) ABORT_U64(origin_ns) ABORT_U64(now_ns) ABORT_INT(observation_count)
    ABORT_INT(max_observations) ABORT_INT(trace_count) ABORT_INT(failure_rc)
    ABORT_INT(failure_system_errno) ABORT_INT(kill_entered) ABORT_INT(kill_returned)
    ABORT_INT(kill_rc) ABORT_INT(kill_errno) ABORT_INT(preexisting_exit)
    ABORT_INT(exit_observation_index) ABORT_INT(exit_valid) ABORT_INT(bsd_status)
    ABORT_I64(bsd_pid) ABORT_I64(bsd_ppid) ABORT_I64(bsd_uid) ABORT_I64(bsd_pgid) ABORT_I64(sid)
#undef ABORT_INT
#undef ABORT_U64
#undef ABORT_I64
    if (a->trace_count > WOC_ABORT_FACTS || a->observation_count > WOC_ABORT_OBSERVATIONS) {
        PyErr_SetString(PyExc_RuntimeError, "Corrupt native abort bounds"); goto failed;
    }
    PyObject *facts = PyTuple_New(a->trace_count);
    if (!facts) goto failed;
    for (uint32_t i = 0; i < a->trace_count; ++i) {
        const woc_child_call_fact *f = &a->trace[i];
        PyObject *entry = Py_BuildValue("(iiiLi)", f->kind, f->entered, f->returned,
                                        (long long)f->raw_rc, f->raw_errno);
        if (!entry) { Py_DECREF(facts); goto failed; }
        PyTuple_SET_ITEM(facts, i, entry);
    }
    if (put(m, "trace", facts) < 0) goto failed;
    PyObject *slots = PyTuple_New(WOC_ABORT_RAW_SLOTS);
    if (!slots) goto failed;
    for (uint32_t i = 0; i < WOC_ABORT_RAW_SLOTS; ++i) {
        const woc_abort_observation *o = &a->observations[i];
        PyObject *entry = Py_BuildValue("{s:i,s:i,s:i,s:i,s:K,s:L,s:L,s:i,s:i}",
            "entered", o->entered, "returned", o->returned, "rc", o->rc,
            "system_errno", o->system_errno, "raw_siginfo_bytes", (unsigned long long)o->raw_siginfo_bytes,
            "pid", (long long)o->pid, "uid", (long long)o->uid, "code", o->code, "status", o->status);
        if (!entry) { Py_DECREF(slots); goto failed; }
        PyTuple_SET_ITEM(slots, i, entry);
    }
    if (put(m, "observations", slots) < 0 || put(m, "record", record_mapping(&a->record)) < 0)
        goto failed;
    return m;
failed:
    Py_DECREF(m); return NULL;
}

static PyObject *registration_mapping(const woc_registration_observation *o)
{
    PyObject *m = PyDict_New();
    if (!m) return NULL;
#define REG_INT(name) if (put(m, #name, PyLong_FromLong(o->name)) < 0) goto failed;
#define REG_I64(name) if (put(m, #name, PyLong_FromLongLong(o->name)) < 0) goto failed;
    REG_INT(entered) REG_INT(returned) REG_INT(rc) REG_INT(system_errno)
    REG_INT(idtype) REG_I64(target_pid) REG_INT(options)
    REG_INT(signo) REG_INT(code) REG_I64(pid) REG_I64(uid) REG_INT(status)
#undef REG_INT
#undef REG_I64
    if (put(m, "raw_siginfo_bytes", PyLong_FromSize_t(o->raw_siginfo_bytes)) < 0) goto failed;
    return m;
failed:
    Py_DECREF(m); return NULL;
}

static PyObject *child_mapping(const woc_child_report *r)
{
    PyObject *m = PyDict_New();
    if (!m) return NULL;
#define CHILD_INT(name) if (put(m, #name, PyLong_FromLong(r->name)) < 0) goto failed;
#define CHILD_U64(name) if (put(m, #name, PyLong_FromUnsignedLongLong(r->name)) < 0) goto failed;
#define CHILD_I64(name) if (put(m, #name, PyLong_FromLongLong(r->name)) < 0) goto failed;
    CHILD_U64(generation) CHILD_INT(state) CHILD_INT(rc) CHILD_INT(system_errno)
    CHILD_INT(blocked) CHILD_INT(published) CHILD_INT(pid_valid) CHILD_INT(registered)
    CHILD_INT(registration_bsd_status) CHILD_I64(registration_pid) CHILD_I64(registration_ppid)
    CHILD_I64(registration_pgid) CHILD_I64(registration_uid) CHILD_I64(registration_sid)
    CHILD_INT(spawn_entered) CHILD_INT(spawn_returned) CHILD_INT(spawn_rc) CHILD_INT(spawn_errno)
    CHILD_I64(spawn_pid_raw) CHILD_INT(resume_entered) CHILD_INT(resume_returned)
    CHILD_INT(resume_rc) CHILD_INT(resume_errno) CHILD_INT(launch_entries)
    CHILD_INT(launch_fact_count) CHILD_U64(launch_deadline_ns)
    CHILD_INT(resources_closed) CHILD_INT(resource_error) CHILD_INT(cleanup_started)
    CHILD_INT(cleanup_entries) CHILD_INT(max_cleanup_entries) CHILD_U64(cleanup_deadline_ns)
    CHILD_U64(cleanup_kill_at_ns) CHILD_U64(cleanup_now_ns) CHILD_INT(clock_rc) CHILD_INT(clock_errno)
    CHILD_INT(term_attempted) CHILD_INT(kill_attempted) CHILD_INT(signal_failed)
    CHILD_INT(term_returned) CHILD_INT(term_rc) CHILD_INT(term_errno)
    CHILD_INT(kill_returned) CHILD_INT(kill_rc) CHILD_INT(kill_errno) CHILD_INT(latch_attempted)
    CHILD_INT(sigchld_preflight_entered) CHILD_INT(sigchld_preflight_returned)
    CHILD_INT(sigchld_preflight_rc) CHILD_INT(sigchld_preflight_errno)
    CHILD_INT(sigchld_preflight_policy_valid) CHILD_INT(sigchld_preflight_default)
    CHILD_INT(sigchld_preflight_no_cldwait) CHILD_INT(sigchld_preflight_accepted)
    CHILD_U64(launch_failure_mask) CHILD_INT(launch_first_failure_phase)
    CHILD_INT(launch_first_failure_reason) CHILD_INT(launch_first_woc_rc)
    CHILD_INT(launch_first_system_errno)
#undef CHILD_INT
#undef CHILD_U64
#undef CHILD_I64
    if (put(m, "record", record_mapping(&r->record)) < 0 ||
        put(m, "abort", abort_mapping(&r->abort)) < 0 ||
        put(m, "registration_observation", registration_mapping(&r->registration_observation)) < 0) goto failed;
    if (r->launch_fact_count > WOC_CHILD_LAUNCH_FACTS) {
        PyErr_SetString(PyExc_RuntimeError, "Corrupt native launch trace"); goto failed;
    }
    PyObject *facts = PyTuple_New(r->launch_fact_count);
    if (!facts) goto failed;
    for (uint32_t i = 0; i < r->launch_fact_count; ++i) {
        const woc_child_call_fact *f = &r->launch_facts[i];
        PyObject *entry = Py_BuildValue("(iiiLi)", f->kind, f->entered, f->returned,
                                       (long long)f->raw_rc, f->raw_errno);
        if (!entry) { Py_DECREF(facts); goto failed; }
        PyTuple_SET_ITEM(facts, i, entry);
    }
    if (put(m, "launch_facts", facts) < 0) goto failed;
    return m;
failed:
    Py_DECREF(m); return NULL;
}

static Child *checked_child(Bridge *bridge, PyObject *value)
{
    if (Py_TYPE(value) != &ChildType) {
        PyErr_SetString(PyExc_TypeError, "Exact native-created Child required"); return NULL;
    }
    Child *c = (Child *)value;
    uint64_t generation = 0;
    if (c != retained_child || c->bridge != bridge || c->native == NULL || c->invalid ||
        woc_child_pending(bridge, &generation) != c->native || generation != c->generation) {
        PyErr_SetString(PyExc_RuntimeError, "Foreign/stale/nonretained Child"); return NULL;
    }
    return c;
}

static TerminalSink *checked_sink(Bridge *bridge, PyObject *value)
{
    if (Py_TYPE(value) != &TerminalSinkType) {
        PyErr_SetString(PyExc_TypeError, "Exact preallocated native TerminalSink required"); return NULL;
    }
    TerminalSink *t = (TerminalSink *)value;
    int rooted = 0;
    for (size_t i = 0; i < sink_count; ++i) if (sink_roots[i] == t) rooted = 1;
    if (!rooted || t->bridge != bridge || t->native == NULL || t->generation == 0) {
        PyErr_SetString(PyExc_RuntimeError, "Foreign/unrooted TerminalSink"); return NULL;
    }
    return t;
}

static int exact_string(PyObject *o, woc_child_string *s)
{
    if (!PyBytes_CheckExact(o)) {
        PyErr_SetString(PyExc_TypeError, "Exact copied bytes required"); return 0;
    }
    Py_ssize_t n = PyBytes_GET_SIZE(o);
    if (n <= 0 || n >= WOC_CHILD_STRING_BYTES) {
        PyErr_SetString(PyExc_ValueError, "Bounded nonempty bytes required"); return 0;
    }
    s->data = PyBytes_AS_STRING(o); s->bytes = (size_t)n;
    return 1;
}

static int exact_vector(PyObject *o, woc_child_string *items, size_t *count)
{
    if (!PyTuple_CheckExact(o) || PyTuple_GET_SIZE(o) > WOC_CHILD_VECTOR_ITEMS) {
        PyErr_SetString(PyExc_TypeError, "Exact bounded tuple of bytes required"); return 0;
    }
    *count = (size_t)PyTuple_GET_SIZE(o);
    for (size_t i = 0; i < *count; ++i)
        if (!exact_string(PyTuple_GET_ITEM(o, (Py_ssize_t)i), &items[i])) return 0;
    return 1;
}

static PyObject *bridge_create_common(Bridge *bridge, PyObject *args, int suspended)
{
    if (!module_enter()) return NULL;
    if (!WOC_CHILD_NATIVE_QUALIFIED) {
        PyErr_SetString(PyExc_RuntimeError, "Native Child creation is hard-disabled, unqualified");
        return module_done(NULL);
    }
    PyObject *exe, *argv, *env, *cwd, *fds, *launch, *timeout, *grace, *budget;
    woc_child_spec spec;
    woc_child_string arg_items[WOC_CHILD_VECTOR_ITEMS], env_items[WOC_CHILD_VECTOR_ITEMS];
    memset(&spec, 0, sizeof(spec));
    int64_t a, b, d, e;
    if (!PyArg_ParseTuple(args, "OOOOOOOOO:create", &exe, &argv, &env, &cwd, &fds,
                          &launch, &timeout, &grace, &budget) ||
        !exact_string(exe, &spec.executable) || !exact_string(cwd, &spec.cwd) ||
        !exact_vector(argv, arg_items, &spec.argc) || !exact_vector(env, env_items, &spec.envc) ||
        !integer64(launch, 1, 60000000000LL, &a) ||
        !integer64(timeout, 1, 60000000000LL, &b) ||
        !integer64(grace, 0, 60000000000LL, &d) || !integer64(budget, 1, 512, &e))
        return module_done(NULL);
    if (!PyTuple_CheckExact(fds) || PyTuple_GET_SIZE(fds) != 3 || d > b ||
        retained_owner != NULL || retained_child != NULL || sink_count >= WOC_CHILD_HISTORY) {
        PyErr_SetString(PyExc_RuntimeError, "Invalid stdio/budget or retained/exhausted registry");
        return module_done(NULL);
    }
    for (size_t i = 0; i < 3; ++i) {
        int64_t fd;
        if (!integer64(PyTuple_GET_ITEM(fds, (Py_ssize_t)i), 0, INT_MAX, &fd)) return module_done(NULL);
        spec.stdio[i] = (int)fd;
    }
    spec.argv = arg_items; spec.env = env_items;
    spec.launch_timeout_ns = (uint64_t)a; spec.cleanup_timeout_ns = (uint64_t)b;
    spec.grace_ns = (uint64_t)d; spec.max_cleanup_entries = (uint32_t)e;
    Child *c = PyObject_New(Child, &ChildType);
    if (!c) return module_done(NULL);
    c->bridge = bridge; Py_INCREF((PyObject *)bridge);
    c->native = NULL; c->generation = 0; c->invalid = 0;
    TerminalSink *t = PyObject_New(TerminalSink, &TerminalSinkType);
    if (!t) { Py_DECREF((PyObject *)c); return module_done(NULL); }
    t->bridge = bridge; Py_INCREF((PyObject *)bridge);
    t->native = NULL; t->generation = 0;
    /* Both exact Python objects have permanent strong roots before native creation. */
    retained_child = c; Py_INCREF((PyObject *)c);
    sink_roots[sink_count++] = t; Py_INCREF((PyObject *)t);
    int rc = woc_child_prepare(bridge, &spec, &c->native);
    if (c->native == NULL) {
        /* No native root/resources/creation were entered. Undo only this reservation. */
        retained_child = NULL; sink_roots[--sink_count] = NULL;
        Py_DECREF((PyObject *)c); Py_DECREF((PyObject *)t);
        Py_DECREF((PyObject *)c); Py_DECREF((PyObject *)t);
        PyErr_SetString(PyExc_RuntimeError, "Native preparation rejected before rooted attempt");
        return module_done(NULL);
    }
    uint64_t generation = 0;
    woc_child *pending = woc_child_pending(bridge, &generation);
    c->generation = generation; t->generation = generation;
    t->native = woc_child_terminal(c->native, bridge, generation);
    if (pending != c->native || generation == 0 || t->native == NULL) rc = WOC_OWNERSHIP;
    if (rc == WOC_OK) rc = woc_child_spawn(c->native, bridge, generation);
    if (rc == WOC_OK && !suspended) rc = woc_child_resume(c->native, bridge, generation);
    if (rc == WOC_OK && !suspended) rc = woc_child_publish(c->native, bridge, generation);
    /* Native raw outcomes/rooted ownership are retained before any Python error,
     * allocation, signal check or publication. pending() includes every error state. */
    Py_DECREF((PyObject *)t);
    if (rc != WOC_OK) {
        Py_DECREF((PyObject *)c);
        PyErr_SetString(PyExc_RuntimeError, "Native creation failed/unknown; recover exact Child with pending()");
        return module_done(NULL);
    }
    return module_done((PyObject *)c); /* exact Child, never a returned-object mock wrapper */
}

static PyObject *bridge_create(Bridge *b, PyObject *args)
{ return bridge_create_common(b, args, 0); }
static PyObject *bridge_create_suspended(Bridge *b, PyObject *args)
{ return bridge_create_common(b, args, 1); }

static PyObject *bridge_pending(Bridge *bridge, PyObject *unused)
{
    (void)unused;
    if (!module_enter()) return NULL;
    if (retained_child == NULL) { Py_INCREF(Py_None); return module_done(Py_None); }
    if (retained_child->bridge != bridge) {
        PyErr_SetString(PyExc_RuntimeError, "Foreign Bridge has the retained attempt"); return module_done(NULL);
    }
    Py_INCREF((PyObject *)retained_child);
    return module_done((PyObject *)retained_child);
}

static PyObject *bridge_child_snapshot(Bridge *bridge, PyObject *value)
{
    if (!module_enter()) return NULL;
    Child *c = checked_child(bridge, value);
    woc_child_report report;
    if (!c) return module_done(NULL);
    int rc = woc_child_snapshot(c->native, bridge, c->generation, &report);
    if (rc != WOC_OK) {
        PyErr_SetString(PyExc_RuntimeError, "Native snapshot rejected"); return module_done(NULL);
    }
    return module_done(child_mapping(&report));
}

static PyObject *active_mapping(const woc_active_observation *r, const void *raw,
                                 size_t bytes, int fresh, int first)
{
    if (bytes > PY_SSIZE_T_MAX || (bytes != 0 && raw == NULL)) {
        PyErr_SetString(PyExc_RuntimeError, "Invalid retained active observation bytes"); return NULL;
    }
    PyObject *m = PyDict_New();
    if (!m) return NULL;
#define ACTIVE_INT(name) if (put(m, #name, PyLong_FromLong(r->name)) < 0) goto failed;
#define ACTIVE_U64(name) if (put(m, #name, PyLong_FromUnsignedLongLong(r->name)) < 0) goto failed;
#define ACTIVE_I64(name) if (put(m, #name, PyLong_FromLongLong(r->name)) < 0) goto failed;
    ACTIVE_INT(bytes) ACTIVE_INT(version)
    ACTIVE_U64(generation) ACTIVE_U64(sequence) ACTIVE_U64(started_ns) ACTIVE_U64(finished_ns)
    ACTIVE_U64(birth_seconds) ACTIVE_U64(birth_microseconds)
    ACTIVE_INT(entered) ACTIVE_INT(returned) ACTIVE_INT(rc) ACTIVE_INT(system_errno)
    ACTIVE_INT(wait_entered) ACTIVE_INT(wait_returned) ACTIVE_INT(wait_rc) ACTIVE_INT(wait_errno)
    ACTIVE_INT(bsd_entered) ACTIVE_INT(bsd_returned) ACTIVE_INT(bsd_bytes) ACTIVE_INT(bsd_status)
    ACTIVE_INT(start_clock_rc) ACTIVE_INT(start_clock_errno)
    ACTIVE_INT(finish_clock_rc) ACTIVE_INT(finish_clock_errno)
    ACTIVE_I64(creator_pid) ACTIVE_I64(creator_uid) ACTIVE_I64(observed_sid)
#undef ACTIVE_INT
#undef ACTIVE_U64
#undef ACTIVE_I64
    if (put(m, "fresh_native_call", PyBool_FromLong(fresh)) < 0 ||
        put(m, "first_terminal", PyBool_FromLong(first)) < 0 ||
        put(m, "collection_evidence", PyBool_FromLong(0)) < 0 ||
        put(m, "record", record_mapping(&r->record)) < 0 ||
        put(m, "raw_siginfo", PyBytes_FromStringAndSize(raw, (Py_ssize_t)bytes)) < 0) goto failed;
    return m;
failed:
    Py_DECREF(m); return NULL;
}

static PyObject *bridge_active_observe(Bridge *bridge, PyObject *value)
{
    /* Default gate precedes lifecycle, argument access and every native query. */
    if (!WOC_CHILD_NATIVE_QUALIFIED) {
        PyErr_SetString(PyExc_RuntimeError, "Native active observation is hard-disabled, unqualified"); return NULL;
    }
    if (!module_enter()) return NULL;
    Child *c = checked_child(bridge, value);
    if (!c) return module_done(NULL);
    woc_terminal *sink = NULL;
    for (size_t i = 0; i < sink_count; ++i) {
        if (sink_roots[i]->bridge == bridge && sink_roots[i]->generation == c->generation) {
            sink = sink_roots[i]->native; break;
        }
    }
    woc_active_observation before, after;
    const void *raw = NULL; size_t bytes = 0;
    if (sink == NULL || woc_terminal_active_snapshot(sink, bridge, c->generation, 0,
                                                     &before, &raw, &bytes) != WOC_OK) {
        PyErr_SetString(PyExc_RuntimeError, "Exact active-observation terminal root missing; retained");
        return module_done(NULL);
    }
    int rc = woc_child_active_observe(c->native, bridge, c->generation);
    int snapshot_rc = woc_terminal_active_snapshot(sink, bridge, c->generation, 0, &after, &raw, &bytes);
    if (rc != WOC_OK || snapshot_rc != WOC_OK || before.sequence == UINT64_MAX ||
        after.sequence != before.sequence + 1 || after.generation != c->generation ||
        after.bytes != sizeof(after) || after.version != WOC_ACTIVE_VERSION ||
        !after.entered || !after.returned || after.rc != rc ||
        (after.record.state != WOC_LIVE && after.record.state != WOC_EXITED &&
         after.record.state != WOC_ACTIVE_TRANSITION)) {
        PyErr_SetString(PyExc_RuntimeError, "Native active observation refused or snapshot disagreed; all roots retained");
        return module_done(NULL);
    }
    return module_done(active_mapping(&after, raw, bytes, 1, 0));
}

static PyObject *terminal_active_snapshot(TerminalSink *t, int first)
{
    if (!WOC_CHILD_NATIVE_QUALIFIED) {
        PyErr_SetString(PyExc_RuntimeError, "Native active snapshot is hard-disabled, unqualified"); return NULL;
    }
    if (!module_enter()) return NULL;
    if (!checked_sink(t->bridge, (PyObject *)t)) return module_done(NULL);
    woc_active_observation r;
    const void *raw = NULL; size_t bytes = 0;
    if (woc_terminal_active_snapshot(t->native, t->bridge, t->generation, first, &r, &raw, &bytes) != WOC_OK) {
        PyErr_SetString(PyExc_RuntimeError, "Native active snapshot rejected; terminal retained");
        return module_done(NULL);
    }
    return module_done(active_mapping(&r, raw, bytes, 0, first));
}
static PyObject *terminal_active_observation(TerminalSink *t, PyObject *unused)
{ (void)unused; return terminal_active_snapshot(t, 0); }
static PyObject *terminal_active_first_terminal(TerminalSink *t, PyObject *unused)
{ (void)unused; return terminal_active_snapshot(t, 1); }

static PyObject *bridge_child_operation(Bridge *bridge, PyObject *args, const char *op)
{
    if (!module_enter()) return NULL;
    PyObject *value, *a_value = NULL, *b_value = NULL;
    int64_t a = 0, b = 0;
    if (!strcmp(op, "child_signal") || !strcmp(op, "child_latch")) {
        if (!PyArg_ParseTuple(args, "OOO", &value, &a_value, &b_value) ||
            !integer64(a_value, 1, !strcmp(op, "child_signal") ? WOC_KILL : WOC_EXIT_CORE, &a) ||
            !integer64(b_value, 0, INT_MAX, &b)) return module_done(NULL);
    } else if (!strcmp(op, "child_member")) {
        if (!PyArg_ParseTuple(args, "OO", &value, &a_value) ||
            !integer64(a_value, 2, INT_MAX, &a)) return module_done(NULL);
    } else if (!PyArg_ParseTuple(args, "O", &value)) return module_done(NULL);
    Child *c = checked_child(bridge, value);
    if (!c) return module_done(NULL);
    int rc;
    if (!strcmp(op, "cleanup_begin")) rc = woc_child_cleanup_begin(c->native, bridge, c->generation);
    else if (!strcmp(op, "child_observe")) rc = woc_child_observe(c->native, bridge, c->generation);
    else if (!strcmp(op, "child_members")) rc = woc_child_members(c->native, bridge, c->generation);
    else if (!strcmp(op, "child_member")) rc = woc_child_member(c->native, bridge, c->generation, a);
    else if (!strcmp(op, "child_signal")) rc = woc_child_signal(c->native, bridge, c->generation, (int)a, b);
    else if (!strcmp(op, "child_latch")) rc = woc_child_latch(c->native, bridge, c->generation, a, b);
    else rc = woc_child_reap(c->native, bridge, c->generation);
    woc_child_report report;
    int snapshot_rc = woc_child_snapshot(c->native, bridge, c->generation, &report);
    if (snapshot_rc != WOC_OK || report.rc != rc) {
        PyErr_SetString(PyExc_RuntimeError, "Native operation/snapshot disagreement; retained");
        return module_done(NULL);
    }
    return module_done(child_mapping(&report)); /* failure to map never deletes native facts */
}

#define CHILD_METHOD(name) \
static PyObject *bridge_##name(Bridge *bridge, PyObject *args) \
{ return bridge_child_operation(bridge, args, #name); }
CHILD_METHOD(cleanup_begin)
CHILD_METHOD(child_observe)
CHILD_METHOD(child_members)
CHILD_METHOD(child_member)
CHILD_METHOD(child_signal)
CHILD_METHOD(child_latch)
CHILD_METHOD(child_reap)
#undef CHILD_METHOD

static PyObject *bridge_terminal(Bridge *bridge, PyObject *value)
{
    if (!module_enter()) return NULL;
    Child *c = checked_child(bridge, value);
    if (!c) return module_done(NULL);
    for (size_t i = 0; i < sink_count; ++i) {
        TerminalSink *t = sink_roots[i];
        if (t->bridge == bridge && t->generation == c->generation && t->native != NULL) {
            Py_INCREF((PyObject *)t); return module_done((PyObject *)t);
        }
    }
    PyErr_SetString(PyExc_RuntimeError, "Native terminal reservation missing; retained");
    return module_done(NULL);
}

static PyObject *bridge_handoff(Bridge *bridge, PyObject *args)
{
    if (!module_enter()) return NULL;
    PyObject *child_value, *sink_value;
    if (!PyArg_ParseTuple(args, "OO", &child_value, &sink_value)) return module_done(NULL);
    Child *c = checked_child(bridge, child_value);
    TerminalSink *t = checked_sink(bridge, sink_value);
    if (!c || !t) return module_done(NULL);
    if (c->generation != t->generation) {
        PyErr_SetString(PyExc_RuntimeError, "Terminal generation mismatch"); return module_done(NULL);
    }
    int rc = woc_child_handoff(c->native, bridge, c->generation, t->native);
    if (rc != WOC_OK) {
        PyErr_SetString(PyExc_RuntimeError, "Native terminal handoff refused; all roots retained");
        return module_done(NULL);
    }
    /* No allocation/callback/signal check between native free and invalidation. */
    c->native = NULL; c->invalid = 1;
    retained_child = NULL;
    Py_DECREF((PyObject *)c); /* release only the operational wrapper root */
    Py_INCREF((PyObject *)t);
    return module_done((PyObject *)t);
}


/* Gate/lifecycle precede all argument access. Mapping loss retains native facts. */
static PyObject *bridge_abort_operation(Bridge *bridge, PyObject *value, int operation)
{
    if (!module_enter()) return NULL;
    if (!WOC_CHILD_NATIVE_QUALIFIED) {
        PyErr_SetString(PyExc_RuntimeError, "Native Child abort is hard-disabled, unqualified");
        return module_done(NULL);
    }
    Child *c = checked_child(bridge, value);
    if (!c) return module_done(NULL);
    int rc;
    if (operation == WOC_ABORT_OP_BEGIN) rc = woc_child_abort_begin(c->native, bridge, c->generation);
    else if (operation == WOC_ABORT_OP_OBSERVE) rc = woc_child_abort_observe(c->native, bridge, c->generation);
    else rc = woc_child_abort_reap(c->native, bridge, c->generation);
    woc_child_report r;
    int snapshot_rc = woc_child_snapshot(c->native, bridge, c->generation, &r);
    int valid_phase = snapshot_rc == WOC_OK && r.abort.phase >= WOC_ABORT_NONE_RESERVED &&
                      r.abort.phase <= WOC_ABORT_FAILED_RETAINED;
    if (valid_phase && rc == WOC_OK) {
        if (operation == WOC_ABORT_OP_REAP) valid_phase = r.abort.phase == WOC_ABORT_REAP_RECORDED;
        else valid_phase = r.abort.phase == WOC_ABORT_WAITING_EXIT || r.abort.phase == WOC_ABORT_EXIT_READY;
    }
    /* Top-level rc/errno/blocked/state are immutable original launch facts here. */
    if (!valid_phase || r.abort.last_operation != operation || r.abort.last_rc != rc) {
        PyErr_SetString(PyExc_RuntimeError, "Native abort step/snapshot disagreement; retained");
        return module_done(NULL);
    }
    return module_done(child_mapping(&r));
}
static PyObject *bridge_abort_begin(Bridge *b, PyObject *c)
{ return bridge_abort_operation(b, c, WOC_ABORT_OP_BEGIN); }
static PyObject *bridge_abort_observe(Bridge *b, PyObject *c)
{ return bridge_abort_operation(b, c, WOC_ABORT_OP_OBSERVE); }
static PyObject *bridge_abort_reap(Bridge *b, PyObject *c)
{ return bridge_abort_operation(b, c, WOC_ABORT_OP_REAP); }

static PyObject *bridge_abort_handoff(Bridge *bridge, PyObject *args)
{
    if (!module_enter()) return NULL;
    if (!WOC_CHILD_NATIVE_QUALIFIED) {
        PyErr_SetString(PyExc_RuntimeError, "Native Child abort handoff is hard-disabled, unqualified");
        return module_done(NULL);
    }
    PyObject *child_value, *sink_value;
    if (!PyArg_ParseTuple(args, "OO:abort_handoff", &child_value, &sink_value)) return module_done(NULL);
    Child *c = checked_child(bridge, child_value);
    TerminalSink *t = checked_sink(bridge, sink_value);
    if (!c || !t) return module_done(NULL);
    if (c->generation != t->generation) {
        PyErr_SetString(PyExc_RuntimeError, "Terminal generation mismatch"); return module_done(NULL);
    }
    int rc = woc_child_abort_handoff(c->native, bridge, c->generation, t->native);
    if (rc != WOC_OK) {
        PyErr_SetString(PyExc_RuntimeError, "Native abort handoff refused; roots retained");
        return module_done(NULL);
    }
    /* Native Child was freed. Invalidate BEFORE any read, mapping or publication. */
    c->native = NULL; c->invalid = 1;
    retained_child = NULL;
    woc_terminal_report r;
    const void *raw = NULL;
    int snapshot_rc = woc_terminal_snapshot(t->native, bridge, t->generation, &r, &raw);
    Py_DECREF((PyObject *)c);
    if (snapshot_rc != WOC_OK || raw == NULL || !r.transferred ||
        r.kind != WOC_TERMINAL_DIRECT_CHILD_ABORT_UNRESUMED ||
        !r.wait_entered || !r.wait_returned || r.wait_rc != 0 || !r.consumed ||
        !r.decoded || !r.certificate_matches || r.final_report.resume_entered ||
        r.final_report.abort.last_operation != WOC_ABORT_OP_HANDOFF ||
        r.final_report.abort.last_rc != rc || r.final_report.abort.phase != WOC_ABORT_TRANSFERRED) {
        PyErr_SetString(PyExc_RuntimeError, "Native terminal handoff snapshot disagreement; terminal retained");
        return module_done(NULL);
    }
    Py_INCREF((PyObject *)t);
    return module_done((PyObject *)t);
}

static PyObject *terminal_abort_observation(TerminalSink *t, PyObject *value)
{
    if (!module_enter()) return NULL;
    if (!WOC_CHILD_NATIVE_QUALIFIED) {
        PyErr_SetString(PyExc_RuntimeError, "Native Child abort raw read is hard-disabled, unqualified");
        return module_done(NULL);
    }
    if (!checked_sink(t->bridge, (PyObject *)t)) return module_done(NULL);
    int64_t index;
    if (!integer64(value, 0, WOC_ABORT_RAW_SLOTS - 1, &index)) return module_done(NULL);
    const void *raw = NULL; size_t bytes = 0;
    if (woc_terminal_abort_observation(t->native, t->bridge, t->generation, (uint32_t)index,
                                      &raw, &bytes) != WOC_OK || raw == NULL || bytes > PY_SSIZE_T_MAX) {
        PyErr_SetString(PyExc_RuntimeError, "Native abort observation read rejected");
        return module_done(NULL);
    }
    return module_done(PyBytes_FromStringAndSize(raw, (Py_ssize_t)bytes));
}

static PyObject *terminal_registration_observation(TerminalSink *t, PyObject *unused)
{
    (void)unused;
    if (!module_enter()) return NULL;
    if (!WOC_CHILD_NATIVE_QUALIFIED) {
        PyErr_SetString(PyExc_RuntimeError, "Native Child registration raw read is hard-disabled, unqualified");
        return module_done(NULL);
    }
    if (!checked_sink(t->bridge, (PyObject *)t)) return module_done(NULL);
    const void *raw = NULL; size_t bytes = 0;
    if (woc_terminal_registration_observation(t->native, t->bridge, t->generation,
                                             &raw, &bytes) != WOC_OK || raw == NULL ||
        bytes > PY_SSIZE_T_MAX) {
        PyErr_SetString(PyExc_RuntimeError, "Native registration observation read rejected");
        return module_done(NULL);
    }
    return module_done(PyBytes_FromStringAndSize(raw, (Py_ssize_t)bytes));
}

static PyObject *terminal_snapshot(TerminalSink *t, PyObject *unused)
{
    (void)unused;
    if (!module_enter()) return NULL;
    if (!checked_sink(t->bridge, (PyObject *)t)) return module_done(NULL);
    woc_terminal_report r;
    const void *raw = NULL;
    if (woc_terminal_snapshot(t->native, t->bridge, t->generation, &r, &raw) != WOC_OK || raw == NULL) {
        PyErr_SetString(PyExc_RuntimeError, "Native terminal read rejected"); return module_done(NULL);
    }
    PyObject *m = PyDict_New();
    if (!m) return module_done(NULL);
#define TERMINAL_INT(name) if (put(m, #name, PyLong_FromLong(r.name)) < 0) goto failed;
#define TERMINAL_I64(name) if (put(m, #name, PyLong_FromLongLong(r.name)) < 0) goto failed;
    if (put(m, "generation", PyLong_FromUnsignedLongLong(r.generation)) < 0) goto failed;
    TERMINAL_INT(rooted) TERMINAL_INT(transferred) TERMINAL_INT(known_no_creation) TERMINAL_INT(kind)
    TERMINAL_INT(reap_attempted) TERMINAL_INT(wait_entered) TERMINAL_INT(wait_returned)
    TERMINAL_INT(wait_rc) TERMINAL_INT(wait_errno) TERMINAL_I64(wait_pid) TERMINAL_I64(wait_uid)
    TERMINAL_INT(wait_code) TERMINAL_INT(wait_status) TERMINAL_INT(consumed)
    TERMINAL_INT(decoded) TERMINAL_INT(certificate_matches)
#undef TERMINAL_INT
#undef TERMINAL_I64
    if (r.raw_siginfo_bytes > PY_SSIZE_T_MAX) {
        PyErr_SetString(PyExc_RuntimeError, "Raw native siginfo size overflow"); goto failed;
    }
    if (put(m, "raw_siginfo", PyBytes_FromStringAndSize(raw, (Py_ssize_t)r.raw_siginfo_bytes)) < 0 ||
        put(m, "decoded_record", record_mapping(&r.decoded_record)) < 0 ||
        put(m, "final_report", child_mapping(&r.final_report)) < 0 ||
        put(m, "registration_observation", registration_mapping(&r.registration_observation)) < 0) goto failed;
    return module_done(m);
failed:
    Py_DECREF(m); return module_done(NULL);
}

static PyObject *bridge_history(Bridge *bridge, PyObject *unused)
{
    (void)unused;
    if (!module_enter()) return NULL;
    /* Include reserved/untransferred sinks too: raw reap/mapping failures recover. */
    Py_ssize_t n = 0;
    for (size_t i = 0; i < sink_count; ++i) if (sink_roots[i]->bridge == bridge) ++n;
    PyObject *history = PyTuple_New(n);
    if (!history) return module_done(NULL);
    n = 0;
    for (size_t i = 0; i < sink_count; ++i) {
        TerminalSink *t = sink_roots[i];
        if (t->bridge == bridge) { Py_INCREF((PyObject *)t); PyTuple_SET_ITEM(history, n++, (PyObject *)t); }
    }
    return module_done(history);
}

static void child_dealloc(Child *c)
{
    /* No poll, signal, wait, reap or native resource cleanup in any destructor. */
    if (c->native != NULL) { Py_INCREF((PyObject *)c); return; }
    Py_XDECREF((PyObject *)c->bridge); PyObject_Del(c);
}

static void sink_dealloc(TerminalSink *t)
{
    if (t->native != NULL) { Py_INCREF((PyObject *)t); return; }
    Py_XDECREF((PyObject *)t->bridge); PyObject_Del(t);
}

static PyMethodDef child_methods[] = {
    {"__copy__", forbidden_copy, METH_VARARGS, NULL},
    {"__deepcopy__", forbidden_copy, METH_VARARGS, NULL},
    {"__reduce_ex__", forbidden_copy, METH_VARARGS, NULL},
    {NULL, NULL, 0, NULL}
};
static PyMethodDef sink_methods[] = {
    {"snapshot", (PyCFunction)terminal_snapshot, METH_NOARGS, NULL},
    {"active_observation", (PyCFunction)terminal_active_observation, METH_NOARGS, NULL},
    {"active_first_terminal", (PyCFunction)terminal_active_first_terminal, METH_NOARGS, NULL},
    {"abort_observation", (PyCFunction)terminal_abort_observation, METH_O, NULL},
    {"registration_observation", (PyCFunction)terminal_registration_observation, METH_NOARGS, NULL},
    {"__copy__", forbidden_copy, METH_VARARGS, NULL},
    {"__deepcopy__", forbidden_copy, METH_VARARGS, NULL},
    {"__reduce_ex__", forbidden_copy, METH_VARARGS, NULL},
    {NULL, NULL, 0, NULL}
};
static PyTypeObject ChildType = {
    PyVarObject_HEAD_INIT(NULL, 0)
    .tp_name = "_window_owned_child_bridge.Child",
    .tp_basicsize = sizeof(Child), .tp_flags = Py_TPFLAGS_DEFAULT,
    .tp_methods = child_methods, .tp_new = forbidden_new,
    .tp_dealloc = (destructor)child_dealloc,
};
static PyTypeObject TerminalSinkType = {
    PyVarObject_HEAD_INIT(NULL, 0)
    .tp_name = "_window_owned_child_bridge.TerminalSink",
    .tp_basicsize = sizeof(TerminalSink), .tp_flags = Py_TPFLAGS_DEFAULT,
    .tp_methods = sink_methods, .tp_new = forbidden_new,
    .tp_dealloc = (destructor)sink_dealloc,
};

/* Legacy method guards share module serialization with the new domain. */
#define GUARDED_LEGACY(name) \
static PyObject *guarded_##name(Bridge *b, PyObject *a) \
{ if (!module_enter()) return NULL; return module_done(bridge_##name(b, a)); }
GUARDED_LEGACY(describe)
GUARDED_LEGACY(open)
GUARDED_LEGACY(observe)
GUARDED_LEGACY(members)
GUARDED_LEGACY(member)
GUARDED_LEGACY(signal)
GUARDED_LEGACY(latch)
GUARDED_LEGACY(reap)
GUARDED_LEGACY(release)
#undef GUARDED_LEGACY

static PyMethodDef bridge_methods[] = {
    {"create", (PyCFunction)bridge_create, METH_VARARGS, NULL},
    {"active_observe", (PyCFunction)bridge_active_observe, METH_O, NULL},
    {"create_suspended", (PyCFunction)bridge_create_suspended, METH_VARARGS, NULL},
    {"abort_begin", (PyCFunction)bridge_abort_begin, METH_O, NULL},
    {"abort_observe", (PyCFunction)bridge_abort_observe, METH_O, NULL},
    {"abort_reap", (PyCFunction)bridge_abort_reap, METH_O, NULL},
    {"abort_handoff", (PyCFunction)bridge_abort_handoff, METH_VARARGS, NULL},
    {"pending", (PyCFunction)bridge_pending, METH_NOARGS, NULL},
    {"child_snapshot", (PyCFunction)bridge_child_snapshot, METH_O, NULL},
    {"cleanup_begin", (PyCFunction)bridge_cleanup_begin, METH_VARARGS, NULL},
    {"child_observe", (PyCFunction)bridge_child_observe, METH_VARARGS, NULL},
    {"child_members", (PyCFunction)bridge_child_members, METH_VARARGS, NULL},
    {"child_member", (PyCFunction)bridge_child_member, METH_VARARGS, NULL},
    {"child_signal", (PyCFunction)bridge_child_signal, METH_VARARGS, NULL},
    {"child_latch", (PyCFunction)bridge_child_latch, METH_VARARGS, NULL},
    {"child_reap", (PyCFunction)bridge_child_reap, METH_VARARGS, NULL},
    {"terminal", (PyCFunction)bridge_terminal, METH_O, NULL},
    {"handoff", (PyCFunction)bridge_handoff, METH_VARARGS, NULL},
    {"history", (PyCFunction)bridge_history, METH_NOARGS, NULL},
    {"describe", (PyCFunction)guarded_describe, METH_NOARGS, NULL},
    {"open", (PyCFunction)guarded_open, METH_VARARGS, NULL},
    {"observe", (PyCFunction)guarded_observe, METH_VARARGS, NULL},
    {"members", (PyCFunction)guarded_members, METH_VARARGS, NULL},
    {"member", (PyCFunction)guarded_member, METH_VARARGS, NULL},
    {"signal", (PyCFunction)guarded_signal, METH_VARARGS, NULL},
    {"latch", (PyCFunction)guarded_latch, METH_VARARGS, NULL},
    {"reap", (PyCFunction)guarded_reap, METH_VARARGS, NULL},
    {"release", (PyCFunction)guarded_release, METH_O, NULL},
    {"__copy__", forbidden_copy, METH_VARARGS, NULL},
    {"__deepcopy__", forbidden_copy, METH_VARARGS, NULL},
    {"__reduce_ex__", forbidden_copy, METH_VARARGS, NULL},
    {NULL, NULL, 0, NULL}
};

static PyTypeObject BridgeType = {
    PyVarObject_HEAD_INIT(NULL, 0)
    .tp_name = "_window_owned_child_bridge.Bridge",
    .tp_basicsize = sizeof(Bridge),
    .tp_flags = Py_TPFLAGS_DEFAULT,
    .tp_methods = bridge_methods,
    .tp_new = forbidden_new,
};

static PyTypeObject OwnerType = {
    PyVarObject_HEAD_INIT(NULL, 0)
    .tp_name = "_window_owned_child_bridge.Owner",
    .tp_basicsize = sizeof(Owner),
    .tp_flags = Py_TPFLAGS_DEFAULT,
    .tp_methods = owner_methods,
    .tp_new = forbidden_new,
    .tp_repr = owner_repr,
    .tp_dealloc = (destructor)owner_dealloc,
};

static PyObject *new_bridge(PyObject *module, PyObject *unused)
{
    (void)module;
    (void)unused;
    if (!module_enter()) return NULL;
    /* The process/library registry has one stable Bridge key. A caller losing
     * its Python manager must still recover pending/history from a new factory
     * call, rather than receive a foreign key that strands the retained roots. */
    if (retained_bridge == NULL) {
        retained_bridge = PyObject_New(Bridge, &BridgeType);
        if (retained_bridge == NULL) return module_done(NULL);
    }
    Py_INCREF((PyObject *)retained_bridge);
    return module_done((PyObject *)retained_bridge);
}

static PyMethodDef module_methods[] = {
    {"new_bridge", new_bridge, METH_NOARGS, NULL},
    {NULL, NULL, 0, NULL}
};

static struct PyModuleDef module_definition = {
    PyModuleDef_HEAD_INIT,
    "_window_owned_child_bridge",
    "SOURCE-ONLY Layers A/B; hard-disabled, UNCOMPILED/UNRUN, no trusted bootstrap.",
    -1,
    module_methods,
    NULL, NULL, NULL, NULL
};

PyMODINIT_FUNC PyInit__window_owned_child_bridge(void)
{
    if (!lifecycle()) return NULL;
    if (module_initialization_started) {
        PyErr_SetString(PyExc_RuntimeError, "Library-image reinitialization/reset is unsupported; roots retained");
        return NULL;
    }
    module_initialization_started = 1;
    module_interpreter = PyInterpreterState_Get();
    if (PyType_Ready(&BridgeType) < 0 || PyType_Ready(&OwnerType) < 0 ||
        PyType_Ready(&ChildType) < 0 || PyType_Ready(&TerminalSinkType) < 0) return NULL;
    PyObject *module = PyModule_Create(&module_definition);
    if (module == NULL) return NULL;
    /* PyModule_AddObject steals only on success; use the 3.9 API explicitly. */
    Py_INCREF((PyObject *)&BridgeType);
    if (PyModule_AddObject(module, "Bridge", (PyObject *)&BridgeType) < 0) {
        Py_DECREF((PyObject *)&BridgeType); Py_DECREF(module); return NULL;
    }
    Py_INCREF((PyObject *)&ChildType);
    if (PyModule_AddObject(module, "Child", (PyObject *)&ChildType) < 0) {
        Py_DECREF((PyObject *)&ChildType); Py_DECREF(module); return NULL;
    }
    Py_INCREF((PyObject *)&TerminalSinkType);
    if (PyModule_AddObject(module, "TerminalSink", (PyObject *)&TerminalSinkType) < 0) {
        Py_DECREF((PyObject *)&TerminalSinkType); Py_DECREF(module); return NULL;
    }
    if (PyModule_AddIntConstant(module, "CHILD_NATIVE_QUALIFIED", WOC_CHILD_NATIVE_QUALIFIED) < 0) {
        Py_DECREF(module); return NULL;
    }
    return module;
}
