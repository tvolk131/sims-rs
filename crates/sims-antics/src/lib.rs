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
pub mod ir;
pub mod lexer;
pub mod lift;
pub mod lower;
pub mod manifest;
pub mod parser;

pub use compile::{
    CompiledModule, CompiledObject, CompiledPack, CompiledPackObject, compile_antics,
    compile_object_dir, compile_object_from_dir, compile_object_from_sources, compile_pack_dir,
    package_far,
};
pub use error::{Error, Result, Span};
