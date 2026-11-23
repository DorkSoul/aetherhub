//! Continuous effects (CR 613): characteristics are recomputed only when something changed (dirty flag),
//! layer by layer, with dependency ordering (613.8) and timestamps.

use crate::dsl::*;
use crate::eval::EvalCtx;
use crate::ids::*;
use crate::mana::ManaCost;
use crate::state::*;
use crate::types::*;
use smallvec::SmallVec;

#[derive(Clone, Debug)]
enum Aff {
    Filter(Filter),
    Ids(SmallVec<[CardId; 4]>),
    /// The permanent the source is attached to.
    Attached,
    /// The source itself.
    Source,
}

#[derive(Clone, Debug)]
enum Act {
    Control(PlayerId),
    AddTypes(TypeSet, Vec<Sym>),
    RemoveLegendary,
    SetName(Sym),
    SetChosenNameAndType,
    RemoveTypes(TypeSet, Vec<Sym>),
    NoManaCost,
    AllCreatureNames,
    MustAttackPlayer(PlayerId),
    MustAttackPermanent(CardId),
    AddSupertype(Supertype),
    RemoveSupertype(Supertype),
    SetTypes(TypeSet, Vec<Sym>),
    SetSubtypes(Vec<Sym>),
    SetSubtypesKind(SubtypeKind, Vec<Sym>),
    /// The creature types of the last creature card exiled with the source, plus the listed ones.
    ExiledCreatureTypes(Vec<Sym>),
    SetColors(ColorSet),
    AddColors(ColorSet),
    Grant(Keyword),
    /// The protection qualities printed on the cards exiled with the source.
    ExiledProtections,
    GraveyardProtections,
    /// The twelve evergreen keywords and the protection qualities printed on creature cards in exile with a counter of this kind (Rayami).
    ExiledCounterShare(CounterKind),
    NonIdentityProtections,
    Remove(Keyword),
    LoseDefenses,
    RemoveProtection(ProtectionFrom),
    NoAbilities,
    NoAbilitiesKeepMana,
    /// Loses all abilities, including the mana abilities its basic land types give (Imprisoned in the Moon).
    NoAbilitiesNoLandMana,
    CantAttack,
    DmgByToughness,
    DmgByToughnessGt,
    StationByToughness,
    DmgAbs,
    LethalPower,
    LandwalkChosen,
    NoCombatDamage,
    AttackNoDefender,
    AttackNdVsAttackers,
    AssignUnblocked,
    CantAttackPlayer(Filter, bool, bool),
    /// Can't attack this player (nor, unless the flag is set, planeswalkers/battles they control).
    CantAttackController(bool),
    Goaded,
    CantAttackAlone,
    AttackOnlyAlone,
    CantAttackOthers(u8),
    CantBlockOthers(u8),
    CantAttackGreater,
    CantBlockGreater,
    CantBeBlockedGreater,
    AttackHaste,
    CantBlockAlone,
    CanBlockShadow,
    CanBlockLandwalk,
    IgnoreLandwalk(u8),
    CantBeSacrificed,
    CantBeBeamed,
    CantPhaseOut,
    ZeroLoyaltyStays,
    CantBecomeSuspected,
    CantBlockAttackers(Filter),
    CantBlock,
    ExtraBlocks(u8),
    ExtraBlocksAmount(Amount),
    MaxBlockers(u8),
    MaxAttackers(u8),
    MinBlockers(u8),
    BlockAll,
    MustAttack,
    MustBlock(Option<FilterBox>),
    MustBeBlocked,
    MustBeBlockedBy(Filter, u8),
    CantBeBlockedBy(Filter),
    DefenderCond(Condition, bool),
    AttackerCond(Condition),
    Cda(Amount, Amount),
    SetPT(i32, i32),
    SetColorsChosen,
    SetColorsPregame,
    AddColorsChosen,
    SetPTAmount(Amount, Amount),
    SetPTAmountEach(Amount, Amount),
    /// Type layer of `EnchantedAnimate`: a non-creature becomes a creature (and is marked).
    Animate,
    /// P/T layer of `EnchantedAnimate`: animated permanents get this base P/T.
    AnimatePT(Amount, Amount),
    SetP(i32),
    SetT(i32),
    Modify(Amount, Amount),
    ModifyEach(Amount, Amount),
    EnchantedMod(Amount, Amount, Vec<Keyword>),
    /// Switch power and toughness (layer 7e).
    Switch,
    GrantAbility(std::sync::Arc<Ability>),
}

#[derive(Clone, Debug)]
struct Fx {
    ts: u64,
    source: CardId,
    controller: PlayerId,
    aff: Aff,
    act: Act,
    /// For static abilities: (def ability index) so we can re-check that the source still has it.
    from_static: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Layer {
    Control,
    Types,
    Colors,
    Abilities,
    Cda,
    SetPT,
    ModifyPT,
    SwitchPT,
}

const ORDER: [Layer; 8] =
    [Layer::Control, Layer::Types, Layer::Colors, Layer::Abilities, Layer::Cda, Layer::SetPT, Layer::ModifyPT, Layer::SwitchPT];

fn layer_of(a: &Act) -> Layer {
    match a {
        Act::Control(_) => Layer::Control,
        Act::AddTypes(..) | Act::Animate | Act::SetTypes(..) | Act::RemoveTypes(..) | Act::SetSubtypes(_) | Act::SetSubtypesKind(..) | Act::ExiledCreatureTypes(_) | Act::RemoveLegendary | Act::SetName(_) | Act::SetChosenNameAndType | Act::NoManaCost | Act::AllCreatureNames | Act::AddSupertype(_) | Act::RemoveSupertype(_) => Layer::Types,
        Act::SetColors(_) | Act::SetColorsChosen | Act::SetColorsPregame | Act::AddColorsChosen | Act::AddColors(_) => Layer::Colors,
        Act::GrantAbility(_) | Act::ExiledProtections | Act::GraveyardProtections | Act::ExiledCounterShare(_) | Act::NonIdentityProtections | Act::Grant(_) | Act::Remove(_) | Act::RemoveProtection(_) | Act::LoseDefenses | Act::NoAbilities | Act::NoAbilitiesKeepMana | Act::NoAbilitiesNoLandMana | Act::CantAttack | Act::DmgByToughness | Act::DmgByToughnessGt | Act::StationByToughness | Act::DmgAbs | Act::LethalPower | Act::LandwalkChosen | Act::NoCombatDamage | Act::AttackNoDefender | Act::AttackNdVsAttackers | Act::AssignUnblocked | Act::CantAttackPlayer(..) | Act::CantAttackController(..) | Act::CantBlock | Act::ExtraBlocks(_) | Act::ExtraBlocksAmount(_) | Act::MaxBlockers(_) | Act::MaxAttackers(_) | Act::MinBlockers(_) | Act::BlockAll | Act::MustAttack | Act::Goaded | Act::CantAttackAlone | Act::AttackOnlyAlone | Act::CantAttackOthers(_) | Act::CantBlockOthers(_) | Act::CantAttackGreater | Act::CantBlockGreater | Act::CantBeBlockedGreater | Act::AttackHaste | Act::CantBlockAlone | Act::CanBlockShadow | Act::CanBlockLandwalk | Act::IgnoreLandwalk(_) | Act::CantBeSacrificed | Act::CantBeBeamed | Act::CantPhaseOut | Act::ZeroLoyaltyStays | Act::CantBecomeSuspected | Act::MustAttackPlayer(_) | Act::MustAttackPermanent(_) | Act::CantBlockAttackers(_) | Act::MustBlock(_) | Act::MustBeBlocked | Act::MustBeBlockedBy(..) | Act::CantBeBlockedBy(_) | Act::DefenderCond(..) | Act::AttackerCond(..) => {
            Layer::Abilities
        }
        Act::Cda(..) => Layer::Cda,
        Act::SetPT(..) | Act::SetPTAmount(..) | Act::SetPTAmountEach(..) | Act::AnimatePT(..) | Act::SetP(_) | Act::SetT(_) => Layer::SetPT,
        Act::Modify(..) | Act::ModifyEach(..) | Act::EnchantedMod(..) => Layer::ModifyPT,
        Act::Switch => Layer::SwitchPT,
    }
}

impl Game {
    pub fn mark_layers_dirty(&mut self) {
        self.layers_dirty = true;
    }

    /// Continuous effects never read last-known information (Phyrexian Ingester, Drach'Nyen read the exiled card's printed values).
    pub fn recompute_layers(&mut self) {
        if !self.layers_dirty {
            return;
        }
        let saved = self.last_lki.take();
        self.recompute_layers_inner();
        self.last_lki = saved;
    }

    fn recompute_layers_inner(&mut self) {
        let _g = crate::prof::enter(1);
        if !self.layers_dirty {
            return;
        }
        self.layers_dirty = false;
        self.layer_started.borrow_mut().clear();
        self.sync_global_text();
        self.layers_dirty = false;
        let bf = self.battlefield.clone();
        // Mycosynth Lattice: cards outside the battlefield and spells are colorless
        let colorless_all = bf.iter().any(|&id| {
            let c = &self.cards[id.0 as usize];
            !c.cur.no_abilities && {
                let d = self.db.def(c.def);
                d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::ColorlessEverywhere, .. }))
            }
        });
        // Painter's Servant: cards outside the battlefield and spells gain the chosen color
        let mut color_all = 0u8;
        for &id in &bf {
            let c = &self.cards[id.0 as usize];
            if !c.cur.no_abilities {
                let d = self.db.def(c.def);
                if let Some(col) = c.chosen_color {
                    if d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::ColorAddedEverywhere, .. })) {
                        color_all |= col.bit();
                    }
                }
            }
        }
        // Biotransference: creature spells and creature cards outside the battlefield are also artifacts
        let mut art_bits = 0u8;
        for &id in &bf {
            let c = &self.cards[id.0 as usize];
            if !c.cur.no_abilities {
                let d = self.db.def(c.def);
                if d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::CreaturesAreArtifactsEverywhere, .. })) {
                    art_bits |= 1u8 << (c.cur.controller & 7);
                }
            }
        }
        if art_bits != self.artifact_creatures {
            self.artifact_creatures = art_bits;
            for i in 0..self.cards.len() {
                if self.cards[i].zone != Zone::Battlefield {
                    let base = self.db.def(self.cards[i].def).types;
                    self.cards[i].cur.types = self.types_outside_bf(CardId(i as u32), base);
                }
            }
        }
        // Celestial Dawn: nonland cards (and spells) a player owns outside the battlefield are white
        let mut white_nonland = 0u8;
        for &id in &bf {
            let c = &self.cards[id.0 as usize];
            if !c.cur.no_abilities {
                let d = self.db.def(c.def);
                if d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::NonlandCardsWhiteOutsideBattlefield, .. })) {
                    white_nonland |= 1u8 << (c.cur.controller & 7);
                }
            }
        }
        if colorless_all != self.colorless_all || color_all != self.color_all || white_nonland != self.white_nonland {
            self.colorless_all = colorless_all;
            self.color_all = color_all;
            self.white_nonland = white_nonland;
            for i in 0..self.cards.len() {
                if self.cards[i].zone != Zone::Battlefield {
                    self.cards[i].cur.colors = self.colors_outside_bf(CardId(i as u32));
                }
            }
        }
        for id in std::mem::take(&mut self.zone_granted) {
            let cur = &mut self.cards[id.0 as usize].cur;
            cur.granted.clear();
            cur.granted_by.clear();
        }
        let ctrl_before: Vec<(CardId, PlayerId)> = bf.iter().map(|&id| (id, self.cards[id.0 as usize].cur.controller)).collect();
        // base characteristics
        for &id in &bf {
            let c = &self.cards[id.0 as usize];
            let d = self.db.def(c.def);
            let mut cur = Derived {
                types: d.types,
                supertypes: d.supertypes.clone(),
                subtypes: d.subtypes.clone(),
                colors: d.colors,
                power: d.power.unwrap_or(0),
                toughness: d.toughness.unwrap_or(0),
                kw: d.keywords,
                controller: c.controller,
                ..Default::default()
            };
            if c.free_haste {
                cur.kw.add(Keyword::Haste);
            }
            // Volrath's Shapeshifter: the full text of the creature card on top of its controller's graveyard
            if d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::TextOfTopGraveyardCreature, condition: None, .. })) {
                if let Some(&top) = self.players[c.controller as usize].graveyard.last() {
                    let td = self.db.def(self.cards[top.0 as usize].def);
                    if td.types.has(CardType::Creature) && top != id {
                        cur.types = td.types;
                        cur.supertypes = td.supertypes.clone();
                        cur.subtypes = td.subtypes.clone();
                        cur.colors = td.colors;
                        cur.power = td.power.unwrap_or(0);
                        cur.toughness = td.toughness.unwrap_or(0);
                        cur.kw.0 |= td.keywords.0;
                        cur.name = Some(Sym::new(&td.name));
                        for a in &td.abilities {
                            match a {
                                Ability::Protection { from } => cur.protections.extend(from.iter().cloned()),
                                Ability::HexproofFrom { from } => cur.hexproof_from.extend(from.iter().cloned()),
                                Ability::Triggered { .. } | Ability::Activated { .. } | Ability::Static { .. } => {
                                    cur.granted.push(std::sync::Arc::new(a.clone()));
                                    cur.granted_by.push(id);
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
            for a in &d.abilities {
                match a {
                    Ability::Protection { from } => cur.protections.extend(from.iter().cloned()),
                    Ability::HexproofFrom { from } => cur.hexproof_from.extend(from.iter().cloned()),
                    _ => {}
                }
            }
            // mutate: the cards merged beneath (or replaced on top) contribute their keywords and abilities
            for &pd in &c.mut_parts {
                let pdef = self.db.def(pd);
                cur.kw.0 |= pdef.keywords.0;
                for a in &pdef.abilities {
                    match a {
                        Ability::Protection { from } => cur.protections.extend(from.iter().cloned()),
                        Ability::HexproofFrom { from } => cur.hexproof_from.extend(from.iter().cloned()),
                        Ability::Triggered { .. } | Ability::Activated { .. } => {
                            cur.granted.push(std::sync::Arc::new(a.clone()));
                            cur.granted_by.push(id);
                        }
                        _ => {}
                    }
                }
            }
            if c.face_down {
                let mut types = TypeSet::of(&[CardType::Creature]);
                let mut subtypes = SmallVec::new();
                if let Some(x) = &c.fd_extra {
                    if x.2 {
                        types = TypeSet(0);
                    }
                    for t in &x.0 {
                        types = TypeSet(types.0 | t.bit());
                    }
                    subtypes.extend(x.1.iter().map(|s| Sym::new(s)));
                }
                cur = Derived {
                    types,
                    subtypes,
                    power: if types.has(CardType::Creature) { 2 } else { 0 },
                    toughness: if types.has(CardType::Creature) { 2 } else { 0 },
                    controller: c.controller,
                    no_abilities: true,
                    name: if c.fd_copy { Some(Sym::new("")) } else { None },
                    ..Default::default()
                };
            }
            self.cards[id.0 as usize].cur = cur;
        }
        let stored: Vec<Fx> = self.stored_effects();
        for layer in ORDER {
            if layer == Layer::ModifyPT {
                // what "base power and toughness" refers to: the values before modifications and counters
                for &id in &bf {
                    let c = &mut self.cards[id.0 as usize];
                    c.cur.base_pt = Some((c.cur.power, c.cur.toughness));
                }
            }
            let mut fx: Vec<Fx> = self.static_effects(layer);
            fx.extend(stored.iter().filter(|f| layer_of(&f.act) == layer && !pt_flag_fx(f)).cloned());
            if fx.is_empty() {
                continue;
            }
            // a static "loses / can't have keyword" (Archetype of Courage) wins over grants made later
            fx.sort_by_key(|f| (f.from_static && matches!(f.act, Act::Remove(_)), f.ts));
            let ordered = self.order_with_dependencies(fx);
            // an effect that removes all abilities from a permanent is applied before the static abilities of that permanent (dependency, rule 613.8):
            // an enchanted Lord of Atlantis no longer grants islandwalk
            let ordered = if layer == Layer::Abilities && ordered.iter().any(|f| matches!(f.act, Act::NoAbilities | Act::NoAbilitiesKeepMana | Act::NoAbilitiesNoLandMana)) {
                let (mut early, rest): (Vec<Fx>, Vec<Fx>) = ordered.clone().into_iter().partition(|f| {
                    matches!(f.act, Act::NoAbilities | Act::NoAbilitiesKeepMana | Act::NoAbilitiesNoLandMana)
                        && matches!(&f.aff, Aff::Filter(flt) if ordered.iter().any(|o| o.from_static && !std::ptr::eq(o, f) && !matches!(o.act, Act::NoAbilities | Act::NoAbilitiesKeepMana | Act::NoAbilitiesNoLandMana) && o.source != f.source && self.cards[o.source.0 as usize].zone == Zone::Battlefield && self.matches_card(flt, o.source, f.source, f.controller)))
                });
                early.extend(rest);
                early
            } else {
                ordered
            };
            for f in ordered {
                if layer == Layer::Abilities && f.from_static && self.cards[f.source.0 as usize].cur.no_abilities && !matches!(f.act, Act::NoAbilities | Act::NoAbilitiesKeepMana | Act::NoAbilitiesNoLandMana) && self.cards[f.source.0 as usize].zone == Zone::Battlefield {
                    continue;
                }
                // "can't be blocked by creatures with power >= its toughness" reads the final P/T: applied after layer 7 below
                if matches!((&f.act, &f.aff), (Act::CantBeBlockedBy(_), Aff::Filter(flt)) if flt.power_vs.is_some() || flt.toughness_vs.is_some()) {
                    continue;
                }
                // a temporary "creatures with power N or less can't block" is judged on the final P/T (applied after layer 7 below)
                if !f.from_static && matches!((&f.act, &f.aff), (Act::CantBlock | Act::CantAttack, Aff::Filter(flt)) if flt.power.is_some() || flt.toughness.is_some()) {
                    continue;
                }
                if f.from_static {
                    self.layer_started.borrow_mut().push(f.source);
                }
                self.apply_fx(&f);
            }
        }
        // counters modify power / toughness last (7c, commutative with other modifications)
        for &id in &bf {
            let c = &mut self.cards[id.0 as usize];
            let plus = c.counter(CounterKind::PlusOnePlusOne);
            let minus = c.counter(CounterKind::MinusOneMinusOne);
            c.cur.power += plus - minus;
            c.cur.toughness += plus - minus;
            if !c.counters.is_empty() {
                for (k, dp, dt) in [
                    (CounterKind::PlusOnePlusZero, 1, 0),
                    (CounterKind::PlusZeroPlusOne, 0, 1),
                    (CounterKind::PlusTwoPlusTwo, 2, 2),
                    (CounterKind::PlusZeroPlusTwo, 0, 2),
                    (CounterKind::PlusTwoPlusZero, 2, 0),
                    (CounterKind::MinusZeroMinusOne, 0, -1),
                    (CounterKind::MinusZeroMinusTwo, 0, -2),
                    (CounterKind::MinusOneMinusZero, -1, 0),
                    (CounterKind::MinusTwoMinusTwo, -2, -2),
                    (CounterKind::MinusTwoMinusOne, -2, -1),
                    (CounterKind::PlusOnePlusTwo, 1, 2),
                ] {
                    let n = c.counter(k);
                    if n != 0 {
                        c.cur.power += n * dp;
                        c.cur.toughness += n * dt;
                    }
                }
            }
            if c.counter(CounterKind::Flying) > 0 {
                c.cur.kw.add(Keyword::Flying);
            }
            for (ck, kw) in [
                (CounterKind::FirstStrike, Keyword::FirstStrike),
                (CounterKind::DoubleStrike, Keyword::DoubleStrike),
                (CounterKind::Vigilance, Keyword::Vigilance),
                (CounterKind::Deathtouch, Keyword::Deathtouch),
                (CounterKind::Lifelink, Keyword::Lifelink),
                (CounterKind::Reach, Keyword::Reach),
                (CounterKind::Trample, Keyword::Trample),
                (CounterKind::Menace, Keyword::Menace),
                (CounterKind::Hexproof, Keyword::Hexproof),
                (CounterKind::Indestructible, Keyword::Indestructible),
                (CounterKind::Haste, Keyword::Haste),
                (CounterKind::ShadowCounter, Keyword::Shadow),
            ] {
                if c.counter(ck) > 0 {
                    c.cur.kw.add(kw);
                }
            }
            let ex = c.counter(CounterKind::Exalted);
            if ex > 0 {
                if let Some(a) = crate::carddb::keyword_expansion(Keyword::Exalted) {
                    for _ in 0..ex {
                        c.cur.granted.push(std::sync::Arc::new(a.clone()));
                        c.cur.granted_by.push(id);
                    }
                }
            }
            if c.counter(CounterKind::Shield) > 0 {
                // shield counters are handled as a replacement in damage / destroy
            }
        }
        // hone counters on an Equipment: +1/+0 to the equipped creature per counter, even with no static source
        for &id in &bf {
            let c = &self.cards[id.0 as usize];
            let n = c.counter(CounterKind::Hone);
            if n > 0 {
                if let Some(host) = c.attached_to {
                    let h = &mut self.cards[host.0 as usize];
                    if h.zone == Zone::Battlefield {
                        h.cur.power += n;
                    }
                }
            }
        }
        // rules flags from resolved effects that depend on power / toughness ("creatures with power N or less can't block
        // this turn") are applied only here, against the final P/T (a creature pumped above N can block)
        for f in stored.iter().filter(|f| pt_flag_fx(f)) {
            self.apply_fx(f);
        }
        // flag statics conditioned on power / toughness ("as long as its power is 5 or greater, it can attack ...") see the final P/T
        for f in self.static_effects_pt(Layer::Abilities, true, false) {
            self.apply_fx(&f);
        }
        // statics granted to a permanent during the abilities layer itself (Tesak's "other Dogs have unleash") take effect now
        for f in self.static_effects_pt(Layer::Abilities, false, true) {
            // (only effects that are safe to apply twice: flags, maxima / minima and "set" values; additive ones such as
            // ExtraBlocks, ability grants and removals are left to the main pass)
            if matches!(
                f.act,
                Act::CantBlock | Act::CantAttack | Act::Grant(_) | Act::Goaded | Act::MustAttack | Act::MustBeBlocked | Act::MustAttackPlayer(_) | Act::MustAttackPermanent(_)
                    | Act::CantAttackAlone | Act::AttackOnlyAlone | Act::CantBlockAlone | Act::CantAttackOthers(_) | Act::CantBlockOthers(_) | Act::CantAttackGreater | Act::CantBlockGreater
                    | Act::CantBeBlockedGreater | Act::BlockAll | Act::MaxBlockers(_) | Act::MinBlockers(_) | Act::MaxAttackers(_) | Act::AttackHaste | Act::CanBlockShadow | Act::CanBlockLandwalk
                    | Act::IgnoreLandwalk(_) | Act::CantBeSacrificed | Act::CantBeBeamed | Act::CantPhaseOut | Act::ZeroLoyaltyStays | Act::CantBecomeSuspected
                    | Act::DmgByToughness | Act::DmgByToughnessGt | Act::StationByToughness | Act::DmgAbs | Act::LethalPower | Act::NoCombatDamage | Act::AttackNoDefender | Act::AttackNdVsAttackers | Act::AssignUnblocked
            ) {
                self.apply_fx(&f);
            }
        }
        for f in self.static_effects(Layer::Abilities) {
            if matches!((&f.act, &f.aff), (Act::CantBeBlockedBy(_), Aff::Filter(flt)) if flt.power_vs.is_some() || flt.toughness_vs.is_some()) {
                self.apply_fx(&f);
            }
            if matches!(f.act, Act::DmgByToughness | Act::DmgByToughnessGt | Act::StationByToughness | Act::DmgAbs | Act::LethalPower | Act::LandwalkChosen | Act::NoCombatDamage | Act::AttackNoDefender | Act::AttackNdVsAttackers | Act::AssignUnblocked) {
                self.apply_fx(&f);
            }
        }
        // spells on the stack take type grants of "permanent spells you control are artifact spells" (a static whose filter names no card type)
        let spells: Vec<CardId> = self.stack.iter().filter(|i| matches!(i.kind, ItemKind::Spell { .. }) && !i.copy).map(|i| i.source).collect();
        for sp in spells {
            if self.cards[sp.0 as usize].zone != Zone::Stack {
                continue;
            }
            let d = self.db.def(self.cards[sp.0 as usize].def);
            self.cards[sp.0 as usize].cur.types = self.types_outside_bf(sp, d.types);
            self.cards[sp.0 as usize].cur.subtypes = d.subtypes.clone();
            {
                let mut cur = std::mem::take(&mut self.cards[sp.0 as usize].cur);
                self.apply_nonbf_creature(sp, &mut cur);
                self.cards[sp.0 as usize].cur = cur;
            }
            let mut add_types: Vec<CardType> = vec![];
            let mut add_subs: Vec<Sym> = vec![];
            let mut add_kw: Vec<Keyword> = vec![];
            let mut replace_sub: Option<Sym> = None;
            // (a face-up plane's statics count too: Lake Silencio's "all spells have split second")
            let kw_sources: Vec<CardId> = bf.iter().copied().chain(self.planes.iter().copied()).collect();
            for &src in &kw_sources {
                let sc = &self.cards[src.0 as usize];
                if sc.cur.no_abilities {
                    continue;
                }
                let sd = self.db.def(sc.def);
                for &i in &sd.statics {
                    // keywords that matter for a spell on the stack (split second, lifelink on its damage, ...) granted by a static ("instants you control have split second")
                    if let Ability::Static { effect: StaticEffect::GrantKeyword { affects, keyword }, condition, from: None | Some(Zone::Command) } = &sd.abilities[i as usize] {
                        if matches!(keyword, Keyword::SplitSecond | Keyword::Lifelink | Keyword::Deathtouch | Keyword::Infect | Keyword::Wither | Keyword::CantBeCountered | Keyword::Changeling)
                            && !affects.this
                            && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(src, sc.cur.controller)))
                            && self.matches_card(affects, sp, src, sc.cur.controller)
                        {
                            add_kw.push(*keyword);
                        }
                    }
                    if let Ability::Static { effect: StaticEffect::AddChosenSubtype { affects, zone, fixed, replace }, condition, .. } = &sd.abilities[i as usize] {
                        if let (Some(t), true) = (fixed.as_ref().filter(|f| f.as_str() != "Changeling").map(|f| Sym::new(f)).or(sc.chosen_type), zone.map_or(true, |z| z == Zone::Stack)) {
                            if condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(src, sc.cur.controller))) && self.matches_card(affects, sp, src, sc.cur.controller) {
                                if *replace {
                                    replace_sub = Some(t);
                                }
                                add_subs.push(t);
                            }
                        }
                    }
                    if let Ability::Static { effect: StaticEffect::AddType { affects, types, subtypes }, condition, .. } = &sd.abilities[i as usize] {
                        if !affects.types.is_empty() || condition.as_ref().map_or(false, |cd| !self.eval_cond(cd, &EvalCtx::with_controller(src, sc.cur.controller))) {
                            continue;
                        }
                        if self.matches_card(affects, sp, src, sc.cur.controller) {
                            add_types.extend(types.iter().copied());
                            add_subs.extend(subtypes.iter().map(|t| Sym::new(t)));
                        }
                    }
                }
            }
            // keywords granted to the spell by a resolved effect ("target spell can't be countered this turn")
            let szcc = self.cards[sp.0 as usize].zcc;
            for e in &self.effects {
                if let Modif::Keyword(k) = &e.modif {
                    if matches!(k, Keyword::CantBeCountered | Keyword::SplitSecond | Keyword::Lifelink | Keyword::Deathtouch | Keyword::Infect | Keyword::Wither) && e.affects.iter().any(|(c, z)| *c == sp && *z == szcc) {
                        add_kw.push(*k);
                    }
                }
            }
            // a spell cast for its bestow cost is an Aura enchantment spell, not a creature spell
            if self.cast_for_bestow(sp) {
                let cur = &mut self.cards[sp.0 as usize].cur;
                cur.types = TypeSet::of(&[CardType::Enchantment]);
                cur.subtypes = smallvec::smallvec![Sym::new("Aura")];
            }
            let cur = &mut self.cards[sp.0 as usize].cur;
            for k in add_kw {
                cur.kw.add(k);
            }
            for t in add_types {
                cur.types.add(t);
            }
            if replace_sub.is_some() {
                cur.subtypes.retain(|s| !SubtypeKind::Creature.contains(&s.name()));
            }
            for t in add_subs {
                if !cur.subtypes.contains(&t) {
                    cur.subtypes.push(t);
                }
            }
        }
        // cards in hand / graveyard / exile / library that a chosen-creature-type static also covers
        if self.zone_sub_dirty {
            for i in 0..self.cards.len() {
                if !matches!(self.cards[i].zone, Zone::Battlefield | Zone::Stack) {
                    let subs = self.db.def(self.cards[i].def).subtypes.clone();
                    self.cards[i].cur.subtypes = subs;
                    if !self.db.def(self.cards[i].def).keywords.has(Keyword::Changeling) {
                        self.cards[i].cur.kw.remove(Keyword::Changeling);
                    }
                }
            }
            self.zone_sub_dirty = false;
        }
        {
            let mut grants: Vec<(CardId, PlayerId, Sym, Filter, Option<Zone>, bool)> = vec![];
            for &src in &bf {
                let sc = &self.cards[src.0 as usize];
                if sc.cur.no_abilities {
                    continue;
                }
                let sd = self.db.def(sc.def);
                for &i in &sd.statics {
                    if let Ability::Static { effect: StaticEffect::AddChosenSubtype { affects, zone, fixed, replace }, condition, from: None } = &sd.abilities[i as usize] {
                        let Some(t) = fixed.as_ref().map(|f| Sym::new(f)).or(sc.chosen_type) else { continue };
                        if condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(src, sc.cur.controller))) {
                            grants.push((src, sc.cur.controller, t, affects.clone().into_filter(), *zone, *replace));
                        }
                    }
                }
            }
            for (src, ctl, t, affects, zone, replace) in grants {
                let others: Vec<CardId> = self.players.iter().flat_map(|p| p.hand.iter().chain(p.graveyard.iter()).chain(p.exile.iter()).chain(p.library.iter()).copied()).filter(|id| zone.map_or(true, |z| self.cards[id.0 as usize].zone == z)).collect();
                for id in others {
                    // `fixed: "Changeling"`: the card is every creature type (Maskwood Nexus)
                    if t == Sym::new("Changeling") {
                        if self.matches_card(&affects, id, src, ctl) && !self.cards[id.0 as usize].cur.kw.has(Keyword::Changeling) {
                            self.cards[id.0 as usize].cur.kw.add(Keyword::Changeling);
                            self.zone_sub_dirty = true;
                        }
                    } else if replace && self.matches_card(&affects, id, src, ctl) {
                        let cur = &mut self.cards[id.0 as usize].cur;
                        let mut subs: Vec<Sym> = cur.subtypes.iter().copied().filter(|s| !SubtypeKind::Creature.contains(&s.name())).collect();
                        subs.push(t);
                        if subs.len() != cur.subtypes.len() || !subs.iter().all(|s| cur.subtypes.contains(s)) {
                            cur.subtypes = subs.into_iter().collect();
                            self.zone_sub_dirty = true;
                        }
                    } else if self.matches_card(&affects, id, src, ctl) && !self.cards[id.0 as usize].cur.subtypes.contains(&t) {
                        self.cards[id.0 as usize].cur.subtypes.push(t);
                        self.zone_sub_dirty = true;
                    }
                }
            }
        }
        for (id, old) in ctrl_before {
            let c = &self.cards[id.0 as usize];
            if c.zone == Zone::Battlefield && c.cur.controller != old {
                let to = c.cur.controller;
                self.cards[id.0 as usize].control_turn = self.turn;
                self.emit(Event::ControlLost { card: id, from: old, to });
            }
        }
        // "for as long as X remains tapped" ends when X untaps or leaves the battlefield
        let monarch = self.monarch;
        let still = |cards: &Vec<Card>, e: &ContEffect| match e.duration {
            Duration::WhileSourceTapped => cards[e.source.0 as usize].tapped && cards[e.source.0 as usize].zone == Zone::Battlefield,
            Duration::WhileSourceTappedAffectedPowerLe => {
                let src = &cards[e.source.0 as usize];
                src.tapped && src.zone == Zone::Battlefield && e.affects.iter().all(|(id, _)| cards[id.0 as usize].cur.power <= src.cur.power)
            }
            Duration::WhileSourceAttached => cards[e.source.0 as usize].zone == Zone::Battlefield && cards[e.source.0 as usize].attached_to.is_some(),
            Duration::WhileSourceAttachedToAffected => cards[e.source.0 as usize].zone == Zone::Battlefield && e.affects.iter().all(|(id, _)| cards[e.source.0 as usize].attached_to == Some(*id)),
            Duration::WhileAffectedEnchanted => {
                let aura = Sym::new("Aura");
                e.affects.iter().any(|(id, _)| cards.iter().any(|c| c.zone == Zone::Battlefield && c.attached_to == Some(*id) && c.cur.subtypes.contains(&aura)))
            }
            Duration::WhileAffectedTapped => e.affects.iter().all(|(id, _)| cards[id.0 as usize].zone == Zone::Battlefield && cards[id.0 as usize].tapped),
            Duration::WhileAffectedControlledByOther => e.affects.iter().all(|(id, _)| cards[id.0 as usize].zone == Zone::Battlefield && cards[id.0 as usize].cur.controller != e.controller),
            Duration::UntilAffectedTurnedFaceDown => e.affects.iter().all(|(id, _)| cards[id.0 as usize].zone == Zone::Battlefield && !cards[id.0 as usize].face_down),
            Duration::WhileAffectedHasCounter(k) => e.affects.iter().any(|(id, _)| cards[id.0 as usize].zone == Zone::Battlefield && cards[id.0 as usize].counter(k) > 0),
            Duration::WhileSourceControlledTapped => {
                cards[e.source.0 as usize].tapped && cards[e.source.0 as usize].zone == Zone::Battlefield && cards[e.source.0 as usize].cur.controller == e.controller
            }
            Duration::WhilePrevControllerMonarch => e.players.iter().all(|p| monarch == Some(*p)),
            _ => true,
        };
        let untapped = self.effects.iter().any(|e| !still(&self.cards, e));
        if untapped {
            let cards = &self.cards;
            self.effects.retain(|e| still(cards, e));
            self.layers_dirty = true;
            self.recompute_layers();
        }
        // "for as long as you control X" ends when someone else gains control of X: recompute without the effect
        let lost = self.effects.iter().any(|e| e.duration == Duration::WhileSourceControlled && (self.cards[e.source.0 as usize].cur.controller != e.controller || self.cards[e.source.0 as usize].phased_out));
        if lost {
            let cards = &self.cards;
            self.effects.retain(|e| e.duration != Duration::WhileSourceControlled || (cards[e.source.0 as usize].cur.controller == e.controller && !cards[e.source.0 as usize].phased_out));
            self.layers_dirty = true;
            self.recompute_layers();
        }
    }

    /// Effects that come from spells and abilities that already resolved (durations apply).
    fn stored_effects(&self) -> Vec<Fx> {
        let mut out = vec![];
        // a prototyped permanent has the prototype's color and power/toughness
        for &id in &self.battlefield {
            let c = &self.cards[id.0 as usize];
            let Some(alt) = c.cast_alt else { continue };
            let d = self.db.def(c.def);
            let mut n = 0u8;
            for a in &d.abilities {
                match a {
                    Ability::AlternativeCost { .. } => n += 1,
                    Ability::Prototype { cost, power, toughness } if n > 0 && alt == n - 1 => {
                        let colors = ManaCost::parse(cost).map(|m| m.colors()).unwrap_or_default();
                        out.push(Fx { ts: c.ts, source: id, controller: c.controller, aff: Aff::Ids(smallvec::smallvec![id]), act: Act::SetColors(colors), from_static: false });
                        out.push(Fx { ts: c.ts, source: id, controller: c.controller, aff: Aff::Ids(smallvec::smallvec![id]), act: Act::SetPT(*power, *toughness), from_static: false });
                    }
                    _ => {}
                }
            }
        }
        // the Ring-bearer is legendary (rule 701.54c)
        for pl in &self.players {
            if let Some(id) = pl.ring_bearer {
                let c = &self.cards[id.0 as usize];
                if c.zone == Zone::Battlefield && c.cur.controller == pl.id {
                    out.push(Fx { ts: c.ts, source: id, controller: c.controller, aff: Aff::Ids(smallvec::smallvec![id]), act: Act::AddSupertype(Supertype::Legendary), from_static: false });
                }
            }
        }
        // a bestowed card that is attached is an Aura enchantment (no longer a creature)
        for &id in &self.battlefield {
            let c = &self.cards[id.0 as usize];
            if c.bestowed && c.attached_to.is_some() {
                out.push(Fx { ts: c.ts, source: id, controller: c.controller, aff: Aff::Ids(smallvec::smallvec![id]), act: Act::SetTypes(TypeSet::of(&[CardType::Enchantment]), vec![Sym::new("Aura")]), from_static: false });
            }
        }
        // an Aura returned with no host is a plain enchantment
        for &id in &self.battlefield {
            let c = &self.cards[id.0 as usize];
            if c.non_aura {
                let d = self.db.def(c.def);
                let subs: Vec<Sym> = d.subtypes.iter().copied().filter(|s| s.name() != "Aura").collect();
                out.push(Fx { ts: c.ts, source: id, controller: c.controller, aff: Aff::Ids(smallvec::smallvec![id]), act: Act::SetTypes(d.types, subs), from_static: false });
            }
        }
        // returned as an Aura enchantment (Bronzehide Lion): no longer a creature
        for &id in &self.battlefield {
            let c = &self.cards[id.0 as usize];
            if c.as_aura {
                out.push(Fx { ts: c.ts, source: id, controller: c.controller, aff: Aff::Ids(smallvec::smallvec![id]), act: Act::SetTypes(TypeSet::of(&[CardType::Enchantment]), vec![Sym::new("Aura")]), from_static: false });
            }
            if let Some(s) = c.extra_sub {
                out.push(Fx { ts: c.ts, source: id, controller: c.controller, aff: Aff::Ids(smallvec::smallvec![id]), act: Act::AddTypes(TypeSet::default(), vec![s]), from_static: false });
            }
        }
        // suspected creatures have menace and can't block
        for &id in &self.battlefield {
            if self.cards[id.0 as usize].suspected {
                let ts = self.cards[id.0 as usize].ts;
                out.push(Fx { ts, source: id, controller: self.cards[id.0 as usize].controller, aff: Aff::Ids(smallvec::smallvec![id]), act: Act::Grant(Keyword::Menace), from_static: false });
                out.push(Fx { ts, source: id, controller: self.cards[id.0 as usize].controller, aff: Aff::Ids(smallvec::smallvec![id]), act: Act::CantBlock, from_static: false });
            }
        }
        for e in &self.effects {
            if let Modif::DynStatic(se) = &e.modif {
                for (aff, act) in static_to_fx(se) {
                    out.push(Fx { ts: e.ts, source: e.source, controller: e.controller, aff, act, from_static: false });
                }
                continue;
            }
            let ids: SmallVec<[CardId; 4]> = e
                .affects
                .iter()
                .filter(|(id, zcc)| {
                    let c = &self.cards[id.0 as usize];
                    let zone_grant = match &e.modif {
                        Modif::Static(se) => matches!(&**se, StaticEffect::GrantAbility { ability, .. } if zone_grant(ability).map_or(false, |z| z == c.zone)),
                        _ => false,
                    };
                    // a colour change reaches a spell on the stack too (Lifelace, Thoughtlace)
                    // and so does wither (Spinerock Tyrant: "those spells gain wither")
                    let on_stack = c.zone == Zone::Stack && matches!(e.modif, Modif::Colors(_) | Modif::AddColors(_) | Modif::Keyword(Keyword::Wither));
                    (c.zone == Zone::Battlefield || zone_grant || on_stack) && c.zcc == *zcc
                })
                .map(|(id, _)| *id)
                .collect();
            if ids.is_empty() {
                continue;
            }
            if let Modif::Static(se) = &e.modif {
                for (_, act) in static_to_fx(se) {
                    out.push(Fx { ts: e.ts, source: e.source, controller: e.controller, aff: Aff::Ids(ids.clone()), act, from_static: false });
                }
                continue;
            }
            let act = match &e.modif {
                Modif::Pump { power, toughness } => Act::Modify(Amount::Fixed { n: *power }, Amount::Fixed { n: *toughness }),
                Modif::SetPT { power, toughness } => Act::SetPT(*power, *toughness),
                Modif::SetP { power } => Act::SetP(*power),
                Modif::SetT { toughness } => Act::SetT(*toughness),
                Modif::Keyword(k) => Act::Grant(*k),
                Modif::RemoveKeyword(k) => Act::Remove(*k),
                Modif::RemoveProtection(p) => Act::RemoveProtection(p.clone()),
                Modif::Control(p) => Act::Control(*p),
                Modif::AddTypes(t) => Act::AddTypes(*t, vec![]),
                Modif::RemoveTypes(t) => Act::RemoveTypes(*t, vec![]),
                Modif::SetTypes(t, subs) => Act::SetTypes(*t, subs.clone()),
                Modif::Colors(c) => Act::SetColors(*c),
                Modif::AddSubtype(s) => Act::AddTypes(TypeSet::default(), vec![*s]),
                Modif::AddColors(c) => Act::AddColors(*c),
                Modif::NotLegendary => Act::RemoveLegendary,
                Modif::SetName(n) => Act::SetName(*n),
                Modif::NoManaCost => Act::NoManaCost,
                Modif::MustAttackPlayer(p) => Act::MustAttackPlayer(*p),
                Modif::MustAttackPermanent(p) => Act::MustAttackPermanent(*p),
                Modif::AddSupertype(s) => Act::AddSupertype(*s),
                Modif::RemoveSupertype(s) => Act::RemoveSupertype(*s),
                Modif::SetSubtypes(v) => Act::SetSubtypes(v.clone()),
                Modif::SetSubtypesKind(k, v) => Act::SetSubtypesKind(*k, v.clone()),
                Modif::Protection(p) => Act::GrantAbility(std::sync::Arc::new(Ability::Protection { from: vec![p.clone()] })),
                Modif::MustBlockCard(a) => Act::MustBlock(Some(Filter { only_card: Some(*a), ..Default::default() }.into())),
                Modif::NoAbilities => Act::NoAbilities,
                Modif::CantBlock => Act::CantBlock,
                Modif::CantAttack => Act::CantAttack,
                Modif::SwitchPT => Act::Switch,
                _ => continue,
            };
            out.push(Fx { ts: e.ts, source: e.source, controller: e.controller, aff: Aff::Ids(ids), act, from_static: false });
        }
        out
    }

    /// Static abilities of permanents, restricted to the effects that belong to `layer`.
    fn static_effects(&self, layer: Layer) -> Vec<Fx> {
        self.static_effects_pt(layer, false, false)
    }

    /// `final_pt`: false skips keyword grants conditioned on the permanent's own power / toughness (they are
    /// re-evaluated after layer 7 with `final_pt` true, see the post-P/T pass in `recompute`).
    fn static_effects_pt(&self, layer: Layer, final_pt: bool, granted_only: bool) -> Vec<Fx> {
        let mut out = vec![];
        // permanents, plus cards in graveyards whose static abilities work there (Anger)
        let mut sources: Vec<CardId> = self.battlefield.clone();
        for pl in &self.players {
            for &g in pl.command.iter() {
                let dd = self.db.def(self.cards[g.0 as usize].def);
                if dd.statics.iter().any(|&i| matches!(&dd.abilities[i as usize], Ability::Static { from: Some(Zone::Command), .. })) {
                    sources.push(g);
                }
            }
            for &g in &pl.graveyard {
                if self.db.def(self.cards[g.0 as usize].def).statics.iter().any(|&i| matches!(&self.db.def(self.cards[g.0 as usize].def).abilities[i as usize], Ability::Static { from: Some(Zone::Graveyard), .. })) {
                    sources.push(g);
                }
            }
        }
        // spells on the stack whose statics work there ("as long as this is on the stack, each creature has ...")
        for it in &self.stack {
            if matches!(it.kind, ItemKind::Spell { .. }) && !it.copy && !sources.contains(&it.source) {
                let sd = self.db.def(self.cards[it.source.0 as usize].def);
                if sd.statics.iter().any(|&i| matches!(&sd.abilities[i as usize], Ability::Static { from: Some(Zone::Stack), .. })) {
                    sources.push(it.source);
                }
            }
        }
        for src in sources {
            let c = &self.cards[src.0 as usize];
            if c.cur.no_abilities && layer != Layer::Abilities && !self.layer_started.borrow().contains(&src) {
                continue;
            }
            let d = self.db.def(c.def);
            // statics the permanent has printed, plus statics granted to it by earlier layers (Dan Lewis)
            let mut list: Vec<&Ability> = if granted_only { vec![] } else { d.statics.iter().map(|&i| &d.abilities[i as usize]).collect() };
            if c.zone == Zone::Battlefield && !c.cur.no_abilities {
                list.extend(c.cur.granted.iter().map(|a| &**a));
            }
            for ab in list {
                let Ability::Static { effect, condition, from, .. } = ab else { continue };
                // statics work on the battlefield unless they name another zone
                if from.unwrap_or(Zone::Battlefield) != c.zone {
                    continue;
                }
                if let Some(cond) = condition {
                    let pt_cond = layer == Layer::Abilities
                        && (matches!(effect, StaticEffect::GrantKeyword { affects, .. } if affects.this || affects.attached_by_source) || matches!(effect, StaticEffect::CantAttack { affects } | StaticEffect::CantBlock { affects } if affects.this))
                        && (matches!(cond, Condition::SelIs { filter, .. } if filter.power.is_some() || filter.toughness.is_some())
                            || matches!(cond, Condition::Compare { a: Amount::PowerOf { .. } | Amount::ToughnessOf { .. }, .. })
                            || cond_reads_other_pt(cond));
                    if pt_cond != final_pt && pt_cond {
                        continue;
                    }
                    if final_pt && !pt_cond {
                        continue;
                    }
                    if !self.eval_cond(cond, &EvalCtx::with_controller(src, c.cur.controller)) {
                        continue;
                    }
                } else if final_pt {
                    continue;
                }
                if let StaticEffect::AddChosenSubtype { affects, zone, fixed, replace } = effect {
                    if let (Some(t), true, true) = (c.chosen_type, zone.map_or(true, |z| z == Zone::Battlefield), fixed.is_none()) {
                        let act = if *replace { Act::SetSubtypesKind(SubtypeKind::Creature, vec![t]) } else { Act::AddTypes(TypeSet::of(&[]), vec![t]) };
                        if layer_of(&act) == layer {
                            out.push(Fx { ts: c.ts, source: src, controller: c.cur.controller, aff: Aff::Filter(affects.clone().into_filter()), act, from_static: true });
                        }
                    }
                    continue;
                }
                if let StaticEffect::AddSecondChosenType { affects } = effect {
                    if let Some(t) = c.chosen_type2 {
                        let act = Act::AddTypes(TypeSet::of(&[]), vec![t]);
                        if layer_of(&act) == layer {
                            out.push(Fx { ts: c.ts, source: src, controller: c.cur.controller, aff: Aff::Filter(affects.clone().into_filter()), act, from_static: true });
                        }
                    }
                    continue;
                }
                if matches!(effect, StaticEffect::CantAttackPlayer { .. } | StaticEffect::CantAttackController { .. }) {
                    // judged when attackers are declared (power-based filters see the final characteristics)
                    continue;
                }
                if let StaticEffect::GrantAbilitiesFrom { affects, from, mana_only, non_mana, once_each_turn, loyalty_only, triggered, intrinsic_land_mana } = effect {
                    if layer == Layer::Abilities {
                        let ctx = EvalCtx::with_controller(src, c.cur.controller);
                        for fid in self.sel_cards(from, &ctx) {
                            if fid == src {
                                continue;
                            }
                            let fd = self.db.def(self.cards[fid.0 as usize].def);
                            if *intrinsic_land_mana && !*loyalty_only && !*non_mana && self.cards[fid.0 as usize].cur.types.has(CardType::Land) {
                                for (ci, name) in ["Plains", "Island", "Swamp", "Mountain", "Forest"].iter().enumerate() {
                                    if !self.cards[fid.0 as usize].cur.subtypes.contains(&Sym::new(name)) {
                                        continue;
                                    }
                                    let m = ["W", "U", "B", "R", "G"][ci];
                                    let json = format!("{{\"type\":\"Activated\",\"cost\":[{{\"type\":\"Tap\"}}],\"effects\":[{{\"type\":\"AddMana\",\"mana\":[{{\"t\":\"Fixed\",\"m\":\"{m}\",\"n\":1}}]}}]}}");
                                    if let Ok(ab) = serde_json::from_str::<Ability>(&json) {
                                        out.push(Fx { ts: c.ts, source: src, controller: c.cur.controller, aff: Aff::Filter(affects.clone().into_filter()), act: Act::GrantAbility(std::sync::Arc::new(ab)), from_static: true });
                                    }
                                }
                            }
                            for a in &fd.abilities {
                                if *triggered && !*loyalty_only {
                                    if let Ability::Triggered { .. } = a {
                                        out.push(Fx { ts: c.ts, source: src, controller: c.cur.controller, aff: Aff::Filter(affects.clone().into_filter()), act: Act::GrantAbility(std::sync::Arc::new(a.clone())), from_static: true });
                                    }
                                }
                                if let Ability::Activated { loyalty, effects, .. } = a {
                                    if loyalty.is_some() != *loyalty_only {
                                        continue;
                                    }
                                    if *mana_only && !effects.iter().any(|e| matches!(e, Effect::AddMana { .. })) {
                                        continue;
                                    }
                                    if *non_mana && crate::carddb::is_mana_ability(a) {
                                        continue;
                                    }
                                    out.push(Fx { ts: c.ts, source: src, controller: c.cur.controller, aff: Aff::Filter(affects.clone().into_filter()), act: Act::GrantAbility(std::sync::Arc::new(match (a, *once_each_turn) {
                                        (Ability::Activated { cost, effects, targets, timing, loyalty, once, condition, from, power_up, special_action, .. }, true) => Ability::Activated { cost: cost.clone(), effects: effects.clone(), targets: targets.clone(), timing: *timing, loyalty: *loyalty, limit_per_turn: Some(1), once: *once, condition: condition.clone(), from: *from, power_up: *power_up, special_action: *special_action },
                                        _ => a.clone(),
                                    })), from_static: true });
                                }
                            }
                        }
                    }
                    continue;
                }
                for (aff, act) in static_to_fx(effect) {
                    if layer_of(&act) == layer {
                        out.push(Fx { ts: c.ts, source: src, controller: c.cur.controller, aff, act, from_static: true });
                    }
                }
            }
        }
        out
    }

    fn affected(&self, f: &Fx) -> Vec<CardId> {
        match &f.aff {
            Aff::Filter(flt) if matches!(&f.act, Act::GrantAbility(a) if zone_grant(a).is_some()) => {
                // an ability granted to cards in a graveyard / exile / hand ("each artifact card in your graveyard has unearth")
                let Act::GrantAbility(a) = &f.act else { return vec![] };
                let Some(z) = zone_grant(a) else { return vec![] };
                self.players
                    .iter()
                    .flat_map(|p| match z {
                        Zone::Graveyard => p.graveyard.iter(),
                        Zone::Exile => p.exile.iter(),
                        // only the top card of a library (Fblthp, Lost on the Range: "the top card of your library has plot")
                        Zone::Library => p.library[p.library.len().saturating_sub(1)..].iter(),
                        _ => p.hand.iter(),
                    })
                    .copied()
                    .filter(|id| self.matches_card(flt, *id, f.source, f.controller))
                    .collect()
            }
            Aff::Filter(flt) => {
                self.battlefield.iter().copied().filter(|id| self.matches_card(flt, *id, f.source, f.controller)).collect()
            }
            Aff::Ids(ids) => {
                let zone_grant = match &f.act {
                    Act::GrantAbility(a) => zone_grant(a),
                    _ => None,
                };
                let stack_ok = matches!(&f.act, Act::SetColors(_) | Act::AddColors(_));
                ids.iter().copied().filter(|id| self.cards[id.0 as usize].zone == Zone::Battlefield || zone_grant == Some(self.cards[id.0 as usize].zone) || (stack_ok && self.cards[id.0 as usize].zone == Zone::Stack)).collect()
            }
            Aff::Attached => self.cards[f.source.0 as usize].attached_to.into_iter().collect(),
            Aff::Source => vec![f.source],
        }
    }

    fn apply_fx(&mut self, f: &Fx) {
        if f.from_static && self.cards[f.source.0 as usize].cur.no_abilities && layer_of(&f.act) == Layer::Abilities {
            // the source lost its abilities earlier in this layer
            return;
        }
        let targets = self.affected(f);
        let ctx = EvalCtx::with_controller(f.source, f.controller);
        match &f.act {
            Act::Control(p) => {
                let p = if *p == PlayerId::MAX { Some(f.controller) } else if *p == PlayerId::MAX - 1 { self.monarch } else { Some(*p) };
                // rule 800.4a: effects that would give a player who has left the game control of something end
                let p = p.filter(|p| !self.players.get(*p as usize).map_or(true, |pl| pl.lost));
                if let Some(p) = p {
                    for id in targets {
                        if self.control_change_barred(id, p) {
                            continue;
                        }
                        self.cards[id.0 as usize].cur.controller = p;
                    }
                }
            }
            Act::AddTypes(t, subs) => {
                for id in targets {
                    let cur = &mut self.cards[id.0 as usize].cur;
                    cur.types.0 |= t.0;
                    for s in subs {
                        if !cur.subtypes.contains(s) {
                            cur.subtypes.push(*s);
                        }
                    }
                }
            }
            Act::AddSupertype(st) => {
                for id in targets {
                    let cur = &mut self.cards[id.0 as usize].cur;
                    if !cur.supertypes.contains(st) {
                        cur.supertypes.push(*st);
                    }
                }
            }
            Act::RemoveSupertype(st) => {
                for id in targets {
                    self.cards[id.0 as usize].cur.supertypes.retain(|s| s != st);
                }
            }
            Act::AllCreatureNames => {
                for id in targets {
                    self.cards[id.0 as usize].cur.all_creature_names = true;
                }
            }
            Act::NoManaCost => {
                for id in targets {
                    self.cards[id.0 as usize].cur.no_mana_cost = true;
                }
            }
            Act::SetName(n) => {
                for id in targets {
                    self.cards[id.0 as usize].cur.name = Some(*n);
                }
            }
            Act::SetChosenNameAndType => {
                let (name, ty) = {
                    let sc = &self.cards[f.source.0 as usize];
                    (sc.chosen_name.clone(), sc.chosen_type)
                };
                for id in targets {
                    let cur = &mut self.cards[id.0 as usize].cur;
                    if let Some(n) = &name {
                        cur.name = Some(Sym::new(n));
                    }
                    if let Some(t) = ty {
                        cur.subtypes.retain(|s| !SubtypeKind::Creature.contains(&s.name()));
                        cur.subtypes.push(t);
                    }
                }
            }
            Act::RemoveLegendary => {
                for id in targets {
                    self.cards[id.0 as usize].cur.supertypes.retain(|s| *s != Supertype::Legendary);
                }
            }
            Act::SetTypes(t, subs) => {
                for id in targets {
                    let cur = &mut self.cards[id.0 as usize].cur;
                    cur.types = *t;
                    cur.subtypes = subs.iter().copied().collect();
                    // 305.7: setting a land's subtype to a basic land type removes the abilities from its rules text (its mana ability is replaced by the new type's intrinsic one)
                    if cur.types.has(CardType::Land) && subs.iter().any(|s| ["Plains", "Island", "Swamp", "Mountain", "Forest"].contains(&s.name().as_str())) {
                        cur.no_abilities = true;
                    }
                }
            }
            Act::RemoveTypes(t, subs) => {
                for id in targets {
                    let cur = &mut self.cards[id.0 as usize].cur;
                    cur.types.0 &= !t.0;
                    cur.subtypes.retain(|s| !subs.contains(s));
                    // an object that stops being a type also loses the subtypes of that type (rule 205.1b)
                    for (ty, names) in [
                        (CardType::Artifact, &["Attraction", "Blood", "Bobblehead", "Clue", "Contraption", "Equipment", "Food", "Fortification", "Gold", "Incubator", "Infinity", "Junk", "Map", "Powerstone", "Treasure", "Vehicle"][..]),
                        (CardType::Enchantment, &["Aura", "Background", "Cartouche", "Case", "Class", "Curse", "Role", "Room", "Saga", "Shard", "Shrine"][..]),
                        (CardType::Land, &["Plains", "Island", "Swamp", "Mountain", "Forest", "Desert", "Gate", "Lair", "Locus", "Mine", "Power-Plant", "Tower", "Urza's", "Cave", "Sphere"][..]),
                    ] {
                        if t.has(ty) {
                            cur.subtypes.retain(|s| !names.contains(&s.name().as_str()));
                        }
                    }
                }
            }
            Act::ExiledCreatureTypes(keep) => {
                let on_bf = self.cards[f.source.0 as usize].zone == Zone::Battlefield;
                let z = self.cards[f.source.0 as usize].zcc;
                let last = self
                    .cards
                    .iter()
                    .filter(|c| c.zone == Zone::Exile && c.exiled_with.map_or(false, |(s, zz)| s == f.source && (!on_bf || zz == z)) && self.db.def(c.def).types.has(CardType::Creature))
                    .max_by_key(|c| c.ts)
                    .map(|c| self.db.def(c.def).subtypes.iter().copied().filter(|s| SubtypeKind::Creature.contains(&s.name())).collect::<Vec<Sym>>());
                if let Some(subs) = last {
                    for id in targets {
                        let cur = &mut self.cards[id.0 as usize].cur;
                        cur.subtypes.retain(|s| !SubtypeKind::Creature.contains(&s.name()));
                        for s in subs.iter().chain(keep.iter()) {
                            if !cur.subtypes.contains(s) {
                                cur.subtypes.push(*s);
                            }
                        }
                    }
                }
            }
            Act::SetSubtypesKind(kind, subs) => {
                for id in targets {
                    let cur = &mut self.cards[id.0 as usize].cur;
                    cur.subtypes.retain(|s| !kind.contains(&s.name()));
                    for s in subs {
                        if !cur.subtypes.contains(s) {
                            cur.subtypes.push(*s);
                        }
                    }
                }
            }
            Act::SetSubtypes(subs) => {
                for id in targets {
                    self.cards[id.0 as usize].cur.subtypes = subs.iter().copied().collect();
                }
            }
            Act::SetColors(cs) => {
                for id in targets {
                    self.cards[id.0 as usize].cur.colors = *cs;
                }
            }
            Act::SetColorsChosen => {
                let col = self.cards[f.source.0 as usize].chosen_color;
                for id in targets {
                    self.cards[id.0 as usize].cur.colors = col.map_or(ColorSet::default(), |c| ColorSet::of(&[c]));
                }
            }
            Act::SetColorsPregame => {
                let col = self.players[self.cards[f.source.0 as usize].owner as usize].pregame_color;
                for id in targets {
                    self.cards[id.0 as usize].cur.colors = col.map_or(ColorSet::default(), |c| ColorSet::of(&[c]));
                }
            }
            Act::AddColorsChosen => {
                if let Some(col) = self.cards[f.source.0 as usize].chosen_color {
                    for id in targets {
                        self.cards[id.0 as usize].cur.colors.add(col);
                    }
                }
            }
            Act::AddColors(cs) => {
                for id in targets {
                    self.cards[id.0 as usize].cur.colors.0 |= cs.0;
                }
            }
            Act::Grant(k) => {
                let exp = crate::carddb::keyword_expansion(*k);
                for id in targets {
                    // keywords that are really triggered abilities (prowess, undying ...) need the ability when granted
                    if let Some(a) = &exp {
                        let printed = self.db.def(self.cards[id.0 as usize].def).keywords.has(*k);
                        // a second source of the same triggered keyword is a second instance (two exalted / riot / dethrone triggers)
                        if (!printed && !self.cards[id.0 as usize].cur.kw.has(*k)) || (printed || self.cards[id.0 as usize].cur.kw.has(*k)) && matches!(k, Keyword::Exalted | Keyword::Riot | Keyword::Dethrone | Keyword::Prowess) {
                            self.cards[id.0 as usize].cur.granted.push(std::sync::Arc::new(a.clone()));
                            self.cards[id.0 as usize].cur.granted_by.push(id);
                        }
                    }
                    self.cards[id.0 as usize].cur.add_keyword(*k);
                    // "can't be blocked this turn" and the like from a resolved spell or ability are rules effects, not abilities
                    if !f.from_static && rules_effect_keyword(*k) {
                        self.cards[id.0 as usize].cur.effect_kw.add(*k);
                    }
                }
            }
            Act::GrantAbility(a) if zone_grant(a).is_some() => {
                for id in targets {
                    // "{OWN}" in a Mana cost is the card's own mana cost, "{MV}" its mana value as generic mana
                    let mut ab = a.clone();
                    if let Ability::Activated { cost, .. } = &**a {
                        if cost.iter().any(|c| matches!(c, Cost::Mana { cost } if cost == "{OWN}" || cost == "{MV}")) {
                            let d = self.db.def(self.cards[id.0 as usize].def);
                            let own = d.cost.display();
                            let mv = d.cost.mana_value(0);
                            let mut a2 = (**a).clone();
                            if let Ability::Activated { cost, .. } = &mut a2 {
                                for c in cost.iter_mut() {
                                    if let Cost::Mana { cost: m } = c {
                                        if m == "{OWN}" {
                                            *m = own.clone();
                                        } else if m == "{MV}" {
                                            *m = format!("{{{mv}}}");
                                        }
                                    }
                                }
                            }
                            ab = std::sync::Arc::new(a2);
                        }
                    }
                    self.cards[id.0 as usize].cur.granted.push(ab);
                    self.cards[id.0 as usize].cur.granted_by.push(f.source);
                    self.zone_granted.push(id);
                }
            }
            Act::GrantAbility(a) => {
                for id in targets {
                    match &**a {
                        // keyword-like abilities become characteristics
                        Ability::Protection { from } => {
                            for p in from {
                                // "protection from the chosen colour / type / player" granted by a permanent that made the choice (Sanctuary Blade)
                                if let Some(p) = self.settle_chosen_quality(f.source, p) {
                                    self.cards[id.0 as usize].cur.protections.push(p);
                                }
                            }
                        }
                        Ability::HexproofFrom { from } => {
                            for p in from {
                                // "hexproof from the chosen colour" granted by the permanent that chose it
                                if let Some(p) = self.settle_chosen_quality(f.source, p) {
                                    self.cards[id.0 as usize].cur.hexproof_from.push(p);
                                }
                            }
                        }
                        _ => {
                            self.cards[id.0 as usize].cur.granted.push(a.clone());
                            self.cards[id.0 as usize].cur.granted_by.push(f.source);
                        }
                    }
                }
            }
            Act::ExiledProtections => {
                let on_bf = self.cards[f.source.0 as usize].zone == Zone::Battlefield;
                let z = self.cards[f.source.0 as usize].zcc;
                let mut ps: Vec<ProtectionFrom> = vec![];
                for c in self.cards.iter() {
                    if c.zone == Zone::Exile && c.exiled_with.map_or(false, |(s, zz)| s == f.source && (!on_bf || zz == z)) {
                        for a in &self.db.def(c.def).abilities {
                            if let Ability::Protection { from } = a {
                                ps.extend(from.iter().cloned());
                            }
                        }
                    }
                }
                for id in targets {
                    self.cards[id.0 as usize].cur.protections.extend(ps.iter().cloned());
                }
            }
            Act::GraveyardProtections => {
                let mut ps: Vec<ProtectionFrom> = vec![];
                for p in &self.players {
                    for gid in &p.graveyard {
                        let d = self.db.def(self.cards[gid.0 as usize].def);
                        if d.types.has(CardType::Creature) {
                            for a in &d.abilities {
                                if let Ability::Protection { from } = a {
                                    for q in from {
                                        if !ps.contains(q) {
                                            ps.push(q.clone());
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                for id in targets {
                    self.cards[id.0 as usize].cur.protections.extend(ps.iter().cloned());
                }
            }
            Act::ExiledCounterShare(kind) => {
                let mut ps: Vec<ProtectionFrom> = vec![];
                let mut kws: Vec<Keyword> = vec![];
                for c in self.cards.iter() {
                    if c.zone != Zone::Exile || !c.counters.iter().any(|(k, n)| k == kind && *n > 0) {
                        continue;
                    }
                    let d = self.db.def(c.def);
                    if !d.types.has(CardType::Creature) {
                        continue;
                    }
                    for k in [Keyword::Flying, Keyword::FirstStrike, Keyword::DoubleStrike, Keyword::Deathtouch, Keyword::Haste, Keyword::Hexproof, Keyword::Indestructible, Keyword::Lifelink, Keyword::Menace, Keyword::Reach, Keyword::Trample, Keyword::Vigilance] {
                        if d.keywords.has(k) && !kws.contains(&k) {
                            kws.push(k);
                        }
                    }
                    for a in &d.abilities {
                        if let Ability::Protection { from } = a {
                            for q in from {
                                if !ps.contains(q) {
                                    ps.push(q.clone());
                                }
                            }
                        }
                    }
                }
                for id in targets {
                    for k in &kws {
                        self.cards[id.0 as usize].cur.add_keyword(*k);
                    }
                    self.cards[id.0 as usize].cur.protections.extend(ps.iter().cloned());
                }
            }
            Act::NonIdentityProtections => {
                let ctl = self.cards[f.source.0 as usize].cur.controller;
                let ident = self.commander_identity(ctl);
                // no commander: no colours are outside "its" identity (Commander's Plate ruling)
                let has_commander = self.cards.iter().any(|c| c.is_commander && c.owner == ctl);
                for id in targets.iter().filter(|_| has_commander) {
                    for col in Color::ALL {
                        if !ident.has(col) {
                            self.cards[id.0 as usize].cur.protections.push(ProtectionFrom::Color { c: col });
                        }
                    }
                }
            }
            Act::Remove(k) => {
                for id in targets {
                    self.cards[id.0 as usize].cur.kw.remove(*k);
                    self.cards[id.0 as usize].cur.effect_kw.remove(*k);
                }
            }
            Act::LoseDefenses => {
                for id in targets {
                    let cur = &mut self.cards[id.0 as usize].cur;
                    cur.kw.remove(Keyword::Hexproof);
                    cur.kw.remove(Keyword::Indestructible);
                    cur.kw.remove(Keyword::Shroud);
                    cur.protections.clear();
                    cur.hexproof_from.clear();
                    cur.no_ward = true;
                }
            }
            Act::RemoveProtection(p) => {
                for id in targets {
                    self.cards[id.0 as usize].cur.protections.retain(|q| q != p);
                }
            }
            Act::NoAbilities | Act::NoAbilitiesKeepMana | Act::NoAbilitiesNoLandMana => {
                for id in targets {
                    // changeling is a characteristic-defining ability (layer 4), so it survives "loses all abilities" (layer 6)
                    // (unless an earlier layer-4 effect already set its creature types: Frogify overwrites changeling)
                    let changeling = {
                        let c = &self.cards[id.0 as usize];
                        let d = self.db.def(c.def);
                        d.keywords.has(Keyword::Changeling) && c.cur.subtypes.len() == d.subtypes.len() && d.subtypes.iter().all(|t| c.cur.subtypes.contains(t))
                    };
                    let cur = &mut self.cards[id.0 as usize].cur;
                    cur.keep_mana_abilities = matches!(f.act, Act::NoAbilitiesKeepMana);
                    cur.no_land_mana = matches!(f.act, Act::NoAbilitiesNoLandMana);
                    cur.kw = cur.effect_kw;
                    if changeling {
                        cur.kw.add(Keyword::Changeling);
                    }
                    cur.no_abilities = true;
                    cur.granted.clear();
                    cur.granted_by.clear();
                    cur.protections.clear();
                    cur.hexproof_from.clear();
                }
            }
            Act::DmgByToughnessGt => {
                for id in targets {
                    self.cards[id.0 as usize].cur.dmg_by_toughness_gt = true;
                }
            }
            Act::StationByToughness => {
                for id in targets {
                    self.cards[id.0 as usize].cur.station_by_toughness = true;
                }
            }
            Act::DmgByToughness => {
                for id in targets {
                    self.cards[id.0 as usize].cur.dmg_by_toughness = true;
                }
            }
            Act::LandwalkChosen => {
                if let Some(t) = self.cards[f.source.0 as usize].chosen_type {
                    for id in targets {
                        self.cards[id.0 as usize].cur.extra_landwalk.push(t);
                    }
                }
            }
            Act::DmgAbs => {
                for id in targets {
                    self.cards[id.0 as usize].cur.dmg_abs_power = true;
                }
            }
            Act::LethalPower => {
                for id in targets {
                    self.cards[id.0 as usize].cur.lethal_by_power = true;
                }
            }
            Act::NoCombatDamage => {
                for id in targets {
                    self.cards[id.0 as usize].cur.no_combat_damage = true;
                }
            }
            Act::AssignUnblocked => {
                for id in targets {
                    self.cards[id.0 as usize].cur.assign_as_unblocked = true;
                }
            }
            Act::AttackNdVsAttackers => {
                for id in targets {
                    self.cards[id.0 as usize].cur.attack_nd_vs_attackers = true;
                }
            }
            Act::AttackNoDefender => {
                for id in targets {
                    self.cards[id.0 as usize].cur.attack_no_defender = true;
                }
            }
            Act::CantAttack => {
                for id in targets {
                    self.cards[id.0 as usize].cur.cant_attack = true;
                }
            }
            Act::CantBlock => {
                for id in targets {
                    self.cards[id.0 as usize].cur.cant_block = true;
                }
            }
            Act::MaxBlockers(n) => {
                for id in targets {
                    let e = &mut self.cards[id.0 as usize].cur.max_blockers;
                    *e = if *e == 0 { *n } else { (*e).min(*n) };
                }
            }
            Act::MaxAttackers(n) => {
                for id in targets {
                    let e = &mut self.cards[id.0 as usize].cur.max_attackers;
                    *e = if *e == 0 { *n } else { (*e).min(*n) };
                }
            }
            Act::BlockAll => {
                for id in targets {
                    self.cards[id.0 as usize].cur.block_all = true;
                }
            }
            Act::MinBlockers(n) => {
                for id in targets {
                    let e = &mut self.cards[id.0 as usize].cur.min_blockers;
                    *e = (*e).max(*n);
                }
            }
            Act::ExtraBlocksAmount(a) => {
                let n = self.eval_amount(a, &ctx).clamp(0, 255) as u8;
                for id in targets {
                    let e = &mut self.cards[id.0 as usize].cur.extra_blocks;
                    *e = e.saturating_add(n);
                }
            }
            Act::ExtraBlocks(n) => {
                for id in targets {
                    let e = &mut self.cards[id.0 as usize].cur.extra_blocks;
                    *e = e.saturating_add(*n);
                }
            }
            Act::MustAttackPlayer(pl) => {
                let pl = if *pl == PlayerId::MAX {
                    f.controller
                } else if *pl == PlayerId::MAX - 1 {
                    match self.cards[f.source.0 as usize].attached_to {
                        Some(h) => self.cards[h.0 as usize].cur.controller,
                        None => return,
                    }
                } else {
                    *pl
                };
                let pl = &pl;
                for id in targets {
                    self.cards[id.0 as usize].cur.must_attack_player = Some(*pl);
                }
            }
            Act::MustAttackPermanent(pw) => {
                let pw = if pw.0 == u32::MAX { f.source } else { *pw };
                for id in targets {
                    self.cards[id.0 as usize].cur.must_attack_pw = Some(pw);
                }
            }
            Act::MustAttack => {
                for id in targets {
                    self.cards[id.0 as usize].cur.must_attack = true;
                }
            }
            Act::AttackHaste => {
                for id in targets {
                    self.cards[id.0 as usize].cur.attack_haste = true;
                }
            }
            Act::CantAttackOthers(n) => {
                for id in targets {
                    let c = &mut self.cards[id.0 as usize].cur;
                    c.attack_needs_others = c.attack_needs_others.max(*n);
                }
            }
            Act::CantBlockOthers(n) => {
                for id in targets {
                    let c = &mut self.cards[id.0 as usize].cur;
                    c.block_needs_others = c.block_needs_others.max(*n);
                }
            }
            Act::CantAttackAlone => {
                for id in targets {
                    self.cards[id.0 as usize].cur.cant_attack_alone = true;
                }
            }
            Act::AttackOnlyAlone => {
                for id in targets {
                    self.cards[id.0 as usize].cur.attack_only_alone = true;
                }
            }
            Act::CantBlockAttackers(flt) => {
                for id in targets {
                    self.cards[id.0 as usize].cur.cant_block_attackers.push((flt.clone(), f.source, f.controller));
                }
            }
            Act::CantBecomeSuspected => {
                for id in targets {
                    self.cards[id.0 as usize].cur.cant_become_suspected = true;
                }
            }
            Act::ZeroLoyaltyStays => {
                for id in targets {
                    self.cards[id.0 as usize].cur.zero_loyalty_stays = true;
                }
            }
            Act::CantPhaseOut => {
                for id in targets {
                    self.cards[id.0 as usize].cur.cant_phase_out = true;
                }
            }
            Act::CantBeSacrificed => {
                for id in targets {
                    self.cards[id.0 as usize].cur.cant_be_sacrificed = true;
                }
            }
            Act::CantBeBeamed => {
                for id in targets {
                    self.cards[id.0 as usize].cur.cant_be_beamed = true;
                }
            }
            Act::IgnoreLandwalk(mask) => {
                for id in targets {
                    self.cards[id.0 as usize].cur.ignore_landwalk |= *mask;
                }
            }
            Act::CanBlockLandwalk => {
                for id in targets {
                    self.cards[id.0 as usize].cur.can_block_landwalk = true;
                }
            }
            Act::CanBlockShadow => {
                for id in targets {
                    self.cards[id.0 as usize].cur.can_block_shadow = true;
                }
            }
            Act::CantAttackGreater => {
                for id in targets {
                    self.cards[id.0 as usize].cur.cant_attack_unless_greater = true;
                }
            }
            Act::CantBeBlockedGreater => {
                for id in targets {
                    self.cards[id.0 as usize].cur.cant_be_blocked_greater = true;
                }
            }
            Act::CantBlockGreater => {
                for id in targets {
                    self.cards[id.0 as usize].cur.cant_block_unless_greater = true;
                }
            }
            Act::CantBlockAlone => {
                for id in targets {
                    self.cards[id.0 as usize].cur.cant_block_alone = true;
                }
            }
            Act::Goaded => {
                for id in targets {
                    let cur = &mut self.cards[id.0 as usize].cur;
                    cur.must_attack = true;
                    if !cur.static_goaders.contains(&f.controller) {
                        cur.static_goaders.push(f.controller);
                    }
                }
            }
            Act::MustBlock(af) => {
                for id in targets {
                    self.cards[id.0 as usize].cur.must_block.push((af.clone(), f.source, f.controller));
                }
            }
            Act::MustBeBlockedBy(bf, n) => {
                for id in targets {
                    self.cards[id.0 as usize].cur.must_be_blocked_by.push((bf.clone(), *n, f.source, f.controller));
                }
            }
            Act::MustBeBlocked => {
                for id in targets {
                    self.cards[id.0 as usize].cur.must_be_blocked = true;
                }
            }
            Act::CantAttackPlayer(flt, neg, po) => {
                for id in targets {
                    self.cards[id.0 as usize].cur.attack_restr.push((flt.clone(), *neg, f.source, f.controller, *po));
                }
            }
            Act::CantAttackController(po) => {
                for id in targets {
                    self.cards[id.0 as usize].cur.attack_bar.push((f.controller, *po));
                }
            }
            Act::AttackerCond(cd) => {
                for id in targets {
                    self.cards[id.0 as usize].cur.attacker_conds.push((cd.clone(), f.source));
                }
            }
            Act::DefenderCond(cd, unblockable) => {
                for id in targets {
                    self.cards[id.0 as usize].cur.defender_conds.push((cd.clone(), *unblockable, f.source, f.controller));
                }
            }
            Act::CantBeBlockedBy(flt) => {
                for id in targets {
                    self.cards[id.0 as usize].cur.cant_be_blocked_by.push((flt.clone(), f.source, f.controller));
                }
            }
            Act::Cda(p, t) => {
                let pv = self.eval_amount(p, &ctx);
                let tv = self.eval_amount(t, &ctx);
                for id in targets {
                    let cur = &mut self.cards[id.0 as usize].cur;
                    cur.power = pv;
                    cur.toughness = tv;
                }
            }
            Act::SetPTAmount(p, t) => {
                let pv = self.eval_amount(p, &ctx);
                let tv = self.eval_amount(t, &ctx);
                for id in targets {
                    let cur = &mut self.cards[id.0 as usize].cur;
                    cur.power = pv;
                    cur.toughness = tv;
                }
            }
            Act::Animate => {
                for id in targets {
                    let cur = &mut self.cards[id.0 as usize].cur;
                    if !cur.types.has(CardType::Creature) {
                        cur.types.add(CardType::Creature);
                        cur.animated = true;
                    }
                }
            }
            Act::AnimatePT(p, t) => {
                let pv = self.eval_amount(p, &ctx);
                let tv = self.eval_amount(t, &ctx);
                for id in targets {
                    let cur = &mut self.cards[id.0 as usize].cur;
                    if cur.animated {
                        cur.power = pv;
                        cur.toughness = tv;
                    }
                }
            }
            Act::SetPT(p, t) => {
                for id in targets {
                    let cur = &mut self.cards[id.0 as usize].cur;
                    cur.power = *p;
                    cur.toughness = *t;
                }
            }
            Act::SetP(p) => {
                for id in targets {
                    self.cards[id.0 as usize].cur.power = *p;
                }
            }
            Act::SetT(t) => {
                for id in targets {
                    self.cards[id.0 as usize].cur.toughness = *t;
                }
            }
            Act::Switch => {
                for id in targets {
                    let cur = &mut self.cards[id.0 as usize].cur;
                    std::mem::swap(&mut cur.power, &mut cur.toughness);
                }
            }
            Act::ModifyEach(p, t) => {
                let saved = std::mem::take(&mut self.each_current);
                for id in targets {
                    self.each_current = vec![Obj::card(id)];
                    let pv = self.eval_amount(p, &ctx);
                    let tv = self.eval_amount(t, &ctx);
                    let cur = &mut self.cards[id.0 as usize].cur;
                    cur.power += pv;
                    cur.toughness += tv;
                }
                self.each_current = saved;
            }
            Act::SetPTAmountEach(p, t) => {
                let saved = std::mem::take(&mut self.each_current);
                for id in targets {
                    self.each_current = vec![Obj::card(id)];
                    let pv = self.eval_amount(p, &ctx);
                    let tv = self.eval_amount(t, &ctx);
                    let cur = &mut self.cards[id.0 as usize].cur;
                    cur.power = pv;
                    cur.toughness = tv;
                }
                self.each_current = saved;
            }
            Act::Modify(p, t) => {
                let pv = self.eval_amount(p, &ctx);
                let tv = self.eval_amount(t, &ctx);
                for id in targets {
                    let cur = &mut self.cards[id.0 as usize].cur;
                    cur.power += pv;
                    cur.toughness += tv;
                }
            }
            Act::EnchantedMod(p, t, kws) => {
                let pv = self.eval_amount(p, &ctx);
                let tv = self.eval_amount(t, &ctx);
                for &id in &targets {
                    let cur = &mut self.cards[id.0 as usize].cur;
                    cur.power += pv;
                    cur.toughness += tv;
                    for k in kws {
                        // already granted in the abilities layer (Grant of the same keyword): do not count it twice
                        if !cur.kw.has(*k) {
                            cur.add_keyword(*k);
                        }
                    }
                }
                for id in targets {
                    for k in kws {
                        if let Some(a) = crate::carddb::keyword_expansion(*k) {
                            let printed = self.db.def(self.cards[id.0 as usize].def).keywords.has(*k);
                            let already = self.cards[id.0 as usize].cur.granted.iter().any(|g| **g == a);
                            if !printed && !already {
                                self.cards[id.0 as usize].cur.granted.push(std::sync::Arc::new(a));
                                self.cards[id.0 as usize].cur.granted_by.push(id);
                            }
                        }
                    }
                }
            }
        }
    }

    /// What applying `f` would do right now: the objects it affects and the values it uses.
    fn signature(&self, f: &Fx) -> (Vec<CardId>, Vec<i32>) {
        let ctx = EvalCtx::with_controller(f.source, f.controller);
        let vals = match &f.act {
            Act::Cda(p, t) | Act::SetPTAmount(p, t) | Act::Modify(p, t) | Act::EnchantedMod(p, t, _) => vec![self.eval_amount(p, &ctx), self.eval_amount(t, &ctx)],
            Act::ExtraBlocksAmount(a) => vec![self.eval_amount(a, &ctx)],
            _ => vec![],
        };
        (self.affected(f), vals)
    }

    /// CR 613.8: an effect depends on another when applying the other first would change what it affects or does.
    /// Dependent effects wait for the ones they depend on; loops fall back to timestamp order.
    fn order_with_dependencies(&mut self, fx: Vec<Fx>) -> Vec<Fx> {
        if fx.len() < 2 {
            return fx;
        }
        // Only effects where one writes a characteristic another reads can depend on each other; skip the expensive analysis otherwise.
        let reads: Vec<u8> = fx.iter().map(fx_reads).collect();
        let writes: Vec<u8> = fx.iter().map(|f| act_writes(&f.act)).collect();
        let interact = (0..fx.len()).any(|a| (0..fx.len()).any(|b| a != b && writes[b] & reads[a] != 0));
        if !interact {
            return fx;
        }
        let n = fx.len();
        // dep[a][b]: a depends on b
        let mut dep = vec![vec![false; n]; n];
        let snapshot = |g: &Game| -> Vec<Derived> { g.battlefield.iter().map(|id| g.cards[id.0 as usize].cur.clone()).collect() };
        let restore = |g: &mut Game, s: &Vec<Derived>| {
            for (i, id) in g.battlefield.clone().iter().enumerate() {
                g.cards[id.0 as usize].cur = s[i].clone();
            }
        };
        let base = snapshot(self);
        let base_sig: Vec<_> = fx.iter().map(|f| self.signature(f)).collect();
        for b in 0..n {
            self.apply_fx(&fx[b]);
            for a in 0..n {
                if a != b && self.signature(&fx[a]) != base_sig[a] {
                    dep[a][b] = true;
                }
            }
            restore(self, &base);
        }
        if !dep.iter().any(|r| r.iter().any(|x| *x)) {
            return fx;
        }
        let mut done = vec![false; n];
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            let pick = (0..n)
                .filter(|i| !done[*i])
                .find(|&i| (0..n).all(|j| !dep[i][j] || done[j] || mutual(&dep, i, j)))
                .or_else(|| (0..n).find(|i| !done[*i]))
                .unwrap();
            done[pick] = true;
            out.push(fx[pick].clone());
        }
        out
    }
}

fn mutual(dep: &[Vec<bool>], a: usize, b: usize) -> bool {
    dep[a][b] && dep[b][a]
}

fn static_to_fx(effect: &StaticEffect) -> Vec<(Aff, Act)> {
    match effect {
        StaticEffect::Modify { affects, power, toughness, each } => {
            vec![(Aff::Filter(affects.clone().into_filter()), if *each { Act::ModifyEach(power.clone(), toughness.clone()) } else { Act::Modify(power.clone(), toughness.clone()) })]
        }
        StaticEffect::GrantAbility { affects, ability } => vec![(Aff::Filter(affects.clone().into_filter()), Act::GrantAbility(std::sync::Arc::new((**ability).clone())))],
        StaticEffect::GrantKeyword { affects, keyword } => vec![(Aff::Filter(affects.clone().into_filter()), Act::Grant(*keyword))],
        StaticEffect::GrantExiledProtections { affects } => vec![(Aff::Filter(affects.clone().into_filter()), Act::ExiledProtections)],
        StaticEffect::GrantGraveyardProtections { affects } => vec![(Aff::Filter(affects.clone().into_filter()), Act::GraveyardProtections)],
        StaticEffect::ShareKeywordsFromExiledCounter { affects, kind } => vec![(Aff::Filter(affects.clone().into_filter()), Act::ExiledCounterShare(*kind))],
        StaticEffect::GrantNonIdentityProtections { affects } => vec![(Aff::Filter(affects.clone().into_filter()), Act::NonIdentityProtections)],
        StaticEffect::LoseDefenses { affects } => vec![(Aff::Filter(affects.clone().into_filter()), Act::LoseDefenses)],
        StaticEffect::LoseKeyword { affects, keyword } => vec![(Aff::Filter(affects.clone().into_filter()), Act::Remove(*keyword))],
        StaticEffect::RemoveAbilities { affects, except_mana, no_land_mana } => vec![(Aff::Filter(affects.clone().into_filter()), if *except_mana { Act::NoAbilitiesKeepMana } else if *no_land_mana { Act::NoAbilitiesNoLandMana } else { Act::NoAbilities })],
        StaticEffect::SetBasePT { affects, power, toughness } => vec![(Aff::Filter(affects.clone().into_filter()), Act::SetPT(*power, *toughness))],
        StaticEffect::SetBasePTAmount { affects, power, toughness, each } => vec![(Aff::Filter(affects.clone().into_filter()), if *each { Act::SetPTAmountEach(power.clone(), toughness.clone()) } else { Act::SetPTAmount(power.clone(), toughness.clone()) })],
        StaticEffect::SetBasePower { affects, power } => vec![(Aff::Filter(affects.clone().into_filter()), Act::SetP(*power))],
        StaticEffect::SetBaseToughness { affects, toughness } => vec![(Aff::Filter(affects.clone().into_filter()), Act::SetT(*toughness))],
        StaticEffect::CdaPT { power, toughness } => vec![(Aff::Source, Act::Cda(power.clone(), toughness.clone()))],
        StaticEffect::RemoveSupertype { affects, supertype } => vec![(Aff::Filter(affects.clone().into_filter()), Act::RemoveSupertype(*supertype))],
        StaticEffect::AddSupertype { affects, supertype } => vec![(Aff::Filter(affects.clone().into_filter()), Act::AddSupertype(*supertype))],
        StaticEffect::AddType { affects, types, subtypes } => vec![(
            Aff::Filter(affects.clone().into_filter()),
            Act::AddTypes(TypeSet::of(types), subtypes.iter().map(|s| Sym::new(s)).collect()),
        )],
        StaticEffect::SetTypes { affects, types, subtypes } => vec![(
            Aff::Filter(affects.clone().into_filter()),
            Act::SetTypes(TypeSet::of(types), subtypes.iter().map(|s| Sym::new(s)).collect()),
        )],
        StaticEffect::RemoveTypes { affects, types, subtypes } => vec![(
            Aff::Filter(affects.clone().into_filter()),
            Act::RemoveTypes(TypeSet::of(types), subtypes.iter().map(|s| Sym::new(s)).collect()),
        )],
        StaticEffect::AllCreatureNames { affects } => vec![(Aff::Filter(affects.clone().into_filter()), Act::AllCreatureNames)],
        StaticEffect::SetName { affects, name } => vec![(Aff::Filter(affects.clone().into_filter()), Act::SetName(Sym::new(name)))],
        StaticEffect::SetChosenNameAndType { affects } => vec![(Aff::Filter(affects.clone().into_filter()), Act::SetChosenNameAndType)],
        StaticEffect::SetSubtypesOfKind { affects, kind, subtypes } => vec![(Aff::Filter(affects.clone().into_filter()), Act::SetSubtypesKind(*kind, subtypes.iter().map(|s| Sym::new(s)).collect()))],
        StaticEffect::CreatureTypesOfLastExiled { affects, keep } => vec![(Aff::Filter(affects.clone().into_filter()), Act::ExiledCreatureTypes(keep.iter().map(|s| Sym::new(s)).collect()))],
        StaticEffect::SetColorsChosen { affects } => vec![(Aff::Filter(affects.clone().into_filter()), Act::SetColorsChosen)],
        StaticEffect::SetColorsPregame { affects } => vec![(Aff::Filter(affects.clone().into_filter()), Act::SetColorsPregame)],
        StaticEffect::AddColorsChosen { affects } => vec![(Aff::Filter(affects.clone().into_filter()), Act::AddColorsChosen)],
        StaticEffect::SetColors { affects, colors } => vec![(Aff::Filter(affects.clone().into_filter()), Act::SetColors(ColorSet::of(colors)))],
        StaticEffect::AddColors { affects, colors } => vec![(Aff::Filter(affects.clone().into_filter()), Act::AddColors(ColorSet::of(colors)))],
        StaticEffect::EnchantedGets { power, toughness, keywords } => {
            // The granted keywords are a layer 6 effect: they also apply in the abilities layer, so a layer 7c
            // static that reads abilities (Muraganda Petroglyphs) sees them. The EnchantedMod keeps them too, so a
            // keyword removed in layer 6 by an earlier-sorted effect is still re-granted in the P/T pass.
            let mut v: Vec<(Aff, Act)> = keywords.iter().map(|k| (Aff::Attached, Act::Grant(*k))).collect();
            v.push((Aff::Attached, Act::EnchantedMod(power.clone(), toughness.clone(), keywords.clone())));
            v
        }
        StaticEffect::EnchantedAnimate { power, toughness } => vec![(Aff::Attached, Act::Animate), (Aff::Attached, Act::AnimatePT(power.clone(), toughness.clone()))],
        StaticEffect::EnchantedCant { attack, block } => {
            let mut v = vec![];
            if *attack {
                v.push((Aff::Attached, Act::CantAttack));
            }
            if *block {
                v.push((Aff::Attached, Act::CantBlock));
            }
            v
        }
        StaticEffect::ControlEnchanted => vec![(Aff::Attached, Act::Control(PlayerId::MAX))],
        StaticEffect::ControlEnchantedByMonarch => vec![(Aff::Attached, Act::Control(PlayerId::MAX - 1))],
        StaticEffect::CantAttack { affects } => vec![(Aff::Filter(affects.clone().into_filter()), Act::CantAttack)],
        StaticEffect::CombatDamageByToughnessIfGreater { affects } => vec![(Aff::Filter(affects.clone().into_filter()), Act::DmgByToughnessGt)],
        StaticEffect::StationByToughness { affects } => vec![(Aff::Filter(affects.clone().into_filter()), Act::StationByToughness)],
        StaticEffect::CombatDamageByToughness { affects } => vec![(Aff::Filter(affects.clone().into_filter()), Act::DmgByToughness)],
        StaticEffect::LandwalkChosen { affects } => vec![(Aff::Filter(affects.clone().into_filter()), Act::LandwalkChosen)],
        StaticEffect::CombatDamageAbsPower { affects } => vec![(Aff::Filter(affects.clone().into_filter()), Act::DmgAbs)],
        StaticEffect::LethalByPower { affects } => vec![(Aff::Filter(affects.clone().into_filter()), Act::LethalPower)],
        StaticEffect::AssignNoCombatDamage { affects } => vec![(Aff::Filter(affects.clone().into_filter()), Act::NoCombatDamage)],
        StaticEffect::AttackAsThoughNoDefender { affects } => vec![(Aff::Filter(affects.clone().into_filter()), Act::AttackNoDefender)],
        StaticEffect::AttackAsThoughNoDefenderVsAttackers { affects } => vec![(Aff::Filter(affects.clone().into_filter()), Act::AttackNdVsAttackers)],
        StaticEffect::AssignCombatDamageAsUnblocked { affects } => vec![(Aff::Filter(affects.clone().into_filter()), Act::AssignUnblocked)],
        StaticEffect::CantAttackPlayer { affects, defender_controls, if_controls, players_only } => vec![(Aff::Filter(affects.clone().into_filter()), Act::CantAttackPlayer(defender_controls.clone().into_filter(), *if_controls, *players_only))],
        StaticEffect::CantAttackController { affects, players_only } => vec![(Aff::Filter(affects.clone().into_filter()), Act::CantAttackController(*players_only))],
        StaticEffect::MaxBlockers { affects, n } => vec![(Aff::Filter(affects.clone().into_filter()), Act::MaxBlockers(*n))],
        StaticEffect::MaxAttackersOnThis { affects, n } => vec![(Aff::Filter(affects.clone().into_filter()), Act::MaxAttackers(*n))],
        StaticEffect::MinBlockers { affects, n } => vec![(Aff::Filter(affects.clone().into_filter()), Act::MinBlockers(*n))],
        StaticEffect::CantBeBlockedUnlessAllBlock { affects } => vec![(Aff::Filter(affects.clone().into_filter()), Act::BlockAll)],
        StaticEffect::CanBlockExtraAmount { affects, extra } => vec![(Aff::Filter(affects.clone().into_filter()), Act::ExtraBlocksAmount(extra.clone()))],
        StaticEffect::CanBlockExtra { affects, extra } => vec![(Aff::Filter(affects.clone().into_filter()), Act::ExtraBlocks(if *extra == 0 { 255 } else { *extra }))],
        StaticEffect::CantBlock { affects } => vec![(Aff::Filter(affects.clone().into_filter()), Act::CantBlock)],
        StaticEffect::MustAttack { affects } => vec![(Aff::Filter(affects.clone().into_filter()), Act::MustAttack)],
        StaticEffect::MustAttackSource { affects } => vec![(Aff::Filter(affects.clone().into_filter()), Act::MustAttackPermanent(CardId(u32::MAX)))],
        StaticEffect::MustAttackController { affects, enchanted_controller } => {
            vec![(Aff::Filter(affects.clone().into_filter()), Act::MustAttackPlayer(if *enchanted_controller { PlayerId::MAX - 1 } else { PlayerId::MAX }))]
        }
        StaticEffect::AttackAsThoughHaste { affects } => vec![(Aff::Filter(affects.clone().into_filter()), Act::AttackHaste)],
        StaticEffect::CantAttackUnlessOthers { affects, n } => vec![(Aff::Filter(affects.clone().into_filter()), Act::CantAttackOthers(*n))],
        StaticEffect::CantBlockUnlessOthers { affects, n } => vec![(Aff::Filter(affects.clone().into_filter()), Act::CantBlockOthers(*n))],
        StaticEffect::CantAttackAlone { affects } => vec![(Aff::Filter(affects.clone().into_filter()), Act::CantAttackAlone)],
        StaticEffect::CanOnlyAttackAlone { affects } => vec![(Aff::Filter(affects.clone().into_filter()), Act::AttackOnlyAlone)],
        StaticEffect::CantBlockAttackers { affects, attacker } => vec![(Aff::Filter(affects.clone().into_filter()), Act::CantBlockAttackers(attacker.clone().into_filter()))],
        StaticEffect::CantAttackUnlessGreaterPower { affects } => vec![(Aff::Filter(affects.clone().into_filter()), Act::CantAttackGreater)],
        StaticEffect::CantBeBlockedByGreaterPower { affects } => vec![(Aff::Filter(affects.clone().into_filter()), Act::CantBeBlockedGreater)],
        StaticEffect::CantBlockUnlessGreaterPower { affects } => vec![(Aff::Filter(affects.clone().into_filter()), Act::CantBlockGreater)],
        StaticEffect::CantBlockAlone { affects } => vec![(Aff::Filter(affects.clone().into_filter()), Act::CantBlockAlone)],
        StaticEffect::IgnoreLandwalk { affects, only } => vec![(Aff::Filter(affects.clone().into_filter()), Act::IgnoreLandwalk(landwalk_bit(*only)))],
        StaticEffect::CanBlockLandwalk { affects } => vec![(Aff::Filter(affects.clone().into_filter()), Act::CanBlockLandwalk)],
        StaticEffect::CanBlockShadow { affects } => vec![(Aff::Filter(affects.clone().into_filter()), Act::CanBlockShadow)],
        StaticEffect::CantBeSacrificed { affects } => vec![(Aff::Filter(affects.clone().into_filter()), Act::CantBeSacrificed)],
        StaticEffect::CantBeBeamedUp { affects } => vec![(Aff::Filter(affects.clone().into_filter()), Act::CantBeBeamed)],
        StaticEffect::CantPhaseOut { affects } => vec![(Aff::Filter(affects.clone().into_filter()), Act::CantPhaseOut)],
        StaticEffect::ZeroLoyaltyStays { affects } => vec![(Aff::Filter(affects.clone().into_filter()), Act::ZeroLoyaltyStays)],
        StaticEffect::CantBecomeSuspected { affects } => vec![(Aff::Filter(affects.clone().into_filter()), Act::CantBecomeSuspected)],
        StaticEffect::Goaded { affects } => vec![(Aff::Filter(affects.clone().into_filter()), Act::Goaded)],
        StaticEffect::MustBlock { affects, attacker } => vec![(Aff::Filter(affects.clone().into_filter()), Act::MustBlock(attacker.clone()))],
        StaticEffect::MustBeBlockedBy { affects, by, n } => vec![(Aff::Filter(affects.clone().into_filter()), Act::MustBeBlockedBy(by.clone().into_filter(), *n))],
        StaticEffect::CantBeBlockedWhileDefender { affects, cond } => vec![(Aff::Filter(affects.clone().into_filter()), Act::DefenderCond(cond.clone(), true))],
        StaticEffect::CantBlockWhileAttacker { affects, cond } => vec![(Aff::Filter(affects.clone().into_filter()), Act::AttackerCond(cond.clone()))],
        StaticEffect::CantAttackWhileDefender { affects, cond } => vec![(Aff::Filter(affects.clone().into_filter()), Act::DefenderCond(cond.clone(), false))],
        StaticEffect::MustBeBlocked { affects } => vec![(Aff::Filter(affects.clone().into_filter()), Act::MustBeBlocked)],
        StaticEffect::CantBeBlockedBy { affects, by } => vec![(Aff::Filter(affects.clone().into_filter()), Act::CantBeBlockedBy(by.clone().into_filter()))],
        _ => vec![],
    }
}

const R_TYPES: u8 = 1;
const R_COLORS: u8 = 2;
const R_KW: u8 = 4;
const R_CONTROL: u8 = 8;
const R_PT: u8 = 16;
const R_ABIL: u8 = 32;

fn act_writes(a: &Act) -> u8 {
    match a {
        Act::Control(_) => R_CONTROL,
        Act::AddTypes(..) | Act::Animate | Act::SetTypes(..) | Act::RemoveTypes(..) | Act::SetSubtypes(_) | Act::SetSubtypesKind(..) | Act::ExiledCreatureTypes(_) | Act::RemoveLegendary | Act::SetName(_) | Act::SetChosenNameAndType | Act::NoManaCost | Act::AllCreatureNames | Act::AddSupertype(_) | Act::RemoveSupertype(_) => R_TYPES,
        Act::SetColors(_) | Act::SetColorsChosen | Act::SetColorsPregame | Act::AddColorsChosen | Act::AddColors(_) => R_COLORS,
        Act::Grant(_) | Act::Remove(_) | Act::EnchantedMod(_, _, _) => R_KW | R_PT,
        Act::NoAbilities | Act::NoAbilitiesKeepMana | Act::NoAbilitiesNoLandMana | Act::LoseDefenses => R_KW | R_ABIL,
        Act::GrantAbility(_) | Act::ExiledProtections | Act::GraveyardProtections | Act::ExiledCounterShare(_) | Act::NonIdentityProtections | Act::RemoveProtection(_) => R_ABIL,
        Act::CantAttack | Act::DmgByToughness | Act::DmgByToughnessGt | Act::StationByToughness | Act::DmgAbs | Act::LethalPower | Act::LandwalkChosen | Act::NoCombatDamage | Act::AttackNoDefender | Act::AttackNdVsAttackers | Act::AssignUnblocked | Act::CantAttackPlayer(..) | Act::CantAttackController(..) | Act::CantBlock | Act::ExtraBlocks(_) | Act::MaxBlockers(_) | Act::MaxAttackers(_) | Act::MinBlockers(_) | Act::BlockAll | Act::MustAttack | Act::Goaded | Act::CantAttackAlone | Act::AttackOnlyAlone | Act::CantAttackOthers(_) | Act::CantBlockOthers(_) | Act::CantAttackGreater | Act::CantBlockGreater | Act::CantBeBlockedGreater | Act::AttackHaste | Act::CantBlockAlone | Act::CanBlockShadow | Act::CanBlockLandwalk | Act::IgnoreLandwalk(_) | Act::CantBeSacrificed | Act::CantBeBeamed | Act::CantPhaseOut | Act::ZeroLoyaltyStays | Act::CantBecomeSuspected | Act::MustAttackPlayer(_) | Act::MustAttackPermanent(_) | Act::CantBlockAttackers(_) | Act::MustBlock(_) | Act::MustBeBlocked | Act::MustBeBlockedBy(..) | Act::CantBeBlockedBy(_) | Act::DefenderCond(..) | Act::AttackerCond(..) => 0,
        Act::ExtraBlocksAmount(_) => 0,
        Act::Cda(..) | Act::SetPT(..) | Act::SetPTAmount(..) | Act::SetPTAmountEach(..) | Act::AnimatePT(..) | Act::SetP(_) | Act::SetT(_) | Act::Modify(..) | Act::ModifyEach(..) | Act::Switch => R_PT,
    }
}

fn filter_reads(f: &Filter) -> u8 {
    let mut r = 0;
    if !f.types.is_empty() || !f.not_types.is_empty() || !f.supertypes.is_empty() || !f.subtypes.is_empty() || !f.not_subtypes.is_empty() {
        r |= R_TYPES;
    }
    if !f.colors.is_empty() || !f.not_colors.is_empty() || f.colorless || f.multicolored || f.monocolored {
        r |= R_COLORS;
    }
    if f.has_activated_ability {
        r |= R_ABIL;
    }
    if !f.keywords.is_empty() || !f.not_keywords.is_empty() {
        r |= R_KW;
    }
    if f.controller != PlayerFilter::Any {
        r |= R_CONTROL;
    }
    if f.power.is_some() || f.toughness.is_some() {
        r |= R_PT;
    }
    r
}

fn amount_reads(a: &Amount) -> u8 {
    match a {
        Amount::Count { filter, .. } | Amount::DistinctTypes { filter, .. } => filter_reads(filter) | R_TYPES,
        Amount::DistinctTypesUnion { .. } => R_TYPES,
        Amount::DistinctTypesOf { .. } => R_TYPES,
        Amount::DistinctKeywords { filter, .. } => filter_reads(filter) | R_TYPES | R_KW,
        Amount::HighestPower { filter, .. } | Amount::LowestPower { filter, .. } => filter_reads(filter) | R_PT,
        Amount::PowerOf { .. } | Amount::PowerMinusBasePower { .. } | Amount::ToughnessOf { .. } => R_PT,
        Amount::Plus { a, b } | Amount::Minus { a, b } | Amount::Times { a, b } | Amount::Max { a, b } | Amount::Min { a, b } => amount_reads(a) | amount_reads(b),
        Amount::Half { a, .. } => amount_reads(a),
        Amount::PowerOfTwo { of } => amount_reads(of),
        _ => 0,
    }
}

/// What characteristics deciding "who is affected" and "by how much" this effect looks at (every static also depends on its source keeping its abilities).
/// Keywords that only stand for a rules effect ("can't be blocked", "can't block" ...): granted by a resolved spell or
/// ability they are no ability, so "loses all abilities" doesn't remove them (granted by a static ability they are).
fn rules_effect_keyword(k: Keyword) -> bool {
    matches!(k, Keyword::Unblockable | Keyword::CantBlock | Keyword::CantAttack | Keyword::MustAttack | Keyword::CantBeCountered)
}

/// A rules flag (can't block / can't attack, or such a pseudo-keyword) from a resolved effect whose affected set depends on
/// power or toughness: it is applied after the P/T layers instead of in the abilities layer.
fn pt_flag_fx(f: &Fx) -> bool {
    !f.from_static
        && match (&f.act, &f.aff) {
            (Act::CantBlock | Act::CantAttack, Aff::Filter(flt)) => flt.power.is_some() || flt.toughness.is_some() || flt.power_vs.is_some() || flt.toughness_vs.is_some(),
            (Act::Grant(k), Aff::Filter(flt)) if rules_effect_keyword(*k) => flt.power.is_some() || flt.toughness.is_some() || flt.power_vs.is_some() || flt.toughness_vs.is_some(),
            _ => false,
        }
}

fn fx_reads(f: &Fx) -> u8 {
    let mut r = if f.from_static { R_ABIL } else { 0 };
    if let Aff::Filter(flt) = &f.aff {
        r |= filter_reads(flt);
    }
    match &f.act {
        Act::Cda(p, t) | Act::SetPTAmount(p, t) | Act::SetPTAmountEach(p, t) | Act::AnimatePT(p, t) | Act::Modify(p, t) | Act::ModifyEach(p, t) | Act::EnchantedMod(p, t, _) => r |= amount_reads(p) | amount_reads(t),
        Act::ExtraBlocksAmount(a) => r |= amount_reads(a),
        _ => {}
    }
    r
}

/// Bit of a landwalk keyword in `ignore_landwalk` (Plains, Island, Swamp, Mountain, Forest); no keyword = all five.
pub fn landwalk_bit(k: Option<Keyword>) -> u8 {
    match k {
        Some(Keyword::Plainswalk) => 1,
        Some(Keyword::Islandwalk) => 2,
        Some(Keyword::Swampwalk) => 4,
        Some(Keyword::Mountainwalk) => 8,
        Some(Keyword::Forestwalk) => 16,
        _ => 31,
    }
}

/// The non-battlefield zone an ability is granted to cards in (activated: any zone; triggered: hand / graveyard / exile).
fn zone_grant(a: &Ability) -> Option<Zone> {
    match a {
        Ability::Activated { from: Some(z), .. } if *z != Zone::Battlefield => Some(*z),
        Ability::Triggered { from: Some(z), .. } if matches!(z, Zone::Hand | Zone::Graveyard | Zone::Exile) => Some(*z),
        _ => None,
    }
}

/// "unless you control another creature with power 4 or greater" (Rhonas): the condition looks at other permanents' final power / toughness.
fn cond_reads_other_pt(c: &Condition) -> bool {
    match c {
        Condition::Not { c } => cond_reads_other_pt(c),
        Condition::Control { filter, .. } => filter.power.is_some() || filter.toughness.is_some(),
        _ => false,
    }
}
