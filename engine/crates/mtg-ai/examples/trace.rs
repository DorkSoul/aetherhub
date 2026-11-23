use mtg_ai::*;
use mtg_engine::*;
use rand::SeedableRng;
use std::io::Write;
fn main() {
    let db = mtg_cards::build_db(&mtg_cards::default_cards_dir()).unwrap();
    let seed: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1);
    let players = 2 + (seed % 3) as usize;
    let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(seed);
    let decks: Vec<DeckSpec> = (0..players).map(|i| DeckSpec { name: format!("P{i}"), main: fuzz::random_deck(&db, &mut rng, 40), commanders: vec![], sideboard: vec![], companion: None }).collect();
    let mut g = Game::new(db.clone(), Config::standard(), decks, seed);
    let mut agents: Vec<Box<dyn Agent>> = (0..players).map(|i| Box::new(RandomAgent::new(seed + i as u64)) as Box<dyn Agent>).collect();
    let mut f = std::fs::File::create(std::env::args().nth(2).unwrap()).unwrap();
    let mut n = 0;
    loop {
        for e in g.drain_events() { if let Event::Cast{card,player}=e { writeln!(f, "  cast {} by {}", g.card_name(card), player).unwrap(); } }
        writeln!(f, "run turn {} step {:?} stack {} n={}", g.turn, g.step, g.stack.len(), n).unwrap(); f.flush().unwrap();
        let t0 = std::time::Instant::now();
        let st = g.run();
        writeln!(f, "  run took {:?} cards={} bf={} stack={} steps={}", t0.elapsed(), g.cards.len(), g.battlefield.len(), g.stack.len(), g.steps_taken).unwrap(); f.flush().unwrap();
        if st == Status::Over { break; }
        let d = g.decision().cloned().unwrap();
        writeln!(f, "  decision p{} {:?}", d.player, std::mem::discriminant(&d.kind)).unwrap(); f.flush().unwrap();
        let c = agents[d.player as usize].decide(&g, &d);
        if let (Choice::Attackers{attacks}, true) = (&c, n>=70) { writeln!(f, "  ATTACK {:?} bf={:?}", attacks.iter().map(|(c,_)| g.card_name(*c)).collect::<Vec<_>>(), g.battlefield.iter().map(|c| g.card_name(*c)).collect::<Vec<_>>()).unwrap(); f.flush().unwrap(); }
        if g.apply(c).is_err() { g.apply(Choice::Default).unwrap(); }
        n += 1;
        if n > 400 { break; }
    }
}
