"""SOURCE-ONLY, mockable native-post to API-presentation diagnostics.

No readers, native imports, process controls, or driver enablement live here.
The frozen driver deliberately rejects presentation records. A future combined
reader must be reviewed and bind source, binary, helper receipt, PID lifetime,
window, and journal before this schema can describe a runtime observation.
New Swift source does not validate an existing compiled helper.

This is a deliberately strict subset: one stable, foreground, single-display
window/layer; every draw bound once and positively presented; no null/unbound
startup draws, pre-callback texture reuse, resizing, or post-cutoff presentations.
Host samples retain their serialized order. The native texture-supersession
event may precede the replacement return carrying its earlier captured time;
only that adjacent, identity-bound causal exception is admitted. Other native
clock regressions remain unsupported. Independent application spans retain their
raw enqueue order while UI and per-span causal relationships are validated.
Recycled drawable/texture identifiers require an explicitly completed old
acquisition lifetime, never an identifier-only join. Clock boundary samples must
use the same named QuartzCore API as the post brackets. Mach absolute/continuous
brackets only diagnose sleep/discontinuity; they never convert input timestamps.
Their <=1ms sampling resolution cannot exclude sub-resolution discontinuities.

Bounds cover the actual post call, not receipt delivery or callback delivery.
They use exact rational arithmetic on API-reported numbers, then round outward
to integral microseconds. No gesture subtraction, inferred Unix offset, or
DispatchTime subtraction is used. API accuracy/clock rate, cross-process clock
contract, collector completeness, physical visibility, scanout and photons still
need runtime proof; none is qualified here. Semantic freshness is restricted to
the frozen paused-camera ArrowRight predicate, not arbitrary gameplay effects.

Reference ranges below are inclusive, hashed over original UTF-8 line bytes
including newlines. Only field/accounting concepts and conservative lifecycle
checks are adapted; the root analyzer and its tests are never imported/executed.
"""

import json
import math
from fractions import Fraction


CLOCK_API = 'QuartzCore.CACurrentMediaTime'
CLOCK_DESCRIPTION = 'CACurrentMediaTime host seconds; not UNIX time'
BOUNDARY = 'MTLDrawable.presentedTime from addPresentedHandler'
SOURCE_CONTRACT = 'sims-window-presentation-source-only-v1'
CONTROL_ORIGIN = 'receipt-to-semantic-effect-draw control-effect predicate'
P95_LIMIT_US = 100_000
MAXIMUM_LIMIT_US = 250_000
MAX_RECORDS = 100_000
MAX_CONTROLS = 256
MAX_PAYLOAD_BYTES = 16 * 1024 * 1024
MAX_BRACKET_US = 1000
SOURCE_PINS = {
    'archive': '72f6a9c889a6db932f4702685b698ca024e4f9056bd80580d0c20fc91b6d5da2',
    'inventory_2046': '348f5e5ef472cc86535dedbf4e7220a76dfdb83a5e96ae8af680e336e0f7e3bd',
    'frozen/scripts/window_session_macos.swift':
        '91c11d9886c9e3e49f511a38125fb33271243cae7acc33c3ad5a4f1e7a0a2d25',
    'frozen/scripts/window_evidence.py':
        '6865b8f16dc7d5b4fbd36a94ccfe1fad93bacf7b12c5845a32cbdb63c98c155f',
    'frozen/scripts/window_session.py':
        '66dbb2f6e8792b9300f5c85093266025950f0b8ff62b367c3e3396d600d5f890',
    'root/scripts/window_presentation.py':
        '1169cc0fbc52c9dccba49c8ada3527212208eb2e704016ecce0bcb536c4d4ac8',
    'root/scripts/test_window_presentation.py':
        'db5793340e5252381ed1b484c6f22e893d468fcfb30961d20a7857a8af15c5a4',
    'root/crates/sims-app/native/window_present_macos.h':
        'af8bfedc623d8f6cea9c0aed31dbc1d69a624822296c75f016f3c5133e9c0fdb',
    'root/crates/sims-app/native/window_present_macos.m':
        'fdb8c118c964c5725168182ac06d5343371136581d1757717b109c6450d10c5c',
}
RANGE_PINS = {
    'root/scripts/window_presentation.py:5-38':
        '19a1b1f9ca134df65fa5a11771da8613083951d34030ffea427d3961be3db01d',
    'root/scripts/window_presentation.py:59-89':
        '2aebe253f71bb7238929c573f8e58560452e57b35bc7e5a895ee0729073f1a68',
    'root/scripts/window_presentation.py:135-181':
        '0b8a4b11fa80c4b0e1880ce178f4cf13cb1934b595d93b2b91aca57dee4878ed',
    'root/scripts/window_presentation.py:208-281':
        '75c949f459b999356e821b3df498b70e94daf9fd5751bd1bafbf3e109e792e4b',
    'root/crates/sims-app/native/window_present_macos.h:108-212':
        'f90d16b864bd64299aacf8b8a0c9b5c74e3a7701c03aafdd927073e79004bb67',
    'root/crates/sims-app/native/window_present_macos.m:206-275':
        '888efe23e38154e9312b85b02286ca9e6abfc1da799aee526f73e01f56970183',
    'root/crates/sims-app/native/window_present_macos.m:430-457':
        '7d6cb832e32b458fa3cc7e84c3cbcfe6103e8bf7d6b8965b40f3013d6cd4d782',
    'root/crates/sims-app/native/window_present_macos.m:517-558':
        '81d19270f22da5fba620ee7bdf80675de07c2286e4ea75c9cb9141df80685921',
    'root/crates/sims-app/native/window_present_macos.m:755-837':
        '7f19a3e980757806cc6ccc8307c4fe9830a97f261bf2e946f4b6dac1b21c8bc7',
    'frozen/scripts/window_evidence.py:17-29':
        'b90735c512587e62036b37256d8d8065d6b2b52188f9f1a31875b84a6fe28a77',
    'frozen/scripts/window_evidence.py:293-349':
        'e93c42d91fd16ddfce789419a7297f5fcfdd92e21319832dd14c00a855235be7',
    'frozen/scripts/window_session.py:93-134':
        '07988c0d4827cf60405d2478377a5822f6d829dc8d4e26ccbc09770c47c276b2',
    'frozen/scripts/window_session.py:153-176':
        'f3ce1aa01e23aeafdb75a799356cafe4ca16056a75919d05a744ef334e824fa2',
    'frozen/scripts/window_session_macos.swift:134-144':
        '9d8f0566db979bdd040133b0cfc1816266e32774ed36e103677b7ec6f4d84d9e',
}

EVENT_INTEGERS = (
    'abi_version', 'struct_size', 'kind', 'flags', 'event_sequence',
    'acquisition_sequence', 'layer_lifetime_id', 'drawable_id', 'texture_identity',
    'draw_context_sequence', 'frame_id', 'state_id', 'bound_frame_id',
    'bound_state_id', 'bind_count', 'display_sync_enabled', 'maximum_drawable_count',
    'allows_next_drawable_timeout', 'presents_with_transaction', 'platform_notification',
)
EVENT_TIMES = (
    'host_time', 'acquisition_enter_host_time', 'acquisition_call_host_time',
    'acquisition_return_host_time', 'presented_host_time', 'callback_delivery_host_time',
)
EVENT_REALS = EVENT_TIMES + ('drawable_width', 'drawable_height')
CONTEXT = ('draw_context_sequence', 'frame_id', 'state_id')
IDENTITY = CONTEXT + ('acquisition_sequence', 'layer_lifetime_id', 'acquisition_enter_host_time')
SETTINGS = ('display_sync_enabled', 'maximum_drawable_count',
            'allows_next_drawable_timeout', 'presents_with_transaction')
RETURN = IDENTITY + ('drawable_id', 'texture_identity', 'acquisition_call_host_time',
                     'acquisition_return_host_time', 'drawable_width', 'drawable_height') + SETTINGS
BIND = ('bind_count', 'bound_frame_id', 'bound_state_id')
COUNTERS = (
    'acquisition_entries', 'acquisition_returns', 'acquisition_null_returns',
    'successful_acquisitions', 'acquisition_exceptions', 'handler_registration_attempts',
    'handler_registrations', 'handler_registration_failures', 'presented_callbacks',
    'positive_presentations', 'zero_presented_timestamps', 'invalid_presented_timestamps',
    'duplicate_callbacks', 'callback_lookup_conflicts', 'bind_calls', 'successful_binds',
    'duplicate_binds', 'missing_binds', 'bind_lookup_conflicts', 'binding_conflicts',
    'missing_draw_contexts', 'texture_lifetime_reuses', 'texture_reuses_before_callback',
    'texture_identity_conflicts', 'bookkeeping_overflows', 'overflow_untraced_calls',
    'outstanding_acquisitions', 'active_acquisition_calls', 'callbacks_awaited',
    'emitted_events', 'draw_context_calls', 'platform_notifications', 'observation_errors',
    'unresolved_events', 'late_callbacks', 'late_binds', 'post_callback_bind_attempts',
    'reentrant_acquisition_calls',
)
COUNTED = {'acquisition_entries': 3, 'acquisition_returns': 4,
           'handler_registrations': 5, 'presented_callbacks': 6, 'bind_calls': 7,
           'texture_lifetime_reuses': 8, 'platform_notifications': 9, 'draw_context_calls': 2}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def integer(value, minimum=0, maximum=(1 << 64) - 1):
    return type(value) is int and minimum <= value <= maximum


def finite(value):
    return type(value) in (int, float) and math.isfinite(value) and value >= 0


def rational(value):
    require(finite(value), 'Nonfinite or negative clock/sample')
    return Fraction(value)


def ceil(value):
    return -(-value.numerator // value.denominator)


def validate_tree(value, depth=0, budget=None):
    if budget is None:
        budget = [2_000_000]
    budget[0] -= 1
    require(depth <= 64 and budget[0] >= 0, 'Evidence nesting/size limit exceeded')
    if type(value) is dict:
        require(all(type(name) is str for name in value), 'Non-string object key')
        for name, item in value.items():
            require(not (name == 'error' or name.endswith(('_error', '_errors'))) or not item,
                    'Explicit application/observer error in evidence')
            validate_tree(item, depth + 1, budget)
    elif type(value) is list:
        for item in value:
            validate_tree(item, depth + 1, budget)
    elif type(value) is float:
        require(math.isfinite(value), 'Nonfinite evidence number')
    elif type(value) is int:
        require(value.bit_length() <= 64, 'Unbounded evidence integer')
    else:
        require(value is None or type(value) in (str, bool), 'Non-JSON evidence type')


def parse_json(payload):
    require(type(payload) in (str, bytes) and len(payload) <= MAX_PAYLOAD_BYTES,
            'Oversized or non-text JSON payload')

    def pairs(items):
        result = {}
        for name, value in items:
            require(name not in result, 'Duplicate JSON field: ' + name)
            result[name] = value
        return result

    def number(token):
        value = float(token)
        require(math.isfinite(value), 'Nonfinite JSON exponent/number')
        require(value != 0 or not any(character in '123456789' for character in token.split('e')[0].split('E')[0]),
                'Underflowed JSON exponent/number')
        return value

    def constant(token):
        raise ValueError('Nonfinite JSON constant: ' + token)

    try:
        result = json.loads(payload, object_pairs_hook=pairs, parse_float=number,
                            parse_constant=constant)
        validate_tree(result)
        return result
    except (RecursionError, UnicodeError) as error:
        raise ValueError('Invalid JSON encoding/nesting') from error


def statistics(values):
    require(type(values) is list and all(integer(value) for value in values), 'Invalid bound samples')
    ordered = sorted(values)
    if not ordered:
        return {'samples': 0}
    return {'samples': len(ordered), 'p50': ordered[(len(ordered) * 50 - 1) // 100],
            'p95': ordered[(len(ordered) * 95 - 1) // 100], 'worst': ordered[-1]}


def fixed_limits(values):
    distribution = statistics(values)
    return {'upper_bound_us': distribution, 'p95_limit_us': P95_LIMIT_US,
            'maximum_limit_us': MAXIMUM_LIMIT_US,
            'within_fixed_limits': bool(values and distribution['p95'] <= P95_LIMIT_US
                                        and distribution['worst'] <= MAXIMUM_LIMIT_US)}


def latency_bounds(presented, before, after):
    presented, before, after = map(rational, (presented, before, after))
    require(0 < before <= after <= presented, 'Presentation before/inside native post bracket')
    lower, upper = (presented - after) * 1_000_000, (presented - before) * 1_000_000
    return {'lower_us': lower.numerator // lower.denominator, 'upper_us': ceil(upper),
            'post_bracket_us': ceil((after - before) * 1_000_000)}


def same(actual, expected, fields):
    require(all(actual[name] == expected[name] for name in fields),
            'Conflicting acquisition lifetime, binding, timing or configuration')


def zeros(event, allowed):
    fields = set(EVENT_INTEGERS + EVENT_REALS) - set(allowed) - {
        'abi_version', 'struct_size', 'kind', 'flags', 'event_sequence', 'host_time'}
    require(all(event[name] == 0 for name in fields), 'Unexpected fields for presentation event kind')


def app_journal(records):
    require(type(records) is list and 1 <= len(records) <= MAX_RECORDS, 'Missing/bounded journal required')
    frames, inputs, states, spans, span_ids = {}, [], {}, {}, set()
    previous_ui_elapsed = maximum_elapsed = previous_draw = previous_received = sequence = 0
    require(records[0]['kind'] == 'session' and records[-1]['kind'] == 'session_end',
            'Incomplete session journal')
    session = records[0]['data']
    require(type(session['schema']) is int and session['schema'] == 1 and integer(session['pid'], 1),
            'Invalid application session identity')
    stamp = session['unix_ns']
    require(type(stamp) is str and stamp.isascii() and stamp.isdecimal() and 0 < len(stamp) <= 20,
            'Invalid session timestamp')
    session_time = int(stamp)
    require(integer(session_time, 1), 'Invalid session timestamp range')
    for index, record in enumerate(records):
        require(type(record) is dict and type(record['data']) is dict
                and integer(record['elapsed_us'])
                and type(record['dropped_records']) is int and record['dropped_records'] == 0,
                'Dropped or malformed journal record')
        maximum_elapsed = max(maximum_elapsed, record['elapsed_us'])
        kind, data = record['kind'], record['data']
        require(type(kind) is str and not kind.endswith('error'), 'Application/observer error')
        if kind in ('input', 'frame'):
            require(record['elapsed_us'] >= previous_ui_elapsed, 'Regressing UI journal record')
            previous_ui_elapsed = record['elapsed_us']
        if kind == 'session':
            require(index == 0, 'Duplicate application session')
        elif kind == 'session_end':
            require(index == len(records) - 1 and type(data['dropped_records']) is int
                    and data['dropped_records'] == 0 and record['elapsed_us'] >= maximum_elapsed,
                    'Late records, terminal clock regression or final drops')
        elif kind == 'input':
            require(integer(data['input_sequence'], 1) and data['input_sequence'] == sequence + 1
                    and integer(data['fence']) and integer(data['received_unix_ns'], 1)
                    and data['received_unix_ns'] >= max(previous_received, session_time),
                    'Reordered, stale or malformed input receipt')
            sequence, previous_received = data['input_sequence'], data['received_unix_ns']
            require(data['input'] is None or type(data['input']) is dict, 'Malformed input identity')
            inputs.append(data)
        elif kind == 'frame':
            require(type(data['frame_id']) is int and data['frame_id'] == len(frames) + 1
                    and integer(data['fence']) and type(data['input_sequence']) is int
                    and data['input_sequence'] == sequence and integer(data['draw_started_unix_ns'], 1)
                    and data['draw_started_unix_ns'] >= max(previous_draw, session_time)
                    and integer(data['widget_draw_cpu_us']) and 'cursor' in data
                    and (data['draw_interval_us'] is None or integer(data['draw_interval_us'])),
                    'Malformed, stale or unacknowledged immutable draw')
            previous_draw = data['draw_started_unix_ns']
            state = data['state']
            require(type(state) is dict and integer(state['state_id'], 1)
                    and type(state['value']) is dict and type(state['diagnostics']) is dict,
                    'Invalid immutable state schema')
            value = state['value']
            require('tick' in value and (value['tick'] is None or integer(value['tick']))
                    and all(type(value[name]) is bool for name in (
                        'playing', 'pending', 'clock_pending', 'presentation_children_pending', 'modal'))
                    and not any(value.get(name) for name in ('error', 'menu_error'))
                    and not (value.get('shell') or {}).get('error'), 'Malformed/error application state')
            capabilities = value['observation_capabilities']
            require(type(capabilities) is dict and set(capabilities) == {
                'action_terminals', 'route_terminals', 'transport_readiness'}
                and all(item is False for item in capabilities.values())
                and all(value[name] is None for name in ('terminals', 'hud_normal_ready', 'hud_transport_ready')),
                'Unsupported capability/readiness inference')
            require(state['state_id'] not in states or states[state['state_id']] == value,
                    'Immutable state identity reused with a different value')
            states[state['state_id']] = value
            frames[data['frame_id']] = data
        elif kind == 'span_begin':
            require(integer(data['span_id'], 1) and data['span_id'] not in span_ids
                    and type(data['name']) is str, 'Duplicate/malformed span')
            span_ids.add(data['span_id'])
            spans[data['span_id']] = (data['name'], record['elapsed_us'])
        elif kind == 'span_end':
            require(integer(data['span_id'], 1) and data['span_id'] in spans
                    and spans[data['span_id']][0] == data['name']
                    and spans[data['span_id']][1] <= record['elapsed_us']
                    and integer(data['wall_us']), 'Unmatched or regressing span')
            spans.pop(data['span_id'])
        elif kind in ('scene_prepare', 'scene_encode'):
            require(integer(data['frame_id']) and integer(data['cpu_us']), 'Malformed scene timing')
        else:
            require(kind in ('presentation_library', 'presentation_begin', 'presentation_event',
                             'presentation_sleep_clock', 'presentation_end'), 'Unknown journal record kind')
    require(frames and not spans, 'Missing draws or incomplete journal spans')
    return session, frames, inputs


def sleep_clocks(clocks, summary):
    require(len(clocks) == 2 and [clock['phase'] for clock in clocks] == ['begin', 'end'],
            'Missing ordered sleep/discontinuity evidence')
    offsets, quartz_offsets, samples, brackets = [], [], [], []
    for clock in clocks:
        require(type(clock['schema']) is int and clock['schema'] == 1
                and clock['clock'] == 'mach timebase ticks; absolute stops during sleep; continuous includes sleep'
                and clock['clock_api'] == CLOCK_API, 'Incompatible sleep/QuartzCore clock tags')
        sample = clock['sample']
        require(all(integer(sample[name], 1) for name in (
            'absolute_before', 'absolute_after', 'continuous', 'timebase_numer', 'timebase_denom')),
            'Malformed sleep-clock sample')
        require(sample['absolute_before'] <= sample['absolute_after'], 'Regressing absolute clock bracket')
        scale = Fraction(sample['timebase_numer'], sample['timebase_denom'] * 1_000_000_000)
        before, after = map(rational, (clock['quartz_before_seconds'], clock['quartz_after_seconds']))
        require(0 < before <= after and (after - before) * 1_000_000 <= MAX_BRACKET_US
                and (sample['absolute_after'] - sample['absolute_before']) * scale * 1_000_000 <= MAX_BRACKET_US,
                'Ambiguous sleep/QuartzCore sample bracket')
        offsets.append(((sample['continuous'] - sample['absolute_after']) * scale,
                        (sample['continuous'] - sample['absolute_before']) * scale))
        quartz_offsets.append((before - sample['absolute_after'] * scale,
                               after - sample['absolute_before'] * scale))
        samples.append(sample)
        brackets.append((before, after))
    first, last = samples
    same(first, last, ('timebase_numer', 'timebase_denom'))
    require(first['absolute_after'] <= last['absolute_before']
            and first['continuous'] <= last['continuous']
            and brackets[0][1] <= rational(summary['begin_host_time'])
            and brackets[1][0] >= rational(summary['snapshot_host_time']),
            'Clock evidence does not cover complete measurement/drain')
    for intervals in (offsets, quartz_offsets):
        require(max(item[0] for item in intervals) <= min(item[1] for item in intervals),
                'Sleep or clock regression/discontinuity detected')
    return {'sampling_limit_us': MAX_BRACKET_US, 'sub_resolution_discontinuities_excluded': False}


def presentations(records, frames):
    grouped = {}
    positions = {}
    for position, record in enumerate(records):
        grouped.setdefault(record['kind'], []).append(record['data'])
        positions.setdefault(record['kind'], []).append(position)
    for name in ('presentation_library', 'presentation_begin', 'presentation_end'):
        require(len(grouped.get(name, [])) == 1, 'Missing/duplicate presentation lifecycle metadata')
    library, begin, end = (grouped[name][0] for name in (
        'presentation_library', 'presentation_begin', 'presentation_end'))
    require(library['source_contract'] == SOURCE_CONTRACT and library['source_only'] is True
            and library['source_sha256'] == SOURCE_PINS['root/crates/sims-app/native/window_present_macos.m']
            and all(type(library[name]) is int and library[name] == expected for name, expected in (
                ('abi_version', 1), ('event_size', 192), ('counters_size', 304), ('summary_size', 408))),
            'Unreviewed/unbound presentation source schema (not a binary receipt)')
    require(type(begin['abi_version']) is int and begin['abi_version'] == 1
            and type(begin['status']) is int and begin['status'] == 0
            and type(end['status']) is int and end['status'] == 0
            and begin['clock'] == CLOCK_DESCRIPTION and begin['clock_api'] == CLOCK_API
            and begin['boundary'] == BOUNDARY, 'Incompatible clock/API presentation boundary')
    events = grouped.get('presentation_event', [])
    summary = end['summary']
    require(type(summary['abi_version']) is int and summary['abi_version'] == 1
            and type(summary['struct_size']) is int and summary['struct_size'] == 408
            and type(summary['status']) is int and summary['status'] == 0
            and type(summary['session_state']) is int and summary['session_state'] == 4,
            'Invalid ABI summary or incomplete native drain')
    require(type(summary['capabilities']) is int and summary['capabilities'] == 3
            and type(summary['flags']) is int and summary['flags'] == 12,
            'Hook replaced, drain timeout or missing observation capabilities')
    boundaries = [rational(summary[name]) for name in (
        'begin_host_time', 'cutoff_host_time', 'drain_end_host_time', 'snapshot_host_time')]
    begin_time, cutoff_time, drain_time, snapshot_time = boundaries
    require(0 < begin_time < cutoff_time <= drain_time <= snapshot_time, 'Invalid clock lifecycle boundaries')
    require(len(events) >= 3 and positions['presentation_library'][0] < positions['presentation_event'][0]
            < positions['presentation_event'][1] < positions['presentation_begin'][0]
            < positions['presentation_event'][2] <= positions['presentation_event'][-1]
            < positions['presentation_end'][0], 'Presentation records outside begin/end')
    clocks = grouped.get('presentation_sleep_clock', [])
    require(len(clocks) == 2 and positions['presentation_sleep_clock'][0] < positions['presentation_event'][0]
            and positions['presentation_event'][-1] < positions['presentation_sleep_clock'][-1]
            < positions['presentation_end'][0], 'Sleep-clock record ordering mismatch')
    clock_diagnostic = sleep_clocks(clocks, summary)
    counters = summary['counters']
    require(type(counters) is dict and set(counters) == set(COUNTERS)
            and all(integer(counters[name]) for name in COUNTERS), 'Malformed ABI counters')
    require(all(integer(summary[name]) for name in (
        'last_event_sequence', 'last_acquisition_sequence', 'unresolved_acquisitions',
        'unbound_acquisitions', 'unresolved_unbound_acquisitions'))
        and summary['unresolved_acquisitions'] == summary['unbound_acquisitions']
        == summary['unresolved_unbound_acquisitions'] == 0, 'Unresolved/unbound acquired lifetimes')
    counts, contexts, acquisitions, bound_frames, context_frames = {}, {}, {}, {}, set()
    current_textures, current_drawables, layers = {}, {}, set()
    configuration = None
    previous_host = begin_time
    previous_event = None
    cutoff_seen = False
    for sequence, event in enumerate(events, 1):
        require(type(event) is dict and set(event) == set(EVENT_INTEGERS + EVENT_REALS + ('status',))
                and all(integer(event[name], maximum=(1 << (32 if name in (
                    'abi_version', 'struct_size', 'kind', 'flags', 'display_sync_enabled',
                    'maximum_drawable_count', 'allows_next_drawable_timeout',
                    'presents_with_transaction', 'platform_notification') else 64)) - 1)
                        for name in EVENT_INTEGERS)
                and all(finite(event[name]) for name in EVENT_REALS)
                and type(event['status']) is int and event['status'] == 0
                and event['abi_version'] == 1 and event['struct_size'] == 192
                and event['event_sequence'] == sequence, 'Malformed/nonfinite ABI event or sequence')
        kind, host = event['kind'], rational(event['host_time'])
        replacement_return = (kind == 4 and previous_event is not None and previous_event['kind'] == 8
                              and previous_event['texture_identity'] == event['texture_identity']
                              and previous_event['acquisition_sequence'] != event['acquisition_sequence']
                              and event['acquisition_enter_host_time'] <= host
                              <= rational(previous_event['host_time']))
        require(kind in (1, 2, 3, 4, 5, 6, 7, 8, 9, 11, 13)
                and begin_time <= host <= drain_time
                and (previous_host <= host or replacement_return),
                'Fault/unresolved event or clock regression')
        previous_host = max(previous_host, host)
        previous_event = event
        if kind == 11:
            require(not cutoff_seen and host == cutoff_time, 'Duplicate/inconsistent cutoff')
            cutoff_seen = True
        require(bool(event['flags'] & 1) == cutoff_seen
                and (host <= cutoff_time if not cutoff_seen else kind in (6, 8, 11, 13)),
                'Late/new work or inconsistent cutoff flags')
        flags = event['flags'] & ~1
        counts[kind] = counts.get(kind, 0) + 1
        if kind in (1, 9, 11, 13):
            zeros(event, ('platform_notification',) if kind == 9 else ())
            require(flags == 0 and (kind != 9 or event['platform_notification'] == 6),
                    'Focus, sleep, display or unknown platform ambiguity')
        elif kind == 2:
            zeros(event, CONTEXT)
            require(flags == 4 and event['draw_context_sequence'] == len(contexts) + 1
                    and event['frame_id'] in frames
                    and frames[event['frame_id']]['state']['state_id'] == event['state_id'],
                    'Draw context lacks exact immutable frame/state identity')
            require(event['frame_id'] not in context_frames,
                    'Immutable draw identity reused by another context')
            context_frames.add(event['frame_id'])
            contexts[event['draw_context_sequence']] = event
        elif kind == 3:
            zeros(event, IDENTITY)
            require(flags == 4 | 4096 and event['acquisition_sequence'] == len(acquisitions) + 1
                    and event['layer_lifetime_id'] > 0 and host == rational(event['acquisition_enter_host_time']),
                    'Malformed/duplicate acquisition entry')
            context = contexts[event['draw_context_sequence']]
            same(event, context, CONTEXT)
            require(context['host_time'] <= event['host_time'], 'Acquisition predates its context')
            acquisitions[event['acquisition_sequence']] = {'entry': event, 'context': context}
            layers.add(event['layer_lifetime_id'])
        else:
            acquisition = acquisitions[event['acquisition_sequence']]
            same(event, acquisition['entry'], IDENTITY)
            require(event['platform_notification'] == 0, 'Spurious platform data on acquisition')
            if kind == 4:
                require('returned' not in acquisition and flags == 4 | 8 | 16 | 64 | 4096
                        and host == rational(event['acquisition_return_host_time'])
                        and event['acquisition_enter_host_time'] <= event['acquisition_call_host_time']
                        <= event['acquisition_return_host_time'] and event['texture_identity'] > 0
                        and event['drawable_width'] > 0 and event['drawable_height'] > 0
                        and all(event[name] == 0 for name in BIND + (
                            'presented_host_time', 'callback_delivery_host_time')),
                        'Missing/null/duplicate or malformed drawable return')
                require(event['display_sync_enabled'] in (0, 1)
                        and event['maximum_drawable_count'] in (2, 3)
                        and event['allows_next_drawable_timeout'] in (0, 1)
                        and event['presents_with_transaction'] in (0, 1), 'Invalid layer settings')
                settings = tuple(event[name] for name in SETTINGS + ('drawable_width', 'drawable_height'))
                require(configuration is None or settings == configuration, 'Display/layer/size changed')
                configuration = settings
                drawable = (event['layer_lifetime_id'], event['drawable_id'])
                require(event['texture_identity'] not in current_textures and drawable not in current_drawables,
                        'Reused identifier without completed superseded acquisition lifetime')
                current_textures[event['texture_identity']] = event['acquisition_sequence']
                current_drawables[drawable] = event['acquisition_sequence']
                acquisition['returned'] = event
                continue
            same(event, acquisition['returned'], RETURN)
            require(event['host_time'] >= acquisition['returned']['host_time'], 'Post-return clock regression')
            if kind == 5:
                require('registered' not in acquisition and flags == 4 | 8 | 16 | 64 | 4096
                        and all(event[name] == 0 for name in BIND + (
                            'presented_host_time', 'callback_delivery_host_time')),
                        'Duplicate/out-of-order handler registration')
                acquisition['registered'] = event
            elif kind == 7:
                require('registered' in acquisition and 'bound' not in acquisition
                        and 'callback' not in acquisition and flags == 4 | 8 | 16 | 32
                        and event['bind_count'] == 1 and event['bound_frame_id'] == event['frame_id']
                        and event['bound_state_id'] == event['state_id']
                        and event['frame_id'] not in bound_frames
                        and acquisition['registered']['host_time'] <= event['host_time']
                        and event['presented_host_time'] == event['callback_delivery_host_time'] == 0,
                        'Missing/conflicting/duplicate or late frame binding')
                acquisition['bound'] = event
                bound_frames[event['frame_id']] = acquisition
            elif kind == 6:
                require('bound' in acquisition and 'callback' not in acquisition
                        and flags == 4 | 8 | 16 | 32
                        and acquisition['bound']['host_time'] <= event['presented_host_time']
                        <= min(event['host_time'], summary['cutoff_host_time'])
                        and event['presented_host_time'] > 0
                        and event['callback_delivery_host_time'] == event['host_time'],
                        'Unbound/duplicate/zero/noncausal or post-cutoff presentation')
                same(event, acquisition['bound'], BIND)
                acquisition['callback'] = event
            elif kind == 8:
                replacement = events[sequence] if sequence < len(events) else None
                require(type(replacement) is dict and replacement['kind'] == 4
                        and replacement['texture_identity'] == event['texture_identity']
                        and replacement['acquisition_sequence'] != event['acquisition_sequence']
                        and replacement['acquisition_sequence'] in acquisitions
                        and 'returned' not in acquisitions[replacement['acquisition_sequence']]
                        and replacement['acquisition_enter_host_time']
                        <= replacement['acquisition_return_host_time'] <= event['host_time'],
                        'Texture supersession lacks its native replacement-return sequence')
                require('callback' in acquisition and 'superseded' not in acquisition
                        and flags == 4 | 8 | 16 | 32 | 1024
                        and acquisition['callback']['host_time'] <= event['host_time']
                        and current_textures.get(event['texture_identity']) == event['acquisition_sequence'],
                        'Unresolved/reused or premature drawable lifetime supersession')
                same(event, acquisition['callback'], BIND + ('presented_host_time', 'callback_delivery_host_time'))
                acquisition['superseded'] = event
                del current_textures[event['texture_identity']]
                del current_drawables[(event['layer_lifetime_id'], event['drawable_id'])]
    require(counts.get(1) == counts.get(11) == counts.get(13) == counts.get(9) == 1
            and events[0]['kind'] == 1 and rational(events[0]['host_time']) == begin_time
            and events[-1]['kind'] == 13 and rational(events[-1]['host_time']) == drain_time,
            'Incomplete begin/foreground/cutoff/end lifecycle')
    require(events[1]['kind'] == 9, 'Foreground identity not established before measurement work')
    require(len(layers) == 1 and set(bound_frames) == set(frames)
            and len(acquisitions) == len(contexts) == len(frames)
            and all('callback' in acquisition for acquisition in acquisitions.values()),
            'Missing frame coverage or incomplete acquisition drain')
    expected = dict.fromkeys(COUNTERS, 0)
    expected.update({name: counts.get(kind, 0) for name, kind in COUNTED.items()})
    expected.update(successful_acquisitions=len(acquisitions),
                    handler_registration_attempts=len(acquisitions), positive_presentations=len(acquisitions),
                    successful_binds=len(acquisitions), emitted_events=len(events))
    require(counters == expected and summary['last_event_sequence'] == len(events)
            and summary['last_acquisition_sequence'] == len(acquisitions),
            'Drops/errors/late work or counter/event accounting mismatch')
    return bound_frames, summary, clock_diagnostic


def binding(value, pid):
    require(type(value) is dict and set(value) == {
                'schema', 'pid', 'window_id', 'executable', 'kernel_identity', 'frontmost',
                'on_screen', 'bounds', 'display_scope', 'displays'}
            and value['schema'] == 'sims-window-binding-v2'
            and type(value['pid']) is int and value['pid'] == pid and integer(value['window_id'], 1)
            and 0 < value['pid'] <= 2**31 - 1 and value['window_id'] <= 2**32 - 1
            and type(value['executable']) is str and value['executable'].startswith('/')
            and '\x00' not in value['executable']
            and value['frontmost'] is True and value['on_screen'] is True,
            'Wrong PID/window lifetime, focus or binding schema')
    identity = value['kernel_identity']
    require(type(identity) is dict and set(identity) == {
                'source', 'pid', 'start_tvsec', 'start_tvusec', 'executable'}
            and identity['source'] == 'proc_pidinfo(PROC_PIDTBSDINFO)+proc_pidpath'
            and type(identity['pid']) is int and identity['pid'] == pid
            and integer(identity['start_tvsec'], 1) and identity['start_tvsec'] <= 2**64 - 1
            and integer(identity['start_tvusec']) and identity['start_tvusec'] < 1_000_000
            and identity['executable'] == value['executable'], 'Wrong or malformed kernel process lifetime')
    bounds = value['bounds']
    require(type(bounds) is list and len(bounds) == 4
            and all(type(item) in (int, float) and math.isfinite(item) for item in bounds)
            and bounds[2] > 100 and bounds[3] > 100, 'Invalid owned-window bounds')
    require(value['display_scope'] == 'Current CoreGraphics mode and NSScreen maximum; not instantaneous refresh'
            and type(value['displays']) is list and len(value['displays']) == 1, 'Ambiguous display scope')
    display = value['displays'][0]
    require(type(display) is dict and set(display) == {
            'display_id', 'mode_id', 'width', 'height', 'pixel_width', 'pixel_height',
            'nominal_refresh_hz', 'maximum_frames_per_second', 'builtin', 'active', 'asleep'}
        and integer(display['display_id'], 1, 2**32 - 1)
        and integer(display['mode_id'], 0, 2**32 - 1)
        and all(integer(display[name], 1, 2**63 - 1) for name in (
            'width', 'height', 'pixel_width', 'pixel_height'))
        and integer(display['maximum_frames_per_second'], 0, 2**63 - 1)
        and finite(display['nominal_refresh_hz'])
        and (display['nominal_refresh_hz'] > 0 or display['maximum_frames_per_second'] > 0)
        and type(display['builtin']) is bool and display['active'] is True and display['asleep'] is False,
        'Unavailable, changed or asleep display')


def usable_camera(frame):
    value = frame['state']['value']
    camera = value.get('camera')
    return (integer(value['tick']) and all(value[name] is False for name in (
        'playing', 'pending', 'clock_pending', 'presentation_children_pending', 'modal'))
        and type(camera) is dict and type(camera.get('center')) is list and len(camera['center']) == 2
        and all(type(item) in (int, float) and math.isfinite(item) for item in camera['center']))


def controls_join(controls, session, frames, inputs, bound_frames, summary):
    require(type(controls) is list and 1 <= len(controls) <= MAX_CONTROLS, 'Missing/bounded control results required')
    samples, all_receipts, previous_binding = [], [], None
    previous_finished = previous_post = 0
    used_frames = set()
    for index, item in enumerate(controls):
        control, native = item['control_effect'], item['native_result']
        started, finished, fence = item['started_unix_ns'], item['finished_unix_ns'], item['fence']
        next_started = controls[index + 1]['started_unix_ns'] if index + 1 < len(controls) else None
        require(integer(started, 1) and integer(finished, 1) and integer(fence, 1)
                and previous_finished <= started < finished
                and (next_started is None or finished <= next_started)
                and item['next_started_unix_ns'] == next_started, 'Overlapping/reordered command windows')
        previous_finished = finished
        require(type(control) is dict and set(control) == {
            'complete', 'origin', 'source_origin', 'latency_us', 'p95_limit_us', 'maximum_limit_us',
            'within_fixed_limits', 'baseline_frame_id', 'receipts', 'effect', 'closing_frame_id', 'scope'}
                and control['complete'] is True and control['origin'] == CONTROL_ORIGIN
                and control['source_origin'] == 'window_controls.summarize_controls'
                and integer(control['baseline_frame_id'], 1) and integer(control['closing_frame_id'], 1)
                and type(control['scope']) is str and type(control['within_fixed_limits']) is bool
                and type(control['p95_limit_us']) is int and control['p95_limit_us'] == P95_LIMIT_US
                and type(control['maximum_limit_us']) is int and control['maximum_limit_us'] == MAXIMUM_LIMIT_US,
                'Only an already complete fresh ordered semantic control result may join')
        binding(item['binding_before'], session['pid'])
        binding(item['binding_after'], session['pid'])
        binding(native['binding'], session['pid'])
        require(item['binding_before'] == item['binding_after'] == native['binding']
                and (previous_binding is None or previous_binding == native['binding']),
                'Binding/focus/display/PID lifetime changed across input')
        previous_binding = native['binding']
        require(not native.get('error') and not native.get('release_error')
                and native['operation'] == 'key' and type(native['code']) is int and native['code'] == 124
                and type(native['press_ms']) is int and native['press_ms'] == 20 and native['released'] is True,
                'Wrong or failed bounded native key/release')
        posts = native['posts']
        require(type(posts) is list and len(posts) == 2, 'Missing key-press/release brackets')
        previous_unix = previous_host = 0
        for post, down in zip(posts, (True, False)):
            require(type(post) is dict and post['down'] is down and post['clock_api'] == CLOCK_API
                    and all(integer(post[name], 1) for name in (
                        'unix_before_ns', 'unix_after_ns', 'host_before_ns', 'host_after_ns'))
                    and started <= post['unix_before_ns'] <= post['unix_after_ns'] <= finished
                    and previous_unix <= post['unix_before_ns']
                    and previous_host <= post['host_before_ns'] <= post['host_after_ns'],
                    'Invalid native ownership/post clock metadata or ordering')
            before, after = map(rational, (post['quartz_before_seconds'], post['quartz_after_seconds']))
            require(rational(summary['begin_host_time']) <= before <= after
                    <= rational(summary['cutoff_host_time'])
                    and (after - before) * 1_000_000 <= MAX_BRACKET_US
                    and rational(previous_post) <= before, 'Ambiguous/regressing QuartzCore post interval')
            previous_post = post['quartz_after_seconds']
            previous_unix, previous_host = post['unix_after_ns'], post['host_after_ns']
        require((rational(posts[1]['quartz_after_seconds']) - rational(posts[0]['quartz_before_seconds']))
                * 1_000_000 <= 1_000_000, 'Unbounded gesture interval')
        receipts = control['receipts']
        require(type(receipts) is list and len(receipts) == 2, 'Missing ordered press/release receipts')
        window_inputs = [receipt for receipt in inputs if receipt['input'] is not None
                         and receipt['received_unix_ns'] >= started
                         and (next_started is None or receipt['received_unix_ns'] < next_started)]
        require(receipts == window_inputs, 'Unmatched/malformed/extra native receipt group')
        for receipt, kind, post in zip(receipts, ('key_pressed', 'key_released'), posts):
            require(all(integer(receipt[name], 1) for name in ('input_sequence', 'fence', 'received_unix_ns'))
                    and receipt['input'] == {'kind': kind, 'physical_key': 'Code(ArrowRight)'}
                    and receipt['fence'] == fence and receipt['received_unix_ns'] >= post['unix_before_ns'],
                    'Stale/reordered/wrong-key or wrong-fence receipt')
        require(receipts[0]['input_sequence'] < receipts[1]['input_sequence'], 'Reordered receipt sequence')
        all_receipts.extend(receipts)
        baseline = frames[control['baseline_frame_id']]
        candidates = [frame for frame in frames.values() if frame['fence'] == fence
                      and frame['draw_started_unix_ns'] < started]
        require(candidates and baseline == candidates[-1] and usable_camera(baseline)
                and baseline['input_sequence'] < receipts[0]['input_sequence'], 'Stale/missing fresh baseline')
        window_frames = [frame for frame in frames.values() if frame['draw_started_unix_ns'] >= started
                         and (next_started is None or frame['draw_started_unix_ns'] < next_started)]
        require(all(frame['fence'] == fence and usable_camera(frame) for frame in window_frames),
                'Wrong-fence or ambiguous pending/modal semantic draw')
        changed = [frame for frame in window_frames
                   if frame['input_sequence'] >= receipts[0]['input_sequence']
                   and frame['draw_started_unix_ns'] >= receipts[0]['received_unix_ns']
                   and frame['state']['value']['camera']['center'] != baseline['state']['value']['camera']['center']]
        effect = control['effect']
        require(type(effect) is dict and changed and type(effect['frame_id']) is int
                and effect['frame_id'] == changed[0]['frame_id'] and effect['frame_id'] not in used_frames,
                'Malformed/unmatched effect or unrelated/reused semantic frame')
        frame = changed[0]
        receipt_latency = (frame['draw_started_unix_ns'] - receipts[0]['received_unix_ns']) / 1000
        require(type(effect['state_id']) is int and effect['state_id'] == frame['state']['state_id']
                and type(effect['unix_ns']) is int and effect['unix_ns'] == frame['draw_started_unix_ns']
                and finite(effect['latency_us']) and effect['latency_us'] == receipt_latency,
                'Conflicting effect frame/state/time/receipt latency')
        metric = control['latency_us']
        require(type(metric) is dict and set(metric) == {'samples', 'p50', 'p95', 'p99', 'worst'}
                and type(metric['samples']) is int and metric['samples'] == 1
                and all(finite(metric[name]) and metric[name] == receipt_latency
                        for name in ('p50', 'p95', 'p99', 'worst'))
                and control['within_fixed_limits'] is (receipt_latency <= P95_LIMIT_US
                                                     and receipt_latency <= MAXIMUM_LIMIT_US),
                'Malformed/conflicting preserved receipt-to-semantic-draw metric')
        closings = [draw for draw in window_frames if draw['input_sequence'] >= receipts[-1]['input_sequence']
                    and draw['draw_started_unix_ns'] >= receipts[-1]['received_unix_ns']
                    and draw['draw_started_unix_ns'] > finished]
        require(closings and control['closing_frame_id'] == closings[-1]['frame_id'],
                'Missing fresh post-command closing draw')
        acquisition = bound_frames[effect['frame_id']]
        callback = acquisition['callback']
        same(callback, acquisition['bound'], BIND)
        require(callback['bound_frame_id'] == frame['frame_id']
                and callback['bound_state_id'] == frame['state']['state_id']
                and acquisition['context']['host_time'] >= posts[0]['quartz_after_seconds'],
                'Bound presentation/context predates or does not identify native input effect')
        bounds = latency_bounds(callback['presented_host_time'], posts[0]['quartz_before_seconds'],
                                posts[0]['quartz_after_seconds'])
        samples.append({'control_index': index, 'frame_id': frame['frame_id'],
                        'state_id': frame['state']['state_id'],
                        'acquisition_sequence': callback['acquisition_sequence'],
                        'layer_lifetime_id': callback['layer_lifetime_id'], 'drawable_id': callback['drawable_id'],
                        'texture_identity': callback['texture_identity'], **bounds,
                        'receipt_to_semantic_draw_us': receipt_latency})
        used_frames.add(frame['frame_id'])
    require(all_receipts == [receipt for receipt in inputs if receipt['input'] is not None],
            'Unassociated native receipts outside bounded commands')
    return samples


def analyze(records, controls):
    """Return a diagnostic only; malformed/unsupported evidence fails closed."""
    result = {
        'schema': SOURCE_CONTRACT, 'diagnostic_only': True, 'valid': False, 'errors': [],
        'acceptance_qualified': False, 'native_qualified': False, 'build_qualified': False,
        'startup_qualified': False, 'performance_qualified': False, 'alpha_qualified': False,
        'presentation_qualified': False, 'photon_qualified': False,
        'source_binary_receipt_integration_reviewed': False,
        'driver_enabled': False, 'runtime_proof': False, 'visual_proof': False,
        'worker_quiescence_proven': False, 'full_readiness': None,
        'eligible_seconds': None, 'civil_days': None, 'visibility': None, 'input_to_photon': None,
        'clock_api': CLOCK_API, 'boundary': BOUNDARY, 'samples': [],
        'api_presentation_latency': fixed_limits([]),
        'scope': 'Source-only API-reported post-call bracket bounds; not acceptance, photons, '
                 'visibility, eligible time, civil days or worker quiescence. Requires future '
                 'combined-reader/source/binary/receipt review and Swift recompile/binding.',
    }
    try:
        validate_tree([records, controls])
        session, frames, inputs = app_journal(records)
        bound_frames, summary, clock_diagnostic = presentations(records, frames)
        samples = controls_join(controls, session, frames, inputs, bound_frames, summary)
        result.update(valid=True, samples=samples, clock_diagnostic=clock_diagnostic,
                      api_presentation_latency=fixed_limits([sample['upper_us'] for sample in samples]),
                      receipt_to_semantic_draw_us=[sample['receipt_to_semantic_draw_us'] for sample in samples])
    except (ValueError, KeyError, TypeError, IndexError, OverflowError, RecursionError, AttributeError) as error:
        result['errors'] = [str(error) or type(error).__name__]
    return result
