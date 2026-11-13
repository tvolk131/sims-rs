"""Authored protocol data only; no child/helper/window/PID/native execution."""
import copy
import subprocess
import unittest
from unittest import mock

import window_session as driver
from test_window_session import binding


SCOPE = 'observational candidate; independent retained-execution and exact-window verification required'


class TargetedBindingTests(unittest.TestCase):
    def setUp(self):
        self.current = binding()
        self.identity = copy.deepcopy(self.current['kernel_identity'])
        self.candidate = {'schema': 'sims-owned-window-candidate-v1', 'pid': 42,
                          'generation': 0, 'window_id': self.current['window_id'], 'scope': SCOPE}
        self.publication = {'status': 'DURABLY_RECORDED', 'generation': 0,
                            'window_number': self.current['window_id'], 'scope': SCOPE}
        for name in ('run', 'Popen'):
            patch = mock.patch.object(subprocess, name, side_effect=AssertionError('Native execution forbidden'))
            patch.start()
            self.addCleanup(patch.stop)

    def request(self, operation='observe', **options):
        return driver.targeted_window_request(self.identity, 42, '/private/fake', 0,
                                              self.candidate, self.publication, operation, **options)

    def response(self, operation='observe'):
        return {'operation': operation, 'binding': copy.deepcopy(self.current),
                'launch_services': {'registered': False}, 'posts': []}

    def test_exact_current_candidate_and_expected_lifetime_construct_observation_only(self):
        request = self.request()
        self.assertEqual(set(request), {'operation', 'pid', 'executable', 'target'})
        self.assertEqual(request['target'], {'schema': 'sims-window-target-v1',
                         'window_id': self.current['window_id'], 'kernel_identity': self.identity})
        self.assertEqual(driver.validate_targeted_response(self.response(), request), self.response())

    def test_source_observations_and_returned_requests_do_not_alias(self):
        request = self.request(binding=self.current)
        request['target']['kernel_identity']['start_tvusec'] += 1
        request['binding']['bounds'][0] += 1
        self.assertEqual(self.identity, self.current['kernel_identity'])
        original = binding()
        self.assertEqual(self.current, original)
        request = self.request(binding=self.current)
        self.identity['start_tvusec'] += 1
        self.current['bounds'][0] += 1
        self.assertEqual(request['target']['kernel_identity'], original['kernel_identity'])
        self.assertEqual(request['binding'], original)

    def test_historical_candidate_without_current_durable_ack_is_not_a_target(self):
        for publication in (None, {}, {'status': 'UNAVAILABLE', 'scope': 'observational candidate'},
                            {**self.publication, 'status': 'Pending'},
                            {**self.publication, 'generation': 1},
                            {**self.publication, 'window_number': self.current['window_id'] + 1}):
            self.publication = publication
            with self.subTest(publication=publication), self.assertRaises(ValueError):
                self.request()

    def test_candidate_numeric_aliases_foreign_pid_and_generation_are_rejected(self):
        original = copy.deepcopy(self.candidate)
        for name, value in [('pid', True), ('pid', 42.0), ('pid', 43), ('generation', False),
                            ('generation', 0.0), ('generation', 1), ('window_id', True),
                            ('window_id', float(self.current['window_id'])), ('window_id', 0),
                            ('window_id', 2**32), ('schema', 'other'), ('scope', 'trusted')]:
            self.candidate = {**original, name: value}
            with self.subTest(name=name, value=value), self.assertRaises(ValueError):
                self.request()

    def test_publication_numeric_aliases_and_unclaimed_fields_are_rejected(self):
        original = copy.deepcopy(self.publication)
        for name, value in [('generation', False), ('generation', 0.0), ('window_number', True),
                            ('window_number', float(self.current['window_id'])), ('scope', 'trusted'),
                            ('extra', 'foreign')]:
            self.publication = {**original, name: value}
            with self.subTest(name=name, value=value), self.assertRaises(ValueError):
                self.request()

    def test_required_candidate_and_current_publication_fields_are_exact(self):
        for name in list(self.candidate):
            original = self.candidate.pop(name)
            with self.subTest(candidate=name), self.assertRaises(ValueError):
                self.request()
            self.candidate[name] = original
        for name in list(self.publication):
            original = self.publication.pop(name)
            with self.subTest(publication=name), self.assertRaises(ValueError):
                self.request()
            self.publication[name] = original

    def test_expected_kernel_identity_cannot_be_supplied_by_a_window_binding(self):
        for value in (None, {}, self.current, {**self.identity, 'pid': 43},
                      {**self.identity, 'start_tvsec': True},
                      {**self.identity, 'executable': '/foreign'}):
            self.identity = value
            with self.subTest(value=value), self.assertRaises(ValueError):
                self.request()

    def test_input_close_and_release_require_existing_exact_window_binding(self):
        for operation in ('key', 'release', 'close'):
            with self.subTest(operation=operation), self.assertRaises(ValueError):
                self.request(operation, code=124 if operation != 'close' else None)
        for name, value in [('window_id', self.current['window_id'] + 1),
                            ('kernel_identity', {**self.identity, 'start_tvusec': 123457})]:
            changed = {**self.current, name: value}
            with self.subTest(name=name), self.assertRaises(ValueError):
                self.request('key', binding=changed, code=124)

    def test_only_existing_bounded_key_grammar_is_available(self):
        for operation in ('key', 'release'):
            for code in (36, 124):
                request = self.request(operation, binding=self.current, code=code)
                self.assertEqual(request['code'], code)
                self.assertEqual(request.get('press_ms'), 20 if operation == 'key' else None)
            for code in (True, 124.0, 32, 0, None):
                with self.subTest(operation=operation, code=code), self.assertRaises(ValueError):
                    self.request(operation, binding=self.current, code=code)
        for operation in ('observe', 'close'):
            with self.subTest(operation=operation), self.assertRaises(ValueError):
                self.request(operation, binding=self.current, code=124)
        with self.assertRaises(ValueError):
            self.request('activate', binding=self.current)

    def test_response_must_match_exact_window_and_expected_lifetime_on_first_observe(self):
        request = self.request()
        for name, value in [('window_id', self.current['window_id'] + 1),
                            ('kernel_identity', {**self.identity, 'start_tvusec': 123457}),
                            ('pid', 43), ('executable', '/foreign'), ('frontmost', False),
                            ('on_screen', False)]:
            response = self.response()
            response['binding'][name] = value
            with self.subTest(name=name), self.assertRaises(ValueError):
                driver.validate_targeted_response(response, request)

    def test_not_ready_is_only_initial_observation_with_exact_expected_lifetime(self):
        response = {'status': 'not_ready', 'reason': 'owned_window_unavailable',
                    'kernel_identity': copy.deepcopy(self.identity)}
        self.assertIsNone(driver.validate_targeted_response(response, self.request()))
        for request in (self.request(binding=self.current), self.request('close', binding=self.current)):
            with self.assertRaises(ValueError):
                driver.validate_targeted_response(response, request)
        response['kernel_identity']['start_tvusec'] += 1
        with self.assertRaises(ValueError):
            driver.validate_targeted_response(response, self.request())

    def test_bound_response_cannot_change_geometry_or_optional_process_metadata(self):
        request = self.request(binding=self.current)
        response = self.response()
        response['binding']['bounds'][0] += 1
        with self.assertRaises(ValueError):
            driver.validate_targeted_response(response, request)
        response = self.response()
        response['launch_services'] = {'registered': True, 'executable': '/foreign'}
        with self.assertRaises(ValueError):
            driver.validate_targeted_response(response, request)

    def test_decoder_rejects_mutated_outbound_target_shape_and_key_grammar(self):
        for target in (None, {}, {**self.request()['target'], 'window_id': True},
                       {**self.request()['target'], 'schema': 'other'}):
            with self.subTest(target=target), self.assertRaises(ValueError):
                driver.validate_targeted_response(self.response(), {**self.request(), 'target': target})
        for request in ({**self.request(), 'operation': 'activate'},
                        {**self.request(), 'extra': 0},
                        {**self.request(), 'binding': None},
                        {**self.request(), 'binding': {**self.current, 'window_id': self.current['window_id'] + 1}},
                        {**self.request('key', binding=self.current, code=124), 'press_ms': True},
                        {**self.request('release', binding=self.current, code=36), 'code': 36.0}):
            with self.subTest(request=request), self.assertRaises(ValueError):
                driver.validate_targeted_response(self.response(), request)

    def test_helper_errors_cannot_be_hidden_by_a_valid_observation(self):
        for field in ('error', 'release_error'):
            with self.subTest(field=field), self.assertRaises(ValueError):
                driver.validate_targeted_response({**self.response(), field: 'failed'}, self.request())

    def test_release_ack_and_operation_are_exact(self):
        request = self.request('release', binding=self.current, code=36)
        response = {**self.response('release'), 'code': 36, 'released': True}
        self.assertEqual(driver.validate_targeted_response(response, request), response)
        for name, value in [('code', 124), ('code', 36.0), ('released', 1), ('operation', 'key')]:
            with self.subTest(name=name), self.assertRaises(ValueError):
                driver.validate_targeted_response({**response, name: value}, request)

    def test_production_legacy_backend_fails_before_process_or_helper_creation(self):
        backend = driver.NativeBackend()
        with self.assertRaisesRegex(RuntimeError, 'retained-execution adapter'):
            backend.start(['/private/fake'], '/private', None, None)
        with self.assertRaisesRegex(RuntimeError, 'retained-execution adapter'):
            backend.helper('/private/fake-helper', self.request(), 1)

    def test_swift_requires_target_before_kernel_or_window_query_and_never_censuses(self):
        source = (driver.SOURCE / 'window_session_macos.swift').read_text()
        run = source.split('func run()', 1)[1]
        self.assertLess(run.index('try validateTarget(request["target"]'),
                        run.index('let identity = try kernelIdentity(pid, executable)'))
        owner = source.split('func windowBinding(', 1)[1].split('\n}', 1)[0]
        self.assertIn('CGWindowListCopyWindowInfo(.optionIncludingWindow, targetWindow)', owner)
        for forbidden in ('NSWorkspace', 'kCGNullWindowID', 'optionOnScreenOnly', 'optionAll',
                          'runningApplications', 'CGWindowListCreate'):
            self.assertNotIn(forbidden, source)
        self.assertEqual(source.count('CGWindowListCopyWindowInfo('), 1)


if __name__ == '__main__':
    unittest.main()
