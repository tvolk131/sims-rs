//! The observed Goth45x45 window publication. Geometry borrows the authoritative
//! window bytes; external headers are source owners, never a second game world.
//! Native COMRelease internals/locked surfaces remain outside this adapter.
use super::*;
use sims_vm::{
    scheduler::clock::draw::actions::image,
    ui_effect::widgets::{layout, surface_geometry as geometry, visibility::Call},
};
use std::collections::BTreeSet;
const WINDOW: u32 = 0x0b7c7cb0;
const OLD: u32 = 0x0bb6d5e0;
const SURFACE: u32 = 0x0bb6d7a0;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Owner {
    frame: Option<image::Frame>,
    call: Option<(u32, u32)>,
    external: BTreeMap<u32, Vec<u8>>,
    tables: BTreeMap<u32, Vec<u8>>,
    sentinel: Vec<u8>,
    bitmap: Vec<u8>,
    surface: Vec<u8>,
    source_surface: Vec<u8>,
    interfaces: BTreeSet<u32>,
    freed: BTreeSet<u32>,
}
pub(super) fn import(source: &[u8], headers: &BTreeMap<u32, Vec<u8>>) -> Result<Owner> {
    ensure!(
        sims_fixtures::digest(source)
            == "0260547e0fd281fd8db884bdb6cc146470c858e5e9df58683091d95340864744",
        "image publication source differs"
    );
    let value: serde_json::Value = serde_json::from_slice(source)?;
    let memory = Memory::new(&value)?;
    ensure!(
        memory.read(WINDOW, 0x124)? == headers[&WINDOW],
        "image window source alias differs"
    );
    let external = [0x0315a250, 0x030f6088]
        .into_iter()
        .map(|p| Ok((p, memory.read(p, 0x124)?)))
        .collect::<Result<_>>()?;
    let tables = [0x6689c0, 0x668d74, 0x6738a4]
        .into_iter()
        .map(|p| Ok((p, memory.read(p, 0x1a0)?)))
        .collect::<Result<_>>()?;
    let bitmap = memory.read(OLD, 100)?;
    let surface = memory.read(SURFACE, 0x188)?;
    ensure!(
        word(&bitmap, 0x48) == SURFACE && word(&surface, 0x184) == 0,
        "image surface profile differs"
    );
    let result = Owner {
        frame: None,
        call: None,
        external,
        tables,
        sentinel: memory.read(0x0ed87870, 12)?,
        bitmap,
        surface: surface.clone(),
        source_surface: surface.clone(),
        interfaces: [word(&surface, 4), word(&surface, 8)]
            .into_iter()
            .filter(|p| *p != 0)
            .collect(),
        freed: BTreeSet::new(),
    };
    result.validate(headers)?;
    Ok(result)
}
impl Owner {
    pub(super) fn adjustments(&self) -> Vec<(u32, i32)> {
        self.frame
            .as_ref()
            .map_or_else(Vec::new, image::Frame::adjustments)
    }
    pub(super) fn pending(&self) -> Option<&image::Request> {
        self.frame.as_ref().and_then(image::Frame::pending)
    }
    pub(super) fn validate_counts(
        &self,
        headers: &BTreeMap<u32, Vec<u8>>,
        counts: &BTreeMap<u32, u32>,
    ) -> Result<()> {
        if let Some(frame) = &self.frame {
            frame
                .validate(&headers[&WINDOW], counts)
                .map_err(anyhow::Error::msg)?;
        }
        Ok(())
    }
    pub(super) fn validate(&self, headers: &BTreeMap<u32, Vec<u8>>) -> Result<()> {
        ensure!(
            self.external.keys().copied().eq([0x030f6088, 0x0315a250])
                && self.bitmap.len() == 100
                && self.surface.len() == 0x188
                && self.source_surface.len() == 0x188,
            "publication source extent differs"
        );
        ensure!(
            self.sentinel.len() == 12
                && word(&self.sentinel, 0) == 0x0ed87870
                && word(&self.sentinel, 4) == 0x0ed87870,
            "publication child list differs"
        );
        for (key, hash) in [
            (
                0x6689c0,
                "8c0fc4107a414cb0d444e3b52fe6d6f8aa939382f8c37de429273ea3e6e520d3",
            ),
            (
                0x668d74,
                "3ae1fb9683c15200437039a5868e475f238e1f8bf045f855e44f1125cf393ba0",
            ),
            (
                0x6738a4,
                "25fedf9d2f04c4a7cc80c320373ade092231044525fc614f3c6cc0b02c2347bb",
            ),
        ] {
            ensure!(
                self.tables
                    .get(&key)
                    .is_some_and(|b| sims_fixtures::digest(b) == hash),
                "publication virtual source changed"
            );
        }
        ensure!(
            self.tables.len() == 3
                && sims_fixtures::digest(&self.source_surface)
                    == "3688579c1028856284a1f3f73ae899002a6482e9276a15146d9778ece812e268"
                && sims_fixtures::digest(&self.sentinel)
                    == "8e28de13ed358124179e3325f4e8e2718ff3692e28f1c0d2b532e2645291a17c",
            "publication immutable source changed"
        );
        for (key, hash) in [
            (
                0x0315a250,
                "78b2be16f8277c62917621d9b59a60280fd0cf2039f246696784084250bd3dbd",
            ),
            (
                0x030f6088,
                "2341e037901ac18e93e3593445cbd406d1bc442f34b37463e8dcb1d569d2a2ed",
            ),
        ] {
            ensure!(
                sims_fixtures::digest(&self.external[&key]) == hash,
                "publication external parent changed"
            );
        }
        let retired = self.freed.contains(&OLD);
        ensure!(
            self.freed
                == if retired {
                    BTreeSet::from([OLD, SURFACE])
                } else {
                    BTreeSet::new()
                },
            "publication free ownership differs"
        );
        let mut surface = self.surface.clone();
        for i in [0, 4, 8] {
            surface[i..i + 4].copy_from_slice(&self.source_surface[i..i + 4]);
        }
        surface[0x15c] = self.source_surface[0x15c];
        ensure!(
            surface == self.source_surface,
            "unowned surface bytes changed"
        );
        if retired {
            ensure!(
                self.interfaces.is_empty()
                    && word(&self.surface, 0) == 0x676cf8
                    && word(&self.surface, 4) == 0
                    && word(&self.surface, 8) == 0
                    && self.surface[0x15c] == 0
                    && word(&self.bitmap, 0x48) == 0,
                "retired image retains an owner"
            );
        } else {
            ensure!(
                self.surface == self.source_surface
                    && word(&self.bitmap, 0x48) == SURFACE
                    && self.interfaces
                        == [word(&self.source_surface, 4), word(&self.source_surface, 8)]
                            .into_iter()
                            .filter(|p| *p != 0)
                            .collect(),
                "live image surface changed"
            );
        }
        let mut bitmap = self.bitmap.clone();
        bitmap[0x48..0x4c].copy_from_slice(&SURFACE.to_le_bytes());
        bitmap[0x58..0x5c].copy_from_slice(&2u32.to_le_bytes());
        ensure!(
            sims_fixtures::digest(&bitmap)
                == "9c79b1741d94750d258862d914c81db71b0c41ee6d5d00cee485ff7eefa67fb4",
            "unowned bitmap bytes changed"
        );
        ensure!(
            self.frame.is_some() == self.call.is_some(),
            "publication caller absent"
        );
        if let Some(call) = self.call {
            ensure!(
                call.0 == WINDOW
                    && headers.contains_key(&call.0)
                    && self.frame.as_ref().unwrap().arguments() == call,
                "publication window/arguments absent"
            );
        }
        Ok(())
    }
    pub(super) fn advance(
        &mut self,
        window: u32,
        incoming: u32,
        headers: &mut BTreeMap<u32, Vec<u8>>,
        resources: &mut records::Resources,
    ) -> Result<bool> {
        ensure!(window == WINDOW, "image publication window not qualified");
        if self.frame.is_none() {
            self.frame = Some(image::Frame::new(window, incoming));
            self.call = Some((window, incoming));
        }
        ensure!(
            self.call == Some((window, incoming)),
            "image publication caller changed"
        );
        let sizes = resources.image_sizes()?;
        let h = headers.get_mut(&window).unwrap();
        let request = if let Some(request) = self.frame.as_ref().unwrap().pending().cloned() {
            Some(request)
        } else {
            self.frame
                .as_mut()
                .unwrap()
                .advance(h, &mut resources.images, &sizes)
                .map_err(anyhow::Error::msg)?;
            self.bitmap[0x58..0x5c].copy_from_slice(&resources.images[&OLD].to_le_bytes());
            return Ok(self.frame.as_ref().unwrap().complete());
        };
        if let Some(request) = request {
            match request {
                image::Request::Destroy { image, flags } => {
                    ensure!(
                        image == OLD
                            && resources.images.get(&OLD) == Some(&0)
                            && self.freed.is_empty(),
                        "image deleting owner differs"
                    );
                    self.bitmap[0x58..0x5c].copy_from_slice(&0u32.to_le_bytes());
                    struct Host<'a> {
                        surface: &'a mut Vec<u8>,
                        interfaces: &'a mut BTreeSet<u32>,
                        freed: &'a mut BTreeSet<u32>,
                    }
                    impl image::surface::Backend for Host<'_> {
                        fn release(
                            &mut self,
                            _: &mut [u8],
                            interface: u32,
                        ) -> std::result::Result<(), String> {
                            if !self.interfaces.remove(&interface) {
                                return Err("surface interface already released".into());
                            }
                            Ok(())
                        }
                        fn free(
                            &mut self,
                            _: &mut [u8],
                            owner: u32,
                        ) -> std::result::Result<(), String> {
                            if owner != SURFACE || !self.freed.insert(owner) {
                                return Err("surface allocation differs".into());
                            }
                            Ok(())
                        }
                    }
                    impl image::bitmap::Backend for Host<'_> {
                        fn virtual_call(
                            &mut self,
                            _: &mut [u8],
                            owner: u32,
                            slot: u16,
                            args: &[u32],
                        ) -> std::result::Result<(), String> {
                            if owner != SURFACE {
                                return Err("bitmap external owner not qualified".into());
                            }
                            let mut fields = std::mem::take(self.surface);
                            let result = match (slot, args) {
                                (4, []) => image::surface::close(&mut fields, self),
                                (0, [flags]) => {
                                    image::surface::destroy(owner, &mut fields, *flags, self)
                                }
                                _ => Err("bitmap surface callback not qualified".into()),
                            };
                            *self.surface = fields;
                            result
                        }
                        fn release_image(
                            &mut self,
                            _: &mut [u8],
                            _: u32,
                        ) -> std::result::Result<(), String> {
                            Err("bitmap nested image not qualified".into())
                        }
                        fn destroy_string(
                            &mut self,
                            _: &mut [u8],
                            _: u32,
                        ) -> std::result::Result<(), String> {
                            Err("bitmap string not qualified".into())
                        }
                        fn free(
                            &mut self,
                            _: &mut [u8],
                            owner: u32,
                        ) -> std::result::Result<(), String> {
                            if owner != OLD || !self.freed.insert(owner) {
                                return Err("bitmap allocation differs".into());
                            }
                            Ok(())
                        }
                    }
                    image::bitmap::destroy(
                        image,
                        &mut self.bitmap,
                        flags,
                        &mut Host {
                            surface: &mut self.surface,
                            interfaces: &mut self.interfaces,
                            freed: &mut self.freed,
                        },
                    )
                    .map_err(anyhow::Error::msg)?;
                }
                image::Request::Rectangle { window, rectangle } => {
                    self.invoke(
                        headers,
                        &Call::new(window, 0x60, &rectangle.map(|x| x as u32)),
                        0,
                    )?;
                }
            }
            self.frame
                .as_mut()
                .unwrap()
                .acknowledge(
                    headers.get_mut(&window).unwrap(),
                    &resources.images,
                    &request,
                )
                .map_err(anyhow::Error::msg)?;
        }
        self.validate(headers)?;
        Ok(self.frame.as_ref().unwrap().complete())
    }
    #[cfg(test)]
    pub(super) fn compare_native(
        &self,
        raw: &[u8],
        headers: &BTreeMap<u32, Vec<u8>>,
        counts: &BTreeMap<u32, u32>,
    ) -> Result<()> {
        ensure!(
            sims_fixtures::digest(raw)
                == "a81e6ee77c33ba4bbe269f82708b36788e05bff132bcaaa1fa8ff14c4d1a639e",
            "native Goth publication result differs"
        );
        let result: serde_json::Value = serde_json::from_slice(raw)?;
        let decode = |key: &str| -> Result<Vec<u8>> {
            let value = result[key]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("native publication field absent"))?;
            (0..value.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&value[i..i + 2], 16).map_err(Into::into))
                .collect()
        };
        ensure!(
            self.bitmap == decode("old_header")? && self.surface == decode("surface")?,
            "Rust bitmap/surface differ from original full caller"
        );
        let mut window = decode("window_after")?;
        window[0xdc..0xe0].copy_from_slice(&self.call.unwrap().1.to_le_bytes());
        ensure!(
            headers[&WINDOW] == window
                && counts[&OLD] as u64 == result["old_count"].as_u64().unwrap()
                && counts[&self.call.unwrap().1] as u64 == result["new_count"].as_u64().unwrap(),
            "Rust image publication differs from original"
        );
        Ok(())
    }
    fn header<'a>(&'a self, headers: &'a BTreeMap<u32, Vec<u8>>, window: u32) -> Result<&'a [u8]> {
        headers
            .get(&window)
            .or_else(|| self.external.get(&window))
            .map(Vec::as_slice)
            .ok_or_else(|| anyhow::anyhow!("publication layout owner missing"))
    }
    fn projection(&self, headers: &BTreeMap<u32, Vec<u8>>) -> Result<geometry::State> {
        let mut windows = BTreeMap::new();
        let mut fields = BTreeMap::new();
        for window in [WINDOW, 0x0315a250, 0x030f6088] {
            let h = self.header(headers, window)?;
            let rectangle = |o| std::array::from_fn(|i| word(h, o + 4 * i) as i32);
            windows.insert(
                window,
                layout::Fields {
                    rectangle: rectangle(0x7c),
                    absolute: rectangle(0xc),
                    sentinel: word(h, 0x38),
                    updating: h[0x3c],
                    surface: word(h, 0x58),
                    owned: word(h, 0xd0),
                },
            );
            fields.insert(
                window,
                geometry::Fields {
                    local: rectangle(0x1c),
                    relative: rectangle(0x8c),
                },
            );
        }
        Ok(geometry::State {
            layout: layout::State {
                windows,
                links: BTreeMap::from([(
                    0x0ed87870,
                    layout::Link {
                        next: word(&self.sentinel, 0),
                        window: word(&self.sentinel, 8),
                    },
                )]),
                global_manager: 0,
                roots: BTreeMap::new(),
            },
            geometry: fields,
            regions: BTreeMap::new(),
        })
    }
    fn publish(&mut self, headers: &mut BTreeMap<u32, Vec<u8>>, s: &geometry::State) -> Result<()> {
        for (&window, f) in &s.layout.windows {
            let h = headers
                .get_mut(&window)
                .or_else(|| self.external.get_mut(&window))
                .ok_or_else(|| anyhow::anyhow!("publication layout owner absent"))?;
            for (offset, r) in [
                (0x7c, f.rectangle),
                (0xc, f.absolute),
                (0x1c, s.geometry[&window].local),
                (0x8c, s.geometry[&window].relative),
            ] {
                for (i, x) in r.iter().enumerate() {
                    h[offset + 4 * i..offset + 4 * i + 4].copy_from_slice(&x.to_le_bytes());
                }
            }
            h[0x3c] = f.updating;
            for (offset, p) in [(0x38, f.sentinel), (0x58, f.surface), (0xd0, f.owned)] {
                h[offset..offset + 4].copy_from_slice(&p.to_le_bytes());
            }
        }
        Ok(())
    }
    /// These source-pinned callbacks have no external/yielding child on the
    /// observed same-size, no-owned-surface, empty-child-list path.
    fn invoke(
        &mut self,
        headers: &mut BTreeMap<u32, Vec<u8>>,
        call: &Call,
        depth: u8,
    ) -> Result<Option<u32>> {
        ensure!(depth < 12, "publication layout recursion exceeded");
        let h = self.header(headers, call.window)?;
        let table = self
            .tables
            .get(&word(h, 0))
            .ok_or_else(|| anyhow::anyhow!("publication class absent"))?;
        let target = word(table, call.slot as usize);
        let lf = match (call.slot, target, call.arguments.as_slice()) {
            (0x18, 0x436e78, []) => return Ok(Some(word(h, 0x40))),
            (0x184, 0x603390, []) => return Ok(Some(word(h, 0x58))),
            (0x8c, 0x436fef, [mask]) => return Ok(Some(u32::from(word(h, 0x9c) & mask != 0))),
            (0x60, 0x5ddb0c, [a, b, c, d]) => Some(layout::Function::Rectangle {
                rectangle: [*a as i32, *b as i32, *c as i32, *d as i32],
            }),
            (0x198, 0x5dd4f3, []) => Some(layout::Function::Refresh),
            (0x88, 0x5dc153, []) => Some(layout::Function::Absolute),
            (0x194, 0x5dd43d, []) => None,
            _ => anyhow::bail!("publication layout callback not qualified: {call:?} ->{target:x}"),
        };
        if let Some(function) = lf {
            let mut frame = layout::Invocation::new(call.window, function);
            for _ in 0..128 {
                let mut s = self.projection(headers)?;
                let request = frame.advance(&mut s.layout).map_err(anyhow::Error::msg)?;
                self.publish(headers, &s)?;
                let Some(request) = request else {
                    return Ok(None);
                };
                let layout::Request::Virtual(ref child) = request else {
                    anyhow::bail!("publication external layout child not qualified: {request:?}");
                };
                let reply = self
                    .invoke(headers, child, depth + 1)?
                    .map_or(layout::Reply::Done, layout::Reply::Word);
                frame
                    .acknowledge(&request, reply)
                    .map_err(anyhow::Error::msg)?;
            }
        } else {
            let mut frame = geometry::Invocation::new(call.window, geometry::Function::Local);
            for _ in 0..128 {
                let mut s = self.projection(headers)?;
                let request = frame.advance(&mut s).map_err(anyhow::Error::msg)?;
                self.publish(headers, &s)?;
                let Some(request) = request else {
                    return Ok(None);
                };
                let geometry::Request::Virtual(ref child) = request else {
                    anyhow::bail!("publication external geometry child not qualified: {request:?}");
                };
                let reply = self
                    .invoke(headers, child, depth + 1)?
                    .map_or(geometry::Reply::Done, geometry::Reply::Word);
                frame
                    .acknowledge(&request, reply)
                    .map_err(anyhow::Error::msg)?;
            }
        }
        anyhow::bail!("publication layout work bound exceeded")
    }
}
