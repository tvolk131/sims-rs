#!/usr/bin/env python3
"""Start a fresh household, continue the last saved session, or load a managed save.

This is the ordinary development launcher. Performance qualification remains a
separate operation; this launcher does not claim its build has passed it.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile


REPOSITORY = Path(__file__).resolve().parent.parent
HOUSEHOLDS = {
    'newbie': 'UserData/Houses/House07.iff',
    'goth': 'UserData/Houses/House05.iff',
    'mashuga': 'UserData2/Houses/House09.iff',
}


def digest(path):
    value = hashlib.sha256()
    with path.open('rb') as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            value.update(chunk)
    return value.hexdigest()


def session_root():
    cache = REPOSITORY / 'cache'
    if cache.is_symlink():
        raise ValueError('The local cache must be a real directory')
    cache.mkdir(exist_ok=True)
    root = cache / 'alpha-sessions'
    if root.is_symlink():
        raise ValueError('The session directory must be a real directory')
    root.mkdir(exist_ok=True)
    return root.resolve(strict=True)


def envelope(path):
    with path.open('rb') as stream:
        value = json.load(stream)
    session = value.get('session') if isinstance(value, dict) else None
    if not isinstance(session, dict) or session.get('version') != 2:
        raise ValueError('Select a version-2 managed player save; the selected file was left unchanged')
    source = session.get('source')
    if not isinstance(source, str) or not source or '\0' in source:
        raise ValueError('The managed save has no valid household identity')
    if not isinstance(value.get('dialog_client'), dict):
        raise ValueError('The managed save is missing its player interface state')
    return source


def atomic_json(path, value):
    descriptor, temporary = tempfile.mkstemp(prefix='.session-', dir=path.parent)
    temporary = Path(temporary)
    try:
        with os.fdopen(descriptor, 'w') as stream:
            json.dump(value, stream, indent=2)
            stream.write('\n')
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def copy_input(source, target, expected, executable=False):
    with source.open('rb') as reader, target.open('xb') as writer:
        shutil.copyfileobj(reader, writer)
    if digest(target) != expected or digest(source) != expected:
        raise ValueError('A launch input changed while being copied; restart the launch')
    target.chmod(0o500 if executable else 0o400)


def last_save(root, game_root):
    pointer = root / 'last-session.json'
    if pointer.is_symlink():
        raise ValueError('The last-session record must be a real file')
    if not pointer.is_file():
        raise ValueError('No saved session yet. Start with --household newbie, goth, or mashuga and use Save game.')
    try:
        value = json.loads(pointer.read_bytes())
    except ValueError as error:
        raise ValueError('The last-session record is corrupt; use --load to select your save') from error
    if not isinstance(value, dict) or value.get('schema') != 1:
        raise ValueError('The last-session record is invalid or unsupported; use --load to select your save')
    if value.get('game_root') != str(game_root):
        raise ValueError('The last session belongs to a different installation; use --load or --household')
    selected = value.get('save')
    if not isinstance(selected, str) or not selected or '\0' in selected:
        raise ValueError('The last-session record has no valid save path; use --load to select your save')
    path = Path(selected)
    if not path.is_absolute() or not path.is_relative_to(root) or '..' in path.parts:
        raise ValueError('The last-session path is invalid; use --load to select your save')
    for component in (path, *path.parents):
        if component == root:
            break
        if component.is_symlink():
            raise ValueError('The last-session save must not use symlinks')
    if not path.is_file() or digest(path) != value.get('sha256'):
        raise ValueError('The last saved session is missing or changed; use --load or start a household')
    return path


def window_session_path(value):
    value = os.fspath(value)
    if not isinstance(value, str) or not value or '\0' in value:
        raise ValueError('Window observation paths must be nonempty text without NUL characters')
    path = Path(value)
    if not path.is_absolute():
        path = REPOSITORY / path
    if '..' in path.parts:
        raise ValueError('Window observation paths must not contain traversal')
    if any(component.is_symlink() for component in (path, *path.parents)):
        raise ValueError('Window observation paths must not use symlinks')
    return value, path


def window_session_arguments(output, library, game_root, sessions, inputs):
    if output is None:
        if library is not None:
            raise ValueError('--window-session-present-library requires --window-session-output')
        return []
    output_value, destination = window_session_path(output)
    if not destination.is_relative_to(sessions.parent):
        raise ValueError('Window observation output must remain inside the ignored local cache')
    if any(destination.is_relative_to(protected) or protected.is_relative_to(destination)
           for protected in (game_root, sessions, *inputs)):
        raise ValueError('Window observation output must be separate from installation, inputs, and session saves')
    if destination.exists():
        raise ValueError('Window observation output must be a fresh path; existing paths are never overwritten')
    if not destination.parent.is_dir():
        raise ValueError('Window observation output requires an existing real parent directory')
    arguments = ['--window-session-output', output_value]
    if library is not None:
        library_value, library_path = window_session_path(library)
        if not library_path.is_file():
            raise ValueError('The window observation library must be an existing real file')
        arguments.extend(['--window-session-present-library', library_value])
    return arguments


def launch(binary, game_root, household=None, load=None, resume=False, *,
           window_session_output=None, window_session_present_library=None):
    binary = Path(binary).resolve(strict=True)
    game_root = Path(game_root).resolve(strict=True)
    if not game_root.is_dir() or not binary.is_file() or not os.access(binary, os.X_OK):
        raise ValueError('An executable sims-room build and installed game-data directory are required')
    if sum((household is not None, load is not None, resume)) > 1:
        raise ValueError('Choose one of --household, --load, or --continue')
    root = session_root()
    if resume:
        load = last_save(root, game_root)
    if load is not None:
        load = Path(load).resolve(strict=True)
        input_hash = digest(load)
        original = envelope(load)
        if digest(load) != input_hash:
            raise ValueError('The selected save changed while being inspected')
    else:
        original = HOUSEHOLDS[household or 'newbie']
        input_hash = None
    observation_arguments = window_session_arguments(
        window_session_output, window_session_present_library, game_root, root,
        (binary, load) if load is not None else (binary,))
    run = Path(tempfile.mkdtemp(prefix='household-', dir=root))
    executable = run / 'sims-room'
    binary_hash = digest(binary)
    copy_input(binary, executable, binary_hash, executable=True)
    output = run / 'player-save.json'
    command = [str(executable), '--original', original, '--root', str(game_root),
               '--save', str(output), '--paused']
    starting = None
    if load is not None:
        starting = run / 'starting-save.json'
        copy_input(load, starting, input_hash)
        if envelope(starting) != original:
            raise ValueError('The copied save changed household identity')
        command.extend(['--load', str(starting)])
    else:
        command.extend(['--inferred-routing', '--buy-preview',
                        '--neighborhood-policy', 'installed-houses-ascending-v1'])
    command.extend(observation_arguments)
    receipt = {'schema': 1, 'command': command, 'binary_sha256': binary_hash,
               'input': str(load) if load else None, 'input_sha256': input_hash,
               'save_output': str(output), 'exit_code': None,
               'performance_qualified': False}
    atomic_json(run / 'launch.json', receipt)
    print('Opening household paused. Use Play to begin and Save game to retain progress.', flush=True)
    print('Session save: ' + str(output), flush=True)
    try:
        result = subprocess.run(command, cwd=REPOSITORY, check=False)
        receipt['exit_code'] = result.returncode
        return result.returncode
    finally:
        receipt['executable_unchanged'] = executable.is_file() and digest(executable) == binary_hash
        receipt['starting_copy_unchanged'] = starting is None or (starting.is_file() and digest(starting) == input_hash)
        if receipt['exit_code'] == 0 and receipt['executable_unchanged'] and receipt['starting_copy_unchanged']:
            if output.is_file() and not output.is_symlink():
                try:
                    saved_hash = digest(output)
                    if envelope(output) != original or digest(output) != saved_hash:
                        raise ValueError('The written save has a different identity or changed during validation')
                    atomic_json(root / 'last-session.json', {
                        'schema': 1, 'game_root': str(game_root), 'save': str(output),
                        'sha256': saved_hash, 'source': original,
                    })
                    receipt['continue_available'] = True
                except (OSError, ValueError, KeyError, TypeError) as error:
                    receipt['save_error'] = str(error)
                    print('Save could not become Continue Last Session: ' + str(error), file=sys.stderr)
        atomic_json(run / 'launch.json', receipt)
        if not receipt['executable_unchanged'] or not receipt['starting_copy_unchanged']:
            raise ValueError('A read-only launch input changed; the previous Continue save was preserved')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=REPOSITORY / 'target/release/sims-room')
    parser.add_argument('--root', type=Path, default=REPOSITORY / 'game-data/cabinet-extracted')
    choices = parser.add_mutually_exclusive_group()
    choices.add_argument('--household', choices=HOUSEHOLDS)
    choices.add_argument('--continue', dest='resume', action='store_true', help='Continue Last Session')
    choices.add_argument('--load', type=Path, help='Load an existing managed player save')
    parser.add_argument('--window-session-output', help='Record native observations to a fresh ignored-cache path')
    parser.add_argument('--window-session-present-library', help='Use an existing native presentation observation library')
    arguments = parser.parse_args()
    try:
        return launch(arguments.binary, arguments.root, arguments.household, arguments.load, arguments.resume,
                      window_session_output=arguments.window_session_output,
                      window_session_present_library=arguments.window_session_present_library)
    except (OSError, ValueError, KeyError, TypeError) as error:
        print('Unable to open household: ' + str(error), file=sys.stderr)
        return 1


if __name__ == '__main__':
    raise SystemExit(main())
