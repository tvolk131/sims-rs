//! Object expansion: one concrete `object` declaration to `.air` trees plus its `ObjectManifest`,
//! and variant resolution.

use super::*;

pub(super) fn lower_object(
    resolved: &ResolvedModule,
    obj: &ObjectDecl,
) -> Result<ExpandedV2Object> {
    lower_object_with(resolved, obj, None, false)
}

/// The frame words the object's strings may make the formatter read (see
/// [`mir::strings_read`]): its `strings` tables, its `STR#` 301 dialog table (declared entries and
/// interned inline literals), and its catalog strings (each `CTSS`: a name and a description).
fn object_text_reads(
    strings: &BTreeMap<u16, StringsDecl>,
    dialog_strings: &DialogStringTable,
    catalog: &[&str],
) -> mir::TableLocals {
    mir::strings_read(
        strings
            .values()
            .flat_map(|t| t.entries.iter().map(|(_, _, text)| text.as_str()))
            .chain(
                dialog_strings
                    .entries
                    .iter()
                    .map(|(_, _, text)| text.as_str()),
            )
            .chain(catalog.iter().copied()),
    )
}

/// [`lower_object`], finishing every tree with `text_reads` as the object's text fact when it is
/// given, and otherwise with the fact the strings known before any tree is lowered give; with
/// `in_place_off`, no tree gets an in-place rewrite (a `job_data` comparison without its copy).
fn lower_object_with(
    resolved: &ResolvedModule,
    obj: &ObjectDecl,
    text_reads: Option<mir::TableLocals>,
    in_place_off: bool,
) -> Result<ExpandedV2Object> {
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
    let mut explicit_attributes: Option<u16> = None;
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
    let mut load_blocks: Vec<OwnedStmtBlock> = Vec::new();
    let mut load_pinned_id: Option<u16> = None;
    let mut reset_blocks: Vec<OwnedStmtBlock> = Vec::new();
    let mut reset_pinned_id: Option<u16> = None;
    let mut cleanup_blocks: Vec<OwnedStmtBlock> = Vec::new();
    let mut cleanup_pinned_id: Option<u16> = None;
    // `@frame(...)` per lifecycle tree ("Init", "Main", "Load", "Reset", "Cleanup").
    let mut lifecycle_frames: BTreeMap<&'static str, TreeFrame> = BTreeMap::new();
    let mut note_lifecycle_frame = |label: &'static str, frame: Option<TreeFrame>| -> Result<()> {
        let Some(f) = frame else {
            return Ok(());
        };
        if f.empty {
            return Err(Error::at(
                f.span,
                "@frame(empty) is only allowed on a standalone 'tree' (a lifecycle tree always has nodes)",
            ));
        }
        if lifecycle_frames.insert(label, f).is_some() {
            return Err(Error::at(
                f.span,
                format!("@frame(...) on more than one block of the '{label}' tree"),
            ));
        }
        Ok(())
    };

    let mut local_enums_raw: BTreeMap<String, (String, EnumDecl)> = BTreeMap::new();
    let mut local_bitflags_raw: BTreeMap<String, (String, BitflagsDecl)> = BTreeMap::new();
    let mut state_owners: BTreeMap<String, String> = BTreeMap::new();

    let mut functionals: Vec<OwnedFunctional> = Vec::new();
    let mut interactions: Vec<OwnedInteraction> = Vec::new();
    let mut local_fns: BTreeMap<String, (String, FnDecl)> = BTreeMap::new();
    let mut local_extern_mods: BTreeMap<String, (String, ExternModDecl)> = BTreeMap::new();
    let mut local_extern_bcons: BTreeMap<String, (String, ExternBconDecl)> = BTreeMap::new();
    let mut local_consts: BTreeMap<String, i64> = BTreeMap::new();
    let mut bcon_decls: Vec<BconTableDecl> = Vec::new();
    let mut tree_members: Vec<OwnedTree> = Vec::new();
    let mut variant_decls: Vec<VariantDecl> = Vec::new();
    // Every module that contributes members (templates + the object): helper-fn lookups from any
    // of their code must not be shadowed by an (inherited) tree ident.
    let mut member_modules: BTreeSet<String> = BTreeSet::new();

    let mut apply_members = |owner_mod: &str,
                             members: &[ObjectMember],
                             is_override: bool|
     -> Result<()> {
        member_modules.insert(owner_mod.to_owned());
        let mut seen_sounds_here = BTreeSet::new();
        let mut seen_slots_here = BTreeSet::new();
        let mut seen_functionals_here = BTreeSet::new();
        let mut seen_interactions_here = BTreeSet::new();
        let mut seen_fns_here = BTreeSet::new();
        let mut seen_types_here = BTreeSet::new();
        let mut seen_externs_here = BTreeSet::new();
        let mut seen_bcons_here = BTreeSet::new();
        let mut seen_trees_here = BTreeSet::new();
        let mut seen_consts_here = BTreeSet::new();
        for member in members {
            match member {
                ObjectMember::Const(c) => {
                    if !seen_consts_here.insert(c.name.clone()) {
                        return Err(Error::at(c.span, format!("duplicate const '{}'", c.name)));
                    }
                    local_consts.insert(c.name.clone(), c.value);
                }
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
                    MetaFieldKind::Attributes(v) => explicit_attributes = Some(*v),
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
                    if matches!(
                        s.name.as_str(),
                        "id" | "room"
                            | "graphic"
                            | "prep_score"
                            | "cook_score"
                            | "in_use"
                            | "allowed_heights"
                            | "placement"
                            | "attr"
                            | "data"
                            | "parts"
                    ) {
                        return Err(Error::at(
                            s.span,
                            format!(
                                "state name '{}' conflicts with built-in object property 'self.{}'",
                                s.name, s.name
                            ),
                        ));
                    }
                    if raw_states.contains_key(&s.name) {
                        return Err(Error::at(
                            s.span,
                            format!("duplicate state declaration '{}'", s.name),
                        ));
                    }
                    state_order.push(s.name.clone());
                    state_owners.insert(s.name.clone(), owner_mod.to_owned());
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
                ObjectMember::Enum(e) => {
                    if !seen_types_here.insert(e.name.clone())
                        || local_enums_raw.contains_key(&e.name)
                        || local_bitflags_raw.contains_key(&e.name)
                    {
                        return Err(Error::at(
                            e.span,
                            format!("duplicate local type declaration '{}'", e.name),
                        ));
                    }
                    local_enums_raw.insert(e.name.clone(), (owner_mod.to_owned(), e.clone()));
                }
                ObjectMember::Bitflags(b) => {
                    if !seen_types_here.insert(b.name.clone())
                        || local_enums_raw.contains_key(&b.name)
                        || local_bitflags_raw.contains_key(&b.name)
                    {
                        return Err(Error::at(
                            b.span,
                            format!("duplicate local type declaration '{}'", b.name),
                        ));
                    }
                    local_bitflags_raw.insert(b.name.clone(), (owner_mod.to_owned(), b.clone()));
                }
                ObjectMember::Init(b) => {
                    note_lifecycle_frame("Init", b.frame)?;
                    if let Some(pid) = b.pinned_id {
                        init_pinned_id = Some(pid);
                    }
                    init_blocks.push(OwnedStmtBlock {
                        owner_module: owner_mod.to_owned(),
                        stmts: b.body.clone(),
                    });
                }
                ObjectMember::Main(b) => {
                    note_lifecycle_frame("Main", b.frame)?;
                    if let Some(pid) = b.pinned_id {
                        main_pinned_id = Some(pid);
                    }
                    main_block = Some(OwnedStmtBlock {
                        owner_module: owner_mod.to_owned(),
                        stmts: b.body.clone(),
                    });
                }
                ObjectMember::Load(b) => {
                    note_lifecycle_frame("Load", b.frame)?;
                    if let Some(pid) = b.pinned_id {
                        load_pinned_id = Some(pid);
                    }
                    load_blocks.push(OwnedStmtBlock {
                        owner_module: owner_mod.to_owned(),
                        stmts: b.body.clone(),
                    });
                }
                ObjectMember::Reset(b) => {
                    note_lifecycle_frame("Reset", b.frame)?;
                    if let Some(pid) = b.pinned_id {
                        reset_pinned_id = Some(pid);
                    }
                    reset_blocks.push(OwnedStmtBlock {
                        owner_module: owner_mod.to_owned(),
                        stmts: b.body.clone(),
                    });
                }
                ObjectMember::Cleanup(b) => {
                    note_lifecycle_frame("Cleanup", b.frame)?;
                    if let Some(pid) = b.pinned_id {
                        cleanup_pinned_id = Some(pid);
                    }
                    cleanup_blocks.push(OwnedStmtBlock {
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
                ObjectMember::ExternMod(m) => {
                    if !seen_externs_here.insert(m.name.clone())
                        || (local_extern_mods.contains_key(&m.name) && !is_override)
                        || local_extern_bcons.contains_key(&m.name)
                    {
                        return Err(Error::at(
                            m.span,
                            format!("duplicate extern block '{}'", m.name),
                        ));
                    }
                    local_extern_mods.insert(m.name.clone(), (owner_mod.to_owned(), m.clone()));
                }
                ObjectMember::ExternBcon(b) => {
                    if !seen_externs_here.insert(b.name.clone())
                        || (local_extern_bcons.contains_key(&b.name) && !is_override)
                        || local_extern_mods.contains_key(&b.name)
                    {
                        return Err(Error::at(
                            b.span,
                            format!("duplicate extern block '{}'", b.name),
                        ));
                    }
                    local_extern_bcons.insert(b.name.clone(), (owner_mod.to_owned(), b.clone()));
                }
                ObjectMember::Bcon(t) => {
                    if !seen_bcons_here.insert(t.id) {
                        return Err(Error::at(
                            t.span,
                            format!("duplicate BCON table id {}", t.id),
                        ));
                    }
                    if let Some(pos) = bcon_decls.iter().position(|d| d.id == t.id) {
                        if !is_override {
                            return Err(Error::at(
                                t.span,
                                format!("duplicate BCON table id {}", t.id),
                            ));
                        }
                        bcon_decls[pos] = t.clone();
                    } else {
                        bcon_decls.push(t.clone());
                    }
                }
                ObjectMember::Variant(v) => {
                    if variant_decls.iter().any(|x| x.name == v.name) {
                        return Err(Error::at(v.span, format!("duplicate variant '{}'", v.name)));
                    }
                    variant_decls.push(v.clone());
                }
                ObjectMember::Tree(t) => {
                    if !seen_trees_here.insert(t.ident.clone()) {
                        return Err(Error::at(t.span, format!("duplicate tree '{}'", t.ident)));
                    }
                    let owned = OwnedTree {
                        owner_module: owner_mod.to_owned(),
                        decl: t.clone(),
                    };
                    if let Some(pos) = tree_members.iter().position(|x| x.decl.ident == t.ident) {
                        if !is_override {
                            return Err(Error::at(t.span, format!("duplicate tree '{}'", t.ident)));
                        }
                        tree_members[pos] = owned;
                    } else {
                        tree_members.push(owned);
                    }
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

    // 3b. Resolve all enums and bitflags (built-in + module-level + object/template-local).
    let mut all_enums: BTreeMap<String, ResolvedEnum> = BTreeMap::new();
    let mut all_bitflags: BTreeMap<String, ResolvedBitflags> = builtin_bitflags();
    for scope in resolved.module_scopes.values() {
        for (def_mod, edecl) in scope.enums.values() {
            let key = format!("enum::{def_mod}::{}", edecl.name);
            all_enums
                .entry(key.clone())
                .or_insert_with(|| resolve_enum_decl(key, edecl));
        }
        for (def_mod, bdecl) in scope.bitflags.values() {
            let key = format!("bitflags::{def_mod}::{}", bdecl.name);
            all_bitflags
                .entry(key.clone())
                .or_insert_with(|| resolve_bitflags_decl(key, bdecl));
        }
    }
    let mut local_enums: BTreeMap<String, String> = BTreeMap::new();
    for (ename, (owner_mod, edecl)) in &local_enums_raw {
        let key = format!("local_enum::{owner_mod}::{slug}::{}", edecl.name);
        all_enums.insert(key.clone(), resolve_enum_decl(key.clone(), edecl));
        local_enums.insert(ename.clone(), key);
    }
    let mut local_bitflags: BTreeMap<String, String> = BTreeMap::new();
    for (bname, (owner_mod, bdecl)) in &local_bitflags_raw {
        let key = format!("local_bitflags::{owner_mod}::{slug}::{}", bdecl.name);
        all_bitflags.insert(key.clone(), resolve_bitflags_decl(key.clone(), bdecl));
        local_bitflags.insert(bname.clone(), key);
    }

    // 3c. Resolve imported object schemas across all loaded modules.
    let mut object_schemas: BTreeMap<String, ResolvedObjectSchema> = BTreeMap::new();
    for scope in resolved.module_scopes.values() {
        for (def_mod, odecl) in scope.objects.values() {
            let type_name = object_type_ident(&odecl.name);
            let key = format!("object::{def_mod}::{type_name}");
            if let std::collections::btree_map::Entry::Vacant(slot) = object_schemas.entry(key) {
                let schema = build_object_schema(
                    resolved,
                    def_mod,
                    odecl,
                    slot.key(),
                    &type_name,
                    &mut all_enums,
                    &mut all_bitflags,
                )?;
                slot.insert(schema);
            }
        }
    }

    let self_type_name = object_type_ident(&obj.name);
    let self_object_key = format!("object::{}::{self_type_name}", resolved.module_id);

    // Helper closure to resolve a StateType / V2Type inside this object.
    let resolve_type_here = |ty: &V2Type, owner_mod: &str, span: Span| -> Result<ValueType> {
        match ty {
            V2Type::I16 => Ok(ValueType::I16),
            V2Type::Bool => Ok(ValueType::Bool),
            V2Type::ObjectId => Ok(ValueType::ObjectId),
            V2Type::Named(tname) => {
                if let Some(k) = local_enums.get(tname) {
                    return Ok(ValueType::Enum(k.clone()));
                }
                if let Some(k) = local_bitflags.get(tname) {
                    return Ok(ValueType::Bitflags(k.clone()));
                }
                match tname.as_str() {
                    "Height" => return Ok(ValueType::Bitflags("builtin::Height".to_owned())),
                    "Placement" => return Ok(ValueType::Bitflags("builtin::Placement".to_owned())),
                    "Self" => return Ok(ValueType::ObjectRef(self_object_key.clone())),
                    _ => {}
                }
                if let Some(sc) = resolved.module_scopes.get(owner_mod) {
                    if let Some((def_mod, ed)) = sc.enums.get(tname) {
                        return Ok(ValueType::Enum(format!("enum::{def_mod}::{}", ed.name)));
                    }
                    if let Some((def_mod, bd)) = sc.bitflags.get(tname) {
                        return Ok(ValueType::Bitflags(format!(
                            "bitflags::{def_mod}::{}",
                            bd.name
                        )));
                    }
                    if let Some((def_mod, od)) = sc.objects.get(tname) {
                        return Ok(ValueType::ObjectRef(format!(
                            "object::{def_mod}::{}",
                            object_type_ident(&od.name)
                        )));
                    }
                }
                Err(Error::at(span, format!("unknown type '{tname}'")))
            }
            other => Err(Error::at(
                span,
                format!("unsupported type '{}'", other.display_name()),
            )),
        }
    };

    // 4. Allocate state attribute indices (explicit `@ slot` states first, then the lowest free
    // slot in declaration order; without explicit slots this is exactly `0..N`).
    let state_slots = allocate_state_slots(
        state_order
            .iter()
            .map(|n| (n.as_str(), raw_states[n].slot, raw_states[n].span)),
    )?;
    let mut states = BTreeMap::new();
    let mut self_schema_states = BTreeMap::new();
    for (sname, &idx) in state_order.iter().zip(&state_slots) {
        let s = &raw_states[sname];
        let owner_mod = state_owners
            .get(sname)
            .map_or(resolved.module_id.as_str(), String::as_str);
        let resolved_ty = match &s.ty {
            StateType::Bool => ValueType::Bool,
            StateType::I16 => ValueType::I16,
            StateType::ObjectId => ValueType::ObjectId,
            StateType::Named(tname) => {
                resolve_type_here(&V2Type::Named(tname.clone()), owner_mod, s.span)?
            }
        };
        self_schema_states.insert(sname.clone(), (idx, resolved_ty.clone()));
        states.insert(
            sname.clone(),
            ResolvedState {
                attr_index: idx,
                ty: resolved_ty,
                init: s.init.clone(),
                span: s.span,
                skip_zero_init: s.slot.is_some() && s.init.is_none(),
            },
        );
    }

    // Register self schema (with its resolved states, functionals, and tile GUIDs) so `self.parts()`
    // and self-typed references resolve named states with full type safety!
    let mut self_schema_funcs = BTreeMap::new();
    for f in &functionals {
        let mut p_tys = Vec::with_capacity(f.decl.params.len());
        for fp in &f.decl.params {
            p_tys.push(resolve_type_here(&fp.ty, &f.owner_module, fp.span)?);
        }
        let r_ty = match &f.decl.return_type {
            Some(rt) => Some(resolve_type_here(rt, &f.owner_module, f.decl.span)?),
            None => None,
        };
        self_schema_funcs.insert(
            f.decl.kind.clone(),
            ResolvedFunctionalSig {
                params: p_tys,
                return_type: r_ty,
                guard: summarize_functional_guard(f.decl.guard.as_ref(), &self_schema_states),
            },
        );
    }
    let self_lead_guid = select_lead_tile_guid(&tiles, guid);
    let self_tile_guids = if tiles.is_empty() {
        vec![guid]
    } else {
        tiles.iter().map(|t| t.guid).collect()
    };
    let self_schema = ResolvedObjectSchema {
        canonical_key: self_object_key.clone(),
        display_name: self_type_name,
        master_guid: guid,
        lead_guid: self_lead_guid,
        tile_guids: self_tile_guids,
        states: self_schema_states,
        functionals: self_schema_funcs,
    };
    object_schemas.insert(self_object_key.clone(), self_schema);

    // 5. Assign FWAV sound IDs (`1..`, honoring explicit IDs).
    let mut used_sound_ids: BTreeSet<u16> = BTreeSet::new();
    for sname in &sound_order {
        if let Some(eid) = raw_sounds[sname].explicit_id
            && !used_sound_ids.insert(eid)
        {
            return Err(Error::at(
                raw_sounds[sname].span,
                format!("duplicate sound id {eid}"),
            ));
        }
    }
    let mut next_sound_id = 1u16;
    let mut sounds = BTreeMap::new();
    let mut sound_specs = Vec::with_capacity(sound_order.len());
    // Sound names already used (lowercased, as the compiler compares them) -> (preset, name).
    let mut sound_names: BTreeMap<String, (crate::audio::SoundPreset, String)> = BTreeMap::new();
    let mut shared_sound_names = Vec::new();
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
        // Several FWAV ids may name one sound: the later ones get a unique placeholder in the
        // manifest, and the compiled chunk is renamed back.
        let manifest_name = match sound_names.get(&wav_name.to_ascii_lowercase()) {
            Some((preset, first)) => {
                if s.explicit_id.is_none() {
                    return Err(Error::at(
                        s.span,
                        format!(
                            "sound '{sname}' shares the name '{first}' with an earlier sound: give it an explicit 'id:'"
                        ),
                    ));
                }
                if *preset != s.preset {
                    return Err(Error::at(
                        s.span,
                        format!(
                            "sound '{sname}' shares the name '{first}' with an earlier sound but has a different preset"
                        ),
                    ));
                }
                let placeholder = format!("v2-shared-sound-{id}");
                shared_sound_names.push((id, placeholder.clone(), wav_name.clone()));
                placeholder
            }
            None => {
                sound_names.insert(wav_name.to_ascii_lowercase(), (s.preset, wav_name.clone()));
                wav_name
            }
        };
        sound_specs.push(SoundSpec {
            id,
            name: manifest_name,
            preset: s.preset,
        });
    }
    for (_, placeholder, _) in &shared_sound_names {
        if sound_names.contains_key(placeholder) {
            return Err(Error::new(format!(
                "sound name '{placeholder}' is reserved for shared sound names"
            )));
        }
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

    let bcon_tables = resolve_bcon_tables(&bcon_decls, &i16_param_order, &local_extern_bcons)?;

    let dialog_strings = RefCell::new(DialogStringTable::from_existing(strings.get(&301)));
    let catalog: Vec<&str> = [obj.name.as_str(), description.as_str()]
        .into_iter()
        .chain(variant_decls.iter().flat_map(|v| {
            [v.name.as_str()]
                .into_iter()
                .chain(v.description.as_deref())
        }))
        .collect();
    let text_reads = text_reads
        .unwrap_or_else(|| object_text_reads(&strings, &dialog_strings.borrow(), &catalog));
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
        dialog_strings,
        text_reads,
        in_place_off,
        in_place_unsound: std::cell::Cell::new(false),
        local_fns,
        local_enums,
        local_bitflags,
        all_enums,
        all_bitflags,
        object_schemas,
        self_object_key,
        local_extern_mods,
        local_extern_bcons,
        local_consts,
        bcon_tables,
        trees: BTreeMap::new(),
    };
    let env = {
        let mut env = env;
        let tree_sigs = resolve_tree_sigs(&env, &tree_members, &member_modules)?;
        env.trees = tree_sigs;
        env
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
    if let Some(id) = load_pinned_id {
        register_pinned(id, obj.span)?;
    }
    if let Some(id) = reset_pinned_id {
        register_pinned(id, obj.span)?;
    }
    if let Some(id) = cleanup_pinned_id {
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
    for t in &tree_members {
        if let Some(id) = t.decl.pinned_id {
            register_pinned(id, t.decl.span)?;
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
    // Standalone `tree` IDs are allocated up front so bodies (and siblings) can `call` them.
    let mut tree_ids: BTreeMap<String, u16> = BTreeMap::new();
    for t in &tree_members {
        let id = alloc_tree_id(t.decl.pinned_id, None, t.decl.span)?;
        tree_ids.insert(t.decl.ident.clone(), id);
    }
    let env = {
        let mut env = env;
        for (ident, sig) in &mut env.trees {
            sig.id = tree_ids[ident];
        }
        env
    };

    // 8. Emit all trees into `trees_air` first so inline dialog strings interned into
    // `env.dialog_strings` are complete before we emit the STR# 301 table at the top!
    let mut trees_air = String::new();

    // 8c. Init tree.
    {
        let mut ctx = TreeLowerer::new(&env, TreeContext::ObjectSelf, &resolved.module_id, false);
        let frame = lifecycle_frames.get("Init").copied().unwrap_or_default();
        ctx.set_lifecycle_frame(&frame);
        let mut stack_ops = ExplicitStackOps::default();
        ctx.explicit_stack_obj = init_blocks
            .iter()
            .any(|b| stack_ops.in_stmts(&b.stmts, &env, &b.owner_module, true));
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
            if st.skip_zero_init {
                // `match` lowers the last enum arm as a plain `else`, so an uninitialized explicit
                // slot (0, or whatever a previous owner left) would silently dispatch to it.
                if let ValueType::Enum(ref key) = st.ty
                    && let Some(e) = env.all_enums.get(key)
                {
                    return Err(Error::at(
                        st.span,
                        format!(
                            "state '{sname}' @ {} of enum type '{}' requires an explicit initializer (init leaves an explicit slot without one untouched, so it may hold an undeclared value)",
                            st.attr_index, e.display_name
                        ),
                    ));
                }
                continue;
            }
            let owner_mod = state_owners
                .get(sname)
                .map_or(resolved.module_id.as_str(), String::as_str);
            let rhs = match &st.init {
                None => {
                    if let ValueType::Enum(ref key) = st.ty
                        && let Some(e) = env.all_enums.get(key)
                        && !e.variants.values().any(|&v| v == 0)
                    {
                        return Err(Error::at(
                            st.span,
                            format!(
                                "state '{sname}' of enum type '{}' requires an explicit initializer (0 is not a declared variant)",
                                e.display_name
                            ),
                        ));
                    }
                    "0".to_owned()
                }
                Some(init_val) => {
                    resolve_state_init_air(&env, owner_mod, sname, &st.ty, init_val, st.span)?
                }
            };
            ctx.line(format!("my.attr[{}] := {rhs};", st.attr_index));
        }
        // Append template + object `init { ... }` hooks, each in its own lexical/cleanup scope.
        for blk in &init_blocks {
            ctx.with_owner_module(&blk.owner_module, |c| {
                c.push_scope(ScopeFrame::block());
                let r = c.lower_stmts(&blk.stmts);
                let p = if stmts_always_return(&blk.stmts, &env, &blk.owner_module, true) {
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
            .is_some_and(|b| stmts_always_return(&b.stmts, &env, &b.owner_module, true))
        {
            ctx.emit_all_cleanups()?;
            ctx.line("return true;".to_owned());
        }
        write_framed_tree(&mut trees_air, ctx, init_id, "Init", &frame, 0, "Init")?;
    }

    // 8d. Main tree.
    {
        let mut ctx = TreeLowerer::new(&env, TreeContext::ObjectSelf, &resolved.module_id, false);
        let frame = lifecycle_frames.get("Main").copied().unwrap_or_default();
        ctx.set_lifecycle_frame(&frame);
        ctx.explicit_stack_obj = main_block.as_ref().is_some_and(|b| {
            ExplicitStackOps::default().in_stmts(&b.stmts, &env, &b.owner_module, true)
        });
        if let Some(blk) = &main_block {
            ctx.with_owner_module(&blk.owner_module, |c| {
                c.push_scope(ScopeFrame::block());
                let r = c.lower_stmts(&blk.stmts);
                let p = if stmts_always_return(&blk.stmts, &env, &blk.owner_module, true) {
                    c.pop_scope_discard_cleanups();
                    Ok(())
                } else {
                    c.pop_scope_and_run_cleanups()
                };
                r.and(p)
            })?;
            if !stmts_always_return(&blk.stmts, &env, &blk.owner_module, true) {
                ctx.emit_all_cleanups()?;
                ctx.line("return true;".to_owned());
            }
        } else {
            ctx.line("return true;".to_owned());
        }
        write_framed_tree(&mut trees_air, ctx, main_id, "Main", &frame, 0, "Main")?;
    }

    // 8d2. Optional OBJf lifecycle hooks: `load` (OBJf 2), `reset` (OBJf 3), `cleanup` (OBJf 4).
    let mut callback_specs = Vec::with_capacity(functionals.len() + 3);
    for (cb_index, tree_label, pinned_id, blocks) in [
        (2usize, "Load", load_pinned_id, &load_blocks),
        (3usize, "Reset", reset_pinned_id, &reset_blocks),
        (4usize, "Cleanup", cleanup_pinned_id, &cleanup_blocks),
    ] {
        if blocks.is_empty() {
            continue;
        }
        let tid = alloc_tree_id(pinned_id, None, obj.span)?;
        let mut ctx = TreeLowerer::new(&env, TreeContext::ObjectSelf, &resolved.module_id, false);
        let frame = lifecycle_frames
            .get(tree_label)
            .copied()
            .unwrap_or_default();
        ctx.set_lifecycle_frame(&frame);
        let mut stack_ops = ExplicitStackOps::default();
        ctx.explicit_stack_obj = blocks
            .iter()
            .any(|b| stack_ops.in_stmts(&b.stmts, &env, &b.owner_module, true));
        for blk in blocks {
            ctx.with_owner_module(&blk.owner_module, |c| {
                c.push_scope(ScopeFrame::block());
                let r = c.lower_stmts(&blk.stmts);
                let p = if stmts_always_return(&blk.stmts, &env, &blk.owner_module, true) {
                    c.pop_scope_discard_cleanups();
                    Ok(())
                } else {
                    c.pop_scope_and_run_cleanups()
                };
                r.and(p)
            })?;
        }
        if !blocks
            .last()
            .is_some_and(|b| stmts_always_return(&b.stmts, &env, &b.owner_module, true))
        {
            ctx.emit_all_cleanups()?;
            ctx.line("return true;".to_owned());
        }
        write_framed_tree(&mut trees_air, ctx, tid, tree_label, &frame, 0, tree_label)?;
        callback_specs.push(CallbackSpec {
            index: cb_index,
            action: Some(TreeRef::Id(tid)),
            condition: None,
        });
    }

    // 8e. Functional callback trees (OBJf).
    for func in &functionals {
        let (cb_index, _) = functional_kind_info(&func.decl.kind, func.decl.kind_span)?;
        let guard_id = if let Some(guard) = &func.decl.guard {
            let gid = alloc_tree_id(func.decl.pinned_guard_id, None, guard.span)?;
            let label = format!("Can_{}", capitalize(&func.decl.kind));
            emit_guard_tree(
                &mut trees_air,
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
        let func_ret_ty = match &func.decl.return_type {
            Some(rt) => {
                Some(env.resolve_value_type(rt, &func.owner_module, true, func.decl.span)?)
            }
            None => None,
        };
        if func_ret_ty.is_some()
            && !stmts_always_return(&func.decl.body, &env, &func.owner_module, true)
        {
            return Err(Error::at(
                func.decl.span,
                format!(
                    "functional '{}' declares a return type and must return a value on all control-flow paths",
                    func.decl.kind
                ),
            ));
        }
        let needs_self_save = stmts_may_clobber_stack_obj(
            &func.decl.body,
            &env,
            &func.owner_module,
            true,
            Some(&func.decl.sim_binding),
            0,
        );
        let mut ctx = TreeLowerer::new(
            &env,
            TreeContext::SimOnObject {
                sim_name: func.decl.sim_binding.clone(),
            },
            &func.owner_module,
            needs_self_save,
        );
        ctx.begin_body(ExplicitStackOps::default().in_stmts(
            &func.decl.body,
            &env,
            &func.owner_module,
            true,
        ));
        ctx.value_return = func_ret_ty.map(|ty| ValueReturnAbi {
            ty,
            temp: 0,
            allow_none: true,
            in_place: false,
            what: "functional",
        });
        // The engine runs a functional with a full four-word argument frame, whatever it declares.
        ctx.frame_args = func.decl.params.len().max(4);
        for (pidx, fp) in func.decl.params.iter().enumerate() {
            ctx.bind_func_param(&fp.name, pidx, &fp.ty, fp.span)?;
        }
        ctx.lower_stmts(&func.decl.body)?;
        if !stmts_always_return(&func.decl.body, &env, &func.owner_module, true) {
            ctx.emit_all_cleanups()?;
            ctx.line("return true;".to_owned());
        }
        // The header declares the declared parameters, or every `param[n]` word the body
        // names, whichever is more (the engine's frame has four).
        let header_args = func.decl.params.len().max(ctx.param_words_used);
        let mut header = Vec::new();
        if header_args > 0 {
            header.push(format!("args: {header_args}"));
        }
        ctx.push_frame_floor(&mut header);
        let _ = writeln!(
            trees_air,
            "tree {action_id} \"{action_label}\"{} {{",
            air_header(&header)
        );
        trees_air.push_str(&ctx.finish()?);
        trees_air.push_str("}\n\n");

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

        // Validate optional declarative `lock: self.<bool_state>` and desugar into guard + `with lock(...)`.
        let (effective_guard, effective_run_body) = if let Some((lspan, ref field_name)) =
            inter.decl.lock
        {
            let st = env.states.get(field_name).ok_or_else(|| {
                Error::at(
                    lspan,
                    format!(
                        "interaction '{}' lock 'self.{field_name}' references unknown state '{field_name}'",
                        inter.decl.label
                    ),
                )
            })?;
            if st.ty != ValueType::Bool {
                let actual_ty = env.display_value_type(&st.ty);
                return Err(Error::at(
                    lspan,
                    format!(
                        "interaction '{}' lock 'self.{field_name}' requires a bool state, found '{actual_ty}'",
                        inter.decl.label
                    ),
                ));
            }
            let lock_expr = V2Expr::Field {
                span: lspan,
                base: Box::new(V2Expr::Ident(lspan, "self".to_owned())),
                field: field_name.clone(),
            };
            let not_locked = V2Expr::Unary {
                span: lspan,
                op: V2UnaryOp::Not,
                inner: Box::new(lock_expr.clone()),
            };
            let composed_guard = match &inter.decl.guard {
                None => GuardDecl {
                    span: lspan,
                    sim_binding: inter.decl.sim_binding.clone(),
                    autonomous_binding: None,
                    kind: GuardBody::Expr(not_locked),
                },
                Some(existing) => match &existing.kind {
                    GuardBody::Expr(expr) => {
                        let combined = if expr_starts_with_not_self_field(expr, field_name) {
                            expr.clone()
                        } else {
                            V2Expr::Binary {
                                span: lspan,
                                lhs: Box::new(not_locked),
                                op: V2BinOp::And,
                                rhs: Box::new(expr.clone()),
                            }
                        };
                        GuardDecl {
                            span: existing.span,
                            sim_binding: existing.sim_binding.clone(),
                            autonomous_binding: existing.autonomous_binding.clone(),
                            kind: GuardBody::Expr(combined),
                        }
                    }
                    GuardBody::Block(stmts) => {
                        let mut body = Vec::with_capacity(stmts.len() + 1);
                        body.push(V2Stmt::If {
                            span: lspan,
                            cond: lock_expr.clone(),
                            then_body: vec![V2Stmt::Return {
                                span: lspan,
                                value: Some(V2Expr::Bool(lspan, false)),
                            }],
                            else_body: Vec::new(),
                        });
                        body.extend(stmts.iter().cloned());
                        GuardDecl {
                            span: existing.span,
                            sim_binding: existing.sim_binding.clone(),
                            autonomous_binding: existing.autonomous_binding.clone(),
                            kind: GuardBody::Block(body),
                        }
                    }
                },
            };
            let wrapped_body = vec![V2Stmt::WithLock {
                span: lspan,
                lock_expr,
                body: inter.decl.run_body.clone(),
            }];
            (Some(composed_guard), wrapped_body)
        } else {
            (inter.decl.guard.clone(), inter.decl.run_body.clone())
        };

        let guard_id = if let Some(guard) = &effective_guard {
            let gid = alloc_tree_id(inter.decl.pinned_guard_id, None, guard.span)?;
            let glabel = format!("Can_{base_ident}");
            emit_guard_tree(
                &mut trees_air,
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
        let needs_self_save = stmts_may_clobber_stack_obj(
            &effective_run_body,
            &env,
            &inter.owner_module,
            true,
            Some(&inter.decl.sim_binding),
            0,
        );
        let mut ctx = TreeLowerer::new(
            &env,
            TreeContext::SimOnObject {
                sim_name: inter.decl.sim_binding.clone(),
            },
            &inter.owner_module,
            needs_self_save,
        );
        ctx.begin_body(ExplicitStackOps::default().in_stmts(
            &effective_run_body,
            &env,
            &inter.owner_module,
            true,
        ));
        ctx.lower_stmts(&effective_run_body)?;
        if !stmts_always_return(&effective_run_body, &env, &inter.owner_module, true) {
            ctx.emit_all_cleanups()?;
            ctx.line("return true;".to_owned());
        }
        let mut header = Vec::new();
        ctx.push_frame_floor(&mut header);
        let _ = writeln!(
            trees_air,
            "interaction {action_id} \"{base_ident}\"{} {{",
            air_header(&header)
        );
        trees_air.push_str(&ctx.finish()?);
        trees_air.push_str("}\n\n");

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

    // 8h. Standalone `tree` declarations (non-inlined BHAVs callable via `.air` `call`).
    for t in &tree_members {
        emit_standalone_tree(&mut trees_air, &env, t)?;
    }

    // 8g. Assemble final `.air` output: header + BCON 4096 + STR# tables (including auto-interned 301) + trees.
    let mut air = String::new();
    let _ = writeln!(
        air,
        "// Generated .air (Antics IR) from .antics v2 object \"{}\" (GUID {guid:#010x})",
        escape_str(&env.name)
    );
    if !env.state_order.is_empty() {
        air.push_str("// Attributes: ");
        let mut by_index: Vec<(usize, &String)> = env
            .state_order
            .iter()
            .map(|n| (env.states[n].attr_index, n))
            .collect();
        by_index.sort();
        for (i, (idx, sname)) in by_index.into_iter().enumerate() {
            if i > 0 {
                air.push_str(", ");
            }
            let _ = write!(air, "attr[{idx}] = {sname}");
        }
        air.push('\n');
    }
    air.push('\n');

    if !env.i16_param_order.is_empty() {
        let _ = writeln!(air, "bcon 4096 \"{}Tuning\" {{", camel_label(&env.name));
        for pname in &env.i16_param_order {
            let val = env.i16_params[pname];
            let _ = writeln!(air, "    {pname} = {val},");
        }
        air.push_str("}\n\n");
    }
    // Named private BCON tables: emitted in list form (entries are referenced numerically as
    // `bcon[(table << 7) | entry]`, so they never enter `.air`'s global BCON name namespace).
    let mut emitted_tables = BTreeSet::new();
    for t in env.bcon_tables.values() {
        if !emitted_tables.insert(t.id) {
            continue;
        }
        let vals: Vec<String> = t.values.iter().map(|(_, v)| v.to_string()).collect();
        let _ = writeln!(
            air,
            "bcon {} \"{}\" = [{}];\n",
            t.id,
            escape_str(&t.label),
            vals.join(", ")
        );
    }

    // Every tree was finished with `env.text_reads`. A tree's inline dialog literal can add a
    // token that an earlier tree's text may then read, so lower the object again with the
    // final fact when it differs (the second lowering interns the same strings). Likewise
    // when a tree's in-place rewrite turned out unsound: lower again without it.
    let final_reads = object_text_reads(&env.strings, &env.dialog_strings.borrow(), &catalog);
    let in_place_off = env.in_place_off || env.in_place_unsound.get();
    if final_reads != env.text_reads || in_place_off != env.in_place_off {
        return lower_object_with(resolved, obj, Some(final_reads), in_place_off);
    }

    // Emit STR# tables in deterministic ID order, merging any auto-interned STR# 301 dialog strings.
    let dlg_table = env.dialog_strings.borrow();
    let mut emitted_301 = false;
    for (&str_id, st) in &env.strings {
        if str_id == 301 {
            emitted_301 = true;
            if !dlg_table.entries.is_empty() {
                let lbl = dlg_table
                    .label
                    .clone()
                    .unwrap_or_else(|| format!("{}Strings_301", camel_label(&env.name)));
                let _ = writeln!(air, "str 301 \"{}\" {{", escape_str(&lbl));
                for (_, ename, text) in &dlg_table.entries {
                    let _ = writeln!(air, "    {ename} = \"{}\",", escape_str(text));
                }
                air.push_str("}\n\n");
            }
            continue;
        }
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
    if !emitted_301 && !dlg_table.entries.is_empty() {
        let lbl = dlg_table
            .label
            .clone()
            .unwrap_or_else(|| format!("{}Dialogs", camel_label(&env.name)));
        let _ = writeln!(air, "str 301 \"{}\" {{", escape_str(&lbl));
        for (_, ename, text) in &dlg_table.entries {
            let _ = writeln!(air, "    {ename} = \"{}\",", escape_str(text));
        }
        air.push_str("}\n\n");
    }
    drop(dlg_table);
    air.push_str(&trees_air);

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

    let state_count = env
        .states
        .values()
        .map(|st| st.attr_index + 1)
        .max()
        .unwrap_or(0);
    let state_count = u16::try_from(state_count)
        .map_err(|_| Error::at(obj.span, "object declares too many state attributes"))?;
    let num_attributes = match explicit_attributes {
        Some(explicit) => state_count.max(explicit),
        None => state_count.max(8),
    };

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

    let variants = resolve_variants(
        &variant_decls,
        &manifest,
        &env.trees,
        !manifest.tiles.is_empty(),
    )?;

    Ok(ExpandedV2Object {
        name: env.name,
        slug,
        guid,
        manifest,
        air_source: air,
        variants,
        shared_sound_names,
    })
}

/// Resolves `variant` declarations against the primary manifest and the standalone tree ids.
pub(super) fn resolve_variants(
    decls: &[VariantDecl],
    manifest: &ObjectManifest,
    trees: &BTreeMap<String, CallableSig>,
    multi_tile: bool,
) -> Result<Vec<VariantSpec>> {
    let mut out: Vec<VariantSpec> = Vec::with_capacity(decls.len());
    for d in decls {
        if d.name == manifest.name {
            return Err(Error::at(
                d.span,
                format!(
                    "variant '{}' has the primary object's name; each catalog entry needs its own",
                    d.name
                ),
            ));
        }
        if multi_tile {
            return Err(Error::at(
                d.span,
                "'variant' is not supported on multi-tile objects (their tiles own the OBJD group)",
            ));
        }
        let (gspan, guid) = d
            .guid
            .ok_or_else(|| Error::at(d.span, format!("variant '{}' needs a 'guid:'", d.name)))?;
        if guid == manifest.guid || out.iter().any(|v| v.guid == guid) {
            return Err(Error::at(
                gspan,
                format!("variant '{}' reuses GUID {guid:#010x}", d.name),
            ));
        }
        let price = match d.price {
            Some((pspan, p)) => u16::try_from(p).map_err(|_| {
                Error::at(pspan, format!("variant price {p} out of range (0..=65535)"))
            })?,
            None => manifest.price,
        };
        let tree = |r: &Option<(Span, String)>, default: Option<&TreeRef>| -> Result<u16> {
            match r {
                Some((tspan, ident)) => {
                    let sig = trees.get(ident).ok_or_else(|| {
                        Error::at(
                            *tspan,
                            format!(
                                "variant tree '{ident}' is not a standalone 'tree' of this object"
                            ),
                        )
                    })?;
                    // The VM runs a catalog entry's init/main as the object itself, with no
                    // arguments, like the primary's.
                    if sig.context != CallContext::ObjectSelf
                        || !sig.params.is_empty()
                        || sig.frame_args.unwrap_or(0) > 4
                    {
                        return Err(Error::at(
                            *tspan,
                            format!(
                                "variant tree '{ident}' must be an object-self tree with no parameters (it runs as the object's init/main)"
                            ),
                        ));
                    }
                    Ok(sig.id)
                }
                None => match default {
                    Some(TreeRef::Id(id)) => Ok(*id),
                    _ => Err(Error::at(
                        d.span,
                        "variant needs the primary's init/main tree ids",
                    )),
                },
            }
        };
        out.push(VariantSpec {
            name: d.name.clone(),
            guid,
            price,
            description: d
                .description
                .clone()
                .unwrap_or_else(|| manifest.description.clone()),
            init_id: tree(&d.init, manifest.init.as_ref())?,
            main_id: tree(&d.main, manifest.main.as_ref())?,
        });
    }
    Ok(out)
}
