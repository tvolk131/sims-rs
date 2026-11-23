//! Concrete replay of a candidate counterexample in `sims-vm`.
//!
//! The symbolic checkers only ever *suggest* that two trees differ. A difference is
//! reported only after both trees have actually run in `sims-vm` from the same concrete
//! world and frame, built from the witness inputs, and the two runs ended observably
//! differently. Anything else is undecided.
//!
//! The replay world is a minimal lot: one object (`me`, also the stack object) of a
//! synthetic type whose definition words are zero apart from its GUID and type, with
//! the witness's temps, attributes, parameters and initial frame-storage words, and
//! everything else zero. Each side's program holds its tree and its file's `BCON`
//! tables and nothing else, so only trees made of `Expression` nodes are replayed: a
//! primitive or a call could reach code or state the replay world does not have.
//!
//! A run counts only if it returns: a fault (any VM trap, including running out of
//! the instruction budget) means the witness shows nothing, so the result stays
//! undecided.

use super::LiveOptions;
use super::live::LiveContext;
use crate::ir::BhavChunk;
use sims_content::Catalog;
use sims_formats::behavior::Behavior;
use sims_formats::saved_objects::Slot as SavedSlot;
use sims_sim::{FrameStorage, Object, ScratchPtrs, World};
use sims_vm::{DEFINITION_WORDS, Interpreter, Library, Program, Stream};
use std::collections::{BTreeMap, BTreeSet};

/// The replay object's ID.
const ME: u16 = 1;
/// The replay object's type ID and GUID.
const TYPE: u16 = 1;
const GUID: u32 = 0x7E57_0001;
/// The `OBJD` chunk ID of the synthetic definition.
const DEFINITION: u16 = 128;
/// Instructions one replayed run may execute. A run that needs more is undecided.
const BUDGET: usize = 100_000;
/// Temps every object has (`sims-vm/src/memory.rs:438`).
const TEMPS: usize = 8;

/// Concrete inputs for one replay.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Witness {
    /// The arguments the tree is called with (cut or zero-padded to each tree's count;
    /// for a kind-4 tree the first picks the entry node).
    pub params: Vec<i16>,
    /// Initial frame-storage words, by index after the frame header (`args + local`).
    pub frame_words: BTreeMap<u16, i16>,
    pub temps: BTreeMap<i16, i16>,
    pub attrs: BTreeMap<i16, i16>,
}

impl std::fmt::Display for Witness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fn nonzero<K: std::fmt::Display>(
            f: &mut std::fmt::Formatter<'_>,
            name: &str,
            map: &BTreeMap<K, i16>,
        ) -> std::fmt::Result {
            let set: Vec<String> = map
                .iter()
                .filter(|(_, v)| **v != 0)
                .map(|(k, v)| format!("[{k}]={v}"))
                .collect();
            if set.is_empty() {
                Ok(())
            } else {
                write!(f, ", {name} {}", set.join(" "))
            }
        }
        write!(f, "params {:?}", self.params)?;
        nonzero(f, "frame words", &self.frame_words)?;
        nonzero(f, "temps", &self.temps)?;
        nonzero(f, "my attributes", &self.attrs)
    }
}

/// Whether `bhav` can be replayed from `entry`: every node it can reach is an
/// `Expression`, so the run touches only the frame, the object and the lot's globals.
pub(crate) fn replayable(bhav: &BhavChunk, entry: usize) -> bool {
    let len = bhav.instructions.len();
    if len == 0 || bhav.id < 256 {
        return false;
    }
    let mut seen = BTreeSet::new();
    let mut work = vec![entry.min(len - 1)];
    while let Some(n) = work.pop() {
        if !seen.insert(n) {
            continue;
        }
        let ins = &bhav.instructions[n];
        if ins.opcode != 2 {
            return false;
        }
        for t in [ins.on_true, ins.on_false] {
            let t = usize::from(t);
            if t < len {
                work.push(t);
            }
        }
    }
    true
}

/// Runs both trees from the same world built from `w`. Returns how the runs differ, or
/// `None` when they do not, or when either run faults.
pub(crate) fn replay(
    a: &BhavChunk,
    ctx_a: &LiveContext,
    b: &BhavChunk,
    ctx_b: &LiveContext,
    opts: &LiveOptions,
    w: &Witness,
) -> Option<String> {
    let attributes = ctx_a
        .my_attribute_count
        .max(ctx_b.my_attribute_count)
        .max(super::GUARANTEED_ATTRIBUTES);
    let ra = run(a, ctx_a, w, attributes)?;
    let rb = run(b, ctx_b, w, attributes)?;
    let (oa, ob) = (observe(ra, opts), observe(rb, opts));
    let what = oa.difference(&ob)?;
    Some(format!("sims-vm runs both from {w}: {what}"))
}

fn program(bhav: &BhavChunk, ctx: &LiveContext) -> Option<Program> {
    let resource = match bhav.id {
        256..=4095 => 0,
        4096..=8191 => 1,
        8192.. => 2,
        _ => return None,
    };
    let mut libraries = BTreeMap::from([(0, Library::default()), (1, Library::default())]);
    if resource == 2 {
        libraries.insert(2, Library::default());
    }
    {
        let lib = libraries.get_mut(&resource)?;
        lib.behaviors.insert(
            bhav.id,
            Behavior {
                format: bhav.format,
                kind: bhav.kind,
                args: bhav.args,
                locals: bhav.locals,
                revision: bhav.revision,
                instructions: bhav.instructions.clone(),
            },
        );
        lib.names.insert(bhav.id, bhav.label.clone());
        if let Some(bcons) = &ctx.local_bcons {
            lib.constants = bcons.clone();
        }
    }
    let mut words = [0i16; DEFINITION_WORDS];
    words[9] = 4;
    words[14] = GUID as u16 as i16;
    words[15] = (GUID >> 16) as u16 as i16;
    let own = libraries.get_mut(&1)?;
    own.definitions.insert(DEFINITION, words);
    own.definition_order.push(DEFINITION);
    Some(Program {
        global: 0,
        libraries,
        semi_globals: BTreeMap::from([(1, (resource == 2).then_some(2))]),
        careers: None,
        career_texts_end: true,
        labels_hold_no_token: true,
        zoning: BTreeMap::new(),
        animations: Default::default(),
        people: Default::default(),
        icon_frames: None,
        wardrobe: None,
        ordered_wardrobe: None,
        character_files: Default::default(),
        expansions: 0,
        characters: Default::default(),
    })
}

fn catalog() -> Catalog {
    Catalog {
        global: 0,
        objects: vec![sims_content::Object {
            guid: GUID,
            resource: 1,
            chunk: DEFINITION,
            kind: 4,
            master: 0,
        }],
        by_guid: BTreeMap::from([(GUID, 0)]),
        semi_globals: BTreeMap::new(),
        diagnostics: Vec::new(),
    }
}

fn world(w: &Witness, attribute_count: usize) -> World {
    let set = |len: usize, values: &BTreeMap<i16, i16>| {
        let mut out = vec![0i16; len];
        for (&i, &v) in values {
            if let Some(slot) = usize::try_from(i).ok().and_then(|i| out.get_mut(i)) {
                *slot = v;
            }
        }
        out
    };
    let object = Object {
        id: ME,
        guid: GUID,
        footprint: [0; 4],
        position: [0; 3],
        unknown_short: 0,
        attributes: set(attribute_count, &w.attrs),
        temps: set(TEMPS, &w.temps),
        data: vec![0; 72],
        obsolete_word: 0,
        stack_capacity_bytes: 0,
        stack: Vec::new(),
        relationship_flag: -1,
        relationships: Vec::new(),
        slots: vec![SavedSlot {
            unknown: 0,
            object: 0,
        }],
        dynamic_sprites: Vec::new(),
        multipart: None,
        portal: None,
        person: None,
        registry_word: None,
        last_result: false,
        removal_skips_resets: false,
        selected: false,
    };
    let mut storage = FrameStorage::default();
    if let Some(&last) = w.frame_words.keys().next_back() {
        // The outermost frame starts at word 0; its header is eight words.
        storage.words = vec![Some(0); 8 + usize::from(last) + 1];
        for (&k, &v) in &w.frame_words {
            storage.words[8 + usize::from(k)] = Some(v);
        }
    }
    World {
        globals: vec![0; 64],
        order: vec![ME],
        types: BTreeMap::from([(TYPE, GUID)]),
        objects: BTreeMap::from([(ME, object)]).into(),
        frame_storage: BTreeMap::from([(ME, storage)]).into(),
        ..World::empty()
    }
}

/// One run's end: what it returned and the world it left.
struct Run {
    returned: bool,
    world: World,
}

fn run(bhav: &BhavChunk, ctx: &LiveContext, w: &Witness, attributes: usize) -> Option<Run> {
    let program = program(bhav, ctx)?;
    let catalog = catalog();
    let mut world = world(w, attributes);
    let vm = Interpreter::new(&program, &catalog, Stream::Lot);
    let returned = vm
        .run_tree(&mut world, ME, bhav.id, ME as i16, &w.params, BUDGET)
        .ok()?;
    Some(Run { returned, world })
}

/// What a caller and later code can see of a run, under the observation policy.
#[derive(Debug, PartialEq, Eq)]
struct Observed {
    returned: bool,
    temps: Vec<i16>,
    attributes: Vec<i16>,
    data: Vec<i16>,
    globals: Vec<i16>,
    last_result: Option<bool>,
    /// Frame-storage words past the outermost frame's header, trailing zeros trimmed
    /// (words never written read as zero). The header is rewritten by the next
    /// outermost frame, so nothing reads it back.
    residue: Option<Vec<Option<i16>>>,
    scratch: Option<String>,
    /// Everything else in the lot.
    rest: String,
}

fn observe(run: Run, opts: &LiveOptions) -> Observed {
    let mut world = run.world;
    let me = world.objects.get(&ME).cloned();
    let (temps, attributes, data, last_result) = me.map_or_else(
        || (Vec::new(), Vec::new(), Vec::new(), false),
        |o| (o.temps, o.attributes, o.data, o.last_result),
    );
    let residue = world.frame_storage.get(&ME).map(|s| {
        let mut words: Vec<Option<i16>> = s.words.iter().skip(8).copied().collect();
        while words.last() == Some(&Some(0)) {
            words.pop();
        }
        words
    });
    let scratch = format!("{:?}", world.scratch_ptrs);
    let globals = std::mem::take(&mut world.globals);
    if let Some(o) = world.objects.get_mut(&ME) {
        o.temps.clear();
        o.attributes.clear();
        o.data.clear();
        o.last_result = false;
    }
    world.frame_storage = BTreeMap::new().into();
    world.scratch_ptrs = ScratchPtrs::default();
    Observed {
        returned: run.returned,
        temps,
        attributes,
        data,
        globals,
        last_result: opts.observe_last_result.then_some(last_result),
        residue: if opts.observe_frame_residue {
            residue
        } else {
            None
        },
        scratch: opts.observe_scratch_pointer.then_some(scratch),
        rest: format!("{world:?}"),
    }
}

impl Observed {
    fn difference(&self, other: &Self) -> Option<String> {
        fn first(name: &str, a: &[i16], b: &[i16]) -> Option<String> {
            (a != b).then(|| match a.iter().zip(b).position(|(x, y)| x != y) {
                Some(i) => format!("{name}[{i}] ends as {} on A but {} on B", a[i], b[i]),
                None => format!("{name} differ in length"),
            })
        }
        if self.returned != other.returned {
            return Some(format!(
                "A returns {} but B returns {}",
                self.returned, other.returned
            ));
        }
        first("temp", &self.temps, &other.temps)
            .or_else(|| first("my.attr", &self.attributes, &other.attributes))
            .or_else(|| first("my.data", &self.data, &other.data))
            .or_else(|| first("global", &self.globals, &other.globals))
            .or_else(|| {
                (self.last_result != other.last_result)
                    .then(|| "the last branch results differ".to_owned())
            })
            .or_else(|| {
                (self.residue != other.residue).then(|| {
                    "the frame storage left behind differs (later frames read it as their \
                     initial locals)"
                        .to_owned()
                })
            })
            .or_else(|| {
                (self.scratch != other.scratch)
                    .then(|| "the scratch-pointer state differs".to_owned())
            })
            .or_else(|| (self.rest != other.rest).then(|| "other lot state differs".to_owned()))
    }
}
