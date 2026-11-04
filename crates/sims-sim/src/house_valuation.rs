//! House refresh valuation: original444cb9,43219a and4354d5.
//! Location/tile flags and immutable catalog selection are explicit inputs.
//! This kernel does not advance a thread, invalidate routes or publish money.
mod live;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Prices {
    /// Process6e5838 indexed by widened wall style. Catalog-missing IDs cost0.
    pub styles: Vec<i32>,
    pub pool: i32,
    pub floors: BTreeMap<u16, i32>,
    pub walls: BTreeMap<u16, i32>,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Architecture {
    pub value: i32,
    pub has_structure: bool,
}

///47b006 visits floor, Y, X (with X innermost), including boundary cells unless
///the live flag32 excludes them. Arrays can retain a larger backing stride.
///Wall structure cost accumulates before a single /2; split floor coverings
///truncate each component separately. All additions wrap as signed dwords.
pub fn architecture(
    size: usize,
    stride: usize,
    floors: &[crate::lot_geometry::FloorSource],
    pool: &[u8],
    prices: &Prices,
    mut lookup: impl FnMut(&str, u16),
) -> Result<Architecture, String> {
    if size == 0
        || size > stride
        || stride > 64
        || floors.len() != 2
        || pool.len() != stride * stride
    {
        return Err("invalid valuation arrays or active dimensions".into());
    }
    let mut result = Architecture::default();
    let mut walls = 0i32;
    for (level, floor) in floors.iter().enumerate() {
        if floor.flags.len() != stride * stride || floor.coverings.len() != stride * stride {
            return Err("valuation floor dimensions differ".into());
        }
        if floor.walls.len() != stride * stride {
            return Err("valuation wall dimensions differ".into());
        }
        //4795c5 bounds neighbor ownership by the active world size, not the
        //backing column allocation. Retained imports can be48 in a64 grid.
        let active_walls = if size == stride {
            std::borrow::Cow::Borrowed(floor.walls.as_slice())
        } else {
            std::borrow::Cow::Owned(
                (0..size)
                    .flat_map(|x| floor.walls[x * stride..x * stride + size].iter().cloned())
                    .collect::<Vec<_>>(),
            )
        };
        let grid = crate::lot_walls::Grid::new(size, &active_walls)?;
        for y in 0..size {
            for x in 0..size {
                let cell = x * stride + y;
                if floor.flags[cell] & 32 != 0 {
                    continue;
                }
                let wall = grid.expanded(x, y)?;
                result.has_structure |= wall.segments != 0;
                let mut bit: u8 = if wall.segments == 0 {
                    0
                } else {
                    1 << wall.segments.trailing_zeros()
                };
                while bit != 0 {
                    let index = bit.trailing_zeros() as usize;
                    let style = wall.styles[index].ok_or("valuation wall style unavailable")?;
                    walls = walls.wrapping_add(
                        *prices
                            .styles
                            .get(style as usize)
                            .ok_or("valuation wall style price unavailable")?,
                    );
                    if matches!(style, 1 | 7 | 8 | 9) {
                        let pattern = wall.patterns[index + usize::from(bit >= 16)];
                        lookup("wall", pattern);
                        result.value = result
                            .value
                            .wrapping_add(*prices.walls.get(&pattern).unwrap_or(&0));
                    }
                    loop {
                        bit <<= 1;
                        if bit >= 32 || wall.segments & bit != 0 {
                            break;
                        }
                    }
                    if bit >= 32 {
                        break;
                    }
                }
                let covering = floor.coverings[cell];
                if covering != 0 {
                    if covering == 255 {
                        if wall.segments & 48 != 0 {
                            for id in [wall.styles[0].unwrap(), wall.patterns[0]] {
                                if id != 0 {
                                    lookup("floor", id);
                                    result.value = result.value.wrapping_add(
                                        prices.floors.get(&id).copied().unwrap_or(0) / 2,
                                    );
                                }
                            }
                        }
                    } else {
                        lookup("floor", covering);
                        result.value = result
                            .value
                            .wrapping_add(*prices.floors.get(&covering).unwrap_or(&0));
                    }
                    result.has_structure = true;
                }
                if level == 0 && pool[cell] <= 16 {
                    result.has_structure = true;
                    if pool[cell] < 16 {
                        result.value = result.value.wrapping_add(prices.pool);
                    }
                }
            }
        }
    }
    result.value = (walls / 2).wrapping_add(result.value);
    Ok(result)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Object {
    pub category: i16,
    pub kind: i16,
    pub group: i16,
    pub multipart_class: bool,
    pub head: Option<usize>,
    pub next: Option<usize>,
    pub in_world: bool,
    pub tile_flags: u8,
    pub flags43: u16,
    pub price: i16,
    pub depreciated: bool,
    pub width: i16,
    pub height: i16,
    pub build_type: i16,
    pub catalog_build_type: i16,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Objects {
    pub objects_value: i32,
    pub architecture_objects_value: i32,
    pub family_bit0: bool,
    pub family_bit1: bool,
    pub has_structure: bool,
    pub capabilities: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Publication {
    pub family_flags: u32,
    pub objects_value: i32,
    pub architecture_value: i32,
    /// Original ordered global23,22,37 writes, narrowed to VM signed words.
    pub globals: [i16; 3],
}
///Simulator+a0/a4, separate from its cash balance and expense history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Retained {
    pub objects_value: i32,
    pub architecture_value: i32,
}
pub fn publication(
    objects: &Objects,
    architecture: &Architecture,
    family_number: i32,
    family_flags: u32,
    size: i32,
) -> Publication {
    Publication {
        family_flags: if family_number == 0 {
            family_flags
        } else {
            (family_flags & !3)
                | u32::from(objects.family_bit0)
                | (u32::from(objects.family_bit1) << 1)
        },
        objects_value: objects.objects_value,
        architecture_value: architecture
            .value
            .wrapping_add(objects.architecture_objects_value),
        globals: [
            size as i16,
            i16::from(objects.has_structure || architecture.has_structure),
            objects.capabilities as i16,
        ],
    }
}

///43219a uses the head's signed price, halved toward zero when any physical
///sibling has data15 set. A grouped definition without its subclass returns0.
pub fn price(objects: &[Object], index: usize) -> Result<i16, String> {
    let object = objects.get(index).ok_or("valuation object absent")?;
    let (head, depreciated) = if object.group == 0 {
        (object, object.depreciated)
    } else if !object.multipart_class {
        return Ok(0);
    } else {
        let head = object.head.unwrap_or(index);
        let mut current = Some(head);
        let mut seen = BTreeSet::new();
        let mut depreciated = false;
        while let Some(index) = current {
            if !seen.insert(index) {
                return Err("cyclic valuation sibling chain".into());
            }
            let sibling = objects.get(index).ok_or("valuation sibling absent")?;
            depreciated |= sibling.depreciated;
            current = sibling.next;
        }
        (
            objects.get(head).ok_or("valuation head absent")?,
            depreciated,
        )
    };
    Ok(if depreciated {
        head.price / 2
    } else {
        head.price
    })
}

///444cb9 classifies every registered object, including off-world/secondary
///parts, before the primary-part/value gates. In particular category10 gates
///several other capability bits after the complete enumeration.
pub fn objects(objects: &[Object]) -> Result<Objects, String> {
    let mut result = Objects::default();
    let mut base = false;
    let mut gated = 0;
    for (index, o) in objects.iter().enumerate() {
        match o.category {
            1 => result.family_bit0 |= o.in_world,
            9 => result.family_bit1 = true,
            10 => base = true,
            4 | 46..=48 | 71 => result.capabilities |= 1,
            25 | 26 | 50 | 72 => result.capabilities |= 2,
            73 => result.capabilities |= 4,
            17 | 21..=24 | 30 | 39 | 40 | 58 | 59 | 74 => gated |= 8,
            75 => gated |= 16,
            76 => gated |= 32,
            97 => result.capabilities |= 32,
            43 | 49 | 51 | 77 => result.capabilities |= 64,
            80 => result.capabilities |= 128,
            84 => result.capabilities |= 256,
            85 => result.capabilities |= 512,
            83 => result.capabilities |= 1024,
            95 => result.capabilities |= 2048,
            93 | 94 => result.capabilities |= 4096,
            _ => {}
        }
        if o.group != 0 && o.multipart_class && o.head.is_some_and(|head| head != index) {
            continue;
        }
        if o.kind == 7 {
            continue;
        }
        if o.in_world
            && (o.width != 0 || o.height != 0 || o.build_type != 0)
            && o.tile_flags & 32 == 0
        {
            result.has_structure = true;
        }
        //4354d5's AL result; its redundant bit8 branch returns the same bit
        //once the preceding data43 bit16 gate has succeeded.
        let ordinary = o.flags43 & 16 == 0 && (o.kind == 2 || o.catalog_build_type == 0);
        if o.in_world && ordinary {
            result.objects_value = result
                .objects_value
                .wrapping_add(i32::from(price(objects, index)?));
        } else if o.catalog_build_type != 0 && o.catalog_build_type != 4 {
            result.architecture_objects_value = result
                .architecture_objects_value
                .wrapping_add(i32::from(price(objects, index)?));
        }
    }
    if base {
        result.capabilities |= gated;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn all_original_object_valuation_and_category_cases_match() {
        let corpus: serde_json::Value = serde_json::from_str(include_str!(
            "../tests/fixtures/native-house-valuation.json"
        ))
        .unwrap();
        let rows = corpus["cases"].as_array().unwrap();
        assert_eq!(rows.len(), 651);
        assert_eq!(corpus["native_execution"], true);
        for (i, row) in rows.iter().enumerate() {
            let input: Vec<Object> =
                serde_json::from_value(row["input"]["objects"].clone()).unwrap();
            let expected: Objects = serde_json::from_value(row["original"].clone()).unwrap();
            assert_eq!(
                objects(&input).unwrap(),
                expected,
                "original valuation case{i}"
            );
        }
    }
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn original_valuation_publication_and_global_write_order_match() {
        let corpus: serde_json::Value = serde_json::from_str(include_str!(
            "../tests/fixtures/native-valuation-publication.json"
        ))
        .unwrap();
        assert_eq!(corpus["cases"].as_array().unwrap().len(), 256);
        for (i, row) in corpus["cases"].as_array().unwrap().iter().enumerate() {
            let c = &row["input"];
            let objects: Objects = serde_json::from_value(c["objects"].clone()).unwrap();
            let architecture: Architecture =
                serde_json::from_value(c["architecture"].clone()).unwrap();
            let actual = publication(
                &objects,
                &architecture,
                c["family_number"].as_i64().unwrap() as i32,
                c["family_flags"].as_u64().unwrap() as u32,
                c["size"].as_i64().unwrap() as i32,
            );
            assert_eq!(
                actual,
                serde_json::from_value(row["original"].clone()).unwrap(),
                "original publication case{i}"
            );
            assert_eq!(
                row["original"]["global_writes"],
                serde_json::json!([
                    [23, actual.globals[0] as u16],
                    [22, actual.globals[1] as u16],
                    [37, actual.globals[2] as u16]
                ])
            );
        }
    }
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn two_floor_architecture_matches_original_values_and_lookup_order() {
        let corpus: serde_json::Value = serde_json::from_str(include_str!(
            "../tests/fixtures/native-architecture-valuation.json"
        ))
        .unwrap();
        let rows = corpus["cases"].as_array().unwrap();
        assert_eq!(rows.len(), 180);
        for (i, row) in rows.iter().enumerate() {
            let c = &row["input"];
            let size = c["size"].as_u64().unwrap() as usize;
            let stride = c["stride"].as_u64().unwrap_or(size as u64) as usize;
            let floors: Vec<_> = c["floors"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| crate::lot_geometry::FloorSource {
                    walls: serde_json::from_value(f["walls"].clone()).unwrap(),
                    coverings: serde_json::from_value(f["coverings"].clone()).unwrap(),
                    flags: serde_json::from_value(f["flags"].clone()).unwrap(),
                    object_heads: vec![0; stride * stride],
                })
                .collect();
            let prices = Prices {
                styles: serde_json::from_value(c["style_prices"].clone()).unwrap(),
                pool: c["pool_price"].as_i64().unwrap() as i32,
                floors: serde_json::from_value(c["floor_prices"].clone()).unwrap(),
                walls: serde_json::from_value(c["wall_prices"].clone()).unwrap(),
            };
            let pool: Vec<u8> = serde_json::from_value(c["pool"].clone()).unwrap();
            let mut lookups = vec![];
            let result = architecture(size, stride, &floors, &pool, &prices, |kind, id| {
                lookups.push(serde_json::json!([kind, id]))
            })
            .unwrap();
            assert_eq!(
                result,
                serde_json::from_value(row["original"].clone()).unwrap(),
                "original architecture case{i}"
            );
            assert_eq!(
                serde_json::json!(lookups),
                row["original"]["lookups"],
                "original architecture lookup order{i}"
            );
        }
    }
}
