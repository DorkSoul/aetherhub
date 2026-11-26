//! The AI's mulligan answers: which opening hands it keeps under each mulligan rule, what it bottoms and how it scries.

use mtg_ai::heuristic::policy;
use mtg_engine::types::*;
use mtg_engine::*;
use std::sync::{Arc, OnceLock};

fn db() -> Arc<CardDb> {
    static DB: OnceLock<Arc<CardDb>> = OnceLock::new();
    DB.get_or_init(|| mtg_cards::build_db(&mtg_cards::default_cards_dir()).unwrap()).clone()
}

/// A game under `rule` with `cards` in seat 0's hand and `top` on top of its library (top first).
fn setup(rule: MulliganRule, cards: &[&str], top: &[&str]) -> (Game, Vec<CardId>, Vec<CardId>) {
    let mut g = Game::new_blank(db(), 2, 1);
    g.cfg.mulligan = rule;
    for _ in 0..20 {
        g.add_card("Grizzly Bears", 0, Zone::Library);
    }
    let top: Vec<CardId> = top.iter().rev().map(|n| g.add_card(n, 0, Zone::Library)).collect::<Vec<_>>().into_iter().rev().collect();
    let hand = cards.iter().map(|n| g.add_card(n, 0, Zone::Hand)).collect();
    (g, hand, top)
}

fn answer(rule: MulliganRule, cards: &[&str], mulligans: u8, top: &[&str]) -> Choice {
    let (g, hand, top) = setup(rule, cards, top);
    let peek = if rule == MulliganRule::FreeScry3 { top } else { vec![] };
    policy(&g, &Decision { player: 0, kind: DecisionKind::Mulligan { hand, mulligans, peek } })
}

fn keeps(rule: MulliganRule, cards: &[&str], mulligans: u8) -> bool {
    match answer(rule, cards, mulligans, &[]) {
        Choice::Keep { keep } => keep,
        c => panic!("expected keep or mulligan, got {c:?}"),
    }
}

const L: &str = "Forest";
const BEAR: &str = "Grizzly Bears";
const BIG: &str = "Craw Wurm";
const ROCK: &str = "Sol Ring";

#[test]
fn london_keeps_two_to_five_lands_and_ships_the_rest() {
    let r = MulliganRule::London;
    assert!(!keeps(r, &[L, BIG, BIG, BIG, BIG, BIG, BIG], 0), "one land");
    assert!(!keeps(r, &[BIG; 7], 0), "no land");
    assert!(!keeps(r, &[L, L, L, L, L, L, BEAR], 0), "six lands");
    assert!(keeps(r, &[L, L, L, BEAR, BIG, BIG, BIG], 0));
    assert!(keeps(r, &[L, L, L, L, L, BEAR, BIG], 0));
    // two lands: only with cheap plays or a mana rock
    assert!(keeps(r, &[L, L, BEAR, BEAR, BIG, BIG, BIG], 0));
    assert!(keeps(r, &[L, L, ROCK, BIG, BIG, BIG, BIG], 0));
    assert!(!keeps(r, &[L, L, BIG, BIG, BIG, BIG, BIG], 0));
    // after a mulligan the seven are judged as the six left after bottoming: six lands + one spell is five lands + one spell
    assert!(!keeps(r, &[L, L, L, L, L, L, BEAR], 1));
    // going to four: keep whatever
    assert!(keeps(r, &[BIG; 7], 3));
}

#[test]
fn a_free_mulligan_is_taken_on_borderline_hands() {
    let two_landers = [L, L, BEAR, BEAR, BIG, BIG, BIG];
    assert!(keeps(MulliganRule::London, &two_landers, 0));
    assert!(!keeps(MulliganRule::LondonFreeFirst, &two_landers, 0), "the first Commander mulligan is free");
    assert!(keeps(MulliganRule::LondonFreeFirst, &two_landers, 1), "the second costs a card");
    assert!(!keeps(MulliganRule::Free, &two_landers, 0));
    assert!(keeps(MulliganRule::Free, &[L, L, ROCK, BIG, BIG, BIG, BIG], 0), "two lands and a mana rock");
    // unlimited free mulligans stop: the normal rule after four, anything after eight
    assert!(keeps(MulliganRule::Free, &two_landers, 4));
    assert!(keeps(MulliganRule::Free, &[BIG; 7], 8));
    assert!(!keeps(MulliganRule::Free, &[BIG; 7], 7));
}

#[test]
fn vancouver_and_paris_keep_drawn_small_hands() {
    for r in [MulliganRule::Vancouver, MulliganRule::Paris] {
        assert!(keeps(r, &[L, L, L, L, L, BEAR], 1), "five lands in six");
        assert!(keeps(r, &[L, BEAR, BEAR, BIG, BIG], 2), "one land in five");
        assert!(!keeps(r, &[BEAR, BEAR, BIG, BIG, BIG], 2), "no land in five");
        assert!(keeps(r, &[BIG; 4], 3));
    }
}

#[test]
fn free_scry_three_never_keeps_one_landers_and_digs_for_lands() {
    let r = MulliganRule::FreeScry3;
    // one land, even with lands coming: a fresh seven is free
    assert_eq!(answer(r, &[L, BEAR, BEAR, BIG, BIG, BIG, BIG], 0, &[L, L, BEAR]), Choice::Keep { keep: false });
    // two lands and a land on top: keep; the expensive spells on top go to the bottom to dig for more lands
    assert_eq!(answer(r, &[L, L, BEAR, BIG, BIG, BIG, BIG], 0, &[BIG, L, BEAR]), Choice::Pick { indices: vec![0] });
    // two lands, no land and no mana rock: a fresh seven
    assert_eq!(answer(r, &[L, L, BEAR, BEAR, BIG, BIG, BIG], 0, &[BIG, BIG, BEAR]), Choice::Keep { keep: false });
    // five lands: keep, and the lands on top go to the bottom
    assert_eq!(answer(r, &[L, L, L, L, L, BEAR, BIG], 0, &[L, BEAR, L]), Choice::Pick { indices: vec![0, 2] });
    // three lands, a fine top: keep without scrying anything away
    assert_eq!(answer(r, &[L, L, L, BEAR, BEAR, BIG, BIG], 0, &[L, BEAR, BIG]), Choice::Keep { keep: true });
    // after the scry (fewer than three cards left on top) the hand is kept
    assert_eq!(answer(r, &[L, L, BEAR, BIG, BIG, BIG, BIG], 0, &[L, BEAR]), Choice::Keep { keep: true });
}

#[test]
fn bottoming_keeps_about_half_lands_and_drops_expensive_spells() {
    let (g, hand, _) = setup(MulliganRule::London, &[L, L, L, L, L, BEAR, BIG], &[]);
    let Choice::Bottom { cards } = policy(&g, &Decision { player: 0, kind: DecisionKind::Bottom { hand: hand.clone(), n: 2 } }) else { panic!() };
    let lands_bottomed = cards.iter().filter(|c| g.card(**c).cur.types.has(CardType::Land)).count();
    assert_eq!(lands_bottomed, 2, "five cards kept: three lands and both spells");
    let (g, hand, _) = setup(MulliganRule::London, &[L, L, L, ROCK, BEAR, BIG, BIG], &[]);
    let Choice::Bottom { cards } = policy(&g, &Decision { player: 0, kind: DecisionKind::Bottom { hand: hand.clone(), n: 2 } }) else { panic!() };
    let names: Vec<&str> = cards.iter().map(|c| g.def_of(*c).name.as_str()).collect();
    assert_eq!(names, vec![BIG, BIG]);
}
