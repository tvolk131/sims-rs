/* Authored CPython mapping fixture, UNRUN until parent review/execution.
 * The runner makes exactly THREE asserted edits in positive temporary copies:
 * Child enum, producer assertion and PyC assertion. Production copies use zero.
 * Every function body in the copied producer/PyC is unchanged. All producer
 * process, clock, resource and allocator paths use the included mocks/traps.
 * TEST-ONLY seeding below hand-builds synthetic roots, never native creation.
 */
#ifndef _DARWIN_C_SOURCE
#define _DARWIN_C_SOURCE 1
#endif
#define PY_SSIZE_T_CLEAN
#include <Python.h>
#define main unused_observer_fixture_main
#include "window_active_observation.c"
#undef main

/* Local call-site interception only; no interpreter-wide allocator hooks. */
static int fixture_dictionary_countdown = -1;
static PyObject *fixture_dictionary_new(void)
{
    if (fixture_dictionary_countdown == 0) {
        fixture_dictionary_countdown = -1;
        return PyErr_NoMemory();
    }
    if (fixture_dictionary_countdown > 0) --fixture_dictionary_countdown;
    return PyDict_New();
}
#define PyDict_New() fixture_dictionary_new()
#define PyInit__window_owned_child_bridge fixture_original_module_init
#include "window_owned_child_python.c"
#undef PyInit__window_owned_child_bridge
#undef PyDict_New

static int fixture_seeded;
static Child *fixture_child;
static TerminalSink *fixture_sink;
static Bridge *fixture_bridge;

static PyObject *fixture_seed(PyObject *module, PyObject *unused)
{
    (void)module; (void)unused;
    if (fixture_seeded || retained_child != NULL || sink_count != 0) {
        PyErr_SetString(PyExc_RuntimeError, "one synthetic seed per private test image");
        return NULL;
    }
    PyObject *bridge = new_bridge(NULL, NULL);
    if (bridge == NULL) return NULL;
    Child *child = PyObject_New(Child, &ChildType);
    if (child == NULL) { Py_DECREF(bridge); return NULL; }
    child->bridge = (Bridge *)bridge; Py_INCREF(bridge);
    child->native = NULL; child->generation = 23; child->invalid = 0;
    TerminalSink *sink = PyObject_New(TerminalSink, &TerminalSinkType);
    if (sink == NULL) { Py_DECREF((PyObject *)child); Py_DECREF(bridge); return NULL; }
    sink->bridge = (Bridge *)bridge; Py_INCREF(bridge);
    sink->native = NULL; sink->generation = 23;
    /* Follow the real extension's permanent wrapper reservations, but the
     * native values are EXPLICITLY authored private test data, not a spawn. */
    retained_child = child; Py_INCREF((PyObject *)child);
    sink_roots[sink_count++] = sink; Py_INCREF((PyObject *)sink);
#if TEST_ACTIVE_MOCK
    c.key = t.key = bridge;
    child->native = &c; sink->native = &t;
#else
    /* Gate0 must return before inspecting these poisoned native addresses. */
    child->native = (woc_child *)(uintptr_t)1;
    sink->native = (woc_terminal *)(uintptr_t)1;
#endif
    fixture_child = child; fixture_sink = sink; fixture_bridge = (Bridge *)bridge;
    fixture_seeded = 1;
    PyObject *result = PyTuple_New(3);
    if (result == NULL) {
        Py_DECREF((PyObject *)child); Py_DECREF((PyObject *)sink); Py_DECREF(bridge);
        return NULL; /* permanent roots remain available; no native cleanup */
    }
    PyTuple_SET_ITEM(result, 0, bridge);
    PyTuple_SET_ITEM(result, 1, (PyObject *)child);
    PyTuple_SET_ITEM(result, 2, (PyObject *)sink);
    return result;
}

static PyObject *fixture_stats(PyObject *module, PyObject *unused)
{
    (void)module; (void)unused;
    unsigned long long sequence = 0;
    int first = 0;
#if TEST_ACTIVE_MOCK
    sequence = t.active.sequence; first = t.active_exit_saved;
#endif
    return Py_BuildValue("{s:I,s:I,s:I,s:I,s:I,s:I,s:I,s:I,s:I,s:i,s:i,s:K,s:i,s:i,s:i}",
        "clocks", m.clocks, "pids", m.pids, "uids", m.uids,
        "policies", m.policies, "waits", m.waits, "bsds", m.bsds,
        "sids", m.sids, "frees", m.frees, "calls", calls(),
        "module_busy", module_busy, "raw_bytes", (int)sizeof(siginfo_t),
        "sequence", sequence, "first_saved", first,
        "active_bytes", (int)sizeof(woc_active_observation), "event_uid", (int)m.event.si_uid);
}

static PyObject *fixture_fail_dictionary(PyObject *module, PyObject *count)
{
    (void)module;
    if (!PyLong_CheckExact(count)) {
        PyErr_SetString(PyExc_TypeError, "exact private dictionary failure index required"); return NULL;
    }
    long value = PyLong_AsLong(count);
    if (value < 1 || value > 4 || PyErr_Occurred()) {
        if (!PyErr_Occurred()) PyErr_SetString(PyExc_ValueError, "private failure index outside1..4");
        return NULL;
    }
    fixture_dictionary_countdown = (int)value - 1;
    Py_RETURN_NONE;
}

static PyObject *fixture_scenario(PyObject *module, PyObject *name)
{
    (void)module;
    if (!fixture_seeded || !PyUnicode_CheckExact(name)) {
        PyErr_SetString(PyExc_TypeError, "seeded exact private scenario name required"); return NULL;
    }
    const char *value = PyUnicode_AsUTF8(name);
    if (value == NULL) return NULL;
    if (strcmp(value, "module_busy") == 0) module_busy = 1;
    else if (strcmp(value, "module_idle") == 0) module_busy = 0;
    else if (strcmp(value, "child_generation") == 0) fixture_child->generation = 24;
    else if (strcmp(value, "child_generation_restore") == 0) fixture_child->generation = 23;
    else if (strcmp(value, "sink_generation") == 0) fixture_sink->generation = 24;
    else if (strcmp(value, "sink_generation_restore") == 0) fixture_sink->generation = 23;
    else if (strcmp(value, "sink_unroot") == 0) sink_count = 0;
    else if (strcmp(value, "sink_root_restore") == 0) sink_count = 1;
    else if (strcmp(value, "child_invalid") == 0) fixture_child->invalid = 1;
#if TEST_ACTIVE_MOCK
    else if (strcmp(value, "live") == 0) {
        memset(&m.event, 0, sizeof(m.event)); m.bsd.pbi_status = SRUN; m.bsd.pbi_flags = 0;
    } else if (strcmp(value, "transition") == 0) {
        memset(&m.event, 0, sizeof(m.event)); m.bsd.pbi_status = SZOMB;
    } else if (strcmp(value, "inexit") == 0) {
        memset(&m.event, 0, sizeof(m.event)); m.bsd.pbi_status = SRUN; m.bsd.pbi_flags = PROC_FLAG_INEXIT;
    } else if (strcmp(value, "exit0") == 0) event(CLD_EXITED, 0);
    else if (strcmp(value, "exit7") == 0) event(CLD_EXITED, 7);
    else if (strcmp(value, "signal") == 0) event(CLD_KILLED, SIGTERM);
    else if (strcmp(value, "core") == 0) event(CLD_DUMPED, SIGSEGV);
    else if (strcmp(value, "start_clock_failure") == 0) {
        CHECK(m.clocks + 1 < 8); m.time_rc[m.clocks] = -1; m.time_error[m.clocks] = EIO;
    } else if (strcmp(value, "finish_clock_failure") == 0) {
        CHECK(m.clocks + 1 < 8); m.time_rc[m.clocks + 1] = -1; m.time_error[m.clocks + 1] = EIO;
    } else if (strcmp(value, "cleanup") == 0) c.report.cleanup_started = 1;
    else if (strcmp(value, "wait_failure") == 0) { m.wait_rc = -1; m.wait_error = ECHILD; }
    else if (strcmp(value, "partial_bsd") == 0) m.bsd_bytes = sizeof(m.bsd) - 1;
    else if (strcmp(value, "creator_mismatch") == 0) m.creator = 100;
#endif
    else { PyErr_SetString(PyExc_ValueError, "unknown private finite mapping scenario"); return NULL; }
    Py_RETURN_NONE;
}

static PyObject *fixture_foreign(PyObject *module, PyObject *kind)
{
    (void)module;
    if (!fixture_seeded || !PyUnicode_CheckExact(kind)) {
        PyErr_SetString(PyExc_TypeError, "seeded private foreign-object kind required"); return NULL;
    }
    const char *value = PyUnicode_AsUTF8(kind);
    if (value == NULL) return NULL;
    if (strcmp(value, "bridge") == 0) return (PyObject *)PyObject_New(Bridge, &BridgeType);
    if (strcmp(value, "child") == 0) {
        Child *child = PyObject_New(Child, &ChildType);
        if (child == NULL) return NULL;
        child->bridge = fixture_bridge; Py_INCREF((PyObject *)fixture_bridge);
        child->native = fixture_child->native; child->generation = 23; child->invalid = 0;
        return (PyObject *)child; /* not retained_child: no operational authority */
    }
    if (strcmp(value, "sink") == 0) {
        TerminalSink *sink = PyObject_New(TerminalSink, &TerminalSinkType);
        if (sink == NULL) return NULL;
        sink->bridge = fixture_bridge; Py_INCREF((PyObject *)fixture_bridge);
        sink->native = fixture_sink->native; sink->generation = 23;
        return (PyObject *)sink; /* absent from sink_roots */
    }
    PyErr_SetString(PyExc_ValueError, "unknown private foreign-object kind"); return NULL;
}

static PyObject *fixture_raw(PyObject *module, PyObject *first)
{
    (void)module;
#if TEST_ACTIVE_MOCK
    if (!fixture_seeded || (first != Py_False && first != Py_True)) {
        PyErr_SetString(PyExc_TypeError, "seeded exact private raw selector required"); return NULL;
    }
    const siginfo_t *raw = first == Py_True ? &t.first_active_raw : &t.active_raw;
    return PyBytes_FromStringAndSize((const char *)raw, sizeof(*raw));
#else
    (void)first;
    PyErr_SetString(PyExc_RuntimeError, "no raw native fixture under production0"); return NULL;
#endif
}

static PyObject *fixture_handoff(PyObject *module, PyObject *unused)
{
    (void)module; (void)unused;
#if TEST_ACTIVE_MOCK
    if (!fixture_seeded || !t.active_exit_saved || retained_child != fixture_child) {
        PyErr_SetString(PyExc_RuntimeError, "private handoff requires one synthetic active exit"); return NULL;
    }
    /* Explicit authored collection facts. No consuming wait/reap is performed.
     * The actual PyC+C handoff then invalidates the wrapper and preserves sink. */
    c.report.cleanup_started = 1; c.report.cleanup_deadline_ns = 100000000000ULL;
    c.report.state = WOC_CHILD_REAP_RECORDED;
    c.owner.reap_attempted = c.owner.reaped = c.owner.no_signals = 1;
    t.report.kind = WOC_TERMINAL_NORMAL_GROUP_CERTIFICATE;
    t.report.wait_returned = t.report.consumed = t.report.decoded = t.report.certificate_matches = 1;
    m.only_free = &c;
    PyObject *args = PyTuple_Pack(2, (PyObject *)fixture_child, (PyObject *)fixture_sink);
    if (args == NULL) return NULL;
    PyObject *result = bridge_handoff(fixture_bridge, args);
    Py_DECREF(args); return result;
#else
    PyErr_SetString(PyExc_RuntimeError, "no handoff under production0"); return NULL;
#endif
}

static PyMethodDef fixture_methods[] = {
    {"_fixture_seed", fixture_seed, METH_NOARGS, NULL},
    {"_fixture_stats", fixture_stats, METH_NOARGS, NULL},
    {"_fixture_fail_dictionary", fixture_fail_dictionary, METH_O, NULL},
    {"_fixture_scenario", fixture_scenario, METH_O, NULL},
    {"_fixture_foreign", fixture_foreign, METH_O, NULL},
    {"_fixture_raw", fixture_raw, METH_O, NULL},
    {"_fixture_handoff", fixture_handoff, METH_NOARGS, NULL},
    {NULL, NULL, 0, NULL}
};

PyMODINIT_FUNC PyInit__window_owned_child_bridge(void)
{
#if TEST_ACTIVE_MOCK
    setup();
#else
    memset(&m, 0, sizeof(m)); m.creator = 99; m.uid = 501;
#endif
    PyObject *module = fixture_original_module_init();
    if (module == NULL) return NULL;
    if (PyModule_AddFunctions(module, fixture_methods) < 0) { Py_DECREF(module); return NULL; }
    return module;
}
