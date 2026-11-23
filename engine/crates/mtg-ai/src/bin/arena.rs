//! `arena [games] [budget_ms] [players]` - heuristic AI (seat rotates) against random players; prints win rate and think times.

use mtg_ai::*;
use mtg_engine::*;
use rand::SeedableRng;
use std::time::{Duration, Instant};

fn main() {
    // deeply nested card definitions need more than rayon's default 2 MiB worker stack
    let _ = rayon::ThreadPoolBuilder::new().stack_size(64 << 20).build_global();
    let args: Vec<String> = std::env::args().collect();
    let games: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(100);
    let budget: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(200);
    let players: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(2);
    let db = mtg_cards::build_db(&mtg_cards::default_cards_dir()).unwrap();
    let (mut wins, mut draws, mut total_decisions) = (0u64, 0u64, 0u64);
    let mut think_total = Duration::ZERO;
    let mut think_max = Duration::ZERO;
    let mut think_n = 0u64;
    let mut over_1s = 0u64;
    let mut illegal: std::collections::BTreeMap<String, u64> = Default::default();
    let mut sims = 0u64;
    let t = Instant::now();
    for seed in 0..games {
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(seed + 1000);
        let decks: Vec<DeckSpec> =
            (0..players).map(|i| DeckSpec { name: format!("P{i}"), main: arena_deck(&db, &mut rng), commanders: vec![], sideboard: vec![], companion: None }).collect();
        let mut g = Game::new(db.clone(), Config::standard(), decks, seed);
        let me = (seed as usize % players) as PlayerId;
        let mut ai = HeuristicAgent::new(seed, Duration::from_millis(budget));
        let mut randoms: Vec<RandomAgent> = (0..players).map(|i| RandomAgent::new(seed * 31 + i as u64)).collect();
        let mut n = 0u64;
        loop {
            if g.run() == Status::Over || n > 20_000 {
                break;
            }
            let d = g.decision().cloned().unwrap();
            let c = if d.player == me {
                let t0 = Instant::now();
                let c = ai.decide(&g, &d);
                let el = t0.elapsed();
                think_total += el;
                think_n += 1;
                think_max = think_max.max(el);
                if el > Duration::from_secs(1) {
                    over_1s += 1;
                }
                c
            } else {
                randoms[d.player as usize].decide(&g, &d)
            };
            let ai_seat = d.player as usize == me as usize;
            if g.apply(c).is_err() {
                if ai_seat {
                    let k = format!("{:?}", d.kind);
                    let k = k.split(|ch: char| ch == ' ' || ch == '{' || ch == '(').next().unwrap_or("").to_string();
                    *illegal.entry(k).or_insert(0u64) += 1;
                }
                g.apply(Choice::Default).unwrap();
            }
            n += 1;
        }
        total_decisions += n;
        sims += ai.sims;
        if std::env::var_os("ARENA_VERBOSE").is_some() {
            let pl = g.player(me);
            eprintln!("seed {seed}: turn {} over {:?} me={me} life {} library {} hand {}", g.turn, g.over, pl.life, pl.library.len(), pl.hand.len());
        }
        match &g.over {
            Some(GameResult::Winner { player }) if *player == me => wins += 1,
            Some(GameResult::Draw) | None => draws += 1,
            _ => {}
        }
    }
    println!(
        "{games} games, {players} players: AI won {wins} ({:.0}%), {draws} draws/limit. {:.1}s total, {} decisions, {} sims",
        100.0 * wins as f64 / games as f64,
        t.elapsed().as_secs_f32(),
        total_decisions,
        sims
    );
    println!(
        "AI thinking: {} decisions, avg {:.2} ms, max {:.1} ms, >1s: {}",
        think_n,
        think_total.as_secs_f64() * 1000.0 / think_n.max(1) as f64,
        think_max.as_secs_f64() * 1000.0,
        over_1s
    );
    println!("illegal AI answers (replaced by the default): {illegal:?}");
}

/// `ARENA_CARDS="Card A;Card B"` builds decks from those cards (4 copies each) plus basic lands, to exercise specific cards with the AI.
fn arena_deck(db: &std::sync::Arc<CardDb>, rng: &mut rand_chacha::ChaCha8Rng) -> Vec<DefId> {
    if let Ok(list) = std::env::var("ARENA_CARDS") {
        let mut deck = vec![];
        for n in list.split(';').filter(|n| !n.is_empty()) {
            let id = db.find(n.trim()).unwrap_or_else(|| panic!("unknown card {n}"));
            for _ in 0..4 {
                deck.push(id);
            }
        }
        for b in ["Plains", "Island", "Swamp", "Mountain", "Forest"] {
            let id = db.find(b).unwrap();
            for _ in 0..8 {
                deck.push(id);
            }
        }
        return deck;
    }
    fuzz::random_deck(db, rng, 40)
}
