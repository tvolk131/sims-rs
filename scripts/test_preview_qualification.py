import contextlib
import copy
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock

import play_preview
import preview_qualification


class QualificationTests(unittest.TestCase):
    def prepare(self, directory):
        root = Path(directory).resolve()
        cache = root / 'cache'
        cache.mkdir()

        def artifact(name, value):
            filename = cache / name
            filename.parent.mkdir(parents=True, exist_ok=True)
            filename.write_text(json.dumps(value))
            return {'path': str(filename), 'sha256': play_preview.digest(filename)}

        binary = artifact('sims-room', 'synthetic unexecuted binary')
        supported = artifact('supported-save.json', {'source': 'synthetic'})
        native = artifact('driver', 'synthetic')
        present = artifact('observer', 'synthetic')
        plan = artifact('plan.json', {'schema': 1, 'steps': [{'name': 'ready', 'remember': 'initial'}]})
        endpoints = artifact('endpoints.json', {'schema': 1, 'plan_sha256': plan['sha256'], 'controls': []})
        runner = artifact('runner', 'synthetic')
        presentation_analyzer = artifact('presentation-analyzer', 'synthetic')
        display_analyzer = artifact('display-analyzer', 'synthetic')
        build = artifact('build.json', {'source_unchanged_during_native_build': True,
                         'artifacts': {'sims-room': binary['sha256'],
                                       'window-session-macos': native['sha256'],
                                       'window-present-macos.dylib': present['sha256']}})
        manifest = {'schema': 1, 'binary': binary, 'supported_save': supported,
                    'game_root': str(root / 'game-data'), 'qualification': {
                    'passed': True, 'endpoints': endpoints, 'build_receipt': build, 'cohorts': []}}
        inputs = {name: {**entry, 'final_sha256': entry['sha256']} for name, entry in {
            'binary': binary, 'load': supported, 'plan': plan, 'native_driver': native,
            'present_library': present, 'runner': runner,
            'presentation_analyzer': presentation_analyzer, 'display_analyzer': display_analyzer}.items()}
        for number in range(1, 4):
            start = 1_000_000_000_000_000_000 + number * 1_000_000_000_000
            report = {'passed': True, 'error': None, 'graceful_end': True,
                      'inputs': copy.deepcopy(inputs),
                      'process_lifecycle': {'started_unix_ns': start - 10,
                          'finished_unix_ns': start + 999_000_000_000,
                          'started_host_ns': number * 1_000_000_000_000,
                          'finished_host_ns': number * 1_000_000_000_000 + 999_000_000_000,
                          'host_clock': preview_qualification.HOST_CLOCK},
                      'application_command': ['synthetic', '--root', manifest['game_root']],
                      'journal': [{'native_command': ['window'], 'started_unix_ns': start,
                                   'finished_unix_ns': start + 1},
                                  {'step': 'ready', 'passed': True, 'frame_id': 1,
                                   'state': {'state_id': 1, 'value': {'tick': 0}}}]}
            manifest['qualification']['cohorts'].append({
                'result': artifact(f'run-{number:02d}/result.json', report),
                'telemetry': artifact(f'run-{number:02d}/telemetry/events.jsonl', {
                    'kind': 'frame', 'data': {'frame_id': 1, 'draw_interval_us': 8333,
                                             'state': {'state_id': 1, 'value': {'tick': 0}}}})})
        return root, manifest

    @contextlib.contextmanager
    def analyses(self, presentation=None, controls=None):
        summary = {'valid': True, 'ready_presented_duration_seconds': 900,
                   'ready_leading_gap_us': 10_000, 'ready_terminal_gap_us': 20_000,
                   'ready_presented_interval_us': {'samples': 54000, 'p95': 16667, 'p99': 16667, 'worst': 16667}}
        if presentation:
            summary.update(presentation)
        def presented(records, required):
            presented.calls += 1
            start = presented.calls * 1000 + 1
            return {**summary, 'lifecycle_host_seconds': {'begin': start, 'cutoff': start + 900.01,
                    'drain_end': start + 900.02, 'snapshot': start + 900.03}}
        presented.calls = 0
        with mock.patch.object(preview_qualification, 'summarize_controls', return_value=controls or {
                'valid': True, 'qualified': True, 'latency_us': {'p95': 50000, 'worst': 75000}}), \
                mock.patch.object(preview_qualification, 'summarize_displays', return_value={'valid': True}), \
                mock.patch.object(preview_qualification, 'ready_draw_summary', return_value={
                    'interval_us': {'samples': 54000, 'p95': 8333, 'p99': 16667, 'worst': 50000},
                    'gaps_over_100ms': []}), \
                mock.patch.object(preview_qualification, 'summarize_presentations', side_effect=presented):
            yield

    def verify(self, root, manifest):
        with mock.patch.object(play_preview, 'REPOSITORY', root):
            return preview_qualification.verify_qualification(manifest, play_preview.private_path, play_preview.digest)

    def rewrite_result(self, manifest, index, change):
        entry = manifest['qualification']['cohorts'][index]['result']
        filename = Path(entry['path'])
        report = json.loads(filename.read_text())
        change(report)
        filename.write_text(json.dumps(report))
        entry['sha256'] = play_preview.digest(filename)

    def test_complete_bound_cohorts_use_current_analyzers_and_gates(self):
        with tempfile.TemporaryDirectory() as directory:
            root, manifest = self.prepare(directory)
            with self.analyses(), mock.patch.object(preview_qualification, 'presentation_records', wraps=
                    preview_qualification.presentation_records) as records:
                result = self.verify(root, manifest)
            self.assertTrue(result['passed'])
            self.assertEqual(len(result['cohorts']), 3)
            self.assertEqual(records.call_count, 3)
            self.assertEqual(result['consumed_endpoints_sha256'], manifest['qualification']['endpoints']['sha256'])

    def test_controls_consume_one_verified_endpoint_snapshot_despite_file_mutation(self):
        with tempfile.TemporaryDirectory() as directory:
            root, manifest = self.prepare(directory)
            endpoint_file = Path(manifest['qualification']['endpoints']['path'])
            expected = json.loads(endpoint_file.read_bytes())
            consumed = []
            def analyze(records, plan, report, endpoints):
                consumed.append(copy.deepcopy(endpoints))
                endpoint_file.write_text(json.dumps({**expected, 'controls': ['changed after verification']}))
                return {'valid': True, 'qualified': True, 'latency_us': {'p95': 50000, 'worst': 75000}}
            with self.analyses(), mock.patch.object(preview_qualification, 'summarize_controls', side_effect=analyze), \
                    self.assertRaisesRegex(ValueError, 'changed during verification'):
                self.verify(root, manifest)
            self.assertEqual(consumed, [expected] * 3)

    def test_consumed_json_bytes_must_match_the_verified_file_identity(self):
        for variant in ('endpoints', 'build_receipt', 'result', 'plan'):
            with self.subTest(variant=variant), tempfile.TemporaryDirectory() as directory:
                root, manifest = self.prepare(directory)
                qualification = manifest['qualification']
                entry = (qualification[variant] if variant in ('endpoints', 'build_receipt') else
                         qualification['cohorts'][0]['result'])
                if variant == 'plan':
                    report = json.loads(Path(entry['path']).read_bytes())
                    entry = report['inputs']['plan']
                selected = Path(entry['path'])
                read_bytes = Path.read_bytes
                def changed_snapshot(filename):
                    payload = read_bytes(filename)
                    if filename == selected:
                        return json.dumps({**json.loads(payload), 'unexpected_mutation': True}).encode()
                    return payload
                with self.analyses(), mock.patch.object(Path, 'read_bytes', changed_snapshot), \
                        self.assertRaisesRegex(ValueError, 'Consumed qualification snapshot'):
                    self.verify(root, manifest)

    def test_consumed_telemetry_bytes_must_match_despite_unchanged_source_checksums(self):
        with tempfile.TemporaryDirectory() as directory:
            root, manifest = self.prepare(directory)
            entry = manifest['qualification']['cohorts'][0]['telemetry']
            selected = Path(entry['path'])
            replacement = json.dumps({**json.loads(selected.read_bytes()),
                                      'unverified_substitution': True}).encode()
            original_open, original_digest = Path.open, play_preview.digest
            hashing = False

            def checksum(filename):
                nonlocal hashing
                hashing = True
                try:
                    return original_digest(filename)
                finally:
                    hashing = False

            def changed_stream(filename, mode='r', *arguments, **keywords):
                if filename == selected and not hashing and mode in ('r', 'rb'):
                    return io.BytesIO(replacement) if mode == 'rb' else io.StringIO(replacement.decode())
                return original_open(filename, mode, *arguments, **keywords)

            with self.analyses(), mock.patch.object(Path, 'open', changed_stream), \
                    mock.patch.object(play_preview, 'digest', side_effect=checksum), \
                    self.assertRaisesRegex(ValueError, 'Consumed telemetry snapshot'):
                self.verify(root, manifest)
            self.assertEqual(play_preview.digest(selected), entry['sha256'])
            self.assertEqual(list(selected.parent.glob('.verified-*')), [])

    def test_controls_and_presentations_share_one_private_verified_telemetry_snapshot(self):
        with tempfile.TemporaryDirectory() as directory:
            root, manifest = self.prepare(directory)
            sources = [Path(cohort['telemetry']['path'])
                       for cohort in manifest['qualification']['cohorts']]
            expected = sources[0].read_bytes()
            replacement = json.dumps({**json.loads(expected), 'unverified_substitution': True}).encode()
            consumed_controls, consumed_snapshots = [], []
            original_records = preview_qualification.presentation_records

            def controls(records, plan, report, endpoints):
                consumed_controls.append(list(records))
                sources[0].write_bytes(replacement)
                return {'valid': True, 'qualified': True,
                        'latency_us': {'p95': 50000, 'worst': 75000}}

            def presentations(filename, completed):
                self.assertNotIn(filename, sources)
                self.assertEqual(filename.parent.parent, sources[len(consumed_snapshots)].parent)
                self.assertEqual(filename.read_bytes(), expected)
                self.assertEqual(filename.stat().st_mode & 0o222, 0)
                consumed_snapshots.append(filename)
                sources[0].write_bytes(expected)
                return original_records(filename, completed)

            with self.analyses(), mock.patch.object(preview_qualification, 'summarize_controls',
                    side_effect=controls), mock.patch.object(preview_qualification, 'presentation_records',
                    side_effect=presentations):
                self.assertTrue(self.verify(root, manifest)['passed'])
            self.assertEqual(consumed_controls, [[json.loads(expected)]] * 3)
            self.assertEqual(len(consumed_snapshots), 3)
            self.assertTrue(all(not filename.exists() for filename in consumed_snapshots))

    def test_private_telemetry_snapshot_is_removed_when_analysis_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            root, manifest = self.prepare(directory)
            entry = manifest['qualification']['cohorts'][0]['telemetry']
            selected = Path(entry['path'])
            with self.analyses(), mock.patch.object(preview_qualification, 'summarize_controls',
                    side_effect=ValueError('synthetic analysis failure')), \
                    self.assertRaisesRegex(ValueError, 'synthetic analysis failure'):
                self.verify(root, manifest)
            self.assertEqual(list(selected.parent.glob('.verified-*')), [])

    def test_final_identity_check_covers_earlier_evidence_and_all_retained_inputs(self):
        for variant in ('build_receipt', 'earlier_result', 'earlier_telemetry', 'frozen_input'):
            with self.subTest(variant=variant), tempfile.TemporaryDirectory() as directory:
                root, manifest = self.prepare(directory)
                qualification = manifest['qualification']
                if variant == 'build_receipt':
                    selected = Path(qualification['build_receipt']['path'])
                elif variant in ('earlier_result', 'earlier_telemetry'):
                    selected = Path(qualification['cohorts'][0][variant.removeprefix('earlier_')]['path'])
                else:
                    first = json.loads(Path(qualification['cohorts'][0]['result']['path']).read_bytes())
                    selected = Path(first['inputs']['runner']['path'])
                with self.analyses(), self.assertRaisesRegex(ValueError, 'changed during verification'):
                    original = preview_qualification.summarize_presentations.side_effect
                    def mutate_after_analysis(records, required):
                        summary = original(records, required)
                        if original.calls == 3:
                            selected.write_text('changed after its final analysis')
                        return summary
                    preview_qualification.summarize_presentations.side_effect = mutate_after_analysis
                    self.verify(root, manifest)

    def test_process_envelopes_cover_loading_and_exit_not_only_helper_commands(self):
        for variant in ('wall_overlap', 'host_overlap', 'missing', 'unsupported_clock'):
            with self.subTest(variant=variant), tempfile.TemporaryDirectory() as directory:
                root, manifest = self.prepare(directory)
                def change(report):
                    lifecycle = report['process_lifecycle']
                    if variant == 'wall_overlap':
                        lifecycle['started_unix_ns'] -= 10_000_000_000
                    elif variant == 'host_overlap':
                        lifecycle['started_host_ns'] -= 10_000_000_000
                    elif variant == 'missing':
                        report.pop('process_lifecycle')
                    else:
                        lifecycle['host_clock'] = 'unknown epoch'
                self.rewrite_result(manifest, 1, change)
                with self.analyses(), self.assertRaises((KeyError, ValueError)):
                    self.verify(root, manifest)

    def test_validated_presentation_lifecycles_must_fit_inside_process_envelopes(self):
        with tempfile.TemporaryDirectory() as directory:
            root, manifest = self.prepare(directory)
            self.rewrite_result(manifest, 1, lambda report:
                report['process_lifecycle'].update(finished_host_ns=2_900_000_000_000))
            with self.analyses(), self.assertRaisesRegex(ValueError, 'outside.*process envelope'):
                self.verify(root, manifest)

    def test_validated_presentation_lifecycles_must_not_overlap(self):
        with tempfile.TemporaryDirectory() as directory:
            root, manifest = self.prepare(directory)
            self.rewrite_result(manifest, 0, lambda report:
                report['process_lifecycle'].update(finished_host_ns=2_002_000_000_000))
            self.rewrite_result(manifest, 1, lambda report:
                report['process_lifecycle'].update(started_host_ns=2_003_000_000_000))
            with self.analyses(), self.assertRaisesRegex(ValueError, 'outside.*process envelope|lifecycle envelopes overlap'):
                original = preview_qualification.summarize_presentations.side_effect
                def overlapping_snapshot(records, required):
                    summary = original(records, required)
                    if original.calls == 1:
                        summary['lifecycle_host_seconds']['snapshot'] = 2001.5
                    return summary
                preview_qualification.summarize_presentations.side_effect = overlapping_snapshot
                self.verify(root, manifest)

    def test_absent_or_changed_evidence_fails_before_analysis(self):
        for variant in ('missing', 'changed'):
            with self.subTest(variant=variant), tempfile.TemporaryDirectory() as directory:
                root, manifest = self.prepare(directory)
                filename = Path(manifest['qualification']['cohorts'][0]['telemetry']['path'])
                if variant == 'missing':
                    filename.unlink()
                else:
                    filename.write_text('changed')
                with self.analyses(), self.assertRaises((OSError, ValueError)):
                    self.verify(root, manifest)

    def test_receipt_groups_are_not_counted_as_duplicate_step_completions(self):
        with tempfile.TemporaryDirectory() as directory:
            root, manifest = self.prepare(directory)
            self.rewrite_result(manifest, 0, lambda report: report['journal'].insert(1,
                {'step': 'ready', 'input_receipts': [1, 2]}))
            with self.analyses():
                self.assertTrue(self.verify(root, manifest)['passed'])

    def test_completed_states_must_match_actual_raw_draws(self):
        with tempfile.TemporaryDirectory() as directory:
            root, manifest = self.prepare(directory)
            self.rewrite_result(manifest, 1, lambda report:
                report['journal'][-1]['state']['value'].update(tick=99))
            with self.analyses(), self.assertRaisesRegex(ValueError, 'actual raw draw'):
                self.verify(root, manifest)

    def test_failed_unrelated_changed_or_incomplete_cohorts_are_rejected(self):
        changes = [lambda report: report.update(passed=False),
                   lambda report: report['inputs']['load'].update(sha256='0'*64, final_sha256='0'*64),
                   lambda report: report['inputs']['runner'].update(final_sha256='0'*64),
                   lambda report: report['journal'].pop(),
                   lambda report: report['application_command'].__setitem__(2, '/unrelated')]
        for change in changes:
            with tempfile.TemporaryDirectory() as directory:
                root, manifest = self.prepare(directory)
                self.rewrite_result(manifest, 1, change)
                with self.analyses(), self.assertRaises(ValueError):
                    self.verify(root, manifest)

    def test_nonconsecutive_duplicate_and_overlapping_cohorts_are_rejected(self):
        for variant in ('missing-middle', 'duplicate', 'overlap'):
            with self.subTest(variant=variant), tempfile.TemporaryDirectory() as directory:
                root, manifest = self.prepare(directory)
                cohorts = manifest['qualification']['cohorts']
                if variant == 'missing-middle':
                    cohorts[1], cohorts[2] = cohorts[2], cohorts[1]
                elif variant == 'duplicate':
                    cohorts[1] = copy.deepcopy(cohorts[0])
                else:
                    self.rewrite_result(manifest, 1, lambda report:
                        report['process_lifecycle'].update(started_unix_ns=1_000_000_000_000_000_999))
                with self.analyses(), self.assertRaises(ValueError):
                    self.verify(root, manifest)

    def test_actual_long_and_latency_gates_are_required(self):
        for presentation, controls in (({'ready_presented_duration_seconds': 899.99}, None),
                ({'ready_terminal_gap_us': 250001}, None),
                ({'ready_presented_interval_us': {'samples': 54000, 'p95': 20001, 'p99': 33400}}, None),
                (None, {'valid': True, 'qualified': False})):
            with tempfile.TemporaryDirectory() as directory:
                root, manifest = self.prepare(directory)
                with self.analyses(presentation, controls), self.assertRaises(ValueError):
                    self.verify(root, manifest)

    def test_ready_draw_gates_use_raw_ready_intervals_and_retain_hitches(self):
        records = [{'kind': 'frame', 'data': {'frame_id': index + 1, 'draw_interval_us': interval,
                    'state': {'value': {'tick': None if index == 0 else index}}}}
                   for index, interval in enumerate([900000] + [8333] * 100 + [120000])]
        summary = preview_qualification.ready_draw_summary(records)
        self.assertEqual(summary['interval_us']['samples'], 101)
        self.assertEqual(summary['interval_us']['worst'], 120000)
        self.assertEqual(summary['gaps_over_100ms'], [{'frame_id': 102, 'tick': 101, 'interval_us': 120000}])
        for interval in (None, True, -1, 250001):
            records[-1]['data']['draw_interval_us'] = interval
            with self.subTest(interval=interval), self.assertRaises(ValueError):
                preview_qualification.ready_draw_summary(records)


if __name__ == '__main__':
    unittest.main()
