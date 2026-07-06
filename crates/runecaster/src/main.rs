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

use bevy::prelude::*;

mod fx;
mod input;
mod render;
mod state;
mod textures;
mod ui;

use state::{load_highscore, ActorViews, HighScore, MainCamera, Shake, Sim};

pub(crate) const TILE: f32 = 18.0;
pub(crate) const WINDOW_W: f32 = 1280.0;
pub(crate) const WINDOW_H: f32 = 800.0;
pub(crate) const BOARD_LEFT: f32 = -630.0;
pub(crate) const BOARD_TOP: f32 = 360.0;

pub(crate) const Z_TILE: f32 = 0.0;
pub(crate) const Z_ITEM: f32 = 1.0;
pub(crate) const Z_ACTOR: f32 = 5.0;
pub(crate) const Z_UI: f32 = 10.0;
pub(crate) const Z_FX: f32 = 20.0;

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
            (
                input::handle_input,
                render::sync,
                render::animate,
                fx::particles,
                fx::projectiles,
                render::camera_shake,
            )
                .chain(),
        )
        .run();
}

fn setup(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    commands.spawn((Camera2d, MainCamera));
    commands.insert_resource(textures::build(&mut images));
}
