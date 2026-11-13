//! Recovered F6/E4/E2/F9/84/C2/DA branches of the eight original house listeners.
//! Device/UI work, the saved viewer owner and indicator construction remain
//! acknowledged services. These branches do not stand in for other messages.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
pub mod dispatch;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Receiver {
    #[default]
    Base,
    World,     //41637f
    Module,    //43c0e4
    Objects,   //44302e
    Globals,   //46d2c2
    Lot,       //47b22a
    Viewer,    //4a82db, secondary interface View+8
    Renderer,  //4c3ea5
    Interface, //4f65bf
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fields {
    pub viewer_interface: bool,
    pub include_visible: u8,
    pub renderer_dirty: u8,
    pub ui: Interface,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Interface {
    pub mask_owner_present: bool,
    pub panel_active: bool,
    pub shared_present: bool,
    pub gated: u8,
    /// Interface+8d, independent of the E4 gate at+8c.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub creation_gated: u8,
    pub dirty: u8,
}
fn is_zero(value: &u8) -> bool {
    *value == 0
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    ///4a82db(83): primary view virtual+48, then4b0615.
    ViewerInvalidate,
    ViewerTerrain,
    /// World41637f's full-width registry lookup and person visitor refresh.
    WorldCreated {
        value: u32,
    },
    ViewerRefresh,
    InitializeIndicators,
    Broadcast {
        event: u32,
        value: u32,
    },
    Compose {
        include_visible: u8,
    },
    ViewerZoom {
        value: u32,
    },
    ViewerRotation {
        value: u32,
    },
    UiFlags {
        bits: u32,
    },
    UiRefresh,
    PanelRefresh,
    SharedInterface,
    PanelBefore,
    PanelAfter,
    SpeedOverride {
        active: bool,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Request(Request),
    Panel,
    Shared,
    PanelGuard,
    Done(u16),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Call {
    receiver: Receiver,
    event: u16,
    value: u32,
    stage: Stage,
}
impl Call {
    /// The selected-person getter is evaluated by the live host when Objects
    /// receives F6. Both globals are borrowed from the actual VM, not copies.
    pub fn begin(
        receiver: Receiver,
        event: u32,
        value: u32,
        selected: bool,
        fields: &mut Fields,
        globals: &mut BTreeMap<i16, i16>,
    ) -> Result<Self, String> {
        let event = event as u16;
        if ![0xf6, 0xe4, 0xe2, 0xf9, 0x83, 0x84, 0xc2, 0x109, 0xda, 0xee].contains(&event) {
            return Err("house notification branch not implemented".into());
        }
        let stage = match (receiver, event) {
            (Receiver::Interface, 0x109) => {
                Stage::Request(Request::SpeedOverride { active: value != 0 })
            }
            (Receiver::Viewer, 0x83) => Stage::Request(Request::ViewerInvalidate),
            (_, 0x83) => Stage::Done(0),
            // All eight original house listeners ignore the transport notice.
            // Its broadcast still traverses the real registration order. The
            // control-panel pause setter is a separate caller-owned operation.
            (_, 0xc2 | 0x109) => Stage::Done(0),
            //4f65bf's EE branch checks only+8c and calls4ceceb(4).
            //It returns directly: unlike DA/E4 it does not call4ce41b.
            (Receiver::Interface, 0xee) => {
                if fields.ui.gated == 0 {
                    fields.ui.dirty = 1;
                    Stage::Request(Request::UiFlags { bits: 4 })
                } else {
                    Stage::Done(1)
                }
            }
            (_, 0xee) => Stage::Done(0),
            //4f65bf checks both bytes before setting dirty. Once the flag
            //callback returns, refresh runs without rereading either gate.
            (Receiver::Interface, 0xda) => {
                if fields.ui.gated == 0 && fields.ui.creation_gated == 0 {
                    fields.ui.dirty = 1;
                    Stage::Request(Request::UiFlags { bits: 0x4000 })
                } else {
                    Stage::Done(1)
                }
            }
            //In particular,4a82db returns zero here, not its handled fallback.
            (_, 0xda) => Stage::Done(0),
            (Receiver::World, 0xe2) => Stage::Request(Request::WorldCreated { value }),
            (Receiver::Interface, 0xe2) => {
                if fields.ui.creation_gated == 0 {
                    fields.ui.dirty = 1;
                }
                Stage::Done(1)
            }
            (Receiver::Viewer, 0xe2) => Stage::Done(0),
            (Receiver::Objects, 0xf6) if !selected => Stage::Request(Request::InitializeIndicators),
            (Receiver::Objects, 0xf6) => Stage::Done(1),
            (Receiver::Globals, 0xe4 | 0xf9) => {
                *globals
                    .get_mut(&(if event == 0xe4 { 3 } else { 2 }))
                    .ok_or("house notification global absent")? = value as i16;
                Stage::Done(1)
            }
            (Receiver::Viewer, 0xf6) => Stage::Request(Request::ViewerRefresh),
            (Receiver::Viewer, 0xe4) => Stage::Request(Request::Compose {
                include_visible: fields.include_visible,
            }),
            (Receiver::Viewer, 0xf9) if fields.viewer_interface => {
                Stage::Request(Request::ViewerZoom { value })
            }
            (Receiver::Viewer, 0x84) if fields.viewer_interface => {
                Stage::Request(Request::ViewerRotation { value })
            }
            (Receiver::Viewer, _) => Stage::Done(1),
            (Receiver::Renderer, 0x84) => {
                fields.renderer_dirty = 1;
                Stage::Done(1)
            }
            (Receiver::Interface, 0xf6) if fields.ui.mask_owner_present => {
                Stage::Request(Request::UiFlags { bits: 0x80000 })
            }
            (Receiver::Interface, 0xf6) => Stage::Panel,
            (Receiver::Interface, 0xe4) if fields.ui.gated == 0 => {
                fields.ui.dirty = 1;
                Stage::Request(Request::UiFlags { bits: 8 })
            }
            (Receiver::Interface, 0xe4) => Stage::Done(1),
            _ => Stage::Done(0),
        };
        Ok(Self {
            receiver,
            event,
            value,
            stage,
        })
    }
    pub fn validate(&self) -> Result<(), String> {
        let expected = match (self.receiver, self.event) {
            (Receiver::World | Receiver::Interface, 0xe2) => 1,
            (Receiver::Viewer, 0xf6 | 0xe4 | 0xf9 | 0x83 | 0x84)
            | (Receiver::Objects, 0xf6)
            | (Receiver::Globals, 0xe4 | 0xf9)
            | (Receiver::Renderer, 0x84)
            | (Receiver::Interface, 0xf6 | 0xe4 | 0xda | 0xee | 0x109) => 1,
            _ => 0,
        };
        let valid = match &self.stage {
            Stage::Done(n) => *n == expected,
            Stage::Panel | Stage::Shared | Stage::PanelGuard => {
                self.receiver == Receiver::Interface && self.event == 0xf6
            }
            Stage::Request(r) => match (self.receiver, self.event, r) {
                (Receiver::Viewer, 0x83, Request::ViewerInvalidate | Request::ViewerTerrain) => {
                    true
                }
                (Receiver::World, 0xe2, Request::WorldCreated { value }) => *value == self.value,
                (Receiver::Viewer, 0xf6, Request::ViewerRefresh)
                | (Receiver::Viewer, 0xe4, Request::Compose { .. })
                | (Receiver::Objects, 0xf6, Request::InitializeIndicators)
                | (
                    Receiver::Objects,
                    0xf6,
                    Request::Broadcast {
                        event: 0xe4,
                        value: 0,
                    },
                )
                | (Receiver::Interface, 0xf6, Request::UiFlags { bits: 0x80000 })
                | (Receiver::Interface, 0xe4, Request::UiFlags { bits: 8 } | Request::UiRefresh)
                | (Receiver::Interface, 0xee, Request::UiFlags { bits: 4 })
                | (
                    Receiver::Interface,
                    0xda,
                    Request::UiFlags { bits: 0x4000 } | Request::UiRefresh,
                )
                | (
                    Receiver::Interface,
                    0xf6,
                    Request::PanelRefresh
                    | Request::SharedInterface
                    | Request::PanelBefore
                    | Request::PanelAfter,
                ) => true,
                (Receiver::Viewer, 0xf9, Request::ViewerZoom { value })
                | (Receiver::Viewer, 0x84, Request::ViewerRotation { value }) => {
                    *value == self.value
                }
                (Receiver::Interface, 0x109, Request::SpeedOverride { active }) => {
                    *active == (self.value != 0)
                }
                _ => false,
            },
        };
        if !valid
            || ![0xf6, 0xe4, 0xe2, 0xf9, 0x83, 0x84, 0xc2, 0x109, 0xda, 0xee].contains(&self.event)
        {
            return Err("invalid house notification continuation".into());
        }
        Ok(())
    }
    pub fn advance(
        &mut self,
        fields: &Fields,
        globals: &BTreeMap<i16, i16>,
    ) -> Result<Option<&Request>, String> {
        self.validate()?;
        loop {
            self.stage = match self.stage {
                Stage::Panel if fields.ui.panel_active => Stage::Request(Request::PanelRefresh),
                Stage::Panel => Stage::Shared,
                Stage::Shared if fields.ui.shared_present => {
                    Stage::Request(Request::SharedInterface)
                }
                Stage::Shared => Stage::PanelGuard,
                // The globals array starts at+10; +44 is word26. Keep this
                // live borrowed value, without inferring what it means.
                Stage::PanelGuard => {
                    if *globals.get(&26).ok_or("interface global26 absent")? != 0 {
                        Stage::Request(Request::PanelBefore)
                    } else {
                        Stage::Done(1)
                    }
                }
                _ => break,
            };
        }
        Ok(self.pending())
    }
    pub fn pending(&self) -> Option<&Request> {
        if let Stage::Request(r) = &self.stage {
            Some(r)
        } else {
            None
        }
    }
    pub fn returned(&self) -> Option<u16> {
        if let Stage::Done(r) = self.stage {
            Some(r)
        } else {
            None
        }
    }
    pub fn acknowledge(&mut self, request: &Request) -> Result<(), String> {
        self.validate()?;
        if self.pending() != Some(request) {
            return Err("house notification acknowledgement mismatch".into());
        }
        self.stage = match request {
            Request::ViewerInvalidate => Stage::Request(Request::ViewerTerrain),
            Request::InitializeIndicators => Stage::Request(Request::Broadcast {
                event: 0xe4,
                value: 0,
            }),
            Request::UiFlags { .. } if self.event == 0xf6 => Stage::Panel,
            Request::UiFlags { bits: 4 } if self.event == 0xee => Stage::Done(1),
            Request::UiFlags { .. } => Stage::Request(Request::UiRefresh),
            Request::PanelRefresh => Stage::Shared,
            Request::SharedInterface => Stage::PanelGuard,
            Request::PanelBefore => Stage::Request(Request::PanelAfter),
            _ => Stage::Done(1),
        };
        Ok(())
    }
    pub fn receiver(&self) -> Receiver {
        self.receiver
    }
    pub fn event(&self) -> u16 {
        self.event
    }
    pub fn value(&self) -> u32 {
        self.value
    }
}

#[cfg(test)]
mod speed_override_tests {
    use super::*;

    fn fields() -> Fields {
        Fields {
            viewer_interface: true,
            include_visible: 1,
            renderer_dirty: 0,
            ui: Interface {
                mask_owner_present: true,
                panel_active: true,
                shared_present: true,
                gated: 1,
                creation_gated: 1,
                dirty: 0,
            },
        }
    }

    #[test]
    fn native_speed_override_requires_exact_interface_service_across_restore() {
        for value in [0, 1, 256, u32::MAX] {
            let mut fields = fields();
            let before = fields.clone();
            let mut globals = BTreeMap::new();
            let mut call = Call::begin(
                Receiver::Interface,
                0x109,
                value,
                false,
                &mut fields,
                &mut globals,
            )
            .unwrap();
            let request = Request::SpeedOverride { active: value != 0 };
            assert_eq!(call.advance(&fields, &globals).unwrap(), Some(&request));
            assert_eq!(call.returned(), None);
            assert_eq!(fields, before);
            call = serde_json::from_slice(&serde_json::to_vec(&call).unwrap()).unwrap();
            call.validate().unwrap();
            let saved = call.clone();
            assert!(
                call.acknowledge(&Request::SpeedOverride { active: value == 0 })
                    .is_err()
            );
            assert_eq!(call, saved);
            let mut malformed = serde_json::to_value(&call).unwrap();
            malformed["stage"]["Request"]["SpeedOverride"]["active"] = (value == 0).into();
            assert!(
                serde_json::from_value::<Call>(malformed)
                    .unwrap()
                    .validate()
                    .is_err()
            );
            call.acknowledge(&request).unwrap();
            assert_eq!(call.returned(), Some(1));
            call.validate().unwrap();
            assert_eq!(fields, before);
            assert!(globals.is_empty());
        }
    }

    #[test]
    fn native_speed_override_other_house_listeners_return_unhandled() {
        for receiver in [
            Receiver::Base,
            Receiver::World,
            Receiver::Module,
            Receiver::Objects,
            Receiver::Globals,
            Receiver::Lot,
            Receiver::Viewer,
            Receiver::Renderer,
        ] {
            let mut fields = fields();
            let before = fields.clone();
            let mut globals = BTreeMap::new();
            let call = Call::begin(receiver, 0x109, 1, true, &mut fields, &mut globals).unwrap();
            call.validate().unwrap();
            assert_eq!(call.returned(), Some(0));
            assert!(call.pending().is_none());
            assert_eq!(fields, before);
            assert!(globals.is_empty());
        }
    }
}
