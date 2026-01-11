#ifndef PROCESS_MANAGER_POSIX_H
#define PROCESS_MANAGER_POSIX_H

#include <sys/types.h>
#include <sys/wait.h>
#include <spawn.h>

typedef pid_t process_id;

extern char **environ;

static inline process_id spawn_process(const char *cmd, char * const* restrict argv) {
    pid_t pid;
    int status = posix_spawn(&pid, cmd, NULL, NULL, argv, environ);
    if (status == 0) return pid;
    else return 0;
}

static inline int is_process_running(process_id pid) {
    int status;
    waitpid(pid, &status, WNOHANG);
    return !WIFEXITED(status);
}

#endif /* PROCESS_MANAGER_POSIX_H */
