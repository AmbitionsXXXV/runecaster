use crate::fx::{spawn_cast_fx, spawn_fx, Fading, Flash, Lunge};
use crate::state::{
    save_highscore, ActorId, ActorViews, AnimTarget, HighScore, MainCamera, Mode, Screen, Shake,
    Sim, StaticTag,
};
use crate::textures::Tiles;
use crate::ui::{draw_title, spawn_ui};
use crate::{BOARD_LEFT, BOARD_TOP, TILE, Z_ACTOR, Z_ITEM, Z_TILE};
use bevy::prelude::*;
use rand::RngExt;
use rune_core::{Dir, EntityId, Form, FxKind, Point, RunState, Spell, Tile, World};
use std::collections::HashSet;

// ---------------------------------------------------------------------------
// Coordinate mapping
// ---------------------------------------------------------------------------

pub(crate) fn tile_pos(tx: i32, ty: i32, z: f32) -> Vec3 {
    Vec3::new(
        BOARD_LEFT + tx as f32 * TILE + TILE / 2.0,
        BOARD_TOP - ty as f32 * TILE - TILE / 2.0,
        z,
    )
}

// ---------------------------------------------------------------------------
// Per-step rebuild + actor reconciliation + FX spawning
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub(crate) fn sync(
    mut commands: Commands,
    mut sim: ResMut<Sim>,
    mut views: ResMut<ActorViews>,
    mut high: ResMut<HighScore>,
    mut shake: ResMut<Shake>,
    tiles: Res<Tiles>,
    statics: Query<Entity, With<StaticTag>>,
    fading: Query<Entity, With<Fading>>,
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

    // Hard reset (restart / descend) → drop all actor sprites, including
    // corpses still mid-fade: they belong to the level being left behind.
    if views.last_gen != sim.level_gen {
        for (_, e) in views.map.drain() {
            commands.entity(e).despawn();
        }
        for e in fading.iter() {
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
            // A sim entity that died this step fades out where it fell
            // instead of vanishing on the exact frame; anything else
            // (stale for structural reasons) still despawns instantly.
            let died_here = w.entities.get(id).is_some_and(|ent| !ent.alive);
            if died_here {
                commands
                    .entity(e)
                    .remove::<AnimTarget>()
                    .remove::<Flash>()
                    .insert(Fading { age: 0.0 });
            } else {
                commands.entity(e).despawn();
            }
        }
    }

    // --- fx-driven combat juice: flash the struck, lunge the striker, and
    // nudge the camera when a blow lands or something dies ---
    let player_pos = w.player().pos;
    for ev in &fx {
        match ev.kind {
            FxKind::Hit => {
                if let Some(ent) = w.entities.iter().find(|e| e.alive && e.pos == ev.pos) {
                    if let Some(&view) = views.map.get(&ent.id) {
                        let base = actor_sprite(ent, &tiles).color;
                        // Monsters blaze white (they're tinted, so it reads);
                        // the player is already white, so wash red instead.
                        let peak = if ent.is_player() {
                            Color::srgb(1.0, 0.32, 0.28)
                        } else {
                            Color::WHITE
                        };
                        commands.entity(view).insert(Flash {
                            age: 0.0,
                            base,
                            peak,
                        });
                    }
                }
                // Landing a hit on someone else nudges the camera a little
                // (taking damage already ramps it via the HP delta above).
                if ev.pos != player_pos {
                    shake.trauma = (shake.trauma + 0.05).min(1.0);
                }
                // Melee: dart the attacker toward its victim.
                if let Some(from) = ev.from {
                    if let Some(att) = w.entities.iter().find(|e| e.alive && e.pos == from) {
                        if let Some(&view) = views.map.get(&att.id) {
                            let dir = (tile_pos(ev.pos.x, ev.pos.y, 0.0)
                                - tile_pos(from.x, from.y, 0.0))
                            .truncate()
                            .normalize_or_zero();
                            commands.entity(view).insert(Lunge { dir, age: 0.0 });
                        }
                    }
                }
            }
            FxKind::Death => shake.trauma = (shake.trauma + 0.08).min(1.0),
            _ => {}
        }
    }

    // --- aim preview: see what the loaded word hits *before* committing ---
    if sim.mode == Mode::Aim {
        if let Ok(spell) = w.preview(&sim.composing) {
            draw_aim_preview(&mut commands, w, &spell, sim.aim_dir);
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

/// Paint the aim-mode preview: the tiles the loaded word would strike from
/// the player's tile along `aim` (element-colored fill), a dim flight path
/// for stop-at-first-target bolts, and a bright pip on each enemy the
/// chain/arc is predicted to jump to. Backed by `World::preview_affected`,
/// which mirrors the real resolution — the highlight is a promise, not a
/// guess. Only currently-visible tiles are painted so the preview never
/// reveals map the player hasn't seen.
fn draw_aim_preview(commands: &mut Commands, w: &World, spell: &Spell, aim: Dir) {
    let (tiles, chain) = w.preview_affected(spell, aim);
    let [r, g, b] = spell.primary_color();
    let tint = |a: f32| Color::srgba(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a);

    // Cosmetic flight path up to the first impact tile (Bolt only: a Beam's
    // affected tiles already *are* the line, Touch is adjacent).
    if spell.form == Form::Bolt {
        let mut p = w.player().pos;
        for _ in 0..spell.range {
            p = p.step(aim);
            if !w.map.in_bounds(p) || w.map.is_opaque(p) || tiles.contains(&p) {
                break;
            }
            if w.is_visible(p) {
                spawn_highlight(commands, p, tint(0.14), 1.0, Z_TILE + 0.6);
            }
        }
    }

    for p in &tiles {
        if w.is_visible(*p) {
            spawn_highlight(commands, *p, tint(0.32), 1.0, Z_TILE + 0.6);
        }
    }
    // Chain victims render above the actor sprite: "it will also jump to
    // THAT one" has to read at a glance.
    for p in &chain {
        if w.is_visible(*p) {
            spawn_highlight(commands, *p, tint(0.7), 0.45, Z_ACTOR + 0.5);
        }
    }
}

fn spawn_highlight(commands: &mut Commands, p: Point, color: Color, scale: f32, z: f32) {
    commands.spawn((
        Sprite::from_color(color, Vec2::splat(TILE * scale)),
        Transform::from_translation(tile_pos(p.x, p.y, z)),
        StaticTag,
    ));
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

pub(crate) fn animate(time: Res<Time>, mut q: Query<(&mut Transform, &AnimTarget)>) {
    let t = (time.delta_secs() * 16.0).min(1.0);
    for (mut tf, target) in &mut q {
        tf.translation = tf.translation.lerp(target.0, t);
    }
}

pub(crate) fn camera_shake(
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
