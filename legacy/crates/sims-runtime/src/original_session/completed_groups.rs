//! Current multi-tile owners from one captured epoch; no constructors or
//! lifecycle/history inference and no unrelated World defaults.
use super::completed_animations::bytes;
use anyhow::{Result, ensure};
use serde::Deserialize;
use sims_vm::{
    ObjectId,
    multitile::{self, Part},
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub object: ObjectId,
    pub native_pointer: u32,
    pub vtable: u32,
    pub private_provider: u32,
    pub definition_raw: String,
    pub base_raw: String,
    pub part_raw: String,
    pub part: Part,
}
fn word(raw: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(raw[off..off + 4].try_into().unwrap())
}

/// Inputs include every live native object identity, independently of the part
/// subset. This checks real origin/rotation/traversal semantics on the retained
/// graph; it does not instantiate a World or decide historical Init ownership.
pub fn import(
    rows: &[Input],
    objects: &BTreeMap<ObjectId, u32>,
) -> Result<BTreeMap<ObjectId, Part>> {
    ensure!(
        objects.len() <= 32767 && rows.len() <= objects.len(),
        "retained group identity bound exceeded"
    );
    let mut by_pointer = BTreeMap::new();
    for (&id, &ptr) in objects {
        ensure!(
            id > 0 && ptr != 0 && by_pointer.insert(ptr, id).is_none(),
            "invalid/aliased native object identity"
        );
    }
    let ident = |p: u32| -> Result<Option<ObjectId>> {
        if p == 0 {
            Ok(None)
        } else {
            Ok(Some(*by_pointer.get(&p).ok_or_else(|| {
                anyhow::anyhow!("group link has no live native owner")
            })?))
        }
    };
    let mut parts = BTreeMap::new();
    let mut groups = BTreeMap::new();
    let mut directions = BTreeMap::new();
    for row in rows {
        let base = bytes(&row.base_raw)?;
        let def = bytes(&row.definition_raw)?;
        let raw = bytes(&row.part_raw)?;
        ensure!(
            base.len() == 0x188
                && def.len() == 0xd8
                && raw.len() == 0x20
                && objects.get(&row.object) == Some(&row.native_pointer)
                && word(&base, 0) == row.vtable
                && row.private_provider != 0,
            "retained group raw owner differs"
        );
        let words: Vec<i16> = def
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes(b.try_into().unwrap()))
            .collect();
        ensure!(
            (row.vtable == 0x666c20 && words[9] == 4 || row.vtable == 0x667480 && words[9] == 8)
                && words[10] != 0,
            "unknown retained group class/definition"
        );
        let part = Part {
            next: ident(word(&raw, 0))?,
            head: ident(word(&raw, 4))?,
            origin: std::array::from_fn(|i| word(&raw, 8 + 4 * i) as i32),
            offset: std::array::from_fn(|i| word(&raw, 20 + 4 * i) as i32),
        };
        ensure!(part == row.part, "native group normalization differs");
        ensure!(
            part.origin == multitile::origin(&words).map_err(anyhow::Error::msg)?,
            "native group origin differs from original definition semantics"
        );
        let direction = i16::from_le_bytes(base[0x4c..0x4e].try_into().unwrap());
        ensure!(
            parts.insert(row.object, part).is_none(),
            "duplicate retained group owner"
        );
        groups.insert(row.object, (row.private_provider, words[10]));
        directions.insert(row.object, direction);
    }
    let mut covered = BTreeSet::new();
    for &id in parts.keys() {
        let chain = multitile::retained_chain(&parts, id).map_err(anyhow::Error::msg)?;
        ensure!(
            chain.iter().all(|member| groups[member] == groups[&id]),
            "native group chain crosses provider/group ownership"
        );
        // Same 42275f preparation used by update_offsets: subtract the primary
        // origin first, then rotate with the primary's direction. Keep the
        // captured cache; this comparison must never recompute it in place.
        let head = chain[0];
        let base = parts[&head].origin;
        let relative = std::array::from_fn(|i| parts[&id].origin[i].wrapping_sub(base[i]));
        ensure!(
            parts[&id].offset == multitile::rotate(relative, directions[&head]),
            "native group cached offset differs from primary-relative rotation profile"
        );
        covered.extend(chain);
    }
    ensure!(
        covered.len() == parts.len(),
        "retained group coverage differs"
    );
    Ok(parts)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn hex(b: &[u8]) -> String {
        b.iter().map(|v| format!("{v:02x}")).collect()
    }
    fn put(b: &mut [u8], off: usize, v: u32) {
        b[off..off + 4].copy_from_slice(&v.to_le_bytes())
    }
    fn row(id: i16, ptr: u32, part: Part) -> Input {
        let mut base = vec![0; 0x188];
        put(&mut base, 0, 0x666c20);
        base[0x4c..0x4e].copy_from_slice(&6i16.to_le_bytes());
        let mut def = vec![0i16; 108];
        def[9] = 4;
        def[10] = 1;
        def[11] = ((part.origin[0] as i16) << 8) | (part.origin[1] as i16 & 255);
        def[56] = part.origin[2] as i16;
        let mut raw = vec![0; 32];
        put(&mut raw, 0, part.next.map_or(0, |i| i as u32 * 0x1000));
        put(&mut raw, 4, part.head.map_or(0, |i| i as u32 * 0x1000));
        for i in 0..3 {
            put(&mut raw, 8 + 4 * i, part.origin[i] as u32);
            put(&mut raw, 20 + 4 * i, part.offset[i] as u32)
        }
        Input {
            object: id,
            native_pointer: ptr,
            vtable: 0x666c20,
            private_provider: 0x3000,
            definition_raw: hex(&def
                .into_iter()
                .flat_map(i16::to_le_bytes)
                .collect::<Vec<_>>()),
            base_raw: hex(&base),
            part_raw: hex(&raw),
            part,
        }
    }
    fn fixture() -> (Vec<Input>, BTreeMap<ObjectId, u32>) {
        (
            vec![
                row(
                    1,
                    0x1000,
                    Part {
                        head: None,
                        next: Some(2),
                        origin: [1, 1, 0],
                        offset: [0, 0, 0],
                    },
                ),
                row(
                    2,
                    0x2000,
                    Part {
                        head: Some(1),
                        next: None,
                        origin: [0, 1, 0],
                        offset: [0, 1, 0],
                    },
                ),
            ],
            BTreeMap::from([(1, 0x1000), (2, 0x2000)]),
        )
    }
    #[test]
    fn primary_relative_cache_and_source_order_import_without_recomputing() {
        let (rows, objects) = fixture();
        let before = rows[1].part_raw.clone();
        let parts = import(&rows, &objects).unwrap();
        assert_eq!(parts[&2].offset, [0, 1, 0]);
        assert_eq!(multitile::retained_chain(&parts, 2).unwrap(), vec![1, 2]);
        assert_eq!(rows[1].part_raw, before);
        let b = postcard::to_allocvec(&parts).unwrap();
        let restored: BTreeMap<ObjectId, Part> = postcard::from_bytes(&b).unwrap();
        assert_eq!(restored, parts);
    }
    #[test]
    fn cyclic_dangling_cross_provider_and_inconsistent_cache_reject() {
        for kind in 0..5 {
            let (mut rows, objects) = fixture();
            match kind {
                0 => {
                    rows[1].part.offset = [0, 2, 0];
                    let mut raw = bytes(&rows[1].part_raw).unwrap();
                    put(&mut raw, 24, 2);
                    rows[1].part_raw = hex(&raw)
                }
                1 => {
                    rows[0].part.next = Some(1);
                    let mut raw = bytes(&rows[0].part_raw).unwrap();
                    put(&mut raw, 0, 0x1000);
                    rows[0].part_raw = hex(&raw)
                }
                2 => {
                    let mut raw = bytes(&rows[1].part_raw).unwrap();
                    put(&mut raw, 0, 0xdead);
                    rows[1].part_raw = hex(&raw)
                }
                3 => rows[1].private_provider = 0x4000,
                _ => rows[1].definition_raw = "00".into(),
            }
            assert!(import(&rows, &objects).is_err(), "mutation {kind}");
        }
    }
}
