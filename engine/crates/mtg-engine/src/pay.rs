//! Mana: finding sources, planning a payment, activating mana abilities.

use crate::decision::*;
use crate::dsl::*;
use crate::ids::*;
use crate::mana::*;
use crate::state::*;
use crate::types::*;

#[derive(Clone, Debug, PartialEq)]
pub enum PayUse {
    Pool(ManaType),
    Source { card: CardId, ability: u16, produce: ManaType },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    pub uses: Vec<PayUse>,
    pub life: i32,
}

/// Every way to split `total` mana over `k` colours (counts per colour, summing to `total`), stopping with `false` once `out` passes `limit` splits.
fn unit_splits(total: u8, k: usize, out: &mut Vec<Vec<u8>>, cur: &mut Vec<u8>, limit: usize) -> bool {
    if k == 1 {
        cur.push(total);
        out.push(cur.clone());
        cur.pop();
        return out.len() <= limit;
    }
    for x in 0..=total {
        cur.push(x);
        let ok = unit_splits(total - x, k - 1, out, cur, limit);
        cur.pop();
        if !ok {
            return false;
        }
    }
    true
}

fn push_unique(v: &mut Vec<ManaType>, t: ManaType) {
    if !v.contains(&t) {
        v.push(t)
    }
}

impl Game {
    /// Colors in the color identity of `p`'s commander(s); empty when they have none.
    pub fn commander_identity(&self, p: PlayerId) -> ColorSet {
        let mut id = ColorSet::NONE;
        for c in &self.cards {
            if c.is_commander && c.owner == p {
                id.0 |= self.db.def(c.front).identity.0;
            }
        }
        id
    }

    /// Spells `p` controls on the stack (not copies) that match `filter`, topmost first.
    pub(crate) fn stack_spells_for_cost(&self, p: PlayerId, source: CardId, filter: &Filter) -> Vec<CardId> {
        let mut f = filter.clone();
        f.controller = PlayerFilter::Any;
        self.stack
            .iter()
            .rev()
            .filter(|s| matches!(s.kind, ItemKind::Spell { .. }) && !s.copy && s.controller == p && self.cards[s.source.0 as usize].zone == Zone::Stack && self.matches_card(&f, s.source, source, p))
            .map(|s| s.source)
            .collect()
    }

    /// Every mana type the card could produce through any of its mana abilities (printed or granted).
    pub fn card_mana_types(&self, id: CardId) -> Vec<ManaType> {
        let c = &self.cards[id.0 as usize];
        let d = self.db.def(c.def);
        let mut v: Vec<ManaType> = vec![];
        let mut add = |effects: &[Effect], v: &mut Vec<ManaType>| {
            for t in self.produces_for(c.cur.controller, effects, 0) {
                push_unique(v, t);
            }
        };
        if !c.cur.no_abilities {
            for &ai in &d.mana_abilities {
                if let Ability::Activated { effects, .. } = &d.abilities[ai as usize] {
                    add(effects, &mut v);
                }
            }
        }
        for a in c.cur.granted.iter() {
            if let Ability::Activated { effects, .. } = &**a {
                if crate::carddb::is_mana_ability(a) {
                    add(effects, &mut v);
                }
            }
        }
        // intrinsic mana abilities of basic land types
        if c.cur.types.has(CardType::Land) && !c.cur.no_land_mana {
            for (ci, name) in ["Plains", "Island", "Swamp", "Mountain", "Forest"].iter().enumerate() {
                if c.cur.subtypes.contains(&Sym::new(name)) {
                    push_unique(&mut v, ManaType::of(Color::ALL[ci]));
                }
            }
        }
        v
    }

    /// Mana types `p`'s permanent could produce with these effects (dynamic ones depend on the game).
    pub fn produces_for(&self, p: PlayerId, effects: &[Effect], depth: u8) -> Vec<ManaType> {
        let mut v: Vec<ManaType> = vec![];
        for e in effects {
            let Effect::AddMana { mana, .. } = e else { continue };
            for m in mana {
                match m {
                    ManaProduce::Restricted { chosen: true, .. } => {
                        for c in Color::ALL {
                            push_unique(&mut v, ManaType::of(c));
                        }
                    }
                    ManaProduce::Fixed { m, .. } | ManaProduce::Scaled { m, .. } | ManaProduce::Restricted { m, .. } | ManaProduce::Rider { m, .. } => push_unique(&mut v, *m),
                    ManaProduce::NotedType | ManaProduce::NotedMana { .. } => {
                        for t in ManaType::ALL {
                            push_unique(&mut v, t);
                        }
                    }
                    ManaProduce::ChosenColor { .. } | ManaProduce::SecondChosenColor { .. } | ManaProduce::ChosenColorScaled { .. } => {
                        for c in Color::ALL {
                            push_unique(&mut v, ManaType::of(c));
                        }
                    }
                    // "add X mana in any combination of {R}, {W} and/or {B}": only the listed colours
                    ManaProduce::AnyColorScaled { colors, .. } if !colors.is_empty() => {
                        for c in colors {
                            push_unique(&mut v, ManaType::of(*c));
                        }
                    }
                    ManaProduce::OneProduced | ManaProduce::CostOf { .. } => {}
                    ManaProduce::DevotionOneColor | ManaProduce::MilledSymbols | ManaProduce::AnyColor { .. } | ManaProduce::AnyColorRider { .. } | ManaProduce::RestrictedAny { .. } | ManaProduce::OwnColorRestricted { .. } | ManaProduce::AnyOneColor { .. } | ManaProduce::AnyOneColorScaled { .. } | ManaProduce::AnyColorScaled { .. } | ManaProduce::AnyColorYouCould => {
                        for c in Color::ALL {
                            push_unique(&mut v, ManaType::of(c))
                        }
                    }
                    ManaProduce::AnyCommanderColor | ManaProduce::AnyCommanderColorRider { .. } => {
                        for c in Color::ALL {
                            if self.commander_identity(p).has(c) {
                                push_unique(&mut v, ManaType::of(c))
                            }
                        }
                    }
                    ManaProduce::AnyColorOfExiledWith => {
                        for c in Color::ALL {
                            push_unique(&mut v, ManaType::of(c));
                        }
                    }
                    ManaProduce::AnyColorAmong { filter, zone: Some(Zone::Graveyard) } => {
                        for &id in &self.players[p as usize].graveyard {
                            if !self.matches_card(filter, id, id, p) {
                                continue;
                            }
                            for col in Color::ALL {
                                if self.db.def(self.cards[id.0 as usize].def).colors.has(col) {
                                    push_unique(&mut v, ManaType::of(col));
                                }
                            }
                        }
                    }
                    ManaProduce::AnyColorAmong { filter, .. } => {
                        for &id in &self.battlefield {
                            let c = &self.cards[id.0 as usize];
                            if c.cur.controller != p || !self.matches_card(filter, id, id, p) {
                                continue;
                            }
                            for col in Color::ALL {
                                if c.cur.colors.has(col) {
                                    push_unique(&mut v, ManaType::of(col));
                                }
                            }
                        }
                    }
                    ManaProduce::AnyColorYouCouldFrom { filter, colorless, .. } => {
                        if depth < 3 {
                            for &id in &self.battlefield {
                                let c = &self.cards[id.0 as usize];
                                if c.cur.controller != p || !self.matches_card(filter, id, id, p) {
                                    continue;
                                }
                                let d = self.db.def(c.def);
                                for &ai in &d.mana_abilities {
                                    if let Ability::Activated { effects, .. } = &d.abilities[ai as usize] {
                                        for t in self.produces_for(p, effects, depth + 1) {
                                            if *colorless || t != ManaType::C {
                                                push_unique(&mut v, t);
                                            }
                                        }
                                    }
                                }
                                // basic land types gained from effects (Urborg: Forest is also a Swamp) carry their intrinsic mana ability
                                for (st, col) in [("Plains", Color::W), ("Island", Color::U), ("Swamp", Color::B), ("Mountain", Color::R), ("Forest", Color::G)] {
                                    if c.cur.types.has(CardType::Land) && c.cur.subtypes.contains(&Sym::new(st)) {
                                        push_unique(&mut v, ManaType::of(col));
                                    }
                                }
                            }
                        }
                    }
                    ManaProduce::AnyTypeOfSacrificed | ManaProduce::AnyTypeOfUntapped => {
                        for c in Color::ALL {
                            push_unique(&mut v, ManaType::of(c));
                        }
                        push_unique(&mut v, ManaType::C);
                    }
                    ManaProduce::AnyColorOpponentsCould => {
                        if depth < 3 {
                            for &id in &self.battlefield {
                                let c = &self.cards[id.0 as usize];
                                if c.cur.controller == p || !c.cur.types.has(CardType::Land) {
                                    continue;
                                }
                                let d = self.db.def(c.def);
                                for &ai in &d.mana_abilities {
                                    if let Ability::Activated { effects, .. } = &d.abilities[ai as usize] {
                                        for t in self.produces_for(c.cur.controller, effects, depth + 1) {
                                            // a replacement of the land's mana type (Contamination) applies to what it "could produce"
                                            push_unique(&mut v, self.replace_mana_type(id, t));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        v
    }

    /// The colours of the cards currently exiled with this permanent ("one mana of any of the exiled cards' colors").
    pub fn exiled_with_colors(&self, card: CardId) -> Vec<ManaType> {
        let zcc = self.cards[card.0 as usize].zcc;
        let mut cols: Vec<ManaType> = vec![];
        for x in self.cards.iter() {
            if x.zone == Zone::Exile && x.exiled_with == Some((card, zcc)) {
                for col in Color::ALL {
                    if self.db.def(x.def).colors.has(col) && !cols.contains(&ManaType::of(col)) {
                        cols.push(ManaType::of(col));
                    }
                }
            }
        }
        cols
    }

    /// The alternatives for what one activation of these mana effects yields.
    /// What the source can make, with mana doublers (Mana Reflection, Lavaleaper) counted so the planner knows about the extra mana.
    pub fn outputs_for(&self, p: PlayerId, source: CardId, effects: &[Effect]) -> Vec<Vec<(ManaType, u8)>> {
        let mut out = self.outputs_for_base(p, source, effects);
        let f = self.mana_multiplier(p, source);
        if f > 1 {
            for alt in out.iter_mut() {
                for (_, n) in alt.iter_mut() {
                    *n = n.saturating_mul(f.min(255) as u8);
                }
            }
        }
        out
    }

    fn outputs_for_base(&self, p: PlayerId, source: CardId, effects: &[Effect]) -> Vec<Vec<(ManaType, u8)>> {
        let mut ctx = crate::eval::EvalCtx::of(self, source);
        // "Exile a creature you control: add mana equal to ...": the planner assumes the best creature (highest mana value) is exiled (Food Chain)
        let exile_best: Vec<Obj> = self
            .def_of(source)
            .abilities
            .iter()
            .filter_map(|a| match a {
                Ability::Activated { cost, effects: e2, .. } if std::ptr::eq(e2.as_ptr(), effects.as_ptr()) => cost.iter().find_map(|c| match c {
                    Cost::ExileFromBattlefield { filter, n: 1 } => self.eligible_permanents(p, filter, source).into_iter().max_by_key(|id| self.def_of(*id).mana_value).map(Obj::card),
                    _ => None,
                }),
                _ => None,
            })
            .collect();
        ctx.remembered = &exile_best;
        // "{T}, remove X counters: add X mana": the planner assumes all of them are removed
        for a in &self.def_of(source).abilities {
            if let Ability::Activated { cost, effects: e2, .. } = a {
                if std::ptr::eq(e2.as_ptr(), effects.as_ptr()) {
                    for part in cost {
                        if let Cost::RemoveCountersX { kind } = part {
                            ctx.x = self.cards[source.0 as usize].counter(*kind);
                        }
                        // "Sacrifice X Goats / tap X tokens: add X mana": planned at the most that can be paid
                        if let Cost::XTimes { cost: inner } = part {
                            ctx.x = match &**inner {
                                Cost::TapPermanents { filter, n } => self.tappable_for_cost(p, source, filter).len() as i32 / (*n).max(1),
                                Cost::Sacrifice { filter, n } => self.count_eligible(&self.sac_cost_filter_check(filter), source, p, Zone::Battlefield) as i32 / (*n).max(1),
                                _ => 0,
                            };
                        }
                    }
                }
            }
        }
        let allowed = self.produces_for(p, effects, 0);
        let mut alts: Vec<Vec<(ManaType, u8)>> = vec![vec![]];
        let add = |alts: &mut Vec<Vec<(ManaType, u8)>>, t: ManaType, n: u8| {
            for a in alts.iter_mut() {
                if let Some(e) = a.iter_mut().find(|(x, _)| *x == t) {
                    e.1 = e.1.saturating_add(n);
                } else {
                    a.push((t, n));
                }
            }
        };
        let choose = |alts: &mut Vec<Vec<(ManaType, u8)>>, n: u8| {
            let colors: Vec<ManaType> = allowed.iter().copied().filter(|t| *t != ManaType::C).collect();
            if colors.is_empty() {
                return;
            }
            let base = std::mem::take(alts);
            for c in colors {
                for a in &base {
                    let mut a2 = a.clone();
                    if let Some(e) = a2.iter_mut().find(|(x, _)| *x == c) {
                        e.1 = e.1.saturating_add(n);
                    } else {
                        a2.push((c, n));
                    }
                    alts.push(a2);
                }
            }
        };
        for e in effects {
            let Effect::AddMana { mana, .. } = e else { continue };
            for m in mana {
                match m {
                    ManaProduce::Fixed { m, n } | ManaProduce::Rider { m, n, .. } => add(&mut alts, *m, *n),
                    ManaProduce::Restricted { m, n, amount, chosen, .. } => {
                        let m = if *chosen { self.cards[source.0 as usize].chosen_color.map_or(*m, ManaType::of) } else { *m };
                        match amount {
                            Some(a) => add(&mut alts, m, self.eval_amount(a, &ctx).clamp(0, 255) as u8),
                            None => add(&mut alts, m, *n),
                        }
                    }
                    ManaProduce::NotedMana { .. } => {
                        let all = self.cards[source.0 as usize].noted_mana_all;
                        for (i, t) in ManaType::ALL.iter().enumerate() {
                            if all[i] > 0 {
                                add(&mut alts, *t, all[i]);
                            }
                        }
                    }
                    ManaProduce::NotedType => {
                        let k = self.cards[source.0 as usize].noted_mana;
                        if k > 0 {
                            add(&mut alts, ManaType::ALL[k as usize - 1], 1)
                        }
                    }
                    ManaProduce::ChosenColor { n } => {
                        if let Some(c) = self.cards[source.0 as usize].chosen_color {
                            add(&mut alts, ManaType::of(c), *n)
                        }
                    }
                    ManaProduce::SecondChosenColor { n } => {
                        if let Some(c) = self.cards[source.0 as usize].chosen_color2 {
                            add(&mut alts, ManaType::of(c), *n)
                        }
                    }
                    ManaProduce::ChosenColorScaled { amount } => {
                        if let Some(c) = self.cards[source.0 as usize].chosen_color {
                            add(&mut alts, ManaType::of(c), self.eval_amount(amount, &ctx).clamp(0, 255) as u8)
                        }
                    }
                    ManaProduce::Scaled { m, amount } => add(&mut alts, *m, self.eval_amount(amount, &ctx).clamp(0, 255) as u8),
                    ManaProduce::DevotionOneColor => {
                        let base = std::mem::take(&mut alts);
                        for c in Color::ALL {
                            let d = self.eval_amount(&Amount::Devotion { colors: vec![c] }, &ctx).clamp(0, 255) as u8;
                            if d == 0 {
                                continue;
                            }
                            for a in &base {
                                let mut a2 = a.clone();
                                if let Some(e) = a2.iter_mut().find(|(x, _)| *x == ManaType::of(c)) {
                                    e.1 = e.1.saturating_add(d);
                                } else {
                                    a2.push((ManaType::of(c), d));
                                }
                                alts.push(a2);
                            }
                        }
                        if alts.is_empty() {
                            alts = base;
                        }
                    }
                    ManaProduce::AnyOneColor { n } | ManaProduce::AnyColor { n } | ManaProduce::AnyColorRider { n, .. } => choose(&mut alts, *n),
                    ManaProduce::OwnColorRestricted { .. } => {
                        let own: Vec<ManaType> = Color::ALL.iter().copied().filter(|c| self.cards[source.0 as usize].cur.colors.has(*c)).map(ManaType::of).collect();
                        if !own.is_empty() {
                            let base = std::mem::take(&mut alts);
                            for c in own {
                                for a in &base {
                                    let mut a2 = a.clone();
                                    if let Some(e) = a2.iter_mut().find(|(x, _)| *x == c) {
                                        e.1 = e.1.saturating_add(1);
                                    } else {
                                        a2.push((c, 1));
                                    }
                                    alts.push(a2);
                                }
                            }
                        }
                    }
                    ManaProduce::RestrictedAny { n, amount, one_color, .. } => {
                        let total = match amount { Some(a) => self.eval_amount(a, &ctx).clamp(0, 255) as u8, None => *n };
                        let colors: Vec<ManaType> = allowed.iter().copied().filter(|t| *t != ManaType::C).collect();
                        // "any combination of colors": each mana may pick its own colour, so every split of the mana over the colours is an alternative
                        let mut splits: Vec<Vec<u8>> = Vec::new();
                        if !*one_color && !colors.is_empty() && unit_splits(total, colors.len(), &mut splits, &mut Vec::new(), 256) {
                            let base = std::mem::take(&mut alts);
                            for sp in &splits {
                                for a in &base {
                                    let mut a2 = a.clone();
                                    for (c, &k) in colors.iter().zip(sp) {
                                        if k == 0 {
                                            continue;
                                        }
                                        if let Some(e) = a2.iter_mut().find(|(x, _)| x == c) {
                                            e.1 = e.1.saturating_add(k);
                                        } else {
                                            a2.push((*c, k));
                                        }
                                    }
                                    alts.push(a2);
                                }
                            }
                        } else {
                            choose(&mut alts, total);
                        }
                    }
                    ManaProduce::AnyOneColorScaled { amount } | ManaProduce::AnyColorScaled { amount, .. } => choose(&mut alts, self.eval_amount(amount, &ctx).clamp(0, 255) as u8),
                    ManaProduce::OneProduced | ManaProduce::CostOf { .. } | ManaProduce::MilledSymbols => {}
                    // only the colours of the cards exiled with this permanent (none: no mana)
                    ManaProduce::AnyColorOfExiledWith => {
                        let cols = self.exiled_with_colors(source);
                        if !cols.is_empty() {
                            let base = std::mem::take(&mut alts);
                            for c in cols {
                                for a in &base {
                                    let mut a2 = a.clone();
                                    if let Some(e) = a2.iter_mut().find(|(x, _)| *x == c) {
                                        e.1 = e.1.saturating_add(1);
                                    } else {
                                        a2.push((c, 1));
                                    }
                                    alts.push(a2);
                                }
                            }
                        }
                    }
                    ManaProduce::AnyColorYouCould | ManaProduce::AnyColorYouCouldFrom { .. } | ManaProduce::AnyColorAmong { .. } | ManaProduce::AnyTypeOfSacrificed | ManaProduce::AnyTypeOfUntapped | ManaProduce::AnyCommanderColor | ManaProduce::AnyCommanderColorRider { .. } | ManaProduce::AnyColorOpponentsCould => choose(&mut alts, 1),
                }
            }
        }
        if self.def_of(source).types.has(CardType::Land) && self.land_mana_capped() {
            for a in alts.iter_mut() {
                if a.iter().map(|(_, n)| *n as u32).sum::<u32>() >= 2 {
                    *a = vec![(ManaType::C, 1)];
                }
            }
        }
        alts
    }

    /// Graveyard cards (biggest mana value first) with total mana value `n` or more, if the graveyard has enough.
    /// Cards in `p`'s hand (other than `card`) matching `filter`.
    pub(crate) fn hand_matching(&self, p: PlayerId, card: CardId, filter: &Filter) -> Vec<CardId> {
        self.players[p as usize].hand.iter().copied().filter(|id| *id != card && self.matches_card(filter, *id, card, p)).collect()
    }

    /// Are there `n` of these cards that share a color?
    pub(crate) fn share_color_group(&self, ids: &[CardId], n: i32) -> bool {
        Color::ALL.iter().any(|c| ids.iter().filter(|id| self.def_of(**id).colors.has(*c)).count() >= n.max(0) as usize)
    }

    /// Controlled permanents matching a sacrifice-cost filter.
    pub(crate) fn sac_pool(&self, filter: &Filter, source: CardId, p: PlayerId) -> Vec<CardId> {
        let mut f = self.sac_cost_filter(filter);
        f.controller = PlayerFilter::Any;
        self.battlefield.iter().copied().filter(|id| self.cards[id.0 as usize].cur.controller == p && self.matches_card(&f, *id, source, p)).collect()
    }

    /// At least `n` of these cards have pairwise different names.
    /// The other cards in `p`'s graveyard (not `card`) that could be exiled for `Cost::ExileGraveyardTypes`.
    pub(crate) fn gy_types_pool(&self, p: PlayerId, card: CardId) -> Vec<CardId> {
        self.players[p as usize].graveyard.iter().copied().filter(|x| *x != card).collect()
    }

    /// The cards in `p`'s graveyard (not `card`) matching `filter` that could be exiled for `Cost::ExileGraveyardMv`.
    pub(crate) fn gy_mv_pool(&self, filter: &Filter, p: PlayerId, card: CardId) -> Vec<CardId> {
        let mut f = filter.clone();
        f.controller = PlayerFilter::Any;
        self.players[p as usize].graveyard.iter().copied().filter(|x| *x != card && self.matches_card(&f, *x, card, p)).collect()
    }

    /// The permanents `p` controls matching `bf` and the cards in `p`'s graveyard matching `gy` that could be exiled for `Cost::ExileCraftMaterials` (not `card` itself).
    pub(crate) fn craft_pool(&self, bf: &Filter, gy: &Filter, p: PlayerId, card: CardId) -> Vec<CardId> {
        let mut out: Vec<CardId> = self.eligible_permanents(p, bf, card).into_iter().filter(|x| *x != card).collect();
        out.extend(self.gy_mv_pool(gy, p, card));
        out
    }

    /// Can `Cost::ExileCraftMaterials` be paid: enough materials, and with `share_type` at least `min` of them share a card type.
    pub(crate) fn craft_ok(&self, bf: &Filter, gy: &Filter, min: u8, share_type: bool, p: PlayerId, card: CardId) -> bool {
        let pool = self.craft_pool(bf, gy, p, card);
        let need = min.max(1) as usize;
        if pool.len() < need {
            return false;
        }
        if share_type {
            return (0..32).any(|b| pool.iter().filter(|id| self.cards[id.0 as usize].cur.types.0 as u64 >> b & 1 == 1).count() >= need);
        }
        true
    }

    /// The other permanents `p` controls matching `filter` that could be exiled for `Cost::ExileTotalMvX`.
    pub(crate) fn exile_mv_pool(&self, filter: &Filter, p: PlayerId, card: CardId) -> Vec<CardId> {
        self.eligible_permanents(p, filter, card).into_iter().filter(|x| *x != card).collect()
    }

    /// Is the total mana value of these cards at least `n`?
    pub(crate) fn gy_mv_ok(&self, ids: &[CardId], n: i32) -> bool {
        ids.iter().map(|id| self.def_of(*id).mana_value as i32).sum::<i32>() >= n
    }

    /// Do these cards have `n` or more card types among them?
    pub(crate) fn gy_types_ok(&self, ids: &[CardId], n: i32) -> bool {
        let mut t = 0u64;
        for id in ids {
            t |= self.cards[id.0 as usize].cur.types.0 as u64;
        }
        t.count_ones() as i32 >= n
    }

    pub(crate) fn distinct_names_ok(&self, ids: &[CardId], n: i32) -> bool {
        let mut names: Vec<&str> = ids.iter().map(|id| self.def_of(*id).name.as_str()).collect();
        names.sort();
        names.dedup();
        names.len() >= n.max(0) as usize
    }

    /// `n` of these cards with the same name, if there are that many.
    pub(crate) fn same_name_group(&self, ids: &[CardId], n: i32) -> Option<Vec<CardId>> {
        let n = n.max(0) as usize;
        for id in ids {
            let nm = &self.def_of(*id).name;
            let grp: Vec<CardId> = ids.iter().copied().filter(|x| &self.def_of(*x).name == nm).collect();
            if grp.len() >= n {
                return Some(grp.into_iter().take(n).collect());
            }
        }
        None
    }

    /// `n` of these creatures that share at least one creature type, if there are that many.
    pub(crate) fn share_type_group(&self, ids: &[CardId], n: i32) -> Option<Vec<CardId>> {
        let n = n.max(0) as usize;
        let mut types: Vec<Sym> = vec![];
        for id in ids {
            for s in self.cards[id.0 as usize].cur.subtypes.iter() {
                if crate::dsl::SubtypeKind::Creature.contains(&s.name()) && !types.contains(s) {
                    types.push(*s);
                }
            }
        }
        for t in types {
            let grp: Vec<CardId> = ids.iter().copied().filter(|id| self.cards[id.0 as usize].cur.subtypes.contains(&t)).collect();
            if grp.len() >= n {
                return Some(grp.into_iter().take(n).collect());
            }
        }
        None
    }

    pub(crate) fn evidence_cards(&self, p: PlayerId, n: i32, exclude: CardId) -> Option<Vec<CardId>> {
        let mut gy: Vec<CardId> = self.players[p as usize].graveyard.iter().copied().filter(|g| *g != exclude).collect();
        gy.sort_by_key(|id| std::cmp::Reverse(self.def_of(*id).mana_value));
        let (mut sum, mut out) = (0i32, vec![]);
        for id in gy {
            if sum >= n {
                break;
            }
            sum += self.def_of(id).mana_value as i32;
            out.push(id);
        }
        (sum >= n).then_some(out)
    }

    /// The opponents of `p` who can be chosen for a cost like "an opponent gains 1 life" (those still in the game).
    pub(crate) fn cost_opponents(&self, p: PlayerId) -> Vec<PlayerId> {
        self.opponents(p).into_iter().filter(|q| self.alive(*q)).collect()
    }

    /// `Cost::OpponentCreatesToken` with no choice to make: the only opponent still in the game creates the tokens.
    pub(crate) fn opponent_creates_tokens(&mut self, p: PlayerId, token: &str, n: i32) {
        if let Some(o) = self.cost_opponents(p).first().copied() {
            self.player_creates_tokens(o, token, n);
        }
    }

    /// Player `o` creates `n` tokens of the named definition (a cost paid by someone else).
    pub(crate) fn player_creates_tokens(&mut self, o: PlayerId, token: &str, n: i32) {
        let Some(def) = self.db.find(token) else { return };
        for _ in 0..n.max(0) {
            if self.token_cap_reached() {
                break;
            }
            let id = self.create_card(def, o, Zone::Exile, true);
            self.players[o as usize].exile.retain(|x| *x != id);
            self.cards[id.0 as usize].zone = Zone::Library;
            self.move_card(id, Zone::Battlefield, crate::zones::MoveOpts { controller: Some(o), ..Default::default() });
            self.emit(Event::Token { card: id });
        }
    }

    /// Cards for `ExileSymbolsFromGraveyard`: most `color` symbols first until `n` symbols are reached.
    pub(crate) fn symbol_cards(&self, p: PlayerId, filter: &Filter, color: Color, n: i32, source: CardId) -> Option<Vec<CardId>> {
        let mut f = filter.clone();
        f.controller = PlayerFilter::Any;
        let sym = |id: CardId| -> i32 {
            self.def_of(id)
                .cost
                .shards
                .iter()
                .filter(|sh| match sh {
                    crate::mana::Shard::Color { c: x } | crate::mana::Shard::Phyrexian { c: x } | crate::mana::Shard::HybridGeneric { c: x } | crate::mana::Shard::HybridColorless { c: x } => *x == color,
                    crate::mana::Shard::Hybrid { a, b } | crate::mana::Shard::PhyrexianHybrid { a, b } => *a == color || *b == color,
                    _ => false,
                })
                .count() as i32
        };
        let mut gy: Vec<CardId> = self.players[p as usize].graveyard.iter().copied().filter(|g| *g != source && self.matches_card(&f, *g, source, p)).collect();
        gy.sort_by_key(|id| std::cmp::Reverse(sym(*id)));
        let (mut sum, mut out) = (0i32, vec![]);
        for id in gy {
            if sum >= n {
                break;
            }
            let s = sym(id);
            if s == 0 {
                break;
            }
            sum += s;
            out.push(id);
        }
        (sum >= n).then_some(out)
    }

    /// The inner cost of an `XTimes` cost paid `k` times.
    pub(crate) fn scale_cost(c: &Cost, k: i32) -> Cost {
        match c {
            Cost::PayLife { n } => Cost::PayLife { n: n * k },
            Cost::TakeDamage { n } => Cost::TakeDamage { n: n * k },
            Cost::Sacrifice { filter, n } => Cost::Sacrifice { filter: filter.clone(), n: n * k },
            Cost::Discard { filter, n, random } => Cost::Discard { filter: filter.clone(), n: n * k, random: *random },
            Cost::ExileFromGraveyard { filter, n, bottom } => Cost::ExileFromGraveyard { filter: filter.clone(), n: n * k, bottom: *bottom },
            Cost::ExileFromHand { filter, n, to_library } => Cost::ExileFromHand { filter: filter.clone(), n: n * k, to_library: *to_library },
            Cost::TapPermanents { filter, n } => Cost::TapPermanents { filter: filter.clone(), n: n * k },
            Cost::ReturnToHand { filter, n } => Cost::ReturnToHand { filter: filter.clone(), n: n * k },
            Cost::ExileFromBattlefield { filter, n } => Cost::ExileFromBattlefield { filter: filter.clone(), n: n * k },
            Cost::RemoveCounters { kind, n } => Cost::RemoveCounters { kind: *kind, n: n * k },
            Cost::RemoveAnyCounters { n } => Cost::RemoveAnyCounters { n: n * k },
            Cost::RemoveAnyCountersFrom { filter, n } => Cost::RemoveAnyCountersFrom { filter: filter.clone(), n: n * k },
            Cost::RemoveCountersFrom { filter, kind, n, among, suspended } => Cost::RemoveCountersFrom { filter: filter.clone(), kind: *kind, n: n * k, among: *among, suspended: *suspended },
            Cost::PutCounters { kind, n } => Cost::PutCounters { kind: *kind, n: n * k },
            // Blight X: all X counters go on ONE chosen permanent
            Cost::PutCountersOnChosen { filter, kind, n, opponent } => Cost::PutCountersOnChosen { filter: filter.clone(), kind: *kind, n: n * k, opponent: *opponent },
            Cost::Mill { n } => Cost::Mill { n: n * k },
            Cost::DrawCards { n } => Cost::DrawCards { n: n * k },
            Cost::ControllerDraws { n } => Cost::ControllerDraws { n: n * k },
            Cost::ExileTop { n } => Cost::ExileTop { n: n * k },
            Cost::OpponentGainsLife { n } => Cost::OpponentGainsLife { n: n * k },
            Cost::EachOtherPlayerGainsLife { n } => Cost::EachOtherPlayerGainsLife { n: n * k },
            Cost::OpponentCreatesToken { token, n } => Cost::OpponentCreatesToken { token: token.clone(), n: n * k },
            Cost::Waterbend { n } => Cost::Waterbend { n: n * k },
            Cost::TapCreaturesOrMana { n } => Cost::TapCreaturesOrMana { n: n * k },
            Cost::Reveal { filter, n } => Cost::Reveal { filter: filter.clone(), n: n * k },
            Cost::PayEnergy { n } => Cost::PayEnergy { n: n * k },
            other => other.clone(),
        }
    }

    /// Can this permanent's cost part be paid right now by `p` (only costs without choices are supported for mana abilities)?
    /// The life a `PayLifeX` cost of `card` costs right now.
    pub(crate) fn life_cost(&self, card: CardId, amount: &Amount) -> i32 {
        let mut ec = crate::eval::EvalCtx::of(self, card);
        ec.x = self.cards[card.0 as usize].x;
        self.eval_amount(amount, &ec).max(0)
    }

    pub(crate) fn simple_cost_ok(&self, p: PlayerId, card: CardId, part: &Cost) -> bool {
        let c = &self.cards[card.0 as usize];
        match part {
            Cost::Tap => !c.tapped && !(c.cur.types.has(CardType::Creature) && !self.can_use_tap_symbol(card)) && self.can_become_tapped(card, false),
            Cost::Untap => c.tapped && self.can_use_tap_symbol(card) && !(c.counter(CounterKind::Stun) > 0 && self.stun_locked(card)),
            Cost::SacrificeSelf => c.zone == Zone::Battlefield && !c.cur.cant_be_sacrificed && !(c.cur.types.has(CardType::Creature) && self.creature_sac_costs_barred()) && !(!c.cur.types.has(CardType::Land) && self.nonland_sac_costs_barred()),
            Cost::PutSelfOnLibraryBottom => c.zone == Zone::Battlefield,
            // a cost that has a player gain life can't be paid when that player can't gain life (CR 119.8: Leyline of Punishment, Everlasting Torment)
            Cost::OpponentGainsLife { .. } => self.opponents(p).iter().any(|q| !(self.effects.iter().any(|e| matches!(e.modif, Modif::NoLifeGain) && e.players.contains(q)) || self.static_cant_gain_life(*q))),
            Cost::PlayerGainsLife { player, .. } => self.alive(*player) && !(self.effects.iter().any(|e| matches!(e.modif, Modif::NoLifeGain) && e.players.contains(player)) || self.static_cant_gain_life(*player)),
            Cost::ExileSelf | Cost::DiscardSelf | Cost::ControllerDraws { .. } | Cost::EachOtherPlayerGainsLife { .. } | Cost::OpponentCreatesToken { .. } | Cost::PlayerCreatesToken { .. } => true,
            Cost::Sacrifice { filter, n } => self.count_eligible(&self.sac_cost_filter_check(filter), card, p, Zone::Battlefield) >= *n as usize,
            Cost::PayLife { n } => self.can_pay_life(p, *n),
            Cost::PayHalfLife => self.can_pay_life(p, (self.players[p as usize].life.max(0) + 1) / 2),
            Cost::PayLifeX { amount } => self.can_pay_life(p, self.life_cost(card, amount)),
            Cost::TakeDamage { .. } | Cost::TakeDamageX { .. } => true,
            Cost::Mana { .. } => false,
            Cost::ReturnToHand { filter, .. } if filter.this => c.zone == Zone::Battlefield,
            Cost::ReturnToHand { filter, n } | Cost::ExileFromBattlefield { filter, n } => self.count_eligible(filter, card, p, Zone::Battlefield) >= *n as usize,
            Cost::RemoveCounters { kind, n } => c.counter(*kind) >= *n,
            Cost::RemoveAnyCounters { n } => c.counters.iter().map(|(_, x)| *x as i32).sum::<i32>() >= *n,
            Cost::RemoveAnyCountersFrom { filter, n } => self.any_counters_removable(filter, *n, card, p),
            Cost::RemoveCountersFrom { filter, kind, n, among, suspended } => self.counters_removable_from(filter, *kind, *n, *among, *suspended, card, p),
            Cost::PutCountersOnChosen { filter, opponent, .. } => if *opponent { !self.counter_targets(p, filter, card, true).is_empty() } else { self.count_eligible(filter, card, p, Zone::Battlefield) >= 1 },
            Cost::TapTotalPower { filter, power } => self.tappable_for_cost(p, card, filter).iter().map(|id| self.cards[id.0 as usize].cur.power.max(0)).sum::<i32>() >= *power,
            Cost::SacrificeTotalPower { filter, power } => self.eligible_permanents(p, filter, card).iter().map(|id| self.cards[id.0 as usize].cur.power.max(0)).sum::<i32>() >= *power,
            Cost::TapShareType { filter, n } => self.share_type_group(&self.tappable_for_cost(p, card, filter), *n).is_some(),
            Cost::SacrificeDistinctNames { filter, n } => self.distinct_names_ok(&self.sac_pool(filter, card, p), *n),
            Cost::DiscardSameName { filter, n } => self.same_name_group(&self.hand_matching(p, card, filter), *n).is_some(),
            Cost::DiscardDistinctNames { filter, n } => self.distinct_names_ok(&self.hand_matching(p, card, filter), *n),
            Cost::RevealShareColor { n } => self.share_color_group(&self.hand_matching(p, card, &Filter::default()), *n),
            Cost::RemoveCountersX { .. } | Cost::RemoveXCountersFromOne { .. } | Cost::RemoveAllCounters { .. } | Cost::XTimes { .. } => true,
            // putting counters on this permanent is always possible (Wall of Roots)
            Cost::PutCounters { .. } => c.zone == Zone::Battlefield,
            Cost::Waterbend { n } | Cost::TapCreaturesOrMana { n } => {
                let tappable = if matches!(part, Cost::Waterbend { .. }) { self.waterbend_sources(p).len() } else { self.creature_bend_sources(p).len() } as i32;
                let mana = self.mana_sources_for_cost(p, None).len() as i32;
                tappable + mana >= *n
            }
            Cost::PayEnergy { n } => self.players[p as usize].energy >= *n,
            Cost::TapPermanents { filter, n } => self.tappable_for_cost(p, card, filter).len() >= *n as usize,
            Cost::UntapPermanents { filter, n, opponent } => self.untappable_for_cost(p, card, filter, *opponent).len() >= *n as usize,
            Cost::Mill { n } | Cost::ExileTop { n } => self.players[p as usize].library.len() >= *n as usize,
            Cost::Reveal { filter, n } => {
                let f = Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() };
                self.players[p as usize].hand.iter().filter(|h| (Some(**h) != self.pay_for.get() || filter.this) && self.matches_card(&f, **h, card, p)).count() >= *n as usize
            }
            Cost::Discard { filter, n, .. } | Cost::ExileFromHand { filter, n, .. } => {
                // the spell being paid for is not in the hand it is paid from
                let own = self.pay_for.get().map_or(0, |sp| {
                    let mut f = filter.clone();
                    f.controller = PlayerFilter::Any;
                    (self.cards[sp.0 as usize].zone == Zone::Hand && self.matches_card(&f, sp, card, p)) as usize
                });
                // "discard a card with mana value X": X isn't chosen yet while checking legality, so don't filter on it
                let fx = strip_x_filter(filter);
                self.count_eligible(&fx, card, p, Zone::Hand).saturating_sub(own) >= *n as usize
            }
            Cost::Or { options } => options.iter().any(|opt| opt.iter().all(|c| matches!(c, Cost::Mana { .. }) || self.simple_cost_ok(p, card, c))),
            Cost::ExileFromGraveyard { filter, n, bottom } => self.count_eligible(filter, card, p, Zone::Graveyard) >= *n as usize,
            Cost::ExileFromGraveyardAmount { filter, amount } => self.count_eligible(filter, card, p, Zone::Graveyard) >= self.life_cost(card, amount) as usize,
            Cost::PutExiledIntoGraveyard { filter } => self.count_eligible(filter, card, p, Zone::Exile) >= 1,
            Cost::ExileSpellFromStack { filter } => !self.stack_spells_for_cost(p, card, filter).is_empty(),
            Cost::ExileFromSingleGraveyard { filter, n, .. } => !self.single_graveyard_pools(filter, card, p, *n).is_empty(),
            Cost::ExileGraveyardTypes { n } => self.gy_types_ok(&self.gy_types_pool(p, card), *n),
            Cost::ExileTotalMvX { filter } => !self.exile_mv_pool(filter, p, card).is_empty(),
            Cost::ExileGraveyardMv { filter, n } => self.gy_mv_ok(&self.gy_mv_pool(filter, p, card), *n),
            Cost::ExileCraftMaterials { bf, gy, min, share_type } => self.craft_ok(bf, gy, *min, *share_type, p, card),
            Cost::CollectEvidence { n } => self.evidence_cards(p, *n, card).is_some(),
            Cost::CollectEvidenceTargets => true,
            Cost::ExileSymbolsFromGraveyard { filter, color, n } => self.symbol_cards(p, filter, *color, *n, card).is_some(),
            _ => false,
        }
    }

    /// Untapped permanents `p` controls that match `filter` (tapping them as a cost isn't a {T} ability: summoning sickness is irrelevant).
    pub(crate) fn untappable_for_cost(&self, p: PlayerId, source: CardId, filter: &Filter, opponent: bool) -> Vec<CardId> {
        let mut f = filter.clone();
        f.controller = PlayerFilter::Any;
        self.battlefield
            .iter()
            .copied()
            .filter(|id| (self.cards[id.0 as usize].cur.controller == p) != opponent && self.cards[id.0 as usize].tapped && self.matches_card(&f, *id, source, p))
            .collect()
    }

    pub(crate) fn tappable_for_cost(&self, p: PlayerId, source: CardId, filter: &Filter) -> Vec<CardId> {
        let mut f = filter.clone();
        f.controller = PlayerFilter::Any;
        self.battlefield
            .iter()
            .copied()
            .filter(|id| self.cards[id.0 as usize].cur.controller == p && !self.cards[id.0 as usize].tapped && self.can_become_tapped(*id, false) && self.matches_card(&f, *id, source, p))
            .collect()
    }

    /// Summoning sickness: a creature's {T} abilities (and attacking) need it to have been under its controller's
    /// control continuously since the start of that player's most recent turn.
    pub fn can_use_tap_symbol(&self, card: CardId) -> bool {
        self.can_attack_unsick(card) || self.activates_as_though_haste(card)
    }

    /// No summoning sickness for attacking: not a creature, haste, or controlled since the start of its controller's turn.
    /// ("As though it had haste" for activated abilities, Thousand-Year Elixir, does not let it attack.)
    pub fn can_attack_unsick(&self, card: CardId) -> bool {
        let c = &self.cards[card.0 as usize];
        !c.cur.types.has(CardType::Creature) || c.cur.kw.has(Keyword::Haste) || c.since.max(c.control_turn) < self.players[c.cur.controller as usize].last_turn_start
    }

    fn activates_as_though_haste(&self, card: CardId) -> bool {
        let c = &self.cards[card.0 as usize];
        self.battlefield.iter().any(|&sid| {
            let sc = &self.cards[sid.0 as usize];
            if sc.cur.no_abilities {
                return false;
            }
            let sd = self.db.def(sc.def);
            sd.statics.iter().any(|&i| matches!(&sd.abilities[i as usize], Ability::Static { effect: StaticEffect::ActivateAsThoughHaste { affects }, condition, .. }
                if self.matches_card(affects, card, sid, sc.cur.controller) && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &crate::eval::EvalCtx::with_controller(sid, sc.cur.controller)))))
        })
    }

    /// Battlefield permanents controlled by `p` with an activatable mana ability right now.
    /// Product of the `ManaMultiplier` statics of permanents `p` controls.
    pub fn mana_multiplier(&self, p: PlayerId, src: CardId) -> i32 {
        let mut f = 1;
        let sd = self.def_of(src);
        let basic = sd.types.has(CardType::Land) && sd.supertypes.contains(&Supertype::Basic);
        for &id in &self.battlefield {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                continue;
            }
            let d = self.db.def(c.def);
            for &i in &d.statics {
                if let Ability::Static { effect: StaticEffect::ManaMultiplier { factor, basic_land, all_players }, .. } = &d.abilities[i as usize] {
                    if (c.cur.controller == p || *all_players) && (!*basic_land || basic) {
                        f *= (*factor).max(1);
                    }
                }
            }
        }
        f
    }

    /// The mana sources of a player, in the order the payment search should prefer them.
    /// Does this permanent make "any colour" mana with a "copy that spell" rider (Primal Wellspring)? The copy is made even if the spell is countered by then.
    pub(crate) fn makes_any_color_copy_rider(&self, card: CardId) -> bool {
        self.def_of(card).abilities.iter().any(|a| match a {
            Ability::Activated { effects, .. } => effects.iter().any(|e| matches!(e, Effect::AddMana { mana, .. } if mana.iter().any(|m| matches!(m, ManaProduce::AnyColorRider { rider: crate::dsl::ManaRider::CopySpell, .. })))),
            _ => false,
        })
    }

    pub fn mana_sources_for_payment(&self, p: PlayerId) -> Vec<ManaSourceOpt> {
        self.mana_sources_for_cost(p, None)
    }

    /// Like `mana_sources_for_payment`; `paying` is the cost being paid right now (its colored symbols count as demand too).
    pub fn mana_sources_for_cost(&self, p: PlayerId, paying: Option<&ManaCost>) -> Vec<ManaSourceOpt> {
        let had = self.pay_cost_has_c.replace(paying.map_or(false, |c| c.shards.iter().any(|s| matches!(s, Shard::Colorless | Shard::HybridColorless { .. }))));
        let mut sources = self.mana_sources(p);
        self.pay_cost_has_c.set(had);
        if !self.sac_reserve.borrow().is_empty() {
            let r = self.sac_reserve.borrow();
            sources.retain(|m| !r.contains(&m.card));
        }
        // among otherwise equal sources, spend the ones whose colors the player's hand and abilities need least,
        // so the colors still wanted later stay untapped (the sort is stable, so the search tries them in this order)
        let mut demand = [0u32; 6];
        let mut count = |mc: &ManaCost| {
            for sh in &mc.shards {
                if let Shard::Color { c } = sh {
                    demand[ManaType::of(*c) as usize] += 1;
                }
            }
        };
        if let Some(c) = paying {
            count(c);
        }
        for id in &self.players[p as usize].hand {
            count(&self.def_of(*id).cost);
        }
        for id in &self.battlefield {
            if self.cards[id.0 as usize].cur.controller != p {
                continue;
            }
            for ab in &self.def_of(*id).abilities {
                if let Ability::Activated { cost, .. } = ab {
                    for part in cost {
                        if let Cost::Mana { cost: m } = part {
                            if let Ok(mc) = ManaCost::parse(m) {
                                // the ability being paid for right now is already counted through `paying`
                                if paying.map_or(false, |pc| *pc == mc) {
                                    continue;
                                }
                                count(&mc);
                            }
                        }
                    }
                }
            }
        }
        let mut supply = [0i32; 6];
        for src in &sources {
            for t in &src.produces {
                supply[*t as usize] += 1;
            }
        }
        // sources of a color nobody still needs are spent first; among those the single-color ones (a multi-color land
        // keeps its choice), then the color with the most spare sources (supply minus demand)
        let mut entries: std::collections::HashMap<CardId, usize> = std::collections::HashMap::new();
        for src in &sources {
            *entries.entry(src.card).or_insert(0) += 1;
        }
        // floating mana with a rider that applies to the spell being paid for (Generator Servant, Arena of Glory: haste) goes
        // first, so the spell gets the rider rather than the mana being left over
        let has_rider = |src: &ManaSourceOpt| {
            (crate::cast::RESTRICTED_BASE..crate::cast::INTRINSIC_BASE).contains(&src.ability)
                && match self.players[p as usize].restricted.get((src.ability - crate::cast::RESTRICTED_BASE) as usize) {
                    Some(Some((_, r, _))) if r.rider.is_some() => match (&r.rider_spell, self.pay_for.get()) {
                        (None, _) => true,
                        (Some(f), Some(sp)) => self.matches_card(f, sp, src.card, p),
                        (Some(_), None) => false,
                    },
                    _ => false,
                }
        };
        sources.sort_by_key(|src| {
            let spare = src.produces.iter().map(|t| supply[*t as usize] - demand[*t as usize] as i32).max().unwrap_or(0);
            (!has_rider(src), spare <= 0, entries.get(&src.card).copied().unwrap_or(1), std::cmp::Reverse(spare))
        });
        // Channel: pay 1 life for {C}; tried last so lands are tapped first
        if let Some((t, src)) = self.players[p as usize].life_mana {
            if t == self.turn + 1 {
                let k = (self.players[p as usize].life - 1).clamp(0, 20) as u16;
                for i in 0..k {
                    sources.push(ManaSourceOpt { card: src, ability: crate::cast::LIFE_MANA_BASE + i, produces: vec![ManaType::C], outputs: vec![vec![(ManaType::C, 1)]], cost: String::new(), restrict: None, snow: false });
                }
            }
        }
        sources
    }

    /// Piracy: `p` may tap this land, which they don't control, for mana this turn.
    pub fn foreign_land_ok(&self, p: PlayerId, id: CardId) -> bool {
        let c = &self.cards[id.0 as usize];
        self.players[p as usize].foreign_lands_turn == self.turn + 1 && c.zone == Zone::Battlefield && c.cur.types.has(CardType::Land) && c.cur.controller != p
    }

    pub fn mana_sources(&self, p: PlayerId) -> Vec<ManaSourceOpt> {
        let mut out = vec![];
        // permanents whose mana abilities can't be activated (Collector Ouphe, Detainment Spell)
        let mut blockers: Vec<(CardId, Filter)> = vec![];
        let mut tap_blockers: Vec<(CardId, Filter)> = vec![];
        for &sid in &self.battlefield {
            let sd = self.db.def(self.cards[sid.0 as usize].def);
            for &i in &sd.statics {
                if let Ability::Static { effect: StaticEffect::CantActivate { affects, mana: true, tap_only, .. }, condition, .. } = &sd.abilities[i as usize] {
                    if condition.as_ref().map_or(false, |cd| !self.eval_cond(cd, &crate::eval::EvalCtx::with_controller(sid, self.cards[sid.0 as usize].cur.controller))) {
                        continue;
                    }
                    if *tap_only {
                        tap_blockers.push((sid, affects.clone().into_filter()));
                    } else {
                        blockers.push((sid, affects.clone().into_filter()));
                    }
                }
            }
        }
        // mana abilities shut off by a resolved effect ("its activated abilities can't be activated this turn")
        let effect_blocked: Vec<CardId> = self
            .effects
            .iter()
            .filter_map(|e| match &e.modif {
                Modif::Static(b) => match &**b {
                    StaticEffect::CantActivate { mana: true, .. } => Some(e.affects.iter().map(|(id, _)| *id).collect::<Vec<_>>()),
                    _ => None,
                },
                _ => None,
            })
            .flatten()
            .collect();
        // the same granted with `dynamic` (matched by its filter, also permanents that arrive later)
        for e in &self.effects {
            if let Modif::DynStatic(b) = &e.modif {
                if let StaticEffect::CantActivate { affects, mana: true, tap_only, .. } = &**b {
                    if *tap_only {
                        tap_blockers.push((e.source, affects.clone().into_filter()));
                    } else {
                        blockers.push((e.source, affects.clone().into_filter()));
                    }
                }
            }
        }
        for &id in &self.battlefield {
            let c = &self.cards[id.0 as usize];
            // "any player may activate" mana abilities (Mana Cache) work for every player; their conditions are judged for the activator
            let any_pl = c.cur.controller != p && self.db.def(c.def).any_player;
            // Piracy: lands the player doesn't control may be tapped for mana that only casts spells
            let foreign = c.cur.controller != p && !any_pl && self.foreign_land_ok(p, id);
            if c.cur.controller != p && !any_pl && !foreign {
                continue;
            }
            if effect_blocked.contains(&id) {
                continue;
            }
            if c.detained_by.is_some() || blockers.iter().any(|(sid, f)| self.matches_card(f, id, *sid, self.cards[sid.0 as usize].cur.controller)) {
                continue;
            }
            let tap_blocked = tap_blockers.iter().any(|(sid, f)| self.matches_card(f, id, *sid, self.cards[sid.0 as usize].cur.controller)) || !self.can_become_tapped(id, false);
            let noab = c.cur.no_abilities;
            let d = self.db.def(c.def);
            // intrinsic mana abilities of basic land types the permanent has (Dryad of the Ilysian Grove, Urborg ...)
            if c.cur.types.has(CardType::Land) && !c.tapped && !tap_blocked && !c.cur.no_land_mana {
                for (ci, name) in ["Plains", "Island", "Swamp", "Mountain", "Forest"].iter().enumerate() {
                    if !c.cur.subtypes.contains(&Sym::new(name)) {
                        continue;
                    }
                    let t = ManaType::of(Color::ALL[ci]);
                    let own = !noab && d.mana_abilities.iter().any(|&ai| matches!(&d.abilities[ai as usize], Ability::Activated { effects, .. } if self.produces_for(p, effects, 0).contains(&t)));
                    // a land that lost its abilities but was given the type back by another effect keeps the rule-given ability,
                    // unless something already grants the same mana (Blood Moon style definitions)
                    let granted_same = noab && c.cur.granted.iter().any(|a| matches!(&**a, Ability::Activated { effects, .. } if crate::carddb::is_mana_ability(a) && self.produces_for(p, effects, 0).contains(&t)));
                    if !own && !granted_same {
                        out.push(ManaSourceOpt { card: id, ability: crate::cast::INTRINSIC_BASE + ci as u16, produces: vec![t], outputs: vec![vec![(t, 1)]], cost: String::new(), restrict: None, snow: c.cur.supertypes.contains(&Supertype::Snow) });
                    }
                }
            }
            // abilities granted by other permanents (Chromatic Lantern)
            for (k, a) in c.cur.granted.iter().enumerate() {
                if !crate::carddb::is_mana_ability(a) {
                    continue;
                }
                let Ability::Activated { cost, effects, condition, .. } = &**a else { continue };
                if tap_blocked && cost.iter().any(|p| matches!(p, Cost::Tap)) {
                    continue;
                }
                if let Some(cond) = condition {
                    if !self.eval_cond(cond, &crate::eval::EvalCtx::of(self, id)) {
                        continue;
                    }
                }
                let mut extra = ManaCost::default();
                let mut ok = true;
                for part in cost {
                    match part {
                        Cost::Mana { cost: m } => match ManaCost::parse(m) {
                            Ok(mc) => extra.shards.extend(mc.shards),
                            Err(_) => ok = false,
                        },
                        other => ok &= self.simple_cost_ok(p, id, other),
                    }
                }
                let produces = self.produces_for(p, effects, 0);
                if ok && !produces.is_empty() {
                    out.push(ManaSourceOpt { card: id, ability: crate::cast::GRANTED_BASE + k as u16, produces, outputs: self.outputs_for(p, id, effects), cost: extra.display(), restrict: restricted_filter(effects), snow: c.cur.supertypes.contains(&Supertype::Snow) });
                }
            }
            // a basic land whose basic land type was replaced (Elsewhere Flask) no longer has the mana ability that type gave it
            let lost_basic: Vec<ManaType> = if d.supertypes.contains(&Supertype::Basic) && c.cur.types.has(CardType::Land) {
                ["Plains", "Island", "Swamp", "Mountain", "Forest"].iter().enumerate().filter(|(_, n)| d.subtypes.contains(&Sym::new(n)) && !c.cur.subtypes.contains(&Sym::new(n))).map(|(ci, _)| ManaType::of(Color::ALL[ci])).collect()
            } else {
                vec![]
            };
            for &ai in &d.mana_abilities {
                if noab && !c.cur.keep_mana_abilities {
                    break;
                }
                let Ability::Activated { cost, effects, condition, from, .. } = &d.abilities[ai as usize] else { continue };
                if !lost_basic.is_empty() {
                    let pr = self.produces_for(p, effects, 0);
                    if pr.len() == 1 && lost_basic.contains(&pr[0]) {
                        continue;
                    }
                }
                if tap_blocked && cost.iter().any(|p| matches!(p, Cost::Tap)) {
                    continue;
                }
                // abilities that only work from the hand / graveyard don't work on the battlefield
                if from.map_or(false, |z| z != Zone::Battlefield) {
                    continue;
                }
                if let Some(cond) = condition {
                    let mut ec = crate::eval::EvalCtx::of(self, id);
                    if any_pl {
                        ec.controller = p;
                    }
                    if !self.eval_cond(cond, &ec) {
                        continue;
                    }
                }
                let mut extra = ManaCost::default();
                let mut ok = true;
                for part in cost {
                    match part {
                        Cost::Mana { cost: m } => match ManaCost::parse(m) {
                            Ok(mc) => extra.shards.extend(mc.shards),
                            Err(_) => ok = false,
                        },
                        other => ok &= self.simple_cost_ok(p, id, other),
                    }
                }
                if ok {
                    let produces = self.produces_for(p, effects, 0);
                    if !produces.is_empty() {
                        let outputs = self.outputs_for(p, id, effects);
                        out.push(ManaSourceOpt { card: id, ability: ai, produces, outputs, cost: extra.display(), restrict: restricted_filter(effects), snow: c.cur.supertypes.contains(&Supertype::Snow) });
                    }
                }
            }
        }
        // "Plains produce {R} instead of any other type" (Reality Twist, Contamination): plan with the replaced mana types
        let replacing = self.effects.iter().any(|e| matches!(&e.modif, Modif::Static(se) | Modif::DynStatic(se) if matches!(&**se, StaticEffect::ManaTypeReplace { any_color: false, single: false, .. })))
            || self.battlefield.iter().any(|&id| {
                let d = self.db.def(self.cards[id.0 as usize].def);
                d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::ManaTypeReplace { any_color: false, single: false, .. }, .. }))
            });
        if replacing {
            for s in out.iter_mut() {
                if self.cards[s.card.0 as usize].zone != Zone::Battlefield {
                    continue;
                }
                let card = s.card;
                let mut produces: Vec<ManaType> = vec![];
                for t in s.produces.iter() {
                    push_unique(&mut produces, self.replace_mana_type(card, *t));
                }
                s.produces = produces;
                for o in s.outputs.iter_mut() {
                    for (t, _) in o.iter_mut() {
                        *t = self.replace_mana_type(card, *t);
                    }
                }
            }
        }
        // mana from lands tapped under Piracy can be spent only to cast spells
        for s in out.iter_mut() {
            let c = &self.cards[s.card.0 as usize];
            if c.zone == Zone::Battlefield && c.cur.controller != p && !self.db.def(c.def).any_player && s.restrict.is_none() && self.foreign_land_ok(p, s.card) {
                s.restrict = Some(ManaRestrict { spell: Some((Filter::default()).into()), ability: None, rider: None, cost_c: false, upkeep: false, unlock: false, no_generic: false, rider_spell: None });
            }
        }
        // mana abilities that work from the hand (Simian Spirit Guide: exile it from your hand)
        for &id in &self.players[p as usize].hand {
            let d = self.db.def(self.cards[id.0 as usize].def);
            for &ai in &d.mana_abilities {
                let Ability::Activated { cost, effects, from: Some(Zone::Hand), .. } = &d.abilities[ai as usize] else { continue };
                let simple = cost.iter().all(|c| matches!(c, Cost::ExileSelf | Cost::DiscardSelf));
                let produces = self.produces_for(p, effects, 0);
                if simple && !produces.is_empty() {
                    out.push(ManaSourceOpt { card: id, ability: ai, produces, outputs: self.outputs_for(p, id, effects), cost: String::new(), restrict: restricted_filter(effects), snow: false });
                }
            }
        }
        // restricted mana floating in the pool (added by a spell or trigger) as one-mana virtual sources
        let mut virt: Vec<ManaSourceOpt> = vec![];
        if let Some(sp) = self.pay_for.get() {
            for (i, e) in self.players[p as usize].restricted.iter().enumerate() {
                let Some((t, r, src)) = e else { continue };
                let f = if self.pay_is_ability.get() { &r.ability } else { &r.spell };
                if self.pay_is_ability.get() && self.cards[sp.0 as usize].zone != Zone::Battlefield && !f.as_ref().map_or(false, |f| f.types.contains(&CardType::Artifact)) && !(self.pay_is_foretell.get() && f.as_ref().map_or(false, |f| f.foretell_ability)) && !f.as_ref().map_or(false, |f| f.any_object || **f == crate::dsl::Filter::default()) {
                    continue;
                }
                if r.cost_c && self.pay_cost_has_c.get() {
                    virt.push(ManaSourceOpt { card: *src, ability: crate::cast::RESTRICTED_BASE + i as u16, produces: vec![*t], outputs: vec![vec![(*t, 1)]], cost: String::new(), restrict: None, snow: false });
                    continue;
                }
                if r.no_generic {
                    virt.push(ManaSourceOpt { card: *src, ability: crate::cast::RESTRICTED_BASE + i as u16, produces: vec![*t], outputs: vec![vec![(*t, 1)]], cost: String::new(), restrict: Some(r.clone()), snow: false });
                    continue;
                }
                if f.as_ref().map_or(false, |f| self.matches_card(f, sp, *src, p) && (!f.equip_ability || !self.pay_is_ability.get() || self.pay_is_equip.get()) && (!f.power_up_ability || (self.pay_is_ability.get() && self.pay_is_power_up.get())) && (!f.foretell_ability || (self.pay_is_ability.get() && self.pay_is_foretell.get()))) {
                    virt.push(ManaSourceOpt { card: *src, ability: crate::cast::RESTRICTED_BASE + i as u16, produces: vec![*t], outputs: vec![vec![(*t, 1)]], cost: String::new(), restrict: None, snow: false });
                }
            }
        }
        // restricted mana that may also pay to unlock a Room door (Smoky Lounge)
        if self.pay_for.get().is_none() && self.pay_unlock.get() {
            for (i, e) in self.players[p as usize].restricted.iter().enumerate() {
                let Some((t, r, src)) = e else { continue };
                if r.unlock {
                    virt.push(ManaSourceOpt { card: *src, ability: crate::cast::RESTRICTED_BASE + i as u16, produces: vec![*t], outputs: vec![vec![(*t, 1)]], cost: String::new(), restrict: None, snow: false });
                }
            }
        }
        // restricted mana that may only pay a cumulative upkeep cost (Snowfall)
        if self.pay_for.get().is_none() && self.pay_upkeep.get() {
            for (i, e) in self.players[p as usize].restricted.iter().enumerate() {
                let Some((t, r, src)) = e else { continue };
                if r.upkeep {
                    virt.push(ManaSourceOpt { card: *src, ability: crate::cast::RESTRICTED_BASE + i as u16, produces: vec![*t], outputs: vec![vec![(*t, 1)]], cost: String::new(), restrict: None, snow: false });
                }
            }
        }
        // restricted mana only counts for the spell being paid for
        let spell = self.pay_for.get();
        out.retain(|s| match &s.restrict {
            None => true,
            Some(r) if r.no_generic => true,
            Some(r) if r.cost_c && self.pay_cost_has_c.get() => true,
            // Adarkar Unicorn: a mana ability whose mana may only pay a cumulative upkeep cost
            Some(r) if r.upkeep && spell.is_none() => self.pay_upkeep.get(),
            // Sunken Palace: mana for a spell or an ability whose rider copies it may be made ahead of time and float
            Some(r) if spell.is_none() && r.rider == Some(crate::dsl::ManaRider::CopySpell) && r.spell.is_some() && r.ability.is_some() => true,
            // Creeping Peeper: a mana ability whose mana may also unlock a Room door
            Some(r) if r.unlock && spell.is_none() && self.pay_unlock.get() => true,
            Some(r) => spell.map_or(false, |sp| {
                let f = if self.pay_is_ability.get() { &r.ability } else { &r.spell };
                // abilities: only those of permanents on the battlefield (not cycling / channel from hand)
                if self.pay_is_ability.get() && self.cards[sp.0 as usize].zone != Zone::Battlefield && !f.as_ref().map_or(false, |f| f.types.contains(&CardType::Artifact)) && !(self.pay_is_foretell.get() && f.as_ref().map_or(false, |f| f.foretell_ability)) && !f.as_ref().map_or(false, |f| f.any_object || **f == crate::dsl::Filter::default()) {
                    return false;
                }
                f.as_ref().map_or(false, |f| self.matches_card(f, sp, s.card, p) && (!f.equip_ability || !self.pay_is_ability.get() || self.pay_is_equip.get()) && (!f.power_up_ability || (self.pay_is_ability.get() && self.pay_is_power_up.get())) && (!f.foretell_ability || (self.pay_is_ability.get() && self.pay_is_foretell.get())))
            }),
        });
        // Harvest Mage: a land tapped for mana produces one mana of a color of its controller's choice
        for s in out.iter_mut() {
            if s.restrict.is_none() && s.cost.is_empty() && self.mana_any_color(s.card) {
                s.produces = Color::ALL.iter().map(|c| ManaType::of(*c)).collect();
                s.outputs = s.produces.iter().map(|t| vec![(*t, 1)]).collect();
            }
        }
        // False Dawn: sources of colored mana add white instead
        if self.colored_mana_becomes_white(p) {
            for s in out.iter_mut() {
                if s.restrict.is_none() && s.cost.is_empty() {
                    let w = |t: ManaType| if t == ManaType::C { t } else { ManaType::W };
                    s.produces = s.produces.iter().map(|t| w(*t)).fold(vec![], |mut v, t| { if !v.contains(&t) { v.push(t) } v });
                    let mut outs: Vec<Vec<(ManaType, u8)>> = vec![];
                    for o in s.outputs.iter() {
                        let mut m: Vec<(ManaType, u8)> = vec![];
                        for (t, n) in o {
                            let t = w(*t);
                            match m.iter_mut().find(|(x, _)| *x == t) { Some(e) => e.1 += *n, None => m.push((t, *n)) }
                        }
                        if !outs.contains(&m) {
                            outs.push(m);
                        }
                    }
                    s.outputs = outs;
                }
            }
        }
        // "Spend only mana produced by creatures to cast this spell"
        if let Some(f) = self.spend_only_filter() {
            let sp = self.pay_for.get().unwrap();
            // mana made by abilities of cards that aren't on the battlefield (Elvish Spirit Guide from hand) is not "produced by a creature"
            out.retain(|s| self.cards[s.card.0 as usize].zone == Zone::Battlefield && self.matches_card(&f, s.card, sp, p));
            virt.retain(|s| self.matches_card(&f, s.card, sp, p));
        }
        out.extend(virt);
        // Word of Command: the controlled player can activate mana abilities only of lands
        if self.land_mana_only == Some(p) {
            out.retain(|s| self.cards[s.card.0 as usize].cur.types.has(CardType::Land));
        }
        out
    }

    /// The `SpendOnlyManaFrom` filter of the spell currently being paid for, if it has one.
    pub fn spend_only_filter(&self) -> Option<FilterBox> {
        let sp = self.pay_for.get()?;
        if self.pay_is_ability.get() {
            return None;
        }
        self.spend_only_static(sp)
    }

    /// The `SpendOnlyManaFrom` filter of this spell card that applies now (its condition, e.g. `CastWithAlt`, holds).
    pub fn spend_only_static(&self, sp: CardId) -> Option<FilterBox> {
        let d = self.db.def(self.cards[sp.0 as usize].def);
        d.statics.iter().find_map(|&i| match &d.abilities[i as usize] {
            Ability::Static { effect: StaticEffect::SpendOnlyManaFrom { filter }, condition, .. }
                if condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &crate::eval::EvalCtx::of(self, sp))) =>
            {
                Some(filter.clone())
            }
            _ => None,
        })
    }

    /// Does a `ColoredManaBecomesWhite` static (a permanent's own, or granted until end of turn) apply to mana `p` controls adding?
    pub fn colored_mana_becomes_white(&self, p: PlayerId) -> bool {
        if self.effects.iter().any(|e| e.controller == p && matches!(&e.modif, Modif::DynStatic(se) if matches!(**se, StaticEffect::ColoredManaBecomesWhite))) {
            return true;
        }
        self.battlefield.iter().any(|&id| {
            let c = &self.cards[id.0 as usize];
            if c.cur.controller != p || c.cur.no_abilities {
                return false;
            }
            let d = self.db.def(c.def);
            d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::ColoredManaBecomesWhite, condition, .. } if condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &crate::eval::EvalCtx::with_controller(id, p)))))
        })
    }

    /// Mana of a color added to `p`'s pool since `before` (a snapshot of the main pool) becomes white when `ColoredManaBecomesWhite` applies; also in the until-end-of-turn / combat pools.
    pub fn convert_added_mana_to_white(&mut self, p: PlayerId, before: crate::mana::ManaPool) {
        if !self.colored_mana_becomes_white(p) {
            return;
        }
        let pl = &mut self.players[p as usize];
        for t in [ManaType::U, ManaType::B, ManaType::R, ManaType::G] {
            let d = pl.pool.get(t).saturating_sub(before.get(t));
            if d > 0 {
                pl.pool.0[t as usize] -= d;
                pl.pool.add(ManaType::W, d);
                for x in [&mut pl.pool_turn, &mut pl.pool_keep] {
                    let k = x.get(t).min(d);
                    if k > 0 {
                        x.0[t as usize] -= k;
                        x.add(ManaType::W, k);
                    }
                }
            }
        }
        for t in self.last_tapped_mana.iter_mut() {
            if *t != ManaType::C {
                *t = ManaType::W;
            }
        }
    }

    /// `cost` with the `SpendManaAsColor` statics `p` has applied: a {to} symbol becomes {to/from}, {to/P} becomes {to/from/P}.
    pub fn spend_as_cost(&self, p: PlayerId, cost: &ManaCost) -> ManaCost {
        let mut out = cost.clone();
        // granted for a while (`GrantStatic` dynamic): "until end of turn, you may spend white mana as though it were mana of any color"
        for e in &self.effects {
            if let (true, Modif::DynStatic(se)) = (e.controller == p, &e.modif) {
                if let StaticEffect::SpendManaAsColor { from, to } = &**se {
                    for s in out.shards.iter_mut() {
                        match *s {
                            Shard::Color { c } if c == *to => *s = Shard::Hybrid { a: *to, b: *from },
                            Shard::Phyrexian { c } if c == *to => *s = Shard::PhyrexianHybrid { a: *to, b: *from },
                            _ => {}
                        }
                    }
                }
            }
        }
        for &src in &self.battlefield {
            let sc = &self.cards[src.0 as usize];
            if sc.cur.no_abilities || sc.cur.controller != p {
                continue;
            }
            let sd = self.db.def(sc.def);
            for &i in &sd.statics {
                if let Ability::Static { effect: StaticEffect::PayLifeForMana { color }, condition, .. } = &sd.abilities[i as usize] {
                    if !condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &crate::eval::EvalCtx::with_controller(src, p))) {
                        continue;
                    }
                    for s in out.shards.iter_mut() {
                        match *s {
                            Shard::Color { c } if c == *color => *s = Shard::Phyrexian { c },
                            Shard::Hybrid { a, b } if a == *color || b == *color => *s = Shard::PhyrexianHybrid { a, b },
                            _ => {}
                        }
                    }
                }
                if let Ability::Static { effect: StaticEffect::SpendManaAsColor { from, to }, condition, .. } = &sd.abilities[i as usize] {
                    if !condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &crate::eval::EvalCtx::with_controller(src, p))) {
                        continue;
                    }
                    for s in out.shards.iter_mut() {
                        match *s {
                            Shard::Color { c } if c == *to => *s = Shard::Hybrid { a: *to, b: *from },
                            Shard::Phyrexian { c } if c == *to => *s = Shard::PhyrexianHybrid { a: *to, b: *from },
                            _ => {}
                        }
                    }
                }
            }
        }
        for &src in &self.battlefield {
            let sc = &self.cards[src.0 as usize];
            if sc.cur.no_abilities || sc.cur.controller != p {
                continue;
            }
            let sd = self.db.def(sc.def);
            if sd.statics.iter().any(|&i| matches!(&sd.abilities[i as usize], Ability::Static { effect: StaticEffect::SpendWhiteAsAnyOtherAsColorless, .. })) {
                out.white_any = true;
                break;
            }
        }
        // Draugr Necromancer: snow sources' mana pays the coloured symbols of the spell as though it were any color
        if let (Some(sp), false) = (self.pay_for.get(), self.pay_is_ability.get()) {
            if out.shards.iter().any(|s| matches!(s, Shard::Color { .. })) {
                for &src in &self.battlefield {
                    let sc = &self.cards[src.0 as usize];
                    if sc.cur.no_abilities {
                        continue;
                    }
                    let sd = self.db.def(sc.def);
                    let ctrl = sc.cur.controller;
                    if sd.statics.iter().any(|&i| {
                        matches!(&sd.abilities[i as usize], Ability::Static { effect: StaticEffect::SpendSnowManaAsAnyColor { filter, whose }, condition, .. }
                            if self.player_filter_src(src, *whose, p, ctrl)
                                && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &crate::eval::EvalCtx::with_controller(src, ctrl)))
                                && self.matches_card(&Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() }, sp, src, ctrl))
                    }) {
                        out.snow_any = true;
                        break;
                    }
                }
            }
        }
        out
    }

    /// Plan how to pay `cost` with the pool plus these sources (cheapest assignment found by search).
    pub fn plan_payment(&self, p: PlayerId, cost: &ManaCost, x: u32, pool: ManaPool, can_pay_life: bool) -> Option<Plan> {
        let cost = &self.spend_as_cost(p, cost);
        let sources = self.mana_sources_for_cost(p, Some(cost));
        let pool = if self.spend_only_filter().is_some() { ManaPool::default() } else { pool };
        plan_payment_with(&sources, cost, x, pool, if can_pay_life && self.can_pay_life(p, 1) { self.players[p as usize].life } else { 0 })
    }

    pub fn can_afford(&self, p: PlayerId, cost: &ManaCost, x: u32) -> bool {
        self.plan_payment(p, cost, x, self.players[p as usize].pool, true).is_some()
    }

    /// The additional life the source's own `ManaAbilityLifeCost` static charges for activating its mana abilities.
    fn mana_ability_extra_life(&self, card: CardId, p: PlayerId) -> i32 {
        let c = &self.cards[card.0 as usize];
        if c.cur.no_abilities || c.zone != Zone::Battlefield {
            return 0;
        }
        let d = self.db.def(c.def);
        d.statics
            .iter()
            .map(|&i| match &d.abilities[i as usize] {
                Ability::Static { effect: StaticEffect::ManaAbilityLifeCost { n }, condition, .. } if condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &crate::eval::EvalCtx::with_controller(card, p))) => *n,
                _ => 0,
            })
            .sum()
    }

    /// Activate a mana ability: pay its costs and add the mana to the pool.
    /// A mana ability of `card` with a graveyard-exile cost where the player has more than `n` eligible cards: those cards (graveyard order) and `n`.
    pub fn mana_pick_options(&self, p: PlayerId, card: CardId, ability: u16) -> Option<(Vec<CardId>, usize)> {
        if self.mana_gift_ability(p, card, ability) || ability >= crate::cast::RESTRICTED_BASE {
            return None;
        }
        let owned = self.ability_owned(card, ability)?;
        let Ability::Activated { cost, .. } = &owned else { return None };
        for part in cost {
            if let Cost::ExileFromGraveyard { filter, n, bottom: false } = part {
                let mut f = filter.clone();
                f.controller = PlayerFilter::Any;
                let elig: Vec<CardId> = self.players[p as usize].graveyard.iter().copied().filter(|id| self.matches_card(&f, *id, card, p)).collect();
                if elig.len() > *n as usize && *n > 0 {
                    return Some((elig, *n as usize));
                }
            }
        }
        None
    }

    pub fn activate_mana_ability(&mut self, p: PlayerId, card: CardId, ability: u16, produce: Option<ManaType>) -> Result<(), String> {
        // Channel: pay 1 life, add {C}
        if (crate::cast::LIFE_MANA_BASE..crate::cast::INTRINSIC_BASE).contains(&ability) {
            if !matches!(self.players[p as usize].life_mana, Some((t, _)) if t == self.turn + 1) || self.players[p as usize].life < 2 {
                return Err("no life mana".into());
            }
            self.pay_life(p, 1);
            self.players[p as usize].pool.add(ManaType::C, 1);
            return Ok(());
        }
        // a unit of restricted mana floating in the pool: move it into the plain pool for the payment in progress
        if (crate::cast::RESTRICTED_BASE..crate::cast::INTRINSIC_BASE).contains(&ability) {
            let slot = (ability - crate::cast::RESTRICTED_BASE) as usize;
            let Some(Some((t, r, _))) = self.players[p as usize].restricted.get(slot).cloned() else { return Err("that restricted mana is gone".into()) };
            self.players[p as usize].restricted[slot] = None;
            let rider_ok = match (&r.rider_spell, self.pay_for.get()) {
                (None, _) => true,
                (Some(f), Some(sp)) => self.matches_card(f, sp, card, p),
                (Some(_), None) => false,
            };
            if let Some(rider) = r.rider.filter(|_| rider_ok) {
                // a rider on a spell, or "copy that ability" on an activated ability (Sunken Palace)
                if !self.pay_is_ability.get() || rider == crate::dsl::ManaRider::CopySpell {
                    if rider == crate::dsl::ManaRider::CopySpell && self.makes_any_color_copy_rider(card) {
                        self.copy_rider_always.set(true);
                    }
                    self.pay_riders.borrow_mut().push(rider);
                }
            }
            self.players[p as usize].pool.add(t, 1);
            return Ok(());
        }
        if !self.paying_activation.get() && self.def_of(card).subtypes.iter().any(|s| *s == Sym::new("Treasure")) {
            self.players[p as usize].pool_treasure = true;
        }
        let owned = self.ability_owned(card, ability);
        let Some(Ability::Activated { cost, effects, condition, timing, .. }) = owned.as_ref() else {
            return Err("not an activated ability".into());
        };
        let any_pl = self.cards[card.0 as usize].cur.controller != p && self.def_of(card).any_player;
        let foreign = self.cards[card.0 as usize].cur.controller != p && !any_pl && self.foreign_land_ok(p, card);
        if let Some(cond) = condition {
            let mut ec = crate::eval::EvalCtx::of(self, card);
            if any_pl {
                ec.controller = p;
            }
            if !self.eval_cond(cond, &ec) {
                return Err("activation condition not met".into());
            }
        }
        if self.cards[card.0 as usize].cur.controller != p && !any_pl && !foreign {
            return Err("not your permanent".into());
        }
        if *timing == Timing::Sorcery && !self.sorcery_timing(p) {
            return Err("only as a sorcery".into());
        }
        if !cost.iter().all(|c| matches!(c, Cost::Mana { .. }) || self.simple_cost_ok(p, card, c)) {
            return Err("cost can't be paid".into());
        }
        let extra_life = self.mana_ability_extra_life(card, p);
        if extra_life > 0 && !self.can_pay_life(p, extra_life) {
            return Err("can't pay the additional life".into());
        }
        // a mana cost on the ability itself (filter lands) is paid from the pool before anything else
        let mut owed = ManaCost::default();
        for part in cost {
            if let Cost::Mana { cost: m } = part {
                if let Ok(mc) = ManaCost::parse(m) {
                    owed.shards.extend(mc.shards);
                }
            }
        }
        if !owed.is_empty() {
            self.spend_pool(p, &mut owed);
            if !owed.is_empty() && self.paying_activation.get() {
                // this source was picked on purpose to pay for a spell or ability (Sunken Palace): its own mana cost comes from the player's other sources
                let sources: Vec<ManaSourceOpt> = self.mana_sources_for_cost(p, Some(&owed)).into_iter().filter(|s| s.card != card && s.cost.is_empty() && s.restrict.is_none()).collect();
                let pool = self.players[p as usize].pool;
                if let Some(plan) = plan_payment_with(&sources, &owed, 0, pool, 0) {
                    for u in plan.uses {
                        if let PayUse::Source { card: c2, ability: a2, produce } = u {
                            self.note_source_paid(c2);
                            self.note_snow_color(p, c2, a2, Some(produce));
                            let _ = self.activate_mana_ability(p, c2, a2, Some(produce));
                        }
                    }
                    self.spend_pool(p, &mut owed);
                }
            }
            if !owed.is_empty() {
                return Err("the mana cost of the ability can't be paid from the pool".into());
            }
        }
        let mut x_removed = 0i32;
        let mut exiled_now: Vec<Obj> = vec![];
        // an "either/or" cost (Thornvault Forager's forage): the first option that can be paid is used (a mana ability can't ask)
        // a mana ability with an "X times" cost (Hazel of the Rootbloom): X is the player's choice, else as many as can be paid
        let mut x_times: Option<i32> = None;
        if let Some(Cost::XTimes { cost: inner }) = cost.iter().find(|c| matches!(c, Cost::XTimes { .. })) {
            let max = match &**inner {
                Cost::TapPermanents { filter, n } => self.tappable_for_cost(p, card, filter).len() as i32 / (*n).max(1),
                Cost::Sacrifice { filter, n } => self.count_eligible(&self.sac_cost_filter_check(filter), card, p, Zone::Battlefield) as i32 / (*n).max(1),
                _ => 0,
            };
            x_times = Some(self.mana_x_hint.take().unwrap_or(max).clamp(0, max));
        }
        self.mana_x_hint.set(None);
        let cost_flat: Vec<Cost> = cost
            .iter()
            .flat_map(|part| match part {
                Cost::XTimes { cost: inner } if x_times.is_some() => vec![Self::scale_cost(inner, x_times.unwrap_or(0))],
                Cost::Or { options } => options
                    .iter()
                    .find(|opt| opt.iter().all(|c| matches!(c, Cost::Mana { .. }) || self.simple_cost_ok(p, card, c)))
                    .cloned()
                    .unwrap_or_default(),
                other => vec![other.clone()],
            })
            .collect();
        if let Some(k) = x_times {
            x_removed = k;
        }
        for part in cost_flat.iter() {
            match part {
                Cost::RemoveAllCounters { kind } => {
                    let have: Vec<(CounterKind, u16)> = self.cards[card.0 as usize].counters.iter().copied().collect();
                    for (k, n) in have {
                        if kind.map_or(true, |w| w == k) {
                            x_removed += n as i32;
                            self.cards[card.0 as usize].remove_counters(k, n as i32);
                        }
                    }
                    self.layers_dirty = true;
                }
                Cost::RemoveCountersX { kind } => {
                    x_removed = self.cards[card.0 as usize].counter(*kind);
                    self.cards[card.0 as usize].remove_counters(*kind, x_removed);
                    self.layers_dirty = true;
                }
                Cost::Tap => {
                    self.cards[card.0 as usize].tapped = true;
                    self.emit(Event::Tapped { card });
                }
                Cost::Untap => {
                    self.cards[card.0 as usize].tapped = false;
                }
                Cost::SacrificeSelf => {
                    self.emit(Event::Sacrificed { card, player: p });
                    self.move_card(card, Zone::Graveyard, Default::default());
                }
                Cost::ExileSelf => {
                    self.move_card(card, Zone::Exile, Default::default());
                }
                Cost::PutSelfOnLibraryBottom => {
                    self.move_card(card, Zone::Library, crate::zones::MoveOpts { bottom: true, ..Default::default() });
                }
                Cost::CollectEvidence { n } => {
                    if let Some(cards) = self.evidence_cards(p, *n, card) {
                        for id in cards {
                            self.move_card(id, Zone::Exile, Default::default());
                        }
                        self.emit(Event::EvidenceCollected { player: p });
                    }
                }
                Cost::ReturnToHand { filter, .. } if filter.this => {
                    self.move_card(card, Zone::Hand, Default::default());
                }
                Cost::DiscardSelf => {
                    self.move_card(card, self.discard_zone(card), Default::default());
                    self.emit(Event::Discarded { card, player: p, by: self.cause_ctl.get() });
                }
                Cost::PayLife { n } => {
                    self.pay_life(p, *n);
                }
                Cost::PayHalfLife => {
                    let n = (self.players[p as usize].life.max(0) + 1) / 2;
                    self.pay_life(p, n);
                }
                Cost::PayLifeX { amount } => {
                    let n = self.life_cost(card, amount);
                    self.pay_life(p, n);
                }
                Cost::TakeDamage { n } => {
                    self.deal_damage(Some(card), Obj::player(p), *n, false);
                }
                Cost::RemoveCounters { kind, n } => {
                    self.cards[card.0 as usize].remove_counters(*kind, *n);
                    self.layers_dirty = true;
                }
                Cost::RemoveAnyCounters { n } => {
                    self.remove_any_counters(card, *n);
                }
                Cost::RemoveAnyCountersFrom { filter, n } => {
                    self.remove_any_counters_among(filter, *n, card, p);
                }
                Cost::PutCounters { kind, n } => {
                    self.add_counters_to(Obj::card(card), *kind, *n);
                    self.layers_dirty = true;
                }
                Cost::AddMana { color, n } => {
                    self.players[p as usize].pool.add(crate::mana::ManaType::of(*color), (*n).max(0) as u16);
                }
                Cost::PayEnergy { n } => {
                    self.players[p as usize].energy -= *n;
                    self.players[p as usize].energy_spent += *n;
                }
                Cost::DrawCards { n } => {
                    for _ in 0..*n {
                        self.draw_with_replacement(p, false, false);
                    }
                    self.flush_followups();
                }
                Cost::ControllerDraws { n } => {
                    let ctl = self.cards[card.0 as usize].cur.controller;
                    for _ in 0..*n {
                        self.draw_with_replacement(ctl, false, false);
                    }
                    self.flush_followups();
                }
                Cost::Mill { n } => {
                    self.cost_milled.clear();
                    for _ in 0..*n {
                        if let Some(&id) = self.players[p as usize].library.last() {
                            self.cost_milled.push(id);
                            self.move_card(id, Zone::Graveyard, Default::default());
                        }
                    }
                }
                Cost::OpponentGainsLife { n } => {
                    if let Some(o) = self.cost_opponents(p).first().copied() {
                        self.gain_life(o, *n);
                    }
                }
                Cost::PlayerGainsLife { player, n } => self.gain_life(*player, *n),
                Cost::PlayerCreatesToken { player, token, n } => self.player_creates_tokens(*player, token, *n),
                Cost::EachOtherPlayerGainsLife { n } => {
                    for o in self.opponents(p) {
                        self.gain_life(o, *n);
                    }
                }
                Cost::OpponentCreatesToken { token, n } => self.opponent_creates_tokens(p, token, *n),
                Cost::ExileTop { n } => {
                    for _ in 0..*n {
                        if let Some(&id) = self.players[p as usize].library.last() {
                            self.move_card(id, Zone::Exile, Default::default());
                        }
                    }
                }
                Cost::ExileFromBattlefield { filter, n } => {
                    // a mana ability cannot ask: the creature with the highest mana value goes (its value decides the mana)
                    let mut elig = self.eligible_permanents(p, filter, card);
                    elig.sort_by_key(|id| std::cmp::Reverse(self.def_of(*id).mana_value));
                    for id in elig.into_iter().take(*n as usize) {
                        exiled_now.push(Obj::card(id));
                        self.move_card(id, Zone::Exile, Default::default());
                    }
                }
                Cost::TapPermanents { filter, n } => {
                    for id in self.tappable_for_cost(p, card, filter).into_iter().take(*n as usize) {
                        self.cards[id.0 as usize].tapped = true;
                        self.emit(Event::Tapped { card: id });
                    }
                }
                Cost::UntapPermanents { filter, n, opponent } => {
                    for id in self.untappable_for_cost(p, card, filter, *opponent).into_iter().take(*n as usize) {
                        self.cards[id.0 as usize].tapped = false;
                        self.emit(Event::Untapped { card: id });
                        self.cost_sacrificed.push(id);
                    }
                }
                Cost::ExileFromGraveyard { filter, n, bottom } => {
                    // a mana ability can't ask a question: exile the cheapest eligible cards
                    let mut f = filter.clone();
                    f.controller = PlayerFilter::Any;
                    let mut elig: Vec<CardId> = self.players[p as usize].graveyard.iter().copied().filter(|id| self.matches_card(&f, *id, card, p)).collect();
                    elig.sort_by_key(|id| self.def_of(*id).mana_value);
                    // the player's own picks first (Sunken Palace), then the cheapest
                    let picks: Vec<CardId> = std::mem::take(&mut self.mana_picks).into_iter().filter(|id| elig.contains(id)).take(*n as usize).collect();
                    elig.retain(|id| !picks.contains(id));
                    let left = (*n as usize).saturating_sub(picks.len());
                    for id in picks.into_iter().chain(elig.into_iter().take(left)) {
                        if *bottom {
                            self.move_card(id, Zone::Library, crate::zones::MoveOpts { bottom: true, ..Default::default() });
                        } else {
                            self.move_card(id, Zone::Exile, Default::default());
                        }
                    }
                }
                Cost::ExileFromHand { filter, n, .. } => {
                    // a mana ability can't ask a question: exile the cheapest eligible cards
                    let mut f = filter.clone();
                    f.controller = PlayerFilter::Any;
                    let mut elig: Vec<CardId> = self.players[p as usize].hand.iter().copied().filter(|id| self.matches_card(&f, *id, card, p)).collect();
                    elig.sort_by_key(|id| self.def_of(*id).mana_value);
                    for id in elig.into_iter().take(*n as usize) {
                        self.move_card(id, Zone::Exile, Default::default());
                    }
                }
                Cost::Discard { filter, n, .. } => {
                    // a mana ability can't ask a question: discard the cheapest eligible cards
                    let mut f = filter.clone();
                    f.controller = PlayerFilter::Any;
                    let mut elig: Vec<CardId> = self.players[p as usize].hand.iter().copied().filter(|id| self.matches_card(&f, *id, card, p)).collect();
                    elig.sort_by_key(|id| self.def_of(*id).mana_value);
                    for id in elig.into_iter().take(*n as usize) {
                        self.move_card(id, self.discard_zone(id), Default::default());
                        self.emit(Event::Discarded { card: id, player: p, by: self.cause_ctl.get() });
                    }
                }
                Cost::Sacrifice { filter, n } => {
                    // a mana ability can't ask a question: sacrifice the cheapest eligible permanents
                    let mut f = self.sac_cost_filter(filter);
                    f.controller = PlayerFilter::Any;
                    let mut elig: Vec<CardId> = self
                        .battlefield
                        .iter()
                        .copied()
                        .filter(|id| self.cards[id.0 as usize].cur.controller == p && self.matches_card(&f, *id, card, p))
                        .collect();
                    elig.sort_by_key(|id| (*id == card, self.def_of(*id).mana_value, self.cards[id.0 as usize].cur.power));
                    if effects.iter().any(|e| format!("{e:?}").contains("SacrificedCostManaValue")) {
                        // the mana depends on the sacrificed permanent's mana value: sacrifice the most valuable one
                        elig.reverse();
                    }
                    for id in elig.into_iter().take(*n as usize) {
                        self.emit(Event::Sacrificed { card: id, player: p });
                        self.cost_sacrificed.push(id);
                        self.move_card(id, Zone::Graveyard, Default::default());
                    }
                }
                _ => {}
            }
        }
        if extra_life > 0 {
            self.pay_life(p, extra_life);
        }
        {
            // counts as an activation ("if this ability has been activated four or more times this turn")
            let c = &mut self.cards[card.0 as usize];
            if let Some(e) = c.activations.iter_mut().find(|(i, _)| *i == ability) {
                e.1 = e.1.saturating_add(1);
            } else {
                c.activations.push((ability, 1));
            }
            if c.cur.types.has(CardType::Artifact) {
                self.artifact_act_log.push(p);
            }
        }
        let mut chosen = produce;
        self.last_tapped_mana.clear();
        self.last_tapped_mana_n = 0;
        let allowed = self.produces_for(p, effects, 0);
        let sac_ev = EventCtx { obj: self.cost_sacrificed.last().map(|c| Obj::card(*c)), ..Default::default() };
        let eval = |g: &Game, a: &Amount| {
            let mut ec = crate::eval::EvalCtx::of(g, card);
            ec.event = &sac_ev;
            if !exiled_now.is_empty() {
                ec.remembered = &exiled_now;
            }
            if x_removed > 0 {
                ec.x = x_removed;
            }
            g.eval_amount(a, &ec).max(0) as u16
        };
        let collapse = self.mana_collapsed(card);
        let mut collapsed_done = false;
        let pool_before = self.players[p as usize].pool;
        for e in effects {
            let Effect::AddMana { mana, who, player_choice, .. } = e else {
                // a mana ability can also do other things (put a counter, gain life ...); they happen right away, without the stack
                let saved_x = self.cards[card.0 as usize].x;
                if let Some(k) = x_times {
                    self.cards[card.0 as usize].x = k;
                }
                self.run_effect_now(card, p, e);
                self.cards[card.0 as usize].x = saved_x;
                continue;
            };
            // Spectral Searchlight: the mana goes to the chosen player instead of the controller
            let rp = who.as_ref().and_then(|w| self.sel_players(w, &crate::eval::EvalCtx::of(self, card)).first().copied()).unwrap_or(p);
            let rp = if *player_choice { self.mana_gift.unwrap_or(p) } else { rp };
            for m in mana {
                let (t, n): (ManaType, u16) = match m {
                    ManaProduce::Fixed { m, n } | ManaProduce::Rider { m, n, .. } => (*m, *n as u16),
                    ManaProduce::Restricted { m, n, amount, chosen, .. } => (if *chosen { self.cards[card.0 as usize].chosen_color.map_or(*m, ManaType::of) } else { *m }, match amount { Some(a) => eval(self, a), None => *n as u16 }),
                    ManaProduce::NotedMana { .. } => {
                        let all = self.cards[card.0 as usize].noted_mana_all;
                        for (i, t) in ManaType::ALL.iter().enumerate() {
                            if all[i] > 0 {
                                self.players[rp as usize].pool.add(*t, all[i] as u16);
                                self.last_tapped_mana.push(*t);
                                self.last_tapped_mana_n += all[i] as i32;
                                for _ in 0..all[i] {
                                    self.emit(Event::Mana { player: rp, mana: *t });
                                }
                            }
                        }
                        continue;
                    }
                    ManaProduce::NotedType => match self.cards[card.0 as usize].noted_mana {
                        0 => continue,
                        k => (ManaType::ALL[k as usize - 1], 1),
                    },
                    ManaProduce::ChosenColor { n } => match self.cards[card.0 as usize].chosen_color {
                        Some(c) => (ManaType::of(c), *n as u16),
                        None => continue,
                    },
                    ManaProduce::SecondChosenColor { n } => match self.cards[card.0 as usize].chosen_color2 {
                        Some(c) => (ManaType::of(c), *n as u16),
                        None => continue,
                    },
                    ManaProduce::ChosenColorScaled { amount } => match self.cards[card.0 as usize].chosen_color {
                        Some(c) => (ManaType::of(c), eval(self, amount)),
                        None => continue,
                    },
                    ManaProduce::Scaled { m, amount } => (*m, eval(self, amount)),
                    ManaProduce::DevotionOneColor => {
                        let t = pick_color(&mut chosen, &allowed);
                        let colors: Vec<Color> = Color::ALL.iter().copied().filter(|c| ManaType::of(*c) == t).collect();
                        (t, eval(self, &Amount::Devotion { colors }))
                    }
                    ManaProduce::AnyOneColor { n } => (pick_color(&mut chosen, &allowed), *n as u16),
                    ManaProduce::AnyOneColorScaled { amount } | ManaProduce::AnyColorScaled { amount, .. } => (pick_color(&mut chosen, &allowed), eval(self, amount)),
                    ManaProduce::AnyColor { n } | ManaProduce::AnyColorRider { n, .. } => (pick_color(&mut chosen, &allowed), *n as u16),
                    ManaProduce::OwnColorRestricted { .. } => {
                        let own: Vec<ManaType> = Color::ALL.iter().copied().filter(|c| self.cards[card.0 as usize].cur.colors.has(*c)).map(ManaType::of).collect();
                        if own.is_empty() {
                            continue;
                        }
                        (pick_color(&mut chosen, &own), 1)
                    }
                    ManaProduce::RestrictedAny { n, amount, .. } => (pick_color(&mut chosen, &allowed), match amount { Some(a) => eval(self, a), None => *n as u16 }),
                    ManaProduce::OneProduced => (ManaType::C, 0),
                    ManaProduce::CostOf { .. } => continue,
                    ManaProduce::MilledSymbols => {
                        if let Some(mid) = self.cost_milled.last().copied() {
                            let shards: Vec<Shard> = self.def_of(mid).cost.shards.iter().cloned().collect();
                            let mut add: Vec<ManaType> = vec![];
                            for s in shards {
                                match s {
                                    Shard::Color { c } | Shard::Phyrexian { c } | Shard::HybridGeneric { c } | Shard::HybridColorless { c } => add.push(ManaType::of(c)),
                                    Shard::Hybrid { a, b } | Shard::PhyrexianHybrid { a, b } => add.push(pick_color(&mut chosen, &[ManaType::of(a), ManaType::of(b)])),
                                    _ => {}
                                }
                            }
                            for t in add {
                                self.players[rp as usize].pool.add(t, 1);
                                self.last_tapped_mana.push(t);
                                self.last_tapped_mana_n += 1;
                                self.emit(Event::Mana { player: rp, mana: t });
                            }
                        }
                        continue;
                    }
                    ManaProduce::AnyColorOfExiledWith => {
                        let cols = self.exiled_with_colors(card);
                        if cols.is_empty() {
                            continue;
                        }
                        (pick_color(&mut chosen, &cols), 1)
                    }
                    ManaProduce::AnyTypeOfSacrificed | ManaProduce::AnyTypeOfUntapped => {
                        let mut types: Vec<ManaType> = vec![];
                        if let Some(sid) = self.cost_sacrificed.last().copied() {
                            let d = self.db.def(self.cards[sid.0 as usize].def);
                            // the untapped land is the opponent's: it produces mana for its controller
                            let owner = self.cards[sid.0 as usize].cur.controller;
                            for &ai in &d.mana_abilities {
                                if let Ability::Activated { effects, .. } = &d.abilities[ai as usize] {
                                    for t in self.produces_for(owner, effects, 1) {
                                        if !types.contains(&t) {
                                            types.push(t);
                                        }
                                    }
                                }
                            }
                        }
                        if types.is_empty() {
                            continue;
                        }
                        (pick_color(&mut chosen, &types), 1)
                    }
                    ManaProduce::AnyColorAmong { .. } | ManaProduce::AnyColorYouCouldFrom { .. } if allowed.is_empty() => continue,
                    ManaProduce::AnyColorAmong { .. } => (pick_color(&mut chosen, &allowed), 1),
                    ManaProduce::AnyColorYouCouldFrom { n, .. } => (pick_color(&mut chosen, &allowed), *n as u16),
                    ManaProduce::AnyColorYouCould | ManaProduce::AnyCommanderColor | ManaProduce::AnyCommanderColorRider { .. } | ManaProduce::AnyColorOpponentsCould => (pick_color(&mut chosen, &allowed), 1),
                };
                if n == 0 {
                    continue;
                }
                let n = if collapse {
                    if collapsed_done {
                        continue;
                    }
                    collapsed_done = true;
                    1
                } else {
                    n
                };
                // Nyxbloom Ancient / Mana Reflection: the mana from permanents you control is multiplied (before riders count the mana)
                let n = n.saturating_mul(self.mana_multiplier(p, card).max(1) as u16);
                // mana with a rider that is spent on a spell as it is paid for: the spell gets the rider once per mana
                if self.paying_activation.get() && self.pay_is_ability.get() {
                    // "when you spend this mana to ... activate an ability, copy that ability" (Sunken Palace)
                    if matches!(m, ManaProduce::Rider { rider: crate::dsl::ManaRider::CopySpell, .. } | ManaProduce::Restricted { rider: Some(crate::dsl::ManaRider::CopySpell), .. }) {
                        for _ in 0..n {
                            self.pay_riders.borrow_mut().push(crate::dsl::ManaRider::CopySpell);
                        }
                    }
                }
                if self.paying_activation.get() && !self.pay_is_ability.get() {
                    if let Some(sp) = self.pay_for.get() {
                        let rider = match m {
                            ManaProduce::Restricted { rider, .. } => *rider,
                            ManaProduce::Rider { rider, spell, .. } | ManaProduce::AnyColorRider { rider, spell, .. } | ManaProduce::AnyCommanderColorRider { rider, spell } if spell.as_ref().map_or(true, |f| self.matches_card(f, sp, card, p)) => Some(*rider),
                            _ => None,
                        };
                        if let Some(r) = rider {
                            if r == crate::dsl::ManaRider::CopySpell && matches!(m, ManaProduce::AnyColorRider { .. }) {
                                self.copy_rider_always.set(true);
                            }
                            for _ in 0..n {
                                self.pay_riders.borrow_mut().push(r);
                                if r == crate::dsl::ManaRider::ExileSpell {
                                    self.pay_rider_srcs.borrow_mut().push(card);
                                }
                            }
                        }
                    }
                }
                let t = if self.mana_any_color(card) { produce.filter(|c| *c != ManaType::C).or(Some(t).filter(|c| *c != ManaType::C)).unwrap_or(ManaType::W) } else { self.replace_mana_type(card, t) };
                if self.paying_activation.get() && !self.pay_is_ability.get() {
                    if let (Some(sp), ManaProduce::AnyCommanderColorRider { rider, spell }) = (self.pay_for.get(), m) {
                        if spell.as_ref().map_or(true, |f| self.matches_card(f, sp, card, p)) {
                            for _ in 0..n {
                                self.pay_rider_units.borrow_mut().push((*rider, t, spell.clone()));
                            }
                        }
                    }
                }
                if !self.paying_activation.get() && matches!(m, ManaProduce::Restricted { no_generic: true, .. }) {
                    // mana that can't pay generic costs stays restricted while it floats (Jegantha, the Wellspring)
                    let r = ManaRestrict { spell: None, ability: None, rider: None, cost_c: false, upkeep: false, unlock: false, no_generic: true, rider_spell: None };
                    for _ in 0..n {
                        let pl = &mut self.players[rp as usize];
                        pl.restricted.push(Some((t, r.clone(), card)));
                        pl.restricted_turn.resize(pl.restricted.len() - 1, false);
                        pl.restricted_turn.push(false);
                    }
                    self.last_tapped_mana.push(t);
                    self.last_tapped_mana_n += n as i32;
                    for _ in 0..n {
                        self.emit(Event::Mana { player: rp, mana: t });
                    }
                    continue;
                }
                if !self.paying_activation.get() && matches!(m, ManaProduce::Rider { .. }) && !matches!(m, ManaProduce::Rider { rider: crate::dsl::ManaRider::CopySpell, .. }) {
                    // mana with a spell-conditional rider that floats before the spell is cast keeps its rider (Scaled Nurturer, Biophagus;
                    // haste: Generator Servant, Arena of Glory mana made ahead gives the creature spell haste)
                    if let ManaProduce::Rider { rider, spell, .. } = m {
                        let r = ManaRestrict { spell: Some((Filter::default()).into()), ability: Some((Filter::default()).into()), rider: Some(*rider), cost_c: false, upkeep: false, unlock: false, no_generic: false, rider_spell: spell.clone() };
                        for _ in 0..n {
                            let pl = &mut self.players[rp as usize];
                            pl.restricted.push(Some((t, r.clone(), card)));
                            pl.restricted_turn.resize(pl.restricted.len() - 1, false);
                            pl.restricted_turn.push(false);
                        }
                        self.last_tapped_mana.push(t);
                        self.last_tapped_mana_n += n as i32;
                        for _ in 0..n {
                            self.emit(Event::Mana { player: rp, mana: t });
                        }
                        continue;
                    }
                }
                // Primal Wellspring / Path of Ancestry: any-colour mana whose rider applies to a later spell floats keeping the rider and the spell it applies to
                let copy_rider_spell: Option<(crate::dsl::ManaRider, Option<FilterBox>)> = match m {
                    ManaProduce::AnyColorRider { rider: crate::dsl::ManaRider::CopySpell, spell, .. } | ManaProduce::Rider { rider: crate::dsl::ManaRider::CopySpell, spell, .. } => Some((crate::dsl::ManaRider::CopySpell, spell.clone())),
                    ManaProduce::AnyCommanderColorRider { rider, spell } => Some((*rider, spell.clone())),
                    _ => None,
                };
                if !self.paying_activation.get() && (matches!(m, ManaProduce::Restricted { rider: Some(crate::dsl::ManaRider::CopySpell), .. }) || copy_rider_spell.is_some()) {
                    // Sunken Palace mana made ahead of time floats as restricted mana, keeping its "copy it" rider
                    let (frider, fspell) = match copy_rider_spell.clone() {
                        Some((r, sp)) => (r, sp),
                        None => (crate::dsl::ManaRider::CopySpell, None),
                    };
                    let r = ManaRestrict { spell: Some((Filter::default()).into()), ability: Some((Filter::default()).into()), rider: Some(frider), cost_c: false, upkeep: false, unlock: false, no_generic: false, rider_spell: fspell };
                    for _ in 0..n {
                        let pl = &mut self.players[rp as usize];
                        pl.restricted.push(Some((t, r.clone(), card)));
                        pl.restricted_turn.resize(pl.restricted.len() - 1, false);
                        pl.restricted_turn.push(false);
                    }
                    self.last_tapped_mana.push(t);
                    self.last_tapped_mana_n += n as i32;
                    for _ in 0..n {
                        self.emit(Event::Mana { player: rp, mana: t });
                    }
                    continue;
                }
                self.players[rp as usize].pool.add(t, n);
                self.last_tapped_mana.push(t);
                self.last_tapped_mana_n += n as i32;
                for _ in 0..n {
                    self.emit(Event::Mana { player: rp, mana: t });
                }
            }
        }
        // False Dawn: the colored mana this ability added is white instead
        self.convert_added_mana_to_white(p, pool_before);
        // Damping Sphere: a land tapped for two or more mana produces a single {C} instead
        if self.cards[card.0 as usize].cur.types.has(CardType::Land) && self.land_mana_capped() {
            let after = self.players[p as usize].pool;
            let total: u32 = (0..6).map(|i| after.0[i].saturating_sub(pool_before.0[i]) as u32).sum();
            if total >= 2 {
                let mut np = pool_before;
                np.add(ManaType::C, 1);
                self.players[p as usize].pool = np;
                self.last_tapped_mana.clear();
                self.last_tapped_mana.push(ManaType::C);
                self.last_tapped_mana_n = 1;
            }
        }
        // triggered mana abilities ("whenever a land is tapped for mana, add ...")
        if self.cards[card.0 as usize].cur.types.has(CardType::Land) {
            self.land_mana_turn |= 1u32 << (p as u32 & 31);
        }
        self.emit(Event::ManaTapped { card, player: p });
        // a mana ability is still an activated ability (Elrond, Moon-Reader; Ceaseless Searblades)
        self.emit(Event::Activated { card, ability, player: p, life: 0, mana: true });
        Ok(())
    }
}

/// The chosen color if it's allowed, else the first allowed one (white as a last resort).
fn pick_color(chosen: &mut Option<ManaType>, allowed: &[ManaType]) -> ManaType {
    let c = chosen.take().filter(|t| allowed.contains(t));
    c.or_else(|| allowed.iter().copied().find(|t| *t != ManaType::C)).or_else(|| allowed.first().copied()).unwrap_or(ManaType::W)
}

/// Core payment search, independent of game state (so it's easy to test). A source yields a multiset of mana; mana it
/// produces beyond what is needed floats and can pay for later parts of the cost. A source whose ability has its own mana
/// cost (filter lands) must have that cost paid by other sources and the pool first.
pub fn plan_payment_with(sources: &[ManaSourceOpt], cost: &ManaCost, x: u32, pool: ManaPool, max_life: i32) -> Option<Plan> {
    let _g = crate::prof::enter(0);
    if let Ok(mut l) = crate::prof::LAST.try_lock() {
        *l = format!("cost={:?} x={} pool={:?} n_sources={} sources={:?}", cost, x, pool, sources.len(), sources.iter().map(|s| (s.outputs.clone(), s.cost.clone())).collect::<Vec<_>>());
    }
    #[derive(Clone, Copy)]
    enum Req {
        Color(Color),
        Colorless,
        Hybrid(Color, Color),
        HybridGeneric(Color),
        HybridColorless(Color),
        Phyrexian(Color),
        PhyrexianHybrid(Color, Color),
        Generic,
        Snow,
        /// A coloured symbol that may also be paid with mana of any color from a snow source.
        ColorOrSnow(Color),
        /// {C} while white mana is spent as any colour: colourless mana or coloured mana other than white.
        ColorlessOrOther,
    }
    fn reqs_of(cost: &ManaCost, x: u32) -> Vec<Req> {
        let mut reqs: Vec<Req> = vec![];
        let mut generic = 0u32;
        for s in &cost.shards {
            if cost.white_any {
                // white pays any colour; other coloured mana counts as colourless
                match s {
                    Shard::Color { .. } | Shard::Hybrid { .. } => {
                        reqs.push(Req::Color(Color::W));
                        continue;
                    }
                    Shard::Phyrexian { .. } | Shard::PhyrexianHybrid { .. } => {
                        reqs.push(Req::Phyrexian(Color::W));
                        continue;
                    }
                    Shard::HybridGeneric { .. } => {
                        reqs.push(Req::HybridGeneric(Color::W));
                        continue;
                    }
                    Shard::HybridColorless { .. } => {
                        reqs.push(Req::HybridColorless(Color::W));
                        continue;
                    }
                    Shard::Colorless => {
                        reqs.push(Req::ColorlessOrOther);
                        continue;
                    }
                    _ => {}
                }
            }
            match s {
                Shard::Generic { n } => generic += *n as u32,
                Shard::X => generic += x,
                Shard::Snow => reqs.push(Req::Snow),
                Shard::Color { c } => reqs.push(if cost.snow_any { Req::ColorOrSnow(*c) } else { Req::Color(*c) }),
                Shard::Colorless => reqs.push(Req::Colorless),
                Shard::Hybrid { a, b } => reqs.push(Req::Hybrid(*a, *b)),
                Shard::HybridGeneric { c } => reqs.push(Req::HybridGeneric(*c)),
                Shard::HybridColorless { c } => reqs.push(Req::HybridColorless(*c)),
                Shard::Phyrexian { c } => reqs.push(Req::Phyrexian(*c)),
                Shard::PhyrexianHybrid { a, b } => reqs.push(Req::PhyrexianHybrid(*a, *b)),
            }
        }
        for _ in 0..generic {
            reqs.push(Req::Generic);
        }
        let rank = |r: &Req| match r {
            Req::Color(_) | Req::ColorOrSnow(_) | Req::Colorless | Req::ColorlessOrOther => 0,
            Req::Hybrid(..) | Req::HybridColorless(_) => 1,
            Req::Phyrexian(_) | Req::PhyrexianHybrid(..) => 2,
            Req::HybridGeneric(_) => 3,
            Req::Snow => 3,
            Req::Generic => 4,
        };
        reqs.sort_by_key(rank);
        reqs
    }

    struct Search<'a> {
        sources: &'a [ManaSourceOpt],
        /// Equal ids = interchangeable sources.
        sig: Vec<u32>,
        /// Search nodes left; the search gives up (as "can't pay") when it runs out.
        budget: std::cell::Cell<i64>,
        max_life: i32,
        /// States of the top-level search already known to lead nowhere: (requirement index, count, sources used, pool, life paid).
        dead: std::cell::RefCell<std::collections::HashSet<(usize, usize, u64, [u16; 6], i32)>>,
    }
    type Cont<'c> = &'c mut dyn FnMut(u64, ManaPool, &mut Plan) -> bool;

    /// Search with memoization of dead states; only valid for the top-level search (nested own-cost searches have different continuations).
    fn go_top(s: &Search, reqs: &[Req], i: usize, used: u64, pool: ManaPool, plan: &mut Plan, k: Cont) -> bool {
        let key = (i, reqs.len(), used, pool.0, plan.life);
        if i < reqs.len() && s.dead.borrow().contains(&key) {
            return false;
        }
        let ok = go(s, reqs, i, used, pool, plan, k, true);
        if !ok && i < reqs.len() {
            s.dead.borrow_mut().insert(key);
        }
        ok
    }

    #[allow(clippy::too_many_arguments)]
    fn go(s: &Search, reqs: &[Req], i: usize, used: u64, pool: ManaPool, plan: &mut Plan, k: Cont, top: bool) -> bool {
        if s.budget.get() <= 0 {
            return false;
        }
        s.budget.set(s.budget.get() - 1);
        if i == reqs.len() {
            return k(used, pool, plan);
        }
        let rec = |reqs: &[Req], i: usize, used: u64, pool: ManaPool, plan: &mut Plan, k: Cont| -> bool {
            if top {
                go_top(s, reqs, i, used, pool, plan, k)
            } else {
                go(s, reqs, i, used, pool, plan, k, false)
            }
        };
        let req = reqs[i];
        if let Req::ColorOrSnow(c) = req {
            // either mana of the color, or mana of any color from a snow source
            let mut r1 = reqs.to_vec();
            r1[i] = Req::Color(c);
            if go(s, &r1, i, used, pool, plan, &mut *k, false) {
                return true;
            }
            let mut r2 = reqs.to_vec();
            r2[i] = Req::Snow;
            return go(s, &r2, i, used, pool, plan, k, false);
        }
        let wants: Vec<ManaType> = match req {
            Req::ColorOrSnow(c) => vec![ManaType::of(c)],
            Req::Color(c) => vec![ManaType::of(c)],
            Req::Colorless => vec![ManaType::C],
            Req::ColorlessOrOther => vec![ManaType::C, ManaType::U, ManaType::B, ManaType::R, ManaType::G],
            Req::Hybrid(a, b) | Req::PhyrexianHybrid(a, b) => vec![ManaType::of(a), ManaType::of(b)],
            Req::HybridColorless(c) => vec![ManaType::of(c), ManaType::C],
            Req::Phyrexian(c) | Req::HybridGeneric(c) => vec![ManaType::of(c)],
            Req::Generic | Req::Snow => ManaType::ALL.to_vec(),
        };
        // floating mana first
        for want in wants.iter().filter(|_| !matches!(req, Req::Snow)) {
            if pool.get(*want) > 0 {
                let mut p2 = pool;
                p2.take(*want);
                plan.uses.push(PayUse::Pool(*want));
                if rec(reqs, i + 1, used, p2, plan, k) {
                    return true;
                }
                plan.uses.pop();
            }
        }
        // then a source that can produce something useful; its whole output floats, and the requirement is retried
        let mut idx: Vec<usize> = (0..s.sources.len()).filter(|n| used & (1 << n) == 0).collect();
        idx.sort_by_key(|n| (s.sources[*n].outputs.len(), !s.sources[*n].cost.is_empty()));
        let mut tried_sigs: Vec<u32> = vec![];
        for n in idx.into_iter().filter(|n| (!matches!(req, Req::Snow) || s.sources[*n].snow) && !(matches!(req, Req::Generic | Req::Snow) && s.sources[*n].restrict.as_ref().map_or(false, |r| r.no_generic))) {
            // unused sources with the same signature are interchangeable: try only the first
            if tried_sigs.contains(&s.sig[n]) {
                continue;
            }
            tried_sigs.push(s.sig[n]);
            let src = &s.sources[n];
            // when only generic requirements remain, alternatives of equal size are interchangeable: try one of each size
            let only_generic_left = reqs[i..].iter().all(|r| matches!(r, Req::Generic));
            let mut seen_sizes: Vec<u32> = vec![];
            for out in &src.outputs {
                if !out.iter().any(|(t, _)| wants.contains(t)) {
                    continue;
                }
                if only_generic_left {
                    let size: u32 = out.iter().map(|(_, n)| *n as u32).sum();
                    if seen_sizes.contains(&size) {
                        continue;
                    }
                    seen_sizes.push(size);
                }
                // which color decision does this alternative stand for? the first wanted type it contains
                let produce = out.iter().map(|(t, _)| *t).find(|t| wants.contains(t)).unwrap();
                let own_cost = if src.cost.is_empty() { None } else { ManaCost::parse(&src.cost).ok() };
                let virtual_src = |a: u16| (crate::cast::RESTRICTED_BASE..crate::cast::INTRINSIC_BASE).contains(&a);
                let used2 = if virtual_src(src.ability) {
                    used | (1 << n)
                } else {
                    s.sources.iter().enumerate().filter(|(_, o)| o.card == src.card && !virtual_src(o.ability)).fold(used | (1 << n), |m, (k, _)| m | (1 << k))
                };
                let mut after_cost = |u: u64, mut p2: ManaPool, plan: &mut Plan| -> bool {
                    let base = p2;
                    for (t, c) in out {
                        p2.add(*t, *c as u16);
                    }
                    plan.uses.push(PayUse::Source { card: src.card, ability: src.ability, produce });
                    if matches!(req, Req::Snow) {
                        // {S} is paid with the mana of this snow source right away
                        if p2.get(produce) > 0 {
                            p2.take(produce);
                            plan.uses.push(PayUse::Pool(produce));
                            if rec(reqs, i + 1, u, p2, plan, k) {
                                return true;
                            }
                            plan.uses.pop();
                        }
                        plan.uses.pop();
                        return false;
                    }
                    if src.restrict.as_ref().map_or(false, |r| r.no_generic) {
                        // mana that can't pay generic costs: spend it right away on the non-generic requirements it fits
                        let mut r2 = reqs.to_vec();
                        let n_before = plan.uses.len();
                        for (t, c) in out {
                            for _ in 0..*c {
                                let j = (i..r2.len()).find(|&j| match r2[j] {
                                    Req::Color(x) | Req::Phyrexian(x) => ManaType::of(x) == *t,
                                    Req::Hybrid(a, b) | Req::PhyrexianHybrid(a, b) => ManaType::of(a) == *t || ManaType::of(b) == *t,
                                    Req::HybridColorless(x) => ManaType::of(x) == *t || *t == ManaType::C,
                                    Req::HybridGeneric(x) => ManaType::of(x) == *t,
                                    Req::Colorless => *t == ManaType::C,
                                    Req::ColorlessOrOther => true,
                                    _ => false,
                                });
                                if let Some(j) = j {
                                    r2.remove(j);
                                    plan.uses.push(PayUse::Pool(*t));
                                }
                            }
                        }
                        if plan.uses.len() > n_before {
                            // the rest of the mana can't pay generic costs, so it is not offered to the search
                            if go(s, &r2, i, u, base, plan, k, false) {
                                return true;
                            }
                            plan.uses.truncate(n_before);
                        }
                        plan.uses.pop();
                        return false;
                    }
                    // retry this requirement with the new mana floating
                    if rec(reqs, i, u, p2, plan, k) {
                        return true;
                    }
                    plan.uses.pop();
                    false
                };
                match &own_cost {
                    None => {
                        if after_cost(used2, pool, plan) {
                            return true;
                        }
                    }
                    Some(c) => {
                        // pay the ability's own cost first from other sources / the pool (those payments are made implicitly at activation)
                        let sub = reqs_of(c, 0);
                        let mark = plan.uses.len();
                        let mut inner_plan = Plan::default();
                        let ok = go(s, &sub, 0, used2, pool, &mut inner_plan, &mut |u, p2, ip: &mut Plan| {
                            // keep only the *sources* the sub-payment activated (pool spends happen inside the activation)
                            let srcs: Vec<PayUse> = ip.uses.iter().filter(|x| matches!(x, PayUse::Source { .. })).cloned().collect();
                            let n_before = plan.uses.len();
                            plan.uses.extend(srcs);
                            if after_cost(u, p2, plan) {
                                return true;
                            }
                            plan.uses.truncate(n_before);
                            false
                        }, false);
                        if ok {
                            return true;
                        }
                        plan.uses.truncate(mark);
                    }
                }
            }
        }
        match req {
            Req::Phyrexian(_) | Req::PhyrexianHybrid(..) if plan.life + 2 <= s.max_life => {
                plan.life += 2;
                if rec(reqs, i + 1, used, pool, plan, k) {
                    return true;
                }
                plan.life -= 2;
            }
            Req::HybridGeneric(_) => {
                // pay {2}: two of anything
                let mut r2 = reqs.to_vec();
                r2.splice(i..=i, [Req::Generic, Req::Generic]);
                if rec(&r2, i, used, pool, plan, k) {
                    return true;
                }
            }
            _ => {}
        }
        false
    }

    let reqs = reqs_of(cost, x);
    // identical sources are interchangeable and a source yields at least one mana, so more copies of any kind than there are
    // requirements are never needed; grouping them keeps the search small (and under the 64-source mask limit)
    let reduced: Vec<ManaSourceOpt>;
    let (sources, sig): (&[ManaSourceOpt], Vec<u32>) = {
        let mut keys: Vec<String> = vec![];
        let mut counts: Vec<usize> = vec![];
        let mut kept: Vec<ManaSourceOpt> = vec![];
        let mut ids: Vec<u32> = vec![];
        let mut card_seen: Vec<(usize, CardId)> = vec![];
        // enough copies to pay the requirements plus the own-costs of any filter-style sources
        let own_costs: usize = sources.iter().filter(|s| !s.cost.is_empty()).map(|s| ManaCost::parse(&s.cost).map_or(1, |m| m.mana_value(0) as usize)).sum();
        let extra_generic = reqs.iter().filter(|r| matches!(r, Req::HybridGeneric(_))).count();
        let limit = (reqs.len() + extra_generic + own_costs).max(1);
        for src in sources {
            let k = format!("{:?}|{}|{}", src.outputs, src.cost, src.snow);
            let id = match keys.iter().position(|x| *x == k) {
                Some(i) => i,
                None => {
                    keys.push(k);
                    counts.push(0);
                    keys.len() - 1
                }
            };
            // copies are counted per distinct permanent: two abilities of one Sliver are one tap, so they must not crowd out a second Sliver
            let new_card = !card_seen.contains(&(id, src.card));
            if (counts[id] < limit || !new_card) && kept.len() < 63 && (new_card || counts[id] <= limit * 4) {
                if new_card {
                    counts[id] += 1;
                    card_seen.push((id, src.card));
                }
                kept.push(src.clone());
                ids.push(id as u32);
            }
        }
        reduced = kept;
        (&reduced, ids)
    };
    // first try without the sources that have their own mana cost (filter lands, "{5}: add WUBRG"): usually enough
    if sources.iter().any(|s| !s.cost.is_empty()) && sources.iter().any(|s| s.cost.is_empty()) {
        let plain: Vec<ManaSourceOpt> = sources.iter().filter(|s| s.cost.is_empty()).cloned().collect();
        if let Some(p) = plan_payment_with(&plain, cost, x, pool, max_life) {
            return Some(p);
        }
    }
    let s = Search { sources, sig, budget: std::cell::Cell::new(300_000), max_life, dead: Default::default() };
    // quick feasibility: there must be enough mana in total, and enough of each required color, before searching at all
    let supply_total: u32 = pool.total() + sources.iter().map(|src| src.outputs.iter().map(|o| o.iter().map(|(_, n)| *n as u32).sum::<u32>()).max().unwrap_or(0)).sum::<u32>();
    let mut need_total = 0u32;
    let mut need_color = [0u32; 6];
    let mut free_life = max_life.max(0) as u32 / 2;
    for r in &reqs {
        match r {
            Req::Color(c) => {
                need_total += 1;
                need_color[ManaType::of(*c) as usize] += 1;
            }
            Req::Colorless => {
                need_total += 1;
                need_color[ManaType::C as usize] += 1;
            }
            Req::Generic | Req::Snow | Req::ColorlessOrOther | Req::ColorOrSnow(_) | Req::Hybrid(..) | Req::HybridColorless(_) | Req::HybridGeneric(_) => need_total += 1,
            Req::Phyrexian(_) | Req::PhyrexianHybrid(..) => {
                if free_life > 0 {
                    free_life -= 1; // may be paid with life
                } else {
                    need_total += 1;
                }
            }
        }
    }
    if need_total > supply_total + 0 {
        return None;
    }
    for t in ManaType::ALL {
        let have = pool.get(t) as u32 + sources.iter().map(|src| src.outputs.iter().map(|o| o.iter().filter(|(x, _)| *x == t).map(|(_, n)| *n as u32).sum::<u32>()).max().unwrap_or(0)).sum::<u32>();
        if need_color[t as usize] > have {
            return None;
        }
    }
    let mut plan = Plan::default();
    let mut result: Option<Plan> = None;
    let ok = go_top(&s, &reqs, 0, 0, pool, &mut plan, &mut |_, _, p: &mut Plan| {
        result = Some(p.clone());
        true
    });
    if ok {
        result
    } else {
        None
    }
}

impl Game {
    pub fn lose_life(&mut self, p: PlayerId, n: i32) {
        let n = self.replace_lose_life(p, n);
        if n <= 0 {
            return;
        }
        if self.effects.iter().any(|e| matches!(e.modif, Modif::NoLifeLoss) && e.players.contains(&p)) {
            return;
        }
        self.players[p as usize].life -= n;
        self.players[p as usize].life_lost += n;
        self.layers_dirty = true;
        // speed: an opponent lost life during the active player's turn
        let a = self.active as usize;
        if a != p as usize && self.players[a].speed > 0 && self.players[a].speed < 4 && !self.players[a].speed_up {
            self.players[a].speed += 1;
            self.players[a].speed_up = true;
        }
        // simultaneous combat damage is one life loss per player ("loses exactly 1 life": two 1/1s are 2 life in total)
        if self.combat_batch {
            match self.life_batch.iter_mut().find(|(q, _)| *q == p) {
                Some(e) => e.1 += n,
                None => self.life_batch.push((p, n)),
            }
            return;
        }
        self.emit(Event::Life { player: p, delta: -n });
    }

    pub fn gain_life(&mut self, p: PlayerId, n: i32) {
        self.gain_life_from(p, n, false);
    }

    pub fn gain_life_from(&mut self, p: PlayerId, n: i32, lifelink: bool) {
        let n = self.replace_gain_life_from(p, n, lifelink);
        if n < 0 {
            // "if you would gain life, you lose that much life instead"
            self.lose_life(p, -n);
            return;
        }
        if n == 0 {
            return;
        }
        if let Some((src, ctrl, effects)) = self.gain_life_instead(p) {
            self.dmg_followups.borrow_mut().push((src, ctrl, effects, n, None, None, Some(p), vec![]));
            return;
        }
        if self.effects.iter().any(|e| matches!(e.modif, Modif::NoLifeGain) && e.players.contains(&p)) || self.static_cant_gain_life(p) {
            return;
        }
        self.players[p as usize].life += n;
        self.players[p as usize].life_gained += n;
        self.players[p as usize].life_gain_events += 1;
        // life-total conditions (Serra Ascendant, Felidar Sovereign ...) are re-evaluated
        self.layers_dirty = true;
        self.emit(Event::Life { player: p, delta: n });
    }
}

/// The spell filter of a restricted-mana ability, if its mana is restricted.
fn restricted_filter(effects: &[Effect]) -> Option<ManaRestrict> {
    for e in effects {
        if let Effect::AddMana { mana, .. } = e {
            for m in mana {
                if let ManaProduce::Restricted { spell, ability, rider, cost_c, upkeep, unlock, no_generic, .. } = m {
                    return Some(ManaRestrict { spell: spell.clone(), ability: ability.clone(), rider: *rider, cost_c: *cost_c, upkeep: *upkeep, unlock: *unlock, no_generic: *no_generic, rider_spell: None });
                }
                if let ManaProduce::NotedMana { spell } = m {
                    return Some(ManaRestrict { spell: spell.clone(), ability: None, rider: None, cost_c: false, upkeep: false, unlock: false, no_generic: false, rider_spell: None });
                }
                if let ManaProduce::RestrictedAny { spell, ability, .. } = m {
                    return Some(ManaRestrict { spell: spell.clone(), ability: ability.clone(), rider: None, cost_c: false, upkeep: false, unlock: false, no_generic: false, rider_spell: None });
                }
                if let ManaProduce::OwnColorRestricted { ability } = m {
                    return Some(ManaRestrict { spell: None, ability: Some(ability.clone()), rider: None, cost_c: false, upkeep: false, unlock: false, no_generic: false, rider_spell: None });
                }
            }
        }
    }
    None
}

/// A hand-cost filter with "mana value X" removed: X isn't chosen yet while legality is being checked.
/// Does this amount read a chosen target (Sidisi: the sacrificed creature's mana value is the target's minus one)?
fn amount_reads_target(a: &Amount) -> bool {
    match a {
        Amount::ManaValueOf { of: Sel::Target { .. } } => true,
        Amount::Plus { a, b } | Amount::Minus { a, b } => amount_reads_target(a) || amount_reads_target(b),
        _ => false,
    }
}

pub(crate) fn strip_x_filter(f: &Filter) -> Filter {
    let mut fx = f.clone();
    if fx.mana_value_vs.as_ref().map_or(false, |ac| matches!(*ac.amount, Amount::X) || amount_reads_target(&ac.amount)) {
        fx.mana_value_vs = None;
    }
    fx
}
