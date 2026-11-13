#ifndef SIMS_WINDOW_PRESENT_MACOS_H
#define SIMS_WINDOW_PRESENT_MACOS_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define SIMS_WINDOW_PRESENT_EXPORT __attribute__((visibility("default")))

enum {
    SIMS_WINDOW_PRESENT_ABI_VERSION = 1,
    SIMS_WINDOW_PRESENT_CAPACITY = 512,
    SIMS_WINDOW_PRESENT_MAX_DRAIN_TIMEOUT_MS = 10000,
    SIMS_WINDOW_PRESENT_EVENT_SIZE = 192,
    SIMS_WINDOW_PRESENT_COUNTERS_SIZE = 304,
    SIMS_WINDOW_PRESENT_SUMMARY_SIZE = 408
};

enum {
    SIMS_WINDOW_PRESENT_OK = 0,
    SIMS_WINDOW_PRESENT_DRAIN_TIMEOUT = 1,
    SIMS_WINDOW_PRESENT_DUPLICATE_BIND = 2,
    SIMS_WINDOW_PRESENT_ERROR_ARGUMENT = -1,
    SIMS_WINDOW_PRESENT_ERROR_ABI = -2,
    SIMS_WINDOW_PRESENT_ERROR_ALREADY_STARTED = -3,
    SIMS_WINDOW_PRESENT_ERROR_NOT_ACTIVE = -4,
    SIMS_WINDOW_PRESENT_ERROR_RUNTIME_METHOD = -5,
    SIMS_WINDOW_PRESENT_ERROR_BOOKKEEPING_OVERFLOW = -6,
    SIMS_WINDOW_PRESENT_ERROR_OBSERVATION = -7,
    SIMS_WINDOW_PRESENT_ERROR_BIND_LOOKUP = -8,
    SIMS_WINDOW_PRESENT_ERROR_BIND_CONFLICT = -9,
    SIMS_WINDOW_PRESENT_ERROR_REENTRANT = -10,
    SIMS_WINDOW_PRESENT_ERROR_BUSY = -11,
    SIMS_WINDOW_PRESENT_ERROR_WAIT = -12,
    SIMS_WINDOW_PRESENT_ERROR_MAIN_THREAD_REQUIRED = -13,
    SIMS_WINDOW_PRESENT_ERROR_ORIGINAL_EXCEPTION = -14,
    SIMS_WINDOW_PRESENT_ERROR_TEXTURE_CONFLICT = -15
};

enum {
    SIMS_WINDOW_PRESENT_SESSION_NOT_STARTED = 0,
    SIMS_WINDOW_PRESENT_SESSION_ACTIVE = 1,
    SIMS_WINDOW_PRESENT_SESSION_CUTOFF = 2,
    SIMS_WINDOW_PRESENT_SESSION_DRAINING = 3,
    SIMS_WINDOW_PRESENT_SESSION_ENDED = 4,
    SIMS_WINDOW_PRESENT_SESSION_FAILED = 5
};

enum {
    SIMS_WINDOW_PRESENT_EVENT_BEGIN = 1,
    SIMS_WINDOW_PRESENT_EVENT_DRAW_CONTEXT = 2,
    SIMS_WINDOW_PRESENT_EVENT_ACQUISITION_ENTER = 3,
    SIMS_WINDOW_PRESENT_EVENT_ACQUISITION_RETURN = 4,
    SIMS_WINDOW_PRESENT_EVENT_HANDLER_REGISTERED = 5,
    SIMS_WINDOW_PRESENT_EVENT_PRESENTED_CALLBACK = 6,
    SIMS_WINDOW_PRESENT_EVENT_BIND = 7,
    SIMS_WINDOW_PRESENT_EVENT_TEXTURE_SUPERSEDED = 8,
    SIMS_WINDOW_PRESENT_EVENT_PLATFORM = 9,
    SIMS_WINDOW_PRESENT_EVENT_FAULT = 10,
    SIMS_WINDOW_PRESENT_EVENT_CUTOFF = 11,
    SIMS_WINDOW_PRESENT_EVENT_UNRESOLVED = 12,
    SIMS_WINDOW_PRESENT_EVENT_END = 13
};

enum {
    SIMS_WINDOW_PRESENT_FLAG_AFTER_CUTOFF = 1u << 0,
    SIMS_WINDOW_PRESENT_FLAG_AFTER_END = 1u << 1,
    SIMS_WINDOW_PRESENT_FLAG_DRAW_CONTEXT_VALID = 1u << 2,
    SIMS_WINDOW_PRESENT_FLAG_DRAWABLE_VALID = 1u << 3,
    SIMS_WINDOW_PRESENT_FLAG_LAYER_SETTINGS_VALID = 1u << 4,
    SIMS_WINDOW_PRESENT_FLAG_BOUND = 1u << 5,
    SIMS_WINDOW_PRESENT_FLAG_MISSING_BIND = 1u << 6,
    SIMS_WINDOW_PRESENT_FLAG_BIND_CONFLICT = 1u << 7,
    SIMS_WINDOW_PRESENT_FLAG_DUPLICATE = 1u << 8,
    SIMS_WINDOW_PRESENT_FLAG_LOOKUP_CONFLICT = 1u << 9,
    SIMS_WINDOW_PRESENT_FLAG_TEXTURE_SUPERSEDED = 1u << 10,
    SIMS_WINDOW_PRESENT_FLAG_OBSERVATION_FAULT = 1u << 11,
    SIMS_WINDOW_PRESENT_FLAG_ACQUISITION_ACTIVE = 1u << 12,
    SIMS_WINDOW_PRESENT_FLAG_MISSING_BIND_RECORDED = 1u << 13,
    SIMS_WINDOW_PRESENT_FLAG_INVALID_PRESENTED_TIME = 1u << 14,
    SIMS_WINDOW_PRESENT_FLAG_NULL_DRAWABLE = 1u << 15,
    SIMS_WINDOW_PRESENT_FLAG_BIND_AFTER_CALLBACK = 1u << 16,
    SIMS_WINDOW_PRESENT_FLAG_TEXTURE_CONFLICT = 1u << 17
};

enum {
    SIMS_WINDOW_PRESENT_PLATFORM_APP_BECAME_ACTIVE = 1,
    SIMS_WINDOW_PRESENT_PLATFORM_APP_RESIGNED_ACTIVE = 2,
    SIMS_WINDOW_PRESENT_PLATFORM_DISPLAY_CHANGED = 3,
    SIMS_WINDOW_PRESENT_PLATFORM_WILL_SLEEP = 4,
    SIMS_WINDOW_PRESENT_PLATFORM_DID_WAKE = 5,
    SIMS_WINDOW_PRESENT_PLATFORM_INITIAL_APP_ACTIVE = 6,
    SIMS_WINDOW_PRESENT_PLATFORM_INITIAL_APP_INACTIVE = 7
};

enum {
    SIMS_WINDOW_PRESENT_CAPABILITY_LAYER_SETTINGS = 1u << 0,
    SIMS_WINDOW_PRESENT_CAPABILITY_APP_NOTIFICATIONS = 1u << 1,
    SIMS_WINDOW_PRESENT_CAPABILITY_WORKSPACE_NOTIFICATIONS = 1u << 2
};

enum {
    SIMS_WINDOW_PRESENT_SUMMARY_DRAIN_TIMED_OUT = 1u << 0,
    SIMS_WINDOW_PRESENT_SUMMARY_HOOK_REPLACED = 1u << 1,
    SIMS_WINDOW_PRESENT_SUMMARY_WORKSPACE_NOTIFICATIONS_UNAVAILABLE = 1u << 2,
    SIMS_WINDOW_PRESENT_SUMMARY_TRACING_DISABLED = 1u << 3
};

typedef struct SimsWindowPresentEvent {
    uint32_t abi_version;
    uint32_t struct_size;
    uint32_t kind;
    uint32_t flags;
    uint64_t event_sequence;
    uint64_t acquisition_sequence;
    uint64_t layer_lifetime_id;
    uint64_t drawable_id;
    uint64_t texture_identity;
    uint64_t draw_context_sequence;
    uint64_t frame_id;
    uint64_t state_id;
    uint64_t bound_frame_id;
    uint64_t bound_state_id;
    uint64_t bind_count;
    double host_time;
    double acquisition_enter_host_time;
    double acquisition_call_host_time;
    double acquisition_return_host_time;
    double presented_host_time;
    double callback_delivery_host_time;
    double drawable_width;
    double drawable_height;
    uint32_t display_sync_enabled;
    uint32_t maximum_drawable_count;
    uint32_t allows_next_drawable_timeout;
    uint32_t presents_with_transaction;
    uint32_t platform_notification;
    int32_t status;
} SimsWindowPresentEvent;

typedef struct SimsWindowPresentCounters {
    uint64_t acquisition_entries;
    uint64_t acquisition_returns;
    uint64_t acquisition_null_returns;
    uint64_t successful_acquisitions;
    uint64_t acquisition_exceptions;
    uint64_t handler_registration_attempts;
    uint64_t handler_registrations;
    uint64_t handler_registration_failures;
    uint64_t presented_callbacks;
    uint64_t positive_presentations;
    uint64_t zero_presented_timestamps;
    uint64_t invalid_presented_timestamps;
    uint64_t duplicate_callbacks;
    uint64_t callback_lookup_conflicts;
    uint64_t bind_calls;
    uint64_t successful_binds;
    uint64_t duplicate_binds;
    uint64_t missing_binds;
    uint64_t bind_lookup_conflicts;
    uint64_t binding_conflicts;
    uint64_t missing_draw_contexts;
    uint64_t texture_lifetime_reuses;
    uint64_t texture_reuses_before_callback;
    uint64_t texture_identity_conflicts;
    uint64_t bookkeeping_overflows;
    uint64_t overflow_untraced_calls;
    uint64_t outstanding_acquisitions;
    uint64_t active_acquisition_calls;
    uint64_t callbacks_awaited;
    uint64_t emitted_events;
    uint64_t draw_context_calls;
    uint64_t platform_notifications;
    uint64_t observation_errors;
    uint64_t unresolved_events;
    uint64_t late_callbacks;
    uint64_t late_binds;
    uint64_t post_callback_bind_attempts;
    uint64_t reentrant_acquisition_calls;
} SimsWindowPresentCounters;

typedef struct SimsWindowPresentSummary {
    uint32_t abi_version;
    uint32_t struct_size;
    int32_t status;
    uint32_t session_state;
    uint64_t capabilities;
    uint64_t flags;
    double begin_host_time;
    double cutoff_host_time;
    double drain_end_host_time;
    double snapshot_host_time;
    uint64_t last_event_sequence;
    uint64_t last_acquisition_sequence;
    uint64_t unresolved_acquisitions;
    uint64_t unbound_acquisitions;
    uint64_t unresolved_unbound_acquisitions;
    SimsWindowPresentCounters counters;
} SimsWindowPresentSummary;

typedef void (*SimsWindowPresentCallback)(const SimsWindowPresentEvent *event,
                                        void *callback_context);

SIMS_WINDOW_PRESENT_EXPORT int32_t sims_window_present_begin(
    uint32_t abi_version,
    uint32_t event_struct_size,
    SimsWindowPresentCallback callback,
    void *callback_context);

SIMS_WINDOW_PRESENT_EXPORT int32_t sims_window_present_draw_context(
    uint64_t frame_id,
    uint64_t state_id);

SIMS_WINDOW_PRESENT_EXPORT int32_t sims_window_present_bind(
    uint64_t texture_identity,
    uint64_t frame_id,
    uint64_t state_id);

SIMS_WINDOW_PRESENT_EXPORT int32_t sims_window_present_end(
    uint32_t timeout_ms,
    SimsWindowPresentSummary *summary,
    uint32_t summary_struct_size);

SIMS_WINDOW_PRESENT_EXPORT int32_t sims_window_present_snapshot(
    SimsWindowPresentSummary *summary,
    uint32_t summary_struct_size);

#ifdef __cplusplus
}
#endif

#endif
