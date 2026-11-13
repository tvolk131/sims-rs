"""Validate opt-in Metal presentation evidence without inferring scanout FPS."""
import math


IDENTITY_FIELDS = ('acquisition_sequence', 'layer_lifetime_id', 'draw_context_sequence',
                   'frame_id', 'state_id', 'acquisition_enter_host_time')
RETURN_FIELDS = IDENTITY_FIELDS + (
    'drawable_id', 'texture_identity', 'acquisition_call_host_time',
    'acquisition_return_host_time', 'drawable_width', 'drawable_height',
    'display_sync_enabled', 'maximum_drawable_count', 'allows_next_drawable_timeout',
    'presents_with_transaction')
BIND_FIELDS = ('bind_count', 'bound_frame_id', 'bound_state_id')
INTEGER_FIELDS = (
    'abi_version', 'struct_size', 'kind', 'flags', 'event_sequence', 'acquisition_sequence',
    'layer_lifetime_id', 'drawable_id', 'texture_identity', 'draw_context_sequence',
    'frame_id', 'state_id', 'bound_frame_id', 'bound_state_id', 'bind_count',
    'display_sync_enabled', 'maximum_drawable_count', 'allows_next_drawable_timeout',
    'presents_with_transaction', 'platform_notification')
TIME_FIELDS = ('host_time', 'acquisition_enter_host_time', 'acquisition_call_host_time',
               'acquisition_return_host_time', 'presented_host_time', 'callback_delivery_host_time')
CONFIGURATION_FIELDS = ('display_sync_enabled', 'maximum_drawable_count',
                        'allows_next_drawable_timeout', 'presents_with_transaction')
FATAL_COUNTERS = (
    'acquisition_exceptions', 'handler_registration_failures',
    'invalid_presented_timestamps', 'duplicate_callbacks', 'callback_lookup_conflicts',
    'duplicate_binds', 'bind_lookup_conflicts', 'binding_conflicts', 'missing_draw_contexts',
    'texture_identity_conflicts', 'bookkeeping_overflows',
    'overflow_untraced_calls', 'outstanding_acquisitions', 'active_acquisition_calls',
    'callbacks_awaited', 'observation_errors', 'unresolved_events', 'late_callbacks',
    'late_binds', 'post_callback_bind_attempts', 'reentrant_acquisition_calls')
COUNTED_EVENTS = {'acquisition_entries': 3, 'acquisition_returns': 4,
                  'handler_registrations': 5, 'presented_callbacks': 6, 'bind_calls': 7,
                  'draw_context_calls': 2, 'platform_notifications': 9,
                  'texture_lifetime_reuses': 8}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def finite(value, positive=False):
    return (type(value) in (int, float) and math.isfinite(value)
            and (value > 0 if positive else value >= 0))


def same_fields(actual, expected, names):
    require(all(actual[name] == expected[name] for name in names),
            'Inconsistent acquisition identity, timing, configuration or binding')


def validate_sleep_clocks(records, begin_time, drain_time):
    clocks = [record['data'] for record in records if record['kind'] == 'presentation_sleep_clock']
    require(len(clocks) == 2 and [clock['phase'] for clock in clocks] == ['begin', 'end'],
            'Expected ordered begin/end sleep-clock evidence')
    samples = []
    offsets = []
    uncertainties = []
    for clock in clocks:
        require(clock['schema'] == 1 and clock['clock'] == (
            'mach timebase ticks; absolute stops during sleep; continuous includes sleep'),
            'Unsupported sleep-clock metadata')
        sample = clock['sample']
        require(all(type(sample[name]) is int and sample[name] > 0 for name in (
            'absolute_before', 'continuous', 'absolute_after', 'timebase_numer', 'timebase_denom')),
            'Invalid Mach sleep-clock sample')
        require(sample['absolute_before'] <= sample['absolute_after'], 'Reversed sleep-clock bracket')
        uncertainty = ((sample['absolute_after'] - sample['absolute_before'])
                       * sample['timebase_numer'] / sample['timebase_denom'] / 1000)
        require(uncertainty <= 1000, 'Sleep-clock sampling uncertainty exceeds one millisecond')
        offsets.append((sample['continuous'] - sample['absolute_after'],
                        sample['continuous'] - sample['absolute_before']))
        uncertainties.append(uncertainty)
        samples.append(sample)
    first, last = samples
    require(first['timebase_numer'] == last['timebase_numer']
            and first['timebase_denom'] == last['timebase_denom']
            and first['absolute_after'] <= last['absolute_before']
            and first['continuous'] <= last['continuous'], 'Sleep-clock domain or ordering changed')
    scale = first['timebase_numer'] / first['timebase_denom'] / 1_000_000_000
    require(first['absolute_after'] * scale <= begin_time
            and last['absolute_before'] * scale >= drain_time,
            'Sleep-clock evidence does not cover presentation measurement and drain')
    require(max(offset[0] for offset in offsets) <= min(offset[1] for offset in offsets),
            'Sleep or discontinuous Mach clock offset detected')
    return {'valid': True, 'sampling_uncertainty_us': uncertainties,
            'scope': 'Absolute/continuous offset brackets cover measurement and drain; '
                     'offset changes beyond sampling uncertainty invalidate the session'}


def validate_evidence(records):
    events = [record['data'] for record in records if record['kind'] == 'presentation_event']
    begins = [record['data'] for record in records if record['kind'] == 'presentation_begin']
    ends = [record['data'] for record in records if record['kind'] == 'presentation_end']
    libraries = [record['data'] for record in records if record['kind'] == 'presentation_library']
    require(len(libraries) == len(begins) == len(ends) == 1,
            'Expected exactly one library, begin and end record')
    require(libraries[0]['abi_version'] == begins[0]['abi_version'] == 1
            and begins[0]['status'] == ends[0]['status'] == 0,
            'Presentation observer did not start/end successfully')
    require(begins[0]['clock'] == 'CACurrentMediaTime host seconds; not UNIX time'
            and begins[0]['boundary'] == 'MTLDrawable.presentedTime from addPresentedHandler',
            'Unsupported or missing presentation clock/boundary metadata')
    summary = ends[0]['summary']
    require(summary['abi_version'] == 1 and summary['struct_size'] == 408
            and summary['status'] == 0 and summary['session_state'] == 4,
            'Invalid presentation summary identity or lifecycle')
    require(type(summary['flags']) is int and summary['flags'] == 12
            and type(summary['capabilities']) is int and summary['capabilities'] == 3,
            'Hook changed, drain failed or required AppKit/layer observation unavailable')
    boundaries = [summary[name] for name in (
        'begin_host_time', 'cutoff_host_time', 'drain_end_host_time', 'snapshot_host_time')]
    require(all(finite(value, positive=True) for value in boundaries)
            and boundaries == sorted(boundaries) and boundaries[0] < boundaries[1],
            'Invalid host-domain measurement boundaries')
    begin_time, cutoff_time, drain_time, snapshot_time = boundaries
    sleep_evidence = validate_sleep_clocks(records, begin_time, drain_time)
    counters = summary['counters']
    for name in FATAL_COUNTERS + tuple(COUNTED_EVENTS) + (
            'emitted_events', 'acquisition_null_returns', 'successful_acquisitions',
            'handler_registration_attempts', 'positive_presentations', 'successful_binds', 'missing_binds',
            'zero_presented_timestamps', 'texture_reuses_before_callback'):
        require(type(counters[name]) is int and counters[name] >= 0,
                'Invalid presentation counter: ' + name)
    for name in FATAL_COUNTERS:
        require(counters[name] == 0, 'Nonzero presentation counter: ' + name)
    for name in ('last_event_sequence', 'last_acquisition_sequence', 'unresolved_acquisitions',
                 'unresolved_unbound_acquisitions', 'unbound_acquisitions'):
        require(type(summary[name]) is int and summary[name] >= 0,
                'Invalid presentation summary counter: ' + name)
    require(summary['unresolved_acquisitions'] == summary['unresolved_unbound_acquisitions'] == 0,
            'Unresolved acquired drawables remain')
    require(events and summary['last_event_sequence'] == counters['emitted_events'] == len(events),
            'Presentation event accounting does not reconcile')
    counts = dict.fromkeys(range(1, 14), 0)
    cutoff_sequence = None
    for sequence, event in enumerate(events, 1):
        require(all(type(event[name]) is int and event[name] >= 0 for name in INTEGER_FIELDS)
                and type(event['status']) is int and event['status'] == 0
                and event['abi_version'] == 1 and event['struct_size'] == 192
                and event['event_sequence'] == sequence and 1 <= event['kind'] <= 13,
                'Missing, duplicate or malformed presentation event')
        require(all(finite(event[name]) for name in TIME_FIELDS + ('drawable_width', 'drawable_height')),
                'Invalid nonfinite or negative presentation timestamp/size')
        require(begin_time <= event['host_time'] <= drain_time <= snapshot_time,
                'Presentation event outside measurement lifecycle')
        require(not event['flags'] & ~((1 << 18) - 1)
                and not event['flags'] & (2 | 128 | 256 | 512 | 2048 | 16384 | 65536 | 131072)
                and event['kind'] not in (10, 12), 'Presentation fault or unresolved event')
        if event['kind'] == 11:
            require(cutoff_sequence is None and event['host_time'] == cutoff_time,
                    'Invalid measurement cutoff')
            cutoff_sequence = sequence
        require(bool(event['flags'] & 1) == (cutoff_sequence is not None),
                'Presentation event has inconsistent cutoff flags')
        if cutoff_sequence is None:
            require(event['host_time'] <= cutoff_time, 'Pre-cutoff event exceeds measurement boundary')
        else:
            require(event['kind'] in (4, 5, 6, 8, 11, 13),
                    'New measurement work or platform evidence after cutoff')
        counts[event['kind']] += 1
    require(counts[1] == counts[11] == counts[13] == 1
            and events[0]['kind'] == 1 and events[0]['host_time'] == begin_time
            and events[-1]['kind'] == 13 and events[-1]['host_time'] == drain_time,
            'Invalid presentation begin/cutoff/end ordering')
    for name, kind in COUNTED_EVENTS.items():
        require(counters[name] == counts[kind], 'Persisted event count disagrees with ' + name)
    require(summary['last_acquisition_sequence'] == counts[3], 'Invalid final acquisition sequence')
    frames = {}
    for record in records:
        if record['kind'] != 'frame':
            continue
        frame = record['data']
        identifier = frame['frame_id']
        require(type(identifier) is int and identifier > 0 and identifier not in frames,
                'Invalid or duplicate immutable draw observation')
        require(type(frame['state']['state_id']) is int and frame['state']['state_id'] >= 0,
                'Invalid immutable draw state identity')
        frames[identifier] = frame
    contexts, acquisitions, current_textures = {}, {}, {}
    callbacks, bound_frames, platform = [], set(), []
    configuration = None
    dimensions = None
    resizes = []
    superseded_before_callback = 0
    for event in events:
        kind = event['kind']
        if kind == 9:
            require(event['platform_notification'] in range(1, 8), 'Unknown platform notification')
            platform.append(event)
        elif kind == 2:
            context = event['draw_context_sequence']
            require(context == len(contexts) + 1 and event['flags'] & 4,
                    'Invalid draw-context sequence or validity')
            frame = frames.get(event['frame_id'])
            require(frame is not None and frame['state']['state_id'] == event['state_id'],
                    'Draw context lacks its exact immutable observation')
            contexts[context] = event
        elif kind == 3:
            sequence = event['acquisition_sequence']
            require(sequence == len(acquisitions) + 1 and event['layer_lifetime_id'] > 0
                    and event['flags'] & 4 and event['host_time'] == event['acquisition_enter_host_time']
                    and event['acquisition_call_host_time'] == event['acquisition_return_host_time'] == 0,
                    'Invalid acquisition entry or sequence')
            context = contexts[event['draw_context_sequence']]
            same_fields(event, context, ('frame_id', 'state_id', 'draw_context_sequence'))
            require(context['host_time'] <= event['host_time'], 'Acquisition predates its draw context')
            acquisitions[sequence] = {'entry': event}
        elif kind in (4, 5, 6, 7, 8):
            acquisition = acquisitions[event['acquisition_sequence']]
            same_fields(event, acquisition['entry'], IDENTITY_FIELDS)
            if kind == 4:
                require('returned' not in acquisition and event['host_time'] == event['acquisition_return_host_time']
                        and event['acquisition_enter_host_time'] <= event['acquisition_call_host_time']
                        <= event['acquisition_return_host_time'], 'Invalid or duplicate acquisition return')
                acquisition['returned'] = event
                require(all(event[name] == 0 for name in BIND_FIELDS), 'Acquisition returned already bound')
                if event['flags'] & 32768:
                    require(not event['flags'] & 8 and event['texture_identity'] == event['drawable_id'] == 0,
                            'Null acquisition has a drawable identity')
                    acquisition['null'] = True
                    continue
                require(event['flags'] & 8 and event['flags'] & 16 and event['texture_identity'] > 0
                        and event['drawable_width'] > 0 and event['drawable_height'] > 0,
                        'Drawable identity or layer settings unavailable')
                require(all(event[name] in (0, 1) for name in CONFIGURATION_FIELDS if name != 'maximum_drawable_count')
                        and event['maximum_drawable_count'] in (2, 3), 'Invalid layer configuration')
                settings = tuple(event[name] for name in CONFIGURATION_FIELDS)
                require(configuration is None or settings == configuration,
                        'Layer presentation configuration changed during measurement')
                configuration = settings
                size = (event['drawable_width'], event['drawable_height'])
                if dimensions is not None and dimensions != size:
                    resizes.append({'frame_id': event['frame_id'], 'size': list(size)})
                dimensions = size
                require(event['texture_identity'] not in current_textures,
                        'Texture reused without its previous lifetime being superseded')
                current_textures[event['texture_identity']] = event['acquisition_sequence']
                continue
            require(not acquisition.get('null'), 'Null acquisition has registration, bind or callback')
            returned = acquisition['returned']
            same_fields(event, returned, RETURN_FIELDS)
            require(event['flags'] & 8 and event['flags'] & 16
                    and event['host_time'] >= returned['host_time'], 'Invalid post-return drawable metadata')
            if kind == 5:
                require('registered' not in acquisition and 'callback' not in acquisition
                        and all(event[name] == 0 for name in BIND_FIELDS),
                        'Duplicate or out-of-order handler registration')
                acquisition['registered'] = event
            elif kind == 7:
                require('registered' in acquisition and 'bound' not in acquisition
                        and 'callback' not in acquisition and 'superseded' not in acquisition
                        and current_textures.get(event['texture_identity']) == event['acquisition_sequence']
                        and event['flags'] & 32
                        and event['bind_count'] == 1 and event['bound_frame_id'] == event['frame_id']
                        and event['bound_state_id'] == event['state_id']
                        and event['frame_id'] not in bound_frames
                        and event['host_time'] >= acquisition['registered']['host_time'],
                        'Inconsistent or out-of-order scene-texture binding')
                acquisition['bound'] = event
                bound_frames.add(event['frame_id'])
            elif kind == 6:
                require('registered' in acquisition and 'callback' not in acquisition
                        and event['callback_delivery_host_time'] == event['host_time']
                        and acquisition['registered']['host_time'] <= event['host_time']
                        and bool(event['flags'] & 1024) == ('superseded' in acquisition),
                        'Invalid or duplicate host-domain presented callback')
                if event['presented_host_time'] > 0:
                    require(acquisition['registered']['host_time'] <= event['presented_host_time']
                            <= event['host_time'], 'Noncausal positive presentation timestamp')
                if 'bound' in acquisition:
                    same_fields(event, acquisition['bound'], BIND_FIELDS)
                    require(event['flags'] & 32 and acquisition['bound']['host_time'] <= event['host_time']
                            and (event['presented_host_time'] == 0
                                 or acquisition['bound']['host_time'] <= event['presented_host_time']),
                            'Presentation predates its binding')
                else:
                    require(all(event[name] == 0 for name in BIND_FIELDS) and not event['flags'] & 32
                            and event['flags'] & 64, 'Unbound presentation has inconsistent binding evidence')
                    require(frames[event['frame_id']]['state']['value']['tick'] is None,
                            'Ready presentation lacks its own exact acquisition binding')
                acquisition['callback'] = event
                callbacks.append(event)
            elif kind == 8:
                require('registered' in acquisition and 'superseded' not in acquisition
                        and event['flags'] & 1024
                        and current_textures.get(event['texture_identity']) == event['acquisition_sequence'],
                        'Unowned, duplicate or premature texture supersession')
                if 'bound' in acquisition:
                    same_fields(event, acquisition['bound'], BIND_FIELDS)
                else:
                    require(all(event[name] == 0 for name in BIND_FIELDS),
                            'Unbound texture supersession contains a binding')
                if 'callback' in acquisition:
                    same_fields(event, acquisition['callback'], (
                        'presented_host_time', 'callback_delivery_host_time'))
                    require(event['host_time'] >= acquisition['callback']['host_time'],
                            'Completed texture supersession predates callback')
                else:
                    require(event['presented_host_time'] == event['callback_delivery_host_time'] == 0,
                            'Supersession contains an unobserved future callback')
                    superseded_before_callback += 1
                acquisition['superseded'] = event
                del current_textures[event['texture_identity']]
    require(all('returned' in acquisition and (acquisition.get('null')
                or ('registered' in acquisition and 'callback' in acquisition))
                for acquisition in acquisitions.values()), 'Incomplete per-acquisition evidence')
    successful = sum(not acquisition.get('null', False) for acquisition in acquisitions.values())
    positive = [event for event in callbacks if event['presented_host_time'] > 0]
    zero_timestamp = [event for event in callbacks if event['presented_host_time'] == 0]
    require(successful == counters['successful_acquisitions'] == counters['handler_registration_attempts']
            == counters['handler_registrations'] == counters['presented_callbacks']
            and counters['positive_presentations'] == len(positive)
            and counters['zero_presented_timestamps'] == len(zero_timestamp)
            and successful == len(positive) + len(zero_timestamp)
            and counters['texture_reuses_before_callback'] == superseded_before_callback
            and len(acquisitions) == counters['acquisition_returns']
            and counters['acquisition_null_returns'] + successful == len(acquisitions),
            'Acquisition, registration and presentation totals do not reconcile')
    require(counters['successful_binds'] == counters['bind_calls'] == len(bound_frames)
            and summary['unbound_acquisitions'] == counters['missing_binds'] == successful - len(bound_frames),
            'Bindings and explicitly unbound acquisitions do not reconcile')
    expected_ready = {identifier for identifier, frame in frames.items()
                      if frame['state']['value']['tick'] is not None}
    ready_callbacks = [event for event in callbacks if event['frame_id'] in expected_ready]
    require(expected_ready == {event['frame_id'] for event in ready_callbacks}
            and expected_ready <= bound_frames, 'Ready draw/presentation coverage is incomplete')
    require(len({event['layer_lifetime_id'] for event in callbacks}) == 1,
            'Expected exactly one observed Metal layer')
    timeline = sorted(platform, key=lambda event: (event['host_time'], event['event_sequence']))
    require(timeline and timeline[0]['platform_notification'] in (6, 7)
            and sum(event['platform_notification'] in (6, 7) for event in timeline) == 1,
            'Missing or duplicate initial foreground observation')
    ordered = sorted((event for event in positive if event['presented_host_time'] <= cutoff_time),
                     key=lambda event: event['presented_host_time'])
    measured_ready = [event for event in ordered if event['frame_id'] in expected_ready]
    require(len(measured_ready) >= 2, 'Expected multiple ready presentations before cutoff')
    ready_begin = measured_ready[0]['presented_host_time']
    ready_end = measured_ready[-1]['presented_host_time']
    ready_acquisition_begin = min(acquisitions[event['acquisition_sequence']]['entry']['host_time']
                                  for event in ready_callbacks)
    startup_display_changes = [event['host_time'] for event in timeline
                               if event['platform_notification'] == 3 and event['host_time'] < ready_begin]
    require(not any(event['platform_notification'] in (4, 5)
                    or (event['platform_notification'] == 3 and event['host_time'] >= ready_begin)
                    for event in timeline), 'Display configuration or sleep changed during ready measurement')
    def foreground_at(timestamp):
        preceding = [event for event in timeline if event['host_time'] <= timestamp
                     and event['platform_notification'] in (1, 2, 6, 7)]
        return bool(preceding) and preceding[-1]['platform_notification'] in (1, 6)
    require(all(foreground_at(event['presented_host_time']) for event in measured_ready),
            'Ready presentation occurred without foreground evidence at presentation time')
    for event in ready_callbacks:
        if event['presented_host_time'] != 0:
            continue
        start = acquisitions[event['acquisition_sequence']]['entry']['host_time']
        stop = min(event['host_time'], cutoff_time)
        require(foreground_at(start) and not any(start < item['host_time'] <= stop
                and item['platform_notification'] in (2, 7) for item in timeline),
                'Zero-timestamp ready acquisition intersects an inactive period')
    intervals, ready_intervals, startup_interrupted_intervals = [], [], 0
    for earlier, later in zip(ordered, ordered[1:]):
        start, end = earlier['presented_host_time'], later['presented_host_time']
        require(start < end, 'Duplicate or non-increasing presentation timestamps')
        interval = (end - start) * 1_000_000
        if any(start < timestamp <= end for timestamp in startup_display_changes):
            startup_interrupted_intervals += 1
        else:
            intervals.append(interval)
    for earlier, later in zip(measured_ready, measured_ready[1:]):
        start, end = earlier['presented_host_time'], later['presented_host_time']
        require(not any(start < event['host_time'] <= end
                        and event['platform_notification'] in (2, 7) for event in timeline),
                'Ready presentation interval crosses an inactive period')
        ready_intervals.append((end - start) * 1_000_000)
    return {'acquired_drawables': len(callbacks), 'positive_presentations': len(positive),
            'zero_timestamp_loading_drawables': [event['frame_id'] for event in zero_timestamp
                                                 if event['frame_id'] not in expected_ready],
            'zero_timestamp_ready_drawables': [event['frame_id'] for event in zero_timestamp
                                               if event['frame_id'] in expected_ready],
            'zero_timestamp_ready_acquisitions': [event['acquisition_sequence'] for event in zero_timestamp
                                                  if event['frame_id'] in expected_ready],
            'ready_acquisitions': len(ready_callbacks),
            'ready_presentations': sum(event['presented_host_time'] > 0 for event in ready_callbacks),
            'measured_presentations': len(ordered), 'measured_ready_presentations': len(measured_ready),
            'post_cutoff_presentations': len(positive) - len(ordered),
            'texture_reuses_before_callback': superseded_before_callback,
            'lifecycle_host_seconds': {'begin': begin_time, 'cutoff': cutoff_time,
                                       'drain_end': drain_time, 'snapshot': snapshot_time},
            'sleep_clock': sleep_evidence, 'startup_display_change_host_times': startup_display_changes,
            'startup_interrupted_intervals': startup_interrupted_intervals,
            'layer_configuration': dict(zip(CONFIGURATION_FIELDS, configuration)),
            'ready_presented_duration_seconds': ready_end - ready_begin,
            'ready_measurement_duration_seconds': cutoff_time - ready_acquisition_begin,
            'ready_leading_gap_us': (ready_begin - ready_acquisition_begin) * 1_000_000,
            'ready_terminal_gap_us': (cutoff_time - ready_end) * 1_000_000,
            'drawable_resizes': resizes, 'intervals': intervals, 'ready_intervals': ready_intervals}


def qualify_presentations(summary, minimum_ready_seconds=900):
    errors = []
    try:
        require(finite(minimum_ready_seconds, positive=True) and minimum_ready_seconds >= 900,
                'Long qualification requires at least 900 ready seconds')
        require(summary.get('valid') is True, 'Valid complete presentation evidence is required')
        duration = summary['ready_presented_duration_seconds']
        require(finite(duration) and duration >= minimum_ready_seconds,
                'Insufficient sustained ready presentation duration')
        for name in ('ready_leading_gap_us', 'ready_terminal_gap_us'):
            require(finite(summary[name]) and summary[name] <= 250_000,
                    'Censored ready presentation gap exceeds 250 ms: ' + name)
        intervals = summary['ready_presented_interval_us']
        require(type(intervals['samples']) is int and intervals['samples'] >= 2,
                'Multiple ready presentation intervals are required')
        require(finite(intervals['p95'], positive=True) and intervals['p95'] <= 20_000
                and finite(intervals['p99'], positive=True) and intervals['p99'] <= 33_400,
                'Ready presentation interval percentile target exceeded')
        require(finite(intervals['worst'], positive=True) and intervals['worst'] <= 250_000,
                'Ready presentation interval exceeds 250 ms')
    except (KeyError, TypeError, ValueError) as failure:
        errors.append(str(failure))
    return {'qualified': not errors, 'errors': errors, 'minimum_ready_seconds': minimum_ready_seconds,
            'censored_gap_limit_us': 250_000, 'p95_limit_us': 20_000, 'p99_limit_us': 33_400,
            'maximum_interval_us': 250_000,
            'scope': 'Long ready presentation coverage and pacing, including pending/modal time; '
                     'leading and terminal gaps are censored bounds, not completed intervals'}


def summarize_presentations(records, required=False):
    available = any(record.get('kind', '').startswith('presentation_') for record in records)
    if not available and not required:
        return {'available': False, 'valid': None, 'errors': [], 'scope': 'No presentation observer requested'}
    errors, evidence = [], {}
    try:
        evidence = validate_evidence(records)
    except (KeyError, IndexError, TypeError, ValueError) as failure:
        errors.append(str(failure))
    def distribution(values):
        if not values or errors:
            return {'samples': 0}
        ordered = sorted(values)
        return {'samples': len(ordered), 'p50': ordered[math.ceil(len(ordered)*.5)-1],
                'p95': ordered[math.ceil(len(ordered)*.95)-1],
                'p99': ordered[math.ceil(len(ordered)*.99)-1], 'worst': ordered[-1]}
    intervals, ready_intervals = evidence.pop('intervals', []), evidence.pop('ready_intervals', [])
    return {'available': available, 'valid': not errors, 'errors': errors, **evidence,
            'presented_interval_us': distribution(intervals),
            'ready_presented_interval_us': distribution(ready_intervals),
            'scope': 'MTLDrawable API-reported presentation timestamps before measurement cutoff, '
                     'including capture overhead; ready includes pending and modal draws; '
                     'zero-timestamp callbacks are accounted unmeasured outcomes, not presentation credit; '
                     'positive-to-positive gaps retain intervening unmeasured outcomes; '
                     'loading intervals crossing disclosed startup display changes are segmented; '
                     'post-cutoff presentations reconciled but not timed; not physical scanout FPS, GPU time or photons'}
