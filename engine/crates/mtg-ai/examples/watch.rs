use mtg_ai::*;
use mtg_engine::*;
use rand::SeedableRng;
use std::time::Duration;

fn main() {
    let seed: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    let db = mtg_cards::build_db(&mtg_cards::default_cards_dir()).unwrap();
    let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(seed + 1000);
    let decks: Vec<DeckSpec> = (0..2).map(|i| DeckSpec { name: format!("P{i}"), main: fuzz::random_deck(&db, &mut rng, 40), commanders: vec![], sideboard: vec![], companion: None }).collect();
    let mut g = Game::new(db.clone(), Config::standard(), decks, seed);
    let mut ai0 = HeuristicAgent::new(1, Duration::from_millis(200));
    let mut ai1 = RandomAgent::new(2);
    let name = |g: &Game, c: CardId| g.def_of(c).name.clone();
    let mut n = 0;
    loop {
        if g.run() == Status::Over || n > 5000 { break; }
        for e in g.drain_events() {
            match e {
                Event::TurnBegin { turn, player } => println!("--- turn {turn} (player {player}) life {:?}", g.players.iter().map(|p| p.life).collect::<Vec<_>>()),
                Event::Cast { card, player } => println!("  P{player} casts {}", name(&g, card)),
                Event::Moved { card, from, to } if matches!((from, to), (types::Zone::Hand, types::Zone::Battlefield)) => println!("  land/perm {} enters", name(&g, card)),
                Event::Moved { card, from: types::Zone::Battlefield, to: types::Zone::Graveyard } => println!("  {} dies", name(&g, card)),
                Event::Attacked { card, .. } => println!("  attacks: {}", name(&g, card)),
                Event::Blocked { blocker, attacker } => println!("  {} blocks {}", name(&g, blocker), name(&g, attacker)),
                Event::Damage { target: Obj::Player { id }, amount, .. } => println!("  P{id} takes {amount}"),
                _ => {}
            }
        }
        let d = g.decision().cloned().unwrap();
        let c = if d.player == 0 { ai0.decide(&g, &d) } else { ai1.decide(&g, &d) };
        if g.apply(c).is_err() { g.apply(Choice::Default).unwrap(); }
        n += 1;
    }
    println!("RESULT {:?} after {} turns", g.over, g.turn);
}
