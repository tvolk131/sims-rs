//! Read-only cadence diagnostic, separate from frontend and input admission.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sims_fixtures::readset::Readset;
use sims_vm::scheduler::{
    clock::{Fields, Timer},
    transport::Timing,
};
use std::path::PathBuf;

fn bytes(v: &Value) -> Result<Vec<u8>> {
    let s = v.as_str().context("clock raw bytes absent")?;
    ensure!(
        s.is_ascii() && s.len() % 2 == 0,
        "invalid clock raw encoding"
    );
    (0..s.len())
        .step_by(2)
        .map(|i| Ok(u8::from_str_radix(&s[i..i + 2], 16)?))
        .collect()
}
fn word(b: &[u8], o: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        b.get(o..o + 4)
            .context("clock scalar outside owner")?
            .try_into()?,
    ))
}
#[test]
#[ignore = "private original 33-cut scalar clock cadence; not a draw schedule witness"]
fn original_completed_clock_counters_match_existing_timer_kernel() -> Result<()> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source = root.join(
        "fixtures/private/goth-completed-continuation-20261030-third/results/native-sequence",
    );
    let seal_raw = std::fs::read(source.join("capture-result.json"))?;
    ensure!(
        sims_fixtures::digest(&seal_raw)
            == "4139107d1965925bf3ee5f44fa524d5d4b1fae22bfdb617edfffb69d33bcd393",
        "clock source seal differs"
    );
    let seal: Value = serde_json::from_slice(&seal_raw)?;
    let mut rows = Vec::new();
    let mut previous: Option<Fields> = None;
    let mut samples = Vec::new();
    for index in 0..33 {
        let directory = source.join(format!("completed-{index:02}"));
        let encoded = std::fs::read(directory.join("observation.json.gz"))?;
        let hash = sims_fixtures::digest(&encoded);
        ensure!(
            seal["observations"][index]["sha256"] == hash,
            "clock observation differs"
        );
        let state: Value = serde_json::from_reader(flate2::read::GzDecoder::new(&encoded[..]))?;
        let raw = std::fs::read(directory.join("acknowledged.mem"))?;
        let memory = Readset::parse(&raw)?;
        ensure!(
            state["readset"]["sha256"] == memory.source_sha256() && state["tick"] == 89073 + index,
            "clock source epoch differs"
        );
        let d = &state["domains"]["controls"];
        for read in d["reads"].as_array().context("clock source reads absent")? {
            let b = bytes(&read["raw"])?;
            let address: u32 = read["address"]
                .as_u64()
                .context("clock read address absent")?
                .try_into()?;
            ensure!(
                read["bytes"] == b.len()
                    && read["sha256"] == sims_fixtures::digest(&b)
                    && memory.read(address, b.len())? == b,
                "clock source read differs"
            );
        }
        let simulator: u32 = d["simulator"]
            .as_u64()
            .context("clock simulator absent")?
            .try_into()?;
        let b = bytes(&d["elapsed_raw"])?;
        ensure!(
            b.len() == 41 && memory.read(simulator + 0xb8, 41)? == b,
            "clock retained owner differs"
        );
        let now = word(&bytes(&d["now_raw"])?, 0)?;
        ensure!(
            word(&memory.read(0x6e54e0, 4)?, 0)? == now
                && bytes(&d["stopping_raw"])? == [0]
                && state["snapshot"]["globals"][11] == 0
                && d["requests"] == json!([]),
            "cadence has a pending stop/step/request owner"
        );
        let header = bytes(&d["requests_raw"])?;
        let head = word(&header, 0)?;
        let sentinel = bytes(&d["sentinel_raw"])?;
        ensure!(
            word(&header, 4)? == 0
                && sentinel.len() == 8
                && word(&sentinel, 0)? == head
                && word(&sentinel, 4)? == head
                && memory.read(head, 8)? == sentinel,
            "clock request sentinel differs"
        );
        let fields = Fields {
            timing: Timing {
                last: word(&b, 0)?,
                elapsed: word(&b, 4)?,
            },
            period: word(&b, 8)? as i32,
            timer: Timer {
                elapsed: word(&b, 12)?,
                interval: word(&b, 16)?,
                last: word(&b, 20)?,
                limit: word(&b, 24)?,
            },
            poll_messages: b[28],
            budget_d8: word(&b, 32)? as i32,
            budget_dc: word(&b, 36)? as i32,
            publish: b[40],
        };
        ensure!(
            fields.poll_messages == 0 && fields.publish == 0,
            "cadence has a poll/publication callback owner"
        );
        if let Some(mut predicted) = previous {
            if predicted.timing.last != now {
                ensure!(
                    predicted.timer.elapsed < predicted.timer.interval,
                    "clock sample changed inside a timer-credit batch"
                );
                samples.push(json!({"before_completed_index":index,"previous":predicted.timing.last,"now":now}));
                predicted.timing.elapsed = predicted
                    .timing
                    .elapsed
                    .wrapping_add(now.wrapping_sub(predicted.timing.last));
                predicted.timing.last = now;
            }
            predicted.timer.update(now);
            ensure!(
                predicted.timer.consume(),
                "native update has no timer credit"
            );
            predicted.timing.elapsed = predicted
                .timing
                .elapsed
                .wrapping_sub(predicted.period as u32);
            ensure!(
                predicted == fields,
                "native clock counters differ at completed boundary {index}"
            );
        }
        rows.push(json!({"index":index,"tick":state["tick"],"source_sha256":hash,"now":now,"fields":fields}));
        previous = Some(fields);
    }
    ensure!(
        samples == vec![json!({"before_completed_index":9,"previous":584466,"now":586466})],
        "observed clock-sample boundary differs"
    );
    let report = json!({"schema":1,"kind":"original_scalar_timer_cadence","native_parity_credit":0,"samples":samples,"rows":rows,
        "qualification":"Existing Timer kernel against pinned original scalar timestamps/counters. These are post-update observations; clock-getter return ordering, frontend callbacks and draw schedule remain unqualified."});
    if let Some(path) = std::env::var_os("SIMS_RETAINED_CLOCK_DIAG_OUTPUT") {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        f.write_all(&serde_json::to_vec_pretty(&report)?)?;
    }
    eprintln!("all 33 original scalar clock cuts match; one observed sample change before C9");
    Ok(())
}
