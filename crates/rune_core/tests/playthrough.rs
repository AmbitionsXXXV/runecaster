//! Smoke test: drive the world through many turns and assert it never panics
//! and stays internally consistent.

use rune_core::{Dir, RunState, Rune, World};

#[test]
fn many_turns_stay_consistent() {
    let mut w = World::new(123);

    for step in 0..400 {
        if w.state != RunState::Playing {
            break;
        }
        // Alternate between moving, casting, and waiting.
        match step % 6 {
            0 | 1 | 2 => w.player_move(Dir::ALL[step % Dir::ALL.len()]),
            3 => {
                let _ = w.player_cast(&[Rune::Bolt, Rune::Fire], Dir::ALL[step % 8]);
            }
            4 => w.player_wait(),
            _ => {
                let _ = w.player_cast(&[Rune::OnSelf, Rune::Heal], Dir::North);
            }
        }

        // Invariants that must hold every turn.
        let p = w.player();
        assert!(p.hp >= 0 && p.hp <= p.max_hp, "hp in range");
        assert!(p.mana >= 0 && p.mana <= p.max_mana, "mana in range");
        assert!(w.map.in_bounds(p.pos), "player on the map");
        for e in &w.entities {
            assert!(e.hp >= 0, "no negative hp");
            if e.alive {
                assert!(w.map.is_walkable(e.pos), "living actors never stand in walls");
            }
        }
    }
}

#[test]
fn determinism_same_seed_same_first_state() {
    let a = World::new(999);
    let b = World::new(999);
    assert_eq!(a.player().pos, b.player().pos);
    assert_eq!(a.entities.len(), b.entities.len());
    assert_eq!(a.rune_items, b.rune_items);
}
