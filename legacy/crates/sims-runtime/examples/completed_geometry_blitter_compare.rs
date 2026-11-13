//! Original-call component oracle, not an enclosing Rust Session/tick replay.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sims_sim::lot_presenter::{dual_sprite, sprite_full};
use std::{
    io::{BufRead, BufReader, Write},
    path::PathBuf,
};

fn bytes(v: &Value) -> Result<Vec<u8>> {
    let s = v.as_str().context("hex bytes absent")?;
    ensure!(s.len() % 2 == 0, "odd hex bytes");
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(Into::into))
        .collect()
}
fn word(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
fn read(v: &Value, label: &str, address: u32) -> Result<Vec<u8>> {
    let r = v["reads"]
        .as_array()
        .context("B spans absent")?
        .iter()
        .find(|r| r["label"] == label && r["address"] == address)
        .context("B span absent")?;
    let b = bytes(&r["raw"])?;
    ensure!(
        sims_fixtures::digest(&b) == r["sha256"].as_str().unwrap()
            && b.len() == r["bytes"].as_u64().unwrap() as usize,
        "B span hash/extent differs"
    );
    Ok(b)
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Saved {
    plan: dual_sprite::Plan,
    next: usize,
    color: Vec<u8>,
    depth: Vec<u8>,
}
impl Saved {
    fn advance(&mut self) -> Result<bool> {
        let Some(w) = self.plan.writes.get(self.next) else {
            return Ok(false);
        };
        sprite_full::apply(w, &mut self.color, &mut self.depth).map_err(anyhow::Error::msg)?;
        self.next += 1;
        Ok(true)
    }
    fn finish(&mut self) -> Result<()> {
        while self.advance()? {}
        Ok(())
    }
}
fn main() -> Result<()> {
    let root = std::env::var_os("SIMS_DIAG_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let data = root.join("fixtures/private/goth-geometry-visible-20261101-first");
    let material_raw = std::fs::read(data.join("material-source-third.json"))?;
    let raster_raw = std::fs::read(data.join("raster-source-first.json"))?;
    ensure!(
        sims_fixtures::digest(&material_raw)
            == "c7d259decfb58e83a2c71f3851c4abfe52203f7518ec582a6c91d102b9cf94e8"
            && sims_fixtures::digest(&raster_raw)
                == "d545c1601818be0c588d9a6d49dac0e265d74b4f2bb4d2ad870d40e3af770693",
        "B sources differ"
    );
    let material: Value = serde_json::from_slice(&material_raw)?;
    let raster: Value = serde_json::from_slice(&raster_raw)?;
    ensure!(
        raster["tick"] == 89073 && raster["executes_target_code"] == false,
        "B raster epoch differs"
    );
    let native =
        std::fs::read(std::env::var_os("SIMS_DIAG_NATIVE").context("native trace absent")?)?;
    let native_pin =
        std::env::var("SIMS_DIAG_NATIVE_SHA256").context("qualified native trace pin absent")?;
    ensure!(
        native_pin == "00d73e4fa9f1dcaf978f69a7ed28f9f04e06f2b23e6dd445b010850349d4a241"
            && sims_fixtures::digest(&native) == native_pin,
        "native trace differs"
    );
    let rows: Vec<Value> = BufReader::new(flate2::read::GzDecoder::new(native.as_slice()))
        .lines()
        .map(|l| Ok(serde_json::from_str(&l?)?))
        .collect::<Result<_>>()?;
    let calls: Vec<_> = rows
        .iter()
        .filter(|q| {
            q["kind"] == "world_drawing_setup" && q["ordinal"] == 1 && q["pc"] == 0x4c4907u32
        })
        .collect();
    let returns: Vec<_> = rows
        .iter()
        .filter(|q| {
            q["kind"] == "world_drawing_setup" && q["ordinal"] == 1 && q["pc"] == 0x4c7f7au32
        })
        .collect();
    ensure!(
        calls.len() == 2 && returns.len() == 2,
        "actual ordinary call count differs"
    );
    let resource = |image: u32| {
        material["resources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["image"] == image)
            .unwrap()
    };
    let mask = bytes(&resource(26)["frame_prefix"])?;
    let color = bytes(&resource(1786)["frame_prefix"])?;
    let store = sims_fixtures::Store::create(
        std::env::var_os("SIMS_DIAG_OUTPUT_STORE").context("output store absent")?,
    )?;
    let mut reports = vec![];
    for (ordinal, (q, ret)) in calls.iter().zip(&returns).enumerate() {
        let e = &q["visible_geometry"];
        let out = &ret["visible_geometry"];
        let owners = &e["raster_owners"];
        let stack = bytes(&e["stack"]["raw"])?;
        let a: [u32; 15] = std::array::from_fn(|i| word(&stack, i * 4));
        let context = bytes(&q["context"]["raw"])?;
        let depth_owner = raster["depth_owner"].as_u64().unwrap() as u32;
        ensure!(
            word(&context, 0x44) == depth_owner && owners["depth_owner"] == depth_owner,
            "depth receiver differs"
        );
        let depth = read(&raster, "depth_owner", depth_owner)?;
        ensure!(
            bytes(&owners["depth_raw"])? == depth,
            "live depth owner differs from B"
        );
        let zoom = word(&context, 0x20);
        ensure!((1..=3).contains(&zoom), "unsupported native zoom");
        let curve = word(&depth, 4 + (3 - zoom) as usize * 4);
        let columns = word(&depth, (14 - zoom) as usize * 4);
        let curve_raw = read(&raster, "row_curve", curve - 256)?;
        let column_raw = read(&raster, "column_depth", columns - 256)?;
        ensure!(
            owners["curve"] == curve && owners["columns"] == columns,
            "live depth table aliases differ from B"
        );
        let live_curve = bytes(&owners["curve_raw"])?;
        let live_columns = bytes(&owners["column_raw"])?;
        let resource_raw = bytes(&resource(1786)["raw"])?;
        let palette_address = word(&resource_raw, 8) + 0x420;
        let palette = read(&raster, "palette_16bit", palette_address)?;
        ensure!(
            owners["palette"] == palette_address && bytes(&owners["palette_raw"])? == palette,
            "live wallpaper palette differs from B"
        );
        let frame = bytes(&e["renderer_frame"]["raw"])?;
        let bp = e["renderer_frame"]["bp"].as_u64().unwrap() as u32;
        let local = |p: u32| usize::try_from(p.checked_sub(bp - 0xf0).unwrap()).unwrap();
        ensure!(
            word(&frame, local(a[5]) + 4) == resource(1786)["frame"].as_u64().unwrap() as u32 + 10
                && word(&frame, local(a[6]) + 4)
                    == resource(26)["frame"].as_u64().unwrap() as u32 + 10,
            "native color/coverage frame ownership differs"
        );
        let margins = std::array::from_fn(|i| word(&frame, local(a[11]) + 4 * i) as i32);
        let width = a[3] as usize;
        let height = a[4] as usize;
        let z: Vec<_> = (0..height)
            .map(|y| {
                let index = (word(&context, 0x24) as i32)
                    .wrapping_add(8i32.wrapping_shl(zoom))
                    .wrapping_sub((a[2] as i32).wrapping_add(y as i32));
                let at = usize::try_from(i64::from(index) * 2 + 256)?;
                ensure!(
                    curve_raw.get(at..at + 2) == live_curve.get(at..at + 2),
                    "consumed live row differs from B"
                );
                let sample = i16::from_le_bytes(
                    curve_raw
                        .get(at..at + 2)
                        .context("row depth outside retained span")?
                        .try_into()?,
                );
                Ok(i32::from(sample)
                    .wrapping_add(word(&context, 0x28) as i32)
                    .wrapping_add(a[14] as i32))
            })
            .collect::<Result<_>>()?;
        let column_offset = usize::try_from(
            a[12]
                .checked_sub(columns - 256)
                .context("column pointer precedes retained span")?,
        )?;
        ensure!(
            column_raw.get(column_offset..column_offset + width * 2)
                == live_columns.get(column_offset..column_offset + width * 2),
            "consumed live column differs from B"
        );
        let columns: Vec<u16> = (0..width)
            .map(|x| {
                u16::from_le_bytes(
                    column_raw[column_offset + 2 * x..column_offset + 2 * x + 2]
                        .try_into()
                        .unwrap(),
                )
            })
            .collect();
        let input_color = bytes(&e["buffers"][0]["raw"])?;
        let input_depth = bytes(&e["buffers"][1]["raw"])?;
        let mask_scratch = bytes(&e["scratch"]["mask"])?;
        let color_scratch = bytes(&e["scratch"]["color"])?;
        let plan = dual_sprite::plan(dual_sprite::Input {
            mask: &mask[..634],
            color: &color[..872],
            palette: &palette,
            screen: [a[1] as i32, a[2] as i32],
            margins,
            color_pitch: a[8] as i32,
            depth_pitch: a[10] as i32,
            rows: &z,
            columns: &columns,
            mask_scratch: &mask_scratch,
            color_scratch: &color_scratch,
            depth: &input_depth,
        })
        .map_err(anyhow::Error::msg)?;
        let mut saved = Saved {
            plan,
            next: 0,
            color: input_color,
            depth: input_depth,
        };
        let mut expected = saved.clone();
        expected.finish()?;
        ensure!(
            expected.color == bytes(&out["buffers"][0]["raw"])?
                && expected.depth == bytes(&out["buffers"][1]["raw"])?
                && expected.plan.mask_scratch == bytes(&out["scratch"]["mask"])?
                && expected.plan.color_scratch == bytes(&out["scratch"]["color"])?,
            "ordinary dual-sprite outputs differ at call{ordinal}"
        );
        let n = saved.plan.writes.len();
        let mut cuts = vec![0, 1, 2, n / 2, n.saturating_sub(1), n];
        cuts.sort();
        cuts.dedup();
        let mut cold = vec![];
        for cut in cuts {
            while saved.next < cut {
                saved.advance()?;
            }
            let blob = store.put_value(&saved)?;
            let mut restored: Saved = store.get_value(&blob)?;
            ensure!(restored == saved, "blitter cold restore differs");
            restored.finish()?;
            ensure!(restored == expected, "blitter cold continuation differs");
            cold.push(json!({"next_write":cut,"blob":blob}));
        }
        reports.push(json!({"call":ordinal,"writes":n,"color_sha256":sims_fixtures::digest(&expected.color),"depth_sha256":sims_fixtures::digest(&expected.depth),"scratch_rows_equal":true,"cold_cuts":cold}));
    }
    let report = json!({"schema":1,"success":true,"native_trace_sha256":native_pin,"baseline_raster_sha256":sims_fixtures::digest(&raster_raw),"calls":reports,"observed_call_arguments":true,"component_only":true,"enclosing_Rust_Session_replay":false,"source_admission":false,"native_parity_credit":0,"scope":"Original4c4907 ordinary dual-sprite component: both complete color/depth planes, scratch rows and suspended ordered stores. Caller/context/cache evolution, C4 lifecycle and completed ticks require enclosing Rust continuation."});
    std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(std::env::var_os("SIMS_DIAG_REPORT").context("report absent")?)?
        .write_all(&serde_json::to_vec_pretty(&report)?)?;
    println!(
        "{}",
        json!({"success":true,"calls":2,"writes":reports.iter().map(|q|q["writes"].as_u64().unwrap()).sum::<u64>()})
    );
    Ok(())
}
