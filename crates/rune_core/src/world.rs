//! The simulation: the full game state plus the rules that advance it one
//! player action at a time. The presentation layer issues commands
//! ([`World::player_move`], [`World::player_cast`], [`World::descend`]), reads
//! the resulting state and [`World::log`], and drains [`World::fx`] for VFX.

use crate::entity::{AiKind, Entity, EntityId, Faction};
use crate::fov::compute_fov;
use crate::geometry::{Dir, Point};
use crate::map::{Map, Rect, Tile};
use crate::runes::{compile, ElementKind, Form, Rune, Spell, SpellError};
use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};
use std::collections::HashSet;

pub const MAP_WIDTH: i32 = 60;
pub const MAP_HEIGHT: i32 = 36;
pub const BASE_FOV_RADIUS: i32 = 8;
pub const MAX_DEPTH: i32 = 5;
const CHAIN_RANGE: i32 = 4;
const BARRIER_TURNS: u32 = 8;
const BURN_DAMAGE: i32 = 2;
/// Mana refunded to the player when a monster dies (from any source) — a
/// counterweight to mana-starved stalling near a punching bag. Harness data
/// (examples/balance.rs) showed ~14 starved turns/run at 2; 3 keeps casting
/// the primary verb without reopening a rest-loop.
const KILL_MANA: i32 = 3;
/// Baseline mana-regen cadence: +1 mana every this many turns.
const MANA_REGEN_INTERVAL: u64 = 6;
/// The Battery relic doesn't add to the per-tick amount; it shrinks the
/// regen interval instead, so its value is "regen ticks twice as often".
const BATTERY_REGEN_INTERVAL: u64 = 3;
/// Monsters wake within this radius regardless of the player's own sight
/// range. Decoupling wake distance from `fov_radius` means a vision relic
/// (Farsight) can never backfire by also waking monsters farther away.
const WAKE_RADIUS: i32 = BASE_FOV_RADIUS;
/// Descending heals `max_hp / this` and refills mana (the floor "breather").
const DESCEND_HEAL_DIVISOR: i32 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunState {
    Playing,
    Won,
    Dead,
}

/// A permanent, run-long passive granted by a floor pickup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relic {
    Vitality,
    Focus,
    Farsight,
    Emberbrand,
    Leechstone,
    Battery,
    Bulwark,
}

impl Relic {
    pub const ALL: [Relic; 7] = [
        Relic::Vitality,
        Relic::Focus,
        Relic::Farsight,
        Relic::Emberbrand,
        Relic::Leechstone,
        Relic::Battery,
        Relic::Bulwark,
    ];

    // Relic magnitudes. `apply_relic` and `effect_desc` both read these, so
    // the shown text can never drift from what the relic actually does.
    pub const VITALITY_HP: i32 = 10;
    pub const FOCUS_MANA: i32 = 6;
    pub const FARSIGHT_RADIUS: i32 = 2;
    pub const EMBER_BURN: u32 = 3;
    pub const LEECH_HEAL: i32 = 2;
    pub const BATTERY_REGEN: i32 = 1;
    pub const BULWARK_REGEN: i32 = 1;

    pub fn name(self) -> &'static str {
        match self {
            Relic::Vitality => "Heart of Vitality",
            Relic::Focus => "Lens of Focus",
            Relic::Farsight => "Eye of Farsight",
            Relic::Emberbrand => "Emberbrand",
            Relic::Leechstone => "Leechstone",
            Relic::Battery => "Runic Battery",
            Relic::Bulwark => "Bulwark Sigil",
        }
    }

    pub fn effect_desc(self) -> String {
        match self {
            Relic::Vitality => format!("+{} max HP", Self::VITALITY_HP),
            Relic::Focus => format!("+{} max mana", Self::FOCUS_MANA),
            Relic::Farsight => format!("+{} sight radius", Self::FARSIGHT_RADIUS),
            Relic::Emberbrand => format!("melee ignites ({} turns)", Self::EMBER_BURN),
            Relic::Leechstone => format!("melee heals you for {}", Self::LEECH_HEAL),
            Relic::Battery => format!(
                "mana regen every {} turns (base {})",
                BATTERY_REGEN_INTERVAL, MANA_REGEN_INTERVAL
            ),
            Relic::Bulwark => format!("regenerate {} HP each turn", Self::BULWARK_REGEN),
        }
    }
}

/// A transient visual event for the renderer to consume. Carrying a tiny bit
/// of presentation *intent* as plain data keeps `World` testable while letting
/// the front-end show particles without re-deriving what happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FxKind {
    Hit,
    Heal,
    Death,
    Teleport,
    Barrier,
}

#[derive(Debug, Clone, Copy)]
pub struct Fx {
    pub pos: Point,
    pub color: [u8; 3],
    pub kind: FxKind,
    /// The element behind this event, when there is one, so the renderer can
    /// style the burst (fire embers vs frost shards vs …). `None` = physical.
    pub element: Option<ElementKind>,
    /// Magnitude for damage / heal events (0 = not applicable), so the
    /// renderer can float combat numbers without re-deriving the sim.
    pub amount: i32,
}

impl Fx {
    fn at(pos: Point, color: [u8; 3], kind: FxKind) -> Self {
        Self {
            pos,
            color,
            kind,
            element: None,
            amount: 0,
        }
    }

    fn styled(pos: Point, kind: FxKind, element: ElementKind) -> Self {
        Self {
            pos,
            color: element.color(),
            kind,
            element: Some(element),
            amount: 0,
        }
    }

    /// Attach a combat magnitude (damage dealt or hit points healed).
    fn amount(mut self, n: i32) -> Self {
        self.amount = n;
        self
    }
}

/// A spell's signature motion — projectile flight, drawn beam, expanding nova,
/// or self-aura — handed to the renderer to animate. Carrying form + element +
/// trajectory keeps the simulation testable while enabling rich skill effects.
#[derive(Debug, Clone, Copy)]
pub struct CastFx {
    pub form: Form,
    pub element: ElementKind,
    pub origin: Point,
    pub target: Point,
    pub radius: i32,
}

pub struct World {
    pub map: Map,
    pub entities: Vec<Entity>,
    pub player_id: EntityId,
    pub rng: StdRng,
    /// Runes the player has learned and may compose with.
    pub known_runes: Vec<Rune>,
    /// Unclaimed runes lying on the floor: `(tile, rune)`.
    pub rune_items: Vec<(Point, Rune)>,
    /// Unclaimed relics lying on the floor: `(tile, relic)`.
    pub relic_items: Vec<(Point, Relic)>,
    /// Relics the player has collected this run.
    pub relics: Vec<Relic>,
    /// Conjured walls: `(tile, turns_remaining, original_tile)`.
    pub temp_walls: Vec<(Point, u32, Tile)>,
    pub log: Vec<String>,
    /// Point visual events accumulated this step; the renderer drains them.
    pub fx: Vec<Fx>,
    /// Spell-cast trajectory events this step; the renderer drains them.
    pub cast_fx: Vec<CastFx>,
    pub depth: i32,
    pub turn: u64,
    pub kills: u32,
    pub state: RunState,
    pub fov_radius: i32,
    // Player run-modifiers (granted by relics).
    pub melee_lifesteal: i32,
    pub melee_ignite: bool,
    pub mana_regen_bonus: i32,
    pub visible: Vec<bool>,
    pub explored: Vec<bool>,
}

impl World {
    pub fn new(seed: u64) -> Self {
        let mut rng = StdRng::seed_from_u64(seed);
        let (map, start) = Map::generate(MAP_WIDTH, MAP_HEIGHT, &mut rng);
        let tiles = (map.width * map.height) as usize;

        let player = Entity {
            id: 0,
            name: "you".into(),
            glyph: '@',
            pos: start,
            hp: 30,
            max_hp: 30,
            power: 4,
            mana: 12,
            max_mana: 12,
            faction: Faction::Player,
            ai: None,
            burning: 0,
            slowed: 0,
            regen: 0,
            alive: true,
        };

        let mut world = World {
            map,
            entities: vec![player],
            player_id: 0,
            rng,
            // A starter kit that teaches both forms and the backfire lesson.
            known_runes: vec![Rune::Bolt, Rune::Fire, Rune::OnSelf, Rune::Heal],
            rune_items: Vec::new(),
            relic_items: Vec::new(),
            relics: Vec::new(),
            temp_walls: Vec::new(),
            log: Vec::new(),
            fx: Vec::new(),
            cast_fx: Vec::new(),
            depth: 1,
            turn: 0,
            kills: 0,
            state: RunState::Playing,
            fov_radius: BASE_FOV_RADIUS,
            melee_lifesteal: 0,
            melee_ignite: false,
            mana_regen_bonus: 0,
            visible: vec![false; tiles],
            explored: vec![false; tiles],
        };

        world.populate_level();
        world.refresh_fov();
        world
            .log
            .push("You enter the Glyph Vaults. Weave runes, or be unmade.".into());
        world
    }

    // --- queries ---------------------------------------------------------

    pub fn player(&self) -> &Entity {
        &self.entities[self.player_id]
    }

    fn player_mut(&mut self) -> &mut Entity {
        &mut self.entities[self.player_id]
    }

    pub fn entity(&self, id: EntityId) -> &Entity {
        &self.entities[id]
    }

    pub fn blocking_entity_at(&self, p: Point) -> Option<EntityId> {
        self.entities
            .iter()
            .find(|e| e.alive && e.pos == p)
            .map(|e| e.id)
    }

    pub fn is_visible(&self, p: Point) -> bool {
        self.map.in_bounds(p) && self.visible[self.map.idx(p)]
    }

    pub fn is_explored(&self, p: Point) -> bool {
        self.map.in_bounds(p) && self.explored[self.map.idx(p)]
    }

    pub fn is_temp_wall(&self, p: Point) -> bool {
        self.temp_walls.iter().any(|(tp, _, _)| *tp == p)
    }

    /// Preview what a rune word would compile to (for live UI feedback).
    pub fn preview(&self, words: &[Rune]) -> Result<Spell, SpellError> {
        compile(words)
    }

    /// Read-only aim preview for the UI: the tiles `spell` would affect if
    /// the player cast it right now along `aim`, plus the extra enemies its
    /// chain/arc would jump to. Mirrors `resolve_spell`'s targeting exactly
    /// — same `affected_tiles`, same hit ordering, same "a miss has nothing
    /// to arc from" seeding, same `CHAIN_RANGE` — without mutating anything,
    /// so a renderer can promise "the highlighted tiles are what gets hit"
    /// and be right.
    pub fn preview_affected(&self, spell: &Spell, aim: Dir) -> (Vec<Point>, Vec<Point>) {
        let origin = self.player().pos;
        let tiles = self.affected_tiles(origin, spell, aim);

        let mut chain_targets = Vec::new();
        let arcs = spell.chain + if spell.arcs() { 1 } else { 0 };
        if arcs > 0 && spell.is_damaging() {
            // Direct hits in tile order; the last one seeds the arcs.
            let mut hitset: HashSet<EntityId> = HashSet::new();
            let mut last_hit: Option<EntityId> = None;
            for &t in &tiles {
                for e in &self.entities {
                    if e.alive && e.pos == t && hitset.insert(e.id) {
                        last_hit = Some(e.id);
                    }
                }
            }
            if let Some(last) = last_hit {
                let mut from = self.entities[last].pos;
                for _ in 0..arcs {
                    if let Some(next) = self.nearest_unhit_enemy(from, &hitset, CHAIN_RANGE) {
                        hitset.insert(next);
                        let p = self.entities[next].pos;
                        chain_targets.push(p);
                        from = p;
                    } else {
                        break;
                    }
                }
            }
        }
        (tiles, chain_targets)
    }

    /// Run score; rewards depth, kills, and relics, with a victory bonus.
    pub fn score(&self) -> u32 {
        let base = self.depth as u32 * 120 + self.kills * 15 + self.relics.len() as u32 * 40;
        if self.state == RunState::Won {
            base + 500
        } else {
            base
        }
    }

    // --- player commands -------------------------------------------------

    pub fn player_move(&mut self, dir: Dir) {
        if self.state != RunState::Playing {
            return;
        }
        if self.consume_player_slow() {
            return;
        }
        let dest = self.player().pos.step(dir);

        if let Some(target) = self.blocking_entity_at(dest) {
            if target != self.player_id {
                self.melee(self.player_id, target);
                self.end_turn();
            }
            return;
        }

        if self.map.is_walkable(dest) {
            self.player_mut().pos = dest;
            self.pickup_here();
            self.end_turn();
        }
    }

    pub fn player_wait(&mut self) {
        if self.state == RunState::Playing {
            if self.consume_player_slow() {
                return;
            }
            self.end_turn();
        }
    }

    /// Compile and cast `words`, aiming `aim` (ignored by non-directional
    /// forms). Returns the grammar error if it doesn't compile. `Ok(true)`
    /// means the spell actually resolved (turn spent); `Ok(false)` means no
    /// spell happened — insufficient mana (logged, no turn) or the action was
    /// swallowed by slow (turn passes) — so the caller should keep the
    /// composed word for another attempt.
    pub fn player_cast(&mut self, words: &[Rune], aim: Dir) -> Result<bool, SpellError> {
        if self.state != RunState::Playing {
            return Ok(false);
        }
        let spell = compile(words)?;
        if self.consume_player_slow() {
            return Ok(false);
        }
        if self.player().mana < spell.mana_cost {
            self.log.push(format!(
                "Not enough mana for {} ({} / {}).",
                spell.describe(),
                self.player().mana,
                spell.mana_cost
            ));
            return Ok(false);
        }
        self.player_mut().mana -= spell.mana_cost;
        self.resolve_spell(self.player_id, &spell, aim);
        self.end_turn();
        Ok(true)
    }

    /// If the player is slowed, one stack is consumed and the attempted action
    /// is lost (the world still advances a turn). Returns true if it was eaten.
    fn consume_player_slow(&mut self) -> bool {
        if self.entities[self.player_id].slowed == 0 {
            return false;
        }
        self.entities[self.player_id].slowed -= 1;
        self.log.push("You are slowed and lose your action!".into());
        self.end_turn();
        true
    }

    pub fn descend(&mut self) {
        if self.state != RunState::Playing {
            return;
        }
        if self.map.tile(self.player().pos) != Tile::StairsDown {
            self.log.push("There are no stairs here.".into());
            return;
        }
        if self.depth >= MAX_DEPTH {
            self.state = RunState::Won;
            self.log
                .push("You climb into daylight, runes in hand. You win!".into());
            return;
        }

        self.depth += 1;
        let (map, start) = Map::generate(MAP_WIDTH, MAP_HEIGHT, &mut self.rng);
        self.map = map;

        let mut player = self.entities[self.player_id].clone();
        player.id = 0;
        player.pos = start;
        player.burning = 0;
        player.slowed = 0;
        // Crossing floors is a breather: mend a quarter of max HP and arrive
        // with a full mana pool. Each floor is its own mana budget (kills
        // fund the rest mid-floor), so the pressure stays on without there
        // ever being a profitable rest-loop.
        player.hp = (player.hp + player.max_hp / DESCEND_HEAL_DIVISOR).min(player.max_hp);
        player.mana = player.max_mana;
        self.entities = vec![player];
        self.player_id = 0;
        self.rune_items.clear();
        self.relic_items.clear();
        self.temp_walls.clear();

        let tiles = (self.map.width * self.map.height) as usize;
        self.visible = vec![false; tiles];
        self.explored = vec![false; tiles];

        self.populate_level();
        self.refresh_fov();
        self.log
            .push(format!("You descend to depth {}.", self.depth));
    }

    // --- level setup -----------------------------------------------------

    fn populate_level(&mut self) {
        let depth = self.depth;
        let rooms = self.map.rooms.clone();

        for room in rooms.iter().skip(1) {
            // Density ramps with depth, but a step later than it used to:
            // harness runs showed the old `depth / 2` bonus made d2 and d4
            // attrition walls (d4 death rate 92%).
            let count = self.rng.random_range(0..=2) + (depth - 1) / 2;
            for _ in 0..count {
                if let Some(p) = self.random_floor_in(room) {
                    if self.blocking_entity_at(p).is_none() {
                        self.spawn_monster(p, depth);
                    }
                }
            }
        }

        // Scatter runes to find.
        let pool = rune_drop_pool();
        let drops = 3 + depth;
        for _ in 0..drops {
            let ri = self.rng.random_range(0..rooms.len());
            if let Some(p) = self.random_floor_in(&rooms[ri]) {
                let rune = pool[self.rng.random_range(0..pool.len())];
                self.rune_items.push((p, rune));
            }
        }

        // A relic appears on most levels (rarer than runes).
        if self.rng.random_bool(0.7) && rooms.len() > 1 {
            let ri = self.rng.random_range(1..rooms.len());
            if let Some(p) = self.random_floor_in(&rooms[ri]) {
                if !self.rune_items.iter().any(|(rp, _)| *rp == p) {
                    let relic = Relic::ALL[self.rng.random_range(0..Relic::ALL.len())];
                    self.relic_items.push((p, relic));
                }
            }
        }
    }

    fn spawn_monster(&mut self, pos: Point, depth: i32) {
        let id = self.entities.len();
        let roll = self.rng.random_range(0..100);
        let proto = monster_for(depth, roll);
        self.entities.push(Entity {
            id,
            name: proto.name.into(),
            glyph: proto.glyph,
            pos,
            hp: proto.hp,
            max_hp: proto.hp,
            power: proto.power,
            mana: 0,
            max_mana: 0,
            faction: Faction::Monster,
            ai: Some(AiKind::Chaser),
            burning: 0,
            slowed: 0,
            regen: proto.regen,
            alive: true,
        });
    }

    fn random_floor_in(&mut self, room: &Rect) -> Option<Point> {
        for _ in 0..20 {
            let x = self
                .rng
                .random_range((room.x1 + 1)..room.x2.max(room.x1 + 2));
            let y = self
                .rng
                .random_range((room.y1 + 1)..room.y2.max(room.y1 + 2));
            let p = Point::new(x, y);
            if self.map.tile(p) == Tile::Floor {
                return Some(p);
            }
        }
        None
    }

    // --- turn resolution -------------------------------------------------

    fn pickup_here(&mut self) {
        let pos = self.player().pos;

        if let Some(i) = self.rune_items.iter().position(|(p, _)| *p == pos) {
            let (_, rune) = self.rune_items.remove(i);
            if self.known_runes.contains(&rune) {
                let p = self.player_mut();
                p.mana = (p.mana + 3).min(p.max_mana);
                self.log
                    .push(format!("A spare {} rune dissolves into mana.", rune.name()));
            } else {
                self.known_runes.push(rune);
                self.log
                    .push(format!("You learn the {} rune!", rune.name()));
            }
        }

        if let Some(i) = self.relic_items.iter().position(|(p, _)| *p == pos) {
            let (_, relic) = self.relic_items.remove(i);
            self.apply_relic(relic);
        }
    }

    fn apply_relic(&mut self, relic: Relic) {
        self.relics.push(relic);
        match relic {
            Relic::Vitality => {
                let p = self.player_mut();
                p.max_hp += Relic::VITALITY_HP;
                p.hp += Relic::VITALITY_HP;
            }
            Relic::Focus => {
                let p = self.player_mut();
                p.max_mana += Relic::FOCUS_MANA;
                p.mana += Relic::FOCUS_MANA;
            }
            Relic::Farsight => self.fov_radius += Relic::FARSIGHT_RADIUS,
            Relic::Emberbrand => self.melee_ignite = true,
            Relic::Leechstone => self.melee_lifesteal += Relic::LEECH_HEAL,
            Relic::Battery => self.mana_regen_bonus += Relic::BATTERY_REGEN,
            Relic::Bulwark => self.player_mut().regen += Relic::BULWARK_REGEN,
        }
        self.log.push(format!(
            "Relic found: {} — {}.",
            relic.name(),
            relic.effect_desc()
        ));
        self.fx
            .push(Fx::at(self.player().pos, [255, 220, 120], FxKind::Heal));
    }

    fn melee(&mut self, attacker: EntityId, defender: EntityId) {
        let power = self.entities[attacker].power;
        let atk_is_player = attacker == self.player_id;
        let def_is_player = defender == self.player_id;
        let aname = self.entities[attacker].name.clone();
        let dname = self.entities[defender].name.clone();
        let dpos = self.entities[defender].pos;

        self.entities[defender].take_damage(power);
        let died = !self.entities[defender].alive;

        self.fx
            .push(Fx::at(dpos, [235, 80, 70], FxKind::Hit).amount(power));

        // Player melee relics. Lifesteal also procs on a killing blow; ignite
        // only matters on a survivor.
        if atk_is_player {
            if !died && self.melee_ignite {
                self.entities[defender].burning += Relic::EMBER_BURN;
            }
            if self.melee_lifesteal > 0 {
                let steal = self.melee_lifesteal;
                self.player_mut().heal(steal);
            }
        }

        let subj = if atk_is_player {
            "You".to_string()
        } else {
            cap(&aname)
        };
        let verb = if atk_is_player { "hit" } else { "hits" };
        let obj = if def_is_player {
            "you".to_string()
        } else {
            dname
        };
        let tail = if died {
            if def_is_player {
                " — you fall"
            } else {
                " — slain"
            }
        } else {
            ""
        };
        self.log
            .push(format!("{} {} {} for {}{}.", subj, verb, obj, power, tail));

        if died {
            self.register_death(defender);
        }
    }

    fn resolve_spell(&mut self, caster_id: EntityId, spell: &Spell, aim: Dir) {
        let origin = self.entities[caster_id].pos;
        let caster_is_player = caster_id == self.player_id;

        let verb = if caster_is_player { "cast" } else { "casts" };
        let subj = if caster_is_player {
            "You".to_string()
        } else {
            cap(&self.entities[caster_id].name)
        };
        self.log
            .push(format!("{} {} {}.", subj, verb, spell.describe()));
        if spell.hits_caster {
            self.log.push("The magic recoils into you!".into());
        }

        let tiles = self.affected_tiles(origin, spell, aim);
        let primary = spell.primary_element();
        let target = match spell.form {
            Form::Beam => tiles.last().copied().unwrap_or(origin),
            Form::Nova | Form::OnSelf => origin,
            _ => tiles.first().copied().unwrap_or(origin.step(aim)),
        };
        self.cast_fx.push(CastFx {
            form: spell.form,
            element: primary,
            origin,
            target,
            radius: spell.radius,
        });

        // Direct hits on affected tiles.
        let mut hit: Vec<EntityId> = Vec::new();
        let mut hitset: HashSet<EntityId> = HashSet::new();
        for &t in &tiles {
            for e in &self.entities {
                if e.alive && e.pos == t && hitset.insert(e.id) {
                    hit.push(e.id);
                }
            }
        }

        // Chain: arc to nearby fresh enemies. Lightning arcs to one enemy
        // even without a Chain rune — its signature rider — and Chain runes
        // stack additional arcs on top. A miss (no direct hit) has nothing
        // to arc from, so it must not seed the search from the caster's own
        // position — no target means no chain.
        let arcs = spell.chain + if spell.arcs() { 1 } else { 0 };
        if arcs > 0 && spell.is_damaging() {
            if let Some(&last) = hit.last() {
                let mut from = self.entities[last].pos;
                for _ in 0..arcs {
                    if let Some(next) = self.nearest_unhit_enemy(from, &hitset, CHAIN_RANGE) {
                        hitset.insert(next);
                        hit.push(next);
                        let next_pos = self.entities[next].pos;
                        self.cast_fx.push(CastFx {
                            form: Form::Beam,
                            element: primary,
                            origin: from,
                            target: next_pos,
                            radius: 0,
                        });
                        from = next_pos;
                    } else {
                        break;
                    }
                }
            }
        }

        let mut enemies_damaged = 0;
        for id in hit {
            let (damaged, is_enemy) = self.apply_spell_to(id, spell, caster_id);
            if damaged && is_enemy {
                enemies_damaged += 1;
            }
        }

        // Drain: lifesteal back to the caster.
        if spell.drains() && enemies_damaged > 0 {
            let gain = (spell.power / 2).max(1) * enemies_damaged;
            self.entities[caster_id].heal(gain);
            let cpos = self.entities[caster_id].pos;
            self.fx
                .push(Fx::at(cpos, [120, 255, 150], FxKind::Heal).amount(gain));
            if caster_is_player {
                self.log.push(format!("You drain {} life.", gain));
            }
        }

        // Teleport: blink the caster.
        if spell.teleports() {
            if let Some(dest) = self.teleport_dest(origin, spell, aim) {
                self.fx
                    .push(Fx::at(origin, [150, 230, 255], FxKind::Teleport));
                self.entities[caster_id].pos = dest;
                self.fx
                    .push(Fx::at(dest, [150, 230, 255], FxKind::Teleport));
                if caster_is_player {
                    self.log.push("You blink through space.".into());
                }
            }
        }

        // Barrier: conjure temporary walls on the affected tiles.
        if spell.has_barrier() {
            for t in tiles {
                self.place_temp_wall(t, BARRIER_TURNS);
            }
        }
    }

    /// Which tiles a spell affects, given the caster position and aim.
    fn affected_tiles(&self, origin: Point, spell: &Spell, aim: Dir) -> Vec<Point> {
        let mut tiles = Vec::new();
        match spell.form {
            Form::Bolt => {
                let mut p = origin;
                for _ in 0..spell.range {
                    p = p.step(aim);
                    if self.map.is_opaque(p) {
                        return tiles;
                    }
                    if self.blocking_entity_at(p).is_some() {
                        tiles.push(p);
                        self.add_disc(&mut tiles, p, spell.radius);
                        return tiles;
                    }
                }
                if !self.map.is_opaque(p) {
                    tiles.push(p);
                    self.add_disc(&mut tiles, p, spell.radius);
                }
            }
            Form::Beam => {
                let mut p = origin;
                for _ in 0..spell.range {
                    p = p.step(aim);
                    if self.map.is_opaque(p) {
                        break; // stopped by a wall, but pierces everything else
                    }
                    tiles.push(p);
                }
            }
            Form::Nova => {
                self.add_disc(&mut tiles, origin, spell.radius);
                tiles.retain(|&t| t != origin);
            }
            Form::OnSelf => {
                tiles.push(origin);
                self.add_disc(&mut tiles, origin, spell.radius);
            }
            Form::Touch => {
                let p = origin.step(aim);
                if !self.map.is_opaque(p) {
                    tiles.push(p);
                }
            }
        }
        tiles
    }

    /// The tile a teleporting spell drops the caster on: the farthest open,
    /// unoccupied tile along the aim, up to range.
    fn teleport_dest(&self, origin: Point, spell: &Spell, aim: Dir) -> Option<Point> {
        let range = match spell.form {
            Form::Touch => 1,
            _ => spell.range,
        };
        let mut dest = None;
        let mut p = origin;
        for _ in 0..range {
            p = p.step(aim);
            if self.map.is_opaque(p) {
                break;
            }
            if self.blocking_entity_at(p).is_none() {
                dest = Some(p);
            }
        }
        dest
    }

    fn add_disc(&self, tiles: &mut Vec<Point>, center: Point, radius: i32) {
        if radius <= 0 {
            return;
        }
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                let p = center.offset(dx, dy);
                if center.chebyshev(p) <= radius
                    && self.map.in_bounds(p)
                    && !self.map.is_opaque(p)
                    && !tiles.contains(&p)
                {
                    tiles.push(p);
                }
            }
        }
    }

    fn nearest_unhit_enemy(
        &self,
        from: Point,
        exclude: &HashSet<EntityId>,
        max: i32,
    ) -> Option<EntityId> {
        let mut best: Option<(EntityId, i32)> = None;
        for e in &self.entities {
            if !e.alive || e.faction != Faction::Monster || exclude.contains(&e.id) {
                continue;
            }
            let d = from.chebyshev(e.pos);
            if d <= max && best.is_none_or(|(_, bd)| d < bd) {
                best = Some((e.id, d));
            }
        }
        best.map(|(id, _)| id)
    }

    /// Apply a spell's elements to one entity. Returns `(damaged, is_enemy)`.
    /// Healing only lands on the caster's own side (a Nova of Heal must not mend your enemies).
    fn apply_spell_to(&mut self, id: EntityId, spell: &Spell, caster_id: EntityId) -> (bool, bool) {
        if !self.entities[id].alive {
            return (false, false);
        }
        let is_player = id == self.player_id;
        let is_enemy = self.entities[id].faction == Faction::Monster;
        let name = self.entities[id].name.clone();
        let pos = self.entities[id].pos;
        let same_side = self.entities[id].faction == self.entities[caster_id].faction;

        let mut damage = 0;
        let mut healed = 0;
        let mut burn = 0u32;
        let mut slow = 0u32;
        for &elem in &spell.elements {
            match elem {
                ElementKind::Heal => {
                    if same_side {
                        healed += spell.heal_amount();
                    }
                }
                ElementKind::Fire => {
                    damage += spell.power;
                    burn += 3;
                }
                ElementKind::Frost => {
                    damage += spell.power;
                    slow += 2;
                }
                ElementKind::Lightning => damage += spell.power,
                ElementKind::Drain => damage += spell.power,
                ElementKind::Force => damage += spell.power / 2 + 1,
                ElementKind::Teleport | ElementKind::Barrier => {} // handled at spell level
            }
        }

        {
            let e = &mut self.entities[id];
            e.heal(healed);
            e.burning += burn;
            e.slowed += slow;
            e.take_damage(damage);
        }
        let died = !self.entities[id].alive;

        if healed > 0 {
            self.fx
                .push(Fx::at(pos, [120, 255, 150], FxKind::Heal).amount(healed));
            if is_player {
                self.log.push(format!("You are healed for {}.", healed));
            } else {
                self.log
                    .push(format!("{} is healed for {}.", cap(&name), healed));
            }
        } else if spell.heals() && !same_side && !spell.is_damaging() {
            // A pure heal landing on a hostile target is a deliberate no-op
            // (mending never aids the other side); say so instead of letting
            // the cast look like a silent bug.
            self.log
                .push(format!("{} cannot be mended by this magic.", cap(&name)));
        }
        if damage > 0 {
            self.fx
                .push(Fx::styled(pos, FxKind::Hit, spell.primary_element()).amount(damage));
            if is_player {
                self.log.push(format!(
                    "You take {} damage{}.",
                    damage,
                    if died { " and collapse" } else { "" }
                ));
            } else {
                self.log.push(format!(
                    "{} takes {} damage{}.",
                    cap(&name),
                    damage,
                    if died { " and falls" } else { "" }
                ));
            }
        }
        if died {
            self.register_death(id);
        }

        // Force's signature rider: shove a surviving target one tile away
        // from the caster. Self+Force has caster == target, so the signum of
        // a zero delta yields `None` and it naturally never shoves itself.
        if spell.pushes() && self.entities[id].alive {
            let cpos = self.entities[caster_id].pos;
            let tpos = self.entities[id].pos;
            if let Some(dir) =
                Dir::from_signums((tpos.x - cpos.x).signum(), (tpos.y - cpos.y).signum())
            {
                let dest = tpos.step(dir);
                if self.map.is_walkable(dest) && self.blocking_entity_at(dest).is_none() {
                    self.entities[id].pos = dest;
                }
            }
        }

        (damage > 0, is_enemy)
    }

    /// Bookkeeping when any entity dies: score a kill for monsters (and
    /// refund the player some mana for it), emit a death burst.
    fn register_death(&mut self, id: EntityId) {
        if self.entities[id].faction == Faction::Monster {
            self.kills += 1;
            if self.entities[self.player_id].alive {
                let p = self.player_mut();
                p.mana = (p.mana + KILL_MANA).min(p.max_mana);
            }
        }
        self.fx
            .push(Fx::at(self.entities[id].pos, [185, 45, 45], FxKind::Death));
    }

    fn place_temp_wall(&mut self, p: Point, turns: u32) {
        if self.map.tile(p) == Tile::Floor
            && self.blocking_entity_at(p).is_none()
            && p != self.player().pos
            && !self.is_temp_wall(p)
        {
            let orig = self.map.tile(p);
            self.map.set(p, Tile::Wall);
            self.temp_walls.push((p, turns, orig));
            self.fx.push(Fx::at(p, [140, 175, 230], FxKind::Barrier));
        }
    }

    fn tick_temp_walls(&mut self) {
        let mut i = 0;
        while i < self.temp_walls.len() {
            self.temp_walls[i].1 = self.temp_walls[i].1.saturating_sub(1);
            if self.temp_walls[i].1 == 0 {
                let (p, _, orig) = self.temp_walls[i];
                self.map.set(p, orig);
                self.temp_walls.swap_remove(i);
            } else {
                i += 1;
            }
        }
    }

    fn end_turn(&mut self) {
        self.tick_status(self.player_id);
        if self.entities[self.player_id].alive {
            self.monster_turns();
        }
        self.apply_regen();
        self.tick_temp_walls();
        self.turn += 1;

        // Mana regeneration: +1 every MANA_REGEN_INTERVAL turns, or every
        // BATTERY_REGEN_INTERVAL turns with the Battery relic. Battery's
        // value is a shorter interval, not a bigger per-tick amount.
        let interval = if self.mana_regen_bonus > 0 {
            BATTERY_REGEN_INTERVAL
        } else {
            MANA_REGEN_INTERVAL
        };
        if self.turn.is_multiple_of(interval) && self.entities[self.player_id].alive {
            let p = self.player_mut();
            p.mana = (p.mana + 1).min(p.max_mana);
        }

        self.refresh_fov();

        if !self.entities[self.player_id].alive && self.state == RunState::Playing {
            self.state = RunState::Dead;
            self.log
                .push("You are unmade. The vaults keep your runes.".into());
        }
    }

    fn monster_turns(&mut self) {
        let ids: Vec<EntityId> = self
            .entities
            .iter()
            .filter(|e| e.alive && e.faction == Faction::Monster)
            .map(|e| e.id)
            .collect();
        let player_pos = self.entities[self.player_id].pos;

        for id in ids {
            if !self.entities[id].alive {
                continue;
            }
            // Status effects tick even on brainless actors...
            self.tick_status(id);
            if !self.entities[id].alive || !self.entities[self.player_id].alive {
                continue;
            }
            // ...but only an actor with an AI takes an action at all.
            let Some(ai) = self.entities[id].ai else {
                continue;
            };

            let mpos = self.entities[id].pos;
            let dist = mpos.chebyshev(player_pos);
            if dist > WAKE_RADIUS {
                // Wake distance is fixed, independent of `fov_radius`: a
                // relic that extends the player's own sight (Farsight) must
                // not also wake monsters farther away as a side effect.
                continue;
            }

            // Slow is only spent in place of an action it would otherwise
            // take; dormant monsters keep their stacks until they wake.
            if self.entities[id].slowed > 0 {
                self.entities[id].slowed -= 1;
                continue;
            }

            match ai {
                AiKind::Chaser => {
                    if dist == 1 {
                        self.melee(id, self.player_id);
                    } else {
                        for dir in move_candidates(mpos, player_pos) {
                            let dest = mpos.step(dir);
                            if self.map.is_walkable(dest) && self.blocking_entity_at(dest).is_none()
                            {
                                self.entities[id].pos = dest;
                                break;
                            }
                        }
                    }
                }
            }
        }
    }

    fn tick_status(&mut self, id: EntityId) {
        if !self.entities[id].alive || self.entities[id].burning == 0 {
            return;
        }
        let is_player = id == self.player_id;
        let name = self.entities[id].name.clone();
        let pos = self.entities[id].pos;
        {
            let e = &mut self.entities[id];
            e.burning -= 1;
            e.take_damage(BURN_DAMAGE);
        }
        let died = !self.entities[id].alive;
        self.fx
            .push(Fx::styled(pos, FxKind::Hit, ElementKind::Fire).amount(BURN_DAMAGE));
        if is_player {
            self.log.push(format!(
                "You burn for {}{}.",
                BURN_DAMAGE,
                if died { " and collapse" } else { "" }
            ));
        } else {
            self.log.push(format!(
                "{} burns for {}{}.",
                cap(&name),
                BURN_DAMAGE,
                if died { " and is consumed" } else { "" }
            ));
        }
        if died {
            self.register_death(id);
        }
    }

    fn apply_regen(&mut self) {
        for e in &mut self.entities {
            if e.alive && e.regen > 0 && e.hp < e.max_hp {
                e.heal(e.regen);
            }
        }
    }

    fn refresh_fov(&mut self) {
        let origin = self.player().pos;
        self.visible = compute_fov(&self.map, origin, self.fov_radius);
        for (i, &v) in self.visible.iter().enumerate() {
            if v {
                self.explored[i] = true;
            }
        }
    }
}

/// One row of the monster balance table: spawn gate + stat block.
#[derive(Debug, Clone, Copy)]
pub struct MonsterProto {
    pub name: &'static str,
    pub glyph: char,
    pub hp: i32,
    pub power: i32,
    pub regen: i32,
    /// Minimum dungeon depth at which this monster can appear.
    pub min_depth: i32,
    /// Row matches while the d100 spawn roll is below this bound.
    pub max_roll: i32,
}

/// The monster balance table. Rows are checked top to bottom: the first row
/// whose depth gate and roll bound both match wins; the last row (`rat`,
/// `max_roll` 100) is the guaranteed fallback. Adding a monster = adding a
/// row here; nothing else in the spawn path needs to change.
pub const MONSTERS: &[MonsterProto] = &[
    MonsterProto {
        name: "troll",
        glyph: 'T',
        hp: 26,
        power: 6,
        regen: 2,
        min_depth: 4,
        max_roll: 15,
    },
    MonsterProto {
        name: "ogre",
        glyph: 'O',
        hp: 18,
        power: 6,
        regen: 0,
        min_depth: 3,
        max_roll: 35,
    },
    MonsterProto {
        name: "wraith",
        glyph: 'w',
        hp: 12,
        power: 5,
        regen: 0,
        min_depth: 2,
        max_roll: 55,
    },
    MonsterProto {
        name: "goblin",
        glyph: 'g',
        hp: 8,
        power: 3,
        regen: 0,
        min_depth: 1,
        max_roll: 72,
    },
    MonsterProto {
        name: "bat",
        glyph: 'b',
        hp: 4,
        power: 2,
        regen: 0,
        min_depth: 1,
        max_roll: 86,
    },
    MonsterProto {
        name: "rat",
        glyph: 'r',
        hp: 5,
        power: 2,
        regen: 0,
        min_depth: 1,
        max_roll: 100,
    },
];

/// Pure spawn-table lookup: which monster a given depth and d100 roll
/// produce. Deterministic and directly testable, unlike the RNG-driven
/// spawn path that calls it.
pub fn monster_for(depth: i32, roll: i32) -> &'static MonsterProto {
    MONSTERS
        .iter()
        .find(|m| depth >= m.min_depth && roll < m.max_roll)
        .unwrap_or(&MONSTERS[MONSTERS.len() - 1])
}

fn rune_drop_pool() -> Vec<Rune> {
    // Derived from `Rune::ALL` + per-rune weights so a newly added rune can
    // never silently miss the pool (see `Rune::drop_weight`).
    Rune::ALL
        .iter()
        .flat_map(|&r| std::iter::repeat_n(r, r.drop_weight()))
        .collect()
}

fn move_candidates(from: Point, to: Point) -> Vec<Dir> {
    let dx = (to.x - from.x).signum();
    let dy = (to.y - from.y).signum();
    let mut out = Vec::new();
    if let Some(d) = Dir::from_signums(dx, dy) {
        out.push(d);
    }
    if dx != 0 {
        if let Some(d) = Dir::from_signums(dx, 0) {
            out.push(d);
        }
    }
    if dy != 0 {
        if let Some(d) = Dir::from_signums(0, dy) {
            out.push(d);
        }
    }
    out
}

fn cap(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy(id: EntityId, pos: Point, hp: i32) -> Entity {
        Entity {
            id,
            name: "dummy".into(),
            glyph: 'd',
            pos,
            hp,
            max_hp: hp,
            power: 0,
            mana: 0,
            max_mana: 0,
            faction: Faction::Monster,
            ai: None, // inert: never moves or retaliates
            burning: 0,
            slowed: 0,
            regen: 0,
            alive: true,
        }
    }

    #[test]
    fn new_world_places_a_living_player() {
        let w = World::new(1);
        assert!(w.player().alive);
        assert_eq!(w.player().hp, 30);
        assert_eq!(w.map.tile(w.player().pos), Tile::Floor);
        assert_eq!(w.depth, 1);
    }

    #[test]
    fn walking_into_a_wall_costs_no_turn() {
        let mut w = World::new(2);
        let before = w.turn;
        let pos = w.player().pos;
        for dir in Dir::ALL {
            if !w.map.is_walkable(pos.step(dir)) {
                w.player_move(dir);
                assert_eq!(w.player().pos, pos);
                assert_eq!(w.turn, before);
                return;
            }
        }
    }

    #[test]
    fn self_fire_backfires_onto_the_player() {
        let mut w = World::new(3);
        let hp0 = w.player().hp;
        w.player_cast(&[Rune::OnSelf, Rune::Fire], Dir::North)
            .unwrap();
        assert!(w.player().hp < hp0);
    }

    #[test]
    fn self_heal_restores_hp() {
        let mut w = World::new(4);
        w.entities[0].hp = 10;
        w.player_cast(&[Rune::OnSelf, Rune::Heal], Dir::North)
            .unwrap();
        assert!(w.player().hp > 10);
    }

    #[test]
    fn bolt_fire_damages_a_target_in_line() {
        let mut w = World::new(5);
        let tpos = w.player().pos.step(Dir::East);
        w.map.set(tpos, Tile::Floor);
        let id = w.entities.len();
        w.entities.push(dummy(id, tpos, 10));
        w.player_cast(&[Rune::Bolt, Rune::Fire], Dir::East).unwrap();
        assert!(w.entities[id].hp < 10);
    }

    #[test]
    fn beam_pierces_multiple_targets_in_a_row() {
        let mut w = World::new(11);
        let p = w.player().pos;
        let a = p.step(Dir::East);
        let b = a.step(Dir::East);
        w.map.set(a, Tile::Floor);
        w.map.set(b, Tile::Floor);
        let ia = w.entities.len();
        w.entities.push(dummy(ia, a, 20));
        let ib = w.entities.len();
        w.entities.push(dummy(ib, b, 20));
        w.entities[0].mana = 50;
        w.player_cast(&[Rune::Beam, Rune::Lightning], Dir::East)
            .unwrap();
        assert!(w.entities[ia].hp < 20, "first in line hit");
        assert!(w.entities[ib].hp < 20, "beam pierces to the second");
    }

    #[test]
    fn drain_heals_the_caster() {
        let mut w = World::new(12);
        w.entities[0].hp = 10;
        w.entities[0].mana = 50;
        let tpos = w.player().pos.step(Dir::East);
        w.map.set(tpos, Tile::Floor);
        let id = w.entities.len();
        w.entities.push(dummy(id, tpos, 50));
        w.player_cast(&[Rune::Bolt, Rune::Drain], Dir::East)
            .unwrap();
        assert!(w.entities[id].hp < 50, "target took drain damage");
        assert!(w.player().hp > 10, "caster healed from the drain");
    }

    #[test]
    fn teleport_blinks_the_caster_along_aim() {
        let mut w = World::new(13);
        // Carve a clear corridor east so the blink has somewhere to land.
        let p = w.player().pos;
        for i in 1..=4 {
            w.map.set(p.offset(i, 0), Tile::Floor);
        }
        w.entities[0].mana = 50;
        let before = w.player().pos;
        w.player_cast(&[Rune::Bolt, Rune::Teleport], Dir::East)
            .unwrap();
        assert_ne!(w.player().pos, before, "teleport should move the caster");
        assert_eq!(w.player().pos.y, before.y);
        assert!(w.player().pos.x > before.x);
    }

    #[test]
    fn barrier_conjures_a_temporary_wall() {
        let mut w = World::new(14);
        let tpos = w.player().pos.step(Dir::East);
        w.map.set(tpos, Tile::Floor);
        w.entities[0].mana = 50;
        w.player_cast(&[Rune::Touch, Rune::Barrier], Dir::East)
            .unwrap();
        assert_eq!(w.map.tile(tpos), Tile::Wall, "barrier becomes a wall");
        assert!(w.is_temp_wall(tpos));
    }

    #[test]
    fn chain_arcs_to_a_second_enemy() {
        let mut w = World::new(15);
        let p = w.player().pos;
        let a = p.step(Dir::East);
        let b = a.offset(1, 1); // within chain range of `a`
        w.map.set(a, Tile::Floor);
        w.map.set(b, Tile::Floor);
        let ia = w.entities.len();
        w.entities.push(dummy(ia, a, 30));
        let ib = w.entities.len();
        w.entities.push(dummy(ib, b, 30));
        w.entities[0].mana = 50;
        w.player_cast(&[Rune::Bolt, Rune::Lightning, Rune::Chain], Dir::East)
            .unwrap();
        assert!(w.entities[ia].hp < 30, "primary target hit");
        assert!(w.entities[ib].hp < 30, "chain arced to the neighbour");
    }

    #[test]
    fn preview_affected_matches_a_real_bolt() {
        let mut w = World::new(21);
        let tpos = w.player().pos.step(Dir::East);
        w.map.set(tpos, Tile::Floor);
        let id = w.entities.len();
        w.entities.push(dummy(id, tpos, 10));
        let spell = compile(&[Rune::Bolt, Rune::Fire]).unwrap();
        let (tiles, chain) = w.preview_affected(&spell, Dir::East);
        assert!(tiles.contains(&tpos), "impact tile is in the preview");
        assert!(chain.is_empty(), "one enemy, nothing to arc to");
        // The preview promised a hit; the real cast must deliver it.
        w.entities[0].mana = 50;
        w.player_cast(&[Rune::Bolt, Rune::Fire], Dir::East).unwrap();
        assert!(w.entities[id].hp < 10);
    }

    #[test]
    fn preview_affected_predicts_the_lightning_arc() {
        let mut w = World::new(22);
        let p = w.player().pos;
        let a = p.step(Dir::East);
        let b = a.offset(1, 1); // within chain range of `a`
        w.map.set(a, Tile::Floor);
        w.map.set(b, Tile::Floor);
        let ia = w.entities.len();
        w.entities.push(dummy(ia, a, 30));
        let ib = w.entities.len();
        w.entities.push(dummy(ib, b, 30));
        // Lightning's free arc rider: no Chain rune in the word.
        let spell = compile(&[Rune::Bolt, Rune::Lightning]).unwrap();
        let (tiles, chain) = w.preview_affected(&spell, Dir::East);
        assert!(tiles.contains(&a), "primary impact previewed");
        assert_eq!(chain, vec![b], "free arc previewed onto the neighbour");
    }

    #[test]
    fn killing_a_monster_scores_a_kill() {
        let mut w = World::new(16);
        let tpos = w.player().pos.step(Dir::East);
        w.map.set(tpos, Tile::Floor);
        let id = w.entities.len();
        w.entities.push(dummy(id, tpos, 1));
        w.entities[0].mana = 50;
        let kills0 = w.kills;
        w.player_cast(&[Rune::Bolt, Rune::Fire, Rune::Amplify], Dir::East)
            .unwrap();
        assert!(!w.entities[id].alive);
        assert_eq!(w.kills, kills0 + 1);
        assert!(w.score() > 0);
    }

    #[test]
    fn vitality_relic_raises_max_hp() {
        let mut w = World::new(17);
        let max0 = w.player().max_hp;
        w.apply_relic(Relic::Vitality);
        assert_eq!(w.player().max_hp, max0 + 10);
        assert_eq!(w.relics.len(), 1);
    }

    #[test]
    fn troll_regenerates_each_turn() {
        let mut w = World::new(18);
        // A wounded troll far from the player should heal when a turn passes.
        let id = w.entities.len();
        let mut t = dummy(id, w.map.stairs, 10);
        t.name = "troll".into();
        t.max_hp = 26;
        t.regen = 2;
        w.entities.push(t);
        let hp0 = w.entities[id].hp;
        w.player_wait();
        assert!(w.entities[id].hp > hp0, "troll should regenerate");
    }

    #[test]
    fn descending_increases_depth() {
        let mut w = World::new(6);
        w.entities[0].pos = w.map.stairs;
        w.descend();
        assert_eq!(w.depth, 2);
        assert!(w.player().alive);
    }

    #[test]
    fn insufficient_mana_spends_no_turn() {
        let mut w = World::new(7);
        w.entities[0].mana = 0;
        let before = w.turn;
        w.player_cast(&[Rune::Nova, Rune::Lightning], Dir::North)
            .unwrap();
        assert_eq!(w.turn, before);
    }

    #[test]
    fn drop_pool_is_derived_and_unchanged() {
        use Rune::*;
        // Locked item-by-item on purpose: the pool's order feeds seeded RNG
        // indexing, so reordering `Rune::ALL` or changing any weight would
        // silently shift every seeded run's drops. Note OnSelf never drops
        // (weight 0): it is in the starter kit, a spare would only dissolve.
        let expected = [
            Bolt, Beam, Nova, Touch, Fire, Frost, Frost, Lightning, Lightning, Heal, Force, Drain,
            Drain, Teleport, Barrier, Amplify, Amplify, Spread, Spread, Chain, Chain,
        ];
        assert_eq!(rune_drop_pool(), expected);
    }

    #[test]
    fn monster_table_matches_the_old_cascade() {
        // Boundary (depth, roll) pairs, resolved exactly as the old if-else
        // chain did — locks the data-table translation.
        assert_eq!(monster_for(4, 14).name, "troll");
        assert_eq!(monster_for(4, 15).name, "ogre");
        assert_eq!(monster_for(3, 14).name, "ogre"); // troll needs depth 4
        assert_eq!(monster_for(3, 34).name, "ogre");
        assert_eq!(monster_for(3, 35).name, "wraith");
        assert_eq!(monster_for(2, 54).name, "wraith");
        assert_eq!(monster_for(2, 55).name, "goblin");
        assert_eq!(monster_for(1, 0).name, "goblin");
        assert_eq!(monster_for(1, 71).name, "goblin");
        assert_eq!(monster_for(1, 72).name, "bat");
        assert_eq!(monster_for(1, 85).name, "bat");
        assert_eq!(monster_for(1, 86).name, "rat");
        assert_eq!(monster_for(1, 99).name, "rat");
        assert_eq!(monster_for(5, 0).name, "troll");
    }
}
