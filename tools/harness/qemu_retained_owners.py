"""Current stopped-boundary owners; no historical-page or inferred defaults.

Only addresses/layouts already exercised by the existing native probes are read.
Frame contexts retain their reverse resource identity chain; original filename
and provider identity remain a separate, unresolved observation contract.
"""
import struct


SCHEMA = 1
FIELDS = {
    'resource_globals', 'motive_continuations', 'physical_placement',
    'registry_storage_and_order', 'object_native_flags', 'object_definition_identity',
}


def vector(m, pointer, stride, maximum):
    first, end, capacity = m.words(pointer, 3)
    if first == 0:
        if end or capacity:
            raise ValueError('partially null retained-owner vector')
    elif (first < 0x10000 or end < first or capacity < end
          or (end-first) % stride or (capacity-first) % stride
          or (end-first)//stride > maximum):
        raise ValueError('invalid retained-owner vector extent')
    return first, end, capacity


def resource_globals(m, registry):
    catalog = m.word(registry + 0x14)
    first, end, capacity = vector(m, catalog + 0x1250, 4, 16384)
    rows = []
    for pointer in m.words(first, (end-first)//4):
        guid, count, values = m.words(pointer, 3)
        if count >= 32000:
            raise ValueError('resource-global count outside native allocation domain')
        rows.append(dict(pointer=pointer, guid=guid, count=count,
                         values_pointer=values, values=m.words(values, count, 2)))
    return dict(begin=first, end=end, capacity=capacity, rows=rows)


def resource_binding(m, resource):
    definition = m.word(resource + 0x60)
    flags = m.word(resource + 0xb4)
    #43bceb..43bcff publishes the OBJD chunk ID in the high word; the
    #resource+14 label is not a source filename or chunk identity.
    return dict(resource=resource, definition=definition,
                guid=m.word(definition + 0x1c), flags=flags, chunk_id=flags >> 16,
                definition_storage=m.read(definition, 0xd8).hex())


def frame_context(m, context):
    # 43ba52/47d8d7/401de9: the language owner links back to the Resource.
    # Never infer this resource from the currently executing object's definition.
    private, semi, language = m.words(context + 8, 3)
    resource = m.word(language + 4)
    if m.word(resource + 0xc) != context:
        raise ValueError('frame private-context reverse identity differs')
    if m.word(resource) != private or m.word(resource + 4) != semi:
        raise ValueError('frame private/semiglobal binding differs')
    return dict(context=context, language=language, resource=resource,
                private_file=private, semiglobal_file=semi,
                binding=resource_binding(m, resource))


def grid(m, pointer, stride):
    width, height, directory = m.words(pointer + 4, 3)
    if not 4 <= width <= 64 or not 4 <= height <= 64:
        raise ValueError('retained-owner grid dimensions outside supported profile')
    columns = m.words(directory, width)
    raw = b''.join(m.read(column, height*stride) for column in columns)
    return dict(pointer=pointer, width=width, height=height,
                columns=columns, bytes=raw.hex())


def spatial_order(m, registry, by_pointer):
    """Read440349's actual registry cache; do not rebuild or sort it.

    Dirty and rotation belong to this owner, independently of the live camera.
    An unresolved cached pointer is missing identity evidence, not an empty
    entry or permission to substitute the current registry enumeration.
    """
    dirty = m.byte(registry + 0x44)
    rotation = m.word(registry + 0x48)
    if dirty not in (0, 1) or rotation > 3:
        raise ValueError('invalid retained spatial-order dirty/rotation')
    first, end, capacity = vector(m, registry + 0x4c, 4, 32767)
    pointers = m.words(first, (end-first)//4)
    ids = []
    for pointer in pointers:
        if pointer not in by_pointer:
            raise ValueError('unknown retained spatial-order object identity')
        ids.append(by_pointer[pointer])
    if len(set(ids)) != len(ids):
        raise ValueError('duplicate retained spatial-order object identity')
    return dict(dirty=dirty, rotation=rotation, begin=first, end=end,
                capacity=capacity, pointers=pointers, ids=ids)


def capture(m, state, sim, registry):
    by_pointer = {o['pointer']: o['id'] for o in state['objects']}
    if len(by_pointer) != len(state['objects']):
        raise ValueError('duplicate retained object pointers')
    object_rows = []
    contexts = {}
    for o in state['objects']:
        pointer = o['pointer']
        first, end, capacity = vector(m, pointer + 0x130, 36, 1820)
        slots = m.read(first, end-first) if end != first else b''
        row = dict(id=o['id'], pointer=pointer,
                   tree_error=m.half(pointer + 0x28),
                   native_flags=m.word(pointer + 0x114),
                   keep_dynamic=m.byte(pointer + 0x188),
                   bounds=list(struct.unpack('<4i', m.read(pointer + 0x100, 16))),
                   physical_slots=dict(begin=first, end=end, capacity=capacity,
                       bytes=slots.hex(), ids=[struct.unpack_from('<h', slots, i+16)[0]
                                              for i in range(0, len(slots), 36)]))
        row['binding'] = resource_binding(m, m.word(pointer + 0x11c))
        if row['binding']['definition'] != m.word(pointer + 0x118):
            raise ValueError('current object/resource definition binding differs')
        if o.get('person') is not None:
            begin, finish, limit = vector(m, pointer + 0xad0, 12, 5461)
            row['motive_continuation'] = dict(last_decay=m.word(pointer + 0x810),
                changes=dict(begin=begin, end=finish, capacity=limit,
                    entries=[m.words(p, 3) for p in range(begin, finish, 12)]))
        object_rows.append(row)
        for frame in o['frames']:
            context = frame['code_owner_pointer']
            if context not in contexts:
                contexts[context] = frame_context(m, context)

    # Full allocated arrays, not an active-size crop or a reconstructed chain.
    world = m.word(0x6e552c)
    physical = dict(world=world, active_size=m.word(world + 0x10), floors=[])
    for level in (1, 2):
        physical['floors'].append(dict(level=level,
            heads=grid(m, m.word(world + 0x38 + level*4), 2),
            flags=grid(m, m.word(world + 0x30 + level*4), 1)))
    physical['spatial_dirty'] = m.byte(registry + 0x44)

    first, end, capacity = vector(m, registry + 0x18, 4, 32767)
    pointers = m.words(first, (end-first)//4)
    flag_pointer = m.word(registry + 0x24)
    words = m.words(flag_pointer, len(pointers))
    ids = []
    for i, pointer in enumerate(pointers):
        if pointer == 0:
            ids.append(0)
        elif by_pointer.get(pointer) != i+1:
            raise ValueError('retained registry slot/object identity differs')
        else:
            ids.append(i+1)
    order, seen = [], set()
    pointer = m.word(registry + 0x30)
    while pointer:
        if pointer in seen or pointer not in by_pointer:
            raise ValueError('cyclic or unknown retained registry enumeration')
        seen.add(pointer)
        order.append(by_pointer[pointer])
        pointer = m.word(pointer + 0xe0)
    if seen != set(by_pointer):
        raise ValueError('retained registry enumeration membership differs')
    begin, finish, limit = vector(m, registry + 0x58, 2, 32768)
    pending = m.words(begin, (finish-begin)//2, 2)
    return dict(schema=SCHEMA, fields=sorted(FIELDS),
        frame_private_contexts=list(contexts.values()),
        frame_binding_scope='fresh reverse identity; source/provider normalization not yet proven',
        resource_globals=resource_globals(m, registry), objects=object_rows,
        physical=physical,
        registry=dict(begin=first, end=end, capacity=capacity,
            pointers=pointers, ids=ids, words=words, object_order=order,
            spatial_order=spatial_order(m, registry, by_pointer),
            pending_removals=dict(begin=begin, end=finish, capacity=limit, ids=pending)))
