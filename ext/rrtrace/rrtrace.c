#include "rrtrace.h"
#include "rrtrace_event_ringbuffer.h"

#if defined(_WIN32) || defined(__MINGW32__) || defined(__MINGW64__)
#include "process_manager_windows.h"
#include "shared_memory_windows.h"
#else
#include "process_manager_posix.h"
#include "shared_memory_posix.h"
#endif

// #define RRTRACE_WRITE_DEBUG_LOG

#ifdef RRTRACE_WRITE_DEBUG_LOG
#include<stdio.h>
#endif

typedef struct {
  uint32_t thread_id;
} ThreadData;

typedef struct {
  shared_memory_handle shared_memory;
  RRTraceEventRingBuffer *event_ringbuffer;
  process_id visualizer_process_id;
  rb_internal_thread_event_hook_t *thread_start_hook;
  rb_internal_thread_event_hook_t *thread_ready_hook;
  rb_internal_thread_event_hook_t *thread_suspended_hook;
  rb_internal_thread_event_hook_t *thread_resume_hook;
  rb_internal_thread_event_hook_t *thread_exit_hook;
  VALUE trace_call;
  VALUE trace_return;
  VALUE trace_gc_start;
  VALUE trace_gc_end;
  rb_internal_thread_specific_key_t thread_data_key;
  atomic_uint_fast32_t next_thread_id;
  atomic_flag event_ringbuffer_lock;
  int started;
#ifdef RRTRACE_WRITE_DEBUG_LOG
  FILE *log;
#endif
} TraceContext;

static inline void push_event(TraceContext *context, RRTraceEvent event) {
  if (context->event_ringbuffer == NULL) return;
  while (atomic_flag_test_and_set_explicit(&context->event_ringbuffer_lock, memory_order_acquire)) {
  }
  while (!rrtrace_event_ringbuffer_push(context->event_ringbuffer, event)) {
    if (!is_process_running(context->visualizer_process_id)) {
      context->event_ringbuffer = NULL;
      break;
    }
  }
  atomic_flag_clear_explicit(&context->event_ringbuffer_lock, memory_order_release);
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
#ifdef RRTRACE_WRITE_DEBUG_LOG
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
#ifdef RRTRACE_WRITE_DEBUG_LOG
  const char *method_name = rb_id2name(method_id);
  fprintf(context->log, "RETURN: %s\n", method_name);
  fflush(context->log);
#endif
}

static void tracepoint_gc_start_handler(VALUE tpval, void *data) {
  TraceContext *context = (TraceContext *)data;
  push_event(context, event_gc_start());
#ifdef RRTRACE_WRITE_DEBUG_LOG
  fprintf(context->log, "GC START\n");
  fflush(context->log);
#endif
}

static void tracepoint_gc_end_handler(VALUE tpval, void *data) {
  TraceContext *context = (TraceContext *)data;
  push_event(context, event_gc_end());
#ifdef RRTRACE_WRITE_DEBUG_LOG
  fprintf(context->log, "GC END\n");
  fflush(context->log);
#endif
}

static void thread_start_handler(rb_event_flag_t event, const rb_internal_thread_event_data_t *event_data, void *data) {
  TraceContext *context = (TraceContext *)data;
  push_event(context, event_thread_start(get_thread_id(context, event_data->thread)));
#ifdef RRTRACE_WRITE_DEBUG_LOG
  fprintf(context->log, "THREAD START\n");
  fflush(context->log);
#endif
}

static void thread_ready_handler(rb_event_flag_t event, const rb_internal_thread_event_data_t *event_data, void *data) {
  TraceContext *context = (TraceContext *)data;
  push_event(context, event_thread_ready(get_thread_id(context, event_data->thread)));
#ifdef RRTRACE_WRITE_DEBUG_LOG
  fprintf(context->log, "THREAD READY\n");
  fflush(context->log);
#endif
}

static void thread_suspended_handler(rb_event_flag_t event, const rb_internal_thread_event_data_t *event_data, void *data) {
  TraceContext *context = (TraceContext *)data;
  push_event(context, event_thread_suspended(get_thread_id(context, event_data->thread)));
#ifdef RRTRACE_WRITE_DEBUG_LOG
  fprintf(context->log, "THREAD SUSPENDED\n");
  fflush(context->log);
#endif
}

static void thread_resume_handler(rb_event_flag_t event, const rb_internal_thread_event_data_t *event_data, void *data) {
  TraceContext *context = (TraceContext *)data;
  push_event(context, event_thread_resume(get_thread_id(context, event_data->thread)));
#ifdef RRTRACE_WRITE_DEBUG_LOG
  fprintf(context->log, "THREAD RESUME\n");
  fflush(context->log);
#endif
}

static void thread_exit_handler(rb_event_flag_t event, const rb_internal_thread_event_data_t *event_data, void *data) {
  TraceContext *context = (TraceContext *)data;
  push_event(context, event_thread_exit(get_thread_id(context, event_data->thread)));
#ifdef RRTRACE_WRITE_DEBUG_LOG
  fprintf(context->log, "THREAD EXIT\n");
  fflush(context->log);
#endif
}

static TraceContext trace_context;

static void unregister_tracepoint(VALUE *tracepoint) {
  if (NIL_P(*tracepoint)) return;

  rb_tracepoint_disable(*tracepoint);
  rb_gc_unregister_address(tracepoint);
  *tracepoint = Qnil;
}

static void remove_thread_hook(rb_internal_thread_event_hook_t **hook) {
  if (*hook == NULL) return;

  rb_internal_thread_remove_event_hook(*hook);
  *hook = NULL;
}

static void cleanup_context(TraceContext *context) {
  unregister_tracepoint(&context->trace_call);
  unregister_tracepoint(&context->trace_return);
  unregister_tracepoint(&context->trace_gc_start);
  unregister_tracepoint(&context->trace_gc_end);

  remove_thread_hook(&context->thread_start_hook);
  remove_thread_hook(&context->thread_ready_hook);
  remove_thread_hook(&context->thread_suspended_hook);
  remove_thread_hook(&context->thread_resume_hook);
  remove_thread_hook(&context->thread_exit_hook);

  context->event_ringbuffer = NULL;
  close_shared_memory(&context->shared_memory);

  if (context->visualizer_process_id != invalid_process_id()) {
    terminate_process(context->visualizer_process_id);
    close_process(context->visualizer_process_id);
    context->visualizer_process_id = invalid_process_id();
  }

  context->started = 0;
}

static VALUE rrtrace_native_started_p(VALUE self) {
  return trace_context.started ? Qtrue : Qfalse;
}

static VALUE rrtrace_native_stop(VALUE self) {
  if (!trace_context.started) return Qfalse;

  cleanup_context(&trace_context);
  return Qtrue;
}

static VALUE rrtrace_native_start(VALUE self, VALUE visualizer) {
  TraceContext *context = &trace_context;
  VALUE visualizer_path = rb_str_dup(StringValue(visualizer));
  char *visualizer_path_cstr = StringValueCStr(visualizer_path);

  if (context->started) return Qfalse;

  atomic_store_explicit(&context->next_thread_id, 1, memory_order_relaxed);

  char shm_name[64];
  generate_shared_memory_name(shm_name, sizeof(shm_name));
  context->shared_memory = open_shared_memory(shm_name, sizeof(RRTraceEventRingBuffer));
  if (!shared_memory_opened(context->shared_memory)) {
    rb_raise(rb_eRuntimeError, "Failed to create shared memory for rrtrace");
    return Qfalse;
  }

  RRTraceEventRingBuffer *ringbuffer = shared_memory_ptr(&context->shared_memory);
  rrtrace_event_ringbuffer_init(ringbuffer);
  context->event_ringbuffer = ringbuffer;

#ifdef RRTRACE_WRITE_DEBUG_LOG
  fprintf(context->log, "Visualizer: %s\n", visualizer_path_cstr);
  fprintf(context->log, "Shared Memory: %s\n", shm_name);
#endif
  init_base_timestamp();
  process_id pid = spawn_process(visualizer_path_cstr, (char * const[]){visualizer_path_cstr, shm_name, NULL});
  if (pid == invalid_process_id()) {
    cleanup_context(context);
    rb_raise(rb_eRuntimeError, "Failed to spawn visualizer process");
    return Qfalse;
  }
  context->visualizer_process_id = pid;

  ThreadData *main_thread_data = malloc(sizeof(ThreadData));
  main_thread_data->thread_id = 0;
  VALUE thread = rb_thread_current();
  rb_internal_thread_specific_set(thread, context->thread_data_key, main_thread_data);

  context->trace_call = rb_tracepoint_new(RUBY_Qnil, RUBY_EVENT_CALL | RUBY_EVENT_C_CALL, tracepoint_call_handler, context);
  rb_gc_register_address(&context->trace_call);
  context->trace_return = rb_tracepoint_new(RUBY_Qnil, RUBY_EVENT_RETURN | RUBY_EVENT_C_RETURN, tracepoint_return_handler, context);
  rb_gc_register_address(&context->trace_return);
  context->trace_gc_start = rb_tracepoint_new(RUBY_Qnil, RUBY_INTERNAL_EVENT_GC_ENTER, tracepoint_gc_start_handler, context);
  rb_gc_register_address(&context->trace_gc_start);
  context->trace_gc_end = rb_tracepoint_new(RUBY_Qnil, RUBY_INTERNAL_EVENT_GC_EXIT, tracepoint_gc_end_handler, context);
  rb_gc_register_address(&context->trace_gc_end);
  context->thread_start_hook = rb_internal_thread_add_event_hook(thread_start_handler, RUBY_INTERNAL_THREAD_EVENT_STARTED, context);
  context->thread_ready_hook = rb_internal_thread_add_event_hook(thread_ready_handler, RUBY_INTERNAL_THREAD_EVENT_READY, context);
  context->thread_suspended_hook = rb_internal_thread_add_event_hook(thread_suspended_handler, RUBY_INTERNAL_THREAD_EVENT_SUSPENDED, context);
  context->thread_resume_hook = rb_internal_thread_add_event_hook(thread_resume_handler, RUBY_INTERNAL_THREAD_EVENT_RESUMED, context);
  context->thread_exit_hook = rb_internal_thread_add_event_hook(thread_exit_handler, RUBY_INTERNAL_THREAD_EVENT_EXITED, context);

  rb_tracepoint_enable(context->trace_call);
  rb_tracepoint_enable(context->trace_return);
  rb_tracepoint_enable(context->trace_gc_start);
  rb_tracepoint_enable(context->trace_gc_end);

  context->started = 1;
  return Qtrue;
}

RUBY_FUNC_EXPORTED void
Init_rrtrace(void)
{
  TraceContext *context = &trace_context;
  context->shared_memory = invalid_shared_memory_handle();
  context->event_ringbuffer = NULL;
  context->visualizer_process_id = invalid_process_id();
  context->thread_start_hook = NULL;
  context->thread_ready_hook = NULL;
  context->thread_suspended_hook = NULL;
  context->thread_resume_hook = NULL;
  context->thread_exit_hook = NULL;
  context->trace_call = Qnil;
  context->trace_return = Qnil;
  context->trace_gc_start = Qnil;
  context->trace_gc_end = Qnil;
  context->thread_data_key = rb_internal_thread_specific_key_create();
  atomic_init(&context->next_thread_id, 1);
  atomic_flag_clear(&context->event_ringbuffer_lock);
  context->started = 0;
#ifdef RRTRACE_WRITE_DEBUG_LOG
  context->log = fopen("rrtrace.log", "w");
#endif

  VALUE mRrtrace = rb_const_get(rb_cObject, rb_intern("Rrtrace"));
  rb_define_singleton_method(mRrtrace, "native_start", rrtrace_native_start, 1);
  rb_define_singleton_method(mRrtrace, "native_stop", rrtrace_native_stop, 0);
  rb_define_singleton_method(mRrtrace, "native_started?", rrtrace_native_started_p, 0);
}
