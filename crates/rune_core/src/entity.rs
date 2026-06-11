//! Actors in the dungeon: the player and the monsters. Entities are stored in
//! a flat `Vec` in [`crate::world::World`]; an [`EntityId`] is just its index,
//! and entities are never removed (the dead keep `alive = false`) so ids stay
//! stable for the lifetime of a level.

use crate::geometry::Point;

pub type EntityId = usize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Faction {
    Player,
    Monster,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiKind {
    /// Walks toward the player and bumps to attack when adjacent.
    Chaser,
}

#[derive(Debug, Clone)]
pub struct Entity {
    pub id: EntityId,
    pub name: String,
    pub glyph: char,
    pub pos: Point,
    pub hp: i32,
    pub max_hp: i32,
    pub power: i32,
    pub mana: i32,
    pub max_mana: i32,
    pub faction: Faction,
    pub ai: Option<AiKind>,
    /// Remaining turns of fire damage-over-time.
    pub burning: u32,
    /// Remaining turns of "acts at half speed".
    pub slowed: u32,
    /// HP regenerated at the end of each turn (trolls; the Bulwark relic).
    pub regen: i32,
    pub alive: bool,
}

impl Entity {
    pub fn is_player(&self) -> bool {
        matches!(self.faction, Faction::Player)
    }

    pub fn take_damage(&mut self, amount: i32) {
        if amount <= 0 {
            return;
        }
        self.hp -= amount;
        if self.hp <= 0 {
            self.hp = 0;
            self.alive = false;
        }
    }

    pub fn heal(&mut self, amount: i32) {
        if self.alive && amount > 0 {
            self.hp = (self.hp + amount).min(self.max_hp);
        }
    }
}
