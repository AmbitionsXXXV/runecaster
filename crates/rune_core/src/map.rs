//! The dungeon grid and a classic "random rooms + L-shaped corridors" generator.

use crate::geometry::{Coord, Point};
use rand::rngs::StdRng;
use rand::RngExt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tile {
    Wall,
    Floor,
    StairsDown,
}

impl Tile {
    pub fn blocks_movement(self) -> bool {
        matches!(self, Tile::Wall)
    }
    pub fn blocks_sight(self) -> bool {
        matches!(self, Tile::Wall)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Rect {
    pub x1: Coord,
    pub y1: Coord,
    pub x2: Coord,
    pub y2: Coord,
}

impl Rect {
    pub fn new(x: Coord, y: Coord, w: Coord, h: Coord) -> Self {
        Self {
            x1: x,
            y1: y,
            x2: x + w,
            y2: y + h,
        }
    }

    pub fn center(self) -> Point {
        Point::new((self.x1 + self.x2) / 2, (self.y1 + self.y2) / 2)
    }

    /// True if the two rectangles overlap (with a one-tile margin so rooms
    /// never share a wall directly).
    pub fn intersects(self, other: &Rect) -> bool {
        self.x1 <= other.x2 && self.x2 >= other.x1 && self.y1 <= other.y2 && self.y2 >= other.y1
    }
}

#[derive(Debug, Clone)]
pub struct Map {
    pub width: Coord,
    pub height: Coord,
    pub tiles: Vec<Tile>,
    pub rooms: Vec<Rect>,
    pub stairs: Point,
}

impl Map {
    pub fn idx(&self, p: Point) -> usize {
        (p.y * self.width + p.x) as usize
    }

    pub fn in_bounds(&self, p: Point) -> bool {
        p.x >= 0 && p.y >= 0 && p.x < self.width && p.y < self.height
    }

    pub fn tile(&self, p: Point) -> Tile {
        if self.in_bounds(p) {
            self.tiles[self.idx(p)]
        } else {
            Tile::Wall
        }
    }

    pub fn set(&mut self, p: Point, t: Tile) {
        if self.in_bounds(p) {
            let i = self.idx(p);
            self.tiles[i] = t;
        }
    }

    pub fn is_walkable(&self, p: Point) -> bool {
        self.in_bounds(p) && !self.tile(p).blocks_movement()
    }

    /// Opaque tiles (and everything off the map) block line of sight.
    pub fn is_opaque(&self, p: Point) -> bool {
        !self.in_bounds(p) || self.tile(p).blocks_sight()
    }

    /// Generate a level. Returns the finished map and the player's start tile
    /// (the centre of the first room). Stairs down sit in the last room.
    pub fn generate(width: Coord, height: Coord, rng: &mut StdRng) -> (Map, Point) {
        const MAX_ROOMS: i32 = 16;
        const MIN_SIZE: Coord = 4;
        const MAX_SIZE: Coord = 9;

        let mut map = Map {
            width,
            height,
            tiles: vec![Tile::Wall; (width * height) as usize],
            rooms: Vec::new(),
            stairs: Point::new(0, 0),
        };

        for _ in 0..MAX_ROOMS {
            let w = rng.random_range(MIN_SIZE..=MAX_SIZE);
            let h = rng.random_range(MIN_SIZE..=MAX_SIZE);
            let x = rng.random_range(1..(width - w - 1).max(2));
            let y = rng.random_range(1..(height - h - 1).max(2));
            let room = Rect::new(x, y, w, h);

            if map.rooms.iter().any(|r| room.intersects(r)) {
                continue;
            }

            map.carve_room(&room);

            if let Some(prev) = map.rooms.last().copied() {
                let a = prev.center();
                let b = room.center();
                if rng.random_bool(0.5) {
                    map.carve_h_tunnel(a.x, b.x, a.y);
                    map.carve_v_tunnel(a.y, b.y, b.x);
                } else {
                    map.carve_v_tunnel(a.y, b.y, a.x);
                    map.carve_h_tunnel(a.x, b.x, b.y);
                }
            }

            map.rooms.push(room);
        }

        let start = map.rooms[0].center();
        let stairs = map.rooms.last().unwrap().center();
        map.set(stairs, Tile::StairsDown);
        map.stairs = stairs;
        (map, start)
    }

    fn carve_room(&mut self, room: &Rect) {
        for y in (room.y1 + 1)..room.y2 {
            for x in (room.x1 + 1)..room.x2 {
                self.set(Point::new(x, y), Tile::Floor);
            }
        }
    }

    fn carve_h_tunnel(&mut self, x1: Coord, x2: Coord, y: Coord) {
        for x in x1.min(x2)..=x1.max(x2) {
            self.set(Point::new(x, y), Tile::Floor);
        }
    }

    fn carve_v_tunnel(&mut self, y1: Coord, y2: Coord, x: Coord) {
        for y in y1.min(y2)..=y1.max(y2) {
            self.set(Point::new(x, y), Tile::Floor);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    #[test]
    fn generation_is_deterministic_and_connected_ish() {
        let mut a = StdRng::seed_from_u64(42);
        let mut b = StdRng::seed_from_u64(42);
        let (m1, s1) = Map::generate(60, 36, &mut a);
        let (m2, s2) = Map::generate(60, 36, &mut b);
        assert_eq!(s1, s2, "same seed -> same start");
        assert_eq!(m1.tiles, m2.tiles, "same seed -> same map");
        assert!(!m1.rooms.is_empty());
        assert_eq!(m1.tile(s1), Tile::Floor, "player must start on floor");
        assert_eq!(m1.tile(m1.stairs), Tile::StairsDown);
    }
}
