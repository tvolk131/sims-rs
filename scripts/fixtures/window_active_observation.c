/* Authored synthetic fixture. UNRUN until integrating-parent validation.
 * Includes the actual C implementation through a private copied include path.
 * The mock branch has no process creation/query/control or real clock calls.
 * Static fixture roots are authored private test data, never native evidence.
 */
#ifndef _DARWIN_C_SOURCE
#define _DARWIN_C_SOURCE 1
#endif
#include <errno.h>
#include <fcntl.h>
#include <spawn.h>
#include <time.h>
#include <limits.h>
#include <libproc.h>
#include <signal.h>
#include <stdlib.h>
#include <stdio.h>
#include <string.h>
#include <stdint.h>
#include <sys/proc.h>
#include <sys/proc_info.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

static void failed(int line, const char *expression)
{
    fprintf(stderr, "fixture line %d: %s\n", line, expression);
    exit(1);
}
#define CHECK(expression) do { if (!(expression)) failed(__LINE__, #expression); } while (0)

static int forbidden(const char *name)
{
    fprintf(stderr, "forbidden native path: %s\n", name);
    exit(90);
    return -1;
}

static struct {
    pid_t creator, sid;
    uid_t uid;
    siginfo_t event;
    int wait_rc, wait_error;
    struct proc_bsdinfo bsd;
    int bsd_bytes, bsd_error, sid_error;
    int policy_rc, policy_error, policy_flags, policy_ignored;
    struct timespec times[8];
    int time_rc[8], time_error[8];
    unsigned clocks, pids, uids, policies, waits, bsds, sids, frees;
    void *only_free;
} m;

static unsigned calls(void)
{
    return m.clocks + m.pids + m.uids + m.policies + m.waits + m.bsds + m.sids + m.frees;
}

static pid_t fake_getpid(void) { ++m.pids; return m.creator; }
static uid_t fake_getuid(void) { ++m.uids; return m.uid; }
static pid_t fake_getsid(pid_t pid)
{
    CHECK(pid == 41); ++m.sids; errno = m.sid_error; return m.sid;
}
static int fake_sigaction(int signal_number, const struct sigaction *input, struct sigaction *output)
{
    CHECK(signal_number == SIGCHLD && input == NULL && output != NULL);
    ++m.policies;
    memset(output, 0, sizeof(*output));
    output->sa_handler = m.policy_ignored ? SIG_IGN : SIG_DFL;
    output->sa_flags = m.policy_flags;
    errno = m.policy_error;
    return m.policy_rc;
}
static int fake_clock_gettime(clockid_t clock, struct timespec *output)
{
    CHECK(clock == CLOCK_MONOTONIC && output != NULL && m.clocks < 8);
    unsigned index = m.clocks++;
    *output = m.times[index];
    errno = m.time_error[index];
    return m.time_rc[index];
}
static int fake_waitid(idtype_t type, id_t pid, siginfo_t *output, int options)
{
    /* No consuming wait, stopped-event query or group wait can pass this stub. */
    CHECK(type == P_PID && pid == 41 && output != NULL);
    CHECK(options == (WEXITED | WNOWAIT | WNOHANG));
    ++m.waits; memcpy(output, &m.event, sizeof(*output));
    errno = m.wait_error; return m.wait_rc;
}
static int fake_proc_pidinfo(int pid, int flavor, uint64_t arg, void *output, int size)
{
    CHECK(pid == 41 && flavor == PROC_PIDTBSDINFO && arg == 0);
    CHECK(output != NULL && size == sizeof(struct proc_bsdinfo));
    ++m.bsds;
    size_t copied = m.bsd_bytes <= 0 ? 0 : (size_t)m.bsd_bytes;
    if (copied > sizeof(m.bsd)) copied = sizeof(m.bsd);
    memcpy(output, &m.bsd, copied);
    errno = m.bsd_error;
    return m.bsd_bytes;
}
static void fake_free(void *pointer)
{
    /* Only the deliberately synthetic handoff test may reach this stub. */
    CHECK(pointer != NULL && pointer == m.only_free && m.frees == 0);
    ++m.frees;
}

/* SDK declarations above precede every macro. These never call the real APIs.
 * Traps cover every other process/resource/allocator entry in the included C.
 */
#define getpid() fake_getpid()
#define getuid() fake_getuid()
#define getsid(...) fake_getsid(__VA_ARGS__)
#define sigaction(...) fake_sigaction(__VA_ARGS__)
#define clock_gettime(...) fake_clock_gettime(__VA_ARGS__)
#define waitid(...) fake_waitid(__VA_ARGS__)
#define proc_pidinfo(...) fake_proc_pidinfo(__VA_ARGS__)
#define proc_listpids(...) forbidden("proc_listpids")
#define kill(...) forbidden("kill")
#define killpg(...) forbidden("killpg")
#define waitpid(...) forbidden("waitpid")
#define fcntl(...) forbidden("fcntl")
#define close(...) forbidden("close")
#define posix_spawn(...) forbidden("posix_spawn")
#define posix_spawn_file_actions_init(...) forbidden("posix_spawn_file_actions_init")
#define posix_spawn_file_actions_adddup2(...) forbidden("posix_spawn_file_actions_adddup2")
#define posix_spawn_file_actions_addclose(...) forbidden("posix_spawn_file_actions_addclose")
#define posix_spawn_file_actions_addchdir_np(...) forbidden("posix_spawn_file_actions_addchdir_np")
#define posix_spawn_file_actions_destroy(...) forbidden("posix_spawn_file_actions_destroy")
#define posix_spawnattr_init(...) forbidden("posix_spawnattr_init")
#define posix_spawnattr_setflags(...) forbidden("posix_spawnattr_setflags")
#define posix_spawnattr_destroy(...) forbidden("posix_spawnattr_destroy")
#define calloc(...) ((void *)(uintptr_t)forbidden("calloc"))
#define free(...) fake_free(__VA_ARGS__)
#include "window_owned_child_darwin.c"
_Static_assert(sizeof(woc_active_observation) == 344, "active mapping v1 layout");

#if TEST_ACTIVE_MOCK
_Static_assert(WOC_CHILD_NATIVE_QUALIFIED == 1, "private mock branch only");
_Static_assert(WOC_NATIVE_QUALIFIED == 0, "legacy stays disabled in mocks");

static woc_child c;
static woc_terminal t;
static int bridge_key, other_key;
static const uint64_t generation = 23;

static void setup(void)
{
    memset(&m, 0, sizeof(m));
    memset(&c, 0, sizeof(c));
    memset(&t, 0, sizeof(t));
    memset(terminal_roots, 0, sizeof(terminal_roots));
    child_root = &c;
    terminal_roots[0] = &t;
    terminal_count = 1;
    c.key = t.key = &bridge_key;
    c.terminal = &t;
    c.spawn_pid = c.owner.pid = c.owner.pgid = c.owner.session = 41;
    c.owner.parent = 99;
    c.owner.uid = 501;
    c.report.generation = t.report.generation = generation;
    c.report.state = WOC_CHILD_ACTIVE;
    c.report.published = c.report.pid_valid = c.report.registered = 1;
    c.report.spawn_entered = c.report.spawn_returned = 1;
    c.report.spawn_pid_raw = 41;
    c.report.resume_entered = c.report.resume_returned = 1;
    c.report.registration_pid = c.report.registration_pgid = c.report.registration_sid = 41;
    c.report.registration_ppid = 99;
    c.report.registration_uid = 501;
    c.report.registration_bsd_status = SSTOP;
    c.report.resources_closed = 1;
    /* Canaries establish successful observations do not touch cleanup facts. */
    c.report.cleanup_entries = 7;
    c.report.max_cleanup_entries = 128;
    c.report.cleanup_deadline_ns = 777;
    c.report.cleanup_kill_at_ns = 333;
    c.report.cleanup_now_ns = 222;
    t.report.rooted = 1;
    t.report.raw_siginfo_bytes = sizeof(t.raw_siginfo);
    c.registration_info.pbi_pid = c.registration_info.pbi_pgid = 41;
    c.registration_info.pbi_ppid = 99;
    c.registration_info.pbi_uid = 501;
    c.registration_info.pbi_status = SSTOP;
    c.registration_info.pbi_start_tvsec = 123;
    c.registration_info.pbi_start_tvusec = 45;
    m.creator = 99; m.uid = 501; m.sid = 41;
    m.bsd = c.registration_info;
    m.bsd.pbi_status = SRUN;
    m.bsd_bytes = sizeof(m.bsd);
    for (unsigned index = 0; index < 8; ++index) m.times[index].tv_sec = index + 1;
}

static int observe_active(void)
{
    return woc_child_active_observe(&c, &bridge_key, generation);
}

static woc_active_observation snapshot(int first, const void **raw, size_t *bytes)
{
    woc_active_observation out;
    memset(&out, 0xAA, sizeof(out));
    CHECK(woc_terminal_active_snapshot(&t, &bridge_key, generation, first, &out, raw, bytes) == WOC_OK);
    CHECK(out.bytes == sizeof(out) && out.version == WOC_ACTIVE_VERSION && out.generation == generation);
    return out;
}

static void event(int code, int status)
{
    memset(&m.event, 0, sizeof(m.event));
    m.event.si_pid = 41;
    m.event.si_signo = SIGCHLD;
    m.event.si_code = code;
    m.event.si_status = status;
    m.event.si_uid = 991; /* Deliberately foreign raw UID must never become authority. */
}

static void expect_entered_failure(int result)
{
    CHECK(observe_active() == result);
    CHECK(t.active.entered && t.active.returned && t.active.sequence == 1);
    CHECK(t.active.rc == result && t.active.record.error == result);
    CHECK(c.report.blocked && c.report.state == WOC_CHILD_BLOCKED && !c.busy);
    CHECK(child_root == &c && terminal_roots[0] == &t && !c.invalid);
    unsigned before = calls();
    CHECK(observe_active() == WOC_OWNERSHIP);
    CHECK(calls() == before);
    const void *raw; size_t bytes;
    woc_active_observation out = snapshot(0, &raw, &bytes);
    CHECK(out.rc == result && out.sequence == 1 && calls() == before);
    CHECK(bytes == (out.wait_entered && out.wait_returned ? sizeof(siginfo_t) : 0));
}

static void live_repeat_cache(void)
{
    setup();
    woc_child_report before = c.report;
    woc_owner owner_before = c.owner;
    woc_terminal_report terminal_before = t.report;
    for (unsigned index = 1; index <= 2; ++index) {
        CHECK(observe_active() == WOC_OK);
        CHECK(t.active.sequence == index && t.active.entered && t.active.returned);
        CHECK(t.active.record.state == WOC_LIVE && t.active.record.uid_evidence == WOC_UID_CURRENT);
        CHECK(t.active.record.uid == 501 && t.active.record.reaped == 0 && t.active.record.no_signals == 0);
        CHECK(t.active.birth_seconds == 123 && t.active.birth_microseconds == 45);
        CHECK(t.active.wait_entered && t.active.wait_returned && t.active.bsd_entered && t.active.bsd_returned);
        CHECK(t.active.started_ns < t.active.finished_ns && !c.busy && !t.active_exit_saved);
        CHECK(memcmp(&c.report, &before, sizeof(before)) == 0);
        CHECK(memcmp(&c.owner, &owner_before, sizeof(owner_before)) == 0);
        CHECK(memcmp(&t.report, &terminal_before, sizeof(terminal_before)) == 0);
        unsigned call_count = calls();
        const void *raw; size_t bytes;
        woc_active_observation latest = snapshot(0, &raw, &bytes);
        CHECK(latest.sequence == index && bytes == sizeof(siginfo_t));
        CHECK(raw == &t.active_raw && memcmp(raw, &m.event, bytes) == 0);
        latest = snapshot(1, &raw, &bytes);
        CHECK(!latest.entered && latest.sequence == 0 && raw == NULL && bytes == 0);
        CHECK(calls() == call_count);
    }
    CHECK(m.waits == 2 && m.bsds == 2 && m.sids == 2 && m.clocks == 4);
}

static void live_statuses(void)
{
    const int statuses[] = {SIDL, SRUN, SSLEEP, SSTOP};
    for (unsigned index = 0; index < sizeof(statuses) / sizeof(statuses[0]); ++index) {
        setup(); m.bsd.pbi_status = statuses[index];
        CHECK(observe_active() == WOC_OK && t.active.record.state == WOC_LIVE);
        CHECK(t.active.bsd_status == statuses[index] && m.sids == 1);
    }
}

static void terminal_kinds(void)
{
    const int codes[] = {CLD_EXITED, CLD_EXITED, CLD_KILLED, CLD_DUMPED};
    const int statuses[] = {0, 255, SIGTERM, SIGABRT};
    const int kinds[] = {WOC_EXIT_NORMAL, WOC_EXIT_NORMAL, WOC_EXIT_SIGNAL, WOC_EXIT_CORE};
    for (unsigned index = 0; index < 4; ++index) {
        setup(); event(codes[index], statuses[index]);
        CHECK(observe_active() == WOC_OK && t.active.record.state == WOC_EXITED);
        CHECK(t.active.record.exit_kind == kinds[index] && t.active.record.exit_status == statuses[index]);
        CHECK(t.active.record.returncode == (codes[index] == CLD_EXITED ? statuses[index] : -statuses[index]));
        CHECK(t.active.record.uid == 501 && t.active.record.uid_evidence == WOC_UID_RETAINED);
        CHECK(!t.active.record.wait_uid_available && !t.active.record.reaped && !t.active.record.reap_attempted);
        CHECK(!t.active.record.no_signals && t.active_exit_saved);
        CHECK(m.waits == 1 && m.bsds == 0 && m.sids == 0 && m.clocks == 2);
        CHECK(memcmp(&t.first_active_raw, &m.event, sizeof(m.event)) == 0);
    }
}

static void live_then_terminal(void)
{
    setup(); CHECK(observe_active() == WOC_OK);
    event(CLD_EXITED, 7); CHECK(observe_active() == WOC_OK);
    CHECK(t.active.sequence == 2 && t.first_active_exit.sequence == 2);
    CHECK(t.active.record.uid_evidence == WOC_UID_RETAINED && t.active.record.uid == 501);
    CHECK(t.active.record.returncode == 7 && !t.active.bsd_entered && !t.active.observed_sid);
    CHECK(m.bsds == 1 && m.sids == 1);
}

static void first_terminal_stable(void)
{
    setup(); event(CLD_KILLED, SIGTERM); CHECK(observe_active() == WOC_OK);
    woc_active_observation first = t.first_active_exit;
    siginfo_t first_raw = t.first_active_raw;
    m.event.si_uid = 17;
    CHECK(observe_active() == WOC_OK && t.active.sequence == 2);
    CHECK(memcmp(&first, &t.first_active_exit, sizeof(first)) == 0);
    CHECK(memcmp(&first_raw, &t.first_active_raw, sizeof(first_raw)) == 0);
    const void *raw; size_t bytes; unsigned before = calls();
    woc_active_observation out = snapshot(1, &raw, &bytes);
    CHECK(out.sequence == 1 && raw == &t.first_active_raw && bytes == sizeof(first_raw));
    CHECK(memcmp(raw, &first_raw, bytes) == 0 && calls() == before);
}

static void terminal_contradictions(void)
{
    for (int mismatch = 0; mismatch < 2; ++mismatch) {
        setup(); event(CLD_EXITED, 7); CHECK(observe_active() == WOC_OK);
        woc_active_observation first = t.first_active_exit;
        siginfo_t raw = t.first_active_raw;
        if (mismatch) memset(&m.event, 0, sizeof(m.event));
        else event(CLD_EXITED, 8);
        CHECK(observe_active() == WOC_OWNERSHIP && c.report.blocked && t.active.sequence == 2);
        CHECK(memcmp(&first, &t.first_active_exit, sizeof(first)) == 0);
        CHECK(memcmp(&raw, &t.first_active_raw, sizeof(raw)) == 0 && m.bsds == 0);
    }
}

static void wait_failures(void)
{
    for (int mutation = 0; mutation < 10; ++mutation) {
        setup();
        switch (mutation) {
            case 0: m.wait_rc = -1; m.wait_error = EINTR; break;
            case 1: event(CLD_EXITED, 0); m.event.si_pid = 42; break;
            case 2: m.event.si_signo = SIGCHLD; break;
            case 3: m.event.si_code = CLD_EXITED; break;
            case 4: m.event.si_status = 1; break;
            case 5: event(CLD_EXITED, 0); m.event.si_signo = SIGUSR1; break;
            case 6: event(CLD_STOPPED, 0); break;
            case 7: event(CLD_EXITED, 256); break;
            case 8: event(CLD_KILLED, 0); break;
            case 9: event(CLD_DUMPED, NSIG); break;
        }
        expect_entered_failure(mutation == 0 ? WOC_SYSTEM : WOC_OWNERSHIP);
        CHECK(m.waits == 1 && m.bsds == 0 && !t.active_exit_saved);
        CHECK(memcmp(&t.active_raw, &m.event, sizeof(m.event)) == 0);
        if (mutation == 0) CHECK(t.active.wait_errno == EINTR && t.active.system_errno == EINTR);
    }
}

static void bsd_identity(void)
{
    for (int mutation = 0; mutation < 6; ++mutation) {
        setup();
        switch (mutation) {
            case 0: ++m.bsd.pbi_pid; break;
            case 1: ++m.bsd.pbi_ppid; break;
            case 2: ++m.bsd.pbi_uid; break;
            case 3: ++m.bsd.pbi_pgid; break;
            case 4: ++m.bsd.pbi_start_tvsec; break;
            case 5: ++m.bsd.pbi_start_tvusec; break;
        }
        expect_entered_failure(WOC_OWNERSHIP);
        CHECK(m.bsds == 1 && m.sids == 0 && !t.active_exit_saved);
        CHECK(memcmp(&t.active_bsd, &m.bsd, sizeof(m.bsd)) == 0);
    }
}

static void bsd_failures(void)
{
    for (int mutation = 0; mutation < 5; ++mutation) {
        setup();
        switch (mutation) {
            case 0: m.bsd_bytes = 0; m.bsd_error = ESRCH; break;
            case 1: m.bsd_bytes = sizeof(m.bsd) - 1; m.bsd_error = EIO; break;
            case 2: m.bsd.pbi_status = 999; break;
            case 3: m.sid = 42; break;
            case 4: m.sid = -1; m.sid_error = ESRCH; break;
        }
        expect_entered_failure(mutation == 0 || mutation == 4 ? WOC_SYSTEM :
                               mutation == 1 ? WOC_INCONCLUSIVE : WOC_OWNERSHIP);
        CHECK(m.bsds == 1 && m.sids == (unsigned)(mutation >= 3));
        CHECK(!t.active_exit_saved);
    }
}

static void creator_identity(void)
{
    for (int mutation = 0; mutation < 2; ++mutation) {
        setup(); if (mutation) ++m.uid; else ++m.creator;
        expect_entered_failure(WOC_OWNERSHIP);
        CHECK(m.pids == 1 && m.uids == 1 && m.policies == 0 && m.waits == 0);
    }
}

static void disposition_failures(void)
{
    for (int mutation = 0; mutation < 3; ++mutation) {
        setup();
        if (mutation == 0) { m.policy_rc = -1; m.policy_error = EPERM; }
        if (mutation == 1) m.policy_ignored = 1;
        if (mutation == 2) m.policy_flags = SA_NOCLDWAIT;
        expect_entered_failure(mutation == 0 ? WOC_SYSTEM : WOC_OWNERSHIP);
        CHECK(m.policies == 1 && m.waits == 0 && !t.active.wait_entered);
    }
}

static void exit_transition(void)
{
    for (int mutation = 0; mutation < 2; ++mutation) {
        setup();
        if (mutation) m.bsd.pbi_flags |= PROC_FLAG_INEXIT;
        else m.bsd.pbi_status = SZOMB;
        CHECK(observe_active() == WOC_OK && t.active.record.state == WOC_ACTIVE_TRANSITION);
        CHECK(t.active.record.uid_evidence == WOC_UID_RETAINED && !t.active_exit_saved);
        CHECK(!t.active.observed_sid && m.sids == 0 && !c.report.blocked);
        CHECK(!c.owner.reaped && !c.owner.reap_attempted && !c.owner.no_signals);
        event(CLD_EXITED, 0);
        CHECK(observe_active() == WOC_OK && t.active.record.state == WOC_EXITED);
        CHECK(t.active.sequence == 2 && t.first_active_exit.sequence == 2);
    }
}

static void clock_failures(void)
{
    for (int mutation = 0; mutation < 7; ++mutation) {
        setup();
        switch (mutation) {
            case 0: m.time_rc[0] = -1; m.time_error[0] = EIO; break;
            case 1: m.times[0].tv_sec = -1; break;
            case 2: m.times[0].tv_nsec = 1000000000L; break;
            case 3: m.times[0].tv_sec = (time_t)(UINT64_MAX / 1000000000ULL + 1); break;
            case 4: m.time_rc[1] = -1; m.time_error[1] = EINVAL; break;
            case 5: m.times[1].tv_nsec = -1; break;
            case 6: m.times[1].tv_sec = 0; break;
        }
        expect_entered_failure(mutation == 0 || mutation == 4 ? WOC_SYSTEM : WOC_INCONCLUSIVE);
        CHECK(m.clocks == 2 && !t.active_exit_saved);
        CHECK(m.waits == (unsigned)(mutation >= 4));
    }
}

static void failed_terminal_clock(void)
{
    setup(); event(CLD_EXITED, 17); m.time_rc[1] = -1; m.time_error[1] = EIO;
    expect_entered_failure(WOC_SYSTEM);
    CHECK(t.active_exit_saved && t.first_active_exit.rc == WOC_SYSTEM);
    CHECK(t.first_active_exit.record.state == WOC_EXITED && t.first_active_exit.record.error == WOC_SYSTEM);
    CHECK(t.first_active_exit.record.returncode == 17 && t.first_active_exit.finish_clock_errno == EIO);
    CHECK(memcmp(&t.first_active_raw, &m.event, sizeof(m.event)) == 0);
}

static void admission_guards(void)
{
    for (int mutation = 0; mutation < 29; ++mutation) {
        setup();
        woc_child *argument = &c; const void *key = &bridge_key; uint64_t gen = generation;
        switch (mutation) {
            case 0: argument = NULL; break;
            case 1: argument = (woc_child *)(uintptr_t)1; break;
            case 2: child_root = NULL; break;
            case 3: key = &other_key; break;
            case 4: gen = 0; break;
            case 5: ++gen; break;
            case 6: c.invalid = 1; break;
            case 7: c.busy = 1; break;
            case 8: c.report.abort.started = 1; break;
            case 9: c.report.blocked = 1; break;
            case 10: c.report.state = WOC_CHILD_SUSPENDED; break;
            case 11: c.report.published = 0; break;
            case 12: c.report.resume_entered = 0; break;
            case 13: c.report.resume_returned = 0; break;
            case 14: c.report.resume_rc = -1; break;
            case 15: c.report.cleanup_started = 1; break;
            case 16: c.owner.reap_attempted = 1; break;
            case 17: c.owner.reaped = 1; break;
            case 18: c.owner.no_signals = 1; break;
            case 19: t.report.wait_entered = 1; break;
            case 20: t.report.reap_attempted = 1; break;
            case 21: c.registration_info.pbi_start_tvsec = 0; break;
            case 22: c.registration_info.pbi_start_tvusec = 1000000; break;
            case 23: c.report.spawn_returned = 0; break;
            case 24: c.report.spawn_pid_raw = 42; break;
            case 25: c.registration_info.pbi_ppid = 100; break;
            case 26: t.report.known_no_creation = 1; break;
            case 27: t.report.transferred = 1; break;
            case 28: terminal_count = 0; break;
        }
        woc_child_report before = c.report;
        woc_active_observation active_before = t.active;
        CHECK(woc_child_active_observe(argument, key, gen) == WOC_OWNERSHIP);
        CHECK(calls() == 0 && memcmp(&c.report, &before, sizeof(before)) == 0);
        CHECK(memcmp(&t.active, &active_before, sizeof(active_before)) == 0);
    }
}

static void entry_budget(void)
{
    setup(); t.active.sequence = WOC_ACTIVE_OBSERVATIONS;
    woc_active_observation before = t.active;
    CHECK(observe_active() == WOC_INCONCLUSIVE && calls() == 0 && !c.report.blocked);
    CHECK(memcmp(&before, &t.active, sizeof(before)) == 0);
    setup(); t.active.sequence = WOC_ACTIVE_OBSERVATIONS - 1;
    CHECK(observe_active() == WOC_OK && t.active.sequence == WOC_ACTIVE_OBSERVATIONS);
    unsigned count = calls(); before = t.active;
    CHECK(observe_active() == WOC_INCONCLUSIVE && calls() == count && !c.report.blocked);
    CHECK(memcmp(&before, &t.active, sizeof(before)) == 0);
}

static void snapshot_guards(void)
{
    setup(); const void *raw; size_t bytes;
    woc_active_observation out = snapshot(0, &raw, &bytes);
    CHECK(!out.entered && !out.returned && !out.sequence && raw == NULL && bytes == 0);
    out = snapshot(1, &raw, &bytes);
    CHECK(!out.entered && raw == NULL && bytes == 0 && calls() == 0);
    CHECK(woc_terminal_active_snapshot(&t, &bridge_key, generation, 2, &out, &raw, &bytes) == WOC_ABI);
    CHECK(woc_terminal_active_snapshot(&t, &bridge_key, generation, 0, NULL, &raw, &bytes) == WOC_ABI);
    CHECK(woc_terminal_active_snapshot(&t, &bridge_key, generation, 0, &out, NULL, &bytes) == WOC_ABI);
    CHECK(woc_terminal_active_snapshot(&t, &bridge_key, generation, 0, &out, &raw, NULL) == WOC_ABI);
    CHECK(woc_terminal_active_snapshot((woc_terminal *)(uintptr_t)1, &bridge_key, generation, 0,
                                       &out, &raw, &bytes) == WOC_OWNERSHIP);
    CHECK(woc_terminal_active_snapshot(&t, &other_key, generation, 0, &out, &raw, &bytes) == WOC_OWNERSHIP);
    CHECK(woc_terminal_active_snapshot(&t, &bridge_key, generation + 1, 0, &out, &raw, &bytes) == WOC_OWNERSHIP);
    terminal_count = 0;
    CHECK(woc_terminal_active_snapshot(&t, &bridge_key, generation, 0, &out, &raw, &bytes) == WOC_OWNERSHIP);
    CHECK(calls() == 0);
}

static void handoff_retention(void)
{
    setup(); event(CLD_EXITED, 0); CHECK(observe_active() == WOC_OK);
    woc_active_observation first = t.first_active_exit;
    siginfo_t first_raw = t.first_active_raw;
    /* These collection facts are synthetic fixture inputs. No wait/reap runs.
     * The actual memory handoff implementation then removes the Child root;
     * fake_free retains static test storage rather than freeing a real owner.
     */
    c.report.cleanup_started = 1;
    c.report.cleanup_deadline_ns = 100000000000ULL;
    c.report.state = WOC_CHILD_REAP_RECORDED;
    c.owner.reap_attempted = c.owner.reaped = c.owner.no_signals = 1;
    t.report.kind = WOC_TERMINAL_NORMAL_GROUP_CERTIFICATE;
    t.report.wait_returned = t.report.consumed = t.report.decoded = t.report.certificate_matches = 1;
    m.only_free = &c;
    CHECK(woc_child_handoff(&c, &bridge_key, generation, &t) == WOC_OK);
    CHECK(child_root == NULL && c.invalid && t.report.transferred && m.frees == 1);
    CHECK(t.report.final_report.state == WOC_CHILD_TRANSFERRED);
    unsigned before = calls(); const void *raw; size_t bytes;
    woc_active_observation out = snapshot(1, &raw, &bytes);
    CHECK(memcmp(&out, &first, sizeof(first)) == 0 && raw == &t.first_active_raw);
    CHECK(bytes == sizeof(first_raw) && memcmp(raw, &first_raw, bytes) == 0);
    out = snapshot(0, &raw, &bytes);
    CHECK(out.sequence == 1 && out.record.state == WOC_EXITED);
    CHECK(observe_active() == WOC_OWNERSHIP && calls() == before);
}

int main(int argc, char **argv)
{
    CHECK(argc == 2);
    struct { const char *name; void (*run)(void); } cases[] = {
        {"live_repeat_cache", live_repeat_cache}, {"live_statuses", live_statuses},
        {"terminal_kinds", terminal_kinds}, {"live_then_terminal", live_then_terminal},
        {"first_terminal_stable", first_terminal_stable}, {"terminal_contradictions", terminal_contradictions},
        {"wait_failures", wait_failures}, {"bsd_identity", bsd_identity},
        {"bsd_failures", bsd_failures}, {"creator_identity", creator_identity},
        {"disposition_failures", disposition_failures}, {"exit_transition", exit_transition},
        {"clock_failures", clock_failures}, {"failed_terminal_clock", failed_terminal_clock},
        {"admission_guards", admission_guards}, {"entry_budget", entry_budget},
        {"snapshot_guards", snapshot_guards}, {"handoff_retention", handoff_retention}
    };
    for (unsigned index = 0; index < sizeof(cases) / sizeof(cases[0]); ++index) {
        if (strcmp(argv[1], cases[index].name) == 0) {
            cases[index].run(); printf("PASS %s\n", cases[index].name); return 0;
        }
    }
    failed(__LINE__, "unknown mock case"); return 1;
}

#else
_Static_assert(WOC_CHILD_NATIVE_QUALIFIED == 0, "unmodified production gate required");
_Static_assert(WOC_NATIVE_QUALIFIED == 0, "unmodified legacy gate required");
int main(int argc, char **argv)
{
    CHECK(argc == 2 && strcmp(argv[1], "production_gate") == 0);
    CHECK(woc_child_active_observe((woc_child *)(uintptr_t)1, (const void *)(uintptr_t)1,
                                   UINT64_MAX) == WOC_UNSUPPORTED);
    CHECK(woc_terminal_active_snapshot((woc_terminal *)(uintptr_t)1,
            (const void *)(uintptr_t)1, UINT64_MAX, 99,
            (woc_active_observation *)(uintptr_t)1, (const void **)(uintptr_t)1,
            (size_t *)(uintptr_t)1) == WOC_UNSUPPORTED);
    CHECK(woc_child_active_observe(NULL, NULL, 0) == WOC_UNSUPPORTED);
    CHECK(woc_terminal_active_snapshot(NULL, NULL, 0, 0, NULL, NULL, NULL) == WOC_UNSUPPORTED);
    CHECK(calls() == 0);
    printf("PASS production_gate\n"); return 0;
}
#endif
