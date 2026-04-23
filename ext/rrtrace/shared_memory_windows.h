#ifndef SHARED_MEMORY_WIN_H
#define SHARED_MEMORY_WIN_H

#include <windows.h>
#include <stdio.h>

typedef struct {
    HANDLE handle;
    void *ptr;
} shared_memory_handle;

static inline void generate_shared_memory_name(char *buffer, size_t size) {
    snprintf(buffer, size, "Local\\rrtrace_shm_%lu_%lu", GetCurrentProcessId(), GetTickCount());
}

static inline shared_memory_handle invalid_shared_memory_handle(void) {
    shared_memory_handle shm = {0};
    return shm;
}

static inline shared_memory_handle open_shared_memory(const char *name, int size) {
    HANDLE hMapFile = CreateFileMappingA(
        INVALID_HANDLE_VALUE,
        NULL,
        PAGE_READWRITE,
        0,
        size,
        name);

    if (hMapFile == NULL) {
        return invalid_shared_memory_handle();
    }

    void *ptr = MapViewOfFile(
        hMapFile,
        FILE_MAP_ALL_ACCESS,
        0,
        0,
        size);

    if (ptr == NULL) {
        CloseHandle(hMapFile);
        return invalid_shared_memory_handle();
    }

    shared_memory_handle shm = {hMapFile, ptr};
    return shm;
}

static inline int shared_memory_opened(shared_memory_handle shm) {
    return shm.ptr != NULL;
}

static inline void *shared_memory_ptr(shared_memory_handle *shm) {
    return shm->ptr;
}

static inline void close_shared_memory(shared_memory_handle *shm) {
    if (shm->ptr != NULL) {
        UnmapViewOfFile(shm->ptr);
        shm->ptr = NULL;
    }
    if (shm->handle != NULL) {
        CloseHandle(shm->handle);
        shm->handle = NULL;
    }
}

#endif /* SHARED_MEMORY_WIN_H */
