/* Actual C/PyC normal lifecycle under finite OS mocks. No roots or collected
 * facts are seeded. Only the real producer prepares/registers/publishes/latches/
 * consumes/transfers its roots. The runner changes exactly three gate assertions
 * in temporary copies; production and legacy gates remain zero.
 */
#ifndef _DARWIN_C_SOURCE
#define _DARWIN_C_SOURCE 1
#endif
#define PY_SSIZE_T_CLEAN
#include <Python.h>
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
#include <stddef.h>
#include <sys/proc.h>
#include <sys/proc_info.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

static void fixture_failure(int line, const char *expression)
{
    fprintf(stderr, "normal fixture line %d: %s\n", line, expression);
    exit(90);
}
#define CHECK(x) do { if (!(x)) fixture_failure(__LINE__, #x); } while (0)
static int forbidden(const char *name)
{
    fprintf(stderr, "forbidden real normal path: %s\n", name);
    exit(91); return -1;
}

static struct {
    unsigned clocks, pids, uids, policies, waits, consuming, bsds, sids, lists;
    unsigned duplicates, closes, actions, attributes, spawns, resumes, frees, allocations;
    unsigned mapping_failures;
    int loss_on_next_active, loss_on_next_reap;
    int resumed, mode, raw_errno, consume_error, consume_pid, consume_status;
    int resume_error, list_count, dictionary_countdown;
    uint64_t now_ns;
} m = {.consume_pid=41, .list_count=1, .dictionary_countdown=-1, .now_ns=1000000000};
_Alignas(max_align_t) static unsigned char arenas[2][262144];

static void *fake_calloc(size_t count, size_t size)
{
    CHECK(count==1 && size>0 && size<=sizeof(arenas[0]) && m.allocations<2);
    void *result=arenas[m.allocations++]; memset(result,0,size); return result;
}
static void fake_free(void *p)
{
    if (p==NULL) return;
    CHECK(p==arenas[0] && m.frees==0); ++m.frees;
}
static pid_t fake_getpid(void) { ++m.pids; return 99; }
static uid_t fake_getuid(void) { ++m.uids; return 501; }
static pid_t fake_getsid(pid_t pid)
{ CHECK(pid==41); ++m.sids; errno=m.raw_errno; return 41; }
static int fake_sigaction(int number,const struct sigaction *in,struct sigaction *out)
{
    CHECK(number==SIGCHLD && in==NULL && out!=NULL); ++m.policies;
    memset(out,0,sizeof(*out));out->sa_handler=SIG_DFL;errno=m.raw_errno;return 0;
}
static int fake_clock_gettime(clockid_t clock,struct timespec *out)
{
    CHECK(clock==CLOCK_MONOTONIC && out!=NULL && m.clocks<128);++m.clocks;
    m.now_ns+=1000000;out->tv_sec=(time_t)(m.now_ns/1000000000);
    out->tv_nsec=(long)(m.now_ns%1000000000);errno=m.raw_errno;return 0;
}
static int fake_fcntl(int fd,int op,int minimum)
{
    CHECK(fd==9 && op==F_DUPFD_CLOEXEC && minimum==3 && m.duplicates<3);
    errno=m.raw_errno;return 101+(int)m.duplicates++;
}
static int fake_close(int fd)
{ CHECK(fd>=101 && fd<=103 && m.closes<3);++m.closes;errno=m.raw_errno;return 0; }
static int fake_actions_init(posix_spawn_file_actions_t *p)
{ CHECK(p!=NULL);++m.actions;errno=m.raw_errno;return 0; }
static int fake_actions_dup(posix_spawn_file_actions_t *p,int fd,int target)
{ CHECK(p!=NULL && target>=0 && target<=2 && fd==101+target);++m.actions;errno=m.raw_errno;return 0; }
static int fake_actions_close(posix_spawn_file_actions_t *p,int fd)
{ CHECK(p!=NULL && fd>=101 && fd<=103);++m.actions;errno=m.raw_errno;return 0; }
static int fake_actions_cwd(posix_spawn_file_actions_t *p,const char *cwd)
{ CHECK(p!=NULL && strcmp(cwd,"/private/tmp/mock-normal-child-cwd")==0);++m.actions;errno=m.raw_errno;return 0; }
static int fake_actions_destroy(posix_spawn_file_actions_t *p)
{ CHECK(p!=NULL);++m.actions;errno=m.raw_errno;return 0; }
static int fake_attr_init(posix_spawnattr_t *p)
{ CHECK(p!=NULL);++m.attributes;errno=m.raw_errno;return 0; }
static int fake_attr_flags(posix_spawnattr_t *p,short flags)
{
    CHECK(p!=NULL && flags==(short)(POSIX_SPAWN_SETSID|POSIX_SPAWN_START_SUSPENDED|POSIX_SPAWN_CLOEXEC_DEFAULT));
    ++m.attributes;errno=m.raw_errno;return 0;
}
static int fake_attr_destroy(posix_spawnattr_t *p)
{ CHECK(p!=NULL);++m.attributes;errno=m.raw_errno;return 0; }
static int fake_spawn(pid_t *pid,const char *exe,const posix_spawn_file_actions_t *a,
                      const posix_spawnattr_t *attr,char *const argv[],char *const env[])
{
    CHECK(pid!=NULL && a!=NULL && attr!=NULL && m.spawns==0 && m.duplicates==3);
    CHECK(strcmp(exe,"/usr/bin/true")==0 && strcmp(argv[0],exe)==0 && argv[1]==NULL);
    CHECK(strcmp(env[0],"PATH=/usr/bin:/bin")==0 && strcmp(env[1],"LC_ALL=C")==0 && env[2]==NULL);
    ++m.spawns;*pid=41;errno=m.raw_errno;return 0;
}
static int fake_kill(pid_t pid,int signal_number)
{
    CHECK(pid==41 && signal_number==SIGCONT && m.spawns==1 && m.resumes==0);
    ++m.resumes;errno=m.resume_error?m.resume_error:m.raw_errno;
    if (m.resume_error) return -1;
    m.resumed=1;return 0;
}
static void mock_event(siginfo_t *out,int code,int status,int pid)
{
    memset(out,0,sizeof(*out));out->si_signo=SIGCHLD;out->si_code=code;
    out->si_pid=pid;out->si_uid=0;out->si_status=status;
}
static int fake_waitid(idtype_t type,id_t pid,siginfo_t *out,int options)
{
    CHECK(type==P_PID && pid==41 && out!=NULL && m.spawns==1);++m.waits;
    errno=m.raw_errno;
    if (options==(WEXITED|WSTOPPED|WNOWAIT|WNOHANG)) {
        CHECK(!m.resumed);mock_event(out,CLD_STOPPED,0,41);return 0;
    }
    CHECK(m.resumed && (options==(WEXITED|WNOWAIT|WNOHANG) || options==(WEXITED|WNOHANG)));
    if (!(options&WNOWAIT)) {
        CHECK(m.consuming==0);++m.consuming;
        if (m.consume_error) {errno=m.consume_error;memset(out,0,sizeof(*out));return -1;}
        mock_event(out,CLD_EXITED,m.consume_status,m.consume_pid);
        if (m.loss_on_next_reap) {m.loss_on_next_reap=0;m.dictionary_countdown=0;}
        return 0;
    }
    if (m.mode) memset(out,0,sizeof(*out));
    else mock_event(out,CLD_EXITED,0,41);
    if (m.loss_on_next_active) {m.loss_on_next_active=0;m.dictionary_countdown=0;}
    return 0;
}
static int fake_proc_pidinfo(int pid,int flavor,uint64_t arg,void *out,int bytes)
{
    CHECK(pid==41 && flavor==PROC_PIDTBSDINFO && arg==0 && out!=NULL && bytes==sizeof(struct proc_bsdinfo));
    ++m.bsds;struct proc_bsdinfo info;memset(&info,0,sizeof(info));
    info.pbi_pid=info.pbi_pgid=41;info.pbi_ppid=99;info.pbi_uid=501;
    info.pbi_start_tvsec=123;info.pbi_start_tvusec=45;
    info.pbi_status=!m.resumed?SSTOP:(m.mode==2?SZOMB:SRUN);
    memcpy(out,&info,sizeof(info));errno=m.raw_errno;return sizeof(info);
}
static int fake_proc_listpids(uint32_t type,uint32_t pgid,void *out,int bytes)
{
    CHECK(type==PROC_PGRP_ONLY && pgid==41 && out!=NULL && bytes==2*sizeof(pid_t));
    ++m.lists;pid_t values[2]={41,42};
    memcpy(out,values,m.list_count*sizeof(pid_t));errno=m.raw_errno;
    return m.list_count*(int)sizeof(pid_t);
}

/* Every process, signal, resource and clock name used by the producer is either
 * mocked above or trapped. Allocations are bounded private memory arenas. */
#define calloc(...) fake_calloc(__VA_ARGS__)
#define free(...) fake_free(__VA_ARGS__)
#define getpid() fake_getpid()
#define getuid() fake_getuid()
#define getsid(...) fake_getsid(__VA_ARGS__)
#define sigaction(...) fake_sigaction(__VA_ARGS__)
#define clock_gettime(...) fake_clock_gettime(__VA_ARGS__)
#define fcntl(...) fake_fcntl(__VA_ARGS__)
#define close(...) fake_close(__VA_ARGS__)
#define posix_spawn_file_actions_init(...) fake_actions_init(__VA_ARGS__)
#define posix_spawn_file_actions_adddup2(...) fake_actions_dup(__VA_ARGS__)
#define posix_spawn_file_actions_addclose(...) fake_actions_close(__VA_ARGS__)
#define posix_spawn_file_actions_addchdir_np(...) fake_actions_cwd(__VA_ARGS__)
#define posix_spawn_file_actions_destroy(...) fake_actions_destroy(__VA_ARGS__)
#define posix_spawnattr_init(...) fake_attr_init(__VA_ARGS__)
#define posix_spawnattr_setflags(...) fake_attr_flags(__VA_ARGS__)
#define posix_spawnattr_destroy(...) fake_attr_destroy(__VA_ARGS__)
#define posix_spawn(...) fake_spawn(__VA_ARGS__)
#define kill(...) fake_kill(__VA_ARGS__)
#define killpg(...) forbidden("killpg")
#define waitpid(...) forbidden("waitpid")
#define waitid(...) fake_waitid(__VA_ARGS__)
#define proc_pidinfo(...) fake_proc_pidinfo(__VA_ARGS__)
#define proc_listpids(...) fake_proc_listpids(__VA_ARGS__)
#include "window_owned_child_darwin.c"

static PyObject *mock_dictionary_new(void)
{
    if (m.dictionary_countdown==0) {m.dictionary_countdown=-1;++m.mapping_failures;return PyErr_NoMemory();}
    if (m.dictionary_countdown>0) --m.dictionary_countdown;
    return PyDict_New();
}
#define PyDict_New() mock_dictionary_new()
#define PyInit__window_owned_child_bridge original_normal_module_init
#include "window_owned_child_python.c"
#undef PyInit__window_owned_child_bridge
#undef PyDict_New

static PyObject *fixture_stats(PyObject *module,PyObject *unused)
{
    (void)module;(void)unused;
    return Py_BuildValue("{s:I,s:I,s:I,s:I,s:I,s:I,s:I,s:I,s:I,s:I,s:I,s:I,s:I,s:I,s:I,s:I,s:I,s:I,s:I}",
      "clocks",m.clocks,"pids",m.pids,"uids",m.uids,"policies",m.policies,
      "waits",m.waits,"consuming",m.consuming,"bsds",m.bsds,"sids",m.sids,"lists",m.lists,
      "duplicates",m.duplicates,"closes",m.closes,"actions",m.actions,"attributes",m.attributes,
      "spawns",m.spawns,"resumes",m.resumes,"frees",m.frees,"allocations",m.allocations,
      "raw_bytes",(unsigned)sizeof(siginfo_t),"mapping_failures",m.mapping_failures);
}
static PyObject *fixture_scenario(PyObject *module,PyObject *name)
{
    (void)module;
    if (!PyUnicode_CheckExact(name)) {PyErr_SetString(PyExc_TypeError,"exact scenario required");return NULL;}
    const char *s=PyUnicode_AsUTF8(name);if (!s) return NULL;
    if (strcmp(s,"live")==0) m.mode=1;
    else if (strcmp(s,"transition")==0) m.mode=2;
    else if (strcmp(s,"exit0")==0) m.mode=0;
    else if (strcmp(s,"positive_errno")==0) m.raw_errno=EIO;
    else if (strcmp(s,"consume_error")==0) m.consume_error=ECHILD;
    else if (strcmp(s,"consume_foreign_pid")==0) m.consume_pid=42;
    else if (strcmp(s,"consume_changed_status")==0) m.consume_status=7;
    else if (strcmp(s,"extra_member")==0) m.list_count=2;
    else if (strcmp(s,"expire")==0) m.now_ns+=20000000000ULL;
    else if (strcmp(s,"resume_error")==0 && !m.spawns) m.resume_error=ESRCH;
    else if (strcmp(s,"dictionary_loss")==0) m.dictionary_countdown=0;
    else if (strcmp(s,"loss_on_next_active")==0) m.loss_on_next_active=1;
    else if (strcmp(s,"loss_on_next_reap")==0) m.loss_on_next_reap=1;
    else {PyErr_SetString(PyExc_ValueError,"unsupported finite scenario");return NULL;}
    Py_RETURN_NONE;
}
static PyMethodDef fixture_methods[]={
    {"_fixture_stats",fixture_stats,METH_NOARGS,NULL},
    {"_fixture_scenario",fixture_scenario,METH_O,NULL},
    {NULL,NULL,0,NULL}
};
PyMODINIT_FUNC PyInit__window_owned_child_bridge(void)
{
    PyObject *module=original_normal_module_init();
    if (module!=NULL && PyModule_AddFunctions(module,fixture_methods)<0) {Py_DECREF(module);return NULL;}
    return module;
}
