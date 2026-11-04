//! B-resident cache nodes and raw SPR2 metadata. A cache miss still requires
//! the original loading child; it is never converted into an empty image.
use super::*;
const SOURCE: &str = "56f35639e799e1866e09a72a220d0532b3173ab7bb44136e2b4015410dfc0bbd";
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Cached {
    pointer: u32,
    version: u32,
    count: i32,
    frames: Vec<[u8; 16]>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Source {
    identity: String,
    entries: BTreeMap<u32, Cached>,
    pub master: u32,
    pub category: i16,
    pub kind: i16,
    pub layer: u32,
    pub layer_flags: [u8; 4],
    pub auxiliary_empty: bool,
}
impl Source {
    pub fn same_snapshot_data(&self,other:&Self)->bool{let mut value=self.clone();value.identity=other.identity.clone();value==*other}
    pub fn import(encoded: &[u8]) -> Result<Self> {
        ensure!(
            sims_fixtures::digest(encoded) == SOURCE,
            "sprite frame source differs"
        );
        let v: serde_json::Value = serde_json::from_slice(encoded)?;
        ensure!(
            v["tick"] == 89073 && v["executes_target_code"] == false,
            "sprite frame epoch differs"
        );
        let m = Memory::new(&v)?;
        Self::from_memory(&v, &m, 0x0c77c558, SOURCE)
    }
    pub fn from_memory(
        v: &serde_json::Value,
        m: &Memory,
        pointer: u32,
        identity: &str,
    ) -> Result<Self> {
        let o = m.read(pointer, 0x18c)?;
        let res = m.read(word(&o, 0x11c), 0xb8)?;
        let wrapper = word(&res, 8);
        ensure!(
            v["wrapper"].as_u64() == Some(u64::from(wrapper))
                || v["wrappers"]
                    .as_array()
                    .is_some_and(|a| a.iter().any(|x| x.as_u64() == Some(u64::from(wrapper)))),
            "sprite wrapper relation differs"
        );
        let w = m.read(wrapper, 0x28)?;
        let b = word(&w, 8);
        let e = word(&w, 12);
        ensure!(b < e && (e - b) % 4 == 0, "sprite buckets invalid");
        let buckets = m.read(b, (e - b) as usize)?;
        let mut entries = BTreeMap::new();
        for s in v["sprites"].as_array().context("sprite sources absent")? {
            if s["wrapper"].as_u64().or_else(|| v["wrapper"].as_u64()) != Some(u64::from(wrapper)) {
                continue;
            }
            let key = s["key"].as_u64().context("sprite key absent")? as u32;
            let mut node = word(&buckets, (key % ((e - b) / 4)) as usize * 4);
            let mut found = None;
            let mut seen = std::collections::BTreeSet::new();
            while node != 0 {
                ensure!(seen.insert(node), "sprite cache cycle");
                let n = m.read(node, 12)?;
                if word(&n, 4) == key {
                    found = Some(word(&n, 8));
                    break;
                }
                node = word(&n, 0);
            }
            let p = found.context("resident sprite cache node absent")?;
            ensure!(s["pointer"] == p, "sprite cache pointer differs");
            let r = m.read(p, 0x2c)?;
            let version = word(&r, 0x20);
            let count = word(&r, 4) as i32;
            ensure!(
                count >= 0 && count <= 1024 && s["version"] == version && s["count"] == count,
                "sprite cache header differs"
            );
            let data = word(&r, 12);
            let h = m.read(data, 12 + 4 * count as usize)?;
            let mut frames = Vec::new();
            ensure!(
                version == 1000 && word(&h, 0) == 1000 && word(&h, 4) == count as u32,
                "resident SPR2 version differs"
            );
            for i in 0..count as usize {
                frames.push(
                    m.read(data.wrapping_add(word(&h, 12 + 4 * i)), 16)?
                        .try_into()
                        .unwrap(),
                );
            }
            ensure!(
                entries
                    .insert(
                        key,
                        Cached {
                            pointer: p,
                            version,
                            count,
                            frames
                        }
                    )
                    .is_none(),
                "duplicate sprite cache key"
            );
        }
        let d = m.read(word(&o, 0x118), 0xc0)?;
        let master = word(&res, 0x8c);
        let mr = m.read(master, 0x64)?;
        let md = m.read(word(&mr, 0x60), 0xc0)?;
        let layer_raw = m.read(0x09e88960, 0x520)?;
        let vt = m.read(word(&layer_raw, 0), 0x80)?;
        let layer = word(&vt, 8);
        ensure!(
            layer == 0x4aa5ec && v["layer_callback"] == layer,
            "layer drawing callback differs"
        );
        Ok(Self {
            identity: identity.into(),
            entries,
            master,
            category: i16::from_le_bytes(md[0x8a..0x8c].try_into()?),
            kind: i16::from_le_bytes(d[0x12..0x14].try_into()?),
            layer,
            layer_flags: layer_raw[0x4a..0x4e].try_into()?,
            auxiliary_empty: word(&o, 0x148) == word(&o, 0x14c),
        })
    }
    /// The sprite cache is wrapper-owned. Instance-specific definition and
    /// auxiliary fields still come from that instance's acknowledged B record.
    pub fn for_instance(&self, m: &Memory, pointer: u32) -> Result<Self> {
        let o = m.read(pointer, 0x18c)?;
        let d = m.read(word(&o, 0x118), 0xc0)?;
        let res = m.read(word(&o, 0x11c), 0xb8)?;
        let master = word(&res, 0x8c);
        let mr = m.read(master, 0x64)?;
        let md = m.read(word(&mr, 0x60), 0xc0)?;
        let mut state = self.clone();
        state.master = master;
        state.category = i16::from_le_bytes(md[0x8a..0x8c].try_into()?);
        state.kind = i16::from_le_bytes(d[0x12..0x14].try_into()?);
        state.auxiliary_empty = word(&o, 0x148) == word(&o, 0x14c);
        state.validate()?;
        Ok(state)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (self.identity == SOURCE
                || self.identity == CATALOG_SOURCE
                || self.identity == RESIDENT_CATALOG_SOURCE ||is_override_catalog(&self.identity))
                && self.layer == 0x4aa5ec,
            "sprite frame provenance differs"
        );
        Ok(())
    }
    pub fn resident_pointer(&self,id:u32)->Option<u32>{self.entries.get(&id).map(|e|e.pointer)}
    pub fn count(&self, id: u32) -> Option<i32> {
        self.entries
            .get(&id)
            .map(|e| if e.version > 499 { e.count } else { 0 })
    }
    pub fn frame(&self, id: u32, index: i32) -> Result<&[u8; 16]> {
        self.entries
            .get(&id)
            .context("resident frame requires loading child")?
            .frames
            .get(usize::try_from(index)?)
            .context("sprite frame index invalid")
    }
    pub fn metadata(&self, id: u32, index: i32) -> Result<[i16; 4]> {
        let r = self.frame(id, index)?;
        Ok([
            0,
            0,
            i16::from_le_bytes(r[2..4].try_into()?),
            i16::from_le_bytes(r[..2].try_into()?),
        ])
    }
    /// 4c1a0c clamps a COPY. The fully clipped early exit returns the original
    /// bounding rectangle, before testing context flags or touching a plane.
    pub fn clipped_return(
        &self,
        id: u32,
        index: i32,
        origin: [i32; 2],
        clip: [i16; 4],
    ) -> Result<Option<[i32; 4]>> {
        let f = self.frame(id, index)?;
        let width = u16::from_le_bytes(f[..2].try_into()?) as i32;
        let height = u16::from_le_bytes(f[2..4].try_into()?) as i32;
        let [x, y] = origin;
        let r = [x, y, x.wrapping_add(width), y.wrapping_add(height)];
        let c = sims_sim::lot_presenter::room_masks::clip_rectangle(
            r,
            [
                i32::from(clip[1]),
                i32::from(clip[0]),
                i32::from(clip[3]),
                i32::from(clip[2]),
            ],
        );
        if c != r {
            let m = [
                c[0].wrapping_sub(r[0]),
                c[1].wrapping_sub(r[1]),
                r[2].wrapping_sub(c[2]),
                r[3].wrapping_sub(c[3]),
            ];
            if m[0].wrapping_add(m[2]) >= width
                || m[1].wrapping_add(m[3]) >= height
                || (m[0] | m[1] | m[2] | m[3]) < 0
            {
                return Ok(Some(r));
            }
        }
        Ok(None)
    }
}
