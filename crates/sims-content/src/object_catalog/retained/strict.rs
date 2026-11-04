//! Strict captured input DTOs, leaving existing managed-save readers unchanged.
use serde::{Deserialize, Deserializer};
use sims_sim::object_catalog::{Key, definition, iff};
use std::collections::BTreeMap;

fn required_option<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    de: D,
) -> Result<Option<T>, D::Error> {
    Option::deserialize(de)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DefinitionKey {
    resource: u32,
    chunk: u16,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Definition {
    key: DefinitionKey,
    #[serde(deserialize_with = "required_option")]
    semiglobal: Option<u32>,
    sprite_resource: u32,
    #[serde(deserialize_with = "required_option")]
    global: Option<u32>,
    name: Vec<u8>,
    data: Vec<u8>,
    index: u32,
    flags: u32,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Attributes {
    guid: u32,
    values: Vec<i16>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Catalog {
    entries: Vec<Definition>,
    guids: BTreeMap<u32, usize>,
    attributes: Vec<Attributes>,
}

pub(super) fn catalog<'de, D: Deserializer<'de>>(de: D) -> Result<definition::Catalog, D::Error> {
    let state = Catalog::deserialize(de)?;
    Ok(definition::Catalog {
        entries: state
            .entries
            .into_iter()
            .map(|e| definition::Entry {
                key: Key {
                    resource: e.key.resource,
                    chunk: e.key.chunk,
                },
                semiglobal: e.semiglobal,
                sprite_resource: e.sprite_resource,
                global: e.global,
                name: e.name,
                data: e.data,
                index: e.index,
                flags: e.flags,
            })
            .collect(),
        guids: state.guids,
        attributes: state
            .attributes
            .into_iter()
            .map(|a| definition::Attributes {
                guid: a.guid,
                values: a.values,
            })
            .collect(),
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    handle: u32,
    bytes: Vec<u8>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    tag: u32,
    id: u16,
    flags: u16,
    disk_flags: u16,
    ordinal: u16,
    offset: u32,
    name: Vec<u8>,
    #[serde(deserialize_with = "required_option")]
    payload: Option<Payload>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Iff {
    valid: bool,
    error: i32,
    map_offset: u32,
    unused: u32,
    write_cursor: u32,
    modified: bool,
    table: iff::RetainedTable<iff::RetainedTable<usize>>,
    storage: Vec<Record>,
    next_handle: u32,
}

pub(crate) fn iff<'de, D: Deserializer<'de>>(de: D) -> Result<iff::Iff, D::Error> {
    let state = Iff::deserialize(de)?;
    iff::Iff::from_retained_parts(iff::RetainedParts {
        valid: state.valid,
        error: state.error,
        map_offset: state.map_offset,
        unused: state.unused,
        write_cursor: state.write_cursor,
        modified: state.modified,
        table: state.table,
        storage: state
            .storage
            .into_iter()
            .map(|r| iff::Record {
                tag: r.tag,
                id: r.id,
                flags: r.flags,
                disk_flags: r.disk_flags,
                ordinal: r.ordinal,
                offset: r.offset,
                name: r.name,
                payload: r.payload.map(|p| iff::Payload {
                    handle: p.handle,
                    bytes: p.bytes,
                }),
            })
            .collect(),
        handle_epoch: iff::RetainedHandleEpoch::NewUnpublished {
            next: state.next_handle,
        },
    })
    .map_err(serde::de::Error::custom)
}
