#ifndef _DARWIN_C_SOURCE
#define _DARWIN_C_SOURCE 1
#endif
#include "window_owned_child_darwin.h"

#if !defined(__APPLE__)
#error "Unsupported platform: this source-only backend requires reviewed Darwin SDK headers"
#endif

#include <errno.h>
#include <fcntl.h>
#include <spawn.h>
#include <time.h>
#include <limits.h>
#include <libproc.h>
#include <signal.h>
#include <stdlib.h>
#include <string.h>
#include <sys/proc.h>
#include <sys/proc_info.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

_Static_assert(sizeof(woc_description) == 32, "description ABI mismatch");
_Static_assert(sizeof(woc_record) == 200, "record ABI mismatch");

struct woc_owner {
    pid_t pid;
    pid_t pgid;
    uid_t uid;
    pid_t parent;
    pid_t session;
    pid_t members[2];
    int count;
    int no_signals;
    int reap_attempted;
    int reaped;
    int certified;
    int64_t exit_kind;
    int64_t exit_status;
};

static int failure(woc_record *out, int error, int system_errno)
{
    out->error = error;
    out->system_errno = system_errno;
    return error;
}

static int begin(woc_owner *owner, woc_record *out, size_t bytes, uint32_t version)
{
    if (out == NULL || bytes != sizeof(*out) || version != WOC_VERSION) {
        return WOC_ABI;
    }
    memset(out, 0, sizeof(*out));
    out->bytes = sizeof(*out);
    out->version = WOC_VERSION;
    if (!WOC_NATIVE_QUALIFIED) {
        return failure(out, WOC_UNSUPPORTED, 0);
    }
    if (owner == NULL) {
        return failure(out, WOC_OWNERSHIP, 0);
    }
    out->pid = owner->pid;
    out->pgid = owner->pgid;
    out->uid = owner->uid;
    out->ppid = owner->parent;
    out->sid = owner->session;
    out->uid_evidence = WOC_UID_RETAINED;
    out->pid_bytes = sizeof(pid_t);
    out->capacity = 2;
    out->no_signals = owner->no_signals;
    out->reap_attempted = owner->reap_attempted;
    out->reaped = owner->reaped;
    return WOC_OK;
}

static int disposition(woc_record *out)
{
    struct sigaction action;
    memset(&action, 0, sizeof(action));
    if (sigaction(SIGCHLD, NULL, &action) != 0) {
        return failure(out, WOC_SYSTEM, errno);
    }
    out->sigchld_default = action.sa_handler == SIG_DFL;
    out->no_cldwait = (action.sa_flags & SA_NOCLDWAIT) != 0;
    if (!out->sigchld_default || out->no_cldwait) {
        return failure(out, WOC_OWNERSHIP, 0);
    }
    return WOC_OK;
}

static int decode(const siginfo_t *info, woc_record *out)
{
    int64_t kind, returncode;
    int status = info->si_status;
    /* Invalid output records an error without manufacturing an EXITED state.
     * Valid terminal records retain the same legacy 200-byte representation. */
    if (info->si_signo != SIGCHLD) return failure(out, WOC_INCONCLUSIVE, 0);
    if (info->si_code == CLD_EXITED && status >= 0 && status <= 255) {
        kind = WOC_EXIT_NORMAL;
        returncode = status;
    } else if ((info->si_code == CLD_KILLED || info->si_code == CLD_DUMPED) &&
               status > 0 && status < NSIG) {
        kind = info->si_code == CLD_KILLED ? WOC_EXIT_SIGNAL : WOC_EXIT_CORE;
        returncode = -status;
    } else {
        return failure(out, WOC_INCONCLUSIVE, 0);
    }
    out->state = WOC_EXITED;
    out->exit_status = status;
    out->exit_kind = kind;
    out->returncode = returncode;
    return WOC_OK;
}

static int inspect(woc_owner *owner, pid_t pid, int leader, woc_record *out)
{
    struct proc_bsdinfo info;
    memset(&info, 0, sizeof(info));
    errno = 0;
    int bytes = proc_pidinfo(pid, PROC_PIDTBSDINFO, 0, &info, sizeof(info));
    if (bytes != sizeof(info)) {
        return failure(out, bytes <= 0 ? WOC_SYSTEM : WOC_INCONCLUSIVE, errno);
    }
    if (info.pbi_pid != pid || info.pbi_pgid != owner->pgid || info.pbi_uid != owner->uid ||
        (leader && info.pbi_ppid != owner->parent)) {
        return failure(out, WOC_OWNERSHIP, 0);
    }
    pid_t session = getsid(pid);
    if (session == -1) {
        return failure(out, WOC_SYSTEM, errno);
    }
    if (session != owner->session) {
        return failure(out, WOC_OWNERSHIP, 0);
    }
    out->pid = pid;
    out->uid_evidence = WOC_UID_CURRENT;
    if (info.pbi_status == SZOMB) {
        out->state = WOC_EXITED;
    } else if (info.pbi_status == SIDL || info.pbi_status == SRUN ||
               info.pbi_status == SSLEEP || info.pbi_status == SSTOP) {
        out->state = WOC_LIVE;
    } else {
        return failure(out, WOC_INCONCLUSIVE, 0);
    }
    return WOC_OK;
}

static int observe(woc_owner *owner, woc_record *out)
{
    siginfo_t info;
    if (owner->reaped || owner->reap_attempted) {
        return failure(out, WOC_OWNERSHIP, 0);
    }
    if (disposition(out) != WOC_OK) {
        return out->error;
    }
    memset(&info, 0, sizeof(info));
    if (waitid(P_PID, (id_t)owner->pid, &info, WEXITED | WNOWAIT | WNOHANG) != 0) {
        return failure(out, WOC_SYSTEM, errno);
    }
    if (info.si_pid == owner->pid) {
        return decode(&info, out);
    }
    if (info.si_pid != 0) {
        return failure(out, WOC_OWNERSHIP, 0);
    }
    int result = inspect(owner, owner->pid, 1, out);
    if (result == WOC_OK && out->state != WOC_LIVE) {
        return failure(out, WOC_INCONCLUSIVE, 0);
    }
    return result;
}

static int membership(woc_owner *owner, woc_record *out)
{
    owner->count = 0;
    memset(owner->members, 0, sizeof(owner->members));
    if (owner->reaped || owner->reap_attempted || disposition(out) != WOC_OK) {
        return failure(out, out->error ? out->error : WOC_OWNERSHIP, out->system_errno);
    }
    errno = 0;
    int bytes = proc_listpids(PROC_PGRP_ONLY, (uint32_t)owner->pgid,
                             owner->members, sizeof(owner->members));
    out->raw_bytes = bytes;
    if (bytes <= 0) {
        return failure(out, bytes < 0 || errno ? WOC_SYSTEM : WOC_INCONCLUSIVE, errno);
    }
    if (bytes > sizeof(owner->members) || bytes % sizeof(pid_t) != 0) {
        return failure(out, WOC_INCONCLUSIVE, 0);
    }
    owner->count = bytes / sizeof(pid_t);
    out->count = owner->count;
    out->member0 = owner->members[0];
    out->member1 = owner->members[1];
    if (owner->members[0] <= 0 || (owner->count == 2 &&
        (owner->members[1] <= 0 || owner->members[0] == owner->members[1])) ||
        (owner->members[0] != owner->pid && owner->members[1] != owner->pid)) {
        owner->count = 0;
        return failure(out, WOC_INCONCLUSIVE, 0);
    }
    return WOC_OK;
}

static int same_exit(const woc_record *before, const woc_record *after)
{
    return before->state == WOC_EXITED && after->state == WOC_EXITED &&
           before->pid == after->pid && before->pgid == after->pgid &&
           before->exit_kind == after->exit_kind && before->exit_status == after->exit_status;
}

int woc_describe(woc_description *out, size_t bytes, uint32_t version)
{
    if (out == NULL || bytes != sizeof(*out) || version != WOC_VERSION) {
        return WOC_ABI;
    }
    memset(out, 0, sizeof(*out));
    out->bytes = sizeof(*out);
    out->version = WOC_VERSION;
    out->record_bytes = sizeof(woc_record);
    out->pid_bytes = sizeof(pid_t);
    out->platform = 1;
    out->enabled = WOC_NATIVE_QUALIFIED;
    return WOC_OK;
}

int woc_open(int64_t pid, int64_t pgid, int64_t uid, woc_owner **owner,
             woc_record *out, size_t bytes, uint32_t version)
{
    woc_owner candidate;
    int result = begin(NULL, out, bytes, version);
    if (result != WOC_OWNERSHIP) {
        return result;
    }
    if (owner == NULL || *owner != NULL || pid <= 1 || pid > INT_MAX || pgid != pid ||
        uid < 0 || uid > UINT_MAX || (int64_t)(uid_t)uid != uid) {
        return failure(out, WOC_OWNERSHIP, 0);
    }
    memset(&candidate, 0, sizeof(candidate));
    candidate.pid = (pid_t)pid;
    candidate.pgid = (pid_t)pgid;
    candidate.uid = (uid_t)uid;
    candidate.parent = getpid();
    candidate.session = (pid_t)pid;
    begin(&candidate, out, bytes, version);
    result = observe(&candidate, out);
    if (result != WOC_OK || out->state != WOC_LIVE) {
        return failure(out, result ? result : WOC_INCONCLUSIVE, out->system_errno);
    }
    *owner = calloc(1, sizeof(**owner));
    if (*owner == NULL) {
        return failure(out, WOC_SYSTEM, ENOMEM);
    }
    **owner = candidate;
    return WOC_OK;
}

int woc_observe(woc_owner *owner, woc_record *out, size_t bytes, uint32_t version)
{
    int result = begin(owner, out, bytes, version);
    return result ? result : observe(owner, out);
}

int woc_members(woc_owner *owner, woc_record *out, size_t bytes, uint32_t version)
{
    int result = begin(owner, out, bytes, version);
    return result ? result : membership(owner, out);
}

int woc_member(woc_owner *owner, int64_t pid, woc_record *out,
               size_t bytes, uint32_t version)
{
    int result = begin(owner, out, bytes, version);
    if (result != WOC_OK) {
        return result;
    }
    if (owner->no_signals || owner->reap_attempted || pid <= 1 || pid > INT_MAX ||
        pid == owner->pid || owner->count != 2 ||
        (pid != owner->members[0] && pid != owner->members[1])) {
        return failure(out, WOC_OWNERSHIP, 0);
    }
    result = disposition(out);
    return result ? result : inspect(owner, (pid_t)pid, 0, out);
}

int woc_signal(woc_owner *owner, int kind, int64_t positive_member,
               woc_record *out, size_t bytes, uint32_t version)
{
    woc_record before;
    int result = begin(owner, out, bytes, version);
    if (result != WOC_OK) {
        return result;
    }
    if (owner->no_signals || owner->reap_attempted ||
        (kind != WOC_TERM && kind != WOC_KILL)) {
        return failure(out, WOC_OWNERSHIP, 0);
    }
    result = observe(owner, out);
    if (result != WOC_OK) {
        return result;
    }
    before = *out;
    if (before.state == WOC_EXITED) {
        woc_record member;
        result = woc_member(owner, positive_member, &member, sizeof(member), WOC_VERSION);
        if (result != WOC_OK || member.state != WOC_LIVE) {
            return failure(out, result ? result : WOC_INCONCLUSIVE, member.system_errno);
        }
        result = observe(owner, out);
        if (result != WOC_OK || !same_exit(&before, out)) {
            return failure(out, result ? result : WOC_OWNERSHIP, out->system_errno);
        }
    } else if (positive_member != 0) {
        return failure(out, WOC_OWNERSHIP, 0);
    }
    out->signal_attempted = 1;
    if (killpg(owner->pgid, kind == WOC_TERM ? SIGTERM : SIGKILL) != 0) {
        return failure(out, WOC_SYSTEM, errno);
    }
    return WOC_OK;
}

int woc_latch(woc_owner *owner, int64_t exit_kind, int64_t exit_status,
              woc_record *out, size_t bytes, uint32_t version)
{
    woc_record before;
    woc_record members;
    int result = begin(owner, out, bytes, version);
    if (result != WOC_OK) {
        return result;
    }
    if (owner->no_signals || owner->reap_attempted) {
        return failure(out, WOC_OWNERSHIP, 0);
    }
    owner->no_signals = 1;
    out->no_signals = 1;
    result = observe(owner, out);
    if (result != WOC_OK) {
        return result;
    }
    before = *out;
    begin(owner, &members, sizeof(members), WOC_VERSION);
    result = membership(owner, &members);
    if (result != WOC_OK) {
        return failure(out, result, members.system_errno);
    }
    result = observe(owner, out);
    if (result != WOC_OK || !same_exit(&before, out) ||
        before.exit_kind != exit_kind || before.exit_status != exit_status ||
        members.count != 1 || members.raw_bytes != sizeof(pid_t) ||
        members.member0 != owner->pid || members.member1 != 0) {
        return failure(out, result ? result : WOC_INCONCLUSIVE, out->system_errno);
    }
    owner->exit_kind = exit_kind;
    owner->exit_status = exit_status;
    owner->certified = 1;
    return WOC_OK;
}

int woc_reap(woc_owner *owner, woc_record *out, size_t bytes, uint32_t version)
{
    siginfo_t info;
    int result = begin(owner, out, bytes, version);
    if (result != WOC_OK) {
        return result;
    }
    if (!owner->no_signals || !owner->certified || owner->reap_attempted) {
        return failure(out, WOC_OWNERSHIP, 0);
    }
    owner->reap_attempted = 1;
    out->reap_attempted = 1;
    if (disposition(out) != WOC_OK) {
        return out->error;
    }
    memset(&info, 0, sizeof(info));
    if (waitid(P_PID, (id_t)owner->pid, &info, WEXITED | WNOHANG) != 0) {
        return failure(out, WOC_SYSTEM, errno);
    }
    if (info.si_pid != owner->pid) {
        return failure(out, WOC_OWNERSHIP, 0);
    }
    owner->reaped = 1;
    out->reaped = 1;
    result = decode(&info, out);
    if (result != WOC_OK || out->exit_kind != owner->exit_kind ||
        out->exit_status != owner->exit_status) {
        return failure(out, result ? result : WOC_OWNERSHIP, out->system_errno);
    }
    return WOC_OK;
}

int woc_release(woc_owner *owner, woc_record *out, size_t bytes, uint32_t version)
{
    int result = begin(owner, out, bytes, version);
    if (result != WOC_OK) {
        return result;
    }
    if (!owner->reaped || !owner->no_signals) {
        return failure(out, WOC_OWNERSHIP, 0);
    }
    free(owner);
    return WOC_OK;
}


/* SOURCE-ONLY / UNCOMPILED / UNRUN. No installed SDK/kernel/CPython qualification.
 * The Child gate is independent of the legacy PID-adoption API above.
 * Calls below require the extension's main-interpreter GIL serialization. They
 * neither install a SIGCHLD handler nor establish whole-process exclusive wait.
 */
_Static_assert(WOC_NATIVE_QUALIFIED == 0, "Legacy native gate must stay disabled");
_Static_assert(WOC_CHILD_NATIVE_QUALIFIED == 0, "Child native gate must stay disabled");
#if !defined(POSIX_SPAWN_SETSID) || !defined(POSIX_SPAWN_START_SUSPENDED) || \
    !defined(POSIX_SPAWN_CLOEXEC_DEFAULT)
#error "Child candidate requires reviewed SDK-declared Darwin spawn flags"
#endif

enum {
    LC_CLOCK = 1, LC_GETPID, LC_GETUID, LC_DUP, LC_ACTIONS_INIT,
    LC_DUP2_ACTION, LC_CLOSE_ACTION, LC_CHDIR_ACTION, LC_ATTR_INIT,
    LC_FLAGS, LC_SPAWN, LC_SIGCHLD, LC_WAIT_NOWAIT, LC_PROCINFO,
    LC_GETSID, LC_RESUME, LC_CLOSE_FD, LC_ACTIONS_DESTROY, LC_ATTR_DESTROY
};

struct woc_terminal {
    const void *key;
    woc_terminal_report report;
    siginfo_t raw_siginfo;
    siginfo_t registration_raw; /* SDK-typed permanent slot, reserved before resources/spawn */
    siginfo_t abort_raw[WOC_ABORT_RAW_SLOTS]; /* immutable, reserved before spawn */
    struct proc_bsdinfo abort_bsd;
    woc_active_observation active;
    siginfo_t active_raw;
    struct proc_bsdinfo active_bsd;
    int active_exit_saved;
    woc_active_observation first_active_exit;
    siginfo_t first_active_raw;
};

struct woc_child {
    const void *key;
    int busy;
    int invalid;
    woc_owner owner;                 /* allocated before creation; never woc_open */
    woc_child_report report;
    woc_terminal *terminal;          /* separate allocation, native root before spawn */
    char strings[WOC_CHILD_STRING_BYTES];
    char *executable;
    char *cwd;
    char *argv[WOC_CHILD_VECTOR_ITEMS + 1];
    char *env[WOC_CHILD_VECTOR_ITEMS + 1];
    int stdio[3];
    int actions_initialized;
    int attr_initialized;
    posix_spawn_file_actions_t actions;
    posix_spawnattr_t attr;
    struct proc_bsdinfo registration_info;
    pid_t spawn_pid;                /* posix_spawn writes directly into rooted storage */
    uint64_t cleanup_timeout_ns;
    uint64_t grace_ns;
};

/* Child-only envelope checks. Every wait UID is diagnostic, never identity.
 * These helpers perform no decode, record mutation or native API call. */
static int child_stopped_unresumed(const woc_child *c, const siginfo_t *info)
{
    return !c->report.resume_entered && c->owner.pid > 1 &&
           info->si_pid == c->owner.pid && info->si_signo == SIGCHLD &&
           info->si_code == CLD_STOPPED && info->si_status == 0;
}

static int child_terminal_envelope(const woc_child *c, const siginfo_t *info)
{
    if (c->owner.pid <= 1 || info->si_pid != c->owner.pid ||
        info->si_signo != SIGCHLD) return 0;
    return (info->si_code == CLD_EXITED && info->si_status >= 0 && info->si_status <= 255) ||
           ((info->si_code == CLD_KILLED || info->si_code == CLD_DUMPED) &&
            info->si_status > 0 && info->si_status < NSIG);
}

static woc_child *child_root;
static woc_terminal *terminal_roots[WOC_CHILD_HISTORY];
static size_t terminal_count;
static uint64_t next_child_generation = 1;
static int child_registry_ever_used;

static int terminal_exact(woc_terminal *t, const void *key, uint64_t generation);

/* Called only after an exact native root is established. Preserve the first
 * failure tuple BEFORE any state collapse; later failures only add cause bits. */
static void launch_failure(woc_child *c, int reason, int rc, int error)
{
    if (c->report.launch_failure_mask == 0) {
        c->report.launch_first_failure_phase = c->report.state;
        c->report.launch_first_failure_reason = reason;
        c->report.launch_first_woc_rc = rc;
        c->report.launch_first_system_errno = error;
    }
    c->report.launch_failure_mask |= WOC_LF_MASK(reason);
}

static int child_failure(woc_child *c, int rc, int error, int block)
{
    c->report.rc = rc;
    c->report.system_errno = error;
    if (block) {
        c->report.blocked = 1;
        c->report.state = WOC_CHILD_BLOCKED;
    }
    return rc;
}

static int launch_rejection(woc_child *c)
{
    launch_failure(c, WOC_LF_PUBLICATION_OR_OWNERSHIP, WOC_OWNERSHIP, 0);
    return WOC_OWNERSHIP;
}

static int child_exact(woc_child *c, const void *key, uint64_t generation)
{
    return c != NULL && c == child_root &&
           !c->invalid && c->key == key && generation != 0 &&
           c->report.generation == generation && !c->busy;
}

static int child_valid(woc_child *c, const void *key, uint64_t generation)
{
    return WOC_CHILD_NATIVE_QUALIFIED && child_exact(c, key, generation);
}

static int child_normal_mode(const woc_child *c)
{
    return !c->report.abort.started;
}

/* Persistent, memory-only creation/registration provenance. This is deliberately
 * independent of busy and consuming phase: it applies inside admitted entries
 * and after the one consuming attempt. Wait UID cannot establish this premise.
 * Exclusive waiting, creator liveness and continuous SIGCHLD policy are still
 * external qualifications; these native roots do not establish them. */
static int child_registered_provenance(const woc_child *c)
{
    if (c == NULL || c != child_root || c->invalid || c->key == NULL ||
        c->report.generation == 0) return 0;
    const woc_child_report *r = &c->report;
    return r->spawn_entered && r->spawn_returned && r->spawn_rc == 0 &&
           r->pid_valid && r->registered && c->spawn_pid > 1 &&
           r->spawn_pid_raw == c->spawn_pid && c->owner.pid == c->spawn_pid &&
           c->owner.parent > 1 && c->owner.pgid == c->owner.pid &&
           c->owner.session == c->owner.pid &&
           terminal_exact(c->terminal, c->key, r->generation) &&
           !c->terminal->report.transferred && !c->terminal->report.known_no_creation &&
           c->terminal->report.raw_siginfo_bytes == sizeof(c->terminal->raw_siginfo) &&
           c->registration_info.pbi_pid == c->owner.pid &&
           c->registration_info.pbi_ppid == c->owner.parent &&
           c->registration_info.pbi_uid == c->owner.uid &&
           c->registration_info.pbi_pgid == c->owner.pid &&
           c->registration_info.pbi_status == SSTOP &&
           r->registration_pid == c->owner.pid && r->registration_ppid == c->owner.parent &&
           r->registration_uid == c->owner.uid && r->registration_pgid == c->owner.pid &&
           r->registration_sid == c->owner.pid && r->registration_bsd_status == SSTOP;
}

/* Pre-consume phase checks stay separate from persistent provenance. */
static int child_wait_unattempted(const woc_child *c)
{
    return !c->owner.reap_attempted && !c->owner.reaped &&
           !c->terminal->report.reap_attempted && !c->terminal->report.wait_entered;
}

static void child_terminal_uid(const woc_child *c, woc_record *out)
{
    out->uid = c->owner.uid;
    out->wait_uid_available = 0;
    out->uid_evidence = WOC_UID_RETAINED;
}

/* Child adapters preserve the shared legacy validator/observer and all fresh
 * inspect UID/SID checks. In particular a later terminal result cannot retain
 * CURRENT evidence from an earlier live BSD query in the same record. */
static int child_decode(const woc_child *c, const siginfo_t *info, woc_record *out)
{
    if (!child_registered_provenance(c) || !child_terminal_envelope(c, info))
        return failure(out, WOC_OWNERSHIP, 0);
    int rc = decode(info, out);
    if (rc == WOC_OK) child_terminal_uid(c, out);
    return rc;
}

static int child_observe(woc_child *c, woc_record *out)
{
    if (!child_registered_provenance(c) || !child_wait_unattempted(c))
        return failure(out, WOC_OWNERSHIP, 0);
    int rc = observe(&c->owner, out);
    if (rc == WOC_OK && out->state == WOC_EXITED) child_terminal_uid(c, out);
    return rc;
}

/* A retained terminal record must describe the registered owner and the exact
 * authoritative terminal fields of its own raw observation. */
static int child_terminal_record_matches(const woc_child *c, const siginfo_t *info,
                                          const woc_record *out)
{
    woc_record decoded = {0};
    if (child_decode(c, info, &decoded) != WOC_OK) return 0;
    return out->bytes == sizeof(*out) && out->version == WOC_VERSION &&
           out->error == WOC_OK && out->pid == c->owner.pid &&
           out->pgid == c->owner.pgid && out->uid == c->owner.uid &&
           out->ppid == c->owner.parent && out->sid == c->owner.session &&
           out->wait_uid_available == 0 && out->uid_evidence == WOC_UID_RETAINED &&
           out->state == WOC_EXITED && out->exit_kind == decoded.exit_kind &&
           out->exit_status == decoded.exit_status && out->returncode == decoded.returncode;
}

/* This trace counts exactly the named libc/public API invocations below, NOT
 * kernel syscalls hidden inside those APIs. No hard syscall latency is claimed.
 * Fixed preparation/registration/resume/resource paths need fewer than 64 slots;
 * admission fails before an unrecordable call. Resource failures remain rooted.
 */
static woc_child_call_fact *launch_fact(woc_child *c, int kind)
{
    if (c->report.launch_fact_count >= WOC_CHILD_LAUNCH_FACTS) {
        launch_failure(c, WOC_LF_TRACE_CAPACITY, WOC_INCONCLUSIVE, 0);
        child_failure(c, WOC_INCONCLUSIVE, 0, 1);
        return NULL;
    }
    woc_child_call_fact *f = &c->report.launch_facts[c->report.launch_fact_count++];
    f->kind = kind;
    f->entered = 1;
    return f;
}

static void launch_return(woc_child_call_fact *f, int64_t rc, int error)
{
    f->raw_rc = rc;
    f->raw_errno = error;
    f->returned = 1;
}

static int ns_value(const struct timespec *ts, uint64_t *out)
{
    if (ts->tv_sec < 0 || ts->tv_nsec < 0 || ts->tv_nsec >= 1000000000L ||
        (uint64_t)ts->tv_sec > (UINT64_MAX - (uint64_t)ts->tv_nsec) / 1000000000ULL) {
        return 0;
    }
    *out = (uint64_t)ts->tv_sec * 1000000000ULL + (uint64_t)ts->tv_nsec;
    return 1;
}

static int launch_clock(woc_child *c, uint64_t *now, int reason)
{
    struct timespec ts = {0, 0};
    woc_child_call_fact *f = launch_fact(c, LC_CLOCK);
    if (f == NULL) return 0;
    errno = 0;
    c->report.clock_rc = clock_gettime(CLOCK_MONOTONIC, &ts);
    c->report.clock_errno = errno;
    launch_return(f, c->report.clock_rc, c->report.clock_errno);
    if (c->report.clock_rc != 0 || !ns_value(&ts, now)) {
        launch_failure(c, reason, c->report.clock_rc ? WOC_SYSTEM : WOC_INCONCLUSIVE,
                       c->report.clock_rc ? c->report.clock_errno : 0);
        child_failure(c, c->report.clock_rc ? WOC_SYSTEM : WOC_INCONCLUSIVE,
                      c->report.clock_rc ? c->report.clock_errno : 0, 1);
        return 0;
    }
    return 1;
}

static int launch_in_time(woc_child *c, int clock_reason, int deadline_reason)
{
    uint64_t now;
    if (!launch_clock(c, &now, clock_reason)) return 0;
    if (now >= c->report.launch_deadline_ns) {
        launch_failure(c, deadline_reason, WOC_INCONCLUSIVE, 0);
        child_failure(c, WOC_INCONCLUSIVE, 0, 1);
        return 0;
    }
    return 1;
}

static int valid_string(woc_child_string s, size_t *total, int absolute)
{
    if (s.data == NULL || s.bytes == 0 || s.bytes >= WOC_CHILD_STRING_BYTES ||
        memchr(s.data, 0, s.bytes) != NULL || (absolute && s.data[0] != '/') ||
        *total > WOC_CHILD_STRING_BYTES - s.bytes - 1) return 0;
    *total += s.bytes + 1;
    return 1;
}

static size_t env_name_bytes(woc_child_string s)
{
    size_t i;
    for (i = 0; i < s.bytes && s.data[i] != '='; ++i) {
        unsigned char ch = (unsigned char)s.data[i];
        if (!((ch >= 'A' && ch <= 'Z') || (ch >= 'a' && ch <= 'z') ||
              ch == '_' || (i != 0 && ch >= '0' && ch <= '9'))) return 0;
    }
    return i != 0 && i < s.bytes ? i : 0;
}

static int valid_spec(const woc_child_spec *s)
{
    size_t total = 0;
    if (s == NULL || s->argc == 0 || s->argc > WOC_CHILD_VECTOR_ITEMS ||
        s->envc > WOC_CHILD_VECTOR_ITEMS || s->argv == NULL ||
        (s->envc != 0 && s->env == NULL) ||
        !valid_string(s->executable, &total, 1) || !valid_string(s->cwd, &total, 1) ||
        s->launch_timeout_ns == 0 || s->launch_timeout_ns > 60000000000ULL ||
        s->cleanup_timeout_ns == 0 || s->cleanup_timeout_ns > 60000000000ULL ||
        s->grace_ns > s->cleanup_timeout_ns ||
        s->max_cleanup_entries < 1 || s->max_cleanup_entries > 512) return 0;
    for (size_t i = 0; i < 3; ++i) if (s->stdio[i] < 0) return 0;
    for (size_t i = 0; i < s->argc; ++i) {
        if (!valid_string(s->argv[i], &total, 0)) return 0;
    }
    if (s->argv[0].bytes != s->executable.bytes ||
        memcmp(s->argv[0].data, s->executable.data, s->executable.bytes) != 0) return 0;
    for (size_t i = 0; i < s->envc; ++i) {
        if (!valid_string(s->env[i], &total, 0)) return 0;
        size_t n = env_name_bytes(s->env[i]);
        if (!n) return 0;
        for (size_t j = 0; j < i; ++j) {
            if (env_name_bytes(s->env[j]) == n && memcmp(s->env[i].data, s->env[j].data, n) == 0)
                return 0;
        }
    }
    return 1;
}

static char *copy_string(woc_child *c, size_t *offset, woc_child_string s)
{
    char *p = c->strings + *offset;
    memcpy(p, s.data, s.bytes);
    p[s.bytes] = 0;
    *offset += s.bytes + 1;
    return p;
}

static void close_launch_resources(woc_child *c)
{
    if (c->report.resources_closed) return;
    for (size_t i = 0; i < 3; ++i) {
        if (c->stdio[i] >= 0) {
            woc_child_call_fact *f = launch_fact(c, LC_CLOSE_FD);
            if (f == NULL) { c->report.resource_error = 1; continue; }
            errno = 0;
            int rc = close(c->stdio[i]);
            int error = errno;
            launch_return(f, rc, error);
            /* EINTR is not retried: descriptor lifetime on failure is unresolved.
             * The raw error is sticky and prevents operational release. */
            c->stdio[i] = -1;
            if (rc != 0) {
                c->report.resource_error = 1;
                launch_failure(c, WOC_LF_RESOURCE_ERROR, WOC_SYSTEM, error);
            }
        }
    }
    if (c->actions_initialized) {
        woc_child_call_fact *f = launch_fact(c, LC_ACTIONS_DESTROY);
        if (f == NULL) c->report.resource_error = 1;
        else {
            errno = 0;
            int rc = posix_spawn_file_actions_destroy(&c->actions);
            int error = errno;
            launch_return(f, rc, error);
            c->actions_initialized = 0;
            if (rc != 0) { c->report.resource_error = 1;
                launch_failure(c, WOC_LF_RESOURCE_ERROR, WOC_SYSTEM, rc); }
        }
    }
    if (c->attr_initialized) {
        woc_child_call_fact *f = launch_fact(c, LC_ATTR_DESTROY);
        if (f == NULL) c->report.resource_error = 1;
        else {
            errno = 0;
            int rc = posix_spawnattr_destroy(&c->attr);
            int error = errno;
            launch_return(f, rc, error);
            c->attr_initialized = 0;
            if (rc != 0) { c->report.resource_error = 1;
                launch_failure(c, WOC_LF_RESOURCE_ERROR, WOC_SYSTEM, rc); }
        }
    }
    c->report.resources_closed = !c->actions_initialized && !c->attr_initialized &&
                                 c->stdio[0] < 0 && c->stdio[1] < 0 && c->stdio[2] < 0;
}

static int abort_precreation(woc_child *c, int rc, int error)
{
    if (c->report.launch_failure_mask == 0)
        launch_failure(c, WOC_LF_PREPARATION, rc, error);
    close_launch_resources(c);
    if (c->report.resource_error) launch_failure(c, WOC_LF_RESOURCE_ERROR, WOC_SYSTEM, 0);
    c->terminal->report.known_no_creation = !c->report.spawn_entered;
    if (c->terminal->report.known_no_creation)
        c->terminal->report.kind = WOC_TERMINAL_NO_CREATION;
    c->report.state = WOC_CHILD_ABORTED;
    c->report.blocked = c->report.resource_error;
    c->report.rc = rc;
    c->report.system_errno = error;
    return rc;
}

int woc_child_registry_used(void) { return child_registry_ever_used; }

int woc_child_prepare(const void *key, const woc_child_spec *s, woc_child **out)
{
    if (!WOC_CHILD_NATIVE_QUALIFIED) return WOC_UNSUPPORTED;
    if (key == NULL || out == NULL || *out != NULL || child_root != NULL ||
        terminal_count >= WOC_CHILD_HISTORY || next_child_generation == UINT64_MAX ||
        !valid_spec(s)) return WOC_OWNERSHIP;
    woc_child *c = calloc(1, sizeof(*c));
    woc_terminal *t = calloc(1, sizeof(*t));
    if (c == NULL || t == NULL) { free(c); free(t); return WOC_SYSTEM; }
    c->key = key;
    c->terminal = t;
    c->report.generation = next_child_generation++;
    c->report.state = WOC_CHILD_PREPARED;
    c->report.launch_entries = 1; /* prepare, spawn, resume: separate finite entries */
    c->report.max_cleanup_entries = s->max_cleanup_entries;
    c->report.abort.max_entries = WOC_ABORT_ENTRIES;
    c->report.abort.max_observations = WOC_ABORT_OBSERVATIONS;
    c->report.abort.exit_observation_index = -1;
    c->cleanup_timeout_ns = s->cleanup_timeout_ns;
    c->grace_ns = s->grace_ns;
    for (size_t i = 0; i < 3; ++i) c->stdio[i] = -1;
    t->key = key;
    t->report.generation = c->report.generation;
    t->report.rooted = 1;
    t->report.raw_siginfo_bytes = sizeof(t->raw_siginfo);
    terminal_roots[terminal_count++] = t; /* native strong root, never reset */
    child_root = c;                       /* before ALL resource/creation APIs */
    child_registry_ever_used = 1;
    *out = c;                            /* output points at already rooted attempt */
    size_t offset = 0;
    c->executable = copy_string(c, &offset, s->executable);
    c->cwd = copy_string(c, &offset, s->cwd);
    for (size_t i = 0; i < s->argc; ++i) c->argv[i] = copy_string(c, &offset, s->argv[i]);
    for (size_t i = 0; i < s->envc; ++i) c->env[i] = copy_string(c, &offset, s->env[i]);
    uint64_t now;
    if (!launch_clock(c, &now, WOC_LF_PREPARATION))
        return abort_precreation(c, c->report.rc, c->report.system_errno);
    if (now > UINT64_MAX - s->launch_timeout_ns ||
        now + s->launch_timeout_ns > UINT64_MAX - s->cleanup_timeout_ns) {
        launch_failure(c, WOC_LF_PREPARATION, WOC_INCONCLUSIVE, 0);
        return abort_precreation(c, WOC_INCONCLUSIVE, 0);
    }
    c->report.launch_deadline_ns = now + s->launch_timeout_ns;
    c->report.abort.deadline_ns = c->report.launch_deadline_ns + s->cleanup_timeout_ns;
    c->report.abort.origin_ns = now; c->report.abort.now_ns = now;
    woc_child_call_fact *f = launch_fact(c, LC_GETPID);
    if (!f) return abort_precreation(c, WOC_INCONCLUSIVE, 0);
    errno = 0; c->owner.parent = getpid(); launch_return(f, c->owner.parent, errno);
    f = launch_fact(c, LC_GETUID);
    if (!f) return abort_precreation(c, WOC_INCONCLUSIVE, 0);
    errno = 0; c->owner.uid = getuid(); launch_return(f, c->owner.uid, errno);
    if (c->owner.parent <= 1) return abort_precreation(c, WOC_OWNERSHIP, 0);
    for (size_t i = 0; i < 3; ++i) {
        f = launch_fact(c, LC_DUP);
        if (!f) return abort_precreation(c, WOC_INCONCLUSIVE, 0);
        errno = 0;
        c->stdio[i] = fcntl(s->stdio[i], F_DUPFD_CLOEXEC, 3);
        int error = errno;
        launch_return(f, c->stdio[i], error);
        if (c->stdio[i] < 3) return abort_precreation(c, WOC_SYSTEM, error);
    }
#define LAUNCH_SETUP(kind, expression) do { \
    f = launch_fact(c, kind); \
    if (!f) return abort_precreation(c, WOC_INCONCLUSIVE, 0); \
    errno = 0; int setup_rc = (expression); int setup_errno = errno; \
    launch_return(f, setup_rc, setup_errno); \
    if (setup_rc != 0) { \
        if ((kind) == LC_ACTIONS_INIT || (kind) == LC_ATTR_INIT) c->report.resource_error = 1; \
        return abort_precreation(c, WOC_SYSTEM, setup_rc); \
    } \
} while (0)
    LAUNCH_SETUP(LC_ACTIONS_INIT, posix_spawn_file_actions_init(&c->actions));
    c->actions_initialized = 1;
    for (size_t i = 0; i < 3; ++i) {
        LAUNCH_SETUP(LC_DUP2_ACTION, posix_spawn_file_actions_adddup2(&c->actions, c->stdio[i], (int)i));
        LAUNCH_SETUP(LC_CLOSE_ACTION, posix_spawn_file_actions_addclose(&c->actions, c->stdio[i]));
    }
    LAUNCH_SETUP(LC_CHDIR_ACTION, posix_spawn_file_actions_addchdir_np(&c->actions, c->cwd));
    LAUNCH_SETUP(LC_ATTR_INIT, posix_spawnattr_init(&c->attr));
    c->attr_initialized = 1;
    /* SDK names only: never copied numeric flags, no implicit environment/path search.
     * CLOEXEC_DEFAULT plus explicit dup2 actions is the intended descriptor policy;
     * its exact installed behavior, including races outside this module, is UNQUALIFIED.
     */
    LAUNCH_SETUP(LC_FLAGS, posix_spawnattr_setflags(&c->attr,
        (short)(POSIX_SPAWN_SETSID | POSIX_SPAWN_START_SUSPENDED | POSIX_SPAWN_CLOEXEC_DEFAULT)));
#undef LAUNCH_SETUP
    if (!launch_in_time(c, WOC_LF_PRESPAWN_CLOCK, WOC_LF_PRESPAWN_DEADLINE))
        return abort_precreation(c, c->report.rc, c->report.system_errno);
    return WOC_OK;
}

/* Reject a disposition already known to be unsupported before creation. This
 * read-only query does not install policy, prove an exclusive waiter, or prevent
 * another process thread from changing SIGCHLD before/after spawn. Registration
 * therefore retains its separate post-spawn disposition recheck below.
 */
static int sigchld_preflight(woc_child *c)
{
    struct sigaction action;
    memset(&action, 0, sizeof(action));
    woc_child_call_fact *f = launch_fact(c, LC_SIGCHLD);
    if (!f) return WOC_INCONCLUSIVE;
    c->report.sigchld_preflight_entered = 1;
    errno = 0;
    c->report.sigchld_preflight_rc = sigaction(SIGCHLD, NULL, &action);
    c->report.sigchld_preflight_errno = errno;
    c->report.sigchld_preflight_returned = 1;
    launch_return(f, c->report.sigchld_preflight_rc, c->report.sigchld_preflight_errno);
    /* Retain raw return/errno before interpreting the SDK-defined disposition.
     * A failed query leaves policy_valid/accepted false; no output is decoded. */
    if (c->report.sigchld_preflight_rc != 0) return WOC_SYSTEM;
    c->report.sigchld_preflight_default = action.sa_handler == SIG_DFL;
    c->report.sigchld_preflight_no_cldwait = (action.sa_flags & SA_NOCLDWAIT) != 0;
    c->report.sigchld_preflight_policy_valid = 1;
    c->report.sigchld_preflight_accepted = c->report.sigchld_preflight_default &&
                                           !c->report.sigchld_preflight_no_cldwait;
    if (!c->report.sigchld_preflight_accepted) return WOC_OWNERSHIP;
    return WOC_OK;
}

static void child_record(woc_child *c)
{
    woc_record *r = &c->report.record;
    memset(r, 0, sizeof(*r));
    r->bytes = sizeof(*r); r->version = WOC_VERSION;
    r->pid = c->owner.pid; r->pgid = c->owner.pgid; r->uid = c->owner.uid;
    r->ppid = c->owner.parent; r->sid = c->owner.session;
    r->uid_evidence = c->report.registered ? WOC_UID_RETAINED : 0;
    r->wait_uid_available = 0; r->pid_bytes = sizeof(pid_t); r->capacity = 2;
    r->no_signals = c->owner.no_signals; r->reap_attempted = c->owner.reap_attempted;
    r->reaped = c->owner.reaped;
}

static int registration_failure(woc_child *c, int reason, int rc, int error)
{
    launch_failure(c, reason, rc, error);
    return failure(&c->report.record, rc, error);
}

static int register_suspended(woc_child *c)
{
    struct sigaction action;
    woc_terminal *t = c->terminal;
    siginfo_t *info = &t->registration_raw;
    woc_registration_observation *o = &t->report.registration_observation;
    struct proc_bsdinfo *bsd = &c->registration_info;
    memset(&action, 0, sizeof(action)); memset(bsd, 0, sizeof(*bsd));
    woc_record *r = &c->report.record;
    woc_child_call_fact *f = launch_fact(c, LC_SIGCHLD);
    if (!f) return WOC_INCONCLUSIVE;
    errno = 0; int rc = sigaction(SIGCHLD, NULL, &action); int error = errno;
    launch_return(f, rc, error);
    if (rc) return registration_failure(c, WOC_LF_REGISTRATION_QUERY, WOC_SYSTEM, error);
    r->sigchld_default = action.sa_handler == SIG_DFL;
    r->no_cldwait = (action.sa_flags & SA_NOCLDWAIT) != 0;
    if (!r->sigchld_default || r->no_cldwait) return registration_failure(c, WOC_LF_REGISTRATION_POLICY, WOC_OWNERSHIP, 0);
    f = launch_fact(c, LC_WAIT_NOWAIT); if (!f) return WOC_INCONCLUSIVE;
    /* Single registration wait: write directly into the permanent native root.
     * Preserve even failed-call output before classification, ledger updates or
     * any Python allocation. A completed slot has no subsequent writer. */
    memset(info, 0, sizeof(*info));
    o->idtype = P_PID; o->target_pid = c->owner.pid;
    o->options = WEXITED | WSTOPPED | WNOWAIT | WNOHANG;
    o->raw_siginfo_bytes = sizeof(*info);
    o->entered = 1;
    c->report.registration_observation = *o;
    errno = 0;
    o->rc = waitid(P_PID, (id_t)c->owner.pid, info, WEXITED | WSTOPPED | WNOWAIT | WNOHANG);
    o->system_errno = errno;
    o->signo = info->si_signo; o->code = info->si_code;
    o->pid = info->si_pid; o->uid = info->si_uid; o->status = info->si_status;
    o->returned = 1;
    c->report.registration_observation = *o;
    rc = o->rc; error = o->system_errno;
    launch_return(f, rc, error);
    if (rc) return registration_failure(c, WOC_LF_REGISTRATION_QUERY, WOC_SYSTEM, error);
    if (info->si_pid != 0 && !child_stopped_unresumed(c, info))
        return registration_failure(c, WOC_LF_REGISTRATION_IDENTITY, WOC_OWNERSHIP, 0);
    f = launch_fact(c, LC_PROCINFO); if (!f) return WOC_INCONCLUSIVE;
    errno = 0;
    rc = proc_pidinfo(c->owner.pid, PROC_PIDTBSDINFO, 0, bsd, sizeof(*bsd)); error = errno;
    launch_return(f, rc, error);
    if (rc != sizeof(*bsd)) return registration_failure(c, WOC_LF_REGISTRATION_QUERY, rc <= 0 ? WOC_SYSTEM : WOC_INCONCLUSIVE,
                                         rc <= 0 ? error : 0);
    f = launch_fact(c, LC_GETSID); if (!f) return WOC_INCONCLUSIVE;
    errno = 0; pid_t session = getsid(c->owner.pid); error = errno;
    launch_return(f, session, error);
    if (session < 0) return registration_failure(c, WOC_LF_REGISTRATION_QUERY, WOC_SYSTEM, error);
    c->report.registration_bsd_status = bsd->pbi_status;
    c->report.registration_pid = bsd->pbi_pid; c->report.registration_ppid = bsd->pbi_ppid;
    c->report.registration_uid = bsd->pbi_uid; c->report.registration_pgid = bsd->pbi_pgid;
    c->report.registration_sid = session;
    if (bsd->pbi_pid != c->owner.pid || bsd->pbi_ppid != c->owner.parent ||
        bsd->pbi_uid != c->owner.uid || bsd->pbi_pgid != c->owner.pid ||
        session != c->owner.pid || bsd->pbi_status != SSTOP)
        return registration_failure(c, WOC_LF_REGISTRATION_IDENTITY, WOC_OWNERSHIP, 0);
    r->uid_evidence = WOC_UID_CURRENT; r->state = WOC_LIVE;
    c->report.registered = 1;
    return WOC_OK;
}

int woc_child_spawn(woc_child *c, const void *key, uint64_t generation)
{
    if (!child_valid(c, key, generation)) return WOC_OWNERSHIP;
    if (!child_normal_mode(c)) return WOC_OWNERSHIP;
    if (c->report.blocked || c->report.state != WOC_CHILD_PREPARED ||
        c->report.launch_entries != 1 || c->report.spawn_entered) return launch_rejection(c);
    c->busy = 1;
    c->report.launch_entries++;
    if (!launch_in_time(c, WOC_LF_PRESPAWN_CLOCK, WOC_LF_PRESPAWN_DEADLINE)) {
        int rc = abort_precreation(c, c->report.rc, c->report.system_errno);
        c->busy = 0; return rc;
    }
    int preflight_rc = sigchld_preflight(c);
    if (preflight_rc != WOC_OK) {
        launch_failure(c, WOC_LF_PREFLIGHT_DISPOSITION, preflight_rc,
                       preflight_rc == WOC_SYSTEM ? c->report.sigchld_preflight_errno : 0);
        int rc = abort_precreation(c, preflight_rc,
            preflight_rc == WOC_SYSTEM ? c->report.sigchld_preflight_errno : 0);
        c->busy = 0; return rc;
    }
    /* The disposition query may have consumed the remaining launch time. */
    if (!launch_in_time(c, WOC_LF_PRESPAWN_CLOCK, WOC_LF_PRESPAWN_DEADLINE)) {
        int rc = abort_precreation(c, c->report.rc, c->report.system_errno);
        c->busy = 0; return rc;
    }
    woc_child_call_fact *f = launch_fact(c, LC_SPAWN);
    if (!f) {
        int rc = abort_precreation(c, WOC_INCONCLUSIVE, 0);
        c->busy = 0; return rc;
    }
    c->report.spawn_entered = 1;
    c->report.state = WOC_CHILD_SPAWN_ENTERED; /* entered/not returned is retained unknown */
    errno = 0;
    c->report.spawn_rc = posix_spawn(&c->spawn_pid, c->executable, &c->actions,
                                    &c->attr, c->argv, c->env);
    c->report.spawn_errno = errno;
    c->report.spawn_returned = 1;
    c->report.spawn_pid_raw = c->spawn_pid;
    c->report.pid_valid = c->report.spawn_rc == 0 && c->spawn_pid > 1;
    launch_return(f, c->report.spawn_rc, c->report.spawn_errno);
    /* No Python/API callbacks, allocation or signal checks before the above facts.
     * Nonzero-return PID is unspecified, never installed as owner identity. Even
     * upstream no-child semantics do not qualify this installed platform.
     */
    if (!c->report.pid_valid)
        launch_failure(c, WOC_LF_SPAWN_UNKNOWN, c->report.spawn_rc ? WOC_SYSTEM : WOC_OWNERSHIP,
                       c->report.spawn_rc > 0 ? c->report.spawn_rc : 0);
    close_launch_resources(c);
    if (!c->report.pid_valid) {
        launch_failure(c, WOC_LF_SPAWN_UNKNOWN, c->report.spawn_rc ? WOC_SYSTEM : WOC_OWNERSHIP,
                       c->report.spawn_rc > 0 ? c->report.spawn_rc : 0);
        c->report.state = WOC_CHILD_SPAWN_UNKNOWN;
        c->report.blocked = 1;
        c->report.rc = c->report.spawn_rc ? WOC_SYSTEM : WOC_OWNERSHIP;
        c->report.system_errno = c->report.spawn_rc > 0 ? c->report.spawn_rc : 0;
        c->busy = 0; return c->report.rc;
    }
    c->owner.pid = c->spawn_pid; c->owner.pgid = c->spawn_pid; c->owner.session = c->spawn_pid;
    child_record(c);
    int rc = register_suspended(c);
    if (rc != WOC_OK || c->report.resource_error) {
        if (c->report.resource_error) launch_failure(c, WOC_LF_RESOURCE_ERROR, WOC_SYSTEM, 0);
        child_failure(c, rc ? rc : WOC_SYSTEM, c->report.record.system_errno, 1);
    } else c->report.state = WOC_CHILD_SUSPENDED;
    c->busy = 0;
    return c->report.rc;
}

int woc_child_resume(woc_child *c, const void *key, uint64_t generation)
{
    if (!child_valid(c, key, generation)) return WOC_OWNERSHIP;
    if (!child_normal_mode(c)) return WOC_OWNERSHIP;
    if (!child_registered_provenance(c) || c->report.blocked ||
        c->report.state != WOC_CHILD_SUSPENDED ||
        !c->report.registered || !c->report.pid_valid || c->report.resume_entered ||
        c->report.cleanup_started || c->report.launch_entries != 2) return launch_rejection(c);
    c->busy = 1; c->report.launch_entries++;
    if (!launch_in_time(c, WOC_LF_PRE_RESUME_CLOCK, WOC_LF_PRE_RESUME_DEADLINE)) { c->busy = 0; return c->report.rc; }
    woc_child_call_fact *f = launch_fact(c, LC_RESUME);
    if (!f) { c->busy = 0; return WOC_INCONCLUSIVE; }
    c->report.resume_entered = 1;
    c->report.state = WOC_CHILD_RESUME_ENTERED;
    errno = 0;
    c->report.resume_rc = kill(c->owner.pid, SIGCONT); /* direct child; never cleanup enum */
    c->report.resume_errno = errno;
    c->report.resume_returned = 1;
    launch_return(f, c->report.resume_rc, c->report.resume_errno);
    if (c->report.resume_rc != 0) {
        launch_failure(c, WOC_LF_RESUME_ENTERED_FAILURE, WOC_SYSTEM, c->report.resume_errno);
        child_failure(c, WOC_SYSTEM, c->report.resume_errno, 1);
    }
    else c->report.state = WOC_CHILD_ACTIVE;
    c->busy = 0;
    return c->report.rc;
}

int woc_child_publish(woc_child *c, const void *key, uint64_t generation)
{
    if (!child_valid(c, key, generation)) return WOC_OWNERSHIP;
    if (!child_normal_mode(c)) return WOC_OWNERSHIP;
    if (!child_registered_provenance(c) || c->report.blocked ||
        c->report.state != WOC_CHILD_ACTIVE || c->report.published)
        return launch_rejection(c);
    c->report.published = 1;
    return WOC_OK;
}

static int active_finish(woc_child *c, int rc, int error, int saw_exit)
{
    woc_terminal *t = c->terminal;
    woc_active_observation *a = &t->active;
    struct timespec time = {0, 0};
    errno = 0;
    a->finish_clock_rc = clock_gettime(CLOCK_MONOTONIC, &time);
    a->finish_clock_errno = errno;
    if (a->finish_clock_rc != 0 || !ns_value(&time, &a->finished_ns) ||
        a->finished_ns < a->started_ns) {
        if (rc == WOC_OK) {
            rc = a->finish_clock_rc ? WOC_SYSTEM : WOC_INCONCLUSIVE;
            error = a->finish_clock_rc ? a->finish_clock_errno : 0;
        }
    }
    a->rc = rc; a->system_errno = error; a->returned = 1;
    if (rc != WOC_OK) {
        failure(&a->record, rc, error);
        /* An entered observation failed its ownership/profile checks. Retain
         * all native roots; a later good-looking snapshot cannot erase it. */
        child_failure(c, rc, error, 1);
    }
    if (saw_exit && !t->active_exit_saved) {
        t->active_exit_saved = 1;
        t->first_active_exit = *a;
        t->first_active_raw = t->active_raw;
    }
    c->busy = 0;
    return rc;
}

int woc_child_active_observe(woc_child *c, const void *key, uint64_t generation)
{
    if (!WOC_CHILD_NATIVE_QUALIFIED) return WOC_UNSUPPORTED;
    if (!child_valid(c, key, generation) || !child_normal_mode(c)) return WOC_OWNERSHIP;
    if (!child_registered_provenance(c) || c->report.blocked ||
        c->report.state != WOC_CHILD_ACTIVE || !c->report.published ||
        !c->report.resume_entered || !c->report.resume_returned || c->report.resume_rc != 0 ||
        c->report.cleanup_started || !child_wait_unattempted(c) || c->owner.no_signals ||
        c->registration_info.pbi_start_tvsec == 0 || c->registration_info.pbi_start_tvusec >= 1000000)
        return WOC_OWNERSHIP;
    woc_terminal *t = c->terminal;
    if (t->active.sequence >= WOC_ACTIVE_OBSERVATIONS) return WOC_INCONCLUSIVE;
    uint64_t sequence = t->active.sequence + 1;
    c->busy = 1;
    memset(&t->active, 0, sizeof(t->active));
    memset(&t->active_raw, 0, sizeof(t->active_raw));
    memset(&t->active_bsd, 0, sizeof(t->active_bsd));
    woc_active_observation *a = &t->active;
    a->bytes = sizeof(*a); a->version = WOC_ACTIVE_VERSION;
    a->generation = generation; a->sequence = sequence; a->entered = 1;
    a->birth_seconds = c->registration_info.pbi_start_tvsec;
    a->birth_microseconds = c->registration_info.pbi_start_tvusec;
    woc_record *r = &a->record;
    r->bytes = sizeof(*r); r->version = WOC_VERSION;
    r->pid = c->owner.pid; r->pgid = c->owner.pgid; r->uid = c->owner.uid;
    r->ppid = c->owner.parent; r->sid = c->owner.session;
    r->uid_evidence = WOC_UID_RETAINED; r->pid_bytes = sizeof(pid_t);
    struct timespec time = {0, 0};
    errno = 0;
    a->start_clock_rc = clock_gettime(CLOCK_MONOTONIC, &time);
    a->start_clock_errno = errno;
    if (a->start_clock_rc != 0 || !ns_value(&time, &a->started_ns))
        return active_finish(c, a->start_clock_rc ? WOC_SYSTEM : WOC_INCONCLUSIVE,
                             a->start_clock_rc ? a->start_clock_errno : 0, 0);
    a->creator_pid = getpid(); a->creator_uid = getuid();
    if (a->creator_pid != c->owner.parent || a->creator_uid != c->owner.uid)
        return active_finish(c, WOC_OWNERSHIP, 0, 0);
    int rc = disposition(r);
    if (rc != WOC_OK) return active_finish(c, rc, r->system_errno, 0);
    a->wait_entered = 1; errno = 0;
    a->wait_rc = waitid(P_PID, (id_t)c->owner.pid, &t->active_raw, WEXITED | WNOWAIT | WNOHANG);
    a->wait_errno = errno; a->wait_returned = 1;
    if (a->wait_rc != 0) return active_finish(c, WOC_SYSTEM, a->wait_errno, 0);
    if (t->active_raw.si_pid == c->owner.pid) {
        rc = child_decode(c, &t->active_raw, r);
        if (rc != WOC_OK) return active_finish(c, rc, r->system_errno, 0);
        if (t->active_exit_saved && !same_exit(&t->first_active_exit.record, r))
            return active_finish(c, WOC_OWNERSHIP, 0, 0);
        return active_finish(c, WOC_OK, 0, 1);
    }
    if (t->active_raw.si_pid != 0 || t->active_raw.si_signo != 0 ||
        t->active_raw.si_code != 0 || t->active_raw.si_status != 0 || t->active_exit_saved)
        return active_finish(c, WOC_OWNERSHIP, 0, 0);
    a->bsd_entered = 1; errno = 0;
    a->bsd_bytes = proc_pidinfo(c->owner.pid, PROC_PIDTBSDINFO, 0, &t->active_bsd, sizeof(t->active_bsd));
    int bsd_errno = errno; a->bsd_returned = 1;
    if (a->bsd_bytes != sizeof(t->active_bsd))
        return active_finish(c, a->bsd_bytes <= 0 ? WOC_SYSTEM : WOC_INCONCLUSIVE, bsd_errno, 0);
    const struct proc_bsdinfo *info = &t->active_bsd;
    a->bsd_status = info->pbi_status;
    if (info->pbi_pid != c->owner.pid || info->pbi_ppid != c->owner.parent ||
        info->pbi_uid != c->owner.uid || info->pbi_pgid != c->owner.pgid ||
        info->pbi_start_tvsec != a->birth_seconds || info->pbi_start_tvusec != a->birth_microseconds)
        return active_finish(c, WOC_OWNERSHIP, 0, 0);
    if (info->pbi_status == SZOMB || (info->pbi_flags & PROC_FLAG_INEXIT) != 0) {
        /* Exit can race the no-event wait and this targeted BSD read. Keep a
         * distinct non-live result; a later finite observation may see exit. */
        r->state = WOC_ACTIVE_TRANSITION;
        return active_finish(c, WOC_OK, 0, 0);
    }
    if (!(info->pbi_status == SIDL || info->pbi_status == SRUN ||
          info->pbi_status == SSLEEP || info->pbi_status == SSTOP))
        return active_finish(c, WOC_OWNERSHIP, 0, 0);
    errno = 0; a->observed_sid = getsid(c->owner.pid);
    if (a->observed_sid != c->owner.session)
        return active_finish(c, a->observed_sid == -1 ? WOC_SYSTEM : WOC_OWNERSHIP,
                             a->observed_sid == -1 ? errno : 0, 0);
    r->state = WOC_LIVE; r->uid_evidence = WOC_UID_CURRENT;
    return active_finish(c, WOC_OK, 0, 0);
}

int woc_terminal_active_snapshot(woc_terminal *t, const void *key, uint64_t generation,
                                  int first_terminal, woc_active_observation *out,
                                  const void **raw, size_t *bytes)
{
    if (!WOC_CHILD_NATIVE_QUALIFIED) return WOC_UNSUPPORTED;
    if (out == NULL || raw == NULL || bytes == NULL || (first_terminal != 0 && first_terminal != 1))
        return WOC_ABI;
    if (!terminal_exact(t, key, generation)) return WOC_OWNERSHIP;
    int present = first_terminal ? t->active_exit_saved : t->active.entered;
    if (!present) {
        memset(out, 0, sizeof(*out)); out->bytes = sizeof(*out);
        out->version = WOC_ACTIVE_VERSION; out->generation = generation;
        *raw = NULL; *bytes = 0;
    } else {
        *out = first_terminal ? t->first_active_exit : t->active;
        *raw = first_terminal ? (const void *)&t->first_active_raw : (const void *)&t->active_raw;
        *bytes = out->wait_entered && out->wait_returned ? sizeof(siginfo_t) : 0;
    }
    return WOC_OK;
}

woc_child *woc_child_pending(const void *key, uint64_t *generation)
{
    if (child_root == NULL || child_root->key != key || child_root->invalid) return NULL;
    if (generation != NULL) *generation = child_root->report.generation;
    return child_root; /* includes unpublished, aborted, error, raw-reap/mapping failure */
}

int woc_child_snapshot(woc_child *c, const void *key, uint64_t generation, woc_child_report *out)
{
    if (out == NULL || !child_valid(c, key, generation)) return WOC_OWNERSHIP;
    *out = c->report;
    return WOC_OK;
}

/* Cleanup entries count each admitted native cleanup API entry, not its internal
 * OS calls. One deadline and one total entry budget are native and never reset.
 * Snapshot/pending/terminal reads do no process I/O; they cannot renew budgets.
 */
static int cleanup_clock(woc_child *c)
{
    struct timespec ts = {0, 0};
    errno = 0;
    c->report.clock_rc = clock_gettime(CLOCK_MONOTONIC, &ts);
    c->report.clock_errno = errno;
    if (c->report.clock_rc != 0 || !ns_value(&ts, &c->report.cleanup_now_ns))
        return child_failure(c, c->report.clock_rc ? WOC_SYSTEM : WOC_INCONCLUSIVE,
                             c->report.clock_rc ? c->report.clock_errno : 0, 1);
    return WOC_OK;
}

static int cleanup_enter(woc_child *c, const void *key, uint64_t generation)
{
    if (!child_valid(c, key, generation)) return WOC_OWNERSHIP;
    if (!child_normal_mode(c)) return WOC_OWNERSHIP;
    if (!child_registered_provenance(c) || c->report.blocked || !c->report.cleanup_started ||
        c->report.cleanup_entries >= c->report.max_cleanup_entries ||
        c->report.cleanup_entries == UINT32_MAX)
        return child_failure(c, WOC_OWNERSHIP, 0, 1);
    c->busy = 1;
    ++c->report.cleanup_entries;
    if (cleanup_clock(c) != WOC_OK || c->report.cleanup_now_ns >= c->report.cleanup_deadline_ns) {
        if (!c->report.blocked) child_failure(c, WOC_INCONCLUSIVE, 0, 1);
        c->busy = 0; return c->report.rc;
    }
    c->report.rc = WOC_OK; c->report.system_errno = 0;
    return WOC_OK;
}

static int cleanup_leave(woc_child *c, int rc)
{
    c->report.rc = rc;
    c->report.system_errno = c->report.record.system_errno;
    if (rc != WOC_OK && !(c->report.signal_failed && c->report.record.signal_attempted &&
                          rc == WOC_SYSTEM && c->report.record.system_errno > 0))
        c->report.blocked = 1;
    c->busy = 0;
    return rc;
}

int woc_child_cleanup_begin(woc_child *c, const void *key, uint64_t generation)
{
    if (!child_valid(c, key, generation)) return WOC_OWNERSHIP;
    if (!child_normal_mode(c)) return WOC_OWNERSHIP;
    if (!child_registered_provenance(c) || c->report.cleanup_started ||
        c->report.blocked || c->report.state != WOC_CHILD_ACTIVE)
        return WOC_OWNERSHIP;
    c->busy = 1;
    c->report.cleanup_started = 1; c->report.cleanup_entries = 1;
    if (cleanup_clock(c) != WOC_OK ||
        c->report.cleanup_now_ns > UINT64_MAX - c->cleanup_timeout_ns) {
        if (!c->report.blocked) child_failure(c, WOC_INCONCLUSIVE, 0, 1);
        c->busy = 0; return c->report.rc;
    }
    c->report.cleanup_deadline_ns = c->report.cleanup_now_ns + c->cleanup_timeout_ns;
    /* TERM grace is established only after an actual successful TERM return.
     * This never moves the one absolute cleanup deadline. */
    c->report.cleanup_kill_at_ns = 0;
    c->busy = 0; return WOC_OK;
}

int woc_child_observe(woc_child *c, const void *key, uint64_t generation)
{
    int rc = cleanup_enter(c, key, generation); if (rc) return rc;
    child_record(c);
    if (c->owner.no_signals || c->owner.reap_attempted)
        return cleanup_leave(c, failure(&c->report.record, WOC_OWNERSHIP, 0));
    rc = child_observe(c, &c->report.record);
    return cleanup_leave(c, rc);
}

int woc_child_members(woc_child *c, const void *key, uint64_t generation)
{
    int rc = cleanup_enter(c, key, generation); if (rc) return rc;
    child_record(c);
    if (c->owner.no_signals || !child_wait_unattempted(c))
        return cleanup_leave(c, failure(&c->report.record, WOC_OWNERSHIP, 0));
    rc = membership(&c->owner, &c->report.record);
    return cleanup_leave(c, rc);
}

static int child_member(woc_child *c, int64_t pid, woc_record *out)
{
    if (!child_registered_provenance(c) || !child_wait_unattempted(c) ||
        c->owner.no_signals || pid <= 1 || pid > INT_MAX ||
        pid == c->owner.pid || c->owner.count != 2 ||
        (pid != c->owner.members[0] && pid != c->owner.members[1]))
        return failure(out, WOC_OWNERSHIP, 0);
    int rc = disposition(out);
    return rc ? rc : inspect(&c->owner, (pid_t)pid, 0, out);
}

int woc_child_member(woc_child *c, const void *key, uint64_t generation, int64_t pid)
{
    int rc = cleanup_enter(c, key, generation); if (rc) return rc;
    child_record(c); rc = child_member(c, pid, &c->report.record);
    return cleanup_leave(c, rc);
}

int woc_child_signal(woc_child *c, const void *key, uint64_t generation,
                     int kind, int64_t positive_member)
{
    int rc = cleanup_enter(c, key, generation); if (rc) return rc;
    child_record(c); woc_record *r = &c->report.record;
    if (c->report.signal_failed || c->owner.no_signals || c->owner.reap_attempted ||
        c->report.latch_attempted || (kind != WOC_TERM && kind != WOC_KILL) ||
        (kind == WOC_TERM && c->report.term_attempted) ||
        (kind == WOC_KILL && (!c->report.term_attempted || c->report.kill_attempted ||
                             c->report.cleanup_now_ns < c->report.cleanup_kill_at_ns)))
    {
        c->report.signal_failed = 1;
        return cleanup_leave(c, failure(r, WOC_OWNERSHIP, 0));
    }
    /* Sticky intent precedes all checks; a failed TERM/grace/KILL entry is never retried. */
    if (kind == WOC_TERM) c->report.term_attempted = 1;
    else c->report.kill_attempted = 1;
    rc = child_observe(c, r);
    woc_record before = *r;
    if (rc == WOC_OK && before.state == WOC_EXITED) {
        woc_record member = *r;
        rc = child_member(c, positive_member, &member);
        if (rc != WOC_OK || member.state != WOC_LIVE)
            rc = failure(r, rc ? rc : WOC_INCONCLUSIVE, member.system_errno);
        else {
            rc = child_observe(c, r);
            if (rc == WOC_OK && !same_exit(&before, r)) rc = failure(r, WOC_OWNERSHIP, 0);
        }
    } else if (rc == WOC_OK && positive_member != 0) rc = failure(r, WOC_OWNERSHIP, 0);
    if (rc == WOC_OK) {
        r->signal_attempted = 1;
        errno = 0;
        int signal_rc = killpg(c->owner.pgid, kind == WOC_TERM ? SIGTERM : SIGKILL);
        int signal_errno = errno;
        if (kind == WOC_TERM) {
            c->report.term_rc = signal_rc; c->report.term_errno = signal_errno; c->report.term_returned = 1;
        } else {
            c->report.kill_rc = signal_rc; c->report.kill_errno = signal_errno; c->report.kill_returned = 1;
        }
        if (signal_rc != 0) rc = failure(r, WOC_SYSTEM, signal_errno);
        else if (kind == WOC_TERM) {
            /* Preserve raw TERM results above before the additional clock call.
             * A grace endpoint beyond the cleanup deadline permits observation
             * of natural exit, but never an early KILL or a renewed deadline. */
            if (cleanup_clock(c) != WOC_OK)
                rc = failure(r, c->report.rc, c->report.system_errno);
            else if (c->report.cleanup_now_ns >= c->report.cleanup_deadline_ns ||
                     c->report.cleanup_now_ns > UINT64_MAX - c->grace_ns)
                rc = failure(r, WOC_INCONCLUSIVE, 0);
            else c->report.cleanup_kill_at_ns = c->report.cleanup_now_ns + c->grace_ns;
        }
    }
    if (rc != WOC_OK) c->report.signal_failed = 1;
    return cleanup_leave(c, rc);
}

int woc_child_latch(woc_child *c, const void *key, uint64_t generation,
                    int64_t exit_kind, int64_t exit_status)
{
    int rc = cleanup_enter(c, key, generation); if (rc) return rc;
    child_record(c); woc_record *r = &c->report.record;
    if (c->report.latch_attempted || c->owner.no_signals || c->owner.reap_attempted)
        return cleanup_leave(c, failure(r, WOC_OWNERSHIP, 0));
    c->report.latch_attempted = 1;
    c->owner.no_signals = 1; r->no_signals = 1;
    rc = child_observe(c, r);
    woc_record before = *r, members = *r;
    if (rc == WOC_OK) rc = membership(&c->owner, &members);
    if (rc == WOC_OK) rc = child_observe(c, r);
    if (rc == WOC_OK && (!same_exit(&before, r) || before.exit_kind != exit_kind ||
        before.exit_status != exit_status || members.count != 1 ||
        members.raw_bytes != sizeof(pid_t) || members.member0 != c->owner.pid || members.member1 != 0))
        rc = failure(r, WOC_INCONCLUSIVE, 0);
    if (rc == WOC_OK) {
        c->owner.exit_kind = exit_kind; c->owner.exit_status = exit_status; c->owner.certified = 1;
    } else if (r->error == 0) failure(r, rc, members.system_errno);
    return cleanup_leave(c, rc);
}

/* Internal body only; the public entry retains its original cleanup admission. */
static int child_reap_exact(woc_child *c)
{
    int rc;
    child_record(c); woc_record *r = &c->report.record;
    woc_terminal *t = c->terminal;
    if (!child_registered_provenance(c) || !child_wait_unattempted(c) ||
        !c->owner.no_signals || !c->owner.certified)
        return cleanup_leave(c, failure(r, WOC_OWNERSHIP, 0));
    c->owner.reap_attempted = 1; r->reap_attempted = 1;
    t->report.reap_attempted = 1; /* sticky even disposition/decode/publication failure */
    rc = disposition(r);
    if (rc != WOC_OK) return cleanup_leave(c, rc);
    c->report.state = WOC_CHILD_REAP_ENTERED;
    t->report.wait_entered = 1;
    errno = 0;
    t->report.wait_rc = waitid(P_PID, (id_t)c->owner.pid, &t->raw_siginfo, WEXITED | WNOHANG);
    t->report.wait_errno = errno;
    t->report.wait_returned = 1;
    /* The actual raw wait result and complete siginfo are NATIVE-ROOTED here,
     * before decoding, callbacks, Python mapping/allocation or signal checks. */
    t->report.wait_pid = t->raw_siginfo.si_pid;
    t->report.wait_uid = t->raw_siginfo.si_uid;
    t->report.wait_code = t->raw_siginfo.si_code;
    t->report.wait_status = t->raw_siginfo.si_status;
    /* False means terminal consumption is UNPROVEN, not that no event was consumed. */
    t->report.consumed = t->report.wait_entered && t->report.wait_returned &&
        t->report.wait_rc == 0 && child_registered_provenance(c) &&
        child_terminal_envelope(c, &t->raw_siginfo);
    c->owner.reaped = t->report.consumed; r->reaped = c->owner.reaped;
    c->report.state = WOC_CHILD_REAP_RECORDED;
    if (t->report.wait_rc != 0) rc = failure(r, WOC_SYSTEM, t->report.wait_errno);
    else if (!t->report.consumed) rc = failure(r, WOC_OWNERSHIP, 0);
    else {
        rc = child_decode(c, &t->raw_siginfo, r);
        t->report.decoded = rc == WOC_OK;
        if (t->report.decoded) t->report.decoded_record = *r;
        t->report.certificate_matches = rc == WOC_OK && r->exit_kind == c->owner.exit_kind &&
                                        r->exit_status == c->owner.exit_status;
        if (rc == WOC_OK && !t->report.certificate_matches) rc = failure(r, WOC_OWNERSHIP, 0);
    }
    if (rc == WOC_OK) t->report.kind = WOC_TERMINAL_NORMAL_GROUP_CERTIFICATE;
    if (rc != WOC_OK) c->report.blocked = 1;
    return cleanup_leave(c, rc);
}

int woc_child_reap(woc_child *c, const void *key, uint64_t generation)
{
    int rc = cleanup_enter(c, key, generation); if (rc) return rc;
    return child_reap_exact(c);
}

woc_terminal *woc_child_terminal(woc_child *c, const void *key, uint64_t generation)
{
    return child_valid(c, key, generation) ? c->terminal : NULL;
}

static int terminal_exact(woc_terminal *t, const void *key, uint64_t generation);

int woc_terminal_snapshot(woc_terminal *t, const void *key, uint64_t generation,
                          woc_terminal_report *out, const void **raw_siginfo)
{
    if (out == NULL || !terminal_exact(t, key, generation)) return WOC_OWNERSHIP;
    *out = t->report;
    if (child_root != NULL && child_root->terminal == t) out->final_report = child_root->report;
    if (raw_siginfo != NULL) *raw_siginfo = &t->raw_siginfo;
    return WOC_OK;
}

int woc_child_handoff(woc_child *c, const void *key, uint64_t generation, woc_terminal *t)
{
    if (!child_valid(c, key, generation) || !child_normal_mode(c) || t == NULL || t != c->terminal ||
        !t->report.rooted || t->report.generation != generation || t->report.transferred ||
        !c->report.resources_closed || c->report.resource_error) return WOC_OWNERSHIP;
    if (t->report.known_no_creation) {
        /* Never-entered precreation abort has no child, and no cleanup budget to renew. */
        if (t->report.kind != WOC_TERMINAL_NO_CREATION ||
            c->report.spawn_entered || c->report.state != WOC_CHILD_ABORTED) return WOC_OWNERSHIP;
    } else {
        int rc = cleanup_enter(c, key, generation); if (rc) return rc;
        if (!child_registered_provenance(c) ||
            t->report.kind != WOC_TERMINAL_NORMAL_GROUP_CERTIFICATE ||
            !c->owner.reap_attempted || !c->owner.reaped || !c->owner.no_signals ||
            !t->report.wait_returned || !t->report.consumed || !t->report.decoded ||
            !t->report.certificate_matches) {
            c->busy = 0; return child_failure(c, WOC_OWNERSHIP, 0, 1);
        }
    }
    /* Concrete root handoff. Reserved native terminal root already holds raw wait;
     * copy the full generation/creation/cleanup facts BEFORE invalidation/free.
     * No boolean acknowledgment or Python returncode substitutes for this sink.
     */
    c->report.state = WOC_CHILD_TRANSFERRED;
    t->report.final_report = c->report;
    t->report.transferred = 1;
    c->invalid = 1;
    child_root = NULL;
    free(c); /* terminal_roots still owns t; old operational Child is now invalid */
    return WOC_OK;
}

/* Bounded, exclusive direct-child abort. SOURCE ONLY / UNCOMPILED / UNRUN.
 * This domain never changes original launch rc/errno/blocked/state/record and
 * never sets owner.certified. The root itself prevents all further launches.
 * No hard wall-time bound on a libc/kernel call, external-waiter exclusion or
 * recovery of an entered-not-returned call is asserted by these source bounds.
 */
enum { AC_CLOCK = 1, AC_SIGCHLD, AC_WAIT_NOWAIT, AC_PROCINFO, AC_GETSID,
       AC_DIRECT_KILL, AC_WAIT_CONSUME };
static int terminal_exact(woc_terminal *t, const void *key, uint64_t generation);

static int abort_reply(woc_child *c, int operation, int rc, int error)
{
    c->report.abort.last_operation = operation;
    c->report.abort.last_rc = rc;
    c->report.abort.last_system_errno = error;
    return rc;
}

static int abort_failed(woc_child *c, int rc, int error)
{
    woc_launch_abort_report *a = &c->report.abort;
    if (a->failure_rc == WOC_OK) {
        a->failure_rc = rc;
        a->failure_system_errno = error;
    }
    a->phase = WOC_ABORT_FAILED_RETAINED;
    return rc;
}

static int abort_leave(woc_child *c, int operation, int rc, int error)
{
    if (rc != WOC_OK) abort_failed(c, rc, error);
    abort_reply(c, operation, rc, error);
    c->busy = 0;
    return rc;
}

static woc_child_call_fact *abort_fact(woc_child *c, int kind)
{
    woc_launch_abort_report *a = &c->report.abort;
    if (a->trace_count >= WOC_ABORT_FACTS) {
        abort_failed(c, WOC_INCONCLUSIVE, 0);
        return NULL;
    }
    woc_child_call_fact *f = &a->trace[a->trace_count++];
    f->kind = kind; f->entered = 1;
    return f;
}

static int abort_clock(woc_child *c)
{
    struct timespec ts = {0, 0};
    woc_child_call_fact *f = abort_fact(c, AC_CLOCK);
    if (f == NULL) return WOC_INCONCLUSIVE;
    errno = 0;
    int rc = clock_gettime(CLOCK_MONOTONIC, &ts);
    int error = errno;
    launch_return(f, rc, error);
    if (rc != 0) return abort_failed(c, WOC_SYSTEM, error);
    uint64_t now;
    if (!ns_value(&ts, &now) || now < c->report.abort.now_ns)
        return abort_failed(c, WOC_INCONCLUSIVE, 0);
    c->report.abort.now_ns = now;
    if (now >= c->report.abort.deadline_ns)
        return abort_failed(c, WOC_INCONCLUSIVE, 0);
    return WOC_OK;
}

static int abort_policy(woc_child *c)
{
    struct sigaction action;
    memset(&action, 0, sizeof(action));
    woc_child_call_fact *f = abort_fact(c, AC_SIGCHLD);
    if (f == NULL) return WOC_INCONCLUSIVE;
    errno = 0;
    int rc = sigaction(SIGCHLD, NULL, &action);
    int error = errno;
    launch_return(f, rc, error);
    if (rc != 0) return abort_failed(c, WOC_SYSTEM, error);
    c->report.abort.record.sigchld_default = action.sa_handler == SIG_DFL;
    c->report.abort.record.no_cldwait = (action.sa_flags & SA_NOCLDWAIT) != 0;
    if (!c->report.abort.record.sigchld_default || c->report.abort.record.no_cldwait)
        return abort_failed(c, WOC_OWNERSHIP, 0);
    return WOC_OK;
}

static void abort_record_identity(woc_child *c)
{
    woc_record *r = &c->report.abort.record;
    r->bytes = sizeof(*r); r->version = WOC_VERSION;
    r->pid = c->owner.pid; r->pgid = c->owner.pgid; r->uid = c->owner.uid;
    r->ppid = c->owner.parent; r->sid = c->owner.session;
    r->uid_evidence = c->report.registered ? WOC_UID_RETAINED : 0;
    r->wait_uid_available = 0; r->pid_bytes = sizeof(pid_t);
    r->no_signals = c->owner.no_signals;
}

static int abort_eligible(woc_child *c)
{
    const woc_child_report *r = &c->report;
    const uint64_t allowed = WOC_LF_MASK(WOC_LF_PRE_RESUME_CLOCK) |
                             WOC_LF_MASK(WOC_LF_PRE_RESUME_DEADLINE);
    if (!child_registered_provenance(c) || !child_wait_unattempted(c) ||
        r->abort.started || r->published || c->actions_initialized || c->attr_initialized ||
        c->stdio[0] >= 0 || c->stdio[1] >= 0 || c->stdio[2] >= 0 ||
        !r->resources_closed || r->resource_error ||
        !r->sigchld_preflight_entered || !r->sigchld_preflight_returned ||
        r->sigchld_preflight_rc != 0 || !r->sigchld_preflight_policy_valid ||
        !r->sigchld_preflight_default || r->sigchld_preflight_no_cldwait ||
        !r->sigchld_preflight_accepted || r->resume_entered || r->cleanup_started ||
        r->latch_attempted || r->signal_failed || r->term_attempted || r->kill_attempted ||
        c->owner.no_signals || c->owner.reap_attempted || c->owner.reaped ||
        c->owner.certified || c->terminal->report.reap_attempted ||
        c->terminal->report.wait_entered || c->terminal->report.transferred ||
        c->terminal->report.kind != WOC_TERMINAL_UNKNOWN ||
        !c->terminal->report.rooted || c->terminal->key != c->key ||
        c->terminal->report.generation != r->generation ||
        r->abort.entries != 0 || r->abort.trace_count != 0 ||
        r->abort.phase != WOC_ABORT_NONE_RESERVED || r->abort.kill_entered ||
        r->abort.deadline_ns == 0 || r->launch_fact_count > WOC_CHILD_LAUNCH_FACTS ||
        (r->launch_failure_mask & ~allowed) != 0) return 0;
    for (uint32_t i = 0; i < r->launch_fact_count; ++i)
        if (!r->launch_facts[i].entered || !r->launch_facts[i].returned) return 0;
    if (r->launch_failure_mask == 0)
        return r->state == WOC_CHILD_SUSPENDED && !r->blocked && r->rc == WOC_OK;
    return r->state == WOC_CHILD_BLOCKED && r->blocked && r->rc != WOC_OK &&
           (r->launch_first_failure_reason == WOC_LF_PRE_RESUME_CLOCK ||
            r->launch_first_failure_reason == WOC_LF_PRE_RESUME_DEADLINE);
}

/* Rejected exact-root entries alter only the nonauthoritative last-step reply.
 * All phase, mode, bounds, raw slots and sticky errors remain unchanged. */
static int abort_admit(woc_child *c, int operation, uint32_t facts)
{
    woc_launch_abort_report *a = &c->report.abort;
    if (!child_registered_provenance(c))
        return abort_reply(c, operation, WOC_OWNERSHIP, 0);
    if (a->entries >= WOC_ABORT_ENTRIES ||
        (operation != WOC_ABORT_OP_HANDOFF && a->entries >= WOC_ABORT_ENTRIES - 1) ||
        a->trace_count > WOC_ABORT_FACTS ||
        facts > WOC_ABORT_FACTS - a->trace_count)
        return abort_reply(c, operation, WOC_INCONCLUSIVE, 0);
    c->busy = 1;
    ++a->entries;
    return WOC_OK;
}

/* One raw slot is written once. Even rc!=0 preserves the full SDK output before
 * interpretation. The slots live in the permanent terminal, including after free. */
static int abort_observation(woc_child *c, uint32_t index)
{
    woc_launch_abort_report *a = &c->report.abort;
    if (!child_registered_provenance(c) || !child_wait_unattempted(c) ||
        index >= WOC_ABORT_RAW_SLOTS || a->observations[index].entered)
        return abort_failed(c, WOC_OWNERSHIP, 0);
    woc_child_call_fact *f = abort_fact(c, AC_WAIT_NOWAIT);
    if (f == NULL) return WOC_INCONCLUSIVE;
    woc_abort_observation *o = &a->observations[index];
    siginfo_t *raw = &c->terminal->abort_raw[index];
    o->raw_siginfo_bytes = sizeof(*raw);
    o->entered = 1;
    errno = 0;
    o->rc = waitid(P_PID, (id_t)c->owner.pid, raw, WEXITED | WSTOPPED | WNOWAIT | WNOHANG);
    o->system_errno = errno;
    o->returned = 1;
    launch_return(f, o->rc, o->system_errno);
    o->pid = raw->si_pid; o->uid = raw->si_uid;
    o->code = raw->si_code; o->status = raw->si_status;
    if (o->rc != 0) return abort_failed(c, WOC_SYSTEM, o->system_errno);
    if (raw->si_pid == 0 || child_stopped_unresumed(c, raw))
        return WOC_OK; /* Nonterminal only; existing fresh BSD/SID/deadline checks still apply. */
    if (!child_terminal_envelope(c, raw)) return abort_failed(c, WOC_OWNERSHIP, 0);
    int rc = child_decode(c, raw, &a->record);
    if (rc != WOC_OK) return abort_failed(c, rc, a->record.system_errno);
    a->exit_observation_index = (int)index; a->exit_valid = 1;
    a->phase = WOC_ABORT_EXIT_READY;
    return WOC_OK;
}

/* Fresh full native identity/SSTOP corroboration for the only signal. */
static int abort_live_identity(woc_child *c)
{
    struct proc_bsdinfo *bsd = &c->terminal->abort_bsd;
    woc_child_call_fact *f = abort_fact(c, AC_PROCINFO);
    if (f == NULL) return WOC_INCONCLUSIVE;
    errno = 0;
    int bytes = proc_pidinfo(c->owner.pid, PROC_PIDTBSDINFO, 0, bsd, sizeof(*bsd));
    int error = errno;
    launch_return(f, bytes, error);
    if (bytes != sizeof(*bsd))
        return abort_failed(c, bytes <= 0 ? WOC_SYSTEM : WOC_INCONCLUSIVE,
                            bytes <= 0 ? error : 0);
    woc_launch_abort_report *a = &c->report.abort;
    a->bsd_status = bsd->pbi_status; a->bsd_pid = bsd->pbi_pid;
    a->bsd_ppid = bsd->pbi_ppid; a->bsd_uid = bsd->pbi_uid; a->bsd_pgid = bsd->pbi_pgid;
    f = abort_fact(c, AC_GETSID);
    if (f == NULL) return WOC_INCONCLUSIVE;
    errno = 0;
    pid_t sid = getsid(c->owner.pid);
    error = errno;
    launch_return(f, sid, error);
    a->sid = sid;
    if (sid < 0) return abort_failed(c, WOC_SYSTEM, error);
    if (bsd->pbi_pid != c->owner.pid || bsd->pbi_ppid != c->owner.parent ||
        bsd->pbi_uid != c->owner.uid || bsd->pbi_pgid != c->owner.pid ||
        sid != c->owner.pid || bsd->pbi_status != SSTOP)
        return abort_failed(c, WOC_OWNERSHIP, 0);
    a->record.state = WOC_LIVE; a->record.uid_evidence = WOC_UID_CURRENT;
    return WOC_OK;
}

static int abort_begin_exact(woc_child *c)
{
    if (!abort_eligible(c)) return abort_reply(c, WOC_ABORT_OP_BEGIN, WOC_OWNERSHIP, 0);
    if (abort_admit(c, WOC_ABORT_OP_BEGIN, 7) != WOC_OK) return c->report.abort.last_rc;
    woc_launch_abort_report *a = &c->report.abort;
    a->started = 1; a->phase = WOC_ABORT_BEGIN_ENTERED; /* before clock/evidence */
    abort_record_identity(c);
    int rc = abort_clock(c);
    if (rc == WOC_OK) rc = abort_policy(c);
    if (rc == WOC_OK) rc = abort_observation(c, 0);
    if (rc != WOC_OK) return abort_leave(c, WOC_ABORT_OP_BEGIN, rc, a->failure_system_errno);
    if (a->exit_valid) {
        a->preexisting_exit = 1;
        c->owner.no_signals = 1; a->record.no_signals = 1;
        return abort_leave(c, WOC_ABORT_OP_BEGIN, WOC_OK, 0);
    }
    rc = abort_live_identity(c);
    if (rc != WOC_OK) return abort_leave(c, WOC_ABORT_OP_BEGIN, rc, a->failure_system_errno);
    /* Fresh SAME absolute deadline AFTER corroboration, immediately BEFORE intent. */
    rc = abort_clock(c);
    if (rc != WOC_OK) return abort_leave(c, WOC_ABORT_OP_BEGIN, rc, a->failure_system_errno);
    woc_child_call_fact *f = abort_fact(c, AC_DIRECT_KILL);
    if (f == NULL) return abort_leave(c, WOC_ABORT_OP_BEGIN, WOC_INCONCLUSIVE, 0);
    a->kill_entered = 1; a->phase = WOC_ABORT_KILL_ENTERED;
    a->record.signal_attempted = 1;
    errno = 0;
    a->kill_rc = kill(c->owner.pid, SIGKILL);
    a->kill_errno = errno; a->kill_returned = 1;
    launch_return(f, a->kill_rc, a->kill_errno);
    /* One signal BEFORE latch. No further API, mapping or observation intervenes.
     * Failed kill is sticky too; no signal can ever be retried after this point. */
    c->owner.no_signals = 1; a->record.no_signals = 1;
    if (a->kill_rc != 0) return abort_leave(c, WOC_ABORT_OP_BEGIN, WOC_SYSTEM, a->kill_errno);
    a->phase = WOC_ABORT_WAITING_EXIT;
    return abort_leave(c, WOC_ABORT_OP_BEGIN, WOC_OK, 0);
}

int woc_child_abort_begin(woc_child *c, const void *key, uint64_t generation)
{
    if (!WOC_CHILD_NATIVE_QUALIFIED) return WOC_UNSUPPORTED;
    if (!child_valid(c, key, generation)) return WOC_OWNERSHIP;
    return abort_begin_exact(c);
}

static int abort_observe_exact(woc_child *c)
{
    woc_launch_abort_report *a = &c->report.abort;
    if (!child_registered_provenance(c) || !child_wait_unattempted(c) ||
        !a->started || a->phase != WOC_ABORT_WAITING_EXIT || !c->owner.no_signals ||
        !a->kill_entered || !a->kill_returned || a->kill_rc != 0 ||
        a->observation_count >= WOC_ABORT_OBSERVATIONS || c->owner.reap_attempted)
        return abort_reply(c, WOC_ABORT_OP_OBSERVE, WOC_OWNERSHIP, 0);
    if (abort_admit(c, WOC_ABORT_OP_OBSERVE, 3) != WOC_OK) return a->last_rc;
    ++a->observation_count;
    int rc = abort_clock(c);
    if (rc == WOC_OK) rc = abort_policy(c);
    if (rc == WOC_OK) rc = abort_observation(c, a->observation_count);
    return abort_leave(c, WOC_ABORT_OP_OBSERVE, rc, rc ? a->failure_system_errno : 0);
}

int woc_child_abort_observe(woc_child *c, const void *key, uint64_t generation)
{
    if (!WOC_CHILD_NATIVE_QUALIFIED) return WOC_UNSUPPORTED;
    if (!child_valid(c, key, generation)) return WOC_OWNERSHIP;
    return abort_observe_exact(c);
}

static int abort_reap_exact(woc_child *c)
{
    woc_launch_abort_report *a = &c->report.abort;
    woc_terminal *t = c->terminal;
    if (!child_registered_provenance(c) || !child_wait_unattempted(c) ||
        !a->started || a->phase != WOC_ABORT_EXIT_READY || !a->exit_valid ||
        a->exit_observation_index < 0 || a->exit_observation_index >= WOC_ABORT_RAW_SLOTS ||
        !c->owner.no_signals || c->owner.certified || c->owner.reap_attempted ||
        t->report.wait_entered || t->report.reap_attempted || c->report.resume_entered)
        return abort_reply(c, WOC_ABORT_OP_REAP, WOC_OWNERSHIP, 0);
    const woc_abort_observation *o = &a->observations[a->exit_observation_index];
    const siginfo_t *evidence = &t->abort_raw[a->exit_observation_index];
    if (!o->entered || !o->returned || o->rc != 0 ||
        o->raw_siginfo_bytes != sizeof(*evidence) || !child_terminal_envelope(c, evidence) ||
        o->pid != evidence->si_pid ||
        o->uid != evidence->si_uid || o->code != evidence->si_code || o->status != evidence->si_status ||
        !child_terminal_record_matches(c, evidence, &a->record))
        return abort_reply(c, WOC_ABORT_OP_REAP, WOC_OWNERSHIP, 0);
    woc_record before_record = a->record;
    if (abort_admit(c, WOC_ABORT_OP_REAP, 4) != WOC_OK) return a->last_rc;
    int rc = abort_clock(c);
    if (rc == WOC_OK) rc = abort_policy(c);
    /* Fresh SAME absolute deadline AFTER disposition, immediately BEFORE intent. */
    if (rc == WOC_OK) rc = abort_clock(c);
    if (rc != WOC_OK) return abort_leave(c, WOC_ABORT_OP_REAP, rc, a->failure_system_errno);
    woc_child_call_fact *f = abort_fact(c, AC_WAIT_CONSUME);
    if (f == NULL) return abort_leave(c, WOC_ABORT_OP_REAP, WOC_INCONCLUSIVE, 0);
    a->phase = WOC_ABORT_REAP_ENTERED;
    c->owner.reap_attempted = 1; a->record.reap_attempted = 1;
    t->report.reap_attempted = 1; t->report.wait_entered = 1;
    errno = 0;
    t->report.wait_rc = waitid(P_PID, (id_t)c->owner.pid, &t->raw_siginfo, WEXITED | WNOHANG);
    t->report.wait_errno = errno; t->report.wait_returned = 1;
    launch_return(f, t->report.wait_rc, t->report.wait_errno);
    /* All raw consuming facts rooted before any decode/publication. */
    t->report.wait_pid = t->raw_siginfo.si_pid; t->report.wait_uid = t->raw_siginfo.si_uid;
    t->report.wait_code = t->raw_siginfo.si_code; t->report.wait_status = t->raw_siginfo.si_status;
    /* False means terminal consumption is UNPROVEN, not that no event was consumed. */
    t->report.consumed = t->report.wait_entered && t->report.wait_returned &&
        t->report.wait_rc == 0 && child_registered_provenance(c) &&
        child_terminal_envelope(c, &t->raw_siginfo);
    c->owner.reaped = t->report.consumed; a->record.reaped = c->owner.reaped;
    if (t->report.wait_rc != 0) rc = WOC_SYSTEM;
    else if (!t->report.consumed) rc = WOC_OWNERSHIP;
    else {
        rc = child_decode(c, &t->raw_siginfo, &a->record);
        t->report.decoded = rc == WOC_OK;
        if (t->report.decoded) t->report.decoded_record = a->record;
        const siginfo_t *before = &t->abort_raw[a->exit_observation_index];
        t->report.certificate_matches = rc == WOC_OK && child_registered_provenance(c) &&
            before->si_pid == t->raw_siginfo.si_pid && before->si_signo == t->raw_siginfo.si_signo &&
            before->si_code == t->raw_siginfo.si_code && before->si_status == t->raw_siginfo.si_status &&
            before_record.exit_kind == a->record.exit_kind &&
            before_record.exit_status == a->record.exit_status;
        if (rc == WOC_OK && !t->report.certificate_matches) rc = WOC_OWNERSHIP;
    }
    if (rc == WOC_OK) {
        t->report.kind = WOC_TERMINAL_DIRECT_CHILD_ABORT_UNRESUMED;
        a->phase = WOC_ABORT_REAP_RECORDED;
    }
    return abort_leave(c, WOC_ABORT_OP_REAP, rc,
                       t->report.wait_rc != 0 ? t->report.wait_errno : 0);
}

int woc_child_abort_reap(woc_child *c, const void *key, uint64_t generation)
{
    if (!WOC_CHILD_NATIVE_QUALIFIED) return WOC_UNSUPPORTED;
    if (!child_valid(c, key, generation)) return WOC_OWNERSHIP;
    return abort_reap_exact(c);
}

static int terminal_exact(woc_terminal *t, const void *key, uint64_t generation)
{
    int rooted = 0;
    for (size_t i = 0; i < terminal_count; ++i) if (terminal_roots[i] == t) rooted = 1;
    /* Pointer equality/root membership BEFORE dereferencing an untrusted pointer. */
    return rooted && t != NULL && key != NULL && generation != 0 && t->key == key &&
           t->report.rooted && t->report.generation == generation;
}

static int terminal_registration_raw_exact(woc_terminal *t, const void *key,
                                            uint64_t generation,
                                            const void **raw_siginfo, size_t *bytes)
{
    if (!terminal_exact(t, key, generation) || raw_siginfo == NULL || bytes == NULL)
        return WOC_OWNERSHIP;
    const woc_registration_observation *o = &t->report.registration_observation;
    /* Unentered/in-flight bytes can still change. A failed returned call is a
     * completed observation too; its bytes do not establish registration. */
    if (!o->entered || !o->returned || o->raw_siginfo_bytes != sizeof(t->registration_raw))
        return WOC_OWNERSHIP;
    *raw_siginfo = &t->registration_raw;
    *bytes = sizeof(t->registration_raw);
    return WOC_OK;
}

int woc_terminal_registration_observation(woc_terminal *t, const void *key,
                                          uint64_t generation,
                                          const void **raw_siginfo, size_t *bytes)
{
    if (!WOC_CHILD_NATIVE_QUALIFIED) return WOC_UNSUPPORTED;
    return terminal_registration_raw_exact(t, key, generation, raw_siginfo, bytes);
}

static int terminal_abort_raw_exact(woc_terminal *t, const void *key,
                                    uint64_t generation, uint32_t index,
                                    const void **raw_siginfo, size_t *bytes)
{
    if (!terminal_exact(t, key, generation) || index >= WOC_ABORT_RAW_SLOTS ||
        raw_siginfo == NULL || bytes == NULL) return WOC_OWNERSHIP;
    const woc_child_report *r = &t->report.final_report;
    if (child_root != NULL && child_root->terminal == t) r = &child_root->report;
    const woc_abort_observation *o = &r->abort.observations[index];
    /* An unentered slot may be written later; entered-not-returned is uncertain.
     * Only a completed single write is immutable, including a failed wait's raw. */
    if (r->generation != generation || !o->entered || !o->returned ||
        o->raw_siginfo_bytes != sizeof(t->abort_raw[index])) return WOC_OWNERSHIP;
    *raw_siginfo = &t->abort_raw[index];
    *bytes = sizeof(t->abort_raw[index]);
    return WOC_OK;
}

int woc_terminal_abort_observation(woc_terminal *t, const void *key,
                                   uint64_t generation, uint32_t index,
                                   const void **raw_siginfo, size_t *bytes)
{
    if (!WOC_CHILD_NATIVE_QUALIFIED) return WOC_UNSUPPORTED;
    return terminal_abort_raw_exact(t, key, generation, index, raw_siginfo, bytes);
}

static int abort_handoff_exact(woc_child *c, woc_terminal *t)
{
    woc_launch_abort_report *a = &c->report.abort;
    if (!child_registered_provenance(c) ||
        !a->started || a->phase != WOC_ABORT_REAP_RECORDED ||
        t == NULL || t != c->terminal || !terminal_exact(t, c->key, c->report.generation) ||
        t->report.transferred || t->report.kind != WOC_TERMINAL_DIRECT_CHILD_ABORT_UNRESUMED ||
        c->report.resume_entered || !c->report.resources_closed || c->report.resource_error ||
        !c->owner.no_signals || c->owner.certified || !c->owner.reap_attempted || !c->owner.reaped ||
        !t->report.wait_entered || !t->report.wait_returned || t->report.wait_rc != 0 ||
        !t->report.consumed || !t->report.decoded || !t->report.certificate_matches ||
        !a->exit_valid || a->exit_observation_index < 0 ||
        a->exit_observation_index >= WOC_ABORT_RAW_SLOTS)
        return abort_reply(c, WOC_ABORT_OP_HANDOFF, WOC_OWNERSHIP, 0);
    const siginfo_t *before = &t->abort_raw[a->exit_observation_index];
    if (!child_terminal_envelope(c, before) ||
        !child_terminal_envelope(c, &t->raw_siginfo) ||
        before->si_pid != t->raw_siginfo.si_pid || before->si_signo != t->raw_siginfo.si_signo ||
        before->si_code != t->raw_siginfo.si_code || before->si_status != t->raw_siginfo.si_status ||
        !child_terminal_record_matches(c, &t->raw_siginfo, &t->report.decoded_record) ||
        !child_terminal_record_matches(c, &t->raw_siginfo, &a->record))
        return abort_reply(c, WOC_ABORT_OP_HANDOFF, WOC_OWNERSHIP, 0);
    /* Memory-only late handoff is allowed. It never resets or reads the clock,
     * claims deadline success, or permits another OS call. Reserve one entry. */
    if (abort_admit(c, WOC_ABORT_OP_HANDOFF, 0) != WOC_OK) return a->last_rc;
    a->phase = WOC_ABORT_TRANSFERRED;
    abort_reply(c, WOC_ABORT_OP_HANDOFF, WOC_OK, 0);
    t->report.final_report = c->report;
    t->report.transferred = 1; /* typed terminal facts before invalidation/free */
    c->invalid = 1;
    child_root = NULL;
    free(c);
    return WOC_OK;
}

int woc_child_abort_handoff(woc_child *c, const void *key, uint64_t generation, woc_terminal *t)
{
    if (!WOC_CHILD_NATIVE_QUALIFIED) return WOC_UNSUPPORTED;
    if (!child_valid(c, key, generation)) return WOC_OWNERSHIP;
    return abort_handoff_exact(c, t);
}
