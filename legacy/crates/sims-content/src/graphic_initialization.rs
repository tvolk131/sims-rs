//! Fresh ordinary 3D Animator initialization using actual profile/model assets.
//! The caller supplies the definition's resolved StringSet and view/physical
//! inputs. Definition-cache population and the lot's factory hook are separate.
use crate::{
    Index,
    character_geometry::{CharacterGeometry, MeshPool},
    saved_avatar::{BindingState, BoundAvatar},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_sim::pose::{
    animator::{Settings, Slot},
    appearance,
    appearance_selection::descriptor,
    graphic_appearance as appearance_reload, graphic_frame, graphic_initialize as init,
    graphic_records as records, graphic_reset, graphic_transform as placement, graphic_visibility,
    model::{Bound, Matrix},
    ordinary, transform,
};
use sims_vm::{Definition, ObjectId};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Saved {
    object: ObjectId,
    definition: Definition,
    profile: Vec<String>,
    state: init::State,
    update: init::Update,
    frame: graphic_frame::State,
    appearance: appearance_reload::State,
    vectors: records::Vectors,
    generic: appearance::Record,
    callbacks: BTreeMap<u64, Option<u64>>,
    engaged: i32,
    settings: Settings,
    skeleton: Option<String>,
    bound: Option<Bound>,
    geometry: Option<CharacterGeometry>,
    binding: Option<BindingState>,
}
#[derive(Clone)]
pub struct Prepared {
    state: Saved,
    avatar: Option<BoundAvatar>,
}
pub struct Environment<'a> {
    /// Live owner object-data word 68 (+d2), separate from Animator cache +9c.
    pub owner_appearance_index: i16,
    pub source: placement::Source,
    pub view: transform::View,
    pub terrain: transform::Terrain<'a>,
    pub viewport: [i32; 4],
}
#[derive(Clone)]
pub struct Ready {
    pub object: ObjectId,
    pub definition: Definition,
    pub profile: Vec<String>,
    pub frame: graphic_frame::State,
    pub appearance: appearance_reload::State,
    pub bound: Bound,
    pub geometry: CharacterGeometry,
    pub avatar: BoundAvatar,
}
impl Prepared {
    /// Use the definition-owned, source-backed ordinary StringSet cache.
    pub fn begin_cached(
        object: ObjectId,
        profile: &crate::graphic_profiles::Cached,
        words: &[i16],
        tick: i32,
        settings: Settings,
    ) -> Result<Self> {
        Self::begin(
            object,
            profile.definition().clone(),
            words,
            profile
                .strings()?
                .context("person definition has no ordinary profile")?
                .to_vec(),
            tick,
            settings,
        )
    }
    /// Fresh 4376f6 branch only. The resolved profile is an explicit input,
    /// not a synthesized fallback or an implicit read from person BodyStrings.
    pub fn begin(
        object: ObjectId,
        definition: Definition,
        words: &[i16],
        profile: Vec<String>,
        tick: i32,
        settings: Settings,
    ) -> Result<Self> {
        ensure!(
            object > 0
                && words.get(9).is_some_and(|v| *v != 2)
                && words.get(0xcc / 2).is_some_and(|v| *v & 1 != 0),
            "ordinary 3D object definition required"
        );
        let state = Saved {
            object,
            definition,
            profile,
            state: init::State {
                owner: 0,
                model: None,
                profile: None,
                flags: [1; 4],
                appearance_index: -1,
                selection: -1,
                foreground: None,
            },
            update: init::Update::new(object as u64, init::Entry::Owner(object as u64)),
            frame: graphic_frame::State {
                clock_mode: 0,
                now: ordinary::clock(tick, 0),
                visible: 0,
                blend: None,
                placement: placement::Placement::constructed(),
                matrix: Matrix::IDENTITY,
            },
            appearance: appearance_reload::State::constructed(),
            vectors: records::Vectors {
                generic: vec![0],
                accessories: vec![],
            },
            generic: Default::default(),
            callbacks: BTreeMap::new(),
            engaged: 0,
            settings,
            skeleton: None,
            bound: None,
            geometry: None,
            binding: None,
        };
        Ok(Self {
            state,
            avatar: None,
        })
    }
    pub fn complete(&self) -> bool {
        self.state.update.complete()
    }
    pub fn validate_owner(
        &self,
        object: ObjectId,
        definition: &Definition,
        profile: &[String],
    ) -> Result<()> {
        ensure!(
            self.state.object == object
                && &self.state.definition == definition
                && self.state.profile == profile,
            "ordinary initializer differs from its registered owner/profile"
        );
        Ok(())
    }
    pub fn snapshot(&self) -> Saved {
        let mut saved = self.state.clone();
        saved.binding = self.avatar.as_ref().map(BoundAvatar::snapshot);
        saved
    }
    pub fn restore(index: &Index, saved: Saved) -> Result<Self> {
        saved.update.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            saved.object > 0
                && (saved.state.owner == saved.object as u64 || saved.state.owner == 0),
            "graphic initializer owner mismatch"
        );
        let identity = saved.object as u64;
        ensure!(
            saved.state.profile.is_none_or(|id| id == identity)
                && saved.state.model.is_none_or(|id| id == identity),
            "graphic initializer resource/model identity mismatch"
        );
        ensure!(
            saved.callbacks.len() == usize::from(saved.state.model.is_some())
                && saved
                    .callbacks
                    .iter()
                    .all(|(id, owner)| *id == identity && owner.is_none_or(|id| id == identity)),
            "graphic initializer callback storage mismatch"
        );
        ensure!(
            saved.bound.is_some() == saved.geometry.is_some()
                && saved.bound.is_some() == saved.binding.is_some()
                && saved.bound.is_some() == saved.skeleton.is_some(),
            "graphic initializer lost model assets"
        );
        ensure!(
            saved.bound.is_none() || saved.state.model.is_some(),
            "graphic initializer lost model identity"
        );
        ensure!(
            !saved.update.complete() || saved.bound.is_some(),
            "completed graphic initializer lost model"
        );
        let avatar = if let Some(binding) = &saved.binding {
            let avatar = BoundAvatar::restore(
                index,
                saved.skeleton.as_ref().unwrap(),
                saved.object as u64,
                binding,
            )?;
            ensure!(
                avatar.avatar.skeleton.bones.len()
                    == saved.bound.as_ref().unwrap().skeleton.skin.bones.len(),
                "graphic initializer skeleton changed"
            );
            ensure!(
                avatar.records.entries.get(avatar.records.generic) == Some(&saved.generic),
                "graphic initializer generic record changed"
            );
            Some(avatar)
        } else {
            None
        };
        Ok(Self {
            state: saved,
            avatar,
        })
    }
    /// One completed outer service per call. Snapshots between calls retain
    /// the real initialized model and CRT draw position owned by the caller.
    pub fn advance(
        &mut self,
        index: &Index,
        crt: &mut u32,
        pool: &mut MeshPool,
        env: Environment<'_>,
    ) -> Result<bool> {
        let s = &mut self.state;
        let Some(request) = s
            .update
            .advance(&mut s.state, &mut s.callbacks)
            .map_err(anyhow::Error::msg)?
        else {
            return Ok(true);
        };
        s.frame.placement.dirty = s.state.flags[1] != 0;
        let identity = s.object as u64;
        let reply = match &request {
            init::Request::ClearAppearances => {
                ensure!(s.bound.is_none(), "fresh graphic clear after model binding");
                let mut walk = records::Update::new(records::Entry::Clear);
                while let Some(request) = walk
                    .advance(&mut s.vectors, None)
                    .map_err(anyhow::Error::msg)?
                {
                    ensure!(
                        request == records::Request::Clear(0),
                        "unexpected fresh graphic record"
                    );
                    let mut clear = appearance::Execution::clear();
                    ensure!(
                        clear.next(&mut s.generic).is_none(),
                        "fresh record unexpectedly has a binding"
                    );
                    walk.acknowledge(&mut s.vectors, &request)
                        .map_err(anyhow::Error::msg)?;
                }
                init::Reply::Done
            }
            init::Request::Profile { owner } => {
                ensure!(*owner == identity);
                init::Reply::Profile(Some(identity))
            }
            init::Request::ReloadAppearance { force } => {
                ensure!(
                    s.bound.is_none(),
                    "fresh graphic profile unexpectedly has a model"
                );
                s.appearance.cached = s.state.appearance_index;
                s.appearance.dirty = s.state.flags[1] != 0;
                let mut reload = appearance_reload::Reload::new(*force);
                while let Some(request) = reload
                    .advance(&mut s.appearance, env.owner_appearance_index, None)
                    .map_err(anyhow::Error::msg)?
                {
                    ensure!(request == appearance_reload::Request::Descriptor);
                    let text = s
                        .profile
                        .get(1)
                        .context("ordinary graphic descriptor absent")?;
                    reload
                        .acknowledge(
                            &request,
                            appearance_reload::Reply::Descriptor(descriptor(text, false)),
                        )
                        .map_err(anyhow::Error::msg)?;
                }
                s.state.appearance_index = s.appearance.cached;
                s.state.flags[1] = u8::from(s.appearance.dirty);
                init::Reply::Done
            }
            init::Request::DestroyAccessories => {
                let mut walk = records::Update::new(records::Entry::DestroyAccessories);
                ensure!(
                    walk.advance(&mut s.vectors, None)
                        .map_err(anyhow::Error::msg)?
                        .is_none(),
                    "fresh graphic has accessories to destroy"
                );
                init::Reply::Done
            }
            init::Request::Skeleton { profile } => {
                ensure!(*profile == identity);
                init::Reply::Skeleton(
                    s.profile
                        .first()
                        .context("ordinary graphic skeleton entry absent")?
                        .clone(),
                )
            }
            init::Request::ConstructModel => {
                ensure!(s.callbacks.insert(identity, None).is_none());
                init::Reply::Model(identity)
            }
            init::Request::BindModel { model, skeleton } => {
                ensure!(*model == identity && s.bound.is_none());
                let mut avatar = BoundAvatar::empty(index, skeleton, identity)?;
                // BoundAvatar retains unused person metadata slots as host-only
                // storage. The native ordinary vector still contains one record.
                avatar.records.entries[avatar.records.generic] = s.generic.clone();
                let mut next_crt = *crt;
                let bound = if skeleton.eq_ignore_ascii_case("root") {
                    Bound::root(&mut next_crt)
                } else {
                    Bound::new(&avatar.avatar.skeleton, &mut next_crt)
                }
                .map_err(anyhow::Error::msg)?;
                s.geometry = Some(CharacterGeometry::new(avatar.avatar.skeleton.bones.len()));
                s.bound = Some(bound);
                s.skeleton = Some(skeleton.clone());
                self.avatar = Some(avatar);
                *crt = next_crt;
                init::Reply::Bound(0)
            }
            init::Request::Reset => {
                let bound = s.bound.as_mut().context("graphic Reset model absent")?;
                graphic_reset::apply(
                    bound,
                    &mut s.frame.placement,
                    &mut s.state.flags,
                    &mut s.engaged,
                    s.frame.now,
                    s.settings,
                )
                .map_err(anyhow::Error::msg)?;
                s.state.selection = bound.skeleton.skin.animator.selection();
                s.state.foreground = bound
                    .skeleton
                    .skin
                    .animator
                    .slot(Slot::Foreground)
                    .map(|id| id.0);
                init::Reply::Done
            }
            init::Request::BindAccessories => {
                let mut walk = records::Update::new(records::Entry::BindAccessories);
                while let Some(request) = walk
                    .advance(
                        &mut s.vectors,
                        Some(appearance_reload::Model {
                            model: identity,
                            skin: identity,
                        }),
                    )
                    .map_err(anyhow::Error::msg)?
                {
                    match request {
                        records::Request::Bind { record, skin } => {
                            ensure!(skin == identity);
                            s.geometry
                                .as_mut()
                                .context("graphic geometry absent")?
                                .apply_record(
                                    index,
                                    self.avatar.as_mut().context("graphic assets absent")?,
                                    record,
                                    appearance::Execution::bind(appearance::Input::Current),
                                    s.bound.as_mut().unwrap(),
                                    pool,
                                )?;
                        }
                        records::Request::Invalidate { skin } => {
                            ensure!(skin == identity);
                            s.bound
                                .as_mut()
                                .context("graphic skin absent")?
                                .refresh_empty_accessories();
                        }
                        _ => anyhow::bail!("unexpected graphic accessory service"),
                    }
                    walk.acknowledge(&mut s.vectors, &request)
                        .map_err(anyhow::Error::msg)?;
                }
                init::Reply::Done
            }
            init::Request::Placement => {
                let mut update = placement::Update::default();
                let request = update
                    .advance(
                        &mut s.frame.placement,
                        s.frame.matrix,
                        env.source,
                        env.view,
                        env.terrain,
                    )
                    .map_err(anyhow::Error::msg)?
                    .context("graphic placement absent")?;
                match &request {
                    placement::Request::Publish(matrix) => {
                        s.bound
                            .as_mut()
                            .context("graphic placement model absent")?
                            .publish(*matrix)
                            .map_err(anyhow::Error::msg)?;
                        s.frame.matrix = *matrix;
                    }
                    placement::Request::Position(_) => {} // actual base cXObject +78 is a no-op.
                }
                update
                    .acknowledge(&mut s.frame.placement, &request)
                    .map_err(anyhow::Error::msg)?;
                init::Reply::Done
            }
            init::Request::CameraViewport => init::Reply::Viewport(env.viewport),
            init::Request::Visibility { viewport } => {
                // Actual 585beb fresh-object branch: copy dirty byte and return.
                // A clean object's projection/culling must use its own service.
                ensure!(
                    s.state.flags[1] != 0,
                    "clean graphic requires native visibility projection"
                );
                graphic_visibility::evaluate(
                    &mut s.frame,
                    s.state.flags[1],
                    true,
                    s.bound.as_ref(),
                    *viewport,
                    || Err("fresh initializer unexpectedly requested a projection camera".into()),
                )
                .map_err(anyhow::Error::msg)?;
                init::Reply::Done
            }
        };
        s.update
            .acknowledge(&mut s.state, &request, reply)
            .map_err(anyhow::Error::msg)?;
        s.frame.placement.dirty = s.state.flags[1] != 0;
        s.appearance.cached = s.state.appearance_index;
        s.appearance.dirty = s.state.flags[1] != 0;
        Ok(s.update.complete())
    }
    pub fn into_ready(self) -> Result<Ready> {
        ensure!(
            self.complete() && self.state.update.bind_status() == Some(0),
            "graphic initialization incomplete"
        );
        let s = self.state;
        let identity = s.object as u64;
        ensure!(
            s.callbacks.get(&identity) == Some(&Some(identity)),
            "graphic callback owner absent"
        );
        Ok(Ready {
            object: s.object,
            definition: s.definition,
            profile: s.profile,
            frame: s.frame,
            appearance: s.appearance,
            bound: s.bound.context("initialized graphic model absent")?,
            geometry: s.geometry.context("initialized graphic geometry absent")?,
            avatar: self.avatar.context("initialized graphic assets absent")?,
        })
    }
}
