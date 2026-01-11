#ifndef SHARED_MEMORY_POSIX_H
#define SHARED_MEMORY_POSIX_H

#include <sys/mman.h>
#include <sys/stat.h>
#include <fcntl.h>
#include <unistd.h>
#include <time.h>
#include <stdio.h>

static inline void generate_shared_memory_name(char *buffer, size_t size) {
    struct timespec ts;
    timespec_get(&ts, TIME_UTC);
    snprintf(buffer, size, "/rrtrace_shm_%d_%d", getpid(), (int)ts.tv_nsec);
}

static inline void* open_shared_memory(const char *name, int size) {
    int fd = shm_open(name, O_CREAT | O_RDWR, 0666);
    if (fd == -1) return NULL;
    if (ftruncate(fd, size) == -1) return NULL;
    void *ptr = mmap(0, size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (ptr == MAP_FAILED) return NULL;
    return ptr;
}

#endif /* SHARED_MEMORY_POSIX_H */
