#ifndef SHARED_MEMORY_WIN_H
#define SHARED_MEMORY_WIN_H

#include <windows.h>
#include <stdio.h>

static inline void generate_shared_memory_name(char *buffer, size_t size) {
    snprintf(buffer, size, "Local\\rrprof_shm_%lu_%lu", GetCurrentProcessId(), GetTickCount());
}

static inline void* open_shared_memory(const char *name, int size) {
    HANDLE hMapFile = CreateFileMappingA(
        INVALID_HANDLE_VALUE,
        NULL,
        PAGE_READWRITE,
        0,
        size,
        name);

    if (hMapFile == NULL) {
        return NULL;
    }

    void *ptr = MapViewOfFile(
        hMapFile,
        FILE_MAP_ALL_ACCESS,
        0,
        0,
        size);

    return ptr;
}

#endif /* SHARED_MEMORY_WIN_H */
