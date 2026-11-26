//! Replacement and prevention effects (CR 614/615): static `Replace` abilities of permanents modify events as they happen.
//! Multiple applicable effects apply in timestamp order (the affected player's choice is not modelled).

use crate::dsl::*;
use crate::eval::EvalCtx;
use crate::ids::*;
use crate::state::*;
use crate::types::*;

pub struct ActiveReplace {
    pub source: CardId,
    pub controller: PlayerId,
    pub what: Replace,
    /// Index into `Game.temp_replaces` when this came from `Effect::AddReplacement`.
    pub temp_idx: Option<usize>,
}

impl Game {
    /// Sum of static `AdditionalLandDrop`s of permanents `p` controls (on top of the normal one land per turn).
    pub fn extra_land_drops(&self, p: PlayerId) -> i32 {
        let mut n = 0;
        // face-up planes (Naya) carry statics too
        for id in self.battlefield.iter().copied().chain(self.emblem_srcs()) {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                continue;
            }
            let d = self.db.def(c.def);
            let ctl = self.src_ctrl(id);
            for &i in &d.statics {
                if let Ability::Static { effect: StaticEffect::AdditionalLandDrop { n: k, who }, condition, .. } = &d.abilities[i as usize] {
                    let applies = match who {
                        None => ctl == p,
                        Some(w) => self.player_filter_src(id, *w, p, ctl),
                    };
                    if applies && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &crate::eval::EvalCtx::with_controller(id, ctl))) {
                        n += k;
                    }
                }
            }
        }
        n
    }

    /// "Players can't play lands" (a static of a permanent, or a one-shot restriction on the player).
    pub fn cant_play_lands(&self, p: PlayerId) -> bool {
        if self.effects.iter().any(|e| matches!(e.modif, Modif::NoLandPlay) && e.players.contains(&p)) {
            return true;
        }
        for &id in &self.battlefield {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                continue;
            }
            let d = self.db.def(c.def);
            let ctrl = c.cur.controller;
            for &i in &d.statics {
                if let Ability::Static { effect: StaticEffect::CantPlayLands { who, filter: None }, condition, .. } = &d.abilities[i as usize] {
                    if self.player_filter_src(id, *who, p, ctrl) && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &crate::eval::EvalCtx::with_controller(id, ctrl))) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// "Lands with the chosen name can't be played": filtered `CantPlayLands` statics (permanents, or granted for a while by a resolved spell) checked against this land card, plus the unfiltered ones.
    pub fn cant_play_land_card(&self, p: PlayerId, card: CardId) -> bool {
        if self.cant_play_lands(p) {
            return true;
        }
        if self.cards[card.0 as usize].zone == Zone::Graveyard
            && self.battlefield.iter().any(|&id| {
                let c = &self.cards[id.0 as usize];
                let d = self.db.def(c.def);
                !c.cur.no_abilities
                    && d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::CantPlayLandsFromGraveyard { who }, condition, .. }
                        if self.player_filter_src(id, *who, p, c.cur.controller) && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &crate::eval::EvalCtx::with_controller(id, c.cur.controller)))))
            })
        {
            return true;
        }
        for &id in &self.battlefield {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                continue;
            }
            let d = self.db.def(c.def);
            let ctrl = c.cur.controller;
            for &i in &d.statics {
                if let Ability::Static { effect: StaticEffect::CantPlayLands { who, filter: Some(f) }, condition, .. } = &d.abilities[i as usize] {
                    if self.player_filter_src(id, *who, p, ctrl) && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &crate::eval::EvalCtx::with_controller(id, ctrl))) && self.matches_card(f, card, id, ctrl) {
                        return true;
                    }
                }
            }
        }
        for e in &self.effects {
            if let Modif::DynStatic(b) = &e.modif {
                if let StaticEffect::CantPlayLands { who, filter: Some(f) } = &**b {
                    if self.player_filter_src(e.source, *who, p, e.controller) && self.matches_card(f, card, e.source, e.controller) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Tamiyo, Collector of Tales: the current effect is controlled by an opponent of `p`, and `p` controls a permanent barring discards/sacrifices.
    pub fn forced_discard_sac_barred(&self, p: PlayerId) -> bool {
        self.forced_barred(p, false)
    }

    /// Like `forced_discard_sac_barred` but for sacrifices only: Tajuru Preserver's sacrifice-only static also counts.
    pub fn forced_sac_barred(&self, p: PlayerId) -> bool {
        self.forced_barred(p, true)
    }

    fn forced_barred(&self, p: PlayerId, sac: bool) -> bool {
        match self.cause_ctl.get() {
            Some(c) if c != p => {}
            _ => return false,
        }
        self.battlefield.iter().any(|&id| {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities || c.cur.controller != p {
                return false;
            }
            let d = self.db.def(c.def);
            d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::OpponentsCantForceDiscardSacrifice, .. }) || (sac && matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::OpponentsCantForceSacrifice, .. })))
        })
    }

    /// "Players can't search libraries": a static of a permanent stops `p`'s searches.
    /// A `StaticEffect::SkipUntapStep` (Stasis) on the battlefield makes `p` skip their untap step.
    pub fn untap_step_skipped_by_static(&self, p: PlayerId) -> bool {
        for id in self.battlefield.iter().copied().chain(self.emblem_srcs()) {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                continue;
            }
            let d = self.db.def(c.def);
            let ctrl = self.src_ctrl(id);
            for &i in &d.statics {
                if let Ability::Static { effect: StaticEffect::SkipUntapStep { who }, condition, .. } = &d.abilities[i as usize] {
                    if self.player_filter_src(id, *who, p, ctrl) && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &crate::eval::EvalCtx::with_controller(id, ctrl))) {
                        return true;
                    }
                }
            }
        }
        false
    }

    pub fn cant_search_libraries(&self, p: PlayerId) -> bool {
        if self.effects.iter().any(|e| e.players.contains(&p) && matches!(&e.modif, Modif::Static(b) if matches!(&**b, StaticEffect::IgnoreSearchBan))) {
            return false;
        }
        for &id in &self.battlefield {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                continue;
            }
            let d = self.db.def(c.def);
            let ctrl = c.cur.controller;
            for &i in &d.statics {
                if let Ability::Static { effect: StaticEffect::CantSearchLibraries { who }, condition, .. } = &d.abilities[i as usize] {
                    if self.player_filter_src(id, *who, p, ctrl) && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &crate::eval::EvalCtx::with_controller(id, ctrl))) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Attack limits from statics: (source, controller, which defenders, max attackers).
    pub fn attack_limits(&self) -> Vec<(CardId, PlayerId, PlayerFilter, u8)> {
        let mut v = vec![];
        for id in self.battlefield.iter().copied().chain(self.emblem_srcs()) {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                continue;
            }
            let d = self.db.def(c.def);
            let ctrl = self.src_ctrl(id);
            for &i in &d.statics {
                if let Ability::Static { effect: StaticEffect::MaxAttackers { defending, n }, condition, .. } = &d.abilities[i as usize] {
                    if condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &crate::eval::EvalCtx::with_controller(id, ctrl))) {
                        v.push((id, ctrl, *defending, *n));
                    }
                }
            }
        }
        v
    }

    /// Whether a "must attack with at least one creature" static applies to player `p`.
    pub fn must_attack_one(&self, p: PlayerId) -> bool {
        for &id in &self.battlefield {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                continue;
            }
            let d = self.db.def(c.def);
            let ctrl = c.cur.controller;
            for &i in &d.statics {
                if let Ability::Static { effect: StaticEffect::MustAttackOne { who }, condition, .. } = &d.abilities[i as usize] {
                    if condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &crate::eval::EvalCtx::with_controller(id, ctrl))) && self.player_filter_src(id, *who, p, ctrl) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Whether an "if a creature attacks, all creatures must attack" static applies to player `p`.
    pub fn must_attack_all_if_any(&self, p: PlayerId) -> bool {
        for &id in &self.battlefield {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                continue;
            }
            let d = self.db.def(c.def);
            let ctrl = c.cur.controller;
            for &i in &d.statics {
                if let Ability::Static { effect: StaticEffect::MustAttackAllIfAny { who }, condition, .. } = &d.abilities[i as usize] {
                    if condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &crate::eval::EvalCtx::with_controller(id, ctrl))) && self.player_filter_src(id, *who, p, ctrl) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// The counter kinds of the in-force "if a creature with a counter attacks, all creatures with such counters attack if able" statics (Magnetic Web).
    pub fn counter_attack_kinds(&self) -> Vec<CounterKind> {
        let mut out = vec![];
        for &id in &self.battlefield {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                continue;
            }
            let d = self.db.def(c.def);
            let ctrl = c.cur.controller;
            for &i in &d.statics {
                if let Ability::Static { effect: StaticEffect::MustAttackIfCounteredAttacks { kind }, condition, .. } = &d.abilities[i as usize] {
                    if condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &crate::eval::EvalCtx::with_controller(id, ctrl))) && !out.contains(kind) {
                        out.push(*kind);
                    }
                }
            }
        }
        out
    }

    /// Whether stun counters can't be removed from `id` (Fear of Sleep Paralysis).
    pub fn stun_locked(&self, id: CardId) -> bool {
        for &sid in &self.battlefield {
            let c = &self.cards[sid.0 as usize];
            if c.cur.no_abilities {
                continue;
            }
            let d = self.db.def(c.def);
            let ctrl = c.cur.controller;
            for &i in &d.statics {
                if let Ability::Static { effect: StaticEffect::StunCountersCantBeRemoved { affects }, condition, .. } = &d.abilities[i as usize] {
                    if condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &crate::eval::EvalCtx::with_controller(sid, ctrl))) && self.matches_card(affects, id, sid, ctrl) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// This creature has "if a creature you control attacks, this creature also attacks if able".
    pub fn attacks_if_any_attacks(&self, id: CardId) -> bool {
        let c = &self.cards[id.0 as usize];
        if c.cur.no_abilities {
            return false;
        }
        let d = self.db.def(c.def);
        d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::AttacksIfAnyAttacks, condition, .. }
            if condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &crate::eval::EvalCtx::with_controller(id, c.cur.controller)))))
    }

    /// The smallest "no more than N creatures can block each combat" cap in play (None = no cap).
    pub fn block_total_limit(&self) -> Option<u8> {
        let mut lim: Option<u8> = None;
        for id in self.battlefield.iter().copied().chain(self.emblem_srcs()) {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                continue;
            }
            let d = self.db.def(c.def);
            for &i in &d.statics {
                if let Ability::Static { effect: StaticEffect::MaxBlockersTotal { n }, condition, .. } = &d.abilities[i as usize] {
                    if condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &crate::eval::EvalCtx::with_controller(id, self.src_ctrl(id)))) {
                        lim = Some(lim.map_or(*n, |l| l.min(*n)));
                    }
                }
            }
        }
        lim
    }

    /// True while a permanent says the attacking player chooses how creatures block.
    pub fn attacker_chooses_blocks(&self) -> bool {
        for &id in &self.battlefield {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                continue;
            }
            let d = self.db.def(c.def);
            for &i in &d.statics {
                if let Ability::Static { effect: StaticEffect::AttackerChoosesBlocks, condition, .. } = &d.abilities[i as usize] {
                    if condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &crate::eval::EvalCtx::with_controller(id, c.cur.controller))) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Maximum hand size for `p` (None = no maximum).
    pub fn max_hand_size(&self, p: PlayerId) -> Option<i32> {
        let mut max = Some(self.players[p as usize].max_hand);
        // "is N" and "is increased by N" effects apply in timestamp order (Null Profusion and Minamo Scrollkeeper)
        let mut ops: Vec<(u64, Option<Option<i32>>, i32)> = vec![];
        for &id in &self.battlefield {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                continue;
            }
            let mine = c.cur.controller == p;
            let d = self.db.def(c.def);
            for &i in &d.statics {
                if let Ability::Static { effect, condition, .. } = &d.abilities[i as usize] {
                    if let Some(cd) = condition {
                        if !self.eval_cond(cd, &EvalCtx::with_controller(id, c.cur.controller)) {
                            continue;
                        }
                    }
                    match effect {
                        StaticEffect::NoMaxHandSize if mine => ops.push((c.ts, Some(None), 0)),
                        StaticEffect::HandSizeUnlimited { who } if self.player_filter(*who, p, c.cur.controller) => ops.push((c.ts, Some(None), 0)),
                        StaticEffect::MaxHandSize { n } if mine => ops.push((c.ts, Some(Some(*n)), 0)),
                        StaticEffect::MaxHandSizeFor { who, n } if self.player_filter_src(id, *who, p, c.cur.controller) => ops.push((c.ts, Some(Some(*n)), 0)),
                        StaticEffect::MaxHandSizeDelta { who, delta } if self.player_filter(*who, p, c.cur.controller) => ops.push((c.ts, None, *delta)),
                        _ => {}
                    }
                }
            }
        }
        ops.sort_by_key(|o| o.0);
        let mut hand_delta = 0;
        for (_, set, d) in &ops {
            match set {
                Some(n) => {
                    max = *n;
                    hand_delta = 0;
                }
                None => hand_delta += *d,
            }
        }
        if self.effects.iter().any(|e| e.players.contains(&p) && matches!(&e.modif, Modif::Static(se) if matches!(**se, StaticEffect::NoMaxHandSize))) {
            return None;
        }
        // emblems and face-up planes / phenomena / schemes in the command zone (statics with no `from` work from there)
        for id in self.emblem_srcs() {
            let ctl = self.src_ctrl(id);
            let d = self.db.def(self.cards[id.0 as usize].def);
            for a in &d.abilities {
                match a {
                    Ability::Static { effect: StaticEffect::NoMaxHandSize, .. } if ctl == p => return None,
                    Ability::Static { effect: StaticEffect::HandSizeUnlimited { who }, .. } if self.player_filter(*who, p, ctl) => return None,
                    _ => {}
                }
            }
        }
        // statics that work from exile (Praetor's Counsel)
        for &id in &self.players[p as usize].exile {
            let d = self.db.def(self.cards[id.0 as usize].def);
            for &i in &d.statics {
                match &d.abilities[i as usize] {
                    Ability::Static { effect: StaticEffect::NoMaxHandSize, from: Some(Zone::Exile), .. } => return None,
                    Ability::Static { effect: StaticEffect::HandSizeUnlimited { who }, from: Some(Zone::Exile), .. } if self.player_filter(*who, p, self.cards[id.0 as usize].owner) => return None,
                    _ => {}
                }
            }
        }
        max.map(|m| (m + hand_delta).max(0))
    }

    /// Does a static ability stop `p` from losing the game (Platinum Angel)?
    /// A static says `p` doesn't lose for having 0 or less life.
    pub fn dont_lose_at_zero(&self, p: PlayerId) -> bool {
        self.battlefield.iter().any(|&id| {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                return false;
            }
            let d = self.db.def(c.def);
            d.statics.iter().any(|&i| match &d.abilities[i as usize] {
                Ability::Static { effect: StaticEffect::NoLoseAtZeroLife { who }, condition, .. } => {
                    self.player_filter(*who, p, c.cur.controller) && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &crate::eval::EvalCtx::with_controller(id, c.cur.controller)))
                }
                _ => false,
            })
        })
    }

    /// A resolved effect ("players can't lose the game this turn") that grants `pred` to player `p`.
    pub(crate) fn player_effect_static(&self, p: PlayerId, pred: impl Fn(&StaticEffect) -> bool) -> bool {
        self.effects.iter().any(|e| matches!(&e.modif, Modif::Static(b) if pred(b)) && e.players.contains(&p))
    }

    /// The permanent and controller (if any) that controls `searcher`'s library searches (`StaticEffect::ControlSearches`).
    pub fn search_controller(&self, searcher: PlayerId) -> Option<(CardId, PlayerId)> {
        self.battlefield.iter().find_map(|&id| {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                return None;
            }
            let d = self.db.def(c.def);
            d.statics.iter().any(|&i| match &d.abilities[i as usize] {
                Ability::Static { effect: StaticEffect::ControlSearches { who }, condition, .. } => {
                    self.player_filter(*who, searcher, c.cur.controller) && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(id, c.cur.controller)))
                }
                _ => false,
            })
            .then_some((id, c.cur.controller))
        })
    }

    /// Is this player barred from another venture this turn (`StaticEffect::VentureLimit`, once they have already ventured)?
    pub fn venture_barred(&self, p: PlayerId) -> bool {
        let vt = self.players[p as usize].venture_turn;
        if vt.0 != self.turn || vt.1 < 1 {
            return false;
        }
        self.battlefield.iter().any(|&id| {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                return false;
            }
            let d = self.db.def(c.def);
            d.statics.iter().any(|&i| match &d.abilities[i as usize] {
                Ability::Static { effect: StaticEffect::VentureLimit { who }, condition, .. } => {
                    self.player_filter(*who, p, c.cur.controller) && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(id, c.cur.controller)))
                }
                _ => false,
            })
        })
    }

    /// Does a static ability (or a resolved effect) keep this player's unspent mana of type `t` from emptying?
    pub fn mana_persists(&self, p: PlayerId, t: crate::mana::ManaType) -> bool {
        if self.player_effect_static(p, |s| matches!(s, StaticEffect::ManaPersists { colors, .. } if colors.is_empty() || colors.contains(&t))) {
            return true;
        }
        self.battlefield.iter().any(|&id| {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                return false;
            }
            let d = self.db.def(c.def);
            d.statics.iter().any(|&i| match &d.abilities[i as usize] {
                Ability::Static { effect: StaticEffect::ManaPersists { who, colors }, condition, .. } => {
                    (colors.is_empty() || colors.contains(&t))
                        && self.player_filter(*who, p, c.cur.controller)
                        && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(id, c.cur.controller)))
                }
                _ => false,
            })
        })
    }

    /// Does a static ability on a permanent (or an emblem in the command zone, `from: Command`) give player `p` this effect (`pick` returns the effect's player filter)?
    fn player_static_scan(&self, p: PlayerId, pick: impl Fn(&StaticEffect) -> Option<PlayerFilter>) -> bool {
        self.player_static_count(p, pick) > 0
    }

    /// How many sources (permanents / command-zone cards) have a matching static for `p`.
    fn player_static_count(&self, p: PlayerId, pick: impl Fn(&StaticEffect) -> Option<PlayerFilter>) -> usize {
        let check = |id: CardId, ctl: PlayerId, in_command: bool| {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                return false;
            }
            let d = self.db.def(c.def);
            let ok = |ab: &Ability| match ab {
                Ability::Static { effect, condition, from } => {
                    let zone_ok = if in_command { *from == Some(Zone::Command) } else { from.map_or(true, |z| z == Zone::Battlefield) };
                    zone_ok
                        && pick(effect).map_or(false, |who| self.player_filter(who, p, ctl))
                        && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(id, ctl)))
                }
                _ => false,
            };
            d.statics.iter().any(|&i| ok(&d.abilities[i as usize])) || (!in_command && c.cur.granted.iter().any(|g| ok(&**g)))
        };
        self.battlefield.iter().filter(|&&id| check(id, self.cards[id.0 as usize].cur.controller, false)).count()
            + self.players.iter().enumerate().map(|(q, pl)| pl.command.iter().filter(|&&id| check(id, q as PlayerId, true)).count()).sum::<usize>()
    }

    /// Does a static make mana `p` would lose become colorless instead (Horizon Stone)?
    pub(crate) fn mana_becomes_colorless(&self, p: PlayerId) -> bool {
        self.player_static_scan(p, |e| if let StaticEffect::ManaBecomesColorless { who } = e { Some(*who) } else { None })
    }

    /// Does a static make mana `p` would lose become black instead (Omnath, Locus of All)?
    pub(crate) fn mana_becomes_black(&self, p: PlayerId) -> bool {
        self.player_static_scan(p, |e| if let StaticEffect::ManaBecomesBlack { who } = e { Some(*who) } else { None })
    }

    /// Does a static make mana `p` would lose become red instead (Ozai, the Phoenix King)?
    pub(crate) fn mana_becomes_red(&self, p: PlayerId) -> bool {
        self.player_static_scan(p, |e| if let StaticEffect::ManaBecomesRed { who } = e { Some(*who) } else { None })
    }

    /// Does a static limit `p` to one poison counter per turn (Melira, the Living Cure)?
    pub(crate) fn poison_only_one(&self, p: PlayerId) -> bool {
        self.player_static_scan(p, |e| if let StaticEffect::PoisonOnlyOne { who } = e { Some(*who) } else { None })
    }

    /// Does a static make `p` copy a spell one more time whenever they copy it (Twinning Staff)?
    pub(crate) fn copy_spell_extra(&self, p: PlayerId) -> usize {
        self.player_static_count(p, |e| if let StaticEffect::CopySpellExtra { who } = e { Some(*who) } else { None })
    }

    /// Does a static make `p` lose life for each unspent mana they lose (Yurlok of Scorch Thrash)?
    pub(crate) fn mana_loss_costs_life(&self, p: PlayerId) -> bool {
        self.player_static_scan(p, |e| if let StaticEffect::ManaLossCostsLife { who } = e { Some(*who) } else { None })
    }

    pub fn cant_win(&self, p: PlayerId) -> bool {
        self.safe_until_planeswalk.iter().any(|q| self.is_opponent(*q, p))
            || self.player_effect_static(p, |s| matches!(s, StaticEffect::CantWinGame { .. }))
            || self.player_static_scan(p, |e| if let StaticEffect::CantWinGame { who } = e { Some(*who) } else { None })
    }

    pub fn cant_lose(&self, p: PlayerId) -> bool {
        self.safe_until_planeswalk.contains(&p)
            || self.player_effect_static(p, |s| matches!(s, StaticEffect::CantLoseGame { .. }))
            || self.player_static_scan(p, |e| if let StaticEffect::CantLoseGame { who } = e { Some(*who) } else { None })
    }

    /// Can `p` pay `n` life right now (enough life and no "can't pay life" static)?
    /// Pay `n` life as a cost: replaced by exiling that many cards from the top of the library when a
    /// `PayLifeExilesLibrary` static applies and the library is big enough.
    pub fn pay_life(&mut self, p: PlayerId, n: i32) {
        if n > 0 && self.players[p as usize].library.len() >= n as usize {
            let repl = self.battlefield.iter().any(|&id| {
                let c = &self.cards[id.0 as usize];
                if c.cur.no_abilities {
                    return false;
                }
                let d = self.db.def(c.def);
                d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::PayLifeExilesLibrary { who }, .. } if self.player_filter(*who, p, c.cur.controller)))
            });
            if repl {
                for _ in 0..n {
                    if let Some(&id) = self.players[p as usize].library.last() {
                        self.move_card(id, Zone::Exile, Default::default());
                    }
                }
                return;
            }
        }
        self.lose_life(p, n);
        self.emit(Event::LifePaid { player: p, n });
    }

    pub fn can_pay_life(&self, p: PlayerId, n: i32) -> bool {
        if n <= 0 {
            return true;
        }
        if self.players[p as usize].life < n {
            return false;
        }
        // a player whose life can't change (Flare of Fortitude) can't pay life
        if self.effects.iter().any(|e| matches!(e.modif, Modif::NoLifeLoss) && e.players.contains(&p)) {
            return false;
        }
        !self.battlefield.iter().any(|&id| {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                return false;
            }
            let d = self.db.def(c.def);
            d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::CantPayLife { who }, .. } if self.player_filter(*who, p, c.cur.controller)))
        })
    }

    /// New damage to this creature heals the damage already marked on it (Wolverine, Fierce Fighter).
    pub fn heals_on_damage(&self, id: CardId) -> bool {
        let c = &self.cards[id.0 as usize];
        if c.cur.no_abilities {
            return false;
        }
        let d = self.db.def(c.def);
        d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::HealOnDamaged, .. }))
    }

    /// Permanents can't phase in (Disciple of Caelus Nin).
    pub fn phase_in_barred(&self) -> bool {
        self.battlefield.iter().any(|&id| {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                return false;
            }
            let d = self.db.def(c.def);
            d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::CantPhaseIn, .. }))
        })
    }

    /// A card in `from` can't be put onto the battlefield (Kunoros, Hound of Athreos).
    pub fn enter_battlefield_barred(&self, id: CardId, from: Zone) -> bool {
        if from == Zone::Exile && self.no_enter_from_exile_turn == self.turn + 1 {
            return true;
        }
        self.battlefield.iter().any(|&src| {
            let c = &self.cards[src.0 as usize];
            if c.cur.no_abilities {
                return false;
            }
            let d = self.db.def(c.def);
            d.statics.iter().any(|&i| match &d.abilities[i as usize] {
                Ability::Static { effect: StaticEffect::CantEnterBattlefieldFrom { from: z, filter }, .. } => *z == from && self.matches_card(filter, id, src, c.cur.controller),
                Ability::Static { effect: StaticEffect::CantEnterBattlefield { filter }, .. } => self.matches_card(filter, id, src, c.cur.controller),
                _ => false,
            })
        })
    }

    /// The Master, Multiplied: the effect being resolved is a triggered ability of the token's controller, who controls a permanent with `TriggersCantSacExileOwnTokens`.
    pub fn trigger_token_barred(&self, id: CardId) -> bool {
        if !self.cause_trigger.get() {
            return false;
        }
        let c = &self.cards[id.0 as usize];
        if !c.token || !c.cur.types.has(CardType::Creature) || self.cause_ctl.get() != Some(c.cur.controller) {
            return false;
        }
        let ctl = c.cur.controller;
        self.battlefield.iter().any(|&b| {
            let bc = &self.cards[b.0 as usize];
            if bc.cur.no_abilities || bc.cur.controller != ctl {
                return false;
            }
            let d = self.db.def(bc.def);
            d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::TriggersCantSacExileOwnTokens, .. }))
        })
    }

    /// Creatures can't be sacrificed to pay costs (Angel of Jubilation).
    pub fn creature_sac_costs_barred(&self) -> bool {
        self.battlefield.iter().any(|&id| {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                return false;
            }
            let d = self.db.def(c.def);
            d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::CantSacrificeCreaturesForCosts, .. }))
        })
    }

    /// Nonland permanents can't be sacrificed to pay costs (Yasharn, Implacable Earth).
    pub fn nonland_sac_costs_barred(&self) -> bool {
        self.battlefield.iter().any(|&id| {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                return false;
            }
            let d = self.db.def(c.def);
            d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::CantSacrificeNonlandForCosts, .. }))
        })
    }

    /// The filter a sacrifice cost may choose from: creatures drop out while they're barred.
    /// Like `sac_cost_filter`, for "can this cost be paid at all" checks made before the targets are chosen: a mana value tied to a target is not judged yet.
    pub fn sac_cost_filter_check(&self, f: &Filter) -> Filter {
        crate::pay::strip_x_filter(&self.sac_cost_filter(f))
    }

    pub fn sac_cost_filter(&self, f: &Filter) -> Filter {
        let mut f = f.clone();
        f.sacrificable = true;
        if self.creature_sac_costs_barred() && !f.not_types.contains(&CardType::Creature) {
            f.not_types.push(CardType::Creature);
        }
        f
    }

    /// How many times each planeswalker the player controls may activate a loyalty ability per turn.
    /// May `p` activate the loyalty abilities of `card` at instant speed (Teferi's Talent emblem, Jace's Machinations)?
    pub fn loyalty_instant(&self, p: PlayerId, card: CardId) -> bool {
        let pl = &self.players[p as usize];
        if pl.loyalty_instant_turn == self.turn && pl.loyalty_instant_sub.map_or(true, |s| self.cards[card.0 as usize].cur.subtypes.contains(&s)) {
            return true;
        }
        // a planeswalker with the static itself (Teferi, Master of Time: "on any player's turn any time you could cast an instant")
        let own = {
            let c = &self.cards[card.0 as usize];
            c.zone == Zone::Battlefield && !c.cur.no_abilities && {
                let d = self.db.def(c.def);
                d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::LoyaltyAtInstantSpeed, .. }))
            }
        };
        own || pl.command.iter().any(|&id| {
            let d = self.db.def(self.cards[id.0 as usize].def);
            d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::LoyaltyAtInstantSpeed, .. }))
        })
    }

    pub fn loyalty_limit(&self, p: PlayerId) -> usize {
        let twice = self.battlefield.iter().any(|&id| {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities || c.cur.controller != p {
                return false;
            }
            let d = self.db.def(c.def);
            d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::LoyaltyTwice, .. }))
        });
        if twice { 2 } else { 1 }
    }

    /// Wonder Man: does `p` control a permanent letting power-up abilities be activated an additional time?
    pub fn power_up_twice(&self, p: PlayerId) -> bool {
        self.battlefield.iter().any(|&id| {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities || c.cur.controller != p {
                return false;
            }
            let d = self.db.def(c.def);
            d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::PowerUpTwice, .. }))
        })
    }

    /// Aven Mindcensor: `p` only searches the top N cards of a library (the smallest N among the statics that apply).
    pub fn search_top_limit(&self, p: PlayerId) -> Option<usize> {
        let mut lim: Option<usize> = None;
        for &id in &self.battlefield {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                continue;
            }
            let d = self.db.def(c.def);
            let ctrl = c.cur.controller;
            for &i in &d.statics {
                if let Ability::Static { effect: StaticEffect::SearchTopOnly { who, n }, condition, .. } = &d.abilities[i as usize] {
                    if self.player_filter_src(id, *who, p, ctrl) && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &crate::eval::EvalCtx::with_controller(id, ctrl))) {
                        lim = Some(lim.map_or(*n as usize, |l| l.min(*n as usize)));
                    }
                }
            }
        }
        lim
    }

    /// Elvish Refueler: during p's turn, before p has activated any exhaust ability this turn, exhaust abilities may be activated again.
    pub fn exhaust_reactivate(&self, p: PlayerId) -> bool {
        if self.active != p || self.players[p as usize].exhaust_turn {
            return false;
        }
        self.battlefield.iter().any(|&id| {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities || c.cur.controller != p {
                return false;
            }
            let d = self.db.def(c.def);
            d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::ExhaustReactivate, .. }))
        })
    }

    /// Extra loyalty (net) added to a loyalty ability's cost by `LoyaltyCostChange` statics.
    pub fn loyalty_cost_delta(&self, p: PlayerId, card: CardId) -> i32 {
        let mut total = 0;
        for &id in &self.battlefield {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                continue;
            }
            let d = self.db.def(c.def);
            for &i in &d.statics {
                if let Ability::Static { effect: StaticEffect::LoyaltyCostChange { filter, whose, delta }, condition, .. } = &d.abilities[i as usize] {
                    let ctl = c.cur.controller;
                    if self.player_filter_src(id, *whose, p, ctl)
                        && self.matches_card(filter, card, id, ctl)
                        && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &crate::eval::EvalCtx::with_controller(id, ctl)))
                    {
                        total += *delta;
                    }
                }
            }
        }
        total
    }

    /// A static stops this player from playing lands from their hand.
    pub fn cant_play_lands_from_hand(&self, p: PlayerId) -> bool {
        self.battlefield.iter().any(|&id| {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                return false;
            }
            let d = self.db.def(c.def);
            d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::CantPlayLandsFromHand { who }, condition, .. }
                if self.player_filter_src(id, *who, p, c.cur.controller) && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &crate::eval::EvalCtx::with_controller(id, c.cur.controller)))))
        })
    }

    /// A static makes `p` skip an extra turn it would begin.
    pub fn skips_extra_turn(&self, p: PlayerId) -> bool {
        self.battlefield.iter().any(|&id| {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                return false;
            }
            let d = self.db.def(c.def);
            d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::SkipExtraTurns { who }, .. } if self.player_filter_src(id, *who, p, c.cur.controller)))
        })
    }

    /// A static lets `p` activate Equip abilities at instant speed (Leonin Shikari).
    pub fn equip_at_instant_speed(&self, p: PlayerId) -> bool {
        self.battlefield.iter().any(|&id| {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                return false;
            }
            let d = self.db.def(c.def);
            d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::EquipAtInstantSpeed { who }, condition, .. } if self.player_filter_src(id, *who, p, c.cur.controller) && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(id, c.cur.controller)))))
        })
    }

    /// Forge Anew: p controls a permanent with `EquipFirstFree` and has not yet activated an equip ability this turn (their own turn).
    pub fn equip_first_free(&self, p: PlayerId) -> bool {
        if self.players[p as usize].equip_turn {
            return false;
        }
        self.battlefield.iter().any(|&id| {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities || c.cur.controller != p {
                return false;
            }
            let d = self.db.def(c.def);
            d.statics.iter().any(|&i| match &d.abilities[i as usize] {
                Ability::Static { effect: StaticEffect::EquipFirstFree, condition, .. } => self.active == p && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(id, c.cur.controller))),
                Ability::Static { effect: StaticEffect::EquipFirstFreeEachTurn, condition, .. } => condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(id, c.cur.controller))),
                _ => false,
            })
        })
    }

    /// "Can block <attacker> as though it had reach".
    pub fn blocks_as_reach_vs(&self, blocker: CardId, attacker: CardId) -> bool {
        self.battlefield.iter().any(|&id| {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                return false;
            }
            let d = self.db.def(c.def);
            d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::BlockAsReachVs { affects, attacker: af }, .. } if self.matches_card(affects, blocker, id, c.cur.controller) && self.matches_card(af, attacker, id, c.cur.controller)))
        })
    }

    /// A static makes all damage be dealt as though its source had wither.
    pub fn all_damage_wither(&self) -> bool {
        self.battlefield.iter().any(|&id| {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                return false;
            }
            let d = self.db.def(c.def);
            d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::AllDamageWither, .. }))
        })
    }

    /// A static lets this tapped creature block (Masako the Humorless).
    pub fn can_block_while_tapped(&self, blocker: CardId) -> bool {
        let b = &self.cards[blocker.0 as usize];
        self.battlefield.iter().any(|&id| {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                return false;
            }
            let d = self.db.def(c.def);
            d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::CanBlockWhileTapped { affects }, .. } if self.matches_card(affects, blocker, id, c.cur.controller) && b.cur.controller == c.cur.controller))
        })
    }

    /// The crew number of `vehicle` after "Vehicles you control have crew N" statics.
    pub fn crew_requirement(&self, vehicle: CardId, printed: i32) -> i32 {
        let mut n = printed;
        for &id in &self.battlefield {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                continue;
            }
            let d = self.db.def(c.def);
            for &i in &d.statics {
                if let Ability::Static { effect: StaticEffect::CrewCostOverride { affects, n: k }, .. } = &d.abilities[i as usize] {
                    if self.matches_card(affects, vehicle, id, c.cur.controller) {
                        n = *k;
                    }
                }
            }
        }
        n
    }

    /// Filters of "can't attack unless another creature matching X also attacks" statics that apply to this creature.
    pub fn attack_needs_other(&self, attacker: CardId) -> Vec<FilterBox> {
        let mut out = vec![];
        for &id in &self.battlefield {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                continue;
            }
            let d = self.db.def(c.def);
            for &i in &d.statics {
                if let Ability::Static { effect: StaticEffect::CantAttackUnlessOther { affects, other }, .. } = &d.abilities[i as usize] {
                    if self.matches_card(affects, attacker, id, c.cur.controller) {
                        out.push(other.clone());
                    }
                }
            }
        }
        out
    }

    /// The permanent has a static "can't be equipped" (Goblin Brawler).
    pub fn cant_be_equipped(&self, host: CardId) -> bool {
        let c = &self.cards[host.0 as usize];
        if c.cur.no_abilities {
            return false;
        }
        let d = self.db.def(c.def);
        d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::CantBeEquipped, .. }))
    }

    /// A static of a battlefield permanent (e.g. Guardian Beast) stops new Auras from enchanting `host`.
    pub fn host_guarded_from_auras(&self, host: CardId) -> bool {
        self.battlefield.iter().any(|&b| {
            let c = &self.cards[b.0 as usize];
            if c.cur.no_abilities {
                return false;
            }
            let d = self.db.def(c.def);
            d.statics.iter().any(|&i| {
                matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::NoNewAuras { affects }, condition, .. }
                    if self.matches_card(affects, host, b, c.cur.controller)
                        && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &crate::eval::EvalCtx::with_controller(b, c.cur.controller))))
            })
        })
    }

    /// A static (Guardian Beast) stops `new_ctl` gaining control of `target`.
    pub fn control_change_barred(&self, target: CardId, new_ctl: PlayerId) -> bool {
        self.battlefield.iter().any(|&b| {
            let c = &self.cards[b.0 as usize];
            if c.cur.no_abilities || c.cur.controller == new_ctl {
                return false;
            }
            let d = self.db.def(c.def);
            d.statics.iter().any(|&i| {
                matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::NoControlChangeByOthers { affects }, condition, .. }
                    if self.matches_card(affects, target, b, c.cur.controller)
                        && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &crate::eval::EvalCtx::with_controller(b, c.cur.controller))))
            })
        })
    }

    fn has_bar_other_auras(&self, id: CardId) -> bool {
        let c = &self.cards[id.0 as usize];
        !c.cur.no_abilities && {
            let d = self.db.def(c.def);
            d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::CantBeEnchantedByOthers, .. }))
        }
    }

    /// An Aura attached to `host` says "can't be enchanted by other Auras" and `aura` is a different Aura without that text (Consecrate Land).
    pub fn host_bars_other_auras(&self, host: CardId, aura: CardId) -> bool {
        // the host itself can't be enchanted (the Tetravite token)
        let hc = &self.cards[host.0 as usize];
        if !hc.cur.no_abilities && {
            let d = self.db.def(hc.def);
            d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::CantBeEnchanted, .. }))
        } {
            return true;
        }
        !self.has_bar_other_auras(aura)
            && self.battlefield.iter().any(|&b| b != aura && self.cards[b.0 as usize].attached_to == Some(host) && self.has_bar_other_auras(b))
    }

    /// A static "skip upkeep steps" applies to this player (Eon Hub).
    pub fn skips_upkeep_step(&self, p: PlayerId) -> bool {
        self.battlefield.iter().any(|&id| {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                return false;
            }
            let d = self.db.def(c.def);
            d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::SkipUpkeep { who }, .. } if self.player_filter_src(id, *who, p, c.cur.controller)))
        })
    }

    /// A static "skip your draw step" applies to this player.
    pub fn skips_draw_step(&self, p: PlayerId) -> bool {
        self.battlefield.iter().any(|&id| {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                return false;
            }
            let d = self.db.def(c.def);
            d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::SkipDrawStep { who }, condition, .. }
                if self.player_filter_src(id, *who, p, c.cur.controller) && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(id, c.cur.controller)))))
        })
    }

    /// A static "gain life rather than lose life from radiation" applies to this player.
    pub fn radiation_gains_life(&self, p: PlayerId) -> bool {
        self.battlefield.iter().any(|&id| {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                return false;
            }
            let d = self.db.def(c.def);
            d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::RadiationGainsLife { who }, .. } if self.player_filter_src(id, *who, p, c.cur.controller)))
        })
    }

    /// Extra cards a player looks at when they surveil (Enhanced Surveillance).
    pub fn surveil_extra_look(&self, p: PlayerId) -> i32 {
        let mut total = 0;
        for &id in self.battlefield.iter() {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                continue;
            }
            let d = self.db.def(c.def);
            for &i in d.statics.iter() {
                if let Ability::Static { effect: StaticEffect::SurveilExtraLook { who, n }, .. } = &d.abilities[i as usize] {
                    if self.player_filter_src(id, *who, p, c.cur.controller) {
                        total += *n;
                    }
                }
            }
        }
        total
    }

    /// A static "you may skip your draw step" applies to this player: (source, life to gain).
    pub fn may_skip_draw_step(&self, p: PlayerId) -> Option<(CardId, i32)> {
        self.battlefield.iter().find_map(|&id| {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                return None;
            }
            let d = self.db.def(c.def);
            d.statics.iter().find_map(|&i| match &d.abilities[i as usize] {
                Ability::Static { effect: StaticEffect::MaySkipDrawStep { who, gain }, condition, .. }
                    if self.player_filter_src(id, *who, p, c.cur.controller) && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(id, c.cur.controller))) =>
                {
                    Some((id, *gain))
                }
                _ => None,
            })
        })
    }

    /// Damage can't be prevented right now ("this turn" effect or a static like Leyline of Punishment).
    pub fn damage_unpreventable(&self) -> bool {
        if self.force_unprevent.get() {
            return true;
        }
        if self.no_prevent_turn != 0 && self.no_prevent_turn == self.turn {
            return true;
        }
        self.battlefield.iter().any(|&id| {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                return false;
            }
            let d = self.db.def(c.def);
            d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::DamageCantBePrevented, condition, .. }
                if condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(id, c.cur.controller)))))
        })
    }

    /// Does the permanent say "you may choose not to untap this" (its controller is asked during the untap step)?
    pub fn untap_optional(&self, card: CardId) -> bool {
        let c = &self.cards[card.0 as usize];
        if c.cur.no_abilities {
            return false;
        }
        let d = self.db.def(c.def);
        d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::OptionalUntap { affects }, .. } if self.matches_card(affects, card, card, c.cur.controller)))
    }

    /// A static "mana produced by this permanent is of type X instead" (Contamination, Ritual of Subdual) changes `t`.
    pub fn replace_mana_type(&self, card: CardId, t: crate::mana::ManaType) -> crate::mana::ManaType {
        let mut t = t;
        let swap = |t: crate::mana::ManaType, to: crate::mana::ManaType, from: Option<crate::mana::ManaType>| if from.map_or(true, |f| f == t) { to } else { t };
        for e in &self.effects {
            if let Modif::Static(se) = &e.modif {
                if let StaticEffect::ManaTypeReplace { to, from, any_color: false, .. } = &**se {
                    if e.affects.iter().any(|(a, z)| *a == card && *z == self.cards[card.0 as usize].zcc) {
                        t = swap(t, *to, *from);
                    }
                }
            }
        }
        // "until end of turn, nonbasic lands produce {C}" (GrantStatic dynamic): matches by filter, also lands that enter later
        for e in &self.effects {
            if let Modif::DynStatic(se) = &e.modif {
                if let StaticEffect::ManaTypeReplace { filter, to, from, any_color: false, to_chosen, .. } = &**se {
                    if self.matches_card(filter, card, e.source, e.controller) {
                        let to = if *to_chosen { self.cards[e.source.0 as usize].chosen_color.map_or(*to, crate::mana::ManaType::of) } else { *to };
                        t = swap(t, to, *from);
                    }
                }
            }
        }
        // several permanent statics can apply to one land (Naked Singularity on a Forest Island): the controller picks the order, so the colour asked for wins when one of them yields it
        let before = t;
        let mut outcomes: Vec<crate::mana::ManaType> = vec![];
        for &id in &self.battlefield {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                continue;
            }
            let d = self.db.def(c.def);
            for ab in d.statics.iter().map(|&i| &d.abilities[i as usize]) {
                if let Ability::Static { effect: StaticEffect::ManaTypeReplace { filter, to, from, any_color: false, .. }, condition, .. } = ab {
                    if self.matches_card(filter, card, id, c.cur.controller) && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(id, c.cur.controller))) {
                        t = swap(t, *to, *from);
                        let o = swap(before, *to, *from);
                        if !outcomes.contains(&o) {
                            outcomes.push(o);
                        }
                    }
                }
            }
        }
        if outcomes.len() > 1 {
            if let Some(h) = self.mana_produce_hint.get().or_else(|| self.mana_color_hints.borrow().first().copied()) {
                if outcomes.contains(&h) {
                    t = h;
                }
            }
        }
        t
    }

    /// Is some permanent on the battlefield making lands that produce two or more mana produce {C} instead (Damping Sphere)?
    pub fn land_mana_capped(&self) -> bool {
        self.battlefield.iter().any(|&id| {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                return false;
            }
            let d = self.db.def(c.def);
            d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::LandManaCappedToColorless, .. }))
        })
    }

    /// A static "a land tapped for mana produces just one {X} instead of any other type and amount" (Contamination).
    pub fn mana_collapsed(&self, card: CardId) -> bool {
        // "until end of turn ..." (GrantStatic dynamic) single / any-color replacements (Harvest Mage)
        if self.effects.iter().any(|e| matches!(&e.modif, Modif::DynStatic(se) if matches!(&**se, StaticEffect::ManaTypeReplace { filter, single: true, .. } | StaticEffect::ManaTypeReplace { filter, any_color: true, .. } if self.matches_card(filter, card, e.source, e.controller)))) {
            return true;
        }
        self.battlefield.iter().any(|&id| {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                return false;
            }
            let d = self.db.def(c.def);
            d.statics.iter().map(|&i| &d.abilities[i as usize]).any(|ab| {
                matches!(ab, Ability::Static { effect: StaticEffect::ManaTypeReplace { filter, single: true, .. } | StaticEffect::ManaTypeReplace { filter, any_color: true, .. }, condition, .. }
                    if self.matches_card(filter, card, id, c.cur.controller) && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(id, c.cur.controller))))
            })
        })
    }

    /// A permanent the player controls says their coin flips are made twice, ignoring one (Krark's Thumb).
    pub fn flips_coin_twice(&self, p: PlayerId) -> bool {
        self.battlefield.iter().any(|&id| {
            let c = &self.cards[id.0 as usize];
            if c.cur.controller != p || c.cur.no_abilities {
                return false;
            }
            let d = self.db.def(c.def);
            d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::CoinFlipTwice, condition, .. } if condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(id, p)))))
        })
    }

    /// A permanent the player controls rigs their first coin flip of the turn to come up heads (Two-Headed Coin).
    pub fn first_flip_heads(&self, p: PlayerId) -> bool {
        (self.players[p as usize].coin_flip_turn != self.turn || self.players[p as usize].coin_flip_serial == self.flip_serial)
            && self.battlefield.iter().any(|&id| {
                let c = &self.cards[id.0 as usize];
                if c.cur.controller != p || c.cur.no_abilities {
                    return false;
                }
                let d = self.db.def(c.def);
                d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::FirstCoinFlipHeads, condition, .. } if condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(id, p)))))
            })
    }

    /// May this permanent become tapped now (cost, effect, convoke/crew)? False under a granted `CantBecomeTapped`, unless `attacking` is true (being declared as an attacker) and the effect allows that.
    pub fn can_become_tapped(&self, card: CardId, attacking: bool) -> bool {
        let zcc = self.cards[card.0 as usize].zcc;
        !self.effects.iter().any(|e| {
            matches!(&e.modif, Modif::Static(se) if matches!(&**se, StaticEffect::CantBecomeTapped { except_attacking, .. } if !(*except_attacking && attacking)))
                && e.affects.iter().any(|(a, z)| *a == card && *z == zcc)
        })
    }

    /// Does a static ability stop this permanent from untapping during its controller's untap step?
    pub fn doesnt_untap(&self, card: CardId) -> bool {
        if self.edit_no_untap.contains(&card) {
            return true;
        }
        if self.cards[card.0 as usize].cur.types.has(CardType::Land) && self.players[self.cards[card.0 as usize].cur.controller as usize].lands_no_untap {
            return true;
        }
        if self.cards[card.0 as usize].cur.types.has(CardType::Creature) && self.players[self.cards[card.0 as usize].cur.controller as usize].creatures_no_untap {
            return true;
        }
        // "it doesn't untap" granted for a while (GrantStatic)
        if self.effects.iter().any(|e| matches!(&e.modif, Modif::Static(se) if matches!(**se, StaticEffect::DoesntUntap { .. })) && e.affects.iter().any(|(a, z)| *a == card && *z == self.cards[card.0 as usize].zcc)) {
            return true;
        }
        // a scheme in the command zone (Imprison This Insolent Wretch) counts as a source, controlled by its owner
        self.battlefield.iter().copied().chain(self.emblem_srcs()).any(|id| {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                return false;
            }
            let sctl = self.src_ctrl(id);
            let d = self.db.def(c.def);
            d.statics.iter().map(|&i| &d.abilities[i as usize]).chain(c.cur.granted.iter().map(|g| &**g)).any(|ab| {
                matches!(ab, Ability::Static { effect: StaticEffect::DoesntUntap { filter }, condition, .. }
                    if self.matches_card(filter, card, id, sctl)
                        && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(id, sctl))))
            })
        })
    }

    /// A static "players can't gain life" (Tibalt, Rakish Instigator) applies to this player.
    pub fn static_cant_gain_life(&self, p: PlayerId) -> bool {
        self.battlefield.iter().any(|&id| {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                return false;
            }
            let d = self.db.def(c.def);
            d.statics.iter().any(|&i| {
                matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::CantGainLife { who }, condition, .. }
                    if self.player_filter_src(id, *who, p, c.cur.controller)
                        && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(id, c.cur.controller))))
            })
        })
    }

    /// Is this spell protected from being countered by a static ability?
    /// A spell on the stack that can't be countered: its own keyword (not when cast face down) or a static/granted effect.
    pub fn spell_uncounterable(&self, spell: CardId, by_src: Option<CardId>) -> bool {
        let c = &self.cards[spell.0 as usize];
        let face_down = c.cast_alt == Some(crate::decision::FACE_DOWN_ALT);
        (!face_down && (c.cur.kw.has(Keyword::CantBeCountered) || self.def_of(spell).keywords.has(Keyword::CantBeCountered))) || self.spell_cant_be_countered(spell, by_src)
    }

    pub fn spell_cant_be_countered(&self, spell: CardId, by_src: Option<CardId>) -> bool {
        // "creature spells you cast this turn can't be countered" granted to a player by a resolved effect
        for e in &self.effects {
            if let Modif::Static(b) = &e.modif {
                if let StaticEffect::CantBeCountered { filter, once: false, by } = &**b {
                    if let Some(bf) = by {
                        match by_src {
                            Some(bs) if self.matches_card(&Filter { controller: PlayerFilter::Any, ..bf.clone().into_filter() }, bs, e.source, e.controller) => {}
                            _ => continue,
                        }
                    }
                    let owner = self.cards[spell.0 as usize].controller;
                    if e.players.contains(&owner) && self.matches_card(&Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() }, spell, e.source, e.controller) {
                        return true;
                    }
                }
            }
        }
        self.battlefield.iter().chain(std::iter::once(&spell)).any(|&id| {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities && c.zone == Zone::Battlefield {
                return false;
            }
            let d = self.db.def(c.def);
            // the spell's own "this spell can't be countered" works from the stack
            let ctrl = if c.zone == Zone::Battlefield { c.cur.controller } else { c.owner };
            d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::CantBeCountered { filter, by: None, .. }, condition, .. } if (id != spell || (c.zone == Zone::Stack && filter.this) || c.zone == Zone::Battlefield) && !(id == spell && c.zone == Zone::Stack && c.cast_alt == Some(crate::decision::FACE_DOWN_ALT)) && self.matches_card(filter, spell, id, ctrl) && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &{ let mut ec = EvalCtx::with_controller(id, ctrl); if id == spell { if let Some(si) = self.stack.iter().find(|s| s.source == spell) { ec.x = si.x; } } ec }))))
        })
    }

    /// Costs a creature must pay to attack / block (Brainwash, Crawlspace-style statics), with computed amounts settled.
    pub fn combat_costs(&self, creature: CardId, attack: bool, defender: Option<PlayerId>, blocked: Option<CardId>) -> Vec<Cost> {
        let mut out = vec![];
        for &id in &self.battlefield {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                continue;
            }
            let d = self.db.def(c.def);
            for &i in &d.statics {
                if let Ability::Static { effect, condition, .. } = &d.abilities[i as usize] {
                    let (affects, cost) = match (effect, attack) {
                        (StaticEffect::AttackCost { affects, cost, against }, true) => {
                            if let Some(d) = defender {
                                if !self.player_filter_src(id, *against, d, c.cur.controller) {
                                    continue;
                                }
                            }
                            (affects, cost)
                        }
                        (StaticEffect::BlockCost { affects, cost, attacker }, false) => {
                            if let (Some(f), Some(a)) = (attacker, blocked) {
                                if !self.matches_card(f, a, id, c.cur.controller) {
                                    continue;
                                }
                            }
                            (affects, cost)
                        }
                        _ => continue,
                    };
                    let ec = EvalCtx::with_controller(id, c.cur.controller);
                    if condition.as_ref().map_or(false, |cd| !self.eval_cond(cd, &ec)) {
                        continue;
                    }
                    if !self.matches_card(affects, creature, id, c.cur.controller) {
                        continue;
                    }
                    // the creature itself is the event object, so a tax can scale with its own state (`AllCountersOn TriggerObject`: Nils, Discipline Enforcer)
                    let cev = EventCtx { obj: Some(Obj::card(creature)), ..Default::default() };
                    let ec = EvalCtx { event: &cev, ..ec };
                    for k in cost {
                        out.push(match k {
                            Cost::ManaAmount { amount } => Cost::Mana { cost: format!("{{{}}}", self.eval_amount(amount, &ec).max(0)) },
                            o => o.clone(),
                        });
                    }
                }
            }
        }
        // costs granted by resolved effects ("until end of turn, creatures can't attack you unless ..."): static effects with a duration
        for e in &self.effects {
            let eff: &StaticEffect = match &e.modif {
                Modif::Static(b) => &**b,
                Modif::DynStatic(b) => &**b,
                _ => continue,
            };
            let (affects, cost) = match (eff, attack) {
                (StaticEffect::AttackCost { affects, cost, against }, true) => {
                    if let Some(d) = defender {
                        if !self.player_filter_src(e.source, *against, d, e.controller) {
                            continue;
                        }
                    }
                    (affects, cost)
                }
                (StaticEffect::BlockCost { affects, cost, attacker }, false) => {
                    if let (Some(f), Some(a)) = (attacker, blocked) {
                        if !self.matches_card(f, a, e.source, e.controller) {
                            continue;
                        }
                    }
                    (affects, cost)
                }
                _ => continue,
            };
            let named = !e.affects.is_empty();
            if named && !e.affects.iter().any(|(id, _)| *id == creature) {
                continue;
            }
            if !named && !self.matches_card(affects, creature, e.source, e.controller) {
                continue;
            }
            let ec = EvalCtx::with_controller(e.source, e.controller);
            for k in cost {
                out.push(match k {
                    Cost::ManaAmount { amount } => Cost::Mana { cost: format!("{{{}}}", self.eval_amount(amount, &ec).max(0)) },
                    o => o.clone(),
                });
            }
        }
        out
    }

    /// Generic mana each creature attacking `defender` (or their planeswalkers) costs its controller (Propaganda).
    pub fn attack_tax(&self, defender: PlayerId, at_player: bool) -> i32 {
        let mut n = self.players[defender as usize].shield_tax;
        for &id in &self.battlefield {
            let c = &self.cards[id.0 as usize];
            if c.cur.controller != defender || c.cur.no_abilities {
                continue;
            }
            let d = self.db.def(c.def);
            for &i in &d.statics {
                if let Ability::Static { effect: StaticEffect::AttackTax { generic, only_player, only_planeswalkers, amount }, condition, .. } = &d.abilities[i as usize] {
                    if *only_player && !at_player {
                        continue;
                    }
                    if *only_planeswalkers && at_player {
                        continue;
                    }
                    let ec = EvalCtx::with_controller(id, defender);
                    if condition.as_ref().map_or(false, |cd| !self.eval_cond(cd, &ec)) {
                        continue;
                    }
                    n += match amount {
                        Some(a) => self.eval_amount(a, &ec).max(0),
                        None => *generic,
                    };
                }
            }
        }
        // a tax granted to the defending player by a resolved effect (`GrantStatic` with `dynamic`: Summon: Yojimbo's chapters II and III)
        for e in &self.effects {
            if e.controller != defender {
                continue;
            }
            if let Modif::DynStatic(se) = &e.modif {
                if let StaticEffect::AttackTax { generic, only_player, only_planeswalkers, amount } = &**se {
                    if (*only_player && !at_player) || (*only_planeswalkers && at_player) {
                        continue;
                    }
                    n += match amount {
                        Some(a) => self.eval_amount(a, &EvalCtx::with_controller(e.source, defender)).max(0),
                        None => *generic,
                    };
                }
            }
        }
        n
    }

    /// Replacement abilities currently in force (permanents on the battlefield that still have their abilities).
    pub fn active_replacements(&self) -> Vec<ActiveReplace> {
        let mut out = vec![];
        // emblems (tokens in the command zone) have replacement effects too
        let emblems: Vec<CardId> = self.players.iter().flat_map(|p| p.command.iter().copied()).filter(|id| { let d = self.db.def(self.cards[id.0 as usize].def); d.token || d.types.has(CardType::Scheme) }).collect();
        // face-up planes too (Kessig's prevention)
        for &id in self.battlefield.iter().chain(emblems.iter()).chain(self.planes.iter()) {
            let c = &self.cards[id.0 as usize];
            let d = self.db.def(c.def);
            if c.cur.no_abilities || self.entering_batch.contains(&id) {
                continue;
            }
            let ctrl = if c.zone == Zone::Command { c.owner } else { c.cur.controller };
            // replacement abilities granted to the permanent by other effects
            for g in c.cur.granted.iter() {
                if let Ability::Replace { what, condition, from: None } = &**g {
                    if let Some(cond) = condition {
                        if !self.eval_cond(cond, &EvalCtx::with_controller(id, ctrl)) {
                            continue;
                        }
                    }
                    out.push(ActiveReplace { source: id, controller: ctrl, what: what.clone(), temp_idx: None });
                }
            }
            if !d.has_replace {
                continue;
            }
            for a in &d.abilities {
                if let Ability::Replace { what, condition, from: None | Some(Zone::Command) } = a {
                    if let Some(cond) = condition {
                        if !self.eval_cond(cond, &EvalCtx::with_controller(id, ctrl)) {
                            continue;
                        }
                    }
                    out.push(ActiveReplace { source: id, controller: ctrl, what: what.clone(), temp_idx: None });
                }
            }
        }
        // replacements that work from the graveyard (dredge) or from the hand (a land's own "as it enters" replacement)
        let mut stack_cards: Vec<CardId> = self.stack.iter().filter(|s| !s.copy && matches!(s.kind, ItemKind::Spell { .. })).map(|s| s.source).collect();
        stack_cards.extend(self.resolving_spell.get());
        for pl in &self.players {
            let on_stack: Vec<CardId> = stack_cards.iter().copied().filter(|id| self.cards[id.0 as usize].owner == pl.id).collect();
            for (zone, ids) in [(Zone::Graveyard, &pl.graveyard), (Zone::Hand, &pl.hand), (Zone::Stack, &on_stack), (Zone::Library, &pl.library)] {
                for &id in ids.iter() {
                    let d = self.db.def(self.cards[id.0 as usize].def);
                    if !d.has_replace {
                        continue;
                    }
                    // "If this would be put into a graveyard from anywhere ..." (Darksteel Colossus): works in every zone
                    for a in &d.abilities {
                        if let Ability::Replace { what: what @ Replace::ZoneChange { filter, from: None, to: Zone::Graveyard, .. }, condition: None, from: None } = a {
                            if filter.this && zone != Zone::Graveyard {
                                out.push(ActiveReplace { source: id, controller: pl.id, what: what.clone(), temp_idx: None });
                            }
                        }
                    }
                    for a in &d.abilities {
                        if let Ability::Replace { what, condition, from: Some(z) } = a {
                            if *z != zone {
                                continue;
                            }
                            if let Some(cond) = condition {
                                if !self.eval_cond(cond, &EvalCtx::with_controller(id, pl.id)) {
                                    continue;
                                }
                            }
                            out.push(ActiveReplace { source: id, controller: pl.id, what: what.clone(), temp_idx: None });
                        }
                    }
                }
            }
        }
        // the damaged player picked which of several equivalent redirecting auras (two Pariahs) applies first
        let first = self.redirect_pick.get();
        out.sort_by_key(|r| (first.map_or(true, |h| self.cards[r.source.0 as usize].attached_to != Some(h)), self.cards[r.source.0 as usize].ts));
        for (i, (src, ctrl, what)) in self.temp_replaces.iter().enumerate() {
            out.push(ActiveReplace { source: *src, controller: *ctrl, what: what.clone(), temp_idx: Some(i) });
        }
        for (src, ctrl, what) in self.long_replaces.iter() {
            out.push(ActiveReplace { source: *src, controller: *ctrl, what: what.clone(), temp_idx: None });
        }
        out
    }
    pub fn any_replacements(&self) -> bool {
        !self.temp_replaces.is_empty()
            || !self.long_replaces.is_empty()
            || self.planes.iter().any(|id| self.db.def(self.cards[id.0 as usize].def).has_replace)
            || self.battlefield.iter().any(|id| self.db.def(self.cards[id.0 as usize].def).has_replace)
            || self.battlefield.iter().any(|id| self.cards[id.0 as usize].cur.granted.iter().any(|g| matches!(&**g, Ability::Replace { .. })))
            || self.players.iter().any(|p| p.command.iter().chain(p.graveyard.iter()).chain(p.library.iter()).chain(p.hand.iter()).any(|id| self.db.def(self.cards[id.0 as usize].def).has_replace))
            || self.stack.iter().any(|s| self.db.def(self.cards[s.source.0 as usize].def).has_replace)
            || !self.batch_granted.is_empty()
            || self.batch_left.iter().any(|(c, _)| self.db.def(self.cards[c.0 as usize].def).has_replace)
            || self.lki_slot.as_ref().map_or(false, |l| self.db.def(self.cards[l.card.0 as usize].def).has_replace)
            || self.lki_slot.as_ref().map_or(false, |l| l.derived.granted.iter().any(|g| matches!(&**g, Ability::Replace { .. })))
            || self.resolving_spell.get().map_or(false, |c| self.db.def(self.cards[c.0 as usize].def).has_replace)
    }

    pub fn replace_gain_life(&self, p: PlayerId, n: i32) -> i32 {
        self.replace_gain_life_from(p, n, false)
    }

    /// `lifelink`: the gain comes from combat damage by a lifelink source, which is no spell or ability's effect (Rain of Gore's "lose that much instead" ignores it).
    pub fn replace_gain_life_from(&self, p: PlayerId, n: i32, lifelink: bool) -> i32 {
        if !self.any_replacements() {
            return n;
        }
        let mut n = n;
        for r in self.active_replacements() {
            if let Replace::GainLife { who, multiply, add, instead } = &r.what {
                if !instead.is_empty() || (lifelink && *multiply < 0) {
                    continue;
                }
                if self.player_filter_src(r.source, *who, p, r.controller) {
                    let before = n;
                    n = n * multiply + add;
                    // the event is no longer a life gain (Tainted Remedy, Plague Drone): later gain replacements do not apply to it
                    if n <= 0 && before > 0 {
                        break;
                    }
                }
            }
        }
        n
    }

    /// A life-gain replacement that does other things INSTEAD of gaining life: (the permanent, its controller, the effects).
    pub fn gain_life_instead(&self, p: PlayerId) -> Option<(CardId, PlayerId, Vec<Effect>)> {
        if !self.any_replacements() {
            return None;
        }
        for r in self.active_replacements() {
            if let Replace::GainLife { who, instead, .. } = &r.what {
                if !instead.is_empty() && self.player_filter_src(r.source, *who, p, r.controller) {
                    return Some((r.source, r.controller, instead.clone()));
                }
            }
        }
        None
    }

    pub fn replace_lose_life(&self, p: PlayerId, n: i32) -> i32 {
        if !self.any_replacements() {
            return n;
        }
        let mut n = n;
        for r in self.active_replacements() {
            if let Replace::LoseLife { who, multiply, add } = &r.what {
                if self.player_filter_src(r.source, *who, p, r.controller) {
                    n = n * multiply + add;
                }
            }
        }
        n.max(0)
    }

    /// A draw replacement that does other things INSTEAD of drawing ("instead search your library ...", "instead you and that player each draw"):
    /// (the permanent, its controller, the effects).
    pub fn draw_instead(&self, p: PlayerId, first_in_draw_step: bool, count: i32) -> Option<(CardId, PlayerId, Vec<Effect>, Vec<crate::state::Target>, Option<CardId>)> {
        if !self.any_replacements() {
            return None;
        }
        for r in self.active_replacements() {
            if let Replace::Draw { who, except_first_in_draw_step, instead, optional, once, min_cards, bound_then_target, draw_step_only, cycling_other_nonland, .. } = &r.what {
                if instead.is_empty() || !self.player_filter_src(r.source, *who, p, r.controller) {
                    continue;
                }
                if self.draw_chain.borrow().contains(&r.source) {
                    continue;
                }
                if *cycling_other_nonland && !self.draw_cycled.get().map_or(false, |c| c != r.source) {
                    continue;
                }
                // a replacement with `min_cards` only sees draw effects of that many cards; the others only see single draws
                if (count >= 2 && (*min_cards < 2 || *min_cards > count)) || (count < 2 && *min_cards >= 2) {
                    continue;
                }
                if *except_first_in_draw_step && first_in_draw_step {
                    continue;
                }
                if *draw_step_only && !first_in_draw_step {
                    continue;
                }
                if *once {
                    if let Some(i) = r.temp_idx {
                        self.consumed_temp.borrow_mut().push(i);
                    }
                }
                let effects = if *optional {
                    vec![Effect::May { who: None, remember_answer: false, effects: instead.clone(), otherwise: vec![Effect::Draw { who: Sel::TriggerPlayer, n: Amount::Fixed { n: 1 }, plain: true }] }]
                } else {
                    instead.clone()
                };
                return Some((r.source, r.controller, effects, bound_then_target.iter().cloned().collect(), if *cycling_other_nonland { self.draw_cycled.get() } else { None }));
            }
        }
        None
    }

    /// How many cards one "draw a card" actually draws for `p`.
    pub fn draw_count(&self, p: PlayerId, first_in_draw_step: bool) -> i32 {
        if !self.any_replacements() {
            return 1;
        }
        let mut n = 1;
        for r in self.active_replacements() {
            if let Replace::Draw { who, except_first_in_draw_step, multiply, add, instead, once, after_first, cycling_other_nonland, .. } = &r.what {
                if *cycling_other_nonland || !instead.is_empty() || (*after_first && self.players[p as usize].cards_drawn_turn == 0) {
                    continue;
                }
                if !self.player_filter_src(r.source, *who, p, r.controller) {
                    continue;
                }
                if *except_first_in_draw_step && first_in_draw_step {
                    continue;
                }
                if *once {
                    if let Some(i) = r.temp_idx {
                        self.consumed_temp.borrow_mut().push(i);
                    }
                }
                n = n * multiply + add;
            }
        }
        n.max(0)
    }

    /// Cards actually milled when `p` would mill `n` (Replace::Mill, applied once over every active replacement).
    pub fn replace_mill_count(&self, p: PlayerId, n: i32) -> i32 {
        if !self.any_replacements() || n <= 0 {
            return n;
        }
        let mut n = n;
        for r in self.active_replacements() {
            if let Replace::Mill { who, multiply, add } = &r.what {
                if self.player_filter_src(r.source, *who, p, r.controller) {
                    n = n * multiply + add;
                }
            }
        }
        n.max(0)
    }

    /// Cards actually scried when `p` would scry `n` (Replace::Scry).
    pub fn replace_scry_count(&self, p: PlayerId, n: i32) -> i32 {
        if !self.any_replacements() || n <= 0 {
            return n;
        }
        let mut n = n;
        for r in self.active_replacements() {
            if let Replace::Scry { who, add, .. } = &r.what {
                if self.player_filter_src(r.source, *who, p, r.controller) {
                    n += add;
                }
            }
        }
        n.max(0)
    }

    /// A `Replace::Scry.draw_instead` replacement applies to `p`'s scry.
    pub fn scry_becomes_draw(&self, p: PlayerId) -> bool {
        if !self.any_replacements() {
            return false;
        }
        self.active_replacements().iter().any(|r| matches!(&r.what, Replace::Scry { who, draw_instead: true, .. } if self.player_filter_src(r.source, *who, p, r.controller)))
    }

    pub fn replace_token_count(&self, p: PlayerId, n: i32) -> i32 {
        if !self.any_replacements() || n <= 0 {
            return n;
        }
        let mut n = n;
        for r in self.active_replacements() {
            if let Replace::CreateTokens { who, multiply, add, types, subtypes, .. } = &r.what {
                if types.is_empty() && subtypes.is_empty() && self.player_filter_src(r.source, *who, p, r.controller) {
                    n = n * multiply + add;
                }
            }
        }
        n.max(0)
    }

    /// A Mirrormind Crown style replacement that offers `p` copies of the equipped creature instead of tokens: (source, equipped creature).
    pub fn token_copy_offer(&self, p: PlayerId) -> Option<(CardId, CardId)> {
        if !self.any_replacements() {
            return None;
        }
        for r in self.active_replacements() {
            if let Replace::CreateTokens { who, copy_equipped: true, .. } = &r.what {
                let sc = &self.cards[r.source.0 as usize];
                if sc.token_repl_turn == self.turn || !self.player_filter_src(r.source, *who, p, r.controller) {
                    continue;
                }
                if let Some(h) = sc.attached_to {
                    let hc = &self.cards[h.0 as usize];
                    // an Aura (Moonlit Meditation: "enchanted permanent") copies any permanent it is attached to; Equipment only creatures
                    if hc.zone == Zone::Battlefield && (hc.cur.types.has(CardType::Creature) || sc.cur.subtypes.contains(&Sym::new("Aura"))) {
                        return Some((r.source, h));
                    }
                }
            }
        }
        None
    }

    /// A Jinnie Fay style replacement offering `p` a choice of other tokens instead of the ones being created: (source, token card names).
    pub fn token_choice_offer(&self, p: PlayerId) -> Option<(CardId, Vec<String>)> {
        if !self.any_replacements() {
            return None;
        }
        for r in self.active_replacements() {
            if let Replace::CreateTokens { who, instead_choice, .. } = &r.what {
                if !instead_choice.is_empty() && self.player_filter_src(r.source, *who, p, r.controller) {
                    return Some((r.source, instead_choice.clone()));
                }
            }
        }
        None
    }

    /// An Esix style replacement offering `p` copies of a chosen creature other than the source instead of tokens: (source, candidate creatures). Only during `p`'s own turn.
    pub fn token_copy_pick_offer(&self, p: PlayerId) -> Option<(CardId, Vec<CardId>)> {
        if !self.any_replacements() || self.active != p {
            return None;
        }
        for r in self.active_replacements() {
            if let Replace::CreateTokens { who, copy_chosen: true, .. } = &r.what {
                let sc = &self.cards[r.source.0 as usize];
                if sc.token_repl_turn == self.turn || !self.player_filter_src(r.source, *who, p, r.controller) {
                    continue;
                }
                let opts: Vec<CardId> = self.cards.iter().filter(|c| c.zone == Zone::Battlefield && c.id != r.source && c.cur.types.has(CardType::Creature)).map(|c| c.id).collect();
                if !opts.is_empty() {
                    return Some((r.source, opts));
                }
            }
        }
        None
    }

    /// Token creation with the token being known: returns (token to create, how many, extra tokens (card name, how many)).
    pub fn replace_token_plan(&self, p: PlayerId, token: &str, def: &crate::carddb::Def, n: i32) -> (String, i32, Vec<(String, i32)>, PlayerId) {
        let mut token = token.to_string();
        let mut owner = p;
        let mut n = n;
        let mut extras: Vec<(String, i32)> = vec![];
        if !self.any_replacements() || n <= 0 {
            return (token, n, extras, owner);
        }
        let mut def_types = def.types;
        let mut def_subs: Vec<String> = def.subtypes.iter().map(|s| s.name()).collect();
        let mut manu_src: Option<CardId> = None;
        let mut manu_snap: Vec<(String, i32)> = vec![];
        for r in self.active_replacements() {
            if let Replace::CreateTokens { who, multiply, add, types, subtypes, instead_token, extra_token, extra_per_token, extra_tokens, control_instead, copy_equipped, copy_chosen, .. } = &r.what {
                if *copy_equipped || *copy_chosen {
                    continue;
                }
                if !self.player_filter_src(r.source, *who, p, r.controller) {
                    continue;
                }
                // Academy Manufactor: each token it sees (the one being created and the extras other replacements added) is replaced by one of each kind; the six abilities of one Manufactor apply to the same snapshot, a second Manufactor then replaces all of those again
                if *extra_per_token && !subtypes.is_empty() && types.is_empty() && *multiply == 1 && *add == 0 {
                    if manu_src != Some(r.source) {
                        manu_src = Some(r.source);
                        manu_snap = vec![(token.clone(), n)];
                        manu_snap.extend(extras.iter().cloned());
                    }
                    let mut add_now: Vec<(String, i32)> = vec![];
                    for (name, cnt) in &manu_snap {
                        let subs: Vec<String> = self.db.find(name).map(|d| self.db.def(d).subtypes.iter().map(|s| s.name()).collect()).unwrap_or_default();
                        if subtypes.iter().all(|s| subs.contains(s)) {
                            for t in extra_token.iter().chain(extra_tokens.iter()) {
                                add_now.push((t.clone(), (*cnt).max(0)));
                            }
                        }
                    }
                    extras.extend(add_now);
                    continue;
                }
                if !types.is_empty() && !types.iter().any(|t| def_types.has(*t)) {
                    continue;
                }
                if !subtypes.iter().all(|s| def_subs.contains(s)) {
                    continue;
                }
                n = n * multiply + add;
                if *control_instead {
                    owner = r.controller;
                }
                if let Some(t) = instead_token {
                    if let Some(d2) = self.db.find(t) {
                        token = t.clone();
                        let dd = self.db.def(d2);
                        def_types = dd.types;
                        def_subs = dd.subtypes.iter().map(|s| s.name()).collect();
                    }
                }
                if let Some(t) = extra_token {
                    extras.push((t.clone(), if *extra_per_token { n.max(0) } else { 1 }));
                }
                for t in extra_tokens {
                    extras.push((t.clone(), if *extra_per_token { n.max(0) } else { 1 }));
                }
            }
        }
        (token, n.max(0), extras, owner)
    }

    /// Counters about to be put on a permanent.
    /// A "can't get counters while this stays on the battlefield" effect (Suncleanser) bars counters on `o`.
    pub fn counters_barred(&self, o: Obj) -> bool {
        self.no_counters_src.iter().any(|(t, zc, s, sz)| {
            *t == o
                && self.cards[s.0 as usize].zone == Zone::Battlefield
                && self.cards[s.0 as usize].zcc == *sz
                && match t {
                    Obj::Card { id } => self.cards[id.0 as usize].zcc == *zc,
                    _ => true,
                }
        })
    }

    pub fn replace_counters(&self, target: CardId, kind: CounterKind, n: i32) -> i32 {
        if !self.no_counters_src.is_empty() && self.counters_barred(Obj::card(target)) {
            return 0;
        }
        if !self.any_replacements() || n <= 0 {
            return n;
        }
        let ctrl = self.cards[target.0 as usize].cur.controller;
        let mut n = n;
        for r in self.active_replacements() {
            if let Replace::AddCounters { who, on, kind: k, multiply, add, by, halve, modular, .. } = &r.what {
                if (!*modular || self.modular_cause.get()) && (by.is_some() || !self.counters_as_cost.get()) && k.map_or(true, |k| k == kind) && self.player_filter(*who, ctrl, r.controller) && self.matches_card(on, target, r.source, r.controller) && by.map_or(true, |b| self.cause_ctl.get().map_or(false, |c| self.player_filter(b, c, r.controller))) {
                    n = n * multiply + add;
                    if *halve {
                        n /= 2;
                    }
                }
            }
        }
        n.max(0)
    }

    /// Counters put on a player (poison, energy, experience, rad) after replacement effects that apply to players.
    pub fn replace_player_counters(&self, player: PlayerId, kind: CounterKind, n: i32) -> i32 {
        if !self.no_counters_src.is_empty() && self.counters_barred(Obj::Player { id: player }) {
            return 0;
        }
        // Melira, the Living Cure: one poison counter, and no more this turn
        if kind == CounterKind::Poison && n > 0 && self.poison_only_one(player) {
            let bit = 1u32 << (player as u32 & 31);
            if self.poison_capped.get() & bit != 0 {
                return 0;
            }
            self.poison_capped.set(self.poison_capped.get() | bit);
            return 1;
        }
        if !self.any_replacements() || n <= 0 {
            return n;
        }
        let mut n = n;
        for r in self.active_replacements() {
            if let Replace::AddCounters { who, kind: k, multiply, add, players: true, by, halve, .. } = &r.what {
                if k.map_or(true, |k| k == kind) && self.player_filter(*who, player, r.controller) && by.map_or(true, |b| self.cause_ctl.get().map_or(false, |c| self.player_filter(b, c, r.controller))) {
                    n = n * multiply + add;
                    if *halve {
                        n /= 2;
                    }
                }
            }
        }
        n.max(0)
    }

    /// Where damage that would be dealt to `target` goes instead because of a redirecting `Replace::Damage` (Martyrs of Korlis).
    pub fn damage_redirect(&self, source: Option<CardId>, target: &Obj, n: i32, is_combat: bool) -> Option<Obj> {
        if !self.any_replacements() || n <= 0 {
            return None;
        }
        for r in self.active_replacements() {
            let Replace::Damage { source: sf, to, combat, to_this, to_filter, condition, redirect_to: Some(new), redirect_optional: false, .. } = &r.what else { continue };
            if combat.map_or(false, |c| c != is_combat) || (*to_this && !matches!(target, Obj::Card { id } if *id == r.source)) {
                continue;
            }
            let Some(s) = source else { continue };
            if !self.matches_card(sf, s, r.source, r.controller) {
                continue;
            }
            let to_ok = match (to, target) {
                (DamageTo::Any, _) => true,
                (DamageTo::Player, Obj::Player { .. }) => true,
                (DamageTo::PlayerOrBattle, Obj::Player { .. }) => true,
                (DamageTo::PlayerOrBattle, Obj::Card { id }) => self.cards[id.0 as usize].cur.types.has(CardType::Battle),
                (DamageTo::You, Obj::Player { id }) => *id == r.controller,
                (DamageTo::Opponent, Obj::Player { id }) => *id != r.controller,
                (DamageTo::Creature, Obj::Card { id }) => self.cards[id.0 as usize].cur.types.has(CardType::Creature),
                (DamageTo::Planeswalker, Obj::Card { id }) => self.cards[id.0 as usize].cur.types.has(CardType::Planeswalker),
                _ => false,
            };
            let to_ok = to_ok && to_filter.as_ref().map_or(true, |tf| matches!(target, Obj::Card { id } if self.cards[id.0 as usize].zone == Zone::Battlefield && self.matches_card(tf, *id, r.source, r.controller)));
            let cond_ok = condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &crate::eval::EvalCtx::with_controller(r.source, r.controller)));
            if !(to_ok && cond_ok) {
                continue;
            }
            let c = crate::eval::EvalCtx::with_controller(r.source, r.controller);
            let dest = self.sel_objs(new, &c).into_iter().next();
            if let Some(d) = dest {
                if d != *target {
                    return Some(d);
                }
            }
        }
        None
    }

    pub fn replace_damage(&self, source: Option<CardId>, target: &Obj, n: i32, is_combat: bool) -> i32 {
        if !self.any_replacements() || n <= 0 {
            return n;
        }
        let mut n = n;
        let tcard = if let Obj::Card { id } = target { Some(*id) } else { None };
        let tplayer = if let Obj::Player { id } = target { Some(*id) } else { None };
        let unprev = self.damage_unpreventable();
        for r in self.active_replacements() {
            if let Replace::Damage { source: sf, to, multiply, add, prevent, combat, cause_targets_recipient, to_this, at_most, at_least, to_filter, to_filter_or_you, set_to, then, then_always, instead, once_per_batch, once, prevent_up_to, bound_source, bound_to, divide_by, prevent_fraction, add_amount, prevent_amount, leave_at_least, condition, to_chosen_player_or_permanents, bound_player, bound_to_player, bind_chosen_color, bound_color, bound_then_target, between_chosen_players, redirect_to: opt_redirect, redirect_optional, raise_to_amount, .. } = &r.what {
                if *redirect_optional && self.skip_opt_redirect.get() {
                    continue;
                }
                // a redirect to its own source does nothing once the source left the battlefield (Kjeldoran Royal Guard)
                if then.iter().any(|e| matches!(e, Effect::DealDamage { to: Sel::This, .. })) && self.cards[r.source.0 as usize].zone != Zone::Battlefield {
                    continue;
                }
                if *between_chosen_players {
                    let (a, b) = (self.cards[r.source.0 as usize].chosen_player, self.cards[r.source.0 as usize].chosen_player2);
                    let victim = match target { Obj::Player { id } => Some(*id), Obj::Card { id } => Some(self.cards[id.0 as usize].cur.controller), _ => None };
                    let sctl = source.map(|s| self.cards[s.0 as usize].cur.controller);
                    let hit = matches!((a, b, victim, sctl), (Some(a), Some(b), Some(v), Some(s)) if (s == a && v == b) || (s == b && v == a));
                    if !hit {
                        continue;
                    }
                }
                if *to_chosen_player_or_permanents {
                    let pl = bound_player.or(self.cards[r.source.0 as usize].chosen_player);
                    let hit = match (target, pl) {
                        (Obj::Player { id }, Some(p)) => *id == p,
                        (Obj::Card { id }, Some(p)) => self.cards[id.0 as usize].cur.controller == p,
                        _ => false,
                    };
                    if !hit {
                        continue;
                    }
                }
                if at_most.map_or(false, |m| n > m) || at_least.map_or(false, |m| n < m) {
                    continue;
                }
                if combat.map_or(false, |c| c != is_combat) {
                    continue;
                }
                if *cause_targets_recipient && !self.cause_targets.borrow().contains(target) {
                    continue;
                }
                if bound_source.map_or(false, |b| source != Some(b)) {
                    continue;
                }
                if bound_to.map_or(false, |b| !matches!(target, Obj::Card { id } if *id == b)) {
                    continue;
                }
                if bound_to_player.map_or(false, |b| !matches!(target, Obj::Player { id } if *id == b)) {
                    continue;
                }
                if *bind_chosen_color && !source.map_or(false, |s| bound_color.map_or(false, |c| self.cards[s.0 as usize].cur.colors.has(c))) {
                    continue;
                }
                if *to_this && !matches!(target, Obj::Card { id } if *id == r.source) {
                    continue;
                }
                let src_ok = match source {
                    Some(s) => self.matches_card(sf, s, r.source, r.controller),
                    None => false,
                };
                let to_ok = match (to, target) {
                    (DamageTo::Any, _) => true,
                    (DamageTo::Player, Obj::Player { .. }) => true,
                    (DamageTo::PlayerOrBattle, Obj::Player { .. }) => true,
                    (DamageTo::PlayerOrBattle, Obj::Card { id }) => self.cards[id.0 as usize].cur.types.has(CardType::Battle),
                    (DamageTo::You, Obj::Player { id }) => *id == r.controller,
                    (DamageTo::Opponent, Obj::Player { id }) => *id != r.controller,
                    (DamageTo::EnchantedPlayer, Obj::Player { id }) => self.cards[r.source.0 as usize].enchanted_player == Some(*id),
                    (DamageTo::Creature, Obj::Card { id }) => self.cards[id.0 as usize].cur.types.has(CardType::Creature),
                    (DamageTo::Planeswalker, Obj::Card { id }) => self.cards[id.0 as usize].cur.types.has(CardType::Planeswalker),
                    (DamageTo::Battle, Obj::Card { id }) => self.cards[id.0 as usize].cur.types.has(CardType::Battle),
                    _ => false,
                };
                let to_ok = if *to_filter_or_you && matches!(target, Obj::Player { id } if *id == r.controller) {
                    true
                } else {
                    to_ok && to_filter.as_ref().map_or(true, |tf| matches!(target, Obj::Card { id } if self.cards[id.0 as usize].zone == Zone::Battlefield && self.matches_card(tf, *id, r.source, r.controller)))
                };
                let cond_ok = condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &crate::eval::EvalCtx::with_controller(r.source, r.controller)));
                let raise_v = raise_to_amount.as_ref().map(|a| self.eval_amount(a, &crate::eval::EvalCtx::with_controller(r.source, r.controller)));
                let cond_ok = cond_ok && raise_v.map_or(true, |v| n < v);
                if src_ok && to_ok && cond_ok {
                    if *once {
                        if let Some(i) = r.temp_idx {
                            self.consumed_temp.borrow_mut().push(i);
                        }
                    }
                    if let (true, Some(new)) = (*redirect_optional, opt_redirect) {
                        // hold the damage back; a follow-up asks whether it goes to the new recipient, else it is dealt to the original one
                        let back = match target {
                            Obj::Player { .. } => Sel::TriggerPlayer,
                            _ => Sel::TriggerDamaged,
                        };
                        let eff = vec![Effect::May {
                            who: None,
                            remember_answer: false,
                            effects: vec![Effect::DealDamageAgain { to: new.clone(), combat: is_combat, plain: true, keep_combat: false, less: None }],
                            otherwise: vec![Effect::DealDamageAgain { to: back, combat: is_combat, plain: true, keep_combat: false, less: None }],
                        }];
                        self.dmg_followups.borrow_mut().push((r.source, r.controller, eff, n, source, tcard, tplayer, vec![]));
                        return 0;
                    }
                    if let (true, None, Some(amt)) = (*redirect_optional, opt_redirect, prevent_amount) {
                        // "you may prevent X of that damage" (Battletide Alchemist): hold the damage back; a follow-up asks, then deals it reduced by X or in full
                        let back = match target {
                            Obj::Player { .. } => Sel::TriggerPlayer,
                            _ => Sel::TriggerDamaged,
                        };
                        let eff = vec![Effect::May {
                            who: None,
                            remember_answer: false,
                            effects: vec![Effect::DealDamageAgain { to: back.clone(), combat: is_combat, plain: true, keep_combat: false, less: Some(amt.clone()) }],
                            otherwise: vec![Effect::DealDamageAgain { to: back, combat: is_combat, plain: true, keep_combat: false, less: None }],
                        }];
                        self.dmg_followups.borrow_mut().push((r.source, r.controller, eff, n, source, tcard, tplayer, vec![]));
                        return 0;
                    }
                    if let Some(cap) = prevent_up_to {
                        let p = n.min(*cap);
                        if let Some(i) = r.temp_idx {
                            self.reduced_temp.borrow_mut().push((i, cap - p));
                        }
                        if p > 0 && !then.is_empty() {
                            self.dmg_followups.borrow_mut().push((r.source, r.controller, then.clone(), p, source, tcard, tplayer, bound_then_target.iter().cloned().collect()));
                        }
                        n -= p;
                        if n <= 0 {
                            return 0;
                        }
                        continue;
                    }
                    if *prevent && unprev && !*instead {
                        // damage that can't be prevented is still dealt, but the rest of the effect happens (Gloom Surgeon still exiles the cards)
                        if !then.is_empty() && *then_always {
                            self.dmg_followups.borrow_mut().push((r.source, r.controller, then.clone(), n, source, tcard, tplayer, bound_then_target.iter().cloned().collect()));
                        }
                        continue;
                    }
                    if *prevent {
                        let batch_skip = *once_per_batch && self.prevent_batch.borrow_mut().as_mut().map_or(false, |b| if b.contains(&r.source) { true } else { b.push(r.source); false });
                        if !then.is_empty() && !batch_skip {
                            self.dmg_followups.borrow_mut().push((r.source, r.controller, then.clone(), n, source, tcard, tplayer, bound_then_target.iter().cloned().collect()));
                        }
                        return 0;
                    }
                    let extra = add_amount.as_ref().map_or(0, |a| self.eval_amount(a, &crate::eval::EvalCtx::with_controller(r.source, r.controller)));
                    let mut new_n = match set_to {
                        Some(v) => *v,
                        None => n * multiply + add + extra,
                    };
                    if let Some(v) = raise_v {
                        new_n = v;
                    }
                    if let Some(k) = divide_by {
                        new_n = n / (*k).max(1);
                    }
                    if let Some(k) = prevent_fraction {
                        new_n = n - n / (*k).max(1);
                    }
                    if let Some(a) = prevent_amount {
                        let x = self.eval_amount(a, &crate::eval::EvalCtx::with_controller(r.source, r.controller)).max(0);
                        new_n = (n - x).max(0);
                    }
                    if let (Some(l), Obj::Player { id }) = (leave_at_least, target) {
                        new_n = new_n.min((self.players[*id as usize].life - *l).max(0));
                    }
                    if unprev && new_n < n && !*instead {
                        new_n = n;
                    }
                    let batch_skip = *once_per_batch && self.prevent_batch.borrow_mut().as_mut().map_or(false, |b| if b.contains(&r.source) { true } else { b.push(r.source); false });
                    if !then.is_empty() && (new_n < n || *then_always) && !batch_skip {
                        self.dmg_followups.borrow_mut().push((r.source, r.controller, then.clone(), if *then_always { new_n.max(0) } else { n - new_n.max(0) }, source, tcard, tplayer, bound_then_target.iter().cloned().collect()));
                    }
                    n = new_n;
                }
            }
        }
        n.max(0)
    }

    /// The card's own conditional "isn't a creature" static (Theros gods) is already in force as it would enter: it enters as a non-creature.
    fn enters_without_creature_type(&self, id: CardId) -> bool {
        let def = self.db.def(self.cards[id.0 as usize].def);
        def.abilities.iter().any(|a| match a {
            Ability::Static { effect: StaticEffect::SetTypes { affects, types, .. }, condition: Some(c), from: None } => {
                affects.this && !types.contains(&CardType::Creature) && self.eval_cond(c, &EvalCtx::of(self, id))
            }
            _ => false,
        })
    }

    /// Where a card changing zones really ends up.
    pub fn replace_zone(&self, id: CardId, from: Zone, to: Zone) -> (Zone, bool) {
        if !self.any_replacements() {
            return (to, false);
        }
        let mut bottom = false;
        let owner = self.cards[id.0 as usize].owner;
        let mut to = to;
        let orig_to = to;
        let mut first_applied: Option<(CardId, PlayerId, Vec<crate::dsl::Effect>)> = None;
        let reps = self.active_replacements();
        for r in reps.iter() {
            if let Replace::ZoneChange { filter, from: f, to: t, instead, whose, bottom: b, then, cause_opponent, cause_effect, optional, once, bound_to, on_resolve, bound_damager, bind_damager, not_cycled, not_from_battlefield, .. } = &r.what {
                if *t == to
                    && (!*not_cycled || !self.cycling_move.get())
                    && (!*not_from_battlefield || from != Zone::Battlefield)
                    && (!*cause_opponent || self.cause_ctl.get().map_or(false, |c| c != owner))
                    && (!*cause_effect || self.cause_ctl.get().is_some())
                    && f.map_or(true, |z| z == from)
                    && bound_to.map_or(true, |b| b == id)
                    && (bind_damager.is_none() || bound_damager.map_or(false, |d| self.cards[id.0 as usize].damaged_by.contains(&d)))
                    && (!*on_resolve || self.resolved_ok.get())
                    && self.player_filter_src(r.source, *whose, owner, r.controller)
                    && !(to == Zone::Battlefield && filter.types.contains(&CardType::Creature) && self.enters_without_creature_type(id))
                    && (if from == Zone::Battlefield { self.matches_card(filter, id, r.source, r.controller) } else { self.matches_card_ignore_controller(filter, id, r.source, r.controller) })
                {
                    if *optional {
                        // the card really goes to `to`; its owner is asked afterwards whether to move it on to `instead`
                        self.opt_zone_tmp.borrow_mut().push((id, *instead, *b));
                        continue;
                    }
                    to = *instead;
                    if *once {
                        if let Some(i) = r.temp_idx {
                            self.consumed_temp.borrow_mut().push(i);
                        }
                    }
                    if to == Zone::Exile {
                        self.repl_exile_src.set(Some(r.source));
                    }
                    bottom = *b;
                    if first_applied.is_none() && to == *instead && orig_to != to {
                        first_applied = Some((r.source, r.controller, then.clone()));
                    } else if !then.is_empty() {
                        self.dmg_followups.borrow_mut().push((r.source, r.controller, then.clone(), 1, Some(id), Some(id), Some(owner), vec![]));
                    }
                }
            }
        }
        // several replacements send the card to the same place (Nemata and Rest in Peace): the card's owner chooses which one applies,
        // which matters only for their "then" follow-ups; the choice is asked once the move is done
        if let Some((src, ctrl, then)) = first_applied {
            let mut cands: Vec<(CardId, PlayerId, Vec<crate::dsl::Effect>)> = vec![(src, ctrl, then)];
            for r in reps.iter() {
                if r.source == src {
                    continue;
                }
                if let Replace::ZoneChange { filter, from: f, to: t, instead, whose, then, cause_opponent, cause_effect, optional: false, bound_to, on_resolve, bound_damager, bind_damager, not_cycled, not_from_battlefield, .. } = &r.what {
                    if *t == orig_to
                    && (!*not_cycled || !self.cycling_move.get())
                    && (!*not_from_battlefield || from != Zone::Battlefield)
                    && (!*cause_opponent || self.cause_ctl.get().map_or(false, |c| c != owner))
                    && (!*cause_effect || self.cause_ctl.get().is_some())
                    && f.map_or(true, |z| z == from)
                    && bound_to.map_or(true, |b| b == id)
                    && (bind_damager.is_none() || bound_damager.map_or(false, |d| self.cards[id.0 as usize].damaged_by.contains(&d)))
                    && (!*on_resolve || self.resolved_ok.get())
                    && self.player_filter_src(r.source, *whose, owner, r.controller)
                    && !(orig_to == Zone::Battlefield && filter.types.contains(&CardType::Creature) && self.enters_without_creature_type(id))
                    && (if from == Zone::Battlefield { self.matches_card(filter, id, r.source, r.controller) } else { self.matches_card_ignore_controller(filter, id, r.source, r.controller) })
                    {
                        if *instead == to {
                            cands.push((r.source, r.controller, then.clone()));
                        }
                    }
                }
            }
            if cands.len() > 1 && cands.iter().any(|c| !c.2.is_empty()) {
                self.zone_choice_tmp.borrow_mut().push((id, cands));
            } else if let Some((s0, c0, t0)) = cands.into_iter().next() {
                if !t0.is_empty() {
                    self.dmg_followups.borrow_mut().push((s0, c0, t0, 1, Some(id), Some(id), Some(owner), vec![]));
                }
            }
        }
        (to, bottom)
    }
    /// Filter match that doesn't use the card's controller (cards outside the battlefield have none).
    pub(crate) fn matches_card_ignore_controller_zone(&self, f: &Filter, id: CardId, source: CardId, ctrl: PlayerId) -> bool {
        self.matches_card(f, id, source, ctrl)
    }

    pub(crate) fn matches_card_ignore_controller(&self, f: &Filter, id: CardId, source: CardId, ctrl: PlayerId) -> bool {
        let mut f2 = f.clone();
        f2.controller = PlayerFilter::Any;
        self.matches_card(&f2, id, source, ctrl)
    }

    /// Extra explores for a creature that would explore (Replace::Explore, summed over every active replacement).
    pub fn explore_extra(&self, id: CardId) -> u8 {
        if !self.any_replacements() {
            return 0;
        }
        self.active_replacements().iter().map(|r| match &r.what {
            Replace::Explore { filter, extra, .. } if self.matches_card(filter, id, r.source, r.controller) => *extra,
            _ => 0,
        }).sum()
    }

    /// Cards scried first when a creature would explore (Replace::Explore.scry).
    pub fn explore_scry(&self, id: CardId) -> u8 {
        if !self.any_replacements() {
            return 0;
        }
        self.active_replacements().iter().map(|r| match &r.what {
            Replace::Explore { filter, scry, .. } if self.matches_card(filter, id, r.source, r.controller) => *scry,
            _ => 0,
        }).sum()
    }

    /// Cards drawn first when a creature would connive (Replace::Connive).
    pub fn connive_draw(&self, id: CardId) -> u8 {
        if !self.any_replacements() {
            return 0;
        }
        self.active_replacements().iter().map(|r| match &r.what {
            Replace::Connive { filter, draw } if self.matches_card(filter, id, r.source, r.controller) => *draw,
            _ => 0,
        }).sum()
    }

    /// Who controls a permanent entering the battlefield: `intended` unless its own "enters under an opponent's control" ability or a
    /// `Replace::EntersControlled` (Gather Specimens) changes that.
    pub fn enters_controller(&self, id: CardId, intended: PlayerId) -> PlayerId {
        let mut who = intended;
        let d = self.db.def(self.cards[id.0 as usize].def);
        if d.abilities.iter().any(|a| matches!(a, crate::dsl::Ability::EntersUnderOpponent)) {
            if let Some(o) = self.opponents(who).into_iter().find(|q| self.alive(*q)) {
                who = o;
            }
        }
        if self.any_replacements() {
            for r in self.active_replacements().iter() {
                if let Replace::EntersControlled { filter } = &r.what {
                    if r.controller != who && self.opponents(r.controller).contains(&who) && self.matches_card_ignore_controller_zone(filter, id, r.source, r.controller) {
                        who = r.controller;
                    }
                }
            }
        }
        who
    }

    /// Permanents that enter untapped because of someone else's static ability (overrides every enters-tapped effect).
    pub fn enters_untapped_by_replacement(&self, id: CardId) -> bool {
        if !self.any_replacements() {
            return false;
        }
        self.active_replacements().iter().any(|r| match &r.what {
            Replace::EntersUntapped { filter } => r.source != id && self.matches_card(filter, id, r.source, r.controller),
            _ => false,
        })
    }

    /// Permanents that enter tapped because of someone else's static ability.
    pub fn enters_tapped_by_replacement(&self, id: CardId) -> bool {
        if !self.any_replacements() {
            return false;
        }
        self.active_replacements().iter().any(|r| match &r.what {
            Replace::EntersTapped { filter } => r.source != id && self.matches_card(filter, id, r.source, r.controller),
            _ => false,
        })
    }

    /// "Creatures you control enter as a copy of X": the entering permanent takes the copied permanent's definition before it enters.
    pub fn enters_as_copy_by_replacement(&mut self, id: CardId) {
        if !self.any_replacements() {
            return;
        }
        let reps: Vec<(CardId, PlayerId, Sel, Option<CardId>, Option<usize>)> = self
            .active_replacements()
            .iter()
            .filter_map(|r| match &r.what {
                Replace::EntersAsCopyOf { filter, of, once, bound_of, .. } if r.source != id && self.matches_card_ignore_controller_zone(filter, id, r.source, r.controller) => Some((r.source, r.controller, of.clone(), *bound_of, if *once { r.temp_idx } else { None })),
                _ => None,
            })
            .collect();
        for (src, ctl, of, bound, used) in reps {
            let from = bound.or_else(|| self.sel_cards(&of, &EvalCtx::with_controller(src, ctl)).first().copied());
            if let Some(from) = from {
                if from != id && self.cards[from.0 as usize].zone == Zone::Battlefield {
                    self.cards[id.0 as usize].def = self.cards[from.0 as usize].def;
                    self.layers_dirty = true;
                    if let Some(i) = used {
                        self.consumed_temp.borrow_mut().push(i);
                        self.drain_consumed_replaces();
                    }
                    return;
                }
            }
        }
    }

    /// "As a historic permanent enters, it becomes a 7/7 Dinosaur creature" (`Replace::EntersAsCreature`).
    pub fn enters_as_creature_by_replacement(&mut self, id: CardId) {
        if !self.any_replacements() {
            return;
        }
        let reps: Vec<(CardId, PlayerId, String, i32, i32)> = self
            .active_replacements()
            .iter()
            .filter_map(|r| match &r.what {
                Replace::EntersAsCreature { filter, subtype, power, toughness } if self.matches_card_ignore_controller_zone(filter, id, r.source, r.controller) => Some((r.source, r.controller, subtype.clone(), *power, *toughness)),
                _ => None,
            })
            .collect();
        for (src, ctl, sub, p, t) in reps {
            self.add_effect(src, ctl, &[id], Modif::AddTypes(TypeSet::of(&[CardType::Creature])), Duration::Forever);
            self.add_effect(src, ctl, &[id], Modif::AddSubtype(Sym::new(&sub)), Duration::Forever);
            self.add_effect(src, ctl, &[id], Modif::SetPT { power: p, toughness: t }, Duration::Forever);
        }
    }

    /// Extra counters a permanent enters with because of static abilities.
    pub fn enters_counters_by_replacement(&mut self, id: CardId) {
        if !self.any_replacements() {
            return;
        }
        // the amount is worked out as the permanent enters, so it doesn't count itself
        let pos = self.battlefield.iter().position(|b| *b == id);
        if let Some(p) = pos {
            self.battlefield.remove(p);
        }
        // the permanent is judged as it would exist on the battlefield: creature types granted to it by permanents already there count
        // ("each creature you control is the chosen type": Xenograft + Bramblewood Paragon counts a Runeclaw Bear as a Warrior as it enters)
        let saved_subtypes = self.cards[id.0 as usize].cur.subtypes.clone();
        let mut granted: Vec<(Sym, bool)> = vec![];
        for &b in self.battlefield.iter() {
            let bc = &self.cards[b.0 as usize];
            if bc.cur.no_abilities {
                continue;
            }
            let Some(t) = bc.chosen_type else { continue };
            for ab in self.db.def(bc.def).abilities.iter() {
                if let Ability::Static { effect: StaticEffect::AddChosenSubtype { affects, replace, .. }, condition: None, .. } = ab {
                    if self.matches_card(&affects.clone().into_filter(), id, b, bc.cur.controller) {
                        granted.push((t, *replace));
                    }
                }
            }
        }
        for (t, replace) in granted {
            let st = &mut self.cards[id.0 as usize].cur.subtypes;
            if replace {
                st.clear();
                st.push(t);
            } else if !st.contains(&t) {
                st.push(t);
            }
        }
        let mut list: Vec<(CounterKind, i32)> = vec![];
        for r in self.active_replacements().iter() {
            if let Replace::EntersWithSourceCounterKinds { filter } = &r.what {
                if r.source != id && self.matches_card_ignore_controller_zone(filter, id, r.source, r.controller) {
                    list.extend(self.cards[r.source.0 as usize].counters.iter().filter(|(_, n)| *n > 0).map(|(k, _)| (*k, 1)));
                }
            }
        }
        let rest: Vec<(CounterKind, i32)> = self
            .active_replacements()
            .iter()
            .filter_map(|r| match &r.what {
                Replace::EntersWithCounters { filter, kind, n, once } if r.source != id && self.matches_card_ignore_controller_zone(filter, id, r.source, r.controller) => {
                    if *once {
                        if let Some(i) = r.temp_idx {
                            self.consumed_temp.borrow_mut().push(i);
                        }
                    }
                    // `TriggerObject` in the amount is the permanent that's entering (Coin of Mastery: mana spent to cast it)
                    let ev = EventCtx { obj: Some(Obj::card(id)), ..Default::default() };
                    let mut ec = EvalCtx::with_controller(r.source, r.controller);
                    ec.event = &ev;
                    let n = self.eval_amount(n, &ec);
                    Some((*kind, n))
                }
                _ => None,
            })
            .collect();
        list.extend(rest);
        self.cards[id.0 as usize].cur.subtypes = saved_subtypes;
        if let Some(p) = pos {
            self.battlefield.insert(p.min(self.battlefield.len()), id);
        }
        self.drain_consumed_replaces();
        for (k, n) in list {
            let n = self.replace_counters(id, k, n);
            self.cards[id.0 as usize].add_counters(k, n);
            if n > 0 {
                let by = self.cause_ctl.get().or(Some(self.cards[id.0 as usize].cur.controller));
                self.emit(Event::Counters { card: id, kind: k, delta: n, by });
            }
        }
    }

    /// How many extra times a trigger fires because of `cause` (Panharmonicon). Counts only the trigger source controller's replacements.
    pub fn trigger_extra(&self, cause: CardId, event: TriggerCause, trigger_controller: PlayerId, trigger_source: CardId) -> i32 {
        if !self.any_replacements() {
            return 0;
        }
        let mut extra = 0;
        for r in self.active_replacements() {
            if let Replace::TriggerExtra { cause: f, event: e, additional, own_or_emblem } = &r.what {
                if *own_or_emblem && !(trigger_source == r.source || (self.cards[trigger_source.0 as usize].zone == Zone::Command && self.cards[trigger_source.0 as usize].owner == r.controller)) {
                    continue;
                }
                if *e == event && r.controller == trigger_controller {
                    let ok = match self.lki_slot.as_ref().filter(|l| l.card == cause) {
                        Some(l) => self.matches_view(f, &self.view_lki(l), r.source, r.controller),
                        None => self.matches_card(f, cause, r.source, r.controller),
                    };
                    if ok {
                        extra += additional;
                    }
                }
            }
        }
        // a permanent that left the battlefield at the same time as the dying creature still counts (Drivnod dying with Cordial Vampire: look-back)
        if event == TriggerCause::Dies {
            let mut leavers: Vec<(CardId, PlayerId)> = self.batch_left.clone();
            if let Some(l) = self.lki_slot.as_ref() {
                if !leavers.iter().any(|(c, _)| *c == l.card) {
                    leavers.push((l.card, l.controller));
                }
            }
            for (lid, lctl) in leavers {
                if lctl != trigger_controller || self.cards[lid.0 as usize].zone == Zone::Battlefield || self.batch_noabil.contains(&lid) {
                    continue;
                }
                let d = self.db.def(self.cards[lid.0 as usize].def);
                for a in d.abilities.iter() {
                    if let Ability::Replace { what: Replace::TriggerExtra { cause: f, event: e, additional, own_or_emblem: false }, from: None, .. } = a {
                        if *e == event {
                            let ok = match self.lki_slot.as_ref().filter(|l| l.card == cause) {
                                Some(l) => self.matches_view(f, &self.view_lki(l), lid, lctl),
                                None => self.matches_card(f, cause, lid, lctl),
                            };
                            if ok {
                                extra += additional;
                            }
                        }
                    }
                }
            }
        }
        // a creature that left the battlefield with a granted "triggers an additional time" (The Masamune): it had the ability as it left,
        // for its own death and for creatures dying at the same time
        let from_batch: Vec<&std::sync::Arc<Ability>> = self.batch_granted.iter().filter(|(id, _)| *id == trigger_source).map(|(_, a)| a).collect();
        let from_lki: Vec<&std::sync::Arc<Ability>> = if from_batch.is_empty() && cause == trigger_source {
            self.lki_slot.as_ref().filter(|l| l.card == cause && l.controller == trigger_controller).map(|l| l.derived.granted.iter().collect()).unwrap_or_default()
        } else {
            vec![]
        };
        for g in from_batch.into_iter().chain(from_lki) {
            if let Ability::Replace { what: Replace::TriggerExtra { cause: f, event: e, additional, own_or_emblem: true }, from: None, .. } = &**g {
                if *e == event {
                    let ok = match self.lki_slot.as_ref().filter(|l| l.card == cause) {
                        Some(l) => self.matches_view(f, &self.view_lki(l), trigger_source, trigger_controller),
                        None => self.matches_card(f, cause, trigger_source, trigger_controller),
                    };
                    if ok {
                        extra += additional;
                    }
                }
            }
        }
        extra
    }
}

impl Game {
    /// A dynamic "if you tap a land for mana, it produces one mana of a color of your choice instead" (Harvest Mage) applies to this permanent.
    pub fn mana_any_color(&self, card: CardId) -> bool {
        self.effects.iter().any(|e| matches!(&e.modif, Modif::DynStatic(se) if matches!(&**se, StaticEffect::ManaTypeReplace { filter, any_color: true, .. } if self.matches_card(filter, card, e.source, e.controller))))
            // a permanent's own static (Pulse of Llanowar)
            || self.battlefield.iter().any(|&id| {
                let c = &self.cards[id.0 as usize];
                if c.cur.no_abilities {
                    return false;
                }
                let d = self.db.def(c.def);
                d.statics.iter().map(|&i| &d.abilities[i as usize]).any(|ab| {
                    matches!(ab, Ability::Static { effect: StaticEffect::ManaTypeReplace { filter, any_color: true, .. }, condition, .. }
                        if self.matches_card(filter, card, id, c.cur.controller) && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(id, c.cur.controller))))
                })
            })
    }
}
