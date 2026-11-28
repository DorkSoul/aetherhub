//! Rules tests against the real card files.

use mtg_engine::types::*;
use mtg_engine::*;
use std::sync::{Arc, OnceLock};

fn db() -> Arc<CardDb> {
    static DB: OnceLock<Arc<CardDb>> = OnceLock::new();
    DB.get_or_init(|| mtg_cards::build_db(&mtg_cards::default_cards_dir()).unwrap_or_else(|e| panic!("cards: {e:?}"))).clone()
}

fn game() -> Game {
    Game::new_blank(db(), 2, 1)
}

#[test]
fn bolt_kills_bears() {
    let mut g = game();
    g.add_card("Mountain", 0, Zone::Battlefield);
    g.add_card("Lightning Bolt", 0, Zone::Hand);
    let bears = g.add_card("Grizzly Bears", 1, Zone::Battlefield);
    g.start_at(0, Step::Main1);
    g.cast(0, "Lightning Bolt", &[Obj::card(bears)]);
    g.resolve_stack();
    assert_eq!(g.count_in("Grizzly Bears", Zone::Graveyard), 1);
    assert_eq!(g.count_in("Lightning Bolt", Zone::Graveyard), 1);
    assert!(g.card(g.find("Mountain", Zone::Battlefield).unwrap()).tapped);
}

#[test]
fn bolt_face() {
    let mut g = game();
    g.add_card("Mountain", 0, Zone::Battlefield);
    g.add_card("Lightning Bolt", 0, Zone::Hand);
    g.start_at(0, Step::Main1);
    g.cast(0, "Lightning Bolt", &[Obj::player(1)]);
    g.resolve_stack();
    assert_eq!(g.players[1].life, 17);
}

#[test]
fn cannot_cast_without_mana() {
    let mut g = game();
    g.add_card("Lightning Bolt", 0, Zone::Hand);
    g.start_at(0, Step::Main1);
    let d = g.next_decision().unwrap();
    let DecisionKind::Priority { actions, .. } = d.kind else { panic!() };
    assert!(actions.is_empty());
}

#[test]
fn land_drop_once() {
    let mut g = game();
    g.add_card("Forest", 0, Zone::Hand);
    g.add_card("Forest", 0, Zone::Hand);
    g.start_at(0, Step::Main1);
    g.play_land(0, "Forest");
    let d = g.next_decision().unwrap();
    let DecisionKind::Priority { actions, .. } = d.kind else { panic!() };
    assert!(!actions.iter().any(|a| matches!(a, Action::PlayLand { .. })));
}

#[test]
fn creature_summoning_sick_then_attacks() {
    let mut g = game();
    g.add_card("Forest", 0, Zone::Battlefield);
    g.add_card("Forest", 0, Zone::Battlefield);
    g.add_card("Grizzly Bears", 0, Zone::Hand);
    g.start_at(0, Step::Main1);
    g.cast(0, "Grizzly Bears", &[]);
    g.resolve_stack();
    assert_eq!(g.count_in("Grizzly Bears", Zone::Battlefield), 1);
    // go to combat: it is summoning sick, so no attack declaration is offered
    g.pass_to(Step::Main2);
    assert_eq!(g.players[1].life, 20);
}

#[test]
fn wrath_destroys_all() {
    let mut g = game();
    for _ in 0..4 {
        g.add_card("Plains", 0, Zone::Battlefield);
    }
    g.add_card("Wrath of God", 0, Zone::Hand);
    g.add_card("Grizzly Bears", 0, Zone::Battlefield);
    g.add_card("Hill Giant", 1, Zone::Battlefield);
    g.start_at(0, Step::Main1);
    g.cast(0, "Wrath of God", &[]);
    g.resolve_stack();
    assert_eq!(g.battlefield.iter().filter(|c| g.card(**c).cur.types.has(CardType::Creature)).count(), 0);
}

#[test]
fn counterspell_counters_bolt() {
    let mut g = game();
    g.add_card("Mountain", 0, Zone::Battlefield);
    g.add_card("Lightning Bolt", 0, Zone::Hand);
    g.add_card("Island", 1, Zone::Battlefield);
    g.add_card("Island", 1, Zone::Battlefield);
    g.add_card("Counterspell", 1, Zone::Hand);
    g.start_at(0, Step::Main1);
    g.cast(0, "Lightning Bolt", &[Obj::player(1)]);
    // player 0 passes priority; player 1 responds
    g.pass();
    let bolt_stack = g.stack[0].id;
    g.cast(1, "Counterspell", &[Obj::Stack { id: bolt_stack }]);
    g.resolve_stack();
    assert_eq!(g.players[1].life, 20);
    assert_eq!(g.count_in("Lightning Bolt", Zone::Graveyard), 1);
    assert_eq!(g.count_in("Counterspell", Zone::Graveyard), 1);
}

#[test]
fn glorious_anthem_pumps() {
    let mut g = game();
    g.add_card("Glorious Anthem", 0, Zone::Battlefield);
    let b = g.add_card("Grizzly Bears", 0, Zone::Battlefield);
    let o = g.add_card("Grizzly Bears", 1, Zone::Battlefield);
    g.start_at(0, Step::Main1);
    assert_eq!((g.card(b).cur.power, g.card(b).cur.toughness), (3, 3));
    assert_eq!((g.card(o).cur.power, g.card(o).cur.toughness), (2, 2));
}

#[test]
fn llanowar_elves_taps_for_mana() {
    let mut g = game();
    g.add_card("Llanowar Elves", 0, Zone::Battlefield);
    g.add_card("Grizzly Bears", 0, Zone::Hand);
    g.add_card("Forest", 0, Zone::Battlefield);
    g.start_at(0, Step::Main1);
    g.cast(0, "Grizzly Bears", &[]);
    g.resolve_stack();
    assert_eq!(g.count_in("Grizzly Bears", Zone::Battlefield), 1);
}

#[test]
fn flametongue_etb_kills() {
    let mut g = game();
    for _ in 0..4 {
        g.add_card("Mountain", 0, Zone::Battlefield);
    }
    g.add_card("Flametongue Kavu", 0, Zone::Hand);
    let giant = g.add_card("Hill Giant", 1, Zone::Battlefield);
    g.start_at(0, Step::Main1);
    g.cast(0, "Flametongue Kavu", &[]);
    // the ETB trigger goes on the stack after the spell resolves; choose Hill Giant when asked
    g.pass();
    g.pass();
    g.answer_targets(&[Obj::card(giant)]);
    g.resolve_stack();
    assert_eq!(g.count_in("Hill Giant", Zone::Graveyard), 1);
}

fn creature_pt(g: &Game, id: CardId) -> (i32, i32) {
    (g.card(id).cur.power, g.card(id).cur.toughness)
}

#[test]
fn unblocked_attack_deals_damage() {
    let mut g = game();
    let b = g.add_card("Grizzly Bears", 0, Zone::Battlefield);
    g.start_at(0, Step::Main1);
    g.attack(&[(b, Obj::player(1))]);
    g.finish_combat();
    assert_eq!(g.players[1].life, 18);
    assert!(g.card(b).tapped);
}

#[test]
fn blocked_bears_trade() {
    let mut g = game();
    let a = g.add_card("Grizzly Bears", 0, Zone::Battlefield);
    let b = g.add_card("Grizzly Bears", 1, Zone::Battlefield);
    g.start_at(0, Step::Main1);
    g.attack(&[(a, Obj::player(1))]);
    g.block(&[(b, a)]);
    g.finish_combat();
    assert_eq!(g.count_in("Grizzly Bears", Zone::Graveyard), 2);
    assert_eq!(g.players[1].life, 20);
}

#[test]
fn flyer_unblockable_by_ground() {
    let mut g = game();
    let angel = g.add_card("Serra Angel", 0, Zone::Battlefield);
    let bears = g.add_card("Grizzly Bears", 1, Zone::Battlefield);
    let spider = g.add_card("Giant Spider", 1, Zone::Battlefield);
    g.start_at(0, Step::Main1);
    g.attack(&[(angel, Obj::player(1))]);
    let d = g.to_blockers();
    let DecisionKind::Blockers { options } = d.kind else { panic!() };
    assert!(options.iter().all(|o| o.card != bears), "ground creature can't block a flyer");
    assert!(options.iter().any(|o| o.card == spider), "reach can block a flyer");
    g.apply(Choice::Blockers { blocks: vec![] }).unwrap();
    g.finish_combat();
    assert_eq!(g.players[1].life, 16);
    assert!(!g.card(angel).tapped, "vigilance");
}

#[test]
fn first_strike_wins_combat() {
    let mut g = game();
    let knight = g.add_card("Youthful Knight", 0, Zone::Battlefield);
    let bears = g.add_card("Grizzly Bears", 1, Zone::Battlefield);
    g.start_at(0, Step::Main1);
    g.attack(&[(knight, Obj::player(1))]);
    g.block(&[(bears, knight)]);
    g.finish_combat();
    // 2/1 first striker deals 2 first: the 2/2 dies without dealing damage back
    assert_eq!(g.count_in("Grizzly Bears", Zone::Graveyard), 1);
    assert_eq!(g.count_in("Youthful Knight", Zone::Battlefield), 1);
}

#[test]
fn double_strike_deals_twice() {
    let mut g = game();
    let ace = g.add_card("Fencing Ace", 0, Zone::Battlefield);
    g.start_at(0, Step::Main1);
    g.attack(&[(ace, Obj::player(1))]);
    g.finish_combat();
    assert_eq!(g.players[1].life, 18);
}

#[test]
fn deathtouch_kills_giant() {
    let mut g = game();
    let rats = g.add_card("Typhoid Rats", 0, Zone::Battlefield);
    let giant = g.add_card("Hill Giant", 1, Zone::Battlefield);
    g.start_at(0, Step::Main1);
    g.attack(&[(rats, Obj::player(1))]);
    g.block(&[(giant, rats)]);
    g.finish_combat();
    assert_eq!(g.count_in("Hill Giant", Zone::Graveyard), 1);
    assert_eq!(g.count_in("Typhoid Rats", Zone::Graveyard), 1);
}

#[test]
fn lifelink_gains_life() {
    let mut g = game();
    let vamp = g.add_card("Vampire Nighthawk", 0, Zone::Battlefield);
    g.start_at(0, Step::Main1);
    g.attack(&[(vamp, Obj::player(1))]);
    g.finish_combat();
    assert_eq!(g.players[0].life, 22);
    assert_eq!(g.players[1].life, 18);
}

#[test]
fn trample_assigns_excess() {
    let mut g = game();
    let badger = g.add_card("Charging Badger", 0, Zone::Battlefield);
    g.add_card("Rancor", 0, Zone::Hand);
    g.add_card("Forest", 0, Zone::Battlefield);
    let wall = g.add_card("Wall of Stone", 1, Zone::Battlefield);
    g.start_at(0, Step::Main1);
    g.cast(0, "Rancor", &[Obj::card(badger)]);
    g.resolve_stack();
    assert_eq!(creature_pt(&g, badger), (3, 1));
    g.attack(&[(badger, Obj::player(1))]);
    g.block(&[(wall, badger)]);
    // damage assignment prompt: 1 lethal to the wall? Wall of Stone has 8 toughness so everything stays on it
    let d = g.next_decision().unwrap();
    if let DecisionKind::AssignDamage { total, blockers, .. } = d.kind {
        assert_eq!(total, 3);
        assert_eq!(blockers[0].1, 8);
        let c = g.default_choice(&Decision { player: 0, kind: DecisionKind::AssignDamage { attacker: badger, total, blockers, defender: Some(Obj::player(1)), trample: true } });
        g.apply(c).unwrap();
    }
    g.finish_combat();
    assert_eq!(g.players[1].life, 20);
}

#[test]
fn summoning_sick_creature_cannot_attack() {
    let mut g = game();
    g.add_card("Forest", 0, Zone::Battlefield);
    g.add_card("Forest", 0, Zone::Battlefield);
    g.add_card("Grizzly Bears", 0, Zone::Hand);
    g.start_at(0, Step::Main1);
    g.cast(0, "Grizzly Bears", &[]);
    g.resolve_stack();
    g.pass_to(Step::BeginCombat);
    // no attack declaration is offered; combat is skipped
    g.pass_to(Step::Main2);
    assert_eq!(g.players[1].life, 20);
}

#[test]
fn haste_attacks_immediately() {
    let mut g = game();
    g.add_card("Mountain", 0, Zone::Battlefield);
    g.add_card("Raging Goblin", 0, Zone::Hand);
    g.start_at(0, Step::Main1);
    g.cast(0, "Raging Goblin", &[]);
    g.resolve_stack();
    let goblin = g.find("Raging Goblin", Zone::Battlefield).unwrap();
    g.attack(&[(goblin, Obj::player(1))]);
    g.finish_combat();
    assert_eq!(g.players[1].life, 19);
}

#[test]
fn pacifism_stops_attack_and_block() {
    let mut g = game();
    g.add_card("Plains", 0, Zone::Battlefield);
    g.add_card("Plains", 0, Zone::Battlefield);
    g.add_card("Pacifism", 0, Zone::Hand);
    let target = g.add_card("Hill Giant", 1, Zone::Battlefield);
    g.start_at(0, Step::Main1);
    g.cast(0, "Pacifism", &[Obj::card(target)]);
    g.resolve_stack();
    let pac = g.find("Pacifism", Zone::Battlefield).unwrap();
    assert_eq!(g.card(pac).attached_to, Some(target));
    assert!(g.card(target).cur.cant_attack && g.card(target).cur.cant_block);
}

#[test]
fn legend_rule_keeps_one() {
    let mut g = game();
    g.add_card("Isamaru, Hound of Konda", 0, Zone::Battlefield);
    g.add_card("Plains", 0, Zone::Battlefield);
    g.add_card("Isamaru, Hound of Konda", 0, Zone::Hand);
    g.start_at(0, Step::Main1);
    g.cast(0, "Isamaru, Hound of Konda", &[]);
    g.pass();
    g.pass();
    // the legend rule asks which to keep
    let d = g.next_decision().unwrap();
    assert!(matches!(d.kind, DecisionKind::Pick { .. }));
    g.apply(Choice::Pick { indices: vec![0] }).unwrap();
    g.run();
    assert_eq!(g.count_in("Isamaru, Hound of Konda", Zone::Battlefield), 1);
    assert_eq!(g.count_in("Isamaru, Hound of Konda", Zone::Graveyard), 1);
}

#[test]
fn goblin_king_lords_goblins() {
    let mut g = game();
    g.add_card("Goblin King", 0, Zone::Battlefield);
    let gob = g.add_card("Raging Goblin", 0, Zone::Battlefield);
    let king = g.find("Goblin King", Zone::Battlefield).unwrap();
    g.start_at(0, Step::Main1);
    assert_eq!(creature_pt(&g, gob), (2, 2));
    assert_eq!(creature_pt(&g, king), (2, 2), "other goblins only");
}

#[test]
fn tarmogoyf_counts_types() {
    let mut g = game();
    let goyf = g.add_card("Tarmogoyf", 0, Zone::Battlefield);
    // Tarmogoyf counts the card types among cards in ALL graveyards: a land and a creature card = 2
    g.add_card("Mountain", 0, Zone::Graveyard);
    g.add_card("Grizzly Bears", 1, Zone::Graveyard);
    g.start_at(0, Step::Main1);
    assert_eq!(creature_pt(&g, goyf), (2, 3));
}

#[test]
fn mana_leak_can_be_paid() {
    let mut g = game();
    g.add_card("Mountain", 0, Zone::Battlefield);
    for _ in 0..4 {
        g.add_card("Island", 0, Zone::Battlefield);
    }
    g.add_card("Lightning Bolt", 1, Zone::Hand);
    g.add_card("Mountain", 1, Zone::Battlefield);
    g.add_card("Mana Leak", 0, Zone::Hand);
    g.start_at(1, Step::Main1);
    g.cast(1, "Lightning Bolt", &[Obj::player(0)]);
    g.pass();
    let bolt = g.stack[0].id;
    g.cast(0, "Mana Leak", &[Obj::Stack { id: bolt }]);
    g.pass();
    g.pass();
    // the bolt's controller has no spare mana, so the unless-cost can't be paid: bolt is countered
    g.resolve_stack();
    assert_eq!(g.players[0].life, 20);
    assert_eq!(g.count_in("Lightning Bolt", Zone::Graveyard), 1);
}

#[test]
fn raise_the_alarm_makes_two_soldiers() {
    let mut g = game();
    g.add_card("Plains", 0, Zone::Battlefield);
    g.add_card("Plains", 0, Zone::Battlefield);
    g.add_card("Raise the Alarm", 0, Zone::Hand);
    g.start_at(0, Step::Main1);
    g.cast(0, "Raise the Alarm", &[]);
    g.resolve_stack();
    assert_eq!(g.count_in("Soldier", Zone::Battlefield), 2);
}

#[test]
fn rampant_growth_searches() {
    let mut g = game();
    g.add_card("Forest", 0, Zone::Battlefield);
    g.add_card("Forest", 0, Zone::Battlefield);
    g.add_card("Rampant Growth", 0, Zone::Hand);
    g.add_card("Forest", 0, Zone::Library);
    g.add_card("Mountain", 0, Zone::Library);
    g.start_at(0, Step::Main1);
    g.cast(0, "Rampant Growth", &[]);
    g.pass();
    g.pass();
    let d = g.next_decision().unwrap();
    let DecisionKind::Pick { options, .. } = d.kind else { panic!("expected a search prompt") };
    assert_eq!(options.len(), 2);
    g.apply(Choice::Pick { indices: vec![0] }).unwrap();
    g.resolve_stack();
    assert_eq!(g.count_in("Forest", Zone::Battlefield), 3);
    let f = g.find_all("Forest", Zone::Battlefield).into_iter().filter(|c| g.card(*c).tapped).count();
    assert!(f >= 3, "all three forests tapped (two for mana, one entered tapped), got {f}");
}

#[test]
fn blood_artist_triggers_on_any_death() {
    let mut g = game();
    g.add_card("Blood Artist", 0, Zone::Battlefield);
    g.add_card("Mountain", 0, Zone::Battlefield);
    g.add_card("Lightning Bolt", 0, Zone::Hand);
    let bears = g.add_card("Grizzly Bears", 1, Zone::Battlefield);
    g.start_at(0, Step::Main1);
    g.cast(0, "Lightning Bolt", &[Obj::card(bears)]);
    g.pass();
    g.pass();
    // Blood Artist's trigger needs a target player
    g.answer_targets(&[Obj::player(1)]);
    g.resolve_stack();
    assert_eq!(g.players[1].life, 19);
    assert_eq!(g.players[0].life, 21);
}

#[test]
fn equipment_equip_ability() {
    let mut g = game();
    g.add_card("Short Sword", 0, Zone::Battlefield);
    g.add_card("Plains", 0, Zone::Battlefield);
    let b = g.add_card("Elite Vanguard", 0, Zone::Battlefield);
    g.start_at(0, Step::Main1);
    let sword = g.find("Short Sword", Zone::Battlefield).unwrap();
    let ai = g.def_of(sword).activated[0];
    g.activate(0, "Short Sword", ai, &[Obj::card(b)]);
    g.resolve_stack();
    assert_eq!(creature_pt(&g, b), (3, 2));
    assert_eq!(g.card(sword).attached_to, Some(b));
}

#[test]
fn planeswalker_once_per_turn() {
    let mut g = game();
    g.add_card("Plains", 0, Zone::Battlefield);
    g.add_card("Plains", 0, Zone::Battlefield);
    g.add_card("Plains", 0, Zone::Battlefield);
    g.add_card("Test Walker", 0, Zone::Hand);
    g.start_at(0, Step::Main1);
    g.cast(0, "Test Walker", &[]);
    g.resolve_stack();
    let pw = g.find("Test Walker", Zone::Battlefield).unwrap();
    assert_eq!(g.card(pw).counter(CounterKind::Loyalty), 4);
    g.activate(0, "Test Walker", 0, &[]);
    g.resolve_stack();
    assert_eq!(g.players[0].life, 22);
    assert_eq!(g.card(pw).counter(CounterKind::Loyalty), 5);
    // a second activation this turn is not offered
    let d = g.next_decision().unwrap();
    let DecisionKind::Priority { actions, .. } = d.kind else { panic!() };
    assert!(!actions.iter().any(|a| matches!(a, Action::Activate { .. })));
}

#[test]
fn modal_spell_chooses_mode() {
    let mut g = game();
    g.add_card("Mountain", 0, Zone::Battlefield);
    g.add_card("Plains", 0, Zone::Battlefield);
    g.add_card("Boros Charm", 0, Zone::Hand);
    g.start_at(0, Step::Main1);
    assert!(g.act(0, |g, a| matches!(a, Action::Cast { card, .. } if g.card_name(*card) == "Boros Charm")));
    // mode prompt
    let d = g.next_decision().unwrap();
    let DecisionKind::Pick { options, .. } = d.kind else { panic!() };
    assert_eq!(options.len(), 3);
    g.apply(Choice::Pick { indices: vec![0] }).unwrap();
    g.answer_targets(&[Obj::player(1)]);
    g.resolve_stack();
    assert_eq!(g.players[1].life, 16);
}

#[test]
fn x_less_mulligan_and_opening_hand() {
    // full game start: libraries of 40, London mulligan
    let db = db();
    let forest = db.find("Forest").unwrap();
    let bears = db.find("Grizzly Bears").unwrap();
    let mut main = vec![forest; 20];
    main.extend(vec![bears; 20]);
    let decks = vec![DeckSpec { name: "A".into(), main: main.clone(), commanders: vec![], sideboard: vec![], companion: None }, DeckSpec { name: "B".into(), main, commanders: vec![], sideboard: vec![], companion: None }];
    let mut g = Game::new(db, Config::standard(), decks, 42);
    let d = g.next_decision().unwrap();
    let DecisionKind::Mulligan { hand, mulligans, .. } = d.kind else { panic!() };
    assert_eq!((hand.len(), mulligans), (7, 0));
    g.apply(Choice::Keep { keep: false }).unwrap();
    let d = g.next_decision().unwrap();
    let DecisionKind::Mulligan { hand, mulligans, .. } = d.kind else { panic!() };
    assert_eq!((hand.len(), mulligans), (7, 1));
    g.apply(Choice::Keep { keep: true }).unwrap();
    // must bottom one card
    let d = g.next_decision().unwrap();
    let DecisionKind::Bottom { hand, n } = d.kind else { panic!("expected bottom, got {:?}", d.kind) };
    assert_eq!(n, 1);
    g.apply(Choice::Bottom { cards: vec![hand[0]] }).unwrap();
    g.run();
    assert_eq!(g.players[d.player as usize].hand.len(), 6);
}

#[test]
fn free_vancouver_and_paris_mulligans() {
    let db = db();
    let forest = db.find("Forest").unwrap();
    let bears = db.find("Grizzly Bears").unwrap();
    let mut main = vec![forest; 20];
    main.extend(vec![bears; 20]);
    let deck = || DeckSpec { name: "A".into(), main: main.clone(), commanders: vec![], sideboard: vec![], companion: None };
    let start = |rule: MulliganRule| {
        let cfg = Config { mulligan: rule, ..Config::standard() };
        let mut g = Game::new(db.clone(), cfg, vec![deck(), deck()], 42);
        let d = g.next_decision().unwrap();
        g.apply(Choice::Keep { keep: false }).unwrap();
        assert_eq!(g.next_decision().unwrap().player, d.player);
        g.apply(Choice::Keep { keep: false }).unwrap();
        (g, d.player)
    };
    // free: two mulligans, still seven cards and nothing to put on the bottom
    let (mut g, p) = start(MulliganRule::Free);
    let d = g.next_decision().unwrap();
    let DecisionKind::Mulligan { hand, mulligans, .. } = d.kind else { panic!("{:?}", d.kind) };
    assert_eq!((hand.len(), mulligans, d.player), (7, 2, p));
    g.apply(Choice::Keep { keep: true }).unwrap();
    let d = g.next_decision().unwrap();
    assert!(matches!(d.kind, DecisionKind::Mulligan { .. }) && d.player != p, "free mulligan asked {:?}", d.kind);
    assert_eq!(g.players[p as usize].hand.len(), 7);
    // paris: one fewer each time, no scry
    let (mut g, p) = start(MulliganRule::Paris);
    let d = g.next_decision().unwrap();
    let DecisionKind::Mulligan { hand, .. } = d.kind else { panic!() };
    assert_eq!(hand.len(), 5);
    g.apply(Choice::Keep { keep: true }).unwrap();
    let d = g.next_decision().unwrap();
    assert!(matches!(d.kind, DecisionKind::Mulligan { .. }) && d.player != p);
    // vancouver: one fewer each time, then scry 1 (bottoming the top card)
    let (mut g, p) = start(MulliganRule::Vancouver);
    g.next_decision();
    g.apply(Choice::Keep { keep: true }).unwrap();
    let d = g.next_decision().unwrap();
    let DecisionKind::Pick { options, purpose: PickPurpose::Scry, .. } = d.kind else { panic!("expected the scry, got {:?}", d.kind) };
    assert_eq!((d.player, options.len()), (p, 1));
    let top = *g.players[p as usize].library.last().unwrap();
    g.apply(Choice::Pick { indices: vec![0] }).unwrap();
    g.next_decision();
    assert_eq!(g.players[p as usize].library[0], top);
    assert_eq!(g.players[p as usize].hand.len(), 5);
    // free mulligan + scry 3: always seven, and the top three are scried while deciding
    let (mut g, p) = start(MulliganRule::FreeScry3);
    let d = g.next_decision().unwrap();
    let DecisionKind::Mulligan { hand, mulligans, peek } = d.kind else { panic!("{:?}", d.kind) };
    let lib = &g.players[p as usize].library;
    assert_eq!((hand.len(), mulligans, peek.len(), lib.len()), (7, 2, 3, 33), "the hand and the three looked at went to the bottom, a fresh seven drawn");
    assert_eq!(peek, lib.iter().rev().take(3).copied().collect::<Vec<_>>(), "peek is the top three, top first");
    // one of the three to the bottom while still deciding: the peek shrinks to the two left on top
    g.apply(Choice::Pick { indices: vec![1] }).unwrap();
    let d = g.next_decision().unwrap();
    let DecisionKind::Mulligan { peek: peek2, .. } = d.kind else { panic!("{:?}", d.kind) };
    assert_eq!((d.player, peek2.clone()), (p, vec![peek[0], peek[2]]));
    assert_eq!(g.players[p as usize].library[0], peek[1]);
    // reorder the two still on top
    g.apply(Choice::Order { order: vec![1, 0] }).unwrap();
    let d = g.next_decision().unwrap();
    let DecisionKind::Mulligan { peek: peek3, .. } = d.kind else { panic!("{:?}", d.kind) };
    assert_eq!(peek3, vec![peek[2], peek[0]]);
    assert!(g.apply(Choice::Order { order: vec![0, 0] }).is_err(), "not a permutation");
    // mulligan anyway: the hand and the two still on top go to the bottom; the card already there isn't moved again
    let hand_before = g.players[p as usize].hand.clone();
    g.apply(Choice::Keep { keep: false }).unwrap();
    let d = g.next_decision().unwrap();
    let DecisionKind::Mulligan { peek: peek4, .. } = d.kind else { panic!("{:?}", d.kind) };
    assert_eq!(peek4.len(), 3, "a fresh scry 3");
    let lib = &g.players[p as usize].library;
    assert_eq!(lib.len(), 33);
    let gone: Vec<_> = hand_before.iter().chain([peek[0], peek[2]].iter()).copied().collect();
    assert!(lib[..9].iter().all(|c| gone.contains(c)), "the hand and the two scried cards still on top are at the bottom");
    assert_eq!(lib[9], peek[1], "the card scried to the bottom earlier is just above them");
    // keep: no scry after it, the next seat decides
    g.apply(Choice::Pick { indices: vec![0, 2] }).unwrap();
    g.next_decision();
    g.apply(Choice::Keep { keep: true }).unwrap();
    let d = g.next_decision().unwrap();
    assert!(matches!(d.kind, DecisionKind::Mulligan { .. }) && d.player != p, "{:?}", d.kind);
    let lib = &g.players[p as usize].library;
    assert_eq!(*lib.last().unwrap(), peek4[1], "the card not chosen stays on top");
    assert!(lib[..2].contains(&peek4[0]) && lib[..2].contains(&peek4[2]), "the chosen two went to the bottom");
    assert_eq!((g.players[p as usize].hand.len(), lib.len()), (7, 33));
}

#[test]
fn serum_powder_exiles_the_hand_before_a_keep_decision() {
    let db = db();
    let forest = db.find("Forest").unwrap();
    let powder = db.find("Serum Powder").unwrap();
    // the whole library is Serum Powder so every opening hand holds one
    let main = vec![powder; 40];
    let decks = vec![DeckSpec { name: "A".into(), main: main.clone(), commanders: vec![], sideboard: vec![], companion: None }, DeckSpec { name: "B".into(), main: vec![forest; 40], commanders: vec![], sideboard: vec![], companion: None }];
    let mut g = Game::new(db, Config::standard(), decks, 42);
    let mut d = g.next_decision().unwrap();
    if d.player != 0 {
        // the Forest deck acts first: it keeps
        g.apply(Choice::Keep { keep: true }).unwrap();
        d = g.next_decision().unwrap();
    }
    let who = d.player;
    let DecisionKind::Pick { prompt, options, .. } = &d.kind else { panic!("expected the Serum Powder question, got {:?}", d.kind) };
    assert!(prompt.contains("Serum Powder") && options.len() == 2, "{prompt}");
    let old: Vec<_> = g.players[who as usize].hand.clone();
    g.apply(Choice::Pick { indices: vec![1] }).unwrap();
    // asked again (the new hand holds Serum Powder too); then decline and get the keep / mulligan decision
    let d = g.next_decision().unwrap();
    assert!(matches!(d.kind, DecisionKind::Pick { .. }));
    g.apply(Choice::Pick { indices: vec![0] }).unwrap();
    let d = g.next_decision().unwrap();
    let DecisionKind::Mulligan { hand, mulligans, .. } = d.kind else { panic!("expected the keep decision, got {:?}", d.kind) };
    assert_eq!((hand.len(), mulligans), (7, 0));
    assert!(old.iter().all(|c| g.card(*c).zone == Zone::Exile), "the old hand is exiled");
}

#[test]
fn gemstone_caverns_begins_on_the_battlefield_with_a_luck_counter() {
    let db = db();
    let forest = db.find("Forest").unwrap();
    let caverns = db.find("Gemstone Caverns").unwrap();
    let mk = |caverns_seat: usize| {
        let mut decks = vec![
            DeckSpec { name: "A".into(), main: vec![forest; 40], commanders: vec![], sideboard: vec![], companion: None },
            DeckSpec { name: "B".into(), main: vec![forest; 40], commanders: vec![], sideboard: vec![], companion: None },
        ];
        decks[caverns_seat].main = vec![caverns; 40];
        Game::new(db.clone(), Config::standard(), decks, 42)
    };
    // the first player is fixed by the seed: give Gemstone Caverns to the other seat
    let first = mk(0).first_player as usize;
    let seat = 1 - first;
    let mut g = mk(seat);
    let mut guard = 0;
    loop {
        guard += 1;
        assert!(guard < 20, "no Gemstone Caverns question");
        let d = g.next_decision().unwrap();
        match &d.kind {
            DecisionKind::Mulligan { .. } => g.apply(Choice::Keep { keep: true }).unwrap(),
            DecisionKind::Pick { prompt, .. } if prompt.contains("Gemstone Caverns: begin") => {
                assert_eq!(d.player as usize, seat);
                g.apply(Choice::Pick { indices: vec![1] }).unwrap();
            }
            DecisionKind::Pick { prompt, .. } if prompt.contains("exile a card from your hand") => {
                g.apply(Choice::Pick { indices: vec![0] }).unwrap();
                break;
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    g.run();
    // the first player (who has no Gemstone Caverns) is asked nothing; the other seat began with one on the battlefield and exiled a card
    assert_eq!(g.players[seat].hand.len(), 5, "7 cards, one begins on the battlefield, one is exiled");
    let on_bf = g.battlefield.iter().filter(|id| g.card_name(**id) == "Gemstone Caverns" && g.card(**id).counter(CounterKind::Luck) == 1).count();
    assert_eq!(on_bf, 1);
}

#[test]
fn caller_of_the_untamed_exiles_a_creature_from_the_sideboard_before_the_game() {
    let db = db();
    let forest = db.find("Forest").unwrap();
    let caller = db.find("Caller of the Untamed").unwrap();
    let bears = db.find("Grizzly Bears").unwrap();
    let mut main = vec![forest; 39];
    main.push(caller);
    let decks = vec![
        DeckSpec { name: "A".into(), main, commanders: vec![], sideboard: vec![bears, bears], companion: None },
        DeckSpec { name: "B".into(), main: vec![forest; 40], commanders: vec![], sideboard: vec![], companion: None },
    ];
    let mut g = Game::new(db, Config::standard(), decks, 42);
    let d = g.next_decision().unwrap();
    assert_eq!(d.player, 0);
    let DecisionKind::Pick { prompt, .. } = &d.kind else { panic!("{:?}", d.kind) };
    assert!(prompt.contains("Caller of the Untamed"), "{prompt}");
    g.apply(Choice::Pick { indices: vec![1] }).unwrap();
    let d = g.next_decision().unwrap();
    let DecisionKind::Pick { options, min, .. } = &d.kind else { panic!("{:?}", d.kind) };
    assert_eq!((options.len(), *min), (2, 1));
    g.apply(Choice::Pick { indices: vec![0] }).unwrap();
    let _ = g.next_decision();
    let exiled: Vec<_> = g.cards.iter().filter(|c| c.zone == Zone::Exile && c.pregame_for.is_some()).collect();
    assert_eq!(exiled.len(), 1);
    assert_eq!(g.players[0].library.len() + g.players[0].hand.len(), 40);
}

#[test]
fn word_of_command_controls_the_player_while_the_cast_spell_resolves() {
    let db = db();
    let forest = db.find("Forest").unwrap();
    let decks = vec![
        DeckSpec { name: "A".into(), main: vec![forest; 40], commanders: vec![], sideboard: vec![], companion: None },
        DeckSpec { name: "B".into(), main: vec![forest; 40], commanders: vec![], sideboard: vec![], companion: None },
    ];
    let mut cfg = Config::standard();
    cfg.mulligan = MulliganRule::None;
    let mut g = Game::new(db, cfg, decks, 3);
    g.start_at(0, Step::Main1);
    for p in g.players.iter_mut() {
        p.auto_skip = false;
    }
    g.add_card("Swamp", 0, Zone::Battlefield);
    g.add_card("Swamp", 0, Zone::Battlefield);
    g.add_card("Word of Command", 0, Zone::Hand);
    g.add_card("Island", 1, Zone::Battlefield);
    g.add_card("Brainstorm", 1, Zone::Hand);
    g.cast(0, "Word of Command", &[Obj::player(1)]);
    // resolve until a decision about Brainstorm's choice appears: it is asked of player 0 (the controller), not player 1
    let mut seen = false;
    for _ in 0..40 {
        let Some(d) = g.next_decision() else { break };
        if let DecisionKind::Pick { prompt, options, .. } = &d.kind {
            if prompt.contains("Word of Command: choose") {
                let i = options.iter().position(|o| matches!(o, Opt::Obj { obj: Obj::Card { id } } if g.card_name(*id) == "Brainstorm")).expect("Brainstorm is in the hand");
                g.apply(Choice::Pick { indices: vec![i] }).unwrap();
                continue;
            }
        }
        if let DecisionKind::Pick { prompt, .. } = &d.kind {
            if prompt.contains("Brainstorm: choose") {
                assert_eq!(d.player, 0, "Brainstorm's choice is made by the controller of Word of Command");
                seen = true;
                break;
            }
        }
        // pass priority / take defaults
        g.apply(Choice::Default).unwrap();
    }
    assert!(seen, "no Brainstorm choice appeared");
}

#[test]
fn commander_goes_to_command_zone_and_costs_more() {
    let db = db();
    let forest = db.find("Forest").unwrap();
    let cmd = db.find("Test Commander").unwrap();
    let decks = vec![
        DeckSpec { name: "A".into(), main: vec![forest; 40], commanders: vec![cmd], sideboard: vec![], companion: None },
        DeckSpec { name: "B".into(), main: vec![forest; 40], commanders: vec![], sideboard: vec![], companion: None },
    ];
    let mut cfg = Config::commander();
    cfg.mulligan = MulliganRule::None;
    let mut g = Game::new(db, cfg, decks, 3);
    assert_eq!(g.players[0].life, 40);
    assert_eq!(g.players[0].command.len(), 1);
    let first = g.first_player;
    g.start_at(first, Step::Main1);
    for p in g.players.iter_mut() {
        p.auto_skip = false;
    }
    let cid = g.players[0].command[0];
    for _ in 0..6 {
        g.add_card("Forest", 0, Zone::Battlefield);
    }
    g.start_at(0, Step::Main1);
    g.cast(0, "Test Commander", &[]);
    g.resolve_stack();
    assert_eq!(g.card(cid).zone, Zone::Battlefield);
    // destroy it: it returns to the command zone, and recasting costs {2} more
    g.destroy(cid);
    // dying puts it in the graveyard; the state-based action (903.9a) asks its owner whether to put it into the command zone
    assert_eq!(g.card(cid).zone, Zone::Graveyard);
    g.run_sba();
    let d = g.pending.clone().expect("the owner is asked");
    assert_eq!(d.player, 0);
    assert!(matches!(d.kind, DecisionKind::YesNo { .. }));
    g.pending = None;
    g.answer = Some(Choice::YesNo { yes: true });
    g.run_sba();
    assert_eq!(g.card(cid).zone, Zone::Command);
    let cost = g.effective_cost(0, cid, None);
    assert_eq!(cost.mana_value(0), 6);
}


#[test]
fn floating_mana_then_cast() {
    let mut g = game();
    let forest = g.add_card("Forest", 0, Zone::Battlefield);
    g.add_card("Giant Growth", 0, Zone::Hand);
    let b = g.add_card("Grizzly Bears", 0, Zone::Battlefield);
    g.start_at(0, Step::Main1);
    g.next_decision().unwrap();
    let ab = g.def_of(forest).mana_abilities[0];
    g.apply(Choice::Mana { card: forest, ability: ab, produce: None, x: None, picks: vec![] }).unwrap();
    // asked again after tapping: the mana is floating and the land is tapped
    let d = g.next_decision().unwrap();
    assert!(matches!(d.kind, DecisionKind::Priority { .. }));
    assert_eq!(g.players[0].pool.get(mtg_engine::mana::ManaType::G), 1);
    assert!(g.card(forest).tapped);
    g.cast(0, "Giant Growth", &[Obj::card(b)]);
    g.resolve_stack();
    assert_eq!(g.card(b).cur.power, 5);
    assert_eq!(g.players[0].pool.total(), 0, "the floating mana paid for the spell");
}

#[test]
fn tap_a_land_for_mana_before_sacrificing_it_as_a_cost() {
    // CR 601.2g / 602.2b: mana abilities may be activated while a cost is being paid (Zuran Orb: "Sacrifice a land")
    let mut g = game();
    let forest = g.add_card("Forest", 0, Zone::Battlefield);
    g.add_card("Mountain", 0, Zone::Battlefield);
    g.add_card("Zuran Orb", 0, Zone::Battlefield);
    g.start_at(0, Step::Main1);
    let d = g.next_decision().unwrap();
    let DecisionKind::Priority { actions, .. } = &d.kind else { panic!("expected priority, got {d:?}") };
    let i = actions.iter().position(|a| matches!(a, Action::Activate { card, ability: 0 } if g.card_name(*card) == "Zuran Orb")).expect("Zuran Orb can be activated");
    g.apply(Choice::Priority { action: Some(i) }).unwrap();
    let d = g.next_decision().unwrap();
    assert!(matches!(d.kind, DecisionKind::Pick { purpose: PickPurpose::Cost, .. }), "asked which land to sacrifice: {d:?}");
    let DecisionKind::Pick { options, .. } = d.kind.clone() else { unreachable!() };
    // tap the Forest for mana: the same question stays open, the mana floats
    let ab = g.def_of(forest).mana_abilities[0];
    g.apply(Choice::Mana { card: forest, ability: ab, produce: None, x: None, picks: vec![] }).unwrap();
    assert!(g.card(forest).tapped);
    assert_eq!(g.players[0].pool.get(mtg_engine::mana::ManaType::G), 1);
    let again = g.next_decision().unwrap();
    assert_eq!(again.kind, d.kind, "the sacrifice question is asked again unchanged");
    // tapping it twice is refused
    assert!(g.apply(Choice::Mana { card: forest, ability: ab, produce: None, x: None, picks: vec![] }).is_err());
    // then sacrifice the (tapped) Forest
    let fi = options.iter().position(|o| matches!(o, Opt::Obj { obj } if *obj == Obj::card(forest))).unwrap();
    g.apply(Choice::Pick { indices: vec![fi] }).unwrap();
    g.resolve_stack();
    assert_eq!(g.card(forest).zone, Zone::Graveyard);
    assert_eq!(g.players[0].life, 22);
    assert_eq!(g.players[0].pool.get(mtg_engine::mana::ManaType::G), 1, "the mana from the sacrificed land is still floating");
}

#[test]
fn pass_until_skips_priority() {
    let mut g = game();
    g.add_card("Forest", 0, Zone::Battlefield);
    g.add_card("Giant Growth", 0, Zone::Hand);
    for _ in 0..6 {
        g.add_card("Forest", 0, Zone::Library);
        g.add_card("Forest", 1, Zone::Library);
    }
    g.start_at(0, Step::Main1);
    for p in g.players.iter_mut() {
        p.full_control = false;
        p.auto_skip = true;
    }
    g.players[0].pass_until = Some(PassUntil::Step { turn: 3, step: Step::Main1 });
    // castable Giant Growth would normally stop player 0 at every main phase; the first question arrives on turn 3
    let mut asked = None;
    for _ in 0..50 {
        let Some(d) = g.next_decision() else { break };
        if d.player == 0 {
            asked = Some((g.turn, g.step));
            break;
        }
        let c = g.default_choice(&d);
        g.apply(c).unwrap();
    }
    assert_eq!(asked, Some((3, Step::Main1)));
}

#[test]
fn pass_until_my_turn_stops_when_it_starts() {
    let mut g = game();
    g.add_card("Forest", 0, Zone::Battlefield);
    g.add_card("Giant Growth", 0, Zone::Hand);
    for _ in 0..6 {
        g.add_card("Forest", 0, Zone::Library);
        g.add_card("Forest", 1, Zone::Library);
    }
    g.start_at(0, Step::Main1);
    for p in g.players.iter_mut() {
        p.full_control = false;
        p.auto_skip = true;
        // no stops of their own: only the pass's target stops player 0
        p.stops_mine = 0;
        p.stops_opp = 0;
    }
    g.players[0].pass_until = Some(PassUntil::PlayerTurn { player: 0, after: 1 });
    let mut asked = None;
    for _ in 0..50 {
        let Some(d) = g.next_decision() else { break };
        if d.player == 0 {
            asked = Some((g.turn, g.step));
            break;
        }
        let c = g.default_choice(&d);
        g.apply(c).unwrap();
    }
    assert_eq!(asked, Some((3, Step::Upkeep)));
}

/// The first question for player 0 once player 1 attacks them with Grizzly Bears (player 0 holds Giant Growth and a Forest).
fn first_question_when_attacked(pass_until: Option<PassUntil>, castable: bool) -> (Step, bool) {
    let mut g = game();
    g.add_card("Forest", 0, Zone::Battlefield);
    if castable {
        g.add_card("Giant Growth", 0, Zone::Hand);
    }
    g.add_card("Grizzly Bears", 0, Zone::Battlefield);
    let bears = g.add_card("Grizzly Bears", 1, Zone::Battlefield);
    g.start_at(1, Step::Main1);
    for p in g.players.iter_mut() {
        p.full_control = false;
        p.auto_skip = true;
        p.stops_opp = 0;
    }
    g.players[0].pass_until = pass_until;
    g.attack(&[(bears, Obj::player(0))]);
    for _ in 0..50 {
        let d = g.next_decision().expect("game over");
        if d.player == 0 {
            return (g.step, matches!(d.kind, DecisionKind::Priority { .. }));
        }
        let c = g.default_choice(&d);
        g.apply(c).unwrap();
    }
    panic!("player 0 was never asked");
}

#[test]
fn an_attacked_player_gets_priority_before_blocks_when_they_can_act() {
    assert_eq!(first_question_when_attacked(None, true), (Step::DeclareAttackers, true));
    // passing until their own turn: no stop, straight to blocks
    assert_eq!(first_question_when_attacked(Some(PassUntil::PlayerTurn { player: 0, after: 1 }), true), (Step::DeclareBlockers, false));
    // nothing to cast: straight to blocks
    assert_eq!(first_question_when_attacked(None, false), (Step::DeclareBlockers, false));
}

#[test]
fn revealed_top_card_of_the_library_is_in_the_view() {
    let mut g = game();
    g.add_card("Oracle of Mul Daya", 0, Zone::Battlefield);
    g.add_card("Mystic Forge", 1, Zone::Battlefield);
    let a = g.add_card("Forest", 0, Zone::Library);
    let top0 = g.add_card("Island", 0, Zone::Library);
    let top1 = g.add_card("Swamp", 1, Zone::Library);
    g.start_at(0, Step::Main1);
    let lib = |v: &view::View| v.cards.iter().filter(|c| c.zone == Zone::Library).map(|c| c.id).collect::<Vec<_>>();
    // Oracle of Mul Daya: the top card of player 0's library is revealed to everyone, and only that card
    assert_eq!(lib(&g.view_for(0)).iter().filter(|c| **c == top0).count(), 1);
    assert!(lib(&g.view_for(1)).contains(&top0));
    assert!(!lib(&g.view_for(1)).contains(&a));
    // Mystic Forge: its controller may look at their own top card; the opponent may not
    assert!(lib(&g.view_for(1)).contains(&top1));
    assert!(!lib(&g.view_for(0)).contains(&top1));
}

#[test]
fn a_human_can_tap_a_land_for_mana_at_instant_speed_without_that_holding_priority() {
    let mut g = game();
    g.add_card("Forest", 0, Zone::Battlefield);
    g.start_at(1, Step::Upkeep);
    g.players[0].auto_skip = true;
    g.players[0].offer_mana = true;
    g.players[1].auto_skip = true;
    // on the opponent's turn with nothing to do but tap the Forest, the player is not asked (the mana ability never holds priority)
    g.run();
    assert!(g.decision().map_or(true, |d| d.player != 0 || !matches!(d.kind, DecisionKind::Priority { .. })), "asked for priority only because of a land");
    // with the stack in play the player is asked, and may tap the land in response
    let mut g = game();
    g.add_card("Forest", 0, Zone::Battlefield);
    g.add_card("Lightning Bolt", 1, Zone::Hand);
    g.add_card("Mountain", 1, Zone::Battlefield);
    g.start_at(1, Step::Main1);
    g.players[0].offer_mana = true;
    g.cast(1, "Lightning Bolt", &[Obj::player(0)]);
    let mut tapped = false;
    for _ in 0..20 {
        let Some(d) = g.next_decision() else { break };
        if d.player == 0 {
            if let DecisionKind::Priority { actions, .. } = &d.kind {
                if let Some(i) = actions.iter().position(|a| matches!(a, Action::ActivateMana { .. })) {
                    g.apply(Choice::Priority { action: Some(i) }).unwrap();
                    tapped = true;
                    break;
                }
            }
        }
        g.apply(Choice::Default).unwrap();
    }
    assert!(tapped, "no tap-for-mana action was offered with the Bolt on the stack");
    g.run();
    assert_eq!(g.players[0].pool.total(), 1);
}

#[test]
fn refused_plays_are_explained() {
    let mut g = game();
    g.add_card("Forest", 0, Zone::Battlefield);
    let f1 = g.add_card("Forest", 0, Zone::Hand);
    let f2 = g.add_card("Forest", 0, Zone::Hand);
    let bears = g.add_card("Grizzly Bears", 0, Zone::Hand);
    let bolt = g.add_card("Lightning Bolt", 0, Zone::Hand);
    g.start_at(0, Step::Main1);
    // two lands in hand, one land drop: the second is refused with the reason
    let _ = f1;
    g.players[0].lands_played = 1;
    let why = g.explain_play(0, f2);
    assert!(why.contains("already played 1 land"), "{why}");
    // not enough mana (one Forest, Bears cost {1}{G}); it is tapped?
    let why = g.explain_play(0, bears);
    assert!(why.contains("costs {1}{G}"), "{why}");
    // a Mountain is missing for the Bolt
    let why = g.explain_play(0, bolt);
    assert!(why.contains("nothing you control can make {R}"), "{why}");
    // the opponent's turn: a sorcery-speed card
    g.start_at(1, Step::Main1);
    let why = g.explain_play(0, bears);
    assert!(why.contains("only on your own turn"), "{why}");
}

#[test]
fn refused_blocks_are_explained() {
    let mut g = game();
    let angel = g.add_card("Serra Angel", 0, Zone::Battlefield);
    let bears = g.add_card("Grizzly Bears", 1, Zone::Battlefield);
    g.start_at(0, Step::DeclareAttackers);
    let why = g.why_cant_block(bears, angel).expect("a flier can't be blocked by Bears");
    assert!(why.contains("flying"), "{why}");
}

#[test]
fn cancelling_the_target_of_an_activated_ability_undoes_the_payment() {
    let mut g = game();
    g.add_card("Rod of Ruin", 0, Zone::Battlefield);
    for _ in 0..3 {
        g.add_card("Mountain", 0, Zone::Battlefield);
    }
    g.add_card("Grizzly Bears", 1, Zone::Battlefield);
    g.start_at(0, Step::Main1);
    g.players[0].auto_skip = false;
    g.players[0].allow_cancel = true;
    let d = g.next_decision().unwrap();
    let DecisionKind::Priority { actions, .. } = &d.kind else { panic!() };
    let i = actions.iter().position(|a| matches!(a, Action::Activate { .. })).expect("Rod of Ruin can be activated");
    g.apply(Choice::Priority { action: Some(i) }).unwrap();
    let mut cancelled = false;
    for _ in 0..30 {
        let Some(d) = g.next_decision() else { break };
        match &d.kind {
            DecisionKind::PayMana { .. } => g.apply(Choice::Pay { answer: PayAnswer::Auto }).unwrap(),
            DecisionKind::Pick { cancellable: true, .. } => {
                g.apply(Choice::Cancel).unwrap();
                cancelled = true;
                break;
            }
            _ => break,
        }
    }
    assert!(cancelled, "the target question was never reached");
    let d = g.next_decision().expect("the game goes on after the cancel");
    assert!(matches!(d.kind, DecisionKind::Priority { .. }), "{:?}", d.kind);
    assert_eq!(g.battlefield.iter().filter(|id| g.card(**id).tapped).count(), 0, "everything is untapped again");
}

#[test]
fn cancelling_a_spell_at_its_target_returns_it_to_the_hand_and_untaps_the_land() {
    let mut g = game();
    g.add_card("Mountain", 0, Zone::Battlefield);
    let bolt = g.add_card("Lightning Bolt", 0, Zone::Hand);
    g.start_at(0, Step::Main1);
    g.players[0].auto_skip = false;
    g.players[0].allow_cancel = true;
    let d = g.next_decision().unwrap();
    let DecisionKind::Priority { actions, .. } = &d.kind else { panic!() };
    let i = actions.iter().position(|a| matches!(a, Action::Cast { .. })).unwrap();
    g.apply(Choice::Priority { action: Some(i) }).unwrap();
    for _ in 0..10 {
        let d = g.next_decision().unwrap();
        match &d.kind {
            DecisionKind::PayMana { .. } => g.apply(Choice::Pay { answer: PayAnswer::Auto }).unwrap(),
            DecisionKind::Pick { cancellable: true, .. } => {
                g.apply(Choice::Cancel).unwrap();
                break;
            }
            _ => break,
        }
    }
    let d = g.next_decision().expect("the game goes on");
    assert!(matches!(d.kind, DecisionKind::Priority { .. }));
    assert_eq!(g.card(bolt).zone, Zone::Hand);
    assert!(g.battlefield.iter().all(|id| !g.card(*id).tapped));
}

fn notices(g: &Game) -> Vec<String> {
    g.events.iter().filter_map(|e| if let Event::Notice { text, .. } = e { Some(text.clone()) } else { None }).collect()
}

#[test]
fn ignored_blocks_say_why() {
    let mut g = game();
    let brute = g.add_card("Boggart Brute", 0, Zone::Battlefield);
    let angel = g.add_card("Serra Angel", 0, Zone::Battlefield);
    let bears = g.add_card("Grizzly Bears", 1, Zone::Battlefield);
    g.start_at(0, Step::Main1);
    g.attack(&[(brute, Obj::player(1)), (angel, Obj::player(1))]);
    // the Bears block the Brute alone (menace) and the Angel (flying): both ignored, with reasons
    g.block(&[(bears, brute), (bears, angel)]);
    g.run();
    let n = notices(&g).join(" | ");
    assert!(n.contains("menace"), "{n}");
    assert!(n.contains("flying"), "{n}");
}

/// Rule 800.4a: when a player leaves a multiplayer game, everything they own leaves with them, effects giving them control
/// end (a stolen creature goes back), and what they still control is exiled.
#[test]
fn leaving_player_takes_their_cards_and_gives_back_stolen_ones() {
    let mut g = Game::new_blank(db(), 3, 1);
    let bears = g.add_card("Grizzly Bears", 0, Zone::Battlefield);
    let giant = g.add_card("Hill Giant", 0, Zone::Battlefield);
    for _ in 0..3 {
        g.add_card("Mountain", 1, Zone::Battlefield);
    }
    for _ in 0..4 {
        g.add_card("Island", 1, Zone::Battlefield);
    }
    g.add_card("Threaten", 1, Zone::Hand);
    g.add_card("Control Magic", 1, Zone::Hand);
    let own = g.add_card("Elite Vanguard", 1, Zone::Battlefield);
    let in_hand = g.add_card("Lightning Bolt", 1, Zone::Hand);
    let in_yard = g.add_card("Shock", 1, Zone::Graveyard);
    g.start_at(1, Step::Main1);
    g.cast(1, "Threaten", &[Obj::card(bears)]);
    g.resolve_stack();
    g.cast(1, "Control Magic", &[Obj::card(giant)]);
    g.resolve_stack();
    assert_eq!(g.card(bears).cur.controller, 1, "Threaten took the bears");
    assert_eq!(g.card(giant).cur.controller, 1, "Control Magic took the giant");
    g.concede(1);
    assert!(g.players[1].lost);
    assert!(g.over.is_none(), "two players are left");
    // the stolen creatures are back with their owner, still on the battlefield
    assert_eq!(g.card(bears).zone, Zone::Battlefield);
    assert_eq!(g.card(bears).cur.controller, 0);
    assert_eq!(g.card(giant).zone, Zone::Battlefield);
    assert_eq!(g.card(giant).cur.controller, 0);
    // everything player 1 owned is out of the game, in every zone
    // its own permanent ceases to exist (it is not put anywhere); its other cards are simply out of the game
    assert!(g.card(own).zone != Zone::Battlefield && g.card(own).token, "Elite Vanguard is still in the game");
    for id in [in_hand, in_yard] {
        assert_eq!(g.card(id).zone, Zone::Sideboard, "{} is still in the game", g.card_name(id));
    }
    assert!(g.players[1].hand.is_empty() && g.players[1].graveyard.is_empty() && g.players[1].library.is_empty());
    assert_eq!(g.count_in("Control Magic", Zone::Battlefield), 0);
}

/// Auto-pay keeps the colours the rest of the hand needs: generic mana comes from lands whose colour nothing else wants,
/// and a dual land is kept back when a basic can pay instead.
#[test]
fn auto_pay_keeps_colours_the_hand_needs() {
    let untapped = |g: &Game, name: &str| g.battlefield.iter().filter(|id| g.card_name(**id) == name && !g.card(**id).tapped).count();
    // {1} with Plains + Mountain, holding Lightning Bolt: the Plains pays
    let mut g = game();
    g.add_card("Plains", 0, Zone::Battlefield);
    g.add_card("Mountain", 0, Zone::Battlefield);
    g.add_card("Sol Ring", 0, Zone::Hand);
    g.add_card("Lightning Bolt", 0, Zone::Hand);
    g.start_at(0, Step::Main1);
    g.cast(0, "Sol Ring", &[]);
    assert_eq!(untapped(&g, "Mountain"), 1, "the Mountain is kept for Lightning Bolt");
    // {1}{G} with Forest + Mountain + Taiga, holding Lightning Bolt and Giant Growth: Forest + Mountain pay, the Taiga
    // (either colour) is kept
    let mut g = game();
    g.add_card("Forest", 0, Zone::Battlefield);
    g.add_card("Mountain", 0, Zone::Battlefield);
    g.add_card("Taiga", 0, Zone::Battlefield);
    g.add_card("Grizzly Bears", 0, Zone::Hand);
    g.add_card("Lightning Bolt", 0, Zone::Hand);
    g.add_card("Giant Growth", 0, Zone::Hand);
    g.start_at(0, Step::Main1);
    g.cast(0, "Grizzly Bears", &[]);
    assert_eq!(untapped(&g, "Taiga"), 1, "the dual land is kept");
    // {1}{G} with Forest + Forest + Mountain, holding Lightning Bolt: both Forests pay
    let mut g = game();
    g.add_card("Forest", 0, Zone::Battlefield);
    g.add_card("Forest", 0, Zone::Battlefield);
    g.add_card("Mountain", 0, Zone::Battlefield);
    g.add_card("Grizzly Bears", 0, Zone::Hand);
    g.add_card("Lightning Bolt", 0, Zone::Hand);
    g.start_at(0, Step::Main1);
    g.cast(0, "Grizzly Bears", &[]);
    assert_eq!(untapped(&g, "Mountain"), 1, "the Mountain is kept for Lightning Bolt");
}

/// Auto-pay taps a land before a mana creature, and keeps the colour more cards in hand need.
#[test]
fn auto_pay_prefers_lands_and_the_colour_most_needed() {
    let untapped = |g: &Game, name: &str| g.battlefield.iter().filter(|id| g.card_name(**id) == name && !g.card(**id).tapped).count();
    // {G} with a Forest and an untapped Llanowar Elves: the land pays, the creature stays free to attack or block
    let mut g = game();
    g.add_card("Forest", 0, Zone::Battlefield);
    g.add_card("Llanowar Elves", 0, Zone::Battlefield); // set-up permanents are not summoning sick
    g.add_card("Giant Growth", 0, Zone::Hand);
    let bears = g.add_card("Grizzly Bears", 0, Zone::Battlefield);
    g.start_at(0, Step::Main1);
    g.cast(0, "Giant Growth", &[Obj::card(bears)]);
    let elves_free = untapped(&g, "Llanowar Elves");
    // {1}{G} with Forest x2 + Mountain x2, holding two red cards and one green: keep both Mountains
    let mut g2 = game();
    for n in ["Forest", "Forest", "Mountain", "Mountain"] {
        g2.add_card(n, 0, Zone::Battlefield);
    }
    g2.add_card("Grizzly Bears", 0, Zone::Hand);
    g2.add_card("Lightning Bolt", 0, Zone::Hand);
    g2.add_card("Raging Goblin", 0, Zone::Hand);
    g2.start_at(0, Step::Main1);
    g2.cast(0, "Grizzly Bears", &[]);
    let mountains = untapped(&g2, "Mountain");
    assert_eq!(elves_free, 1, "the Forest pays, Llanowar Elves stays untapped");
    assert_eq!(mountains, 2, "both Mountains are kept for the two red cards");
}

/// The block preview (for the arrows while declaring blockers): menace drops a single block with a reason, and a
/// trampler's damage goes to its blocker and, past lethal, to the player.
#[test]
fn block_preview_reports_dropped_blocks_and_damage_targets() {
    let mut g = game();
    let brute = g.add_card("Boggart Brute", 0, Zone::Battlefield); // 3/2 menace
    let dreadmaw = g.add_card("Colossal Dreadmaw", 0, Zone::Battlefield); // 6/6 trample
    let bears = g.add_card("Grizzly Bears", 1, Zone::Battlefield);
    let elf = g.add_card("Llanowar Elves", 1, Zone::Battlefield);
    g.start_at(0, Step::Main1);
    g.attack(&[(brute, Obj::player(1)), (dreadmaw, Obj::player(1))]);
    g.to_blockers();
    let p = g.preview_blocks(&[(bears, brute), (elf, dreadmaw)]);
    let of = |a: CardId| p.iter().find(|x| x.attacker == a).unwrap();
    assert!(of(brute).blockers.is_empty(), "one blocker can't block a menace creature");
    assert!(of(brute).dropped.iter().any(|(b, why)| *b == bears && why.contains("menace")), "{:?}", of(brute).dropped);
    assert_eq!(of(brute).damage_to, vec![Obj::player(1)], "the brute is unblocked, so it hits the player");
    assert_eq!(of(dreadmaw).blockers, vec![elf]);
    assert_eq!(of(dreadmaw).damage_to, vec![Obj::card(elf), Obj::player(1)], "trample: lethal to the elf, the rest to the player");
    // two blockers on the menace creature are fine; its 3 damage reaches the first blocker only partly beyond lethal
    let p = g.preview_blocks(&[(bears, brute), (elf, brute)]);
    let b = p.iter().find(|x| x.attacker == brute).unwrap();
    assert_eq!(b.blockers.len(), 2);
    assert!(b.dropped.is_empty());
    assert_eq!(b.damage_to, vec![Obj::card(bears), Obj::card(elf)]);
}

/// "Break the rules" also mills, attaches, transforms, pumps and changes poison and commander damage.
#[test]
fn rule_break_moves_top_cards_attaches_transforms_and_pumps() {
    let mut g = game();
    let bears = g.add_card("Grizzly Bears", 0, Zone::Battlefield);
    let rancor = g.add_card("Rancor", 0, Zone::Battlefield);
    let delver = g.add_card("Delver of Secrets // Insectile Aberration", 0, Zone::Battlefield);
    for _ in 0..3 {
        g.add_card("Forest", 1, Zone::Library);
    }
    g.start_at(0, Step::Main1);
    g.apply_edit(0, &EditOp::TopCards { player: 1, n: 2, to: Zone::Graveyard }).unwrap();
    assert_eq!((g.players[1].library.len(), g.players[1].graveyard.len()), (1, 2));
    g.apply_edit(0, &EditOp::Attach { card: rancor, to: Some(bears) }).unwrap();
    assert_eq!(g.card(rancor).attached_to, Some(bears));
    g.apply_edit(0, &EditOp::Transform { card: delver }).unwrap();
    assert_eq!(g.card_name(delver), "Insectile Aberration");
    assert!(g.apply_edit(0, &EditOp::Transform { card: bears }).is_err(), "no other face");
    g.apply_edit(0, &EditOp::Pump { card: bears, power: 3, toughness: 3, eot: false }).unwrap();
    g.run();
    assert_eq!((g.card(bears).cur.power, g.card(bears).cur.toughness), (7, 5), "2/2, +2/+0 Rancor, +3/+3");
    g.apply_edit(0, &EditOp::PlayerCounters { player: 1, kind: PlayerCounter::Poison, delta: 3 }).unwrap();
    assert_eq!(g.players[1].poison, 3);
    g.apply_edit(0, &EditOp::CommanderDamage { player: 1, commander: bears, delta: 5 }).unwrap();
    assert_eq!(g.players[1].commander_damage.as_slice(), &[(bears, 5)]);
}

/// "Break the rules" does everything the sandbox does: face down, meld, notes, "doesn't untap", markers, dungeons,
/// commander tax, and taking another player's cards.
#[test]
fn rule_break_turns_face_down_melds_takes_cards_and_sets_markers() {
    let mut g = game();
    let bears = g.add_card("Grizzly Bears", 0, Zone::Battlefield);
    let bruna = g.add_card("Bruna, the Fading Light", 0, Zone::Battlefield);
    let gisela = g.add_card("Gisela, the Broken Blade", 0, Zone::Battlefield);
    let theirs = g.add_card("Lightning Bolt", 1, Zone::Hand);
    g.add_card("Forest", 1, Zone::Graveyard);
    g.start_at(0, Step::Main1);
    g.apply_edit(0, &EditOp::FaceDown { card: bears, face_down: true }).unwrap();
    g.run();
    assert!(g.card(bears).face_down && g.card(bears).cur.power == 2);
    g.apply_edit(0, &EditOp::FaceDown { card: bears, face_down: false }).unwrap();
    assert!(!g.card(bears).face_down);
    g.apply_edit(0, &EditOp::Meld { card: gisela, with: bruna, into: None }).unwrap();
    assert!(g.battlefield.iter().any(|id| g.card_name(*id) == "Brisela, Voice of Nightmares"));
    // taking a card from another player's hand, and a random one from their graveyard
    g.apply_edit(0, &EditOp::Move { card: theirs, to: Zone::Hand, bottom: false, controller: None, owner: Some(0) }).unwrap();
    assert!(g.players[0].hand.contains(&theirs) && !g.players[1].hand.contains(&theirs) && g.card(theirs).owner == 0);
    g.apply_edit(0, &EditOp::RandomCard { player: 1, zone: Zone::Graveyard, to: Zone::Hand, owner: Some(0) }).unwrap();
    assert!(g.players[1].graveyard.is_empty() && g.players[0].hand.len() == 2);
    // notes and "doesn't untap" show on the card
    g.apply_edit(0, &EditOp::Note { card: bears, text: "can't block".into(), eot: true, remove: false }).unwrap();
    g.apply_edit(0, &EditOp::NoUntap { card: bears, value: true }).unwrap();
    let v = g.view_for(0);
    let cv = v.cards.iter().find(|c| c.id == bears).unwrap();
    assert_eq!(cv.notes, vec!["can't block · EOT".to_string()]);
    assert!(cv.no_untap && g.doesnt_untap(bears));
    g.apply_edit(0, &EditOp::Monarch { player: Some(1) }).unwrap();
    g.apply_edit(0, &EditOp::Ring { player: 0, delta: 9 }).unwrap();
    g.apply_edit(0, &EditOp::Marker { player: Some(1), label: "rad".into(), delta: 3, remove: false }).unwrap();
    g.apply_edit(0, &EditOp::CommanderTax { player: 0, commander: bears, delta: 2 }).unwrap();
    g.apply_edit(0, &EditOp::Dungeon { player: 0, dungeon: Some("Lost Mine of Phandelver".into()), room: "Goblin Lair".into() }).unwrap();
    let v = g.view_for(0);
    assert!(v.players[1].monarch);
    assert_eq!(v.players[0].ring, 4);
    assert_eq!(v.players[1].markers, vec![("rad".to_string(), 3)]);
    assert_eq!(v.players[0].commander_tax, vec![(bears, 2)]);
    assert_eq!(v.players[0].dungeon, Some(("Lost Mine of Phandelver".to_string(), "Goblin Lair".to_string())));
}

/// "Break the rules" can make the tokens a card failed to make: a named token, and token copies of a permanent.
#[test]
fn rule_break_creates_tokens_and_token_copies() {
    let mut g = game();
    let bears = g.add_card("Grizzly Bears", 1, Zone::Battlefield);
    g.start_at(0, Step::Main1);
    g.apply_edit(0, &EditOp::CreateToken { player: 0, name: "Soldier".into(), n: 2, tapped: false }).unwrap();
    let soldiers: Vec<CardId> = g.battlefield.iter().copied().filter(|id| g.card_name(*id) == "Soldier").collect();
    assert_eq!(soldiers.len(), 2);
    assert!(soldiers.iter().all(|id| g.card(*id).token && g.card(*id).controller == 0));
    g.apply_edit(0, &EditOp::TokenCopy { card: bears, player: 0, n: 1 }).unwrap();
    let copies: Vec<CardId> = g.battlefield.iter().copied().filter(|id| g.card_name(*id) == "Grizzly Bears" && g.card(*id).token).collect();
    assert_eq!(copies.len(), 1);
    assert_eq!(g.card(copies[0]).controller, 0);
    assert!(g.apply_edit(0, &EditOp::CreateToken { player: 0, name: "No Such Token".into(), n: 1, tapped: false }).is_err());
}

#[test]
fn hand_card_views_name_foretell_and_alternative_costs() {
    let mut g = game();
    let doomskar = g.add_card("Doomskar", 0, Zone::Hand);
    let mulldrifter = g.add_card("Mulldrifter", 0, Zone::Hand);
    g.start_at(0, Step::Main1);
    let view = g.view_for(0);
    let v = view.cards.iter().find(|c| c.id == doomskar).unwrap();
    assert!(v.abilities.iter().any(|a| a.text.starts_with("Foretell ({2}")), "{:?}", v.abilities);
    assert_eq!(v.alt_costs.len(), 1, "{:?}", v.alt_costs);
    assert!(v.alt_costs[0].starts_with("from exile: "), "{:?}", v.alt_costs);
    let v = view.cards.iter().find(|c| c.id == mulldrifter).unwrap();
    assert_eq!(v.alt_costs, vec!["{2}{U}".to_string()]);
}

#[test]
fn bounce_land_taps_for_both_colours_as_one_output() {
    let mut g = game();
    let farm = g.add_card("Golgari Rot Farm", 0, Zone::Battlefield);
    let guild = g.add_card("Command Tower", 0, Zone::Battlefield);
    g.start_at(0, Step::Main1);
    let srcs = g.mana_sources(0);
    let f = srcs.iter().find(|s| s.card == farm).unwrap();
    assert_eq!(f.outputs.len(), 1, "{:?}", f.outputs);
    assert_eq!(f.outputs[0].len(), 2, "{:?}", f.outputs);
    let _ = guild;
}

#[test]
fn a_clone_shows_what_it_copies() {
    let mut g = game();
    let bears = g.add_card("Grizzly Bears", 0, Zone::Battlefield);
    let clone = g.add_card("Clone", 0, Zone::Hand);
    for _ in 0..4 {
        g.add_card("Island", 0, Zone::Battlefield);
    }
    g.start_at(0, Step::Main1);
    g.cast(0, "Clone", &[]);
    g.resolve_stack();
    // the enter-as-a-copy choice: copy the Bears
    for _ in 0..5 {
        let Some(d) = g.pending.clone() else { break };
        match d.kind {
            DecisionKind::Pick { options, .. } => {
                let i = options.iter().position(|o| matches!(o, Opt::Obj { obj: Obj::Card { id } } if *id == bears)).unwrap_or(0);
                g.apply(Choice::Pick { indices: vec![i] }).unwrap();
            }
            DecisionKind::YesNo { .. } => g.apply(Choice::YesNo { yes: true }).unwrap(),
            _ => break,
        }
        g.resolve_stack();
    }
    let view = g.view_for(0);
    let v = view.cards.iter().find(|c| c.id == clone).unwrap();
    assert_eq!(v.copy_of.as_deref(), Some("Grizzly Bears"), "{:?}", v.name);
    assert_eq!(view.cards.iter().find(|c| c.id == bears).unwrap().copy_of, None);
}

/// Answer enter-as-a-copy questions by picking `want` (or yes to a may).
fn answer_copy_choice(g: &mut Game, want: CardId) {
    for _ in 0..5 {
        let Some(d) = g.pending.clone() else { break };
        match d.kind {
            DecisionKind::Pick { options, .. } => {
                let i = options.iter().position(|o| matches!(o, Opt::Obj { obj: Obj::Card { id } } if *id == want)).unwrap_or(0);
                g.apply(Choice::Pick { indices: vec![i] }).unwrap();
            }
            DecisionKind::YesNo { .. } => g.apply(Choice::YesNo { yes: true }).unwrap(),
            _ => break,
        }
        g.resolve_stack();
    }
}

#[test]
fn chameleon_keeps_its_name_and_looks_like_what_it_copies() {
    let mut g = game();
    let bears = g.add_card("Grizzly Bears", 0, Zone::Battlefield);
    let cham = g.add_card("Chameleon, Master of Disguise", 0, Zone::Hand);
    for _ in 0..4 {
        g.add_card("Island", 0, Zone::Battlefield);
    }
    g.start_at(0, Step::Main1);
    g.cast(0, "Chameleon, Master of Disguise", &[]);
    g.resolve_stack();
    answer_copy_choice(&mut g, bears);
    let view = g.view_for(1);
    let v = view.cards.iter().find(|c| c.id == cham).unwrap();
    assert_eq!(v.name, "Chameleon, Master of Disguise");
    assert_eq!(v.copy_of.as_deref(), Some("Grizzly Bears"));
    assert_eq!((v.art.as_str(), v.art_token), ("Grizzly Bears", false));
    assert_eq!((v.power, v.toughness), (Some(2), Some(2)));
    assert!(v.types.contains("Bear"), "{}", v.types);
    // the original is untouched
    let b = view.cards.iter().find(|c| c.id == bears).unwrap();
    assert_eq!((b.copy_of.clone(), b.art.as_str(), b.name.as_str()), (None, "Grizzly Bears", "Grizzly Bears"));
}

#[test]
fn token_copies_and_copies_of_tokens_show_what_they_copy() {
    let mut g = game();
    let bears = g.add_card("Grizzly Bears", 0, Zone::Battlefield);
    let clone = g.add_card("Clone", 0, Zone::Hand);
    for _ in 0..4 {
        g.add_card("Island", 0, Zone::Battlefield);
    }
    g.start_at(0, Step::Main1);
    // a token copy of a card looks like the card
    g.apply_edit(0, &EditOp::TokenCopy { card: bears, player: 0, n: 1 }).unwrap();
    g.apply_edit(0, &EditOp::CreateToken { player: 0, name: "Soldier".into(), n: 1, tapped: false }).unwrap();
    let soldier = *g.battlefield.iter().find(|id| g.card_name(**id) == "Soldier").unwrap();
    let view = g.view_for(0);
    let t = view.cards.iter().find(|c| c.token && c.name == "Grizzly Bears").unwrap();
    assert_eq!((t.copy_of.as_deref(), t.art.as_str(), t.art_token), (Some("Grizzly Bears"), "Grizzly Bears", false));
    // a plain token is not a copy and its picture is a token's
    let s = view.cards.iter().find(|c| c.id == soldier).unwrap();
    assert_eq!((s.copy_of.clone(), s.art.as_str(), s.art_token), (None, "Soldier", true));
    // Clone copying the Soldier token: a copy of a token, pictured as that token
    g.cast(0, "Clone", &[]);
    g.resolve_stack();
    answer_copy_choice(&mut g, soldier);
    let view = g.view_for(0);
    let c = view.cards.iter().find(|c| c.id == clone).unwrap();
    assert_eq!((c.copy_of.as_deref(), c.art.as_str(), c.art_token, c.name.as_str()), (Some("Soldier"), "Soldier", true, "Soldier"));
}

#[test]
fn granted_alternative_costs_are_named() {
    let mut g = game();
    g.add_card("Omniscience", 0, Zone::Battlefield);
    let giant = g.add_card("Hill Giant", 0, Zone::Hand);
    g.start_at(0, Step::Main1);
    g.run();
    let view = g.view_for(0);
    let v = view.cards.iter().find(|c| c.id == giant).unwrap();
    assert!(v.granted_alts.iter().any(|t| t.contains("(Omniscience)")), "{:?}", v.granted_alts);
}

#[test]
fn melded_and_mutated_permanents_are_not_called_copies() {
    // mutate over Grizzly Bears: the pile is a Gemrazer, not a copy of one
    let mut g = game();
    let bears = g.add_card("Grizzly Bears", 0, Zone::Battlefield);
    let gem = g.add_card("Gemrazer", 0, Zone::Hand);
    for _ in 0..3 {
        g.add_card("Forest", 0, Zone::Battlefield);
    }
    g.add_card("Pacifism", 1, Zone::Battlefield);
    g.start_at(0, Step::Main1);
    assert!(g.act(0, |_, a| matches!(a, Action::Cast { card, alt: Some(0) } if *card == gem)));
    g.answer_targets(&[Obj::card(bears)]);
    g.resolve_stack();
    if let Some(DecisionKind::YesNo { .. }) = g.pending.as_ref().map(|d| d.kind.clone()) {
        g.apply(Choice::YesNo { yes: true }).unwrap();
        g.resolve_stack();
    }
    let view = g.view_for(0);
    let pile = view.cards.iter().find(|c| c.zone == Zone::Battlefield && c.name == "Gemrazer").expect("mutated over");
    assert_eq!((pile.copy_of.clone(), pile.art.as_str()), (None, "Gemrazer"));
    // Graf Rats + Midnight Scavengers meld into Chittering Host at the beginning of combat
    let mut g = game();
    g.add_card("Graf Rats", 0, Zone::Battlefield);
    g.add_card("Midnight Scavengers", 0, Zone::Battlefield);
    g.start_at(0, Step::Main1);
    g.pass_to(Step::BeginCombat);
    g.resolve_stack();
    let view = g.view_for(0);
    let host = view.cards.iter().find(|c| c.name == "Chittering Host").expect("melded");
    assert_eq!((host.copy_of.clone(), host.art.as_str(), host.art_token), (None, "Chittering Host", false));
}

#[test]
fn treasures_pay_for_a_spell() {
    let mut g = game();
    g.add_card("Chameleon, Master of Disguise", 0, Zone::Hand);
    g.add_card("Grizzly Bears", 0, Zone::Hand);
    g.start_at(0, Step::Main1);
    g.apply_edit(0, &EditOp::CreateToken { player: 0, name: "Treasure".into(), n: 4, tapped: false }).unwrap();
    g.run();
    let Some(DecisionKind::Priority { actions: acts, .. }) = g.pending.as_ref().map(|d| d.kind.clone()) else { panic!("no priority") };
    let names: Vec<String> = acts.iter().filter_map(|a| if let Action::Cast { card, alt: None } = a { Some(g.card_name(*card).to_string()) } else { None }).collect();
    assert!(names.iter().any(|n| n == "Grizzly Bears"), "{acts:?}");
    assert!(names.iter().any(|n| n == "Chameleon, Master of Disguise"), "{acts:?}");
}

// Answers come from the network as-is: ids that name nothing (or something the question doesn't offer) are refused
// with an error, never followed into an out-of-bounds index.

#[test]
fn out_of_range_combat_answers_are_refused() {
    let mut g = game();
    let a = g.add_card("Grizzly Bears", 0, Zone::Battlefield);
    let b = g.add_card("Grizzly Bears", 1, Zone::Battlefield);
    g.start_at(0, Step::Main1);
    g.to_attackers();
    assert!(g.apply(Choice::Attackers { attacks: vec![(CardId(999_999), Obj::player(1))] }).is_err());
    assert!(g.apply(Choice::Attackers { attacks: vec![(a, Obj::card(CardId(999_999)))] }).is_err());
    g.apply(Choice::Attackers { attacks: vec![(a, Obj::player(1))] }).unwrap();
    g.to_blockers();
    assert!(g.apply(Choice::Blockers { blocks: vec![(CardId(999_999), CardId(0))] }).is_err());
    assert!(g.apply(Choice::Blockers { blocks: vec![(b, CardId(999_999))] }).is_err());
    // the question is still open and a legal answer works as before
    g.apply(Choice::Blockers { blocks: vec![(b, a)] }).unwrap();
    g.finish_combat();
    assert_eq!(g.count_in("Grizzly Bears", Zone::Graveyard), 2);
}

#[test]
fn damage_assignment_must_go_to_the_blockers_and_add_up() {
    let mut g = game();
    let giant = g.add_card("Hill Giant", 0, Zone::Battlefield);
    let b1 = g.add_card("Grizzly Bears", 1, Zone::Battlefield);
    let b2 = g.add_card("Grizzly Bears", 1, Zone::Battlefield);
    let other = g.add_card("Grizzly Bears", 1, Zone::Battlefield);
    g.start_at(0, Step::Main1);
    g.attack(&[(giant, Obj::player(1))]);
    g.block(&[(b1, giant), (b2, giant)]);
    let mut d = g.next_decision().unwrap();
    while matches!(d.kind, DecisionKind::Priority { .. }) {
        g.apply(Choice::Priority { action: None }).unwrap();
        d = g.next_decision().unwrap();
    }
    let DecisionKind::AssignDamage { total, .. } = d.kind else { panic!("{:?}", d.kind) };
    assert_eq!(total, 3);
    assert!(g.apply(Choice::Damage { assign: vec![(Obj::card(other), 3)] }).is_err(), "not a blocker");
    assert!(g.apply(Choice::Damage { assign: vec![(Obj::player(1), 3)] }).is_err(), "no trample");
    assert!(g.apply(Choice::Damage { assign: vec![(Obj::card(CardId(999_999)), 3)] }).is_err());
    // sums that only match after wrapping around
    assert!(g.apply(Choice::Damage { assign: vec![(Obj::card(b1), i32::MAX), (Obj::card(b1), i32::MAX), (Obj::card(b2), 5)] }).is_err());
    g.apply(Choice::Damage { assign: vec![(Obj::card(b1), 2), (Obj::card(b2), 1)] }).unwrap();
    g.finish_combat();
    assert_eq!(g.card(b1).zone, Zone::Graveyard);
    assert_eq!(g.card(other).zone, Zone::Battlefield);
}

#[test]
fn hand_tapping_a_source_the_payment_does_not_offer_is_refused() {
    let mut g = game();
    let mine = g.add_card("Mountain", 0, Zone::Battlefield);
    let theirs = g.add_card("Mountain", 1, Zone::Battlefield);
    g.add_card("Lightning Bolt", 0, Zone::Hand);
    g.start_at(0, Step::Main1);
    g.players[0].manual_pay = true;
    let d = g.next_decision().unwrap();
    let DecisionKind::Priority { actions, .. } = &d.kind else { panic!() };
    let i = actions.iter().position(|a| matches!(a, Action::Cast { .. })).unwrap();
    g.apply(Choice::Priority { action: Some(i) }).unwrap();
    let mut paid = false;
    for _ in 0..10 {
        let d = g.next_decision().unwrap();
        match &d.kind {
            DecisionKind::Pick { .. } => {
                let c = g.default_choice(&d);
                g.apply(c).unwrap();
            }
            DecisionKind::PayMana { sources, .. } => {
                let ab = sources.iter().find(|s| s.card == mine).expect("my Mountain is offered").ability;
                assert!(g.apply(Choice::Pay { answer: PayAnswer::Tap { card: CardId(999_999), ability: 0, produce: None } }).is_err());
                assert!(g.apply(Choice::Pay { answer: PayAnswer::Tap { card: theirs, ability: ab, produce: None } }).is_err());
                assert!(g.apply(Choice::Pay { answer: PayAnswer::Tap { card: mine, ability: 999, produce: None } }).is_err());
                g.apply(Choice::Pay { answer: PayAnswer::Tap { card: mine, ability: ab, produce: None } }).unwrap();
                paid = true;
                break;
            }
            k => panic!("unexpected {k:?}"),
        }
    }
    assert!(paid, "no payment question");
    g.resolve_stack();
    assert!(g.card(mine).tapped);
    assert!(!g.card(theirs).tapped);
    assert_eq!(g.count_in("Lightning Bolt", Zone::Graveyard), 1);
}

/// A face-up plane's "players play with the top card of their libraries revealed" (Skybreen) works from the
/// command zone: the opponent sees the top card of my library.
#[test]
fn plane_reveals_library_tops() {
    let mut g = game();
    let top = g.add_card("Grizzly Bears", 0, Zone::Library);
    g.start_at(0, Step::Main1);
    assert!(!g.view_for(1).cards.iter().any(|c| c.id == top), "hidden without the plane");
    let plane = g.add_card("Skybreen", 0, Zone::Command);
    g.planes.push(plane);
    assert_eq!(g.players[0].library.last(), Some(&top));
    assert!(g.view_for(1).cards.iter().any(|c| c.id == top && c.zone == Zone::Library), "Skybreen reveals the top card");
}

/// "Can't be blocked this turn" from a resolved spell is a rules effect, not an ability: a later "loses all
/// abilities" (Humble) doesn't remove it, while a granted real keyword (Jump's flying) is lost.
#[test]
fn resolved_unblockable_survives_losing_all_abilities() {
    let mut g = game();
    for _ in 0..3 {
        g.add_card("Island", 0, Zone::Battlefield);
    }
    for _ in 0..2 {
        g.add_card("Plains", 0, Zone::Battlefield);
    }
    g.add_card("Ghostform", 0, Zone::Hand);
    g.add_card("Jump", 0, Zone::Hand);
    g.add_card("Humble", 0, Zone::Hand);
    let bears = g.add_card("Grizzly Bears", 0, Zone::Battlefield);
    g.start_at(0, Step::Main1);
    g.cast(0, "Ghostform", &[Obj::card(bears)]);
    g.resolve_stack();
    g.cast(0, "Jump", &[Obj::card(bears)]);
    g.resolve_stack();
    assert!(g.card(bears).cur.kw.has(Keyword::Unblockable));
    assert!(g.card(bears).cur.kw.has(Keyword::Flying));
    g.cast(0, "Humble", &[Obj::card(bears)]);
    g.resolve_stack();
    assert!(g.card(bears).cur.no_abilities, "Humble resolved");
    assert!(!g.card(bears).cur.kw.has(Keyword::Flying), "granted flying is an ability and is lost");
    assert!(g.card(bears).cur.kw.has(Keyword::Unblockable), "can't be blocked this turn is a rules effect");
}

#[test]
fn lotus_field_trigger_stops_so_lands_can_be_tapped_before_the_sacrifice() {
    let mut g = game();
    g.add_card("Forest", 0, Zone::Battlefield);
    g.add_card("Forest", 0, Zone::Battlefield);
    g.add_card("Lotus Field", 0, Zone::Hand);
    g.players[0].auto_skip = true;
    g.players[0].offer_mana = true;
    g.start_at(0, Step::Main1);
    g.play_land(0, "Lotus Field");
    // the trigger is on the stack and nothing could be cast: still asked, with the Forests' mana abilities offered
    let d = g.next_decision().unwrap();
    let DecisionKind::Priority { actions, stack_len } = d.kind else { panic!("{:?}", d.kind) };
    assert_eq!((d.player, stack_len), (0, 1));
    assert!(actions.iter().any(|a| matches!(a, Action::ActivateMana { .. })), "{actions:?}");
}

/// The steps where `asked` is offered priority in a combat where player 1's Grizzly Bears attack player 0 (or player 0's
/// attack player 1 when `p0_attacks`), the other player blocking with their own Bears. `asked` holds Giant Growth and a Forest.
fn combat_priority_steps(asked: PlayerId, p0_attacks: bool, pass_until: Option<PassUntil>) -> Vec<Step> {
    let mut g = game();
    g.add_card("Forest", asked, Zone::Battlefield);
    g.add_card("Giant Growth", asked, Zone::Hand);
    let b0 = g.add_card("Grizzly Bears", 0, Zone::Battlefield);
    let b1 = g.add_card("Grizzly Bears", 1, Zone::Battlefield);
    let (active, attacker, blocker, defender) = if p0_attacks { (0, b0, b1, 1) } else { (1, b1, b0, 0) };
    g.start_at(active, Step::Main1);
    for p in g.players.iter_mut() {
        p.full_control = false;
        p.auto_skip = true;
        p.stops_mine = 0;
        p.stops_opp = 0;
    }
    g.players[asked as usize].pass_until = pass_until;
    g.attack(&[(attacker, Obj::player(defender))]);
    let mut steps = vec![];
    for _ in 0..100 {
        if g.step == Step::EndCombat || g.step == Step::Main2 {
            break;
        }
        let Some(d) = g.next_decision() else { break };
        match d.kind {
            DecisionKind::Priority { .. } => {
                if d.player == asked && matches!(g.step, Step::DeclareAttackers | Step::DeclareBlockers | Step::FirstStrikeDamage | Step::CombatDamage) {
                    steps.push(g.step);
                }
                g.apply(Choice::Priority { action: None }).unwrap();
            }
            DecisionKind::Blockers { .. } => g.apply(Choice::Blockers { blocks: vec![(blocker, attacker)] }).unwrap(),
            _ => {
                let c = g.default_choice(&d);
                g.apply(c).unwrap();
            }
        }
    }
    steps
}

#[test]
fn both_sides_of_a_combat_get_priority_after_blocks_when_they_can_act() {
    use Step::*;
    // the defender: before blocks and before damage
    assert_eq!(combat_priority_steps(0, false, None), vec![DeclareAttackers, DeclareBlockers]);
    // ... none while passing until their own turn
    assert_eq!(combat_priority_steps(0, false, Some(PassUntil::PlayerTurn { player: 0, after: 1 })), Vec::<Step>::new());
    // the attacker: before damage
    assert_eq!(combat_priority_steps(0, true, None), vec![DeclareBlockers]);
    // ... none while passing to the end step
    assert_eq!(combat_priority_steps(0, true, Some(PassUntil::Step { turn: 1, step: End })), Vec::<Step>::new());
}
