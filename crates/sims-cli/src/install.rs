//! `sims install`: detect an install and summarize its indexed content.

use sims_content::{Catalog, Index, Install};
use std::fmt::Write as _;
use std::path::Path;

pub fn report(root: &Path) -> sims_content::Result<String> {
    let install = Install::detect(root)?;
    let index = Index::build(&install)?;
    let mut out = String::new();
    let _ = writeln!(out, "Install:    {}", install.root.display());
    let _ = writeln!(out, "Layout:     {:?}", install.layout);
    let _ = writeln!(
        out,
        "Expansions: {} (edition bits 0x{:02X})",
        install.expansions, install.expansions.0
    );
    let _ = writeln!(
        out,
        "\n{:<22} {:>9} {:>9} {:>9}",
        "Mount", "files", "members", "iff"
    );
    for (number, mount) in index.mounts.iter().enumerate() {
        let resources = index.resources.iter().filter(|r| r.mount == number);
        let (mut files, mut members, mut iffs) = (0, 0, 0);
        for resource in resources {
            match resource.member {
                Some(_) => members += 1,
                None => files += 1,
            }
            if resource.name().to_ascii_lowercase().ends_with(".iff") {
                iffs += 1;
            }
        }
        let path = if mount.recursive {
            format!("{}/**", mount.path)
        } else {
            format!("{}/*", mount.path)
        };
        let _ = writeln!(out, "{path:<22} {files:>9} {members:>9} {iffs:>9}");
    }
    let _ = writeln!(out, "\nResources: {}", index.resources.len());
    let catalog = Catalog::build(&index)?;
    let links = catalog
        .semi_globals
        .values()
        .filter(|l| l.is_some())
        .count();
    let _ = writeln!(
        out,
        "Objects:   {} definitions, {} GUIDs, {} semi-global links",
        catalog.objects.len(),
        catalog.by_guid.len(),
        links
    );
    let diagnostics: Vec<_> = index
        .diagnostics
        .iter()
        .chain(&catalog.diagnostics)
        .collect();
    if !diagnostics.is_empty() {
        let _ = writeln!(out, "\nDiagnostics ({}):", diagnostics.len());
        for diagnostic in diagnostics {
            let _ = writeln!(out, "  {diagnostic}");
        }
    }
    Ok(out)
}
