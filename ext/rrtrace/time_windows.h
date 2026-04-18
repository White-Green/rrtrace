#ifndef RRTRACE_TIME_WINDOWS_H
#define RRTRACE_TIME_WINDOWS_H

#include <stdint.h>
#include <windows.h>

static LARGE_INTEGER rrtrace_base_counter = {0};
static LARGE_INTEGER rrtrace_counter_frequency = {0};

static inline uint64_t rrtrace_counter_to_ns(uint64_t counter) {
    uint64_t frequency = (uint64_t)rrtrace_counter_frequency.QuadPart;
    uint64_t seconds = counter / frequency;
    uint64_t remainder = counter % frequency;
    return seconds * 1000000000ull + (remainder * 1000000000ull) / frequency;
}

static inline void init_base_timestamp(void) {
    if (!QueryPerformanceFrequency(&rrtrace_counter_frequency) || rrtrace_counter_frequency.QuadPart <= 0) {
        rrtrace_counter_frequency.QuadPart = 1;
    }
    if (!QueryPerformanceCounter(&rrtrace_base_counter)) {
        rrtrace_base_counter.QuadPart = 0;
    }
}

static inline uint64_t now(void) {
    LARGE_INTEGER counter;
    if (!QueryPerformanceCounter(&counter)) return 0;

    return rrtrace_counter_to_ns((uint64_t)(counter.QuadPart - rrtrace_base_counter.QuadPart));
}

#endif /* RRTRACE_TIME_WINDOWS_H */
