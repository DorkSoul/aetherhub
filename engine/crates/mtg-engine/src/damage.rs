//! Dealing damage, destroying permanents, counters on players and permanents.

use crate::dsl::*;
use crate::eval::EvalCtx;
use crate::ids::*;
use crate::state::*;
use crate::types::*;

impl Game {
    /// Does `target` have protection from the damage source?
    pub(crate) fn protected_from(&self, target: CardId, source: CardId) -> bool {
        self.protected_from_c(target, source, self.cards[source.0 as usize].cur.colors)
    }

    /// Like `protected_from`, but for damage: the colours of a source under `StaticEffect::ColorlessDamageSources` (Ghostly Flame) count as none.
    pub(crate) fn protected_from_dmg(&self, target: CardId, source: CardId) -> bool {
        self.protected_from_c(target, source, self.damage_colors(source))
    }

    /// The colours a damage source is seen to have: none while a `ColorlessDamageSources` static affects it.
    pub(crate) fn damage_colors(&self, source: CardId) -> ColorSet {
        let colors = self.cards[source.0 as usize].cur.colors;
        if colors.0 == 0 {
            return colors;
        }
        for &id in &self.battlefield {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                continue;
            }
            let d = self.db.def(c.def);
            for &i in &d.statics {
                if let Ability::Static { effect: StaticEffect::ColorlessDamageSources { affects }, condition, .. } = &d.abilities[i as usize] {
                    if condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(id, c.cur.controller))) && self.matches_card(affects, source, id, c.cur.controller) {
                        return ColorSet::NONE;
                    }
                }
            }
        }
        colors
    }

    /// For an Aura whose own protection grant "doesn't remove Auras" (Spectra Ward): whether the host has protection from the Aura's colours beyond the entries the Aura itself grants.
    pub(crate) fn protection_beyond_own(&self, host: CardId, aura: CardId) -> bool {
        let colors = self.cards[aura.0 as usize].cur.colors;
        let d = self.def_of(aura);
        let mut own = 0usize;
        for a in &d.abilities {
            if let Ability::Static { effect: StaticEffect::GrantAbility { ability, .. }, .. } = a {
                if let Ability::Protection { from } = &**ability {
                    own += from.iter().filter(|p| match p { ProtectionFrom::Color { c } => colors.has(*c), ProtectionFrom::ChosenColor => self.cards[aura.0 as usize].chosen_color.map_or(false, |c| colors.has(c)), _ => false }).count();
                }
            }
        }
        if own == 0 && d.oracle_text.contains("protection from the chosen color") && self.cards[aura.0 as usize].chosen_color.map_or(false, |c| colors.has(c)) {
            own = 1;
        }
        let have = self.cards[host.0 as usize].cur.protections.iter().filter(|p| matches!(p, ProtectionFrom::Color { c } if colors.has(*c))).count();
        have > own
    }

    fn protected_from_c(&self, target: CardId, source: CardId, colors: ColorSet) -> bool {
        let t = &self.cards[target.0 as usize];
        let s = &self.cards[source.0 as usize];
        t.cur.protections.iter().any(|pr| match pr {
            ProtectionFrom::Color { c } => colors.has(*c),
            ProtectionFrom::Type { t } => s.cur.types.has(*t),
            ProtectionFrom::Everything => true,
            ProtectionFrom::Abilities => false,
            ProtectionFrom::Colorless => colors.count() == 0,
            ProtectionFrom::Monocolored => colors.count() == 1,
            ProtectionFrom::NotColor { c } => !colors.has(*c),
            ProtectionFrom::NonSubtype { name } => s.cur.types.has(CardType::Creature) && !s.cur.subtypes.contains(&Sym::new(name)) && !s.cur.kw.has(Keyword::Changeling),
            ProtectionFrom::Multicolored => colors.count() > 1,
            ProtectionFrom::Subtype { name } => s.cur.subtypes.contains(&Sym::new(name)) || (s.cur.kw.has(Keyword::Changeling) && s.cur.types.has(CardType::Creature)),
            ProtectionFrom::ChosenPlayer => t.chosen_player == Some(s.cur.controller),
            ProtectionFrom::Opponents => s.cur.controller != t.cur.controller,
            ProtectionFrom::ControlledBy { p, .. } => *p == Some(s.cur.controller),
            ProtectionFrom::ChosenColor if colors != s.cur.colors => t.chosen_color.map_or(false, |c| colors.has(c)),
            ProtectionFrom::ChosenColor | ProtectionFrom::ChosenType | ProtectionFrom::Matching { .. } | ProtectionFrom::ArtifactManaValues => self.chosen_prot_hit(t, s, pr),
        })
    }

    /// Deal damage; returns the amount actually dealt.
    /// Prevention shields (Fog, Healing Salve): reduce the damage by whatever matching shields absorb.
    fn apply_shields(&mut self, source: Option<CardId>, target: &Obj, amount: i32, combat: bool) -> (i32, Vec<(Obj, i32, Option<CardId>)>) {
        if self.shields.is_empty() || self.damage_unpreventable() {
            return (amount, vec![]);
        }
        let mut left = amount;
        let mut redirects: Vec<(Obj, i32, Option<CardId>)> = vec![];
        let mut shield_counters: Vec<(Obj, i32)> = vec![];
        let mut end_counters: Vec<(Obj, i32, CounterKind, CardId, PlayerId)> = vec![];
        for sh in self.shields.iter_mut() {
            if left <= 0 {
                break;
            }
            if sh.combat_only && !combat {
                continue;
            }
            if sh.redirect.is_some() && self.redirecting > 0 {
                continue;
            }
            // a redirect to a permanent that left the battlefield no longer applies ("if it's still on the battlefield": Heroic Sacrifice)
            if let Some(Obj::Card { id }) = sh.redirect {
                if self.cards[id.0 as usize].zone != Zone::Battlefield {
                    continue;
                }
            }
            if let Some(tos) = &sh.to {
                if !tos.contains(target) && sh.or_player.map_or(true, |p| *target != Obj::Player { id: p }) {
                    continue;
                }
            }
            if let Some(froms) = &sh.from {
                match source {
                    Some(s) if froms.contains(&s) => {}
                    Some(s) if sh.from_ctl.as_ref().map_or(false, |ps| ps.contains(&self.cards[s.0 as usize].cur.controller) && self.cards[s.0 as usize].zone == Zone::Battlefield && self.cards[s.0 as usize].cur.types.has(CardType::Creature)) => {}
                    _ => continue,
                }
            }
            let before = left;
            if sh.once {
                // only the next damage event: all of it, or the first `amount` points of it when an amount is given
                let keep_open = self.combat_batch && sh.remaining.is_none() && sh.redirect.is_some();
                match sh.remaining {
                    Some(r) if r > 0 => left -= r.min(left),
                    Some(_) => {}
                    None => left = 0,
                }
                if keep_open {
                    // simultaneous combat damage from several sources is one event: the whole of it is redirected
                    let key = (sh.to.clone(), sh.redirect);
                    if !self.batch_once_used.contains(&key) {
                        self.batch_once_used.push(key);
                    }
                } else {
                    sh.remaining = Some(0);
                }
            } else {
                match sh.remaining {
                    None => left = 0,
                    Some(r) if r > 0 => {
                        let used = r.min(left);
                        sh.remaining = Some(r - used);
                        left -= used;
                    }
                    _ => {}
                }
            }
            if sh.counters && before - left > 0 {
                shield_counters.push((*target, before - left));
            }
            if let (Some((k, s, c)), true, Obj::Card { id }) = (sh.end_counters, before - left > 0, target) {
                if self.cards[id.0 as usize].cur.types.has(CardType::Creature) {
                    end_counters.push((*target, before - left, k, s, c));
                }
            }
            if let Some(r) = sh.redirect {
                if before - left > 0 {
                    redirects.push((r, before - left, sh.redirect_by));
                }
            }
        }
        self.shields.retain(|s| s.remaining.map_or(true, |r| r > 0));
        for (o, n) in shield_counters {
            self.add_counters_to(o, CounterKind::PlusOnePlusOne, n);
        }
        for (o, n, kind, source, controller) in end_counters {
            let id = self.next_effect_id;
            self.next_effect_id += 1;
            self.delayed.push(DelayedTrigger {
                id,
                controller,
                source,
                when: DelayedWhen::NextEndStep,
                effects: vec![Effect::AddCounters { what: Sel::Remembered, kind, n: Amount::Fixed { n } }],
                created_turn: self.turn,
                targets: vec![],
                ctx: EventCtx::default(),
                remembered: vec![o],
            });
        }
        (left, redirects)
    }

    /// Remove the one-shot temporary replacements that applied (`once`).
    pub fn drain_consumed_replaces(&mut self) {
        let mut used: Vec<usize> = std::mem::take(&mut *self.consumed_temp.borrow_mut());
        used.sort_unstable();
        used.dedup();
        for i in used.into_iter().rev() {
            if i < self.temp_replaces.len() {
                self.temp_replaces.remove(i);
            }
        }
    }

    /// Illusionary Mask: a face-down creature marked by it is turned face up instead of dealing or being dealt damage or becoming tapped (once).
    pub(crate) fn unmask_creature(&mut self, id: CardId) {
        if !self.cards[id.0 as usize].unmask {
            return;
        }
        self.cards[id.0 as usize].unmask = false;
        if self.cards[id.0 as usize].face_down && self.cards[id.0 as usize].zone == Zone::Battlefield && !self.cant_turn_face_up(id) {
            self.cards[id.0 as usize].face_down = false;
            self.layers_dirty = true;
            self.recompute_layers();
            self.queue_face_up_choice(id);
            self.emit(Event::TurnedFaceUp { card: id });
        }
    }

    pub fn deal_damage(&mut self, source: Option<CardId>, target: Obj, amount: i32, combat: bool) -> i32 {
        if amount > 0 {
            if let Some(s) = source {
                self.unmask_creature(s);
            }
            if let Obj::Card { id } = target {
                self.unmask_creature(id);
            }
        }
        if let Obj::Player { id } = target {
            if self.players[id as usize].damage_shield && !self.damage_unpreventable() {
                return 0;
            }
        }
        // shield counter: damage that would be dealt to the permanent is prevented and a shield counter is removed instead
        if let Obj::Card { id } = target {
            if amount > 0 && self.cards[id.0 as usize].zone == Zone::Battlefield && self.cards[id.0 as usize].counter(CounterKind::Shield) > 0 {
                let unpreventable = self.damage_unpreventable() || self.damage_locked(&target) || self.scoped_unpreventable(source, combat);
                self.remove_counters_from(Obj::card(id), CounterKind::Shield, 1);
                if !unpreventable {
                    return 0;
                }
                // damage that can't be prevented is still dealt, but the shield counter is removed anyway
            }
        }
        let dealt = self.deal_damage_inner(source, target, amount, combat);
        if let (Some(s), true) = (source, dealt > 0) {
            *self.dmg_by_source.borrow_mut().entry(s).or_insert(0) += dealt;
            self.cards[s.0 as usize].dealt_damage_ever = true;
        }
        let reduced: Vec<(usize, i32)> = std::mem::take(&mut *self.reduced_temp.borrow_mut());
        let mut used: Vec<usize> = std::mem::take(&mut *self.consumed_temp.borrow_mut());
        for (i, left) in reduced {
            if i < self.temp_replaces.len() {
                if left <= 0 {
                    used.push(i);
                } else if let (_, _, Replace::Damage { prevent_up_to, .. }) = &mut self.temp_replaces[i] {
                    *prevent_up_to = Some(left);
                }
            }
        }
        used.sort_unstable();
        used.dedup();
        for i in used.into_iter().rev() {
            if i < self.temp_replaces.len() {
                self.temp_replaces.remove(i);
            }
        }
        self.flush_followups();
        dealt
    }


    /// Put the follow-up effects of replacement effects (damage `then`, draw `instead`) on the stack.
    pub(crate) fn flush_followups(&mut self) {
        let follow: Vec<_> = std::mem::take(&mut *self.dmg_followups.borrow_mut());
        let chains = std::mem::take(&mut *self.followup_chains.borrow_mut());
        for (fi, (src, ctrl, effects, amt, dmg_src, dmg_to, player, tgts)) in follow.into_iter().enumerate() {
            let chain = chains.iter().find(|c| c.0 == fi).map(|c| c.1.clone()).unwrap_or_default();
            let did = self.next_effect_id;
            self.next_effect_id += 1;
            let d = DelayedTrigger {
                id: did,
                controller: ctrl,
                source: src,
                when: DelayedWhen::EndOfTurn,
                effects,
                created_turn: self.turn,
                targets: tgts,
                ctx: EventCtx { amount: amt, dmg_combat: self.dmg_combat_now.get(), source: dmg_src, obj: dmg_src.map(Obj::card), damaged: dmg_to, player, cards: chain, ..Default::default() },
                remembered: vec![],
            };
            self.push_delayed_item(d);
        }
    }

    /// A static like Questing Beast's makes this source's (combat) damage unpreventable.
    fn scoped_unpreventable(&self, source: Option<CardId>, combat: bool) -> bool {
        let Some(s) = source else { return false };
        self.battlefield.iter().any(|&id| {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                return false;
            }
            let d = self.db.def(c.def);
            d.statics.iter().any(|&i| match &d.abilities[i as usize] {
                Ability::Static { effect: StaticEffect::SourceDamageCantBePrevented { filter, combat_only }, condition, .. } => {
                    (!*combat_only || combat)
                        && self.matches_card(&Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() }, s, id, c.cur.controller)
                        && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(id, c.cur.controller)))
                }
                _ => false,
            })
        })
    }

    /// A static such as Deification's keeps one loyalty counter on this planeswalker when damage would remove them all.
    fn loyalty_damage_floor(&self, pw: CardId) -> bool {
        self.battlefield.iter().copied().any(|id| {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                return false;
            }
            let ctrl = c.cur.controller;
            let d = self.db.def(c.def);
            d.statics.iter().any(|&i| match &d.abilities[i as usize] {
                Ability::Static { effect: StaticEffect::LoyaltyDamageFloor { affects }, condition, .. } => {
                    self.matches_card(affects, pw, id, ctrl) && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(id, ctrl)))
                }
                _ => false,
            })
        })
    }

    fn deal_damage_inner(&mut self, source: Option<CardId>, target: Obj, amount: i32, combat: bool) -> i32 {
        if !self.force_unprevent.get() && (self.damage_locked(&target) || self.scoped_unpreventable(source, combat)) {
            self.force_unprevent.set(true);
            let r = self.deal_damage_inner2(source, target, amount, combat);
            self.force_unprevent.set(false);
            return r;
        }
        self.deal_damage_inner2(source, target, amount, combat)
    }

    /// Damage to this permanent can't be prevented or redirected this turn (Whippoorwill).
    fn damage_locked(&self, target: &Obj) -> bool {
        matches!(target, Obj::Card { id } if self.cards[id.0 as usize].dmg_lock_turn == self.turn && self.cards[id.0 as usize].zone == Zone::Battlefield)
    }

    /// Excess damage is measured against the creature's toughness even when prevention stops the damage (Ravenous Tyrannosaurus ruling).
    fn note_prevented_excess(&mut self, target: &Obj, orig: i32) {
        if let Obj::Card { id } = target {
            let c = &self.cards[id.0 as usize];
            if orig > 0 && c.zone == Zone::Battlefield && c.cur.types.has(CardType::Creature) {
                let left = (self.lethal_threshold(*id) - c.damage as i32).max(0);
                self.last_damage.set((0, (orig - left).max(0)));
            }
        }
    }

    fn deal_damage_inner2(&mut self, source: Option<CardId>, target: Obj, amount: i32, combat: bool) -> i32 {
        if self.redirecting < 3 && !self.damage_locked(&target) {
            if let Some(new_target) = self.damage_redirect(source, &target, amount, combat) {
                self.redirecting += 1;
                let r = self.deal_damage_inner(source, new_target, amount, combat);
                self.redirecting -= 1;
                return r;
            }
        }
        let orig_amount = amount;
        self.dmg_combat_now.set(combat);
        // damage to a player: the damaged player chooses the order of the effects that apply, and preventing first is never worse for them
        // (Mending Hands before Szadek's "that many counters and mills" replacement leaves only the unprevented damage to be replaced)
        let (amount, redirects) = if matches!(target, Obj::Player { .. }) {
            let (a, r) = self.apply_shields(source, &target, amount, combat);
            if a <= 0 {
                (a, r)
            } else {
                (self.replace_damage(source, &target, a, combat), r)
            }
        } else {
            (self.replace_damage(source, &target, amount, combat), vec![])
        };
        if amount <= 0 && redirects.is_empty() {
            self.note_prevented_excess(&target, orig_amount);
            return 0;
        }
        let (amount, redirects) = if matches!(target, Obj::Player { .. }) { (amount, redirects) } else if amount <= 0 { (amount, redirects) } else { self.apply_shields(source, &target, amount, combat) };
        for (o, n, by) in redirects {
            self.redirecting += 1;
            match by {
                Some(b) => self.deal_damage(Some(b), o, n, false),
                None => self.deal_damage(source, o, n, combat),
            };
            self.redirecting -= 1;
        }
        if amount <= 0 {
            self.note_prevented_excess(&target, orig_amount);
            return 0;
        }
        let toxic: i32 = if !combat {
            0
        } else {
            source.map_or(0, |s| {
                let c = &self.cards[s.0 as usize];
                let live = c.zone == Zone::Battlefield || c.zone == Zone::Stack;
                let kw = if live { c.cur.kw } else { self.lki_slot.as_ref().map_or(c.cur.kw, |l| l.derived.kw) };
                let extra = if live { c.cur.toxic_extra } else { self.lki_slot.as_ref().map_or(c.cur.toxic_extra, |l| l.derived.toxic_extra) } as i32;
                // toxic N: poison counters equal to N for each source with the keyword (they add up)
                [(Keyword::Toxic1, 1), (Keyword::Toxic2, 2), (Keyword::Toxic3, 3), (Keyword::Toxic4, 4), (Keyword::Toxic5, 5), (Keyword::Toxic9, 9)].iter().filter(|(k, _)| kw.has(*k)).map(|(_, n)| *n).sum::<i32>() + extra
            })
        };
        let (infect, wither, deathtouch, lifelink, src_ctrl) = match source {
            Some(s) => {
                let c = &self.cards[s.0 as usize];
                let live = c.zone == Zone::Battlefield || c.zone == Zone::Stack;
                let kw = if live { c.cur.kw } else { self.lki_slot.as_ref().map(|l| l.derived.kw).or_else(|| self.left_bf_kw.iter().rev().find(|(i, _)| *i == s).map(|(_, k)| *k)).unwrap_or(c.cur.kw) };
                (kw.has(Keyword::Infect), kw.has(Keyword::Wither) || self.all_damage_wither(), kw.has(Keyword::Deathtouch), kw.has(Keyword::Lifelink), c.cur.controller)
            }
            None => (false, false, false, false, 0),
        };
        let mut dealt = 0;
        let mut excess = 0;
        match target {
            Obj::Player { id } => {
                if !self.alive(id) {
                    return 0;
                }
                if let Some(s) = source {
                    if self.player_protected_from(id, s) && !self.damage_unpreventable() {
                        return 0;
                    }
                }
                if infect {
                    let prev = self.cause_ctl.get();
                    if let Some(s) = source {
                        self.cause_ctl.set(Some(self.cards[s.0 as usize].cur.controller));
                    }
                    let k = self.replace_player_counters(id, CounterKind::Poison, amount);
                    self.cause_ctl.set(prev);
                    self.players[id as usize].poison += k;
                } else {
                    self.lose_life_from_damage(id, amount);
                }
                dealt = amount;
                if combat && amount > 0 {
                    if let Some(s) = source {
                        if !self.combat_damagers.contains(&s) {
                            self.combat_damagers.push(s);
                            let ctl = self.cards[s.0 as usize].cur.controller;
                            self.combat_damagers_ctl.push((s, ctl));
                        }
                    }
                }
                // the monarchy passes to a player whose creature deals combat damage to the monarch
                if combat && self.monarch == Some(id) {
                    if let Some(s) = source {
                        let sc = &self.cards[s.0 as usize];
                        if sc.cur.types.has(CardType::Creature) && sc.cur.controller != id && !self.effects.iter().any(|e| matches!(e.modif, Modif::NoMonarch) && e.players.contains(&sc.cur.controller)) {
                            let np = sc.cur.controller;
                            self.monarch = Some(np);
                            self.layers_dirty = true;
                            self.emit(Event::Monarch { player: np });
                        }
                    }
                }
                if toxic > 0 {
                    // the counters are put by the damage source's controller (Vorinclex doubles them)
                    let prev = self.cause_ctl.get();
                    if let Some(s) = source {
                        self.cause_ctl.set(Some(self.cards[s.0 as usize].cur.controller));
                    }
                    let k = self.replace_player_counters(id, CounterKind::Poison, toxic);
                    self.cause_ctl.set(prev);
                    self.players[id as usize].poison += k;
                }
                if combat {
                    if let Some(s) = source {
                        if self.cards[s.0 as usize].is_commander {
                            let pl = &mut self.players[id as usize];
                            if let Some(e) = pl.commander_damage.iter_mut().find(|(c, _)| *c == s) {
                                e.1 += amount;
                            } else {
                                pl.commander_damage.push((s, amount));
                            }
                        }
                    }
                }
            }
            Obj::Card { id } => {
                if self.cards[id.0 as usize].zone != Zone::Battlefield {
                    return 0;
                }
                if let Some(s) = source {
                    if self.protected_from_dmg(id, s) && !self.damage_unpreventable() {
                        return 0;
                    }
                }
                let types = self.cards[id.0 as usize].cur.types;
                if types.has(CardType::Creature) {
                    if let Some(s) = source {
                        if amount > 0 && !self.cards[id.0 as usize].damaged_by.contains(&s) {
                            self.cards[id.0 as usize].damaged_by.push(s);
                        }
                    }
                    if infect || wither {
                        // Solemnity / Melira-style "can't get counters" and doubling replacements apply to the -1/-1 counters too
                        let k = self.replace_counters(id, CounterKind::MinusOneMinusOne, amount);
                        if k > 0 {
                            self.cards[id.0 as usize].add_counters(CounterKind::MinusOneMinusOne, k);
                            self.layers_dirty = true;
                            self.emit(Event::Counters { card: id, kind: CounterKind::MinusOneMinusOne, delta: k, by: source.map(|s| self.cards[s.0 as usize].cur.controller).or(self.cause_ctl.get()) });
                        }
                    } else {
                        if self.heals_on_damage(id) {
                            self.cards[id.0 as usize].damage = 0;
                        }
                        let mut left = (self.lethal_threshold(id) - self.cards[id.0 as usize].damage as i32).max(0);
                        // damage from a deathtouch source: 1 is already lethal (Rith, Liberated Primeval)
                        if deathtouch && amount > 0 {
                            left = left.min(1);
                        }
                        excess = (amount - left).max(0);
                        if self.cards[id.0 as usize].damage == 0 {
                            self.cards[id.0 as usize].dmg_parts.clear();
                        }
                        if let (Some(s), true) = (source, amount > 0) {
                            let parts = &mut self.cards[id.0 as usize].dmg_parts;
                            match parts.iter_mut().find(|(c, _)| *c == s) {
                                Some(e) => e.1 += amount,
                                None => parts.push((s, amount)),
                            }
                        }
                        self.cards[id.0 as usize].damage += amount;
                    }
                    if deathtouch {
                        self.cards[id.0 as usize].deathtouch_hit = true;
                    }
                    dealt = amount;
                }
                if types.has(CardType::Planeswalker) {
                    let amount = {
                        let have = self.cards[id.0 as usize].counter(CounterKind::Loyalty);
                        if amount > 0 && have > 0 && amount >= have && self.loyalty_damage_floor(id) {
                            have - 1
                        } else {
                            amount
                        }
                    };
                    let n = self.cards[id.0 as usize].remove_counters(CounterKind::Loyalty, amount);
                    self.layers_dirty = true;
                    dealt = dealt.max(n.max(amount));
                    excess = excess.max((amount - n).max(0));
                    self.emit(Event::Counters { card: id, kind: CounterKind::Loyalty, delta: -n, by: self.cause_ctl.get() });
                }
                if types.has(CardType::Battle) {
                    let n = self.cards[id.0 as usize].remove_counters(CounterKind::Defense, amount);
                    dealt = dealt.max(n.max(amount));
                }
            }
            Obj::Stack { .. } => return 0,
        }
        self.last_damage.set((dealt, excess));
        if dealt > 0 {
            let target_ctrl = match target {
                Obj::Card { id } => {
                    let c = &self.cards[id.0 as usize];
                    if c.cur.types.has(CardType::Creature) || c.cur.types.has(CardType::Planeswalker) { Some(c.cur.controller) } else { None }
                }
                _ => None,
            };
            if let Some(s) = source {
                self.dmg_game.push((s, target, combat));
            }
            if combat && dealt > 0 {
                if let Obj::Player { id } = target {
                    self.players[id as usize].combat_dmg_turn = self.turn;
                }
            }
            self.dmg_log.push(DmgRec { source, target, amount: dealt, combat, excess, target_ctrl, source_ctrl: source.map(|_| src_ctrl), combat_no: self.combats_this_turn });
            if lifelink {
                self.gain_life_from(src_ctrl, dealt, true);
            }
            self.emit(Event::Damage { source, target, amount: dealt, combat });
        }
        dealt
    }

    /// A permanent's printed static "damage that would reduce your life total to less than 1 reduces it to 1 instead" (Fortune Thief).
    fn battlefield_damage_life_floor(&self, p: PlayerId) -> bool {
        self.battlefield_damage_static(p, false)
    }

    /// The highest `DamageLifeFloorAt` floor in force for `p` (Elderscale Wurm).
    fn battlefield_damage_floor_at(&self, p: PlayerId) -> Option<i32> {
        self.battlefield.iter().copied().filter_map(|id| {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                return None;
            }
            let ctrl = c.cur.controller;
            let d = self.db.def(c.def);
            d.statics.iter().filter_map(|&i| match &d.abilities[i as usize] {
                Ability::Static { effect: StaticEffect::DamageLifeFloorAt { who, floor }, condition, .. }
                    if self.player_filter(*who, p, ctrl) && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(id, ctrl))) => Some(*floor),
                _ => None,
            }).max()
        }).max()
    }

    /// A permanent's printed static "damage doesn't cause you to lose life" (Archon of Coronation).
    fn battlefield_damage_no_life_loss(&self, p: PlayerId) -> bool {
        self.battlefield_damage_static(p, true)
    }

    fn battlefield_damage_static(&self, p: PlayerId, no_loss: bool) -> bool {
        // emblems (command zone, Serra the Benevolent) work like printed statics of their owner
        let emblems = self.players.iter().flat_map(|pl| pl.command.iter().copied()).filter(|&e| !self.cards[e.0 as usize].is_commander);
        self.battlefield.iter().copied().chain(emblems).any(|id| {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                return false;
            }
            let ctrl = if c.zone == Zone::Command { c.owner } else { c.cur.controller };
            let d = self.db.def(c.def);
            d.statics.iter().any(|&i| match &d.abilities[i as usize] {
                Ability::Static { effect: StaticEffect::DamageLifeFloor { who }, condition, .. } | Ability::Static { effect: StaticEffect::DamageNoLifeLoss { who }, condition, .. }
                    if matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::DamageNoLifeLoss { .. }, .. }) == no_loss =>
                {
                    self.player_filter(*who, p, ctrl) && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(id, ctrl)))
                }
                _ => false,
            })
        })
    }

    fn lose_life_from_damage(&mut self, p: PlayerId, mut n: i32) {
        // the damage was dealt even if Worship / Archon of Coronation stop the life loss (Knollspine Dragon counts it)
        if n > 0 {
            self.players[p as usize].damaged = true;
            self.players[p as usize].damage_taken_turn += n;
        }
        if n > 0 && self.battlefield_damage_no_life_loss(p) {
            n = 0;
        }
        if n > 0 && (self.player_effect_static(p, |s| matches!(s, StaticEffect::DamageLifeFloor { .. })) || self.battlefield_damage_life_floor(p)) {
            // damage that would take life below 1 sets it to 1 instead, even from 0 or less (Angel's Grace ruling)
            if self.players[p as usize].life < 1 {
                self.players[p as usize].life = 1;
                n = 0;
            }
            n = n.min((self.players[p as usize].life - 1).max(0));
        }
        if n > 0 {
            if let Some(floor) = self.battlefield_damage_floor_at(p) {
                n = n.min((self.players[p as usize].life - floor).max(0));
            }
        }
        self.lose_life(p, n);
    }

    /// Destroy a permanent (indestructible and shield counters respected).
    pub fn destroy(&mut self, id: CardId) -> bool {
        self.destroy_opts(id, true)
    }

    /// Destroy a permanent; a regeneration shield replaces it (tap, remove from combat, remove damage) unless `regen_ok` is false.
    pub fn destroy_opts(&mut self, id: CardId, regen_ok: bool) -> bool {
        let c = &self.cards[id.0 as usize];
        if c.zone != Zone::Battlefield || c.cur.kw.has(Keyword::Indestructible) {
            return false;
        }
        if self.any_replacements() {
            let rep = self.active_replacements().into_iter().find(|r| {
                matches!(&r.what, Replace::DestroyInsteadSacSource { filter } if self.cards[r.source.0 as usize].zone == Zone::Battlefield && self.matches_card(filter, id, r.source, r.controller))
            });
            if let Some(r) = rep {
                self.move_card(r.source, Zone::Graveyard, Default::default());
                self.add_effect(r.source, r.controller, &[id], Modif::Keyword(Keyword::Indestructible), Duration::UntilEndOfTurn);
                return false;
            }
        }
        if self.any_replacements() {
            let rep = self.active_replacements().into_iter().find(|r| {
                matches!(&r.what, Replace::DestroyInsteadRemoveCounters { filter } if self.matches_card(filter, id, r.source, r.controller))
            });
            if rep.is_some() {
                let n = self.cards[id.0 as usize].counter(CounterKind::PlusOnePlusOne);
                self.remove_counters_from(Obj::card(id), CounterKind::PlusOnePlusOne, n);
                let c = &mut self.cards[id.0 as usize];
                c.damage = 0;
                c.deathtouch_hit = false;
                return false;
            }
        }
        let c = &self.cards[id.0 as usize];
        if c.heal_shield {
            let c = &mut self.cards[id.0 as usize];
            c.heal_shield = false;
            c.damage = 0;
            c.deathtouch_hit = false;
            return false;
        }
        let regen_ok = regen_ok
            && c.no_regen_turn != self.turn
            && !(!self.no_regen_dealers.is_empty() && c.damaged_by.iter().any(|d| self.no_regen_dealers.iter().any(|(s, t)| s == d && *t == self.turn)));
        let auto_regen = regen_ok
            && self.any_replacements()
            && self.active_replacements().iter().any(|r| matches!(&r.what, Replace::DestroyRegenerate { filter } if self.matches_card(filter, id, r.source, r.controller)));
        let c = &self.cards[id.0 as usize];
        if regen_ok && (c.regen > 0 || auto_regen) {
            let can_tap = self.can_become_tapped(id, false);
            let c = &mut self.cards[id.0 as usize];
            // each regeneration shield is used up by one destruction; a replacement that regenerates uses none
            c.regen = c.regen.saturating_sub(1);
            if c.regen_count_turn != self.turn {
                c.regen_count_turn = self.turn;
                c.regen_count = 0;
            }
            c.regen_count = c.regen_count.saturating_add(1);
            let steal = c.regen_steal.take();
            c.tapped |= can_tap;
            c.damage = 0;
            c.deathtouch_hit = false;
            if let Some(cb) = self.combat.as_mut() {
                cb.attackers.retain(|a| a.card != id);
                for a in cb.attackers.iter_mut() {
                    a.blockers.retain(|b| *b != id);
                }
            }
            self.emit(Event::Tapped { card: id });
            self.emit(Event::Regenerated { card: id });
            if let Some((src, ctl)) = steal {
                self.add_effect(src, ctl, &[id], Modif::Control(ctl), Duration::Forever);
            }
            return false;
        }
        if c.counter(CounterKind::Shield) > 0 {
            self.cards[id.0 as usize].damage = 0;
            self.remove_counters_from(Obj::card(id), CounterKind::Shield, 1);
            return false;
        }
        // umbra armor: an Aura on it is destroyed instead (not regeneration: "can't be regenerated" does not stop it)
        {
            let aura = self.battlefield.iter().copied().find(|a| {
                self.cards[a.0 as usize].attached_to == Some(id) && (self.db.def(self.cards[a.0 as usize].def).abilities.iter().any(|x| matches!(x, Ability::UmbraArmor)) || self.cards[a.0 as usize].cur.granted.iter().any(|x| matches!(**x, Ability::UmbraArmor)))
            });
            // umbra armor granted to the creature by an Aura ("enchanted creature has umbra armor"): that Aura is the one destroyed
            let aura = aura.or_else(|| {
                let c = &self.cards[id.0 as usize];
                c.cur.granted.iter().zip(c.cur.granted_by.iter()).find_map(|(g, by)| {
                    (matches!(**g, Ability::UmbraArmor) && self.cards[by.0 as usize].zone == Zone::Battlefield && *by != id).then_some(*by)
                })
            });
            if let Some(a) = aura {
                let c = &mut self.cards[id.0 as usize];
                c.damage = 0;
                c.deathtouch_hit = false;
                self.move_card(a, Zone::Graveyard, Default::default());
                return false;
            }
        }
        let ctl = self.cards[id.0 as usize].cur.controller;
        let noncreature = !self.cards[id.0 as usize].cur.types.has(CardType::Creature);
        self.cause_destroy.set(true);
        let moved = self.move_card(id, Zone::Graveyard, Default::default());
        self.cause_destroy.set(false);
        if moved && noncreature && self.cause_ctl.get().map_or(false, |c| c != ctl) {
            self.nc_destroyed_by_opp |= 1 << ctl;
        }
        moved
    }

    pub fn add_counters_to(&mut self, o: Obj, kind: CounterKind, n: i32) {
        if n <= 0 {
            return;
        }
        // a spell on the stack ("put two charge counters on Lightning Storm") is addressed as a stack object
        let o = match o {
            Obj::Stack { id } => match self.stack.iter().find(|s| s.id == id) {
                Some(it) if matches!(it.kind, ItemKind::Spell { .. }) => Obj::card(it.source),
                _ => return,
            },
            other => other,
        };
        let n = match o {
            Obj::Card { id } => self.replace_counters(id, kind, n),
            Obj::Player { id } => self.replace_player_counters(id, kind, n),
            _ => n,
        };
        if n <= 0 {
            return;
        }
        match o {
            Obj::Card { id } => {
                if matches!(self.cards[id.0 as usize].zone, Zone::Battlefield | Zone::Exile | Zone::Stack | Zone::Command) {
                    let mut n = n;
                    let cap = self.def_of(id).abilities.iter().filter_map(|a| if let Ability::Static { effect: StaticEffect::MaxCounters { kind: k, max }, .. } = a { if *k == kind { Some(*max as i32) } else { None } } else { None }).min();
                    if let Some(cap) = cap {
                        n = n.min(cap - self.cards[id.0 as usize].counter(kind)).max(0);
                        if n == 0 {
                            return;
                        }
                    }
                    self.cards[id.0 as usize].add_counters(kind, n);
                    if let Some(by) = self.cause_ctl.get() {
                        let turn = self.turn;
                        let c = &mut self.cards[id.0 as usize];
                        if c.counter_put_turn != turn {
                            c.counter_put_turn = turn;
                            c.counter_put_mask = 0;
                        }
                        c.counter_put_mask |= 1 << by;
                    }
                    self.layers_dirty = true;
                    self.emit(Event::Counters { card: id, kind, delta: n, by: self.cause_ctl.get() });
                }
            }
            Obj::Player { id } => match kind {
                CounterKind::Poison => {
                    self.players[id as usize].poison += n;
                    self.layers_dirty = true;
                    self.emit(Event::PlayerCounters { player: id, kind, delta: n, by: self.cause_ctl.get() });
                }
                CounterKind::Energy => {
                    self.players[id as usize].energy += n;
                    self.layers_dirty = true;
                    self.emit(Event::EnergyGained { player: id, n });
                    self.emit(Event::PlayerCounters { player: id, kind, delta: n, by: self.cause_ctl.get() });
                }
                CounterKind::Experience => {
                    self.players[id as usize].experience += n;
                    self.layers_dirty = true;
                    self.emit(Event::PlayerCounters { player: id, kind, delta: n, by: self.cause_ctl.get() });
                }
                CounterKind::Rad => {
                    self.players[id as usize].rad += n;
                    self.emit(Event::PlayerCounters { player: id, kind, delta: n, by: self.cause_ctl.get() });
                }
                _ => {}
            },
            _ => {}
        }
    }

    /// Remove `n` counters of any kinds from a permanent, the most plentiful kind first.
    pub fn remove_any_counters(&mut self, card: CardId, n: i32) {
        let mut left = n;
        while left > 0 {
            let Some((k, _)) = self.cards[card.0 as usize].counters.iter().filter(|(_, x)| *x > 0).max_by_key(|(_, x)| *x).copied() else { break };
            self.remove_counters_from(Obj::card(card), k, 1);
            left -= 1;
        }
    }

    /// Can `n` counters of any kinds in total be removed from permanents `p` controls that match `filter`?
    pub(crate) fn any_counters_removable(&self, filter: &Filter, n: i32, source: CardId, p: PlayerId) -> bool {
        let mut f = filter.clone();
        f.controller = PlayerFilter::Any;
        let total: i32 = self
            .battlefield
            .iter()
            .filter(|id| self.cards[id.0 as usize].cur.controller == p && self.matches_card(&f, **id, source, p))
            .map(|id| self.cards[id.0 as usize].counters.iter().map(|(_, x)| *x as i32).sum::<i32>())
            .sum();
        total >= n
    }

    pub(crate) fn remove_any_counters_among(&mut self, filter: &Filter, n: i32, source: CardId, p: PlayerId) {
        let mut f = filter.clone();
        f.controller = PlayerFilter::Any;
        let ids: Vec<CardId> = self.battlefield.iter().copied().filter(|id| self.cards[id.0 as usize].cur.controller == p && self.matches_card(&f, *id, source, p)).collect();
        let mut left = n;
        for id in ids {
            while left > 0 && self.cards[id.0 as usize].counters.iter().any(|(_, x)| *x > 0) {
                self.remove_any_counters(id, 1);
                left -= 1;
            }
        }
    }

    pub fn remove_counters_from(&mut self, o: Obj, kind: CounterKind, n: i32) {
        match o {
            Obj::Card { id } => {
                if kind == CounterKind::Stun && self.stun_locked(id) {
                    return;
                }
                let took = self.cards[id.0 as usize].remove_counters(kind, n);
                if took > 0 {
                    self.counters_removed.set(self.counters_removed.get() + took as i32);
                    self.layers_dirty = true;
                    self.emit(Event::Counters { card: id, kind, delta: -took, by: self.cause_ctl.get() });
                }
            }
            Obj::Player { id } => match kind {
                CounterKind::Poison => self.players[id as usize].poison = (self.players[id as usize].poison - n).max(0),
                CounterKind::Energy => {
                    let lost = n.min(self.players[id as usize].energy).max(0);
                    self.players[id as usize].energy -= lost;
                    self.players[id as usize].energy_spent += lost;
                }
                CounterKind::Experience => {
                    self.players[id as usize].experience = (self.players[id as usize].experience - n).max(0);
                    self.layers_dirty = true;
                }
                CounterKind::Rad => self.players[id as usize].rad = (self.players[id as usize].rad - n).max(0),
                _ => {}
            },
            _ => {}
        }
    }

    pub fn tap_card(&mut self, id: CardId) {
        if !self.cards[id.0 as usize].tapped && self.can_become_tapped(id, self.declaring_attacker.get()) {
            self.cards[id.0 as usize].tapped = true;
            self.emit(Event::Tapped { card: id });
        }
    }

    pub fn untap_card(&mut self, id: CardId) {
        if self.cards[id.0 as usize].tapped {
            self.cards[id.0 as usize].tapped = false;
            self.layers_dirty = true;
            self.emit(Event::Untapped { card: id });
        }
    }

    /// "If you would lose the game, instead ...": applies the first matching replacement and returns true when the loss was replaced.
    fn lose_game_replaced(&mut self, p: PlayerId) -> bool {
        if !self.any_replacements() {
            return false;
        }
        let found = self.active_replacements().into_iter().find(|r| r.controller == p && matches!(r.what, Replace::LoseGame { .. }));
        let Some(r) = found else { return false };
        let Replace::LoseGame { exile_source, shuffle_all, draw, life, starting_life } = r.what else { return false };
        if let Some(i) = r.temp_idx {
            self.consumed_temp.borrow_mut().push(i);
            self.drain_consumed_replaces();
        }
        if exile_source && self.cards[r.source.0 as usize].zone != Zone::Exile {
            self.move_card(r.source, Zone::Exile, Default::default());
        }
        if shuffle_all {
            let owned: Vec<CardId> = self
                .cards
                .iter()
                .filter(|c| c.owner == p && matches!(c.zone, Zone::Hand | Zone::Graveyard | Zone::Battlefield))
                .map(|c| c.id)
                .collect();
            for id in owned {
                self.move_card(id, Zone::Library, Default::default());
            }
            self.shuffle_library(p);
        }
        for _ in 0..draw {
            self.draw_card(p);
        }
        let target = if starting_life { Some(self.cfg.starting_life) } else { life };
        if let Some(t) = target {
            let now = self.players[p as usize].life;
            if t > now {
                self.gain_life(p, t - now);
            } else if t < now {
                self.players[p as usize].life = t;
            }
        }
        true
    }

    pub fn player_loses(&mut self, p: PlayerId, reason: &str) {
        if self.players[p as usize].lost || (self.cant_lose(p) && reason != "conceded") {
            return;
        }
        if reason != "conceded" && self.lose_game_replaced(p) {
            return;
        }
        self.players[p as usize].lost = true;
        self.emit(Event::Lost { player: p, reason: reason.to_string() });
        // Rule 800.4a applies when the game goes on without them (multiplayer). When this loss ends the game, the board is
        // left as it was at the end (their permanents and spells are removed, the rest stays for the record).
        let others_left = self.players.iter().filter(|q| q.id != p && !q.lost).count();
        if others_left < 2 {
            let owned: Vec<CardId> = self.cards.iter().filter(|c| c.owner == p && matches!(c.zone, Zone::Battlefield | Zone::Stack)).map(|c| c.id).collect();
            for id in owned {
                if self.cards[id.0 as usize].zone == Zone::Stack {
                    self.stack.retain(|s| s.source != id);
                    self.cards[id.0 as usize].zone = Zone::Exile;
                } else {
                    self.cards[id.0 as usize].token = true; // ceases to exist
                    self.move_card(id, Zone::Exile, Default::default());
                }
            }
            self.stack.retain(|s| s.controller != p);
            self.layers_dirty = true;
            if self.monarch == Some(p) {
                self.monarch = None;
            }
            return;
        }
        // Rule 800.4a, in its order. 1) Every object the player owns leaves the game: permanents (as if they had never
        // existed, so they are not "put into" any zone), spells, and the cards in every other zone.
        // Planechase: the face-up planes pass to the next player still in the game (the planar controller) and the
        // planar deck stays; neither leaves with the player
        let heir = self.next_player(p);
        for id in self.planes.clone() {
            if self.cards[id.0 as usize].owner == p && heir != p {
                self.players[p as usize].command.retain(|x| *x != id);
                self.players[heir as usize].command.push(id);
                self.cards[id.0 as usize].owner = heir;
                self.cards[id.0 as usize].controller = heir;
            }
        }
        let shared: Vec<CardId> = self.planes.iter().chain(self.planar_deck.iter()).copied().collect();
        let owned: Vec<CardId> = self.cards.iter().filter(|c| c.owner == p && c.zone != Zone::Sideboard && !shared.contains(&c.id)).map(|c| c.id).collect();
        for id in owned {
            match self.cards[id.0 as usize].zone {
                Zone::Battlefield => {
                    self.cards[id.0 as usize].token = true; // ceases to exist
                    self.move_card(id, Zone::Exile, Default::default());
                }
                Zone::Stack => {
                    self.stack.retain(|s| s.source != id || !matches!(s.kind, ItemKind::Spell { .. }));
                    self.cards[id.0 as usize].zone = Zone::Sideboard;
                }
                // hand, library, graveyard, exile, command zone: simply out of the game
                _ => self.cards[id.0 as usize].zone = Zone::Sideboard,
            }
        }
        {
            let pl = &mut self.players[p as usize];
            pl.library.clear();
            pl.hand.clear();
            pl.graveyard.clear();
            pl.exile.clear();
            pl.command.clear();
            pl.scheme_deck.clear();
        }
        // 2) Effects that give the player control of anything end: the control layer never gives control to a player who
        // has left (layers.rs, Act::Control), so stolen permanents go back to whoever controls them otherwise.
        self.layers_dirty = true;
        self.recompute_layers();
        // 3) Objects on the stack they control that aren't cards cease to exist; spells they control (owned by someone
        // else) are exiled, like 4) any other object they still control.
        let spells: Vec<CardId> = self.stack.iter().filter(|s| s.controller == p && matches!(s.kind, ItemKind::Spell { .. })).map(|s| s.source).collect();
        self.stack.retain(|s| s.controller != p);
        for id in spells {
            if self.cards[id.0 as usize].zone == Zone::Stack {
                self.move_card(id, Zone::Exile, Default::default());
            }
        }
        let still: Vec<CardId> = self.cards.iter().filter(|c| c.zone == Zone::Battlefield && c.cur.controller == p).map(|c| c.id).collect();
        for id in still {
            self.move_card(id, Zone::Exile, Default::default());
        }
        self.layers_dirty = true;
        if self.monarch == Some(p) {
            self.monarch = None;
            self.layers_dirty = true;
        }
    }
}
