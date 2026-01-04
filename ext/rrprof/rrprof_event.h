#ifndef RRPROF_EVENT_H
#define RRPROF_EVENT_H

#include <time.h>
#include <stdatomic.h>
#include <stdint.h>

#define EVENT_TYPE_CALL             0x0000000000000000ull
#define EVENT_TYPE_RETURN           0x1000000000000000ull
#define EVENT_TYPE_GC_START         0x2000000000000000ull
#define EVENT_TYPE_GC_END           0x3000000000000000ull
#define EVENT_TYPE_THREAD_START     0x4000000000000000ull
#define EVENT_TYPE_THREAD_READY     0x5000000000000000ull
#define EVENT_TYPE_THREAD_SUSPENDED 0x6000000000000000ull
#define EVENT_TYPE_THREAD_RESUME    0x7000000000000000ull
#define EVENT_TYPE_THREAD_EXIT      0x8000000000000000ull

#define EVENT_TYPE_MASK             0xF000000000000000ull

typedef struct {
    uint64_t timestamp_and_event_type;
    uint64_t data;
} RRProfTraceEvent;

static inline uint64_t now(void) {
    struct timespec ts;
    timespec_get(&ts, TIME_UTC);
    uint64_t timestamp = (uint64_t)ts.tv_sec * 1000000000ull + (uint64_t)ts.tv_nsec;

    static atomic_uint_fast64_t base_timestamp = 0;
    uint64_t expected = 0;
    if (atomic_compare_exchange_strong_explicit(&base_timestamp, &expected, timestamp, memory_order_relaxed, memory_order_relaxed)) {
        return 0;
    } else {
        return timestamp - expected;
    }
}

static inline RRProfTraceEvent event_call(uint64_t method_id) {
    RRProfTraceEvent event;
    event.timestamp_and_event_type = now() | EVENT_TYPE_CALL;
    event.data = method_id;
    return event;
}

static inline RRProfTraceEvent event_return(uint64_t method_id) {
    RRProfTraceEvent event;
    event.timestamp_and_event_type = now() | EVENT_TYPE_RETURN;
    event.data = method_id;
    return event;
}

static inline RRProfTraceEvent event_gc_start(void) {
    RRProfTraceEvent event;
    event.timestamp_and_event_type = now() | EVENT_TYPE_GC_START;
    return event;
}

static inline RRProfTraceEvent event_gc_end(void) {
    RRProfTraceEvent event;
    event.timestamp_and_event_type = now() | EVENT_TYPE_GC_END;
    return event;
}

static inline RRProfTraceEvent event_thread_start(uint32_t thread_id) {
    RRProfTraceEvent event;
    event.timestamp_and_event_type = now() | EVENT_TYPE_THREAD_START;
    event.data = thread_id;
    return event;
}

static inline RRProfTraceEvent event_thread_ready(uint32_t thread_id) {
    RRProfTraceEvent event;
    event.timestamp_and_event_type = now() | EVENT_TYPE_THREAD_READY;
    event.data = thread_id;
    return event;
}

static inline RRProfTraceEvent event_thread_suspended(uint32_t thread_id) {
    RRProfTraceEvent event;
    event.timestamp_and_event_type = now() | EVENT_TYPE_THREAD_SUSPENDED;
    event.data = thread_id;
    return event;
}

static inline RRProfTraceEvent event_thread_resume(uint32_t thread_id) {
    RRProfTraceEvent event;
    event.timestamp_and_event_type = now() | EVENT_TYPE_THREAD_RESUME;
    event.data = thread_id;
    return event;
}

static inline RRProfTraceEvent event_thread_exit(uint32_t thread_id) {
    RRProfTraceEvent event;
    event.timestamp_and_event_type = now() | EVENT_TYPE_THREAD_EXIT;
    event.data = thread_id;
    return event;
}

#undef EVENT_TYPE_CALL
#undef EVENT_TYPE_RETURN
#undef EVENT_TYPE_GC_START
#undef EVENT_TYPE_GC_END
#undef EVENT_TYPE_THREAD_START
#undef EVENT_TYPE_THREAD_READY
#undef EVENT_TYPE_THREAD_SUSPENDED
#undef EVENT_TYPE_THREAD_RESUME
#undef EVENT_TYPE_THREAD_EXIT
#undef EVENT_TYPE_MASK

#endif /* RRPROF_EVENT_H */
