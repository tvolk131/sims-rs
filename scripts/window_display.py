"""Validate target-window display snapshots without treating mode metadata as FPS."""
import json
import math


def summarize_displays(journal, required=False):
    errors, snapshots = [], []
    try:
        for entry in journal:
            if entry.get('native_command') == ['window'] and entry.get('exit_code') == 0:
                snapshots.append(json.loads(entry['stdout']))
        available = any('displays' in snapshot for snapshot in snapshots)
        if not available and not required:
            return {'available': False, 'valid': None, 'errors': [], 'snapshots': len(snapshots)}
        if len(snapshots) < 2 or not available:
            raise ValueError('Multiple target-window display snapshots are required')
        configuration, owner = None, None
        for snapshot in snapshots:
            displays = snapshot['displays']
            if not isinstance(displays, list) or not 1 <= len(displays) <= 8:
                raise ValueError('Invalid target-window display list')
            if snapshot['display_scope'] != 'Current CoreGraphics mode and NSScreen maximum; not instantaneous refresh':
                raise ValueError('Unsupported display measurement scope')
            current_owner = (snapshot['pid'], snapshot['window_id'])
            if not all(type(value) is int and value > 0 for value in current_owner):
                raise ValueError('Invalid target-window display owner')
            if owner is not None and owner != current_owner:
                raise ValueError('Target window or process changed between display snapshots')
            owner = current_owner
            identifiers = set()
            for display in displays:
                for name in ('display_id', 'width', 'height', 'pixel_width', 'pixel_height'):
                    if type(display[name]) is not int or display[name] <= 0:
                        raise ValueError('Invalid display geometry or identity')
                if display['display_id'] in identifiers:
                    raise ValueError('Duplicate target display')
                identifiers.add(display['display_id'])
                if (type(display['mode_id']) is not int or display['mode_id'] < 0
                        or type(display['maximum_frames_per_second']) is not int
                        or display['maximum_frames_per_second'] < 0):
                    raise ValueError('Invalid display mode or maximum frame rate')
                refresh = display['nominal_refresh_hz']
                if (type(refresh) not in (int, float) or not math.isfinite(refresh) or refresh < 0
                        or (refresh == 0 and display['maximum_frames_per_second'] == 0)):
                    raise ValueError('Display refresh configuration is unavailable or malformed')
                if (type(display['builtin']) is not bool or display['active'] is not True
                        or display['asleep'] is not False):
                    raise ValueError('Target display is inactive, asleep or malformed')
            current = sorted(displays, key=lambda display: display['display_id'])
            if configuration is not None and configuration != current:
                raise ValueError('Target display configuration changed during the session')
            configuration = current
        return {'available': True, 'valid': True, 'errors': [], 'snapshots': len(snapshots),
                'configuration': configuration,
                'nominal_refresh_unknown': any(display['nominal_refresh_hz'] == 0 for display in configuration),
                'scope': 'Current CoreGraphics mode and NSScreen maximum for the target window; '
                         'zero nominal refresh is unknown/adaptive, not zero Hz; '
                         'neither field is an instantaneous refresh or presentation-rate measurement'}
    except (KeyError, TypeError, ValueError) as failure:
        errors.append(str(failure))
    return {'available': bool(snapshots), 'valid': False, 'errors': errors, 'snapshots': len(snapshots)}
