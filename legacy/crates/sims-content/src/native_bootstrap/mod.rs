//! Real host services for the recovered constructors. These services do not
//! create the missing application/static inputs or admit a playable World.
//!
//! Logical addresses belong to this heap's new address space, never to a host
//! pointer or a captured process. Keep one heap with its constructor graph;
//! neither an address nor a byte snapshot authenticates a different epoch.
pub mod files;
pub mod executable;
pub mod locks;
pub mod memory;
pub mod resources;
pub mod services;

pub use memory::Memory;
pub use resources::{ArrayHost, ResourceSource};
pub use services::{BootstrapServices, LocalePolicy};

#[cfg(test)]
mod tests;
