#ifndef PROCESS_MANAGER_POSIX_H
#define PROCESS_MANAGER_POSIX_H

#include <sys/types.h>
#include <sys/wait.h>
#include <spawn.h>
#include <signal.h>

typedef pid_t process_id;

static inline process_id invalid_process_id(void) {
    return 0;
}

extern char **environ;

static inline process_id spawn_process(const char *cmd, char * const* restrict argv) {
    pid_t pid;
    int status = posix_spawn(&pid, cmd, NULL, NULL, argv, environ);
    if (status == 0) return pid;
    else return 0;
}

static inline int is_process_running(process_id pid) {
    if (pid == 0) return 0;
    int status;
    pid_t result = waitpid(pid, &status, WNOHANG);
    return result == 0;
}

static inline void terminate_process(process_id pid) {
    if (pid == 0) return;
    kill(pid, SIGTERM);
    waitpid(pid, NULL, 0);
}

static inline void close_process(process_id pid) {
    (void)pid;
}

#endif /* PROCESS_MANAGER_POSIX_H */
