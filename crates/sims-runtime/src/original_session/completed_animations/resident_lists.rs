//! Already constructed native lists, not BCF construction inputs. Accept only
//! the idempotent transport subset: unique lowercase ASCII keys/values and
//! strictly increasing signed event times. Never repair, fold, sort or dedupe.
use super::{AssetInput, CharacterInput, bytes, word};
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::Value;
use sims_fixtures::readset::Readset;
use sims_formats::animation::{Motion, TimedProperty};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Vector {
    begin: u32,
    end: u32,
    capacity: u32,
    stride: u32,
    used_raw: String,
    inactive_capacity_observed: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Properties {
    pointer: u32,
    raw: String,
    vector: Vector,
    pairs_raw: Vec<[String; 2]>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Events {
    pointer: u32,
    raw: String,
    vector: Vector,
    property_owners: Vec<u32>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Lists {
    properties: Vec<Properties>,
    events: Vec<Events>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    pointer: u32,
    raw: String,
    owner: u32,
    index_bits: u32,
}
type Pairs = Vec<(String, String)>;

fn header(pointer: u32, hex: &str, count: usize, m: &Readset<'_>) -> Result<Vec<u8>> {
    let raw = bytes(hex)?;
    ensure!(
        pointer != 0 && raw.len() == count && m.read(pointer, count)? == raw,
        "resident animation owner header differs from native bytes"
    );
    Ok(raw)
}
fn vector(
    h: &[u8],
    off: usize,
    stride: usize,
    maximum: usize,
    raw: &[u8],
    m: &Readset<'_>,
) -> Result<()> {
    let begin = word(h, off);
    let end = word(h, off + 4);
    let capacity = word(h, off + 8);
    ensure!(
        begin <= end
            && end <= capacity
            && (begin != 0 || capacity == 0)
            && raw.len() % stride == 0
            && raw.len() / stride <= maximum
            && (end - begin) as usize == raw.len()
            && (capacity - begin) as usize % stride == 0,
        "resident animation vector extent differs"
    );
    ensure!(
        m.read(begin, raw.len())? == raw,
        "resident animation vector differs from native bytes"
    );
    Ok(())
}
fn words(values: impl IntoIterator<Item = u32>) -> Vec<u8> {
    values.into_iter().flat_map(u32::to_le_bytes).collect()
}

/// Bind semantic metadata back to the real motion/layer/header owners. A valid
/// list alone does not establish which native motion/binding references it.
pub(super) fn backing(
    assets: &[AssetInput],
    characters: &[CharacterInput],
    m: &Readset<'_>,
) -> Result<()> {
    ensure!(
        assets.len() <= 256 && characters.len() <= 256,
        "resident backing owner bound exceeded"
    );
    for a in assets {
        let h = header(a.pointer, &a.raw, 0x74, m)?;
        for (off, raw) in [(4, &a.name_raw), (0x10, &a.skill_raw)] {
            vector(&h, off, 1, 256, &bytes(raw)?, m)?;
        }
        vector(&h, 0x20, 4, 1024, &bytes(&a.motion_vector_raw)?, m)?;
        for motion in &a.motions {
            let h = header(motion.pointer, &motion.raw, 0x34, m)?;
            vector(&h, 0, 1, 256, &bytes(&motion.bone_raw)?, m)?;
        }
        for (raw, pointer, count, stride) in [
            (&a.translation_samples_raw, 0x60, 0x5c, 12),
            (&a.rotation_samples_raw, 0x68, 0x64, 16),
        ] {
            let raw = bytes(raw)?;
            ensure!(
                word(&h, count) <= 65536
                    && raw.len() == word(&h, count) as usize * stride
                    && m.read(word(&h, pointer), raw.len())? == raw,
                "resident sample bank differs from native buffer"
            );
        }
    }
    for c in characters {
        let ar = header(c.animator_pointer, &c.animator_raw, 0x2a0, m)?;
        let sr = header(c.skin_pointer, &c.skin_raw, 0xd4, m)?;
        let person_owner = c
            .person_pointer
            .checked_add(0xadc)
            .context("person/animator owner overflow")?;
        ensure!(
            word(&m.read(person_owner, 4)?, 0) == c.animator_pointer
                && word(&m.read(word(&ar, 8), 8)?, 4) == c.skin_pointer,
            "resident person/model/skin ownership differs"
        );
        vector(&ar, 0x2c, 4, 4096, &bytes(&c.numbered_events_raw)?, m)?;
        vector(
            &sr,
            0x64,
            4,
            512,
            &words(c.bone_pointers.iter().copied()),
            m,
        )?;
        vector(&sr, 0x70, 4, 4096, &words(c.bone_lookup.iter().copied()), m)?;
        vector(
            &sr,
            0x7c,
            4,
            256,
            &words(c.native_layers.iter().map(|l| l.pointer)),
            m,
        )?;
        for l in &c.native_layers {
            let h = header(l.pointer, &l.raw, 0x80, m)?;
            vector(&h, 8, 12, 1024, &bytes(&l.bindings_raw)?, m)?;
        }
    }
    Ok(())
}

pub(super) struct Owners {
    properties: BTreeMap<u32, Pairs>,
    events: BTreeMap<u32, Vec<(i32, u32)>>,
    cursors: BTreeMap<u32, Cursor>,
    used_properties: BTreeSet<u32>,
    used_events: BTreeSet<u32>,
    used_cursors: BTreeSet<u32>,
}

fn rows(pointer: u32, raw: &str, v: &Vector, maximum: usize, m: &Readset<'_>) -> Result<Vec<u8>> {
    let h = bytes(raw)?;
    let data = bytes(&v.used_raw)?;
    ensure!(
        pointer != 0 && h.len() == 16 && word(&h, 0) > 0 && word(&h, 0) <= i32::MAX as u32,
        "invalid resident list header/reference count"
    );
    ensure!(
        v.stride == 8
            && !v.inactive_capacity_observed
            && v.begin == word(&h, 4)
            && v.end == word(&h, 8)
            && v.capacity == word(&h, 12)
            && v.begin <= v.end
            && v.end <= v.capacity
            && (v.begin != 0 || v.capacity == 0)
            && (v.end - v.begin) as usize == data.len()
            && data.len() % 8 == 0
            && data.len() / 8 <= maximum
            && (v.capacity - v.begin) % 8 == 0,
        "resident list vector metadata differs"
    );
    ensure!(
        m.read(pointer, 16)? == h && m.read(v.begin, data.len())? == data,
        "resident list differs from observed native memory"
    );
    Ok(data)
}
fn text(pointer: u32, hex: &str, m: &Readset<'_>) -> Result<String> {
    let raw = bytes(hex)?;
    ensure!(
        pointer != 0
            && raw.len() < 256
            && raw.is_ascii()
            && !raw.contains(&0)
            && !raw.iter().any(u8::is_ascii_uppercase),
        "resident property text is not an idempotent bounded transport"
    );
    let native = m.read(pointer, raw.len() + 1)?;
    ensure!(
        native.last() == Some(&0) && native[..raw.len()] == raw,
        "resident property text differs from native terminated bytes"
    );
    Ok(String::from_utf8(raw)?)
}

impl Owners {
    pub(super) fn read(lists: &Value, cursors: &Value, memory: &Readset<'_>) -> Result<Self> {
        // Actual original constructors; arbitrary/self-consistent metadata does
        // not substitute for the observed code in this component profile.
        for (address, count, hash) in [
            (
                0x59431a,
                118,
                "50ffffe95ca9b9605bcb186831a5dadbbcf4803030c835d679f5017eb37d7561",
            ),
            (
                0x5946fc,
                124,
                "3b494a34d6788f4d282c08dcbabb2d233da64ef28dfe979e33ce024f94fc4d58",
            ),
            (
                0x594778,
                27,
                "9ad955ad37e6be7253c659c8dbdf71a9f7b71d5e98dfe889ae1a614e6605931c",
            ),
        ] {
            ensure!(
                sims_fixtures::digest(&memory.read(address, count)?) == hash,
                "resident list constructor witness differs"
            );
        }
        let lists: Lists = serde_json::from_value(lists.clone())?;
        let cursors: Vec<Cursor> = serde_json::from_value(cursors.clone())?;
        ensure!(
            lists.properties.len() <= 4096 && lists.events.len() <= 4096 && cursors.len() <= 65536,
            "resident list/cursor owner bound exceeded"
        );
        let mut result = Self {
            properties: BTreeMap::new(),
            events: BTreeMap::new(),
            cursors: BTreeMap::new(),
            used_properties: BTreeSet::new(),
            used_events: BTreeSet::new(),
            used_cursors: BTreeSet::new(),
        };
        for p in lists.properties {
            let raw = rows(p.pointer, &p.raw, &p.vector, 4096, memory)?;
            ensure!(
                !result.properties.contains_key(&p.pointer) && p.pairs_raw.len() == raw.len() / 8,
                "duplicate/incorrect resident property owner"
            );
            let mut pairs = Vec::with_capacity(p.pairs_raw.len());
            let mut names = BTreeSet::new();
            for (i, [key, value]) in p.pairs_raw.iter().enumerate() {
                let key = text(word(&raw, i * 8), key, memory)?;
                ensure!(
                    names.insert(key.clone()),
                    "resident property transport would collapse duplicate keys"
                );
                pairs.push((key, text(word(&raw, i * 8 + 4), value, memory)?));
            }
            result.properties.insert(p.pointer, pairs);
        }
        for e in lists.events {
            let raw = rows(e.pointer, &e.raw, &e.vector, 16384, memory)?;
            ensure!(
                !result.events.contains_key(&e.pointer)
                    && !result.properties.contains_key(&e.pointer)
                    && e.property_owners.len() == raw.len() / 8,
                "duplicate/incorrect resident event owner"
            );
            let mut entries = Vec::with_capacity(e.property_owners.len());
            for (i, owner) in e.property_owners.into_iter().enumerate() {
                let time = word(&raw, i * 8) as i32;
                ensure!(
                    word(&raw, i * 8 + 4) == owner
                        && result.properties.contains_key(&owner)
                        && entries.last().is_none_or(|(previous, _)| *previous < time),
                    "resident event ordering/property owner differs"
                );
                entries.push((time, owner));
            }
            result.events.insert(e.pointer, entries);
        }
        for c in cursors {
            let raw = bytes(&c.raw)?;
            ensure!(
                c.pointer != 0
                    && raw.len() == 8
                    && word(&raw, 0) == c.owner
                    && word(&raw, 4) == c.index_bits
                    && result.events.contains_key(&c.owner)
                    && !result.cursors.contains_key(&c.pointer)
                    && !result.events.contains_key(&c.pointer)
                    && !result.properties.contains_key(&c.pointer),
                "resident event cursor owner/raw differs"
            );
            ensure!(
                memory.read(c.pointer, 8)? == raw,
                "resident cursor differs from observed native memory"
            );
            result.cursors.insert(c.pointer, c);
        }
        Ok(result)
    }
    pub(super) fn motion(&mut self, raw: &[u8], motion: &Motion) -> Result<()> {
        let property = word(raw, 0x2c);
        if property == 0 {
            ensure!(
                motion.properties.is_empty(),
                "unowned resident motion properties"
            );
        } else {
            let pairs = self
                .properties
                .get(&property)
                .context("missing resident motion property owner")?;
            ensure!(
                motion.properties.len() == 1 && &motion.properties[0] == pairs,
                "resident motion properties differ from native list"
            );
            self.used_properties.insert(property);
        }
        let event = word(raw, 0x30);
        if event == 0 {
            ensure!(
                motion.event_lists.is_empty(),
                "unowned resident motion events"
            );
        } else {
            let entries = self
                .events
                .get(&event)
                .context("missing resident motion event owner")?;
            ensure!(
                motion.event_lists.len() == 1 && motion.event_lists[0].len() == entries.len(),
                "resident motion event extent differs"
            );
            for (
                TimedProperty {
                    time_ms,
                    bone,
                    pairs,
                },
                (time, owner),
            ) in motion.event_lists[0].iter().zip(entries)
            {
                ensure!(
                    time_ms == time
                        && bone == &motion.bone
                        && Some(pairs) == self.properties.get(owner),
                    "resident timed properties differ from native list"
                );
                self.used_properties.insert(*owner);
            }
            self.used_events.insert(event);
        }
        Ok(())
    }
    pub(super) fn cursor(&mut self, pointer: u32, owner: u32, retained: Option<i32>) -> Result<()> {
        if pointer == 0 {
            ensure!(retained.is_none(), "invented resident cursor");
        } else {
            let cursor = self
                .cursors
                .get(&pointer)
                .context("missing resident cursor storage")?;
            ensure!(
                owner != 0
                    && owner == cursor.owner
                    && retained == Some(cursor.index_bits as i32)
                    && self.used_events.contains(&owner)
                    && self.used_cursors.insert(pointer),
                "resident cursor alias/list/index differs"
            );
        }
        Ok(())
    }
    pub(super) fn finish(&self) -> Result<()> {
        ensure!(
            self.used_properties.len() == self.properties.len()
                && self.used_events.len() == self.events.len()
                && self.used_cursors.len() == self.cursors.len(),
            "unclaimed resident list/cursor owners"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::original_session::completed_animations::{self, AssetInput, CharacterInput};
    use std::{io::Read, path::PathBuf};

    fn hex(raw: &[u8]) -> String {
        raw.iter().map(|b| format!("{b:02x}")).collect()
    }
    fn recorded(rows: &[(u32, &[u8])]) -> Vec<u8> {
        let mut out = b"SIMS-MEM-1\0".to_vec();
        out.extend((rows.len() as u32).to_le_bytes());
        for (address, data) in rows {
            out.extend(address.to_le_bytes());
            out.extend((data.len() as u32).to_le_bytes());
            out.extend(*data);
        }
        out
    }
    #[test]
    fn native_text_requires_terminated_idempotent_bytes_and_actual_coverage() -> Result<()> {
        let source = recorded(&[
            (100, b"abc\0"),
            (200, b"ABC\0"),
            (300, b"abc"),
            (400, b"\x80\0"),
        ]);
        let memory = Readset::parse(&source)?;
        assert_eq!(text(100, &hex(b"abc"), &memory)?, "abc");
        for (pointer, value) in [
            (100, b"abd".as_slice()),
            (200, b"ABC"),
            (300, b"abc"),
            (400, b"\x80"),
            (0, b""),
        ] {
            assert!(text(pointer, &hex(value), &memory).is_err());
        }
        Ok(())
    }

    #[test]
    fn signed_live_cursor_is_preserved_and_storage_cannot_be_shared() -> Result<()> {
        // Authored ownership control only; not native admission.
        let mut owners = Owners {
            properties: BTreeMap::new(),
            events: BTreeMap::from([(100, vec![])]),
            cursors: BTreeMap::from([(
                200,
                Cursor {
                    pointer: 200,
                    raw: String::new(),
                    owner: 100,
                    index_bits: u32::MAX,
                },
            )]),
            used_properties: BTreeSet::new(),
            used_events: BTreeSet::from([100]),
            used_cursors: BTreeSet::new(),
        };
        assert!(owners.cursor(200, 100, Some(0)).is_err());
        assert!(owners.cursor(200, 101, Some(-1)).is_err());
        owners.cursor(200, 100, Some(-1))?;
        assert!(owners.cursor(200, 100, Some(-1)).is_err());
        owners.finish()?;
        assert!(owners.cursor(0, 0, Some(-1)).is_err());
        Ok(())
    }

    struct Cut {
        observation: Value,
        acknowledged: Vec<u8>,
    }
    fn base() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/private/goth-completed-continuation-20261030-third/results/native-sequence")
    }
    fn source() -> Result<Value> {
        let raw = std::fs::read(base().join("capture-result.json"))?;
        ensure!(
            sims_fixtures::digest(&raw)
                == "4139107d1965925bf3ee5f44fa524d5d4b1fae22bfdb617edfffb69d33bcd393",
            "private native sequence receipt changed"
        );
        Ok(serde_json::from_slice(&raw)?)
    }
    fn cut(index: usize, source: &Value) -> Result<Cut> {
        let directory = base().join(format!("completed-{index:02}"));
        let raw = std::fs::read(directory.join("observation.json.gz"))?;
        let receipt = &source["observations"][index];
        ensure!(
            raw.len() as u64 == receipt["bytes"].as_u64().context("compressed length")?
                && sims_fixtures::digest(&raw)
                    == receipt["sha256"].as_str().context("compressed hash")?,
            "compressed observation changed"
        );
        let mut decoder = flate2::bufread::GzDecoder::new(raw.as_slice());
        let mut decoded = Vec::new();
        decoder
            .by_ref()
            .take((64 << 20) + 1)
            .read_to_end(&mut decoded)?;
        ensure!(
            decoded.len() <= 64 << 20
                && decoder.into_inner().is_empty()
                && decoded.len() as u64
                    == receipt["decoded_bytes"]
                        .as_u64()
                        .context("decoded length")?
                && sims_fixtures::digest(&decoded)
                    == receipt["decoded_sha256"].as_str().context("decoded hash")?,
            "unbounded/trailing/altered decoded observation"
        );
        let observation: Value = serde_json::from_slice(&decoded)?;
        ensure!(
            observation["tick"] == 89073 + index && observation["natural_completed_index"] == index,
            "native cut identity changed"
        );
        let acknowledged = std::fs::read(directory.join("acknowledged.mem"))?;
        ensure!(
            acknowledged.len() as u64
                == observation["readset"]["bytes"]
                    .as_u64()
                    .context("readset length")?
                && sims_fixtures::digest(&acknowledged)
                    == observation["readset"]["sha256"]
                        .as_str()
                        .context("readset hash")?,
            "native acknowledged memory changed"
        );
        Ok(Cut {
            observation,
            acknowledged,
        })
    }
    fn input(value: &Value) -> Result<(Vec<AssetInput>, Vec<CharacterInput>)> {
        let a = &value["domains"]["animations"];
        Ok((
            serde_json::from_value(a["payload"]["assets"].clone())?,
            serde_json::from_value(a["characters"].clone())?,
        ))
    }
    const EPOCH: &str = "5ec18620f4412251ffe0f78a29b68c64aff0f609893a998a67d2ba455a359299";
    fn import(value: &Value, memory: &Readset<'_>) -> Result<completed_animations::Imported> {
        let (assets, characters) = input(value)?;
        completed_animations::import_with_resident_lists(
            &assets,
            &characters,
            EPOCH,
            &value["domains"]["animations"]["payload"]["resident_lists"],
            &value["characters"]["event_cursors"],
            memory,
        )
    }
    fn actions(value: &Value, memory: &Readset<'_>) -> Result<usize> {
        use crate::original_session::completed_actions::{self, Identity};
        let rows = value["snapshot"]["objects"]
            .as_array()
            .context("native objects")?;
        let identities: Vec<_> = rows
            .iter()
            .map(|o| -> Result<Identity> {
                Ok(Identity {
                    object: o["id"].as_i64().context("object ID")?.try_into()?,
                    pointer: o["pointer"]
                        .as_u64()
                        .context("object pointer")?
                        .try_into()?,
                    person: o.get("person").is_some(),
                    frame_depth: o["frames"].as_array().context("native frame count")?.len(),
                })
            })
            .collect::<Result<_>>()?;
        let imported = completed_actions::import(&identities, memory)?;
        ensure!(
            serde_json::to_value(&imported.actors)?
                == value["domains"]["systems"]["fields"]["actions"]["actors"]
                && serde_json::to_value(&imported.uses)?
                    == value["domains"]["systems"]["action_uses"],
            "Rust native action/use import differs from independent original projection"
        );
        let native = value["threads"]["objects"]
            .as_array()
            .context("native threads")?;
        let contexts: BTreeMap<_, _> = native
            .iter()
            .map(|o| -> Result<_> {
                Ok((
                    i16::try_from(o["id"].as_i64().context("native ID")?)?,
                    o["resource_context"]
                        .as_u64()
                        .context("native resource context")?,
                ))
            })
            .collect::<Result<_>>()?;
        let mut count = 0;
        for (&actor, uses) in &imported.uses {
            let thread = native
                .iter()
                .find(|o| o["id"] == actor)
                .context("native actor thread")?;
            let frames = thread["frames"].as_array().context("native actor frames")?;
            let scopes = completed_actions::invocations(actor, frames.len(), uses)?;
            for (scope, frame) in scopes.iter().zip(frames) {
                ensure!(
                    Some(
                        &frame["code_owner_context"]
                            .as_u64()
                            .context("native frame context")?
                    ) == contexts.get(&scope.callee),
                    "action-use depth does not explain actual native frame code-owner context"
                );
                count += 1;
            }
        }
        Ok(count)
    }

    #[test]
    #[ignore = "private original 89073..89105 capture; component checks, zero replay parity credit"]
    fn original_33_cuts_import_retained_components_without_restarting_live_state() -> Result<()> {
        let started = std::time::Instant::now();
        let source = source()?;
        ensure!(
            source["observations"]
                .as_array()
                .context("observations")?
                .len()
                == 33,
            "native interval extent differs"
        );
        let mut live = 0;
        let mut scopes = 0;
        for i in 0..33 {
            let cut = cut(i, &source)?;
            let memory = Readset::parse(&cut.acknowledged)?;
            scopes += actions(&cut.observation, &memory)
                .with_context(|| format!("native action cut {i}"))?;
            let imported = import(&cut.observation, &memory)
                .with_context(|| format!("native animation cut {i}"))?;
            let (assets, characters) = input(&cut.observation)?;
            ensure!(
                imported.characters.len() == characters.len()
                    && imported.assets.len() == assets.len(),
                "component extent changed"
            );
            for c in &characters {
                let retained = serde_json::to_value(&imported.characters[&c.object].animator)?;
                ensure!(
                    retained["layers"] == c.animator["layers"],
                    "live native layer state changed"
                );
                let saved = postcard::to_allocvec(&imported.characters[&c.object].animator)?;
                let restored: sims_sim::pose::animator::Animator = postcard::from_bytes(&saved)?;
                ensure!(
                    serde_json::to_value(restored)? == retained,
                    "retained animation component cold roundtrip changed state"
                );
                for l in retained["layers"].as_array().context("layers")? {
                    live += l["layer"]["bindings"]
                        .as_array()
                        .context("bindings")?
                        .iter()
                        .filter(|b| !b["event_cursor"].is_null())
                        .count();
                }
            }
            ensure!(
                completed_animations::import(&assets, &characters, EPOCH).is_err(),
                "old property-free profile was weakened"
            );
        }
        eprintln!(
            "original retained components: 33 cuts, ticks 89073..89105, {live} live cursor positions, {scopes} use-derived native frame owners, animation cold roundtrips, {:.3}s; replay parity credit 0",
            started.elapsed().as_secs_f64()
        );
        Ok(())
    }

    #[test]
    #[ignore = "private captured native memory and adversarial component mutations"]
    fn original_native_bytes_reject_metadata_repairs_cursor_resets_and_missing_owners() -> Result<()>
    {
        let cut = cut(0, &source()?)?;
        let memory = Readset::parse(&cut.acknowledged)?;
        import(&cut.observation, &memory)?;
        let mut reset = cut.observation.clone();
        let binding = reset["domains"]["animations"]["characters"]
            .as_array_mut()
            .context("characters")?
            .iter_mut()
            .flat_map(|c| c["animator"]["layers"].as_array_mut().unwrap().iter_mut())
            .flat_map(|l| l["layer"]["bindings"].as_array_mut().unwrap().iter_mut())
            .find(|b| b["event_cursor"].as_i64().is_some_and(|v| v != 0))
            .context("nonzero original cursor")?;
        binding["event_cursor"] = Value::from(0);
        ensure!(
            import(&reset, &memory).is_err(),
            "invented cursor reset accepted"
        );
        let mut altered = cut.observation.clone();
        let cursor = &mut altered["characters"]["event_cursors"][0];
        let index = (cursor["index_bits"].as_u64().context("cursor bits")? as u32) ^ 1;
        let mut raw = bytes(cursor["raw"].as_str().context("cursor raw")?)?;
        raw[4..8].copy_from_slice(&index.to_le_bytes());
        cursor["index_bits"] = Value::from(index);
        cursor["raw"] = Value::from(raw.iter().map(|b| format!("{b:02x}")).collect::<String>());
        ensure!(
            import(&altered, &memory).is_err(),
            "self-consistent metadata substituted for native cursor bytes"
        );
        let mut missing = cut.observation.clone();
        missing["domains"]["animations"]["payload"]["resident_lists"]["properties"]
            .as_array_mut()
            .unwrap()
            .remove(0);
        ensure!(
            import(&missing, &memory).is_err(),
            "missing native property owner accepted"
        );
        let mut altered_motion = cut.observation.clone();
        let motion =
            &mut altered_motion["domains"]["animations"]["payload"]["assets"][0]["motions"][0];
        let mut raw = bytes(motion["raw"].as_str().context("motion raw")?)?;
        raw[0x30] ^= 1;
        motion["raw"] = Value::from(hex(&raw));
        let error = import(&altered_motion, &memory)
            .err()
            .context("forged motion/list association accepted")?;
        ensure!(
            error.to_string().contains("owner header differs"),
            "motion association was not checked against native owner"
        );
        let mut unobserved = cut.acknowledged.clone();
        let constructor = 0x594778u32;
        for r in memory.records() {
            if r.address <= constructor
                && u64::from(constructor) < u64::from(r.address) + r.bytes.len() as u64
            {
                let at = r.bytes.as_ptr() as usize - cut.acknowledged.as_ptr() as usize
                    + (constructor - r.address) as usize;
                unobserved[at] ^= 1;
            }
        }
        ensure!(
            import(&cut.observation, &Readset::parse(&unobserved)?).is_err(),
            "altered constructor code accepted"
        );
        Ok(())
    }
}
