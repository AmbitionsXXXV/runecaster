use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

#[derive(Resource)]
pub(crate) struct Tiles {
    pub(crate) floor: Handle<Image>,
    pub(crate) wall: Handle<Image>,
    pub(crate) stairs: Handle<Image>,
    pub(crate) player: Handle<Image>,
    pub(crate) creature: Handle<Image>,
    pub(crate) gem: Handle<Image>,
    pub(crate) relic: Handle<Image>,
    pub(crate) spark: Handle<Image>,
    pub(crate) ring: Handle<Image>,
    pub(crate) shard: Handle<Image>,
    pub(crate) streak: Handle<Image>,
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
    if (x.wrapping_mul(7).wrapping_add(y.wrapping_mul(13))).is_multiple_of(11) {
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
        if band < 2 && x > step && x <= 13 {
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

/// Build the full procedural tile set (moved verbatim from `setup`).
pub(crate) fn build(images: &mut Assets<Image>) -> Tiles {
    Tiles {
        floor: make_image(images, px_floor),
        wall: make_image(images, px_wall),
        stairs: make_image(images, px_stairs),
        player: make_image(images, px_player),
        creature: make_image(images, px_creature),
        gem: make_image(images, px_gem),
        relic: make_image(images, px_relic),
        spark: make_image(images, px_spark),
        ring: make_image(images, px_ring),
        shard: make_image(images, px_shard),
        streak: make_image(images, px_streak),
    }
}
