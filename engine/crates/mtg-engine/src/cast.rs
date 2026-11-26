//! Casting spells and activating abilities (CR 601 / 602), putting triggers on the stack, legal actions.

pub const GRANT_ALT_BASE: u8 = 100;
use crate::decision::*;
use crate::eval::EvalCtx;
use crate::dsl::*;
use crate::ids::*;
use crate::mana::*;
use crate::pay::*;
use crate::state::*;
use crate::types::*;
use smallvec::SmallVec;

/// Ability indices at or above this refer to abilities granted by other permanents (`Derived::granted`).
pub const GRANTED_BASE: u16 = 0x8000;
/// Intrinsic mana abilities of basic land types (CR 305.6): `INTRINSIC_BASE + color index`.
pub const INTRINSIC_BASE: u16 = 0xF000;
/// Virtual mana sources for restricted mana floating in a player's pool: `RESTRICTED_BASE + slot`.
pub const RESTRICTED_BASE: u16 = 0xE000;
/// Virtual mana sources for "pay 1 life: add {C}" (Channel): `LIFE_MANA_BASE + n`, above every restricted-mana slot.
pub const LIFE_MANA_BASE: u16 = 0xEC00;

pub enum Flow {
    Continue,
    Wait,
}

/// Outcome of paying one non-mana cost part.
enum Extra {
    Paid,
    Wait,
    Abort,
    /// The part was replaced (an `Or` cost was resolved to a choice); look at the same index again.
    Retry,
}

#[derive(Clone, Debug)]
pub enum Origin {
    Spell { card: CardId, alt: Option<u8> },
    Ability { card: CardId, ability: u16 },
    Trigger,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Splice cards from hand onto an Arcane (or other matching) spell being cast.
    Splice,
    /// `StaticEffect::OptionalSpellCost`: asked before everything else.
    OptCost,
    /// `StaticEffect::SpellManaForCounters`: the amount of extra mana to pay.
    Bonus,
    /// A kicker cost with `Behold.choose_type` was chosen: ask for the creature type (stored as the spell's `chosen_type`).
    BeholdType,
    Kicker,
    /// Convoke / improvise / delve: choose permanents to tap (or cards to exile) to pay part of the cost.
    Alt,
    Modes,
    X,
    /// `Ability::ChooseNumberOnCast`
    Num,
    /// `Ability::RemoveCountersReduceCost`: how many counters to remove from your creatures (each lowers the generic cost).
    Frenzy,
    /// `Ability::ChooseCreatureTypeOnCast`
    CastType,
    /// `Ability::XColoredDistinct`: pick which colors X is paid with (then `type_next`).
    XColors,
    Targets,
    Extra,
    Pay,
    Finish,
}

#[derive(Clone, Debug)]
pub struct CastCtx {
    pub player: PlayerId,
    pub origin: Origin,
    pub item: StackItem,
    pub stage: Stage,
    pub specs: Vec<TargetSpec>,
    pub spec_idx: usize,
    pub mana: ManaCost,
    pub extra: Vec<Cost>,
    pub extra_idx: usize,
    /// "Target ... of an opponent's choice" with several opponents: the opponent the caster chose to pick the target of
    /// target slot `.0`; `asking_chooser` while that question is open.
    pub target_chooser: Option<(usize, PlayerId)>,
    pub asking_chooser: bool,
    pub remaining: ManaCost,
    pub backup: Option<Box<Game>>,
    pub from: Zone,
    pub modal: Option<(u8, u8, usize)>,
    /// The same mode may be chosen more than once (asked one at a time).
    pub modal_repeat: bool,
    pub what: String,
    pub paid_life: i32,
    /// Which of convoke (0), improvise (1), delve (2) is being asked next.
    pub alt_idx: u8,
    /// Assist progress: 0 not asked, 1 waiting for the helper, 2 waiting for the amount, 3 done.
    pub assist_state: u8,
    pub assist_player: PlayerId,
    /// Which Kicker ability (in order) is being asked about.
    pub kicker_idx: u8,
    /// Hand cards already asked about splicing onto this spell.
    pub splice_asked: Vec<CardId>,
    /// Costs that give up permanents (sacrifice / return) are paid after the mana, so the permanent can still be tapped for mana first.
    pub extra_late: bool,
    /// Cost reduction that applies to the chosen X.
    pub x_red: u32,
    /// Stage to continue with after the optional static spell costs have been asked.
    pub opt_next: Stage,
    /// Index of the next `optional_spell_costs` entry to ask about.
    pub opt_idx: u8,
    /// Stage to continue with after the Behold creature type has been chosen.
    pub type_next: Stage,
    /// A mana source picked to pay, waiting for the player to choose the cards its cost exiles (Sunken Palace).
    pub tap_pick: Option<(CardId, u16, Option<ManaType>, Vec<CardId>)>,
    /// A cost that removes counters of any kind, being paid one counter at a time: (counters still to remove, the permanent
    /// chosen for the next counter, or for all of them when the cost names one permanent).
    pub rm_any: Option<(i32, Option<CardId>)>,
}

impl Game {
    // ------------------------------------------------------------------ timing

    pub fn sorcery_timing(&self, p: PlayerId) -> bool {
        self.active == p && self.step.is_main() && self.stack.is_empty()
    }

    /// A static ability lets `p` cast this spell as though it had flash.
    fn flash_granted(&self, p: PlayerId, card: CardId) -> bool {
        // "you may cast spells this turn as though they had flash" granted to a player by a resolved effect
        for e in &self.effects {
            if let Modif::Static(b) = &e.modif {
                if let StaticEffect::CastAsThoughFlash { filter, whose, .. } = &**b {
                    if e.players.contains(&p) && self.player_filter_src(e.source, *whose, p, e.controller) && self.matches_card(filter, card, e.source, e.controller) {
                        return true;
                    }
                }
            }
        }
        // a card in hand that has flash under a condition ("has flash as long as you control a Desert")
        if self.cards[card.0 as usize].zone == Zone::Hand {
            let d = self.def_of(card);
            for &i in &d.statics {
                if let Ability::Static { effect: StaticEffect::GrantKeyword { affects, keyword: Keyword::Flash }, condition, from: Some(Zone::Hand) } = &d.abilities[i as usize] {
                    if affects.this && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(card, p))) {
                        return true;
                    }
                }
            }
        }
        // the battlefield, plus the card's own "you may cast this as though it had flash" (works from hand)
        let emb = self.emblem_srcs();
        self.battlefield.iter().chain(emb.iter()).chain(std::iter::once(&card)).any(|&src| {
            let sc = &self.cards[src.0 as usize];
            if sc.zone == Zone::Battlefield && sc.cur.no_abilities {
                return false;
            }
            let own = src == card && sc.zone != Zone::Battlefield;
            if own && sc.zone != Zone::Hand {
                return false;
            }
            let ctrl = if own { p } else { self.src_ctrl(src) };
            let sd = self.db.def(sc.def);
            sd.statics.iter().any(|&i| {
                matches!(&sd.abilities[i as usize], Ability::Static { effect: StaticEffect::CastAsThoughFlash { filter, whose, if_targets, .. }, condition, .. }
                    if (!own || filter.this)
                        && if_targets.as_deref().map_or(true, |tf| self.flash_target_exists(p, card, tf))
                        && self.player_filter_src(src, *whose, p, ctrl)
                        && self.matches_card(filter, card, src, ctrl)
                        && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(src, ctrl))))
            })
        })
    }

    /// The spell is being cast out of sorcery timing and the only thing letting it is its own "you may cast this as though it had flash" (no other effect grants flash; Armor of Thorns).
    pub(crate) fn cast_with_own_flash_only(&self, p: PlayerId, card: CardId) -> bool {
        let c = &self.cards[card.0 as usize];
        let d = self.db.def(c.def);
        if d.types.has(CardType::Instant) || c.cur.kw.has(Keyword::Flash) || d.keywords.has(Keyword::Flash) || self.sorcery_timing(p) {
            return false;
        }
        let own = d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::CastAsThoughFlash { filter, .. }, .. } if filter.this));
        if !own {
            return false;
        }
        // any other source (a player-wide effect or another permanent) that grants flash to this card
        for e in &self.effects {
            if let Modif::Static(b) = &e.modif {
                if let StaticEffect::CastAsThoughFlash { filter, whose, .. } = &**b {
                    if e.players.contains(&p) && self.player_filter_src(e.source, *whose, p, e.controller) && self.matches_card(filter, card, e.source, e.controller) {
                        return false;
                    }
                }
            }
        }
        !self.battlefield.iter().any(|&src| {
            let sc = &self.cards[src.0 as usize];
            if sc.cur.no_abilities {
                return false;
            }
            let sd = self.db.def(sc.def);
            sd.statics.iter().any(|&i| {
                matches!(&sd.abilities[i as usize], Ability::Static { effect: StaticEffect::CastAsThoughFlash { filter, whose, .. }, condition, .. }
                    if self.player_filter_src(src, *whose, p, sc.cur.controller)
                        && self.matches_card(filter, card, src, sc.cur.controller)
                        && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(src, sc.cur.controller))))
            })
        })
    }

    /// Does the spell have a legal first target that is a permanent matching `tf` ("as though it had flash if it targets a permanent you control")?
    fn flash_target_exists(&self, p: PlayerId, card: CardId, tf: &Filter) -> bool {
        let (specs, _) = self.spell_specs(card);
        let Some(spec) = specs.first() else { return false };
        self.legal_targets(spec, card, p).iter().any(|o| matches!(o, Obj::Card { id } if self.matches_card(tf, *id, card, p)))
    }

    /// While `card` is being cast out of sorcery timing only through its own `CastAsThoughFlash{if_targets}`, the filter its first target must match.
    pub(crate) fn flash_target_limit(&self, p: PlayerId, card: CardId) -> Option<Filter> {
        let c = &self.cards[card.0 as usize];
        if !matches!(c.zone, Zone::Hand | Zone::Stack) {
            return None;
        }
        let d = self.db.def(c.def);
        if d.types.has(CardType::Instant) || c.cur.kw.has(Keyword::Flash) || d.keywords.has(Keyword::Flash) || self.sorcery_timing(p) {
            return None;
        }
        d.statics.iter().find_map(|&i| match &d.abilities[i as usize] {
            Ability::Static { effect: StaticEffect::CastAsThoughFlash { filter, if_targets: Some(tf), .. }, condition, .. }
                if filter.this && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(card, p))) =>
            {
                Some((**tf).clone())
            }
            _ => None,
        })
    }

    pub fn can_cast_timing(&self, p: PlayerId, card: CardId) -> bool {
        let c = &self.cards[card.0 as usize];
        let d = self.db.def(c.def);
        let instant_speed = d.types.has(CardType::Instant) || c.cur.kw.has(Keyword::Flash) || d.keywords.has(Keyword::Flash);
        instant_speed || self.sorcery_timing(p) || self.flash_granted(p, card)
    }

    // ------------------------------------------------------------------ costs

    /// Alternative mana costs granted to `card` by static abilities (`AlternativeManaCost`): (source, mana string, once per turn). The alt index is `GRANT_ALT_BASE + position`.
    pub(crate) fn granted_alt_costs(&self, p: PlayerId, card: CardId, zone: Zone) -> Vec<(CardId, String, bool, bool, Vec<Cost>, bool)> {
        let mut out = vec![];
        // emblems (command zone) grant alternative costs to their owner too
        let mut srcs: Vec<CardId> = self.battlefield.clone();
        for pl in &self.players {
            srcs.extend(pl.command.iter().copied().filter(|e| !self.cards[e.0 as usize].is_commander));
        }
        for src in srcs {
            let sc = &self.cards[src.0 as usize];
            if sc.cur.no_abilities && sc.zone == Zone::Battlefield {
                continue;
            }
            let sd = self.db.def(sc.def);
            let ctrl = if sc.zone == Zone::Battlefield { sc.cur.controller } else { sc.owner };
            for &i in &sd.statics {
                if let Ability::Static { effect: StaticEffect::AlternativeManaCost { filter, whose, cost, once_per_turn, from, pay_life_mv, extra, exile_after, prowl }, condition, .. } = &sd.abilities[i as usize] {
                    if from.unwrap_or(Zone::Hand) != zone {
                        out.push((src, String::new(), false, false, vec![], false)); // keep positions stable
                        continue;
                    }
                    let ok = self.player_filter_src(src, *whose, p, ctrl)
                        && self.matches_card(filter, card, src, ctrl)
                        && (!*prowl || self.prowl_ok(p, card))
                        && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(src, ctrl)))
                        && !(*once_per_turn && self.alt_grants_used.iter().any(|(s, t)| *s == src && *t == self.turn));
                    out.push((src, if ok { cost.clone() } else { String::new() }, *once_per_turn, *pay_life_mv, self.resolve_wb_mv(card, extra), *exile_after));
                }
            }
        }
        // alternative costs granted to a player by a resolved effect ("you may cast the next spell this turn by paying life ..."): used up by the first spell cast with it
        for e in &self.effects {
            if let Modif::Static(b) = &e.modif {
                if let StaticEffect::AlternativeManaCost { filter, whose, cost, from, pay_life_mv, extra, exile_after, .. } = &**b {
                    if from.unwrap_or(Zone::Hand) != zone {
                        out.push((e.source, String::new(), false, false, vec![], false));
                        continue;
                    }
                    let ok = e.players.contains(&p)
                        && self.player_filter_src(e.source, *whose, p, e.controller)
                        && self.matches_card(&Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() }, card, e.source, e.controller)
                        && !self.alt_grants_used.iter().any(|(s, t)| *s == e.source && *t == self.turn);
                    out.push((e.source, if ok { cost.clone() } else { String::new() }, true, *pay_life_mv, self.resolve_wb_mv(card, extra), *exile_after));
                }
            }
        }
        out
    }

    /// Prowl: a creature `p` controlled dealt combat damage to a player this turn and shares a creature type with `card`.
    fn prowl_ok(&self, p: PlayerId, card: CardId) -> bool {
        let d = self.def_of(card);
        let card_changeling = d.keywords.has(Keyword::Changeling);
        self.combat_damagers_ctl.iter().any(|(id, ctl)| {
            if *ctl != p {
                return false;
            }
            let c = &self.cards[id.0 as usize];
            c.cur.types.has(CardType::Creature) || self.def_of(*id).types.has(CardType::Creature)
        } && {
            let c = &self.cards[id.0 as usize];
            let dd = self.def_of(*id);
            card_changeling || c.cur.kw.has(Keyword::Changeling) || dd.keywords.has(Keyword::Changeling) || d.subtypes.iter().any(|t| c.cur.subtypes.contains(t) || dd.subtypes.contains(t))
        })
    }

    /// `Cost::WaterbendMV` in a granted alternative cost becomes `Waterbend` of the card's mana value.
    fn resolve_wb_mv(&self, card: CardId, extra: &[Cost]) -> Vec<Cost> {
        extra.iter().map(|c| if matches!(c, Cost::WaterbendMV) { Cost::Waterbend { n: self.def_of(card).mana_value as i32 } } else { c.clone() }).collect()
    }

    /// A `once` cost change granted to a player is used up by the spell it applied to.
    fn consume_once_cost_changes(&mut self, p: PlayerId, card: CardId) {
        let mut gone: Vec<u32> = vec![];
        let mut cant_counter = false;
        for e in &self.effects {
            if let Modif::Static(b) = &e.modif {
                if let StaticEffect::CostChange { filter, whose, once: true, .. } = &**b {
                    if e.players.contains(&p) && self.player_filter_src(e.source, *whose, p, e.controller) && self.matches_card(&Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() }, card, e.source, e.controller) {
                        gone.push(e.id);
                    }
                }
                if let StaticEffect::CastAsThoughFlash { filter, whose, once: true, .. } = &**b {
                    if e.players.contains(&p) && self.player_filter_src(e.source, *whose, p, e.controller) && self.matches_card(filter, card, e.source, e.controller) {
                        gone.push(e.id);
                    }
                }
                if let StaticEffect::GrantSpellKeyword { filter, once: true, .. } = &**b {
                    if e.players.contains(&p) && self.matches_card(&Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() }, card, e.source, e.controller) {
                        gone.push(e.id);
                    }
                }
                if let StaticEffect::CantBeCountered { filter, once: true, .. } = &**b {
                    if e.players.contains(&p) && self.matches_card(&Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() }, card, e.source, e.controller) {
                        gone.push(e.id);
                        cant_counter = true;
                    }
                }
            }
        }
        if !gone.is_empty() {
            self.effects.retain(|e| !gone.contains(&e.id));
        }
        if cant_counter {
            self.add_effect(card, p, &[card], Modif::Keyword(Keyword::CantBeCountered), Duration::UntilEndOfTurn);
        }
    }

    /// The grant behind alt index `i` (the card is already off its zone, so the zone it was cast from is given).
    fn granted_alt_costs_at_cast(&self, p: PlayerId, card: CardId, from: Zone, i: u8) -> Option<(CardId, String, bool, bool, Vec<Cost>, bool)> {
        self.granted_alt_costs(p, card, from).get((i - GRANT_ALT_BASE) as usize).cloned()
    }

    /// The mana cost to cast `card` right now (alternative cost, commander tax and cost-changing statics applied).
    pub fn effective_cost(&self, p: PlayerId, card: CardId, alt: Option<u8>) -> ManaCost {
        self.effective_cost_t(p, card, alt, None)
    }

    /// Like `effective_cost`, with the spell's chosen targets: a reduction that depends on the target ("costs {2} less if it targets a
    /// blue spell") is only applied when the target fits. Without targets such a reduction is assumed (so the spell is offered).
    pub fn effective_cost_t(&self, p: PlayerId, card: CardId, alt: Option<u8>, targets: Option<(&[Target], &[u8])>) -> ManaCost {
        self.face_down_cast.set(if alt == Some(FACE_DOWN_ALT) { Some(card) } else { None });
        let prev_probe = self.cost_probe.replace((self.cards[card.0 as usize].zone != Zone::Stack).then_some(card));
        let cost = self.effective_cost_t_inner(p, card, alt, targets);
        self.cost_probe.set(prev_probe);
        self.face_down_cast.set(None);
        cost
    }

    fn effective_cost_t_inner(&self, p: PlayerId, card: CardId, alt: Option<u8>, targets: Option<(&[Target], &[u8])>) -> ManaCost {
        self.x_red.set(0);
        self.red_overflow.set(0);
        self.color_left.borrow_mut().clear();
        let c = &self.cards[card.0 as usize];
        let d = self.db.def(c.def);
        // a spell's own conditional cost reduction is read once the spell has left hand (CR 601.2a: it is moved to the stack before its total
        // cost is determined), so hand-size and similar amounts no longer count it (Orysa, Tide Choreographer with Kiyomaro)
        if c.zone == Zone::Hand && d.abilities.iter().any(|a| matches!(a, Ability::Static { effect: StaticEffect::CostChange { filter, .. }, condition: Some(_), .. } if filter.this)) {
            let mut g2 = self.clone();
            g2.remove_from_zone_lists(card);
            {
                let c2 = &mut g2.cards[card.0 as usize];
                c2.zone = Zone::Stack;
                c2.cast_from = Some(Zone::Hand);
            }
            g2.layers_dirty = true;
            g2.recompute_layers();
            let cost = g2.effective_cost_t(p, card, alt, targets);
            self.x_red.set(g2.x_red.get());
            self.red_overflow.set(g2.red_overflow.get());
            return cost;
        }
        let mut cost = match alt {
            Some(FACE_DOWN_ALT) => ManaCost::parse("{3}").unwrap_or_default(),
            Some(i) if i >= GRANT_ALT_BASE && i < FACE_DOWN_ALT => {
                let zone = if c.zone == Zone::Stack { c.cast_from.unwrap_or(Zone::Hand) } else { c.zone };
                self.granted_alt_costs(p, card, zone).get((i - GRANT_ALT_BASE) as usize).and_then(|g| if g.1 == "own" { Some(d.cost.clone()) } else { ManaCost::parse(&g.1).ok() }).unwrap_or_default()
            }
            Some(i) => {
                let mut m = ManaCost::default();
                if let Some(Ability::AlternativeCost { cost, .. }) =
                    d.abilities.iter().filter(|a| matches!(a, Ability::AlternativeCost { .. })).nth(i as usize)
                {
                    for part in cost {
                        if let Cost::Mana { cost } = part {
                            if let Ok(mc) = ManaCost::parse(cost) {
                                m.shards.extend(mc.shards);
                            }
                        }
                    }
                }
                m
            }
            None => match (&c.exile_cast_cost, c.zone) {
                (Some(s), Zone::Exile) => ManaCost::parse(s).unwrap_or_else(|_| d.cost.clone()),
                _ => d.cost.clone(),
            },
        };
        if c.zone == Zone::Command && c.is_commander && !d.abilities.iter().any(|a| matches!(a, Ability::CommanderTaxInLife)) {
            let tax = self.players[p as usize].commander_cast.iter().find(|(id, _)| *id == card).map_or(0, |(_, n)| *n);
            cost.add_generic((tax * 2) as u8);
        }
        // cost increases / reductions from permanents (and from the spell itself)
        let mut delta = 0i32;
        let mut spend_any_static = false;
        let mut color_delta: Vec<(Color, i32, bool)> = vec![];
        let mut cost_floor = 0u8;
        let mut sources: Vec<CardId> = self.battlefield.clone();
        if !sources.contains(&card) {
            sources.push(card);
        }
        // spells on the stack with a "from the stack" static ("spells that target it cost {2} more": Kaervek's Torch)
        for it in &self.stack {
            if matches!(it.kind, ItemKind::Spell { .. }) && !it.copy && it.source != card && !sources.contains(&it.source) {
                let d = self.db.def(self.cards[it.source.0 as usize].def);
                if d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { from: Some(Zone::Stack), .. })) {
                    sources.push(it.source);
                }
            }
        }
        // emblems (command zone) reduce / raise costs for their owner
        for pl in &self.players {
            for &e in &pl.command {
                let ec = &self.cards[e.0 as usize];
                // a commander's eminence statics (`from: Command`) work from the command zone
                let eminence = ec.is_commander && { let ed = self.db.def(ec.def); ed.statics.iter().any(|&i| matches!(&ed.abilities[i as usize], Ability::Static { from: Some(Zone::Command), .. })) };
                if (!ec.is_commander || eminence) && !sources.contains(&e) {
                    sources.push(e);
                }
            }
        }
        for src in sources {
            let sc = &self.cards[src.0 as usize];
            if sc.cur.no_abilities && sc.zone == Zone::Battlefield {
                continue;
            }
            let sd = self.db.def(sc.def);
            let on_stack = sc.zone == Zone::Stack && src != card;
            let ctrl = if sc.zone == Zone::Battlefield { sc.cur.controller } else if (sc.zone == Zone::Command || on_stack) && src != card { sc.owner } else { p };
            let granted_statics = sc.cur.granted.iter().filter(|g| matches!(***g, Ability::Static { .. })).map(|g| &**g);
            for ab in sd.statics.iter().map(|&i| &sd.abilities[i as usize]).chain(if sc.zone == Zone::Battlefield { Some(granted_statics) } else { None }.into_iter().flatten()) {
                if let Ability::Static { effect: StaticEffect::SpendManaAsAnyType { whose, filter, abilities: false }, condition, .. } = ab {
                    let cond_ok = condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(src, ctrl)));
                    if cond_ok && self.player_filter_src(src, *whose, p, ctrl) && self.matches_card(filter, card, src, ctrl) {
                        spend_any_static = true;
                    }
                }
                if let Ability::Static { effect: StaticEffect::CostMinimum { filter, whose, min }, condition, .. } = ab {
                    if sc.zone == Zone::Battlefield
                        && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(src, ctrl)))
                        && self.player_filter_src(src, *whose, p, ctrl)
                        && self.matches_card(filter, card, src, ctrl)
                    {
                        cost_floor = cost_floor.max(*min);
                    }
                }
                if on_stack && !matches!(ab, Ability::Static { from: Some(Zone::Stack), .. }) {
                    continue;
                }
                if sc.is_commander && src != card {
                    if let Ability::Static { from, .. } = ab {
                        if from.unwrap_or(Zone::Battlefield) != sc.zone {
                            continue;
                        }
                    }
                }
                if let Ability::Static { effect: StaticEffect::CostChange { filter, whose, generic, per, not_from, color, per_caster, flashback, dash, generic_fallback, .. }, condition, .. } = ab {
                    if *not_from == Some(c.zone) {
                        continue;
                    }
                    if *flashback && !self.is_flashback_cast(card, alt) {
                        continue;
                    }
                    if *dash && !self.is_dash_cast(card, alt) {
                        continue;
                    }
                    if src == card && sc.zone != Zone::Battlefield && !filter.this {
                        continue;
                    }
                    if let Some(cond) = condition {
                        // the spell is already on the stack when its cost is determined, so it no longer counts in its owner's graveyard (Scour the Laboratory with flashback)
                        self.costing_card.set(Some(card));
                        let ok = match targets {
                            // before targets are chosen, assume the best case: a target-dependent reduction applies, a target-dependent tax doesn't
                            None if Self::cond_uses_target(cond) => *generic < 0 || self.assume_target_tax.get(),
                            None => self.eval_cond(cond, &EvalCtx::with_controller(src, ctrl)),
                            Some((ts, tidx)) => {
                                let mut e = EvalCtx::with_controller(src, ctrl);
                                e.targets = ts;
                                e.tidx = tidx;
                                self.eval_cond(cond, &e)
                            }
                        };
                        self.costing_card.set(None);
                        if !ok {
                            continue;
                        }
                    }
                    // a spell-target predicate ("spells that target a creature you control cost {2} more") reads the chosen targets;
                    // before they are chosen a tax is assumed only on request and a reduction always
                    let st_of = |f: &Filter| f.spell_has_targets || f.spell_targets.is_some() || f.spell_targets_stack.is_some() || f.spell_targets_you || f.spell_targets_opponent || f.spell_targets_enchanted_player || f.spell_targets_only_players;
                    // an `any_of` list of spell-target predicates ("spells that target you or a permanent you control": Monastery Siege) holds when one alternative does
                    let any_st = !filter.any_of.is_empty() && filter.any_of.iter().all(|f| st_of(f));
                    let uses_st = st_of(filter) || any_st;
                    let st_ok = !uses_st
                        || match targets {
                            Some((ts, _)) => if any_st { filter.any_of.iter().any(|f| self.spell_targets_match(f, ts, src, ctrl)) } else { self.spell_targets_match(filter, ts, src, ctrl) },
                            None => *generic < 0 || self.assume_target_tax.get(),
                        };
                    let f2;
                    let filter = if uses_st {
                        f2 = Filter { spell_has_targets: false, spell_targets: None, spell_targets_stack: None, spell_targets_you: false, spell_targets_opponent: false, spell_targets_enchanted_player: false, spell_targets_only_players: false, any_of: if any_st { vec![] } else { filter.any_of.clone() }, ..filter.clone().into_filter() };
                        &f2
                    } else {
                        filter
                    };
                    if st_ok && self.player_filter_src(src, *whose, p, ctrl) && ({
                        // a spell cast for its bestow cost is an Aura spell (Hero of Iroas's discount)
                        let bestow = alt.map_or(false, |i| self.def_of(card).abilities.iter().filter(|a| matches!(a, Ability::AlternativeCost { .. })).nth(i as usize).map_or(false, |a| matches!(a, Ability::AlternativeCost { effects: Some(e), .. } if matches!(e.as_slice(), [Effect::Nothing]))));
                        if bestow && filter.subtypes.iter().any(|s| s == "Aura") {
                            let mut f2 = filter.clone();
                            f2.subtypes.retain(|s| s != "Aura");
                            self.matches_card(&f2, card, src, ctrl)
                        } else {
                            self.matches_card(filter, card, src, ctrl)
                        }
                    }) {
                        let k = per.as_ref().map_or(1, |a| {
                            let mut e = EvalCtx::with_controller(src, if *per_caster { p } else { ctrl });
                            // "costs {2} less for each X" (an any-number additional cost): X is the spell's chosen count
                            e.x = self.cards[card.0 as usize].x;
                            match targets {
                                Some((ts, tidx)) => {
                                    e.targets = ts;
                                    e.tidx = tidx;
                                }
                                // before targets are chosen: assume one target
                                None if matches!(a, Amount::SpellTargetCount { .. }) => return 1,
                                None => {}
                            }
                            self.costing_card.set(Some(card));
                            let v = self.eval_amount(a, &e);
                            self.costing_card.set(None);
                            v
                        });
                        match color {
                            Some(col) => color_delta.push((*col, *generic * k, *generic_fallback)),
                            None => delta += *generic * k,
                        }
                    }
                }
            }
        }
        // cost changes granted to a player by a resolved effect ("the next spell you cast this turn costs {1} less")
        let mut free_now = false;
        for e in &self.effects {
            if let Modif::Static(b) = &e.modif {
                if let StaticEffect::CostChange { filter, whose, generic, per, free, .. } = &**b {
                    if e.players.contains(&p) && self.player_filter_src(e.source, *whose, p, e.controller) && self.matches_card(&Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() }, card, e.source, e.controller) {
                        if *free {
                            free_now = true;
                        }
                        let k = per.as_ref().map_or(1, |a| self.eval_amount(a, &EvalCtx::with_controller(e.source, e.controller)));
                        delta += *generic * k;
                    }
                    // granted to permanents ("target creature gains: spells cost {2} more"): works as if printed on them
                    if e.players.is_empty() {
                        for (gid, _) in e.affects.iter() {
                            let gc = &self.cards[gid.0 as usize];
                            if gc.zone != Zone::Battlefield {
                                continue;
                            }
                            let gctrl = gc.cur.controller;
                            if self.player_filter_src(*gid, *whose, p, gctrl) && self.matches_card(filter, card, *gid, gctrl) {
                                let k = per.as_ref().map_or(1, |a| self.eval_amount(a, &EvalCtx::with_controller(*gid, gctrl)));
                                delta += *generic * k;
                            }
                        }
                    }
                }
            }
        }
        // blitz costs cost {1} less for each time the caster cast their commander from the command zone
        if let Some(i) = alt {
            if i >= GRANT_ALT_BASE && i < FACE_DOWN_ALT {
                let zone = if c.zone == Zone::Stack { c.cast_from.unwrap_or(Zone::Hand) } else { c.zone };
                if self.granted_alt_costs(p, card, zone).get((i - GRANT_ALT_BASE) as usize).map_or(false, |g| g.4.iter().any(|x| matches!(x, Cost::Blitz))) {
                    delta -= self.players[p as usize].commander_cast.iter().map(|(_, n)| *n as i32).sum::<i32>();
                }
            }
        }
        // Mavinda: a graveyard spell cast through the grant costs more unless it targets a creature its caster controls
        if c.zone == Zone::Graveyard {
            if let Some(tax) = self.gy_card_riders.iter().find(|(cid, z, t, _, tx)| *tx > 0 && *t == self.turn && *cid == card && *z == c.zcc).map(|r| r.4) {
                if self.gy_card_grants.iter().any(|(pl, t, cid, z)| *pl == p && *t == self.turn && *cid == card && *z == c.zcc) {
                    let own_creature_target = match targets {
                        Some((ts, _)) => ts.iter().any(|t| matches!(t.obj, Obj::Card { id } if self.cards[id.0 as usize].zone == Zone::Battlefield && self.cards[id.0 as usize].cur.controller == p && self.cards[id.0 as usize].cur.types.has(CardType::Creature))),
                        None => !self.assume_target_tax.get() && d.abilities.iter().any(|a| matches!(a, Ability::Spell { targets, .. } if !targets.is_empty())),
                    };
                    if !own_creature_target {
                        delta += tax as i32;
                    }
                }
            }
        }
        // "it has affinity for artifacts" (granted with the exile-cast permission)
        if c.zone == Zone::Exile && c.play_extra > 0 {
            delta += c.play_extra as i32;
        }
        if c.zone == Zone::Exile {
            if let Some(af) = c.play_affinity.clone() {
                let n = self.battlefield.iter().filter(|&&id| self.matches_card(&af, id, card, p)).count();
                delta -= n as i32;
            }
        }
        // undaunted granted to cards that aren't on the battlefield ("creature cards exiled with this have undaunted": Durnan of the Yawning Portal)
        let granted_undaunted = c.zone != Zone::Battlefield
            && self.battlefield.iter().any(|&sid| {
                let sc2 = &self.cards[sid.0 as usize];
                !sc2.cur.no_abilities
                    && self.db.def(sc2.def).statics.iter().any(|&i| {
                        matches!(&self.db.def(sc2.def).abilities[i as usize], Ability::Static { effect: StaticEffect::GrantKeyword { affects, keyword: Keyword::Undaunted, .. }, .. }
                            if self.matches_card(&affects.clone().into_filter(), card, sid, sc2.cur.controller))
                    })
            });
        if d.keywords.has(Keyword::Undaunted) || c.cur.kw.has(Keyword::Undaunted) || granted_undaunted || (c.zone == Zone::Hand && self.spell_alt_keywords(p, card).has(Keyword::Undaunted)) {
            delta -= self.opponents(p).len() as i32;
        }
        // cast from exile without paying its mana cost (nonland cards; additional costs still apply): cost increases (Aven Interrupter's {2}) still count
        if c.zone == Zone::Graveyard && self.gy_free_grants.iter().any(|(pl, t, cid, z)| *pl == p && *t == self.turn && *cid == card && *z == c.zcc) {
            cost.shards.clear();
        }
        if free_now || (c.zone == Zone::Library && self.lib_free_grant(p, card)) {
            cost.shards.clear();
        }
        if (c.play_free || (c.free_turn == self.turn && c.free_via.is_some()) || (c.zone == Zone::Exile && self.play_free_perm(p, card))) && c.zone == Zone::Exile {
            cost.shards.clear();
        }
        // reductions of coloured mana ("costs {R} less"): remove that many coloured shards of the colour
        for (col, change, fallback) in color_delta {
            // a cost increase of a colour adds that coloured mana ("costs {W} more")
            if change > 0 {
                for _ in 0..change {
                    cost.shards.push(Shard::Color { c: col });
                }
                continue;
            }
            let mut reduce = -change;
            if reduce > 0 {
                cost.shards.retain(|s| {
                    if reduce > 0 && matches!(s, Shard::Color { c } if *c == col) {
                        reduce -= 1;
                        false
                    } else {
                        true
                    }
                });
            }
            // a hybrid symbol that can be paid with that colour is reduced by it too ({R/W} paid with {R}; Ragemonger, Edgewalker)
            if reduce > 0 {
                cost.shards.retain(|s| {
                    if reduce > 0 && matches!(s, Shard::Hybrid { a, b } if *a == col || *b == col) || reduce > 0 && matches!(s, Shard::HybridGeneric { c } | Shard::HybridColorless { c } if *c == col) {
                        reduce -= 1;
                        false
                    } else {
                        true
                    }
                });
            }
            // the rest of a coloured reduction with no such coloured mana left comes off the generic part
            if fallback && reduce > 0 {
                delta -= reduce;
            } else if reduce > 0 {
                self.color_left.borrow_mut().push((col, reduce));
            }
        }
        if delta > 0 {
            cost.add_generic(delta as u8);
        } else if delta < 0 {
            let mut reduce = -delta;
            let had_x = cost.has_x();
            for s in cost.shards.iter_mut() {
                if let Shard::Generic { n } = s {
                    let r = (*n as i32).min(reduce);
                    *n -= r as u8;
                    reduce -= r;
                }
            }
            // a generic reduction also applies to the {2} half of a {2/W} symbol paid with generic mana
            for s in cost.shards.iter_mut() {
                if reduce > 0 {
                    if let Shard::HybridGeneric { .. } = s {
                        let r = reduce.min(2);
                        *s = Shard::Generic { n: (2 - r) as u8 };
                        reduce -= r;
                    }
                }
            }
            cost.shards.retain(|s| !matches!(s, Shard::Generic { n: 0 }));
            self.x_red.set(if had_x { reduce.max(0) as u32 } else { 0 });
            self.red_overflow.set(if had_x { 0 } else { reduce.max(0) as u32 });
        }
        // cost minimum (Trinisphere): the shortfall is generic mana; with {X} the shortfall also comes off X, so the total is max(cost, floor)
        if cost_floor > 0 {
            let have = cost.mana_value(0);
            if have < cost_floor as u32 {
                let add = cost_floor as u32 - have;
                cost.add_generic(add as u8);
                if cost.has_x() {
                    self.x_red.set(self.x_red.get() + add);
                }
            }
        }
        // "you may spend mana as though it were mana of any type" to cast this exiled card
        let gy_any = c.zone == Zone::Graveyard && self.gy_play_grants.iter().any(|(pl, ow, t, f, src, any)| *any && *pl == p && *ow == c.owner && *t == self.turn && self.matches_card(&Filter { controller: PlayerFilter::Any, ..f.clone() }, card, *src, p));
        let lib_any = c.zone == Zone::Library && self.lib_play_grants.iter().any(|(pl, ow, t)| *pl == p && *ow == c.owner && *t == self.turn);
        let one_spell_any = self.players[p as usize].any_type_turn == self.turn && self.turn > 0;
        if c.spend_any || spend_any_static || gy_any || lib_any || one_spell_any {
            for s in cost.shards.iter_mut() {
                if matches!(s, Shard::Color { .. } | Shard::Colorless | Shard::Hybrid { .. }) {
                    *s = Shard::Generic { n: 1 };
                }
            }
        }
        cost
    }

    /// Is the card being cast for its flashback cost: a printed `AlternativeCost` from the graveyard on a card whose text has a Flashback line?
    /// The spell is cast for a printed dash alternative cost (Warbringer).
    fn is_dash_cast(&self, card: CardId, alt: Option<u8>) -> bool {
        let c = &self.cards[card.0 as usize];
        let d = self.db.def(c.def);
        let Some(i) = alt else { return false };
        if i >= GRANT_ALT_BASE {
            return false;
        }
        let alt_ok = matches!(d.abilities.iter().filter(|a| matches!(a, Ability::AlternativeCost { .. })).nth(i as usize), Some(Ability::AlternativeCost { from: None, .. }));
        alt_ok && d.oracle_text.to_lowercase().lines().any(|l| l.trim().starts_with("dash"))
    }

    fn is_flashback_cast(&self, card: CardId, alt: Option<u8>) -> bool {
        let c = &self.cards[card.0 as usize];
        let d = self.db.def(c.def);
        let Some(i) = alt else { return false };
        if i >= GRANT_ALT_BASE {
            return false;
        }
        let from_gy = matches!(d.abilities.iter().filter(|a| matches!(a, Ability::AlternativeCost { .. })).nth(i as usize), Some(Ability::AlternativeCost { from: Some(Zone::Graveyard), .. }));
        from_gy && d.oracle_text.to_lowercase().lines().any(|l| l.split(',').any(|part| part.trim().starts_with("flashback")))
    }

    /// A card's own "costs {1} less" static also reduces its hand-activated abilities (channel: Eiganjo, Seat of the Empire).
    fn has_sac_self_cost(a: &Ability) -> bool {
        matches!(a, Ability::Activated { cost, .. } if cost.iter().any(|c| matches!(c, Cost::SacrificeSelf | Cost::ExileGrantSource)))
    }

    /// Can the controller of this creature boast with it twice each turn (a `BoastTwice` permanent they control, during their own turn: Birgi, God of Storytelling)?
    pub(crate) fn boast_twice(&self, card: CardId) -> bool {
        let owner = self.cards[card.0 as usize].cur.controller;
        if self.active != owner {
            return false;
        }
        self.battlefield.iter().any(|&b| {
            let sc = &self.cards[b.0 as usize];
            if sc.cur.controller != owner || sc.cur.no_abilities {
                return false;
            }
            let sd = self.db.def(sc.def);
            sd.statics.iter().any(|&i| matches!(&sd.abilities[i as usize], Ability::Static { effect: StaticEffect::BoastTwice, .. }))
        })
    }

    fn is_boast_ability(a: &Ability) -> bool {
        matches!(a, Ability::Activated { limit_per_turn: Some(1), condition: Some(c), .. } if format!("{c:?}").contains("attacked_this_turn: true"))
    }

    fn is_foretell_ability(a: &Ability) -> bool {
        matches!(a, Ability::Activated { effects, .. } if effects.iter().any(|e| matches!(e, Effect::MarkExiled { mark: ExileMark::Foretold })))
    }

    fn is_plot_ability(a: &Ability) -> bool {
        matches!(a, Ability::Activated { effects, .. } if effects.iter().any(|e| matches!(e, Effect::MarkExiled { mark: ExileMark::Plotted })))
    }

    /// The additional life a spell cast by `p` with these targets costs from `ExtraLifeToTargetSpells` statics (Terror of the Peaks).
    fn targeting_spell_life(&self, p: PlayerId, targets: &[Target]) -> i32 {
        let mut total = 0;
        for &src in &self.battlefield {
            let sc = &self.cards[src.0 as usize];
            if sc.cur.no_abilities {
                continue;
            }
            let sd = self.db.def(sc.def);
            let ctrl = sc.cur.controller;
            for &i in &sd.statics {
                if let Ability::Static { effect: StaticEffect::ExtraLifeToTargetSpells { affects, whose, life }, condition, .. } = &sd.abilities[i as usize] {
                    let e = EvalCtx::with_controller(src, ctrl);
                    if condition.as_ref().map_or(false, |cd| !self.eval_cond(cd, &e)) {
                        continue;
                    }
                    if !self.player_filter_src(src, *whose, p, ctrl) {
                        continue;
                    }
                    if targets.iter().any(|t| match t.obj { Obj::Card { id: c } => self.cards[c.0 as usize].zcc == t.zcc && self.battlefield.contains(&c) && self.matches_card(affects, c, src, ctrl), _ => false }) {
                        total += *life;
                    }
                }
            }
        }
        total
    }

    /// Equip, not Reconfigure: a creature card's attach ability is its reconfigure ability (Bronzeplate Boar), which isn't an equip ability.
    fn is_equip_for(&self, card: CardId, a: &Ability) -> bool {
        Self::is_equip_ability(a) && !self.def_of(card).types.has(CardType::Creature)
    }

    fn is_equip_ability(a: &Ability) -> bool {
        matches!(a, Ability::Activated { effects, loyalty: None, .. } if matches!(effects.first(), Some(Effect::Attach { what: Sel::This, .. })))
    }

    /// The change to an activated ability's generic cost from `AbilityCostChange { target: Some(..) }` statics, once its first target is known.
    /// With `target: None` (before targets are chosen) it assumes the best case: the most negative change over the permanents the player controls.
    fn target_ability_cost_delta(&self, p: PlayerId, card: CardId, equip: bool, target: Option<CardId>) -> i32 {
        let Some(tc) = target else {
            return self.battlefield.iter().copied().filter(|t| self.cards[t.0 as usize].cur.controller == p).map(|t| self.target_ability_cost_delta(p, card, equip, Some(t))).min().unwrap_or(0).min(0);
        };
        let tgt = [Target { obj: Obj::card(tc), zcc: self.cards[tc.0 as usize].zcc }];
        let tidx = [0u8];
        let mut delta = 0i32;
        for &src in &self.battlefield {
            let sc = &self.cards[src.0 as usize];
            if sc.cur.no_abilities {
                continue;
            }
            let sd = self.db.def(sc.def);
            let ctrl = sc.cur.controller;
            for &i in &sd.statics {
                if let Ability::Static { effect: StaticEffect::AbilityCostChange { filter, whose, generic, per, equip_only, target: Some(tf), first_creature_target_per_turn, .. }, condition, .. } = &sd.abilities[i as usize] {
                    if *equip_only && !equip {
                        continue;
                    }
                    if *first_creature_target_per_turn && self.creature_tgt_act_log.contains(&p) {
                        continue;
                    }
                    let e = EvalCtx::with_controller(src, ctrl);
                    if condition.as_ref().map_or(false, |cd| !self.eval_cond(cd, &e)) {
                        continue;
                    }
                    if self.player_filter_src(src, *whose, p, ctrl) && self.matches_card(filter, card, src, ctrl) && self.matches_card(tf, tc, src, ctrl) {
                        let mut et = EvalCtx::with_controller(src, ctrl);
                        et.targets = &tgt;
                        et.tidx = &tidx;
                        delta += *generic * per.as_ref().map_or(1, |a| self.eval_amount(a, &et));
                    }
                }
            }
        }
        delta
    }

    /// What unlocking a door of a Room costs `p`: the door's mana cost with the generic part changed by `UnlockCostChange` statics (Inquisitive Glimmer).
    pub(crate) fn unlock_cost(&self, p: PlayerId, card: CardId, door: u8) -> ManaCost {
        let d = self.db.def(self.cards[card.0 as usize].def);
        let mut cost = if door == 1 { d.cost.clone() } else { d.other_face.map(|b| self.db.def(b).cost.clone()).unwrap_or_default() };
        let mut delta = 0i32;
        for &src in &self.battlefield {
            let sc = &self.cards[src.0 as usize];
            if sc.cur.no_abilities {
                continue;
            }
            let sd = self.db.def(sc.def);
            let ctrl = sc.cur.controller;
            for &i in &sd.statics {
                if let Ability::Static { effect: StaticEffect::UnlockCostChange { whose, generic }, condition, .. } = &sd.abilities[i as usize] {
                    if condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(src, ctrl))) && self.player_filter_src(src, *whose, p, ctrl) {
                        delta += *generic;
                    }
                }
            }
        }
        if delta > 0 {
            cost.add_generic(delta.min(250) as u8);
        } else if delta < 0 {
            let mut reduce = -delta;
            for sh in cost.shards.iter_mut() {
                if let Shard::Generic { n } = sh {
                    let r = (*n as i32).min(reduce);
                    *n -= r as u8;
                    reduce -= r;
                }
            }
            cost.shards.retain(|s| !matches!(s, Shard::Generic { n: 0 }));
        }
        cost
    }

    /// "You may spend mana as though it were mana of any color to activate abilities of permanents ..." (`SpendManaAsAnyType.abilities`): colored costs become generic.
    fn anyfy_ability_mana(&self, p: PlayerId, card: CardId, mana: &mut ManaCost) {
        if self.cards[card.0 as usize].zone != Zone::Battlefield {
            return;
        }
        let any = self.battlefield.iter().any(|&src| {
            let sc = &self.cards[src.0 as usize];
            if sc.cur.no_abilities {
                return false;
            }
            let sd = self.db.def(sc.def);
            let ctrl = sc.cur.controller;
            sd.statics.iter().any(|&i| {
                matches!(&sd.abilities[i as usize], Ability::Static { effect: StaticEffect::SpendManaAsAnyType { whose, filter, abilities: true }, condition, .. }
                    if condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(src, ctrl)))
                        && self.player_filter_src(src, *whose, p, ctrl)
                        && self.matches_card(&Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() }, card, src, ctrl)
                        && self.cards[card.0 as usize].cur.controller == p)
            })
        });
        if any {
            for s in mana.shards.iter_mut() {
                if matches!(s, Shard::Color { .. } | Shard::Colorless | Shard::Hybrid { .. }) {
                    *s = Shard::Generic { n: 1 };
                }
            }
        }
    }

    fn reduce_hand_ability_cost(&self, p: PlayerId, card: CardId, mana: &mut ManaCost, equip: bool, sac_self: bool, boast: bool, foretell: bool, once: bool, plot: bool, power_up: bool) {
        if equip && self.equip_first_free(p) {
            mana.shards.clear();
        }
        self.anyfy_ability_mana(p, card, mana);
        let c = &self.cards[card.0 as usize];
        // statics of permanents that change what activating abilities of other (or their own) permanents costs
        let mut abl_delta = 0i32;
        let mut min_one = false;
        let mut removals: Vec<ManaCost> = vec![];
        let mut make_free = false;
        if matches!(c.zone, Zone::Battlefield | Zone::Hand | Zone::Graveyard | Zone::Exile) {
            // a card's own off-battlefield static ("this ability costs {1} less", from Graveyard) counts too
            let own: Vec<CardId> = if c.zone != Zone::Battlefield { vec![card] } else { vec![] };
            let emblems = self.emblem_srcs();
            for &src in self.battlefield.iter().chain(own.iter()).chain(emblems.iter()) {
                let sc = &self.cards[src.0 as usize];
                if sc.cur.no_abilities {
                    continue;
                }
                let sd = self.db.def(sc.def);
                let ctrl = if sc.zone == Zone::Command { self.src_ctrl(src) } else { sc.cur.controller };
                // printed statics, plus statics granted to a permanent (Firion's token copies: "this Equipment's equip abilities cost {2} less")
                let granted = if sc.zone == Zone::Battlefield { Some(sc.cur.granted.iter().map(|g| &**g)) } else { None };
                for ab in sd.statics.iter().map(|&i| &sd.abilities[i as usize]).chain(granted.into_iter().flatten()) {
                    if let Ability::Static { effect: StaticEffect::AbilityCostChange { filter, whose, generic, per, min_one: m1, equip_only, target, remove, sac_self_only, boast_only, foretell_only, once_only, free, first_per_turn, plot_only, power_up_only, first_power_up_per_turn, in_zone, first_cycle_per_turn, .. }, condition, .. } = ab {
                        if in_zone.map_or(false, |z| z != c.zone) {
                            continue;
                        }
                        if *first_cycle_per_turn && self.players[p as usize].cycled_turn > 0 {
                            continue;
                        }
                        // a foretell discount on a card in hand never applies to that same card (Cosmos Charger: only while it is on the battlefield)
                        if *foretell_only && src == card && c.zone == Zone::Hand {
                            continue;
                        }
                        if (*boast_only && !boast) || (*foretell_only && !foretell) || (*plot_only && !plot) || (*once_only && !once) || (*first_per_turn && self.artifact_act_log.contains(&p)) || (*power_up_only && !power_up) || (*first_power_up_per_turn && self.power_up_act_log.contains(&p)) {
                            continue;
                        }
                        if *equip_only && !equip {
                            continue;
                        }
                        if *sac_self_only && !sac_self {
                            continue;
                        }
                        // settled once the targets are chosen
                        if target.is_some() {
                            continue;
                        }
                        let e = EvalCtx::with_controller(src, ctrl);
                        if condition.as_ref().map_or(false, |cd| !self.eval_cond(cd, &e)) {
                            continue;
                        }
                        // hand activations (cycling etc.) are only reduced by filters that name an ability keyword
                        // (an empty filter, "abilities your opponents activate", taxes every activation from any zone)
                        if in_zone.is_none() && c.zone != Zone::Battlefield && !(filter.this && src == card) && filter.keywords.is_empty() && filter.has_keyword_word.is_none() && *filter != (Filter::default()).into() {
                            continue;
                        }
                        if self.player_filter_src(src, *whose, p, ctrl) && self.matches_card(filter, card, src, ctrl) {
                            if *free {
                                make_free = true;
                                continue;
                            }
                            if let Some(rm) = remove {
                                if let Ok(rc) = ManaCost::parse(rm) {
                                    removals.push(rc);
                                }
                                continue;
                            }
                            abl_delta += *generic * per.as_ref().map_or(1, |a| self.eval_amount(a, &e));
                            min_one |= *m1;
                        }
                    }
                }
            }
        }
        if make_free {
            mana.shards.clear();
            return;
        }
        for rc in &removals {
            for sh in &rc.shards {
                match sh {
                    Shard::Generic { n } => {
                        let mut reduce = *n as i32;
                        for s2 in mana.shards.iter_mut() {
                            if let Shard::Generic { n: g } = s2 {
                                let r = (*g as i32).min(reduce);
                                *g -= r as u8;
                                reduce -= r;
                            }
                        }
                    }
                    other => {
                        if let Some(pos) = mana.shards.iter().position(|s2| s2 == other) {
                            mana.shards.remove(pos);
                        }
                    }
                }
            }
            mana.shards.retain(|s| !matches!(s, Shard::Generic { n: 0 }));
        }
        if abl_delta != 0 {
            if abl_delta > 0 {
                mana.add_generic(abl_delta.min(250) as u8);
            } else {
                let had = mana.shards.len();
                let had_x = mana.has_x();
                let mut reduce = -abl_delta;
                if min_one && had > 0 {
                    // keep at least one mana: the reduction can't take the total mana value to zero
                    let mv: i32 = mana.shards.iter().map(|s| if let Shard::Generic { n } = s { *n as i32 } else { 1 }).sum();
                    reduce = reduce.min((mv - 1).max(0));
                }
                for sh in mana.shards.iter_mut() {
                    if let Shard::Generic { n } = sh {
                        let r = (*n as i32).min(reduce);
                        *n -= r as u8;
                        reduce -= r;
                    }
                }
                mana.shards.retain(|s| !matches!(s, Shard::Generic { n: 0 }));
                // what is left of the reduction comes off {X} once X is chosen (Training Grounds)
                if had_x && reduce > 0 {
                    self.x_red.set(self.x_red.get() + reduce as u32);
                }
            }
        }
        if c.zone != Zone::Hand {
            return;
        }
        let sd = self.db.def(c.def);
        let mut delta = 0i32;
        for &i in &sd.statics {
            if let Ability::Static { effect: StaticEffect::CostChange { filter, generic, per, .. }, condition, .. } = &sd.abilities[i as usize] {
                if !filter.this || *generic >= 0 {
                    continue;
                }
                let e = EvalCtx::with_controller(card, p);
                if condition.as_ref().map_or(false, |cd| !self.eval_cond(cd, &e)) {
                    continue;
                }
                delta += *generic * per.as_ref().map_or(1, |a| self.eval_amount(a, &e));
            }
        }
        let mut reduce = -delta;
        for sh in mana.shards.iter_mut() {
            if let Shard::Generic { n } = sh {
                let r = (*n as i32).min(reduce);
                *n -= r as u8;
                reduce -= r;
            }
        }
        mana.shards.retain(|s| !matches!(s, Shard::Generic { n: 0 }));
    }

    fn generic_of(m: &ManaCost) -> u32 {
        m.shards.iter().map(|s| if let Shard::Generic { n } = s { *n as u32 } else { 0 }).sum()
    }

    fn cond_uses_target(c: &Condition) -> bool {
        match c {
            Condition::TargetIs { .. } => true,
            Condition::Not { c } => Self::cond_uses_target(c),
            Condition::All { cs } | Condition::AnyOf { cs } => cs.iter().any(Self::cond_uses_target),
            Condition::ForSomePlayer { cond, .. } => Self::cond_uses_target(cond),
            // "costs {3} less if it targets a creature whose controller has eight or more cards in their graveyard": an amount that reads the target
            Condition::Compare { .. } => format!("{c:?}").contains("Target {"),
            _ => false,
        }
    }

    // ------------------------------------------------------------------ targets

    /// A static says `controller`'s spell or ability may target `target` as though it didn't have hexproof (`ward_only`: only whether ward is ignored).
    pub(crate) fn hexproof_ignored(&self, controller: PlayerId, target: Obj, ward_only: bool) -> bool {
        // a resolved effect granted to `controller` (Detection Tower)
        let granted = self.effects.iter().any(|e| {
            e.players.contains(&controller)
                && matches!(&e.modif, Modif::Static(b) if match &**b {
                    StaticEffect::IgnoreHexproof { affects, players, ignore_ward, .. } if !ward_only || *ignore_ward => match target {
                        Obj::Card { id } => self.matches_card(affects, id, e.source, e.controller),
                        Obj::Player { id } => *players && id != e.controller,
                        _ => false,
                    },
                    _ => false,
                })
        });
        granted || self.battlefield.iter().any(|&sid| {
            let sc = &self.cards[sid.0 as usize];
            if sc.cur.no_abilities {
                return false;
            }
            let sd = self.db.def(sc.def);
            sd.statics.iter().any(|&i| {
                let Ability::Static { effect: StaticEffect::IgnoreHexproof { affects, players, own_only, ignore_ward }, condition, .. } = &sd.abilities[i as usize] else { return false };
                if (ward_only && !*ignore_ward) || (*own_only && sc.cur.controller != controller) {
                    return false;
                }
                if condition.as_ref().map_or(false, |cd| !self.eval_cond(cd, &EvalCtx::with_controller(sid, sc.cur.controller))) {
                    return false;
                }
                match target {
                    Obj::Card { id } => self.matches_card(affects, id, sid, sc.cur.controller),
                    Obj::Player { id } => *players && id != sc.cur.controller,
                    _ => false,
                }
            })
        })
    }

    /// A resolved effect says `controller` may target this permanent as though it didn't have shroud.
    pub(crate) fn shroud_ignored(&self, controller: PlayerId, target: CardId) -> bool {
        self.effects.iter().any(|e| {
            e.players.contains(&controller)
                && matches!(&e.modif, Modif::Static(b) if matches!(&**b, StaticEffect::IgnoreShroud { affects } if self.matches_card(affects, target, e.source, e.controller)))
        })
    }

    /// A permanent says cards in graveyards can't be targeted.
    pub(crate) fn graveyard_untargetable(&self) -> bool {
        self.battlefield.iter().any(|&sid| {
            let sc = &self.cards[sid.0 as usize];
            !sc.cur.no_abilities
                && self.db.def(sc.def).statics.iter().any(|&i| {
                    matches!(&self.db.def(sc.def).abilities[i as usize], Ability::Static { effect: StaticEffect::GraveyardCardsCantBeTargeted, condition, .. }
                        if condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(sid, sc.cur.controller))))
                })
        })
    }

    /// A static (Tomik) bars `controller` (an opponent of its controller) from targeting this battlefield permanent or graveyard card.
    pub(crate) fn opp_untargetable(&self, controller: PlayerId, target: CardId) -> bool {
        let z = self.cards[target.0 as usize].zone;
        if z != Zone::Battlefield && z != Zone::Graveyard {
            return false;
        }
        self.battlefield.iter().any(|&sid| {
            let sc = &self.cards[sid.0 as usize];
            if sc.cur.no_abilities || sc.cur.controller == controller {
                return false;
            }
            let sd = self.db.def(sc.def);
            sd.statics.iter().any(|&i| {
                matches!(&sd.abilities[i as usize], Ability::Static { effect: StaticEffect::CantBeTargetedByOpponents { affects }, condition, .. }
                    if self.matches_card(&Filter { controller: PlayerFilter::Any, ..affects.clone().into_filter() }, target, sid, sc.cur.controller)
                        && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(sid, sc.cur.controller))))
            })
        })
    }

    /// Can `controller`'s `source` (spell/ability) target this permanent?
    /// A modal double-faced card whose back face is a land may be played from a graveyard / exile zone that lets you play lands.
    fn push_mdfc_land_face(&self, p: PlayerId, id: CardId, out: &mut Vec<Action>) {
        let d = self.def_of(id);
        let player = &self.players[p as usize];
        if d.face_castable && d.other_face.map_or(false, |b| self.db.def(b).types.has(CardType::Land)) && (player.lands_played as i32) < player.land_limit as i32 + self.extra_land_drops(p) && self.sorcery_timing(p) && !self.cant_play_land_card(p, id) {
            out.push(Action::PlayLandFace { card: id });
        }
    }

    pub fn can_target_card(&self, source: CardId, controller: PlayerId, target: CardId) -> bool {
        let t = &self.cards[target.0 as usize];
        if t.zone == Zone::Battlefield {
            if t.phased_out || t.phased_indirect {
                return false;
            }
            if self.opp_untargetable(controller, target) {
                return false;
            }
            if t.cur.kw.has(Keyword::Shroud) && !self.shroud_ignored(controller, target) {
                return false;
            }
            if t.cur.kw.has(Keyword::Hexproof) && t.cur.controller != controller && !self.hexproof_ignored(controller, Obj::card(target), false) {
                return false;
            }
            let s = &self.cards[source.0 as usize];
            if (s.zone == Zone::Stack || s.zone == Zone::Hand) && s.cur.subtypes.contains(&Sym::new("Aura")) && (self.host_bars_other_auras(target, source) || self.host_guarded_from_auras(target)) {
                return false;
            }
            // a spell: its card is on the stack or being cast from hand (abilities of a sacrificed source resolve from the graveyard)
            if s.zone == Zone::Stack || s.zone == Zone::Hand {
                for &sid in &self.battlefield {
                    let sc = &self.cards[sid.0 as usize];
                    if sc.cur.no_abilities {
                        continue;
                    }
                    let sd = self.db.def(sc.def);
                    for &i in &sd.statics {
                        if let Ability::Static { effect: StaticEffect::CantBeTargetedBySpells { affects, spell }, condition, .. } = &sd.abilities[i as usize] {
                            if self.matches_card(affects, target, sid, sc.cur.controller)
                                && self.matches_card(spell, source, sid, sc.cur.controller)
                                && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(sid, sc.cur.controller)))
                            {
                                return false;
                            }
                        }
                    }
                }
            }
            // "can't be the target of spells or abilities your opponents control this turn" (Vines of Vastwood): the opponents of whoever made the effect, even the creature's own controller
            if self.effects.iter().any(|e| e.controller != controller && e.affects.iter().any(|(c, _)| *c == target) && matches!(&e.modif, Modif::Static(b) if matches!(&**b, StaticEffect::CantBeTargetedByOpponents { .. }))) {
                return false;
            }
            if (s.zone == Zone::Stack || s.zone == Zone::Hand)
                && self.effects.iter().any(|e| match &e.modif {
                    Modif::Static(b) => match &**b {
                        StaticEffect::CantBeTargetedBySpells { affects, spell } => {
                            self.matches_card(affects, target, e.source, e.controller)
                                && self.matches_card(spell, source, e.source, e.controller)
                        }
                        _ => false,
                    },
                    _ => false,
                })
            {
                return false;
            }
            // "can't be the target of blue spells or abilities from blue sources" (any controller, the permanent's own included)
            for &sid in &self.battlefield {
                let sc = &self.cards[sid.0 as usize];
                if sc.cur.no_abilities {
                    continue;
                }
                let sd = self.db.def(sc.def);
                for &i in &sd.statics {
                    if let Ability::Static { effect: StaticEffect::CantBeTargetedBySource { affects, source: sf, abilities_only }, condition, .. } = &sd.abilities[i as usize] {
                        if !(*abilities_only && matches!(s.zone, Zone::Stack | Zone::Hand))
                            && self.matches_card(affects, target, sid, sc.cur.controller)
                            && self.matches_card(&Filter { controller: PlayerFilter::Any, ..sf.clone().into_filter() }, source, sid, sc.cur.controller)
                            && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(sid, sc.cur.controller)))
                        {
                            return false;
                        }
                    }
                }
            }
            if t.cur.controller != controller
                && !self.hexproof_ignored(controller, Obj::card(target), false)
                && t.cur.hexproof_from.iter().any(|pr| match pr {
                    // "hexproof from the colour of your choice": the colour this permanent chose
                    ProtectionFrom::ChosenColor => t.chosen_color.map_or(false, |c| s.cur.colors.has(c)),
                    ProtectionFrom::Matching { .. } | ProtectionFrom::ArtifactManaValues | ProtectionFrom::ChosenType => self.chosen_prot_hit(t, s, pr),
                    other => Self::source_matches_prot(other, s),
                })
            {
                return false;
            }
            for pr in &t.cur.protections {
                let hit = match pr {
                    ProtectionFrom::Color { c } => s.cur.colors.has(*c),
                    ProtectionFrom::Type { t } => s.cur.types.has(*t),
                    ProtectionFrom::Everything => true,
                    ProtectionFrom::Abilities => false,
                    ProtectionFrom::Colorless => s.cur.colors.count() == 0,
                    ProtectionFrom::Monocolored => s.cur.colors.count() == 1,
            ProtectionFrom::NotColor { c } => !s.cur.colors.has(*c),
            ProtectionFrom::NonSubtype { name } => s.cur.types.has(CardType::Creature) && !s.cur.subtypes.contains(&Sym::new(name)) && !s.cur.kw.has(Keyword::Changeling),
                    ProtectionFrom::Multicolored => s.cur.colors.count() > 1,
                    ProtectionFrom::Subtype { name } => s.cur.subtypes.contains(&Sym::new(name)) || (s.cur.kw.has(Keyword::Changeling) && s.cur.types.has(CardType::Creature)),
                    ProtectionFrom::ChosenPlayer => t.chosen_player == Some(controller),
                    ProtectionFrom::Opponents => controller != t.cur.controller,
                    ProtectionFrom::ControlledBy { p, .. } => *p == Some(controller),
                    ProtectionFrom::ChosenColor | ProtectionFrom::ChosenType | ProtectionFrom::Matching { .. } | ProtectionFrom::ArtifactManaValues => self.chosen_prot_hit(t, s, pr),
                };
                if hit {
                    return false;
                }
            }
        }
        true
    }

    /// A granted `PlayerHexproofFrom` keeps an opponent's matching source from targeting the player.
    fn player_hexproof_from(&self, p: PlayerId, controller: PlayerId, source: CardId) -> bool {
        controller != p
            && self.player_effect_static(p, |s| matches!(s, StaticEffect::PlayerHexproofFrom { from, .. } if Self::source_matches_prot(from, &self.cards[source.0 as usize])))
            && !self.hexproof_ignored(controller, Obj::player(p), false)
    }

    pub fn can_target_player(&self, source: CardId, controller: PlayerId, target: PlayerId) -> bool {
        self.alive(target) && !self.player_protected_from(target, source) && !(controller != target && self.player_hexproof(target) && !self.hexproof_ignored(controller, Obj::player(target), false)) && !self.player_shroud(target) && !self.player_hexproof_from(target, controller, source)
    }

    /// Does a static ability give this player shroud?
    pub fn player_shroud(&self, p: PlayerId) -> bool {
        if self.player_effect_static(p, |s| matches!(s, StaticEffect::PlayerShroud { .. })) {
            return true;
        }
        self.battlefield.iter().any(|&id| {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                return false;
            }
            let d = self.db.def(c.def);
            d.statics.iter().any(|&i| {
                matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::PlayerShroud { who }, condition, .. }
                    if self.player_filter(*who, p, c.cur.controller) && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(id, c.cur.controller))))
            })
        })
    }

    /// Does a static ability give this player hexproof?
    pub fn player_hexproof(&self, p: PlayerId) -> bool {
        if self.player_effect_static(p, |s| matches!(s, StaticEffect::PlayerHexproof { .. })) {
            return true;
        }
        for id in self.battlefield.iter().copied().chain(self.emblem_srcs()) {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                continue;
            }
            let ctl = self.src_ctrl(id);
            let d = self.db.def(c.def);
            for &i in &d.statics {
                if let Ability::Static { effect: StaticEffect::PlayerHexproof { who }, condition, .. } = &d.abilities[i as usize] {
                    if self.player_filter(*who, p, ctl)
                        && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(id, ctl)))
                    {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn source_matches_prot(pr: &ProtectionFrom, s: &Card) -> bool {
        match pr {
            ProtectionFrom::Color { c } => s.cur.colors.has(*c),
            ProtectionFrom::Type { t } => s.cur.types.has(*t),
            ProtectionFrom::Everything => true,
            ProtectionFrom::Abilities => !matches!(s.zone, Zone::Stack | Zone::Hand),
            ProtectionFrom::Colorless => s.cur.colors.count() == 0,
            ProtectionFrom::Monocolored => s.cur.colors.count() == 1,
            ProtectionFrom::NotColor { c } => !s.cur.colors.has(*c),
            ProtectionFrom::NonSubtype { name } => s.cur.types.has(CardType::Creature) && !s.cur.subtypes.contains(&Sym::new(name)) && !s.cur.kw.has(Keyword::Changeling),
            ProtectionFrom::Multicolored => s.cur.colors.count() > 1,
            ProtectionFrom::Subtype { name } => s.cur.subtypes.contains(&Sym::new(name)) || (s.cur.kw.has(Keyword::Changeling) && s.cur.types.has(CardType::Creature)),
            ProtectionFrom::ControlledBy { p, .. } => *p == Some(s.cur.controller),
            ProtectionFrom::ChosenPlayer | ProtectionFrom::ChosenColor | ProtectionFrom::ChosenType | ProtectionFrom::Opponents | ProtectionFrom::Matching { .. } | ProtectionFrom::ArtifactManaValues => false,
        }
    }

    /// Does the player have protection (granted by an effect) from this source?
    pub fn player_protected_from(&self, player: PlayerId, source: CardId) -> bool {
        let s = &self.cards[source.0 as usize];
        let static_prot = self.battlefield.iter().any(|&id| {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities {
                return false;
            }
            let d = self.db.def(c.def);
            d.statics.iter().any(|&i| match &d.abilities[i as usize] {
                Ability::Static { effect: StaticEffect::PlayerProtection { who, from }, condition, .. } => {
                    self.player_filter(*who, player, c.cur.controller)
                        && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(id, c.cur.controller)))
                        && match from {
                            ProtectionFrom::Opponents => s.cur.controller != player,
                            ProtectionFrom::Everything => true,
                            ProtectionFrom::Color { c } => s.cur.colors.has(*c),
                            ProtectionFrom::Type { t } => s.cur.types.has(*t),
                            ProtectionFrom::Colorless => s.cur.colors.count() == 0,
                            ProtectionFrom::Monocolored => s.cur.colors.count() == 1,
            ProtectionFrom::NotColor { c } => !s.cur.colors.has(*c),
            ProtectionFrom::NonSubtype { name } => s.cur.types.has(CardType::Creature) && !s.cur.subtypes.contains(&Sym::new(name)) && !s.cur.kw.has(Keyword::Changeling),
                            ProtectionFrom::Multicolored => s.cur.colors.count() > 1,
                            ProtectionFrom::Subtype { name } => s.cur.subtypes.contains(&Sym::new(name)) || (s.cur.kw.has(Keyword::Changeling) && s.cur.types.has(CardType::Creature)),
                            // protection from sources matching a filter read from the granting permanent (Runed Halo: `chosen_name`)
                            ProtectionFrom::Matching { filter } => self.matches_card(filter, source, id, c.cur.controller),
                            _ => false,
                        }
                }
                _ => false,
            })
        });
        static_prot || self.player_prot.iter().filter(|pp| pp.player == player).any(|pp| match &pp.from {
            ProtectionFrom::Color { c } => s.cur.colors.has(*c),
            ProtectionFrom::Type { t } => s.cur.types.has(*t),
            ProtectionFrom::Everything => true,
            ProtectionFrom::Abilities => false,
            ProtectionFrom::Colorless => s.cur.colors.count() == 0,
            ProtectionFrom::Monocolored => s.cur.colors.count() == 1,
            ProtectionFrom::NotColor { c } => !s.cur.colors.has(*c),
            ProtectionFrom::NonSubtype { name } => s.cur.types.has(CardType::Creature) && !s.cur.subtypes.contains(&Sym::new(name)) && !s.cur.kw.has(Keyword::Changeling),
            ProtectionFrom::Multicolored => s.cur.colors.count() > 1,
            ProtectionFrom::Subtype { name } => s.cur.subtypes.contains(&Sym::new(name)) || (s.cur.kw.has(Keyword::Changeling) && s.cur.types.has(CardType::Creature)),
            ProtectionFrom::ChosenPlayer | ProtectionFrom::ChosenColor | ProtectionFrom::ChosenType | ProtectionFrom::Matching { .. } | ProtectionFrom::ArtifactManaValues => false,
            ProtectionFrom::Opponents => s.cur.controller != player,
            ProtectionFrom::ControlledBy { p, .. } => *p == Some(s.cur.controller),
        })
    }

    /// Replace each `per_opponent` slot by one slot per opponent (restricted to what that opponent controls).
    pub(crate) fn expand_per_opponent(&self, specs: &[TargetSpec], p: PlayerId) -> Vec<TargetSpec> {
        let mut out = vec![];
        for s in specs {
            if s.per_player {
                let mut ps = vec![p];
                ps.extend(self.opponents(p));
                for o in ps {
                    let mut c = s.clone();
                    c.per_player = false;
                    c.only_controller = Some(o);
                    out.push(c);
                }
            } else if s.per_opponent {
                for o in self.opponents(p) {
                    if s.corrupted && self.players[o as usize].poison < 3 {
                        continue;
                    }
                    let mut c = s.clone();
                    c.per_opponent = false;
                    c.only_controller = Some(o);
                    out.push(c);
                }
            } else {
                out.push(s.clone());
            }
        }
        out
    }

    pub fn legal_targets(&self, spec: &TargetSpec, source: CardId, controller: PlayerId) -> Vec<Obj> {
        let mut out = self.legal_targets_raw(spec, source, controller);
        // Peace Talks: players and permanents can't be targeted by spells or activated abilities
        if self.no_target_until >= self.turn && self.no_target_until != 0 && !self.target_trig.get() {
            out.retain(|o| match o {
                Obj::Player { .. } => false,
                Obj::Card { id } => self.cards[id.0 as usize].zone != Zone::Battlefield,
                _ => true,
            });
        }
        // "can't be the target of spells or abilities that can target only Walls"
        fn only_subtype(k: &TargetKind, st: &str) -> bool {
            match k {
                TargetKind::Permanent { filter } => filter.subtypes.iter().any(|x| x == st),
                TargetKind::AnyOf { kinds } => !kinds.is_empty() && kinds.iter().all(|k| only_subtype(k, st)),
                _ => false,
            }
        }
        if matches!(spec.what, TargetKind::Permanent { .. } | TargetKind::AnyOf { .. }) {
            let sts: Vec<(CardId, PlayerId, &FilterBox, &str)> = self
                .battlefield
                .iter()
                .filter(|&&sid| !self.cards[sid.0 as usize].cur.no_abilities)
                .flat_map(|&sid| {
                    let sd = self.db.def(self.cards[sid.0 as usize].def);
                    let ctl = self.cards[sid.0 as usize].cur.controller;
                    sd.statics.iter().filter_map(move |&i| match &sd.abilities[i as usize] {
                        Ability::Static { effect: StaticEffect::CantBeTargetedByOnlySubtype { affects, subtype }, .. } => Some((sid, ctl, affects, subtype.as_str())),
                        _ => None,
                    })
                })
                .collect();
            // a modal spell with a mode that isn't Wall-only is not a spell "that can target only Walls" (Wall of Shadows ruling)
            let modal_src = self.db.def(self.cards[source.0 as usize].def).abilities.iter().any(|ab| matches!(ab, Ability::Modal { modes, .. } if modes.len() > 1))
                && self.cards[source.0 as usize].zone == Zone::Stack;
            for (sid, ctl, affects, st) in sts {
                if !modal_src && only_subtype(&spec.what, st) {
                    out.retain(|o| match o {
                        Obj::Card { id } => !self.matches_card(affects, *id, sid, ctl),
                        _ => true,
                    });
                }
            }
        }
        // Flagbearer: while casting a spell or activating an ability, an opponent of the Flagbearer rule's controller must choose a Flagbearer if able
        if spec.min > 0 && !self.target_trig.get() && out.len() > 1 {
            let flags: Vec<CardId> = self.battlefield.iter().copied().filter(|id| self.cards[id.0 as usize].cur.subtypes.contains(&Sym::new("Flagbearer"))).collect();
            if !flags.is_empty()
                && self.battlefield.iter().any(|&sid| {
                    let sc = &self.cards[sid.0 as usize];
                    !sc.cur.no_abilities && sc.cur.controller != controller && self.db.def(sc.def).statics.iter().any(|&i| matches!(&self.db.def(sc.def).abilities[i as usize], Ability::Static { effect: StaticEffect::FlagbearerMustBeTargeted, .. }))
                })
                && out.iter().any(|o| matches!(o, Obj::Card { id } if flags.contains(id)))
            {
                out.retain(|o| matches!(o, Obj::Card { id } if flags.contains(id)));
            }
        }
        if let Some(c) = spec.only_controller {
            out.retain(|o| match o {
                Obj::Player { id } => *id == c,
                Obj::Card { id } => {
                    let k = &self.cards[id.0 as usize];
                    if k.zone == Zone::Battlefield { k.cur.controller == c } else { k.owner == c }
                }
                Obj::Stack { id } => self.stack.iter().find(|s| s.id == *id).map_or(false, |s| s.controller == c),
                _ => false,
            });
        }
        out
    }

    fn legal_targets_raw(&self, spec: &TargetSpec, source: CardId, controller: PlayerId) -> Vec<Obj> {
        let mut out = vec![];
        match &spec.what {
            TargetKind::Permanent { filter } => {
                for &id in &self.battlefield {
                    if self.matches_card(filter, id, source, controller) && self.can_target_card(source, controller, id) {
                        out.push(Obj::card(id));
                    }
                }
            }
            TargetKind::AnyTarget => {
                for &id in &self.battlefield {
                    let t = self.cards[id.0 as usize].cur.types;
                    if (t.has(CardType::Creature) || t.has(CardType::Planeswalker) || t.has(CardType::Battle))
                        && self.can_target_card(source, controller, id)
                    {
                        out.push(Obj::card(id));
                    }
                }
                for p in self.alive_players() {
                    if self.can_target_player(source, controller, p) {
                        out.push(Obj::player(p));
                    }
                }
            }
            TargetKind::AnyOf { kinds } => {
                for k in kinds {
                    let sub = TargetSpec { what: k.clone(), ..spec.clone() };
                    for o in self.legal_targets(&sub, source, controller) {
                        if !out.contains(&o) {
                            out.push(o);
                        }
                    }
                }
            }
            TargetKind::OpponentOrPlaneswalker => {
                for &id in &self.battlefield {
                    if self.cards[id.0 as usize].cur.types.has(CardType::Planeswalker) && self.can_target_card(source, controller, id) {
                        out.push(Obj::card(id));
                    }
                }
                for p in self.alive_players() {
                    if p != controller && self.can_target_player(source, controller, p) {
                        out.push(Obj::player(p));
                    }
                }
            }
            TargetKind::PlayerOrPlaneswalker => {
                for &id in &self.battlefield {
                    if self.cards[id.0 as usize].cur.types.has(CardType::Planeswalker) && self.can_target_card(source, controller, id) {
                        out.push(Obj::card(id));
                    }
                }
                for p in self.alive_players() {
                    if self.can_target_player(source, controller, p) {
                        out.push(Obj::player(p));
                    }
                }
            }
            TargetKind::Player { filter } => {
                for p in self.alive_players() {
                    if self.player_filter_src(source, *filter, p, controller) && self.can_target_player(source, controller, p) {
                        out.push(Obj::player(p));
                    }
                }
            }
            TargetKind::CardIn { zone, filter, whose } => {
                if *zone == Zone::Graveyard && self.graveyard_untargetable() {
                    return out;
                }
                for pl in &self.players {
                    if !self.player_filter_src(source, *whose, pl.id, controller) {
                        continue;
                    }
                    let list: &Vec<CardId> = match zone {
                        Zone::Graveyard => &pl.graveyard,
                        Zone::Hand => &pl.hand,
                        Zone::Exile => &pl.exile,
                        Zone::Library => &pl.library,
                        _ => continue,
                    };
                    for &id in list {
                        // a spell cast from the graveyard (flashback) has already left it: it can't target itself there (Mystic Retrieval)
                        if id == source && *zone == Zone::Graveyard && self.cards[id.0 as usize].zone == Zone::Graveyard && (self.def_of(id).types.has(CardType::Instant) || self.def_of(id).types.has(CardType::Sorcery)) {
                            continue;
                        }
                        let mut f = filter.clone();
                        f.controller = PlayerFilter::Any;
                        if self.matches_card(&f, id, source, controller) && !(*zone == Zone::Graveyard && self.opp_untargetable(controller, id)) {
                            out.push(Obj::card(id));
                        }
                    }
                }
            }
            TargetKind::Spell { filter } => {
                for s in &self.stack {
                    if matches!(s.kind, ItemKind::Spell { .. }) && s.source != source {
                        let mut f = filter.clone();
                        let ctrl_filter = std::mem::take(&mut f.controller);
                        if self.matches_stack_spell(&f, s, source, controller)
                            && self.player_filter(ctrl_filter, s.controller, controller)
                        {
                            out.push(Obj::Stack { id: s.id });
                        }
                    }
                }
            }
            TargetKind::StackObject { filter } => {
                for s in &self.stack {
                    if s.source == source {
                        continue;
                    }
                    // target predicates work for abilities too
                    let n = s.targets.len();
                    // one object targeted several times still counts as a single target (Seeds of Strength on the same creature three times)
                    let distinct = if s.targets.iter().all(|t| Some(t.obj) == s.targets.first().map(|f| f.obj)) { n.min(1) } else { n };
                    if (filter.single_target && distinct != 1) || (filter.spell_has_targets && n == 0) {
                        continue;
                    }
                    if filter.single_permanent_or_player_target
                        && !(n == 1 && match s.targets[0].obj {
                            Obj::Player { .. } => true,
                            Obj::Card { id } => self.cards[id.0 as usize].zone == Zone::Battlefield,
                            _ => false,
                        })
                    {
                        continue;
                    }
                    // `spell_targets_you` together with `spell_targets`: it targets you OR a matching permanent
                    let t_you = s.targets.iter().any(|t| matches!(t.obj, Obj::Player { id } if id == controller));
                    let t_perm = filter.spell_targets.as_ref().map_or(false, |tf| s.targets.iter().any(|t| matches!(t.obj, Obj::Card { id } if self.matches_card(tf, id, source, controller))));
                    match (filter.spell_targets_you, filter.spell_targets.is_some()) {
                        (true, true) if !(t_you || t_perm) => continue,
                        (true, false) if !t_you => continue,
                        (false, true) if !t_perm => continue,
                        _ => {}
                    }
                    // "a spell or ability an opponent controls"
                    if !self.player_filter_src(source, filter.controller, s.controller, controller) {
                        continue;
                    }
                    if matches!(s.kind, ItemKind::Spell { .. }) {
                        let mut f = filter.clone();
                        f.controller = PlayerFilter::Any;
                        f.single_target = false;
                        f.spell_has_targets = false;
                        f.spell_targets = None;
                        f.spell_targets_you = false;
                        if !self.matches_card(&f, s.source, source, controller) {
                            continue;
                        }
                    }
                    out.push(Obj::Stack { id: s.id });
                }
            }
            TargetKind::StackAbility { controller: cf, kind, filter } => {
                for s in &self.stack {
                    let ok = match (&s.kind, kind) {
                        (ItemKind::Ability { .. }, StackAbilityKind::Any | StackAbilityKind::Activated) => true,
                        (ItemKind::Triggered { .. } | ItemKind::Delayed { .. }, StackAbilityKind::Any | StackAbilityKind::Triggered) => true,
                        _ => false,
                    };
                    if filter.single_target && !(!s.targets.is_empty() && s.targets.iter().all(|t| t.obj == s.targets[0].obj)) {
                        continue;
                    }
                    if ok && self.player_filter(*cf, s.controller, controller) {
                        let mut f = filter.clone();
                        f.controller = PlayerFilter::Any;
                        f.single_target = false;
                        if self.matches_card(&f, s.source, source, controller) {
                            out.push(Obj::Stack { id: s.id });
                        }
                    }
                }
            }
        }
        out
    }

    /// A modal spell that says "Choose X": X is announced before the modes are chosen.
    fn modal_x_first(&self, card: CardId) -> bool {
        self.def_of(card).abilities.iter().any(|a| matches!(a, Ability::Modal { x_modes: true, .. }))
    }

    /// The target specs a spell has before modes are chosen (None for a modal spell).
    /// A permanent with `GraveyardLoseAbilities` (Yixlid Jailer) is on the battlefield.
    pub(crate) fn graveyard_abilities_off(&self) -> bool {
        self.battlefield.iter().any(|&b| {
            let bc = &self.cards[b.0 as usize];
            !bc.cur.no_abilities && { let bd = self.db.def(bc.def); bd.statics.iter().any(|&i| matches!(&bd.abilities[i as usize], Ability::Static { effect: StaticEffect::GraveyardLoseAbilities, .. })) }
        })
    }

    pub(crate) fn spell_specs(&self, card: CardId) -> (Vec<TargetSpec>, Option<(u8, u8, usize)>) {
        let d = self.def_of(card);
        let mut specs = vec![];
        for a in &d.abilities {
            match a {
                Ability::Spell { targets, .. } => specs.extend(targets.iter().cloned()),
                Ability::Modal { choose_min, choose_max, modes, max_if, min_if, .. } => {
                    let mut hi = match max_if {
                        Some((c, m)) if self.eval_cond(c, &EvalCtx::with_controller(card, self.cards[card.0 as usize].controller)) => (*m).max(*choose_max),
                        _ => *choose_max,
                    };
                    if self.cards[card.0 as usize].extra_mode && matches!(self.cards[card.0 as usize].zone, Zone::Sideboard | Zone::Stack) {
                        hi = hi.saturating_add(1).min(modes.len() as u8);
                    }
                    let lo = match min_if {
                        Some((c, m)) if self.eval_cond(c, &EvalCtx::with_controller(card, self.cards[card.0 as usize].controller)) => (*m).min(hi),
                        _ => *choose_min,
                    };
                    return (vec![], Some((lo, hi, modes.len())));
                }
                Ability::EnchantPlayer => specs.push(TargetSpec {
                    what: TargetKind::Player { filter: PlayerFilter::Any },
                    min: 1,
                    max: 1,
                    label: "enchant player".into(),
                    max_x: false, exact_x: false, max_amount: None, max_total_mv: None, max_total_power: None, chooser: None,
                    distinct: false, when_kicked: None, per_opponent: false, per_player: false, same_graveyard: false, distinct_by: None, random: false, only_controller: None, distinct_modes: false, corrupted: false,
                }),
                Ability::EnchantOpponent => specs.push(TargetSpec {
                    what: TargetKind::Player { filter: PlayerFilter::Opponent },
                    min: 1,
                    max: 1,
                    label: "enchant opponent".into(),
                    max_x: false, exact_x: false, max_amount: None, max_total_mv: None, max_total_power: None, chooser: None,
                    distinct: false, when_kicked: None, per_opponent: false, per_player: false, same_graveyard: false, distinct_by: None, random: false, only_controller: None, distinct_modes: false, corrupted: false,
                }),
                Ability::AttachGraveyard { to, .. } => specs.push(TargetSpec {
                    what: TargetKind::CardIn { zone: Zone::Graveyard, filter: to.clone(), whose: PlayerFilter::Any },
                    min: 1,
                    max: 1,
                    label: "enchant card in a graveyard".into(),
                    max_x: false, exact_x: false, max_amount: None, max_total_mv: None, max_total_power: None, chooser: None,
                    distinct: false, when_kicked: None, per_opponent: false, per_player: false, same_graveyard: false, distinct_by: None, random: false, only_controller: None, distinct_modes: false, corrupted: false,
                }),
                Ability::Attach { to } => specs.push(TargetSpec {
                    what: TargetKind::Permanent { filter: to.clone() },
                    min: 1,
                    max: 1,
                    label: "enchant".into(),
                    max_x: false, exact_x: false, max_amount: None, max_total_mv: None, max_total_power: None, chooser: None,
                    distinct: false, when_kicked: None, per_opponent: false, per_player: false, same_graveyard: false, distinct_by: None, random: false, only_controller: None, distinct_modes: false, corrupted: false,
                }),
                _ => {}
            }
        }
        (specs, None)
    }

    pub(crate) fn specs_satisfiable(&self, specs: &[TargetSpec], source: CardId, controller: PlayerId) -> bool {
        // slots that depend on whether the spell gets kicked are checked when they are actually chosen
        // and slots whose filter refers to an earlier target ("another creature that player controls") can only be judged once that one is chosen
        let is_dep = |s: &TargetSpec| match &s.what {
            TargetKind::Permanent { filter } | TargetKind::CardIn { filter, .. } | TargetKind::Spell { filter } => filter.controlled_by.as_ref().map_or(false, |c| format!("{c:?}").contains("Target")) || filter.shares_type_with.as_ref().map_or(false, |c| format!("{c:?}").contains("Target")) || filter.shares_card_type_with.as_ref().map_or(false, |c| format!("{c:?}").contains("Target")) || filter.blocked_pair_this_turn.as_ref().map_or(false, |c| format!("{c:?}").contains("Target")),
            _ => false,
        };
        // a dependent required slot is satisfiable if SOME choice for the first slot leaves it enough legal targets
        if specs.len() >= 2 && specs[0].when_kicked.is_none() {
            for dep in specs.iter().skip(1).filter(|s| is_dep(s) && s.min > 0 && s.when_kicked.is_none()) {
                let firsts = self.legal_targets(&specs[0], source, controller);
                let ok = firsts.iter().any(|o| {
                    let zcc = self.obj_zcc(o);
                    *self.filter_targets.borrow_mut() = (vec![Target { obj: *o, zcc }], vec![0]);
                    let n = self.legal_targets(dep, source, controller).into_iter().filter(|x| !(dep.distinct && x == o)).count();
                    *self.filter_targets.borrow_mut() = (vec![], vec![]);
                    n >= dep.min as usize
                });
                if !ok {
                    return false;
                }
            }
        }
        let specs: Vec<TargetSpec> = specs.iter().filter(|s| s.when_kicked.is_none() && !is_dep(s)).cloned().collect();
        let specs = &specs[..];
        if !specs.iter().all(|s| s.min == 0 || (s.max_x && s.exact_x) || self.legal_targets(s, source, controller).len() >= s.min as usize) {
            return false;
        }
        if let Some(last) = specs.iter().rposition(|s| s.distinct_by == Some(ChooseDistinct::Controller) && s.min > 0) {
            // "controlled by different players": the slots up to the last such one need that many different controllers overall
            let mut seen: Vec<PlayerId> = vec![];
            for s in specs[..=last].iter().filter(|s| s.min > 0) {
                for o in self.legal_targets(s, source, controller) {
                    if let Obj::Card { id } = o {
                        let c = self.cards[id.0 as usize].cur.controller;
                        if !seen.contains(&c) { seen.push(c); }
                    }
                }
            }
            if seen.len() < specs[..=last].iter().filter(|s| s.min > 0).map(|s| s.min as usize).sum::<usize>() {
                return false;
            }
        }
        if specs.iter().any(|s| s.distinct && s.min > 0) {
            // slots that must differ from earlier ones need enough distinct objects overall (greedy, scarcest slot first)
            let mut pools: Vec<(usize, Vec<Obj>)> = specs.iter().filter(|s| s.min > 0).map(|s| (s.min as usize, self.legal_targets(s, source, controller))).collect();
            pools.sort_by_key(|(_, p)| p.len());
            let mut used: Vec<Obj> = vec![];
            for (need, pool) in pools {
                let free: Vec<Obj> = pool.into_iter().filter(|o| !used.contains(o)).take(need).collect();
                if free.len() < need {
                    return false;
                }
                used.extend(free);
            }
        }
        true
    }

    // ------------------------------------------------------------------ legal actions

    /// Every non-mana action `p` could take with priority right now.
    pub fn legal_actions(&self, p: PlayerId) -> Vec<Action> {
        // safety valve against runaway loops that stack hundreds of abilities
        if self.stack.len() >= 300 {
            return vec![];
        }
        let mut acts = self.legal_actions_unrestricted(p);
        // "can't cast spells" statics (Grand Abolisher, Rule of Law ...)
        let mut cant_cast_cards: Vec<(Filter, CardId, PlayerId)> = vec![];
        let mut no_act_nonloyalty = false;
        let (mut no_cast, mut no_act) = (self.effects.iter().any(|e| (matches!(e.modif, Modif::NoCast) || matches!(e.modif, Modif::SpellCap(cap) if self.players[p as usize].spells_cast >= cap)) && e.players.contains(&p)), false);
        // emblems (tokens in the command zone) have static abilities too
        let emblem_srcs: Vec<CardId> = self.emblem_srcs();
        for &src in self.battlefield.iter().chain(emblem_srcs.iter()) {
            let sc = &self.cards[src.0 as usize];
            if sc.cur.no_abilities {
                continue;
            }
            let sd = self.db.def(sc.def);
            let sctrl = if sc.zone == Zone::Command { sc.owner } else { sc.cur.controller };
            for &i in &sd.statics {
                if let Ability::Static { effect, condition, .. } = &sd.abilities[i as usize] {
                    // a conditional static ("as long as it's your turn", "after casting one spell this turn") only applies while the condition holds
                    if condition.as_ref().map_or(false, |cd| !self.eval_cond(cd, &EvalCtx::with_controller(sc.id, sctrl))) {
                        continue;
                    }
                    match effect {
                        StaticEffect::CantCast { who } if self.player_filter_src(sc.id, *who, p, sctrl) => no_cast = true,
                        StaticEffect::PlayerCantActivate { who } if self.player_filter_src(sc.id, *who, p, sctrl) => no_act_nonloyalty = true,
                        StaticEffect::CastOnlyAtSorcerySpeed { who } if self.player_filter_src(sc.id, *who, p, sctrl) && !self.sorcery_timing(p) => no_cast = true,
                        StaticEffect::CantCastCards { who, filter, caster_relative } if self.player_filter_src(sc.id, *who, p, sctrl) => cant_cast_cards.push((filter.clone().into_filter(), sc.id, if *caster_relative { p } else { sctrl })),
                        StaticEffect::NoSpellsOrAbilitiesInCombat if matches!(self.step, Step::BeginCombat | Step::DeclareAttackers | Step::DeclareBlockers | Step::FirstStrikeDamage | Step::CombatDamage | Step::EndCombat) => {
                            no_cast = true;
                            no_act = true;
                        }
                        StaticEffect::NoSpellsInCombat if matches!(self.step, Step::BeginCombat | Step::DeclareAttackers | Step::DeclareBlockers | Step::FirstStrikeDamage | Step::CombatDamage | Step::EndCombat) => no_cast = true,
                        StaticEffect::OpponentsCantCastDuringYourTurn if p != sctrl && self.active == sctrl => {
                            no_cast = true;
                        }
                        _ => {}
                    }
                }
            }
        }
        // "You can't cast this card if you've played a land this turn" (Rock Jockey): a static that works while the card is in the hand
        for &hid in &self.players[p as usize].hand {
            let hd = self.db.def(self.cards[hid.0 as usize].def);
            for &i in &hd.statics {
                if let Ability::Static { effect: StaticEffect::CantCastCards { filter, caster_relative, .. }, condition, from: Some(Zone::Hand) } = &hd.abilities[i as usize] {
                    if condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(hid, p))) {
                        cant_cast_cards.push((filter.clone().into_filter(), hid, if *caster_relative { p } else { p }));
                    }
                }
            }
        }
        // "can't cast ..." granted by a resolved spell or ability for a while (Academic Probation)
        for e in &self.effects {
            if let Modif::DynStatic(b) = &e.modif {
                match &**b {
                    StaticEffect::CantCast { who } if self.player_filter_src(e.source, *who, p, e.controller) => no_cast = true,
                    StaticEffect::CantCastCards { who, filter, caster_relative } if self.player_filter_src(e.source, *who, p, e.controller) => cant_cast_cards.push((filter.clone().into_filter(), e.source, if *caster_relative { p } else { e.controller })),
                    _ => {}
                }
            }
        }
        if no_cast {
            acts.retain(|a| !matches!(a, Action::Cast { .. }));
        }
        if self.effects.iter().any(|e| matches!(e.modif, Modif::NoCastCreatures) && e.players.contains(&p)) {
            acts.retain(|a| match a {
                Action::Cast { card, .. } => !self.def_of(*card).types.has(CardType::Creature),
                _ => true,
            });
        }
        if self.effects.iter().any(|e| matches!(e.modif, Modif::NoCastNoncreature) && e.players.contains(&p)) {
            acts.retain(|a| match a {
                Action::Cast { card, .. } => self.def_of(*card).types.has(CardType::Creature),
                _ => true,
            });
        }
        if self.effects.iter().any(|e| matches!(e.modif, Modif::NoCastNotFromHand) && e.players.contains(&p)) {
            acts.retain(|a| match a {
                Action::Cast { card, .. } => self.cards[card.0 as usize].zone == Zone::Hand,
                _ => true,
            });
        }
        if self.effects.iter().any(|e| matches!(e.modif, Modif::NoCastInstantSorcery) && e.players.contains(&p)) {
            acts.retain(|a| match a {
                Action::Cast { card, .. } => !(self.def_of(*card).types.has(CardType::Instant) || self.def_of(*card).types.has(CardType::Sorcery)),
                _ => true,
            });
        }
        if self.effects.iter().any(|e| matches!(e.modif, Modif::NoActivateNonMana) && e.players.contains(&p)) {
            acts.retain(|a| !matches!(a, Action::Activate { .. }));
        }
        if !cant_cast_cards.is_empty() {
            acts.retain(|a| match a {
                Action::Cast { card, alt } => {
                    crate::eval::AS_SPELL.with(|c| c.set(true));
                    // a face-down spell has no name, so a card name chosen by Voidstone Gargoyle does not stop it
                    let blocked = cant_cast_cards.iter().any(|(f, src, ctrl)| !(f.chosen_name && *alt == Some(FACE_DOWN_ALT)) && self.matches_card(f, *card, *src, *ctrl));
                    crate::eval::AS_SPELL.with(|c| c.set(false));
                    !blocked
                }
                _ => true,
            });
        }
        if no_act {
            acts.retain(|a| !matches!(a, Action::Activate { .. }));
        }
        if no_act_nonloyalty {
            acts.retain(|a| match a {
                Action::Activate { card, ability } => {
                    let c = &self.cards[card.0 as usize];
                    let ab = if *ability >= GRANTED_BASE { c.cur.granted.get((*ability - GRANTED_BASE) as usize).map(|x| &**x) } else { self.db.def(c.def).abilities.get(*ability as usize) };
                    matches!(ab, Some(Ability::Activated { loyalty: Some(_), .. }))
                }
                _ => true,
            });
        }
        // "can't play lands or cast spells from their graveyards" (Shaman's Trance)
        if self.effects.iter().any(|e| matches!(e.modif, Modif::NoGraveyardPlay) && e.players.contains(&p)) {
            acts.retain(|a| match a {
                Action::Cast { card, .. } | Action::PlayLand { card } => self.cards[card.0 as usize].zone != Zone::Graveyard,
                _ => true,
            });
        }
        acts
    }

    fn legal_actions_unrestricted(&self, p: PlayerId) -> Vec<Action> {
        // split second: while such a spell is on the stack nobody can cast spells or activate non-mana abilities (mana abilities stay legal)
        let split = self.stack.iter().any(|s| {
            matches!(s.kind, ItemKind::Spell { .. })
                && (self.db.def(s.def).keywords.has(Keyword::SplitSecond)
                    || self.cards[s.source.0 as usize].cur.kw.has(Keyword::SplitSecond)
                    // "if this spell was kicked, it has split second"
                    || (self.cards[s.source.0 as usize].kicked
                        && self.db.def(s.def).abilities.iter().any(|a| matches!(a, Ability::Static { effect: StaticEffect::GrantKeyword { affects, keyword: Keyword::SplitSecond }, condition: Some(Condition::Kicked), .. } if affects.this))))
        });
        let mut acts = self.legal_actions_inner(p);
        if split {
            acts.retain(|a| match a {
                Action::Activate { card, ability } => (*ability as usize) < GRANTED_BASE as usize && self.def_of(*card).abilities.get(*ability as usize).map_or(false, |a| crate::carddb::is_mana_ability(a) || matches!(a, Ability::Activated { effects, .. } if matches!(effects.as_slice(), [Effect::TurnFaceUp { .. }]))),
                // special actions (turning a permanent face up) aren't restricted by split second
                Action::TurnFaceUp { .. } => true,
                _ => false,
            });
        }
        acts
    }

    fn legal_actions_inner(&self, p: PlayerId) -> Vec<Action> {
        let _g = crate::prof::enter(2);
        let mut out = vec![];
        let player = &self.players[p as usize];
        // hand
        for &id in &player.hand {
            let c = &self.cards[id.0 as usize];
            let d = self.db.def(c.def);
            let can_land = (player.lands_played as i32) < player.land_limit as i32 + self.extra_land_drops(p) && self.sorcery_timing(p) && !self.cant_play_land_card(p, id) && !self.cant_play_lands_from_hand(p);
            if d.face_castable {
                if let Some(back) = d.other_face.map(|b| self.db.def(b)) {
                    if back.types.has(CardType::Land) && can_land {
                        out.push(Action::PlayLandFace { card: id });
                    }
                }
            }
            if d.types.has(CardType::Land) {
                if can_land {
                    out.push(Action::PlayLand { card: id });
                }
                // a land with an adventure / spell half (Zanarkand // Lasting Fayth)
                self.push_face_cast(p, id, Zone::Hand, &mut out);
                // a land with Morph / Disguise can be cast face down as a 2/2 creature (never as a spell face up)
                if d.abilities.iter().any(|a| matches!(a, Ability::Morph { .. })) {
                    self.push_cast_actions(p, id, Zone::Hand, &mut out);
                }
                continue;
            }
            self.push_cast_actions(p, id, Zone::Hand, &mut out);
        }
        // another player's hand, through `MayPlayThisTurn { zone: Hand, of }` (Sen Triplets)
        for &(pl, ow, t) in &self.hand_play_grants {
            if pl != p || t != self.turn || ow == p {
                continue;
            }
            for &id in &self.players[ow as usize].hand {
                if self.def_of(id).types.has(CardType::Land) {
                    if (player.lands_played as i32) < player.land_limit as i32 + self.extra_land_drops(p) && self.sorcery_timing(p) && !self.cant_play_land_card(p, id) && !self.cant_play_lands_from_hand(p) {
                        out.push(Action::PlayLand { card: id });
                    }
                } else {
                    self.push_cast_actions(p, id, Zone::Hand, &mut out);
                }
            }
        }
        // cards the player owns from outside the game, through `MayPlayFromOutsideGame` (Wish)
        let all_outside = self.outside_grants.iter().any(|&(pl, t)| pl == p && t == self.turn);
        if !all_outside && self.outside_card_grants.iter().any(|g| g.0 == p && g.1 == self.turn) {
            for &id in &player.sideboard {
                if self.outside_card_grant_idx(p, id).is_none() {
                    continue;
                }
                if self.def_of(id).types.has(CardType::Land) {
                    if (player.lands_played as i32) < player.land_limit as i32 + self.extra_land_drops(p) && self.sorcery_timing(p) && !self.cant_play_land_card(p, id) {
                        out.push(Action::PlayLand { card: id });
                    }
                } else {
                    self.push_cast_actions(p, id, Zone::Sideboard, &mut out);
                }
            }
        }
        // Old Companion: a companion with that ability may be cast from the sideboard (the {3} special action is skipped)
        if !all_outside {
            for &id in &player.sideboard {
                if self.cards[id.0 as usize].is_companion && self.def_of(id).abilities.iter().any(|a| matches!(a, Ability::OldCompanion)) {
                    self.push_cast_actions(p, id, Zone::Sideboard, &mut out);
                }
            }
        }
        // Planechase: the active player may roll the planar die at sorcery speed, paying {N} for the rolls before it this turn
        if !self.planes.is_empty() && p == self.active && self.sorcery_timing(p) {
            let n = self.planar_die_cost(p);
            self.pay_for.set(None);
            let cost = crate::mana::ManaCost::parse(&format!("{{{n}}}")).unwrap();
            if n == 0 || self.can_afford(p, &cost, 0) {
                out.push(Action::RollPlanarDie);
            }
        }
        // the companion special action: pay {3} to put your companion from the sideboard into your hand
        if self.sorcery_timing(p) {
            for &id in &player.sideboard {
                if self.cards[id.0 as usize].is_companion {
                    self.pay_for.set(None);
                    let cost = crate::mana::ManaCost::parse("{3}").unwrap();
                    if self.can_afford(p, &cost, 0) {
                        out.push(Action::Companion { card: id });
                    }
                }
            }
        }
        if all_outside {
            for &id in &player.sideboard {
                if self.def_of(id).types.has(CardType::Land) {
                    if (player.lands_played as i32) < player.land_limit as i32 + self.extra_land_drops(p) && self.sorcery_timing(p) && !self.cant_play_land_card(p, id) {
                        out.push(Action::PlayLand { card: id });
                    }
                } else {
                    self.push_cast_actions(p, id, Zone::Sideboard, &mut out);
                }
            }
        }
        for &id in &player.command {
            if self.cards[id.0 as usize].is_commander {
                self.push_cast_actions(p, id, Zone::Command, &mut out);
            }
        }
        for &id in player.graveyard.iter().chain(player.exile.iter()) {
            let zone = self.cards[id.0 as usize].zone;
            let d = self.def_of(id);
            // a land exiled "on an adventure" can be played from exile
            if d.types.has(CardType::Land) && zone == Zone::Exile && self.cards[id.0 as usize].on_adventure {
                if (player.lands_played as i32) < player.land_limit as i32 + self.extra_land_drops(p) && self.sorcery_timing(p) && !self.cant_play_land_card(p, id) {
                    out.push(Action::PlayLand { card: id });
                }
                continue;
            }
            let on_adv = zone == Zone::Exile && self.cards[id.0 as usize].on_adventure;
            let warped = zone == Zone::Exile && self.cards[id.0 as usize].exile_mark == Some(ExileMark::Warped) && self.cards[id.0 as usize].exile_mark_turn < self.turn;
            let prep_spell = zone == Zone::Exile && self.cards[id.0 as usize].prepare_of.is_some();
            let foretold_own_cost = zone == Zone::Exile && self.cards[id.0 as usize].exile_mark == Some(ExileMark::Foretold) && self.cards[id.0 as usize].exile_mark_turn < self.turn && self.cards[id.0 as usize].exile_cast_cost.is_some() && self.cards[id.0 as usize].owner == p;
            let has_alt_from_zone = on_adv || warped || prep_spell || foretold_own_cost
                || self.granted_alt_costs(p, id, zone).iter().any(|g| !g.1.is_empty())
                || d.abilities
                .iter()
                .any(|a| matches!(a, Ability::AlternativeCost { from: Some(z), .. } if *z == zone));
            if has_alt_from_zone {
                self.push_cast_actions(p, id, zone, &mut out);
            } else if zone == Zone::Graveyard {
                self.push_face_cast(p, id, zone, &mut out);
            }
        }
        // a card another player owns, exiled on an adventure by this player's Adventure cast (Robber of the Rich): its caster may cast the creature half
        for opl in self.players.iter().filter(|o| o.id != p) {
            for &id in opl.exile.iter() {
                let c = &self.cards[id.0 as usize];
                if c.zone == Zone::Exile && c.on_adventure && c.adventure_by == Some(p) {
                    self.push_cast_actions(p, id, Zone::Exile, &mut out);
                }
            }
        }
        // a prepare spell copy waits in its owner's exile, but whoever controls the prepared creature now may cast it
        for opl in self.players.iter().filter(|o| o.id != p) {
            for &id in opl.exile.iter() {
                let c = &self.cards[id.0 as usize];
                if let (Zone::Exile, Some(host)) = (c.zone, c.prepare_of) {
                    if self.cards[host.0 as usize].zone == Zone::Battlefield && self.cards[host.0 as usize].cur.controller == p {
                        self.push_cast_actions(p, id, Zone::Exile, &mut out);
                    }
                }
            }
        }
        // cards playable from the graveyard / top of the library through a static permission
        {
            let mut cands: Vec<CardId> = player.graveyard.clone();
            if let Some(&top) = player.library.last() {
                cands.push(top);
            }
            // cards in exile a static lets you play ("cast spells from among cards exiled with this")
            if self.turn_play_grants.iter().any(|g| g.0 == p && g.1 == Zone::Exile && g.4 == self.turn)
                || self.battlefield.iter().copied().chain(self.emblem_srcs()).any(|b| self.db.def(self.cards[b.0 as usize].def).statics.iter().any(|&i| matches!(&self.db.def(self.cards[b.0 as usize].def).abilities[i as usize], Ability::Static { effect: StaticEffect::MayPlayFrom { zone: Zone::Exile, .. }, .. }))) {
                for pl in &self.players {
                    cands.extend(pl.exile.iter().copied());
                }
            } else {
                // a card in exile with its own "you may cast this card from exile" permission
                for &x in &player.exile {
                    let d = self.db.def(self.cards[x.0 as usize].def);
                    if d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::MayPlayFrom { zone: Zone::Exile, .. }, from: Some(Zone::Exile), .. })) {
                        cands.push(x);
                    }
                }
            }
            for (pl, ow, t, _, _, _) in &self.gy_play_grants {
                if *pl == p && *t == self.turn && *ow != p {
                    cands.extend(self.players[*ow as usize].graveyard.iter().copied());
                }
            }
            for (pl, ow, t) in &self.lib_play_grants {
                if *pl == p && *t == self.turn && *ow != p {
                    if let Some(&top) = self.players[*ow as usize].library.last() {
                        cands.push(top);
                    }
                }
            }
            for (pl, t, cid, z) in &self.gy_card_grants {
                if *pl == p && *t == self.turn && self.cards[cid.0 as usize].zone == Zone::Graveyard && self.cards[cid.0 as usize].zcc == *z {
                    cands.push(*cid);
                }
            }
            if self.any_graveyard_perm(p) {
                for pl in &self.players {
                    if pl.id != p {
                        cands.extend(pl.graveyard.iter().copied());
                    }
                }
            }
            if self.any_library_perm(p) {
                for pl in &self.players {
                    if pl.id != p {
                        if let Some(&top) = pl.library.last() {
                            cands.push(top);
                        }
                    }
                }
            }
            if self.effects.iter().any(|e| matches!(e.modif, Modif::PlayOthersGraveyards) && e.players.contains(&p)) {
                for pl in &self.players {
                    if pl.id != p {
                        cands.extend(pl.graveyard.iter().copied());
                    }
                }
            }
            for id in cands {
                if !self.play_permission(p, id) {
                    // the Adventure half of the top card may be allowed (Johann: instants and sorceries) when the card's own types are not
                    let d = self.def_of(id);
                    if self.cards[id.0 as usize].zone == Zone::Library && d.adventure && d.face_castable {
                        self.push_face_cast(p, id, Zone::Library, &mut out);
                    }
                    continue;
                }
                if self.def_of(id).types.has(CardType::Land) {
                    if (player.lands_played as i32) < player.land_limit as i32 + self.extra_land_drops(p) && self.sorcery_timing(p) && !self.cant_play_land_card(p, id) {
                        out.push(Action::PlayLand { card: id });
                    }
                } else {
                    self.push_mdfc_land_face(p, id, &mut out);
                    self.push_cast_actions(p, id, self.cards[id.0 as usize].zone, &mut out);
                }
            }
        }
        // exiled cards this player may play ("you may play that card this turn"), including other players' exile zones
        for pl in &self.players {
            for &id in &pl.exile {
                if self.cards[id.0 as usize].on_adventure || !self.playable_from_exile(p, id) {
                    continue;
                }
                if self.def_of(id).types.has(CardType::Land) {
                    if (player.lands_played as i32) < player.land_limit as i32 + self.extra_land_drops(p) && self.sorcery_timing(p) && !self.cant_play_land_card(p, id) {
                        out.push(Action::PlayLand { card: id });
                    }
                } else {
                    self.push_mdfc_land_face(p, id, &mut out);
                    self.push_cast_actions(p, id, Zone::Exile, &mut out);
                }
            }
        }
        // abilities that work from other zones (cycling, channel, ...)
        for &id in player.hand.iter().chain(player.graveyard.iter()).chain(player.exile.iter()).chain(player.command.iter()).chain(player.library.last()) {
            let zone = self.cards[id.0 as usize].zone;
            let d = self.db.def(self.cards[id.0 as usize].def);
            for &ai in &d.activated {
                if let Some(Ability::Activated { from: Some(z), .. }) = d.abilities.get(ai as usize) {
                    if *z == zone && !d.mana_abilities.contains(&ai) && self.can_activate(p, id, ai) {
                        out.push(Action::Activate { card: id, ability: ai });
                    }
                }
            }
            // abilities granted by a static on the battlefield ("each artifact card in your graveyard has unearth")
            let granted: Vec<std::sync::Arc<Ability>> = self.cards[id.0 as usize].cur.granted.to_vec();
            if !granted.is_empty() && self.zone_granted.contains(&id) {
                for (k, a) in granted.iter().enumerate() {
                    if matches!(**a, Ability::Activated { .. }) && self.can_activate_with(p, id, GRANTED_BASE + k as u16, a) {
                        out.push(Action::Activate { card: id, ability: GRANTED_BASE + k as u16 });
                    }
                }
            }
        }
        // abilities that work while the card is a spell on the stack (Lightning Storm: "any player may activate this ability but only if it is on the stack")
        for it in &self.stack {
            if !matches!(it.kind, ItemKind::Spell { .. }) || it.copy {
                continue;
            }
            let id = it.source;
            let c = &self.cards[id.0 as usize];
            if c.zone != Zone::Stack || (it.controller != p && !self.db.def(c.def).any_player) {
                continue;
            }
            let d = self.db.def(c.def);
            for &ai in &d.activated {
                if let Some(Ability::Activated { from: Some(Zone::Stack), .. }) = d.abilities.get(ai as usize) {
                    if self.can_activate(p, id, ai) {
                        out.push(Action::Activate { card: id, ability: ai });
                    }
                }
            }
        }
        // abilities of permanents
        for &id in &self.battlefield {
            let c = &self.cards[id.0 as usize];
            if c.cur.controller != p && !self.db.def(c.def).any_player {
                // an ability only the attached permanent's controller may activate (Volrath's Curse)
                if let Some(ai) = self.db.def(c.def).enchanted_ctl_ability {
                    if !c.cur.no_abilities && self.can_activate(p, id, ai) {
                        out.push(Action::Activate { card: id, ability: ai });
                    }
                }
                continue;
            }
            if c.cur.controller == p && self.db.def(c.def).opponents_only {
                continue;
            }
            let noab = c.cur.no_abilities;
            let d = self.db.def(c.def);
            // Room: unlock a locked door by paying its mana cost (sorcery speed)
            if d.room && c.cur.controller == p && self.sorcery_timing(p) {
                for door in 1u8..=2 {
                    if c.doors & (1 << (door - 1)) == 0 {
                        let cost = self.unlock_cost(p, id, door);
                        self.pay_for.set(None);
                        self.pay_unlock.set(true);
                        let afford = self.can_afford(p, &cost, 0);
                        self.pay_unlock.set(false);
                        if afford {
                            out.push(Action::UnlockDoor { card: id, door });
                        }
                    }
                }
            }
            // a face-down creature card (manifested / cloaked) may be turned face up by paying its mana cost
            if c.face_down && c.cur.controller == p && d.types.has(CardType::Creature) && (c.cast_alt != Some(crate::decision::FACE_DOWN_ALT) || !d.abilities.iter().any(|a| matches!(a, Ability::Morph { .. }))) && !d.cost.shards.is_empty() {
                self.pay_for.set(Some(id));
                self.pay_is_ability.set(true);
                let afford = self.can_afford(p, &d.cost, 0);
                self.pay_for.set(None);
                self.pay_is_ability.set(false);
                if afford && !self.cant_turn_face_up(id) {
                    out.push(Action::TurnFaceUp { card: id });
                }
            }
            for &ai in &d.activated {
                if d.mana_abilities.contains(&ai) || (noab && !matches!(d.abilities.get(ai as usize), Some(Ability::Activated { effects, .. }) if matches!(effects.as_slice(), [Effect::TurnFaceUp { .. }]))) {
                    continue;
                }
                if self.can_activate(p, id, ai) {
                    out.push(Action::Activate { card: id, ability: ai });
                }
            }
            for (k, a) in c.cur.granted.iter().enumerate() {
                if matches!(**a, Ability::Activated { .. }) && !crate::carddb::is_mana_ability(a) {
                    let idx = GRANTED_BASE + k as u16;
                    if self.can_activate_with(p, id, idx, a) {
                        out.push(Action::Activate { card: id, ability: idx });
                    }
                }
            }
        }
        out
    }

    /// Mana abilities `p` could activate right now (shown as actions to humans so they can tap for mana whenever they have priority).
    pub(crate) fn mana_actions(&self, p: PlayerId) -> Vec<Action> {
        let mut out = vec![];
        for &id in &self.battlefield {
            let c = &self.cards[id.0 as usize];
            if c.cur.controller != p {
                continue;
            }
            let d = self.def_of(id);
            for &ai in &d.mana_abilities {
                if self.can_activate(p, id, ai) {
                    out.push(Action::ActivateMana { card: id, ability: ai });
                }
            }
        }
        out
    }

    /// Casting the other face (modal DFC spell half, split half, adventure) of a card in hand.
    fn push_face_cast(&self, p: PlayerId, id: CardId, zone: Zone, out: &mut Vec<Action>) {
        let d = self.def_of(id);
        // disturb: the back face of a transforming card can be cast from the graveyard through its own AlternativeCost
        // More Than Meets the Eye: the back face of a transforming card can be cast from HAND through its own AlternativeCost {from: Hand}
        let hand_alt = zone == Zone::Hand
            && d.is_front
            && d.other_face.map_or(false, |b| self.db.def(b).abilities.iter().any(|a| matches!(a, Ability::AlternativeCost { from: Some(Zone::Hand), .. })));
        if !d.face_castable && !hand_alt && !(zone == Zone::Graveyard && d.is_front && d.other_face.is_some()) {
            return;
        }
        let Some(back) = d.other_face else { return };
        let bd = self.db.def(back);
        if bd.types.has(CardType::Land) {
            return;
        }
        let adv_gy = zone == Zone::Graveyard && d.adventure && {
            let c = &self.cards[id.0 as usize];
            c.owner == p && c.adv_gy_until != 0 && c.adv_gy_until >= self.turn
        };
        let mut g2 = self.clone();
        g2.cards[id.0 as usize].def = back;
        g2.reset_derived(id);
        let mut tmp = vec![];
        g2.push_cast_actions(p, id, if adv_gy || (zone == Zone::Exile && d.adventure) { Zone::Hand } else { zone }, &mut tmp);
        // aftermath: the other half is cast from the graveyard through its own alternative cost
        let ok = if (zone == Zone::Hand && !hand_alt) || adv_gy || (matches!(zone, Zone::Exile | Zone::Library) && d.adventure) { tmp.iter().any(|a| matches!(a, Action::Cast { alt: None, .. }) || matches!(a, Action::Cast { alt: Some(g), .. } if *g >= GRANT_ALT_BASE && *g < FACE_DOWN_ALT)) } else { tmp.iter().any(|a| matches!(a, Action::Cast { alt: Some(_), .. })) };
        if ok {
            out.push(Action::Cast { card: id, alt: Some(FACE_ALT) });
        }
    }

    /// A cost that gives up a permanent (also as one option of an "or" cost): paid after the mana so the permanent can still be tapped for it.
    fn gives_up_permanent(c: &Cost) -> bool {
        match c {
            Cost::Sacrifice { .. } | Cost::SacrificeTotalPower { .. } | Cost::SacrificeDistinctNames { .. } | Cost::SacrificeAmount { .. } | Cost::SacrificeAll { .. } | Cost::ReturnToHand { .. } | Cost::ExileFromBattlefield { .. } | Cost::ExileCraftMaterials { .. } => true,
            Cost::Or { options } => options.iter().any(|o| o.iter().any(Self::gives_up_permanent)),
            _ => false,
        }
    }

    /// A `MayPlayThisTurn { tapped }` permission lets `p` play this graveyard card (it enters tapped).
    pub(crate) fn gy_tapped_grant(&self, p: PlayerId, id: CardId) -> bool {
        self.turn_play_grants.iter().any(|(pl, z, f, src, t, ids, tapped)| *tapped && *pl == p && *z == Zone::Graveyard && *t == self.turn && (ids.is_empty() || ids.contains(&id)) && self.matches_card(&Filter { controller: PlayerFilter::Any, ..f.clone() }, id, *src, p))
    }

    /// `p` controls a permanent with `MayPlayFrom { zone: Graveyard, any_graveyard }` (cards in every graveyard are offered).
    fn any_graveyard_perm(&self, p: PlayerId) -> bool {
        self.battlefield.iter().any(|&b| {
            let sc = &self.cards[b.0 as usize];
            sc.cur.controller == p && !sc.cur.no_abilities && self.db.def(sc.def).statics.iter().any(|&i| matches!(&self.db.def(sc.def).abilities[i as usize], Ability::Static { effect: StaticEffect::MayPlayFrom { zone: Zone::Graveyard, any_graveyard: true, .. }, .. }))
        })
    }

    /// `p` controls a permanent (or face-up plane) with `MayPlayFrom { zone: Library, any_library }` (the top card of every library is offered).
    fn any_library_perm(&self, p: PlayerId) -> bool {
        self.battlefield.iter().copied().chain(self.emblem_srcs()).any(|b| {
            let sc = &self.cards[b.0 as usize];
            !sc.cur.no_abilities && self.src_ctrl(b) == p && self.db.def(sc.def).statics.iter().any(|&i| matches!(&self.db.def(sc.def).abilities[i as usize], Ability::Static { effect: StaticEffect::MayPlayFrom { zone: Zone::Library, any_library: true, .. }, .. }))
        })
    }

    /// `PlayTopFreeWhileOnTop`: `p` may play this card free because it was revealed this turn and has stayed on top of its owner's library.
    pub(crate) fn lib_free_grant(&self, p: PlayerId, id: CardId) -> bool {
        let c = &self.cards[id.0 as usize];
        c.zone == Zone::Library && self.players[c.owner as usize].library.last() == Some(&id) && self.lib_free_grants.iter().any(|(pl, t, cid, z)| *pl == p && *t == self.turn && *cid == id && *z == c.zcc)
    }

    /// The index of a filtered `MayPlayFromOutsideGame` grant (this turn) that lets `p` play this sideboard card.
    pub(crate) fn outside_card_grant_idx(&self, p: PlayerId, id: CardId) -> Option<usize> {
        if self.cards[id.0 as usize].owner != p {
            return None;
        }
        self.outside_card_grants.iter().position(|(pl, t, f, src, _)| *pl == p && *t == self.turn && self.matches_card(f, id, *src, p))
    }

    /// A static "you may play cards from your graveyard / the top of your library" lets `p` play this card.
    pub(crate) fn play_permission(&self, p: PlayerId, id: CardId) -> bool {
        let c = &self.cards[id.0 as usize];
        if c.owner != p && c.zone == Zone::Graveyard && self.effects.iter().any(|e| matches!(e.modif, Modif::PlayOthersGraveyards) && e.players.contains(&p)) {
            return true;
        }
        if c.zone == Zone::Graveyard && self.gy_play_grants.iter().any(|(pl, ow, t, f, src, _)| *pl == p && *ow == c.owner && *t == self.turn && self.matches_card(&Filter { controller: PlayerFilter::Any, ..f.clone() }, id, *src, p)) {
            return true;
        }
        if c.zone == Zone::Graveyard && self.gy_free_grants.iter().any(|(pl, t, cid, z)| *pl == p && *t == self.turn && *cid == id && *z == c.zcc) {
            return true;
        }
        if c.zone == Zone::Graveyard && c.owner == p && self.gy_once_grants.iter().any(|(pl, t, f, src, _, _)| *pl == p && *t == self.turn && self.matches_card(&Filter { controller: PlayerFilter::Any, ..f.clone() }, id, *src, p)) {
            return true;
        }
        if c.zone == Zone::Graveyard && self.gy_card_grants.iter().any(|(pl, t, cid, z)| *pl == p && *t == self.turn && *cid == id && *z == c.zcc) {
            return true;
        }
        if c.zone == Zone::Sideboard {
            return c.owner == p && ((c.is_companion && self.def_of(id).abilities.iter().any(|a| matches!(a, Ability::OldCompanion))) || self.outside_grants.iter().any(|&(pl, t)| pl == p && t == self.turn) || self.outside_card_grant_idx(p, id).is_some());
        }
        if c.zone == Zone::Library && self.lib_free_grant(p, id) {
            return true;
        }
        if c.zone == Zone::Library && self.players[c.owner as usize].library.last() == Some(&id) && self.lib_play_grants.iter().any(|(pl, ow, t)| *pl == p && *ow == c.owner && *t == self.turn) {
            return true;
        }
        if c.owner != p && c.zone != Zone::Exile && !(c.zone == Zone::Graveyard && self.any_graveyard_perm(p)) && !(c.zone == Zone::Library && self.any_library_perm(p)) {
            return false;
        }
        let zone = c.zone;
        if zone == Zone::Library && self.players[c.owner as usize].library.last() != Some(&id) {
            return false;
        }
        if zone != Zone::Graveyard && zone != Zone::Library && zone != Zone::Exile {
            return false;
        }
        // the card's own permission ("you may cast this from your graveyard if ...") works from the zone it names
        let own = {
            let sd = self.db.def(c.def);
            sd.statics.iter().any(|&i| {
                matches!(&sd.abilities[i as usize], Ability::Static { effect: StaticEffect::MayPlayFrom { zone: z, filter, .. }, condition, from: Some(fz) }
                    if *z == zone
                        && *fz == zone
                        && self.matches_card(&Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() }, id, id, p)
                        && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(id, p))))
            })
        };
        own
            || (zone == Zone::Graveyard && self.gy_play_grants.iter().any(|(pl, ow, t, f, src, _)| *pl == p && *ow == c.owner && *t == self.turn && self.matches_card(&Filter { controller: PlayerFilter::Any, ..f.clone() }, id, *src, p)))
            || self.turn_play_grants.iter().any(|(pl, z, f, src, t, ids, _)| *pl == p && *z == zone && *t == self.turn && (ids.is_empty() || ids.contains(&id)) && self.matches_card(&Filter { controller: PlayerFilter::Any, ..f.clone() }, id, *src, p))
            || self.battlefield.iter().copied().chain(self.emblem_srcs()).any(|src| {
            let sc = &self.cards[src.0 as usize];
            if sc.cur.no_abilities {
                return false;
            }
            let sctl = if sc.zone == Zone::Command { sc.owner } else { sc.cur.controller };
            let sd = self.db.def(sc.def);
            sd.statics.iter().any(|&i| {
                matches!(&sd.abilities[i as usize], Ability::Static { effect: StaticEffect::MayPlayFrom { zone: z, filter, once_per_turn, per_type, per_kind, active_player, exiler, .. }, condition, .. }
                    if *z == zone
                        && (if *exiler { c.exiled_by == Some(p) } else { sctl == p || (*active_player && p == self.active) })
                        && !(*once_per_turn && sc.may_play_used_turn == self.turn)
                        && (!(*per_type || *per_kind) || self.per_type_pick(sc, id, *per_kind).is_some())
                        && self.matches_card(&Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() }, id, src, p)
                        && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(src, p))))
            })
        })
    }

    /// A once-each-turn `MayPlayFrom` permission was used (`b`): outside-the-game grants made by the same permanent are used up too
    /// (The Holodeck: "once during each of your turns, you may play a permanent card you own from outside the game or from your graveyard").
    pub(crate) fn once_play_used(&mut self, b: CardId) {
        self.cards[b.0 as usize].may_play_used_turn = self.turn;
        let turn = self.turn;
        self.outside_card_grants.retain(|g| !(g.3 == b && g.1 == turn));
    }

    /// A card was played from outside the game through a grant made by `src`: if `src` also has a once-each-turn `MayPlayFrom`, that is used up as well.
    pub(crate) fn outside_play_used(&mut self, src: CardId) {
        let has = {
            let sc = &self.cards[src.0 as usize];
            sc.zone == Zone::Battlefield && self.db.def(sc.def).statics.iter().any(|&i| matches!(&self.db.def(sc.def).abilities[i as usize], Ability::Static { effect: StaticEffect::MayPlayFrom { once_per_turn: true, .. }, .. }))
        };
        if has {
            self.cards[src.0 as usize].may_play_used_turn = self.turn;
        }
    }

    /// The permanent whose once-each-turn `MayPlayFrom` permission would pay for `p` playing / casting `card` from `from` (and its `leave_exile_gain`).
    pub(crate) fn once_play_src(&self, p: PlayerId, card: CardId, from: Zone) -> Option<(CardId, u8)> {
        if !matches!(from, Zone::Graveyard | Zone::Library | Zone::Exile) {
            return None;
        }
        self.battlefield.iter().copied().find_map(|b| {
            let sc = &self.cards[b.0 as usize];
            if sc.cur.controller != p || sc.cur.no_abilities || sc.may_play_used_turn == self.turn {
                return None;
            }
            let sd = self.db.def(sc.def);
            sd.statics.iter().find_map(|&i| match &sd.abilities[i as usize] {
                Ability::Static { effect: StaticEffect::MayPlayFrom { zone: z, filter, once_per_turn: true, leave_exile_gain, leave_exile, .. }, .. }
                    if *z == from && self.matches_card(&Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() }, card, b, p) =>
                {
                    Some((b, if *leave_exile { u8::MAX } else { *leave_exile_gain }))
                }
                _ => None,
            })
        })
    }

    /// Does a `MayPlayFrom{free}` permission of `p` (not yet used this turn) let them cast `card` from exile without paying its mana cost?
    pub(crate) fn play_free_perm(&self, p: PlayerId, card: CardId) -> bool {
        self.battlefield.iter().any(|&b| {
            let sc = &self.cards[b.0 as usize];
            if sc.cur.controller != p || sc.cur.no_abilities {
                return false;
            }
            let sd = self.db.def(sc.def);
            sd.statics.iter().any(|&i| {
                matches!(&sd.abilities[i as usize], Ability::Static { effect: StaticEffect::MayPlayFrom { zone: Zone::Exile, filter, once_per_turn, free: true, .. }, condition, .. }
                    if !(*once_per_turn && sc.may_play_used_turn == self.turn)
                        && self.matches_card(&Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() }, card, b, p)
                        && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(b, p))))
            })
        })
    }

    /// Life `p` must pay on top of the other costs to cast `card` from `from` through a `MayPlayFrom{extra_life}` permission (no alternative cost).
    pub(crate) fn play_extra_life(&self, p: PlayerId, card: CardId, from: Zone) -> i32 {
        if !matches!(from, Zone::Graveyard | Zone::Library | Zone::Exile) {
            return 0;
        }
        let mut best = 0;
        for &b in &self.battlefield {
            let sc = &self.cards[b.0 as usize];
            if sc.cur.controller != p || sc.cur.no_abilities {
                continue;
            }
            let sd = self.db.def(sc.def);
            for &i in &sd.statics {
                if let Ability::Static { effect: StaticEffect::MayPlayFrom { zone: z, filter, extra_life, .. }, condition, .. } = &sd.abilities[i as usize] {
                    if *extra_life > 0
                        && *z == from
                        && self.matches_card(&Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() }, card, b, p)
                        && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(b, p)))
                    {
                        best = best.max(*extra_life as i32);
                    }
                }
            }
        }
        best
    }

    /// The smallest X a spell's `CastOnlyIf {X >= n}` (or `X > n`) allows; 0 when it has none.
    pub(crate) fn cast_x_floor(d: &crate::carddb::Def) -> i32 {
        d.abilities
            .iter()
            .filter_map(|a| match a {
                Ability::CastOnlyIf { condition: Condition::Compare { a: Amount::X, cmp } } => match cmp {
                    Cmp::Ge { n } => Some(*n),
                    Cmp::Gt { n } => Some(*n + 1),
                    _ => None,
                },
                _ => None,
            })
            .max()
            .unwrap_or(0)
            .max(0)
    }

    /// Extra costs of the `MayPlayFrom{extra_cost}` permission `p` would use to cast `card` from `from` (no alternative cost).
    pub(crate) fn play_extra_costs(&self, p: PlayerId, card: CardId, from: Zone) -> Vec<Cost> {
        if !matches!(from, Zone::Graveyard | Zone::Library | Zone::Exile) {
            return vec![];
        }
        for &b in &self.battlefield {
            let sc = &self.cards[b.0 as usize];
            if sc.cur.controller != p || sc.cur.no_abilities {
                continue;
            }
            let sd = self.db.def(sc.def);
            for &i in &sd.statics {
                if let Ability::Static { effect: StaticEffect::MayPlayFrom { zone: z, filter, extra_cost, .. }, condition, .. } = &sd.abilities[i as usize] {
                    if !extra_cost.is_empty()
                        && *z == from
                        && self.matches_card(&Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() }, card, b, p)
                        && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(b, p)))
                    {
                        return extra_cost.clone();
                    }
                }
            }
        }
        vec![]
    }

    /// Does a `MayPlayFrom{sac_exile}` permission of `p` (the one that would be used) require sacrificing a creature to cast `card` from `from`, and exile it afterwards (Maestros Ascendancy)?
    pub(crate) fn play_sac_exile(&self, p: PlayerId, card: CardId, from: Zone) -> bool {
        if !matches!(from, Zone::Graveyard | Zone::Library | Zone::Exile) {
            return false;
        }
        self.battlefield.iter().any(|&b| {
            let sc = &self.cards[b.0 as usize];
            if sc.cur.controller != p || sc.cur.no_abilities || sc.may_play_used_turn == self.turn {
                return false;
            }
            let sd = self.db.def(sc.def);
            sd.statics.iter().any(|&i| {
                matches!(&sd.abilities[i as usize], Ability::Static { effect: StaticEffect::MayPlayFrom { zone: z, filter, sac_exile: true, .. }, condition, .. }
                    if *z == from
                        && self.matches_card(&Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() }, card, b, p)
                        && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(b, p))))
            })
        })
    }

    /// Emblems (token cards in a command zone) and face-up planes, phenomena and schemes, which carry static abilities too.
    pub(crate) fn emblem_srcs(&self) -> Vec<CardId> {
        self.players.iter().flat_map(|pl| pl.command.iter().copied()).filter(|id| {
            let d = self.db.def(self.cards[id.0 as usize].def);
            d.token || d.types.has(CardType::Plane) || d.types.has(CardType::Phenomenon) || d.types.has(CardType::Scheme)
        }).collect()
    }

    /// The controller of a static / triggered ability source: a command-zone card's owner, else its controller.
    pub(crate) fn src_ctrl(&self, id: CardId) -> PlayerId {
        let c = &self.cards[id.0 as usize];
        if c.zone == Zone::Command { c.owner } else { c.cur.controller }
    }

    /// Does a `MayPlayFrom{exile_after}` permission of `p` (still unused this turn) let `p` cast `card` from `from`? Then the spell is exiled instead of going to the graveyard (Kess, Dissident Mage).
    pub(crate) fn play_perm_exile_after(&self, p: PlayerId, card: CardId, from: Zone) -> bool {
        if !matches!(from, Zone::Graveyard | Zone::Library | Zone::Exile) {
            return false;
        }
        self.battlefield.iter().copied().chain(self.emblem_srcs()).any(|b| {
            let sc = &self.cards[b.0 as usize];
            let sctl = if sc.zone == Zone::Command { sc.owner } else { sc.cur.controller };
            if sctl != p || sc.cur.no_abilities || (sc.may_play_used_turn == self.turn) {
                return false;
            }
            let sd = self.db.def(sc.def);
            sd.statics.iter().any(|&i| {
                matches!(&sd.abilities[i as usize], Ability::Static { effect: StaticEffect::MayPlayFrom { zone: z, filter, exile_after: true, .. }, condition, .. }
                    if *z == from
                        && self.matches_card(&Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() }, card, b, p)
                        && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(b, p))))
            })
        })
    }

    /// Does a cost-change amount depend on the number of kicks paid (`KickCount`, possibly inside a `Plus`: Dargo, the Shipwrecker)?
    pub(crate) fn amount_uses_kicks(a: &Amount) -> bool {
        match a {
            Amount::KickCount => true,
            Amount::Plus { a, b } => Self::amount_uses_kicks(a) || Self::amount_uses_kicks(b),
            _ => false,
        }
    }

    /// The permanent whose `MayPlayFrom{exile_top}` permission lets `p` play / cast `card` from `from`: playing it then exiles the top card of `p`'s library (Share the Spoils).
    pub(crate) fn play_exile_top_src(&self, p: PlayerId, card: CardId, from: Zone) -> Option<CardId> {
        if from != Zone::Exile {
            return None;
        }
        self.battlefield.iter().copied().chain(self.emblem_srcs()).find(|&b| {
            let sc = &self.cards[b.0 as usize];
            if sc.cur.no_abilities {
                return false;
            }
            let sd = self.db.def(sc.def);
            sd.statics.iter().any(|&i| {
                matches!(&sd.abilities[i as usize], Ability::Static { effect: StaticEffect::MayPlayFrom { zone: Zone::Exile, filter, exile_top: true, active_player, .. }, condition, .. }
                    if (self.src_ctrl(b) == p || (*active_player && p == self.active))
                        && self.matches_card(&Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() }, card, b, p)
                        && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(b, p))))
            })
        })
    }

    /// Exile the top card of `p`'s library as a permission source's reward (`MayPlayFrom{exile_top}`).
    pub(crate) fn exile_top_for(&mut self, p: PlayerId, src: CardId) {
        if let Some(&id) = self.players[p as usize].library.last() {
            self.cur_src = Some(src);
            self.move_card(id, Zone::Exile, crate::zones::MoveOpts::default());
        }
    }

    /// The source of a `MayPlayFrom{mark_played}` permission that lets `p` play `card` from `from` (call before the card moves).
    pub(crate) fn play_mark_src(&self, p: PlayerId, card: CardId, from: Zone) -> Option<CardId> {
        self.battlefield.iter().copied().chain(self.emblem_srcs()).find(|&b| {
            let sc = &self.cards[b.0 as usize];
            if sc.cur.no_abilities {
                return false;
            }
            let sd = self.db.def(sc.def);
            sd.statics.iter().any(|&i| {
                matches!(&sd.abilities[i as usize], Ability::Static { effect: StaticEffect::MayPlayFrom { zone, filter, mark_played: true, active_player, .. }, condition, .. }
                    if *zone == from
                        && (self.src_ctrl(b) == p || (*active_player && p == self.active))
                        && self.matches_card(&Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() }, card, b, p)
                        && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(b, p))))
            })
        })
    }

    /// The permanent type a `MayPlayFrom{per_type}` permission of `src` would use up for `card` this turn (None = all its permanent types are used).
    pub(crate) fn per_type_pick(&self, src: &Card, card: CardId, kinds: bool) -> Option<CardType> {
        let used = if src.may_play_types_turn == self.turn { src.may_play_types } else { 0 };
        let d = self.def_of(card);
        if kinds {
            // two buckets: lands (CardType::Land) and spells (CardType::Creature stands for "any nonland card")
            let t = if d.types.has(CardType::Land) { CardType::Land } else { CardType::Creature };
            return (used & (1 << (t as u32)) == 0).then_some(t);
        }
        [CardType::Land, CardType::Artifact, CardType::Enchantment, CardType::Planeswalker, CardType::Battle, CardType::Creature]
            .into_iter()
            .find(|t| d.types.has(*t) && used & (1 << (*t as u32)) == 0)
    }

    /// Use up the permanent type of every `per_type` permission that lets `p` play `card` from `from` (call before the card moves).
    pub(crate) fn use_per_type(&mut self, p: PlayerId, card: CardId, from: Zone) {
        let mut hits: Vec<(CardId, CardType)> = vec![];
        for &b in &self.battlefield {
            let sc = &self.cards[b.0 as usize];
            if sc.cur.controller != p || sc.cur.no_abilities {
                continue;
            }
            let sd = self.db.def(sc.def);
            let kinds = sd.statics.iter().find_map(|&i| match &sd.abilities[i as usize] {
                Ability::Static { effect: StaticEffect::MayPlayFrom { zone: z, filter, per_type, per_kind, .. }, .. }
                    if (*per_type || *per_kind) && *z == from && self.matches_card(&Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() }, card, b, p) =>
                {
                    Some(*per_kind)
                }
                _ => None,
            });
            if let Some(kinds) = kinds {
                if let Some(t) = self.per_type_pick(sc, card, kinds) {
                    hits.push((b, t));
                    break; // one permission pays for the play
                }
            }
        }
        for (b, t) in hits {
            let sc = &mut self.cards[b.0 as usize];
            if sc.may_play_types_turn != self.turn {
                sc.may_play_types_turn = self.turn;
                sc.may_play_types = 0;
            }
            sc.may_play_types |= 1 << (t as u32);
        }
    }

    /// Does `p` hold a play-from-exile permission for this card right now?
    pub(crate) fn playable_from_exile(&self, p: PlayerId, id: CardId) -> bool {
        let c = &self.cards[id.0 as usize];
        c.zone == Zone::Exile
            && c.play_until > 0
            && c.play_until >= self.turn
            && !(c.play_stop_end && c.play_until == self.turn && matches!(self.step, Step::End | Step::Cleanup))
            && !(c.play_stop_combat && c.play_until == self.turn && matches!(self.step, Step::Main2 | Step::End | Step::Cleanup))
            && c.play_by == p
            && (!c.play_per_type || self.per_type_available(id))
            && c.play_src.map_or(true, |s| {
                let sc = &self.cards[s.0 as usize];
                sc.zone == Zone::Battlefield && sc.cur.controller == p
            })
            && c.play_cond.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(id, p)))
    }

    fn push_cast_actions(&self, p: PlayerId, id: CardId, zone: Zone, out: &mut Vec<Action>) {
        let d = self.def_of(id);
        if !d.is_permanent() && !d.types.has(CardType::Instant) && !d.types.has(CardType::Sorcery) {
            return;
        }
        let mut alts: Vec<Option<u8>> = vec![];
        // a land is never cast face up
        let uncastable = d.no_mana_cost || d.types.has(CardType::Land);
        // a tiered instant/sorcery has no effects of its own: it can only be cast through one of its tier alternatives
        let tiered_only = (d.types.has(CardType::Instant) || d.types.has(CardType::Sorcery))
            && !d.abilities.iter().any(|a| matches!(a, Ability::Spell { .. } | Ability::Modal { .. }))
            && d.abilities.iter().any(|a| matches!(a, Ability::AlternativeCost { effects: Some(e), from: None, .. } if !matches!(e.as_slice(), [Effect::Nothing])));
        if !uncastable && !tiered_only && zone == Zone::Hand || zone == Zone::Command || (zone == Zone::Exile && (self.cards[id.0 as usize].on_adventure || self.cards[id.0 as usize].prepare_of.is_some() || (self.cards[id.0 as usize].exile_mark == Some(ExileMark::Warped) && self.cards[id.0 as usize].exile_mark_turn < self.turn) || (self.cards[id.0 as usize].exile_mark == Some(ExileMark::Foretold) && self.cards[id.0 as usize].exile_mark_turn < self.turn && self.cards[id.0 as usize].exile_cast_cost.is_some() && self.cards[id.0 as usize].owner == p) || self.playable_from_exile(p, id))) || ((zone == Zone::Graveyard || zone == Zone::Library || zone == Zone::Exile || zone == Zone::Sideboard) && self.play_permission(p, id)) {
            alts.push(None);
        }
        let mut ai = 0u8;
        for a in &d.abilities {
            if let Ability::AlternativeCost { from, condition, .. } = a {
                let cond_ok = condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(id, p)));
                // an alternative cost with no zone of its own (mutate, evoke, dash, overload ...) can be used when the spell is cast from the library or exile by a permission (Garruk's Horde)
                let anywhere = from.is_none() && matches!(zone, Zone::Library | Zone::Exile) && self.play_permission(p, id);
                if (from.unwrap_or(Zone::Hand) == zone || anywhere) && cond_ok && !(zone == Zone::Graveyard && self.graveyard_abilities_off()) {
                    alts.push(Some(ai));
                }
                ai += 1;
            }
        }
        for (k, g) in self.granted_alt_costs(p, id, zone).iter().enumerate() {
            if !g.1.is_empty() && !uncastable && (g.4.is_empty() || if g.4.iter().any(|c| matches!(c, Cost::Discard { .. })) { self.cost_parts_payable(p, id, &g.4) } else { self.can_pay_costs(p, id, &g.4) }) {
                alts.push(Some(GRANT_ALT_BASE + k as u8));
            }
        }
        if zone == Zone::Hand {
            self.push_face_cast(p, id, Zone::Hand, out);
            if d.abilities.iter().any(|a| matches!(a, Ability::Morph { .. })) && d.is_permanent() {
                alts.push(Some(FACE_DOWN_ALT));
            }
        } else if (zone == Zone::Library || (zone == Zone::Exile && !alts.is_empty())) && d.adventure && d.face_castable {
            // the Adventure half of a card cast from the library / exile by a permission (Kellan, Robber of the Rich)
            self.push_face_cast(p, id, zone, out);
        } else if zone == Zone::Library && self.play_permission(p, id) && d.abilities.iter().any(|a| matches!(a, Ability::Morph { .. })) && d.is_permanent() {
            // a morph card cast from the top of the library by a permission can still be cast face down (Into the Pit)
            alts.push(Some(FACE_DOWN_ALT));
        }
        for alt in alts {
            let alt_ab = alt.filter(|&i| i != FACE_DOWN_ALT).and_then(|i| d.abilities.iter().filter(|a| matches!(a, Ability::AlternativeCost { .. })).nth(i as usize));
            let alt_flash = matches!(alt_ab, Some(Ability::AlternativeCost { flash: true, .. }))
                || alt.map_or(false, |i| i >= GRANT_ALT_BASE && i < FACE_DOWN_ALT && self.granted_alt_costs(p, id, zone).get((i - GRANT_ALT_BASE) as usize).map_or(false, |g| g.4.iter().any(|c| matches!(c, Cost::AltFlash))));
            if !alt_flash && !self.can_cast_timing(p, id) {
                continue;
            }
            let (mut specs, modal) = self.spell_specs(id);
            // cast face down it is a 2/2 creature spell with no abilities: no Aura target, no modes
            let (mut specs, modal) = if alt == Some(FACE_DOWN_ALT) { (vec![], None) } else { (std::mem::take(&mut specs), modal) };
            // an alternative cost with its own effects (overload, cleave) brings its own targets
            if let Some(Ability::AlternativeCost { effects: Some(_), targets, .. }) = alt_ab {
                specs = targets.clone().unwrap_or_default();
            }
            if modal.is_none() && !specs.is_empty() {
                let ok = if self.def_of(id).cost.has_x() {
                    // the targets may depend on X (Spell Blast): is there some X that works?
                    let cost0 = self.effective_cost(p, id, alt);
                    let max = (self.max_x(p, &cost0, id, self.x_red.get()) + self.kick_x_bonus(p, id)).clamp(0, 12);
                    // a reduction that depends on the chosen target (Lullmage's Domination) must be affordable for a target that fits X
                    let none: [Target; 0] = [];
                    let pessimistic = self.effective_cost_t(p, id, alt, Some((&none, &[])));
                    let pess_red = self.x_red.get();
                    let cost0 = self.effective_cost(p, id, alt);
                    let target_dep = pessimistic != cost0 || pess_red != self.x_red.get();
                    (0..=max).any(|x| {
                        let mut g2 = self.clone();
                        g2.cards[id.0 as usize].x = x;
                        if !g2.specs_satisfiable(&specs, id, p) {
                            return false;
                        }
                        if !target_dep {
                            return true;
                        }
                        g2.legal_targets(&specs[0], id, p).into_iter().any(|o| {
                            let t = [Target { obj: o, zcc: 0 }];
                            let c2 = g2.effective_cost_t(p, id, alt, Some((&t, &[0])));
                            g2.max_x(p, &c2, id, g2.x_red.get()) + g2.kick_x_bonus(p, id) >= x
                        })
                    })
                } else {
                    self.specs_satisfiable(&specs, id, p)
                };
                if !ok {
                    continue;
                }
            }
            // 601.2c: a modal spell needs enough modes whose targets can all be chosen
            if let (Some((min, _, _)), false) = (modal, self.def_of(id).cost.has_x()) {
                if min > 0 {
                    let usable = self
                        .def_of(id)
                        .abilities
                        .iter()
                        .find_map(|a| if let Ability::Modal { modes, allow_repeat, .. } = a { Some(modes.iter().filter(|m| self.specs_satisfiable(&m.targets, id, p)).count().min(if *allow_repeat { usize::MAX } else { usize::MAX }).max(if *allow_repeat && modes.iter().any(|m| self.specs_satisfiable(&m.targets, id, p)) { min as usize } else { 0 })) } else { None })
                        .unwrap_or(usize::MAX);
                    if usable < min as usize {
                        continue;
                    }
                }
            }
            if false {
                continue;
            }
            // a card with no mana cost can't be cast from the graveyard by paying its mana cost (flashback granted by Snapcaster Mage)
            if zone == Zone::Graveyard && alt.is_none() && d.cost.shards.is_empty() && !d.types.has(CardType::Land)
                && !self.gy_free_grants.iter().any(|(pl, t, cid, z)| *pl == p && *t == self.turn && *cid == id && *z == self.cards[id.0 as usize].zcc)
            {
                continue;
            }
            let cost = self.effective_cost(p, id, alt);
            let base_cost = self.cost_after_alt_help(p, id, &cost);
            // "you can't spend mana to cast this spell": convoke / delve must pay all of it
            if d.abilities.iter().any(|a| matches!(a, Ability::CantSpendManaToCast)) && base_cost.shards.iter().any(|s| !matches!(s, Shard::Generic { n: 0 })) {
                continue;
            }
            // spree: at least one mode's cost must be affordable on top
            let mut variants: Vec<ManaCost> = vec![base_cost.clone()];
            if let Some(Ability::Modal { modes, .. }) = d.abilities.iter().find(|a| matches!(a, Ability::Modal { modes, .. } if modes.iter().all(|m| !m.cost.is_empty()))) {
                variants.clear();
                for m in modes {
                    let mut ac = base_cost.clone();
                    for part in &m.cost {
                        if let Cost::Mana { cost } = part {
                            if let Ok(x) = ManaCost::parse(cost) {
                                ac.shards.extend(x.shards);
                            }
                        }
                    }
                    variants.push(ac);
                }
            }
            // emerge / offering / harmonize: the best permanent to give up decides what is affordable
            let granted_reducer = alt.filter(|&i| i >= GRANT_ALT_BASE && i < FACE_DOWN_ALT).and_then(|i| {
                self.granted_alt_costs(p, id, zone).get((i - GRANT_ALT_BASE) as usize).and_then(|g| {
                    g.4.iter().find_map(|c| match c {
                        Cost::SacrificeReduceByManaValue { filter } => Some((filter.clone(), 0)),
                        Cost::SacrificeReduceByManaCost { filter } => Some((filter.clone(), 1)),
                        Cost::TapReduceByPower { filter } => Some((filter.clone(), 2)),
                        _ => None,
                    })
                })
            });
            if let Some((rf, rk)) = Self::alt_reducer(d, alt.filter(|&i| i != FACE_DOWN_ALT && i < GRANT_ALT_BASE)).or(granted_reducer) {
                let cands: Vec<CardId> = if rk == 2 { self.tappable_for_cost(p, id, &rf) } else { self.eligible_battlefield(&rf, id, p) };
                let mut out = vec![];
                for v in &variants {
                    for &cand in &cands {
                        let mut m = v.clone();
                        self.reduce_mana_by(&mut m, rk, cand);
                        out.push(m);
                    }
                }
                variants = out;
            }
            // an optional static additional cost that removes a coloured shard (Defiler of Dreams)
            if zone == Zone::Hand || zone == Zone::Command || zone == Zone::Graveyard || zone == Zone::Exile || zone == Zone::Library {
                for (ocost, color) in self.optional_spell_costs(p, id) {
                    if !self.cost_parts_payable(p, id, &ocost) {
                        continue;
                    }
                    let reduced: Vec<ManaCost> = variants
                        .iter()
                        .filter_map(|v| {
                            let pos = v.shards.iter().position(|s| matches!(s, Shard::Color { c } if *c == color))?;
                            let mut m = v.clone();
                            m.shards.remove(pos);
                            Some(m)
                        })
                        .collect();
                    variants.extend(reduced);
                }
            }
            // remove counters from your creatures to lower the generic cost (Hierophant Bio-Titan)
            if let Some((_, per)) = self.frenzy_params(id) {
                let mut extra_v: Vec<ManaCost> = vec![];
                for v in &variants {
                    let k = self.frenzy_max(p, id, v);
                    if k > 0 {
                        let mut m = v.clone();
                        let mut cut = k * per;
                        for sh in m.shards.iter_mut() {
                            if let Shard::Generic { n } = sh {
                                let r = (*n as i32).min(cut);
                                *n -= r as u8;
                                cut -= r;
                            }
                        }
                        m.shards.retain(|s| !matches!(s, Shard::Generic { n: 0 }));
                        extra_v.push(m);
                    }
                }
                variants.extend(extra_v);
            }
            // bargain-like kickers (a non-mana additional cost) with a "costs {N} less if kicked" self CostChange: the reduced cost is a further affordable variant (Ice Out)
            if alt.is_none() && d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::CostChange { filter, .. }, condition: Some(Condition::Kicked), .. } if filter.this)) {
                for ab in &d.abilities {
                    if let Ability::Kicker { cost, multi: false, .. } = ab {
                        if cost.iter().all(|c| !matches!(c, Cost::Mana { .. })) && self.cost_parts_payable(p, id, cost) {
                            let mut g2 = self.clone();
                            g2.cards[id.0 as usize].kicked = true;
                            let kc = g2.effective_cost(p, id, alt);
                            variants.push(self.cost_after_alt_help(p, id, &kc));
                        }
                    }
                }
            }
            // a multikicker with no mana part whose kicks lower the cost ("costs {1} less for each creature tapped this way": Explosive Singularity):
            // the cost after the most kicks that can be paid is a further affordable variant
            if alt.is_none() && d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::CostChange { filter, per: Some(per), .. }, .. } if filter.this && Self::amount_uses_kicks(per))) {
                for ab in &d.abilities {
                    if let Ability::Kicker { cost, multi: true, .. } = ab {
                        if cost.iter().all(|c| !matches!(c, Cost::Mana { .. })) {
                            let mut best = 0u32;
                            for k in 1..=20u32 {
                                let extras: Vec<Cost> = match cost.as_slice() {
                                    [Cost::TapPermanents { filter, n }] => vec![Cost::TapPermanents { filter: filter.clone(), n: *n * k as i32 }],
                                    [Cost::Sacrifice { filter, n }] => vec![Cost::Sacrifice { filter: filter.clone(), n: *n * k as i32 }],
                                    _ => (0..k).flat_map(|_| cost.iter().cloned()).collect(),
                                };
                                if self.cost_parts_payable(p, id, &extras) {
                                    best = k;
                                } else {
                                    break;
                                }
                            }
                            if best > 0 {
                                let mut g2 = self.clone();
                                g2.cards[id.0 as usize].kicked = true;
                                g2.cards[id.0 as usize].kicks = best as u8;
                                let kc = g2.effective_cost(p, id, alt);
                                variants.push(self.cost_after_alt_help(p, id, &kc));
                            }
                        }
                    }
                }
            }
            // an "any number" additional cost (`XTimes`) with a self `CostChange` per X (Gorex, Torgaar): the cost after the most X that can be paid is a further affordable variant
            if alt.is_none() && d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::CostChange { filter, per: Some(Amount::X), .. }, .. } if filter.this)) {
                for ab in &d.abilities {
                    if let Ability::AdditionalCost { cost } = ab {
                        for part in cost {
                            if let Cost::XTimes { cost: inner } = part {
                                let prev = self.pay_for.replace(Some(id));
                                let mut best = 0;
                                for k in 1..=40 {
                                    if self.simple_cost_ok(p, id, &Self::scale_cost(inner, k)) { best = k } else { break }
                                }
                                self.pay_for.set(prev);
                                if best > 0 {
                                    let mut g2 = self.clone();
                                    g2.cards[id.0 as usize].x = best;
                                    let kc = g2.effective_cost(p, id, alt);
                                    variants.push(self.cost_after_alt_help(p, id, &kc));
                                }
                            }
                        }
                    }
                }
            }
            self.pay_for.set(Some(id));
            // cast face down: restricted mana for "a face-down creature spell" counts (Qarsi Deceiver)
            self.face_down_cast.set(if alt == Some(FACE_DOWN_ALT) { Some(id) } else { None });
            self.pay_alt_probe.set(Some(alt));
            let mut sac_parts: Vec<Cost> = d.abilities.iter().filter_map(|a| if let Ability::AdditionalCost { cost } = a { Some(cost.iter().cloned()) } else { None }).flatten().collect();
            if let Some(i) = alt {
                if let Some(Ability::AlternativeCost { cost, .. }) = d.abilities.iter().filter(|a| matches!(a, Ability::AlternativeCost { .. })).nth(i as usize) {
                    sac_parts.extend(cost.iter().cloned());
                }
            }
            let prev_reserve = self.sac_reserve.replace(self.forced_sac_reserve(p, id, &sac_parts));
            let affordable = variants.iter().any(|v| self.can_afford(p, v, Self::cast_x_floor(d) as u32));
            *self.sac_reserve.borrow_mut() = prev_reserve;
            self.pay_alt_probe.set(None);
            self.face_down_cast.set(None);
            self.pay_for.set(None);
            if !affordable {
                continue;
            }
            // pay life equal to the mana value rather than mana
            let life_mv = match alt {
                Some(i) if i >= GRANT_ALT_BASE && i < FACE_DOWN_ALT => self.granted_alt_costs(p, id, zone).get((i - GRANT_ALT_BASE) as usize).map_or(false, |g| g.3),
                _ => zone == Zone::Exile && self.cards[id.0 as usize].play_life_mv,
            };
            if life_mv && !self.can_pay_life(p, self.def_of(id).mana_value as i32) {
                continue;
            }
            // a reduction that depends on the target ("costs {2} less if it targets a blue spell"): is there a target for which it is affordable?
            if !specs.is_empty() && !d.cost.has_x() {
                let none: [Target; 0] = [];
                let worst = self.effective_cost_t(p, id, alt, Some((&none, &[])));
                self.assume_target_tax.set(true);
                let pessimistic = self.effective_cost(p, id, alt);
                self.assume_target_tax.set(false);
                if worst != cost || pessimistic != cost {
                    let spec0 = &specs[0];
                    let mut any = false;
                    for o in self.legal_targets(spec0, id, p) {
                        let t = [Target { obj: o, zcc: 0 }];
                        let c2 = self.effective_cost_t(p, id, alt, Some((&t, &[0])));
                        let a2 = self.cost_after_alt_help(p, id, &c2);
                        self.pay_for.set(Some(id));
                        let ok = self.can_afford(p, &a2, 0);
                        self.pay_for.set(None);
                        if ok {
                            any = true;
                            break;
                        }
                    }
                    if !any {
                        continue;
                    }
                }
            }
            if !self.additional_costs_payable(p, id, d, &variants[0]) {
                continue;
            }
            if alt.is_none() && zone == Zone::Graveyard {
                if let Some(fc) = self.forage_cost(p, id) {
                    if !self.cost_parts_payable(p, id, &[fc]) {
                        continue;
                    }
                }
            }
            if alt.is_none() || zone == Zone::Library || alt == Some(FACE_DOWN_ALT) {
                let xl = self.play_extra_life(p, id, zone);
                if xl > 0 && !self.can_pay_life(p, xl) {
                    continue;
                }
                let xc = self.play_extra_costs(p, id, zone);
                if !xc.is_empty() && !self.cost_parts_payable(p, id, &xc) {
                    continue;
                }
                if self.play_sac_exile(p, id, zone) && !self.cost_parts_payable(p, id, &[Cost::Sacrifice { filter: (Filter { types: vec![CardType::Creature], controller: PlayerFilter::You, ..Default::default() }).into(), n: 1 }]) {
                    continue;
                }
            }
            self.cast_hint.set(Some(zone));
            let has_cond = d.abilities.iter().any(|a| matches!(a, Ability::CastOnlyIf { .. }));
            // the condition may ask how the spell is being cast (CastWithAlt): evaluate it as if cast with this alternative
            let probe;
            let gref: &Game = if has_cond && self.cards[id.0 as usize].cast_alt != alt {
                let mut g2 = self.clone();
                g2.cards[id.0 as usize].cast_alt = alt;
                probe = g2;
                &probe
            } else {
                self
            };
            // "X can't be 0" is a CastOnlyIf on X: probe it with the smallest X it allows, not 0
            let x_floor = Self::cast_x_floor(d);
            let blocked = d.abilities.iter().any(|a| matches!(a, Ability::CastOnlyIf { condition } if !gref.eval_cond(condition, &{ let mut ec = EvalCtx::with_controller(id, p); ec.x = x_floor; ec })));
            self.cast_hint.set(None);
            if blocked {
                continue;
            }
            if let Some(i) = alt {
                if let Some(Ability::AlternativeCost { cost, .. }) = d.abilities.iter().filter(|a| matches!(a, Ability::AlternativeCost { .. })).nth(i as usize) {
                    let non_mana: Vec<Cost> = cost.iter().filter(|c| !matches!(c, Cost::Mana { .. })).cloned().collect();
                    if !self.cost_parts_payable(p, id, &non_mana) {
                        continue;
                    }
                }
            }
            out.push(Action::Cast { card: id, alt });
        }
    }

    /// Permanents / cards that could help pay with convoke (0), improvise (1) or delve (2).
    pub(crate) fn alt_options(&self, p: PlayerId, which: u8, spell: CardId) -> Vec<CardId> {
        match which {
            0 | 1 => self
                .battlefield
                .iter()
                .copied()
                .filter(|id| {
                    let k = &self.cards[id.0 as usize];
                    k.cur.controller == p && !k.tapped && self.can_become_tapped(*id, false) && k.cur.types.has(if which == 0 { CardType::Creature } else { CardType::Artifact }) && *id != spell
                })
                .collect(),
            _ => self.players[p as usize].graveyard.iter().copied().filter(|id| *id != spell).collect(),
        }
    }

    /// Whether the spell has convoke, improvise or delve (paid in the `Stage::Alt` stage).
    pub(crate) fn spell_has_alt_cost(&self, p: PlayerId, spell: CardId) -> bool {
        let kws = self.spell_alt_keywords(p, spell);
        kws.has(Keyword::Convoke) || kws.has(Keyword::Improvise) || kws.has(Keyword::Delve)
    }

    /// Convoke / improvise / delve a spell has: printed, or granted by a static on a permanent `p` controls ("spells you cast have delve").
    pub(crate) fn spell_alt_keywords(&self, p: PlayerId, spell: CardId) -> KeywordSet {
        let mut kws = self.db.def(self.cards[spell.0 as usize].def).keywords;
        // "spells you cast have delve" / "artifact spells you cast have convoke": battlefield statics grant these to the spell
        // (a face-up plane in the command zone grants it to every player's spells: Ood Sphere)
        for src in self.battlefield.iter().copied().chain(self.emblem_srcs()) {
            let sc = &self.cards[src.0 as usize];
            if sc.cur.no_abilities || (sc.zone != Zone::Command && sc.cur.controller != p) || (sc.zone == Zone::Command && !self.db.def(sc.def).types.has(CardType::Plane) && self.src_ctrl(src) != p) {
                continue;
            }
            let sd = self.db.def(sc.def);
            for &i in &sd.statics {
                if let Ability::Static { effect: StaticEffect::GrantKeyword { affects, keyword }, condition, .. } = &sd.abilities[i as usize] {
                    if matches!(keyword, Keyword::Convoke | Keyword::Improvise | Keyword::Delve | Keyword::Assist)
                        && !kws.has(*keyword)
                        && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(src, p)))
                        && self.matches_card(affects, spell, src, p)
                    {
                        kws.add(*keyword);
                    }
                }
            }
        }
        // "the next spell you cast this turn has improvise": granted to the player by a resolved effect
        for e in &self.effects {
            if let Modif::Static(b) = &e.modif {
                if let StaticEffect::GrantSpellKeyword { filter, keyword, .. } = &**b {
                    if e.players.contains(&p) && self.matches_card(&Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() }, spell, e.source, e.controller) {
                        kws.add(*keyword);
                    }
                }
            }
        }
        kws
    }

    /// The mana cost left after the most that convoke (coloured shards by matching creatures, then generic), improvise and delve could pay.
    pub(crate) fn alt_help_parts(&self, p: PlayerId, spell: CardId, cost: &ManaCost) -> (ManaCost, u32) {
        let kws = self.spell_alt_keywords(p, spell);
        let mut c2 = cost.clone();
        let mut generic_help = 0u32;
        if kws.has(Keyword::Convoke) {
            let mut creatures = self.alt_options(p, 0, spell);
            // coloured shards first, each paid by a creature of that colour; the colours the lands can't cover go first, so a
            // multicoloured creature pays the colour that is short (Black Panther's {W} for {G}{W}{W} with a Forest and a Plains)
            {
                let srcs = self.mana_sources_for_payment(p);
                let supply = |c: Color| srcs.iter().filter(|m| m.produces.contains(&ManaType::of(c))).count() as i32;
                let demand = |c2: &ManaCost, c: Color| c2.shards.iter().filter(|s| matches!(s, Shard::Color { c: x } if *x == c)).count() as i32;
                let key = |c2: &ManaCost, s: &Shard| match s {
                    Shard::Color { c } => -(demand(c2, *c) - supply(*c)),
                    _ => 1000,
                };
                let snapshot = c2.clone();
                c2.shards.sort_by_key(|s| key(&snapshot, s));
            }
            let mut i = 0;
            while i < c2.shards.len() {
                let paid_by = |c2: &ManaCost, creatures: &[CardId]| match c2.shards[i] {
                    Shard::Color { c } => creatures.iter().position(|id| self.cards[id.0 as usize].cur.colors.has(c)),
                    Shard::Hybrid { a, b } => creatures.iter().position(|id| self.cards[id.0 as usize].cur.colors.has(a) || self.cards[id.0 as usize].cur.colors.has(b)),
                    _ => None,
                };
                if let Some(pos) = paid_by(&c2, &creatures) {
                    creatures.remove(pos);
                    c2.shards.remove(i);
                    continue;
                }
                i += 1;
            }
            // a creature that taps for mana is already counted as a mana source: it can pay once, not twice
            let src: Vec<CardId> = self.mana_sources_for_payment(p).iter().map(|m| m.card).collect();
            generic_help += creatures.iter().filter(|id| !src.contains(id)).count() as u32;
        }
        if kws.has(Keyword::Improvise) {
            // an artifact that taps for mana is already counted as a mana source: it can pay once, not twice
            let src: Vec<CardId> = self.mana_sources_for_payment(p).iter().map(|m| m.card).collect();
            generic_help += self.alt_options(p, 1, spell).into_iter().filter(|id| !src.contains(id)).count() as u32;
        }
        if kws.has(Keyword::Delve) {
            generic_help += self.alt_options(p, 2, spell).len() as u32;
        }
        if kws.has(Keyword::Assist) {
            // other players can pay generic mana: as much as they have available (at most the generic part)
            // an {X} cost: X can be as large as the helpers can pay
            let generic: u32 = c2.shards.iter().map(|s| if let Shard::Generic { n } = s { *n as u32 } else if matches!(s, Shard::X) { 1000 } else { 0 }).sum();
            let avail: u32 = self.alive_players().into_iter().filter(|q| *q != p).map(|q| self.players[q as usize].pool.total() + self.mana_sources_for_payment(q).len() as u32).sum();
            generic_help += generic.min(avail);
        }
        (c2, generic_help)
    }

    /// A battlefield `GrantReplicate` static of the player's gives this spell replicate (cost equal to its mana cost).
    pub(crate) fn replicate_granted(&self, p: PlayerId, spell: CardId) -> bool {
        self.replicate_cost(p, spell).is_some()
    }

    /// How many extra dice `p`'s rolls get from `RollExtraDieIgnoreLowest` statics on permanents they control.
    pub(crate) fn extra_roll_dice(&self, p: PlayerId) -> usize {
        self.battlefield
            .iter()
            .filter(|&&src| {
                let sc = &self.cards[src.0 as usize];
                if sc.cur.no_abilities || sc.cur.controller != p {
                    return false;
                }
                let sd = self.db.def(sc.def);
                sd.statics.iter().any(|&i| {
                    matches!(&sd.abilities[i as usize], Ability::Static { effect: StaticEffect::RollExtraDieIgnoreLowest, condition, .. }
                        if condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(src, p))))
                })
            })
            .count()
    }

    /// A battlefield `GrantConspire` static of the player's gives this spell conspire.
    pub(crate) fn conspire_granted(&self, p: PlayerId, spell: CardId) -> bool {
        if self.cards[spell.0 as usize].zone == Zone::Battlefield {
            return false;
        }
        self.battlefield.iter().any(|&src| {
            let sc = &self.cards[src.0 as usize];
            if sc.cur.no_abilities || sc.cur.controller != p {
                return false;
            }
            let sd = self.db.def(sc.def);
            sd.statics.iter().any(|&i| {
                matches!(&sd.abilities[i as usize], Ability::Static { effect: StaticEffect::GrantConspire { affects }, condition, .. }
                    if condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(src, p))) && self.matches_card(affects, spell, src, p))
            })
        })
    }

    /// The offspring cost a battlefield `GrantOffspring` static of the player's gives this creature spell, if any.
    pub(crate) fn offspring_cost(&self, p: PlayerId, spell: CardId) -> Option<String> {
        if self.cards[spell.0 as usize].zone == Zone::Battlefield {
            return None;
        }
        for &src in &self.battlefield {
            let sc = &self.cards[src.0 as usize];
            if sc.cur.no_abilities || sc.cur.controller != p {
                continue;
            }
            let sd = self.db.def(sc.def);
            for &i in &sd.statics {
                if let Ability::Static { effect: StaticEffect::GrantOffspring { affects, cost }, condition, .. } = &sd.abilities[i as usize] {
                    if condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(src, p))) && self.matches_card(affects, spell, src, p) {
                        return Some(cost.clone());
                    }
                }
            }
        }
        None
    }

    /// The casualty power a battlefield `GrantCasualty` static of the player's gives this spell, if any.
    pub(crate) fn casualty_granted(&self, p: PlayerId, spell: CardId) -> Option<i32> {
        if self.cards[spell.0 as usize].zone == Zone::Battlefield {
            return None;
        }
        for &src in &self.battlefield {
            let sc = &self.cards[src.0 as usize];
            if sc.cur.no_abilities || sc.cur.controller != p {
                continue;
            }
            let sd = self.db.def(sc.def);
            for &i in &sd.statics {
                if let Ability::Static { effect: StaticEffect::GrantCasualty { affects, n, first_each_turn }, condition, .. } = &sd.abilities[i as usize] {
                    if condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(src, p)))
                        && self.matches_card(affects, spell, src, p)
                        && (!*first_each_turn || !self.spells_log.iter().any(|(pl, c)| *pl == p && *c != spell && self.matches_card(affects, *c, src, p)))
                    {
                        return Some(*n);
                    }
                }
            }
        }
        None
    }

    /// The granted replicate cost for this spell: `Some(None)` = equal to its mana cost, `Some(Some(c))` = the fixed cost `c`.
    pub(crate) fn replicate_cost(&self, p: PlayerId, spell: CardId) -> Option<Option<String>> {
        if self.cards[spell.0 as usize].zone == Zone::Battlefield {
            return None;
        }
        for &src in &self.battlefield {
            let sc = &self.cards[src.0 as usize];
            if sc.cur.no_abilities || sc.cur.controller != p {
                continue;
            }
            let sd = self.db.def(sc.def);
            for &i in &sd.statics {
                if let Ability::Static { effect: StaticEffect::GrantReplicate { affects, cost }, condition, .. } = &sd.abilities[i as usize] {
                    if condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(src, p))) && self.matches_card(affects, spell, src, p) {
                        return Some(cost.clone());
                    }
                }
            }
        }
        None
    }

    pub(crate) fn cost_after_alt_help(&self, p: PlayerId, spell: CardId, cost: &ManaCost) -> ManaCost {
        let (mut c2, generic_help) = self.alt_help_parts(p, spell, cost);
        let mut left = generic_help;
        for sh in c2.shards.iter_mut() {
            if let Shard::Generic { n } = sh {
                let r = (*n as u32).min(left);
                *n -= r as u8;
                left -= r;
            }
        }
        c2.shards.retain(|s| !matches!(s, Shard::Generic { n: 0 }));
        c2
    }

    /// How much generic mana convoke / improvise / delve could contribute to casting `spell`.
    pub(crate) fn alt_reduction(&self, p: PlayerId, spell: CardId) -> u32 {
        let kws = self.spell_alt_keywords(p, spell);
        let mut n = 0;
        for (i, k) in [Keyword::Convoke, Keyword::Improvise, Keyword::Delve].iter().enumerate() {
            if kws.has(*k) {
                let src: Vec<CardId> = if i < 2 { self.mana_sources_for_payment(p).iter().map(|m| m.card).collect() } else { vec![] };
                n += self.alt_options(p, i as u8, spell).into_iter().filter(|id| !src.contains(id)).count() as u32;
            }
        }
        n
    }

    /// Total power of the untapped creatures `p` could tap to crew `vehicle` (summoning sickness doesn't matter).
    pub(crate) fn crew_power(&self, p: PlayerId, vehicle: CardId) -> i32 {
        self.battlefield
            .iter()
            .filter(|id| **id != vehicle && self.cards[id.0 as usize].cur.controller == p && !self.cards[id.0 as usize].tapped && self.can_become_tapped(**id, false) && self.cards[id.0 as usize].cur.types.has(CardType::Creature))
            .map(|id| self.crew_value(*id, vehicle))
            .sum()
    }

    /// What a creature contributes when tapped for Crew / Saddle: its power, plus any "crew as though its power were N greater" statics.
    pub(crate) fn crew_value(&self, id: CardId, vehicle: CardId) -> i32 {
        let vs = &self.cards[vehicle.0 as usize].cur.subtypes;
        let saddling = vs.contains(&Sym::new("Mount")) && !vs.contains(&Sym::new("Vehicle"));
        let mut bonus = 0;
        let mut by_toughness = false;
        for &sid in &self.battlefield {
            let sc = &self.cards[sid.0 as usize];
            if sc.cur.no_abilities {
                continue;
            }
            let sd = self.db.def(sc.def);
            for &i in &sd.statics {
                if let Ability::Static { effect: StaticEffect::CrewPowerBonus { affects, bonus: b, crew_only }, .. } = &sd.abilities[i as usize] {
                    if !(*crew_only && saddling) && self.matches_card(affects, id, sid, sc.cur.controller) {
                        bonus += *b;
                    }
                }
                // "can't crew Vehicles": contributes nothing
                if let Ability::Static { effect: StaticEffect::CrewUsesToughness { affects }, .. } = &sd.abilities[i as usize] {
                    if self.matches_card(affects, id, sid, sc.cur.controller) {
                        by_toughness = true;
                    }
                }
                if let Ability::Static { effect: StaticEffect::CantCrew { affects }, .. } = &sd.abilities[i as usize] {
                    if self.matches_card(affects, id, sid, sc.cur.controller) {
                        return 0;
                    }
                }
            }
        }
        let base = if by_toughness { self.cards[id.0 as usize].cur.toughness } else { self.cards[id.0 as usize].cur.power };
        (base + bonus).max(0) * if self.crew_station_triples(id) { 3 } else { 1 }
    }

    /// Does a "stations / crews as though its power were tripled" static apply to this creature (Sulu, Heroic Helmsman)?
    pub(crate) fn crew_station_triples(&self, id: CardId) -> bool {
        self.battlefield.iter().any(|&sid| {
            let sc = &self.cards[sid.0 as usize];
            if sc.cur.no_abilities {
                return false;
            }
            let sd = self.db.def(sc.def);
            sd.statics.iter().any(|&i| matches!(&sd.abilities[i as usize], Ability::Static { effect: StaticEffect::CrewStationTriple { affects }, .. } if self.matches_card(affects, id, sid, sc.cur.controller)))
        })
    }

    /// Permanents that `parts` force `p` to sacrifice (as many eligible ones as the cost needs): they can't also pay mana.
    pub(crate) fn forced_sac_reserve(&self, p: PlayerId, card: CardId, parts: &[Cost]) -> Vec<CardId> {
        let mut out = vec![];
        for part in parts {
            if let Cost::Sacrifice { filter, n } = part {
                let mut f = self.sac_cost_filter_check(filter);
                f.controller = PlayerFilter::Any;
                let els: Vec<CardId> = self.battlefield.iter().copied().filter(|id| self.cards[id.0 as usize].cur.controller == p && self.matches_card(&f, *id, card, p)).collect();
                if els.len() == *n as usize {
                    // only permanents whose mana ability sacrifices them (Lotus Petal, Treasure, Eldrazi Spawn) lose their mana: a land can tap first, then be sacrificed
                    out.extend(els.into_iter().filter(|id| {
                        self.def_of(*id).abilities.iter().any(|a| matches!(a, Ability::Activated { cost, effects, .. } if cost.iter().any(|c| matches!(c, Cost::SacrificeSelf)) && effects.iter().any(|e| matches!(e, Effect::AddMana { .. }))))
                    }));
                }
            }
        }
        out
    }

    /// Could `p` pay all of these cost parts right now (each checked on its own; mana via the planner)?
    pub(crate) fn cost_parts_payable(&self, p: PlayerId, card: CardId, parts: &[Cost]) -> bool {
        let reserve = self.forced_sac_reserve(p, card, parts);
        let prev = self.sac_reserve.replace(reserve);
        let ok = self.cost_parts_payable_inner(p, card, parts);
        *self.sac_reserve.borrow_mut() = prev;
        ok
    }

    fn cost_parts_payable_inner(&self, p: PlayerId, card: CardId, parts: &[Cost]) -> bool {
        // several discards need that many distinct cards in hand (Foil: an Island and another card), not counting the spell itself
        let discards = parts.iter().filter(|c| matches!(c, Cost::Discard { .. })).count();
        if discards > 1 {
            let total: i32 = parts.iter().map(|c| if let Cost::Discard { n, .. } = c { *n } else { 0 }).sum();
            let own = (self.cards[card.0 as usize].zone == Zone::Hand) as usize;
            if self.players[p as usize].hand.len().saturating_sub(own) < total.max(0) as usize {
                return false;
            }
        }
        let mut mana = ManaCost::default();
        let mut reducers: Vec<&Cost> = vec![];
        for part in parts {
            let ok = match part {
                Cost::SacrificeReduceByManaValue { filter } | Cost::SacrificeReduceByManaCost { filter } => {
                    reducers.push(part);
                    self.count_eligible(filter, card, p, Zone::Battlefield) >= 1
                }
                Cost::TapReduceByPower { filter } => {
                    reducers.push(part);
                    !self.tappable_for_cost(p, card, filter).is_empty()
                }
                Cost::Mana { cost } => {
                    if let Ok(m) = ManaCost::parse(cost) {
                        mana.shards.extend(m.shards);
                    }
                    true
                }
                Cost::PayLife { n } => self.can_pay_life(p, *n),
                Cost::PayLifeX { amount } => self.can_pay_life(p, self.life_cost(card, amount)),
                Cost::CollectEvidence { n } => self.evidence_cards(p, *n, card).is_some(),
                Cost::CollectEvidenceTargets => true,
                Cost::Sacrifice { filter, n } => self.count_eligible(&self.sac_cost_filter_check(filter), card, p, Zone::Battlefield) >= *n as usize,
                Cost::Discard { filter, n, .. } | Cost::ExileFromHand { filter, n, .. } => {
                    let filter = &crate::pay::strip_x_filter(filter);
                    let own = self.cards[card.0 as usize].zone == Zone::Hand && self.matches_card(filter, card, card, p);
                    self.count_eligible(filter, card, p, Zone::Hand).saturating_sub(own as usize) >= *n as usize
                }
                Cost::PutExiledIntoGraveyard { filter } => self.count_eligible(filter, card, p, Zone::Exile) >= 1,
                Cost::ExileFromSingleGraveyard { filter, n, .. } => !self.single_graveyard_pools(filter, card, p, *n).is_empty(),
                Cost::ExileGraveyardTypes { n } => self.gy_types_ok(&self.gy_types_pool(p, card), *n),
                Cost::ExileGraveyardMv { filter, n } => self.gy_mv_ok(&self.gy_mv_pool(filter, p, card), *n),
                Cost::ExileTotalMvX { filter } => !self.exile_mv_pool(filter, p, card).is_empty(),
                Cost::ExileCraftMaterials { bf, gy, min, share_type } => self.craft_ok(bf, gy, *min, *share_type, p, card),
                Cost::ExileFromGraveyardAmount { filter, amount } => self.count_eligible(filter, card, p, Zone::Graveyard) >= self.life_cost(card, amount) as usize,
                Cost::ExileFromGraveyardPayCost { filter } => match self.cheapest_graveyard_cost(p, filter, card, &mana) {
                    Some(shards) => {
                        mana.shards.extend(shards);
                        true
                    }
                    None => false,
                },
                Cost::ExileFromGraveyard { filter, n, bottom } => {
                    // escape: the spell itself can't pay for its own "exile other cards" cost
                    let own = self.cards[card.0 as usize].zone == Zone::Graveyard && self.matches_card(filter, card, card, p);
                    self.count_eligible(filter, card, p, Zone::Graveyard).saturating_sub(own as usize) >= *n as usize
                }
                Cost::RemoveCounters { kind, n } => self.cards[card.0 as usize].counter(*kind) >= *n,
                Cost::RemoveAnyCounters { n } => self.cards[card.0 as usize].counters.iter().map(|(_, x)| *x as i32).sum::<i32>() >= *n,
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
                Cost::Behold { filter, n, choose_type } => self.beholdable(filter, card, p, *n, *choose_type),
                Cost::ChooseCreatureOrWarped => self.greatest_creature_or_warped_power(p).is_some(),
                Cost::UnattachSelf => self.cards[card.0 as usize].attached_to.is_some(),
                Cost::UnattachAttached { filter } => self.attached_matching(card, filter, p).is_some(),
                Cost::PayEnergy { n } => self.players[p as usize].energy >= *n,
                Cost::SacrificeSelf | Cost::OpponentGainsLife { .. } | Cost::PlayerGainsLife { .. } => self.simple_cost_ok(p, card, part),
                Cost::Tap => !self.cards[card.0 as usize].tapped && self.can_use_tap_symbol(card) && self.can_become_tapped(card, false),
                Cost::Crew { power } => self.crew_power(p, card) >= self.crew_requirement(card, *power),
                Cost::TapPermanents { filter, n } => self.tappable_for_cost(p, card, filter).len() >= *n as usize,
                Cost::UntapPermanents { filter, n, opponent } => self.untappable_for_cost(p, card, filter, *opponent).len() >= *n as usize,
                Cost::Waterbend { n } => {
                    let cnt = self.waterbend_sources(p).len() as i32;
                    mana.add_generic((*n - cnt).max(0) as u8);
                    true
                }
                Cost::TapCreaturesOrMana { n } => {
                    let cnt = self.creature_bend_sources(p).len() as i32;
                    mana.add_generic((*n - cnt).max(0) as u8);
                    true
                }
                Cost::Or { options } => options.iter().any(|o| self.cost_parts_payable(p, card, o)),
                Cost::ReturnToHand { filter, n } | Cost::ExileFromBattlefield { filter, n } => self.count_eligible(filter, card, p, Zone::Battlefield) >= *n as usize,
                Cost::RemoveCountersFrom { filter, kind, n, among, suspended } => self.counters_removable_from(filter, *kind, *n, *among, *suspended, card, p),
                Cost::RemoveAnyCounters { n } => self.cards[card.0 as usize].counters.iter().map(|(_, x)| *x as i32).sum::<i32>() >= *n,
                Cost::RemoveAnyCountersFrom { filter, n } => self.any_counters_removable(filter, *n, card, p),
                Cost::PutCountersOnChosen { filter, opponent, .. } => if *opponent { !self.counter_targets(p, filter, card, true).is_empty() } else { self.count_eligible(filter, card, p, Zone::Battlefield) >= 1 },
                Cost::TapTotalPower { filter, power } => self.tappable_for_cost(p, card, filter).iter().map(|id| self.cards[id.0 as usize].cur.power.max(0)).sum::<i32>() >= *power,
                Cost::SacrificeTotalPower { filter, power } => self.eligible_permanents(p, filter, card).iter().map(|id| self.cards[id.0 as usize].cur.power.max(0)).sum::<i32>() >= *power,
                Cost::TapShareType { filter, n } => self.share_type_group(&self.tappable_for_cost(p, card, filter), *n).is_some(),
                Cost::SacrificeDistinctNames { filter, n } => self.distinct_names_ok(&self.sac_pool(filter, card, p), *n),
                Cost::DiscardSameName { filter, n } => self.same_name_group(&self.hand_matching(p, card, filter), *n).is_some(),
                Cost::DiscardDistinctNames { filter, n } => self.distinct_names_ok(&self.hand_matching(p, card, filter), *n),
                Cost::RevealShareColor { n } => self.share_color_group(&self.hand_matching(p, card, &Filter::default()), *n),
                Cost::Reveal { filter, n } => self.players[p as usize].hand.iter().filter(|h| (**h != card || filter.this) && self.matches_card(&Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() }, **h, card, p)).count() >= *n as usize,
                _ => true,
            };
            if !ok {
                return false;
            }
        }
        if let Some(red) = reducers.first() {
            // the best permanent to give up decides: try each and see whether some choice makes the rest payable
            let (filter, kind) = match red {
                Cost::SacrificeReduceByManaValue { filter } => (filter, 0),
                Cost::SacrificeReduceByManaCost { filter } => (filter, 1),
                Cost::TapReduceByPower { filter } => (filter, 2),
                _ => unreachable!(),
            };
            let cands: Vec<CardId> = if kind == 2 { self.tappable_for_cost(p, card, filter) } else { self.eligible_battlefield(filter, card, p) };
            return cands.into_iter().any(|id| {
                let mut m = mana.clone();
                let mut g: u32 = match kind {
                    0 => self.def_of(id).cost.mana_value(0) as u32,
                    2 => self.cards[id.0 as usize].cur.power.max(0) as u32,
                    _ => self.def_of(id).cost.shards.iter().map(|s| if let Shard::Generic { n } = s { *n as u32 } else { 0 }).sum(),
                };
                if kind == 1 {
                    for s in &self.def_of(id).cost.shards {
                        if !matches!(s, Shard::Generic { .. }) {
                            if let Some(pos) = m.shards.iter().position(|x| x == s) {
                                m.shards.remove(pos);
                            }
                        }
                    }
                }
                for sh in m.shards.iter_mut() {
                    if let Shard::Generic { n } = sh {
                        let r = (*n as u32).min(g);
                        *n -= r as u8;
                        g -= r;
                    }
                }
                m.shards.retain(|s| !matches!(s, Shard::Generic { n: 0 }));
                m.is_empty() || self.can_afford(p, &m, 0)
            });
        }
        self.anyfy_ability_mana(p, card, &mut mana);
        mana.is_empty() || self.can_afford(p, &mana, 0)
    }

    /// The forage additional cost (exile three other graveyard cards or sacrifice a Food) `p` must pay to cast `card` from a graveyard through `Effect::MayCastForaging`.
    /// Is this spell on the stack being cast for its bestow cost (an Aura spell, not a creature spell)?
    pub(crate) fn cast_for_bestow(&self, card: CardId) -> bool {
        let c = &self.cards[card.0 as usize];
        c.zone == Zone::Stack
            && c.cast_alt.map_or(false, |i| {
                self.def_of(card).abilities.iter().filter(|a| matches!(a, Ability::AlternativeCost { .. })).nth(i as usize).map_or(false, |a| matches!(a, Ability::AlternativeCost { effects: Some(e), .. } if matches!(e.as_slice(), [Effect::Nothing])))
            })
    }

    pub(crate) fn forage_cost(&self, p: PlayerId, card: CardId) -> Option<Cost> {
        if self.forage_grants.is_empty() {
            return None;
        }
        if !self.forage_grants.iter().any(|(pl, t, f, src)| *pl == p && *t == self.turn && self.matches_card(&Filter { controller: PlayerFilter::Any, ..f.clone() }, card, *src, p)) {
            return None;
        }
        Some(Cost::Or {
            options: vec![
                vec![Cost::ExileFromGraveyard { filter: (Filter { other: true, ..Default::default() }).into(), n: 3, bottom: false }],
                vec![Cost::Sacrifice { filter: (Filter { subtypes: vec!["Food".into()], ..Default::default() }).into(), n: 1 }],
            ],
        })
    }

    /// `Ability::RemoveCountersReduceCost` on `card`: (counter kind, generic mana saved per counter).
    fn frenzy_params(&self, card: CardId) -> Option<(CounterKind, i32)> {
        self.def_of(card).abilities.iter().find_map(|a| if let Ability::RemoveCountersReduceCost { kind, per } = a { Some((*kind, (*per).max(1))) } else { None })
    }

    /// The most counters worth removing for `RemoveCountersReduceCost` against `mana`: limited by the counters on `p`'s creatures and by the generic cost left.
    pub(crate) fn frenzy_max(&self, p: PlayerId, card: CardId, mana: &ManaCost) -> i32 {
        let Some((kind, per)) = self.frenzy_params(card) else { return 0 };
        let total: i32 = self.battlefield.iter().filter(|&&id| { let c = &self.cards[id.0 as usize]; c.cur.controller == p && c.cur.types.has(CardType::Creature) }).map(|&id| self.cards[id.0 as usize].counter(kind)).sum();
        let generic: i32 = mana.shards.iter().map(|s| if let Shard::Generic { n } = s { *n as i32 } else { 0 }).sum();
        total.min((generic + per - 1) / per)
    }

    /// A `StaticEffect::SpellManaForCounters` on the battlefield lets `p` pay any amount of extra mana for +1/+1 counters on `card`.
    pub(crate) fn spell_mana_for_counters(&self, p: PlayerId, card: CardId) -> bool {
        for &src in &self.battlefield {
            let sc = &self.cards[src.0 as usize];
            if sc.cur.no_abilities {
                continue;
            }
            let sd = self.db.def(sc.def);
            let ctrl = sc.cur.controller;
            for &i in &sd.statics {
                if let Ability::Static { effect: StaticEffect::SpellManaForCounters { filter, whose }, condition, .. } = &sd.abilities[i as usize] {
                    if condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(src, ctrl)))
                        && self.player_filter_src(src, *whose, p, ctrl)
                        && self.matches_card(filter, card, src, ctrl)
                    {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Optional additional costs offered to `p` for casting `card` by `StaticEffect::OptionalSpellCost` statics on the battlefield: (cost, coloured shard it removes).
    pub(crate) fn optional_spell_costs(&self, p: PlayerId, card: CardId) -> Vec<(Vec<Cost>, Color)> {
        let mut out = vec![];
        for &src in &self.battlefield {
            let sc = &self.cards[src.0 as usize];
            if sc.cur.no_abilities {
                continue;
            }
            let sd = self.db.def(sc.def);
            let ctrl = sc.cur.controller;
            for &i in &sd.statics {
                if let Ability::Static { effect: StaticEffect::OptionalSpellCost { filter, whose, cost, reduce }, condition, .. } = &sd.abilities[i as usize] {
                    if condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(src, ctrl)))
                        && self.player_filter_src(src, *whose, p, ctrl)
                        && self.matches_card(filter, card, src, ctrl)
                    {
                        out.push((cost.clone(), *reduce));
                    }
                }
            }
        }
        out
    }

    /// Mana shards of the first graveyard card matching `filter` (cheapest first) whose mana cost is payable on top of `base`; if none is, the cheapest one; None when no card matches.
    fn cheapest_graveyard_cost(&self, p: PlayerId, filter: &Filter, card: CardId, base: &ManaCost) -> Option<Vec<Shard>> {
        let mut f = filter.clone();
        f.controller = PlayerFilter::Any;
        let mut els: Vec<CardId> = self.players[p as usize].graveyard.iter().copied().filter(|id| self.matches_card(&f, *id, card, p)).collect();
        els.sort_by_key(|id| self.def_of(*id).mana_value);
        let shards_of = |id: CardId| -> Vec<Shard> { self.def_of(id).cost.shards.iter().filter(|s| !matches!(s, Shard::X)).cloned().collect() };
        for &id in &els {
            let mut total = base.clone();
            total.shards.extend(shards_of(id));
            if self.can_afford(p, &total, 0) {
                return Some(shards_of(id));
            }
        }
        els.first().map(|id| shards_of(*id))
    }

    /// Life paid as the commander tax when `card` (with `Ability::CommanderTaxInLife`) is cast from the command zone: 2 for each previous cast.
    fn commander_life_tax(&self, p: PlayerId, card: CardId) -> i32 {
        let c = &self.cards[card.0 as usize];
        if c.zone != Zone::Command || !c.is_commander || !self.db.def(c.def).abilities.iter().any(|a| matches!(a, Ability::CommanderTaxInLife)) {
            return 0;
        }
        self.players[p as usize].commander_cast.iter().find(|(id, _)| *id == card).map_or(0, |(_, n)| *n as i32 * 2)
    }

    /// Can the non-mana additional costs of casting `card` (sacrifice, discard, exile ...) be paid?
    /// Coloured reductions of the spell's cost that found nothing to remove also lower a chosen additional mana cost ("costs {W}{B} less ... reduces only
    /// the coloured mana you pay": Edgewalker with Soaring Stoneglider's {1}{W}).
    pub(crate) fn reduce_extra_mana(&self, p: PlayerId, card: CardId, mc: &mut ManaCost) {
        let _ = self.effective_cost(p, card, self.cards[card.0 as usize].cast_alt);
        let left: Vec<(Color, i32)> = self.color_left.borrow().clone();
        for (col, mut n) in left {
            mc.shards.retain(|s| {
                if n > 0 && matches!(s, Shard::Color { c } if *c == col) {
                    n -= 1;
                    false
                } else {
                    true
                }
            });
        }
    }

    /// A permanent sacrificed to a mana ability while a spell with emerge is being paid for (Foul Emissary with Ashnod's Altar).
    pub(crate) fn sacrificed_while_casting_emerge(&self) -> bool {
        self.pay_for.get().map_or(false, |c| self.cards[c.0 as usize].zone == Zone::Stack && self.def_of(c).abilities.iter().any(|a| matches!(a, Ability::Emerge { .. })))
    }

    fn additional_costs_payable(&self, p: PlayerId, card: CardId, d: &crate::carddb::Def, base: &ManaCost) -> bool {
        let lt = self.commander_life_tax(p, card);
        if lt > 0 && !self.can_pay_life(p, lt) {
            return false;
        }
        let sx = self.spell_extra_costs(p, card, d);
        let mut lists: Vec<&Vec<Cost>> = d.abilities.iter().filter_map(|a| if let Ability::AdditionalCost { cost } = a { Some(cost) } else { None }).collect();
        if !sx.is_empty() {
            lists.push(&sx);
        }
        for cost in lists {
            for part in cost {
                let ok = match part {
                    Cost::Sacrifice { filter, n } => self.count_eligible(&self.sac_cost_filter_check(filter), card, p, Zone::Battlefield) >= *n as usize,
                    Cost::SacrificeAmount { filter, amount } => self.count_eligible(&self.sac_cost_filter(filter), card, p, Zone::Battlefield) >= self.life_cost(card, amount) as usize,
                    Cost::Discard { filter, n, .. } | Cost::ExileFromHand { filter, n, .. } => {
                        let filter = &crate::pay::strip_x_filter(filter);
                        let own = self.cards[card.0 as usize].zone == Zone::Hand && self.matches_card(filter, card, card, p);
                        self.count_eligible(filter, card, p, Zone::Hand).saturating_sub(own as usize) >= *n as usize
                    }
                    Cost::PutExiledIntoGraveyard { filter } => self.count_eligible(filter, card, p, Zone::Exile) >= 1,
                Cost::ExileFromSingleGraveyard { filter, n, .. } => !self.single_graveyard_pools(filter, card, p, *n).is_empty(),
                    Cost::ExileFromGraveyard { filter, n, bottom } => {
                        // escape: the spell itself can't pay for its own "exile other cards" cost
                        let own = self.cards[card.0 as usize].zone == Zone::Graveyard && self.matches_card(filter, card, card, p);
                        self.count_eligible(filter, card, p, Zone::Graveyard).saturating_sub(own as usize) >= *n as usize
                    }
                    Cost::ExileFromGraveyardAmount { filter, amount } => self.count_eligible(filter, card, p, Zone::Graveyard) >= self.life_cost(card, amount) as usize,
                    Cost::PayLife { n } => self.can_pay_life(p, *n),
                    Cost::Behold { filter, n, choose_type } => self.beholdable(filter, card, p, *n, *choose_type),
                    Cost::ChooseCreatureOrWarped => self.greatest_creature_or_warped_power(p).is_some(),
                    Cost::PayLifeX { amount } => self.can_pay_life(p, self.life_cost(card, amount)),
                    Cost::CollectEvidence { n } => self.evidence_cards(p, *n, card).is_some(),
                    Cost::CollectEvidenceTargets => true,
                    Cost::Or { options } => options.iter().any(|o| {
                        // a mana option is paid on top of the spell's own cost
                        let mut total = base.clone();
                        for c in o {
                            if let Cost::Mana { cost } = c {
                                if let Ok(mut mc) = ManaCost::parse(cost) {
                                    self.reduce_extra_mana(p, card, &mut mc);
                                    total.shards.extend(mc.shards);
                                }
                            }
                        }
                        self.cost_parts_payable(p, card, o) && (total.shards.len() == base.shards.len() || self.can_afford(p, &total, 0))
                    }),
                    Cost::ReturnToHand { filter, n } | Cost::ExileFromBattlefield { filter, n } => self.count_eligible(filter, card, p, Zone::Battlefield) >= *n as usize,
                Cost::RemoveCountersFrom { filter, kind, n, among, suspended } => self.counters_removable_from(filter, *kind, *n, *among, *suspended, card, p),
                Cost::RemoveAnyCounters { n } => self.cards[card.0 as usize].counters.iter().map(|(_, x)| *x as i32).sum::<i32>() >= *n,
                Cost::RemoveAnyCountersFrom { filter, n } => self.any_counters_removable(filter, *n, card, p),
                Cost::PutCountersOnChosen { filter, opponent, .. } => if *opponent { !self.counter_targets(p, filter, card, true).is_empty() } else { self.count_eligible(filter, card, p, Zone::Battlefield) >= 1 },
                Cost::TapTotalPower { filter, power } => self.tappable_for_cost(p, card, filter).iter().map(|id| self.cards[id.0 as usize].cur.power.max(0)).sum::<i32>() >= *power,
                Cost::SacrificeTotalPower { filter, power } => self.eligible_permanents(p, filter, card).iter().map(|id| self.cards[id.0 as usize].cur.power.max(0)).sum::<i32>() >= *power,
                Cost::TapShareType { filter, n } => self.share_type_group(&self.tappable_for_cost(p, card, filter), *n).is_some(),
                Cost::SacrificeDistinctNames { filter, n } => self.distinct_names_ok(&self.sac_pool(filter, card, p), *n),
                Cost::DiscardSameName { filter, n } => self.same_name_group(&self.hand_matching(p, card, filter), *n).is_some(),
                Cost::DiscardDistinctNames { filter, n } => self.distinct_names_ok(&self.hand_matching(p, card, filter), *n),
                Cost::RevealShareColor { n } => self.share_color_group(&self.hand_matching(p, card, &Filter::default()), *n),
                Cost::Reveal { filter, n } => self.players[p as usize].hand.iter().filter(|h| (**h != card || filter.this) && self.matches_card(&Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() }, **h, card, p)).count() >= *n as usize,
                    Cost::TapPermanents { filter, n } => self.tappable_for_cost(p, card, filter).len() >= *n as usize,
                    Cost::UntapPermanents { filter, n, opponent } => self.untappable_for_cost(p, card, filter, *opponent).len() >= *n as usize,
                    _ => true,
                };
                if !ok {
                    return false;
                }
            }
        }
        true
    }

    /// An ability granted to `card` by another permanent.
    pub fn granted_ab(&self, card: CardId, idx: u16) -> Option<std::sync::Arc<Ability>> {
        idx.checked_sub(GRANTED_BASE).and_then(|i| self.cards[card.0 as usize].cur.granted.get(i as usize).cloned())
    }

    /// The ability at `idx` of `card`: printed, granted, or an intrinsic land mana ability (owned copy; not for hot paths).
    pub fn ability_owned(&self, card: CardId, idx: u16) -> Option<Ability> {
        if idx >= INTRINSIC_BASE {
            let col = *Color::ALL.get((idx - INTRINSIC_BASE) as usize)?;
            return Some(Ability::Activated {
                cost: vec![Cost::Tap],
                targets: vec![],
                effects: vec![Effect::AddMana { mana: vec![ManaProduce::Fixed { m: ManaType::of(col), n: 1 }], until_end_of_combat: false, until_end_of_turn: false, who: None, player_choice: false }],
                timing: Timing::Instant,
                loyalty: None,
                limit_per_turn: None,
                once: false,
                condition: None,
                from: None,
                power_up: false, special_action: false,
            });
        }
        if idx >= GRANTED_BASE {
            return self.granted_ab(card, idx).map(|a| (*a).clone());
        }
        self.def_of(card).abilities.get(idx as usize).cloned()
    }

    /// Can `p` activate this (non-mana) ability right now?
    pub fn can_activate(&self, p: PlayerId, card: CardId, ability: u16) -> bool {
        let granted = self.granted_ab(card, ability);
        let d = self.db.def(self.cards[card.0 as usize].def);
        match granted.as_deref().or_else(|| d.abilities.get(ability as usize)) {
            Some(ab) => self.can_activate_with(p, card, ability, ab),
            None => false,
        }
    }

    /// How many mana symbols of `col` the cost has (hybrid and Phyrexian symbols of that colour count).
    fn count_symbols(mc: &ManaCost, col: Color) -> i32 {
        mc.shards
            .iter()
            .filter(|s| match s {
                Shard::Color { c } | Shard::Phyrexian { c } | Shard::HybridGeneric { c } | Shard::HybridColorless { c } => *c == col,
                Shard::Hybrid { a, b } | Shard::PhyrexianHybrid { a, b } => *a == col || *b == col,
                _ => false,
            })
            .count() as i32
    }

    /// The `cost` parts for `k` symbols: Sacrifice parts are repeated k times (nothing when k is 0).
    fn scale_per_symbol(cost: &[Cost], k: i32) -> Vec<Cost> {
        if k <= 0 {
            return vec![];
        }
        cost.iter()
            .map(|c| match c {
                Cost::Sacrifice { filter, n } => Cost::Sacrifice { filter: filter.clone(), n: n * k },
                o => o.clone(),
            })
            .collect()
    }

    /// Additional costs that `StaticEffect::SpellExtraCost` statics put on casting `card` (cast by `p`).
    pub(crate) fn spell_extra_costs(&self, p: PlayerId, card: CardId, d: &crate::carddb::Def) -> Vec<Cost> {
        let mut out = vec![];
        for &sid in &self.battlefield {
            let sc = &self.cards[sid.0 as usize];
            if sc.cur.no_abilities {
                continue;
            }
            let sd = self.db.def(sc.def);
            let ctrl = sc.cur.controller;
            for &i in &sd.statics {
                if let Ability::Static { effect: StaticEffect::SpellExtraCost { filter, whose, cost, per_symbol }, condition, .. } = &sd.abilities[i as usize] {
                    if condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(sid, ctrl)))
                        && self.player_filter_src(sid, *whose, p, ctrl)
                        && self.matches_card(&Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() }, card, sid, ctrl)
                    {
                        match per_symbol {
                            None => out.extend(cost.iter().cloned()),
                            Some(col) => out.extend(Self::scale_per_symbol(cost, Self::count_symbols(&d.cost, *col))),
                        }
                    }
                }
            }
        }
        // an emblem granting an additional cost to cards (Wrenn and Six: retrace's land discard)
        for sid in self.emblem_srcs() {
            let sc = &self.cards[sid.0 as usize];
            let sd = self.db.def(sc.def);
            for &i in &sd.statics {
                if let Ability::Static { effect: StaticEffect::GrantAbility { affects, ability }, .. } = &sd.abilities[i as usize] {
                    if let Ability::AdditionalCost { cost } = &**ability {
                        if self.matches_card(affects, card, sid, sc.owner) {
                            out.extend(cost.iter().cloned());
                        }
                    }
                }
            }
        }
        out
    }

    /// Additional costs that statics (`StaticEffect::AbilityExtraCost`) put on activating an ability of `card`.
    fn static_extra_costs(&self, card: CardId, ability_cost: &[Cost]) -> Vec<Cost> {
        let mut out = vec![];
        for &sid in &self.battlefield {
            let sc = &self.cards[sid.0 as usize];
            if sc.cur.no_abilities {
                continue;
            }
            let sd = self.db.def(sc.def);
            for &i in &sd.statics {
                if let Ability::Static { effect: StaticEffect::AbilityExtraCost { filter, cost, per_symbol }, condition, .. } = &sd.abilities[i as usize] {
                    if condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(sid, sc.cur.controller))) && self.matches_card(filter, card, sid, sc.cur.controller) {
                        match per_symbol {
                            None => out.extend(cost.iter().cloned()),
                            Some(col) => {
                                let mut mc = ManaCost::default();
                                for part in ability_cost {
                                    if let Cost::Mana { cost } = part {
                                        if let Ok(m) = ManaCost::parse(cost) {
                                            mc.shards.extend(m.shards);
                                        }
                                    }
                                }
                                out.extend(Self::scale_per_symbol(cost, Self::count_symbols(&mc, *col)));
                            }
                        }
                    }
                }
            }
        }
        out
    }

    /// `Ability::ActivationManaOnlyChosenColor`: the generic mana of this card's activated abilities must be mana of its chosen color.
    fn restrict_activation_mana(&self, card: CardId, mana: &mut ManaCost) {
        if self.db.def(self.cards[card.0 as usize].def).abilities.iter().any(|a| matches!(a, Ability::SpendAnyTypeForOwnAbilities)) && !self.cards[card.0 as usize].cur.no_abilities {
            for s in mana.shards.iter_mut() {
                if matches!(s, Shard::Color { .. } | Shard::Colorless | Shard::Hybrid { .. }) {
                    *s = Shard::Generic { n: 1 };
                }
            }
        }
        if !self.db.def(self.cards[card.0 as usize].def).abilities.iter().any(|a| matches!(a, Ability::ActivationManaOnlyChosenColor)) {
            return;
        }
        let Some(c) = self.cards[card.0 as usize].chosen_color else { return };
        let old = std::mem::take(&mut mana.shards);
        for s in old {
            match s {
                Shard::Generic { n } => (0..n).for_each(|_| mana.shards.push(Shard::Color { c })),
                o => mana.shards.push(o),
            }
        }
    }

    /// A `StaticEffect::CantTurnFaceUp` stops this face-down permanent from being turned face up.
    pub(crate) fn cant_turn_face_up(&self, card: CardId) -> bool {
        if !self.cards[card.0 as usize].face_down {
            return false;
        }
        for &sid in &self.battlefield {
            let sc = &self.cards[sid.0 as usize];
            if sc.cur.no_abilities {
                continue;
            }
            let sd = self.db.def(sc.def);
            for &i in &sd.statics {
                if let Ability::Static { effect: StaticEffect::CantTurnFaceUp { affects }, condition, .. } = &sd.abilities[i as usize] {
                    if condition.as_ref().map_or(false, |cd| !self.eval_cond(cd, &EvalCtx::with_controller(sid, sc.cur.controller))) {
                        continue;
                    }
                    if self.matches_card(affects, card, sid, sc.cur.controller) {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn can_activate_with(&self, p: PlayerId, card: CardId, ability: u16, ab: &Ability) -> bool {
        let c = &self.cards[card.0 as usize];
        if c.detained_by.is_some() {
            return false;
        }
        if matches!(ab, Ability::Activated { effects, .. } if matches!(effects.as_slice(), [Effect::TurnFaceUp { what: None, .. }])) && self.cant_turn_face_up(card) {
            return false;
        }
        if self.db.def(c.def).enchanted_ctl_ability == Some(ability) && c.attached_to.map_or(true, |a| self.cards[a.0 as usize].cur.controller != p) {
            return false;
        }
        {
            for &sid in &self.battlefield {
                let sc = &self.cards[sid.0 as usize];
                let sd = self.db.def(sc.def);
                for &i in &sd.statics {
                    if let Ability::Static { effect: StaticEffect::CantActivate { affects, tap_only, power_up_only, .. }, condition, .. } = &sd.abilities[i as usize] {
                        if *tap_only && !matches!(ab, Ability::Activated { cost, .. } if cost.iter().any(|c| matches!(c, Cost::Tap))) {
                            continue;
                        }
                        if *power_up_only && !matches!(ab, Ability::Activated { power_up: true, .. }) {
                            continue;
                        }
                        if condition.as_ref().map_or(false, |cd| !self.eval_cond(cd, &EvalCtx::with_controller(sid, sc.cur.controller))) {
                            continue;
                        }
                        if self.matches_card(affects, card, sid, sc.cur.controller) {
                            return false;
                        }
                        // landcycling / typecycling is cycling too (Stabilizer)
                        if affects.keywords.contains(&Keyword::Cycling)
                            && matches!(ab, Ability::Activated { from: Some(Zone::Hand), cost, .. } if cost.iter().any(|c| matches!(c, Cost::DiscardSelf)))
                            && self.def_of(card).oracle_text.to_lowercase().contains("cycling")
                            && self.matches_card(&Filter { keywords: vec![], ..(**affects).clone() }, card, sid, sc.cur.controller)
                        {
                            return false;
                        }
                    }
                }
            }
        }
        // "its activated abilities can't be activated this turn" granted by a resolved effect (a `dynamic` GrantStatic
        // matches by its filter, also permanents that arrive later: Revel in Silence)
        for e in &self.effects {
            if let Modif::Static(b) | Modif::DynStatic(b) = &e.modif {
                if let StaticEffect::CantActivate { affects, tap_only, power_up_only, .. } = &**b {
                    if *tap_only && !matches!(ab, Ability::Activated { cost, .. } if cost.iter().any(|c| matches!(c, Cost::Tap))) {
                        continue;
                    }
                    if *power_up_only && !matches!(ab, Ability::Activated { power_up: true, .. }) {
                        continue;
                    }
                    let named = !e.affects.is_empty();
                    if (named && e.affects.iter().any(|(id, _)| *id == card)) || (!named && self.matches_card(affects, card, e.source, e.controller)) {
                        return false;
                    }
                }
            }
        }
        let Ability::Activated { cost, targets, timing, loyalty, limit_per_turn, from, condition, once, power_up, .. } = ab else {
            return false;
        };
        let cost_with_extra: Vec<Cost>;
        let cost = {
            let ex = self.static_extra_costs(card, cost);
            if ex.is_empty() {
                cost
            } else {
                cost_with_extra = cost.iter().cloned().chain(ex).collect();
                &cost_with_extra
            }
        };
        if from.unwrap_or(Zone::Battlefield) != c.zone {
            return false;
        }
        if !self.exile_slots_joint_ok(p, card, cost) {
            return false;
        }
        if let Some(cond) = condition {
            let mut ec = EvalCtx::of(self, card);
            if self.db.def(c.def).any_player {
                ec.controller = p;
            }
            if !self.eval_cond(cond, &ec) {
                return false;
            }
        }
        if c.cur.types.has(CardType::Creature) && !self.db.def(c.def).types.has(CardType::Creature) && c.cur.subtypes.contains(&Sym::new("Equipment")) && matches!(ab, Ability::Activated { effects, .. } if matches!(effects.first(), Some(Effect::Attach { .. }))) {
            // an Equipment that is a creature can't equip (rule 301.5c)
            return false;
        }
        if *timing == Timing::Sorcery && !self.sorcery_timing(p) {
            let is_equip = !self.db.def(c.def).types.has(CardType::Creature) && c.cur.subtypes.contains(&Sym::new("Equipment")) && matches!(ab, Ability::Activated { effects, .. } if matches!(effects.first(), Some(Effect::Attach { .. })));
            if !(is_equip && self.equip_at_instant_speed(p)) {
                return false;
            }
        }
        if let Some(l) = loyalty {
            if !(self.sorcery_timing(p) || self.loyalty_instant(p, card)) || c.activations.iter().filter(|(i, _)| *i == u16::MAX).count() >= self.loyalty_limit(p).max(if c.loyalty_double_turn == self.turn || self.players[p as usize].loyalty_all_double_turn == self.turn { 2 } else { 1 }).max(if c.loyalty_extra_turn == self.turn { 1 + c.loyalty_extra as usize } else { 1 }) {
                return false;
            }
            if *l < 0 && c.counter(CounterKind::Loyalty) < -*l {
                return false;
            }
        }
        if *once && c.exhausted.contains(&ability) && !self.exhaust_reactivate(p) && !(*power_up && c.exhausted.iter().filter(|a| **a == ability).count() < 2 && self.power_up_twice(p)) {
            return false;
        }
        if let Some(max) = limit_per_turn {
            let max = *max + u8::from(Self::is_boast_ability(ab) && self.boast_twice(card));
            if c.activations.iter().find(|(i, _)| *i == ability).map_or(0, |(_, n)| *n) >= max {
                return false;
            }
        }
        let has_x_cost = cost.iter().any(|part| matches!(part, Cost::Mana { cost } if cost.contains("{X}")) || matches!(part, Cost::RemoveLoyaltyX | Cost::RemoveCountersX { .. } | Cost::RemoveXCountersFromOne { .. } | Cost::ExileTotalMvX { .. } | Cost::XTimes { .. }));
        if has_x_cost && !targets.is_empty() {
            // the targets may depend on X (The Mycosynth Gardens): is there some X that works?
            let top = if cost.iter().any(|part| matches!(part, Cost::RemoveLoyaltyX)) { c.counter(CounterKind::Loyalty).clamp(0, 12) } else if let Some(k) = cost.iter().find_map(|part| if let Cost::RemoveCountersX { kind } = part { Some(*kind) } else { None }) { c.counter(k).clamp(0, 12) } else if let Some(xf) = cost.iter().find_map(|part| if let Cost::ExileTotalMvX { filter } = part { Some(filter) } else { None }) { self.exile_mv_pool(xf, p, card).iter().map(|id| self.def_of(*id).mana_value as i32).sum::<i32>().clamp(0, 12) } else if let Some(n) = cost.iter().find_map(|part| if let Cost::XTimes { cost: inner } = part { if let Cost::PayEnergy { n } = &**inner { Some(*n) } else { None } } else { None }) { (self.players[p as usize].energy / n.max(1)).clamp(0, 12) } else if let Some(mc) = cost.iter().find_map(|part| if let Cost::Mana { cost } = part { if cost.contains("{X}") { ManaCost::parse(cost).ok() } else { None } } else { None }) { self.max_x(p, &mc, card, 0).clamp(0, 12) } else { 12 };
            if !(0..=top).any(|x| {
                let mut g2 = self.clone();
                g2.cards[card.0 as usize].x = x;
                g2.specs_satisfiable(targets, card, p)
            }) {
                return false;
            }
        } else if !self.specs_satisfiable(targets, card, p) {
            return false;
        }
        // several separate sacrifice costs need different permanents (Angel's Herald: one creature can't be all three colours' sacrifice)
        {
            let pools: Vec<Vec<CardId>> = cost.iter().filter_map(|c| if let Cost::Sacrifice { filter, n: 1 } = c { Some(self.sac_pool(&crate::pay::strip_x_filter(filter), card, p)) } else { None }).collect();
            if pools.len() >= 2 {
                fn assign(pools: &[Vec<CardId>], i: usize, used: &mut Vec<CardId>) -> bool {
                    if i == pools.len() {
                        return true;
                    }
                    for &c in &pools[i] {
                        if !used.contains(&c) {
                            used.push(c);
                            if assign(pools, i + 1, used) {
                                return true;
                            }
                            used.pop();
                        }
                    }
                    false
                }
                if !assign(&pools, 0, &mut vec![]) {
                    return false;
                }
            }
        }
        // costs
        let mut mana = ManaCost::default();
        let mut sources_used: Vec<CardId> = vec![];
        for part in cost {
            match part {
                Cost::Mana { cost } => {
                    if let Ok(m) = ManaCost::parse(cost) {
                        mana.shards.extend(m.shards);
                    }
                }
                Cost::AttachedManaCost => {
                    if let Some(h) = c.attached_to {
                        if let Ok(m) = ManaCost::parse(&self.def_of(h).cost.to_symbols()) {
                            mana.shards.extend(m.shards);
                        }
                    }
                }
                Cost::ManaAmount { amount } => {
                    // sized by the chosen target: judge by the cheapest legal choice for the first target slot
                    let cheapest = if targets.is_empty() {
                        // no targets: the amount is computed from the board alone (Fugitive Codebreaker's disguise cost)
                        let ec: EvalCtx<'_> = EvalCtx::with_controller(card, p);
                        Some(self.eval_amount(amount, &ec).max(0))
                    } else { None }.or_else(|| if targets.len() >= 2 {
                        // two target slots (Wayta's fight costs {2} less when both are creatures you control): the cheapest pair of distinct legal targets
                        let a_opts = self.legal_targets(&targets[0], card, p);
                        let b_opts = self.legal_targets(&targets[1], card, p);
                        let mut best: Option<i32> = None;
                        for a in a_opts.iter().take(40) {
                            for b in b_opts.iter().take(40) {
                                if a == b {
                                    continue;
                                }
                                let two = [Target { obj: *a, zcc: self.obj_zcc(a) }, Target { obj: *b, zcc: self.obj_zcc(b) }];
                                let mut ec: EvalCtx<'_> = EvalCtx::with_controller(card, p);
                                ec.targets = &two;
                                ec.tidx = &[0, 1];
                                let v = self.eval_amount(amount, &ec).max(0);
                                best = Some(best.map_or(v, |x| x.min(v)));
                            }
                        }
                        best
                    } else { None }).or_else(|| targets.first().and_then(|spec| {
                        self.legal_targets(spec, card, p)
                            .into_iter()
                            .map(|o| {
                                let one = [Target { obj: o, zcc: self.obj_zcc(&o) }];
                                let mut ec: EvalCtx<'_> = EvalCtx::with_controller(card, p);
                                ec.targets = &one;
                                ec.tidx = &[0];
                                self.eval_amount(amount, &ec).max(0)
                            })
                            .min()
                    }));
                    mana.add_generic(cheapest.unwrap_or(0).min(250) as u8);
                }
                Cost::Tap => {
                    if c.tapped || !self.can_use_tap_symbol(card) || !self.can_become_tapped(card, false) {
                        return false;
                    }
                    sources_used.push(card);
                }
                Cost::Untap => {
                    // {Q} has the same summoning-sickness restriction as {T}
                    if !c.tapped || !self.can_use_tap_symbol(card) || (c.counter(CounterKind::Stun) > 0 && self.stun_locked(card)) {
                        return false;
                    }
                }
                Cost::PayLife { n } => {
                    if !self.can_pay_life(p, *n) {
                        return false;
                    }
                }
                Cost::PayLifeX { amount } => {
                    if !self.can_pay_life(p, self.life_cost(card, amount)) {
                        return false;
                    }
                }
                Cost::Mill { n } | Cost::ExileTop { n } => {
                    if !self.simple_cost_ok(p, card, part) {
                        return false;
                    }
                }
                Cost::SacrificeSelf => {
                    if !self.simple_cost_ok(p, card, part) {
                        return false;
                    }
                }
                Cost::Sacrifice { filter, n } => {
                    if self.count_eligible(&self.sac_cost_filter_check(filter), card, p, Zone::Battlefield) < *n as usize {
                        return false;
                    }
                }
                Cost::Discard { filter, n, .. } => {
                    // a mana value tied to the ability's own target (Kozilek, the Great Distortion) is checked once the target is chosen
                    if self.count_eligible(&crate::pay::strip_x_filter(filter), card, p, Zone::Hand) < *n as usize {
                        return false;
                    }
                }
                Cost::PutExiledIntoGraveyard { filter } => {
                    if self.count_eligible(filter, card, p, Zone::Exile) < 1 {
                        return false;
                    }
                }
                Cost::ExileSpellFromStack { filter } => {
                    if self.stack_spells_for_cost(p, card, filter).is_empty() {
                        return false;
                    }
                }
                Cost::ExileFromSingleGraveyard { filter, n, .. } => {
                    if self.single_graveyard_pools(filter, card, p, *n).is_empty() {
                        return false;
                    }
                }
                Cost::ExileGraveyardTypes { n } => {
                    if !self.gy_types_ok(&self.gy_types_pool(p, card), *n) {
                        return false;
                    }
                }
                Cost::ExileGraveyardMv { filter, n } => {
                    if !self.gy_mv_ok(&self.gy_mv_pool(filter, p, card), *n) {
                        return false;
                    }
                }
                Cost::ExileTotalMvX { filter } => {
                    if self.exile_mv_pool(filter, p, card).is_empty() {
                        return false;
                    }
                }
                Cost::ExileCraftMaterials { bf, gy, min, share_type } => {
                    if !self.craft_ok(bf, gy, *min, *share_type, p, card) {
                        return false;
                    }
                }
                Cost::ExileFromGraveyard { filter, n, bottom } => {
                    if self.count_eligible(&crate::pay::strip_x_filter(filter), card, p, Zone::Graveyard) < *n as usize {
                        return false;
                    }
                }
                Cost::ExileFromGraveyardAmount { filter, amount } => {
                    if self.count_eligible(filter, card, p, Zone::Graveyard) < self.life_cost(card, amount) as usize {
                        return false;
                    }
                }
                Cost::ExileFromGraveyardPayCost { filter } => {
                    let Some(shards) = self.cheapest_graveyard_cost(p, filter, card, &mana) else {
                        return false;
                    };
                    mana.shards.extend(shards);
                }
                Cost::CollectEvidence { n } => {
                    if self.evidence_cards(p, *n, card).is_none() {
                        return false;
                    }
                }
                Cost::ExileSymbolsFromGraveyard { filter, color, n } => {
                    if self.symbol_cards(p, filter, *color, *n, card).is_none() {
                        return false;
                    }
                }
                Cost::ExileFromHand { filter, n, .. } => {
                    if self.count_eligible(&crate::pay::strip_x_filter(filter), card, p, Zone::Hand) < *n as usize {
                        return false;
                    }
                }
                Cost::TapPermanents { filter, n } => {
                    let cnt = self
                        .battlefield
                        .iter()
                        .filter(|id| {
                            let k = &self.cards[id.0 as usize];
                            k.cur.controller == p && !k.tapped && self.can_become_tapped(**id, false) && self.matches_card(filter, **id, card, p)
                        })
                        .count();
                    if cnt < *n as usize {
                        return false;
                    }
                }
                Cost::UntapPermanents { filter, n, opponent } => {
                    if self.untappable_for_cost(p, card, filter, *opponent).len() < *n as usize {
                        return false;
                    }
                }
                Cost::RemoveCounters { kind, n } => {
                    if c.counter(*kind) < *n {
                        return false;
                    }
                }
                Cost::RemoveAnyCounters { n } => {
                    if c.counters.iter().map(|(_, x)| *x as i32).sum::<i32>() < *n {
                        return false;
                    }
                }
                Cost::RemoveAnyCountersFrom { filter, n } => {
                    if !self.any_counters_removable(filter, *n, card, p) {
                        return false;
                    }
                }
                Cost::RemoveCountersFrom { filter, kind, n, among, suspended } => {
                    if !self.counters_removable_from(filter, *kind, *n, *among, *suspended, card, p) {
                        return false;
                    }
                }
                Cost::PutCountersOnChosen { filter, opponent, .. } => {
                    if if *opponent { self.counter_targets(p, filter, card, true).is_empty() } else { self.count_eligible(filter, card, p, Zone::Battlefield) < 1 } {
                        return false;
                    }
                }
                Cost::TapShareType { filter, n } => {
                    if self.share_type_group(&self.tappable_for_cost(p, card, filter), *n).is_none() {
                        return false;
                    }
                }
                Cost::SacrificeDistinctNames { filter, n } => {
                    if !self.distinct_names_ok(&self.sac_pool(filter, card, p), *n) {
                        return false;
                    }
                }
                Cost::DiscardSameName { filter, n } => {
                    if self.same_name_group(&self.hand_matching(p, card, filter), *n).is_none() {
                        return false;
                    }
                }
                Cost::DiscardDistinctNames { filter, n } => {
                    if !self.distinct_names_ok(&self.hand_matching(p, card, filter), *n) {
                        return false;
                    }
                }
                Cost::RevealShareColor { n } => {
                    if !self.share_color_group(&self.hand_matching(p, card, &Filter::default()), *n) {
                        return false;
                    }
                }
                Cost::TapTotalPower { filter, power } => {
                    if self.tappable_for_cost(p, card, filter).iter().map(|id| self.cards[id.0 as usize].cur.power.max(0)).sum::<i32>() < *power {
                        return false;
                    }
                }
                Cost::SacrificeTotalPower { filter, power } => {
                    if self.eligible_permanents(p, filter, card).iter().map(|id| self.cards[id.0 as usize].cur.power.max(0)).sum::<i32>() < *power {
                        return false;
                    }
                }
                Cost::Behold { filter, n, choose_type } => {
                    if !self.beholdable(filter, card, p, *n, *choose_type) {
                        return false;
                    }
                }
                Cost::ChooseCreatureOrWarped => {
                    if self.greatest_creature_or_warped_power(p).is_none() {
                        return false;
                    }
                }
                Cost::UnattachSelf => {
                    if self.cards[card.0 as usize].attached_to.is_none() {
                        return false;
                    }
                }
                Cost::UnattachAttached { filter } => {
                    if self.attached_matching(card, filter, p).is_none() {
                        return false;
                    }
                }
                Cost::PayEnergy { n } => {
                    if self.players[p as usize].energy < *n {
                        return false;
                    }
                }
                Cost::Crew { power } => {
                    if self.crew_power(p, card) < self.crew_requirement(card, *power) {
                        return false;
                    }
                }
                Cost::ReturnToHand { filter, n } | Cost::ExileFromBattlefield { filter, n } => {
                    if self.count_eligible(filter, card, p, Zone::Battlefield) < *n as usize {
                        return false;
                    }
                }
                Cost::Reveal { filter, n } => {
                    if self.players[p as usize].hand.iter().filter(|h| (**h != card || filter.this) && self.matches_card(&Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() }, **h, card, p)).count() < *n as usize {
                        return false;
                    }
                }
                Cost::Waterbend { n } => {
                    let cnt = self.waterbend_sources(p).len() as i32;
                    mana.add_generic((*n - cnt).max(0) as u8);
                }
                Cost::TapCreaturesOrMana { n } => {
                    let cnt = self.creature_bend_sources(p).len() as i32;
                    mana.add_generic((*n - cnt).max(0) as u8);
                }
                Cost::Or { options } => {
                    if !options.iter().any(|o| self.cost_parts_payable(p, card, o)) {
                        return false;
                    }
                }
                Cost::SacrificeGrantSource | Cost::ExileGrantSource | Cost::RemoveAllCountersGrantSource { .. } => {
                    let gb = if ability >= GRANTED_BASE { c.cur.granted_by.get((ability - GRANTED_BASE) as usize).copied() } else { None };
                    if !gb.map_or(false, |g| self.cards[g.0 as usize].zone == Zone::Battlefield && (!matches!(part, Cost::SacrificeGrantSource | Cost::ExileGrantSource) || self.cards[g.0 as usize].cur.controller == p)) {
                        return false;
                    }
                }
                Cost::DiscardSelf | Cost::ExileSelf | Cost::SacrificeSelf | Cost::PutSelfOnLibraryBottom => {}
                _ => {}
            }
        }
        self.reduce_hand_ability_cost(p, card, &mut mana, self.is_equip_for(card, ab), Self::has_sac_self_cost(ab), Self::is_boast_ability(ab), Self::is_foretell_ability(ab), matches!(ab, Ability::Activated { once: true, .. }), Self::is_plot_ability(ab), matches!(ab, Ability::Activated { power_up: true, .. }));
        self.restrict_activation_mana(card, &mut mana);
        // a cost change that depends on the chosen target: assume the best case for "can it be activated"
        let tdelta = if matches!(ab, Ability::Activated { targets, .. } if targets.is_empty()) { 0 } else { self.target_ability_cost_delta(p, card, self.is_equip_for(card, ab), None) };
        if tdelta < 0 {
            let mut reduce = -tdelta;
            for sh in mana.shards.iter_mut() {
                if let Shard::Generic { n } = sh {
                    let r = (*n as i32).min(reduce);
                    *n -= r as u8;
                    reduce -= r;
                }
            }
            mana.shards.retain(|s| !matches!(s, Shard::Generic { n: 0 }));
        }
        if !mana.is_empty() {
            // the source can't tap for mana to pay its own {T} cost
            self.pay_for.set(Some(card));
            self.pay_is_ability.set(true);
            self.pay_is_equip.set(self.is_equip_for(card, ab));
            self.pay_is_power_up.set(matches!(ab, Ability::Activated { power_up: true, .. }));
            self.pay_is_foretell.set(Self::is_foretell_ability(ab));
            self.pay_ability_has_x.set(cost.iter().any(|part| matches!(part, Cost::Mana { cost } if cost.contains("{X}")) || matches!(part, Cost::XTimes { .. })));
            let prev_reserve = self.sac_reserve.replace(self.forced_sac_reserve(p, card, cost));
            let sources: Vec<ManaSourceOpt> = self.mana_sources_for_cost(p, Some(&mana)).into_iter().filter(|s| !sources_used.contains(&s.card)).collect();
            *self.sac_reserve.borrow_mut() = prev_reserve;
            self.pay_for.set(None);
            self.pay_is_ability.set(false);
            self.pay_is_equip.set(false);
            self.pay_is_power_up.set(false);
            self.pay_is_foretell.set(false);
            // an X that must be at least N (XMinimum): the cost has to be payable with X = N
            let min_x = cost.iter().filter_map(|c| if let Cost::XMinimum { n } = c { Some(*n as u32) } else { None }).max().unwrap_or(0);
            if min_x > 0 {
                for part in cost {
                    if let Cost::XTimes { cost: inner } = part {
                        if !self.simple_cost_ok(p, card, &Self::scale_cost(inner, min_x as i32)) {
                            return false;
                        }
                    }
                }
            }
            if plan_payment_with(&sources, &self.spend_as_cost(p, &mana), min_x, self.players[p as usize].pool, if self.can_pay_life(p, 1) { self.players[p as usize].life } else { 0 }).is_none() {
                return false;
            }
        }
        true
    }

    /// Emerge / offering / harmonize: lower `m` by what giving up `id` is worth (`kind`: 0 = generic by mana value, 1 = by its whole mana cost, 2 = generic by power).
    pub(crate) fn reduce_mana_by(&self, m: &mut ManaCost, kind: u8, id: CardId) {
        let mut g: u32 = match kind {
            0 => self.def_of(id).cost.mana_value(0) as u32,
            2 => self.cards[id.0 as usize].cur.power.max(0) as u32,
            _ => self.def_of(id).cost.shards.iter().map(|s| if let Shard::Generic { n } = s { *n as u32 } else { 0 }).sum(),
        };
        if kind == 1 {
            for s in &self.def_of(id).cost.shards {
                if !matches!(s, Shard::Generic { .. }) {
                    if let Some(pos) = m.shards.iter().position(|x| x == s) {
                        m.shards.remove(pos);
                    }
                }
            }
        }
        for sh in m.shards.iter_mut() {
            if let Shard::Generic { n } = sh {
                let r = (*n as u32).min(g);
                *n -= r as u8;
                g -= r;
            }
        }
        m.shards.retain(|s| !matches!(s, Shard::Generic { n: 0 }));
    }

    /// The reducing cost part of an alternative cost, if it has one: (filter, kind).
    pub(crate) fn alt_reducer(d: &crate::carddb::Def, alt: Option<u8>) -> Option<(FilterBox, u8)> {
        let i = alt?;
        match d.abilities.iter().filter(|a| matches!(a, Ability::AlternativeCost { .. })).nth(i as usize) {
            Some(Ability::AlternativeCost { cost, .. }) => cost.iter().find_map(|c| match c {
                Cost::SacrificeReduceByManaValue { filter } => Some((filter.clone(), 0)),
                Cost::SacrificeReduceByManaCost { filter } => Some((filter.clone(), 1)),
                Cost::TapReduceByPower { filter } => Some((filter.clone(), 2)),
                _ => None,
            }),
            _ => None,
        }
    }

    /// Untapped artifacts and creatures `p` controls (what waterbend can tap).
    pub(crate) fn creature_bend_sources(&self, p: PlayerId) -> Vec<CardId> {
        self.waterbend_sources(p).into_iter().filter(|id| self.cards[id.0 as usize].cur.types.has(CardType::Creature)).collect()
    }

    pub(crate) fn waterbend_sources(&self, p: PlayerId) -> Vec<CardId> {
        self.battlefield
            .iter()
            .copied()
            .filter(|id| {
                let c = &self.cards[id.0 as usize];
                c.cur.controller == p && !c.tapped && self.can_become_tapped(*id, false) && (c.cur.types.has(CardType::Artifact) || c.cur.types.has(CardType::Creature))
            })
            .collect()
    }

    /// Permanents `p` controls that match the filter (ignoring its controller field), e.g. to sacrifice.
    pub(crate) fn eligible_battlefield(&self, filter: &Filter, source: CardId, p: PlayerId) -> Vec<CardId> {
        let mut f = filter.clone();
        f.controller = PlayerFilter::Any;
        self.battlefield.iter().copied().filter(|id| self.cards[id.0 as usize].cur.controller == p && self.matches_card(&f, *id, source, p)).collect()
    }

    /// For `Cost::ExileFromSingleGraveyard`: per player, their graveyard cards matching `filter`, for each graveyard holding at least `n`.
    pub(crate) fn single_graveyard_pools(&self, filter: &Filter, source: CardId, p: PlayerId, n: i32) -> Vec<Vec<CardId>> {
        let mut f = filter.clone();
        f.controller = PlayerFilter::Any;
        (0..self.players.len())
            .map(|q| self.players[q].graveyard.iter().copied().filter(|id| self.matches_card(&f, *id, source, p)).collect::<Vec<CardId>>())
            .filter(|v| v.len() as i32 >= n.max(1))
            .collect()
    }

    /// Several "exile a <type> from the battlefield / your graveyard" cost slots (Throne of the Grim Captain's craft) each need their own card:
    /// one changeling can't pay for two of them. True when distinct cards can be assigned to all the slots.
    fn exile_slots_joint_ok(&self, p: PlayerId, card: CardId, cost: &[Cost]) -> bool {
        type Opt = (Zone, Filter, usize);
        fn single(part: &Cost) -> Option<Opt> {
            match part {
                Cost::ExileFromBattlefield { filter, n } => Some((Zone::Battlefield, filter.clone().into_filter(), *n as usize)),
                Cost::ExileFromGraveyard { filter, n, .. } => Some((Zone::Graveyard, filter.clone().into_filter(), *n as usize)),
                _ => None,
            }
        }
        let mut slots: Vec<Vec<Opt>> = vec![];
        for part in cost {
            if let Some(o) = single(part) {
                slots.push(vec![o]);
            } else if let Cost::Or { options } = part {
                let opts: Vec<Opt> = options.iter().filter_map(|o| if o.len() == 1 { single(&o[0]) } else { None }).collect();
                if !opts.is_empty() && opts.len() == options.len() {
                    slots.push(opts);
                }
            }
        }
        if slots.len() < 2 {
            return true;
        }
        let cands = |o: &Opt| -> Vec<CardId> {
            let mut f = o.1.clone();
            f.controller = PlayerFilter::Any;
            match o.0 {
                Zone::Battlefield => self.battlefield.iter().copied().filter(|id| self.cards[id.0 as usize].cur.controller == p && self.matches_card(&f, *id, card, p)).collect(),
                _ => self.players[p as usize].graveyard.iter().copied().filter(|id| self.matches_card(&f, *id, card, p)).collect(),
            }
        };
        fn go(slots: &[Vec<Opt>], i: usize, used: &mut Vec<CardId>, cands: &dyn Fn(&Opt) -> Vec<CardId>) -> bool {
            let Some(slot) = slots.get(i) else { return true };
            for o in slot {
                let free: Vec<CardId> = cands(o).into_iter().filter(|c| !used.contains(c)).collect();
                if o.2 <= 1 {
                    for c in free.iter().take(if o.2 == 0 { 1 } else { free.len() }) {
                        if o.2 == 0 {
                            if go(slots, i + 1, used, cands) {
                                return true;
                            }
                            break;
                        }
                        used.push(*c);
                        if go(slots, i + 1, used, cands) {
                            return true;
                        }
                        used.pop();
                    }
                    if o.2 == 0 && free.is_empty() && go(slots, i + 1, used, cands) {
                        return true;
                    }
                } else if free.len() >= o.2 {
                    let k = used.len();
                    used.extend(free.iter().take(o.2).copied());
                    if go(slots, i + 1, used, cands) {
                        return true;
                    }
                    used.truncate(k);
                }
            }
            false
        }
        go(&slots, 0, &mut vec![], &cands)
    }

    pub(crate) fn count_eligible(&self, filter: &Filter, source: CardId, p: PlayerId, zone: Zone) -> usize {
        let mut f = filter.clone();
        f.controller = PlayerFilter::Any;
        match zone {
            Zone::Battlefield => self
                .battlefield
                .iter()
                .filter(|id| self.cards[id.0 as usize].cur.controller == p && self.matches_card(&f, **id, source, p))
                .count(),
            Zone::Hand => self.players[p as usize].hand.iter().filter(|id| self.matches_card(&f, **id, source, p)).count(),
            Zone::Graveyard => self.players[p as usize].graveyard.iter().filter(|id| self.matches_card(&f, **id, source, p)).count(),
            Zone::Exile => self.cards.iter().enumerate().filter(|(i, c)| c.zone == Zone::Exile && self.matches_card(&f, CardId(*i as u32), source, p)).count(),
            _ => 0,
        }
    }

    // ------------------------------------------------------------------ starting a cast / activation

    fn new_item(&mut self, p: PlayerId, source: CardId, kind: ItemKind) -> StackItem {
        self.next_stack_id += 1;
        let locked_x = {
            let db = self.db.clone();
            let d = db.def(self.cards[source.0 as usize].def);
            d.abilities.iter().find_map(|a| if let Ability::XFromAmount { amount } = a { Some(amount.clone()) } else { None }).map(|amt| {
                let mut ctx = EvalCtx::of(self, source);
                ctx.controller = p;
                self.eval_amount(&amt, &ctx)
            })
        };
        let c = &self.cards[source.0 as usize];
        let mut item = StackItem {
            id: StackId(self.next_stack_id),
            controller: p,
            source,
            src_zcc: c.zcc,
            def: c.def,
            kind,
            targets: vec![],
            x: self.cards[source.0 as usize].x,
            modes: SmallVec::new(),
            ctx: EventCtx::default(),
            copy: false,
            copy_except: vec![],
            divisions: SmallVec::new(),
            spec_counts: SmallVec::new(),
            remembered: vec![],
            granted_snap: None,
            spliced: vec![],
        };
        if let Some(x) = locked_x {
            item.x = x;
        }
        item
    }

    pub fn begin_cast(&mut self, p: PlayerId, card: CardId, alt: Option<u8>) {
        // a fresh cast starts with X = 0 (a free cast of a card that was cast with X before must not reuse that X)
        self.cards[card.0 as usize].x = 0;
        let backup = if self.players[p as usize].allow_cancel {
            let mut b = Box::new(self.clone());
            b.pending = None;
            b.answer = None;
            Some(b)
        } else {
            None
        };
        // casting the other face: the card is that face from now on (it returns to its front face when it leaves the stack/battlefield)
        let alt = if alt == Some(FACE_ALT) {
            let mut chosen = None;
            if let Some(back) = self.def_of(card).other_face {
                self.cards[card.0 as usize].def = back;
                self.reset_derived(card);
                if self.cards[card.0 as usize].zone == Zone::Graveyard {
                    chosen = self.db.def(back).abilities.iter().filter(|a| matches!(a, Ability::AlternativeCost { .. })).position(|a| matches!(a, Ability::AlternativeCost { from: Some(Zone::Graveyard), .. })).map(|i| i as u8);
                } else if self.cards[card.0 as usize].zone == Zone::Hand
                    && self.db.def(back).abilities.iter().any(|a| matches!(a, Ability::AlternativeCost { from: Some(Zone::Hand), .. }))
                {
                    chosen = self.db.def(back).abilities.iter().filter(|a| matches!(a, Ability::AlternativeCost { .. })).position(|a| matches!(a, Ability::AlternativeCost { from: Some(Zone::Hand), .. })).map(|i| i as u8);
                }
            }
            // an Adventure half the player can only afford through a granted alternative cost (As Foretold's {0})
            if chosen.is_none() && self.cards[card.0 as usize].zone == Zone::Hand && !self.can_afford(p, &self.effective_cost(p, card, None), 0) {
                let z = self.cards[card.0 as usize].zone;
                if let Some(k) = self.granted_alt_costs(p, card, z).iter().position(|g| !g.1.is_empty()) {
                    chosen = Some(GRANT_ALT_BASE + k as u8);
                }
            }
            chosen
        } else {
            alt
        };
        self.pay_riders.borrow_mut().clear();
        self.pay_rider_srcs.borrow_mut().clear();
        self.pay_rider_units.borrow_mut().clear();
        self.copy_rider_always.set(false);
        self.cards[card.0 as usize].rider_counters = 0;
        self.cards[card.0 as usize].rider_charge = 0;
        self.cards[card.0 as usize].rider_loyalty = 0;
        self.cards[card.0 as usize].riot_rider = false;
        self.cards[card.0 as usize].exile_link = None;
        self.cards[card.0 as usize].finality_rider = false;
        self.cards[card.0 as usize].extra_mode = false;
        let from = self.cards[card.0 as usize].zone;
        if from == Zone::Sideboard && !self.outside_grants.iter().any(|&(pl, t)| pl == p && t == self.turn) {
            if let Some(i) = self.outside_card_grant_idx(p, card) {
                let g = self.outside_card_grants.remove(i);
                self.cards[card.0 as usize].extra_mode = g.4;
                self.outside_play_used(g.3);
            }
        }
        let def = self.db.clone();
        let d = def.def(self.cards[card.0 as usize].def);
        let (mut specs, mut modal) = self.spell_specs(card);
        if alt == Some(FACE_DOWN_ALT) {
            specs.clear();
            modal = None;
        }
        // overload: cast this way the spell has no targets
        if let Some(i) = alt {
            if let Some(Ability::AlternativeCost { effects: Some(_), targets, .. }) = d.abilities.iter().filter(|a| matches!(a, Ability::AlternativeCost { .. })).nth(i as usize) {
                specs = targets.clone().unwrap_or_default();
                modal = None;
            }
        }
        self.cards[card.0 as usize].cast_alt = alt;
        let mut mana = if self.free_cast_now.replace(false) { ManaCost::default() } else { self.effective_cost(p, card, alt) };
        if let Some(em) = self.cards[card.0 as usize].cast_extra_mana.take() {
            if let Some(whole) = em.strip_prefix('=') {
                // the whole cost paid instead of the mana cost ("by paying {1} rather than paying its mana cost")
                if let Ok(m) = ManaCost::parse(whole) {
                    mana = m;
                }
            } else if let Some(less) = em.strip_prefix('-') {
                // "costs {N} less to cast": reduce the generic part
                if let Ok(m) = ManaCost::parse(less) {
                    let mut left: u32 = m.shards.iter().map(|sh| if let Shard::Generic { n } = sh { *n as u32 } else { 0 }).sum();
                    for sh in mana.shards.iter_mut() {
                        if let Shard::Generic { n } = sh {
                            let d = (*n as u32).min(left);
                            *n -= d as u8;
                            left -= d;
                        }
                    }
                    mana.shards.retain(|sh| !matches!(sh, Shard::Generic { n: 0 }));
                }
            } else if let Ok(m) = ManaCost::parse(&em) {
                mana.shards.extend(m.shards);
            }
        }
        let x_red = self.x_red.get();
        let mut extra: Vec<Cost> = vec![];
        for a in &d.abilities {
            if let Ability::AdditionalCost { cost } = a {
                extra.extend(cost.iter().cloned());
            }
        }
        extra.extend(self.spell_extra_costs(p, card, d));
        let life_tax = self.commander_life_tax(p, card);
        if life_tax > 0 {
            extra.push(Cost::PayLife { n: life_tax });
        }
        if alt.is_none() && from == Zone::Graveyard {
            if let Some(fc) = self.forage_cost(p, card) {
                extra.push(fc);
            }
        }
        let mut sac_exile = false;
        if alt.is_none() || from == Zone::Library || alt == Some(FACE_DOWN_ALT) {
            let xl = self.play_extra_life(p, card, from);
            if xl > 0 {
                extra.push(Cost::PayLife { n: xl });
            }
            extra.extend(self.play_extra_costs(p, card, from));
            if self.play_sac_exile(p, card, from) {
                extra.push(Cost::Sacrifice { filter: (Filter { types: vec![CardType::Creature], controller: PlayerFilter::You, ..Default::default() }).into(), n: 1 });
                sac_exile = true;
            }
            if self.play_perm_exile_after(p, card, from) {
                sac_exile = true;
            }
        }
        if let Some(i) = alt {
            if let Some(Ability::AlternativeCost { cost, .. }) =
                d.abilities.iter().filter(|a| matches!(a, Ability::AlternativeCost { .. })).nth(i as usize)
            {
                for part in cost {
                    if !matches!(part, Cost::Mana { .. }) {
                        extra.push(part.clone());
                    }
                }
            }
        }
        // pay life equal to the mana value instead of mana: a granted alternative cost, or an exile-cast permission
        let life_mv_grant = match alt {
            Some(i) if i >= GRANT_ALT_BASE && i < FACE_DOWN_ALT => self.granted_alt_costs_at_cast(p, card, from, i).map_or(false, |g| g.3),
            _ => false,
        };
        self.cards[card.0 as usize].cast_exile_after = sac_exile;
        if let Some(i) = alt {
            if i >= GRANT_ALT_BASE && i < FACE_DOWN_ALT {
                if let Some(g) = self.granted_alt_costs_at_cast(p, card, from, i) {
                    extra.extend(g.4.iter().cloned());
                    self.cards[card.0 as usize].cast_exile_after = g.5;
                }
            }
        }
        if from == Zone::Graveyard && self.gy_exile_grants.iter().any(|(pl, t, f, src)| *pl == p && *t == self.turn && self.matches_card(&Filter { controller: PlayerFilter::Any, graveyard_top_card: false, ..f.clone() }, card, *src, p)) {
            self.cards[card.0 as usize].cast_exile_after = true;
        }
        if from == Zone::Graveyard && self.gy_card_riders.iter().any(|(cid, z, t, ex, _)| *ex && *t == self.turn && *cid == card && *z == self.cards[card.0 as usize].zcc) && self.gy_card_grants.iter().any(|(pl, t, cid, z)| *pl == p && *t == self.turn && *cid == card && *z == self.cards[card.0 as usize].zcc) {
            self.cards[card.0 as usize].cast_exile_after = true;
        }
        if from == Zone::Graveyard && self.gy_free_grants.iter().any(|(pl, t, cid, z)| *pl == p && *t == self.turn && *cid == card && *z == self.cards[card.0 as usize].zcc) {
            self.cards[card.0 as usize].cast_exile_after = true;
        }
        if from == Zone::Exile && self.cards[card.0 as usize].play_exile_after {
            self.cards[card.0 as usize].cast_exile_after = true;
        }
        if life_mv_grant || (self.cards[card.0 as usize].play_life_mv && from == Zone::Exile) {
            extra.push(Cost::PayLifeX { amount: Amount::ManaValueOf { of: Sel::This } });
        }
        // a once-each-turn "may cast from your graveyard / library" permission is used up (found before the card moves, so zone filters still match)
        let used_src: Option<(CardId, u8)> = self.once_play_src(p, card, from);
        let top_src = self.play_exile_top_src(p, card, from);
        let mark_src = self.play_mark_src(p, card, from);
        if from == Zone::Exile {
            self.revoke_play_one(card);
            self.use_play_per_type(card);
            self.revoke_free_one(card);
            // Shattered Seraph: "until this card is cast from exile"
            if self.effects.iter().any(|e| e.source == card && e.duration == Duration::UntilSourceCast) {
                self.effects.retain(|e| !(e.source == card && e.duration == Duration::UntilSourceCast));
                self.layers_dirty = true;
            }
        }
        // "if you do, it enters tapped" (Kaldring, the Rimestaff)
        let gy_tapped = from == Zone::Graveyard && self.gy_tapped_grant(p, card);
        // use up the once-each-turn kind / type of a `per_type` / `per_kind` permission while the card is still in the graveyard (its filter may read graveyard state)
        if from == Zone::Graveyard {
            self.use_per_type(p, card, from);
        }
        // move the card to the stack
        self.remove_from_zone_lists(card);
        if from == Zone::Graveyard {
            // a card cast from the graveyard (flashback, harmonize, ...) left it ("if a card left your graveyard this turn")
            let o = self.cards[card.0 as usize].owner as usize;
            if self.left_graveyard_by.len() <= o {
                self.left_graveyard_by.resize(o + 1, 0);
            }
            self.left_graveyard_by[o] += 1;
            if self.def_of(card).types.has(CardType::Creature) {
                if self.left_graveyard_creature_by.len() <= o {
                    self.left_graveyard_creature_by.resize(o + 1, 0);
                }
                self.left_graveyard_creature_by[o] += 1;
            }
        }
        let mut snap: Vec<Sym> = vec![];
        let mut snap_power = 0i32;
        for &b in &self.battlefield {
            let bc = &self.cards[b.0 as usize];
            if bc.cur.controller == p {
                if bc.cur.types.has(CardType::Creature) {
                    snap_power = snap_power.max(bc.cur.power);
                }
                for s in bc.cur.subtypes.iter() {
                    if !snap.contains(s) {
                        snap.push(*s);
                    }
                }
                if bc.cur.kw.has(Keyword::Changeling) && !snap.contains(&Sym::new("*")) {
                    snap.push(Sym::new("*"));
                }
            }
        }
        let own_flash = from == Zone::Hand && self.cast_with_own_flash_only(p, card);
        {
            let c = &mut self.cards[card.0 as usize];
            c.cast_own_flash = own_flash;
            c.cast_subtypes = snap;
            c.cast_power = snap_power;
            c.zone = Zone::Stack;
            c.zcc += 1;
            c.cast_from = Some(from);
            c.controller = p;
            c.free_turn = 0;
            c.free_via = None;
            c.enter_tapped_once = gy_tapped;
        }
        self.cards[card.0 as usize].play_leave_gain = used_src.map_or(0, |u| u.1);
        if let Some((b, _)) = used_src {
            self.once_play_used(b);
        }
        if let Some(b) = top_src {
            self.exile_top_for(p, b);
        }
        if let Some(b) = mark_src {
            self.cards[card.0 as usize].played_via = Some((b, self.cards[b.0 as usize].zcc));
        }
        // cast from a graveyard through a "finality" permission (not an own/flashback cast): the creature enters with a finality counter
        if from == Zone::Graveyard && alt.is_none() {
            let fin = self.battlefield.iter().any(|&b| {
                let sc = &self.cards[b.0 as usize];
                if sc.cur.controller != p || sc.cur.no_abilities {
                    return false;
                }
                let sd = self.db.def(sc.def);
                sd.statics.iter().any(|&i| matches!(&sd.abilities[i as usize], Ability::Static { effect: StaticEffect::MayPlayFrom { zone: Zone::Graveyard, filter, finality: true, .. }, condition, .. }
                    if self.matches_card(&Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() }, card, b, p)
                        && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(b, p)))))
            });
            let fin = fin || self.forage_cost(p, card).is_some();
            self.cards[card.0 as usize].finality_rider = fin;
            // "you may cast a creature spell from your graveyard this turn. If you do, it enters with a finality counter and is a Vampire" (Tarrian's Journal): the grant is used up
            if let Some(pos) = self.gy_once_grants.iter().position(|(pl, t, f, src, _, _)| *pl == p && *t == self.turn && self.matches_card(&Filter { controller: PlayerFilter::Any, ..f.clone() }, card, *src, p)) {
                let (_, _, _, _, fin2, sub) = self.gy_once_grants.remove(pos);
                if fin2 {
                    self.cards[card.0 as usize].finality_rider = true;
                }
                if sub.is_some() {
                    self.cards[card.0 as usize].extra_sub = sub;
                }
            }
        }
        // casting a prepare spell unprepares its creature
        if let Some(cr) = self.cards[card.0 as usize].prepare_of.take() {
            self.cards[cr.0 as usize].prepared_spell = None;
            self.cards[card.0 as usize].cast_prepared = true;
        }
        self.reset_derived(card);
        self.cards[card.0 as usize].cur.controller = p;
        // a card cast from a graveyard / exile leaves that zone ("whenever a card leaves your graveyard")
        if matches!(from, Zone::Graveyard | Zone::Exile) {
            self.emit(Event::Moved { card, from, to: Zone::Stack });
        }
        if let Some(i) = alt {
            // warp: casting with the warp alternative cost (Void counts spells warped this turn)
            if i < GRANT_ALT_BASE {
                if let Some(Ability::AlternativeCost { cost, .. }) = d.abilities.iter().filter(|a| matches!(a, Ability::AlternativeCost { .. })).nth(i as usize) {
                    if d.abilities.iter().any(|a| matches!(a, Ability::Warp { cost: wc } if cost.as_slice() == [Cost::Mana { cost: wc.clone() }])) {
                        self.warped_this_turn = true;
                    }
                }
            }
            if i >= GRANT_ALT_BASE && i < FACE_DOWN_ALT {
                if let Some(g) = self.granted_alt_costs_at_cast(p, card, from, i) {
                    if g.2 {
                        self.alt_grants_used.push((g.0, self.turn));
                    }
                    if g.4.iter().any(|c| matches!(c, Cost::Blitz)) {
                        self.cards[card.0 as usize].haste_sac_rider = true;
                        self.cards[card.0 as usize].blitz_rider = true;
                    }
                    if g.4.iter().any(|c| matches!(c, Cost::SneakEntry)) {
                        self.cards[card.0 as usize].sneak_rider = true;
                    }
                    if g.4.iter().any(|c| matches!(c, Cost::Evoke)) {
                        self.cards[card.0 as usize].evoke_rider = true;
                    }
                    if g.4.iter().any(|c| matches!(c, Cost::Warp)) {
                        self.cards[card.0 as usize].warp_rider = true;
                        self.warped_this_turn = true;
                    }
                }
            }
        }
        let mut item = self.new_item(p, card, ItemKind::Spell { from, kicked: false, alt });
        item.src_zcc = self.cards[card.0 as usize].zcc;
        // an alternative cost that exiles a card "with mana value X" still announces X (Blazing Shoal)
        let x_from_hand_card = alt.is_some()
            && !mana.has_x()
            && d.cost.has_x()
            && match alt {
                Some(i) if i != FACE_DOWN_ALT && i != FACE_ALT => d
                    .abilities
                    .iter()
                    .filter(|a| matches!(a, Ability::AlternativeCost { .. }))
                    .nth(i as usize)
                    .map_or(false, |a| matches!(a, Ability::AlternativeCost { cost, .. } if cost.iter().any(|c| matches!(c, Cost::ExileFromHand { filter, .. } if filter.mana_value_vs.as_ref().map_or(false, |ac| matches!(*ac.amount, Amount::X)))))),
                _ => false,
            };
        // "sacrifice an artifact or creature with mana value X" on a spell with no {X}: X is still announced first (Nahiri's Sacrifice)
        let x_from_sacrifice = !mana.has_x() && !d.cost.has_x() && extra.iter().any(|c| matches!(c, Cost::Sacrifice { filter, .. } if filter.mana_value_vs.as_ref().map_or(false, |ac| matches!(*ac.amount, Amount::X))));
        let has_x = mana.has_x() || x_from_hand_card || x_from_sacrifice || extra.iter().any(|c| matches!(c, Cost::XTimes { .. }));
        if from == Zone::Command {
            // commander tax is already in `mana`
        }
        let _ = &mut mana;
        let has_kicker = d.abilities.iter().any(|a| matches!(a, Ability::Kicker { .. })) || self.replicate_granted(p, card) || self.offspring_cost(p, card).is_some() || self.conspire_granted(p, card) || self.casualty_granted(p, card).is_some();
        let can_splice = self.players[p as usize].hand.iter().any(|h| {
            *h != card && self.db.def(self.cards[h.0 as usize].def).abilities.iter().any(|a| matches!(a, Ability::Splice { .. }))
        });
        let stage = if can_splice {
            Stage::Splice
        } else if has_kicker {
            Stage::Kicker
        } else if modal.is_some() && !(has_x && self.modal_x_first(card)) {
            Stage::Modes
        } else if has_x {
            Stage::X
        } else if d.abilities.iter().any(|a| matches!(a, Ability::ChooseNumberOnCast { .. })) {
            Stage::Num
        } else if d.abilities.iter().any(|a| matches!(a, Ability::ChooseCreatureTypeOnCast)) {
            Stage::CastType
        } else {
            Stage::Targets
        };
        let remaining = mana.clone();
        let opt_next = stage;
        let stage = if self.optional_spell_costs(p, card).is_empty() { stage } else { Stage::OptCost };
        let stage = if self.frenzy_max(p, card, &mana) > 0 { Stage::Frenzy } else { stage };
        // a spell with convoke / improvise / delve defers the extra-mana question until after those tap or exile permanents (see Stage::Alt)
        let stage = if self.spell_mana_for_counters(p, card) && !self.spell_has_alt_cost(p, card) { Stage::Bonus } else { stage };
        let ctx = CastCtx {
            player: p,
            origin: Origin::Spell { card, alt },
            item,
            stage,
            specs: self.expand_per_opponent(&specs, p),
            spec_idx: 0,
            mana,
            extra,
            extra_idx: 0,
            target_chooser: None,
            asking_chooser: false,
            remaining,
            backup,
            from,
            modal,
            modal_repeat: d.abilities.iter().any(|a| matches!(a, Ability::Modal { allow_repeat: true, .. })),
            what: d.name.clone(),
            paid_life: 0,
            alt_idx: 0,
            assist_state: 0,
            assist_player: 0,
            kicker_idx: 0,
            splice_asked: vec![],
            extra_late: false,
            x_red,
            opt_next,
            opt_idx: 0,
            type_next: Stage::Targets,
            tap_pick: None,
            rm_any: None,
        };
        self.run = Run::Cast(Box::new(ctx));
    }

    pub fn remove_from_zone_lists(&mut self, id: CardId) {
        let (zone, owner) = {
            let c = &self.cards[id.0 as usize];
            (c.zone, c.owner)
        };
        let rm = |v: &mut Vec<CardId>| {
            if let Some(i) = v.iter().position(|x| *x == id) {
                v.remove(i);
            }
        };
        match zone {
            Zone::Battlefield => rm(&mut self.battlefield),
            Zone::Library => rm(&mut self.players[owner as usize].library),
            Zone::Hand => rm(&mut self.players[owner as usize].hand),
            Zone::Graveyard => rm(&mut self.players[owner as usize].graveyard),
            Zone::Exile => rm(&mut self.players[owner as usize].exile),
            Zone::Command => rm(&mut self.players[owner as usize].command),
            Zone::Sideboard => rm(&mut self.players[owner as usize].sideboard),
            Zone::Stack => {}
        }
    }

    pub fn begin_activate(&mut self, p: PlayerId, card: CardId, ability: u16) {
        let backup = if self.players[p as usize].allow_cancel {
            let mut b = Box::new(self.clone());
            b.pending = None;
            b.answer = None;
            Some(b)
        } else {
            None
        };
        let db = self.db.clone();
        let d = db.def(self.cards[card.0 as usize].def);
        let owned = self.ability_owned(card, ability);
        let Some(Ability::Activated { cost, targets, .. }) = owned.as_ref() else { return };
        if self.cards[card.0 as usize].zone == Zone::Graveyard {
            self.graveyard_use_log.push(p);
        }
        let mut mana = ManaCost::default();
        let mut extra = vec![];
        for part in cost {
            if let Cost::Mana { cost } = part {
                if let Ok(m) = ManaCost::parse(cost) {
                    mana.shards.extend(m.shards);
                }
            } else if let Cost::AttachedManaCost = part {
                if let Some(h) = self.cards[card.0 as usize].attached_to {
                    if let Ok(m) = ManaCost::parse(&self.def_of(h).cost.to_symbols()) {
                        mana.shards.extend(m.shards);
                    }
                }
            } else {
                extra.push(part.clone());
            }
        }
        extra.extend(self.static_extra_costs(card, cost));
        self.x_red.set(0);
        self.reduce_hand_ability_cost(p, card, &mut mana, owned.as_ref().map_or(false, |a| self.is_equip_for(card, a)), owned.as_ref().map_or(false, Self::has_sac_self_cost), owned.as_ref().map_or(false, Self::is_boast_ability), owned.as_ref().map_or(false, Self::is_foretell_ability), owned.as_ref().map_or(false, |a| matches!(a, Ability::Activated { once: true, .. })), owned.as_ref().map_or(false, Self::is_plot_ability), owned.as_ref().map_or(false, |a| matches!(a, Ability::Activated { power_up: true, .. })));
        self.restrict_activation_mana(card, &mut mana);
        let mut item = self.new_item(p, card, ItemKind::Ability { ability, mode: None });
        if ability >= GRANTED_BASE {
            item.granted_snap = self.granted_ab(card, ability);
            item.ctx.granted_by = self.cards[card.0 as usize].cur.granted_by.get((ability - GRANTED_BASE) as usize).copied();
        }
        let stage = if mana.has_x() || extra.iter().any(|c| matches!(c, Cost::RemoveLoyaltyX | Cost::RemoveCountersX { .. } | Cost::RemoveXCountersFromOne { .. } | Cost::ExileTotalMvX { .. } | Cost::XTimes { .. })) { Stage::X } else { Stage::Targets };
        let remaining = mana.clone();
        let ctx = CastCtx {
            player: p,
            origin: Origin::Ability { card, ability },
            item,
            stage,
            specs: self.expand_per_opponent(&targets, p),
            spec_idx: 0,
            mana,
            extra,
            extra_idx: 0,
            target_chooser: None,
            asking_chooser: false,
            remaining,
            backup,
            from: self.cards[card.0 as usize].zone,
            modal: None,
            modal_repeat: false,
            what: d.name.clone(),
            paid_life: 0,
            alt_idx: 0,
            assist_state: 0,
            assist_player: 0,
            kicker_idx: 0,
            splice_asked: vec![],
            extra_late: false,
            x_red: self.x_red.get(),
            opt_next: Stage::Targets,
            opt_idx: 0,
            type_next: Stage::Targets,
            tap_pick: None,
            rm_any: None,
        };
        let mut ctx = ctx;
        if let Some((lo, hi, modes)) = self.stack_modal(self.cards[card.0 as usize].def, ability) {
            ctx.modal_repeat = self.stack_modal_repeat(self.cards[card.0 as usize].def, ability);
            ctx.modal = Some((lo, hi, modes.len()));
            ctx.stage = Stage::Modes;
        }
        self.run = Run::Cast(Box::new(ctx));
    }

    /// A "you may" triggered ability being put on the stack, which its controller may decline while choosing its targets.
    /// That is an optional ability, or one whose whole effect is a single "you may [pay ...]" (declining it does nothing else).
    fn trigger_declinable(&self, ctx: &CastCtx) -> bool {
        let ItemKind::Triggered { ability, optional } = ctx.item.kind else { return false };
        if !matches!(ctx.origin, Origin::Trigger) {
            return false;
        }
        if optional {
            return true;
        }
        let owned = if ability < GRANTED_BASE { self.db.def(ctx.item.def).abilities.get(ability as usize).cloned() } else { ctx.item.granted_snap.as_ref().map(|a| (**a).clone()) };
        let Some(Ability::Triggered { effects, .. }) = owned else { return false };
        matches!(
            effects.as_slice(),
            [Effect::May { otherwise, who: None | Some(Sel::You), .. }] if otherwise.is_empty()
        ) || matches!(effects.as_slice(), [Effect::MayPay { payer: Sel::You, .. }])
    }

    /// Start putting a triggered ability on the stack (choose targets, then it goes on).
    pub fn begin_trigger(&mut self, t: PendingTrigger) {
        let db = self.db.clone();
        let d = db.def(t.def);
        // the face the permanent had when the trigger fired (a transformed card that died is back on its front face now)
        let owned = if t.ability < GRANTED_BASE { d.abilities.get(t.ability as usize).cloned() } else { t.granted.as_ref().map(|a| (**a).clone()).or_else(|| self.ability_owned(t.source, t.ability)) };
        let Some(Ability::Triggered { targets, optional, .. }) = owned.as_ref() else { return };
        let mut item = self.new_item(t.controller, t.source, ItemKind::Triggered { ability: t.ability, optional: *optional });
        item.def = t.def;
        if t.ability >= GRANTED_BASE { item.granted_snap = t.granted.clone(); }
        item.ctx = t.ctx;
        item.src_zcc = t.src_zcc;
        item.ctx.ts = t.ts;
        let ctx = CastCtx {
            player: t.controller,
            origin: Origin::Trigger,
            item,
            stage: Stage::Targets,
            specs: self.expand_per_opponent(&targets, t.controller),
            spec_idx: 0,
            mana: ManaCost::default(),
            extra: vec![],
            extra_idx: 0,
            target_chooser: None,
            asking_chooser: false,
            remaining: ManaCost::default(),
            backup: None,
            from: Zone::Battlefield,
            modal: None,
            modal_repeat: false,
            what: d.name.clone(),
            paid_life: 0,
            alt_idx: 0,
            assist_state: 0,
            assist_player: 0,
            kicker_idx: 0,
            splice_asked: vec![],
            extra_late: false,
            x_red: 0,
            opt_next: Stage::Targets,
            opt_idx: 0,
            type_next: Stage::Targets,
            tap_pick: None,
            rm_any: None,
        };
        let mut ctx = ctx;
        if let Some((lo, hi, modes)) = self.stack_modal(t.def, t.ability) {
            ctx.modal_repeat = self.stack_modal_repeat(t.def, t.ability);
            ctx.modal = Some((lo, hi, modes.len()));
            ctx.stage = Stage::Modes;
        }
        self.run = Run::Cast(Box::new(ctx));
    }

    // ------------------------------------------------------------------ the cast state machine

    pub(crate) fn step_cast(&mut self, mut ctx: Box<CastCtx>) -> Flow {
        let p = ctx.player;
        let cancellable = ctx.backup.is_some();
        let answer = self.answer.take();
        // "don't use it": a "you may" trigger declined while its targets are chosen is left off the stack
        if matches!(answer, Some(Choice::Cancel)) && ctx.stage == Stage::Targets && self.trigger_declinable(&ctx) {
            let first = if self.alive(self.active) { self.active } else { self.next_player(self.active) };
            self.run = Run::Priority { player: first, passes: 0 };
            return Flow::Continue;
        }
        // cancel at any point
        if let Some(Choice::Cancel) = answer {
            if let Some(b) = ctx.backup.take() {
                let log = std::mem::take(&mut self.decision_log);
                let events = std::mem::take(&mut self.events);
                *self = *b;
                self.decision_log = log;
                self.events = events;
                // the snapshot was taken while the cast was being set up, so its run state is a placeholder: the player has priority again
                self.run = Run::Priority { player: p, passes: 0 };
                return Flow::Continue;
            }
        }
        let mut answer = answer;
        let mut spins = 0u32;
        loop {
            spins += 1;
            assert!(spins < 10_000, "cast of '{}' never progresses (stage {:?})", ctx.what, ctx.stage);
            match ctx.stage {
                Stage::Bonus => {
                    let src = ctx.item.source;
                    let next = if self.optional_spell_costs(p, src).is_empty() { ctx.opt_next } else { Stage::OptCost };
                    if let Some(Choice::Number { n }) = answer.take() {
                        let mut left = n.clamp(0, 200);
                        if left > 0 {
                            self.cards[src.0 as usize].rider_counters = self.cards[src.0 as usize].rider_counters.saturating_add(left as u8);
                        }
                        while left > 0 {
                            let k = left.min(200) as u8;
                            ctx.mana.add_generic(k);
                            ctx.remaining.add_generic(k);
                            left -= k as i32;
                        }
                        ctx.stage = next;
                        continue;
                    }
                    let prev = self.pay_for.replace(Some(src));
                    let mut max = 0i32;
                    while max < 40 {
                        // the cost still to pay (after any convoke / improvise / delve already applied)
                        let mut m = ctx.remaining.clone();
                        m.add_generic((max + 1) as u8);
                        if self.can_afford(p, &m, 0) {
                            max += 1;
                        } else {
                            break;
                        }
                    }
                    self.pay_for.set(prev);
                    if max == 0 {
                        ctx.stage = next;
                        continue;
                    }
                    self.pending = Some(Decision {
                        player: p,
                        kind: DecisionKind::Number { prompt: format!("Pay how much extra mana for +1/+1 counters on {}?", ctx.what), min: 0, max, cancellable: false },
                    });
                    self.run = Run::Cast(ctx);
                    return Flow::Wait;
                }
                Stage::OptCost => {
                    let src = ctx.item.source;
                    let opts = self.optional_spell_costs(p, src);
                    let i = ctx.opt_idx as usize;
                    if i >= opts.len() {
                        ctx.stage = ctx.opt_next;
                        continue;
                    }
                    let (cost, color) = opts[i].clone();
                    let pos = ctx.mana.shards.iter().position(|s| matches!(s, Shard::Color { c } if *c == color));
                    if let Some(Choice::YesNo { yes }) = answer.take() {
                        ctx.opt_idx += 1;
                        if yes {
                            ctx.extra.extend(cost);
                            if let Some(pos) = pos {
                                ctx.mana.shards.remove(pos);
                            }
                            ctx.remaining = ctx.mana.clone();
                        }
                        continue;
                    }
                    if pos.is_none() || !self.cost_parts_payable(p, src, &cost) {
                        ctx.opt_idx += 1;
                        continue;
                    }
                    self.pending = Some(Decision {
                        player: p,
                        kind: DecisionKind::YesNo { prompt: format!("Pay the additional cost for {}?", ctx.what), source: Some(src) },
                    });
                    self.run = Run::Cast(ctx);
                    return Flow::Wait;
                }
                Stage::Splice => {
                    let src = ctx.item.source;
                    // the answer to the previous question: splice that card (the last one asked)
                    if let Some(Choice::YesNo { yes }) = answer.take() {
                        if yes {
                            if let Some(&h) = ctx.splice_asked.last() {
                                let hd = self.cards[h.0 as usize].def;
                                let db = self.db.clone();
                                if let Some(Ability::Splice { cost, .. }) = db.def(hd).abilities.iter().find(|a| matches!(a, Ability::Splice { .. })) {
                                    for part in cost {
                                        match part {
                                            Cost::Mana { cost } => {
                                                if let Ok(m) = ManaCost::parse(cost) {
                                                    ctx.mana.shards.extend(m.shards);
                                                }
                                            }
                                            other => ctx.extra.push(other.clone()),
                                        }
                                    }
                                }
                                for a in &db.def(hd).abilities {
                                    if let Ability::Spell { targets, .. } = a {
                                        ctx.specs.extend(targets.iter().cloned());
                                    }
                                }
                                ctx.item.spliced.push(hd);
                                ctx.remaining = ctx.mana.clone();
                            }
                        }
                    }
                    // the next card that could be spliced and that the player can afford
                    let db = self.db.clone();
                    let mut ask: Option<CardId> = None;
                    for &h in &self.players[p as usize].hand {
                        if h == src || ctx.splice_asked.contains(&h) {
                            continue;
                        }
                        let Some((onto, cost)) = db.def(self.cards[h.0 as usize].def).abilities.iter().find_map(|a| if let Ability::Splice { onto, cost } = a { Some((onto.clone(), cost.clone())) } else { None }) else { continue };
                        if !self.matches_card(&onto, src, h, p) {
                            continue;
                        }
                        let mut m = ctx.mana.clone();
                        let mut extras: Vec<Cost> = vec![];
                        for part in &cost {
                            match part {
                                Cost::Mana { cost } => {
                                    if let Ok(x) = ManaCost::parse(cost) {
                                        m.shards.extend(x.shards);
                                    }
                                }
                                other => extras.push(other.clone()),
                            }
                        }
                        if self.can_afford(p, &m, 0) && (extras.is_empty() || self.cost_parts_payable(p, src, &extras)) {
                            ask = Some(h);
                            break;
                        }
                    }
                    if let Some(h) = ask {
                        ctx.splice_asked.push(h);
                        self.pending = Some(Decision {
                            player: p,
                            kind: DecisionKind::YesNo { prompt: format!("Splice {} onto {}?", self.def_of(h).name, ctx.what), source: Some(h) },
                        });
                        self.run = Run::Cast(ctx);
                        return Flow::Wait;
                    }
                    let has_kicker = self.db.def(ctx.item.def).abilities.iter().any(|a| matches!(a, Ability::Kicker { .. })) || self.replicate_granted(p, ctx.item.source) || self.offspring_cost(p, ctx.item.source).is_some() || self.conspire_granted(p, ctx.item.source) || self.casualty_granted(p, ctx.item.source).is_some();
                    ctx.stage = if has_kicker {
                        Stage::Kicker
                    } else if ctx.modal.is_some() && !(ctx.mana.has_x() && self.modal_x_first(ctx.item.source)) {
                        Stage::Modes
                    } else if ctx.mana.has_x() {
                        Stage::X
                    } else {
                        Stage::Targets
                    };
                    continue;
                }
                Stage::Kicker => {
                    let mut kickers: Vec<(Vec<Cost>, bool, bool)> = self
                        .db
                        .def(ctx.item.def)
                        .abilities
                        .iter()
                        .filter_map(|a| if let Ability::Kicker { cost, multi, buyback } = a { Some((cost.clone(), *multi, *buyback)) } else { None })
                        .collect();
                    // granted replicate (Ian Chesterton): a multikicker-like extra cost equal to the spell's mana cost; each payment is one copy
                    if let Some(rc) = self.replicate_cost(p, ctx.item.source) {
                        kickers.push((vec![Cost::Mana { cost: rc.unwrap_or_else(|| self.db.def(ctx.item.def).cost.display()) }], true, false));
                    }
                    // granted offspring (Zinnia, Valley's Voice): an optional extra cost; the token is made by a trigger on the granting permanent
                    let off_idx = self.offspring_cost(p, ctx.item.source).map(|oc| {
                        kickers.push((vec![Cost::Mana { cost: oc }], false, false));
                        kickers.len() - 1
                    });
                    // granted conspire (Wort, the Raidmother): tap two untapped creatures you control that share a color with the spell
                    if self.conspire_granted(p, ctx.item.source) {
                        let f = Filter { types: vec![CardType::Creature], controller: PlayerFilter::You, shares_color_with: Some(Box::new(Sel::This)), ..Default::default() };
                        kickers.push((vec![Cost::TapPermanents { filter: f.into(), n: 2 }], false, false));
                    }
                    // granted casualty (Anhelo, the Painter): sacrifice a creature with power N or greater
                    if let Some(cn) = self.casualty_granted(p, ctx.item.source) {
                        let f = Filter { types: vec![CardType::Creature], controller: PlayerFilter::You, power: Some(Cmp::Ge { n: cn }), ..Default::default() };
                        kickers.push((vec![Cost::Sacrifice { filter: f.into(), n: 1 }], false, false));
                    }
                    let ki = ctx.kicker_idx as usize;
                    let (kcost, multi, buyback) = kickers[ki.min(kickers.len() - 1)].clone();
                    // a kicker with no cost is how a gift is promised
                    let is_gift = kcost.is_empty() && !multi && !buyback;
                    let more = ki + 1 < kickers.len();
                    let x_first = self.modal_x_first(ctx.item.source);
                    let next = |c: &CastCtx| if more { Stage::Kicker } else if c.modal.is_some() && !(c.mana.has_x() && x_first) { Stage::Modes } else if c.mana.has_x() { Stage::X } else { Stage::Targets };
                    // what one more kick adds
                    let mut add_mana = ManaCost::default();
                    let mut add_extra: Vec<Cost> = vec![];
                    for part in &kcost {
                        match part {
                            Cost::Mana { cost } => {
                                if let Ok(m) = ManaCost::parse(cost) {
                                    add_mana.shards.extend(m.shards);
                                }
                            }
                            other => add_extra.push(other.clone()),
                        }
                    }
                    // "buyback costs cost {2} less" (Memory Crystal): only the generic part of the buyback cost shrinks
                    if buyback {
                        let mut less = 0i32;
                        for &src in &self.battlefield {
                            let sc = &self.cards[src.0 as usize];
                            if sc.cur.no_abilities {
                                continue;
                            }
                            let sd = self.db.def(sc.def);
                            for &i in &sd.statics {
                                if let Ability::Static { effect: StaticEffect::BuybackCostChange { generic }, condition, .. } = &sd.abilities[i as usize] {
                                    if condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(src, sc.cur.controller))) {
                                        less -= *generic;
                                    }
                                }
                            }
                        }
                        for sh in add_mana.shards.iter_mut() {
                            if let Shard::Generic { n } = sh {
                                let r = (*n as i32).min(less.max(0));
                                *n -= r as u8;
                                less -= r;
                            }
                        }
                        add_mana.shards.retain(|s| !matches!(s, Shard::Generic { n: 0 }));
                    }
                    let can_kick = |g: &Game, c: &CastCtx, k: u32| {
                        let mut m = c.mana.clone();
                        for _ in 0..k {
                            m.shards.extend(add_mana.shards.clone());
                        }
                        let mut extras: Vec<Cost> = (0..k).flat_map(|_| add_extra.iter().cloned()).collect();
                        // several kicks that each tap permanents of one kind need that many different permanents (Explosive Singularity)
                        if k > 1 && add_extra.len() == 1 {
                            if let Cost::TapPermanents { filter, n } = &add_extra[0] {
                                extras = vec![Cost::TapPermanents { filter: filter.clone(), n: *n * k as i32 }];
                            }
                            if let Cost::Sacrifice { filter, n } = &add_extra[0] {
                                extras = vec![Cost::Sacrifice { filter: filter.clone(), n: *n * k as i32 }];
                            }
                        }
                        // mana usable only on kicked spells (Throne of Makindi) counts once the kick is chosen
                        let afford = if k > 0 {
                            let mut g2 = g.clone();
                            g2.cards[c.item.source.0 as usize].kicked = true;
                            g2.cards[c.item.source.0 as usize].kicks = k.min(255) as u8;
                            g2.pay_for.set(Some(c.item.source));
                            // a "costs {N} less if kicked" reduction (Ice Out) counts toward what can be afforded
                            let (src, alt) = (c.item.source, g.cards[c.item.source.0 as usize].cast_alt);
                            let gen_of = |m: &ManaCost| m.shards.iter().map(|s| if let Shard::Generic { n } = s { *n as i32 } else { 0 }).sum::<i32>();
                            let e0 = g.effective_cost(p, src, alt);
                            let o0 = g.red_overflow.get() as i32;
                            let e1 = g2.effective_cost(p, src, alt);
                            let o1 = g2.red_overflow.get() as i32;
                            let mut left = gen_of(&e0) - gen_of(&e1) + o1.max(o1 - o0);
                            for sh in m.shards.iter_mut() {
                                if let (Shard::Generic { n }, true) = (sh, left > 0) {
                                    let r = (*n as i32).min(left);
                                    *n -= r as u8;
                                    left -= r;
                                }
                            }
                            g2.can_afford(p, &m, 0)
                        } else {
                            g.can_afford(p, &m, 0)
                        };
                        // a target that exists only when kicked ("if the gift was promised, return target creature card") must have a legal choice, or the kicked cast would be abandoned
                        let kicked_targets = k == 0 || c.specs.iter().filter(|s| s.when_kicked == Some(true) && s.min > 0).all(|s| {
                            *g.filter_event.borrow_mut() = Some(c.item.ctx.clone());
                            g.legal_targets(s, c.item.source, p).len() >= s.min as usize
                        });
                        afford && kicked_targets && (extras.is_empty() || g.cost_parts_payable(p, c.item.source, &extras))
                    };
                    let mut kicks: Option<u32> = None;
                    match answer.take() {
                        Some(Choice::YesNo { yes }) => kicks = Some(yes as u32),
                        Some(Choice::Number { n }) => kicks = Some(n.max(0) as u32),
                        _ => {}
                    }
                    if let Some(k) = kicks {
                        for _ in 0..k {
                            ctx.mana.shards.extend(add_mana.shards.clone());
                            ctx.extra.extend(add_extra.iter().cloned());
                        }
                        // cost changes that count the kicks ("costs {2} less for each card exiled this way") apply now that the count is known
                        let generic_of = |m: &ManaCost| m.shards.iter().map(|s| if let Shard::Generic { n } = s { *n as i32 } else { 0 }).sum::<i32>();
                        let (src, alt) = (ctx.item.source, self.cards[ctx.item.source.0 as usize].cast_alt);
                        let before = if k > 0 { let m = self.effective_cost(p, src, alt); Some((generic_of(&m), self.x_red.get(), self.red_overflow.get() as i32)) } else { None };
                        if k > 0 {
                            let c = &mut self.cards[ctx.item.source.0 as usize];
                            c.kicked = true;
                            c.kicks = (c.kicks as u32 + k).min(255) as u8;
                            c.kicked_mask |= 1 << ki.min(7);
                            if ki < 4 {
                                c.kick_by[ki] = (c.kick_by[ki] as u32 + k).min(255) as u8;
                            }
                            if off_idx == Some(ki) {
                                c.granted_offspring = true;
                            }
                            if add_extra.iter().any(|x| matches!(x, Cost::TapTotalPower { .. })) {
                                c.teamwork = true;
                            }
                        }
                        if let Some((g0, r0, o0)) = before {
                            let m = self.effective_cost(p, src, alt);
                            let (dg, dr) = (generic_of(&m) - g0 - (self.red_overflow.get() as i32 - o0), self.x_red.get() as i32 - r0 as i32);
                            if dg > 0 {
                                ctx.mana.add_generic(dg as u8);
                            } else if dg < 0 {
                                let mut left = -dg;
                                for sh in ctx.mana.shards.iter_mut() {
                                    if let Shard::Generic { n } = sh {
                                        let r = (*n as i32).min(left);
                                        *n -= r as u8;
                                        left -= r;
                                    }
                                }
                                ctx.mana.shards.retain(|s| !matches!(s, Shard::Generic { n: 0 }));
                            }
                            ctx.x_red = (ctx.x_red as i32 + dr).max(0) as u32;
                            // a cost reduction left over after the base cost (Rakdos, Lord of Riots) also reduces the kicker's generic mana
                            let mut left = self.red_overflow.get() as i32;
                            if k > 0 && left > 0 {
                                for sh in ctx.mana.shards.iter_mut() {
                                    if let Shard::Generic { n } = sh {
                                        let r = (*n as i32).min(left);
                                        *n -= r as u8;
                                        left -= r;
                                    }
                                }
                                ctx.mana.shards.retain(|s| !matches!(s, Shard::Generic { n: 0 }));
                                // what is still left reduces a kicker {X} (Conduit of Ruin on Thieving Skydiver)
                                if left > 0 && ctx.mana.has_x() {
                                    ctx.x_red += left as u32;
                                }
                            }
                        }
                        ctx.remaining = ctx.mana.clone();
                        ctx.kicker_idx += 1;
                        ctx.stage = next(&ctx);
                        if k > 0 && add_extra.iter().any(|x| matches!(x, Cost::Behold { choose_type: true, .. })) {
                            ctx.type_next = ctx.stage;
                            ctx.stage = Stage::BeholdType;
                        }
                        continue;
                    }
                    // the most kicks that can be paid (a kick that lowers the cost may only become affordable together with others: Explosive Singularity)
                    let max_kicks = if multi { (1..=20u32).rev().find(|&k| can_kick(self, &ctx, k)) } else { can_kick(self, &ctx, 1).then_some(1) };
                    let Some(max_kicks) = max_kicks else {
                        ctx.kicker_idx += 1;
                        ctx.stage = next(&ctx);
                        continue;
                    };
                    self.pending = Some(Decision {
                        player: p,
                        kind: if multi {
                            DecisionKind::Number { prompt: format!("How many times do you kick {}?", ctx.what), min: 0, max: max_kicks as i32, cancellable }
                        } else {
                            DecisionKind::YesNo { prompt: if is_gift { format!("Promise a gift for {}?", ctx.what) } else { format!("Pay the kicker for {}?", ctx.what) }, source: Some(ctx.item.source) }
                        },
                    });
                    self.run = Run::Cast(ctx);
                    return Flow::Wait;
                }
                Stage::Modes => {
                    let (min, max, n) = {
                        let stored = ctx.modal.unwrap();
                        // the maximum can depend on things settled since the cast began (a kicker paid: "if kicked, choose any number")
                        match (&ctx.origin, self.spell_specs(ctx.item.source).1) {
                            (Origin::Spell { .. }, Some((lo, hi, cnt))) if cnt == stored.2 => (lo, hi, cnt),
                            _ => stored,
                        }
                    };
                    let repeat = ctx.modal_repeat;
                    let x_first = self.modal_x_first(ctx.item.source);
                    let (min, max) = if x_first { { let k = ctx.item.x.clamp(0, 255) as u8; (k, k) } } else { (min, max) };
                    if x_first && ctx.item.x <= 0 && answer.is_none() {
                        answer = Some(Choice::Pick { indices: vec![] });
                    }
                    // the same mode can't be picked twice unless the spell allows it: such an answer is asked again
                    let all_modes = self.db.def(ctx.item.def).entwine && self.cards[ctx.item.source.0 as usize].kicked;
                    if all_modes {
                        answer = Some(Choice::Pick { indices: (0..n).collect() });
                    }
                    let total_paw = self.db.def(ctx.item.def).abilities.iter().find_map(|a| if let Ability::Modal { total_paw, .. } = a { Some(*total_paw) } else { None }).unwrap_or(0);
                    let paw_of = |g: &Self, i: usize| g.db.def(ctx.item.def).abilities.iter().find_map(|a| if let Ability::Modal { modes, .. } = a { modes.get(i).map(|m| m.paw as u32) } else { None }).unwrap_or(0);
                    let picked = answer.take().filter(|c| match c {
                        Choice::Pick { indices } if total_paw > 0 => indices.iter().map(|i| paw_of(self, *i)).sum::<u32>() == total_paw as u32,
                        Choice::Pick { indices } => repeat || indices.iter().enumerate().all(|(k, i)| !indices[..k].contains(i)),
                        _ => true,
                    });
                    if let Some(Choice::Pick { indices }) = picked {
                        ctx.item.modes = indices.iter().map(|i| *i as u8).collect();
                        ctx.item.modes.sort();
                        // choosing one or more modes is facing a dilemma (Star Trek)
                        if !ctx.item.modes.is_empty() {
                            let pl = ctx.item.controller;
                            self.players[pl as usize].dilemmas_turn += 1;
                            self.emit(Event::FacedDilemma { player: pl });
                        }
                        // spree: every chosen mode adds its own cost
                        {
                            let d = self.db.def(ctx.item.def);
                            for a in &d.abilities {
                                if let Ability::Modal { modes, .. } = a {
                                    for &m in &ctx.item.modes {
                                        for part in &modes[m as usize].cost {
                                            match part {
                                                Cost::Mana { cost } => {
                                                    if let Ok(mc) = ManaCost::parse(cost) {
                                                        ctx.mana.shards.extend(mc.shards);
                                                    }
                                                }
                                                other => ctx.extra.push(other.clone()),
                                            }
                                        }
                                    }
                                }
                            }
                            // escalate: an extra cost for every mode beyond the first
                            for a in &d.abilities {
                                if let Ability::Modal { escalate, .. } = a {
                                    for _ in 1..ctx.item.modes.len() {
                                        for part in escalate {
                                            match part {
                                                Cost::Mana { cost } => {
                                                    if let Ok(mc) = ManaCost::parse(cost) {
                                                        ctx.mana.shards.extend(mc.shards);
                                                    }
                                                }
                                                other => ctx.extra.push(other.clone()),
                                            }
                                        }
                                    }
                                }
                            }
                            // a cost reduction left over after the base cost (Goblin Electromancer on a {R} spell) also reduces the escalate cost
                            if ctx.item.modes.len() > 1 {
                                let (src, alt) = (ctx.item.source, self.cards[ctx.item.source.0 as usize].cast_alt);
                                let _ = self.effective_cost(ctx.player, src, alt);
                                let mut left = self.red_overflow.get() as i32;
                                if left > 0 {
                                    for sh in ctx.mana.shards.iter_mut() {
                                        if let Shard::Generic { n } = sh {
                                            let r = (*n as i32).min(left);
                                            *n -= r as u8;
                                            left -= r;
                                        }
                                    }
                                    ctx.mana.shards.retain(|s| !matches!(s, Shard::Generic { n: 0 }));
                                }
                            }
                            if !x_first {
                                ctx.remaining = ctx.mana.clone();
                            }
                        }
                        // targets of the chosen modes
                        if let Some((_, _, sm)) = self.stack_modal_of(&ctx) {
                            for &m in &ctx.item.modes {
                                ctx.specs.extend(sm[m as usize].specs.iter().cloned());
                            }
                        }
                        let d = self.db.def(ctx.item.def);
                        for a in &d.abilities {
                            if let Ability::Modal { modes, .. } = a {
                                for &m in &ctx.item.modes {
                                    ctx.specs.extend(modes[m as usize].targets.iter().cloned());
                                }
                            }
                        }
                        ctx.stage = if ctx.mana.has_x() && !x_first { Stage::X } else { Stage::Targets };
                        continue;
                    }
                    let d = self.db.def(ctx.item.def);
                    let mut options = vec![];
                    for a in &d.abilities {
                        if let Ability::Modal { modes, .. } = a {
                            for m in modes {
                                options.push(Opt::Text { text: m.text.clone() });
                            }
                        }
                    }
                    if let Some((_, _, sm)) = self.stack_modal_of(&ctx) {
                        options = sm.iter().map(|m| Opt::Text { text: m.text.clone() }).collect();
                    }
                    debug_assert_eq!(options.len(), n);
                    self.pending = Some(Decision {
                        player: p,
                        kind: DecisionKind::Pick {
                            prompt: format!("Choose mode for {}", ctx.what),
                            source: Some(ctx.item.source),
                            options,
                            min,
                            max,
                            cancellable,
                            hidden: false,
                            purpose: if repeat { PickPurpose::ModeRepeat } else { PickPurpose::Mode },
                        },
                    });
                    self.run = Run::Cast(ctx);
                    return Flow::Wait;
                }
                Stage::BeholdType => {
                    let (filter, n) = ctx
                        .extra
                        .iter()
                        .find_map(|x| if let Cost::Behold { filter, n, choose_type: true } = x { Some((filter.clone(), *n)) } else { None })
                        .unwrap_or(((Filter::default()).into(), 1));
                    let types = self.behold_types(&filter, ctx.item.source, p, n);
                    if let Some(Choice::Pick { indices }) = answer.take() {
                        if let Some(t) = indices.first().and_then(|i| types.get(*i)) {
                            self.cards[ctx.item.source.0 as usize].chosen_type = Some(Sym::new(t));
                        }
                        ctx.stage = ctx.type_next;
                        continue;
                    }
                    if types.len() <= 1 {
                        if let Some(t) = types.first() {
                            self.cards[ctx.item.source.0 as usize].chosen_type = Some(Sym::new(t));
                        }
                        ctx.stage = ctx.type_next;
                        continue;
                    }
                    self.pending = Some(Decision {
                        player: p,
                        kind: DecisionKind::Pick {
                            prompt: format!("{}: choose a creature type to behold", ctx.what),
                            source: Some(ctx.item.source),
                            options: types.iter().map(|t| Opt::Text { text: t.clone() }).collect(),
                            min: 1,
                            max: 1,
                            cancellable: false,
                            hidden: false,
                            purpose: PickPurpose::Generic,
                        },
                    });
                    self.run = Run::Cast(ctx);
                    return Flow::Wait;
                }
                Stage::CastType => {
                    let mut types: Vec<String> = vec![];
                    for d in &self.db.defs {
                        if d.types.has(CardType::Creature) || d.types.has(CardType::Kindred) {
                            for s in &d.subtypes {
                                let n = s.name();
                                if !types.contains(&n) {
                                    types.push(n);
                                }
                            }
                        }
                    }
                    types.sort();
                    if let Some(Choice::Pick { indices }) = answer.take() {
                        if let Some(t) = indices.first().and_then(|i| types.get(*i)) {
                            self.cards[ctx.item.source.0 as usize].chosen_type = Some(Sym::new(t));
                        }
                        ctx.stage = Stage::Targets;
                        continue;
                    }
                    self.pending = Some(Decision {
                        player: p,
                        kind: DecisionKind::Pick {
                            prompt: format!("{}: choose a creature type", ctx.what),
                            source: Some(ctx.item.source),
                            options: types.iter().map(|t| Opt::Text { text: t.clone() }).collect(),
                            min: 1,
                            max: 1,
                            cancellable: true,
                            hidden: false,
                            purpose: PickPurpose::Generic,
                        },
                    });
                    self.run = Run::Cast(ctx);
                    return Flow::Wait;
                }
                Stage::XColors => {
                    let n = ctx.item.x.max(0) as u32;
                    let prev = self.pay_for.replace(Some(ctx.item.source));
                    let valid: Vec<u8> = (0u8..32).filter(|m| m.count_ones() == n && self.can_afford(p, &Self::with_color_shards(&ctx.remaining, *m), 0)).collect();
                    self.pay_for.set(prev);
                    if valid.is_empty() {
                        return self.abort_cast(ctx);
                    }
                    let union = valid.iter().fold(0u8, |a, m| a | m);
                    let colors: Vec<Color> = Color::ALL.iter().copied().filter(|c| union & c.bit() != 0).collect();
                    let mut chosen: Option<u8> = if valid.len() == 1 { Some(valid[0]) } else { None };
                    if chosen.is_none() {
                        if let Some(Choice::Pick { indices }) = answer.take() {
                            let mask = indices.iter().filter_map(|i| colors.get(*i)).fold(0u8, |a, c| a | c.bit());
                            if valid.contains(&mask) {
                                chosen = Some(mask);
                            }
                        }
                    }
                    if let Some(mask) = chosen {
                        ctx.remaining = Self::with_color_shards(&ctx.remaining, mask);
                        ctx.stage = ctx.type_next;
                        continue;
                    }
                    self.pending = Some(Decision {
                        player: p,
                        kind: DecisionKind::Pick {
                            prompt: format!("{}: choose {} different colors of mana to pay X with", ctx.what, n),
                            source: Some(ctx.item.source),
                            options: colors.iter().map(|c| Opt::Color { c: *c }).collect(),
                            min: n as u8,
                            max: n as u8,
                            cancellable: true,
                            hidden: false,
                            purpose: PickPurpose::Generic,
                        },
                    });
                    self.run = Run::Cast(ctx);
                    return Flow::Wait;
                }
                Stage::Frenzy => {
                    let src = ctx.item.source;
                    let next = if self.optional_spell_costs(p, src).is_empty() { ctx.opt_next } else { Stage::OptCost };
                    let Some((kind, per)) = self.frenzy_params(src) else { ctx.stage = next; continue };
                    // any number of counters may be removed (more than the cost can use is allowed; the reduction stops at the generic part)
                    let max = if self.frenzy_max(p, src, &ctx.mana) > 0 { self.battlefield.iter().filter(|&&id| { let c = &self.cards[id.0 as usize]; c.cur.controller == p && c.cur.types.has(CardType::Creature) }).map(|&id| self.cards[id.0 as usize].counter(kind)).sum::<i32>() } else { 0 };
                    if let Some(Choice::Number { n }) = answer.take() {
                        let n = n.clamp(0, max);
                        let mut left = n;
                        let holders: Vec<CardId> = self.battlefield.iter().copied().filter(|&id| { let c = &self.cards[id.0 as usize]; c.cur.controller == p && c.cur.types.has(CardType::Creature) && c.counter(kind) > 0 }).collect();
                        let mut holders = holders;
                        holders.sort_by_key(|id| -self.cards[id.0 as usize].counter(kind));
                        for h in holders {
                            if left <= 0 {
                                break;
                            }
                            let take = left.min(self.cards[h.0 as usize].counter(kind));
                            self.remove_counters_from(Obj::Card { id: h }, kind, take);
                            left -= take;
                        }
                        let mut cut = n * per;
                        for sh in ctx.mana.shards.iter_mut() {
                            if let Shard::Generic { n: gn } = sh {
                                let r = (*gn as i32).min(cut);
                                *gn -= r as u8;
                                cut -= r;
                            }
                        }
                        ctx.mana.shards.retain(|s| !matches!(s, Shard::Generic { n: 0 }));
                        ctx.remaining = ctx.mana.clone();
                        ctx.stage = next;
                        continue;
                    }
                    if max <= 0 {
                        ctx.stage = next;
                        continue;
                    }
                    self.pending = Some(Decision {
                        player: p,
                        kind: DecisionKind::Number { prompt: format!("Remove how many counters from your creatures to cast {} for less?", ctx.what), min: 0, max, cancellable: false },
                    });
                    self.run = Run::Cast(ctx);
                    return Flow::Wait;
                }
                Stage::Num => {
                    let (mn, mx) = self.def_of(ctx.item.source).abilities.iter().find_map(|a| if let Ability::ChooseNumberOnCast { min, max } = a { Some((*min, *max)) } else { None }).unwrap_or((0, 0));
                    if let Some(Choice::Number { n }) = answer.take() {
                        self.cards[ctx.item.source.0 as usize].chosen_number = Some(n.clamp(mn, mx));
                        ctx.stage = Stage::Targets;
                        continue;
                    }
                    self.pending = Some(Decision {
                        player: p,
                        kind: DecisionKind::Number { prompt: format!("Choose a number for {}", ctx.what), min: mn, max: mx, cancellable: true },
                    });
                    self.run = Run::Cast(ctx);
                    return Flow::Wait;
                }
                Stage::X => {
                    // an {X} defined by the card's text ("X is the number of ... as you cast this spell") is paid, not chosen
                    if answer.is_none() && ctx.mana.has_x() && self.def_of(ctx.item.source).abilities.iter().any(|a| matches!(a, Ability::XFromAmount { .. })) {
                        if ctx.item.x > self.max_x(p, &ctx.mana, ctx.item.source, ctx.x_red) {
                            return self.abort_cast(ctx);
                        }
                        answer = Some(Choice::Number { n: ctx.item.x });
                    }
                    if let Some(Choice::Number { n }) = answer.take() {
                        ctx.item.x = n;
                        // targets that depend on X (Spell Blast) are judged with the chosen value
                        self.cards[ctx.item.source.0 as usize].x = n;
                        // "costs {2} less for each X" with an any-number additional cost (Gorex, Torgaar): the count is known now, so work the cost out again
                        if matches!(ctx.origin, Origin::Spell { .. })
                            && !ctx.mana.shards.is_empty()
                            && ctx.extra.iter().any(|c| matches!(c, Cost::XTimes { .. }))
                            && self.def_of(ctx.item.source).statics.iter().any(|&i| matches!(&self.def_of(ctx.item.source).abilities[i as usize], Ability::Static { effect: StaticEffect::CostChange { filter, per: Some(Amount::X), .. }, .. } if filter.this))
                        {
                            let alt = match ctx.item.kind {
                                ItemKind::Spell { alt, .. } => alt,
                                _ => None,
                            };
                            ctx.mana = self.effective_cost(p, ctx.item.source, alt);
                        }
                        for s in ctx.mana.shards.clone().iter() {
                            if matches!(s, Shard::X) {
                                // X shards count n generic mana each
                            }
                        }
                        ctx.remaining = ctx.mana.clone();
                        let x = n.max(0) as u32;
                        ctx.remaining.shards.retain(|s| !matches!(s, Shard::X));
                        ctx.remaining.add_generic(0);
                        let xs = ctx.mana.shards.iter().filter(|s| matches!(s, Shard::X)).count() as u32;
                        let distinct = x > 0 && self.def_of(ctx.item.source).abilities.iter().any(|a| matches!(a, Ability::XColoredDistinct));
                        if distinct {
                            // paid with colored mana, one of each color: chosen in the next stage
                        } else if x * xs > 0 {
                            let mut total = (x * xs).saturating_sub(ctx.x_red);
                            let only = self.db.def(self.cards[ctx.item.source.0 as usize].def).abilities.iter().find_map(|a| if let Ability::XOnlyColor { color, or_color } = a { Some((*color, *or_color)) } else { None });
                            while total > 0 {
                                if let Some((c, oc)) = only {
                                    ctx.remaining.shards.push(match oc {
                                        Some(b) => Shard::Hybrid { a: c, b },
                                        None => Shard::Color { c },
                                    });
                                    total -= 1;
                                    continue;
                                }
                                let k = total.min(200) as u8;
                                ctx.remaining.add_generic(k);
                                total -= k as u32;
                            }
                        }
                        // "discard X cards": expand the X-times costs now that X is known
                        let mut expanded: Vec<Cost> = vec![];
                        for c in ctx.extra.drain(..) {
                            match c {
                                Cost::XTimes { cost } => {
                                    if n > 0 {
                                        expanded.push(Self::scale_cost(&cost, n));
                                    }
                                }
                                other => expanded.push(other),
                            }
                        }
                        ctx.extra = expanded;
                        ctx.stage = if ctx.modal.is_some() && ctx.item.modes.is_empty() && self.modal_x_first(ctx.item.source) { Stage::Modes } else { Stage::Targets };
                        if distinct {
                            ctx.type_next = ctx.stage;
                            ctx.stage = Stage::XColors;
                        }
                        continue;
                    }
                    let x_times_cost = ctx.extra.iter().any(|c| matches!(c, Cost::XTimes { .. }));
                    let mut max = if ctx.mana.has_x() {
                        // harmonize: the tapped creature's power will pay down X, so allow for the strongest one
                        let tap_help = ctx.extra.iter().filter_map(|c| if let Cost::TapReduceByPower { filter } = c { Some(filter) } else { None }).next()
                            .map(|f| self.tappable_for_cost(p, ctx.item.source, f).iter().map(|id| self.cards[id.0 as usize].cur.power.max(0) as u32).max().unwrap_or(0))
                            .unwrap_or(0);
                        let is_ab = matches!(ctx.item.kind, ItemKind::Ability { .. });
                        let (pa, ph) = (self.pay_is_ability.replace(is_ab), self.pay_ability_has_x.replace(is_ab));
                        let mx = self.max_x(p, &ctx.mana, ctx.item.source, ctx.x_red + tap_help);
                        self.pay_is_ability.set(pa);
                        self.pay_ability_has_x.set(ph);
                        mx
                    } else if x_times_cost {
                        i32::MAX
                    } else if let (Origin::Spell { .. }, Some(f)) = (&ctx.origin, ctx.extra.iter().find_map(|c| if let Cost::Sacrifice { filter, .. } = c { if filter.mana_value_vs.as_ref().map_or(false, |ac| matches!(*ac.amount, Amount::X)) { Some(filter.clone()) } else { None } } else { None })) {
                        // X is the mana value of a permanent sacrificed as an additional cost: at most the biggest eligible one
                        let f = crate::pay::strip_x_filter(&self.sac_cost_filter(&f));
                        self.battlefield.iter().filter(|id| self.cards[id.0 as usize].cur.controller == p && self.matches_card(&f, **id, ctx.item.source, p)).map(|id| self.def_of(*id).mana_value as i32).max().unwrap_or(0)
                    } else if matches!(ctx.origin, Origin::Spell { .. }) {
                        // X comes from the mana value of a card exiled from hand: at most the biggest card there
                        self.players[p as usize].hand.iter().filter(|h| **h != ctx.item.source).map(|h| self.db.def(self.cards[h.0 as usize].def).mana_value as i32).max().unwrap_or(0)
                    } else {
                        i32::MAX
                    };
                    for c in ctx.extra.iter() {
                        if let Cost::XTimes { cost } = c {
                            let src = ctx.item.source;
                            let prev = self.pay_for.replace(if matches!(ctx.origin, Origin::Spell { .. }) { Some(src) } else { None });
                            let mut top = 0;
                            for k in 1..=40 {
                                if self.simple_cost_ok(p, src, &Self::scale_cost(cost, k)) { top = k } else { break }
                            }
                            self.pay_for.set(prev);
                            max = max.min(top);
                        }
                    }
                    if let Some(cap) = self.def_of(ctx.item.source).abilities.iter().find_map(|a| if let Ability::XCap { amount } = a { Some(amount.clone()) } else { None }) {
                        let cv = self.eval_amount(&cap, &EvalCtx::with_controller(ctx.item.source, p));
                        max = max.min(cv.max(0));
                    }
                    if ctx.extra.iter().any(|c| matches!(c, Cost::RemoveLoyaltyX)) {
                        max = max.min(self.cards[ctx.item.source.0 as usize].counter(CounterKind::Loyalty));
                    }
                    if let Some(k) = ctx.extra.iter().find_map(|c| if let Cost::RemoveCountersX { kind } = c { Some(*kind) } else { None }) {
                        max = max.min(self.cards[ctx.item.source.0 as usize].counter(k));
                    }
                    if let Some(f) = ctx.extra.iter().find_map(|c| if let Cost::ExileTotalMvX { filter } = c { Some(filter.clone()) } else { None }) {
                        let total: i32 = self.exile_mv_pool(&f, p, ctx.item.source).iter().map(|id| self.def_of(*id).mana_value as i32).sum();
                        max = max.min(total);
                    }
                    if let Some(f) = ctx.extra.iter().find_map(|c| if let Cost::RemoveXCountersFromOne { filter } = c { Some(filter.clone()) } else { None }) {
                        let mut f = f;
                        f.controller = PlayerFilter::Any;
                        let most = self.battlefield.iter().filter(|id| self.cards[id.0 as usize].cur.controller == p && self.matches_card(&f, **id, ctx.item.source, p)).map(|id| self.cards[id.0 as usize].counters.iter().map(|(_, x)| *x as i32).sum::<i32>()).max().unwrap_or(0);
                        max = max.min(most);
                    }
                    // a kicker {X} on a spell with no X of its own can't be 0 (Thieving Skydiver)
                    let min_x = if self.cards[ctx.item.source.0 as usize].kicked && !self.def_of(ctx.item.source).cost.has_x() { 1.min(max) } else { 0 };
                    let marker_min = ctx.extra.iter().filter_map(|c| if let Cost::XMinimum { n } = c { Some(*n as i32) } else { None }).max().unwrap_or(0);
                    if marker_min > 0 && max < marker_min {
                        // X must be at least the marker's minimum, but that can't be afforded
                        return self.abort_cast(ctx);
                    }
                    let min_x = if marker_min > 0 { marker_min.min(max) } else { min_x };
                    let min_x = min_x.max(Self::cast_x_floor(self.def_of(ctx.item.source)).min(max));
self.pending = Some(Decision {
                        player: p,
                        kind: DecisionKind::Number { prompt: format!("Choose X for {}", ctx.what), min: min_x, max, cancellable },
                    });
                    self.run = Run::Cast(ctx);
                    return Flow::Wait;
                }
                Stage::Targets => {
                    // the answer to "which opponent chooses this target"
                    if ctx.asking_chooser {
                        ctx.asking_chooser = false;
                        if let (Some(Choice::Pick { indices }), Some(spec)) = (answer.take(), ctx.specs.get(ctx.spec_idx)) {
                            let opts = self.target_chooser_options(&ctx, spec);
                            if let Some(q) = indices.first().and_then(|i| opts.get(*i)).or(opts.first()).copied() {
                                ctx.target_chooser = Some((ctx.spec_idx, q));
                            }
                        }
                    }
                    if ctx.spec_idx >= ctx.specs.len() {
                        // damage divided as the spell is cast (Conflagrate): one number per target but the last, which gets the rest
                        // the alternative cost being cast with and the triggering event are known now (Avacyn's Judgment madness X, Polukranos's monstrosity X)
                        let alt_now = if let Origin::Spell { alt, .. } = ctx.origin { Some(alt) } else { None };
                        let (old_probe, old_for) = (self.pay_alt_probe.get(), self.pay_for.get());
                        if let Some(a) = alt_now {
                            self.pay_alt_probe.set(Some(a));
                            self.pay_for.set(Some(ctx.item.source));
                        }
                        let at_cast_amt = {
                            let mut ec = EvalCtx::with_controller(ctx.item.source, p);
                            ec.x = ctx.item.x;
                            ec.event = &ctx.item.ctx;
                            let fm = |effects: &[Effect]| find_at_cast_division(self, effects, &ec);
                            if matches!(ctx.origin, Origin::Spell { .. }) {
                                self.def_of(ctx.item.source).abilities.iter().find_map(|a| match a {
                                    Ability::Spell { effects, .. } => fm(effects),
                                    Ability::Modal { modes, .. } => ctx.item.modes.iter().find_map(|m| modes.get(*m as usize).and_then(|md| fm(&md.effects))),
                                    _ => None,
                                })
                            } else if let ItemKind::Ability { ability, .. } = &ctx.item.kind {
                                // an activated ability that divides damage as it is activated (Mogg Mob)
                                match self.def_of(ctx.item.source).abilities.get(*ability as usize) {
                                    Some(Ability::Activated { effects, .. }) => fm(effects),
                                    _ => None,
                                }
                            } else if let (Origin::Trigger, ItemKind::Triggered { ability, .. }) = (&ctx.origin, &ctx.item.kind) {
                                // a triggered ability that divides as it is put on the stack (Quirion Beastcaller, Kuldotha Flamefiend)
                                let owned = if *ability < GRANTED_BASE { self.def_of(ctx.item.source).abilities.get(*ability as usize).cloned() } else { None };
                                match owned { Some(Ability::Triggered { effects, .. }) => fm(&effects), _ => None }
                            } else { None }
                        };
                        if alt_now.is_some() {
                            self.pay_alt_probe.set(old_probe);
                            self.pay_for.set(old_for);
                        }
                        if let Some((amt, slot)) = at_cast_amt {
                            // only the targets of the dividing slot share the amount (Fiery Justice: not its life-gain opponent)
                            let nt = match slot {
                                Some(sl) => ctx.item.spec_counts.get(sl as usize).copied().unwrap_or(0) as usize,
                                None => ctx.item.targets.len(),
                            };
                            let total = {
                                let mut ec = EvalCtx::with_controller(ctx.item.source, p);
                                ec.x = ctx.item.x;
                                ec.event = &ctx.item.ctx;
                                self.eval_amount(&amt, &ec)
                            };
                            while ctx.item.divisions.len() < nt {
                                let idx = ctx.item.divisions.len();
                                let left = total - ctx.item.divisions.iter().sum::<i32>();
                                let rest = (nt - idx - 1) as i32;
                                let maxv = left - rest;
                                if rest == 0 {
                                    ctx.item.divisions.push(left.max(0));
                                } else if maxv < 1 {
                                    ctx.item.divisions.push(left.clamp(0, 1));
                                } else if let Some(Choice::Number { n }) = answer.take() {
                                    ctx.item.divisions.push(n.clamp(1, maxv));
                                } else {
                                    self.pending = Some(Decision {
                                        player: p,
                                        kind: DecisionKind::Number { prompt: format!("{}: damage to target {} of {}", ctx.what, idx + 1, nt), min: 1, max: maxv, cancellable: true },
                                    });
                                    self.run = Run::Cast(ctx);
                                    return Flow::Wait;
                                }
                            }
                        }
                        // a cost reduction that depends on the chosen targets is settled now
                        if let (Origin::Spell { alt, .. }, false) = (ctx.origin.clone(), ctx.item.targets.is_empty() || ctx.mana.shards.is_empty()) {
                            let mut tidx: Vec<u8> = vec![];
                            let mut at = 0u8;
                            for cnt in ctx.item.spec_counts.iter() {
                                tidx.push(if *cnt == 0 { 255 } else { at });
                                at = at.saturating_add(*cnt);
                            }
                            let fresh = self.effective_cost_t(p, ctx.item.source, alt, Some((&ctx.item.targets, &tidx)));
                            let (g_old, g_new) = (Self::generic_of(&ctx.mana), Self::generic_of(&fresh));
                            if g_new > g_old {
                                let d = (g_new - g_old).min(255) as u8;
                                ctx.mana.add_generic(d);
                                ctx.remaining.add_generic(d);
                            }
                        }
                        // an activated ability's cost change that depends on its chosen target ("Equip abilities that target this creature cost {2} less")
                        if let Origin::Ability { card: acard, ability: aidx } = ctx.origin.clone() {
                            let first = ctx.item.targets.first().and_then(|t| if let Obj::Card { id } = t.obj { Some(id) } else { None });
                            let equip = self.ability_owned(acard, aidx).as_ref().map_or(false, |a| self.is_equip_for(acard, a));
                            let d = self.target_ability_cost_delta(p, acard, equip, first);
                            if d < 0 {
                                for m in [&mut ctx.mana, &mut ctx.remaining] {
                                    let mut reduce = -d;
                                    for sh in m.shards.iter_mut() {
                                        if let Shard::Generic { n } = sh {
                                            let r = (*n as i32).min(reduce);
                                            *n -= r as u8;
                                            reduce -= r;
                                        }
                                    }
                                    m.shards.retain(|s| !matches!(s, Shard::Generic { n: 0 }));
                                }
                            } else if d > 0 {
                                ctx.mana.add_generic(d.min(250) as u8);
                                ctx.remaining.add_generic(d.min(250) as u8);
                            }
                        }
                        // generic mana sized by an Amount that reads the chosen targets ("{X}, {T}: ... X is twice the mana value of that spell", Reflecting Mirror)
                        if matches!(ctx.origin, Origin::Ability { .. }) && ctx.extra.iter().any(|c| matches!(c, Cost::ManaAmount { .. })) {
                            let mut tidx: Vec<u8> = vec![];
                            let mut at = 0u8;
                            for cnt in ctx.item.spec_counts.iter() {
                                tidx.push(if *cnt == 0 { 255 } else { at });
                                at = at.saturating_add(*cnt);
                            }
                            let mut total = 0;
                            for c in ctx.extra.iter() {
                                if let Cost::ManaAmount { amount } = c {
                                    let mut ec: EvalCtx<'_> = EvalCtx::with_controller(ctx.item.source, p);
                                    ec.targets = &ctx.item.targets;
                                    ec.tidx = &tidx;
                                    total += self.eval_amount(amount, &ec).max(0);
                                }
                            }
                            ctx.extra.retain(|c| !matches!(c, Cost::ManaAmount { .. }));
                            ctx.mana.add_generic(total.min(250) as u8);
                            ctx.remaining.add_generic(total.min(250) as u8);
                        }
                        // life paid for each chosen target
                        if let Origin::Spell { .. } = ctx.origin {
                            let per: i32 = self.def_of(ctx.item.source).abilities.iter().find_map(|a| if let Ability::LifePerTarget { n } = a { Some(*n) } else { None }).unwrap_or(0);
                            if per > 0 && !ctx.item.targets.is_empty() {
                                ctx.extra.push(Cost::PayLife { n: per * ctx.item.targets.len() as i32 });
                            }
                        }
                        // strive: more mana for each target beyond the first
                        if let Origin::Spell { .. } = ctx.origin {
                            let strive: Option<String> = self.def_of(ctx.item.source).abilities.iter().find_map(|a| if let Ability::Strive { cost } = a { Some(cost.clone()) } else { None });
                            if let Some(c) = strive {
                                if let Ok(unit) = ManaCost::parse(&c) {
                                    let extra_targets = ctx.item.targets.len().saturating_sub(1);
                                    for _ in 0..extra_targets {
                                        ctx.mana.shards.extend(unit.shards.iter().cloned());
                                        ctx.remaining.shards.extend(unit.shards.iter().cloned());
                                    }
                                }
                            }
                        }
                        ctx.extra_late = ctx.extra.iter().any(Self::gives_up_permanent);
                        ctx.stage = if ctx.extra_late { Stage::Alt } else { Stage::Extra };
                        continue;
                    }
                    let spec = ctx.specs[ctx.spec_idx].clone();
                    if let Some(want) = spec.when_kicked {
                        if self.cards[ctx.item.source.0 as usize].kicked != want {
                            ctx.item.spec_counts.push(0);
                            ctx.spec_idx += 1;
                            continue;
                        }
                    }
                    *self.filter_event.borrow_mut() = Some(ctx.item.ctx.clone());
                    // a later target slot may refer to the earlier ones ("target creature they control")
                    let mut tidx_so_far: Vec<u8> = vec![];
                    let mut off = 0usize;
                    for &c in ctx.item.spec_counts.iter() {
                        tidx_so_far.push(if c == 0 { 255 } else { off as u8 });
                        off += c as usize;
                    }
                    *self.filter_targets.borrow_mut() = (ctx.item.targets.to_vec(), tidx_so_far.clone());
                    self.target_trig.set(matches!(ctx.item.kind, ItemKind::Triggered { .. } | ItemKind::Delayed { .. }));
                    let mut legal = self.legal_targets(&spec, ctx.item.source, p);
                    self.target_trig.set(false);
                    if ctx.spec_idx == 0 {
                        if let Some(tf) = self.flash_target_limit(p, ctx.item.source) {
                            legal.retain(|o| matches!(o, Obj::Card { id } if self.matches_card(&tf, *id, ctx.item.source, p)));
                        }
                    }
                    *self.filter_targets.borrow_mut() = (vec![], vec![]);
                    if spec.distinct {
                        legal.retain(|o| !ctx.item.targets.iter().any(|t| t.obj == *o));
                    }
                    if spec.distinct_by == Some(ChooseDistinct::Controller) {
                        // controllers must also differ from those of the targets chosen for earlier slots
                        let taken: Vec<PlayerId> = ctx.item.targets.iter().filter_map(|t| if let Obj::Card { id } = t.obj { Some(self.cards[id.0 as usize].cur.controller) } else { None }).collect();
                        legal.retain(|o| !matches!(o, Obj::Card { id } if taken.contains(&self.cards[id.0 as usize].cur.controller)));
                    }
                    *self.filter_event.borrow_mut() = None;
                    if let Some(Choice::Pick { indices }) = answer.take() {
                        // a total-mana-value budget: keep targets in the order picked while they fit
                        let indices: Vec<usize> = if let Some(a) = &spec.max_total_mv {
                            let evt = ctx.item.ctx.clone();
                            let mut ec = EvalCtx::with_controller(ctx.item.source, p);
                            ec.event = &evt;
                            ec.x = ctx.item.x;
                            let budget = self.eval_amount(a, &ec);
                            let mut used = 0;
                            let mut keep = vec![];
                            for i in indices {
                                let mv = legal.get(i).and_then(|o| if let Obj::Card { id } = o { Some(self.def_of(*id).mana_value as i32) } else { None }).unwrap_or(0);
                                if used + mv <= budget {
                                    used += mv;
                                    keep.push(i);
                                }
                            }
                            keep
                        } else {
                            indices
                        };
                        let indices: Vec<usize> = if let Some(a) = &spec.max_total_power {
                            let evt = ctx.item.ctx.clone();
                            let mut ec = EvalCtx::with_controller(ctx.item.source, p);
                            ec.event = &evt;
                            ec.x = ctx.item.x;
                            let budget = self.eval_amount(a, &ec);
                            let mut used = 0;
                            let mut keep = vec![];
                            for i in indices {
                                let pw = legal.get(i).and_then(|o| if let Obj::Card { id } = o { Some(self.cards[id.0 as usize].cur.power.max(0)) } else { None }).unwrap_or(0);
                                if used + pw <= budget {
                                    used += pw;
                                    keep.push(i);
                                }
                            }
                            keep
                        } else {
                            indices
                        };
                        // every card from one graveyard: keep the picks owned by the first pick's owner
                        let indices: Vec<usize> = if spec.same_graveyard {
                            let owner = indices.first().and_then(|i| legal.get(*i)).and_then(|o| if let Obj::Card { id } = o { Some(self.cards[id.0 as usize].owner) } else { None });
                            indices.into_iter().filter(|i| legal.get(*i).map_or(false, |o| matches!(o, Obj::Card { id } if Some(self.cards[id.0 as usize].owner) == owner))).collect()
                        } else {
                            indices
                        };
                        // equal toughness: keep the picks whose toughness matches the first pick's
                        let indices: Vec<usize> = if spec.distinct_by == Some(ChooseDistinct::SameToughness) {
                            let first_t = indices.first().and_then(|i| legal.get(*i)).and_then(|o| if let Obj::Card { id } = o { Some(self.cards[id.0 as usize].cur.toughness) } else { None });
                            indices.into_iter().filter(|i| matches!(legal.get(*i), Some(Obj::Card { id }) if Some(self.cards[id.0 as usize].cur.toughness) == first_t)).collect()
                        } else {
                            indices
                        };
                        let indices: Vec<usize> = if spec.distinct_by == Some(ChooseDistinct::SharedCreatureType) {
                            let mut kept: Vec<CardId> = vec![];
                            indices
                                .into_iter()
                                .filter(|i| match legal.get(*i) {
                                    Some(Obj::Card { id }) => {
                                        kept.push(*id);
                                        let ok = self.max_shared_creature_type(&kept) == kept.len();
                                        if !ok {
                                            kept.pop();
                                        }
                                        ok
                                    }
                                    _ => true,
                                })
                                .collect()
                        } else if let Some(by) = spec.distinct_by {
                            let mut seen: Vec<String> = vec![];
                            indices
                                .into_iter()
                                .filter(|i| {
                                    let key = match legal.get(*i) {
                                        Some(Obj::Card { id }) => match by {
                                            ChooseDistinct::Name => self.def_of(*id).name.clone(),
                                            ChooseDistinct::Power => self.cards[id.0 as usize].cur.power.to_string(),
                                            ChooseDistinct::ManaValue => self.def_of(*id).mana_value.to_string(),
                                            ChooseDistinct::Controller => self.cards[id.0 as usize].cur.controller.to_string(),
                                            ChooseDistinct::SharedCreatureType | ChooseDistinct::SameToughness => return true,
                                        },
                                        _ => return true,
                                    };
                                    if seen.contains(&key) {
                                        false
                                    } else {
                                        seen.push(key);
                                        true
                                    }
                                })
                                .collect()
                        } else {
                            indices
                        };
                        for i in &indices {
                            if let Some(o) = legal.get(*i) {
                                let zcc = self.obj_zcc(o);
                                ctx.item.targets.push(Target { obj: *o, zcc });
                            }
                        }
                        ctx.item.spec_counts.push(indices.len() as u8);
                        ctx.spec_idx += 1;
                        continue;
                    }
                    // `exact_x` with `max_amount` (no `max_x`): exactly that many targets (Strength of the Tajuru: KickCount + 1)
                    let exact_amt: Option<usize> = match (&spec.max_amount, spec.exact_x, spec.max_x) {
                        (Some(a), true, false) => {
                            let evt = ctx.item.ctx.clone();
                            let mut ec = EvalCtx::with_controller(ctx.item.source, p);
                            ec.event = &evt;
                            ec.x = ctx.item.x;
                            Some(self.eval_amount(a, &ec).max(0) as usize)
                        }
                        _ => None,
                    };
                    if spec.max_x && spec.exact_x && ctx.item.x <= 0 {
                        ctx.item.spec_counts.push(0);
                        ctx.spec_idx += 1;
                        continue;
                    }
                    let min = if let Some(n) = exact_amt { n } else if spec.max_x && spec.exact_x { ctx.item.x as usize } else { spec.min as usize };
                    if spec.distinct_by == Some(ChooseDistinct::Controller) {
                        // enough different controllers among the legal permanents (greedy keep-first per controller)
                        let mut seen: Vec<PlayerId> = vec![];
                        let n = legal.iter().filter(|o| match o { Obj::Card { id } => { let c = self.cards[id.0 as usize].cur.controller; if seen.contains(&c) { false } else { seen.push(c); true } } _ => true }).count();
                        if n < min {
                            return self.abort_cast(ctx);
                        }
                    }
                    if legal.len() < min {
                        // can't be cast / put on the stack: abandon
                        return self.abort_cast(ctx);
                    }
                    if spec.distinct_by == Some(ChooseDistinct::SharedCreatureType) && min > 0 {
                        // some creature type must be shared by enough of the legal creatures
                        let ids: Vec<CardId> = legal.iter().filter_map(|o| if let Obj::Card { id } = o { Some(*id) } else { None }).collect();
                        if self.max_shared_creature_type(&ids) < min {
                            return self.abort_cast(ctx);
                        }
                    }
                    if spec.random && min > 0 && !legal.is_empty() {
                        // chosen at random as it is put on the stack
                        if let Some(o) = rand::seq::SliceRandom::choose(&legal[..], &mut self.rng).copied() {
                            let zcc = self.obj_zcc(&o);
                            ctx.item.targets.push(Target { obj: o, zcc });
                            ctx.item.spec_counts.push(1);
                            ctx.spec_idx += 1;
                            continue;
                        }
                    }
                    if legal.len() <= min && (spec.max as usize == min || exact_amt.is_some() || (spec.max_x && spec.exact_x)) || (matches!(ctx.origin, Origin::Trigger) && legal.len() == 1 && min == 1) {
                        // forced choice
                        let mut forced: Vec<Obj> = legal.iter().take(min.max(1).min(legal.len())).copied().collect();
                        if spec.distinct_by == Some(ChooseDistinct::Controller) {
                            let mut seen: Vec<PlayerId> = vec![];
                            forced.retain(|o| match o { Obj::Card { id } => { let c = self.cards[id.0 as usize].cur.controller; if seen.contains(&c) { false } else { seen.push(c); true } } _ => true });
                        }
                        if spec.distinct_by == Some(ChooseDistinct::SharedCreatureType) {
                            let mut kept: Vec<CardId> = vec![];
                            forced.retain(|o| match o {
                                Obj::Card { id } => {
                                    kept.push(*id);
                                    let ok = self.max_shared_creature_type(&kept) == kept.len();
                                    if !ok {
                                        kept.pop();
                                    }
                                    ok
                                }
                                _ => true,
                            });
                        }
                        if spec.same_graveyard {
                            let owner = forced.first().and_then(|o| if let Obj::Card { id } = o { Some(self.cards[id.0 as usize].owner) } else { None });
                            forced.retain(|o| matches!(o, Obj::Card { id } if Some(self.cards[id.0 as usize].owner) == owner));
                        }
                        for o in forced.iter() {
                            let zcc = self.obj_zcc(o);
                            ctx.item.targets.push(Target { obj: *o, zcc });
                        }
                        ctx.item.spec_counts.push(forced.len() as u8);
                        ctx.spec_idx += 1;
                        continue;
                    }
                    if legal.is_empty() {
                        ctx.item.spec_counts.push(0);
                        ctx.spec_idx += 1;
                        continue;
                    }
                    let spec_max = if let Some(n) = exact_amt {
                        n
                    } else if let Some(a) = &spec.max_amount {
                        let evt = ctx.item.ctx.clone();
                        let mut ec = EvalCtx::with_controller(ctx.item.source, p);
                        ec.event = &evt;
                        ec.x = ctx.item.x;
                        // an earlier target of this spell may be read (Reap: "target opponent" decides X)
                        let mut tidx: Vec<u8> = vec![];
                        let mut pos = 0usize;
                        for c in ctx.item.spec_counts.iter() {
                            tidx.push(if *c == 0 { 255 } else { pos as u8 });
                            pos += *c as usize;
                        }
                        ec.targets = &ctx.item.targets;
                        ec.tidx = &tidx;
                        self.eval_amount(a, &ec).max(0) as usize
                    } else if spec.max_x {
                        ctx.item.x.max(0) as usize
                    } else {
                        spec.max as usize
                    };
                    let mut max = spec_max.min(legal.len()) as u8;
                    // life per target: you can't choose more targets than you can pay life for
                    if let Origin::Spell { .. } = ctx.origin {
                        let per: i32 = self.def_of(ctx.item.source).abilities.iter().find_map(|a| if let Ability::LifePerTarget { n } = a { Some(*n) } else { None }).unwrap_or(0);
                        if per > 0 {
                            let mut k = 0u8;
                            while k < max && self.can_pay_life(p, per * (k as i32 + 1)) {
                                k += 1;
                            }
                            max = k;
                        }
                    }
                    // strive: you can't choose more targets than you can pay for
                    if let Origin::Spell { .. } = ctx.origin {
                        let strive: Option<String> = self.def_of(ctx.item.source).abilities.iter().find_map(|a| if let Ability::Strive { cost } = a { Some(cost.clone()) } else { None });
                        if let Some(Ok(unit)) = strive.map(|c| ManaCost::parse(&c)) {
                            let mut k = 0u8;
                            let mut total = ctx.remaining.clone();
                            // restricted mana (Nexos's {X}-only mana) is judged against the spell being cast
                            let prev_pay_for = self.pay_for.replace(Some(ctx.item.source));
                            while k < max.saturating_sub(1) {
                                total.shards.extend(unit.shards.iter().cloned());
                                if !self.can_afford(p, &total, 0) {
                                    break;
                                }
                                k += 1;
                            }
                            self.pay_for.set(prev_pay_for);
                            max = max.min(1 + k);
                        }
                    }
                    let pol = self.target_polarity(&ctx, ctx.spec_idx as u8);
                    // "target ... of an opponent's choice": another player picks the target
                    let picker = match &spec.chooser {
                        Some(_) => {
                            let opts = self.target_chooser_options(&ctx, &spec);
                            match ctx.target_chooser {
                                Some((i, q)) if i == ctx.spec_idx => q,
                                _ if opts.len() > 1 => {
                                    // several opponents could make the choice: the caster chooses which one does
                                    ctx.asking_chooser = true;
                                    self.pending = Some(Decision {
                                        player: p,
                                        kind: DecisionKind::Pick {
                                            prompt: format!("{}: choose the opponent who chooses {}", ctx.what, if spec.label.is_empty() { "the target".to_string() } else { spec.label.clone() }),
                                            source: Some(ctx.item.source),
                                            options: opts.iter().map(|q| Opt::Obj { obj: Obj::player(*q) }).collect(),
                                            min: 1,
                                            max: 1,
                                            cancellable,
                                            hidden: false,
                                            purpose: PickPurpose::Generic,
                                        },
                                    });
                                    self.run = Run::Cast(ctx);
                                    return Flow::Wait;
                                }
                                _ => opts.first().copied().unwrap_or(p),
                            }
                        }
                        None => p,
                    };
                    // a "you may" trigger's controller may decline it here instead of choosing targets for nothing
                    let declinable = picker == p && self.trigger_declinable(&ctx);
                    self.pending = Some(Decision {
                        player: picker,
                        kind: DecisionKind::Pick {
                            prompt: if spec.label.is_empty() { format!("Choose target for {}", ctx.what) } else { format!("{}: {}", ctx.what, spec.label) },
                            source: Some(ctx.item.source),
                            options: legal.into_iter().map(|obj| Opt::Obj { obj }).collect(),
                            min: if exact_amt.is_some() || (spec.max_x && spec.exact_x) { max } else { spec.min },
                            max,
                            cancellable: cancellable || declinable,
                            hidden: false,
                            purpose: PickPurpose::Targets { polarity: pol, optional: declinable },
                        },
                    });
                    self.run = Run::Cast(ctx);
                    return Flow::Wait;
                }
                Stage::Alt => {
                    let kws = self.spell_alt_keywords(p, ctx.item.source);
                    let flags = [Keyword::Convoke, Keyword::Improvise, Keyword::Delve];
                    // answer to the previous question
                    if let Some(Choice::Pick { indices }) = if ctx.alt_idx < 3 { answer.take() } else { None } {
                        let which = ctx.alt_idx;
                        let opts = self.alt_options(p, which, ctx.item.source);
                        for i in indices {
                            let Some(&id) = opts.get(i) else { continue };
                            if which == 2 {
                                self.cur_src = Some(ctx.item.source);
                                self.move_card(id, Zone::Exile, Default::default());
                            } else {
                                self.cards[id.0 as usize].tapped = true;
                                self.emit(Event::Tapped { card: id });
                                if which == 0 {
                                    self.cards[ctx.item.source.0 as usize].convoked.push(id);
                                }
                            }
                            // reduce the cost: a coloured shard of the creature's colour first (convoke), else generic
                            let cols = self.cards[id.0 as usize].cur.colors;
                            let pos = if which == 0 {
                                // the colour the lands can't cover is the one the creature pays (Black Panther's {W} for {G}{W}{W} with a Forest and a Plains)
                                let srcs = self.mana_sources_for_payment(p);
                                let supply = |c: Color| srcs.iter().filter(|m| m.produces.contains(&ManaType::of(c))).count() as i32;
                                let rem = &ctx.remaining.shards;
                                let demand = |c: Color| rem.iter().filter(|s| matches!(s, Shard::Color { c: x } if *x == c)).count() as i32;
                                let mut best: Option<(i32, usize)> = None;
                                for (j, s) in rem.iter().enumerate() {
                                    let sc = match s {
                                        Shard::Color { c } if cols.has(*c) => demand(*c) - supply(*c),
                                        Shard::Hybrid { a, b } if cols.has(*a) || cols.has(*b) => -1000,
                                        _ => continue,
                                    };
                                    if best.map_or(true, |(b, _)| sc > b) {
                                        best = Some((sc, j));
                                    }
                                }
                                best.map(|(_, j)| j)
                            } else {
                                None
                            };
                            if let Some(pos) = pos {
                                ctx.remaining.shards.remove(pos);
                            } else if let Some(pos) = ctx.remaining.shards.iter().position(|s| matches!(s, Shard::Generic { n } if *n > 0)) {
                                if let Shard::Generic { n } = &mut ctx.remaining.shards[pos] {
                                    *n -= 1;
                                }
                                ctx.remaining.shards.retain(|s| !matches!(s, Shard::Generic { n: 0 }));
                            }
                        }
                        ctx.alt_idx += 1;
                    }
                    while (ctx.alt_idx as usize) < flags.len() {
                        let which = ctx.alt_idx;
                        let has_generic = ctx.remaining.shards.iter().any(|s| matches!(s, Shard::Generic { n } if *n > 0))
                            || (which == 0 && ctx.remaining.shards.iter().any(|s| matches!(s, Shard::Color { .. } | Shard::Hybrid { .. })));
                        let opts = self.alt_options(p, which, ctx.item.source);
                        if kws.has(flags[which as usize]) && has_generic && !opts.is_empty() {
                            let total: usize = ctx.remaining.shards.iter().map(|s| match s { Shard::Generic { n } => *n as usize, Shard::Color { .. } | Shard::Hybrid { .. } if which == 0 => 1, _ => 0 }).sum();
                            // "can't spend mana to cast this spell": everything the later helpers can't pay must be paid here
                            let min_pick = if self.def_of(ctx.item.source).abilities.iter().any(|a| matches!(a, Ability::CantSpendManaToCast)) {
                                let later: usize = (which as usize + 1..flags.len()).filter(|&w| kws.has(flags[w])).map(|w| self.alt_options(p, w as u8, ctx.item.source).len()).sum();
                                total.saturating_sub(later).min(opts.len())
                            } else {
                                0
                            };
                            self.pending = Some(Decision {
                                player: p,
                                kind: DecisionKind::Pick {
                                    prompt: match which {
                                        0 => format!("Convoke: tap creatures to help pay for {}", ctx.what),
                                        1 => format!("Improvise: tap artifacts to help pay for {}", ctx.what),
                                        _ => format!("Delve: exile cards from your graveyard to help pay for {}", ctx.what),
                                    },
                                    source: Some(ctx.item.source),
                                    options: opts.iter().map(|c| Opt::Obj { obj: Obj::card(*c) }).collect(),
                                    min: min_pick as u8,
                                    max: total.min(opts.len()) as u8,
                                    cancellable: false,
                                    hidden: false,
                                    purpose: PickPurpose::Cost,
                                },
                            });
                            self.run = Run::Cast(ctx);
                            return Flow::Wait;
                        }
                        ctx.alt_idx += 1;
                    }
                    // assist: another player may pay part of the generic cost
                    if kws.has(Keyword::Assist) && ctx.assist_state < 3 && matches!(ctx.origin, Origin::Spell { .. }) {
                        let generic: i32 = ctx.remaining.shards.iter().map(|s| if let Shard::Generic { n } = s { *n as i32 } else { 0 }).sum();
                        if ctx.assist_state == 1 {
                            ctx.assist_state = 3;
                            if let Some(Choice::Pick { indices }) = answer.take() {
                                let helpers: Vec<PlayerId> = self.alive_players().into_iter().filter(|q| *q != p).collect();
                                if let Some(&h) = indices.first().and_then(|i| helpers.get(*i)) {
                                    let avail = self.players[h as usize].pool.total() as i32 + self.mana_sources_for_payment(h).len() as i32;
                                    let hi = generic.min(avail);
                                    if hi > 0 {
                                        ctx.assist_player = h;
                                        ctx.assist_state = 2;
                                        self.pending = Some(Decision {
                                            player: h,
                                            kind: DecisionKind::Number { prompt: format!("Assist: how much of {}'s generic cost do you pay?", ctx.what), min: 0, max: hi, cancellable: false },
                                        });
                                        self.run = Run::Cast(ctx);
                                        return Flow::Wait;
                                    }
                                }
                            }
                        } else if ctx.assist_state == 2 {
                            ctx.assist_state = 3;
                            if let Some(Choice::Number { n }) = answer.take() {
                                let n = (n.max(0)).min(generic);
                                if n > 0 {
                                    let h = ctx.assist_player;
                                    let mut tmp = (*ctx).clone();
                                    tmp.remaining = ManaCost { shards: vec![Shard::Generic { n: n as u8 }].into(), snow_any: false, white_any: false };
                                    self.pay_for.set(Some(ctx.item.source));
                                    let ok = self.auto_pay(h, &mut tmp);
                                    self.pay_for.set(None);
                                    if ok {
                                        self.cards[ctx.item.source.0 as usize].assist_by = Some(h);
                                        let mut left = n;
                                        for sh in ctx.remaining.shards.iter_mut() {
                                            if let Shard::Generic { n: g } = sh {
                                                let r = (*g as i32).min(left);
                                                *g -= r as u8;
                                                left -= r;
                                            }
                                        }
                                        ctx.remaining.shards.retain(|s| !matches!(s, Shard::Generic { n: 0 }));
                                    }
                                }
                            }
                        } else if generic > 0 {
                            let helpers: Vec<PlayerId> = self.alive_players().into_iter().filter(|q| *q != p).collect();
                            let can: Vec<PlayerId> = helpers.iter().copied().filter(|h| self.players[*h as usize].pool.total() > 0 || !self.mana_sources_for_payment(*h).is_empty()).collect();
                            if can.is_empty() {
                                ctx.assist_state = 3;
                            } else {
                                ctx.assist_state = 1;
                                self.pending = Some(Decision {
                                    player: p,
                                    kind: DecisionKind::Pick {
                                        prompt: format!("Assist: choose a player to help pay for {} (or none)", ctx.what),
                                        source: Some(ctx.item.source),
                                        options: helpers.iter().map(|h| Opt::Obj { obj: Obj::player(*h) }).collect(),
                                        min: 0,
                                        max: 1,
                                        cancellable: false,
                                        hidden: false,
                                        purpose: PickPurpose::Generic,
                                    },
                                });
                                self.run = Run::Cast(ctx);
                                return Flow::Wait;
                            }
                        } else {
                            ctx.assist_state = 3;
                        }
                        // asked something: come back through the same stage for the next step
                        if ctx.assist_state < 3 || ctx.assist_state == 3 {
                            continue;
                        }
                    }
                    // the deferred extra-mana question (Stage::Bonus) comes now, with the convoked cost already removed; its next stage is Pay
                    if self.spell_mana_for_counters(p, ctx.item.source) && self.spell_has_alt_cost(p, ctx.item.source) {
                        ctx.opt_next = Stage::Pay;
                        ctx.stage = Stage::Bonus;
                        continue;
                    }
                    ctx.stage = Stage::Pay;
                    continue;
                }
                Stage::Extra => {
                    if ctx.extra_idx >= ctx.extra.len() {
                        ctx.stage = if ctx.extra_late { Stage::Finish } else { Stage::Alt };
                        continue;
                    }
                    let part = ctx.extra[ctx.extra_idx].clone();
                    match self.step_extra_cost(&mut ctx, &part, &mut answer) {
                        Extra::Wait => {
                            self.run = Run::Cast(ctx);
                            return Flow::Wait;
                        }
                        Extra::Abort => return self.abort_cast(ctx),
                        Extra::Retry => continue,
                        Extra::Paid => {
                            ctx.extra_idx += 1;
                            continue;
                        }
                    }
                }
                Stage::Pay => {
                    // spend floating mana first (not for a spell that only takes mana from certain sources)
                    if !self.spend_only_item(&ctx.item) {
                        self.spend_pool(p, &mut ctx.remaining);
                    }
                    if ctx.remaining.is_empty() {
                        self.players[p as usize].manual_once = false;
                        ctx.stage = if ctx.extra_late { Stage::Extra } else { Stage::Finish };
                        continue;
                    }
                    let manual = self.players[p as usize].manual_pay || self.players[p as usize].manual_once;
                    // the cards a picked mana source exiles were just chosen: tap it with them
                    if let Some((card, ability, produce, options)) = ctx.tap_pick.take() {
                        if let Some(Choice::Pick { indices }) = answer.take() {
                            self.mana_picks = indices.iter().filter_map(|i| options.get(*i).copied()).collect();
                        }
                        answer = Some(Choice::Pay { answer: PayAnswer::Tap { card, ability, produce } });
                    }
                    match answer.take() {
                        Some(Choice::Pay { answer: pa }) => match pa {
                            PayAnswer::Tap { card, .. } if (card.0 as usize) >= self.cards.len() => {
                                // `validate` refuses sources that name nothing; never index out of bounds here
                                continue;
                            }
                            PayAnswer::Tap { card, ability, produce } => {
                                if self.mana_picks.is_empty() {
                                    if let Some((options, n)) = self.mana_pick_options(p, card, ability) {
                                        self.pending = Some(Decision {
                                            player: p,
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
                                        ctx.tap_pick = Some((card, ability, produce, options));
                                        self.run = Run::Cast(ctx);
                                        return Flow::Wait;
                                    }
                                }
                                self.note_source_paid(card);
                                self.note_snow_color(p, card, ability, produce);
                                self.paying_activation.set(true);
                                self.pay_is_ability.set(matches!(ctx.item.kind, ItemKind::Ability { .. }));
                                self.pay_for.set(Some(ctx.item.source));
                                let r = self.activate_mana_ability(p, card, ability, produce);
                                self.mana_picks.clear();
                                self.pay_for.set(None);
                                self.paying_activation.set(false);
                                if r.is_err() {
                                    answer = None;
                                }
                                continue;
                            }
                            PayAnswer::Pool { mana } if self.spend_only_item(&ctx.item) => {
                                let _ = mana;
                                continue;
                            }
                            PayAnswer::Pool { mana } => {
                                let mut pool = self.players[p as usize].pool;
                                if pool.take(mana) {
                                    self.note_spent(mana);
                                    let mut single = ManaPool::default();
                                    single.add(mana, 1);
                                    self.spend_specific(&mut ctx.remaining, mana);
                                    self.players[p as usize].pool = pool;
                                }
                                continue;
                            }
                            PayAnswer::Life => {
                                if let Some(i) = self.spend_as_cost(p, &ctx.remaining).shards.iter().position(|s| matches!(s, Shard::Phyrexian { .. } | Shard::PhyrexianHybrid { .. })) {
                                    ctx.remaining.shards.remove(i);
                                    ctx.paid_life += 2;
                                }
                                continue;
                            }
                            PayAnswer::Auto => {
                                if !self.auto_pay(p, &mut ctx) {
                                    return self.abort_cast(ctx);
                                }
                                continue;
                            }
                        },
                        _ => {}
                    }
                    self.pay_is_ability.set(matches!(ctx.item.kind, ItemKind::Ability { .. }));
                    if let ItemKind::Ability { ability, .. } = ctx.item.kind {
                        let src = ctx.item.source;
                        self.pay_is_equip.set(self.def_of(src).abilities.get(ability as usize).map_or(false, |a| self.is_equip_for(src, a)));
                        self.pay_is_power_up.set(self.def_of(src).abilities.get(ability as usize).map_or(false, |a| matches!(a, Ability::Activated { power_up: true, .. })));
                        self.pay_is_foretell.set(self.def_of(src).abilities.get(ability as usize).map_or(false, Self::is_foretell_ability));
                        let hx = self.def_of(src).abilities.get(ability as usize).map_or(false, |a| matches!(a, Ability::Activated { cost, .. } if cost.iter().any(|part| matches!(part, Cost::Mana { cost } if cost.contains("{X}")) || matches!(part, Cost::XTimes { .. }))));
                        self.pay_ability_has_x.set(hx);
                    }
                    if !manual {
                        self.pay_for.set(Some(ctx.item.source));
                        let paid = self.auto_pay(p, &mut ctx);
                        self.pay_for.set(None);
                        self.pay_is_ability.set(false);
                        if !paid {
                            return self.abort_cast(ctx);
                        }
                        continue;
                    }
                    let pool = self.players[p as usize].pool;
                    self.pay_for.set(Some(ctx.item.source));
                    let sources = self.mana_sources(p);
                    self.pay_for.set(None);
                    self.pay_is_ability.set(false);
                    self.pending = Some(Decision {
                        player: p,
                        kind: DecisionKind::PayMana {
                            what: ctx.what.clone(),
                            remaining: ctx.remaining.display(),
                            pool,
                            sources,
                            can_pay_life: self.spend_as_cost(p, &ctx.remaining).shards.iter().any(|s| matches!(s, Shard::Phyrexian { .. } | Shard::PhyrexianHybrid { .. })),
                        },
                    });
                    self.run = Run::Cast(ctx);
                    return Flow::Wait;
                }
                Stage::Finish => {
                    return self.finish_cast(ctx);
                }
            }
        }
    }

    /// Whether the `idx`-th target of what is being cast should be chosen from opponents' or one's own things.
    fn target_polarity(&self, ctx: &CastCtx, idx: u8) -> crate::polarity::Polarity {
        let d = self.db.def(ctx.item.def);
        let mut effects: Vec<Effect> = vec![];
        match &ctx.origin {
            Origin::Spell { .. } => {
                for a in &d.abilities {
                    match a {
                        Ability::Spell { effects: e, .. } => effects.extend(e.iter().cloned()),
                        Ability::Modal { modes, .. } => {
                            for &m in &ctx.item.modes {
                                effects.extend(modes[m as usize].effects.iter().cloned());
                            }
                        }
                        Ability::Attach { .. } => return crate::polarity::Polarity::Friendly,
                        _ => {}
                    }
                }
            }
            Origin::Ability { ability, card } => {
                if let Some(Ability::Activated { effects: e, .. }) = self.ability_owned(*card, *ability) {
                    effects.extend(e.iter().cloned());
                }
            }
            Origin::Trigger => {
                if let ItemKind::Triggered { ability, .. } = &ctx.item.kind {
                    if let Some(Ability::Triggered { effects: e, .. }) = d.abilities.get(*ability as usize) {
                        effects.extend(e.iter().cloned());
                    }
                }
            }
        }
        crate::polarity::polarity_of(&effects, idx)
    }

    /// How much larger X could get by exiling cards from hand as a multikicker that makes the spell cost {N} less each ("costs {2} less for each card exiled this way"):
    /// N times the number of cards in hand that could be exiled. Only used to judge whether X-dependent targets could exist.
    fn kick_x_bonus(&self, p: PlayerId, id: CardId) -> i32 {
        let d = self.def_of(id);
        let mut cards = 0;
        for a in &d.abilities {
            if let Ability::Kicker { cost, multi: true, .. } = a {
                for c in cost {
                    if let Cost::ExileFromHand { filter, n, .. } = c {
                        let mut f = filter.clone();
                        f.controller = PlayerFilter::Any;
                        let k = self.players[p as usize].hand.iter().filter(|h| **h != id && self.matches_card(&f, **h, id, p)).count() as i32;
                        cards = cards.max(k / (*n).max(1));
                    }
                }
            }
        }
        if cards == 0 {
            return 0;
        }
        let mut per = 0;
        for &i in &d.statics {
            if let Ability::Static { effect: StaticEffect::CostChange { filter, generic, per: Some(Amount::KickCount), .. }, .. } = &d.abilities[i as usize] {
                if filter.this && *generic < 0 {
                    per += -*generic;
                }
            }
        }
        cards * per
    }

    /// `base` plus one colored shard for each color in the bit mask (`Color::bit`).
    fn with_color_shards(base: &ManaCost, mask: u8) -> ManaCost {
        let mut m = base.clone();
        for c in Color::ALL {
            if mask & c.bit() != 0 {
                m.shards.push(Shard::Color { c });
            }
        }
        m
    }

    fn max_x(&self, p: PlayerId, cost: &ManaCost, spell: CardId, x_red: u32) -> i32 {
        let (cost, reduction) = self.alt_help_parts(p, spell, cost);
        let cost = &cost;
        let xs = cost.shards.iter().filter(|s| matches!(s, Shard::X)).count() as u32;
        if xs == 0 {
            return 0;
        }
        // X is paid with distinct colors only (Emblazoned Golem): the largest set of colors that can be paid along with the rest of the cost
        if self.def_of(spell).abilities.iter().any(|a| matches!(a, Ability::XColoredDistinct)) {
            let mut base = cost.clone();
            base.shards.retain(|s| !matches!(s, Shard::X));
            let prev = self.pay_for.replace(Some(spell));
            let mut best = 0u32;
            for mask in 0u8..32 {
                let n = mask.count_ones();
                if n > best && self.can_afford(p, &Self::with_color_shards(&base, mask), 0) {
                    best = n;
                }
            }
            self.pay_for.set(prev);
            return best as i32;
        }
        // convoke / improvise / delve first pay down the fixed generic part, then X
        let base_generic: u32 = cost.shards.iter().map(|s| if let Shard::Generic { n } = s { *n as u32 } else { 0 }).sum();
        let on_base = reduction.min(base_generic);
        let on_x = reduction - on_base + x_red;
        let mut c2 = cost.clone();
        let mut left = on_base;
        for sh in c2.shards.iter_mut() {
            if let Shard::Generic { n } = sh {
                let r = (*n as u32).min(left);
                *n -= r as u8;
                left -= r;
            }
        }
        c2.shards.retain(|s| !matches!(s, Shard::Generic { n: 0 }));
        // the largest X whose total cost (every {X} counts X generic mana) can still be paid
        let mut x = 0u32;
        // restricted mana ("spend only on spells with {X}") counts toward the maximum when it fits this spell
        let prev = self.pay_for.replace(Some(spell));
        while x < 99 && {
            // every {X} counts X generic mana, then the cost reduction that could not come off the fixed generic part
            let mut c3 = c2.clone();
            c3.shards.retain(|s| !matches!(s, Shard::X));
            let total = ((x + 1) * xs).saturating_sub(on_x);
            if total > 0 {
                c3.add_generic(total.min(250) as u8);
            }
            self.can_afford(p, &c3, 0)
        } {
            x += 1;
        }
        self.pay_for.set(prev);
        x as i32
    }

    pub fn obj_zcc(&self, o: &Obj) -> u32 {
        match o {
            Obj::Card { id } => self.cards[id.0 as usize].zcc,
            _ => 0,
        }
    }

    /// Returns None when the cost part was paid, Some(Wait) when a question is pending, Some(Continue) to abandon.
    /// The players who may pick the targets of a slot with a `chooser` ("target ... of an opponent's choice"): the
    /// players it selects who are still in the game. The chooser may name an earlier target ("that creature's controller").
    fn target_chooser_options(&self, ctx: &CastCtx, spec: &TargetSpec) -> Vec<PlayerId> {
        let Some(sel) = &spec.chooser else { return vec![] };
        let mut tidx: Vec<u8> = vec![];
        let mut off = 0usize;
        for &c in ctx.item.spec_counts.iter().take(ctx.spec_idx) {
            tidx.push(if c == 0 { 255 } else { off as u8 });
            off += c as usize;
        }
        let targets: Vec<Target> = ctx.item.targets.to_vec();
        let evt = ctx.item.ctx.clone();
        let mut ec = EvalCtx::with_controller(ctx.item.source, ctx.player);
        ec.event = &evt;
        ec.targets = &targets;
        ec.tidx = &tidx;
        self.sel_players(sel, &ec).into_iter().filter(|q| self.alive(*q)).collect()
    }

    fn step_extra_cost(&mut self, ctx: &mut CastCtx, part: &Cost, answer: &mut Option<Choice>) -> Extra {
        // a cost filter may read the chosen targets (Sidisi: "mana value X ... target creature card with mana value X plus 1")
        let mut tidx: Vec<u8> = vec![];
        let mut off = 0usize;
        for &c in ctx.item.spec_counts.iter() {
            tidx.push(if c == 0 { 255 } else { off as u8 });
            off += c as usize;
        }
        *self.filter_targets.borrow_mut() = (ctx.item.targets.to_vec(), tidx);
        let r = self.step_extra_cost_inner(ctx, part, answer);
        *self.filter_targets.borrow_mut() = (vec![], vec![]);
        r
    }

    fn step_extra_cost_inner(&mut self, ctx: &mut CastCtx, part: &Cost, answer: &mut Option<Choice>) -> Extra {
        let p = ctx.player;
        let source = ctx.item.source;
        // "an opponent gains 3 life" / "an opponent creates ...": the caster chooses the opponent; it is paid with the other costs
        if matches!(part, Cost::OpponentGainsLife { .. } | Cost::OpponentCreatesToken { .. }) {
            let opps = self.cost_opponents(p);
            if opps.len() <= 1 {
                return Extra::Paid;
            }
            if let Some(Choice::Pick { indices }) = answer.take() {
                let o = indices.first().and_then(|i| opps.get(*i)).copied().unwrap_or(opps[0]);
                ctx.extra[ctx.extra_idx] = match part {
                    Cost::OpponentGainsLife { n } => Cost::PlayerGainsLife { player: o, n: *n },
                    Cost::OpponentCreatesToken { token, n } => Cost::PlayerCreatesToken { player: o, token: token.clone(), n: *n },
                    other => other.clone(),
                };
                return Extra::Paid;
            }
            self.pending = Some(Decision {
                player: p,
                kind: DecisionKind::Pick {
                    prompt: format!("Choose an opponent for {}: {}", ctx.what, crate::describe::cost_text(part)),
                    source: Some(source),
                    options: opps.iter().map(|q| Opt::Obj { obj: Obj::player(*q) }).collect(),
                    min: 1,
                    max: 1,
                    cancellable: ctx.backup.is_some(),
                    hidden: false,
                    purpose: PickPurpose::Cost,
                },
            });
            return Extra::Wait;
        }
        if matches!(part, Cost::RemoveAnyCounters { .. } | Cost::RemoveAnyCountersFrom { .. } | Cost::RemoveXCountersFromOne { .. }) {
            return self.pay_any_kind_counters(ctx, part, answer);
        }
        let mut random_cost = false;
        let (zone, filter, n, kind): (Zone, Filter, i32, &str) = match part {
            Cost::Sacrifice { filter, n } => (Zone::Battlefield, self.sac_cost_filter(filter), *n, "sacrifice"),
            Cost::SacrificeAmount { filter, amount } => {
                let n = self.life_cost(source, amount);
                if n <= 0 {
                    return Extra::Paid;
                }
                (Zone::Battlefield, self.sac_cost_filter(filter), n, "sacrifice")
            }
            Cost::SacrificeReduceByManaValue { filter } | Cost::SacrificeReduceByManaCost { filter } => (Zone::Battlefield, self.sac_cost_filter(filter), 1, "sacrifice"),
            Cost::TapReduceByPower { filter } => (Zone::Battlefield, filter.clone().into_filter(), 1, "tap"),
            Cost::SacrificeAll { filter, power_to_x } => {
                let mut f = filter.clone();
                f.controller = PlayerFilter::Any;
                let all: Vec<CardId> = self.battlefield.iter().copied().filter(|id| self.cards[id.0 as usize].cur.controller == p && self.matches_card(&f, *id, source, p)).collect();
                if *power_to_x {
                    ctx.item.x = all.iter().map(|id| self.power_of(*id).max(0)).sum();
                }
                for id in all {
                    self.emit(Event::Sacrificed { card: id, player: p });
                    self.move_card(id, Zone::Graveyard, Default::default());
                }
                return Extra::Paid;
            }
            Cost::CollectEvidenceTargets => {
                let n: i32 = ctx.item.targets.iter().filter_map(|t| if let Obj::Card { id } = &t.obj { Some(if self.cards[id.0 as usize].cur.no_mana_cost { 0 } else { self.def_of(*id).mana_value as i32 }) } else { None }).sum();
                let Some(cards) = self.evidence_cards(p, n, source) else {
                    return Extra::Abort;
                };
                for id in cards {
                    self.move_card(id, Zone::Exile, Default::default());
                }
                self.emit(Event::EvidenceCollected { player: p });
                return Extra::Paid;
            }
            Cost::ExileSymbolsFromGraveyard { filter, color, n } => {
                let Some(cards) = self.symbol_cards(p, filter, *color, *n, source) else {
                    return Extra::Abort;
                };
                for id in cards {
                    self.move_card(id, Zone::Exile, Default::default());
                    ctx.item.remembered.push(Obj::card(id));
                }
                return Extra::Paid;
            }
            Cost::DiscardHand => {
                let hand = self.players[p as usize].hand.clone();
                for id in hand {
                    if id == source {
                        continue;
                    }
                    self.move_card(id, self.discard_zone(id), Default::default());
                    self.emit(Event::Discarded { card: id, player: p, by: self.cause_ctl.get() });
                    ctx.item.remembered.push(Obj::card(id));
                }
                return Extra::Paid;
            }
            Cost::Discard { filter, n, random } => {
                random_cost = *random;
                (Zone::Hand, filter.clone().into_filter(), *n, "discard")
            }
            Cost::PutExiledIntoGraveyard { filter } => (Zone::Exile, filter.clone().into_filter(), 1, "exiled_to_gy"),
            Cost::ExileSpellFromStack { filter } => (Zone::Stack, filter.clone().into_filter(), 1, "exile_stack"),
            Cost::ExileFromGraveyard { filter, n, bottom } => (Zone::Graveyard, filter.clone().into_filter(), *n, if *bottom { "to_bottom" } else { "exile" }),
            Cost::ExileFromGraveyardAmount { filter, amount } => {
                let n = self.life_cost(source, amount);
                if n <= 0 {
                    return Extra::Paid;
                }
                (Zone::Graveyard, filter.clone().into_filter(), n, "exile")
            }
            Cost::ExileFromGraveyardPayCost { filter } => (Zone::Graveyard, filter.clone().into_filter(), 1, "exile_pay"),
            Cost::ExileFromHand { filter, n, to_library } => (Zone::Hand, filter.clone().into_filter(), *n, if *to_library { "to_library_top" } else { "exile" }),
            // revealing a card from hand: the chosen card is remembered ("the revealed card's mana value"); revealing this card itself stays a plain cost
            Cost::Reveal { filter, n } if !filter.this => (Zone::Hand, filter.clone().into_filter(), *n, "reveal"),
            Cost::TapPermanents { filter, n } => (Zone::Battlefield, filter.clone().into_filter(), *n, "tap"),
            Cost::UntapPermanents { filter, n, .. } => (Zone::Battlefield, filter.clone().into_filter(), *n, "untap"),
            Cost::ReturnToHand { filter, n } => (Zone::Battlefield, filter.clone().into_filter(), *n, "return"),
            Cost::ExileFromBattlefield { filter, n } => (Zone::Battlefield, filter.clone().into_filter(), *n, "exile_bf"),
            Cost::RemoveCountersFrom { filter, kind, n, among: true, suspended } => {
                // counters taken automatically from as many permanents as needed
                let mut left = *n;
                for (id, k) in self.counter_pool(filter, *kind, *suspended, source, p) {
                    if left <= 0 {
                        break;
                    }
                    let take = k.min(left);
                    self.remove_counters_from(Obj::card(id), *kind, take);
                    left -= take;
                }
                return if left <= 0 { Extra::Paid } else { Extra::Abort };
            }
            Cost::RemoveCountersFrom { filter, .. } => (Zone::Battlefield, filter.clone().into_filter(), 1, "rm_counter"),
            Cost::PutCountersOnChosen { filter, .. } => (Zone::Battlefield, filter.clone().into_filter(), 1, "put_counter"),
            Cost::DiscardSameName { filter, n } => {
                let eligible = self.hand_matching(p, source, filter);
                let Some(auto) = self.same_name_group(&eligible, *n) else {
                    return Extra::Abort;
                };
                let mut chosen: Vec<CardId> = auto;
                if let Some(Choice::Pick { indices }) = answer.take() {
                    let picked: Vec<CardId> = indices.iter().filter_map(|i| eligible.get(*i).copied()).collect();
                    if picked.len() == *n as usize && self.same_name_group(&picked, *n).is_some() {
                        chosen = picked;
                    }
                } else if eligible.len() > *n as usize {
                    self.pending = Some(Decision {
                        player: p,
                        kind: DecisionKind::Pick {
                            prompt: format!("Discard {n} cards with the same name for {}", ctx.what),
                            source: Some(source),
                            options: eligible.iter().map(|id| Opt::Obj { obj: Obj::card(*id) }).collect(),
                            min: *n as u8,
                            max: *n as u8,
                            cancellable: ctx.backup.is_some(),
                            hidden: false,
                            purpose: PickPurpose::Cost,
                        },
                    });
                    return Extra::Wait;
                }
                for id in &chosen {
                    self.move_card(*id, self.discard_zone(*id), Default::default());
                    self.emit(Event::Discarded { card: *id, player: p, by: self.cause_ctl.get() });
                    ctx.item.remembered.push(Obj::card(*id));
                }
                return Extra::Paid;
            }
            Cost::DiscardDistinctNames { filter, n } => {
                let eligible = self.hand_matching(p, source, filter);
                if !self.distinct_names_ok(&eligible, *n) {
                    return Extra::Abort;
                }
                let mut chosen: Vec<CardId> = vec![];
                if let Some(Choice::Pick { indices }) = answer.take() {
                    let picked: Vec<CardId> = indices.iter().filter_map(|i| eligible.get(*i).copied()).collect();
                    if picked.len() == *n as usize && self.distinct_names_ok(&picked, *n) {
                        chosen = picked;
                    }
                }
                if chosen.is_empty() {
                    if eligible.len() as i32 == *n {
                        chosen = eligible.clone();
                    } else {
                        self.pending = Some(Decision {
                            player: p,
                            kind: DecisionKind::Pick {
                                prompt: format!("Discard {n} cards with different names for {}", ctx.what),
                                source: Some(source),
                                options: eligible.iter().map(|id| Opt::Obj { obj: Obj::card(*id) }).collect(),
                                min: *n as u8,
                                max: *n as u8,
                                cancellable: ctx.backup.is_some(),
                                hidden: false,
                                purpose: PickPurpose::Cost,
                            },
                        });
                        return Extra::Wait;
                    }
                }
                for id in &chosen {
                    self.move_card(*id, self.discard_zone(*id), Default::default());
                    self.emit(Event::Discarded { card: *id, player: p, by: self.cause_ctl.get() });
                    ctx.item.remembered.push(Obj::card(*id));
                }
                return Extra::Paid;
            }
            Cost::SacrificeDistinctNames { filter, n } => {
                let eligible = self.sac_pool(filter, source, p);
                if !self.distinct_names_ok(&eligible, *n) {
                    return Extra::Abort;
                }
                let mut chosen: Vec<CardId> = vec![];
                if let Some(Choice::Pick { indices }) = answer.take() {
                    let picked: Vec<CardId> = indices.iter().filter_map(|i| eligible.get(*i).copied()).collect();
                    if picked.len() == *n as usize && self.distinct_names_ok(&picked, *n) {
                        chosen = picked;
                    }
                }
                if chosen.is_empty() {
                    if eligible.len() as i32 == *n {
                        chosen = eligible.clone();
                    } else {
                        self.pending = Some(Decision {
                            player: p,
                            kind: DecisionKind::Pick {
                                prompt: format!("Sacrifice {n} permanents with different names for {}", ctx.what),
                                source: Some(source),
                                options: eligible.iter().map(|id| Opt::Obj { obj: Obj::card(*id) }).collect(),
                                min: *n as u8,
                                max: *n as u8,
                                cancellable: ctx.backup.is_some(),
                                hidden: false,
                                purpose: PickPurpose::Cost,
                            },
                        });
                        return Extra::Wait;
                    }
                }
                for id in chosen {
                    self.emit(Event::Sacrificed { card: id, player: p });
                    self.move_card(id, Zone::Graveyard, Default::default());
                    ctx.item.remembered.push(Obj::card(id));
                }
                return Extra::Paid;
            }
            Cost::TapShareType { filter, n } => {
                let eligible = self.tappable_for_cost(p, source, filter);
                let Some(auto) = self.share_type_group(&eligible, *n) else {
                    return Extra::Abort;
                };
                let mut chosen: Vec<CardId> = auto;
                if let Some(Choice::Pick { indices }) = answer.take() {
                    let picked: Vec<CardId> = indices.iter().filter_map(|i| eligible.get(*i).copied()).collect();
                    if picked.len() == *n as usize && self.share_type_group(&picked, *n).is_some() {
                        chosen = picked;
                    }
                } else if eligible.len() > *n as usize {
                    self.pending = Some(Decision {
                        player: p,
                        kind: DecisionKind::Pick {
                            prompt: format!("Tap {n} creatures that share a creature type for {}", ctx.what),
                            source: Some(source),
                            options: eligible.iter().map(|id| Opt::Obj { obj: Obj::card(*id) }).collect(),
                            min: *n as u8,
                            max: *n as u8,
                            cancellable: ctx.backup.is_some(),
                            hidden: false,
                            purpose: PickPurpose::Cost,
                        },
                    });
                    return Extra::Wait;
                }
                for id in &chosen {
                    self.cards[id.0 as usize].tapped = true;
                    self.emit(Event::Tapped { card: *id });
                    ctx.item.remembered.push(Obj::card(*id));
                }
                return Extra::Paid;
            }
            Cost::TapTotalPower { filter, power } => {
                let eligible = self.tappable_for_cost(p, source, filter);
                let total = |ids: &[CardId], g: &Game| ids.iter().map(|id| g.cards[id.0 as usize].cur.power.max(0)).sum::<i32>();
                if total(&eligible, self) < *power {
                    return Extra::Abort;
                }
                if let Some(Choice::Pick { indices }) = answer.take() {
                    let chosen: Vec<CardId> = indices.iter().filter_map(|i| eligible.get(*i).copied()).collect();
                    if total(&chosen, self) >= *power {
                        for id in &chosen {
                            self.cards[id.0 as usize].tapped = true;
                            self.crew_log.push((*id, source, self.turn));
                            self.emit(Event::CrewPaid { creature: *id, vehicle: source, saddle: true });
                            self.teamwork_tap.set(!matches!(ctx.item.kind, ItemKind::Ability { .. }));
                            self.emit(Event::Tapped { card: *id });
                            self.teamwork_tap.set(false);
                            ctx.item.remembered.push(Obj::card(*id));
                        }
                        return Extra::Paid;
                    }
                }
                self.pending = Some(Decision {
                    player: p,
                    kind: DecisionKind::Pick {
                        prompt: format!("Tap creatures with total power {power} or greater for {}", ctx.what),
                        source: Some(source),
                        options: eligible.iter().map(|id| Opt::Obj { obj: Obj::card(*id) }).collect(),
                        min: 1,
                        max: eligible.len() as u8,
                        cancellable: ctx.backup.is_some(),
                        hidden: false,
                        purpose: PickPurpose::Cost,
                    },
                });
                return Extra::Wait;
            }
            Cost::SacrificeTotalPower { filter, power } => {
                let eligible = self.eligible_permanents(p, filter, source);
                let total = |ids: &[CardId], g: &Game| ids.iter().map(|id| g.cards[id.0 as usize].cur.power.max(0)).sum::<i32>();
                if total(&eligible, self) < *power {
                    return Extra::Abort;
                }
                if let Some(Choice::Pick { indices }) = answer.take() {
                    let chosen: Vec<CardId> = indices.iter().filter_map(|i| eligible.get(*i).copied()).collect();
                    if total(&chosen, self) >= *power {
                        for id in &chosen {
                            self.emit(Event::Sacrificed { card: *id, player: p });
                            self.move_card(*id, Zone::Graveyard, Default::default());
                            ctx.item.remembered.push(Obj::card(*id));
                        }
                        return Extra::Paid;
                    }
                }
                self.pending = Some(Decision {
                    player: p,
                    kind: DecisionKind::Pick {
                        prompt: format!("Sacrifice creatures with total power {power} or greater for {}", ctx.what),
                        source: Some(source),
                        options: eligible.iter().map(|id| Opt::Obj { obj: Obj::card(*id) }).collect(),
                        min: 1,
                        max: eligible.len() as u8,
                        cancellable: ctx.backup.is_some(),
                        hidden: false,
                        purpose: PickPurpose::Cost,
                    },
                });
                return Extra::Wait;
            }
            Cost::Waterbend { n } | Cost::TapCreaturesOrMana { n } => {
                let eligible = if matches!(part, Cost::Waterbend { .. }) { self.waterbend_sources(p) } else { self.creature_bend_sources(p) };
                let chosen: Vec<CardId> = if let Some(Choice::Pick { indices }) = answer.take() {
                    indices.iter().filter_map(|i| eligible.get(*i).copied()).take((*n).max(0) as usize).collect()
                } else if eligible.is_empty() || *n <= 0 {
                    vec![]
                } else {
                    self.pending = Some(Decision {
                        player: p,
                        kind: DecisionKind::Pick {
                            prompt: format!("Waterbend {n}: tap artifacts and creatures to help pay for {}", ctx.what),
                            source: Some(source),
                            options: eligible.iter().map(|id| Opt::Obj { obj: Obj::card(*id) }).collect(),
                            min: 0,
                            max: (*n).min(eligible.len() as i32) as u8,
                            cancellable: ctx.backup.is_some(),
                            hidden: false,
                            purpose: PickPurpose::Cost,
                        },
                    });
                    return Extra::Wait;
                };
                for id in &chosen {
                    self.cards[id.0 as usize].tapped = true;
                    self.emit(Event::Tapped { card: *id });
                    ctx.item.remembered.push(Obj::card(*id));
                }
                let rest = (*n - chosen.len() as i32).max(0) as u8;
                ctx.mana.add_generic(rest);
                ctx.remaining.add_generic(rest);
                if matches!(part, Cost::Waterbend { .. }) {
                    self.emit(Event::Bent { player: p, kind: BendKind::Water });
                }
                return Extra::Paid;
            }
            Cost::ExileGraveyardTypes { n } => {
                let eligible = self.gy_types_pool(p, source);
                if !self.gy_types_ok(&eligible, *n) {
                    return Extra::Abort;
                }
                let mut chosen: Option<Vec<CardId>> = None;
                if let Some(Choice::Pick { indices }) = answer.take() {
                    let picked: Vec<CardId> = indices.iter().filter_map(|i| eligible.get(*i).copied()).collect();
                    if self.gy_types_ok(&picked, *n) {
                        chosen = Some(picked);
                    }
                }
                let Some(chosen) = chosen else {
                    self.pending = Some(Decision {
                        player: p,
                        kind: DecisionKind::Pick {
                            prompt: format!("Choose cards from your graveyard with {n} or more card types among them to exile for {}", ctx.what),
                            source: Some(source),
                            options: eligible.iter().map(|id| Opt::Obj { obj: Obj::card(*id) }).collect(),
                            min: 1,
                            max: eligible.len() as u8,
                            cancellable: ctx.backup.is_some(),
                            hidden: false,
                            purpose: PickPurpose::Cost,
                        },
                    });
                    return Extra::Wait;
                };
                for id in chosen {
                    self.cur_src = Some(ctx.item.source);
                    self.move_card(id, Zone::Exile, Default::default());
                    ctx.item.remembered.push(Obj::card(id));
                }
                return Extra::Paid;
            }
            Cost::UnattachAttached { filter } => {
                // the player picks which attached permanent is unattached; it is Remembered for the ability's effects ("that Equipment's mana value")
                let mut f = filter.clone();
                f.controller = PlayerFilter::Any;
                let eligible: Vec<CardId> = self.battlefield.iter().copied().filter(|id| self.cards[id.0 as usize].attached_to == Some(source) && self.matches_card(&f, *id, source, p)).collect();
                if eligible.is_empty() {
                    return Extra::Abort;
                }
                let pick = if eligible.len() == 1 {
                    eligible[0]
                } else if let Some(Choice::Pick { indices }) = answer.take() {
                    match indices.first().and_then(|i| eligible.get(*i).copied()) {
                        Some(id) => id,
                        None => eligible[0],
                    }
                } else {
                    self.pending = Some(Decision {
                        player: p,
                        kind: DecisionKind::Pick {
                            prompt: format!("Choose a permanent to unattach for {}", ctx.what),
                            source: Some(source),
                            options: eligible.iter().map(|id| Opt::Obj { obj: Obj::card(*id) }).collect(),
                            min: 1,
                            max: 1,
                            cancellable: ctx.backup.is_some(),
                            hidden: false,
                            purpose: PickPurpose::Cost,
                        },
                    });
                    return Extra::Wait;
                };
                ctx.item.remembered.push(Obj::card(pick));
                return Extra::Paid;
            }
            Cost::ExileCraftMaterials { bf, gy, min, share_type } => {
                let eligible = self.craft_pool(bf, gy, p, source);
                let need = (*min).max(1) as usize;
                if !self.craft_ok(bf, gy, *min, *share_type, p, source) {
                    return Extra::Abort;
                }
                let mut chosen: Option<Vec<CardId>> = None;
                if let Some(Choice::Pick { indices }) = answer.take() {
                    let picked: Vec<CardId> = indices.iter().filter_map(|i| eligible.get(*i).copied()).collect();
                    let share_ok = !*share_type || (picked.len() == need && (0..32).any(|b| picked.iter().all(|id| self.cards[id.0 as usize].cur.types.0 as u64 >> b & 1 == 1)));
                    if picked.len() >= need && share_ok {
                        chosen = Some(picked);
                    }
                }
                let Some(chosen) = chosen else {
                    self.pending = Some(Decision {
                        player: p,
                        kind: DecisionKind::Pick {
                            prompt: format!("Choose creatures you control and/or creature cards in your graveyard to exile for {}", ctx.what),
                            source: Some(source),
                            options: eligible.iter().map(|id| Opt::Obj { obj: Obj::card(*id) }).collect(),
                            min: need as u8,
                            max: if *share_type { need as u8 } else { eligible.len() as u8 },
                            cancellable: ctx.backup.is_some(),
                            hidden: false,
                            purpose: PickPurpose::Cost,
                        },
                    });
                    return Extra::Wait;
                };
                for id in chosen {
                    self.cur_src = Some(ctx.item.source);
                    self.move_card(id, Zone::Exile, Default::default());
                    ctx.item.remembered.push(Obj::card(id));
                }
                return Extra::Paid;
            }
            Cost::ExileTotalMvX { filter } => {
                let eligible = self.exile_mv_pool(filter, p, source);
                let x = ctx.item.x;
                let sum = |ids: &[CardId], g: &Game| ids.iter().map(|id| g.def_of(*id).mana_value as i32).sum::<i32>();
                if sum(&eligible, self) < x || eligible.is_empty() {
                    return Extra::Abort;
                }
                let mut chosen: Option<Vec<CardId>> = None;
                if let Some(Choice::Pick { indices }) = answer.take() {
                    let picked: Vec<CardId> = indices.iter().filter_map(|i| eligible.get(*i).copied()).collect();
                    if !picked.is_empty() && sum(&picked, self) == x {
                        chosen = Some(picked);
                    }
                } else if eligible.len() == 1 && sum(&eligible, self) == x {
                    chosen = Some(eligible.clone());
                }
                let Some(chosen) = chosen else {
                    self.pending = Some(Decision {
                        player: p,
                        kind: DecisionKind::Pick {
                            prompt: format!("Choose permanents with total mana value {x} to exile for {}", ctx.what),
                            source: Some(source),
                            options: eligible.iter().map(|id| Opt::Obj { obj: Obj::card(*id) }).collect(),
                            min: 1,
                            max: eligible.len() as u8,
                            cancellable: ctx.backup.is_some(),
                            hidden: false,
                            purpose: PickPurpose::Cost,
                        },
                    });
                    return Extra::Wait;
                };
                for id in chosen {
                    self.cur_src = Some(ctx.item.source);
                    self.move_card(id, Zone::Exile, Default::default());
                    ctx.item.remembered.push(Obj::card(id));
                }
                return Extra::Paid;
            }
            Cost::ExileGraveyardMv { filter, n } => {
                let eligible = self.gy_mv_pool(filter, p, source);
                if !self.gy_mv_ok(&eligible, *n) {
                    return Extra::Abort;
                }
                let mut chosen: Option<Vec<CardId>> = None;
                if let Some(Choice::Pick { indices }) = answer.take() {
                    let picked: Vec<CardId> = indices.iter().filter_map(|i| eligible.get(*i).copied()).collect();
                    if self.gy_mv_ok(&picked, *n) {
                        chosen = Some(picked);
                    }
                }
                let Some(chosen) = chosen else {
                    self.pending = Some(Decision {
                        player: p,
                        kind: DecisionKind::Pick {
                            prompt: format!("Choose cards from your graveyard with total mana value {n} or greater to exile for {}", ctx.what),
                            source: Some(source),
                            options: eligible.iter().map(|id| Opt::Obj { obj: Obj::card(*id) }).collect(),
                            min: 1,
                            max: eligible.len() as u8,
                            cancellable: ctx.backup.is_some(),
                            hidden: false,
                            purpose: PickPurpose::Cost,
                        },
                    });
                    return Extra::Wait;
                };
                for id in chosen {
                    self.cur_src = Some(ctx.item.source);
                    self.move_card(id, Zone::Exile, Default::default());
                    ctx.item.remembered.push(Obj::card(id));
                }
                return Extra::Paid;
            }
            Cost::ExileFromSingleGraveyard { filter, n, bottom } => {
                let pools = self.single_graveyard_pools(filter, source, p, *n);
                if pools.is_empty() {
                    return Extra::Abort;
                }
                let eligible: Vec<CardId> = pools.concat();
                let mut chosen: Option<Vec<CardId>> = None;
                if let Some(Choice::Pick { indices }) = answer.take() {
                    let picked: Vec<CardId> = indices.iter().filter_map(|i| eligible.get(*i).copied()).collect();
                    if picked.len() == *n as usize && pools.iter().any(|pl| picked.iter().all(|c| pl.contains(c))) {
                        chosen = Some(picked);
                    }
                } else if pools.len() == 1 && pools[0].len() == *n as usize {
                    chosen = Some(pools[0].clone());
                }
                let Some(chosen) = chosen else {
                    self.pending = Some(Decision {
                        player: p,
                        kind: DecisionKind::Pick {
                            prompt: format!("Choose {n} cards from a single graveyard to exile for {}", ctx.what),
                            source: Some(source),
                            options: eligible.iter().map(|id| Opt::Obj { obj: Obj::card(*id) }).collect(),
                            min: *n as u8,
                            max: *n as u8,
                            cancellable: ctx.backup.is_some(),
                            hidden: false,
                            purpose: PickPurpose::Cost,
                        },
                    });
                    return Extra::Wait;
                };
                for id in chosen {
                    if *bottom {
                        self.move_card(id, Zone::Library, crate::zones::MoveOpts { bottom: true, ..Default::default() });
                    } else {
                        self.cur_src = Some(ctx.item.source);
                        self.move_card(id, Zone::Exile, Default::default());
                    }
                    ctx.item.remembered.push(Obj::card(id));
                }
                return Extra::Paid;
            }
            Cost::Or { options } => {
                // pick which alternative to pay, then replace this part with it
                if let Some(Choice::Pick { indices }) = answer.take() {
                    let i = indices.first().copied().unwrap_or(0).min(options.len() - 1);
                    let chosen = options[i].clone();
                    ctx.extra.remove(ctx.extra_idx);
                    // mana in the chosen alternative joins the mana still to be paid (Crystal Shard: {3},{T} or {U},{T})
                    let mut k = 0;
                    for c in chosen.into_iter() {
                        if let Cost::Mana { cost } = &c {
                            if let Ok(mut mc) = ManaCost::parse(cost) {
                                if self.cards[source.0 as usize].zone == Zone::Stack {
                                    self.reduce_extra_mana(p, source, &mut mc);
                                }
                                ctx.mana.shards.extend(mc.shards.clone());
                                ctx.remaining.shards.extend(mc.shards);
                                continue;
                            }
                        }
                        ctx.extra.insert(ctx.extra_idx + k, c);
                        k += 1;
                    }
                    return Extra::Retry;
                }
                self.pending = Some(Decision {
                    player: p,
                    kind: DecisionKind::Pick {
                        prompt: format!("Choose how to pay for {}", ctx.what),
                        source: Some(source),
                        options: options.iter().map(|o| Opt::Text { text: o.iter().map(crate::describe::cost_text).collect::<Vec<_>>().join(", ") }).collect(),
                        min: 1,
                        max: 1,
                        cancellable: ctx.backup.is_some(),
                        hidden: false,
                        purpose: PickPurpose::Cost,
                    },
                });
                return Extra::Wait;
            }
            _ => {
                // deterministic costs are paid at the end (Finish)
                return Extra::Paid;
            }
        };
        let mut f = filter.clone();
        f.controller = PlayerFilter::Any;
        let eligible: Vec<CardId> = match zone {
            Zone::Battlefield => {
              let mut v: Vec<CardId> = self
                .battlefield
                .iter()
                .copied()
                .filter(|id| {
                    let c = &self.cards[id.0 as usize];
                    (c.cur.controller == p) != matches!(part, Cost::UntapPermanents { opponent: true, .. })
                        && self.matches_card(&f, *id, source, p)
                        && (kind != "tap" || (!c.tapped && self.can_become_tapped(*id, false)))
                        && (kind != "untap" || c.tapped)
                        && (kind != "rm_counter" || matches!(part, Cost::RemoveCountersFrom { kind: ck, n: cn, .. } if c.counter(*ck) >= *cn))
                        && !(kind == "sacrifice" && ctx.item.targets.iter().any(|t| t.obj == Obj::card(*id)) && false)
                })
                .collect();
              // a suspended card (or other exiled card holding the counters) can pay a `suspended` RemoveCountersFrom
              if let Cost::RemoveCountersFrom { kind: ck, n: cn, suspended: true, .. } = part {
                  v.extend(self.players[p as usize].exile.iter().copied().filter(|id| self.cards[id.0 as usize].counter(*ck) >= *cn));
              }
              v
            }
            Zone::Hand => self.players[p as usize].hand.iter().copied().filter(|id| self.matches_card(&f, *id, source, p) && *id != source).collect(),
            Zone::Graveyard => self.players[p as usize].graveyard.iter().copied().filter(|id| self.matches_card(&f, *id, source, p)).collect(),
            Zone::Exile => (0..self.cards.len() as u32).map(CardId).filter(|id| self.cards[id.0 as usize].zone == Zone::Exile && self.matches_card(&f, *id, source, p)).collect(),
            Zone::Stack => self.stack_spells_for_cost(p, source, &f),
            _ => vec![],
        };
        if (eligible.len() as i32) < n {
            return Extra::Abort;
        }
        let chosen: Vec<CardId> = if random_cost {
            let mut pool = eligible.clone();
            let mut out = vec![];
            for _ in 0..n.max(0) {
                if pool.is_empty() {
                    break;
                }
                let i = rand::Rng::gen_range(&mut self.rng, 0..pool.len());
                out.push(pool.remove(i));
            }
            out
        } else if let Some(Choice::Pick { indices }) = answer.take() {
            indices.iter().filter_map(|i| eligible.get(*i).copied()).collect()
        } else if eligible.len() as i32 == n {
            eligible.clone()
        } else {
            self.pending = Some(Decision {
                player: p,
                kind: DecisionKind::Pick {
                    prompt: format!("Choose {n} to {kind} for {}", ctx.what),
                    source: Some(source),
                    options: eligible.iter().map(|id| Opt::Obj { obj: Obj::card(*id) }).collect(),
                    min: n as u8,
                    max: n as u8,
                    cancellable: ctx.backup.is_some(),
                    hidden: zone == Zone::Hand,
                    purpose: PickPurpose::Cost,
                },
            });
            return Extra::Wait;
        };
        for id in chosen {
            // emerge / offering / harmonize: the chosen permanent lowers the mana still to pay
            match part {
                Cost::SacrificeReduceByManaValue { .. } | Cost::SacrificeReduceByManaCost { .. } | Cost::TapReduceByPower { .. } => {
                    let mut shards_off: Vec<Shard> = vec![];
                    let mut generic_off: u32 = 0;
                    match part {
                        Cost::SacrificeReduceByManaValue { .. } => generic_off = self.def_of(id).cost.mana_value(0) as u32,
                        Cost::TapReduceByPower { .. } => generic_off = self.cards[id.0 as usize].cur.power.max(0) as u32,
                        _ => {
                            for s in &self.def_of(id).cost.shards {
                                match s {
                                    Shard::Generic { n } => generic_off += *n as u32,
                                    other => shards_off.push(other.clone()),
                                }
                            }
                        }
                    }
                    for m in [&mut ctx.mana, &mut ctx.remaining] {
                        let mut g = generic_off;
                        for so in &shards_off {
                            if let Some(pos) = m.shards.iter().position(|x| x == so) {
                                m.shards.remove(pos);
                            } else if g == 0 {
                                // an offered colored shard the cost doesn't have still pays for nothing
                            }
                        }
                        for sh in m.shards.iter_mut() {
                            if let Shard::Generic { n } = sh {
                                let r = (*n as u32).min(g);
                                *n -= r as u8;
                                g -= r;
                            }
                        }
                        m.shards.retain(|s| !matches!(s, Shard::Generic { n: 0 }));
                    }
                }
                _ => {}
            }
            // a creature given up for emerge can still be tapped for mana first (601.2g comes before the costs of 601.2h are paid): do so when the rest of
            // the payment needs that mana (Llanowar Elves for Decimator of the Provinces)
            if kind == "sacrifice" && matches!(part, Cost::SacrificeReduceByManaValue { .. } | Cost::SacrificeReduceByManaCost { .. }) && !ctx.remaining.is_empty() {
                let without = {
                    let mut g2 = self.clone();
                    g2.cards[id.0 as usize].tapped = true;
                    g2.can_afford(p, &ctx.remaining, 0)
                };
                if !without {
                    if let Some(src) = self.mana_sources(p).into_iter().find(|m| m.card == id) {
                        let _ = self.activate_mana_ability(p, id, src.ability, None);
                    }
                }
            }
            match kind {
                "sacrifice" => {
                    self.emerge_sac.set(matches!(part, Cost::SacrificeReduceByManaValue { .. }));
                    self.emit(Event::Sacrificed { card: id, player: p });
                    self.emerge_sac.set(false);
                    self.move_card(id, Zone::Graveyard, Default::default());
                    // "the sacrificed creature" (Jarad): its last-known information travels with the ability
                    ctx.item.ctx.obj = Some(Obj::card(id));
                    ctx.item.remembered.push(Obj::card(id));
                    ctx.item.ctx.lki = self.last_lki.clone().filter(|l| l.card == id).map(Box::new);
                    if matches!(part, Cost::SacrificeReduceByManaValue { .. }) {
                        self.cards[source.0 as usize].emerge_toughness = ctx.item.ctx.lki.as_ref().map_or(0, |l| l.derived.toughness);
                    }
                }
                "discard" => {
                    self.move_card(id, self.discard_zone(id), Default::default());
                    self.emit(Event::Discarded { card: id, player: p, by: self.cause_ctl.get() });
                    ctx.item.remembered.push(Obj::card(id));
                }
                "exile" => {
                    self.cur_src = Some(ctx.item.source);
                    self.move_card(id, Zone::Exile, Default::default());
                    ctx.item.remembered.push(Obj::card(id));
                }
                "exile_pay" => {
                    // the exiled card's mana cost is paid on top (X counts as 0)
                    let shards: Vec<Shard> = self.def_of(id).cost.shards.iter().filter(|s| !matches!(s, Shard::X)).cloned().collect();
                    self.cur_src = Some(ctx.item.source);
                    self.move_card(id, Zone::Exile, Default::default());
                    ctx.item.remembered.push(Obj::card(id));
                    ctx.mana.shards.extend(shards.iter().cloned());
                    ctx.remaining.shards.extend(shards);
                }
                "exiled_to_gy" => {
                    self.move_card(id, Zone::Graveyard, Default::default());
                    ctx.item.remembered.push(Obj::card(id));
                }
                "exile_stack" => {
                    // the spell leaves the stack without resolving (not countered)
                    self.stack.retain(|s| !(s.source == id && matches!(s.kind, ItemKind::Spell { .. }) && !s.copy));
                    self.exile_marked.retain(|c| *c != id);
                    self.cur_src = Some(ctx.item.source);
                    self.move_card(id, Zone::Exile, Default::default());
                    ctx.item.remembered.push(Obj::card(id));
                }
                "to_library_top" => {
                    self.move_card(id, Zone::Library, Default::default());
                    ctx.item.remembered.push(Obj::card(id));
                }
                "to_bottom" => {
                    self.move_card(id, Zone::Library, crate::zones::MoveOpts { bottom: true, ..Default::default() });
                    ctx.item.remembered.push(Obj::card(id));
                }
                "reveal" => {
                    ctx.item.remembered.push(Obj::card(id));
                }
                "tap" => {
                    self.cards[id.0 as usize].tapped = true;
                    self.tap_cost_log.push((id, source, self.turn));
                    self.emit(Event::Tapped { card: id });
                    ctx.item.remembered.push(Obj::card(id));
                }
                "untap" => {
                    self.cards[id.0 as usize].tapped = false;
                    self.emit(Event::Untapped { card: id });
                    ctx.item.remembered.push(Obj::card(id));
                }
                "return" => {
                    self.move_card(id, Zone::Hand, Default::default());
                    ctx.item.remembered.push(Obj::card(id));
                }
                "exile_bf" => {
                    self.cur_src = Some(ctx.item.source);
                    // a craft ability's materials: the cost also exiles the crafted permanent itself
                    self.craft_exile.set(matches!(ctx.item.kind, ItemKind::Ability { .. }) && ctx.extra.iter().any(|c| matches!(c, Cost::ExileSelf)));
                    self.move_card(id, Zone::Exile, Default::default());
                    self.craft_exile.set(false);
                    ctx.item.remembered.push(Obj::card(id));
                }
                "put_counter" => {
                    if let Cost::PutCountersOnChosen { kind: ck, n: cn, .. } = part {
                        self.add_counters_to(Obj::card(id), *ck, *cn);
                    }
                    ctx.item.remembered.push(Obj::card(id));
                }
                "rm_counter" => {
                    if let Cost::RemoveCountersFrom { kind: ck, n: cn, .. } = part {
                        self.remove_counters_from(Obj::card(id), *ck, *cn);
                        // the last time counter came off a suspended card: it is cast without paying its mana cost (with haste, if a creature)
                        let c = &self.cards[id.0 as usize];
                        if *ck == CounterKind::Time && c.zone == Zone::Exile && c.counter(CounterKind::Time) == 0 && self.def_of(id).abilities.iter().any(|a| matches!(a, Ability::Suspend { .. })) {
                            if self.def_of(id).types.has(CardType::Creature) {
                                self.cards[id.0 as usize].free_haste = true;
                            }
                            self.free_casts.push((self.cards[id.0 as usize].owner, id, false, None));
                        }
                    }
                    ctx.item.remembered.push(Obj::card(id));
                }
                _ => {}
            }
        }
        Extra::Paid
    }

    /// Pays a cost that removes counters of any kind ("remove a counter from this", "remove a counter from a permanent you
    /// control", "remove X counters from a permanent you control"), one counter at a time. The player chooses the permanent
    /// when more than one can give a counter, and the kind of counter when the permanent has more than one kind.
    fn pay_any_kind_counters(&mut self, ctx: &mut CastCtx, part: &Cost, answer: &mut Option<Choice>) -> Extra {
        let p = ctx.player;
        let source = ctx.item.source;
        // `one`: every counter comes off the same permanent
        let (filter, one): (Option<Filter>, bool) = match part {
            Cost::RemoveAnyCounters { .. } => (None, true),
            Cost::RemoveAnyCountersFrom { filter, .. } => (Some(filter.clone().into_filter()), false),
            Cost::RemoveXCountersFromOne { filter } => (Some(filter.clone().into_filter()), true),
            _ => return Extra::Paid,
        };
        let (mut left, mut card) = ctx.rm_any.take().unwrap_or_else(|| match part {
            Cost::RemoveAnyCounters { n } => (*n, Some(source)),
            Cost::RemoveAnyCountersFrom { n, .. } => (*n, None),
            _ => (ctx.item.x, None),
        });
        loop {
            if left <= 0 {
                return Extra::Paid;
            }
            let id = match card {
                Some(id) => id,
                None => {
                    let mut f = filter.clone().unwrap_or_default();
                    f.controller = PlayerFilter::Any;
                    let need = if one { left } else { 1 };
                    let eligible: Vec<CardId> = self
                        .battlefield
                        .iter()
                        .copied()
                        .filter(|id| {
                            let c = &self.cards[id.0 as usize];
                            c.cur.controller == p && self.matches_card(&f, *id, source, p) && c.counters.iter().map(|(_, x)| *x as i32).sum::<i32>() >= need
                        })
                        .collect();
                    let picked = match answer.take() {
                        Some(Choice::Pick { indices }) => indices.first().and_then(|i| eligible.get(*i)).copied(),
                        _ => None,
                    };
                    let id = match picked {
                        Some(id) => id,
                        None if eligible.is_empty() => return Extra::Abort,
                        None if eligible.len() == 1 => eligible[0],
                        None => {
                            ctx.rm_any = Some((left, None));
                            self.pending = Some(Decision {
                                player: p,
                                kind: DecisionKind::Pick {
                                    prompt: if one { format!("Choose a permanent to remove {left} counter(s) from for {}", ctx.what) } else { format!("Choose a permanent to remove a counter from for {}", ctx.what) },
                                    source: Some(source),
                                    options: eligible.iter().map(|id| Opt::Obj { obj: Obj::card(*id) }).collect(),
                                    min: 1,
                                    max: 1,
                                    cancellable: ctx.backup.is_some(),
                                    hidden: false,
                                    purpose: PickPurpose::Cost,
                                },
                            });
                            return Extra::Wait;
                        }
                    };
                    if !ctx.item.remembered.contains(&Obj::card(id)) {
                        ctx.item.remembered.push(Obj::card(id));
                    }
                    card = Some(id);
                    id
                }
            };
            let kinds: Vec<CounterKind> = self.cards[id.0 as usize].counters.iter().filter(|(_, x)| *x > 0).map(|(k, _)| *k).collect();
            if kinds.is_empty() {
                return Extra::Abort;
            }
            let k = if kinds.len() == 1 {
                kinds[0]
            } else if let Some(Choice::Pick { indices }) = answer.take() {
                indices.first().and_then(|i| kinds.get(*i)).copied().unwrap_or(kinds[0])
            } else {
                ctx.rm_any = Some((left, card));
                self.pending = Some(Decision {
                    player: p,
                    kind: DecisionKind::Pick {
                        prompt: format!("Choose the kind of counter to remove from {} for {}", self.def_of(id).name, ctx.what),
                        source: Some(source),
                        options: kinds.iter().map(|k| Opt::Text { text: k.label() }).collect(),
                        min: 1,
                        max: 1,
                        cancellable: ctx.backup.is_some(),
                        hidden: false,
                        purpose: PickPurpose::Mode,
                    },
                });
                return Extra::Wait;
            };
            self.remove_counters_from(Obj::card(id), k, 1);
            left -= 1;
            if !one {
                card = None;
            }
        }
    }

    /// Does `p` control a permanent matching `filter` that holds at least `n` counters of `kind` (Cost::RemoveCountersFrom)?
    pub(crate) fn counters_removable_from(&self, filter: &Filter, kind: CounterKind, n: i32, among: bool, suspended: bool, source: CardId, p: PlayerId) -> bool {
        let pool = self.counter_pool(filter, kind, suspended, source, p);
        if among {
            pool.iter().map(|(_, k)| *k).sum::<i32>() >= n
        } else {
            pool.iter().any(|(_, k)| *k >= n)
        }
    }

    /// A permanent matching `filter` that is attached to `host` (Equipment / Aura / Fortification), if any.
    pub(crate) fn attached_matching(&self, host: CardId, filter: &Filter, p: PlayerId) -> Option<CardId> {
        let mut f = filter.clone();
        f.controller = PlayerFilter::Any;
        self.battlefield.iter().copied().find(|id| self.cards[id.0 as usize].attached_to == Some(host) && self.matches_card(&f, *id, host, p))
    }

    /// Behold: does `p` have a matching card in hand (other than the thing being cast) or a matching permanent on the battlefield?
    /// Greatest power among the creatures `p` controls and the warped creature cards `p` owns in exile (Close Encounter); None when there are none.
    pub(crate) fn greatest_creature_or_warped_power(&self, p: PlayerId) -> Option<i32> {
        let bf = self.battlefield.iter().filter(|id| self.cards[id.0 as usize].cur.controller == p && self.cards[id.0 as usize].cur.types.has(CardType::Creature)).map(|id| self.cards[id.0 as usize].cur.power);
        let ex = self
            .cards
            .iter()
            .filter(|c| c.zone == Zone::Exile && c.owner == p && c.exile_mark == Some(ExileMark::Warped) && self.db.def(c.def).types.has(CardType::Creature))
            .map(|c| self.db.def(c.def).power.unwrap_or(0));
        bf.chain(ex).max()
    }

    pub(crate) fn beholdable(&self, filter: &Filter, source: CardId, p: PlayerId, n: u8, choose_type: bool) -> bool {
        if choose_type {
            return !self.behold_types(filter, source, p, n).is_empty();
        }
        let mut f = filter.clone();
        f.controller = PlayerFilter::Any;
        let in_hand = self.players[p as usize].hand.iter().filter(|h| **h != source && self.matches_card(&f, **h, source, p)).count();
        let on_bf = self.battlefield.iter().filter(|id| self.cards[id.0 as usize].cur.controller == p && self.matches_card(&f, **id, source, p)).count();
        in_hand + on_bf >= (n.max(1) as usize)
    }

    /// Creature types `p` could choose for a `choose_type` Behold of `n`: at least `n` matching cards in hand (not the spell) or permanents they control share it (changelings share every type).
    pub(crate) fn behold_types(&self, filter: &Filter, source: CardId, p: PlayerId, n: u8) -> Vec<String> {
        let mut f = filter.clone();
        f.controller = PlayerFilter::Any;
        // (subtypes, changeling)
        let mut cands: Vec<(Vec<Sym>, bool)> = vec![];
        for h in self.players[p as usize].hand.iter().filter(|h| **h != source && self.matches_card(&f, **h, source, p)) {
            let d = self.def_of(*h);
            cands.push((d.subtypes.iter().copied().collect(), d.keywords.has(Keyword::Changeling)));
        }
        for id in self.battlefield.iter().filter(|id| self.cards[id.0 as usize].cur.controller == p && self.matches_card(&f, **id, source, p)) {
            let c = &self.cards[id.0 as usize];
            cands.push((c.cur.subtypes.iter().copied().collect(), c.cur.kw.has(Keyword::Changeling)));
        }
        let mut types: Vec<Sym> = vec![];
        for (st, _) in &cands {
            for t in st {
                if !types.contains(t) {
                    types.push(*t);
                }
            }
        }
        let mut out: Vec<String> = types.into_iter().filter(|t| cands.iter().filter(|(st, ch)| *ch || st.contains(t)).count() >= n.max(1) as usize).map(|t| t.name()).collect();
        out.sort();
        out
    }

    /// The permanents `p` controls that match `filter` and hold counters of `kind`, with how many.
    /// With `suspended`, the cards `p` owns in exile that hold counters of the kind count too (a suspended card's time counters: Rift Elemental).
    pub(crate) fn counter_pool(&self, filter: &Filter, kind: CounterKind, suspended: bool, source: CardId, p: PlayerId) -> Vec<(CardId, i32)> {
        let mut f = filter.clone();
        f.controller = PlayerFilter::Any;
        let mut out: Vec<(CardId, i32)> = self
            .battlefield
            .iter()
            .copied()
            .filter_map(|id| {
                let c = &self.cards[id.0 as usize];
                let k = c.counter(kind);
                (c.cur.controller == p && k > 0 && self.matches_card(&f, id, source, p)).then_some((id, k))
            })
            .collect();
        if suspended {
            out.extend(self.players[p as usize].exile.iter().copied().filter_map(|id| {
                let k = self.cards[id.0 as usize].counter(kind);
                (k > 0).then_some((id, k))
            }));
        }
        out
    }

    /// The most of these creatures that share one creature type (changelings share every type): Selective Snare's "creatures of the creature type of your choice".
    pub(crate) fn max_shared_creature_type(&self, ids: &[CardId]) -> usize {
        let is_ch = |id: &CardId| self.cards[id.0 as usize].cur.kw.has(Keyword::Changeling);
        let ch = ids.iter().filter(|id| is_ch(id)).count();
        let mut best = 0;
        for id in ids.iter().filter(|id| !is_ch(id)) {
            for t in self.cards[id.0 as usize].cur.subtypes.iter() {
                best = best.max(ids.iter().filter(|o| !is_ch(o) && self.cards[o.0 as usize].cur.subtypes.contains(t)).count() + ch);
            }
        }
        best.max(ch)
    }

    fn abort_cast(&mut self, mut ctx: Box<CastCtx>) -> Flow {
        if let Some(b) = ctx.backup.take() {
            let log = std::mem::take(&mut self.decision_log);
            let events = std::mem::take(&mut self.events);
            *self = *b;
            self.decision_log = log;
            // a cancelled "pay by hand" cast doesn't carry over to the next one
            for pl in self.players.iter_mut() {
                pl.manual_once = false;
            }
            self.events = events;
            self.run = Run::Priority { player: ctx.player, passes: 0 };
        } else if let Origin::Spell { card, .. } = ctx.origin {
            // put the card back where it came from
            let from = ctx.from;
            self.cards[card.0 as usize].zone = Zone::Stack;
            self.move_card(card, from, Default::default());
            self.run = Run::Priority { player: ctx.player, passes: 0 };
        } else {
            self.run = Run::Priority { player: ctx.player, passes: 0 };
        }
        Flow::Continue
    }

    /// Spend floating mana against what's still owed.
    pub fn spend_pool(&mut self, p: PlayerId, remaining: &mut ManaCost) {
        loop {
            let pool = self.players[p as usize].pool;
            if pool.total() == 0 {
                self.players[p as usize].pool_treasure = false;
            }
            if pool.total() == 0 || remaining.is_empty() {
                return;
            }
            // find a shard that the pool can pay: colored first, then hybrid, then generic
            let order = |s: &Shard| match s {
                Shard::Color { .. } | Shard::Colorless => 0,
                Shard::Hybrid { .. } | Shard::HybridColorless { .. } => 1,
                Shard::Generic { .. } | Shard::Snow => 2,
                _ => 3,
            };
            let mut idxs: Vec<usize> = (0..remaining.shards.len()).collect();
            idxs.sort_by_key(|i| order(&remaining.shards[*i]));
            let mut paid = false;
            for i in idxs {
                let s = remaining.shards[i];
                let pay_with: Option<ManaType> = match s {
                    Shard::Color { c } => (pool.get(ManaType::of(c)) > 0).then(|| ManaType::of(c)),
                    Shard::Colorless => (pool.get(ManaType::C) > 0).then_some(ManaType::C),
                    Shard::Hybrid { a, b } => [ManaType::of(a), ManaType::of(b)].into_iter().find(|t| pool.get(*t) > 0),
                    Shard::HybridColorless { c } => [ManaType::of(c), ManaType::C].into_iter().find(|t| pool.get(*t) > 0),
                    Shard::Phyrexian { c } => (pool.get(ManaType::of(c)) > 0).then(|| ManaType::of(c)),
                    Shard::PhyrexianHybrid { a, b } => [ManaType::of(a), ManaType::of(b)].into_iter().find(|t| pool.get(*t) > 0),
                    Shard::HybridGeneric { c } => (pool.get(ManaType::of(c)) > 0).then(|| ManaType::of(c)),
                    Shard::Generic { .. } | Shard::Snow => {
                        // spend the type least likely to be needed: colorless first, then colors the rest of the cost doesn't ask for
                        let needed = remaining.colors();
                        ManaType::ALL
                            .iter()
                            .copied()
                            .filter(|t| pool.get(*t) > 0)
                            .min_by_key(|t| match t.color() {
                                None => 0,
                                Some(c) if !needed.has(c) => 1,
                                _ => 2,
                            })
                    }
                    Shard::X => None,
                };
                if let Some(t) = pay_with {
                    self.players[p as usize].pool.take(t);
                    self.note_spent(t);
                    if self.players[p as usize].pool_treasure {
                        self.treasure_paid.set(true);
                        let mut st = self.src_track.get();
                        st[0] = st[0].saturating_add(1);
                        st[1] = st[1].saturating_add(1);
                        self.src_track.set(st);
                    }
                    match &mut remaining.shards[i] {
                        Shard::Generic { n } if *n > 1 => *n -= 1,
                        _ => {
                            remaining.shards.remove(i);
                        }
                    }
                    paid = true;
                    break;
                }
            }
            if !paid {
                return;
            }
        }
    }

    fn spend_specific(&mut self, remaining: &mut ManaCost, t: ManaType) {
        let pos = remaining.shards.iter().position(|s| match (s, t.color()) {
            (Shard::Color { c }, Some(tc)) => *c == tc,
            (Shard::Colorless, None) => true,
            (Shard::Hybrid { a, b }, Some(tc)) => *a == tc || *b == tc,
            (Shard::Phyrexian { c }, Some(tc)) => *c == tc,
            (Shard::PhyrexianHybrid { a, b }, Some(tc)) => *a == tc || *b == tc,
            (Shard::HybridGeneric { c }, Some(tc)) => *c == tc,
            (Shard::HybridColorless { c }, Some(tc)) => *c == tc,
            (Shard::HybridColorless { .. }, None) => true,
            _ => false,
        });
        match pos {
            Some(i) => {
                remaining.shards.remove(i);
            }
            None => {
                if let Some(i) = remaining.shards.iter().position(|s| matches!(s, Shard::Generic { .. })) {
                    match &mut remaining.shards[i] {
                        Shard::Generic { n } if *n > 1 => *n -= 1,
                        _ => {
                            remaining.shards.remove(i);
                        }
                    }
                }
            }
        }
    }

    /// Pay what's still owed with the cheapest plan: pool mana, then sources, then life for Phyrexian symbols.
    pub(crate) fn note_source_paid(&self, card: CardId) {
        if self.def_of(card).subtypes.iter().any(|s| *s == Sym::new("Treasure")) {
            self.treasure_paid.set(true);
        }
        let mut st = self.src_track.get();
        if self.def_of(card).subtypes.iter().any(|s| *s == Sym::new("Treasure")) {
            st[0] = st[0].saturating_add(1);
        }
        if self.cards[card.0 as usize].cur.types.has(CardType::Artifact) {
            st[1] = st[1].saturating_add(1);
        }
        if self.cards[card.0 as usize].cur.subtypes.iter().any(|s| *s == Sym::new("Cave")) {
            st[2] = st[2].saturating_add(1);
        }
        if self.cards[card.0 as usize].cur.subtypes.iter().any(|s| *s == Sym::new("Desert")) {
            st[3] = st[3].saturating_add(1);
        }
        if self.cards[card.0 as usize].cur.types.has(CardType::Creature) {
            st[4] = st[4].saturating_add(1);
        }
        self.src_track.set(st);
        self.src_cards_track.borrow_mut().push(card);
        if self.cards[card.0 as usize].cur.supertypes.contains(&Supertype::Snow) {
            self.snow_track.set(self.snow_track.get().saturating_add(1));
        }
    }

    /// The stack item is a spell with a `SpendOnlyManaFrom` restriction.
    fn spend_only_item(&self, item: &StackItem) -> bool {
        if matches!(item.kind, ItemKind::Ability { .. }) {
            return false;
        }
        self.spend_only_static(item.source).is_some()
    }

    /// Remember the color a snow source produced for the spell being paid (`Amount::SnowColorsMatching`).
    pub(crate) fn note_snow_color(&self, p: PlayerId, card: CardId, ability: u16, produce: Option<ManaType>) {
        if !self.cards[card.0 as usize].cur.supertypes.contains(&Supertype::Snow) {
            return;
        }
        let t = produce.or_else(|| self.mana_sources_for_cost(p, None).into_iter().find(|s| s.card == card && s.ability == ability).and_then(|s| s.produces.first().copied()));
        if let Some(col) = t.and_then(|t| t.color()) {
            self.snow_colors_track.set(self.snow_colors_track.get() | col.bit());
        }
    }

    pub(crate) fn note_spent(&self, t: ManaType) {
        let mut s = self.spent_track.get();
        s[t as usize] = s[t as usize].saturating_add(1);
        self.spent_track.set(s);
    }

    fn auto_pay(&mut self, p: PlayerId, ctx: &mut CastCtx) -> bool {
        // sources already used for {T} costs of this activation must not tap for mana: tap them first
        let pool = if self.spend_only_filter().is_some() { ManaPool::default() } else { self.players[p as usize].pool };
        let exclude: Vec<CardId> = match &ctx.origin {
            Origin::Ability { card, .. } if ctx.extra.iter().any(|c| matches!(c, Cost::Tap)) => vec![*card],
            _ => vec![],
        };
        let sources: Vec<ManaSourceOpt> = self.mana_sources_for_cost(p, Some(&ctx.remaining)).into_iter().filter(|s| !exclude.contains(&s.card)).collect();
        let life = if self.can_pay_life(p, 1) { self.players[p as usize].life } else { 0 };
        let Some(plan) = plan_payment_with(&sources, &self.spend_as_cost(p, &ctx.remaining), 0, pool, life) else { return false };
        for u in plan.uses {
            match u {
                PayUse::Pool(t) => {
                    self.players[p as usize].pool.take(t);
                    self.note_spent(t);
                }
                PayUse::Source { card, ability, produce } => {
                    self.note_source_paid(card);
                    self.note_snow_color(p, card, ability, Some(produce));
                    // the whole output floats; the plan's following `Pool` uses spend it
                    self.paying_activation.set(true);
                    let r = self.activate_mana_ability(p, card, ability, Some(produce));
                    self.paying_activation.set(false);
                    if r.is_err() {
                        return false;
                    }
                }
            }
        }
        ctx.paid_life += plan.life;
        ctx.remaining = ManaCost::default();
        true
    }

    /// Ward: the spell or ability on top of the stack is countered unless its controller pays the ward cost of each warded
    /// permanent it targets (an opponent's permanent). Paid automatically when possible.
    fn apply_ward(&mut self) {
        let Some(item) = self.stack.last().cloned() else { return };
        self.emit(Event::StackPut { id: item.id, targets: item.targets.len().min(255) as u8 });
        // a crime: the spell or ability targets an opponent, an opponent's permanent or a card in an opponent's graveyard
        let crime = item.targets.iter().any(|t| match t.obj {
            Obj::Player { id } => id != item.controller,
            Obj::Card { id } => {
                let c = &self.cards[id.0 as usize];
                match c.zone {
                    Zone::Battlefield => c.cur.controller != item.controller,
                    Zone::Graveyard => c.owner != item.controller,
                    _ => false,
                }
            }
            _ => false,
        });
        if crime && !matches!(item.kind, ItemKind::Delayed { .. }) {
            self.players[item.controller as usize].crime = true;
            self.emit(Event::Crime { player: item.controller });
        }
        for t in &item.targets {
            match t.obj {
                Obj::Card { id } => self.emit(Event::Targeted { card: id, by: item.source, controller: item.controller, spell: matches!(item.kind, ItemKind::Spell { .. }), sid: item.id }),
                Obj::Player { id } => self.emit(Event::PlayerTargeted { player: id, by: item.source, controller: item.controller, spell: matches!(item.kind, ItemKind::Spell { .. }), sid: item.id }),
                // a spell on the stack becoming a target ("whenever a creature spell you control becomes the target of a spell or ability")
                Obj::Stack { id: sid } => {
                    if let Some(si) = self.stack.iter().find(|s| s.id == sid) {
                        if matches!(si.kind, ItemKind::Spell { .. }) {
                            let card = si.source;
                            self.emit(Event::Targeted { card, by: item.source, controller: item.controller, spell: matches!(item.kind, ItemKind::Spell { .. }), sid: item.id });
                        }
                    }
                }
                _ => {}
            }
        }
        let mut wards: Vec<(CardId, Vec<Cost>)> = vec![];
        for t in &item.targets {
            let Obj::Card { id } = t.obj else { continue };
            let c = &self.cards[id.0 as usize];
            if c.zone != Zone::Battlefield || c.cur.controller == item.controller || self.hexproof_ignored(item.controller, Obj::card(id), true) {
                continue;
            }
            let d = self.db.def(c.def);
            // a face-down Disguise permanent has ward {2} though it has no other abilities
            if c.face_down && (c.cloaked || d.abilities.iter().any(|a| matches!(a, Ability::Morph { disguise: true, .. }))) {
                wards.push((id, vec![Cost::Mana { cost: "{2}".into() }]));
                continue;
            }
            if c.cur.no_abilities || c.cur.no_ward {
                continue;
            }
            for a in d.abilities.iter().chain(c.cur.granted.iter().map(|g| &**g)) {
                if let Ability::Ward { cost } = a {
                    wards.push((id, cost.clone()));
                }
            }
        }
        for (ward_src, cost) in wards {
            // a ward cost that scales with the warded permanent ("pay life equal to its power") reads that permanent
            let cost: Vec<Cost> = cost
                .into_iter()
                .map(|c| match c {
                    Cost::PayLifeX { amount } => Cost::PayLife { n: self.life_cost(ward_src, &amount) },
                    // "ward {X}, where X is the number of experience counters you have": counted for the warded permanent's controller (Minthara)
                    Cost::ManaAmount { amount } => Cost::Mana { cost: format!("{{{}}}", self.eval_amount(&amount, &EvalCtx::of(self, ward_src)).max(0)) },
                    o => o,
                })
                .collect();
            if !self.try_pay_costs(item.controller, item.source, &cost) {
                // a spell that can't be countered stays on the stack even when its ward cost isn't paid
                if matches!(item.kind, ItemKind::Spell { .. }) && !item.copy && self.spell_uncounterable(item.source, None) {
                    continue;
                }
                self.stack.pop();
                if matches!(item.kind, ItemKind::Spell { .. }) {
                    if !item.copy {
                        self.move_card(item.source, Zone::Graveyard, Default::default());
                    }
                    self.emit(Event::Countered { card: item.source, by: None });
                }
                self.log("ward: the spell or ability was countered");
                return;
            }
        }
    }

    fn finish_cast(&mut self, ctx: Box<CastCtx>) -> Flow {
        let p = ctx.player;
        let CastCtx { origin, mut item, extra, paid_life, .. } = *ctx;
        let mut life_paid = paid_life;
        // pay the deterministic non-mana costs
        let source = item.source;
        if matches!(origin, Origin::Spell { .. }) {
            let noted = self.def_of(source).abilities.iter().find_map(|a| if let Ability::NoteOnCast { amount } = a { Some(amount.clone()) } else { None });
            if let Some(amount) = noted {
                let n = self.eval_amount(&amount, &EvalCtx::with_controller(source, p));
                self.cards[source.0 as usize].chosen_number = Some(n);
            }
        }
        let spent = self.spent_track.replace([0; 6]);
        let snow = self.snow_track.replace(0);
        let snow_colors = self.snow_colors_track.replace(0);
        let src = self.src_track.replace([0; 5]);
        let src_cards = std::mem::take(&mut *self.src_cards_track.borrow_mut());
        let from_treasure = self.treasure_paid.replace(false);
        let mut riders = std::mem::take(&mut *self.pay_riders.borrow_mut());
        // rider mana produced while paying but left unspent (Mana Reflection doubling Path of Ancestry) floats on keeping its rider; the spell only gets
        // the rider once per unit of mana actually spent
        let units = std::mem::take(&mut *self.pay_rider_units.borrow_mut());
        let spent_units: usize = spent.iter().map(|&n| n as usize).sum();
        if !units.is_empty() && units.len() > spent_units && riders.len() >= units.len() {
            for (rider, t, spell) in units.into_iter().skip(spent_units) {
                if self.players[p as usize].pool.take(t) {
                    if let Some(i) = riders.iter().rposition(|r| *r == rider) {
                        riders.remove(i);
                    }
                    let r = ManaRestrict { spell: Some((Filter::default()).into()), ability: Some((Filter::default()).into()), rider: Some(rider), cost_c: false, upkeep: false, unlock: false, no_generic: false, rider_spell: spell };
                    let pl = &mut self.players[p as usize];
                    pl.restricted.push(Some((t, r, source)));
                    pl.restricted_turn.resize(pl.restricted.len() - 1, false);
                    pl.restricted_turn.push(false);
                }
            }
        }
        let mut rider_srcs = std::mem::take(&mut *self.pay_rider_srcs.borrow_mut());
        let mut rider_effs: Vec<Effect> = vec![];
        let ability_copies = if matches!(item.kind, ItemKind::Ability { .. }) { riders.iter().filter(|r| **r == crate::dsl::ManaRider::CopySpell).count() } else { 0 };
        if matches!(item.kind, ItemKind::Ability { .. }) {
            // "if mana from a Treasure was spent to activate this ability" reads the source's flag while the ability resolves
            self.cards[source.0 as usize].mana_from_treasure = from_treasure;
            self.cards[source.0 as usize].act_spent = spent;
            self.cards[source.0 as usize].src_spent = src;
        }
        if matches!(item.kind, ItemKind::Spell { .. }) {
            self.cards[source.0 as usize].mana_spent = spent;
            self.cards[source.0 as usize].snow_spent = snow;
            self.cards[source.0 as usize].snow_colors = snow_colors;
            self.cards[source.0 as usize].src_spent = src;
            self.cards[source.0 as usize].paid_src = src_cards;
            self.cards[source.0 as usize].mana_from_treasure = from_treasure;
            // mana with a rider was spent on this spell
            for r in riders {
                match r {
                    crate::dsl::ManaRider::CantBeCountered => self.add_effect(source, p, &[source], Modif::Keyword(Keyword::CantBeCountered), Duration::UntilEndOfTurn),
                    crate::dsl::ManaRider::Haste => self.add_effect(source, p, &[source], Modif::Keyword(Keyword::Haste), Duration::UntilEndOfTurn),
                    crate::dsl::ManaRider::HasteForever => self.add_effect(source, p, &[source], Modif::Keyword(Keyword::Haste), Duration::Forever),
                    crate::dsl::ManaRider::PlusOneCounter => self.cards[source.0 as usize].rider_counters = self.cards[source.0 as usize].rider_counters.saturating_add(1),
                    crate::dsl::ManaRider::PlusOneCounterHexproof => {
                        self.cards[source.0 as usize].rider_counters = self.cards[source.0 as usize].rider_counters.saturating_add(1);
                        self.add_effect(source, p, &[source], Modif::Keyword(Keyword::Hexproof), Duration::UntilYourNextTurn);
                    }
                    crate::dsl::ManaRider::PlusCountersCommanderCasts => {
                        let prior = self.players[p as usize].commander_cast.iter().find(|(id, _)| *id == source).map_or(0, |(_, n)| *n);
                        let now = prior + (self.cards[source.0 as usize].cast_from == Some(Zone::Command)) as u32;
                        self.cards[source.0 as usize].rider_counters = self.cards[source.0 as usize].rider_counters.saturating_add(now.min(255) as u8);
                    }
                    crate::dsl::ManaRider::Scry(n) => rider_effs.push(Effect::Scry { who: Sel::You, n: Amount::Fixed { n: n as i32 }, decider: None }),
                    crate::dsl::ManaRider::Draw(n) => rider_effs.push(Effect::Draw { who: Sel::You, n: Amount::Fixed { n: n as i32 }, plain: false }),
                    crate::dsl::ManaRider::GainLife(n) => rider_effs.push(Effect::GainLife { who: Sel::You, n: Amount::Fixed { n: n as i32 } }),
                    crate::dsl::ManaRider::ScryCommanderCasts => rider_effs.push(Effect::Scry { who: Sel::You, n: Amount::CommanderCasts, decider: None }),
                    crate::dsl::ManaRider::ExileSpell => {
                        if let Some(src) = rider_srcs.pop() {
                            self.cards[source.0 as usize].exile_link = Some(src);
                            rider_effs.push(Effect::May { effects: vec![Effect::ExileSpellOnResolve], otherwise: vec![], who: None, remember_answer: false });
                        }
                    }
                    crate::dsl::ManaRider::Riot => {
                        if self.def_of(source).types.has(CardType::Creature) {
                            self.cards[source.0 as usize].riot_rider = true;
                        }
                    }
                    crate::dsl::ManaRider::CopySpell => rider_effs.push(Effect::CopySpell { what: Sel::TriggerObject, new_targets: true, by: None, except: vec![], target: None, each_other_target: false, for_opponent: None, even_if_gone: self.copy_rider_always.get() }),
                }
            }
        }
        let total: u32 = spent.iter().map(|&n| n as u32).sum();
        if total > 0 && matches!(item.kind, ItemKind::Spell { .. }) {
            let before = self.players[p as usize].mana_spent_turn;
            self.players[p as usize].mana_spent_turn = before + total;
            for n in before + 1..=(before + total).min(before + 30) {
                self.emit(Event::Expended { player: p, n });
            }
        }
        if matches!(origin, Origin::Spell { .. }) {
            self.cards[item.source.0 as usize].phyrexian_life = (paid_life / 2).clamp(0, 255) as u8;
        }
        if paid_life > 0 {
            self.lose_life(p, paid_life);
        }
        if matches!(origin, Origin::Spell { .. }) && !item.targets.is_empty() {
            let extra_life = self.targeting_spell_life(p, &item.targets);
            if extra_life > 0 {
                self.pay_life(p, extra_life);
                life_paid += extra_life;
            }
        }
        for part in &extra {
            match part {
                Cost::Tap => {
                    self.cards[source.0 as usize].tapped = true;
                    self.emit(Event::Tapped { card: source });
                }
                Cost::Untap => {
                    self.cards[source.0 as usize].tapped = false;
                    self.emit(Event::Untapped { card: source });
                }
                Cost::PayLife { n } => {
                    self.pay_life(p, *n);
                    life_paid += *n;
                }
                Cost::PayLifeX { amount } => {
                    let n = self.life_cost(source, amount);
                    self.pay_life(p, n);
                    life_paid += n;
                }
                Cost::SacrificeSelf => {
                    self.emit(Event::Sacrificed { card: source, player: p });
                    self.move_card(source, Zone::Graveyard, Default::default());
                    // the ability uses the permanent's last-known information (counters, power ...)
                    item.ctx.lki = self.last_lki.clone().filter(|l| l.card == source).map(Box::new);
                }
                Cost::RemoveAllCountersGrantSource { kind } => {
                    if let Some(g) = item.ctx.granted_by {
                        let have: Vec<(CounterKind, u16)> = self.cards[g.0 as usize].counters.iter().copied().collect();
                        item.x = 0;
                        for (k, n) in have {
                            if kind.map_or(true, |w| w == k) {
                                item.x += n as i32;
                                self.cards[g.0 as usize].remove_counters(k, n as i32);
                            }
                        }
                        self.layers_dirty = true;
                    }
                }
                Cost::SacrificeGrantSource => {
                    if let Some(g) = item.ctx.granted_by {
                        self.emit(Event::Sacrificed { card: g, player: p });
                        self.move_card(g, Zone::Graveyard, Default::default());
                    }
                }
                Cost::ExileGrantSource => {
                    if let Some(g) = item.ctx.granted_by {
                        self.move_card(g, Zone::Exile, Default::default());
                    }
                }
                Cost::DiscardSelf => {
                    let is_cycling = self.matches_card(&Filter { has_keyword_word: Some("cycling".into()), ..Default::default() }, source, source, p);
                    self.cycling_move.set(is_cycling);
                    self.move_card(source, Zone::Graveyard, Default::default());
                    self.cycling_move.set(false);
                    self.emit(Event::Discarded { card: source, player: p, by: self.cause_ctl.get() });
                    if is_cycling {
                        self.players[p as usize].cycled_turn += 1;
                    }
                    self.emit(Event::Cycled { card: source, player: p });
                }
                Cost::ExileSelf => {
                    // a graveyard card with a power/toughness CDA (Boneyard Mycodrax): its P/T as it last existed travels with the ability
                    let cda = if self.cards[source.0 as usize].zone == Zone::Graveyard {
                        self.def_of(source).abilities.iter().find_map(|a| match a {
                            Ability::Static { effect: StaticEffect::CdaPT { power, toughness }, .. } => Some((power.clone(), toughness.clone())),
                            _ => None,
                        })
                    } else {
                        None
                    };
                    let cda = cda.map(|(pw, tg)| {
                        let ec = EvalCtx::of(self, source);
                        (self.eval_amount(&pw, &ec), self.eval_amount(&tg, &ec))
                    });
                    self.move_card(source, Zone::Exile, Default::default());
                    if let Some((pw, tg)) = cda {
                        let c = &self.cards[source.0 as usize];
                        let mut derived = c.cur.clone();
                        derived.power = pw;
                        derived.toughness = tg;
                        item.ctx.lki = Some(Box::new(Lki { card: source, derived, controller: c.controller, counters: c.counters.clone(), def: c.def, token: c.token, face_down: false, attached_to: None, attachments: Default::default(), snapshot: Vec::new() }));
                    }
                }
                Cost::PutSelfOnLibraryBottom => {
                    self.move_card(source, Zone::Library, crate::zones::MoveOpts { bottom: true, ..Default::default() });
                }
                Cost::CollectEvidence { n } => {
                    if let Some(cards) = self.evidence_cards(p, *n, source) {
                        // the exiled cards are "exiled with" the source (Kylox's Voltstrider)
                        let prev = self.cur_src.replace(source);
                        for id in cards {
                            self.move_card(id, Zone::Exile, Default::default());
                        }
                        self.cur_src = prev;
                        self.emit(Event::EvidenceCollected { player: p });
                    }
                }
                Cost::RemoveCounters { kind, n } => {
                    self.cards[source.0 as usize].remove_counters(*kind, *n);
                    self.layers_dirty = true;
                }
                // paid in the Extra stage (`pay_any_kind_counters`)
                Cost::RemoveAnyCounters { .. } | Cost::RemoveAnyCountersFrom { .. } => {}
                Cost::PutCounters { kind, n } => {
                    self.cards[source.0 as usize].add_counters(*kind, *n);
                    self.layers_dirty = true;
                }
                Cost::Crew { power } => {
                    let mut crew: Vec<CardId> = self
                        .battlefield
                        .iter()
                        .copied()
                        .filter(|id| *id != source && self.cards[id.0 as usize].cur.controller == p && !self.cards[id.0 as usize].tapped && self.can_become_tapped(*id, false) && self.cards[id.0 as usize].cur.types.has(CardType::Creature))
                        .collect();
                    crew.sort_by_key(|id| -self.crew_value(*id, source));
                    let mut total = 0;
                    for id in crew {
                        if total >= self.crew_requirement(source, *power) {
                            break;
                        }
                        total += self.crew_value(id, source);
                        self.crew_log.push((id, source, self.turn));
                        self.emit(Event::CrewPaid { creature: id, vehicle: source, saddle: false });
                        self.cards[id.0 as usize].tapped = true;
                        self.emit(Event::Tapped { card: id });
                    }
                    self.emit(Event::Crewed { card: source, player: p });
                }
                Cost::UnattachSelf => {
                    self.attach(source, None);
                }
                Cost::ExertSelf => {
                    if self.cards[source.0 as usize].zone == Zone::Battlefield {
                        self.cards[source.0 as usize].skip_untap = true;
                        self.cards[source.0 as usize].skip_untap_by = Some(self.cards[source.0 as usize].cur.controller);
                        self.emit(Event::Exerted { card: source });
                    }
                }
                Cost::Behold { filter, .. } => {
                    let mut f = filter.clone();
                    f.controller = PlayerFilter::Any;
                    let hand: Vec<CardId> = self.players[p as usize].hand.iter().copied().filter(|h| *h != source && self.matches_card(&f, *h, source, p)).collect();
                    let bf: Vec<CardId> = self.battlefield.iter().copied().filter(|id| self.cards[id.0 as usize].cur.controller == p && self.matches_card(&f, *id, source, p)).collect();
                    let best = hand
                        .iter()
                        .map(|h| self.def_of(*h).power.unwrap_or(0))
                        .chain(bf.iter().map(|id| self.cards[id.0 as usize].cur.power))
                        .max();
                    self.cards[source.0 as usize].beheld_power = best;
                }
                Cost::ChooseCreatureOrWarped => {
                    self.cards[source.0 as usize].beheld_power = self.greatest_creature_or_warped_power(p);
                }
                Cost::UnattachAttached { filter } => {
                    let mut f = filter.clone();
                    f.controller = PlayerFilter::Any;
                    let chosen = item.remembered.iter().find_map(|o| match o {
                        Obj::Card { id } if self.cards[id.0 as usize].attached_to == Some(source) && self.matches_card(&f, *id, source, p) => Some(*id),
                        _ => None,
                    });
                    if let Some(a) = chosen.or_else(|| self.attached_matching(source, filter, p)) {
                        self.attach(a, None);
                    }
                }
                Cost::PayEnergy { n } => {
                    self.players[p as usize].energy -= *n;
                    self.players[p as usize].energy_spent += *n;
                }
                Cost::RemoveLoyaltyX => {
                    self.cards[source.0 as usize].remove_counters(CounterKind::Loyalty, item.x);
                    self.layers_dirty = true;
                }
                Cost::RemoveAllCounters { kind } => {
                    let have: Vec<(CounterKind, u16)> = self.cards[source.0 as usize].counters.iter().copied().collect();
                    item.x = 0;
                    for (k, n) in have {
                        if kind.map_or(true, |w| w == k) {
                            item.x += n as i32;
                            self.cards[source.0 as usize].remove_counters(k, n as i32);
                        }
                    }
                    self.layers_dirty = true;
                }
                Cost::RemoveCountersX { kind } => {
                    self.cards[source.0 as usize].remove_counters(*kind, item.x);
                    self.layers_dirty = true;
                }
                Cost::RollDie { sides } => {
                    use rand::Rng;
                    let forced = {
                        let mut f = self.forced_rolls.borrow_mut();
                        if f.is_empty() { None } else { Some(f.remove(0)) }
                    };
                    let mut roll = forced.map(|r| r.clamp(1, (*sides).max(1))).unwrap_or_else(|| self.rng.gen_range(1..=(*sides).max(1)));
                    self.log(&format!("Rolled a {roll} on a d{sides}"));
                    self.emit(Event::DieRolled { player: p, n: roll, sides: *sides });
                    // Barbarian Class: the extra dice are rolled too, only the highest counts
                    for _ in 0..self.extra_roll_dice(p) {
                        let forced = {
                            let mut f = self.forced_rolls.borrow_mut();
                            if f.is_empty() { None } else { Some(f.remove(0)) }
                        };
                        let r = forced.map(|r| r.clamp(1, (*sides).max(1))).unwrap_or_else(|| self.rng.gen_range(1..=(*sides).max(1)));
                        self.log(&format!("Rolled a {r} on a d{sides}"));
                        self.emit(Event::DieRolled { player: p, n: r, sides: *sides });
                        roll = roll.max(r);
                    }
                    self.last_die = roll;
                    item.x = roll;
                    self.emit(Event::DiceRolled { player: p });
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
                        if let Some(id) = self.players[p as usize].library.pop() {
                            self.cards[id.0 as usize].zone = Zone::Library;
                            self.players[p as usize].library.push(id);
                            self.move_card(id, Zone::Exile, Default::default());
                            item.remembered.push(Obj::card(id));
                        }
                    }
                }
                Cost::Mill { n } => {
                    self.cost_milled.clear();
                    for _ in 0..*n {
                        if let Some(id) = self.players[p as usize].library.pop() {
                            self.cards[id.0 as usize].zone = Zone::Library;
                            self.players[p as usize].library.push(id);
                            self.cost_milled.push(id);
                            self.move_card(id, Zone::Graveyard, Default::default());
                        }
                    }
                }
                _ => {}
            }
        }
        self.recompute_layers();
        let ts = self.next_ts();
        let _ = ts;
        match origin {
            Origin::Spell { card, alt } => {
                if self.cards[card.0 as usize].is_commander && self.cards[card.0 as usize].cast_from == Some(Zone::Command) {
                    let pl = &mut self.players[p as usize];
                    if let Some(e) = pl.commander_cast.iter_mut().find(|(id, _)| *id == card) {
                        e.1 += 1;
                    } else {
                        pl.commander_cast.push((card, 1));
                    }
                }
                self.players[p as usize].spells_cast += 1;
                self.players[p as usize].spell_cast_turn = self.turn;
                self.players[p as usize].spells_cast_game += 1;
                // statics conditioned on spells cast this turn (Prahv: "if you cast a spell this turn, you can't attack")
                self.layers_dirty = true;
                if alt == Some(FACE_DOWN_ALT) {
                    self.players[p as usize].face_down_cast_turn = self.turn;
                }
                // North Star: the one-spell allowance is used up by a spell that has colored or colorless mana symbols
                if self.players[p as usize].any_type_turn == self.turn && self.db.def(self.cards[card.0 as usize].def).cost.shards.iter().any(|s| matches!(s, Shard::Color { .. } | Shard::Colorless | Shard::Hybrid { .. })) {
                    self.players[p as usize].any_type_turn = 0;
                }
                if self.active == p {
                    self.players[p as usize].act_turn = self.turn;
                }
                if self.cards[card.0 as usize].cast_from.map_or(false, |z| z != Zone::Hand) {
                    self.players[p as usize].nonhand_turn = self.turn;
                }
                self.spells_this_turn += 1;
                self.spells_log.push((p, card));
                if self.cards[card.0 as usize].cast_from == Some(Zone::Graveyard) {
                    self.graveyard_use_log.push(p);
                    if self.cards[card.0 as usize].cur.types.has(CardType::Instant) || self.cards[card.0 as usize].cur.types.has(CardType::Sorcery) {
                        self.gy_instant_sorcery_log.push(p);
                    }
                }
                let nm = self.def_of(card).name.clone();
                self.spells_cast_names.push((p, nm));
                let _ = alt;
                self.stack_lki.push((item.id, item.def));
                self.stack_lki_src.push((item.id, item.source));
                self.stack_lki_x.push((item.id, item.x as i32));
                self.stack_lki_items.push(item.clone());
                self.stack.push(item);
                self.layers_dirty = true;
                // cast triggers see the spell with its granted types ("whenever you cast an enchantment spell" for an Aura-like grant)
                self.recompute_layers();
                // Soul Sculptor: "until a player casts a creature spell"
                if self.cards[card.0 as usize].cur.types.has(CardType::Creature) && self.effects.iter().any(|e| e.duration == Duration::UntilCreatureSpellCast) {
                    self.effects.retain(|e| e.duration != Duration::UntilCreatureSpellCast);
                    self.layers_dirty = true;
                    self.recompute_layers();
                }
                self.consume_once_cost_changes(p, card);
                // "the next spell they cast this turn has cascade" (Bigger on the Inside)
                if self.players[p as usize].cascade_turn == self.turn && self.turn > 0 {
                    self.players[p as usize].cascade_turn = 0;
                    if let Some(a) = crate::carddb::keyword_expansion(Keyword::Cascade) {
                        self.cards[card.0 as usize].cur.granted.push(std::sync::Arc::new(a));
                        self.cards[card.0 as usize].cur.granted_by.push(card);
                    }
                }
                // Yidris: every spell cast from the hand this turn has cascade
                if self.players[p as usize].hand_cascade_turn == self.turn && self.turn > 0 && self.cards[card.0 as usize].cast_from == Some(Zone::Hand) {
                    if let Some(a) = crate::carddb::keyword_expansion(Keyword::Cascade) {
                        self.cards[card.0 as usize].cur.granted.push(std::sync::Arc::new(a));
                        self.cards[card.0 as usize].cur.granted_by.push(card);
                    }
                }
                self.emit(Event::Cast { card, player: p });
                // Word of Command: the cast it forced is finished; its controller controls the player while the spell resolves
                if let Some((c, v, ctl)) = self.wok_pending {
                    if c == card {
                        self.wok_pending = None;
                        self.spell_ctl.push((card, v, ctl));
                        self.resolve_ctl = None;
                        self.land_mana_only = None;
                    }
                }
                let mut i = 0;
                while i < self.cast_after.len() {
                    if self.cast_after[i].0 == card {
                        let (_, src, effs) = self.cast_after.remove(i);
                        self.dmg_followups.borrow_mut().push((src, p, effs, 0, Some(card), None, Some(p), vec![]));
                    } else {
                        i += 1;
                    }
                }
                for _ in 0..std::mem::take(&mut self.cards[card.0 as usize].cast_copies) {
                    rider_effs.push(Effect::CopySpell { what: Sel::TriggerObject, new_targets: true, by: None, except: vec![], target: None, each_other_target: false, for_opponent: None, even_if_gone: false });
                }
                for eff in rider_effs {
                    self.dmg_followups.borrow_mut().push((card, p, vec![eff], 0, Some(card), None, Some(p), vec![]));
                }
                self.flush_followups();
                self.apply_ward();
            }
            Origin::Ability { card, ability } => {
                if self.ability_owned(card, ability).as_ref().map_or(false, |a| self.is_equip_for(card, a)) {
                    self.players[p as usize].equip_turn = true;
                }
                // loyalty cost
                if let Some(Ability::Activated { loyalty, limit_per_turn, once, .. }) = self.ability_owned(card, ability) {
                    if once {
                        self.players[p as usize].exhaust_turn = true;
                        self.cards[card.0 as usize].exhausted.push(ability);
                    }
                    if let Some(l) = loyalty {
                        let lc = self.cards[card.0 as usize].cur.controller;
                        self.players[lc as usize].loyalty_turn = true;
                        let extra = self.loyalty_cost_delta(p, card);
                        // loyalty counters put on as a cost go through counter replacements (Vorinclex doubles a +N cost, with any Carth surcharge: one event)
                        let put_total = if l >= 0 && extra >= 0 { let prev = self.cause_ctl.replace(Some(p)); self.counters_as_cost.set(true); let t = self.replace_counters(card, CounterKind::Loyalty, l + extra); self.counters_as_cost.set(false); self.cause_ctl.set(prev); Some(t) } else { None };
                        let c = &mut self.cards[card.0 as usize];
                        if let Some(t) = put_total {
                            c.add_counters(CounterKind::Loyalty, t);
                        } else if l >= 0 {
                            c.add_counters(CounterKind::Loyalty, l);
                        } else {
                            c.remove_counters(CounterKind::Loyalty, -l);
                        }
                        if put_total.is_some() {
                        } else if extra > 0 {
                            c.add_counters(CounterKind::Loyalty, extra);
                        } else if extra < 0 {
                            c.remove_counters(CounterKind::Loyalty, -extra);
                        }
                        c.activations.push((u16::MAX, 1));
                        self.layers_dirty = true;
                        if l > 0 {
                            // putting loyalty counters is "counters put on this permanent" for triggers (Bioessence Hydra)
                            self.emit(Event::Counters { card, kind: CounterKind::Loyalty, delta: l, by: self.cause_ctl.get() });
                        } else if l < 0 {
                            self.emit(Event::Counters { card, kind: CounterKind::Loyalty, delta: l, by: Some(p) });
                        }
                        if extra != 0 {
                            self.emit(Event::Counters { card, kind: CounterKind::Loyalty, delta: extra, by: self.cause_ctl.get() });
                        }
                    }
                    let _ = limit_per_turn;
                }
                let c = &mut self.cards[card.0 as usize];
                if let Some(e) = c.activations.iter_mut().find(|(i, _)| *i == ability) {
                    e.1 = e.1.saturating_add(1);
                } else {
                    c.activations.push((ability, 1));
                }
                if self.cards[card.0 as usize].cur.types.has(CardType::Artifact) {
                    self.artifact_act_log.push(p);
                }
                if self.active == p && item.targets.iter().any(|t| matches!(t.obj, Obj::Card { id } if self.cards[id.0 as usize].zone == Zone::Battlefield && self.cards[id.0 as usize].cur.controller == p && self.cards[id.0 as usize].cur.types.has(CardType::Creature))) {
                    self.creature_tgt_act_log.push(p);
                }
                if self.def_of(card).abilities.get(ability as usize).map_or(false, |a| matches!(a, Ability::Activated { power_up: true, .. })) {
                    self.power_up_act_log.push(p);
                }
                item.src_zcc = self.cards[card.0 as usize].zcc;
                // a battlefield ability whose own cost sacrificed (or returned) its source came from the object that just left: it is a different object now (Ghost Council)
                if ability < GRANTED_BASE
                    && self.cards[card.0 as usize].zone != Zone::Battlefield
                    && matches!(self.def_of(card).abilities.get(ability as usize), Some(Ability::Activated { from: None, .. }))
                    && !matches!(self.cards[card.0 as usize].zone, Zone::Hand | Zone::Stack)
                {
                    item.src_zcc = item.src_zcc.saturating_sub(1);
                }
                // suspend / plot / foretell are special actions: exiling the card from hand never uses the stack (CR 116.2)
                let special_action = ability < GRANTED_BASE
                    && self.cards[card.0 as usize].zone == Zone::Hand
                    && matches!(self.def_of(card).abilities.get(ability as usize), Some(Ability::Activated { from: Some(Zone::Hand), targets, effects, .. })
                        if targets.is_empty() && matches!(effects.first(), Some(Effect::Move { what: Sel::This, to: Zone::Exile, from: None, .. })));
                // turning a face-down permanent face up is a special action too (CR 116.2b, 702.37e): it never uses the stack
                let special_action = special_action
                    || (ability < GRANTED_BASE
                        && matches!(self.def_of(card).abilities.get(ability as usize), Some(Ability::Activated { targets, effects, .. })
                            if targets.is_empty() && effects.len() == 1 && matches!(effects.first(), Some(Effect::TurnFaceUp { what: None, .. }))));
                // an ability the card marks `special_action` (paying {U} to end Gliding Licid's effect) is a special action too: no stack
                let special_action = special_action
                    || (ability < GRANTED_BASE
                        && matches!(self.def_of(card).abilities.get(ability as usize), Some(Ability::Activated { special_action: true, .. })));
                self.stack.push(item);
                self.emit(Event::Activated { card, ability, player: p, life: life_paid, mana: false });
                if special_action {
                    self.begin_resolve();
                    return Flow::Continue;
                }
                for _ in 0..ability_copies {
                    let eff = Effect::CopySpell { what: Sel::TriggerObject, new_targets: true, by: None, except: vec![], target: None, each_other_target: false, for_opponent: None, even_if_gone: false };
                    self.dmg_followups.borrow_mut().push((card, p, vec![eff], 0, Some(card), None, Some(p), vec![]));
                }
                self.flush_followups();
                self.apply_ward();
            }
            Origin::Trigger => {
                let (sid, src, ctl, entering, attacking) = (item.id, item.source, item.controller, item.ctx.entering, item.ctx.attacking);
                self.stack.push(item);
                self.emit(Event::AbilityTriggered { id: sid, source: src, player: ctl, entering, attacking });
                self.apply_ward();
                // more triggers may be waiting; either way the active player gets priority next (CR 117.3b)
                let first = if self.alive(self.active) { self.active } else { self.next_player(self.active) };
                self.run = Run::Priority { player: first, passes: 0 };
                return Flow::Continue;
            }
        }
        self.run = Run::Priority { player: p, passes: 0 };
        if !self.free_casts.is_empty() {
            self.start_next_free_cast();
        }
        Flow::Continue
    }
}


/// The amount (and the target slot, for `DivideDamage.slot`) of an `at_cast` damage / counter / prevention division among
/// these effects. Looks inside `If` branches (when both branches divide, the condition is evaluated now: Avacyn's Judgment's
/// madness X), `MayPay` / `Unless` effects (Kuldotha Flamefiend: "you may sacrifice an artifact. If you do, ... divided").
fn find_at_cast_division(g: &Game, effects: &[Effect], ec: &EvalCtx) -> Option<(Amount, Option<u8>)> {
    effects.iter().find_map(|e| match e {
        Effect::DivideDamage { amount, at_cast: true, slot, .. } => Some((amount.clone(), *slot)),
        Effect::DivideCounters { total, at_cast: true, .. } => Some((total.clone(), None)),
        Effect::DividePrevention { amount, at_cast: true } => Some((amount.clone(), None)),
        Effect::If { cond, then, otherwise } => {
            let a = find_at_cast_division(g, then, ec);
            let b = find_at_cast_division(g, otherwise, ec);
            match (a, b) {
                (Some(a), Some(b)) => Some(if g.eval_cond(cond, ec) { a } else { b }),
                (a, b) => a.or(b),
            }
        }
        Effect::MayPay { effects, .. } => find_at_cast_division(g, effects, ec),
        Effect::Unless { effects, if_paid, .. } => find_at_cast_division(g, if_paid, ec).or_else(|| find_at_cast_division(g, effects, ec)),
        _ => None,
    })
}

/// One mode of a triggered or activated ability whose modes are chosen as it is put on the stack (`ChooseModes.at_stack`).
#[derive(Clone, Debug)]
pub(crate) struct StackMode {
    pub text: String,
    pub specs: Vec<TargetSpec>,
    pub effects: Vec<Effect>,
}

impl Game {
    /// For a triggered/activated ability whose only effect is a `ChooseModes { at_stack: true }`: (min, max, modes).
    pub(crate) fn stack_modal(&self, def: DefId, ability: u16) -> Option<(u8, u8, Vec<StackMode>)> {
        if ability >= GRANTED_BASE {
            return None;
        }
        let d = self.db.def(def);
        let effects = match d.abilities.get(ability as usize)? {
            Ability::Triggered { effects, .. } | Ability::Activated { effects, .. } => effects,
            _ => return None,
        };
        let Some(Effect::ChooseModes { choose_min, choose_max, modes, at_stack: true, .. }) = effects.first() else { return None };
        if effects.len() != 1 {
            return None;
        }
        let out = modes
            .iter()
            .map(|m| match m.effects.first() {
                Some(Effect::Reflexive { targets, effects, untargeted: false }) => {
                    let mut e = effects.clone();
                    e.extend(m.effects.iter().skip(1).cloned());
                    StackMode { text: m.text.clone(), specs: targets.clone(), effects: e }
                }
                _ => StackMode { text: m.text.clone(), specs: m.targets.clone(), effects: m.effects.clone() },
            })
            .collect();
        Some((*choose_min, *choose_max, out))
    }
}

impl Game {
    /// Does the `at_stack` `ChooseModes` of this ability allow the same mode more than once?
    pub(crate) fn stack_modal_repeat(&self, def: DefId, ability: u16) -> bool {
        if ability >= GRANTED_BASE {
            return false;
        }
        match self.db.def(def).abilities.get(ability as usize) {
            Some(Ability::Triggered { effects, .. }) | Some(Ability::Activated { effects, .. }) => matches!(effects.first(), Some(Effect::ChooseModes { allow_repeat: true, .. })),
            _ => false,
        }
    }

    /// `stack_modal` for the ability a cast context is putting on the stack.
    pub(crate) fn stack_modal_of(&self, ctx: &CastCtx) -> Option<(u8, u8, Vec<StackMode>)> {
        match &ctx.item.kind {
            ItemKind::Triggered { ability, .. } | ItemKind::Ability { ability, .. } => self.stack_modal(ctx.item.def, *ability),
            _ => None,
        }
    }
}
