//! `match` lowering support: arm bodies, pattern alternatives, single-range tests and
//! exhaustiveness.

use super::*;

impl MatchCase<'_> {
    pub(super) fn contains(&self, v: i16) -> bool {
        self.alts.iter().any(|&(lo, hi)| (lo..=hi).contains(&v))
    }
}

/// Whether an arm may `break` / `continue` the loop it sits in: through its statements, or
/// through a diverging block of a value arm (`else if c { break; }`, `0 => break`).
pub(super) fn arm_exits_loop(b: ArmBody<'_>) -> bool {
    match b {
        ArmBody::Stmts(s) => stmts_exit_loop(s),
        ArmBody::Value(e) => super::specialize::init_blocks(e)
            .into_iter()
            .any(stmts_exit_loop),
    }
}

/// A pattern's alternatives: itself, or an OR-pattern's members.
pub(super) fn pattern_alternatives(p: &MatchPattern) -> &[MatchPattern] {
    match p {
        MatchPattern::Or(_, alts) => alts,
        other => std::slice::from_ref(other),
    }
}

/// `scrut` in `lo..=hi` as one compare, when one suffices.
pub(super) fn single_range_test(scrut: &str, lo: i16, hi: i16) -> Option<String> {
    if lo == hi {
        Some(format!("{scrut} == {lo}"))
    } else if lo == i16::MIN {
        Some(format!("{scrut} <= {hi}"))
    } else if hi == i16::MAX {
        Some(format!("{scrut} >= {lo}"))
    } else {
        None
    }
}

pub(super) fn match_is_exhaustive(
    scrutinee: &V2Expr,
    arms: &[MatchArm],
    env: &ObjectEnv<'_>,
    owner_module: &str,
    allow_local: bool,
) -> bool {
    if arms
        .iter()
        .any(|a| matches!(a.pattern, MatchPattern::Wildcard(_)))
    {
        return true;
    }
    // `match dialog(...)` is always validated as exhaustive by `lower_match_dialog`.
    if matches!(
        scrutinee,
        V2Expr::Call { callee, .. } if matches!(callee.as_ref(), V2Expr::Ident(_, n) if n == "dialog")
    ) {
        return true;
    }
    // Type-aware enum exhaustiveness: if every arm matches `Enum::Variant` of the same enum
    // and every declared variant discriminant is covered, the match is exhaustive!
    let mut enum_key: Option<String> = None;
    let mut covered = BTreeSet::new();
    for pattern in arms.iter().flat_map(|a| pattern_alternatives(&a.pattern)) {
        let MatchPattern::Path(_, segs) = pattern else {
            return false;
        };
        if segs.len() != 2 {
            return false;
        }
        let Some(e) = env.lookup_enum(owner_module, &segs[0], allow_local) else {
            return false;
        };
        if let Some(ref k) = enum_key {
            if k != &e.canonical_key {
                return false;
            }
        } else {
            enum_key = Some(e.canonical_key.clone());
        }
        let Some(&val) = e.variants.get(&segs[1]) else {
            return false;
        };
        covered.insert(val);
    }
    if let Some(key) = enum_key
        && let Some(e) = env.all_enums.get(&key)
    {
        // An engine enum is open: only a `_` arm covers it.
        return !is_engine_enum_key(&key) && e.variants.values().all(|v| covered.contains(v));
    }
    false
}
