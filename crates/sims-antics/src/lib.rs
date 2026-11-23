//! Clean-room SimAntics compiler, `.air` (Antics IR) layer, `.antics` v2 high-level
//! authoring language, IFF/FAR binary writer, procedural isometric sprite generator,
//! and synthesized PCM WAV generator for `sims-rs`.
//!
//! Architectural layers:
//! - **`.antics` v2 ([`v2`]):** High-level Rust-flavored object authoring language
//!   (`use`, `pub fn`, `pub template`, `object`, `with lock`, `defer`, `repeat cancel_on_abort`).
//! - **`.air` (Antics IR):** Explicit low-level textual IR ([`lexer`], [`parser`], [`ast`],
//!   [`lower`], [`lift`], [`structure_lift`], [`verify`]) with 100% lossless round-trip and
//!   formal trace-equivalence proofs.
//!
//! See `docs/decisions/20261122-antics-v2-language-and-air-ir.md` and
//! `docs/decisions/20261122-sims-antics-compiler-and-open-pack.md`.

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
pub mod v2;
pub mod verify;

pub use compile::{
    CompiledModule, CompiledObject, CompiledPack, CompiledPackObject, compile_air, compile_antics,
    compile_object_dir, compile_object_from_dir, compile_object_from_sources, compile_pack_dir,
    package_far,
};
pub use error::{Error, Result, Span};
pub use lift::{
    format_bhav_air, format_bhav_antics, format_bhav_labeled_air,
    format_bhav_labeled_air_with_arities, format_bhav_labeled_air_with_context,
    format_bhav_raw_air,
};
pub use sim_pack::{StandalonePack, compile_standalone_pack_dir};
pub use structure_lift::{
    LiftOptions, LiftSymbols, StructuredLiftOutcome, extract_lift_symbols_from_iff,
    lift_bhav_air_structured, lift_bhav_air_structured_with_options, lift_bhav_structured,
    lift_bhav_structured_with_arities, lift_bhav_structured_with_context,
    lift_bhav_structured_with_options,
};
pub use v2::{
    ExpandedV2Object, PackResolverConfig, compile_v2_object_file, compile_v2_pack_dir,
    compile_v2_source, expand_v2_to_air,
};
pub use verify::{
    EquivOptions, EquivResult, PackSafetyReport, SafetyCategory, SafetyReport, SafetyViolation,
    TraceStep, check_bhav_equivalent, check_bhav_equivalent_with_arities, verify_iff_safety,
    verify_object_safety, verify_pack_safety,
};
