use mtg_ai::*;
use mtg_engine::*;
fn main() {
    let db = mtg_cards::build_db(&mtg_cards::default_cards_dir()).unwrap();
    let mut rng = <rand_chacha::ChaCha8Rng as rand::SeedableRng>::seed_from_u64(5);
    let decks: Vec<DeckSpec> = (0..2).map(|i| DeckSpec { name: format!("P{i}"), main: mtg_ai::fuzz::random_deck(&db, &mut rng, 40), commanders: vec![], sideboard: vec![], companion: None }).collect();
    let mut g = Game::new(db, Config::standard(), decks, 5);
    let mut agents: Vec<Box<dyn Agent>> = vec![Box::new(RandomAgent::new(1)), Box::new(RandomAgent::new(2))];
    let out = play_game(&mut g, &mut agents, 20000, |_| Ok(()));
    println!("{:?} turn={} step={:?} run={:?} over={:?} pending={:?}", out, g.turn, g.step, std::mem::discriminant(&g.run), g.over, g.pending.as_ref().map(|d| &d.kind));
    for e in g.events.iter().take(30) { println!("{e:?}"); }
    println!("steps_taken={} max={}", g.steps_taken, g.max_steps);
}
