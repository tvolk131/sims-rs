"""Measure explicitly frozen native-input effects, not action completion or FPS."""
import argparse
import json
import math
from pathlib import Path

from window_session import (MAC_KEYS, distribution, file_digest, matches,
                            validate_plan, validate_predicates)


def require(condition, message):
    if not condition:
        raise ValueError(message)


def validate_endpoints(plan, specification):
    validate_plan(plan)
    require(set(specification) == {'schema', 'plan_sha256', 'controls'}
            and specification['schema'] == 1, 'Unsupported control endpoint specification')
    digest = specification['plan_sha256']
    require(isinstance(digest, str) and len(digest) == 64
            and all(character in '0123456789abcdef' for character in digest), 'Invalid plan digest')
    endpoints = specification['controls']
    require(isinstance(endpoints, list), 'Control endpoints must be an ordered list')
    inputs = [step for step in plan['steps'] if 'input' in step]
    require(len(inputs) == len(endpoints) and inputs, 'Every input needs exactly one frozen endpoint')
    available, cursor = set(), 0
    for step in plan['steps']:
        if 'remember' in step:
            available.add(step['remember'])
        if 'input' not in step:
            continue
        endpoint = endpoints[cursor]
        require(step['input']['kind'] in ('click', 'key'), 'Only click/key controls are qualified')
        require(isinstance(endpoint, dict) and set(endpoint) == {'step', 'conditions'}
                and endpoint['step'] == step['name'] and endpoint['conditions'],
                'Endpoints must follow exact plan input order')
        validate_predicates(endpoint['conditions'], available)
        cursor += 1
    return endpoints


def expected_receipts(action):
    if action['kind'] == 'key':
        physical = 'Code({})'.format(MAC_KEYS[action['code']])
        return [{'kind': 'key_pressed', 'physical_key': physical},
                {'kind': 'key_released', 'physical_key': physical}]
    return [{'kind': 'pointer_moved', 'point': action['point']},
            {'kind': 'button_pressed', 'button': 'Left'},
            {'kind': 'button_released', 'button': 'Left'}]


def receipt_matches(actual, expected):
    if not isinstance(actual, dict) or actual.get('kind') != expected['kind']:
        return False
    if 'point' in expected:
        point = actual.get('point')
        return (isinstance(point, list) and len(point) == 2
                and all(type(value) in (int, float) and math.isfinite(value)
                        and abs(value - wanted) <= 1 for value, wanted in zip(point, expected['point'])))
    return all(actual.get(name) == value for name, value in expected.items())


def summarize_controls(records, plan, report, specification):
    endpoints = validate_endpoints(plan, specification)
    require(report.get('passed') is True and report.get('graceful_end') is True,
            'A passed, gracefully ended native journey is required')
    completed, receipt_groups = {}, {}
    for entry in report['journal']:
        if entry.get('passed') is True and 'state' in entry:
            require(entry['step'] not in completed, 'Duplicate completed step')
            completed[entry['step']] = entry
        if 'input_receipts' in entry:
            require(entry['step'] not in receipt_groups, 'Duplicate input receipt group')
            receipt_groups[entry['step']] = entry
    require(set(receipt_groups) == {endpoint['step'] for endpoint in endpoints},
            'Missing or unexpected input receipt groups')
    remembered, remember_frames, controls, receipt_indices = {}, {}, [], {}
    previous_index, previous_finish, endpoint_index = -1, 0, 0
    for step in plan['steps']:
        if 'remember' in step:
            entry = completed[step['name']]
            remembered[step['remember']] = entry['state']['value']
            remember_frames.setdefault(entry['frame_id'], []).append(entry['state'])
        if 'input' not in step:
            continue
        name = step['name']
        group = receipt_groups[name]
        expected = expected_receipts(step['input'])
        indices, sequences = group['input_receipts'], group['receipt_input_sequences']
        started, finished, fence = (group['input_started_unix_ns'],
                                    group['input_finished_unix_ns'], group['fence'])
        require(type(started) is int and type(finished) is int and previous_finish < started < finished
                and type(fence) is int and fence > 0, 'Invalid control command timestamps or fence')
        require(isinstance(indices, list) and len(indices) == len(expected)
                and all(type(index) is int and index > previous_index for index in indices)
                and indices == sorted(set(indices)) and isinstance(sequences, list)
                and len(sequences) == len(expected)
                and all(type(sequence) is int and sequence > 0 for sequence in sequences)
                and sequences == sorted(set(sequences)), 'Malformed ordered input receipt group')
        control = {'step': name, 'kind': step['input']['kind'], 'conditions': endpoints[endpoint_index]['conditions'],
                   'started_unix_ns': started, 'finished_unix_ns': finished, 'fence': fence,
                   'receipt_indices': indices, 'receipt_sequences': sequences, 'expected': expected,
                   'remembered': dict(remembered), 'baseline': None, 'receipt': None, 'effect': None,
                   'closing_frame': completed[name]['frame_id'], 'closing_state': completed[name]['state'],
                   'receipts_seen': 0, 'last_received_unix_ns': 0, 'closing_seen': False}
        controls.append(control)
        for ordinal, index in enumerate(indices):
            receipt_indices[index] = (control, ordinal)
        previous_index, previous_finish = indices[-1], finished
        endpoint_index += 1
    for earlier, later in zip(controls, controls[1:]):
        earlier['next_started_unix_ns'] = later['started_unix_ns']
    controls[-1]['next_started_unix_ns'] = None
    previous_frame, previous_draw_time, ended = 0, 0, False
    for record_index, record in enumerate(records):
        require(not record.get('dropped_records', 0), 'Recorder loss invalidates control timing')
        if record['kind'] == 'session_end':
            require(not ended, 'Duplicate session end')
            ended = True
        if record_index in receipt_indices:
            control, ordinal = receipt_indices[record_index]
            data = record['data']
            received = data.get('received_unix_ns')
            require(record['kind'] == 'input' and data.get('fence') == control['fence']
                    and data.get('input_sequence') == control['receipt_sequences'][ordinal]
                    and type(received) is int and received > control['started_unix_ns']
                    and received >= control['last_received_unix_ns']
                    and receipt_matches(data.get('input'), control['expected'][ordinal]),
                    'Mismatched, stale or misidentified native receipt for ' + control['step'])
            require(control['baseline'] is not None and control['baseline']['input_sequence']
                    < control['receipt_sequences'][0], 'Missing fresh pre-input observation')
            require(ordinal == control['receipts_seen'], 'Reordered receipt evidence')
            control['receipts_seen'] += 1
            control['last_received_unix_ns'] = received
            if ordinal == (len(control['expected']) - 1 if control['kind'] == 'click' else 0):
                control['receipt'] = {'record_index': record_index, 'unix_ns': received,
                                      'input_sequence': data['input_sequence']}
        if record['kind'] != 'frame':
            continue
        require(not ended, 'Draw after session end')
        frame = record['data']
        draw_time, identifier = frame['draw_started_unix_ns'], frame['frame_id']
        require(type(draw_time) is int and type(identifier) is int and identifier > previous_frame
                and draw_time >= previous_draw_time, 'Regressing draw identity or timestamp')
        previous_frame, previous_draw_time = identifier, draw_time
        state = frame['state']['value']
        require(not (state.get('error') or state.get('menu_error') or state.get('shell', {}).get('error')),
                'Application error invalidates control evidence')
        for remembered_state in remember_frames.pop(identifier, []):
            require(frame['state'] == remembered_state, 'Remembered state differs from its actual draw')
        for control in controls:
            if frame['fence'] == control['fence'] and draw_time < control['started_unix_ns']:
                control['baseline'] = {'frame_id': identifier, 'input_sequence': frame['input_sequence'],
                                       'already_satisfied': matches(state, control['conditions'], control['remembered'])}
            if identifier == control['closing_frame']:
                require(frame['state'] == control['closing_state']
                        and control['receipts_seen'] == len(control['expected'])
                        and frame['input_sequence'] >= control['receipt_sequences'][-1]
                        and draw_time >= control['last_received_unix_ns']
                        and draw_time > control['finished_unix_ns'], 'Closing draw lacks exact receipt acknowledgment')
                control['closing_seen'] = True
            receipt = control['receipt']
            next_start = control['next_started_unix_ns']
            if (receipt is not None and control['effect'] is None
                    and frame['input_sequence'] >= receipt['input_sequence']
                    and draw_time >= receipt['unix_ns'] and (next_start is None or draw_time < next_start)
                    and matches(state, control['conditions'], control['remembered'])):
                control['effect'] = {'frame_id': identifier, 'state_id': frame['state']['state_id'],
                                     'record_index': record_index, 'unix_ns': draw_time,
                                     'latency_us': (draw_time - receipt['unix_ns']) / 1000}
    require(ended and not remember_frames, 'Missing session end or actual remembered frames')
    for control in controls:
        require(control['receipts_seen'] == len(control['expected']) and control['closing_seen']
                and control['effect'] is not None, 'Incomplete receipt/effect evidence for ' + control['step'])
        require(not control['baseline']['already_satisfied'],
                'Effect was already satisfied before input: ' + control['step'])
    samples = [{name: control[name] for name in ('step', 'kind', 'conditions', 'baseline', 'receipt', 'effect')}
               for control in controls]
    latencies = distribution([sample['effect']['latency_us'] for sample in samples])
    return {'valid': True, 'qualified': latencies['p95'] <= 100_000 and latencies['worst'] <= 250_000,
            'samples': samples, 'latency_us': latencies, 'p95_limit_us': 100_000, 'maximum_limit_us': 250_000,
            'scope': 'Fresh pointer-release/key-press receipt to first acknowledged exact-effect draw '
                     'before the next command; not action-completion or input-to-photon latency'}


def summarize_run(directory, endpoint_file):
    report = json.loads((directory / 'result.json').read_text())
    specification = json.loads(endpoint_file.read_text())
    plan_file = Path(report['inputs']['plan']['path'])
    require(file_digest(plan_file) == specification['plan_sha256'] == report['inputs']['plan']['sha256'],
            'Frozen plan identity mismatch')
    require(all(identity['sha256'] == identity['final_sha256'] for identity in report['inputs'].values()),
            'Run inputs changed during execution')
    plan = json.loads(plan_file.read_text())
    telemetry = directory / 'telemetry/events.jsonl'
    before = file_digest(telemetry)
    with telemetry.open() as stream:
        summary = summarize_controls((json.loads(line) for line in stream), plan, report, specification)
    require(file_digest(telemetry) == before, 'Telemetry changed during analysis')
    summary['identities'] = {'telemetry_sha256': before, 'endpoints_sha256': file_digest(endpoint_file),
                             'result_sha256': file_digest(directory / 'result.json'),
                             'analyzer_sha256': file_digest(Path(__file__))}
    return summary


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('run', type=Path)
    parser.add_argument('--endpoints', type=Path, required=True)
    arguments = parser.parse_args()
    print(json.dumps(summarize_run(arguments.run, arguments.endpoints), indent=2))
