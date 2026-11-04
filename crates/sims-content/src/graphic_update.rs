//! Services on a retained ordinary Animator. Native controllers own ordering;
//! this adapter owns assets and nested continuations. Object callbacks are
//! exposed to the lot host and never acknowledged merely by observing them.
use crate::{
    Index,
    character_geometry::MeshPool,
    graphic_initialization::Environment as Placement,
    graphic_loading::Graphic,
    graphic_materials::{Backend, GraphicMaterials},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_sim::pose::{
    animator::Event,
    appearance::{Execution, Input},
    appearance_selection::descriptor,
    graphic_appearance as appearance, graphic_frame as frame, graphic_transform as transform,
    graphic_visibility,
    model::Matrix,
    model_frame, model_material,
};
use sims_vm::{ObjectId, Program};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Command {
    /// 58486d reads the parent's Animator matrix; 585515 copies it. This is
    /// neither the scaled skeleton matrix nor an ongoing parent reference.
    BindTransform(Matrix),
    TransformControl(bool),
    Reload {
        force: bool,
    },
    Frame {
        entry: frame::Entry,
        only_visible: bool,
    },
}
pub struct Environment<'a> {
    pub placement: Placement<'a>,
    pub tick: i32,
    pub outside: bool,
    pub visibility_enabled: bool,
    pub camera: Matrix,
    /// Explicit CPU backend policy, not a recovered native device factory.
    pub materials: Backend,
    pub normal_material_mode: u32,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Request {
    Invalidate {
        object: ObjectId,
    },
    Position {
        object: ObjectId,
        position: [i32; 2],
    },
    Property {
        object: ObjectId,
        event: Event,
    },
    ReleaseBlend {
        object: ObjectId,
        model: u64,
        layer: u64,
    },
}
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    Progress,
    Pending(Request),
    Complete,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
enum Pass {
    Bind(Matrix),
    Control(bool),
    Reload(appearance::Reload),
    Frame(frame::Update),
    Done,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
enum Child {
    Placement(transform::Update),
    Model(model_frame::Update),
    Skin(model_frame::Skin),
    Material(model_material::Update),
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Update {
    object: ObjectId,
    pass: Pass,
    child: Option<Child>,
    pending: Option<Request>,
}
impl Update {
    pub fn new(object: ObjectId, command: Command) -> Self {
        Self {
            object,
            pass: match command {
                Command::BindTransform(matrix) => Pass::Bind(matrix),
                Command::TransformControl(value) => Pass::Control(value),
                Command::Reload { force } => Pass::Reload(appearance::Reload::new(force)),
                Command::Frame {
                    entry,
                    only_visible,
                } => Pass::Frame(frame::Update::new(entry, only_visible)),
            },
            child: None,
            pending: None,
        }
    }
    pub fn complete(&self) -> bool {
        matches!(self.pass, Pass::Done)
    }
    pub fn validate(&self, object: ObjectId) -> Result<()> {
        ensure!(
            object > 0 && self.object == object,
            "ordinary update owner differs"
        );
        match &self.pass {
            Pass::Reload(pass) => pass.validate().map_err(anyhow::Error::msg)?,
            Pass::Frame(pass) => pass.validate().map_err(anyhow::Error::msg)?,
            Pass::Bind(matrix) => ensure!(
                matrix.values.iter().all(|v| v.is_finite()),
                "invalid ordinary matrix"
            ),
            _ => (),
        }
        ensure!(
            matches!(
                (&self.pass, &self.child),
                (_, None)
                    | (Pass::Reload(_), Some(Child::Material(_)))
                    | (
                        Pass::Frame(_),
                        Some(Child::Placement(_) | Child::Model(_) | Child::Skin(_))
                    )
            ),
            "ordinary child continuation has no matching owner"
        );
        if let Some(request) = &self.pending {
            let id = match request {
                Request::Invalidate { object }
                | Request::Position { object, .. }
                | Request::Property { object, .. }
                | Request::ReleaseBlend { object, .. } => *object,
            };
            ensure!(
                id == object && matches!(self.pass, Pass::Frame(_)),
                "invalid ordinary callback owner"
            );
            ensure!(
                match (request, &self.child) {
                    (Request::Invalidate { .. } | Request::ReleaseBlend { .. }, None) => true,
                    (Request::Position { .. }, Some(Child::Placement(_))) => true,
                    (Request::Property { .. }, Some(Child::Model(_) | Child::Skin(_))) => true,
                    _ => false,
                },
                "ordinary callback has no matching continuation"
            );
        }
        if let Some(Child::Placement(pass)) = &self.child {
            pass.validate().map_err(anyhow::Error::msg)?;
        }
        if let Some(Child::Material(pass)) = &self.child {
            pass.validate().map_err(anyhow::Error::msg)?;
        }
        Ok(())
    }
    /// At most one asset/native callback boundary per call. Failed work keeps
    /// its pending native service so a save does not skip it or replay its prefix.
    pub fn advance(
        &mut self,
        index: &Index,
        program: &Program,
        graphic: &mut Graphic,
        pool: &mut MeshPool,
        env: Environment<'_>,
    ) -> Result<Step> {
        self.validate(graphic.ready.object)?;
        if let Some(pending) = &self.pending {
            return Ok(Step::Pending(pending.clone()));
        }
        let r = &mut graphic.ready;
        ensure!(
            r.appearance.dirty == r.frame.placement.dirty,
            "ordinary Animator dirty aliases differ"
        );
        let model = appearance::Model {
            model: self.object as u64,
            skin: self.object as u64,
        };
        match &mut self.pass {
            Pass::Done => return Ok(Step::Complete),
            Pass::Bind(matrix) => {
                r.frame.matrix = *matrix;
                r.bound.publish(*matrix).map_err(anyhow::Error::msg)?;
                self.pass = Pass::Done;
            }
            Pass::Control(value) => {
                r.frame.placement.explicit = *value;
                self.pass = Pass::Done;
            }
            Pass::Reload(pass) => {
                let request = pass
                    .advance(
                        &mut r.appearance,
                        env.placement.owner_appearance_index,
                        Some(model),
                    )
                    .map_err(anyhow::Error::msg)?;
                // Both representations refer to native Animator +d9.
                r.frame.placement.dirty = r.appearance.dirty;
                let Some(request) = request else {
                    self.pass = Pass::Done;
                    return Ok(Step::Complete);
                };
                let reply = match &request {
                    appearance::Request::Descriptor => appearance::Reply::Descriptor(descriptor(
                        r.profile
                            .get(1)
                            .context("ordinary appearance descriptor absent")?,
                        false,
                    )),
                    appearance::Request::Bind { skin, descriptor } => {
                        ensure!(*skin == model.skin, "ordinary skin replaced during bind");
                        let record = r.avatar.records.generic;
                        r.geometry.apply_record(
                            index,
                            &mut r.avatar,
                            record,
                            Execution::bind(Input::Replace {
                                name: descriptor.name.clone(),
                                texture: descriptor.texture.clone(),
                            }),
                            &mut r.bound,
                            pool,
                        )?;
                        // This CPU publication owner belongs to the new mesh set.
                        // It does not implement native device resource destruction.
                        graphic.materials = None;
                        appearance::Reply::Done
                    }
                    appearance::Request::Tint {
                        model: identity,
                        tint,
                    } => {
                        ensure!(
                            *identity == model.model,
                            "ordinary model replaced during tint"
                        );
                        if graphic.materials.is_none() {
                            graphic.materials = Some(GraphicMaterials::from_builtin_geometry(
                                &r.geometry,
                                &r.avatar,
                                [0; 3],
                                env.materials,
                            )?);
                        }
                        let child = self.child.get_or_insert_with(|| {
                            Child::Material(model_material::Update::new(*tint))
                        });
                        let Child::Material(child) = child else {
                            anyhow::bail!("ordinary material continuation differs")
                        };
                        if !graphic
                            .materials
                            .as_mut()
                            .unwrap()
                            .advance(child, env.normal_material_mode)?
                        {
                            return Ok(Step::Progress);
                        }
                        self.child = None;
                        appearance::Reply::Done
                    }
                };
                pass.acknowledge(&request, reply)
                    .map_err(anyhow::Error::msg)?;
            }
            Pass::Frame(pass) => {
                let Some(request) = pass
                    .advance(&mut r.frame, env.tick, Some(model))
                    .map_err(anyhow::Error::msg)?
                else {
                    self.pass = Pass::Done;
                    return Ok(Step::Complete);
                };
                let reply = match &request {
                    frame::Request::OutsideLot => frame::Reply::Outside(env.outside),
                    frame::Request::Viewport => frame::Reply::Viewport(env.placement.viewport),
                    frame::Request::Visibility { viewport } => {
                        let dirty = u8::from(r.frame.placement.dirty);
                        let visible = graphic_visibility::evaluate(
                            &mut r.frame,
                            dirty,
                            env.visibility_enabled,
                            Some(&r.bound),
                            *viewport,
                            || Ok(env.camera),
                        )
                        .map_err(anyhow::Error::msg)?;
                        frame::Reply::Visible(visible != 0)
                    }
                    frame::Request::Invalidate => {
                        self.pending = Some(Request::Invalidate {
                            object: self.object,
                        });
                        return Ok(Step::Pending(self.pending.clone().unwrap()));
                    }
                    frame::Request::ReleaseBlend { model, layer } => {
                        self.pending = Some(Request::ReleaseBlend {
                            object: self.object,
                            model: *model,
                            layer: *layer,
                        });
                        return Ok(Step::Pending(self.pending.clone().unwrap()));
                    }
                    frame::Request::Placement => {
                        let child = self
                            .child
                            .get_or_insert_with(|| Child::Placement(transform::Update::default()));
                        let Child::Placement(child) = child else {
                            anyhow::bail!("ordinary placement continuation differs")
                        };
                        if let Some(request) = child
                            .advance(
                                &mut r.frame.placement,
                                r.frame.matrix,
                                env.placement.source,
                                env.placement.view,
                                env.placement.terrain,
                            )
                            .map_err(anyhow::Error::msg)?
                        {
                            match request {
                                transform::Request::Publish(matrix) => {
                                    r.frame.matrix = matrix;
                                    r.bound.publish(matrix).map_err(anyhow::Error::msg)?;
                                    child
                                        .acknowledge(&mut r.frame.placement, &request)
                                        .map_err(anyhow::Error::msg)?;
                                }
                                transform::Request::Position(position) => {
                                    self.pending = Some(Request::Position {
                                        object: self.object,
                                        position,
                                    });
                                    return Ok(Step::Pending(self.pending.clone().unwrap()));
                                }
                            }
                        }
                        self.child = None;
                        frame::Reply::Done
                    }
                    frame::Request::Model {
                        model: identity,
                        matrix,
                        now,
                        visible,
                    } => {
                        ensure!(
                            *identity == model.model,
                            "ordinary model replaced during frame"
                        );
                        let child = self.child.get_or_insert_with(|| {
                            Child::Model(model_frame::Update::new(*matrix, *now, *visible, true))
                        });
                        let Child::Model(child) = child else {
                            anyhow::bail!("ordinary model continuation differs")
                        };
                        if let Some(request) = child
                            .next(&mut r.bound, program)
                            .map_err(anyhow::Error::msg)?
                        {
                            match &request {
                                model_frame::Request::Geometry => {
                                    r.geometry
                                        .state
                                        .refresh(&mut r.bound, &mut pool.meshes, false)
                                        .map_err(anyhow::Error::msg)?;
                                    child
                                        .acknowledge(&mut r.bound, &request)
                                        .map_err(anyhow::Error::msg)?;
                                    return Ok(Step::Progress);
                                }
                                model_frame::Request::Property(event) => {
                                    self.pending = Some(Request::Property {
                                        object: self.object,
                                        event: event.clone(),
                                    });
                                    return Ok(Step::Pending(self.pending.clone().unwrap()));
                                }
                            }
                        }
                        self.child = None;
                        frame::Reply::Done
                    }
                    frame::Request::Skin { skin, now, sample } => {
                        ensure!(*skin == model.skin, "ordinary skin replaced during frame");
                        let child = self.child.get_or_insert_with(|| {
                            Child::Skin(model_frame::Skin::new(*now, *sample != 0, true))
                        });
                        let Child::Skin(child) = child else {
                            anyhow::bail!("ordinary skin continuation differs")
                        };
                        if let Some(event) = child
                            .next(&mut r.bound, program)
                            .map_err(anyhow::Error::msg)?
                        {
                            self.pending = Some(Request::Property {
                                object: self.object,
                                event,
                            });
                            return Ok(Step::Pending(self.pending.clone().unwrap()));
                        }
                        self.child = None;
                        frame::Reply::Done
                    }
                    frame::Request::Hierarchy { skin } => {
                        ensure!(*skin == model.skin, "ordinary hierarchy skin replaced");
                        r.bound
                            .skeleton
                            .hierarchy
                            .update(&mut r.bound.skeleton.skin, false)
                            .map_err(anyhow::Error::msg)?;
                        frame::Reply::Done
                    }
                    frame::Request::Geometry { model: identity } => {
                        ensure!(*identity == model.model, "ordinary geometry model replaced");
                        r.geometry
                            .state
                            .refresh(&mut r.bound, &mut pool.meshes, false)
                            .map_err(anyhow::Error::msg)?;
                        frame::Reply::Done
                    }
                };
                pass.acknowledge(&mut r.frame, &request, reply)
                    .map_err(anyhow::Error::msg)?;
                r.appearance.dirty = r.frame.placement.dirty;
            }
        }
        Ok(if self.complete() {
            Step::Complete
        } else {
            Step::Progress
        })
    }
    /// The host calls this only after performing the requested world/layer
    /// callback. Wrong or repeated replies cannot advance the continuation.
    pub fn acknowledge(&mut self, graphic: &mut Graphic, request: &Request) -> Result<()> {
        self.validate(graphic.ready.object)?;
        ensure!(
            self.pending.as_ref() == Some(request),
            "stale ordinary graphic callback"
        );
        let Pass::Frame(pass) = &mut self.pass else {
            anyhow::bail!("ordinary callback outside frame")
        };
        let r = &mut graphic.ready;
        match request {
            Request::Invalidate { .. } => pass.acknowledge(
                &mut r.frame,
                &frame::Request::Invalidate,
                frame::Reply::Done,
            ),
            Request::ReleaseBlend { model, layer, .. } => pass.acknowledge(
                &mut r.frame,
                &frame::Request::ReleaseBlend {
                    model: *model,
                    layer: *layer,
                },
                frame::Reply::Done,
            ),
            Request::Position { position, .. } => {
                let Some(Child::Placement(child)) = &mut self.child else {
                    anyhow::bail!("ordinary position owner absent")
                };
                child.acknowledge(
                    &mut r.frame.placement,
                    &transform::Request::Position(*position),
                )
            }
            Request::Property { event, .. } => match &mut self.child {
                Some(Child::Model(child)) => {
                    child.acknowledge(&mut r.bound, &model_frame::Request::Property(event.clone()))
                }
                Some(Child::Skin(child)) => child.acknowledge(&mut r.bound, event),
                _ => anyhow::bail!("ordinary property owner absent"),
            },
        }
        .map_err(anyhow::Error::msg)?;
        self.pending = None;
        Ok(())
    }
}
