//! P0 regression tests: deterministic replay, cross-depth cleanup, rune-composition edge cases, and combat-number invariants.

use rune_core::{Dir, Point, RunState, Rune, Tile, World};

/// Neutralize all monsters so a hand-placed dummy is the only actor besides
/// the player (spawned monsters would otherwise interfere).
fn clear_monsters(w: &mut World) {
    for e in w.entities.iter_mut().skip(1) {
        e.alive = false;
    }
}

/// Spawn an inert monster-faction dummy at `pos` (also carves floor there).
fn add_dummy(w: &mut World, pos: Point, hp: i32, max_hp: i32) -> usize {
    w.map.set(pos, Tile::Floor);
    let id = w.entities.len();
    let mut d = w.entities[0].clone(); // clone player as a template
    d.id = id;
    d.name = "dummy".into();
    d.glyph = 'd';
    d.pos = pos;
    d.hp = hp;
    d.max_hp = max_hp;
    d.power = 0;
    d.mana = 0;
    d.max_mana = 0;
    d.faction = rune_core::Faction::Monster;
    d.ai = None;
    d.burning = 0;
    d.slowed = 0;
    d.regen = 0;
    d.alive = true;
    w.entities.push(d);
    id
}

/// The subset of entity state that must stay identical between two
/// independently-driven, identically-seeded worlds.
fn entity_snapshot(w: &World) -> Vec<(Point, i32, bool)> {
    w.entities.iter().map(|e| (e.pos, e.hp, e.alive)).collect()
}

/// A fixed, varied action sequence: mostly movement, with periodic bolts,
/// self-heals, and waits mixed in.
fn drive(w: &mut World, step: usize) {
    match step % 7 {
        0..=2 => w.player_move(Dir::ALL[step % 8]),
        3 => {
            let _ = w.player_cast(&[Rune::Bolt, Rune::Fire], Dir::ALL[(step * 3) % 8]);
        }
        4 => w.player_wait(),
        5 => {
            let _ = w.player_cast(&[Rune::OnSelf, Rune::Heal], Dir::North);
        }
        _ => w.player_move(Dir::ALL[(step + 5) % 8]),
    }
}

#[test]
fn determinism_full_replay_300_steps() {
    let mut a = World::new(777);
    let mut b = World::new(777);

    for step in 0..300 {
        drive(&mut a, step);
        drive(&mut b, step);

        assert_eq!(a.state, b.state, "state diverged at step {step}");
        assert_eq!(
            a.player().pos,
            b.player().pos,
            "player pos diverged at step {step}"
        );
        assert_eq!(
            a.player().hp,
            b.player().hp,
            "player hp diverged at step {step}"
        );
        assert_eq!(
            a.player().mana,
            b.player().mana,
            "player mana diverged at step {step}"
        );
        assert_eq!(a.turn, b.turn, "turn diverged at step {step}");
        assert_eq!(a.kills, b.kills, "kills diverged at step {step}");
        assert_eq!(
            a.entities.len(),
            b.entities.len(),
            "entity count diverged at step {step}"
        );
        assert_eq!(
            entity_snapshot(&a),
            entity_snapshot(&b),
            "entity (pos, hp, alive) snapshot diverged at step {step}"
        );
        assert_eq!(
            a.rune_items, b.rune_items,
            "rune_items diverged at step {step}"
        );
        assert_eq!(
            a.relic_items, b.relic_items,
            "relic_items diverged at step {step}"
        );
        assert_eq!(
            a.log.len(),
            b.log.len(),
            "log length diverged at step {step}"
        );

        if a.state != RunState::Playing {
            break;
        }
    }
}

#[test]
fn descend_clears_transient_state_and_keeps_progress() {
    let mut w = World::new(42);
    clear_monsters(&mut w);
    w.entities[0].pos = w.map.stairs;

    // Stairs sit at a room center, so its east neighbour is always floor.
    w.player_cast(&[Rune::Touch, Rune::Barrier], Dir::East)
        .unwrap();
    assert_eq!(w.temp_walls.len(), 1, "barrier conjures one temp wall");
    assert_eq!(w.turn, 1);
    assert_eq!(w.player().mana, 7, "12 - (Touch 1 + Barrier 4)");

    // Establish transient vs. persistent state right before descending.
    w.entities[0].burning = 5;
    w.entities[0].slowed = 3;
    w.entities[0].hp = 20;

    w.descend();

    assert_eq!(w.depth, 2);
    // The roster is rebuilt: slot 0 is the player again, and everything else
    // is a freshly spawned, full-health monster for the new floor (the old
    // floor's dead monsters are gone).
    assert_eq!(w.entities[0].id, 0);
    assert!(w.entities[0].is_player());
    assert!(
        w.entities
            .iter()
            .skip(1)
            .all(|e| e.faction == rune_core::Faction::Monster && e.alive && e.hp == e.max_hp),
        "all non-player entities are fresh spawns for the new depth"
    );
    // The floor breather: descending mends max_hp/4 and refills mana.
    assert_eq!(w.player().hp, 27, "20 carried over + 30/4 breather heal");
    assert_eq!(w.player().mana, 12, "mana refills to full on descend");
    assert_eq!(w.turn, 1, "turn counter carries over across depths");
    assert_eq!(w.player().burning, 0, "burning is cleared on descend");
    assert_eq!(w.player().slowed, 0, "slowed is cleared on descend");
    assert!(
        w.temp_walls.is_empty(),
        "temp walls do not survive a depth change"
    );
    assert_eq!(
        w.explored, w.visible,
        "a freshly generated floor's explored set equals its just-computed FOV"
    );
    assert_eq!(w.map.tile(w.player().pos), Tile::Floor);
}

#[test]
fn slowed_player_loses_move_action() {
    let mut w = World::new(5);
    clear_monsters(&mut w);
    w.entities[0].slowed = 1;
    let pos0 = w.player().pos;
    let turn0 = w.turn;
    assert_eq!(turn0, 0);

    let dir = Dir::ALL
        .into_iter()
        .find(|&d| w.map.is_walkable(w.player().pos.step(d)))
        .unwrap();
    w.player_move(dir);

    assert_eq!(w.player().pos, pos0, "the move action itself is swallowed");
    assert_eq!(w.player().slowed, 0, "the slowed stack is consumed");
    assert_eq!(w.turn, turn0 + 1, "the turn still advances");
}

#[test]
fn slowed_player_loses_cast_and_pays_no_mana() {
    let mut w = World::new(6);
    clear_monsters(&mut w);
    w.entities[0].slowed = 1;
    w.entities[0].hp = 15;
    let mana0 = w.player().mana;

    let result = w.player_cast(&[Rune::OnSelf, Rune::Heal], Dir::North);

    assert!(result.is_ok());
    assert_eq!(w.player().hp, 15, "the spell never actually casts");
    assert_eq!(w.player().mana, mana0, "no mana is spent");
    assert_eq!(w.player().slowed, 0, "the slowed stack is consumed");
    assert_eq!(w.turn, 1, "the turn still advances");
}

#[test]
fn nova_heal_does_not_heal_enemies() {
    let mut w = World::new(7);
    clear_monsters(&mut w);
    let dummy_pos = w.player().pos.step(Dir::East);
    let dummy_id = add_dummy(&mut w, dummy_pos, 10, 20);
    w.entities[0].hp = 20;

    // Nova + Heal costs 8 mana; the player starts with 12.
    w.player_cast(&[Rune::Nova, Rune::Heal], Dir::North)
        .unwrap();

    assert_eq!(
        w.entities[dummy_id].hp, 10,
        "enemies are not healed by the player's Nova+Heal"
    );
    assert_eq!(
        w.player().hp,
        20,
        "Nova excludes the caster's own tile, so the player isn't healed either"
    );
}

#[test]
fn chain_does_not_seek_on_a_complete_miss() {
    let mut w = World::new(8);
    clear_monsters(&mut w);
    let p = w.player().pos;
    // Clear a straight corridor east so the Bolt (range 6) hits neither a
    // wall nor an entity — a pure miss.
    for i in 1..=7 {
        w.map.set(p.offset(i, 0), Tile::Floor);
    }
    // Within chain range (4) of the origin, but nowhere near the bolt's path.
    let dummy_pos = p.offset(0, -2);
    let dummy_id = add_dummy(&mut w, dummy_pos, 30, 30);

    w.player_cast(&[Rune::Bolt, Rune::Lightning, Rune::Chain], Dir::East)
        .unwrap();

    assert_eq!(
        w.entities[dummy_id].hp, 30,
        "a complete miss must not chain-seek from the caster's own position"
    );
}

#[test]
fn melee_kill_still_procs_lifesteal() {
    let mut w = World::new(9);
    clear_monsters(&mut w);
    w.entities[0].hp = 10;
    w.melee_lifesteal = 2;
    let dummy_pos = w.player().pos.step(Dir::East);
    let dummy_id = add_dummy(&mut w, dummy_pos, 1, 1);

    // Bumping into the dummy is a melee attack; power 4 one-shots a 1-hp target.
    w.player_move(Dir::East);

    assert!(!w.entities[dummy_id].alive, "the dummy is killed");
    assert_eq!(w.player().hp, 12, "lifesteal still procs on a killing blow");
    assert_eq!(w.kills, 1);
}

#[test]
fn bolt_fire_full_turn_numbers() {
    let mut w = World::new(10);
    clear_monsters(&mut w);
    let dummy_pos = w.player().pos.step(Dir::East);
    let dummy_id = add_dummy(&mut w, dummy_pos, 20, 20);

    // power = 4 (one power-contributing element, Fire) = 4 direct damage,
    // plus a same-turn burn tick (burning 3 -> 2) for 2 more.
    w.player_cast(&[Rune::Bolt, Rune::Fire], Dir::East).unwrap();

    assert_eq!(w.entities[dummy_id].hp, 14, "20 - 4 direct - 2 burn tick");
    assert_eq!(
        w.entities[dummy_id].burning, 2,
        "3 - 1 from the same-turn tick"
    );
}

#[test]
fn damage_formula_regression() {
    // Bolt + Fire + Amplify: power = (4*1) * 3/2 = 6 direct damage, plus a
    // same-turn burn tick (Fire sets burning = 3) for another 2.
    {
        let mut w = World::new(20);
        clear_monsters(&mut w);
        let dummy_pos = w.player().pos.step(Dir::East);
        let dummy_id = add_dummy(&mut w, dummy_pos, 20, 20);
        w.player_cast(&[Rune::Bolt, Rune::Fire, Rune::Amplify], Dir::East)
            .unwrap();
        assert_eq!(w.entities[dummy_id].hp, 12, "20 - 6 direct - 2 burn tick");
    }

    // Bolt + Force: power = 4*1 = 4; Force damage is power/2 + 1 = 3. No
    // status effects, so there is no same-turn tick. A surviving Force
    // target is also shoved one tile further away from the caster.
    {
        let mut w = World::new(21);
        clear_monsters(&mut w);
        let dummy_pos = w.player().pos.step(Dir::East);
        // Carve the tile beyond the dummy too, so the push has somewhere to land.
        w.map.set(dummy_pos.step(Dir::East), Tile::Floor);
        let dummy_id = add_dummy(&mut w, dummy_pos, 20, 20);
        w.player_cast(&[Rune::Bolt, Rune::Force], Dir::East)
            .unwrap();
        assert_eq!(w.entities[dummy_id].hp, 17, "20 - 3, no status damage");
        assert_eq!(
            w.entities[dummy_id].pos,
            dummy_pos.step(Dir::East),
            "Force shoves the surviving target one tile away from the caster"
        );
    }

    // Bolt + Fire + Frost: power = 4*2 = 8; Fire and Frost each deal `power`
    // damage (16 total), plus burning += 3 and slowed += 2. The same-turn
    // burn tick deals 2 more. Slow is only spent in place of an action, and
    // this AI-less dummy never acts, so its stacks stay untouched.
    {
        let mut w = World::new(22);
        clear_monsters(&mut w);
        let dummy_pos = w.player().pos.step(Dir::East);
        let dummy_id = add_dummy(&mut w, dummy_pos, 20, 20);
        w.player_cast(&[Rune::Bolt, Rune::Fire, Rune::Frost], Dir::East)
            .unwrap();
        assert_eq!(w.entities[dummy_id].hp, 2, "20 - 16 direct - 2 burn tick");
        assert_eq!(
            w.entities[dummy_id].burning, 2,
            "3 - 1 from the same-turn tick"
        );
        assert_eq!(
            w.entities[dummy_id].slowed, 2,
            "an actor with no AI never spends slow"
        );
    }
}

/// `player_cast` tells the caller whether a spell actually happened, so the
/// front-end can keep the composed word when nothing was cast (mana refusal)
/// or when the action was eaten by slow.
#[test]
fn cast_reports_whether_a_spell_actually_happened() {
    let mut w = World::new(4242);
    clear_monsters(&mut w);

    // A real cast reports true and spends the turn.
    let before = w.turn;
    assert!(matches!(
        w.player_cast(&[Rune::Bolt, Rune::Fire], Dir::East),
        Ok(true)
    ));
    assert_eq!(w.turn, before + 1);

    // Mana refusal reports false, spends neither turn nor mana.
    w.entities[0].mana = 0;
    let before = w.turn;
    assert!(matches!(
        w.player_cast(&[Rune::Bolt, Rune::Fire], Dir::East),
        Ok(false)
    ));
    assert_eq!(w.turn, before, "a refused cast must not spend a turn");
    assert_eq!(w.entities[0].mana, 0, "a refused cast must not spend mana");

    // A slow-eaten cast also reports false; the turn passes but no mana is
    // paid (regen may add, never subtract — so `>=` guards the cost).
    w.entities[0].mana = 12;
    w.entities[0].slowed = 1;
    let before = w.turn;
    assert!(matches!(
        w.player_cast(&[Rune::Bolt, Rune::Fire], Dir::East),
        Ok(false)
    ));
    assert_eq!(
        w.turn,
        before + 1,
        "a slow-eaten action still advances time"
    );
    assert!(
        w.entities[0].mana >= 12,
        "no mana cost is paid on a swallowed cast"
    );
}

/// Heal's exchange rate: it must out-heal an equivalent-power damage spell,
/// since casting it spends a turn not dealing damage.
#[test]
fn heal_exchange_rate() {
    let mut w = World::new(201);
    clear_monsters(&mut w);
    w.entities[0].hp = 10;
    w.player_cast(&[Rune::OnSelf, Rune::Heal], Dir::North)
        .unwrap();
    assert_eq!(w.player().hp, 16, "10 + power 4 + HEAL_BONUS 2");
}

/// Lightning arcs to a nearby second enemy on its own — its signature
/// rider — with no Chain rune in the word at all.
#[test]
fn lightning_arcs_without_chain_rune() {
    let mut w = World::new(202);
    clear_monsters(&mut w);
    let p = w.player().pos;
    let a = p.step(Dir::East);
    let b = a.offset(1, 1); // within chain range of `a`
    let ia = add_dummy(&mut w, a, 30, 30);
    let ib = add_dummy(&mut w, b, 30, 30);

    w.player_cast(&[Rune::Bolt, Rune::Lightning], Dir::East)
        .unwrap();

    assert!(w.entities[ia].hp < 30, "primary target hit");
    assert!(
        w.entities[ib].hp < 30,
        "Lightning arcs to a neighbour even without a Chain rune"
    );
}

/// Killing a monster (by any means) refunds the player a little mana — the
/// counterweight to the kill-mana economy.
#[test]
fn kill_grants_mana() {
    let mut w = World::new(203);
    clear_monsters(&mut w);
    w.entities[0].mana = 5;
    let dummy_pos = w.player().pos.step(Dir::East);
    let dummy_id = add_dummy(&mut w, dummy_pos, 1, 1);

    // Bumping into the 1-hp dummy is a one-shot melee kill.
    w.player_move(Dir::East);

    assert!(!w.entities[dummy_id].alive, "the dummy is killed");
    assert_eq!(
        w.player().mana,
        8,
        "5 + KILL_MANA(3) on the kill; turn 1 doesn't trigger time-based regen"
    );
}

/// Passive mana regen now ticks every 6 turns (not 3); the Battery relic's
/// value is shrinking that interval to 3, not adding to the per-tick amount.
#[test]
fn mana_regen_slowed_to_six_turns() {
    let mut w = World::new(204);
    clear_monsters(&mut w);
    w.entities[0].mana = 0;
    for _ in 0..5 {
        w.player_wait();
    }
    assert_eq!(w.player().mana, 0, "regen has not fired before turn 6");
    w.player_wait();
    assert_eq!(w.player().mana, 1, "regen fires on the 6th turn");

    let mut battery = World::new(205);
    clear_monsters(&mut battery);
    battery.entities[0].mana = 0;
    battery.mana_regen_bonus = 1;
    for _ in 0..3 {
        battery.player_wait();
    }
    assert_eq!(
        battery.player().mana,
        1,
        "Battery shrinks the interval to 3 turns rather than adding a bonus amount"
    );
}

/// Farsight (or any FOV-radius increase) must not also wake monsters farther
/// away — waking distance is fixed, independent of the player's sight range.
#[test]
fn farsight_does_not_wake_distant_monsters() {
    let mut w = World::new(101);
    clear_monsters(&mut w);
    let p = w.player().pos;
    for i in 1..=10 {
        w.map.set(p.offset(i, 0), Tile::Floor);
    }
    let monster_pos = p.offset(9, 0);
    let monster_id = add_dummy(&mut w, monster_pos, 10, 10);
    w.entities[monster_id].ai = Some(rune_core::AiKind::Chaser);
    w.fov_radius = 10; // simulates the Farsight relic

    w.player_wait();

    assert_eq!(
        w.entities[monster_id].pos, monster_pos,
        "distance 9 exceeds the fixed wake radius (8); Farsight's extra visibility must not wake it"
    );
}
