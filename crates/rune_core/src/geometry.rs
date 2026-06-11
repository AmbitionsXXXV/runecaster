//! Grid geometry: integer points and the eight compass directions.
//!
//! Convention: the y axis points *down* (row 0 is the top). The renderer is
//! responsible for flipping to its own coordinate system.

pub type Coord = i32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Point {
    pub x: Coord,
    pub y: Coord,
}

impl Point {
    pub const fn new(x: Coord, y: Coord) -> Self {
        Self { x, y }
    }

    pub fn offset(self, dx: Coord, dy: Coord) -> Self {
        Self::new(self.x + dx, self.y + dy)
    }

    pub fn step(self, dir: Dir) -> Self {
        let (dx, dy) = dir.delta();
        self.offset(dx, dy)
    }

    /// 4-directional ("taxicab") distance.
    pub fn manhattan(self, other: Point) -> Coord {
        (self.x - other.x).abs() + (self.y - other.y).abs()
    }

    /// 8-directional ("king move") distance.
    pub fn chebyshev(self, other: Point) -> Coord {
        (self.x - other.x).abs().max((self.y - other.y).abs())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    North,
    South,
    East,
    West,
    NorthEast,
    NorthWest,
    SouthEast,
    SouthWest,
}

impl Dir {
    pub const CARDINAL: [Dir; 4] = [Dir::North, Dir::South, Dir::East, Dir::West];
    pub const ALL: [Dir; 8] = [
        Dir::North,
        Dir::South,
        Dir::East,
        Dir::West,
        Dir::NorthEast,
        Dir::NorthWest,
        Dir::SouthEast,
        Dir::SouthWest,
    ];

    pub fn delta(self) -> (Coord, Coord) {
        match self {
            Dir::North => (0, -1),
            Dir::South => (0, 1),
            Dir::East => (1, 0),
            Dir::West => (-1, 0),
            Dir::NorthEast => (1, -1),
            Dir::NorthWest => (-1, -1),
            Dir::SouthEast => (1, 1),
            Dir::SouthWest => (-1, 1),
        }
    }

    /// Direction matching a pair of signums (each in {-1, 0, 1}), if any.
    pub fn from_signums(dx: Coord, dy: Coord) -> Option<Dir> {
        match (dx.signum(), dy.signum()) {
            (0, -1) => Some(Dir::North),
            (0, 1) => Some(Dir::South),
            (1, 0) => Some(Dir::East),
            (-1, 0) => Some(Dir::West),
            (1, -1) => Some(Dir::NorthEast),
            (-1, -1) => Some(Dir::NorthWest),
            (1, 1) => Some(Dir::SouthEast),
            (-1, 1) => Some(Dir::SouthWest),
            _ => None,
        }
    }
}
