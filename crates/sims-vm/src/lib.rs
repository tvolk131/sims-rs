//! The SimAntics interpreter.
//!
//! [`Program::link`] gathers the code a lot's objects run: the global
//! behaviors, each object's own file and its semi-global file. Tree IDs
//! resolve the way the original's do, and [`Program::signature`] reproduces
//! the per-type signatures saves store, which checks the linking against the
//! original's own data.
//!
//! [`Interpreter`] runs that code against a [`sims_sim::World`]. An
//! object's thread is its saved stack of frames: [`Interpreter::step`]
//! executes the top frame's instruction, [`Interpreter::turn`] runs the
//! thread until it yields or returns, and [`Interpreter::run_tree`] runs a
//! tree on an object to completion. Primitives without an implementation,
//! and memory scopes over state the simulation doesn't hold yet, fail with
//! a [`Fault`] that names them.
//!
//! [`Interpreter::create`] is the original's object factory, and
//! [`Interpreter::remove_now`] and [`Interpreter::delete`] take an object
//! off the lot and destroy it. [`Interpreter::resize`],
//! [`Interpreter::clear`], [`Interpreter::read_types`],
//! [`Interpreter::reconcile_controllers`], [`Interpreter::assign_rooms`],
//! [`Interpreter::visit_objects`] and [`Interpreter::create_indicators`]
//! are the lot load's steps that run scripts or need the linked code.
//! Visiting an object can rebuild it, which takes it off its tile and
//! puts it back (`placement.rs`). Visiting a person gives it the fields
//! its neighborhood record keeps and resets it in its own way
//! (`people.rs`). [`Interpreter::update_rooms`] counts the rooms with the
//! objects that stand in them (`rooms.rs`). The load ends with
//! [`Interpreter::announce_house`], [`Interpreter::select_resident`],
//! [`Interpreter::go_live`] and [`Interpreter::before_frame`]
//! (`selection.rs`), which leave the lot as the first tick finds it.
//!
//! [`boot::Lot`] runs all of that for a house file, then one tick at a
//! time: what `sims boot` prints and what presentation crates watch.

mod animator;
mod autonomy;
pub mod boot;
mod both_ways;
mod build;
mod buy;
mod cancel;
mod clip_events;
mod daily;
mod error;
mod factory;
mod fault;
mod forget;
mod frame;
mod group_factory;
mod head;
mod interpreter;
pub mod lifecycle;
mod load;
mod memory;
pub mod neighborhood_open;
mod object_uses;
mod objects;
pub mod orders;
mod outfits;
mod people;
mod person_factory;
mod pick;
mod placement;
pub mod play;
mod primitives;
mod program;
mod removal;
mod restore;
mod rooms;
mod routing;
mod selection;
pub mod sounds;
mod tick;
mod visits;

pub use animator::snap_to_path;
pub use build::{
    ARCHITECTURE_BUDGET, BuildHistory, BuildPrices, DEFAULT_GRASS_PRICE, DEFAULT_POOL_PRICE,
    DEFAULT_ROOF_PATTERN, DEFAULT_TERRAIN_PRICE, DEFAULT_WATER_PRICE, FLAG_LOCKED, FloorDragMode,
    FloorPlan, GRASS_ARRAY, GRASS_RESTORED_VALUE, MAX_TERRAIN_SLOPE, PoolMode, PoolPlan,
    ROOF_META_ARRAY, RoofPitch, RoofPlan, RoofState, SPLIT_FLOOR_ID, TARGET_GRASS_ARRAY,
    TERRAIN_DIRT_GRASS, TerrainMode, TerrainPlan, ToolPrices, WALL_ASCENDING, WALL_CURSOR_GUID,
    WALL_DESCENDING, WALL_EAST, WALL_NORTH, WALL_SOUTH, WALL_WEST, WallCursor, WallDragMode,
    WallFace, WallPlan, WallSegment, WallStyle, WallpaperDragMode, WallpaperPlan, WaterMode,
    WaterPlan, build_refund, is_buildable_tile, is_grass_tile_editable, is_pool_tile,
    is_terrain_vertex_editable, is_water_tile, plan_floor_edit, plan_pool_edit, plan_roof_edit,
    plan_roof_pattern, plan_roof_pitch, plan_terrain_edit, plan_wall_drag, plan_wallpaper_edit,
    plan_water_edit, preview_floor_on_world, preview_pool_on_world, preview_roof_on_world,
    preview_terrain_on_world, preview_wall_on_world, preview_wallpaper_on_world,
    preview_water_on_world, roof_state, room_wallpaper_faces, snap_wall_corner,
    terrain_vertex_height, wall_cursor_fine, wall_line_segments, wall_room_box_segments,
    wallpaper_cursor_facing, wallpaper_cursor_fine,
};
pub use buy::{
    BuyActionKind, BuyCursor, BuyPlacement, BuyPurchase, COMEANDSEE_GUID, TILE_CURSOR_GUID,
    available_funds, default_buy_direction, panel_mode, snap_to_tile_centre,
};
pub use error::Error;
pub use factory::TURN_BUDGET;
pub use fault::{Fault, Location, Trap};
pub use frame::PanelMode;
pub use interpreter::{Asked, Interpreter, MAX_STACK, Status, Stream};
pub use load::{INDICATORS, Reconciled, Roomed};
pub use memory::scope_name;
pub use objects::{READ_BEFORE_THE_FIRST_LOAD, SavedRead, read_before_the_first_load};
pub use people::{curve_table, persistent_fields, reload_profile};
pub use primitives::primitive_name;
pub use primitives::ui_effect::UiEffectEvent;
pub use program::{
    Callback, Career, DEFINITION_WORDS, ICON_FILE, LABEL_FILE, Library, Program, Tree, icon_frames,
    labels_hold_no_token, loaded_order, sprite_frames, unported_strings,
};
pub use selection::selected;
pub use sounds::SoundEvent;
pub use tick::{Answer, Answers, Player, Shown, SkippedFrame, TICK_BUDGET, TickError, Update};
pub use visits::Visited;

pub type Result<T> = std::result::Result<T, Error>;
