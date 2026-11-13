//! Source-bound prefix of4a8935. The profiler and4b4324 constructor are retained
//! children. Window services and the remaining renderer are not acknowledged.
use super::super::completed_lot::Memory;
use super::{Buffer, word};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use sims_vm::scheduler::clock::{draw::context, profiler::Record};
mod prefix;
pub(super) mod composition;
pub(super) mod scene;
pub(super) mod classification;
pub(super) mod camera_prepare;
pub(super) mod person_visibility;
pub(super) mod drawing;
pub(super) mod paint;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Timestamp {
        name: u32,
    },
    Context(context::Request),
    /// The protected call at4a89a7. The scratch rectangle aliases the preceding
    /// profiler record, so an untouched/failed output cannot become zeroes.
    ClientRectangle {
        window: u32,
        rectangle: [u32; 4],
    },
    RectangleEmpty {
        rectangle: [u32; 4],
    },
    RedrawRegion {
        rectangle: [i32; 4],
    },
    Overlay {
        owner: u32,
    },
    Scan,
    Compose {
        include_visible: u8,
    },
    PrepareScene,
    RefreshIndicators,
    ClassifyObjects,
    PrepareCamera,
    PersonVisibility,
    PrepareDrawing,
    PaintWorld,
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context as _;
    use std::io::Read;
    #[test]
    #[ignore = "private frozen World and live C8 context observation"]
    fn frozen_world_context_matches_original_c8_across_every_getter_cut() -> Result<()> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let source = std::fs::read(
            root.join("fixtures/private/goth-world-draw-20261031-first/world-source-second.json"),
        )?;
        let mut owner = import(&source)?;
        let encoded = std::fs::read(root.join(
            "fixtures/private/goth-world-window-witness-20261031-first/world-window-trace.jsonl.gz",
        ))?;
        ensure!(
            sims_fixtures::digest(&encoded)
                == "5fe281c64299fb12a399916a497a1aa8b8cd5c90b4dc4cfa2ecf284f94c4be11",
            "native World witness differs"
        );
        let mut raw = String::new();
        flate2::read::GzDecoder::new(encoded.as_slice()).read_to_string(&mut raw)?;
        let rows: Vec<serde_json::Value> = raw
            .lines()
            .map(serde_json::from_str)
            .collect::<std::result::Result<_, _>>()?;
        let row = rows
            .iter()
            .find(|r| r["kind"] == "same_stop_outer_readback" && r["pc"] == 0x4a8982)
            .context("native context observation absent")?;
        ensure!(row["tick"] == 89081, "native context cut differs");
        let stack = crate::original_session::completed_animations::bytes(
            row["world_stack"]["raw"]
                .as_str()
                .context("native stack absent")?,
        )?;
        let bp = row["registers"]["ebp"].as_u64().unwrap() as usize;
        let sp = row["world_stack"]["address"].as_u64().unwrap() as usize;
        let expected: Vec<u32> = stack[bp - 0x10c - sp..bp - 0x10c - sp + 116]
            .chunks_exact(4)
            .map(|x| word(x, 0))
            .collect();
        let marker: Vec<u32> = stack[bp - 0x48 - sp..bp - 0x48 - sp + 16]
            .chunks_exact(4)
            .map(|x| word(x, 0))
            .collect();
        ensure!(
            marker[2..] == [0x6c7318, 0],
            "original context scratch alias differs"
        );
        let time = u64::from(marker[0]) | (u64::from(marker[1]) << 32);
        let mut profiler = Buffer::from_captured(vec![], 8).map_err(anyhow::Error::msg)?;
        let mut cuts = 0;
        for _ in 0..24 {
            let wire = postcard::to_allocvec(&owner)?;
            let mut cold: Owner = postcard::from_bytes(&wire)?;
            cold.validate()?;
            let mut cold_profiler = profiler.clone();
            if matches!(owner.pending(), Some(Request::Timestamp { .. })) {
                let mut full = Buffer::from_captured(vec![], 0).map_err(anyhow::Error::msg)?;
                let before = owner.clone();
                assert!(owner.timestamp(time, &mut full).is_err());
                assert_eq!(owner, before);
                owner.timestamp(time, &mut profiler)?;
                cold.timestamp(time, &mut cold_profiler)?;
            } else {
                assert_eq!(owner.advance()?, cold.advance()?);
            }
            assert_eq!(owner, cold);
            assert_eq!(profiler, cold_profiler);
            cuts += 1;
            if matches!(owner.pending(), Some(Request::ClientRectangle { .. })) {
                break;
            }
        }
        assert_eq!(owner.context().unwrap().words().as_slice(), expected);
        assert_eq!(
            owner.pending(),
            Some(Request::ClientRectangle {
                window: 0xa006a,
                rectangle: marker.try_into().unwrap()
            })
        );
        let before = owner.clone();
        assert!(!owner.advance()?);
        assert_eq!(owner, before);
        assert_eq!(profiler.records().len(), 1);
        assert!(cuts >= 16);
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Getters {
    layer: u32,
    renderer: u32,
    #[serde(alias = "row_step")]
    display_rotation: u32,
    zoom: u32,
    origin: [u32; 2],
    camera: u32,
    mode: u32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    Entry,
    Timestamp,
    Context,
    Window,
    Rectangle,
    Region,
    SetupTimestamp,
    Overlay,
    Scan,
    Compose,
    Prepare,
    EndTimestamp,
    Complete,
    Indicators,
    Classify,
    Camera,
    PeopleVisibility,
    Drawing,
    Painting,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Owner {
    owner: u32,
    getters: Getters,
    window: u32,
    phase: Phase,
    context: Option<context::Frame>,
    scratch: Option<[u32; 4]>,
    #[serde(default)]
    prefix: Option<prefix::Fields>,
    #[serde(default)]
    client_result: Option<u32>,
    #[serde(default)]
    empty_result: Option<u32>,
    #[serde(default)]
    camera:Option<camera_prepare::State>,
    #[serde(default)]
    people:Option<person_visibility::State>,
    #[serde(default)]
    drawing:Option<drawing::State>,
    #[serde(default)]
    paint:Option<paint::State>,
}
pub(super) fn import(source: &[u8]) -> Result<Owner> {
    ensure!(
        sims_fixtures::digest(source)
            == "0692556999f16d0cfc52405cd60db7352b2b39607c485278d8892000301056ce",
        "World context source differs"
    );
    let value: serde_json::Value = serde_json::from_slice(source)?;
    ensure!(
        value["tick"] == 89073
            && value["executes_target_code"] == false
            && value["native_parity_credit"] == 0,
        "World context epoch differs"
    );
    let m = Memory::new(&value)?;
    let owner = 0x09e88960;
    let world = m.read(owner, 0x4c0)?;
    let table = m.read(word(&world, 0), 0x60)?;
    for (slot, target) in [
        (0xc, 0x4aaf34),
        (0x10, 0x4aaf38),
        (0x18, 0x4aaf42),
        (0x14, 0x4aaf3c),
        (0x20, 0x4aafd4),
        (0x24, 0x4a6e11),
        (0x34, 0x4acd1e),
        (0x3c, 0x4a8935),
    ] {
        ensure!(
            word(&table, slot) == target,
            "World virtual constructor getter differs"
        );
    }
    let coordinates = m.read(0x6f00d0, 24)?;
    let manager = word(&world, 0x6c);
    let mh = m.read(manager, 0x520)?;
    let process = word(&m.read(0x6e48bc, 4)?, 0);
    ensure!(
        word(&m.read(process, 0x30)?, 0x2c) == manager,
        "World/window manager alias differs"
    );
    // The alternate manager gates skip the body. They are not implemented by
    // this prefix and cannot be silently treated as the observed ready path.
    ensure!(
        word(&mh, 4) == 0 && word(&mh, 0x504) == 0,
        "World manager skip branch requires another continuation"
    );
    let getters = Getters {
        layer: word(&world, 0x68),
        renderer: word(&world, 0x54),
        display_rotation: word(&coordinates, 12),
        zoom: word(&coordinates, 8),
        origin: [word(&coordinates, 0), word(&coordinates, 4)],
        camera: owner + 0xb8,
        mode: word(&world, 0x1c4),
    };
    let result = Owner {
        owner,
        getters,
        window: word(&mh, 0x12c),
        phase: Phase::Entry,
        context: None,
        scratch: None,
        prefix: None,
        client_result: None,
        empty_result: None,
        camera:None,
        people:None,
        drawing:None,
        paint:None,
    };
    result.validate()?;
    Ok(result)
}
impl Owner {
    pub(super) fn at_entry(&self) -> bool {
        self.phase == Phase::Entry
    }
    pub(super) fn complete(&self) -> bool {
        self.phase == Phase::Complete
    }
    pub(super) fn client_returned(&self) -> bool {
        self.client_result.is_some()
    }
    pub(super) fn pending(&self) -> Option<Request> {
        match self.phase {
            Phase::Entry => None,
            Phase::Timestamp => Some(Request::Timestamp { name: 0x6c7318 }),
            Phase::Context => self
                .context
                .as_ref()
                .and_then(context::Frame::pending)
                .map(Request::Context),
            Phase::Window => self.scratch.map(|rectangle| Request::ClientRectangle {
                window: self.window,
                rectangle,
            }),
            Phase::Rectangle => self
                .scratch
                .map(|rectangle| Request::RectangleEmpty { rectangle }),
            Phase::Region => self
                .prefix
                .as_ref()
                .filter(|p| p.drawing_all != 0 || p.dirty_region != 0)
                .map(|p| Request::RedrawRegion {
                    rectangle: p.rectangle,
                }),
            Phase::SetupTimestamp => Some(Request::Timestamp { name: 0x6c7304 }),
            Phase::Overlay => self
                .prefix
                .as_ref()
                .filter(|p| p.overlay != 0 && p.overlay_guard == 0)
                .map(|p| Request::Overlay { owner: p.overlay }),
            Phase::Scan => Some(Request::Scan),
            Phase::Compose => self.prefix.as_ref().map(|p| Request::Compose {
                include_visible: p.include_visible,
            }),
            Phase::Prepare => Some(Request::PrepareScene),
            Phase::Indicators => Some(Request::RefreshIndicators),
            Phase::Classify => Some(Request::ClassifyObjects),
            Phase::Camera => Some(Request::PrepareCamera),
            Phase::PeopleVisibility => Some(Request::PersonVisibility),
            Phase::Drawing => if self.drawing.as_ref().is_some_and(|d|d.pending()==Some(drawing::Request::Timestamp)){Some(Request::Timestamp{name:0x6c72f8})}else{Some(Request::PrepareDrawing)},
            Phase::Painting => self.paint.as_ref().and_then(paint::State::timestamp_name).map_or(Some(Request::PaintWorld),|name|Some(Request::Timestamp{name})),
            Phase::EndTimestamp => Some(Request::Timestamp { name: 0x6c7310 }),
            Phase::Complete => None,
        }
    }
    pub(super) fn context(&self) -> Option<&context::State> {
        self.context.as_ref().map(context::Frame::state)
    }
    pub(super) fn compare_native_context(&self, encoded: &[u8]) -> Result<()> {
        use anyhow::Context as _;
        use std::io::Read;
        self.validate()?;
        ensure!(
            self.context.as_ref().is_some_and(context::Frame::complete),
            "World context is not constructed"
        );
        ensure!(
            sims_fixtures::digest(encoded)
                == "5fe281c64299fb12a399916a497a1aa8b8cd5c90b4dc4cfa2ecf284f94c4be11",
            "native World context witness differs"
        );
        let mut raw = String::new();
        flate2::read::GzDecoder::new(encoded).read_to_string(&mut raw)?;
        let rows: Vec<serde_json::Value> = raw
            .lines()
            .map(serde_json::from_str)
            .collect::<std::result::Result<_, _>>()?;
        let r = rows
            .iter()
            .find(|r| r["kind"] == "same_stop_outer_readback" && r["pc"] == 0x4a8982)
            .context("native World context cut absent")?;
        ensure!(r["tick"] == 89081, "native World context epoch differs");
        let stack = crate::original_session::completed_animations::bytes(
            r["world_stack"]["raw"]
                .as_str()
                .context("native context stack absent")?,
        )?;
        let bp = r["registers"]["ebp"]
            .as_u64()
            .context("native context frame absent")? as usize;
        let sp = r["world_stack"]["address"]
            .as_u64()
            .context("native context stack address absent")? as usize;
        let at = bp
            .checked_sub(0x10c + sp)
            .context("native context outside stack")?;
        let raw = stack
            .get(at..at + 116)
            .context("native context extent absent")?;
        let expected: Vec<u32> = raw.chunks_exact(4).map(|x| word(x, 0)).collect();
        ensure!(
            self.context().unwrap().words().as_slice() == expected,
            "World constructed context differs from original C8"
        );
        Ok(())
    }
    pub(super) fn validate(&self) -> Result<()> {
        ensure!(!matches!(self.phase,Phase::Drawing|Phase::Painting) || self.people.as_ref().is_some_and(person_visibility::State::complete),"World skipped its person visibility owner");
        ensure!(self.phase!=Phase::Painting||self.drawing.as_ref().is_some_and(drawing::State::complete),"World skipped its drawing owner");
        if let Some(drawing)=&self.drawing{drawing.validate()?;}
        if let Some(paint)=&self.paint{paint.validate()?;paint.validate_post(self.drawing.as_ref(),self.context.as_ref())?;ensure!(paint.at_entry()||self.phase==Phase::Painting,"paint escaped its World caller");}
        if let Some(people)=&self.people { people.validate()?;if matches!(self.phase,Phase::Drawing|Phase::Painting) {ensure!(people.complete(),"World skipped person visibility");} }
        if let Some(camera)=&self.camera{camera.validate()?;if self.at_entry(){ensure!(camera.at_entry(),"camera ran before World entry");}if matches!(self.phase,Phase::PeopleVisibility|Phase::Drawing|Phase::Painting){ensure!(camera.complete(),"World skipped camera preparation");}}
        ensure!(
            self.owner == 0x09e88960 && self.window == 0x000a006a,
            "World/window receiver differs"
        );
        ensure!(
            self.getters
                == Getters {
                    layer: 0x09e89030,
                    renderer: 0x09e88e88,
                    display_rotation: 0,
                    zoom: 1,
                    origin: [368, (-130i32) as u32],
                    camera: 0x09e88a18,
                    mode: 0x09e89840
                },
            "World constructor read footprint differs"
        );
        let started = !matches!(self.phase, Phase::Entry | Phase::Timestamp);
        ensure!(
            self.context.is_some() == started && self.scratch.is_some() == started,
            "World context lifetime differs"
        );
        if let Some(context) = &self.context {
            context.validate().map_err(anyhow::Error::msg)?;
            ensure!(
                context.state().words()[0x18 / 4] == self.owner,
                "World context receiver alias differs"
            );
            ensure!(
                self.phase == Phase::Context || context.complete(),
                "window query bypassed context construction"
            );
        }
        if let Some(scratch) = self
            .scratch
            .filter(|_| matches!(self.phase, Phase::Context | Phase::Window))
        {
            ensure!(
                scratch[2..] == [0x6c7318, 0],
                "World profiler/window scratch alias differs"
            );
        }
        let returned = !matches!(
            self.phase,
            Phase::Entry | Phase::Timestamp | Phase::Context | Phase::Window
        );
        ensure!(
            self.client_result.is_some() == returned,
            "World OS reply lifetime differs"
        );
        if returned {
            ensure!(self.prefix.is_some(), "World prefix inputs absent");
        }
        if matches!(
            self.phase,
            Phase::Overlay | Phase::Scan | Phase::Compose | Phase::Prepare | Phase::Indicators | Phase::Classify | Phase::Camera | Phase::PeopleVisibility | Phase::Drawing | Phase::Painting
        ) {
            ensure!(
                self.scratch.is_some_and(|s| s[2..] == [if let Some(name)=self.paint.as_ref().and_then(paint::State::record_name){name}else if self.drawing.as_ref().is_some_and(|d|d.has_record()){0x6c72f8}else{0x6c7304}, 0]),
                "World setup marker/scratch alias differs"
            );
        }
        if let Some(prefix) = &self.prefix {
            prefix.validate()?;
        }
        Ok(())
    }
    pub(super) fn timestamp(&mut self, time: u64, profiler: &mut Buffer) -> Result<()> {
        self.validate()?;
        let Some(Request::Timestamp { name }) = self.pending() else {
            anyhow::bail!("World timestamp is not pending")
        };
        let record = Record {
            timestamp: time,
            name,
            argument: 0,
        };
        if self.phase==Phase::Drawing {
            let r=self.drawing.as_mut().ok_or_else(||anyhow::anyhow!("drawing source absent"))?.timestamp(time)?;
            self.scratch=Some(r.words());return self.validate();
        }
        if self.phase==Phase::Painting {
            let r=self.paint.as_mut().ok_or_else(||anyhow::anyhow!("paint source absent"))?.timestamp(time)?;
            self.scratch=Some(r.words());return self.validate();
        }
        profiler.append(record).map_err(anyhow::Error::msg)?;
        match self.phase {
            Phase::Timestamp => {
                self.scratch = Some(record.words());
                self.context = Some(context::Frame::new(self.owner));
                self.phase = Phase::Context;
            }
            Phase::SetupTimestamp => {
                self.scratch = Some(record.words());
                self.phase = Phase::Overlay;
            }
            Phase::EndTimestamp => self.phase = Phase::Complete,
            _ => unreachable!(),
        }
        self.validate()
    }
    pub(super) fn advance(&mut self) -> Result<bool> {
        self.validate()?;
        match self.phase {
            Phase::Entry => self.phase = Phase::Timestamp,
            Phase::Timestamp
            | Phase::Window
            | Phase::SetupTimestamp
            | Phase::EndTimestamp
            | Phase::Compose
            | Phase::Prepare
            | Phase::Indicators
            | Phase::Classify
            | Phase::Camera
            | Phase::PeopleVisibility | Phase::Drawing | Phase::Painting
            | Phase::Scan
            | Phase::Complete => return Ok(false),
            Phase::Rectangle => {
                let result = sims_vm::ui_effect::widgets::platform::is_rect_empty(
                    self.scratch.map(|s| s.map(|v| v as i32)),
                );
                self.empty_result = Some(result);
                self.phase = if result != 0 {
                    Phase::EndTimestamp
                } else {
                    Phase::Region
                };
            }
            Phase::Region => {
                if self.pending().is_some() {
                    return Ok(false);
                }
                self.phase = Phase::SetupTimestamp;
            }
            Phase::Overlay => {
                if self.pending().is_some() {
                    return Ok(false);
                }
                self.phase = Phase::Scan;
            }
            Phase::Context => {
                let frame = self.context.as_mut().unwrap();
                if let Some(request) = frame.pending() {
                    use context::{Getter as G, Reply};
                    let reply = match request.getter {
                        G::Layer => Reply::Word(self.getters.layer),
                        G::Renderer => Reply::Word(self.getters.renderer),
                        G::DisplayRotation => Reply::Word(self.getters.display_rotation),
                        G::Zoom => Reply::Word(self.getters.zoom),
                        G::Origin => Reply::Origin(self.getters.origin),
                        G::Camera => Reply::Word(self.getters.camera),
                        G::Mode => Reply::Word(self.getters.mode),
                    };
                    ensure!(
                        request.owner == self.owner,
                        "World constructor changed receiver"
                    );
                    frame
                        .acknowledge(request, reply)
                        .map_err(anyhow::Error::msg)?;
                } else if frame.complete() {
                    self.phase = Phase::Window;
                } else {
                    frame.advance().map_err(anyhow::Error::msg)?;
                }
            }
        }
        self.validate()?;
        Ok(true)
    }
}
