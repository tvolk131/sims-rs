use super::*;
use clap::Parser;
use sims_content::{Resource, character_loading::live, owned_resources::Neighborhood};
use sims_formats::behavior::Instruction;
use sims_runtime::original_session::{Options, View, managed};
use sims_sim::{
    Simulation, Snapshot,
    architecture::Architecture,
    lot_geometry::{FloorSource, Lot},
    lot_links::Links,
    lot_placement,
    lot_runtime::State as LotState,
    pose::animator::Settings,
};
use sims_vm::{
    Definition, Object, Program, Vm, World, autonomy::Placement, person::WorldState,
    person_loading::RoomMembership,
};
use std::{collections::BTreeMap, path::PathBuf};

pub(super) const GLOBAL: &str = "GameData_ALL_Recursive/Global/Global.iff";
pub(super) const HOUSE: &str = "UserData/Houses/House07.iff";
const NEIGHBORHOOD: &str = "UserData/Neighborhood.iff";

fn resource(root: &std::path::Path, locator: &str, name: &str, bytes: Vec<u8>) -> Resource {
    Resource {
        locator: locator.into(),
        name: name.into(),
        source: root.join(locator),
        offset: 0,
        size: bytes.len() as u64,
        unpacked_size: bytes.len() as u64,
        archived: false,
        owned: Some(Arc::from(bytes)),
    }
}

fn iff(chunks: Vec<([u8; 4], u16, Vec<u8>)>) -> Vec<u8> {
    let mut bytes = b"IFF FILE 2.5:TYPE FOLLOWED BY SIZE\0 JAMIE DOORNBOS & MAXIS 1".to_vec();
    bytes.resize(64, 0);
    for (tag, identifier, payload) in chunks {
        bytes.extend(tag);
        bytes.extend(((76 + payload.len()) as u32).to_be_bytes());
        bytes.extend(identifier.to_be_bytes());
        bytes.extend(0_u16.to_be_bytes());
        bytes.extend([0; 64]);
        bytes.extend(payload);
    }
    bytes
}

fn instruction(opcode: u16, operands: [u8; 8], next: u8) -> Instruction {
    Instruction {
        opcode,
        operands,
        on_true: next,
        on_false: 255,
    }
}

fn behavior(instructions: Vec<Instruction>) -> Vec<u8> {
    let mut bytes = Vec::from(0x8002_u16.to_le_bytes());
    bytes.extend((instructions.len() as u16).to_le_bytes());
    bytes.extend([0; 8]);
    for instruction in instructions {
        bytes.extend(instruction.opcode.to_le_bytes());
        bytes.extend([instruction.on_true, instruction.on_false]);
        bytes.extend(instruction.operands);
    }
    bytes
}

fn global() -> Vec<u8> {
    let returned = instruction(2, [0, 0, 0, 0, 0, 5, 8, 8], 254);
    let mut chunks = vec![];
    for (definition, main) in [(1_u16, 4096_u16), (2, 4097), (4, 4110)] {
        let mut words = vec![0_i16; 108];
        words[0] = 138;
        words[9] = 4;
        words[14] = definition as i16;
        words[67] = 8;
        chunks.push((
            *b"OBJD",
            definition,
            words.into_iter().flat_map(i16::to_le_bytes).collect(),
        ));
        let mut functions = vec![0; 8];
        functions.extend(b"fJBO");
        functions.extend(27_u32.to_le_bytes());
        for slot in 0..27 {
            functions.extend(0_u16.to_le_bytes());
            functions.extend(if slot == 1 { main } else { 0 }.to_le_bytes());
        }
        chunks.push((*b"OBJf", definition, functions));
        let instructions = if main == 4096 {
            vec![
                instruction(8, [0, 0, 8, 0, 19, 0, 7, 0], 1),
                instruction(42, [2, 0, 0, 0, 6, 0, 0, 0], 2),
                instruction(8, [0, 0, 8, 0, 19, 0, 7, 0], 254),
            ]
        } else {
            vec![returned.clone()]
        };
        chunks.push((*b"BHAV", main, behavior(instructions)));
    }
    for identifier in [1, 5, 6] {
        let mut constants = Vec::from(0_u32.to_le_bytes());
        constants.extend(1_u32.to_le_bytes());
        constants.extend(b"SNCF");
        constants.extend(0_u32.to_le_bytes());
        chunks.push((*b"FCNS", identifier, constants));
    }
    for identifier in [501, 502, 503, 504] {
        let mut strings = Vec::from(0_i16.to_le_bytes());
        strings.extend(9_u16.to_le_bytes());
        strings.extend([0; 9]);
        chunks.push((*b"STR#", identifier, strings));
    }
    iff(chunks)
}

fn bitmap(width: u32, height: u32) -> Result<Vec<u8>> {
    let image = image::RgbImage::from_pixel(width, height, image::Rgb([23, 47, 71]));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Bmp)?;
    Ok(bytes.into_inner())
}

fn font() -> Vec<u8> {
    let mut bytes = vec![0; 32];
    bytes[..4].copy_from_slice(b"FNTF");
    bytes[8..10].copy_from_slice(&101_u16.to_le_bytes());
    bytes[10..12].copy_from_slice(&1_u16.to_le_bytes());
    bytes[12..16].copy_from_slice(&9_u32.to_le_bytes());
    bytes[20..24].copy_from_slice(&32_u32.to_le_bytes());
    bytes[28..32].copy_from_slice(&43_u32.to_le_bytes());
    bytes.extend(65_u16.to_le_bytes());
    bytes.extend([1, 23, 0, 0, 0, 0, 0, 0, 0]);
    bytes.extend(0x7a_u32.to_le_bytes());
    bytes.extend(1_u16.to_le_bytes());
    bytes.extend(23_u16.to_le_bytes());
    bytes.extend([0; 8]);
    bytes.extend([0xf0; 23]);
    let length = bytes.len() as u32;
    bytes[4..8].copy_from_slice(&length.to_le_bytes());
    bytes
}

pub(super) fn index() -> Result<Index> {
    let root = PathBuf::from("/authored-full-envelope-no-external-files");
    let mut resources = vec![
        resource(&root, GLOBAL, "Global.iff", global()),
        resource(
            &root,
            "GameData_ALL_Recursive/Global/Sprites.iff",
            "Sprites.iff",
            iff(vec![]),
        ),
    ];
    for (locator, name) in [
        (HOUSE, "House07.iff"),
        (NEIGHBORHOOD, "Neighborhood.iff"),
        ("UserData/Characters/authored.iff", "authored.iff"),
        (
            "GameData_ALL_Recursive/Objects/authored.txt",
            "authored.txt",
        ),
        (
            "GameData_ALL_Recursive/UserObjects/authored.txt",
            "authored.txt",
        ),
    ] {
        resources.push(resource(&root, locator, name, iff(vec![])));
    }
    for binding in sims_content::avatar_installation::complete_collection_asset_bindings(&root) {
        let directory = binding.physical.strip_prefix(&root)?.to_string_lossy();
        let locator = format!("{directory}/authored.txt");
        resources.push(resource(&root, &locator, "authored.txt", vec![]));
    }
    let identifiers = [
        150, 151, 152, 153, 154, 155, 156, 157, 2003, 2008, 2009, 2010, 2011, 2012, 2013, 2014,
        2023, 2024, 2037, 4500, 4501, 4502, 4503, 4504, 4505, 4506, 4507, 4510, 4705, 4706, 4707,
        4803, 4804, 4805, 4806, 4910, 4911, 4916, 4917, 4918, 4950, 4951, 4952, 4957, 4958, 4959,
        2026,
    ];
    let mut table = String::new();
    for identifier in identifiers {
        let name = format!("authored/{identifier}.bmp");
        let dimensions = match identifier {
            2026 => [60, 30],
            4506 | 4510 => [27, 25],
            _ => [4, 1],
        };
        resources.push(resource(
            &root,
            &format!("UIGraphics/{name}"),
            &name,
            bitmap(dimensions[0], dimensions[1])?,
        ));
        table.push_str(&format!("{{ BITMAP, {identifier}, \"{name}\" }}\n"));
    }
    for name in [
        "Res_CPanel.h",
        "Res_Nbhd.h",
        "Res_Other.h",
        "Res_Languages.h",
    ] {
        resources.push(resource(&root, &format!("UIGraphics/{name}"), name, vec![]));
    }
    for (name, bytes) in [
        ("Res_CPanel.RT", table.into_bytes()),
        ("Res_Nbhd.RT", vec![]),
        ("Res_Other.RT", vec![]),
    ] {
        resources.push(resource(&root, &format!("UIGraphics/{name}"), name, bytes));
    }
    for (name, width, height) in [
        ("CPanel/People/ThumbBack.BMP", 1, 1),
        ("CPanel/People/PeopleTemplate.bmp", 1, 1),
        ("CPanel/TutHigh.bmp", 150, 50),
        ("CPanel/Backgrounds/JobIconMultiPopup.bmp", 1, 22),
        ("CPanel/Backgrounds/PopupInfoTiles.bmp", 36, 36),
        ("Shared/Sys/WinBtn.bmp", 260, 33),
        ("Shared/Sys/MinimizeBox.bmp", 48, 11),
    ] {
        resources.push(resource(
            &root,
            &format!("UIGraphics/{name}"),
            name,
            bitmap(width, height)?,
        ));
    }
    resources.push(resource(
        &root,
        "UIGraphics/Fonts/variablesans_12.ffn",
        "Fonts/variablesans_12.ffn",
        font(),
    ));
    ensure!(resources.iter().all(|resource| resource.owned.is_some()));
    Ok(Index {
        root,
        resources,
        archives: 0,
        diagnostics: vec![],
    })
}

fn unsigned(bytes: &mut Vec<u8>, mut value: u64) {
    while value >= 128 {
        bytes.push((value as u8 & 127) | 128);
        value >>= 7;
    }
    bytes.push(value as u8);
}

fn text(bytes: &mut Vec<u8>, value: &str) {
    unsigned(bytes, value.len() as u64);
    bytes.extend(value.as_bytes());
}

fn house_metadata(sources: &BTreeMap<String, String>) -> Vec<u8> {
    let mut bytes = vec![];
    text(&mut bytes, HOUSE);
    unsigned(&mut bytes, 14);
    text(&mut bytes, NEIGHBORHOOD);
    unsigned(&mut bytes, sources.len() as u64);
    for (source, hash) in sources {
        text(&mut bytes, source);
        text(&mut bytes, hash);
    }
    bytes.extend([6, 6, 2]);
    for _ in 0..2 {
        bytes.push(36);
        bytes.extend([0; 36]);
        bytes.push(36);
        bytes.extend([0; 36 * 8]);
        bytes.push(36);
        bytes.extend([0; 36]);
        bytes.push(36);
        bytes.extend([0; 36]);
    }
    bytes.extend([0; 3]);
    bytes.push(38);
    text(&mut bytes, "authored full-envelope callback");
    bytes.extend([0; 13]);
    bytes.push(38);
    bytes.extend([0; 10]);
    bytes.push(62);
    bytes.extend([0; 4]);
    bytes.extend([7, 11]);
    bytes.extend([0; 6]);
    bytes.extend([2, 0]);
    bytes.extend([0; 6]);
    bytes.extend([0; 3]);
    bytes.push(20);
    bytes.extend([0; 30]);
    bytes.extend([0; 4]);
    bytes
}

fn simulation(program: &Program, mode: i16) -> Result<Simulation> {
    let floor = FloorSource {
        coverings: vec![0; 36],
        walls: vec![Default::default(); 36],
        object_heads: vec![0; 36],
        flags: vec![0; 36],
    };
    let lot = Lot::rebuild(
        6,
        vec![[0; 4]; 36],
        vec![0; 36],
        vec![255; 36],
        vec![floor.clone(), floor],
    )
    .map_err(anyhow::Error::msg)?;
    let mut architecture = Architecture::from_original(lot, program).map_err(anyhow::Error::msg)?;
    architecture
        .bind_footprints(program)
        .map_err(anyhow::Error::msg)?;
    let mut world = World {
        person: Some(WorldState {
            object_order: vec![2, 4, 1, 3],
            ..Default::default()
        }),
        scheduler_storage: Some(Default::default()),
        refresh: Some(Default::default()),
        ..Default::default()
    };
    world
        .globals
        .extend([(10, 1), (11, 0), (17, 0), (19, mode), (34, 0)]);
    for identifier in 1..=4 {
        let mut data: BTreeMap<_, _> = (0..72).map(|field| (field, 0)).collect();
        data.extend([
            (3, -1),
            (
                14,
                if identifier == 2 || identifier == 3 {
                    0
                } else {
                    -1
                },
            ),
            (
                26,
                match identifier {
                    2 => 3,
                    3 => 1,
                    _ => 0,
                },
            ),
            (29, if identifier == 4 { -5 } else { 0 }),
        ]);
        let definition = if identifier == 4 { 1 } else { 4 };
        let mut object = Object {
            definition: Some(Definition {
                source: GLOBAL.into(),
                object: definition,
                guid: u32::from(definition),
                slots: 0,
            }),
            data,
            native_tree_error: Some(0),
            native_flags: Some(0x240),
            ..Default::default()
        };
        world
            .register_scheduler_object(identifier, &mut object)
            .map_err(anyhow::Error::msg)?;
        object.idle.as_mut().unwrap().flags = 3;
        world.objects.insert(identifier, object);
        world.positions.insert(
            identifier,
            if identifier == 4 {
                Placement {
                    x: -32,
                    y: -16,
                    level: 2,
                }
            } else {
                Placement {
                    x: 40,
                    y: 40,
                    level: 1,
                }
            },
        );
    }
    architecture.publish_room_context(&mut world);
    let mut heads = vec![vec![0; 36]; 2];
    heads[0][14] = 1;
    let links = Links {
        size: 6,
        heads,
        next: [(1, 3), (2, 0), (3, 2), (4, 0)].into(),
    };
    links
        .publish_tile_chains(&mut world)
        .map_err(anyhow::Error::msg)?;
    let native = LotState {
        placement: lot_placement::State::new(links, &architecture).map_err(anyhow::Error::msg)?,
        membership: RoomMembership::constructed(&world),
        indicators: Default::default(),
        observers: vec![],
        diagnostics: sims_sim::lot_acceptance::person::Diagnostics {
            error: 0,
            blocker: None,
        },
        route: None,
        shoo: None,
        routing_policy: Default::default(),
        movement: None,
        snap: None,
        find_location: None,
        removal: None,
        snap_offsets: Default::default(),
        completion: None,
        route_paths: Default::default(),
        selected_person: None,
        room_effects: vec![],
    };
    let mut vm = Vm::new(world, 17);
    vm.native_accessory_host = true;
    vm.lot_rng_state = Some(1825128472);
    vm.crt_rng_state = Some(23);
    for identifier in 1..=4 {
        vm.start_object_lifecycle(program, identifier)
            .map_err(anyhow::Error::msg)?;
    }
    let mut simulation =
        Simulation::from_loaded_lot(vm, architecture, native).map_err(anyhow::Error::msg)?;
    simulation
        .enable_household(
            program,
            sims_sim::clock::LotClock {
                hour: 12,
                day: 1,
                period: 0,
                minute: 0,
                second: 0,
                month: 1,
                year: 1997,
                elapsed_days: 0,
                month_days: 30,
                ticks: 0,
                statistics: [[0; 8]; 6],
            },
        )
        .map_err(anyhow::Error::msg)?;
    simulation
        .enable_original_environment(
            program,
            sims_sim::lot_environment::manager::Settings {
                daylight_multiplier: 1.,
                light_range: 12.,
                rooms_enabled: true,
                models: std::array::from_fn(|index| Some(index as u8)),
            },
        )
        .map_err(anyhow::Error::msg)?;
    simulation
        .enable_registry_scheduler(0)
        .map_err(anyhow::Error::msg)?;
    Ok(simulation)
}

pub(super) fn session(index: &Index) -> Result<Session> {
    session_with_mode(index, 2)
}

pub(super) fn live_session(index: &Index) -> Result<Session> {
    session_with_mode(index, 0)
}

fn session_with_mode(index: &Index, mode: i16) -> Result<Session> {
    let mut program = sims_content::vm::load_program(index, GLOBAL, GLOBAL)?;
    sims_content::vm::load_headline_catalog(index, &mut program)?;
    let simulation = simulation(&program, mode)?;
    let sources: BTreeMap<_, _> = [HOUSE, NEIGHBORHOOD]
        .into_iter()
        .map(|source| {
            let resource = &index.resources[index.find(source)?];
            Ok((
                source.into(),
                engine::digest(
                    resource
                        .owned
                        .as_ref()
                        .context("authored resource unowned")?,
                ),
            ))
        })
        .collect::<Result<_>>()?;
    let assets = live::Snapshot {
        floors: BTreeMap::new(),
        neighborhood_source: NEIGHBORHOOD.into(),
        initializer_settings: Settings {
            fade_enabled: false,
            fade_duration: 0,
            sampling_config: 0,
        },
        fresh: BTreeMap::new(),
        reloads: BTreeMap::new(),
        dependent_jobs: BTreeMap::new(),
        accessories: BTreeMap::new(),
        retirements: BTreeMap::new(),
        notification_jobs: BTreeMap::new(),
        notifications: None,
        characters: BTreeMap::new(),
        mesh_pool: Default::default(),
        clothing: Default::default(),
        skeletons: None,
        ordinary: Default::default(),
        autonomy_resources: None,
    };
    let saved = Saved {
        version: 2,
        source: HOUSE.into(),
        source_hashes: sources.clone(),
        options: Options::default(),
        modules: program.modules.keys().cloned().collect(),
        normalized_object_definitions: program.modules.keys().cloned().collect(),
        animations: Default::default(),
        headline_catalog: true,
        simulation: Snapshot::capture(&program, &simulation),
        assets,
        view: View::default(),
        managed: Some(Box::new(managed::Content {
            schema: 1,
            house: house_metadata(&sources),
            owned: Neighborhood::capture(index, NEIGHBORHOOD)?,
            catalog: managed::Catalog {
                globals: program.globals.clone(),
                text_language: program.text_language,
                neighbor_resource_flags: program.neighbor_resource_flags.clone(),
                resource_storage: program.resource_storage.clone(),
                accessories: program.accessories.clone(),
                strings: program
                    .modules
                    .iter()
                    .map(|(name, module)| (name.clone(), module.strings.keys().copied().collect()))
                    .collect(),
                careers: false,
            },
        })),
    };
    let session = Session::restore(index, saved).context("restore authored managed Session")?;
    ensure!(
        session.house.number == 7 && session.house.width == 6 && session.house.floors.len() == 2
    );
    ensure!(session.program.fingerprint() == program.fingerprint());
    ensure!(session.simulation == simulation);
    Ok(session)
}

pub(super) fn application() -> Result<Application> {
    let index = index()?;
    let mut session = session(&index)?;
    let client = DialogClient::new(&mut session).context("create authored DialogClient")?;
    let icons = Arc::new(dialog_view::Icons::load(&index).context("decode authored Icons")?);
    let args = Args::parse_from(["sims-room", "--paused", "--original", HOUSE]);
    let (mut viewer, _) = Viewer::new(args);
    viewer.session = Some(session);
    viewer.client = Some(client);
    viewer.icons = Some(icons);
    viewer.diagnostics = true;
    let mut application = Application {
        viewer,
        epoch: Instant::now(),
        millis: 13,
        paths: Default::default(),
        progress_burst_ticks: 0,
        camera_pointer: Default::default(),
    };
    let mut local = application.local();
    local.options_open = true;
    local.display_size = [617, 431];
    local.tutorial_highlight = Some(2037);
    local.resident = 4;
    local.selected = Some(1);
    local.floor = 2;
    local.zoom = 0.75;
    local.playing = true;
    local.speed = 1.75;
    local.last_ms = Some(13);
    local.accumulator = 0.013;
    local.shift = true;
    local.status = "before actual service entry".into();
    local.menu_error = None;
    local.application_revision = 29;
    local.camera_pointer.left = Some([41., 73.]);
    application.apply_local(local);
    Ok(application)
}

pub(super) fn device(session: &Session) -> Result<CameraDevice> {
    let frame = json!({
        "input": {"viewport": [0, 0, 1, 1], "center": [0, 0], "zoom": 2, "rotation": 0},
        "floor": 1, "tick": session.simulation.vm.tick,
    });
    CameraDevice::restore(
        session,
        serde_json::from_value(json!({
            "source": session.house.source, "version": 1,
            "frame": frame, "published_frame": frame, "dirty": false,
            "generation": 0, "published_generation": 0, "controls_generation": 0,
            "controls": {"flags": 0, "zoom": 2, "rotation": 0, "floor": 1},
            "interface_flags": 0, "selected_person": null, "drawing_mode": 0,
            "frozen_pixels": [23, 47, 71, 255],
        }))?,
    )
}
