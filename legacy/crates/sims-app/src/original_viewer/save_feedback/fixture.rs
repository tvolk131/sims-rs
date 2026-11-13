//! Source-authored managed-session and renderer resources for Save handler tests.
//! Adapted from exploration/service_capture/authored_fixture.rs and
//! loaded_input_tests.rs, without private assets or exploration Application owners.
use super::*;

#[allow(dead_code)]
mod base {
    use super::*;
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
        for binding in sims_content::avatar_installation::complete_collection_asset_bindings(&root)
        {
            let directory = binding.physical.strip_prefix(&root)?.to_string_lossy();
            let locator = format!("{directory}/authored.txt");
            resources.push(resource(&root, &locator, "authored.txt", vec![]));
        }
        let identifiers = [
            150, 151, 152, 153, 154, 155, 156, 157, 2003, 2008, 2009, 2010, 2011, 2012, 2013, 2014,
            2023, 2024, 2037, 4500, 4501, 4502, 4503, 4504, 4505, 4506, 4507, 4510, 4705, 4706,
            4707, 4803, 4804, 4805, 4806, 4910, 4911, 4916, 4917, 4918, 4950, 4951, 4952, 4957,
            4958, 4959, 2026,
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
        let mut architecture =
            Architecture::from_original(lot, program).map_err(anyhow::Error::msg)?;
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
            placement: lot_placement::State::new(links, &architecture)
                .map_err(anyhow::Error::msg)?,
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
                        .map(|(name, module)| {
                            (name.clone(), module.strings.keys().copied().collect())
                        })
                        .collect(),
                    careers: false,
                },
            })),
        };
        let session = Session::restore(index, saved).context("restore authored managed Session")?;
        ensure!(
            session.house.number == 7
                && session.house.width == 6
                && session.house.floors.len() == 2
        );
        ensure!(session.program.fingerprint() == program.fingerprint());
        ensure!(session.simulation == simulation);
        Ok(session)
    }
}

fn player_resource(index: &mut Index, group: &str, name: &str, bytes: Vec<u8>) {
    let locator = format!("GameData_ALL_Recursive/{group}/{group}.far::{name}");
    assert!(
        !index
            .resources
            .iter()
            .any(|resource| resource.locator.eq_ignore_ascii_case(&locator))
    );
    index.resources.push(sims_content::Resource {
        source: index
            .root
            .join(format!("GameData_ALL_Recursive/{group}/{name}")),
        locator,
        name: name.into(),
        offset: 0,
        size: bytes.len() as u64,
        unpacked_size: bytes.len() as u64,
        archived: false,
        owned: Some(Arc::from(bytes)),
    });
}
fn player_bcf_string(bytes: &mut Vec<u8>, value: &str) {
    assert!(value.is_ascii() && value.len() < 255);
    bytes.push(value.len() as u8);
    bytes.extend(value.as_bytes());
}
fn player_skeleton() -> Vec<u8> {
    let mut bytes = Vec::from(1_u32.to_le_bytes()); // one skeleton
    player_bcf_string(&mut bytes, "adult");
    bytes.extend(1_u32.to_le_bytes());
    player_bcf_string(&mut bytes, "ROOT");
    player_bcf_string(&mut bytes, "NULL");
    bytes.extend(0_u32.to_le_bytes()); // no bone properties
    for value in [0_f32, 0., 0., 0., 0., 0., 1.] {
        bytes.extend(value.to_le_bytes());
    }
    for flag in [1_u32, 1, 0] {
        bytes.extend(flag.to_le_bytes());
    }
    bytes.extend(0_i32.to_le_bytes());
    bytes.extend(0_f32.to_le_bytes()); // no wiggle
    bytes.extend(0_u32.to_le_bytes());
    bytes.extend(0_u32.to_le_bytes()); // no appearances/animations
    bytes
}
fn player_appearance(name: &str) -> Vec<u8> {
    let mut bytes = Vec::from(0_u32.to_le_bytes());
    bytes.extend(1_u32.to_le_bytes());
    player_bcf_string(&mut bytes, name);
    for value in [0_u32, 0, 1] {
        bytes.extend(value.to_le_bytes());
    } // kind, reserved, binding count
    player_bcf_string(&mut bytes, "ROOT");
    player_bcf_string(&mut bytes, "authored-player-triangle");
    for value in [0_u32, 0, 0] {
        bytes.extend(value.to_le_bytes());
    } // censor, reserved, animations
    bytes
}
fn player_mesh() -> Vec<u8> {
    let mut bytes = vec![];
    player_bcf_string(&mut bytes, "authored-player-triangle");
    player_bcf_string(&mut bytes, "authored-player-texture");
    bytes.extend(1_u32.to_le_bytes());
    player_bcf_string(&mut bytes, "ROOT");
    for value in [1_u32, 0, 1, 2, 1, 0, 0, 3, 0, 0, 3] {
        bytes.extend(value.to_le_bytes());
    }
    // One triangle, one ROOT binding covering all three vertices, no blends.
    for value in [0_f32, 0., 1., 0., 0., 1.] {
        bytes.extend(value.to_le_bytes());
    }
    bytes.extend(0_u32.to_le_bytes());
    bytes.extend(3_u32.to_le_bytes());
    for position in [[0_f32, 0., 0.], [0.25, 0., 0.], [0., 0.25, 0.]] {
        for value in position.into_iter().chain([0_f32, 0., 1.]) {
            bytes.extend(value.to_le_bytes());
        }
    }
    bytes
}
fn player_bitmap() -> Result<Vec<u8>> {
    let pixels = image::RgbImage::from_pixel(2, 2, image::Rgb([23, 47, 71]));
    let mut bytes = std::io::Cursor::new(Vec::new());
    pixels.write_to(&mut bytes, image::ImageFormat::Bmp)?;
    Ok(bytes.into_inner())
}
fn player_iff(chunks: Vec<([u8; 4], u16, Vec<u8>)>) -> Vec<u8> {
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
fn player_pool_sprite() -> Vec<u8> {
    let mut bytes = vec![];
    for value in [1000_u32, 1, 1, 16] {
        bytes.extend(value.to_le_bytes());
    }
    bytes.extend(1_u16.to_le_bytes());
    bytes.extend(1_u16.to_le_bytes()); // one opaque pixel
    bytes.extend(7_u32.to_le_bytes());
    bytes.extend([0; 8]); // flags, palette/reserved, zero offsets
    bytes.extend(6_u16.to_le_bytes());
    bytes.extend(0x2001_u16.to_le_bytes());
    bytes.extend([8, 0]);
    bytes.extend(0xa000_u16.to_le_bytes()); // depth, palette index, end
    bytes
}
fn player_pool_floors() -> Result<Vec<u8>> {
    let mut palette = Vec::from(0_u32.to_le_bytes());
    palette.extend(1_u32.to_le_bytes());
    palette.extend([0; 8]);
    palette.extend([23, 47, 71]);
    let mut chunks = vec![(*b"PALT", 1, palette)];
    // The base managed simulation genuinely has pool=0 in all 36 cells. Keep
    // it unchanged and supply every surface/rim/corner lookup the real binder
    // requires at all three zooms, including shapes absent in this exact view.
    for zoom in 0..3_u16 {
        for shape in 0..21_u16 {
            let id = match shape {
                0..=15 => 1024 + zoom * 16 + shape,
                16 => 12 + zoom * 256,
                _ => 1072 + zoom * 4 + shape - 17,
            };
            chunks.push((*b"SPR2", id, player_pool_sprite()));
        }
    }
    let bytes = player_iff(chunks);
    let iff = sims_formats::iff::Iff::parse(bytes.clone())?;
    for chunk in iff.chunks.iter().filter(|chunk| chunk.tag == *b"SPR2") {
        let raw = iff.data(chunk);
        let set = sims_formats::sprite::SpriteSet::parse(raw)?;
        let frame = set.decode(raw, 0, &iff)?;
        ensure!(
            set.frames.len() == 1
                && frame.width == 1
                && frame.height == 1
                && frame.x == 0
                && frame.y == 0
                && frame.rgba == [23, 47, 71, 255],
            "authored pool sprite did not decode through its real palette"
        );
    }
    Ok(bytes)
}
fn player_index() -> Result<Index> {
    let mut index = base::index()?;
    let skeleton = player_skeleton();
    let decoded = sims_formats::avatar::read_bcf(&skeleton)?;
    ensure!(
        decoded.skeletons.len() == 1
            && decoded.skeletons[0].bones.len() == 1
            && decoded.skeletons[0].bones[0].name == "ROOT",
        "authored skeleton codec differs"
    );
    player_resource(&mut index, "Animation", "adult-skeleton.cmx.bcf", skeleton);
    let mesh = player_mesh();
    let decoded = sims_formats::avatar::read_bmf(&mesh)?;
    ensure!(
        decoded.vertices.len() == 3
            && decoded.triangles.len() == 1
            && decoded.bindings.len() == 1
            && decoded.vertices.iter().all(|vertex| vertex.bone == 0),
        "authored mesh must really bind its complete triangle"
    );
    player_resource(
        &mut index,
        "Animation",
        "authored-player-triangle.bmf",
        mesh,
    );
    for name in [
        "b002mafit_01",
        "nmfit_01",
        "pajamamfit_01",
        "c001ma_ross",
        "hmlo",
        "hmro",
    ] {
        let bytes = player_appearance(name);
        let decoded = sims_formats::avatar::read_bcf(&bytes)?;
        ensure!(
            decoded.appearances.len() == 1
                && decoded.appearances[0].name == name
                && decoded.appearances[0].bindings.len() == 1,
            "authored appearance codec differs"
        );
        player_resource(&mut index, "Animation", &format!("{name}.cmx.bcf"), bytes);
    }
    for name in [
        "B002MAFitlgt_Ross.bmp",
        "B002MAFitlgt_casual.bmp",
        "NMFitlgt_01.bmp",
        "PajamaMlgt_01.bmp",
        "C001MAlgt_Ross.bmp",
        "HUAOlgt.bmp",
    ] {
        player_resource(&mut index, "Textures", name, player_bitmap()?);
    }
    player_resource(&mut index, "Floors", "floors.iff", player_pool_floors()?);
    // No wall segments exist in this fixture; a valid empty shape catalog is
    // still a required owned resource, rather than a bypass of wall binding.
    player_resource(&mut index, "Walls", "walls.iff", player_iff(vec![]));
    ensure!(
        index
            .resources
            .iter()
            .all(|resource| resource.owned.is_some())
    );
    Ok(index)
}
fn player_house_arrays(session: &mut Session) -> Result<()> {
    let lot = session
        .simulation
        .architecture
        .as_ref()
        .and_then(|architecture| architecture.original.as_ref())
        .context("authored player lot absent")?;
    ensure!(
        lot.size == session.house.width
            && lot.size == session.house.height
            && lot.pool.len() == lot.size * lot.size
            && lot.pool.iter().all(|value| *value == 0)
            && lot.floors.iter().all(|floor| floor
                .source
                .walls
                .iter()
                .all(|wall| wall.segments == 0))
            && session
                .house
                .floors
                .iter()
                .all(|floor| floor.walls.iter().all(|wall| wall.segments == 0)),
        "authored player surface resources no longer describe this fixture"
    );
    ensure!(
        !session.house.arrays.contains_key(&6) && !session.house.arrays.contains_key(&9),
        "authored player surface arrays must not overwrite existing imported data"
    );
    for (id, data) in [(6, vec![0; lot.size * lot.size]), (9, lot.pool.clone())] {
        session.house.arrays.insert(
            id,
            sims_formats::lot::Array {
                width: lot.size,
                height: lot.size,
                stride: 1,
                encoding: 0,
                data,
            },
        );
    }
    Ok(())
}

pub(super) fn viewer(root: &Path, destination: &Path) -> Result<Viewer> {
    let index = player_index()?;
    let mut session = base::session(&index)?;
    player_house_arrays(&mut session)?;
    let client = DialogClient::new(&mut session)?;
    session.house.household = Some(serde_json::from_value(json!({
        "id":1,"name":"authored Save household",
        "family":{"version":7,"house":7,"number":1,"budget":0,"architecture_value":0,
            "friends":0,"flags":0,"members":[1],"tail":[]},
        "residents":[{"guid":1,"neighbor":1,"object":4,"name":"authored",
            "character_source":base::GLOBAL,"body_strings":[]}]
    }))?);
    let mut args = Args::parse_from(["sims-room", "--original", base::HOUSE]);
    args.root = root.to_path_buf();
    args.save = destination.to_path_buf();
    let (mut viewer, task) = Viewer::new(args);
    drop(task);
    viewer.session = Some(session);
    viewer.client = Some(client);
    viewer.icons = Some(Arc::new(dialog_view::Icons::load(&index)?));
    viewer.resident = 4;
    viewer.floor = 1;
    viewer.diagnostics = true;
    viewer.last = Some(Instant::now());
    viewer.accumulator = 0.013;
    viewer.status = "Ready to save".into();
    Ok(viewer)
}
