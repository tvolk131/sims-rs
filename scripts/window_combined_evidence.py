"""Offline, unauthenticated diagnostic provenance, never an acceptance verifier.

Input v1 is exactly {schema, artifacts, sources, binaries}. artifacts maps the
six REQUIRED_ROLES and any OPTIONAL_ROLES to {path, size, sha256}; sources maps
repository-relative names (or explicit staged/ aliases) and binaries maps opaque
names to the same descriptor. Staged aliases must hash to an inventory source.
selected_save and selected_save_pinned are explicit raw metadata artifacts for
the original selected source and launcher-pinned input. Neither is an output save.
No receipt path is an authority to read. In particular installation, save, SDK,
compiler, journal-cache and package paths in receipts are never followed.

Policy v1 is exactly {schema, approved_roots, expected_pins, source_inventory,
bindings, budgets}. expected_pins independently supplies the complete flattened
role map (source:<name>, binary:<name> for explicit extra artifacts). Bindings
name those roles, not new paths. presentation_sources maps source roles to
explicit staged-source roles, or None for an unstaged source. All three budgets are mandatory: metadata_bytes
(including journal and source), binary_bytes, artifacts. They can lower, never
raise, the hard limits below. A source inventory is a complete independently
reviewed path->sha256 map, not a source map copied out of a build receipt.

read_combined requires an explicitly constructed DescriptorReader, not an
arbitrary callback or a permissive default. Its trusted contract is component-
wise descriptor-relative O_NOFOLLOW/O_NONBLOCK reads, single-link regular files,
bounded streaming hashing, and identical pre/open/post descriptor AND pathname
fingerprints, including directory components. This is observation, not a lock
against a hostile kernel or a process capable of restoring filesystem state.
analyze_combined does no I/O. A hand-constructed bundle cannot establish that
DescriptorReader actually ran; its observations are descriptive, not signed.

contract_valid means the supplied diagnostic contracts join, not acceptance.
Scope blockers survive even a wholly synthetic complete contract. The versioned
camera export remains offline-only and unqualified. Current producers do not
export journal lifetime/build IDs. Selected-input association requires the
reviewed v2 owner-preparation contract and independently supplied raw bytes; no
fallback reconstructs them.
Journal build_ids/kernel_identity remain proposed missing producer contracts,
NOT fields claimed to exist today. The camera v2 producer captures associations.
Each association explicitly names control_index and before/key/after_command_index
in native_commands. The current single-control producer also requires native_posts
to equal that key response under the legacy contract. Only the exact
sims-window-camera-control-raw-v2 tag selects native_posts_raw_v2 and requires
legacy native_posts to be its deterministic enriched copy. No indices or
enclosing timestamps are inferred.
"""

import copy
import hashlib
import json
import math
import os
import stat
import sys

import window_evidence as ordinary
import window_presentation_evidence as presentation


INPUT_SCHEMA = 'sims-window-combined-input-v1'
POLICY_SCHEMA = 'sims-window-combined-policy-v1'
BUNDLE_SCHEMA = 'sims-window-combined-bundle-v1'
OUTPUT_SCHEMA = 'sims-window-combined-diagnostic-v1'
CAMERA_EXPORT_CONTRACT = 'sims-window-camera-control-raw-v2'
FINAL_OBSERVATION_CONTRACT = 'sims-window-journal-final-observation-v1'
LOADED_INPUT_CONTRACT = 'sims-window-loaded-input-observation-v2'
REQUIRED_ROLES = frozenset(('launch', 'result', 'journal', 'plan', 'endpoints', 'helper_build'))
OPTIONAL_ROLES = frozenset(('campaign_build', 'campaign_run', 'app_package_build',
                            'presentation_build', 'controls', 'selected_save', 'selected_save_pinned'))
MAX_JOURNAL_BYTES = 64 * 1024**2
MAX_JSON_BYTES = 16 * 1024**2
MAX_RECORDS = 100_000
MAX_CONTROLS = 256
MAX_DEPTH = 64
MAX_NODES = 2_000_000
MAX_METADATA_BYTES = 256 * 1024**2
MAX_BINARY_BYTES = 1024**3
MAX_BINARY_FILE_BYTES = 512 * 1024**2
MAX_ARTIFACTS = 4096
CHUNK_BYTES = 64 * 1024
ANALYZER_SHA256 = '992bd23e769724c0218e668bff83fada45703acfad70fc0d54ca4900e5147aa6'
ANALYZER_SOURCE = 'scripts/window_presentation_evidence.py'
COLLECTOR_SOURCE = 'crates/sims-app/src/window_session/presentation.rs'
ORDINARY_KINDS = frozenset(('session', 'session_end', 'input', 'frame', 'span_begin',
                            'span_end', 'scene_prepare', 'scene_encode'))
PRESENTATION_KINDS = frozenset(('presentation_library', 'presentation_begin',
                                'presentation_event', 'presentation_sleep_clock', 'presentation_end'))
CONTROL_FIELDS = frozenset(('control_effect', 'native_result', 'binding_before', 'binding_after',
                            'fence', 'started_unix_ns', 'finished_unix_ns', 'next_started_unix_ns'))
BINDINGS = frozenset(('application', 'application_pinned', 'launcher_source', 'helper_source',
                      'helper', 'helper_pinned', 'driver_source', 'collector_source', 'analyzer_source',
                      'presentation', 'presentation_pinned', 'presentation_sources'))


def require(condition, message):
    if not condition:
        raise ValueError(message)


def integer(value, minimum=0, maximum=2**64 - 1):
    return type(value) is int and minimum <= value <= maximum


def digest(data):
    return hashlib.sha256(data).hexdigest()


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=False,
                      allow_nan=False).encode('utf-8')


def same(left, right):
    if type(left) is not type(right):
        return False
    if type(left) is dict:
        return left.keys() == right.keys() and all(same(left[key], right[key]) for key in left)
    if type(left) is list:
        return len(left) == len(right) and all(same(first, second) for first, second in zip(left, right))
    return left == right


def tree(value, depth=0, budget=None):
    if budget is None:
        budget = [MAX_NODES]
    budget[0] -= 1
    require(depth <= MAX_DEPTH and budget[0] >= 0, 'JSON nesting/node bound exceeded')
    if type(value) is dict:
        for key, item in value.items():
            require(type(key) is str, 'JSON object key must be text')
            key.encode('utf-8', errors='strict')
            tree(item, depth + 1, budget)
    elif type(value) is list:
        for item in value:
            tree(item, depth + 1, budget)
    elif type(value) is str:
        value.encode('utf-8', errors='strict')
    elif type(value) is int:
        require(-(2**63) <= value <= 2**64 - 1, 'JSON integer outside fixed range')
    elif type(value) is float:
        require(math.isfinite(value) and (value == 0 or abs(value) >= sys.float_info.min),
                'Nonfinite or underflowing JSON float')
    else:
        require(value is None or type(value) is bool, 'Non-JSON scalar type')


def parse_json(payload):
    require(type(payload) is bytes and len(payload) <= MAX_JSON_BYTES, 'JSON payload byte bound exceeded')
    text = payload.decode('utf-8', errors='strict')
    depth, quoted, escaped = 0, False, False
    for character in text:
        if quoted:
            if escaped:
                escaped = False
            elif character == '\\':
                escaped = True
            elif character == '"':
                quoted = False
        elif character == '"':
            quoted = True
        elif character in '[{':
            depth += 1
            require(depth <= MAX_DEPTH, 'JSON nesting bound exceeded')
        elif character in ']}':
            depth -= 1

    def pairs(items):
        result = {}
        for name, value in items:
            require(name not in result, 'Duplicate JSON key: ' + name)
            result[name] = value
        return result

    def floating(token):
        value = float(token)
        nonzero = any(character in '123456789' for character in token.lower().split('e', 1)[0])
        require(math.isfinite(value) and not (value == 0 and nonzero)
                and (value == 0 or abs(value) >= sys.float_info.min),
                'Nonfinite or underflowing JSON float')
        return value

    def integral(token):
        require(len(token.lstrip('-')) <= 20, 'JSON integer digit bound exceeded')
        value = int(token)
        require(-(2**63) <= value <= 2**64 - 1, 'JSON integer outside fixed range')
        return value

    def constant(token):
        raise ValueError('Nonfinite JSON constant: ' + token)

    value = json.loads(text, object_pairs_hook=pairs, parse_float=floating,
                       parse_int=integral, parse_constant=constant)
    tree(value)
    return value


def safe_path(value, absolute=True):
    require(type(value) is str and 0 < len(value) <= 4096 and '\0' not in value and '\\' not in value,
            'Invalid artifact path')
    require(value.startswith('/') is absolute, 'Artifact path has wrong absolute/relative form')
    parts = value.split('/')[1:] if absolute else value.split('/')
    require(parts and all(part not in ('', '.', '..', '.git') for part in parts),
            'Traversal, empty component or .git artifact path')
    return value


def sha256(value):
    return type(value) is str and len(value) == 64 and all(character in '0123456789abcdef' for character in value)


def descriptor(value):
    require(type(value) is dict and set(value) == {'path', 'size', 'sha256'},
            'Artifact descriptor requires exactly path/size/sha256')
    safe_path(value['path'])
    require(integer(value['size']) and sha256(value['sha256']), 'Invalid artifact size/hash')


def contract(spec, policy):
    tree(spec)
    tree(policy)
    require(type(spec) is dict and set(spec) == {'schema', 'artifacts', 'sources', 'binaries'}
            and spec['schema'] == INPUT_SCHEMA, 'Unsupported exact input role-map schema')
    require(type(policy) is dict and set(policy) == {
        'schema', 'approved_roots', 'expected_pins', 'source_inventory', 'bindings', 'budgets'}
        and policy['schema'] == POLICY_SCHEMA, 'Unsupported independent policy schema')
    roles = spec['artifacts']
    require(type(roles) is dict and REQUIRED_ROLES <= roles.keys()
            and roles.keys() <= REQUIRED_ROLES | OPTIONAL_ROLES, 'Missing/unknown artifact role')
    flat = dict(roles)
    for group, prefix in (('sources', 'source:'), ('binaries', 'binary:')):
        require(type(spec[group]) is dict, 'Artifact group must be an object')
        for name, item in spec[group].items():
            safe_path(name, absolute=False)
            flat[prefix + name] = item
    budgets = policy['budgets']
    require(type(budgets) is dict and set(budgets) == {'metadata_bytes', 'binary_bytes', 'artifacts'},
            'Explicit aggregate metadata/binary/artifact budgets required')
    for key, maximum in (('metadata_bytes', MAX_METADATA_BYTES), ('binary_bytes', MAX_BINARY_BYTES),
                         ('artifacts', MAX_ARTIFACTS)):
        require(integer(budgets[key], 1, maximum), 'Invalid aggregate budget: ' + key)
    require(len(flat) <= budgets['artifacts'], 'Aggregate artifact budget exceeded')
    roots = policy['approved_roots']
    require(type(roots) is list and 1 <= len(roots) <= MAX_ARTIFACTS
            and len(roots) == len(set(roots)), 'Explicit unique approved roots required')
    for root in roots:
        safe_path(root)
        require(root != '/', 'Filesystem root is not an approved evidence root')
    metadata_bytes = binary_bytes = 0
    paths = set()
    for role, item in flat.items():
        descriptor(item)
        path, size = item['path'], item['size']
        require(any(path.startswith(root + '/') for root in roots), 'Artifact outside approved roots: ' + role)
        require(path not in paths, 'Path reused across artifact roles')
        paths.add(path)
        if role.startswith('binary:'):
            require(size <= MAX_BINARY_FILE_BYTES, 'Binary file byte bound exceeded')
            binary_bytes += size
        else:
            require(size <= (MAX_JOURNAL_BYTES if role == 'journal' else MAX_JSON_BYTES),
                    'Metadata file byte bound exceeded')
            metadata_bytes += size
    require(metadata_bytes <= budgets['metadata_bytes'] and binary_bytes <= budgets['binary_bytes'],
            'Aggregate metadata/binary byte budget exceeded')
    require(same(flat, policy['expected_pins']), 'Spec differs from independently authorized expected pins')
    inventory = policy['source_inventory']
    require(type(inventory) is dict and bool(inventory), 'Independent complete source inventory required')
    for name, value in inventory.items():
        safe_path(name, absolute=False)
        require(sha256(value), 'Invalid expected source-inventory hash')
    for name, item in spec['sources'].items():
        require((name.startswith('staged/') and item['sha256'] in inventory.values())
                or inventory.get(name) == item['sha256'],
                'Source artifact differs from independent inventory: ' + name)
    bindings = policy['bindings']
    require(type(bindings) is dict and bindings.keys() <= BINDINGS, 'Unknown binding policy role')
    for name, value in bindings.items():
        if name == 'presentation_sources':
            require(type(value) is dict, 'Presentation sources require explicit source/staged role pairs')
            for source, staged in value.items():
                require(type(source) is str and source.startswith('source:') and source in flat
                        and (staged is None or (type(staged) is str and staged in flat
                                                and staged.startswith('source:'))), 'Unapproved source/staged role')
        else:
            require(type(value) is str and value in flat, 'Binding must name an explicitly approved artifact')
            expected_prefix = 'source:' if name.endswith('_source') else 'binary:'
            require(value.startswith(expected_prefix), 'Source/binary binding type substitution')
    return flat


def fingerprint(info):
    return {key: getattr(info, 'st_' + attribute) for key, attribute in (
        ('device', 'dev'), ('inode', 'ino'), ('mode', 'mode'), ('links', 'nlink'),
        ('uid', 'uid'), ('size', 'size'), ('mtime_ns', 'mtime_ns'), ('ctime_ns', 'ctime_ns'))}


def valid_file_fingerprint(value, size):
    return (type(value) is dict and set(value) == {
        'device', 'inode', 'mode', 'links', 'uid', 'size', 'mtime_ns', 'ctime_ns'}
        and all(integer(item) for item in value.values()) and value['links'] == 1
        and stat.S_ISREG(value['mode']) and same(value['size'], size))


def final_observation_claim(value, diagnostic, payload, source, reader_observation, flat, payloads, observations):
    require(type(value) is dict and set(value) == {
        'contract', 'status', 'child_collected', 'forced_teardown', 'source', 'limit_bytes',
        'complete', 'eof', 'bytes_read', 'prefix_sha256', 'size_bytes', 'wholefile_sha256',
        'components_before', 'components_after', 'earlier_components', 'previous_size',
        'retained_buffer', 'semantic', 'snapshot', 'error'}, 'Malformed exact final-observation object')
    require(type(value['contract']) is str and value['contract'] == FINAL_OBSERVATION_CONTRACT,
            'Unknown final-observation version; no legacy fallback')
    require(type(value['status']) is str and value['status'] == 'observed'
            and value['child_collected'] is True and value['forced_teardown'] is False
            and value['complete'] is True and value['eof'] is True and value['error'] is None,
            'Incomplete/failed final observation')
    require(diagnostic.get('child_outcome') == 'exited'
            and type(diagnostic.get('actual_child_returncode')) is int
            and diagnostic.get('forced_teardown') is False, 'Final observation contradicts child lifecycle')
    require(integer(value['limit_bytes'], 0, MAX_JOURNAL_BYTES)
            and integer(value['bytes_read'], 0, value['limit_bytes'])
            and integer(value['size_bytes'], 0, value['limit_bytes'])
            and value['size_bytes'] == value['bytes_read'] == len(payload)
            and sha256(value['prefix_sha256']) and sha256(value['wholefile_sha256'])
            and value['prefix_sha256'] == value['wholefile_sha256'] == digest(payload)
            and integer(value['previous_size'], 0, len(payload)), 'Final whole-file size/hash differs')
    association = value['source']
    require(type(association) is dict and set(association) == {'path', 'output_directory'},
            'Malformed final source association')
    safe_path(association['path'])
    safe_path(association['output_directory'])
    require(association['output_directory'] != '/'
            and association['path'] == source['path']
            and association['path'].startswith(association['output_directory'] + '/'),
            'Final source path differs from explicitly supplied journal manifest')
    parts = source['path'].split('/')[1:]
    paths = ['/'] + ['/' + '/'.join(parts[:index]) for index in range(1, len(parts) + 1)]
    before, after = value['components_before'], value['components_after']
    require(type(before) is list and type(after) is list and len(before) == len(paths)
            and same(before, after), 'Missing/changed final component fingerprints')
    states = {}
    for index, entry in enumerate(before):
        require(type(entry) is dict and set(entry) == {'path', 'fingerprint'}
                and type(entry['path']) is str and entry['path'] == paths[index],
                'Malformed final component association')
        state = entry['fingerprint']
        require(type(state) is dict and set(state) == {
            'device', 'inode', 'mode', 'links', 'uid', 'size', 'mtime_ns', 'ctime_ns'}
            and all(integer(item) for item in state.values()), 'Malformed component fingerprint types')
        require(valid_file_fingerprint(state, len(payload)) if index == len(paths) - 1
                else stat.S_ISDIR(state['mode']), 'Unsafe final component')
        states[entry['path']] = state
    require(type(reader_observation) is dict
            and reader_observation.get('contract') == 'descriptor-nofollow-nonblocking-streaming-prepost-v1'
            and same(reader_observation.get('before'), before[-1]['fingerprint'])
            and same(reader_observation.get('after'), before[-1]['fingerprint'])
            and same(reader_observation.get('size'), len(payload))
            and reader_observation.get('sha256') == digest(payload),
            'Actual supplied source object is not associated with final descriptor observation')
    earlier = value['earlier_components']
    require(type(earlier) is list and 1 <= len(earlier) <= len(paths), 'Missing earlier owned-directory identities')
    seen = set()
    for entry in earlier:
        require(type(entry) is dict and set(entry) == {'path', 'identity'}
                and type(entry['path']) is str and entry['path'] in states and entry['path'] not in seen,
                'Malformed earlier component association')
        seen.add(entry['path'])
        expected = {name: states[entry['path']][name] for name in ('device', 'inode', 'mode', 'uid')}
        require(type(entry['identity']) is dict and set(entry['identity']) == set(expected)
                and all(integer(item) for item in entry['identity'].values())
                and same(entry['identity'], expected), 'Original ancestor/leaf identity differs')
    require(all(name in seen for name in paths[:paths.index(association['output_directory']) + 1]),
            'Original owned output ancestors were not retained')
    retained = value['retained_buffer']
    require(type(retained) is dict and set(retained) == {'bytes', 'sha256', 'prefix_matches'}
            and integer(retained['bytes'], 0, len(payload)) and sha256(retained['sha256'])
            and retained['prefix_matches'] is True
            and retained['sha256'] == digest(payload[:retained['bytes']]), 'Retained prefix claim differs')
    if retained['bytes'] or value['previous_size']:
        require(all(name in seen for name in paths),
                'Earlier nonempty journal observation lost telemetry/leaf identities')
    semantic = value['semantic']
    require(type(semantic) is dict and set(semantic) == {'status', 'records', 'error'}
            and type(semantic['status']) is str and semantic['status'] == 'parsed'
            and integer(semantic['records'], 0, MAX_RECORDS) and semantic['error'] is None,
            'Final semantic parse failed or malformed')
    snapshot = value['snapshot']
    require(type(snapshot) is dict and set(snapshot) == {
        'status', 'path', 'bytes_written', 'prefix_sha256', 'sha256', 'fingerprint', 'error'},
        'Malformed snapshot accounting')
    require(type(snapshot['status']) is str and snapshot['status'] in ('not_requested', 'written'),
            'Raw snapshot write failed')
    if snapshot['status'] == 'not_requested':
        require(snapshot['path'] is None and type(snapshot['bytes_written']) is int
                and snapshot['bytes_written'] == 0 and snapshot['prefix_sha256'] == digest(b'')
                and snapshot['sha256'] is None and snapshot['fingerprint'] is None and snapshot['error'] is None,
                'Contradictory absent snapshot')
    else:
        require(type(snapshot['path']) is str
                and snapshot['path'] == association['output_directory'] + '/journal-final-observed.raw'
                and type(snapshot['bytes_written']) is int and snapshot['bytes_written'] == len(payload)
                and sha256(snapshot['sha256']) and snapshot['sha256'] == snapshot['prefix_sha256'] == digest(payload)
                and snapshot['error'] is None and valid_file_fingerprint(snapshot['fingerprint'], len(payload))
                and stat.S_IMODE(snapshot['fingerprint']['mode']) == 0o600, 'Contradictory snapshot bytes/identity')
        roles = [role for role, item in flat.items() if item['path'] == snapshot['path']]
        require(len(roles) == 1 and roles[0] != 'journal', 'Explicit independent snapshot manifest association missing')
        role = roles[0]
        require(payloads.get(role) == payload and flat[role]['sha256'] == digest(payload)
                and same(flat[role]['size'], len(payload)), 'Independent supplied raw snapshot differs')
        observed = observations.get(role)
        require(type(observed) is dict
                and observed.get('contract') == 'descriptor-nofollow-nonblocking-streaming-prepost-v1'
                and same(observed.get('before'), snapshot['fingerprint'])
                and same(observed.get('after'), snapshot['fingerprint'])
                and same(observed.get('size'), len(payload)) and observed.get('sha256') == digest(payload),
                'Supplied raw snapshot source object association differs')
    return semantic['records']


class DescriptorReader:
    """Trusted local reader; no arbitrary injected callback implementation is accepted."""

    __slots__ = ()

    def read(self, item):
        descriptor(item)
        require(item['size'] <= MAX_BINARY_FILE_BYTES, 'Reader hard byte bound exceeded')
        require(all(hasattr(os, name) for name in ('O_NOFOLLOW', 'O_NONBLOCK', 'O_DIRECTORY')),
                'Required nofollow/nonblocking/directory primitives unavailable')
        flags = os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | getattr(os, 'O_CLOEXEC', 0)
        opened, edges = [], []
        try:
            parent = os.open('/', flags | os.O_DIRECTORY)
            opened.append(parent)
            parts = item['path'].split('/')[1:]
            for index, component in enumerate(parts):
                last = index == len(parts) - 1
                before = os.stat(component, dir_fd=parent, follow_symlinks=False)
                require((stat.S_ISREG(before.st_mode) and before.st_nlink == 1) if last
                        else stat.S_ISDIR(before.st_mode), 'Symlink/hardlink/special path component')
                child = os.open(component, flags | (0 if last else os.O_DIRECTORY), dir_fd=parent)
                opened.append(child)
                state = fingerprint(os.fstat(child))
                require(state == fingerprint(before), 'Path substituted during descriptor open')
                edges.append((parent, component, child, state))
                parent = child
            require(state['size'] == item['size'], 'Artifact size differs before read')
            chunks, hasher, total = [], hashlib.sha256(), 0
            while True:
                chunk = os.read(parent, min(CHUNK_BYTES, item['size'] - total + 1))
                if not chunk:
                    break
                total += len(chunk)
                require(total <= item['size'], 'Artifact grew beyond descriptor bound')
                hasher.update(chunk)
                chunks.append(chunk)
            for directory, component, child, before in reversed(edges):
                require(before == fingerprint(os.fstat(child))
                        == fingerprint(os.stat(component, dir_fd=directory, follow_symlinks=False)),
                        'Artifact/component mutation or path substitution during read')
            require(total == item['size'] and hasher.hexdigest() == item['sha256'], 'Artifact content/hash differs')
            return {'bytes': b''.join(chunks), 'before': state, 'after': fingerprint(os.fstat(parent)),
                    'sha256': hasher.hexdigest(), 'size': total,
                    'contract': 'descriptor-nofollow-nonblocking-streaming-prepost-v1'}
        finally:
            for descriptor_fd in reversed(opened):
                os.close(descriptor_fd)


def issue(code, edge, message, scope_only=False):
    return {'code': code, 'edge': edge, 'message': message, 'scope_only': scope_only}


def empty_result():
    result = {'schema': OUTPUT_SCHEMA, 'diagnostic_only': True, 'contract_valid': False,
              'errors': [], 'blockers': [], 'joins': [], 'input_identity': None,
              'original_analyzer': None, 'presentation_analyzer': None,
              'api_presentation_latency': presentation.fixed_limits([]), 'outcomes': {},
              'api_metric_scope': 'within_fixed_limits is arithmetic only at 100000us p95/250000us '
              'maximum, from the unchanged analyzer and explicit complete exports; never qualification.',
              'producer_result': None, 'artifact_descriptors': {}, 'reader_observations': {},
              'analyzer_source_identity': {'path': ANALYZER_SOURCE, 'sha256': ANALYZER_SHA256,
                                          'scope': 'Frozen source contract, not loaded Python/image attestation'},
              'collector_source_identity': None,
              'journal': None, 'scope': 'External hash/receipt consistency only; no authenticated receipts, '
              'execution proof, loaded-image attestation, toolchain/SDK closure, visibility or eligibility.'}
    for name in ('acceptance_qualified', 'native_qualified', 'build_qualified', 'startup_qualified',
                 'performance_qualified', 'alpha_qualified', 'presentation_qualified', 'photon_qualified',
                 'driver_enabled', 'runtime_proof', 'visual_proof', 'worker_quiescence_proven',
                 'source_binary_receipt_integration_reviewed', 'toolchain_closure', 'sdk_closure',
                 'receipts_authenticated', 'loaded_image_attested', 'actual_execution_proven'):
        result[name] = False
    for name in ('eligible_seconds', 'civil_days', 'visibility', 'full_readiness', 'input_to_photon',
                 'worker_quiescence'):
        result[name] = None
    for code, message in (
        ('SOURCE_ONLY_NO_LOADED_ATTESTATION', 'Collector explicitly denies a loaded binary receipt/attestation.'),
        ('FROZEN_BUILDER_STALE_COLLECTOR_PIN', 'Frozen builder collector pin is stale; do not repin or execute.'),
        ('REVIEWED_BUILDER_EXECUTION_DISABLED', 'Reviewed successor remains execution-disabled; os.waitid '
         'is unavailable in its reviewed environment. No host probe is performed.'),
        ('FUTURE_DRIVER_REVIEW_REQUIRED', 'Driver rejection gates stay in place; future integration requires '
         'separate review/approval and launcher child_runner reuse.')):
        result['blockers'].append(issue(code, 'scope', message, True))
    return result


def read_combined(spec, *, reader, policy):
    """Read only the independent allowlist, then return offline diagnostics."""
    try:
        require(type(reader) is DescriptorReader, 'Trusted DescriptorReader instance required; no callback default')
        flat = contract(spec, policy)
    except (ValueError, TypeError, KeyError, UnicodeError, RecursionError, OverflowError) as error:
        result = empty_result()
        result['errors'].append(issue('INPUT_CONTRACT', 'input', str(error)))
        return result
    payloads, observations, failures = {}, {}, []
    for role, item in flat.items():
        try:
            observation = DescriptorReader.read(reader, item)
            payloads[role] = observation['bytes']
            observations[role] = {name: value for name, value in observation.items() if name != 'bytes'}
        except (OSError, ValueError, TypeError, KeyError, OverflowError) as error:
            failures.append(issue('ARTIFACT_READ', role, str(error)))
    bundle = {'schema': BUNDLE_SCHEMA, 'input': copy.deepcopy(spec), 'identity': digest(canonical(spec)),
              'payloads': payloads, 'observations': observations}
    result = analyze_combined(bundle, policy=policy)
    result['errors'] = failures + result['errors']
    result['contract_valid'] = result['contract_valid'] and not failures
    return result


def reject_errors(value):
    if type(value) is dict:
        for name, item in value.items():
            if name == 'error' or name.endswith('_error'):
                require(item is None or (type(item) is str and not item),
                        'Explicit or malformed application/observer error: ' + name)
            elif name.endswith('_errors'):
                require((type(item) is list and not item)
                        or (name in presentation.COUNTERS and type(item) is int and item == 0),
                        'Explicit or malformed application/observer errors: ' + name)
            require(not (name == 'error' or name.endswith(('_error', '_errors'))) or not item,
                    'Explicit application/observer error: ' + name)
            reject_errors(item)
    elif type(value) is list:
        for item in value:
            reject_errors(item)


def enriched_native_posts(raw):
    posts = raw['posts']
    require(type(posts) is list and len(posts) == 2, 'Missing raw native post timestamps')
    previous = 0
    for post, down in zip(posts, (True, False)):
        require(type(post) is dict and post['down'] is down
                and all(integer(post[name], 1) for name in (
                    'unix_before_ns', 'unix_after_ns', 'host_before_ns', 'host_after_ns'))
                and previous <= post['host_before_ns'] <= post['host_after_ns']
                and post['unix_before_ns'] <= post['unix_after_ns'], 'Invalid raw native post bracket')
        previous = post['host_after_ns']
    require(posts[0]['unix_after_ns'] <= posts[1]['unix_before_ns'], 'Regressing raw native post clocks')
    lower = (posts[1]['host_before_ns'] - posts[0]['host_after_ns']) / 1000
    upper = (posts[1]['host_after_ns'] - posts[0]['host_before_ns']) / 1000
    require(0 <= lower <= upper <= 1_000_000, 'Unbounded raw native press interval')
    duration = {'lower': lower, 'upper': upper,
        'clock': 'DispatchTime uptime nanoseconds; post-call brackets, not app receipt or presentation time'}
    require('press_duration_us' not in raw or same(raw['press_duration_us'], duration),
            'Raw stdout press duration contradicts deterministic legacy enrichment')
    derived = copy.deepcopy(raw)
    derived['press_duration_us'] = duration
    return derived


def selected_save_input(*, selected_bytes, selected_descriptor, pinned_bytes,
                        pinned_descriptor, launch_receipt):
    """Pure supplied-byte/launcher join. Receipt paths never authorize reads."""
    for raw, item in ((selected_bytes, selected_descriptor), (pinned_bytes, pinned_descriptor)):
        descriptor(item)
        require(type(raw) is bytes and 0 < len(raw) <= MAX_JSON_BYTES
                and same(item['size'], len(raw)) and item['sha256'] == digest(raw),
                'Selected input bytes differ from independent size/hash descriptor')
    require(selected_bytes == pinned_bytes, 'Selected source and pinned raw bytes differ')
    envelope = parse_json(selected_bytes)
    require(type(envelope) is dict and type(envelope.get('session')) is dict,
            'Selected input is not a player application envelope')
    session = envelope['session']
    require(type(session.get('options')) is dict and session['options'].get('routing') == 'Strict',
            'Actual selected input session.options.routing must be Strict')
    source = session.get('source')
    require(type(source) is str and bool(source) and '\0' not in source,
            'Actual selected input session source absent')
    require(type(launch_receipt) is dict and launch_receipt.get('source') == source
            and launch_receipt.get('input') == selected_descriptor['path']
            and launch_receipt.get('input_sha256') == selected_descriptor['sha256'],
            'Selected source/hash differs from launcher input/source receipt')
    command = launch_receipt.get('command')
    require(type(command) is list and all(type(argument) is str for argument in command),
            'Selected-input launch command malformed')
    require('--' not in command
            and not any(argument.startswith(flag + '=') for argument in command
                        for flag in ('--original', '--load')),
            'Selected input requires launcher-generated separate CLI options without a separator')
    for flag, expected in (('--original', source), ('--load', pinned_descriptor['path'])):
        require(command.count(flag) == 1 and command.index(flag) + 1 < len(command)
                and command[command.index(flag) + 1] == expected,
                'Selected input requires unique exact CLI ' + flag)
    inputs = launch_receipt.get('inputs')
    require(type(inputs) is list, 'Selected-input launcher pin array absent')
    matches = [item for item in inputs if type(item) is dict and item.get('kind') == 'selected_save']
    require(len(matches) == 1, 'Selected-input launcher pin missing or duplicated')
    pin = matches[0]
    require(pin.get('source') == selected_descriptor['path']
            and pin.get('pinned') == pinned_descriptor['path'] and pin.get('unchanged') is True,
            'Selected source/pinned launcher association changed')
    state_fields = {'sha256', 'size', 'mode', 'device', 'inode', 'links', 'mtime_ns', 'ctime_ns'}
    for before, after, item in (('before', 'after', selected_descriptor),
                                ('pinned_before', 'pinned_after', pinned_descriptor)):
        state = pin.get(before)
        require(type(state) is dict and set(state) == state_fields and same(state, pin.get(after))
                and state['sha256'] == item['sha256'] and same(state['size'], item['size'])
                and all(integer(state[name]) for name in ('size', 'device', 'inode', 'mtime_ns', 'ctime_ns'))
                and type(state['links']) is int and state['links'] == 1
                and integer(state['mode'], 0, 0o7777),
                'Selected launcher descriptor chain changed or malformed')
    return {'sha256': digest(selected_bytes), 'byte_count': len(selected_bytes),
            'requested_path': pinned_descriptor['path'], 'pinned_state': copy.deepcopy(pin['pinned_before'])}


def selected_input_observation(value, expected):
    fields = {'boundary', 'sha256', 'byte_count', 'requested_path', 'path_identity_observed',
              'descriptor_before', 'descriptor_after', 'descriptor_status'}
    require(type(value) is dict and set(value) == fields
            and value['boundary'] == 'player_application_envelope'
            and value['sha256'] == expected['sha256']
            and same(value['byte_count'], expected['byte_count'])
            and value['requested_path'] == expected['requested_path']
            and value['path_identity_observed'] is False and value['descriptor_status'] == 'consistent',
            'Loaded observation does not identify the supplied selected raw input')
    before, after = value['descriptor_before'], value['descriptor_after']
    fields = {'bytes', 'regular', 'modified_unix_ns', 'device', 'inode', 'mode',
              'change_seconds', 'change_nanoseconds'}
    require(type(before) is dict and set(before) == fields and same(before, after)
            and same(before['bytes'], expected['byte_count']) and before['regular'] is True
            and all(integer(before[name]) for name in ('bytes', 'device', 'inode', 'mode'))
            and stat.S_ISREG(before['mode'])
            and integer(before['change_seconds'], -(2**63), 2**63 - 1)
            and integer(before['change_nanoseconds'], 0, 999_999_999),
            'Loaded input lacks consistent same-open regular-file descriptors')
    pinned = expected['pinned_state']
    require(before['device'] == pinned['device'] and before['inode'] == pinned['inode']
            and stat.S_IMODE(before['mode']) == pinned['mode']
            and before['modified_unix_ns'] == str(pinned['mtime_ns'])
            and before['change_seconds'] * 1_000_000_000 + before['change_nanoseconds'] == pinned['ctime_ns'],
            'Loaded same-open descriptor differs from normalized launcher pinned object')


def selected_owner_snapshot(value, expected, *, diagnostics=None, terminal=False):
    """Validate v2 source facts at one immutable capture, never at draw time."""
    fields = {'extension', 'viewer_lifetime_ordinal', 'transition_ordinal', 'work_ordinal',
              'live_lifetime', 'accounted_slot', 'last_recorded_owner', 'active_work',
              'invalidation_reasons', 'last_operation', 'closed', 'ownership_status',
              'unresolved_in_flight', 'current_owner_asserted', 'eligible', 'snapshot_only',
              'worker_finalized', 'quiescence_proven', 'owner_preparation_complete',
              'owner_mutation_pending', 'publication_delivery_pending', 'capture_boundary',
              'ui_retirement_intent_lifetime', 'request_channel_closed_after_ui_retirement',
              'retired_lifetime'}
    require(type(value) is dict, 'Loaded owner snapshot absent')
    if terminal:
        require(set(value) == fields, 'Terminal loaded owner schema differs')
        observed = value
    else:
        require(set(value) == (fields - {'work_ordinal', 'active_work'} | {'capture_owner_matches_account'})
                and type(diagnostics) is dict and set(diagnostics) == {'work_ordinal', 'active_work'},
                'Loaded value and same-capture work diagnostics schema differ')
        observed = {**value, **diagnostics}
        require(observed['capture_owner_matches_account'] is True,
                'Captured actual owner differs from loaded account')
    require(observed['extension'] == LOADED_INPUT_CONTRACT
            and observed['snapshot_only'] is True and observed['worker_finalized'] is False
            and observed['quiescence_proven'] is False,
            'Unknown loaded input extension or unsupported finalization/quiescence claim')
    for name in ('viewer_lifetime_ordinal', 'transition_ordinal', 'work_ordinal'):
        require(integer(observed[name], 1), 'Invalid loaded owner ordinal: ' + name)
    lifetime = observed['viewer_lifetime_ordinal']
    for name in ('live_lifetime', 'ui_retirement_intent_lifetime', 'retired_lifetime'):
        require(observed[name] is None or integer(observed[name], 1, lifetime),
                'Invalid loaded owner lifetime: ' + name)
    active = observed['active_work']
    require(type(active) is list and all(integer(scope, 1, observed['work_ordinal']) for scope in active)
            and active == sorted(set(active)) and observed['invalidation_reasons'] == []
            and type(observed['invalidation_reasons']) is list,
            'Invalid, failed or malformed loaded owner work history')
    for name in ('closed', 'unresolved_in_flight', 'current_owner_asserted', 'eligible',
                 'owner_preparation_complete', 'owner_mutation_pending', 'publication_delivery_pending',
                 'request_channel_closed_after_ui_retirement'):
        require(type(observed[name]) is bool, 'Malformed loaded owner flag: ' + name)
    require(observed['unresolved_in_flight'] is bool(active)
            and observed['owner_mutation_pending'] is False,
            'Unresolved owner mutation or inconsistent scope diagnostics')
    slot = observed['accounted_slot']
    require(type(slot) is dict and set(slot) == {'lifetime', 'present', 'input', 'install'}
            and same(slot['lifetime'], lifetime) and slot['present'] is True
            and slot['install'] == 'installed' and same(observed['last_recorded_owner'], slot),
            'Loaded owner is absent, pending, failed, replaced or unattributed')
    selected_input_observation(slot['input'], expected)
    intent = observed['ui_retirement_intent_lifetime']
    require(not observed['request_channel_closed_after_ui_retirement'] or same(intent, lifetime),
            'Request-channel closure lacks matching UI retirement intent')
    if terminal:
        require(observed['closed'] is True and not active
                and observed['eligible'] is False and observed['current_owner_asserted'] is False
                and observed['publication_delivery_pending'] is False,
                'Terminal accounting is open, unresolved or claims a current eligible owner')
        if observed['live_lifetime'] is None:
            require(same(observed['retired_lifetime'], lifetime) and same(intent, lifetime)
                    and observed['request_channel_closed_after_ui_retirement'] is True
                    and observed['last_operation'] == 'viewer_registration_closed'
                    and observed['ownership_status'] == 'no_live_registration'
                    and observed['owner_preparation_complete'] is False
                    and observed['capture_boundary'] == 'owner_preparation_incomplete',
                    'Terminal registration retirement lacks exact source facts')
        else:
            require(same(observed['live_lifetime'], lifetime)
                    and observed['retired_lifetime'] != lifetime
                    and observed['last_operation'] == 'success'
                    and observed['ownership_status'] == 'accounted_at_snapshot'
                    and observed['owner_preparation_complete'] is True
                    and observed['capture_boundary'] == 'completed_owner_work',
                    'Terminal live owner lacks completed recorded work')
    else:
        require(observed['closed'] is False and same(observed['live_lifetime'], lifetime)
                and observed['retired_lifetime'] != lifetime and observed['eligible'] is True
                and observed['current_owner_asserted'] is True
                and observed['owner_preparation_complete'] is True
                and observed['publication_delivery_pending'] is bool(active)
                and observed['last_operation'] == ('owner_prepared' if active else 'success')
                and observed['ownership_status'] == ('owner_prepared_delivery_pending' if active else 'accounted_at_snapshot')
                and observed['capture_boundary'] == ('owner_prepared_before_read_only_publication'
                                                     if active else 'completed_owner_work'),
                'Frame attribution lacks completed owner preparation at its immutable capture')
    return observed


def selected_save_association(records, controls, *, selected_bytes, selected_descriptor,
                              pinned_bytes, pinned_descriptor, launch_receipt):
    """Observed input association through terminal accounting, not worker shutdown."""
    expected = selected_save_input(selected_bytes=selected_bytes, selected_descriptor=selected_descriptor,
        pinned_bytes=pinned_bytes, pinned_descriptor=pinned_descriptor, launch_receipt=launch_receipt)
    session, frames, _ = presentation.app_journal(records)
    require(type(session.get('extensions')) is list
            and all(type(extension) is str and bool(extension) for extension in session['extensions'])
            and len(session['extensions']) == len(set(session['extensions']))
            and session['extensions'].count(LOADED_INPUT_CONTRACT) == 1
            and 'sims-window-loaded-input-observation-v1' not in session['extensions'],
            'Reviewed loaded-input v2 extension required; no v1 attribution fallback')
    require(type(controls) is list and 1 <= len(controls) <= MAX_CONTROLS,
            'Selected input requires complete explicit control frame references')
    referenced = []
    for control in controls:
        require(type(control) is dict and type(control.get('control_effect')) is dict,
                'Selected-input control references malformed')
        effect = control['control_effect']
        require(type(effect.get('effect')) is dict, 'Selected-input effect reference absent')
        ids = (effect.get('baseline_frame_id'), effect['effect'].get('frame_id'), effect.get('closing_frame_id'))
        require(all(integer(identifier, 1) and identifier in frames for identifier in ids)
                and ids[0] <= ids[1] <= ids[2], 'Selected-input referenced frame absent or reordered')
        referenced.extend(ids)
    first = min(referenced)
    relevant = [frame for identifier, frame in frames.items() if identifier >= first]
    previous_work, history = 0, None
    for frame in relevant:
        state = frame['state']
        observed = selected_owner_snapshot(state['value'].get('loaded_input'), expected,
            diagnostics=state['diagnostics'].get('loaded_input_work'))
        identity = (observed['viewer_lifetime_ordinal'], observed['transition_ordinal'])
        require(history is None or same(identity, history),
                'Selected owner lifetime/transition changed within referenced frame interval or tail')
        require(observed['work_ordinal'] >= previous_work, 'Regressing same-capture loaded work ordinal')
        previous_work, history = observed['work_ordinal'], identity
    terminal = selected_owner_snapshot(records[-1]['data'].get('loaded_input'), expected, terminal=True)
    require(same((terminal['viewer_lifetime_ordinal'], terminal['transition_ordinal']), history)
            and terminal['work_ordinal'] >= previous_work,
            'Terminal selected owner history differs; stale publication or unobserved replacement')
    return {'sha256': expected['sha256'], 'byte_count': expected['byte_count'],
            'viewer_lifetime_ordinal': history[0], 'transition_ordinal': history[1],
            'frame_ids': [frame['frame_id'] for frame in relevant],
            'scope': 'Supplied selected input associated with immutable captures through terminal accounting snapshot; '
                     'not current owner at draw time, post-closure work exclusion, worker finalization or quiescence'}


def journal_records(payload):
    require(type(payload) is bytes and 0 < len(payload) <= MAX_JOURNAL_BYTES
            and payload.endswith(b'\n'), 'Missing, oversized or truncated journal')
    require(payload.count(b'\n') <= MAX_RECORDS, 'Journal record count bound exceeded')
    records = []
    nodes = [MAX_NODES]
    for line in payload.split(b'\n')[:-1]:
        require(bool(line) and not line.endswith(b'\r'), 'Blank/noncanonical journal line')
        record = parse_json(line)
        tree(record, budget=nodes)
        require(type(record) is dict and set(record) == {'kind', 'data', 'elapsed_us', 'dropped_records'}
                and type(record['kind']) is str and record['kind'] in ORDINARY_KINDS | PRESENTATION_KINDS
                and type(record['data']) is dict and integer(record['elapsed_us'])
                and type(record['dropped_records']) is int and record['dropped_records'] == 0,
                'Unknown/malformed/dropped journal record')
        reject_errors(record)
        records.append(record)
    presentation.app_journal(records)
    frames = {record['data']['frame_id']: (position, record['data'])
              for position, record in enumerate(records) if record['kind'] == 'frame'}
    contexts = set()
    for position, record in enumerate(records):
        event = record['data']
        if record['kind'] == 'presentation_event' and event.get('kind') == 2:
            identifier = event.get('frame_id')
            require(integer(identifier, 1) and identifier in frames, 'Draw context references missing frame')
            frame_position, frame = frames[identifier]
            require(position < frame_position and identifier not in contexts
                    and same(event.get('state_id'), frame['state']['state_id']),
                    'Draw context must precede exact immutable frame in original source order')
            contexts.add(identifier)
    return records


def analyze_combined(bundle, *, policy):
    """Pure, non-mutating analysis; no reads, callbacks, sorting or receipt path following."""
    result = empty_result()

    def blocker(code, edge, message):
        result['blockers'].append(issue(code, edge, message))

    def join(edge, operation):
        try:
            detail = operation()
            result['joins'].append({'edge': edge, 'class': 'matched', 'detail': detail})
        except KeyError as error:
            result['joins'].append({'edge': edge, 'class': 'missing', 'detail': str(error)})
            blocker('MISSING_EVIDENCE', edge, str(error))
        except ValueError as error:
            result['joins'].append({'edge': edge, 'class': 'mismatch', 'detail': str(error)})
            result['errors'].append(issue('JOIN_MISMATCH', edge, str(error)))
        except (TypeError, IndexError, AttributeError, OverflowError) as error:
            result['joins'].append({'edge': edge, 'class': 'mismatch', 'detail': str(error)})
            result['errors'].append(issue('MALFORMED_EVIDENCE', edge, str(error)))

    try:
        require(type(bundle) is dict and set(bundle) == {
            'schema', 'input', 'identity', 'payloads', 'observations'} and bundle['schema'] == BUNDLE_SCHEMA,
            'Unsupported exact bundle schema')
        spec = bundle['input']
        flat = contract(spec, policy)
        identity = digest(canonical(spec))
        require(type(bundle['identity']) is str and bundle['identity'] == identity, 'Bundle own identity mismatch')
        result['input_identity'] = identity
        result['artifact_descriptors'] = copy.deepcopy(flat)
        payloads = bundle['payloads']
        require(type(payloads) is dict and payloads.keys() <= flat.keys(), 'Unapproved payload role')
        require(type(bundle['observations']) is dict and bundle['observations'].keys() <= flat.keys(),
                'Unapproved reader observation role')
        tree(bundle['observations'])
        result['reader_observations'] = copy.deepcopy(bundle['observations'])
    except (ValueError, TypeError, KeyError, UnicodeError, RecursionError, OverflowError) as error:
        result['errors'].append(issue('INPUT_CONTRACT', 'input', str(error)))
        return result
    parsed, available = {}, set()
    for role, item in flat.items():
        try:
            payload = payloads[role]
            require(type(payload) is bytes and len(payload) == item['size']
                    and digest(payload) == item['sha256'], 'Payload differs from independent descriptor')
            observation = bundle['observations'].get(role)
            if observation is not None:
                require(type(observation) is dict and observation.get('contract') ==
                        'descriptor-nofollow-nonblocking-streaming-prepost-v1'
                        and observation.get('sha256') == item['sha256']
                        and same(observation.get('size'), item['size'])
                        and valid_file_fingerprint(observation.get('before'), item['size'])
                        and same(observation['before'], observation.get('after')),
                        'Malformed or changed descriptor observation')
            if role in REQUIRED_ROLES | OPTIONAL_ROLES and role != 'journal':
                value = parse_json(payload)
                require(type(value) is (list if role == 'controls' else dict), 'Wrong artifact scalar/object type')
                parsed[role] = value
            available.add(role)
            result['joins'].append({'edge': 'artifact:' + role, 'class': 'matched',
                                    'detail': 'Independently pinned bytes; not authenticated content'})
        except KeyError:
            blocker('ARTIFACT_MISSING', role, 'Explicit artifact bytes unavailable')
            result['joins'].append({'edge': 'artifact:' + role, 'class': 'missing', 'detail': 'No bytes'})
        except (ValueError, UnicodeError, TypeError, RecursionError, OverflowError) as error:
            result['errors'].append(issue('ARTIFACT_INVALID', role, str(error)))
            result['joins'].append({'edge': 'artifact:' + role, 'class': 'mismatch', 'detail': str(error)})
    bindings = policy['bindings']

    def artifact(name):
        role = bindings[name]
        if role not in available:
            raise KeyError('unavailable explicit artifact ' + role)
        return flat[role]

    def pin(items, kind, source, pinned, selector='kind'):
        require(type(items) is list, 'Missing receipt pin array')
        matches = [item for item in items if type(item) is dict and item.get(selector) == kind]
        require(len(matches) == 1, 'Missing/duplicate receipt pin: ' + kind)
        item = matches[0]
        require(item['source'] == source['path'] and item['pinned'] == pinned['path'], 'Source/pinned path differs')
        for first, last, expected in (('before', 'after', source), ('pinned_before', 'pinned_after', pinned)):
            require(type(item[first]) is dict and same(item[first], item[last])
                    and item[first]['sha256'] == expected['sha256']
                    and same(item[first]['size'], expected['size']), 'Changed/mismatched receipt pin state')
        require(source['sha256'] == pinned['sha256'], 'Source and pinned hashes differ')
        require('unchanged' not in item or item['unchanged'] is True, 'Receipt explicitly reports changed pin')
        return item

    def metadata_pin(name, item):
        matches = [entry for entry in diagnostic['inputs']
                   if type(entry) is dict and entry.get('name') == name]
        require(len(matches) == 1, 'Missing/duplicate independently hashed metadata pin')
        entry = matches[0]
        require(entry['pinned'] == item['path'] and type(entry['source']) is str,
                'Metadata role must describe the explicitly authorized pinned file')
        for before, after in (('before', 'after'), ('pinned_before', 'pinned_after')):
            require(same(entry[before], entry[after]) and entry[before]['sha256'] == item['sha256']
                    and same(entry[before]['size'], item['size']), 'Metadata pin content/accounting differs')
        require('unchanged' not in entry or entry['unchanged'] is True, 'Metadata pin marked changed')

    outcome_names = ('status', 'exit_code', 'actual_child_returncode', 'signal', 'child_outcome',
                     'primary_failure', 'post_errors', 'cleanup', 'forced_teardown', 'graceful_end',
                     'inputs_unchanged', 'report_error', 'timed_out', 'interrupted', 'journal_read_errors',
                     'journal_inspection_errors', 'partial_control_effect_diagnostic',
                     'control_export_status', 'control_export_error', 'controls_write_error')
    launch, diagnostic = parsed.get('launch', {}), parsed.get('result', {})
    result['producer_result'] = copy.deepcopy(parsed.get('result'))
    result['outcomes'] = {
        'raw_child_exit': copy.deepcopy(diagnostic.get('actual_child_returncode')),
        'diagnostic_code': copy.deepcopy(diagnostic.get('exit_code')),
        'launcher_code': copy.deepcopy(launch.get('launcher_exit_code')),
        'launcher_raw_exit': copy.deepcopy(launch.get('exit_code')),
        'result': {name: copy.deepcopy(diagnostic[name]) for name in outcome_names if name in diagnostic},
        'launcher': {name: copy.deepcopy(launch[name]) for name in (
            'process', 'diagnostic', 'launch_error', 'save_error', 'publication_error', 'integrity_error',
            'report_error', 'launcher_exit_code', 'exit_code') if name in launch},
        'campaign': {role: copy.deepcopy(parsed[role]) for role in ('campaign_build', 'campaign_run') if role in parsed},
        'presentation_build': copy.deepcopy(parsed.get('presentation_build')),
        'app_package_build': copy.deepcopy(parsed.get('app_package_build')),
        'native_commands': copy.deepcopy(diagnostic.get('native_commands')),
        'native_posts': copy.deepcopy(diagnostic.get('native_posts'))}

    def outcome_join():
        for name in ('actual_child_returncode', 'exit_code'):
            require(type(diagnostic[name]) is int, 'Child/diagnostic code must be exact integers')
        require(type(launch['launcher_exit_code']) is int and type(launch['exit_code']) is int,
                'Launcher codes must be exact integers')
        raw = diagnostic['actual_child_returncode']
        expected_signal = -raw if raw < 0 else None
        require(same(diagnostic['signal'], expected_signal), 'Raw exit and signal disagree')
        require(same(launch['exit_code'], raw) and same(launch['process']['returncode'], raw)
                and same(launch['launcher_exit_code'], diagnostic['exit_code'])
                and launch['process']['status'] == diagnostic['status'], 'Launcher/diagnostic exit layers disagree')
        require(same(launch['diagnostic'], diagnostic), 'Embedded and separate diagnostic result disagree')
        reject_errors({name: diagnostic[name] for name in (
            'post_errors', 'journal_read_errors', 'journal_inspection_errors', 'report_error') if name in diagnostic})
        reject_errors({name: launch[name] for name in (
            'process', 'launch_error', 'save_error', 'publication_error', 'integrity_error', 'report_error')
            if name in launch})
        require(diagnostic['primary_failure'] is None or type(diagnostic['primary_failure']) is str,
                'Malformed primary failure diagnostic')
        for name in ('timed_out', 'interrupted'):
            require(name not in diagnostic or type(diagnostic[name]) is bool,
                    'Malformed diagnostic outcome flag: ' + name)
        require(diagnostic['status'] == 'exited' and raw == diagnostic['exit_code'] == launch['launcher_exit_code'] == 0
                and diagnostic['graceful_end'] is True and diagnostic['forced_teardown'] is False
                and diagnostic['inputs_unchanged'] is True and diagnostic['primary_failure'] is None
                and diagnostic['post_errors'] == [] and not diagnostic.get('report_error')
                and not diagnostic.get('timed_out') and not diagnostic.get('interrupted')
                and not diagnostic.get('journal_read_errors') and not diagnostic.get('journal_inspection_errors')
                and not any(launch.get(name) for name in ('launch_error', 'save_error', 'publication_error',
                                                         'integrity_error', 'report_error')),
                'Failure/timeout/interruption/forced teardown/post/report errors prevent a clean diagnostic')
        require(type(diagnostic['cleanup']) is list and all(type(entry) is dict for entry in diagnostic['cleanup']),
                'Malformed cleanup diagnostics')
        reject_errors(diagnostic['cleanup'])
        require(not any(entry.get('operation') in ('terminate', 'kill') for entry in diagnostic['cleanup']),
                'Forced cleanup action conflicts with clean diagnostic claim')
        return 'Separate child, diagnostic and launcher codes; zero alone is not success'

    join('process_outcomes', outcome_join)

    def plan_join():
        ordinary.validate_specification(parsed['plan'], parsed['endpoints'], flat['plan']['sha256'])
        metadata_pin('plan.json', flat['plan'])
        metadata_pin('endpoints.json', flat['endpoints'])
        return 'Exact plan/endpoints and independent receipt descriptors'

    join('plan_endpoints', plan_join)

    def application_join():
        source, pinned = artifact('application'), artifact('application_pinned')
        pin(launch['inputs'], 'executable', source, pinned)
        require(launch['binary_sha256'] == source['sha256'] and launch['command'][0] == pinned['path'],
                'Launch command/executable hash differs from independently pinned application')
        artifact('launcher_source')
        return 'Candidate/pinned application and launcher source; campaign top executable/PID is NOT app identity'

    join('application_launcher', application_join)

    def build_join():
        build = parsed['campaign_build']
        require(type(build['exit_code']) is int and build['exit_code'] == 0 and build['timed_out'] is False
                and build['source_unchanged'] is True and not build.get('interrupted')
                and not build.get('disk_space_stop'), 'Not an unchanged successful campaign build')
        require(same(build['source_before'], policy['source_inventory'])
                and same(build['source_after'], policy['source_inventory']), 'Build source inventory differs')
        candidate = artifact('application')
        require(build['built_executables'][candidate['path']] == candidate['sha256'],
                'Campaign build does not bind application path/hash')
        for name in ('launcher_source', 'helper_source', 'driver_source', 'collector_source', 'analyzer_source'):
            artifact(name)
        return 'Complete expected source map and exact built application; not full toolchain closure'

    join('campaign_build_sources_application', build_join)

    def run_join():
        run = parsed['campaign_run']
        candidate = artifact('application')
        require(type(run['exit_code']) is int and run['exit_code'] == 0
                and run['timed_out'] is False and run['source_unchanged'] is True
                and run['inputs_unchanged'] is True and run['built_inputs_unchanged'] is True
                and run['executable_unchanged'] is True and not run.get('interrupted')
                and not run.get('disk_space_stop'), 'Unclean campaign wrapper outcome')
        require(same(run['source_before'], policy['source_inventory'])
                and same(run['source_after'], policy['source_inventory'])
                and run['build_receipt'] == flat['campaign_build']['path'], 'Campaign source/build receipt mismatch')
        for field in ('built_inputs_before', 'built_inputs_after'):
            require(run[field][candidate['path']] == candidate['sha256'], 'Campaign candidate executable mismatch')
        for field in ('inputs_before', 'inputs_after'):
            for name in ('launcher_source', 'helper_source', 'driver_source', 'collector_source', 'analyzer_source'):
                item = artifact(name)
                require(run[field][item['path']] == item['sha256'], 'Campaign source input pin mismatch')
        return 'Wrapper PID/executable deliberately not joined to application PID'

    join('campaign_run', run_join)

    def helper_join():
        build = parsed['helper_build']
        require(set(build) == {'schema', 'source_sha256', 'executable_sha256', 'source_path',
                               'binary_path', 'compiler', 'command'}
                and build['schema'] == 'sims-window-helper-build-v1', 'Helper exact seven-field schema required')
        source, binary, pinned = artifact('helper_source'), artifact('helper'), artifact('helper_pinned')
        require(build['source_path'] == source['path'] and build['binary_path'] == binary['path']
                and build['source_sha256'] == source['sha256']
                and build['executable_sha256'] == binary['sha256'], 'Helper source/binary receipt mismatch')
        command = build['command']
        require(type(build['compiler']) is str and bool(build['compiler'].strip()) and type(command) is list
                and len(command) >= 4 and all(type(item) is str for item in command)
                and command[0].split('/')[-1] == 'swiftc' and command.count(source['path']) == 1
                and command.count('-o') == 1 and command.index('-o') + 1 < len(command)
                and command[command.index('-o') + 1] == binary['path'], 'Helper compiler/command mismatch')
        pin(diagnostic['inputs'], 'native-helper', binary, pinned, 'name')
        metadata_pin('helper-build.json', flat['helper_build'])
        artifact('driver_source')
        return 'Seven-field helper receipt plus driver-pinned binary; no compiler/SDK closure'

    join('helper_build_driver_pin', helper_join)

    def collector_source_join():
        require(bindings['analyzer_source'] == 'source:' + ANALYZER_SOURCE
                and bindings['collector_source'] == 'source:' + COLLECTOR_SOURCE,
                'Analyzer and Rust collector must name their distinct repository source roles')
        analyzer, collector = artifact('analyzer_source'), artifact('collector_source')
        require(analyzer['sha256'] == ANALYZER_SHA256, 'Offline analyzer source contract differs')
        result['collector_source_identity'] = {'path': COLLECTOR_SOURCE, 'sha256': collector['sha256'],
                                             'scope': 'Independent source inventory, not binary attestation'}
        return 'Separate analyzer contract and independently pinned Rust collector source'

    join('offline_collector_source_contract', collector_source_join)
    records, controls = None, None
    if 'journal' in available:
        try:
            records = journal_records(payloads['journal'])
            result['journal'] = {'bytes': len(payloads['journal']), 'sha256': digest(payloads['journal']),
                                 'records': len(records), 'source_order_preserved': True,
                                 'scope': 'Independently reparsed supplied whole bytes, not retained-buffer claims'}
        except (ValueError, TypeError, KeyError, UnicodeError, RecursionError, OverflowError, IndexError) as error:
            result['errors'].append(issue('JOURNAL_INVALID', 'journal', str(error)))

    def wholefile_claims_join():
        require(records is not None, 'Independent journal parse failed')
        if 'journal_final_observation' not in diagnostic:
            raise KeyError('Legacy journal_on_disk/journal_buffer are not tagged final observations')
        count = final_observation_claim(diagnostic['journal_final_observation'], diagnostic,
            payloads['journal'], flat['journal'], bundle['observations'].get('journal'), flat,
            payloads, bundle['observations'])
        require(count == len(records), 'Final semantic record count contradicts supplied whole bytes')
        return 'Tagged descriptor observation matches explicit whole bytes; unauthenticated, not execution proof'

    def cached_claims_join():
        retained = diagnostic['journal_buffer']
        require(type(retained) is dict and integer(retained.get('bytes'), 0, len(payloads['journal']))
                and sha256(retained.get('sha256')), 'Malformed retained-buffer claim')
        size = retained['bytes']
        require(retained['sha256'] == digest(payloads['journal'][:size]), 'Retained buffer prefix differs')
        if 'journal_bytes' in diagnostic:
            require(same(diagnostic['journal_bytes'], size), 'Legacy retained-buffer byte alias differs')
        if 'journal_sha256' in diagnostic:
            require(diagnostic['journal_sha256'] == retained['sha256'], 'Legacy retained-buffer hash alias differs')
        if 'journal_final_observation' in diagnostic:
            final = diagnostic['journal_final_observation']
            require(type(final) is dict and type(final.get('retained_buffer')) is dict
                    and same(final['retained_buffer'].get('bytes'), size)
                    and final['retained_buffer'].get('sha256') == retained['sha256'],
                    'Final retained metadata contradicts separately reported legacy buffer')
        if 'records' in diagnostic:
            expected = ordinary.journal(payloads['journal'][:size], diagnostic['pid'], complete=False)
            require(same(diagnostic['records'], expected), 'Cached parsed records disagree with retained prefix')
        return 'Retained buffer checked only as an independently supplied prefix; never a whole-file claim'

    def legacy_disk_claims_join():
        disk = diagnostic['journal_on_disk']
        require(type(disk) is dict, 'Malformed legacy disk inspection claim')
        if disk.get('sha256') is not None:
            require(sha256(disk['sha256']) and disk['sha256'] == digest(payloads['journal']),
                    'Legacy populated disk hash differs from supplied journal')
        if 'size' in disk:
            require(same(disk['size'], len(payloads['journal'])), 'Legacy disk size differs')
        return 'Legacy inspection claims checked separately; never whole-file observation authority'

    join('journal_legacy_disk_claims', legacy_disk_claims_join)
    join('journal_wholefile_claims', wholefile_claims_join)
    join('journal_retained_buffer_claims', cached_claims_join)

    def controls_join():
        nonlocal controls
        exported = parsed['controls']
        if 'control_export_contract' in diagnostic:
            require(type(diagnostic['control_export_contract']) is str
                    and diagnostic['control_export_contract'] == CAMERA_EXPORT_CONTRACT,
                    'Unknown control export contract; no raw-result fallback')
            require(diagnostic['control_export_status'] == 'exported_unqualified'
                    and diagnostic['control_export_offline_only'] is True
                    and diagnostic['control_export_qualified'] is False
                    and diagnostic['control_export_integration_reviewed'] is False
                    and diagnostic['control_export_driver_enabled'] is False
                    and diagnostic['control_export_error'] is None and diagnostic['controls_write_error'] is None,
                    'Versioned camera export must remain clean, unqualified and offline-only')
            require(same(diagnostic['exported_controls'], exported),
                    'Controls artifact differs from exact diagnostic exported list')
        require(1 <= len(exported) <= MAX_CONTROLS, 'Control count bound exceeded')
        for index, control in enumerate(exported):
            require(type(control) is dict, 'Control envelope must be an object')
            missing = CONTROL_FIELDS - control.keys()
            if missing:
                raise KeyError('complete exported control envelope missing ' + ','.join(sorted(missing)))
            require(set(control) == CONTROL_FIELDS, 'Unknown exported control envelope fields')
            if index == len(exported) - 1 and control['next_started_unix_ns'] is not None:
                blocker('LAST_CONTROL_CLOSE_CUTOFF_CONFLICT', 'controls',
                        'Current runner uses close-start cutoff; final export must instead have next-start None')
                raise ValueError('Last control next_started_unix_ns must be None; do not invent an export')
            require(integer(control['fence'], 1) and integer(control['started_unix_ns'], 1)
                    and integer(control['finished_unix_ns'], 1)
                    and (control['next_started_unix_ns'] is None or integer(control['next_started_unix_ns'], 1)),
                    'Control fence/time scalar types invalid')
            reject_errors(control)
        controls = exported
        return 'Explicit complete per-control exports only; never reconstructed from nested posts/timestamps'

    join('exported_controls', controls_join)
    if controls is None:
        blocker('PRODUCER_CONTROL_EXPORT_MISSING', 'controls', 'Current runner does not export complete per-control envelopes')

    def selected_save_join():
        if records is None or controls is None:
            raise KeyError('Valid whole journal and explicit controls required for selected input association')
        for role in ('selected_save', 'selected_save_pinned'):
            if role not in available:
                raise KeyError('Explicit independently pinned raw bytes unavailable: ' + role)
        return selected_save_association(records, controls,
            selected_bytes=payloads['selected_save'], selected_descriptor=flat['selected_save'],
            pinned_bytes=payloads['selected_save_pinned'], pinned_descriptor=flat['selected_save_pinned'],
            launch_receipt=launch)

    extensions = records[0]['data'].get('extensions') if records is not None else None
    if type(extensions) is list and LOADED_INPUT_CONTRACT in extensions:
        join('selected_save_session_identity', selected_save_join)
        selected_matched = result['joins'][-1]['class'] == 'matched'
    else:
        selected_matched = False
        result['joins'].append({'edge': 'selected_save_session_identity', 'class': 'unverifiable',
                                'detail': 'Missing reviewed v2 producer contract; no private save path is read'})
    if not selected_matched:
        blocker('PRODUCER_JOURNAL_SAVE_IDENTITY_MISSING', 'selected_save_session_identity',
                'Complete reviewed v2 selected-input association is unavailable; receipt paths, '
                'household labels, v1 snapshots and synthetic fields cannot replace the required contract')

    def identity_join():
        if records is None:
            raise KeyError('valid journal required for identity joins')
        session = records[0]['data']
        pid = diagnostic['pid']
        require(integer(pid, 1, 2**31 - 1) and same(session['pid'], pid), 'Result/application session PID differs')
        presentation.binding(diagnostic['binding'], pid)
        require(same(diagnostic['kernel_identity'], diagnostic['binding']['kernel_identity']),
                'Result helper/kernel lifetime identity differs')
        require(diagnostic['binding']['executable'] == artifact('application_pinned')['path'],
                'Kernel executable differs from launched pinned application')
        if 'kernel_identity' not in session or 'build_ids' not in session:
            blocker('PRODUCER_JOURNAL_LIFETIME_BUILD_IDS_MISSING', 'application_identity',
                    'Current journal lacks kernel lifetime and application/helper/presentation build IDs')
            raise KeyError('explicit journal kernel_identity and build_ids required; no reconstruction')
        require(same(session['kernel_identity'], diagnostic['kernel_identity']), 'Journal PID reuse/kernel lifetime mismatch')
        expected_ids = {name: artifact(binding)['sha256'] for name, binding in (
            ('application_sha256', 'application_pinned'), ('helper_sha256', 'helper_pinned'),
            ('presentation_sha256', 'presentation_pinned'))}
        require(same(session['build_ids'], expected_ids), 'Journal build IDs differ from independent binary pins')
        if controls is None:
            raise KeyError('complete exported controls required for every helper binding')
        for control in controls:
            for binding in (control['binding_before'], control['binding_after'], control['native_result']['binding']):
                presentation.binding(binding, pid)
                require(same(binding, diagnostic['binding']), 'Per-control PID lifetime/window/bounds/focus/display mismatch')
        return 'Exact scalar PID/lifetime/executable/window/geometry/focus/display join, not live identity proof'

    join('application_identity', identity_join)

    def native_controls_join():
        if controls is None:
            raise KeyError('complete exported controls required for native associations')
        commands = diagnostic['native_commands']
        require(type(commands) is list and len(commands) <= MAX_RECORDS, 'Unbounded/malformed native command diagnostics')
        responses = []
        for command in commands:
            reject_errors(command)
            request = command['request']
            require(same(request['pid'], diagnostic['pid']) and request['executable'] == artifact('application_pinned')['path']
                    and type(command['returncode']) is int and command['returncode'] == 0,
                    'Helper request PID/executable/outcome differs')
            require(integer(command['started_unix_ns'], 1) and integer(command['finished_unix_ns'], 1)
                    and command['started_unix_ns'] <= command['finished_unix_ns'],
                    'Helper native-call bracket invalid')
            require(request['operation'] == 'observe' or 'binding' in request,
                    'Non-observe helper request requires explicit binding')
            if 'binding' in request:
                require(same(request['binding'], diagnostic['binding']), 'Helper request window/lifetime differs')
            response = parse_json(command['stdout'].encode('utf-8', errors='strict'))
            require(type(response) is dict, 'Helper response must be an object')
            reject_errors(response)
            if 'binding' in response:
                presentation.binding(response['binding'], diagnostic['pid'])
                require(same(response['binding'], diagnostic['binding']), 'Helper response binding differs')
                require('status' not in response and response['operation'] == request['operation']
                        and request['operation'] in ('observe', 'key', 'release', 'close'),
                        'Helper response operation/status contradicts request')
                if request['operation'] in ('key', 'release'):
                    code = request['code'] if request['operation'] == 'key' else request.get('code', 124)
                    require(integer(code) and code in (124, 36) and same(response['code'], code)
                            and response['released'] is True, 'Helper key/release semantics differ')
                if request['operation'] == 'key':
                    require(integer(request['press_ms']) and request['press_ms'] == 20
                            and same(response['press_ms'], request['press_ms']), 'Helper press duration differs')
            elif response.get('status') == 'not_ready':
                require(request['operation'] == 'observe'
                        and set(response) == {'status', 'reason', 'kernel_identity'}
                        and response['reason'] in ('owned_window_unavailable', 'foreground_application_unavailable')
                        and same(response['kernel_identity'], diagnostic['kernel_identity']),
                        'Initial helper kernel PID lifetime differs')
            else:
                raise ValueError('Helper response lacks explicit binding/lifetime')
            responses.append(response)
        associations = diagnostic['control_native_associations']
        require(type(associations) is list and len(associations) == len(controls) == 1,
                'Current driver has exactly one control/native_posts result per invocation')
        used_keys = []
        for control_index, (association, control) in enumerate(zip(associations, controls)):
            require(type(association) is dict and set(association) == {
                'control_index', 'before_command_index', 'key_command_index', 'after_command_index'},
                'Exact explicit control-to-native association required')
            require(integer(association['control_index']) and association['control_index'] == control_index,
                    'Association control index differs')
            indices = [association[name + '_command_index'] for name in ('before', 'key', 'after')]
            require(all(integer(index, 0, len(commands) - 1) for index in indices)
                    and indices[0] < indices[1] < indices[2], 'Invalid/reordered helper association indices')
            before_index, key_index, after_index = indices
            require(commands[before_index]['finished_unix_ns'] <= control['started_unix_ns']
                    <= commands[key_index]['started_unix_ns'] <= commands[key_index]['finished_unix_ns']
                    <= control['finished_unix_ns'] <= commands[after_index]['started_unix_ns'],
                    'Associated helper calls contradict explicitly exported enclosing window')
            for index, expected_binding in ((before_index, control['binding_before']),
                                            (after_index, control['binding_after'])):
                require(commands[index]['request']['operation'] == responses[index]['operation'] == 'observe'
                        and same(responses[index]['binding'], expected_binding),
                        'Associated observation binding/operation differs')
            require(commands[key_index]['request']['operation'] == 'key'
                    and same(responses[key_index], control['native_result']),
                    'Exported native result differs from associated actual helper response')
            if diagnostic.get('control_export_contract') == CAMERA_EXPORT_CONTRACT:
                require(same(diagnostic['native_posts_raw_v2'], responses[key_index]),
                        'Versioned raw result differs from exact decoded helper stdout')
                require(same(diagnostic['native_posts'], enriched_native_posts(responses[key_index])),
                        'Legacy native_posts differs from deterministic raw-result enrichment')
            else:
                require(same(diagnostic['native_posts'], responses[key_index]),
                        'Exported native result/native_posts differs from associated actual helper response')
            posts = responses[key_index]['posts']
            require(type(posts) is list and len(posts) == 2 and all(
                type(post) is dict and integer(post['unix_before_ns']) and integer(post['unix_after_ns'])
                and commands[key_index]['started_unix_ns'] <= post['unix_before_ns']
                <= post['unix_after_ns'] <= commands[key_index]['finished_unix_ns']
                for post in posts), 'Native post brackets escape their associated helper invocation')
            used_keys.append(key_index)
        require(used_keys == [index for index, command in enumerate(commands)
                              if command['request']['operation'] == 'key'], 'Unassociated or repeated key command')
        return 'Explicit producer association to decoded helper semantics; enclosing times are not inferred'

    join('control_native_results', native_controls_join)

    def presentation_build_join():
        build = parsed['presentation_build']
        require(type(build['schema']) is int and build['schema'] == 1
                and build['kind'] == 'window_presentation_source_build' and build['mode'] == 'execute',
                'Presentation source-build receipt schema/kind/mode differs')
        process, cleanup = build['process'], build['cleanup']
        require(build['outcome'] == 'built_unqualified' and process['attempted'] is True
                and type(process['returncode']) is int and process['returncode'] == 0
                and process['timed_out'] is False and process['error'] is None
                and cleanup['reaped'] is True and type(cleanup['returncode']) is int
                and cleanup['returncode'] == 0 and cleanup['errors'] == []
                and build['setup_error'] is None and build['receipt_write_error'] is None
                and build['integrity_errors'] == [] and not build.get('primary_failure')
                and not build.get('post_errors'), 'Presentation build process/cleanup/integrity failure')
        require(type(build['qualifications']) is dict and bool(build['qualifications'])
                and all(value is False for value in build['qualifications'].values()), 'Build must remain unqualified')
        binary, pinned = artifact('presentation'), artifact('presentation_pinned')
        require(build['output']['path'] == binary['path'] and build['output_path'] == binary['path']
                and build['output']['state']['sha256'] == binary['sha256']
                and same(build['output']['state']['size'], binary['size']), 'Presentation output state/hash differs')
        pairs = bindings['presentation_sources']
        require(bool(pairs) and type(build['sources']) is list, 'Explicit source/staged pairs required')
        command = build['command']
        require(type(command) is list and len(command) >= 4 and all(type(item) is str for item in command)
                and command[0] == build['compiler']['path'] and command.count('-o') == 1
                and command.index('-o') + 1 < len(command)
                and command[command.index('-o') + 1] == binary['path'],
                'Presentation compiler/command/output path mismatch; compiler itself is not attested')
        seen = set()
        for source_role, staged_role in pairs.items():
            source = flat[source_role]
            staged = flat[staged_role] if staged_role is not None else None
            require(source_role in available and (staged_role is None or staged_role in available),
                    'Missing approved source/staged bytes')
            matches = [item for item in build['sources'] if item.get('path') == source['path']]
            require(len(matches) == 1, 'Missing/duplicate presentation build source')
            item = matches[0]
            require(item['sha256'] == source['sha256']
                    and same(item['before'], item['after']) and item['before']['sha256'] == source['sha256']
                    and same(item['before']['size'], source['size']), 'Presentation source integrity mismatch')
            if staged is not None:
                require(source['sha256'] == staged['sha256'] and item['staged_path'] == staged['path']
                        and same(item['staged_before'], item['staged_after'])
                        and item['staged_before']['sha256'] == staged['sha256']
                        and same(item['staged_before']['size'], staged['size']), 'Presentation staged integrity mismatch')
            else:
                require(item['staged_path'] is None and item['staged_before'] is None
                        and item['staged_after'] is None, 'Unexpected staged copy for unstaged source')
            seen.add(source_role.split(':', 1)[1])
        require({'crates/sims-app/native/window_present_macos.m',
                 'crates/sims-app/native/window_present_macos.h',
                 'crates/sims-app/src/window_session/presentation.rs'} <= seen,
                'Native .m/.h source AND staging pins, plus collector Rust source, required')
        for name in ('crates/sims-app/native/window_present_macos.m', 'crates/sims-app/native/window_present_macos.h'):
            require(pairs['source:' + name] is not None, 'Native compiler inputs must have explicit staged source pins')
        native_source = flat[pairs['source:crates/sims-app/native/window_present_macos.m']]
        native_header = flat[pairs['source:crates/sims-app/native/window_present_macos.h']]
        require(native_source['path'].rsplit('/', 1)[-1] == 'window_present_macos.m'
                and native_header['path'] == native_source['path'].rsplit('/', 1)[0] + '/window_present_macos.h',
                'Staged native source/header must have reviewed co-located exact basenames')
        require(command.count(native_source['path']) == 1, 'Compiler command does not use exact staged native source')
        for item in build['sources']:
            require(type(item) is dict and same(item['before'], item['after'])
                    and item['before']['sha256'] == item['sha256'], 'Additional claimed build input changed')
        pin(launch['inputs'], 'presentation_library', binary, pinned)
        command = launch['command']
        require(command.count('--window-session-present-library') == 1
                and command[command.index('--window-session-present-library') + 1] == pinned['path'],
                'CLI presentation library path differs')
        if records is None:
            raise KeyError('valid journal required for collector library-path join')
        libraries = [record['data'] for record in records if record['kind'] == 'presentation_library']
        require(len(libraries) == 1 and libraries[0]['path'] == pinned['path'], 'Collector descriptive library path differs')
        library = libraries[0]
        require(library['source_only'] is True and library['loaded_binary_receipt'] is False
                and library['loaded_binary_source_attested'] is False, 'Source-only collector cannot attest loaded image')
        if 'qualifications' in library:
            require(type(library['qualifications']) is dict and all(value is False for value in library['qualifications'].values()),
                    'Collector qualifications must stay false')
        for suffix, field in (('.m', 'source_sha256'), ('.h', 'header_sha256')):
            matching = [flat[role]['sha256'] for role in pairs if role.endswith('window_present_macos' + suffix)]
            require(len(matching) == 1 and library[field] == matching[0], 'Collector native source/header pin differs')
        artifact('collector_source')
        return 'Source/staged/output/launcher-pin/CLI/descriptive collector path; NOT loaded binary attestation'

    join('presentation_build_source_output_library', presentation_build_join)
    result['joins'].append({'edge': 'loaded_image_attestation', 'class': 'unverifiable',
                            'detail': 'Source-only collector explicitly denies loaded binary receipt and source attestation'})
    if 'app_package_build' in parsed:
        result['joins'].append({'edge': 'app_package_build_source', 'class': 'unverifiable',
                                'detail': 'Package executable_sha256 alone is insufficient source proof'})
        blocker('PACKAGE_HASH_NOT_SOURCE_PROOF', 'app_package_build_source',
                'Use the unchanged campaign build and independent complete source inventory instead')
        candidate = bindings.get('application')
        if candidate in flat and parsed['app_package_build'].get('executable_sha256') != flat[candidate]['sha256']:
            result['errors'].append(issue('PACKAGE_EXECUTABLE_MISMATCH', 'app_package_build', 'Package executable hash differs'))
    if records is not None:
        try:
            projection = [record for record in records if record['kind'] in ORDINARY_KINDS]
            projected_bytes = b''.join(canonical(record) + b'\n' for record in projection)
            checked = ordinary.journal(projected_bytes, diagnostic['pid'])
            result['original_analyzer'] = ordinary.summarize(checked, copy.deepcopy(diagnostic.get('control_effect')))
            result['original_analyzer_scope'] = 'Checked ordinary projection only; full records preserved separately'
        except (ValueError, TypeError, KeyError, UnicodeError, RecursionError, OverflowError) as error:
            result['errors'].append(issue('ORIGINAL_ANALYZER_INVALID', 'ordinary_projection', str(error)))
        if controls is not None:
            result['presentation_analyzer'] = presentation.analyze(records, controls)
            if not result['presentation_analyzer']['valid']:
                result['errors'].append(issue('PRESENTATION_ANALYZER_INVALID', 'presentation',
                                             '; '.join(result['presentation_analyzer']['errors'])))
            else:
                result['api_presentation_latency'] = copy.deepcopy(result['presentation_analyzer']['api_presentation_latency'])
    result['contract_valid'] = not result['errors'] and not any(not item['scope_only'] for item in result['blockers'])
    return result
