use anyhow::{Context, Result, ensure};
use sims_content::Index;
use sims_render::{camera_device::Device, software};
use sims_runtime::original_session::{Saved, Session};
use sims_sim::edit_history::{
    List, Store, Vector,
    command::{Method, Request},
    object::{Class, Object, Objects, budget, driver},
    snapshot::{self, CaptureView, References, Registry, Snapshot, lifecycle, placement},
};
use std::{collections::BTreeMap, sync::Arc};

//A retained command whose original target is already absent. Native removal
//returns success without a virtual call, then Undo applies its actual refund.
//This deliberately tests the budget child, not unimplemented placement/factory.
struct Absent;
impl Registry for Absent {
    fn lookup(&self, _: i16) -> Result<Option<u32>, String> {
        Ok(None)
    }
    fn on_lot(&mut self, _: u32) -> Result<bool, String> {
        Err("absent target queried on lot".into())
    }
}
impl CaptureView for Absent {
    fn normalized_master(&self, _: u32) -> Result<Option<u32>, String> {
        Err("unexpected capture".into())
    }
    fn script_id(&self, _: u32) -> Result<i16, String> {
        Err("unexpected capture".into())
    }
    fn capture(&mut self, _: u32) -> Result<snapshot::Position, String> {
        Err("unexpected capture".into())
    }
    fn definition(&self, _: u32) -> Result<u32, String> {
        Err("unexpected capture".into())
    }
    fn facing(&self, _: u32) -> Result<i16, String> {
        Err("unexpected facing".into())
    }
}
impl placement::View for Absent {
    fn facing(&self, _: u32) -> Result<i16, String> {
        Err("unexpected placement".into())
    }
}
impl lifecycle::View for Absent {
    fn manager(&self) -> Result<u32, String> {
        Err("unexpected factory".into())
    }
}
#[derive(serde::Serialize, serde::Deserialize)]
struct Command {
    store: Store,
    objects: Objects,
    refs: References,
    budgets: budget::Budgets,
    execution: driver::Execution,
}
impl Command {
    fn new(id: u32) -> Result<Self> {
        let mut store = Store::default();
        store.construct_command(id).map_err(anyhow::Error::msg)?;
        store.lists.insert(List::Objects(id), Vector::default());
        let mut refs = References::new(1);
        let key = refs.acquire(31000, &Absent).map_err(anyhow::Error::msg)?;
        let objects = Objects(BTreeMap::from([(
            id,
            Object {
                cost: Some(250),
                budget: Some(900),
                kind: 0,
                special: Some(1),
                class: Class::Object,
                snapshots: [
                    Snapshot {
                        key,
                        active: 1,
                        ..Default::default()
                    },
                    Snapshot::default(),
                ],
            },
        )]));
        Ok(Self {
            store,
            objects,
            refs,
            budgets: budget::Budgets(BTreeMap::from([(
                900,
                budget::State {
                    pending: 11,
                    category: 123,
                },
            )])),
            execution: driver::Execution::new(Request {
                command: id,
                method: Method::Undo,
            })
            .map_err(anyhow::Error::msg)?,
        })
    }
    fn advance(&mut self) -> Result<driver::Step> {
        self.execution
            .advance(
                &mut self.store,
                &mut self.objects,
                &mut self.refs,
                &mut self.budgets,
                &mut Absent,
                100,
            )
            .map_err(anyhow::Error::msg)
    }
}
#[test]
#[ignore = "private original Goth/Mashuga saved sessions and headless camera backend"]
fn original_house_budget_broadcast_resumes_without_recharging_or_advancing_vm() -> Result<()> {
    //Debug Serde frames for the full saved character graph exceed libtest's
    //default small worker stack. Keep the larger stack scoped to this test.
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run_house_budget_regression)?
        .join()
        .expect("house budget worker panicked")
}
fn run_house_budget_regression() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = root.join(std::env::var("SIMS_OBJECT_BUDGET_OUTPUT")?);
    ensure!(out.starts_with(root.join("cache")) && !out.exists());
    std::fs::create_dir_all(&out)?;
    let index = Arc::new(Index::scan(&root.join("game-data/cabinet-extracted"))?);
    let mut reports = vec![];
    for lot in ["goth", "mashuga"] {
        eprintln!("{lot}: restoring real household and camera");
        let wrapper: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join(
            format!("cache/original-ui-timer-dispatch-iced/{lot}/resumed.json"),
        ))?)?;
        let saved: Saved = serde_json::from_value(wrapper["session"].clone())?;
        let mut session = Session::restore(&index, saved)?;
        let mut device = Device::restore(
            &session,
            serde_json::from_value(wrapper["camera_device"].clone())?,
        )?;
        let pixels = software::render(device.scene());
        ensure!(session.simulation.vm.world.globals[&32] != 1);
        //These saved timer fixtures have seven engine listeners, predating the
        //replacement Interface binding. Register that real UI owner now through
        //407577's registry operation; do not invent a captured startup order.
        let context = &session
            .host
            .presentation
            .notifications
            .as_ref()
            .context("listener registry absent")?
            .dispatch;
        let mut registry = context.registry.clone();
        ensure!(
            !registry
                .nodes
                .values()
                .any(|n| n.handler == sims_vm::commander::house::Receiver::Interface)
        );
        let identity = registry
            .nodes
            .keys()
            .copied()
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .context("listener ID overflow")?;
        registry.construct(identity).map_err(anyhow::Error::msg)?;
        registry
            .bind(identity, sims_vm::commander::house::Receiver::Interface)
            .map_err(anyhow::Error::msg)?;
        let mut fields = context.fields.clone();
        ensure!(
            session.host.presentation.transport.is_some(),
            "actual UI controls absent"
        );
        fields.ui.mask_owner_present = true;
        session.bind_house_listeners(registry, fields)?;
        //The restored UI has its original independent gates. This regression
        //requires the ungated branch so both real backend services are observed.
        let fields = &session
            .host
            .presentation
            .notifications
            .as_ref()
            .context("listener registry absent")?
            .dispatch
            .fields;
        ensure!(fields.ui.gated == 0 && fields.ui.creation_gated == 0);
        let controls_before = serde_json::to_value(device.save(&session)?)?["controls_generation"]
            .as_u64()
            .context("control generation absent")?;
        let mut expected = session.simulation.clone();
        let mut command = Command::new(1)?;
        let driver::Step::Service(driver::Service::Budget(apply)) = command.advance()? else {
            anyhow::bail!("Undo did not reach its budget child")
        };
        ensure!(
            apply
                == budget::Apply {
                    category: 7,
                    amount: -239
                }
        );
        let b = expected
            .vm
            .world
            .budget
            .as_mut()
            .context("house budget absent")?;
        b.balance = b.balance.wrapping_sub(apply.amount);
        b.expenses[7] = b.expenses[7].wrapping_add(apply.amount);
        expected
            .vm
            .world
            .globals
            .insert(14, (b.balance / 10000) as i16);
        expected
            .vm
            .world
            .globals
            .insert(13, (b.balance % 10000) as i16);
        expected
            .household
            .as_mut()
            .context("household absent")?
            .clock
            .statistics[5]
            .copy_from_slice(&b.expenses[..8]);
        let balance = b.balance;
        session.begin_object_budget(&command.execution)?;
        eprintln!("{lot}: budget applied; entering saved DA callbacks");
        ensure!(
            session
                .finish_object_budget(&mut command.execution)
                .is_err(),
            "acknowledged before DA returned"
        );
        let mut restores = 0;
        let mut done = false;
        for _ in 0..100 {
            ensure!(session.step().is_err(), "tick ran inside budget child");
            ensure!(
                command.budgets.0[&900]
                    == budget::State {
                        pending: -239,
                        category: 7
                    }
            );
            let progress =
                session.advance_house_notification(session.retained_camera_input()?, 1)?;
            //Cold-load the whole household at actual externally observable
            //callback boundaries. The small unit corpus restores every step.
            if restores == 0
                || matches!(
                    progress,
                    sims_runtime::original_session::notifications::Progress::Service(_)
                )
            {
                let bytes =
                    serde_json::to_vec(&(session.save(), device.save(&session)?, &command))?;
                let (saved, camera, c): (Saved, sims_render::camera_device::Saved, Command) =
                    serde_json::from_slice(&bytes)?;
                session = Session::restore(&index, saved)?;
                device = Device::restore(&session, camera)?;
                command = c;
                restores += 1;
                eprintln!("{lot}: restored callback boundary {restores}");
            }
            ensure!(
                serde_json::to_value(&session.simulation)? == serde_json::to_value(&expected)?,
                "VM drift across budget save/resume"
            );
            if matches!(
                progress,
                sims_runtime::original_session::notifications::Progress::Complete(_)
            ) {
                done = true;
                break;
            }
            if matches!(
                progress,
                sims_runtime::original_session::notifications::Progress::Service(_)
            ) {
                ensure!(!device.advance_notification(&mut session, 1)?);
            }
        }
        ensure!(done, "budget broadcast did not return");
        ensure!(
            serde_json::to_value(device.save(&session)?)?["controls_generation"]
                == controls_before + 1,
            "UI refresh count differs"
        );
        ensure!(
            session.house_notification_pending(),
            "budget caller returned before acknowledgement"
        );
        let mut different = Command::new(2)?;
        ensure!(matches!(
            different.advance()?,
            driver::Step::Service(driver::Service::Budget(_))
        ));
        ensure!(
            session
                .finish_object_budget(&mut different.execution)
                .unwrap_err()
                .to_string()
                .contains("caller changed"),
            "equal charge accepted from another command"
        );
        session.finish_object_budget(&mut command.execution)?;
        ensure!(
            command.budgets.0[&900].pending == -239,
            "caller cleared before original return point"
        );
        ensure!(matches!(command.advance()?, driver::Step::Complete(0)));
        ensure!(
            command.budgets.0[&900]
                == budget::State {
                    pending: 0,
                    category: 123
                }
        );
        ensure!(!session.house_notification_pending());
        ensure!(serde_json::to_value(&session.simulation)? == serde_json::to_value(&expected)?);
        let image = software::render(device.scene());
        ensure!(image == pixels, "budget changed lot pixels");
        image.save(out.join(format!("{lot}.png")))?;
        std::fs::write(
            out.join(format!("{lot}.json")),
            serde_json::to_vec(&(session.save(), device.save(&session)?, &command))?,
        )?;
        reports.push(serde_json::json!({"lot":lot,"tick":session.simulation.vm.tick,"balance":balance,"amount":apply.amount,"category":apply.category,"restores":restores,"interface_registered_by_test":true,"real_dispatch_and_ui":true,"vm_matches_budget_only":true,"pixels_unchanged":true,"caller_cleared_after_listeners":true}));
    }
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"native_execution":false,"scope":"budget child from an absent-target Undo command, not placement/factory integration","cases":reports}),
        )?,
    )?;
    Ok(())
}
