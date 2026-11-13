//! Actual elapsed owner and returning child at fresh Goth baseline 89073.
//! No entry, timer accumulation, notification or application callback runs.
use super::*;
use sims_fixtures::readset::Readset;

fn source(state: &Value, memory: &Readset<'_>) -> Result<()> {
    ensure!(
        state["schema"] == 1
            && state["tick"] == 89073
            && state["natural_completed_index"] == 0
            && state["caller_trace_stop"] == 1
            && state["readset"]["sha256"] == memory.source_sha256()
            && memory.source_sha256()
                == "b69876043a973299166719573a3c2cf9295d0b0c64eef72cee35b21b2f7ebda6"
            && state["physical_baseline"]["archive"]["compressed_sha256"]
                == "b5cb62a85640b361bb183e0207d58418f655c2aef49561cbfcd6f5f8e882a1e4",
        "captured elapsed baseline source differs"
    );
    Ok(())
}

pub fn fields(state: &Value, memory: &Readset<'_>) -> Result<Fields> {
    // The source-checked control projection also establishes CPState and the
    // simulator aliases. Its elapsed counters have exactly one eventual owner.
    let controls = completed_controls::captured::import(state, memory)?;
    let input = &state["domains"]["controls"];
    for (address, count, hash) in [
        (
            0x46d9d6,
            0x130,
            "25d16b6553a209d266b1b2ec04068ff56432af7fea2738fd3b3c53d1462eb0ff",
        ),
        (
            0x46dbab,
            0x100,
            "cf33bcd6e734ae8f952d3b1a50e2f3590519226140244725867d3d44cdf08670",
        ),
    ] {
        ensure!(
            sims_fixtures::digest(&memory.read(address, count)?) == hash,
            "captured elapsed code profile differs"
        );
    }
    ensure!(
        memory.read(0x46da8c, 4)? == [0xcc; 4],
        "captured protected poll profile differs"
    );
    let simulator = pointer(&input["simulator"])?;
    let raw = memory.read(simulator + 0xb8, 41)?;
    ensure!(
        raw == bytes(&input["elapsed_raw"])?,
        "captured elapsed body differs"
    );
    let fields = Fields {
        timing: Timing {
            last: word(&raw, 0),
            elapsed: word(&raw, 4),
        },
        period: word(&raw, 8) as i32,
        timer: Timer {
            elapsed: word(&raw, 12),
            interval: word(&raw, 16),
            last: word(&raw, 20),
            limit: word(&raw, 24),
        },
        poll_messages: raw[28],
        budget_d8: word(&raw, 32) as i32,
        budget_dc: word(&raw, 36) as i32,
        publish: raw[40],
    };
    ensure!(
        fields.timing == controls.timing && fields.poll_messages == 0,
        "captured elapsed counters/poll ownership differs"
    );
    let header = memory.read(simulator + 0x1a4, 8)?;
    let head = word(&header, 0);
    let sentinel = memory.read(head, 8)?;
    ensure!(
        header == bytes(&input["requests_raw"])?
            && word(&header, 4) == 0
            && sentinel == bytes(&input["sentinel_raw"])?
            && word(&sentinel, 0) == head
            && word(&sentinel, 4) == head
            && input["requests"] == json!([]),
        "nonempty/contradictory captured request queue needs its retained service owner"
    );
    Ok(fields)
}

pub fn resume_baseline(
    state: &Value,
    memory: &Readset<'_>,
) -> Result<(transport::State, clock::State)> {
    source(state, memory)?;
    let controls = completed_controls::captured::import(state, memory)?;
    let fields = fields(state, memory)?;
    let input = &state["domains"]["controls"];
    let cpu = super::super::completed_animations::bytes(
        state["barrier"]["raw_g_before"]
            .as_str()
            .context("captured baseline CPU absent")?,
    )?;
    ensure!(
        cpu.len() >= 36
            && state["barrier"]["raw_g_before"] == state["physical_baseline"]["before_g"]
            && word(&cpu, 16) == 0x81fc4c
            && word(&cpu, 20) == 0x81fc80
            && word(&cpu, 32) == 0x46d51d
            && state["identity"]["registers"]["eip"] == 0x46d51d_u32
            && state["identity"]["registers"]["ebp"] == 0x81fc80_u32
            && state["identity"]["process_id"] == 236
            && state["identity"]["thread_id"] == 240,
        "captured elapsed return CPU differs"
    );
    ensure!(
        sims_fixtures::digest(&memory.read(0x46d51d, 8)?)
            == "9fe5e4b912ae090b1d0ee45966c85be1a006085e1b6777d1ae2fb9af6d62a1a3",
        "captured completed child epilogue differs"
    );
    for (address, expected) in [
        (0x81fc84, 0x46da75),
        (0x81fcb4, 0x4d1ab8),
        (0x81fe08, 0x4f7563),
        (0x81fe18, 0x5d5e7c),
    ] {
        ensure!(
            word(&memory.read(address, 4)?, 0) == expected,
            "captured actual enclosing return chain differs"
        );
    }
    let simulator = pointer(&input["simulator"])?;
    ensure!(
        word(&memory.read(0x81fc50, 4)?, 0) == simulator && word(&cpu, 12) == simulator,
        "captured simulator/return child alias differs"
    );
    let now = memory.read(0x6e54e0, 4)?;
    let stopping = memory.read(0x6e54a8, 1)?;
    ensure!(
        now == bytes(&input["now_raw"])? && stopping == bytes(&input["stopping_raw"])?,
        "captured application inputs differ"
    );
    let mut transport = controls.into_transport();
    transport.timing = None;
    let clock = clock::State::from_completed_update_profile(
        fields,
        word(&now, 0),
        stopping[0],
        89073,
        clock::CapturedCode::Goth89073DormantProtectedPoll20261031,
    );
    Ok((transport, clock))
}
