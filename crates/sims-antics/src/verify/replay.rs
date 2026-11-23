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
//! everything else zero. Each side's program is the one the VM builds for an object
//! whose file holds the tree: the tree and the file's `BCON` tables in the object's own
//! library, an empty global library and no semi-global one. So only private trees
//! (4096 to 8191) are replayed, and only those made of `Expression` nodes: a primitive
//! or a call could reach code or state the replay world does not have.
//!
//! A run counts only if it returns: a fault (any VM trap, including running out of
//! the instruction budget) means the witness shows nothing, so the result stays
//! undecided. Two runs differ only in the plain words the replay defines and the VM
//! reads back as stored (see `Observed`); a difference anywhere else in the world is
//! not one this replay vouches for.
//!
//! The replay needs `sims-vm`, so it is behind the `vm-replay` feature (on by default).
//! Without it nothing is replayed, and every candidate difference stays undecided.

use super::LiveOptions;
use super::live::LiveContext;
use crate::ir::BhavChunk;
use std::collections::{BTreeMap, BTreeSet};
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
    if !cfg!(feature = "vm-replay") || len == 0 || !super::iff::runs_from_own_file(bhav.id) {
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
#[cfg(feature = "vm-replay")]
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
    let ra = vm::run(a, ctx_a, w, attributes)?;
    let rb = vm::run(b, ctx_b, w, attributes)?;
    let (oa, ob) = (vm::observe(ra, opts), vm::observe(rb, opts));
    let what = oa.difference(&ob)?;
    Some(format!("sims-vm runs both from {w}: {what}"))
}

/// Without the `vm-replay` feature nothing is replayed ([`replayable`] is false), so no
/// difference is ever confirmed.
#[cfg(not(feature = "vm-replay"))]
pub(crate) fn replay(
    _: &BhavChunk,
    _: &LiveContext,
    _: &BhavChunk,
    _: &LiveContext,
    _: &LiveOptions,
    _: &Witness,
) -> Option<String> {
    None
}

#[cfg(feature = "vm-replay")]
mod vm {
    use super::{BhavChunk, LiveContext, LiveOptions, Witness};
    use sims_content::Catalog;
    use sims_formats::behavior::Behavior;
    use sims_formats::saved_objects::Slot as SavedSlot;
    use sims_sim::{FrameStorage, Object, World};
    use sims_vm::{DEFINITION_WORDS, Interpreter, Library, Program, Stream};
    use std::collections::BTreeMap;
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
    /// The frame-storage word the replayed frame's parameters start at: the call marker
    /// and the frame header, eight words each.
    const FRAME_START: usize = 16;

    /// The program the VM would build for the replay object: its own file (resource 1)
    /// holds the tree and the file's `BCON`s, as an object's file does for a private tree
    /// (`program.rs:1015`); the global library (0) is empty and there is no semi-global
    /// one. Only private trees are run from an object's file, so any other tree has no
    /// such program and is not replayed.
    fn program(bhav: &BhavChunk, ctx: &LiveContext) -> Option<Program> {
        if !super::super::iff::runs_from_own_file(bhav.id) {
            return None;
        }
        let mut libraries = BTreeMap::from([(0, Library::default()), (1, Library::default())]);
        {
            let lib = libraries.get_mut(&1)?;
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
            semi_globals: BTreeMap::from([(1, None)]),
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
            // `run_tree` runs the tree as a synchronous call: an eight-word call marker
            // comes first (`interpreter.rs:1171`), then the frame's eight-word header, then
            // its parameters and locals (`interpreter.rs:1217`).
            storage.words = vec![Some(0); FRAME_START + usize::from(last) + 1];
            for (&k, &v) in &w.frame_words {
                storage.words[FRAME_START + usize::from(k)] = Some(v);
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
    pub(super) struct Run {
        returned: bool,
        world: World,
    }

    pub(super) fn run(
        bhav: &BhavChunk,
        ctx: &LiveContext,
        w: &Witness,
        attributes: usize,
    ) -> Option<Run> {
        let program = program(bhav, ctx)?;
        let catalog = catalog();
        let mut world = world(w, attributes);
        let vm = Interpreter::new(&program, &catalog, Stream::Lot);
        let returned = vm
            .run_tree(&mut world, ME, bhav.id, ME as i16, &w.params, BUDGET)
            .ok()?;
        Some(Run { returned, world })
    }

    /// What a caller and later code can see of a run, field by field: the return value,
    /// the running object's temps, attributes and data words, the globals, and (when
    /// observed) the object's last branch result. These are plain words the VM reads back
    /// as stored (`sims-vm/src/memory.rs`). A difference anywhere else in the world (frame
    /// storage, scratch pointers, records the world keeps in another representation, such
    /// as advertisement deltas) is not one this replay can vouch for, so it never makes a
    /// counterexample.
    #[derive(Debug, PartialEq, Eq)]
    pub(super) struct Observed {
        returned: bool,
        temps: Vec<i16>,
        attributes: Vec<i16>,
        data: Vec<i16>,
        globals: Vec<i16>,
        last_result: Option<bool>,
    }

    pub(super) fn observe(run: Run, opts: &LiveOptions) -> Observed {
        let me = run.world.objects.get(&ME).cloned();
        let (temps, attributes, data, last_result) = me.map_or_else(
            || (Vec::new(), Vec::new(), Vec::new(), false),
            |o| (o.temps, o.attributes, o.data, o.last_result),
        );
        Observed {
            returned: run.returned,
            temps,
            attributes,
            data,
            globals: run.world.globals,
            last_result: opts.observe_last_result.then_some(last_result),
        }
    }

    impl Observed {
        pub(super) fn difference(&self, other: &Self) -> Option<String> {
            // Same-length words differ at an index; words of different lengths are a
            // representation question this replay does not decide.
            fn first(name: &str, a: &[i16], b: &[i16]) -> Option<String> {
                if a.len() != b.len() {
                    return None;
                }
                let i = a.iter().zip(b).position(|(x, y)| x != y)?;
                Some(format!(
                    "{name}[{i}] ends as {} on A but {} on B",
                    a[i], b[i]
                ))
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
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use sims_formats::behavior::Instruction;

        /// `temp[dst] = scope[idx]`.
        fn copy_to_temp(dst: i16, scope: u8, idx: i16, next: u8) -> Instruction {
            let [d0, d1] = dst.to_le_bytes();
            let [i0, i1] = idx.to_le_bytes();
            Instruction {
                opcode: 2,
                on_true: next,
                on_false: next,
                operands: [d0, d1, i0, i1, 0, 5, 8, scope],
            }
        }

        /// The replay reads back exactly the witness frame: parameters as passed, and each
        /// frame word as the local the witness names it for.
        #[test]
        fn replay_frame_reads_back_the_witness() {
            let bhav = BhavChunk {
                id: 4096,
                label: String::new(),
                format: 0x8002,
                kind: 0,
                args: 2,
                locals: 2,
                revision: 0,
                instructions: vec![
                    copy_to_temp(0, 25, 0, 1),
                    copy_to_temp(1, 25, 1, 2),
                    copy_to_temp(2, 9, 0, 3),
                    copy_to_temp(3, 9, 1, 0xFE),
                ],
                trailing: Vec::new(),
            };
            // Frame words are keyed `args + local`.
            let w = Witness {
                params: vec![5, -6],
                frame_words: BTreeMap::from([(2, 7), (3, -3)]),
                ..Witness::default()
            };
            let run = run(&bhav, &LiveContext::default(), &w, 8).expect("the run returns");
            assert!(run.returned);
            let temps = &run.world.objects[&ME].temps;
            assert_eq!(&temps[..4], &[7, -3, 5, -6]);
        }
    }
}
