use crate::state::{Mode, Sim, StaticTag};
use crate::{BOARD_LEFT, Z_PANEL, Z_UI};
use bevy::prelude::*;
use bevy::sprite::Anchor;
use rune_core::world::MAX_DEPTH;
use rune_core::{RunState, RuneCategory};

// ---------------------------------------------------------------------------
// UI (Text2d, rebuilt each step)
// ---------------------------------------------------------------------------

// Right-column layout: blocks stack from `y = 384.0` downward so a longer
// rune/relic list can never collide with a fixed-position neighbor.
/// Vertical gap inserted between stacked right-column blocks.
const GAP: f32 = 14.0;
/// Line height for 14px rows (RUNES body lines, WORD block line-count math).
const LINE_H_18: f32 = 18.0;
/// Line height for 13px rows (RELICS lines, log band lines).
const LINE_H_17: f32 = 17.0;
/// Vertical spacing between the STATUS block's HP/MP/SLOWED rows.
const STATUS_ROW_H: f32 = 20.0;

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

    // --- semi-transparent backing panels: above the map, below UI text ---
    let panel_color = Color::srgba(0.055, 0.06, 0.10, 0.94);
    for (cx, cy, pw, ph) in [
        (-140.0, 376.0, 960.0, 40.0),   // top bar
        (492.0, -4.0, 288.0, 792.0),    // right column
        (-140.0, -314.0, 960.0, 164.0), // log band
    ] {
        commands.spawn((
            Sprite::from_color(panel_color, Vec2::new(pw, ph)),
            Transform::from_xyz(cx, cy, Z_PANEL),
            StaticTag,
        ));
    }

    // --- top bar ---
    let mut header = format!(
        "Depth {}/{}   Kills {}   Score {}   Best {}   Turn {}",
        w.depth,
        MAX_DEPTH,
        w.kills,
        w.score(),
        best,
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
    text_block(
        commands,
        BOARD_LEFT + 8.0,
        388.0,
        15.0,
        header_color,
        header,
    );

    // --- right column: dynamically stacked blocks (no fixed-y overlap) ---
    let panel_x = 356.0;
    let mut y = 384.0;

    // a. STATUS — HP/MP bars, plus a note when the player is slowed.
    let hp_frac = frac(p.hp, p.max_hp);
    let hp_fg = if hp_frac > 0.5 {
        Color::srgb(0.35, 0.8, 0.42)
    } else if hp_frac > 0.25 {
        Color::srgb(0.9, 0.72, 0.3)
    } else {
        Color::srgb(0.9, 0.35, 0.3)
    };
    y = status_row(commands, panel_x, y, "HP", p.hp, p.max_hp, hp_fg);
    y = status_row(
        commands,
        panel_x,
        y,
        "MP",
        p.mana,
        p.max_mana,
        Color::srgb(0.42, 0.6, 0.95),
    );
    if p.slowed > 0 {
        text_block(
            commands,
            panel_x,
            y,
            13.0,
            Color::srgb(0.75, 0.9, 1.0),
            format!("SLOWED — next action lost (x{})", p.slowed),
        );
        y -= STATUS_ROW_H;
    }
    y -= GAP;

    // b. RUNES — title, then one individually colored line per known rune.
    text_block(
        commands,
        panel_x,
        y,
        13.0,
        Color::srgb(0.65, 0.7, 0.85),
        "RUNES — number adds (Shift: 11+)".to_string(),
    );
    y -= LINE_H_18;
    for (i, rune) in w.known_runes.iter().enumerate() {
        // Slots 1-10 are plain digits, 11-20 are Shift+digit ("s1".."s0");
        // must mirror the slot math in `input::handle_normal`.
        let key = match i {
            0..=9 => format!("{}", (i + 1) % 10),
            10..=19 => format!("s{}", (i + 1) % 10),
            _ => "··".to_string(), // unbound; unreachable with 16 rune kinds
        };
        let cat = match rune.category() {
            RuneCategory::Form => "form",
            RuneCategory::Element => "elem",
            RuneCategory::Modifier => "mod ",
        };
        let (cr, cg, cb) = rune.color();
        text_block(
            commands,
            panel_x,
            y,
            14.0,
            Color::srgb_u8(cr, cg, cb),
            format!("[{key:>2}] {:<9} {cat}", rune.name()),
        );
        y -= LINE_H_18;
    }
    y -= GAP;

    // c. WORD — the composing preview; unchanged logic, now its own block
    // that advances the cursor by its actual rendered line count.
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
    let word_lines = word.lines().count().max(1) as f32;
    text_block(commands, panel_x, y, 14.0, word_color, word);
    y -= word_lines * LINE_H_18;
    y -= GAP;

    // d. RELICS — title + one line per relic, only when the player has any.
    if !w.relics.is_empty() {
        text_block(
            commands,
            panel_x,
            y,
            13.0,
            Color::srgb(0.85, 0.78, 0.55),
            "RELICS".to_string(),
        );
        y -= LINE_H_17;
        for r in &w.relics {
            text_block(
                commands,
                panel_x,
                y,
                13.0,
                Color::srgb(0.85, 0.78, 0.55),
                format!("* {} — {}", r.name(), r.effect_desc()),
            );
            y -= LINE_H_17;
        }
    }

    // e. HINTS — pinned to the bottom of the right column, not stacked.
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
    text_block(commands, panel_x, -340.0, 13.0, hint_color, hint);

    // --- log band: older lines dim, the newest line highlighted ---
    let log_x = BOARD_LEFT + 8.0;
    let start = w.log.len().saturating_sub(8);
    let recent = &w.log[start..];
    if let Some((last, older)) = recent.split_last() {
        let mut ly = -238.0;
        if !older.is_empty() {
            text_block(
                commands,
                log_x,
                ly,
                13.0,
                Color::srgb(0.55, 0.57, 0.66),
                older.join("\n"),
            );
            ly -= older.len() as f32 * LINE_H_17;
        }
        text_block(
            commands,
            log_x,
            ly,
            13.0,
            Color::srgb(0.85, 0.88, 0.95),
            last.clone(),
        );
    }

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
        // Dim backdrop so the end-of-run text reads clearly over the board.
        commands.spawn((
            Sprite::from_color(
                Color::srgba(0.01, 0.01, 0.03, 0.86),
                Vec2::new(560.0, 300.0),
            ),
            Transform::from_xyz(0.0, 6.0, 49.0),
            StaticTag,
        ));
    }
}

/// One STATUS row: label, background+foreground bar, and a "cur/max" value.
/// Returns the y for the next stacked row (`y - STATUS_ROW_H`).
#[allow(clippy::too_many_arguments)]
fn status_row(
    commands: &mut Commands,
    panel_x: f32,
    y: f32,
    label: &str,
    cur: i32,
    max: i32,
    fg: Color,
) -> f32 {
    let f = frac(cur, max);
    text_block(
        commands,
        panel_x,
        y,
        14.0,
        Color::srgb(0.8, 0.85, 0.95),
        label.to_string(),
    );
    commands.spawn((
        Sprite::from_color(Color::srgb(0.16, 0.17, 0.24), Vec2::new(150.0, 9.0)),
        Transform::from_xyz(463.0, y - 7.0, 9.5),
        StaticTag,
    ));
    commands.spawn((
        Sprite::from_color(fg, Vec2::new(150.0 * f, 9.0)),
        Transform::from_xyz(388.0 + 75.0 * f, y - 7.0, 9.6),
        StaticTag,
    ));
    text_block(
        commands,
        545.0,
        y,
        14.0,
        Color::srgb(0.8, 0.85, 0.95),
        format!("{cur}/{max}"),
    );
    y - STATUS_ROW_H
}

/// Current/max ratio clamped to `0..1`; `0` for a non-positive max so a
/// degenerate stat (e.g. 0 max mana) draws an empty bar instead of panicking.
fn frac(cur: i32, max: i32) -> f32 {
    if max > 0 {
        (cur as f32 / max as f32).clamp(0.0, 1.0)
    } else {
        0.0
    }
}
