//! Clean-room SimAntics compiler, IFF/FAR binary writer, procedural isometric
//! sprite generator, and synthesized PCM WAV generator for `sims-rs`.
//!
//! See `docs/decisions/20261122-sims-antics-compiler-and-open-pack.md` for the
//! clean-room architecture and legal boundary.

pub mod art;
pub mod ast;
pub mod audio;
pub mod compile;
pub mod encode;
pub mod error;
pub mod house;
pub mod ir;
pub mod lexer;
pub mod lift;
pub mod lower;
pub mod manifest;
pub mod parser;
pub mod sim_pack;
pub mod structure_lift;
pub mod verify;

pub use compile::{
    CompiledModule, CompiledObject, CompiledPack, CompiledPackObject, compile_antics,
    compile_object_dir, compile_object_from_dir, compile_object_from_sources, compile_pack_dir,
    package_far,
};
pub use error::{Error, Result, Span};
pub use sim_pack::{StandalonePack, compile_standalone_pack_dir};
pub use structure_lift::{StructuredLiftOutcome, lift_bhav_structured};
pub use verify::{
    EquivOptions, EquivResult, PackSafetyReport, SafetyCategory, SafetyReport, SafetyViolation,
    TraceStep, check_bhav_equivalent, verify_iff_safety, verify_object_safety, verify_pack_safety,
};
