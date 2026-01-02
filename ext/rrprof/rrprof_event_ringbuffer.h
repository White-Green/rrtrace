#ifndef RRPROF_EVENT_RINGBUFFER_H
#define RRPROF_EVENT_RINGBUFFER_H

#include "rrprof_event.h"

#define SIZE 65536
#define MASK (SIZE - 1)

typedef struct {
    RRProfTraceEvent buffer[SIZE];
    alignas(64) struct {
        atomic_ulong write_index;
        ulong read_index_cache;
    } writer;
    alignas(64) struct {
        atomic_ulong read_index;
        ulong write_index_cache;
    } reader;
} RRProfEventRingBuffer;

static inline void rrprof_event_ringbuffer_init(RRProfEventRingBuffer *rb) {
    atomic_store_explicit(&rb->writer.write_index, 0, memory_order_relaxed);
    rb->writer.read_index_cache = 0;
    atomic_store_explicit(&rb->reader.read_index, 0, memory_order_relaxed);
    rb->reader.write_index_cache = 0;
}

static inline int rrprof_event_ringbuffer_push(RRProfEventRingBuffer *rb, RRProfTraceEvent event) {
    if (rb == NULL) return 1;
    ulong write_index = atomic_load_explicit(&rb->writer.write_index, memory_order_relaxed);
    ulong read_index_cache = rb->writer.read_index_cache;
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

#endif /* RRPROF_EVENT_RINGBUFFER_H */
