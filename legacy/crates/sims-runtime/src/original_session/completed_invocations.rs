//! Raw completed stacks with native use-depth invocation ownership. This is a
//! constructorless storage component, not a Thread, scheduler status or Session.
//! The partial Program contains only observed definitions and active routines;
//! empty unrelated banks grant no resource/execution capability or source epoch.
pub mod resident_sources;
pub mod resident_constants;
use super::completed_actions::{self, Identity, Use};
use anyhow::{Context, Result, ensure};
use sims_fixtures::readset::Readset;
use sims_vm::{
    Definition, Frame, Location, ObjectId, Program,
    program::{Module, Routine},
    retained_frames::{ContextBinding, RetainedFrame, RetainedStack},
};
use std::collections::{BTreeMap, BTreeSet};

fn word(raw: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(raw[off..off + 4].try_into().unwrap())
}
fn short(raw: &[u8], off: usize) -> u16 {
    u16::from_le_bytes(raw[off..off + 2].try_into().unwrap())
}
fn at(p: u32, offset: u32) -> Result<u32> {
    p.checked_add(offset)
        .context("native invocation address overflow")
}
fn name(p: u32) -> String {
    format!("native-provider:{p:08x}")
}

fn vector(m: &Readset<'_>, header: &[u8], stride: usize, maximum: usize) -> Result<Vec<u8>> {
    ensure!(
        header.len() == 12,
        "native invocation vector header extent differs"
    );
    let (begin, end, capacity) = (word(header, 0), word(header, 4), word(header, 8));
    ensure!(
        begin <= end
            && end <= capacity
            && (begin != 0 || capacity == 0)
            && (end - begin) as usize % stride == 0
            && (capacity - begin) as usize % stride == 0
            && (end - begin) as usize / stride <= maximum,
        "native invocation vector extent differs"
    );
    m.read(begin, (end - begin) as usize)
}

pub struct Object {
    pub binding: ContextBinding,
    /// None means an observed empty active stack, not a request to install Main.
    pub stack: Option<RetainedStack>,
    pub raw_arena: Vec<u8>,
    pub frame_pointers: Vec<u32>,
}
pub struct Imported {
    pub readset_sha256: String,
    pub program: Program,
    pub objects: BTreeMap<ObjectId, Object>,
    pub routines: Vec<Receipt>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt {
    pub provider: u32,
    pub component: u32,
    pub record: u32,
    pub handle: u32,
    pub data: u32,
    pub bytes: usize,
    pub tree: u16,
    pub sha256: String,
}
struct ContextOwner {
    binding: ContextBinding,
    global: u32,
    private: u32,
    semi: u32,
    words: Vec<i16>,
}

/// Ordered aggregate by-ID lookup, with no type-filter, filename, lazy load,
/// callback or starting-file fallback. Capture-wide budgets bound corrupt chains.
struct Resources<'a, 'b> {
    memory: &'a Readset<'b>,
    providers: BTreeMap<u32, Vec<u32>>,
    components: BTreeMap<u32, BTreeMap<(u16, u8), (u32, u32)>>,
    payloads: BTreeMap<(u32, u16), (Vec<u8>, Receipt)>,
    traversed: usize,
}
impl<'a, 'b> Resources<'a, 'b> {
    fn provider(&mut self, p: u32) -> Result<Vec<u32>> {
        if let Some(children) = self.providers.get(&p) {
            return Ok(children.clone());
        }
        let raw = self.memory.read(p, 172)?;
        ensure!(
            word(&raw, 0) == 0x6672b0,
            "unqualified native aggregate provider class"
        );
        let mut children = Vec::new();
        for i in 0..8 {
            let child = word(&raw, 12 + 20 * i);
            if child == 0 {
                break;
            }
            children.push(child);
        }
        self.providers.insert(p, children.clone());
        Ok(children)
    }
    fn buckets(&self, raw: &[u8]) -> Result<Vec<u32>> {
        let raw = vector(self.memory, &raw[4..16], 4, 65536)?;
        Ok(raw.chunks_exact(4).map(|b| word(b, 0)).collect())
    }
    fn chain(&mut self, heads: Vec<u32>, length: usize) -> Result<Vec<(u32, Vec<u8>)>> {
        let mut seen = BTreeSet::new();
        let mut rows = Vec::new();
        for mut p in heads {
            let mut chain_seen = BTreeSet::new();
            while p != 0 {
                self.traversed += 1;
                ensure!(
                    self.traversed <= 262144 && chain_seen.insert(p),
                    "cyclic/excess native resource chain"
                );
                let raw = self.memory.read(p, length)?;
                let next = word(&raw, 0);
                if seen.insert(p) {
                    rows.push((p, raw));
                }
                p = next;
            }
        }
        Ok(rows)
    }
    fn component(&mut self, p: u32) -> Result<()> {
        if self.components.contains_key(&p) {
            return Ok(());
        }
        let result=self.type_records(p,0x42484156)?;
        self.components.insert(p, result);
        Ok(())
    }
    fn type_records(&mut self,p:u32,tag:u32)->Result<BTreeMap<(u16,u8),(u32,u32)>> {
        let raw = self.memory.read(p, 44)?;
        ensure!(
            word(&raw, 0) == 0x666944,
            "unqualified native IFF component class"
        );
        let map = self.memory.read(word(&raw, 24), 24)?;
        let heads = self.buckets(&map)?;
        let outer = self.chain(heads, 0x1c)?;
        let mut result = BTreeMap::new();
        for (_, row) in outer {
            if word(&row, 4) != tag {
                continue;
            }
            let heads = self.buckets(&row[8..])?;
            for (record, row) in self.chain(heads, 0x64)? {
                let key = (short(&row, 4), row[6]);
                ensure!(
                    result.insert(key, (record, word(&row, 15))).is_none(),
                    "duplicate native current typed resource map key"
                );
            }
        }
        Ok(result)
    }
    fn routine(&mut self, provider: u32, tree: u16) -> Result<(Vec<u8>, Receipt)> {
        if let Some(result) = self.payloads.get(&(provider, tree)) {
            return Ok(result.clone());
        }
        ensure!(provider != 0, "native current BHAV has no provider");
        for component in self.provider(provider)? {
            self.component(component)?;
            if let Some(&(record, handle)) = self.components[&component].get(&(tree, 0)) {
                ensure!(
                    handle != 0,
                    "native current BHAV is nonresident; no file substitution"
                );
                let header = self.memory.read(handle, 8)?;
                let (data, count) = (word(&header, 0), word(&header, 4) as usize);
                ensure!(
                    (12..=1 << 20).contains(&count),
                    "native current BHAV payload exceeds bound"
                );
                let payload = self.memory.read(data, count)?;
                let receipt = Receipt {
                    provider,
                    component,
                    record,
                    handle,
                    data,
                    bytes: count,
                    tree,
                    sha256: sims_fixtures::digest(&payload),
                };
                self.payloads
                    .insert((provider, tree), (payload.clone(), receipt.clone()));
                return Ok((payload, receipt));
            }
        }
        anyhow::bail!("active native BHAV absent from ordered current provider")
    }
}

fn context(object: Identity, m: &Readset<'_>) -> Result<ContextOwner> {
    ensure!(
        short(&m.read(at(object.pointer, 0xf0)?, 2)?, 0) as i16 == object.object,
        "native invocation object identity differs"
    );
    let association = m.read(at(object.pointer, 0x118)?, 8)?;
    let (definition, resource) = (word(&association, 0), word(&association, 4));
    let rr = m.read(resource, 0xb8)?;
    let pointer = word(&rr, 12);
    let c = m.read(pointer, 24)?;
    ensure!(
        word(&c, 0) == 0x666514
            && word(&c, 8) == word(&rr, 0)
            && word(&c, 12) == word(&rr, 4)
            && word(&rr, 0x60) == definition,
        "native resource/context/definition reverse owner differs"
    );
    let language = m.read(word(&c, 16), 8)?;
    ensure!(
        word(&language, 0) == 0x667aa0 && word(&language, 4) == resource,
        "native context language owner differs"
    );
    let raw = m.read(definition, 0xd8)?;
    let words: Vec<_> = raw.chunks_exact(2).map(|b| short(b, 0) as i16).collect();
    let private = word(&c, 8);
    let semi = word(&c, 12);
    ensure!(
        private != 0 && word(&c, 4) != 0,
        "native context current private/global provider missing"
    );
    Ok(ContextOwner {
        binding: ContextBinding {
            object: object.object,
            definition: Definition {
                source: name(private),
                object: (word(&rr, 0xb4) >> 16) as u16,
                guid: word(&raw, 0x1c),
                slots: words[20] as u16,
            },
            native_context: pointer,
            registry_lifetime: None,
        },
        global: word(&c, 4),
        private,
        semi,
        words,
    })
}

pub fn import(
    objects: &[Identity],
    uses: &BTreeMap<ObjectId, Vec<Use>>,
    memory: &Readset<'_>,
) -> Result<Imported> {
    ensure!(
        objects.len() <= 32767,
        "native invocation inventory exceeds bound"
    );
    let mut pointers = BTreeSet::new();
    let mut owners = BTreeMap::new();
    for &o in objects {
        ensure!(
            o.object > 0
                && o.pointer >= 0x10000
                && pointers.insert(o.pointer)
                && !owners.contains_key(&o.object),
            "invalid/aliased native invocation identity"
        );
        owners.insert(o.object, context(o, memory)?);
    }
    ensure!(
        uses.keys().copied().collect::<BTreeSet<_>>()
            == objects
                .iter()
                .filter(|o| o.person)
                .map(|o| o.object)
                .collect(),
        "native invocation person/use coverage differs"
    );
    let globals: BTreeSet<_> = owners.values().map(|o| o.global).collect();
    ensure!(
        globals.len() == 1,
        "native invocation global provider is ambiguous/missing"
    );
    let mut program = Program {
        globals: name(*globals.first().unwrap()),
        ..Program::default()
    };
    let mut semi_owners = BTreeMap::new();
    for o in owners.values() {
        let semi = (o.semi != 0).then(|| name(o.semi));
        if let Some(old) = semi_owners.insert(o.private, semi.clone()) {
            ensure!(
                old == semi,
                "native private provider has conflicting semiglobal owner"
            );
        }
        let module = program
            .modules
            .entry(name(o.private))
            .or_insert_with(|| Module {
                source: name(o.private),
                ..Module::default()
            });
        module.semiglobal = semi;
        if let Some(old) = module
            .object_definition_words
            .insert(o.binding.definition.object, o.words.clone())
        {
            ensure!(
                old == o.words,
                "native definition key aliases conflicting OBJD words"
            );
        }
    }
    let mut resources = Resources {
        memory,
        providers: BTreeMap::new(),
        components: BTreeMap::new(),
        payloads: BTreeMap::new(),
        traversed: 0,
    };
    let mut result = BTreeMap::new();
    for &o in objects {
        let header = memory.read(o.pointer, 0x2c)?;
        let (begin, end) = (word(&header, 8), word(&header, 12));
        ensure!(
            begin <= end
                && (begin != 0 || end == 0)
                && begin % 8 == 0
                && (end - begin) % 8 == 0
                && end - begin <= sims_vm::MAX_STACK as u32 * 1040,
            "native invocation arena extent differs"
        );
        let raw_arena = memory.read(begin, (end - begin) as usize)?;
        let vector = vector(memory, &header[0x10..0x1c], 4, sims_vm::MAX_STACK)?;
        let frame_pointers: Vec<_> = vector.chunks_exact(4).map(|b| word(b, 0)).collect();
        ensure!(
            frame_pointers.len() == o.frame_depth,
            "native active frame count differs"
        );
        let empty = vec![];
        let invocation = completed_actions::invocations(
            o.object,
            frame_pointers.len(),
            uses.get(&o.object).unwrap_or(&empty),
        )?;
        let mut frames = Vec::new();
        let mut after = begin;
        let mut bindings = BTreeMap::new();
        for (&pointer, inv) in frame_pointers.iter().zip(invocation) {
            ensure!(
                pointer % 8 == 0 && pointer >= after && u64::from(pointer) + 16 <= u64::from(end),
                "native active frame overlaps/exceeds arena"
            );
            let offset = (pointer - begin) as usize;
            let h = &raw_arena[offset..offset + 16];
            let (tree, node) = (short(h, 0), short(h, 2) as usize);
            let (argc, locals) = (h[7] as usize, h[6] as usize);
            let footprint = 16 + 2 * ((argc + locals + 3) & !3);
            ensure!(
                tree & 0x8000 == 0 && node <= 0xfa && offset + footprint <= raw_arena.len(),
                "flagged/sentinel/out-of-arena native frame needs another profile"
            );
            after = pointer + footprint as u32;
            let callee = owners
                .get(&inv.callee)
                .context("native invocation callee is absent")?;
            ensure!(
                word(h, 12) == callee.binding.native_context,
                "native use scope differs from frame code-owner context"
            );
            let provider = if tree < 4096 {
                callee.global
            } else if tree < 8192 {
                callee.private
            } else {
                callee.semi
            };
            let (payload, _) = resources.routine(provider, tree)?;
            let source = name(provider);
            let module = program
                .modules
                .entry(source.clone())
                .or_insert_with(|| Module {
                    source: source.clone(),
                    ..Module::default()
                });
            if !module.routines.contains_key(&tree) {
                module.routines.insert(
                    tree,
                    Routine {
                        name: format!("resident:{tree}"),
                        body: sims_formats::behavior::Behavior::parse(&payload)?,
                    },
                );
            }
            let values: Vec<_> = raw_arena[offset + 16..offset + 16 + 2 * (argc + locals)]
                .chunks_exact(2)
                .map(|b| short(b, 0) as i16)
                .collect();
            frames.push(RetainedFrame {
                context_object: inv.callee,
                frame: Frame {
                    continuation: word(h, 8),
                    action_target: inv.action_target,
                    action_icon: inv.action_icon,
                    code_owner: callee.binding.definition.source.clone(),
                    location: Location {
                        source,
                        bhav: tree,
                        instruction: node,
                    },
                    caller: inv.caller,
                    callee: inv.callee,
                    stack_object: short(h, 4) as i16,
                    args: values[..argc].to_vec(),
                    locals: values[argc..].to_vec(),
                    opaque_locals: BTreeSet::new(),
                    storage_offset: offset / 2,
                },
            });
            bindings.insert(inv.callee, callee.binding.clone());
        }
        let stack = if frames.is_empty() {
            None
        } else {
            Some(
                RetainedStack::from_bytes(
                    &program,
                    &raw_arena,
                    frames,
                    bindings.into_values().collect(),
                )
                .map_err(anyhow::Error::msg)?,
            )
        };
        result.insert(
            o.object,
            Object {
                binding: owners[&o.object].binding.clone(),
                stack,
                raw_arena,
                frame_pointers,
            },
        );
    }
    Ok(Imported {
        readset_sha256: memory.source_sha256().into(),
        program,
        objects: result,
        routines: resources.payloads.into_values().map(|(_, r)| r).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Read, path::PathBuf};
    fn recorded(rows: &[(u32, Vec<u8>)]) -> Vec<u8> {
        let mut raw = b"SIMS-MEM-1\0".to_vec();
        raw.extend((rows.len() as u32).to_le_bytes());
        for (p, b) in rows {
            raw.extend(p.to_le_bytes());
            raw.extend((b.len() as u32).to_le_bytes());
            raw.extend(b);
        }
        raw
    }
    fn toy_resources(first_tree: u16, first_live: bool, cycle: bool) -> Vec<u8> {
        let mut rows = Vec::new();
        let mut provider = vec![0; 172];
        provider[..4].copy_from_slice(&0x6672b0u32.to_le_bytes());
        for (i, tree, live, marker) in [
            (0usize, first_tree, first_live, 0x11u8),
            (1, 4100, true, 0x22),
        ] {
            let p = 0x20000 + i as u32 * 0x1000;
            provider[12 + 20 * i..16 + 20 * i].copy_from_slice(&p.to_le_bytes());
            let mut component = vec![0; 44];
            component[..4].copy_from_slice(&0x666944u32.to_le_bytes());
            component[24..28].copy_from_slice(&(p + 0x100).to_le_bytes());
            rows.push((p, component));
            let mut map = vec![0; 24];
            for (off, v) in [(4, p + 0x200), (8, p + 0x204), (12, p + 0x204)] {
                map[off..off + 4].copy_from_slice(&v.to_le_bytes());
            }
            rows.push((p + 0x100, map));
            rows.push((p + 0x200, (p + 0x210).to_le_bytes().to_vec()));
            let mut outer = vec![0; 0x1c];
            outer[4..8].copy_from_slice(&0x42484156u32.to_le_bytes());
            for (off, v) in [(12, p + 0x300), (16, p + 0x304), (20, p + 0x304)] {
                outer[off..off + 4].copy_from_slice(&v.to_le_bytes());
            }
            rows.push((p + 0x210, outer));
            rows.push((p + 0x300, (p + 0x310).to_le_bytes().to_vec()));
            let mut inner = vec![0; 0x64];
            inner[4..6].copy_from_slice(&tree.to_le_bytes());
            if live {
                inner[15..19].copy_from_slice(&(p + 0x400).to_le_bytes());
            }
            if i == 0 && cycle {
                inner[..4].copy_from_slice(&(p + 0x310).to_le_bytes());
            }
            rows.push((p + 0x310, inner));
            if live {
                let mut handle = (p + 0x420).to_le_bytes().to_vec();
                handle.extend(12u32.to_le_bytes());
                rows.push((p + 0x400, handle));
                rows.push((p + 0x420, vec![marker; 12]));
            }
        }
        rows.push((0x10000, provider));
        recorded(&rows)
    }
    fn resources<'a, 'b>(memory: &'a Readset<'b>) -> Resources<'a, 'b> {
        Resources {
            memory,
            providers: BTreeMap::new(),
            components: BTreeMap::new(),
            payloads: BTreeMap::new(),
            traversed: 0,
        }
    }
    #[test]
    fn native_by_id_keeps_component_order_and_never_substitutes_for_lazy_match() -> Result<()> {
        // Authored data-structure controls, never original-engine execution.
        let raw = toy_resources(4100, true, false);
        let m = Readset::parse(&raw)?;
        assert_eq!(resources(&m).routine(0x10000, 4100)?.0, vec![0x11; 12]);
        let raw = toy_resources(4101, true, false);
        let m = Readset::parse(&raw)?;
        assert_eq!(resources(&m).routine(0x10000, 4100)?.0, vec![0x22; 12]);
        let raw = toy_resources(4100, false, false);
        let m = Readset::parse(&raw)?;
        assert!(resources(&m).routine(0x10000, 4100).is_err());
        let raw = toy_resources(4100, true, true);
        let m = Readset::parse(&raw)?;
        assert!(resources(&m).routine(0x10000, 4100).is_err());
        Ok(())
    }
    #[test]
    fn repeated_component_slots_and_shared_bucket_heads_keep_native_aliases() -> Result<()> {
        let source = toy_resources(4101, true, false);
        let m = Readset::parse(&source)?;
        let mut rows: Vec<_> = m
            .records()
            .iter()
            .map(|r| (r.address, r.bytes.to_vec()))
            .collect();
        let provider = rows.iter_mut().find(|r| r.0 == 0x10000).unwrap();
        provider.1[32..36].copy_from_slice(&0x20000u32.to_le_bytes());
        provider.1[52..56].copy_from_slice(&0x21000u32.to_le_bytes());
        let raw = recorded(&rows);
        let m = Readset::parse(&raw)?;
        assert_eq!(resources(&m).routine(0x10000, 4100)?.0, vec![0x22; 12]);
        let source = toy_resources(4100, true, false);
        let m = Readset::parse(&source)?;
        let mut rows: Vec<_> = m
            .records()
            .iter()
            .map(|r| (r.address, r.bytes.to_vec()))
            .collect();
        rows.iter_mut()
            .find(|r| r.0 == 0x20200)
            .unwrap()
            .1
            .extend(0x20210u32.to_le_bytes());
        let map = &mut rows.iter_mut().find(|r| r.0 == 0x20100).unwrap().1;
        map[8..12].copy_from_slice(&0x20208u32.to_le_bytes());
        map[12..16].copy_from_slice(&0x20208u32.to_le_bytes());
        let raw = recorded(&rows);
        let m = Readset::parse(&raw)?;
        assert_eq!(resources(&m).routine(0x10000, 4100)?.0, vec![0x11; 12]);
        Ok(())
    }
    fn patched(source: &[u8], address: u32, value: &[u8]) -> Result<Vec<u8>> {
        let m = Readset::parse(source)?;
        m.read(address, value.len())?;
        let mut changed = source.to_vec();
        for r in m.records() {
            for (i, &byte) in value.iter().enumerate() {
                let p = u64::from(address) + i as u64;
                if u64::from(r.address) <= p && p < u64::from(r.address) + r.bytes.len() as u64 {
                    let offset = r.bytes.as_ptr() as usize - source.as_ptr() as usize
                        + (p - u64::from(r.address)) as usize;
                    changed[offset] = byte;
                }
            }
        }
        Ok(changed)
    }
    #[test]
    #[ignore = "private native B source-owner and route mutation controls; zero parity credit"]
    fn original_import_rejects_source_mixing_lazy_code_and_route_owner_repairs() -> Result<()> {
        let (state, raw) = original(0)?;
        let memory = Readset::parse(&raw)?;
        let ids = identities(&state)?;
        let actions = completed_actions::import(&ids, &memory)?;
        let invocations = import(&ids, &actions.uses, &memory)?;
        let routes = super::super::completed_routes::import(&ids, &invocations, &memory)?;
        let (_, other) = original(32)?;
        ensure!(
            super::super::completed_routes::import(&ids, &invocations, &Readset::parse(&other)?)
                .is_err(),
            "mixed completed cuts accepted"
        );
        let (actor, route) = (16, &routes[&16]);
        let owner = route.contexts[0].pointer;
        let other_person = ids
            .iter()
            .find(|i| i.object == 18)
            .context("other native person")?;
        for (address, value) in [
            (owner + 0x4c, other_person.pointer),
            (owner + 0x68, other_person.pointer),
            (route.animator_pointer + 0x184, other_person.pointer + 0xae0),
            (
                route
                    .contexts
                    .last()
                    .context("native child route")?
                    .frame_pointer
                    + 8,
                0,
            ),
        ] {
            let changed = patched(&raw, address, &value.to_le_bytes())?;
            let m = Readset::parse(&changed)?;
            let imported = import(&ids, &actions.uses, &m)?;
            ensure!(
                super::super::completed_routes::import(&ids, &imported, &m).is_err(),
                "native route owner/continuation repair accepted for {actor} at {address:08x}"
            );
        }
        let receipt = invocations
            .routines
            .first()
            .context("resident routine receipt")?;
        let lazy = patched(&raw, receipt.record + 15, &0u32.to_le_bytes())?;
        ensure!(
            import(&ids, &actions.uses, &Readset::parse(&lazy)?).is_err(),
            "nonresident native BHAV got a substitute"
        );
        let cycle = patched(&raw, receipt.record, &receipt.record.to_le_bytes())?;
        ensure!(
            import(&ids, &actions.uses, &Readset::parse(&cycle)?).is_err(),
            "cyclic native source chain accepted"
        );
        Ok(())
    }
    fn original(index: usize) -> Result<(serde_json::Value, Vec<u8>)> {
        let base=PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/private/goth-completed-continuation-20261030-third/results/native-sequence");
        let manifest = std::fs::read(base.join("capture-result.json"))?;
        ensure!(
            sims_fixtures::digest(&manifest)
                == "4139107d1965925bf3ee5f44fa524d5d4b1fae22bfdb617edfffb69d33bcd393",
            "native source capture receipt changed"
        );
        let manifest: serde_json::Value = serde_json::from_slice(&manifest)?;
        let source = &manifest["observations"][index];
        let cut = base.join(format!("completed-{index:02}"));
        let raw = std::fs::read(cut.join("observation.json.gz"))?;
        ensure!(
            sims_fixtures::digest(&raw)
                == source["sha256"]
                    .as_str()
                    .context("compressed source hash")?,
            "native compressed source changed"
        );
        let mut decoder = flate2::bufread::GzDecoder::new(raw.as_slice());
        let mut decoded = vec![];
        decoder
            .by_ref()
            .take((64 << 20) + 1)
            .read_to_end(&mut decoded)?;
        ensure!(
            decoded.len() <= 64 << 20
                && decoder.into_inner().is_empty()
                && decoded.len() as u64
                    == source["decoded_bytes"]
                        .as_u64()
                        .context("decoded source length")?
                && sims_fixtures::digest(&decoded)
                    == source["decoded_sha256"]
                        .as_str()
                        .context("decoded source hash")?,
            "native decoded source changed"
        );
        let state: serde_json::Value = serde_json::from_slice(&decoded)?;
        let memory = std::fs::read(cut.join("acknowledged.mem"))?;
        ensure!(
            sims_fixtures::digest(&memory)
                == state["readset"]["sha256"]
                    .as_str()
                    .context("native memory hash")?,
            "native observed memory changed"
        );
        Ok((state, memory))
    }
    #[test]
    #[ignore = "private original moving B and current neighborhood projection; component join, no tick credit"]
    fn original_completed_moving_world_joins_without_starting_lifecycles() -> Result<()> {
        for index in [0, 32] {
            moving_world_cut(index, false)?;
        }
        Ok(())
    }
    #[test]
    #[ignore = "private completed B VM initial assembly and named restore; no gameplay tick credit"]
    fn original_completed_retained_vm_initial_assembly() -> Result<()> {
        moving_world_cut(0, true)
    }
    fn moving_world_cut(index: usize, assemble_vm: bool) -> Result<()> {
        let (state, raw) = original(index)?;
        let memory = Readset::parse(&raw)?;
        let ids = identities(&state)?;
        let actions = completed_actions::import(&ids, &memory)?;
        let invocations = import(&ids, &actions.uses, &memory)?;
        let (file, hash) = if index == 0 {
            (
                "baseline",
                "988b2c729c956c522621582539c16adc4a4557621a38a171ac5736dd973d47fb",
            )
        } else {
            (
                "endpoint",
                "4163ea4d19c985a244dfcc1489522753e5c380f389760c042c5e5a7831a49877",
            )
        };
        let projection = std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../../fixtures/private/goth-moving-world-20261030-first/person-world-{file}-first.json")))?;
        ensure!(
            sims_fixtures::digest(&projection) == hash,
            "moving neighborhood projection differs"
        );
        let p: serde_json::Value = serde_json::from_slice(&projection)?;
        ensure!(
            p["readset_sha256"] == memory.source_sha256() && p["tick"] == state["tick"],
            "moving neighborhood source epoch differs"
        );
        let person = super::super::completed_person_world::import(
            p["person_fields"].clone(),
            serde_json::from_value(p["native_relationship_orders"].clone())?,
            p["retained_external_family_owners"].clone(),
            serde_json::from_value(p["family_delivery"].clone())?,
        )?;
        let joined = super::super::completed_world::captured::import(
            &state,
            &invocations,
            &actions,
            person,
            &memory,
            &format!(
                "goth{}-retained-source-first",
                state["tick"].as_u64().unwrap()
            ),
        )?;
        ensure!(
            joined.world.objects.len() == 346,
            "moving World object coverage differs"
        );
        ensure!(
            joined.world.objects[&21].accessories == ["suit-book101"],
            "moving World dropped the loaded book"
        );
        let source = &joined.program.accessories["suit-book101"];
        ensure!(
            source.len() == 1
                && source
                    .keys()
                    .all(|k| k.starts_with("native-resident-readset:")),
            "native accessory receipt was mislabeled as an immutable file source"
        );
        ensure!(
            joined.world.actions.as_ref().unwrap().actors == actions.actors,
            "moving World changed action owners"
        );
        // World has fields omitted from legacy named serialization. Positional
        // postcard decoding is not its managed/fixture restore contract.
        let encoded = serde_json::to_vec(&joined.world)?;
        let restored: sims_vm::World =
            serde_json::from_slice(&encoded).context("moving World named-field storage restore")?;
        ensure!(
            restored == joined.world,
            "moving World storage roundtrip changed state"
        );
        if index == 0 {
            let path=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/private/goth-moving-world-20261030-first/function-caches-physical-B-first.json");
            let raw = std::fs::read(path)?;
            ensure!(
                sims_fixtures::digest(&raw)
                    == "94292555ebb5fabda0993f8ce9d5af2dbd3722d0b6fe8bf60f0e3545ef64cf74",
                "physical cached callback artifact differs"
            );
            let source: serde_json::Value = serde_json::from_slice(&raw)?;
            let mut callbacks = super::super::completed_world::callbacks::import(
                &state,
                &source,
                &memory,
                &invocations,
                joined.program.clone(),
            )?;
            if assemble_vm {
                let raw=std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/private/goth-retained-vm-20261030-first/interactions-physical-B-second.json"))?;
                ensure!(
                    sims_fixtures::digest(&raw)
                        == "e557ed55ed3b4e8671156dd1d17084007aadced8ea9cefec840b35364dc22d67",
                    "native interaction artifact differs"
                );
                let source: serde_json::Value = serde_json::from_slice(&raw)?;
                callbacks.program = super::super::completed_world::interactions::import(
                    &state,
                    &source,
                    &memory,
                    &invocations,
                    callbacks.program,
                )?;
            }
            let stacks = super::super::completed_world::callbacks::bind_lifetimes(
                &invocations,
                &callbacks.program,
                &joined.world,
            )?;
            ensure!(
                callbacks.cached.len() == 346 && stacks.len() == 346,
                "native callbacks/lifetime stack coverage differs"
            );
            ensure!(
                stacks.values().map(|s| s.frames().len()).sum::<usize>() == 799,
                "fresh lifetime binding changed active frame count"
            );
            eprintln!(
                "moving B bound cached callbacks for {} objects and fresh lifetimes for {} retained frames; no VM execution or parity credit",
                callbacks.cached.len(),
                799
            );
            let mut damaged = source.clone();
            damaged["tables"][0]["records"][0]["action"] = serde_json::json!(65535);
            ensure!(
                super::super::completed_world::callbacks::import(
                    &state,
                    &damaged,
                    &memory,
                    &invocations,
                    joined.program.clone()
                )
                .is_err(),
                "callback decoded/raw mutation accepted"
            );
            if assemble_vm {
                let path=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/private/goth-retained-vm-20261030-first/registry-controls-physical-B-first.json");
                let raw = std::fs::read(path)?;
                ensure!(
                    sims_fixtures::digest(&raw)
                        == "814527960753cc8da8d802a85aedafcbd5b53e0622ba19044805751a9a85b128",
                    "physical registry caller artifact differs"
                );
                let caller: serde_json::Value = serde_json::from_slice(&raw)?;
                let routes = super::super::completed_routes::import(&ids, &invocations, &memory)?;
                let mut delivery = Vec::new();
                let mut next = 1;
                for (&actor, person) in &routes {
                    let owner = &invocations.objects[&actor];
                    let depth = owner.stack.as_ref().unwrap().frames().len();
                    for c in &person.contexts {
                        delivery.push(super::super::completed_route_controls::Delivery {
                            actor,
                            frame_pointer: c.frame_pointer,
                            request: next,
                        });
                        next += 1;
                    }
                    if !person.contexts.iter().any(|c| c.frame_depth == depth) {
                        delivery.push(super::super::completed_route_controls::Delivery {
                            actor,
                            frame_pointer: *owner.frame_pointers.last().unwrap(),
                            request: next,
                        });
                        next += 1;
                    }
                }
                let pending = super::super::completed_route_controls::import(
                    &invocations,
                    &routes,
                    &delivery,
                )?;
                use sims_vm::retained_completed::{CompletedAdapterEpoch, IdentityMapping};
                let key = "goth89073-retained-vm-first";
                let thread_ids: BTreeMap<_, _> = joined
                    .world
                    .objects
                    .keys()
                    .map(|&id| (id, id as u64))
                    .collect();
                let epoch = CompletedAdapterEpoch {
                    fresh: sims_vm::retained_epoch::FreshUnpublishedRequestEpoch {
                        source_key: key.into(),
                        accepted_commands: 0,
                        published_thread_handles: 0,
                        published_request_handles: 0,
                    },
                    threads: thread_ids
                        .iter()
                        .map(|(&id, &adapter)| IdentityMapping {
                            source_owner: format!("{key}/native-object-{id}"),
                            adapter,
                        })
                        .collect(),
                    requests: delivery
                        .iter()
                        .map(|d| IdentityMapping {
                            source_owner: format!(
                                "{key}/frame:{}:{:08x}",
                                d.actor, d.frame_pointer
                            ),
                            adapter: d.request,
                        })
                        .collect(),
                    next_thread: thread_ids.values().copied().max().unwrap() + 1,
                    next_request: next,
                };
                let vm = super::super::completed_world::threads::import(
                    super::super::completed_world::threads::Input {
                        state: &state,
                        caller: &caller,
                        memory: &memory,
                        invocations: &invocations,
                        program: &callbacks.program,
                        world: joined.world.clone(),
                        cached: &callbacks.cached,
                        stacks,
                        pending: &pending,
                        thread_ids,
                        epoch,
                    },
                )?;
                ensure!(
                    vm.tick == 89073 && vm.threads.len() == 346,
                    "retained VM initial epoch differs"
                );
                let saved = sims_vm::Snapshot::capture(&callbacks.program, &vm);
                let bytes = serde_json::to_vec(&saved)?;
                let restored = sims_vm::Snapshot::from_json(&bytes, &callbacks.program)
                    .map_err(anyhow::Error::msg)?;
                ensure!(
                    restored == vm,
                    "retained VM named cold roundtrip changed owned state"
                );
                let lot = super::super::completed_lot::import_captured(
                    &state["domains"]["lot"],
                    &memory,
                )?;
                let physical = super::super::completed_lot::physical::import(&lot, &vm.world)?;
                let architecture =
                    super::super::completed_lot::architecture(&lot, &callbacks.program, &vm.world)?;
                eprintln!(
                    "moving B lot geometry, physical placement and architecture imported; selected person {:?}; no VM execution",
                    physical.selected_person
                );
                let _ = architecture;
                let owners = super::super::completed_simulation::import_captured(
                    &state["domains"]["simulation"],
                    &vm,
                    &memory,
                )?;
                let scored = owners.with_captured_score_owners(
                    sims_vm::Snapshot::capture(&callbacks.program, &vm),
                    &memory,
                )?;
                let household = owners.household_captured(
                    &scored,
                    &memory,
                    "goth89073-household-fresh-unpublished-v2",
                )?;
                let environment = super::super::completed_lot::environment::import(
                    &lot,
                    &state["domains"]["room_environment"],
                    &scored.state.world,
                )?;
                ensure!(
                    household.state.people.len() == 4
                        && scored.state.rng_state == vm.rng_state
                        && scored.state.lot_rng_state == vm.lot_rng_state
                        && scored.state.crt_rng_state == vm.crt_rng_state,
                    "current household coverage/RNG changed during owner join"
                );
                let _ = environment;
                eprintln!(
                    "moving B Simulation clock/settings, four person score caches, household and room environment imported; no VM execution"
                );
                let (characters, resident_clips, skin_proof) = {
                    use super::super::{completed_animations as animation, completed_skeletons as skeleton, completed_skin as skin};
                    let a = &state["domains"]["animations"];
                    let assets = serde_json::from_value::<Vec<animation::AssetInput>>(a["payload"]["assets"].clone())?;
                    let characters = serde_json::from_value::<Vec<animation::CharacterInput>>(a["characters"].clone())?;
                    let animation = animation::import_with_resident_lists(
                        &assets, &characters,
                        "5ec18620f4412251ffe0f78a29b68c64aff0f609893a998a67d2ba455a359299",
                        &a["payload"]["resident_lists"], &state["characters"]["event_cursors"], &memory,
                    )?;
                    let parts = skeleton::captured::import(
                        &state, animation.characters.into_iter().map(|(id,c)| (id,c.animator)).collect(),
                        &memory, "goth89073-retained-character-v2",
                    )?;
                    let mut bad = serde_json::json!({"skeletons":state["skeletons"],"characters":state["characters"]});
                    let bone = &mut bad["skeletons"]["bones"][0];
                    let mut raw_bone = animation::bytes(bone["raw"].as_str().unwrap())?;
                    raw_bone[0x68] ^= 1;
                    bone["priority_bits"] = serde_json::json!(u32::from_le_bytes(raw_bone[0x68..0x6c].try_into()?));
                    bone["raw"] = serde_json::json!(raw_bone.iter().map(|b|format!("{b:02x}")).collect::<String>());
                    ensure!(skeleton::captured::import(&bad,
                        parts.iter().map(|(&id,p)| (id,p.animator.clone())).collect(),
                        &memory, "bad-bone-source").is_err(),
                        "current skeleton accepted self-consistent bytes absent from native memory");
                    let path=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/private/goth-retained-characters-20261030-first/skin-physical-B-first.json");
                    let raw = std::fs::read(path)?;
                    ensure!(sims_fixtures::digest(&raw) == "0c51407619621e0c271e1404a4113248318a81200d43a410b5173615e81e3836", "current physical skin proof differs");
                    let proof: skin::Input = serde_json::from_slice(&raw)?;
                    ensure!(skin::import(parts.clone(), &proof, "v1-must-reject-current-cut").is_err(),
                        "legacy skin gate accepted current profile");
                    let mut bad = proof.clone();
                    bad.parent_closure_sha256 = "0".repeat(64);
                    ensure!(skin::captured::import(parts.clone(), &bad, &state, &memory, "bad-parent").is_err(),
                        "current skin accepted altered caller binding");
                    let mut bad = proof.clone();
                    bad.code[0].raw.replace_range(..2, "00");
                    ensure!(skin::captured::import(parts.clone(), &bad, &state, &memory, "bad-code").is_err(),
                        "current skin accepted altered code");
                    let mut bad = proof.clone();
                    bad.actors.pop();
                    ensure!(skin::captured::import(parts.clone(), &bad, &state, &memory, "missing-actor").is_err(),
                        "current skin accepted missing actor");
                    let skins = skin::captured::import(parts, &proof, &state, &memory,
                        "goth89073-retained-character-v2")?;
                    let mut bad_routes = routes.clone();
                    bad_routes.get_mut(&21).unwrap().readset_sha256 = "0".repeat(64);
                    ensure!(super::super::completed_residents::import_captured(
                        &state["domains"]["residents"], skins.clone(), &state["domains"]["geometry"],
                        "bad-route-epoch", vm.tick, &bad_routes, &memory,
                    ).is_err(), "current resident accepted route from another source epoch");
                    let mut bad_routes = routes.clone();
                    let progress = &mut bad_routes.get_mut(&21).unwrap().movement.path.as_mut().unwrap().progress;
                    *progress = f32::from_bits(progress.to_bits() + 1);
                    ensure!(super::super::completed_residents::import_captured(
                        &state["domains"]["residents"], skins.clone(), &state["domains"]["geometry"],
                        "bad-movement", vm.tick, &bad_routes, &memory,
                    ).is_err(), "current resident accepted different retained movement");
                    let mut characters = super::super::completed_residents::import_captured(
                        &state["domains"]["residents"], skins, &state["domains"]["geometry"],
                        "goth89073-retained-character-v2", vm.tick, &routes, &memory,
                    )?;
                    let raw = std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/private/goth-retained-characters-20261030-first/person-labels-physical-B-first.json"))?;
                    ensure!(sims_fixtures::digest(&raw) == "78d42316a0552113276a6bdb322e73ccfba4c14e90a25af33b9e7b69cce64ebd", "current native label artifact differs");
                    let labels: serde_json::Value = serde_json::from_slice(&raw)?;
                    let values = super::super::completed_labels::captured::import(&labels, &proof, &state, &memory)?;
                    ensure!(characters.residents.keys().eq(values.keys()), "current label/character coverage differs");
                    for (id, value) in values {
                        characters.residents.get_mut(&id).unwrap().displayed_animation = value;
                    }
                    let mut bad = labels.clone();
                    bad["tick"] = serde_json::json!(vm.tick + 1);
                    ensure!(super::super::completed_labels::captured::import(&bad, &proof, &state, &memory).is_err(),
                        "current labels accepted different boundary");
                    let mut bad = labels.clone();
                    bad["objects"].as_array_mut().unwrap().pop();
                    ensure!(super::super::completed_labels::captured::import(&bad, &proof, &state, &memory).is_err(),
                        "current labels accepted incomplete coverage");
                    ensure!(characters.residents.len() == 4,
                        "current character owner coverage differs");
                    let cold: super::super::completed_residents::Imported = serde_json::from_slice(&serde_json::to_vec(&characters)?)?;
                    ensure!(cold == characters, "current character named cold roundtrip changed state");
                    ensure!(sims_vm::Snapshot::capture(&callbacks.program, &vm) == saved,
                        "character import changed retained VM/RNG");
                    eprintln!("moving B four Characters/Residents, retained active movement and mesh caches imported; no VM execution");
                    (characters, animation.assets, proof)
                };
                {
                    use super::super::completed_simulation::compose;
                    let raw=std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/private/goth-retained-execution-20261030-first/route-execution-physical-B-first.json"))?;
                    ensure!(sims_fixtures::digest(&raw)=="58efe83c82592e0da4cc64bd9dd10c4c5246eeb72160996de61e039f73478175","current route execution artifact differs");
                    let route_source:serde_json::Value=serde_json::from_slice(&raw)?;
                    let catalogs=std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/private/goth-retained-execution-20261030-first/prototype-catalogs-first.json"))?;
                    ensure!(sims_fixtures::digest(&catalogs)=="9b182f0644af38d5e909370c29048c03cfd76c9804997a332b377afa97b5ec4d","immutable prototype catalog payloads differ");
                    let catalogs:serde_json::Value=serde_json::from_slice(&catalogs)?;
                    let prototypes=super::super::completed_prototypes::import_captured(
                        &state["domains"]["prototypes"],&catalogs,characters.clone(),
                        "goth89073-retained-source-prototypes-v2",vm.tick,&memory,
                    )?;
                    let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../cache/managed-independent-20261022-third/immutable-assets");
                    let index=sims_content::Index::scan(&root)?;
                    ensure!(!index.resources.iter().any(|r|r.locator.split('/').any(|p|p.starts_with("UserData"))),
                        "current immutable avatar index contains starting-save paths");
                    let code=std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/private/goth-retained-assets-20261030-first/appearance-code-physical-B-first.json"))?;
                    ensure!(sims_fixtures::digest(&code)=="0976077eefe572601a0102eea5625679fb58521f64d2a1ce8284b394dac31f0b","current appearance code artifact differs");
                    let code:serde_json::Value=serde_json::from_slice(&code)?;
                    let appearances=super::super::completed_appearances::import_captured(
                        &state["domains"]["residents"],&code,prototypes,
                        &index,vm.tick,&memory,
                    )?;
                    let characters=appearances.characters.characters.clone();
                    eprintln!("moving B immutable avatars and native mesh signatures qualified for {} people; no retained character/VM mutation",appearances.saved.len());
                    let (program,current)=compose::with_captured_clips(callbacks.program.clone(),scored,resident_clips.clone())?;
                    let raw=std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/private/goth-animation-join-20261030-first/private-posture-physical-B-second.json"))?;
                    ensure!(sims_fixtures::digest(&raw)=="e503556956fd834378d0c5bfac8dbbe4351d90230ed34caedd073a91a140e391","current private posture sources differ");
                    let posture_source:serde_json::Value=serde_json::from_slice(&raw)?;
                    let mut wrong=posture_source.clone();wrong["private"][0]["cache_pointer"]=serde_json::json!(1234);
                    ensure!(super::super::completed_animation_cache::captured::join(&state,&memory,&wrong,&skin_proof.observation_sha256,
                        program.clone(),current.clone()).is_err(),"current posture lookup accepted a fabricated private-cache miss");
                    let named=super::super::completed_animation_cache::captured::join(&state,&memory,&posture_source,
                        &skin_proof.observation_sha256,program,current)?;
                    let mut program=named.program;let mut current=named.vm;
                    let mut resident_routine_sources=serde_json::Value::Null;
                    if std::env::var("SIMS_RETAINED_RESIDENT_ROUTINES").as_deref()==Ok("1") {
                        let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/private/goth-route-physical-20261030-first");
                        let raw=std::fs::read(root.join("resident-bhav-B-second.json"))?;
                        ensure!(sims_fixtures::digest(&raw)=="85c351b101932c16df04317c6e89bf72a352b944c5ab4c91169f111c7a4f84ca","resident routine source differs");
                        let proof:serde_json::Value=serde_json::from_slice(&raw)?;
                        let raw=std::fs::read(root.join("resident-bhav-B-second.mem"))?;let supplement=Readset::parse(&raw)?;
                        let before=serde_json::to_vec(&current.clone().restore(&program).map_err(anyhow::Error::msg)?)?;
                        resident_routine_sources=resident_sources::join(&state,&memory,&proof,&supplement,&mut program)?;
                        current.program_hash=program.fingerprint();
                        ensure!(serde_json::to_vec(&current.clone().restore(&program).map_err(anyhow::Error::msg)?)?==before,"resident routine source join changed VM/RNG");
                        eprintln!("moving B resident BHAV closure: {} added, {} already bound, {} remain lazy",resident_routine_sources["added"],resident_routine_sources["already_bound"],resident_routine_sources["lazy_demands"]);
                    }
                    let mut resident_constant_sources=serde_json::Value::Null;
                    if std::env::var("SIMS_RETAINED_RESIDENT_CONSTANTS").as_deref()==Ok("1") {
                        let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/private/goth-route-physical-20261030-first");
                        let raw=std::fs::read(root.join("resident-bcon-B-first.json"))?;
                        ensure!(sims_fixtures::digest(&raw)=="b6c5877142070c1bcd87886b17e8450dab6e5a909d6a9ee78a622175c2b79a17","native BCON source differs");
                        let proof:serde_json::Value=serde_json::from_slice(&raw)?;
                        let raw=std::fs::read(root.join("resident-bcon-B-first.mem"))?;let supplement=Readset::parse(&raw)?;
                        let before=serde_json::to_vec(&current.clone().restore(&program).map_err(anyhow::Error::msg)?)?;
                        resident_constant_sources=resident_constants::join(&state,&memory,&proof,&supplement,&mut program)?;
                        current.program_hash=program.fingerprint();
                        ensure!(serde_json::to_vec(&current.clone().restore(&program).map_err(anyhow::Error::msg)?)?==before,"native BCON join changed VM/RNG");
                        eprintln!("moving B {} native resident BCON tables and {} empty OTF owners bound",resident_constant_sources["tables"],resident_constant_sources["observed_empty_OTF_owners"]);
                    }
                    ensure!(program.modules[&program.globals].strings[&150][20]=="a2o-walking-loop",
                        "current effective walking name was not retained");
                    eprintln!("moving B thirteen effective Global animation tables and four actual private150 misses joined without VM/RNG mutation");
                    let source=serde_json::json!({"resident_input":state["domains"]["residents"]});
                    let joined=compose::import_captured(compose::CapturedInput {
                        vm:current,program:&program,owners:&owners,lot:&lot,
                        environment:&state["domains"]["room_environment"],resident_source:&source,
                        boundary:&skin_proof,residents:characters.clone(),state:&state,memory:&memory,
                        routes:&routes,pending:&pending,route_source:&route_source,
                        epoch:"goth89073-retained-execution-fresh-unpublished-v1",
                    })?;
                    let restored=joined.snapshot.clone().restore(&program).map_err(anyhow::Error::msg)?;
                    ensure!(restored.vm.tick==89073 && restored.route_control.len()==5
                        && restored.characters.values().filter(|r|r.native_walk.is_some()).count()==3,
                        "retained moving Simulation execution owner coverage differs");
                    let cold:sims_sim::Snapshot=serde_json::from_slice(&serde_json::to_vec(&joined.snapshot)?)?;
                    ensure!(cold.restore(&program).map_err(anyhow::Error::msg)?==restored,
                        "retained moving Simulation named restore changed state");
                    let mut unchanged=restored.characters.clone();
                    for resident in unchanged.values_mut() {resident.native_walk=None;}
                    ensure!(unchanged==characters.residents,"route binding changed retained Character/animation/movement state");
                    let mut bad=joined.snapshot.clone();
                    bad.state.characters.get_mut(&21).unwrap().native_walk.as_mut().unwrap()
                        .retained_origin.as_mut().unwrap().completed_tick+=1;
                    ensure!(bad.restore(&program).is_err(),"retained walk accepted a contradictory ownership epoch");
                    // Current compact component writes must include the None
                    // origin slot. Named old checkpoints can omit the field.
                    let mut walk=restored.characters[&21].native_walk.as_ref().unwrap().clone();
                    for retained in [true,false] {
                        if !retained {walk.retained_origin=None;}
                        let bytes=postcard::to_allocvec(&walk)?;
                        let cold:sims_sim::lot_route::animator::Walk=postcard::from_bytes(&bytes)?;
                        ensure!(cold==walk,"native walk compact storage shifted optional owner fields");
                    }
                    if let Ok(output)=std::env::var("SIMS_RETAINED_EXECUTION_OUTPUT") {
                        let path=std::path::Path::new(&output);std::fs::create_dir(path)?;
                        let store=sims_fixtures::Store::create(path.join("store"))?;
                        let reference=store.put_value(&joined.snapshot)?;
                        let cold:sims_sim::Snapshot=store.get_value(&reference)?;
                        ensure!(cold.restore(&program).map_err(anyhow::Error::msg)?==restored,
                            "durable retained moving Simulation changed state");
                        std::fs::write(path.join("refs.json"),serde_json::to_vec_pretty(&serde_json::json!({
                            "schema":1,"tick":89073,"simulation":reference,"program_hash":program.fingerprint(),
                            "native_parity_credit":0,"complete_Session_admitted":false,"source":joined.source
                        }))?)?;
                    }
                    eprintln!("moving B Simulation structurally restored: five route controls, three native walks, retained portal caches; no VM execution or parity credit");
                    if let Ok(output)=std::env::var("SIMS_RETAINED_EXECUTION_PROBE_OUTPUT") {
                        let mut probe=restored;
                        let baseline_fields=super::super::completed_fields::compare(&state,&probe)?;
                        let mut polls=Vec::new();
                        let before=serde_json::json!({"tick":probe.vm.tick,"global":probe.vm.rng_state,"lot":probe.vm.lot_rng_state,"crt":probe.vm.crt_rng_state});
                        let mut presentation=super::super::completed_presentation::import_captured(
                            &state["domains"]["presentation"],&source,vm.tick,&memory)?;
                        let mut assets=appearances.saved.clone();
                        let mut avatars=BTreeMap::new();
                        for (&actor,a)in &assets {
                            let avatar=sims_content::saved_avatar::BoundAvatar::restore(&index,&a.skeleton,actor as u64,&a.binding)?;
                            ensure!(avatar.snapshot()==a.binding,"current avatar restore changed binding identities");
                            avatars.insert(actor,avatar);
                        }
                        let definitions:BTreeMap<_,_>=probe.vm.world.objects.iter()
                            .map(|(&id,o)|Ok((id,o.definition.clone().context("ordinary definition absent")?)))
                            .collect::<Result<_>>()?;
                        let lifetimes:BTreeMap<_,_>=definitions.keys().map(|&id|Ok((id,
                            probe.vm.world.scheduler_storage.as_ref().context("ordinary registry absent")?
                                .handle(id).context("ordinary registry lifetime absent")?))).collect::<Result<_>>()?;
                        let mut asset_index=index.clone();
                        let streams=&state["domains"]["source_streams"];
                        super::super::completed_skeletons::captured::reads(streams,&memory)?;
                        let mut ordinary_files=Vec::new();
                        for actor in state["domains"]["ordinary"]["actors"].as_array().context("ordinary actors absent")? {
                            let id=actor["object"].as_i64().context("ordinary actor id absent")? as i16;
                            let resource=super::super::completed_geometry::bytes(actor,"resource_raw")?;
                            let provider=super::super::completed_geometry::u(&resource,0)?;
                            let components=streams["providers"][provider.to_string()]["components"].as_array().context("ordinary provider components absent")?;
                            let mut selected=Vec::new();
                            for pointer in components {
                                let component=&streams["components"][pointer.as_u64().context("ordinary component absent")?.to_string()];
                                let logical=component["logical_name"]["ascii"].as_str().context("ordinary logical name absent")?.replace('\\',"/").to_ascii_lowercase();
                                if !logical.ends_with(".iff") {continue;}
                                ensure!(component["modified"]==0 && component["writable"]==0 && component["shared_slice"]==1,
                                    "ordinary source requires an explicit mutable overlay");
                                let backend=&streams["backends"][component["backend"].as_u64().context("ordinary backend absent")?.to_string()];
                                let archive=backend["path"]["ascii"].as_str().context("ordinary archive path absent")?.replace('\\',"/").to_ascii_lowercase();
                                let archive=archive.rsplit('/').next().unwrap();
                                let name=logical.rsplit('/').next().unwrap();
                                for r in &index.resources {
                                    if r.name.eq_ignore_ascii_case(name) && r.source.file_name().is_some_and(|p|p.to_string_lossy().eq_ignore_ascii_case(archive))
                                        && Some(r.offset)==component["base_offset"].as_u64() && Some(r.size)==component["logical_size"].as_u64() {
                                        selected.push(r.clone());
                                    }
                                }
                            }
                            ensure!(selected.len()==1,"ordinary native source/file extent is missing or ambiguous for {id}");
                            let mut alias=selected.pop().unwrap();
                            ordinary_files.push(serde_json::json!({"object":id,"source":definitions[&id].source,"locator":alias.locator,"sha256":sims_fixtures::digest(&alias.read()?)}));
                            alias.locator=definitions[&id].source.clone();
                            ensure!(!asset_index.resources.iter().any(|r|r.locator==alias.locator),"ordinary source alias collision");
                            asset_index.resources.push(alias);
                        }
                        let ordinary=super::super::completed_ordinary::import_captured(&state["domains"]["ordinary"],
                            &definitions,&lifetimes,&asset_index,"goth89073-retained-ordinary-v1",&state,&memory)?;
                        let raw=std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/private/goth-retained-assets-20261030-first/ordinary-profiles-physical-B-first.json"))?;
                        ensure!(sims_fixtures::digest(&raw)=="ec63c99f4b473064fd836e812f4210313fc0b00c69eaea63a5a20fbe86191303","current ordinary profiles artifact differs");
                        let profiles:serde_json::Value=serde_json::from_slice(&raw)?;
                        ensure!(super::super::completed_ordinary::import(&state["domains"]["ordinary"],
                            &definitions,&lifetimes,&asset_index,"old-cut-must-reject-current").is_err(),
                            "historical ordinary import admitted the moving cut");
                        let mut wrong_profiles=profiles.clone();wrong_profiles["source_readset_sha256"]=serde_json::json!("0".repeat(64));
                        ensure!(super::super::completed_ordinary::live::import_captured(ordinary.clone(),&definitions,&wrong_profiles,
                            &skin_proof,&state,&memory,&asset_index).is_err(),"ordinary profiles accepted another sparse epoch");
                        let ordinary=super::super::completed_ordinary::live::import_captured(ordinary,&definitions,&profiles,
                            &skin_proof,&state,&memory,&asset_index)?;
                        let heads=super::super::completed_slot_anchors::import_captured_heads(&state,&memory,&probe.vm.world)?;
                        for (&id,head)in &heads {
                            let slots=probe.vm.world.objects.get_mut(&id).unwrap().native_slot_geometry.as_mut().context("retained physical slots absent")?;
                            let slot=slots.get_mut(1).context("retained HEAD slot absent")?;
                            ensure!(slot.anchor.as_ref().is_none_or(|old|old==head),"retained HEAD name contradicts existing source");
                            slot.anchor=Some(head.clone());
                        }
                        if std::env::var("SIMS_RETAINED_DISABLED_SOUND").as_deref()==Ok("1") {
                            let raw=std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/private/goth-disabled-sound-20261030-first/original-disabled-sound-source-first.json"))?;
                            ensure!(sims_fixtures::digest(&raw)=="6290847f79f2c79899a98b00583569412e287f772205b42fb64cdbf6e47af16f","disabled sound source differs");
                            let source:serde_json::Value=serde_json::from_slice(&raw)?;
                            presentation.disabled_sound=Some(super::super::completed_presentation::sound::Disabled::import(&state,&memory,&source)?);
                        }
                        let (ordinary_saved,mut mesh_pool,native_meshes,ordinary_remap,mesh_aliases)=super::super::completed_assets::join_geometry(
                            &assets,&characters.geometry,&ordinary,&asset_index)?;
                        let mut ordinary=sims_content::graphic_loading::Registry::restore(&asset_index,ordinary_saved)?;
                        let mut dependent_jobs=BTreeMap::new();
                        eprintln!("moving B two retained ordinary ROOT models, source-matched current caches and initialized HEAD slots qualified; shared mesh aliases {mesh_aliases}");
                        let floors=if std::env::var("SIMS_RETAINED_FLOOR_SERVICES").as_deref()==Ok("1") {
                            let lot=probe.architecture.as_ref().and_then(|a|a.original.as_ref()).context("retained floor architecture absent")?;
                            let floors=sims_content::character_loading::floors::from_builtin_coverings(&asset_index,lot.floors.iter().flat_map(|f|f.source.coverings.iter().copied()))?;
                            sims_content::character_loading::floors::restore(&asset_index,&floors)?;Some(floors)
                        }else{None};
                        if let Ok(path)=std::env::var("SIMS_RETAINED_SCENE_PREFIX_OUTPUT") {
                            super::super::completed_replay::retained::verification::scene_prefix(&asset_index,&program,std::path::Path::new(&path))?;return Ok(());
                        }
                        if let Ok(path)=std::env::var("SIMS_RETAINED_COLD_REPORT") {
                            let destination=std::env::var("SIMS_RETAINED_COLD_OUTPUT")?;
                            super::super::completed_replay::retained::verification::run(
                                &asset_index,&program,std::path::Path::new(&path),std::path::Path::new(&destination))?;
                            return Ok(());
                        }
                        let store=sims_fixtures::Store::create(std::path::Path::new(&output).with_extension("store"))?;
                        let record_cut=|probe:&sims_sim::Simulation,assets:&BTreeMap<sims_vm::ObjectId,sims_content::character_loading::live::SavedAssets>,ordinary:&sims_content::graphic_loading::Registry,mesh_pool:&sims_content::character_geometry::MeshPool,dependent_jobs:&BTreeMap<sims_vm::ObjectId,sims_content::character_loading::live::dependent::Job>,presentation:&super::super::completed_presentation::Bindings| {
                            super::super::completed_assets::checkpoints::Retained::put(&store,&asset_index,
                                super::super::completed_assets::checkpoints::Owners{program:&program,simulation:probe,characters:assets,ordinary,meshes:mesh_pool,native_meshes:&native_meshes,dependent:dependent_jobs,presentation,floors:&floors})
                        };
                        let baseline_cut=if std::env::var("SIMS_RETAINED_STORE_BOUNDARIES").as_deref()==Ok("1") {
                            Some(record_cut(&probe,&assets,&ordinary,&mesh_pool,&dependent_jobs,&presentation)?)
                        }else{None};
                        let mut slices=Vec::new();let mut replies=Vec::new();let mut ordered_trace=Vec::new();
                        let count=std::env::var("SIMS_RETAINED_COMPLETED_BOUNDARIES").ok().map(|n|n.parse::<usize>()).transpose()?.unwrap_or(1);
                        ensure!((1..=32).contains(&count)&&(count==1||std::env::var("SIMS_RETAINED_FINISH_PASS").as_deref()==Ok("1")),"retained diagnostic boundary count invalid");
                        let native_root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/private/goth-completed-continuation-20261030-third/results/native-sequence");
                        let seal_raw=std::fs::read(native_root.join("capture-result.json"))?;
                        ensure!(sims_fixtures::digest(&seal_raw)=="4139107d1965925bf3ee5f44fa524d5d4b1fae22bfdb617edfffb69d33bcd393","original completed-sequence seal differs");
                        let seal:serde_json::Value=serde_json::from_slice(&seal_raw)?;
                        let trace_raw=std::fs::read(native_root.join("native-bytecode.json"))?;
                        ensure!(sims_fixtures::digest(&trace_raw)=="7430b5bcddc06cbc4a5be8ab0708da56f18bce00ef3a4bb9bc14a20552f05c44","native dispatch evidence differs");
                        let native_trace:Vec<serde_json::Value>=serde_json::from_slice(&trace_raw)?;
                        let mut result=serde_json::json!({"status":"not_run"});
                        let mut before_finish=sims_sim::Snapshot::capture(&program,&probe);
                        let mut boundary_fields=serde_json::Value::Null;
                        let mut ordered_comparison=serde_json::Value::Null;
                        let mut boundary_source_sha=serde_json::Value::Null;
                        let mut boundary_sequence=Vec::new();
                        // Explicit research composition of the independently
                        // witnessed C8 frontend phase. This is not admission
                        // of a complete renderer or enclosing Session caller.
                        let draw_people=if std::env::var("SIMS_RETAINED_C8_CPU_PUBLICATION").as_deref()==Ok("1") {
                            let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
                            let raw=std::fs::read(root.join("fixtures/private/goth-draw-order-20261031-first/verified-draw-third.json"))?;
                            ensure!(sims_fixtures::digest(&raw)=="b1d620383ed52dcf0227cad69268ef4fef11837ce4ba2cdbf71927eba67c82bc","native draw verification differs");
                            let proof:serde_json::Value=serde_json::from_slice(&raw)?;
                            ensure!(proof["complete_simulation_unchanged"]==true&&proof["native_parity_credit"]==0
                                &&proof["native_actor_order"]==serde_json::json!([21,383,383,18,16,259]),"native C8 publication profile differs");
                            let raw=std::fs::read(root.join("fixtures/private/goth-draw-witness-20261031-sixth/draw-trace.jsonl.gz"))?;
                            ensure!(sims_fixtures::digest(&raw)=="59bd84e71302a7ba7ac5689ad2ca767b4b79496b0b97ebe5927dbdffd30212d6","actual native draw witness differs");
                            Some([21_i16,18,16,259])
                        }else{None};
                        let acknowledge_cpu=std::env::var("SIMS_RETAINED_C8_CPU_ACK").as_deref()==Ok("1");
                        if acknowledge_cpu {
                            ensure!(draw_people.is_some(),"native upload acknowledgement requires the verified C8 publication phase");
                            let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
                            let raw=std::fs::read(root.join("fixtures/private/goth-draw-replay-20261031-first/verified-draw-fourth.json"))?;
                            ensure!(sims_fixtures::digest(&raw)=="8c40f4d348e63590aa2bd00a1a99cb4358d28d5d57e670b1964192ca3ea56eef","native acknowledged-publication proof differs");
                            let proof:serde_json::Value=serde_json::from_slice(&raw)?;
                            ensure!(proof["renderer_notifications_acknowledged"]==true&&proof["complete_simulation_unchanged"]==true,"native upload profile absent");
                        }
                        let mut cpu_publications=Vec::new();
                        let mut previous_stop=state["caller_trace_stop"].as_u64().context("baseline completed stop absent")?;
                        for boundary in 1..=count {
                            if boundary==9 {
                                if let Some(people)=draw_people {
                                    ensure!(probe.vm.tick==89081,"C8 frontend publication crossed another tick");
                                    let before=sims_sim::Snapshot::capture(&program,&probe);
                                    for actor in people {
                                        let a=assets.get_mut(&actor).context("C8 native draw asset owner absent")?;
                                        let avatar=sims_content::saved_avatar::BoundAvatar::restore(&asset_index,&a.skeleton,actor as u64,&a.binding)?;
                                        let resident=probe.characters.get_mut(&actor).context("C8 native draw resident absent")?;
                                        let control=resident.character.skin.animator.fp_control();
                                        ensure!(control==0x003f,"C8 native draw precision differs");
                                        let uploads=a.model.with_bound(resident,|bound|a.geometry.update_loaded_vertices_with_control(&avatar,bound,false,control))?;
                                        if acknowledge_cpu {
                                            for &binding in &uploads {
                                                a.geometry.vertices.get_mut(&binding).context("native notified buffer missing")?
                                                    .acknowledge_null_bounds_upload(false)?;
                                            }
                                        }
                                        cpu_publications.push(serde_json::json!({"actor":actor,"after_tick":89081,"before_boundary":boundary,"control":control,"uploads":uploads,"notifications_acknowledged":acknowledge_cpu}));
                                    }
                                    ensure!(sims_sim::Snapshot::capture(&program,&probe)==before,"C8 publication changed simulation/VM/RNG/continuations");
                                    eprintln!("actual-witness C8 character CPU publication composed; complete simulation unchanged");
                                }
                            }
                            let trace_start=ordered_trace.len();let slice_start=slices.len();let tick=probe.vm.tick;
                        for _ in 0..256 {
                            let poll_start=polls.len();
                            let step=probe.run_current_tick_with_host_polls(&program,100_000,&mut polls);
                            let stopped=match step {
                                Ok(step)=>{
                                    for index in 0..=step.trace.len() {
                                        for poll in &polls[poll_start..] {
                                            if poll.before_instruction==index {ordered_trace.push(poll.instruction.clone());}
                                        }
                                        if let Some(instruction)=step.trace.get(index) {ordered_trace.push(instruction.clone());}
                                    }
                                    let done=step.stop!=sims_sim::StepStop::Budget;
                                    result=serde_json::json!({"status":"returned_slice","stop":format!("{:?}",step.stop),"trace":step.trace,"events":step.events});
                                    done
                                }
                                Err(error)=>{result=serde_json::json!({"status":"error","error":error});true}
                            };
                            slices.push(result.clone());
                            if stopped {break;}
                            let pending=probe.pending_character_services();
                            let mut completed=false;
                            for (actor,request)in pending {
                                use sims_sim::pose::refresh::{Request,Response};
                                if request==Request::OrdinaryCarriedObject {
                                    let child=probe.vm.world.objects[&actor].slots[1].context("dependent child absent")?;
                                    let outside=!probe.native_lot.as_ref().context("dependent lot absent")?.placement.links
                                        .capture_raw(&probe.vm.world,child).map_err(anyhow::Error::msg)?.on_lot;
                                    let view=sims_content::character_loading::live::graphic::View {
                                        viewport:presentation.viewport,outside,visibility_enabled:presentation.visibility.enabled,
                                        camera:presentation.visibility.camera,
                                        materials:sims_content::graphic_materials::Backend{mode:0,flags:0,references:1,ramp_size:32},normal_material_mode:0,
                                    };
                                    let mut owners=sims_content::character_loading::live::dependent::Owners {
                                        index:&asset_index,ordinary:&mut ordinary,mesh_pool:&mut mesh_pool,jobs:&mut dependent_jobs,
                                    };
                                    match owners.service_dependent(&mut probe,&program,actor,view) {
                                        Ok(())=>{completed=true;replies.push(serde_json::json!({"actor":actor,"request":request,"response":"production retained dependent service"}));}
                                        Err(error)=>{result=serde_json::json!({"status":"dependent_error","error":format!("{error:#}")});}
                                    }
                                    continue;
                                }
                                let a=assets.get_mut(&actor).context("retained geometry asset owner absent")?;
                                if let Some(uploads)=a.geometry.service_resident(&mut a.model,
                                    probe.characters.get_mut(&actor).context("retained geometry Character absent")?,
                                    &avatars[&actor],&mut mesh_pool)? {
                                    replies.push(serde_json::json!({"actor":actor,"request":request,"response":"production retained geometry service","renderer_uploads":uploads}));
                                    completed=true;
                                    continue;
                                }
                                let reply=match request {
                                    Request::Property(sims_sim::pose::properties::Request::FootstepSurface{..}) if floors.is_some()=> {
                                        Response::PropertySurface(probe.original_footstep_surface(actor,&sims_content::character_loading::floors::categories(floors.as_ref().unwrap())).map_err(anyhow::Error::msg)?)
                                    },
                                    Request::Property(ref property@sims_sim::pose::properties::Request::Sound{..}) if presentation.disabled_sound.is_some()=> {
                                        use sims_content::character_loading::Presentation;
                                        presentation.property(actor,&property)?
                                    },
                                    Request::Viewport=>Response::Viewport(presentation.viewport),
                                    Request::Visibility{viewport}=>Response::Visible(presentation.visibility.visible(
                                        &probe.characters[&actor],&a.model,viewport)?),
                                    _=>continue,
                                };
                                replies.push(serde_json::json!({"actor":actor,"request":request,"response":format!("{reply:?}")}));
                                probe.complete_character_service(actor,&request,reply).map_err(anyhow::Error::msg)?;
                                completed=true;
                            }
                            if !completed {break;}
                        }

                            before_finish=sims_sim::Snapshot::capture(&program,&probe);
                            if std::env::var("SIMS_RETAINED_FINISH_PASS").as_deref()!=Ok("1") {break;}
                            let clean=result["stop"]=="Waiting"&&result["status"]=="returned_slice"
                                &&probe.pending_character_services().is_empty()&&probe.pending_person_services().is_empty()
                                &&probe.pending_character_updates().is_empty()
                                &&slices[slice_start..].iter().all(|s|s["events"].as_array().is_some_and(Vec::is_empty));
                            if !clean {
                                boundary_sequence.push(serde_json::json!({"boundary":boundary,"tick_before":tick,"status":"unfinished_or_unsupported","result":result}));break;
                            }
                            ensure!(probe.finish_registry_pass().map_err(anyhow::Error::msg)?,"retained pass did not complete game time");
                            // Drawing and its environment rebuild are separate caller work.
                            let encoded=std::fs::read(native_root.join(format!("completed-{boundary:02}/observation.json.gz")))?;
                            let hash=sims_fixtures::digest(&encoded);
                            ensure!(seal["observations"][boundary]["sha256"]==hash,"native completed boundary source differs");
                            let next:serde_json::Value=serde_json::from_reader(flate2::read::GzDecoder::new(&encoded[..]))?;
                            if let Some(sound)=&presentation.disabled_sound {
                                let raw=std::fs::read(native_root.join(format!("completed-{boundary:02}/acknowledged.mem")))?;
                                sound.validate_current(&next,&Readset::parse(&raw)?)?;
                            }
                            boundary_fields=super::super::completed_fields::compare(&next,&probe)?;
                            let high=next["caller_trace_stop"].as_u64().context("next completed stop absent")?;
                            let selected:Vec<_>=native_trace.iter().filter(|r|r["stop_index"].as_u64().is_some_and(|n|n>previous_stop&&n<high)).cloned().collect();
                            ordered_comparison=match super::super::completed_replay::trace::compare_dispatches(&serde_json::json!(selected),&ordered_trace[trace_start..],tick) {
                                Ok(report)=>report,
                                Err(error)=>serde_json::json!({"status":"different_or_unsupported","error":format!("{error:#}")}),
                            };
                            boundary_source_sha=serde_json::json!(hash);
                            boundary_sequence.push(serde_json::json!({"boundary":boundary,"tick":probe.vm.tick,
                                "source_sha256":hash,"fields":boundary_fields,"dispatches":ordered_comparison}));
                            eprintln!("retained diagnostic boundary {boundary}: tick {}, field differences {}, dispatches {}",probe.vm.tick,boundary_fields["differences"],ordered_comparison["instructions"]);
                            // Source installation is checked before execution.
                            // A historical Rust C1 is not an engine golden:
                            // native-backed arithmetic fixes can change it.
                            // Compare current-run source variants separately
                            // and check the original's pinned boundary fields.
                            if baseline_cut.is_some() {
                                let cut=record_cut(&probe,&assets,&ordinary,&mesh_pool,&dependent_jobs,&presentation)?;
                                boundary_sequence.last_mut().unwrap()["retained_cut"]=serde_json::to_value(cut)?;
                            }
                            previous_stop=high;
                            if boundary_fields["differences"]!=0||ordered_comparison.get("error").is_some(){break;}
                        }
                        let pending=probe.pending_character_services();
                        let cut=sims_sim::Snapshot::capture(&program,&probe);
                        ensure!(cut.clone().restore(&program).map_err(anyhow::Error::msg)?==probe,
                            "first moving Rust service cut cannot cold restore");

                        let presentation_ref=store.put_value(&presentation)?;
                        let cold:super::super::completed_presentation::Bindings=store.get_value(&presentation_ref)?;
                        ensure!(cold==presentation,"durable presentation owner changed");
                        let cut_ref=store.put_value(&cut)?;
                        let before_finish_ref=store.put_value(&before_finish)?;
                        let cold:sims_sim::Snapshot=store.get_value(&cut_ref)?;
                        ensure!(cold.restore(&program).map_err(anyhow::Error::msg)?==probe,
                            "durable moving Rust service cut changed state/RNG/continuations");
                        let asset_ref=store.put_value(&assets)?;
                        let cold:BTreeMap<sims_vm::ObjectId,sims_content::character_loading::live::SavedAssets>=store.get_value(&asset_ref)?;
                        ensure!(cold==assets,"current geometry asset cut changed during fixture restore");
                        for (&actor,a)in &cold {
                            let avatar=sims_content::saved_avatar::BoundAvatar::restore(&index,&a.skeleton,actor as u64,&a.binding)?;
                            ensure!(avatar.snapshot()==a.binding,"cold geometry assets changed immutable avatar bindings");
                        }
                        let mesh_ref=store.put_value(&mesh_pool)?;
                        let cold:sims_content::character_geometry::MeshPool=store.get_value(&mesh_ref)?;
                        ensure!(cold==mesh_pool,"current mesh cache cut changed during fixture restore");
                        let ordinary_cut=ordinary.snapshot();
                        let ordinary_ref=store.put_value(&ordinary_cut)?;
                        let cold:sims_content::graphic_loading::Saved=store.get_value(&ordinary_ref)?;
                        ensure!(sims_content::graphic_loading::Registry::restore(&asset_index,cold)?.snapshot()==ordinary_cut,
                            "durable ordinary service cut changed owners");
                        let jobs_ref=store.put_value(&dependent_jobs)?;
                        let cold:BTreeMap<sims_vm::ObjectId,sims_content::character_loading::live::dependent::Job>=store.get_value(&jobs_ref)?;
                        ensure!(cold==dependent_jobs,"durable dependent service cut changed continuations");
                        let owners=sims_content::character_loading::live::dependent::Owners {
                            index:&asset_index,ordinary:&mut ordinary,mesh_pool:&mut mesh_pool,jobs:&mut dependent_jobs,
                        };
                        owners.validate_dependent(&probe)?;
                        let mut report=serde_json::json!({"schema":1,"kind":"retained_moving_registry_boundaries_diagnostic","native_parity_credit":0,
                            "cold_ordinary":ordinary_ref,"cold_dependent_jobs":jobs_ref,"ordinary_files":ordinary_files,"animation_name_sources":named.source,
                            "native_meshes":native_meshes,"ordinary_mesh_remap":ordinary_remap,"mesh_aliases":mesh_aliases,
                            "material_backend":"production defaults; not independently qualified for this cut",
                            "cold_rust_cut":cut_ref,"cold_before_finish":before_finish_ref,"route_observer_prior_verified_cut":"2991d9010e8660b88781dddf4292cef1b818da12740df63a168013d5d56233d6","ordered_trace":ordered_trace,"ordered_comparison":ordered_comparison,"cold_character_assets":asset_ref,"cold_mesh_pool":mesh_ref,
                            "immutable_avatar_sources":appearances.immutable_sources,"rust_cut_roundtrip":"passed","original_cut_comparison":"not performed",
                            "baseline_fields":baseline_fields,"boundary_fields":boundary_fields,"boundary_source_sha256":boundary_source_sha,
                            "before":before,"result":result,"slices":slices,"checked_replies":replies,"host_polls":polls.iter().map(|p|serde_json::json!({"before_instruction":p.before_instruction,"instruction":p.instruction})).collect::<Vec<_>>(),"pending_character_services":pending,
                            "pending_person_services":format!("{:?}",probe.pending_person_services()),"pending_character_updates":format!("{:?}",probe.pending_character_updates()),
                            "after":{"tick":probe.vm.tick,"global":probe.vm.rng_state,"lot":probe.vm.lot_rng_state,"crt":probe.vm.crt_rng_state},
                            "faults":probe.vm.threads.iter().filter_map(|(&id,t)|if let sims_vm::Status::Faulted(e)=&t.status {Some((id,e))}else{None}).collect::<Vec<_>>()});
                        report["baseline_retained_cut"]=serde_json::to_value(baseline_cut)?;
                        report["cold_presentation"]=serde_json::to_value(presentation_ref)?;
                        report["footstep_floor_sources"]=serde_json::to_value(&floors)?;
                        report["resident_constant_sources"]=resident_constant_sources;
                        report["resident_routine_sources"]=resident_routine_sources;
                        report["boundary_sequence"]=serde_json::to_value(&boundary_sequence)?;
                        report["requested_boundaries"]=serde_json::json!(count);
                        report["character_cpu_publications"]=serde_json::json!({"events":cpu_publications,
                            "scope":"Explicit diagnostic composition of verified C8 character publication only. Indicator intermediate passes and the complete frontend/clock caller are not executed or admitted."});
                        std::fs::write(output,serde_json::to_vec_pretty(&report)?)?;
                        eprintln!("retained moving diagnostic status {}; stop {}; tick {}; scalar differences {}",report["result"]["status"],report["result"]["stop"],report["after"]["tick"],report["boundary_fields"]["differences"]);
                    }
                }
                if let Ok(output) = std::env::var("SIMS_RETAINED_VM_OUTPUT") {
                    let output = std::path::Path::new(&output);
                    std::fs::create_dir(output)?;
                    let store = sims_fixtures::Store::create(output.join("model-store"))?;
                    let reference = store.put_value(&saved)?;
                    let cold: sims_vm::Snapshot = store.get_value(&reference)?;
                    ensure!(
                        cold.restore(&callbacks.program)
                            .map_err(anyhow::Error::msg)?
                            == vm,
                        "durable retained VM cold restore changed owned state"
                    );
                    let characters_reference = store.put_value(&characters)?;
                    let cold: super::super::completed_residents::Imported = store.get_value(&characters_reference)?;
                    ensure!(cold == characters, "durable current character cold restore changed state");
                    let clips_reference = store.put_value(&state["domains"]["animations"])?;
                    let cold: serde_json::Value = store.get_value(&clips_reference)?;
                    let cold = super::super::completed_animations::import_with_resident_lists(
                        &serde_json::from_value::<Vec<super::super::completed_animations::AssetInput>>(cold["payload"]["assets"].clone())?,
                        &serde_json::from_value::<Vec<super::super::completed_animations::CharacterInput>>(cold["characters"].clone())?,
                        "5ec18620f4412251ffe0f78a29b68c64aff0f609893a998a67d2ba455a359299",
                        &cold["payload"]["resident_lists"], &state["characters"]["event_cursors"], &memory,
                    )?;
                    ensure!(serde_json::to_vec(&cold.assets)? == serde_json::to_vec(&resident_clips)?,
                        "durable current clip cold restore changed resident samples");
                    std::fs::write(
                        output.join("model-refs.json"),
                        serde_json::to_vec_pretty(&serde_json::json!({
                        "schema":1,"tick":vm.tick,"vm":reference,"characters":characters_reference,"resident_clip_transport":clips_reference,"original_animation_file_identity_claimed":false,"program_hash":callbacks.program.fingerprint(),
                        "source_readset_sha256":memory.source_sha256(),"native_parity_credit":0,"complete_Session_admitted":false}))?,
                    )?;
                }
                eprintln!(
                    "retained B VM structurally admitted/restored: {} threads, {} frames; no VM instructions or parity credit",
                    vm.threads.len(),
                    799
                );
            }
        }
        let mut damaged = state.clone();
        let index = damaged["domains"]["objects"]["objects"]
            .as_array()
            .unwrap()
            .iter()
            .position(|v| v["object"] == 21)
            .unwrap();
        let location = &mut damaged["domains"]["objects"]["objects"][index]["person"]["accessory_records"]
            [0]["bound_owner"];
        let before = std::mem::replace(location, serde_json::json!(0));
        ensure!(
            super::super::completed_object_storage::accessories::import(&damaged, &memory).is_err(),
            "unloaded accessory owner became a loaded Program entry"
        );
        damaged["domains"]["objects"]["objects"][index]["person"]["accessory_records"][0]["bound_owner"] =
            before;
        damaged["readset"]["sha256"] = serde_json::json!("0".repeat(64));
        ensure!(
            super::super::completed_object_storage::accessories::import(&damaged, &memory).is_err(),
            "cross-epoch accessory owner accepted"
        );
        let mut leaf_counts = BTreeMap::<u16, usize>::new();
        for object in invocations.objects.values() {
            if let Some(frame) = object.stack.as_ref().and_then(|s| s.frames().last()) {
                let (_, routine) = invocations
                    .program
                    .resolve(&frame.code_owner, frame.location.bhav)
                    .map_err(anyhow::Error::msg)?;
                *leaf_counts
                    .entry(routine.body.instructions[frame.location.instruction].opcode)
                    .or_default() += 1;
            }
        }
        eprintln!("moving B retained leaf opcode counts {leaf_counts:?}");
        eprintln!(
            "moving B World: {} objects, {} people, {} live actions, {} retained groups; no VM execution or parity credit",
            joined.world.objects.len(),
            joined.world.person.as_ref().unwrap().person_order.len(),
            joined
                .world
                .actions
                .as_ref()
                .unwrap()
                .actors
                .values()
                .filter(|a| a.active.is_some())
                .count(),
            joined
                .world
                .objects
                .values()
                .filter(|o| o.multitile.is_some())
                .count()
        );
        Ok(())
    }
    fn identities(state: &serde_json::Value) -> Result<Vec<Identity>> {
        state["snapshot"]["objects"]
            .as_array()
            .context("native objects")?
            .iter()
            .map(|o| {
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
            .collect()
    }
    #[test]
    #[ignore = "private original B/B+32 raw invocation and resident-BHAV source; no executable Thread or parity credit"]
    fn original_full_source_cuts_keep_complete_raw_arenas_routes_and_nested_continuations()
    -> Result<()> {
        for index in [0, 32] {
            let (state, raw) = original(index)?;
            let memory = Readset::parse(&raw)?;
            let identities = identities(&state)?;
            let actions = completed_actions::import(&identities, &memory)?;
            let mut result = import(&identities, &actions.uses, &memory)
                .with_context(|| format!("original invocation cut {index}"))?;
            let routes = super::super::completed_routes::import(&identities, &result, &memory)
                .with_context(|| format!("original route cut {index}"))?;
            use super::super::completed_route_controls::{self, Delivery};
            let mut delivery = Vec::new();
            let mut next = 1;
            for (&actor, person) in &routes {
                let owner = &result.objects[&actor];
                let depth = owner.stack.as_ref().unwrap().frames().len();
                for c in &person.contexts {
                    delivery.push(Delivery {
                        actor,
                        frame_pointer: c.frame_pointer,
                        request: next,
                    });
                    next += 1;
                }
                if !person.contexts.iter().any(|c| c.frame_depth == depth) {
                    delivery.push(Delivery {
                        actor,
                        frame_pointer: *owner.frame_pointers.last().unwrap(),
                        request: next,
                    });
                    next += 1;
                }
            }
            // These are explicit fresh replacement IDs, not observed native
            // request counters. No ID allocation or instruction is executed.
            let pending = completed_route_controls::import(&result, &routes, &delivery)?;
            ensure!(
                delivery.len() == 6 && pending.values().map(|p| p.routes.len()).sum::<usize>() == 5,
                "retained request ownership extent differs"
            );
            for (actor, depth, callee) in [(16, 7, 87), (18, 5, 203)] {
                let service = pending[&actor]
                    .service
                    .as_ref()
                    .context("retained portal service missing")?;
                ensure!(
                    service.base_depth == depth
                        && service.callee == callee
                        && service.bhav == 4105
                        && service.result.is_none(),
                    "native pending portal service changed"
                );
            }
            ensure!(
                matches!(pending[&18].leaf, sims_vm::Status::AwaitingAnimation(_))
                    && pending[&21].service.is_none()
                    && pending[&259].service.is_none(),
                "retained native leaf/service profile differs"
            );
            for value in pending.values() {
                let saved = postcard::to_allocvec(value)?;
                let restored: completed_route_controls::Pending = postcard::from_bytes(&saved)?;
                ensure!(
                    postcard::to_allocvec(&restored)? == saved,
                    "pending request/service serialization changed"
                );
            }
            let mut duplicate = delivery.clone();
            duplicate[1].request = duplicate[0].request;
            ensure!(
                completed_route_controls::import(&result, &routes, &duplicate).is_err(),
                "duplicate replacement request accepted"
            );
            ensure!(
                completed_route_controls::import(&result, &routes, &delivery[..delivery.len() - 1])
                    .is_err(),
                "missing retained delivery synthesized"
            );
            let mut altered = routes.clone();
            altered.get_mut(&16).unwrap().contexts[0].descriptor.0[7] ^= 0x100;
            ensure!(
                completed_route_controls::import(&result, &altered, &delivery).is_err(),
                "retained descriptor was silently repaired"
            );
            let mut exhausted = delivery.clone();
            exhausted[0].request = u64::MAX;
            ensure!(
                completed_route_controls::import(&result, &routes, &exhausted).is_err(),
                "replacement request with no possible successor accepted"
            );
            let mut mixed = routes.clone();
            mixed.get_mut(&16).unwrap().readset_sha256 = "0".repeat(64);
            ensure!(
                completed_route_controls::import(&result, &mixed, &delivery).is_err(),
                "cross-epoch request/service owner accepted"
            );
            let binding = result.objects[&87].binding.definition.clone();
            let original_kind = result
                .program
                .modules
                .get_mut(&binding.source)
                .unwrap()
                .object_definition_words
                .get_mut(&binding.object)
                .unwrap()[9];
            result
                .program
                .modules
                .get_mut(&binding.source)
                .unwrap()
                .object_definition_words
                .get_mut(&binding.object)
                .unwrap()[9] = 4;
            let rejected = completed_route_controls::import(&result, &routes, &delivery).is_err();
            result
                .program
                .modules
                .get_mut(&binding.source)
                .unwrap()
                .object_definition_words
                .get_mut(&binding.object)
                .unwrap()[9] = original_kind;
            ensure!(rejected, "non-portal state2 service accepted");
            eprintln!(
                "native request cut {index}: 5 routes, 6 fresh unpublished IDs, 2 entered portal services; no VM execution or parity credit"
            );
            let native_routes = state["domains"]["person_routes"]["actors"]
                .as_array()
                .context("native person routes")?;
            ensure!(
                routes.len() == native_routes.len(),
                "native route person coverage differs"
            );
            for native in native_routes {
                let id: i16 = native["object"]
                    .as_i64()
                    .context("native route actor")?
                    .try_into()?;
                let route = &routes[&id];
                ensure!(
                    route.path_raw
                        == super::super::completed_animations::bytes(
                            native["path_raw"].as_str().context("native path raw")?
                        )?,
                    "native retained path bytes changed"
                );
                let rows = native["contexts"]
                    .as_array()
                    .context("native context rows")?;
                ensure!(
                    route.contexts.len() == rows.len(),
                    "native context count changed"
                );
                for (context, native) in route.contexts.iter().zip(rows) {
                    ensure!(
                        context.pointer as u64
                            == native["pointer"].as_u64().context("context pointer")?
                            && context.raw
                                == super::super::completed_animations::bytes(
                                    native["raw"].as_str().context("context raw")?
                                )?
                            && context.candidates_raw
                                == super::super::completed_animations::bytes(
                                    native["candidates_raw"].as_str().context("candidate raw")?
                                )?,
                        "native context/candidate opaque bytes changed"
                    );
                }
                let saved = postcard::to_allocvec(&route.movement)?;
                let restored: sims_sim::pose::movement::Movement = postcard::from_bytes(&saved)?;
                ensure!(
                    postcard::to_allocvec(&restored)? == saved,
                    "retained native movement serialization changed bits"
                );
            }
            let native = state["threads"]["objects"]
                .as_array()
                .context("native threads")?;
            ensure!(
                result.objects.len() == native.len(),
                "raw native stack object coverage differs"
            );
            let (mut frames, mut suspended) = (0, 0);
            for o in native {
                let id: i16 = o["id"].as_i64().context("native ID")?.try_into()?;
                let r = &result.objects[&id];
                let raw = super::super::completed_animations::bytes(
                    o["arena"]["raw"].as_str().context("native arena raw")?,
                )?;
                ensure!(
                    raw == r.raw_arena,
                    "native unused/padding arena bytes changed"
                );
                let expected = o["frames"].as_array().context("native frames")?;
                if let Some(stack) = &r.stack {
                    ensure!(
                        stack.raw_bytes() == raw && stack.frames().len() == expected.len(),
                        "retained native raw stack differs"
                    );
                    for (f, n) in stack.frames().iter().zip(expected) {
                        ensure!(
                            f.continuation as u64
                                == n["continuation_word_raw"]
                                    .as_u64()
                                    .context("native continuation")?
                                && f.location.bhav as u64
                                    == n["tree_raw"].as_u64().context("native tree")?
                                && f.location.instruction as u64
                                    == n["node_raw"].as_u64().context("native node")?,
                            "native active frame was normalized/reset"
                        );
                        suspended += usize::from(f.continuation != 0);
                        frames += 1;
                    }
                } else {
                    ensure!(expected.is_empty(), "invented empty native stack");
                }
            }
            eprintln!(
                "native invocation cut {index}: {} objects, {frames} active frames, {suspended} nonzero continuations, {} current resident routines; parity credit 0",
                result.objects.len(),
                result.routines.len()
            );
        }
        Ok(())
    }
    #[test]
    #[ignore = "middle-cut source closure deliberately not captured; preserve the original demand"]
    fn original_middle_cut_does_not_borrow_unobserved_resource_maps_from_baseline() -> Result<()> {
        let (state, raw) = original(1)?;
        ensure!(
            state["domains"].get("source_cached_slots").is_none()
                && state["domains"].get("source_residents").is_none(),
            "middle-cut source capture scope changed"
        );
        let memory = Readset::parse(&raw)?;
        let ids = identities(&state)?;
        let actions = completed_actions::import(&ids, &memory)?;
        let error = import(&ids, &actions.uses, &memory)
            .err()
            .context("uncaptured middle-cut resource maps got supplied implicitly")?;
        ensure!(
            error
                .to_string()
                .contains("observed native read 0x08851f10+0x304"),
            "middle-cut demand changed: {error:#}"
        );
        Ok(())
    }
    #[test]
    #[ignore = "private retained moving execution cut; no original guest execution"]
    fn retained_movement_foreground_cut_diagnostic() -> Result<()> {
        let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let output=root.join("fixtures/private/goth-retained-assets-20261030-first");
        let report:serde_json::Value=serde_json::from_slice(&std::fs::read(output.join("dependent-probe-second.json"))?)?;
        let store=sims_fixtures::Store::open(output.join("dependent-probe-second.store"))?;
        let reference=serde_json::from_value::<sims_fixtures::BlobRef>(report["cold_rust_cut"].clone())?;
        ensure!(reference.sha256=="5def8134f4e537b0227792e6f35960be3771bc3d144690fc2823d2e3be34ca1f","retained movement cut differs");
        let current:sims_sim::Snapshot=store.get_value(&reference)?;
        let path=root.join("fixtures/private/goth-retained-execution-20261030-first/assembled-simulation-final");
        let refs:serde_json::Value=serde_json::from_slice(&std::fs::read(path.join("refs.json"))?)?;
        let before:sims_sim::Snapshot=sims_fixtures::Store::open(path.join("store"))?
            .get_value(&serde_json::from_value::<sims_fixtures::BlobRef>(refs["simulation"].clone())?)?;
        let projection=|simulation:&sims_sim::Simulation| simulation.characters.iter().map(|(&id,r)|{
            serde_json::json!({"object":id,"mode":r.controls.ordinary.mode,
                "slots":r.character.skin.animator.destruction_layers(),
                "movement":r.controls.movement,"ordinary_update":r.ordinary_update,
                "person_update":r.person_update,"native_walk":r.native_walk})
        }).collect::<Vec<_>>();
        let report=serde_json::json!({"schema":1,"native_parity_credit":0,
            "before":projection(&before.state),"after":projection(&current.state)});
        ensure!(before.state.characters[&259].controls.ordinary.mode==4
            && before.state.characters[&259].character.skin.animator.slot(sims_sim::pose::animator::Slot::Foreground).is_some()
            && current.state.characters[&259].controls.ordinary.mode==9
            && current.state.characters[&259].character.skin.animator.slot(sims_sim::pose::animator::Slot::Foreground).is_none(),
            "historical diagnostic no longer isolates the retained movement selection cut");
        if let Ok(path)=std::env::var("SIMS_RETAINED_MOVEMENT_DIAGNOSTIC_OUTPUT") {
            use std::io::Write;
            let mut file=std::fs::OpenOptions::new().write(true).create_new(true).open(path)?;
            file.write_all(&serde_json::to_vec_pretty(&report)?)?;
        }
        Ok(())
    }

    #[test]
    #[ignore = "actual current C1 animation state versus retained Rust endpoint"]
    fn retained_first_boundary_animator_comparison() -> Result<()> {
        let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let output=root.join("fixtures/private/goth-animation-join-20261030-first");
        let report_name=std::env::var("SIMS_RETAINED_ANIMATOR_REPORT").unwrap_or_else(|_|"fpu-boundary-second.json".into());
        let report_path=output.join(report_name);
        let refs:serde_json::Value=serde_json::from_slice(&std::fs::read(&report_path)?)?;
        let reference:sims_fixtures::BlobRef=serde_json::from_value(refs["cold_rust_cut"].clone())?;
        let simulation:sims_sim::Snapshot=sims_fixtures::Store::open(report_path.with_extension("store"))?.get_value(&reference)?;
        let source=root.join("fixtures/private/goth-completed-continuation-20261030-third/results/native-sequence/completed-01");
        let encoded=std::fs::read(source.join("observation.json.gz"))?;
        let hash=sims_fixtures::digest(&encoded);
        ensure!(hash=="bca55b52701af064a75176057eb037cac2ae15df0f38aa428bbc4b926cc632bf","current native C1 differs");
        let state:serde_json::Value=serde_json::from_reader(flate2::read::GzDecoder::new(&encoded[..]))?;
        let raw=std::fs::read(source.join("acknowledged.mem"))?;
        let memory=Readset::parse(&raw)?;
        let report=super::super::completed_fields::animation::compare(&state,&memory,&simulation.state,&hash)?;
        eprintln!("C1 animator comparison: {} differing top-level fields",report["differences"].as_array().unwrap().len());
        ensure!(report["differences"].as_array().context("animator comparison rows absent")?.is_empty(),
            "current native C1 animator/layer/clock fields differ");
        if let Ok(path)=std::env::var("SIMS_RETAINED_ANIMATOR_DIAG_OUTPUT") {
            use std::io::Write;
            let mut file=std::fs::OpenOptions::new().write(true).create_new(true).open(path)?;
            file.write_all(&serde_json::to_vec_pretty(&report)?)?;
        }
        Ok(())
    }

    #[test]
    #[ignore = "actual current C1 pose, movement and geometry versus retained Rust endpoint"]
    fn retained_first_boundary_pose_comparison() -> Result<()> {
        let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let report_path=std::env::var("SIMS_RETAINED_POSE_REPORT_PATH").map(std::path::PathBuf::from)
            .unwrap_or_else(|_|root.join("fixtures/private/goth-pose-compare-20261030-first/corrected-boundary-first.json"));
        let refs:serde_json::Value=serde_json::from_slice(&std::fs::read(&report_path)?)?;
        let store=sims_fixtures::Store::open(report_path.with_extension("store"))?;
        let simulation:sims_sim::Snapshot=store.get_value(&serde_json::from_value(refs["cold_rust_cut"].clone())?)?;
        let assets:BTreeMap<sims_vm::ObjectId,sims_content::character_loading::live::SavedAssets>=store.get_value(&serde_json::from_value(refs["cold_character_assets"].clone())?)?;
        let pool:sims_content::character_geometry::MeshPool=store.get_value(&serde_json::from_value(refs["cold_mesh_pool"].clone())?)?;
        let pointers:Vec<u32>=serde_json::from_value(refs["native_meshes"].clone())?;
        let source=root.join("fixtures/private/goth-completed-continuation-20261030-third/results/native-sequence/completed-01");
        let encoded=std::fs::read(source.join("observation.json.gz"))?;
        let hash=sims_fixtures::digest(&encoded);
        ensure!(hash=="bca55b52701af064a75176057eb037cac2ae15df0f38aa428bbc4b926cc632bf","current native C1 differs");
        let state:serde_json::Value=serde_json::from_reader(flate2::read::GzDecoder::new(&encoded[..]))?;
        let raw=std::fs::read(source.join("acknowledged.mem"))?;let memory=Readset::parse(&raw)?;
        let report=super::super::completed_fields::pose::compare(&state,&memory,&simulation.state,&assets,&pool,&pointers,&hash)?;
        eprintln!("C1 pose comparison: {} differing fields of {}",report["differences"],report["checks"]);
        if let Ok(path)=std::env::var("SIMS_RETAINED_POSE_DIAG_OUTPUT") {
            use std::io::Write;
            let mut file=std::fs::OpenOptions::new().write(true).create_new(true).open(path)?;
            file.write_all(&serde_json::to_vec_pretty(&report)?)?;
        }
        if std::env::var("SIMS_RETAINED_POSE_REQUIRE_MATCH").as_deref()==Ok("1") {
            ensure!(report["differences"]==0,"native C1 pose/movement/geometry differs");
        }
        Ok(())
    }

    #[test]
    #[ignore="actual pre/post precision C1 snapshots; all other retained state must be identical"]
    fn retained_precision_only_changes_three_observed_delta_words()->Result<()> {
        let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let load=|path:&str,expected:&str|->Result<sims_sim::Snapshot>{
            let path=root.join(path);
            let report:serde_json::Value=serde_json::from_slice(&std::fs::read(&path)?)?;
            let blob:sims_fixtures::BlobRef=serde_json::from_value(report["cold_rust_cut"].clone())?;
            ensure!(blob.sha256==expected,"precision endpoint source differs");
            sims_fixtures::Store::open(path.with_extension("store"))?.get_value(&blob)
        };
        let before=load("fixtures/private/goth-animation-join-20261030-first/fpu-boundary-second.json",
            "533042b0321671eb9a19bd6718021fc41a32b6daefd874265980b9281509a295")?;
        let mut after=load("fixtures/private/goth-pose-compare-20261030-first/corrected-boundary-first.json",
            "b50adb045cf6f9aa8410f75089f4540bd23f555805d2950facd306ac99d5243c")?;
        for (actor,native,prior)in [(16,3210241679_u32,3210241681_u32),(21,1065421353,1065421352),(259,1064621546,1064621545)] {
            let old=before.state.characters[&actor].controls.movement.path.as_ref().context("prior movement path absent")?;
            let new=after.state.characters.get_mut(&actor).context("corrected actor absent")?.controls.movement.path.as_mut().context("corrected movement path absent")?;
            ensure!(old.delta[0].to_bits()==prior&&new.delta[0].to_bits()==native,"observed precision delta differs");
            new.delta[0]=old.delta[0];
        }
        ensure!(after==before&&serde_json::to_vec(&after)?==serde_json::to_vec(&before)?,
            "precision fix changed retained state beyond the three native delta words");
        Ok(())
    }

    #[test]
    #[ignore="actual current C1 physical ownership versus the retained Rust endpoint"]
    fn retained_first_boundary_physical_comparison()->Result<()> {
        let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let report_path=root.join("fixtures/private/goth-pose-compare-20261030-first/corrected-boundary-first.json");
        let refs:serde_json::Value=serde_json::from_slice(&std::fs::read(&report_path)?)?;
        let simulation:sims_sim::Snapshot=sims_fixtures::Store::open(report_path.with_extension("store"))?
            .get_value(&serde_json::from_value(refs["cold_rust_cut"].clone())?)?;
        let source=root.join("fixtures/private/goth-completed-continuation-20261030-third/results/native-sequence/completed-01");
        let encoded=std::fs::read(source.join("observation.json.gz"))?;
        ensure!(sims_fixtures::digest(&encoded)=="bca55b52701af064a75176057eb037cac2ae15df0f38aa428bbc4b926cc632bf","native C1 source differs");
        let state:serde_json::Value=serde_json::from_reader(flate2::read::GzDecoder::new(&encoded[..]))?;
        let raw=std::fs::read(source.join("acknowledged.mem"))?;let memory=Readset::parse(&raw)?;
        let report=super::super::completed_fields::physical::compare(&state,&memory,&simulation.state)?;
        eprintln!("C1 physical comparison: {} differing fields of {}",report["differences"],report["checks"]);
        if let Ok(path)=std::env::var("SIMS_RETAINED_PHYSICAL_DIAG_OUTPUT") {
            use std::io::Write;let mut file=std::fs::OpenOptions::new().write(true).create_new(true).open(path)?;
            file.write_all(&serde_json::to_vec_pretty(&report)?)?;
        }
        ensure!(report["differences"]==0,"native C1 physical ownership differs");Ok(())
    }
    #[test]
    #[ignore="actual current C1 route ownership versus the retained Rust endpoint"]
    fn retained_first_boundary_route_comparison()->Result<()> {
        let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let report_path=root.join("fixtures/private/goth-pose-compare-20261030-first/corrected-boundary-first.json");
        let refs:serde_json::Value=serde_json::from_slice(&std::fs::read(&report_path)?)?;
        let simulation:sims_sim::Snapshot=sims_fixtures::Store::open(report_path.with_extension("store"))?
            .get_value(&serde_json::from_value(refs["cold_rust_cut"].clone())?)?;
        let source=root.join("fixtures/private/goth-completed-continuation-20261030-third/results/native-sequence/completed-01");
        let encoded=std::fs::read(source.join("observation.json.gz"))?;
        ensure!(sims_fixtures::digest(&encoded)=="bca55b52701af064a75176057eb037cac2ae15df0f38aa428bbc4b926cc632bf","native C1 source differs");
        let state:serde_json::Value=serde_json::from_reader(flate2::read::GzDecoder::new(&encoded[..]))?;
        let raw=std::fs::read(source.join("acknowledged.mem"))?;let memory=Readset::parse(&raw)?;
        let report=super::super::completed_fields::routes::compare(&state,&memory,&simulation.state)?;
        eprintln!("C1 route comparison: {} differing fields of {}",report["differences"],report["checks"]);
        if let Ok(path)=std::env::var("SIMS_RETAINED_ROUTE_DIAG_OUTPUT") {
            use std::io::Write;let mut file=std::fs::OpenOptions::new().write(true).create_new(true).open(path)?;
            file.write_all(&serde_json::to_vec_pretty(&report)?)?;
        }
        ensure!(report["differences"]==0,"native C1 route ownership differs");Ok(())
    }

    #[test]
    #[ignore="stored actual C2 replay and original dispatcher observations"]
    fn retained_second_boundary_dispatch_comparison()->Result<()> {
        let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let report:serde_json::Value=serde_json::from_slice(&std::fs::read(root.join("fixtures/private/goth-route-physical-20261030-first/sequence-resident-constants-first.json"))?)?;
        let raw=std::fs::read(root.join("fixtures/private/goth-completed-continuation-20261030-third/results/native-sequence/native-bytecode.json"))?;
        ensure!(sims_fixtures::digest(&raw)=="7430b5bcddc06cbc4a5be8ab0708da56f18bce00ef3a4bb9bc14a20552f05c44","native trace differs");
        let rows:Vec<serde_json::Value>=serde_json::from_slice(&raw)?;
        let native:Vec<_>=rows.into_iter().filter(|r|r["stop_index"].as_u64().is_some_and(|n|n>64&&n<346)).collect();
        let trace:Vec<sims_vm::Trace>=serde_json::from_value(report["ordered_trace"].clone())?;
        let trace:Vec<_>=trace.into_iter().filter(|r|r.tick==89074).collect();
        let result=super::super::completed_replay::trace::compare_dispatches(&serde_json::json!(native),&trace,89074)?;
        eprintln!("C2 dispatcher comparison: {} instructions",result["instructions"]);
        if let Ok(path)=std::env::var("SIMS_RETAINED_C2_TRACE_DIAG_OUTPUT") {
            use std::io::Write;let mut file=std::fs::OpenOptions::new().write(true).create_new(true).open(path)?;
            file.write_all(&serde_json::to_vec_pretty(&result)?)?;
        }
        Ok(())
    }

    #[test]
    #[ignore="actual C2 pose/physical/route owners versus the retained Rust endpoint"]
    fn retained_second_boundary_component_comparison()->Result<()> {
        let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let path=root.join("fixtures/private/goth-route-physical-20261030-first/sequence-resident-constants-first.json");
        let report:serde_json::Value=serde_json::from_slice(&std::fs::read(&path)?)?;
        ensure!(report["after"]["tick"]==89075,"Rust C2 absent");
        let store=sims_fixtures::Store::open(path.with_extension("store"))?;
        let cut:sims_sim::Snapshot=store.get_value(&serde_json::from_value(report["cold_rust_cut"].clone())?)?;
        let assets:BTreeMap<sims_vm::ObjectId,sims_content::character_loading::live::SavedAssets>=store.get_value(&serde_json::from_value(report["cold_character_assets"].clone())?)?;
        let pool:sims_content::character_geometry::MeshPool=store.get_value(&serde_json::from_value(report["cold_mesh_pool"].clone())?)?;
        let pointers:Vec<u32>=serde_json::from_value(report["native_meshes"].clone())?;
        let source=root.join("fixtures/private/goth-completed-continuation-20261030-third/results/native-sequence");
        let raw=std::fs::read(source.join("capture-result.json"))?;
        ensure!(sims_fixtures::digest(&raw)=="4139107d1965925bf3ee5f44fa524d5d4b1fae22bfdb617edfffb69d33bcd393","native sequence seal differs");
        let seal:serde_json::Value=serde_json::from_slice(&raw)?;
        let encoded=std::fs::read(source.join("completed-02/observation.json.gz"))?;let hash=sims_fixtures::digest(&encoded);
        ensure!(seal["observations"][2]["sha256"]==hash,"native C2 differs");
        let state:serde_json::Value=serde_json::from_reader(flate2::read::GzDecoder::new(&encoded[..]))?;
        let raw=std::fs::read(source.join("completed-02/acknowledged.mem"))?;let m=Readset::parse(&raw)?;
        let animation=super::super::completed_fields::animation::compare(&state,&m,&cut.state,&hash)?;
        let pose=super::super::completed_fields::pose::compare(&state,&m,&cut.state,&assets,&pool,&pointers,&hash)?;
        let physical=super::super::completed_fields::physical::compare(&state,&m,&cut.state)?;
        let routes=super::super::completed_fields::routes::compare(&state,&m,&cut.state)?;
        let result=serde_json::json!({"schema":1,"native_parity_credit":0,"animation":animation,"pose":pose,"physical":physical,"routes":routes});
        if let Ok(path)=std::env::var("SIMS_RETAINED_C2_COMPONENT_DIAG_OUTPUT") {
            use std::io::Write;let mut file=std::fs::OpenOptions::new().write(true).create_new(true).open(path)?;
            file.write_all(&serde_json::to_vec_pretty(&result)?)?;
        }
        eprintln!("C2 components: animation {}, pose {}, physical {}, routes {}",animation["differences"],pose["differences"],physical["differences"],routes["differences"]);
        ensure!(animation["differences"].as_array().context("animation differences absent")?.is_empty()
            &&pose["differences"]==0&&physical["differences"]==0&&routes["differences"]==0,"native C2 component difference");
        Ok(())
    }

    #[test]
    #[ignore="all actual stored completed cuts and their pinned original components"]
    fn retained_stored_sequence_component_matrix()->Result<()> {
        let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let path=std::env::var("SIMS_RETAINED_MATRIX_REPORT_PATH").map(std::path::PathBuf::from).unwrap_or_else(|_|root.join("fixtures/private/goth-disabled-sound-20261030-first/sequence-stored-third.json"));
        let report:serde_json::Value=serde_json::from_slice(&std::fs::read(&path)?)?;
        let store=sims_fixtures::Store::open(path.with_extension("store"))?;
        let result=super::super::completed_fields::sequence::compare(&report,&store,&root.join("fixtures/private/goth-completed-continuation-20261030-third/results/native-sequence"))?;
        if let Ok(path)=std::env::var("SIMS_RETAINED_MATRIX_DIAG_OUTPUT") {
            use std::io::Write;let mut file=std::fs::OpenOptions::new().write(true).create_new(true).open(path)?;
            file.write_all(&serde_json::to_vec_pretty(&result)?)?;
        }
        ensure!(result["matched"]==true,"stored component matrix differs");Ok(())
    }

    #[test]
    #[ignore="completed B+32 retained endpoint versus original native components"]
    fn retained_completed_endpoint_component_comparison()->Result<()> {
        let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let path=std::env::var("SIMS_RETAINED_ENDPOINT_REPORT_PATH").map(std::path::PathBuf::from)
            .unwrap_or_else(|_|root.join("fixtures/private/goth-disabled-sound-20261030-first/sequence-sound-second.json"));
        let report:serde_json::Value=serde_json::from_slice(&std::fs::read(&path)?)?;
        let boundary=report["after"]["tick"].as_u64().context("endpoint tick absent")?.checked_sub(89073).context("endpoint predates B")?as usize;
        ensure!((1..=32).contains(&boundary)&&report["boundary_sequence"].as_array().is_some_and(|r|r.len()==boundary&&r.last().is_some_and(|r|r.get("status").is_none())),"completed endpoint absent");
        let store=sims_fixtures::Store::open(path.with_extension("store"))?;
        let cut=super::super::completed_assets::checkpoints::Retained {
            simulation:store.get_value(&serde_json::from_value(report["cold_rust_cut"].clone())?)?,
            characters:store.get_value(&serde_json::from_value(report["cold_character_assets"].clone())?)?,
            ordinary:store.get_value(&serde_json::from_value(report["cold_ordinary"].clone())?)?,
            meshes:store.get_value(&serde_json::from_value(report["cold_mesh_pool"].clone())?)?,
            dependent:store.get_value(&serde_json::from_value(report["cold_dependent_jobs"].clone())?)?,
            presentation:store.get_value(&serde_json::from_value(report["cold_presentation"].clone())?)?,
            native_meshes:serde_json::from_value(report["native_meshes"].clone())?,
            floors:serde_json::from_value(report["footstep_floor_sources"].clone())?,
        };
        let source=root.join("fixtures/private/goth-completed-continuation-20261030-third/results/native-sequence");
        let raw=std::fs::read(source.join("capture-result.json"))?;
        ensure!(sims_fixtures::digest(&raw)=="4139107d1965925bf3ee5f44fa524d5d4b1fae22bfdb617edfffb69d33bcd393","original sequence seal differs");
        let seal:serde_json::Value=serde_json::from_slice(&raw)?;
        let encoded=std::fs::read(source.join(format!("completed-{boundary:02}/observation.json.gz")))?;let hash=sims_fixtures::digest(&encoded);
        ensure!(seal["observations"][boundary]["sha256"]==hash,"native B+32 differs");
        let state:serde_json::Value=serde_json::from_reader(flate2::read::GzDecoder::new(&encoded[..]))?;
        let raw=std::fs::read(source.join(format!("completed-{boundary:02}/acknowledged.mem")))?;let m=Readset::parse(&raw)?;
        let result=super::super::completed_fields::sequence::components(&state,&m,&cut,&hash)?;
        if let Ok(path)=std::env::var("SIMS_RETAINED_ENDPOINT_DIAG_OUTPUT") {
            use std::io::Write;let mut file=std::fs::OpenOptions::new().write(true).create_new(true).open(path)?;
            file.write_all(&serde_json::to_vec_pretty(&result)?)?;
        }
        eprintln!("B+{boundary} components: {}",result["matched"]);
        ensure!(result["matched"]==true,"native B+32 component difference");Ok(())
    }

    #[test]
    #[ignore="snapshot of already-published typed cuts in a live retained diagnostic store"]
    fn retained_stored_prefix_component_matrix()->Result<()> {
        let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let path=std::env::var("SIMS_RETAINED_PREFIX_STORE_PATH").map(std::path::PathBuf::from)
            .unwrap_or_else(|_|root.join("fixtures/private/goth-disabled-sound-20261030-first/sequence-stored-third.store"));
        // An explicit already-published watermark makes a concurrent reader
        // independent of later append-only cuts appearing during directory IO.
        let maximum=std::env::var("SIMS_RETAINED_PREFIX_MAX_BOUNDARY").ok().map(|s|s.parse::<u64>()).transpose()?;
        ensure!(maximum.is_none_or(|n|n<=32),"retained prefix watermark out of range");
        let store=sims_fixtures::Store::open(&path)?;let mut cuts=BTreeMap::new();
        for directory in std::fs::read_dir(path.join("blobs/sha256"))? {
            let directory=directory?;ensure!(directory.file_type()?.is_dir(),"unexpected blob namespace entry");
            for file in std::fs::read_dir(directory.path())? {
                let file=file?;if file.path().extension().is_none_or(|v|v!="blob"){continue;}
                use std::io::Read;let mut header=[0_u8;49];std::fs::File::open(file.path())?.read_exact(&mut header)?;
                ensure!(&header[..8]==b"SIMSBL01"&&matches!(header[8],0|1),"retained blob header differs");
                let reference=sims_fixtures::BlobRef{bytes:u64::from_le_bytes(header[9..17].try_into()?),sha256:header[17..49].iter().map(|b|format!("{b:02x}")).collect()};
                ensure!(store.blob_path(&reference)?==file.path().canonicalize()?,"retained blob filename/header differs");
                let cut:super::super::completed_assets::checkpoints::Retained=store.get_value(&reference)?;
                let index=cut.simulation.state.vm.tick.checked_sub(89073).context("retained cut predates B")?;
                if maximum.is_some_and(|n|index>n){continue;}
                ensure!(index<=32&&cuts.insert(index,reference).is_none(),"duplicate/misaligned retained cut");
            }
        }
        ensure!(!cuts.is_empty()&&cuts.keys().copied().eq(0..cuts.len()as u64),"retained cuts are not a complete prefix");
        ensure!(maximum.is_none_or(|n|cuts.len()==n as usize+1),"requested published prefix is absent");
        let report=serde_json::json!({"baseline_retained_cut":cuts[&0],"boundary_sequence":cuts.iter().filter(|(i,_)|**i!=0).map(|(&i,r)|serde_json::json!({"boundary":i,"retained_cut":r})).collect::<Vec<_>>()});
        let native=root.join("fixtures/private/goth-completed-continuation-20261030-third/results/native-sequence");
        let result=if std::env::var("SIMS_RETAINED_PREFIX_ALL").as_deref()==Ok("1") {
            super::super::completed_fields::sequence::analyze_all(&report,&store,&native)?
        }else{super::super::completed_fields::sequence::compare(&report,&store,&native)?};
        if let Ok(path)=std::env::var("SIMS_RETAINED_PREFIX_DIAG_OUTPUT") {
            use std::io::Write;let mut file=std::fs::OpenOptions::new().write(true).create_new(true).open(path)?;
            file.write_all(&serde_json::to_vec_pretty(&result)?)?;
        }
        ensure!(result["matched"]==true,"stored component prefix differs");Ok(())
    }

    #[test]
    #[ignore="publication timing diagnostic over two independently stored retained endpoints"]
    fn retained_cpu_vertex_publication_diagnostic()->Result<()> {
        let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let load=|path:&str|->Result<BTreeMap<sims_vm::ObjectId,sims_content::character_loading::live::SavedAssets>>{
            let path=root.join(path);let report:serde_json::Value=serde_json::from_slice(&std::fs::read(&path)?)?;
            sims_fixtures::Store::open(path.with_extension("store"))?.get_value(&serde_json::from_value(report["cold_character_assets"].clone())?)
        };
        let before=load("fixtures/private/goth-route-physical-20261030-first/sequence-resident-constants-first.json")?;
        let after=load("fixtures/private/goth-disabled-sound-20261030-first/sequence-sound-second.json")?;
        let mut rows=Vec::new();
        for (&id,a)in &after {
            let b=&before[&id];
            rows.push(serde_json::json!({"actor":id,"vertices_unchanged_from_C2":a.geometry.vertices==b.geometry.vertices,
                "before_sha256":sims_fixtures::digest(&serde_json::to_vec(&b.geometry.vertices)?),
                "after_sha256":sims_fixtures::digest(&serde_json::to_vec(&a.geometry.vertices)?)}));
        }
        eprintln!("CPU vertex publication {rows:?}");
        if let Ok(path)=std::env::var("SIMS_RETAINED_VERTEX_DIAG_OUTPUT") {
            use std::io::Write;let mut file=std::fs::OpenOptions::new().write(true).create_new(true).open(path)?;
            file.write_all(&serde_json::to_vec_pretty(&serde_json::json!({"native_parity_credit":0,"rows":rows}))?)?;
        }
        Ok(())
    }

    #[test]
    #[ignore="offline vertex publication at captured candidate frame boundaries; not an input schedule proof"]
    fn retained_vertex_frame_candidate_diagnostic()->Result<()> {
        let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let path=root.join("fixtures/private/goth-disabled-sound-20261030-first/sequence-stored-third.store");let store=sims_fixtures::Store::open(&path)?;
        let mut cuts=BTreeMap::new();
        for directory in std::fs::read_dir(path.join("blobs/sha256"))? {for file in std::fs::read_dir(directory?.path())? {
            let file=file?;if file.path().extension().is_none_or(|v|v!="blob"){continue;}
            use std::io::Read;let mut h=[0_u8;49];std::fs::File::open(file.path())?.read_exact(&mut h)?;ensure!(&h[..8]==b"SIMSBL01","blob header differs");
            let r=sims_fixtures::BlobRef{bytes:u64::from_le_bytes(h[9..17].try_into()?),sha256:h[17..49].iter().map(|b|format!("{b:02x}")).collect()};
            let cut:super::super::completed_assets::checkpoints::Retained=store.get_value(&r)?;let tick=cut.simulation.state.vm.tick;
            if matches!(tick,89081|89082){cuts.insert(tick,(r,cut));}
        }}
        ensure!(cuts.len()==2,"candidate C8/C9 cuts absent");
        let source=root.join("fixtures/private/goth-completed-continuation-20261030-third/results/native-sequence/completed-09");
        let encoded=std::fs::read(source.join("observation.json.gz"))?;
        let seal_raw=std::fs::read(source.parent().unwrap().join("capture-result.json"))?;
        ensure!(sims_fixtures::digest(&seal_raw)=="4139107d1965925bf3ee5f44fa524d5d4b1fae22bfdb617edfffb69d33bcd393","original frame seal differs");
        let seal:serde_json::Value=serde_json::from_slice(&seal_raw)?;
        ensure!(seal["observations"][9]["sha256"]==sims_fixtures::digest(&encoded),"original C9 frame differs");
        let state:serde_json::Value=serde_json::from_reader(flate2::read::GzDecoder::new(&encoded[..]))?;
        let bytes=std::fs::read(source.join("acknowledged.mem"))?;let m=Readset::parse(&bytes)?;
        ensure!(state["readset"]["sha256"]==m.source_sha256(),"native draw-source readset differs");
        super::super::completed_skeletons::captured::reads(&state["domains"]["geometry"],&m)?;
        let g=&state["domains"]["geometry"];let mut expected=BTreeMap::new();
        for actor in g["characters"].as_array().context("native geometry actors absent")? {
            let id:i16=actor["object"].as_i64().context("native vertex actor absent")?.try_into()?;let mut buffers=Vec::new();
            for appearance in actor["ordered_appearances"].as_array().context("native appearance order absent")? {
                let a=g["appearances"].as_array().unwrap().iter().find(|a|a["pointer"]==*appearance).context("native appearance absent")?;
                for record in a["records"].as_array().unwrap() {
                    let b=g["bindings"].as_array().unwrap().iter().find(|b|b["pointer"]==record["binding"]).context("native binding absent")?;
                    let d=g["drawables"].as_array().unwrap().iter().find(|d|d["pointer"]==b["drawable"]).context("native drawable absent")?;
                    let raw=super::super::completed_animations::bytes(d["vertices_raw"].as_str().unwrap())?;
                    let values:Vec<_>=raw.chunks_exact(32).flat_map(|v|v[..24].chunks_exact(4).map(|b|u32::from_le_bytes(b.try_into().unwrap()))).collect();buffers.push(values);
                }
            }
            expected.insert(id,buffers);
        }
        let index=sims_content::Index::scan(&root.join("cache/managed-independent-20261022-third/immutable-assets"))?;
        let mut avatars=BTreeMap::new();let mut rows=Vec::new();
        for (tick,(reference,mut cut))in cuts {
            for (&id,a)in &mut cut.characters {
                if !avatars.contains_key(&id){avatars.insert(id,sims_content::saved_avatar::BoundAvatar::restore(&index,&a.skeleton,id as u64,&a.binding)?);}
                let resident=cut.simulation.state.characters.get_mut(&id).unwrap();
                let control=if std::env::var("SIMS_RETAINED_VERTEX_FRAME_CONTROL").ok().as_deref()==Some("1") {resident.character.skin.animator.fp_control()}else{0x037f};
                let uploads=a.model.with_bound(resident,|bound|a.geometry.update_loaded_vertices_with_control(&avatars[&id],bound,false,control))?;
                let mut different=0;let mut checked=0;let mut max_error=0_f32;
                ensure!(a.geometry.vertices.len()==expected[&id].len(),"candidate binding domain differs");
                for (i,native)in expected[&id].iter().enumerate() {
                    let actual:Vec<_>=a.geometry.vertices[&i].vertices.iter().flat_map(|v|v.position.into_iter().chain(v.normal).map(f32::to_bits)).collect();
                    ensure!(native.len()==actual.len(),"candidate vertex domain differs");
                    for (&a,&b)in native.iter().zip(&actual){checked+=1;if a!=b{different+=1;max_error=max_error.max((f32::from_bits(a)-f32::from_bits(b)).abs());}}
                }
                rows.push(serde_json::json!({"source_tick":tick,"source_cut":reference,"actor":id,"control":control,"checked_words":checked,"different_words":different,"max_error":max_error,"uploads":uploads}));
            }
        }
        if let Ok(path)=std::env::var("SIMS_RETAINED_VERTEX_FRAME_DIAG_OUTPUT") {
            use std::io::Write;let mut file=std::fs::OpenOptions::new().write(true).create_new(true).open(path)?;
            file.write_all(&serde_json::to_vec_pretty(&serde_json::json!({"native_parity_credit":0,"rows":rows,"qualification":"Candidate publication calculations only. Native draw callsite and placement in the input schedule are not established."}))?)?;
        }
        eprintln!("vertex frame candidates {rows:?}");Ok(())
    }

}
