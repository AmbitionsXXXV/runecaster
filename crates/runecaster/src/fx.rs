use crate::render::tile_pos;
use crate::textures::Tiles;
use crate::{TILE, Z_FX};
use bevy::color::Mix;
use bevy::prelude::*;
use rand::RngExt;
use rune_core::{CastFx, ElementKind, Form, Fx, FxKind, Point};
use std::f32::consts::{PI, TAU};

#[derive(Component)]
pub(crate) struct Particle {
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
pub(crate) struct Projectile {
    from: Vec3,
    to: Vec3,
    age: f32,
    life: f32,
    element: ElementKind,
    trail: [u8; 3],
}

/// A floating combat number (damage or heal) that rises and fades; spawned
/// alongside the impact burst so hits read at a glance, not just as color.
#[derive(Component)]
pub(crate) struct FloatText {
    pub age: f32,
    pub life: f32,
    pub vel: Vec2,
    pub base: Color,
}

/// A struck actor's sprite blazes `peak` for an instant and decays back to
/// `base` — the classic 0.1s hit flash. `base` is recomputed from the
/// actor's glyph at insert time, so re-flashing mid-flash can't drift it.
#[derive(Component)]
pub(crate) struct Flash {
    pub age: f32,
    pub base: Color,
    pub peak: Color,
}

const FLASH_LIFE: f32 = 0.11;

/// A melee attacker's sprite darts toward its victim and settles back — a
/// half-sine offset composed on top of the `animate` lerp.
#[derive(Component)]
pub(crate) struct Lunge {
    pub dir: Vec2,
    pub age: f32,
}

const LUNGE_LIFE: f32 = 0.16;
const LUNGE_DIST: f32 = 5.5;

/// A dying actor's sprite fades and shrinks out instead of vanishing on the
/// exact frame the sim marks it dead.
#[derive(Component)]
pub(crate) struct Fading {
    pub age: f32,
}

const FADE_LIFE: f32 = 0.35;

/// Advance hit flashes: hold-and-decay from `peak` back to `base`.
pub(crate) fn flashes(
    time: Res<Time>,
    mut commands: Commands,
    mut q: Query<(Entity, &mut Flash, &mut Sprite)>,
) {
    let dt = time.delta_secs();
    for (e, mut f, mut sp) in &mut q {
        f.age += dt;
        if f.age >= FLASH_LIFE {
            sp.color = f.base;
            commands.entity(e).remove::<Flash>();
            continue;
        }
        let t = (f.age / FLASH_LIFE).clamp(0.0, 1.0);
        sp.color = f.peak.mix(&f.base, t);
    }
}

/// Advance melee lunges: a sin-arc dart toward the victim and back. Runs
/// after `render::animate` (see the system chain), whose lerp reclaims any
/// leftover offset, so the two compose without fighting.
pub(crate) fn lunges(
    time: Res<Time>,
    mut commands: Commands,
    mut q: Query<(Entity, &mut Lunge, &mut Transform)>,
) {
    let dt = time.delta_secs();
    for (e, mut l, mut tf) in &mut q {
        l.age += dt;
        if l.age >= LUNGE_LIFE {
            commands.entity(e).remove::<Lunge>();
            continue;
        }
        let k = (PI * l.age / LUNGE_LIFE).sin();
        tf.translation += (l.dir * LUNGE_DIST * k).extend(0.0);
    }
}

/// Advance death fades: alpha out with a slight shrink, then despawn.
pub(crate) fn fades(
    time: Res<Time>,
    mut commands: Commands,
    mut q: Query<(Entity, &mut Fading, &mut Sprite, &mut Transform)>,
) {
    let dt = time.delta_secs();
    for (e, mut f, mut sp, mut tf) in &mut q {
        f.age += dt;
        if f.age >= FADE_LIFE {
            commands.entity(e).despawn();
            continue;
        }
        let t = f.age / FADE_LIFE;
        sp.color = sp.color.with_alpha(1.0 - t);
        tf.scale = Vec3::splat(1.0 - 0.25 * t);
    }
}

pub(crate) fn particles(
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
pub(crate) fn projectiles(
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

/// Advance floating combat numbers: rise, decelerate, and fade to
/// transparent over their lifetime; despawn once they expire.
pub(crate) fn float_texts(
    time: Res<Time>,
    mut commands: Commands,
    mut q: Query<(Entity, &mut FloatText, &mut Transform, &mut TextColor)>,
) {
    let dt = time.delta_secs();
    for (e, mut ft, mut tf, mut color) in &mut q {
        ft.age += dt;
        if ft.age >= ft.life {
            commands.entity(e).despawn();
            continue;
        }
        tf.translation += ft.vel.extend(0.0) * dt;
        ft.vel.y *= 1.0 - dt * 0.3;
        let t = ft.age / ft.life;
        *color = TextColor(ft.base.with_alpha(1.0 - t));
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

/// Per-element burst recipe: texture, count, and motion. Color is *not*
/// part of the recipe — every burst derives its tint from the element's
/// one canonical color (`ElementKind::color()` in runes.rs), so particle,
/// UI, and preview colors can never drift apart.
struct Burst {
    image: Handle<Image>,
    count: u32,
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
        Fire => Burst {
            image: tiles.spark.clone(),
            count: 9,
            jitter: 40,
            life: 0.55,
            speed: 26.0,
            accel_y: 40.0,
            s0: 0.55,
            s1: 0.12,
            spin: 0.0,
        },
        // shards that scatter and fall
        Frost => Burst {
            image: tiles.shard.clone(),
            count: 8,
            jitter: 30,
            life: 0.55,
            speed: 44.0,
            accel_y: -70.0,
            s0: 0.5,
            s1: 0.42,
            spin: 6.0,
        },
        // quick bright streaks
        Lightning => Burst {
            image: tiles.streak.clone(),
            count: 7,
            jitter: 20,
            life: 0.26,
            speed: 72.0,
            accel_y: 0.0,
            s0: 0.7,
            s1: 0.2,
            spin: 0.0,
        },
        // a violet shove
        Force => Burst {
            image: tiles.spark.clone(),
            count: 6,
            jitter: 25,
            life: 0.4,
            speed: 56.0,
            accel_y: 0.0,
            s0: 0.6,
            s1: 0.2,
            spin: 0.0,
        },
        // magenta motes
        Drain => Burst {
            image: tiles.spark.clone(),
            count: 7,
            jitter: 25,
            life: 0.5,
            speed: 30.0,
            accel_y: 12.0,
            s0: 0.5,
            s1: 0.14,
            spin: 0.0,
        },
        // green motes rising
        Heal => Burst {
            image: tiles.spark.clone(),
            count: 7,
            jitter: 25,
            life: 0.7,
            speed: 18.0,
            accel_y: 34.0,
            s0: 0.5,
            s1: 0.2,
            spin: 0.0,
        },
        // imploding cyan ring + motes
        Teleport => Burst {
            image: tiles.ring.clone(),
            count: 3,
            jitter: 15,
            life: 0.5,
            speed: 8.0,
            accel_y: 0.0,
            s0: 1.3,
            s1: 0.1,
            spin: 2.5,
        },
        // settling stone shards
        Barrier => Burst {
            image: tiles.shard.clone(),
            count: 6,
            jitter: 20,
            life: 0.5,
            speed: 20.0,
            accel_y: -24.0,
            s0: 0.5,
            s1: 0.5,
            spin: 3.0,
        },
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
    // Single-source tint: the element's canonical color, uniformly lifted
    // a touch so bursts pop brighter than flat UI swatches — one rule
    // instead of a hand-tuned per-element palette that can drift.
    let ec = element.color();
    const LIFT: i32 = 18;
    let mut rng = rand::rng();
    let n = ((b.count as f32) * intensity).round().max(1.0) as u32;
    for _ in 0..n {
        let ang = rng.random_range(0.0f32..TAU);
        let spd = rng.random_range(b.speed * 0.35..b.speed.max(0.1));
        let vel = Vec3::new(ang.cos() * spd, ang.sin() * spd, 0.0);
        let j = b.jitter;
        let base = Color::srgb_u8(
            (ec[0] as i32 + LIFT + rng.random_range(-j..=j)).clamp(0, 255) as u8,
            (ec[1] as i32 + LIFT + rng.random_range(-j..=j)).clamp(0, 255) as u8,
            (ec[2] as i32 + LIFT + rng.random_range(-j..=j)).clamp(0, 255) as u8,
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
    push_particle(
        commands,
        tiles.ring.clone(),
        pos,
        base,
        0.5,
        0.3,
        1.9,
        Vec3::ZERO,
        Vec3::ZERO,
        0.0,
    );
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

/// Blend an `[u8;3]` color toward white by `t` (0..1) for a softer, more
/// legible floating combat number than the raw impact-particle color.
fn blend_white(c: [u8; 3], t: f32) -> Color {
    let mix = |ch: u8| (ch as f32 / 255.0) * (1.0 - t) + t;
    Color::srgb(mix(c[0]), mix(c[1]), mix(c[2]))
}

/// Spawn a rising, fading damage/heal number at `pos` (already at fx height).
fn spawn_float_text(commands: &mut Commands, pos: Vec3, text: String, base: Color) {
    let mut rng = rand::rng();
    let jitter_x = rng.random_range(-5.0f32..5.0);
    commands.spawn((
        Text2d::new(text),
        TextFont {
            font_size: 15.0,
            ..default()
        },
        TextColor(base),
        Transform::from_translation(pos + Vec3::new(jitter_x, 6.0, 21.0)),
        FloatText {
            age: 0.0,
            life: 0.8,
            vel: Vec2::new(0.0, 26.0),
            base,
        },
    ));
}

/// Point FX (impacts, heals, deaths, teleports, barriers).
pub(crate) fn spawn_fx(commands: &mut Commands, tiles: &Tiles, ev: &Fx) {
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

    // Float a combat number over hits and heals so magnitude reads at a
    // glance; other kinds never carry an amount (stays 0), so this is a
    // no-op for them.
    if ev.amount > 0 {
        match ev.kind {
            FxKind::Hit => spawn_float_text(
                commands,
                pos,
                format!("-{}", ev.amount),
                blend_white(ev.color, 0.35),
            ),
            FxKind::Heal => spawn_float_text(
                commands,
                pos,
                format!("+{}", ev.amount),
                Color::srgb(0.55, 1.0, 0.65),
            ),
            _ => {}
        }
    }
}

/// Signature spell motion by form: projectile, beam, nova, aura, touch.
pub(crate) fn spawn_cast_fx(commands: &mut Commands, tiles: &Tiles, cf: &CastFx) {
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
            push_particle(
                commands,
                tiles.ring.clone(),
                from,
                base,
                0.45,
                0.4,
                ring_to,
                Vec3::ZERO,
                Vec3::ZERO,
                0.0,
            );
            spawn_element_burst(commands, tiles, from, cf.element, 1.3);
        }
        Form::OnSelf => {
            let ring_to = cf.radius.max(0) as f32 * 2.0 + 1.5;
            push_particle(
                commands,
                tiles.ring.clone(),
                from,
                base,
                0.55,
                0.2,
                ring_to,
                Vec3::ZERO,
                Vec3::ZERO,
                1.5,
            );
            spawn_element_burst(commands, tiles, from, cf.element, 0.8);
        }
        Form::Touch => spawn_element_burst(commands, tiles, to, cf.element, 1.0),
    }
}

/// Draw a beam as a row of oriented streak particles from origin to target.
fn draw_beam(
    commands: &mut Commands,
    tiles: &Tiles,
    origin: Point,
    target: Point,
    element: ElementKind,
) {
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
