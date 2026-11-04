//! Borrow current 0x44-byte native action records and one-based use records.
//! No dispatcher, stack constructor, RNG, callback or action serial is run.
//! Raw inactive records/ring words remain separate from semantic null actions.
//! This component does not admit Thread status/continuations or a whole Session.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_fixtures::readset::Readset;
use sims_vm::{
    ObjectId,
    action::{Action, Actor, Queue},
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Clone, Copy)]
pub struct Identity {
    pub object: ObjectId,
    pub pointer: u32,
    pub person: bool,
    pub frame_depth: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Use {
    pub target: ObjectId,
    pub depth: usize,
    pub icon: bool,
}
pub struct Native {
    pub first: u32,
    pub end: u32,
    pub active: Vec<u8>,
    pub previous: Vec<u8>,
    pub pending: Vec<Vec<u8>>,
    pub uses: Vec<u8>,
}
pub struct Imported {
    pub actors: BTreeMap<ObjectId, Actor>,
    pub uses: BTreeMap<ObjectId, Vec<Use>>,
    pub native: BTreeMap<ObjectId, Native>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    pub caller: ObjectId,
    pub callee: ObjectId,
    pub action_target: Option<ObjectId>,
    pub action_icon: bool,
}
fn word(raw: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(raw[off..off + 4].try_into().unwrap())
}
fn address(pointer: u32, offset: u32) -> Result<u32> {
    pointer
        .checked_add(offset)
        .context("native action owner address overflow")
}
fn object(pointer: u32, identities: &BTreeMap<u32, ObjectId>) -> Result<ObjectId> {
    if pointer == 0 {
        Ok(0)
    } else {
        identities
            .get(&pointer)
            .copied()
            .context("native action references an absent object")
    }
}
fn record(
    raw: &[u8],
    actor: ObjectId,
    identities: &BTreeMap<u32, ObjectId>,
    m: &Readset<'_>,
) -> Result<(Option<Action>, ObjectId)> {
    ensure!(raw.len() == 0x44, "native action record extent differs");
    let caller = object(word(raw, 4), identities)?;
    let target = object(word(raw, 8), identities)?;
    let icon_owner = object(word(raw, 12), identities)?;
    ensure!(
        caller == actor || (target == 0 && caller == 0),
        "native action requires a foreign-caller profile"
    );
    let (begin, end, capacity) = (word(raw, 40), word(raw, 44), word(raw, 48));
    ensure!(
        (begin == 0 && end == 0 && capacity == 0)
            || (begin >= 0x10000 && begin <= end && end < capacity),
        "native action label extent differs"
    );
    ensure!(
        end - begin <= 4096,
        "native action label exceeds bounded profile"
    );
    let label = if begin == 0 {
        None
    } else {
        let text = m.read(begin, (end - begin) as usize + 1)?;
        ensure!(
            text.last() == Some(&0)
                && !text[..text.len() - 1].contains(&0)
                && text[..text.len() - 1].is_ascii(),
            "native action label needs a terminated/code-page-qualified owner"
        );
        if text.len() == 1 {
            None
        } else {
            Some(String::from_utf8(text[..text.len() - 1].to_vec())?)
        }
    };
    let args = std::array::from_fn(|i| {
        i16::from_le_bytes(raw[20 + i * 2..22 + i * 2].try_into().unwrap())
    });
    Ok((
        if target == 0 {
            None
        } else {
            Some(Action {
                id: word(raw, 60),
                target,
                icon_owner,
                menu_id: word(raw, 16),
                priority: word(raw, 28) as i32,
                flags: word(raw, 64),
                args,
                routine: u16::from_le_bytes(raw[32..34].try_into().unwrap()),
                attenuation_bits: word(raw, 36),
                label,
            })
        },
        icon_owner,
    ))
}

pub fn import(objects: &[Identity], memory: &Readset<'_>) -> Result<Imported> {
    ensure!(
        objects.len() <= 32767,
        "native action object inventory exceeds bound"
    );
    let mut identities = BTreeMap::new();
    let mut ids = BTreeSet::new();
    for o in objects {
        ensure!(
            o.object > 0
                && o.pointer >= 0x10000
                && o.frame_depth <= sims_vm::MAX_STACK
                && identities.insert(o.pointer, o.object).is_none()
                && ids.insert(o.object),
            "invalid/aliased native action object inventory"
        );
    }
    let mut result = Imported {
        actors: BTreeMap::new(),
        uses: BTreeMap::new(),
        native: BTreeMap::new(),
    };
    for o in objects.iter().filter(|o| o.person) {
        let ring = memory.read(address(o.pointer, 0xa40)?, 8)?;
        let (first, end) = (word(&ring, 0), word(&ring, 4));
        let count = end.wrapping_sub(first);
        ensure!(count <= 8, "native action ring exceeds eight slots");
        let mut pending = VecDeque::new();
        let mut pending_raw = Vec::new();
        for i in 0..count {
            let raw = memory.read(
                address(o.pointer, 0x820 + (first.wrapping_add(i) & 7) * 0x44)?,
                0x44,
            )?;
            pending.push_back(
                record(&raw, o.object, &identities, memory)?
                    .0
                    .context("native queued action has null target")?,
            );
            pending_raw.push(raw);
        }
        let active_raw = memory.read(address(o.pointer, 0xa48)?, 0x44)?;
        let previous_raw = memory.read(address(o.pointer, 0xa8c)?, 0x44)?;
        let (active, icon) = record(&active_raw, o.object, &identities, memory)?;
        let (previous, _) = record(&previous_raw, o.object, &identities, memory)?;
        let header = memory.read(address(o.pointer, 0xb00)?, 12)?;
        let (begin, end_use, capacity) = (word(&header, 0), word(&header, 4), word(&header, 8));
        ensure!(
            begin <= end_use
                && end_use <= capacity
                && (begin != 0 || capacity == 0)
                && (end_use - begin) % 12 == 0
                && (capacity - begin) % 12 == 0
                && (end_use - begin) / 12 <= 1024,
            "native action-use vector extent differs"
        );
        let raw = memory.read(begin, (end_use - begin) as usize)?;
        let mut uses = Vec::new();
        let mut previous_depth = 1;
        for row in raw.chunks_exact(12) {
            let target = object(word(row, 0), &identities)?;
            let depth = word(row, 4) as usize;
            ensure!(
                target != 0 && previous_depth < depth && depth <= o.frame_depth && row[8] <= 1,
                "native action-use depth/target/icon differs"
            );
            uses.push(Use {
                target,
                depth,
                icon: row[8] != 0,
            });
            previous_depth = depth;
        }
        result.actors.insert(
            o.object,
            Actor {
                queue: Queue { pending },
                empty_action_icon: if active.is_none() { icon } else { 0 },
                active,
                previous,
            },
        );
        result.uses.insert(o.object, uses);
        result.native.insert(
            o.object,
            Native {
                first,
                end,
                active: active_raw,
                previous: previous_raw,
                pending: pending_raw,
                uses: raw,
            },
        );
    }
    Ok(result)
}

/// 4592f5's one-based use-depth ownership. This establishes invocation fields,
/// not the primitive continuation/status that must accompany an active route.
/// A caller still verifies each native frame's actual code-owner context.
pub fn invocations(actor: ObjectId, depth: usize, uses: &[Use]) -> Result<Vec<Invocation>> {
    ensure!(
        actor > 0 && depth <= sims_vm::MAX_STACK,
        "invalid native invocation actor/depth"
    );
    let mut previous = 1;
    for u in uses {
        ensure!(
            u.target > 0 && previous < u.depth && u.depth <= depth,
            "invalid invocation use depth/order"
        );
        previous = u.depth;
    }
    let mut callee = actor;
    let mut current = uses.iter().peekable();
    Ok((1..=depth)
        .map(|d| {
            let action = if current.peek().is_some_and(|u| u.depth == d) {
                current.next()
            } else {
                None
            };
            if let Some(u) = action {
                callee = u.target;
            }
            Invocation {
                caller: actor,
                callee,
                action_target: action.map(|u| u.target),
                action_icon: action.is_some_and(|u| u.icon),
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nested_use_scopes_release_at_their_own_frame_without_replacing_caller() -> Result<()> {
        let uses = [
            Use {
                target: 132,
                depth: 5,
                icon: false,
            },
            Use {
                target: 87,
                depth: 8,
                icon: true,
            },
        ];
        let frames = invocations(16, 9, &uses)?;
        assert_eq!(
            frames.iter().map(|f| f.callee).collect::<Vec<_>>(),
            vec![16, 16, 16, 16, 132, 132, 132, 87, 87]
        );
        assert_eq!(
            frames.iter().map(|f| f.action_target).collect::<Vec<_>>(),
            vec![
                None,
                None,
                None,
                None,
                Some(132),
                None,
                None,
                Some(87),
                None
            ]
        );
        assert!(frames.iter().all(|f| f.caller == 16));
        for invalid in [
            vec![Use {
                target: 87,
                depth: 1,
                icon: false,
            }],
            vec![uses[1].clone(), uses[0].clone()],
            vec![Use {
                target: 87,
                depth: 10,
                icon: false,
            }],
        ] {
            assert!(invocations(16, 9, &invalid).is_err());
        }
        Ok(())
    }
    #[test]
    fn ring_wrap_preserves_order_signed_fields_and_inactive_native_bytes() -> Result<()> {
        let (actor, target) = (0x100000u32, 0x200000u32);
        let mut rows: Vec<(u32, Vec<u8>)> = Vec::new();
        let mut ring = (u32::MAX - 1).to_le_bytes().to_vec();
        ring.extend(2u32.to_le_bytes());
        rows.push((actor + 0xa40, ring));
        rows.push((actor + 0xa48, vec![0; 0x44]));
        let mut previous = vec![0; 0x44];
        previous[64..68].copy_from_slice(&32u32.to_le_bytes());
        rows.push((actor + 0xa8c, previous.clone()));
        rows.push((actor + 0xb00, vec![0; 12]));
        for (ordinal, slot) in [6u32, 7, 0, 1].into_iter().enumerate() {
            let mut raw = vec![0; 0x44];
            for (off, value) in [
                (4, actor),
                (8, target),
                (12, target),
                (16, 3),
                (28, u32::MAX),
                (36, 0x3f800000),
                (60, 100 + ordinal as u32),
            ] {
                raw[off..off + 4].copy_from_slice(&value.to_le_bytes());
            }
            raw[20..22].copy_from_slice(&(-3i16).to_le_bytes());
            raw[32..34].copy_from_slice(&4100u16.to_le_bytes());
            rows.push((actor + 0x820 + slot * 0x44, raw));
        }
        let mut source = b"SIMS-MEM-1\0".to_vec();
        source.extend((rows.len() as u32).to_le_bytes());
        for (address, raw) in rows {
            source.extend(address.to_le_bytes());
            source.extend((raw.len() as u32).to_le_bytes());
            source.extend(raw);
        }
        let memory = Readset::parse(&source)?;
        let identities = [
            Identity {
                object: 1,
                pointer: actor,
                person: true,
                frame_depth: 2,
            },
            Identity {
                object: 2,
                pointer: target,
                person: false,
                frame_depth: 0,
            },
        ];
        let state = import(&identities, &memory)?;
        let pending = &state.actors[&1].queue.pending;
        assert_eq!(
            pending.iter().map(|a| a.id).collect::<Vec<_>>(),
            vec![100, 101, 102, 103]
        );
        assert!(
            pending
                .iter()
                .all(|a| a.priority == -1 && a.args[0] == -3 && a.attenuation_bits == 0x3f800000)
        );
        assert_eq!(state.native[&1].first, u32::MAX - 1);
        assert_eq!(state.native[&1].end, 2);
        assert_eq!(state.native[&1].previous, previous);
        assert!(state.actors[&1].previous.is_none());
        assert!(import(&[identities[0], identities[0]], &memory).is_err());
        Ok(())
    }
}
