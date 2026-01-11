#ifndef RRTRACE_EVENT_RINGBUFFER_H
#define RRTRACE_EVENT_RINGBUFFER_H

#include "rrtrace_event.h"

#define SIZE 65536
#define MASK (SIZE - 1)

typedef struct {
    RRTraceEvent buffer[SIZE];
    alignas(64) struct {
        atomic_uint_fast64_t write_index;
        uint64_t read_index_cache;
    } writer;
    alignas(64) struct {
        atomic_uint_fast64_t read_index;
        uint64_t write_index_cache;
    } reader;
} RRTraceEventRingBuffer;

static inline void rrtrace_event_ringbuffer_init(RRTraceEventRingBuffer *rb) {
    atomic_store_explicit(&rb->writer.write_index, 0, memory_order_relaxed);
    rb->writer.read_index_cache = 0;
    atomic_store_explicit(&rb->reader.read_index, 0, memory_order_relaxed);
    rb->reader.write_index_cache = 0;
}

static inline int rrtrace_event_ringbuffer_push(RRTraceEventRingBuffer *rb, RRTraceEvent event) {
    if (rb == NULL) return 1;
    uint64_t write_index = atomic_load_explicit(&rb->writer.write_index, memory_order_relaxed);
    uint64_t read_index_cache = rb->writer.read_index_cache;
    if (write_index - read_index_cache >= SIZE) {
        read_index_cache = atomic_load_explicit(&rb->reader.read_index, memory_order_acquire);
        rb->writer.read_index_cache = read_index_cache;
        if (write_index - read_index_cache >= SIZE) return 0;
    }
    rb->buffer[write_index & MASK] = event;
    atomic_store_explicit(&rb->writer.write_index, write_index + 1, memory_order_release);
    return 1;
}

#undef MASK
#undef SIZE

#endif /* RRTRACE_EVENT_RINGBUFFER_H */
