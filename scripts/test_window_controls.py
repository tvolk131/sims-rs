import copy
import json
from pathlib import Path
import tempfile
import unittest

from window_controls import summarize_controls, summarize_run, validate_endpoints
from window_session import file_digest


def fixture(click=False):
    conditions = [{'path': 'camera', 'op': 'eq', 'value': 1},
                  {'path': 'tick', 'op': 'eq', 'ref': 'initial.tick'}]
    action = {'kind': 'click', 'point': [20, 30]} if click else {'kind': 'key', 'code': 124}
    plan = {'schema': 1, 'steps': [{'name': 'remember', 'remember': 'initial'},
                                  {'name': 'change', 'input': action},
                                  {'name': 'changed', 'wait': conditions}]}
    endpoints = {'schema': 1, 'plan_sha256': '0' * 64,
                 'controls': [{'step': 'change', 'conditions': conditions}]}
    def frame(identifier, timestamp, sequence, camera, fence):
        return {'kind': 'frame', 'data': {'frame_id': identifier, 'draw_started_unix_ns': timestamp,
                'input_sequence': sequence, 'fence': fence, 'state': {'state_id': camera + 1,
                'value': {'tick': 10, 'camera': camera}}}}
    def receipt(sequence, timestamp, value):
        return {'kind': 'input', 'data': {'fence': 7, 'received_unix_ns': timestamp,
                                         'input_sequence': sequence, 'input': value}}
    baseline = frame(1, 100, 1, 0, 7)
    transient = frame(2, 125, 3 if click else 2, 1, 7)
    closing = frame(3, 160, 4 if click else 3, 1, 8)
    inputs = ([receipt(2, 110, {'kind': 'pointer_moved', 'point': [20, 30]}),
               receipt(3, 120, {'kind': 'button_pressed', 'button': 'Left'}),
               receipt(4, 140, {'kind': 'button_released', 'button': 'Left'})] if click else
              [receipt(2, 110, {'kind': 'key_pressed', 'physical_key': 'Code(ArrowRight)'}),
               receipt(3, 140, {'kind': 'key_released', 'physical_key': 'Code(ArrowRight)'})])
    records = [baseline] + inputs[:-1] + [transient, inputs[-1], closing, {'kind': 'session_end', 'data': {}}]
    group = {'step': 'change', 'input_receipts': [1, 2, 4] if click else [1, 3],
             'receipt_input_sequences': [2, 3, 4] if click else [2, 3],
             'input_started_unix_ns': 105, 'input_finished_unix_ns': 150, 'fence': 7}
    report = {'passed': True, 'graceful_end': True, 'journal': [
        {'step': 'remember', 'passed': True, 'frame_id': 1, 'state': baseline['data']['state']}, group,
        {'step': 'change', 'passed': True, 'frame_id': 3, 'state': closing['data']['state']}]}
    return records, plan, report, endpoints


class ControlTimingTests(unittest.TestCase):
    def test_key_press_can_match_an_exact_transient_before_key_release(self):
        result = summarize_controls(*fixture())
        self.assertTrue(result['qualified'])
        self.assertEqual(result['samples'][0]['effect']['frame_id'], 2)
        self.assertAlmostEqual(result['latency_us']['worst'], .015)

    def test_click_starts_at_release_not_earlier_button_press_or_effect(self):
        result = summarize_controls(*fixture(click=True))
        self.assertEqual(result['samples'][0]['effect']['frame_id'], 3)
        self.assertAlmostEqual(result['latency_us']['worst'], .020)

    def test_stale_wrong_key_wrong_fence_and_wrong_sequence_receipts_fail(self):
        for field, value in (('received_unix_ns', 105), ('fence', 6), ('input_sequence', 1),
                             ('input', {'kind': 'key_pressed', 'physical_key': 'Code(ArrowLeft)'})):
            with self.subTest(field=field):
                records, plan, report, endpoints = fixture()
                records[1]['data'][field] = value
                with self.assertRaises(ValueError):
                    summarize_controls(records, plan, report, endpoints)

    def test_unacknowledged_effect_draw_is_not_credited(self):
        records, plan, report, endpoints = fixture()
        records[2]['data']['input_sequence'] = 1
        result = summarize_controls(records, plan, report, endpoints)
        self.assertEqual(result['samples'][0]['effect']['frame_id'], 3)

    def test_effect_after_the_next_command_cannot_rescue_an_input(self):
        records, plan, report, endpoints = fixture()
        records[2]['data']['state']['value']['camera'] = 0
        records[4]['data']['draw_started_unix_ns'] = 152
        records[4]['data']['state'] = {'state_id': 1, 'value': {'tick': 10, 'camera': 0}}
        report['journal'][2]['state'] = records[4]['data']['state']
        baseline = copy.deepcopy(records[4])
        baseline['data'].update(frame_id=4, draw_started_unix_ns=153, fence=9)
        second_effect = copy.deepcopy(records[2])
        second_effect['data'].update(frame_id=5, draw_started_unix_ns=160, fence=10, input_sequence=5)
        second_effect['data']['state'] = {'state_id': 2, 'value': {'tick': 10, 'camera': 1}}
        second_close = copy.deepcopy(baseline)
        second_close['data'].update(frame_id=6, draw_started_unix_ns=165, fence=11, input_sequence=5)
        records[-1:] = [baseline,
            {'kind': 'input', 'data': {'fence': 9, 'received_unix_ns': 156, 'input_sequence': 4,
             'input': {'kind': 'key_pressed', 'physical_key': 'Code(ArrowLeft)'}}},
            {'kind': 'input', 'data': {'fence': 9, 'received_unix_ns': 157, 'input_sequence': 5,
             'input': {'kind': 'key_released', 'physical_key': 'Code(ArrowLeft)'}}},
            second_effect, second_close, {'kind': 'session_end', 'data': {}}]
        plan['steps'] += [{'name': 'second', 'input': {'kind': 'key', 'code': 123}}]
        endpoints['controls'].append({'step': 'second', 'conditions': [{'path': 'camera', 'op': 'eq', 'value': 0}]})
        report['journal'] += [{'step': 'second', 'input_receipts': [6, 7], 'receipt_input_sequences': [4, 5],
                               'input_started_unix_ns': 155, 'input_finished_unix_ns': 158, 'fence': 9},
                              {'step': 'second', 'passed': True, 'frame_id': 6, 'state': second_close['data']['state']}]
        with self.assertRaisesRegex(ValueError, 'Incomplete receipt/effect'):
            summarize_controls(records, plan, report, endpoints)

    def test_already_true_effect_cannot_be_presented_as_response(self):
        records, plan, report, endpoints = fixture()
        records[0]['data']['state']['value']['camera'] = 1
        with self.assertRaisesRegex(ValueError, 'already satisfied'):
            summarize_controls(records, plan, report, endpoints)

    def test_missing_closing_acknowledgment_and_overwritten_memory_fail(self):
        for missing_close in (True, False):
            records, plan, report, endpoints = fixture()
            report = copy.deepcopy(report)
            if missing_close:
                records[4]['data']['input_sequence'] = 2
            else:
                report['journal'][0]['state']['value']['tick'] = 99
            with self.subTest(missing_close=missing_close), self.assertRaises(ValueError):
                summarize_controls(records, plan, report, endpoints)

    def test_losses_failed_sessions_missing_end_and_errors_are_fatal(self):
        for variant in ('loss', 'failed', 'missing_end', 'error'):
            records, plan, report, endpoints = fixture()
            if variant == 'loss':
                records[0]['dropped_records'] = 1
            elif variant == 'failed':
                report['passed'] = False
            elif variant == 'missing_end':
                records.pop()
            else:
                records[0]['data']['state']['value']['error'] = 'synthetic failure'
            with self.subTest(variant=variant), self.assertRaises(ValueError):
                summarize_controls(records, plan, report, endpoints)

    def test_missing_duplicate_and_forward_referencing_endpoints_are_rejected(self):
        for variant in ('missing', 'duplicate', 'future'):
            records, plan, report, endpoints = fixture()
            if variant == 'missing':
                endpoints['controls'].clear()
            elif variant == 'duplicate':
                endpoints['controls'] *= 2
            else:
                endpoints['controls'][0]['conditions'] = [{'path': 'tick', 'op': 'eq', 'ref': 'future.tick'}]
            with self.subTest(variant=variant), self.assertRaises(ValueError):
                validate_endpoints(plan, endpoints)

    def test_unqualified_slow_latency_is_not_a_passing_distribution(self):
        records, plan, report, endpoints = fixture()
        for record in records:
            if record['kind'] == 'frame' and record['data']['frame_id'] > 1:
                record['data']['draw_started_unix_ns'] += 300_000_000
        result = summarize_controls(records, plan, report, endpoints)
        self.assertTrue(result['valid'])
        self.assertFalse(result['qualified'])
        self.assertGreater(result['latency_us']['worst'], 250_000)

    def test_latency_gates_are_independent_and_accept_exact_boundaries(self):
        for latencies, qualified, expected_p95, expected_max in (
                ([110_000] * 20, False, 110_000, 110_000),
                ([50_000] * 19 + [250_001], False, 50_000, 250_001),
                ([100_000] * 19 + [250_000], True, 100_000, 250_000)):
            records, plan, report, endpoints = [], {'schema': 1, 'steps': []}, {
                'passed': True, 'graceful_end': True, 'journal': []}, {
                'schema': 1, 'plan_sha256': '0' * 64, 'controls': []}
            for index, latency in enumerate(latencies):
                segment, segment_plan, segment_report, segment_endpoints = fixture()
                effect_time = 110 + latency * 1000
                segment[2]['data']['draw_started_unix_ns'] = effect_time
                segment[3]['data']['received_unix_ns'] = effect_time + 1
                segment[4]['data']['draw_started_unix_ns'] = effect_time + 20
                segment_report['journal'][1]['input_finished_unix_ns'] = effect_time + 10
                for record in segment[:-1]:
                    payload = record['data']
                    payload['fence'] += index * 2
                    payload['input_sequence'] += index * 3
                    if record['kind'] == 'frame':
                        payload['frame_id'] += index * 3
                        payload['draw_started_unix_ns'] += index * 1_000_000_000
                    else:
                        payload['received_unix_ns'] += index * 1_000_000_000
                for item in segment_report['journal']:
                    item['step'] += str(index)
                    if 'frame_id' in item:
                        item['frame_id'] += index * 3
                    else:
                        item['input_receipts'] = [value + len(records) for value in item['input_receipts']]
                        item['receipt_input_sequences'] = [value + index * 3 for value in item['receipt_input_sequences']]
                        item['fence'] += index * 2
                        for field in ('input_started_unix_ns', 'input_finished_unix_ns'):
                            item[field] += index * 1_000_000_000
                for step in segment_plan['steps']:
                    step['name'] += str(index)
                    if 'remember' in step:
                        step['remember'] += str(index)
                conditions = segment_endpoints['controls'][0]['conditions']
                conditions[1]['ref'] = f'initial{index}.tick'
                segment_plan['steps'][2]['wait'] = conditions
                segment_endpoints['controls'][0]['step'] += str(index)
                records.extend(segment[:-1])
                plan['steps'].extend(segment_plan['steps'])
                report['journal'].extend(segment_report['journal'])
                endpoints['controls'].extend(segment_endpoints['controls'])
            records.append({'kind': 'session_end', 'data': {}})
            with self.subTest(latencies=latencies):
                result = summarize_controls(records, plan, report, endpoints)
                self.assertTrue(result['valid'])
                self.assertEqual(result['qualified'], qualified)
                self.assertEqual(result['latency_us']['p95'], expected_p95)
                self.assertEqual(result['latency_us']['worst'], expected_max)

    def test_cli_input_identity_is_frozen_and_reported(self):
        records, plan, report, endpoints = fixture()
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            (directory / 'telemetry').mkdir()
            plan_file = directory / 'plan.json'
            plan_file.write_text(json.dumps(plan))
            endpoints['plan_sha256'] = file_digest(plan_file)
            endpoint_file = directory / 'endpoints.json'
            endpoint_file.write_text(json.dumps(endpoints))
            report['inputs'] = {'plan': {'path': str(plan_file), 'sha256': file_digest(plan_file),
                                         'final_sha256': file_digest(plan_file)}}
            (directory / 'result.json').write_text(json.dumps(report))
            (directory / 'telemetry/events.jsonl').write_text(''.join(json.dumps(record) + '\n' for record in records))
            result = summarize_run(directory, endpoint_file)
            self.assertTrue(result['qualified'])
            self.assertEqual(result['identities']['endpoints_sha256'], file_digest(endpoint_file))
            plan_file.write_text('{}')
            with self.assertRaisesRegex(ValueError, 'identity mismatch'):
                summarize_run(directory, endpoint_file)


if __name__ == '__main__':
    unittest.main()
