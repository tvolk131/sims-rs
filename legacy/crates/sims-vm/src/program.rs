use serde::Serialize;
use sha2::{Digest, Sha256};
use sims_formats::{
    animation::{Animation, Samples},
    behavior::Behavior,
    execution::{ObjectFunction, Slots},
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Debug, Clone, Serialize)]
pub struct Routine {
    pub name: String,
    pub body: Behavior,
}
#[derive(Debug, Clone, Serialize, Default)]
pub struct MenuEntry {
    pub id: u32,
    pub flags: u32,
    pub label: String,
    pub action: u16,
    pub check: u16,
    pub autonomy: AutonomyAdvertisement,
}
#[derive(Debug, Clone, Serialize, Default)]
pub struct AutonomyAdvertisement {
    pub threshold: u32,
    pub attenuation_code: u32,
    pub attenuation: f32,
    pub joining_index: i32,
    pub motives: Vec<crate::motives::Advertisement>,
    pub flags2: u32,
}
impl From<&sims_formats::behavior::Interaction> for AutonomyAdvertisement {
    fn from(i: &sims_formats::behavior::Interaction) -> Self {
        Self {
            threshold: i.autonomy,
            attenuation_code: i.attenuation_code,
            attenuation: i.attenuation,
            joining_index: i.joining_index,
            motives: i
                .motives
                .iter()
                .map(|m| crate::motives::Advertisement {
                    personality: m.personality as i16,
                    minimum: m.minimum,
                    delta: m.delta,
                })
                .collect(),
            flags2: i.flags2,
        }
    }
}
#[derive(Debug, Clone, Serialize, Default)]
pub struct Module {
    pub headline_catalog: Option<crate::headline::Catalog>,
    pub source: String,
    /// SHA-256 of the original IFF; also binds snapshots to unparsed resource data.
    pub content_hash: String,
    pub semiglobal: Option<String>,
    pub routines: BTreeMap<u16, Routine>,
    /// Native BHAV revision words at header offset 8, used by original objt
    /// compatibility signatures. These are not hashes of the instruction bytes.
    pub routine_versions: BTreeMap<u16, u32>,
    pub constants: BTreeMap<u16, Vec<i16>>,
    pub named_constants: BTreeMap<u16, sims_formats::tuning::Constants>,
    pub slots: BTreeMap<u16, Slots>,
    pub strings: BTreeMap<u16, Vec<String>>,
    /// Native STR#301 comment markers, using one-based StringSet indices.
    pub dialog_marked: BTreeSet<usize>,
    /// Catalog display names, keyed by the OBJD resource ID.
    pub catalog_names: BTreeMap<u16, String>,
    pub object_animation_tables: BTreeMap<u16, u16>,
    pub object_functions: BTreeMap<u16, Vec<ObjectFunction>>,
    pub object_function_sources: BTreeMap<u16, String>,
    pub object_definition_words: BTreeMap<u16, Vec<i16>>,
    /// Loader provenance for exact immutable relinking. The normalized words
    /// themselves enter the fingerprint; this marker never changes its schema.
    #[serde(skip)]
    pub normalized_object_definitions: bool,
    /// Original resource enumeration order, used by multi-tile construction.
    pub object_definition_order: Vec<u16>,
    pub object_stand_functions: BTreeMap<u16, u16>,
    pub object_interactions: BTreeMap<u16, Vec<MenuEntry>>,
    pub interaction_tables: BTreeMap<u16, Vec<MenuEntry>>,
    pub sounds: BTreeMap<u16, String>,
}
#[derive(Debug, Clone, Serialize)]
pub struct AnimationAsset {
    pub source: String,
    pub content_hash: String,
    pub samples_source: String,
    pub samples_hash: String,
    pub clip: Animation,
    pub samples: Samples,
}
#[derive(Debug, Clone, Serialize, Default)]
pub struct CareerCatalog {
    pub source: String,
    pub content_hash: String,
    /// Native loaded career-vector order. Career IDs are signed 32-bit values;
    /// the primitive's destination remains a signed 16-bit VM variable.
    pub ids: Vec<i32>,
    /// Same native vector order as `ids`. None is an identity-only catalog and
    /// cannot answer career field reads.
    pub records: Option<Vec<sims_formats::career::Career>>,
    /// Native runtime STR# overlays in the same vector order. None means the
    /// text loading service is unavailable, not an empty career name.
    pub text: Option<Vec<crate::career_text::Text>>,
}
#[derive(Debug, Clone, Serialize, Default)]
pub struct Program {
    /// Complete registered GUID -> resource +b4 flags for this neighborhood.
    /// None is an unavailable catalog; an absent GUID in Some is a null linked
    /// resource. Classification is registration metadata, not person species.
    pub neighbor_resource_flags: Option<BTreeMap<u32, u32>>,
    /// Native language-manager word for locale-dependent text. Resource loaders
    /// supply their selected language explicitly; absence is an unresolved input.
    pub text_language: Option<u32>,
    pub resource_storage: Option<Vec<crate::resource_storage::Definition>>,
    pub careers: Option<CareerCatalog>,
    /// Available accessory names and hashes of their source catalogs/meshes/textures.
    pub accessories: BTreeMap<String, BTreeMap<String, String>>,
    pub globals: String,
    pub modules: BTreeMap<String, Module>,
    pub animations: BTreeMap<String, AnimationAsset>,
}
impl Program {
    /// Original 0x004026ed: root revision shifted by 16, plus the revision of
    /// every distinct statically reachable tree (including the root). All
    /// lookups keep the original object's resource context. Missing trees add
    /// nothing; cycles and duplicate calls are counted once. Arithmetic wraps.
    pub fn behavior_signature(&self, owner: &str, root: u16) -> Result<u32, String> {
        if !self.modules.contains_key(owner) {
            return Err(format!("missing signature owner {owner}"));
        }
        let version = |id| -> Result<u32, String> {
            let Ok((source, _)) = self.resolve(owner, id) else {
                return Ok(0);
            };
            self.modules[source]
                .routine_versions
                .get(&id)
                .copied()
                .ok_or_else(|| format!("missing native BHAV revision {source}:{id}"))
        };
        let mut value = version(root)?.wrapping_shl(16);
        let mut seen = BTreeSet::new();
        let mut pending = vec![root];
        while let Some(id) = pending.pop() {
            if !seen.insert(id) {
                continue;
            }
            value = value.wrapping_add(version(id)?);
            if let Ok((_, routine)) = self.resolve(owner, id) {
                for instruction in &routine.body.instructions {
                    let target = instruction.opcode & 0x7fff;
                    if target >= 256 {
                        pending.push(target);
                    }
                }
            }
        }
        Ok(value)
    }
    pub fn resolve(&self, owner: &str, id: u16) -> Result<(&str, &Routine), String> {
        self.lookup_routine(owner, id)?
            .ok_or_else(|| format!("missing BHAV {id:#06x} for {owner}"))
    }
    /// A failed native resource lookup is distinct from an incomplete linked
    /// program. Callers choose whether absence returns false or faults.
    pub fn lookup_routine(&self, owner: &str, id: u16) -> Result<Option<(&str, &Routine)>, String> {
        let private = self
            .modules
            .get(owner)
            .ok_or_else(|| format!("missing code owner {owner}"))?;
        let source = match id {
            0..=255 => return Err(format!("{id:#06x} is a primitive, not a BHAV")),
            256..=4095 => &self.globals,
            4096..=8191 => &private.source,
            _ => match private.semiglobal.as_ref() {
                Some(source) => source,
                None => return Ok(None),
            },
        };
        let module = self
            .modules
            .get(source)
            .ok_or_else(|| format!("missing module {source}"))?;
        Ok(module
            .routines
            .get(&id)
            .map(|routine| (module.source.as_str(), routine)))
    }
    pub fn fingerprint(&self) -> String {
        // BTreeMap ordering, schema fields and the VM semantics version make this stable.
        let mut hash = Sha256::new();
        // v33 rejects checkpoints whose dynamic factory overwrote placement
        // flags (word 42) with price instead of writing word 41.
        // v69 connects native floor creation and late root-parameter publication.
        // v80 executes person-field image invalidation at native write-intent resolution.
        // v81 services native Remove Object requests inside deletion Reset callbacks.
        // v82 queues native date/star notifications at person write intent.
        // v83 retains the neighbor friend-count cache and the original NBRS
        // dirty initializer. Earlier imports cannot reconstruct that history.
        // v84 executes imported Look Towards through the shared native animator,
        // including its physical callbacks; earlier turn histories differ.
        // v85 retains the off-lot person wrapper through its init-visitor tail.
        // v86 executes category-7 portrait headlines with native resource and
        // raw-local selection; faulted v85 saves cannot reconstruct that history.
        hash.update(b"sims-vm/program/v86\0");
        hash.update(
            serde_json::to_vec(&(
                &self.globals,
                &self.neighbor_resource_flags,
                &self.modules,
                &self.accessories,
                &self.careers,
                &self.resource_storage,
                &self.text_language,
            ))
            .expect("finite parsed program data"),
        );
        // Hundreds of original idle clips make float-to-JSON conversion dominate
        // save time. Hash every sample as exact little-endian bits instead.
        hash.update((self.animations.len() as u64).to_le_bytes());
        for (name, a) in &self.animations {
            hash.update(
                serde_json::to_vec(&(
                    name,
                    &a.source,
                    &a.content_hash,
                    &a.samples_source,
                    &a.samples_hash,
                    &a.clip,
                ))
                .expect("finite animation metadata"),
            );
            hash.update((a.samples.translations.len() as u64).to_le_bytes());
            hash.update((a.samples.rotations.len() as u64).to_le_bytes());
            let mut bytes = Vec::with_capacity(
                a.samples.translations.len() * 12 + a.samples.rotations.len() * 16,
            );
            for value in a
                .samples
                .translations
                .iter()
                .flatten()
                .chain(a.samples.rotations.iter().flatten())
            {
                bytes.extend_from_slice(&value.to_bits().to_le_bytes());
            }
            hash.update(bytes);
        }
        format!("{:x}", hash.finalize())
    }
    pub fn tuning(&self, owner: &str, address: u16) -> Result<i16, String> {
        let table = address >> 7;
        let index = (address & 127) as usize;
        let private = self.modules.get(owner).ok_or("missing tuning owner")?;
        let (source, id) = match table {
            0..=63 => (&private.source, table + 4096),
            64..=127 => (
                private
                    .semiglobal
                    .as_ref()
                    .ok_or("missing semiglobal tuning")?,
                table - 64 + 8192,
            ),
            128..=191 => (&self.globals, table - 128 + 256),
            _ => return Err(format!("unsupported tuning address {address:#06x}")),
        };
        self.modules
            .get(source)
            .and_then(|m| m.constants.get(&id))
            .and_then(|v| v.get(index))
            .copied()
            .ok_or_else(|| {
                format!(
                    "missing BCON {id} entry {index} in {source}; OTF overrides are unsupported"
                )
            })
    }
    pub fn dependencies(&self, owner: &str, entry: u16) -> Dependencies {
        let mut result = Dependencies::default();
        let mut queue = VecDeque::from([entry]);
        let mut seen = BTreeSet::new();
        while let Some(id) = queue.pop_front() {
            if !seen.insert(id) {
                continue;
            }
            match self.resolve(owner, id) {
                Err(e) => result.missing.push(e),
                Ok((source, routine)) => {
                    result
                        .routines
                        .push(format!("{source} :: BHAV {id:#06x} {}", routine.name));
                    for ins in &routine.body.instructions {
                        if ins.opcode >= 256 {
                            queue.push_back(ins.opcode);
                        } else {
                            result.primitives.insert(ins.opcode);
                        }
                    }
                }
            }
        }
        result
    }
}
#[derive(Debug, Default, Serialize)]
pub struct Dependencies {
    pub routines: Vec<String>,
    pub primitives: BTreeSet<u16>,
    pub missing: Vec<String>,
}
