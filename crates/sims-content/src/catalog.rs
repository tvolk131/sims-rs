//! Controller definition records and production module linking.
//! Historical extraction-order scans live in tests/support/catalog.rs;
//! live loading resolves definitions through object_catalog::Catalog.
use crate::{Index, saves::DefinitionSource};
use anyhow::{Result, ensure};
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Debug, Clone, Serialize)]
pub struct Controller {
    pub guid: u32,
    pub label: String,
    pub definition: DefinitionSource,
}

/// Merge controller modules and their semiglobals into a linked house program.
/// This does not construct instances or advance any ordinary saved stack.
pub fn link_controllers(
    index: &Index,
    program: &mut sims_vm::Program,
    catalog: &[Controller],
) -> Result<()> {
    let sources: BTreeSet<_> = catalog.iter().map(|c| &c.definition.source).collect();
    for source in sources {
        if program.modules.contains_key(source) {
            continue;
        }
        let loaded = crate::vm::load_program(index, source, "Global.iff")?;
        ensure!(
            program.globals == loaded.globals,
            "inconsistent globals catalog"
        );
        for (name, module) in loaded.modules {
            if let Some(old) = program.modules.get(&name) {
                ensure!(
                    old.content_hash == module.content_hash,
                    "inconsistent module {name}"
                );
            } else {
                program.modules.insert(name, module);
            }
        }
    }
    Ok(())
}
