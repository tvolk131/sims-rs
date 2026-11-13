#ifndef SIMS_WINDOW_OWNED_CHILD_DARWIN_H
#define SIMS_WINDOW_OWNED_CHILD_DARWIN_H

#include <stddef.h>
#include <stdint.h>

enum {
    WOC_VERSION = 1,
    WOC_NATIVE_QUALIFIED = 0,
    WOC_OK = 0,
    WOC_ABI = 1,
    WOC_UNSUPPORTED = 2,
    WOC_SYSTEM = 3,
    WOC_OWNERSHIP = 4,
    WOC_INCONCLUSIVE = 5,
    WOC_LIVE = 1,
    WOC_EXITED = 2,
    WOC_EXIT_NORMAL = 1,
    WOC_EXIT_SIGNAL = 2,
    WOC_EXIT_CORE = 3,
    WOC_UID_CURRENT = 1,
    WOC_UID_RETAINED = 2,
    WOC_TERM = 1,
    WOC_KILL = 2
};

typedef struct {
    uint32_t bytes;
    uint32_t version;
    int32_t error;
    int32_t system_errno;
    uint32_t record_bytes;
    uint32_t pid_bytes;
    uint32_t platform;
    uint32_t enabled;
} woc_description;

typedef struct {
    uint32_t bytes;
    uint32_t version;
    int32_t error;
    int32_t system_errno;
    int64_t pid;
    int64_t pgid;
    int64_t uid;
    int64_t ppid;
    int64_t sid;
    int64_t state;
    int64_t exit_kind;
    int64_t exit_status;
    int64_t wait_uid_available;
    int64_t uid_evidence;
    int64_t sigchld_default;
    int64_t no_cldwait;
    int64_t raw_bytes;
    int64_t pid_bytes;
    int64_t capacity;
    int64_t count;
    int64_t member0;
    int64_t member1;
    int64_t no_signals;
    int64_t reap_attempted;
    int64_t reaped;
    int64_t returncode;
    int64_t signal_attempted;
} woc_record;

typedef struct woc_owner woc_owner;

int woc_describe(woc_description *out, size_t bytes, uint32_t version);
int woc_open(int64_t pid, int64_t pgid, int64_t uid, woc_owner **owner,
             woc_record *out, size_t bytes, uint32_t version);
int woc_observe(woc_owner *owner, woc_record *out, size_t bytes, uint32_t version);
int woc_members(woc_owner *owner, woc_record *out, size_t bytes, uint32_t version);
int woc_member(woc_owner *owner, int64_t pid, woc_record *out,
               size_t bytes, uint32_t version);
int woc_signal(woc_owner *owner, int kind, int64_t positive_member,
               woc_record *out, size_t bytes, uint32_t version);
int woc_latch(woc_owner *owner, int64_t exit_kind, int64_t exit_status,
              woc_record *out, size_t bytes, uint32_t version);
int woc_reap(woc_owner *owner, woc_record *out, size_t bytes, uint32_t version);
int woc_release(woc_owner *owner, woc_record *out, size_t bytes, uint32_t version);

/* Additive source-only Child protocol. This does not qualify the legacy API.
 * Child v1 treats every wait-output UID as unavailable diagnostic data: raw and
 * scalar copies are preserved, but never establish owner identity or equality
 * across events. Child records always have wait_uid_available == 0. Before BSD
 * registration uid is only the creator's expected UID and uid_evidence is zero;
 * accepted fresh BSD queries use CURRENT, terminal/copy evidence uses RETAINED.
 * CURRENT describes the UID query; other tuple fields may remain retained.
 * Terminal uid is the registered owner UID, not a final credential observation.
 */
enum {
    WOC_CHILD_VERSION = 1,
    WOC_CHILD_NATIVE_QUALIFIED = 0,
    WOC_CHILD_STRING_BYTES = 65536,
    WOC_CHILD_VECTOR_ITEMS = 128,
    WOC_CHILD_HISTORY = 64,
    WOC_CHILD_LAUNCH_FACTS = 64,
    WOC_CHILD_PREPARED = 1,
    WOC_CHILD_SPAWN_ENTERED = 2,
    WOC_CHILD_SPAWN_UNKNOWN = 3,
    WOC_CHILD_SUSPENDED = 4,
    WOC_CHILD_RESUME_ENTERED = 5,
    WOC_CHILD_ACTIVE = 6,
    WOC_CHILD_ABORTED = 7,
    WOC_CHILD_BLOCKED = 8,
    WOC_CHILD_REAP_ENTERED = 9,
    WOC_CHILD_REAP_RECORDED = 10,
    WOC_CHILD_TRANSFERRED = 11
};

typedef struct { const char *data; size_t bytes; } woc_child_string;
typedef struct {
    woc_child_string executable;
    const woc_child_string *argv;
    size_t argc;
    const woc_child_string *env;
    size_t envc;
    woc_child_string cwd;
    int stdio[3];
    uint64_t launch_timeout_ns;
    uint64_t cleanup_timeout_ns;
    uint64_t grace_ns;
    uint32_t max_cleanup_entries; /* 1..512 total admitted backend entries, never OS syscalls */
} woc_child_spec;

typedef struct {
    int kind;
    int entered;
    int returned;
    int64_t raw_rc;
    int raw_errno;
} woc_child_call_fact;

/* Additive active observation ABI. No existing Child/terminal/legacy report
 * layout changes. Each call is finite, with a fixed lifetime entry cap; the
 * coordinator still owns its absolute session deadline. This is never evidence
 * that status was consumed or a process was collected. */
enum {
    WOC_ACTIVE_VERSION = 1, WOC_ACTIVE_OBSERVATIONS = 1048576,
    /* Active-only result: matching lifetime is exiting but no terminal event
     * was observed yet. Neither live/input-ready nor collected evidence. */
    WOC_ACTIVE_TRANSITION = 3
};
typedef struct {
    uint32_t bytes;
    uint32_t version;
    uint64_t generation;
    uint64_t sequence;
    uint64_t started_ns;
    uint64_t finished_ns;
    uint64_t birth_seconds;
    uint64_t birth_microseconds;
    int entered;
    int returned;
    int rc;
    int system_errno;
    int wait_entered;
    int wait_returned;
    int wait_rc;
    int wait_errno;
    int bsd_entered;
    int bsd_returned;
    int bsd_bytes;
    int bsd_status;
    int start_clock_rc;
    int start_clock_errno;
    int finish_clock_rc;
    int finish_clock_errno;
    int64_t creator_pid;
    int64_t creator_uid;
    int64_t observed_sid;
    woc_record record;
} woc_active_observation;

/* Fixed, pre-spawn reserved launch-abort domain. No caller budgets or adoption. */
enum {
    WOC_ABORT_ENTRIES = 16, WOC_ABORT_OBSERVATIONS = 8,
    WOC_ABORT_RAW_SLOTS = 9, WOC_ABORT_FACTS = 64,
    WOC_ABORT_NONE_RESERVED = 0, WOC_ABORT_BEGIN_ENTERED,
    WOC_ABORT_KILL_ENTERED, WOC_ABORT_WAITING_EXIT, WOC_ABORT_EXIT_READY,
    WOC_ABORT_REAP_ENTERED, WOC_ABORT_REAP_RECORDED, WOC_ABORT_TRANSFERRED,
    WOC_ABORT_FAILED_RETAINED,
    WOC_ABORT_OP_NONE = 0, WOC_ABORT_OP_BEGIN, WOC_ABORT_OP_OBSERVE,
    WOC_ABORT_OP_REAP, WOC_ABORT_OP_HANDOFF,
    WOC_TERMINAL_UNKNOWN = 0, WOC_TERMINAL_NO_CREATION,
    WOC_TERMINAL_NORMAL_GROUP_CERTIFICATE, WOC_TERMINAL_DIRECT_CHILD_ABORT_UNRESUMED
};
enum {
    WOC_LF_PREPARATION = 1, WOC_LF_PRESPAWN_CLOCK, WOC_LF_PRESPAWN_DEADLINE,
    WOC_LF_PREFLIGHT_DISPOSITION, WOC_LF_SPAWN_UNKNOWN, WOC_LF_REGISTRATION_POLICY,
    WOC_LF_REGISTRATION_QUERY, WOC_LF_REGISTRATION_IDENTITY, WOC_LF_RESOURCE_ERROR,
    WOC_LF_PRE_RESUME_CLOCK, WOC_LF_PRE_RESUME_DEADLINE, WOC_LF_TRACE_CAPACITY,
    WOC_LF_RESUME_ENTERED_FAILURE, WOC_LF_PUBLICATION_OR_OWNERSHIP
};
#define WOC_LF_MASK(reason) (UINT64_C(1) << ((reason) - 1))

typedef struct {
    int entered;
    int returned;
    int rc;
    int system_errno;
    size_t raw_siginfo_bytes;
    int64_t pid;
    int64_t uid;
    int code;
    int status;
} woc_abort_observation;

/* Diagnostic facts only: raw wait output, never registration/creation proof. */
typedef struct {
    int entered;
    int returned;
    int rc;
    int system_errno;
    int idtype;
    int64_t target_pid;
    int options;
    size_t raw_siginfo_bytes;
    int signo;
    int code;
    int64_t pid;
    int64_t uid;
    int status;
} woc_registration_observation;

typedef struct {
    int last_operation;
    int last_rc;
    int last_system_errno;
    int phase;
    int started;
    uint32_t entries;
    uint32_t max_entries;
    uint64_t deadline_ns;
    uint64_t origin_ns;
    uint64_t now_ns;
    uint32_t observation_count;
    uint32_t max_observations;
    uint32_t trace_count;
    woc_child_call_fact trace[WOC_ABORT_FACTS];
    woc_abort_observation observations[WOC_ABORT_RAW_SLOTS];
    int failure_rc;                  /* first admitted abort failure, sticky */
    int failure_system_errno;
    int kill_entered;
    int kill_returned;
    int kill_rc;
    int kill_errno;
    int preexisting_exit;
    int exit_observation_index;
    int exit_valid;
    int bsd_status;
    int64_t bsd_pid;
    int64_t bsd_ppid;
    int64_t bsd_uid;
    int64_t bsd_pgid;
    int64_t sid;
    woc_record record;               /* never replaces original launch record */
} woc_launch_abort_report;

typedef struct {
    uint64_t generation;
    int state;
    int rc;
    int system_errno;
    int blocked;
    int published;
    int pid_valid;
    int registered;
    int registration_bsd_status;
    int64_t registration_pid;
    int64_t registration_ppid;
    int64_t registration_pgid;
    int64_t registration_uid;
    int64_t registration_sid;
    int spawn_entered;
    int spawn_returned;
    int spawn_rc;
    int spawn_errno;
    int64_t spawn_pid_raw;
    int resume_entered;
    int resume_returned;
    int resume_rc;
    int resume_errno;
    uint32_t launch_entries;
    uint32_t launch_fact_count;
    woc_child_call_fact launch_facts[WOC_CHILD_LAUNCH_FACTS];
    uint64_t launch_deadline_ns;
    int resources_closed;
    int resource_error;
    int cleanup_started;
    uint32_t cleanup_entries;
    uint32_t max_cleanup_entries;
    uint64_t cleanup_deadline_ns;
    uint64_t cleanup_kill_at_ns;
    uint64_t cleanup_now_ns;
    int clock_rc;
    int clock_errno;
    int term_attempted;
    int kill_attempted;
    int signal_failed;
    int term_returned;
    int term_rc;
    int term_errno;
    int kill_returned;
    int kill_rc;
    int kill_errno;
    int latch_attempted;
    woc_record record;
    /* Retained separately: child_record() clears record after spawn/observation.
     * Policy facts are valid only after a successful preflight sigaction query. */
    int sigchld_preflight_entered;
    int sigchld_preflight_returned;
    int sigchld_preflight_rc;
    int sigchld_preflight_errno;
    int sigchld_preflight_policy_valid;
    int sigchld_preflight_default;
    int sigchld_preflight_no_cldwait;
    int sigchld_preflight_accepted;
    uint64_t launch_failure_mask;
    int launch_first_failure_phase;
    int launch_first_failure_reason;
    int launch_first_woc_rc;
    int launch_first_system_errno;
    woc_launch_abort_report abort;
    woc_registration_observation registration_observation;
} woc_child_report;

typedef struct {
    uint64_t generation;
    int rooted;
    int transferred;
    int known_no_creation;
    int kind;
    int reap_attempted;
    int wait_entered;
    int wait_returned;
    int wait_rc;
    int wait_errno;
    size_t raw_siginfo_bytes;
    int64_t wait_pid;
    int64_t wait_uid;
    int wait_code;
    int wait_status;
    int consumed;
    int decoded;
    int certificate_matches;
    woc_record decoded_record;
    woc_child_report final_report;
    woc_registration_observation registration_observation;
} woc_terminal_report;

typedef struct woc_child woc_child;
typedef struct woc_terminal woc_terminal;

/* No adoption API. key is the strongly rooted exact extension Bridge identity.
 * prepare copies all strings, duplicates stdio, roots the attempt and reserves a
 * terminal root BEFORE spawn. A prepared attempt is NOT proof of a child.
 * Native registry/history live for this loaded library image's process lifetime;
 * there is deliberately no reset/forget/free-terminal entry point.
 */
int woc_child_prepare(const void *key, const woc_child_spec *spec, woc_child **out);
int woc_child_spawn(woc_child *child, const void *key, uint64_t generation);
int woc_child_abort_begin(woc_child *child, const void *key, uint64_t generation);
int woc_child_abort_observe(woc_child *child, const void *key, uint64_t generation);
int woc_child_abort_reap(woc_child *child, const void *key, uint64_t generation);
int woc_child_abort_handoff(woc_child *child, const void *key, uint64_t generation,
                            woc_terminal *sink);
/* SDK-sized immutable raw bytes from an entered+returned slot only; valid only
 * for the exact same-image rooted sink. Unentered/in-flight slots reject. */
int woc_terminal_abort_observation(woc_terminal *sink, const void *key,
                                   uint64_t generation, uint32_t index,
                                   const void **raw_siginfo, size_t *bytes);
/* Completed registration wait slot only, including failed-call output. Same
 * exact root/key/generation rules as abort raw reads; no registration authority. */
int woc_terminal_registration_observation(woc_terminal *sink, const void *key,
                                          uint64_t generation,
                                          const void **raw_siginfo, size_t *bytes);
int woc_child_resume(woc_child *child, const void *key, uint64_t generation);
int woc_child_publish(woc_child *child, const void *key, uint64_t generation);
/* Exact registered, resumed and published child only, before cleanup. Uses
 * WEXITED|WNOWAIT|WNOHANG and targeted BSD/SID reads; never a group census. */
int woc_child_active_observe(woc_child *child, const void *key, uint64_t generation);
/* Memory-only latest or first terminal observation (selector0/1), retained
 * through handoff/mapping failure. Empty slots have entered=0 and raw bytes0;
 * cached reads do not establish freshness or collection. */
int woc_terminal_active_snapshot(woc_terminal *sink, const void *key, uint64_t generation,
                                  int first_terminal, woc_active_observation *out,
                                  const void **raw_siginfo, size_t *bytes);
int woc_child_snapshot(woc_child *child, const void *key, uint64_t generation,
                       woc_child_report *out);
woc_child *woc_child_pending(const void *key, uint64_t *generation);
int woc_child_registry_used(void);
int woc_child_cleanup_begin(woc_child *child, const void *key, uint64_t generation);
int woc_child_observe(woc_child *child, const void *key, uint64_t generation);
int woc_child_members(woc_child *child, const void *key, uint64_t generation);
int woc_child_member(woc_child *child, const void *key, uint64_t generation, int64_t pid);
int woc_child_signal(woc_child *child, const void *key, uint64_t generation,
                     int kind, int64_t positive_member);
int woc_child_latch(woc_child *child, const void *key, uint64_t generation,
                    int64_t exit_kind, int64_t exit_status);
int woc_child_reap(woc_child *child, const void *key, uint64_t generation);
woc_terminal *woc_child_terminal(woc_child *child, const void *key, uint64_t generation);
int woc_child_handoff(woc_child *child, const void *key, uint64_t generation,
                      woc_terminal *sink);
int woc_terminal_snapshot(woc_terminal *sink, const void *key, uint64_t generation,
                          woc_terminal_report *out, const void **raw_siginfo);

#endif
