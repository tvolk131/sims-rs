use super::*;

pub(super) const POLICY: &str = "installed-houses-ascending-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Choice {
    InstalledHouses,
    BuyPreview,
    InferredRouting,
}

impl Choice {
    pub fn label(self) -> &'static str {
        match self {
            Self::InstalledHouses => "Installed-house numerical order",
            Self::BuyPreview => "Experimental Buy frontend",
            Self::InferredRouting => "Unverified inferred rectangle routing",
        }
    }

    pub fn warning(self) -> &'static str {
        match self {
            Self::InstalledHouses => {
                "Changes gameplay and visitor ordering. Includes installed vacant/reserved slots; excludes this lot. Numerical order, NOT original proximity; no original-parity claim."
            }
            Self::BuyPreview => {
                "Initializes the experimental replacement Buy frontend on a fresh household. Changes frontend ownership; not a parity-neutral option or original-parity claim."
            }
            Self::InferredRouting => {
                "Enables the existing unverified inferred rectangle routing on a fresh household. Changes gameplay routing; no original-parity claim. Never changes a retained session in place."
            }
        }
    }

    pub fn stage(self, args: &mut Args) {
        match self {
            Self::InstalledHouses => {
                args.neighborhood_policy = Some(POLICY.into());
                args.neighborhood_observation = None;
                args.neighborhood_summary = None;
                args.neighborhood_sample = 0;
            }
            Self::BuyPreview => args.buy_preview = true,
            Self::InferredRouting => args.inferred_routing = true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Confirmation {
    pub choice: Choice,
    pub revision: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub(super) struct Snapshot {
    pub provenance: String,
    pub neighborhood: Option<String>,
    pub lot: Option<i16>,
    pub count: Option<usize>,
    pub manifest_sha256: Option<String>,
    pub routing: Option<String>,
    pub buy_frontend: Option<bool>,
    pub requested_policy: Option<String>,
    pub requested_buy_preview: bool,
    pub requested_inferred_routing: bool,
    pub notice: String,
}

impl Snapshot {
    pub fn loading(args: &Args) -> Self {
        Self {
            provenance: "loading".into(),
            neighborhood: None,
            lot: None,
            count: None,
            manifest_sha256: None,
            routing: None,
            buy_frontend: None,
            requested_policy: args.neighborhood_policy.clone(),
            requested_buy_preview: args.buy_preview,
            requested_inferred_routing: args.inferred_routing,
            notice: "Neighborhood input: Loading".into(),
        }
    }

    pub fn capture(session: &Session, args: &Args) -> Result<Self> {
        let mut out = Self::loading(args);
        out.neighborhood = Some(session.house.neighborhood_source.clone());
        out.lot = Some(session.house.number);
        out.routing = Some(match session.options.routing {
            sims_sim::lot_route::path::Policy::Strict => "Strict".into(),
            sims_sim::lot_route::path::Policy::InferredRectangles => {
                "Inferred rectangles (unverified; no original-parity claim)".into()
            }
        });
        out.buy_frontend = Some(session.host.presentation.tool_selection.is_some());
        let label = out.describe(session.neighborhood_input_status()?);
        out.notice = format!(
            "Neighborhood input: {label} [{} · lot {}{}]",
            session.house.neighborhood_source,
            session.house.number,
            out.count
                .map(|count| format!(" · {count} candidates"))
                .unwrap_or_default(),
        );
        Ok(out)
    }

    fn describe(
        &mut self,
        status: sims_runtime::original_session::neighborhood_input::Status<'_>,
    ) -> &'static str {
        use sims_runtime::original_session::neighborhood_input::Status;
        self.count = None;
        self.manifest_sha256 = None;
        match status {
            Status::Missing => {
                self.provenance = "missing".into();
                "Missing — Play can fault when neighborhood input is required; use Application Options to configure and restart."
            }
            Status::ScopedCapture { count, .. } => {
                self.provenance = "scoped-capture".into();
                self.count = Some(count);
                "Scoped capture — supplied observation only, not universal native input."
            }
            Status::Replacement(replacement) => {
                self.provenance = replacement.policy_id().into();
                self.count = Some(replacement.candidate_count());
                self.manifest_sha256 = Some(replacement.manifest_hash().into());
                "Replacement: installed houses, numerical order — includes installed vacant/reserved slots; NOT original proximity; no original-parity claim."
            }
            Status::PreexistingVector { count, .. } => {
                self.provenance = "preexisting-vector-provenance-unknown".into();
                self.count = Some(count);
                "Preexisting vector/provenance unknown — no inferred native claim."
            }
        }
    }
}

pub(super) fn bind_policy(session: &mut Session, args: &Args, fresh: bool) -> Result<()> {
    let Some(policy) = args.neighborhood_policy.as_deref() else {
        return Ok(());
    };
    ensure!(
        policy == POLICY,
        "unsupported neighborhood replacement policy: {policy}"
    );
    ensure!(
        args.neighborhood_observation.is_none()
            && args.neighborhood_summary.is_none()
            && args.neighborhood_sample == 0,
        "neighborhood replacement conflicts with capture input"
    );
    session.neighborhood_input_status()?;
    if let Some(retained) = &session.host.presentation.neighborhood_replacement {
        ensure!(
            retained.policy_id() == policy,
            "requested neighborhood policy conflicts with retained replacement"
        );
        return Ok(());
    }
    ensure!(
        session.host.presentation.neighborhood_input.is_none(),
        "requested neighborhood policy conflicts with retained capture"
    );
    ensure!(
        session
            .simulation
            .vm
            .world
            .person
            .as_ref()
            .and_then(|people| people.nearby_houses.as_ref())
            .is_none(),
        "requested neighborhood policy conflicts with a preexisting vector of unknown provenance"
    );
    ensure!(
        fresh,
        "replacement policy requires a fresh Ready boundary; restart instead of changing a retained session"
    );
    session.bind_installed_houses_ascending_v1()
}

pub(super) fn controls(
    snapshot: &Snapshot,
    confirmation: Option<Confirmation>,
    enabled: bool,
) -> Element<'_, Message> {
    let mut content = widget::column![
        text("Application-owned experimental choices (not native Options)").size(14),
        text(&snapshot.notice).size(12),
        text(format!(
            "Retained routing: {} · Buy frontend: {}",
            snapshot.routing.as_deref().unwrap_or("Loading"),
            snapshot
                .buy_frontend
                .map(|bound| if bound {
                    "initialized"
                } else {
                    "not initialized"
                })
                .unwrap_or("Loading")
        ))
        .size(12),
    ]
    .spacing(6);
    if let Some(pending) = confirmation {
        content = content.push(text(format!("{} and RESTART?", pending.choice.label())).size(16))
            .push(text("Discards unsaved progress and starts a fresh household; does not repair or continue a faulted VM.").size(12))
            .push(text(pending.choice.warning()).size(12))
            .push(row![
                button("Confirm and restart").on_press(Message::ConfirmApplicationChoice(pending)),
                button("Cancel").on_press(Message::CancelApplicationChoice(pending)),
            ].spacing(8));
    } else {
        for choice in [
            Choice::InstalledHouses,
            Choice::BuyPreview,
            Choice::InferredRouting,
        ] {
            content = content.push(
                button(text(format!("Configure {}…", choice.label())).size(12))
                    .on_press_maybe(enabled.then_some(Message::RequestApplicationChoice(choice))),
            );
        }
    }
    widget_lifetimes::retain_owner_key(
        (confirmation, enabled),
        container(content)
            .padding(12)
            .style(container::rounded_box)
            .into(),
    )
}

pub(super) fn notice(snapshot: &Snapshot) -> Element<'_, Message> {
    container(text(&snapshot.notice).size(12))
        .max_width(650)
        .padding(8)
        .into()
}

pub(super) fn recovery(view: Element<'_, Message>, faulted: bool) -> Element<'_, Message> {
    if !faulted {
        return view;
    }
    widget::stack![
        view,
        container(
            row![
                text("Application recovery (faulted VM stays stopped)").size(12),
                button("Load pre-fault save").on_press(Message::Load),
                button("Restart fresh household").on_press(Message::Restart),
            ]
            .spacing(8)
        )
        .width(Fill)
        .align_x(iced::alignment::Horizontal::Right)
        .padding(8)
    ]
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use sims_runtime::original_session::neighborhood_input::Status;

    #[test]
    fn cli_replacement_is_explicit_and_capture_modes_are_exclusive() {
        let mut defaults = Args::try_parse_from(["sims-room"]).unwrap();
        defaults.select_default_house();
        assert!(defaults.neighborhood_policy.is_none());
        assert!(!defaults.buy_preview && !defaults.inferred_routing);
        let mut selected =
            Args::try_parse_from(["sims-room", "--neighborhood-policy", POLICY]).unwrap();
        selected.select_default_house();
        assert_eq!(selected.original, defaults.original);
        assert_eq!(selected.neighborhood_policy.as_deref(), Some(POLICY));
        for extra in [
            vec!["--neighborhood-sample", "0"],
            vec!["--neighborhood-sample", "1"],
            vec![
                "--neighborhood-observation",
                "log",
                "--neighborhood-summary",
                "summary",
            ],
            vec!["--neighborhood-summary", "summary"],
            vec!["--laboratory"],
        ] {
            let mut argv = vec![
                "sims-room",
                "--original",
                "UserData/Houses/House07.iff",
                "--neighborhood-policy",
                POLICY,
            ];
            argv.extend(extra);
            assert!(Args::try_parse_from(argv).is_err());
        }
        assert!(Args::try_parse_from(["sims-room", "--neighborhood-policy", "native"]).is_err());
        assert!(
            Args::try_parse_from([
                "sims-room",
                "--original",
                "UserData/Houses/House07.iff",
                "--load",
                "managed.json",
                "--neighborhood-policy",
                POLICY
            ])
            .is_ok()
        );
    }

    #[test]
    fn choices_stage_only_the_requested_configuration() {
        let mut args = Args::try_parse_from([
            "sims-room",
            "--paused",
            "--floor",
            "2",
            "--save",
            "own.json",
        ])
        .unwrap();
        Choice::BuyPreview.stage(&mut args);
        assert!(args.buy_preview && !args.inferred_routing);
        assert!(args.neighborhood_policy.is_none());
        Choice::InferredRouting.stage(&mut args);
        assert!(args.inferred_routing && args.buy_preview);
        Choice::InstalledHouses.stage(&mut args);
        assert_eq!(args.neighborhood_policy.as_deref(), Some(POLICY));
        assert!(args.paused && args.buy_preview && args.inferred_routing);
        assert_eq!(args.floor, 2);
        assert_eq!(args.save, std::path::PathBuf::from("own.json"));
    }

    #[test]
    fn status_does_not_infer_native_provenance_or_replace_missing_with_empty() {
        let args = Args::try_parse_from(["sims-room"]).unwrap();
        let mut snapshot = Snapshot::loading(&args);
        assert!(snapshot.notice.contains("Loading"));
        assert!(
            snapshot
                .describe(Status::Missing)
                .contains("Play can fault")
        );
        assert!(snapshot.count.is_none());
        let label = snapshot.describe(Status::PreexistingVector {
            house: "UserData/Houses/House07.iff",
            neighborhood: "UserData/Neighborhood.iff",
            lot: 7,
            count: 0,
        });
        assert!(label.contains("provenance unknown"));
        assert_eq!(snapshot.count, Some(0));
        assert_ne!(snapshot.provenance, "scoped-capture");
        let label = snapshot.describe(Status::ScopedCapture {
            house: "UserData/Houses/House07.iff",
            neighborhood: "UserData/Neighborhood.iff",
            lot: 7,
            count: 2,
        });
        assert!(label.contains("Scoped capture") && label.contains("not universal"));
        assert!(
            Choice::InstalledHouses
                .warning()
                .contains("vacant/reserved")
        );
        assert!(
            Choice::InstalledHouses
                .warning()
                .contains("NOT original proximity")
        );
        assert!(Choice::InferredRouting.warning().contains("unverified"));
    }

    #[test]
    fn player_save_rejects_application_and_menu_errors_before_refresh_or_publication() -> Result<()>
    {
        let path = std::env::temp_dir().join(format!(
            "sims-player-save-guard-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        let mut args = Args::try_parse_from(["sims-room"])?;
        args.save = path.clone();
        let (mut viewer, task) = Viewer::new(args);
        drop(task);
        std::fs::write(&path, b"last-known-good-player-save")?;
        for application_error in [true, false] {
            viewer.error = application_error.then(|| "synthetic application error".into());
            viewer.menu_error = (!application_error).then(|| "synthetic menu error".into());
            let before = (
                viewer.last,
                viewer.widget_epoch,
                viewer.playing,
                viewer.accumulator,
                viewer.status.clone(),
            );
            let error = viewer
                .save()
                .expect_err("Save with error must fail before refresh");
            assert!(format!("{error:#}").contains(if application_error {
                "application error"
            } else {
                "menu error"
            }));
            assert_eq!(std::fs::read(&path)?, b"last-known-good-player-save");
            assert_eq!(
                before,
                (
                    viewer.last,
                    viewer.widget_epoch,
                    viewer.playing,
                    viewer.accumulator,
                    viewer.status.clone()
                )
            );
            assert!(viewer.session.is_none() && viewer.client.is_none() && viewer.device.is_none());
        }
        std::fs::remove_file(path)?;
        Ok(())
    }
}
