//! # rune_core
//!
//! Engine-agnostic game logic for **Runecaster**, a rune-grammar roguelike.
//! Nothing in this crate depends on Bevy (or any renderer); it is a pure,
//! deterministic simulation that the presentation layer drives and observes.
//!
//! The headline system lives in [`runes`]: the player collects runes and
//! composes them into an ordered "spell word", which [`runes::compile`] turns
//! into a concrete, castable [`runes::Spell`]. Combining a self-targeting form
//! with a damaging element backfires onto the caster — the emergent "mistake
//! you make once".

#![allow(dead_code)]

pub mod entity;
pub mod fov;
pub mod geometry;
pub mod map;
pub mod runes;
pub mod world;

pub use entity::{AiKind, Entity, EntityId, Faction};
pub use geometry::{Dir, Point};
pub use map::{Map, Rect, Tile};
pub use runes::{compile, ElementKind, Form, Rune, RuneCategory, Spell, SpellError, MAX_RUNES};
pub use world::{monster_for, CastFx, Fx, FxKind, MonsterProto, Relic, RunState, World, MONSTERS};
