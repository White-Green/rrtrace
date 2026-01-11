#ifndef PROCESS_MANAGER_WIN_H
#define PROCESS_MANAGER_WIN_H

#include <windows.h>
#include <string.h>

typedef HANDLE process_id;

static inline process_id spawn_process(const char *cmd, char * const* argv) {
    STARTUPINFOA si;
    PROCESS_INFORMATION pi;
    ZeroMemory(&si, sizeof(si));
    si.cb = sizeof(si);
    ZeroMemory(&pi, sizeof(pi));

    char cmdline[4096] = {0};
    for (int i = 0; argv[i] != NULL; i++) {
        if (i > 0) strncat(cmdline, " ", sizeof(cmdline) - strlen(cmdline) - 1);
        strncat(cmdline, "\"", sizeof(cmdline) - strlen(cmdline) - 1);
        strncat(cmdline, argv[i], sizeof(cmdline) - strlen(cmdline) - 1);
        strncat(cmdline, "\"", sizeof(cmdline) - strlen(cmdline) - 1);
    }

    if (!CreateProcessA(NULL, cmdline, NULL, NULL, FALSE, 0, NULL, NULL, &si, &pi)) {
        return 0;
    }
    CloseHandle(pi.hThread);
    return pi.hProcess;
}

static inline int is_process_running(process_id pid) {
    if (pid == NULL) return 0;
    DWORD exitCode;
    if (GetExitCodeProcess(pid, &exitCode)) {
        return exitCode == STILL_ACTIVE;
    }
    return 0;
}

#endif /* PROCESS_MANAGER_WIN_H */
