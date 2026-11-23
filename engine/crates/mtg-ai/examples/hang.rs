use mtg_ai::*;
use mtg_engine::*;
use rand::SeedableRng;
use std::io::Write;
fn main() {
    let db = mtg_cards::build_db(&mtg_cards::default_cards_dir()).unwrap();
    let from: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1);
    for seed in from..from + 40 {
        let players = 2 + (seed % 3) as usize;
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(seed);
        let decks: Vec<DeckSpec> = (0..players).map(|i| DeckSpec { name: format!("P{i}"), main: fuzz::random_deck(&db, &mut rng, 40), commanders: vec![], sideboard: vec![], companion: None }).collect();
        eprint!("seed {seed} ({players}p) ");
        std::io::stderr().flush().unwrap();
        let mut g = Game::new(db.clone(), Config::standard(), decks, seed);
        let mut agents: Vec<Box<dyn Agent>> = (0..players).map(|i| Box::new(RandomAgent::new(seed + i as u64)) as Box<dyn Agent>).collect();
        let out = play_game(&mut g, &mut agents, 100_000, |_| Ok(())).unwrap();
        eprintln!("-> {} decisions, turn {}", out.decisions, g.turn);
    }
}
