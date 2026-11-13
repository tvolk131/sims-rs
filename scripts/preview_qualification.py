"""Recompute local preview gates; a manifest's passed flag is never an attestation."""
import contextlib
import hashlib
import json
from pathlib import Path
import re
import tempfile

from window_controls import summarize_controls
from window_display import summarize_displays
from window_presentation import qualify_presentations, summarize_presentations
from window_session import HOST_CLOCK, distribution, matches


def require(condition, message):
    if not condition:
        raise ValueError(message)


@contextlib.contextmanager
def telemetry_snapshot(filename, expected):
    with tempfile.TemporaryDirectory(prefix='.verified-', dir=filename.parent) as directory:
        snapshot = Path(directory) / 'events.jsonl'
        checksum = hashlib.sha256()
        with filename.open('rb') as source, snapshot.open('xb') as target:
            for chunk in iter(lambda: source.read(1024 * 1024), b''):
                target.write(chunk)
                checksum.update(chunk)
        require(checksum.hexdigest() == expected,
                'Consumed telemetry snapshot differs from its frozen identity: ' + str(filename))
        snapshot.chmod(0o400)
        yield snapshot


def presentation_records(filename, completed=()):
    records = []
    expected_states = {}
    for entry in completed:
        expected_states.setdefault(entry['frame_id'], []).append(entry['state'])
    with filename.open() as source:
        for line in source:
            record = json.loads(line)
            if record.get('kind', '').startswith('presentation_'):
                records.append(record)
            elif record.get('kind') == 'frame':
                frame = record['data']
                state = frame['state']
                for expected in expected_states.pop(frame['frame_id'], []):
                    require(state == expected, 'Completed journey state differs from its actual raw draw')
                records.append({'kind': 'frame', 'data': {'frame_id': frame['frame_id'],
                                'draw_interval_us': frame.get('draw_interval_us'),
                                'state': {'state_id': state['state_id'],
                                          'value': {'tick': state['value']['tick']}}}})
    require(not expected_states, 'Completed journey draw is missing from raw telemetry')
    return records


def ready_draw_summary(records):
    intervals, gaps = [], []
    for record in records:
        if record['kind'] != 'frame' or record['data']['state']['value']['tick'] is None:
            continue
        frame = record['data']
        interval = frame['draw_interval_us']
        if interval is None and frame['frame_id'] == 1:
            continue
        require(type(interval) is int and interval >= 0, 'Malformed ready application draw interval')
        intervals.append(interval)
        if interval > 100_000:
            gaps.append({'frame_id': frame['frame_id'], 'tick': frame['state']['value']['tick'],
                         'interval_us': interval})
    summary = distribution(intervals)
    require(summary.get('samples', 0) >= 2 and summary['p95'] <= 20_000
            and summary['p99'] <= 33_400 and summary['worst'] <= 250_000,
            'Ready application draw gates did not pass')
    return {'interval_us': summary, 'gaps_over_100ms': gaps}


def verify_qualification(manifest, resolve, checksum):
    qualification = manifest.get('qualification', {})
    cohorts = qualification.get('cohorts')
    require(isinstance(cohorts, list) and len(cohorts) == 3,
            'Complete performance qualification requires three consecutive evidence cohorts')
    retained, private_entries = {}, []

    def retain(filename, expected):
        require(filename.is_file() and checksum(filename) == expected,
                'Qualification input is missing or changed: ' + str(filename))
        require(retained.get(filename, expected) == expected,
                'Conflicting retained qualification input identities')
        retained[filename] = expected
        return filename

    def verified(entry):
        filename = resolve(entry['path'])
        private_entries.append(entry['path'])
        return retain(filename, entry['sha256'])

    def snapshot(filename, expected):
        payload = filename.read_bytes()
        require(hashlib.sha256(payload).hexdigest() == expected,
                'Consumed qualification snapshot differs from its frozen identity: ' + str(filename))
        return json.loads(payload)

    endpoint_file = verified(qualification['endpoints'])
    endpoints = snapshot(endpoint_file, qualification['endpoints']['sha256'])
    build_file = verified(qualification['build_receipt'])
    build = snapshot(build_file, qualification['build_receipt']['sha256'])
    require(build.get('source_unchanged_during_native_build') is True,
            'The native build lacks a stable-source receipt')
    require(build['artifacts']['sims-room'] == manifest['binary']['sha256'],
            'Build receipt does not identify the preview binary')
    directories, reports = [], []
    identities = None
    for cohort in cohorts:
        report_file, telemetry = verified(cohort['result']), verified(cohort['telemetry'])
        directory = report_file.parent
        require(report_file.name == 'result.json' and telemetry == directory / 'telemetry/events.jsonl',
                'Cohort result and raw telemetry must belong to the same native run')
        require(directory not in directories, 'Qualification cohorts must be distinct')
        directories.append(directory)
        reports.append(snapshot(report_file, cohort['result']['sha256']))
    series = [re.fullmatch(r'(.+)-(\d+)', directory.name) for directory in directories]
    require(all(series) and len({directory.parent for directory in directories}) == 1
            and len({match[1] for match in series}) == 1,
            'Qualification cohorts must share one numbered attempt series')
    numbers = [int(match[2]) for match in series]
    require(numbers == list(range(numbers[0], numbers[0] + 3)),
            'Qualification cohorts are not consecutive attempts')
    summaries, previous_end, previous_host_end, previous_presentation_end = [], None, None, None
    for cohort, directory, report in zip(cohorts, directories, reports):
        require(report.get('passed') is True and report.get('graceful_end') is True
                and report.get('error') is None, 'A native cohort failed or did not end gracefully')
        current = {name: entry['sha256'] for name, entry in report['inputs'].items()}
        require(set(current) == {'binary', 'native_driver', 'load', 'plan', 'runner',
                                 'present_library', 'presentation_analyzer', 'display_analyzer'},
                'Incomplete frozen native-input identities')
        require(all(entry['sha256'] == entry['final_sha256'] for entry in report['inputs'].values()),
                'Cohort inputs changed while running')
        require(identities is None or current == identities, 'Cohorts have different frozen inputs')
        identities = current
        require(current['binary'] == manifest['binary']['sha256']
                and current['load'] == manifest['supported_save']['sha256']
                and current['native_driver'] == build['artifacts']['window-session-macos']
                and current['present_library'] == build['artifacts']['window-present-macos.dylib'],
                'Qualification inputs do not identify this preview bundle')
        for name, entry in report['inputs'].items():
            retain(Path(entry['path']), entry['sha256'])
        plan_identity = report['inputs']['plan']
        plan = snapshot(Path(plan_identity['path']), plan_identity['sha256'])
        require(endpoints['plan_sha256'] == plan_identity['sha256'], 'Frozen endpoint/plan identity mismatch')
        completed = [entry for entry in report['journal']
                     if 'step' in entry and 'input_receipts' not in entry]
        require([entry['step'] for entry in completed] == [step['name'] for step in plan['steps']]
                and all(entry.get('passed') is True and 'frame_id' in entry and 'state' in entry
                        for entry in completed),
                'Incomplete or reordered native journey steps')
        remembered = {}
        for step, entry in zip(plan['steps'], completed):
            state = entry['state']['value']
            if 'remember' in step:
                remembered[step['remember']] = state
            conditions = step.get('wait', step.get('observe', {}).get('conditions', []))
            require(matches(state, conditions, remembered), 'Native journey effect did not match: ' + step['name'])
        command = report['application_command']
        require(Path(command[command.index('--root')+1]).resolve() == Path(manifest['game_root']).resolve(),
                'Preview game-data root differs from qualification')
        commands = [entry for entry in report['journal'] if 'native_command' in entry]
        lifecycle = report['process_lifecycle']
        start, end = lifecycle['started_unix_ns'], lifecycle['finished_unix_ns']
        host_start, host_end = lifecycle['started_host_ns'], lifecycle['finished_host_ns']
        require(all(type(value) is int and value > 0 for value in (start, end, host_start, host_end))
                and start < end and host_start < host_end and commands
                and lifecycle['host_clock'] == HOST_CLOCK
                and all(type(command['started_unix_ns']) is int and type(command['finished_unix_ns']) is int
                        and start <= command['started_unix_ns'] <= command['finished_unix_ns'] <= end
                        for command in commands)
                and (previous_end is None or start > previous_end)
                and (previous_host_end is None or host_start > previous_host_end),
                'Qualification cohorts overlap or are not in chronological order')
        previous_end, previous_host_end = end, host_end
        with telemetry_snapshot(directory / 'telemetry/events.jsonl',
                                cohort['telemetry']['sha256']) as consumed_telemetry:
            with consumed_telemetry.open() as source:
                controls = summarize_controls((json.loads(line) for line in source), plan, report, endpoints)
            require(controls['valid'] is True and controls['qualified'] is True,
                    'Native control response gates did not pass')
            displays = summarize_displays(report['journal'], required=True)
            require(displays['valid'] is True, 'Native target-display evidence is invalid')
            records = presentation_records(consumed_telemetry, completed)
            application_draws = ready_draw_summary(records)
            presentation = summarize_presentations(records, required=True)
        long_gate = qualify_presentations(presentation)
        require(long_gate['qualified'] is True,
                'Extended presentation gates did not pass: ' + '; '.join(long_gate['errors']))
        presentation_lifecycle = presentation['lifecycle_host_seconds']
        require(host_start / 1_000_000_000 <= presentation_lifecycle['begin']
                and presentation_lifecycle['snapshot'] <= host_end / 1_000_000_000,
                'Presentation lifecycle lies outside its complete application process envelope')
        require(previous_presentation_end is None or presentation_lifecycle['begin'] > previous_presentation_end,
                'Qualification presentation lifecycle envelopes overlap')
        previous_presentation_end = presentation_lifecycle['snapshot']
        summaries.append({'run': str(directory), 'controls': controls['latency_us'],
                          'application_draws': application_draws,
                          'ready_seconds': presentation['ready_presented_duration_seconds'],
                          'presentation_intervals_us': presentation['ready_presented_interval_us'],
                          'process_lifecycle': lifecycle, 'presentation_lifecycle': presentation_lifecycle})
    for value in private_entries:
        require(resolve(value) in retained, 'Private qualification path changed during verification')
    for filename, expected in retained.items():
        require(checksum(filename) == expected, 'Qualification input changed during verification: ' + str(filename))
    return {'passed': True, 'cohorts': summaries,
            'consumed_endpoints_sha256': qualification['endpoints']['sha256'],
            'scope': 'Locally recomputed raw-evidence gates, not cryptographic provenance or protection '
                     'against malicious changes to this checkout, recorder or installed game data'}
