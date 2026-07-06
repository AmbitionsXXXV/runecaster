//! Headless balance harness for `rune_core` (ROADMAP Phase 1.5).
//!
//! `rune_core` has no renderer and no Bevy dependency, so it can run whole
//! dungeon dives in a tight loop, in-process, at native speed. This example
//! drives a small deterministic greedy bot through `N` seeded runs and prints
//! an aggregate survival/pressure report — the harness the Phase 1 balance
//! pass (rune-grammar rebalance, mana economy reform) needs to check its
//! work against, without a human replaying dozens of runs by hand.
//!
//! The bot only touches `rune_core`'s public API (`World`, `Map`, `Dir`,
//! `Rune`, ...) and carries no RNG of its own: every run's outcome is a pure
//! function of its seed, so two invocations with the same `N` produce
//! byte-identical stdout.
//!
//! Movement toward a goal (nearest monster, or the stairs) is a real
//! shortest-path (BFS) step rather than a 1-step chebyshev-distance
//! heuristic: `Map::generate`'s room-and-corridor layout routinely produces
//! dead-end branches that tie a pure 1-step lookahead against the correct
//! route around them, which a bot that must survive to depth 5 for this
//! report to mean anything cannot afford to get lost in.
//!
//! Usage: `cargo run -p rune_core --example balance -- [N]` (`N` defaults to
//! 200; seeds used are `0..N`).

use std::collections::VecDeque;

use rune_core::world::MAX_DEPTH;
use rune_core::{Dir, EntityId, Faction, Point, RunState, Rune, Tile, World};

/// Per-run turn budget. A run still `Playing` past this is called instead of
/// trusted to keep going — guards the harness against a single pathological
/// seed hanging the whole batch.
const TURN_BUDGET: u64 = 2000;

/// Defensive cap on bot *decisions* (distinct from world turns: a handful of
/// decisions, like descending, don't advance `w.turn` at all). Should never
/// bind in practice — see `take_decision`'s progress guarantee — but keeps
/// the harness itself from spinning forever if that guarantee is ever wrong.
const MAX_DECISIONS: u64 = 200_000;

/// Turns without shortening the path to the current goal before the bot
/// forces a one-off detour (first walkable direction, ignoring distance) —
/// a pure safety net against a goal that itself keeps moving (a fleeing or
/// repositioning monster); the BFS step below doesn't need it to avoid
/// getting stuck in place, since it always walks a genuine shortest path.
const STAGNATION_LIMIT: u64 = 40;

/// Consecutive decisions that leave the world completely unchanged (no turn,
/// no depth change, no state change) before the bot forces a `player_wait`.
const STALL_LIMIT: u32 = 3;

fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(200);
    let n = n.max(1);

    let records: Vec<RunRecord> = (0..n as u64).map(run_episode).collect();
    print_report(&records);
}

// --- run outcome bookkeeping ------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Won,
    Dead,
    Timeout,
}

struct RunRecord {
    #[allow(dead_code)] // kept for ad-hoc debugging; not surfaced in the report
    seed: u64,
    outcome: Outcome,
    final_depth: i32,
    turns: u64,
    kills: u32,
    mana_starved: u32,
    /// `(depth_left_from, hp_percent_at_that_moment)` for every successful
    /// `descend()`, including the final one that clears `MAX_DEPTH` and wins.
    descends: Vec<(i32, f64)>,
}

/// Bot memory that must survive across decisions within one episode: the
/// anti-softlock stagnation tracker, the immediate-stall counter, and which
/// monster (if any) the bot has committed to chasing (see `pick_goal`).
struct BotState {
    stall_count: u32,
    goal: Option<Point>,
    best_dist: u32,
    turn_of_best: u64,
    committed_monster: Option<EntityId>,
    /// A floor item the bot has spotted and is walking to. Sticky until
    /// picked up (or gone): goal selection must not depend on *current*
    /// visibility, or rounding a corner mid-walk would blind the bot and
    /// flip the goal back to the stairs every other turn.
    committed_loot: Option<Point>,
}

impl BotState {
    fn new() -> Self {
        BotState {
            stall_count: 0,
            goal: None,
            best_dist: u32::MAX,
            turn_of_best: 0,
            committed_monster: None,
            committed_loot: None,
        }
    }

    /// Record progress toward `goal` and report whether the bot has gone
    /// `STAGNATION_LIMIT` turns without beating its best-ever (BFS) distance
    /// to it, meaning a forced detour should override the normal step.
    fn should_detour(&mut self, goal: Point, dist: u32, turn: u64) -> bool {
        if self.goal != Some(goal) || dist < self.best_dist {
            self.goal = Some(goal);
            self.best_dist = dist;
            self.turn_of_best = turn;
            return false;
        }
        if turn.saturating_sub(self.turn_of_best) >= STAGNATION_LIMIT {
            // One-off nudge: restart the clock so this fires again only
            // after another full stagnation window, not on every turn after.
            self.turn_of_best = turn;
            true
        } else {
            false
        }
    }

    /// Forget the current goal and chase/loot commitments (called on floor
    /// transitions) so the next decision starts with a fresh baseline.
    fn forget_goal(&mut self) {
        self.goal = None;
        self.committed_monster = None;
        self.committed_loot = None;
    }

    /// The point the bot should currently be walking toward: whichever
    /// monster it's already committed to (as long as that monster is still
    /// alive — a monster once picked up stays the goal until it dies,
    /// deliberately with no distance-based un-commit), else the nearest
    /// living monster within 6 tiles (which becomes the new commitment),
    /// else the stairs.
    ///
    /// Sticking with a commitment regardless of how its *raw chebyshev*
    /// distance wanders matters because a monster near the 6-tile pickup
    /// boundary can have an *actual* path much longer than that raw
    /// distance suggests (behind a wall/detour): re-evaluating from scratch
    /// every decision would flip-flop forever between it and the stairs —
    /// one step toward the monster exits the 6-tile net (falling back to
    /// the stairs), one step toward the stairs re-enters it. Since a
    /// monster this far out is a dormant `Chaser` (out of its own sight
    /// range) that won't itself close the distance, only the bot's
    /// approach does, so an unconditional commitment is what makes that
    /// approach monotonic instead of oscillating.
    fn pick_goal(&mut self, w: &World, from: Point) -> Point {
        if let Some(id) = self.committed_monster {
            let e = &w.entities[id];
            if e.alive {
                return e.pos;
            }
            self.committed_monster = None;
        }
        match nearest_monster_id_within(w, from, 6) {
            Some((id, pos)) => {
                self.committed_monster = Some(id);
                pos
            }
            // No live threat in range: sweep up nearby loot before heading
            // for the stairs — learned runes are the bot's only growth, so
            // leaving them on the floor understates how a real player's
            // power curve rises across floors. A spotted item stays the
            // goal until it's actually picked up.
            None => {
                if let Some(p) = self.committed_loot {
                    if item_at(w, p) {
                        return p;
                    }
                    self.committed_loot = None;
                }
                match nearest_item_within(w, from, 8) {
                    Some(p) => {
                        self.committed_loot = Some(p);
                        p
                    }
                    None => w.map.stairs,
                }
            }
        }
    }
}

/// Whether a floor item (rune or relic) still lies on `p`.
fn item_at(w: &World, p: Point) -> bool {
    w.rune_items.iter().any(|(q, _)| *q == p) || w.relic_items.iter().any(|(q, _)| *q == p)
}

/// Nearest currently-visible floor item (rune or relic) within `max_dist`,
/// ties broken by list order (runes before relics, then insertion order).
fn nearest_item_within(w: &World, from: Point, max_dist: i32) -> Option<Point> {
    let mut best: Option<(Point, i32)> = None;
    let rune_spots = w.rune_items.iter().map(|(p, _)| *p);
    let relic_spots = w.relic_items.iter().map(|(p, _)| *p);
    for p in rune_spots.chain(relic_spots) {
        if !w.is_visible(p) {
            continue;
        }
        let d = from.chebyshev(p);
        if d <= max_dist && best.is_none_or(|(_, bd)| d < bd) {
            best = Some((p, d));
        }
    }
    best.map(|(p, _)| p)
}

fn run_episode(seed: u64) -> RunRecord {
    let mut w = World::new(seed);
    let mut mana_starved = 0u32;
    let mut descends: Vec<(i32, f64)> = Vec::new();
    let mut bot = BotState::new();
    let mut decisions: u64 = 0;

    let outcome = loop {
        if w.state != RunState::Playing {
            break if w.state == RunState::Won {
                Outcome::Won
            } else {
                Outcome::Dead
            };
        }
        if w.turn > TURN_BUDGET {
            break Outcome::Timeout;
        }
        decisions += 1;
        if decisions > MAX_DECISIONS {
            break Outcome::Timeout;
        }
        take_decision(&mut w, &mut bot, &mut mana_starved, &mut descends);
    };

    RunRecord {
        seed,
        outcome,
        final_depth: w.depth,
        turns: w.turn,
        kills: w.kills,
        mana_starved,
        descends,
    }
}

// --- the greedy bot ----------------------------------------------------------

/// True once *something* has actually happened to the world since the start
/// of this decision (a turn ticked, depth changed, or the run ended). Used
/// to cascade through the priority list: a step that fires but changes
/// nothing (e.g. a spell attempt that bounces off insufficient mana) must
/// fall through to the next priority *within the same decision*, while a
/// step that truly consumes the turn (even via a "swallowed by slow" no-op
/// spell) must stop the cascade there.
fn acted(w: &World, turn0: u64, depth0: i32) -> bool {
    w.turn != turn0 || w.depth != depth0 || w.state != RunState::Playing
}

/// Candidate attack words, strongest first, restricted to what the bot has
/// actually learned this run. The ladder is deliberately short: an
/// amplified fire bolt when the mana is there, a lightning bolt when its
/// free arc has a second target worth jumping to, then the fire-bolt
/// bread-and-butter, then the bare-element cantrip jab. Pure function of
/// world state — no randomness — so runs stay seed-deterministic.
fn attack_words(w: &World) -> Vec<Vec<Rune>> {
    let knows = |r: Rune| w.known_runes.contains(&r);
    let mut ladder: Vec<Vec<Rune>> = Vec::new();
    if knows(Rune::Amplify) {
        ladder.push(vec![Rune::Bolt, Rune::Fire, Rune::Amplify]);
    }
    if knows(Rune::Lightning) && monsters_within(w, 6) >= 2 {
        ladder.push(vec![Rune::Bolt, Rune::Lightning]);
    }
    ladder.push(vec![Rune::Bolt, Rune::Fire]);
    ladder.push(vec![Rune::Fire]);
    ladder
}

/// Living monsters within `d` (chebyshev) of the player.
fn monsters_within(w: &World, d: i32) -> usize {
    let from = w.player().pos;
    w.entities
        .iter()
        .filter(|e| e.alive && e.faction == Faction::Monster && from.chebyshev(e.pos) <= d)
        .count()
}

/// Evaluate the bot's priority list once and mutate `w` accordingly. Always
/// leaves the world having made progress: either a real game turn passed,
/// the depth changed, or the run ended.
fn take_decision(
    w: &mut World,
    bot: &mut BotState,
    mana_starved: &mut u32,
    descends: &mut Vec<(i32, f64)>,
) {
    let turn0 = w.turn;
    let depth0 = w.depth;

    // 1. `state != Playing` is handled by the caller before this is invoked.

    // 2. Standing on the stairs: always descend and stop.
    if w.map.tile(w.player().pos) == Tile::StairsDown {
        let hp_pct = hp_percent(w);
        let depth_leaving = w.depth;
        w.descend();
        descends.push((depth_leaving, hp_pct));
        bot.stall_count = 0;
        bot.forget_goal();
        return;
    }

    // 3. Emergency self-heal.
    {
        let p = w.player();
        let (hp, max_hp, mana) = (p.hp, p.max_hp, p.mana);
        let knows_self_heal =
            w.known_runes.contains(&Rune::OnSelf) && w.known_runes.contains(&Rune::Heal);
        if hp * 5 < max_hp * 2 && knows_self_heal && mana >= 5 {
            let _ = w.player_cast(&[Rune::OnSelf, Rune::Heal], Dir::North);
        }
    }

    // 4. Ranged attack in the first line-of-fire direction (8-way probe),
    // casting the strongest affordable word off the learned-rune ladder.
    if !acted(w, turn0, depth0) {
        if let Some(dir) = find_castable_direction(w) {
            for word in attack_words(w) {
                let cost = match w.preview(&word) {
                    Ok(spell) => spell.mana_cost,
                    Err(_) => continue,
                };
                if w.player().mana >= cost {
                    let _ = w.player_cast(&word, dir);
                    break;
                }
            }
            if !acted(w, turn0, depth0) {
                *mana_starved += 1;
            }
        }
    }

    // 5. Melee an adjacent monster.
    if !acted(w, turn0, depth0) {
        let from = w.player().pos;
        if let Some(target) = adjacent_monster(w, from) {
            let dx = (target.x - from.x).signum();
            let dy = (target.y - from.y).signum();
            if let Some(dir) = Dir::from_signums(dx, dy) {
                w.player_move(dir);
            }
        }
    }

    // 6 / 7. Chase the nearest monster within 6 tiles; with no threat in
    // range, sweep visible loot within 8; otherwise head for the stairs.
    if !acted(w, turn0, depth0) {
        let from = w.player().pos;
        let target = bot.pick_goal(w, from);
        chase(w, bot, from, target);
    }

    // Anti-softlock: if nothing above moved the world at all, force it.
    if acted(w, turn0, depth0) {
        bot.stall_count = 0;
    } else {
        bot.stall_count += 1;
        if bot.stall_count >= STALL_LIMIT {
            w.player_wait();
            bot.stall_count = 0;
        }
    }
}

/// Take one step toward `target` along the shortest walkable path, forcing
/// a plain first-walkable-direction detour if the bot has gone
/// `STAGNATION_LIMIT` turns without shortening that path (see
/// `STAGNATION_LIMIT`'s doc comment for why that's a rare-case safety net
/// rather than the thing keeping the bot un-stuck).
///
/// If the only route to `target` is itself blocked by a monster standing
/// somewhere on it — routine once a level's single connecting corridor has
/// a wandering monster on it, given `Map::generate`'s chain-of-rooms layout
/// rarely offers an alternate route — that monster becomes the effective
/// goal instead. Clearing it is a prerequisite to `target` anyway, and
/// walking toward it is exactly how it later ends up within priority 4's
/// firing line or priority 5's melee reach, even when it started out
/// farther than either one's detection radius.
fn chase(w: &mut World, bot: &mut BotState, from: Point, target: Point) {
    let step = bfs_next_step(w, from, target);
    let (goal, step) = if step.is_some() {
        (target, step)
    } else {
        match first_blocker_on_open_path(w, from, target) {
            Some(blocker) => (blocker, bfs_next_step(w, from, blocker)),
            None => (target, step),
        }
    };
    let dist = step.map(|(_, d)| d).unwrap_or(u32::MAX);
    let dir = if bot.should_detour(goal, dist, w.turn) {
        first_walkable_dir(w, from)
    } else {
        step.map(|(dir, _)| dir)
            .or_else(|| first_walkable_dir(w, from))
    };
    match dir {
        Some(d) => w.player_move(d),
        None => w.player_wait(),
    }
}

fn hp_percent(w: &World) -> f64 {
    let p = w.player();
    if p.max_hp <= 0 {
        0.0
    } else {
        p.hp as f64 / p.max_hp as f64 * 100.0
    }
}

/// First direction (`Dir::ALL` order) whose 1..=6 tile probe reaches a
/// living monster before running into a wall.
fn find_castable_direction(w: &World) -> Option<Dir> {
    for dir in Dir::ALL {
        let mut p = w.player().pos;
        let mut hit = false;
        for _ in 0..6 {
            p = p.step(dir);
            if w.blocking_entity_at(p).is_some() {
                hit = true;
                break;
            }
            if !w.map.is_walkable(p) {
                break;
            }
        }
        if hit {
            return Some(dir);
        }
    }
    None
}

fn adjacent_monster(w: &World, from: Point) -> Option<Point> {
    w.entities
        .iter()
        .filter(|e| e.alive && e.faction == Faction::Monster)
        .find(|e| from.chebyshev(e.pos) == 1)
        .map(|e| e.pos)
}

/// Nearest living monster within `max_dist` (chebyshev), ties broken by
/// entity id order (i.e. spawn order).
fn nearest_monster_id_within(w: &World, from: Point, max_dist: i32) -> Option<(EntityId, Point)> {
    let mut best: Option<(EntityId, Point, i32)> = None;
    for e in &w.entities {
        if !e.alive || e.faction != Faction::Monster {
            continue;
        }
        let d = from.chebyshev(e.pos);
        if d <= max_dist && best.is_none_or(|(_, _, bd)| d < bd) {
            best = Some((e.id, e.pos, d));
        }
    }
    best.map(|(id, p, _)| (id, p))
}

/// Shortest-path (BFS) first step from `from` toward `target` over walkable,
/// unoccupied tiles — `target` itself is always treated as steppable even
/// if a monster currently stands on it, since the bot only needs to walk up
/// to it (adjacency triggers priority 5's melee on a later decision), never
/// actually onto it. Returns the first step's direction and the path length
/// in tiles; `None` if `from == target` or no path exists (the latter
/// shouldn't happen on a connected dungeon floor).
fn bfs_next_step(w: &World, from: Point, target: Point) -> Option<(Dir, u32)> {
    if from == target {
        return None;
    }
    let width = w.map.width;
    let height = w.map.height;
    let tiles = (width * height) as usize;
    let idx = |p: Point| (p.y * width + p.x) as usize;

    let mut visited = vec![false; tiles];
    let mut came_from: Vec<Option<Dir>> = vec![None; tiles];
    let mut queue: VecDeque<(Point, u32)> = VecDeque::new();
    visited[idx(from)] = true;
    queue.push_back((from, 0));

    while let Some((cur, dist)) = queue.pop_front() {
        if cur == target {
            // Walk the recorded path back toward `from` one hop at a time:
            // the direction whose backward step *lands* on `from` is the
            // first step of the forward path, which is what the bot takes.
            let mut p = cur;
            loop {
                let dir = came_from[idx(p)]?;
                let back = p.step(opposite(dir));
                if back == from {
                    return Some((dir, dist));
                }
                p = back;
            }
        }
        for dir in Dir::ALL {
            let next = cur.step(dir);
            if !w.map.in_bounds(next) || visited[idx(next)] {
                continue;
            }
            let passable =
                w.map.is_walkable(next) && (next == target || w.blocking_entity_at(next).is_none());
            if !passable {
                continue;
            }
            visited[idx(next)] = true;
            came_from[idx(next)] = Some(dir);
            queue.push_back((next, dist + 1));
        }
    }
    None
}

/// The nearest (in tiles) monster standing on the walls-only shortest path
/// from `from` to `target` — i.e. the thing actually stopping
/// `bfs_next_step` from reaching `target`. `None` if that path doesn't
/// exist either (the map truly has no route at all) or has no monster on
/// it (shouldn't happen when `bfs_next_step` already failed, but this is
/// the fallback path for an already-unusual situation, so stay defensive).
fn first_blocker_on_open_path(w: &World, from: Point, target: Point) -> Option<Point> {
    let width = w.map.width;
    let height = w.map.height;
    let idx = |p: Point| (p.y * width + p.x) as usize;
    let mut visited = vec![false; (width * height) as usize];
    let mut prev: Vec<Option<Point>> = vec![None; (width * height) as usize];
    let mut queue: VecDeque<Point> = VecDeque::new();
    visited[idx(from)] = true;
    queue.push_back(from);
    while let Some(cur) = queue.pop_front() {
        if cur == target {
            let mut path = vec![cur];
            let mut p = cur;
            while let Some(pp) = prev[idx(p)] {
                path.push(pp);
                p = pp;
            }
            // `path` was built target -> ... -> from; reverse it so the
            // first blocker found walking it is the nearest one to `from`
            // (the very next obstacle in the way, not the last one).
            path.reverse();
            return path
                .into_iter()
                .find(|&p| p != from && w.blocking_entity_at(p).is_some());
        }
        for dir in Dir::ALL {
            let next = cur.step(dir);
            if w.map.in_bounds(next) && !visited[idx(next)] && w.map.is_walkable(next) {
                visited[idx(next)] = true;
                prev[idx(next)] = Some(cur);
                queue.push_back(next);
            }
        }
    }
    None
}

fn opposite(dir: Dir) -> Dir {
    match dir {
        Dir::North => Dir::South,
        Dir::South => Dir::North,
        Dir::East => Dir::West,
        Dir::West => Dir::East,
        Dir::NorthEast => Dir::SouthWest,
        Dir::NorthWest => Dir::SouthEast,
        Dir::SouthEast => Dir::NorthWest,
        Dir::SouthWest => Dir::NorthEast,
    }
}

/// Plain first walkable, unoccupied direction (`Dir::ALL` order), ignoring
/// distance entirely — the forced-detour action (see `STAGNATION_LIMIT`),
/// and the fallback when the map ever hands back no path at all.
fn first_walkable_dir(w: &World, from: Point) -> Option<Dir> {
    Dir::ALL.into_iter().find(|&dir| {
        let dest = from.step(dir);
        w.map.is_walkable(dest) && w.blocking_entity_at(dest).is_none()
    })
}

// --- reporting ---------------------------------------------------------------

fn print_report(records: &[RunRecord]) {
    let total = records.len();
    let wins = records.iter().filter(|r| r.outcome == Outcome::Won).count();
    let deaths = records
        .iter()
        .filter(|r| r.outcome == Outcome::Dead)
        .count();
    let timeouts = records
        .iter()
        .filter(|r| r.outcome == Outcome::Timeout)
        .count();

    println!("=== Runecaster Balance Harness (rune_core, headless) ===");
    println!("runs: {total}  (seeds 0..{})", total.saturating_sub(1));
    println!();

    println!("-- Summary --");
    println!("  {:<10} {:>6}  ({:>5.1}%)", "wins", wins, pct(wins, total));
    println!(
        "  {:<10} {:>6}  ({:>5.1}%)",
        "deaths",
        deaths,
        pct(deaths, total)
    );
    println!(
        "  {:<10} {:>6}  ({:>5.1}%)",
        "timeouts",
        timeouts,
        pct(timeouts, total)
    );
    println!();

    println!("-- Per floor --");
    println!(
        "  {:<6}{:>9}{:>9}{:>13}{:>16}",
        "depth", "reached", "deaths", "death-rate", "avg-leave-hp%"
    );
    let mut death_rates: Vec<(i32, f64)> = Vec::new();
    for depth in 1..=MAX_DEPTH {
        let reached = records.iter().filter(|r| r.final_depth >= depth).count();
        let died = records
            .iter()
            .filter(|r| r.outcome == Outcome::Dead && r.final_depth == depth)
            .count();
        let rate = pct(died, reached);

        let leave_samples: Vec<f64> = records
            .iter()
            .flat_map(|r| r.descends.iter())
            .filter(|(d, _)| *d == depth)
            .map(|(_, hp)| *hp)
            .collect();
        let avg_leave = if leave_samples.is_empty() {
            "-".to_string()
        } else {
            format!("{:.1}%", mean(leave_samples.iter().copied()))
        };

        println!(
            "  {:<6}{:>9}{:>9}{:>12.1}%{:>16}",
            depth, reached, died, rate, avg_leave
        );
        if reached > 0 {
            death_rates.push((depth, rate));
        }
    }
    println!();

    let avg_turns = mean(records.iter().map(|r| r.turns as f64));
    let avg_kills = mean(records.iter().map(|r| r.kills as f64));
    let avg_mana_starved = mean(records.iter().map(|r| r.mana_starved as f64));

    println!("-- Global --");
    println!("  {:<16}{:>10.1}", "avg turns", avg_turns);
    println!("  {:<16}{:>10.2}", "avg kills", avg_kills);
    println!("  {:<16}{:>10.2}", "avg mana-starved", avg_mana_starved);
    println!();

    println!("Conclusion: {}", gradient_conclusion(&death_rates));
}

fn pct(part: usize, whole: usize) -> f64 {
    if whole == 0 {
        0.0
    } else {
        part as f64 / whole as f64 * 100.0
    }
}

fn mean<I: Iterator<Item = f64>>(it: I) -> f64 {
    let mut sum = 0.0;
    let mut count = 0usize;
    for v in it {
        sum += v;
        count += 1;
    }
    if count == 0 {
        0.0
    } else {
        sum / count as f64
    }
}

/// Whether the per-floor death-rate sequence rises monotonically with
/// depth, as a well-tuned difficulty curve should — the harness's headline
/// balance verdict.
fn gradient_conclusion(rates: &[(i32, f64)]) -> String {
    const EPS: f64 = 1e-9;
    if rates.len() < 2 {
        return "not enough floors with data to judge a difficulty gradient.".to_string();
    }
    let strictly_increasing = rates.windows(2).all(|w| w[1].1 > w[0].1 + EPS);
    let non_decreasing = rates.windows(2).all(|w| w[1].1 >= w[0].1 - EPS);
    let non_increasing = rates.windows(2).all(|w| w[1].1 <= w[0].1 + EPS);

    if strictly_increasing {
        "death rate rises strictly with depth — the difficulty gradient is monotonic and well-tuned.".to_string()
    } else if non_decreasing {
        "death rate is non-decreasing with depth — the difficulty gradient trends monotonically harder (with some ties).".to_string()
    } else if non_increasing {
        "death rate never rises with depth — deeper floors are NOT harder for this bot, which is suspicious.".to_string()
    } else {
        let (worst_depth, worst_rate) = rates.iter().copied().fold(
            (rates[0].0, rates[0].1),
            |acc, r| if r.1 > acc.1 { r } else { acc },
        );
        format!(
            "death rate is NOT monotonic across floors (peak {worst_rate:.1}% at depth {worst_depth}) — the difficulty gradient has a bump/dip worth investigating."
        )
    }
}
