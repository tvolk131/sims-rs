//! One object visit within native 467e05. Live fields are read at their original
//! stages; lighting/location services may mutate them before the next stage.
use super::Totals;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Input {
    pub hidden: i16,
    pub room: i16,
    pub category: i16,
    pub kind: i16,
    pub flags: u16,
    pub contribution: i16,
    /// Object data 16, passed to the light-map services (not facing/data 1).
    pub light_mode: i16,
    pub field39: i16,
    pub impact: i16,
    /// Native physical X/Y (opposite the plan coordinate convention).
    pub position: [i32; 2],
    pub level: i32,
}
impl Input {
    pub fn read(
        world: &sims_vm::World,
        program: &sims_vm::Program,
        id: sims_vm::ObjectId,
    ) -> Result<Self, String> {
        Self::read_with(world, id, |definition| {
            program
                .modules
                .get(&definition.source)
                .and_then(|m| m.object_definition_words.get(&definition.object))
                .and_then(|w| w.get(9))
                .copied()
                .ok_or_else(|| "room contribution kind absent".to_owned())
        })
    }
    /// A resource adapter supplies exactly OBJD word9 at the native read site.
    /// All instance fields and their read order remain in this shared path.
    pub fn read_with(
        world: &sims_vm::World,
        id: sims_vm::ObjectId,
        mut kind: impl FnMut(&sims_vm::Definition) -> Result<i16, String>,
    ) -> Result<Self, String> {
        let object = world
            .objects
            .get(&id)
            .ok_or("room contribution object absent")?;
        let field = |index| {
            object
                .data
                .get(&index)
                .copied()
                .ok_or_else(|| format!("room contribution object {id} data {index} absent"))
        };
        let definition = object
            .definition
            .as_ref()
            .ok_or("room contribution definition absent")?;
        let kind = kind(definition)?;
        let position = world
            .positions
            .get(&id)
            .ok_or("room contribution position absent")?;
        Ok(Self {
            hidden: field(34)?,
            room: field(29)?,
            category: field(59)?,
            kind,
            flags: field(40)? as u16,
            contribution: field(51)?,
            light_mode: field(16)?,
            field39: field(39)?,
            impact: field(7)?,
            position: [position.x, position.y],
            level: position.level,
        })
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    /// 432595: includes 4325b5's actual containment/location side effects.
    Location,
    /// 467ef7..467f6e: retain a light model when fewer than six are in use,
    /// sample terrain, project its position and append the light-position vector.
    /// The lighting host owns this composite service and its mutable vectors.
    PrepareLight {
        position: [i32; 2],
    },
    ClearLight {
        tile: [i8; 3],
        mode: i16,
    },
    DrawLight {
        tile: [i8; 3],
        mode: i16,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Response {
    Located(bool),
    Done,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Begin,
    Light(bool),
    Prepare,
    Clear,
    Draw,
    Finish,
    Done,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Execution {
    room: i16,
    outdoors: bool,
    stage: Stage,
    pending: Option<Request>,
    position: [i32; 2],
    tile: [i8; 3],
    draw: bool,
}
impl Execution {
    pub fn new(room: i16, outdoors: bool) -> Self {
        Self {
            room,
            outdoors,
            stage: Stage::Begin,
            pending: None,
            position: [0; 2],
            tile: [0; 3],
            draw: false,
        }
    }
    pub fn next(
        &mut self,
        totals: &mut Totals,
        category8: &mut bool,
        input: &Input,
    ) -> Option<Request> {
        if let Some(request) = &self.pending {
            return Some(request.clone());
        }
        loop {
            let request = match self.stage {
                Stage::Begin => {
                    if input.hidden != 0 || input.room < 0 {
                        self.stage = Stage::Done;
                        continue;
                    }
                    totals.objects = totals.objects.wrapping_add(1);
                    match input.category {
                        4 => totals.category4 = totals.category4.wrapping_add(1),
                        5..=7 => totals.category5_to7 = totals.category5_to7.wrapping_add(1),
                        8 => *category8 = self.room as u16 & 1023 != 0,
                        _ => {}
                    }
                    if input.kind == 2 {
                        totals.people = totals.people.wrapping_add(1);
                    }
                    if input.flags & 0x400 != 0 {
                        Some(Request::Location)
                    } else {
                        self.stage = Stage::Finish;
                        None
                    }
                }
                Stage::Light(on_lot) => {
                    self.stage = Stage::Finish;
                    if on_lot {
                        if input.kind != 8 {
                            totals.lights = totals.lights.wrapping_add(1);
                        }
                        if input.contribution == 0 {
                            self.position = input.position;
                            self.draw = false;
                            self.stage = Stage::Clear;
                        } else {
                            let sum = if input.kind == 8 {
                                &mut totals.window_light
                            } else {
                                &mut totals.artificial_light
                            };
                            *sum = sum.wrapping_add(i32::from(input.contribution));
                            if input.light_mode == 1 {
                                self.position = input.position;
                                self.draw = true;
                                self.stage = Stage::Prepare;
                            }
                        }
                    }
                    None
                }
                Stage::Prepare => Some(Request::PrepareLight {
                    position: self.position,
                }),
                Stage::Clear => {
                    self.tile = [
                        (self.position[1] >> 4) as i8,
                        (self.position[0] >> 4) as i8,
                        input.level as i8,
                    ];
                    Some(Request::ClearLight {
                        tile: self.tile,
                        mode: input.light_mode,
                    })
                }
                Stage::Draw => Some(Request::DrawLight {
                    tile: self.tile,
                    mode: input.light_mode,
                }),
                Stage::Finish => {
                    totals.object_field39 =
                        totals.object_field39.wrapping_add(i32::from(input.field39));
                    if !self.outdoors || input.impact < 0 {
                        totals.impact = totals.impact.wrapping_add(i32::from(input.impact));
                    }
                    if input.impact >= 0 {
                        totals.good = totals.good.wrapping_add(1);
                    }
                    self.stage = Stage::Done;
                    None
                }
                Stage::Done => return None,
            };
            if request.is_some() {
                self.pending = request.clone();
                return request;
            }
        }
    }
    pub fn acknowledge(&mut self, response: Response) -> Result<(), String> {
        let stage = match (&self.pending, response) {
            (Some(Request::Location), Response::Located(on_lot)) => Stage::Light(on_lot),
            (Some(Request::PrepareLight { .. }), Response::Done) => Stage::Clear,
            (Some(Request::ClearLight { .. }), Response::Done) => {
                if self.draw {
                    Stage::Draw
                } else {
                    Stage::Finish
                }
            }
            (Some(Request::DrawLight { .. }), Response::Done) => Stage::Finish,
            _ => return Err("unexpected room object response".into()),
        };
        self.pending = None;
        self.stage = stage;
        Ok(())
    }
}
