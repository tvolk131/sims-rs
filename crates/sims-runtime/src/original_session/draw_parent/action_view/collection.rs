//! Collection's source-bound caller and read-only person/queue services borrow
//! the scheduler's actual Simulation. Native pointer identities remain explicit.
use super::*;
use anyhow::Context;
use sims_vm::{
    ObjectId, interface::person_selection as native, scheduler::clock::draw::actions::collect,
};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Person {
    object: ObjectId,
    pointer: u32,
    guid: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Owner {
    people: Vec<Person>,
    frame: Option<collect::Frame>,
    selected: Option<u32>,
    #[serde(default)]
    temporary_defined: u32,
}
pub(super) fn import(source: &[u8]) -> Result<Owner> {
    ensure!(
        sims_fixtures::digest(source)
            == "0960f0abfb6f086205d95f1025b8d133cb67a7674b407b438500e2d6371b2c02",
        "collection source differs"
    );
    let v: serde_json::Value = serde_json::from_slice(source)?;
    let m = Memory::new(&v)?;
    ensure!(
        v["schema"] == 1
            && v["tick"] == 89073
            && v["executes_target_code"] == false
            && v["native_parity_credit"] == 0,
        "collection source epoch differs"
    );
    for (p, n, h) in [
        (
            0x4ea31c,
            199,
            "81f2885d2616615fefc4d78387a082512ce7669360b2bfd9efef6588fcd977c7",
        ),
        (
            0x4e9608,
            74,
            "6471ebb6e89dc3fcd34f2ef9fff6d80318b0d034c45d810ed17a0864c398a2a8",
        ),
        (
            0x45d78b,
            51,
            "4815ad6d376de446194be0e732ecd4278333eb0869cdbd45613c94040365cab8",
        ),
    ] {
        ensure!(
            sims_fixtures::digest(&m.read(p, n)?) == h,
            "collection instruction source differs"
        );
    }
    let people: Vec<Person> = serde_json::from_value(v["people"].clone())?;
    for person in &people {
        ensure!(
            i16::from_le_bytes(m.read(person.pointer + 0x60, 2)?.try_into().unwrap())
                == person.object,
            "collection person identity differs"
        );
        let resource = word(&m.read(person.pointer + 0x11c, 4)?, 0);
        let definition = word(&m.read(resource + 0x60, 4)?, 0);
        ensure!(
            word(&m.read(definition + 0x1c, 4)?, 0) == person.guid,
            "collection person GUID differs"
        );
    }
    let owner = Owner {
        people,
        frame: None,
        selected: None,
        temporary_defined: 0,
    };
    owner.validate()?;
    Ok(owner)
}
impl Owner {
    pub(super) fn validate(&self) -> Result<()> {
        let expected=self.temporary().map_or(0,|raw|if actions::record::word(raw,12)==0 {actions::record::CONSTRUCTED_BYTES}else{actions::record::BUILT_BYTES});
        ensure!(self.temporary_defined==expected,"collection temporary initialized-byte ownership differs");
        if let Some(frame) = &self.frame {
            frame.validate().map_err(anyhow::Error::msg)?;
            ensure!(
                Some(frame.selected_index()) == self.selected,
                "collection selected cursor differs"
            );
        }
        ensure!(
            self.frame.is_some() == self.selected.is_some(),
            "collection source cursor ownership differs"
        );
        ensure!(
            self.people
                .iter()
                .map(|p| (p.object, p.pointer, p.guid))
                .collect::<Vec<_>>()
                == [
                    (16, 0x0c6c2a88, 3240130835),
                    (18, 0x0c5d6a58, 3234212238),
                    (21, 0x0c3df738, 3229573508),
                    (259, 0x0c7ca650, 1151887980)
                ],
            "collection retained person identities differ"
        );
        Ok(())
    }
    pub(super) fn pending(&self) -> Option<&collect::Request> {
        self.frame.as_ref().and_then(collect::Frame::pending)
    }
    pub(super) fn temporary(&self)->Option<&actions::Record> {self.frame.as_ref().and_then(collect::Frame::temporary)}
    pub(super) fn actor_for_node(&self,node:u32)->Result<ObjectId> {
        self.people.iter().find(|p|p.pointer.wrapping_add(0xa48)==node).map(|p|p.object).ok_or_else(||anyhow::anyhow!("action node requires a retained queue pointer binding"))
    }
    pub(super) fn complete(&self) -> bool {
        self.frame.as_ref().is_some_and(collect::Frame::complete)
    }
    pub(super) fn advance(&mut self, selected: u32) -> Result<()> {
        self.validate()?;
        if self.frame.is_none() {
            self.frame = Some(collect::Frame::at_entry(selected));
            self.selected = Some(selected);
        }
        ensure!(
            self.selected == Some(selected),
            "collection selected index changed during its caller"
        );
        self.frame
            .as_mut()
            .unwrap()
            .advance()
            .map_err(anyhow::Error::msg)?;
        Ok(())
    }
    pub(super) fn service_storage(&mut self, resources: &mut records::Resources, output: &mut Vec<actions::Record>) -> Result<bool> {
        self.validate()?;
        if self.pending() != Some(&collect::Request::Construct) {return Ok(false);}
        let Some(record) = resources.construct()? else {return Ok(false);};
        self.frame.as_mut().unwrap().acknowledge(output,collect::Request::Construct,collect::Reply::Record(record)).map_err(anyhow::Error::msg)?;
        self.temporary_defined = actions::record::CONSTRUCTED_BYTES;
        self.validate()?;
        Ok(true)
    }
    pub(super) fn service_program(&mut self,resources:&mut records::Resources,simulation:&mut sims_sim::Simulation,program:&sims_vm::Program,output:&mut Vec<actions::Record>)->Result<bool> {
        self.validate()?;
        let Some(request)=self.pending().cloned() else{return Ok(false);};
        let find=|person|self.people.iter().find(|p|p.pointer==person).map(|p|p.object).ok_or_else(||anyhow::anyhow!("record person pointer binding absent"));
        let reply=match &request {
            collect::Request::Build{person,..}=>{
                let Some((record,success))=resources.build(simulation,find(*person)?,&request)? else{return Ok(false);};
                if success {self.temporary_defined=actions::record::BUILT_BYTES;}
                collect::Reply::Built{record,success}
            }
            collect::Request::Append{record}=>{
                let node=actions::record::word(record,12);let actor=find(node.wrapping_sub(0xa48))?;
                let Some(copy)=resources.copy_record(simulation,program,actor,node,record,output.len())? else{return Ok(false);};
                collect::Reply::Record(copy)
            }
            collect::Request::DestroyString{..}=>{
                if resources.destroy_temporary(&request)?.is_none(){return Ok(false);}
                collect::Reply::Done
            }
            collect::Request::DestroyImage{..}=>{
                let Some(record)=resources.destroy_temporary(&request)? else{return Ok(false);};
                collect::Reply::Record(record)
            }
            _=>return Ok(false),
        };
        self.frame.as_mut().unwrap().acknowledge(output,request,reply).map_err(anyhow::Error::msg)?;
        self.validate()?;Ok(true)
    }
    /// Read only scalar/lookup services from the actual scheduler state.
    pub(super) fn service_scalar(
        &mut self,
        simulation: &sims_sim::Simulation,
        output: &mut Vec<actions::Record>,
    ) -> Result<bool> {
        self.validate()?;
        let Some(request) = self.pending().cloned() else {
            return Ok(false);
        };
        let world = &simulation.vm.world;
        let reply = match request {
            collect::Request::Person { index } => {
                let guid = native::cache_guid(&world.image_cache, index);
                let mut found = None;
                if guid != 0 {
                    if let Some(persons) = &world.person {
                        for &id in &persons.person_order {
                            let definition = world
                                .objects
                                .get(&id)
                                .and_then(|o| o.definition.as_ref())
                                .context("collection person definition absent")?;
                            if definition.guid == guid {
                                let binding = self
                                    .people
                                    .iter()
                                    .find(|p| p.object == id)
                                    .context("collection person pointer binding absent")?;
                                ensure!(
                                    binding.guid == guid,
                                    "collection person resource identity changed"
                                );
                                found = Some(binding.pointer);
                                break;
                            }
                        }
                    }
                }
                collect::Reply::Person(found)
            }
            collect::Request::QueueCount { person } => {
                let identity = self
                    .people
                    .iter()
                    .find(|p| p.pointer == person)
                    .context("collection queued person binding absent")?
                    .object;
                let queue = &world
                    .actions
                    .as_ref()
                    .context("collection action registry absent")?
                    .actors
                    .get(&identity)
                    .context("collection person action queue absent")?
                    .queue;
                collect::Reply::Count(queue.pending.len().try_into()?)
            }
            _ => return Ok(false),
        };
        self.frame
            .as_mut()
            .unwrap()
            .acknowledge(output, request, reply)
            .map_err(anyhow::Error::msg)?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "private source pointer bindings and record resources; supplied VM getter context"]
    fn collection_borrows_one_vm_and_preserves_constructor_cut() -> Result<()> {
        check_collection(false,false)
    }
    #[test]
    #[ignore = "private original constructor code and retained caption ownership"]
    fn collection_constructs_once_and_restores_before_image_build() -> Result<()> {
        check_collection(true,false)
    }
    #[test]
    #[ignore="private frozen thumbnail/caption services with supplied VM context"]
    fn collection_builds_copies_and_tears_down_one_owned_record_across_restore()->Result<()> {check_collection(true,true)}
    fn check_collection(construct: bool, services:bool) -> Result<()> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let source =
            std::fs::read(root.join(
                "fixtures/private/goth-action-refresh-20261031-first/action-source-first.json",
            ))?;
        let resources =
            std::fs::read(root.join(
                "fixtures/private/goth-action-records-20261031-first/record-source-first.json",
            ))?;
        let collect = std::fs::read(root.join(
            "fixtures/private/goth-action-records-20261031-first/collection-source-third.json",
        ))?;
        let mut owner = super::super::import(&source)?;
        owner.bind_records(&resources)?;
        owner.bind_collection(&collect)?;
        if construct {
            owner.bind_record_build(&std::fs::read(root.join("fixtures/private/goth-action-build-20261031-first/build-source-first.json"))?)?;
        }
        if services {
            owner.bind_services(&std::fs::read(root.join("fixtures/private/goth-record-services-20261031-first/service-source-first.json"))?,&std::fs::read(root.join("fixtures/private/goth-action-build-20261031-first/build-source-first.json"))?,&std::fs::read(root.join("fixtures/private/goth-thumbnail-20261031-first/thumbnail-source-first.json"))?)?;
        }
        // Supplied integration context, not a reconstructed full native World.
        // Source-derived pointer/GUID bindings resolve the same identities, but
        // other cache entries and whole-household parity are not claimed here.
        let mut world = sims_vm::World::default();
        let bindings = &owner.collection.as_ref().unwrap().people;
        let mut people = sims_vm::person::WorldState::default();
        people.person_order = bindings.iter().map(|p| p.object).collect();
        for p in bindings {
            world.objects.insert(
                p.object,
                sims_vm::Object {
                    native_flags: Some(if p.object == 21 { 2 } else { 0 }),
                    person:services.then(BTreeMap::new),
                    definition: Some(sims_vm::Definition {
                        source: "supplied-getter-context".into(),
                        object: 0,
                        guid: p.guid,
                        slots: 0,
                    }),
                    ..Default::default()
                },
            );
        }
        world.person = Some(people);
        world.image_cache.entries = (0..327)
            .map(|_| sims_vm::image_cache::Entry::new(0))
            .collect();
        let guid = bindings.iter().find(|p| p.object == 21).unwrap().guid;
        world.image_cache.entries[326] = sims_vm::image_cache::Entry::new(guid);
        if services {
            world.objects.insert(132,sims_vm::Object{definition:Some(sims_vm::Definition{source:"supplied-getter-context".into(),object:0,guid:0x5cdc712f,slots:0}),..Default::default()});
            for object in world.objects.values_mut() {object.data.insert(1,0);}
            world.objects.get_mut(&21).unwrap().person=Some(BTreeMap::from([(39,1)]));
            let id=word(&owner.state.headers[&0x0b7c7cb0],0x108);
            world.actions=Some(sims_vm::action::WorldState{actors:BTreeMap::from([(21,sims_vm::action::Actor{active:Some(sims_vm::action::Action{id,icon_owner:132,target:132,flags:128,label:Some("Read a Book".into()),..Default::default()}),..Default::default()})]),..Default::default()});
        }
        let unplaced = if services {Default::default()}else{world.objects.keys().copied().collect()};
        let bodies=if services {world.objects.keys().enumerate().map(|(i,&id)|(id,sims_sim::Body{position:sims_sim::Point::new(i as i32+2,2),facing:1,level:1,radius:0,elevation:0})).collect()}else{BTreeMap::new()};
        let vm = sims_vm::Vm::new(world, 0x12345678);
        let mut simulation = sims_sim::Simulation::from_placements(
            vm,
            sims_sim::Room {
                width: 16,
                height: 16,
                obstacles: vec![],
            },
            bodies,
            unplaced,
            None,
        )
        .map_err(anyhow::Error::msg)?;
        let mut program = sims_vm::Program::default();
        if services {
            program.modules.insert("supplied-getter-context".into(),sims_vm::Module{source:"supplied-getter-context".into(),object_interactions:BTreeMap::from([(0,vec![sims_vm::program::MenuEntry::default()])]),..Default::default()});
        }
        let before = sims_sim::Snapshot::capture(&program, &simulation);
        let mut controls = state::State::constructed(0);
        controls.mode = 2;
        assert_eq!(
            owner.advance_for_simulation(&controls, &simulation)?,
            Some(actions::Request::Collect { selected: 326 })
        );
        if construct {
            let Some(collect::Request::Build{index,person,node,current,record}) = owner.collection_pending() else {anyhow::bail!("constructed caller did not stop at its image factory");};
            assert_eq!((*index,*person,*node,*current),(326,0x0c3df738,0x0c3e0180,1));
            assert_eq!((actions::record::word(record,4),actions::record::word(record,12)),(0,0));
            assert_eq!([16,20,24].map(|off|actions::record::word(record,off)),[0xf0000000,0xf0000000,0xf0000008]);
            assert_eq!(owner.collection.as_ref().unwrap().temporary_defined,actions::record::CONSTRUCTED_BYTES);
            let mut bad = owner.clone();
            bad.collection.as_mut().unwrap().temporary_defined ^= 1;
            assert!(bad.validate().is_err());
            let mut bad = serde_json::to_value(&owner)?;
            for field in ["temp","pending"] {
                let row = if field == "temp" {&mut bad["collection"]["frame"][field]} else {&mut bad["collection"]["frame"][field]["Build"]["record"]};
                for (off,value) in [(16,0xf0000008u32),(20,0xf0000008),(24,0xf0000010)] {
                    for (i,byte) in value.to_le_bytes().iter().enumerate() {row[off+i]=serde_json::json!(*byte);}
                }
            }
            let bad: super::super::Owner = serde_json::from_value(bad)?;
            assert!(bad.collection.as_ref().unwrap().frame.as_ref().unwrap().validate().is_ok());
            assert!(bad.validate().is_err(),"restored temporary accepted another allocation handle");
        } else {assert_eq!(owner.collection_pending(),Some(&collect::Request::Construct));}
        assert_eq!(sims_sim::Snapshot::capture(&program, &simulation), before);
        assert!(owner.resources.as_ref().unwrap().cleared());
        let wire = postcard::to_allocvec(&owner)?;
        let mut restored: super::super::Owner = postcard::from_bytes(&wire)?;
        // The two getters are already consumed. Altering the borrowed world
        // cannot redirect their suspended constructor to another person.
        simulation
            .vm
            .world
            .objects
            .get_mut(&21)
            .unwrap()
            .native_flags = Some(0);
        simulation
            .vm
            .world
            .objects
            .get_mut(&18)
            .unwrap()
            .native_flags = Some(2);
        assert_eq!(
            restored.advance_for_simulation(&controls, &simulation)?,
            Some(actions::Request::Collect { selected: 326 })
        );
        assert_eq!(postcard::to_allocvec(&restored)?, wire);
        if services {
            let before=sims_sim::Snapshot::capture(&program,&simulation);
            let mut cold=before.clone().restore(&program).map_err(anyhow::Error::msg)?;
            let expected=Some(actions::Request::Image{window:0x0b7c7cb0,image:0xe0000000});
            assert_eq!(owner.advance_for_program(&controls,&mut simulation,&program)?,expected);
            assert_eq!(restored.advance_for_program(&controls,&mut cold,&program)?,expected);
            assert_eq!(postcard::to_allocvec(&restored)?,postcard::to_allocvec(&owner)?);
            assert_eq!(sims_sim::Snapshot::capture(&program,&simulation),sims_sim::Snapshot::capture(&program,&cold));
            let mut expected_vm=before;expected_vm.state.vm.world.actions.as_mut().unwrap().actors.get_mut(&21).unwrap().active.as_mut().unwrap().flags &=!128;
            assert_eq!(sims_sim::Snapshot::capture(&program,&simulation),expected_vm);
            assert!(owner.collection.as_ref().unwrap().complete());assert_eq!(owner.state.records.len(),1);
            let raw=owner.state.records[0];assert_eq!(actions::record::word(&raw,0),326);assert_eq!(&raw[8..10],&[0,1]);assert_eq!(actions::record::word(&raw,12),0x0c3e0180);
            assert_eq!([16,20,24].map(|off|actions::record::word(&raw,off)),[0xf0000010,0xf000001b,0xf000001c]);
            assert_eq!(owner.resources.as_ref().unwrap().image_counts(),&BTreeMap::from([(0x0bb6d5e0,1),(0xe0000000,1)]));
            let wire=postcard::to_allocvec(&owner)?;let mut resumed:super::super::Owner=postcard::from_bytes(&wire)?;
            assert_eq!(resumed.advance_for_program(&controls,&mut simulation,&program)?,expected);assert_eq!(postcard::to_allocvec(&resumed)?,wire);
        }
        Ok(())
    }
    #[test]
    #[ignore="private C8 retained household component cut and frozen UI inputs"]
    fn real_c8_vm_runs_collection_without_mutating_other_state()->Result<()> {
        std::thread::Builder::new().stack_size(32*1024*1024).spawn(||real_c8_vm_collection(false,false,false))?.join().map_err(|_|anyhow::anyhow!("C8 collection worker panicked"))?
    }
    #[test]
    #[ignore="private C8 retained household component and frozen window/surface inputs"]
    fn real_c8_vm_publishes_image_across_every_child_restore()->Result<()> {
        std::thread::Builder::new().stack_size(32*1024*1024).spawn(||real_c8_vm_collection(true,false,false))?.join().map_err(|_|anyhow::anyhow!("C8 publication worker panicked"))?
    }
    #[test]
    #[ignore="private C8 retained VM and original complete action-window caller"]
    fn real_c8_vm_finishes_action_caller_across_restore()->Result<()> {
        std::thread::Builder::new().stack_size(32*1024*1024).spawn(||real_c8_vm_collection(true,true,false))?.join().map_err(|_|anyhow::anyhow!("C8 action tail worker panicked"))?
    }
    #[test]
    #[ignore="private C8 retained VM with complete framework region services"]
    fn real_c8_vm_framework_reaches_world_across_restore()->Result<()> {
        std::thread::Builder::new().stack_size(32*1024*1024).spawn(||real_c8_vm_collection(true,true,true))?.join().map_err(|_|anyhow::anyhow!("C8 framework worker panicked"))?
    }
    fn real_c8_vm_collection(publish:bool,finish_tail:bool,finish_framework:bool)->Result<()> {
        real_c8_vm_collection_world(publish,finish_tail,finish_framework,false)
    }
    #[test]
    #[ignore="private C8 VM, World context and read-only original trace"]
    fn real_c8_vm_world_context_reaches_window_service_across_restore()->Result<()> {
        std::thread::Builder::new().stack_size(32*1024*1024).spawn(||real_c8_vm_collection_world(true,true,true,true))?.join().map_err(|_|anyhow::anyhow!("C8 World context worker panicked"))?
    }
    fn real_c8_vm_collection_world(publish:bool,finish_tail:bool,finish_framework:bool,finish_world_context:bool)->Result<()> {
        real_c8_vm_collection_prefix(publish,finish_tail,finish_framework,finish_world_context,false,false,false,false,false,false,false,false,false,false)
    }
    #[test]
    #[ignore="private C8 VM, observed OS window replies and original redraw footprint"]
    fn real_c8_vm_world_prefix_reaches_composition_across_restore()->Result<()> {
        std::thread::Builder::new().stack_size(32*1024*1024).spawn(||real_c8_vm_collection_prefix(true,true,true,true,true,false,false,false,false,false,false,false,false,false))?.join().map_err(|_|anyhow::anyhow!("C8 World prefix worker panicked"))?
    }
    #[test]
    #[ignore = "private frozen view, actual C8 VM and native cursor inputs"]
    fn real_c8_vm_world_composition_matches_original_across_restore()->Result<()> {
        std::thread::Builder::new().stack_size(32*1024*1024).spawn(||real_c8_vm_collection_prefix(true,true,true,true,true,true,false,false,false,false,false,false,false,false))?.join().map_err(|_|anyhow::anyhow!("C8 World composition worker panicked"))?
    }
    #[test]
    #[ignore = "private C8 scene, current room kinds and native state/regions"]
    fn real_c8_vm_scene_matches_original_across_restore()->Result<()> {
        std::thread::Builder::new().stack_size(32*1024*1024).spawn(||real_c8_vm_collection_prefix(true,true,true,true,true,true,true,false,false,false,false,false,false,false))?.join().map_err(|_|anyhow::anyhow!("C8 scene worker panicked"))?
    }
    #[test]
    #[ignore = "private C8 original indicator/classification state and callback order"]
    fn real_c8_vm_classification_matches_original_across_restore()->Result<()> {
        std::thread::Builder::new().stack_size(32*1024*1024).spawn(||real_c8_vm_collection_prefix(true,true,true,true,true,true,true,true,false,false,false,false,false,false))?.join().map_err(|_|anyhow::anyhow!("C8 classification worker panicked"))?
    }
    #[test]
    #[ignore = "private C8 original cached-camera and shared motion frame"]
    fn real_c8_vm_camera_preparation_matches_original_across_restore()->Result<()> {
        std::thread::Builder::new().stack_size(32*1024*1024).spawn(||real_c8_vm_collection_prefix(true,true,true,true,true,true,true,true,true,false,false,false,false,false))?.join().map_err(|_|anyhow::anyhow!("C8 camera worker panicked"))?
    }
    #[test]
    #[ignore = "private C8 actual person models, geometry, device and original virtual calls"]
    fn real_c8_vm_person_visibility_matches_original_across_restore()->Result<()> {
        std::thread::Builder::new().stack_size(32*1024*1024).spawn(||real_c8_vm_collection_prefix(true,true,true,true,true,true,true,true,true,true,false,false,false,false))?.join().map_err(|_|anyhow::anyhow!("C8 person visibility worker panicked"))?
    }
    #[test]
    #[ignore="private C8 shared framework queue and original World drawing setup"]
    fn real_c8_vm_drawing_setup_matches_original_with_shared_regions_and_restore()->Result<()> {
        std::thread::Builder::new().stack_size(32*1024*1024).spawn(||real_c8_vm_collection_prefix(true,true,true,true,true,true,true,true,true,true,true,false,false,false))?.join().map_err(|_|anyhow::anyhow!("C8 drawing worker panicked"))?
    }
    #[test]
    #[ignore="private C8 original paint markers and retained clipping/renderer owners"]
    fn real_c8_vm_paint_prefix_matches_original_with_shared_regions_and_restore()->Result<()> {
        std::thread::Builder::new().stack_size(32*1024*1024).spawn(||real_c8_vm_collection_prefix(true,true,true,true,true,true,true,true,true,true,true,true,false,false))?.join().map_err(|_|anyhow::anyhow!("C8 paint worker panicked"))?
    }
    #[test]
    #[ignore="private C8 original DirectDraw inputs and retained plane/backend/target state"]
    fn real_c8_vm_surface_acquisition_matches_original_across_every_child_restore()->Result<()> {
        std::thread::Builder::new().stack_size(32*1024*1024).spawn(||real_c8_vm_collection_prefix(true,true,true,true,true,true,true,true,true,true,true,true,true,false))?.join().map_err(|_|anyhow::anyhow!("C8 surface worker panicked"))?
    }
    #[test]
    #[ignore="private original window background, OffsetRect readbacks and retained pixel storage"]
    fn real_c8_vm_window_background_matches_original_and_selected_cold_cuts()->Result<()>{
        std::thread::Builder::new().stack_size(32*1024*1024).spawn(||real_c8_vm_collection_prefix(true,true,true,true,true,true,true,true,true,true,true,true,true,true))?.join().map_err(|_|anyhow::anyhow!("C8 background worker panicked"))?
    }
    fn real_c8_vm_collection_prefix(publish:bool,finish_tail:bool,finish_framework:bool,finish_world_context:bool,finish_prefix:bool,finish_composition:bool,finish_scene:bool,finish_classification:bool,finish_camera:bool,finish_people:bool,finish_drawing:bool,finish_paint:bool,finish_surfaces:bool,finish_background:bool)->Result<()> {
        let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let load=|p:&str|std::fs::read(root.join(p)).map_err(Into::<anyhow::Error>::into);
        let mut owner=super::super::import(&load("fixtures/private/goth-action-refresh-20261031-first/action-source-first.json")?)?;
        owner.bind_records(&load("fixtures/private/goth-action-records-20261031-first/record-source-first.json")?)?;
        owner.bind_collection(&load("fixtures/private/goth-action-records-20261031-first/collection-source-third.json")?)?;
        let object=load("fixtures/private/goth-action-build-20261031-first/build-source-first.json")?;
        owner.bind_record_build(&object)?;
        owner.bind_services(&load("fixtures/private/goth-record-services-20261031-first/service-source-first.json")?,&object,&load("fixtures/private/goth-thumbnail-20261031-first/thumbnail-source-first.json")?)?;
        if publish {owner.bind_publication(&load("fixtures/private/goth-image-publication-20261031-first/source-third.json")?)?;}
        if finish_tail {owner.bind_tail(&load("fixtures/private/goth-action-tail-20261031-first/tail-source-second.json")?)?;}
        let source_window=owner.state.headers[&0x0b7c7cb0].clone();
        let report=load(if finish_scene{"fixtures/private/goth-world-scene-20261031-first/corrected-prefix-first.json"}else{"fixtures/private/goth-retained-cold-20261031-first/reexecution-first.json"})?;
        ensure!(sims_fixtures::digest(&report)==if finish_scene{"c0258d64c53fefe82d163a1d76ac12187f63a0dad41fba788b4533c3e64061d6"}else{"80a9d9ade1d61a513cffd06e444428d376514fa533ddb096905c32ffee3c3849"},"C8 component producer changed");
        let report:serde_json::Value=serde_json::from_slice(&report)?;
        let reference:sims_fixtures::BlobRef=serde_json::from_value(report["boundaries"][7]["state"].clone())?;
        ensure!(reference.sha256==if finish_scene{"11f6beb853aea4c830eb3c0cd078e7c0d3350642d4beba20073669c6f58420ed"}else{"d5a9c9eec564ef0020b0191b0fed9c2b31bb6ed928c040c44ebcd133d0b78c63"},"C8 component cut changed");
        let store=sims_fixtures::Store::open(root.join(if finish_scene{"fixtures/private/goth-world-scene-20261031-first/corrected-prefix-first.store"}else{"fixtures/private/goth-retained-cold-20261031-first/reexecution-first.store"}))?;
        let retained:crate::original_session::completed_assets::checkpoints::Retained=store.get_value(&reference)?;
        let mut simulation=retained.simulation.state;
        ensure!(simulation.vm.tick==89081,"C8 component tick differs");
        let action=simulation.vm.world.actions.as_ref().unwrap().actors[&21].active.as_ref().unwrap();
        ensure!(action.label.as_deref()==Some("Read a Book") && action.icon_owner==132,"C8 needs its actual captured label and icon owner");
        let flags=action.flags;
        // This exact cached-name branch never queries Program. A default Program
        // is an explicit unused argument, not a replacement TTAB/source binding.
        let program=sims_vm::Program::default();
        let before=sims_sim::Snapshot::capture(&program,&simulation);
        let mut controls=state::State::constructed(0);controls.mode=2;
        let initial_for_draw=finish_tail.then(||owner.clone());
        let wire=postcard::to_allocvec(&owner)?;let mut cold_owner:super::super::Owner=postcard::from_bytes(&wire)?;
        let mut cold=before.state.clone();
        let expected=Some(actions::Request::Image{window:0x0b7c7cb0,image:0xe0000000});
        assert_eq!(owner.advance_for_program(&controls,&mut simulation,&program)?,expected);
        assert_eq!(cold_owner.advance_for_program(&controls,&mut cold,&program)?,expected);
        assert_eq!(postcard::to_allocvec(&owner)?,postcard::to_allocvec(&cold_owner)?);
        assert_eq!(sims_sim::Snapshot::capture(&program,&simulation),sims_sim::Snapshot::capture(&program,&cold));
        let mut publication_cuts=Vec::new();
        let mut native_window_compared=false;
        if publish {
            publication_cuts.push(serde_json::json!({"child":owner.image_pending(),"before_first_service":true}));
            let mut request=expected.clone();
            for _ in 0..8 {
                owner.validate()?;
                let wire=postcard::to_allocvec(&owner)?;
                let mut restored:super::super::Owner=postcard::from_bytes(&wire)?;restored.validate()?;
                let r=owner.advance_for_program(&controls,&mut simulation,&program)?;
                let restored_r=restored.advance_for_program(&controls,&mut cold,&program)?;
                assert_eq!(r,restored_r);assert_eq!(postcard::to_allocvec(&owner)?,postcard::to_allocvec(&restored)?);
                publication_cuts.push(serde_json::json!({"child":owner.image_pending(),"old_count":owner.resources.as_ref().unwrap().images[&0x0bb6d5e0],"new_count":owner.resources.as_ref().unwrap().images[&0xe0000000]}));
                request=r;
                if !matches!(request,Some(actions::Request::Image{..})){break;}
            }
            if finish_tail {
                assert_eq!(request,None);assert!(owner.complete());
                owner.tail.as_ref().unwrap().compare_native(&load("fixtures/private/goth-action-tail-20261031-first/native-goth-second.json")?,&owner.vector_headers,&owner.state)?;
            }else{assert!(matches!(request,Some(actions::Request::CaptionDirty{..})),"{request:?}");}
            let mut expected_window=source_window;expected_window[0xdc..0xe0].copy_from_slice(&0xe0000000u32.to_le_bytes());
            assert_eq!(owner.state.headers[&0x0b7c7cb0],expected_window);
            let witness=load("fixtures/private/goth-action-image-witness-20261031-first/action-image-trace.jsonl.gz")?;
            ensure!(sims_fixtures::digest(&witness)=="d99614a43a88842f9a8d5f65625a91364e0682d76b8ceaea2aef3fa149688176","original C8 witness differs");
            use std::io::Read;
            let mut text=String::new();flate2::read::GzDecoder::new(witness.as_slice()).read_to_string(&mut text)?;
            let native=text.lines().map(serde_json::from_str::<serde_json::Value>).collect::<std::result::Result<Vec<_>,_>>()?.into_iter().find(|r|r["kind"]=="same_stop_outer_readback"&&r["tick"]==89081&&r["registers"]["eip"]==0x4d2b7au32).ok_or_else(||anyhow::anyhow!("original C8 window return absent"))?;
            let memory=super::super::Memory::new(&native)?;
            let mut native=memory.read(0x0b7c7cb0,0x124)?;
            ensure!(sims_fixtures::digest(&native)=="1df940e758adc7636e19e2690537483a4d3eab81c163287072cce1051720efe3"&&super::super::word(&native,0xdc)==0x0b477270,"original C8 window witness changed");
            native[0xdc..0xe0].copy_from_slice(&0xe0000000u32.to_le_bytes());
            assert_eq!(owner.state.headers[&0x0b7c7cb0],native);native_window_compared=true;
            assert_eq!(owner.resources.as_ref().unwrap().images[&0x0bb6d5e0],0);
            assert_eq!(owner.resources.as_ref().unwrap().images[&0xe0000000],2);
            owner.validate()?;
            owner.publication.as_ref().unwrap().compare_native(&load("fixtures/private/goth-image-publication-20261031-first/native-goth-second.json")?,&owner.state.headers,&owner.resources.as_ref().unwrap().images)?;
        }
        let mut enclosing_draw_framework_pending=false;
        let mut enclosing_draw_world_pending=false;
        let mut world_context_verified=false;
        let mut world_cold_cuts=0;
        let mut world_prefix_verified=false;
        let mut world_composition_verified=false;
        let mut world_scene_verified=false;
        let mut world_classification_verified=false;
        let mut world_camera_verified=false;
        let mut camera_cold_cuts=0;
        let mut person_visibility_verified=false;
        let mut person_visibility_cold_cuts=0;
        let mut person_visibility_calls=0;
        let mut person_visibility_resume_inputs=Vec::new();
        let mut drawing_verified=false;
        let mut drawing_resume_inputs=Vec::new();
        let mut drawing_native_stops=0;
        let mut background_verified=false;let mut background_stops=0;let mut background_cold=Vec::new();
        let mut surfaces_verified=false;let mut surfaces_native_stops=0;let mut surfaces_resume_inputs=Vec::new();let mut surfaces_inputs=0;
        let mut paint_verified=false;let mut paint_native_stops=0;let mut paint_resume_inputs=Vec::new();let mut paint_next=None;
        let mut classification_cold_cuts=0;
        let mut classification_invalidations=Vec::new();
        let mut classification_callback_checks=0;
        let mut world_scene_cold_cuts=0;
        let mut world_scene_invalidations=Vec::new();
        let mut world_scene_rectangles=Vec::new();
        let mut cursor_inputs_consumed=0;
        let mut world_next=None;
        let mut world_redraw_match=None;
        let mut window_inputs_consumed=0;
        if let Some(initial)=initial_for_draw {
            use sims_vm::scheduler::clock::{draw,profiler::Buffer};
            let mut parent=crate::original_session::draw_parent::import(&load("fixtures/private/goth-render-parent-20261031-first/render-source-second.json")?)?;
            parent.bind_child_source(&load("fixtures/private/goth-draw-services-20261031-first/services-source-first.json")?)?;
            // Exact retained action owner at entry; the enclosing Draw uses its
            // real prefix and acknowledges ApplicationView only after completion.
            parent.actions=Some(initial);
            if finish_framework {parent.bind_framework_source(&load("fixtures/private/goth-framework-20261031-first/framework-source-fourth.json")?)?;}
            if finish_world_context {parent.bind_world_source(&load("fixtures/private/goth-world-draw-20261031-first/world-source-second.json")?)?;}
            if finish_camera {parent.bind_world_camera(&load("fixtures/private/goth-world-classification-20261031-first/classification-source-third.json")?)?;}
            if finish_people {parent.bind_world_people(&load("fixtures/private/goth-world-person-visibility-20261031-first/person-source-first.json")?)?;}
            if finish_drawing {parent.bind_world_drawing(&load("fixtures/private/goth-world-drawing-20261031-first/drawing-source-second.json")?)?;}
            if finish_paint {parent.bind_world_paint(&load("fixtures/private/goth-world-paint-20261031-first/paint-source-first.json")?)?;}
            if finish_background {parent.bind_world_background(&load("fixtures/private/goth-window-background-20261031-first/background-source-first.json")?)?;}
            if finish_surfaces {parent.bind_world_surfaces(&load("fixtures/private/goth-world-surfaces-20261031-first/surface-source-first.json")?,&load("fixtures/private/goth-world-surface-witness-20261031-first/world-classification-trace.jsonl.gz")?)?;}
            if finish_prefix {parent.bind_world_prefix(&load("fixtures/private/goth-world-prefix-20261031-first/prefix-source-first.json")?)?;}
            // Profiler time is a controlled diagnostic input; it does not
            // qualify the original performance-counter stream or a Session.
            let mut profiler=Buffer::from_captured(vec![],32).map_err(anyhow::Error::msg)?;
            for _ in 0..16 {
                let request=parent.advance()?.ok_or_else(||anyhow::anyhow!("Draw returned before action view"))?;
                if request==draw::Request::ApplicationView {break;}
                if parent.service_child(&mut profiler)? {continue;}
                let reply=match request{draw::Request::FpsClock=>draw::Reply::Clock(758330),draw::Request::Timestamp{..}=>draw::Reply::Timestamp(0),_=>anyhow::bail!("unexpected Draw prefix {request:?}")};
                parent.acknowledge(request,reply,&mut profiler)?;
            }
            ensure!(parent.pending()==Some(draw::Request::ApplicationView),"Draw action child absent");
            let mut parent_vm=before.state.clone();
            for _ in 0..8 {
                let wire=postcard::to_allocvec(&parent)?;let mut restored:crate::original_session::draw_parent::State=postcard::from_bytes(&wire)?;restored.validate()?;
                let mut restored_vm=parent_vm.clone();
                let request=parent.advance_action_view_for_program(&controls,&mut parent_vm,&program)?;
                assert_eq!(request,restored.advance_action_view_for_program(&controls,&mut restored_vm,&program)?);
                assert_eq!(postcard::to_allocvec(&parent)?,postcard::to_allocvec(&restored)?);
                assert_eq!(sims_sim::Snapshot::capture(&program,&parent_vm),sims_sim::Snapshot::capture(&program,&restored_vm));
                if request.is_none(){break;}
            }
            assert_eq!(postcard::to_allocvec(parent.actions.as_ref().unwrap())?,postcard::to_allocvec(&owner)?);
            assert_eq!(sims_sim::Snapshot::capture(&program,&parent_vm),sims_sim::Snapshot::capture(&program,&simulation));
            assert!(parent.service_child(&mut profiler)?);
            assert!(matches!(parent.advance()?,Some(draw::Request::Unload{..})));
            assert!(parent.service_child(&mut profiler)?);
            assert!(matches!(parent.advance()?,Some(draw::Request::Framework{owner:0x6fedf8})));
            let restored:crate::original_session::draw_parent::State=postcard::from_bytes(&postcard::to_allocvec(&parent)?)?;restored.validate()?;assert_eq!(restored,parent);
            enclosing_draw_framework_pending=true;
            if finish_framework {
                let before_framework=sims_sim::Snapshot::capture(&program,&parent_vm);
                for _ in 0..32 {
                    let wire=postcard::to_allocvec(&parent)?;let mut restored:crate::original_session::draw_parent::State=postcard::from_bytes(&wire)?;restored.validate()?;
                    let mut cold_profiler=profiler.clone();
                    assert!(parent.service_child(&mut profiler)?);assert!(restored.service_child(&mut cold_profiler)?);
                    assert_eq!(postcard::to_allocvec(&parent)?,postcard::to_allocvec(&restored)?);
                    assert_eq!(profiler,cold_profiler);
                    if parent.pending().is_none(){break;}
                }
                parent.framework.as_ref().unwrap().compare_native(&load("fixtures/private/goth-framework-20261031-first/native-goth-third.json")?)?;
                assert!(matches!(parent.advance()?,Some(draw::Request::World{owner:0x09e88960,argument:0})));
                assert_eq!(sims_sim::Snapshot::capture(&program,&parent_vm),before_framework);
                let restored:crate::original_session::draw_parent::State=postcard::from_bytes(&postcard::to_allocvec(&parent)?)?;restored.validate()?;assert_eq!(restored,parent);
                enclosing_draw_framework_pending=false;enclosing_draw_world_pending=true;
                if finish_world_context {
                    let request=parent.pending().unwrap();let saved_parent=parent.clone();let saved_profiler=profiler.clone();
                    assert!(parent.acknowledge(request,draw::Reply::Done,&mut profiler).is_err());
                    assert_eq!(parent,saved_parent);assert_eq!(profiler,saved_profiler);
                    for _ in 0..24 {
                        let mut restored:crate::original_session::draw_parent::State=postcard::from_bytes(&postcard::to_allocvec(&parent)?)?;restored.validate()?;
                        let mut cold_profiler=profiler.clone();
                        if matches!(parent.world_pending(),Some(crate::original_session::draw_parent::WorldRequest::Timestamp{..})) {
                            parent.acknowledge_world_timestamp(0,&mut profiler)?;restored.acknowledge_world_timestamp(0,&mut cold_profiler)?;
                        } else {assert_eq!(parent.service_child(&mut profiler)?,restored.service_child(&mut cold_profiler)?);}
                        assert_eq!(parent,restored);assert_eq!(profiler,cold_profiler);world_cold_cuts+=1;
                        if matches!(parent.world_pending(),Some(crate::original_session::draw_parent::WorldRequest::ClientRectangle{window:0xa006a,..})){break;}
                    }
                    parent.world.as_ref().unwrap().compare_native_context(&load("fixtures/private/goth-world-window-witness-20261031-first/world-window-trace.jsonl.gz")?)?;
                    assert!(matches!(parent.world_pending(),Some(crate::original_session::draw_parent::WorldRequest::ClientRectangle{window:0xa006a,rectangle:[0,0,0x6c7318,0]})));
                    assert!(!parent.service_child(&mut profiler)?);world_context_verified=true;
                    assert_eq!(sims_sim::Snapshot::capture(&program,&parent_vm),before_framework);
                    if finish_prefix {
                        use crate::original_session::draw_parent::{WindowInputs,WorldRequest};
                        let mut inputs=WindowInputs::import(&load("fixtures/private/goth-world-api-witness-20261031-first/world-api-trace.jsonl.gz")?)?;
                        let saved=parent.clone();let saved_inputs=inputs.clone();
                        assert!(parent.service_world_window(parent_vm.vm.tick-1,&mut inputs).is_err());assert_eq!(parent,saved);assert_eq!(inputs,saved_inputs);
                        for _ in 0..24 {
                            let mut restored:crate::original_session::draw_parent::State=postcard::from_bytes(&postcard::to_allocvec(&parent)?)?;restored.validate()?;
                            let mut cold_inputs:WindowInputs=postcard::from_bytes(&postcard::to_allocvec(&inputs)?)?;cold_inputs.validate()?;
                            let mut cold_profiler=profiler.clone();
                            match parent.world_pending() {
                                Some(WorldRequest::ClientRectangle{..})=>{assert!(parent.service_world_window(parent_vm.vm.tick,&mut inputs)?);assert!(restored.service_world_window(parent_vm.vm.tick,&mut cold_inputs)?);}
                                Some(WorldRequest::Timestamp{..})=>{parent.acknowledge_world_timestamp(0,&mut profiler)?;restored.acknowledge_world_timestamp(0,&mut cold_profiler)?;}
                                Some(WorldRequest::Scan)=>{parent.advance_world_scan(&parent_vm)?;restored.advance_world_scan(&parent_vm)?;}
                                Some(WorldRequest::Compose{..}|WorldRequest::PrepareScene)=>break,
                                _=>{assert!(parent.service_child(&mut profiler)?);assert!(restored.service_child(&mut cold_profiler)?);}
                            }
                            assert_eq!(parent,restored);assert_eq!(inputs,cold_inputs);assert_eq!(profiler,cold_profiler);world_cold_cuts+=1;
                        }
                        parent.world.as_ref().unwrap().compare_native_prefix(&load("fixtures/private/goth-world-prefix-witness-20261031-second/world-prefix-trace.jsonl.gz")?,&parent_vm)?;
                        assert_eq!(parent.world_pending(),Some(WorldRequest::Compose{include_visible:1}));
                        assert!(!parent.service_child(&mut profiler)?);assert_eq!(inputs.consumed(),1);
                        world_redraw_match=parent.world_redraw_match();assert_eq!(world_redraw_match,Some(21));window_inputs_consumed=inputs.consumed();world_prefix_verified=true;world_next=parent.world_pending();
                        if finish_composition {
                            use crate::original_session::draw_parent::{WorldComposition,CursorInputs,CompositionRequest};
                            let mut view=WorldComposition::import(&load("fixtures/private/goth-world-composition-20261031-first/composition-source-fourth.json")?)?;
                            let native=load("fixtures/private/goth-world-composition-witness-20261031-fifth/world-composition-trace.jsonl.gz")?;
                            view.compare_native(&native,0x4ab992,&parent_vm,&controls)?;
                            let mut cursor=CursorInputs::import(&load("fixtures/private/goth-world-composition-witness-20261031-second/world-composition-trace.jsonl.gz")?)?;
                            for _ in 0..12 {
                                let mut restored:crate::original_session::draw_parent::State=postcard::from_bytes(&postcard::to_allocvec(&parent)?)?;
                                let mut cold_view:WorldComposition=postcard::from_bytes(&postcard::to_allocvec(&view)?)?;cold_view.validate()?;
                                let mut cold_inputs:CursorInputs=postcard::from_bytes(&postcard::to_allocvec(&cursor)?)?;cold_inputs.validate()?;
                                if matches!(view.pending(),Some(CompositionRequest::GetCursorPos|CompositionRequest::ScreenToClient{..})) {
                                    let before_view=view.clone();let before_inputs=cursor.clone();assert!(cursor.service(&mut view,parent_vm.vm.tick-1).is_err());assert_eq!(view,before_view);assert_eq!(cursor,before_inputs);
                                    assert!(cursor.service(&mut view,parent_vm.vm.tick)?);assert!(cold_inputs.service(&mut cold_view,parent_vm.vm.tick)?);
                                } else {assert!(parent.advance_world_composition(&mut view,&parent_vm,&controls)?);assert!(restored.advance_world_composition(&mut cold_view,&parent_vm,&controls)?);}
                                assert_eq!(parent,restored);assert_eq!(view,cold_view);assert_eq!(cursor,cold_inputs);world_cold_cuts+=1;
                                if view.pending()==Some(CompositionRequest::GetCursorPos) {view.compare_native(&native,0x4abca4,&parent_vm,&controls)?;}
                                if view.projected.is_some() {view.compare_native(&native,0x4abd68,&parent_vm,&controls)?;view.compare_native(&native,0x4abe87,&parent_vm,&controls)?;}
                                if view.complete(){break;}
                            }
                            assert!(view.complete());view.compare_native(&native,0x4ac078,&parent_vm,&controls)?;
                            assert_eq!(cursor.consumed(),2);cursor_inputs_consumed=cursor.consumed();
                            assert_eq!(parent.world_pending(),Some(WorldRequest::PrepareScene));assert!(!parent.service_child(&mut profiler)?);
                            world_composition_verified=true;world_next=parent.world_pending();
                            if finish_scene {
                                use crate::original_session::draw_parent::WorldScene;
                                let native=load("fixtures/private/goth-world-scene-witness-20261031-fifth/world-scene-trace.jsonl.gz")?;
                                let mut scene=WorldScene::import(&load("fixtures/private/goth-world-scene-20261031-first/scene-source-second.json")?,&parent_vm)?;
                                scene.compare_native(&native,0x4a8b01,&parent_vm,&view)?;
                                for _ in 0..128 {
                                    let mut cold_parent:crate::original_session::draw_parent::State=postcard::from_bytes(&postcard::to_allocvec(&parent)?)?;
                                    let mut cold_view:WorldComposition=postcard::from_bytes(&postcard::to_allocvec(&view)?)?;
                                    let mut cold_scene:WorldScene=postcard::from_bytes(&postcard::to_allocvec(&scene)?)?;cold_scene.validate()?;
                                    let mut cold_vm:sims_sim::Simulation=serde_json::from_slice(&serde_json::to_vec(&parent_vm)?)?;
                                    assert!(parent.advance_world_scene(&mut scene,&mut view,&mut parent_vm)?);
                                    assert!(cold_parent.advance_world_scene(&mut cold_scene,&mut cold_view,&mut cold_vm)?);
                                    assert_eq!(parent,cold_parent);assert_eq!(scene,cold_scene);assert_eq!(view,cold_view);assert_eq!(sims_sim::Snapshot::capture(&program,&parent_vm),sims_sim::Snapshot::capture(&program,&cold_vm));world_cold_cuts+=1;world_scene_cold_cuts+=1;
                                    if scene.after_rooms(){scene.compare_native(&native,0x4a88b0,&parent_vm,&view)?;}
                                    if scene.after_history(){scene.compare_native(&native,0x4a88dd,&parent_vm,&view)?;}
                                    if scene.before_clear(){scene.compare_native(&native,0x4a8917,&parent_vm,&view)?;}
                                    if scene.complete(){break;}
                                }
                                assert!(scene.complete());scene.compare_native(&native,0x4a8b08,&parent_vm,&view)?;
                                world_scene_invalidations=scene.invalidations.clone();
                                world_scene_rectangles=(0..scene.regions.len()).map(|i|scene.regions.rectangle(i).map_err(anyhow::Error::msg)).collect::<Result<_>>()?;
                                assert_eq!(parent.world_pending(),Some(WorldRequest::RefreshIndicators));assert!(!parent.service_child(&mut profiler)?);
                                world_scene_verified=true;world_next=parent.world_pending();
                                if finish_classification {
                                    use crate::original_session::draw_parent::WorldClassification;
                                    let source=load("fixtures/private/goth-world-classification-20261031-first/classification-source-third.json")?;
                                    let native=load("fixtures/private/goth-world-classification-storage-20261031-second/world-classification-trace.jsonl.gz")?;
                                    let mut classify=WorldClassification::import(&source,&parent_vm)?;
                                    classify.compare_native(&native,0x4a8b08,&parent_vm,&retained.ordinary,&scene)?;
                                    for _ in 0..256 {
                                        if matches!(classify.pending(),Some(crate::original_session::draw_parent::ClassificationRequest::Invalidate(_))){classify.compare_invalidation(&native,&parent_vm)?;classification_callback_checks+=1;}
                                        let mut cold_parent:crate::original_session::draw_parent::State=postcard::from_bytes(&postcard::to_allocvec(&parent)?)?;
                                        let mut cold_scene:WorldScene=postcard::from_bytes(&postcard::to_allocvec(&scene)?)?;
                                        let mut cold_classify:WorldClassification=postcard::from_bytes(&postcard::to_allocvec(&classify)?)?;
                                        let mut cold_vm:sims_sim::Simulation=serde_json::from_slice(&serde_json::to_vec(&parent_vm)?)?;
                                        assert!(parent.advance_world_classification(&mut classify,&mut parent_vm,&retained.ordinary,&mut scene,&view)?);
                                        assert!(cold_parent.advance_world_classification(&mut cold_classify,&mut cold_vm,&retained.ordinary,&mut cold_scene,&view)?);
                                        assert_eq!(parent,cold_parent);assert_eq!(classify,cold_classify);assert_eq!(scene,cold_scene);assert_eq!(sims_sim::Snapshot::capture(&program,&parent_vm),sims_sim::Snapshot::capture(&program,&cold_vm));classification_cold_cuts+=1;
                                        if classification_cold_cuts==1 {classify.compare_native(&native,0x4a8b17,&parent_vm,&retained.ordinary,&scene)?;}
                                        if classify.complete(){break;}
                                    }
                                    assert!(classify.complete());classify.compare_native(&native,0x4a8b1e,&parent_vm,&retained.ordinary,&scene)?;
                                    assert_eq!(parent.world_pending(),Some(WorldRequest::PrepareCamera));assert!(!parent.service_child(&mut profiler)?);
                                    world_classification_verified=true;classification_invalidations=classify.invalidations.clone();world_next=parent.world_pending();
                                    if finish_camera {
                                        let native=load("fixtures/private/goth-world-camera-prepare-20261031-second/world-classification-trace.jsonl.gz")?;
                                        parent.world_camera().unwrap().compare_native(&native,0x4a8b1e)?;
                                        let before_camera=sims_sim::Snapshot::capture(&program,&parent_vm);
                                        for _ in 0..8 {
                                            let mut cold_parent:crate::original_session::draw_parent::State=postcard::from_bytes(&postcard::to_allocvec(&parent)?)?;
                                            let mut cold_view:WorldComposition=postcard::from_bytes(&postcard::to_allocvec(&view)?)?;
                                            assert!(parent.advance_world_camera(&mut view)?);assert!(cold_parent.advance_world_camera(&mut cold_view)?);
                                            assert_eq!(parent,cold_parent);assert_eq!(view,cold_view);camera_cold_cuts+=1;
                                            if parent.world_camera().unwrap().complete(){break;}
                                        }
                                        assert!(parent.world_camera().unwrap().complete());parent.world_camera().unwrap().compare_native(&native,0x4a8bad)?;
                                        assert_eq!(sims_sim::Snapshot::capture(&program,&parent_vm),before_camera);
                                        assert_eq!(parent.world_pending(),Some(WorldRequest::PersonVisibility));assert!(!parent.service_child(&mut profiler)?);
                                        world_camera_verified=true;world_next=parent.world_pending();
                                        if finish_people {
                                            use std::io::Read as _;
                                            let native=load("fixtures/private/goth-world-person-visibility-witness-20261031-fourth/world-classification-trace.jsonl.gz")?;
                                            ensure!(sims_fixtures::digest(&native)=="baf64c28cf8cb428ab7a71a0e5c0f6f71d6564b5c208afc9875c13072901b629","person projection witness differs");
                                            let mut text=String::new();flate2::read::GzDecoder::new(native.as_slice()).read_to_string(&mut text)?;
                                            let rows=text.lines().map(serde_json::from_str::<serde_json::Value>).collect::<std::result::Result<Vec<_>,_>>()?;
                                            let rows:Vec<_>=rows.iter().filter(|r|r["kind"]=="world_person_visibility").collect();
                                            let before_people=sims_sim::Snapshot::capture(&program,&parent_vm);
                                            let cold_path=root.join(if finish_background{"fixtures/private/goth-window-background-20261031-first/person-cold-first.store"}else if finish_surfaces{"fixtures/private/goth-world-surfaces-20261031-first/person-cold-first.store"}else if finish_paint{"fixtures/private/goth-world-paint-20261031-first/person-cold-first.store"}else if finish_drawing{"fixtures/private/goth-world-drawing-20261031-first/person-cold-first.store"}else{"fixtures/private/goth-world-person-visibility-20261031-first/person-cold-first.store"});
                                            let cold_store=if cold_path.exists(){sims_fixtures::Store::open(&cold_path)?}else{sims_fixtures::Store::create(&cold_path)?};
                                            let assets_blob=cold_store.put_value(&retained.characters)?;
                                            let cold_assets:std::collections::BTreeMap<sims_vm::ObjectId,sims_content::character_loading::live::SavedAssets>=cold_store.get_value(&assets_blob)?;
                                            assert_eq!(retained.characters,cold_assets);
                                            for row in &rows {
                                                let pc=row["pc"].as_u64().ok_or_else(||anyhow::anyhow!("person native stop absent"))? as u32;
                                                for _ in 0..16 {
                                                    if parent.world_people().unwrap().at_native_cut(pc){break;}
                                                    let cut=cold_store.put_value(&(&parent,&classify,&parent_vm,retained.presentation.visibility))?;
                                                    let(mut cold_parent,mut cold_classify,mut cold_vm,cold_source):(crate::original_session::draw_parent::State,WorldClassification,sims_sim::Simulation,crate::original_session::character_visibility::Source)=cold_store.get_value(&cut)?;
                                                    person_visibility_resume_inputs.push(serde_json::json!({"cut":cut,"assets":assets_blob}));
                                                    assert!(parent.advance_world_people(&mut parent_vm,&retained.characters,retained.presentation.visibility,&mut classify)?);
                                                    assert!(cold_parent.advance_world_people(&mut cold_vm,&cold_assets,cold_source,&mut cold_classify)?);
                                                    assert_eq!(parent,cold_parent);assert_eq!(classify,cold_classify);
                                                    assert_eq!(sims_sim::Snapshot::capture(&program,&parent_vm),sims_sim::Snapshot::capture(&program,&cold_vm));
                                                    person_visibility_cold_cuts+=1;
                                                }
                                                parent.world_people().unwrap().compare_row(row,&parent_vm,&retained.characters,retained.presentation.visibility,parent.world_camera().unwrap().viewport(),&classify)?;
                                                if pc==0x4a8be3 {
                                                    let id=row["people"].as_array().unwrap().iter().find(|p|p["pointer"]==row["person"]).and_then(|p|p["id"].as_i64()).ok_or_else(||anyhow::anyhow!("native person identity absent"))? as sims_vm::ObjectId;
                                                    let v=retained.presentation.visibility;let viewport=parent.world_camera().unwrap().viewport();
                                                    assert_eq!(v.visible_person(&parent_vm.characters[&id],Some(&retained.characters[&id].model),viewport)?,crate::original_session::character_visibility::Source{enabled:!v.enabled,..v}.visible_person(&parent_vm.characters[&id],Some(&retained.characters[&id].model),viewport)?);
                                                    person_visibility_calls+=1;
                                                }
                                            }
                                            assert!(parent.world_people().unwrap().complete());
                                            let mut expected_people=before_people;
                                            for &(id,value) in &parent.world_people().unwrap().results {expected_people.state.characters.get_mut(&id).unwrap().character.visible=value;}
                                            assert_eq!(sims_sim::Snapshot::capture(&program,&parent_vm),expected_people);
                                            assert_eq!(person_visibility_calls,4);assert_eq!(rows.len(),22);
                                            assert!(parent.world_people().unwrap().rectangle_clears.is_empty());
                                            assert_eq!(parent.world_pending(),Some(WorldRequest::PrepareDrawing));assert!(!parent.service_child(&mut profiler)?);
                                            person_visibility_verified=true;world_next=parent.world_pending();
                                            if finish_drawing {
                                                let native=load("fixtures/private/goth-world-drawing-witness-20261031-second/world-classification-trace.jsonl.gz")?;
                                                ensure!(sims_fixtures::digest(&native)=="1839dfa121db8607b64ed72ae5574106e1405510a48dd630be28007bd46eca66","drawing witness differs");
                                                let mut text=String::new();flate2::read::GzDecoder::new(native.as_slice()).read_to_string(&mut text)?;
                                                let rows=text.lines().map(serde_json::from_str::<serde_json::Value>).collect::<std::result::Result<Vec<_>,_>>()?;
                                                let rows:Vec<_>=rows.iter().filter(|r|r["kind"]=="world_drawing_setup").collect();
                                                let marker=rows.iter().find(|r|r["pc"]==0x4a8c1cu32).ok_or_else(||anyhow::anyhow!("drawing original marker absent"))?;
                                                let raw=crate::original_session::completed_animations::bytes(marker["world_stack"]["raw"].as_str().unwrap())?;
                                                let timestamp=u64::from_le_bytes(raw[0x130-0x48..0x130-0x40].try_into()?);
                                                let before_drawing=sims_sim::Snapshot::capture(&program,&parent_vm);let before_profiler=profiler.records().to_vec();
                                                let path=root.join(if finish_background{"fixtures/private/goth-window-background-20261031-first/drawing-cold-first.store"}else if finish_surfaces{"fixtures/private/goth-world-surfaces-20261031-first/drawing-cold-first.store"}else if finish_paint{"fixtures/private/goth-world-paint-20261031-first/drawing-cold-first.store"}else{"fixtures/private/goth-world-drawing-20261031-first/drawing-cold-first.store"});
                                                let store=if path.exists(){sims_fixtures::Store::open(path)?}else{sims_fixtures::Store::create(path)?};
                                                for row in &rows {
                                                    let pc=row["pc"].as_u64().unwrap() as u32;
                                                    for _ in 0..128 {
                                                        if parent.world_drawing().unwrap().at_native_cut(pc){break;}
                                                        let cut=store.put_value(&(&parent,&scene,&view,&parent_vm,&profiler))?;
                                                        let(mut cold_parent,cold_scene,cold_view,cold_vm,mut cold_profiler):(crate::original_session::draw_parent::State,WorldScene,WorldComposition,sims_sim::Simulation,Buffer)=store.get_value(&cut)?;
                                                        if matches!(parent.world_pending(),Some(WorldRequest::Timestamp{name:0x6c72f8})) {
                                                            parent.acknowledge_world_timestamp(timestamp,&mut profiler)?;cold_parent.acknowledge_world_timestamp(timestamp,&mut cold_profiler)?;
                                                        }else{
                                                            assert!(parent.advance_world_drawing(&mut profiler)?,"drawing stalled at{:?}",parent.world_drawing().unwrap().pending());
                                                            assert!(cold_parent.advance_world_drawing(&mut cold_profiler)?);
                                                        }
                                                        assert_eq!(parent,cold_parent);assert_eq!(scene,cold_scene);assert_eq!(view,cold_view);assert_eq!(profiler,cold_profiler);
                                                        assert_eq!(sims_sim::Snapshot::capture(&program,&parent_vm),sims_sim::Snapshot::capture(&program,&cold_vm));
                                                        drawing_resume_inputs.push(cut);
                                                    }
                                                    parent.compare_world_drawing(row,&scene,&profiler)?;drawing_native_stops+=1;
                                                }
                                                assert_eq!(drawing_native_stops,15);assert!(parent.world_drawing().unwrap().complete());
                                                assert_eq!(sims_sim::Snapshot::capture(&program,&parent_vm),before_drawing);
                                                assert_eq!(&profiler.records()[..before_profiler.len()],before_profiler.as_slice());assert_eq!(profiler.records().len(),before_profiler.len()+1);
                                                assert_eq!(parent.world_pending(),Some(WorldRequest::PaintWorld));assert!(!parent.service_child(&mut profiler)?);
                                                drawing_verified=true;world_next=parent.world_pending();
                                                if finish_paint {
                                                    let native=load("fixtures/private/goth-world-paint-witness-20261031-fourth/world-classification-trace.jsonl.gz")?;
                                                    ensure!(sims_fixtures::digest(&native)=="e28e40533223f983568a98e7407e459ed243e4c1dbaf268c0f10fafe1676e154","paint inner witness differs");
                                                    let mut text=String::new();flate2::read::GzDecoder::new(native.as_slice()).read_to_string(&mut text)?;
                                                    let rows=text.lines().map(serde_json::from_str::<serde_json::Value>).collect::<std::result::Result<Vec<_>,_>>()?;
                                                    let pcs=[0x4a9262u32,0x4a9297,0x4aceb8,0x4acf1c,0x4acf29];
                                                    let rows:Vec<_>=rows.iter().filter(|r|r["kind"]=="world_drawing_setup"&&pcs.contains(&(r["pc"].as_u64().unwrap()as u32))).collect();
                                                    ensure!(rows.len()==5,"paint original cut coverage differs");
                                                    let marker=crate::original_session::completed_animations::bytes(rows[1]["profiler"]["used_raw"].as_str().unwrap())?;
                                                    let time=u64::from_le_bytes(marker[marker.len()-16..marker.len()-8].try_into()?);
                                                    let before=sims_sim::Snapshot::capture(&program,&parent_vm);
                                                    let path=root.join(if finish_background{"fixtures/private/goth-window-background-20261031-first/paint-cold-first.store"}else if finish_surfaces{"fixtures/private/goth-world-surfaces-20261031-first/paint-cold-first.store"}else{"fixtures/private/goth-world-paint-20261031-first/paint-cold-first.store"});
                                                    let store=if path.exists(){sims_fixtures::Store::open(path)?}else{sims_fixtures::Store::create(path)?};
                                                    for row in rows {
                                                        let pc=row["pc"].as_u64().unwrap()as u32;
                                                        for _ in 0..16 {
                                                            if parent.world_paint().unwrap().at_native_cut(pc){break;}
                                                            let cut=store.put_value(&(&parent,&scene,&view,&parent_vm,&profiler))?;
                                                            let(mut cold_parent,mut cold_scene,cold_view,cold_vm,mut cold_profiler):(crate::original_session::draw_parent::State,WorldScene,WorldComposition,sims_sim::Simulation,Buffer)=store.get_value(&cut)?;
                                                            if matches!(parent.world_pending(),Some(WorldRequest::Timestamp{name:0x6c735c})) {
                                                                parent.acknowledge_world_timestamp(time,&mut profiler)?;cold_parent.acknowledge_world_timestamp(time,&mut cold_profiler)?;
                                                            }else{
                                                                assert!(parent.advance_world_paint(&mut scene,&mut profiler)?,"paint stalled at{:?}",parent.world_paint().unwrap().pending());
                                                                assert!(cold_parent.advance_world_paint(&mut cold_scene,&mut cold_profiler)?);
                                                            }
                                                            assert_eq!(parent,cold_parent);assert_eq!(scene,cold_scene);assert_eq!(view,cold_view);assert_eq!(profiler,cold_profiler);
                                                            assert_eq!(sims_sim::Snapshot::capture(&program,&parent_vm),sims_sim::Snapshot::capture(&program,&cold_vm));
                                                            paint_resume_inputs.push(cut);
                                                        }
                                                        parent.compare_world_paint(row,&scene,&profiler)?;paint_native_stops+=1;
                                                    }
                                                    assert_eq!(sims_sim::Snapshot::capture(&program,&parent_vm),before);
                                                    paint_next=parent.world_paint().unwrap().pending();
                                                    if !finish_surfaces {assert!(!parent.advance_world_paint(&mut scene,&mut profiler)?);}
                                                    assert_eq!(parent.world_pending(),Some(WorldRequest::PaintWorld));
                                                    paint_verified=true;
                                                    if finish_surfaces {
                                                        let native=load("fixtures/private/goth-world-surface-witness-20261031-first/world-classification-trace.jsonl.gz")?;
                                                        ensure!(sims_fixtures::digest(&native)=="2404b08975f88c3aed37fb32bf2da4ec901526774aacb4d36cd91017485fac52","surface native witness differs");
                                                        let mut text=String::new();flate2::read::GzDecoder::new(native.as_slice()).read_to_string(&mut text)?;
                                                        let rows=text.lines().map(serde_json::from_str::<serde_json::Value>).collect::<std::result::Result<Vec<_>,_>>()?;
                                                        let rows:Vec<_>=rows.iter().filter(|r|r["kind"]=="world_drawing_setup"&&!r["surface_state"].is_null()).collect();ensure!(rows.len()==34,"surface native cut coverage differs");
                                                        let before=sims_sim::Snapshot::capture(&program,&parent_vm);let prior_profiler=profiler.clone();
                                                        let path=root.join(if finish_background{"fixtures/private/goth-window-background-20261031-first/surface-cold-first.store"}else{"fixtures/private/goth-world-surfaces-20261031-first/surface-cold-first.store"});
                                                        let store=if path.exists(){sims_fixtures::Store::open(path)?}else{sims_fixtures::Store::create(path)?};
                                                        for row in rows {
                                                            for _ in 0..128 {
                                                                if parent.world_paint().unwrap().at_surface_cut(row){break;}
                                                                let cut=store.put_value(&(&parent,&scene,&view,&parent_vm,&profiler))?;
                                                                let(mut cold_parent,mut cold_scene,cold_view,cold_vm,mut cold_profiler):(crate::original_session::draw_parent::State,WorldScene,WorldComposition,sims_sim::Simulation,Buffer)=store.get_value(&cut)?;
                                                                assert!(parent.advance_world_paint(&mut scene,&mut profiler)?,"surface stalled before PC{:x}",row["pc"].as_u64().unwrap());
                                                                assert!(cold_parent.advance_world_paint(&mut cold_scene,&mut cold_profiler)?);
                                                                assert_eq!(parent,cold_parent);assert_eq!(scene,cold_scene);assert_eq!(view,cold_view);assert_eq!(profiler,cold_profiler);
                                                                assert_eq!(sims_sim::Snapshot::capture(&program,&parent_vm),sims_sim::Snapshot::capture(&program,&cold_vm));surfaces_resume_inputs.push(cut);
                                                            }
                                                            parent.compare_world_surfaces(row)?;surfaces_native_stops+=1;
                                                        }
                                                        assert_eq!(sims_sim::Snapshot::capture(&program,&parent_vm),before);assert_eq!(profiler,prior_profiler);
                                                        surfaces_inputs=parent.world_paint().unwrap().surface_inputs_consumed().unwrap();assert_eq!(surfaces_inputs,2);
                                                        if !finish_background {assert!(!parent.advance_world_paint(&mut scene,&mut profiler)?);}
                                                        paint_next=parent.world_paint().unwrap().pending();assert_eq!(format!("{paint_next:?}"),"Some(WindowContents)");
                                                        surfaces_verified=true;
                                                        if finish_background {
                                                            let native=load("fixtures/private/goth-window-background-witness-20261031-third/world-classification-trace.jsonl.gz")?;
                                                            ensure!(sims_fixtures::digest(&native)=="c1e7b237d08581694b47272d4ab0c52901bed20a73dc83b1bc850c1b43f10e6c","background original witness differs");
                                                            let mut text=String::new();flate2::read::GzDecoder::new(native.as_slice()).read_to_string(&mut text)?;
                                                            let rows=text.lines().map(serde_json::from_str::<serde_json::Value>).collect::<std::result::Result<Vec<_>,_>>()?;
                                                            let pcs=[0x4acf4e,0x4b3e06,0x4acfb5,0x4ad0a9,0x4ad0ee,0x4ad19e];
                                                            let rows:Vec<_>=rows.iter().filter(|r|r["kind"]=="background_offset"||r["kind"]=="world_drawing_setup"&&pcs.contains(&(r["pc"].as_u64().unwrap()as u32))).collect();
                                                            ensure!(rows.len()==538,"background original cut coverage differs");
                                                            let before=sims_sim::Snapshot::capture(&program,&parent_vm);let previous_profiler=profiler.clone();
                                                            let path=root.join("fixtures/private/goth-window-background-20261031-first/background-cold-first.store");let store=if path.exists(){sims_fixtures::Store::open(path)?}else{sims_fixtures::Store::create(path)?};
                                                            for(index,row)in rows.into_iter().enumerate(){
                                                                for _ in 0..2048{if parent.world_paint().unwrap().at_background_cut(row){break;}assert!(parent.advance_world_paint(&mut scene,&mut profiler)?,"background stalled before PC{:x}",row["pc"].as_u64().unwrap());}
                                                                parent.compare_world_background(row,&scene)?;background_stops+=1;
                                                                if [0,1,2,67,135,269,400,536].contains(&index){
                                                                    let cut=store.put_value(&(&parent,&scene,&view,&parent_vm,&profiler))?;
                                                                    let(mut cold_parent,mut cold_scene,cold_view,cold_vm,mut cold_profiler):(crate::original_session::draw_parent::State,WorldScene,WorldComposition,sims_sim::Simulation,Buffer)=store.get_value(&cut)?;
                                                                    assert!(parent.advance_world_paint(&mut scene,&mut profiler)?);assert!(cold_parent.advance_world_paint(&mut cold_scene,&mut cold_profiler)?);
                                                                    assert_eq!(parent,cold_parent);assert_eq!(scene,cold_scene);assert_eq!(view,cold_view);assert_eq!(profiler,cold_profiler);assert_eq!(sims_sim::Snapshot::capture(&program,&parent_vm),sims_sim::Snapshot::capture(&program,&cold_vm));background_cold.push(cut);
                                                                }else if index<537{assert!(parent.advance_world_paint(&mut scene,&mut profiler)?,"background did not leave its matched native epoch");}
                                                            }
                                                            assert_eq!(sims_sim::Snapshot::capture(&program,&parent_vm),before);assert_eq!(profiler,previous_profiler);assert!(!parent.advance_world_paint(&mut scene,&mut profiler)?);
                                                            paint_next=parent.world_paint().unwrap().pending();assert_eq!(format!("{paint_next:?}"),"Some(WindowTiles)");background_verified=true;
                                                        }

                                                    }

                                                }

                                            }

                                        }

                                    }

                                }

                            }
                        }
                        if !finish_scene {assert_eq!(sims_sim::Snapshot::capture(&program,&parent_vm),before_framework);}
                    }
                }
            }
        }
        let mut expected_state=before;expected_state.state.vm.world.actions.as_mut().unwrap().actors.get_mut(&21).unwrap().active.as_mut().unwrap().flags &=!128;
        assert_eq!(sims_sim::Snapshot::capture(&program,&simulation),expected_state);
        let raw=owner.state.records[0];assert_eq!(actions::record::word(&raw,0),326);assert_eq!(&raw[8..10],&[0,1]);assert_eq!(actions::record::word(&raw,12),0x0c3e0180);
        assert_eq!([16,20,24].map(|off|actions::record::word(&raw,off)),[0xf0000010,0xf000001b,0xf000001c]);
        if let Some(path)=std::env::var_os("SIMS_RECORD_C8_REPORT") {
            use std::io::Write;
            let path=std::path::PathBuf::from(path);let path=if path.is_absolute(){path}else{root.join(path)};
            let mut value=serde_json::json!({"schema":1,"kind":"real_retained_C8_collection_diagnostic","native_parity_credit":0,"input":reference,"tick":simulation.vm.tick,"record":raw,"flags_before":flags,"flags_after":flags &!128,"all_other_simulation_state_unchanged":true,"owner_binary_restore_equal":true,"native_output_pixels_available":false,"image_publication":publish,"full_original_caller_exit_match":finish_tail,"enclosing_draw_framework_pending":enclosing_draw_framework_pending,"enclosing_draw_world_pending":enclosing_draw_world_pending,"world_context_verified":world_context_verified,"world_prefix_verified":world_prefix_verified,"world_composition_verified":world_composition_verified,"world_camera_verified":world_camera_verified,"camera_cold_cuts":camera_cold_cuts,"world_classification_verified":world_classification_verified,"classification_cold_cuts":classification_cold_cuts,"classification_invalidations":classification_invalidations,"classification_callback_checks":classification_callback_checks,"world_scene_verified":world_scene_verified,"world_scene_cold_cuts":world_scene_cold_cuts,"world_scene_invalidations":world_scene_invalidations,"world_scene_rectangles":world_scene_rectangles,"simulation_unchanged_scope":"action collection excludes its flag-128 acknowledgement; World scene has its separately native-compared room/refresh mutations","cursor_inputs_consumed":cursor_inputs_consumed,"world_next":world_next,"world_redraw_match":world_redraw_match,"window_inputs_consumed":window_inputs_consumed,"world_cold_cuts":world_cold_cuts,"enclosing_draw_scope":"controlled profiler timestamps; captured FPS clock; explicit original OS window replies; no performance-counter stream or whole Session qualification","publication_cuts":publication_cuts,"original_window_bytes_equal_after_bitmap_identity_mapping":native_window_compared,"program_scope":"cached label branch only; default unused Program; no full Source/Session admission"});
            value["background"]=serde_json::json!({"verified":background_verified,"native_stops":background_stops,"cold_cuts":background_cold.len(),"resume_inputs":background_cold,"expected_state_injected":false,"whole_paint_completed":false});
            value["surfaces"]=serde_json::json!({"verified":surfaces_verified,"native_stops":surfaces_native_stops,"cold_cuts":surfaces_resume_inputs.len(),"resume_inputs":surfaces_resume_inputs,"DirectDraw_inputs_consumed":surfaces_inputs,"whole_paint_completed":false});
            value["paint"]=serde_json::json!({"verified":paint_verified,"native_stops":paint_native_stops,"cold_cuts":paint_resume_inputs.len(),"resume_inputs":paint_resume_inputs,"next":paint_next,"whole_paint_completed":false});
            value["drawing"]=serde_json::json!({"verified":drawing_verified,"native_stops":drawing_native_stops,"cold_cuts":drawing_resume_inputs.len(),"resume_inputs":drawing_resume_inputs,"shared_framework_owner":true});
            value["person_visibility"]=serde_json::json!({"verified":person_visibility_verified,"cold_cuts":person_visibility_cold_cuts,"calls":person_visibility_calls,"resume_inputs":person_visibility_resume_inputs});
            let mut file=std::fs::OpenOptions::new().write(true).create_new(true).open(path)?;file.write_all(&serde_json::to_vec_pretty(&value)?)?;
        }
        Ok(())
    }

}
