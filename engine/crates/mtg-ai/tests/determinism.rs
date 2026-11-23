use mtg_ai::*;
use mtg_engine::*;
use rand::SeedableRng;
use std::sync::{Arc, OnceLock};

fn db() -> Arc<CardDb> {
    static DB: OnceLock<Arc<CardDb>> = OnceLock::new();
    DB.get_or_init(|| mtg_cards::build_db(&mtg_cards::default_cards_dir()).unwrap()).clone()
}

fn setup(seed: u64, players: usize) -> (Config, Vec<DeckSpec>) {
    let db = db();
    let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(seed);
    let decks = (0..players).map(|i| DeckSpec { name: format!("P{i}"), main: fuzz::random_deck(&db, &mut rng, 40), commanders: vec![], sideboard: vec![], companion: None }).collect();
    (Config::standard(), decks)
}

fn play(seed: u64, players: usize) -> (Game, GameRecord) {
    let (cfg, decks) = setup(seed, players);
    let mut g = Game::new(db(), cfg.clone(), decks.clone(), seed);
    let mut agents: Vec<Box<dyn Agent>> = (0..players).map(|i| Box::new(RandomAgent::new(seed + i as u64)) as Box<dyn Agent>).collect();
    // a random agent can pump a ping-pong ability (Viscid Lemures) until the stack is huge: cap the game, replay still must match
    play_game(&mut g, &mut agents, 2_500, |_| Ok(())).unwrap();
    g.run(); // a replay always settles to the next decision, so settle the original too (a capped game stops mid-step)
    let rec = GameRecord { cfg, decks, seed, log: g.decision_log.clone() };
    (g, rec)
}

#[test]
fn same_seed_same_decisions_same_game() {
    for seed in 1..40u64 {
        let (g, rec) = play(seed, 2 + (seed % 3) as usize);
        let r = rec.replay(db()).unwrap();
        assert_eq!(g.state_hash(), r.state_hash(), "seed {seed} diverged on replay");
        assert_eq!(g.over, r.over, "seed {seed}");
        assert_eq!(g.turn, r.turn);
    }
}

#[test]
fn undo_by_replaying_fewer_decisions() {
    let (g, rec) = play(7, 2);
    let n = rec.log.len();
    assert!(n > 10);
    let undone = rec.undo(3).replay(db()).unwrap();
    assert_eq!(undone.decision_log.len(), n - 3);
    // replaying the three decisions again returns to the final state
    let mut again = undone;
    again.replay_onto(&rec.log[n - 3..]).unwrap();
    assert_eq!(again.state_hash(), g.state_hash());
}

#[test]
fn clone_is_independent() {
    let (cfg, decks) = setup(11, 2);
    let mut a = Game::new(db(), cfg, decks, 11);
    let mut agents: Vec<Box<dyn Agent>> = vec![Box::new(RandomAgent::new(1)), Box::new(RandomAgent::new(2))];
    // play a few decisions, clone, advance the clone, original stays put
    for _ in 0..30 {
        if a.run() == Status::Over {
            break;
        }
        let d = a.decision().cloned().unwrap();
        let c = agents[d.player as usize].decide(&a, &d);
        a.apply(c).unwrap();
    }
    let before = a.state_hash();
    let mut b = a.clone();
    for _ in 0..40 {
        if b.run() == Status::Over {
            break;
        }
        let d = b.decision().cloned().unwrap();
        let c = agents[d.player as usize].decide(&b, &d);
        b.apply(c).unwrap();
    }
    assert_eq!(a.state_hash(), before);
    assert_ne!(b.state_hash(), before);
}
