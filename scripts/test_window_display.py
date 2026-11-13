import copy
import json
import unittest

from window_display import summarize_displays


def snapshot():
    return {'pid': 123, 'window_id': 456,
            'display_scope': 'Current CoreGraphics mode and NSScreen maximum; not instantaneous refresh',
            'displays': [{'display_id': 1, 'mode_id': 7, 'width': 1440, 'height': 900,
                          'pixel_width': 2880, 'pixel_height': 1800, 'nominal_refresh_hz': 0,
                          'maximum_frames_per_second': 120, 'builtin': True,
                          'active': True, 'asleep': False}]}


def journal(*snapshots):
    return [{'native_command': ['window'], 'exit_code': 0, 'stdout': json.dumps(value)}
            for value in snapshots]


class DisplayTests(unittest.TestCase):
    def test_stable_unknown_nominal_refresh_is_not_reported_as_zero_hz_or_fps(self):
        report = summarize_displays(journal(snapshot(), snapshot()), required=True)
        self.assertTrue(report['valid'], report['errors'])
        self.assertTrue(report['nominal_refresh_unknown'])
        self.assertEqual(report['configuration'][0]['maximum_frames_per_second'], 120)
        self.assertNotIn('fps', report)

    def test_legacy_snapshots_are_unavailable_but_cannot_qualify_when_required(self):
        entries = journal({'pid': 123, 'window_id': 456}, {'pid': 123, 'window_id': 456})
        self.assertIsNone(summarize_displays(entries)['valid'])
        self.assertFalse(summarize_displays(entries, required=True)['valid'])

    def test_any_mode_change_is_latched_even_if_last_snapshot_recovers(self):
        changed = snapshot()
        changed['displays'][0]['mode_id'] += 1
        self.assertFalse(summarize_displays(journal(snapshot(), changed, snapshot()))['valid'])

    def test_inactive_asleep_nonfinite_and_missing_metadata_fail_closed(self):
        for field, value in (('active', False), ('asleep', True), ('nominal_refresh_hz', float('nan')),
                             ('nominal_refresh_hz', float('inf')), ('maximum_frames_per_second', -1),
                             ('pixel_width', 0), ('builtin', 'yes')):
            with self.subTest(field=field):
                changed = copy.deepcopy(snapshot())
                changed['displays'][0][field] = value
                self.assertFalse(summarize_displays(journal(snapshot(), changed))['valid'])
        changed = snapshot()
        del changed['displays'][0]['mode_id']
        self.assertFalse(summarize_displays(journal(snapshot(), changed))['valid'])

    def test_single_snapshot_wrong_owner_or_mixed_coverage_cannot_qualify(self):
        self.assertFalse(summarize_displays(journal(snapshot()), required=True)['valid'])
        for key in ('pid', 'window_id'):
            changed = snapshot()
            changed[key] += 1
            self.assertFalse(summarize_displays(journal(snapshot(), changed))['valid'])
        self.assertFalse(summarize_displays(journal(snapshot(), {'pid': 123, 'window_id': 456}))['valid'])


if __name__ == '__main__':
    unittest.main()
