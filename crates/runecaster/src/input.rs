use crate::state::{Mode, Screen, Sim};
use bevy::prelude::*;
use rune_core::{Dir, Faction, Point, RunState, World, MAX_RUNES};

// ---------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------

pub(crate) fn handle_input(keys: Res<ButtonInput<KeyCode>>, mut sim: ResMut<Sim>) {
    if sim.screen == Screen::Title {
        if keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::Space) {
            sim.screen = Screen::Play;
            sim.dirty = true;
        }
        return;
    }
    if sim.world.state != RunState::Playing {
        if keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::KeyR) {
            sim.restart();
        }
        return;
    }
    match sim.mode {
        Mode::Normal => handle_normal(&keys, &mut sim),
        Mode::Aim => handle_aim(&keys, &mut sim),
    }
}

const DIGIT_KEYS: [KeyCode; 10] = [
    KeyCode::Digit1,
    KeyCode::Digit2,
    KeyCode::Digit3,
    KeyCode::Digit4,
    KeyCode::Digit5,
    KeyCode::Digit6,
    KeyCode::Digit7,
    KeyCode::Digit8,
    KeyCode::Digit9,
    KeyCode::Digit0,
];

fn handle_normal(keys: &ButtonInput<KeyCode>, sim: &mut Sim) {
    // Plain digits pick runes 1-10; Shift+digit reaches slots 11-20, so every
    // rune stays selectable late in a run (the full pool is 16 kinds).
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    for (i, key) in DIGIT_KEYS.iter().enumerate() {
        if keys.just_pressed(*key) {
            let slot = if shift { i + 10 } else { i };
            if let Some(&rune) = sim.world.known_runes.get(slot) {
                if sim.composing.contains(&rune) {
                    sim.world
                        .log
                        .push(format!("The {} rune is already in the word.", rune.name()));
                    sim.dirty = true;
                } else if sim.composing.len() < MAX_RUNES {
                    sim.composing.push(rune);
                    sim.dirty = true;
                }
            }
        }
    }
    if keys.just_pressed(KeyCode::Backspace) {
        sim.composing.pop();
        sim.dirty = true;
    }
    if keys.just_pressed(KeyCode::Escape) && !sim.composing.is_empty() {
        sim.composing.clear();
        sim.dirty = true;
    }

    if keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::KeyC) {
        begin_cast(sim);
        return;
    }

    // Spellbook: Q/E/R casts the bound word in one key; Shift+Q/E/R binds
    // the composed word to that slot (binding an empty word clears it).
    for (key, idx, name) in [
        (KeyCode::KeyQ, 0, "Q"),
        (KeyCode::KeyE, 1, "E"),
        (KeyCode::KeyR, 2, "R"),
    ] {
        if keys.just_pressed(key) {
            if shift {
                bind_slot(sim, idx, name);
            } else {
                cast_slot(sim, idx, name);
            }
            return;
        }
    }

    if keys.just_pressed(KeyCode::Period)
        && (keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight))
    {
        let before = sim.world.depth;
        sim.world.descend();
        if sim.world.depth != before {
            sim.level_gen += 1;
        }
        sim.dirty = true;
        return;
    }

    if keys.just_pressed(KeyCode::Space) {
        sim.world.player_wait();
        sim.dirty = true;
        return;
    }

    if let Some(dir) = movement_dir(keys) {
        sim.world.player_move(dir);
        sim.dirty = true;
    }
}

/// Compile the composed word and either enter Aim mode (directional forms)
/// or resolve it on the spot. The word stays loaded after every cast, so
/// repeat-firing is just Enter (+ a direction) instead of retyping the
/// whole word; Esc clears, Backspace edits.
fn begin_cast(sim: &mut Sim) {
    if sim.composing.is_empty() {
        return;
    }
    match sim.world.preview(&sim.composing) {
        Ok(spell) if spell.needs_aim() => {
            // Pre-aim at the nearest visible enemy so the obvious shot
            // ("the thing coming at me") needs no re-aiming: its direction
            // press fires immediately, or Enter accepts the suggestion.
            if let Some(dir) = suggest_aim(&sim.world) {
                sim.aim_dir = dir;
            }
            sim.mode = Mode::Aim;
            sim.dirty = true;
        }
        Ok(_) => {
            let words = sim.composing.clone();
            let _ = sim.world.player_cast(&words, Dir::North);
            sim.dirty = true;
        }
        Err(e) => {
            let msg = format!("Cannot cast: {}", e.message());
            sim.world.log.push(msg);
            sim.dirty = true;
        }
    }
}

/// Direction (by signum) from the player toward the nearest visible living
/// monster — Aim mode's opening suggestion.
fn suggest_aim(w: &World) -> Option<Dir> {
    let from = w.player().pos;
    let mut best: Option<(i32, Point)> = None;
    for e in &w.entities {
        if !e.alive || e.faction != Faction::Monster || !w.is_visible(e.pos) {
            continue;
        }
        let d = from.chebyshev(e.pos);
        if best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, e.pos));
        }
    }
    let (_, pos) = best?;
    Dir::from_signums((pos.x - from.x).signum(), (pos.y - from.y).signum())
}

/// Load spellbook slot `idx` into the composing word and fire it — the
/// one-key cast that keeps the rune grammar off the per-cast critical path.
fn cast_slot(sim: &mut Sim, idx: usize, name: &str) {
    sim.dirty = true;
    if sim.spellbook[idx].is_empty() {
        sim.world.log.push(format!(
            "Slot {name} is empty - compose a word, then Shift+{name} binds it."
        ));
        return;
    }
    // A half-composed draft survives the quick-cast: stash it, fire the
    // slot word, and bring the draft back once the cast resolves (or is
    // cancelled in Aim mode) — an emergency Q/E never eats your word.
    let draft = (!sim.composing.is_empty() && sim.composing != sim.spellbook[idx])
        .then(|| sim.composing.clone());
    sim.composing = sim.spellbook[idx].clone();
    begin_cast(sim);
    if sim.mode == Mode::Aim {
        sim.stashed_word = draft;
    } else if let Some(draft) = draft {
        sim.composing = draft;
    }
}

/// Bind the composed word to spellbook slot `idx`. Only a word that
/// compiles is bindable (a slot must always hold something castable);
/// binding while the word is empty clears the slot instead.
fn bind_slot(sim: &mut Sim, idx: usize, name: &str) {
    sim.dirty = true;
    if sim.composing.is_empty() {
        if !sim.spellbook[idx].is_empty() {
            sim.spellbook[idx].clear();
            sim.world.log.push(format!("Slot {name} cleared."));
        }
        return;
    }
    match sim.world.preview(&sim.composing) {
        Ok(spell) => {
            sim.spellbook[idx] = sim.composing.clone();
            sim.world
                .log
                .push(format!("Bound to {name}: {}.", spell.describe()));
        }
        Err(e) => {
            sim.world
                .log
                .push(format!("Won't bind a broken word: {}", e.message()));
        }
    }
}

/// Aim mode: the board highlights what the loaded word would hit along
/// `sim.aim_dir`. Pressing that same direction (or Enter/C) fires; a
/// *different* direction only re-aims, moving the highlight with it — so
/// what gets struck is always shown before it's committed.
fn handle_aim(keys: &ButtonInput<KeyCode>, sim: &mut Sim) {
    if keys.just_pressed(KeyCode::Escape) {
        exit_aim(sim);
        return;
    }
    if keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::KeyC) {
        fire_aimed(sim);
        return;
    }
    if let Some(dir) = movement_dir(keys) {
        if dir == sim.aim_dir {
            fire_aimed(sim);
        } else {
            sim.aim_dir = dir;
            sim.dirty = true;
        }
    }
}

fn fire_aimed(sim: &mut Sim) {
    let words = sim.composing.clone();
    let aim = sim.aim_dir;
    // Word stays loaded (see begin_cast): re-cast is Enter + direction.
    let _ = sim.world.player_cast(&words, aim);
    exit_aim(sim);
}

/// Leave Aim mode; a draft stashed by a quick-cast comes back now (after
/// firing and after Esc-cancel alike).
fn exit_aim(sim: &mut Sim) {
    sim.mode = Mode::Normal;
    if let Some(draft) = sim.stashed_word.take() {
        sim.composing = draft;
    }
    sim.dirty = true;
}

fn movement_dir(keys: &ButtonInput<KeyCode>) -> Option<Dir> {
    use KeyCode::*;
    let p = |k| keys.just_pressed(k);
    if p(KeyY) {
        Some(Dir::NorthWest)
    } else if p(KeyU) {
        Some(Dir::NorthEast)
    } else if p(KeyB) {
        Some(Dir::SouthWest)
    } else if p(KeyN) {
        Some(Dir::SouthEast)
    } else if p(ArrowUp) || p(KeyK) || p(KeyW) {
        Some(Dir::North)
    } else if p(ArrowDown) || p(KeyJ) || p(KeyS) {
        Some(Dir::South)
    } else if p(ArrowLeft) || p(KeyH) || p(KeyA) {
        Some(Dir::West)
    } else if p(ArrowRight) || p(KeyL) || p(KeyD) {
        Some(Dir::East)
    } else {
        None
    }
}
