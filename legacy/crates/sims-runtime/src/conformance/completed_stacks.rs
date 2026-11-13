//! Cross-engine stack-storage comparison at one completed original boundary.
//! This never constructs a Thread, lifecycle, World or Session, and cannot
//! execute. Invocation/action-use metadata is outside this component contract.
use super::{AdapterResult, Scope, Status};
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use sims_fixtures::{ArtifactFormat, CaseManifest, Coverage, Store};
use sims_vm::retained_frames::{ContextBinding, RetainedFrame, RetainedStack};
use sims_vm::{Definition, Frame, Location, Module, Program, Routine};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
};

pub const CONTRACT: &str = "completed-native-stack-component-v1";
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    schema: u32,
    contract: String,
    tick: u64,
    global_source: String,
    modules: Vec<ModuleInput>,
    objects: Vec<ObjectInput>,
    scope: String,
    unqualified: Vec<String>,
    native_parity_ticks: u64,
    provenance: Value,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModuleInput {
    source: String,
    semiglobal: Option<String>,
    definitions: BTreeMap<u16, Vec<i16>>,
    routines: BTreeMap<u16, RoutineInput>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RoutineInput {
    id: u16,
    payload: String,
    receipt: Value,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ObjectInput {
    object: i16,
    definition: Definition,
    context: u32,
    arena: String,
    frames: Vec<NativeFrame>,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeFrame {
    tree: u16,
    node: usize,
    stack_object: i16,
    args: Vec<i16>,
    locals: Vec<i16>,
    continuation: u32,
    context: u32,
    offset_words: usize,
    routine_source: String,
}
fn bytes(value: &str) -> Result<Vec<u8>> {
    ensure!(
        value.len() % 2 == 0 && value.len() <= 16 << 20,
        "invalid bounded hex"
    );
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|s| Ok(u8::from_str_radix(std::str::from_utf8(s)?, 16)?))
        .collect()
}

fn import_program_input(input: &Input) -> Result<Program> {
    let mut program = Program {
        globals: input.global_source.clone(),
        ..Program::default()
    };
    for source in &input.modules {
        ensure!(
            !source.source.is_empty() && !program.modules.contains_key(&source.source),
            "duplicate module owner"
        );
        let mut module = Module {
            source: source.source.clone(),
            semiglobal: source.semiglobal.clone(),
            object_definition_words: source.definitions.clone(),
            ..Module::default()
        };
        // This deliberately partial Program supplies only the native payloads
        // consumed by RetainedStack validation. Empty unrelated banks grant no
        // resource capability, Thread owner, execution, or Session admission.
        for (&id, routine) in &source.routines {
            ensure!(id == routine.id, "routine key differs");
            let payload = bytes(&routine.payload)?;
            ensure!(
                sims_fixtures::digest(&payload)
                    == routine.receipt["payload_sha256"]
                        .as_str()
                        .context("resident payload hash absent")?,
                "resident BHAV digest differs"
            );
            module.routines.insert(
                id,
                Routine {
                    name: format!("resident:{id}"),
                    body: sims_formats::behavior::Behavior::parse(&payload)?,
                },
            );
        }
        program.modules.insert(source.source.clone(), module);
    }
    Ok(program)
}

fn evaluate(input: Input) -> Result<Value> {
    ensure!(
        input.schema == 1 && input.contract == CONTRACT && input.native_parity_ticks == 0,
        "unsupported stack component schema/credit"
    );
    ensure!(
        !input.scope.is_empty() && input.unqualified.len() >= 5,
        "component exclusions absent"
    );
    ensure!(
        input.provenance["image_sha256"]
            == "877982344b1c8b95eec2858bd40f11551b8f2b2805dd1ce7fc6eaac2c983109f"
            && input.provenance["observation_sha256"]
                == "63d1eff4057d012969978990c689889561e158f58ccee5e894a536f2c570e00c",
        "original boundary source profile differs"
    );
    ensure!(
        input.tick == 88448 && input.objects.len() == 344,
        "incomplete original object coverage"
    );
    let program = import_program_input(&input)?;
    let routine_count: usize = program.modules.values().map(|m| m.routines.len()).sum();
    let mut seen = BTreeSet::new();
    let mut rows = Vec::new();
    let mut frame_count = 0;
    for native in input.objects {
        ensure!(
            native.object > 0 && seen.insert(native.object),
            "duplicate/invalid original object"
        );
        ensure!(
            native.context != 0 && !native.frames.is_empty(),
            "active native context/frames absent"
        );
        frame_count += native.frames.len();
        let stack = import_object(&program, &native)?;
        let raw = bytes(&native.arena)?;
        rows.push(
            json!({"object":native.object,"frames":stack.frames().len(),"arena_bytes":raw.len(),
            "arena_sha256":sims_fixtures::digest(&raw),"native_stack_equal":true}),
        );
    }
    ensure!(
        frame_count == 781 && routine_count == 158,
        "incomplete frame/routine coverage"
    );
    Ok(
        json!({"schema":1,"contract":CONTRACT,"tick":input.tick,"objects":rows,
        "frames":frame_count,"routines":routine_count,"native_parity_ticks":0,
        "resumable_baseline":false,"unqualified":input.unqualified}),
    )
}

fn import_object(program: &Program, native: &ObjectInput) -> Result<RetainedStack> {
    import_object_lifetime(program, native, None)
}

fn import_object_lifetime(
    program: &Program,
    native: &ObjectInput,
    lifetime: Option<u32>,
) -> Result<RetainedStack> {
    let raw = bytes(&native.arena)?;
    let mut frames = Vec::new();
    for f in native.frames.clone() {
        ensure!(
            f.context == native.context && f.continuation == 0 && f.tree < 0x8000,
            "unsupported invocation/context/continuation profile"
        );
        let (source, _) = program
            .resolve(&native.definition.source, f.tree)
            .map_err(anyhow::Error::msg)?;
        ensure!(
            source == f.routine_source,
            "resident scope/provider differs"
        );
        // Only raw header/value/context fields below enter this comparison.
        // These object association fields do not assert native invocation
        // or action-use ownership, and no Thread is created from this view.
        frames.push(RetainedFrame {
            context_object: native.object,
            frame: Frame {
                continuation: f.continuation,
                action_target: None,
                action_icon: false,
                code_owner: native.definition.source.clone(),
                location: Location {
                    source: f.routine_source,
                    bhav: f.tree,
                    instruction: f.node,
                },
                caller: native.object,
                callee: native.object,
                stack_object: f.stack_object,
                args: f.args,
                locals: f.locals,
                opaque_locals: BTreeSet::new(),
                storage_offset: f.offset_words,
            },
        });
    }
    let stack = RetainedStack::from_bytes(
        program,
        &raw,
        frames,
        vec![ContextBinding {
            object: native.object,
            definition: native.definition.clone(),
            native_context: native.context,
            registry_lifetime: lifetime, // Unknown: stack decoding only, not Thread admission.
        }],
    )
    .map_err(anyhow::Error::msg)
    .with_context(|| format!("native object {}", native.object))?;
    ensure!(stack.raw_bytes() == raw, "retained Rust arena differs");
    Ok(stack)
}

/// Reuse the same raw-arena/resource decoder for a separately qualified owner.
/// Supplying fresh canonical registry lifetimes adds no execution capability.
pub(super) fn import_storage(
    value: &Value,
    storage: &sims_vm::scheduler::storage::Storage,
) -> Result<(Program, BTreeMap<i16, RetainedStack>)> {
    let input: Input = serde_json::from_value(value.clone())?;
    // Keep the original component's complete profile/provenance/coverage checks.
    // This result is not a whole-Program or executable VM admission.
    evaluate(serde_json::from_value(value.clone())?)?;
    let program = import_program_input(&input)?;
    let mut stacks = BTreeMap::new();
    for row in &input.objects {
        let lifetime = storage
            .handle(row.object)
            .context("retained registry lifetime absent")?;
        ensure!(
            stacks
                .insert(
                    row.object,
                    import_object_lifetime(&program, row, Some(lifetime))?
                )
                .is_none(),
            "duplicate retained stack owner"
        );
    }
    Ok((program, stacks))
}

pub fn run(store: &Store, manifest: &CaseManifest, output: &Path) -> Result<AdapterResult> {
    ensure!(
        manifest.source.format == ArtifactFormat::Json && manifest.source.schema == 1,
        "stack fixture must preserve exact source JSON"
    );
    ensure!(
        matches!(&manifest.coverage, Coverage::Component { scope, .. } if scope == CONTRACT),
        "stack fixture contract coverage differs"
    );
    let input: Input = serde_json::from_slice(&store.read_bytes(&manifest.source.blob, 16 << 20)?)?;
    let report = evaluate(input)?;
    let path = output.join("completed-stacks.json");
    let mut f = OpenOptions::new().write(true).create_new(true).open(path)?;
    serde_json::to_writer_pretty(&mut f, &report)?;
    f.write_all(b"\n")?;
    Ok(AdapterResult { status:Status::Passed,scope:Scope::OriginalComponent,contract:CONTRACT.into(),
        observations:344,completed_ticks:0,matched_completed_ticks:0,difference:None,
        detail:Some("344 captured arenas/781 frames match the Rust retained-stack importer; lifecycle/invocation/World/Session and gameplay progression remain unqualified".into()),
        artifacts:vec![PathBuf::from("completed-stacks.json")] })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn hex(b: &[u8]) -> String {
        b.iter().map(|v| format!("{v:02x}")).collect()
    }
    fn fixture() -> (Program, ObjectInput) {
        let mut words = vec![0; 108];
        words[14] = 0x3344;
        words[15] = 0x1122;
        words[20] = 2;
        let mut payload = vec![0x02, 0x80, 1, 0, 0, 1, 1, 0, 0, 0, 0, 0];
        payload.extend_from_slice(&[0, 0, 254, 255, 0, 0, 0, 0, 0, 0, 0, 0]);
        let mut module = Module {
            source: "private".into(),
            ..Module::default()
        };
        module.routines.insert(
            4096,
            Routine {
                name: "authored".into(),
                body: sims_formats::behavior::Behavior::parse(&payload).unwrap(),
            },
        );
        module.object_definition_words.insert(1, words);
        let p = Program {
            globals: "global".into(),
            modules: BTreeMap::from([("private".into(), module)]),
            ..Program::default()
        };
        let mut arena = vec![0x5a; 64];
        for (i, v) in [4096i16, 0, -1, 0x101, 0, 0, 0, 0x70, -32768, -4096]
            .iter()
            .enumerate()
        {
            arena[2 * i..2 * i + 2].copy_from_slice(&v.to_le_bytes());
        }
        let o = ObjectInput {
            object: 7,
            definition: Definition {
                source: "private".into(),
                object: 1,
                guid: 0x11223344,
                slots: 2,
            },
            context: 0x700000,
            arena: hex(&arena),
            frames: vec![NativeFrame {
                tree: 4096,
                node: 0,
                stack_object: -1,
                args: vec![-32768],
                locals: vec![-4096],
                continuation: 0,
                context: 0x700000,
                offset_words: 0,
                routine_source: "private".into(),
            }],
        };
        (p, o)
    }
    #[test]
    fn signed_values_headers_padding_and_spare_arena_survive_real_importer() {
        let (p, o) = fixture();
        let raw = bytes(&o.arena).unwrap();
        let stack = import_object(&p, &o).unwrap();
        assert_eq!(stack.raw_bytes(), raw);
        assert_eq!(stack.frames()[0].locals, vec![-4096]);
        assert_eq!(stack.frames()[0].args, vec![-32768]);
        assert_eq!(&stack.raw_bytes()[24..], &[0x5a; 40]);
    }
    #[test]
    fn raw_bit_context_definition_scope_and_dimension_disagreement_reject() {
        for change in 0..9 {
            let (mut p, mut o) = fixture();
            match change {
                0 => {
                    let mut raw = bytes(&o.arena).unwrap();
                    raw[18] ^= 1;
                    o.arena = hex(&raw);
                }
                1 => o.frames[0].locals[0] ^= 1,
                2 => o.frames[0].context += 4,
                3 => o.definition.guid ^= 1,
                4 => o.frames[0].routine_source = "different".into(),
                5 => o.frames[0].node = 1,
                6 => o.frames[0].continuation = 1,
                7 => {
                    p.modules
                        .get_mut("private")
                        .unwrap()
                        .routines
                        .get_mut(&4096)
                        .unwrap()
                        .body
                        .locals = 2
                }
                _ => o.frames[0].offset_words = 4,
            }
            assert!(import_object(&p, &o).is_err(), "mutation {change} admitted");
        }
    }
}
