//! `aibench [games] [players]` - plays full games with every seat controlled by the AI and checks the speed targets:
//! no decision may take longer than the 2 s cap, and nearly all must be near-instant. Exits non-zero on a miss (CI gate).

use mtg_ai::*;
use mtg_engine::*;
use rand::SeedableRng;
use std::time::{Duration, Instant};

fn main() {
    // deeply nested card definitions need more than rayon's default 2 MiB worker stack
    let _ = rayon::ThreadPoolBuilder::new().stack_size(64 << 20).build_global();
    let args: Vec<String> = std::env::args().collect();
    let games: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(20);
    let players: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(4);
    let db = mtg_cards::build_db(&mtg_cards::default_cards_dir()).unwrap_or_else(|e| {
        eprintln!("card errors: {e:?}");
        std::process::exit(2)
    });
    if std::env::var_os("FUZZ_PROF").is_some() {
        std::thread::spawn(|| loop {
            std::thread::sleep(Duration::from_secs(5));
            eprintln!("--- prof\n{}", mtg_engine::prof::report());
        });
    }
    let mut times: Vec<Duration> = vec![];
    let mut trivial: Vec<Duration> = vec![];
    for seed in 0..games {
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(seed + 77);
        let decks: Vec<DeckSpec> = (0..players).map(|i| DeckSpec { name: format!("P{i}"), main: fuzz::random_deck(&db, &mut rng, 60), commanders: vec![], sideboard: vec![], companion: None }).collect();
        let mut g = Game::new(db.clone(), Config::standard(), decks, seed);
        // "Hard" budgets everywhere: the worst case the server will see
        let mut ais: Vec<HeuristicAgent> = (0..players).map(|i| HeuristicAgent::new(seed * 13 + i as u64, Duration::from_millis(1200))).collect();
        g.max_steps = 200_000;
        let mut n = 0;
        while g.run() != Status::Over && n < 30_000 {
            let d = g.decision().cloned().unwrap();
            // forced choices: nothing to decide, so no thinking should happen
            let trivial_q = matches!(&d.kind, DecisionKind::Priority { actions, .. } if actions.is_empty())
                || matches!(&d.kind, DecisionKind::Pick { options, min, .. } if options.len() <= 1 && *min as usize >= options.len())
                || matches!(&d.kind, DecisionKind::Blockers { options } if options.is_empty())
                || matches!(&d.kind, DecisionKind::Attackers { options, .. } if options.is_empty());
            let t = Instant::now();
            let c = ais[d.player as usize].decide(&g, &d);
            let el = t.elapsed();
            times.push(el);
            if trivial_q {
                trivial.push(el);
            }
            if g.apply(c).is_err() {
                g.apply(Choice::Default).unwrap();
            }
            n += 1;
        }
        if g.steps_taken > g.max_steps {
            eprintln!("game {seed} hit the engine step limit at turn {} step {:?}; last events: {:?}", g.turn, g.step, g.events.iter().rev().take(14).collect::<Vec<_>>());
            eprintln!("battlefield: {:?} stack {}", g.battlefield.iter().map(|c| g.card_name(*c).to_string()).collect::<Vec<_>>(), g.stack.len());
        }
    }
    times.sort();
    trivial.sort();
    let pct = |v: &Vec<Duration>, p: f64| v.get(((v.len() as f64 * p) as usize).min(v.len().saturating_sub(1))).copied().unwrap_or_default();
    println!(
        "{} decisions over {games} {players}-player games: p50 {:?}, p95 {:?}, p99 {:?}, max {:?}; trivial decisions p99 {:?}",
        times.len(),
        pct(&times, 0.5),
        pct(&times, 0.95),
        pct(&times, 0.99),
        times.last().copied().unwrap_or_default(),
        pct(&trivial, 0.99)
    );
    let mut bad = vec![];
    if times.last().copied().unwrap_or_default() > Duration::from_secs(2) {
        bad.push("a decision took longer than the 2 s cap");
    }
    if pct(&times, 0.95) > Duration::from_millis(250) {
        bad.push("p95 decision time above 250 ms");
    }
    if pct(&trivial, 0.99) > Duration::from_millis(5) {
        bad.push("trivial decisions slower than 5 ms at p99");
    }
    for b in &bad {
        eprintln!("FAIL: {b}");
    }
    if !bad.is_empty() {
        std::process::exit(1);
    }
    println!("speed targets met");
}
