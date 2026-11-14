"""Read-only raw neighborhood gameplay owners for a stopped Sims1.91 boundary.

No guest API, target execution, writes, inferred values, or Rust normalization.
The caller supplies an exact memory.read(address, size) adapter and is responsible
for stopping the original and verifying the pinned playable image. See PROVENANCE.
Limits below are observer safety limits, not claims about native allocation caps.
Used vector payloads are retained; unused capacity bytes are not semantic entries.
"""
import struct


SCHEMA = 1
PLAYABLE_SHA256 = 'a4d9f7134f52a186685961800608af72c23036ba62637eb9a99d75d4c5b1b321'
GLOBAL_OWNER = 0x6e48bc
PROVENANCE = {
    'neighborhood_root': 'qemu_startup_context.py; manager+24; application+34 cross-reference',
    'neighbor_slots': '4256f4/425b15; native_neighbor_metadata.py; header+148',
    'neighbor_record': '423736/430ad6 allocation0x274; native_neighbor_attachment.py; person+74/revision+18',
    'neighbor_friends': '427148; native_neighbor_friend_count.py; cache+20/dirty+24',
    'relationships': '44d17c; native_relationship.py; record+c vector of pointers; row{values vector,target+12}',
    'families': '424806 allocation0x15c;41003c/4104ce; native_family_removal.py/native_family_magic.py; header+154/members+12c',
    'family_friends': '427bd7/native_home_friends.py; cached count+120/dirty+124',
    'inventories': '4254f6/41f369/41f779; native_neighborhood_read.py; header+194; record{neighbor,vector+4}; item stride12',
    'neighborhood_words_options': '4254f6; native_neighborhood_read.py;16shorts+128/11bytes+1a0',
    'coordinates_nearby_zoning': 'observe_neighborhood.py/qemu_startup_context.py; headers+110/+11c/+174',
    'person_order': 'native_next_family.py/native_family_removal.py; registry+84 pointer vector; prefix+80 retained opaque',
    'relationship_dirty': '44d17c/native_relationship.py; registry+12 byte',
    'process_options': '45dd39; native_person_main.py; free_will byte6c2980/pet-autonomy word6e4f68',
    'friend_threshold': '4271c5 compares both first relationship columns against signed dword6c4398; capture runtime value, not a pinned image default',
    'autonomy_constants_loaded': '45962b/459844 checks raw process byte6e5291 before first-person459120 FCNS2 initialization; capture independently of threshold',
    'linked_resource': 'native_neighbor_metadata.py/native_neighbor_friend_count.py; record+8 resource+60 definition/resource+b4 flags',
}


class ObservationError(ValueError):
    pass


class Reader:
    def __init__(self, memory, max_total_bytes):
        if isinstance(max_total_bytes, bool) or not isinstance(max_total_bytes, int) or max_total_bytes < 4:
            raise ObservationError('invalid observer byte budget')
        self.memory = memory
        self.limit = max_total_bytes
        self.total = 0
        self.reads = []

    def read(self, address, size, path):
        if (isinstance(address, bool) or not isinstance(address, int)
                or not isinstance(size, int) or size < 0 or address < 0x10000
                or address + size > 0x100000000):
            raise ObservationError(f'{path}: invalid32-bit read {address!r}+{size!r}')
        if self.total + size > self.limit:
            raise ObservationError(f'{path}: observer byte budget exceeded')
        if size == 0:
            return b''
        try:
            raw = self.memory.read(address, size)
        except Exception as error:
            raise ObservationError(f'{path}: unreadable {address:#x}+{size:#x}: {error}') from error
        if not isinstance(raw, (bytes, bytearray)) or len(raw) != size:
            raise ObservationError(f'{path}: short/nonbyte read at {address:#x}, expected{size}')
        self.total += size
        self.reads.append(dict(path=path, address=address, size=size))
        return bytes(raw)

    def word(self, address, path):
        raw = self.read(address, 4, path)
        return struct.unpack('<I', raw)[0], raw.hex()

    def vector(self, address, stride, path, maximum=32768):
        header = self.read(address, 12, path + '.header')
        begin, end, capacity = struct.unpack('<3I', header)
        if begin == 0:
            if end or capacity:
                raise ObservationError(f'{path}: partially null vector')
        elif (begin < 0x10000 or begin % 4 or not begin <= end <= capacity
              or (end-begin) % stride or (capacity-begin) % stride
              or (capacity-begin)//stride > maximum):
            raise ObservationError(f'{path}: invalid/beyond-bound vector extent')
        payload = b'' if begin == 0 else self.read(begin, end-begin, path + '.payload')
        return dict(header_address=address, header=header.hex(), begin=begin,
                    end=end, capacity=capacity, stride=stride, bytes=payload.hex())

    def binding(self, resource, path):
        if resource == 0:
            return None
        definition, definition_word = self.word(resource + 0x60, path + '.definition_pointer')
        flags = self.read(resource + 0xb4, 4, path + '.flags')
        raw = None if definition == 0 else self.read(definition, 0xd8, path + '.definition').hex()
        return dict(resource=resource, definition=definition,
                    definition_pointer_bytes=definition_word, flags_bytes=flags.hex(), definition_storage=raw)

    def relationships(self, address, path):
        vector = self.vector(address, 4, path)
        rows = []
        for index, (pointer,) in enumerate(struct.iter_unpack('<I', bytes.fromhex(vector['bytes']))):
            if pointer == 0:
                rows.append(None)
                continue
            row_path = f'{path}.rows[{index}]'
            raw = self.read(pointer, 16, row_path + '.storage')
            rows.append(dict(pointer=pointer, storage=raw.hex(),
                             values=self.vector(pointer, 4, row_path + '.values', maximum=65536)))
        return dict(**vector, rows=rows)

    def family(self, pointer, path):
        if pointer == 0:
            return None
        raw = self.read(pointer, 0x15c, path + '.storage')
        #4100e0 destroys these independently. They are NOT another vector's
        #begin/end/capacity triple; their payload extents are not established.
        unresolved = [dict(offset=offset, pointer=struct.unpack_from('<I',raw,offset)[0],
                           status='observed pointer; external payload layout unresolved')
                      for offset in (0x140,0x144,0x148)]
        return dict(pointer=pointer, storage=raw.hex(),
                    members=self.vector(pointer + 0x12c, 4, path + '.members'),
                    unresolved_external_owners=unresolved)


def capture(memory, owner=None, *, max_total_bytes=64*1024*1024):
    """Return raw observed owners; invalid/unreadable payloads raise with path.

    An explicit null pointer is retained, never treated as an unreadable owner.
    Holes, duplicate pointers, GUIDs and relationship targets stay ordered. No
    cached field is recomputed or zero-filled, including revision/padding bytes.
    Capturing memory is not itself proof of an atomic boundary or image identity.
    """
    r = Reader(memory, max_total_bytes)
    actual_owner, root_bytes = r.word(GLOBAL_OWNER, 'owner_pointer')
    if owner is not None and owner != actual_owner:
        raise ObservationError('supplied owner differs from observed global owner')
    result = dict(schema=SCHEMA, scope='raw supplied-reader neighborhood owners; not normalized parity',
                  playable_sha256_required=PLAYABLE_SHA256, owner=actual_owner,
                  global_owner_bytes=root_bytes)
    if actual_owner == 0:
        return dict(**result, status='observed_null_owner', reads=r.reads, total_bytes=r.total)
    owner_storage = r.read(actual_owner, 0x28, 'owner_storage')
    application = struct.unpack_from('<I', owner_storage)[0]
    neighborhood = struct.unpack_from('<I', owner_storage, 0x24)[0]
    result.update(owner_storage=owner_storage.hex(), application=application, neighborhood=neighborhood)
    if application == 0 or neighborhood == 0:
        return dict(**result, status='observed_null_application_or_neighborhood', reads=r.reads, total_bytes=r.total)
    application_storage = r.read(application, 0x38, 'application_storage')
    selected = struct.unpack_from('<I', application_storage, 0x10)[0]
    registry = struct.unpack_from('<I', application_storage, 0x14)[0]
    result.update(application_storage=application_storage.hex(), registry=registry,
                  selected_family=selected, neighborhood_storage=r.read(neighborhood, 0x1ac, 'neighborhood_storage').hex())
    for name, offset, stride in [('coordinates',0x110,8), ('nearby_houses',0x11c,4), ('zoning',0x174,8)]:
        result[name] = r.vector(neighborhood + offset, stride, name)
    neighbors = r.vector(neighborhood + 0x148, 4, 'neighbors')
    rows = []
    for index, (pointer,) in enumerate(struct.iter_unpack('<I', bytes.fromhex(neighbors['bytes']))):
        if pointer == 0:
            rows.append(None)
            continue
        path = f'neighbors.rows[{index}]'
        raw = r.read(pointer, 0x274, path + '.storage')
        resource = struct.unpack_from('<I', raw, 8)[0]
        rows.append(dict(pointer=pointer, storage=raw.hex(), linked_resource=r.binding(resource,path+'.resource'),
                         relationships=r.relationships(pointer + 0xc, path + '.relationships')))
    result['neighbors'] = dict(**neighbors, rows=rows)
    families = r.vector(neighborhood + 0x154, 4, 'families')
    result['families'] = dict(**families, rows=[r.family(pointer, f'families.rows[{index}]')
        for index, (pointer,) in enumerate(struct.iter_unpack('<I', bytes.fromhex(families['bytes'])))])
    result['selected_family_owner'] = r.family(selected, 'selected_family_owner')
    inventories = r.vector(neighborhood + 0x194, 4, 'inventories')
    rows = []
    for index, (pointer,) in enumerate(struct.iter_unpack('<I', bytes.fromhex(inventories['bytes']))):
        if pointer == 0:
            rows.append(None)
            continue
        path = f'inventories.rows[{index}]'
        rows.append(dict(pointer=pointer, storage=r.read(pointer,16,path+'.storage').hex(),
                         items=r.vector(pointer+4,12,path+'.items')))
    result['inventories'] = dict(**inventories, rows=rows)
    result['person_order'] = None if registry == 0 else r.vector(registry+0x84,4,'person_order')
    result['registry_relationship_dirty_bytes'] = None if registry == 0 else r.read(registry+0x12,1,'registry_relationship_dirty').hex()
    result['person_order_prefix_bytes'] = None if registry == 0 else r.read(registry+0x80,4,'person_order_prefix').hex()
    result['process_options'] = dict(free_will_address=0x6c2980,
        free_will_bytes=r.read(0x6c2980,1,'process_options.free_will').hex(),
        pet_autonomy_address=0x6e4f68, pet_autonomy_bytes=r.read(0x6e4f68,4,'process_options.pet_autonomy').hex(),
        friend_threshold_address=0x6c4398,
        friend_threshold_bytes=r.read(0x6c4398,4,'process_options.friend_threshold').hex(),
        autonomy_constants_loaded_address=0x6e5291,
        autonomy_constants_loaded_bytes=r.read(0x6e5291,1,'process_options.autonomy_constants_loaded').hex())
    result.update(status='captured', reads=r.reads, total_bytes=r.total)
    return result
