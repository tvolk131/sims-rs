use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use sims_content::{Index, audit};
use sims_formats::sprite::{self, Channel};
use std::{fs, path::PathBuf};
#[path = "../save_cli.rs"]
mod save_cli;
#[path = "../vm_cli.rs"]
mod vm_cli;

#[derive(Parser)]
#[command(about = "Read-only Sims 1 content inspector", version)]
struct Args {
    #[arg(long, global = true, default_value = "game-data/cabinet-extracted")]
    root: PathBuf,
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Inventory original houses and neighborhoods without changing the source files.
    Saves {
        #[arg(long)]
        resource: Option<String>,
        #[arg(long, default_value = "cache/original-save-inventory.json")]
        out: PathBuf,
        /// Include decoded arrays and raw object records in the private report.
        #[arg(long)]
        full: bool,
    },
    /// Headless SimAntics dependency analysis, chair trace, and snapshot resume.
    Vm {
        #[command(subcommand)]
        command: vm_cli::Command,
    },
    /// Find resources; full locators disambiguate duplicate names.
    List {
        #[arg(default_value = "")]
        query: String,
        #[arg(long, default_value_t = 40)]
        limit: usize,
    },
    /// Inspect an IFF's objects, drawing groups, menus, and behavior labels.
    Inspect { resource: String },
    /// Disassemble a BHAV; branches and raw operands are retained.
    Disassemble {
        resource: String,
        #[arg(long)]
        bhav: u16,
    },
    /// Export all views and color/alpha/depth channels of one drawgroup.
    Render {
        resource: String,
        #[arg(long)]
        group: Option<u16>,
        #[arg(long)]
        object: Option<u16>,
        #[arg(long, default_value = "extracted/chair-preview")]
        out: PathBuf,
    },
    /// Check every IFF and known chunk parser; write a machine-readable report.
    Audit {
        #[arg(long)]
        decode_sprites: bool,
        #[arg(long, default_value = "cache/asset-audit.json")]
        out: PathBuf,
    },
}
fn main() -> Result<()> {
    let args = Args::parse();
    let index = Index::scan(&args.root)?;
    for diagnostic in &index.diagnostics {
        eprintln!("Index warning: {diagnostic}");
    }
    match args.command {
        Command::Saves {
            resource,
            out,
            full,
        } => save_cli::run(&index, resource.as_deref(), &out, full)?,
        Command::Vm { command } => vm_cli::run(&index, command)?,
        Command::List { query, limit } => {
            let query = query.to_lowercase();
            let found: Vec<_> = index
                .resources
                .iter()
                .filter(|r| r.locator.to_lowercase().contains(&query))
                .collect();
            println!(
                "{} resources · {} archives · {} matches",
                index.resources.len(),
                index.archives,
                found.len()
            );
            for r in found.iter().take(limit) {
                println!("{:>10}  {}", r.size, r.locator);
            }
        }
        Command::Audit {
            decode_sprites,
            out,
        } => {
            let report = audit(&index, decode_sprites);
            if let Some(p) = out.parent() {
                fs::create_dir_all(p)?;
            }
            fs::write(&out, serde_json::to_vec_pretty(&report)?)?;
            println!(
                "{} IFF files · {} decoded sprite frames · {} diagnostics\n{}",
                report.iff_files,
                report.decoded_frames,
                report.failures.len(),
                out.display()
            );
            for e in report.failures.iter().take(12) {
                eprintln!("{e}");
            }
            if !report.failures.is_empty() {
                bail!(
                    "audit found {} unsupported or invalid resources; see report",
                    report.failures.len()
                );
            }
        }
        command => {
            let selector = match &command {
                Command::Inspect { resource }
                | Command::Disassemble { resource, .. }
                | Command::Render { resource, .. } => resource,
                _ => unreachable!(),
            };
            let doc = index.document(index.find(selector)?)?;
            match command {
                Command::Inspect { .. } => {
                    println!("{}\n{} chunks\n", doc.source, doc.iff.chunks.len());
                    for o in &doc.objects {
                        println!(
                            "Object #{}: {} · GUID {:08X} · §{} · DGRP {} · TTAB {}",
                            o.id,
                            doc.object_title(o),
                            o.value.guid,
                            o.value.price,
                            o.value.graphic,
                            o.value.table
                        );
                    }
                    for g in &doc.groups {
                        println!("DGRP #{}: {} · {} views", g.id, g.name, g.value.views.len());
                    }
                    for t in &doc.tables {
                        println!("\n{}", doc.menu_text(t));
                    }
                    for b in &doc.behaviors {
                        println!(
                            "BHAV #{}: {} · {} instructions",
                            b.id,
                            b.name,
                            b.value.instructions.len()
                        );
                    }
                    for e in &doc.diagnostics {
                        eprintln!("{e}");
                    }
                }
                Command::Disassemble { bhav, .. } => {
                    let b = doc
                        .behaviors
                        .iter()
                        .find(|b| b.id == bhav)
                        .context("BHAV not found")?;
                    println!(
                        "{} :: BHAV #{} {}\n{}",
                        doc.source,
                        b.id,
                        b.name,
                        b.value.disassemble(|id| doc.behavior_name(id))
                    );
                }
                Command::Render {
                    group, object, out, ..
                } => {
                    let group_id = match (group, object) {
                        (Some(id), _) => id,
                        (None, Some(id)) => {
                            doc.objects
                                .iter()
                                .find(|o| o.id == id)
                                .context("object not found")?
                                .value
                                .graphic
                        }
                        (None, None) => doc
                            .objects
                            .first()
                            .map(|o| o.value.graphic)
                            .or_else(|| doc.groups.first().map(|g| g.id))
                            .context("no drawing groups")?,
                    };
                    let group = doc
                        .groups
                        .iter()
                        .find(|g| g.id == group_id)
                        .context("drawing group not found")?;
                    fs::create_dir_all(&out)?;
                    for (n, view) in group.value.views.iter().enumerate() {
                        let preview = sprite::render(&doc.iff, view)
                            .with_context(|| format!("DGRP #{group_id} view {n}"))?;
                        for channel in [Channel::Color, Channel::Alpha, Channel::Depth] {
                            let p = out.join(format!(
                                "dgrp{group_id}-view{n}-dir{}-zoom{}-{}.png",
                                view.direction,
                                view.zoom,
                                channel.to_string().to_lowercase()
                            ));
                            image::save_buffer(
                                &p,
                                &preview.channel(channel, false),
                                preview.width,
                                preview.height,
                                image::ColorType::Rgba8,
                            )?;
                        }
                        println!(
                            "view {n}: direction {} · zoom {} · {}×{} · {} layers",
                            view.direction,
                            view.zoom,
                            preview.width,
                            preview.height,
                            view.sprites.len()
                        );
                        for note in preview.notes {
                            eprintln!("{note}");
                        }
                    }
                    println!("Exported to {}", out.display());
                }
                _ => unreachable!(),
            }
        }
    }
    Ok(())
}
