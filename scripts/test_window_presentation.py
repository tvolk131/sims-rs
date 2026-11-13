import copy
import unittest

from window_presentation import (CONFIGURATION_FIELDS, COUNTED_EVENTS, FATAL_COUNTERS,
                                 INTEGER_FIELDS, TIME_FIELDS, qualify_presentations,
                                 summarize_presentations)


def trace(frame_count=3):
    events = []
    def event(kind, host_time, **values):
        payload = dict.fromkeys(INTEGER_FIELDS + TIME_FIELDS + ('drawable_width', 'drawable_height'), 0)
        payload.update(abi_version=1, struct_size=192, kind=kind, status=0,
                       event_sequence=len(events)+1, host_time=host_time)
        payload.update(values)
        events.append({'kind': 'presentation_event', 'data': payload})
    event(1, 1)
    event(9, 1.001, platform_notification=6)
    frames = []
    for frame_id in range(1, frame_count+1):
        start = 1 + frame_id * .01
        frames.append({'kind': 'frame', 'data': {'frame_id': frame_id,
            'state': {'state_id': frame_id*10, 'value': {'tick': None if frame_id == 1 else 100+frame_id}}}})
        common = {'draw_context_sequence': frame_id, 'frame_id': frame_id, 'state_id': frame_id*10}
        event(2, start, flags=4, **common)
        common.update(acquisition_sequence=frame_id, layer_lifetime_id=1,
                      acquisition_enter_host_time=start+.0001)
        event(3, start+.0001, flags=4|4096, **common)
        common.update(acquisition_call_host_time=start+.0002,
                      acquisition_return_host_time=start+.0003, texture_identity=1000+frame_id,
                      drawable_id=frame_id-1, drawable_width=2640, drawable_height=1800,
                      display_sync_enabled=1, maximum_drawable_count=3)
        event(4, start+.0003, flags=4|8|16|64|4096, **common)
        event(5, start+.0004, flags=4|8|16|64|4096, **common)
        if frame_id > 1:
            common.update(bind_count=1, bound_frame_id=frame_id, bound_state_id=frame_id*10)
            event(7, start+.0005, flags=4|8|16|32, **common)
        event(6, start+.006, flags=4|8|16|(32 if frame_id > 1 else 64|8192),
              **common, presented_host_time=start+.005, callback_delivery_host_time=start+.006)
    cutoff = 1 + (frame_count+2)*.01
    event(11, cutoff, flags=1)
    event(13, cutoff+.002, flags=1)
    counters = dict.fromkeys(FATAL_COUNTERS + tuple(COUNTED_EVENTS), 0)
    counters.update(acquisition_entries=frame_count, acquisition_returns=frame_count, acquisition_null_returns=0,
                    successful_acquisitions=frame_count, handler_registration_attempts=frame_count,
                    handler_registrations=frame_count, presented_callbacks=frame_count, positive_presentations=frame_count,
                    bind_calls=frame_count-1, successful_binds=frame_count-1, missing_binds=1, draw_context_calls=frame_count,
                    platform_notifications=1, emitted_events=len(events),
                    zero_presented_timestamps=0, texture_reuses_before_callback=0)
    summary = {'abi_version': 1, 'struct_size': 408, 'status': 0, 'session_state': 4,
               'flags': 12, 'capabilities': 3, 'last_event_sequence': len(events),
               'last_acquisition_sequence': frame_count, 'unresolved_acquisitions': 0,
               'unresolved_unbound_acquisitions': 0, 'unbound_acquisitions': 1,
               'begin_host_time': 1, 'cutoff_host_time': cutoff, 'drain_end_host_time': cutoff+.002,
               'snapshot_host_time': cutoff+.003, 'counters': counters}
    def sleep_sample(phase, absolute):
        return {'kind': 'presentation_sleep_clock', 'data': {'schema': 1, 'phase': phase,
            'clock': 'mach timebase ticks; absolute stops during sleep; continuous includes sleep',
            'sample': {'absolute_before': absolute, 'continuous': absolute+1_000_000_001,
                       'absolute_after': absolute+2, 'timebase_numer': 1, 'timebase_denom': 1}}}
    return ([sleep_sample('begin', 990_000_000),
             {'kind': 'presentation_library', 'data': {'abi_version': 1}},
             {'kind': 'presentation_begin', 'data': {'status': 0, 'abi_version': 1,
                 'clock': 'CACurrentMediaTime host seconds; not UNIX time',
                 'boundary': 'MTLDrawable.presentedTime from addPresentedHandler'}}]
            + events + frames + [sleep_sample('end', int((cutoff+.01)*1_000_000_000)),
                                {'kind': 'presentation_end', 'data': {'status': 0, 'summary': summary}}])


def native_events(records, kind=None, acquisition=None):
    return [record['data'] for record in records if record['kind'] == 'presentation_event'
            and (kind is None or record['data']['kind'] == kind)
            and (acquisition is None or record['data']['acquisition_sequence'] == acquisition)]


def insert_notification(records, notification, timestamp):
    sample = copy.deepcopy(native_events(records, 9)[0])
    sample.update(platform_notification=notification, host_time=timestamp)
    index = next(index for index, record in enumerate(records)
                 if record['kind'] == 'presentation_event' and record['data']['host_time'] > timestamp)
    records.insert(index, {'kind': 'presentation_event', 'data': sample})
    for sequence, event in enumerate(native_events(records), 1):
        event['event_sequence'] = sequence
    summary = records[-1]['data']['summary']
    summary['last_event_sequence'] = summary['counters']['emitted_events'] = len(native_events(records))
    summary['counters']['platform_notifications'] += 1


class PresentationTests(unittest.TestCase):
    def assert_invalid(self, records):
        report = summarize_presentations(records, required=True)
        self.assertFalse(report['valid'], report)
        self.assertEqual(report['presented_interval_us']['samples'], 0)
        self.assertEqual(report['ready_presented_interval_us']['samples'], 0)
        return report

    def test_complete_exactly_bound_trace_uses_host_presentation_times(self):
        report = summarize_presentations(trace(), required=True)
        self.assertTrue(report['valid'], report['errors'])
        self.assertEqual(report['positive_presentations'], 3)
        self.assertEqual(report['ready_presentations'], 2)
        self.assertAlmostEqual(report['ready_presented_interval_us']['p50'], 10000)
        self.assertEqual(report['lifecycle_host_seconds'], {
            'begin': 1, 'cutoff': 1.05, 'drain_end': 1.052, 'snapshot': 1.053})

    def test_legacy_trace_never_claims_presentation_measurement(self):
        self.assertIsNone(summarize_presentations([])['valid'])
        self.assert_invalid([])

    def test_unbound_ready_acquisition_cannot_borrow_another_acquisitions_binding(self):
        records = trace()
        for event in native_events(records):
            if event['frame_id'] == 1:
                event.update(frame_id=2, state_id=20)
        report = self.assert_invalid(records)
        self.assertIn('own exact acquisition binding', report['errors'][0])

    def test_short_probe_is_not_long_qualification(self):
        report = summarize_presentations(trace(), required=True)
        self.assertTrue(report['valid'], report['errors'])
        qualification = qualify_presentations(report)
        self.assertFalse(qualification['qualified'])
        self.assertIn('duration', qualification['errors'][0])

    def test_long_lifecycle_with_silent_tail_does_not_earn_ready_duration(self):
        report = summarize_presentations(trace(), required=True)
        report.update(ready_measurement_duration_seconds=1000, ready_terminal_gap_us=999_000_000)
        self.assertFalse(qualify_presentations(report)['qualified'])
        report['ready_presented_duration_seconds'] = 900
        self.assertFalse(qualify_presentations(report)['qualified'])

    def test_long_qualification_requires_both_censored_boundaries_and_percentiles(self):
        report = summarize_presentations(trace(4), required=True)
        self.assertGreater(report['ready_leading_gap_us'], 0)
        self.assertGreater(report['ready_terminal_gap_us'], 0)
        report['ready_presented_duration_seconds'] = 900
        self.assertTrue(qualify_presentations(report)['qualified'])
        for field in ('ready_leading_gap_us', 'ready_terminal_gap_us'):
            for value in (250_001, float('nan'), -1):
                with self.subTest(field=field, value=value):
                    changed = copy.deepcopy(report)
                    changed[field] = value
                    self.assertFalse(qualify_presentations(changed)['qualified'])
        for field, value in (('p95', 20_001), ('p99', 33_401), ('worst', 250_001)):
            changed = copy.deepcopy(report)
            changed['ready_presented_interval_us'][field] = value
            self.assertFalse(qualify_presentations(changed)['qualified'])
        report['ready_presented_interval_us'].update(p95=20_000, p99=33_400, worst=250_000)
        self.assertTrue(qualify_presentations(report)['qualified'])

    def test_interleaved_loading_cannot_hide_a_long_ready_interval(self):
        records = trace(6)
        for record in records:
            if record['kind'] == 'frame' and record['data']['frame_id'] == 3:
                record['data']['state']['value']['tick'] = None
        for event in native_events(records):
            if event['frame_id'] >= 4 or event['kind'] in (11, 13):
                for field in TIME_FIELDS:
                    if event[field] > 0:
                        event[field] += 900
        summary = records[-1]['data']['summary']
        for field in ('cutoff_host_time', 'drain_end_host_time', 'snapshot_host_time'):
            summary[field] += 900
        clock = next(record['data']['sample'] for record in records
                     if record['kind'] == 'presentation_sleep_clock' and record['data']['phase'] == 'end')
        for field in ('absolute_before', 'continuous', 'absolute_after'):
            clock[field] += 900_000_000_000
        report = summarize_presentations(records, required=True)
        self.assertTrue(report['valid'], report['errors'])
        self.assertGreaterEqual(report['ready_presented_duration_seconds'], 900)
        self.assertLess(report['ready_leading_gap_us'], 250_000)
        self.assertLess(report['ready_terminal_gap_us'], 250_000)
        self.assertEqual(report['ready_presented_interval_us']['samples'], 3)
        self.assertGreater(report['ready_presented_interval_us']['worst'], 900_000_000)
        self.assertFalse(qualify_presentations(report)['qualified'])

    def test_interleaved_loading_cannot_hide_ready_interval_inactivity(self):
        records = trace(5)
        for record in records:
            if record['kind'] == 'frame' and record['data']['frame_id'] == 3:
                record['data']['state']['value']['tick'] = None
        insert_notification(records, 2, 1.037)
        insert_notification(records, 1, 1.038)
        report = self.assert_invalid(records)
        self.assertIn('Ready presentation interval crosses an inactive period', report['errors'])

    def test_missing_event_invalidates_distributions(self):
        records = trace()
        del records[5]
        self.assert_invalid(records)

    def test_actual_bind_identity_must_match_even_when_callback_unchanged(self):
        for field, value in (('bound_state_id', 999), ('acquisition_sequence', 3),
                             ('texture_identity', 999), ('draw_context_sequence', 3)):
            with self.subTest(field=field):
                records = trace()
                native_events(records, 7, 2)[0][field] = value
                self.assert_invalid(records)

    def test_duplicate_registration_owner_is_not_rescued_by_counts(self):
        records = trace()
        native_events(records, 5, 2)[0]['acquisition_sequence'] = 1
        self.assert_invalid(records)

    def test_each_join_requires_exact_drawable_texture_and_draw_context(self):
        for kind in (4, 5, 6, 7):
            for field in ('texture_identity', 'drawable_id', 'layer_lifetime_id',
                          'draw_context_sequence', 'frame_id', 'state_id'):
                with self.subTest(kind=kind, field=field):
                    records = trace()
                    native_events(records, kind, 2)[0][field] += 10
                    self.assert_invalid(records)

    def test_required_native_fields_cannot_be_omitted(self):
        for field in INTEGER_FIELDS + TIME_FIELDS + ('drawable_width', 'drawable_height'):
            with self.subTest(field=field):
                records = trace()
                del native_events(records, 7, 2)[0][field]
                self.assert_invalid(records)

    def test_ready_draw_without_presentation_is_not_silently_excluded(self):
        records = trace()
        records.append({'kind': 'frame', 'data': {'frame_id': 4,
                        'state': {'state_id': 40, 'value': {'tick': 104}}}})
        report = self.assert_invalid(records)
        self.assertIn('Ready draw/presentation coverage is incomplete', report['errors'])

    def test_unresolved_tail_and_counter_failures_latch(self):
        for name in ('outstanding_acquisitions', 'late_callbacks', 'bookkeeping_overflows'):
            records = trace()
            records[-1]['data']['summary']['counters'][name] = 1
            self.assert_invalid(records)

    def test_all_timestamps_must_be_finite_and_nonnegative(self):
        for field in TIME_FIELDS:
            for value in (float('nan'), float('inf'), -1):
                with self.subTest(field=field, value=value):
                    records = trace()
                    native_events(records, 6, 2)[0][field] = value
                    self.assert_invalid(records)

    def test_summary_clock_and_boundaries_are_required_and_causal(self):
        for field in ('begin_host_time', 'cutoff_host_time', 'drain_end_host_time', 'snapshot_host_time'):
            for value in (float('nan'), 0, 1.04 if field == 'snapshot_host_time' else 100000):
                with self.subTest(field=field, value=value):
                    records = trace()
                    records[-1]['data']['summary'][field] = value
                    self.assert_invalid(records)
        for field in ('clock', 'boundary'):
            records = trace()
            begin = next(record['data'] for record in records if record['kind'] == 'presentation_begin')
            begin[field] = 'unsupported'
            self.assert_invalid(records)
            del begin[field]
            self.assert_invalid(records)

    def test_shifted_callback_times_outside_unchanged_lifecycle_fail(self):
        records = trace()
        callback = native_events(records, 6, 2)[0]
        callback['host_time'] = callback['callback_delivery_host_time'] = 100
        callback['presented_host_time'] = 99
        self.assert_invalid(records)

    def test_lifecycle_events_cannot_be_moved_before_measurement_work(self):
        for kind in (11, 13):
            records = trace()
            index = next(index for index, record in enumerate(records)
                         if record['kind'] == 'presentation_event' and record['data']['kind'] == kind)
            records.insert(4, records.pop(index))
            for sequence, event in enumerate(native_events(records), 1):
                event['event_sequence'] = sequence
            self.assert_invalid(records)

    def test_inactive_presentation_with_callback_after_reactivation_fails(self):
        records = trace()
        insert_notification(records, 2, 1.024)
        insert_notification(records, 1, 1.0255)
        self.assert_invalid(records)

    def test_foreground_presentation_with_delayed_callback_after_resign_is_valid(self):
        records = trace()
        insert_notification(records, 2, 1.0355)
        report = summarize_presentations(records)
        self.assertTrue(report['valid'], report['errors'])

    def test_foreground_endpoints_cannot_bridge_inactivity(self):
        records = trace()
        insert_notification(records, 2, 1.027)
        insert_notification(records, 1, 1.028)
        self.assert_invalid(records)

    def test_post_cutoff_presentation_is_accounted_but_not_timed(self):
        records = trace(4)
        summary = records[-1]['data']['summary']
        summary.update(cutoff_host_time=1.044, drain_end_host_time=1.05, snapshot_host_time=1.051)
        native_events(records, 11)[0]['host_time'] = 1.044
        native_events(records, 13)[0]['host_time'] = 1.05
        callback_record = next(record for record in records
                               if record['kind'] == 'presentation_event'
                               and record['data']['kind'] == 6 and record['data']['acquisition_sequence'] == 4)
        records.remove(callback_record)
        index = next(index for index, record in enumerate(records)
                     if record['kind'] == 'presentation_event' and record['data']['kind'] == 13)
        records.insert(index, callback_record)
        callback_record['data']['flags'] |= 1
        for sequence, event in enumerate(native_events(records), 1):
            event['event_sequence'] = sequence
        report = summarize_presentations(records)
        self.assertTrue(report['valid'], report['errors'])
        self.assertEqual(report['positive_presentations'], 4)
        self.assertEqual(report['measured_presentations'], 3)
        self.assertEqual(report['post_cutoff_presentations'], 1)
        self.assertEqual(report['ready_presented_interval_us']['samples'], 1)
        self.assertAlmostEqual(report['ready_presented_duration_seconds'], .01)
        self.assertAlmostEqual(report['ready_terminal_gap_us'], 9_000)
        callback_record['data'].update(presented_host_time=1.049, callback_delivery_host_time=1.0495,
                                       host_time=1.0495)
        later = summarize_presentations(records)
        self.assertTrue(later['valid'], later['errors'])
        self.assertEqual(later['ready_presented_duration_seconds'], report['ready_presented_duration_seconds'])
        self.assertEqual(later['ready_terminal_gap_us'], report['ready_terminal_gap_us'])

    def test_silent_tail_cannot_qualify_a_long_lifecycle(self):
        records = trace()
        summary = records[-1]['data']['summary']
        summary.update(cutoff_host_time=902, drain_end_host_time=902.002, snapshot_host_time=902.003)
        native_events(records, 11)[0]['host_time'] = 902
        native_events(records, 13)[0]['host_time'] = 902.002
        clocks = [record['data'] for record in records if record['kind'] == 'presentation_sleep_clock']
        clocks[-1]['sample'].update(absolute_before=902_010_000_000, continuous=903_010_000_001,
                                    absolute_after=902_010_000_002)
        report = summarize_presentations(records)
        self.assertTrue(report['valid'], report['errors'])
        self.assertGreater(report['ready_measurement_duration_seconds'], 900)
        self.assertAlmostEqual(report['ready_presented_duration_seconds'], .01)
        self.assertGreater(report['ready_terminal_gap_us'], 900_000_000)
        self.assertFalse(qualify_presentations(report)['qualified'])

    def test_layer_policy_changes_fail_even_when_all_joined_fields_agree(self):
        for field in CONFIGURATION_FIELDS:
            with self.subTest(field=field):
                records = trace()
                for event in native_events(records, acquisition=3):
                    if event['kind'] in (4, 5, 6, 7):
                        event[field] = (2 if event[field] == 3 else 1-event[field])
                report = self.assert_invalid(records)
                self.assertIn('Layer presentation configuration changed during measurement', report['errors'])

    def test_settings_valid_flag_is_required(self):
        records = trace()
        native_events(records, 4, 2)[0]['flags'] &= ~16
        self.assert_invalid(records)

    def test_sleep_guard_requires_two_covering_same_domain_brackets(self):
        for field, value in (('timebase_numer', 0), ('timebase_denom', 0),
                             ('continuous', 5), ('absolute_before', 1_010_000_000),
                             ('absolute_after', 980_000_000)):
            with self.subTest(field=field):
                records = trace()
                clocks = [record['data'] for record in records if record['kind'] == 'presentation_sleep_clock']
                clocks[0]['sample'][field] = value
                self.assert_invalid(records)
        records = trace()
        del records[0]
        self.assert_invalid(records)

    def test_sleep_offset_change_and_uncertain_sampling_invalidate_measurement(self):
        for field, increase in (('continuous', 1_000_000_000), ('absolute_after', 2_000_000)):
            records = trace()
            clocks = [record['data'] for record in records if record['kind'] == 'presentation_sleep_clock']
            clocks[-1]['sample'][field] += increase
            self.assert_invalid(records)

    def test_startup_display_change_is_disclosed_not_a_ready_phase_change(self):
        records = trace()
        insert_notification(records, 3, 1.017)
        report = summarize_presentations(records)
        self.assertTrue(report['valid'], report['errors'])
        self.assertEqual(report['startup_display_change_host_times'], [1.017])
        self.assertEqual(report['startup_interrupted_intervals'], 1)
        self.assertEqual(report['presented_interval_us']['samples'], 1)
        self.assertEqual(report['ready_presented_interval_us']['samples'], 1)
        insert_notification(records, 3, 1.027)
        self.assert_invalid(records)

    def test_null_loading_acquisition_has_no_registration_bind_or_callback(self):
        records = trace()
        records[:] = [record for record in records if not (
            record['kind'] == 'presentation_event' and record['data']['acquisition_sequence'] == 1
            and record['data']['kind'] in (5, 6))]
        returned = native_events(records, 4, 1)[0]
        returned.update(flags=4|32768|4096, drawable_id=0, texture_identity=0,
                        drawable_width=0, drawable_height=0, display_sync_enabled=0,
                        maximum_drawable_count=0)
        summary = records[-1]['data']['summary']
        summary['unbound_acquisitions'] = 0
        summary['counters'].update(acquisition_null_returns=1, successful_acquisitions=2,
            handler_registration_attempts=2, handler_registrations=2, presented_callbacks=2,
            positive_presentations=2, missing_binds=0, emitted_events=len(native_events(records)))
        summary['last_event_sequence'] = len(native_events(records))
        for sequence, event in enumerate(native_events(records), 1):
            event['event_sequence'] = sequence
        report = summarize_presentations(records)
        self.assertTrue(report['valid'], report['errors'])
        self.assertEqual(report['positive_presentations'], 2)
        records[-1]['data']['summary']['counters']['acquisition_null_returns'] = 0
        self.assert_invalid(records)

    def test_texture_reuse_requires_exact_predecessor_supersession(self):
        records = trace(4)
        for event in native_events(records, acquisition=4):
            if event['kind'] in (4, 5, 6, 7):
                event['texture_identity'] = 1001
        self.assert_invalid(records)
        predecessor = copy.deepcopy(native_events(records, 6, 1)[0])
        predecessor.update(kind=8, host_time=native_events(records, 4, 4)[0]['host_time'])
        predecessor['flags'] |= 1024
        index = next(index for index, record in enumerate(records)
                     if record['kind'] == 'presentation_event' and record['data']['kind'] == 4
                     and record['data']['acquisition_sequence'] == 4)
        records.insert(index, {'kind': 'presentation_event', 'data': predecessor})
        summary = records[-1]['data']['summary']
        summary['last_event_sequence'] = summary['counters']['emitted_events'] = len(native_events(records))
        summary['counters']['texture_lifetime_reuses'] = 1
        for sequence, event in enumerate(native_events(records), 1):
            event['event_sequence'] = sequence
        report = summarize_presentations(records)
        self.assertTrue(report['valid'], report['errors'])
        delayed = next(record for record in records if record['kind'] == 'presentation_event'
                       and record['data']['kind'] == 6 and record['data']['acquisition_sequence'] == 1)
        delayed['data'].update(host_time=1.041, callback_delivery_host_time=1.041)
        predecessor.update(host_time=1.042, callback_delivery_host_time=1.041)
        records.remove(delayed)
        index = next(index for index, record in enumerate(records)
                     if record['kind'] == 'presentation_event' and record['data'] is predecessor)
        records.insert(index, delayed)
        native_events(records, 5, 4)[0]['host_time'] = 1.0421
        native_events(records, 7, 4)[0]['host_time'] = 1.0422
        for sequence, event in enumerate(native_events(records), 1):
            event['event_sequence'] = sequence
        report = summarize_presentations(records)
        self.assertTrue(report['valid'], report['errors'])
        predecessor['drawable_id'] += 1
        self.assert_invalid(records)

    def test_texture_reuse_before_callback_keeps_both_acquisitions_distinct(self):
        records = trace(4)
        for event in native_events(records, acquisition=4):
            if event['kind'] in (4, 5, 6, 7):
                event['texture_identity'] = 1001
        callback_record = next(record for record in records if record['kind'] == 'presentation_event'
                               and record['data']['kind'] == 6 and record['data']['acquisition_sequence'] == 1)
        predecessor = copy.deepcopy(callback_record['data'])
        predecessor.update(kind=8, host_time=native_events(records, 4, 4)[0]['host_time'],
                           presented_host_time=0, callback_delivery_host_time=0)
        predecessor['flags'] |= 1024
        records.remove(callback_record)
        index = next(index for index, record in enumerate(records) if record['kind'] == 'presentation_event'
                     and record['data']['kind'] == 4 and record['data']['acquisition_sequence'] == 4)
        records.insert(index, {'kind': 'presentation_event', 'data': predecessor})
        index = next(index for index, record in enumerate(records) if record['kind'] == 'presentation_event'
                     and record['data']['kind'] == 6 and record['data']['acquisition_sequence'] == 4)
        callback_record['data'].update(host_time=1.041, callback_delivery_host_time=1.041)
        callback_record['data']['flags'] |= 1024
        records.insert(index, callback_record)
        summary = records[-1]['data']['summary']
        summary['last_event_sequence'] = summary['counters']['emitted_events'] = len(native_events(records))
        summary['counters'].update(texture_lifetime_reuses=1, texture_reuses_before_callback=1)
        for sequence, event in enumerate(native_events(records), 1):
            event['event_sequence'] = sequence
        report = summarize_presentations(records)
        self.assertTrue(report['valid'], report['errors'])
        self.assertEqual(report['texture_reuses_before_callback'], 1)
        self.assertEqual(report['positive_presentations'], 4)
        predecessor['presented_host_time'] = 1.015
        self.assert_invalid(records)

    def test_zero_loading_timestamps_are_accounted_without_inventing_presentation(self):
        records = trace()
        native_events(records, 6, 1)[0]['presented_host_time'] = 0
        summary = records[-1]['data']['summary']
        summary['counters'].update(positive_presentations=2, zero_presented_timestamps=1)
        report = summarize_presentations(records)
        self.assertTrue(report['valid'], report['errors'])
        self.assertEqual(report['acquired_drawables'], 3)
        self.assertEqual(report['positive_presentations'], 2)
        self.assertEqual(report['zero_timestamp_loading_drawables'], [1])
        self.assertEqual(report['presented_interval_us']['samples'], 1)
        native_events(records, 6, 2)[0]['presented_host_time'] = 0
        summary['counters'].update(positive_presentations=1, zero_presented_timestamps=2)
        report = self.assert_invalid(records)
        self.assertIn('Expected multiple ready presentations before cutoff', report['errors'])

    def test_zero_ready_outcomes_cannot_dilute_positive_presentation_gaps(self):
        records = trace(5)
        native_events(records, 6, 3)[0]['presented_host_time'] = 0
        records[-1]['data']['summary']['counters'].update(positive_presentations=4,
                                                        zero_presented_timestamps=1)
        report = summarize_presentations(records)
        self.assertTrue(report['valid'], report['errors'])
        self.assertEqual(report['zero_timestamp_ready_drawables'], [3])
        self.assertEqual(report['zero_timestamp_ready_acquisitions'], [3])
        self.assertEqual(report['zero_timestamp_loading_drawables'], [])
        self.assertEqual(report['ready_acquisitions'], 4)
        self.assertEqual(report['ready_presentations'], 3)
        self.assertEqual(report['measured_ready_presentations'], 3)
        self.assertEqual(report['ready_presented_interval_us']['samples'], 2)
        self.assertAlmostEqual(report['ready_presented_interval_us']['worst'], 20_000)
        self.assertAlmostEqual(report['ready_presented_duration_seconds'], .03)
        native_events(records, 6, 3)[0]['bound_state_id'] += 1
        self.assert_invalid(records)

    def test_zero_ready_boundaries_cannot_earn_duration_or_shrink_censored_gaps(self):
        for identifier, expected_duration, leading, terminal in (
                (2, .01, 14900, 15000), (4, .01, 4900, 25000)):
            records = trace(4)
            native_events(records, 6, identifier)[0]['presented_host_time'] = 0
            records[-1]['data']['summary']['counters'].update(positive_presentations=3,
                                                            zero_presented_timestamps=1)
            report = summarize_presentations(records)
            with self.subTest(identifier=identifier):
                self.assertTrue(report['valid'], report['errors'])
                self.assertAlmostEqual(report['ready_presented_duration_seconds'], expected_duration)
                self.assertAlmostEqual(report['ready_leading_gap_us'], leading)
                self.assertAlmostEqual(report['ready_terminal_gap_us'], terminal)
                self.assertFalse(qualify_presentations(report)['qualified'])

    def test_zero_ready_callbacks_still_need_exact_binding_accounting_and_foreground(self):
        for variant in ('counter', 'binding', 'foreground-leading', 'foreground-tail'):
            records = trace(4)
            identifier = 2 if variant == 'foreground-leading' else 4 if variant == 'foreground-tail' else 3
            callback = native_events(records, 6, identifier)[0]
            callback['presented_host_time'] = 0
            records[-1]['data']['summary']['counters'].update(positive_presentations=3,
                                                            zero_presented_timestamps=1)
            if variant == 'counter':
                records[-1]['data']['summary']['counters']['zero_presented_timestamps'] = 0
            elif variant == 'binding':
                callback.update(bind_count=0, bound_frame_id=0, bound_state_id=0)
                callback['flags'] &= ~32
            else:
                insert_notification(records, 2, 1 + identifier * .01 + .0002)
                insert_notification(records, 1, 1 + identifier * .01 + .00025)
            with self.subTest(variant=variant):
                self.assert_invalid(records)

    def test_explicit_resize_preserves_attribution_without_pooling_policy_changes(self):
        records = trace()
        for event in native_events(records, acquisition=3):
            if event['kind'] in (4, 5, 6, 7):
                event.update(drawable_width=2400, drawable_height=1600)
        report = summarize_presentations(records)
        self.assertTrue(report['valid'], report['errors'])
        self.assertEqual(report['drawable_resizes'], [{'frame_id': 3, 'size': [2400, 1600]}])


if __name__ == '__main__':
    unittest.main()
