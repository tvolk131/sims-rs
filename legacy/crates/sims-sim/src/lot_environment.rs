//! Native room contributions and environment arithmetic (4665d1/467980).
//! Room rebuilding, lighting publication and dirty notification ordering belong
//! to the room manager; a score alone does not complete those services.
use serde::{Deserialize, Serialize};
pub mod lighting;
pub mod live;
pub mod manager;
pub mod objects;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tuning {
    pub object_light: f32,
    pub area_clip: f32,
    pub area_zero: f32,
    pub area_slope: f32,
    pub light_zero: f32,
    pub light_slope: f32,
    pub impact_area_min: f32,
    pub impact_area_max: f32,
    pub impact_multiplier: f32,
    pub outdoor_impact: f32,
    pub outdoor_good: f32,
    pub outdoor_divisor: f32,
    pub daylight_bonus: f32,
    pub dawn_dusk_bonus: f32,
    pub nighttime_bonus: f32,
    pub floor_zero: f32,
    pub floor_slope: f32,
    pub wall_zero: f32,
    pub wall_slope: f32,
}
impl Tuning {
    /// 4665d1. The original provider selects global FCNS 3. Defaults apply to
    /// missing names, not to an absent/unloaded content module.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<f32>) -> Result<Self, String> {
        let get = |name, fallback| lookup(name).unwrap_or(fallback);
        let area_zero = get("area score for zero tiles", -30.0);
        let light_zero = get("light score for 0.0", -20.0);
        let floor_zero = get("floor score for 0%", -40.0);
        let wall_zero = get("wall score for 0%", -40.0);
        let tuning = Self {
            object_light: get("object light contribution factor", 30.0),
            area_clip: get("area clip to score", 60.0),
            area_zero,
            area_slope: get("area score for clip tile", 30.0) - area_zero,
            light_zero,
            light_slope: get("light score for 1.0", 20.0) - light_zero,
            impact_area_min: get("room impact effective area min", 10.0),
            impact_area_max: get("room impact effective area max", 45.0),
            impact_multiplier: get("room impact multiplier", 10.0),
            outdoor_impact: get("outdoor room impact factor", 2.0),
            outdoor_good: get("outdoor good object count factor", 1.0),
            outdoor_divisor: get("outdoor object divisor", 3.0),
            daylight_bonus: get("outdoor daylight bonus", 0.0),
            dawn_dusk_bonus: get("outdoor dawn dusk bonus", -20.0),
            nighttime_bonus: get("outdoor nighttime bonus", -50.0),
            floor_zero,
            floor_slope: get("floor score for 100%", 0.0) - floor_zero,
            wall_zero,
            wall_slope: get("wall score for 100%", 0.0) - wall_zero,
        };
        if serde_json::to_value(&tuning)
            .map_err(|e| e.to_string())?
            .as_object()
            .unwrap()
            .values()
            .any(|v| v.is_null())
        {
            return Err("nonfinite room tuning".into());
        }
        Ok(tuning)
    }
    pub fn from_program(program: &sims_vm::Program) -> Result<Self, String> {
        let module = program
            .modules
            .get(&program.globals)
            .ok_or("room tuning global module absent")?;
        Self::from_lookup(|name| module.named_constants.get(&3)?.get(name))
    }
}

/// Accumulators at room+38..64, 280 and 284. Count arithmetic wraps like the
/// original 32-bit fields; repeated registered tiles must not be deduplicated.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Totals {
    pub half_tiles: i32,
    pub covered_half_tiles: i32,
    pub objects: i32,
    pub window_light: i32,
    pub artificial_light: i32,
    pub object_field39: i32,
    pub impact: i32,
    pub good: i32,
    pub category4: i32,
    pub category5_to7: i32,
    pub walls: i32,
    pub covered_walls: i32,
    pub lights: i32,
    pub people: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scores {
    /// Native single-precision stored bits, including exceptional inputs.
    pub light: u32,
    pub environment: u32,
    /// Argument to 46772d; does not replace the stored room light.
    pub published_light: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomScore {
    pub id: i16,
    pub active: bool,
    pub outdoors: bool,
    pub half_tiles: i32,
    pub environment: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Aggregate {
    pub indoor_area: i32,
    pub outdoor_sum: u32,
    pub outdoor_environment: u32,
    pub dirty_floors: u32,
}

/// The aggregate/finalization portion of 466c3c. Call only after the selected
/// rooms have completed their native rebuild and lighting services. The input
/// order is the native room-directory order, not physical tile order.
pub fn aggregate(
    rooms: &[RoomScore],
    tuning: &Tuning,
    period: i16,
    dirty_floors: u32,
    rebuilt_floor: i32,
) -> Aggregate {
    let mut indoor_area = 0i32;
    let mut sum = 0.0f32;
    for room in rooms.iter().filter(|r| r.active) {
        if room.outdoors {
            sum = (f64::from(f32::from_bits(room.environment)) + f64::from(sum)) as f32;
        } else {
            indoor_area = indoor_area.wrapping_add(room.half_tiles / 2);
        }
    }
    let value = f64::from(sum) / f64::from(tuning.outdoor_divisor);
    // The native FST before the time switch does not pop the extended value.
    let value = match period {
        0 => (value + f64::from(tuning.daylight_bonus)) as f32,
        1 | 3 => (value + f64::from(tuning.dawn_dusk_bonus)) as f32,
        2 => (value + f64::from(tuning.nighttime_bonus)) as f32,
        _ => value as f32,
    };
    // Unlike the room's scalar calculation, the manager leaves unordered values.
    let value = if value < -100.0 {
        -100.0
    } else if value > 100.0 {
        100.0
    } else {
        value
    };
    Aggregate {
        indoor_area,
        outdoor_sum: sum.to_bits(),
        outdoor_environment: value.to_bits(),
        dirty_floors: match rebuilt_floor {
            0 => 0,
            1 => dirty_floors & !1,
            2 => dirty_floors & !2,
            _ => dirty_floors,
        },
    }
}

/// 45cef4. A missing room returns zero; an outdoor room reads the lot aggregate.
pub fn environment(rooms: &[RoomScore], aggregate: &Aggregate, id: i16) -> u32 {
    rooms.iter().find(|r| r.id == id).map_or(0, |r| {
        if r.outdoors {
            aggregate.outdoor_environment
        } else {
            r.environment
        }
    })
}

/// 46740f returns its result in an x87 register, without a float store.
pub fn daylight(period: i16, multiplier: f32) -> f64 {
    let factor = match period {
        1 | 3 => 0.5,
        2 => 0.0,
        _ => 1.0,
    };
    factor * f64::from(multiplier)
}

/// 467980, retaining the original stores and addition order. `occupied_dark`
/// is the room's existing +73 flag; its producer is separate from this routine.
pub fn score(
    totals: &Totals,
    tuning: &Tuning,
    outdoors: bool,
    pool: bool,
    occupied_dark: bool,
    daylight: f64,
) -> Scores {
    let area = f64::from(totals.half_tiles / 2);
    let light = if outdoors {
        daylight as f32
    } else {
        let artificial = (f64::from(totals.artificial_light) * f64::from(0.01_f32) / area
            * f64::from(tuning.object_light)) as f32;
        let window = f64::from(totals.window_light) * f64::from(0.01_f32) / area
            * daylight
            * f64::from(tuning.object_light);
        let window = if window > daylight { daylight } else { window } as f32;
        let value = f64::from(window) + f64::from(artificial);
        // x87 unordered comparisons take the below branch at the first gate.
        if !(value >= f64::from(0.01_f32)) {
            0.0
        } else if value > 0.9900000002235174 {
            1.0
        } else {
            value as f32
        }
    };
    let environment = if outdoors {
        (f64::from(totals.good) * f64::from(tuning.outdoor_good)
            + f64::from(totals.impact) * f64::from(tuning.outdoor_impact)) as f32
    } else {
        let clipped = if area > f64::from(tuning.area_clip) {
            f64::from(tuning.area_clip)
        } else {
            area
        };
        let area_score = (clipped / f64::from(tuning.area_clip) * f64::from(tuning.area_slope)
            + f64::from(tuning.area_zero)) as f32;
        let light_score = (f64::from(light) * f64::from(tuning.light_slope)
            + f64::from(tuning.light_zero)) as f32;
        let effective = if !(area >= f64::from(tuning.impact_area_min)) {
            f64::from(tuning.impact_area_min)
        } else if area > f64::from(tuning.impact_area_max) {
            f64::from(tuning.impact_area_max)
        } else {
            area
        };
        let walls = f64::from(totals.covered_walls) / f64::from(totals.walls)
            * f64::from(tuning.wall_slope)
            + f64::from(tuning.wall_zero);
        let floors = f64::from(totals.covered_half_tiles) / f64::from(totals.half_tiles)
            * f64::from(tuning.floor_slope)
            + f64::from(tuning.floor_zero);
        let value = walls
            + floors
            + f64::from(totals.impact) * f64::from(tuning.impact_multiplier) / effective
            + f64::from(light_score)
            + f64::from(area_score);
        if !(value >= -100.0) {
            -100.0
        } else if value > 100.0 {
            100.0
        } else {
            value as f32
        }
    };
    let published = if !outdoors
        && (pool
            || occupied_dark && totals.lights == 0 && totals.people != 0 && !(light >= 0.8_f32))
    {
        0.8_f32
    } else {
        light
    };
    Scores {
        light: light.to_bits(),
        environment: environment.to_bits(),
        published_light: published.to_bits(),
    }
}

/// 467b74: wall styles which contribute a decoration score.
pub fn decorated_style(style: u16) -> bool {
    matches!(style, 1 | 7 | 8 | 9 | 3 | 5 | 6 | 15 | 23 | 22)
}

impl Totals {
    /// Build the structural contribution from the original registration vector.
    /// This preserves its duplicate tiles and leaves live object contributions
    /// to 467e05's object visits. Exterior classification uses the rebuilt lot.
    pub fn geometry(lot: &crate::lot_geometry::Lot, room: i16) -> Result<Self, String> {
        let floor = lot
            .floor(i32::from((room as u16 >> 10) & 3) + 1)
            .ok_or("environment room floor absent")?;
        let metrics = floor
            .metrics
            .get(&room)
            .ok_or("environment room metrics absent")?;
        let tiles = floor
            .rooms
            .room_tiles
            .get(&room)
            .ok_or("environment room tiles absent")?;
        let grid = crate::lot_walls::Grid::new(lot.size, &floor.source.walls)?;
        let mut result = Self::default();
        for &[x, y, _] in tiles {
            if x >= lot.size || y >= lot.size {
                return Err("environment room tile outside lot".into());
            }
            let cell = x * lot.size + y;
            result.add_tile(
                metrics.outdoors,
                *lot.pool.get(cell).ok_or("environment ground cell absent")?,
                *floor
                    .source
                    .coverings
                    .get(cell)
                    .ok_or("environment floor cover absent")?,
                &grid.expanded(x, y)?,
            )?;
        }
        Ok(result)
    }
    /// 467bb1. `ground` is the native 478f42 byte (the pool/ground-region grid).
    /// Diagonal floor coverage deliberately observes expanded style/pattern
    /// words, as the original does, rather than the ordinary floor-covering grid.
    pub fn add_tile(
        &mut self,
        outdoors: bool,
        ground: u8,
        floor: u16,
        wall: &crate::lot_walls::Expanded,
    ) -> Result<(), String> {
        if ground < 16 {
            self.good = self.good.wrapping_add(1);
        }
        if outdoors {
            self.half_tiles = self.half_tiles.wrapping_add(2);
            return Ok(());
        }
        if wall.segments & 48 == 0 {
            self.half_tiles = self.half_tiles.wrapping_add(2);
            if floor != 0 {
                self.covered_half_tiles = self.covered_half_tiles.wrapping_add(2);
            }
            for i in 0..4 {
                if wall.segments & (1 << i) != 0
                    && decorated_style(wall.styles[i].ok_or("room wall style absent")?)
                {
                    self.walls = self.walls.wrapping_add(1);
                    if wall.patterns[i] != 0 {
                        self.covered_walls = self.covered_walls.wrapping_add(1);
                    }
                }
            }
        } else {
            self.half_tiles = self.half_tiles.wrapping_add(1);
            let (style, patterns) = if wall.segments & 16 != 0 {
                (4, [4, 5])
            } else {
                (5, [6, 7])
            };
            if decorated_style(wall.styles[style].ok_or("diagonal room wall style absent")?) {
                self.walls = self.walls.wrapping_add(1);
                if patterns.iter().any(|&i| wall.patterns[i] != 0) {
                    self.covered_walls = self.covered_walls.wrapping_add(1);
                }
            }
            if wall.styles[0].ok_or("diagonal coverage style absent")? != 0 || wall.patterns[0] != 0
            {
                self.covered_half_tiles = self.covered_half_tiles.wrapping_add(1);
            }
        }
        Ok(())
    }
}
