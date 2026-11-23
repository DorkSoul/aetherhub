//! Token definitions that effects refer to by name.

use mtg_engine::dsl::*;
use mtg_engine::mana::ManaType;
use mtg_engine::types::*;

fn token(name: &str, types: &[CardType], subtypes: &[&str], colors: &[Color], pt: Option<(i32, i32)>) -> CardDef {
    CardDef {
        name: name.to_string(),
        types: types.to_vec(),
        subtypes: subtypes.iter().map(|s| s.to_string()).collect(),
        colors: Some(colors.to_vec()),
        power: pt.map(|p| p.0),
        toughness: pt.map(|p| p.1),
        token: true,
        ..Default::default()
    }
}

fn sac_for(effects: Vec<Effect>, mana: Option<&str>, tap: bool) -> Ability {
    let mut cost = vec![];
    if let Some(m) = mana {
        cost.push(Cost::Mana { cost: m.to_string() });
    }
    if tap {
        cost.push(Cost::Tap);
    }
    cost.push(Cost::SacrificeSelf);
    Ability::Activated { cost, targets: vec![], effects, timing: Timing::Instant, loyalty: None, limit_per_turn: None, once: false, condition: None, from: None, power_up: false, special_action: false }
}

pub fn builtin_tokens() -> Vec<CardDef> {
    let mut v = vec![];
    for (n, st, c, pt) in [
        ("Soldier", "Soldier", Color::W, (1, 1)),
        ("Spirit", "Spirit", Color::W, (1, 1)),
        ("Knight", "Knight", Color::W, (2, 2)),
        ("Zombie", "Zombie", Color::B, (2, 2)),
        ("Goblin", "Goblin", Color::R, (1, 1)),
        ("Elf Warrior", "Elf Warrior", Color::G, (1, 1)),
        ("Saproling", "Saproling", Color::G, (1, 1)),
        ("Beast", "Beast", Color::G, (3, 3)),
        ("Elemental", "Elemental", Color::R, (1, 1)),
        ("Insect", "Insect", Color::G, (1, 1)),
        ("Bird", "Bird", Color::W, (1, 1)),
        ("Human", "Human", Color::W, (1, 1)),
        ("Rat", "Rat", Color::B, (1, 1)),
        ("Wolf", "Wolf", Color::G, (2, 2)),
        ("Cat", "Cat", Color::G, (1, 1)),
        ("Thopter", "Thopter", Color::W, (1, 1)),
    ] {
        let mut t = token(n, &[CardType::Creature], &st.split(' ').collect::<Vec<_>>(), &[c], Some(pt));
        if n == "Thopter" {
            t.colors = Some(vec![]);
            t.types = vec![CardType::Artifact, CardType::Creature];
            t.keywords = vec![Keyword::Flying];
        }
        if n == "Spirit" || n == "Bird" {
            t.keywords = vec![Keyword::Flying];
        }
        v.push(t);
    }
    // Army starts as 0/0 and grows with +1/+1 counters
    v.push(token("Army", &[CardType::Creature], &["Zombie", "Army"], &[Color::B], Some((0, 0))));
    let mut treasure = token("Treasure", &[CardType::Artifact], &["Treasure"], &[], None);
    treasure.abilities.push(sac_for(
        vec![Effect::AddMana { mana: vec![ManaProduce::AnyColor { n: 1 }], until_end_of_combat: false, until_end_of_turn: false, who: None, player_choice: false }],
        None,
        true,
    ));
    v.push(treasure);
    let mut clue = token("Clue", &[CardType::Artifact], &["Clue"], &[], None);
    clue.abilities.push(sac_for(vec![Effect::Draw { who: Sel::You, n: Amount::Fixed { n: 1 }, plain: false }], Some("{2}"), false));
    v.push(clue);
    let mut food = token("Food", &[CardType::Artifact], &["Food"], &[], None);
    food.abilities.push(sac_for(vec![Effect::GainLife { who: Sel::You, n: Amount::Fixed { n: 3 } }], Some("{2}"), true));
    v.push(food);
    let mut blood = token("Blood", &[CardType::Artifact], &["Blood"], &[], None);
    blood.abilities.push(Ability::Activated {
        cost: vec![Cost::Mana { cost: "{1}".into() }, Cost::Tap, Cost::Discard { filter: (Filter::default()).into(), n: 1, random: false }, Cost::SacrificeSelf],
        targets: vec![],
        effects: vec![Effect::Draw { who: Sel::You, n: Amount::Fixed { n: 1 }, plain: false }],
        timing: Timing::Instant,
        loyalty: None,
        limit_per_turn: None, once: false,
        condition: None,
        from: None, power_up: false, special_action: false,
    });
    v.push(blood);
    let mut powerstone = token("Powerstone", &[CardType::Artifact], &["Powerstone"], &[], None);
    powerstone.abilities.push(Ability::Activated {
        cost: vec![Cost::Tap],
        targets: vec![],
        effects: vec![Effect::AddMana {
            until_end_of_combat: false,
            until_end_of_turn: false,
            // "This mana can't be spent to cast a nonartifact spell": artifact spells and any ability
            mana: vec![ManaProduce::Restricted { m: ManaType::C, n: 1, spell: Some((Filter { types: vec![CardType::Artifact], ..Default::default() }).into()), ability: Some((Filter::default()).into()), amount: None, rider: None, cost_c: false, upkeep: false, unlock: false, chosen: false, no_generic: false }],
            who: None,
            player_choice: false,
        }],
        timing: Timing::Instant,
        loyalty: None,
        limit_per_turn: None, once: false,
        condition: None,
        from: None, power_up: false, special_action: false,
    });
    v.push(powerstone);
    v
}
