//! The AI's sampled worlds never contain information its seat could not have.

use mtg_ai::determinize::Belief;
use mtg_engine::types::Zone;
use mtg_engine::*;
use rand::SeedableRng;
use std::sync::{Arc, OnceLock};

fn db() -> Arc<CardDb> {
    static DB: OnceLock<Arc<CardDb>> = OnceLock::new();
    DB.get_or_init(|| mtg_cards::build_db(&mtg_cards::default_cards_dir()).unwrap()).clone()
}

fn deck(db: &CardDb, land: &str, spell: &str) -> Vec<DefId> {
    let (l, s) = (db.find(land).unwrap(), db.find(spell).unwrap());
    (0..40).map(|i| if i < 17 { l } else { s }).collect()
}

fn game() -> Game {
    let db = db();
    let decks = vec![
        DeckSpec { name: "me".into(), main: deck(&db, "Plains", "Savannah Lions"), commanders: vec![], sideboard: vec![], companion: None },
        DeckSpec { name: "opp".into(), main: deck(&db, "Mountain", "Shivan Dragon"), commanders: vec![], sideboard: vec![], companion: None },
    ];
    let mut g = Game::new(db, Config::standard(), decks, 3);
    g.run();
    g
}

#[test]
fn unseen_opponent_cards_are_never_the_real_ones() {
    let g = game();
    let dragon = g.db.find("Shivan Dragon").unwrap();
    let mountain = g.db.find("Mountain").unwrap();
    let belief = Belief::new(&g, 0);
    let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(1);
    for _ in 0..50 {
        let w = belief.sample(&g, &mut rng);
        let opp = &w.players[1];
        assert_eq!(opp.hand.len(), g.players[1].hand.len(), "hand size is public");
        assert_eq!(opp.library.len(), g.players[1].library.len(), "library size is public");
        for &id in opp.hand.iter().chain(opp.library.iter()) {
            let d = w.cards[id.0 as usize].def;
            // nothing was seen of their deck: no Shivan Dragon may appear, and no guessed card is a Mountain either
            assert_ne!(d, dragon, "the opponent's real unseen card leaked into the sample");
            assert_ne!(d, mountain, "the opponent's real land leaked into the sample");
        }
    }
}

#[test]
fn seen_cards_stay_and_are_expected_again() {
    let mut g = game();
    let dragon = g.db.find("Shivan Dragon").unwrap();
    // one card of theirs in hand became known to seat 0 (revealed, or bounced back from the table)
    let known = g.players[1].hand[0];
    g.cards[known.0 as usize].def = dragon;
    g.cards[known.0 as usize].front = dragon;
    g.cards[known.0 as usize].known_to = 1;
    let belief = Belief::new(&g, 0);
    let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(2);
    let mut extra_dragons = 0;
    for _ in 0..50 {
        let w = belief.sample(&g, &mut rng);
        assert_eq!(w.cards[known.0 as usize].def, dragon, "a known card keeps its identity");
        assert_eq!(w.players[1].hand[0], known);
        extra_dragons += w.players[1].library.iter().filter(|&&id| w.cards[id.0 as usize].def == dragon).count();
    }
    assert!(extra_dragons > 0, "a seen card should make more copies plausible");
}

#[test]
fn own_library_keeps_seen_cards_in_place() {
    let mut g = game();
    let top = *g.players[0].library.last().unwrap();
    g.cards[top.0 as usize].known_to = 1; // scried: I know it is on top
    let belief = Belief::new(&g, 0);
    let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(3);
    let mut order_changed = false;
    for _ in 0..20 {
        let w = belief.sample(&g, &mut rng);
        assert_eq!(*w.players[0].library.last().unwrap(), top);
        order_changed |= w.players[0].library[..5] != g.players[0].library[..5];
        for &id in &w.players[0].library {
            assert_eq!(w.cards[id.0 as usize].zone, Zone::Library);
        }
    }
    assert!(order_changed, "unseen library cards are reshuffled");
}

/// The ONNX evaluation loaded through tract gives the same logit as onnxruntime did when the model was exported
/// (reference computed with `scripts/ai/train_eval.py`'s output: f[i] = 0.1 * (i % 7)). Regenerate this number when `eval_mlp.onnx` is retrained.
#[test]
fn onnx_model_matches_onnxruntime() {
    use mtg_ai::features::{has_onnx, onnx_logit, N};
    assert_eq!(N, 58);
    assert!(has_onnx(), "eval_mlp.onnx failed to load");
    let f: Vec<f32> = (0..N).map(|i| 0.1 * (i % 7) as f32).collect();
    let z = onnx_logit(&f).expect("model ran");
    assert!((z - 0.22448089718818665).abs() < 1e-4, "tract gave {z}");
}
