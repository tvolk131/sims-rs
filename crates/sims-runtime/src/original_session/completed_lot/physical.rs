//! Current physical owners, borrowed validation against the retained World.
//! No tile chains, coordinates, occupied rooms or indicators are rebuilt.
use super::*;
use sims_sim::{lot_links::Links, lot_placement};
use sims_vm::{
    ObjectId, World, indicator_pools::Pools, load_indicators::Indicators,
    person_loading::RoomMembership,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Imported {
    pub placement: lot_placement::State,
    pub membership: RoomMembership,
    pub indicators: Indicators,
    pub pools: Pools,
    pub selected_person: Option<ObjectId>,
    pub native_parity_ticks: u64,
}

pub fn import(geometry: &super::Imported, world: &World) -> Result<Imported> {
    let input = &geometry.source;
    let m = Memory::new(input)?;
    let rows = input["objects"]
        .as_array()
        .context("physical source objects absent")?;
    ensure!(
        rows.len() == world.objects.len(),
        "physical/World object coverage differs"
    );
    let mut next = BTreeMap::new();
    let mut bounds = BTreeMap::new();
    let mut membership = BTreeMap::new();
    let mut pointers = BTreeMap::new();
    let mut selected = None;
    for row in rows {
        let id: ObjectId = row["object"]
            .as_i64()
            .context("physical object absent")?
            .try_into()?;
        let p = pointer(&row["pointer"])?;
        let raw = bytes(&row["raw"])?;
        ensure!(
            raw.len() == 0x188
                && m.read(p, raw.len())? == raw
                && i16::from_le_bytes(raw[0xf0..0xf2].try_into()?) == id,
            "physical native identity differs"
        );
        let object = world
            .objects
            .get(&id)
            .context("physical native object is not in World")?;
        ensure!(
            object.native_flags == Some(word(&raw, 0x114)?),
            "physical native flags/World differ"
        );
        let data = bytes(&row["slots_raw"])?;
        let (begin, end, capacity) = (word(&raw, 0x130)?, word(&raw, 0x134)?, word(&raw, 0x138)?);
        ensure!(
            begin <= end
                && end <= capacity
                && end - begin == data.len() as u32
                && data.len() >= 36
                && data.len() % 36 == 0
                && row["slots"]["begin"] == begin
                && row["slots"]["end"] == end
                && row["slots"]["capacity"] == capacity
                && m.read(begin, data.len())? == data,
            "physical source slots differ"
        );
        let slots: Vec<i16> = data
            .chunks_exact(36)
            .map(|b| i16::from_le_bytes(b[16..18].try_into().unwrap()))
            .collect();
        let logical: Vec<_> = slots
            .iter()
            .skip(1)
            .map(|&id| if id == 0 { None } else { Some(id) })
            .collect();
        ensure!(
            object.slots == logical,
            "physical source/World script slot aliases differ"
        );
        ensure!(
            next.insert(id, slots[0]).is_none() && pointers.insert(p, id).is_none(),
            "duplicate physical identity"
        );
        let b = std::array::from_fn(|i| {
            i32::from_le_bytes(raw[0x100 + i * 4..0x104 + i * 4].try_into().unwrap())
        });
        bounds.insert(id, b);
        let position = world
            .positions
            .get(&id)
            .context("physical World position absent")?;
        ensure!(
            position.x == word(&raw, 0xf4)? as i32
                && position.y == word(&raw, 0xf8)? as i32
                && position.level == i32::from(raw[0xfe] as i8),
            "physical source/World position differs"
        );
        if object.person.is_some() {
            let raw_membership = bytes(&row["membership_raw"])?;
            ensure!(
                raw_membership.len() == 4 && m.read(p + 0xafc, 4)? == raw_membership,
                "native cached person room absent/different"
            );
            membership.insert(id, i16::from_le_bytes(raw_membership[..2].try_into()?));
            if word(&raw, 0x114)? & 2 != 0 {
                ensure!(
                    selected.replace(id).is_none(),
                    "multiple selected native people"
                );
            }
        } else {
            ensure!(
                row["membership_raw"].is_null(),
                "nonperson cached room owner supplied"
            );
        }
    }
    ensure!(
        next.keys().eq(world.objects.keys()),
        "native physical registry differs"
    );
    let links = Links {
        size: geometry.lot.size,
        heads: (1..=2)
            .map(|level| Ok(shorts(&array(input, &format!("heads{level}"), 2)?)))
            .collect::<Result<_>>()?,
        next,
    };
    links.validate_arrays().map_err(anyhow::Error::msg)?;
    links
        .validate_tile_chains(world)
        .map_err(anyhow::Error::msg)?;
    let raw = bytes(&input["registry_raw"])?;
    let registry = pointer(&input["registry"])?;
    ensure!(
        raw.len() == 0x100
            && m.read(registry, raw.len())? == raw
            && raw[0x44] <= 1
            && world
                .registry_order
                .as_ref()
                .context("World dirty owner absent")?
                .dirty
                == (raw[0x44] != 0),
        "physical registry dirty ownership differs"
    );
    let identity = |p: u32| -> Result<Option<ObjectId>> {
        if p == 0 {
            Ok(None)
        } else {
            Ok(Some(
                *pointers
                    .get(&p)
                    .context("native indicator outside registry")?,
            ))
        }
    };
    let owner = word(&m.read(0x6e48bc, 4)?, 0)?;
    let simulator = word(&m.read(owner + 0x1c, 4)?, 0)?;
    ensure!(
        word(&m.read(simulator + 0xb0, 4)?, 0)? == registry,
        "native physical registry/application alias differs"
    );
    let mut vectors = vec![];
    for offset in [0xd0, 0xdc] {
        let (begin, end, capacity) = (
            word(&raw, offset)?,
            word(&raw, offset + 4)?,
            word(&raw, offset + 8)?,
        );
        ensure!(
            begin <= end && end <= capacity && (end - begin) % 4 == 0 && end - begin <= 65536,
            "native indicator pool extent differs"
        );
        let data = if end > begin {
            m.read(begin, (end - begin) as usize)?
        } else {
            vec![]
        };
        vectors.push(
            data.chunks_exact(4)
                .map(|b| identity(u32::from_le_bytes(b.try_into().unwrap())))
                .collect::<Result<Vec<_>>>()?,
        );
    }
    let pools = Pools {
        active: vectors.remove(0),
        spare: vectors.remove(0),
    };
    let indicators = Indicators {
        primary: identity(word(&raw, 0xc8)?)?,
        secondary: identity(word(&raw, 0xcc)?)?,
    };
    Ok(Imported {
        placement: lot_placement::State {
            links,
            bounds,
            flags: (1..=2)
                .map(|level| array(input, &format!("flags{level}"), 1))
                .collect::<Result<_>>()?,
            spatial_dirty: None,
        },
        membership: RoomMembership(membership),
        indicators,
        pools,
        selected_person: selected,
        native_parity_ticks: 0,
    })
}
