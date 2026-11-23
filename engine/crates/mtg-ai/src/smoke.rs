//! Smoke test for one card: put it in several situations and let random players hammer on it. It must never crash,
//! hang, or leave the game in an inconsistent state. Cheap enough to run for every card.

use crate::agent::Agent;
use mtg_engine::types::*;
use mtg_engine::*;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

#[derive(Debug, Clone, Default)]
pub struct SmokeReport {
    pub runs: u32,
    /// The card was cast, activated or entered the battlefield in at least one run.
    pub exercised: bool,
    pub decisions: u64,
}

/// Prefers actions that involve the subject card; otherwise random.
struct Subject {
    rng: ChaCha8Rng,
    name: String,
    /// The other face's name (a split / double-faced card is named for the face it was cast as).
    alt: Option<String>,
}

impl Agent for Subject {
    fn decide(&mut self, game: &Game, d: &Decision) -> Choice {
        if let DecisionKind::Priority { actions, .. } = &d.kind {
            let mine: Vec<usize> = actions
                .iter()
                .enumerate()
                .filter(|(_, a)| match a {
                    Action::Cast { card, .. } | Action::Activate { card, .. } | Action::PlayLand { card } => game.card_name(*card) == self.name || Some(game.card_name(*card).to_string()) == self.alt,
                    _ => false,
                })
                .map(|(i, _)| i)
                .collect();
            if let Some(i) = mine.choose(&mut self.rng) {
                return Choice::Priority { action: Some(*i) };
            }
            if let Some(i) = actions.iter().position(|a| matches!(a, Action::PlayLand { .. })) {
                return Choice::Priority { action: Some(i) };
            }
            if actions.is_empty() || self.rng.gen_bool(0.4) {
                return Choice::Priority { action: None };
            }
            return Choice::Priority { action: Some(self.rng.gen_range(0..actions.len())) };
        }
        crate::random::RandomAgent::new(self.rng.gen()).decide(game, d)
    }
}

/// The opponent: casts what it can (so the subject has spells to answer and permanents to target), otherwise random.
struct Hostile {
    rng: ChaCha8Rng,
    /// Always attacks with everything (cards that answer an attacking creature).
    aggressive: bool,
}

impl Agent for Hostile {
    fn decide(&mut self, game: &Game, d: &Decision) -> Choice {
        if let DecisionKind::Priority { actions, .. } = &d.kind {
            let casts: Vec<usize> = actions.iter().enumerate().filter(|(_, a)| matches!(a, Action::Cast { .. } | Action::Activate { .. })).map(|(i, _)| i).collect();
            if !casts.is_empty() && self.rng.gen_bool(0.7) {
                return Choice::Priority { action: casts.choose(&mut self.rng).copied() };
            }
        }
        if self.aggressive {
            if let DecisionKind::Attackers { options, .. } = &d.kind {
                let attacks = options.iter().filter_map(|o| o.players.first().map(|p| (o.card, Obj::player(*p)))).collect();
                return Choice::Attackers { attacks };
            }
        }
        crate::random::RandomAgent::new(self.rng.gen()).decide(game, d)
    }
}

/// States the random boards do not build: face-down creatures, Heroes and Villains in the graveyard, Time Lords, Doctors, spells that target lands,
/// Arcane spells for splice, and the opponent's attackers.
fn special_setup(g: &mut Game, db: &Arc<CardDb>, name: &str) {
    let mut add = |g: &mut Game, n: &str, p: u8, z: Zone| {
        if db.find(n).is_some() {
            g.add_card(n, p, z);
        }
    };
    match name {
        "Break Open" => {
            if db.find("Grizzly Bears").is_some() {
                let id = g.add_card("Grizzly Bears", 1, Zone::Battlefield);
                g.cards[id.0 as usize].face_down = true;
                g.reset_derived(id);
            }
        }
        "Decoy Ploy" => {
            for (n, z) in [("Hela, Mistress of Darkness", Zone::Graveyard), ("Magik, Illyana Rasputin", Zone::Graveyard), ("Hela, Mistress of Darkness", Zone::Graveyard), ("Magik, Illyana Rasputin", Zone::Graveyard)] {
                add(g, n, 0, z);
            }
        }
        "Time Lord Regeneration" => {
            add(g, "The Thirteenth Doctor", 0, Zone::Battlefield);
            add(g, "Rassilon, the War President", 0, Zone::Battlefield);
            add(g, "The Third Doctor", 0, Zone::Library);
            add(g, "Missy", 0, Zone::Library);
            add(g, "Lightning Bolt", 1, Zone::Hand);
            add(g, "Lightning Bolt", 1, Zone::Hand);
            add(g, "Terror", 1, Zone::Hand);
        }
        "Twice Upon a Time // Unlikely Meeting" | "Twice Upon a Time" => {
            add(g, "The Third Doctor", 0, Zone::Battlefield);
            add(g, "The Fifteenth Doctor", 0, Zone::Battlefield);
            add(g, "The Thirteenth Doctor", 0, Zone::Library);
        }
        "Teferi's Response" => {
            for n in ["Stone Rain", "Stone Rain", "Ice Storm", "Wasteland", "Sinkhole"] {
                add(g, n, 1, Zone::Hand);
            }
            add(g, "Wasteland", 1, Zone::Battlefield);
        }
        "Evermind" => {
            for n in ["Reach Through Mists", "Reach Through Mists", "Blessed Breath", "Glacial Ray"] {
                add(g, n, 0, Zone::Hand);
            }
        }
        _ => {}
    }
}

fn setup(db: &Arc<CardDb>, name: &str, zone: Zone, seed: u64, variant: u64) -> Game {
    let mut g = Game::new_blank(db.clone(), 2, seed);
    for p in 0..2u8 {
        for l in ["Plains", "Island", "Swamp", "Mountain", "Forest"] {
            for _ in 0..5 {
                g.add_card(l, p, Zone::Battlefield);
            }
        }
        for _ in 0..25 {
            g.add_card(["Plains", "Island", "Swamp", "Mountain", "Forest"][(seed as usize + p as usize) % 5], p, Zone::Library);
        }
    }
    g.add_card("Grizzly Bears", 0, Zone::Battlefield);
    g.add_card("Hill Giant", 1, Zone::Battlefield);
    g.add_card("Serra Angel", 1, Zone::Battlefield);
    // something in graveyards and hands for effects that look there
    g.add_card("Grizzly Bears", 0, Zone::Graveyard);
    g.add_card("Lightning Bolt", 1, Zone::Graveyard);
    g.add_card("Divination", 0, Zone::Hand);
    // targets and things to answer for the subject: an artifact, an enchantment, a planeswalker, and spells in the opponent's hand
    // richer boards for conditional cards: snow land, tapped creatures' sources, triggers and activated abilities to answer, legends and artifacts in graveyards
    for (n, p, z) in [("Snow-Covered Forest", 1, Zone::Battlefield), ("Prodigal Sorcerer", 1, Zone::Battlefield), ("Elvish Visionary", 1, Zone::Hand), ("Isamaru, Hound of Konda", 0, Zone::Graveyard),
        ("Sol Ring", 0, Zone::Graveyard), ("Glorious Anthem", 0, Zone::Graveyard), ("Hill Giant", 1, Zone::Graveyard), ("Sol Ring", 1, Zone::Graveyard), ("Mountain", 1, Zone::Graveyard), ("Pacifism", 1, Zone::Hand), ("Pacifism", 0, Zone::Hand)] {
        if db.find(n).is_some() {
            g.add_card(n, p, z);
        }
    }
    for (n, p, z) in [("Sol Ring", 1, Zone::Battlefield), ("Glorious Anthem", 1, Zone::Battlefield), ("Sol Ring", 0, Zone::Battlefield), ("Lightning Bolt", 1, Zone::Hand), ("Grizzly Bears", 1, Zone::Hand), ("Divination", 1, Zone::Hand), ("Sol Ring", 1, Zone::Hand), ("Counterspell", 1, Zone::Hand)] {
        if db.find(n).is_some() {
            g.add_card(n, p, z);
        }
    }
    let tys = db.find(name).map(|id| db.def(id).types);
    if tys.map_or(false, |t| t.has(CardType::Plane) || t.has(CardType::Phenomenon)) {
        // Planechase: the subject is the face-up plane, with more copies in the planar deck (planar die rolls and
        // planeswalks keep turning it up again); a phenomenon starts in the planar deck under a plane-free top
        let pid = db.find(name).unwrap();
        let mut deck = vec![];
        for _ in 0..4 {
            deck.push(pid);
        }
        g.setup_variants(&deck, &[]);
    } else if tys.map_or(false, |t| t.has(CardType::Scheme)) {
        // Archenemy: player 0 is the archenemy with a scheme deck of the subject
        let sid = db.find(name).unwrap();
        g.setup_variants(&[], &[vec![sid; 4], vec![]]);
    } else if db.find(name).map_or(false, |id| db.def(id).types.has(CardType::Land) && zone == Zone::Hand) {
        g.add_card(name, 0, Zone::Hand);
    } else {
        g.add_card(name, 0, zone);
    }
    // later variants (only run for cards the first ones never exercised): random garnish from a pool of varied cards, so conditional cards find
    // coloured, legendary, tapped-able, artifact, flying and hybrid things to act on
    if variant > 0 {
        let mut rng = ChaCha8Rng::seed_from_u64(seed * 7919 + variant);
        const POOL: &[&str] = &["Llanowar Elves", "Wall of Omens", "Wall of Stone", "Serra Angel", "Air Elemental", "Vampire Nighthawk", "Gray Ogre", "Giant Spider", "Ornithopter", "Juggernaut", "Isamaru, Hound of Konda", "Thalia, Guardian of Thraben",
            "Black Knight", "White Knight", "Mogg Fanatic", "Phyrexian Walker", "Shivan Dragon", "Mahamoti Djinn", "Gravedigger", "Walking Ballista", "Pacifism", "Glorious Anthem", "Bonesplitter", "Sol Ring", "Wayfarer's Bauble", "Giant Growth",
            "Counterspell", "Lightning Bolt", "Terror", "Doom Blade", "Divination", "Opt", "Test Walker", "Kor Skyfisher", "Birds of Paradise", "Wrath of God", "Bear Umbra", "Rancor", "Treetop Village", "Snow-Covered Island", "Basalt Monolith", "Shock", "Elvish Visionary", "Mulldrifter", "Dragon Egg", "Sakura-Tribe Elder", "Eternal Witness", "Faerie Conclave",
            "Skyknight Legionnaire", "Mystic Snake", "Vulturous Zombie", "Lightning Helix", "Electrolyze", "Hill Giant", "Hill Giant", "Vampire Nighthawk", "Vampire Nighthawk", "Kithkin Greatheart", "Merfolk Looter", "Spectral Procession",
            "Llanowar Elves", "Elvish Mystic", "Fyndhorn Elves", "Wood Elves", "Order of Leitbur", "Cloudgoat Ranger", "Lotus Cobra", "Stitcher's Supplier", "Spirit of the Hearth", "Dryad Arbor", "Mana Tithe", "Memnite",
            "Reach Through Mists", "Think Twice", "Lu Bu, Master-at-Arms", "Exalted Angel", "Ancient Grudge", "Swords to Plowshares", "Lingering Souls", "Kithkin Spellslinger"];
        let present: Vec<&str> = POOL.iter().copied().filter(|n| db.find(n).is_some()).collect();
        for _ in 0..16 {
            let n = present[rng.gen_range(0..present.len())];
            let (p, z) = [(0u8, Zone::Battlefield), (1, Zone::Battlefield), (0, Zone::Graveyard), (1, Zone::Graveyard), (1, Zone::Hand), (0, Zone::Hand), (1, Zone::Battlefield)][rng.gen_range(0..7)];
            g.add_card(n, p, z);
        }
    }
    special_setup(&mut g, db, name);
    for p in g.players.iter_mut() {
        p.full_control = true;
        p.auto_skip = false;
    }
    // odd variants: the opponent's turn, so the subject answers spells and attacks instead of acting first
    g.start_at(if variant % 2 == 1 { 1 } else { 0 }, Step::Main1);
    g
}

/// Run the smoke test for a card. Returns an error describing the first problem.
pub fn smoke_card(db: &Arc<CardDb>, name: &str) -> Result<SmokeReport, String> {
    let Some(id) = db.find(name) else { return Err(format!("unknown card {name}")) };
    let def = db.def(id);
    let mut zones = vec![Zone::Hand, Zone::Battlefield];
    if def.is_permanent() || def.types.has(CardType::Instant) || def.types.has(CardType::Sorcery) {
        zones.push(Zone::Graveyard);
    }
    let mut report = SmokeReport::default();
    // variant 0 is the plain board; if the card was never exercised there, richer random boards are tried until it is
    for variant in 0..=24u64 {
      if variant > 0 && report.exercised {
          break;
      }
      for (zi, zone) in zones.iter().enumerate() {
        if *zone == Zone::Battlefield && !def.is_permanent() {
            continue;
        }
        for k in 0..3u64 {
            let seed = 1000 + zi as u64 * 10 + k + variant * 100;
            let db2 = db.clone();
            let nm = name.to_string();
            let alt = def.other_face.map(|b| db.def(b).name.to_string());
            let subject_def = id;
            let z = *zone;
            let r = catch_unwind(AssertUnwindSafe(move || -> Result<(bool, u64), String> {
                let mut g = setup(&db2, &nm, z, seed, variant);
                let mut agents: Vec<Box<dyn Agent>> = vec![
                    Box::new(Subject { rng: ChaCha8Rng::seed_from_u64(seed), name: nm.clone(), alt: alt.clone() }),
                    Box::new(Hostile { rng: ChaCha8Rng::seed_from_u64(seed + 99), aggressive: nm == "Order" }),
                ];
                let mut exercised = false;
                let mut n = 0u64;
                let mut guard = 0u64;
                while n < 700 {
                    if g.run() == Status::Over {
                        break;
                    }
                    let d = g.decision().cloned().ok_or("run() waited without a decision")?;
                    let c = agents[d.player as usize].decide(&g, &d);
                    if let Err(e) = g.apply(c) {
                        g.apply(Choice::Default).map_err(|e2| format!("default choice rejected for {:?}: {e2} (after {e})", d.kind))?;
                    }
                    for e in g.drain_events() {
                        match e {
                            Event::Cast { card, .. } | Event::Activated { card, .. } if g.card_name(card) == nm || Some(g.card_name(card).to_string()) == alt => exercised = true,
                            Event::Moved { card, to: Zone::Battlefield, .. } if g.card_name(card) == nm || Some(g.card_name(card).to_string()) == alt => exercised = true,
                            // cards that start on the battlefield (meld results) count once they attack or one of their abilities triggers
                            Event::Attacked { card, .. } if g.card_name(card) == nm || Some(g.card_name(card).to_string()) == alt => exercised = true,
                            Event::AbilityTriggered { source, .. } if g.card_name(source) == nm || Some(g.card_name(source).to_string()) == alt => exercised = true,
                            _ => {}
                        }
                    }
                    // a splice card is never cast itself: it counts once it has been spliced onto a spell on the stack
                    if !exercised && g.stack.iter().any(|it| it.spliced.contains(&subject_def)) {
                        exercised = true;
                    }
                    g.check_invariants()?;
                    n += 1;
                    guard += 1;
                    if guard > 5000 {
                        return Err("too many steps".into());
                    }
                }
                Ok((exercised, n))
            }));
            match r {
                Ok(Ok((ex, n))) => {
                    report.runs += 1;
                    report.exercised |= ex;
                    report.decisions += n;
                }
                Ok(Err(e)) => return Err(format!("{name} ({zone:?}, seed {seed}): {e}")),
                Err(p) => {
                    let msg = p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_else(|| "panic".into());
                    return Err(format!("{name} ({zone:?}, seed {seed}): panic: {msg}"));
                }
            }
        }
      }
    }
    Ok(report)
}
