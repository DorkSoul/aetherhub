use mtg_ai::*;
use mtg_engine::*;
use std::sync::{Arc, OnceLock};

fn db() -> Arc<CardDb> {
    static DB: OnceLock<Arc<CardDb>> = OnceLock::new();
    DB.get_or_init(|| mtg_cards::build_db(&mtg_cards::default_cards_dir()).unwrap()).clone()
}

fn deck(gr: bool) -> Vec<(String, u32)> {
    let v: Vec<(&str, u32)> = if gr {
        vec![("Forest", 8), ("Mountain", 8), ("Grizzly Bears", 4), ("Hill Giant", 4), ("Llanowar Elves", 4), ("Lightning Bolt", 4), ("Giant Growth", 4), ("Raging Goblin", 4)]
    } else {
        vec![("Plains", 8), ("Swamp", 8), ("Elite Vanguard", 4), ("Serra Angel", 3), ("Typhoid Rats", 4), ("Vampire Nighthawk", 3), ("Terror", 4), ("Swords to Plowshares", 3), ("Glorious Anthem", 3)]
    };
    v.into_iter().map(|(n, c)| (n.to_string(), c)).collect()
}

fn setup(seed: u64) -> GameSetup {
    let seat = |name: &str, human: bool, gr: bool, d: Difficulty| SeatSetup { name: name.into(), human, deck: deck(gr), commander: vec![], sideboard: vec![], companion: None, difficulty: d, schemes: vec![] };
    GameSetup {
        seed,
        commander: false,
        mulligan: MulliganRule::London,
        seats: vec![seat("Human", true, true, Difficulty::Normal), seat("Bot 1", false, false, Difficulty::Normal), seat("Bot 2", false, true, Difficulty::Easy), seat("Bot 3", false, false, Difficulty::Hard)],
        planar: vec![],
    }
}

fn simple(d: &Decision) -> Choice {
    match &d.kind {
        DecisionKind::Mulligan { .. } => Choice::Keep { keep: true },
        DecisionKind::Priority { actions, .. } => Choice::Priority { action: actions.iter().position(|a| matches!(a, Action::PlayLand { .. })) },
        _ => Choice::Default,
    }
}

#[test]
fn restore_reproduces_the_same_state() {
    for seed in [7u64, 8, 9] {
        let mut s = Session::new(db(), setup(seed)).unwrap();
        for _ in 0..40 {
            let adv = s.advance();
            let Some(d) = adv.waiting else { break };
            s.answer(d.player, simple(&d)).unwrap();
        }
        let rec = s.record();
        let r = Session::restore(db(), rec.clone()).unwrap();
        // the restored game is exactly where the original was right after its last answer, waiting on the same question
        // (playing on from here is not compared: the AI searches for a wall-clock budget, so its later choices can differ)
        assert_eq!(s.game.decision_log.len(), r.game.decision_log.len());
        assert_eq!(s.game.state_hash(), r.game.state_hash(), "seed {seed}: turn {} vs {}", s.game.turn, r.game.turn);
        assert_eq!(format!("{:?}", s.game.decision()), format!("{:?}", r.game.decision()), "seed {seed}");
    }
}

#[test]
fn find_first_divergence() {
    let seed = 7u64;
    let st = setup(seed);
    let mut s = Session::new(db(), st.clone()).unwrap();
    let mut hashes: Vec<u64> = vec![];
    // drive manually: record the state hash after every applied decision
    for _ in 0..40 {
        loop {
            if s.game.run() == Status::Over {
                break;
            }
            let d = s.game.decision().cloned().unwrap();
            if d.player == 0 {
                break;
            }
            // an AI answer taken from a throw-away Session agent via advance-like path: reuse HeuristicAgent
            let mut ag = HeuristicAgent::new(seed ^ ((d.player as u64 + 1) * 0x9e37), std::time::Duration::from_millis(50));
            let c = ag.decide(&s.game, &d);
            if s.game.apply(c).is_err() {
                s.game.apply(Choice::Default).unwrap();
            }
            hashes.push(s.game.state_hash());
        }
        let Some(d) = s.game.decision().cloned() else { break };
        s.game.apply(simple(&d)).unwrap();
        hashes.push(s.game.state_hash());
    }
    let log = s.game.decision_log.clone();
    assert_eq!(log.len(), hashes.len());
    // replay from scratch, comparing after each decision
    let mut r = Session::new(db(), st).unwrap();
    for (i, c) in log.iter().enumerate() {
        if r.game.run() == Status::Over {
            break;
        }
        let d = r.game.decision().cloned().unwrap();
        r.game.apply(c.clone()).unwrap_or_else(|e| panic!("decision {i} rejected: {e}; decision was {:?}", d.kind));
        if r.game.state_hash() != hashes[i] {
            panic!("diverged at decision {i}: {:?} answered {:?} (turn {})", d.kind, c, r.game.turn);
        }
    }
}

fn duel(seed: u64) -> GameSetup {
    let seat = |name: &str, human: bool, gr: bool| SeatSetup { name: name.into(), human, deck: deck(gr), commander: vec![], sideboard: vec![], companion: None, difficulty: Difficulty::Easy, schemes: vec![] };
    GameSetup { seed, commander: false, mulligan: MulliganRule::London, seats: vec![seat("Human", true, true), seat("Bot", false, true)], planar: vec![] }
}

/// "End turn" passes every plain priority (also with spells on the stack) until the target; the passes are not in the
/// decision log, so the command itself is recorded and undo / restore reproduce the game exactly.
#[test]
fn end_turn_passes_through_the_stack_and_replays() {
    for seed in [3u64, 11] {
        let mut s = Session::new(db(), duel(seed)).unwrap();
        let mut ended = 0;
        let mut conceded = false;
        for i in 0..120 {
            let adv = s.advance();
            let Some(d) = adv.waiting else { break };
            if let DecisionKind::Priority { .. } = d.kind {
                assert!(s.passing(0).is_none(), "seed {seed}: asked for priority while passing (turn {})", s.game.turn);
                if i % 3 == 0 {
                    s.pass_until(0, s.game.turn + 1, mtg_engine::types::Step::Untap);
                    ended += 1;
                    s.answer(0, Choice::Priority { action: None }).unwrap();
                    continue;
                }
            }
            if i == 100 {
                s.concede(0);
                conceded = true;
                continue;
            }
            s.answer(d.player, simple(&d)).unwrap();
        }
        assert!(ended > 3, "seed {seed}: end turn used {ended} times");
        let rec = s.record();
        assert!(!rec.commands.is_empty());
        let mut r = Session::restore(db(), rec.clone()).unwrap();
        assert_eq!(s.game.decision_log.len(), r.game.decision_log.len());
        let (a, b) = (s.advance(), r.advance());
        assert_eq!(a.waiting.is_some(), b.waiting.is_some());
        assert_eq!(s.game.state_hash(), r.game.state_hash(), "seed {seed}: continuing differs");
        if !conceded {
            // undo the last human answer, then restoring that record matches too
            s.undo(1).unwrap();
            let mut r2 = Session::restore(db(), s.record()).unwrap();
            let (a, b) = (s.advance(), r2.advance());
            assert_eq!(a.waiting.is_some(), b.waiting.is_some());
            assert_eq!(s.game.state_hash(), r2.game.state_hash(), "seed {seed}: undo + restore differs");
        }
    }
}

#[test]
fn after_the_human_leaves_the_ais_play_on_in_small_steps() {
    let mut st = setup(11);
    for s in st.seats.iter_mut().skip(1) {
        s.difficulty = Difficulty::Easy;
    }
    let mut s = Session::new(db(), st).unwrap();
    let adv = s.advance();
    let d = adv.waiting.expect("the human is asked first");
    s.answer(d.player, simple(&d)).unwrap();
    s.advance();
    s.concede(0);
    let before = s.game.decision_log.len();
    let adv = s.advance();
    assert!(adv.humans_out && adv.waiting.is_none() && adv.over.is_none());
    // one AI decision (or a few that showed nothing) per call, not the rest of the game
    assert!(s.game.decision_log.len() - before < 20, "{} decisions in one step", s.game.decision_log.len() - before);
    let mut steps = 0;
    while s.advance().humans_out && steps < 50 {
        steps += 1;
    }
    assert!(s.game.decision_log.len() > before + 10);
}

#[test]
fn undo_takes_back_the_whole_action_not_only_its_last_answer() {
    // cast a spell (the choice to cast, its payment, the pass that lets it resolve), then undo once:
    // the game is back where the player had priority before casting, with nothing tapped for it
    let mut done = 0;
    for seed in [7u64, 8, 9, 10, 11, 12] {
        let mut s = Session::new(db(), setup(seed)).unwrap();
        let mut before: Option<u64> = None;
        let mut casting = false;
        for _ in 0..400 {
            let adv = s.advance();
            let Some(d) = adv.waiting else { break };
            if d.player != 0 {
                s.answer(d.player, simple(&d)).unwrap();
                continue;
            }
            match &d.kind {
                DecisionKind::Priority { actions, stack_len } => {
                    if casting {
                        if *stack_len > 0 {
                            // the spell is on the stack: pass so it resolves
                            s.answer(0, Choice::Priority { action: None }).unwrap();
                            continue;
                        }
                        break;
                    }
                    if let Some(i) = actions.iter().position(|a| matches!(a, Action::PlayLand { .. })) {
                        s.answer(0, Choice::Priority { action: Some(i) }).unwrap();
                    } else if let Some(i) = actions.iter().position(|a| matches!(a, Action::Cast { .. })).filter(|_| *stack_len == 0) {
                        before = Some(s.game.state_hash());
                        casting = true;
                        s.answer(0, Choice::Priority { action: Some(i) }).unwrap();
                    } else {
                        s.answer(0, Choice::Priority { action: None }).unwrap();
                    }
                }
                DecisionKind::PayMana { .. } => s.answer(0, Choice::Pay { answer: PayAnswer::Auto }).unwrap(),
                // the spell resolved and combat began: what follows is another action
                DecisionKind::Attackers { .. } | DecisionKind::Blockers { .. } if casting => break,
                _ => s.answer(0, simple(&d)).unwrap(),
            }
        }
        let Some(before) = before else { continue };
        if !casting {
            continue;
        }
        s.undo(1).unwrap();
        let adv = s.advance();
        let d = adv.waiting.expect("the player is asked again");
        assert!(matches!(d.kind, DecisionKind::Priority { stack_len: 0, .. }), "seed {seed}: back at an empty stack with priority");
        assert_eq!(s.game.state_hash(), before, "seed {seed}: one undo takes back the cast, its payment and the pass");
        done += 1;
    }
    assert!(done >= 2, "the test cast a spell in {done} games");
}

#[test]
fn conceding_while_asked_to_keep_or_mulligan_does_not_stall_the_ais() {
    // the human leaves during the mulligans: the game must not wait on their keep / mulligan answer
    for seed in [3u64, 11, 21] {
        let mut st = setup(seed);
        for s in st.seats.iter_mut().skip(1) {
            s.difficulty = Difficulty::Easy;
        }
        let mut s = Session::new(db(), st).unwrap();
        let adv = s.advance();
        let d = adv.waiting.expect("the human is asked");
        assert_eq!(d.player, 0);
        assert!(matches!(d.kind, DecisionKind::Mulligan { .. }), "seed {seed}: the first question to the human is the mulligan");
        s.concede(0);
        let mut steps = 0;
        while s.game.turn < 2 && s.game.over.is_none() && steps < 400 {
            let adv = s.advance();
            assert!(adv.waiting.is_none(), "seed {seed}: nobody human is left to ask");
            steps += 1;
        }
        assert!(s.game.turn >= 2 || s.game.over.is_some(), "seed {seed}: the AIs got past the first turn (turn {} after {steps} steps)", s.game.turn);
        // a save of that game restores to the same place: the restored game first plays out what the last answer caused
        // (the live one already showed it), then both stand before the same next decision
        let mut r = Session::restore(db(), s.record()).unwrap();
        assert_eq!(s.game.decision_log.len(), r.game.decision_log.len());
        let _ = r.advance();
        assert_eq!(s.game.decision_log.len(), r.game.decision_log.len(), "seed {seed}: the restored game only caught up");
        assert_eq!(s.game.state_hash(), r.game.state_hash(), "seed {seed}: restore differs");
    }
}

#[test]
fn conceding_at_any_question_lets_the_game_go_on() {
    // not only the mulligan: a player may leave while asked to discard, to block, to pay... The game goes on without them.
    let mut kinds: Vec<&'static str> = vec![];
    for seed in [5u64, 6, 14, 15, 16, 17] {
        let mut st = setup(seed);
        for s in st.seats.iter_mut().skip(1) {
            s.difficulty = Difficulty::Easy;
        }
        let mut s = Session::new(db(), st).unwrap();
        // play lands only, so the hand fills up and the cleanup step asks for a discard; blocks and other questions come up too
        let mut left_at: Option<&'static str> = None;
        for _ in 0..400 {
            let adv = s.advance();
            let Some(d) = adv.waiting else { break };
            let name = match &d.kind {
                DecisionKind::Mulligan { .. } | DecisionKind::Priority { .. } => None,
                DecisionKind::Pick { .. } => Some("Pick"),
                DecisionKind::Blockers { .. } => Some("Blockers"),
                DecisionKind::Attackers { .. } => Some("Attackers"),
                DecisionKind::PayMana { .. } => Some("PayMana"),
                DecisionKind::YesNo { .. } => Some("YesNo"),
                _ => Some("other"),
            };
            if let Some(k) = name {
                left_at = Some(k);
                break;
            }
            s.answer(d.player, simple(&d)).unwrap();
        }
        let Some(k) = left_at else { continue };
        kinds.push(k);
        let turn = s.game.turn;
        s.concede(0);
        let mut steps = 0;
        while s.game.turn < turn + 2 && s.game.over.is_none() && steps < 600 {
            let adv = s.advance();
            assert!(adv.waiting.is_none(), "seed {seed}: nobody human is left to ask");
            steps += 1;
        }
        assert!(s.game.turn >= turn + 2 || s.game.over.is_some(), "seed {seed}: left while asked {k} on turn {turn}; the game is still on turn {} after {steps} steps", s.game.turn);
    }
    assert!(!kinds.is_empty(), "the human was asked something other than priority in at least one game");
    eprintln!("left while asked: {kinds:?}");
}

#[test]
fn conceding_while_asked_which_cards_to_bottom_does_not_stall_the_ais() {
    // London mulligan: take one mulligan, keep, and leave at the "put a card on the bottom" question
    for seed in [3u64, 11] {
        let mut st = setup(seed);
        for s in st.seats.iter_mut().skip(1) {
            s.difficulty = Difficulty::Easy;
        }
        let mut s = Session::new(db(), st).unwrap();
        let mut asked = false;
        for k in 0..6 {
            let adv = s.advance();
            let d = adv.waiting.expect("the human is asked");
            match &d.kind {
                DecisionKind::Mulligan { .. } => s.answer(0, Choice::Keep { keep: k > 0 }).unwrap(),
                DecisionKind::Bottom { .. } => {
                    asked = true;
                    break;
                }
                other => panic!("seed {seed}: unexpected question before the game: {other:?}"),
            }
        }
        assert!(asked, "seed {seed}: the human was asked which card to put on the bottom");
        s.concede(0);
        let mut steps = 0;
        while s.game.turn < 2 && s.game.over.is_none() && steps < 400 {
            let adv = s.advance();
            assert!(adv.waiting.is_none(), "seed {seed}: nobody human is left to ask");
            steps += 1;
        }
        assert!(s.game.turn >= 2 || s.game.over.is_some(), "seed {seed}: the AIs got past the first turn (turn {} after {steps} steps)", s.game.turn);
        // the game log holds no answer made up for the player who left
        let mut r = Session::restore(db(), s.record()).unwrap();
        let _ = r.advance();
        assert_eq!(s.game.state_hash(), r.game.state_hash(), "seed {seed}: restore differs");
    }
}

/// Planechase and Archenemy in a session: the archenemy starts at 40 life against a team of two, schemes are set in
/// motion, planes are face up, the planar die can be rolled, and AI-only play reaches the end without breaking.
#[test]
fn planechase_and_archenemy_play_a_game() {
    for seed in [5u64, 6, 7] {
        let mut g = setup(seed);
        g.seats.truncate(3);
        for s in g.seats.iter_mut() {
            s.human = false;
        }
        g.seats[0].human = true;
        g.planar = vec![("Goldmeadow".into(), 3), ("Mutual Epiphany".into(), 1)];
        g.seats[0].schemes = vec![("Behold the Power of Destruction".into(), 3), ("The Very Soil Shall Shake".into(), 3)];
        let mut s = Session::new(db(), g).unwrap();
        assert_eq!(s.game.players[0].life, 40, "the archenemy starts at 40");
        assert_eq!(s.game.players[1].life, 20);
        assert!(s.game.is_opponent(0, 1) && !s.game.is_opponent(1, 2), "the archenemy's opponents are one team");
        assert_eq!(s.game.planes.len(), 1);
        let mut rolled = false;
        for _ in 0..600 {
            let adv = s.advance();
            let Some(d) = adv.waiting else { break };
            // the human roll the planar die whenever it is free, otherwise play simply
            let c = match &d.kind {
                DecisionKind::Priority { actions, .. } if !rolled => match actions.iter().position(|a| matches!(a, Action::RollPlanarDie)) {
                    Some(i) => {
                        rolled = true;
                        Choice::Priority { action: Some(i) }
                    }
                    None => simple(&d),
                },
                _ => simple(&d),
            };
            if s.answer(d.player, c).is_err() {
                s.answer(d.player, Choice::Default).unwrap();
            }
            if s.game.over.is_some() {
                break;
            }
        }
        assert!(rolled, "seed {seed}: the planar die was offered");
        assert!(s.game.check_invariants().is_ok(), "seed {seed}: {:?}", s.game.check_invariants());
        assert!(!s.game.planes.is_empty(), "seed {seed}: a plane is always face up");
    }
}

/// "Each player chooses" (Two Streams Facility at the first upkeep) asks the players one at a time with the picks hidden: the AI's
/// determinized copy of the game must not contain the picks of players who chose before it; in that copy they choose again
/// after it, while the real game goes on to the next player.
#[test]
fn hidden_picks_are_not_seen_by_the_ai() {
    use rand::SeedableRng;
    let mut g = setup(11);
    g.seats.truncate(3);
    for s in g.seats.iter_mut() {
        s.human = true;
    }
    g.planar = vec![("Two Streams Facility".into(), 1)];
    let mut s = Session::new(db(), g).unwrap();
    let is_stream = |d: &Decision| matches!(&d.kind, DecisionKind::Pick { prompt, .. } if prompt.starts_with("Two Streams Facility"));
    // play until the first player is asked, and answer "red waterfall"
    let mut first = None;
    for _ in 0..200 {
        let d = s.advance().waiting.expect("game over before the choice");
        if is_stream(&d) {
            first = Some(d.player);
            s.answer(d.player, Choice::Pick { indices: vec![1] }).unwrap();
            break;
        }
        let c = simple(&d);
        if s.answer(d.player, c).is_err() {
            s.answer(d.player, Choice::Default).unwrap();
        }
    }
    let first = first.expect("Two Streams Facility asked nobody");
    let d = s.advance().waiting.expect("a second player is asked");
    assert!(is_stream(&d), "the second player is asked next: {:?}", d.kind);
    let second = d.player;
    assert_ne!(second, first);
    // the real game: after the second answer, the third player is asked (not the first again)
    let mut real = s.game.clone();
    real.apply(Choice::Pick { indices: vec![0] }).unwrap();
    let next = real.next_decision().expect("the third player is asked");
    assert!(is_stream(&next) && next.player != first && next.player != second, "real game asks the third player: {:?} {}", next.kind, next.player);
    // the second player's AI view: the first player's pick is forgotten, so after the second and third answer the first is asked again
    let mut rng = rand::rngs::StdRng::seed_from_u64(1);
    let mut det = mtg_ai::determinize::determinize(&s.game, second, &mut rng);
    det.apply(Choice::Pick { indices: vec![0] }).unwrap();
    let mut asked = vec![];
    for _ in 0..3 {
        let Some(n) = det.next_decision() else { break };
        if !is_stream(&n) {
            break;
        }
        asked.push(n.player);
        det.apply(Choice::Pick { indices: vec![0] }).unwrap();
    }
    assert!(asked.contains(&first), "in the AI's copy the first player has to choose again: asked {asked:?}");
    assert!(!asked.contains(&second), "the AI's own pick is kept: asked {asked:?}");
}

#[test]
fn undo_after_casting_then_attacking_takes_back_only_the_attack() {
    // nobody else answers anything between the cast and the attack (the AI's passes aren't questions), yet the attack
    // declaration is an action of its own: one undo goes back to the attackers question, not to before the cast
    let mut done = 0;
    for seed in [7u64, 8, 9, 10, 11, 12] {
        let mut s = Session::new(db(), duel(seed)).unwrap();
        let mut cast = false;
        let mut at_attack: Option<u64> = None;
        for _ in 0..400 {
            let adv = s.advance();
            let Some(d) = adv.waiting else { break };
            if d.player != 0 {
                s.answer(d.player, simple(&d)).unwrap();
                continue;
            }
            match &d.kind {
                DecisionKind::Priority { actions, stack_len } => {
                    if let Some(i) = actions.iter().position(|a| matches!(a, Action::PlayLand { .. })) {
                        s.answer(0, Choice::Priority { action: Some(i) }).unwrap();
                    } else if let Some(i) = actions.iter().position(|a| matches!(a, Action::Cast { .. })).filter(|_| *stack_len == 0 && !cast) {
                        cast = true;
                        s.answer(0, Choice::Priority { action: Some(i) }).unwrap();
                    } else {
                        s.answer(0, Choice::Priority { action: None }).unwrap();
                    }
                }
                DecisionKind::Attackers { options, .. } if cast && !options.is_empty() => {
                    at_attack = Some(s.game.state_hash());
                    let o = &options[0];
                    s.answer(0, Choice::Attackers { attacks: vec![(o.card, Obj::player(o.players[0]))] }).unwrap();
                    break;
                }
                DecisionKind::PayMana { .. } => s.answer(0, Choice::Pay { answer: PayAnswer::Auto }).unwrap(),
                _ => s.answer(0, simple(&d)).unwrap(),
            }
        }
        let Some(h) = at_attack else { continue };
        s.undo(1).unwrap();
        let adv = s.advance();
        assert!(matches!(adv.waiting.map(|d| d.kind), Some(DecisionKind::Attackers { .. })), "seed {seed}: back at the attackers question");
        assert_eq!(s.game.state_hash(), h, "seed {seed}");
        done += 1;
    }
    assert!(done >= 2, "attacked after casting in {done} games");
}

/// "Skip until my next turn" stops at the start of that turn, also when the request reaches the engine after the turn
/// it was clicked in (the AI was still playing): the next turn is counted from the turn the player saw, and when that turn
/// has already begun the priority waiting in it is kept.
#[test]
fn skip_until_my_turn_stops_when_it_starts() {
    for seed in [3u64, 11, 21] {
        for late in [false, true] {
            let mut s = Session::new(db(), duel(seed)).unwrap();
            let mut armed: Option<u32> = None;
            let mut stopped = None;
            for _ in 0..400 {
                let adv = s.advance();
                let Some(d) = adv.waiting else { break };
                if d.player == 0 && matches!(d.kind, DecisionKind::Priority { .. }) {
                    if armed.is_some() {
                        stopped = Some((s.game.turn, s.game.step, s.game.active));
                        break;
                    }
                    if late && s.game.active == 0 && s.game.turn >= 3 {
                        // clicked during the opponent's turn, arriving in our own: nothing is armed, and the priority
                        // waiting now (not a later one) is where it stops
                        armed = Some(s.game.turn);
                        let at = (s.game.turn, s.game.step);
                        s.pass_until_player(0, 0, Some(s.game.turn - 1));
                        assert!(s.passing(0).is_none(), "seed {seed}: a reached target is not armed");
                        // an "End turn" clicked on the turn before is already reached too
                        s.pass_until(0, s.game.turn, mtg_engine::types::Step::Untap);
                        assert!(s.passing(0).is_none(), "seed {seed}: end turn already reached");
                        stopped = Some((at.0, at.1, s.game.active));
                        break;
                    }
                    if !late && s.game.active == 1 && s.game.turn >= 2 {
                        armed = Some(s.game.turn);
                        s.pass_until_player(0, 0, Some(s.game.turn));
                        s.answer(0, Choice::Priority { action: None }).unwrap();
                        continue;
                    }
                }
                s.answer(d.player, simple(&d)).unwrap();
            }
            let t = armed.expect("armed");
            let (turn, step, active) = stopped.expect("stopped");
            assert_eq!(active, 0, "seed {seed} late {late}");
            assert!(s.passing(0).is_none(), "seed {seed} late {late}: still passing");
            if late {
                assert_eq!(turn, t, "seed {seed}: the turn that had already begun");
            } else {
                assert_eq!((turn, step), (t + 1, mtg_engine::types::Step::Upkeep), "seed {seed}");
            }
        }
    }
}

/// Breaking the rules, a player may look at an AI's hand and library (in order) until the break ends; nobody else sees
/// them, a human's cards stay hidden, and undo / restore replay the reveal.
#[test]
fn rule_break_shows_an_ai_hand_and_library_to_that_player_only() {
    let mut s = Session::new(db(), duel(5)).unwrap();
    s.advance();
    let opp_hand = |s: &Session, viewer: u8| s.view(viewer).cards.iter().filter(|c| c.owner == 1 && c.zone == types::Zone::Hand).count();
    assert_eq!(opp_hand(&s, 0), 0, "the AI's hand is hidden");
    s.edit(0, EditOp::Show { player: 1, zone: types::Zone::Hand }).unwrap();
    s.edit(0, EditOp::Show { player: 1, zone: types::Zone::Library }).unwrap();
    assert_eq!(opp_hand(&s, 0), s.game.players[1].hand.len());
    let v = s.view(0);
    let lib: Vec<CardId> = s.game.players[1].library.iter().rev().copied().collect();
    assert_eq!(v.players[1].library_shown, lib, "the library in order, top first");
    assert!(s.view(1).players[1].library_shown.is_empty(), "shown to the player who asked only");
    assert!(s.edit(1, EditOp::Show { player: 0, zone: types::Zone::Hand }).is_err(), "a human's hand stays theirs");
    // a restored game replays the edit
    let r = Session::restore(db(), s.record()).unwrap();
    assert_eq!(opp_hand(&r, 0), s.game.players[1].hand.len());
    s.end_break(0);
    assert_eq!(opp_hand(&s, 0), 0, "hidden again once the rule break ends");
    assert!(s.view(0).players[1].library_shown.is_empty());
}
