//! B-owned material directory and resident sprites. Lookup arguments come from
//! current Rust descriptors. Absent resources remain explicit loading work.
use super::*;
use sims_sim::lot_presenter::dual_sprite;
use std::collections::BTreeMap;
pub(crate) const META: &str = "22398e4a5f136f9ddf2bad2d7c374a47bce628f175ae5a46a9873867cec4e02b";
const READSET: &str = "4d79de0f1b1e98c2e5adca8b783411877e42ddb5df68381a46a63ac281c7db0f";
const META2: &str = "fd8445e993152333ce4001f37edc876ffc8551e51751013b7045fbdafd4ba255";
const READSET2: &str = "9540a4fe7aaa8dfb519565114371524e6cb1d77983068cea51deb27446e2a725";
const META3: &str = "ccc546801d43e899c594800dada039b60dcf6594e36708b7535f995638916ada";
const READSET3: &str = "430b73d4489c7809f797d556e0a296c92b176829469d3dd79a311f42089587ce";
pub(crate) fn is_source(s: &str) -> bool {
    s == META || s == META2 || s == META3
}
fn identity(s: &str, h: &str, n: u64) -> bool {
    (s == META && h == READSET && n == 102762)
        || (s == META2 && h == READSET2 && n == 127776)
        || (s == META3 && h == READSET3 && n == 274950)
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Material {
    pub source: String,
    pub pool: u32,
    pub key: u32,
    pub images: [u32; 3],
    pub palette: u32,
    pub default_palette: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Resource {
    pub cache: u32,
    pub image: u32,
    pub pointer: u32,
    pub header: Vec<u8>,
    pub frames: BTreeMap<u32, (u32, Vec<u8>)>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Prototype {
    kind: u16,
    cache: u32,
    picture: i16,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Catalog {
    source: String,
    readset: sims_fixtures::BlobRef,
    spans: Vec<(u32, Vec<u8>)>,
    pub pool: u32,
    pub default_cache: u32,
    pub resources: Vec<Resource>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    prototypes: BTreeMap<u32, Prototype>,
}
impl Catalog {
    pub fn import(meta: &[u8], raw: &[u8]) -> Result<Self> {
        ensure!(
            identity(
                &sims_fixtures::digest(meta),
                &sims_fixtures::digest(raw),
                raw.len() as u64
            ),
            "geometry catalog input differs"
        );
        let v: serde_json::Value = serde_json::from_slice(meta)?;
        ensure!(
            v["tick"] == 89073
                && v["executes_target_code"] == false
                && v["readset"]["bytes"] == raw.len()
                && v["readset"]["sha256"] == sims_fixtures::digest(raw),
            "geometry catalog epoch/extent differs"
        );
        let m = sims_fixtures::readset::Readset::parse(raw)?;
        //The borrowed readset coverage can have adjacent slices. Coalesce
        //their observed bytes, preserving real gaps rather than filling them.
        let mut spans: Vec<(u32, Vec<u8>)> = vec![];
        for r in m.coverage() {
            if let Some((a, b)) = spans.last_mut() {
                if u64::from(*a) + b.len() as u64 == u64::from(r.address) {
                    b.extend_from_slice(r.bytes);
                    continue;
                }
            }
            spans.push((r.address, r.bytes.to_vec()));
        }
        let mut c = Self {
            source: sims_fixtures::digest(meta),
            readset: sims_fixtures::BlobRef {
                sha256: sims_fixtures::digest(raw),
                bytes: raw.len() as u64,
            },
            spans,
            pool: 0,
            default_cache: 0,
            resources: vec![],
            prototypes: BTreeMap::new(),
        };
        c.pool = word(&c.read(0x6f0bf8, 4)?, 0);
        let pool = c.read(c.pool, 64)?;
        let a = word(&pool, 0x24);
        let z = word(&pool, 0x28);
        ensure!(
            a < z && (z - a) % 8 == 0 && z - a <= 65536,
            "catalog material palettes differ"
        );
        c.default_cache = word(&c.read(a, 8)?, 4);
        ensure!(
            v["material_pool"] == c.pool && v["default_cache"] == c.default_cache,
            "catalog directory aliases differ"
        );
        for entry in v["materials"]
            .as_array()
            .context("catalog material metadata absent")?
        {
            let key = entry["key"]
                .as_u64()
                .context("catalog material key absent")? as u32;
            let material = c.material(key)?;
            ensure!(
                entry["images"] == serde_json::to_value(material.images)?
                    && entry["cache"] == material.palette,
                "catalog material payload differs"
            );
        }
        for entry in v["resources"]
            .as_array()
            .context("catalog resource metadata absent")?
        {
            let cache = entry["cache"].as_u64().context("catalog cache absent")? as u32;
            let image = entry["image"].as_u64().context("catalog image absent")? as u32;
            let header = c.read(cache, 128)?;
            let (a, z) = (word(&header, 8), word(&header, 12));
            ensure!(
                a < z && (z - a) % 4 == 0 && z - a <= 1 << 20,
                "catalog sprite buckets differ"
            );
            let mut node = word(&c.read(a + 4 * (image % ((z - a) / 4)), 4)?, 0);
            let mut seen = std::collections::BTreeSet::new();
            let mut pointer = 0;
            while node != 0 {
                ensure!(
                    seen.insert(node) && seen.len() < 2048,
                    "catalog sprite chain cycle"
                );
                let n = c.read(node, 12)?;
                if word(&n, 4) == image {
                    pointer = word(&n, 8);
                    break;
                }
                node = word(&n, 0);
            }
            ensure!(
                entry["resource"] == pointer && entry["resident"] == (pointer != 0),
                "catalog residency/alias differs"
            );
            if pointer == 0 {
                continue;
            }
            let header = c.read(pointer, 44)?;
            ensure!(
                word(&header, 0) == image && word(&header, 4) <= 256,
                "catalog resource identity differs"
            );
            let directory = word(&header, 12);
            let mut frames = BTreeMap::new();
            for frame in entry["frames"]
                .as_array()
                .context("catalog frame list absent")?
            {
                let index = frame["index"]
                    .as_u64()
                    .context("catalog frame index absent")? as u32;
                ensure!(
                    index < word(&header, 4),
                    "catalog frame index out of bounds"
                );
                let at = word(&c.read(directory + 4 * index, 4)?, 0);
                let size = frame["bytes"]
                    .as_u64()
                    .context("catalog frame extent absent")? as usize;
                let body = c.read(at, size)?;
                ensure!(
                    frame["pointer"] == at
                        && frame["sha256"] == sims_fixtures::digest(&body)
                        && dual_sprite::frame_bytes(&body).map_err(anyhow::Error::msg)?
                            == body.len(),
                    "catalog frame ownership/body differs"
                );
                ensure!(
                    frames.insert(index, (at, body)).is_none(),
                    "catalog duplicate frame"
                );
            }
            c.resources.push(Resource {
                cache,
                image,
                pointer,
                header,
                frames,
            });
        }
        if let Some(entries) = v.get("prototypes").and_then(|v| v.as_array()) {
            for e in entries {
                let native = c.read(
                    e["pointer"].as_u64().context("prototype instance absent")? as u32,
                    0x124,
                )?;
                let definition = word(&native, 0x118);
                ensure!(
                    e["definition"] == definition,
                    "prototype definition alias differs"
                );
                let body = c.read(
                    definition,
                    e["definition_bytes"]
                        .as_u64()
                        .context("prototype definition extent absent")?
                        as usize,
                )?;
                let guid = word(&body, 0x1c);
                let kind = u16::from_le_bytes(body[0x12..0x14].try_into()?);
                ensure!(
                    e["guid"] == guid && e["kind"] == kind && e["portal"] == (kind == 8),
                    "prototype kind/guid differs"
                );
                let resource = word(&native, 0x11c);
                ensure!(
                    e["resource"] == resource,
                    "prototype resource alias differs"
                );
                let (cache, picture) = if kind == 8 {
                    (
                        word(&c.read(resource, 12)?, 8),
                        i16::from_le_bytes(body[0xa2..0xa4].try_into()?),
                    )
                } else {
                    (0, 0)
                };
                ensure!(
                    e["cache"] == cache && e["picture"] == picture,
                    "prototype portal fields differ"
                );
                let p = Prototype {
                    kind,
                    cache,
                    picture,
                };
                if let Some(old) = c.prototypes.insert(guid, p.clone()) {
                    ensure!(old == p, "one GUID has conflicting retained prototypes");
                }
            }
        }
        c.validate()?;
        Ok(c)
    }
    pub fn extends(&self, old: &Self) -> Result<()> {
        ensure!(
            self.source != old.source
                && self.pool == old.pool
                && self.default_cache == old.default_cache,
            "catalog enrichment identity differs"
        );
        for (a, b) in &old.spans {
            ensure!(
                self.read(*a, b.len())? == *b,
                "catalog enrichment changes an observed byte"
            );
        }
        Ok(())
    }
    pub fn portal(
        &self,
        sim: &sims_sim::Simulation,
        point: [u8; 3],
        rotation: u8,
        mask: u32,
    ) -> Result<Option<(u32, i16)>> {
        ensure!(
            !self.prototypes.is_empty(),
            "portal prototype metadata is unobserved"
        );
        ensure!(rotation < 4, "portal selector rotation differs");
        let wanted = if rotation == 0 {
            mask
        } else {
            let at = 0x6f4a70u32
                .wrapping_add(mask.wrapping_sub(u32::from(rotation) * 64).wrapping_mul(4));
            word(&self.read(at, 4)?, 0)
        };
        let world = &sim.vm.world;
        let links = &sim
            .native_lot
            .as_ref()
            .context("portal physical owner absent")?
            .placement
            .links;
        let Some(root) = links.head(world, point.map(|v| v as i8)) else {
            return Ok(None);
        };
        let mut cursor = sims_sim::lot_links::Cursor::new(root);
        let mut seen = std::collections::BTreeSet::new();
        while let Some(id) = cursor.current {
            ensure!(seen.insert(id), "portal physical iterator cycle");
            let object = world
                .objects
                .get(&id)
                .context("portal candidate object absent")?;
            let def = object
                .definition
                .as_ref()
                .context("portal candidate definition absent")?;
            let prototype = self
                .prototypes
                .get(&def.guid)
                .context("portal candidate prototype is unobserved")?;
            if prototype.kind == 8 {
                let facing = *object.data.get(&1).context("portal direction absent")?;
                let flags = *object.data.get(&13).context("portal wall flags absent")?;
                let side = sims_sim::lot_acceptance::person::wall_side(flags, facing)
                    .map_err(anyhow::Error::msg)?;
                if u32::from(side) == wanted {
                    return Ok(Some((prototype.cache, prototype.picture)));
                }
            }
            cursor.advance(links, world).map_err(anyhow::Error::msg)?;
        }
        Ok(None)
    }
    pub fn source(&self) -> &str {
        &self.source
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            identity(&self.source, &self.readset.sha256, self.readset.bytes),
            "catalog retained identity differs"
        );
        let mut end = 0u64;
        for (a, b) in &self.spans {
            ensure!(
                !b.is_empty() && u64::from(*a) >= end && u64::from(*a) + b.len() as u64 <= 1 << 32,
                "catalog retained spans overlap/overflow"
            );
            end = u64::from(*a) + b.len() as u64;
        }
        Ok(())
    }
    pub fn read(&self, address: u32, count: usize) -> Result<Vec<u8>> {
        let i = self.spans.partition_point(|(a, _)| *a <= address);
        let (a, b) = self
            .spans
            .get(i.checked_sub(1).context("catalog unobserved address")?)
            .context("catalog span absent")?;
        let at = (address - *a) as usize;
        Ok(
            b.get(at..at.checked_add(count).context("catalog read overflow")?)
                .with_context(|| {
                    format!("catalog read {address:08x}+{count} crosses an unobserved hole")
                })?
                .to_vec(),
        )
    }
    pub fn material(&self, key: u32) -> Result<Material> {
        let pool = self.read(self.pool, 64)?;
        let head = word(&pool, 0);
        let mut pending = vec![word(&self.read(head, 24)?, 4)];
        let mut seen = std::collections::BTreeSet::new();
        let mut found = None;
        while let Some(at) = pending.pop() {
            if at == 0 || at == head {
                continue;
            }
            ensure!(
                seen.insert(at) && seen.len() < 8192,
                "catalog material tree cycle"
            );
            let n = self.read(at, 24)?;
            pending.extend([word(&n, 8), word(&n, 12)]);
            if word(&n, 16) == key {
                ensure!(found.is_none(), "duplicate catalog material");
                let p = self.read(word(&n, 20), 20)?;
                let a = word(&pool, 0x24);
                let z = word(&pool, 0x28);
                let index = word(&p, 0);
                ensure!(
                    u64::from(index) * 8 + 8 <= u64::from(z - a),
                    "catalog material palette index invalid"
                );
                found = Some(Material {
                    source: self.source.clone(),
                    pool: self.pool,
                    key,
                    images: std::array::from_fn(|i| word(&p, 4 + 4 * i)),
                    palette: word(&self.read(a + 8 * index + 4, 4)?, 0),
                    default_palette: self.default_cache,
                });
            }
        }
        found.context("original material directory has no requested key")
    }
}
