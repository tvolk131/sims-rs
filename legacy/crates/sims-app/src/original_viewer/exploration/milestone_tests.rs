use super::*;

#[test]
fn release_requires_queue_use_frame_unwind_and_containment_to_be_clear() -> Result<()> {
    let mut vm = sims_vm::Vm::new(sims_vm::World::default(), 17);
    vm.world.objects.insert(1, sims_vm::Object::default());
    let mut target = sims_vm::Object::default();
    target.data.insert(62, 0);
    vm.world.objects.insert(2, target);
    ensure!(interaction_released(&vm, 1, 2).is_err());
    vm.world.actions = Some(Default::default());
    vm.world
        .actions
        .as_mut()
        .unwrap()
        .actors
        .insert(1, Default::default());
    ensure!(interaction_released(&vm, 1, 2)?);
    let action = sims_vm::action::Action {
        target: 2,
        ..Default::default()
    };
    vm.world
        .actions
        .as_mut()
        .unwrap()
        .actors
        .get_mut(&1)
        .unwrap()
        .queue
        .pending
        .push_back(action.clone());
    ensure!(!interaction_released(&vm, 1, 2)?);
    let owner = vm
        .world
        .actions
        .as_mut()
        .unwrap()
        .actors
        .get_mut(&1)
        .unwrap();
    owner.queue.pending.clear();
    owner.active = Some(action);
    ensure!(!interaction_released(&vm, 1, 2)?);
    vm.world
        .actions
        .as_mut()
        .unwrap()
        .actors
        .get_mut(&1)
        .unwrap()
        .active = None;
    vm.world.objects.get_mut(&1).unwrap().container = Some((2, 0));
    ensure!(!interaction_released(&vm, 1, 2)?);
    vm.world.objects.get_mut(&1).unwrap().container = None;
    vm.world.objects.get_mut(&2).unwrap().slots.push(Some(1));
    ensure!(!interaction_released(&vm, 1, 2)?);
    vm.world.objects.get_mut(&2).unwrap().slots.clear();
    let frame = sims_vm::Frame {
        continuation: 0,
        action_target: Some(2),
        action_icon: false,
        code_owner: "fixture".into(),
        location: sims_vm::Location {
            source: "fixture".into(),
            bhav: 4096,
            instruction: 0,
        },
        caller: 1,
        callee: 2,
        stack_object: 2,
        args: vec![],
        locals: vec![],
        opaque_locals: Default::default(),
        storage_offset: 0,
    };
    vm.threads.insert(
        7,
        sims_vm::Thread {
            service: None,
            dispatch: None,
            autonomy: None,
            find_best: vec![],
            completed_actions: vec![],
            last_result: false,
            callback: None,
            callback_result: None,
            stack: vec![frame],
            frame_memory: vec![],
            retained_frame_arena: None,
            check: false,
            interrupted: false,
            status: sims_vm::Status::Ready,
        },
    );
    ensure!(!interaction_released(&vm, 1, 2)?);
    ensure!(!object_uses_accounted(&vm, 2)?);
    vm.world.objects.get_mut(&2).unwrap().data.insert(62, 1);
    ensure!(object_uses_accounted(&vm, 2)?);
    // A source match on a different actor or BHAV cannot satisfy an actual
    // stair/Paint milestone; no suffix or substring matching is accepted.
    ensure!(script_frame_present(&vm, 1, "fixture", Some(4096))?);
    ensure!(!script_frame_present(&vm, 2, "fixture", Some(4096))?);
    ensure!(!script_frame_present(&vm, 1, "fixture", Some(4104))?);
    ensure!(!script_frame_present(&vm, 1, "fix", None)?);
    ensure!(script_frame_present(&vm, 1, "", None).is_err());
    let thread = vm.threads.get_mut(&7).unwrap();
    thread.stack.clear();
    thread.completed_actions.push((1, 2, true, false, 1));
    ensure!(!interaction_released(&vm, 1, 2)?);
    ensure!(object_uses_accounted(&vm, 2)?);
    vm.threads.get_mut(&7).unwrap().completed_actions.clear();
    ensure!(interaction_released(&vm, 1, 2)?);
    ensure!(!object_uses_accounted(&vm, 2)?);
    vm.world.objects.get_mut(&2).unwrap().data.insert(62, 0);
    ensure!(object_uses_accounted(&vm, 2)?);
    // A different actor's use is not this actor's ownership, while the global
    // object count is an independent assertion for exclusive-use scenarios.
    vm.world.objects.get_mut(&2).unwrap().data.insert(62, 1);
    ensure!(interaction_released(&vm, 1, 2)?);
    ensure!(object_use_count(&vm, 2)? == 1);
    vm.world.objects.get_mut(&2).unwrap().data.insert(62, -1);
    ensure!(object_use_count(&vm, 2).is_err());
    vm.world.objects.get_mut(&2).unwrap().data.remove(&62);
    ensure!(object_use_count(&vm, 2).is_err());
    Ok(())
}

#[test]
fn animation_phase_requires_live_finite_layer_and_valid_interval() -> Result<()> {
    ensure!(animation_phase_in_range(0.3, false, 0.3, 0.7)?);
    ensure!(!animation_phase_in_range(0.7, false, 0.3, 0.7)?);
    ensure!(!animation_phase_in_range(0.5, true, 0.3, 0.7)?);
    ensure!(!animation_phase_in_range(0.29, false, 0.3, 0.7)?);
    for (minimum, maximum) in [
        (0.7, 0.3),
        (0.3, 0.3),
        (-0.1, 0.7),
        (0.3, 1.1),
        (f32::NAN, 0.7),
    ] {
        ensure!(animation_phase_in_range(0.5, false, minimum, maximum).is_err());
    }
    ensure!(animation_phase_in_range(f32::NAN, false, 0.3, 0.7).is_err());
    Ok(())
}

#[test]
fn exclusive_seat_requires_both_links_and_rejects_another_person() -> Result<()> {
    let mut vm = sims_vm::Vm::new(sims_vm::World::default(), 17);
    let mut person = sims_vm::Object::default();
    person.person = Some(Default::default());
    person.container = Some((2, 0));
    vm.world.objects.insert(1, person.clone());
    vm.world.objects.insert(2, sims_vm::Object::default());
    ensure!(!sole_person_contained(&vm, 1, 2)?);
    vm.world.objects.get_mut(&2).unwrap().slots.push(Some(1));
    ensure!(sole_person_contained(&vm, 1, 2)?);
    vm.world.objects.get_mut(&1).unwrap().container = Some((2, 1));
    ensure!(!sole_person_contained(&vm, 1, 2)?);
    vm.world.objects.get_mut(&1).unwrap().container = Some((2, 0));
    vm.world.objects.insert(3, person);
    ensure!(!sole_person_contained(&vm, 1, 2)?);
    vm.world.objects.get_mut(&3).unwrap().container = None;
    ensure!(sole_person_contained(&vm, 1, 2)?);
    Ok(())
}
