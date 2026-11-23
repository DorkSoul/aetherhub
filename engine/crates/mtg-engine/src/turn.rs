//! The run loop: turn structure, priority, resolving the stack, cleanup, and answering decisions.

use crate::cast::Flow;
use crate::decision::*;
use crate::dsl::*;
use crate::ids::*;
use crate::sba::SbaResult;
use crate::state::*;
use crate::types::*;
use rand::seq::SliceRandom;

impl Game {
    /// Advance the game until a decision is needed (see `decision()`) or the game is over.
    pub fn run(&mut self) -> Status {
        loop {
            if self.over.is_some() {
                return Status::Over;
            }
            if self.pending.is_some() {
                self.remap_decision();
                return Status::NeedDecision;
            }
            self.steps_taken += 1;
            if let Some(limit) = self.abort_at {
                if self.steps_taken % 16 == 0 && std::time::Instant::now() > limit {
                    return Status::Over;
                }
            }
            if self.steps_taken > self.max_steps {
                self.over = Some(GameResult::Draw);
                self.run = Run::Over;
                self.emit(Event::Over { result: GameResult::Draw });
                return Status::Over;
            }
            self.copy_resolving.set(false);
            let run = std::mem::replace(&mut self.run, Run::Over);
            let flow = match run {
                Run::PreGame { stage } => self.step_pregame(stage),
                Run::BeginStep => self.step_begin(),
                Run::Priority { player, passes } => self.step_priority(player, passes),
                Run::Cast(ctx) => self.step_cast(ctx),
                Run::Resolve(ctx) => self.step_resolve(ctx),
                Run::Attackers => self.step_attackers(),
                Run::Blockers { idx } => self.step_blockers(idx),
                Run::DamageAssign(ctx) => self.step_damage(ctx),
                Run::Cleanup => self.step_cleanup(),
                Run::EnterChoice { card, kind, options, player, passes } => self.step_enter_choice(card, kind, options, player, passes),
                Run::EnterPay { card, life, player, passes } => self.step_enter_pay(card, life, player, passes),
                Run::EnterMayTap { card, player, passes } => self.step_enter_may_tap(card, player, passes),
                Run::OptZone { card, instead, bottom, player, passes } => self.step_opt_zone(card, instead, bottom, player, passes),
                Run::ZoneChoice { card, cands, player, passes } => self.step_zone_choice(card, cands, player, passes),
                Run::EnterExileAny { card, options, player, passes } => self.step_enter_exile_any(card, options, player, passes),
                Run::EnterCounter { card, options, player, passes } => self.step_enter_counter(card, options, player, passes),
                Run::EnterUnless { card, player, passes } => self.step_enter_unless(card, player, passes),
                Run::EnterExile { card, options, left, player, passes } => self.step_enter_exile(card, options, left, player, passes),
                Run::EnterReveal { card, player, passes } => self.step_enter_reveal(card, player, passes),
                Run::UntapChoose { queue, asked } => self.step_untap_choose(queue, asked),
                Run::DrawSkipAsk { gain } => self.step_draw_skip_ask(gain),
                Run::UntapPickType { options } => self.step_untap_pick_type(options),
                Run::UntapLimitPick { ids } => self.step_untap_limit_pick(ids),
                Run::AttachPlayerAsk { card, options, resume } => self.step_attach_player_ask(card, options, resume),
                Run::ManaGiftAsk { card, ability, produce, options, player, passes } => self.step_mana_gift_ask(card, ability, produce, options, player, passes),
                Run::ManaPickAsk { card, ability, produce, options, player, passes } => self.step_mana_pick_ask(card, ability, produce, options, player, passes),
                Run::EnterCopy { card, options, player, passes } => self.step_enter_copy(card, options, player, passes),
                Run::EnterAttach { card, options, player, passes } => self.step_enter_attach(card, options, player, passes),
                Run::Over => return Status::Over,
            };
            if matches!(self.run, Run::Over) && self.over.is_none() {
                panic!("a handler left the game without a run state (it forgot to restore `self.run`)");
            }
            if self.restart_pending.is_some() && self.over.is_none() {
                self.do_restart();
                continue;
            }
            if let Flow::Wait = flow {
                if self.pending.is_none() {
                    // a handler waited without asking: treat as a bug in debug builds
                    debug_assert!(false, "handler waited without a pending decision");
                    return Status::NeedDecision;
                }
                self.remap_decision();
                return Status::NeedDecision;
            }
        }
    }

    /// During a turn controlled by another player (Mindslaver) the controlled player's decisions are handed to the controller.
    fn remap_decision(&mut self) {
        let Some((v, c)) = self.resolve_ctl.or(self.turn_ctl) else { return };
        if self.resolve_ctl.is_none() && self.turn_ctl_combat && !matches!(self.step, Step::BeginCombat | Step::DeclareAttackers | Step::DeclareBlockers | Step::FirstStrikeDamage | Step::CombatDamage | Step::EndCombat) {
            return;
        }
        let n = self.decision_log.len();
        if self.remapped_at.map_or(false, |(k, _)| k == n) {
            return;
        }
        if let Some(d) = self.pending.as_mut() {
            if d.player == v && v != c {
                d.player = c;
                self.remapped_at = Some((n, v));
            }
        }
    }

    /// The player whose resources the pending decision is about (the controlled player during a controlled turn).
    fn decision_subject(&self, d: &Decision) -> PlayerId {
        match self.remapped_at {
            Some((k, v)) if k == self.decision_log.len() => v,
            _ => d.player,
        }
    }

    /// Answer the pending decision. Call `run()` afterwards.
    pub fn apply(&mut self, choice: Choice) -> Result<(), IllegalChoice> {
        let Some(d) = self.pending.as_ref() else {
            return Err(IllegalChoice("no decision is pending".into()));
        };
        // CR 601.2g / 602.2b: while choosing what to give up for a cost (a land to sacrifice...) the player may activate
        // mana abilities, e.g. tap that land for mana first. The mana floats; the same question stays open.
        if let (DecisionKind::Pick { purpose: PickPurpose::Cost, .. }, Choice::Mana { card, ability, produce, x, picks }) = (&d.kind, &choice) {
            let p = self.decision_subject(d);
            let (card, ability, produce) = (*card, *ability, *produce);
            if !self.cost_time_mana_ok(p, card, ability) {
                return Err(IllegalChoice("that can't be tapped for mana right now".into()));
            }
            self.mana_x_hint.set(*x);
            *self.mana_color_hints.borrow_mut() = picks.clone();
            self.mana_produce_hint.set(produce);
            self.activate_mana_ability(p, card, ability, produce).map_err(IllegalChoice)?;
            // the open question keeps its place in the log (a controlled turn's hand-over is keyed to it)
            let n = self.decision_log.len();
            self.decision_log.push(choice);
            if let Some((k, v)) = self.remapped_at {
                if k == n {
                    self.remapped_at = Some((n + 1, v));
                }
            }
            return Ok(());
        }
        let choice = self.validate(d, choice)?;
        self.decision_log.push(choice.clone());
        self.pending = None;
        self.answer = Some(choice);
        Ok(())
    }

    /// A mana ability that may be activated in the middle of paying a cost: one of `p`'s mana sources whose only costs are
    /// tapping and mana (so the permanent stays where it is), with no extra question of its own (who gets the mana, which cards to exile).
    fn cost_time_mana_ok(&self, p: PlayerId, card: CardId, ability: u16) -> bool {
        if !self.mana_sources(p).iter().any(|s| s.card == card && s.ability == ability) {
            return false;
        }
        if (crate::cast::LIFE_MANA_BASE..crate::cast::INTRINSIC_BASE).contains(&ability) {
            return false;
        }
        if self.mana_gift_ability(p, card, ability) || self.mana_pick_options(p, card, ability).is_some() {
            return false;
        }
        ability >= crate::cast::INTRINSIC_BASE || matches!(self.ability_owned(card, ability), Some(Ability::Activated { ref cost, .. }) if cost.iter().all(|c| matches!(c, Cost::Tap | Cost::Mana { .. })))
    }

    /// Is this choice legal for the given decision? (No state is changed.)
    pub fn check_choice(&self, d: &Decision, choice: &Choice) -> bool {
        self.validate(d, choice.clone()).is_ok()
    }

    fn card_in_range(&self, c: CardId) -> bool {
        (c.0 as usize) < self.cards.len()
    }

    /// The most entries an attack or block declaration may list (a bound on the work an answer can cause).
    fn combat_answer_cap(&self) -> usize {
        self.cards.len() * 2 + 64
    }

    /// Check a choice against the pending decision; `Default` is replaced by the safest legal answer.
    fn validate(&self, d: &Decision, choice: Choice) -> Result<Choice, IllegalChoice> {
        let bad = |m: &str| Err(IllegalChoice(m.to_string()));
        let choice = if matches!(choice, Choice::Default) { self.default_choice(d) } else { choice };
        match (&d.kind, &choice) {
            (DecisionKind::Mulligan { .. }, Choice::Keep { .. }) => {}
            (DecisionKind::Mulligan { peek, .. }, Choice::Pick { indices }) => {
                let mut seen = indices.clone();
                seen.sort();
                seen.dedup();
                if peek.is_empty() || seen.len() != indices.len() || indices.iter().any(|i| *i >= peek.len()) {
                    return bad("invalid scry picks");
                }
            }
            (DecisionKind::Mulligan { peek, .. }, Choice::Order { order }) => {
                let mut o = order.clone();
                o.sort();
                if peek.is_empty() || o != (0..peek.len()).collect::<Vec<_>>() {
                    return bad("not a permutation of the scried cards");
                }
            }
            (DecisionKind::Bottom { hand, n }, Choice::Bottom { cards }) => {
                if cards.len() != *n as usize || cards.iter().any(|c| !hand.contains(c)) {
                    return bad("wrong cards to bottom");
                }
            }
            (DecisionKind::Priority { actions, .. }, Choice::Priority { action }) => {
                if let Some(i) = action {
                    if *i >= actions.len() {
                        return bad("no such action");
                    }
                }
            }
            (DecisionKind::Priority { .. }, Choice::Mana { card, ability, .. }) => {
                if !self.mana_sources(self.decision_subject(d)).iter().any(|s| s.card == *card && s.ability == *ability) {
                    return bad("that can't be tapped for mana right now");
                }
            }
            (DecisionKind::Pick { options, min, max, cancellable, purpose, .. }, Choice::Pick { indices }) => {
                let mut seen = indices.clone();
                seen.sort();
                seen.dedup();
                let repeats_ok = matches!(purpose, PickPurpose::ModeRepeat);
                if (!repeats_ok && seen.len() != indices.len()) || indices.iter().any(|i| *i >= options.len()) {
                    return bad("invalid picks");
                }
                if indices.len() < *min as usize || indices.len() > *max as usize {
                    let _ = cancellable;
                    return bad("wrong number of picks");
                }
            }
            (DecisionKind::Pick { cancellable: true, .. }, Choice::Cancel) => {}
            (DecisionKind::Number { min, max, .. }, Choice::Number { n }) => {
                if n < min || n > max {
                    return bad("number out of range");
                }
            }
            (DecisionKind::Number { cancellable: true, .. }, Choice::Cancel) => {}
            (DecisionKind::YesNo { .. }, Choice::YesNo { .. }) => {}
            // a source tapped by hand must be one the payment question offers
            (DecisionKind::PayMana { sources, .. }, Choice::Pay { answer: PayAnswer::Tap { card, ability, .. } }) => {
                if !sources.iter().any(|s| s.card == *card && s.ability == *ability) {
                    return bad("that can't be tapped for this payment");
                }
            }
            (DecisionKind::PayMana { .. }, Choice::Pay { .. }) | (DecisionKind::PayMana { .. }, Choice::Cancel) => {}
            // Attacks and blocks that break a restriction are not refused here: the combat step drops them and tells the
            // player why (scenarios and the table rely on that). Only ids that name nothing are refused.
            (DecisionKind::Attackers { .. }, Choice::Attackers { attacks }) => {
                if attacks.len() > self.combat_answer_cap() {
                    return bad("too many attackers");
                }
                // (an attack on a player who isn't in the game is harmless: it is dropped like any other illegal attack)
                if attacks.iter().any(|(c, def)| !self.card_in_range(*c) || matches!(def, Obj::Card { id } if !self.card_in_range(*id)) || matches!(def, Obj::Stack { .. })) {
                    return bad("no such attacker or defender");
                }
            }
            (DecisionKind::Blockers { .. }, Choice::Blockers { blocks }) => {
                if blocks.len() > self.combat_answer_cap() {
                    return bad("too many blocks");
                }
                if blocks.iter().any(|(b, a)| !self.card_in_range(*b) || !self.card_in_range(*a)) {
                    return bad("no such blocker or attacker");
                }
            }
            (DecisionKind::AssignDamage { total, blockers, defender, .. }, Choice::Damage { assign }) => {
                // summed wide so huge numbers can't wrap around to the right total
                if assign.iter().map(|(_, n)| *n as i64).sum::<i64>() != *total as i64 || assign.iter().any(|(_, n)| *n < 0) {
                    return bad("damage must add up to the attacker's power");
                }
                let recipient = |o: &Obj| match o {
                    Obj::Card { id } => blockers.iter().any(|(b, _)| b == id) || defender.as_ref() == Some(o),
                    _ => defender.as_ref() == Some(o),
                };
                if assign.iter().any(|(o, n)| *n > 0 && !recipient(o)) {
                    return bad("damage can only be assigned to the blockers or the defender");
                }
            }
            (DecisionKind::Order { items, .. }, Choice::Order { order }) => {
                let mut o = order.clone();
                o.sort();
                if o != (0..items.len()).collect::<Vec<_>>() {
                    return bad("not a permutation");
                }
            }
            _ => return bad("answer doesn't match the question"),
        }
        Ok(choice)
    }

    /// A safe answer for any decision (used by `Choice::Default` and the passive agent).
    pub fn default_choice(&self, d: &Decision) -> Choice {
        match &d.kind {
            DecisionKind::Mulligan { .. } => Choice::Keep { keep: true },
            DecisionKind::Bottom { hand, n } => Choice::Bottom { cards: hand.iter().rev().take(*n as usize).copied().collect() },
            DecisionKind::Priority { .. } => Choice::Priority { action: None },
            DecisionKind::Pick { purpose: PickPurpose::Proliferate, options, .. } => Choice::Pick { indices: (0..options.len()).collect() },
            DecisionKind::Pick { purpose: PickPurpose::ModeRepeat, min, options, .. } if !options.is_empty() => Choice::Pick { indices: (0..*min as usize).map(|i| i % options.len()).collect() },
            DecisionKind::Pick { min, options, .. } => Choice::Pick { indices: (0..(*min as usize).min(options.len())).collect() },
            DecisionKind::Number { min, .. } => Choice::Number { n: *min },
            DecisionKind::YesNo { .. } => Choice::YesNo { yes: false },
            DecisionKind::PayMana { .. } => Choice::Pay { answer: PayAnswer::Auto },
            DecisionKind::Attackers { must_attack, options } => Choice::Attackers {
                attacks: must_attack
                    .iter()
                    .filter_map(|c| options.iter().find(|o| o.card == *c).and_then(|o| o.players.first().map(|p| (*c, Obj::player(*p)))))
                    .collect(),
            },
            DecisionKind::Blockers { .. } => Choice::Blockers { blocks: vec![] },
            DecisionKind::AssignDamage { attacker: _, total, blockers, defender, trample } => {
                let mut left = *total;
                let mut assign = vec![];
                for (b, lethal) in blockers {
                    let n = if *trample { left.min(*lethal) } else { left };
                    if n > 0 {
                        assign.push((Obj::card(*b), n));
                        left -= n;
                    }
                    if left == 0 {
                        break;
                    }
                }
                if left > 0 {
                    match (defender, assign.last_mut()) {
                        (Some(def), _) if *trample => assign.push((*def, left)),
                        (_, Some(last)) => last.1 += left,
                        _ => {}
                    }
                }
                Choice::Damage { assign }
            }
            DecisionKind::Order { items, .. } => Choice::Order { order: (0..items.len()).collect() },
        }
    }

    // ------------------------------------------------------------------ before the game

    pub(crate) fn step_pregame(&mut self, stage: PreGame) -> Flow {
        // A player who has left the game (conceded before it began) is asked nothing: their part of every stage is skipped.
        // Without this the game would wait for ever on their mulligan, their cards to bottom, their scry...
        if let Some(next) = self.pregame_after_departed(&stage) {
            self.answer = None;
            self.run = Run::PreGame { stage: next };
            return Flow::Continue;
        }
        match stage {
            PreGame::Color { idx } => {
                if idx >= self.players.len() {
                    self.run = Run::PreGame { stage: PreGame::Mulligan { idx: 0 } };
                    return Flow::Continue;
                }
                let p = idx as PlayerId;
                let wants = self.players[idx].pregame_color.is_none()
                    && self.players[idx].command.iter().any(|&id| self.cards[id.0 as usize].is_commander && self.db.def(self.cards[id.0 as usize].def).abilities.iter().any(|a| matches!(a, Ability::PregameChooseColor)));
                if !wants {
                    self.run = Run::PreGame { stage: PreGame::Color { idx: idx + 1 } };
                    return Flow::Continue;
                }
                if let Some(Choice::Pick { indices }) = self.answer.take() {
                    self.players[idx].pregame_color = Color::ALL.get(indices.first().copied().unwrap_or(0)).copied();
                    self.layers_dirty = true;
                    self.run = Run::PreGame { stage: PreGame::Color { idx: idx + 1 } };
                    return Flow::Continue;
                }
                self.pending = Some(Decision {
                    player: p,
                    kind: DecisionKind::Pick { prompt: "Choose a color before the game begins".into(), source: None, options: Color::ALL.iter().map(|c| Opt::Color { c: *c }).collect(), min: 1, max: 1, cancellable: false, hidden: false, purpose: PickPurpose::Generic },
                });
                self.run = Run::PreGame { stage: PreGame::Color { idx } };
                Flow::Wait
            }
            PreGame::Mulligan { idx } => {
                let n = self.players.len();
                if idx >= n {
                    // everyone kept
                    self.run = Run::PreGame { stage: PreGame::Begin { idx: 0 } };
                    return Flow::Continue;
                }
                // order: starting from the first player
                let p = ((self.first_player as usize + idx) % n) as PlayerId;
                if self.cfg.mulligan == MulliganRule::None || self.players[p as usize].kept {
                    self.players[p as usize].kept = true;
                    self.run = Run::PreGame { stage: PreGame::Mulligan { idx: idx + 1 } };
                    return Flow::Continue;
                }
                let peek_n = if self.cfg.mulligan == MulliganRule::FreeScry3 { 3usize.saturating_sub(self.players[p as usize].scry_bottomed as usize) } else { 0 };
                let peek: Vec<CardId> = self.players[p as usize].library.iter().rev().take(peek_n).copied().collect();
                match self.answer.take() {
                    // free mulligan + scry 3, while deciding: some of the scried cards to the bottom, or the rest reordered
                    Some(Choice::Pick { indices }) => {
                        let lib = &mut self.players[p as usize].library;
                        for &i in &indices {
                            if let Some(&id) = peek.get(i) {
                                lib.retain(|x| *x != id);
                                lib.insert(0, id);
                            }
                        }
                        self.players[p as usize].scry_bottomed += indices.len() as u8;
                        self.run = Run::PreGame { stage: PreGame::Mulligan { idx } };
                        return Flow::Continue;
                    }
                    Some(Choice::Order { order }) => {
                        let lib = &mut self.players[p as usize].library;
                        let base = lib.len() - peek.len();
                        // `order[0]` becomes the top card, the last element of the library
                        for (k, &i) in order.iter().enumerate() {
                            lib[base + peek.len() - 1 - k] = peek[i];
                        }
                        self.run = Run::PreGame { stage: PreGame::Mulligan { idx } };
                        return Flow::Continue;
                    }
                    Some(other) => self.answer = Some(other),
                    None => {}
                }
                if let Some(Choice::Keep { keep }) = self.answer.take() {
                    self.emit(Event::Mulligan { player: p, keep });
                    if keep {
                        let extra = self.mulligans_to_bottom(p);
                        if extra > 0 && !self.players[p as usize].hand.is_empty() {
                            self.run = Run::PreGame { stage: PreGame::Bottom { idx, n: extra } };
                        } else {
                            self.players[p as usize].kept = true;
                            self.run = Run::PreGame { stage: self.after_keep(p, idx) };
                        }
                    } else if self.cfg.mulligan == MulliganRule::FreeScry3 {
                        // the hand and the scried cards still on top go to the bottom in a random order (those already put
                        // on the bottom stay where they are), then a fresh seven
                        let mut gone = std::mem::take(&mut self.players[p as usize].hand);
                        for _ in 0..peek.len() {
                            if let Some(top) = self.players[p as usize].library.pop() {
                                gone.push(top);
                            }
                        }
                        self.players[p as usize].scry_bottomed = 0;
                        gone.shuffle(&mut self.rng);
                        for id in gone {
                            self.cards[id.0 as usize].zone = Zone::Library;
                            self.players[p as usize].library.insert(0, id);
                        }
                        self.players[p as usize].mulligans += 1;
                        for _ in 0..self.cfg.hand_size {
                            self.draw_card(p);
                        }
                        self.players[p as usize].drew_from_empty = false;
                        self.players[p as usize].cards_drawn_turn = 0;
                        self.run = Run::PreGame { stage: PreGame::Mulligan { idx } };
                    } else {
                        // shuffle the hand back and draw a new seven
                        let hand = std::mem::take(&mut self.players[p as usize].hand);
                        for id in hand {
                            self.cards[id.0 as usize].zone = Zone::Library;
                            self.players[p as usize].library.push(id);
                        }
                        self.shuffle_library(p);
                        self.players[p as usize].mulligans += 1;
                        let draw = match self.cfg.mulligan {
                            MulliganRule::Vancouver | MulliganRule::Paris => self.cfg.hand_size.saturating_sub(self.players[p as usize].mulligans),
                            _ => self.cfg.hand_size,
                        };
                        for _ in 0..draw {
                            self.draw_card(p);
                        }
                        self.players[p as usize].drew_from_empty = false;
                        self.players[p as usize].cards_drawn_turn = 0;
                        self.run = Run::PreGame { stage: PreGame::Mulligan { idx } };
                    }
                    return Flow::Continue;
                }
                if !self.players[p as usize].powder_declined && self.players[p as usize].hand.iter().any(|c| self.db.def(self.cards[c.0 as usize].def).abilities.iter().any(|a| matches!(a, Ability::MulliganExile))) {
                    self.run = Run::PreGame { stage: PreGame::Powder { idx } };
                    return Flow::Continue;
                }
                self.players[p as usize].powder_declined = false;
                let hand = self.players[p as usize].hand.clone();
                let mulligans = self.players[p as usize].mulligans;
                // the scrying player has seen these cards
                for id in &peek {
                    self.cards[id.0 as usize].known_to |= 1u8 << (p as u32 & 7);
                }
                self.pending = Some(Decision { player: p, kind: DecisionKind::Mulligan { hand, mulligans, peek } });
                self.run = Run::PreGame { stage: PreGame::Mulligan { idx } };
                Flow::Wait
            }
            PreGame::Sideboard { idx, k } => {
                let n = self.players.len();
                if idx >= n {
                    self.run = Run::PreGame { stage: PreGame::Color { idx: 0 } };
                    return Flow::Continue;
                }
                let p = idx as PlayerId;
                let cands = self.sideboard_candidates(p);
                let Some(&(card, _, _)) = cands.get(k) else {
                    self.run = Run::PreGame { stage: PreGame::Sideboard { idx: idx + 1, k: 0 } };
                    return Flow::Continue;
                };
                if let Some(Choice::Pick { indices }) = self.answer.take() {
                    let _ = card;
                    self.run = if indices.first() == Some(&1) { Run::PreGame { stage: PreGame::SideboardPick { idx, k } } } else { Run::PreGame { stage: PreGame::Sideboard { idx, k: k + 1 } } };
                    return Flow::Continue;
                }
                self.pending = Some(Decision {
                    player: p,
                    kind: DecisionKind::Pick { prompt: format!("Reveal {} from your deck and exile cards from your sideboard?", self.def_of(card).name), source: Some(card), options: vec![Opt::Text { text: "No".into() }, Opt::Text { text: "Yes".into() }], min: 1, max: 1, cancellable: false, hidden: false, purpose: PickPurpose::Generic },
                });
                self.run = Run::PreGame { stage: PreGame::Sideboard { idx, k } };
                Flow::Wait
            }
            PreGame::SideboardPick { idx, k } => {
                let p = idx as PlayerId;
                let cands = self.sideboard_candidates(p);
                let Some((card, options, min)) = cands.get(k).cloned() else {
                    self.run = Run::PreGame { stage: PreGame::Sideboard { idx: idx + 1, k: 0 } };
                    return Flow::Continue;
                };
                if let Some(Choice::Pick { indices }) = self.answer.take() {
                    let name = Sym::new(&self.def_of(card).name);
                    let mut picked: Vec<CardId> = indices.iter().filter_map(|i| options.get(*i).copied()).collect();
                    picked.dedup();
                    if picked.len() >= min as usize {
                        for id in picked {
                            self.cards[id.0 as usize].pregame_for = Some(name);
                            self.move_card(id, Zone::Exile, Default::default());
                            self.cards[id.0 as usize].pregame_for = Some(name);
                        }
                    }
                    self.run = Run::PreGame { stage: PreGame::Sideboard { idx, k: k + 1 } };
                    return Flow::Continue;
                }
                self.pending = Some(Decision {
                    player: p,
                    kind: DecisionKind::Pick { prompt: format!("Exile {} or more cards from your sideboard for {}", min, self.def_of(card).name), source: Some(card), options: options.iter().map(|c| Opt::Obj { obj: Obj::card(*c) }).collect(), min, max: options.len().min(255) as u8, cancellable: false, hidden: false, purpose: PickPurpose::Generic },
                });
                self.run = Run::PreGame { stage: PreGame::SideboardPick { idx, k } };
                Flow::Wait
            }
            PreGame::Powder { idx } => {
                let n = self.players.len();
                let p = ((self.first_player as usize + idx) % n) as PlayerId;
                if let Some(Choice::Pick { indices }) = self.answer.take() {
                    if indices.first() == Some(&1) {
                        let hand = std::mem::take(&mut self.players[p as usize].hand);
                        let k = hand.len();
                        for id in hand {
                            self.players[p as usize].hand.push(id);
                            self.move_card(id, Zone::Exile, Default::default());
                        }
                        for _ in 0..k {
                            self.draw_card(p);
                        }
                        self.players[p as usize].drew_from_empty = false;
                        self.players[p as usize].cards_drawn_turn = 0;
                    } else {
                        self.players[p as usize].powder_declined = true;
                    }
                    self.run = Run::PreGame { stage: PreGame::Mulligan { idx } };
                    return Flow::Continue;
                }
                self.pending = Some(Decision {
                    player: p,
                    kind: DecisionKind::Pick { prompt: "Serum Powder: exile your hand and draw that many cards?".into(), source: None, options: vec![Opt::Text { text: "No".into() }, Opt::Text { text: "Exile your hand and draw that many cards".into() }], min: 1, max: 1, cancellable: false, hidden: false, purpose: PickPurpose::Generic },
                });
                self.run = Run::PreGame { stage: PreGame::Powder { idx } };
                Flow::Wait
            }
            PreGame::Begin { idx } => {
                let n = self.players.len();
                if idx >= n {
                    self.start_first_turn();
                    return Flow::Continue;
                }
                let p = ((self.first_player as usize + idx) % n) as PlayerId;
                let has = p != self.first_player && self.players[p as usize].hand.iter().any(|c| self.db.def(self.cards[c.0 as usize].def).abilities.iter().any(|a| matches!(a, Ability::OpeningHandLuck)));
                if !has {
                    self.run = Run::PreGame { stage: PreGame::Begin { idx: idx + 1 } };
                    return Flow::Continue;
                }
                if let Some(Choice::Pick { indices }) = self.answer.take() {
                    if indices.first() == Some(&1) {
                        let card = self.players[p as usize].hand.iter().copied().find(|c| self.db.def(self.cards[c.0 as usize].def).abilities.iter().any(|a| matches!(a, Ability::OpeningHandLuck)));
                        if let Some(c) = card {
                            self.move_card(c, Zone::Battlefield, crate::zones::MoveOpts { controller: Some(p), ..Default::default() });
                            self.cards[c.0 as usize].since = 0;
                            self.add_counters_to(Obj::card(c), CounterKind::Luck, 1);
                            self.run = Run::PreGame { stage: PreGame::BeginExile { idx } };
                            return Flow::Continue;
                        }
                    }
                    // declined: this card stays in hand; look at the next seat
                    self.players[p as usize].luck_declined = true;
                    self.run = Run::PreGame { stage: PreGame::Begin { idx: idx + 1 } };
                    return Flow::Continue;
                }
                if self.players[p as usize].luck_declined {
                    self.run = Run::PreGame { stage: PreGame::Begin { idx: idx + 1 } };
                    return Flow::Continue;
                }
                self.pending = Some(Decision {
                    player: p,
                    kind: DecisionKind::Pick { prompt: "Gemstone Caverns: begin the game with it on the battlefield (exile a card from your hand)?".into(), source: None, options: vec![Opt::Text { text: "No".into() }, Opt::Text { text: "Begin the game on the battlefield".into() }], min: 1, max: 1, cancellable: false, hidden: false, purpose: PickPurpose::Generic },
                });
                self.run = Run::PreGame { stage: PreGame::Begin { idx } };
                Flow::Wait
            }
            PreGame::BeginExile { idx } => {
                let n = self.players.len();
                let p = ((self.first_player as usize + idx) % n) as PlayerId;
                if self.players[p as usize].hand.is_empty() {
                    self.run = Run::PreGame { stage: PreGame::Begin { idx: idx + 1 } };
                    return Flow::Continue;
                }
                if let Some(Choice::Pick { indices }) = self.answer.take() {
                    let hand = self.players[p as usize].hand.clone();
                    if let Some(&c) = indices.first().and_then(|i| hand.get(*i)) {
                        self.move_card(c, Zone::Exile, Default::default());
                    }
                    self.run = Run::PreGame { stage: PreGame::Begin { idx: idx + 1 } };
                    return Flow::Continue;
                }
                let options: Vec<Opt> = self.players[p as usize].hand.iter().map(|c| Opt::Obj { obj: Obj::card(*c) }).collect();
                self.pending = Some(Decision {
                    player: p,
                    kind: DecisionKind::Pick { prompt: "Gemstone Caverns: exile a card from your hand".into(), source: None, options, min: 1, max: 1, cancellable: false, hidden: false, purpose: PickPurpose::Generic },
                });
                self.run = Run::PreGame { stage: PreGame::BeginExile { idx } };
                Flow::Wait
            }
            PreGame::Scry { idx } => {
                let n_players = self.players.len();
                let p = ((self.first_player as usize + idx) % n_players) as PlayerId;
                let Some(&top) = self.players[p as usize].library.last() else {
                    self.run = Run::PreGame { stage: PreGame::Mulligan { idx: idx + 1 } };
                    return Flow::Continue;
                };
                if let Some(Choice::Pick { indices }) = self.answer.take() {
                    if indices.contains(&0) {
                        let lib = &mut self.players[p as usize].library;
                        lib.pop();
                        lib.insert(0, top);
                    }
                    self.run = Run::PreGame { stage: PreGame::Mulligan { idx: idx + 1 } };
                    return Flow::Continue;
                }
                self.pending = Some(Decision {
                    player: p,
                    kind: DecisionKind::Pick {
                        prompt: "Vancouver mulligan: scry 1 (choose the card to put on the bottom)".into(),
                        source: None,
                        options: vec![Opt::Obj { obj: Obj::card(top) }],
                        min: 0,
                        max: 1,
                        cancellable: false,
                        hidden: true,
                        purpose: PickPurpose::Scry,
                    },
                });
                self.run = Run::PreGame { stage: PreGame::Scry { idx } };
                Flow::Wait
            }
            PreGame::Bottom { idx, n } => {
                let n_players = self.players.len();
                let p = ((self.first_player as usize + idx) % n_players) as PlayerId;
                if let Some(Choice::Bottom { cards }) = self.answer.take() {
                    for id in cards {
                        self.players[p as usize].hand.retain(|x| *x != id);
                        self.cards[id.0 as usize].zone = Zone::Library;
                        self.players[p as usize].library.insert(0, id);
                    }
                    self.players[p as usize].kept = true;
                    self.run = Run::PreGame { stage: self.after_keep(p, idx) };
                    return Flow::Continue;
                }
                let hand = self.players[p as usize].hand.clone();
                self.pending = Some(Decision { player: p, kind: DecisionKind::Bottom { hand, n } });
                self.run = Run::PreGame { stage: PreGame::Bottom { idx, n } };
                Flow::Wait
            }
        }
    }

    /// When the seat a pre-game stage is about has left the game: the stage to go on with (the next seat's).
    fn pregame_after_departed(&mut self, stage: &PreGame) -> Option<PreGame> {
        let n = self.players.len();
        let in_turn_order = |idx: usize| (self.first_player as usize + idx) % n;
        let (idx, seat, next) = match *stage {
            PreGame::Sideboard { idx, .. } | PreGame::SideboardPick { idx, .. } => (idx, idx, PreGame::Sideboard { idx: idx + 1, k: 0 }),
            PreGame::Color { idx } => (idx, idx, PreGame::Color { idx: idx + 1 }),
            PreGame::Mulligan { idx } | PreGame::Powder { idx } | PreGame::Bottom { idx, .. } | PreGame::Scry { idx } => (idx, in_turn_order(idx), PreGame::Mulligan { idx: idx + 1 }),
            PreGame::Begin { idx } | PreGame::BeginExile { idx } => (idx, in_turn_order(idx), PreGame::Begin { idx: idx + 1 }),
        };
        // past the last seat the stage itself moves on to the next one
        if idx >= n || !self.players[seat].lost {
            return None;
        }
        self.players[seat].kept = true;
        Some(next)
    }

    /// Cards in `p`'s library with `PregameSideboardExile` that have enough matching sideboard cards: (card, matching sideboard cards, minimum).
    fn sideboard_candidates(&self, p: PlayerId) -> Vec<(CardId, Vec<CardId>, u8)> {
        let mut out = vec![];
        for &c in &self.players[p as usize].library {
            let Some((filter, min)) = self.db.def(self.cards[c.0 as usize].def).abilities.iter().find_map(|a| if let Ability::PregameSideboardExile { filter, min } = a { Some((filter.clone(), *min)) } else { None }) else { continue };
            let opts: Vec<CardId> = self.cards.iter().filter(|x| x.zone == Zone::Sideboard && x.owner == p).map(|x| x.id).filter(|&id| self.matches_card(&Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() }, id, c, p)).collect();
            if opts.len() >= min.max(1) as usize {
                out.push((c, opts, min.max(1)));
            }
        }
        out
    }

    fn mulligans_to_bottom(&self, p: PlayerId) -> u8 {
        let m = self.players[p as usize].mulligans;
        match self.cfg.mulligan {
            MulliganRule::London => m,
            MulliganRule::LondonFreeFirst => m.saturating_sub(1),
            MulliganRule::None | MulliganRule::Free | MulliganRule::Vancouver | MulliganRule::Paris | MulliganRule::FreeScry3 => 0,
        }
    }

    /// The stage after seat `idx` (player `p`) has kept: the Vancouver scry when the hand is smaller than the starting size, else the next seat.
    /// (Free mulligan + scry 3 scries while deciding, so keeping ends it.)
    fn after_keep(&self, p: PlayerId, idx: usize) -> PreGame {
        let pl = &self.players[p as usize];
        if self.cfg.mulligan == MulliganRule::Vancouver && pl.mulligans > 0 && pl.hand.len() < self.cfg.hand_size as usize && !pl.library.is_empty() {
            PreGame::Scry { idx }
        } else {
            PreGame::Mulligan { idx: idx + 1 }
        }
    }

    /// Opening-hand permanents (Leylines) begin the game on the battlefield, starting with the first player.
    pub(crate) fn begin_with_opening_hand_permanents(&mut self, auto_luck: bool) {
        let n = self.players.len();
        for k in 0..n {
            let p = ((self.first_player as usize + k) % n) as PlayerId;
            // Gemstone Caverns without the interactive pre-game (scenario driver): always taken by a player who isn't first, exiling the last card of the hand
            if auto_luck && p != self.first_player {
                let luck = self.players[p as usize].hand.iter().copied().find(|c| self.db.def(self.cards[c.0 as usize].def).abilities.iter().any(|a| matches!(a, Ability::OpeningHandLuck)));
                if let Some(c) = luck {
                    self.move_card(c, Zone::Battlefield, crate::zones::MoveOpts { controller: Some(p), ..Default::default() });
                    self.cards[c.0 as usize].since = 0;
                    self.add_counters_to(Obj::card(c), CounterKind::Luck, 1);
                    if let Some(&x) = self.players[p as usize].hand.last() {
                        self.move_card(x, Zone::Exile, Default::default());
                    }
                }
            }
            let leys: Vec<CardId> = self.players[p as usize]
                .hand
                .iter()
                .copied()
                .filter(|c| self.db.def(self.cards[c.0 as usize].def).abilities.iter().any(|a| matches!(a, Ability::OpeningHand)))
                .collect();
            for c in leys {
                self.move_card(c, Zone::Battlefield, crate::zones::MoveOpts { controller: Some(p), ..Default::default() });
                self.cards[c.0 as usize].since = 0;
            }
            // Chancellors: revealed from the opening hand, their effects happen at the beginning of the first upkeep
            let revealed: Vec<(CardId, Vec<Effect>, bool)> = self.players[p as usize]
                .hand
                .iter()
                .copied()
                .filter_map(|c| self.db.def(self.cards[c.0 as usize].def).abilities.iter().find_map(|a| if let Ability::OpeningHandReveal { effects, main_phase } = a { Some((c, effects.clone(), *main_phase)) } else { None }))
                .collect();
            for (c, effects, main_phase) in revealed {
                let did = self.next_effect_id;
                self.next_effect_id += 1;
                self.delayed.push(DelayedTrigger { id: did, controller: p, source: c, when: if main_phase { DelayedWhen::NextYourMainPhase } else { DelayedWhen::NextUpkeep }, effects, created_turn: 0, targets: vec![], ctx: Default::default(), remembered: vec![] });
            }
        }
    }

    fn start_first_turn(&mut self) {
        self.begin_with_opening_hand_permanents(false);
        for (p, id) in std::mem::take(&mut self.restart_return) {
            if self.cards[id.0 as usize].zone == Zone::Exile {
                self.move_card(id, Zone::Battlefield, crate::zones::MoveOpts { controller: Some(p), ..Default::default() });
            }
        }
        self.emit(Event::GameStart);
        self.turn = 0;
        self.active = self.prev_player(self.first_player);
        self.regular_active = self.active;
        self.begin_next_turn();
    }

    fn prev_player(&self, p: PlayerId) -> PlayerId {
        let n = self.players.len() as PlayerId;
        (p + n - 1) % n
    }

    // ------------------------------------------------------------------ turns and steps

    pub(crate) fn begin_next_turn(&mut self) {
        // who plays next: queued extra turns first
        let mut extra = self.extra_turns.pop();
        self.lose_end_turn = None;
        while let Some(ep) = extra {
            let idx = self.extra_turns.len();
            if self.skips_extra_turn(ep) {
                self.extra_lose_marks.retain(|m| *m != idx);
                extra = self.extra_turns.pop();
            } else {
                if self.extra_lose_marks.contains(&idx) {
                    self.extra_lose_marks.retain(|m| *m != idx);
                    self.lose_end_turn = Some(ep);
                }
                break;
            }
        }
        let mut next = extra.unwrap_or_else(|| self.next_player(self.regular_active));
        // skipped turns
        let mut guard = 0;
        while self.players[next as usize].skip_turns > 0 && guard < 64 {
            self.players[next as usize].skip_turns -= 1;
            next = self.next_player(next);
            guard += 1;
        }
        self.in_extra_turn = extra.is_some();
        if let Some(ep) = extra {
            self.players[ep as usize].began_extra_turn = true;
        }
        if extra.is_none() {
            self.regular_active = next;
        }
        // day / night (726.2): checked as a new turn begins, based on the spells the previous active player cast
        if self.turn >= 1 {
            if let Some(day) = self.day_night {
                let cast = self.players[self.active as usize].spells_cast;
                let now = if day && cast == 0 {
                    Some(false)
                } else if !day && cast >= 2 {
                    Some(true)
                } else {
                    None
                };
                if let Some(day_now) = now {
                    self.day_night = Some(day_now);
                    self.emit(Event::DayNightChanged { day: day_now });
                    let want = if day_now { Keyword::Nightbound } else { Keyword::Daybound };
                    let flip: Vec<CardId> = self.battlefield.iter().copied().filter(|id| self.def_of(*id).keywords.has(want)).collect();
                    for id in flip {
                        self.transform(id);
                    }
                }
            }
        }
        // "can't attack you during their next turn" bars: the turn that just ended expires them, the turn that begins arms them
        let ended = self.active;
        self.no_attack_next.retain(|e| !(e.2 && e.0 == ended));
        self.no_attack_pending.retain(|e| e.1 != next);
        self.attack_nearest.retain(|e| e.0 != next);
        self.sanctuary.retain(|q| *q != next);
        for e in self.no_attack_next.iter_mut() {
            if e.0 == next {
                e.2 = true;
            }
        }
        self.turn += 1;
        self.monarch_turn_start = self.monarch;
        self.active = next;
        self.planechase_new_turn();
        {
            let t = self.turn;
            let p = &mut self.players[next as usize];
            p.prev_turn = p.cur_turn;
            p.cur_turn = t;
        }
        self.step = Step::Untap;
        for i in 0..self.battlefield.len() {
            let id = self.battlefield[i];
            self.cards[id.0 as usize].tapped_at_turn_start = self.cards[id.0 as usize].tapped;
        }
        // "your turn" conditions and the like change with the active player
        self.layers_dirty = true;
        self.temp_triggers.retain(|t| !(t.until == TempUntil::YourNextTurn && t.d.controller == next && t.d.created_turn < self.turn));
        // "until your next turn" lasts until that controller's next turn begins (with more than two players, not just through the next turn)
        {
            let now = self.turn;
            // a controller who has left the game: the effect ends when their turn would have begun (their seat is passed)
            let n = self.players.len() as PlayerId;
            let skipped = |c: PlayerId| {
                let mut s = (ended + 1) % n;
                while s != next {
                    if s == c {
                        return true;
                    }
                    s = (s + 1) % n;
                }
                false
            };
            let lost: Vec<bool> = self.players.iter().map(|p| p.lost).collect();
            self.effects.retain(|e| !(e.duration == Duration::UntilYourNextTurn && e.created_turn < now && (e.controller == next || (lost[e.controller as usize] && skipped(e.controller)))));
        }
        self.combat_damagers.clear();
        self.combat_damagers_ctl.clear();
        self.adapt_ignore.clear();
        self.no_attack_pairs.clear();
        self.no_attack_pw_pairs.clear();
        self.no_attack_perms.clear();
        self.ignored_until_eot.clear();
        self.block_chooser.clear();
        self.attack_chooser = None;
        let starting = self.active;
        for c in self.cards.iter_mut() {
            if c.detained_by == Some(starting) {
                c.detained_by = None;
            }
        }
        for p in self.players.iter_mut() {
            p.lands_played = 0;
            p.land_limit = 1;
            p.spells_cast_last = p.spells_cast;
            p.spells_cast = 0;
            p.life_gained = 0;
            p.life_gain_events = 0;
            p.life_lost_last = p.life_lost;
            p.life_turn_start = p.life;
            p.life_lost = 0;
            p.damaged = false;
            p.damage_taken_turn = 0;
            p.discarded_turn = 0;
            p.cycled_turn = 0;
            p.crime = false;
            p.speed_up = false;
            p.mana_spent_turn = 0;
            p.cards_drawn_turn = 0;
            p.attacked_turn = false;
            p.searched_turn = false;
            p.investigated_turn = 0;
            p.scried_turn = false;
            p.dilemmas_turn = 0;
            p.loyalty_turn = false;
            p.exhaust_turn = false;
            p.equip_turn = false;
            p.surveilled_turn = false;
            p.surveil_count_turn = 0;
            p.lands_entered = 0;
            p.creatures_entered_last = p.creatures_entered;
            p.creatures_entered = 0;
        }
        self.turn_ctl = None;
        self.turn_ctl_combat = false;
        if let Some(i) = self.next_turn_ctl.iter().position(|(v, _, _)| *v == next) {
            let (v, c, combat) = self.next_turn_ctl.remove(i);
            self.turn_ctl = Some((v, c));
            self.turn_ctl_combat = combat;
        }
        self.players[next as usize].attacked_last_turn = 0;
        self.players[next as usize].prev_turn_start = self.players[next as usize].last_turn_start;
        self.players[next as usize].last_turn_start = self.turn;
        self.players[next as usize].hand_turn_start = self.players[next as usize].hand.len() as i32;
        {
            let n = self.battlefield.iter().filter(|id| { let c = &self.cards[id.0 as usize]; c.controller == next && !c.tapped && c.cur.types.has(CardType::Land) }).count() as i32;
            self.players[next as usize].lands_untapped_turn_start = n;
        }
        self.players[next as usize].damage_shield = false;
        self.players[next as usize].shield_bar = false;
        self.players[next as usize].shield_tax = 0;
        self.players[next as usize].turns_taken += 1;
        for c in self.cards.iter_mut() {
            c.activations.clear();
        }
        self.lands_entered_turn = 0;
        self.entered_log.clear();
        self.attackers_log.clear();
        self.token_attackers_log.clear();
        self.commander_attackers_log.clear();
        self.defended_log.clear();
        self.counters_log.clear();
        self.creatures_died_turn = 0;
        self.pile_guess_turn = 0;
        self.dmg_by_source.borrow_mut().clear();
        self.creatures_died_by.clear();
        self.left_graveyard_by.clear();
        self.left_graveyard_creature_by.clear();
        self.gy_entries.clear();
        self.exile_entries.clear();
        self.exile_mv_log.clear();
        self.combat_gy_start = 0;
        self.sacrificed_turn.clear();
        self.land_mana_turn = 0;
        self.nc_destroyed_by_opp = 0;
        self.sacrificed_types.clear();
        self.tokens_made_by.clear();
        self.treasures_made_by.clear();
        self.bounced_turn.clear();
        self.left_bf_turn.clear();
        self.left_bf_power.clear();
        self.left_bf_kw.clear();
        self.token_gy_turn.clear();
        self.oil_died_turn = false;
        for c in self.cards.iter_mut() {
            c.oil_removed_by = None;
        }
        self.died_turn.clear();
        self.died_power.clear();
        self.died_modified.clear();
        self.combats_this_turn = 0;
        self.attacked_players_turn.clear();
        self.block_log.clear();
        self.extra_combat_now = 0;
        self.extra_upkeep = 0;
        self.upkeep_return = None;
        self.extra_beginning = 0;
        self.in_extra_beginning = false;
        self.extra_beginning_combat = 0;
        self.extra_beginning_return = None;
        self.upkeeps_this_turn = 0;
        self.extra_end = 0;
        self.end_steps_this_turn = 0;
        self.extra_combat_after_main = 0;
        self.extra_main_pending = 0;
        self.extra_main_repeat = 0;
        self.spells_this_turn = 0;
        self.spells_log.clear();
        self.mana_drained_by.clear();
        self.poison_capped.set(0);
        self.graveyard_use_log.clear();
        self.gy_instant_sorcery_log.clear();
        self.foretold_log.clear();
        self.artifact_act_log.clear();
        self.creature_tgt_act_log.clear();
        self.power_up_act_log.clear();
        self.countered_by.clear();
        self.spells_countered.clear();
        for r in self.revolt.iter_mut() {
            *r = false;
        }
        for r in self.descended.iter_mut() {
            *r = false;
        }
        self.nonland_left_turn = false;
        self.warped_this_turn = false;
        self.emit(Event::TurnBegin { turn: self.turn, player: next });
        self.run = Run::BeginStep;
    }

    /// Cards that gained suspend from an effect: at their owner's upkeep remove a time counter; with the last gone they are cast for free.
    pub(crate) fn granted_suspend_upkeep(&mut self, p: PlayerId) {
        let list: Vec<CardId> = self.players[p as usize].exile.iter().copied().filter(|id| self.cards[id.0 as usize].granted_suspend && self.cards[id.0 as usize].counter(CounterKind::Time) > 0).collect();
        for id in list {
            self.remove_counters_from(Obj::card(id), CounterKind::Time, 1);
            if self.cards[id.0 as usize].counter(CounterKind::Time) == 0 {
                if self.def_of(id).types.has(CardType::Creature) {
                    self.cards[id.0 as usize].free_haste = true;
                }
                self.cards[id.0 as usize].granted_suspend = false;
                // "when the last time counter is removed, you may cast it without paying its mana cost": a trigger the owner may decline
                let did = self.next_effect_id;
                self.next_effect_id += 1;
                let cast = Effect::CastFree { what: Sel::This, may: true, decline_to_bottom: false, decline_to_hand: false, exile_after: false, copy: false, pay: false, by: None, any_mana: false, lands: false, alt: None, extra_mana: None, otherwise: vec![], suspend: false, lock_spells_after: false, on_cast: vec![], after_cast: vec![], bottom_after: false, copies: None, face_choice: false };
                self.push_delayed_item(DelayedTrigger { id: did, controller: p, source: id, when: DelayedWhen::NextUpkeep, effects: vec![cast], created_turn: self.turn, targets: vec![], ctx: Default::default(), remembered: vec![] });
            }
        }
    }

    /// Ertai's Meddling: at the upkeep of the spell's controller a delay counter comes off the exiled card; with none left a copy of the original spell goes onto the stack.
    pub(crate) fn meddle_upkeep(&mut self, p: PlayerId) {
        let mut i = 0;
        while i < self.meddled.len() {
            let (card, zcc, pl, _) = &self.meddled[i];
            let (card, zcc, pl) = (*card, *zcc, *pl);
            if self.cards[card.0 as usize].zone != Zone::Exile || self.cards[card.0 as usize].zcc != zcc {
                self.meddled.remove(i);
                continue;
            }
            if pl != p {
                i += 1;
                continue;
            }
            self.remove_counters_from(Obj::card(card), CounterKind::Delay, 1);
            if self.cards[card.0 as usize].counter(CounterKind::Delay) == 0 {
                let (_, _, _, item) = self.meddled.remove(i);
                self.next_stack_id += 1;
                let mut copy = item;
                copy.id = StackId(self.next_stack_id);
                copy.copy = true;
                copy.controller = p;
                let src = copy.source;
                self.stack.push(copy);
                self.emit(Event::Copied { card: src, player: p });
                continue;
            }
            i += 1;
        }
    }

    /// "End the turn": exile all spells on the stack, drop abilities, remove combat, jump to cleanup.
    pub(crate) fn do_end_turn(&mut self) {
        self.end_turn_pending = false;
        let items: Vec<StackItem> = std::mem::take(&mut self.stack);
        for it in items {
            if matches!(it.kind, ItemKind::Spell { .. }) && !it.copy && self.cards[it.source.0 as usize].zone == Zone::Stack {
                self.move_card(it.source, Zone::Exile, Default::default());
            }
        }
        self.pending_triggers.clear();
        self.combat = None;
        self.block_limits.clear();
        self.goto_step(Step::Cleanup);
    }

    /// "End the combat phase": exile all spells on the stack, remove combat, continue in the postcombat main phase.
    pub(crate) fn do_end_combat(&mut self) {
        self.end_combat_pending = false;
        let items: Vec<StackItem> = std::mem::take(&mut self.stack);
        for it in items {
            if matches!(it.kind, ItemKind::Spell { .. }) && !it.copy && self.cards[it.source.0 as usize].zone == Zone::Stack {
                self.move_card(it.source, Zone::Exile, Default::default());
            }
        }
        self.pending_triggers.clear();
        self.combat = None;
        self.block_limits.clear();
        self.goto_step(Step::Main2);
    }

    /// Past a skipped draw step: the precombat main phase, or the end step when this was an additional beginning phase.
    fn goto_after_draw(&mut self) {
        if self.in_extra_beginning {
            self.in_extra_beginning = false;
            let r = self.extra_beginning_return.take().unwrap_or(Step::End);
            self.goto_step(r);
        } else {
            self.goto_step(Step::Main1);
        }
    }

    fn goto_step(&mut self, s: Step) {
        // "unblocked" and similar filters depend on the step
        self.layers_dirty = true;
        // last-known information only lives for the step it was recorded in (zone moves of other cards don't wipe it)
        self.last_lki = None;
        self.left_lki.clear();
        if s == Step::BeginCombat {
            self.combats_this_turn = self.combats_this_turn.saturating_add(1);
            let ap = self.active as usize;
            self.players[ap].combat_serial = self.players[ap].combat_serial.saturating_add(1);
            self.combat_gy_start = self.gy_entries.len();
        }
        if s == Step::Upkeep {
            self.upkeeps_this_turn += 1;
        }
        if s == Step::End {
            self.end_steps_this_turn += 1;
        }
        self.step = s;
        self.run = Run::BeginStep;
    }

    pub(crate) fn step_begin(&mut self) -> Flow {
        let p = self.active;
        if self.answer.is_none() && !self.attach_pending.is_empty() {
            self.run = Run::BeginStep;
            if let Some(f) = self.ask_attach_player(None) {
                return f;
            }
        }
        // mana empties at the end of each step and phase
        let in_combat = matches!(self.step, Step::BeginCombat | Step::DeclareAttackers | Step::DeclareBlockers | Step::FirstStrikeDamage | Step::CombatDamage | Step::EndCombat);
        let persist: Vec<[bool; 6]> = (0..self.players.len() as PlayerId).map(|q| {
            let mut a = [false; 6];
            for ty in crate::mana::ManaType::ALL {
                a[ty as usize] = self.mana_persists(q, ty);
            }
            a
        }).collect();
        let to_colorless: Vec<bool> = (0..self.players.len() as PlayerId).map(|q| self.mana_becomes_colorless(q)).collect();
        let to_red: Vec<bool> = (0..self.players.len() as PlayerId).map(|q| self.mana_becomes_red(q)).collect();
        let to_black: Vec<bool> = (0..self.players.len() as PlayerId).map(|q| self.mana_becomes_black(q)).collect();
        let loss_life: Vec<bool> = (0..self.players.len() as PlayerId).map(|q| self.mana_loss_costs_life(q)).collect();
        let mut life_lost: Vec<(PlayerId, i32)> = vec![];
        for (pi, pl) in self.players.iter_mut().enumerate() {
            let before = pl.pool;
            // firebending: mana flagged "until end of combat" stays while combat lasts (as far as it has not been spent)
            let kept = pl.pool_keep;
            let kept_turn = pl.pool_turn;
            if self.step == Step::Untap {
                pl.restricted.clear();
                pl.restricted_turn.clear();
            } else {
                pl.restricted_turn.resize(pl.restricted.len(), false);
                for (i, slot) in pl.restricted.iter_mut().enumerate() {
                    if !pl.restricted_turn[i] {
                        // Horizon Stone / Kruphix: restricted mana turns colourless (keeping its restriction) instead of emptying
                        match slot {
                            Some(m) if to_colorless[pi] => m.0 = crate::mana::ManaType::C,
                            _ => *slot = None,
                        }
                    }
                }
            }
            let mut new_turn = kept_turn;
            for t in crate::mana::ManaType::ALL {
                let n = (if in_combat { kept.get(t) } else { 0 }).max(kept_turn.get(t)).max(if persist[pi][t as usize] { pl.pool.get(t) } else { 0 }).min(pl.pool.get(t));
                pl.pool.0[t as usize] = n;
                new_turn.0[t as usize] = kept_turn.get(t).min(n);
            }
            pl.pool_keep = if in_combat { pl.pool } else { Default::default() };
            pl.pool_turn = new_turn;
            if self.step == Step::Untap {
                // a new turn: nothing is kept (except mana that a static keeps)
                for ty in crate::mana::ManaType::ALL {
                    if !persist[pi][ty as usize] {
                        pl.pool.0[ty as usize] = 0;
                    }
                }
                pl.pool_keep.clear();
                pl.pool_turn.clear();
            }
            let lost: u16 = crate::mana::ManaType::ALL.iter().map(|&t| before.get(t).saturating_sub(pl.pool.get(t))).sum();
            if to_colorless[pi] {
                // Horizon Stone: the mana that would be lost becomes colorless instead
                pl.pool.add(crate::mana::ManaType::C, lost);
            } else if to_red[pi] {
                // Ozai, the Phoenix King: the mana that would be lost becomes red instead
                pl.pool.add(crate::mana::ManaType::R, lost);
            } else if to_black[pi] {
                // Omnath, Locus of All: the mana that would be lost becomes black instead
                pl.pool.add(crate::mana::ManaType::B, lost);
            } else if loss_life[pi] && lost > 0 {
                life_lost.push((pi as PlayerId, lost as i32));
            }
        }
        for (q, n) in life_lost {
            self.lose_life(q, n);
        }
        let step = self.step;
        if step == Step::Untap {
            self.player_prot.retain(|pp| !(pp.until == Duration::UntilYourNextTurn && pp.owner == p) && !(pp.until == Duration::UntilAffectedPlayersNextTurn && pp.player == p));
            self.effects.retain(|e| !(e.duration == Duration::UntilAffectedPlayersNextTurn && e.players.contains(&p)));
            self.long_replaces.retain(|(_, c, _)| *c != p);
            let (due, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut self.copy_reverts_turn).into_iter().partition(|r| r.3 == p);
            self.copy_reverts_turn = keep;
            for (id, zcc, def, _) in due.into_iter().rev() {
                if self.cards[id.0 as usize].zcc == zcc && self.cards[id.0 as usize].zone == Zone::Battlefield {
                    self.cards[id.0 as usize].def = def;
                    self.layers_dirty = true;
                }
            }
        }
        // a skipped untap step: no untapping, no phasing, no step
        if step == Step::Untap && self.untap_step_skipped_by_static(p) {
            self.goto_step(Step::Upkeep);
            return Flow::Continue;
        }
        if step == Step::Untap && self.players[p as usize].skip_untap > 0 {
            self.players[p as usize].skip_untap -= 1;
            self.goto_step(Step::Upkeep);
            return Flow::Continue;
        }
        if step == Step::Untap && !std::mem::take(&mut self.untap_pick_done) {
            self.untap_only = None;
            let mut options: Vec<CardType> = vec![];
            for &sid in &self.battlefield {
                let sc = &self.cards[sid.0 as usize];
                if sc.cur.no_abilities {
                    continue;
                }
                let sd = self.db.def(sc.def);
                for &i in &sd.statics {
                    if let Ability::Static { effect: StaticEffect::UntapOnlyChosenType { options: o }, condition, .. } = &sd.abilities[i as usize] {
                        if condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &crate::eval::EvalCtx::with_controller(sid, sc.cur.controller))) {
                            options = if options.is_empty() { o.clone() } else { options.into_iter().filter(|t| o.contains(t)).collect() };
                        }
                    }
                }
            }
            if !options.is_empty() && self.alive(p) {
                self.pending = Some(Decision {
                    player: p,
                    kind: DecisionKind::Pick {
                        prompt: "Choose a permanent type: you can untap only permanents of that type this step".into(),
                        source: None,
                        options: options.iter().map(|t| Opt::Text { text: t.name().to_string() }).collect(),
                        min: 1,
                        max: 1,
                        cancellable: false,
                        hidden: false,
                        purpose: PickPurpose::Generic,
                    },
                });
                self.run = Run::UntapPickType { options };
                return Flow::Wait;
            }
        }
        if step == Step::Untap && !std::mem::take(&mut self.untap_limit_done) {
            self.untap_prefer = None;
            // Winter Orb-style limits ("can't untap more than two permanents"): the affected player chooses which untap
            let limits = self.untap_limits(p);
            let mut pick: Option<(Vec<CardId>, u8)> = None;
            for (flt, max, sid, sctl, _) in &limits {
                let cands: Vec<CardId> = self
                    .battlefield
                    .iter()
                    .copied()
                    .filter(|id| self.cards[id.0 as usize].cur.controller == p && self.cards[id.0 as usize].tapped && !self.doesnt_untap(*id) && self.matches_card(flt, *id, *sid, *sctl))
                    .collect();
                if cands.len() > *max as usize && *max > 0 && pick.as_ref().map_or(true, |(_, m)| *max < *m) {
                    pick = Some((cands, *max));
                }
            }
            if let Some((cands, max)) = pick {
                if self.alive(p) {
                    self.pending = Some(Decision {
                        player: p,
                        kind: DecisionKind::Pick {
                            prompt: format!("Choose up to {max} permanents to untap"),
                            source: None,
                            options: cands.iter().map(|c| Opt::Obj { obj: Obj::card(*c) }).collect(),
                            min: max,
                            max: max,
                            cancellable: false,
                            hidden: false,
                            purpose: PickPurpose::Generic,
                        },
                    });
                    self.run = Run::UntapLimitPick { ids: cands };
                    return Flow::Wait;
                }
            }
        }
        if step == Step::Upkeep {
            self.players[p as usize].lands_no_untap = false;
            self.players[p as usize].creatures_no_untap = false;
            let (act, now) = (p, self.turn);
            self.effects.retain(|e| !(e.duration == Duration::UntilYourNextUpkeep && e.controller == act && e.created_turn < now));
            self.layers_dirty = true;
        }
        // skipped upkeep steps (Eon Hub): no step, no triggers, no priority
        if step == Step::Upkeep && self.skips_upkeep_step(p) {
            self.goto_step(Step::Draw);
            return Flow::Continue;
        }
        // phases skipped for this whole turn (Fatespinner)
        {
            let (t, bits) = self.players[p as usize].skip_phases;
            if t == self.turn && bits != 0 {
                if step == Step::BeginCombat && bits & 4 != 0 {
                    self.combat = None;
                    self.goto_step(Step::Main2);
                    return Flow::Continue;
                }
                if step == Step::Main1 && bits & 2 != 0 {
                    self.goto_step(Step::BeginCombat);
                    return Flow::Continue;
                }
                if step == Step::Main2 && bits & 2 != 0 {
                    self.goto_step(Step::End);
                    return Flow::Continue;
                }
            }
        }
        // a skipped combat phase: from the beginning of combat straight to the second main phase
        if step == Step::BeginCombat && self.players[p as usize].skip_combat > 0 && self.turn > self.players[p as usize].skip_combat_after {
            self.players[p as usize].skip_combat -= 1;
            self.combat = None;
            self.goto_step(Step::Main2);
            return Flow::Continue;
        }
        // skipped draw steps (Fatigue, Necropotence): no step, no priority
        if step == Step::Draw && (self.players[p as usize].skip_draw > 0 || self.skips_draw_step(p) || (self.players[p as usize].skip_phases.0 == self.turn && self.players[p as usize].skip_phases.1 & 1 != 0)) {
            if self.players[p as usize].skip_draw > 0 {
                self.players[p as usize].skip_draw -= 1;
            }
            self.goto_after_draw();
            return Flow::Continue;
        }
        // 103.8a: the starting player of a two-player game skips the whole draw step of their first turn
        if step == Step::Draw && self.turn == 1 && self.players.len() == 2 && !self.in_extra_beginning {
            self.goto_step(Step::Main1);
            return Flow::Continue;
        }
        if step == Step::Draw && std::mem::take(&mut self.draw_skip_declined) == false {
            if let Some((src, gain)) = self.may_skip_draw_step(p) {
                self.pending = Some(Decision {
                    player: p,
                    kind: DecisionKind::YesNo { prompt: format!("Skip your draw step (gain {gain} life)?"), source: Some(src) },
                });
                self.run = Run::DrawSkipAsk { gain };
                return Flow::Wait;
            }
        }
        // radiation: at the beginning of the precombat main phase the player mills cards equal to their rad counters; each nonland card
        // milled this way costs them 1 life and removes a rad counter
        if step == Step::Main1 && self.players[p as usize].rad > 0 && self.alive(p) {
            let n = self.players[p as usize].rad;
            let mut lost = 0;
            for _ in 0..n {
                if let Some(&id) = self.players[p as usize].library.last() {
                    let is_land = self.db.def(self.cards[id.0 as usize].def).types.has(CardType::Land);
                    self.move_card(id, Zone::Graveyard, Default::default());
                    if !is_land {
                        lost += 1;
                    }
                }
            }
            if lost > 0 {
                self.players[p as usize].rad = (self.players[p as usize].rad - lost).max(0);
                if self.radiation_gains_life(p) {
                    self.gain_life(p, lost);
                } else {
                    self.lose_life(p, lost);
                }
            }
        }
        self.emit(Event::StepBegin { step, player: p });
        match step {
            Step::Untap => {
                // "as you untap": delayed effects that happen at once, before anything untaps
                let (now, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut self.delayed).into_iter().partition(|d| d.when == DelayedWhen::NextYourUntap && d.controller == p);
                self.delayed = keep;
                for d in now {
                    for e in d.effects.iter() {
                        self.run_effect_now(d.source, d.controller, e);
                    }
                }
                // "until its controller's next untap step"
                let cards = &self.cards;
                self.effects.retain(|e| !(e.duration == Duration::UntilAffectedUntaps && e.affects.iter().any(|(id, _)| cards[id.0 as usize].cur.controller == p)));
                self.layers_dirty = true;
                // phasing: permanents with phasing phase out, and this player's phased-out permanents phase in, simultaneously
                let phasing_out: Vec<CardId> = self.battlefield.iter().copied().filter(|id| self.cards[id.0 as usize].cur.controller == p && self.cards[id.0 as usize].cur.kw.has(Keyword::Phasing)).collect();
                let phasing_in: Vec<CardId> = self.phased.iter().copied().filter(|id| self.cards[id.0 as usize].cur.controller == p && !self.cards[id.0 as usize].phased_indirect && self.cards[id.0 as usize].phase_hold.is_none()).collect();
                for id in phasing_in {
                    self.phase_in(id);
                }
                for id in phasing_out {
                    if self.cards[id.0 as usize].zone == Zone::Battlefield && !self.cards[id.0 as usize].phased_out {
                        self.phase_out(id, false);
                    }
                }
                let mut mine: Vec<CardId> = self.battlefield.iter().copied().filter(|id| self.cards[id.0 as usize].cur.controller == p).collect();
                let mut optional: Vec<CardId> = vec![];
                let mut limits = self.untap_limits(p);
                let prefer = self.untap_prefer.take();
                // the permanents the player chose to untap claim the limits' allowances first
                if let Some(pf) = &prefer {
                    mine.sort_by_key(|id| !pf.contains(id));
                }
                let only = self.untap_only.take();
                for id in mine {
                    if let Some(t) = only {
                        if !self.cards[id.0 as usize].cur.types.has(t) {
                            continue;
                        }
                    }
                    if self.cards[id.0 as usize].tapped {
                        // a permanent that one limit blocks doesn't use up another limit's allowance (Winter Moon with Static Orb)
                        let blocked = limits.iter().any(|(flt, max, sid, sctl, n)| self.matches_card(flt, id, *sid, *sctl) && *n >= *max as u32);
                        if blocked {
                            continue;
                        }
                        for (flt, _max, sid, sctl, n) in limits.iter_mut() {
                            if self.matches_card(flt, id, *sid, *sctl) {
                                *n += 1;
                            }
                        }
                    }
                    if self.cards[id.0 as usize].tapped && !self.doesnt_untap(id) {
                        // "if this would untap, remove a counter from it instead; if you do, untap it" (Bewitching Leechcraft)
                        let mut put: Option<(CounterKind, u8)> = None;
                        for &sid in self.battlefield.iter().chain(self.planes.iter()) {
                            let sc = &self.cards[sid.0 as usize];
                            if sc.cur.no_abilities {
                                continue;
                            }
                            let sd = self.db.def(sc.def);
                            for &i in &sd.statics {
                                if let Ability::Static { effect: StaticEffect::UntapAddCounters { filter, kind, n }, condition, .. } = &sd.abilities[i as usize] {
                                    if self.matches_card(filter, id, sid, sc.cur.controller)
                                        && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &crate::eval::EvalCtx::with_controller(sid, sc.cur.controller)))
                                    {
                                        put = Some((*kind, *n));
                                    }
                                }
                            }
                        }
                        if let Some((k, n)) = put {
                            self.add_counters_to(Obj::card(id), k, n as i32);
                            self.layers_dirty = true;
                            continue;
                        }
                        let mut pay: Option<CounterKind> = None;
                        for &sid in &self.battlefield {
                            let sc = &self.cards[sid.0 as usize];
                            if sc.cur.no_abilities {
                                continue;
                            }
                            let sd = self.db.def(sc.def);
                            for &i in &sd.statics {
                                if let Ability::Static { effect: StaticEffect::UntapPayCounter { filter, kind }, condition, .. } = &sd.abilities[i as usize] {
                                    if self.matches_card(filter, id, sid, sc.cur.controller)
                                        && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &crate::eval::EvalCtx::with_controller(sid, sc.cur.controller)))
                                    {
                                        pay = Some(*kind);
                                    }
                                }
                            }
                        }
                        if let Some(k) = pay {
                            if self.cards[id.0 as usize].counter(k) > 0 {
                                self.cards[id.0 as usize].remove_counters(k, 1);
                                self.layers_dirty = true;
                                self.untap_card(id);
                            }
                            continue;
                        }
                        let mut kinds: Vec<CounterKind> = vec![];
                        for &sid in &self.battlefield {
                            let sc = &self.cards[sid.0 as usize];
                            if sc.cur.no_abilities {
                                continue;
                            }
                            let sd = self.db.def(sc.def);
                            for &i in &sd.statics {
                                if let Ability::Static { effect: StaticEffect::UntapRemoveCounters { filter, kind }, condition, .. } = &sd.abilities[i as usize] {
                                    if self.cards[id.0 as usize].counter(*kind) > 0
                                        && self.matches_card(filter, id, sid, sc.cur.controller)
                                        && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &crate::eval::EvalCtx::with_controller(sid, sc.cur.controller)))
                                    {
                                        kinds.push(*kind);
                                    }
                                }
                            }
                        }
                        if !kinds.is_empty() {
                            for k in kinds {
                                let n = self.cards[id.0 as usize].counter(k);
                                if n > 0 {
                                    self.cards[id.0 as usize].remove_counters(k, n);
                                }
                            }
                            continue;
                        }
                    }
                    let stun = self.cards[id.0 as usize].counter(CounterKind::Stun);
                    if self.cards[id.0 as usize].skip_untap && self.cards[id.0 as usize].skip_untap_by.map_or(false, |b| b != p) {
                        // exerted by someone else: that player's untap step is the one skipped, not this one
                        self.cards[id.0 as usize].skip_untap = false;
                        self.cards[id.0 as usize].skip_untap_by = None;
                        if self.cards[id.0 as usize].tapped && !self.doesnt_untap(id) {
                            self.untap_card(id);
                        }
                    } else if self.cards[id.0 as usize].skip_untap {
                        self.cards[id.0 as usize].skip_untap = false;
                        self.cards[id.0 as usize].skip_untap_by = None;
                    } else if stun > 0 {
                        if !self.stun_locked(id) {
                            self.cards[id.0 as usize].remove_counters(CounterKind::Stun, 1);
                        }
                    } else if self.cards[id.0 as usize].tapped && !self.doesnt_untap(id) {
                        if self.untap_optional(id) {
                            optional.push(id);
                        } else {
                            self.untap_card(id);
                        }
                    }
                }
                // "untap during each other player's untap step": the static's controller's matching permanents untap now
                let mut extra: Vec<CardId> = vec![];
                // a face-up plane (Horizon Boughs) applies to every player's permanents, whoever's untap step it is
                let sources: Vec<(CardId, bool)> = self.battlefield.iter().map(|&c| (c, false)).chain(self.planes.iter().map(|&c| (c, true))).chain(self.emblem_srcs().into_iter().filter(|c| !self.planes.contains(c)).map(|c| (c, false))).collect();
                for (sid, is_plane) in sources {
                    let sc = &self.cards[sid.0 as usize];
                    // a scheme (command zone) is controlled by its owner
                    let sctl = self.src_ctrl(sid);
                    if sc.cur.no_abilities || (!is_plane && sctl == p) {
                        continue;
                    }
                    let sd = self.db.def(sc.def);
                    for &i in &sd.statics {
                        if let Ability::Static { effect: StaticEffect::UntapDuringOthers { filter }, condition, .. } = &sd.abilities[i as usize] {
                            if condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &crate::eval::EvalCtx::with_controller(sid, sctl))) {
                                for &id in &self.battlefield {
                                    if self.cards[id.0 as usize].tapped && (is_plane || self.cards[id.0 as usize].cur.controller == sctl) && self.matches_card(filter, id, sid, sctl) {
                                        extra.push(id);
                                    }
                                }
                            }
                        }
                    }
                }
                extra.sort_by_key(|c| c.0);
                extra.dedup();
                for id in extra {
                    if only.map_or(false, |t| !self.cards[id.0 as usize].cur.types.has(t)) {
                        continue;
                    }
                    // "doesn't untap during your untap step" (Claustrophobia) says nothing about another player's untap step
                    // but "creatures don't untap during target player's next untap step" (Blinding Beam) covers every untap in it
                    if self.cards[id.0 as usize].skip_untap && self.cards[id.0 as usize].skip_untap_by == Some(p) {
                        self.cards[id.0 as usize].skip_untap = false;
                        continue;
                    }
                    self.untap_card(id);
                }
                if !optional.is_empty() {
                    self.run = Run::UntapChoose { queue: optional, asked: false };
                    return Flow::Continue;
                }
                self.goto_step(Step::Upkeep);
                Flow::Continue
            }
            Step::Draw => {
                self.draw_with_replacement(p, true, false);
                self.flush_followups();
                self.fire_delayed(DelayedWhen::NextTriggerPlayerDrawStep);
                self.run = Run::Priority { player: p, passes: 0 };
                Flow::Continue
            }
            Step::DeclareAttackers => {
                self.run = Run::Attackers;
                Flow::Continue
            }
            Step::DeclareBlockers => {
                if self.combat.as_ref().map_or(true, |c| c.attackers.is_empty()) {
                    self.combat = None;
                    self.goto_step(Step::EndCombat);
                } else {
                    self.run = Run::Blockers { idx: 0 };
                }
                Flow::Continue
            }
            Step::FirstStrikeDamage => {
                if self.combat.is_some() && self.first_strike_step_needed() {
                    self.begin_damage(true);
                } else {
                    self.goto_step(Step::CombatDamage);
                }
                Flow::Continue
            }
            Step::CombatDamage => {
                if self.combat.is_some() {
                    self.begin_damage(false);
                } else {
                    self.goto_step(Step::EndCombat);
                }
                Flow::Continue
            }
            Step::End => {
                if self.lose_end_turn == Some(p) {
                    self.lose_end_turn = None;
                    self.player_loses(p, "lost at the end of an extra turn");
                }
                // copies that last "until the next end step" end now; the monarch draws
                let (due, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut self.copy_reverts).into_iter().partition(|r| r.3);
                self.copy_reverts = keep;
                for (id, zcc, def, _) in due.into_iter().rev() {
                    if self.cards[id.0 as usize].zcc == zcc && self.cards[id.0 as usize].zone == Zone::Battlefield {
                        self.cards[id.0 as usize].def = def;
                        self.layers_dirty = true;
                    }
                }
                if self.monarch == Some(p) {
                    self.draw_card(p);
                }
                // statics that depend on the step ("during your end step ...") are judged in the end step
                self.layers_dirty = true;
                self.recompute_layers();
                self.fire_delayed(DelayedWhen::NextEndStep);
                // `Effect::Delayed { at: EndOfTurn }` ("reset at end of turn") fires with the end step's triggers
                self.fire_delayed(DelayedWhen::EndOfTurn);
                self.run = Run::Priority { player: p, passes: 0 };
                Flow::Continue
            }
            Step::Upkeep => {
                // "untapped lands you controlled at the beginning of the turn": counted once the untap step is done (Power Surge)
                let n = self.battlefield.iter().filter(|id| { let c = &self.cards[id.0 as usize]; c.cur.controller == p && !c.tapped && c.cur.types.has(CardType::Land) }).count() as i32;
                self.players[p as usize].lands_untapped_turn_start = n;
                self.fire_delayed(DelayedWhen::NextUpkeep);
                self.granted_suspend_upkeep(p);
                self.meddle_upkeep(p);
                self.run = Run::Priority { player: p, passes: 0 };
                // cards that just finished their granted suspend are cast now, before anyone gets priority
                self.start_next_free_cast();
                Flow::Continue
            }
            Step::Cleanup => {
                self.run = Run::Cleanup;
                Flow::Continue
            }
            Step::EndCombat => {
                self.fire_delayed(DelayedWhen::EndOfCombat);
                self.run = Run::Priority { player: p, passes: 0 };
                Flow::Continue
            }
            Step::Main1 | Step::Main2 | Step::BeginCombat => {
                if step == Step::Main1 {
                    self.fire_delayed(DelayedWhen::NextYourMainPhase);
                    // Archenemy: the archenemy sets a scheme in motion at the start of their precombat main phase
                    if !self.players[p as usize].scheme_deck.is_empty() && !self.scheme_this_turn {
                        self.scheme_this_turn = true;
                        self.archenemy_main_phase();
                    }
                }
                self.run = Run::Priority { player: p, passes: 0 };
                Flow::Continue
            }
        }
    }

    pub(crate) fn fire_delayed(&mut self, when: DelayedWhen) {
        let mut fire = vec![];
        let mut keep = vec![];
        for d in std::mem::take(&mut self.delayed) {
            if (d.when == when && (!matches!(when, DelayedWhen::NextYourMainPhase | DelayedWhen::NextYourDeclareAttackers) || d.controller == self.active) && (when != DelayedWhen::NextTriggerPlayerDrawStep || d.ctx.player == Some(self.active))) || (when == DelayedWhen::NextEndStep && d.when == DelayedWhen::NextYourEndStep && d.controller == self.active) || (when == DelayedWhen::NextEndStep && matches!(d.when, DelayedWhen::NextChosenPlayerEndStep | DelayedWhen::NextCtxPlayerEndStep) && d.ctx.player == Some(self.active)) || (when == DelayedWhen::NextUpkeep && d.when == DelayedWhen::NextYourUpkeep && d.controller == self.active) || (when == DelayedWhen::NextUpkeep && matches!(d.when, DelayedWhen::NextTriggerPlayerUpkeep | DelayedWhen::NextTargetPlayerUpkeep) && d.ctx.player == Some(self.active)) || (when == DelayedWhen::NextUpkeep && d.when == DelayedWhen::NextOwnerUpkeep && self.cards[d.source.0 as usize].owner == self.active) {
                fire.push(d);
            } else {
                keep.push(d);
            }
        }
        self.delayed = keep;
        for d in fire {
            self.push_delayed_item(d);
        }
    }

    /// Put a delayed / temporary trigger's effects on the stack.
    pub(crate) fn push_delayed_item(&mut self, d: DelayedTrigger) {
        let mut d = d;
        // the trigger object of an end-of-combat delayed effect is gone if it changed zones since (it is a new object)
        if d.when == DelayedWhen::EndOfCombat {
            if let (Some(Obj::Card { id }), Some((_, zs))) = (d.ctx.obj, self.delayed_rem_zcc.iter().find(|(i, _)| *i == d.id)) {
                if zs.iter().any(|(c, z)| *c == id.0 && self.cards[id.0 as usize].zcc != *z) {
                    d.ctx.obj = None;
                    d.ctx.lki = None;
                }
            }
        }
        self.next_stack_id += 1;
        let src = d.source;
        let item = StackItem {
            id: StackId(self.next_stack_id),
            controller: d.controller,
            source: src,
            src_zcc: self.cards[src.0 as usize].zcc,
            def: self.cards[src.0 as usize].def,
            kind: ItemKind::Delayed { effects: d.effects },
            targets: d.targets,
            x: 0,
            modes: Default::default(),
            ctx: d.ctx,
            copy: false,
            copy_except: vec![],
            divisions: Default::default(),
            spec_counts: Default::default(),
            granted_snap: None,
            spliced: vec![],
            remembered: {
                let zs = self.delayed_rem_zcc.iter().position(|(i, _)| *i == d.id).map(|k| self.delayed_rem_zcc.remove(k).1);
                match zs {
                    Some(zs) => d.remembered.into_iter().filter(|o| match o {
                        Obj::Card { id } => zs.iter().find(|(c, _)| *c == id.0).map_or(true, |(_, z)| self.cards[id.0 as usize].zcc == *z),
                        _ => true,
                    }).collect(),
                    None => d.remembered,
                }
            },
        };
        self.stack.push(item);
    }

    /// Run state-based actions and put waiting triggers on the stack. Returns Some(flow) if something else must happen first.
    /// State triggers (704.x / 603.8): "when <condition>" triggers once whenever the condition is true and the ability isn't already
    /// waiting or on the stack.
    fn check_state_triggers(&mut self) {
        let db = self.db.clone();
        let mut found = vec![];
        for id in self.battlefield.iter().copied().chain(self.emblem_srcs()) {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities || !self.alive(self.src_ctrl(id)) {
                continue;
            }
            let d = db.def(c.def);
            for &ai in &d.triggers {
                let Ability::Triggered { trigger: Trigger::State { condition }, .. } = &d.abilities[ai as usize] else { continue };
                let waiting = self.pending_triggers.iter().any(|t| t.source == id && t.ability == ai && t.src_zcc == c.zcc)
                    || self.stack.iter().any(|s| s.source == id && s.src_zcc == c.zcc && matches!(s.kind, ItemKind::Triggered { ability, .. } if ability == ai));
                if waiting {
                    continue;
                }
                if self.eval_cond(condition, &crate::eval::EvalCtx::of(self, id)) {
                    found.push(PendingTrigger { controller: self.src_ctrl(id), source: id, src_zcc: c.zcc, def: c.def, ability: ai, ctx: EventCtx::default(), ts: self.ts, granted: None });
                }
            }
        }
        self.pending_triggers.extend(found);
    }

    fn checkpoint(&mut self) -> Option<Flow> {
        loop {
            match self.run_sba() {
                SbaResult::Ask => return Some(Flow::Wait),
                SbaResult::Settled => {}
            }
            if self.is_over() {
                return Some(Flow::Continue);
            }
            self.check_state_triggers();
            if let Some(t) = self.pop_next_trigger() {
                // abilities of permanents that left play still trigger; only skip if the controller lost
                if !self.alive(t.controller) {
                    continue;
                }
                self.begin_trigger(t);
                return Some(Flow::Continue);
            }
            return None;
        }
    }

    /// Does this permanent still have an as-enters choice to make?
    /// The abilities a permanent has as it enters: its own plus any granted by static abilities already in effect.
    pub(crate) fn enter_abilities<'a>(&self, card: CardId, d: &'a crate::Def) -> std::borrow::Cow<'a, [Ability]> {
        let granted = &self.cards[card.0 as usize].cur.granted;
        if granted.is_empty() {
            return std::borrow::Cow::Borrowed(&d.abilities);
        }
        let mut v = d.abilities.clone();
        v.extend(granted.iter().map(|a| (**a).clone()));
        std::borrow::Cow::Owned(v)
    }

    pub(crate) fn enter_pending(&self, card: CardId) -> bool {
        let done = self.cards[card.0 as usize].enter_done;
        let abils = self.enter_abilities(card, self.db.def(self.cards[card.0 as usize].def));
        abils.iter().any(|a| match a {
            Ability::EntersPayLifeOrTapped { .. } => done & 1 == 0,
            Ability::EntersRevealOrTapped { .. } => done & 2 == 0,
            Ability::EntersAsCopy { .. } => done & 4 == 0,
            Ability::TurnedFaceUpMayAttach { .. } => self.cards[card.0 as usize].turnup_choice && done & 1024 == 0,
            Ability::EntersChoose { .. } => done & 8 == 0 || (done & 64 == 0 && abils.iter().filter(|x| matches!(x, Ability::EntersChoose { .. })).count() > 1),
            Ability::EntersMayTapped { .. } => done & 16 == 0,
            Ability::EntersCounterOnChosen { .. } | Ability::EntersLinkChosen { .. } => done & 128 == 0,
            Ability::EntersUnlessCost { .. } => done & 32 == 0,
            Ability::EntersExileXCounters { .. } => done & 256 == 0,
            Ability::EntersSacrificeAll { .. } => done & 512 == 0,
            Ability::EntersExileAny { .. } => done & 2048 == 0,
            _ => false,
        })
    }

    /// Ask the controller of the first permanent waiting for its "as this enters, choose" answer.
    fn ask_enter_choice(&mut self, player: PlayerId, passes: u8) -> Flow {
        while let Some(&card) = self.enter_choices.first() {
            if !self.enter_pending(card) {
                self.enter_choices.remove(0);
                continue;
            }
            let sac_all = self.db.def(self.cards[card.0 as usize].def).abilities.iter().find_map(|a| if let Ability::EntersSacrificeAll { filter } = a { Some(filter.clone()) } else { None });
            if let (Some(filter), true) = (sac_all.filter(|_| self.cards[card.0 as usize].enter_done & 512 == 0), self.cards[card.0 as usize].zone == Zone::Battlefield) {
                self.cards[card.0 as usize].enter_done |= 512;
                let who = self.cards[card.0 as usize].cur.controller;
                let victims: Vec<CardId> = self.battlefield.iter().copied().filter(|id| *id != card && self.matches_card(&filter, *id, card, who) && !self.cards[id.0 as usize].cur.cant_be_sacrificed).collect();
                for id in victims {
                    if self.cards[id.0 as usize].zone == Zone::Battlefield {
                        let owner = self.cards[id.0 as usize].cur.controller;
                        self.emit(Event::Sacrificed { card: id, player: owner });
                        self.move_card(id, Zone::Graveyard, Default::default());
                    }
                }
                continue;
            }
            let c = &self.cards[card.0 as usize];
            let shock = self.db.def(c.def).abilities.iter().find_map(|a| if let Ability::EntersPayLifeOrTapped { life } = a { Some(*life) } else { None });
            if let (Some(life), true) = (shock.filter(|_| c.enter_done & 1 == 0), c.zone == Zone::Battlefield) {
                let who = c.cur.controller;
                if !self.can_pay_life(who, life) {
                    self.cards[card.0 as usize].enter_done |= 1;
                    continue;
                }
                self.pending = Some(Decision {
                    player: who,
                    kind: DecisionKind::YesNo { prompt: format!("Pay {life} life to have {} enter untapped?", self.def_of(card).name), source: Some(card) },
                });
                self.run = Run::EnterPay { card, life, player, passes };
                return Flow::Wait;
            }
            if c.enter_done & 16 == 0 && c.zone == Zone::Battlefield && self.db.def(c.def).abilities.iter().any(|a| matches!(a, Ability::EntersMayTapped { .. })) {
                let who = c.cur.controller;
                self.pending = Some(Decision {
                    player: who,
                    kind: DecisionKind::YesNo { prompt: format!("Have {} enter tapped?", self.def_of(card).name), source: Some(card) },
                });
                self.run = Run::EnterMayTap { card, player, passes };
                return Flow::Wait;
            }
            let counter_on = self.db.def(c.def).abilities.iter().find_map(|a| match a { Ability::EntersCounterOnChosen { filter, kind } => Some((filter.clone(), Some(*kind))), Ability::EntersLinkChosen { filter, .. } => Some((filter.clone(), None)), _ => None });
            if let (Some((filter, kind)), true) = (counter_on.filter(|_| c.enter_done & 128 == 0), c.zone == Zone::Battlefield) {
                let who = c.cur.controller;
                let options: Vec<CardId> = self.battlefield.iter().copied().filter(|id| self.matches_card(&filter, *id, card, who)).collect();
                let optional = self.db.def(c.def).abilities.iter().any(|a| matches!(a, Ability::EntersLinkChosen { optional: true, .. }));
                if options.is_empty() || (options.len() <= 1 && !optional) {
                    if let Some(&t) = options.first() {
                        self.enter_chosen_apply(card, t, kind);
                    }
                    self.cards[card.0 as usize].enter_done |= 128;
                    continue;
                }
                self.pending = Some(Decision {
                    player: who,
                    kind: DecisionKind::Pick {
                        prompt: format!("{}: choose a permanent to put a counter on", self.def_of(card).name),
                        source: Some(card),
                        options: options.iter().map(|id| Opt::Obj { obj: Obj::card(*id) }).collect(),
                        min: if optional { 0 } else { 1 },
                        max: 1,
                        cancellable: false,
                        hidden: false,
                        purpose: PickPurpose::Generic,
                    },
                });
                self.run = Run::EnterCounter { card, options, player, passes };
                return Flow::Wait;
            }
            let exile_x = self.db.def(c.def).abilities.iter().find_map(|a| if let Ability::EntersExileXCounters { filter, .. } = a { Some(filter.clone()) } else { None });
            if let (Some(filter), true) = (exile_x.filter(|_| c.enter_done & 256 == 0), c.zone == Zone::Battlefield) {
                let who = c.cur.controller;
                let x = c.x.max(0) as usize;
                let f = Filter { controller: PlayerFilter::Any, ..filter.into_filter() };
                let options: Vec<CardId> = self.players[who as usize].graveyard.iter().copied().filter(|g| self.matches_card(&f, *g, card, who)).collect();
                if options.len() < x {
                    // can't exile X cards: it goes to the graveyard instead
                    self.cards[card.0 as usize].enter_done |= 256;
                    self.move_card(card, Zone::Graveyard, Default::default());
                    continue;
                }
                if x == 0 {
                    self.cards[card.0 as usize].enter_done |= 256;
                    continue;
                }
                if options.len() == x {
                    for id in &options {
                        self.move_card(*id, Zone::Exile, Default::default());
                    }
                    return self.enter_exile_counters(card, x as u8, player, passes);
                }
                self.pending = Some(Decision {
                    player: who,
                    kind: DecisionKind::Pick {
                        prompt: format!("{}: choose {x} cards from your graveyard to exile", self.def_of(card).name),
                        source: Some(card),
                        options: options.iter().map(|id| Opt::Obj { obj: Obj::card(*id) }).collect(),
                        min: x as u8,
                        max: x as u8,
                        cancellable: false,
                        hidden: false,
                        purpose: PickPurpose::Generic,
                    },
                });
                self.run = Run::EnterExile { card, options, left: x as u8, player, passes };
                return Flow::Wait;
            }
            let exile_any = self.db.def(c.def).abilities.iter().find_map(|a| if let Ability::EntersExileAny { filter } = a { Some(filter.clone()) } else { None });
            if let (Some(filter), true) = (exile_any.filter(|_| c.enter_done & 2048 == 0), c.zone == Zone::Battlefield) {
                let who = c.cur.controller;
                let f = Filter { controller: PlayerFilter::Any, ..filter.into_filter() };
                let options: Vec<CardId> = self.players[who as usize].graveyard.iter().copied().filter(|g| self.matches_card(&f, *g, card, who)).collect();
                if options.is_empty() {
                    self.cards[card.0 as usize].enter_done |= 2048;
                    continue;
                }
                self.pending = Some(Decision {
                    player: who,
                    kind: DecisionKind::Pick {
                        prompt: format!("{}: choose any number of cards from your graveyard to exile", self.def_of(card).name),
                        source: Some(card),
                        options: options.iter().map(|id| Opt::Obj { obj: Obj::card(*id) }).collect(),
                        min: 0,
                        max: options.len() as u8,
                        cancellable: false,
                        hidden: false,
                        purpose: PickPurpose::Generic,
                    },
                });
                self.run = Run::EnterExileAny { card, options, player, passes };
                return Flow::Wait;
            }
            let unless = self.db.def(c.def).abilities.iter().find_map(|a| if let Ability::EntersUnlessCost { cost } = a { Some(cost.clone()) } else { None });
            if let (Some(cost), true) = (unless.filter(|_| c.enter_done & 32 == 0), c.zone == Zone::Battlefield) {
                let who = c.cur.controller;
                if !self.can_pay_costs(who, card, &cost) {
                    self.cards[card.0 as usize].enter_done |= 32;
                    self.move_card(card, Zone::Graveyard, Default::default());
                    continue;
                }
                self.pending = Some(Decision {
                    player: who,
                    kind: DecisionKind::YesNo { prompt: format!("Pay the cost to have {} enter the battlefield (otherwise it goes to the graveyard)?", self.def_of(card).name), source: Some(card) },
                });
                self.run = Run::EnterUnless { card, player, passes };
                return Flow::Wait;
            }
            let attach_to = self.db.def(c.def).abilities.iter().find_map(|a| if let Ability::TurnedFaceUpMayAttach { to } = a { Some(to.clone()) } else { None });
            if let (Some(filter), true) = (attach_to.filter(|_| c.enter_done & 1024 == 0 && c.turnup_choice), c.zone == Zone::Battlefield) {
                let who = c.cur.controller;
                self.cards[card.0 as usize].enter_done |= 1024;
                let options: Vec<CardId> = self.battlefield.iter().copied().filter(|id| *id != card && self.matches_card(&filter, *id, card, who)).collect();
                if options.is_empty() {
                    continue;
                }
                self.pending = Some(Decision {
                    player: who,
                    kind: DecisionKind::Pick {
                        prompt: format!("{}: you may attach it to a creature", self.def_of(card).name),
                        source: Some(card),
                        options: options.iter().map(|id| Opt::Obj { obj: Obj::card(*id) }).collect(),
                        min: 0,
                        max: 1,
                        cancellable: false,
                        hidden: false,
                        purpose: PickPurpose::Generic,
                    },
                });
                self.run = Run::EnterAttach { card, options, player, passes };
                return Flow::Wait;
            }
            let as_copy = self.db.def(c.def).abilities.iter().find_map(|a| if let Ability::EntersAsCopy { filter, zone, condition, .. } = a { Some((filter.clone(), *zone, condition.clone())) } else { None });
            let as_copy = as_copy.map(|(f, z, cd)| (f, z, cd.map_or(false, |cd| !self.eval_cond(&cd, &crate::eval::EvalCtx::with_controller(card, c.cur.controller)))));
            if let (Some((filter, copy_zone, gated)), true) = (as_copy.filter(|_| c.enter_done & 4 == 0), c.zone == Zone::Battlefield) {
                let who = c.cur.controller;
                let pool: Vec<CardId> = if copy_zone == Zone::Graveyard {
                    self.players.iter().flat_map(|p| p.graveyard.iter().copied()).collect()
                } else if copy_zone == Zone::Exile {
                    self.players.iter().flat_map(|p| p.exile.iter().copied()).collect()
                } else {
                    self.battlefield.clone()
                };
                let options: Vec<CardId> = pool.into_iter().filter(|id| *id != card && self.matches_card(&if copy_zone == Zone::Graveyard || copy_zone == Zone::Exile { Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() } } else { filter.clone().into_filter() }, *id, card, who)).collect();
                let two = self.db.def(c.def).abilities.iter().any(|a| matches!(a, Ability::EntersAsCopy { exile_two_power_counters: true, .. }));
                if gated || options.is_empty() || (two && options.len() < 2) {
                    self.cards[card.0 as usize].enter_done |= 4;
                    continue;
                }
                self.pending = Some(Decision {
                    player: who,
                    kind: DecisionKind::Pick {
                        prompt: if two { format!("{}: choose two creature cards to exile (the one to copy first), or none", self.def_of(card).name) } else { format!("{}: choose a permanent to copy (or none)", self.def_of(card).name) },
                        source: Some(card),
                        options: options.iter().map(|id| Opt::Obj { obj: Obj::card(*id) }).collect(),
                        min: 0,
                        max: if two { 2 } else { 1 },
                        cancellable: false,
                        hidden: false,
                        purpose: PickPurpose::Generic,
                    },
                });
                self.run = Run::EnterCopy { card, options, player, passes };
                return Flow::Wait;
            }
            let reveal = self.db.def(c.def).abilities.iter().find_map(|a| if let Ability::EntersRevealOrTapped { filter, .. } = a { Some(filter.clone()) } else { None });
            let or_control = self.db.def(c.def).abilities.iter().any(|a| matches!(a, Ability::EntersRevealOrTapped { or_control: true, .. }));
            if let (Some(filter), true) = (reveal.clone().filter(|_| c.enter_done & 2 == 0), c.zone == Zone::Battlefield) {
                let who = c.cur.controller;
                if or_control && self.battlefield.iter().any(|b| *b != card && self.cards[b.0 as usize].cur.controller == who && self.matches_card(&Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() }, *b, card, who)) {
                    self.cards[card.0 as usize].tapped = false;
                    self.cards[card.0 as usize].enter_done |= 2;
                    continue;
                }
                let has = self.players[who as usize].hand.iter().any(|h| self.matches_card(&filter, *h, card, who));
                if !has {
                    self.cards[card.0 as usize].enter_done |= 2;
                    continue;
                }
                self.pending = Some(Decision {
                    player: who,
                    kind: DecisionKind::YesNo { prompt: format!("Reveal a card from your hand to have {} enter untapped?", self.def_of(card).name), source: Some(card) },
                });
                self.run = Run::EnterReveal { card, player, passes };
                return Flow::Wait;
            }
            // a second `EntersChoose` ("choose a color and a creature type": Volrath's Laboratory) is asked after the first (done bit 64)
            let nth_choose = if c.enter_done & 8 == 0 { 0 } else { 1 };
            let kind_ex = self.enter_abilities(card, self.db.def(c.def)).iter().filter_map(|a| if let Ability::EntersChoose { kind, except, cap } = a { Some((*kind, except.clone(), cap.clone())) } else { None }).nth(nth_choose);
            let kind = kind_ex.as_ref().map(|k| k.0).filter(|_| c.enter_done & if nth_choose == 0 { 8 } else { 64 } == 0);
            let cap_amount = kind_ex.as_ref().and_then(|k| k.2.clone());
            let except: Vec<Color> = kind_ex.map(|k| k.1).unwrap_or_default();
            let (Some(kind), true) = (kind, c.zone == Zone::Battlefield) else {
                if c.zone != Zone::Battlefield {
                    self.enter_choices.remove(0);
                } else {
                    self.cards[card.0 as usize].enter_done |= 8 | 64;
                }
                continue;
            };
            let mut who = c.cur.controller;
            if kind == ChooseKind::WarPeace {
                // the controller chooses first, then each other player in turn order
                let mut p = who;
                let mut found = None;
                for _ in 0..self.players.len() {
                    if self.alive(p) && !c.player_pick.iter().any(|(q, _)| *q == p) {
                        found = Some(p);
                        break;
                    }
                    p = self.next_player(p);
                }
                match found {
                    Some(p) => who = p,
                    None => {
                        self.cards[card.0 as usize].enter_done |= 8;
                        continue;
                    }
                }
            }
            if kind == ChooseKind::RandomTwoToFour {
                let n: i32 = rand::Rng::gen_range(&mut self.rng, 2..=4);
                self.cards[card.0 as usize].chosen_number = Some(n);
                self.cards[card.0 as usize].enter_done |= if nth_choose == 0 { 8 } else { 64 };
                self.layers_dirty = true;
                self.recompute_layers();
                self.apply_enter_counters(card);
                continue;
            }
            let (prompt, options): (String, Vec<String>) = match kind {
                ChooseKind::PayLife => {
                    let cap = cap_amount.as_ref().map_or(0, |a| self.eval_amount(a, &crate::eval::EvalCtx::with_controller(card, who))).min(self.players[who as usize].life).max(0);
                    ("Pay any amount of life".into(), (0..=cap).map(|n| n.to_string()).collect())
                }
                ChooseKind::Color => ("Choose a color".into(), Color::ALL.iter().filter(|c| !except.contains(c)).map(|c| format!("{c:?}")).collect()),
                ChooseKind::SecondColor => ("Choose a second color".into(), Color::ALL.iter().filter(|col| Some(**col) != c.chosen_color).map(|c| format!("{c:?}")).collect()),
                ChooseKind::CardType => ("Choose a card type".into(), ["Artifact", "Battle", "Creature", "Enchantment", "Instant", "Kindred", "Land", "Planeswalker", "Sorcery"].iter().map(|s| s.to_string()).collect()),
                ChooseKind::SharedCraftType => {
                    let z = c.zcc;
                    let mut opts = vec![];
                    for (nm, ty) in [("Artifact", CardType::Artifact), ("Battle", CardType::Battle), ("Creature", CardType::Creature), ("Enchantment", CardType::Enchantment), ("Instant", CardType::Instant), ("Land", CardType::Land), ("Planeswalker", CardType::Planeswalker), ("Sorcery", CardType::Sorcery)] {
                        let n = self.cards.iter().enumerate().filter(|(i, x)| CardId(*i as u32) != card && x.zone == Zone::Exile && x.exiled_with.map_or(false, |(s, zz)| s == card && zz == z) && x.cur.types.has(ty)).count();
                        if n >= 2 {
                            opts.push(nm.to_string());
                        }
                    }
                    ("Choose a card type shared among two exiled cards".into(), opts)
                }
                ChooseKind::SagaChapter => ("Read ahead: choose a chapter number".into(), (1..=self.db.def(self.cards[card.0 as usize].def).saga_max.max(1)).map(|n| n.to_string()).collect()),
                ChooseKind::OddEven => ("Choose odd or even".into(), vec!["Odd".into(), "Even".into()]),
                ChooseKind::RandomTwoToFour => (String::new(), vec![]),
                ChooseKind::LeftRight => ("Choose left or right".into(), vec!["Left".into(), "Right".into()]),
                ChooseKind::WarPeace => ("Choose war or peace".into(), vec!["War".into(), "Peace".into()]),
                ChooseKind::MarduJeskai => ("Choose Mardu or Jeskai".into(), vec!["Mardu".into(), "Jeskai".into()]),
                ChooseKind::KhansDragons => ("Choose Khans or Dragons".into(), vec!["Khans".into(), "Dragons".into()]),
                ChooseKind::EclipsedTypes => ("Choose a creature type".into(), ["Elemental", "Elf", "Faerie", "Giant", "Goblin", "Kithkin", "Merfolk", "Treefolk"].iter().map(|s| s.to_string()).collect()),
                ChooseKind::FiveOneOrOneFive => ("Choose 5/1 or 1/5".into(), vec!["5/1".into(), "1/5".into()]),
                ChooseKind::Opponent => ("Choose an opponent".into(), self.opponents(who).iter().map(|p| format!("Player {}", p + 1)).collect()),
                ChooseKind::SecondPlayer => ("Choose a second player".into(), self.alive_players().iter().filter(|p| Some(**p) != c.chosen_player).map(|p| format!("Player {}", p + 1)).collect()),
                ChooseKind::Player => ("Choose a player".into(), self.alive_players().iter().map(|p| format!("Player {}", p + 1)).collect()),
                ChooseKind::ExileSpellFromGraveyard => ("Exile an instant or sorcery card from your graveyard".into(), self.players[who as usize].graveyard.iter().filter(|g| self.cards[g.0 as usize].cur.types.has(CardType::Instant) || self.cards[g.0 as usize].cur.types.has(CardType::Sorcery)).map(|g| g.0.to_string()).collect()),
                ChooseKind::LandType => ("Choose a land type".into(), ["Cave", "Desert", "Forest", "Gate", "Island", "Lair", "Locus", "Mine", "Mountain", "Plains", "Power-Plant", "Sphere", "Swamp", "Tower", "Town", "Urza's"].iter().map(|s| s.to_string()).collect()),
                ChooseKind::SecondBasicLandType => ("Choose a second basic land type".into(), ["Plains", "Island", "Swamp", "Mountain", "Forest"].iter().filter(|s| c.chosen_type != Some(Sym::new(s))).map(|s| s.to_string()).collect()),
                ChooseKind::IslandOrSwamp => ("Choose Island or Swamp".into(), vec!["Island".into(), "Swamp".into()]),
                ChooseKind::BasicLandType => ("Choose a basic land type".into(), ["Plains", "Island", "Swamp", "Mountain", "Forest"].iter().map(|s| s.to_string()).collect()),
                ChooseKind::PlaneswalkerType => {
                    let mut v: Vec<String> = vec![];
                    for d in &self.db.defs {
                        if d.types.has(CardType::Planeswalker) {
                            for s in &d.subtypes {
                                let n = s.name();
                                if !v.contains(&n) {
                                    v.push(n);
                                }
                            }
                        }
                    }
                    v.sort();
                    ("Choose a planeswalker type".into(), v)
                }
                ChooseKind::CreatureType => {
                    let mut v: Vec<String> = vec![];
                    for d in &self.db.defs {
                        if d.types.has(CardType::Creature) || d.types.has(CardType::Kindred) {
                            for s in &d.subtypes {
                                let n = s.name();
                                if !v.contains(&n) {
                                    v.push(n);
                                }
                            }
                        }
                    }
                    v.sort();
                    ("Choose a creature type".into(), v)
                }
            };
            if options.is_empty() {
                self.cards[card.0 as usize].enter_done |= if nth_choose == 0 { 8 } else { 64 };
                continue;
            }
            if kind == ChooseKind::PayLife && options.len() == 1 {
                // nothing can be paid
                self.cards[card.0 as usize].chosen_number = Some(0);
                self.cards[card.0 as usize].enter_done |= if nth_choose == 0 { 8 } else { 64 };
                self.layers_dirty = true;
                self.recompute_layers();
                self.apply_enter_counters(card);
                continue;
            }
            if kind == ChooseKind::Opponent && options.len() == 1 {
                self.cards[card.0 as usize].chosen_player = self.opponents(who).first().copied();
                self.cards[card.0 as usize].enter_done |= if nth_choose == 0 { 8 } else { 64 };
                self.layers_dirty = true;
                self.recompute_layers();
                self.apply_enter_counters(card);
                continue;
            }
            self.pending = Some(Decision {
                player: who,
                kind: DecisionKind::Pick {
                    prompt: format!("{}: {}", self.def_of(card).name, prompt),
                    source: Some(card),
                    options: if kind == ChooseKind::ExileSpellFromGraveyard { options.iter().filter_map(|n| n.parse::<u32>().ok()).map(|n| Opt::Obj { obj: Obj::card(CardId(n)) }).collect() } else if kind == ChooseKind::Color { Color::ALL.iter().filter(|c| !except.contains(c)).map(|c| Opt::Color { c: *c }).collect() } else if kind == ChooseKind::SecondColor { options.iter().filter_map(|n| Color::ALL.iter().copied().find(|c| format!("{c:?}") == *n)).map(|c| Opt::Color { c }).collect() } else { options.iter().map(|t| Opt::Text { text: t.clone() }).collect() },
                    min: 1,
                    max: 1,
                    cancellable: false,
                    hidden: false,
                    purpose: PickPurpose::Generic,
                },
            });
            self.run = Run::EnterChoice { card, kind, options, player, passes };
            return Flow::Wait;
        }
        self.run = Run::Priority { player, passes };
        Flow::Continue
    }

    pub(crate) fn step_enter_attach(&mut self, card: CardId, options: Vec<CardId>, player: PlayerId, passes: u8) -> Flow {
        if let Some(Choice::Pick { indices }) = self.answer.take() {
            if let Some(&to) = indices.first().and_then(|i| options.get(*i)) {
                if self.cards[card.0 as usize].zone == Zone::Battlefield && self.cards[to.0 as usize].zone == Zone::Battlefield {
                    self.attach(card, Some(to));
                    self.recompute_layers();
                }
            }
        }
        self.cards[card.0 as usize].turnup_choice = false;
        if !self.enter_pending(card) {
            if let Some(p) = self.enter_choices.iter().position(|c| *c == card) {
                self.enter_choices.remove(p);
            }
        }
        self.ask_enter_choice(player, passes)
    }

    pub(crate) fn step_enter_copy(&mut self, card: CardId, options: Vec<CardId>, player: PlayerId, passes: u8) -> Flow {
        let answer = self.answer.take();
        self.apply_enter_copy(card, &options, answer);
        self.ask_enter_choice(player, passes)
    }

    /// The decision a permanent entering as a copy asks of its controller, when it has to ask one now (inside a resolving effect).
    pub(crate) fn enter_copy_decision(&self, card: CardId) -> Option<(Decision, Vec<CardId>)> {
        let c = &self.cards[card.0 as usize];
        if c.zone != Zone::Battlefield || c.enter_done & 4 != 0 {
            return None;
        }
        let (filter, copy_zone, condition) = self.db.def(c.def).abilities.iter().find_map(|a| if let Ability::EntersAsCopy { filter, zone, condition, .. } = a { Some((filter.clone(), *zone, condition.clone())) } else { None })?;
        let who = c.cur.controller;
        if condition.map_or(false, |cd| !self.eval_cond(&cd, &crate::eval::EvalCtx::with_controller(card, who))) {
            return None;
        }
        if self.db.def(c.def).abilities.iter().any(|a| matches!(a, Ability::EntersAsCopy { exile_two_power_counters: true, .. })) || copy_zone != Zone::Battlefield {
            return None;
        }
        let options: Vec<CardId> = self.battlefield.iter().copied().filter(|id| *id != card && self.matches_card(&filter.clone().into_filter(), *id, card, who)).collect();
        if options.is_empty() {
            return None;
        }
        let d = Decision {
            player: who,
            kind: DecisionKind::Pick {
                prompt: format!("{}: choose a permanent to copy (or none)", self.def_of(card).name),
                source: Some(card),
                options: options.iter().map(|id| Opt::Obj { obj: Obj::card(*id) }).collect(),
                min: 0,
                max: 1,
                cancellable: false,
                hidden: false,
                purpose: PickPurpose::Generic,
            },
        };
        Some((d, options))
    }

    /// Makes the permanent a copy of the chosen one (or leaves it as it is), as the answer says.
    pub(crate) fn apply_enter_copy(&mut self, card: CardId, options: &[CardId], answer: Option<Choice>) {
        if let Some(Choice::Pick { indices }) = answer {
            let two = self.db.def(self.cards[card.0 as usize].def).abilities.iter().any(|a| matches!(a, Ability::EntersAsCopy { exile_two_power_counters: true, .. }));
            // The Mimeoplasm: the second card chosen is exiled too and its power becomes counters
            let donor: Option<CardId> = if two { if indices.len() == 2 { options.get(indices[1]).copied() } else { None } } else { None };
            if let Some(&from) = indices.first().filter(|_| !two || donor.is_some()).and_then(|i| options.get(*i)) {
                let donor_power = donor.map(|d| self.cards[d.0 as usize].cur.power.max(self.db.def(self.cards[d.0 as usize].def).power.unwrap_or(0)).max(0)).unwrap_or(0);
                if let Some(d) = donor {
                    if self.cards[d.0 as usize].zone == Zone::Graveyard {
                        self.move_card(d, Zone::Exile, Default::default());
                    }
                }
                let (except, exile_copied, enter_tapped, until_eot, link_copied, haste_eot) = self.db.def(self.cards[card.0 as usize].def).abilities.iter().find_map(|a| if let Ability::EntersAsCopy { except, exile_copied, tapped, until_end_of_turn, tap_and_link_copied, haste_until_eot, .. } = a { Some((except.clone(), *exile_copied, *tapped, *until_end_of_turn, *tap_and_link_copied, *haste_until_eot)) } else { None }).unwrap_or_default();
                let until_fd = self.db.def(self.cards[card.0 as usize].def).abilities.iter().any(|a| matches!(a, Ability::EntersAsCopy { until_turned_face_down: true, .. }));
                let effects_before = self.effects.len();
                let old_def = self.cards[card.0 as usize].def;
                if enter_tapped {
                    self.cards[card.0 as usize].tapped = true;
                }
                let who = self.cards[card.0 as usize].cur.controller;
                // "its power and toughness are equal to this creature's": the printed (or prototype) size, taken before the copy replaces the card
                let except: Vec<CopyExcept> = if except.iter().any(|x| matches!(x, CopyExcept::SetPTOwn)) {
                    let (mut p, mut t) = { let d = self.db.def(self.cards[card.0 as usize].def); (d.power.unwrap_or(0), d.toughness.unwrap_or(0)) };
                    if let Some(alt) = self.cards[card.0 as usize].cast_alt {
                        let mut n = 0u8;
                        for a in &self.db.def(self.cards[card.0 as usize].def).abilities {
                            match a {
                                Ability::AlternativeCost { .. } => n += 1,
                                Ability::Prototype { power, toughness, .. } if n > 0 && alt == n - 1 => { p = *power; t = *toughness; }
                                _ => {}
                            }
                        }
                    }
                    except.into_iter().map(|x| if matches!(x, CopyExcept::SetPTOwn) { CopyExcept::SetPT { power: p, toughness: t } } else { x }).collect()
                } else {
                    except
                };
                let except: Vec<CopyExcept> = except
                    .into_iter()
                    .flat_map(|x| match x {
                        CopyExcept::AddCountersIfControlled { kind, n } => (self.cards[from.0 as usize].cur.controller == who).then_some(CopyExcept::AddCounters { kind, n, per_x: false, if_creature: false, if_planeswalker: false }).into_iter().collect::<Vec<_>>(),
                        CopyExcept::SameCounters => self.cards[from.0 as usize].counters.iter().map(|(kind, n)| CopyExcept::AddCounters { kind: *kind, n: *n as i32, per_x: false, if_creature: false, if_planeswalker: false }).collect::<Vec<_>>(),
                        x => vec![x],
                    })
                    .collect();
                if until_eot {
                    let (zcc, old) = (self.cards[card.0 as usize].zcc, self.cards[card.0 as usize].def);
                    self.copy_reverts.push((card, zcc, old, false));
                }
                if link_copied && self.cards[from.0 as usize].zone == Zone::Battlefield {
                    self.tap_card(from);
                    self.cards[card.0 as usize].linked = Some((from, self.cards[from.0 as usize].zcc));
                }
                self.cards[card.0 as usize].def = self.cards[from.0 as usize].def;
                if exile_copied && self.cards[from.0 as usize].zone == Zone::Graveyard {
                    self.move_card(from, Zone::Exile, Default::default());
                }
                // a copy of a planeswalker enters with that planeswalker's printed loyalty
                if let (Some(l), true) = (self.db.def(self.cards[card.0 as usize].def).loyalty, self.db.def(self.cards[card.0 as usize].def).types.has(CardType::Planeswalker)) {
                    if self.cards[card.0 as usize].counter(CounterKind::Loyalty) == 0 {
                        self.cards[card.0 as usize].add_counters(CounterKind::Loyalty, l);
                    }
                }
                self.apply_copy_except(card, who, card, &except, &[]);
                if until_fd {
                    self.cards[card.0 as usize].fd_revert = Some(old_def);
                    for e in self.effects.iter_mut().skip(effects_before) {
                        e.duration = Duration::UntilAffectedTurnedFaceDown;
                    }
                }
                if haste_eot {
                    self.add_effect(card, who, &[card], Modif::Keyword(Keyword::Haste), Duration::UntilEndOfTurn);
                }
                if donor_power > 0 {
                    self.cards[card.0 as usize].add_counters(CounterKind::PlusOnePlusOne, donor_power);
                }
                self.layers_dirty = true;
                // the copied creature's own enters-the-battlefield triggers happen now
                self.recompute_layers();
                // counters the copied permanent enters with (Ravaging Riftwurm's time counters) are put on the copy
                if self.cards[card.0 as usize].counters.iter().all(|(_, n)| *n == 0) && self.db.def(self.cards[card.0 as usize].def).abilities.iter().any(|a| matches!(a, Ability::EntersWithCounters { .. })) {
                    self.apply_enter_counters(card);
                }
                if !self.cards[card.0 as usize].turnup_choice && !self.copy_deferred.iter().any(|(c, _)| *c == card) {
                    crate::triggers::on_event_for(self, &Event::Moved { card, from: Zone::Stack, to: Zone::Battlefield }, Some(card));
                }
            }
        }
        self.cards[card.0 as usize].turnup_choice = false;
        self.cards[card.0 as usize].enter_done |= 4;
        // every trigger that was waiting for the copy choice (its own and other permanents') now sees the permanent as what it is
        if let Some(i) = self.copy_deferred.iter().position(|(c, _)| *c == card) {
            let (_, from) = self.copy_deferred.remove(i);
            crate::triggers::on_event(self, &Event::Moved { card, from, to: Zone::Battlefield });
        }
        if !self.enter_pending(card) {
            if let Some(p) = self.enter_choices.iter().position(|c| *c == card) {
                self.enter_choices.remove(p);
            }
        }
    }

    /// Untap step: ask whether to keep each optional-untap permanent tapped.
    pub(crate) fn step_untap_choose(&mut self, mut queue: Vec<CardId>, asked: bool) -> Flow {
        if asked && !queue.is_empty() {
            let id = queue.remove(0);
            if !matches!(self.answer.take(), Some(Choice::YesNo { yes: true })) && self.cards[id.0 as usize].zone == Zone::Battlefield {
                self.untap_card(id);
            }
        }
        if let Some(&id) = queue.first() {
            let who = self.cards[id.0 as usize].cur.controller;
            self.pending = Some(Decision {
                player: who,
                kind: DecisionKind::YesNo { prompt: format!("Keep {} tapped (don't untap it)?", self.def_of(id).name), source: Some(id) },
            });
            self.run = Run::UntapChoose { queue, asked: true };
            return Flow::Wait;
        }
        self.recompute_layers();
        self.goto_step(Step::Upkeep);
        Flow::Continue
    }

    /// A permanent that transformed into a Curse face (`Ability::AttachPlayerOnTransform`) asks its controller which player it enchants.
    /// Returns None when nothing is waiting; otherwise the decision is pending (or the only possible player was attached) and the flow to return.
    fn ask_attach_player(&mut self, resume: Option<(PlayerId, u8)>) -> Option<Flow> {
        while let Some(&card) = self.attach_pending.first() {
            self.attach_pending.remove(0);
            let c = &self.cards[card.0 as usize];
            if c.zone != Zone::Battlefield || c.enchanted_player.is_some() || !self.def_of(card).abilities.iter().any(|a| matches!(a, Ability::AttachPlayerOnTransform)) {
                continue;
            }
            let who = c.cur.controller;
            let mut options: Vec<PlayerId> = self.opponents(who).into_iter().filter(|p| self.alive(*p)).collect();
            if self.alive(who) {
                options.push(who);
            }
            if options.is_empty() {
                continue;
            }
            if options.len() == 1 {
                self.cards[card.0 as usize].enchanted_player = Some(options[0]);
                self.layers_dirty = true;
                continue;
            }
            self.pending = Some(Decision {
                player: who,
                kind: DecisionKind::Pick {
                    prompt: format!("Attach {} to a player", self.def_of(card).name),
                    source: Some(card),
                    options: options.iter().map(|p| Opt::Obj { obj: Obj::player(*p) }).collect(),
                    min: 1,
                    max: 1,
                    cancellable: false,
                    hidden: false,
                    purpose: PickPurpose::Generic,
                },
            });
            self.run = Run::AttachPlayerAsk { card, options, resume };
            return Some(Flow::Wait);
        }
        None
    }

    pub(crate) fn step_attach_player_ask(&mut self, card: CardId, options: Vec<PlayerId>, resume: Option<(PlayerId, u8)>) -> Flow {
        let idx = match self.answer.take() {
            Some(Choice::Pick { indices }) => indices.first().copied().unwrap_or(0),
            _ => 0,
        };
        if self.cards[card.0 as usize].zone == Zone::Battlefield {
            self.cards[card.0 as usize].enchanted_player = options.get(idx).or(options.first()).copied();
            self.layers_dirty = true;
            self.recompute_layers();
        }
        self.run = match resume {
            Some((player, passes)) => Run::Priority { player, passes },
            None => Run::BeginStep,
        };
        Flow::Continue
    }

    /// Does this mana ability say "a player of your choice adds ..." (`AddMana.player_choice`)?
    pub fn mana_gift_ability(&self, p: PlayerId, card: CardId, ability: u16) -> bool {
        matches!(self.ability_owned(card, ability), Some(Ability::Activated { effects, .. }) if effects.iter().any(|e| matches!(e, Effect::AddMana { player_choice: true, .. })))
            && self.cards[card.0 as usize].cur.controller == p
    }

    pub(crate) fn step_mana_gift_ask(&mut self, card: CardId, ability: u16, produce: Option<crate::mana::ManaType>, options: Vec<PlayerId>, player: PlayerId, passes: u8) -> Flow {
        let idx = match self.answer.take() {
            Some(Choice::Pick { indices }) => indices.first().copied().unwrap_or(0),
            _ => 0,
        };
        self.mana_gift = options.get(idx).or(options.first()).copied();
        let _ = self.activate_mana_ability(player, card, ability, produce);
        self.mana_gift = None;
        self.run = Run::Priority { player, passes };
        Flow::Continue
    }

    pub(crate) fn step_mana_pick_ask(&mut self, card: CardId, ability: u16, produce: Option<crate::mana::ManaType>, options: Vec<CardId>, player: PlayerId, passes: u8) -> Flow {
        if let Some(Choice::Pick { indices }) = self.answer.take() {
            self.mana_picks = indices.iter().filter_map(|i| options.get(*i).copied()).collect();
        }
        let _ = self.activate_mana_ability(player, card, ability, produce);
        self.mana_picks.clear();
        self.run = Run::Priority { player, passes };
        Flow::Continue
    }

    /// Untap-count limits (Winter Orb style) that apply to `p`'s untap step, from battlefield permanents and emblems.
    pub(crate) fn untap_limits(&self, p: PlayerId) -> Vec<(Filter, u8, CardId, PlayerId, u32)> {
        let mut limits: Vec<(Filter, u8, CardId, PlayerId, u32)> = vec![];
        let emblems: Vec<CardId> = self.players.iter().flat_map(|pl| pl.command.iter().copied()).filter(|id| self.db.def(self.cards[id.0 as usize].def).token).collect();
        for &sid in self.battlefield.iter().chain(emblems.iter()) {
            let sc = &self.cards[sid.0 as usize];
            if sc.cur.no_abilities {
                continue;
            }
            let sd = self.db.def(sc.def);
            for &i in &sd.statics {
                if let Ability::Static { effect: StaticEffect::UntapLimit { who, filter, max }, condition, .. } = &sd.abilities[i as usize] {
                    if self.player_filter_src(sid, *who, p, sc.cur.controller) && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &crate::eval::EvalCtx::with_controller(sid, sc.cur.controller))) {
                        limits.push((filter.clone().into_filter(), *max, sid, sc.cur.controller, 0));
                    }
                }
            }
        }
        limits
    }

    pub(crate) fn step_untap_limit_pick(&mut self, ids: Vec<CardId>) -> Flow {
        if let Some(Choice::Pick { indices }) = self.answer.take() {
            self.untap_prefer = Some(indices.iter().filter_map(|&i| ids.get(i).copied()).collect());
        }
        self.untap_pick_done = true;
        self.untap_limit_done = true;
        self.run = Run::BeginStep;
        Flow::Continue
    }

    pub(crate) fn step_untap_pick_type(&mut self, options: Vec<CardType>) -> Flow {
        let idx = match self.answer.take() {
            Some(Choice::Pick { indices }) => indices.first().copied().unwrap_or(0),
            _ => 0,
        };
        self.untap_only = options.get(idx).or(options.first()).copied();
        self.untap_pick_done = true;
        self.run = Run::BeginStep;
        Flow::Continue
    }

    pub(crate) fn step_draw_skip_ask(&mut self, gain: i32) -> Flow {
        let p = self.active;
        if let Some(Choice::YesNo { yes: true }) = self.answer.take() {
            self.gain_life(p, gain);
            self.goto_after_draw();
        } else {
            self.draw_skip_declined = true;
            self.run = Run::BeginStep;
        }
        Flow::Continue
    }

    pub(crate) fn step_enter_reveal(&mut self, card: CardId, player: PlayerId, passes: u8) -> Flow {
        if let Some(Choice::YesNo { yes: true }) = self.answer.take() {
            self.cards[card.0 as usize].tapped = false;
        }
        self.cards[card.0 as usize].enter_done |= 2;
        if !self.enter_pending(card) {
            if let Some(p) = self.enter_choices.iter().position(|c| *c == card) {
                self.enter_choices.remove(p);
            }
        }
        self.ask_enter_choice(player, passes)
    }

    pub(crate) fn step_enter_may_tap(&mut self, card: CardId, player: PlayerId, passes: u8) -> Flow {
        if let Some(Choice::YesNo { yes: true }) = self.answer.take() {
            let who = self.cards[card.0 as usize].cur.controller;
            self.cards[card.0 as usize].tapped = true;
            let then: Vec<Effect> = self.db.def(self.cards[card.0 as usize].def).abilities.iter().find_map(|a| if let Ability::EntersMayTapped { then } = a { Some(then.clone()) } else { None }).unwrap_or_default();
            for e in &then {
                self.run_effect_now(card, who, e);
            }
        }
        self.cards[card.0 as usize].enter_done |= 16;
        if !self.enter_pending(card) {
            if let Some(p) = self.enter_choices.iter().position(|c| *c == card) {
                self.enter_choices.remove(p);
            }
        }
        self.ask_enter_choice(player, passes)
    }

    /// The chosen permanent of an as-enters choice gets a counter, or (no kind) is linked to the entering permanent.
    fn enter_chosen_apply(&mut self, card: CardId, t: CardId, kind: Option<CounterKind>) {
        match kind {
            Some(k) => self.add_counters_to(Obj::card(t), k, 1),
            None => {
                let l = (t, self.cards[t.0 as usize].zcc);
                self.cards[card.0 as usize].linked = Some(l);
            }
        }
    }

    pub(crate) fn step_enter_counter(&mut self, card: CardId, options: Vec<CardId>, player: PlayerId, passes: u8) -> Flow {
        let kind = self.db.def(self.cards[card.0 as usize].def).abilities.iter().find_map(|a| match a { Ability::EntersCounterOnChosen { kind, .. } => Some(Some(*kind)), Ability::EntersLinkChosen { .. } => Some(None), _ => None });
        let optional = self.db.def(self.cards[card.0 as usize].def).abilities.iter().any(|a| matches!(a, Ability::EntersLinkChosen { optional: true, .. }));
        let pick = match self.answer.take() {
            Some(Choice::Pick { indices }) => indices.first().and_then(|i| options.get(*i)).copied(),
            _ => None,
        };
        let pick = if optional { pick } else { pick.or(options.first().copied()) };
        if let (Some(kind), Some(t)) = (kind, pick) {
            self.enter_chosen_apply(card, t, kind);
        }
        self.cards[card.0 as usize].enter_done |= 128;
        if !self.enter_pending(card) {
            if let Some(p) = self.enter_choices.iter().position(|c| *c == card) {
                self.enter_choices.remove(p);
            }
        }
        self.ask_enter_choice(player, passes)
    }

    /// `EntersExileXCounters`: pick the counter kind for each of the `left` cards exiled (automatic with one kind), then finish the as-enters choice.
    fn enter_exile_counters(&mut self, card: CardId, left: u8, player: PlayerId, passes: u8) -> Flow {
        let kinds: Vec<CounterKind> = self.db.def(self.cards[card.0 as usize].def).abilities.iter().find_map(|a| if let Ability::EntersExileXCounters { kinds, .. } = a { Some(kinds.clone()) } else { None }).unwrap_or_default();
        if left > 0 && kinds.len() > 1 {
            let who = self.cards[card.0 as usize].cur.controller;
            self.pending = Some(Decision {
                player: who,
                kind: DecisionKind::Pick {
                    prompt: format!("{}: choose a counter for an exiled card ({left} left)", self.def_of(card).name),
                    source: Some(card),
                    options: kinds.iter().map(|k| Opt::Text { text: format!("{k:?}") }).collect(),
                    min: 1,
                    max: 1,
                    cancellable: false,
                    hidden: false,
                    purpose: PickPurpose::Generic,
                },
            });
            self.run = Run::EnterExile { card, options: vec![], left, player, passes };
            return Flow::Wait;
        }
        if let (Some(k), true) = (kinds.first(), left > 0) {
            self.add_counters_to(Obj::card(card), *k, left as i32);
        }
        self.cards[card.0 as usize].enter_done |= 256;
        self.layers_dirty = true;
        if !self.enter_pending(card) {
            if let Some(p) = self.enter_choices.iter().position(|c| *c == card) {
                self.enter_choices.remove(p);
            }
        }
        self.ask_enter_choice(player, passes)
    }

    pub(crate) fn step_enter_exile(&mut self, card: CardId, options: Vec<CardId>, left: u8, player: PlayerId, passes: u8) -> Flow {
        let ans = self.answer.take();
        if !options.is_empty() {
            let mut picked: Vec<CardId> = match ans {
                Some(Choice::Pick { indices }) => indices.iter().filter_map(|i| options.get(*i)).copied().collect(),
                _ => vec![],
            };
            picked.dedup();
            if picked.len() != left as usize {
                picked = options.iter().copied().take(left as usize).collect();
            }
            for id in &picked {
                self.move_card(*id, Zone::Exile, Default::default());
            }
            return self.enter_exile_counters(card, left, player, passes);
        }
        let kinds: Vec<CounterKind> = self.db.def(self.cards[card.0 as usize].def).abilities.iter().find_map(|a| if let Ability::EntersExileXCounters { kinds, .. } = a { Some(kinds.clone()) } else { None }).unwrap_or_default();
        let idx = match ans {
            Some(Choice::Pick { indices }) => indices.first().copied().unwrap_or(0),
            _ => 0,
        };
        if let Some(k) = kinds.get(idx).or(kinds.first()) {
            self.add_counters_to(Obj::card(card), *k, 1);
        }
        self.enter_exile_counters(card, left.saturating_sub(1), player, passes)
    }

    pub(crate) fn step_enter_exile_any(&mut self, card: CardId, options: Vec<CardId>, player: PlayerId, passes: u8) -> Flow {
        let mut picked: Vec<CardId> = match self.answer.take() {
            Some(Choice::Pick { indices }) => indices.iter().filter_map(|i| options.get(*i)).copied().collect(),
            _ => vec![],
        };
        picked.dedup();
        let (mut p, mut t) = (0, 0);
        for id in &picked {
            // a characteristic-defining P/T works in every zone (Dungrove Elder in the graveyard still counts its controller's Forests)
            let cda = self.db.def(self.cards[id.0 as usize].def).abilities.iter().find_map(|a| if let Ability::Static { effect: StaticEffect::CdaPT { power, toughness }, .. } = a { Some((power.clone(), toughness.clone())) } else { None });
            let (cp, ct) = match cda {
                Some((cp, ct)) => {
                    let ec = crate::eval::EvalCtx::with_controller(*id, player);
                    (self.eval_amount(&cp, &ec), self.eval_amount(&ct, &ec))
                }
                None => (self.cards[id.0 as usize].cur.power, self.cards[id.0 as usize].cur.toughness),
            };
            p += cp.max(0);
            t += ct.max(0);
            self.move_card(*id, Zone::Exile, Default::default());
        }
        self.cards[card.0 as usize].enter_exiled_pt = (p, t);
        self.cards[card.0 as usize].enter_done |= 2048;
        self.layers_dirty = true;
        if !self.enter_pending(card) {
            if let Some(i) = self.enter_choices.iter().position(|c| *c == card) {
                self.enter_choices.remove(i);
            }
        }
        self.ask_enter_choice(player, passes)
    }

    pub(crate) fn step_enter_unless(&mut self, card: CardId, player: PlayerId, passes: u8) -> Flow {
        let cost = self.db.def(self.cards[card.0 as usize].def).abilities.iter().find_map(|a| if let Ability::EntersUnlessCost { cost } = a { Some(cost.clone()) } else { None }).unwrap_or_default();
        let who = self.cards[card.0 as usize].cur.controller;
        let paid = matches!(self.answer.take(), Some(Choice::YesNo { yes: true })) && self.try_pay_costs(who, card, &cost);
        self.cards[card.0 as usize].enter_done |= 32;
        // the enters event waited for this payment: a declined Mox never entered (no enters triggers), a paid one enters now
        let waiting = self.copy_deferred.iter().position(|(c, _)| *c == card).map(|i| self.copy_deferred.remove(i));
        if !paid {
            self.move_card(card, Zone::Graveyard, Default::default());
        } else if let Some((_, from)) = waiting {
            crate::triggers::on_event(self, &Event::Moved { card, from, to: Zone::Battlefield });
        }
        if !self.enter_pending(card) {
            if let Some(p) = self.enter_choices.iter().position(|c| *c == card) {
                self.enter_choices.remove(p);
            }
        }
        self.ask_enter_choice(player, passes)
    }

    pub(crate) fn step_enter_pay(&mut self, card: CardId, life: i32, player: PlayerId, passes: u8) -> Flow {
        if let Some(Choice::YesNo { yes: true }) = self.answer.take() {
            let who = self.cards[card.0 as usize].cur.controller;
            if self.can_pay_life(who, life) {
                self.lose_life(who, life);
                // paying can't untap a land another effect makes enter tapped (Archon of Emeria)
                if !self.enters_tapped_by_replacement(card) {
                    self.cards[card.0 as usize].tapped = false;
                }
            }
        }
        self.cards[card.0 as usize].enter_done |= 1;
        if !self.enter_pending(card) {
            if let Some(p) = self.enter_choices.iter().position(|c| *c == card) {
                self.enter_choices.remove(p);
            }
        }
        self.ask_enter_choice(player, passes)
    }

    pub(crate) fn step_enter_choice(&mut self, card: CardId, kind: ChooseKind, options: Vec<String>, player: PlayerId, passes: u8) -> Flow {
        if let Some(Choice::Pick { indices }) = self.answer.take() {
            let i = indices.first().copied().unwrap_or(0);
            match kind {
                ChooseKind::SagaChapter => {
                    let n: i32 = options.get(i).and_then(|t| t.parse().ok()).unwrap_or(1);
                    self.cards[card.0 as usize].add_counters(CounterKind::Lore, n);
                    // only the chosen chapter triggers (the event counts as one counter being added)
                    self.emit(Event::Counters { card, kind: CounterKind::Lore, delta: 1, by: self.cause_ctl.get() });
                }
                ChooseKind::PayLife => {
                    let n: i32 = options.get(i).and_then(|t| t.parse().ok()).unwrap_or(0);
                    if n > 0 {
                        let who = self.cards[card.0 as usize].cur.controller;
                        self.pay_life(who, n);
                    }
                    self.cards[card.0 as usize].chosen_number = Some(n);
                }
                ChooseKind::ExileSpellFromGraveyard => {
                    if let Some(g) = options.get(i).and_then(|t| t.parse::<u32>().ok()).map(CardId) {
                        self.repl_exile_src.set(Some(card));
                        self.move_card(g, Zone::Exile, Default::default());
                        self.repl_exile_src.set(None);
                    }
                }
                ChooseKind::Opponent => {
                    let who = self.cards[card.0 as usize].cur.controller;
                    self.cards[card.0 as usize].chosen_player = self.opponents(who).get(i).copied();
                }
                ChooseKind::Player => self.cards[card.0 as usize].chosen_player = self.alive_players().get(i).copied(),
                ChooseKind::WarPeace => {
                    let mut p = self.cards[card.0 as usize].cur.controller;
                    for _ in 0..self.players.len() {
                        if self.alive(p) && !self.cards[card.0 as usize].player_pick.iter().any(|(q, _)| *q == p) {
                            break;
                        }
                        p = self.next_player(p);
                    }
                    self.cards[card.0 as usize].player_pick.push((p, i == 1));
                }
                ChooseKind::SecondPlayer => {
                    let first = self.cards[card.0 as usize].chosen_player;
                    self.cards[card.0 as usize].chosen_player2 = self.alive_players().into_iter().filter(|p| Some(*p) != first).nth(i);
                }
                ChooseKind::SecondBasicLandType => self.cards[card.0 as usize].chosen_type2 = options.get(i).map(|t| Sym::new(t)),
                ChooseKind::SecondColor => self.cards[card.0 as usize].chosen_color2 = options.get(i).and_then(|n| Color::ALL.iter().copied().find(|c| format!("{c:?}") == *n)),
                ChooseKind::Color => self.cards[card.0 as usize].chosen_color = options.get(i).and_then(|n| Color::ALL.iter().copied().find(|c| format!("{c:?}") == *n)),
                _ => self.cards[card.0 as usize].chosen_type = options.get(i).map(|t| Sym::new(t)),
            }
            self.layers_dirty = true;
            self.recompute_layers();
            self.apply_enter_counters(card);
        }
        let more_players = kind == ChooseKind::WarPeace && self.alive_players().iter().any(|p| !self.cards[card.0 as usize].player_pick.iter().any(|(q, _)| q == p));
        if more_players {
            return self.ask_enter_choice(player, passes);
        }
        let first_done = self.cards[card.0 as usize].enter_done & 8 != 0;
        self.cards[card.0 as usize].enter_done |= if first_done { 64 } else { 8 };
        if !self.enter_pending(card) {
            if let Some(p) = self.enter_choices.iter().position(|c| *c == card) {
                self.enter_choices.remove(p);
            }
        }
        self.ask_enter_choice(player, passes)
    }

    /// Asks the owners of cards that an optional zone-change replacement (Library of Leng) could have sent elsewhere.
    fn ask_opt_zone(&mut self, player: PlayerId, passes: u8) -> Flow {
        while !self.opt_zone_q.is_empty() {
            let (card, zcc, instead, bottom) = self.opt_zone_q.remove(0);
            let c = &self.cards[card.0 as usize];
            if c.zcc != zcc || c.zone == Zone::Battlefield || c.token {
                continue;
            }
            let owner = c.owner;
            self.pending = Some(Decision {
                player: owner,
                kind: DecisionKind::YesNo { prompt: format!("Put {} on {} of your library instead?", self.def_of(card).name, if bottom { "the bottom" } else { "top" }), source: Some(card) },
            });
            self.run = Run::OptZone { card, instead, bottom, player, passes };
            return Flow::Wait;
        }
        self.run = Run::Priority { player, passes };
        Flow::Continue
    }

    /// Asks the owner which of several zone-change replacements applied to a card (Nemata and Rest in Peace); only that one's follow-up happens.
    fn ask_zone_choice(&mut self, player: PlayerId, passes: u8) -> Flow {
        while !self.zone_choice_q.is_empty() {
            let (card, zcc, cands) = self.zone_choice_q.remove(0);
            let c = &self.cards[card.0 as usize];
            if c.zcc != zcc {
                continue;
            }
            let owner = c.owner;
            self.pending = Some(Decision {
                player: owner,
                kind: DecisionKind::Pick {
                    prompt: format!("Choose which replacement effect applies to {}", self.def_of(card).name),
                    source: Some(card),
                    options: cands.iter().map(|c| Opt::Obj { obj: Obj::card(c.0) }).collect(),
                    min: 1,
                    max: 1,
                    cancellable: false,
                    hidden: false,
                    purpose: PickPurpose::Generic,
                },
            });
            self.run = Run::ZoneChoice { card, cands, player, passes };
            return Flow::Wait;
        }
        self.run = Run::Priority { player, passes };
        Flow::Continue
    }

    pub(crate) fn step_zone_choice(&mut self, card: CardId, cands: Vec<(CardId, PlayerId, Vec<Effect>)>, player: PlayerId, passes: u8) -> Flow {
        let i = match self.answer.take() {
            Some(Choice::Pick { indices }) => indices.first().copied().unwrap_or(0),
            _ => 0,
        };
        if let Some((src, ctrl, then)) = cands.get(i).or(cands.first()).cloned() {
            if !then.is_empty() {
                let owner = self.cards[card.0 as usize].owner;
                self.dmg_followups.borrow_mut().push((src, ctrl, then, 1, Some(card), Some(card), Some(owner), vec![]));
                self.flush_followups();
            }
        }
        self.ask_zone_choice(player, passes)
    }

    pub(crate) fn step_opt_zone(&mut self, card: CardId, instead: Zone, bottom: bool, player: PlayerId, passes: u8) -> Flow {
        if let Some(Choice::YesNo { yes: true }) = self.answer.take() {
            if self.cards[card.0 as usize].zone != Zone::Battlefield {
                self.move_card(card, instead, crate::zones::MoveOpts { bottom, ..Default::default() });
            }
        }
        self.ask_opt_zone(player, passes)
    }

    pub(crate) fn step_priority(&mut self, player: PlayerId, passes: u8) -> Flow {
        if self.answer.is_none() && !self.attach_pending.is_empty() {
            self.run = Run::Priority { player, passes };
            if let Some(f) = self.ask_attach_player(Some((player, passes))) {
                return f;
            }
        }
        if self.answer.is_none() && !self.opt_zone_q.is_empty() {
            return self.ask_opt_zone(player, passes);
        }
        if self.answer.is_none() && !self.zone_choice_q.is_empty() {
            return self.ask_zone_choice(player, passes);
        }
        if self.answer.is_none() && !self.enter_choices.is_empty() {
            return self.ask_enter_choice(player, passes);
        }
        if self.sba_waiting || self.answer.is_none() {
            self.run = Run::Priority { player, passes };
            if let Some(f) = self.checkpoint() {
                if let Flow::Continue = f {
                    // a trigger cast started (run was replaced) or the game ended
                    return Flow::Continue;
                }
                return Flow::Wait;
            }
            if self.sba_waiting {
                self.sba_waiting = false;
            }
            // state-based actions can put a permanent onto the battlefield that has an as-enters choice (a defeated battle's back face)
            if self.answer.is_none() && !self.enter_choices.is_empty() {
                return self.ask_enter_choice(player, passes);
            }
            // the checkpoint may have sent cards away under several competing replacements (Nemata and Rest in Peace)
            if self.answer.is_none() && !self.zone_choice_q.is_empty() {
                return self.ask_zone_choice(player, passes);
            }
        }
        let mut player = player;
        if !self.alive(player) {
            player = self.next_player(player);
        }
        if let Some(Choice::Mana { card, ability, produce, x: mana_x, picks }) = self.answer.clone() {
            self.mana_x_hint.set(mana_x);
            *self.mana_color_hints.borrow_mut() = picks;
            self.mana_produce_hint.set(produce);
            // floating mana: tap the source, then ask for priority again (nothing else changes)
            self.answer = None;
            // "a player of your choice adds ...": ask who gets the mana (Victory Chimes)
            if self.mana_gift_ability(player, card, ability) {
                let options: Vec<PlayerId> = std::iter::once(player).chain(self.players.iter().map(|p| p.id).filter(|q| *q != player && self.alive(*q))).collect();
                if options.len() > 1 {
                    self.pending = Some(Decision {
                        player,
                        kind: DecisionKind::Pick {
                            prompt: format!("{}: choose a player to add the mana", self.def_of(card).name),
                            source: Some(card),
                            options: options.iter().map(|p| Opt::Obj { obj: Obj::player(*p) }).collect(),
                            min: 1,
                            max: 1,
                            cancellable: false,
                            hidden: false,
                            purpose: PickPurpose::Generic,
                        },
                    });
                    self.run = Run::ManaGiftAsk { card, ability, produce, options, player, passes };
                    return Flow::Wait;
                }
            }
            // a graveyard-exile cost with a choice in it (Sunken Palace): ask which cards
            if let Some((options, n)) = self.mana_pick_options(player, card, ability) {
                self.pending = Some(Decision {
                    player,
                    kind: DecisionKind::Pick {
                        prompt: format!("Exile {n} cards from your graveyard for {}", self.def_of(card).name),
                        source: Some(card),
                        options: options.iter().map(|id| Opt::Obj { obj: Obj::card(*id) }).collect(),
                        min: n as u8,
                        max: n as u8,
                        cancellable: false,
                        hidden: false,
                        purpose: PickPurpose::Cost,
                    },
                });
                self.run = Run::ManaPickAsk { card, ability, produce, options, player, passes };
                return Flow::Wait;
            }
            let _ = self.activate_mana_ability(player, card, ability, produce);
            self.run = Run::Priority { player, passes };
            return Flow::Continue;
        }
        if let Some(Choice::Priority { action }) = self.answer.take() {
            match action {
                None => return self.pass_priority(player, passes),
                Some(i) => {
                    let Some(a) = self.last_actions.get(i).cloned() else {
                        self.run = Run::Priority { player, passes };
                        return Flow::Continue;
                    };
                    self.do_action(player, a);
                    return Flow::Continue;
                }
            }
        }
        // ask
        let actions = self.legal_actions(player);
        // "end turn" / "pass to a phase" / "pass to a stack item": keep passing every plain priority (also with things on
        // the stack) until the target is reached; questions other than priority (blocks, costs, targets...) still come through
        let mut skipping = false;
        if let Some(target) = self.players[player as usize].pass_until {
            let reached = match target {
                PassUntil::Step { turn, step } => (self.turn, self.step as u8) >= (turn, step as u8),
                PassUntil::StackTop { item } => self.stack.last().map_or(true, |top| top.id == item) || !self.stack.iter().any(|x| x.id == item),
                // a player who has left the game never gets that turn: stop passing rather than pass forever
                PassUntil::PlayerTurn { player: who, after } => (self.active == who && self.turn > after) || self.players[who as usize].lost,
            };
            if reached {
                self.players[player as usize].pass_until = None;
            } else {
                skipping = true;
            }
        }
        let auto = skipping || {
            let pl = &self.players[player as usize];
            let mask = if player == self.active { pl.stops_mine } else { pl.stops_opp };
            let stopped_here = mask & Step::stop_bit(self.step) != 0;
            if pl.full_control {
                false
            } else if actions.is_empty() {
                // nothing to cast, but the spell or ability about to resolve makes players sacrifice: stop so lands and
                // other mana sources can be tapped first and the mana kept (Lotus Field's "sacrifice two lands")
                pl.auto_skip && !(self.top_makes_sacrifice() && !self.mana_actions(player).is_empty())
            } else {
                // something could be done: ask only when responding to the stack or at a step the player stops on
                pl.auto_skip && self.stack.is_empty() && !stopped_here
            }
        };
        if auto {
            return self.pass_priority(player, passes);
        }
        // mana abilities are only offered along with the real actions (they never decide whether the player is asked)
        let mut actions = actions;
        if self.players[player as usize].offer_mana {
            actions.extend(self.mana_actions(player));
        }
        self.last_actions = actions.clone();
        self.pending = Some(Decision { player, kind: DecisionKind::Priority { actions, stack_len: self.stack.len() as u32 } });
        self.run = Run::Priority { player, passes };
        Flow::Wait
    }

    /// The top of the stack has a "sacrifice" effect of its own (not counting nested effects).
    fn top_makes_sacrifice(&self) -> bool {
        use crate::dsl::Ability;
        let Some(top) = self.stack.last() else { return false };
        let d = self.db.def(top.def);
        let effects: &[Effect] = match &top.kind {
            ItemKind::Spell { .. } => d.abilities.iter().find_map(|a| if let Ability::Spell { effects, .. } = a { Some(&effects[..]) } else { None }).unwrap_or(&[]),
            ItemKind::Ability { ability, .. } | ItemKind::Triggered { ability, .. } => match d.abilities.get(*ability as usize) {
                Some(Ability::Activated { effects, .. } | Ability::Triggered { effects, .. }) => effects,
                _ => &[],
            },
            ItemKind::Delayed { effects } => effects,
        };
        effects.iter().any(|e| matches!(e, Effect::Sacrifice { .. }))
    }

    /// Play a land card during an effect's resolution (uses a land drop; nothing happens when none is left).
    pub(crate) fn play_land_free(&mut self, player: PlayerId, card: CardId) -> bool {
        let pl = &self.players[player as usize];
        if (pl.lands_played as i32) >= pl.land_limit as i32 + self.extra_land_drops(player) || self.cant_play_land_card(player, card) {
            return false;
        }
        let from = self.cards[card.0 as usize].zone;
        self.players[player as usize].lands_played += 1;
        self.move_card(card, Zone::Battlefield, crate::zones::MoveOpts { controller: Some(player), ..Default::default() });
        self.cards[card.0 as usize].land_played = true;
        self.emit(Event::LandPlayed { card, player, from_hand: from == Zone::Hand, from });
        true
    }

    fn do_action(&mut self, player: PlayerId, a: Action) {
        self.zone_batch.clear();
        match a {
            Action::PlayLand { card } => {
                let from = self.cards[card.0 as usize].zone;
                let from_hand = from == Zone::Hand;
                if from == Zone::Graveyard {
                    self.use_per_type(player, card, from);
                }
                if from == Zone::Sideboard && !self.outside_grants.iter().any(|&(pl, t)| pl == player && t == self.turn) {
                    if let Some(i) = self.outside_card_grant_idx(player, card) {
                        let g = self.outside_card_grants.remove(i);
                        self.outside_play_used(g.3);
                    }
                }
                let once = self.once_play_src(player, card, from);
                let top_src = self.play_exile_top_src(player, card, from);
                let mark_src = self.play_mark_src(player, card, from);
                self.cards[card.0 as usize].play_leave_gain = once.map_or(0, |u| u.1);
                if let Some((b, _)) = once {
                    self.once_play_used(b);
                }
                self.players[player as usize].lands_played += 1;
                let gy_tapped = (from == Zone::Graveyard && self.gy_tapped_grant(player, card)) || (from == Zone::Exile && self.cards[card.0 as usize].play_land_tapped);
                self.move_card(card, Zone::Battlefield, crate::zones::MoveOpts { controller: Some(player), tapped: gy_tapped, ..Default::default() });
                self.cards[card.0 as usize].land_played = true;
                if let Some(b) = top_src {
                    self.exile_top_for(player, b);
                }
                if let Some(b) = mark_src {
                    self.cards[card.0 as usize].played_via = Some((b, self.cards[b.0 as usize].zcc));
                }
                self.emit(Event::LandPlayed { card, player, from_hand, from });
                self.run = Run::Priority { player, passes: 0 };
            }
            Action::PlayLandFace { card } => {
                let from = self.cards[card.0 as usize].zone;
                let from_hand = from == Zone::Hand;
                if let Some(back) = self.def_of(card).other_face {
                    self.cards[card.0 as usize].def = back;
                }
                self.players[player as usize].lands_played += 1;
                self.move_card(card, Zone::Battlefield, crate::zones::MoveOpts { controller: Some(player), ..Default::default() });
                self.cards[card.0 as usize].land_played = true;
                self.emit(Event::LandPlayed { card, player, from_hand, from });
                self.run = Run::Priority { player, passes: 0 };
            }
            Action::Cast { card, alt } => self.begin_cast(player, card, alt),
            Action::Activate { card, ability } => self.begin_activate(player, card, ability),
            Action::ActivateMana { card, ability } => {
                let _ = self.activate_mana_ability(player, card, ability, None);
                self.run = Run::Priority { player, passes: 0 };
            }
            Action::UnlockDoor { card, door } => {
                let cost = self.unlock_cost(player, card, door).display();
                self.pay_unlock.set(true);
                let paid = self.cards[card.0 as usize].doors & (1 << (door - 1)) == 0 && self.try_pay_costs(player, card, &[crate::dsl::Cost::Mana { cost }]);
                self.pay_unlock.set(false);
                if paid {
                    self.cards[card.0 as usize].doors |= 1 << (door - 1);
                    self.layers_dirty = true;
                    self.recompute_layers();
                    self.emit(Event::Unlocked { card, door });
                    if self.cards[card.0 as usize].doors == 3 {
                        self.emit(Event::FullyUnlocked { card });
                    }
                }
                self.run = Run::Priority { player, passes: 0 };
            }
            Action::RollPlanarDie => {
                // special action: pay {N} (N = this player's rolls before this one this turn), then roll
                let cost = self.planar_die_cost(player);
                let src = self.planes.first().copied();
                let paid = match src {
                    Some(s) if cost == 0 => {
                        let _ = s;
                        true
                    }
                    Some(s) => self.try_pay_costs(player, s, &[crate::dsl::Cost::Mana { cost: format!("{{{cost}}}") }]),
                    None => false,
                };
                if paid {
                    self.roll_planar_die(player);
                }
                self.run = Run::Priority { player, passes: 0 };
            }
            Action::Companion { card } => {
                if self.cards[card.0 as usize].is_companion && self.cards[card.0 as usize].zone == Zone::Sideboard && self.try_pay_costs(player, card, &[crate::dsl::Cost::Mana { cost: "{3}".into() }]) {
                    self.move_card(card, Zone::Hand, crate::zones::MoveOpts::default());
                }
                self.run = Run::Priority { player, passes: 0 };
            }
            Action::TurnFaceUp { card } => {
                // a manifested creature card is turned face up for its mana cost (a special action)
                let cost = self.def_of(card).cost.display();
                // paying to turn a manifested creature face up is paying for an ability of that permanent (Qarsi Deceiver's restricted mana)
                self.pay_for.set(Some(card));
                self.pay_is_ability.set(true);
                let paid = self.try_pay_costs(player, card, &[crate::dsl::Cost::Mana { cost }]);
                self.pay_for.set(None);
                self.pay_is_ability.set(false);
                if paid {
                    self.cards[card.0 as usize].face_down = false;
                    self.layers_dirty = true;
                    self.recompute_layers();
                    self.queue_face_up_choice(card);
                    self.emit(Event::TurnedFaceUp { card });
                }
                self.run = Run::Priority { player, passes: 0 };
            }
        }
    }

    fn pass_priority(&mut self, player: PlayerId, passes: u8) -> Flow {
        let alive = self.alive_players().len() as u8;
        let passes = passes + 1;
        if passes >= alive {
            if let Some(top) = self.stack.last() {
                let _ = top;
                self.begin_resolve();
            } else {
                self.end_step();
            }
        } else {
            self.run = Run::Priority { player: self.next_player(player), passes };
        }
        Flow::Continue
    }

    fn end_step(&mut self) {
        if self.step == Step::EndCombat {
            self.combat = None;
            self.block_limits.clear();
            let act = self.active;
            self.effects.retain(|e| e.duration != Duration::UntilEndOfCombat && !(e.duration == Duration::UntilAffectedControllersCombatEnd && e.affects.iter().any(|(id, _)| self.cards[id.0 as usize].cur.controller == act)));
            self.temp_triggers.retain(|t| t.until != TempUntil::EndOfCombat);
            self.layers_dirty = true;
        }
        if self.step == Step::EndCombat && self.extra_combat_now > 0 {
            self.extra_combat_now -= 1;
            self.goto_step(Step::BeginCombat);
            return;
        }
        if self.step == Step::EndCombat && self.extra_beginning_combat > 0 {
            // an additional beginning phase right after the combat phase: untap, upkeep, draw, then the post-combat main phase
            self.extra_beginning_combat -= 1;
            self.in_extra_beginning = true;
            self.extra_beginning_return = Some(Step::Main2);
            self.goto_step(Step::Untap);
            return;
        }
        if self.step == Step::EndCombat && self.extra_upkeep > 0 && self.upkeep_return.is_none() {
            // Obeka: additional upkeep steps after the combat phase, then the post-combat main phase
            self.extra_upkeep -= 1;
            self.upkeep_return = Some(Step::Main2);
            self.goto_step(Step::Upkeep);
            return;
        }
        if self.step == Step::Upkeep {
            let (act, now) = (self.active, self.turn);
            self.effects.retain(|e| !(e.duration == Duration::UntilEndOfYourNextUpkeep && e.controller == act && e.created_turn < now));
            self.layers_dirty = true;
        }
        if self.step == Step::Upkeep {
            if self.extra_upkeep > 0 {
                self.extra_upkeep -= 1;
                self.goto_step(Step::Upkeep);
                return;
            }
            if let Some(r) = self.upkeep_return.take() {
                self.goto_step(r);
                return;
            }
        }
        if self.step == Step::Draw && self.in_extra_beginning {
            self.in_extra_beginning = false;
            let r = self.extra_beginning_return.take().unwrap_or(Step::End);
            self.goto_step(r);
            return;
        }
        if self.step == Step::Main2 && self.extra_beginning > 0 && self.extra_main_repeat == 0 && self.extra_combat_after_main == 0 {
            self.extra_beginning -= 1;
            self.in_extra_beginning = true;
            self.goto_step(Step::Untap);
            return;
        }
        if self.step == Step::EndCombat && self.extra_main_pending > 0 {
            self.extra_main_pending -= 1;
            self.extra_main_repeat += 1;
        }
        if self.step == Step::Main2 && self.extra_main_repeat > 0 && self.extra_combat_after_main == 0 {
            self.extra_main_repeat -= 1;
            self.goto_step(Step::Main2);
            return;
        }
        if self.step == Step::Main2 && self.extra_combat_after_main > 0 {
            self.extra_combat_after_main -= 1;
            self.goto_step(Step::BeginCombat);
            return;
        }
        if self.step == Step::End && self.extra_end > 0 {
            self.extra_end -= 1;
            self.goto_step(Step::End);
            return;
        }
        // rule 514.3a: players received priority during cleanup (a trigger or state-based action), so another cleanup step follows
        if self.step == Step::Cleanup {
            self.run = Run::Cleanup;
            return;
        }
        match self.step.next() {
            Some(s) => self.goto_step(s),
            None => self.begin_next_turn(),
        }
    }

    pub(crate) fn step_cleanup(&mut self) -> Flow {
        let p = self.active;
        self.player_prot.retain(|pp| pp.until != Duration::UntilEndOfTurn);
        self.expire_text_changes();
        if self.wok_pending.take().is_some() {
            self.resolve_ctl = None;
            self.land_mana_only = None;
        }
        // discard down to the maximum hand size
        let max = self.max_hand_size(p).unwrap_or(-1);
        let hand = self.players[p as usize].hand.clone();
        if max >= 0 && hand.len() as i32 > max {
            let excess = hand.len() as i32 - max;
            if let Some(Choice::Pick { indices }) = self.answer.take() {
                for i in indices {
                    if let Some(&id) = hand.get(i) {
                        self.move_card(id, Zone::Graveyard, Default::default());
                        self.emit(Event::Discarded { card: id, player: p, by: self.cause_ctl.get() });
                    }
                }
            } else {
                self.pending = Some(Decision {
                    player: p,
                    kind: DecisionKind::Pick {
                        prompt: format!("Discard {excess} card(s) to your maximum hand size"),
                        source: None,
                        options: hand.iter().map(|c| Opt::Obj { obj: Obj::card(*c) }).collect(),
                        min: excess as u8,
                        max: excess as u8,
                        cancellable: false,
                        hidden: true,
                        purpose: PickPurpose::Discard,
                    },
                });
                self.run = Run::Cleanup;
                return Flow::Wait;
            }
        }
        self.temp_replaces.clear();
        // emblems that last until end of turn cease to exist
        let gone: Vec<CardId> = self.players.iter().flat_map(|pl| pl.command.iter().copied()).filter(|id| self.cards[id.0 as usize].emblem_eot).collect();
        for id in gone {
            self.move_card(id, Zone::Exile, Default::default());
        }
        for (id, zcc, kind, n) in std::mem::take(&mut self.cleanup_counter_removals) {
            if self.cards[id.0 as usize].zcc == zcc && self.cards[id.0 as usize].zone == Zone::Battlefield {
                let have = self.cards[id.0 as usize].counter(kind);
                let d = n.min(have);
                if d > 0 {
                    self.cards[id.0 as usize].remove_counters(kind, d);
                    self.layers_dirty = true;
                    self.emit(Event::Counters { card: id, kind, delta: -d, by: None });
                }
            }
        }
        // damage wears off and "until end of turn" effects end simultaneously
        let keeps_damage: Vec<bool> = self
            .cards
            .iter()
            .map(|c| c.zone == Zone::Battlefield && !c.cur.no_abilities && self.db.def(c.def).statics.iter().any(|&i| matches!(&self.db.def(c.def).abilities[i as usize], crate::dsl::Ability::Static { effect: crate::dsl::StaticEffect::DamageStaysAtCleanup, .. })))
            .collect();
        for (idx, c) in self.cards.iter_mut().enumerate() {
            if !keeps_damage[idx] {
                c.damage = 0;
            }
            c.saddled = false;
            c.deathtouch_hit = false;
            c.damaged_by.clear();
            if matches!(c.phase_lock, Some((p, t)) if p == self.active && self.turn > t) {
                c.phase_lock = None;
            }
            c.regen = 0;
            c.heal_shield = false;
            c.regen_steal = None;
            c.exile_if_dies = false;
        }
        self.noncombat_dmg_last = self.dmg_log.iter().filter(|r| !r.combat).filter_map(|r| if let Obj::Player { id } = r.target { Some(id) } else { None }).collect();
        self.dmg_log.clear();
        self.ability_res.clear();
        for pl in self.players.iter_mut() {
            pl.energy_spent = 0;
        }
        self.shields.clear();
        // last-known stack objects stay readable for delayed triggers that fire in a later turn (Mana Drain cast on the opponent's turn); trimmed when they pile up
        if self.stack_lki.len() > 128 || self.stack_lki_src.len() > 128 || self.stack_lki_items.len() > 128 {
            self.stack_lki.clear();
            self.stack_lki_src.clear();
            self.stack_lki_x.clear();
            self.stack_lki_items.clear();
        }
        let (act, now) = (self.active, self.turn);
        self.temp_triggers.retain(|t| t.until != TempUntil::EndOfTurn && !(t.until == TempUntil::EndOfYourNextTurn && t.d.controller == act && t.d.created_turn < now) && !(t.until == TempUntil::EndOfCarriedPlayersNextTurn && matches!(t.d.remembered.first(), Some(Obj::Player { id }) if *id == act) && t.d.created_turn < now));
        self.exile_marked.clear();
        for (id, zcc, def, _) in std::mem::take(&mut self.copy_reverts).into_iter().rev() {
            if self.cards[id.0 as usize].zcc == zcc && self.cards[id.0 as usize].zone == Zone::Battlefield {
                self.cards[id.0 as usize].def = def;
            }
        }
        let turn = self.turn;
        self.effects.retain(|e| match e.duration {
            Duration::UntilEndOfTurn | Duration::Instant | Duration::UntilEndOfCombat => false,
            Duration::UntilEndOfYourNextTurn => !(e.controller == self.active && e.created_turn < turn),
            Duration::UntilEndOfAffectedControllersNextTurn => !(e.created_turn < turn && (e.affects.iter().any(|(id, _)| self.cards[id.0 as usize].cur.controller == self.active) || (e.affects.is_empty() && self.cards[e.source.0 as usize].chosen_player == Some(self.active)))),
            Duration::UntilEndOfAffectedPlayersNextTurn => !(e.created_turn < turn && e.players.contains(&self.active)),
            Duration::UntilYourNextTurn => true,
            _ => true,
        });
        self.layers_dirty = true;
        // triggers and state-based actions during cleanup give players priority
        self.run = Run::Cleanup;
        if let Some(f) = self.checkpoint() {
            return match f {
                Flow::Wait => Flow::Wait,
                Flow::Continue => {
                    if let Run::Over = self.run {
                        self.run = Run::Cleanup;
                    }
                    Flow::Continue
                }
            };
        }
        self.begin_next_turn();
        Flow::Continue
    }
}
