//! Planechase (CR 901) and Archenemy (CR 904).
//!
//! Planechase: a shared planar deck. The face-up plane(s) are in the planar controller's command zone (the active
//! player's: "you" on a plane is the planar controller), so their static abilities (`from: "Command"`) and triggered
//! abilities work like an emblem's. The planar die is a special action of the active player at sorcery speed, costing
//! {N} for the N rolls they made before it this turn: {CHAOS} triggers the planes' chaos abilities, the planeswalk face
//! planeswalks. A face-up phenomenon is planeswalked away from once its "when you encounter" ability has left the stack.
//!
//! Archenemy: the archenemy is one team, everyone else the other (players of one team are not opponents). The archenemy
//! starts at 40 life and sets the top scheme of their scheme deck in motion at the start of their precombat main phase.
//! A one-shot scheme goes to the bottom of the scheme deck once its "when you set this in motion" ability has left the
//! stack; an ongoing scheme stays until abandoned.
//!
//! Cards in the planar deck and the scheme decks are kept in the command zone but outside every zone list (face down),
//! so no game effect can see them; `Game.planar_deck` / `Player.scheme_deck` hold them, top = last.

use crate::dsl::Duration;
use crate::ids::*;
use crate::state::*;
use crate::types::*;
use rand::seq::SliceRandom;
use rand::Rng;

impl Game {
    /// Before the game begins: the shared planar deck, and the scheme decks of the archenemies (`schemes[p]` non-empty
    /// makes `p` an archenemy). Both are shuffled; the first plane is turned face up.
    pub fn setup_variants(&mut self, planar: &[DefId], schemes: &[Vec<DefId>]) {
        if !planar.is_empty() {
            let first = self.active;
            for &d in planar {
                let id = self.create_card(d, first, Zone::Command, false);
                self.players[first as usize].command.retain(|x| *x != id);
                self.planar_deck.push(id);
            }
            let mut deck = std::mem::take(&mut self.planar_deck);
            deck.shuffle(&mut self.rng);
            self.planar_deck = deck;
            // CR 901.5: a phenomenon turned up at the start goes to the bottom until a plane is face up
            for _ in 0..self.planar_deck.len() {
                let top = *self.planar_deck.last().unwrap();
                if !self.def_of(top).types.has(CardType::Phenomenon) {
                    break;
                }
                self.planar_deck.pop();
                self.planar_deck.insert(0, top);
            }
            self.turn_up_plane(first);
        }
        let archenemies: Vec<PlayerId> = schemes.iter().enumerate().filter(|(_, s)| !s.is_empty()).map(|(i, _)| i as PlayerId).collect();
        if archenemies.is_empty() {
            return;
        }
        self.archenemy = true;
        for (p, list) in schemes.iter().enumerate() {
            for &d in list {
                let id = self.create_card(d, p as PlayerId, Zone::Command, false);
                self.players[p].command.retain(|x| *x != id);
                self.players[p].scheme_deck.push(id);
            }
            let mut deck = std::mem::take(&mut self.players[p].scheme_deck);
            deck.shuffle(&mut self.rng);
            self.players[p].scheme_deck = deck;
        }
        // two teams: the archenemies, and everyone else; an archenemy starts at 40 life
        for p in 0..self.players.len() {
            let arch = archenemies.contains(&(p as PlayerId));
            self.players[p].team = if arch { 0 } else { 1 };
            if arch {
                self.players[p].life = self.players[p].life.max(40);
            }
        }
    }

    pub fn is_archenemy(&self, p: PlayerId) -> bool {
        !self.players[p as usize].scheme_deck.is_empty() || self.players[p as usize].command.iter().any(|c| self.def_of(*c).types.has(CardType::Scheme))
    }

    /// Turn over the top card of the planar deck into `p`'s command zone.
    fn turn_up_plane(&mut self, p: PlayerId) -> Option<CardId> {
        let id = self.planar_deck.pop()?;
        let c = &mut self.cards[id.0 as usize];
        c.owner = p;
        c.controller = p;
        self.players[p as usize].command.push(id);
        self.planes.push(id);
        self.planeswalked_to_this_turn.push(id);
        self.reset_derived(id);
        self.layers_dirty = true;
        Some(id)
    }

    /// A new turn: the face-up planes move to the new planar controller (the active player); planar die rolls reset.
    pub(crate) fn planechase_new_turn(&mut self) {
        for pl in &mut self.players {
            pl.planar_rolls = 0;
        }
        self.scheme_this_turn = std::mem::take(&mut self.scheme_ban_next);
        self.planeswalked_to_this_turn.clear();
        let act = self.active;
        self.fixed_point_until.retain(|p| *p != act);
        let to = self.active;
        for id in self.planes.clone() {
            let from = self.cards[id.0 as usize].owner;
            if from == to {
                continue;
            }
            self.players[from as usize].command.retain(|x| *x != id);
            self.players[to as usize].command.push(id);
            let c = &mut self.cards[id.0 as usize];
            c.owner = to;
            c.controller = to;
            self.reset_derived(id);
        }
        if !self.planes.is_empty() {
            self.layers_dirty = true;
        }
    }

    /// Planeswalk: the face-up planes go to the bottom of the planar deck and the top card is turned over.
    pub fn planeswalk(&mut self, p: PlayerId) {
        if self.planes.is_empty() && self.planar_deck.is_empty() {
            return;
        }
        self.planeswalk_away(p);
        let ctl = self.active;
        if let Some(to) = self.turn_up_plane(ctl) {
            self.emit(Event::Planeswalked { player: ctl, to });
        }
    }

    /// The first half of a planeswalk: the face-up planes leave and go to the bottom of the planar deck.
    fn planeswalk_away(&mut self, p: PlayerId) {
        let from = self.planes.clone();
        self.safe_until_planeswalk.clear();
        self.effects.retain(|e| e.duration != Duration::UntilPlaneswalk);
        if from.iter().any(|id| self.def_of(*id).types.has(CardType::Plane)) {
            // away from a plane (not a phenomenon): Chaotic Aether and Agyrem's chaos effects end
            self.chaotic_aether = false;
            self.no_attack_until_planeswalk.clear();
        }
        if !from.is_empty() {
            self.emit(Event::PlaneswalkingAway { player: p, from: from.clone() });
        }
        for id in from {
            // permanents a plane held phased out (The Doctor's Childhood Barn) phase in when it is left
            let held: Vec<CardId> = self.phased.iter().copied().filter(|h| self.cards[h.0 as usize].phase_hold.map_or(false, |(s, _)| s == id)).collect();
            for h in held {
                self.cards[h.0 as usize].phase_hold = None;
                self.phase_in(h);
            }
            let owner = self.cards[id.0 as usize].owner;
            self.players[owner as usize].command.retain(|x| *x != id);
            self.planar_deck.insert(0, id);
        }
        self.planes.clear();
    }

    /// Reveal cards from the top of the planar deck until `n` plane cards are revealed (or the deck runs out): (the planes, the
    /// other revealed cards), both in the order revealed.
    pub(crate) fn reveal_planes(&mut self, n: usize) -> (Vec<CardId>, Vec<CardId>) {
        let (mut planes, mut rest) = (vec![], vec![]);
        while planes.len() < n {
            let Some(id) = self.planar_deck.pop() else { break };
            if self.def_of(id).types.has(CardType::Plane) {
                planes.push(id);
            } else {
                rest.push(id);
            }
        }
        (planes, rest)
    }

    /// Spatial Merging: reveal until `n` planes, planeswalk to all of them at once; the other revealed cards go to the bottom.
    /// With `stay`, the face-up planes are not planeswalked away from (Norn's Seedcore).
    pub fn planeswalk_to_revealed(&mut self, p: PlayerId, n: usize, stay: bool) {
        let (planes, rest) = self.reveal_planes(n);
        if !stay {
            self.planeswalk_away(p);
        }
        for id in rest {
            self.planar_deck.insert(0, id);
        }
        let ctl = self.active;
        let mut turned = vec![];
        for id in planes {
            self.planar_deck.push(id);
            if let Some(to) = self.turn_up_plane(ctl) {
                turned.push(to);
            }
        }
        for to in turned {
            self.emit(Event::Planeswalked { player: ctl, to });
        }
    }

    /// Interplanar Tunnel, once its plane card is chosen: it goes on top of the planar deck, the rest of the revealed cards
    /// on the bottom in a random order.
    pub(crate) fn tunnel_finish(&mut self, mut others: Vec<CardId>, chosen: Option<CardId>) {
        others.shuffle(&mut self.rng);
        for id in others {
            self.planar_deck.insert(0, id);
        }
        if let Some(id) = chosen {
            self.planar_deck.push(id);
        }
    }

    /// The cost in generic mana of the next planar die roll of `p` this turn.
    pub fn planar_die_cost(&self, p: PlayerId) -> u32 {
        self.players[p as usize].planar_rolls
    }

    /// Roll the planar die (its cost already paid): one face {CHAOS}, one planeswalk, four blank.
    pub(crate) fn roll_planar_die(&mut self, p: PlayerId) {
        self.players[p as usize].planar_rolls += 1;
        let forced = {
            let mut f = self.forced_rolls.borrow_mut();
            if f.is_empty() { None } else { Some(f.remove(0)) }
        };
        let face = match forced.unwrap_or_else(|| self.rng.gen_range(1..=6)) {
            1 => PlanarFace::Chaos,
            2 if !self.fixed_point_until.is_empty() => PlanarFace::Chaos,
            2 => PlanarFace::Planeswalk,
            _ if self.chaotic_aether => PlanarFace::Chaos,
            _ => PlanarFace::Blank,
        };
        self.emit(Event::PlanarDieRolled { player: p, face });
        match face {
            PlanarFace::Chaos => self.emit(Event::ChaosEnsued { player: p }),
            PlanarFace::Planeswalk => self.planeswalk(p),
            PlanarFace::Blank => {}
        }
    }

    /// Pools of Becoming: the top `n` planar cards are revealed, each one's chaos abilities trigger, then they go to the bottom of the planar deck.
    pub(crate) fn chaos_from_planar_deck(&mut self, p: PlayerId, n: usize) {
        let top: Vec<CardId> = self.planar_deck.iter().rev().take(n).copied().collect();
        let ctl = self.active;
        for &id in &top {
            // the planar controller controls a revealed card's abilities
            let c = &mut self.cards[id.0 as usize];
            c.owner = ctl;
            c.controller = ctl;
        }
        for &id in &top {
            crate::triggers::on_event_for(self, &Event::ChaosEnsued { player: p }, Some(id));
        }
        self.planar_deck.retain(|x| !top.contains(x));
        for id in top {
            self.planar_deck.insert(0, id);
        }
    }

    /// Set the top scheme of `p`'s scheme deck in motion.
    pub fn set_scheme_in_motion(&mut self, p: PlayerId) {
        // Plots That Span Centuries: this one becomes three
        if let Some(i) = self.scheme_triple.iter().position(|x| *x == p) {
            self.scheme_triple.remove(i);
            for _ in 0..3 {
                self.set_one_scheme_in_motion(p);
            }
            return;
        }
        self.set_one_scheme_in_motion(p);
    }

    fn set_one_scheme_in_motion(&mut self, p: PlayerId) {
        let Some(id) = self.players[p as usize].scheme_deck.pop() else { return };
        let c = &mut self.cards[id.0 as usize];
        c.owner = p;
        c.controller = p;
        self.players[p as usize].command.push(id);
        self.reset_derived(id);
        self.layers_dirty = true;
        let name = self.def_of(id).name.clone();
        self.emit(Event::SchemeSetInMotion { player: p, card: id, name });
    }

    /// A scheme leaves play: face down to the bottom of its owner's scheme deck.
    pub fn abandon_scheme(&mut self, id: CardId) {
        let owner = self.cards[id.0 as usize].owner;
        if !self.players[owner as usize].command.contains(&id) {
            return;
        }
        self.players[owner as usize].command.retain(|x| *x != id);
        self.players[owner as usize].scheme_deck.insert(0, id);
        self.layers_dirty = true;
    }

    /// The start of a precombat main phase: an archenemy sets a scheme in motion (a turn-based action, CR 904.9).
    pub(crate) fn archenemy_main_phase(&mut self) {
        let p = self.active;
        if !self.players[p as usize].scheme_deck.is_empty() {
            self.set_scheme_in_motion(p);
        }
    }

    /// Variant state-based actions (CR 704.6e/f): a face-up phenomenon whose encounter ability has left the stack is
    /// planeswalked away from; a face-up one-shot scheme whose set-in-motion ability has left the stack goes to the
    /// bottom of its scheme deck. Returns whether anything happened.
    pub(crate) fn variant_sbas(&mut self) -> bool {
        // a trigger about this scheme (My Laughter Echoes: "set that scheme in motion again") keeps it face up too
        let about = |o: &Option<Obj>, id: CardId| matches!(o, Some(Obj::Card { id: x }) if *x == id);
        let pending = |g: &Game, id: CardId| g.pending_triggers.iter().any(|t| t.source == id || about(&t.ctx.obj, id)) || g.stack.iter().any(|s| s.source == id || about(&s.ctx.obj, id));
        if let Some(&ph) = self.planes.iter().find(|id| self.def_of(**id).types.has(CardType::Phenomenon)) {
            if !pending(self, ph) {
                let p = self.active;
                self.planeswalk(p);
                return true;
            }
        }
        for p in 0..self.players.len() {
            let done: Vec<CardId> = self.players[p].command.iter().copied().filter(|id| {
                let d = self.def_of(*id);
                d.types.has(CardType::Scheme) && !d.supertypes.contains(&Supertype::Ongoing) && !pending(self, *id)
            }).collect();
            if !done.is_empty() {
                for id in done {
                    self.abandon_scheme(id);
                }
                return true;
            }
        }
        false
    }
}
