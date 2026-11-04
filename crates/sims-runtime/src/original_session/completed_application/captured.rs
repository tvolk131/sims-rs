//! Actual fresh-B application parent. No post-clock callback is executed.
use super::*;
use sims_fixtures::readset::Readset;

pub struct Imported {
    pub fields: Fields,
    pub timer: sims_vm::scheduler::clock::Timer,
    pub source: Value,
}
impl Imported {
    pub fn runtime_parent(
        &self,
        child: &super::super::clock::State,
    ) -> Result<super::super::application_parent::State> {
        ensure!(
            self.source["tick"] == 89073,
            "fresh application source lost its baseline"
        );
        let mut parent = super::super::application_parent::State::from_fresh_completed_inputs(
            self.fields.clone(),
            child,
            89073,
        )?;
        parent.bind_return_timer(self.timer)?;
        Ok(parent)
    }
}

pub fn import(state: &Value, memory: &Readset<'_>, source: &[u8]) -> Result<Imported> {
    ensure!(
        sims_fixtures::digest(source)
            == "5cdae845c1903909086cba8154226758b70a5c6ee072e527b1983ab116bc7367",
        "fresh application physical-source version differs"
    );
    let proof: Value = serde_json::from_slice(source)?;
    ensure!(
        state["tick"] == 89073
            && proof["tick"] == state["tick"]
            && proof["source_readset_sha256"] == memory.source_sha256()
            && memory.source_sha256()
                == "b69876043a973299166719573a3c2cf9295d0b0c64eef72cee35b21b2f7ebda6"
            && proof["physical_compressed_sha256"]
                == state["physical_baseline"]["archive"]["compressed_sha256"]
            && proof["physical_decoded_sha256"]
                == state["physical_baseline"]["archive"]["decoded_sha256"]
            && proof["executes_target_code"] == false,
        "fresh application source epoch differs"
    );
    let m = Memory::new(&proof)?;
    for r in proof["reads"]
        .as_array()
        .context("application source reads absent")?
    {
        let raw = bytes(&r["raw"])?;
        super::super::completed_skin::captured::overlap(memory, number(&r["address"])?, &raw)?;
    }
    let stack = m.read(0x81fcb4, 360)?;
    ensure!(
        stack == memory.read(0x81fcb4, 360)?
            && word(&stack, 0) == 0x4d1ab8
            && word(&stack, 340) == 0x4f7563
            && word(&stack, 356) == 0x5d5e7c,
        "fresh application suspended return chain differs"
    );
    ensure!(
        sims_fixtures::digest(&m.read(0x4d1901, 566)?)
            == "13f790293f95a2e0c75f1e862be46135d7da46efe66406ff2fbb12ab3299cd97",
        "fresh application code profile differs"
    );
    let receiver = word(&stack, 320);
    let raw = m.read(receiver, 0x1c0)?;
    ensure!(
        proof["receiver"] == receiver && word(&raw, 0) == 0x668d74,
        "fresh active application receiver differs"
    );
    ensure!(
        word(&m.read(0x668d74 + 0x168, 4)?, 0) == 0x5de715
            && sims_fixtures::digest(&m.read(0x5de715, 5)?) == "46eada20450a6b6f8e09daac29adb4f006003d8603766bb4989a349a40e5c67d",
        "fresh application post-clock virtual differs"
    );
    let global = word(&m.read(0x6f2a58, 4)?, 0);
    let interface = m.read(global, 0xb4)?;
    let cpstate = word(&interface, 0xb0);
    let controls = &state["domains"]["controls"];
    ensure!(
        global != receiver
            && controls["application"] == global
            && controls["pointer"] == cpstate
            && m.read(cpstate, 0x220)? == memory.read(cpstate, 0x220)?,
        "fresh application distinct interface/control aliases differ"
    );
    let fields = Fields {
        in_update: raw[0x118],
        suppressed: raw[0x119],
        post_clock_flag: raw[0x60],
    };
    ensure!(
        serde_json::to_value(&fields)? == proof["fields"],
        "fresh application flag projection differs"
    );
    Ok(Imported {
        fields,
        timer: sims_vm::scheduler::clock::Timer {
            elapsed: word(&raw, 0x1a0),
            interval: word(&raw, 0x1a4),
            last: word(&raw, 0x1a8),
            limit: word(&raw, 0x1ac),
        },
        source: json!({"tick":89073,"physical_source_sha256":sims_fixtures::digest(source),
        "receiver":receiver,"separate_global_interface":global,"CPState":cpstate,
        "native_parent":"4d1901 awaiting46d9d6 at4d1ab8","native_parity_credit":0,
        "complete_native_Session_admitted":false}),
    })
}
