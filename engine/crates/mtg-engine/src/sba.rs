//! State-based actions (CR 704) and the end-of-game check.

use crate::dsl::Ability;
use crate::decision::*;
use crate::ids::*;
use crate::state::*;
use crate::types::*;

pub enum SbaResult {
    /// Nothing more to do.
    Settled,
    /// A decision is needed (legend rule); `pending` is set.
    Ask,
}

impl Game {
    /// Perform state-based actions repeatedly until none apply. Returns `Ask` if a choice is required.
    /// An "as this enters" trigger of this permanent is waiting or on the stack.
    /// `LethalNeedsSingleSource` holds for this creature and no single source has marked lethal damage on it.
    fn lethal_needs_single_unmet(&self, id: CardId) -> bool {
        let c = &self.cards[id.0 as usize];
        if c.cur.no_abilities {
            return false;
        }
        let sd = self.db.def(c.def);
        let has = sd.statics.iter().any(|&i| matches!(&sd.abilities[i as usize], Ability::Static { effect: crate::dsl::StaticEffect::LethalNeedsSingleSource, .. }));
        has && !c.dmg_parts.iter().any(|(_, n)| *n >= c.cur.toughness)
    }

    /// Damage that is lethal to a creature: its toughness, or its power (at least 1) under `LethalByPower`.
    pub(crate) fn lethal_threshold(&self, id: CardId) -> i32 {
        let c = &self.cards[id.0 as usize];
        if c.cur.lethal_by_power {
            c.cur.power.max(1)
        } else {
            c.cur.toughness
        }
    }

    pub(crate) fn as_enters_waiting(&self, id: CardId) -> bool {
        let c = &self.cards[id.0 as usize];
        let d = self.db.def(c.def);
        let is_ae = |ai: u16| matches!(d.abilities.get(ai as usize), Some(Ability::Triggered { as_enters: true, .. }));
        self.pending_triggers.iter().any(|p| p.source == id && p.src_zcc == c.zcc && is_ae(p.ability))
            || self.stack.iter().any(|s| s.source == id && s.src_zcc == c.zcc && matches!(&s.kind, ItemKind::Triggered { ability, .. } if is_ae(*ability)))
    }

    pub fn run_sba(&mut self) -> SbaResult {
        let _g = crate::prof::enter(3);
        let mut rounds = 0u32;
        loop {
            rounds += 1;
            if rounds > 5_000 {
                // an effect keeps undoing itself (e.g. two permanents that keep killing each other): end the game as a draw
                self.log("state-based actions never settle; the game is a draw");
                self.over = Some(GameResult::Draw);
                self.run = Run::Over;
                self.emit(Event::Over { result: GameResult::Draw });
                return SbaResult::Settled;
            }
            if !self.copy_while_source.is_empty() {
                let mut revert = false;
                for (id, zcc, old, src, src_zcc, attached) in std::mem::take(&mut self.copy_while_source) {
                    let s = &self.cards[src.0 as usize];
                    if s.zone == Zone::Battlefield && s.zcc == src_zcc && match attached { 1 => s.attached_to == Some(id), 2 => s.tapped, _ => true } {
                        self.copy_while_source.push((id, zcc, old, src, src_zcc, attached));
                    } else if self.cards[id.0 as usize].zcc == zcc && self.cards[id.0 as usize].zone == Zone::Battlefield {
                        self.cards[id.0 as usize].def = old;
                        revert = true;
                    }
                }
                if revert {
                    self.layers_dirty = true;
                }
            }
            // Planechase / Archenemy: phenomena and one-shot schemes move on once their abilities have left the stack
            if (!self.planes.is_empty() || self.archenemy) && self.variant_sbas() {
                continue;
            }
            self.recompute_layers();
            if self.is_over() {
                return SbaResult::Settled;
            }
            let mut acted = false;
            // an attacking or blocking permanent that is no longer a creature is removed from combat (Xenagos when devotion drops)
            if let Some(cb) = self.combat.as_mut() {
                let gone: Vec<CardId> = cb
                    .attackers
                    .iter()
                    .map(|a| a.card)
                    .chain(cb.attackers.iter().flat_map(|a| a.blockers.iter().copied()))
                    .filter(|id| {
                        let c = &self.cards[id.0 as usize];
                        c.zone == Zone::Battlefield && !c.cur.types.has(CardType::Creature)
                    })
                    .collect();
                if !gone.is_empty() {
                    for id in gone {
                        cb.attackers.retain(|a| a.card != id);
                        for a in cb.attackers.iter_mut() {
                            a.blockers.retain(|b| *b != id);
                        }
                    }
                    acted = true;
                }
            }
            // Start your engines!: a player with no speed who controls a permanent with it gets speed 1 (also after a control change)
            if self.players.iter().any(|q| q.speed == 0) {
                for id in self.battlefield.clone() {
                    let c = &self.cards[id.0 as usize];
                    if c.zone == Zone::Battlefield && self.db.def(c.def).engines {
                        let ctl = c.cur.controller as usize;
                        if self.players[ctl].speed == 0 {
                            self.players[ctl].speed = 1;
                        }
                    }
                }
            }
            // Ascend: ten or more permanents give the city's blessing for the rest of the game
            for p in self.alive_players() {
                // only a permanent with ascend latches it (CR 702.131b); an ascend instant or sorcery checks when it resolves
                if !self.players[p as usize].city_blessing
                    && self.battlefield.iter().filter(|id| self.cards[id.0 as usize].cur.controller == p).count() >= 10
                    && self.battlefield.iter().any(|id| self.cards[id.0 as usize].cur.controller == p && self.def_of(*id).oracle_text.lines().any(|l| l.starts_with("Ascend")))
                {
                    self.players[p as usize].city_blessing = true;
                    self.layers_dirty = true;
                    acted = true;
                }
            }
            // Storied: three or more artifacts, legendaries and/or Sagas latch an enduring story
            for p in self.alive_players() {
                if !self.players[p as usize].enduring_story {
                    let n = self
                        .battlefield
                        .iter()
                        .filter(|id| {
                            let c = &self.cards[id.0 as usize];
                            c.cur.controller == p
                                && (c.cur.types.has(CardType::Artifact) || c.cur.supertypes.contains(&Supertype::Legendary) || c.cur.subtypes.iter().any(|s| Sym::name(*s) == "Saga"))
                        })
                        .count();
                    if n >= 3 {
                        self.players[p as usize].enduring_story = true;
                        self.layers_dirty = true;
                        acted = true;
                    }
                }
            }
            // Enduring Angel: a life total of 0 or less becomes 3 as the creature transforms
            for p in self.alive_players() {
                if self.players[p as usize].life > 0 {
                    continue;
                }
                let angel = self.battlefield.iter().copied().find(|&id| {
                    let c = &self.cards[id.0 as usize];
                    let d = self.db.def(c.def);
                    c.cur.controller == p && !c.cur.no_abilities && d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: crate::dsl::StaticEffect::LifeZeroTransform, .. }))
                });
                if let Some(id) = angel {
                    let before = self.cards[id.0 as usize].def;
                    self.transform(id);
                    if self.cards[id.0 as usize].def != before {
                        self.players[p as usize].life = 3;
                        self.layers_dirty = true;
                        acted = true;
                    }
                }
            }
            // 704.5a/b/c: players
            for p in self.alive_players() {
                let pl = &self.players[p as usize];
                let why = if pl.life <= 0 && !self.dont_lose_at_zero(p) {
                    Some("life total 0 or less")
                } else if pl.drew_from_empty {
                    Some("drew from an empty library")
                } else if pl.poison >= 10 {
                    Some("ten poison counters")
                } else if pl.commander_damage.iter().any(|(_, d)| *d >= self.cfg.commander_damage) && self.cfg.commander {
                    Some("21 commander damage")
                } else {
                    None
                };
                if let Some(w) = why {
                    self.player_loses(p, w);
                    // "can't lose" (Platinum Angel) leaves the state unchanged: not progress
                    acted |= self.players[p as usize].lost;
                }
            }
            if self.check_game_over() {
                return SbaResult::Settled;
            }
            // creatures: 0 toughness, then lethal damage / deathtouch
            let bf = self.battlefield.clone();
            let mut to_graveyard: Vec<CardId> = vec![];
            let mut to_destroy: Vec<CardId> = vec![];
            let mut defeated: Vec<CardId> = vec![];
            for &id in &bf {
                let c = &self.cards[id.0 as usize];
                let t = c.cur.types;
                if t.has(CardType::Creature) {
                    if c.cur.toughness <= 0 {
                        if !self.as_enters_waiting(id) {
                            to_graveyard.push(id);
                        }
                    } else if (c.damage >= self.lethal_threshold(id) && !self.lethal_needs_single_unmet(id)) || (c.deathtouch_hit && c.damage > 0) {
                        to_destroy.push(id);
                    }
                }
                if t.has(CardType::Planeswalker) && c.counter(CounterKind::Loyalty) <= 0 && !c.cur.zero_loyalty_stays {
                    to_graveyard.push(id);
                }
                if t.has(CardType::Battle) && c.counter(CounterKind::Defense) <= 0 {
                    if self.db.def(c.def).other_face.is_some() {
                        defeated.push(id);
                    } else {
                        to_graveyard.push(id);
                    }
                }
                // a saga whose final chapter has finished (nothing of it waiting or on the stack) is sacrificed (714.4)
                let smax = self.db.def(c.def).saga_max;
                if smax > 0 && t.has(CardType::Enchantment) && c.counter(CounterKind::Lore) >= smax as i32 {
                    let busy = self.pending_triggers.iter().any(|p| p.source == id && p.src_zcc == c.zcc)
                        || self.stack.iter().any(|s| s.source == id && s.src_zcc == c.zcc);
                    if !busy {
                        to_graveyard.push(id);
                    }
                }
            }
            self.begin_batch();
            for id in to_graveyard {
                if self.move_card(id, Zone::Graveyard, Default::default()) {
                    acted = true;
                }
            }
            // a defeated battle is exiled, then its controller casts it transformed (simplified: it simply enters transformed)
            for id in defeated {
                let ctl = self.cards[id.0 as usize].cur.controller;
                if self.move_card(id, Zone::Exile, Default::default()) {
                    // an instant or sorcery back face can't enter the battlefield: its controller may cast it (transformed) without paying its mana cost
                    let nonperm = self.db.def(self.cards[id.0 as usize].def).other_face.map_or(false, |b| {
                        let t = &self.db.def(b).types;
                        t.has(CardType::Instant) || t.has(CardType::Sorcery)
                    });
                    if nonperm {
                        let cast: crate::dsl::Effect = serde_json::from_str(&format!(r#"{{"type":"CastFree","what":{{"type":"This"}},"may":true,"alt":{}}}"#, crate::decision::FACE_ALT)).expect("defeated siege cast");
                        self.dmg_followups.borrow_mut().push((id, ctl, vec![cast], 0, Some(id), None, Some(ctl), vec![]));
                        acted = true;
                        continue;
                    }
                    // it enters as its back face (so the front face's "when this enters" does not fire again)
                    if let Some(back) = self.db.def(self.cards[id.0 as usize].def).other_face {
                        self.cards[id.0 as usize].def = back;
                        self.reset_derived(id);
                    }
                    self.move_card(id, Zone::Battlefield, crate::zones::MoveOpts { controller: Some(ctl), ..Default::default() });
                    acted = true;
                }
            }
            self.flush_followups();
            // simultaneous deaths: creatures carrying replacement effects (Incendiary Oracle) leave last so the
            // effect still applies to the others dying at the same time
            to_destroy.sort_by_key(|id| { let d = self.db.def(self.cards[id.0 as usize].def); (d.has_replace, !d.statics.is_empty()) });
            for id in to_destroy {
                if self.cards[id.0 as usize].zone == Zone::Battlefield && self.destroy(id) {
                    acted = true;
                } else if self.layers_dirty {
                    // umbra armor / regeneration changed the board without the creature dying: look again
                    acted = true;
                }
            }
            self.end_batch();
            // auras and equipment
            let bf = self.battlefield.clone();
            for &id in &bf {
                let c = &self.cards[id.0 as usize];
                if let Some(host) = c.attached_to {
                    let h = &self.cards[host.0 as usize];
                    let attachment_ok = (h.zone == Zone::Battlefield && !h.phased_out) || self.gy_aura_host_ok(id, host);
                    let is_aura = c.cur.subtypes.contains(&Sym::new("Aura"));
                    let (is_ench, cdef) = (c.cur.types.has(CardType::Enchantment), c.def);
                    let is_equip = c.cur.subtypes.contains(&Sym::new("Equipment")) || c.cur.subtypes.contains(&Sym::new("Fortification"));
                    if is_aura && c.cur.types.has(CardType::Creature) && !self.db.def(cdef).types.has(CardType::Creature) {
                        // an Aura that is also a creature (March of the Machines) can't enchant anything: it is put into the graveyard (rule 704.5m)
                        self.cards[id.0 as usize].attached_to = None;
                        if self.move_card(id, Zone::Graveyard, Default::default()) {
                            acted = true;
                        }
                    } else if attachment_ok && is_equip && (self.cant_be_equipped(host) || self.protected_from(host, id)) {
                        self.cards[id.0 as usize].attached_to = None;
                        self.layers_dirty = true;
                        acted = true;
                    } else if is_equip && c.cur.types.has(CardType::Creature) {
                        // an Equipment that is also a creature isn't attached to anything (rule 704.5n)
                        self.cards[id.0 as usize].attached_to = None;
                        self.layers_dirty = true;
                        acted = true;
                    } else if !attachment_ok || (is_equip && !(h.cur.types.has(CardType::Creature) || c.cur.subtypes.contains(&Sym::new("Fortification")))) {
                        self.cards[id.0 as usize].attached_to = None;
                        self.layers_dirty = true;
                        acted = true;
                    } else if !is_aura && !is_equip {
                        self.cards[id.0 as usize].attached_to = None;
                        self.layers_dirty = true;
                        acted = true;
                    } else if is_equip {
                        // "can be attached only to ..." restriction on the Equipment itself
                        let db = self.db.clone();
                        let filter = db.def(cdef).abilities.iter().find_map(|a| if let crate::dsl::Ability::AttachOnlyTo { to } = a { Some(to.clone()) } else { None });
                        if let Some(mut f) = filter {
                            f.controller = crate::dsl::PlayerFilter::Any;
                            let ctrl = self.cards[id.0 as usize].cur.controller;
                            self.cards[id.0 as usize].attached_to = None;
                            // "can't choose an untapped permanent" (Dream Leash) restricts the choice when the Aura is cast, not afterwards
                            let mut f = (*f).clone();
                            f.tapped = None;
                            let ok = self.matches_card(&f, host, id, ctrl);
                            if ok {
                                self.cards[id.0 as usize].attached_to = Some(host);
                            } else {
                                self.layers_dirty = true;
                                acted = true;
                            }
                        }
                    }
                    if is_aura && attachment_ok && !self.cards[id.0 as usize].bestowed && self.protected_from(host, id) && (!self.def_of(id).oracle_text.contains("doesn't remove") || self.protection_beyond_own(host, id)) {
                        // an Aura attached to a permanent with protection from it (Necromancy on a pro-black creature) falls off
                        if self.move_card(id, Zone::Graveyard, Default::default()) {
                            acted = true;
                        }
                    } else if is_aura && attachment_ok && self.host_bars_other_auras(host, id) {
                        if self.move_card(id, Zone::Graveyard, Default::default()) {
                            acted = true;
                        }
                    } else if is_aura && attachment_ok && is_ench && self.cards[id.0 as usize].attached_to == Some(host) {
                        // the host must still satisfy "enchant <filter>" (the Aura itself doesn't count as an attachment)
                        let db = self.db.clone();
                        let filter = db.def(cdef).abilities.iter().find_map(|a| if let crate::dsl::Ability::Attach { to } = a { Some(to.clone()) } else { None });
                        if let Some(f) = filter {
                            let ctrl = self.cards[id.0 as usize].cur.controller;
                            self.cards[id.0 as usize].attached_to = None;
                            // "You can't choose an untapped permanent as this spell's target" is a casting restriction only (Dream Leash, Enthralling Hold)
                            let f = if db.def(cdef).oracle_text.contains("can't choose an untapped") { let mut g = f.into_filter(); g.tapped = None; crate::dsl::FilterBox::from(g) } else { f };
                            let ok = self.matches_card(&f, host, id, ctrl);
                            self.cards[id.0 as usize].attached_to = Some(host);
                            if !ok && self.move_card(id, Zone::Graveyard, Default::default()) {
                                acted = true;
                            }
                        } else if self.cards[id.0 as usize].as_aura && !(self.cards[host.0 as usize].cur.types.has(CardType::Creature) && self.cards[host.0 as usize].cur.controller == self.cards[id.0 as usize].cur.controller) {
                            // "enchant creature you control" (Bronzehide Lion)
                            if self.move_card(id, Zone::Graveyard, Default::default()) {
                                acted = true;
                            }
                        }
                    }
                } else if !c.bestowed && !(c.turnup_choice && self.enter_choices.contains(&id)) && c.cur.subtypes.contains(&Sym::new("Aura")) && c.cur.types.has(CardType::Enchantment) && c.enchanted_player.map_or(!self.attach_pending.contains(&id), |p| !self.alive(p)) {
                    // an Aura attached to nothing (or to a player who left the game) goes to the graveyard
                    if self.move_card(id, Zone::Graveyard, Default::default()) {
                        acted = true;
                    }
                }
            }
            // soulbond: a pair ends when either creature leaves, stops being a creature or the two have different controllers
            {
                let pairs: Vec<(CardId, CardId)> = self.battlefield.iter().copied().filter_map(|id| self.cards[id.0 as usize].paired.map(|p| (id, p))).collect();
                for (a, b) in pairs {
                    let ca = &self.cards[a.0 as usize];
                    let cb = &self.cards[b.0 as usize];
                    let ok = cb.zone == Zone::Battlefield && ca.cur.types.has(CardType::Creature) && cb.cur.types.has(CardType::Creature) && ca.cur.controller == cb.cur.controller && cb.paired == Some(a);
                    if !ok {
                        self.cards[a.0 as usize].paired = None;
                        if self.cards[b.0 as usize].paired == Some(a) {
                            self.cards[b.0 as usize].paired = None;
                        }
                        self.layers_dirty = true;
                        acted = true;
                    }
                }
            }
            // Role rule: a permanent can't have two Roles controlled by the same player; the older ones are put into the graveyard
            {
                let role = Sym::new("Role");
                let mut roles: Vec<(CardId, PlayerId, CardId, u64)> = vec![];
                for &id in &self.battlefield {
                    let c = &self.cards[id.0 as usize];
                    if let Some(h) = c.attached_to {
                        if c.cur.subtypes.contains(&role) {
                            roles.push((id, c.cur.controller, h, c.ts as u64));
                        }
                    }
                }
                for &(id, ctl, host, ts) in &roles {
                    if roles.iter().any(|&(o, octl, ohost, ots)| o != id && octl == ctl && ohost == host && ots > ts) {
                        if self.move_card(id, Zone::Graveyard, Default::default()) {
                            acted = true;
                        }
                    }
                }
            }
            // +1/+1 and -1/-1 counters annihilate
            let bf = self.battlefield.clone();
            for &id in &bf {
                let c = &mut self.cards[id.0 as usize];
                let (a, b) = (c.counter(CounterKind::PlusOnePlusOne), c.counter(CounterKind::MinusOneMinusOne));
                if a > 0 && b > 0 {
                    let n = a.min(b);
                    c.remove_counters(CounterKind::PlusOnePlusOne, n);
                    c.remove_counters(CounterKind::MinusOneMinusOne, n);
                    self.layers_dirty = true;
                    acted = true;
                }
            }
            // world rule (704.5k): only the newest world permanent stays (a tie puts them all into the graveyard)
            {
                let worlds: Vec<CardId> = self.battlefield.iter().copied().filter(|id| self.cards[id.0 as usize].cur.supertypes.contains(&Supertype::World)).collect();
                if worlds.len() > 1 {
                    let newest = worlds.iter().map(|id| self.cards[id.0 as usize].ts).max().unwrap_or(0);
                    let tied = worlds.iter().filter(|id| self.cards[id.0 as usize].ts == newest).count();
                    for id in worlds {
                        if self.cards[id.0 as usize].ts != newest || tied > 1 {
                            if self.move_card(id, Zone::Graveyard, Default::default()) {
                                acted = true;
                            }
                        }
                    }
                }
            }
            // a commander put into a graveyard or exile since the last check: its owner may put it into the command zone (903.9a)
            if self.cfg.commander {
                let offer = self.cards.iter().find(|c| c.is_commander && c.cmd_offer && matches!(c.zone, Zone::Graveyard | Zone::Exile)).map(|c| c.id);
                if let Some(id) = offer {
                    if self.sba_waiting && matches!(self.answer, Some(Choice::YesNo { .. })) {
                        let yes = matches!(self.answer.take(), Some(Choice::YesNo { yes: true }));
                        self.sba_waiting = false;
                        self.cards[id.0 as usize].cmd_offer = false;
                        if yes {
                            self.move_card(id, Zone::Command, Default::default());
                        }
                        acted = true;
                    } else {
                        let c = &self.cards[id.0 as usize];
                        let zone = if c.zone == Zone::Graveyard { "graveyard" } else { "exile" };
                        self.pending = Some(Decision {
                            player: c.owner,
                            kind: DecisionKind::YesNo { prompt: format!("Put your commander {} into the command zone from {zone}?", self.def_of(id).name), source: Some(id) },
                        });
                        self.sba_waiting = true;
                        return SbaResult::Ask;
                    }
                }
            }
            // legend rule
            if let Some(flow) = self.legend_rule() {
                match flow {
                    LegendFlow::Asked => return SbaResult::Ask,
                    LegendFlow::Acted => acted = true,
                }
            }
            if !acted {
                return SbaResult::Settled;
            }
        }
    }

    /// A static `IgnoreLegendRule` covers this permanent.
    fn ignores_legend_rule(&self, id: CardId) -> bool {
        // "the legend rule doesn't apply to permanents you control this turn" granted to a player
        for e in &self.effects {
            if let Modif::Static(b) = &e.modif {
                if let crate::dsl::StaticEffect::IgnoreLegendRule { affects } = &**b {
                    let ctl = self.cards[id.0 as usize].cur.controller;
                    if e.players.contains(&ctl) && self.matches_card(affects, id, e.source, e.controller) {
                        return true;
                    }
                }
            }
        }
        self.battlefield.iter().any(|&src| {
            let c = &self.cards[src.0 as usize];
            if c.cur.no_abilities {
                return false;
            }
            let d = self.db.def(c.def);
            d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], crate::dsl::Ability::Static { effect: crate::dsl::StaticEffect::IgnoreLegendRule { affects }, .. } if self.matches_card(affects, id, src, c.cur.controller)))
                || c.cur.granted.iter().any(|g| matches!(&**g, crate::dsl::Ability::Static { effect: crate::dsl::StaticEffect::IgnoreLegendRule { affects }, .. } if self.matches_card(affects, id, src, c.cur.controller)))
        })
    }

    fn legend_rule(&mut self) -> Option<LegendFlow> {
        // group legendary permanents by (controller, name)
        let mut groups: Vec<(PlayerId, String, Vec<CardId>)> = vec![];
        for &id in &self.battlefield {
            let c = &self.cards[id.0 as usize];
            if !c.cur.supertypes.contains(&Supertype::Legendary) && !self.is_ring_bearer(id) {
                continue;
            }
            if self.ignores_legend_rule(id) {
                continue;
            }
            let name = c.cur.name.map_or_else(|| self.db.def(c.def).name.clone(), Sym::name);
            if let Some(g) = groups.iter_mut().find(|(p, n, _)| *p == c.cur.controller && *n == name) {
                g.2.push(id);
            } else {
                groups.push((c.cur.controller, name, vec![id]));
            }
        }
        for (p, name, ids) in groups {
            if ids.len() < 2 {
                continue;
            }
            if let (true, Some(Choice::Pick { indices })) = (self.sba_waiting, self.answer.take()) {
                self.sba_waiting = false;
                let keep = indices.first().copied().unwrap_or(0).min(ids.len() - 1);
                for (i, id) in ids.iter().enumerate() {
                    if i != keep {
                        self.move_card(*id, Zone::Graveyard, Default::default());
                    }
                }
                return Some(LegendFlow::Acted);
            }
            self.pending = Some(Decision {
                player: p,
                kind: DecisionKind::Pick {
                    prompt: format!("Legend rule: choose which {name} to keep"),
                    source: None,
                    options: ids.iter().map(|id| Opt::Obj { obj: Obj::card(*id) }).collect(),
                    min: 1,
                    max: 1,
                    cancellable: false,
                    hidden: false,
                    purpose: PickPurpose::Legend,
                },
            });
            self.sba_waiting = true;
            return Some(LegendFlow::Asked);
        }
        None
    }

    /// Ends the game when one player (or none) remains.
    pub fn check_game_over(&mut self) -> bool {
        if self.over.is_some() {
            return true;
        }
        let alive = self.alive_players();
        // Archenemy: the game is over when only one team is left (the first living player of it is named the winner)
        let one_team = alive.len() > 1 && alive.iter().all(|p| self.players[*p as usize].team == self.players[alive[0] as usize].team);
        if (alive.len() <= 1 || one_team) && self.players.len() > 1 {
            if alive.len() == 1 && self.cant_win(alive[0]) {
                return false;
            }
            let result = match alive.first() {
                Some(p) => GameResult::Winner { player: *p },
                None => GameResult::Draw,
            };
            self.over = Some(result.clone());
            self.run = Run::Over;
            self.pending = None;
            self.emit(Event::Over { result });
            return true;
        }
        false
    }
}

enum LegendFlow {
    Asked,
    Acted,
}
