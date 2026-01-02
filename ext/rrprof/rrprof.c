#include "rrprof.h"
#include "rrprof_event_ringbuffer.h"

#include "process_manager_posix.h"
#include "shared_memory_posix.h"

// #define RRPROF_WRITE_DEBUG_LOG

#ifdef RRPROF_WRITE_DEBUG_LOG
#include<stdio.h>
#endif

typedef struct {
  uint32_t thread_id;
} ThreadData;

typedef struct {
  RRProfEventRingBuffer *event_ringbuffer;
  process_id visualizer_process_id;
  rb_internal_thread_specific_key_t thread_data_key;
  atomic_uint_fast32_t next_thread_id;
#ifdef RRPROF_WRITE_DEBUG_LOG
  FILE *log;
#endif
} TraceContext;

static inline void push_event(TraceContext *context, RRProfTraceEvent event) {
  if (context->event_ringbuffer == NULL) return;
  while (!rrprof_event_ringbuffer_push(context->event_ringbuffer, event)) {
    if (!is_process_running(context->visualizer_process_id)) {
      context->event_ringbuffer = NULL;
      break;
    }
  }
}

static uint32_t get_thread_id(TraceContext *context, VALUE thread) {
  ThreadData *data = rb_internal_thread_specific_get(thread, context->thread_data_key);
  if (data == NULL) {
    data = malloc(sizeof(ThreadData));
    data->thread_id = atomic_fetch_add_explicit(&context->next_thread_id, 1, memory_order_relaxed);
    rb_internal_thread_specific_set(thread, context->thread_data_key, data);
  }
  return data->thread_id;
}

static void tracepoint_call_handler(VALUE tpval, void *data) {
  TraceContext *context = (TraceContext *)data;
  struct rb_trace_arg_struct *tracearg = rb_tracearg_from_tracepoint(tpval);
  uint64_t method_id = RB_SYM2ID(rb_tracearg_method_id(tracearg));
  push_event(context, event_call(method_id));
#ifdef RRPROF_WRITE_DEBUG_LOG
  const char *method_name = rb_id2name(method_id);
  fprintf(context->log, "CALL: %s\n", method_name);
  fflush(context->log);
#endif
}

static void tracepoint_return_handler(VALUE tpval, void *data) {
  TraceContext *context = (TraceContext *)data;
  struct rb_trace_arg_struct *tracearg = rb_tracearg_from_tracepoint(tpval);
  uint64_t method_id = RB_SYM2ID(rb_tracearg_method_id(tracearg));
  push_event(context, event_return(method_id));
#ifdef RRPROF_WRITE_DEBUG_LOG
  const char *method_name = rb_id2name(method_id);
  fprintf(context->log, "RETURN: %s\n", method_name);
  fflush(context->log);
#endif
}

static void tracepoint_gc_start_handler(VALUE tpval, void *data) {
  TraceContext *context = (TraceContext *)data;
  push_event(context, event_gc_start());
#ifdef RRPROF_WRITE_DEBUG_LOG
  fprintf(context->log, "GC START\n");
  fflush(context->log);
#endif
}

static void tracepoint_gc_end_handler(VALUE tpval, void *data) {
  TraceContext *context = (TraceContext *)data;
  push_event(context, event_gc_end());
#ifdef RRPROF_WRITE_DEBUG_LOG
  fprintf(context->log, "GC END\n");
  fflush(context->log);
#endif
}

static void thread_start_handler(rb_event_flag_t event, const rb_internal_thread_event_data_t *event_data, void *data) {
  TraceContext *context = (TraceContext *)data;
  push_event(context, event_thread_start(get_thread_id(context, event_data->thread)));
#ifdef RRPROF_WRITE_DEBUG_LOG
  fprintf(context->log, "THREAD START\n");
  fflush(context->log);
#endif
}

static void thread_ready_handler(rb_event_flag_t event, const rb_internal_thread_event_data_t *event_data, void *data) {
  TraceContext *context = (TraceContext *)data;
  push_event(context, event_thread_ready(get_thread_id(context, event_data->thread)));
#ifdef RRPROF_WRITE_DEBUG_LOG
  fprintf(context->log, "THREAD READY\n");
  fflush(context->log);
#endif
}

static void thread_suspended_handler(rb_event_flag_t event, const rb_internal_thread_event_data_t *event_data, void *data) {
  TraceContext *context = (TraceContext *)data;
  push_event(context, event_thread_suspended(get_thread_id(context, event_data->thread)));
#ifdef RRPROF_WRITE_DEBUG_LOG
  fprintf(context->log, "THREAD SUSPENDED\n");
  fflush(context->log);
#endif
}

static void thread_resume_handler(rb_event_flag_t event, const rb_internal_thread_event_data_t *event_data, void *data) {
  TraceContext *context = (TraceContext *)data;
  push_event(context, event_thread_resume(get_thread_id(context, event_data->thread)));
#ifdef RRPROF_WRITE_DEBUG_LOG
  fprintf(context->log, "THREAD RESUME\n");
  fflush(context->log);
#endif
}

static void thread_exit_handler(rb_event_flag_t event, const rb_internal_thread_event_data_t *event_data, void *data) {
  TraceContext *context = (TraceContext *)data;
  push_event(context, event_thread_exit(get_thread_id(context, event_data->thread)));
#ifdef RRPROF_WRITE_DEBUG_LOG
  fprintf(context->log, "THREAD EXIT\n");
  fflush(context->log);
#endif
}

RUBY_FUNC_EXPORTED void
Init_rrprof(void)
{
  TraceContext *context = malloc(sizeof(TraceContext));
  context->thread_data_key = rb_internal_thread_specific_key_create();
  atomic_init(&context->next_thread_id, 1);
#ifdef RRPROF_WRITE_DEBUG_LOG
  context->log = fopen("rrprof.log", "w");
#endif

  char shm_name[64];
  generate_shared_memory_name(shm_name, sizeof(shm_name));
  RRProfEventRingBuffer *ringbuffer = open_shared_memory(shm_name, sizeof(RRProfEventRingBuffer));
  if (ringbuffer == NULL) {
    rb_raise(rb_eRuntimeError, "Failed to create shared memory for rrprof");
    return;
  }
  rrprof_event_ringbuffer_init(ringbuffer);
  context->event_ringbuffer = ringbuffer;

  VALUE mMyGem = rb_const_get(rb_cObject, rb_intern("Rrprof"));
  VALUE visualizer = rb_funcall(mMyGem, rb_intern("visualizer_path"), 0);
  char *visualizer_path = rb_string_value_cstr(&visualizer);
#ifdef RRPROF_WRITE_DEBUG_LOG
  fprintf(context->log, "Visualizer: %s\n", visualizer_path);
  fprintf(context->log, "Shared Memory: %s\n", shm_name);
#endif
  process_id pid = spawn_process(visualizer_path, (char * const[]){visualizer_path, shm_name, NULL});
  if (pid == 0) {
    rb_raise(rb_eRuntimeError, "Failed to spawn visualizer process");
    return;
  }
  context->visualizer_process_id = pid;

  ThreadData *main_thread_data = malloc(sizeof(ThreadData));
  main_thread_data->thread_id = 0;
  VALUE thread = rb_thread_current();
  rb_internal_thread_specific_set(thread, context->thread_data_key, main_thread_data);

  VALUE trace_call = rb_tracepoint_new(RUBY_Qnil, RUBY_EVENT_CALL | RUBY_EVENT_C_CALL, tracepoint_call_handler, context);
  VALUE trace_return = rb_tracepoint_new(RUBY_Qnil, RUBY_EVENT_RETURN | RUBY_EVENT_C_RETURN, tracepoint_return_handler, context);
  VALUE trace_gc_start = rb_tracepoint_new(RUBY_Qnil, RUBY_INTERNAL_EVENT_GC_ENTER, tracepoint_gc_start_handler, context);
  VALUE trace_gc_end = rb_tracepoint_new(RUBY_Qnil, RUBY_INTERNAL_EVENT_GC_EXIT, tracepoint_gc_end_handler, context);
  rb_internal_thread_add_event_hook(thread_start_handler, RUBY_INTERNAL_THREAD_EVENT_STARTED, context);
  rb_internal_thread_add_event_hook(thread_ready_handler, RUBY_INTERNAL_THREAD_EVENT_READY, context);
  rb_internal_thread_add_event_hook(thread_suspended_handler, RUBY_INTERNAL_THREAD_EVENT_SUSPENDED, context);
  rb_internal_thread_add_event_hook(thread_resume_handler, RUBY_INTERNAL_THREAD_EVENT_RESUMED, context);
  rb_internal_thread_add_event_hook(thread_exit_handler, RUBY_INTERNAL_THREAD_EVENT_EXITED, context);

  rb_tracepoint_enable(trace_call);
  rb_tracepoint_enable(trace_return);
  rb_tracepoint_enable(trace_gc_start);
  rb_tracepoint_enable(trace_gc_end);
}
