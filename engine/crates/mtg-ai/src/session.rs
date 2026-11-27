//! A game in progress with its agents: AI seats play themselves, human seats wait for answers.
//! This is what the server drives; everything crossing the boundary is serializable.

use crate::agent::Agent;
use crate::mcts::{MctsAgent, MctsConfig};
use mtg_engine::types::Step;
use mtg_engine::view::View;
use mtg_engine::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use ts_rs::TS;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum Difficulty {
    Easy,
    #[default]
    Normal,
    Hard,
}

impl Difficulty {
    pub fn budget(self) -> Duration {
        match self {
            Difficulty::Easy => Duration::from_millis(40),
            Difficulty::Normal => Duration::from_millis(250),
            Difficulty::Hard => Duration::from_millis(600),
        }
    }

    /// The search settings of a tier: a stronger tier thinks longer; the easy one also misjudges close positions and sometimes slips on purpose.
    pub fn mcts_config(self) -> MctsConfig {
        let mut c = MctsConfig::new(self.budget());
        match self {
            Difficulty::Easy => {
                c.noise = 0.25;
                c.blunder = 0.3;
                c.threads = 1;
            }
            // more search does not make the AI stronger (measured), so the tiers differ by how well it judges and how often it slips
            Difficulty::Normal => {
                c.noise = 0.1;
                c.blunder = 0.08;
            }
            Difficulty::Hard => {}
        }
        c
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SeatSetup {
    pub name: String,
    pub human: bool,
    /// (card name, copies)
    pub deck: Vec<(String, u32)>,
    #[serde(default)]
    pub commander: Vec<String>,
    /// (card name, copies): cards outside the game, for Wish-style effects.
    #[serde(default)]
    pub sideboard: Vec<(String, u32)>,
    /// The chosen companion's name (one card, also listed in `sideboard`): it may be put into hand for {3}.
    #[serde(default)]
    pub companion: Option<String>,
    #[serde(default)]
    pub difficulty: Difficulty,
    /// Archenemy: (card name, copies) of this player's scheme deck; a player with schemes is the archenemy.
    #[serde(default)]
    pub schemes: Vec<(String, u32)>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GameSetup {
    #[ts(type = "number")]
    pub seed: u64,
    pub commander: bool,
    #[serde(default)]
    pub mulligan: MulliganRule,
    pub seats: Vec<SeatSetup>,
    /// Planechase: (card name, copies) of the shared planar deck (empty: no Planechase).
    #[serde(default)]
    pub planar: Vec<(String, u32)>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SessionRecord {
    pub setup: GameSetup,
    pub log: Vec<Choice>,
    /// Things a player did outside of answering a question (end turn, concede...), each at the decision-log length it happened at.
    #[serde(default)]
    pub commands: Vec<LoggedCommand>,
}

/// Something a player does to the game outside of answering its pending question. Recorded with the game so that undo and
/// save / resume (which replay the decision log) reproduce it at the same moment.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "t")]
pub enum SessionCommand {
    /// pass priority automatically until the target ("end turn", a phase, a stack item)
    PassUntil { seat: PlayerId, until: PassUntil },
    /// stop passing automatically
    CancelPass { seat: PlayerId },
    Concede { seat: PlayerId },
    /// tap lands by hand for every payment (the "auto-tap" setting off) or let the engine pay
    ManualPay { seat: PlayerId, on: bool },
    /// tap lands by hand for the next payment only ("cast, tapping mana by hand")
    ManualOnce { seat: PlayerId },
    /// "break the rules": a direct change to put right something the engine got wrong
    Edit { seat: PlayerId, op: EditOp },
    /// the rule break is over: the open priority question is asked again
    EndBreak { seat: PlayerId },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LoggedCommand {
    /// the decision-log length when it happened: it applies while decision number `at` is pending, before it is answered
    pub at: u32,
    pub cmd: SessionCommand,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Frame {
    pub seq: u32,
    pub events: Vec<Event>,
    /// One view per human seat (keyed by seat number).
    pub views: BTreeMap<u8, View>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Advance {
    pub frames: Vec<Frame>,
    /// A human seat must answer this.
    pub waiting: Option<Decision>,
    pub over: Option<GameResult>,
    /// Every human seat has left the game (lost or conceded) while AIs play on. The game goes on to be watched: `advance` then
    /// returns after each AI decision, so the caller can show it and call `advance` again.
    #[serde(default)]
    pub humans_out: bool,
}

pub struct Session {
    pub db: Arc<CardDb>,
    pub setup: GameSetup,
    pub game: Game,
    agents: Vec<Option<MctsAgent>>,
    seq: u32,
    commands: Vec<LoggedCommand>,
}

impl Session {
    /// Build a game from card names. Unknown names are an error listing all of them.
    pub fn new(db: Arc<CardDb>, setup: GameSetup) -> Result<Session, String> {
        if setup.seats.len() < 2 || setup.seats.len() > 8 {
            return Err("a game needs 2 to 8 seats".into());
        }
        let mut unknown: Vec<String> = vec![];
        let mut decks = vec![];
        for seat in &setup.seats {
            let mut main = vec![];
            for (name, n) in &seat.deck {
                match db.find(name) {
                    Some(id) => main.extend(std::iter::repeat(id).take(*n as usize)),
                    None => unknown.push(name.clone()),
                }
            }
            let mut commanders = vec![];
            for name in &seat.commander {
                match db.find(name) {
                    Some(id) => commanders.push(id),
                    None => unknown.push(name.clone()),
                }
            }
            let mut sideboard = vec![];
            for (name, n) in &seat.sideboard {
                match db.find(name) {
                    Some(id) => sideboard.extend(std::iter::repeat(id).take(*n as usize)),
                    None => unknown.push(name.clone()),
                }
            }
            let companion = match &seat.companion {
                Some(name) => match db.find(name) {
                    Some(id) => {
                        if !sideboard.contains(&id) {
                            sideboard.push(id);
                        }
                        Some(id)
                    }
                    None => {
                        unknown.push(name.clone());
                        None
                    }
                },
                None => None,
            };
            decks.push(DeckSpec { name: seat.name.clone(), main, commanders, sideboard, companion });
        }
        if !unknown.is_empty() {
            unknown.sort();
            unknown.dedup();
            return Err(format!("unsupported cards: {}", unknown.join(", ")));
        }
        // Planechase and Archenemy decks (cards the engine doesn't know are left out rather than refused)
        let named = |list: &[(String, u32)]| -> Vec<DefId> { list.iter().filter_map(|(n, k)| db.find(n).map(|id| std::iter::repeat(id).take(*k as usize))).flatten().collect() };
        let planar = named(&setup.planar);
        let schemes: Vec<Vec<DefId>> = setup.seats.iter().map(|s| named(&s.schemes)).collect();
        let mut cfg = if setup.commander { Config::commander() } else { Config::standard() };
        cfg.mulligan = setup.mulligan;
        let mut game = Game::new(db.clone(), cfg, decks, setup.seed);
        if !planar.is_empty() || schemes.iter().any(|s| !s.is_empty()) {
            game.setup_variants(&planar, &schemes);
        }
        let mut agents = vec![];
        for (i, seat) in setup.seats.iter().enumerate() {
            if seat.human {
                game.players[i].auto_skip = true;
                game.players[i].allow_cancel = true;
                game.players[i].offer_mana = true;
                agents.push(None);
            } else {
                game.players[i].auto_skip = true;
                agents.push(Some(MctsAgent::new(setup.seed ^ (i as u64 + 1) * 0x9e37, seat.difficulty.mcts_config())));
            }
        }
        Ok(Session { db, setup, game, agents, seq: 0, commands: vec![] })
    }

    pub fn restore(db: Arc<CardDb>, record: SessionRecord) -> Result<Session, String> {
        let mut s = Session::new(db, record.setup)?;
        s.replay(&record.log, &record.commands, |_, _, _, _| {})?;
        Ok(s)
    }

    /// Re-apply recorded decisions (AI seats are re-derived; only human answers are needed, but the full log also works
    /// because AI answers are deterministic given the same game state and seed). Commands are applied at the moment they
    /// were given: while decision `at` was pending, before it was answered. `seen(i, player)` is told who answered decision i.
    /// `seen(i, player, gone, casting)`: log entry `i` answers a question put to `player`; `gone` when that player had
    /// already left the game, so the answer was the engine's stand-in and not theirs; `casting` when the question was
    /// part of casting a spell or activating an ability (its modes, targets, costs, payment).
    fn replay(&mut self, log: &[Choice], commands: &[LoggedCommand], mut seen: impl FnMut(usize, PlayerId, bool, bool)) -> Result<(), String> {
        let mut k = 0;
        for (i, c) in log.iter().enumerate() {
            if self.game.run() == Status::Over {
                break;
            }
            let mut any = false;
            while k < commands.len() && commands[k].at as usize <= i {
                self.do_command(&commands[k]);
                k += 1;
                any = true;
            }
            // a concession drops the pending question: get to the next one
            if any && self.game.run() == Status::Over {
                break;
            }
            if let Some(d) = self.game.decision() {
                let gone = self.game.players.get(d.player as usize).map_or(false, |p| p.lost || p.conceded);
                seen(i, d.player, gone, matches!(self.game.run, mtg_engine::state::Run::Cast(_)));
            }
            self.game.apply(c.clone()).map_err(|e| format!("decision {i} rejected on restore: {e}"))?;
        }
        if k < commands.len() && self.game.run() != Status::Over {
            for c in &commands[k..] {
                self.do_command(c);
            }
        }
        self.game.drain_events();
        Ok(())
    }

    /// Apply a command and remember it (with the current decision-log length) for replay.
    fn do_command(&mut self, c: &LoggedCommand) {
        match c.cmd.clone() {
            SessionCommand::PassUntil { seat, until } => self.game.players[seat as usize].pass_until = Some(until),
            SessionCommand::CancelPass { seat } => self.game.players[seat as usize].pass_until = None,
            SessionCommand::Concede { seat } => self.game.concede(seat),
            SessionCommand::ManualPay { seat, on } => self.game.players[seat as usize].manual_pay = on,
            SessionCommand::ManualOnce { seat } => self.game.players[seat as usize].manual_once = true,
            SessionCommand::Edit { seat, op } => {
                let _ = self.game.apply_edit(seat, &op);
            }
            SessionCommand::EndBreak { .. } => self.game.end_rule_break(),
        }
        self.commands.push(c.clone());
    }

    fn command(&mut self, cmd: SessionCommand) {
        let at = self.game.decision_log.len() as u32;
        self.do_command(&LoggedCommand { at, cmd });
    }

    pub fn record(&self) -> SessionRecord {
        SessionRecord { setup: self.setup.clone(), log: self.game.decision_log.clone(), commands: self.commands.clone() }
    }

    fn human_views(&self) -> BTreeMap<u8, View> {
        self.setup.seats.iter().enumerate().filter(|(_, s)| s.human).map(|(i, _)| (i as u8, self.game.view_for(i as u8))).collect()
    }

    fn frame(&mut self) -> Option<Frame> {
        let events = self.game.drain_events();
        let visible = events.iter().any(|e| !matches!(e, Event::Log { .. }));
        if !visible {
            return None;
        }
        self.seq += 1;
        Some(Frame { seq: self.seq, events, views: self.human_views() })
    }

    /// Run until a human must answer (or the game ends). AI decisions in between become frames for playback.
    pub fn advance(&mut self) -> Advance {
        let mut frames = vec![];
        loop {
            let status = self.game.run();
            if let Some(f) = self.frame() {
                frames.push(f);
            }
            if status == Status::Over {
                break;
            }
            // watching AIs only: hand back after a decision that showed something, so playback stays live
            if self.humans_out() && !frames.is_empty() {
                break;
            }
            let d = self.game.decision().cloned().expect("decision pending");
            // a player who has left the game never answers: without this the game would wait on them for ever
            // (a concession while they were being asked something other than priority, such as the mulligan)
            if self.game.players.get(d.player as usize).map_or(false, |p| p.lost || p.conceded) && self.game.apply(Choice::Default).is_ok() {
                continue;
            }
            match self.agents.get_mut(d.player as usize).and_then(|a| a.as_mut()) {
                Some(agent) => {
                    agent.reseed(self.setup.seed ^ ((d.player as u64 + 1) << 40) ^ (self.game.decision_log.len() as u64).wrapping_mul(0x9e3779b97f4a7c15));
                    let c = agent.decide(&self.game, &d);
                    if self.game.apply(c).is_err() {
                        self.game.apply(Choice::Default).expect("default choice must be legal");
                    }
                }
                None => break,
            }
        }
        // always give the clients the final state after the last event, including the question
        if frames.is_empty() {
            self.seq += 1;
            frames.push(Frame { seq: self.seq, events: vec![], views: self.human_views() });
        } else {
            let last = frames.last_mut().unwrap();
            last.views = self.human_views();
        }
        let humans_out = self.game.over.is_none() && self.humans_out();
        Advance { frames, waiting: if humans_out { None } else { self.game.decision().cloned() }, over: self.game.over.clone(), humans_out }
    }

    /// The game has human seats and all of them have left it.
    pub fn humans_out(&self) -> bool {
        let humans: Vec<usize> = self.setup.seats.iter().enumerate().filter(|(_, s)| s.human).map(|(i, _)| i).collect();
        !humans.is_empty() && humans.iter().all(|&i| self.game.players.get(i).map_or(true, |p| p.lost || p.conceded))
    }

    /// A human seat answers its pending question.
    pub fn answer(&mut self, seat: PlayerId, choice: Choice) -> Result<(), String> {
        match self.game.decision() {
            Some(d) if d.player == seat => {}
            Some(_) => return Err("it isn't your decision".into()),
            None => return Err("no decision is pending".into()),
        }
        self.game.apply(choice).map_err(|e| e.to_string())
    }

    /// Why something the seat tried (`kind`: "play", "attack" or "block") isn't allowed right now, in plain words.
    pub fn explain(&self, seat: PlayerId, kind: &str, card: Option<u32>, other: Option<u32>) -> String {
        let ok = |c: u32| (c as usize) < self.game.cards.len();
        let card = card.filter(|c| ok(*c)).map(CardId);
        let other = other.filter(|c| ok(*c)).map(CardId);
        match (kind, card) {
            ("play", Some(c)) => self.game.explain_play(seat, c),
            ("attack", Some(c)) => self.game.explain_attack(seat, c),
            ("tap", Some(c)) => self.game.explain_tap(seat, c),
            ("block", _) => self.game.explain_block(seat, card, other),
            _ => "Rules are enforced in this game.".into(),
        }
    }

    /// Choose where a seat wants priority: bit masks over steps (`Step::stop_bit`) for its own turn and for opponents' turns.
    pub fn set_stops(&mut self, seat: PlayerId, mine: u16, opp: u16, full_control: bool) {
        let p = &mut self.game.players[seat as usize];
        p.stops_mine = mine;
        p.stops_opp = opp;
        p.full_control = full_control;
    }

    /// Pass priority for `seat` until `turn`/`step` is reached (questions other than priority still stop it).
    pub fn pass_until(&mut self, seat: PlayerId, turn: u32, step: Step) {
        self.arm_pass(seat, PassUntil::Step { turn, step });
    }

    /// Start passing toward a turn or step. One the game has already reached (the request was made while the player
    /// was still watching earlier moves) arms nothing, so the question waiting now is the place it stops.
    fn arm_pass(&mut self, seat: PlayerId, until: PassUntil) {
        if self.game.pass_reached(until) {
            self.cancel_pass(seat);
        } else {
            self.command(SessionCommand::PassUntil { seat, until });
        }
    }

    /// Pass priority for `seat` until the stack item is on top of the stack (or has left it).
    pub fn pass_until_stack(&mut self, seat: PlayerId, item: StackId) {
        self.command(SessionCommand::PassUntil { seat, until: PassUntil::StackTop { item } });
    }

    /// Pass priority automatically until `player`'s next turn starts: a turn after `after` (the turn the player saw
    /// when they asked; the current turn when not given), so a request that arrives after that turn has begun still stops there.
    pub fn pass_until_player(&mut self, seat: PlayerId, player: PlayerId, after: Option<u32>) {
        let after = after.map_or(self.game.turn, |t| t.min(self.game.turn));
        self.arm_pass(seat, PassUntil::PlayerTurn { player, after });
    }

    /// Stop passing priority automatically.
    pub fn cancel_pass(&mut self, seat: PlayerId) {
        if self.game.players[seat as usize].pass_until.is_some() {
            self.command(SessionCommand::CancelPass { seat });
        }
    }

    /// Whether `seat` is passing priority automatically.
    pub fn passing(&self, seat: PlayerId) -> Option<PassUntil> {
        self.game.players[seat as usize].pass_until
    }

    pub fn concede(&mut self, seat: PlayerId) {
        self.command(SessionCommand::Concede { seat });
    }

    /// Tap lands by hand for every payment (`on`), or let the engine pay automatically.
    pub fn set_manual_pay(&mut self, seat: PlayerId, on: bool) {
        if self.game.players[seat as usize].manual_pay != on {
            self.command(SessionCommand::ManualPay { seat, on });
        }
    }

    /// Tap lands by hand for the next payment only.
    pub fn manual_once(&mut self, seat: PlayerId) {
        self.command(SessionCommand::ManualOnce { seat });
    }

    /// "Break the rules": apply a direct change (recorded for replay). An error changes nothing.
    pub fn edit(&mut self, seat: PlayerId, op: EditOp) -> Result<(), String> {
        // an AI shows its hidden cards to whoever breaks the rules; another player's hand or library stays theirs
        if let EditOp::Show { player, .. } = op {
            if player != seat && self.agents.get(player as usize).map_or(true, |a| a.is_none()) {
                return Err("Only your own or an AI's hand and library can be looked at while breaking the rules".into());
            }
        }
        self.game.apply_edit(seat, &op)?;
        let at = self.game.decision_log.len() as u32;
        self.commands.push(LoggedCommand { at, cmd: SessionCommand::Edit { seat, op } });
        Ok(())
    }

    /// Finish a rule break: the open priority question is asked again from scratch.
    pub fn end_break(&mut self, seat: PlayerId) {
        self.command(SessionCommand::EndBreak { seat });
    }

    /// What proposed blocks (blocker, attacker) would do: kept and dropped blocks, and where the damage would go.
    pub fn preview_blocks(&self, blocks: &[(CardId, CardId)]) -> Vec<mtg_engine::view::BlockPreview> {
        self.game.preview_blocks(blocks)
    }

    pub fn view(&self, seat: PlayerId) -> View {
        self.game.view_for(seat)
    }

    /// Undo the last `n` human actions by replaying everything else. AI answers are recomputed.
    ///
    /// One action is everything a player answered for it, not one answer: activating an ability is the choice to
    /// activate, the cost choices and payment that follow it, and the pass of priority that let it resolve. Taking back
    /// only the last of those would leave the cost paid (the permanent tapped) with the ability back on the stack.
    pub fn undo(&mut self, n: usize) -> Result<(), String> {
        let (who, gone, casting) = self.decision_players();
        let humans: Vec<bool> = self.setup.seats.iter().map(|x| x.human).collect();
        // answers given for a player who had left are not that player's to take back
        let human: Vec<usize> = (0..who.len()).filter(|&i| !gone[i] && who[i].map_or(false, |p| humans[p as usize])).collect();
        if n == 0 || human.is_empty() {
            return Err("nothing to undo".into());
        }
        let log = &self.game.decision_log;
        // 0 = a pass of priority, 1 = an action taken with priority, 2 = a question asked while doing something
        let kind = |k: usize| match &log[human[k]] {
            Choice::Priority { action: None } => 0,
            Choice::Priority { .. } => 1,
            _ => 2,
        };
        // answered one straight after the other by the same player, with nobody else deciding in between
        let follows = |a: usize, b: usize| human[a] + 1 == human[b] && who[human[a]] == who[human[b]];
        let group_start = |k: usize| -> usize {
            let mut s = k;
            if kind(s) == 0 {
                // a pass given straight after the player's own action is part of letting that action happen
                if s > 0 && follows(s - 1, s) && kind(s - 1) != 0 {
                    s -= 1;
                } else {
                    return s;
                }
            }
            // the questions of casting / activating (modes, targets, costs, payment) go with the action that started it;
            // other questions (attackers, blockers, choices while something resolves) are actions of their own, even
            // when nobody else's answer happens to sit between them and the player's previous action
            while kind(s) == 2 && casting[human[s]] && s > 0 && follows(s - 1, s) {
                s -= 1;
            }
            s
        };
        let mut end = human.len();
        let mut cut = 0;
        for _ in 0..n {
            if end == 0 {
                return Err("nothing to undo".into());
            }
            end = group_start(end - 1);
            cut = human[end];
        }
        let log: Vec<Choice> = self.game.decision_log[..cut].to_vec();
        // commands given while the undone decision was pending (an "end turn" answered by that pass) go with it, but a
        // rule break made then stays: undo takes back the answer, not the repair
        let commands: Vec<LoggedCommand> = self.commands.iter().filter(|c| (c.at as usize) < cut || (c.at as usize == cut && matches!(c.cmd, SessionCommand::Edit { .. } | SessionCommand::EndBreak { .. }))).cloned().collect();
        let rec = SessionRecord { setup: self.setup.clone(), log, commands };
        let fresh = Session::restore(self.db.clone(), rec)?;
        self.game = fresh.game;
        self.agents = fresh.agents;
        self.commands = fresh.commands;
        Ok(())
    }

    /// Who each entry of the decision log answers for, whether that player had already left the game, and whether the
    /// question was part of casting / activating (found by replaying with a cursor).
    fn decision_players(&self) -> (Vec<Option<PlayerId>>, Vec<bool>, Vec<bool>) {
        let n = self.game.decision_log.len();
        let mut who: Vec<Option<PlayerId>> = vec![None; n];
        let mut gone = vec![false; n];
        let mut casting = vec![false; n];
        let Ok(mut s) = Session::new(self.db.clone(), self.setup.clone()) else { return (who, gone, casting) };
        let _ = s.replay(&self.game.decision_log.clone(), &self.commands.clone(), |i, p, left, cast| {
            who[i] = Some(p);
            gone[i] = left;
            casting[i] = cast;
        });
        (who, gone, casting)
    }
}
