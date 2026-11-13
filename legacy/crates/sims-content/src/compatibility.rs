//! Full physical-corpus census. Never equates static presence with runtime reachability.
use crate::{Index, Resource};
use anyhow::{Context, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sims_formats::{
    behavior::{Behavior, InteractionTable, ObjectDefinition},
    execution,
    iff::Iff,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{Read, Seek, SeekFrom},
};

#[derive(Debug, Serialize, Default)]
pub struct Counts {
    pub containers: u64,
    pub behaviors: u64,
    pub instructions: u64,
    pub primitive_instructions: u64,
    pub literal_calls: u64,
}
#[derive(Debug, Serialize, Default)]
pub struct Variant {
    pub instructions: u64,
    pub examples: Vec<String>,
}
#[derive(Debug, Serialize)]
pub struct Opcode {
    #[serde(flatten)]
    pub support: sims_vm::compatibility::PrimitiveSupport,
    pub instructions: u64,
    pub routines: u64,
    pub groups: BTreeMap<String, u64>,
    pub operand_variants: BTreeMap<String, Variant>,
}
#[derive(Debug, Serialize, Default)]
pub struct ExpressionUsage {
    pub operators: BTreeMap<u8, u64>,
    pub byte4: BTreeMap<u8, u64>,
    pub lhs_scopes: BTreeMap<u8, u64>,
    pub rhs_scopes: BTreeMap<u8, u64>,
    pub unsupported_read_scope_instructions: u64,
    pub unsupported_write_scope_instructions: u64,
    pub negative_literal_sqrt: u64,
}
#[derive(Debug, Serialize)]
pub struct Routine {
    pub id: u16,
    pub format: u16,
    pub args: u8,
    pub locals: u16,
    pub instructions: usize,
    pub primitives: BTreeMap<u16, u64>,
    pub calls: BTreeMap<u16, u64>,
    pub invalid_branches: u64,
    pub exceeds_vm_dimensions: bool,
    pub has_unsupported_opcode: bool,
    pub has_dynamic_dispatch: bool,
}
#[derive(Debug, Serialize)]
pub struct Container {
    pub locator: String,
    pub name: String,
    pub group: String,
    pub sha256: String,
    pub chunk_types: BTreeMap<String, u64>,
    pub glob_links: Vec<String>,
    pub routines: Vec<Routine>,
    pub interaction_entries: Vec<InteractionEntry>,
    pub function_tables: Vec<FunctionTable>,
    pub object_callbacks: Vec<ObjectCallbacks>,
    pub duplicate_bhav_ids: Vec<u16>,
}
#[derive(Debug, Serialize)]
pub struct FunctionTable {
    pub id: u16,
    pub table: execution::ObjectFunctions,
}
#[derive(Debug, Serialize)]
pub struct ObjectCallbacks {
    pub object: u16,
    pub uses_function_table: bool,
    /// Slot -> BHAV, from the on-disk fields corresponding to native 0x00456012.
    /// Retained alongside OBJf, without pretending that callback precedence is implemented.
    pub fallback_entries: BTreeMap<u16, u16>,
}
#[derive(Debug, Serialize)]
pub struct InteractionEntry {
    pub table: u16,
    pub action: u16,
    pub check: u16,
}
#[derive(Debug, Serialize)]
pub struct Dependency {
    pub source: String,
    pub from_bhav: u16,
    pub to_bhav: u16,
    pub instructions: u64,
    pub scope: &'static str,
    pub resolution: &'static str,
    pub candidate_sources: Vec<String>,
}
#[derive(Debug, Serialize)]
pub struct Report {
    pub schema: u32,
    pub corpus_sha256: String,
    pub indexed_resources: usize,
    pub archives: usize,
    pub probed_resources: usize,
    pub totals: Counts,
    pub groups: BTreeMap<String, Counts>,
    pub container_extensions: BTreeMap<String, u64>,
    pub bhav_versions: BTreeMap<u16, u64>,
    pub opcodes: Vec<Opcode>,
    pub expression: ExpressionUsage,
    pub containers: Vec<Container>,
    pub dependencies: Vec<Dependency>,
    pub dependency_resolution_counts: BTreeMap<String, u64>,
    pub diagnostics: Vec<String>,
    pub native_evidence: Option<serde_json::Value>,
    pub caveats: Vec<&'static str>,
}

fn prefix(resource: &Resource) -> Result<Vec<u8>> {
    let mut file = File::open(&resource.source)?;
    anyhow::ensure!(
        resource.size == resource.unpacked_size,
        "compressed resource cannot be signature-probed"
    );
    let length = file.metadata()?.len();
    anyhow::ensure!(
        resource
            .offset
            .checked_add(resource.size)
            .is_some_and(|end| end <= length),
        "resource outside file"
    );
    file.seek(SeekFrom::Start(resource.offset))?;
    let mut bytes = vec![0; resource.size.min(64) as usize];
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}
fn increment<K: Ord>(map: &mut BTreeMap<K, u64>, key: K) {
    *map.entry(key).or_default() += 1;
}

pub fn inventory(index: &Index) -> Report {
    let mut report = Report {
        schema: 1,
        corpus_sha256: String::new(),
        indexed_resources: index.resources.len(),
        archives: index.archives,
        probed_resources: 0,
        totals: Counts::default(),
        groups: BTreeMap::new(),
        container_extensions: BTreeMap::new(),
        bhav_versions: BTreeMap::new(),
        opcodes: (0..256)
            .map(|opcode| Opcode {
                support: sims_vm::compatibility::primitive(opcode),
                instructions: 0,
                routines: 0,
                groups: BTreeMap::new(),
                operand_variants: BTreeMap::new(),
            })
            .collect(),
        expression: ExpressionUsage::default(),
        containers: vec![],
        dependencies: vec![],
        dependency_resolution_counts: BTreeMap::new(),
        diagnostics: index.diagnostics.clone(),
        native_evidence: None,
        caveats: vec![
            "Physical corpus: all indexed resources are signature-probed, including non-.iff extensions; copies/overrides are not collapsed.",
            "Group means extraction source directory, not an inferred expansion runtime configuration.",
            "Static instruction counts include unreachable code; opcode support is partial and not gameplay coverage.",
            "Literal-call resolution retains duplicate/ambiguous sources; expansion load precedence is not guessed.",
            "Shared modules execute in a caller's private context. A local call inside a global/semiglobal module may require that external context.",
            "TTAB, OBJf and OBJD callbacks are inventoried as additional roots; callback binding/precedence and dynamic tree lookup are not closed by the literal-call graph.",
            "Scope presence does not establish index/field/side-effect compatibility; native memory-scope discrepancies remain visible research work.",
        ],
    };
    let mut hash = Sha256::new();
    hash.update(b"sims-rs/physical-bhav-corpus/v1\0");
    for resource in &index.resources {
        hash.update((resource.locator.len() as u64).to_le_bytes());
        hash.update(resource.locator.as_bytes());
        hash.update(resource.size.to_le_bytes());
        let header = match prefix(resource) {
            Ok(bytes) => {
                report.probed_resources += 1;
                bytes
            }
            Err(error) => {
                report
                    .diagnostics
                    .push(format!("{}: signature probe: {error:#}", resource.locator));
                continue;
            }
        };
        if !header.starts_with(b"IFF FILE 2.5:") && !header.starts_with(b"IFF FILE 2.0:") {
            if resource.is_iff() {
                report.diagnostics.push(format!(
                    "{}: .iff without recognized signature",
                    resource.locator
                ));
            }
            continue;
        }
        let iff = match resource
            .read()
            .and_then(|b| Iff::parse(b).context("IFF framing"))
        {
            Ok(iff) => iff,
            Err(error) => {
                report
                    .diagnostics
                    .push(format!("{}: {error:#}", resource.locator));
                continue;
            }
        };
        let digest = format!("{:x}", Sha256::digest(&iff.bytes));
        hash.update(digest.as_bytes());
        let group = resource
            .locator
            .split('/')
            .next()
            .unwrap_or("loose")
            .to_owned();
        report.totals.containers += 1;
        report.groups.entry(group.clone()).or_default().containers += 1;
        increment(
            &mut report.container_extensions,
            resource
                .name
                .rsplit_once('.')
                .map_or("<none>", |(_, ext)| ext)
                .to_ascii_lowercase(),
        );
        let mut container = Container {
            locator: resource.locator.clone(),
            name: resource.name.clone(),
            group: group.clone(),
            sha256: digest,
            chunk_types: BTreeMap::new(),
            glob_links: vec![],
            routines: vec![],
            interaction_entries: vec![],
            function_tables: vec![],
            object_callbacks: vec![],
            duplicate_bhav_ids: vec![],
        };
        let mut ids = BTreeSet::new();
        for chunk in &iff.chunks {
            increment(&mut container.chunk_types, chunk.kind());
            let bytes = iff.data(chunk);
            let result: Result<()> = (|| {
                match &chunk.tag {
                    b"OBJf" => container.function_tables.push(FunctionTable {
                        id: chunk.id,
                        table: execution::ObjectFunctions::parse(bytes)?,
                    }),
                    b"OBJD" => {
                        ObjectDefinition::parse(bytes)?;
                        let word = |offset: usize| {
                            bytes
                                .get(offset..offset + 2)
                                .map(|b| u16::from_le_bytes(b.try_into().unwrap()))
                        };
                        let offsets = [
                            0x56, 0x0a, 0x5e, 0x68, 0x78, 0x2a, 0x7c, 0x64, 0x9c, 0x58, 0x94, 0x60,
                            0x5a, 0x6a, 0x6e, 0x22, 0x0c, 0x18, 0x30, 0x32, 0x34, 0x36, 0x38, 0x3a,
                            0x3c, 0x3e, 0x40, 0x42, 0x76, 0xa0,
                        ];
                        let fallback_entries = offsets
                            .iter()
                            .enumerate()
                            .filter_map(|(slot, &offset)| {
                                word(offset)
                                    .filter(|&v| v != 0)
                                    .map(|target| (slot as u16, target))
                            })
                            .collect();
                        container.object_callbacks.push(ObjectCallbacks {
                            object: chunk.id,
                            uses_function_table: word(0x2c).unwrap_or(0) != 0,
                            fallback_entries,
                        });
                    }
                    b"GLOB" => container.glob_links.push(execution::global_link(bytes)?),
                    b"TTAB" => {
                        for entry in InteractionTable::parse(bytes)?.interactions {
                            container.interaction_entries.push(InteractionEntry {
                                table: chunk.id,
                                action: entry.action,
                                check: entry.test,
                            });
                        }
                    }
                    b"BHAV" => {
                        let behavior = Behavior::parse(bytes)?;
                        if !ids.insert(chunk.id) {
                            container.duplicate_bhav_ids.push(chunk.id);
                        }
                        increment(&mut report.bhav_versions, behavior.format);
                        let mut row = Routine {
                            id: chunk.id,
                            format: behavior.format,
                            args: behavior.args,
                            locals: behavior.locals,
                            instructions: behavior.instructions.len(),
                            primitives: BTreeMap::new(),
                            calls: BTreeMap::new(),
                            invalid_branches: 0,
                            exceeds_vm_dimensions: behavior.instructions.is_empty()
                                || (behavior.kind == 4 && behavior.instructions.len() > 251),
                            has_unsupported_opcode: false,
                            has_dynamic_dispatch: false,
                        };
                        report.totals.behaviors += 1;
                        report.groups.get_mut(&group).unwrap().behaviors += 1;
                        for (pc, ins) in behavior.instructions.iter().enumerate() {
                            report.totals.instructions += 1;
                            report.groups.get_mut(&group).unwrap().instructions += 1;
                            row.invalid_branches += [ins.on_true, ins.on_false]
                                .iter()
                                .filter(|&&b| {
                                    b < 253
                                        && (b > 250 || b as usize >= behavior.instructions.len())
                                })
                                .count() as u64;
                            if ins.opcode >= 256 {
                                increment(&mut row.calls, ins.opcode);
                                report.totals.literal_calls += 1;
                                report.groups.get_mut(&group).unwrap().literal_calls += 1;
                                continue;
                            }
                            report.totals.primitive_instructions += 1;
                            report
                                .groups
                                .get_mut(&group)
                                .unwrap()
                                .primitive_instructions += 1;
                            increment(&mut row.primitives, ins.opcode);
                            row.has_dynamic_dispatch |=
                                matches!(ins.opcode, 1 | 3 | 13 | 14 | 20 | 28 | 30 | 42);
                            let op = &mut report.opcodes[ins.opcode as usize];
                            op.instructions += 1;
                            increment(&mut op.groups, group.clone());
                            row.has_unsupported_opcode |= op.support.rust_support == "unsupported";
                            let key = ins
                                .operands
                                .iter()
                                .map(|b| format!("{b:02x}"))
                                .collect::<String>();
                            let variant = op.operand_variants.entry(key).or_default();
                            variant.instructions += 1;
                            if variant.examples.len() < 3 {
                                variant.examples.push(format!(
                                    "{}#BHAV:{:04x}[{pc}]",
                                    resource.locator, chunk.id
                                ));
                            }
                            if ins.opcode == 2 {
                                let (operator, ls, rs) =
                                    (ins.operands[5], ins.operands[6], ins.operands[7]);
                                let e = &mut report.expression;
                                increment(&mut e.operators, operator);
                                increment(&mut e.byte4, ins.operands[4]);
                                increment(&mut e.lhs_scopes, ls);
                                increment(&mut e.rhs_scopes, rs);
                                e.unsupported_read_scope_instructions += u64::from(
                                    !sims_vm::compatibility::read_scope_supported(ls)
                                        || !sims_vm::compatibility::read_scope_supported(rs),
                                );
                                if !matches!(operator, 0..=2 | 8 | 14..=16) {
                                    e.unsupported_write_scope_instructions += u64::from(
                                        !sims_vm::compatibility::write_scope_supported(ls),
                                    );
                                }
                                e.negative_literal_sqrt += u64::from(
                                    operator == 20
                                        && rs == 7
                                        && i16::from_le_bytes(
                                            ins.operands[2..4].try_into().unwrap(),
                                        ) < 0,
                                );
                            }
                        }
                        for &opcode in row.primitives.keys() {
                            report.opcodes[opcode as usize].routines += 1;
                        }
                        container.routines.push(row);
                    }
                    _ => (),
                }
                Ok(())
            })();
            if let Err(error) = result {
                report.diagnostics.push(format!(
                    "{} {}#{}: {error:#}",
                    resource.locator,
                    chunk.kind(),
                    chunk.id
                ));
            }
        }
        report.containers.push(container);
    }
    report.corpus_sha256 = format!("{:x}", hash.finalize());
    let mut names: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (id, container) in report.containers.iter().enumerate() {
        names
            .entry(container.name.to_ascii_lowercase())
            .or_default()
            .push(id);
    }
    let globals = names.get("global.iff").cloned().unwrap_or_default();
    let links: Vec<Vec<usize>> = report
        .containers
        .iter()
        .map(|container| {
            container
                .glob_links
                .iter()
                .flat_map(|name| {
                    let mut name = name.to_ascii_lowercase();
                    if !name.ends_with(".iff") {
                        name.push_str(".iff");
                    }
                    names.get(&name).cloned().unwrap_or_default()
                })
                .collect()
        })
        .collect();
    let mut linked_owners: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (owner, targets) in links.iter().enumerate() {
        for &target in targets {
            linked_owners.entry(target).or_default().push(owner);
        }
    }
    for (id, container) in report.containers.iter().enumerate() {
        let caller_owners = if globals.contains(&id) {
            (0..report.containers.len())
                .filter(|&i| {
                    !report.containers[i].object_callbacks.is_empty()
                        || report.containers[i]
                            .routines
                            .iter()
                            .any(|r| (4096..8192).contains(&r.id))
                })
                .collect::<Vec<_>>()
        } else {
            linked_owners.get(&id).cloned().unwrap_or_else(|| vec![id])
        };
        for routine in &container.routines {
            for (&target, &instructions) in &routine.calls {
                let (scope, candidates) = match target {
                    256..=4095 => ("global", globals.clone()),
                    4096..=8191 => ("private", caller_owners.clone()),
                    _ => (
                        "semiglobal",
                        caller_owners
                            .iter()
                            .flat_map(|&owner| links[owner].iter().copied())
                            .collect::<BTreeSet<_>>()
                            .into_iter()
                            .collect(),
                    ),
                };
                let matches = candidates
                    .iter()
                    .flat_map(|&i| {
                        report.containers[i]
                            .routines
                            .iter()
                            .filter(move |r| r.id == target)
                            .map(move |_| i)
                    })
                    .count();
                let resolution = if target & 0x8000 != 0 {
                    "high_bit_opcode_needs_upstream_classification"
                } else if candidates.is_empty() {
                    "missing_module_or_unbound_caller_context"
                } else if candidates.len() > 1 || matches > 1 {
                    "ambiguous_physical_sources"
                } else if matches == 0 && scope == "private" {
                    "missing_private_target_or_external_caller_context"
                } else if matches == 0 {
                    "missing_target"
                } else if scope == "private" && !(4096..8192).contains(&routine.id) {
                    "resolved_in_container_caller_context_unproven"
                } else {
                    "unique_physical_target"
                };
                increment(
                    &mut report.dependency_resolution_counts,
                    resolution.to_owned(),
                );
                report.dependencies.push(Dependency {
                    source: container.locator.clone(),
                    from_bhav: routine.id,
                    to_bhav: target,
                    instructions,
                    scope,
                    resolution,
                    candidate_sources: candidates
                        .iter()
                        .map(|&i| report.containers[i].locator.clone())
                        .collect(),
                });
            }
        }
    }
    report
}
