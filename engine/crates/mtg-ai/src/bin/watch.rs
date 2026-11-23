//! `watch <seed> <players> <kind> [deck index]` - one mirrored game with a per-turn summary (life, creatures, hand, lands) and every attack declaration.
use mtg_ai::*;
use mtg_engine::types::*;
use mtg_engine::*;

fn main() {
    let _ = rayon::ThreadPoolBuilder::new().stack_size(64 << 20).build_global();
    let a: Vec<String> = std::env::args().collect();
    let seed: u64 = a[1].parse().unwrap();
    let players: usize = a[2].parse().unwrap();
    let kind = &a[3];
    let max_turn: u32 = std::env::var("MAXTURN").ok().and_then(|v| v.parse().ok()).unwrap_or(60);
    let db = mtg_cards::build_db(&mtg_cards::default_cards_dir()).unwrap();
    std::env::set_var("FUZZ_DECK_DIR", format!("{}/decks/fuzz", std::env::var("AETHERHUB_ROOT").unwrap_or_else(|_| ".".into())));
    let real = fuzz::real_decks(&db);
    let deck = real[a.get(4).and_then(|s| s.parse::<usize>().ok()).unwrap_or(seed as usize) % real.len()].clone();
    let specs: Vec<DeckSpec> = (0..players).map(|i| DeckSpec { name: format!("P{i}"), main: deck.clone(), commanders: vec![], sideboard: vec![], companion: None }).collect();
    let mut g = Game::new(db.clone(), Config::standard(), specs, seed);
    let mut agents: Vec<Box<dyn Agent>> = (0..players).map(|s| make_agent(kind, seed * 131 + s as u64).unwrap()).collect();
    let mut last = 0;
    loop {
        if g.run() == Status::Over || g.turn > max_turn {
            break;
        }
        let d = g.decision().cloned().unwrap();
        if g.turn != last {
            last = g.turn;
            let mut line = format!("T{:<3} p{} |", g.turn, g.active);
            for p in 0..players {
                let pl = g.player(p as PlayerId);
                let (mut cr, mut ln, mut pw) = (0, 0, 0);
                for &id in &g.battlefield {
                    let c = g.card(id);
                    if c.cur.controller == p as PlayerId {
                        if c.cur.types.has(CardType::Creature) { cr += 1 }
                        if c.cur.types.has(CardType::Land) { ln += 1 }
                        if c.cur.types.has(CardType::Planeswalker) { pw += 1 }
                    }
                }
                line += &format!(" p{p}: {}{} life {} cr {} ln {} h {} |", if pl.lost { "X " } else { "" }, if pw > 0 { "pw " } else { "" }, pl.life, cr, ln, pl.hand.len());
            }
            println!("{line}");
        }
        let c = agents[d.player as usize].decide(&g, &d);
        if let (DecisionKind::Attackers { options, .. }, Choice::Attackers { attacks }) = (&d.kind, &c) {
            println!("      p{} attackers: {} of {} possible {:?}", d.player, attacks.len(), options.len(), attacks.iter().map(|(c, o)| format!("{}->{:?}", g.def_of(*c).name, o)).collect::<Vec<_>>());
        }
        if g.apply(c).is_err() {
            g.apply(Choice::Default).unwrap();
        }
    }
    println!("result {:?} at turn {}", g.over, g.turn);
}
