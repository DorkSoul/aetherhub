//! Convenience for tests, puzzles and scripted agents: set up boards and drive decisions by name.

use crate::decision::*;
use crate::ids::*;
use crate::state::*;
use crate::types::*;
use crate::zones::MoveOpts;

impl Game {
    /// A game with empty libraries, no mulligans and `players` seats, ready for `add_card` and `start_at`.
    pub fn new_blank(db: std::sync::Arc<crate::CardDb>, players: usize, seed: u64) -> Game {
        let decks = (0..players).map(|i| DeckSpec { name: format!("P{i}"), main: vec![], commanders: vec![], sideboard: vec![], companion: None }).collect();
        let mut cfg = Config::standard();
        cfg.mulligan = MulliganRule::None;
        let mut g = Game::new(db, cfg, decks, seed);
        g.first_player = 0;
        for p in g.players.iter_mut() {
            p.kept = true;
            p.auto_skip = false;
            p.full_control = true;
        }
        g
    }

    /// Put a named card straight into a zone (puzzle / test set-up). Panics on unknown names.
    pub fn add_card(&mut self, name: &str, owner: PlayerId, zone: Zone) -> CardId {
        let def = self.db.find(name).unwrap_or_else(|| panic!("unknown card {name}"));
        let token = self.db.def(def).token;
        let id = self.create_card(def, owner, Zone::Library, token);
        self.players[owner as usize].library.retain(|x| *x != id);
        if zone == Zone::Battlefield {
            self.cards[id.0 as usize].zone = Zone::Library;
            self.move_card(id, Zone::Battlefield, MoveOpts { controller: Some(owner), ..Default::default() });
            // permanents added by set-up are not summoning sick and are not tapped (they keep their natural enters-with counters)
            let c = &mut self.cards[id.0 as usize];
            c.since = 0;
            c.control_turn = 0;
            c.tapped = false;
        } else {
            self.cards[id.0 as usize].zone = Zone::Library;
            self.move_card(id, zone, Default::default());
        }
        self.events.clear();
        id
    }

    /// Begin play at `step` of a fresh turn for `player` (skipping the pre-game and earlier steps).
    pub fn start_at(&mut self, player: PlayerId, step: Step) {
        self.turn = 1;
        self.active = player;
        self.step = step;
        self.first_player = player;
        self.players[player as usize].last_turn_start = 1;
        self.players[player as usize].cur_turn = 1;
        self.players[player as usize].hand_turn_start = self.players[player as usize].hand.len() as i32;
        {
            let n = self.battlefield.iter().filter(|id| { let c = &self.cards[id.0 as usize]; c.controller == player && !c.tapped && c.cur.types.has(CardType::Land) }).count() as i32;
            self.players[player as usize].lands_untapped_turn_start = n;
        }
        self.players[player as usize].turns_taken = 1;
        for p in self.players.iter_mut() {
            p.kept = true;
        }
        self.begin_with_opening_hand_permanents(true);
        self.emit(Event::GameStart);
        self.run = Run::Priority { player, passes: 0 };
        self.layers_dirty = true;
        self.recompute_layers();
    }

    /// Put a named card on the stack as if cast for free by `player` (no cost, no targets chosen).
    pub fn priority_player(&self) -> Option<PlayerId> {
        match (&self.pending, &self.run) {
            (Some(Decision { player, kind: DecisionKind::Priority { .. } }), _) => Some(*player),
            _ => None,
        }
    }

    /// Run until a decision is pending; returns it.
    pub fn next_decision(&mut self) -> Option<Decision> {
        match self.run() {
            Status::NeedDecision => self.pending.clone(),
            Status::Over => None,
        }
    }

    fn priority_actions(&mut self, player: PlayerId) -> Vec<Action> {
        let d = self.next_decision().expect("game ended while waiting for priority");
        match d.kind {
            DecisionKind::Priority { actions, .. } if d.player == player => actions,
            other => panic!("expected priority for player {player}, got {:?} for player {}", other, d.player),
        }
    }

    /// Pass priority for whoever holds it, once.
    pub fn pass(&mut self) {
        let d = self.next_decision().expect("game over");
        assert!(matches!(d.kind, DecisionKind::Priority { .. }), "expected a priority decision, got {:?}", d.kind);
        self.apply(Choice::Priority { action: None }).unwrap();
    }

    /// Pass priority until the stack is empty again (everything resolves), stopping at any non-priority decision.
    pub fn resolve_stack(&mut self) {
        let mut guard = 0;
        loop {
            self.run();
            if self.stack.is_empty() && matches!(self.run, Run::Priority { .. }) && self.pending.as_ref().map_or(true, |d| matches!(d.kind, DecisionKind::Priority { .. })) {
                break;
            }
            match self.pending.as_ref().map(|d| matches!(d.kind, DecisionKind::Priority { .. })) {
                Some(true) => {
                    self.apply(Choice::Priority { action: None }).unwrap();
                }
                _ => break,
            }
            guard += 1;
            assert!(guard < 10_000, "stack never empties");
        }
    }

    /// Perform the first action satisfying `pred` for `player`.
    pub fn act(&mut self, player: PlayerId, pred: impl Fn(&Game, &Action) -> bool) -> bool {
        let actions = self.priority_actions(player);
        if let Some(i) = actions.iter().position(|a| pred(self, a)) {
            self.apply(Choice::Priority { action: Some(i) }).unwrap();
            true
        } else {
            false
        }
    }

    pub fn card_name(&self, id: CardId) -> std::borrow::Cow<'_, str> {
        match self.cards[id.0 as usize].cur.name {
            Some(n) if self.cards[id.0 as usize].zone == Zone::Battlefield => std::borrow::Cow::Owned(Sym::name(n)),
            _ => std::borrow::Cow::Borrowed(&self.def_of(id).name),
        }
    }

    /// Cast a named card from hand; answer target prompts with `targets` (by object); pay automatically.
    pub fn cast(&mut self, player: PlayerId, name: &str, targets: &[Obj]) {
        let ok = self.act(player, |g, a| matches!(a, Action::Cast { card, .. } if g.card_name(*card) == name));
        assert!(ok, "{name} is not castable by player {player}");
        self.answer_targets(targets);
    }

    /// Answer pending Pick decisions that are target choices using the given objects in order.
    pub fn answer_targets(&mut self, targets: &[Obj]) {
        let mut remaining: Vec<Obj> = targets.to_vec();
        loop {
            let Some(d) = self.next_decision() else { return };
            match d.kind {
                DecisionKind::Pick { options, min, .. } => {
                    let mut indices = vec![];
                    for _ in 0..(min as usize).max(1) {
                        if remaining.is_empty() {
                            break;
                        }
                        let want = remaining.remove(0);
                        if let Some(i) = options.iter().position(|o| matches!(o, Opt::Obj { obj } if *obj == want)) {
                            indices.push(i);
                        }
                    }
                    self.apply(Choice::Pick { indices }).unwrap();
                }
                DecisionKind::PayMana { .. } => {
                    self.apply(Choice::Pay { answer: PayAnswer::Auto }).unwrap();
                }
                _ => return,
            }
        }
    }

    pub fn play_land(&mut self, player: PlayerId, name: &str) {
        let ok = self.act(player, |g, a| matches!(a, Action::PlayLand { card } if g.card_name(*card) == name));
        assert!(ok, "can't play {name}");
    }

    pub fn activate(&mut self, player: PlayerId, name: &str, ability: u16, targets: &[Obj]) {
        let ok = self.act(player, |g, a| matches!(a, Action::Activate { card, ability: i } if g.card_name(*card) == name && *i == ability));
        assert!(ok, "can't activate {name} #{ability}");
        self.answer_targets(targets);
    }

    pub fn find(&self, name: &str, zone: Zone) -> Option<CardId> {
        self.cards.iter().find(|c| c.zone == zone && self.db.def(c.def).name == name).map(|c| c.id)
    }

    pub fn find_all(&self, name: &str, zone: Zone) -> Vec<CardId> {
        self.cards.iter().filter(|c| c.zone == zone && self.db.def(c.def).name == name).map(|c| c.id).collect()
    }

    pub fn count_in(&self, name: &str, zone: Zone) -> usize {
        self.find_all(name, zone).len()
    }

    /// Advance to the given step of the current turn, passing priority (declining everything) on the way.
    pub fn pass_to(&mut self, step: Step) {
        let mut guard = 0;
        while !(self.step == step && matches!(self.run, Run::Priority { .. }) && self.pending.is_some()) {
            let Some(d) = self.next_decision() else { return };
            match d.kind {
                DecisionKind::Priority { .. } => {
                    if self.step == step && self.stack.is_empty() {
                        return;
                    }
                    self.apply(Choice::Priority { action: None }).unwrap();
                }
                _ => {
                    let c = self.default_choice(&d);
                    self.apply(c).unwrap();
                }
            }
            guard += 1;
            assert!(guard < 100_000, "never reached {step:?}");
        }
    }
}

impl Game {
    /// Pass priority (declining everything) until the attackers decision comes up; returns it.
    pub fn to_attackers(&mut self) -> Decision {
        let mut guard = 0;
        loop {
            let d = self.next_decision().expect("game over before combat");
            match d.kind {
                DecisionKind::Attackers { .. } => return d,
                DecisionKind::Priority { .. } => {
                    self.apply(Choice::Priority { action: None }).unwrap();
                }
                _ => {
                    let c = self.default_choice(&d);
                    self.apply(c).unwrap();
                }
            }
            guard += 1;
            assert!(guard < 10_000, "no attack step reached");
        }
    }

    /// Declare attackers (each attacking the given defender).
    pub fn attack(&mut self, attacks: &[(CardId, Obj)]) {
        self.to_attackers();
        self.apply(Choice::Attackers { attacks: attacks.to_vec() }).unwrap();
    }

    /// Pass priority until the blockers decision is up (declare attackers priority passes); returns it.
    pub fn to_blockers(&mut self) -> Decision {
        let mut guard = 0;
        loop {
            let d = self.next_decision().expect("game over before blocks");
            match d.kind {
                DecisionKind::Blockers { .. } => return d,
                DecisionKind::Priority { .. } => {
                    self.apply(Choice::Priority { action: None }).unwrap();
                }
                _ => {
                    let c = self.default_choice(&d);
                    self.apply(c).unwrap();
                }
            }
            guard += 1;
            assert!(guard < 10_000, "no block step reached");
        }
    }

    pub fn block(&mut self, blocks: &[(CardId, CardId)]) {
        self.to_blockers();
        self.apply(Choice::Blockers { blocks: blocks.to_vec() }).unwrap();
    }

    /// Keep passing priority until the given step has finished (the next step's priority is up).
    pub fn finish_combat(&mut self) {
        self.pass_to(Step::Main2);
    }
}
