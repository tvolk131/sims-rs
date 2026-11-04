use anyhow::{Context, Result, ensure};
use sims_content::{Index, saves::House};
use sims_sim::house_valuation;
use std::path::Path;

#[test]
#[ignore = "private three-house assets and read-only original paused catalog capture"]
fn installed_prices_match_original_catalog_and_value_all_three_homes() -> Result<()> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let output = repo.join(std::env::var("SIMS_VALUATION_OUTPUT")?);
    ensure!(output.starts_with(repo.join("cache")));
    std::fs::create_dir(&output)?;
    let index = Index::scan(&repo.join("game-data/cabinet-extracted"))?;
    let capture = repo.join("cache/original-valuation-catalog-20261022-correct-tick.json");
    let original: serde_json::Value = serde_json::from_slice(&std::fs::read(&capture)?)?;
    ensure!(original["paused"] == true && original["tick"] == 196247);
    let mut report = vec![];
    for (name, source) in [
        ("newbie", "UserData/Houses/House07.iff"),
        ("goth", "UserData/Houses/House05.iff"),
        ("mashuga", "UserData2/Houses/House09.iff"),
    ] {
        let h = House::load(&index, source)?;
        let size = match h.simulator.globals.get(23).copied() {
            Some(0) => 64,
            Some(n) if n > 3 => n as usize,
            _ => h.width,
        };
        let pool = h.pool_cells()?;
        let c = sims_content::house_valuation::load(&index, &h, size, h.width, &h.floors, pool)?;
        c.validate(&index)?;
        ensure!(
            serde_json::to_value(&c.prices.styles)? == original["style_prices"],
            "style price table differs"
        );
        ensure!(c.prices.pool == original["pool_price"].as_i64().unwrap() as i32);
        let mut builtin = 0;
        for (kind, values) in [("floors", &c.prices.floors), ("walls", &c.prices.walls)] {
            for (&id, &price) in values {
                if kind == "floors" && !(1..30).contains(&id)
                    || kind == "walls" && !(id <= 30 || id >= 248 && id <= 255)
                {
                    continue;
                }
                let row = original[kind]["rows"]
                    .as_array()
                    .context("original price rows absent")?
                    .iter()
                    .find(|r| r["id"] == id)
                    .context("original built-in price absent")?;
                ensure!(row["price"] == price, "{name} {kind} {id} price differs");
                builtin += 1;
            }
        }
        let valuation =
            house_valuation::architecture(size, h.width, &h.floors, pool, &c.prices, |_, _| {})
                .map_err(anyhow::Error::msg)?;
        std::fs::write(
            output.join(format!("{name}-inputs.json")),
            serde_json::to_vec(
                &serde_json::json!({"source":source,"size":size,"stride":h.width,"floors":h.floors,"pool":pool,"catalog":c,"rust":valuation}),
            )?,
        )?;
        report.push(serde_json::json!({"household":name,"builtin_prices_matched":builtin,"all_256_style_prices_matched":true,"pool_price_matched":true,
            "architecture_rust_only":valuation,"original_house_value_comparison":false,"original_parity_ticks_added":0}));
    }
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(())
}
