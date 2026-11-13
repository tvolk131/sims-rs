"""Synthetic journal tests only; no native APIs, processes, assets or saves."""
import copy
import json
import unittest

import window_evidence as evidence


BASE = 1_000_000_000_000
ACTION = {'kind': 'key', 'code': 124, 'press_ms': 20}


def state(center=None, **changes):
    value = {'tick': 192923, 'playing': False, 'pending': False, 'clock_pending': False,
             'presentation_children_pending': False, 'modal': False,
             'camera': {'center': [0, 0] if center is None else center}, 'display_size': [800, 600],
             'terminals': None, 'hud_normal_ready': None, 'hud_transport_ready': None,
             'observation_capabilities': dict(evidence.CAPABILITIES), 'error': None, 'menu_error': None}
    value.update(changes)
    return value


def record(kind, elapsed, data):
    return {'kind': kind, 'elapsed_us': elapsed, 'dropped_records': 0, 'data': data}


def frame(identifier, elapsed, center=None, sequence=0, fence=1, **changes):
    return record('frame', elapsed, {'frame_id': identifier, 'fence': fence, 'input_sequence': sequence,
        'draw_started_unix_ns': BASE + elapsed * 1000, 'cursor': None, 'widget_draw_cpu_us': 50,
        'draw_interval_us': None, 'state': {'state_id': 1 if center is None else 2,
            'value': state(center, **changes), 'diagnostics': {'clock': None, 'worker_busy': False}}})


def records():
    return [record('session', 0, {'schema': 1, 'pid': 42, 'unix_ns': str(BASE)}),
            frame(1, 1000),
            record('input', 3000, {'fence': 1, 'input_sequence': 1,
                'received_unix_ns': BASE + 3_000_000,
                'input': {'kind': 'key_pressed', 'physical_key': 'Code(ArrowRight)'}}),
            frame(2, 13000, [1, 0], sequence=1),
            record('input', 23000, {'fence': 1, 'input_sequence': 2,
                'received_unix_ns': BASE + 23_000_000,
                'input': {'kind': 'key_released', 'physical_key': 'Code(ArrowRight)'}}),
            frame(3, 24000, [1, 0], sequence=2),
            record('session_end', 26000, {'dropped_records': 0})]


def payload(items):
    return b''.join(json.dumps(item).encode() + b'\n' for item in items)


def control(items, **changes):
    options = {'action': ACTION, 'conditions': evidence.CAMERA_CONDITIONS,
               'remembered': {'baseline': state()}, 'fence': 1,
               'started_unix_ns': BASE + 2_000_000, 'finished_unix_ns': BASE + 22_000_000,
               'next_started_unix_ns': BASE + 25_000_000}
    options.update(changes)
    return evidence.control_effect(items, **options)


def binding():
    return {'schema': 'sims-window-binding-v1', 'pid': 42, 'window_id': 71,
            'executable': '/private/fake', 'launch_unix_seconds': 1000.0,
            'frontmost': True, 'on_screen': True, 'bounds': [10, 10, 800, 625],
            'display_scope': 'Current CoreGraphics mode and NSScreen maximum; not instantaneous refresh',
            'displays': [{'display_id': 1, 'mode_id': 2, 'width': 1920, 'height': 1080,
                'pixel_width': 3840, 'pixel_height': 2160, 'nominal_refresh_hz': 0,
                'maximum_frames_per_second': 120, 'builtin': True, 'active': True, 'asleep': False}]}


class EvidenceTests(unittest.TestCase):
    def test_concurrent_span_enqueue_keeps_original_timestamps_and_control_latency(self):
        items = records()
        items.insert(2, record('span_begin', 2000, {'span_id': 1, 'name': 'authored-worker'}))
        items.insert(4, record('span_end', 13001, {'span_id': 1, 'name': 'authored-worker', 'wall_us': 11001}))
        parsed = evidence.journal(payload(items), 42)
        self.assertEqual(parsed, items)
        self.assertEqual(control(parsed)['latency_us']['p95'], 10_000)

    def test_ui_sequence_span_causality_and_terminal_time_stay_fail_closed(self):
        items = records()
        items[3]['elapsed_us'] = 2999
        with self.assertRaises(ValueError):
            evidence.journal(payload(items), 42)
        items = records()
        items.insert(2, record('span_begin', 5000, {'span_id': 1, 'name': 'authored-worker'}))
        items.insert(3, record('span_end', 4999, {'span_id': 1, 'name': 'authored-worker', 'wall_us': 1}))
        with self.assertRaises(ValueError):
            evidence.journal(payload(items), 42)
        items = records()
        items[-1]['elapsed_us'] = 23_999
        with self.assertRaises(ValueError):
            evidence.journal(payload(items), 42)

    def test_complete_journal_and_named_metric(self):
        items = evidence.journal(payload(records()), 42)
        result = control(items)
        self.assertTrue(result['complete'])
        self.assertEqual(result['origin'], evidence.METRIC_ORIGIN)
        self.assertEqual(result['latency_us']['p95'], 10_000)
        self.assertTrue(result['within_fixed_limits'])
        self.assertIn('gesture duration is not subtracted', result['scope'])

    def test_missing_duplicate_and_post_end_records_rejected(self):
        original = records()
        for items in (original[:-1], original + [original[-1]], original + [original[1]],
                      original + [record('span_end', 27000, {})], original[1:]):
            with self.subTest(items=len(items)), self.assertRaises(ValueError):
                evidence.journal(payload(items), 42)

    def test_malformed_partial_duplicate_json_and_constants(self):
        for bad in (b'', b'{}\n', b'\xff\n', payload(records())[:-1], b'{"a":1,"a":2}\n',
                    b'{"bad":NaN}\n', b'[]\n', b'\n', b'[' * 2000 + b'\n'):
            with self.subTest(bad=bad[:20]), self.assertRaises(ValueError):
                evidence.journal(bad, 42)

    def test_incomplete_incremental_tail_is_not_terminal_proof(self):
        data = payload(records()[:-1]) + b'{"kind":"sess'
        self.assertEqual(len(evidence.journal(data, 42, complete=False)), 6)
        with self.assertRaises(ValueError):
            evidence.journal(data, 42)

    def test_exponent_overflow_is_rejected_in_arbitrary_json_fields(self):
        for value in (b'{"value":1e400}', b'[-1e400]',
                      b'{"state":{"diagnostics":{"overflow":1e400}}}'):
            with self.subTest(value=value), self.assertRaises(ValueError):
                evidence.parse_json(value)
        self.assertEqual(evidence.parse_json(b'{"value":1e300}'), {'value': 1e300})

    def test_journal_rejects_exponent_overflow_inside_diagnostics(self):
        items = records()
        items[3]['data']['state']['diagnostics']['overflow'] = 0.25
        encoded = payload(items)
        overflowing = encoded.replace(b'"overflow": 0.25', b'"overflow": 1e400')
        self.assertNotEqual(encoded, overflowing)
        with self.assertRaises(ValueError):
            evidence.journal(overflowing, 42)

    def test_valid_final_draw_cannot_hide_an_unusable_effect_draw(self):
        for camera in (None, {}, {'center': None}, {'center': [1]}, {'center': [1, 'bad']}):
            items = records()
            items[3]['data']['state']['value']['camera'] = camera
            items[5]['data']['state'] = {
                'state_id': 3, 'value': state([0, 0]),
                'diagnostics': {'clock': None, 'worker_busy': False},
            }
            parsed = evidence.journal(payload(items), 42)
            with self.subTest(camera=camera), self.assertRaises(ValueError):
                control(parsed)

    def test_control_window_requires_usable_baseline_effect_and_closing_draws(self):
        for index in (1, 3, 5):
            for flag in ('playing', 'pending', 'clock_pending', 'presentation_children_pending', 'modal'):
                items = records()
                items[index]['data']['state']['value'][flag] = True
                with self.subTest(index=index, flag=flag), self.assertRaises(ValueError):
                    control(items)

    def test_drops_errors_wrong_pid_and_stale_time_rejected(self):
        for mutate in (
            lambda items: items[0]['data'].update(pid=43),
            lambda items: items[0]['data'].update(schema=True),
            lambda items: items[2].update(dropped_records=1),
            lambda items: items[-1]['data'].update(dropped_records=1),
            lambda items: items[2].update(kind='host_error'),
            lambda items: items[2].update(kind='application_error'),
            lambda items: items[2].update(kind='presentation_error'),
            lambda items: items[2]['data'].update(received_unix_ns=BASE - 1),
            lambda items: items[2]['data'].update(input_sequence=2),
            lambda items: items[4]['data'].update(input_sequence=1),
            lambda items: items[3]['data'].update(input_sequence=0),
            lambda items: items[3]['data'].update(frame_id=8),
            lambda items: items[3]['data']['state']['value'].update(menu_error='synthetic error'),
            lambda items: items[3]['data']['state']['value'].update(shell={'error': 'synthetic'}),
            lambda items: items[3]['data']['state']['value'].update(shell=[]),
        ):
            items = records()
            mutate(items)
            with self.subTest(items=items[2]), self.assertRaises(ValueError):
                evidence.journal(payload(items), 42)

    def test_null_capabilities_remain_explicitly_unsupported(self):
        items = evidence.journal(payload(records()), 42)
        result = evidence.summarize(items, control(items))
        for name in evidence.QUALIFICATIONS:
            self.assertIs(result[name], False)
        for name in ('full_readiness', 'eligible_seconds', 'civil_days', 'worker_quiescence'):
            self.assertIsNone(result[name])
        for name in ('presentation', 'native_post_to_receipt_delivery', 'input_to_photon'):
            self.assertIs(result[name]['available'], False)
        self.assertFalse(result['visual_proof'])
        for field, replacement in (('terminals', {}), ('hud_normal_ready', True),
                                   ('observation_capabilities', {**evidence.CAPABILITIES, 'action_terminals': True})):
            items = records()
            items[1]['data']['state']['value'][field] = replacement
            with self.subTest(field=field), self.assertRaises(ValueError):
                evidence.journal(payload(items), 42)

    def test_missing_capability_fields_are_not_null_proof(self):
        for field in ('terminals', 'hud_normal_ready', 'hud_transport_ready', 'observation_capabilities'):
            items = records()
            del items[1]['data']['state']['value'][field]
            with self.subTest(field=field), self.assertRaises(ValueError):
                evidence.journal(payload(items), 42)

    def test_immutable_state_identity_and_busy_diagnostics(self):
        items = records()
        items[3]['data']['state']['state_id'] = 1
        with self.assertRaises(ValueError):
            evidence.journal(payload(items), 42)
        items = records()
        items[5]['data']['state']['diagnostics']['worker_busy'] = True
        evidence.journal(payload(items), 42)
        self.assertIsNone(evidence.summarize(items, control(items))['worker_quiescence'])

    def test_fence_sequence_unrelated_and_already_true_rejected(self):
        for mutate in (
            lambda items: items[2]['data'].update(fence=2),
            lambda items: items[4]['data']['input'].update(physical_key='Code(ArrowLeft)'),
            lambda items: items[2]['data']['input'].update(kind='key_released'),
            lambda items: items[1]['data'].update(input_sequence=2),
            lambda items: items[3]['data'].update(fence=2),
        ):
            items = records()
            mutate(items)
            if items[3]['data']['fence'] == 2:
                items[5]['data']['fence'] = 2
            with self.subTest(items=items[2]), self.assertRaises(ValueError):
                control(items)
        with self.assertRaises(ValueError):
            control(records(), conditions=[{'path': 'camera.center', 'op': 'eq', 'value': [0, 0]}])

    def test_next_command_and_missing_release_cannot_close_effect(self):
        for options in ({'next_started_unix_ns': BASE + 13_000_000},
                        {'finished_unix_ns': BASE + 25_000_000}):
            with self.subTest(options=options), self.assertRaises(ValueError):
                control(records(), **options)
        with self.assertRaises(ValueError):
            control([item for item in records() if not (item['kind'] == 'input'
                    and item['data']['input']['kind'] == 'key_released')])

    def test_null_receipt_is_not_a_key_acknowledgment(self):
        items = records()
        items[2]['data']['input'] = None
        with self.assertRaises(ValueError):
            control(items)

    def test_fresh_draw_acknowledges_input_not_just_changed_state(self):
        items = records()
        for item in items:
            if item['kind'] == 'frame' and item['data']['frame_id'] > 1:
                item['data']['input_sequence'] = 0
        with self.assertRaises(ValueError):
            control(items)

    def test_nearest_rank_thresholds_are_fixed(self):
        self.assertEqual(evidence.distribution([None, 5, 1, 4, 3, 2])['p95'], 5)
        self.assertEqual(evidence.distribution(range(1, 21))['p95'], 19)
        self.assertTrue(evidence.metric_summary([100_000])['within_fixed_limits'])
        self.assertFalse(evidence.metric_summary([100_000.001])['within_fixed_limits'])
        self.assertTrue(evidence.metric_summary([50_000] * 19 + [250_000])['within_fixed_limits'])
        self.assertFalse(evidence.metric_summary([50_000] * 19 + [250_000.001])['within_fixed_limits'])
        self.assertFalse(evidence.metric_summary([])['within_fixed_limits'])
        for sample in (-1, float('nan'), float('inf'), True):
            with self.subTest(sample=sample), self.assertRaises(ValueError):
                evidence.distribution([sample])

    def test_tick_and_modal_are_not_full_or_camera_readiness(self):
        self.assertTrue(evidence.usable_camera(state()))
        for changes in ({'tick': None}, {'modal': True}, {'pending': True}, {'playing': True},
                        {'clock_pending': True}, {'camera': None}, {'presentation_children_pending': True}):
            self.assertFalse(evidence.usable_camera(state(**changes)))

    def test_phase_wall_time_does_not_become_eligibility(self):
        items = records()
        items[1]['data']['state']['value'].update(tick=None)
        items[3]['data']['state']['value'].update(modal=True)
        result = evidence.summarize(items, None)
        self.assertGreater(result['phase_wall_us_not_eligibility']['startup'], 0)
        self.assertGreater(result['phase_wall_us_not_eligibility']['modal'], 0)
        self.assertIsNone(result['eligible_seconds'])

    def test_unbound_presentation_rejected_not_clock_subtracted(self):
        items = records()
        items[2]['kind'] = 'presentation_event'
        with self.assertRaisesRegex(ValueError, 'Unbound'):
            evidence.journal(payload(items), 42)

    def test_display_validation_is_metadata_not_fps(self):
        evidence.validate_displays(binding())
        for mutate in (
            lambda value: value['displays'][0].update(asleep=True),
            lambda value: value['displays'][0].update(active=False),
            lambda value: value['displays'][0].update(maximum_frames_per_second=0),
            lambda value: value['displays'].append(copy.deepcopy(value['displays'][0])),
            lambda value: value.update(display_scope='fps'),
        ):
            value = binding()
            mutate(value)
            with self.subTest(value=value), self.assertRaises(ValueError):
                evidence.validate_displays(value)

    def test_lookup_and_predicates_preserve_reference_semantics(self):
        self.assertEqual(evidence.lookup({'items': [7]}, 'items.0'), 7)
        self.assertTrue(evidence.matches({'items': [{'name': 'camera'}]}, [
            {'path': 'items', 'op': 'any_match', 'value': [{'path': 'name', 'op': 'eq', 'value': 'camera'}]}], {}))
        self.assertFalse(evidence.matches({}, [{'path': 'missing', 'op': 'ne', 'value': 1}], {}))
        self.assertTrue(evidence.receipt_matches({'kind': 'pointer_moved', 'point': [10.5, 20]},
                                               {'kind': 'pointer_moved', 'point': [10, 20]}))
        self.assertFalse(evidence.receipt_matches({'kind': 'pointer_moved', 'point': [12, 20]},
                                                {'kind': 'pointer_moved', 'point': [10, 20]}))


def opening_dialog():
    return {'id': 7, 'caller': 12, 'source': '/authored/objects.far::Tutorial.iff',
            'origin': {'source': '/authored/objects.far::Tutorial.iff', 'bhav': 4097, 'instruction': 2},
            'status': 1, 'offered_results': [2], 'enter_result': 2,
            'escape_result': None, 'space_result': None}


def opening_answer():
    dialog = opening_dialog()
    return {**{name: dialog[name] for name in ('id', 'caller', 'source', 'origin')},
            'result': 2, 'status': 2, 'tick': 192923}


def tutorial_records():
    items = records()
    for item in items:
        data = item['data']
        if item['kind'] == 'input':
            data['input']['physical_key'] = 'Code(Enter)'
        if item['kind'] == 'frame':
            value = data['state']['value']
            value.update({'modal': data['frame_id'] != 3,
                          'dialog': opening_dialog() if data['frame_id'] != 3 else None,
                          'dialog_answer': opening_answer() if data['frame_id'] == 3 else None})
            data['state']['state_id'] = data['frame_id']
    items.insert(5, record('dialog_answer', 23500, opening_answer()))
    return items


def response(items, **changes):
    baseline = copy.deepcopy(items[1]['data'])
    options = {'reservation': {'identity': evidence.offered_opening(baseline['state']['value']),
                               'frame': baseline}, 'fence': 1,
               'started_unix_ns': BASE + 2_000_000, 'finished_unix_ns': BASE + 22_000_000,
               'next_started_unix_ns': BASE + 25_000_000}
    options.update(changes)
    return evidence.offered_response(items, **options)


class OfferedResponseTests(unittest.TestCase):
    def test_actual_no_prose_schema_acknowledges_once_without_control_gates(self):
        items = evidence.journal(payload(tutorial_records()), 42)
        result = response(items)
        self.assertTrue(result['complete'])
        self.assertEqual(result['origin'], evidence.RESPONSE_ORIGIN)
        self.assertEqual(result['acknowledgment_us']['p95'], 21000)
        for field in ('latency_us', 'p95_limit_us', 'maximum_limit_us', 'within_fixed_limits'):
            self.assertNotIn(field, result)
        self.assertNotIn('operands', result['reserved_observed_identity'])
        self.assertNotIn('stack_object', result['reserved_observed_identity'])

    def test_separately_named_response_is_not_camera_or_completion_evidence(self):
        items = tutorial_records()
        report = evidence.summarize(items, None, response(items))
        self.assertIsNone(report['control_effect'])
        self.assertTrue(report['offered_response_acknowledgment']['complete'])
        self.assertFalse(any(report[name] for name in evidence.QUALIFICATIONS))
        self.assertFalse(report['tutorial_completion_claimed'])
        self.assertFalse(report['first_tick_claimed'])
        self.assertFalse(report['presentation']['available'])

    def test_explicit_plan_and_bound_acknowledgment_only(self):
        plan = {'schema': 2, 'mode': evidence.TUTORIAL_MODE, 'timeout_seconds': 300,
                'household': 'newbie', 'steps': [{'name': 'tutorial-opening-enter',
                                                'input': dict(evidence.TUTORIAL_ACTION)}]}
        specification = {'schema': 2, 'plan_sha256': 'authored', 'controls': evidence.TUTORIAL_CONTROLS}
        evidence.validate_specification(plan, specification, 'authored')
        for change in ({'mode': 'camera-right'}, {'schema': True}, {'timeout_seconds': 301},
                       {'timeout_seconds': True}, {'household': 'goth'}, {'mode': None},
                       {'steps': plan['steps'] * 2}, {'unknown': True}):
            with self.subTest(change=change), self.assertRaises(ValueError):
                evidence.validate_specification({**plan, **change}, specification, 'authored')
        for field in ('code', 'press_ms'):
            altered = copy.deepcopy(plan)
            altered['steps'][0]['input'][field] = float(altered['steps'][0]['input'][field])
            with self.subTest(field=field), self.assertRaises(ValueError):
                evidence.validate_specification(altered, specification, 'authored')
        for change in ({'schema': 1}, {'plan_sha256': 'foreign'},
                       {'controls': [{'step': 'tutorial-opening-enter', 'conditions': []}]}):
            with self.subTest(change=change), self.assertRaises(ValueError):
                evidence.validate_specification(plan, {**specification, **change}, 'authored')

    def test_old_camera_plan_never_falls_back_to_enter(self):
        plan = {'schema': 1, 'timeout_seconds': 10, 'household': 'newbie',
                'steps': [{'name': 'camera-right', 'input': dict(evidence.TUTORIAL_ACTION)}]}
        specification = {'schema': 1, 'plan_sha256': 'authored',
                         'controls': [{'step': 'camera-right', 'conditions': evidence.CAMERA_CONDITIONS}]}
        with self.assertRaises(ValueError):
            evidence.validate_specification(plan, specification, 'authored')
        with self.assertRaises(ValueError):
            control(tutorial_records(), action=evidence.TUTORIAL_ACTION)
        with self.assertRaises(ValueError):
            control(tutorial_records())

    def test_only_explicit_supported_native_key_codes_and_exact_twenty_ms(self):
        for code in (0, 13, 35, 37, 123, True, 36.0):
            with self.subTest(code=code), self.assertRaises(ValueError):
                evidence.expected_receipts({**evidence.TUTORIAL_ACTION, 'code': code})
        for duration in (0, 19, 21, True, 20.0):
            with self.subTest(duration=duration), self.assertRaises(ValueError):
                evidence.expected_receipts({**evidence.TUTORIAL_ACTION, 'press_ms': duration})

    def test_opening_resource_namespace_matches_the_headless_selector(self):
        for source in ('Tutorial.iff', '/authored/Tutorial.iff', 'C:\\authored\\Tutorial.iff',
                       '/authored/objects.far::Tutorial.iff'):
            value = tutorial_records()[1]['data']['state']['value']
            value['dialog']['source'] = source
            value['dialog']['origin']['source'] = source
            self.assertEqual(evidence.offered_opening(value)['source'], source)

    def test_unrelated_changed_or_unoffered_opening_is_unsupported(self):
        for change in ({'status': 2}, {'status': True}, {'source': '/authored/Other.iff'},
                       {'offered_results': [2, 3]}, {'offered_results': [3]}, {'offered_results': [True]},
                       {'enter_result': None}, {'enter_result': 3}, {'id': True}, {'caller': 12.0},
                       {'operands': [0] * 8}, {'title': 'not published'}):
            value = tutorial_records()[1]['data']['state']['value']
            value['dialog'].update(change)
            with self.subTest(change=change), self.assertRaises(ValueError):
                evidence.offered_opening(value)
        for change in ({'bhav': 4098}, {'instruction': 3}, {'source': 'Other.iff'}, {'stack_object': 12}):
            value = tutorial_records()[1]['data']['state']['value']
            value['dialog']['origin'].update(change)
            with self.subTest(change=change), self.assertRaises(ValueError):
                evidence.offered_opening(value)

    def test_absent_observation_is_unsupported_not_fabricated(self):
        for field in ('dialog', 'dialog_answer'):
            value = tutorial_records()[1]['data']['state']['value']
            value.pop(field)
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, 'Unsupported'):
                evidence.offered_opening(value)

    def test_submission_or_press_alone_is_not_acknowledgment(self):
        items = tutorial_records()[:4]
        self.assertFalse(response(items, complete=False)['complete'])
        with self.assertRaises(ValueError):
            response(items)
        items = tutorial_records()
        items[6]['data']['state']['value'] = copy.deepcopy(items[1]['data']['state']['value'])
        self.assertFalse(response(items, complete=False)['complete'])

    def test_matching_result_requires_actual_status_two_and_modal_release(self):
        for change in ({'status': 1}, {'status': 3}, {'status': 5}, {'result': 3},
                       {'id': 8}, {'caller': 13}, {'source': 'Other.iff'}, {'tick': 192924}):
            items = tutorial_records()
            items[6]['data']['state']['value']['dialog_answer'].update(change)
            with self.subTest(change=change), self.assertRaises(ValueError):
                response(items)
        for change in ({'dialog_answer': None}, {'dialog': opening_dialog()}, {'modal': True}):
            items = tutorial_records()
            items[6]['data']['state']['value'].update(change)
            with self.subTest(change=change), self.assertRaises(ValueError):
                response(items)

    def test_changed_origin_or_hidden_answer_fields_never_pass(self):
        for change in ({'bhav': 4098}, {'instruction': 3}, {'source': 'Other.iff'}):
            items = tutorial_records()
            items[6]['data']['state']['value']['dialog_answer']['origin'].update(change)
            with self.subTest(change=change), self.assertRaises(ValueError):
                response(items)
        items = tutorial_records()
        items[6]['data']['state']['value']['dialog_answer']['stack_object'] = 12
        with self.assertRaises(ValueError):
            evidence.journal(payload(items), 42)

    def test_intervening_draw_must_match_reserved_current_identity(self):
        items = tutorial_records()
        reservation = {'identity': evidence.offered_opening(items[1]['data']['state']['value']),
                       'frame': copy.deepcopy(items[1]['data'])}
        extra = copy.deepcopy(items[1])
        extra['data']['frame_id'] = 99
        items.insert(2, extra)
        with self.assertRaisesRegex(ValueError, 'immediately preceding'):
            response(items, reservation=reservation)

    def test_wrong_fence_reordered_receipts_and_extra_enter_are_rejected(self):
        for index, field, value in ((2, 'fence', 2), (4, 'fence', 2), (3, 'fence', 2),
                                    (6, 'fence', 2), (2, 'input_sequence', 2), (4, 'input_sequence', 3)):
            items = tutorial_records()
            items[index]['data'][field] = value
            with self.subTest(index=index, field=field), self.assertRaises(ValueError):
                response(items)
        for index in (2, 4):
            items = tutorial_records()
            items[index]['data']['input']['physical_key'] = 'Code(ArrowRight)'
            with self.subTest(index=index), self.assertRaises(ValueError):
                response(items)
        items = tutorial_records()
        items.insert(5, copy.deepcopy(items[2]))
        with self.assertRaises(ValueError):
            response(items)

    def test_acknowledgment_draw_must_follow_release_and_helper_return(self):
        for change in ({'finished_unix_ns': BASE + 24_000_000},
                       {'next_started_unix_ns': BASE + 24_000_000}):
            with self.subTest(change=change), self.assertRaises(ValueError):
                response(tutorial_records(), **change)
        items = tutorial_records()
        items[6]['data']['input_sequence'] = 1
        self.assertFalse(response(items, complete=False)['complete'])

    def test_response_timing_never_substitutes_camera_performance_gates(self):
        items = tutorial_records()
        items[6]['elapsed_us'] = 500000
        items[6]['data']['draw_started_unix_ns'] = BASE + 500_000_000
        items[7]['elapsed_us'] = 600000
        parsed = evidence.journal(payload(items), 42)
        result = response(parsed, next_started_unix_ns=BASE + 550_000_000)
        self.assertTrue(result['complete'])
        self.assertGreater(result['acknowledgment_us']['p95'], evidence.MAXIMUM_LIMIT_US)
        self.assertNotIn('within_fixed_limits', result)

    def test_press_release_order_is_required_not_just_two_enter_receipts(self):
        for index, kind in ((2, 'key_released'), (4, 'key_pressed')):
            items = tutorial_records()
            items[index]['data']['input']['kind'] = kind
            with self.subTest(index=index), self.assertRaises(ValueError):
                response(items)

    def test_already_answered_or_modal_false_is_never_reserved(self):
        for change in ({'dialog_answer': opening_answer()}, {'modal': False}, {'dialog': None}):
            value = tutorial_records()[1]['data']['state']['value']
            value.update(change)
            with self.subTest(change=change), self.assertRaises(ValueError):
                evidence.offered_opening(value)

    def test_draw_answer_without_the_actual_emitted_answer_event_is_not_proof(self):
        items = tutorial_records()
        items.pop(5)
        self.assertFalse(response(items, complete=False)['complete'])
        with self.assertRaises(ValueError):
            response(items)

    def test_actual_answer_event_must_match_reserved_identity_result_and_status(self):
        for change in ({'id': 8}, {'caller': 13}, {'source': 'Other.iff'},
                       {'result': 3}, {'status': 5}, {'tick': 192924}):
            items = tutorial_records()
            items[5]['data'].update(change)
            with self.subTest(change=change), self.assertRaises(ValueError):
                response(items)

    def test_answer_event_before_press_or_repeated_event_is_rejected(self):
        items = tutorial_records()
        answer = items.pop(5)
        items.insert(2, answer)
        with self.assertRaises(ValueError):
            response(items)
        items = tutorial_records()
        items.insert(6, copy.deepcopy(items[5]))
        with self.assertRaises(ValueError):
            response(items)

    def test_answer_event_may_precede_release_but_does_not_replace_it(self):
        items = tutorial_records()
        answer = items.pop(5)
        answer['elapsed_us'] = 4000
        items.insert(3, answer)
        parsed = evidence.journal(payload(items), 42)
        self.assertTrue(response(parsed)['complete'])
        items = parsed[:5]
        self.assertFalse(response(items, complete=False)['complete'])

    def test_answer_event_schema_and_ui_order_remain_fail_closed(self):
        for change in ({'operands': [0] * 8}, {'status': True}, {'caller': 12.0}):
            items = tutorial_records()
            items[5]['data'].update(change)
            with self.subTest(change=change), self.assertRaises(ValueError):
                evidence.journal(payload(items), 42)
        items = tutorial_records()
        items[5]['elapsed_us'] = 1
        with self.assertRaises(ValueError):
            evidence.journal(payload(items), 42)

    def test_next_modal_pending_playing_or_tick_change_stops_response(self):
        for change in ({'pending': True}, {'clock_pending': True}, {'presentation_children_pending': True},
                       {'playing': True}, {'tick': 192924}, {'error': 'authored failure'},
                       {'menu_error': 'authored failure'}):
            items = tutorial_records()
            items[3]['data']['state']['value'].update(change)
            with self.subTest(change=change), self.assertRaises(ValueError):
                response(items)
        for index in (3, 6):
            items = tutorial_records()
            items[index]['data']['state']['value'].update({'modal': True, 'dialog': opening_dialog(),
                                                          'dialog_answer': None})
            items[index]['data']['state']['value']['dialog']['id'] = 8
            with self.subTest(index=index), self.assertRaises(ValueError):
                response(items)

    def test_journal_rejects_invalid_frames_drops_nonfinite_and_late_records(self):
        for index, change in ((1, {'playing': 0}), (3, {'modal': 1}), (6, {'pending': None})):
            items = tutorial_records()
            items[index]['data']['state']['value'].update(change)
            with self.subTest(change=change), self.assertRaises(ValueError):
                evidence.journal(payload(items), 42)
        for mutate in ('drop', 'nonfinite', 'late', 'error', 'presentation'):
            items = tutorial_records()
            if mutate == 'drop':
                items[3]['dropped_records'] = 1
            elif mutate == 'nonfinite':
                items[1]['data']['state']['value']['camera']['center'][0] = float('nan')
            elif mutate == 'late':
                items.append(copy.deepcopy(items[6]))
            else:
                items.insert(-1, record('application_error' if mutate == 'error' else 'presentation_frame', 25000, {}))
            with self.subTest(mutate=mutate), self.assertRaises(ValueError):
                evidence.journal(payload(items), 42)

    def test_displays_require_exact_emitted_fields_even_without_previous_binding(self):
        for field, value in (('builtin', 1), ('active', 1), ('asleep', 0), ('mode_id', True),
                             ('nominal_refresh_hz', float('inf')), ('unexpected', 1)):
            snapshot = binding()
            snapshot['displays'][0][field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                evidence.validate_displays(snapshot)


def history_blocker(value, name):
    if name == 'changed-modal':
        value['dialog']['id'] += 1
    elif name == 'tick':
        value['tick'] += 1
    elif name == 'geometry':
        value['display_size'][0] += 100
    elif name != 'stable':
        value[name] = True


def pre_command_recovery(name):
    items = tutorial_records()
    blocked = copy.deepcopy(items[1])
    blocked['elapsed_us'] = 1100
    blocked['data']['draw_started_unix_ns'] = BASE + 1_100_000
    history_blocker(blocked['data']['state']['value'], name)
    recovered = copy.deepcopy(items[1])
    recovered['elapsed_us'] = 1200
    recovered['data']['draw_started_unix_ns'] = BASE + 1_200_000
    items[2:2] = [blocked, recovered]
    identifier = 0
    for item in items:
        if item['kind'] == 'frame':
            identifier += 1
            item['data']['frame_id'] = identifier
            item['data']['state']['state_id'] = identifier
    reservation = {'identity': evidence.offered_opening(items[1]['data']['state']['value']),
                   'frame': copy.deepcopy(items[1]['data']),
                   'pre_post_frame': copy.deepcopy(recovered['data'])}
    return items, reservation


class ReservationHistoryEvidenceTests(unittest.TestCase):
    def reject_recovery(self, name):
        items, reservation = pre_command_recovery(name)
        parsed = evidence.journal(payload(items), 42)
        original = copy.deepcopy(reservation)
        with self.assertRaisesRegex(ValueError, 'reservation history blocker at record 2'):
            response(parsed, reservation=reservation)
        self.assertEqual(reservation, original)

    def test_pending_then_recovered_pre_command_history_is_rejected(self):
        self.reject_recovery('pending')

    def test_playing_then_recovered_pre_command_history_is_rejected(self):
        self.reject_recovery('playing')

    def test_changed_modal_then_recovered_pre_command_history_is_rejected(self):
        self.reject_recovery('changed-modal')

    def test_tick_then_recovered_pre_command_history_is_rejected(self):
        self.reject_recovery('tick')

    def test_geometry_then_recovered_pre_command_history_is_rejected(self):
        self.reject_recovery('geometry')

    def test_other_pending_owners_then_recovered_history_is_rejected(self):
        for name in ('clock_pending', 'presentation_children_pending'):
            with self.subTest(name=name):
                self.reject_recovery(name)

    def test_valid_intervening_draws_do_not_replace_original_reservation(self):
        items, reservation = pre_command_recovery('stable')
        original = copy.deepcopy(reservation)
        result = response(evidence.journal(payload(items), 42), reservation=reservation)
        self.assertTrue(result['complete'])
        self.assertEqual(result['baseline_frame_id'], reservation['pre_post_frame']['frame_id'])
        self.assertEqual(reservation, original)
        self.assertNotEqual(reservation['frame']['frame_id'], reservation['pre_post_frame']['frame_id'])

    def test_answer_event_then_recovered_draw_cannot_be_rebased_away(self):
        items, reservation = pre_command_recovery('stable')
        items.insert(2, record('dialog_answer', 1050, opening_answer()))
        parsed = evidence.journal(payload(items), 42)
        with self.assertRaisesRegex(ValueError, 'reservation history blocker at record 2'):
            response(parsed, reservation=reservation)

    def test_null_input_then_recovered_draw_cannot_be_rebased_away(self):
        items, reservation = pre_command_recovery('stable')
        items.insert(2, record('input', 1050, {'fence': 1, 'input_sequence': 1,
            'received_unix_ns': BASE + 1_050_000, 'input': None}))
        for item in items[3:]:
            if item['kind'] in ('frame', 'input'):
                item['data']['input_sequence'] += 1
        reservation['pre_post_frame'] = copy.deepcopy(items[4]['data'])
        parsed = evidence.journal(payload(items), 42)
        with self.assertRaisesRegex(ValueError, 'reservation history blocker at record 2'):
            response(parsed, reservation=reservation)

    def test_late_pre_command_draw_after_press_cannot_escape_history_scan(self):
        items = tutorial_records()
        late = copy.deepcopy(items[1])
        late['data']['frame_id'] = 99
        late['data']['state']['state_id'] = 99
        late['data']['state']['value']['pending'] = True
        late['elapsed_us'] = 3500
        late['data']['draw_started_unix_ns'] = BASE + 1_100_000
        late['data']['input_sequence'] = 1
        items.insert(3, late)
        identifier = 0
        for item in items:
            if item['kind'] == 'frame':
                identifier += 1
                item['data']['frame_id'] = identifier
                item['data']['state']['state_id'] = identifier
        parsed = evidence.journal(payload(items), 42)
        with self.assertRaisesRegex(ValueError, 'Late pre-command'):
            response(parsed)


if __name__ == '__main__':
    unittest.main()
