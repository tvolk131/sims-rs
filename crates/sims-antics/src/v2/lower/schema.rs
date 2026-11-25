//! Object schema and per-object resolution: state slots, BCON tables, tree signatures, sprites,
//! functional guards and state initialisers.

use super::*;

/// Allocates state attribute slots shared by object lowering and imported object schemas.
///
/// Explicit `state x @ N: T;` slots are reserved first (overlaps are errors), then every
/// auto-allocated state takes the lowest free slot in declaration order. Without explicit slots
/// this is exactly the historical `0..N` layout.
pub(super) fn allocate_state_slots<'s>(
    states: impl IntoIterator<Item = (&'s str, Option<(Span, u16)>, Span)>,
) -> Result<Vec<usize>> {
    let states: Vec<_> = states.into_iter().collect();
    let mut used: BTreeMap<u16, &str> = BTreeMap::new();
    for &(name, slot, _) in &states {
        if let Some((sspan, n)) = slot
            && let Some(other) = used.insert(n, name)
        {
            return Err(Error::at(
                sspan,
                format!(
                    "state '{name}' @ {n} overlaps state '{other}' (both bind attribute slot {n})"
                ),
            ));
        }
    }
    let mut next = 0u16;
    let mut out = Vec::with_capacity(states.len());
    for &(name, slot, span) in &states {
        let idx = match slot {
            Some((_, n)) => n,
            None => {
                while used.contains_key(&next) {
                    next += 1;
                }
                if next > crate::v2::parser::MAX_STATE_SLOT {
                    return Err(Error::at(
                        span,
                        format!(
                            "state '{name}' does not fit in attribute slots 0..={}",
                            crate::v2::parser::MAX_STATE_SLOT
                        ),
                    ));
                }
                used.insert(next, name);
                next
            }
        };
        out.push(usize::from(idx));
    }
    Ok(out)
}

/// Resolves private named BCON tables into operand addresses, keyed by both reference spellings
/// (`LampTuning` and `lamp_tuning`).
pub(super) fn resolve_bcon_tables(
    decls: &[BconTableDecl],
    i16_param_order: &[String],
    local_extern_bcons: &BTreeMap<String, (String, ExternBconDecl)>,
) -> Result<BTreeMap<String, ResolvedBconTable>> {
    let mut out: BTreeMap<String, ResolvedBconTable> = BTreeMap::new();
    for d in decls {
        if !(4096..=4159).contains(&d.id) {
            return Err(Error::at(
                d.span,
                format!(
                    "BCON table id {} must be a private BCON chunk id in 4096..=4159 (use 'extern bcon' for global/semi-global constants)",
                    d.id
                ),
            ));
        }
        if d.id == 4096 && !i16_param_order.is_empty() {
            return Err(Error::at(
                d.span,
                "BCON 4096 is reserved for this object's i16 params (pick 4097..=4159)",
            ));
        }
        if d.entries.len() > 128 {
            return Err(Error::at(
                d.span,
                format!(
                    "BCON table '{}' has {} entries (max 128)",
                    d.label,
                    d.entries.len()
                ),
            ));
        }
        let table = d.id - 4096;
        let mut entries = BTreeMap::new();
        let mut values = Vec::with_capacity(d.entries.len());
        for (i, (espan, ename, v)) in d.entries.iter().enumerate() {
            let entry = u16::try_from(i).expect("<= 128 entries");
            if entries
                .insert(ename.clone(), ((table << 7) | entry, *v))
                .is_some()
            {
                return Err(Error::at(
                    *espan,
                    format!("duplicate entry '{ename}' in BCON table '{}'", d.label),
                ));
            }
            values.push((ename.clone(), *v));
        }
        let resolved = ResolvedBconTable {
            id: d.id,
            label: d.label.clone(),
            entries,
            values,
        };
        let mut names = vec![object_type_ident(&d.label), snake_case_ident(&d.label)];
        names.dedup();
        for name in names {
            if local_extern_bcons.contains_key(&name) {
                return Err(Error::at(
                    d.span,
                    format!(
                        "BCON table '{}' reference name '{name}' collides with an 'extern bcon' block",
                        d.label
                    ),
                ));
            }
            if let Some(prev) = out.insert(name.clone(), resolved.clone()) {
                return Err(Error::at(
                    d.span,
                    format!(
                        "BCON table '{}' reference name '{name}' collides with BCON table '{}' (BCON {})",
                        d.label, prev.label, prev.id
                    ),
                ));
            }
        }
    }
    Ok(out)
}

/// Resolves standalone `tree` call signatures (IDs are filled in after allocation).
pub(super) fn resolve_tree_sigs(
    env: &ObjectEnv<'_>,
    trees: &[OwnedTree],
    member_modules: &BTreeSet<String>,
) -> Result<BTreeMap<String, CallableSig>> {
    let mut out = BTreeMap::new();
    for t in trees {
        let d = &t.decl;
        if BUILTIN_CALL_NAMES.contains(&d.ident.as_str()) {
            return Err(Error::at(
                d.span,
                format!("tree '{}' shadows the built-in '{}()'", d.ident, d.ident),
            ));
        }
        if let Some(m) = std::iter::once(&t.owner_module)
            .chain(member_modules)
            .find(|m| env.lookup_fn(m, &d.ident, true).is_some())
        {
            return Err(Error::at(
                d.span,
                format!(
                    "tree '{}' collides with helper function '{}' in scope (module '{m}')",
                    d.ident, d.ident
                ),
            ));
        }
        let mut sig = env.resolve_callable_sig(
            format!("tree '{}'", d.ident),
            0,
            if d.inherit_context {
                CallContext::Inherit
            } else if d.sim_binding.is_some() {
                CallContext::SimOnObject
            } else {
                CallContext::ObjectSelf
            },
            &d.params,
            &d.ret,
            &t.owner_module,
            true,
            d.span,
        )?;
        // A tree without `->` is a plain SimAntics check/action tree: it exits true/false
        // (`return true;` by default), so callers may test it like a `-> bool` tree.
        if sig.ret == ResolvedReturnAbi::Void {
            sig.ret = ResolvedReturnAbi::Bool;
        }
        let positional = sig
            .params
            .iter()
            .filter(|p| matches!(p.slot, AbiParamSlot::Positional))
            .count();
        sig.frame_args = Some(d.frame.and_then(|f| f.args).map_or(positional, usize::from));
        out.insert(d.ident.clone(), sig);
    }
    Ok(out)
}

pub(super) fn sprite_decl_to_spec(sp: SpriteDecl) -> SpriteSpec {
    SpriteSpec {
        shape: sp.shape,
        primary_rgb: sp.primary_rgb,
        accent_rgb: sp.accent_rgb,
        trim_rgb: sp.trim_rgb,
    }
}

pub(super) fn build_object_schema(
    resolved: &ResolvedModule,
    owner_mod: &str,
    obj: &ObjectDecl,
    canonical_key: &str,
    display_name: &str,
    all_enums: &mut BTreeMap<String, ResolvedEnum>,
    all_bitflags: &mut BTreeMap<String, ResolvedBitflags>,
) -> Result<ResolvedObjectSchema> {
    let mut guid: Option<u32> = None;
    let mut tiles: Vec<crate::v2::ast::TileDecl> = Vec::new();
    let mut raw_states: Vec<(String, String, StateDecl)> = Vec::new();
    let mut seen_states = BTreeSet::new();
    let mut local_enums: BTreeMap<String, String> = BTreeMap::new();
    let mut local_bitflags: BTreeMap<String, String> = BTreeMap::new();
    let mut raw_funcs: Vec<(String, FunctionalDecl)> = Vec::new();
    let slug = make_slug(&obj.name);

    let mut collect = |m_owner: &str, members: &[ObjectMember]| {
        for member in members {
            match member {
                ObjectMember::Meta(MetaField {
                    kind: MetaFieldKind::Guid(g),
                    ..
                }) => guid = Some(*g),
                ObjectMember::Meta(MetaField {
                    kind: MetaFieldKind::Tiles(t),
                    ..
                }) => tiles = t.clone(),
                ObjectMember::Enum(e) => {
                    let key = format!("local_enum::{m_owner}::{slug}::{}", e.name);
                    all_enums
                        .entry(key.clone())
                        .or_insert_with(|| resolve_enum_decl(key.clone(), e));
                    local_enums.insert(e.name.clone(), key);
                }
                ObjectMember::Bitflags(b) => {
                    let key = format!("local_bitflags::{m_owner}::{slug}::{}", b.name);
                    all_bitflags
                        .entry(key.clone())
                        .or_insert_with(|| resolve_bitflags_decl(key.clone(), b));
                    local_bitflags.insert(b.name.clone(), key);
                }
                ObjectMember::State(s) => {
                    if seen_states.insert(s.name.clone()) {
                        raw_states.push((m_owner.to_owned(), s.name.clone(), s.clone()));
                    }
                }
                ObjectMember::Functional(f) => {
                    if let Some(pos) = raw_funcs.iter().position(|(_, x)| x.kind == f.kind) {
                        raw_funcs[pos] = (m_owner.to_owned(), f.clone());
                    } else {
                        raw_funcs.push((m_owner.to_owned(), f.clone()));
                    }
                }
                _ => {}
            }
        }
    };

    if let Some((tpl_name, _)) = &obj.from_template
        && let Some(scope) = resolved.module_scopes.get(owner_mod)
        && let Some((tpl_mod, tpl_decl)) = scope.templates.get(tpl_name)
    {
        collect(tpl_mod, &tpl_decl.members);
    }
    collect(owner_mod, &obj.members);

    let resolve_ty = |ty: &V2Type, m_owner: &str, span: Span| -> Result<ValueType> {
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
                    "Self" => return Ok(ValueType::ObjectRef(canonical_key.to_owned())),
                    _ => {}
                }
                if let Some(sc) = resolved.module_scopes.get(m_owner) {
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
                Err(Error::at(
                    span,
                    format!("unknown type '{tname}' in imported object '{display_name}'"),
                ))
            }
            other => Err(Error::at(
                span,
                format!(
                    "unsupported type '{}' in imported object '{display_name}'",
                    other.display_name()
                ),
            )),
        }
    };

    let state_slots = allocate_state_slots(
        raw_states
            .iter()
            .map(|(_, n, d)| (n.as_str(), d.slot, d.span)),
    )?;
    let mut states = BTreeMap::new();
    for ((m_owner, sname, sdecl), idx) in raw_states.into_iter().zip(state_slots) {
        let vty = match &sdecl.ty {
            StateType::Bool => ValueType::Bool,
            StateType::I16 => ValueType::I16,
            StateType::ObjectId => ValueType::ObjectId,
            StateType::Named(tname) => {
                resolve_ty(&V2Type::Named(tname.clone()), &m_owner, sdecl.span)?
            }
        };
        states.insert(sname, (idx, vty));
    }

    let mut functionals = BTreeMap::new();
    for (m_owner, fdecl) in raw_funcs {
        let mut p_tys = Vec::with_capacity(fdecl.params.len());
        for fp in &fdecl.params {
            p_tys.push(resolve_ty(&fp.ty, &m_owner, fp.span)?);
        }
        let r_ty = match &fdecl.return_type {
            Some(rt) => Some(resolve_ty(rt, &m_owner, fdecl.span)?),
            None => None,
        };
        functionals.insert(
            fdecl.kind,
            ResolvedFunctionalSig {
                params: p_tys,
                return_type: r_ty,
                guard: summarize_functional_guard(fdecl.guard.as_ref(), &states),
            },
        );
    }

    let guid = guid.ok_or_else(|| {
        Error::at(
            obj.span,
            format!("imported object '{display_name}' is missing required 'guid' field"),
        )
    })?;
    let lead_guid = select_lead_tile_guid(&tiles, guid);
    let tile_guids = if tiles.is_empty() {
        vec![guid]
    } else {
        tiles.iter().map(|t| t.guid).collect()
    };

    Ok(ResolvedObjectSchema {
        canonical_key: canonical_key.to_owned(),
        display_name: display_name.to_owned(),
        master_guid: guid,
        lead_guid,
        tile_guids,
        states,
        functionals,
    })
}

/// Mirrors `compile.rs`'s multi-tile lead selection rule: explicit `lead: true` first,
/// otherwise the `(sub_index == 0, level == 0)` origin tile, falling back to the first tile.
pub(super) fn select_lead_tile_guid(tiles: &[crate::v2::ast::TileDecl], master_guid: u32) -> u32 {
    if tiles.is_empty() {
        return master_guid;
    }
    if let Some(t) = tiles.iter().find(|t| t.lead) {
        return t.guid;
    }
    if let Some(t) = tiles.iter().find(|t| t.sub_index == 0 && t.level == 0) {
        return t.guid;
    }
    tiles[0].guid
}

pub(super) fn summarize_functional_guard(
    guard: Option<&GuardDecl>,
    states: &BTreeMap<String, (usize, ValueType)>,
) -> FunctionalGuardSummary {
    let Some(g) = guard else {
        return FunctionalGuardSummary::None;
    };
    let GuardBody::Expr(expr) = &g.kind else {
        return FunctionalGuardSummary::Complex;
    };
    let (inner, expect_true) = match expr {
        V2Expr::Unary {
            op: V2UnaryOp::Not,
            inner,
            ..
        } => (inner.as_ref(), false),
        other => (other, true),
    };
    if let V2Expr::Field { base, field, .. } = inner
        && matches!(base.as_ref(), V2Expr::Ident(_, b) if b == "self")
        && let Some((idx, ValueType::Bool)) = states.get(field)
    {
        return FunctionalGuardSummary::AttrBool {
            attr_index: *idx,
            expect_true,
        };
    }
    FunctionalGuardSummary::Complex
}

pub(super) fn resolve_state_init_air(
    env: &ObjectEnv<'_>,
    owner_mod: &str,
    sname: &str,
    ty: &ValueType,
    init: &StateInitValue,
    span: Span,
) -> Result<String> {
    match init {
        StateInitValue::Int(v) => {
            if !matches!(ty, ValueType::I16 | ValueType::ObjectId) {
                return Err(Error::at(
                    span,
                    format!(
                        "state '{sname}' of type '{}' cannot initialize from integer literal {v}",
                        env.display_value_type(ty)
                    ),
                ));
            }
            Ok(v.to_string())
        }
        StateInitValue::Bool(b) => {
            if *ty != ValueType::Bool {
                return Err(Error::at(
                    span,
                    format!(
                        "state '{sname}' of type '{}' cannot initialize from bool literal",
                        env.display_value_type(ty)
                    ),
                ));
            }
            Ok(if *b { "1" } else { "0" }.to_owned())
        }
        StateInitValue::Param(pname) => {
            if env.i16_params.contains_key(pname) {
                if !matches!(ty, ValueType::I16 | ValueType::ObjectId) {
                    return Err(Error::at(
                        span,
                        format!(
                            "state '{sname}' of type '{}' cannot initialize from i16 param '{pname}'",
                            env.display_value_type(ty)
                        ),
                    ));
                }
                Ok(format!("bcon.{pname}"))
            } else if let Some(&bval) = env.bool_params.get(pname) {
                if *ty != ValueType::Bool {
                    return Err(Error::at(
                        span,
                        format!(
                            "state '{sname}' of type '{}' cannot initialize from bool param '{pname}'",
                            env.display_value_type(ty)
                        ),
                    ));
                }
                Ok(if bval { "1" } else { "0" }.to_owned())
            } else {
                Err(Error::at(
                    span,
                    format!("state '{sname}' initializer references unknown param '{pname}'"),
                ))
            }
        }
        StateInitValue::Path(pspan, segs) => {
            if segs.len() != 2 {
                return Err(Error::at(
                    *pspan,
                    format!(
                        "expected 'Type::Variant' in state '{sname}' initializer, found '{}'",
                        segs.join("::")
                    ),
                ));
            }
            let (tname, vname) = (&segs[0], &segs[1]);
            if let Some(e) = env.lookup_enum(owner_mod, tname, true) {
                let Some(&val) = e.variants.get(vname) else {
                    return Err(Error::at(
                        *pspan,
                        format!("enum '{}' has no variant '{vname}'", e.display_name),
                    ));
                };
                if *ty != ValueType::Enum(e.canonical_key.clone()) {
                    return Err(Error::at(
                        *pspan,
                        format!(
                            "type mismatch: state '{sname}' has type '{}', cannot initialize from '{}'",
                            env.display_value_type(ty),
                            e.display_name
                        ),
                    ));
                }
                return Ok(val.to_string());
            }
            if let Some(bf) = env.lookup_bitflags(owner_mod, tname, true) {
                if *ty != ValueType::Bitflags(bf.canonical_key.clone()) {
                    return Err(Error::at(
                        *pspan,
                        format!(
                            "type mismatch: state '{sname}' has type '{}', cannot initialize from '{}'",
                            env.display_value_type(ty),
                            bf.display_name
                        ),
                    ));
                }
                let mask = if vname == "None" {
                    0i16
                } else if let Some(&bit) = bf.flags.get(vname) {
                    bit_to_mask(bit)
                } else {
                    return Err(Error::at(
                        *pspan,
                        format!("bitflags '{}' has no flag '{vname}'", bf.display_name),
                    ));
                };
                return Ok(mask.to_string());
            }
            Err(Error::at(
                *pspan,
                format!("unknown enum or bitflags type '{tname}' in state '{sname}' initializer"),
            ))
        }
        StateInitValue::BitOr(bspan, lhs, rhs) => {
            let ValueType::Bitflags(_) = ty else {
                return Err(Error::at(
                    *bspan,
                    format!(
                        "'|' in state '{sname}' initializer is only supported for bitflags types"
                    ),
                ));
            };
            let l_val: i16 = resolve_state_init_air(env, owner_mod, sname, ty, lhs, *bspan)?
                .parse()
                .map_err(|_| Error::at(*bspan, "bitflags initializer must be constant"))?;
            let r_val: i16 = resolve_state_init_air(env, owner_mod, sname, ty, rhs, *bspan)?
                .parse()
                .map_err(|_| Error::at(*bspan, "bitflags initializer must be constant"))?;
            Ok((l_val | r_val).to_string())
        }
    }
}

/// Maps a v2 functional kind name to `(OBJf callback index, optional score object data word)`.
pub(super) fn functional_kind_info(kind: &str, span: Span) -> Result<(usize, Option<usize>)> {
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
