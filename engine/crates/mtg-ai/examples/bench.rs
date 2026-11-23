use mtg_ai::*;
use mtg_engine::*;
use rand::SeedableRng;
use std::time::Instant;

fn main() {
    let db = mtg_cards::build_db(&mtg_cards::default_cards_dir()).unwrap();
    // 1. random playout speed (no invariant checks)
    let t = Instant::now();
    let mut decisions = 0u64;
    let mut games = 0;
    for seed in 0..300u64 {
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(seed);
        let decks: Vec<DeckSpec> = (0..2).map(|i| DeckSpec { name: format!("P{i}"), main: fuzz::random_deck(&db, &mut rng, 40), commanders: vec![], sideboard: vec![], companion: None }).collect();
        let mut g = Game::new(db.clone(), Config::standard(), decks, seed);
        let mut agents: Vec<Box<dyn Agent>> = vec![Box::new(RandomAgent::new(seed)), Box::new(RandomAgent::new(seed + 1))];
        let out = play_game(&mut g, &mut agents, 100_000, |_| Ok(())).unwrap();
        decisions += out.decisions;
        games += 1;
    }
    let el = t.elapsed().as_secs_f64();
    println!("{games} games, {decisions} decisions in {el:.2}s => {:.0} decisions/s, {:.2} ms/game", decisions as f64 / el, el * 1000.0 / games as f64);

    // 2. clone speed on a mid-game state
    let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(99);
    let decks: Vec<DeckSpec> = (0..4).map(|i| DeckSpec { name: format!("P{i}"), main: fuzz::random_deck(&db, &mut rng, 60), commanders: vec![], sideboard: vec![], companion: None }).collect();
    let mut g = Game::new(db.clone(), Config::standard(), decks, 99);
    let mut agents: Vec<Box<dyn Agent>> = (0..4).map(|i| Box::new(RandomAgent::new(i)) as Box<dyn Agent>).collect();
    for _ in 0..400 {
        if g.run() == Status::Over { break; }
        let d = g.decision().cloned().unwrap();
        let c = agents[d.player as usize].decide(&g, &d);
        g.apply(c).unwrap();
    }
    let t = Instant::now();
    let n = 20000;
    let mut keep = 0usize;
    for _ in 0..n {
        let c = g.clone();
        keep += c.cards.len();
    }
    println!("clone: {:.2} us ({} cards, turn {}) [{keep}]", t.elapsed().as_secs_f64() * 1e6 / n as f64, g.cards.len(), g.turn);
    // 3. legal_actions speed
    let t = Instant::now();
    let mut k = 0;
    for _ in 0..20000 { k += g.legal_actions(g.active).len(); }
    println!("legal_actions: {:.2} us [{k}]", t.elapsed().as_secs_f64() * 1e6 / 20000.0);
}
