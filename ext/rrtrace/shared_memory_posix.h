#ifndef SHARED_MEMORY_POSIX_H
#define SHARED_MEMORY_POSIX_H

#include <sys/mman.h>
#include <sys/stat.h>
#include <fcntl.h>
#include <unistd.h>
#include <time.h>
#include <stdio.h>
#include <string.h>

typedef struct {
    int fd;
    void *ptr;
    size_t size;
    char name[64];
} shared_memory_handle;

static inline void generate_shared_memory_name(char *buffer, size_t size) {
    struct timespec ts;
    timespec_get(&ts, TIME_UTC);
    snprintf(buffer, size, "/rrtrace_shm_%d_%d", getpid(), (int)ts.tv_nsec);
}

static inline shared_memory_handle invalid_shared_memory_handle(void) {
    shared_memory_handle shm;
    shm.fd = -1;
    shm.ptr = NULL;
    shm.size = 0;
    shm.name[0] = '\0';
    return shm;
}

static inline shared_memory_handle open_shared_memory(const char *name, int size) {
    int fd = shm_open(name, O_CREAT | O_RDWR, 0666);
    if (fd == -1) return invalid_shared_memory_handle();
    if (ftruncate(fd, size) == -1) {
        close(fd);
        return invalid_shared_memory_handle();
    }
    void *ptr = mmap(0, size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (ptr == MAP_FAILED) {
        close(fd);
        return invalid_shared_memory_handle();
    }

    shared_memory_handle shm;
    shm.fd = fd;
    shm.ptr = ptr;
    shm.size = size;
    snprintf(shm.name, sizeof(shm.name), "%s", name);
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
        munmap(shm->ptr, shm->size);
        shm->ptr = NULL;
    }
    if (shm->fd != -1) {
        close(shm->fd);
        shm->fd = -1;
    }
    if (shm->name[0] != '\0') {
        shm_unlink(shm->name);
        shm->name[0] = '\0';
    }
}

#endif /* SHARED_MEMORY_POSIX_H */
