//! The AI's answers to text-change and number questions are chosen on purpose, not by position.

use mtg_ai::*;
use mtg_engine::textchange::*;
use mtg_engine::types::*;
use mtg_engine::*;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

fn db() -> Arc<CardDb> {
    static DB: OnceLock<Arc<CardDb>> = OnceLock::new();
    DB.get_or_init(|| mtg_cards::build_db(&mtg_cards::default_cards_dir()).unwrap()).clone()
}

/// Let the AI answer every question of the seat `me` until the stack is empty again (everyone else passes).
fn run_ai(g: &mut Game, me: PlayerId) {
    let mut ai = HeuristicAgent::new(5, Duration::from_millis(250));
    for _ in 0..80 {
        let Some(d) = g.next_decision() else { return };
        if matches!(d.kind, DecisionKind::Priority { .. }) {
            if g.stack.is_empty() {
                return;
            }
            g.apply(Choice::Priority { action: None }).unwrap();
        } else if d.player == me {
            let c = ai.decide(g, &d);
            g.apply(c).unwrap();
        } else {
            g.apply(Choice::Default).unwrap();
        }
    }
}

fn bog_wraith_game(wraith_owner: PlayerId) -> (Game, CardId) {
    let mut g = Game::new_blank(db(), 2, 1);
    let other = 1 - wraith_owner;
    // the caster has the Islands for Magical Hack and Swamps; the other player has Forests and Mountains
    g.add_card("Island", 0, Zone::Battlefield);
    g.add_card("Swamp", 0, Zone::Battlefield);
    g.add_card("Magical Hack", 0, Zone::Hand);
    g.add_card("Forest", 1, Zone::Battlefield);
    g.add_card("Forest", 1, Zone::Battlefield);
    g.add_card("Mountain", 1, Zone::Battlefield);
    let w = g.add_card("Bog Wraith", wraith_owner, Zone::Battlefield);
    let _ = other;
    g.start_at(0, Step::Main1);
    g.cast(0, "Magical Hack", &[Obj::card(w)]);
    (g, w)
}

#[test]
fn own_swampwalk_becomes_the_land_type_the_opponent_plays() {
    let (mut g, w) = bog_wraith_game(0);
    run_ai(&mut g, 0);
    assert!(g.card(w).cur.kw.has(Keyword::Forestwalk), "kw {:?}", g.def_of(w).keywords_list());
}

#[test]
fn opposing_swampwalk_becomes_a_land_type_we_do_not_play() {
    let (mut g, w) = bog_wraith_game(1);
    run_ai(&mut g, 0);
    let kws = g.def_of(w).keywords_list();
    assert!(!kws.contains(&Keyword::Swampwalk), "still swampwalk: {kws:?}");
    assert!(!kws.contains(&Keyword::Islandwalk), "islandwalk: we have an Island: {kws:?}");
}

#[test]
fn deadpool_swaps_its_drawback_onto_an_opposing_creature() {
    let mut g = Game::new_blank(db(), 2, 1);
    for _ in 0..4 {
        g.add_card("Swamp", 0, Zone::Battlefield);
        g.add_card("Mountain", 0, Zone::Battlefield);
    }
    let dp = g.add_card("Deadpool, Trading Card", 0, Zone::Hand);
    let mine = g.add_card("Grizzly Bears", 0, Zone::Battlefield);
    let theirs = g.add_card("Serra Angel", 1, Zone::Battlefield);
    g.start_at(0, Step::Main1);
    g.cast(0, "Deadpool, Trading Card", &[]);
    let mut ai = HeuristicAgent::new(5, Duration::from_millis(250));
    for _ in 0..40 {
        let Some(d) = g.next_decision() else { break };
        match &d.kind {
            DecisionKind::Priority { .. } if g.stack.is_empty() => break,
            DecisionKind::Priority { .. } => g.apply(Choice::Priority { action: None }).unwrap(),
            _ if d.player == 0 => {
                let c = ai.decide(&g, &d);
                g.apply(c).unwrap()
            }
            _ => g.apply(Choice::Default).unwrap(),
        }
    }
    let _ = (dp, mine);
    // Deadpool took the Angel's flying and vigilance text; the Angel got "lose 3 life at your upkeep"
    let angel_text = &g.def_of(theirs).abilities;
    assert!(angel_text.iter().any(|a| serde_json::to_string(a).unwrap().contains("LoseLife")), "the Angel did not get the drawback");
}

#[test]
fn minion_of_the_wastes_does_not_pay_almost_all_its_life() {
    let mut g = Game::new_blank(db(), 2, 1);
    for _ in 0..6 {
        g.add_card("Swamp", 0, Zone::Battlefield);
    }
    let m = g.add_card("Minion of the Wastes", 0, Zone::Hand);
    g.start_at(0, Step::Main1);
    g.cast(0, "Minion of the Wastes", &[]);
    run_ai(&mut g, 0);
    assert_eq!(g.card(m).zone, Zone::Battlefield);
    let life = g.players[0].life;
    assert!((12..20).contains(&life), "life left {life}");
}

#[test]
fn x_spell_still_takes_all_the_mana() {
    let mut g = Game::new_blank(db(), 2, 1);
    for _ in 0..5 {
        g.add_card("Mountain", 0, Zone::Battlefield);
    }
    g.add_card("Fireball", 0, Zone::Hand);
    g.start_at(0, Step::Main1);
    g.cast(0, "Fireball", &[Obj::player(1)]);
    run_ai(&mut g, 0);
    assert!(g.players[1].life <= 20 - 3, "life {}", g.players[1].life);
}

#[test]
fn candidates_cover_the_range_and_start_with_the_default() {
    let c = mtg_ai::textchoice::number_candidates(0, 999, 0);
    assert_eq!(c[0], 0);
    assert!(c.contains(&999) && c.contains(&1) && c.len() <= 10);
    assert_eq!(mtg_ai::textchoice::number_default("Pay how much life?", 0, 20), 0);
    assert_eq!(mtg_ai::textchoice::number_default("Choose X for Fireball", 0, 7), 7);
}

fn deadpool_priority(life: i32) -> bool {
    let mut g = Game::new_blank(db(), 2, 1);
    for _ in 0..4 {
        g.add_card("Swamp", 0, Zone::Battlefield);
    }
    g.add_card("Deadpool, Trading Card", 0, Zone::Battlefield);
    for _ in 0..10 {
        g.add_card("Forest", 1, Zone::Library);
        g.add_card("Forest", 0, Zone::Library);
    }
    g.players[0].life = life;
    g.start_at(0, Step::Main2);
    let mut ai = HeuristicAgent::new(5, Duration::from_millis(250));
    let d = g.next_decision().unwrap();
    let c = ai.decide(&g, &d);
    if let (DecisionKind::Priority { actions, .. }, Choice::Priority { action: Some(i) }) = (&d.kind, &c) {
        eprintln!("life {life}: chose {:?} of {actions:?}", actions[*i]);
    }
    matches!(c, Choice::Priority { action: Some(_) })
}

#[test]
fn deadpool_is_kept_while_healthy_and_sacrificed_when_its_drain_gets_dangerous() {
    assert!(!deadpool_priority(20), "sacrificed at 20 life");
    assert!(deadpool_priority(6), "kept at 6 life");
}
