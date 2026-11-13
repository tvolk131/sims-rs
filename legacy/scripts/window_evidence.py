"""Fail-closed analysis of a single diagnostic key's application receipts and draws."""
import json
import math


MAC_KEYS = {124: 'ArrowRight', 36: 'Enter'}
OPERATORS = {
    'eq': lambda actual, expected: actual == expected,
    'ne': lambda actual, expected: actual != expected,
    'gt': lambda actual, expected: actual > expected,
    'ge': lambda actual, expected: actual >= expected,
    'lt': lambda actual, expected: actual < expected,
    'contains': lambda actual, expected: expected in actual,
    'not_contains': lambda actual, expected: expected not in actual,
    'one_of': lambda actual, expected: actual in expected,
}
METRIC_ORIGIN = 'receipt-to-semantic-effect-draw control-effect predicate'
RESPONSE_ORIGIN = 'receipt-to-offered-response acknowledgment diagnostic'
TUTORIAL_MODE = 'offered-tutorial-entry'
TUTORIAL_ACTION = {'kind': 'key', 'code': 36, 'press_ms': 20}
TUTORIAL_CONTROLS = [{'step': 'tutorial-opening-enter',
                      'acknowledgment': 'offered-tutorial-response-v1'}]
P95_LIMIT_US = 100_000
MAXIMUM_LIMIT_US = 250_000
MAX_JOURNAL_BYTES = 64 * 1024 * 1024
QUALIFICATIONS = {
    'native_qualified': False, 'startup_qualified': False,
    'performance_qualified': False, 'alpha_qualified': False,
    'tutorial_qualified': False, 'completion_qualified': False, 'window_qualified': False,
    'presentation_qualified': False, 'eligibility_qualified': False, 'game_day_qualified': False,
}
CAPABILITIES = {'action_terminals': False, 'route_terminals': False,
                'transport_readiness': False}
CAMERA_CONDITIONS = [{'path': 'camera.center', 'op': 'ne', 'ref': 'baseline.camera.center'}]


def require(condition, message):
    if not condition:
        raise ValueError(message)


def integer(value, minimum=0):
    return type(value) is int and value >= minimum


def finite(value, minimum=0):
    return type(value) in (int, float) and math.isfinite(value) and value >= minimum


def unique_object(pairs):
    result = {}
    for name, value in pairs:
        require(name not in result, 'Duplicate JSON field: ' + name)
        result[name] = value
    return result


def parse_json(payload):
    def invalid_constant(value):
        raise ValueError('Nonfinite JSON constant: ' + value)

    def finite_float(value):
        parsed = float(value)
        require(math.isfinite(parsed), 'Nonfinite JSON number: ' + value)
        return parsed

    try:
        return json.loads(payload, object_pairs_hook=unique_object,
                          parse_constant=invalid_constant, parse_float=finite_float)
    except (RecursionError, UnicodeError) as error:
        raise ValueError('Invalid JSON encoding or nesting') from error


def lookup(value, path):
    for component in path.split('.'):
        value = value[int(component)] if isinstance(value, list) else value[component]
    return value


def matches(state, conditions, remembered):
    for condition in conditions:
        try:
            if condition['op'] == 'any_match':
                records = lookup(state, condition['path'])
                if not isinstance(records, list) or not any(
                        isinstance(record, dict) and matches(record, condition['value'], remembered)
                        for record in records):
                    return False
                continue
            expected = (lookup(remembered, condition['ref'])
                        if 'ref' in condition else condition.get('value'))
            if not OPERATORS[condition['op']](lookup(state, condition['path']), expected):
                return False
        except (KeyError, IndexError, TypeError, ValueError):
            return False
    return True


def distribution(values):
    samples = [value for value in values if value is not None]
    require(all(finite(value) for value in samples), 'Invalid distribution sample')
    samples.sort()
    if not samples:
        return {'samples': 0}
    def percentile(fraction):
        return samples[max(0, math.ceil(len(samples) * fraction) - 1)]
    return {'samples': len(samples), 'p50': percentile(.5), 'p95': percentile(.95),
            'p99': percentile(.99), 'worst': samples[-1]}


def metric_summary(values):
    samples = distribution(values)
    return {'latency_us': samples, 'p95_limit_us': P95_LIMIT_US,
            'maximum_limit_us': MAXIMUM_LIMIT_US,
            'within_fixed_limits': bool(samples['samples'] and samples['p95'] <= P95_LIMIT_US
                                        and samples['worst'] <= MAXIMUM_LIMIT_US)}


def expected_receipts(action):
    require(isinstance(action, dict) and action in (
        {'kind': 'key', 'code': 124, 'press_ms': 20}, TUTORIAL_ACTION)
        and type(action.get('code')) is int and type(action.get('press_ms')) is int,
        'Only explicitly supported ArrowRight or Enter/20ms diagnostics are supported')
    physical = 'Code({})'.format(MAC_KEYS[action['code']])
    return [{'kind': 'key_pressed', 'physical_key': physical},
            {'kind': 'key_released', 'physical_key': physical}]


def receipt_matches(actual, expected):
    if not isinstance(actual, dict) or actual.get('kind') != expected['kind']:
        return False
    if 'point' in expected:
        point = actual.get('point')
        return (isinstance(point, list) and len(point) == 2
                and all(type(value) in (int, float) and math.isfinite(value)
                        and abs(value - wanted) <= 1 for value, wanted in zip(point, expected['point'])))
    return all(actual.get(name) == value for name, value in expected.items())


def validate_specification(plan, specification, plan_sha256):
    if isinstance(plan, dict) and plan.get('mode') == TUTORIAL_MODE:
        require(set(plan) == {'schema', 'mode', 'timeout_seconds', 'household', 'steps'}
                and type(plan['schema']) is int and plan['schema'] == 2
                and finite(plan['timeout_seconds'], 10) and plan['timeout_seconds'] <= 300
                and plan['household'] == 'newbie'
                and plan['steps'] == [{'name': 'tutorial-opening-enter', 'input': TUTORIAL_ACTION}],
                'Unsupported explicit offered-tutorial-entry plan')
        expected_receipts(plan['steps'][0]['input'])
        require(isinstance(specification, dict)
                and set(specification) == {'schema', 'plan_sha256', 'controls'}
                and type(specification['schema']) is int and specification['schema'] == 2
                and specification['plan_sha256'] == plan_sha256
                and specification['controls'] == TUTORIAL_CONTROLS,
                'Tutorial acknowledgment must bind the explicit plan; no synthetic endpoints')
        return
    require(isinstance(plan, dict) and set(plan) == {'schema', 'timeout_seconds', 'household', 'steps'}
            and type(plan['schema']) is int and plan['schema'] == 1
            and finite(plan['timeout_seconds'], 10) and plan['timeout_seconds'] <= 300
            and plan['household'] == 'newbie', 'Unsupported diagnostic plan')
    require(plan['steps'] == [{'name': 'camera-right', 'input': {
        'kind': 'key', 'code': 124, 'press_ms': 20}}], 'Unsupported diagnostic steps')
    require(type(plan['steps'][0]['input']['code']) is int
            and type(plan['steps'][0]['input']['press_ms']) is int, 'Invalid native input scalar types')
    require(isinstance(specification, dict) and set(specification) == {'schema', 'plan_sha256', 'controls'}
            and type(specification['schema']) is int and specification['schema'] == 1
            and specification['plan_sha256'] == plan_sha256
            and specification['controls'] == [{'step': 'camera-right', 'conditions': CAMERA_CONDITIONS}],
            'Endpoints must bind the exact plan and camera-center change predicate')


def usable_camera(value):
    camera = value.get('camera')
    return (integer(value.get('tick')) and value.get('playing') is False
            and all(value.get(name) is False for name in
                    ('pending', 'clock_pending', 'presentation_children_pending', 'modal'))
            and isinstance(camera, dict) and isinstance(camera.get('center'), list)
            and len(camera['center']) == 2
            and all(type(number) in (int, float) and math.isfinite(number) for number in camera['center']))


def validate_displays(snapshot):
    require(snapshot.get('display_scope') ==
            'Current CoreGraphics mode and NSScreen maximum; not instantaneous refresh',
            'Unsupported display scope')
    displays = snapshot.get('displays')
    require(isinstance(displays, list) and 1 <= len(displays) <= 8, 'Invalid target display list')
    identifiers = set()
    for display in displays:
        require(isinstance(display, dict) and set(display) == {
                'display_id', 'mode_id', 'width', 'height', 'pixel_width', 'pixel_height',
                'nominal_refresh_hz', 'maximum_frames_per_second', 'builtin', 'active', 'asleep'}
                and all(integer(display.get(name), 1) for name in
                ('display_id', 'width', 'height', 'pixel_width', 'pixel_height'))
                and display['display_id'] not in identifiers, 'Invalid or duplicate display identity/geometry')
        identifiers.add(display['display_id'])
        require(integer(display.get('mode_id')) and integer(display.get('maximum_frames_per_second'))
                and finite(display.get('nominal_refresh_hz'))
                and (display['nominal_refresh_hz'] > 0 or display['maximum_frames_per_second'] > 0)
                and type(display.get('builtin')) is bool and display.get('active') is True
                and display.get('asleep') is False, 'Unavailable, asleep or malformed display configuration')
    require([display['display_id'] for display in displays] == sorted(identifiers), 'Display order changed')


def validate_dialog(value, answer=False):
    fields = {'id', 'caller', 'source', 'origin', 'status'}
    fields |= {'result', 'tick'} if answer else {
        'offered_results', 'enter_result', 'escape_result', 'space_result'}
    require(isinstance(value, dict) and set(value) == fields
            and integer(value.get('id'), 1) and value['id'] <= 2**64 - 1
            and type(value.get('caller')) is int and -32768 <= value['caller'] <= 32767
            and isinstance(value.get('source'), str) and bool(value['source'])
            and '\0' not in value['source']
            and integer(value.get('status')) and value['status'] <= 5,
            'Unsupported no-prose dialog observation')
    origin = value['origin']
    require(isinstance(origin, dict) and set(origin) == {'source', 'bhav', 'instruction'}
            and isinstance(origin['source'], str) and bool(origin['source']) and '\0' not in origin['source']
            and integer(origin['bhav']) and origin['bhav'] <= 65535
            and integer(origin['instruction']) and origin['instruction'] <= 65535,
            'Unsupported observed dialog origin')
    if answer:
        require(integer(value['result'], 2) and value['result'] <= 4 and integer(value['tick']),
                'Malformed dialog answer observation')
    else:
        require(isinstance(value['offered_results'], list)
                and all(integer(result, 2) and result <= 4 for result in value['offered_results'])
                and all(value[name] is None or integer(value[name], 2) and value[name] <= 4
                        for name in ('enter_result', 'escape_result', 'space_result')),
                'Malformed offered dialog results')


def paused_response_state(value, tick=None):
    require(integer(value.get('tick')) and (tick is None or value['tick'] == tick)
            and value.get('playing') is False
            and all(value.get(name) is False for name in
                    ('pending', 'clock_pending', 'presentation_children_pending')),
            'Offered response encountered next blocker, playing state or tick change')
    require('dialog' in value and 'dialog_answer' in value,
            'Unsupported: published dialog observations are absent')


def offered_opening(value):
    paused_response_state(value)
    require(value.get('modal') is True and value['dialog'] is not None
            and value['dialog_answer'] is None, 'Unsupported: current unanswered opening modal absent')
    dialog = value['dialog']
    validate_dialog(dialog)
    resource = dialog['source'].replace('\\', '/').rsplit('/', 1)[-1].rsplit('::', 1)[-1]
    require(dialog['status'] == 1 and dialog['source'] == dialog['origin']['source']
            and resource == 'Tutorial.iff' and dialog['origin']['bhav'] == 4097
            and dialog['origin']['instruction'] == 2 and dialog['offered_results'] == [2]
            and dialog['enter_result'] == 2,
            'Unsupported: modal is not the actually offered Tutorial opening')
    return {name: dialog[name] for name in ('id', 'caller', 'source', 'origin', 'offered_results')}


def offered_answer(answer, identity, tick):
    validate_dialog(answer, answer=True)
    require(all(answer[name] == identity[name] for name in ('id', 'caller', 'source', 'origin'))
            and answer['result'] == 2 and answer['status'] == 2 and answer['tick'] == tick,
            'Unrelated, changed or unsuccessful dialog_answer')


def journal(payload, expected_pid, complete=True):
    require(integer(expected_pid, 1), 'Invalid owned child PID')
    require(isinstance(payload, bytes) and len(payload) <= MAX_JOURNAL_BYTES, 'Journal exceeds byte bound')
    if complete:
        require(bool(payload) and payload.endswith(b'\n'), 'Missing or partial journal record')
    lines = payload.splitlines(keepends=True)
    if lines and not lines[-1].endswith(b'\n'):
        lines.pop()
    records, states, spans, seen_spans = [], {}, {}, set()
    ended, sequence, latest_elapsed, previous_draw, session_unix_ns = False, 0, 0, 0, None
    previous_ui_elapsed = 0
    previous_input_unix = 0
    frame_count = 0
    for line in lines:
        require(not ended, 'Record after session_end')
        record = parse_json(line)
        require(isinstance(record, dict) and isinstance(record.get('kind'), str)
                and isinstance(record.get('data'), dict)
                and integer(record.get('elapsed_us'))
                and type(record.get('dropped_records')) is int and record['dropped_records'] == 0,
                'Malformed, regressing or dropped journal record')
        kind, data = record['kind'], record['data']
        if kind in ('input', 'frame', 'dialog_answer'):
            require(record['elapsed_us'] >= previous_ui_elapsed, 'Regressing ordered UI observation')
            previous_ui_elapsed = record['elapsed_us']
        require(not kind.endswith('error'), 'Application, host or observer error')
        require(not kind.startswith('presentation_'), 'Unbound presentation observer is unsupported')
        if not records:
            require(kind == 'session', 'Journal must begin with one session identity')
        if kind == 'session':
            require(not records and type(data.get('schema')) is int and data['schema'] == 1
                    and type(data.get('pid')) is int and data['pid'] == expected_pid,
                    'Duplicate, stale or wrong-PID session')
            stamp = data.get('unix_ns')
            require(isinstance(stamp, str) and stamp.isascii() and stamp.isdecimal()
                    and int(stamp) > 0, 'Invalid session Unix identity')
            session_unix_ns = int(stamp)
        elif kind == 'input':
            require(integer(data.get('input_sequence'), 1) and data['input_sequence'] == sequence + 1
                    and integer(data.get('fence')) and integer(data.get('received_unix_ns'), 1)
                    and data['received_unix_ns'] >= max(session_unix_ns, previous_input_unix) and 'input' in data
                    and (data['input'] is None or isinstance(data['input'], dict)),
                    'Stale, missing or reordered input receipt')
            sequence = data['input_sequence']
            previous_input_unix = data['received_unix_ns']
            receipt = data['input']
            if receipt is not None:
                receipt_kind = receipt.get('kind')
                require(receipt_kind in ('key_pressed', 'key_released', 'pointer_moved',
                                         'button_pressed', 'button_released'), 'Malformed native receipt shape')
                if receipt_kind.startswith('key_'):
                    require(isinstance(receipt.get('physical_key'), str) and bool(receipt['physical_key']),
                            'Missing physical key identity')
                elif receipt_kind.startswith('button_'):
                    require(isinstance(receipt.get('button'), str) and bool(receipt['button']),
                            'Missing button identity')
                else:
                    point = receipt.get('point')
                    require(isinstance(point, list) and len(point) == 2 and all(
                        type(number) in (int, float) and math.isfinite(number) for number in point),
                        'Malformed pointer receipt')
        elif kind == 'dialog_answer':
            validate_dialog(data, answer=True)
        elif kind == 'frame':
            frame_count += 1
            require(type(data.get('frame_id')) is int and data['frame_id'] == frame_count
                    and integer(data.get('fence')) and type(data.get('input_sequence')) is int
                    and data['input_sequence'] == sequence
                    and integer(data.get('draw_started_unix_ns'), 1)
                    and data['draw_started_unix_ns'] >= max(previous_draw, session_unix_ns)
                    and integer(data.get('widget_draw_cpu_us'))
                    and (data.get('draw_interval_us') is None or integer(data['draw_interval_us']))
                    and 'cursor' in data, 'Malformed or unacknowledged draw')
            previous_draw = data['draw_started_unix_ns']
            state = data.get('state')
            require(isinstance(state, dict) and integer(state.get('state_id'), 1)
                    and isinstance(state.get('value'), dict) and isinstance(state.get('diagnostics'), dict),
                    'Missing immutable rendered state or diagnostics')
            value = state['value']
            require((value.get('tick') is None or integer(value['tick']))
                    and all(type(value.get(name)) is bool for name in ('playing', 'pending',
                        'clock_pending', 'presentation_children_pending', 'modal')), 'Malformed observation scalars')
            shell = value.get('shell')
            require(shell is None or isinstance(shell, dict), 'Malformed shell state')
            require('tick' in value and not (value.get('error') or value.get('menu_error')
                    or (shell or {}).get('error')), 'Application error in draw')
            for name in ('dialog', 'dialog_answer'):
                if name in value and value[name] is not None:
                    validate_dialog(value[name], answer=name == 'dialog_answer')
            capabilities = value.get('observation_capabilities')
            require(isinstance(capabilities, dict) and set(capabilities) == set(CAPABILITIES)
                    and all(capabilities[name] is False for name in CAPABILITIES)
                    and all(name in value and value[name] is None for name in
                            ('terminals', 'hud_normal_ready', 'hud_transport_ready')),
                    'Unsupported capability schema; no inferred terminal or readiness evidence')
            identifier = state['state_id']
            require(identifier not in states or states[identifier] == value, 'State identity changed its value')
            states[identifier] = value
        elif kind == 'span_begin':
            identifier = data.get('span_id')
            require(integer(identifier, 1) and identifier not in seen_spans
                    and isinstance(data.get('name'), str), 'Malformed or duplicate span')
            seen_spans.add(identifier)
            spans[identifier] = (data['name'], record['elapsed_us'])
        elif kind == 'span_end':
            identifier = data.get('span_id')
            span = spans.pop(identifier, None) if integer(identifier, 1) else None
            require(span is not None and span[0] == data.get('name')
                    and record['elapsed_us'] >= span[1] and integer(data.get('wall_us')),
                    'Unmatched or invalid span end')
        elif kind in ('scene_prepare', 'scene_encode'):
            require(integer(data.get('frame_id')) and integer(data.get('cpu_us')), 'Invalid scene timing')
        elif kind == 'session_end':
            require(type(data.get('dropped_records')) is int and data['dropped_records'] == 0,
                    'Dropped records at session end')
            require(record['elapsed_us'] >= latest_elapsed, 'Session end predates retained records')
            ended = True
        else:
            raise ValueError('Unsupported journal kind: ' + kind)
        latest_elapsed = max(latest_elapsed, record['elapsed_us'])
        records.append(record)
    if complete:
        require(ended, 'Missing session_end')
        require(not spans, 'Incomplete worker/publication spans')
    return records


def control_effect(records, action, conditions, remembered, fence, started_unix_ns,
                   finished_unix_ns, next_started_unix_ns=None, complete=True):
    require(action == {'kind': 'key', 'code': 124, 'press_ms': 20},
            'Camera control never answers a modal')
    require(integer(fence, 1) and integer(started_unix_ns, 1)
            and integer(finished_unix_ns, 1) and started_unix_ns < finished_unix_ns,
            'Invalid control fence or command timestamps')
    expected = expected_receipts(action)
    receipts, baseline, effect, closing = [], None, None, None
    for record in records:
        data = record['data']
        if (record['kind'] == 'frame' and data['draw_started_unix_ns'] >= started_unix_ns
                and (next_started_unix_ns is None or data['draw_started_unix_ns'] < next_started_unix_ns)):
            require(data['fence'] == fence and usable_camera(data['state']['value']),
                    'Control window contains an unusable or wrong-fence draw')
        if record['kind'] == 'frame' and data['draw_started_unix_ns'] < started_unix_ns:
            if data['fence'] == fence:
                baseline = data
        if record['kind'] == 'input' and data['received_unix_ns'] >= started_unix_ns:
            if next_started_unix_ns is not None and data['received_unix_ns'] >= next_started_unix_ns:
                continue
            if data['input'] is None:
                continue
            require(len(receipts) < len(expected) and data['fence'] == fence
                    and receipt_matches(data['input'], expected[len(receipts)]),
                    'Wrong fence, unrelated or reordered native receipt')
            require(baseline is not None and baseline['input_sequence'] < data['input_sequence']
                    and (not receipts or receipts[-1]['received_unix_ns'] <= data['received_unix_ns']),
                    'Missing fresh baseline or regressing receipt')
            receipts.append(data)
        if record['kind'] != 'frame' or baseline is None or not receipts:
            continue
        draw = data['draw_started_unix_ns']
        if (data['fence'] == fence and data['input_sequence'] >= receipts[0]['input_sequence']
                and draw >= receipts[0]['received_unix_ns']
                and (next_started_unix_ns is None or draw < next_started_unix_ns)
                and effect is None and matches(data['state']['value'], conditions, remembered)):
            effect = {'frame_id': data['frame_id'], 'state_id': data['state']['state_id'],
                      'unix_ns': draw, 'latency_us': (draw - receipts[0]['received_unix_ns']) / 1000}
        if (len(receipts) == len(expected) and data['fence'] == fence
                and data['input_sequence'] >= receipts[-1]['input_sequence']
                and draw >= receipts[-1]['received_unix_ns'] and draw > finished_unix_ns
                and (next_started_unix_ns is None or draw < next_started_unix_ns)):
            closing = data
    require(baseline is not None and usable_camera(baseline['state']['value'])
            and not matches(baseline['state']['value'], conditions, remembered),
            'Endpoint already true or missing baseline')
    ready = len(receipts) == len(expected) and effect is not None and closing is not None
    if complete:
        require(ready, 'Incomplete ordered receipts, acknowledged effect or closing draw')
    return {'complete': ready, 'origin': METRIC_ORIGIN,
            'source_origin': 'window_controls.summarize_controls',
            **metric_summary([effect['latency_us']] if ready else []),
            'baseline_frame_id': baseline['frame_id'], 'receipts': receipts, 'effect': effect,
            'closing_frame_id': closing['frame_id'] if closing else None,
            'scope': 'Fresh key-press receipt to first acknowledged exact-effect draw before next command; '
                     'not native-post, action-completion, displayed-frame or input-to-photon latency; '
                     'gesture duration is not subtracted'}


def offered_history(records, reservation, fence, started_unix_ns=None):
    require(isinstance(reservation, dict) and set(reservation) in (
        {'identity', 'frame'}, {'identity', 'frame', 'pre_post_frame'}),
        'Missing original reserved opening')
    original = reservation['frame']
    positions = [index for index, record in enumerate(records) if record['kind'] == 'frame'
                 and record['data']['frame_id'] == original['frame_id']]
    require(len(positions) == 1 and records[positions[0]]['data'] == original,
            'Original reserved opening draw changed or disappeared')
    latest = None
    post_started = False
    for index in range(positions[0], len(records)):
        record = records[index]
        kind, data = record['kind'], record['data']
        stamp = data.get('draw_started_unix_ns') if kind == 'frame' else data.get('received_unix_ns')
        if started_unix_ns is not None and stamp is not None and stamp >= started_unix_ns:
            post_started = True
            continue
        if post_started:
            require(stamp is None, 'Late pre-command observation after response input or draw')
            continue
        try:
            require(type(record.get('dropped_records')) is int and record['dropped_records'] == 0
                    and not kind.endswith('error') and not kind.startswith('presentation_')
                    and kind != 'session_end', 'Error, drop, presentation or end during opening reservation')
            require(kind not in ('input', 'dialog_answer'),
                    'Unacknowledged input or answer after the immediately preceding draw')
            if kind != 'frame':
                continue
            value = data['state']['value']
            paused_response_state(value, original['state']['value']['tick'])
            require(not (value.get('error') or value.get('menu_error') or (value.get('shell') or {}).get('error')),
                    'Application error during opening reservation')
            require(offered_opening(value) == reservation['identity'],
                    'Offered opening changed immediately before native Enter')
            require(integer(data.get('fence'), 1) and data['fence'] == fence
                    and type(data.get('input_sequence')) is int
                    and data['input_sequence'] == original['input_sequence'],
                    'Fence or input sequence changed during opening reservation')
            size = value.get('display_size')
            require(isinstance(size, list) and len(size) == 2
                    and all(finite(number, 1) for number in size)
                    and size == original['state']['value'].get('display_size'),
                    'Opening reservation geometry changed')
            latest = data
        except ValueError as error:
            raise ValueError('Opening reservation history blocker at record {}: {}'.format(index, error)) from error
    require(latest is not None, 'Missing pre-command original opening draw')
    return latest


def offered_response(records, reservation, fence, started_unix_ns, finished_unix_ns,
                     next_started_unix_ns=None, complete=True):
    require(integer(fence, 1) and integer(started_unix_ns, 1)
            and integer(finished_unix_ns, 1) and started_unix_ns < finished_unix_ns,
            'Invalid offered-response fence or timestamps')
    baseline = offered_history(records, reservation, fence, started_unix_ns)
    require(baseline == reservation.get('pre_post_frame', reservation['frame']),
            'Reserved opening is not the immediately preceding observed draw')
    start_position = next(index for index, record in enumerate(records)
                          if record['kind'] == 'frame' and record['data'] == reservation['frame'])
    receipts, acknowledgment, answer_event = [], None, None
    expected = expected_receipts(TUTORIAL_ACTION)
    for record in records[start_position:]:
        data = record['data']
        if record['kind'] == 'frame' and data['draw_started_unix_ns'] < started_unix_ns:
            continue
        if record['kind'] == 'dialog_answer':
            require(baseline is not None and receipts and answer_event is None,
                    'Missing Enter press or repeated/unrelated dialog_answer event')
            offered_answer(data, reservation['identity'], reservation['frame']['state']['value']['tick'])
            answer_event = record
            continue
        stamp = data.get('draw_started_unix_ns') if record['kind'] == 'frame' else data.get('received_unix_ns')
        if stamp is None or stamp < started_unix_ns or (
                next_started_unix_ns is not None and stamp >= next_started_unix_ns):
            continue
        require(baseline['fence'] == fence
                and offered_opening(baseline['state']['value']) == reservation['identity'],
                'Reserved opening is not the immediately preceding observed draw')
        if record['kind'] == 'input':
            if data['input'] is None:
                continue
            require(len(receipts) < 2 and data['fence'] == fence
                    and data['input_sequence'] == baseline['input_sequence'] + len(receipts) + 1
                    and receipt_matches(data['input'], expected[len(receipts)])
                    and (not receipts or stamp >= receipts[-1]['received_unix_ns']),
                    'Wrong fence, sequences or unrelated/repeated Enter receipt')
            receipts.append(data)
            continue
        if record['kind'] != 'frame':
            continue
        require(data['fence'] == fence, 'Wrong offered-response draw fence')
        value = data['state']['value']
        paused_response_state(value, baseline['state']['value']['tick'])
        require(value.get('display_size') == reservation['frame']['state']['value']['display_size'],
                'Offered-response geometry changed')
        require(not (value.get('error') or value.get('menu_error') or (value.get('shell') or {}).get('error')),
                'Application error before response acknowledgment')
        if value['modal'] is True:
            require(acknowledgment is None and offered_opening(value) == reservation['identity'],
                    'Changed or next modal requires player input')
            continue
        require(value['modal'] is False and value['dialog'] is None and value['dialog_answer'] is not None,
                'Unsupported: modal release lacks an actual dialog_answer')
        answer = value['dialog_answer']
        offered_answer(answer, reservation['identity'], baseline['state']['value']['tick'])
        if (len(receipts) == 2 and data['input_sequence'] == receipts[-1]['input_sequence']
                and stamp >= receipts[-1]['received_unix_ns'] and stamp > finished_unix_ns
                and answer_event is not None and answer_event['data'] == answer
                and acknowledgment is None):
            acknowledgment = {'frame_id': data['frame_id'], 'state_id': data['state']['state_id'],
                'unix_ns': stamp, 'answer': answer,
                'receipt_to_acknowledgment_us': (stamp - receipts[0]['received_unix_ns']) / 1000}
    require(baseline['fence'] == fence
            and offered_opening(baseline['state']['value']) == reservation['identity'],
            'Missing or changed reserved opening baseline')
    ready = len(receipts) == 2 and acknowledgment is not None
    if complete:
        require(ready, 'Incomplete Enter receipts or offered-response acknowledgment')
    return {'complete': ready, 'origin': RESPONSE_ORIGIN, 'baseline_frame_id': baseline['frame_id'],
            'reserved_observed_identity': reservation['identity'], 'receipts': receipts,
            'acknowledgment': acknowledgment, 'answer_event': answer_event,
            'acknowledgment_us': distribution([acknowledgment['receipt_to_acknowledgment_us']] if ready else []),
            'scope': 'One offered native Enter response only; receipt-to-acknowledging-draw diagnostic, '
                     'not actual-input or presentation latency, tutorial completion or first-tick proof; '
                     'no 100ms/250ms control gates apply'}


def summarize(records, control, response=None):
    phases = dict.fromkeys(('startup', 'pending', 'modal', 'camera_observable_not_full_readiness'), 0)
    previous = None
    for record in records:
        if record['kind'] != 'frame':
            continue
        value = record['data']['state']['value']
        phase = ('startup' if value.get('tick') is None or value.get('camera') is None else
                 'modal' if value.get('modal') else 'pending' if any(value.get(name) for name in
                 ('pending', 'clock_pending', 'presentation_children_pending')) else
                 'camera_observable_not_full_readiness')
        if previous is not None:
            phases[previous[0]] += record['elapsed_us'] - previous[1]
        previous = (phase, record['elapsed_us'])
    return {**QUALIFICATIONS, 'diagnostic_only': True, 'control_effect': control,
            'offered_response_acknowledgment': response, 'tutorial_completion_claimed': False,
            'first_tick_claimed': False,
            'phase_wall_us_not_eligibility': phases, 'unsupported_capabilities': CAPABILITIES,
            'full_readiness': None, 'eligible_seconds': None, 'civil_days': None,
            'visual_proof': False, 'worker_quiescence': None,
            'native_post_to_receipt_delivery': {'available': False, 'latency_us': None,
                'reason': 'No validated cross-process realtime/monotonic correlation and uncertainty bound'},
            'presentation': {'available': False, 'abi': 1, 'event_bytes': 192, 'summary_bytes': 408,
                'reason': 'Authoritative snapshot has no native hook; no separately bound observer accepted'},
            'input_to_photon': {'available': False, 'latency_us': None}}
