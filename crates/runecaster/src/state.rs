use bevy::prelude::*;
use rune_core::{EntityId, Rune, World};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Normal,
    Aim,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Screen {
    Title,
    Play,
}

#[derive(Resource)]
pub(crate) struct Sim {
    pub(crate) world: World,
    seed: u64,
    pub(crate) mode: Mode,
    pub(crate) screen: Screen,
    pub(crate) composing: Vec<Rune>,
    pub(crate) dirty: bool,
    pub(crate) level_gen: u64,
    /// Player HP at the last sync, for damage-driven screen shake.
    pub(crate) last_hp: i32,
    /// Whether this run's final score has been recorded to the high-score file.
    pub(crate) recorded: bool,
}

impl Sim {
    pub(crate) fn new(seed: u64) -> Self {
        let world = World::new(seed);
        let last_hp = world.player().hp;
        Self {
            world,
            seed,
            mode: Mode::Normal,
            screen: Screen::Title,
            composing: Vec::new(),
            dirty: true,
            level_gen: 0,
            last_hp,
            recorded: false,
        }
    }

    pub(crate) fn restart(&mut self) {
        self.seed = self.seed.wrapping_add(1);
        self.world = World::new(self.seed);
        self.mode = Mode::Normal;
        self.screen = Screen::Play;
        self.composing.clear();
        self.dirty = true;
        self.level_gen += 1;
        self.last_hp = self.world.player().hp;
        self.recorded = false;
    }
}

#[derive(Resource, Default)]
pub(crate) struct ActorViews {
    pub(crate) map: HashMap<EntityId, Entity>,
    pub(crate) last_gen: u64,
}

#[derive(Resource)]
pub(crate) struct HighScore(pub(crate) u32);

#[derive(Resource, Default)]
pub(crate) struct Shake {
    pub(crate) trauma: f32,
}

#[derive(Component)]
pub(crate) struct StaticTag;

#[derive(Component)]
pub(crate) struct MainCamera;

/// Marks a persistent actor sprite and records which sim entity it mirrors.
#[derive(Component)]
#[allow(dead_code)]
pub(crate) struct ActorId(pub(crate) EntityId);

#[derive(Component)]
pub(crate) struct AnimTarget(pub(crate) Vec3);

// ---------------------------------------------------------------------------
// High-score persistence
// ---------------------------------------------------------------------------

fn highscore_path() -> std::path::PathBuf {
    match std::env::var("HOME") {
        Ok(home) => std::path::Path::new(&home).join(".runecaster_highscore"),
        Err(_) => std::path::PathBuf::from("runecaster_highscore.txt"),
    }
}

pub(crate) fn load_highscore() -> u32 {
    std::fs::read_to_string(highscore_path())
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

pub(crate) fn save_highscore(v: u32) {
    let _ = std::fs::write(highscore_path(), v.to_string());
}
