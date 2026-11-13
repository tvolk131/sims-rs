#import "window_present_macos.h"

#import <AppKit/AppKit.h>
#import <Foundation/Foundation.h>
#import <Metal/Metal.h>
#import <QuartzCore/QuartzCore.h>
#import <objc/runtime.h>

#include <errno.h>
#include <math.h>
#include <pthread.h>
#include <stdatomic.h>
#include <stdbool.h>
#include <stddef.h>
#include <time.h>

_Static_assert(sizeof(SimsWindowPresentEvent) == SIMS_WINDOW_PRESENT_EVENT_SIZE,
               "SimsWindowPresentEvent ABI size");
_Static_assert(sizeof(SimsWindowPresentCounters) == SIMS_WINDOW_PRESENT_COUNTERS_SIZE,
               "SimsWindowPresentCounters ABI size");
_Static_assert(sizeof(SimsWindowPresentSummary) == SIMS_WINDOW_PRESENT_SUMMARY_SIZE,
               "SimsWindowPresentSummary ABI size");
_Static_assert(offsetof(SimsWindowPresentEvent, host_time) == 104,
               "SimsWindowPresentEvent timestamp offset");
_Static_assert(offsetof(SimsWindowPresentSummary, counters) == 104,
               "SimsWindowPresentSummary counters offset");

typedef id<CAMetalDrawable> (*SimsNextDrawableImplementation)(CAMetalLayer *, SEL);

typedef struct SimsDrawContext {
    uint64_t sequence;
    uint64_t frame_id;
    uint64_t state_id;
    uint32_t valid;
} SimsDrawContext;

typedef struct SimsAcquisitionSlot {
    SimsWindowPresentEvent event;
    uint32_t used;
    uint32_t active;
    uint32_t successful;
    uint32_t current_texture;
    uint32_t callback_seen;
    uint32_t callback_expected;
    uint32_t missing_bind_recorded;
    uint32_t unresolved_reported;
} SimsAcquisitionSlot;

static pthread_mutex_t sims_mutex = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t sims_condition = PTHREAD_COND_INITIALIZER;
static _Atomic(IMP) sims_original_implementation;
static atomic_bool sims_tracing_enabled;
static _Thread_local SimsDrawContext sims_draw_context;
static _Thread_local uint32_t sims_callback_depth;
static SimsAcquisitionSlot sims_slots[SIMS_WINDOW_PRESENT_CAPACITY];
static SimsWindowPresentCounters sims_counters;
static SimsWindowPresentCallback sims_callback;
static void *sims_callback_context;
static Method sims_next_drawable_method;
static uint64_t sims_event_sequence;
static uint64_t sims_acquisition_sequence;
static uint64_t sims_layer_sequence;
static uint64_t sims_context_sequence;
static uint64_t sims_capabilities;
static uint32_t sims_session_state;
static int32_t sims_fault_status;
static uint32_t sims_end_in_progress;
static uint32_t sims_end_completed;
static uint32_t sims_drain_timed_out;
static double sims_begin_host_time;
static double sims_cutoff_host_time;
static double sims_drain_end_host_time;
static unsigned char sims_layer_token_key;
static id sims_notification_tokens[3];

static id<CAMetalDrawable> sims_next_drawable(CAMetalLayer *layer, SEL selector);

static SimsWindowPresentEvent sims_new_event(uint32_t kind, double host_time) {
    SimsWindowPresentEvent event = {0};
    event.abi_version = SIMS_WINDOW_PRESENT_ABI_VERSION;
    event.struct_size = sizeof(event);
    event.kind = kind;
    event.host_time = host_time;
    return event;
}

static void sims_emit_locked(SimsWindowPresentEvent event) {
    event.event_sequence = ++sims_event_sequence;
    if (sims_cutoff_host_time != 0.0) {
        event.flags |= SIMS_WINDOW_PRESENT_FLAG_AFTER_CUTOFF;
    }
    if (sims_end_completed) {
        event.flags |= SIMS_WINDOW_PRESENT_FLAG_AFTER_END;
    }
    sims_counters.emitted_events += 1;
    sims_callback_depth += 1;
    sims_callback(&event, sims_callback_context);
    sims_callback_depth -= 1;
}

static SimsWindowPresentEvent sims_slot_event(const SimsAcquisitionSlot *slot,
                                              uint32_t kind,
                                              double host_time) {
    SimsWindowPresentEvent event = slot->event;
    event.kind = kind;
    event.host_time = host_time;
    if (slot->active) {
        event.flags |= SIMS_WINDOW_PRESENT_FLAG_ACQUISITION_ACTIVE;
    }
    if (slot->successful && slot->event.bind_count == 0) {
        event.flags |= SIMS_WINDOW_PRESENT_FLAG_MISSING_BIND;
    }
    if (slot->missing_bind_recorded) {
        event.flags |= SIMS_WINDOW_PRESENT_FLAG_MISSING_BIND_RECORDED;
    }
    return event;
}

static void sims_record_missing_bind_locked(SimsAcquisitionSlot *slot) {
    if (slot->successful && slot->event.bind_count == 0 && !slot->missing_bind_recorded) {
        slot->missing_bind_recorded = 1;
        sims_counters.missing_binds += 1;
    }
}

static void sims_fail_locked(int32_t status, SimsWindowPresentEvent event) {
    if (sims_fault_status == SIMS_WINDOW_PRESENT_OK) {
        sims_fault_status = status;
    }
    atomic_store_explicit(&sims_tracing_enabled, false, memory_order_release);
    if (sims_cutoff_host_time == 0.0) {
        sims_cutoff_host_time = CACurrentMediaTime();
    }
    if (sims_session_state == SIMS_WINDOW_PRESENT_SESSION_ACTIVE) {
        sims_session_state = SIMS_WINDOW_PRESENT_SESSION_CUTOFF;
    }
    event.kind = SIMS_WINDOW_PRESENT_EVENT_FAULT;
    event.status = status;
    event.flags |= SIMS_WINDOW_PRESENT_FLAG_OBSERVATION_FAULT;
    sims_emit_locked(event);
    pthread_cond_broadcast(&sims_condition);
}

static uint64_t sims_layer_lifetime_locked(CAMetalLayer *layer) {
    NSNumber *token = objc_getAssociatedObject(layer, &sims_layer_token_key);
    if (token == nil) {
        uint64_t lifetime_id = ++sims_layer_sequence;
        token = [[NSNumber alloc] initWithUnsignedLongLong:lifetime_id];
        objc_setAssociatedObject(layer, &sims_layer_token_key, token,
                                 OBJC_ASSOCIATION_RETAIN_NONATOMIC);
    }
    return token.unsignedLongLongValue;
}

static uint32_t sims_reserve_slot_locked(void) {
    uint32_t selected = SIMS_WINDOW_PRESENT_CAPACITY;
    uint64_t oldest_sequence = UINT64_MAX;
    for (uint32_t slot_index = 0; slot_index < SIMS_WINDOW_PRESENT_CAPACITY; ++slot_index) {
        SimsAcquisitionSlot *slot = &sims_slots[slot_index];
        if (!slot->used) {
            return slot_index;
        }
        if (!slot->active && (!slot->successful || slot->callback_seen)
            && slot->event.acquisition_sequence < oldest_sequence) {
            selected = slot_index;
            oldest_sequence = slot->event.acquisition_sequence;
        }
    }
    return selected;
}

static void sims_make_texture_current_locked(uint32_t selected, double host_time) {
    SimsAcquisitionSlot *current = &sims_slots[selected];
    if (current->event.texture_identity == 0) {
        return;
    }
    uint32_t identity_conflict = 0;
    for (uint32_t slot_index = 0; slot_index < SIMS_WINDOW_PRESENT_CAPACITY; ++slot_index) {
        SimsAcquisitionSlot *previous = &sims_slots[slot_index];
        if (slot_index == selected || !previous->used || !previous->current_texture
            || previous->event.texture_identity != current->event.texture_identity) {
            continue;
        }
        previous->current_texture = 0;
        previous->event.flags |= SIMS_WINDOW_PRESENT_FLAG_TEXTURE_SUPERSEDED;
        sims_counters.texture_lifetime_reuses += 1;
        if (!previous->callback_seen) {
            sims_counters.texture_reuses_before_callback += 1;
        }
        if (previous->active) {
            identity_conflict = 1;
            previous->event.flags |= SIMS_WINDOW_PRESENT_FLAG_TEXTURE_CONFLICT;
            sims_counters.texture_identity_conflicts += 1;
        }
        sims_record_missing_bind_locked(previous);
        sims_emit_locked(sims_slot_event(previous,
                                        SIMS_WINDOW_PRESENT_EVENT_TEXTURE_SUPERSEDED,
                                        host_time));
    }
    if (identity_conflict) {
        current->event.flags |= SIMS_WINDOW_PRESENT_FLAG_TEXTURE_CONFLICT;
        sims_fail_locked(SIMS_WINDOW_PRESENT_ERROR_TEXTURE_CONFLICT,
                         sims_slot_event(current, SIMS_WINDOW_PRESENT_EVENT_FAULT, host_time));
    } else {
        current->current_texture = 1;
    }
}

static void sims_presented(uint32_t slot_index,
                           uint64_t acquisition_sequence,
                           uint64_t layer_lifetime_id,
                           uint64_t drawable_id,
                           uint64_t texture_identity,
                           double presented_host_time,
                           double callback_host_time,
                           uint32_t observation_failed) {
    pthread_mutex_lock(&sims_mutex);
    sims_counters.presented_callbacks += 1;
    if (sims_end_completed) {
        sims_counters.late_callbacks += 1;
    }
    SimsAcquisitionSlot *slot = &sims_slots[slot_index];
    SimsWindowPresentEvent event;
    if (!slot->used || slot->event.acquisition_sequence != acquisition_sequence) {
        sims_counters.callback_lookup_conflicts += 1;
        sims_counters.duplicate_callbacks += 1;
        event = sims_new_event(SIMS_WINDOW_PRESENT_EVENT_PRESENTED_CALLBACK, callback_host_time);
        event.acquisition_sequence = acquisition_sequence;
        event.layer_lifetime_id = layer_lifetime_id;
        event.drawable_id = drawable_id;
        event.texture_identity = texture_identity;
        event.flags |= SIMS_WINDOW_PRESENT_FLAG_LOOKUP_CONFLICT
            | SIMS_WINDOW_PRESENT_FLAG_DUPLICATE | SIMS_WINDOW_PRESENT_FLAG_DRAWABLE_VALID;
    } else if (slot->callback_seen) {
        sims_counters.duplicate_callbacks += 1;
        event = sims_slot_event(slot, SIMS_WINDOW_PRESENT_EVENT_PRESENTED_CALLBACK,
                                callback_host_time);
        event.flags |= SIMS_WINDOW_PRESENT_FLAG_DUPLICATE;
    } else {
        slot->callback_seen = 1;
        slot->event.presented_host_time = presented_host_time;
        slot->event.callback_delivery_host_time = callback_host_time;
        sims_counters.outstanding_acquisitions -= 1;
        if (slot->callback_expected) {
            slot->callback_expected = 0;
            sims_counters.callbacks_awaited -= 1;
        }
        if (observation_failed || !isfinite(presented_host_time) || presented_host_time < 0.0) {
            sims_counters.invalid_presented_timestamps += 1;
            slot->event.flags |= SIMS_WINDOW_PRESENT_FLAG_INVALID_PRESENTED_TIME;
        } else if (presented_host_time > 0.0) {
            sims_counters.positive_presentations += 1;
        } else {
            sims_counters.zero_presented_timestamps += 1;
        }
        sims_record_missing_bind_locked(slot);
        event = sims_slot_event(slot, SIMS_WINDOW_PRESENT_EVENT_PRESENTED_CALLBACK,
                                callback_host_time);
    }
    event.kind = SIMS_WINDOW_PRESENT_EVENT_PRESENTED_CALLBACK;
    event.host_time = callback_host_time;
    event.presented_host_time = presented_host_time;
    event.callback_delivery_host_time = callback_host_time;
    if (observation_failed || !isfinite(presented_host_time) || presented_host_time < 0.0) {
        sims_counters.observation_errors += 1;
        event.flags |= SIMS_WINDOW_PRESENT_FLAG_INVALID_PRESENTED_TIME
            | SIMS_WINDOW_PRESENT_FLAG_OBSERVATION_FAULT;
        event.status = SIMS_WINDOW_PRESENT_ERROR_OBSERVATION;
        sims_fail_locked(SIMS_WINDOW_PRESENT_ERROR_OBSERVATION, event);
    }
    sims_emit_locked(event);
    pthread_cond_broadcast(&sims_condition);
    pthread_mutex_unlock(&sims_mutex);
}

static id<CAMetalDrawable> sims_next_drawable(CAMetalLayer *layer, SEL selector) {
    IMP original_implementation = atomic_load_explicit(&sims_original_implementation,
                                                       memory_order_acquire);
    if (original_implementation == NULL) {
        pthread_mutex_lock(&sims_mutex);
        original_implementation = atomic_load_explicit(&sims_original_implementation,
                                                       memory_order_acquire);
        pthread_mutex_unlock(&sims_mutex);
    }
    SimsNextDrawableImplementation original = (SimsNextDrawableImplementation)original_implementation;
    if (sims_callback_depth != 0) {
        sims_counters.reentrant_acquisition_calls += 1;
        sims_counters.observation_errors += 1;
        if (sims_fault_status == SIMS_WINDOW_PRESENT_OK) {
            sims_fault_status = SIMS_WINDOW_PRESENT_ERROR_REENTRANT;
        }
        atomic_store_explicit(&sims_tracing_enabled, false, memory_order_release);
        if (sims_cutoff_host_time == 0.0) {
            sims_cutoff_host_time = CACurrentMediaTime();
        }
        if (sims_session_state == SIMS_WINDOW_PRESENT_SESSION_ACTIVE) {
            sims_session_state = SIMS_WINDOW_PRESENT_SESSION_CUTOFF;
        }
        return original(layer, selector);
    }
    if (!atomic_load_explicit(&sims_tracing_enabled, memory_order_acquire)) {
        return original(layer, selector);
    }
    double entry_host_time = CACurrentMediaTime();
    pthread_mutex_lock(&sims_mutex);
    if (!atomic_load_explicit(&sims_tracing_enabled, memory_order_acquire)) {
        pthread_mutex_unlock(&sims_mutex);
        return original(layer, selector);
    }
    uint32_t slot_index = sims_reserve_slot_locked();
    if (slot_index == SIMS_WINDOW_PRESENT_CAPACITY) {
        sims_counters.bookkeeping_overflows += 1;
        sims_counters.overflow_untraced_calls += 1;
        sims_fail_locked(SIMS_WINDOW_PRESENT_ERROR_BOOKKEEPING_OVERFLOW,
                         sims_new_event(SIMS_WINDOW_PRESENT_EVENT_FAULT, entry_host_time));
        pthread_mutex_unlock(&sims_mutex);
        return original(layer, selector);
    }
    SimsAcquisitionSlot *slot = &sims_slots[slot_index];
    *slot = (SimsAcquisitionSlot){0};
    slot->used = 1;
    slot->active = 1;
    slot->event = sims_new_event(SIMS_WINDOW_PRESENT_EVENT_ACQUISITION_ENTER, entry_host_time);
    slot->event.acquisition_sequence = ++sims_acquisition_sequence;
    slot->event.acquisition_enter_host_time = entry_host_time;
    if (sims_draw_context.valid) {
        slot->event.flags |= SIMS_WINDOW_PRESENT_FLAG_DRAW_CONTEXT_VALID;
        slot->event.draw_context_sequence = sims_draw_context.sequence;
        slot->event.frame_id = sims_draw_context.frame_id;
        slot->event.state_id = sims_draw_context.state_id;
    } else {
        sims_counters.missing_draw_contexts += 1;
    }
    sims_counters.acquisition_entries += 1;
    sims_counters.active_acquisition_calls += 1;
    @try {
        slot->event.layer_lifetime_id = sims_layer_lifetime_locked(layer);
        if (slot->event.layer_lifetime_id == 0) {
            sims_counters.observation_errors += 1;
            slot->event.flags |= SIMS_WINDOW_PRESENT_FLAG_OBSERVATION_FAULT;
            sims_fail_locked(SIMS_WINDOW_PRESENT_ERROR_OBSERVATION, slot->event);
        }
    } @catch (__unused NSException *exception) {
        sims_counters.observation_errors += 1;
        slot->event.flags |= SIMS_WINDOW_PRESENT_FLAG_OBSERVATION_FAULT;
        sims_fail_locked(SIMS_WINDOW_PRESENT_ERROR_OBSERVATION, slot->event);
    }
    sims_emit_locked(sims_slot_event(slot, SIMS_WINDOW_PRESENT_EVENT_ACQUISITION_ENTER,
                                    entry_host_time));
    pthread_mutex_unlock(&sims_mutex);

    @try {
        double call_host_time = CACurrentMediaTime();
        id<CAMetalDrawable> drawable;
        @try {
            drawable = original(layer, selector);
        } @catch (__unused NSException *exception) {
            double exception_host_time = CACurrentMediaTime();
            pthread_mutex_lock(&sims_mutex);
            slot->event.acquisition_call_host_time = call_host_time;
            sims_counters.acquisition_exceptions += 1;
            sims_fail_locked(SIMS_WINDOW_PRESENT_ERROR_ORIGINAL_EXCEPTION,
                             sims_slot_event(slot, SIMS_WINDOW_PRESENT_EVENT_FAULT,
                                             exception_host_time));
            pthread_mutex_unlock(&sims_mutex);
            @throw;
        }
        double return_host_time = CACurrentMediaTime();
        pthread_mutex_lock(&sims_mutex);
        slot->event.acquisition_call_host_time = call_host_time;
        slot->event.acquisition_return_host_time = return_host_time;
        sims_counters.acquisition_returns += 1;
        if (drawable == nil) {
            slot->event.flags |= SIMS_WINDOW_PRESENT_FLAG_NULL_DRAWABLE;
            sims_counters.acquisition_null_returns += 1;
            sims_emit_locked(sims_slot_event(slot, SIMS_WINDOW_PRESENT_EVENT_ACQUISITION_RETURN,
                                            return_host_time));
            pthread_mutex_unlock(&sims_mutex);
            return nil;
        }
        slot->successful = 1;
        slot->event.flags |= SIMS_WINDOW_PRESENT_FLAG_DRAWABLE_VALID;
        sims_counters.successful_acquisitions += 1;
        sims_counters.outstanding_acquisitions += 1;
        pthread_mutex_unlock(&sims_mutex);

        uint64_t drawable_id = 0;
        uint64_t texture_identity = 0;
        CGSize drawable_size = CGSizeZero;
        uint32_t display_sync_enabled = 0;
        uint32_t maximum_drawable_count = 0;
        uint32_t allows_timeout = 0;
        uint32_t presents_with_transaction = 0;
        uint32_t settings_valid = 0;
        uint32_t metadata_failed = 0;
        @try {
            texture_identity = (uint64_t)(uintptr_t)(__bridge const void *)drawable.texture;
            drawable_id = drawable.drawableID;
        } @catch (__unused NSException *exception) {
            metadata_failed = 1;
        }
        @try {
            drawable_size = layer.drawableSize;
            display_sync_enabled = layer.displaySyncEnabled ? 1 : 0;
            maximum_drawable_count = (uint32_t)layer.maximumDrawableCount;
            allows_timeout = layer.allowsNextDrawableTimeout ? 1 : 0;
            presents_with_transaction = layer.presentsWithTransaction ? 1 : 0;
            settings_valid = 1;
        } @catch (__unused NSException *exception) {
            metadata_failed = 1;
        }
        pthread_mutex_lock(&sims_mutex);
        slot->event.drawable_id = drawable_id;
        slot->event.texture_identity = texture_identity;
        slot->event.drawable_width = drawable_size.width;
        slot->event.drawable_height = drawable_size.height;
        slot->event.display_sync_enabled = display_sync_enabled;
        slot->event.maximum_drawable_count = maximum_drawable_count;
        slot->event.allows_next_drawable_timeout = allows_timeout;
        slot->event.presents_with_transaction = presents_with_transaction;
        if (settings_valid) {
            slot->event.flags |= SIMS_WINDOW_PRESENT_FLAG_LAYER_SETTINGS_VALID;
        }
        if (metadata_failed || texture_identity == 0) {
            sims_counters.observation_errors += 1;
            slot->event.flags |= SIMS_WINDOW_PRESENT_FLAG_OBSERVATION_FAULT;
            sims_fail_locked(SIMS_WINDOW_PRESENT_ERROR_OBSERVATION,
                             sims_slot_event(slot, SIMS_WINDOW_PRESENT_EVENT_FAULT,
                                             CACurrentMediaTime()));
        }
        sims_make_texture_current_locked(slot_index, CACurrentMediaTime());
        slot->callback_expected = 1;
        sims_counters.callbacks_awaited += 1;
        sims_counters.handler_registration_attempts += 1;
        uint64_t acquisition_sequence = slot->event.acquisition_sequence;
        uint64_t layer_lifetime_id = slot->event.layer_lifetime_id;
        sims_emit_locked(sims_slot_event(slot, SIMS_WINDOW_PRESENT_EVENT_ACQUISITION_RETURN,
                                        return_host_time));
        pthread_mutex_unlock(&sims_mutex);
        @try {
            [drawable addPresentedHandler:^(id<MTLDrawable> presented_drawable) {
                double callback_host_time = CACurrentMediaTime();
                double presented_host_time = 0.0;
                uint32_t observation_failed = 0;
                @try {
                    presented_host_time = presented_drawable.presentedTime;
                } @catch (__unused NSException *exception) {
                    observation_failed = 1;
                }
                sims_presented(slot_index, acquisition_sequence, layer_lifetime_id,
                               drawable_id, texture_identity, presented_host_time,
                               callback_host_time, observation_failed);
            }];
            pthread_mutex_lock(&sims_mutex);
            sims_counters.handler_registrations += 1;
            sims_emit_locked(sims_slot_event(slot, SIMS_WINDOW_PRESENT_EVENT_HANDLER_REGISTERED,
                                            CACurrentMediaTime()));
            pthread_mutex_unlock(&sims_mutex);
        } @catch (__unused NSException *exception) {
            pthread_mutex_lock(&sims_mutex);
            sims_counters.observation_errors += 1;
            sims_counters.handler_registration_failures += 1;
            slot->event.flags |= SIMS_WINDOW_PRESENT_FLAG_OBSERVATION_FAULT;
            sims_fail_locked(SIMS_WINDOW_PRESENT_ERROR_OBSERVATION,
                             sims_slot_event(slot, SIMS_WINDOW_PRESENT_EVENT_FAULT,
                                             CACurrentMediaTime()));
            pthread_mutex_unlock(&sims_mutex);
        }
        return drawable;
    } @finally {
        pthread_mutex_lock(&sims_mutex);
        slot->active = 0;
        sims_counters.active_acquisition_calls -= 1;
        pthread_cond_broadcast(&sims_condition);
        pthread_mutex_unlock(&sims_mutex);
    }
}

static void sims_platform_notification(uint32_t notification_kind) {
    if (sims_callback_depth != 0
        || !atomic_load_explicit(&sims_tracing_enabled, memory_order_acquire)) {
        return;
    }
    double host_time = CACurrentMediaTime();
    pthread_mutex_lock(&sims_mutex);
    if (atomic_load_explicit(&sims_tracing_enabled, memory_order_acquire)) {
        SimsWindowPresentEvent event = sims_new_event(SIMS_WINDOW_PRESENT_EVENT_PLATFORM, host_time);
        event.platform_notification = notification_kind;
        sims_counters.platform_notifications += 1;
        sims_emit_locked(event);
    }
    pthread_mutex_unlock(&sims_mutex);
}

static id sims_observe_notification(NSNotificationCenter *center,
                                    NSNotificationName name,
                                    uint32_t notification_kind) {
    return [center addObserverForName:name object:nil queue:nil
                          usingBlock:^(__unused NSNotification *notification) {
        sims_platform_notification(notification_kind);
    }];
}

static int32_t sims_current_status_locked(void) {
    if (sims_fault_status != SIMS_WINDOW_PRESENT_OK) {
        return sims_fault_status;
    }
    if (sims_session_state == SIMS_WINDOW_PRESENT_SESSION_NOT_STARTED) {
        return SIMS_WINDOW_PRESENT_ERROR_NOT_ACTIVE;
    }
    if (sims_drain_timed_out) {
        return SIMS_WINDOW_PRESENT_DRAIN_TIMEOUT;
    }
    return SIMS_WINDOW_PRESENT_OK;
}

static void sims_summary_locked(SimsWindowPresentSummary *summary) {
    *summary = (SimsWindowPresentSummary){0};
    summary->abi_version = SIMS_WINDOW_PRESENT_ABI_VERSION;
    summary->struct_size = sizeof(*summary);
    summary->status = sims_current_status_locked();
    summary->session_state = sims_session_state;
    summary->capabilities = sims_capabilities;
    summary->begin_host_time = sims_begin_host_time;
    summary->cutoff_host_time = sims_cutoff_host_time;
    summary->drain_end_host_time = sims_drain_end_host_time;
    summary->snapshot_host_time = CACurrentMediaTime();
    summary->last_event_sequence = sims_event_sequence;
    summary->last_acquisition_sequence = sims_acquisition_sequence;
    summary->unbound_acquisitions = sims_counters.successful_acquisitions
        - sims_counters.successful_binds;
    summary->counters = sims_counters;
    if (sims_drain_timed_out) {
        summary->flags |= SIMS_WINDOW_PRESENT_SUMMARY_DRAIN_TIMED_OUT;
    }
    if (sims_next_drawable_method != NULL
        && method_getImplementation(sims_next_drawable_method) != (IMP)sims_next_drawable) {
        summary->flags |= SIMS_WINDOW_PRESENT_SUMMARY_HOOK_REPLACED;
    }
    if (!(sims_capabilities & SIMS_WINDOW_PRESENT_CAPABILITY_WORKSPACE_NOTIFICATIONS)) {
        summary->flags |= SIMS_WINDOW_PRESENT_SUMMARY_WORKSPACE_NOTIFICATIONS_UNAVAILABLE;
    }
    if (!atomic_load_explicit(&sims_tracing_enabled, memory_order_acquire)) {
        summary->flags |= SIMS_WINDOW_PRESENT_SUMMARY_TRACING_DISABLED;
    }
    for (uint32_t slot_index = 0; slot_index < SIMS_WINDOW_PRESENT_CAPACITY; ++slot_index) {
        const SimsAcquisitionSlot *slot = &sims_slots[slot_index];
        if (slot->used && (slot->active || (slot->successful && !slot->callback_seen))) {
            summary->unresolved_acquisitions += 1;
            if (slot->successful && slot->event.bind_count == 0) {
                summary->unresolved_unbound_acquisitions += 1;
            }
        }
    }
}

int32_t sims_window_present_begin(uint32_t abi_version,
                                  uint32_t event_struct_size,
                                  SimsWindowPresentCallback callback,
                                  void *callback_context) {
    if (sims_callback_depth != 0) {
        return SIMS_WINDOW_PRESENT_ERROR_REENTRANT;
    }
    if (abi_version != SIMS_WINDOW_PRESENT_ABI_VERSION
        || event_struct_size != sizeof(SimsWindowPresentEvent)) {
        return SIMS_WINDOW_PRESENT_ERROR_ABI;
    }
    if (callback == NULL) {
        return SIMS_WINDOW_PRESENT_ERROR_ARGUMENT;
    }
    if (![NSThread isMainThread]) {
        return SIMS_WINDOW_PRESENT_ERROR_MAIN_THREAD_REQUIRED;
    }
    pthread_mutex_lock(&sims_mutex);
    if (sims_session_state != SIMS_WINDOW_PRESENT_SESSION_NOT_STARTED) {
        pthread_mutex_unlock(&sims_mutex);
        return SIMS_WINDOW_PRESENT_ERROR_ALREADY_STARTED;
    }
    Class layer_class = [CAMetalLayer class];
    Method method = class_getInstanceMethod(layer_class, @selector(nextDrawable));
    Method superclass_method = class_getInstanceMethod(class_getSuperclass(layer_class),
                                                       @selector(nextDrawable));
    char return_type[8] = {0};
    char receiver_type[8] = {0};
    char selector_type[8] = {0};
    if (method != NULL) {
        method_getReturnType(method, return_type, sizeof(return_type));
        method_getArgumentType(method, 0, receiver_type, sizeof(receiver_type));
        method_getArgumentType(method, 1, selector_type, sizeof(selector_type));
    }
    if (method == NULL || method == superclass_method || method_getNumberOfArguments(method) != 2
        || return_type[0] != '@' || receiver_type[0] != '@' || selector_type[0] != ':'
        || method_getImplementation(method) == NULL
        || method_getImplementation(method) == (IMP)sims_next_drawable) {
        sims_session_state = SIMS_WINDOW_PRESENT_SESSION_FAILED;
        sims_fault_status = SIMS_WINDOW_PRESENT_ERROR_RUNTIME_METHOD;
        pthread_mutex_unlock(&sims_mutex);
        return SIMS_WINDOW_PRESENT_ERROR_RUNTIME_METHOD;
    }
    @try {
        NSNotificationCenter *default_center = NSNotificationCenter.defaultCenter;
        sims_notification_tokens[0] = sims_observe_notification(
            default_center, NSApplicationDidBecomeActiveNotification,
            SIMS_WINDOW_PRESENT_PLATFORM_APP_BECAME_ACTIVE);
        sims_notification_tokens[1] = sims_observe_notification(
            default_center, NSApplicationDidResignActiveNotification,
            SIMS_WINDOW_PRESENT_PLATFORM_APP_RESIGNED_ACTIVE);
        sims_notification_tokens[2] = sims_observe_notification(
            default_center, NSApplicationDidChangeScreenParametersNotification,
            SIMS_WINDOW_PRESENT_PLATFORM_DISPLAY_CHANGED);
        sims_capabilities = SIMS_WINDOW_PRESENT_CAPABILITY_LAYER_SETTINGS
            | SIMS_WINDOW_PRESENT_CAPABILITY_APP_NOTIFICATIONS;
    } @catch (__unused NSException *exception) {
        sims_session_state = SIMS_WINDOW_PRESENT_SESSION_FAILED;
        sims_fault_status = SIMS_WINDOW_PRESENT_ERROR_OBSERVATION;
        pthread_mutex_unlock(&sims_mutex);
        return SIMS_WINDOW_PRESENT_ERROR_OBSERVATION;
    }
    sims_callback = callback;
    sims_callback_context = callback_context;
    sims_next_drawable_method = method;
    sims_begin_host_time = CACurrentMediaTime();
    IMP previous_implementation = method_setImplementation(method, (IMP)sims_next_drawable);
    atomic_store_explicit(&sims_original_implementation, previous_implementation,
                          memory_order_release);
    sims_session_state = SIMS_WINDOW_PRESENT_SESSION_ACTIVE;
    atomic_store_explicit(&sims_tracing_enabled, true, memory_order_release);
    sims_emit_locked(sims_new_event(SIMS_WINDOW_PRESENT_EVENT_BEGIN, sims_begin_host_time));
    int32_t status = sims_current_status_locked();
    pthread_mutex_unlock(&sims_mutex);
    if (NSApp != nil) {
        sims_platform_notification(NSApp.active
            ? SIMS_WINDOW_PRESENT_PLATFORM_INITIAL_APP_ACTIVE
            : SIMS_WINDOW_PRESENT_PLATFORM_INITIAL_APP_INACTIVE);
    }
    return status;
}

int32_t sims_window_present_draw_context(uint64_t frame_id, uint64_t state_id) {
    if (sims_callback_depth != 0) {
        return SIMS_WINDOW_PRESENT_ERROR_REENTRANT;
    }
    if (!atomic_load_explicit(&sims_tracing_enabled, memory_order_acquire)) {
        return SIMS_WINDOW_PRESENT_ERROR_NOT_ACTIVE;
    }
    double host_time = CACurrentMediaTime();
    pthread_mutex_lock(&sims_mutex);
    if (!atomic_load_explicit(&sims_tracing_enabled, memory_order_acquire)) {
        pthread_mutex_unlock(&sims_mutex);
        return SIMS_WINDOW_PRESENT_ERROR_NOT_ACTIVE;
    }
    sims_draw_context.sequence = ++sims_context_sequence;
    sims_draw_context.frame_id = frame_id;
    sims_draw_context.state_id = state_id;
    sims_draw_context.valid = 1;
    sims_counters.draw_context_calls += 1;
    SimsWindowPresentEvent event = sims_new_event(SIMS_WINDOW_PRESENT_EVENT_DRAW_CONTEXT, host_time);
    event.flags |= SIMS_WINDOW_PRESENT_FLAG_DRAW_CONTEXT_VALID;
    event.draw_context_sequence = sims_draw_context.sequence;
    event.frame_id = frame_id;
    event.state_id = state_id;
    sims_emit_locked(event);
    pthread_mutex_unlock(&sims_mutex);
    return SIMS_WINDOW_PRESENT_OK;
}

int32_t sims_window_present_bind(uint64_t texture_identity, uint64_t frame_id, uint64_t state_id) {
    if (sims_callback_depth != 0) {
        return SIMS_WINDOW_PRESENT_ERROR_REENTRANT;
    }
    double host_time = CACurrentMediaTime();
    pthread_mutex_lock(&sims_mutex);
    if (sims_callback == NULL) {
        pthread_mutex_unlock(&sims_mutex);
        return SIMS_WINDOW_PRESENT_ERROR_NOT_ACTIVE;
    }
    sims_counters.bind_calls += 1;
    if (sims_end_completed) {
        sims_counters.late_binds += 1;
    }
    if (texture_identity == 0
        || !atomic_load_explicit(&sims_tracing_enabled, memory_order_acquire)) {
        sims_counters.binding_conflicts += 1;
        SimsWindowPresentEvent event = sims_new_event(SIMS_WINDOW_PRESENT_EVENT_BIND, host_time);
        event.texture_identity = texture_identity;
        event.bound_frame_id = frame_id;
        event.bound_state_id = state_id;
        event.status = texture_identity == 0
            ? SIMS_WINDOW_PRESENT_ERROR_ARGUMENT : SIMS_WINDOW_PRESENT_ERROR_NOT_ACTIVE;
        event.flags |= SIMS_WINDOW_PRESENT_FLAG_BIND_CONFLICT;
        sims_emit_locked(event);
        pthread_mutex_unlock(&sims_mutex);
        return event.status;
    }
    SimsAcquisitionSlot *selected = NULL;
    uint32_t match_count = 0;
    for (uint32_t slot_index = 0; slot_index < SIMS_WINDOW_PRESENT_CAPACITY; ++slot_index) {
        SimsAcquisitionSlot *slot = &sims_slots[slot_index];
        if (slot->used && slot->current_texture
            && slot->event.texture_identity == texture_identity) {
            selected = slot;
            match_count += 1;
        }
    }
    if (match_count != 1) {
        sims_counters.bind_lookup_conflicts += 1;
        SimsWindowPresentEvent event = sims_new_event(SIMS_WINDOW_PRESENT_EVENT_BIND, host_time);
        event.texture_identity = texture_identity;
        event.bound_frame_id = frame_id;
        event.bound_state_id = state_id;
        event.status = SIMS_WINDOW_PRESENT_ERROR_BIND_LOOKUP;
        event.flags |= SIMS_WINDOW_PRESENT_FLAG_LOOKUP_CONFLICT;
        sims_emit_locked(event);
        pthread_mutex_unlock(&sims_mutex);
        return SIMS_WINDOW_PRESENT_ERROR_BIND_LOOKUP;
    }
    SimsWindowPresentEvent event = sims_slot_event(selected, SIMS_WINDOW_PRESENT_EVENT_BIND, host_time);
    event.bound_frame_id = frame_id;
    event.bound_state_id = state_id;
    int32_t status = SIMS_WINDOW_PRESENT_OK;
    if (selected->callback_seen) {
        sims_counters.post_callback_bind_attempts += 1;
        event.flags |= SIMS_WINDOW_PRESENT_FLAG_BIND_AFTER_CALLBACK;
    }
    if (!(selected->event.flags & SIMS_WINDOW_PRESENT_FLAG_DRAW_CONTEXT_VALID)
        || selected->event.frame_id != frame_id || selected->event.state_id != state_id
        || (selected->callback_seen && selected->event.bind_count == 0)) {
        sims_counters.binding_conflicts += 1;
        event.flags |= SIMS_WINDOW_PRESENT_FLAG_BIND_CONFLICT;
        status = SIMS_WINDOW_PRESENT_ERROR_BIND_CONFLICT;
    } else {
        if (selected->event.bind_count != 0) {
            sims_counters.duplicate_binds += 1;
            event.flags |= SIMS_WINDOW_PRESENT_FLAG_DUPLICATE;
            status = SIMS_WINDOW_PRESENT_DUPLICATE_BIND;
        } else {
            sims_counters.successful_binds += 1;
        }
        selected->event.bind_count += 1;
        selected->event.bound_frame_id = frame_id;
        selected->event.bound_state_id = state_id;
        selected->event.flags |= SIMS_WINDOW_PRESENT_FLAG_BOUND;
        event.bind_count = selected->event.bind_count;
        event.flags |= SIMS_WINDOW_PRESENT_FLAG_BOUND;
        event.flags &= ~SIMS_WINDOW_PRESENT_FLAG_MISSING_BIND;
    }
    event.status = status;
    sims_emit_locked(event);
    pthread_mutex_unlock(&sims_mutex);
    return status;
}

int32_t sims_window_present_end(uint32_t timeout_ms,
                                SimsWindowPresentSummary *summary,
                                uint32_t summary_struct_size) {
    if (sims_callback_depth != 0) {
        return SIMS_WINDOW_PRESENT_ERROR_REENTRANT;
    }
    if (summary == NULL || timeout_ms > SIMS_WINDOW_PRESENT_MAX_DRAIN_TIMEOUT_MS) {
        return SIMS_WINDOW_PRESENT_ERROR_ARGUMENT;
    }
    if (summary_struct_size != sizeof(*summary)) {
        return SIMS_WINDOW_PRESENT_ERROR_ABI;
    }
    pthread_mutex_lock(&sims_mutex);
    if (sims_callback == NULL) {
        sims_summary_locked(summary);
        int32_t status = summary->status;
        pthread_mutex_unlock(&sims_mutex);
        return status;
    }
    if (sims_end_in_progress) {
        pthread_mutex_unlock(&sims_mutex);
        return SIMS_WINDOW_PRESENT_ERROR_BUSY;
    }
    if (sims_end_completed) {
        sims_summary_locked(summary);
        int32_t status = summary->status;
        pthread_mutex_unlock(&sims_mutex);
        return status;
    }
    sims_end_in_progress = 1;
    atomic_store_explicit(&sims_tracing_enabled, false, memory_order_release);
    double drain_start_host_time = CACurrentMediaTime();
    if (sims_cutoff_host_time == 0.0) {
        sims_cutoff_host_time = drain_start_host_time;
    }
    sims_session_state = SIMS_WINDOW_PRESENT_SESSION_DRAINING;
    SimsWindowPresentEvent cutoff_event = sims_new_event(SIMS_WINDOW_PRESENT_EVENT_CUTOFF,
                                                        sims_cutoff_host_time);
    cutoff_event.status = sims_current_status_locked();
    sims_emit_locked(cutoff_event);
    double deadline = drain_start_host_time + (double)timeout_ms / 1000.0;
    while (sims_counters.active_acquisition_calls != 0
           || sims_counters.outstanding_acquisitions != 0) {
        double remaining = deadline - CACurrentMediaTime();
        if (remaining <= 0.0) {
            sims_drain_timed_out = 1;
            break;
        }
        struct timespec relative_timeout;
        relative_timeout.tv_sec = (time_t)remaining;
        relative_timeout.tv_nsec = (long)((remaining - (double)relative_timeout.tv_sec) * 1000000000.0);
        int wait_status = pthread_cond_timedwait_relative_np(&sims_condition, &sims_mutex,
                                                            &relative_timeout);
        if (wait_status != 0 && wait_status != ETIMEDOUT) {
            sims_fail_locked(SIMS_WINDOW_PRESENT_ERROR_WAIT,
                             sims_new_event(SIMS_WINDOW_PRESENT_EVENT_FAULT, CACurrentMediaTime()));
            break;
        }
    }
    sims_drain_end_host_time = CACurrentMediaTime();
    for (uint32_t slot_index = 0; slot_index < SIMS_WINDOW_PRESENT_CAPACITY; ++slot_index) {
        SimsAcquisitionSlot *slot = &sims_slots[slot_index];
        if (!slot->used) {
            continue;
        }
        sims_record_missing_bind_locked(slot);
        if ((slot->active || (slot->successful && !slot->callback_seen))
            && !slot->unresolved_reported) {
            slot->unresolved_reported = 1;
            sims_counters.unresolved_events += 1;
            sims_emit_locked(sims_slot_event(slot, SIMS_WINDOW_PRESENT_EVENT_UNRESOLVED,
                                            sims_drain_end_host_time));
        }
    }
    sims_session_state = SIMS_WINDOW_PRESENT_SESSION_ENDED;
    SimsWindowPresentEvent end_event = sims_new_event(SIMS_WINDOW_PRESENT_EVENT_END,
                                                     sims_drain_end_host_time);
    end_event.status = sims_current_status_locked();
    sims_emit_locked(end_event);
    sims_end_completed = 1;
    sims_end_in_progress = 0;
    sims_summary_locked(summary);
    int32_t status = summary->status;
    pthread_mutex_unlock(&sims_mutex);
    return status;
}

int32_t sims_window_present_snapshot(SimsWindowPresentSummary *summary,
                                     uint32_t summary_struct_size) {
    if (sims_callback_depth != 0) {
        return SIMS_WINDOW_PRESENT_ERROR_REENTRANT;
    }
    if (summary == NULL) {
        return SIMS_WINDOW_PRESENT_ERROR_ARGUMENT;
    }
    if (summary_struct_size != sizeof(*summary)) {
        return SIMS_WINDOW_PRESENT_ERROR_ABI;
    }
    pthread_mutex_lock(&sims_mutex);
    sims_summary_locked(summary);
    int32_t status = summary->status;
    pthread_mutex_unlock(&sims_mutex);
    return status;
}
