#ifndef RRTRACE_TIME_POSIX_H
#define RRTRACE_TIME_POSIX_H

#include <stdint.h>
#include <time.h>

static struct timespec rrtrace_base_timestamp = {0};

static inline void init_base_timestamp(void) {
    clock_gettime(CLOCK_MONOTONIC, &rrtrace_base_timestamp);
}

static inline uint64_t now(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);

    uint64_t seconds = (uint64_t)(ts.tv_sec - rrtrace_base_timestamp.tv_sec);
    uint64_t nanoseconds;
    if (ts.tv_nsec >= rrtrace_base_timestamp.tv_nsec) {
        nanoseconds = (uint64_t)(ts.tv_nsec - rrtrace_base_timestamp.tv_nsec);
    } else {
        seconds -= 1;
        nanoseconds = (uint64_t)ts.tv_nsec + 1000000000ull - (uint64_t)rrtrace_base_timestamp.tv_nsec;
    }

    return seconds * 1000000000ull + nanoseconds;
}

#endif /* RRTRACE_TIME_POSIX_H */
