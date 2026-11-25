//! Parameter domains: values a tree's parameters are assumed to hold on entry
//! (`verify-equiv --param-domain`, `--v2-enum-domains`;
//! `docs/decisions/20261124-verifier-domains-and-speed.md`).

use crate::error::{Error, Result};
use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

/// A set of `i16` values: sorted, disjoint, non-adjacent inclusive ranges, never empty.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ParamDomain {
    ranges: Vec<(i16, i16)>,
}

/// `tree id -> parameter index -> domain`.
pub type ParamDomains = BTreeMap<u16, BTreeMap<u16, ParamDomain>>;

/// The most entry regions one tree is split into; past it each domain is widened to its
/// hull, which assumes less.
pub(crate) const MAX_DOMAIN_REGIONS: usize = 256;

impl ParamDomain {
    /// The union of `ranges` (each `lo <= hi`), normalized.
    pub fn from_ranges(ranges: impl IntoIterator<Item = (i16, i16)>) -> Result<Self> {
        let mut ranges: Vec<(i16, i16)> = ranges.into_iter().collect();
        if let Some(&(lo, hi)) = ranges.iter().find(|(lo, hi)| lo > hi) {
            return Err(Error::new(format!("empty range {lo}..={hi}")));
        }
        ranges.sort_unstable();
        let mut merged: Vec<(i16, i16)> = Vec::with_capacity(ranges.len());
        for (lo, hi) in ranges {
            match merged.last_mut() {
                Some(last) if i32::from(lo) <= i32::from(last.1) + 1 => last.1 = last.1.max(hi),
                _ => merged.push((lo, hi)),
            }
        }
        if merged.is_empty() {
            return Err(Error::new("a parameter domain needs at least one value"));
        }
        Ok(Self { ranges: merged })
    }

    /// Exactly `values`.
    pub fn from_values(values: impl IntoIterator<Item = i16>) -> Result<Self> {
        Self::from_ranges(values.into_iter().map(|v| (v, v)))
    }

    /// Parses `1,4,9`, `0..=3` or a mix (`-1,2..=5`).
    pub fn parse(text: &str) -> Result<Self> {
        let number = |s: &str| -> Result<i16> {
            let s = s.trim();
            let (neg, digits) = s.strip_prefix('-').map_or((false, s), |d| (true, d));
            // One optional minus sign, then digits only: `--1` and `+1` are errors.
            let bad = || Error::new(format!("'{s}' is not an integer"));
            let (radix, body) = match digits
                .strip_prefix("0x")
                .or_else(|| digits.strip_prefix("0X"))
            {
                Some(hex) => (16, hex),
                None => (10, digits),
            };
            if body.is_empty() || !body.chars().all(|c| c.is_digit(radix)) {
                return Err(bad());
            }
            let magnitude = i32::from_str_radix(body, radix).map_err(|_| bad())?;
            let value = if neg { -magnitude } else { magnitude };
            i16::try_from(value).map_err(|_| Error::new(format!("{value} is out of i16 range")))
        };
        let mut ranges = Vec::new();
        for part in text.split(',') {
            let part = part.trim();
            if part.is_empty() {
                return Err(Error::new(format!("empty item in domain '{text}'")));
            }
            match part.split_once("..=") {
                Some((lo, hi)) => ranges.push((number(lo)?, number(hi)?)),
                None if part.contains("..") => {
                    return Err(Error::new(format!(
                        "'{part}': write inclusive ranges as lo..=hi"
                    )));
                }
                None => {
                    let v = number(part)?;
                    ranges.push((v, v));
                }
            }
        }
        Self::from_ranges(ranges)
    }

    /// The ranges, sorted and disjoint.
    #[must_use]
    pub fn ranges(&self) -> &[(i16, i16)] {
        &self.ranges
    }

    #[must_use]
    pub fn contains(&self, value: i16) -> bool {
        self.ranges
            .iter()
            .any(|&(lo, hi)| (lo..=hi).contains(&value))
    }

    /// The smallest single range holding every value.
    #[must_use]
    pub fn hull(&self) -> (i16, i16) {
        (self.ranges[0].0, self.ranges[self.ranges.len() - 1].1)
    }

    /// Whether this is every `i16`, which assumes nothing.
    #[must_use]
    pub fn is_everything(&self) -> bool {
        self.ranges == [(i16::MIN, i16::MAX)]
    }
}

impl fmt::Display for ParamDomain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, &(lo, hi)) in self.ranges.iter().enumerate() {
            if i > 0 {
                f.write_str(",")?;
            }
            if lo == hi {
                write!(f, "{lo}")?;
            } else {
                write!(f, "{lo}..={hi}")?;
            }
        }
        Ok(())
    }
}

/// Parses one `--param-domain` argument, `<tree>:<index>=<set>` (tree in decimal or
/// `0x` hex).
pub fn parse_param_domain_arg(arg: &str) -> Result<(u16, u16, ParamDomain)> {
    let bad = || {
        Error::new(format!(
            "--param-domain '{arg}': expected <tree>:<index>=<set>"
        ))
    };
    let (target, set) = arg.split_once('=').ok_or_else(bad)?;
    let (tree, index) = target.split_once(':').ok_or_else(bad)?;
    let tree = tree.trim();
    let tree = match tree.strip_prefix("0x").or_else(|| tree.strip_prefix("0X")) {
        Some(hex) => u16::from_str_radix(hex, 16),
        None => tree.parse(),
    }
    .map_err(|_| Error::new(format!("--param-domain '{arg}': '{tree}' is not a tree ID")))?;
    let index: u16 = index.trim().parse().map_err(|_| {
        Error::new(format!(
            "--param-domain '{arg}': '{index}' is not a parameter index"
        ))
    })?;
    let domain =
        ParamDomain::parse(set).map_err(|e| Error::new(format!("--param-domain '{arg}': {e}")))?;
    Ok((tree, index, domain))
}

/// Entry regions for a tree whose constrained parameters (below `args`) are `domains`:
/// each region bounds every constrained parameter to one range, `(index, lo, hi)`. The
/// regions partition the domain. With no constraint there is one empty region.
pub(crate) fn entry_regions(
    domains: Option<&BTreeMap<u16, ParamDomain>>,
    args: usize,
) -> Vec<Vec<(usize, i16, i16)>> {
    let constrained: Vec<(usize, &ParamDomain)> = domains
        .into_iter()
        .flatten()
        .filter(|(i, d)| usize::from(**i) < args && !d.is_everything())
        .map(|(&i, d)| (usize::from(i), d))
        .collect();
    let count = constrained
        .iter()
        .try_fold(1usize, |n, (_, d)| n.checked_mul(d.ranges().len()));
    let wide = count.is_none_or(|n| n > MAX_DOMAIN_REGIONS);
    let mut regions: Vec<Vec<(usize, i16, i16)>> = vec![Vec::new()];
    for (i, d) in constrained {
        let ranges = if wide {
            vec![d.hull()]
        } else {
            d.ranges().to_vec()
        };
        regions = regions
            .into_iter()
            .flat_map(|region| {
                ranges.iter().map(move |&(lo, hi)| {
                    let mut r = region.clone();
                    r.push((i, lo, hi));
                    r
                })
            })
            .collect();
    }
    regions
}

/// One assumption [`v2_enum_param_domains`] found in a v2 source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct V2EnumDomain {
    /// The BHAV the tree lowers to.
    pub tree: u16,
    /// The tree's name in the source.
    pub tree_name: String,
    /// The BHAV label it lowers to, which the files compared must give that BHAV too.
    pub label: String,
    /// The object's GUID, which the files compared must define too.
    pub object_guid: u32,
    pub param_index: u16,
    pub param_name: String,
    pub enum_name: String,
    pub domain: ParamDomain,
}

/// Every enum-typed positional parameter of every standalone `#[bhav] fn` in the v2 source at
/// `path`, with its enum's exact variant values.
///
/// `param[i]` counts positional parameters only: a leading `sim: Sim` binding and
/// `#[temp(N)]` parameters are not BHAV parameters. Trees are matched to BHAV IDs by label
/// in the lowered object, so the source must declare exactly one object.
pub fn v2_enum_param_domains(path: &Path, pack_dir: Option<&Path>) -> Result<Vec<V2EnumDomain>> {
    use crate::v2::ast::{AbiParamSlot, ObjectMember, V2Type};

    let resolved = crate::v2::resolve_v2_file(path, pack_dir)?;
    let [object] = resolved.objects.as_slice() else {
        return Err(Error::new(format!(
            "{} declares {} objects; --v2-enum-domains needs exactly one",
            path.display(),
            resolved.objects.len()
        )));
    };
    let expanded = crate::v2::lower_resolved_module(&resolved)?;
    let [expanded] = expanded.as_slice() else {
        return Err(Error::new("the v2 source lowered to more than one object"));
    };
    // Plain trees by label; interactions (which may share a label) take no arguments.
    let air = crate::parser::parse_module(&expanded.air_source)?;
    let compiled = crate::compile::compile_antics(&expanded.air_source)?;
    let mut by_label: BTreeMap<&str, Vec<u16>> = BTreeMap::new();
    for item in &air.items {
        if let crate::ast::Item::Tree(t) = item
            && !t.is_interaction
        {
            by_label.entry(t.label.as_str()).or_default().push(t.id);
        }
    }

    // The object's effective trees, each with the module it was declared in, merged as
    // the v2 lowering does: the template's first, then the object's, an object tree
    // replacing a template tree of the same name.
    let template = match &object.from_template {
        Some((name, _)) => Some(
            resolved
                .templates
                .get(name)
                .ok_or_else(|| Error::new(format!("unknown template '{name}'")))?,
        ),
        None => None,
    };
    let layers = template
        .map(|(module, t)| (module.as_str(), t.members.as_slice()))
        .into_iter()
        .chain(std::iter::once((
            resolved.module_id.as_str(),
            object.members.as_slice(),
        )));
    let mut trees: Vec<(&str, &crate::v2::ast::TreeMemberDecl)> = Vec::new();
    // Object- and template-local types by name: an enum's values, or `None` for bitflags.
    let mut local_types: BTreeMap<&str, Option<Vec<i16>>> = BTreeMap::new();
    for (module, members) in layers {
        for member in members {
            match member {
                ObjectMember::Tree(t) => match trees.iter().position(|(_, x)| x.ident == t.ident) {
                    Some(pos) => trees[pos] = (module, t),
                    None => trees.push((module, t)),
                },
                ObjectMember::Enum(e) => {
                    local_types.insert(&e.name, Some(e.variants.iter().map(|v| v.value).collect()));
                }
                ObjectMember::Bitflags(b) => {
                    local_types.insert(&b.name, None);
                }
                _ => {}
            }
        }
    }
    // A parameter type resolves like the v2 lowering's `lookup_named_type`: local types,
    // then the built-in names, then the scope of the module that declared the tree. A
    // name that is not an enum there gets no domain, which only assumes less.
    let enum_values = |module: &str, name: &str| -> Option<Vec<i16>> {
        if let Some(local) = local_types.get(name) {
            return local.clone();
        }
        if matches!(name, "Height" | "Placement" | "Self") {
            return None;
        }
        let (_, e) = resolved.module_scopes.get(module)?.enums.get(name)?;
        Some(e.variants.iter().map(|v| v.value).collect())
    };

    let mut out = Vec::new();
    for &(module, tree) in &trees {
        let positional = tree
            .params
            .iter()
            .filter(|p| p.slot == AbiParamSlot::Positional);
        for (index, param) in positional.enumerate() {
            let V2Type::Named(enum_name) = &param.ty else {
                continue;
            };
            let Some(values) = enum_values(module, enum_name) else {
                continue;
            };
            let labeled = by_label.get(tree.label.as_str()).map(Vec::as_slice);
            let id = match (tree.pinned_id, labeled) {
                // `#[id(N)]` names the BHAV even when other trees share the label.
                (Some(pinned), Some(ids)) if ids.contains(&pinned) => pinned,
                (Some(pinned), _) => {
                    return Err(Error::new(format!(
                        "tree '{}' is pinned to BHAV #{pinned}, which the lowered object does not label \"{}\"",
                        tree.ident, tree.label
                    )));
                }
                (None, Some([id])) => *id,
                (None, Some(ids)) => {
                    return Err(Error::new(format!(
                        "tree '{}' (label \"{}\") matches {} BHAVs in the lowered object",
                        tree.ident,
                        tree.label,
                        ids.len()
                    )));
                }
                (None, None) => {
                    return Err(Error::new(format!(
                        "tree '{}' (label \"{}\") is not in the lowered object",
                        tree.ident, tree.label
                    )));
                }
            };
            // The lowered tree must take the positional parameters as its arguments.
            let positional_count = tree
                .params
                .iter()
                .filter(|p| p.slot == AbiParamSlot::Positional)
                .count();
            // `#[frame(args = N)]` may give it more.
            let args = compiled.behaviors.get(&id).map(|b| usize::from(b.args));
            if args.is_none_or(|args| args < positional_count) {
                return Err(Error::new(format!(
                    "tree '{}' lowered to BHAV #{id} with {args:?} arguments, fewer than its {positional_count} positional parameters",
                    tree.ident
                )));
            }
            out.push(V2EnumDomain {
                tree: id,
                tree_name: tree.ident.clone(),
                label: tree.label.clone(),
                object_guid: expanded.guid,
                param_index: u16::try_from(index).unwrap_or(u16::MAX),
                param_name: param.name.clone(),
                enum_name: enum_name.clone(),
                domain: ParamDomain::from_values(values.iter().copied()).map_err(|e| {
                    Error::new(format!("enum {enum_name} of tree '{}': {e}", tree.ident))
                })?,
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domains_parse_merge_and_print() {
        let d = ParamDomain::parse("3, 0..=1, 2, -4, 9..=9, 0x10").unwrap();
        assert_eq!(d.ranges(), &[(-4, -4), (0, 3), (9, 9), (16, 16)]);
        assert_eq!(d.to_string(), "-4,0..=3,9,16");
        assert!(d.contains(2) && !d.contains(4) && !d.contains(-3));
        assert_eq!(d.hull(), (-4, 16));
        for bad in [
            "", "1,,2", "3..=1", "0..3", "x", "40000", "--1", "+1", "-+1", "0x-1", "0x",
        ] {
            assert!(ParamDomain::parse(bad).is_err(), "{bad}");
        }
        let (tree, index, domain) = parse_param_domain_arg("0x1001:2=0..=3").unwrap();
        assert_eq!((tree, index, domain.to_string()), (4097, 2, "0..=3".into()));
        assert!(parse_param_domain_arg("4097=1").is_err());
    }

    #[test]
    fn regions_partition_and_widen() {
        let mut domains = BTreeMap::new();
        domains.insert(0, ParamDomain::parse("0..=1,5").unwrap());
        domains.insert(1, ParamDomain::parse("7,9").unwrap());
        // Parameter 3 is past the tree's arguments: ignored.
        domains.insert(3, ParamDomain::parse("1").unwrap());
        let regions = entry_regions(Some(&domains), 2);
        assert_eq!(regions.len(), 4);
        assert!(regions.contains(&vec![(0, 5, 5), (1, 9, 9)]));
        assert_eq!(entry_regions(None, 4), vec![Vec::new()]);
        let many = ParamDomain::from_values((0..300).map(|v| v * 2)).unwrap();
        let wide = entry_regions(Some(&BTreeMap::from([(0, many)])), 1);
        assert_eq!(wide, vec![vec![(0, 0, 598)]]);
    }
}
