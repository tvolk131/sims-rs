//! High-level `.antics` v2 → `.air` (Antics IR) + `ObjectManifest` lowering compiler.
//!
//! Performs template expansion, parameter validation, state-to-attribute allocation,
//! context safety (`self` vs `sim`), automatic `stack_obj` preservation, hygienic
//! helper inlining with `&mut` references, `with lock(...)`, LIFO `defer { ... }`,
//! `repeat ... cancel_on_abort`, and short-circuit boolean/return lowering.

use crate::art::FurnitureShape;
use crate::error::{Error, Result, Span};
use crate::manifest::{
    CallbackSpec, InteractionSpec, MotiveAdSpec, ObjectManifest, SlotSpec, SoundSpec, SpriteSpec,
    TileSpec, TreeRef,
};
use crate::parser::{motive_by_name, skill_by_name};
use crate::v2::ast::{
    CallArg, ConstIntExpr, FnDecl, ForEachIter, FunctionalDecl, GuardBody, GuardDecl,
    InteractionDecl, MetaField, MetaFieldKind, ObjectDecl, ObjectMember, ParamType, ParamValue,
    SlotDecl, SoundDecl, SpriteDecl, StateDecl, StateInitValue, StateType, StringsDecl,
    TemplateDecl, TileDecl, V2AssignOp, V2BinOp, V2Expr, V2Stmt, V2Type, V2UnaryOp,
};
use crate::v2::loader::{ModuleScope, ResolvedModule};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

const MAX_INLINE_DEPTH: usize = 16;
const MAX_INLINE_EXPANSIONS: usize = 256;

/// Output of expanding and lowering one concrete v2 `.antics` `object` declaration.
#[derive(Debug, Clone)]
pub struct ExpandedV2Object {
    pub name: String,
    pub slug: String,
    pub guid: u32,
    pub manifest: ObjectManifest,
    /// Complete lowered `.air` (Antics IR) source text ready for `compile_air` / `sims-antics expand`.
    pub air_source: String,
}

/// Lowers every concrete `object` in a [`ResolvedModule`] into `.air` + [`ObjectManifest`].
pub fn lower_resolved_module(resolved: &ResolvedModule) -> Result<Vec<ExpandedV2Object>> {
    if resolved.objects.is_empty() {
        return Err(Error::new(format!(
            "module '{}' declares no 'object' items",
            resolved.module_id
        )));
    }
    let mut out = Vec::with_capacity(resolved.objects.len());
    for obj in &resolved.objects {
        out.push(lower_object(resolved, obj)?);
    }
    Ok(out)
}

#[derive(Debug, Clone)]
struct OwnedStmtBlock {
    owner_module: String,
    stmts: Vec<V2Stmt>,
}

#[derive(Debug, Clone)]
struct OwnedInteraction {
    owner_module: String,
    decl: InteractionDecl,
}

#[derive(Debug, Clone)]
struct OwnedFunctional {
    owner_module: String,
    decl: FunctionalDecl,
}

#[derive(Debug, Clone)]
struct ResolvedState {
    attr_index: usize,
    ty: StateType,
    init: Option<StateInitValue>,
    span: Span,
}

#[derive(Debug, Clone)]
struct ResolvedSound {
    id: u16,
}

/// Flattened, validated object environment ready to emit `.air` and `ObjectManifest`.
struct ObjectEnv<'a> {
    resolved: &'a ResolvedModule,
    name: String,
    i16_params: BTreeMap<String, i16>,
    i16_param_order: Vec<String>,
    bool_params: BTreeMap<String, bool>,
    states: BTreeMap<String, ResolvedState>,
    state_order: Vec<String>,
    sounds: BTreeMap<String, ResolvedSound>,
    /// Slot declarations paired with their routing-only ordinal (`Some(idx)` for `kind == 3`).
    slots: BTreeMap<String, (Option<u16>, SlotDecl)>,
    strings: BTreeMap<u16, StringsDecl>,
    string_names: BTreeMap<String, u16>,
    local_fns: BTreeMap<String, (String, FnDecl)>,
}

impl<'a> ObjectEnv<'a> {
    fn resolve_const_int(&self, expr: &ConstIntExpr, span: Span) -> Result<i16> {
        match expr {
            ConstIntExpr::Literal(v) => Ok(*v),
            ConstIntExpr::Param { name, negate } => {
                let Some(&val) = self.i16_params.get(name) else {
                    if self.bool_params.contains_key(name) {
                        return Err(Error::at(
                            span,
                            format!("bool param '{name}' cannot be used as an i16 constant"),
                        ));
                    }
                    return Err(Error::at(span, format!("unknown i16 parameter '{name}'")));
                };
                if *negate {
                    val.checked_neg().ok_or_else(|| {
                        Error::at(span, format!("negating param '{name}' overflows i16"))
                    })
                } else {
                    Ok(val)
                }
            }
        }
    }

    fn lookup_fn(
        &self,
        owner_module: &str,
        name: &str,
        allow_local_fns: bool,
    ) -> Option<(String, FnDecl, bool)> {
        if allow_local_fns && let Some((m, decl)) = self.local_fns.get(name) {
            return Some((m.clone(), decl.clone(), true));
        }
        self.resolved
            .module_scopes
            .get(owner_module)
            .and_then(|s: &ModuleScope| s.functions.get(name))
            .map(|(m, decl)| (m.clone(), decl.clone(), false))
    }
}

fn lower_object(resolved: &ResolvedModule, obj: &ObjectDecl) -> Result<ExpandedV2Object> {
    // 1. Resolve optional `from Template`.
    let template_opt: Option<(String, TemplateDecl)> = match &obj.from_template {
        Some((tpl_name, tspan)) => {
            let found = resolved
                .templates
                .get(tpl_name)
                .cloned()
                .ok_or_else(|| Error::at(*tspan, format!("unknown template '{tpl_name}'")))?;
            Some(found)
        }
        None => None,
    };

    let slug = make_slug(&obj.name);

    // 2. Merge metadata, params, states, sounds, slots, strings, hooks, interactions, functionals, local fns.
    let mut guid: Option<u32> = None;
    let mut price: u16 = 100;
    let mut description = String::new();
    let mut catalog_sort_flags: u16 = 0;
    let mut object_type: u16 = 4;
    let mut objd_id: u16 = 128;
    let mut ttab_id: u16 = 128;
    let mut slot_id: u16 = 128;
    let mut ctss_id: u16 = 2000;
    let mut dgrp_id: u16 = 100;
    let mut spr2_id: u16 = 100;
    let mut palt_id: u16 = 100;
    let mut master_id: Option<u16> = None;
    let mut interaction_group: i16 = 0;
    let mut data_58: i16 = 0;
    let mut explicit_attributes: u16 = 0;
    let mut sprite: Option<SpriteDecl> = None;
    let mut graphics: Vec<SpriteDecl> = Vec::new();
    let mut tiles: Vec<TileDecl> = Vec::new();

    struct ParamSlot {
        span: Span,
        ty: ParamType,
        default: Option<ParamValue>,
    }
    let mut declared_params: BTreeMap<String, ParamSlot> = BTreeMap::new();
    let mut param_order: Vec<String> = Vec::new();
    let mut param_assigns: BTreeMap<String, (Span, ParamValue)> = BTreeMap::new();

    let mut raw_states: BTreeMap<String, StateDecl> = BTreeMap::new();
    let mut state_order: Vec<String> = Vec::new();

    let mut raw_sounds: BTreeMap<String, SoundDecl> = BTreeMap::new();
    let mut sound_order: Vec<String> = Vec::new();

    let mut raw_slots: BTreeMap<String, SlotDecl> = BTreeMap::new();
    let mut slot_order: Vec<String> = Vec::new();

    let mut strings: BTreeMap<u16, StringsDecl> = BTreeMap::new();
    let mut string_names: BTreeMap<String, u16> = BTreeMap::new();

    let mut init_blocks: Vec<OwnedStmtBlock> = Vec::new();
    let mut init_pinned_id: Option<u16> = None;
    let mut main_block: Option<OwnedStmtBlock> = None;
    let mut main_pinned_id: Option<u16> = None;

    let mut functionals: Vec<OwnedFunctional> = Vec::new();
    let mut interactions: Vec<OwnedInteraction> = Vec::new();
    let mut local_fns: BTreeMap<String, (String, FnDecl)> = BTreeMap::new();

    let mut apply_members = |owner_mod: &str,
                             members: &[ObjectMember],
                             is_override: bool|
     -> Result<()> {
        let mut seen_sounds_here = BTreeSet::new();
        let mut seen_slots_here = BTreeSet::new();
        let mut seen_functionals_here = BTreeSet::new();
        let mut seen_interactions_here = BTreeSet::new();
        let mut seen_fns_here = BTreeSet::new();
        for member in members {
            match member {
                ObjectMember::Meta(MetaField { span, kind }) => match kind {
                    MetaFieldKind::Guid(v) => guid = Some(*v),
                    MetaFieldKind::Price(v) => price = *v,
                    MetaFieldKind::Description(s) => description = s.clone(),
                    MetaFieldKind::Category(v) => catalog_sort_flags = *v,
                    MetaFieldKind::ObjectType(v) => object_type = *v,
                    MetaFieldKind::ObjdId(v) => objd_id = *v,
                    MetaFieldKind::TtabId(v) => ttab_id = *v,
                    MetaFieldKind::SlotId(v) => slot_id = *v,
                    MetaFieldKind::CtssId(v) => ctss_id = *v,
                    MetaFieldKind::DgrpId(v) => dgrp_id = *v,
                    MetaFieldKind::Spr2Id(v) => spr2_id = *v,
                    MetaFieldKind::PaltId(v) => palt_id = *v,
                    MetaFieldKind::MasterId(v) => master_id = Some(*v),
                    MetaFieldKind::InteractionGroup(v) => interaction_group = *v,
                    MetaFieldKind::Data58(v) => data_58 = *v,
                    MetaFieldKind::Attributes(v) => explicit_attributes = *v,
                    MetaFieldKind::Sprite(sp) => sprite = Some(sp.clone()),
                    MetaFieldKind::Graphics(list) => {
                        if list.is_empty() {
                            return Err(Error::at(*span, "'graphics' list cannot be empty"));
                        }
                        graphics = list.clone();
                    }
                    MetaFieldKind::Tiles(t) => tiles = t.clone(),
                },
                ObjectMember::Param(p) => {
                    if declared_params.contains_key(&p.name) {
                        return Err(Error::at(
                            p.span,
                            format!("duplicate parameter declaration '{}'", p.name),
                        ));
                    }
                    param_order.push(p.name.clone());
                    declared_params.insert(
                        p.name.clone(),
                        ParamSlot {
                            span: p.span,
                            ty: p.ty,
                            default: p.default,
                        },
                    );
                }
                ObjectMember::ParamAssign(pa) => {
                    if !is_override {
                        return Err(Error::at(
                            pa.span,
                            format!(
                                "template cannot assign parameter '{}' directly; declare 'param {}: ... = ...;' instead",
                                pa.name, pa.name
                            ),
                        ));
                    }
                    if param_assigns.contains_key(&pa.name) {
                        return Err(Error::at(
                            pa.span,
                            format!("duplicate parameter assignment '{}'", pa.name),
                        ));
                    }
                    param_assigns.insert(pa.name.clone(), (pa.span, pa.value));
                }
                ObjectMember::State(s) => {
                    if raw_states.contains_key(&s.name) {
                        return Err(Error::at(
                            s.span,
                            format!("duplicate state declaration '{}'", s.name),
                        ));
                    }
                    state_order.push(s.name.clone());
                    raw_states.insert(s.name.clone(), s.clone());
                }
                ObjectMember::Sound(s) => {
                    if !seen_sounds_here.insert(s.name.clone()) {
                        return Err(Error::at(
                            s.span,
                            format!("duplicate sound declaration '{}'", s.name),
                        ));
                    }
                    if let Some(existing) = raw_sounds.get_mut(&s.name) {
                        if !is_override {
                            return Err(Error::at(
                                s.span,
                                format!("duplicate sound declaration '{}'", s.name),
                            ));
                        }
                        *existing = s.clone();
                    } else {
                        sound_order.push(s.name.clone());
                        raw_sounds.insert(s.name.clone(), s.clone());
                    }
                }
                ObjectMember::Slot(s) => {
                    if !seen_slots_here.insert(s.name.clone()) {
                        return Err(Error::at(
                            s.span,
                            format!("duplicate slot declaration '{}'", s.name),
                        ));
                    }
                    if let Some(existing) = raw_slots.get_mut(&s.name) {
                        if !is_override {
                            return Err(Error::at(
                                s.span,
                                format!("duplicate slot declaration '{}'", s.name),
                            ));
                        }
                        *existing = s.clone();
                    } else {
                        slot_order.push(s.name.clone());
                        raw_slots.insert(s.name.clone(), s.clone());
                    }
                }
                ObjectMember::Strings(st) => {
                    if strings.contains_key(&st.id) {
                        return Err(Error::at(
                            st.span,
                            format!("duplicate strings table id {}", st.id),
                        ));
                    }
                    for (espan, ename, _) in &st.entries {
                        if string_names.insert(ename.clone(), st.id).is_some() {
                            return Err(Error::at(
                                *espan,
                                format!("duplicate string entry name '{ename}'"),
                            ));
                        }
                    }
                    strings.insert(st.id, st.clone());
                }
                ObjectMember::Init(b) => {
                    if let Some(pid) = b.pinned_id {
                        init_pinned_id = Some(pid);
                    }
                    init_blocks.push(OwnedStmtBlock {
                        owner_module: owner_mod.to_owned(),
                        stmts: b.body.clone(),
                    });
                }
                ObjectMember::Main(b) => {
                    if let Some(pid) = b.pinned_id {
                        main_pinned_id = Some(pid);
                    }
                    main_block = Some(OwnedStmtBlock {
                        owner_module: owner_mod.to_owned(),
                        stmts: b.body.clone(),
                    });
                }
                ObjectMember::Functional(f) => {
                    if !seen_functionals_here.insert(f.kind.clone()) {
                        return Err(Error::at(
                            f.span,
                            format!("duplicate functional '{}'", f.kind),
                        ));
                    }
                    if let Some(pos) = functionals.iter().position(|x| x.decl.kind == f.kind) {
                        if !is_override {
                            return Err(Error::at(
                                f.span,
                                format!("duplicate functional '{}'", f.kind),
                            ));
                        }
                        functionals[pos] = OwnedFunctional {
                            owner_module: owner_mod.to_owned(),
                            decl: f.clone(),
                        };
                    } else {
                        functionals.push(OwnedFunctional {
                            owner_module: owner_mod.to_owned(),
                            decl: f.clone(),
                        });
                    }
                }
                ObjectMember::Interaction(inter) => {
                    if !seen_interactions_here.insert(inter.label.clone()) {
                        return Err(Error::at(
                            inter.span,
                            format!("duplicate interaction '{}'", inter.label),
                        ));
                    }
                    if let Some(pos) = interactions
                        .iter()
                        .position(|x| x.decl.label == inter.label)
                    {
                        if !is_override {
                            return Err(Error::at(
                                inter.span,
                                format!("duplicate interaction '{}'", inter.label),
                            ));
                        }
                        interactions[pos] = OwnedInteraction {
                            owner_module: owner_mod.to_owned(),
                            decl: inter.clone(),
                        };
                    } else {
                        interactions.push(OwnedInteraction {
                            owner_module: owner_mod.to_owned(),
                            decl: inter.clone(),
                        });
                    }
                }
                ObjectMember::Fn(f) => {
                    if !seen_fns_here.insert(f.name.clone())
                        || (local_fns.contains_key(&f.name) && !is_override)
                    {
                        return Err(Error::at(
                            f.span,
                            format!("duplicate helper function '{}'", f.name),
                        ));
                    }
                    local_fns.insert(f.name.clone(), (owner_mod.to_owned(), f.clone()));
                }
            }
        }
        Ok(())
    };

    if let Some((tpl_mod, tpl_decl)) = &template_opt {
        apply_members(tpl_mod, &tpl_decl.members, false)?;
    }
    apply_members(&resolved.module_id, &obj.members, true)?;

    // 3. Validate GUID & resolve all parameters (checking unknown, type mismatch, and missing required).
    let guid = guid.ok_or_else(|| {
        Error::at(
            obj.span,
            format!(
                "object '{}' is missing required 'guid: 0x...;' field",
                obj.name
            ),
        )
    })?;

    for (aname, (aspan, aval)) in &param_assigns {
        let Some(slot) = declared_params.get(aname) else {
            return Err(Error::at(
                *aspan,
                format!("unknown parameter '{aname}' in object '{}'", obj.name),
            ));
        };
        match (slot.ty, aval) {
            (ParamType::I16, ParamValue::I16(_)) | (ParamType::Bool, ParamValue::Bool(_)) => {}
            (ParamType::I16, ParamValue::Bool(_)) => {
                return Err(Error::at(
                    *aspan,
                    format!("parameter '{aname}' expects i16, found bool"),
                ));
            }
            (ParamType::Bool, ParamValue::I16(_)) => {
                return Err(Error::at(
                    *aspan,
                    format!("parameter '{aname}' expects bool, found integer"),
                ));
            }
        }
    }

    let mut i16_params = BTreeMap::new();
    let mut i16_param_order = Vec::new();
    let mut bool_params = BTreeMap::new();
    for pname in &param_order {
        let slot = &declared_params[pname];
        let val = match param_assigns.get(pname) {
            Some((_, v)) => *v,
            None => slot.default.ok_or_else(|| {
                Error::at(
                    obj.span,
                    format!(
                        "object '{}' is missing required parameter '{}' (declared at {})",
                        obj.name, pname, slot.span
                    ),
                )
            })?,
        };
        match val {
            ParamValue::I16(v) => {
                i16_param_order.push(pname.clone());
                i16_params.insert(pname.clone(), v);
            }
            ParamValue::Bool(b) => {
                bool_params.insert(pname.clone(), b);
            }
        }
    }
    if i16_param_order.len() > 128 {
        return Err(Error::at(
            obj.span,
            format!(
                "object '{}' declares {} i16 params, exceeding BCON 4096 limit of 128",
                obj.name,
                i16_param_order.len()
            ),
        ));
    }

    // 4. Allocate state attribute indices `0..N`.
    let mut states = BTreeMap::new();
    for (idx, sname) in state_order.iter().enumerate() {
        let s = &raw_states[sname];
        states.insert(
            sname.clone(),
            ResolvedState {
                attr_index: idx,
                ty: s.ty,
                init: s.init.clone(),
                span: s.span,
            },
        );
    }

    // 5. Assign FWAV sound IDs (`1..`, honoring explicit IDs).
    let mut used_sound_ids: BTreeSet<u16> = BTreeSet::new();
    for sname in &sound_order {
        if let Some(eid) = raw_sounds[sname].explicit_id {
            if eid == 0 {
                return Err(Error::at(
                    raw_sounds[sname].span,
                    format!("sound '{sname}' id must be >= 1"),
                ));
            }
            if !used_sound_ids.insert(eid) {
                return Err(Error::at(
                    raw_sounds[sname].span,
                    format!("duplicate sound id {eid}"),
                ));
            }
        }
    }
    let mut next_sound_id = 1u16;
    let mut sounds = BTreeMap::new();
    let mut sound_specs = Vec::with_capacity(sound_order.len());
    for sname in &sound_order {
        let s = &raw_sounds[sname];
        let id = if let Some(eid) = s.explicit_id {
            eid
        } else {
            while used_sound_ids.contains(&next_sound_id) {
                next_sound_id = next_sound_id
                    .checked_add(1)
                    .ok_or_else(|| Error::at(s.span, "FWAV sound id overflow"))?;
            }
            let assigned = next_sound_id;
            used_sound_ids.insert(assigned);
            next_sound_id = next_sound_id.saturating_add(1);
            assigned
        };
        let wav_name = s
            .wav_name
            .clone()
            .unwrap_or_else(|| format!("{}_{}", slug.replace('-', "_"), sname));
        crate::manifest::validate_relative_filename(&wav_name, "sound wav name")
            .map_err(|e| Error::at(s.span, e.message))?;
        sounds.insert(sname.clone(), ResolvedSound { id });
        sound_specs.push(SoundSpec {
            id,
            name: wav_name,
            preset: s.preset,
        });
    }

    // 6. Build slot table (routing_ordinal counts ONLY kind == 3 routing slots, matching sims-vm).
    let mut slots = BTreeMap::new();
    let mut slot_specs = Vec::with_capacity(slot_order.len());
    let mut routing_ordinal = 0u16;
    for sl_name in &slot_order {
        let sl = raw_slots[sl_name].clone();
        let routing_idx = if sl.kind == 3 {
            let idx = routing_ordinal;
            routing_ordinal = routing_ordinal
                .checked_add(1)
                .ok_or_else(|| Error::at(sl.span, "routing slot count overflow"))?;
            Some(idx)
        } else {
            None
        };
        slot_specs.push(SlotSpec {
            kind: sl.kind,
            offset: sl.offset,
            standing: sl.standing,
            sitting: sl.sitting,
            ground: sl.ground,
            flags: sl.flags,
            snap_target: sl.snap_target,
            proximity: sl.proximity,
            max_size: sl.max_size,
            gradient: sl.gradient,
            height: sl.height,
            facing: sl.facing,
            resolution: sl.resolution,
        });
        slots.insert(sl_name.clone(), (routing_idx, sl));
    }

    let env = ObjectEnv {
        resolved,
        name: obj.name.clone(),
        i16_params,
        i16_param_order,
        bool_params,
        states,
        state_order,
        sounds,
        slots,
        strings,
        string_names,
        local_fns,
    };

    // 7. Allocate deterministic private BHAV IDs (`4096..=8191`), respecting `@id` / `@guard_id`.
    let mut pinned_ids = BTreeSet::new();
    let mut register_pinned = |id: u16, span: Span| -> Result<()> {
        if !(4096..=8191).contains(&id) {
            return Err(Error::at(
                span,
                format!("@id({id}) must be in private BHAV range 4096..=8191"),
            ));
        }
        if !pinned_ids.insert(id) {
            return Err(Error::at(span, format!("duplicate pinned BHAV @id({id})")));
        }
        Ok(())
    };
    if let Some(id) = init_pinned_id {
        register_pinned(id, obj.span)?;
    }
    if let Some(id) = main_pinned_id {
        register_pinned(id, obj.span)?;
    }
    for f in &functionals {
        if let Some(id) = f.decl.pinned_guard_id {
            register_pinned(id, f.decl.span)?;
        }
        if let Some(id) = f.decl.pinned_id {
            register_pinned(id, f.decl.span)?;
        }
    }
    for inter in &interactions {
        if let Some(id) = inter.decl.pinned_guard_id {
            register_pinned(id, inter.decl.span)?;
        }
        if let Some(id) = inter.decl.pinned_id {
            register_pinned(id, inter.decl.span)?;
        }
    }

    let mut used_tree_ids = pinned_ids;
    let mut next_tree_id = 4096u16;
    let mut alloc_tree_id = |pinned: Option<u16>,
                             preferred: Option<u16>,
                             span: Span|
     -> Result<u16> {
        if let Some(id) = pinned {
            return Ok(id);
        }
        if let Some(pref) = preferred
            && !used_tree_ids.contains(&pref)
        {
            used_tree_ids.insert(pref);
            return Ok(pref);
        }
        while used_tree_ids.contains(&next_tree_id) {
            next_tree_id = next_tree_id
                .checked_add(1)
                .filter(|&id| id <= 8191)
                .ok_or_else(|| Error::at(span, "exhausted private BHAV ID range 4096..=8191"))?;
        }
        let id = next_tree_id;
        used_tree_ids.insert(id);
        next_tree_id += 1;
        Ok(id)
    };

    let init_id = alloc_tree_id(init_pinned_id, Some(4096), obj.span)?;
    let main_id = alloc_tree_id(main_pinned_id, Some(4097), obj.span)?;

    // 8. Emit `.air` (Antics IR) text deterministically!
    let mut air = String::new();
    let _ = writeln!(
        air,
        "// Generated .air (Antics IR) from .antics v2 object \"{}\" (GUID {guid:#010x})",
        escape_str(&env.name)
    );
    if !env.state_order.is_empty() {
        air.push_str("// Attributes: ");
        for (i, sname) in env.state_order.iter().enumerate() {
            if i > 0 {
                air.push_str(", ");
            }
            let _ = write!(air, "attr[{i}] = {sname}");
        }
        air.push('\n');
    }
    air.push('\n');

    // 8a. BCON 4096 from i16 params.
    if !env.i16_param_order.is_empty() {
        let _ = writeln!(air, "bcon 4096 \"{}Tuning\" {{", camel_label(&env.name));
        for pname in &env.i16_param_order {
            let val = env.i16_params[pname];
            let _ = writeln!(air, "    {pname} = {val},");
        }
        air.push_str("}\n\n");
    }

    // 8b. STR# tables from `strings` blocks.
    for st in env.strings.values() {
        let lbl = st
            .label
            .clone()
            .unwrap_or_else(|| format!("{}Strings_{}", camel_label(&env.name), st.id));
        let _ = writeln!(air, "str {} \"{}\" {{", st.id, escape_str(&lbl));
        for (_, ename, text) in &st.entries {
            let _ = writeln!(air, "    {ename} = \"{}\",", escape_str(text));
        }
        air.push_str("}\n\n");
    }

    // 8c. Init tree.
    {
        let mut ctx = TreeLowerer::new(&env, TreeContext::ObjectSelf, &resolved.module_id, false);
        // Set functional score data words first (e.g., `my.prep_score := ...`).
        for func in &functionals {
            if let Some(score_expr) = &func.decl.score {
                let (_, score_word) = functional_kind_info(&func.decl.kind, func.decl.kind_span)?;
                let Some(word_idx) = score_word else {
                    return Err(Error::at(
                        func.decl.span,
                        format!(
                            "functional '{}' uses a fixed score in Sims 1 and cannot set a score data word",
                            func.decl.kind
                        ),
                    ));
                };
                let rhs = match score_expr {
                    ConstIntExpr::Literal(v) => v.to_string(),
                    ConstIntExpr::Param { name, negate } => {
                        let resolved_v = env.resolve_const_int(score_expr, func.decl.span)?;
                        if *negate {
                            resolved_v.to_string()
                        } else {
                            format!("bcon.{name}")
                        }
                    }
                };
                ctx.line(format!("my.data[{word_idx}] := {rhs};"));
            }
        }
        // Auto-initialize all declared states in declaration order.
        for sname in &env.state_order {
            let st = &env.states[sname];
            let rhs = match &st.init {
                None => "0".to_owned(),
                Some(StateInitValue::Int(v)) => v.to_string(),
                Some(StateInitValue::Bool(b)) => if *b { "1" } else { "0" }.to_owned(),
                Some(StateInitValue::Param(pname)) => {
                    if env.i16_params.contains_key(pname) {
                        if st.ty == StateType::Bool {
                            return Err(Error::at(
                                st.span,
                                format!(
                                    "bool state '{sname}' cannot initialize from i16 param '{pname}'"
                                ),
                            ));
                        }
                        format!("bcon.{pname}")
                    } else if let Some(&bval) = env.bool_params.get(pname) {
                        if st.ty != StateType::Bool {
                            return Err(Error::at(
                                st.span,
                                format!(
                                    "state '{sname}' cannot initialize from bool param '{pname}'"
                                ),
                            ));
                        }
                        if bval { "1" } else { "0" }.to_owned()
                    } else {
                        return Err(Error::at(
                            st.span,
                            format!(
                                "state '{sname}' initializer references unknown param '{pname}'"
                            ),
                        ));
                    }
                }
            };
            ctx.line(format!("my.attr[{}] := {rhs};", st.attr_index));
        }
        // Append template + object `init { ... }` hooks, each in its own lexical/cleanup scope.
        for blk in &init_blocks {
            ctx.with_owner_module(&blk.owner_module, |c| {
                c.push_scope(ScopeFrame::block());
                let r = c.lower_stmts(&blk.stmts);
                let p = if stmts_always_return(&blk.stmts) {
                    c.pop_scope_discard_cleanups();
                    Ok(())
                } else {
                    c.pop_scope_and_run_cleanups()
                };
                r.and(p)
            })?;
        }
        if !init_blocks
            .last()
            .is_some_and(|b| stmts_always_return(&b.stmts))
        {
            ctx.emit_all_cleanups()?;
            ctx.line("return true;".to_owned());
        }
        let _ = writeln!(air, "tree {init_id} \"Init\" {{");
        air.push_str(&ctx.finish());
        air.push_str("}\n\n");
    }

    // 8d. Main tree.
    {
        let mut ctx = TreeLowerer::new(&env, TreeContext::ObjectSelf, &resolved.module_id, false);
        if let Some(blk) = &main_block {
            ctx.with_owner_module(&blk.owner_module, |c| {
                c.push_scope(ScopeFrame::block());
                let r = c.lower_stmts(&blk.stmts);
                let p = if stmts_always_return(&blk.stmts) {
                    c.pop_scope_discard_cleanups();
                    Ok(())
                } else {
                    c.pop_scope_and_run_cleanups()
                };
                r.and(p)
            })?;
            if !stmts_always_return(&blk.stmts) {
                ctx.emit_all_cleanups()?;
                ctx.line("return true;".to_owned());
            }
        } else {
            ctx.line("return true;".to_owned());
        }
        let _ = writeln!(air, "tree {main_id} \"Main\" {{");
        air.push_str(&ctx.finish());
        air.push_str("}\n\n");
    }

    // 8e. Functional callback trees (OBJf).
    let mut callback_specs = Vec::with_capacity(functionals.len());
    for func in &functionals {
        let (cb_index, _) = functional_kind_info(&func.decl.kind, func.decl.kind_span)?;
        let guard_id = if let Some(guard) = &func.decl.guard {
            let gid = alloc_tree_id(func.decl.pinned_guard_id, None, guard.span)?;
            let label = format!("Can_{}", capitalize(&func.decl.kind));
            emit_guard_tree(
                &mut air,
                &env,
                &func.owner_module,
                gid,
                &label,
                guard,
                &func.decl.params,
            )?;
            Some(gid)
        } else {
            None
        };
        let action_id = alloc_tree_id(func.decl.pinned_id, None, func.decl.span)?;
        let action_label = format!("Functional_{}", capitalize(&func.decl.kind));
        let needs_self_save =
            stmts_may_clobber_stack_obj(&func.decl.body, &env, &func.owner_module, true, 0);
        let mut ctx = TreeLowerer::new(
            &env,
            TreeContext::SimOnObject {
                sim_name: func.decl.sim_binding.clone(),
            },
            &func.owner_module,
            needs_self_save,
        );
        if needs_self_save {
            ctx.line("let __self_obj = stack_obj;".to_owned());
        }
        for (pidx, fp) in func.decl.params.iter().enumerate() {
            ctx.bind_func_param(&fp.name, pidx, &fp.ty, fp.span)?;
        }
        ctx.lower_stmts(&func.decl.body)?;
        if !ends_with_unconditional_return(&func.decl.body) {
            ctx.emit_all_cleanups()?;
            ctx.line("return true;".to_owned());
        }
        if func.decl.params.is_empty() {
            let _ = writeln!(air, "tree {action_id} \"{action_label}\" {{");
        } else {
            let _ = writeln!(
                air,
                "tree {action_id} \"{action_label}\" (args: {}) {{",
                func.decl.params.len()
            );
        }
        air.push_str(&ctx.finish());
        air.push_str("}\n\n");

        callback_specs.push(CallbackSpec {
            index: cb_index,
            action: Some(TreeRef::Id(action_id)),
            condition: guard_id.map(TreeRef::Id),
        });
    }

    // 8f. Interactions (guard + action trees + TTAB metadata).
    let mut interaction_specs = Vec::with_capacity(interactions.len());
    for (idx, inter) in interactions.iter().enumerate() {
        let base_ident = sanitize_tree_ident(&inter.decl.label, idx);
        let guard_id = if let Some(guard) = &inter.decl.guard {
            let gid = alloc_tree_id(inter.decl.pinned_guard_id, None, guard.span)?;
            let glabel = format!("Can_{base_ident}");
            emit_guard_tree(
                &mut air,
                &env,
                &inter.owner_module,
                gid,
                &glabel,
                guard,
                &[],
            )?;
            Some(gid)
        } else {
            None
        };
        let action_id = alloc_tree_id(inter.decl.pinned_id, None, inter.decl.span)?;
        let needs_self_save =
            stmts_may_clobber_stack_obj(&inter.decl.run_body, &env, &inter.owner_module, true, 0);
        let mut ctx = TreeLowerer::new(
            &env,
            TreeContext::SimOnObject {
                sim_name: inter.decl.sim_binding.clone(),
            },
            &inter.owner_module,
            needs_self_save,
        );
        if needs_self_save {
            ctx.line("let __self_obj = stack_obj;".to_owned());
        }
        ctx.lower_stmts(&inter.decl.run_body)?;
        if !ends_with_unconditional_return(&inter.decl.run_body) {
            ctx.emit_all_cleanups()?;
            ctx.line("return true;".to_owned());
        }
        let _ = writeln!(air, "interaction {action_id} \"{base_ident}\" {{");
        air.push_str(&ctx.finish());
        air.push_str("}\n\n");

        let mut motives = Vec::with_capacity(inter.decl.advertises.len());
        for ad in &inter.decl.advertises {
            let delta = env.resolve_const_int(&ad.delta, ad.span)?;
            let minimum = env.resolve_const_int(&ad.minimum, ad.span)?;
            motives.push(MotiveAdSpec {
                motive_index: ad.motive_index,
                minimum,
                delta,
                personality: ad.personality,
            });
        }
        interaction_specs.push(InteractionSpec {
            name: inter.decl.label.clone(),
            action: TreeRef::Id(action_id),
            test: guard_id.map(TreeRef::Id),
            flags: inter.decl.flags,
            flags2: inter.decl.flags2,
            autonomy: inter.decl.autonomy,
            attenuation_code: inter.decl.attenuation_code,
            attenuation: inter.decl.attenuation,
            joining_index: inter.decl.joining_index,
            motives,
        });
    }

    // 9. Build SpriteSpec + optional extra graphics + multi-tile TileSpecs.
    let (base_sprite, extra_graphics) = if !graphics.is_empty() {
        let mut iter = graphics.into_iter().map(sprite_decl_to_spec);
        let first = iter.next().expect("non-empty graphics checked above");
        (first, iter.collect())
    } else if let Some(sp) = sprite {
        (sprite_decl_to_spec(sp), Vec::new())
    } else {
        (SpriteSpec::default(), Vec::new())
    };

    let tile_specs: Vec<TileSpec> = tiles
        .into_iter()
        .map(|t| TileSpec {
            guid: t.guid,
            name: t.name,
            objd_id: t.objd_id,
            sub_index: t.sub_index,
            level: t.level,
            lead: t.lead,
            interaction_group: t.interaction_group,
            dgrp_id: t.dgrp_id,
            shape: t.shape,
            slot_id: t.slot_id,
            ttab_id: t.ttab_id,
            init: None,
            main: None,
            attributes: t.attributes,
        })
        .collect();

    let num_attributes = (env.state_order.len() as u16).max(explicit_attributes);

    let manifest = ObjectManifest {
        name: env.name.clone(),
        description,
        guid,
        price,
        object_type,
        catalog_sort_flags,
        objd_id,
        ttab_id,
        slot_id,
        ctss_id,
        dgrp_id,
        spr2_id,
        palt_id,
        master_id,
        attributes: num_attributes,
        interaction_group,
        data_58,
        init: Some(TreeRef::Id(init_id)),
        main: Some(TreeRef::Id(main_id)),
        scripts: Vec::new(),
        sprite: base_sprite,
        extra_graphics,
        interactions: interaction_specs,
        slots: slot_specs,
        sounds: sound_specs,
        constants: Vec::new(),
        callbacks: callback_specs,
        tiles: tile_specs,
        string_tables: Vec::new(),
    };

    Ok(ExpandedV2Object {
        name: env.name,
        slug,
        guid,
        manifest,
        air_source: air,
    })
}

fn sprite_decl_to_spec(sp: SpriteDecl) -> SpriteSpec {
    SpriteSpec {
        shape: sp.shape,
        primary_rgb: sp.primary_rgb,
        accent_rgb: sp.accent_rgb,
        trim_rgb: sp.trim_rgb,
    }
}

fn emit_guard_tree(
    air: &mut String,
    env: &ObjectEnv<'_>,
    owner_module: &str,
    tree_id: u16,
    label: &str,
    guard: &GuardDecl,
    func_params: &[crate::v2::ast::FnParam],
) -> Result<()> {
    let needs_self_save = match &guard.kind {
        GuardBody::Expr(e) => expr_may_clobber_stack_obj(e, env, owner_module, true, 0),
        GuardBody::Block(stmts) => stmts_may_clobber_stack_obj(stmts, env, owner_module, true, 0),
    };
    let mut ctx = TreeLowerer::new(
        env,
        TreeContext::SimOnObject {
            sim_name: guard.sim_binding.clone(),
        },
        owner_module,
        needs_self_save,
    );
    if needs_self_save {
        ctx.line("let __self_obj = stack_obj;".to_owned());
    }
    for (pidx, fp) in func_params.iter().enumerate() {
        ctx.bind_func_param(&fp.name, pidx, &fp.ty, fp.span)?;
    }
    match &guard.kind {
        GuardBody::Expr(expr) => {
            ctx.emit_return_bool_expr(expr)?;
        }
        GuardBody::Block(stmts) => {
            ctx.lower_stmts(stmts)?;
            if !ends_with_unconditional_return(stmts) {
                ctx.emit_all_cleanups()?;
                ctx.line("return true;".to_owned());
            }
        }
    }
    let _ = writeln!(air, "tree {tree_id} \"{label}\" {{");
    air.push_str(&ctx.finish());
    air.push_str("}\n\n");
    Ok(())
}

/// Maps a v2 functional kind name to `(OBJf callback index, optional score object data word)`.
fn functional_kind_info(kind: &str, span: Span) -> Result<(usize, Option<usize>)> {
    match kind {
        "prep" => Ok((18, Some(31))),
        "cook" => Ok((19, Some(32))),
        "surface" => Ok((20, Some(33))),
        "dispose" => Ok((21, Some(36))),
        "eat" => Ok((24, Some(37))),
        "pick_up" => Ok((25, Some(38))),
        "sit" => Ok((26, None)),
        "clean" => Ok((14, Some(61))),
        "repair" => Ok((28, Some(39))),
        "serve" => Ok((16, Some(64))),
        "rest" => Ok((29, Some(15))),
        other => Err(Error::at(
            span,
            format!(
                "unknown functional kind '{other}' (expected prep, cook, surface, dispose, eat, pick_up, sit, clean, repair, serve, or rest)"
            ),
        )),
    }
}

#[derive(Debug, Clone)]
enum TreeContext {
    ObjectSelf,
    SimOnObject { sim_name: String },
}

/// Binding of a local variable or helper parameter during tree lowering.
#[derive(Debug, Clone)]
enum Binding {
    /// Regular local variable in `.air` (`local_name` is unique within the tree).
    Local { air_name: String, ty: ValueType },
    /// Immutable BHAV `param[idx]` (from a `functional` parameter).
    BhavParam { index: usize, ty: ValueType },
    /// Mutable reference (`&mut T`) bound to a caller lvalue during helper inlining.
    MutRef { target: LValue },
    /// Compile-time `Motive` index passed to a helper function.
    Motive(u8),
    /// Compile-time `Sound` FWAV ID passed to a helper function.
    Sound(u16),
    /// Compile-time `Slot` index passed to a helper function.
    Slot(u16),
    /// `Sim` handle passed to a helper function.
    SimHandle,
    /// `self` Object handle passed to a helper function.
    SelfHandle,
    /// Foreign `ObjectId` passed as `Object` handle to a helper function.
    ForeignObject { air_name: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ValueType {
    I16,
    Bool,
    ObjectId,
}

/// A resolved assignable storage location in `.air`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum LValue {
    /// Direct `.air` operand (e.g., `stack.attr[0]`, `my.motive.energy`, `v_x_0`, `temp[2]`).
    Direct { air: String, ty: ValueType },
    /// Attribute on a foreign `ObjectId` local (`stack_obj := obj_air; stack.attr[idx] ...; stack_obj := __self_obj;`).
    ForeignAttr { obj_air: String, attr_index: i16 },
    /// Object data word on a foreign `ObjectId` local.
    ForeignData { obj_air: String, data_index: i16 },
}

impl LValue {
    fn value_type(&self) -> ValueType {
        match self {
            LValue::Direct { ty, .. } => *ty,
            LValue::ForeignAttr { .. } | LValue::ForeignData { .. } => ValueType::I16,
        }
    }
}

#[derive(Debug, Clone)]
enum CleanupAction {
    Unlock(LValue),
    Defer {
        owner_module: String,
        in_object_scope: bool,
        body: Vec<V2Stmt>,
        saved_bindings: Vec<BTreeMap<String, Binding>>,
    },
}

#[derive(Debug, Clone)]
struct ScopeFrame {
    cleanups: Vec<CleanupAction>,
    is_loop_body: bool,
    /// `true` when this frame is the root lexical scope of an inlined helper function.
    is_fn_root: bool,
    /// `true` when this frame corresponds to an emitted `.air` `while` loop
    /// (`repeat`, `while`, `for`, or wrapped inline helper `while 1 == 1`).
    is_air_while: bool,
    cancel_flag: Option<String>,
    repeat_counter: Option<String>,
}

impl ScopeFrame {
    fn block() -> Self {
        Self {
            cleanups: Vec::new(),
            is_loop_body: false,
            is_fn_root: false,
            is_air_while: false,
            cancel_flag: None,
            repeat_counter: None,
        }
    }
}

struct ActiveInlineFn {
    ret_flag: String,
    ret_val: String,
    has_return_type: bool,
}

struct TreeLowerer<'a> {
    env: &'a ObjectEnv<'a>,
    tree_ctx: TreeContext,
    owner_module: String,
    /// `true` when lowering code inside the object/template (can see `local_fns`),
    /// `false` inside an imported/module-level helper function.
    in_object_scope: bool,
    has_self_obj_local: bool,
    out: String,
    indent: usize,
    next_id: usize,
    var_scopes: Vec<BTreeMap<String, Binding>>,
    cleanup_scopes: Vec<ScopeFrame>,
    /// Locks currently held at this point in lowering (including during `defer` execution
    /// until the lock's own `Unlock` cleanup executes).
    held_locks: Vec<LValue>,
    in_defer: bool,
    inline_call_stack: Vec<String>,
    inline_expansions: usize,
    active_inline_fn: Option<ActiveInlineFn>,
}

impl<'a> TreeLowerer<'a> {
    fn new(
        env: &'a ObjectEnv<'a>,
        tree_ctx: TreeContext,
        owner_module: &str,
        has_self_obj_local: bool,
    ) -> Self {
        Self {
            env,
            tree_ctx,
            owner_module: owner_module.to_owned(),
            in_object_scope: true,
            has_self_obj_local,
            out: String::new(),
            indent: 1,
            next_id: 0,
            var_scopes: vec![BTreeMap::new()],
            cleanup_scopes: vec![ScopeFrame::block()],
            held_locks: Vec::new(),
            in_defer: false,
            inline_call_stack: Vec::new(),
            inline_expansions: 0,
            active_inline_fn: None,
        }
    }

    fn finish(self) -> String {
        self.out
    }

    fn fresh(&mut self, prefix: &str) -> String {
        let id = self.next_id;
        self.next_id += 1;
        format!("__{prefix}_{id}")
    }

    fn line(&mut self, text: String) {
        for _ in 0..self.indent {
            self.out.push_str("    ");
        }
        self.out.push_str(&text);
        self.out.push('\n');
    }

    fn with_owner_module<T>(
        &mut self,
        module: &str,
        f: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        let prev = std::mem::replace(&mut self.owner_module, module.to_owned());
        let res = f(self);
        self.owner_module = prev;
        res
    }

    fn bind_var(&mut self, name: String, binding: Binding) {
        if let Some(top) = self.var_scopes.last_mut() {
            top.insert(name, binding);
        }
    }

    fn lookup_var(&self, name: &str) -> Option<&Binding> {
        for scope in self.var_scopes.iter().rev() {
            if let Some(b) = scope.get(name) {
                return Some(b);
            }
        }
        None
    }

    fn bind_func_param(&mut self, name: &str, idx: usize, ty: &V2Type, span: Span) -> Result<()> {
        let vty = match ty {
            V2Type::I16 => ValueType::I16,
            V2Type::Bool => ValueType::Bool,
            V2Type::ObjectId => ValueType::ObjectId,
            other => {
                return Err(Error::at(
                    span,
                    format!(
                        "functional parameter '{name}' has unsupported type '{}' (expected i16, bool, or ObjectId)",
                        other.display_name()
                    ),
                ));
            }
        };
        self.bind_var(
            name.to_owned(),
            Binding::BhavParam {
                index: idx,
                ty: vty,
            },
        );
        Ok(())
    }

    fn is_sim_ident(&self, name: &str) -> bool {
        if matches!(self.lookup_var(name), Some(Binding::SimHandle)) {
            return true;
        }
        matches!(&self.tree_ctx, TreeContext::SimOnObject { sim_name } if sim_name == name)
    }

    fn is_self_ident(&self, name: &str) -> bool {
        if matches!(self.lookup_var(name), Some(Binding::SelfHandle)) {
            return true;
        }
        name == "self"
    }

    fn self_prefix(&self) -> &'static str {
        match self.tree_ctx {
            TreeContext::ObjectSelf => "my",
            TreeContext::SimOnObject { .. } => "stack",
        }
    }

    fn enclosing_cancel_flag(&self) -> Option<(usize, String)> {
        for (idx, frame) in self.cleanup_scopes.iter().enumerate().rev() {
            if let Some(flag) = &frame.cancel_flag {
                return Some((idx, flag.clone()));
            }
        }
        None
    }

    fn nearest_air_while_idx(&self) -> Option<usize> {
        self.cleanup_scopes.iter().rposition(|f| f.is_air_while)
    }

    fn push_scope(&mut self, frame: ScopeFrame) {
        self.var_scopes.push(BTreeMap::new());
        self.cleanup_scopes.push(frame);
    }

    fn pop_scope_and_run_cleanups(&mut self) -> Result<()> {
        let frame = self.cleanup_scopes.pop().expect("cleanup_scopes balanced");
        let res = self.run_frame_cleanups(&frame);
        self.var_scopes.pop();
        res
    }

    fn pop_scope_discard_cleanups(&mut self) {
        self.cleanup_scopes.pop().expect("cleanup_scopes balanced");
        self.var_scopes.pop();
    }

    fn fn_scope_floor(&self) -> usize {
        self.cleanup_scopes
            .iter()
            .rposition(|f| f.is_fn_root)
            .unwrap_or(0)
    }

    fn has_active_cleanups(&self) -> bool {
        let floor = self.fn_scope_floor();
        self.cleanup_scopes[floor..]
            .iter()
            .any(|f| !f.cleanups.is_empty())
    }

    fn has_any_active_cleanups(&self) -> bool {
        self.cleanup_scopes.iter().any(|f| !f.cleanups.is_empty())
    }

    fn emit_cleanups_down_to(&mut self, start: usize) -> Result<()> {
        let saved = self.cleanup_scopes.split_off(start);
        let saved_locks = self.held_locks.clone();
        let mut res = Ok(());
        for frame in saved.iter().rev() {
            if let Err(e) = self.run_frame_cleanups(frame) {
                res = Err(e);
                break;
            }
        }
        self.held_locks = saved_locks;
        self.cleanup_scopes.extend(saved);
        res
    }

    fn emit_all_cleanups(&mut self) -> Result<()> {
        let floor = self.fn_scope_floor();
        self.emit_cleanups_down_to(floor)
    }

    fn emit_loop_cleanups(&mut self, span: Span, is_continue: bool) -> Result<()> {
        let floor = self.fn_scope_floor();
        let loop_idx = self.cleanup_scopes[floor..]
            .iter()
            .rposition(|f| f.is_loop_body)
            .map(|rel| floor + rel)
            .ok_or_else(|| {
                Error::at(
                    span,
                    if is_continue {
                        "'continue' outside of a loop"
                    } else {
                        "'break' outside of a loop"
                    },
                )
            })?;
        self.emit_cleanups_down_to(loop_idx)?;
        if is_continue && let Some(counter) = self.cleanup_scopes[loop_idx].repeat_counter.clone() {
            self.line(format!("{counter} += 1;"));
        }
        Ok(())
    }

    fn emit_cancel_propagation(&mut self) -> Result<()> {
        if self.in_defer {
            return Ok(());
        }
        let Some((_, flag)) = self.enclosing_cancel_flag() else {
            return Ok(());
        };
        let target_while = self
            .nearest_air_while_idx()
            .expect("enclosing cancel_flag implies an enclosing air while loop");
        self.line(format!("if {flag} != 0 {{"));
        self.indent += 1;
        self.emit_cleanups_down_to(target_while)?;
        self.line("break;".to_owned());
        self.indent -= 1;
        self.line("}".to_owned());
        Ok(())
    }

    fn run_frame_cleanups(&mut self, frame: &ScopeFrame) -> Result<()> {
        // LIFO order within each lexical scope!
        for action in frame.cleanups.iter().rev() {
            match action {
                CleanupAction::Unlock(lval) => {
                    self.emit_lvalue_assign(lval, V2AssignOp::Set, "0")?;
                    if let Some(pos) = self.held_locks.iter().rposition(|x| x == lval) {
                        self.held_locks.remove(pos);
                    }
                }
                CleanupAction::Defer {
                    owner_module,
                    in_object_scope,
                    body,
                    saved_bindings,
                } => {
                    let prev_in_defer = self.in_defer;
                    let prev_in_obj = self.in_object_scope;
                    let prev_vars = std::mem::replace(&mut self.var_scopes, saved_bindings.clone());
                    self.in_defer = true;
                    self.in_object_scope = *in_object_scope;
                    let res = self.with_owner_module(owner_module, |c| {
                        c.push_scope(ScopeFrame::block());
                        let r = c.lower_stmts(body);
                        let p = c.pop_scope_and_run_cleanups();
                        r.and(p)
                    });
                    self.in_defer = prev_in_defer;
                    self.in_object_scope = prev_in_obj;
                    self.var_scopes = prev_vars;
                    res?;
                }
            }
        }
        Ok(())
    }

    fn lower_stmts(&mut self, stmts: &[V2Stmt]) -> Result<()> {
        for stmt in stmts {
            self.lower_stmt(stmt)?;
        }
        Ok(())
    }

    fn lower_stmt(&mut self, stmt: &V2Stmt) -> Result<()> {
        match stmt {
            V2Stmt::Let {
                span,
                name,
                ty,
                init,
            } => {
                let (val_air, inferred_ty) = self.lower_value_expr(init)?;
                let final_ty = if let Some(ann) = ty {
                    let expected = match ann {
                        V2Type::I16 => ValueType::I16,
                        V2Type::Bool => ValueType::Bool,
                        V2Type::ObjectId => ValueType::ObjectId,
                        other => {
                            return Err(Error::at(
                                *span,
                                format!(
                                    "unsupported 'let' variable type '{}'",
                                    other.display_name()
                                ),
                            ));
                        }
                    };
                    if expected == ValueType::Bool && inferred_ty != ValueType::Bool {
                        return Err(Error::at(
                            *span,
                            format!(
                                "cannot initialize bool variable '{name}' from non-bool expression"
                            ),
                        ));
                    }
                    if expected != ValueType::Bool && inferred_ty == ValueType::Bool {
                        return Err(Error::at(
                            *span,
                            format!(
                                "cannot initialize '{}' variable '{name}' from bool expression",
                                ann.display_name()
                            ),
                        ));
                    }
                    expected
                } else {
                    inferred_ty
                };
                let air_name = self.fresh(&format!("v_{name}"));
                self.line(format!("let {air_name} = {val_air};"));
                self.bind_var(
                    name.clone(),
                    Binding::Local {
                        air_name,
                        ty: final_ty,
                    },
                );
                Ok(())
            }
            V2Stmt::LetSome {
                span,
                name,
                init,
                else_body,
            } => self.lower_let_some(*span, name, init, else_body),
            V2Stmt::Assign {
                span,
                target,
                op,
                value,
            } => {
                let lval = self.resolve_lvalue(target)?;
                let (val_air, val_ty) = self.lower_value_expr(value)?;
                let target_ty = lval.value_type();
                if target_ty == ValueType::Bool && *op != V2AssignOp::Set {
                    return Err(Error::at(
                        *span,
                        "compound arithmetic assignment is not allowed on a bool target",
                    ));
                }
                if target_ty == ValueType::Bool && val_ty != ValueType::Bool {
                    return Err(Error::at(
                        *span,
                        "cannot assign a non-bool expression to a bool target",
                    ));
                }
                if target_ty != ValueType::Bool && val_ty == ValueType::Bool {
                    return Err(Error::at(
                        *span,
                        "cannot assign a bool expression to a non-bool target",
                    ));
                }
                self.emit_lvalue_assign(&lval, *op, &val_air)
            }
            V2Stmt::WithLock {
                span,
                lock_expr,
                body,
            } => {
                let lval = self.resolve_lvalue(lock_expr)?;
                if lval.value_type() != ValueType::Bool {
                    return Err(Error::at(
                        *span,
                        "with lock(...) requires a bool state or &mut bool target",
                    ));
                }
                if self.held_locks.contains(&lval) {
                    return Err(Error::at(
                        *span,
                        "re-entrant 'with lock(...)' on an already-locked target in the same scope chain",
                    ));
                }
                self.emit_lvalue_assign(&lval, V2AssignOp::Set, "1")?;
                self.held_locks.push(lval.clone());
                self.push_scope(ScopeFrame {
                    cleanups: vec![CleanupAction::Unlock(lval)],
                    is_loop_body: false,
                    is_fn_root: false,
                    is_air_while: false,
                    cancel_flag: None,
                    repeat_counter: None,
                });
                let res = self.lower_stmts(body);
                let pop_res = self.pop_scope_and_run_cleanups();
                res.and(pop_res)
            }
            V2Stmt::Defer { span, body } => {
                if self.in_defer {
                    return Err(Error::at(
                        *span,
                        "nested 'defer' inside another 'defer' block is not allowed",
                    ));
                }
                let saved_bindings = self.var_scopes.clone();
                let owner_module = self.owner_module.clone();
                let in_object_scope = self.in_object_scope;
                if let Some(frame) = self.cleanup_scopes.last_mut() {
                    frame.cleanups.push(CleanupAction::Defer {
                        owner_module,
                        in_object_scope,
                        body: body.clone(),
                        saved_bindings,
                    });
                }
                Ok(())
            }
            V2Stmt::Repeat {
                span,
                count,
                cancel_on_abort,
                body,
            } => {
                if *cancel_on_abort && matches!(self.tree_ctx, TreeContext::ObjectSelf) {
                    return Err(Error::at(
                        *span,
                        "'cancel_on_abort' requires a Sim interaction context (not available in init/main)",
                    ));
                }
                if *cancel_on_abort && self.in_defer {
                    return Err(Error::at(
                        *span,
                        "'cancel_on_abort' is not allowed inside a 'defer' block",
                    ));
                }
                let (limit_air, _) = self.lower_value_expr(count)?;
                let cancel_flag = if *cancel_on_abort {
                    let f = self.fresh("abort");
                    self.line(format!("let {f} = 0;"));
                    Some(f)
                } else {
                    None
                };
                let counter = self.fresh("rep");
                self.line(format!("let {counter} = 0;"));
                self.line(format!("while {counter} < {limit_air} {{"));
                self.indent += 1;
                self.push_scope(ScopeFrame {
                    cleanups: Vec::new(),
                    is_loop_body: true,
                    is_fn_root: false,
                    is_air_while: true,
                    cancel_flag: cancel_flag.clone(),
                    repeat_counter: Some(counter.clone()),
                });
                if let Some(flag) = &cancel_flag {
                    self.line("if my.person.priority == 0 {".to_owned());
                    self.indent += 1;
                    self.line(format!("{flag} := 1;"));
                    self.line("break;".to_owned());
                    self.indent -= 1;
                    self.line("}".to_owned());
                }
                let res = self.lower_stmts(body);
                let pop_res = self.pop_scope_and_run_cleanups();
                self.line(format!("{counter} += 1;"));
                self.indent -= 1;
                self.line("}".to_owned());
                res.and(pop_res)?;
                if let Some(inner_flag) = &cancel_flag
                    && let Some((_, outer_flag)) = self.enclosing_cancel_flag()
                {
                    self.line(format!("if {inner_flag} != 0 {{"));
                    self.indent += 1;
                    self.line(format!("{outer_flag} := 1;"));
                    self.indent -= 1;
                    self.line("}".to_owned());
                }
                self.emit_inline_fn_return_propagation()?;
                self.emit_cancel_propagation()?;
                Ok(())
            }
            V2Stmt::While {
                span,
                cond,
                cancel_on_abort,
                body,
            } => {
                if *cancel_on_abort && matches!(self.tree_ctx, TreeContext::ObjectSelf) {
                    return Err(Error::at(
                        *span,
                        "'cancel_on_abort' requires a Sim interaction context (not available in init/main)",
                    ));
                }
                if *cancel_on_abort && self.in_defer {
                    return Err(Error::at(
                        *span,
                        "'cancel_on_abort' is not allowed inside a 'defer' block",
                    ));
                }
                let cancel_flag = if *cancel_on_abort {
                    let f = self.fresh("abort");
                    self.line(format!("let {f} = 0;"));
                    Some(f)
                } else {
                    None
                };
                if self.is_pure_simple_air_cond(cond)
                    && let Some(simple_cond) = self.try_lower_simple_air_cond(cond)?
                {
                    self.line(format!("while {simple_cond} {{"));
                    self.indent += 1;
                    self.push_scope(ScopeFrame {
                        cleanups: Vec::new(),
                        is_loop_body: true,
                        is_fn_root: false,
                        is_air_while: true,
                        cancel_flag: cancel_flag.clone(),
                        repeat_counter: None,
                    });
                    if let Some(flag) = &cancel_flag {
                        self.line("if my.person.priority == 0 {".to_owned());
                        self.indent += 1;
                        self.line(format!("{flag} := 1;"));
                        self.line("break;".to_owned());
                        self.indent -= 1;
                        self.line("}".to_owned());
                    }
                    let res = self.lower_stmts(body);
                    let pop_res = self.pop_scope_and_run_cleanups();
                    self.indent -= 1;
                    self.line("}".to_owned());
                    res.and(pop_res)?;
                } else {
                    self.line("while 1 == 1 {".to_owned());
                    self.indent += 1;
                    self.push_scope(ScopeFrame {
                        cleanups: Vec::new(),
                        is_loop_body: true,
                        is_fn_root: false,
                        is_air_while: true,
                        cancel_flag: cancel_flag.clone(),
                        repeat_counter: None,
                    });
                    if let Some(flag) = &cancel_flag {
                        self.line("if my.person.priority == 0 {".to_owned());
                        self.indent += 1;
                        self.line(format!("{flag} := 1;"));
                        self.line("break;".to_owned());
                        self.indent -= 1;
                        self.line("}".to_owned());
                    }
                    let cond_loc = self.fresh("wcond");
                    self.line(format!("let {cond_loc} = 0;"));
                    let cond_res = self.emit_bool_into_local(cond, &cond_loc);
                    self.line(format!("if {cond_loc} == 0 {{"));
                    self.indent += 1;
                    self.line("break;".to_owned());
                    self.indent -= 1;
                    self.line("}".to_owned());
                    let body_res = cond_res.and_then(|()| self.lower_stmts(body));
                    let pop_res = self.pop_scope_and_run_cleanups();
                    self.indent -= 1;
                    self.line("}".to_owned());
                    body_res.and(pop_res)?;
                }
                if let Some(inner_flag) = &cancel_flag
                    && let Some((_, outer_flag)) = self.enclosing_cancel_flag()
                {
                    self.line(format!("if {inner_flag} != 0 {{"));
                    self.indent += 1;
                    self.line(format!("{outer_flag} := 1;"));
                    self.indent -= 1;
                    self.line("}".to_owned());
                }
                self.emit_inline_fn_return_propagation()?;
                self.emit_cancel_propagation()?;
                Ok(())
            }
            V2Stmt::ForEach {
                span: _,
                binding,
                iter,
                body,
            } => {
                let iter_var = self.fresh(&format!("v_{binding}"));
                let mut parts_root_opt = None;
                match iter {
                    ForEachIter::ObjectsByGuid(guid) => {
                        self.line(format!("let {iter_var} = 0;"));
                        self.line(format!(
                            "while set_to_next(guid, {guid:#010x}, dest: {iter_var}) {{"
                        ));
                    }
                    ForEachIter::SelfParts => {
                        let root_expr = match self.tree_ctx {
                            TreeContext::ObjectSelf => {
                                let root_loc = self.fresh("parts_root");
                                self.line(format!("let {root_loc} = my.id;"));
                                root_loc
                            }
                            TreeContext::SimOnObject { .. } => {
                                if self.has_self_obj_local {
                                    "__self_obj".to_owned()
                                } else {
                                    let root_loc = self.fresh("parts_root");
                                    self.line(format!("let {root_loc} = stack_obj;"));
                                    root_loc
                                }
                            }
                        };
                        self.line(format!("let {iter_var} = {root_expr};"));
                        self.line(format!("while set_to_next(part, dest: {iter_var}) {{"));
                        parts_root_opt = Some(root_expr);
                    }
                }
                self.indent += 1;
                if let Some(root_expr) = parts_root_opt {
                    self.line(format!("if {iter_var} == {root_expr} {{"));
                    self.indent += 1;
                    self.line("break;".to_owned());
                    self.indent -= 1;
                    self.line("}".to_owned());
                }
                self.push_scope(ScopeFrame {
                    cleanups: Vec::new(),
                    is_loop_body: true,
                    is_fn_root: false,
                    is_air_while: true,
                    cancel_flag: None,
                    repeat_counter: None,
                });
                self.bind_var(
                    binding.clone(),
                    Binding::Local {
                        air_name: iter_var,
                        ty: ValueType::ObjectId,
                    },
                );
                let res = self.lower_stmts(body);
                let pop_res = self.pop_scope_and_run_cleanups();
                self.indent -= 1;
                self.line("}".to_owned());
                if self.has_self_obj_local {
                    self.line("stack_obj := __self_obj;".to_owned());
                }
                res.and(pop_res)?;
                self.emit_inline_fn_return_propagation()?;
                self.emit_cancel_propagation()?;
                Ok(())
            }
            V2Stmt::If {
                span: _,
                cond,
                then_body,
                else_body,
            } => {
                // Constant-fold literal boolean conditions (e.g. from `bool` params!).
                if let Some(const_b) = self.try_eval_const_bool(cond) {
                    let chosen = if const_b { then_body } else { else_body };
                    self.push_scope(ScopeFrame::block());
                    let r = self.lower_stmts(chosen);
                    let p = self.pop_scope_and_run_cleanups();
                    return r.and(p);
                }

                let cond_text = if let Some(simple) = self.try_lower_simple_air_cond(cond)? {
                    simple
                } else {
                    let c = self.fresh("cond");
                    self.line(format!("let {c} = 0;"));
                    self.emit_bool_into_local(cond, &c)?;
                    format!("{c} != 0")
                };

                self.line(format!("if {cond_text} {{"));
                self.indent += 1;
                self.push_scope(ScopeFrame::block());
                let r1 = self.lower_stmts(then_body);
                let p1 = self.pop_scope_and_run_cleanups();
                self.indent -= 1;
                if else_body.is_empty() {
                    self.line("}".to_owned());
                    r1.and(p1)
                } else {
                    self.line("} else {".to_owned());
                    self.indent += 1;
                    self.push_scope(ScopeFrame::block());
                    let r2 = self.lower_stmts(else_body);
                    let p2 = self.pop_scope_and_run_cleanups();
                    self.indent -= 1;
                    self.line("}".to_owned());
                    r1.and(p1).and(r2).and(p2)
                }
            }
            V2Stmt::Break { span } => {
                if self.in_defer {
                    return Err(Error::at(
                        *span,
                        "'break' cannot exit out of a 'defer' block",
                    ));
                }
                self.emit_loop_cleanups(*span, false)?;
                self.line("break;".to_owned());
                Ok(())
            }
            V2Stmt::Continue { span } => {
                if self.in_defer {
                    return Err(Error::at(
                        *span,
                        "'continue' cannot exit out of a 'defer' block",
                    ));
                }
                self.emit_loop_cleanups(*span, true)?;
                self.line("continue;".to_owned());
                Ok(())
            }
            V2Stmt::Return { span, value } => {
                if self.in_defer {
                    return Err(Error::at(
                        *span,
                        "'return' cannot exit out of a 'defer' block",
                    ));
                }
                if let Some(active_fn) = self.active_inline_fn.as_ref() {
                    let ret_flag = active_fn.ret_flag.clone();
                    let ret_val = active_fn.ret_val.clone();
                    let has_ret = active_fn.has_return_type;
                    if let Some(expr) = value {
                        if !has_ret {
                            return Err(Error::at(
                                *span,
                                "void helper function cannot return a value",
                            ));
                        }
                        self.emit_bool_into_local(expr, &ret_val)?;
                    } else if has_ret {
                        return Err(Error::at(
                            *span,
                            "helper function with return type must return a value",
                        ));
                    }
                    self.emit_all_cleanups()?;
                    self.line(format!("{ret_flag} := 1;"));
                    self.line("break;".to_owned());
                    return Ok(());
                }

                match value {
                    None => {
                        self.emit_all_cleanups()?;
                        self.line("return true;".to_owned());
                        Ok(())
                    }
                    Some(expr) => {
                        if !self.has_active_cleanups() {
                            self.emit_return_bool_expr(expr)
                        } else {
                            // Evaluate return expression BEFORE running cleanups (so cleanups cannot
                            // clobber values read by the return expression), then run LIFO cleanups!
                            if let Some(const_b) = self.try_eval_const_bool(expr) {
                                self.emit_all_cleanups()?;
                                self.line(if const_b {
                                    "return true;".to_owned()
                                } else {
                                    "return false;".to_owned()
                                });
                                Ok(())
                            } else {
                                let ret_loc = self.fresh("ret");
                                self.line(format!("let {ret_loc} = 0;"));
                                self.emit_bool_into_local(expr, &ret_loc)?;
                                self.emit_all_cleanups()?;
                                self.line(format!("if {ret_loc} != 0 {{"));
                                self.indent += 1;
                                self.line("return true;".to_owned());
                                self.indent -= 1;
                                self.line("} else {".to_owned());
                                self.indent += 1;
                                self.line("return false;".to_owned());
                                self.indent -= 1;
                                self.line("}".to_owned());
                                Ok(())
                            }
                        }
                    }
                }
            }
            V2Stmt::Expr { span: _, expr } => self.lower_expr_stmt(expr),
            V2Stmt::RawAir { span, code } => {
                if self.in_defer {
                    return Err(Error::at(
                        *span,
                        "'raw { ... }' is not allowed inside a 'defer' block",
                    ));
                }
                if self.has_any_active_cleanups()
                    || !self.inline_call_stack.is_empty()
                    || self.enclosing_cancel_flag().is_some()
                {
                    let wrapped = format!("tree 4096 \"__raw_check\" {{\n{code}\n}}\n");
                    let parsed = crate::parser::parse_module(&wrapped).map_err(|e| {
                        Error::at(
                            *span,
                            format!("invalid .air syntax in raw block: {}", e.message),
                        )
                    })?;
                    let stmts = match parsed.items.first() {
                        Some(crate::ast::Item::Tree(t)) => &t.body,
                        _ => unreachable!("synthetic wrapper always has one tree"),
                    };
                    let mut defined_labels = BTreeSet::new();
                    collect_raw_air_labels(stmts, &mut defined_labels);
                    validate_raw_air_stmts(stmts, false, &defined_labels, *span)?;
                }
                for raw_line in code.lines() {
                    let trimmed = raw_line.trim();
                    if !trimmed.is_empty() {
                        self.line(trimmed.to_owned());
                    }
                }
                Ok(())
            }
        }
    }

    fn emit_inline_fn_return_propagation(&mut self) -> Result<()> {
        if let Some(active) = self.active_inline_fn.as_ref() {
            let ret_flag = active.ret_flag.clone();
            // If we're still inside an outer loop inside the inline fn (Wait: the inline fn's
            // own wrapper loop is at `fn_scope_depth`!), breaking out of an inner loop needs to
            // propagate upward until we exit the wrapper loop.
            self.line(format!("if {ret_flag} != 0 {{"));
            self.indent += 1;
            self.line("break;".to_owned());
            self.indent -= 1;
            self.line("}".to_owned());
        }
        Ok(())
    }

    fn lower_let_some(
        &mut self,
        span: Span,
        name: &str,
        init: &V2Expr,
        else_body: &[V2Stmt],
    ) -> Result<()> {
        if !stmts_unconditionally_diverge(else_body) {
            return Err(Error::at(
                span,
                format!(
                    "'let Some({name}) = ... else {{ ... }}' else-block must diverge (end with return, break, or continue)"
                ),
            ));
        }
        let V2Expr::Call { callee, args, .. } = init else {
            return Err(Error::at(
                span,
                "'let Some(x) = ... else { ... }' requires find_best_object(...) or create_object(...)",
            ));
        };
        let V2Expr::Ident(_, fname) = callee.as_ref() else {
            return Err(Error::at(
                span,
                "'let Some(x) = ... else { ... }' requires find_best_object(...) or create_object(...)",
            ));
        };
        let prim_air = match fname.as_str() {
            "find_best_object" => self.format_find_best_object(args, span)?,
            "create_object" => self.format_create_object(args, span)?,
            other => {
                return Err(Error::at(
                    span,
                    format!(
                        "'let Some({name}) = {other}(...)' is not supported (expected 'find_best_object' or 'create_object')"
                    ),
                ));
            }
        };

        self.line(format!("if !{prim_air} {{"));
        self.indent += 1;
        if self.has_self_obj_local {
            self.line("stack_obj := __self_obj;".to_owned());
        }
        self.push_scope(ScopeFrame::block());
        let r = self.lower_stmts(else_body);
        let p = self.pop_scope_and_run_cleanups();
        self.indent -= 1;
        self.line("}".to_owned());
        r.and(p)?;

        let air_name = self.fresh(&format!("v_{name}"));
        self.line(format!("let {air_name} = stack_obj;"));
        if self.has_self_obj_local {
            self.line("stack_obj := __self_obj;".to_owned());
        }
        self.bind_var(
            name.to_owned(),
            Binding::Local {
                air_name,
                ty: ValueType::ObjectId,
            },
        );
        Ok(())
    }

    fn lower_expr_stmt(&mut self, expr: &V2Expr) -> Result<()> {
        match expr {
            V2Expr::Call { span, callee, args } => {
                if let V2Expr::Ident(_, fname) = callee.as_ref() {
                    match fname.as_str() {
                        "sleep" | "idle" => return self.emit_sleep(args, *span),
                        "play_sound" => {
                            let s = self.format_play_sound(args, *span)?;
                            self.line(format!("{s};"));
                            return Ok(());
                        }
                        "stop_all_sounds" => {
                            let s = self.format_stop_all_sounds(args, *span)?;
                            self.line(format!("{s};"));
                            return Ok(());
                        }
                        "debit_funds" => {
                            let s = self.format_alter_budget(args, *span, false, false)?;
                            self.line(format!("{s};"));
                            return Ok(());
                        }
                        "credit_funds" => {
                            let s = self.format_alter_budget(args, *span, false, true)?;
                            self.line(format!("{s};"));
                            return Ok(());
                        }
                        "destroy_self" => {
                            if !args.is_empty() {
                                return Err(Error::at(*span, "destroy_self() takes no arguments"));
                            }
                            match self.tree_ctx {
                                TreeContext::ObjectSelf => {
                                    self.line("remove_object(me);".to_owned())
                                }
                                TreeContext::SimOnObject { .. } => {
                                    self.line("remove_object(stack_obj);".to_owned())
                                }
                            }
                            return Ok(());
                        }
                        "set_graphic" => {
                            if args.len() != 1 || args[0].name.is_some() {
                                return Err(Error::at(
                                    *span,
                                    "set_graphic(index) expects exactly one positional argument",
                                ));
                            }
                            let (val, _) = self.lower_value_expr(&args[0].value)?;
                            let prefix = self.self_prefix();
                            self.line(format!("{prefix}.data[0] := {val};"));
                            return Ok(());
                        }
                        "route_to" => {
                            let s = self.format_slot_prim("route_to", args, *span)?;
                            self.line(format!("{s};"));
                            return Ok(());
                        }
                        "snap" => {
                            let s = self.format_slot_prim("snap", args, *span)?;
                            self.line(format!("{s};"));
                            return Ok(());
                        }
                        "set_motive_change" => {
                            let s = self.format_set_motive_change(args, *span)?;
                            self.line(format!("{s};"));
                            return Ok(());
                        }
                        "clear_motive_changes" => {
                            if !args.is_empty() {
                                return Err(Error::at(
                                    *span,
                                    "clear_motive_changes() takes no arguments",
                                ));
                            }
                            self.line("clear_motive_changes();".to_owned());
                            return Ok(());
                        }
                        "dialog" => {
                            let s = self.format_dialog(args, *span)?;
                            self.line(format!("{s};"));
                            return Ok(());
                        }
                        _ => {
                            // User helper function call (discarding optional bool return).
                            if let Some((fn_mod, fn_decl, is_local)) =
                                self.env
                                    .lookup_fn(&self.owner_module, fname, self.in_object_scope)
                            {
                                let _ = self
                                    .inline_helper_call(&fn_mod, &fn_decl, is_local, args, *span)?;
                                return Ok(());
                            }
                            return Err(Error::at(*span, format!("unknown function '{fname}'")));
                        }
                    }
                }
                Err(Error::at(*span, "unsupported call target expression"))
            }
            V2Expr::MethodCall {
                span,
                receiver,
                method,
                args,
            } => match method.as_str() {
                "run_functional" => {
                    self.emit_run_functional(receiver, args, *span, None)?;
                    Ok(())
                }
                _ => Err(Error::at(*span, format!("unknown method '.{method}(...)'"))),
            },
            other => Err(Error::at(
                other.span(),
                "expression has no effect as a statement",
            )),
        }
    }

    fn emit_sleep(&mut self, args: &[CallArg], span: Span) -> Result<()> {
        if args.len() != 1 || args[0].name.is_some() {
            return Err(Error::at(
                span,
                "sleep(ticks) expects 1 positional argument",
            ));
        }
        let (ticks_air, _) = self.lower_value_expr(&args[0].value)?;
        let cancel = if self.in_defer {
            None
        } else {
            self.enclosing_cancel_flag()
        };
        if let Some((_, ref flag)) = cancel {
            let target_while = self
                .nearest_air_while_idx()
                .expect("enclosing cancel_flag implies an enclosing air while loop");
            self.line("if my.person.priority == 0 {".to_owned());
            self.indent += 1;
            self.line(format!("{flag} := 1;"));
            self.emit_cleanups_down_to(target_while)?;
            self.line("break;".to_owned());
            self.indent -= 1;
            self.line("}".to_owned());
        }
        self.line(format!("idle({ticks_air});"));
        if let Some((_, ref flag)) = cancel {
            let target_while = self
                .nearest_air_while_idx()
                .expect("enclosing cancel_flag implies an enclosing air while loop");
            self.line("if my.person.priority == 0 {".to_owned());
            self.indent += 1;
            self.line(format!("{flag} := 1;"));
            self.emit_cleanups_down_to(target_while)?;
            self.line("break;".to_owned());
            self.indent -= 1;
            self.line("}".to_owned());
        }
        Ok(())
    }

    fn emit_run_functional(
        &mut self,
        receiver: &V2Expr,
        args: &[CallArg],
        span: Span,
        result_local: Option<&str>,
    ) -> Result<()> {
        if args.is_empty() {
            return Err(Error::at(
                span,
                "run_functional(kind [, args...]) requires a functional kind",
            ));
        }
        let V2Expr::Ident(kspan, kind_name) = &args[0].value else {
            return Err(Error::at(
                args[0].span,
                "first argument to run_functional() must be a functional kind identifier (e.g. prep, cook)",
            ));
        };
        let _ = functional_kind_info(kind_name, *kspan)?;
        let (target_air, target_ty) = self.lower_value_expr(receiver)?;
        if target_ty == ValueType::Bool {
            return Err(Error::at(
                receiver.span(),
                "run_functional() receiver must be an ObjectId, not bool",
            ));
        }
        let staged_target = self.fresh("rf_obj");
        self.line(format!("let {staged_target} = {target_air};"));
        let mut change_icon = false;
        let mut staged_args = Vec::new();
        for arg in &args[1..] {
            if let Some(aname) = &arg.name {
                if aname == "change_icon" {
                    change_icon = self.eval_const_bool_required(&arg.value, arg.span)?;
                } else {
                    return Err(Error::at(
                        arg.span,
                        format!("unknown run_functional option '{aname}'"),
                    ));
                }
            } else {
                let (val, _) = self.lower_value_expr(&arg.value)?;
                let staged = self.fresh("rf_arg");
                self.line(format!("let {staged} = {val};"));
                staged_args.push(staged);
            }
        }
        if staged_args.len() > 8 {
            return Err(Error::at(
                span,
                "run_functional() supports at most 8 temp arguments (temp[0..7])",
            ));
        }
        for (i, val) in staged_args.iter().enumerate() {
            self.line(format!("temp[{i}] := {val};"));
        }
        self.line(format!("stack_obj := {staged_target};"));
        let prim = if change_icon {
            format!("run_functional_tree({kind_name}, change_icon: true)")
        } else {
            format!("run_functional_tree({kind_name})")
        };
        if let Some(dst) = result_local {
            self.line(format!("if {prim} {{"));
            self.indent += 1;
            self.line(format!("{dst} := 1;"));
            self.indent -= 1;
            self.line("} else {".to_owned());
            self.indent += 1;
            self.line(format!("{dst} := 0;"));
            self.indent -= 1;
            self.line("}".to_owned());
        } else {
            self.line(format!("{prim};"));
        }
        if self.has_self_obj_local {
            self.line("stack_obj := __self_obj;".to_owned());
        }
        Ok(())
    }

    fn inline_helper_call(
        &mut self,
        fn_mod: &str,
        fn_decl: &FnDecl,
        is_local_fn: bool,
        args: &[CallArg],
        call_span: Span,
    ) -> Result<Option<String>> {
        let fqn = format!(
            "{}::{fn_mod}::{}",
            if is_local_fn { "local" } else { "mod" },
            fn_decl.name
        );
        if let Some(pos) = self.inline_call_stack.iter().position(|n| n == &fqn) {
            let mut chain: Vec<&str> = self.inline_call_stack[pos..]
                .iter()
                .map(String::as_str)
                .collect();
            chain.push(&fqn);
            return Err(Error::at(
                call_span,
                format!(
                    "recursive function call cycle detected: {}",
                    chain.join(" -> ")
                ),
            ));
        }
        if self.inline_call_stack.len() >= MAX_INLINE_DEPTH {
            return Err(Error::at(
                call_span,
                format!("helper function nesting exceeds maximum depth {MAX_INLINE_DEPTH}"),
            ));
        }
        self.inline_expansions += 1;
        if self.inline_expansions > MAX_INLINE_EXPANSIONS {
            return Err(Error::at(
                call_span,
                format!("helper function expansion budget ({MAX_INLINE_EXPANSIONS}) exceeded"),
            ));
        }
        if args.len() != fn_decl.params.len() {
            return Err(Error::at(
                call_span,
                format!(
                    "function '{}' expects {} argument(s), found {}",
                    fn_decl.name,
                    fn_decl.params.len(),
                    args.len()
                ),
            ));
        }

        // Evaluate and bind each argument in the caller's scope before switching to callee's scope.
        let mut bound_params: BTreeMap<String, Binding> = BTreeMap::new();
        for (param, arg) in fn_decl.params.iter().zip(args.iter()) {
            if arg.name.is_some() {
                return Err(Error::at(
                    arg.span,
                    "named arguments are not supported on helper function calls",
                ));
            }
            let binding = match &param.ty {
                V2Type::I16 => {
                    let (air, ty) = self.lower_value_expr(&arg.value)?;
                    if ty == ValueType::Bool {
                        return Err(Error::at(
                            arg.span,
                            format!("parameter '{}' expects i16, found bool", param.name),
                        ));
                    }
                    let loc = self.fresh(&format!("arg_{}", param.name));
                    self.line(format!("let {loc} = {air};"));
                    Binding::Local {
                        air_name: loc,
                        ty: ValueType::I16,
                    }
                }
                V2Type::Bool => {
                    let (air, ty) = self.lower_value_expr(&arg.value)?;
                    if ty != ValueType::Bool {
                        return Err(Error::at(
                            arg.span,
                            format!("parameter '{}' expects bool", param.name),
                        ));
                    }
                    let loc = self.fresh(&format!("arg_{}", param.name));
                    self.line(format!("let {loc} = {air};"));
                    Binding::Local {
                        air_name: loc,
                        ty: ValueType::Bool,
                    }
                }
                V2Type::ObjectId => {
                    let (air, ty) = self.lower_value_expr(&arg.value)?;
                    let is_null_zero = matches!(arg.value, V2Expr::Int(_, 0));
                    if ty != ValueType::ObjectId && !is_null_zero {
                        return Err(Error::at(
                            arg.span,
                            format!(
                                "parameter '{}' expects ObjectId (or 0), found {ty:?} (convert numeric temps explicitly via 'let id: ObjectId = ...;')",
                                param.name
                            ),
                        ));
                    }
                    let loc = self.fresh(&format!("arg_{}", param.name));
                    self.line(format!("let {loc} = {air};"));
                    Binding::Local {
                        air_name: loc,
                        ty: ValueType::ObjectId,
                    }
                }
                V2Type::MutRef(inner) => {
                    let mut lval = match &arg.value {
                        V2Expr::MutRef { inner: place, .. } => self.resolve_lvalue(place)?,
                        V2Expr::Ident(_, pname) => match self.lookup_var(pname).cloned() {
                            Some(Binding::MutRef { target }) => target,
                            _ => {
                                return Err(Error::at(
                                    arg.span,
                                    format!(
                                        "parameter '{}' expects '&mut {}'",
                                        param.name,
                                        inner.display_name()
                                    ),
                                ));
                            }
                        },
                        _ => {
                            return Err(Error::at(
                                arg.span,
                                format!(
                                    "parameter '{}' expects '&mut {}'",
                                    param.name,
                                    inner.display_name()
                                ),
                            ));
                        }
                    };
                    let expected_inner = match inner.as_ref() {
                        V2Type::Bool => ValueType::Bool,
                        V2Type::I16 => ValueType::I16,
                        V2Type::ObjectId => ValueType::ObjectId,
                        _ => unreachable!("parser restricts &mut inner types"),
                    };
                    if lval.value_type() != expected_inner {
                        return Err(Error::at(
                            arg.span,
                            format!(
                                "type mismatch for '&mut {}' parameter '{}'",
                                inner.display_name(),
                                param.name
                            ),
                        ));
                    }
                    // Snapshot foreign object handle so later reassignment of caller local cannot alias.
                    match &mut lval {
                        LValue::ForeignAttr { obj_air, .. }
                        | LValue::ForeignData { obj_air, .. } => {
                            let snap = self.fresh(&format!("ref_obj_{}", param.name));
                            self.line(format!("let {snap} = {obj_air};"));
                            *obj_air = snap;
                        }
                        LValue::Direct { .. } => {}
                    }
                    Binding::MutRef { target: lval }
                }
                V2Type::Sim => {
                    let V2Expr::Ident(ispan, ident) = &arg.value else {
                        return Err(Error::at(
                            arg.span,
                            format!("parameter '{}' expects Sim handle", param.name),
                        ));
                    };
                    if !self.is_sim_ident(ident) {
                        return Err(Error::at(
                            *ispan,
                            format!(
                                "expected Sim handle for parameter '{}', found '{ident}'",
                                param.name
                            ),
                        ));
                    }
                    Binding::SimHandle
                }
                V2Type::Object => {
                    if let V2Expr::Ident(_, ident) = &arg.value
                        && self.is_self_ident(ident)
                    {
                        Binding::SelfHandle
                    } else {
                        let (air, ty) = self.lower_value_expr(&arg.value)?;
                        if ty != ValueType::ObjectId {
                            return Err(Error::at(
                                arg.span,
                                format!(
                                    "parameter '{}' expects 'self' or ObjectId, found {ty:?}",
                                    param.name
                                ),
                            ));
                        }
                        let loc = self.fresh(&format!("obj_{}", param.name));
                        self.line(format!("let {loc} = {air};"));
                        Binding::ForeignObject { air_name: loc }
                    }
                }
                V2Type::Motive => {
                    let m = self.resolve_motive_expr(&arg.value)?;
                    Binding::Motive(m)
                }
                V2Type::Sound => {
                    let sid = self.resolve_sound_expr(&arg.value)?;
                    Binding::Sound(sid)
                }
                V2Type::Slot => {
                    let slot_idx = self.resolve_slot_expr(&arg.value)?;
                    Binding::Slot(slot_idx)
                }
            };
            bound_params.insert(param.name.clone(), binding);
        }

        let has_ret = match &fn_decl.return_type {
            None => false,
            Some(V2Type::Bool) => true,
            Some(other) => {
                return Err(Error::at(
                    fn_decl.span,
                    format!(
                        "helper function '{}' has unsupported return type '{}' (only '-> bool' or void is supported)",
                        fn_decl.name,
                        other.display_name()
                    ),
                ));
            }
        };
        if has_ret && !stmts_always_return(&fn_decl.body) {
            return Err(Error::at(
                fn_decl.span,
                format!(
                    "helper function '{}' declares '-> bool' but does not return on all control-flow paths",
                    fn_decl.name
                ),
            ));
        }

        let has_returns = stmts_have_return(&fn_decl.body);
        let has_enclosing_cancel = !self.in_defer && self.enclosing_cancel_flag().is_some();
        let needs_wrapper = has_returns || has_enclosing_cancel;
        let ret_val_local = if has_ret {
            let rv = self.fresh(&format!("ret_{}", fn_decl.name));
            self.line(format!("let {rv} = 0;"));
            Some(rv)
        } else {
            None
        };

        self.inline_call_stack.push(fqn);
        let prev_vars = std::mem::replace(&mut self.var_scopes, vec![bound_params]);
        let prev_active_fn = self.active_inline_fn.take();
        let prev_in_obj = self.in_object_scope;
        self.in_object_scope = is_local_fn;

        let res = self.with_owner_module(fn_mod, |c| {
            if needs_wrapper {
                if has_returns {
                    let ret_flag = c.fresh("fn_ret");
                    let rv_name = ret_val_local
                        .clone()
                        .unwrap_or_else(|| c.fresh("fn_void_ret"));
                    c.line(format!("let {ret_flag} = 0;"));
                    c.active_inline_fn = Some(ActiveInlineFn {
                        ret_flag,
                        ret_val: rv_name,
                        has_return_type: has_ret,
                    });
                }
                c.line("while 1 == 1 {".to_owned());
                c.indent += 1;
                c.push_scope(ScopeFrame {
                    cleanups: Vec::new(),
                    is_loop_body: false,
                    is_fn_root: true,
                    is_air_while: true,
                    cancel_flag: None,
                    repeat_counter: None,
                });
                let r = c.lower_stmts(&fn_decl.body);
                let p = c.pop_scope_and_run_cleanups();
                c.line("break;".to_owned());
                c.indent -= 1;
                c.line("}".to_owned());
                r.and(p)
            } else {
                c.push_scope(ScopeFrame {
                    cleanups: Vec::new(),
                    is_loop_body: false,
                    is_fn_root: true,
                    is_air_while: false,
                    cancel_flag: None,
                    repeat_counter: None,
                });
                let r = c.lower_stmts(&fn_decl.body);
                let p = c.pop_scope_and_run_cleanups();
                r.and(p)
            }
        });

        self.in_object_scope = prev_in_obj;
        self.active_inline_fn = prev_active_fn;
        self.var_scopes = prev_vars;
        self.inline_call_stack.pop();
        res?;
        if needs_wrapper {
            self.emit_cancel_propagation()?;
        }
        Ok(ret_val_local)
    }

    fn resolve_lvalue(&mut self, expr: &V2Expr) -> Result<LValue> {
        match expr {
            V2Expr::Deref { span, inner } => {
                let V2Expr::Ident(_, pname) = inner.as_ref() else {
                    return Err(Error::at(
                        *span,
                        "only '*param' dereference on '&mut' parameters is supported",
                    ));
                };
                match self.lookup_var(pname).cloned() {
                    Some(Binding::MutRef { target, .. }) => Ok(target),
                    Some(_) => Err(Error::at(
                        *span,
                        format!("cannot dereference non-'&mut' variable '{pname}'"),
                    )),
                    None => Err(Error::at(*span, format!("unknown variable '{pname}'"))),
                }
            }
            V2Expr::Ident(span, name) => match self.lookup_var(name).cloned() {
                Some(Binding::Local { air_name, ty }) => Ok(LValue::Direct { air: air_name, ty }),
                Some(Binding::BhavParam { .. }) => Err(Error::at(
                    *span,
                    format!("cannot assign directly to immutable parameter '{name}'"),
                )),
                Some(Binding::MutRef { .. }) => Err(Error::at(
                    *span,
                    format!("use '*{name} = ...' to assign through '&mut' parameter '{name}'"),
                )),
                Some(_) => Err(Error::at(*span, format!("'{name}' is not assignable"))),
                None => {
                    if self.env.i16_params.contains_key(name)
                        || self.env.bool_params.contains_key(name)
                    {
                        Err(Error::at(
                            *span,
                            format!("cannot assign to compile-time param '{name}'"),
                        ))
                    } else {
                        Err(Error::at(*span, format!("unknown variable '{name}'")))
                    }
                }
            },
            V2Expr::Field { span, base, field } => {
                // `sim.motive.energy`, `sim.skill.cooking`, `self.<state>`, `self.graphic`, etc.
                if let V2Expr::Field {
                    base: root,
                    field: group,
                    ..
                } = base.as_ref()
                    && let V2Expr::Ident(rspan, rname) = root.as_ref()
                {
                    if rname == "sim" && matches!(self.tree_ctx, TreeContext::ObjectSelf) {
                        return Err(Error::at(
                            *rspan,
                            "'sim' is not available in init/main (object lifecycle context)",
                        ));
                    }
                    if self.is_sim_ident(rname) {
                        return match group.as_str() {
                            "motive" => {
                                let _ = motive_by_name(field).ok_or_else(|| {
                                    Error::at(*span, format!("unknown motive '{field}'"))
                                })?;
                                Ok(LValue::Direct {
                                    air: format!("my.motive.{field}"),
                                    ty: ValueType::I16,
                                })
                            }
                            "skill" => {
                                let _ = skill_by_name(field).ok_or_else(|| {
                                    Error::at(*span, format!("unknown skill '{field}'"))
                                })?;
                                Ok(LValue::Direct {
                                    air: format!("my.person.{field}"),
                                    ty: ValueType::I16,
                                })
                            }
                            other => Err(Error::at(
                                *span,
                                format!("unknown Sim member '{rname}.{other}.{field}'"),
                            )),
                        };
                    }
                    if self.is_self_ident(rname) {
                        return Err(Error::at(
                            *span,
                            format!(
                                "'self' is an Object, not a Sim (cannot access 'self.{group}.{field}')"
                            ),
                        ));
                    }
                }

                if let V2Expr::Ident(bspan, bname) = base.as_ref() {
                    if self.is_self_ident(bname) {
                        if let Some(st) = self.env.states.get(field) {
                            let ty = match st.ty {
                                StateType::Bool => ValueType::Bool,
                                StateType::I16 => ValueType::I16,
                                StateType::ObjectId => ValueType::ObjectId,
                            };
                            return Ok(LValue::Direct {
                                air: format!("{}.attr[{}]", self.self_prefix(), st.attr_index),
                                ty,
                            });
                        }
                        return match field.as_str() {
                            "graphic" => Ok(LValue::Direct {
                                air: format!("{}.data[0]", self.self_prefix()),
                                ty: ValueType::I16,
                            }),
                            "prep_score" => Ok(LValue::Direct {
                                air: format!("{}.prep_score", self.self_prefix()),
                                ty: ValueType::I16,
                            }),
                            "cook_score" => Ok(LValue::Direct {
                                air: format!("{}.cook_score", self.self_prefix()),
                                ty: ValueType::I16,
                            }),
                            "in_use" => Ok(LValue::Direct {
                                air: format!("{}.in_use", self.self_prefix()),
                                ty: ValueType::I16,
                            }),
                            "motive" | "skill" | "priority" => Err(Error::at(
                                *span,
                                format!(
                                    "'self' is an Object, not a Sim (cannot access 'self.{field}')"
                                ),
                            )),
                            other => Err(Error::at(
                                *span,
                                format!("unknown object state or field 'self.{other}'"),
                            )),
                        };
                    }
                    if self.is_sim_ident(bname) {
                        return Err(Error::at(
                            *span,
                            format!(
                                "cannot assign to '{bname}.{field}' (use '{bname}.motive.<name>' or '{bname}.skill.<name>'; use 'self.<state>' for object state)"
                            ),
                        ));
                    }
                    if bname == "sim" && matches!(self.tree_ctx, TreeContext::ObjectSelf) {
                        return Err(Error::at(
                            *bspan,
                            "'sim' is not available in init/main (object lifecycle context)",
                        ));
                    }
                }
                Err(Error::at(
                    *span,
                    "expression is not a valid assignment target",
                ))
            }
            V2Expr::Index { span, base, index } => {
                // `temp[i]`, `global[i]`, `self.attr[i]`, `self.data[i]`, `sim.motive[m]`, `foreign.attr[i]`
                if let V2Expr::Ident(_, bname) = base.as_ref() {
                    match bname.as_str() {
                        "temp" => {
                            let idx = self.eval_const_i16(index)?;
                            return Ok(LValue::Direct {
                                air: format!("temp[{idx}]"),
                                ty: ValueType::I16,
                            });
                        }
                        "global" => {
                            let idx = self.eval_const_i16(index)?;
                            return Ok(LValue::Direct {
                                air: format!("global[{idx}]"),
                                ty: ValueType::I16,
                            });
                        }
                        _ => {}
                    }
                }
                if let V2Expr::Field {
                    base: root, field, ..
                } = base.as_ref()
                    && let V2Expr::Ident(_, rname) = root.as_ref()
                {
                    if self.is_sim_ident(rname) && field == "motive" {
                        let m = self.resolve_motive_expr(index)?;
                        return Ok(LValue::Direct {
                            air: format!("my.motive[{m}]"),
                            ty: ValueType::I16,
                        });
                    }
                    if self.is_self_ident(rname) {
                        let idx = self.eval_const_i16(index)?;
                        return match field.as_str() {
                            "attr" => Ok(LValue::Direct {
                                air: format!("{}.attr[{idx}]", self.self_prefix()),
                                ty: ValueType::I16,
                            }),
                            "data" => Ok(LValue::Direct {
                                air: format!("{}.data[{idx}]", self.self_prefix()),
                                ty: ValueType::I16,
                            }),
                            other => Err(Error::at(
                                *span,
                                format!("cannot index 'self.{other}[...]'"),
                            )),
                        };
                    }
                    // Foreign ObjectId variable: `plate.attr[0] = nutrition;`
                    if let Some(binding) = self.lookup_var(rname).cloned() {
                        let obj_air = match binding {
                            Binding::Local {
                                air_name,
                                ty: ValueType::ObjectId,
                            }
                            | Binding::ForeignObject { air_name } => air_name,
                            Binding::BhavParam {
                                index,
                                ty: ValueType::ObjectId,
                            } => format!("param[{index}]"),
                            Binding::Local { ty, .. } | Binding::BhavParam { ty, .. } => {
                                return Err(Error::at(
                                    *span,
                                    format!(
                                        "cannot index '{rname}.{field}[...]': '{rname}' has type {ty:?}, expected ObjectId"
                                    ),
                                ));
                            }
                            _ => {
                                return Err(Error::at(
                                    *span,
                                    format!("cannot index '{rname}.{field}[...]'"),
                                ));
                            }
                        };
                        let idx = self.eval_const_i16(index)?;
                        return match field.as_str() {
                            "attr" => Ok(LValue::ForeignAttr {
                                obj_air,
                                attr_index: idx,
                            }),
                            "data" => Ok(LValue::ForeignData {
                                obj_air,
                                data_index: idx,
                            }),
                            other => Err(Error::at(
                                *span,
                                format!("cannot index '{rname}.{other}[...]'"),
                            )),
                        };
                    }
                }
                Err(Error::at(*span, "unsupported indexed assignment target"))
            }
            other => Err(Error::at(
                other.span(),
                "expression is not a valid assignment target",
            )),
        }
    }

    fn emit_lvalue_assign(&mut self, lval: &LValue, op: V2AssignOp, rhs_air: &str) -> Result<()> {
        let op_str = match op {
            V2AssignOp::Set => ":=",
            V2AssignOp::Add => "+=",
            V2AssignOp::Sub => "-=",
            V2AssignOp::Mul => "*=",
            V2AssignOp::Div => "/=",
            V2AssignOp::Mod => "%=",
            V2AssignOp::BitAnd => "&=",
            V2AssignOp::BitOr => "|=",
            V2AssignOp::BitXor => "^=",
            V2AssignOp::FlagSet => "flag_set",
            V2AssignOp::FlagClear => "flag_clear",
        };
        match lval {
            LValue::Direct { air, .. } => {
                self.line(format!("{air} {op_str} {rhs_air};"));
            }
            LValue::ForeignAttr {
                obj_air,
                attr_index,
            } => {
                let effective_rhs = if rhs_air.starts_with("stack.") || rhs_air == "stack_obj" {
                    let snap = self.fresh("frhs");
                    self.line(format!("let {snap} = {rhs_air};"));
                    snap
                } else {
                    rhs_air.to_owned()
                };
                self.line(format!("stack_obj := {obj_air};"));
                self.line(format!(
                    "stack.attr[{attr_index}] {op_str} {effective_rhs};"
                ));
                if self.has_self_obj_local {
                    self.line("stack_obj := __self_obj;".to_owned());
                }
            }
            LValue::ForeignData {
                obj_air,
                data_index,
            } => {
                let effective_rhs = if rhs_air.starts_with("stack.") || rhs_air == "stack_obj" {
                    let snap = self.fresh("frhs");
                    self.line(format!("let {snap} = {rhs_air};"));
                    snap
                } else {
                    rhs_air.to_owned()
                };
                self.line(format!("stack_obj := {obj_air};"));
                self.line(format!(
                    "stack.data[{data_index}] {op_str} {effective_rhs};"
                ));
                if self.has_self_obj_local {
                    self.line("stack_obj := __self_obj;".to_owned());
                }
            }
        }
        Ok(())
    }

    /// Lowers a value-producing expression into an `.air` operand string + its [`ValueType`].
    fn lower_value_expr(&mut self, expr: &V2Expr) -> Result<(String, ValueType)> {
        match expr {
            V2Expr::Int(span, v) => {
                let n = i16::try_from(*v)
                    .map_err(|_| Error::at(*span, format!("integer {v} out of i16 range")))?;
                Ok((n.to_string(), ValueType::I16))
            }
            V2Expr::Bool(_, b) => Ok((if *b { "1" } else { "0" }.to_owned(), ValueType::Bool)),
            V2Expr::Ident(span, name) => {
                if let Some(binding) = self.lookup_var(name).cloned() {
                    return match binding {
                        Binding::Local { air_name, ty } => Ok((air_name, ty)),
                        Binding::BhavParam { index, ty } => Ok((format!("param[{index}]"), ty)),
                        Binding::ForeignObject { air_name } => Ok((air_name, ValueType::ObjectId)),
                        Binding::MutRef { .. } => Err(Error::at(
                            *span,
                            format!("use '*{name}' to read through '&mut' parameter '{name}'"),
                        )),
                        Binding::SelfHandle => match self.tree_ctx {
                            TreeContext::ObjectSelf => {
                                Ok(("my.id".to_owned(), ValueType::ObjectId))
                            }
                            TreeContext::SimOnObject { .. } => Ok((
                                if self.has_self_obj_local {
                                    "__self_obj".to_owned()
                                } else {
                                    "stack_obj".to_owned()
                                },
                                ValueType::ObjectId,
                            )),
                        },
                        Binding::SimHandle => Ok(("my.id".to_owned(), ValueType::ObjectId)),
                        Binding::Motive(_) | Binding::Sound(_) | Binding::Slot(_) => {
                            Err(Error::at(
                                *span,
                                format!("'{name}' cannot be used directly as a numeric value"),
                            ))
                        }
                    };
                }
                if self.is_self_ident(name) {
                    return match self.tree_ctx {
                        TreeContext::ObjectSelf => Ok(("my.id".to_owned(), ValueType::ObjectId)),
                        TreeContext::SimOnObject { .. } => Ok((
                            if self.has_self_obj_local {
                                "__self_obj".to_owned()
                            } else {
                                "stack_obj".to_owned()
                            },
                            ValueType::ObjectId,
                        )),
                    };
                }
                if name == "sim" && matches!(self.tree_ctx, TreeContext::ObjectSelf) {
                    return Err(Error::at(
                        *span,
                        "'sim' is not available in init/main (object lifecycle context)",
                    ));
                }
                if self.env.i16_params.contains_key(name) {
                    return Ok((format!("bcon.{name}"), ValueType::I16));
                }
                if let Some(&b) = self.env.bool_params.get(name) {
                    return Ok((if b { "1" } else { "0" }.to_owned(), ValueType::Bool));
                }
                Err(Error::at(*span, format!("unknown identifier '{name}'")))
            }
            V2Expr::Deref { span, inner } => {
                let lval = self.resolve_lvalue(&V2Expr::Deref {
                    span: *span,
                    inner: inner.clone(),
                })?;
                self.read_lvalue(&lval)
            }
            V2Expr::Field { span, base, field } => {
                // Handle read-only properties like `sim.id`, `sim.priority`, `self.id`, `self.room`
                if let V2Expr::Ident(bspan, bname) = base.as_ref() {
                    if self.is_sim_ident(bname) {
                        return match field.as_str() {
                            "id" => Ok(("my.id".to_owned(), ValueType::ObjectId)),
                            "priority" => Ok(("my.person.priority".to_owned(), ValueType::I16)),
                            "room" => Ok(("my.room".to_owned(), ValueType::I16)),
                            other => Err(Error::at(
                                *span,
                                format!(
                                    "unknown Sim property '{bname}.{other}' (expected motive, skill, id, priority, or room; use 'self.<state>' for object state)"
                                ),
                            )),
                        };
                    }
                    if bname == "sim" && matches!(self.tree_ctx, TreeContext::ObjectSelf) {
                        return Err(Error::at(
                            *bspan,
                            "'sim' is not available in init/main (object lifecycle context)",
                        ));
                    }
                    if self.is_self_ident(bname) {
                        match field.as_str() {
                            "id" => {
                                return Ok((
                                    format!("{}.id", self.self_prefix()),
                                    ValueType::ObjectId,
                                ));
                            }
                            "room" => {
                                return Ok((
                                    format!("{}.room", self.self_prefix()),
                                    ValueType::I16,
                                ));
                            }
                            _ => {}
                        }
                    }
                }
                let lval = self.resolve_lvalue(expr)?;
                self.read_lvalue(&lval)
            }
            V2Expr::Index { .. } => {
                let lval = self.resolve_lvalue(expr)?;
                self.read_lvalue(&lval)
            }
            V2Expr::Unary { span, op, inner } => match op {
                V2UnaryOp::Not => {
                    let tmp = self.fresh("not");
                    self.line(format!("let {tmp} = 0;"));
                    self.emit_bool_into_local(expr, &tmp)?;
                    Ok((tmp, ValueType::Bool))
                }
                V2UnaryOp::Neg => {
                    if let V2Expr::Int(_, v) = inner.as_ref() {
                        let n = v
                            .checked_neg()
                            .and_then(|x| i16::try_from(x).ok())
                            .ok_or_else(|| {
                                Error::at(*span, format!("negated integer -{v} out of i16 range"))
                            })?;
                        return Ok((n.to_string(), ValueType::I16));
                    }
                    let (v_air, _) = self.lower_value_expr(inner)?;
                    let tmp = self.fresh("neg");
                    self.line(format!("let {tmp} = 0;"));
                    self.line(format!("{tmp} -= {v_air};"));
                    Ok((tmp, ValueType::I16))
                }
            },
            V2Expr::Binary {
                span: _,
                lhs,
                op,
                rhs,
            } => match op {
                V2BinOp::And
                | V2BinOp::Or
                | V2BinOp::Eq
                | V2BinOp::Ne
                | V2BinOp::Lt
                | V2BinOp::Le
                | V2BinOp::Gt
                | V2BinOp::Ge
                | V2BinOp::FlagTest => {
                    let tmp = self.fresh("bool");
                    self.line(format!("let {tmp} = 0;"));
                    self.emit_bool_into_local(expr, &tmp)?;
                    Ok((tmp, ValueType::Bool))
                }
                V2BinOp::Add
                | V2BinOp::Sub
                | V2BinOp::Mul
                | V2BinOp::Div
                | V2BinOp::Mod
                | V2BinOp::BitAnd
                | V2BinOp::BitOr
                | V2BinOp::BitXor => {
                    let (l_air, _) = self.lower_value_expr(lhs)?;
                    let tmp = self.fresh("arith");
                    self.line(format!("let {tmp} = {l_air};"));
                    let (r_air, _) = self.lower_value_expr(rhs)?;
                    let aop = match op {
                        V2BinOp::Add => "+=",
                        V2BinOp::Sub => "-=",
                        V2BinOp::Mul => "*=",
                        V2BinOp::Div => "/=",
                        V2BinOp::Mod => "%=",
                        V2BinOp::BitAnd => "&=",
                        V2BinOp::BitOr => "|=",
                        V2BinOp::BitXor => "^=",
                        _ => unreachable!(),
                    };
                    self.line(format!("{tmp} {aop} {r_air};"));
                    Ok((tmp, ValueType::I16))
                }
            },
            V2Expr::Call { span, callee, args } => {
                if let V2Expr::Ident(_, fname) = callee.as_ref() {
                    if fname == "random" {
                        if args.len() != 1 || args[0].name.is_some() {
                            return Err(Error::at(*span, "random(range) expects 1 argument"));
                        }
                        let (range_air, _) = self.lower_value_expr(&args[0].value)?;
                        let tmp = self.fresh("rnd");
                        self.line(format!("let {tmp} = 0;"));
                        self.line(format!("random({tmp}, {range_air});"));
                        return Ok((tmp, ValueType::I16));
                    }
                    if matches!(
                        fname.as_str(),
                        "can_afford"
                            | "debit_funds"
                            | "credit_funds"
                            | "dialog"
                            | "route_to"
                            | "snap"
                            | "find_best_object"
                            | "create_object"
                    ) {
                        let tmp = self.fresh("call");
                        self.line(format!("let {tmp} = 0;"));
                        self.emit_bool_into_local(expr, &tmp)?;
                        return Ok((tmp, ValueType::Bool));
                    }
                    if let Some((fn_mod, fn_decl, is_local)) =
                        self.env
                            .lookup_fn(&self.owner_module, fname, self.in_object_scope)
                    {
                        let ret =
                            self.inline_helper_call(&fn_mod, &fn_decl, is_local, args, *span)?;
                        let rv = ret.ok_or_else(|| {
                            Error::at(
                                *span,
                                format!("void helper function '{fname}' does not return a value"),
                            )
                        })?;
                        return Ok((rv, ValueType::Bool));
                    }
                }
                Err(Error::at(*span, "unsupported value call expression"))
            }
            V2Expr::MethodCall {
                span,
                receiver,
                method,
                args,
            } if method == "run_functional" => {
                let tmp = self.fresh("rf");
                self.line(format!("let {tmp} = 0;"));
                self.emit_run_functional(receiver, args, *span, Some(&tmp))?;
                Ok((tmp, ValueType::Bool))
            }
            other => Err(Error::at(
                other.span(),
                "expression cannot be evaluated as a value",
            )),
        }
    }

    fn read_lvalue(&mut self, lval: &LValue) -> Result<(String, ValueType)> {
        match lval {
            LValue::Direct { air, ty } => Ok((air.clone(), *ty)),
            LValue::ForeignAttr {
                obj_air,
                attr_index,
            } => {
                let tmp = self.fresh("fattr");
                self.line(format!("stack_obj := {obj_air};"));
                self.line(format!("let {tmp} = stack.attr[{attr_index}];"));
                if self.has_self_obj_local {
                    self.line("stack_obj := __self_obj;".to_owned());
                }
                Ok((tmp, ValueType::I16))
            }
            LValue::ForeignData {
                obj_air,
                data_index,
            } => {
                let tmp = self.fresh("fdata");
                self.line(format!("stack_obj := {obj_air};"));
                self.line(format!("let {tmp} = stack.data[{data_index}];"));
                if self.has_self_obj_local {
                    self.line("stack_obj := __self_obj;".to_owned());
                }
                Ok((tmp, ValueType::I16))
            }
        }
    }

    fn try_eval_const_bool(&self, expr: &V2Expr) -> Option<bool> {
        match expr {
            V2Expr::Bool(_, b) => Some(*b),
            V2Expr::Ident(_, name) => {
                if self.lookup_var(name).is_none() {
                    self.env.bool_params.get(name).copied()
                } else {
                    None
                }
            }
            V2Expr::Unary {
                op: V2UnaryOp::Not,
                inner,
                ..
            } => self.try_eval_const_bool(inner).map(|b| !b),
            V2Expr::Binary {
                lhs,
                op: V2BinOp::And,
                rhs,
                ..
            } => match self.try_eval_const_bool(lhs) {
                Some(false) => Some(false),
                Some(true) => self.try_eval_const_bool(rhs),
                None => None,
            },
            V2Expr::Binary {
                lhs,
                op: V2BinOp::Or,
                rhs,
                ..
            } => match self.try_eval_const_bool(lhs) {
                Some(true) => Some(true),
                Some(false) => self.try_eval_const_bool(rhs),
                None => None,
            },
            _ => None,
        }
    }

    fn eval_const_bool_required(&self, expr: &V2Expr, span: Span) -> Result<bool> {
        self.try_eval_const_bool(expr).ok_or_else(|| {
            Error::at(
                span,
                "expected compile-time boolean ('true', 'false', or bool param)",
            )
        })
    }

    fn eval_const_i16(&self, expr: &V2Expr) -> Result<i16> {
        match expr {
            V2Expr::Int(span, v) => i16::try_from(*v)
                .map_err(|_| Error::at(*span, format!("integer {v} out of i16 range"))),
            V2Expr::Unary {
                span,
                op: V2UnaryOp::Neg,
                inner,
            } => {
                let inner_v = self.eval_const_i16(inner)?;
                inner_v
                    .checked_neg()
                    .ok_or_else(|| Error::at(*span, "i16 negation overflow"))
            }
            V2Expr::Ident(span, name) => {
                self.env.i16_params.get(name).copied().ok_or_else(|| {
                    Error::at(*span, format!("expected constant i16, found '{name}'"))
                })
            }
            other => Err(Error::at(other.span(), "expected constant i16 index")),
        }
    }

    /// Returns `true` iff lowering `expr` emits zero setup statements and has zero side effects.
    fn is_pure_single_operand(&self, expr: &V2Expr) -> bool {
        match expr {
            V2Expr::Int(_, _) | V2Expr::Bool(_, _) | V2Expr::Ident(_, _) => true,
            V2Expr::Unary {
                op: V2UnaryOp::Neg,
                inner,
                ..
            } => matches!(inner.as_ref(), V2Expr::Int(_, _)),
            V2Expr::Deref { inner, .. } => {
                if let V2Expr::Ident(_, pname) = inner.as_ref() {
                    matches!(
                        self.lookup_var(pname),
                        Some(Binding::MutRef {
                            target: LValue::Direct { .. }
                        })
                    )
                } else {
                    false
                }
            }
            V2Expr::Field { base, .. } => match base.as_ref() {
                V2Expr::Ident(_, bname) => self.is_self_ident(bname) || self.is_sim_ident(bname),
                V2Expr::Field { base: root, .. } => {
                    matches!(root.as_ref(), V2Expr::Ident(_, rname) if self.is_sim_ident(rname))
                }
                _ => false,
            },
            V2Expr::Index { base, .. } => match base.as_ref() {
                V2Expr::Ident(_, bname) => matches!(bname.as_str(), "temp" | "global"),
                V2Expr::Field { base: root, .. } => {
                    matches!(
                        root.as_ref(),
                        V2Expr::Ident(_, rname) if self.is_self_ident(rname) || self.is_sim_ident(rname)
                    )
                }
                _ => false,
            },
            _ => false,
        }
    }

    /// Returns `true` iff `expr` can be lowered directly into an `.air` `while <cond>` header
    /// without hoisting any setup statements before the loop!
    fn is_pure_simple_air_cond(&self, expr: &V2Expr) -> bool {
        match expr {
            V2Expr::Bool(_, _) => true,
            V2Expr::Unary {
                op: V2UnaryOp::Not,
                inner,
                ..
            } => self.is_pure_simple_air_cond(inner),
            V2Expr::Binary { lhs, op, rhs, .. } => match op {
                V2BinOp::Eq
                | V2BinOp::Ne
                | V2BinOp::Lt
                | V2BinOp::Le
                | V2BinOp::Gt
                | V2BinOp::Ge
                | V2BinOp::FlagTest => {
                    self.is_pure_single_operand(lhs) && self.is_pure_single_operand(rhs)
                }
                _ => false,
            },
            V2Expr::Call { .. } | V2Expr::MethodCall { .. } => false,
            _ => self.is_pure_single_operand(expr),
        }
    }

    fn lower_cmp_operands(&mut self, lhs: &V2Expr, rhs: &V2Expr) -> Result<(String, String)> {
        let (l, _) = self.lower_value_expr(lhs)?;
        if self.is_pure_single_operand(rhs) {
            let (r, _) = self.lower_value_expr(rhs)?;
            Ok((l, r))
        } else {
            let snap_l = self.fresh("cmp_l");
            self.line(format!("let {snap_l} = {l};"));
            let (r, _) = self.lower_value_expr(rhs)?;
            Ok((snap_l, r))
        }
    }

    /// Tries to lower a boolean condition into a single `.air` `Condition` string (no `&&`/`||`).
    fn try_lower_simple_air_cond(&mut self, expr: &V2Expr) -> Result<Option<String>> {
        match expr {
            V2Expr::Bool(_, true) => Ok(Some("1 == 1".to_owned())),
            V2Expr::Bool(_, false) => Ok(Some("1 == 0".to_owned())),
            V2Expr::Unary {
                op: V2UnaryOp::Not,
                inner,
                ..
            } => {
                if let Some(neg) = self.try_lower_negated_simple_air_cond(inner)? {
                    return Ok(Some(neg));
                }
                Ok(None)
            }
            V2Expr::Binary { lhs, op, rhs, .. } => match op {
                V2BinOp::And | V2BinOp::Or => Ok(None),
                V2BinOp::Eq
                | V2BinOp::Ne
                | V2BinOp::Lt
                | V2BinOp::Le
                | V2BinOp::Gt
                | V2BinOp::Ge
                | V2BinOp::FlagTest => {
                    let (l, r) = self.lower_cmp_operands(lhs, rhs)?;
                    let sym = cmp_op_str(*op);
                    Ok(Some(format!("{l} {sym} {r}")))
                }
                _ => Err(Error::at(
                    expr.span(),
                    "arithmetic expression cannot be used directly as a boolean condition",
                )),
            },
            V2Expr::Call { span, callee, args } => {
                if let V2Expr::Ident(_, fname) = callee.as_ref() {
                    match fname.as_str() {
                        "can_afford" => {
                            return Ok(Some(self.format_alter_budget(args, *span, true, false)?));
                        }
                        "debit_funds" => {
                            return Ok(Some(self.format_alter_budget(args, *span, false, false)?));
                        }
                        "credit_funds" => {
                            return Ok(Some(self.format_alter_budget(args, *span, false, true)?));
                        }
                        "dialog" => return Ok(Some(self.format_dialog(args, *span)?)),
                        "route_to" => {
                            return Ok(Some(self.format_slot_prim("route_to", args, *span)?));
                        }
                        "snap" => return Ok(Some(self.format_slot_prim("snap", args, *span)?)),
                        _ => return Ok(None),
                    }
                }
                Ok(None)
            }
            V2Expr::MethodCall { method, .. } if method == "run_functional" => Ok(None),
            _ => {
                let (val, ty) = self.lower_value_expr(expr)?;
                if ty != ValueType::Bool {
                    return Err(Error::at(
                        expr.span(),
                        "expected boolean condition (use an explicit comparison like 'x != 0')",
                    ));
                }
                Ok(Some(format!("{val} != 0")))
            }
        }
    }

    fn try_lower_negated_simple_air_cond(&mut self, expr: &V2Expr) -> Result<Option<String>> {
        match expr {
            V2Expr::Bool(_, true) => Ok(Some("1 == 0".to_owned())),
            V2Expr::Bool(_, false) => Ok(Some("1 == 1".to_owned())),
            V2Expr::Unary {
                op: V2UnaryOp::Not,
                inner,
                ..
            } => self.try_lower_simple_air_cond(inner),
            V2Expr::Binary { lhs, op, rhs, .. } => match op {
                V2BinOp::And | V2BinOp::Or => Ok(None),
                V2BinOp::Eq => {
                    let (l, r) = self.lower_cmp_operands(lhs, rhs)?;
                    Ok(Some(format!("{l} != {r}")))
                }
                V2BinOp::Ne => {
                    let (l, r) = self.lower_cmp_operands(lhs, rhs)?;
                    Ok(Some(format!("{l} == {r}")))
                }
                V2BinOp::Lt => {
                    let (l, r) = self.lower_cmp_operands(lhs, rhs)?;
                    Ok(Some(format!("{l} >= {r}")))
                }
                V2BinOp::Le => {
                    let (l, r) = self.lower_cmp_operands(lhs, rhs)?;
                    Ok(Some(format!("{l} > {r}")))
                }
                V2BinOp::Gt => {
                    let (l, r) = self.lower_cmp_operands(lhs, rhs)?;
                    Ok(Some(format!("{l} <= {r}")))
                }
                V2BinOp::Ge => {
                    let (l, r) = self.lower_cmp_operands(lhs, rhs)?;
                    Ok(Some(format!("{l} < {r}")))
                }
                V2BinOp::FlagTest => {
                    let (l, r) = self.lower_cmp_operands(lhs, rhs)?;
                    Ok(Some(format!("!{l} flag_test {r}")))
                }
                _ => Err(Error::at(
                    expr.span(),
                    "cannot apply '!' to arithmetic expression",
                )),
            },
            V2Expr::Call { .. } => {
                if let Some(pos) = self.try_lower_simple_air_cond(expr)? {
                    return Ok(Some(format!("!{pos}")));
                }
                Ok(None)
            }
            V2Expr::MethodCall { method, .. } if method == "run_functional" => Ok(None),
            _ => {
                let (val, ty) = self.lower_value_expr(expr)?;
                if ty != ValueType::Bool {
                    return Err(Error::at(
                        expr.span(),
                        "cannot apply '!' to non-bool expression",
                    ));
                }
                Ok(Some(format!("{val} == 0")))
            }
        }
    }

    /// Evaluates an arbitrary boolean expression (including short-circuit `&&`, `||`, `!`,
    /// helper calls, and `run_functional`) into `dst_local` (`0` or `1`).
    fn emit_bool_into_local(&mut self, expr: &V2Expr, dst_local: &str) -> Result<()> {
        if let Some(b) = self.try_eval_const_bool(expr) {
            self.line(format!("{dst_local} := {};", if b { 1 } else { 0 }));
            return Ok(());
        }
        match expr {
            V2Expr::Binary {
                lhs,
                op: V2BinOp::And,
                rhs,
                ..
            } => {
                // Short-circuit `lhs && rhs`: evaluate rhs ONLY when lhs is true!
                self.emit_bool_into_local(lhs, dst_local)?;
                self.line(format!("if {dst_local} != 0 {{"));
                self.indent += 1;
                self.emit_bool_into_local(rhs, dst_local)?;
                self.indent -= 1;
                self.line("}".to_owned());
                Ok(())
            }
            V2Expr::Binary {
                lhs,
                op: V2BinOp::Or,
                rhs,
                ..
            } => {
                // Short-circuit `lhs || rhs`: evaluate rhs ONLY when lhs is false!
                self.emit_bool_into_local(lhs, dst_local)?;
                self.line(format!("if {dst_local} == 0 {{"));
                self.indent += 1;
                self.emit_bool_into_local(rhs, dst_local)?;
                self.indent -= 1;
                self.line("}".to_owned());
                Ok(())
            }
            V2Expr::Unary {
                op: V2UnaryOp::Not,
                inner,
                ..
            } => {
                if let Some(neg) = self.try_lower_negated_simple_air_cond(inner)? {
                    self.line(format!("if {neg} {{"));
                    self.indent += 1;
                    self.line(format!("{dst_local} := 1;"));
                    self.indent -= 1;
                    self.line("} else {".to_owned());
                    self.indent += 1;
                    self.line(format!("{dst_local} := 0;"));
                    self.indent -= 1;
                    self.line("}".to_owned());
                } else {
                    self.emit_bool_into_local(inner, dst_local)?;
                    self.line(format!("if {dst_local} == 0 {{"));
                    self.indent += 1;
                    self.line(format!("{dst_local} := 1;"));
                    self.indent -= 1;
                    self.line("} else {".to_owned());
                    self.indent += 1;
                    self.line(format!("{dst_local} := 0;"));
                    self.indent -= 1;
                    self.line("}".to_owned());
                }
                Ok(())
            }
            V2Expr::MethodCall {
                span,
                receiver,
                method,
                args,
            } if method == "run_functional" => {
                self.emit_run_functional(receiver, args, *span, Some(dst_local))
            }
            V2Expr::Call { span, callee, args } => {
                if let Some(simple) = self.try_lower_simple_air_cond(expr)? {
                    self.line(format!("if {simple} {{"));
                    self.indent += 1;
                    self.line(format!("{dst_local} := 1;"));
                    self.indent -= 1;
                    self.line("} else {".to_owned());
                    self.indent += 1;
                    self.line(format!("{dst_local} := 0;"));
                    self.indent -= 1;
                    self.line("}".to_owned());
                    return Ok(());
                }
                if let V2Expr::Ident(_, fname) = callee.as_ref()
                    && let Some((fn_mod, fn_decl, is_local)) =
                        self.env
                            .lookup_fn(&self.owner_module, fname, self.in_object_scope)
                {
                    let rv = self
                        .inline_helper_call(&fn_mod, &fn_decl, is_local, args, *span)?
                        .ok_or_else(|| {
                            Error::at(
                                *span,
                                format!("void helper function '{fname}' cannot be used as bool"),
                            )
                        })?;
                    self.line(format!("{dst_local} := {rv};"));
                    return Ok(());
                }
                Err(Error::at(*span, "unsupported boolean call expression"))
            }
            _ => {
                let simple = self
                    .try_lower_simple_air_cond(expr)?
                    .ok_or_else(|| Error::at(expr.span(), "expected boolean expression"))?;
                self.line(format!("if {simple} {{"));
                self.indent += 1;
                self.line(format!("{dst_local} := 1;"));
                self.indent -= 1;
                self.line("} else {".to_owned());
                self.indent += 1;
                self.line(format!("{dst_local} := 0;"));
                self.indent -= 1;
                self.line("}".to_owned());
                Ok(())
            }
        }
    }

    /// Emits direct, zero-temporary short-circuit `return true` / `return false` for
    /// `guard(sim) => <expr>` and cleanup-free `return <expr>;`.
    fn emit_return_bool_expr(&mut self, expr: &V2Expr) -> Result<()> {
        if let Some(b) = self.try_eval_const_bool(expr) {
            self.line(if b {
                "return true;".to_owned()
            } else {
                "return false;".to_owned()
            });
            return Ok(());
        }
        match expr {
            V2Expr::Binary {
                lhs,
                op: V2BinOp::And,
                rhs,
                ..
            } => {
                if let Some(neg_lhs) = self.try_lower_negated_simple_air_cond(lhs)? {
                    self.line(format!("if {neg_lhs} {{"));
                    self.indent += 1;
                    self.line("return false;".to_owned());
                    self.indent -= 1;
                    self.line("}".to_owned());
                    self.emit_return_bool_expr(rhs)
                } else {
                    let tmp = self.fresh("and");
                    self.line(format!("let {tmp} = 0;"));
                    self.emit_bool_into_local(lhs, &tmp)?;
                    self.line(format!("if {tmp} == 0 {{"));
                    self.indent += 1;
                    self.line("return false;".to_owned());
                    self.indent -= 1;
                    self.line("}".to_owned());
                    self.emit_return_bool_expr(rhs)
                }
            }
            V2Expr::Binary {
                lhs,
                op: V2BinOp::Or,
                rhs,
                ..
            } => {
                if let Some(pos_lhs) = self.try_lower_simple_air_cond(lhs)? {
                    self.line(format!("if {pos_lhs} {{"));
                    self.indent += 1;
                    self.line("return true;".to_owned());
                    self.indent -= 1;
                    self.line("}".to_owned());
                    self.emit_return_bool_expr(rhs)
                } else {
                    let tmp = self.fresh("or");
                    self.line(format!("let {tmp} = 0;"));
                    self.emit_bool_into_local(lhs, &tmp)?;
                    self.line(format!("if {tmp} != 0 {{"));
                    self.indent += 1;
                    self.line("return true;".to_owned());
                    self.indent -= 1;
                    self.line("}".to_owned());
                    self.emit_return_bool_expr(rhs)
                }
            }
            _ => {
                if let Some(neg) = self.try_lower_negated_simple_air_cond(expr)? {
                    self.line(format!("if {neg} {{"));
                    self.indent += 1;
                    self.line("return false;".to_owned());
                    self.indent -= 1;
                    self.line("}".to_owned());
                    self.line("return true;".to_owned());
                    Ok(())
                } else {
                    let tmp = self.fresh("ret");
                    self.line(format!("let {tmp} = 0;"));
                    self.emit_bool_into_local(expr, &tmp)?;
                    self.line(format!("if {tmp} != 0 {{"));
                    self.indent += 1;
                    self.line("return true;".to_owned());
                    self.indent -= 1;
                    self.line("} else {".to_owned());
                    self.indent += 1;
                    self.line("return false;".to_owned());
                    self.indent -= 1;
                    self.line("}".to_owned());
                    Ok(())
                }
            }
        }
    }

    fn resolve_motive_expr(&self, expr: &V2Expr) -> Result<u8> {
        match expr {
            V2Expr::Ident(span, name) => {
                if let Some(Binding::Motive(m)) = self.lookup_var(name) {
                    return Ok(*m);
                }
                motive_by_name(name)
                    .ok_or_else(|| Error::at(*span, format!("unknown motive '{name}'")))
            }
            V2Expr::Path(span, segs) if segs.len() == 2 && segs[0] == "Motive" => {
                let lower = segs[1].to_ascii_lowercase();
                motive_by_name(&lower)
                    .ok_or_else(|| Error::at(*span, format!("unknown Motive::{}", segs[1])))
            }
            V2Expr::Int(span, v) => u8::try_from(*v)
                .ok()
                .filter(|&i| i < 16)
                .ok_or_else(|| Error::at(*span, format!("motive index {v} out of 0..16 range"))),
            other => Err(Error::at(
                other.span(),
                "expected motive name (e.g. energy, Motive::Energy) or index",
            )),
        }
    }

    fn resolve_sound_expr(&self, expr: &V2Expr) -> Result<u16> {
        match expr {
            V2Expr::Ident(span, name) => {
                if let Some(Binding::Sound(id)) = self.lookup_var(name) {
                    return Ok(*id);
                }
                self.env
                    .sounds
                    .get(name)
                    .map(|s| s.id)
                    .ok_or_else(|| Error::at(*span, format!("unknown sound '{name}'")))
            }
            other => Err(Error::at(
                other.span(),
                "expected typed sound identifier (declared with 'sound name = preset(...);')",
            )),
        }
    }

    fn resolve_slot_expr(&self, expr: &V2Expr) -> Result<u16> {
        match expr {
            V2Expr::Ident(span, name) => {
                if let Some(Binding::Slot(idx)) = self.lookup_var(name) {
                    return Ok(*idx);
                }
                match self.env.slots.get(name) {
                    Some((Some(idx), _)) => Ok(*idx),
                    Some((None, _)) => Err(Error::at(
                        *span,
                        format!(
                            "slot '{name}' is not a routing slot (only 'kind: routing' slots can be used with route_to/snap/Slot)"
                        ),
                    )),
                    None => Err(Error::at(*span, format!("unknown slot '{name}'"))),
                }
            }
            V2Expr::Int(span, v) => u16::try_from(*v)
                .map_err(|_| Error::at(*span, format!("slot index {v} out of u16 range"))),
            other => Err(Error::at(other.span(), "expected slot identifier or index")),
        }
    }

    fn format_play_sound(&self, args: &[CallArg], span: Span) -> Result<String> {
        if args.is_empty() || args.len() > 2 {
            return Err(Error::at(
                span,
                "play_sound(sound [, target]) expects 1 or 2 arguments",
            ));
        }
        let sound_id = self.resolve_sound_expr(&args[0].value)?;
        let use_stack_obj = if let Some(target_arg) = args.get(1) {
            match &target_arg.value {
                V2Expr::Ident(tspan, name) => {
                    if self.is_self_ident(name) {
                        matches!(self.tree_ctx, TreeContext::SimOnObject { .. })
                    } else if self.is_sim_ident(name) {
                        false
                    } else {
                        return Err(Error::at(
                            *tspan,
                            format!("play_sound target must be 'self' or 'sim', found '{name}'"),
                        ));
                    }
                }
                other => {
                    return Err(Error::at(
                        other.span(),
                        "play_sound target must be 'self' or 'sim'",
                    ));
                }
            }
        } else {
            matches!(self.tree_ctx, TreeContext::SimOnObject { .. })
        };
        if use_stack_obj {
            Ok(format!("play_sound({sound_id}, stack_obj)"))
        } else {
            Ok(format!("play_sound({sound_id}, false)"))
        }
    }

    fn format_stop_all_sounds(&self, args: &[CallArg], span: Span) -> Result<String> {
        let use_stack = if let Some(arg) = args.first() {
            match &arg.value {
                V2Expr::Ident(_, n) if self.is_self_ident(n) => {
                    matches!(self.tree_ctx, TreeContext::SimOnObject { .. })
                }
                V2Expr::Ident(_, n) if self.is_sim_ident(n) => false,
                _ => {
                    return Err(Error::at(
                        span,
                        "stop_all_sounds target must be 'self' or 'sim'",
                    ));
                }
            }
        } else {
            matches!(self.tree_ctx, TreeContext::SimOnObject { .. })
        };
        Ok(format!("stop_all_sounds({use_stack})"))
    }

    fn format_alter_budget(
        &mut self,
        args: &[CallArg],
        span: Span,
        test_only: bool,
        income: bool,
    ) -> Result<String> {
        if args.is_empty() {
            return Err(Error::at(span, "budget call requires an amount argument"));
        }
        let (amount_air, _) = self.lower_value_expr(&args[0].value)?;
        let mut category = 0u8;
        for arg in &args[1..] {
            match arg.name.as_deref() {
                Some("category") => {
                    let c = self.eval_const_i16(&arg.value)?;
                    category = u8::try_from(c).map_err(|_| {
                        Error::at(arg.span, format!("budget category {c} out of u8 range"))
                    })?;
                }
                Some(other) => {
                    return Err(Error::at(
                        arg.span,
                        format!("unknown budget option '{other}'"),
                    ));
                }
                None => {
                    return Err(Error::at(
                        arg.span,
                        "extra budget arguments must be named (e.g. category: 5)",
                    ));
                }
            }
        }
        let mut out = format!("alter_budget({amount_air}, category: {category}");
        if test_only {
            out.push_str(", test_only: true");
        }
        if income {
            out.push_str(", income: true");
        }
        out.push(')');
        Ok(out)
    }

    fn format_find_best_object(&self, args: &[CallArg], span: Span) -> Result<String> {
        if args.is_empty() {
            return Err(Error::at(
                span,
                "find_best_object(kind [, flags: N]) requires a functional kind",
            ));
        }
        let V2Expr::Ident(kspan, kind_name) = &args[0].value else {
            return Err(Error::at(
                args[0].span,
                "find_best_object() expects a functional kind identifier (e.g. prep, cook)",
            ));
        };
        let _ = functional_kind_info(kind_name, *kspan)?;
        let mut flags: Option<u8> = None;
        for arg in &args[1..] {
            if arg.name.as_deref() == Some("flags") || arg.name.is_none() {
                let f = self.eval_const_i16(&arg.value)?;
                flags = Some(
                    u8::try_from(f)
                        .map_err(|_| Error::at(arg.span, format!("flags {f} out of u8 range")))?,
                );
            } else {
                return Err(Error::at(
                    arg.span,
                    format!(
                        "unknown find_best_object option '{}'",
                        arg.name.as_deref().unwrap_or("")
                    ),
                ));
            }
        }
        Ok(match flags {
            Some(f) => format!("find_best_object({kind_name}, flags: {f})"),
            None => format!("find_best_object({kind_name})"),
        })
    }

    fn format_create_object(&self, args: &[CallArg], span: Span) -> Result<String> {
        if args.is_empty() {
            return Err(Error::at(
                span,
                "create_object(guid [, mode: ..., flags: ...]) requires a GUID",
            ));
        }
        let V2Expr::Int(gspan, gval) = args[0].value else {
            return Err(Error::at(
                args[0].span,
                "create_object() requires an integer GUID literal",
            ));
        };
        let guid = u32::try_from(gval)
            .map_err(|_| Error::at(gspan, format!("GUID {gval} out of u32 range")))?;
        let mut mode = "here".to_owned();
        let mut flags = 0u8;
        for arg in &args[1..] {
            match arg.name.as_deref() {
                Some("mode") => match &arg.value {
                    V2Expr::Ident(_, m) => mode = m.clone(),
                    V2Expr::Int(_, n) => mode = n.to_string(),
                    other => {
                        return Err(Error::at(
                            other.span(),
                            "create_object mode must be an identifier (e.g. stack_ahead) or integer",
                        ));
                    }
                },
                Some("flags") => {
                    let f = self.eval_const_i16(&arg.value)?;
                    flags = u8::try_from(f)
                        .map_err(|_| Error::at(arg.span, format!("flags {f} out of u8 range")))?;
                }
                Some(other) => {
                    return Err(Error::at(
                        arg.span,
                        format!("unknown create_object option '{other}'"),
                    ));
                }
                None => {
                    return Err(Error::at(
                        arg.span,
                        "extra create_object arguments must be named ('mode:', 'flags:')",
                    ));
                }
            }
        }
        Ok(format!(
            "create_object({guid:#010x}, mode: {mode}, flags: {flags:#04x})"
        ))
    }

    fn format_slot_prim(&self, prim: &str, args: &[CallArg], span: Span) -> Result<String> {
        if args.is_empty() {
            return Err(Error::at(
                span,
                format!("{prim}(slot [, flags: N]) requires a slot"),
            ));
        }
        let slot_idx = self.resolve_slot_expr(&args[0].value)?;
        let mut flags = 0u8;
        if let Some(farg) = args.get(1) {
            let f = self.eval_const_i16(&farg.value)?;
            flags = u8::try_from(f)
                .map_err(|_| Error::at(farg.span, format!("flags {f} out of u8 range")))?;
        }
        if flags == 0 {
            Ok(format!("{prim}({slot_idx})"))
        } else {
            Ok(format!("{prim}({slot_idx}, {flags})"))
        }
    }

    fn format_set_motive_change(&mut self, args: &[CallArg], span: Span) -> Result<String> {
        if args.len() != 3 {
            return Err(Error::at(
                span,
                "set_motive_change(motive, delta_per_hour, stop_at) expects 3 arguments",
            ));
        }
        let m = self.resolve_motive_expr(&args[0].value)?;
        let (delta, _) = self.lower_value_expr(&args[1].value)?;
        let delta_arg = if self.is_pure_single_operand(&args[2].value) {
            delta
        } else {
            let snap = self.fresh("mc_d");
            self.line(format!("let {snap} = {delta};"));
            snap
        };
        let (stop, _) = self.lower_value_expr(&args[2].value)?;
        Ok(format!("set_motive_change({m}, {delta_arg}, {stop})"))
    }

    fn format_dialog(&self, args: &[CallArg], span: Span) -> Result<String> {
        let mut parts = Vec::new();
        for arg in args {
            let Some(k) = arg.name.as_deref() else {
                return Err(Error::at(arg.span, "dialog() arguments must be named"));
            };
            let formatted = match k {
                "kind" => match &arg.value {
                    V2Expr::Ident(_, s) => format!("kind: {s}"),
                    V2Expr::Int(_, n) => format!("kind: {n}"),
                    _ => return Err(Error::at(arg.span, "invalid dialog kind")),
                },
                "title" | "message" | "text" | "button1" | "button2" | "button3" | "mode4_str" => {
                    match &arg.value {
                        V2Expr::Ident(ispan, sname) => {
                            if !self.env.string_names.contains_key(sname) {
                                return Err(Error::at(
                                    *ispan,
                                    format!(
                                        "unknown string entry '{sname}' (declare it in 'strings {{ ... }}')"
                                    ),
                                ));
                            }
                            format!("{k}: str.{sname}")
                        }
                        V2Expr::Int(_, n) => format!("{k}: {n}"),
                        _ => return Err(Error::at(arg.span, format!("invalid dialog {k}"))),
                    }
                }
                "temp" | "icon_mode" | "flags" => {
                    let n = self.eval_const_i16(&arg.value)?;
                    format!("{k}: {n}")
                }
                "inform" | "non_modal" => {
                    let b = self.eval_const_bool_required(&arg.value, arg.span)?;
                    format!("{k}: {b}")
                }
                other => {
                    return Err(Error::at(
                        arg.span,
                        format!("unknown dialog() argument '{other}'"),
                    ));
                }
            };
            parts.push(formatted);
        }
        let _ = span;
        Ok(format!("dialog({})", parts.join(", ")))
    }
}

fn cmp_op_str(op: V2BinOp) -> &'static str {
    match op {
        V2BinOp::Eq => "==",
        V2BinOp::Ne => "!=",
        V2BinOp::Lt => "<",
        V2BinOp::Le => "<=",
        V2BinOp::Gt => ">",
        V2BinOp::Ge => ">=",
        V2BinOp::FlagTest => "flag_test",
        _ => unreachable!("non-comparison op"),
    }
}

fn collect_raw_air_labels<'a>(stmts: &'a [crate::ast::Stmt], labels: &mut BTreeSet<&'a str>) {
    for s in stmts {
        match s {
            crate::ast::Stmt::Label { name, .. } => {
                labels.insert(name.as_str());
            }
            crate::ast::Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                collect_raw_air_labels(then_body, labels);
                collect_raw_air_labels(else_body, labels);
            }
            crate::ast::Stmt::While { body, .. } => {
                collect_raw_air_labels(body, labels);
            }
            _ => {}
        }
    }
}

fn validate_raw_air_target(
    target: &crate::ast::Target,
    defined_labels: &BTreeSet<&str>,
    span: Span,
) -> Result<()> {
    match target {
        crate::ast::Target::Label(name) if defined_labels.contains(name.as_str()) => Ok(()),
        other => Err(Error::at(
            span,
            format!(
                "'raw {{ ... }}' cannot branch to '{other:?}' while cleanups, inline helpers, or cancel_on_abort loops are active"
            ),
        )),
    }
}

fn validate_raw_air_stmts(
    stmts: &[crate::ast::Stmt],
    in_raw_loop: bool,
    defined_labels: &BTreeSet<&str>,
    span: Span,
) -> Result<()> {
    for s in stmts {
        match s {
            crate::ast::Stmt::Return { .. } => {
                return Err(Error::at(
                    span,
                    "'raw { ... }' cannot contain 'return' while cleanups, inline helpers, or cancel_on_abort loops are active",
                ));
            }
            crate::ast::Stmt::Break { .. } | crate::ast::Stmt::Continue { .. } if !in_raw_loop => {
                return Err(Error::at(
                    span,
                    "'raw { ... }' cannot break/continue out of its block while cleanups, inline helpers, or cancel_on_abort loops are active",
                ));
            }
            crate::ast::Stmt::RawOp { branches, .. } => {
                validate_raw_air_target(&branches.on_true, defined_labels, span)?;
                validate_raw_air_target(&branches.on_false, defined_labels, span)?;
            }
            crate::ast::Stmt::Primitive {
                branches: Some(branches),
                ..
            } => {
                validate_raw_air_target(&branches.on_true, defined_labels, span)?;
                validate_raw_air_target(&branches.on_false, defined_labels, span)?;
            }
            crate::ast::Stmt::Call {
                branches: Some(branches),
                ..
            } => {
                validate_raw_air_target(&branches.on_true, defined_labels, span)?;
                validate_raw_air_target(&branches.on_false, defined_labels, span)?;
            }
            crate::ast::Stmt::Call { branches: None, .. } => {
                return Err(Error::at(
                    span,
                    "'raw { ... }' cannot invoke an unbranched subroutine call (whose false branch exits the tree) while cleanups, inline helpers, or cancel_on_abort loops are active",
                ));
            }
            crate::ast::Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                validate_raw_air_stmts(then_body, in_raw_loop, defined_labels, span)?;
                validate_raw_air_stmts(else_body, in_raw_loop, defined_labels, span)?;
            }
            crate::ast::Stmt::While { body, .. } => {
                validate_raw_air_stmts(body, true, defined_labels, span)?;
            }
            _ => {}
        }
    }
    Ok(())
}

fn ends_with_unconditional_return(stmts: &[V2Stmt]) -> bool {
    matches!(stmts.last(), Some(V2Stmt::Return { .. }))
}

fn stmts_unconditionally_diverge(stmts: &[V2Stmt]) -> bool {
    match stmts.last() {
        Some(V2Stmt::Return { .. } | V2Stmt::Break { .. } | V2Stmt::Continue { .. }) => true,
        Some(V2Stmt::If {
            then_body,
            else_body,
            ..
        }) => {
            !else_body.is_empty()
                && stmts_unconditionally_diverge(then_body)
                && stmts_unconditionally_diverge(else_body)
        }
        Some(V2Stmt::WithLock { body, .. }) => stmts_unconditionally_diverge(body),
        _ => false,
    }
}

fn stmts_have_return(stmts: &[V2Stmt]) -> bool {
    stmts.iter().any(|s| match s {
        V2Stmt::Return { .. } => true,
        V2Stmt::WithLock { body, .. }
        | V2Stmt::Defer { body, .. }
        | V2Stmt::Repeat { body, .. }
        | V2Stmt::While { body, .. }
        | V2Stmt::ForEach { body, .. } => stmts_have_return(body),
        V2Stmt::LetSome { else_body, .. } => stmts_have_return(else_body),
        V2Stmt::If {
            then_body,
            else_body,
            ..
        } => stmts_have_return(then_body) || stmts_have_return(else_body),
        _ => false,
    })
}

fn stmts_always_return(stmts: &[V2Stmt]) -> bool {
    for stmt in stmts {
        match stmt {
            V2Stmt::Return { .. } => return true,
            V2Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                if !else_body.is_empty()
                    && stmts_always_return(then_body)
                    && stmts_always_return(else_body)
                {
                    return true;
                }
            }
            V2Stmt::WithLock { body, .. } if stmts_always_return(body) => return true,
            _ => {}
        }
    }
    false
}

fn stmts_may_clobber_stack_obj(
    stmts: &[V2Stmt],
    env: &ObjectEnv<'_>,
    owner_mod: &str,
    allow_local_fns: bool,
    depth: usize,
) -> bool {
    if depth > MAX_INLINE_DEPTH {
        return true;
    }
    stmts.iter().any(|s| match s {
        V2Stmt::LetSome {
            init, else_body, ..
        } => {
            expr_may_clobber_stack_obj(init, env, owner_mod, allow_local_fns, depth)
                || stmts_may_clobber_stack_obj(else_body, env, owner_mod, allow_local_fns, depth)
        }
        V2Stmt::Let { init, .. } => {
            expr_may_clobber_stack_obj(init, env, owner_mod, allow_local_fns, depth)
        }
        V2Stmt::Assign { target, value, .. } => {
            expr_may_clobber_stack_obj(target, env, owner_mod, allow_local_fns, depth)
                || expr_may_clobber_stack_obj(value, env, owner_mod, allow_local_fns, depth)
        }
        V2Stmt::WithLock {
            lock_expr, body, ..
        } => {
            expr_may_clobber_stack_obj(lock_expr, env, owner_mod, allow_local_fns, depth)
                || stmts_may_clobber_stack_obj(body, env, owner_mod, allow_local_fns, depth)
        }
        V2Stmt::Defer { body, .. } => {
            stmts_may_clobber_stack_obj(body, env, owner_mod, allow_local_fns, depth)
        }
        V2Stmt::Repeat { count, body, .. } => {
            expr_may_clobber_stack_obj(count, env, owner_mod, allow_local_fns, depth)
                || stmts_may_clobber_stack_obj(body, env, owner_mod, allow_local_fns, depth)
        }
        V2Stmt::While { cond, body, .. } => {
            expr_may_clobber_stack_obj(cond, env, owner_mod, allow_local_fns, depth)
                || stmts_may_clobber_stack_obj(body, env, owner_mod, allow_local_fns, depth)
        }
        V2Stmt::ForEach { .. } | V2Stmt::RawAir { .. } => true,
        V2Stmt::If {
            cond,
            then_body,
            else_body,
            ..
        } => {
            expr_may_clobber_stack_obj(cond, env, owner_mod, allow_local_fns, depth)
                || stmts_may_clobber_stack_obj(then_body, env, owner_mod, allow_local_fns, depth)
                || stmts_may_clobber_stack_obj(else_body, env, owner_mod, allow_local_fns, depth)
        }
        V2Stmt::Return { value: Some(e), .. } | V2Stmt::Expr { expr: e, .. } => {
            expr_may_clobber_stack_obj(e, env, owner_mod, allow_local_fns, depth)
        }
        _ => false,
    })
}

fn expr_may_clobber_stack_obj(
    expr: &V2Expr,
    env: &ObjectEnv<'_>,
    owner_mod: &str,
    allow_local_fns: bool,
    depth: usize,
) -> bool {
    if depth > MAX_INLINE_DEPTH {
        return true;
    }
    match expr {
        V2Expr::Call { callee, args, .. } => {
            if let V2Expr::Ident(_, fname) = callee.as_ref() {
                if matches!(fname.as_str(), "find_best_object" | "create_object") {
                    return true;
                }
                if let Some((fmod, fdecl, is_local)) =
                    env.lookup_fn(owner_mod, fname, allow_local_fns)
                    && stmts_may_clobber_stack_obj(&fdecl.body, env, &fmod, is_local, depth + 1)
                {
                    return true;
                }
            }
            args.iter().any(|a| {
                expr_may_clobber_stack_obj(&a.value, env, owner_mod, allow_local_fns, depth)
            })
        }
        V2Expr::MethodCall {
            receiver,
            method,
            args,
            ..
        } => {
            method == "run_functional"
                || expr_may_clobber_stack_obj(receiver, env, owner_mod, allow_local_fns, depth)
                || args.iter().any(|a| {
                    expr_may_clobber_stack_obj(&a.value, env, owner_mod, allow_local_fns, depth)
                })
        }
        V2Expr::Index { base, index, .. } => {
            // Foreign `obj.attr[i]` or `obj.data[i]` swaps `stack_obj`!
            if let V2Expr::Field {
                base: root, field, ..
            } = base.as_ref()
                && matches!(field.as_str(), "attr" | "data")
                && let V2Expr::Ident(_, rname) = root.as_ref()
                && rname != "self"
                && rname != "sim"
            {
                return true;
            }
            expr_may_clobber_stack_obj(base, env, owner_mod, allow_local_fns, depth)
                || expr_may_clobber_stack_obj(index, env, owner_mod, allow_local_fns, depth)
        }
        V2Expr::Field { base, .. }
        | V2Expr::Deref { inner: base, .. }
        | V2Expr::MutRef { inner: base, .. }
        | V2Expr::Unary { inner: base, .. } => {
            expr_may_clobber_stack_obj(base, env, owner_mod, allow_local_fns, depth)
        }
        V2Expr::Binary { lhs, rhs, .. } => {
            expr_may_clobber_stack_obj(lhs, env, owner_mod, allow_local_fns, depth)
                || expr_may_clobber_stack_obj(rhs, env, owner_mod, allow_local_fns, depth)
        }
        _ => false,
    }
}

fn make_slug(name: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "object".to_owned()
    } else {
        out
    }
}

fn camel_label(name: &str) -> String {
    let mut out = String::new();
    let mut cap = true;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            if cap {
                out.push(ch.to_ascii_uppercase());
                cap = false;
            } else {
                out.push(ch);
            }
        } else {
            cap = true;
        }
    }
    if out.is_empty() {
        "Object".to_owned()
    } else {
        out
    }
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => {
            let mut out = first.to_uppercase().to_string();
            out.push_str(chars.as_str());
            out
        }
        None => String::new(),
    }
}

fn sanitize_tree_ident(label: &str, idx: usize) -> String {
    let camel = camel_label(label);
    if camel.is_empty() {
        format!("Interaction_{idx}")
    } else {
        camel
    }
}

fn escape_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\0' => out.push_str("\\0"),
            c if c.is_control() => {
                for b in c.encode_utf8(&mut [0u8; 4]).as_bytes() {
                    let _ = write!(out, "\\x{b:02x}");
                }
            }
            c => out.push(c),
        }
    }
    out
}

#[allow(dead_code)]
fn _keep_furniture_shape_import(_: FurnitureShape) {}
