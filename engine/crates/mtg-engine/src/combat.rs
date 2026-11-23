//! Combat (CR 506-511): declaring attackers and blockers, assigning and dealing combat damage.

use crate::cast::Flow;
use crate::decision::*;
use crate::dsl::*;
use crate::ids::*;
use crate::state::*;
use crate::types::*;
use smallvec::SmallVec;

#[derive(Clone, Debug)]
pub struct DamageCtx {
    pub first_strike: bool,
    /// Attackers whose damage still needs an assignment decision.
    pub queue: Vec<CardId>,
    pub done: Vec<(CardId, Obj, i32)>,
    /// Assignments from creatures that don't need a decision.
    pub current: Option<CardId>,
}

impl Game {
    // ------------------------------------------------------------------ attackers

    /// Players who goaded this creature (goads expire at the start of their next turn).
    pub fn goaded_by(&self, id: CardId) -> Vec<PlayerId> {
        let zcc = self.cards[id.0 as usize].zcc;
        self.goaded
            .iter()
            .filter(|(c, z, p, t)| *c == id && *z == zcc && !(self.turn > *t && self.active == *p) && !(*t == u32::MAX - 1 && self.cards[id.0 as usize].cur.controller == *p)
                && (*t != u32::MAX - 2 || self.goad_while_src.iter().any(|(c, z2, s, sz)| *c == id && z2 == z && self.cards[s.0 as usize].zone == Zone::Battlefield && self.cards[s.0 as usize].zcc == *sz)))
            .map(|(_, _, p, _)| *p)
            .chain(self.cards[id.0 as usize].cur.static_goaders.iter().copied())
            .collect()
    }

    /// Damage a creature assigns in combat: its power, or its toughness under "assigns combat damage equal to its toughness".
    pub fn combat_power(&self, id: CardId) -> i32 {
        let c = &self.cards[id.0 as usize];
        if c.cur.no_combat_damage {
            0
        } else if c.cur.dmg_by_toughness || (c.cur.dmg_by_toughness_gt && c.cur.toughness > c.cur.power) { c.cur.toughness.max(0) } else if c.cur.dmg_abs_power { c.cur.power.abs() } else { c.cur.power.max(0) }
    }

    pub fn can_attack(&self, id: CardId, p: PlayerId) -> bool {
        let c = &self.cards[id.0 as usize];
        c.zone == Zone::Battlefield
            && c.cur.controller == p
            && c.cur.types.has(CardType::Creature)
            && !c.tapped
            && !c.cur.cant_attack
            && (!c.cur.kw.has(Keyword::Defender) || c.cur.attack_no_defender || c.cur.attack_nd_vs_attackers)
            && !c.cur.kw.has(Keyword::CantAttack)
            && (self.can_attack_unsick(id) || c.cur.attack_haste)
    }

    /// The player who defends this permanent when it is attacked: a battle's protector, else its controller.
    pub fn defender_side(&self, id: CardId) -> PlayerId {
        let c = &self.cards[id.0 as usize];
        if c.cur.types.has(CardType::Battle) {
            if let Some(p) = c.protector {
                return p;
            }
        }
        c.cur.controller
    }

    pub fn attacker_options(&self, p: PlayerId) -> Vec<AttackerOpt> {
        if self.no_attack_until >= self.turn && self.no_attack_until != 0 {
            return vec![];
        }
        if self.effects.iter().any(|e| matches!(e.modif, Modif::NoAttack) && e.players.contains(&p)) {
            return vec![];
        }
        let players: Vec<PlayerId> = self.opponents(p);
        let pws: Vec<CardId> = self
            .battlefield
            .iter()
            .copied()
            .filter(|id| {
                let c = &self.cards[id.0 as usize];
                self.defender_side(*id) != p
                    && self.alive(self.defender_side(*id))
                    && (c.cur.types.has(CardType::Planeswalker) || c.cur.types.has(CardType::Battle))
                    && (c.cur.no_abilities || {
                        let sd = self.db.def(c.def);
                        !sd.statics.iter().any(|&i| match &sd.abilities[i as usize] {
                            Ability::Static { effect: StaticEffect::CantBeAttacked, condition, .. } => condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &crate::eval::EvalCtx::with_controller(*id, c.cur.controller))),
                            _ => false,
                        })
                    })
            })
            .collect();
        // schemes, emblems and other command-zone objects whose attack restrictions work from there (Bow to My Command)
        let command_sources: Vec<CardId> = self
            .players
            .iter()
            .flat_map(|pl| pl.command.iter().copied())
            .filter(|g| {
                let d = self.db.def(self.cards[g.0 as usize].def);
                d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { from: Some(Zone::Command), .. }))
            })
            .collect();
        self.battlefield
            .iter()
            .copied()
            .filter(|id| self.can_attack(*id, p))
            .filter_map(|id| {
                let mut restr: Vec<(Filter, bool, CardId, PlayerId, bool)> = self.cards[id.0 as usize].cur.attack_restr.to_vec();
                let mut not_owner = false;
                let mut bar_all: Vec<PlayerId> = vec![];
                let mut bar_pl: Vec<PlayerId> = vec![];
                for (q, po) in self.cards[id.0 as usize].cur.attack_bar.iter() {
                    if *po { bar_pl.push(*q) } else { bar_all.push(*q) }
                }
                let mut bar_again = false;
                for &sid in self.battlefield.iter().chain(command_sources.iter()) {
                    let sc = &self.cards[sid.0 as usize];
                    if sc.cur.no_abilities {
                        continue;
                    }
                    let sd = self.db.def(sc.def);
                    let granted = sc.cur.granted.iter().map(|g| &**g);
                    let in_command = sc.zone == Zone::Command;
                    for a in sd.statics.iter().map(|&i| &sd.abilities[i as usize]).chain(granted) {
                        if in_command && !matches!(a, Ability::Static { from: Some(Zone::Command), .. }) {
                            continue;
                        }
                        if let Ability::Static { effect: StaticEffect::CantAttackOwner { affects }, condition, .. } = a {
                            if condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &crate::eval::EvalCtx::with_controller(sid, sc.cur.controller))) && self.matches_card(affects, id, sid, sc.cur.controller) {
                                not_owner = true;
                            }
                        }
                        if let Ability::Static { effect: StaticEffect::AttackOnlyNearestInDirection, condition, .. } = a {
                            if condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &crate::eval::EvalCtx::with_controller(sid, sc.cur.controller))) {
                                let right = sc.chosen_type.map_or(false, |t| t.name() == "Right");
                                let mut nearest = p;
                                for _ in 0..self.players.len() {
                                    nearest = if right { self.previous_player(nearest) } else { self.next_player(nearest) };
                                    if nearest != p {
                                        break;
                                    }
                                }
                                for q in 0..self.players.len() as PlayerId {
                                    if q != p && q != nearest {
                                        bar_all.push(q);
                                    }
                                }
                            }
                        }
                        if let Ability::Static { effect: StaticEffect::CantAttackPlayerAgain { affects }, condition, .. } = a {
                            if condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &crate::eval::EvalCtx::with_controller(sid, sc.cur.controller))) && self.matches_card(affects, id, sid, sc.cur.controller) {
                                bar_again = true;
                            }
                        }
                        if let Ability::Static { effect: StaticEffect::CantAttackController { affects, players_only }, condition, .. } = a {
                            if condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &crate::eval::EvalCtx::with_controller(sid, sc.cur.controller))) && self.matches_card(affects, id, sid, sc.cur.controller) {
                                if *players_only { bar_pl.push(sc.cur.controller) } else { bar_all.push(sc.cur.controller) }
                            }
                        }
                        if let Ability::Static { effect: StaticEffect::CantAttackPlayer { affects, defender_controls, if_controls, players_only }, condition, .. } = a {
                            if condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &crate::eval::EvalCtx::with_controller(sid, sc.cur.controller))) && self.matches_card(affects, id, sid, sc.cur.controller) {
                                restr.push((defender_controls.clone().into_filter(), *if_controls, sid, sc.cur.controller, *players_only));
                            }
                        }
                    }
                }
                for &(_, right) in &self.attack_nearest {
                    let mut nearest = p;
                    for _ in 0..self.players.len() {
                        nearest = if right { self.previous_player(nearest) } else { self.next_player(nearest) };
                        if nearest != p {
                            break;
                        }
                    }
                    for q in 0..self.players.len() as PlayerId {
                        if q != p && q != nearest {
                            bar_all.push(q);
                        }
                    }
                }
                let restr = &restr;
                let dconds: Vec<(Condition, CardId)> = self.cards[id.0 as usize].cur.defender_conds.iter().filter(|c| !c.1).map(|c| (c.0.clone(), c.2)).collect();
                let cond_ok = |def: PlayerId| dconds.iter().all(|(cd, csrc)| !self.eval_cond(cd, &crate::eval::EvalCtx::with_controller(*csrc, def)));
                let owner = self.cards[id.0 as usize].owner;
                let cc = &self.cards[id.0 as usize];
                let again = |q: PlayerId| bar_again && cc.attacked_players_turn == self.turn && cc.attacked_players & (1u8 << (q & 7)) != 0;
                let next_bar = |q: PlayerId| self.no_attack_next.iter().any(|e| e.2 && e.0 == p && e.1 == q) || self.no_attack_pending.contains(&(p, q));
                let cond_ok_pw = |def: PlayerId| dconds.iter().all(|(cd, csrc)| matches!(cd, Condition::PlayerAttackOnly { .. }) || !self.eval_cond(cd, &crate::eval::EvalCtx::with_controller(*csrc, def)));
                let sanct_bar = !(cc.cur.kw.has(Keyword::Flying) || cc.cur.kw.has(Keyword::Islandwalk));
                let bar_p = |q: PlayerId| next_bar(q) || (sanct_bar && self.sanctuary.contains(&q)) || again(q) || self.no_attack_pairs.contains(&(p, q)) || self.players[q as usize].shield_bar || (not_owner && q == owner) || bar_all.contains(&q) || bar_pl.contains(&q) || (self.no_attack_until_planeswalk.contains(&q) && p != q);
                let bar_w = |q: PlayerId| next_bar(q) || self.no_attack_pw_pairs.contains(&(p, q)) || self.players[q as usize].shield_bar || (not_owner && q == owner) || bar_all.contains(&q);
                let bar_perm = |w: CardId| self.no_attack_perms.iter().any(|(a, ctl, src, f)| *a == p && self.matches_card(f, w, *src, *ctl));
                if restr.is_empty() && dconds.is_empty() {
                    if true {
                        let pl: Vec<PlayerId> = players.iter().copied().filter(|q| !bar_p(*q)).collect();
                        let pw: Vec<CardId> = pws.iter().copied().filter(|w| !bar_w(self.defender_side(*w)) && !bar_perm(*w)).collect();
                        return (!pl.is_empty() || !pw.is_empty()).then(|| AttackerOpt { card: id, players: pl, permanents: pw });
                    }
                    return Some(AttackerOpt { card: id, players: players.clone(), permanents: pws.clone() });
                }
                // "can't attack unless / if defending player controls ...": judged per attacked player
                let ok_for = |def: PlayerId| {
                    restr.iter().all(|(f, neg, fsrc, fctrl, _)| {
                        let has = self.battlefield.iter().any(|b| self.cards[b.0 as usize].cur.controller == def && self.matches_card_ignore_controller(f, *b, *fsrc, *fctrl));
                        has != *neg
                    })
                };
                let pl: Vec<PlayerId> = players.iter().copied().filter(|q| ok_for(*q) && cond_ok(*q) && !bar_p(*q)).collect();
                let ok_pw = |def: PlayerId| {
                    restr.iter().all(|(f, neg, fsrc, fctrl, po)| {
                        *po || {
                            let has = self.battlefield.iter().any(|b| self.cards[b.0 as usize].cur.controller == def && self.matches_card_ignore_controller(f, *b, *fsrc, *fctrl));
                            has != *neg
                        }
                    })
                };
                let pw: Vec<CardId> = pws.iter().copied().filter(|w| ok_pw(self.defender_side(*w)) && cond_ok_pw(self.defender_side(*w)) && !bar_w(self.defender_side(*w)) && !bar_perm(*w)).collect();
                (!pl.is_empty() || !pw.is_empty()).then(|| AttackerOpt { card: id, players: pl, permanents: pw })
            })
            .filter_map(|mut o| {
                let c = &self.cards[o.card.0 as usize];
                if c.cur.kw.has(Keyword::Defender) && !c.cur.attack_no_defender && c.cur.attack_nd_vs_attackers {
                    // only players who attacked this creature's controller during their last turn
                    o.players.retain(|q| self.players[*q as usize].attacked_last_turn & (1u8 << (p & 7)) != 0);
                    o.permanents.clear();
                    if o.players.is_empty() {
                        return None;
                    }
                }
                Some(o)
            })
            .collect()
    }

    /// The player a creature must attack if able: a granted `MustAttackPlayer`, or the player whose `Taunt` is in force this turn.
    pub(crate) fn must_attack_pl(&self, card: CardId) -> Option<PlayerId> {
        let c = &self.cards[card.0 as usize];
        c.cur.must_attack_player.or_else(|| self.taunts.iter().find(|t| t.0 == c.cur.controller && t.2 == self.turn && t.1 != t.0).map(|t| t.1))
    }

    /// Does a creature have to attack this turn if able for a reason other than its own state: `Effect::AttackAllThisTurn` (Rowan Kenrith)?
    pub(crate) fn must_attack_all(&self, card: CardId) -> bool {
        let ctl = self.cards[card.0 as usize].cur.controller;
        self.attack_all.iter().any(|(p, t)| *p == ctl && *t == self.turn)
    }

    pub(crate) fn step_attackers(&mut self) -> Flow {
        let p = self.active;
        let options = self.attacker_options(p);
        if options.is_empty() {
            self.finish_attackers(vec![]);
            return Flow::Continue;
        }
        if let Some(Choice::Attackers { attacks }) = self.answer.take() {
            let requested: Vec<(CardId, Obj)> = attacks.iter().copied().filter(|(c, _)| (c.0 as usize) < self.cards.len()).collect();
            let mut valid: Vec<(CardId, Obj)> = vec![];
            for (card, def) in attacks {
                if let Some(o) = options.iter().find(|o| o.card == card) {
                    let ok = match def {
                        Obj::Player { id } => o.players.contains(&id),
                        Obj::Card { id } => o.permanents.contains(&id),
                        _ => false,
                    };
                    if ok && !valid.iter().any(|(c, _)| *c == card) {
                        // a goaded creature can't attack the player who goaded it while another attack is possible
                        let def = match def {
                            Obj::Player { id } if self.goaded_by(card).contains(&id) => o.players.iter().copied().find(|q| !self.goaded_by(card).contains(q)).map_or(def, Obj::player),
                            d => d,
                        };
                        // a creature that must attack a given player attacks that player if it can
                        let def = match self.cards[card.0 as usize].cur.must_attack_pw {
                            Some(pw) if o.permanents.contains(&pw) => Obj::Card { id: pw },
                            _ => def,
                        };
                        let def = match self.must_attack_pl(card) {
                            Some(mp) if o.players.contains(&mp) => Obj::player(mp),
                            _ => def,
                        };
                        valid.push((card, def));
                    }
                }
            }
            // creatures that must attack a specific planeswalker
            for o in &options {
                if let Some(pw) = self.cards[o.card.0 as usize].cur.must_attack_pw {
                    if o.permanents.contains(&pw) && !valid.iter().any(|(c, _)| *c == o.card) {
                        valid.push((o.card, Obj::Card { id: pw }));
                    }
                }
            }
            // creatures that must attack a specific player
            for o in &options {
                if let Some(mp) = self.must_attack_pl(o.card) {
                    if o.players.contains(&mp) && !valid.iter().any(|(c, _)| *c == o.card) {
                        valid.push((o.card, Obj::player(mp)));
                    }
                }
            }
            // creatures that must attack
            for o in &options {
                if (self.cards[o.card.0 as usize].cur.must_attack || self.cards[o.card.0 as usize].cur.kw.has(Keyword::MustAttack) || self.must_attack_all(o.card) || !self.goaded_by(o.card).is_empty()) && !valid.iter().any(|(c, _)| *c == o.card) {
                    let goaders = self.goaded_by(o.card);
                    if let Some(&pl) = o.players.iter().find(|q| !goaders.contains(q)).or(o.players.first()) {
                        // a creature that must attack but whose attack costs something isn't forced to pay: leaving it home satisfies the requirement as far as possible (CR 508.1d)
                        let costs = !self.combat_costs(o.card, true, Some(pl), None).is_empty() || self.attack_tax(pl, true) > 0;
                        if !costs {
                            valid.push((o.card, Obj::player(pl)));
                        }
                    }
                }
            }
            // "must attack with at least one creature if able"
            if valid.is_empty() && self.must_attack_one(p) {
                if let Some(o) = options.iter().find(|o| !o.players.is_empty()) {
                    valid.push((o.card, Obj::player(o.players[0])));
                }
            }
            // "If a creature attacks, all creatures that player controls attack if able"
            if !valid.is_empty() && self.must_attack_all_if_any(p) {
                let first = match valid[0].1 {
                    Obj::Player { id } => Some(id),
                    Obj::Card { id } => Some(self.defender_side(id)),
                    _ => None,
                };
                for o in &options {
                    if !valid.iter().any(|(c, _)| *c == o.card) {
                        let pl = first.filter(|f| o.players.contains(f)).or(o.players.first().copied());
                        if let Some(pl) = pl {
                            valid.push((o.card, Obj::player(pl)));
                        }
                    }
                }
            }
            // "If a creature with a magnet counter attacks, all creatures with magnet counters attack if able"
            if !valid.is_empty() {
                for k in self.counter_attack_kinds() {
                    let first = valid.iter().find(|(c, _)| self.cards[c.0 as usize].counter(k) > 0).map(|(_, d)| match d {
                        Obj::Player { id } => *id,
                        Obj::Card { id } => self.defender_side(*id),
                        _ => 0,
                    });
                    if let Some(first) = first {
                        for o in &options {
                            if self.cards[o.card.0 as usize].counter(k) > 0 && !valid.iter().any(|(c, _)| *c == o.card) {
                                let pl = Some(first).filter(|f| o.players.contains(f)).or(o.players.first().copied());
                                if let Some(pl) = pl {
                                    valid.push((o.card, Obj::player(pl)));
                                }
                            }
                        }
                    }
                }
            }
            // "If a creature you control attacks, this creature also attacks if able"
            if !valid.is_empty() {
                let first = match valid[0].1 {
                    Obj::Player { id } => Some(id),
                    Obj::Card { id } => Some(self.defender_side(id)),
                    _ => None,
                };
                for o in &options {
                    if !valid.iter().any(|(c, _)| *c == o.card) && self.attacks_if_any_attacks(o.card) {
                        let pl = first.filter(|f| o.players.contains(f)).or(o.players.first().copied());
                        if let Some(pl) = pl {
                            valid.push((o.card, Obj::player(pl)));
                        }
                    }
                }
            }
            // "No more than N creatures can attack (you) each combat": later attackers are dropped
            for (src, ctrl, who, n) in self.attack_limits() {
                let mut kept = 0u8;
                valid.retain(|(_, def)| {
                    let dp = match def {
                        Obj::Player { id } => Some(*id),
                        // "attack you" (Crawlspace) limits attacks on the player only, not on their planeswalkers
                        Obj::Card { id } if who == PlayerFilter::Any => Some(self.defender_side(*id)),
                        _ => None,
                    };
                    if dp.map_or(false, |dp| self.player_filter_src(src, who, dp, ctrl)) {
                        kept += 1;
                        kept <= n
                    } else {
                        true
                    }
                });
            }
            // "No more than N creatures can attack this planeswalker each combat"
            {
                let mut counts: Vec<(CardId, u8)> = vec![];
                valid.retain(|(_, def)| {
                    let Obj::Card { id } = def else { return true };
                    let max = self.cards[id.0 as usize].cur.max_attackers;
                    if max == 0 {
                        return true;
                    }
                    match counts.iter_mut().find(|(c, _)| c == id) {
                        Some((_, k)) => {
                            *k += 1;
                            *k <= max
                        }
                        None => {
                            counts.push((*id, 1));
                            true
                        }
                    }
                });
            }
            // "can't attack unless another creature matching X also attacks"
            loop {
                let before = valid.len();
                let snapshot: Vec<CardId> = valid.iter().map(|(c, _)| *c).collect();
                valid.retain(|(c, _)| {
                    self.attack_needs_other(*c).iter().all(|f| {
                        snapshot.iter().any(|o| *o != *c && self.matches_card(f, *o, *c, p))
                    })
                });
                if valid.len() == before {
                    break;
                }
            }
            // "can't attack unless a creature with greater power also attacks"
            loop {
                let before = valid.len();
                let pw: Vec<(CardId, i32)> = valid.iter().map(|(c, _)| (*c, self.cards[c.0 as usize].cur.power)).collect();
                valid.retain(|(c, _)| {
                    let cc = &self.cards[c.0 as usize].cur;
                    !cc.cant_attack_unless_greater || pw.iter().any(|(o, op)| *o != *c && *op > cc.power)
                });
                if valid.len() == before {
                    break;
                }
            }
            // "can't attack unless at least N other creatures attack"
            loop {
                let before = valid.len();
                let n = valid.len();
                valid.retain(|(c, _)| (self.cards[c.0 as usize].cur.attack_needs_others as usize) < n);
                if valid.len() == before {
                    break;
                }
            }
            let required = |c: CardId| {
                let cc = &self.cards[c.0 as usize].cur;
                cc.must_attack || cc.kw.has(Keyword::MustAttack) || self.must_attack_all(c) || cc.must_attack_pw.is_some() || self.must_attack_pl(c).is_some() || !self.goaded_by(c).is_empty()
            };
            // "can't attack alone": a lone attacker with the restriction doesn't attack, unless it must attack and another
            // creature can join it for free (CR 508.1d: attacking with both obeys the requirement and breaks no restriction:
            // Mogg Flunkies enchanted with Furor of the Bitten)
            if valid.len() == 1 && self.cards[valid[0].0 .0 as usize].cur.cant_attack_alone {
                let (lone, def) = valid[0];
                let lone_pl = match def {
                    Obj::Player { id } => Some(id),
                    Obj::Card { id } => Some(self.defender_side(id)),
                    _ => None,
                };
                let partner = if required(lone) && self.attack_limits().is_empty() {
                    options.iter().filter(|o| o.card != lone && !o.players.is_empty()).find_map(|o| {
                        let cc = &self.cards[o.card.0 as usize].cur;
                        let pl = lone_pl.filter(|q| o.players.contains(q)).unwrap_or(o.players[0]);
                        let free = self.combat_costs(o.card, true, Some(pl), None).is_empty() && self.attack_tax(pl, true) == 0;
                        let unrestricted = !cc.attack_only_alone && cc.attack_needs_others <= 1 && !cc.cant_attack_unless_greater && self.attack_needs_other(o.card).is_empty();
                        (free && unrestricted).then_some((o.card, Obj::player(pl)))
                    })
                } else {
                    None
                };
                match partner {
                    Some(x) => valid.push(x),
                    None => valid.clear(),
                }
            }
            // "can attack only alone": the other attackers are dropped, unless one of them must attack (CR 508.1d: obeying
            // that requirement wins, so the creature that may only attack alone stays home: Master of Cruelties + Juggernaut)
            if valid.len() > 1 {
                if let Some(lone) = valid.iter().position(|(c, _)| self.cards[c.0 as usize].cur.attack_only_alone) {
                    if valid.iter().any(|(c, _)| !self.cards[c.0 as usize].cur.attack_only_alone && required(*c)) {
                        valid.retain(|(c, _)| !self.cards[c.0 as usize].cur.attack_only_alone);
                    } else {
                        let keep = valid.remove(lone);
                        valid.clear();
                        valid.push(keep);
                    }
                }
            }
            self.note_attack_changes(p, &requested, &options, &valid);
            self.finish_attackers(valid);
            return Flow::Continue;
        }
        let must: Vec<CardId> = options.iter().filter(|o| self.cards[o.card.0 as usize].cur.must_attack || self.must_attack_all(o.card) || self.cards[o.card.0 as usize].cur.must_attack_pw.map_or(false, |pw| o.permanents.contains(&pw)) || self.must_attack_pl(o.card).map_or(false, |mp| o.players.contains(&mp)) || self.cards[o.card.0 as usize].cur.kw.has(Keyword::MustAttack) || !self.goaded_by(o.card).is_empty()).map(|o| o.card).collect();
        let decider = self.attack_chooser.unwrap_or(p);
        self.pending = Some(Decision { player: decider, kind: DecisionKind::Attackers { options, must_attack: must } });
        self.run = Run::Attackers;
        Flow::Wait
    }

    fn finish_attackers(&mut self, attacks: Vec<(CardId, Obj)>) {
        let p = self.active;
        let mut attacks = attacks;
        // "If this creature attacks, all creatures you control attack if able" (Viashino Bey): once such a creature is declared, every able creature joins
        if attacks.iter().any(|(c, _)| {
            let d = self.def_of(*c);
            d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::MustAttack { .. }, condition: Some(Condition::Control { filter, .. }), .. } if filter.this && filter.attacking))
        }) {
            for o in self.attacker_options(p) {
                if !attacks.iter().any(|(c, _)| *c == o.card) {
                    if let Some(&q) = o.players.first() {
                        attacks.push((o.card, Obj::player(q)));
                    }
                }
            }
        }
        // attack taxes (Propaganda): pay for each attacker or it doesn't attack
        let mut i = 0;
        while i < attacks.len() {
            let defender = match attacks[i].1 {
                Obj::Player { id } => Some(id),
                Obj::Card { id } => Some(self.defender_side(id)),
                _ => None,
            };
            let at_player = matches!(attacks[i].1, Obj::Player { .. });
            let tax = defender.map_or(0, |d| self.attack_tax(d, at_player));
            let own_costs = self.combat_costs(attacks[i].0, true, defender, None);
            if !own_costs.is_empty() && !self.try_pay_costs(p, attacks[i].0, &own_costs) {
                attacks.remove(i);
                continue;
            }
            if tax > 0 {
                let src = attacks[i].0;
                let cost = vec![crate::dsl::Cost::Mana { cost: format!("{{{tax}}}") }];
                if !self.try_pay_costs(p, src, &cost) {
                    attacks.remove(i);
                    continue;
                }
            }
            i += 1;
        }
        if attacks.is_empty() {
            if self.delayed.iter().any(|d| d.when == DelayedWhen::NextYourDeclareAttackers && d.controller == p) {
                // a creature is due to return attacking at the beginning of this step: keep the step, with no attackers yet
                self.combat = Some(Combat::default());
                self.fs_dealers.clear();
                self.fire_delayed(DelayedWhen::NextYourDeclareAttackers);
                self.run = Run::Priority { player: p, passes: 0 };
                return;
            }
            self.combat = None;
            // no attackers: skip to the end of combat
            self.step = Step::EndCombat;
            self.run = Run::BeginStep;
            return;
        }
        let mut cb = Combat::default();
        for (card, def) in &attacks {
            cb.attackers.push(AttackInfo { card: *card, defender: *def, blocked: false, blockers: SmallVec::new(), band: 0 });
        }
        // banding: any number of attackers with banding plus at most one without may attack as a band (same defender)
        let wanted = std::mem::take(&mut self.attack_bands);
        let mut next_band = 1u8;
        for band in wanted {
            let members: Vec<usize> = band.iter().filter_map(|c| cb.attackers.iter().position(|a| a.card == *c && a.band == 0)).collect();
            if members.len() < 2 || members.len() != band.len() {
                continue;
            }
            let non_banding = members.iter().filter(|&&i| !self.cards[cb.attackers[i].card.0 as usize].cur.kw.has(Keyword::Banding)).count();
            let same_def = members.iter().all(|&i| cb.attackers[i].defender == cb.attackers[members[0]].defender);
            let legendary_band = members.iter().all(|&i| self.cards[cb.attackers[i].card.0 as usize].cur.supertypes.contains(&Supertype::Legendary))
                && members.iter().any(|&i| self.cards[cb.attackers[i].card.0 as usize].cur.kw.has(Keyword::BandsWithLegendary));
            let same_name_band = members.iter().any(|&i| self.cards[cb.attackers[i].card.0 as usize].cur.kw.has(Keyword::BandsWithSameName))
                && members.iter().all(|&i| self.view_name(&self.view(cb.attackers[i].card)) == self.view_name(&self.view(cb.attackers[members[0]].card)));
            if same_def && ((non_banding <= 1 && members.iter().any(|&i| self.cards[cb.attackers[i].card.0 as usize].cur.kw.has(Keyword::Banding))) || legendary_band || same_name_band) {
                for &i in &members {
                    cb.attackers[i].band = next_band;
                }
                next_band += 1;
            }
        }
        self.fs_dealers.clear();
        self.combat = Some(cb);
        self.layers_dirty = true;
        self.players[p as usize].attacked_turn = true;
        self.batch_seen.clear();
        self.batch_seen_pw.clear();
        // "has flying as long as it's attacking" must already hold when attack triggers look at the attacker (Windreader Sphinx)
        self.recompute_layers();
        for (card, def) in attacks {
            let no_tap = self.effects.iter().any(|e| matches!(&e.modif, Modif::DynStatic(b) if matches!(&**b, StaticEffect::AttackersDontTap)) && e.controller == p && self.cards[e.source.0 as usize].zone == Zone::Battlefield && !self.cards[e.source.0 as usize].tapped)
                && self.cards[card.0 as usize].cur.controller == p;
            if !no_tap && !self.cards[card.0 as usize].cur.kw.has(Keyword::Vigilance) {
                self.declaring_attacker.set(true);
                self.tap_card(card);
                self.declaring_attacker.set(false);
            }
            self.cards[card.0 as usize].attacked_turn = self.turn;
            {
                let forced = {
                    let c = &self.cards[card.0 as usize];
                    c.cur.must_attack || self.must_attack_all(card) || c.cur.kw.has(Keyword::MustAttack) || !self.goaded_by(card).is_empty() || c.cur.must_attack_pw.is_some() || self.must_attack_pl(card).is_some()
                };
                self.cards[card.0 as usize].had_to_attack_turn = if forced { self.turn } else { 0 };
            }
            {
                let c = &mut self.cards[card.0 as usize];
                if c.attack_count_turn != self.turn {
                    c.attack_count_turn = self.turn;
                    c.attack_count = 0;
                }
                c.attack_count = c.attack_count.saturating_add(1);
            }
            if let Obj::Player { id } = def {
                let ac = self.cards[card.0 as usize].cur.controller;
                self.players[ac as usize].attacked_last_turn |= 1u8 << (id & 7);
                let c = &mut self.cards[card.0 as usize];
                if c.attacked_players_turn != self.turn {
                    c.attacked_players_turn = self.turn;
                    c.attacked_players = 0;
                }
                c.attacked_players |= 1u8 << (id & 7);
            }
            if let Obj::Card { id } = def {
                if self.cards[id.0 as usize].cur.types.has(CardType::Battle) {
                    self.cards[card.0 as usize].attacked_battle_turn = self.turn;
                }
            }
            self.cards[card.0 as usize].attacked_combat = self.players[self.cards[card.0 as usize].cur.controller as usize].combat_serial;
            self.cards[card.0 as usize].attacked_player = match def {
                Obj::Player { id } => Some(id),
                Obj::Card { id } => Some(self.cards[id.0 as usize].cur.controller),
                _ => None,
            };
            let actl = self.cards[card.0 as usize].cur.controller;
            let (subs, ch) = {
                let c = &self.cards[card.0 as usize];
                (c.cur.subtypes.to_vec(), c.cur.kw.has(Keyword::Changeling))
            };
            // attacking a planeswalker or battle does not count as attacking its controller (Ramses, Assassin Lord)
            if let (Some(dp), Obj::Player { .. }) = (self.cards[card.0 as usize].attacked_player, def) {
                self.defended_log.push((actl, dp, subs.clone(), ch));
            }
            if self.cards[card.0 as usize].token {
                self.token_attackers_log.push(actl);
            }
            if self.cards[card.0 as usize].is_commander {
                self.commander_attackers_log.push(actl);
            }
            self.attackers_log.push((actl, subs, ch));
            self.emit(Event::Attacked { card, defender: def });
        }
        self.fire_delayed(DelayedWhen::NextYourDeclareAttackers);
        self.run = Run::Priority { player: p, passes: 0 };
    }

    /// A token or creature put onto the battlefield attacking.
    pub fn add_attacker_late(&mut self, id: CardId, controller: PlayerId) {
        let fallback = Obj::player(self.next_player(controller));
        let Some(cb) = self.combat.as_mut() else { return };
        let defender = cb.attackers.first().map(|a| a.defender).unwrap_or(fallback);
        cb.attackers.push(AttackInfo { card: id, defender, blocked: false, blockers: SmallVec::new(), band: 0 });
    }

    /// Like `add_attacker_late`, but attacking a given defender.
    pub fn add_attacker_at(&mut self, id: CardId, defender: Obj) {
        let Some(cb) = self.combat.as_mut() else { return };
        cb.attackers.push(AttackInfo { card: id, defender, blocked: false, blockers: SmallVec::new(), band: 0 });
    }

    // ------------------------------------------------------------------ blockers

    fn defending_players(&self) -> Vec<PlayerId> {
        let Some(cb) = &self.combat else { return vec![] };
        let mut v: Vec<PlayerId> = vec![];
        for a in &cb.attackers {
            let p = match a.defender {
                Obj::Player { id } => id,
                Obj::Card { id } => self.defender_side(id),
                _ => continue,
            };
            if !v.contains(&p) && self.alive(p) {
                v.push(p);
            }
        }
        // APNAP
        let order = self.apnap();
        v.sort_by_key(|p| order.iter().position(|q| q == p).unwrap_or(99));
        v
    }

    pub fn can_block_attacker(&self, blocker: CardId, attacker: CardId) -> bool {
        let b = &self.cards[blocker.0 as usize];
        let a = &self.cards[attacker.0 as usize];
        if (b.tapped && !self.can_block_while_tapped(blocker)) || !b.cur.types.has(CardType::Creature) || b.cur.cant_block || b.cur.kw.has(Keyword::CantBlock) || b.cur.kw.has(Keyword::Decayed) {
            return false;
        }
        if b.cur.attacker_conds.iter().any(|(cd, csrc)| self.eval_cond(cd, &crate::eval::EvalCtx::with_controller(*csrc, a.cur.controller))) {
            return false;
        }
        if b.cur.cant_block_attackers.iter().any(|(flt, src, ctl)| self.matches_card(flt, attacker, *src, *ctl)) {
            return false;
        }
        if self.sector_block_turn == self.turn && self.turn > 0 && a.sector != b.sector {
            return false;
        }
        let ak = a.cur.kw;
        if ak.has(Keyword::Unblockable) {
            return false;
        }
        if ak.has(Keyword::Flying) && !(b.cur.kw.has(Keyword::Flying) || b.cur.kw.has(Keyword::Reach) || self.blocks_as_reach_vs(blocker, attacker)) {
            return false;
        }
        if ak.has(Keyword::Shadow) != b.cur.kw.has(Keyword::Shadow) && !(b.cur.can_block_shadow && ak.has(Keyword::Shadow)) {
            return false;
        }
        if ak.has(Keyword::Horsemanship) && !b.cur.kw.has(Keyword::Horsemanship) {
            return false;
        }
        if ak.has(Keyword::Fear) && !(b.cur.colors.has(Color::B) || b.cur.types.has(CardType::Artifact)) {
            return false;
        }
        if ak.has(Keyword::Intimidate) && !(b.cur.types.has(CardType::Artifact) || b.cur.colors.0 & a.cur.colors.0 != 0) {
            return false;
        }
        if self.is_ring_bearer(attacker) && b.cur.power > a.cur.power {
            return false;
        }
        if ak.has(Keyword::Skulk) && b.cur.power > a.cur.power {
            return false;
        }
        if a.cur.ignore_landwalk != 31 && !b.cur.can_block_landwalk && !a.cur.extra_landwalk.is_empty() {
            let defender = b.cur.controller;
            if a.cur.extra_landwalk.iter().any(|t| self.battlefield.iter().any(|id| {
                let l = &self.cards[id.0 as usize];
                l.cur.controller == defender && l.cur.subtypes.contains(t)
            })) {
                return false;
            }
        }
        for (kw, land) in [(Keyword::Plainswalk, "Plains"), (Keyword::Islandwalk, "Island"), (Keyword::Swampwalk, "Swamp"), (Keyword::Mountainwalk, "Mountain"), (Keyword::Forestwalk, "Forest")] {
            if ak.has(kw) && !b.cur.can_block_landwalk && a.cur.ignore_landwalk & crate::layers::landwalk_bit(Some(kw)) == 0 {
                let defender = b.cur.controller;
                let sym = Sym::new(land);
                if self.battlefield.iter().any(|id| {
                    let l = &self.cards[id.0 as usize];
                    l.cur.controller == defender && l.cur.subtypes.contains(&sym)
                }) {
                    return false;
                }
            }
        }
        for (cd, unblockable, csrc, cctrl) in &a.cur.defender_conds {
            if *unblockable && self.eval_cond(cd, &crate::eval::EvalCtx::with_controller(*csrc, b.cur.controller)) {
                let _ = cctrl;
                return false;
            }
        }
        if a.cur.cant_be_blocked_greater && b.cur.power > a.cur.power {
            return false;
        }
        for (f, fsrc, fctrl) in &a.cur.cant_be_blocked_by {
            if self.matches_card(f, blocker, *fsrc, *fctrl) {
                return false;
            }
        }
        for pr in &a.cur.protections {
            let hit = match pr {
                ProtectionFrom::Color { c } => b.cur.colors.has(*c),
                ProtectionFrom::Type { t } => b.cur.types.has(*t),
                ProtectionFrom::Everything => true,
                ProtectionFrom::Abilities => false,
                ProtectionFrom::Colorless => b.cur.colors.count() == 0,
                ProtectionFrom::Monocolored => b.cur.colors.count() == 1,
            ProtectionFrom::NotColor { c } => !b.cur.colors.has(*c),
            ProtectionFrom::NonSubtype { name } => b.cur.types.has(CardType::Creature) && !b.cur.subtypes.contains(&Sym::new(name)) && !b.cur.kw.has(Keyword::Changeling),
                ProtectionFrom::Multicolored => b.cur.colors.count() > 1,
                ProtectionFrom::Subtype { name } => b.cur.subtypes.contains(&Sym::new(name)) || (b.cur.kw.has(Keyword::Changeling) && b.cur.types.has(CardType::Creature)),
                ProtectionFrom::ChosenPlayer => a.chosen_player == Some(b.cur.controller),
                ProtectionFrom::Opponents => b.cur.controller != a.cur.controller,
                ProtectionFrom::ControlledBy { p, .. } => *p == Some(b.cur.controller),
                ProtectionFrom::ChosenColor | ProtectionFrom::ChosenType | ProtectionFrom::Matching { .. } | ProtectionFrom::ArtifactManaValues => self.chosen_prot_hit(a, b, pr),
            };
            if hit {
                return false;
            }
        }
        true
    }

    pub(crate) fn step_blockers(&mut self, idx: usize) -> Flow {
        // pumps and other effects from the declare attackers step must be applied before "can't be blocked by power N or less" is judged
        if self.layers_dirty {
            self.recompute_layers();
        }
        let defenders = self.defending_players();
        if idx >= defenders.len() {
            // finished: blockers are set, give priority
            self.after_blocks();
            return Flow::Continue;
        }
        let p = defenders[idx];
        let attackers: Vec<CardId> = self
            .combat
            .as_ref()
            .unwrap()
            .attackers
            .iter()
            .filter(|a| match a.defender {
                Obj::Player { id } => id == p,
                Obj::Card { id } => self.defender_side(id) == p,
                _ => false,
            })
            .map(|a| a.card)
            .collect();
        let options: Vec<BlockerOpt> = self
            .battlefield
            .iter()
            .copied()
            .filter(|id| self.cards[id.0 as usize].cur.controller == p)
            .filter_map(|b| {
                let can: Vec<CardId> = attackers.iter().copied().filter(|a| self.can_block_attacker(b, *a)).collect();
                if can.is_empty() {
                    None
                } else {
                    Some(BlockerOpt { card: b, can_block: can })
                }
            })
            .collect();
        if options.is_empty() {
            self.run = Run::Blockers { idx: idx + 1 };
            return Flow::Continue;
        }
        // Camouflage: blockers are declared as piles assigned to the attackers at random
        if self.pile_blocks_turn == self.turn && self.turn != 0 {
            let n = attackers.len();
            let (_, mut piles) = self.pile_state.take().filter(|(pp, _)| *pp == p).unwrap_or((p, vec![]));
            let cands = |piles: &Vec<Vec<CardId>>| -> Vec<CardId> {
                options.iter().map(|o| o.card).filter(|b| piles.iter().filter(|pl| pl.contains(b)).count() < 1 + self.cards[b.0 as usize].cur.extra_blocks as usize).collect()
            };
            if let Some(Choice::Pick { indices }) = self.answer.take() {
                let c = cands(&piles);
                piles.push(indices.iter().filter_map(|i| c.get(*i).copied()).collect());
            }
            while piles.len() < n {
                let c = cands(&piles);
                if c.is_empty() {
                    piles.push(vec![]);
                    continue;
                }
                let decider = self.block_chooser.iter().find(|(d, _)| *d == p).map_or(p, |(_, c)| *c);
                self.pending = Some(Decision {
                    player: decider,
                    kind: DecisionKind::Pick {
                        prompt: format!("Camouflage: choose the creatures in pile {} of {}", piles.len() + 1, n),
                        source: None,
                        options: c.iter().map(|b| Opt::Obj { obj: Obj::card(*b) }).collect(),
                        min: 0,
                        max: c.len().min(255) as u8,
                        cancellable: false,
                        hidden: false,
                        purpose: PickPurpose::Generic,
                    },
                });
                self.pile_state = Some((p, piles));
                self.run = Run::Blockers { idx };
                return Flow::Wait;
            }
            // each pile goes to a different attacker, at random
            let mut order: Vec<usize> = (0..n).collect();
            rand::seq::SliceRandom::shuffle(&mut order[..], &mut self.rng);
            let mut blocks: Vec<(CardId, CardId)> = vec![];
            for (k, pile) in piles.iter().enumerate() {
                for b in pile {
                    blocks.push((*b, attackers[order[k]]));
                }
            }
            self.answer = Some(Choice::Blockers { blocks });
        }
        if let Some(Choice::Blockers { blocks }) = self.answer.take() {
            let requested: Vec<(CardId, CardId)> = blocks.iter().copied().filter(|(b, a)| (b.0 as usize) < self.cards.len() && (a.0 as usize) < self.cards.len()).collect();
            let mut used: Vec<CardId> = vec![];
            let mut pairs: Vec<(CardId, CardId)> = vec![];
            for (b, a) in blocks {
                // `validate` refuses ids that name nothing; skip them here too rather than index out of bounds
                if (b.0 as usize) >= self.cards.len() || (a.0 as usize) >= self.cards.len() {
                    continue;
                }
                let allowed = 1 + self.cards[b.0 as usize].cur.extra_blocks as usize;
                if used.iter().filter(|u| **u == b).count() >= allowed || pairs.contains(&(b, a)) {
                    continue;
                }
                if options.iter().any(|o| o.card == b && o.can_block.contains(&a)) {
                    used.push(b);
                    pairs.push((b, a));
                }
            }
            // creatures that must block (if able) and attackers that must be blocked (if able)
            for o in &options {
                let reqs = self.cards[o.card.0 as usize].cur.must_block.clone();
                for (af, src, ctrl) in reqs {
                    let targets: Vec<CardId> = o.can_block.iter().copied().filter(|a| af.as_ref().map_or(true, |f| self.matches_card(f, *a, src, ctrl))).collect();
                    // a creature that can block several attackers ("blocks each attacking creature if able") blocks as many matching ones as it may
                    let allowed = 1 + self.cards[o.card.0 as usize].cur.extra_blocks as usize;
                    for a in if af.is_some() { targets } else { targets.into_iter().take(1).collect() } {
                        // a plain requirement is met by any block
                        let met = pairs.iter().any(|(b, x)| *b == o.card && (af.is_none() || *x == a));
                        let used = pairs.iter().filter(|(b, _)| *b == o.card).count();
                        if !met && used < allowed.max(1) && (af.is_some() || !pairs.iter().any(|(b, _)| *b == o.card)) {
                            pairs.push((o.card, a));
                        } else if !met && af.is_some() && used >= allowed.max(1) {
                            // the creature chose to block something else: the requirement to block this attacker wins (Lure)
                            if let Some(pos) = pairs.iter().position(|(b, _)| *b == o.card) {
                                pairs[pos] = (o.card, a);
                            }
                        }
                    }
                }
            }
            for &a in &attackers {
                if self.cards[a.0 as usize].cur.must_be_blocked && !pairs.iter().any(|(_, x)| *x == a) {
                    if let Some(o) = options.iter().find(|o| o.can_block.contains(&a) && !pairs.iter().any(|(b, _)| *b == o.card)) {
                        pairs.push((o.card, a));
                    } else if let Some(o) = options.iter().find(|o| {
                        // a blocker already blocking an attacker that does not need it (not must-be-blocked, or blocked by another creature too) is redirected
                        o.can_block.contains(&a)
                            && pairs.iter().any(|(b, x)| *b == o.card && (!self.cards[x.0 as usize].cur.must_be_blocked || pairs.iter().filter(|(_, y)| y == x).count() > 1))
                    }) {
                        if let Some(pos) = pairs.iter().position(|(b, x)| *b == o.card && (!self.cards[x.0 as usize].cur.must_be_blocked || pairs.iter().filter(|(_, y)| y == x).count() > 1)) {
                            pairs[pos] = (o.card, a);
                        }
                    }
                }
            }
            // attackers that must be blocked by N creatures matching a filter if able
            for &a in &attackers {
                let reqs = self.cards[a.0 as usize].cur.must_be_blocked_by.clone();
                for (bf, n, src, ctrl) in reqs {
                    let have = |pairs: &Vec<(CardId, CardId)>, g: &Game| pairs.iter().filter(|(b, x)| *x == a && g.matches_card(&bf, *b, src, ctrl)).count();
                    while have(&pairs, self) < n as usize {
                        let cand = options.iter().find(|o| o.can_block.contains(&a) && !pairs.iter().any(|(b, _)| *b == o.card) && self.matches_card(&bf, o.card, src, ctrl));
                        match cand {
                            Some(o) => pairs.push((o.card, a)),
                            None => break,
                        }
                    }
                }
            }
            // "can't be blocked by more than N creatures": only the first N blockers count
            for &a in &attackers {
                let max = self.cards[a.0 as usize].cur.max_blockers as usize;
                if max > 0 {
                    let mut seen = 0usize;
                    pairs.retain(|(_, x)| {
                        if *x != a {
                            return true;
                        }
                        seen += 1;
                        seen <= max
                    });
                }
            }
            // "can't be blocked except by N or more creatures": a smaller block is dropped
            for &a in &attackers {
                let min = self.cards[a.0 as usize].cur.min_blockers as usize;
                if min > 1 {
                    let cnt = pairs.iter().filter(|(_, x)| *x == a).count();
                    if cnt > 0 && cnt < min {
                        pairs.retain(|(_, x)| *x != a);
                    }
                }
            }
            // "can't be blocked unless all creatures defending player controls block it"
            for &a in &attackers {
                if !self.cards[a.0 as usize].cur.block_all {
                    continue;
                }
                let Some(ctl) = pairs.iter().find(|(_, x)| *x == a).map(|(b, _)| self.cards[b.0 as usize].cur.controller) else { continue };
                let total = self.battlefield.iter().filter(|id| {
                    let c = &self.cards[id.0 as usize];
                    c.cur.controller == ctl && c.cur.types.has(CardType::Creature)
                }).count();
                let cnt = pairs.iter().filter(|(_, x)| *x == a).count();
                if cnt < total {
                    pairs.retain(|(_, x)| *x != a);
                }
            }
            // costs to block ("can't block unless its controller pays ...")
            {
                let mut paid: Vec<CardId> = vec![];
                let mut refused: Vec<CardId> = vec![];
                let pairs_now: Vec<(CardId, CardId)> = pairs.clone();
                for (b, att) in pairs_now {
                    if paid.contains(&b) || refused.contains(&b) {
                        continue;
                    }
                    let costs = self.combat_costs(b, false, None, Some(att));
                    let ctl = self.cards[b.0 as usize].cur.controller;
                    if costs.is_empty() || self.try_pay_costs(ctl, b, &costs) {
                        paid.push(b);
                    } else {
                        refused.push(b);
                    }
                }
                pairs.retain(|(b, _)| !refused.contains(b));
            }
            // menace: needs two or more blockers
            let mut counts: Vec<(CardId, usize)> = vec![];
            for (_, a) in &pairs {
                if let Some(e) = counts.iter_mut().find(|(c, _)| c == a) {
                    e.1 += 1;
                } else {
                    counts.push((*a, 1));
                }
            }
            pairs.retain(|(_, a)| {
                let n = counts.iter().find(|(c, _)| c == a).map_or(0, |e| e.1);
                !(self.cards[a.0 as usize].cur.kw.has(Keyword::Menace) && n < 2)
            });
            // "can't block unless a creature with greater power also blocks"
            loop {
                let before = pairs.len();
                let mut bl: Vec<CardId> = pairs.iter().map(|(b, _)| *b).collect();
                bl.sort_by_key(|c| c.0);
                bl.dedup();
                let snapshot = pairs.clone();
                pairs.retain(|(b, _)| {
                    let bc = &self.cards[b.0 as usize].cur;
                    !bc.cant_block_unless_greater || bl.iter().any(|o| *o != *b && self.cards[o.0 as usize].cur.power > bc.power && snapshot.iter().any(|(x, _)| x == o))
                });
                if pairs.len() == before {
                    break;
                }
            }
            // "can't block unless at least N other creatures block"
            loop {
                let before = pairs.len();
                let mut bl: Vec<CardId> = pairs.iter().map(|(b, _)| *b).collect();
                bl.sort_by_key(|c| c.0);
                bl.dedup();
                let n = bl.len();
                pairs.retain(|(b, _)| (self.cards[b.0 as usize].cur.block_needs_others as usize) < n);
                if pairs.len() == before {
                    break;
                }
            }
            // "can't block alone": a lone blocker with the restriction doesn't block
            {
                let mut distinct: Vec<CardId> = pairs.iter().map(|(b, _)| *b).collect();
                distinct.sort_by_key(|c| c.0);
                distinct.dedup();
                if distinct.len() == 1 && self.cards[distinct[0].0 as usize].cur.cant_block_alone {
                    // a blocker that must block (or blocks an attacker that must be blocked) blocks with a partner when another
                    // creature can join it for free (CR 509.1c: that obeys the requirement and breaks no restriction)
                    let b = distinct[0];
                    let required = !self.cards[b.0 as usize].cur.must_block.is_empty()
                        || pairs.iter().any(|(_, a)| self.cards[a.0 as usize].cur.must_be_blocked || !self.cards[a.0 as usize].cur.must_be_blocked_by.is_empty());
                    let fits = |a: CardId| {
                        let ac = &self.cards[a.0 as usize].cur;
                        // one more blocker on `a` keeps its block legal
                        let n = pairs.iter().filter(|(_, x)| *x == a).count() + 1;
                        !ac.block_all && (ac.max_blockers == 0 || n <= ac.max_blockers as usize) && (!ac.kw.has(Keyword::Menace) || n >= 2) && n >= (ac.min_blockers as usize).max(1)
                    };
                    let partner = if required && self.block_total_limit().is_none() {
                        let first = pairs[0].1;
                        options.iter().filter(|o| o.card != b && !self.cards[o.card.0 as usize].cur.cant_block_unless_greater && self.cards[o.card.0 as usize].cur.block_needs_others <= 1).find_map(|o| {
                            let a = if o.can_block.contains(&first) && fits(first) { first } else { *o.can_block.iter().find(|a| fits(**a))? };
                            self.combat_costs(o.card, false, None, Some(a)).is_empty().then_some((o.card, a))
                        })
                    } else {
                        None
                    };
                    match partner {
                        Some(x) => pairs.push(x),
                        None => pairs.clear(),
                    }
                }
            }
            // "No more than N creatures can block each combat": blockers beyond the cap (counting earlier declarations) are dropped
            if let Some(lim) = self.block_total_limit() {
                let mut have: Vec<CardId> = self.combat.as_ref().unwrap().attackers.iter().flat_map(|x| x.blockers.iter().copied()).collect();
                have.sort_by_key(|c| c.0);
                have.dedup();
                let mut kept: Vec<CardId> = vec![];
                pairs.retain(|(b, _)| {
                    if kept.contains(b) {
                        return true;
                    }
                    if have.len() + kept.len() < lim as usize {
                        kept.push(*b);
                        true
                    } else {
                        false
                    }
                });
            }
            // "Can't block with more than N creatures this combat" (Mirri, Weatherlight Duelist): blockers beyond the cap are dropped
            if let Some(lim) = self.block_limits.iter().filter(|(bp, _)| *bp == p).map(|(_, n)| *n).min() {
                let mut kept: Vec<CardId> = vec![];
                pairs.retain(|(b, _)| {
                    if kept.contains(b) {
                        return true;
                    }
                    if kept.len() < lim as usize {
                        kept.push(*b);
                        true
                    } else {
                        false
                    }
                });
            }
            self.note_block_changes(p, &requested, &options, &pairs);
            self.layers_dirty = true;
            // all of this player's blocks are recorded before any "blocks" trigger looks at the board
            for (b, a) in pairs.iter() {
                if let Some(info) = self.combat.as_mut().unwrap().attackers.iter_mut().find(|x| x.card == *a) {
                    info.blocked = true;
                    info.blockers.push(*b);
                }
            }
            for (b, a) in pairs {
                self.emit(Event::Blocked { blocker: b, attacker: a });
                // Flanking: a blocker without flanking gets -1/-1 until end of turn
                if self.cards[a.0 as usize].cur.kw.has(Keyword::Flanking) && !self.cards[b.0 as usize].cur.kw.has(Keyword::Flanking) {
                    let ac = self.cards[a.0 as usize].cur.controller;
                    // each instance of flanking triggers: the printed one plus one for each static that grants it (two Sidewinder Slivers)
                    let granted = self
                        .battlefield
                        .iter()
                        .filter(|&&sid| {
                            let sc = &self.cards[sid.0 as usize];
                            !sc.cur.no_abilities
                                && self.db.def(sc.def).statics.iter().any(|&i| {
                                    matches!(&self.db.def(sc.def).abilities[i as usize], Ability::Static { effect: StaticEffect::GrantKeyword { affects, keyword: Keyword::Flanking, .. }, .. }
                                        if self.matches_card(&affects.clone().into_filter(), a, sid, sc.cur.controller))
                                })
                        })
                        .count() as i32;
                    let n = (granted + self.db.def(self.cards[a.0 as usize].def).keywords.has(Keyword::Flanking) as i32).max(1);
                    self.add_effect(a, ac, &[b], Modif::Pump { power: -n, toughness: -n }, Duration::UntilEndOfTurn);
                }
            }
            self.run = Run::Blockers { idx: idx + 1 };
            return Flow::Continue;
        }
        let mut decider = self.block_chooser.iter().find(|(d, _)| *d == p).map_or(p, |(_, c)| *c);
        if self.attacker_chooses_blocks() {
            decider = self.active;
        }
        self.pending = Some(Decision { player: decider, kind: DecisionKind::Blockers { options } });
        self.run = Run::Blockers { idx };
        Flow::Wait
    }

    fn after_blocks(&mut self) {
        // banding: when a creature in a band is blocked, every creature in that band is blocked by the same creatures
        if let Some(cb) = self.combat.as_mut() {
            let bands: Vec<u8> = cb.attackers.iter().map(|a| a.band).filter(|b| *b != 0).collect();
            for b in bands {
                let mut union: Vec<CardId> = vec![];
                for a in cb.attackers.iter().filter(|a| a.band == b) {
                    for x in &a.blockers {
                        if !union.contains(x) {
                            union.push(*x);
                        }
                    }
                }
                if !union.is_empty() {
                    for a in cb.attackers.iter_mut().filter(|a| a.band == b) {
                        a.blocked = true;
                        a.blockers = union.iter().copied().collect();
                    }
                }
            }
        }
        let unblocked: Vec<CardId> = self.combat.as_ref().map(|c| c.attackers.iter().filter(|a| !a.blocked && self.cards[a.card.0 as usize].zone == Zone::Battlefield).map(|a| a.card).collect()).unwrap_or_default();
        for c in unblocked {
            self.emit(Event::Unblocked { card: c });
        }
        self.run = Run::Priority { player: self.active, passes: 0 };
    }

    // ------------------------------------------------------------------ damage

    fn deals_damage_in_step(&self, id: CardId, first_strike_step: bool) -> bool {
        let c = &self.cards[id.0 as usize];
        if c.zone != Zone::Battlefield {
            return false;
        }
        let kw = c.cur.kw;
        let fs = kw.has(Keyword::FirstStrike) || kw.has(Keyword::DoubleStrike);
        if first_strike_step {
            fs
        } else if self.fs_dealers.contains(&id) {
            // it had first or double strike in the first-strike step: only double strike deals damage again
            kw.has(Keyword::DoubleStrike)
        } else {
            // it did not deal first-strike damage (it lacked first strike then, or there was no first-strike step): it deals regular damage now, whatever it has since gained (CR 510.4)
            true
        }
    }

    /// Does any creature in combat have first or double strike (so the extra damage step happens)?
    pub fn first_strike_step_needed(&self) -> bool {
        let Some(cb) = &self.combat else { return false };
        cb.attackers.iter().any(|a| {
            let k = &self.cards[a.card.0 as usize].cur.kw;
            k.has(Keyword::FirstStrike) || k.has(Keyword::DoubleStrike) || a.blockers.iter().any(|b| {
                let k = &self.cards[b.0 as usize].cur.kw;
                k.has(Keyword::FirstStrike) || k.has(Keyword::DoubleStrike)
            })
        })
    }

    /// Does the card itself have "trample over planeswalkers"?
    fn trample_over_pw(&self, card: CardId) -> bool {
        let c = &self.cards[card.0 as usize];
        if c.cur.no_abilities {
            return false;
        }
        let d = self.db.def(c.def);
        d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::TrampleOverPlaneswalkers, .. }))
    }

    /// Does the card itself say it may divide its combat damage among the defending player and their creatures (Butcher Orgg)?
    fn divides_among_defenders(&self, card: CardId) -> bool {
        let c = &self.cards[card.0 as usize];
        if c.cur.no_abilities {
            return false;
        }
        let d = self.db.def(c.def);
        d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::DivideCombatDamageAmongDefenders, .. }))
    }

    /// The defending player (the attacked player) who makes this attacker's combat damage assignment because of a `DefenderAssignsCombatDamage` permanent they control (Defensive Formation).
    fn formation_assigner(&self, info: &AttackInfo) -> Option<PlayerId> {
        let Obj::Player { id } = info.defender else { return None };
        self.battlefield
            .iter()
            .any(|&b| {
                let c = &self.cards[b.0 as usize];
                if c.cur.controller != id || c.cur.no_abilities {
                    return false;
                }
                let d = self.db.def(c.def);
                d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::DefenderAssignsCombatDamage, .. }))
            })
            .then_some(id)
    }

    /// Where an attacker's combat damage goes with the default assignment, without changing anything: unblocked, to the
    /// defender; blocked, to its blockers in order until each has lethal damage, and with trample the rest to the defender.
    pub fn damage_targets(&self, attacker: CardId, defender: Obj, blocked: bool, blockers: &[CardId]) -> Vec<Obj> {
        let a = &self.cards[attacker.0 as usize];
        let mut left = a.cur.power;
        if left <= 0 || a.zone != Zone::Battlefield {
            return vec![];
        }
        if !blocked {
            return vec![defender];
        }
        let trample = a.cur.kw.has(Keyword::Trample);
        let mut out = vec![];
        for &b in blockers {
            if self.cards[b.0 as usize].zone != Zone::Battlefield || left <= 0 {
                continue;
            }
            out.push(Obj::card(b));
            left -= self.lethal_for(b, attacker);
        }
        if trample && left > 0 {
            out.push(defender);
        }
        out
    }

    /// What a proposed set of blocks (blocker, attacker) would do, without changing anything: the per-pair rules (evasion,
    /// "can't block") and the rules about the whole block (menace and "N or more", "no more than N", "all creatures",
    /// "unless a creature with greater power also blocks", "unless N others block"). Block costs are not paid here.
    pub fn preview_blocks(&self, blocks: &[(CardId, CardId)]) -> Vec<crate::view::BlockPreview> {
        let Some(cb) = &self.combat else { return vec![] };
        let name = |id: CardId| self.card_name(id).into_owned();
        let mut pairs: Vec<(CardId, CardId)> = vec![];
        let mut dropped: Vec<(CardId, CardId, String)> = vec![];
        for &(b, a) in blocks {
            if (b.0 as usize) >= self.cards.len() || !cb.attackers.iter().any(|x| x.card == a) {
                continue;
            }
            if pairs.iter().any(|(x, _)| *x == b) {
                dropped.push((b, a, format!("{} already blocks another attacker", name(b))));
            } else if self.can_block_attacker(b, a) {
                pairs.push((b, a));
            } else {
                dropped.push((b, a, self.explain_block(self.cards[b.0 as usize].cur.controller, Some(b), Some(a))));
            }
        }
        for x in &cb.attackers {
            let a = x.card;
            let c = &self.cards[a.0 as usize].cur;
            let max = c.max_blockers as usize;
            if max > 0 {
                let mut seen = 0usize;
                let mut keep = vec![];
                for p in pairs.drain(..) {
                    if p.1 == a {
                        seen += 1;
                        if seen > max {
                            dropped.push((p.0, a, format!("{} can't be blocked by more than {max} creature{}", name(a), if max == 1 { "" } else { "s" })));
                            continue;
                        }
                    }
                    keep.push(p);
                }
                pairs = keep;
            }
            let menace = c.kw.has(Keyword::Menace);
            let min = if menace { (c.min_blockers as usize).max(2) } else { c.min_blockers as usize };
            let cnt = pairs.iter().filter(|p| p.1 == a).count();
            if min > 1 && cnt > 0 && cnt < min {
                let why = if menace && c.min_blockers <= 2 { format!("{} has menace: it needs two or more blockers", name(a)) } else { format!("{} can't be blocked except by {min} or more creatures", name(a)) };
                for p in pairs.iter().filter(|p| p.1 == a) {
                    dropped.push((p.0, a, why.clone()));
                }
                pairs.retain(|p| p.1 != a);
            }
            if c.block_all {
                if let Some(ctl) = pairs.iter().find(|p| p.1 == a).map(|p| self.cards[p.0 .0 as usize].cur.controller) {
                    let total = self.battlefield.iter().filter(|id| self.cards[id.0 as usize].cur.controller == ctl && self.cards[id.0 as usize].cur.types.has(CardType::Creature)).count();
                    if pairs.iter().filter(|p| p.1 == a).count() < total {
                        for p in pairs.iter().filter(|p| p.1 == a) {
                            dropped.push((p.0, a, format!("{} can't be blocked unless all your creatures block it", name(a))));
                        }
                        pairs.retain(|p| p.1 != a);
                    }
                }
            }
        }
        loop {
            let snapshot = pairs.clone();
            let mut gone = vec![];
            pairs.retain(|(b, a)| {
                let bc = &self.cards[b.0 as usize].cur;
                let ok = !bc.cant_block_unless_greater || snapshot.iter().any(|(o, _)| *o != *b && self.cards[o.0 as usize].cur.power > bc.power);
                if !ok {
                    gone.push((*b, *a, format!("{} can't block unless a creature with greater power also blocks", name(*b))));
                }
                ok
            });
            let n = {
                let mut bl: Vec<CardId> = pairs.iter().map(|(b, _)| *b).collect();
                bl.sort_by_key(|c| c.0);
                bl.dedup();
                bl.len()
            };
            pairs.retain(|(b, a)| {
                let need = self.cards[b.0 as usize].cur.block_needs_others as usize;
                let ok = need < n;
                if !ok {
                    gone.push((*b, *a, format!("{} can't block unless at least {need} other creatures block", name(*b))));
                }
                ok
            });
            let changed = !gone.is_empty();
            dropped.extend(gone);
            if !changed {
                break;
            }
        }
        cb.attackers
            .iter()
            .map(|x| {
                let bl: Vec<CardId> = pairs.iter().filter(|p| p.1 == x.card).map(|p| p.0).collect();
                crate::view::BlockPreview {
                    attacker: x.card,
                    damage_to: self.damage_targets(x.card, x.defender, !bl.is_empty(), &bl),
                    blockers: bl,
                    dropped: dropped.iter().filter(|d| d.1 == x.card).map(|d| (d.0, d.2.clone())).collect(),
                }
            })
            .collect()
    }

    fn lethal_for(&self, blocker: CardId, source: CardId) -> i32 {
        let b = &self.cards[blocker.0 as usize];
        let dt = self.cards[source.0 as usize].cur.kw.has(Keyword::Deathtouch);
        if dt {
            1
        } else {
            (self.lethal_threshold(blocker) - b.damage).max(1)
        }
    }

    pub(crate) fn begin_damage(&mut self, first_strike: bool) {
        if first_strike {
            let all: Vec<CardId> = self.combat.as_ref().map_or(vec![], |cb| cb.attackers.iter().flat_map(|a| std::iter::once(a.card).chain(a.blockers.iter().copied())).collect());
            self.fs_dealers = all.into_iter().filter(|c| self.deals_damage_in_step(*c, true)).collect();
        }
        // Illusionary Mask: a marked face-down creature that deals damage in this step is turned face up first
        if let Some(cb) = &self.combat {
            let marked: Vec<CardId> = cb.attackers.iter().flat_map(|a| std::iter::once(a.card).chain(a.blockers.iter().copied())).filter(|c| self.cards[c.0 as usize].unmask).collect();
            for c in marked {
                if self.deals_damage_in_step(c, first_strike) {
                    self.unmask_creature(c);
                }
            }
        }
        let mut queue = vec![];
        if let Some(cb) = &self.combat {
            for a in &cb.attackers {
                if !self.deals_damage_in_step(a.card, first_strike) {
                    continue;
                }
                let n_blockers = a.blockers.iter().filter(|b| self.cards[b.0 as usize].zone == Zone::Battlefield).count();
                let trample = self.cards[a.card.0 as usize].cur.kw.has(Keyword::Trample);
                if !a.blocked && self.divides_among_defenders(a.card) {
                    queue.push(a.card);
                    continue;
                }
                if a.blocked && !self.cards[a.card.0 as usize].cur.assign_as_unblocked && self.formation_assigner(a).is_some() {
                    // the defending player divides the damage freely among the blockers: nothing to ask with a single blocker
                    if n_blockers > 1 {
                        queue.push(a.card);
                    }
                    continue;
                }
                if (a.blocked && n_blockers > 1) || (a.blocked && trample && n_blockers >= 1) || (a.blocked && self.cards[a.card.0 as usize].cur.assign_as_unblocked) {
                    queue.push(a.card);
                } else if a.blocked && n_blockers == 1 && a.blockers.iter().any(|b| self.cards[b.0 as usize].zone == Zone::Battlefield && self.cards[b.0 as usize].cur.kw.has(Keyword::Banding)) {
                    // a blocker with banding: the defending player assigns this attacker's damage
                    queue.push(a.card);
                }
                // blockers of a band that has a banding creature: the attacking player divides each blocker's damage among the band
                if a.band != 0 && a.blocked && cb.attackers.iter().filter(|x| x.band == a.band).any(|x| self.cards[x.card.0 as usize].cur.kw.has(Keyword::Banding) || self.cards[x.card.0 as usize].cur.kw.has(Keyword::BandsWithLegendary) || self.cards[x.card.0 as usize].cur.kw.has(Keyword::BandsWithSameName)) {
                    for b in &a.blockers {
                        if self.cards[b.0 as usize].zone == Zone::Battlefield && self.deals_damage_in_step(*b, first_strike) && !queue.contains(b) {
                            queue.push(*b);
                        }
                    }
                }
            }
        }
        self.run = Run::DamageAssign(Box::new(DamageCtx { first_strike, queue, done: vec![], current: None }));
    }

    pub(crate) fn step_damage(&mut self, mut ctx: Box<DamageCtx>) -> Flow {
        let p = self.active;
        // answer for the current attacker
        if let (Some(cur), Some(Choice::Damage { assign })) = (ctx.current, self.answer.take()) {
            for (o, n) in assign {
                if n > 0 {
                    ctx.done.push((cur, o, n));
                }
            }
            ctx.current = None;
        }
        while let Some(&att) = ctx.queue.first() {
            ctx.queue.remove(0);
            let total = self.combat_power(att);
            if total == 0 {
                continue;
            }
            let cb = self.combat.as_ref().unwrap();
            let Some(info) = cb.attackers.iter().find(|a| a.card == att).cloned() else {
                // a blocker whose attacker is in a band with a banding creature: the attacking player divides its damage among the band
                let Some(target) = cb.attackers.iter().find(|a| a.blockers.contains(&att)).cloned() else { continue };
                let members: Vec<(CardId, i32)> = cb
                    .attackers
                    .iter()
                    .filter(|a| a.band == target.band && self.cards[a.card.0 as usize].zone == Zone::Battlefield)
                    .map(|a| (a.card, 0))
                    .collect();
                ctx.current = Some(att);
                self.pending = Some(Decision { player: p, kind: DecisionKind::AssignDamage { attacker: att, total, blockers: members, defender: None, trample: false } });
                self.run = Run::DamageAssign(ctx);
                return Flow::Wait;
            };
            if !info.blocked && self.divides_among_defenders(att) {
                // the defending player's creatures may take any share; by default everything goes to the defender
                let dp = match info.defender {
                    Obj::Player { id } => Some(id),
                    Obj::Card { id } => Some(self.cards[id.0 as usize].cur.controller),
                    _ => None,
                };
                let mut blockers: Vec<(CardId, i32)> = vec![];
                if let Some(dp) = dp {
                    for &id in &self.battlefield {
                        let k = &self.cards[id.0 as usize];
                        if k.cur.controller == dp && k.cur.types.has(CardType::Creature) {
                            blockers.push((id, 0));
                        }
                    }
                }
                ctx.current = Some(att);
                self.pending = Some(Decision { player: p, kind: DecisionKind::AssignDamage { attacker: att, total, blockers, defender: Some(info.defender), trample: true } });
                self.run = Run::DamageAssign(ctx);
                return Flow::Wait;
            }
            // "assign combat damage as though it weren't blocked": no blocker needs lethal damage first, and the defender can take it all
            let unblocked_opt = self.cards[att.0 as usize].cur.assign_as_unblocked;
            // banding: a blocker with banding lets its controller (the defending player) divide the attacker's damage freely
            let banding_blocker = info.blockers.iter().copied().find(|b| {
                let c = &self.cards[b.0 as usize];
                c.zone == Zone::Battlefield
                    && (c.cur.kw.has(Keyword::Banding)
                        // "bands with other legendary creatures": two legendary creatures of the same controller block this attacker, one of them with the keyword
                        || (c.cur.kw.has(Keyword::BandsWithLegendary)
                            && c.cur.supertypes.contains(&Supertype::Legendary)
                            && info.blockers.iter().any(|o| o != b && self.cards[o.0 as usize].zone == Zone::Battlefield && self.cards[o.0 as usize].cur.controller == c.cur.controller && self.cards[o.0 as usize].cur.supertypes.contains(&Supertype::Legendary)))
                        // "bands with other creatures named ...": another creature of the same controller with the same name blocks it too
                        || (c.cur.kw.has(Keyword::BandsWithSameName)
                            && info.blockers.iter().any(|o| o != b && self.cards[o.0 as usize].zone == Zone::Battlefield && self.cards[o.0 as usize].cur.controller == c.cur.controller && self.view_name(&self.view(*o)) == self.view_name(&self.view(*b)))))
            });
            let formation = if unblocked_opt || banding_blocker.is_some() { None } else { self.formation_assigner(&info) };
            let chooser = banding_blocker.map_or(formation.unwrap_or(p), |b| self.cards[b.0 as usize].cur.controller);
            let blockers: Vec<(CardId, i32)> = info
                .blockers
                .iter()
                .filter(|b| self.cards[b.0 as usize].zone == Zone::Battlefield)
                .map(|b| (*b, if unblocked_opt || banding_blocker.is_some() || formation.is_some() { 0 } else { self.lethal_for(*b, att) }))
                .collect();
            let trample = unblocked_opt || (self.cards[att.0 as usize].cur.kw.has(Keyword::Trample) && formation.is_none());
            ctx.current = Some(att);
            self.pending = Some(Decision {
                player: chooser,
                kind: DecisionKind::AssignDamage { attacker: att, total, blockers, defender: if trample && banding_blocker.is_none() { Some(info.defender) } else { None }, trample: trample && banding_blocker.is_none() },
            });
            self.run = Run::DamageAssign(ctx);
            return Flow::Wait;
        }
        self.deal_combat_damage(*ctx);
        Flow::Continue
    }

    fn deal_combat_damage(&mut self, ctx: DamageCtx) {
        self.combat_batch = true;
        self.batch_once_used.clear();
        let first = ctx.first_strike;
        let mut dmg: Vec<(CardId, Obj, i32)> = ctx.done.clone();
        let decided: Vec<CardId> = ctx.done.iter().map(|(c, _, _)| *c).collect();
        let Some(cb) = self.combat.clone() else {
            self.after_damage();
            return;
        };
        for a in &cb.attackers {
            let power = self.combat_power(a.card);
            if self.deals_damage_in_step(a.card, first) && !decided.contains(&a.card) && power > 0 {
                let alive_blockers: Vec<CardId> =
                    a.blockers.iter().copied().filter(|b| self.cards[b.0 as usize].zone == Zone::Battlefield).collect();
                if !a.blocked {
                    dmg.push((a.card, a.defender, power));
                } else if let Some(&b) = alive_blockers.first() {
                    dmg.push((a.card, Obj::card(b), power));
                } else if self.cards[a.card.0 as usize].cur.kw.has(Keyword::Trample) {
                    dmg.push((a.card, a.defender, power));
                }
            }
            // blockers hit the attacker they block (a blocker of several attackers deals lethal damage to each in declaration order, the rest to the last)
            for b in &a.blockers {
                if self.deals_damage_in_step(*b, first) && !decided.contains(b) && self.cards[a.card.0 as usize].zone == Zone::Battlefield {
                    let bp = self.combat_power(*b);
                    if bp <= 0 {
                        continue;
                    }
                    let mine: Vec<CardId> = cb.attackers.iter().filter(|x| x.blockers.contains(b) && self.cards[x.card.0 as usize].zone == Zone::Battlefield).map(|x| x.card).collect();
                    if mine.len() <= 1 {
                        dmg.push((*b, Obj::card(a.card), bp));
                        continue;
                    }
                    let mut left = bp;
                    for (i, t) in mine.iter().enumerate() {
                        let cardt = &self.cards[t.0 as usize];
                        let lethal = (self.lethal_threshold(*t) - cardt.damage as i32).max(1);
                        let give = if i + 1 == mine.len() { left } else { lethal.min(left) };
                        if *t == a.card && give > 0 {
                            dmg.push((*b, Obj::card(a.card), give));
                        }
                        left -= give;
                        if left <= 0 && i + 1 < mine.len() {
                            left = 0;
                        }
                    }
                }
            }
        }
        self.batch_seen.clear();
        self.batch_seen_pw.clear();
        let hit_players: Vec<CardId> = dmg.iter().filter(|(_, t, n)| *n > 0 && matches!(t, Obj::Player { .. })).map(|(s, _, _)| *s).collect();
        // trample over planeswalkers: the excess over the planeswalker's loyalty is dealt to its controller
        let mut split: Vec<(CardId, Obj, i32)> = vec![];
        for (src, target, n) in dmg {
            if let Obj::Card { id: pw } = target {
                if n > 0 && self.cards[pw.0 as usize].cur.types.has(CardType::Planeswalker) && self.trample_over_pw(src) {
                    let loyalty = self.cards[pw.0 as usize].counter(CounterKind::Loyalty).max(0);
                    let to_pw = n.min(loyalty);
                    if n > to_pw {
                        split.push((src, Obj::player(self.cards[pw.0 as usize].cur.controller), n - to_pw));
                    }
                    if to_pw > 0 {
                        split.push((src, target, to_pw));
                    }
                    continue;
                }
            }
            split.push((src, target, n));
        }
        *self.prevent_batch.borrow_mut() = Some(vec![]);
        for (src, target, n) in split {
            self.deal_damage(Some(src), target, n, true);
        }
        *self.prevent_batch.borrow_mut() = None;
        // cipher: the creature an encoded card is encoded on dealt combat damage to a player: its controller casts a copy of the card
        if !self.encoded.is_empty() {
            let enc = self.encoded.clone();
            for (card, cr, zcc) in enc {
                if hit_players.contains(&cr) && self.cards[cr.0 as usize].zone == Zone::Battlefield && self.cards[cr.0 as usize].zcc == zcc && self.cards[card.0 as usize].zone == Zone::Exile {
                    // a trigger: its controller may cast a copy of the encoded card without paying its mana cost
                    let ctl = self.cards[cr.0 as usize].cur.controller;
                    let effect: Effect = serde_json::from_value(serde_json::json!({
                        "type": "CastFree", "what": {"type": "Remembered"}, "may": true, "copy": true, "pay": false
                    })).expect("cipher cast effect");
                    let d = DelayedTrigger {
                        id: 0,
                        controller: ctl,
                        source: card,
                        when: DelayedWhen::NextEndStep,
                        effects: vec![effect],
                        created_turn: self.turn,
                        targets: vec![],
                        ctx: EventCtx::default(),
                        remembered: vec![Obj::Card { id: card }],
                    };
                    self.push_delayed_item(d);
                }
            }
            // forget encodings whose creature is gone
            let bf: Vec<(CardId, u32)> = self.encoded.iter().map(|(_, cr, z)| (*cr, *z)).collect();
            let keep: Vec<bool> = bf.iter().map(|(cr, z)| self.cards[cr.0 as usize].zone == Zone::Battlefield && self.cards[cr.0 as usize].zcc == *z).collect();
            let mut i = 0;
            self.encoded.retain(|_| {
                let k = keep[i];
                i += 1;
                k
            });
        }
        self.after_damage();
    }

    fn after_damage(&mut self) {
        // the once-only redirects that applied during this damage step are used up now
        self.combat_batch = false;
        for (p, n) in std::mem::take(&mut self.life_batch) {
            self.emit(Event::Life { player: p, delta: -n });
        }
        let used = std::mem::take(&mut self.batch_once_used);
        if !used.is_empty() {
            self.shields.retain(|s| !(s.once && s.remaining.is_none() && s.redirect.is_some() && used.contains(&(s.to.clone(), s.redirect))));
        }
        self.run = Run::Priority { player: self.active, passes: 0 };
        // encoded cards (cipher) triggered by this combat damage are cast now
        self.start_next_free_cast();
    }
}
