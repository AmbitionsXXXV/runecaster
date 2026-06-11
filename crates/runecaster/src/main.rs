//! Runecaster — a rune-grammar roguelike.
//!
//! Bevy 0.18 presentation layer. All game rules live in the engine-agnostic
//! `rune_core` crate; here we translate keyboard input into `World` commands,
//! draw the resulting state, and turn the world's [`rune_core::Fx`] event
//! buffer into particles.
//!
//! ## Rendering model
//! - **Tiles, floor items, and UI** are rebuilt each game step (tagged
//!   [`StaticTag`], despawned + respawned when `Sim::dirty`).
//! - **Actors** are *persistent* sprites tracked in [`ActorViews`]; each step
//!   sets their [`AnimTarget`] and [`animate`] lerps them so they glide.
//! - **Particles** spawn from the drained FX buffer and self-despawn.
//! - **Camera shake** ramps up when the player takes damage.
//! - Sprite textures are generated procedurally at startup (no external
//!   assets). To use a real tileset, load `Handle<Image>`s in `setup` instead.
//! - **Sound** is not wired (needs audio asset files); drop `.ogg` files in
//!   `assets/` and play them with `AudioPlayer` to add it.
//!
//! ## Controls
//! - Move / attack: arrows, WASD, or hjkl; diagonals y/u/b/n
//! - Wait: Space · Descend `>`: Shift+Period on the stairs
//! - Compose: number keys append runes · Backspace drop · Esc clear
//! - Cast: Enter (or C); Bolt/Beam/Touch then ask you to aim
//! - Title / after death/victory: Enter (or R) to play

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::sprite::Anchor;
use rand::RngExt;
use rune_core::world::MAX_DEPTH;
use rune_core::{
    CastFx, Dir, ElementKind, EntityId, Form, Fx, FxKind, Point, RunState, Rune, RuneCategory,
    Tile, World, MAX_RUNES,
};
use std::f32::consts::TAU;
use std::collections::{HashMap, HashSet};

const TILE: f32 = 18.0;
const WINDOW_W: f32 = 1280.0;
const WINDOW_H: f32 = 800.0;
const BOARD_LEFT: f32 = -630.0;
const BOARD_TOP: f32 = 360.0;

const Z_TILE: f32 = 0.0;
const Z_ITEM: f32 = 1.0;
const Z_ACTOR: f32 = 5.0;
const Z_UI: f32 = 10.0;
const Z_FX: f32 = 20.0;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Runecaster — Rune-Grammar Roguelike".into(),
                resolution: (WINDOW_W as u32, WINDOW_H as u32).into(),
                resizable: false,
                ..default()
            }),
            ..default()
        }))
        .insert_resource(ClearColor(Color::srgb(0.03, 0.03, 0.05)))
        .insert_resource(Sim::new(1))
        .insert_resource(HighScore(load_highscore()))
        .insert_resource(Shake::default())
        .init_resource::<ActorViews>()
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (handle_input, sync, animate, particles, projectiles, camera_shake).chain(),
        )
        .run();
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Normal,
    Aim,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Title,
    Play,
}

#[derive(Resource)]
struct Sim {
    world: World,
    seed: u64,
    mode: Mode,
    screen: Screen,
    composing: Vec<Rune>,
    dirty: bool,
    level_gen: u64,
    /// Player HP at the last sync, for damage-driven screen shake.
    last_hp: i32,
    /// Whether this run's final score has been recorded to the high-score file.
    recorded: bool,
}

impl Sim {
    fn new(seed: u64) -> Self {
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

    fn restart(&mut self) {
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

#[derive(Resource)]
struct Tiles {
    floor: Handle<Image>,
    wall: Handle<Image>,
    stairs: Handle<Image>,
    player: Handle<Image>,
    creature: Handle<Image>,
    gem: Handle<Image>,
    relic: Handle<Image>,
    spark: Handle<Image>,
    ring: Handle<Image>,
    shard: Handle<Image>,
    streak: Handle<Image>,
}

#[derive(Resource, Default)]
struct ActorViews {
    map: HashMap<EntityId, Entity>,
    last_gen: u64,
}

#[derive(Resource)]
struct HighScore(u32);

#[derive(Resource, Default)]
struct Shake {
    trauma: f32,
}

#[derive(Component)]
struct StaticTag;

#[derive(Component)]
struct MainCamera;

/// Marks a persistent actor sprite and records which sim entity it mirrors.
#[derive(Component)]
#[allow(dead_code)]
struct ActorId(EntityId);

#[derive(Component)]
struct AnimTarget(Vec3);

#[derive(Component)]
struct Particle {
    age: f32,
    life: f32,
    from_scale: f32,
    to_scale: f32,
    base: Color,
    vel: Vec3,
    accel: Vec3,
    spin: f32,
}

/// A cosmetic spell projectile flying from caster to target; on arrival it
/// despawns and spawns the element's impact burst.
#[derive(Component)]
struct Projectile {
    from: Vec3,
    to: Vec3,
    age: f32,
    life: f32,
    element: ElementKind,
    trail: [u8; 3],
}

fn setup(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    commands.spawn((Camera2d::default(), MainCamera));
    let tiles = Tiles {
        floor: make_image(&mut images, px_floor),
        wall: make_image(&mut images, px_wall),
        stairs: make_image(&mut images, px_stairs),
        player: make_image(&mut images, px_player),
        creature: make_image(&mut images, px_creature),
        gem: make_image(&mut images, px_gem),
        relic: make_image(&mut images, px_relic),
        spark: make_image(&mut images, px_spark),
        ring: make_image(&mut images, px_ring),
        shard: make_image(&mut images, px_shard),
        streak: make_image(&mut images, px_streak),
    };
    commands.insert_resource(tiles);
}

// ---------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------

fn handle_input(keys: Res<ButtonInput<KeyCode>>, mut sim: ResMut<Sim>) {
    if sim.screen == Screen::Title {
        if keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::Space) {
            sim.screen = Screen::Play;
            sim.dirty = true;
        }
        return;
    }
    if sim.world.state != RunState::Playing {
        if keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::KeyR) {
            sim.restart();
        }
        return;
    }
    match sim.mode {
        Mode::Normal => handle_normal(&keys, &mut sim),
        Mode::Aim => handle_aim(&keys, &mut sim),
    }
}

const DIGIT_KEYS: [KeyCode; 10] = [
    KeyCode::Digit1,
    KeyCode::Digit2,
    KeyCode::Digit3,
    KeyCode::Digit4,
    KeyCode::Digit5,
    KeyCode::Digit6,
    KeyCode::Digit7,
    KeyCode::Digit8,
    KeyCode::Digit9,
    KeyCode::Digit0,
];

fn handle_normal(keys: &ButtonInput<KeyCode>, sim: &mut Sim) {
    for (i, key) in DIGIT_KEYS.iter().enumerate() {
        if keys.just_pressed(*key) {
            if let Some(&rune) = sim.world.known_runes.get(i) {
                if sim.composing.len() < MAX_RUNES {
                    sim.composing.push(rune);
                    sim.dirty = true;
                }
            }
        }
    }
    if keys.just_pressed(KeyCode::Backspace) {
        sim.composing.pop();
        sim.dirty = true;
    }
    if keys.just_pressed(KeyCode::Escape) && !sim.composing.is_empty() {
        sim.composing.clear();
        sim.dirty = true;
    }

    if keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::KeyC) {
        if !sim.composing.is_empty() {
            match sim.world.preview(&sim.composing) {
                Ok(spell) if spell.needs_aim() => {
                    sim.mode = Mode::Aim;
                    sim.dirty = true;
                }
                Ok(_) => {
                    let words = sim.composing.clone();
                    let _ = sim.world.player_cast(&words, Dir::North);
                    sim.composing.clear();
                    sim.dirty = true;
                }
                Err(e) => {
                    let msg = format!("Cannot cast: {}", e.message());
                    sim.world.log.push(msg);
                    sim.dirty = true;
                }
            }
        }
        return;
    }

    if keys.just_pressed(KeyCode::Period)
        && (keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight))
    {
        let before = sim.world.depth;
        sim.world.descend();
        if sim.world.depth != before {
            sim.level_gen += 1;
        }
        sim.dirty = true;
        return;
    }

    if keys.just_pressed(KeyCode::Space) {
        sim.world.player_wait();
        sim.dirty = true;
        return;
    }

    if let Some(dir) = movement_dir(keys) {
        sim.world.player_move(dir);
        sim.dirty = true;
    }
}

fn handle_aim(keys: &ButtonInput<KeyCode>, sim: &mut Sim) {
    if keys.just_pressed(KeyCode::Escape) {
        sim.mode = Mode::Normal;
        sim.dirty = true;
        return;
    }
    if let Some(dir) = movement_dir(keys) {
        let words = sim.composing.clone();
        let _ = sim.world.player_cast(&words, dir);
        sim.composing.clear();
        sim.mode = Mode::Normal;
        sim.dirty = true;
    }
}

fn movement_dir(keys: &ButtonInput<KeyCode>) -> Option<Dir> {
    use KeyCode::*;
    let p = |k| keys.just_pressed(k);
    if p(KeyY) {
        Some(Dir::NorthWest)
    } else if p(KeyU) {
        Some(Dir::NorthEast)
    } else if p(KeyB) {
        Some(Dir::SouthWest)
    } else if p(KeyN) {
        Some(Dir::SouthEast)
    } else if p(ArrowUp) || p(KeyK) || p(KeyW) {
        Some(Dir::North)
    } else if p(ArrowDown) || p(KeyJ) || p(KeyS) {
        Some(Dir::South)
    } else if p(ArrowLeft) || p(KeyH) || p(KeyA) {
        Some(Dir::West)
    } else if p(ArrowRight) || p(KeyL) || p(KeyD) {
        Some(Dir::East)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Coordinate mapping
// ---------------------------------------------------------------------------

fn tile_pos(tx: i32, ty: i32, z: f32) -> Vec3 {
    Vec3::new(
        BOARD_LEFT + tx as f32 * TILE + TILE / 2.0,
        BOARD_TOP - ty as f32 * TILE - TILE / 2.0,
        z,
    )
}

// ---------------------------------------------------------------------------
// Per-step rebuild + actor reconciliation + FX spawning
// ---------------------------------------------------------------------------

fn sync(
    mut commands: Commands,
    mut sim: ResMut<Sim>,
    mut views: ResMut<ActorViews>,
    mut high: ResMut<HighScore>,
    mut shake: ResMut<Shake>,
    tiles: Res<Tiles>,
    statics: Query<Entity, With<StaticTag>>,
) {
    if !sim.dirty {
        return;
    }
    sim.dirty = false;

    for e in statics.iter() {
        commands.entity(e).despawn();
    }

    if sim.screen == Screen::Title {
        for (_, e) in views.map.drain() {
            commands.entity(e).despawn();
        }
        draw_title(&mut commands, high.0);
        return;
    }

    // Damage-driven shake + one-shot score recording (mutate sim before the
    // shared borrow below).
    let hp_now = sim.world.player().hp;
    let hp_lost = sim.last_hp - hp_now;
    sim.last_hp = hp_now;
    if hp_lost > 0 {
        shake.trauma = (shake.trauma + hp_lost as f32 / 12.0).min(1.0);
    }
    if sim.world.state != RunState::Playing && !sim.recorded {
        sim.recorded = true;
        let s = sim.world.score();
        if s > high.0 {
            high.0 = s;
            save_highscore(s);
        }
    }

    // Drain visual events (mutates the world).
    let fx = std::mem::take(&mut sim.world.fx);
    let cast = std::mem::take(&mut sim.world.cast_fx);

    // Hard reset (restart / descend) → drop all actor sprites.
    if views.last_gen != sim.level_gen {
        for (_, e) in views.map.drain() {
            commands.entity(e).despawn();
        }
        views.last_gen = sim.level_gen;
    }

    let w = &sim.world;

    // --- tiles ---
    for ty in 0..w.map.height {
        for tx in 0..w.map.width {
            let p = Point::new(tx, ty);
            if !w.is_explored(p) {
                continue;
            }
            let image = match w.map.tile(p) {
                Tile::Wall => {
                    if w.is_temp_wall(p) {
                        tiles.stairs.clone() // distinct look handled by tint below
                    } else {
                        tiles.wall.clone()
                    }
                }
                Tile::Floor => tiles.floor.clone(),
                Tile::StairsDown => tiles.stairs.clone(),
            };
            let visible = w.is_visible(p);
            let tint = if w.is_temp_wall(p) {
                if visible {
                    Color::srgb(0.45, 0.6, 0.95)
                } else {
                    Color::srgb(0.2, 0.27, 0.42)
                }
            } else if visible {
                Color::WHITE
            } else {
                Color::srgb(0.42, 0.42, 0.52)
            };
            commands.spawn((
                Sprite {
                    image,
                    color: tint,
                    custom_size: Some(Vec2::splat(TILE)),
                    ..default()
                },
                Transform::from_translation(tile_pos(tx, ty, Z_TILE)),
                StaticTag,
            ));
        }
    }

    // --- floor runes ---
    for (p, rune) in &w.rune_items {
        if !w.is_visible(*p) {
            continue;
        }
        let (cr, cg, cb) = rune.color();
        commands.spawn((
            Sprite {
                image: tiles.gem.clone(),
                color: Color::srgb_u8(cr, cg, cb),
                custom_size: Some(Vec2::splat(TILE * 0.72)),
                ..default()
            },
            Transform::from_translation(tile_pos(p.x, p.y, Z_ITEM)),
            StaticTag,
        ));
    }

    // --- floor relics ---
    for (p, _relic) in &w.relic_items {
        if !w.is_visible(*p) {
            continue;
        }
        commands.spawn((
            Sprite {
                image: tiles.relic.clone(),
                custom_size: Some(Vec2::splat(TILE * 0.92)),
                ..default()
            },
            Transform::from_translation(tile_pos(p.x, p.y, Z_ITEM)),
            StaticTag,
        ));
    }

    // --- reconcile actor sprites (persistent + animated) ---
    let mut seen: HashSet<EntityId> = HashSet::new();
    for ent in &w.entities {
        if !ent.alive {
            continue;
        }
        seen.insert(ent.id);
        let target = tile_pos(ent.pos.x, ent.pos.y, Z_ACTOR);
        let vis = if w.is_visible(ent.pos) {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if let Some(&e) = views.map.get(&ent.id) {
            commands.entity(e).insert((AnimTarget(target), vis));
        } else {
            let e = commands
                .spawn((
                    actor_sprite(ent, &tiles),
                    Transform::from_translation(target),
                    AnimTarget(target),
                    ActorId(ent.id),
                    vis,
                ))
                .id();
            views.map.insert(ent.id, e);
        }
    }
    let stale: Vec<EntityId> = views
        .map
        .keys()
        .copied()
        .filter(|id| !seen.contains(id))
        .collect();
    for id in stale {
        if let Some(e) = views.map.remove(&id) {
            commands.entity(e).despawn();
        }
    }

    spawn_ui(&mut commands, &sim, high.0);

    // --- skill effects: signature trajectories, then point bursts ---
    for cf in &cast {
        spawn_cast_fx(&mut commands, &tiles, cf);
    }
    for ev in &fx {
        spawn_fx(&mut commands, &tiles, ev);
    }
}

fn actor_sprite(ent: &rune_core::Entity, tiles: &Tiles) -> Sprite {
    if ent.is_player() {
        return Sprite {
            image: tiles.player.clone(),
            custom_size: Some(Vec2::splat(TILE * 0.98)),
            ..default()
        };
    }
    let (tint, scale) = match ent.glyph {
        'T' => (Color::srgb(0.45, 0.72, 0.42), 1.35), // troll
        'O' => (Color::srgb(0.95, 0.42, 0.36), 1.30), // ogre
        'w' => (Color::srgb(0.76, 0.83, 0.98), 1.00), // wraith
        'g' => (Color::srgb(0.52, 0.85, 0.42), 0.98), // goblin
        'b' => (Color::srgb(0.64, 0.5, 0.74), 0.72),  // bat
        _ => (Color::srgb(0.74, 0.58, 0.45), 0.78),   // rat / other
    };
    Sprite {
        image: tiles.creature.clone(),
        color: tint,
        custom_size: Some(Vec2::splat(TILE * scale)),
        ..default()
    }
}

// ---------------------------------------------------------------------------
// Per-frame animation, particles, shake
// ---------------------------------------------------------------------------

fn animate(time: Res<Time>, mut q: Query<(&mut Transform, &AnimTarget)>) {
    let t = (time.delta_secs() * 16.0).min(1.0);
    for (mut tf, target) in &mut q {
        tf.translation = tf.translation.lerp(target.0, t);
    }
}

fn particles(
    time: Res<Time>,
    mut commands: Commands,
    mut q: Query<(Entity, &mut Particle, &mut Transform, &mut Sprite)>,
) {
    let dt = time.delta_secs();
    for (e, mut p, mut tf, mut sp) in &mut q {
        p.age += dt;
        if p.age >= p.life {
            commands.entity(e).despawn();
            continue;
        }
        let t = p.age / p.life;
        let accel = p.accel;
        let vel = p.vel + accel * dt;
        p.vel = vel;
        tf.translation += vel * dt;
        let s = p.from_scale + (p.to_scale - p.from_scale) * t;
        tf.scale = Vec3::splat(s);
        if p.spin != 0.0 {
            tf.rotation = Quat::from_rotation_z(p.spin * p.age);
        }
        sp.color = p.base.with_alpha((1.0 - t).max(0.0));
    }
}

/// Advance cosmetic spell projectiles; lay a short trail and burst on arrival.
fn projectiles(
    time: Res<Time>,
    mut commands: Commands,
    tiles: Res<Tiles>,
    mut q: Query<(Entity, &mut Projectile, &mut Transform)>,
) {
    let dt = time.delta_secs();
    for (e, mut pr, mut tf) in &mut q {
        pr.age += dt;
        let t = (pr.age / pr.life).min(1.0);
        tf.translation = pr.from.lerp(pr.to, t);
        let trail = Color::srgb_u8(pr.trail[0], pr.trail[1], pr.trail[2]);
        push_particle(
            &mut commands,
            tiles.spark.clone(),
            tf.translation,
            trail,
            0.2,
            0.55,
            0.05,
            Vec3::ZERO,
            Vec3::ZERO,
            0.0,
        );
        if pr.age >= pr.life {
            spawn_element_burst(&mut commands, &tiles, pr.to, pr.element, 1.0);
            commands.entity(e).despawn();
        }
    }
}

fn camera_shake(
    time: Res<Time>,
    mut shake: ResMut<Shake>,
    mut q: Query<&mut Transform, With<MainCamera>>,
) {
    shake.trauma = (shake.trauma - time.delta_secs() * 1.8).max(0.0);
    let amount = shake.trauma * shake.trauma;
    let mut rng = rand::rng();
    let max = 14.0;
    let ox = rng.random_range(-1.0f32..1.0) * amount * max;
    let oy = rng.random_range(-1.0f32..1.0) * amount * max;
    if let Ok(mut tf) = q.single_mut() {
        tf.translation.x = ox;
        tf.translation.y = oy;
    }
}

/// Low-level: spawn a single particle.
#[allow(clippy::too_many_arguments)]
fn push_particle(
    commands: &mut Commands,
    image: Handle<Image>,
    pos: Vec3,
    base: Color,
    life: f32,
    from_scale: f32,
    to_scale: f32,
    vel: Vec3,
    accel: Vec3,
    spin: f32,
) {
    commands.spawn((
        Sprite {
            image,
            color: base,
            custom_size: Some(Vec2::splat(TILE)),
            ..default()
        },
        Transform {
            translation: pos,
            scale: Vec3::splat(from_scale),
            ..default()
        },
        Particle {
            age: 0.0,
            life,
            from_scale,
            to_scale,
            base,
            vel,
            accel,
            spin,
        },
    ));
}

/// Per-element burst recipe: texture, count, palette, motion.
struct Burst {
    image: Handle<Image>,
    count: u32,
    base: [u8; 3],
    jitter: i32,
    life: f32,
    speed: f32,
    accel_y: f32,
    s0: f32,
    s1: f32,
    spin: f32,
}

fn elem_burst(element: ElementKind, tiles: &Tiles) -> Burst {
    use ElementKind::*;
    match element {
        // embers that rise and shrink
        Fire => Burst { image: tiles.spark.clone(), count: 9, base: [255, 140, 50], jitter: 40, life: 0.55, speed: 26.0, accel_y: 40.0, s0: 0.55, s1: 0.12, spin: 0.0 },
        // shards that scatter and fall
        Frost => Burst { image: tiles.shard.clone(), count: 8, base: [150, 210, 255], jitter: 30, life: 0.55, speed: 44.0, accel_y: -70.0, s0: 0.5, s1: 0.42, spin: 6.0 },
        // quick bright streaks
        Lightning => Burst { image: tiles.streak.clone(), count: 7, base: [255, 240, 120], jitter: 20, life: 0.26, speed: 72.0, accel_y: 0.0, s0: 0.7, s1: 0.2, spin: 0.0 },
        // a violet shove
        Force => Burst { image: tiles.spark.clone(), count: 6, base: [220, 170, 255], jitter: 25, life: 0.4, speed: 56.0, accel_y: 0.0, s0: 0.6, s1: 0.2, spin: 0.0 },
        // magenta motes
        Drain => Burst { image: tiles.spark.clone(), count: 7, base: [222, 80, 170], jitter: 25, life: 0.5, speed: 30.0, accel_y: 12.0, s0: 0.5, s1: 0.14, spin: 0.0 },
        // green motes rising
        Heal => Burst { image: tiles.spark.clone(), count: 7, base: [120, 255, 150], jitter: 25, life: 0.7, speed: 18.0, accel_y: 34.0, s0: 0.5, s1: 0.2, spin: 0.0 },
        // imploding cyan ring + motes
        Teleport => Burst { image: tiles.ring.clone(), count: 3, base: [150, 230, 255], jitter: 15, life: 0.5, speed: 8.0, accel_y: 0.0, s0: 1.3, s1: 0.1, spin: 2.5 },
        // settling stone shards
        Barrier => Burst { image: tiles.shard.clone(), count: 6, base: [150, 180, 235], jitter: 20, life: 0.5, speed: 20.0, accel_y: -24.0, s0: 0.5, s1: 0.5, spin: 3.0 },
    }
}

fn spawn_element_burst(
    commands: &mut Commands,
    tiles: &Tiles,
    pos: Vec3,
    element: ElementKind,
    intensity: f32,
) {
    let b = elem_burst(element, tiles);
    let mut rng = rand::rng();
    let n = ((b.count as f32) * intensity).round().max(1.0) as u32;
    for _ in 0..n {
        let ang = rng.random_range(0.0f32..TAU);
        let spd = rng.random_range(b.speed * 0.35..b.speed.max(0.1));
        let vel = Vec3::new(ang.cos() * spd, ang.sin() * spd, 0.0);
        let j = b.jitter;
        let base = Color::srgb_u8(
            (b.base[0] as i32 + rng.random_range(-j..=j)).clamp(0, 255) as u8,
            (b.base[1] as i32 + rng.random_range(-j..=j)).clamp(0, 255) as u8,
            (b.base[2] as i32 + rng.random_range(-j..=j)).clamp(0, 255) as u8,
        );
        let life = b.life * rng.random_range(0.8..1.2);
        let spin = if b.spin > 0.0 {
            rng.random_range(-b.spin..b.spin)
        } else {
            0.0
        };
        push_particle(
            commands,
            b.image.clone(),
            pos,
            base,
            life,
            b.s0,
            b.s1,
            vel,
            Vec3::new(0.0, b.accel_y, 0.0),
            spin,
        );
    }
}

fn physical_burst(commands: &mut Commands, tiles: &Tiles, pos: Vec3, color: [u8; 3]) {
    let base = Color::srgb_u8(color[0], color[1], color[2]);
    let mut rng = rand::rng();
    for _ in 0..5 {
        let ang = rng.random_range(0.0f32..TAU);
        let spd = rng.random_range(22.0..46.0);
        push_particle(
            commands,
            tiles.spark.clone(),
            pos,
            base,
            0.22,
            0.6,
            0.12,
            Vec3::new(ang.cos() * spd, ang.sin() * spd, 0.0),
            Vec3::ZERO,
            0.0,
        );
    }
}

fn death_burst(commands: &mut Commands, tiles: &Tiles, pos: Vec3, color: [u8; 3]) {
    let base = Color::srgb_u8(color[0], color[1], color[2]);
    push_particle(commands, tiles.ring.clone(), pos, base, 0.5, 0.3, 1.9, Vec3::ZERO, Vec3::ZERO, 0.0);
    let mut rng = rand::rng();
    for _ in 0..10 {
        let ang = rng.random_range(0.0f32..TAU);
        let spd = rng.random_range(40.0..72.0);
        push_particle(
            commands,
            tiles.spark.clone(),
            pos,
            base,
            0.55,
            0.55,
            0.1,
            Vec3::new(ang.cos() * spd, ang.sin() * spd, 0.0),
            Vec3::new(0.0, -34.0, 0.0),
            0.0,
        );
    }
}

/// Point FX (impacts, heals, deaths, teleports, barriers).
fn spawn_fx(commands: &mut Commands, tiles: &Tiles, ev: &Fx) {
    let pos = tile_pos(ev.pos.x, ev.pos.y, Z_FX);
    match ev.kind {
        FxKind::Hit => match ev.element {
            Some(el) => spawn_element_burst(commands, tiles, pos, el, 1.0),
            None => physical_burst(commands, tiles, pos, ev.color),
        },
        FxKind::Heal => spawn_element_burst(commands, tiles, pos, ElementKind::Heal, 1.0),
        FxKind::Death => death_burst(commands, tiles, pos, ev.color),
        FxKind::Teleport => spawn_element_burst(commands, tiles, pos, ElementKind::Teleport, 1.0),
        FxKind::Barrier => spawn_element_burst(commands, tiles, pos, ElementKind::Barrier, 1.0),
    }
}

/// Signature spell motion by form: projectile, beam, nova, aura, touch.
fn spawn_cast_fx(commands: &mut Commands, tiles: &Tiles, cf: &CastFx) {
    let c = cf.element.color();
    let base = Color::srgb_u8(c[0], c[1], c[2]);
    let from = tile_pos(cf.origin.x, cf.origin.y, Z_FX);
    let to = tile_pos(cf.target.x, cf.target.y, Z_FX);
    match cf.form {
        Form::Bolt => {
            let dist = (to - from).length() / TILE;
            let life = (dist * 0.035).clamp(0.08, 0.45);
            commands.spawn((
                Sprite {
                    image: tiles.spark.clone(),
                    color: base,
                    custom_size: Some(Vec2::splat(TILE * 0.9)),
                    ..default()
                },
                Transform {
                    translation: from,
                    scale: Vec3::splat(0.85),
                    ..default()
                },
                Projectile {
                    from,
                    to,
                    age: 0.0,
                    life,
                    element: cf.element,
                    trail: c,
                },
            ));
        }
        Form::Beam => draw_beam(commands, tiles, cf.origin, cf.target, cf.element),
        Form::Nova => {
            let ring_to = cf.radius.max(1) as f32 * 2.0 + 1.0;
            push_particle(commands, tiles.ring.clone(), from, base, 0.45, 0.4, ring_to, Vec3::ZERO, Vec3::ZERO, 0.0);
            spawn_element_burst(commands, tiles, from, cf.element, 1.3);
        }
        Form::OnSelf => {
            let ring_to = cf.radius.max(0) as f32 * 2.0 + 1.5;
            push_particle(commands, tiles.ring.clone(), from, base, 0.55, 0.2, ring_to, Vec3::ZERO, Vec3::ZERO, 1.5);
            spawn_element_burst(commands, tiles, from, cf.element, 0.8);
        }
        Form::Touch => spawn_element_burst(commands, tiles, to, cf.element, 1.0),
    }
}

/// Draw a beam as a row of oriented streak particles from origin to target.
fn draw_beam(commands: &mut Commands, tiles: &Tiles, origin: Point, target: Point, element: ElementKind) {
    let c = element.color();
    let base = Color::srgb_u8(c[0], c[1], c[2]);
    let from_w = tile_pos(origin.x, origin.y, Z_FX);
    let to_w = tile_pos(target.x, target.y, Z_FX);
    let angle = (to_w.truncate() - from_w.truncate()).to_angle();
    let dx = (target.x - origin.x).signum();
    let dy = (target.y - origin.y).signum();
    let mut p = origin;
    for _ in 0..40 {
        if p == target {
            break;
        }
        p = Point::new(p.x + dx, p.y + dy);
        let wp = tile_pos(p.x, p.y, Z_FX);
        commands.spawn((
            Sprite {
                image: tiles.streak.clone(),
                color: base,
                custom_size: Some(Vec2::splat(TILE)),
                ..default()
            },
            Transform {
                translation: wp,
                rotation: Quat::from_rotation_z(angle),
                ..default()
            },
            Particle {
                age: 0.0,
                life: 0.3,
                from_scale: 1.0,
                to_scale: 0.55,
                base,
                vel: Vec3::ZERO,
                accel: Vec3::ZERO,
                spin: 0.0,
            },
        ));
    }
}

// ---------------------------------------------------------------------------
// UI (Text2d, rebuilt each step)
// ---------------------------------------------------------------------------

fn text_block(commands: &mut Commands, x: f32, y: f32, size: f32, color: Color, text: String) {
    commands.spawn((
        Text2d::new(text),
        TextFont {
            font_size: size,
            ..default()
        },
        TextColor(color),
        TextLayout::new_with_justify(Justify::Left),
        Anchor::TOP_LEFT,
        Transform::from_xyz(x, y, Z_UI),
        StaticTag,
    ));
}

fn center_text(commands: &mut Commands, y: f32, size: f32, color: Color, text: &str) {
    commands.spawn((
        Text2d::new(text.to_string()),
        TextFont {
            font_size: size,
            ..default()
        },
        TextColor(color),
        TextLayout::new_with_justify(Justify::Center),
        Transform::from_xyz(0.0, y, 50.0),
        StaticTag,
    ));
}

fn draw_title(commands: &mut Commands, best: u32) {
    center_text(commands, 170.0, 72.0, Color::srgb(0.85, 0.8, 1.0), "RUNECASTER");
    center_text(
        commands,
        90.0,
        22.0,
        Color::srgb(0.65, 0.7, 0.85),
        "a rune-grammar roguelike",
    );
    center_text(
        commands,
        0.0,
        24.0,
        Color::srgb(0.95, 0.9, 0.6),
        "Press ENTER to descend into the Glyph Vaults",
    );
    center_text(
        commands,
        -90.0,
        16.0,
        Color::srgb(0.6, 0.62, 0.72),
        "Compose runes into spells. Bolt+Fire burns. Self+Fire burns YOU.\nMove wasd/hjkl/arrows (diag yubn) · number keys build a spell · Enter casts",
    );
    if best > 0 {
        center_text(
            commands,
            -190.0,
            18.0,
            Color::srgb(0.8, 0.75, 0.5),
            &format!("Best score: {best}"),
        );
    }
}

fn spawn_ui(commands: &mut Commands, sim: &Sim, best: u32) {
    let w = &sim.world;
    let p = w.player();

    text_block(
        commands,
        BOARD_LEFT,
        390.0,
        16.0,
        Color::srgb(0.9, 0.9, 1.0),
        format!(
            "RUNECASTER   Depth {}/{}   HP {}/{}   Mana {}/{}   Kills {}   Score {}   Turn {}",
            w.depth,
            MAX_DEPTH,
            p.hp,
            p.max_hp,
            p.mana,
            p.max_mana,
            w.kills,
            w.score(),
            w.turn
        ),
    );

    let panel_x = 462.0;
    let mut panel = String::from("RUNES — press the number\n");
    for (i, rune) in w.known_runes.iter().enumerate() {
        let key = (i + 1) % 10;
        let cat = match rune.category() {
            RuneCategory::Form => "form",
            RuneCategory::Element => "elem",
            RuneCategory::Modifier => "mod ",
        };
        panel.push_str(&format!("[{key}] {:<9} {cat}\n", rune.name()));
    }
    text_block(commands, panel_x, 372.0, 14.0, Color::srgb(0.8, 0.85, 0.95), panel);

    let mut word = String::from("WORD: ");
    if sim.composing.is_empty() {
        word.push_str("(empty)");
    } else {
        let names: Vec<&str> = sim.composing.iter().map(|r| r.name()).collect();
        word.push_str(&names.join(" + "));
    }
    word.push('\n');

    let mut word_color = Color::srgb(0.95, 0.95, 0.7);
    match w.preview(&sim.composing) {
        Ok(spell) => {
            word.push_str(&format!("=> {}\n", spell.describe()));
            let kind = if spell.heals() && !spell.is_damaging() {
                format!("heal {}", spell.power)
            } else {
                format!("power {}", spell.power)
            };
            word.push_str(&format!("   {kind}   cost {} mana\n", spell.mana_cost));
            if spell.hits_caster {
                word.push_str("   ! THIS WILL HIT YOU !\n");
                word_color = Color::srgb(1.0, 0.5, 0.4);
            }
            word.push_str(if spell.needs_aim() {
                "   Enter, then aim a direction\n"
            } else {
                "   Enter to cast\n"
            });
        }
        Err(e) => {
            if !sim.composing.is_empty() {
                word.push_str(&format!("   {}\n", e.message()));
                word_color = Color::srgb(1.0, 0.6, 0.5);
            }
        }
    }
    text_block(commands, panel_x, 150.0, 14.0, word_color, word);

    let hint = match sim.mode {
        Mode::Normal => {
            "move wasd/hjkl/arrows · diag yubn\nSpace wait · Shift+. descend\nnumbers compose · Enter cast · Esc clear"
                .to_string()
        }
        Mode::Aim => ">>> AIM: press a direction\n(Esc to cancel)".to_string(),
    };
    let hint_color = if sim.mode == Mode::Aim {
        Color::srgb(1.0, 0.9, 0.4)
    } else {
        Color::srgb(0.6, 0.62, 0.72)
    };
    text_block(commands, panel_x, 20.0, 13.0, hint_color, hint);

    if !w.relics.is_empty() {
        let mut rel = String::from("RELICS\n");
        for r in &w.relics {
            rel.push_str(&format!("* {}\n", r.name()));
        }
        text_block(commands, panel_x, -60.0, 13.0, Color::srgb(0.85, 0.78, 0.55), rel);
    }

    let start = w.log.len().saturating_sub(7);
    let log = w.log[start..].join("\n");
    text_block(commands, BOARD_LEFT, -296.0, 13.0, Color::srgb(0.7, 0.72, 0.8), log);

    if w.state != RunState::Playing {
        let (title, color) = match w.state {
            RunState::Won => ("VICTORY!", Color::srgb(0.5, 1.0, 0.6)),
            RunState::Dead => ("YOU DIED", Color::srgb(1.0, 0.4, 0.4)),
            RunState::Playing => unreachable!(),
        };
        center_text(commands, 60.0, 56.0, color, title);
        center_text(
            commands,
            -10.0,
            26.0,
            Color::srgb(0.92, 0.92, 1.0),
            &format!("Score {}      Best {}", w.score(), best.max(w.score())),
        );
        center_text(
            commands,
            -70.0,
            20.0,
            Color::srgb(0.7, 0.72, 0.82),
            "Enter / R for a new run",
        );
    }
}

// ---------------------------------------------------------------------------
// High-score persistence
// ---------------------------------------------------------------------------

fn highscore_path() -> std::path::PathBuf {
    match std::env::var("HOME") {
        Ok(home) => std::path::Path::new(&home).join(".runecaster_highscore"),
        Err(_) => std::path::PathBuf::from("runecaster_highscore.txt"),
    }
}

fn load_highscore() -> u32 {
    std::fs::read_to_string(highscore_path())
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

fn save_highscore(v: u32) {
    let _ = std::fs::write(highscore_path(), v.to_string());
}

// ---------------------------------------------------------------------------
// Procedural 16x16 sprite textures
// ---------------------------------------------------------------------------

const SPRITE_SIZE: u32 = 16;

fn make_image(images: &mut Assets<Image>, paint: impl Fn(u32, u32) -> [u8; 4]) -> Handle<Image> {
    let s = SPRITE_SIZE;
    let mut data = vec![0u8; (s * s * 4) as usize];
    for y in 0..s {
        for x in 0..s {
            let px = paint(x, y);
            let i = ((y * s + x) * 4) as usize;
            data[i..i + 4].copy_from_slice(&px);
        }
    }
    let mut img = Image::new(
        Extent3d {
            width: s,
            height: s,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    );
    img.sampler = ImageSampler::nearest();
    images.add(img)
}

fn disc(x: u32, y: u32, cx: f32, cy: f32, r: f32) -> bool {
    let dx = x as f32 + 0.5 - cx;
    let dy = y as f32 + 0.5 - cy;
    dx * dx + dy * dy <= r * r
}

fn px_floor(x: u32, y: u32) -> [u8; 4] {
    let mut c = [26u8, 28, 40, 255];
    if (x.wrapping_mul(7).wrapping_add(y.wrapping_mul(13))) % 11 == 0 {
        c = [34, 37, 52, 255];
    }
    if x == 0 || y == 0 {
        c = [20, 21, 31, 255];
    }
    c
}

fn px_wall(x: u32, y: u32) -> [u8; 4] {
    let mut c = [52u8, 56, 84, 255];
    if y < 3 {
        c = [80, 86, 120, 255];
    } else if y >= 13 {
        c = [32, 35, 54, 255];
    }
    if y == 8 {
        c = [38, 41, 63, 255];
    }
    if x == 0 {
        c = [
            c[0].saturating_add(16),
            c[1].saturating_add(16),
            c[2].saturating_add(18),
            255,
        ];
    }
    if x == 15 {
        c = [28, 30, 48, 255];
    }
    c
}

fn px_stairs(x: u32, y: u32) -> [u8; 4] {
    let c = px_floor(x, y);
    let yi = y as i32;
    if (2..=13).contains(&yi) {
        let band = (yi - 2) % 3;
        let step = ((yi - 2) / 3).max(0) as u32;
        if band < 2 && x >= step + 1 && x <= 13 {
            return [214, 182, 86, 255];
        }
    }
    c
}

fn px_creature(x: u32, y: u32) -> [u8; 4] {
    let (cx, cy) = (8.0, 9.0);
    if disc(x, y, cx, cy, 5.7) {
        if disc(x, y, 5.6, 7.6, 1.25) || disc(x, y, 10.4, 7.6, 1.25) {
            return [20, 20, 28, 255];
        }
        if !disc(x, y, cx, cy, 4.8) {
            return [200, 200, 212, 255];
        }
        return [246, 246, 250, 255];
    }
    [0, 0, 0, 0]
}

fn px_gem(x: u32, y: u32) -> [u8; 4] {
    let dx = (x as i32 - 8).abs();
    let dy = (y as i32 - 8).abs();
    let d = dx + dy;
    if d <= 5 {
        if x as i32 + y as i32 <= 14 {
            [250, 250, 255, 255]
        } else {
            [196, 196, 210, 255]
        }
    } else if d == 6 {
        [120, 120, 138, 255]
    } else {
        [0, 0, 0, 0]
    }
}

fn px_relic(x: u32, y: u32) -> [u8; 4] {
    let ax = (x as i32 - 8).abs();
    let ay = (y as i32 - 8).abs();
    if ax + ay <= 3 {
        return [255, 228, 120, 255]; // bright core
    }
    if (ax <= 1 && ay <= 6) || (ay <= 1 && ax <= 6) {
        return [255, 205, 80, 255]; // four spikes
    }
    [0, 0, 0, 0]
}

fn px_spark(x: u32, y: u32) -> [u8; 4] {
    let dx = x as f32 + 0.5 - 8.0;
    let dy = y as f32 + 0.5 - 8.0;
    let d = (dx * dx + dy * dy).sqrt();
    let r = 7.5;
    if d <= r {
        let a = (1.0 - d / r).clamp(0.0, 1.0);
        [255, 255, 255, (a * a * 255.0) as u8]
    } else {
        [0, 0, 0, 0]
    }
}

fn px_player(x: u32, y: u32) -> [u8; 4] {
    let xi = x as i32;
    let yi = y as i32;
    if yi < 6 {
        let hw = yi;
        if (xi - 8).abs() <= hw {
            return if yi >= 5 {
                [60, 90, 170, 255]
            } else {
                [82, 122, 212, 255]
            };
        }
    }
    if (6..9).contains(&yi) && (xi - 8).abs() <= 2 {
        if disc(x, y, 6.6, 7.4, 0.7) || disc(x, y, 9.4, 7.4, 0.7) {
            return [40, 40, 55, 255];
        }
        return [240, 220, 190, 255];
    }
    if yi >= 9 {
        let hw = 2 + (yi - 9) / 2;
        if (xi - 8).abs() <= hw {
            if yi == 11 {
                return [150, 120, 60, 255];
            }
            return [214, 219, 238, 255];
        }
    }
    [0, 0, 0, 0]
}

/// Hollow soft ring — shockwaves, novas, auras, teleport.
fn px_ring(x: u32, y: u32) -> [u8; 4] {
    let dx = x as f32 + 0.5 - 8.0;
    let dy = y as f32 + 0.5 - 8.0;
    let d = (dx * dx + dy * dy).sqrt();
    let (inner, outer) = (5.0, 7.6);
    if d >= inner && d <= outer {
        let edge = ((d - inner).min(outer - d) / ((outer - inner) * 0.5)).clamp(0.0, 1.0);
        [255, 255, 255, (edge * 255.0) as u8]
    } else {
        [0, 0, 0, 0]
    }
}

/// Tall crystalline shard — frost / barrier.
fn px_shard(x: u32, y: u32) -> [u8; 4] {
    let dx = (x as f32 + 0.5 - 8.0).abs();
    let dy = (y as f32 + 0.5 - 8.0).abs();
    let d = dx * 1.7 + dy * 0.85;
    if d <= 6.5 {
        if d <= 3.0 {
            [255, 255, 255, 255]
        } else {
            [220, 235, 255, 235]
        }
    } else {
        [0, 0, 0, 0]
    }
}

/// Horizontal streak — lightning / beam segments (rotated to the beam angle).
fn px_streak(x: u32, y: u32) -> [u8; 4] {
    let dy = (y as f32 + 0.5 - 8.0).abs();
    let half = 2.3;
    if dy <= half {
        let band = (1.0 - dy / half).clamp(0.0, 1.0);
        let dx = (x as f32 + 0.5 - 8.0).abs();
        let ends = (1.0 - (dx / 8.0) * 0.5).clamp(0.0, 1.0);
        [255, 255, 255, ((band * ends) * 255.0) as u8]
    } else {
        [0, 0, 0, 0]
    }
}
