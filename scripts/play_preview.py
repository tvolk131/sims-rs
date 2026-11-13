"""Launch the qualified local household preview without modifying its supported input."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

from preview_qualification import verify_qualification


REPOSITORY = Path(__file__).resolve().parent.parent


def private_path(value, exists=True):
    candidate = Path(value)
    if not candidate.is_absolute():
        candidate = REPOSITORY / candidate
    cache = REPOSITORY / 'cache'
    if cache.is_symlink() or not cache.is_dir():
        raise ValueError('The checkout needs a real, private cache directory')
    try:
        relative = candidate.relative_to(cache)
    except ValueError:
        raise ValueError('Preview inputs and outputs must remain beneath this checkout/cache')
    if not relative.parts or any(component in ('.', '..') for component in relative.parts):
        raise ValueError('Invalid private preview path')
    current = cache
    for component in relative.parts:
        current = current / component
        if current.is_symlink():
            raise ValueError('Refusing symlinked private preview paths')
    resolved = candidate.resolve(strict=exists)
    if not resolved.is_relative_to(cache.resolve(strict=True)):
        raise ValueError('Private preview path escapes cache')
    return resolved


def digest(filename):
    checksum = hashlib.sha256()
    with filename.open('rb') as source:
        for chunk in iter(lambda: source.read(1024*1024), b''):
            checksum.update(chunk)
    return checksum.hexdigest()


def verified_file(entry):
    filename = private_path(entry['path'])
    if not filename.is_file() or digest(filename) != entry['sha256']:
        raise ValueError('Qualified preview file is missing or changed: ' + str(filename))
    return filename


def launch(manifest_path):
    manifest_path = private_path(manifest_path)
    manifest_bytes = manifest_path.read_bytes()
    manifest_sha256 = hashlib.sha256(manifest_bytes).hexdigest()
    manifest = json.loads(manifest_bytes)
    if manifest.get('schema') != 1 or manifest.get('qualification', {}).get('passed') is not True:
        raise ValueError('This preview has not passed its complete performance qualification')
    binary = verified_file(manifest['binary'])
    supported_save = verified_file(manifest['supported_save'])
    if not os.access(binary, os.X_OK):
        raise ValueError('Qualified preview binary is not executable')
    game_root = Path(manifest['game_root'])
    if not game_root.is_absolute():
        game_root = REPOSITORY / game_root
    game_root = game_root.resolve(strict=True)
    if not game_root.is_dir():
        raise ValueError('The separately installed private game-data root is unavailable')
    print('Verifying three extended native evidence cohorts before launch...', flush=True)
    qualification = verify_qualification({**manifest, 'game_root': str(game_root)}, private_path, digest)
    run_root = private_path('cache/preview-runs', exists=False)
    run_root.mkdir(exist_ok=True)
    run = Path(tempfile.mkdtemp(prefix='household-', dir=run_root))
    executable = run / 'sims-room'
    with binary.open('rb') as source, executable.open('xb') as target:
        shutil.copyfileobj(source, target)
    executable.chmod(0o500)
    if digest(executable) != manifest['binary']['sha256']:
        raise ValueError('Preview binary changed while preparing its private executable copy')
    starting_save = run / 'starting-save.json'
    with supported_save.open('rb') as source, starting_save.open('xb') as target:
        shutil.copyfileobj(source, target)
    if digest(starting_save) != manifest['supported_save']['sha256']:
        raise ValueError('Supported save changed while preparing its private copy')
    starting_save.chmod(0o400)
    saved = json.loads(starting_save.read_bytes())
    original = saved.get('session', saved)['source']
    if not isinstance(original, str) or not original:
        raise ValueError('Supported save has no original-house identity')
    if digest(manifest_path) != manifest_sha256:
        raise ValueError('Preview manifest changed during verification')
    output_save = run / 'player-save.json'
    command = [str(executable), '--original', original, '--root', str(game_root),
               '--load', str(starting_save), '--save', str(output_save), '--paused']
    receipt = {'schema': 1, 'manifest': str(manifest_path), 'manifest_sha256': manifest_sha256,
               'binary_sha256': manifest['binary']['sha256'],
               'supported_save_sha256': manifest['supported_save']['sha256'],
               'starting_save': str(starting_save), 'save_output': str(output_save),
               'command': command, 'exit_code': None, 'qualification': qualification}
    receipt_path = run / 'launch.json'
    with receipt_path.open('x') as target:
        target.write(json.dumps(receipt, indent=2)+'\n')
    print('Starting paused. Use the game\'s Play control when ready.', flush=True)
    print('Private session: ' + str(run), flush=True)
    print('Saves, if requested in the UI: ' + str(output_save), flush=True)
    try:
        result = subprocess.run(command, cwd=REPOSITORY, check=False)
        receipt['exit_code'] = result.returncode
        return result.returncode
    finally:
        for name, filename, expected in (
                ('supported_input_unchanged', supported_save, manifest['supported_save']['sha256']),
                ('starting_copy_unchanged', starting_save, manifest['supported_save']['sha256']),
                ('executable_copy_unchanged', executable, manifest['binary']['sha256']),
                ('bundle_binary_unchanged', binary, manifest['binary']['sha256'])):
            try:
                receipt[name] = digest(filename) == expected
            except OSError:
                receipt[name] = False
        receipt_path.write_text(json.dumps(receipt, indent=2)+'\n')
        if not all(receipt[name] for name in ('supported_input_unchanged', 'starting_copy_unchanged',
                                               'executable_copy_unchanged')):
            raise ValueError('A read-only preview input changed; preserve the private session for investigation')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--manifest', type=Path, default=Path('cache/performance-preview/manifest.json'))
    arguments = parser.parse_args()
    try:
        return launch(arguments.manifest)
    except (OSError, KeyError, TypeError, ValueError) as error:
        print('Preview unavailable: ' + str(error), file=sys.stderr)
        return 1


if __name__ == '__main__':
    sys.exit(main())
