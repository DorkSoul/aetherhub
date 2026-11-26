//! "Break the rules": direct changes a player makes to put right a game the engine got wrong (a card bug). The game is
//! paused while they happen (the server lets nobody act); finishing asks the open priority question again from scratch.
//! The session records every edit so that undo and save / resume replay them at the same moment.

use crate::decision::*;
use crate::dsl::*;
use crate::ids::*;
use crate::state::*;
use crate::types::*;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "t")]
pub enum EditOp {
    /// Put a card into a zone (library: on top unless `bottom`; battlefield: under `controller`, else its owner).
    Move {
        card: CardId,
        to: Zone,
        #[serde(default)]
        bottom: bool,
        #[serde(default)]
        controller: Option<PlayerId>,
    },
    Tap { card: CardId, tapped: bool },
    /// Add (positive) or remove (negative) counters.
    Counters { card: CardId, kind: CounterKind, delta: i32 },
    Life { player: PlayerId, delta: i32 },
    /// Give a permanent to another player.
    Control { card: CardId, player: PlayerId },
    Draw { player: PlayerId },
    Shuffle { player: PlayerId },
    /// Create tokens by the name of one of the engine's token definitions (a token a card failed to make).
    CreateToken {
        player: PlayerId,
        name: String,
        #[serde(default = "one")]
        n: u32,
        #[serde(default)]
        tapped: bool,
    },
    /// Create token copies of a permanent (or of any card: the copy uses its current face).
    TokenCopy {
        card: CardId,
        player: PlayerId,
        #[serde(default = "one")]
        n: u32,
    },
    /// Show `player`'s hand or library (in order) to the player breaking the rule, until the rule break ends.
    Show { player: PlayerId, zone: Zone },
    /// Put the top `n` cards of `player`'s library into `to` (mill, exile from the top).
    TopCards { player: PlayerId, n: u32, to: Zone },
    /// Attach a permanent to another one, or unattach it (`to` none).
    Attach { card: CardId, to: Option<CardId> },
    /// Turn a double-faced permanent to its other face.
    Transform { card: CardId },
    /// Raise (or lower) a creature's power and toughness, until end of turn or for good.
    Pump { card: CardId, power: i32, toughness: i32, eot: bool },
    /// Poison, energy or experience counters on a player.
    PlayerCounters { player: PlayerId, kind: PlayerCounter, delta: i32 },
    /// Combat damage `player` has taken from the commander `commander`.
    CommanderDamage { player: PlayerId, commander: CardId, delta: i32 },
}

/// The counters a player can have that a rule break can change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum PlayerCounter {
    Poison,
    Energy,
    Experience,
}

fn one() -> u32 {
    1
}

/// More tokens than this in one edit is a typo.
const MAX_EDIT_TOKENS: u32 = 50;

impl Game {
    /// Apply one "break the rules" change made by player `by`. Errors leave the game as it was.
    pub fn apply_edit(&mut self, by: PlayerId, op: &EditOp) -> Result<(), String> {
        let card_ok = |g: &Game, c: CardId| (c.0 as usize) < g.cards.len() && g.cards[c.0 as usize].zone != Zone::Sideboard;
        let player_ok = |g: &Game, p: PlayerId| (p as usize) < g.players.len() && !g.players[p as usize].lost;
        match op.clone() {
            EditOp::Move { card, to, bottom, controller } => {
                if !card_ok(self, card) {
                    return Err("no such card".into());
                }
                if to == Zone::Sideboard {
                    return Err("cards can't be moved out of the game".into());
                }
                if let Some(p) = controller {
                    if !player_ok(self, p) {
                        return Err("no such player".into());
                    }
                }
                if self.cards[card.0 as usize].zone == Zone::Stack {
                    // a spell taken off the stack: its stack entry goes with it
                    self.stack.retain(|s| !(s.source == card && matches!(s.kind, ItemKind::Spell { .. })));
                }
                self.move_card(card, to, crate::zones::MoveOpts { bottom, controller, ..Default::default() });
            }
            EditOp::Tap { card, tapped } => {
                if !card_ok(self, card) || self.cards[card.0 as usize].zone != Zone::Battlefield {
                    return Err("only a permanent can be tapped or untapped".into());
                }
                self.cards[card.0 as usize].tapped = tapped;
            }
            EditOp::Counters { card, kind, delta } => {
                if !card_ok(self, card) {
                    return Err("no such card".into());
                }
                if delta > 0 {
                    self.add_counters_to(Obj::card(card), kind, delta);
                } else if delta < 0 {
                    self.remove_counters_from(Obj::card(card), kind, -delta);
                }
            }
            EditOp::Life { player, delta } => {
                if !player_ok(self, player) {
                    return Err("no such player".into());
                }
                self.players[player as usize].life += delta;
            }
            EditOp::Control { card, player } => {
                if !card_ok(self, card) || self.cards[card.0 as usize].zone != Zone::Battlefield || !player_ok(self, player) {
                    return Err("only a permanent can change control".into());
                }
                let turn = self.turn;
                let c = &mut self.cards[card.0 as usize];
                c.controller = player;
                c.control_turn = turn;
            }
            EditOp::Draw { player } => {
                if !player_ok(self, player) {
                    return Err("no such player".into());
                }
                self.draw_card(player);
            }
            EditOp::Shuffle { player } => {
                if !player_ok(self, player) {
                    return Err("no such player".into());
                }
                self.shuffle_library(player);
            }
            EditOp::CreateToken { player, name, n, tapped } => {
                if !player_ok(self, player) {
                    return Err("no such player".into());
                }
                let def = self.db.find(&name).ok_or_else(|| format!("the rules engine has no token called \"{name}\""))?;
                for _ in 0..n.clamp(1, MAX_EDIT_TOKENS) {
                    self.edit_token(def, player, tapped, None);
                }
            }
            EditOp::TokenCopy { card, player, n } => {
                if !card_ok(self, card) || !player_ok(self, player) {
                    return Err("no such card or player".into());
                }
                let def = self.cards[card.0 as usize].def;
                for _ in 0..n.clamp(1, MAX_EDIT_TOKENS) {
                    self.edit_token(def, player, false, Some(card));
                }
            }
            EditOp::Show { player, zone } => {
                if (player as usize) >= self.players.len() || !matches!(zone, Zone::Hand | Zone::Library) {
                    return Err("only a player's hand or library can be shown".into());
                }
                if !self.edit_shown.contains(&(by, player, zone)) {
                    self.edit_shown.push((by, player, zone));
                }
            }
            EditOp::TopCards { player, n, to } => {
                if !player_ok(self, player) || matches!(to, Zone::Sideboard | Zone::Library | Zone::Stack) {
                    return Err("the top cards can go to a hand, the battlefield, a graveyard, exile or the command zone".into());
                }
                for _ in 0..n.min(999) {
                    let Some(&top) = self.players[player as usize].library.last() else { break };
                    self.move_card(top, to, crate::zones::MoveOpts::default());
                }
            }
            EditOp::Attach { card, to } => {
                let on_bf = |g: &Game, c: CardId| card_ok(g, c) && g.cards[c.0 as usize].zone == Zone::Battlefield;
                if !on_bf(self, card) || to.map_or(false, |h| !on_bf(self, h) || h == card) {
                    return Err("only a permanent can be attached to another permanent".into());
                }
                self.attach(card, to);
            }
            EditOp::Transform { card } => {
                if !card_ok(self, card) || self.cards[card.0 as usize].zone != Zone::Battlefield {
                    return Err("only a permanent can be transformed".into());
                }
                let before = self.cards[card.0 as usize].def;
                self.transform(card);
                if self.cards[card.0 as usize].def == before {
                    return Err("this permanent has no other face to transform to".into());
                }
            }
            EditOp::Pump { card, power, toughness, eot } => {
                if !card_ok(self, card) || self.cards[card.0 as usize].zone != Zone::Battlefield {
                    return Err("only a permanent's power and toughness can be changed".into());
                }
                let ctl = self.cards[card.0 as usize].controller;
                self.add_effect(card, ctl, &[card], Modif::Pump { power, toughness }, if eot { Duration::UntilEndOfTurn } else { Duration::Forever });
            }
            EditOp::PlayerCounters { player, kind, delta } => {
                if !player_ok(self, player) {
                    return Err("no such player".into());
                }
                let pl = &mut self.players[player as usize];
                let v = match kind {
                    PlayerCounter::Poison => &mut pl.poison,
                    PlayerCounter::Energy => &mut pl.energy,
                    PlayerCounter::Experience => &mut pl.experience,
                };
                *v = (*v + delta).max(0);
            }
            EditOp::CommanderDamage { player, commander, delta } => {
                if !player_ok(self, player) || !card_ok(self, commander) {
                    return Err("no such player or commander".into());
                }
                let list = &mut self.players[player as usize].commander_damage;
                match list.iter_mut().find(|(c, _)| *c == commander) {
                    Some(e) => e.1 = (e.1 + delta).max(0),
                    None if delta > 0 => list.push((commander, delta)),
                    None => {}
                }
            }
        }
        self.layers_dirty = true;
        Ok(())
    }

    /// Put a new token onto the battlefield under `player` (as an effect creating a token copy does: made in a staging zone,
    /// then entering, so its enters-the-battlefield abilities and the "a token entered" triggers happen).
    fn edit_token(&mut self, def: crate::ids::DefId, player: PlayerId, tapped: bool, copy_of: Option<CardId>) {
        let id = self.create_card(def, player, Zone::Exile, true);
        self.players[player as usize].exile.retain(|x| *x != id);
        self.cards[id.0 as usize].zone = Zone::Library;
        if let Some(src) = copy_of {
            self.cards[id.0 as usize].linked_card = Some(src);
        }
        self.move_card(id, Zone::Battlefield, crate::zones::MoveOpts { controller: Some(player), tapped, ..Default::default() });
        if let Some(src) = copy_of {
            // a copy of an "X/X token" keeps that size (it is the token's own characteristic)
            if let (Some((p, t)), true) = (self.cards[src.0 as usize].token_set_pt, self.cards[id.0 as usize].zone == Zone::Battlefield) {
                self.add_effect(src, player, &[id], Modif::SetPT { power: p, toughness: t }, Duration::Forever);
                self.cards[id.0 as usize].token_set_pt = Some((p, t));
            }
        }
        self.emit(Event::Token { card: id });
    }

    /// Finish a rule break: the open priority question is asked again from scratch, after state-based actions and any
    /// triggers the changes caused (the actions offered before may no longer be right).
    pub fn end_rule_break(&mut self) {
        self.edit_shown.clear();
        if let (Some(Decision { player, kind: DecisionKind::Priority { .. } }), Run::Priority { .. }) = (&self.pending, &self.run) {
            let p = *player;
            self.pending = None;
            self.run = Run::Priority { player: p, passes: 0 };
        }
        self.layers_dirty = true;
    }
}
