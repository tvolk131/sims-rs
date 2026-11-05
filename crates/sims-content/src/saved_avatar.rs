//! Original appearance assets and native record/direct binding services. Saved
//! records, fresh profile selection and masks share the mesh/material decoder.
//! This does not execute the native geometry refresh or household activation.
use crate::{
    Index,
    avatar::{Avatar, Part, Texture},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sims_formats::{avatar, saved_objects::Person};
use sims_sim::pose::appearance::{Binding, Execution, Input, Records, Request, Response};
use std::{collections::BTreeMap, io::Cursor};

/// Selection prefix of native 0x599e87. Keys match a case-sensitive substring
/// of the mesh name. Empty/no-match results ask the mesh loader for its default.
pub fn texture_override<'a>(mesh_name: &str, binding: &'a str) -> &'a str {
    // The native search's end iterator doubles as its not-found result.
    if mesh_name.is_empty() {
        return "";
    }
    binding
        .split_once('=')
        .filter(|(key, _)| mesh_name.contains(key))
        .map_or("", |(_, value)| value)
}

fn equivalent_resource(index: &Index, name: &str) -> Result<usize> {
    let matches: Vec<_> = index
        .resources
        .iter()
        .enumerate()
        // Expansion archives store entries such as Skins\CSpellbound...bmp,
        // while appearance descriptors contain the texture's bare filename.
        // Keep every provider. Conflicting bytes need native path resolution,
        // not the extraction index's alphabetical archive ordering.
        .filter(|(_, r)| {
            r.name
                .rsplit(['\\', '/'])
                .next()
                .is_some_and(|basename| basename.eq_ignore_ascii_case(name))
        })
        .collect();
    let &(id, first) = matches
        .first()
        .with_context(|| format!("missing saved avatar resource {name}"))?;
    let bytes = first.read()?;
    for (_, r) in &matches[1..] {
        if r.read()? != bytes {
            let context = || {
                format!(
                    "conflicting saved avatar providers: {} and {}; native texture prefix could not resolve them",
                    first.locator, r.locator
                )
            };
            let selected = installed_texture_prefix(index, name).with_context(context)?;
            return selected.with_context(context);
        }
    }
    Ok(id)
}

/// Source-backed first success in the fixed leading texture search paths.
/// Original 5c9a3b -> 5c1c65 -> 5c1ec8 opens directory+name+extension before
/// decoding. Later directories cannot override that successful open. The
/// existing complete-collection adapter supplies filesystem enumeration order;
/// this is not a claim to have run the installer or the full Windows app.
fn installed_texture_prefix(index: &Index, name: &str) -> Result<Option<usize>> {
    use crate::avatar_installation::{Installation, complete_collection_asset_bindings};
    ensure!(
        name.is_ascii()
            && !name.contains(['/', '\\', '\0'])
            && name.to_ascii_lowercase().ends_with(".bmp"),
        "texture prefix needs an ASCII BMP basename"
    );
    let mut installation =
        Installation::new(index, complete_collection_asset_bindings(&index.root))?;
    installation.initial_mounts(b"./", 255, false)?;
    let paths = installation.texture_prefix(b"./", 255)?;
    ensure!(
        installation.open_failures().is_empty(),
        "texture installation has failed archive opens"
    );
    let directories = installation.directories()?;
    // 5c80eb registers IMAG aliases as BMP then TGA; 5c1a18/5c2594 append
    // them in that order. A BMP prefix hit precedes any later format search.
    let source = directories.open_plain_paths(
        index,
        &paths,
        &name.as_bytes()[..name.len() - 4],
        u32::from_be_bytes(*b"BMP\0"),
    )?;
    // Unknown provider suffixes, numbered/wildcard fallbacks and unsupported
    // installations retain the conflict boundary. Never choose an arbitrary
    // globally indexed file or invent an empty provider suffix after a miss.
    Ok(source)
}

#[cfg(test)]
mod resource_tests {
    use super::*;

    #[test]
    fn archive_texture_paths_keep_the_provider_conflict_guard() {
        // Only provider bytes matter to this resolver; image decoding belongs
        // to the original-asset integration test.
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        let entry = |name: &str, offset| crate::Resource {
            locator: format!("test.far::{name}"),
            name: name.into(),
            source: source.clone(),
            offset,
            size: 8,
            unpacked_size: 8,
            archived: true,
            owned: None,
        };
        let mut index = Index {
            root: source.parent().unwrap().to_owned(),
            resources: vec![entry("Skins\\Head.bmp", 0)],
            archives: 1,
            diagnostics: vec![],
        };
        assert_eq!(equivalent_resource(&index, "head.BMP").unwrap(), 0);
        index.resources.push(entry("Head.bmp", 0));
        assert_eq!(equivalent_resource(&index, "head.bmp").unwrap(), 0);
        index.resources.push(entry("Skins/HEAD.bmp", 1));
        assert!(
            equivalent_resource(&index, "head.bmp")
                .unwrap_err()
                .to_string()
                .contains("conflicting saved avatar providers")
        );
        assert!(equivalent_resource(&index, "absent.bmp").is_err());
    }

    #[test]
    #[ignore = "requires original Complete Collection mounted textures"]
    fn shared_accessory_texture_precedes_the_different_base_texture() -> Result<()> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
        let selected = equivalent_resource(&index, "brush.bmp")?;
        assert_eq!(
            index.resources[selected].locator,
            "ExpansionShared/ExpansionShared.far::Accessories\\brush.bmp"
        );
        let base = index
            .resources
            .iter()
            .find(|r| r.locator == "GameData_ALL_Recursive/Textures/Textures.far::brush.bmp")
            .context("base brush absent")?;
        let bytes = index.resources[selected].read()?;
        assert_ne!(bytes, base.read()?);
        assert_eq!(equivalent_resource(&index, "BRUSH.BMP")?, selected);
        let image = image::load_from_memory_with_format(&bytes, image::ImageFormat::Bmp)?;
        std::fs::write(
            root.join("cache/native-brush-texture-source.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "selected":index.resources[selected].locator,"selected_sha256":format!("{:x}",Sha256::digest(&bytes)),
                "base":base.locator,"base_sha256":format!("{:x}",Sha256::digest(base.read()?)),
                "dimensions":[image.width(),image.height()],"case_insensitive_same_source":true,
                "scope":"First plain BMP open in the native fixed leading texture paths over the explicit Complete Collection virtual installation; provider suffix and fallback remain outside this adapter."
            }))?,
        )?;
        Ok(())
    }
}

/// Asset-side result of native record binding. Handles are local to this avatar;
/// the caller supplies its owner identity. Geometry ranges retain record identity
/// instead of flattening away which body/hand/head/accessory supplied each mesh.
#[derive(Clone)]
pub struct BoundAvatar {
    pub avatar: Avatar,
    pub records: Records,
    pub part_ranges: BTreeMap<u64, std::ops::Range<usize>>,
    auxiliary: BTreeMap<u64, AuxiliaryBinding>,
    order: Vec<u64>,
    owner: u64,
    next_binding: u64,
    catalogs: BTreeMap<String, usize>,
    /// Derived lookup cache for this read-only installation. Like `catalogs`,
    /// it is rebuilt after restore and never substitutes for a failed read.
    missing_catalogs: std::collections::BTreeSet<String>,
}

/// Save binding identities/order without embedding decoded textures or meshes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BindingState {
    pub records: Records,
    pub order: Vec<u64>,
    pub next_binding: u64,
    #[serde(default)]
    pub auxiliary: BTreeMap<u64, AuxiliaryBinding>,
}

/// Direct 599427 bindings (animator +250), outside the 583559 record array.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuxiliaryBinding {
    pub name: String,
    pub mask: u32,
}

/// Geometry services occur inside the native record operation: the new binding
/// is installed before the old one is removed, and the final dirty write occurs
/// after the record's old pointer is cleared. Asset-only callers may omit them.
pub(crate) enum GeometryEvent<'a> {
    Attach {
        binding: Binding,
        catalog_id: usize,
        appearance: &'a avatar::Appearance,
        source_hash: &'a str,
        skeleton: &'a avatar::Skeleton,
        parts: &'a [Part],
        ordinals: &'a [usize],
    },
    Detach(Binding),
    Dirty(u64),
}

impl BoundAvatar {
    pub fn snapshot(&self) -> BindingState {
        BindingState {
            records: self.records.clone(),
            order: self.order.clone(),
            next_binding: self.next_binding,
            auxiliary: self.auxiliary.clone(),
        }
    }

    /// Rehydrate immutable assets in saved attachment order, preserving holes
    /// left by removed bindings and the next identity across save/resume.
    pub fn restore(
        index: &Index,
        skeleton: &str,
        owner: u64,
        state: &BindingState,
    ) -> Result<Self> {
        Self::restore_with_startup(index, skeleton, owner, state, None)
    }

    pub(crate) fn restore_with_startup(
        index: &Index,
        skeleton: &str,
        owner: u64,
        state: &BindingState,
        startup: Option<&mut crate::startup_memo::Memo>,
    ) -> Result<Self> {
        Self::restore_with_geometry(index, skeleton, owner, state, startup, |_| Ok(()))
    }

    /// The same immutable reconstruction, with the exact prototype keys used
    /// by live attachment. Retained mesh caches can bind these aliases without
    /// evaluating bounds, resetting generations or constructing new meshes.
    pub fn restore_with_mesh_keys(
        index: &Index,
        skeleton: &str,
        owner: u64,
        state: &BindingState,
    ) -> Result<(Self, BTreeMap<u64, Vec<String>>)> {
        let mut keys = BTreeMap::new();
        let avatar = Self::restore_with_geometry(index, skeleton, owner, state, None, |event| {
            if let GeometryEvent::Attach {
                binding,
                catalog_id,
                appearance,
                source_hash,
                ordinals,
                ..
            } = event
            {
                let values = ordinals
                    .iter()
                    .map(|&ordinal| {
                        crate::character_geometry::prototype_key(
                            &index.resources[catalog_id].locator,
                            source_hash,
                            &appearance.name,
                            ordinal,
                        )
                    })
                    .collect::<Result<_>>()?;
                ensure!(
                    keys.insert(binding.id, values).is_none(),
                    "duplicate retained mesh binding"
                );
            }
            Ok(())
        })?;
        Ok((avatar, keys))
    }

    fn restore_with_geometry(
        index: &Index,
        skeleton: &str,
        owner: u64,
        state: &BindingState,
        startup: Option<&mut crate::startup_memo::Memo>,
        mut geometry: impl FnMut(GeometryEvent<'_>) -> Result<()>,
    ) -> Result<Self> {
        let mut expected = BTreeMap::new();
        for (record, entry) in state.records.entries.iter().enumerate() {
            if let Some(binding) = entry.binding {
                ensure!(
                    binding.owner == owner && binding.id > 0 && binding.id < state.next_binding,
                    "invalid saved appearance binding identity"
                );
                ensure!(
                    expected.insert(binding.id, Some(record)).is_none(),
                    "duplicate appearance binding identity"
                );
            }
        }
        for &id in state.auxiliary.keys() {
            ensure!(
                id > 0 && id < state.next_binding && expected.insert(id, None).is_none(),
                "invalid or duplicate auxiliary binding identity"
            );
        }
        ensure!(
            state.next_binding > 0 && state.order.len() == expected.len(),
            "saved binding order length mismatch"
        );
        let mut bound = Self::empty_with_startup(index, skeleton, owner, startup)?;
        bound.records = state.records.clone();
        for entry in &mut bound.records.entries {
            entry.binding = None;
        }
        for &id in &state.order {
            let record = expected
                .remove(&id)
                .context("saved binding order is not a permutation")?;
            bound.next_binding = id;
            if let Some(record) = record {
                bound.apply_record_with_geometry(
                    index,
                    record,
                    Execution::bind(Input::Current),
                    &mut geometry,
                )?;
            } else {
                let auxiliary = &state.auxiliary[&id];
                bound.bind_auxiliary_with_geometry(
                    index,
                    &auxiliary.name,
                    auxiliary.mask,
                    &mut geometry,
                )?;
            }
        }
        bound.next_binding = state.next_binding;
        ensure!(
            bound.snapshot() == *state,
            "saved appearance reconstruction changed bindings"
        );
        Ok(bound)
    }

    pub fn empty(index: &Index, skeleton_name: &str, owner: u64) -> Result<Self> {
        Self::empty_with_startup(index, skeleton_name, owner, None)
    }

    pub(crate) fn empty_with_startup(
        index: &Index,
        skeleton_name: &str,
        owner: u64,
        startup: Option<&mut crate::startup_memo::Memo>,
    ) -> Result<Self> {
        let mut sources = BTreeMap::new();
        let skeleton = if skeleton_name.eq_ignore_ascii_case("root") {
            // Native 5a06d4 constructs this rig without a source catalog.
            sims_sim::pose::skeleton::root_source()
        } else {
            let (skeleton, id) =
                crate::avatar::load_skeleton_with_startup(index, skeleton_name, startup)?;
            // Preserve the decoded catalog's source/hash in saved appearance
            // provenance, as for the earlier single-catalog loader.
            read(index, id, &mut sources)?;
            skeleton
        };
        ensure!(
            skeleton.name.eq_ignore_ascii_case(skeleton_name),
            "avatar skeleton name mismatch"
        );
        Ok(Self {
            avatar: Avatar {
                skeleton,
                parts: vec![],
                sources,
            },
            records: Records::default(),
            part_ranges: BTreeMap::new(),
            auxiliary: BTreeMap::new(),
            order: Vec::new(),
            owner,
            next_binding: 1,
            catalogs: BTreeMap::new(),
            missing_catalogs: Default::default(),
        })
    }

    pub fn auxiliary(&self) -> &BTreeMap<u64, AuxiliaryBinding> {
        &self.auxiliary
    }

    fn resolve(&mut self, index: &Index, name: &str) -> Result<(usize, avatar::Appearance)> {
        self.resolve_record(index, name)?
            .with_context(|| format!("appearance {name} absent from installed catalogs"))
    }

    /// 5a044d returns null for a key absent from the appearance registry;
    /// 583559 then leaves the previous record untouched. Establish absence
    /// using the complete source-backed startup, not a catch-all decode error.
    fn resolve_record(
        &mut self,
        index: &Index,
        name: &str,
    ) -> Result<Option<(usize, avatar::Appearance)>> {
        if self.missing_catalogs.contains(name) {
            return Ok(None);
        }
        let key = name.to_ascii_lowercase();
        let catalog_id = if let Some(id) = self.catalogs.get(&key) {
            *id
        } else {
            let Some(id) = crate::avatar::appearance_catalog(index, name)? else {
                ensure!(
                    name.is_ascii() && !name.contains('\0'),
                    "native missing-appearance proof needs an ASCII counted name"
                );
                let (installed, _) = crate::clothing_catalog::installed_catalog(index)?;
                ensure!(
                    installed
                        .catalog
                        .get(
                            sims_formats::avatar::index::Kind::Appearance,
                            name.as_bytes()
                        )
                        .is_none(),
                    "appearance {name} is registered but unsupported by the descriptor resolver"
                );
                self.missing_catalogs.insert(name.into());
                return Ok(None);
            };
            self.catalogs.insert(key, id);
            id
        };
        let catalog = crate::avatar::read_catalog(
            &index.resources[catalog_id].name,
            &read(index, catalog_id, &mut self.avatar.sources)?,
        )?;
        let appearance = catalog
            .appearances
            .into_iter()
            .find(|a| a.name.eq_ignore_ascii_case(name))
            .with_context(|| format!("appearance {name} absent from catalog"))?;
        Ok(Some((catalog_id, appearance)))
    }

    fn attach(
        &mut self,
        index: &Index,
        catalog_id: usize,
        appearance: &avatar::Appearance,
        texture: &str,
        mask: u32,
        geometry: &mut impl FnMut(GeometryEvent<'_>) -> Result<()>,
    ) -> Result<Binding> {
        let (parts, ordinals) = decode_parts(
            index,
            catalog_id,
            appearance,
            texture,
            mask,
            &self.avatar.skeleton,
            &mut self.avatar.sources,
        )?;
        let handle = self.next_binding;
        let next = handle
            .checked_add(1)
            .context("appearance binding identity exhausted")?;
        let binding = Binding {
            id: handle,
            owner: self.owner,
        };
        geometry(GeometryEvent::Attach {
            binding,
            catalog_id,
            appearance,
            source_hash: self
                .avatar
                .sources
                .get(&index.resources[catalog_id].locator)
                .context("appearance provenance absent")?,
            skeleton: &self.avatar.skeleton,
            parts: &parts,
            ordinals: &ordinals,
        })?;
        let start = self.avatar.parts.len();
        self.avatar.parts.extend(parts);
        self.part_ranges
            .insert(handle, start..self.avatar.parts.len());
        self.order.push(handle);
        self.next_binding = next;
        Ok(binding)
    }

    fn detach(
        &mut self,
        binding: Binding,
        geometry: &mut impl FnMut(GeometryEvent<'_>) -> Result<()>,
    ) -> Result<()> {
        ensure!(binding.owner == self.owner, "appearance owner mismatch");
        let range = self
            .part_ranges
            .get(&binding.id)
            .context("appearance binding geometry absent")?
            .clone();
        geometry(GeometryEvent::Detach(binding))?;
        self.part_ranges.remove(&binding.id);
        self.order.retain(|&id| id != binding.id);
        self.avatar.parts.drain(range.clone());
        for remaining in self.part_ranges.values_mut() {
            if remaining.start >= range.end {
                remaining.start -= range.len();
                remaining.end -= range.len();
            }
        }
        Ok(())
    }

    pub(crate) fn bind_auxiliary_with_geometry(
        &mut self,
        index: &Index,
        name: &str,
        mask: u32,
        mut geometry: impl FnMut(GeometryEvent<'_>) -> Result<()>,
    ) -> Result<u64> {
        let (catalog, appearance) = self.resolve(index, name)?;
        let binding = self.attach(index, catalog, &appearance, "", mask, &mut geometry)?;
        self.auxiliary.insert(
            binding.id,
            AuxiliaryBinding {
                name: name.into(),
                mask,
            },
        );
        Ok(binding.id)
    }

    pub(crate) fn detach_auxiliary_with_geometry(
        &mut self,
        id: u64,
        mut geometry: impl FnMut(GeometryEvent<'_>) -> Result<()>,
    ) -> Result<()> {
        ensure!(self.auxiliary.contains_key(&id), "auxiliary binding absent");
        self.detach(
            Binding {
                id,
                owner: self.owner,
            },
            &mut geometry,
        )?;
        self.auxiliary.remove(&id);
        Ok(())
    }

    /// Drive the independently verified 583559/5835c9 record operation. New
    /// geometry is decoded before removing the previous binding. Returns the
    /// skeleton owners whose geometry the caller must invalidate, in order.
    /// Host errors remain errors; they are not converted into missing assets.
    pub fn apply_record(
        &mut self,
        index: &Index,
        id: usize,
        execution: Execution,
    ) -> Result<Vec<u64>> {
        let mut dirty = vec![];
        self.apply_record_with_geometry(index, id, execution, |event| {
            if let GeometryEvent::Dirty(owner) = event {
                dirty.push(owner);
            }
            Ok(())
        })?;
        Ok(dirty)
    }

    pub(crate) fn apply_record_with_geometry(
        &mut self,
        index: &Index,
        id: usize,
        mut execution: Execution,
        mut geometry: impl FnMut(GeometryEvent<'_>) -> Result<()>,
    ) -> Result<()> {
        ensure!(id < self.records.entries.len(), "appearance record absent");
        let mut resolved: Option<(usize, avatar::Appearance)> = None;
        while let Some(request) = execution.next(&mut self.records.entries[id]) {
            let response = match request {
                Request::Lookup { name } => {
                    resolved = self.resolve_record(index, &name)?;
                    Response::Asset(resolved.as_ref().map(|(catalog_id, _)| *catalog_id as u64))
                }
                Request::Bind { asset, texture } => {
                    let (catalog_id, appearance) =
                        resolved.take().context("appearance bind before lookup")?;
                    ensure!(
                        asset == catalog_id as u64,
                        "appearance asset identity changed"
                    );
                    let binding = self.attach(
                        index,
                        catalog_id,
                        &appearance,
                        texture.as_deref().unwrap_or(""),
                        u32::MAX,
                        &mut geometry,
                    )?;
                    Response::Binding(Some(binding))
                }
                Request::Unbind { binding } => {
                    self.detach(binding, &mut geometry)?;
                    Response::Done
                }
                Request::Dirty { owner } => {
                    geometry(GeometryEvent::Dirty(owner))?;
                    Response::Done
                }
            };
            execution
                .acknowledge(&mut self.records.entries[id], response)
                .map_err(anyhow::Error::msg)?;
        }
        ensure!(
            execution.result().is_some(),
            "appearance binding incomplete"
        );
        Ok(())
    }
}

fn read(index: &Index, id: usize, sources: &mut BTreeMap<String, String>) -> Result<Vec<u8>> {
    let r = &index.resources[id];
    let bytes = r.read()?;
    sources.insert(r.locator.clone(), format!("{:x}", Sha256::digest(&bytes)));
    Ok(bytes)
}

fn decode_parts(
    index: &Index,
    catalog_id: usize,
    appearance: &avatar::Appearance,
    override_: &str,
    mask: u32,
    skeleton: &avatar::Skeleton,
    sources: &mut BTreeMap<String, String>,
) -> Result<(Vec<Part>, Vec<usize>)> {
    let name = &appearance.name;
    ensure!(
        appearance.kind <= 1,
        "appearance {name} has an unsupported kind"
    );
    let mut parts = vec![];
    let mut ordinals = vec![];
    for (ordinal, binding) in appearance.bindings.iter().enumerate() {
        if !sims_sim::pose::appearance::binding_selected(binding.censor, mask)
            || !skeleton
                .bones
                .iter()
                .any(|b| b.name.eq_ignore_ascii_case(&binding.bone))
        {
            continue;
        }
        let mesh_id = crate::avatar::mesh_resource(index, catalog_id, &binding.mesh)?;
        let mesh = crate::avatar::read_mesh(
            &index.resources[mesh_id].name,
            &read(index, mesh_id, sources)?,
        )?;
        for bone in mesh.bones.iter().chain(std::iter::once(&binding.bone)) {
            ensure!(
                skeleton
                    .bones
                    .iter()
                    .any(|b| b.name.eq_ignore_ascii_case(bone)),
                "appearance {name} references absent bone {bone}"
            );
        }
        let selected = texture_override(&mesh.name, override_);
        let texture = if selected.is_empty() {
            &mesh.texture
        } else {
            selected
        };
        ensure!(!texture.is_empty(), "appearance {name} has no texture");
        if let Some(material) = crate::avatar_material::builtin(texture) {
            parts.push(Part {
                attachment: binding.bone.clone(),
                censor: appearance.kind == 1,
                texture: material.texture(),
                builtin_material: Some(material),
                mesh,
            });
            ordinals.push(ordinal);
            continue;
        }
        let texture_id = equivalent_resource(index, &format!("{texture}.bmp"))?;
        let mut reader = image::ImageReader::with_format(
            Cursor::new(read(index, texture_id, sources)?),
            image::ImageFormat::Bmp,
        );
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(2048);
        limits.max_image_height = Some(2048);
        limits.max_alloc = Some(32 * 1024 * 1024);
        reader.limits(limits);
        let image = reader.decode()?.to_rgba8();
        parts.push(Part {
            attachment: binding.bone.clone(),
            builtin_material: None,
            censor: appearance.kind == 1,
            mesh,
            texture: Texture {
                width: image.width(),
                height: image.height(),
                rgba: image.into_raw(),
            },
        });
        ordinals.push(ordinal);
    }
    Ok((parts, ordinals))
}

/// Present the saved body, optional extra layer, hands, head and accessories.
/// The zero owner is a standalone preview identity, not a household object.
pub fn load(index: &Index, skeleton_name: &str, saved: &Person) -> Result<Avatar> {
    Ok(load_bound(index, skeleton_name, saved, 0)?.avatar)
}

/// Consume record lookup/bind requests in 5918ba's serialized order. Asset
/// decoding errors remain errors; no missing referenced resource is replaced.
/// This does not execute appearance selection, the rig initializer or load tail.
pub fn load_bound(
    index: &Index,
    skeleton_name: &str,
    saved: &Person,
    owner: u64,
) -> Result<BoundAvatar> {
    let mut bound = BoundAvatar::empty(index, skeleton_name, owner)?;
    bound.records = Records::from_saved(saved);
    let order: Vec<_> = bound
        .records
        .saved_order()
        .into_iter()
        .chain(bound.records.accessories.iter().copied())
        .collect();
    for id in order {
        let dirty = bound.apply_record(index, id, Execution::bind(Input::Current))?;
        ensure!(
            dirty.is_empty(),
            "fresh saved appearance unexpectedly replaced a binding"
        );
    }
    Ok(bound)
}
