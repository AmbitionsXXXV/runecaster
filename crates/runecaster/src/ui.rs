use crate::state::{Mode, Sim, StaticTag};
use crate::{BOARD_LEFT, Z_UI};
use bevy::prelude::*;
use bevy::sprite::Anchor;
use rune_core::world::MAX_DEPTH;
use rune_core::{RunState, RuneCategory};

// ---------------------------------------------------------------------------
// UI (Text2d, rebuilt each step)
// ---------------------------------------------------------------------------

fn text_block(commands: &mut Commands, x: f32, y: f32, size: f32, color: Color, text: String) {
    commands.spawn((
        Text2d::new(text),
        TextFont {
            font_size: size,
            ..default()
        },
        TextColor(color),
        TextLayout::new_with_justify(Justify::Left),
        Anchor::TOP_LEFT,
        Transform::from_xyz(x, y, Z_UI),
        StaticTag,
    ));
}

fn center_text(commands: &mut Commands, y: f32, size: f32, color: Color, text: &str) {
    commands.spawn((
        Text2d::new(text.to_string()),
        TextFont {
            font_size: size,
            ..default()
        },
        TextColor(color),
        TextLayout::new_with_justify(Justify::Center),
        Transform::from_xyz(0.0, y, 50.0),
        StaticTag,
    ));
}

pub(crate) fn draw_title(commands: &mut Commands, best: u32) {
    center_text(
        commands,
        170.0,
        72.0,
        Color::srgb(0.85, 0.8, 1.0),
        "RUNECASTER",
    );
    center_text(
        commands,
        90.0,
        22.0,
        Color::srgb(0.65, 0.7, 0.85),
        "a rune-grammar roguelike",
    );
    center_text(
        commands,
        0.0,
        24.0,
        Color::srgb(0.95, 0.9, 0.6),
        "Press ENTER to descend into the Glyph Vaults",
    );
    center_text(
        commands,
        -90.0,
        16.0,
        Color::srgb(0.6, 0.62, 0.72),
        "Compose runes into spells. Bolt+Fire burns. Self+Fire burns YOU.\nMove wasd/hjkl/arrows (diag yubn) · number keys build a spell · Enter casts",
    );
    if best > 0 {
        center_text(
            commands,
            -190.0,
            18.0,
            Color::srgb(0.8, 0.75, 0.5),
            &format!("Best score: {best}"),
        );
    }
}

pub(crate) fn spawn_ui(commands: &mut Commands, sim: &Sim, best: u32) {
    let w = &sim.world;
    let p = w.player();

    let mut header = format!(
        "RUNECASTER   Depth {}/{}   HP {}/{}   Mana {}/{}   Kills {}   Score {}   Turn {}",
        w.depth,
        MAX_DEPTH,
        p.hp,
        p.max_hp,
        p.mana,
        p.max_mana,
        w.kills,
        w.score(),
        w.turn
    );
    // Frost slow on the player is otherwise invisible until an action is
    // swallowed; surface it in the header so the lost turn reads as fair.
    let header_color = if p.slowed > 0 {
        header.push_str(&format!("   !! SLOWED x{} !!", p.slowed));
        Color::srgb(0.75, 0.9, 1.0)
    } else {
        Color::srgb(0.9, 0.9, 1.0)
    };
    text_block(commands, BOARD_LEFT, 390.0, 16.0, header_color, header);

    let panel_x = 462.0;
    let mut panel = String::from("RUNES — press the number\n");
    for (i, rune) in w.known_runes.iter().enumerate() {
        let key = (i + 1) % 10;
        let cat = match rune.category() {
            RuneCategory::Form => "form",
            RuneCategory::Element => "elem",
            RuneCategory::Modifier => "mod ",
        };
        panel.push_str(&format!("[{key}] {:<9} {cat}\n", rune.name()));
    }
    text_block(
        commands,
        panel_x,
        372.0,
        14.0,
        Color::srgb(0.8, 0.85, 0.95),
        panel,
    );

    let mut word = String::from("WORD: ");
    if sim.composing.is_empty() {
        word.push_str("(empty)");
    } else {
        let names: Vec<&str> = sim.composing.iter().map(|r| r.name()).collect();
        word.push_str(&names.join(" + "));
    }
    word.push('\n');

    let mut word_color = Color::srgb(0.95, 0.95, 0.7);
    match w.preview(&sim.composing) {
        Ok(spell) => {
            word.push_str(&format!("=> {}\n", spell.describe()));
            let kind = if spell.heals() && !spell.is_damaging() {
                format!("heal {}", spell.power)
            } else {
                format!("power {}", spell.power)
            };
            word.push_str(&format!("   {kind}   cost {} mana\n", spell.mana_cost));
            if spell.hits_caster {
                word.push_str("   ! THIS WILL HIT YOU !\n");
                word_color = Color::srgb(1.0, 0.5, 0.4);
            }
            word.push_str(if spell.needs_aim() {
                "   Enter, then aim a direction\n"
            } else {
                "   Enter to cast\n"
            });
        }
        Err(e) => {
            if !sim.composing.is_empty() {
                word.push_str(&format!("   {}\n", e.message()));
                word_color = Color::srgb(1.0, 0.6, 0.5);
            }
        }
    }
    text_block(commands, panel_x, 150.0, 14.0, word_color, word);

    let hint = match sim.mode {
        Mode::Normal => {
            "move wasd/hjkl/arrows · diag yubn\nSpace wait · Shift+. descend\nnumbers compose · Enter cast · Esc clear"
                .to_string()
        }
        Mode::Aim => ">>> AIM: press a direction\n(Esc to cancel)".to_string(),
    };
    let hint_color = if sim.mode == Mode::Aim {
        Color::srgb(1.0, 0.9, 0.4)
    } else {
        Color::srgb(0.6, 0.62, 0.72)
    };
    text_block(commands, panel_x, 20.0, 13.0, hint_color, hint);

    if !w.relics.is_empty() {
        let mut rel = String::from("RELICS\n");
        for r in &w.relics {
            rel.push_str(&format!("* {}\n", r.name()));
        }
        text_block(
            commands,
            panel_x,
            -60.0,
            13.0,
            Color::srgb(0.85, 0.78, 0.55),
            rel,
        );
    }

    let start = w.log.len().saturating_sub(7);
    let log = w.log[start..].join("\n");
    text_block(
        commands,
        BOARD_LEFT,
        -296.0,
        13.0,
        Color::srgb(0.7, 0.72, 0.8),
        log,
    );

    if w.state != RunState::Playing {
        let (title, color) = match w.state {
            RunState::Won => ("VICTORY!", Color::srgb(0.5, 1.0, 0.6)),
            RunState::Dead => ("YOU DIED", Color::srgb(1.0, 0.4, 0.4)),
            RunState::Playing => unreachable!(),
        };
        center_text(commands, 60.0, 56.0, color, title);
        center_text(
            commands,
            -10.0,
            26.0,
            Color::srgb(0.92, 0.92, 1.0),
            &format!("Score {}      Best {}", w.score(), best.max(w.score())),
        );
        center_text(
            commands,
            -70.0,
            20.0,
            Color::srgb(0.7, 0.72, 0.82),
            "Enter / R for a new run",
        );
    }
}
