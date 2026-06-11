# Runecaster

A turn-based **rune-grammar roguelike** built in Rust with [Bevy](https://bevyengine.org).
You don't pick spells from a list — you *compose* them. Collect runes, arrange
them into a "spell word", and a compiler turns that word into a concrete spell.
Combos are emergent, and some are mistakes you only make once.

```
Bolt + Fire               → a fireball hurled in a direction
Beam + Lightning          → a bolt that pierces a whole row of enemies
Nova + Fire               → a burst of flame around you
Nova + Lightning + Spread → a wide ring of lightning
Bolt + Lightning + Chain  → strikes a target, then arcs to nearby foes
Bolt + Drain              → damage an enemy and heal yourself
Bolt + Teleport           → blink to where the spell lands
Nova + Barrier            → wall yourself in with conjured stone
Self + Heal               → mend yourself
Self + Fire               → set YOURSELF on fire (the spell recoils — backfire!)
```

## Run it

```bash
# from the repository root
cargo run -p runecaster              # debug
cargo run --release -p runecaster    # smooth framerate (recommended)
```

The first build compiles all of Bevy and takes a while; later builds are fast.

## Controls

| Action | Keys |
|---|---|
| Move / bump-attack | Arrow keys, `WASD`, or `hjkl` |
| Move diagonally | `y` `u` `b` `n` (NW NE SW SE) |
| Wait a turn | `Space` |
| Descend stairs `>` | `Shift` + `.` while standing on the `>` |
| Compose a spell | number keys — each appends the matching rune from the right panel |
| Drop last rune | `Backspace` |
| Clear the word | `Esc` |
| Cast | `Enter` (or `C`). Bolt/Touch spells then ask you to **aim** a direction. |
| New run (after death/victory) | `Enter` or `R` |

The right panel shows your known runes with their hotkeys, the word you're
building, and a **live preview** of what it compiles to — power, mana cost, a
backfire warning, and any grammar error.

## The rune grammar

A spell word is `FORM? ELEMENT+ MODIFIER*`:

- **Forms** (delivery — at most one; defaults to `Bolt`):
  `Bolt` (stops at the first target), `Beam` (pierces a whole line),
  `Nova` (burst around you), `Self` (affects your tile), `Touch` (the tile you face).
- **Elements** (effect — at least one required):
  `Fire` (damage + burn), `Frost` (damage + slow), `Lightning` (damage),
  `Force` (impact damage), `Heal` (restores HP), `Drain` (damage + lifesteal),
  `Teleport` (blink to the landing tile), `Barrier` (conjure a temporary wall).
- **Modifiers** (any number):
  `Amplify` (more power), `Spread` (bigger area / longer range),
  `Chain` (arc the effect to nearby enemies).

**Backfire:** a self-targeting form (`Self`) carrying a *damaging* element
strikes the caster. The UI warns you, but it's still castable — sometimes you
*want* an effect centered on yourself.

## Monsters, relics & scoring

- **Monsters** scale with depth: rats and bats, goblins, wraiths, ogres, and
  **trolls** (which regenerate every turn — burst them down).
- **Relics** are floor pickups granting a permanent run-long passive: more HP /
  mana / sight, melee that ignites or heals you, faster mana regen, or HP
  regeneration. They stack across a run.
- **Score** rewards depth, kills, and relics (with a victory bonus) and your
  best is persisted to `~/.runecaster_highscore`.

Descend through 5 levels of the Glyph Vaults to win.

## Architecture

A two-crate Cargo workspace that deliberately separates rules from rendering:

```
runecaster/
├─ crates/
│  ├─ rune_core/      ← pure game logic, NO engine dependency
│  │  ├─ geometry.rs    points, directions
│  │  ├─ map.rs         dungeon generation (rooms + corridors)
│  │  ├─ fov.rs         field of view
│  │  ├─ runes.rs       ★ the rune-grammar spell compiler
│  │  ├─ entity.rs      actors
│  │  └─ world.rs       turn engine, casting, combat, AI, levels
│  └─ runecaster/     ← Bevy 0.18 front-end (input + rendering only)
│     └─ main.rs
```

`rune_core` has no Bevy dependency, so the entire simulation is unit-tested and
runs in seconds without compiling the engine:

```bash
cargo test -p rune_core      # 20 tests: spell grammar, combat, FOV, determinism
cargo run  -p runecaster 7   # (the binary also has a tiny ASCII level preview;
                             #  see the bottom of main.rs history) 
```

Because the rules are engine-agnostic, the same `rune_core` could later drive a
terminal client, a web build, or a fuller Bevy game with sprites and audio.

## Rendering & art

- **Tiles, floor items, and UI** are rebuilt each game step (tagged `StaticTag`).
- **Actors** (player + monsters) are *persistent* sprites tracked in `ActorViews`
  by entity id. Each step sets their `AnimTarget`; the `animate` system lerps
  toward it every frame, so they **glide between tiles** instead of snapping.
  A `level_gen` counter wipes actor sprites on restart/descent so we never
  animate a sprite across a level change.
- **Sprite textures are generated procedurally at startup** (`px_floor`,
  `px_wall`, `px_player`, …) — pixel-art drawn straight into RGBA buffers, with
  nearest-neighbour sampling for a crisp look. No external assets required.
- **Skill effects** are driven by two event buffers the sim fills and the
  renderer drains each step:
  - `cast_fx: Vec<CastFx>` — a spell's *signature motion* (form + element +
    trajectory). `spawn_cast_fx` turns each into: **Bolt** → a flying
    `Projectile` that trails and bursts on arrival; **Beam** → a row of
    oriented `streak` particles; **Nova** → an expanding shock `ring` +
    element burst; **Self** → an aura ring; **Touch** → an element burst.
  - `fx: Vec<Fx>` — point events (impacts, heals, deaths, teleports, barriers),
    each carrying its `element` so the burst is styled per element.
- **Per-element particle styles** live in `elem_burst`: fire embers rise and
  shrink, frost shards scatter and fall, lightning streaks fast, drain motes
  drift magenta, heal motes rise green. Particles support velocity, gravity
  (`accel`), spin, and a scale/alpha curve.
- **Tuning knobs:** `elem_burst` (counts/colours/motion per element),
  `spawn_cast_fx` (form motion), and the procedural textures `px_spark` /
  `px_ring` / `px_shard` / `px_streak`. All are plain data + pixel math.
- **Camera shake** ramps up when the player takes damage and decays over time.
- A **title screen** and a **game-over card** show the run score and the
  persisted best.
- **Sound is not wired** (it needs audio asset files). To add it, drop `.ogg`
  files in `crates/runecaster/assets/` and play them with `AudioPlayer` in the
  systems that already react to events (e.g. when draining `Fx`).

**Swapping in a real tileset:** replace the `make_image(...)` calls in `setup`
with `asset_server.load("tiles/wall.png")` (drop art in `crates/runecaster/assets/`).
For a packed atlas, add a `TextureAtlasLayout` and set `Sprite::texture_atlas`.
Everything downstream already works off `Handle<Image>`, so nothing else changes.

## Extending it

- **A new rune:** add a variant in `runes.rs` (`Rune`, its `category`/`name`/
  `mana_cost`/`color`), handle it in `compile`, and apply its effect in
  `world.rs::apply_spell_to`. Add it to `rune_drop_pool` so it appears in
  dungeons. The compiler's unit tests will guide you.
- **A new monster:** extend `World::spawn_monster`.
- **Smarter AI:** `AiKind` + `World::monster_turns` (currently a greedy chaser).
