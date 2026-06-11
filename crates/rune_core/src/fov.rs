//! A simple, permissive field of view: a tile within `radius` is visible if a
//! Bresenham line from the origin reaches it without first crossing an opaque
//! tile. Walls that the line *ends* on are themselves visible (so you can see
//! the walls of the room you're in).

use crate::geometry::Point;
use crate::map::Map;

pub fn compute_fov(map: &Map, origin: Point, radius: i32) -> Vec<bool> {
    let mut visible = vec![false; (map.width * map.height) as usize];
    if map.in_bounds(origin) {
        visible[map.idx(origin)] = true;
    }
    let r2 = radius * radius;

    for y in (origin.y - radius)..=(origin.y + radius) {
        for x in (origin.x - radius)..=(origin.x + radius) {
            let target = Point::new(x, y);
            if !map.in_bounds(target) {
                continue;
            }
            let dx = x - origin.x;
            let dy = y - origin.y;
            if dx * dx + dy * dy > r2 {
                continue;
            }
            if line_reaches(map, origin, target) {
                visible[map.idx(target)] = true;
            }
        }
    }
    visible
}

/// True if the straight line from `a` to `b` is not blocked *before* reaching
/// `b`. `b` itself may be opaque and still count as reached (you see the wall).
fn line_reaches(map: &Map, a: Point, b: Point) -> bool {
    let (mut x, mut y) = (a.x, a.y);
    let dx = (b.x - x).abs();
    let dy = -(b.y - y).abs();
    let sx = if x < b.x { 1 } else { -1 };
    let sy = if y < b.y { 1 } else { -1 };
    let mut err = dx + dy;

    loop {
        if x == b.x && y == b.y {
            return true;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
        if x == b.x && y == b.y {
            return true; // reached the target tile; visible even if it's a wall
        }
        if map.is_opaque(Point::new(x, y)) {
            return false; // an intermediate tile blocks the line
        }
    }
}
