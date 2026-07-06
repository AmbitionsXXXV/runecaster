//! The rune-grammar magic system — the heart of the game.
//!
//! The player collects **runes** and arranges them into an ordered "spell
//! word". [`compile`] parses that word into a concrete [`Spell`]:
//!
//! ```text
//! word     := FORM? ELEMENT+ MODIFIER*
//!   FORM     — how the spell is delivered. At most one. Defaults to `Bolt`.
//!   ELEMENT  — what it does. At least one is required.
//!   MODIFIER — tweaks power / area / behaviour. Any number.
//! ```
//!
//! Combos are emergent: `Bolt + Fire` is a fireball; `Nova + Fire` is a burst
//! around you; `Beam + Lightning` skewers a whole row; `Bolt + Teleport` is a
//! blink; `Nova + Barrier` walls you in; `Self + Heal` mends you; and
//! `Self + Fire` — a self-targeting form carrying a damaging element —
//! **backfires onto the caster**. That last one is the mistake you make once.

/// One collectable rune. The variant order is also the canonical display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rune {
    // --- Forms (delivery) ---
    Bolt,
    Beam,
    Nova,
    OnSelf,
    Touch,
    // --- Elements (effect) ---
    Fire,
    Frost,
    Lightning,
    Heal,
    Force,
    Drain,
    Teleport,
    Barrier,
    // --- Modifiers ---
    Amplify,
    Spread,
    Chain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuneCategory {
    Form,
    Element,
    Modifier,
}

impl Rune {
    pub const ALL: [Rune; 16] = [
        Rune::Bolt,
        Rune::Beam,
        Rune::Nova,
        Rune::OnSelf,
        Rune::Touch,
        Rune::Fire,
        Rune::Frost,
        Rune::Lightning,
        Rune::Heal,
        Rune::Force,
        Rune::Drain,
        Rune::Teleport,
        Rune::Barrier,
        Rune::Amplify,
        Rune::Spread,
        Rune::Chain,
    ];

    pub fn category(self) -> RuneCategory {
        use Rune::*;
        match self {
            Bolt | Beam | Nova | OnSelf | Touch => RuneCategory::Form,
            Fire | Frost | Lightning | Heal | Force | Drain | Teleport | Barrier => {
                RuneCategory::Element
            }
            Amplify | Spread | Chain => RuneCategory::Modifier,
        }
    }

    pub fn name(self) -> &'static str {
        use Rune::*;
        match self {
            Bolt => "Bolt",
            Beam => "Beam",
            Nova => "Nova",
            OnSelf => "Self",
            Touch => "Touch",
            Fire => "Fire",
            Frost => "Frost",
            Lightning => "Lightning",
            Heal => "Heal",
            Force => "Force",
            Drain => "Drain",
            Teleport => "Teleport",
            Barrier => "Barrier",
            Amplify => "Amplify",
            Spread => "Spread",
            Chain => "Chain",
        }
    }

    /// Per-rune mana contribution; a spell's cost is the sum of its runes.
    pub fn mana_cost(self) -> i32 {
        use Rune::*;
        match self {
            Bolt => 1,
            Beam => 3,
            Nova => 4,
            OnSelf => 1,
            Touch => 1,
            Fire => 3,
            Frost => 3,
            Lightning => 4,
            Heal => 4,
            Force => 3,
            Drain => 4,
            Teleport => 5,
            Barrier => 4,
            Amplify => 3,
            Spread => 2,
            Chain => 3,
        }
    }

    /// Relative weight in the floor-drop pool (0 = never drops). Exhaustive on
    /// purpose: adding a rune without deciding its drop rate must not compile.
    pub fn drop_weight(self) -> usize {
        use Rune::*;
        match self {
            Bolt => 1,
            Beam => 1,
            Nova => 1,
            OnSelf => 0, // starter kit; a spare would only dissolve to mana
            Touch => 1,
            Fire => 1,
            Frost => 2,
            Lightning => 2,
            Heal => 1,
            Force => 1,
            Drain => 2,
            Teleport => 1,
            Barrier => 1,
            Amplify => 2,
            Spread => 2,
            Chain => 2,
        }
    }

    /// RGB tint used by the renderer / UI.
    pub fn color(self) -> (u8, u8, u8) {
        use Rune::*;
        match self {
            Fire => (255, 120, 60),
            Frost => (120, 200, 255),
            Lightning => (255, 230, 90),
            Heal => (120, 255, 150),
            Force => (220, 160, 255),
            Drain => (210, 70, 160),
            Teleport => (150, 230, 255),
            Barrier => (140, 175, 230),
            Bolt | Beam | Nova | OnSelf | Touch => (130, 180, 255),
            Amplify | Spread | Chain => (205, 205, 205),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Form {
    Bolt,
    Beam,
    Nova,
    OnSelf,
    Touch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementKind {
    Fire,
    Frost,
    Lightning,
    Heal,
    Force,
    Drain,
    Teleport,
    Barrier,
}

impl ElementKind {
    /// Deals hit-point damage to entities it touches.
    pub fn is_damaging(self) -> bool {
        matches!(
            self,
            ElementKind::Fire
                | ElementKind::Frost
                | ElementKind::Lightning
                | ElementKind::Force
                | ElementKind::Drain
        )
    }

    /// Contributes to the spell's `power` value (damage or heal magnitude).
    /// Pure-utility elements (Teleport, Barrier) do not.
    pub fn contributes_power(self) -> bool {
        !matches!(self, ElementKind::Teleport | ElementKind::Barrier)
    }

    pub fn name(self) -> &'static str {
        match self {
            ElementKind::Fire => "Fire",
            ElementKind::Frost => "Frost",
            ElementKind::Lightning => "Lightning",
            ElementKind::Heal => "Heal",
            ElementKind::Force => "Force",
            ElementKind::Drain => "Drain",
            ElementKind::Teleport => "Teleport",
            ElementKind::Barrier => "Barrier",
        }
    }

    pub fn color(self) -> [u8; 3] {
        match self {
            ElementKind::Fire => [255, 120, 60],
            ElementKind::Frost => [120, 200, 255],
            ElementKind::Lightning => [255, 230, 90],
            ElementKind::Heal => [120, 255, 150],
            ElementKind::Force => [220, 160, 255],
            ElementKind::Drain => [210, 70, 160],
            ElementKind::Teleport => [150, 230, 255],
            ElementKind::Barrier => [140, 175, 230],
        }
    }
}

/// A fully-resolved, castable spell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spell {
    /// The original rune sequence (for display / replay).
    pub words: Vec<Rune>,
    pub form: Form,
    pub elements: Vec<ElementKind>,
    /// Magnitude applied to each affected entity (damage, or healing for Heal).
    pub power: i32,
    /// Area radius for Nova / Self / Bolt-splash (0 = single tile).
    pub radius: i32,
    /// Reach for Bolt / Beam / Touch.
    pub range: i32,
    /// Extra enemies the effect arcs to (the Chain modifier).
    pub chain: i32,
    pub mana_cost: i32,
    /// Backfire: a self-targeting form carrying a damaging element. Casting it
    /// hurts you. Still castable — sometimes you *want* the area effect.
    pub hits_caster: bool,
}

impl Spell {
    pub fn is_damaging(&self) -> bool {
        self.elements.iter().any(|e| e.is_damaging())
    }

    pub fn heals(&self) -> bool {
        self.elements.iter().any(|e| matches!(e, ElementKind::Heal))
    }

    pub fn drains(&self) -> bool {
        self.elements.iter().any(|e| matches!(e, ElementKind::Drain))
    }

    pub fn teleports(&self) -> bool {
        self.elements.iter().any(|e| matches!(e, ElementKind::Teleport))
    }

    pub fn has_barrier(&self) -> bool {
        self.elements.iter().any(|e| matches!(e, ElementKind::Barrier))
    }

    /// Whether the form needs the player to choose an aim direction.
    pub fn needs_aim(&self) -> bool {
        matches!(self.form, Form::Bolt | Form::Beam | Form::Touch)
    }

    /// The element that best represents this spell, for VFX styling: the first
    /// damaging element, else the first element.
    pub fn primary_element(&self) -> ElementKind {
        self.elements
            .iter()
            .copied()
            .find(|e| e.is_damaging())
            .or_else(|| self.elements.first().copied())
            .unwrap_or(ElementKind::Force)
    }

    /// The colour that best represents this spell, for VFX tinting.
    pub fn primary_color(&self) -> [u8; 3] {
        self.primary_element().color()
    }

    /// Short human description, e.g. `"Nova of Fire+Frost"`.
    pub fn describe(&self) -> String {
        let form = match self.form {
            Form::Bolt => "Bolt",
            Form::Beam => "Beam",
            Form::Nova => "Nova",
            Form::OnSelf => "Self",
            Form::Touch => "Touch",
        };
        let elems: Vec<&str> = self.elements.iter().map(|e| e.name()).collect();
        format!("{} of {}", form, elems.join("+"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpellError {
    Empty,
    NoElement,
    MultipleForms,
    TooComplex,
}

impl SpellError {
    pub fn message(self) -> &'static str {
        match self {
            SpellError::Empty => "Empty incantation.",
            SpellError::NoElement => {
                "A spell needs an element rune (Fire, Frost, Heal, Drain, Teleport, …)."
            }
            SpellError::MultipleForms => "A spell may have only one form rune.",
            SpellError::TooComplex => "That incantation is too tangled to hold (6 runes max).",
        }
    }
}

pub const MAX_RUNES: usize = 6;
const BASE_ELEMENT_POWER: i32 = 4;
const AMPLIFY_BONUS: i32 = 3;
const BOLT_BASE_RANGE: i32 = 6;
const BEAM_BASE_RANGE: i32 = 10;

/// Parse an ordered rune word into a castable [`Spell`], or explain why not.
pub fn compile(words: &[Rune]) -> Result<Spell, SpellError> {
    if words.is_empty() {
        return Err(SpellError::Empty);
    }
    if words.len() > MAX_RUNES {
        return Err(SpellError::TooComplex);
    }

    let mut form: Option<Form> = None;
    let mut elements: Vec<ElementKind> = Vec::new();
    let mut amplify = 0;
    let mut spread = 0;
    let mut chain = 0;
    let mut mana_cost = 0;

    for &rune in words {
        mana_cost += rune.mana_cost();
        match rune {
            Rune::Bolt => set_form(&mut form, Form::Bolt)?,
            Rune::Beam => set_form(&mut form, Form::Beam)?,
            Rune::Nova => set_form(&mut form, Form::Nova)?,
            Rune::OnSelf => set_form(&mut form, Form::OnSelf)?,
            Rune::Touch => set_form(&mut form, Form::Touch)?,
            Rune::Fire => elements.push(ElementKind::Fire),
            Rune::Frost => elements.push(ElementKind::Frost),
            Rune::Lightning => elements.push(ElementKind::Lightning),
            Rune::Heal => elements.push(ElementKind::Heal),
            Rune::Force => elements.push(ElementKind::Force),
            Rune::Drain => elements.push(ElementKind::Drain),
            Rune::Teleport => elements.push(ElementKind::Teleport),
            Rune::Barrier => elements.push(ElementKind::Barrier),
            Rune::Amplify => amplify += 1,
            Rune::Spread => spread += 1,
            Rune::Chain => chain += 1,
        }
    }

    if elements.is_empty() {
        return Err(SpellError::NoElement);
    }

    let form = form.unwrap_or(Form::Bolt);
    let power_elems = elements.iter().filter(|e| e.contributes_power()).count() as i32;
    let power = BASE_ELEMENT_POWER * power_elems + AMPLIFY_BONUS * amplify;

    let (radius, range) = match form {
        Form::Bolt => (spread, BOLT_BASE_RANGE + spread),
        Form::Beam => (0, BEAM_BASE_RANGE + spread * 2),
        Form::Nova => (1 + spread, 0),
        Form::OnSelf => (spread, 0),
        Form::Touch => (0, 1),
    };

    let damaging = elements.iter().any(|e| e.is_damaging());
    let hits_caster = matches!(form, Form::OnSelf) && damaging;

    Ok(Spell {
        words: words.to_vec(),
        form,
        elements,
        power,
        radius,
        range,
        chain,
        mana_cost,
        hits_caster,
    })
}

fn set_form(slot: &mut Option<Form>, f: Form) -> Result<(), SpellError> {
    if slot.is_some() {
        Err(SpellError::MultipleForms)
    } else {
        *slot = Some(f);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_word_is_an_error() {
        assert_eq!(compile(&[]), Err(SpellError::Empty));
    }

    #[test]
    fn modifier_without_element_is_an_error() {
        assert_eq!(compile(&[Rune::Amplify]), Err(SpellError::NoElement));
        assert_eq!(compile(&[Rune::Bolt, Rune::Chain]), Err(SpellError::NoElement));
    }

    #[test]
    fn two_forms_conflict() {
        assert_eq!(
            compile(&[Rune::Bolt, Rune::Beam, Rune::Fire]),
            Err(SpellError::MultipleForms)
        );
    }

    #[test]
    fn too_many_runes() {
        assert_eq!(compile(&[Rune::Fire; 7]), Err(SpellError::TooComplex));
    }

    #[test]
    fn bare_element_defaults_to_bolt() {
        let s = compile(&[Rune::Fire]).unwrap();
        assert_eq!(s.form, Form::Bolt);
        assert_eq!(s.power, BASE_ELEMENT_POWER);
        assert!(s.is_damaging());
        assert!(!s.hits_caster);
    }

    #[test]
    fn beam_has_long_range_and_needs_aim() {
        let s = compile(&[Rune::Beam, Rune::Lightning]).unwrap();
        assert_eq!(s.form, Form::Beam);
        assert!(s.range >= BEAM_BASE_RANGE);
        assert!(s.needs_aim());
    }

    #[test]
    fn chain_is_counted() {
        let s = compile(&[Rune::Bolt, Rune::Lightning, Rune::Chain, Rune::Chain]).unwrap();
        assert_eq!(s.chain, 2);
    }

    #[test]
    fn drain_is_damaging_and_flagged() {
        let s = compile(&[Rune::Bolt, Rune::Drain]).unwrap();
        assert!(s.is_damaging());
        assert!(s.drains());
        assert_eq!(s.power, BASE_ELEMENT_POWER);
    }

    #[test]
    fn teleport_and_barrier_are_utility_no_power() {
        let s = compile(&[Rune::Bolt, Rune::Teleport]).unwrap();
        assert!(s.teleports());
        assert!(!s.is_damaging());
        assert_eq!(s.power, 0, "pure-utility elements add no power");

        let b = compile(&[Rune::Touch, Rune::Barrier]).unwrap();
        assert!(b.has_barrier());
        assert_eq!(b.power, 0);
    }

    #[test]
    fn amplify_raises_power_and_cost() {
        let plain = compile(&[Rune::Bolt, Rune::Fire]).unwrap();
        let amped = compile(&[Rune::Bolt, Rune::Fire, Rune::Amplify]).unwrap();
        assert_eq!(amped.power, plain.power + AMPLIFY_BONUS);
        assert!(amped.mana_cost > plain.mana_cost);
    }

    #[test]
    fn spread_grows_the_nova() {
        let small = compile(&[Rune::Nova, Rune::Fire]).unwrap();
        let big = compile(&[Rune::Nova, Rune::Fire, Rune::Spread]).unwrap();
        assert_eq!(small.radius, 1);
        assert_eq!(big.radius, 2);
    }

    #[test]
    fn self_plus_fire_backfires() {
        let s = compile(&[Rune::OnSelf, Rune::Fire]).unwrap();
        assert!(s.hits_caster);
    }

    #[test]
    fn self_plus_heal_is_safe() {
        let s = compile(&[Rune::OnSelf, Rune::Heal]).unwrap();
        assert!(!s.hits_caster);
        assert!(s.heals());
        assert!(!s.is_damaging());
    }

    #[test]
    fn self_plus_drain_backfires() {
        let s = compile(&[Rune::OnSelf, Rune::Drain]).unwrap();
        assert!(s.hits_caster, "Drain is damaging, so Self+Drain recoils");
    }
}
