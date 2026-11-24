//! Resolving the top of the stack and executing effects. Effects that need a decision park their progress
//! in `ResolveCtx::sub`, ask, and are re-entered with the answer.

use crate::cast::Flow;
use crate::decision::*;
use crate::dsl::*;
use crate::eval::*;
use crate::ids::*;
use crate::mana::*;
use crate::state::*;
use crate::types::*;
use crate::zones::MoveOpts;
use rand::seq::SliceRandom;
use rand::Rng;
use smallvec::SmallVec;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct Frame {
    pub list: Arc<Vec<Effect>>,
    pub pc: usize,
    /// Remaining repetitions after this pass (Repeat).
    pub repeat: i32,
    /// For ForEach: the objects still to visit (current one is bound to `Remembered`).
    pub each: Vec<Obj>,
    pub each_active: bool,
    /// Index into the target-spec list where this effect list's own `Target { idx: 0 }` starts (modal spells: each mode counts from 0).
    pub tbase: usize,
}

#[derive(Clone, Debug)]
pub enum Sub {
    May,
    /// `Effect::PlanarTunnel`: the revealed plane cards and the other revealed cards, waiting for the plane to put on top.
    PlanarTunnel { planes: Vec<CardId>, rest: Vec<CardId> },
    /// `Effect::AssignSector`: the creatures still waiting for a sector, and the one being asked about.
    SectorPick { queue: Vec<CardId>, cur: CardId },
    /// `Effect::ChangeText` waiting for an answer: stage 1 = kind, 3 = old word, 5 = new word.
    TextChange { stage: u8, target: CardId, kind: u8, old: String },
    CrownCopy { player: PlayerId, host: CardId },
    RotatePick { options: Vec<CardId>, queue: Vec<PlayerId>, plan: Vec<(CardId, PlayerId)>, who: PlayerId },
    TokenCopyPick { player: PlayerId, options: Vec<CardId> },
    TokenChoice { player: PlayerId, options: Vec<String> },
    ChooseBudget { options: Vec<CardId>, acc: Vec<CardId>, remaining: i32, chooser: PlayerId },
    MutateOver { t: CardId },
    LibPos { queue: Vec<CardId> },
    /// An owner is ordering the cards going on top of their library (`PutOnTopOwnersOrder`): the rest of the queue, then this owner's group.
    OwnerTop { queue: Vec<CardId>, group: Vec<CardId> },
    Discard { players: Vec<PlayerId>, n: Vec<i32>, filter: Filter, idx: usize },
    Sacrifice { players: Vec<PlayerId>, n: Vec<i32>, filter: Filter, idx: usize },
    Scry { player: PlayerId, cards: Vec<CardId>, surveil: bool },
    /// `Effect::PayLifeRounds`: the players of this round, whose turn it is to choose, and whether anyone has paid this round.
    PayLife { players: Vec<PlayerId>, idx: usize, any: bool },
    /// `Effect::RedistributeLife`: `chosen` players (empty while they are being picked), the totals still unassigned, the totals given so far and the distinct totals on offer.
    Redist { chosen: Vec<PlayerId>, remaining: Vec<i32>, assigned: Vec<i32>, offer: Vec<i32> },
    /// `Effect::JoinForces`: the players in turn order and whose payment is being chosen.
    JoinPay { players: Vec<PlayerId>, idx: usize },
    /// `Effect::BidLife`: the players in turn order, whose turn it is, the high bid and its bidder (index into `players`).
    Bid { players: Vec<PlayerId>, idx: usize, high: i32, bidder: usize, opened: bool },
    /// `Effect::HideNumbers`: the players in turn order and whose number is being chosen.
    Hide { players: Vec<PlayerId>, idx: usize },
    Search { player: PlayerId, found: Vec<CardId>, to: Zone, shuffle: bool, tapped: bool },
    LookAtTop { player: PlayerId, cards: Vec<CardId>, rest: Zone, take: u8, others: Vec<CardId> },
    /// The controller is ordering the rest of a look-at-top before they go to the bottom of the library.
    BottomOrder { cards: Vec<CardId> },
    /// `RevealUntil.cascade_land`: the revealed non-found cards (sitting at the bottom of the library) from which one land may be put onto the battlefield tapped.
    CascadeLand { lands: Vec<CardId> },
    /// `DestroyBlockedBy.reanimate`: the players whose graveyards still owe a reanimated creature card (the first one is being asked).
    ReanimateFrom { queue: Vec<PlayerId> },
    /// `Effect::Amass` with several Armies: the controller picks which one gets the counters.
    AmassPick { armies: Vec<CardId> },
    TopOrder { cards: Vec<CardId> },
    /// The controller is reordering this player's graveyard (`ReorderGraveyard`).
    GyOrder { player: PlayerId, cards: Vec<CardId> },
    /// LookAtTop.top_ordered: ordering the kept cards; `left` then go to the bottom.
    LookTopOrder { cards: Vec<CardId>, left: Vec<CardId>, rest: Zone },
    UnlessPay { payer: PlayerId },
    LootDiscard { player: PlayerId, n: i32 },
    /// Connive: `creature` drew and its controller is choosing the discard; `queue` creatures still have to connive.
    Connive { creature: CardId, player: PlayerId, queue: Vec<CardId> },
    DivideDamage { amounts: Vec<i32> },
    Choose { options: Vec<CardId>, then: Vec<Effect> },
    /// Auras put onto the battlefield without being cast: the controller picks what each one enchants (aura, legal hosts); the rest are still waiting.
    AuraHost { queue: Vec<(CardId, Vec<CardId>)>, tapped: bool, put_remember: Vec<Obj>, remember: bool },
    /// `Effect::ReturnAsAura`: the card returning and the legal hosts to pick from.
    AsAuraHost { aura: CardId, hosts: Vec<CardId> },
    /// `Effect::EachPlayerChooses`: the candidates offered to `chooser`, the players still to choose, and every player's (hidden) picks so far.
    HiddenChoose { options: Vec<CardId>, queue: Vec<PlayerId>, chooser: PlayerId, cards: Vec<(PlayerId, Vec<CardId>)>, named: Vec<(PlayerId, u8)> },
    /// `Effect::Choose` with several choosers: picks so far and the players still to choose.
    ChooseEach { options: Vec<CardId>, queue: Vec<PlayerId>, acc: Vec<CardId> },
    TimeTravel { queue: Vec<CardId>, cur: CardId, changes: Vec<(CardId, i32)> },
    Encode { options: Vec<CardId> },
    MayPay { payer: PlayerId },
    /// `MayPay` with an `XTimes` cost, after the yes: the number of times is being chosen.
    MayPayTimes { payer: PlayerId, max: i32 },
    /// Explore: may put the revealed nonland card into the graveyard.
    ExploreKeep { player: PlayerId, card: CardId, queue: Vec<CardId> },
    /// Explore replaced by "scry first": the controller is scrying before `creature` explores; `queue` is the rest.
    ExploreScry { creature: CardId, cards: Vec<CardId>, queue: Vec<CardId> },
    FreeCast { player: PlayerId, card: CardId },
    /// `SearchLibrary`: asking whether to cast a card from the library while searching (Panglacial Wurm).
    SearchCast { player: PlayerId, card: CardId },
    /// `CastFree.face_choice`: asking whether to cast the back face.
    FreeCastFace { player: PlayerId, card: CardId },
    /// `Effect::CastFreeAny`: the card being asked about, and the cards still to ask about.
    FreeCastAny { player: PlayerId, card: CardId, rest: Vec<CardId> },
    /// The controller of a spell being countered "unless it pays" is asked whether to pay.
    CounterPay { sid: StackId },
    /// A resolution-time choice (color / type / number / player) by `player`.
    Modes { modes: Vec<Vec<Effect>>, orig: Vec<u8> },
    /// `ChooseModes { budget }`: modes picked so far (indices into the mode list) and the pawprints still to spend.
    ModesBudget { chosen: Vec<usize>, remaining: u8 },
    Reflex { specs: Vec<TargetSpec>, idx: usize, chosen: Vec<Target>, options: Vec<Obj> },
    /// A reflexive ability's targets are chosen; its `at_cast` damage division is being announced (Blaster Hulk, Death to Our Enemies).
    ReflexDivide { chosen: Vec<Target>, divs: Vec<i32> },
    ChooseVal { player: PlayerId, kind: u8, texts: Vec<String>, players: Vec<PlayerId> },
    /// The payer of a may-pay / unless cost is choosing how to pay: which alternative, which permanents or cards.
    PayPick { payer: PlayerId, costs: Vec<Cost>, idx: usize, choice: PayChoice, unless: bool },
    /// Choosing the colour of mana added by an effect: entry `idx` of the list, `per` mana per pick, `left` picks to go.
    ManaColor { idx: usize, per: i32, left: i32 },
    Rearrange { player: PlayerId, cards: Vec<CardId> },
    Proliferate { options: Vec<Obj>, again: u8 },
    /// BecomeAttacking.defender_among: the defenders offered.
    DefenderPick { options: Vec<Obj> },
    /// A permanent this effect put onto the battlefield enters as a copy: its controller is choosing what to copy (a Clone put in by The First Tyrannic War).
    EnterCopy { card: CardId, options: Vec<CardId> },
    /// DealDamage: the damaged player picks which of several redirecting auras applies first (two Pariahs).
    RedirectPick { options: Vec<CardId> },
    /// Removing / moving counters of a kind the player picks: the permanents still to do (current first), counters left for the current one, counters per permanent, the kinds offered now, and where moved counters go.
    /// `Effect::DuplicateCounter`: the object and the kinds offered.
    DupKind { obj: Obj, kinds: Vec<CounterKind> },
    /// `Effect::CounterKindsAmongOnto`: the kinds still to place (current first) and the permanents they may go on.
    KindsOnto { kinds: Vec<CounterKind>, tokens: Vec<CardId> },
    KindPick { ids: Vec<CardId>, left: i32, n_each: i32, kinds: Vec<CounterKind>, dst: Option<CardId>, up_to: bool },
    /// DivideCounters.up_to: choosing the total to distribute (between one per object and `max`).
    DivideTotal { objs: Vec<Obj>, max: i32 },
    Divide { targets: Vec<Obj>, idx: usize, left: i32, parts: Vec<i32>, counters: Option<CounterKind> },
    /// Choosing new targets for a copied spell: the copy, the target slot being chosen, and the options offered.
    Retarget { sid: StackId, idx: usize, options: Vec<Obj> },
    /// Clash: stage 0 = the controller decides on the bottom, 1 = the opponent; `won` = the controller won.
    /// Voting: the players still to vote, the tallies so far and (for card votes) the options.
    Vote { queue: Vec<PlayerId>, counts: Vec<i32>, cards: Vec<CardId>, ballots: Vec<(PlayerId, usize)> },
    /// SacrificeAny: the permanents the controller may sacrifice.
    SacAny { options: Vec<CardId> },
    /// ChooseCardName: the names offered.
    NamePick { options: Vec<String> },
    /// NameAndRevealTop: the players still to name a card, the names chosen so far and the names offered.
    NameEach { queue: Vec<PlayerId>, done: Vec<(PlayerId, String)>, options: Vec<String> },
    /// Manifest dread: the player picks which of the two looked-at cards to manifest.
    Dread { player: PlayerId, cards: Vec<CardId> },
    /// Venture: choose a dungeon (`new_dungeon`) or the next room by name.
    Venture { options: Vec<String>, new_dungeon: bool },
    /// The Ring tempts: the controller picks the Ring-bearer among these creatures.
    RingPick { options: Vec<CardId> },
    Clash { opp: PlayerId, mine: Option<CardId>, theirs: Option<CardId>, won: bool, stage: u8 },
    /// "Clash with an opponent of your choice": the opponents to choose from.
    ClashPick { options: Vec<PlayerId> },
    /// Bolster: the creatures tied for least toughness, one of which gets the counters.
    BolsterPick { options: Vec<CardId>, n: i32 },
}

#[derive(Clone, Debug)]
pub enum AuraHostPlan {
    NotAura,
    NoHost,
    One(CardId),
    Choose,
}

#[derive(Clone, Debug)]
pub enum PayChoice {
    Alternative(Vec<Vec<Cost>>),
    Sacrifice(Vec<CardId>, usize),
    Discard(Vec<CardId>, usize),
    Counters(Vec<CardId>),
    Control(Vec<CardId>),
    SacrificePower(Vec<CardId>, i32),
    /// Cards in the payer's graveyard to exile (the cost's own picks go through `mana_picks`).
    Graveyard(Vec<CardId>, usize),
    /// The opponent who gains the life / creates the tokens of an "an opponent ..." cost.
    Opponent(Vec<PlayerId>),
}

#[derive(Clone, Debug)]
pub struct ResolveCtx {
    pub item: StackItem,
    pub frames: Vec<Frame>,
    pub remembered: Vec<Obj>,
    pub targets: Vec<Target>,
    pub tidx: SmallVec<[u8; 4]>,
    pub sub: Option<Sub>,
    pub fizzled: bool,
    pub cur_tbase: usize,
    /// Permanents sacrificed by the current Sacrifice effect (becomes `Remembered` when it finishes).
    pub sac_done: Vec<Obj>,
    /// CopySpell: spells still to copy / copies made so far (a retarget question pauses the loop).
    pub copy_rest: Vec<StackId>,
    pub unless_rest: Vec<PlayerId>,
    pub copy_made: Vec<Obj>,
    /// The full target list and slot indices while a mode's `ScopeTargets` narrows `targets` / `tidx`.
    pub full_targets: Option<Box<(Vec<Target>, SmallVec<[u8; 4]>)>>,
}

enum Exec {
    Done,
    Wait,
    Push(Frame),
}

impl Game {
    /// For an AI's determinized copy of the game (never the real one): forget the hidden `EachPlayerChooses` picks that other
    /// players have already made and put those players back in the queue after the rest, so the AI's search sees only
    /// `viewer`'s own picks and the others choose again in the simulation (the real game reveals nothing until all have chosen).
    pub fn forget_hidden_picks(&mut self, viewer: PlayerId) {
        let Run::Resolve(ctx) = &mut self.run else { return };
        let Some(Sub::HiddenChoose { queue, cards, named, .. }) = &mut ctx.sub else { return };
        let mut again: Vec<PlayerId> = cards.iter().map(|(p, _)| *p).chain(named.iter().map(|(p, _)| *p)).filter(|p| *p != viewer).collect();
        again.dedup();
        cards.retain(|(p, _)| *p == viewer);
        named.retain(|(p, _)| *p == viewer);
        for p in again {
            if !queue.contains(&p) {
                queue.push(p);
            }
        }
    }

    /// For an Aura card (not bestowed) that is about to be put onto the battlefield without being cast: the permanents it could legally enchant.
    /// `None` = not an Aura with a plain "enchant <filter>" ability.
    pub(crate) fn aura_hosts(&self, aura: CardId) -> Option<Vec<CardId>> {
        let d = self.def_of(aura);
        if !d.types.has(CardType::Enchantment) || !d.subtypes.contains(&Sym::new("Aura")) {
            return None;
        }
        let mut f = d.abilities.iter().find_map(|a| if let Ability::Attach { to } = a { Some(to.clone()) } else { None })?;
        f.controller = PlayerFilter::Any;
        let ctrl = self.cards[aura.0 as usize].owner;
        Some(self.battlefield.iter().copied().filter(|h| *h != aura && self.matches_card(&f, *h, aura, ctrl)).collect())
    }
}

impl Game {
    /// Does an opponent of `fp` control a permanent that makes `fp` face villainous choices an additional time?
    fn villainous_twice(&self, fp: PlayerId) -> bool {
        self.battlefield.iter().any(|id| {
            let c = &self.cards[id.0 as usize];
            c.cur.controller != fp && !c.cur.no_abilities && self.def_of(*id).statics.iter().any(|&i| matches!(&self.def_of(*id).abilities[i as usize], Ability::Static { effect: StaticEffect::VillainousChoiceTwice, .. }))
        })
    }

    /// `ChooseNumber { record }`: remember `n` as `p`'s number.
    fn record_number(&mut self, p: Option<PlayerId>, n: i32) {
        if self.player_numbers.len() < self.players.len() {
            self.player_numbers.resize(self.players.len(), None);
        }
        if let Some(p) = p {
            self.player_numbers[p as usize] = Some(n);
        }
    }
}

impl Game {
    /// The card names offered to a "choose a card name" effect: every non-token card in the game that fits `kind`, unsorted.
    pub(crate) fn name_options(&self, kind: NameKind) -> Vec<String> {
        let mut names: Vec<String> = vec![];
        for c in &self.cards {
            if c.token || c.ceased {
                continue;
            }
            let d = self.db.def(c.front);
            let ok = match kind {
                NameKind::Any => true,
                NameKind::Nonland => !d.types.has(CardType::Land),
                NameKind::Creature => d.types.has(CardType::Creature),
                NameKind::Artifact => d.types.has(CardType::Artifact),
                NameKind::NotBasicLand => !(d.types.has(CardType::Land) && d.supertypes.contains(&Supertype::Basic)),
                NameKind::NonartifactNonland => !d.types.has(CardType::Land) && !d.types.has(CardType::Artifact),
                NameKind::NoncreatureNonland => !d.types.has(CardType::Land) && !d.types.has(CardType::Creature),
                NameKind::Land => d.types.has(CardType::Land),
            };
            // a double-faced card offers each face's own name
            if let (Some(back), Some((front_half, _))) = (d.other_face, d.name.split_once(" // ")) {
                for n in [front_half.to_string(), self.db.def(back).name.clone()] {
                    if ok && !names.contains(&n) {
                        names.push(n);
                    }
                }
                continue;
            }
            if ok && !names.contains(&d.name) {
                names.push(d.name.clone());
            }
        }
        names
    }

    /// Pop the top of the stack and start resolving it.
    pub fn begin_resolve(&mut self) {
        let item = self.stack.pop().expect("resolve with empty stack");
        // stored slots only live for one resolution (lasting objects pin what they read: `pin_stored`)
        *self.stored_sets.borrow_mut() = Default::default();
        if let Some(i) = self.spell_ctl.iter().position(|(c, _, _)| *c == item.source) {
            let (c, v, ctl) = self.spell_ctl.remove(i);
            self.resolve_ctl = Some((v, ctl));
            self.resolve_ctl_item = Some(c);
        }
        self.recompute_layers();
        // Ascend on an instant or sorcery: ten or more permanents as it resolves give the city's blessing for good (CR 702.131b)
        if matches!(item.kind, ItemKind::Spell { .. }) {
            let (ctl, src) = (item.controller, item.source);
            if (ctl as usize) < self.players.len()
                && !self.players[ctl as usize].city_blessing
                && self.def_of(src).oracle_text.lines().any(|l| l.starts_with("Ascend"))
                && self.battlefield.iter().filter(|id| self.cards[id.0 as usize].cur.controller == ctl).count() >= 10
            {
                self.players[ctl as usize].city_blessing = true;
            }
        }
        // recheck targets
        let mut targets = vec![];
        let mut tidx: SmallVec<[u8; 4]> = SmallVec::new();
        let mut pos = 0usize;
        let delayed = matches!(item.kind, ItemKind::Delayed { .. });
        // a reflexive trigger (Delayed with announced targets) fizzles like any ability when all its targets are gone
        let reflexive = delayed && !item.spec_counts.is_empty();
        let had_targets = !item.targets.is_empty() && (!delayed || reflexive);
        let mut legal_any = false;
        let db = self.db.clone();
        let specs = self.item_specs(&item, &db);
        // a "for each player/opponent, target ..." slot was announced as one slot per player: re-check each against its own player's restriction
        let specs = if specs.len() < item.spec_counts.len() && specs.iter().any(|sp| sp.per_player || sp.per_opponent) { self.expand_per_opponent(&specs, item.controller) } else { specs };
        for (si, cnt) in item.spec_counts.iter().enumerate() {
            let mut first: Option<u8> = None;
            for k in 0..*cnt as usize {
                let Some(t) = item.targets.get(pos + k) else { continue };
                if self.target_still_legal(&item, t, specs.get(si)) {
                    if first.is_none() {
                        first = Some(targets.len() as u8);
                    }
                    targets.push(t.clone());
                    legal_any = true;
                }
            }
            // "any number of target creatures with equal toughness" (V.A.T.S.): if the chosen creatures no longer match, none are affected
            if specs.get(si).map_or(false, |sp| sp.distinct_by == Some(ChooseDistinct::SameToughness)) {
                if let Some(f0) = first {
                    let ts: Vec<i32> = targets[f0 as usize..].iter().filter_map(|t| if let Obj::Card { id } = &t.obj { Some(self.cards[id.0 as usize].cur.toughness) } else { None }).collect();
                    if ts.windows(2).any(|w| w[0] != w[1]) {
                        targets.truncate(f0 as usize);
                        first = None;
                    }
                }
            }
            pos += *cnt as usize;
            tidx.push(first.unwrap_or(255));
        }
        if delayed {
            // a delayed trigger keeps the targets of the ability that created it
            targets = item.targets.clone();
            tidx = (0..targets.len() as u8).collect();
        }
        let legal_any = legal_any && !targets.is_empty();
        // "This ability still resolves if its target becomes illegal" (Gilded Drake): its effects run even with no legal target left
        let resolves_anyway = matches!(item.kind, ItemKind::Triggered { .. }) && db.def(item.def).oracle_text.contains("still resolves if its target becomes illegal");
        let mut fizzled = had_targets && !legal_any && !resolves_anyway;
        // intervening "if" (CR 603.4): "When/Whenever/At ..., if <cond>, <effect>" does nothing if <cond> is false as it resolves.
        // The DSL `condition` is also used for non-"if" triggers (battalion, training ...), so this only applies when the card's
        // oracle text has an intervening-if line, and only while the source is still the same object on the battlefield
        // (a leaves-the-battlefield trigger reads last-known information).
        if !fizzled {
            if let ItemKind::Triggered { ability, .. } = &item.kind {
                let sc = &self.cards[item.source.0 as usize];
                if item.granted_snap.is_none() && sc.zcc == item.src_zcc {
                    let d = db.def(item.def);
                    if let Some(Ability::Triggered { condition: Some(cond), from, .. }) = d.abilities.get(*ability as usize).filter(|a| matches!(a, Ability::Triggered { from: None, .. }) && sc.zone == Zone::Battlefield || matches!(a, Ability::Triggered { from: Some(z), .. } if *z == sc.zone)) {
                        let _ = from;
                        if has_intervening_if(&d.oracle_text)
                            && !matches!(d.abilities.get(*ability as usize), Some(Ability::Triggered { no_recheck: true, .. }))
                            && ((item.ctx.obj.is_none() && item.ctx.source.is_none())
                                || !{ let t = serde_json::to_string(cond).unwrap_or_default().replace("TriggerPlayer", ""); t.contains("Trigger") || t.contains("Event") || t.contains("Remembered") || t.contains("same_name_as") })
                        {
                            let ec = EvalCtx { source: item.source, controller: item.controller, x: item.x, targets: &targets, tidx: &tidx, event: &item.ctx, remembered: &item.remembered };
                            *self.filter_event.borrow_mut() = Some(item.ctx.clone());
                            let ok = self.eval_cond(cond, &ec);
                            *self.filter_event.borrow_mut() = None;
                            if !ok {
                                fizzled = true;
                            }
                        }
                    }
                }
            }
        }
        // the source left before the trigger resolved: an intervening "if" that only asks about other things still applies
        // (Walltop Sentries: "if there's a Lesson in your graveyard"), and "if <this> is on the battlefield" is false (Sidar Jabari's eminence)
        if !fizzled {
            if let ItemKind::Triggered { ability, .. } = &item.kind {
                let sc = &self.cards[item.source.0 as usize];
                if item.granted_snap.is_none() && (sc.zcc != item.src_zcc || sc.zone != Zone::Battlefield) {
                    let d = db.def(item.def);
                    if let Some(Ability::Triggered { condition: Some(cond), from: None, no_recheck: false, .. }) = d.abilities.get(*ability as usize) {
                        if has_intervening_if(&d.oracle_text) {
                            let t = serde_json::to_string(cond).unwrap_or_default();
                            let only_on_battlefield = matches!(cond, Condition::Control { filter, .. } if filter.this);
                            let independent = !(t.contains("Trigger") || t.contains("Event") || t.contains("Remembered") || t.contains("same_name_as") || t.contains("\"this\"") || t.contains("\"This\""));
                            // "if <this> has a counter on it" is false once the source has left the battlefield (The Ozolith)
                            let only_source_counters = matches!(cond, Condition::SourceHasCounter { .. }) || matches!(cond, Condition::AnyOf { cs } if !cs.is_empty() && cs.iter().all(|x| matches!(x, Condition::SourceHasCounter { .. })));
                            // a counter threshold on the source (Helix Pinnacle: 100 or more) is read from its last-known counters
                            let lki_counters = if let Condition::SourceHasCounter { kind, at_least } = cond {
                                item.ctx.lki.as_deref().or(self.last_lki.as_ref()).filter(|l| l.card == item.source).or_else(|| self.left_lki.iter().rev().find(|l| l.card == item.source)).filter(|_| *at_least >= 2).map(|l| l.counters.iter().filter(|(k, _)| k == kind).map(|(_, n)| *n as i32).sum::<i32>() >= *at_least)
                            } else {
                                None
                            };
                            if let Some(ok) = lki_counters {
                                if !ok {
                                    fizzled = true;
                                }
                            } else if only_on_battlefield || only_source_counters {
                                fizzled = true;
                            } else if independent && matches!(cond, Condition::Compare { a: Amount::Count { .. }, .. }) {
                                let ec = EvalCtx { source: item.source, controller: item.controller, x: item.x, targets: &targets, tidx: &tidx, event: &item.ctx, remembered: &item.remembered };
                                *self.filter_event.borrow_mut() = Some(item.ctx.clone());
                                let ok = self.eval_cond(cond, &ec);
                                *self.filter_event.borrow_mut() = None;
                                if !ok {
                                    fizzled = true;
                                }
                            }
                        }
                    }
                }
            }
        }
        if let ItemKind::Ability { ability, .. } | ItemKind::Triggered { ability, .. } = &item.kind {
            let n = self.ability_res.entry((item.source, *ability)).or_insert(0);
            *n += 1;
            self.cur_ability_res = *n;
        }
        if !fizzled && crate::coverage::ON.load(std::sync::atomic::Ordering::Relaxed) {
            crate::coverage::note(&item, &db);
        }
        let mut ctx = ResolveCtx { item, frames: vec![], remembered: vec![], targets, tidx, sub: None, fizzled, cur_tbase: 0, sac_done: vec![], copy_rest: vec![], copy_made: vec![], unless_rest: vec![], full_targets: None };
        ctx.remembered = ctx.item.remembered.clone();
        self.flip_serial += 1;
        if !fizzled {
            // a promised gift is given as the spell resolves
            if matches!(ctx.item.kind, ItemKind::Spell { .. }) && !ctx.item.copy && self.cards[ctx.item.source.0 as usize].kicked && db.def(ctx.item.def).abilities.iter().any(|a| matches!(a, Ability::Kicker { cost, .. } if cost.is_empty())) {
                self.emit(Event::GiftGiven { player: ctx.item.controller });
            }
            // spliced cards run after the spell's own text (frames run last-pushed first): their targets follow the spell's own
            if matches!(ctx.item.kind, ItemKind::Spell { .. }) && !ctx.item.spliced.is_empty() {
                let mut plain = ctx.item.clone();
                plain.spliced.clear();
                let mut base = self.item_specs(&plain, &db).len();
                let mut frames: Vec<Frame> = vec![];
                for sd in ctx.item.spliced.clone() {
                    let d = db.def(sd);
                    for (i, a) in d.abilities.iter().enumerate() {
                        if let Ability::Spell { targets, .. } = a {
                            if let Some(list) = d.programs[i].first() {
                                frames.push(Frame { list: list.clone(), pc: 0, repeat: 0, each: vec![], each_active: false, tbase: base });
                            }
                            base += targets.len();
                        }
                    }
                }
                frames.reverse();
                ctx.frames.extend(frames);
            }
            if let Some(modes) = self.modal_programs(&ctx.item, &db) {
                // each chosen mode is its own list whose `Target { idx: 0 }` is that mode's first target (first mode runs first)
                let nmodes = modes.len();
                let spec_n: Vec<u8> = if nmodes > 1 {
                    match d_modes_spec_counts(&db, &ctx.item) { Some(v) => v, None => vec![] }
                } else { vec![] };
                for (k, (list, tbase)) in modes.into_iter().enumerate().rev() {
                    let list = if nmodes > 1 && spec_n.len() == nmodes {
                        let mut v: Vec<Effect> = vec![Effect::ScopeTargets { base: tbase as u8, n: spec_n[k] }];
                        v.extend(list.iter().cloned());
                        if k + 1 == nmodes { v.push(Effect::ScopeTargets { base: 0, n: 255 }); }
                        Arc::new(v)
                    } else { list };
                    ctx.frames.push(Frame { list, pc: 0, repeat: 0, each: vec![], each_active: false, tbase: if nmodes > 1 && spec_n.len() == nmodes { 0 } else { tbase } });
                }
            } else if let Some(list) = self.program_of(&ctx.item, &db) {
                ctx.frames.push(Frame { list, pc: 0, repeat: 0, each: vec![], each_active: false, tbase: ctx.cur_tbase });
            }
            if let ItemKind::Delayed { effects } = &ctx.item.kind {
                ctx.frames.push(Frame { list: Arc::new(effects.clone()), pc: 0, repeat: 0, each: vec![], each_active: false, tbase: ctx.cur_tbase });
            }
        }
        self.run = Run::Resolve(Box::new(ctx));
    }

    /// For a modal spell: the effect list of each chosen mode with the index of that mode's first target spec.
    fn modal_programs(&self, item: &StackItem, db: &Arc<crate::carddb::CardDb>) -> Option<Vec<(Arc<Vec<Effect>>, usize)>> {
        if let ItemKind::Ability { ability, .. } | ItemKind::Triggered { ability, .. } = &item.kind {
            let (_, _, sm) = self.stack_modal(item.def, *ability)?;
            let mut out = vec![];
            let mut base = 0usize;
            for &m in &item.modes {
                let sp = &sm[m as usize];
                out.push((Arc::new(sp.effects.clone()), base));
                base += sp.specs.len();
            }
            return Some(out);
        }
        if !matches!(item.kind, ItemKind::Spell { .. }) {
            return None;
        }
        let d = db.def(item.def);
        // an alternative cost with its own effects (overload, "choose two") replaces the modes
        if let ItemKind::Spell { alt: Some(ai), .. } = &item.kind {
            if d.abilities.iter().filter_map(|a| if let Ability::AlternativeCost { effects, .. } = a { Some(effects.is_some()) } else { None }).nth(*ai as usize) == Some(true) {
                return None;
            }
        }
        for (i, a) in d.abilities.iter().enumerate() {
            if let Ability::Modal { modes, .. } = a {
                let mut out = vec![];
                let mut base = 0usize;
                for &m in &item.modes {
                    if let Some(p) = d.programs[i].get(m as usize) {
                        out.push((p.clone(), base));
                    }
                    base += modes[m as usize].targets.len();
                }
                return Some(out);
            }
        }
        None
    }

    fn item_specs(&self, item: &StackItem, db: &Arc<crate::carddb::CardDb>) -> Vec<TargetSpec> {
        let d = db.def(item.def);
        match &item.kind {
            ItemKind::Spell { .. } => {
                let mut v = vec![];
                // an alternative cost with its own effects (overload, cleave) brings its own targets
                if let Some(i) = self.cards[item.source.0 as usize].cast_alt {
                    if i == crate::decision::FACE_DOWN_ALT {
                        // a face-down spell has no abilities: no Aura target
                        return v;
                    }
                    if i != crate::decision::FACE_DOWN_ALT {
                        if let Some(Ability::AlternativeCost { effects: Some(_), targets, .. }) = d.abilities.iter().filter(|a| matches!(a, Ability::AlternativeCost { .. })).nth(i as usize) {
                            return targets.clone().unwrap_or_default();
                        }
                    }
                }
                for a in &d.abilities {
                    match a {
                        Ability::Spell { targets, .. } => v.extend(targets.iter().cloned()),
                        Ability::Modal { modes, .. } => {
                            for &m in &item.modes {
                                v.extend(modes[m as usize].targets.iter().cloned());
                            }
                        }
                        Ability::Attach { to } => v.push(TargetSpec { what: TargetKind::Permanent { filter: to.clone() }, min: 1, max: 1, label: String::new(), max_x: false, exact_x: false, max_amount: None, max_total_mv: None, max_total_power: None, chooser: None, distinct: false, when_kicked: None, per_opponent: false, per_player: false, same_graveyard: false, distinct_by: None, random: false, only_controller: None, distinct_modes: false, corrupted: false }),
                        Ability::EnchantPlayer => v.push(TargetSpec { what: TargetKind::Player { filter: PlayerFilter::Any }, min: 1, max: 1, label: String::new(), max_x: false, exact_x: false, max_amount: None, max_total_mv: None, max_total_power: None, chooser: None, distinct: false, when_kicked: None, per_opponent: false, per_player: false, same_graveyard: false, distinct_by: None, random: false, only_controller: None, distinct_modes: false, corrupted: false }),
                        Ability::EnchantOpponent => v.push(TargetSpec { what: TargetKind::Player { filter: PlayerFilter::Opponent }, min: 1, max: 1, label: String::new(), max_x: false, exact_x: false, max_amount: None, max_total_mv: None, max_total_power: None, chooser: None, distinct: false, when_kicked: None, per_opponent: false, per_player: false, same_graveyard: false, distinct_by: None, random: false, only_controller: None, distinct_modes: false, corrupted: false }),
                        _ => {}
                    }
                }
                // cards spliced onto the spell bring their targets after its own
                for sd in &item.spliced {
                    for a in &db.def(*sd).abilities {
                        if let Ability::Spell { targets, .. } = a {
                            v.extend(targets.iter().cloned());
                        }
                    }
                }
                v
            }
            ItemKind::Ability { ability, .. } | ItemKind::Triggered { ability, .. } => {
                // an ability whose modes were chosen as it went on the stack: the targets of the chosen modes
                if let Some((_, _, sm)) = self.stack_modal(item.def, *ability) {
                    return item.modes.iter().flat_map(|&m| sm[m as usize].specs.clone()).collect();
                }
                let granted = if *ability >= crate::cast::GRANTED_BASE { item.granted_snap.clone().or_else(|| self.granted_ab(item.source, *ability)) } else { None };
                match granted.as_deref().or_else(|| d.abilities.get(*ability as usize)) {
                    Some(Ability::Activated { targets, .. }) | Some(Ability::Triggered { targets, .. }) => targets.clone(),
                    _ => vec![],
                }
            }
            ItemKind::Delayed { .. } => vec![],
        }
    }

    /// The spec re-checked at resolution against the targets chosen (type / controller changes, hexproof, "spell that targets ...").
    fn spec_still_legal(&self, item: &StackItem, t: &Target, spec: &TargetSpec) -> bool {
        *self.filter_event.borrow_mut() = Some(item.ctx.clone());
        // a slot that refers to an earlier target ("target creature they control") is re-checked with the chosen targets
        let mut tidx: Vec<u8> = vec![];
        let mut off = 0usize;
        for &c in item.spec_counts.iter() {
            tidx.push(if c == 0 { 255 } else { off as u8 });
            off += c as usize;
        }
        *self.filter_targets.borrow_mut() = (item.targets.to_vec(), tidx);
        self.target_trig.set(matches!(item.kind, ItemKind::Triggered { .. } | ItemKind::Delayed { .. }));
        let legal = self.legal_targets(spec, item.source, item.controller);
        self.target_trig.set(false);
        *self.filter_targets.borrow_mut() = (vec![], vec![]);
        *self.filter_event.borrow_mut() = None;
        legal.contains(&t.obj)
    }

    fn target_still_legal(&self, item: &StackItem, t: &Target, spec: Option<&TargetSpec>) -> bool {
        match t.obj {
            Obj::Player { id } => self.can_target_player(item.source, item.controller, id),
            Obj::Card { id } => {
                let c = &self.cards[id.0 as usize];
                if c.zcc != t.zcc {
                    return false;
                }
                match spec {
                    Some(spec) => self.spec_still_legal(item, t, spec),
                    None => true,
                }
            }
            Obj::Stack { id } => self.stack.iter().any(|s| s.id == id) && spec.map_or(true, |spec| !matches!(spec.what, TargetKind::Spell { .. }) || self.spec_still_legal(item, t, spec)),
        }
    }

    fn program_of(&self, item: &StackItem, db: &Arc<crate::carddb::CardDb>) -> Option<Arc<Vec<Effect>>> {
        let prog = self.program_of_base(item, db)?;
        // Cast Through Time: instant and sorcery spells you control have rebound
        if matches!(item.kind, ItemKind::Spell { from: Zone::Hand, .. }) && !item.copy {
            let d = db.def(item.def);
            if (d.types.has(CardType::Instant) || d.types.has(CardType::Sorcery)) && !d.keywords.has(Keyword::Rebound) && (self.rebound_granted(item.controller) || self.cards[item.source.0 as usize].gained_rebound) {
                let mut v = (*prog).clone();
                v.push(crate::carddb::rebound_effect());
                return Some(Arc::new(v));
            }
        }
        Some(prog)
    }

    /// Does a static ability give `p`'s instant and sorcery spells rebound?
    fn rebound_granted(&self, p: PlayerId) -> bool {
        if self.planes.iter().any(|&id| {
            let d = self.db.def(self.cards[id.0 as usize].def);
            d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::GrantReboundAll, .. }))
        }) {
            return true;
        }
        self.battlefield.iter().any(|&id| {
            let c = &self.cards[id.0 as usize];
            if c.cur.no_abilities || c.cur.controller != p {
                return false;
            }
            let d = self.db.def(c.def);
            d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::GrantRebound, .. }))
        })
    }

    pub(crate) fn program_of_base(&self, item: &StackItem, db: &Arc<crate::carddb::CardDb>) -> Option<Arc<Vec<Effect>>> {
        let d = db.def(item.def);
        match &item.kind {
            ItemKind::Spell { alt, .. } => {
                // overload: the alternative cost brings its own effects
                if let Some(ai) = alt {
                    let mut n = 0u8;
                    for (i, a) in d.abilities.iter().enumerate() {
                        if let Ability::AlternativeCost { effects, .. } = a {
                            if n == *ai && effects.is_some() {
                                return d.programs[i].first().cloned();
                            }
                            n += 1;
                        }
                    }
                }
                for (i, a) in d.abilities.iter().enumerate() {
                    match a {
                        Ability::Spell { .. } => return d.programs[i].first().cloned(),
                        Ability::Modal { .. } => {
                            // concatenate chosen modes in printed order
                            let mut all = vec![];
                            for &m in &item.modes {
                                if let Some(p) = d.programs[i].get(m as usize) {
                                    all.extend(p.iter().cloned());
                                }
                            }
                            return Some(Arc::new(all));
                        }
                        _ => {}
                    }
                }
                None
            }
            ItemKind::Ability { ability, .. } | ItemKind::Triggered { ability, .. } => {
                if *ability >= crate::cast::GRANTED_BASE {
                    let a = item.granted_snap.clone().or_else(|| self.granted_ab(item.source, *ability))?;
                    let list = crate::carddb::ability_effect_lists(&a).first().map(|l| (*l).clone())?;
                    if let (ItemKind::Triggered { optional: true, .. }, false) = (&item.kind, matches!(list.first(), Some(Effect::May { .. }))) {
                        return Some(Arc::new(vec![Effect::May { who: None, remember_answer: false, otherwise: vec![], effects: list }]));
                    }
                    return Some(Arc::new(list));
                }
                let prog = d.programs.get(*ability as usize).and_then(|p| p.first().cloned())?;
                if let (ItemKind::Triggered { optional: true, .. }, false) = (&item.kind, matches!(prog.first(), Some(Effect::May { .. }))) {
                    // "you may": the controller decides on resolution
                    return Some(Arc::new(vec![Effect::May { who: None, remember_answer: false, otherwise: vec![], effects: (*prog).clone() }]));
                }
                Some(prog)
            }
            ItemKind::Delayed { .. } => None,
        }
    }

    /// Mutate: merge card `m` (a creature spell) with the creature `t`; `over`: `m` becomes the top card.
    pub(crate) fn merge_cards(&mut self, t: CardId, m: CardId, over: bool) {
        self.stack.retain(|s| s.source != m);
        let mdef = self.cards[m.0 as usize].def;
        {
            let c = &mut self.cards[m.0 as usize];
            c.zone = Zone::Exile; // held off every zone list while merged
            c.zcc += 1;
        }
        let tdef = self.cards[t.0 as usize].def;
        if over {
            self.cards[t.0 as usize].mut_parts.push(tdef);
            self.cards[t.0 as usize].def = mdef;
        } else {
            self.cards[t.0 as usize].mut_parts.push(mdef);
        }
        self.cards[t.0 as usize].merged.push(m);
        // keep what the permanent has (counters, damage, attachments): only its characteristics are rebuilt
        let (tap, dmg) = (self.cards[t.0 as usize].tapped, self.cards[t.0 as usize].damage);
        self.reset_derived(t);
        self.cards[t.0 as usize].tapped = tap;
        self.cards[t.0 as usize].damage = dmg;
        self.layers_dirty = true;
        self.recompute_layers();
        self.emit(Event::Mutated { card: t });
    }

    /// Connive's discard: the card goes to the graveyard; a nonland card puts a +1/+1 counter on the conniving creature.
    fn connive_discard(&mut self, creature: CardId, player: PlayerId, card: CardId) {
        let nonland = !self.cards[card.0 as usize].cur.types.has(CardType::Land);
        self.move_card(card, Zone::Graveyard, Default::default());
        self.emit(Event::Discarded { card, player, by: self.cause_ctl.get() });
        if nonland && self.cards[creature.0 as usize].zone == Zone::Battlefield {
            self.add_counters_to(Obj::card(creature), CounterKind::PlusOnePlusOne, 1);
        }
    }

    pub(crate) fn step_resolve(&mut self, mut ctx: Box<ResolveCtx>) -> Flow {
        self.copy_resolving.set(ctx.item.copy);
        *self.filter_event.borrow_mut() = Some(ctx.item.ctx.clone());
        let mut spins = 0u32;
        loop {
            spins += 1;
            assert!(spins < 100_000, "resolution of '{}' never finishes", self.db.def(ctx.item.def).name);
            if self.is_over() {
                return Flow::Continue;
            }
            // a copy choice asked in the middle of this resolution is answered before the rest of it runs
            if let Some(Sub::EnterCopy { card, options }) = ctx.sub.clone() {
                ctx.sub = None;
                let answer = self.answer.take();
                self.apply_enter_copy(card, &options, answer);
                self.layers_dirty = true;
            }
            // find next effect
            let Some(frame) = ctx.frames.last_mut() else {
                self.finish_resolve(*ctx);
                return Flow::Continue;
            };
            if frame.pc >= frame.list.len() {
                // end of the list: repeat or next foreach item or pop
                if frame.repeat > 0 {
                    frame.repeat -= 1;
                    frame.pc = 0;
                    continue;
                }
                if frame.each_active && !frame.each.is_empty() {
                    let next = frame.each.remove(0);
                    frame.pc = 0;
                    ctx.remembered = vec![next];
                    self.each_current = vec![next];
                    continue;
                }
                ctx.frames.pop();
                continue;
            }
            let list = frame.list.clone();
            let pc = frame.pc;
            ctx.cur_tbase = frame.tbase;
            // earlier effects of this resolution (a pump) are visible to later ones (damage equal to power)
            if self.layers_dirty {
                self.recompute_layers();
            }
            let eff = &list[pc];
            self.zone_batch.clear();
            let is_draw = matches!(eff, Effect::Draw { .. });
            let is_move_bf = matches!(eff, Effect::Move { to: Zone::Battlefield, .. });
            let stack_before = self.stack.len();
            match self.exec(&mut ctx, eff) {
                Exec::Done => {
                    let top = ctx.frames.len().wrapping_sub(1);
                    if let Some(f) = ctx.frames.get_mut(top) {
                        // the frame may have been replaced by a push; only advance the frame that ran
                        if Arc::ptr_eq(&f.list, &list) && f.pc == pc {
                            f.pc += 1;
                        }
                    }
                    // a permanent this effect put onto the battlefield enters as a copy: the choice is made now, so the rest of the effect sees the finished permanent
                    if is_move_bf && !ctx.item.copy {
                        let waiting: Vec<CardId> = ctx.remembered.iter().filter_map(|o| if let Obj::Card { id } = o { Some(*id) } else { None }).filter(|id| self.enter_choices.contains(id)).collect();
                        for id in waiting {
                            if let Some((dec, options)) = self.enter_copy_decision(id) {
                                self.pending = Some(dec);
                                ctx.sub = Some(Sub::EnterCopy { card: id, options });
                                self.run = Run::Resolve(ctx);
                                return Flow::Wait;
                            }
                        }
                    }
                    // a replaced draw (Abundance, dredge ...) went onto the stack: the rest of this ability happens after it ("draw a card, then discard a card")
                    if is_draw && self.stack.len() > stack_before && ctx.frames.len() == 1 && ctx.targets.is_empty() && !ctx.item.copy {
                        let f = &mut ctx.frames[0];
                        if f.pc < f.list.len() && f.repeat == 0 && !f.each_active {
                            let rest: Vec<Effect> = f.list[f.pc..].to_vec();
                            f.pc = f.list.len();
                            let did = self.next_effect_id;
                            self.next_effect_id += 1;
                            let d = DelayedTrigger {
                                id: did,
                                controller: ctx.item.controller,
                                source: ctx.item.source,
                                when: DelayedWhen::EndOfTurn,
                                effects: rest,
                                created_turn: self.turn,
                                targets: vec![],
                                ctx: ctx.item.ctx.clone(),
                                remembered: ctx.remembered.clone(),
                            };
                            self.push_delayed_item(d);
                            if let Some(it) = self.stack.pop() {
                                self.stack.insert(stack_before, it);
                            }
                        }
                    }
                }
                Exec::Push(new) => {
                    // advance the parent past the container effect, then run the nested list
                    if let Some(f) = ctx.frames.last_mut() {
                        f.pc += 1;
                    }
                    let mut new = new;
                    if new.each_active {
                        if new.each.is_empty() {
                            continue;
                        }
                        let first = new.each.remove(0);
                        ctx.remembered = vec![first];
                        self.each_current = vec![first];
                    }
                    ctx.frames.push(new);
                }
                Exec::Wait => {
                    self.run = Run::Resolve(ctx);
                    return Flow::Wait;
                }
            }
        }
    }

    fn finish_resolve(&mut self, ctx: ResolveCtx) {
        self.pay_upkeep.set(false);
        let item = ctx.item;
        if self.resolve_ctl_item == Some(item.source) {
            self.resolve_ctl = None;
            self.resolve_ctl_item = None;
        }
        match &item.kind {
            ItemKind::Spell { alt, .. } => {
                let card = item.source;
                let exile_after = {
                    let d = self.db.def(self.cards[card.0 as usize].def);
                    alt.map_or(false, |i| {
                        d.abilities
                            .iter()
                            .filter_map(|a| if let Ability::AlternativeCost { exile_after, .. } = a { Some(*exile_after) } else { None })
                            .nth(i as usize)
                            .unwrap_or(false)
                    }) || self.cards[card.0 as usize].cast_exile_after
                };
                // a copy of a spell resolves on its own even when the card it was copied from has already left the stack (countered or resolved)
                if (self.cards[card.0 as usize].zone == Zone::Stack && self.cards[card.0 as usize].zcc == item.src_zcc) || item.copy {
                    if ctx.fizzled {
                        self.emit(Event::Fizzled { card });
                    }
                    let dbc = self.db.clone();
                    let d = dbc.def(self.cards[card.0 as usize].def);
                    let permanent = d.is_permanent();
                    // bestow: cast as an Aura; if its target is gone it resolves as a creature
                    let bestow = alt.map_or(false, |i| {
                        d.abilities.iter().filter(|a| matches!(a, Ability::AlternativeCost { .. })).nth(i as usize).map_or(false, |a| {
                            matches!(a, Ability::AlternativeCost { effects: Some(e), targets: Some(_), .. } if e.as_slice() == [Effect::Nothing] || matches!(e.first(), Some(Effect::MutateOnto { .. })))
                        })
                    });
                    if permanent && (!ctx.fizzled || bestow) && !item.copy {
                        let host = ctx.targets.first().and_then(|t| if let Obj::Card { id } = t.obj { Some(id) } else { None });
                        let is_attach = *alt != Some(FACE_DOWN_ALT) && d.abilities.iter().any(|a| matches!(a, Ability::Attach { .. } | Ability::AttachGraveyard { .. }));
                        let is_reanim = d.abilities.iter().any(|a| matches!(a, Ability::AttachGraveyard { stay: false, .. }));
                        let reanim_tapped = d.abilities.iter().any(|a| matches!(a, Ability::AttachGraveyard { tapped: true, .. }));
                        let host = if is_reanim && !ctx.fizzled {
                            match host.filter(|h| matches!(self.cards[h.0 as usize].zone, Zone::Graveyard)) {
                                Some(h) => {
                                    self.move_card(h, Zone::Battlefield, MoveOpts { controller: Some(item.controller), tapped: reanim_tapped, ..Default::default() });
                                    (self.cards[h.0 as usize].zone == Zone::Battlefield).then_some(h)
                                }
                                None => None,
                            }
                        } else if is_reanim {
                            None
                        } else {
                            host
                        };
                        let enchant_player = d.abilities.iter().any(|a| matches!(a, Ability::EnchantPlayer | Ability::EnchantOpponent));
                        let player_host = ctx.targets.first().and_then(|t| if let Obj::Player { id } = t.obj { Some(id) } else { None });
                        if enchant_player {
                            if let Some(pl) = player_host {
                                self.cards[card.0 as usize].x = item.x;
                                self.move_card(card, Zone::Battlefield, MoveOpts { controller: Some(item.controller), ..Default::default() });
                                // a daybound Curse entering at night has already transformed into its creature face
                                if self.def_of(card).abilities.iter().any(|a| matches!(a, Ability::EnchantPlayer | Ability::EnchantOpponent)) {
                                    self.cards[card.0 as usize].enchanted_player = Some(pl);
                                }
                                self.layers_dirty = true;
                                self.emit(Event::Resolved { card });
                            } else {
                                self.move_card(card, Zone::Graveyard, Default::default());
                            }
                        } else if is_attach && host.is_none() {
                            self.move_card(card, Zone::Graveyard, Default::default());
                        } else {
                            self.cards[card.0 as usize].x = item.x;
                            self.cards[card.0 as usize].cast_remembered = item.remembered.clone();
                            self.move_card(card, Zone::Battlefield, MoveOpts { controller: Some(item.controller), face_down: *alt == Some(FACE_DOWN_ALT), attach_to: if is_attach { host } else { None }, ..Default::default() });
                            if is_attach {
                                if let Some(h) = host {
                                    self.attach(card, Some(h));
                                }
                            }
                            if bestow && !ctx.fizzled {
                                if let Some(h) = host {
                                    if self.cards[h.0 as usize].zone == Zone::Battlefield {
                                        self.cards[card.0 as usize].bestowed = true;
                                        self.attach(card, Some(h));
                                        self.layers_dirty = true;
                                    }
                                }
                            }
                            self.emit(Event::Resolved { card });
                        }
                    } else if item.copy && permanent && !ctx.fizzled {
                        // a copy of a permanent spell becomes a token (707.10b)
                        let def = self.cards[card.0 as usize].def;
                        if !self.token_cap_reached() {
                            let id = self.create_card(def, item.controller, Zone::Exile, true);
                            self.players[item.controller as usize].exile.retain(|x| *x != id);
                            self.cards[id.0 as usize].zone = Zone::Library;
                            self.cards[id.0 as usize].x = item.x;
                            // 707.10: the copy keeps the choices made on casting (kicker)
                            let (k, km, kb, kc) = { let o = &self.cards[card.0 as usize]; (o.kicked, o.kicked_mask, o.kick_by, o.kicks) };
                            { let t = &mut self.cards[id.0 as usize]; t.kicked = k; t.kicked_mask = km; t.kick_by = kb; t.kicks = kc; }
                            // a copy of an Aura spell enters attached to the object it targets (the copy's own target, possibly retargeted)
                            let aura_host = if d.abilities.iter().any(|a| matches!(a, Ability::Attach { .. })) {
                                item.targets.first().and_then(|t| if let Obj::Card { id } = t.obj { Some(id) } else { None }).filter(|h| self.cards[h.0 as usize].zone == Zone::Battlefield)
                            } else {
                                None
                            };
                            self.move_card(id, Zone::Battlefield, MoveOpts { controller: Some(item.controller), attach_to: aura_host, ..Default::default() });
                            // "starting loyalty X": amounts that read the copied spell's event context (the creature sacrificed for casualty) are fixed now
                            let except: Vec<CopyExcept> = item
                                .copy_except
                                .iter()
                                .map(|x| match x {
                                    CopyExcept::SetLoyalty { amount } => {
                                        let mut ec = crate::eval::EvalCtx::with_controller(card, item.controller);
                                        ec.event = &item.ctx;
                                        CopyExcept::SetLoyalty { amount: Amount::Fixed { n: self.eval_amount(amount, &ec) } }
                                    }
                                    other => other.clone(),
                                })
                                .collect();
                            self.apply_copy_except(card, item.controller, id, &except, &[]);
                            self.emit(Event::Token { card: id });
                        }
                    } else if item.copy {
                        // a copy of a spell just ceases to exist; the card it was copied from is unaffected
                    } else {
                        // an adventure spell is exiled instead, to be cast later as the creature
                        let adventure = !item.copy && !ctx.fizzled && self.db.def(self.cards[card.0 as usize].front).adventure && self.cards[card.0 as usize].def != self.cards[card.0 as usize].front;
                        let marked = self.exile_marked.iter().position(|c| *c == card);
                        if let Some(i) = marked {
                            self.exile_marked.remove(i);
                        }
                        // an Omen spell that resolved is shuffled into its owner's library instead
                        let omen = !item.copy && !ctx.fizzled && !exile_after && marked.is_none() && self.db.def(self.cards[card.0 as usize].front).omen && self.cards[card.0 as usize].def != self.cards[card.0 as usize].front;
                        let dest = if omen { Zone::Library } else if exile_after || adventure || marked.is_some() { Zone::Exile } else { Zone::Graveyard };
                        self.resolving_spell.set(Some(card));
                        self.resolved_ok.set(!ctx.fizzled);
                        if dest == Zone::Exile {
                            // exiled through a mana rider: the card is exiled with the permanent that made the mana
                            if let Some(link) = self.cards[card.0 as usize].exile_link.take() {
                                self.repl_exile_src.set(Some(link));
                            }
                        }
                        self.move_card(card, dest, Default::default());
                        self.resolved_ok.set(false);
                        self.resolving_spell.set(None);
                        if adventure && self.cards[card.0 as usize].zone == Zone::Exile {
                            self.cards[card.0 as usize].on_adventure = true;
                            self.cards[card.0 as usize].adventure_by = Some(item.controller);
                        }
                        if omen && self.cards[card.0 as usize].zone == Zone::Library {
                            let owner = self.cards[card.0 as usize].owner;
                            self.shuffle_library(owner);
                        }
                        self.emit(Event::Resolved { card });
                    }
                }
            }
            ItemKind::Triggered { ability, .. } if *ability < crate::cast::GRANTED_BASE && !ctx.fizzled => {
                // the final chapter ability of a Saga finished resolving
                let dbc = self.db.clone();
                let d = dbc.def(item.def);
                if d.saga_max > 0 {
                    if let Some(Ability::Triggered { trigger: Trigger::SagaChapter { chapter }, .. }) = d.abilities.get(*ability as usize) {
                        if *chapter == d.saga_max {
                            self.emit(Event::SagaFinalChapter { card: item.source });
                        }
                    }
                }
            }
            _ => {}
        }
        self.run = Run::Priority { player: self.active, passes: 0 };
        if !self.alive(self.active) {
            self.run = Run::Priority { player: self.next_player(self.active), passes: 0 };
        }
        *self.filter_event.borrow_mut() = None;
        self.cause_ctl.set(None);
        self.cause_src.set(None);
        self.cause_trigger.set(false);
        self.cause_targets.borrow_mut().clear();
        if self.end_turn_pending {
            self.do_end_turn();
            return;
        }
        if self.end_combat_pending {
            self.do_end_combat();
            return;
        }
        // a cascade-style effect finished: the chosen card is cast now, before anyone gets priority
        self.start_next_free_cast();
    }

    /// A token card with the same definition as `orig`, ready to be cast as a copy ("copy that card and cast the copy"); it ceases to exist when it leaves the stack or battlefield.
    pub(crate) fn make_cast_copy(&mut self, orig: CardId, player: PlayerId) -> CardId {
        let def = self.cards[orig.0 as usize].def;
        let id = self.create_card(def, player, Zone::Exile, true);
        self.players[player as usize].exile.retain(|x| *x != id);
        id
    }

    /// The index (among the card's alternative costs) of the never-offered alternative cost that stands for its suspend cost (`CastFree.suspend`).
    pub(crate) fn suspend_alt(&self, card: CardId) -> Option<u8> {
        let d = self.def_of(card);
        let cost = d.abilities.iter().find_map(|a| if let Ability::Suspend { cost, .. } = a { Some(cost.clone()) } else { None })?;
        d.abilities
            .iter()
            .filter(|a| matches!(a, Ability::AlternativeCost { .. }))
            .position(|a| matches!(a, Ability::AlternativeCost { cost: c, condition: Some(Condition::AnyOf { cs }), .. } if cs.is_empty() && c.as_slice() == [Cost::Mana { cost: cost.clone() }]))
            .map(|i| i as u8)
    }

    /// "You still pay its costs" casts (Harness the Storm): when the normal mana cost can't be paid, use the first printed alternative cost the player can pay (Snuff Out's 4 life).
    fn free_cast_alt(&self, p: PlayerId, card: CardId, pay: bool, alt: Option<u8>) -> Option<u8> {
        if !pay || alt.is_some() || self.can_afford(p, &self.effective_cost(p, card, None), 0) {
            return alt;
        }
        let d = self.def_of(card);
        let mut ai = 0u8;
        for a in &d.abilities {
            if let Ability::AlternativeCost { from: None, condition, cost, effects: None, .. } = a {
                let cond_ok = condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &EvalCtx::with_controller(card, p)));
                if cond_ok && self.can_pay_costs(p, card, cost) {
                    return Some(ai);
                }
            }
            if matches!(a, Ability::AlternativeCost { .. }) {
                ai += 1;
            }
        }
        alt
    }

    /// Begin casting the next card queued by a "cast without paying" effect (several at once: the first now, the rest as each cast finishes).
    pub(crate) fn start_next_free_cast(&mut self) {
        while !self.free_casts.is_empty() {
            let (p, card, pay, alt) = self.free_casts.remove(0);
            if !matches!(self.cards[card.0 as usize].zone, Zone::Stack | Zone::Battlefield) {
                self.free_cast_now.set(!pay);
                self.begin_cast(p, card, alt);
                return;
            }
        }
    }

    /// An Aura with `AttachGraveyard { stay: true }` may be attached to `host` while it is in a graveyard and matches the Aura's "enchant" filter.
    pub(crate) fn gy_aura_host_ok(&self, aura: CardId, host: CardId) -> bool {
        if self.cards[host.0 as usize].zone != Zone::Graveyard || host == aura {
            return false;
        }
        let ctrl = self.cards[aura.0 as usize].cur.controller;
        self.db
            .def(self.cards[aura.0 as usize].def)
            .abilities
            .iter()
            .find_map(|a| if let Ability::AttachGraveyard { to, stay: true, .. } = a { Some(to.clone()) } else { None })
            .map_or(false, |mut f| {
                f.controller = PlayerFilter::Any;
                self.matches_card(&f, host, aura, ctrl)
            })
    }

    /// How an Aura put onto the battlefield without being cast finds what it enchants (CR 303.4f: the player putting it
    /// there chooses): no legal permanent, exactly one, or several (it enters and its controller is asked before anyone
    /// gets priority, see `attach_pending`).
    pub(crate) fn aura_host_plan(&self, id: CardId) -> AuraHostPlan {
        match self.aura_hosts(id) {
            None => AuraHostPlan::NotAura,
            Some(h) if h.is_empty() => AuraHostPlan::NoHost,
            Some(h) if h.len() == 1 => AuraHostPlan::One(h[0]),
            Some(_) => AuraHostPlan::Choose,
        }
    }

    pub fn attach(&mut self, card: CardId, to: Option<CardId>) {
        // only permanents on the battlefield can be attached to one another
        if self.cards[card.0 as usize].zone != Zone::Battlefield {
            return;
        }
        if let Some(h) = to {
            if (self.cards[h.0 as usize].zone != Zone::Battlefield && !self.gy_aura_host_ok(card, h)) || h == card {
                return;
            }
            // attaching to the permanent it is already attached to does nothing (no new "becomes attached" event)
            if self.cards[card.0 as usize].attached_to == Some(h) && self.cards[card.0 as usize].cur.subtypes.contains(&Sym::new("Equipment")) {
                return;
            }
        }
        if let Some(h) = to {
            // an Equipment that is also a creature can't be attached to anything (CR 301.5c; Reconfigure ends its creature type while attached)
            if self.cards[card.0 as usize].cur.subtypes.contains(&Sym::new("Equipment")) && self.cards[card.0 as usize].cur.types.has(CardType::Creature) && !self.cards[card.0 as usize].cur.kw.has(Keyword::Reconfigure)
                && !self.def_of(card).abilities.iter().any(|a| matches!(a, Ability::Activated { effects, .. } if effects.iter().any(|e| matches!(e, Effect::Attach { what: Sel::This, .. })))) {
                return;
            }
            if self.cards[card.0 as usize].cur.subtypes.contains(&Sym::new("Equipment")) && self.cant_be_equipped(h) {
                return;
            }
        }
        let old = self.cards[card.0 as usize].attached_to;
        self.cards[card.0 as usize].attached_to = to;
        self.layers_dirty = true;
        // an Aura that changes control of its host does so before the "becomes attached" trigger is controlled (Brood Keeper)
        if to.is_some() {
            self.recompute_layers();
        }
        self.emit(Event::Attach { card, to });
        if let (Some(from), true) = (old, to != old) {
            self.emit(Event::Unattached { card, from });
        }
    }

    fn evalctx<'a>(&self, ctx: &'a ResolveCtx) -> EvalCtx<'a> {
        // filters evaluated during this effect can refer to the remembered objects ("a creature with lesser mana value than the sacrificed one")
        *self.filter_rem.borrow_mut() = ctx.remembered.clone();
        *self.filter_targets.borrow_mut() = (ctx.targets.to_vec(), ctx.tidx[ctx.cur_tbase.min(ctx.tidx.len())..].to_vec());
        EvalCtx {
            source: ctx.item.source,
            controller: ctx.item.controller,
            x: ctx.item.x,
            targets: &ctx.targets,
            tidx: &ctx.tidx[ctx.cur_tbase.min(ctx.tidx.len())..],
            event: &ctx.item.ctx,
            remembered: &ctx.remembered,
        }
    }

    /// Remove (or, with `dst`, move) counters one at a time, asking the controller which kind whenever the permanent holds several. `pick` is the answer to the question just asked.
    fn counter_kind_job(&mut self, ctx: &mut ResolveCtx, source: CardId, controller: PlayerId, mut ids: Vec<CardId>, mut left: i32, n_each: i32, dst: Option<CardId>, mut pick: Option<CounterKind>, up_to: bool) -> Exec {
        loop {
            let Some(&id) = ids.first() else { return Exec::Done };
            let gone = self.cards[id.0 as usize].zone != Zone::Battlefield;
            let pool: Vec<(CounterKind, u16)> = if gone { self.cards[id.0 as usize].left_counters.iter().copied().collect() } else { self.cards[id.0 as usize].counters.iter().copied().collect() };
            let kinds: Vec<CounterKind> = pool.iter().filter(|(_, x)| *x > 0).map(|(k, _)| *k).collect();
            if left <= 0 || kinds.is_empty() || (gone && dst.is_none()) {
                ids.remove(0);
                left = n_each;
                continue;
            }
            let k = if let Some(k) = pick.take().filter(|k| kinds.contains(k)) {
                k
            } else if kinds.len() == 1 && (!up_to || (self.kind_pick_min1.get() && self.counters_removed.get() == 0)) {
                kinds[0]
            } else {
                ctx.sub = Some(Sub::KindPick { ids, left, n_each, kinds: kinds.clone(), dst, up_to });
                self.pending = Some(Decision {
                    player: controller,
                    kind: DecisionKind::Pick {
                        prompt: "Choose the kind of counter".into(),
                        source: Some(source),
                        options: kinds.iter().map(|k| Opt::Text { text: k.label() }).chain((up_to && !(self.kind_pick_min1.get() && self.counters_removed.get() == 0)).then(|| Opt::Text { text: "Stop".into() })).collect(),
                        min: 1,
                        max: 1,
                        cancellable: false,
                        hidden: false,
                        purpose: PickPurpose::Mode,
                    },
                });
                return Exec::Wait;
            };
            if let Some(d) = dst {
                if !gone {
                    self.remove_counters_from(Obj::card(id), k, 1);
                } else if let Some(e) = self.cards[id.0 as usize].left_counters.iter_mut().find(|(kk, _)| *kk == k) {
                    e.1 = e.1.saturating_sub(1);
                }
                self.add_counters_to(Obj::card(d), k, 1);
            } else {
                self.remove_counters_from(Obj::card(id), k, 1);
            }
            left -= 1;
        }
    }

    fn exec(&mut self, ctx: &mut ResolveCtx, e: &Effect) -> Exec {
        if let Effect::AddMana { .. } = e {
            // False Dawn: colored mana added by this effect becomes white
            let p = ctx.item.controller;
            let before = self.players[p as usize].pool;
            let r = self.exec_inner(ctx, e);
            self.convert_added_mana_to_white(p, before);
            return r;
        }
        self.exec_inner(ctx, e)
    }

    fn exec_inner(&mut self, ctx: &mut ResolveCtx, e: &Effect) -> Exec {
        let controller = ctx.item.controller;
        let source = ctx.item.source;
        *self.exec_spell.borrow_mut() = if matches!(ctx.item.kind, ItemKind::Spell { .. }) && !ctx.item.targets.is_empty() { Some((source, ctx.item.targets.clone())) } else { None };
        self.cur_src = Some(source);
        self.cause_ctl.set(Some(controller));
        self.cause_src.set(Some(source));
        self.cause_trigger.set(matches!(ctx.item.kind, ItemKind::Triggered { .. } | ItemKind::Delayed { .. }));
        *self.cause_targets.borrow_mut() = ctx.item.targets.iter().map(|t| t.obj).collect();
        macro_rules! ec {
            () => {
                self.evalctx(ctx)
            };
        }
        match e {
            Effect::Nothing => Exec::Done,
            Effect::DealDamage { to, amount, source: src, unpreventable, no_regen } => {
                let (objs, n, src_card) = {
                    let c = ec!();
                    let src_card = src.as_ref().and_then(|s| self.sel_cards(s, &c).first().copied()).unwrap_or(source);
                    (self.sel_objs(to, &c), self.eval_amount(amount, &c), src_card)
                };
                // a damage effect with no legal recipient left deals nothing: the previous damage figures do not carry over (Contest of Claws)
                if objs.is_empty() {
                    self.last_damage.set((0, 0));
                }
                if let Some(Sub::RedirectPick { options }) = ctx.sub.clone() {
                    ctx.sub = None;
                    let i = match self.answer.take() {
                        Some(Choice::Pick { indices }) => indices.first().copied().unwrap_or(0),
                        _ => 0,
                    };
                    self.redirect_pick.set(options.get(i).or(options.first()).copied());
                } else if let (Some(&Obj::Player { id: dp }), true) = (objs.first(), n > 0) {
                    let mut options: Vec<CardId> = vec![];
                    for r in self.active_replacements() {
                        if let Replace::Damage { to: DamageTo::You, prevent: true, then, .. } = &r.what {
                            if r.controller == dp && !then.is_empty() {
                                if let Some(h) = self.cards[r.source.0 as usize].attached_to {
                                    if !options.contains(&h) {
                                        options.push(h);
                                    }
                                }
                            }
                        }
                    }
                    if options.len() > 1 {
                        ctx.sub = Some(Sub::RedirectPick { options: options.clone() });
                        self.pending = Some(Decision {
                            player: dp,
                            kind: DecisionKind::Pick {
                                prompt: "Choose which creature the damage is dealt to instead".into(),
                                source: Some(source),
                                options: options.iter().map(|o| Opt::Obj { obj: Obj::card(*o) }).collect(),
                                min: 1,
                                max: 1,
                                cancellable: false,
                                hidden: false,
                                purpose: PickPurpose::Generic,
                            },
                        });
                        return Exec::Wait;
                    }
                }
                let was = self.force_unprevent.replace(*unpreventable);
                for o in objs.iter().copied() {
                    let dealt = self.deal_damage(Some(src_card), o, n, false);
                    if *no_regen && dealt > 0 {
                        if let Obj::Card { id, .. } = o {
                            self.cards[id.0 as usize].no_regen_turn = self.turn;
                        }
                    }
                }
                self.force_unprevent.set(was);
                self.redirect_pick.set(None);
                Exec::Done
            }
            Effect::DealDamageExcessToController { to, amount, source: src } => {
                let (objs, n, src_card) = {
                    let c = ec!();
                    let src_card = src.as_ref().and_then(|s| self.sel_cards(s, &c).first().copied()).unwrap_or(source);
                    (self.sel_objs(to, &c), self.eval_amount(amount, &c), src_card)
                };
                let mut last = (0, 0);
                for o in objs {
                    let (mut to_creature, mut excess) = (n, 0);
                    let mut ctrl = None;
                    if let Obj::Card { id } = o {
                        let c = &self.cards[id.0 as usize];
                        if c.zone == Zone::Battlefield && c.cur.types.has(CardType::Creature) {
                            let mut left = (self.lethal_threshold(id) - c.damage as i32).max(0);
                            if self.cards[src_card.0 as usize].cur.kw.has(Keyword::Deathtouch) && n > 0 {
                                left = left.min(1);
                            }
                            to_creature = n.min(left);
                            excess = n - to_creature;
                            ctrl = Some(c.cur.controller);
                        }
                    }
                    let dealt = if to_creature > 0 || excess == 0 { self.deal_damage(Some(src_card), o, to_creature, false) } else { 0 };
                    let mut ex = 0;
                    if let (Some(p), true) = (ctrl, excess > 0) {
                        self.deal_damage(Some(src_card), Obj::player(p), excess, false);
                        ex = excess;
                    }
                    last = (dealt, ex);
                }
                self.last_damage.set(last);
                Exec::Done
            }
            Effect::EachDealsDamage { sources, to, amount } => {
                let (srcs, objs, n) = {
                    let c = ec!();
                    (self.sel_cards(sources, &c), self.sel_objs(to, &c), self.eval_amount(amount, &c))
                };
                for s in srcs {
                    for &o in &objs {
                        self.deal_damage(Some(s), o, n, false);
                    }
                }
                Exec::Done
            }
            Effect::EmitBend { kind } => {
                self.emit(Event::Bent { player: controller, kind: *kind });
                Exec::Done
            }
            Effect::DealDamageAgain { to, combat, plain, less, keep_combat } => {
                let (objs, n, src_card) = {
                    let c = ec!();
                    let x = less.as_ref().map_or(0, |a| self.eval_amount(a, &c).max(0));
                    (self.sel_objs(to, &c), (c.event.amount - x).max(0), c.event.source.unwrap_or(source))
                };
                let was = self.skip_opt_redirect.replace(*plain);
                for o in objs {
                    self.deal_damage(Some(src_card), o, n, *combat || (*keep_combat && ctx.item.ctx.dmg_combat));
                }
                self.skip_opt_redirect.set(was);
                Exec::Done
            }
            Effect::Draw { who, n, plain } => {
                let (ps, k) = {
                    let c = ec!();
                    (self.sel_players(who, &c), self.eval_amount(n, &c))
                };
                // a draw from a cycling ability (the cycled card sits in the graveyard): replacements may ask about the cycled card
                let cycled = Some(source).filter(|&s| {
                    matches!(ctx.item.kind, ItemKind::Ability { .. })
                        && self.cards[s.0 as usize].zone == Zone::Graveyard
                        && !self.def_of(s).types.has(CardType::Land)
                        && self.matches_card(&Filter { has_keyword_word: Some("cycling".into()), ..Default::default() }, s, s, controller)
                });
                self.draw_cycled.set(cycled);
                // a plain draw made by a draw replacement only skips the replacements already applied on the way to it
                let prev_chain = std::mem::replace(&mut *self.draw_chain.borrow_mut(), if *plain { ctx.item.ctx.cards.clone() } else { vec![] });
                let chained = !self.draw_chain.borrow().is_empty();
                for p in ps {
                    if k >= 2 && !*plain {
                        if let Some((src, ctrl, effects, tgts, cyc)) = self.draw_instead(p, false, k) {
                            self.dmg_followups.borrow_mut().push((src, ctrl, effects, 1, cyc, None, Some(p), tgts));
                            self.drain_consumed_replaces();
                            self.flush_followups();
                            continue;
                        }
                    }
                    for i in 0..k.max(0) {
                        // a replaced draw (dredge ...) is dealt with before the next draw of the same effect is looked at: the remaining draws wait
                        // underneath the replacement's follow-up on the stack (each draw is replaced on its own, with its own question)
                        if (!*plain || chained) && i + 1 < k {
                            let mark = self.consumed_temp.borrow().len();
                            let replaced = self.draw_instead(p, false, 1).is_some();
                            self.consumed_temp.borrow_mut().truncate(mark);
                            if replaced {
                                let rest = Effect::Draw { who: Sel::TriggerPlayer, n: Amount::Fixed { n: k - i - 1 }, plain: *plain };
                                self.push_draw_followup(source, controller, vec![rest], None, p, vec![], false);
                                self.draw_with_replacement(p, false, *plain);
                                break;
                            }
                        }
                        self.draw_with_replacement(p, false, *plain);
                    }
                    self.flush_followups();
                }
                *self.draw_chain.borrow_mut() = prev_chain;
                self.draw_cycled.set(None);
                Exec::Done
            }
            Effect::StartDrawCount => {
                self.draw_log.borrow_mut().clear();
                Exec::Done
            }
            Effect::DrawRemember { who, n } => {
                let (ps, k) = {
                    let c = ec!();
                    (self.sel_players(who, &c), self.eval_amount(n, &c))
                };
                self.draw_log.borrow_mut().clear();
                for p in ps {
                    for _ in 0..k.max(0) {
                        self.draw_with_replacement(p, false, false);
                    }
                    self.flush_followups();
                }
                let drawn: Vec<CardId> = std::mem::take(&mut *self.draw_log.borrow_mut());
                ctx.remembered = drawn.into_iter().filter(|id| self.cards[id.0 as usize].zone == Zone::Hand).map(Obj::card).collect();
                Exec::Done
            }
            Effect::GainLife { who, n } => {
                let (ps, k) = {
                    let c = ec!();
                    (self.sel_players(who, &c), self.eval_amount(n, &c))
                };
                // a negative amount (toughness below zero) gains nothing; it never loses life
                let k = k.max(0);
                for p in ps {
                    self.gain_life(p, k);
                }
                self.flush_followups();
                Exec::Done
            }
            Effect::LoseLife { who, n } => {
                let (ps, k) = {
                    let c = ec!();
                    (self.sel_players(who, &c), self.eval_amount(n, &c))
                };
                let mut lost = 0;
                for p in ps {
                    let before = self.players[p as usize].life;
                    self.lose_life(p, k);
                    lost += (before - self.players[p as usize].life).max(0);
                }
                self.last_life_lost.set(lost);
                Exec::Done
            }
            Effect::SetLife { who, n } => {
                let (ps, k) = {
                    let c = ec!();
                    (self.sel_players(who, &c), self.eval_amount(n, &c))
                };
                for p in ps {
                    let cur = self.players[p as usize].life;
                    if k > cur {
                        self.gain_life(p, k - cur)
                    } else {
                        self.lose_life(p, cur - k)
                    }
                }
                Exec::Done
            }
            Effect::GrantSuspend { what } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                for id in cs {
                    let has = self.def_of(id).abilities.iter().any(|a| matches!(a, Ability::Suspend { .. }));
                    if self.cards[id.0 as usize].zone == Zone::Exile && !has {
                        self.cards[id.0 as usize].granted_suspend = true;
                    }
                }
                Exec::Done
            }
            Effect::ReverseTurnOrder => {
                self.turn_reversed = !self.turn_reversed;
                Exec::Done
            }
            Effect::ExileSpellDelayed { what, counters } => {
                let (ids, n): (Vec<StackId>, i32) = {
                    let c = ec!();
                    let ids = self
                        .sel_objs(what, &c)
                        .into_iter()
                        .filter_map(|o| match o {
                            Obj::Stack { id } => Some(id),
                            Obj::Card { id } if self.cards[id.0 as usize].zone == Zone::Stack => self.stack.iter().rev().find(|s| s.source == id && !s.copy && matches!(s.kind, ItemKind::Spell { .. })).map(|s| s.id),
                            _ => None,
                        })
                        .collect();
                    (ids, self.eval_amount(counters, &c).max(0))
                };
                for sid in ids {
                    let Some(pos) = self.stack.iter().position(|s| s.id == sid) else { continue };
                    if !matches!(self.stack[pos].kind, ItemKind::Spell { .. }) {
                        continue;
                    }
                    let item = self.stack.remove(pos);
                    if item.copy {
                        continue;
                    }
                    self.move_card(item.source, Zone::Exile, Default::default());
                    if self.cards[item.source.0 as usize].zone == Zone::Exile {
                        self.add_counters_to(Obj::card(item.source), CounterKind::Delay, n);
                        let zcc = self.cards[item.source.0 as usize].zcc;
                        self.meddled.push((item.source, zcc, item.controller, item));
                    }
                }
                Exec::Done
            }
            Effect::Encode => {
                if let Some(Sub::Encode { options }) = ctx.sub.clone() {
                    ctx.sub = None;
                    if let Some(Choice::Pick { indices }) = self.answer.take() {
                        if let Some(&cr) = indices.first().and_then(|i| options.get(*i)) {
                            let zcc = self.cards[cr.0 as usize].zcc;
                            self.encoded.push((source, cr, zcc));
                            self.exile_marked.push(source);
                        }
                    }
                    return Exec::Done;
                }
                if ctx.item.copy || !matches!(ctx.item.kind, ItemKind::Spell { .. }) {
                    return Exec::Done;
                }
                let options: Vec<CardId> = self.battlefield.iter().copied().filter(|id| self.cards[id.0 as usize].cur.controller == controller && self.cards[id.0 as usize].cur.types.has(CardType::Creature)).collect();
                if options.is_empty() {
                    return Exec::Done;
                }
                ctx.sub = Some(Sub::Encode { options: options.clone() });
                self.pending = Some(Decision {
                    player: controller,
                    kind: DecisionKind::Pick {
                        prompt: format!("Encode {} on a creature you control?", self.def_of(source).name),
                        source: Some(source),
                        options: options.iter().map(|c| Opt::Obj { obj: Obj::card(*c) }).collect(),
                        min: 0,
                        max: 1,
                        cancellable: false,
                        hidden: false,
                        purpose: PickPurpose::Generic,
                    },
                });
                Exec::Wait
            }
            Effect::TimeTravel => {
                let mut changes: Vec<(CardId, i32)> = vec![];
                let mut queue: Vec<CardId>;
                if let Some(Sub::TimeTravel { queue: q, cur, changes: mut ch }) = ctx.sub.clone() {
                    ctx.sub = None;
                    if let Some(Choice::Pick { indices }) = self.answer.take() {
                        match indices.first() {
                            Some(0) => ch.push((cur, 1)),
                            Some(1) => ch.push((cur, -1)),
                            _ => {}
                        }
                    }
                    queue = q;
                    changes = ch;
                } else {
                    // permanents you control with a time counter, and your cards in exile with one (suspended)
                    queue = self.battlefield.iter().copied().filter(|id| self.cards[id.0 as usize].cur.controller == controller && self.cards[id.0 as usize].counter(CounterKind::Time) > 0).collect();
                    queue.extend(self.players[controller as usize].exile.iter().copied().filter(|id| self.cards[id.0 as usize].counter(CounterKind::Time) > 0));
                }
                if queue.is_empty() {
                    for (id, d) in changes {
                        if d > 0 {
                            self.add_counters_to(Obj::card(id), CounterKind::Time, d);
                        } else {
                            self.remove_counters_from(Obj::card(id), CounterKind::Time, -d);
                        }
                    }
                    return Exec::Done;
                }
                let cur = queue.remove(0);
                let name = self.def_of(cur).name.clone();
                ctx.sub = Some(Sub::TimeTravel { queue, cur, changes });
                self.pending = Some(Decision {
                    player: controller,
                    kind: DecisionKind::Pick {
                        prompt: format!("Time travel: {name}"),
                        source: Some(source),
                        options: vec![Opt::Text { text: "Add a time counter".into() }, Opt::Text { text: "Remove a time counter".into() }, Opt::Text { text: "Do nothing".into() }],
                        min: 1,
                        max: 1,
                        cancellable: false,
                        hidden: false,
                        purpose: PickPurpose::Generic,
                    },
                });
                Exec::Wait
            }
            Effect::EndTheTurn => {
                self.end_turn_pending = true;
                Exec::Done
            }
            Effect::EndCombatPhase => {
                if self.combat.is_some() {
                    self.end_combat_pending = true;
                }
                Exec::Done
            }
            Effect::ExchangeLife { a, b } => {
                let (pa, pb) = {
                    let c = ec!();
                    (self.sel_players(a, &c).first().copied(), self.sel_players(b, &c).first().copied())
                };
                if let (Some(pa), Some(pb)) = (pa, pb) {
                    if pa != pb {
                        let (la, lb) = (self.players[pa as usize].life, self.players[pb as usize].life);
                        // a player who can't gain life can't exchange with a higher total (rule 118.8 / Sulfuric Vortex): nothing changes
                        let blocked = |g: &Game, p: PlayerId| g.effects.iter().any(|e| matches!(e.modif, Modif::NoLifeGain) && e.players.contains(&p)) || g.static_cant_gain_life(p) || g.replace_gain_life(p, 1) <= 0;
                        if (lb > la && blocked(self, pa)) || (la > lb && blocked(self, pb)) {
                            return Exec::Done;
                        }
                        for (p, cur, k) in [(pa, la, lb), (pb, lb, la)] {
                            if k > cur {
                                self.gain_life(p, k - cur)
                            } else {
                                self.lose_life(p, cur - k)
                            }
                        }
                    }
                }
                Exec::Done
            }
            Effect::DestroyBlockedBy { by, reanimate } => {
                let mut queue: Vec<PlayerId> = vec![];
                if let Some(Sub::ReanimateFrom { queue: q }) = ctx.sub.clone() {
                    ctx.sub = None;
                    queue = q;
                    if let Some(p) = queue.first().copied() {
                        let cands: Vec<CardId> = self.players[p as usize].graveyard.iter().copied().filter(|c| self.cards[c.0 as usize].cur.types.has(CardType::Creature)).collect();
                        if let Some(Choice::Pick { indices }) = self.answer.take() {
                            if let Some(&id) = indices.first().and_then(|i| cands.get(*i)) {
                                let owner = self.cards[id.0 as usize].owner;
                                self.move_card(id, Zone::Battlefield, MoveOpts { controller: Some(owner), ..Default::default() });
                            }
                        }
                        queue.remove(0);
                    }
                } else {
                    let wall = {
                        let c = ec!();
                        self.sel_cards(by, &c).into_iter().next()
                    };
                    let Some(wall) = wall else { return Exec::Done };
                    let mut victims: Vec<(CardId, PlayerId)> = vec![];
                    for &(b, a, z, ctl) in self.block_log.iter() {
                        if b != wall {
                            continue;
                        }
                        victims.retain(|v| v.0 != a);
                        if self.cards[a.0 as usize].zone == Zone::Battlefield && self.cards[a.0 as usize].zcc == z {
                            victims.push((a, ctl));
                        }
                    }
                    self.begin_batch();
                    for &(a, ctl) in &victims {
                        self.destroy_opts(a, false);
                        if *reanimate && self.cards[a.0 as usize].zone == Zone::Graveyard {
                            queue.push(ctl);
                        }
                    }
                    self.end_batch();
                }
                while let Some(&p) = queue.first() {
                    let cands: Vec<CardId> = self.players[p as usize].graveyard.iter().copied().filter(|c| self.cards[c.0 as usize].cur.types.has(CardType::Creature)).collect();
                    match cands.len() {
                        0 => {}
                        1 => {
                            let id = cands[0];
                            let owner = self.cards[id.0 as usize].owner;
                            self.move_card(id, Zone::Battlefield, MoveOpts { controller: Some(owner), ..Default::default() });
                        }
                        _ => {
                            ctx.sub = Some(Sub::ReanimateFrom { queue: queue.clone() });
                            self.pending = Some(Decision {
                                player: controller,
                                kind: DecisionKind::Pick {
                                    prompt: "Choose a creature card from that graveyard to put onto the battlefield".into(),
                                    source: Some(source),
                                    options: cands.iter().map(|c| Opt::Obj { obj: Obj::card(*c) }).collect(),
                                    min: 1,
                                    max: 1,
                                    cancellable: false,
                                    hidden: false,
                                    purpose: PickPurpose::Generic,
                                },
                            });
                            return Exec::Wait;
                        }
                    }
                    queue.remove(0);
                }
                Exec::Done
            }
            Effect::Destroy { what, no_regeneration, remember, existing_at_trigger, died_only } => {
                let cs = {
                    let c = ec!();
                    let mut cs = self.sel_cards(what, &c);
                    if *existing_at_trigger && c.event.ts > 0 {
                        cs.retain(|id| self.cards[id.0 as usize].ts <= c.event.ts);
                    }
                    cs
                };
                self.begin_batch();
                let mut gone: Vec<Obj> = vec![];
                // simultaneous destruction: permanents with static abilities (lords granting keywords) go last so the others still see them
                let mut cs = cs;
                cs.sort_by_key(|id| { let d = self.db.def(self.cards[id.0 as usize].def); !(d.statics.is_empty() && !d.abilities.iter().any(|a| matches!(a, Ability::Replace { .. }))) });
                for id in cs {
                    self.destroy_opts(id, !*no_regeneration);
                    // destroyed even if a replacement effect (Rest in Peace) sent it elsewhere than the graveyard
                    if *remember && self.cards[id.0 as usize].zone != Zone::Battlefield && (!*died_only || self.cards[id.0 as usize].zone == Zone::Graveyard) {
                        gone.push(Obj::card(id));
                    }
                }
                self.end_batch();
                if *remember {
                    ctx.remembered = gone;
                }
                Exec::Done
            }
            Effect::CantRegenerate { what } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                for id in cs {
                    self.cards[id.0 as usize].no_regen_turn = self.turn;
                }
                Exec::Done
            }
            Effect::DamageLockThisTurn { what } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                for id in cs {
                    self.cards[id.0 as usize].dmg_lock_turn = self.turn;
                }
                Exec::Done
            }
            Effect::DestroyHealShield { what } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                for id in cs {
                    if self.cards[id.0 as usize].zone == Zone::Battlefield {
                        self.cards[id.0 as usize].heal_shield = true;
                    }
                }
                Exec::Done
            }
            Effect::Regenerate { what, steal } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                for id in cs {
                    if self.cards[id.0 as usize].zone == Zone::Battlefield {
                        self.cards[id.0 as usize].regen = self.cards[id.0 as usize].regen.saturating_add(1);
                        if *steal {
                            self.cards[id.0 as usize].regen_steal = Some((source, controller));
                        }
                    }
                }
                Exec::Done
            }
            Effect::Exile { what, return_at_end, return_at_owners_end, face_down, until_source_leaves, return_to, manual_return, return_counters, return_bonus } => {
                // "exile this spell" on a copy of a spell: the copy is not a card (it just ceases to exist), the original is untouched (Bygone Marvels)
                if ctx.item.copy && matches!(what, Sel::This) && matches!(ctx.item.kind, ItemKind::Spell { .. }) {
                    return Exec::Done;
                }
                // an activated or triggered ability that exiles its own source does nothing if that permanent already became a new object (killed in response; 400.7)
                if matches!(what, Sel::This) && matches!(ctx.item.kind, ItemKind::Ability { .. } | ItemKind::Triggered { .. }) && self.cards[source.0 as usize].zcc != ctx.item.src_zcc && ((self.cards[source.0 as usize].zone != Zone::Battlefield && !self.def_of(source).types.has(CardType::Planeswalker)) || (self.cards[source.0 as usize].zone == Zone::Battlefield && matches!(ctx.item.kind, ItemKind::Triggered { .. }))) {
                    return Exec::Done;
                }
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                let mut exiled = vec![];
                // the types each permanent had on the battlefield (an Opalescence enchantment is a creature): the "returns with a counter if it's a creature" bonus reads them
                let pre_types: Vec<(CardId, crate::types::TypeSet)> = cs.iter().filter(|id| self.cards[id.0 as usize].zone == Zone::Battlefield).map(|id| {
                    let mut t = self.cards[id.0 as usize].cur.types;
                    // a temporary animation (Mutavault until end of turn) is gone by the time it returns; a static one (Opalescence) is not
                    if !self.def_of(*id).types.has(CardType::Creature) && self.effects.iter().any(|e| e.affects.iter().any(|(i, _)| i == id)) {
                        t = self.def_of(*id).types;
                    }
                    (*id, t)
                }).collect();
                // several permanents exiled by one effect leave together: their leave abilities see one another go (Three Tree Scribe)
                let exile_batch = cs.len() > 1 && !self.batch_active;
                if exile_batch {
                    self.begin_batch();
                }
                for id in cs {
                    let owner_zone = self.cards[id.0 as usize].zone;
                    // a phased-out permanent is treated as though it doesn't exist: it can't be exiled (Zirilan's Dragon under Teferi's Protection)
                    if owner_zone == Zone::Battlefield && self.cards[id.0 as usize].phased_out {
                        continue;
                    }
                    if owner_zone == Zone::Stack {
                        // exiling a spell (its copies stay on the stack)
                        self.stack.retain(|s| s.source != id || s.copy);
                    }
                    // a card a cost already exiled (champion / behold): just link it
                    let already = owner_zone == Zone::Exile;
                    if already || self.move_card(id, Zone::Exile, MoveOpts { face_down: *face_down, ..Default::default() }) {
                        exiled.push(id);
                        if *until_source_leaves && *manual_return && self.cards[source.0 as usize].zone == Zone::Battlefield {
                            self.manual_linked.push((source, self.cards[source.0 as usize].zcc, id, self.cards[id.0 as usize].zcc));
                        } else if *until_source_leaves && self.cards[source.0 as usize].zone == Zone::Battlefield {
                            self.linked_exiles.push((source, self.cards[source.0 as usize].zcc, id, self.cards[id.0 as usize].zcc, return_to.unwrap_or(Zone::Battlefield)));
                            if *return_counters > 0 {
                                self.linked_counters.push((id, self.cards[id.0 as usize].zcc, *return_counters, controller));
                            }
                        }
                    }
                }
                if exile_batch {
                    self.end_batch();
                }
                if *return_at_owners_end {
                    for id in &exiled {
                        let did = self.next_effect_id;
                        self.next_effect_id += 1;
                        let mut dctx = EventCtx::default();
                        dctx.player = Some(self.cards[id.0 as usize].owner);
                        self.delayed.push(DelayedTrigger {
                            id: did,
                            controller,
                            source,
                            when: DelayedWhen::NextCtxPlayerEndStep,
                            effects: vec![Effect::Move {
                                what: Sel::Remembered,
                                to: Zone::Battlefield,
                                top_of_library: false,
                                tapped: false,
                                under_your_control: false,
                                remember: false,
                                transformed: false,
                                from: None,
                                discard: false,
                                position: None, position_x: false, ordered: false, bottom_ordered: false, order_chooser: None,
                                attacking: false, choose_host: false, host_creature: false, host_only: None, host_chooser: None, non_aura_if_no_host: false, face_down: false, under_control_of: None, random_order: false, enters_bare: false,
                            }],
                            created_turn: self.turn,
                            targets: vec![],
                            ctx: dctx,
                            remembered: vec![Obj::card(*id)],
                        });
                    }
                }
                if *return_at_end {
                    for id in &exiled {
                        if *return_bonus && self.cards[id.0 as usize].zone == Zone::Exile {
                            let ty = pre_types.iter().find(|(i, _)| i == id).map(|(_, t)| *t).unwrap_or_else(|| self.def_of(*id).types);
                            let c = &mut self.cards[id.0 as usize];
                            if ty.has(CardType::Creature) {
                                c.rider_counters = c.rider_counters.saturating_add(1);
                            }
                            if ty.has(CardType::Planeswalker) {
                                c.rider_loyalty = c.rider_loyalty.saturating_add(1);
                            }
                        }
                        let zcc = self.cards[id.0 as usize].zcc;
                        let did = self.next_effect_id;
                        self.next_effect_id += 1;
                        self.delayed.push(DelayedTrigger {
                            id: did,
                            controller,
                            source,
                            when: DelayedWhen::NextEndStep,
                            effects: vec![Effect::Move {
                                what: Sel::Remembered,
                                to: Zone::Battlefield,
                                top_of_library: false,
                                tapped: false,
                                under_your_control: false,
                                remember: false,
                                transformed: false,
                                from: None,
                                discard: false,
                                position: None, position_x: false, ordered: false, bottom_ordered: false, order_chooser: None,
                                attacking: false, choose_host: false, host_creature: false, host_only: None, host_chooser: None, non_aura_if_no_host: false, face_down: false, under_control_of: None, random_order: false, enters_bare: false,
                            }],
                            created_turn: self.turn,
                            targets: vec![],
                            ctx: EventCtx::default(),
                            remembered: vec![Obj::card(*id)],
                        });
                        let _ = zcc;
                    }
                }
                ctx.remembered = exiled.iter().map(|c| Obj::card(*c)).collect();
                Exec::Done
            }
            Effect::Move { what, to, top_of_library, tapped, under_your_control, under_control_of, random_order, remember, transformed, from, discard, position, position_x, ordered, bottom_ordered, order_chooser, attacking, choose_host, host_creature, host_only, host_chooser, non_aura_if_no_host, face_down, enters_bare } => {
                // an ability that moves its own source ("put this on top of its owner's library") does nothing if the source left its zone since the ability was put on the stack: it is a new object (Sensei's Divining Top)
                if matches!(what, Sel::This) && ctx.sub.is_none() && ctx.item.src_zcc != 0 && self.cards[source.0 as usize].zcc != ctx.item.src_zcc && !(*from == Some(Zone::Graveyard) && self.cards[source.0 as usize].zone == Zone::Graveyard && self.cards[source.0 as usize].zcc == ctx.item.src_zcc + 1) && matches!(ctx.item.kind, ItemKind::Ability { .. }) {
                    return Exec::Done;
                }
                let controller = under_control_of.as_ref().and_then(|s| { let c = self.evalctx(ctx); self.sel_players(s, &c).first().copied() }).unwrap_or(controller);
                // the player who chooses an Aura's host (Takklemaggot: the dead creature's controller)
                let chooser = host_chooser.as_ref().and_then(|s| { let c = self.evalctx(ctx); self.sel_players(s, &c).first().copied() }).unwrap_or(controller);
                let mut no_host_ids: Vec<CardId> = vec![];
                let under_your_control = &(*under_your_control || under_control_of.is_some());
                if let Some(Sub::AuraHost { mut queue, tapped: tp, mut put_remember, remember: rem }) = ctx.sub.clone() {
                    ctx.sub = None;
                    let ans = self.answer.take();
                    if !queue.is_empty() {
                        let (aura, hosts) = queue.remove(0);
                        let host = match ans {
                            Some(Choice::Pick { indices }) => indices.first().and_then(|i| hosts.get(*i)).copied().or(hosts.first().copied()),
                            _ => hosts.first().copied(),
                        };
                        if let Some(h) = host {
                            self.move_card(aura, Zone::Battlefield, MoveOpts { tapped: tp, attach_to: Some(h), controller: if *under_your_control { Some(controller) } else { None }, ..Default::default() });
                            put_remember.push(Obj::card(aura));
                        }
                    }
                    if let Some((aura, hosts)) = queue.first().cloned() {
                        ctx.sub = Some(Sub::AuraHost { queue, tapped: tp, put_remember, remember: rem });
                        self.pending = Some(Decision {
                            player: chooser,
                            kind: DecisionKind::Pick {
                                prompt: format!("{}: choose what {} enchants", self.def_of(source).name, self.def_of(aura).name),
                                source: Some(source),
                                options: hosts.iter().map(|c| Opt::Obj { obj: Obj::card(*c) }).collect(),
                                min: 1,
                                max: 1,
                                cancellable: false,
                                hidden: false,
                                purpose: PickPurpose::Generic,
                            },
                        });
                        return Exec::Wait;
                    }
                    if rem {
                        ctx.remembered = put_remember;
                    }
                    return Exec::Done;
                }
                if let Some(Sub::TopOrder { cards }) = ctx.sub.clone() {
                    ctx.sub = None;
                    let mut order: Vec<CardId> = match self.answer.take() {
                        Some(Choice::Pick { indices }) => indices.iter().filter_map(|i| cards.get(*i).copied()).collect(),
                        _ => vec![],
                    };
                    for c in &cards {
                        if !order.contains(c) {
                            order.push(*c);
                        }
                    }
                    if *top_of_library {
                        // the first card chosen ends up on top: put them back to front
                        for id in order.into_iter().rev() {
                            self.move_card(id, Zone::Library, MoveOpts::default());
                        }
                    } else {
                        // on the bottom: the first card chosen goes in first, so it ends up highest of the group
                        for id in order {
                            self.move_card(id, Zone::Library, MoveOpts { bottom: true, ..Default::default() });
                        }
                    }
                    ctx.remembered = cards.into_iter().map(Obj::card).collect();
                    return Exec::Done;
                }
                // a copy of a spell is not a card: "exile this spell" must not move the original card
                if ctx.item.copy && matches!(what, Sel::This) && matches!(ctx.item.kind, ItemKind::Spell { .. }) {
                    return Exec::Done;
                }
                let (cs, spells) = {
                    let c = ec!();
                    let spells: Vec<StackId> = self.sel_objs(what, &c).into_iter().filter_map(|o| if let Obj::Stack { id } = o { Some(id) } else { None }).collect();
                    let mut cs = self.sel_cards(what, &c);
                    if let Some(z) = from {
                        cs.retain(|id| self.cards[id.0 as usize].zone == *z);
                    }
                    if *random_order {
                        use rand::seq::SliceRandom;
                        cs.shuffle(&mut self.rng);
                    }
                    (cs, spells)
                };
                if ((*ordered && *top_of_library) || *bottom_ordered) && *to == Zone::Library && cs.len() >= 2 {
                    let orderer = order_chooser.as_ref().and_then(|s| { let c = ec!(); self.sel_players(s, &c).first().copied() }).unwrap_or(controller);
                    ctx.sub = Some(Sub::TopOrder { cards: cs.clone() });
                    self.pending = Some(Decision {
                        player: orderer,
                        kind: DecisionKind::Pick {
                            prompt: format!("Put these cards on {} of the library: choose the order, topmost first", if *top_of_library { "top" } else { "the bottom" }),
                            source: Some(source),
                            options: cs.iter().map(|c| Opt::Obj { obj: Obj::card(*c) }).collect(),
                            min: cs.len() as u8,
                            max: cs.len() as u8,
                            cancellable: false,
                            hidden: false,
                            purpose: PickPurpose::Generic,
                        },
                    });
                    return Exec::Wait;
                }
                let mut spell_cards: Vec<Obj> = vec![];
                for sid in spells {
                    if let Some(pos) = self.stack.iter().position(|s| s.id == sid) {
                        let item = self.stack.remove(pos);
                        if !item.copy && matches!(item.kind, ItemKind::Spell { .. }) {
                            spell_cards.push(Obj::card(item.source));
                            self.move_card(item.source, *to, MoveOpts { tapped: *tapped, bottom: *to == Zone::Library && !*top_of_library, controller: if *under_your_control { Some(controller) } else { None }, ..Default::default() });
                        }
                    }
                }
                let keep_rem = *remember;
                let mut moved: Vec<Obj> = spell_cards;
                moved.extend(cs.iter().map(|c| Obj::card(*c)));
                let mut aura_wait: Vec<(CardId, Vec<CardId>)> = vec![];
                let only_set: Option<Vec<CardId>> = if *choose_host { host_only.as_ref().map(|s| { let c = ec!(); self.sel_cards(s, &c) }) } else { None };
                // permanents entering together don't see each other's replacement effects (Living Death)
                self.entering_batch.clear();
                if *to == Zone::Battlefield && cs.len() > 1 {
                    self.entering_batch = cs.clone();
                }
                // permanents put onto the battlefield together enter simultaneously: each sees the others enter (Gladewalker Ritualist + Living Death)
                let defer_move = *to == Zone::Battlefield && cs.len() > 1 && self.defer_events.is_none();
                if defer_move {
                    self.defer_events = Some(vec![]);
                }
                for id in cs {
                    // "you may sacrifice <the permanent that granted this ability>": only its controller can (Trickster's Talisman after a control change)
                    if matches!(what, Sel::GrantSource) && *to == Zone::Graveyard && self.cards[id.0 as usize].zone == Zone::Battlefield && self.cards[id.0 as usize].cur.controller != controller {
                        moved.retain(|o| *o != Obj::card(id));
                        continue;
                    }
                    if self.cards[id.0 as usize].zone == Zone::Stack {
                        self.stack.retain(|s| s.source != id || s.copy);
                    }
                    let mut opts = MoveOpts {
                        tapped: *tapped,
                        bottom: *to == Zone::Library && !*top_of_library,
                        controller: if *under_your_control { Some(controller) } else { None },
                        face_down: *face_down && *to == Zone::Battlefield,
                        ..Default::default()
                    };
                    // an Aura put onto the battlefield without being cast: its controller chooses what it enchants (none legal: it stays where it is)
                    if let (Some(set), true) = (&only_set, *to == Zone::Battlefield && self.cards[id.0 as usize].zone != Zone::Battlefield && !*transformed) {
                        let is_equip = self.def_of(id).subtypes.contains(&Sym::new("Equipment"));
                        let hosts: Vec<CardId> = if is_equip {
                            set.iter().copied().filter(|h| self.cards[h.0 as usize].zone == Zone::Battlefield && *h != id).collect()
                        } else {
                            self.aura_hosts(id).unwrap_or_default().into_iter().filter(|h| set.contains(h)).collect()
                        };
                        let Some(&h) = hosts.first() else {
                            moved.retain(|o| *o != Obj::card(id));
                            continue;
                        };
                        // several legal hosts for an Aura: its controller chooses (Storm Herald)
                        if !is_equip && hosts.len() > 1 {
                            moved.retain(|o| *o != Obj::card(id));
                            aura_wait.push((id, hosts));
                            continue;
                        }
                        opts.attach_to = Some(h);
                    } else if *choose_host && *to == Zone::Battlefield && self.cards[id.0 as usize].zone != Zone::Battlefield && !*transformed {
                        if let Some(mut hosts) = self.aura_hosts(id) {
                            if *host_creature {
                                hosts.retain(|h| self.cards[h.0 as usize].cur.types.has(CardType::Creature));
                            }
                            // an Aura can't enchant a permanent that enters at the same time
                            hosts.retain(|h| !moved.contains(&Obj::card(*h)));
                            moved.retain(|o| *o != Obj::card(id));
                            match hosts.len() {
                                0 if *non_aura_if_no_host => {
                                    moved.push(Obj::card(id));
                                    no_host_ids.push(id);
                                }
                                0 => continue,
                                1 => {
                                    opts.attach_to = Some(hosts[0]);
                                    moved.push(Obj::card(id));
                                }
                                _ => {
                                    aura_wait.push((id, hosts));
                                    continue;
                                }
                            }
                        }
                    }
                    let was_in_hand = self.cards[id.0 as usize].zone == Zone::Hand;
                    let hand_owner = self.cards[id.0 as usize].owner;
                    // "returns transformed": it enters as its back face (the front's "when this enters" must not fire)
                    let mut entered_back = false;
                    if *transformed && *to == Zone::Battlefield && self.cards[id.0 as usize].zone != Zone::Battlefield {
                        if let Some(back) = self.def_of(id).other_face {
                            self.cards[id.0 as usize].def = back;
                            self.reset_derived(id);
                            entered_back = true;
                        }
                    }
                    self.entering_attacking.set(*attacking);
                    self.entering_bare.set(*enters_bare);
                    self.move_card(id, *to, opts);
                    self.entering_bare.set(false);
                    self.entering_attacking.set(false);
                    if no_host_ids.contains(&id) && self.cards[id.0 as usize].zone == Zone::Battlefield {
                        self.cards[id.0 as usize].non_aura = true;
                        self.layers_dirty = true;
                    }
                    if entered_back && self.cards[id.0 as usize].zone == Zone::Battlefield {
                        // a crafted permanent keeps the cards exiled as the craft cost ("exiled cards used to craft it": Locus of Enlightenment)
                        let nz = self.cards[id.0 as usize].zcc;
                        for x in self.cards.iter_mut() {
                            if x.zone == Zone::Exile && matches!(x.exiled_with, Some((s, zz)) if s == id && zz < nz) {
                                x.exiled_with = Some((id, nz));
                            }
                        }
                        self.emit(Event::EnteredTransformed { card: id });
                    }
                    let position_xv = if *position_x { Some(ctx.item.x.max(0).min(255) as u8) } else { *position };
                    if let (Some(pos), Zone::Library, true) = (position_xv.as_ref(), *to, *top_of_library) {
                        // "second from the top": move it down from the top by `pos` places
                        let owner = self.cards[id.0 as usize].owner;
                        let lib = &mut self.players[owner as usize].library;
                        if lib.last() == Some(&id) && *pos > 0 {
                            lib.pop();
                            let at = lib.len().saturating_sub(*pos as usize);
                            lib.insert(at, id);
                        }
                    }
                    if *discard && was_in_hand && *to == Zone::Graveyard && self.cards[id.0 as usize].zone == Zone::Graveyard {
                        self.emit(Event::Discarded { card: id, player: hand_owner, by: self.cause_ctl.get() });
                    }
                    if *transformed && *to == Zone::Battlefield && self.cards[id.0 as usize].zone == Zone::Battlefield {
                        if !entered_back {
                            self.transform(id);
                        }
                        // a Saga that enters transformed gets its first lore counter
                        if self.def_of(id).saga_max > 0 && self.cards[id.0 as usize].counter(CounterKind::Lore) == 0 {
                            self.cards[id.0 as usize].add_counters(CounterKind::Lore, 1);
                            self.emit(Event::Counters { card: id, kind: CounterKind::Lore, delta: 1, by: self.cause_ctl.get() });
                        }
                    }
                }
                self.entering_batch.clear();
                if defer_move {
                    self.flush_deferred_events();
                }
                if !aura_wait.is_empty() {
                    let (aura, hosts) = aura_wait[0].clone();
                    ctx.sub = Some(Sub::AuraHost { queue: aura_wait, tapped: *tapped, put_remember: if keep_rem { moved.clone() } else { vec![] }, remember: keep_rem });
                    self.pending = Some(Decision {
                        player: chooser,
                        kind: DecisionKind::Pick {
                            prompt: format!("{}: choose what {} enchants", self.def_of(source).name, self.def_of(aura).name),
                            source: Some(source),
                            options: hosts.iter().map(|c| Opt::Obj { obj: Obj::card(*c) }).collect(),
                            min: 1,
                            max: 1,
                            cancellable: false,
                            hidden: false,
                            purpose: PickPurpose::Generic,
                        },
                    });
                    return Exec::Wait;
                }
                if keep_rem {
                    ctx.remembered = moved;
                }
                Exec::Done
            }
            Effect::Bounce { what } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                // simultaneous departures: leaves-the-battlefield abilities of the bounced permanents see each other go (Evacuation + Dour Port-Mage)
                self.begin_batch();
                for id in cs {
                    if self.cards[id.0 as usize].zone == Zone::Battlefield {
                        self.move_card(id, Zone::Hand, Default::default());
                    }
                }
                self.end_batch();
                Exec::Done
            }
            Effect::SearchSameNames { of, tapped } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(of, &c)
                };
                if self.cant_search_libraries(controller) {
                    return Exec::Done;
                }
                self.players[controller as usize].searched_turn = true;
                self.emit(Event::Searched { player: controller });
                let mut found: Vec<CardId> = vec![];
                for id in cs {
                    let name = self.def_of(id).name.clone();
                    let hit = self.players[controller as usize].library.iter().rev().copied().find(|l| !found.contains(l) && self.def_of(*l).name == name);
                    found.extend(hit);
                }
                for id in found {
                    self.move_card(id, Zone::Battlefield, MoveOpts { tapped: *tapped, ..Default::default() });
                }
                self.shuffle_library(controller);
                Exec::Done
            }
            Effect::ReturnWithCounters { what, kind, n } => {
                let (cs, k) = {
                    let c = ec!();
                    (self.sel_cards(what, &c), self.eval_amount(n, &c))
                };
                for id in cs {
                    if matches!(self.cards[id.0 as usize].zone, Zone::Graveyard | Zone::Exile) {
                        self.move_card(id, Zone::Battlefield, Default::default());
                        if k > 0 && self.cards[id.0 as usize].zone == Zone::Battlefield {
                            self.add_counters_to(Obj::card(id), *kind, k);
                            self.layers_dirty = true;
                        }
                    }
                }
                Exec::Done
            }
            Effect::MarkUnmask { what } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                for id in cs {
                    self.cards[id.0 as usize].unmask = true;
                }
                Exec::Done
            }
            Effect::NoteManaSpentAll => {
                let spent = self.cards[source.0 as usize].act_spent;
                self.cards[source.0 as usize].noted_mana_all = spent;
                Exec::Done
            }
            Effect::NoteManaSpent => {
                let spent = self.cards[source.0 as usize].act_spent;
                self.cards[source.0 as usize].noted_mana = spent.iter().position(|&n| n > 0).map_or(0, |i| i as u8 + 1);
                Exec::Done
            }
            Effect::MarkEnlisted { what } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                for id in cs {
                    self.cards[id.0 as usize].enlisted_turn = self.turn;
                }
                Exec::Done
            }
            Effect::PayTally { who, amount, reset } => {
                let (ps, n) = {
                    let c = ec!();
                    (self.sel_players(who, &c), self.eval_amount(amount, &c))
                };
                if *reset || self.pay_tally.len() != self.players.len() {
                    self.pay_tally = vec![0; self.players.len()];
                }
                for p in ps {
                    self.pay_tally[p as usize] += n;
                }
                Exec::Done
            }
            Effect::BidLife { opening, with } => {
                let (players, mut idx, high, bidder, opened) = if let Some(Sub::Bid { players, idx, high, bidder, opened }) = ctx.sub.clone() {
                    ctx.sub = None;
                    let p = players[idx];
                    let n = if let Some(Choice::Number { n }) = self.answer.take() { n.clamp(0, if matches!(self.def_of(source).name.as_str(), "Illicit Auction" | "Pain's Reward") { 1_000_000 } else { self.players[p as usize].life.max(0) }) } else { 0 };
                    let (mut high, mut bidder) = (high, bidder);
                    if !opened || n > high {
                        high = n;
                        bidder = idx;
                    }
                    let next = (idx + 1) % players.len();
                    (players, next, high, bidder, true)
                } else {
                    self.life_rounds = vec![0; self.players.len()];
                    let mut v = vec![];
                    let mut q = controller;
                    let only: Option<Vec<PlayerId>> = with.as_ref().map(|w| { let c = self.evalctx(ctx); self.sel_players(w, &c) });
                    for _ in 0..self.alive_players().len() {
                        if q == controller || only.as_ref().map_or(true, |o| o.contains(&q)) {
                            v.push(q);
                        }
                        q = self.next_player(q);
                    }
                    match opening {
                        Some(o) => {
                            let len = v.len().max(1);
                            (v, 1 % len, (*o).max(0), 0, true)
                        }
                        None => (v, 0, 0, 0, false),
                    }
                };
                loop {
                    if opened && idx == bidder {
                        let w = players[bidder];
                        self.life_rounds[w as usize] = high;
                        ctx.remembered = vec![Obj::player(w)];
                        return Exec::Done;
                    }
                    let p = players[idx];
                    let life = self.players[p as usize].life;
                    // Illicit Auction: the bid is life LOSS, not a payment, so a player may bid more life than they have (official rulings for Illicit Auction and Pain's Reward; Exit Through the Grift Shop can't)
                    let loss_bid = matches!(self.def_of(source).name.as_str(), "Illicit Auction" | "Pain's Reward");
                    if opened && life <= high && !loss_bid {
                        idx = (idx + 1) % players.len();
                        continue;
                    }
                    ctx.sub = Some(Sub::Bid { players, idx, high, bidder, opened });
                    let prompt = if opened { format!("{}: top the high bid of {} life? (a number above {} tops it, anything else passes)", self.def_of(source).name, high, high) } else { format!("{}: open the bidding with how much life?", self.def_of(source).name) };
                    self.pending = Some(Decision { player: p, kind: DecisionKind::Number { prompt, min: 0, max: if loss_bid { life.max(0).max(high + 1000) } else { life.max(0) }, cancellable: false } });
                    return Exec::Wait;
                }
            }
            Effect::HideNumbers { min } => {
                let (players, idx) = if let Some(Sub::Hide { players, idx }) = ctx.sub.clone() {
                    ctx.sub = None;
                    let p = players[idx];
                    let n = if let Some(Choice::Number { n }) = self.answer.take() { n.clamp(*min, self.players[p as usize].life.max(*min).max(10)) } else { *min };
                    self.life_rounds[p as usize] = n;
                    (players, idx + 1)
                } else {
                    self.life_rounds = vec![0; self.players.len()];
                    let mut v = vec![];
                    let mut q = controller;
                    for _ in 0..self.alive_players().len() {
                        v.push(q);
                        q = self.next_player(q);
                    }
                    (v, 0)
                };
                if idx >= players.len() {
                    return Exec::Done;
                }
                let p = players[idx];
                // a player may hide more items than they have life (and lose into a negative total); the cap only keeps the choice finite
                let max = self.players[p as usize].life.max(*min).max(10);
                ctx.sub = Some(Sub::Hide { players, idx });
                self.pending = Some(Decision { player: p, kind: DecisionKind::Number { prompt: format!("{}: hide how many items (at least {})?", self.def_of(source).name, min), min: *min, max, cancellable: false } });
                Exec::Wait
            }
            Effect::RedistributeLife => {
                let name = self.def_of(source).name.clone();
                let (mut chosen, mut remaining, mut assigned);
                if let Some(Sub::Redist { chosen: ch, remaining: rem, assigned: asg, offer }) = ctx.sub.clone() {
                    ctx.sub = None;
                    let Some(Choice::Pick { indices }) = self.answer.take() else { return Exec::Done };
                    if ch.is_empty() {
                        // the players were just picked
                        let alive = self.alive_players();
                        let mut v: Vec<PlayerId> = indices.iter().filter_map(|i| alive.get(*i).copied()).collect();
                        v.sort_unstable();
                        v.dedup();
                        if v.len() < 2 {
                            return Exec::Done;
                        }
                        remaining = v.iter().map(|p| self.players[*p as usize].life).collect();
                        chosen = v;
                        assigned = vec![];
                    } else {
                        let Some(&t) = indices.first().and_then(|i| offer.get(*i)) else { return Exec::Done };
                        chosen = ch;
                        remaining = rem;
                        assigned = asg;
                        if let Some(pos) = remaining.iter().position(|x| *x == t) {
                            remaining.remove(pos);
                        }
                        assigned.push(t);
                    }
                } else {
                    let alive = self.alive_players();
                    if alive.len() < 2 {
                        return Exec::Done;
                    }
                    ctx.sub = Some(Sub::Redist { chosen: vec![], remaining: vec![], assigned: vec![], offer: vec![] });
                    self.pending = Some(Decision {
                        player: controller,
                        kind: DecisionKind::Pick {
                            prompt: format!("{name}: choose any number of players"),
                            source: Some(source),
                            options: alive.iter().map(|p| Opt::Obj { obj: Obj::player(*p) }).collect(),
                            min: 0,
                            max: alive.len() as u8,
                            cancellable: false,
                            hidden: false,
                            purpose: PickPurpose::Generic,
                        },
                    });
                    return Exec::Wait;
                }
                // give each chosen player but the last one of the remaining totals
                while assigned.len() + 1 < chosen.len() {
                    let mut offer = remaining.clone();
                    offer.sort_unstable_by(|a, b| b.cmp(a));
                    offer.dedup();
                    if offer.len() == 1 {
                        let t = offer[0];
                        remaining.remove(0);
                        assigned.push(t);
                        continue;
                    }
                    let p = chosen[assigned.len()];
                    let prompt = format!("{name}: choose the life total for player {p}");
                    let options = offer.iter().map(|t| Opt::Text { text: t.to_string() }).collect();
                    ctx.sub = Some(Sub::Redist { chosen, remaining, assigned, offer });
                    self.pending = Some(Decision { player: controller, kind: DecisionKind::Pick { prompt, source: Some(source), options, min: 1, max: 1, cancellable: false, hidden: false, purpose: PickPurpose::Generic } });
                    return Exec::Wait;
                }
                if let Some(&last) = remaining.first() {
                    assigned.push(last);
                }
                for (p, t) in chosen.iter().zip(assigned.iter()) {
                    let cur = self.players[*p as usize].life;
                    if *t > cur {
                        self.gain_life(*p, *t - cur);
                    } else if *t < cur {
                        self.lose_life(*p, cur - *t);
                    }
                }
                Exec::Done
            }
            Effect::JoinForces => {
                let (players, mut idx) = if let Some(Sub::JoinPay { players, idx }) = ctx.sub.clone() {
                    ctx.sub = None;
                    let p = players[idx];
                    if let Some(Choice::Number { n }) = self.answer.take() {
                        if n > 0 && self.try_pay_costs(p, source, &[Cost::Mana { cost: format!("{{{n}}}") }]) {
                            self.life_rounds[p as usize] += n;
                        }
                    }
                    (players, idx + 1)
                } else {
                    self.life_rounds = vec![0; self.players.len()];
                    let mut v = vec![];
                    let mut q = controller;
                    for _ in 0..self.alive_players().len() {
                        v.push(q);
                        q = self.next_player(q);
                    }
                    (v, 0)
                };
                loop {
                    if idx >= players.len() {
                        return Exec::Done;
                    }
                    let p = players[idx];
                    let mut max = 0;
                    for k in 1..=30 {
                        if self.can_pay_costs(p, source, &[Cost::Mana { cost: format!("{{{k}}}") }]) { max = k } else { break }
                    }
                    if max == 0 {
                        idx += 1;
                        continue;
                    }
                    ctx.sub = Some(Sub::JoinPay { players, idx });
                    self.pending = Some(Decision { player: p, kind: DecisionKind::Number { prompt: format!("{}: pay how much mana?", self.def_of(source).name), min: 0, max, cancellable: false } });
                    return Exec::Wait;
                }
            }
            Effect::PayLifeRounds => {
                let (players, mut idx, mut any) = if let Some(Sub::PayLife { players, idx, any }) = ctx.sub.clone() {
                    ctx.sub = None;
                    let p = players[idx];
                    let mut any = any;
                    if let Some(Choice::Number { n }) = self.answer.take() {
                        let n = n.clamp(0, self.players[p as usize].life.max(0));
                        if n > 0 && self.can_pay_life(p, n) {
                            self.pay_life(p, n);
                            self.life_rounds[p as usize] += n;
                            any = true;
                        }
                    }
                    (players, idx + 1, any)
                } else {
                    self.life_rounds = vec![0; self.players.len()];
                    let mut v = vec![];
                    let mut q = controller;
                    for _ in 0..self.alive_players().len() {
                        v.push(q);
                        q = self.next_player(q);
                    }
                    (v, 0, false)
                };
                loop {
                    if idx >= players.len() {
                        if !any {
                            return Exec::Done;
                        }
                        idx = 0;
                        any = false;
                    }
                    let p = players[idx];
                    if self.players[p as usize].life <= 0 || !self.can_pay_life(p, 1) {
                        idx += 1;
                        continue;
                    }
                    ctx.sub = Some(Sub::PayLife { players, idx, any });
                    self.pending = Some(Decision {
                        player: p,
                        kind: DecisionKind::Number { prompt: format!("{}: pay how much life?", self.def_of(source).name), min: 0, max: self.players[p as usize].life, cancellable: false },
                    });
                    return Exec::Wait;
                }
            }
            Effect::CantRegenerateDamagedBy { by } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(by, &c)
                };
                let turn = self.turn;
                self.no_regen_dealers.retain(|(_, t)| *t == turn);
                for id in cs {
                    self.no_regen_dealers.push((id, turn));
                }
                Exec::Done
            }
            Effect::CounterAllAbilities => {
                let ids: Vec<StackId> = self.stack.iter().filter(|s| !matches!(s.kind, ItemKind::Spell { .. })).map(|s| s.id).collect();
                for sid in ids {
                    self.counter_stack_item(sid, false, controller, source);
                }
                Exec::Done
            }
            Effect::CounterOpponentAbilities => {
                let ids: Vec<StackId> = self.stack.iter().filter(|s| !matches!(s.kind, ItemKind::Spell { .. }) && s.controller != controller).map(|s| s.id).collect();
                for sid in ids {
                    self.counter_stack_item(sid, false, controller, source);
                }
                Exec::Done
            }
            Effect::CounterOpposingAll => {
                let ids: Vec<(StackId, CardId)> = self.stack.iter().filter(|s| s.controller != controller).map(|s| (s.id, s.source)).collect();
                let mut done: Vec<Obj> = vec![];
                for (sid, src) in ids {
                    self.counter_stack_item(sid, false, controller, source);
                    if !self.stack.iter().any(|s| s.id == sid) {
                        done.push(Obj::card(src));
                    }
                }
                ctx.remembered = done;
                Exec::Done
            }
            Effect::CounterSpell { what, unless_pays, exile, remember, remember_targeted, include_abilities } => {
                let x = self.cards[source.0 as usize].x;
                let amt_ctx = ec!();
                let paying: Option<Vec<Cost>> = unless_pays.as_ref().map(|costs| {
                    costs
                        .iter()
                        .map(|c| match c {
                            Cost::Mana { cost } if cost.contains("{X}") => Cost::Mana { cost: cost.replace("{X}", &format!("{{{x}}}")) },
                            Cost::ManaAmount { amount } => Cost::Mana { cost: format!("{{{}}}", self.eval_amount(amount, &amt_ctx).max(0)) },
                            o => o.clone(),
                        })
                        .collect()
                });
                if let Some(Sub::FreeCast { player, card }) = ctx.sub.clone() {
                    let ans = self.answer.take();
                    ctx.sub = None;
                    if let Some(Choice::YesNo { yes: true }) = ans {
                        self.free_casts.push((player, card, false, None));
                    }
                    return Exec::Done;
                }
                // the countered spell's controller was asked whether to pay
                if let Some(Sub::CounterPay { sid }) = ctx.sub.clone() {
                    let ans = self.answer.take();
                    ctx.sub = None;
                    if let Some(item) = self.stack.iter().find(|s| s.id == sid).cloned() {
                        let yes = matches!(ans, Some(Choice::YesNo { yes: true }));
                        if yes && paying.as_ref().map_or(false, |cs| self.try_pay_costs(item.controller, item.source, cs)) {
                            return Exec::Done;
                        }
                        self.counter_stack_item(sid, *exile, controller, source);
                        if *remember && !item.copy {
                            ctx.remembered = vec![Obj::card(item.source)];
                        }
                        if self.counter_offer_ask(ctx, source) {
                            return Exec::Wait;
                        }
                    }
                    return Exec::Done;
                }
                let ids: Vec<StackId> = {
                    let c = ec!();
                    self.sel_objs(what, &c)
                        .into_iter()
                        .filter_map(|o| match o {
                            Obj::Stack { id } => Some(id),
                            // a spell named by its card (TriggerObject of a cast trigger)
                            Obj::Card { id } if self.cards[id.0 as usize].zone == Zone::Stack => self.stack.iter().rev().find(|s| s.source == id && !s.copy && matches!(s.kind, ItemKind::Spell { .. })).map(|s| s.id),
                            _ => None,
                        })
                        .collect()
                };
                let mut ids = ids;
                if *include_abilities {
                    // every ability on the stack that an opponent of the controller controls (activated and triggered)
                    let opps = self.opponents(controller);
                    ids.extend(self.stack.iter().filter(|s| !matches!(s.kind, ItemKind::Spell { .. } | ItemKind::Delayed { .. }) && opps.contains(&s.controller)).map(|s| s.id));
                }
                for sid in ids {
                    let Some(pos) = self.stack.iter().position(|s| s.id == sid) else { continue };
                    let item = self.stack[pos].clone();
                    if let Some(costs) = &paying {
                        if self.can_pay_costs(item.controller, item.source, costs) {
                            // a spell that can't be countered never asks
                            let immune = (self.cards[item.source.0 as usize].cur.kw.has(Keyword::CantBeCountered) && self.cards[item.source.0 as usize].cast_alt != Some(crate::decision::FACE_DOWN_ALT)) || (self.def_of(item.source).keywords.has(Keyword::CantBeCountered) && self.cards[item.source.0 as usize].cast_alt != Some(crate::decision::FACE_DOWN_ALT)) || self.spell_cant_be_countered(item.source, Some(source));
                            if !immune {
                                ctx.sub = Some(Sub::CounterPay { sid });
                                self.pending = Some(Decision {
                                    player: item.controller,
                                    kind: DecisionKind::YesNo { prompt: format!("Pay for {} to keep {} from being countered?", self.def_of(source).name, self.def_of(item.source).name), source: Some(source) },
                                });
                                return Exec::Wait;
                            }
                        }
                    }
                    self.counter_stack_item(sid, *exile, controller, source);
                    // only a spell that was really countered is remembered ("if a spell is countered this way ...")
                    if (*remember && !item.copy && !self.stack.iter().any(|s| s.id == sid)) || (*remember_targeted && !item.copy) {
                        ctx.remembered.push(Obj::card(item.source));
                    }
                    if self.counter_offer_ask(ctx, source) {
                        return Exec::Wait;
                    }
                }
                Exec::Done
            }
            Effect::Pump { what, power, toughness, duration } => {
                let (cs, p, t) = {
                    let c = ec!();
                    (self.sel_cards(what, &c), self.eval_amount(power, &c), self.eval_amount(toughness, &c))
                };
                self.add_effect(source, controller, &cs, Modif::Pump { power: p, toughness: t }, *duration);
                Exec::Done
            }
            Effect::SetBasePT { what, power, toughness, duration } => {
                let (cs, p, t) = {
                    let c = ec!();
                    (self.sel_cards(what, &c), self.eval_amount(power, &c), self.eval_amount(toughness, &c))
                };
                self.add_effect(source, controller, &cs, Modif::SetPT { power: p, toughness: t }, *duration);
                Exec::Done
            }
            Effect::SetBasePower { what, power, duration } => {
                let (cs, p) = {
                    let c = ec!();
                    (self.sel_cards(what, &c), self.eval_amount(power, &c))
                };
                self.add_effect(source, controller, &cs, Modif::SetP { power: p }, *duration);
                Exec::Done
            }
            Effect::SetBaseToughness { what, toughness, duration } => {
                let (cs, t) = {
                    let c = ec!();
                    (self.sel_cards(what, &c), self.eval_amount(toughness, &c))
                };
                self.add_effect(source, controller, &cs, Modif::SetT { toughness: t }, *duration);
                Exec::Done
            }
            Effect::GrantKeyword { what, keyword, duration } => {
                let cs = {
                    let c = ec!();
                    let mut cs = self.sel_cards(what, &c);
                    // a spell on the stack gets the keyword on its card ("target spell can't be countered this turn")
                    for o in self.sel_objs(what, &c) {
                        if let Obj::Stack { id } = o {
                            if let Some(si) = self.stack.iter().find(|s| s.id == id && matches!(s.kind, ItemKind::Spell { .. })) {
                                if !cs.contains(&si.source) {
                                    cs.push(si.source);
                                }
                            }
                        }
                    }
                    cs
                };
                self.add_effect(source, controller, &cs, Modif::Keyword(*keyword), *duration);
                Exec::Done
            }
            Effect::RemoveKeyword { what, keyword, duration } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                self.add_effect(source, controller, &cs, Modif::RemoveKeyword(*keyword), *duration);
                Exec::Done
            }
            Effect::RemoveProtection { what, from, duration } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                self.add_effect(source, controller, &cs, Modif::RemoveProtection(from.clone()), *duration);
                Exec::Done
            }
            Effect::AddCounters { what, kind, n } => {
                let (os, k) = {
                    let c = ec!();
                    (self.sel_objs(what, &c), self.eval_amount(n, &c))
                };
                for o in os {
                    // a card in a graveyard is no permanent: counters can't be put on it (Charging Cinderhorn leaving in response to its own trigger)
                    if matches!(o, Obj::Card { id } if self.cards[id.0 as usize].zone == Zone::Graveyard) {
                        continue;
                    }
                    self.add_counters_to(o, *kind, k);
                }
                Exec::Done
            }
            Effect::ModularCounters { what, n } => {
                let (os, k) = {
                    let c = ec!();
                    (self.sel_objs(what, &c), self.eval_amount(n, &c))
                };
                self.modular_cause.set(true);
                for o in os {
                    self.add_counters_to(o, CounterKind::PlusOnePlusOne, k);
                }
                self.modular_cause.set(false);
                Exec::Done
            }
            Effect::BumpChosenCounterKind { what } => {
                let mut queue: Vec<CardId>;
                if let Some(Sub::KindPick { ids, left, kinds, .. }) = ctx.sub.clone() {
                    ctx.sub = None;
                    queue = ids.clone();
                    if left == 0 {
                        // the kind was picked: may another counter of it be added?
                        let idx = match self.answer.take() {
                            Some(Choice::Pick { indices }) => indices.first().copied(),
                            _ => None,
                        };
                        let k = idx.and_then(|i| kinds.get(i)).copied().or_else(|| kinds.first().copied());
                        if let (Some(k), Some(_)) = (k, ids.first()) {
                            ctx.sub = Some(Sub::KindPick { ids, left: 1, n_each: 1, kinds: vec![k], dst: None, up_to: false });
                            self.pending = Some(Decision {
                                player: controller,
                                kind: DecisionKind::YesNo { prompt: format!("Put an additional {k:?} counter on it?"), source: Some(source) },
                            });
                            return Exec::Wait;
                        }
                    } else {
                        let yes = matches!(self.answer.take(), Some(Choice::YesNo { yes: true }));
                        if let (true, Some(&k), Some(&id)) = (yes, kinds.first(), ids.first()) {
                            if self.cards[id.0 as usize].zone == Zone::Battlefield {
                                self.add_counters_to(Obj::card(id), k, 1);
                            }
                        }
                    }
                    if !queue.is_empty() {
                        queue.remove(0);
                    }
                } else {
                    let c = ec!();
                    queue = self.sel_cards(what, &c);
                }
                while let Some(&id) = queue.first() {
                    let kinds: Vec<CounterKind> = if self.cards[id.0 as usize].zone == Zone::Battlefield { self.cards[id.0 as usize].counters.iter().filter(|(_, x)| *x > 0).map(|(k, _)| *k).collect() } else { vec![] };
                    match kinds.len() {
                        0 => {
                            queue.remove(0);
                        }
                        1 => {
                            ctx.sub = Some(Sub::KindPick { ids: queue.clone(), left: 1, n_each: 1, kinds: kinds.clone(), dst: None, up_to: false });
                            self.pending = Some(Decision {
                                player: controller,
                                kind: DecisionKind::YesNo { prompt: format!("Put an additional {:?} counter on {}?", kinds[0], self.def_of(id).name), source: Some(source) },
                            });
                            return Exec::Wait;
                        }
                        _ => {
                            ctx.sub = Some(Sub::KindPick { ids: queue.clone(), left: 0, n_each: 1, kinds: kinds.clone(), dst: None, up_to: false });
                            self.pending = Some(Decision {
                                player: controller,
                                kind: DecisionKind::Pick {
                                    prompt: format!("Choose a kind of counter on {}", self.def_of(id).name),
                                    source: Some(source),
                                    options: kinds.iter().map(|k| Opt::Text { text: k.label() }).collect(),
                                    min: 1,
                                    max: 1,
                                    cancellable: false,
                                    hidden: false,
                                    purpose: PickPurpose::Mode,
                                },
                            });
                            return Exec::Wait;
                        }
                    }
                }
                Exec::Done
            }
            Effect::AddCountersOfEventKind { what, n } => {
                let (os, k, kind) = {
                    let c = ec!();
                    (self.sel_objs(what, &c), self.eval_amount(n, &c), c.event.counter_kind)
                };
                if let Some(kind) = kind {
                    for o in os {
                        self.add_counters_to(o, kind, k);
                    }
                }
                Exec::Done
            }
            Effect::SetExiledBy { what, who } => {
                let (cs, p) = {
                    let c = ec!();
                    (self.sel_cards(what, &c), self.sel_players(who, &c).first().copied())
                };
                for id in cs {
                    if self.cards[id.0 as usize].zone == Zone::Exile {
                        self.cards[id.0 as usize].exiled_by = p;
                    }
                }
                Exec::Done
            }
            Effect::DiscardSplicedNames { of } => {
                let objs = {
                    let c = ec!();
                    self.sel_objs(of, &c)
                };
                for o in objs {
                    let Obj::Stack { id } = o else { continue };
                    let Some(item) = self.stack.iter().find(|s| s.id == id).cloned() else { continue };
                    let names: Vec<String> = item.spliced.iter().map(|d| self.db.def(*d).name.clone()).collect();
                    let hand: Vec<CardId> = self.players[item.controller as usize].hand.iter().copied().filter(|h| names.contains(&self.def_of(*h).name)).collect();
                    for h in hand {
                        let p = self.cards[h.0 as usize].owner;
                        self.move_card(h, Zone::Graveyard, Default::default());
                        self.emit(Event::Discarded { card: h, player: p, by: self.cause_ctl.get() });
                    }
                }
                Exec::Done
            }
            Effect::AddCountersOfEventKinds { what } => {
                let (os, kinds) = {
                    let c = ec!();
                    (self.sel_objs(what, &c), c.event.counter_kinds.clone())
                };
                for o in os {
                    for k in &kinds {
                        self.add_counters_to(o, *k, 1);
                    }
                }
                Exec::Done
            }
            Effect::ChooseCounterAdjust { what } => {
                let mut picked: Option<(CardId, CounterKind)> = None;
                if let Some(Sub::KindPick { ids, kinds, left, .. }) = ctx.sub.clone() {
                    ctx.sub = None;
                    let Some(&id) = ids.first() else { return Exec::Done };
                    if left == 2 {
                        let add = matches!(self.answer.take(), Some(Choice::YesNo { yes: true }));
                        if let Some(&k) = kinds.first() {
                            if matches!(self.cards[id.0 as usize].zone, Zone::Battlefield | Zone::Exile) {
                                if add {
                                    self.add_counters_to(Obj::card(id), k, 1);
                                } else {
                                    self.remove_counters_from(Obj::card(id), k, 1);
                                }
                            }
                        }
                        return Exec::Done;
                    }
                    let idx = match self.answer.take() {
                        Some(Choice::Pick { indices }) => indices.first().copied(),
                        _ => None,
                    };
                    picked = idx.and_then(|i| kinds.get(i)).copied().or_else(|| kinds.first().copied()).map(|k| (id, k));
                } else {
                    let id = {
                        let c = ec!();
                        self.sel_cards(what, &c).first().copied()
                    };
                    let Some(id) = id else { return Exec::Done };
                    if !matches!(self.cards[id.0 as usize].zone, Zone::Battlefield | Zone::Exile) {
                        return Exec::Done;
                    }
                    let kinds: Vec<CounterKind> = self.cards[id.0 as usize].counters.iter().filter(|(_, x)| *x > 0).map(|(k, _)| *k).collect();
                    match kinds.len() {
                        0 => return Exec::Done,
                        1 => picked = Some((id, kinds[0])),
                        _ => {
                            ctx.sub = Some(Sub::KindPick { ids: vec![id], left: 1, n_each: 1, kinds: kinds.clone(), dst: None, up_to: false });
                            self.pending = Some(Decision {
                                player: controller,
                                kind: DecisionKind::Pick {
                                    prompt: "Choose a kind of counter".into(),
                                    source: Some(source),
                                    options: kinds.iter().map(|k| Opt::Text { text: k.label() }).collect(),
                                    min: 1,
                                    max: 1,
                                    cancellable: false,
                                    hidden: false,
                                    purpose: PickPurpose::Mode,
                                },
                            });
                            return Exec::Wait;
                        }
                    }
                }
                if let Some((id, k)) = picked {
                    ctx.sub = Some(Sub::KindPick { ids: vec![id], left: 2, n_each: 1, kinds: vec![k], dst: None, up_to: false });
                    self.pending = Some(Decision {
                        player: controller,
                        kind: DecisionKind::YesNo { prompt: format!("{k:?} counter: put another one on it? (no: remove one)"), source: Some(source) },
                    });
                    return Exec::Wait;
                }
                Exec::Done
            }
            Effect::AdjustEachCounterKind { what } => {
                let mut state: Option<(CardId, Vec<CounterKind>)> = None;
                if let Some(Sub::KindPick { ids, kinds, .. }) = ctx.sub.clone() {
                    ctx.sub = None;
                    let add = matches!(self.answer.take(), Some(Choice::YesNo { yes: true }));
                    if let (Some(&k), Some(&id)) = (kinds.first(), ids.first()) {
                        if self.cards[id.0 as usize].zone == Zone::Battlefield {
                            if add {
                                self.add_counters_to(Obj::card(id), k, 1);
                            } else {
                                self.remove_counters_from(Obj::card(id), k, 1);
                            }
                        }
                        state = Some((id, kinds[1..].to_vec()));
                    }
                } else {
                    let id = {
                        let c = ec!();
                        self.sel_cards(what, &c).first().copied()
                    };
                    if let Some(id) = id {
                        if self.cards[id.0 as usize].zone == Zone::Battlefield {
                            let kinds: Vec<CounterKind> = self.cards[id.0 as usize].counters.iter().filter(|(_, x)| *x > 0).map(|(k, _)| *k).collect();
                            state = Some((id, kinds));
                        }
                    }
                }
                if let Some((id, kinds)) = state {
                    if let Some(k) = kinds.first() {
                        ctx.sub = Some(Sub::KindPick { ids: vec![id], left: 1, n_each: 1, kinds: kinds.clone(), dst: None, up_to: false });
                        self.pending = Some(Decision {
                            player: controller,
                            kind: DecisionKind::YesNo { prompt: format!("{k:?} counter: put another one on it? (no: remove one)"), source: Some(source) },
                        });
                        return Exec::Wait;
                    }
                }
                Exec::Done
            }
            Effect::UnlinkAll => {
                let c = &mut self.cards[source.0 as usize];
                c.linked = None;
                c.linked_extra.clear();
                Exec::Done
            }
            Effect::LinkRemembered => {
                let first = ctx.remembered.iter().find_map(|o| if let Obj::Card { id } = o { Some(*id) } else { None });
                if let Some(id) = first.filter(|id| self.cards[id.0 as usize].zone == Zone::Battlefield) {
                    let l = (id, self.cards[id.0 as usize].zcc);
                    if self.cards[source.0 as usize].linked.is_none() {
                        self.cards[source.0 as usize].linked = Some(l);
                    } else {
                        self.cards[source.0 as usize].linked_extra.push(l);
                    }
                }
                Exec::Done
            }
            Effect::ChooseCounterKindAdd { among, to } => {
                let kind: Option<CounterKind>;
                if let Some(Sub::KindPick { kinds, .. }) = ctx.sub.clone() {
                    ctx.sub = None;
                    let idx = match self.answer.take() {
                        Some(Choice::Pick { indices }) => indices.first().copied(),
                        _ => None,
                    };
                    kind = idx.and_then(|i| kinds.get(i)).copied().or_else(|| kinds.first().copied());
                } else {
                    let mut kinds: Vec<CounterKind> = vec![];
                    {
                        let c = ec!();
                        for id in self.sel_cards(among, &c) {
                            if self.cards[id.0 as usize].zone == Zone::Battlefield {
                                for (k, n) in self.cards[id.0 as usize].counters.iter() {
                                    if *n > 0 && !kinds.contains(k) {
                                        kinds.push(*k);
                                    }
                                }
                            }
                        }
                    }
                    match kinds.len() {
                        0 => return Exec::Done,
                        1 => kind = Some(kinds[0]),
                        _ => {
                            ctx.sub = Some(Sub::KindPick { ids: vec![], left: 1, n_each: 1, kinds: kinds.clone(), dst: None, up_to: false });
                            self.pending = Some(Decision {
                                player: controller,
                                kind: DecisionKind::Pick {
                                    prompt: "Choose a kind of counter".into(),
                                    source: Some(source),
                                    options: kinds.iter().map(|k| Opt::Text { text: k.label() }).collect(),
                                    min: 1,
                                    max: 1,
                                    cancellable: false,
                                    hidden: false,
                                    purpose: PickPurpose::Mode,
                                },
                            });
                            return Exec::Wait;
                        }
                    }
                }
                if let Some(k) = kind {
                    let dsts: Vec<CardId> = {
                        let c = ec!();
                        self.sel_cards(to, &c)
                    };
                    for d in dsts {
                        if self.cards[d.0 as usize].zone == Zone::Battlefield && !self.cards[d.0 as usize].counters.iter().any(|(kk, n)| *kk == k && *n > 0) {
                            self.add_counters_to(Obj::card(d), k, 1);
                        }
                    }
                }
                Exec::Done
            }
            Effect::SpreadCounterKind { among } => {
                let chosen: Option<(CardId, Option<CounterKind>)>;
                if let Some(Sub::KindPick { ids, kinds, .. }) = ctx.sub.clone() {
                    ctx.sub = None;
                    let idx = match self.answer.take() {
                        Some(Choice::Pick { indices }) => indices.first().copied(),
                        _ => None,
                    };
                    if kinds.is_empty() {
                        chosen = idx.and_then(|i| ids.get(i)).or(ids.first()).map(|id| (*id, None));
                    } else {
                        chosen = ids.first().map(|id| (*id, idx.and_then(|i| kinds.get(i)).copied().or_else(|| kinds.first().copied())));
                    }
                } else {
                    let cands: Vec<CardId> = {
                        let c = ec!();
                        self.sel_cards(among, &c).into_iter().filter(|id| self.cards[id.0 as usize].zone == Zone::Battlefield && self.cards[id.0 as usize].counters.iter().any(|(_, n)| *n > 0)).collect()
                    };
                    match cands.len() {
                        0 => return Exec::Done,
                        1 => chosen = Some((cands[0], None)),
                        _ => {
                            self.pending = Some(Decision {
                                player: controller,
                                kind: DecisionKind::Pick {
                                    prompt: format!("{}: choose a creature with the kind of counter", self.def_of(source).name),
                                    source: Some(source),
                                    options: cands.iter().map(|c| Opt::Obj { obj: Obj::card(*c) }).collect(),
                                    min: 1,
                                    max: 1,
                                    cancellable: false,
                                    hidden: false,
                                    purpose: PickPurpose::Generic,
                                },
                            });
                            ctx.sub = Some(Sub::KindPick { ids: cands, left: 0, n_each: 0, kinds: vec![], dst: None, up_to: false });
                            return Exec::Wait;
                        }
                    }
                }
                let Some((id, mut kind)) = chosen else { return Exec::Done };
                if kind.is_none() {
                    let kinds: Vec<CounterKind> = self.cards[id.0 as usize].counters.iter().filter(|(_, x)| *x > 0).map(|(k, _)| *k).collect();
                    match kinds.len() {
                        0 => return Exec::Done,
                        1 => kind = Some(kinds[0]),
                        _ => {
                            ctx.sub = Some(Sub::KindPick { ids: vec![id], left: 1, n_each: 1, kinds: kinds.clone(), dst: None, up_to: false });
                            self.pending = Some(Decision {
                                player: controller,
                                kind: DecisionKind::Pick {
                                    prompt: "Choose a kind of counter".into(),
                                    source: Some(source),
                                    options: kinds.iter().map(|k| Opt::Text { text: k.label() }).collect(),
                                    min: 1,
                                    max: 1,
                                    cancellable: false,
                                    hidden: false,
                                    purpose: PickPurpose::Mode,
                                },
                            });
                            return Exec::Wait;
                        }
                    }
                }
                if let Some(k) = kind {
                    let others: Vec<CardId> = {
                        let c = ec!();
                        self.sel_cards(among, &c)
                    };
                    for o in others {
                        if o != id && self.cards[o.0 as usize].zone == Zone::Battlefield {
                            self.add_counters_to(Obj::card(o), k, 1);
                        }
                    }
                }
                Exec::Done
            }
            Effect::AddCounterOfChosenKind { what } => {
                if let Some(Sub::KindPick { ids, kinds, .. }) = ctx.sub.clone() {
                    ctx.sub = None;
                    let idx = match self.answer.take() {
                        Some(Choice::Pick { indices }) => indices.first().copied(),
                        _ => None,
                    };
                    let pick = idx.and_then(|i| kinds.get(i)).copied().or_else(|| kinds.first().copied());
                    if let (Some(k), Some(&id)) = (pick, ids.first()) {
                        self.add_counters_to(Obj::card(id), k, 1);
                    }
                    return Exec::Done;
                }
                let id = {
                    let c = ec!();
                    self.sel_cards(what, &c).first().copied()
                };
                let Some(id) = id else { return Exec::Done };
                if self.cards[id.0 as usize].zone != Zone::Battlefield {
                    return Exec::Done;
                }
                let kinds: Vec<CounterKind> = self.cards[id.0 as usize].counters.iter().filter(|(_, x)| *x > 0).map(|(k, _)| *k).collect();
                match kinds.len() {
                    0 => {}
                    1 => self.add_counters_to(Obj::card(id), kinds[0], 1),
                    _ => {
                        ctx.sub = Some(Sub::KindPick { ids: vec![id], left: 1, n_each: 1, kinds: kinds.clone(), dst: None, up_to: false });
                        self.pending = Some(Decision {
                            player: controller,
                            kind: DecisionKind::Pick {
                                prompt: "Choose a kind of counter".into(),
                                source: Some(source),
                                options: kinds.iter().map(|k| Opt::Text { text: k.label() }).collect(),
                                min: 1,
                                max: 1,
                                cancellable: false,
                                hidden: false,
                                purpose: PickPurpose::Mode,
                            },
                        });
                        return Exec::Wait;
                    }
                }
                Exec::Done
            }
            Effect::RemoveCounters { what, kind, n } => {
                let (os, k) = {
                    let c = ec!();
                    (self.sel_objs(what, &c), self.eval_amount(n, &c))
                };
                for o in os {
                    self.remove_counters_from(o, *kind, k);
                }
                Exec::Done
            }
            Effect::MustAttackPlayer { what, player, duration } => {
                let (cs, ps) = {
                    let c = ec!();
                    (self.sel_cards(what, &c), self.sel_players(player, &c))
                };
                if let Some(&pl) = ps.first() {
                    self.add_effect(source, controller, &cs, Modif::MustAttackPlayer(pl), *duration);
                }
                Exec::Done
            }
            Effect::AttackRandomOpponentNotLast => {
                let serial = self.players[controller as usize].combat_serial;
                let last = if self.cards[source.0 as usize].attacked_combat + 1 == serial { self.cards[source.0 as usize].attacked_player } else { None };
                let opps: Vec<PlayerId> = self.opponents(controller).into_iter().filter(|p| Some(*p) != last).collect();
                if opps.is_empty() {
                    self.tap_card(source);
                } else {
                    let pick = {
                        let c = ec!();
                        opps[if opps.len() > 1 { self.rand_index(&c, opps.len()) } else { 0 }]
                    };
                    self.add_effect(source, controller, &[source], Modif::MustAttackPlayer(pick), Duration::UntilEndOfCombat);
                }
                Exec::Done
            }
            Effect::TauntAllCreatures { who, player } => {
                let (vs, ps) = {
                    let c = ec!();
                    (self.sel_players(who, &c), self.sel_players(player, &c))
                };
                if let Some(&pl) = ps.first() {
                    for v in vs {
                        self.taunts.push((v, pl, self.turn));
                    }
                }
                Exec::Done
            }
            Effect::IgnoreSourceEffect => {
                self.effect_ignored.push((source, controller, self.turn));
                Exec::Done
            }
            Effect::AttackAllThisTurn { who } => {
                let vs = {
                    let c = ec!();
                    self.sel_players(who, &c)
                };
                for v in vs {
                    self.attack_all.push((v, self.turn));
                }
                Exec::Done
            }
            Effect::MustAttackPermanent { what, permanent, duration } => {
                let (cs, ps) = {
                    let c = ec!();
                    (self.sel_cards(what, &c), self.sel_cards(permanent, &c))
                };
                if let Some(&pw) = ps.first() {
                    self.add_effect(source, controller, &cs, Modif::MustAttackPermanent(pw), *duration);
                }
                Exec::Done
            }
            Effect::DoubleCounters { what, kind } => {
                let os = {
                    let c = ec!();
                    self.sel_objs(what, &c)
                };
                for o in os {
                    if let Obj::Card { id } = o {
                        let have: Vec<(CounterKind, u16)> = self.cards[id.0 as usize].counters.iter().copied().collect();
                        for (k, n) in have {
                            if kind.map_or(true, |w| w == k) && n > 0 {
                                self.add_counters_to(o, k, n as i32);
                            }
                        }
                    }
                }
                Exec::Done
            }
            Effect::AddOneOfEachCounter { what } => {
                let os = {
                    let c = ec!();
                    self.sel_objs(what, &c)
                };
                for o in os {
                    match o {
                        Obj::Card { id } => {
                            let kinds: Vec<CounterKind> = self.cards[id.0 as usize].counters.iter().filter(|(_, n)| *n > 0).map(|(k, _)| *k).collect();
                            for k in kinds {
                                self.add_counters_to(o, k, 1);
                            }
                        }
                        Obj::Player { id } => {
                            let mut gained: Vec<CounterKind> = vec![];
                            {
                                let pl = &mut self.players[id as usize];
                                if pl.poison > 0 {
                                    pl.poison += 1;
                                    gained.push(CounterKind::Poison);
                                }
                                if pl.energy > 0 {
                                    pl.energy += 1;
                                    gained.push(CounterKind::Energy);
                                }
                                if pl.experience > 0 {
                                    pl.experience += 1;
                                    gained.push(CounterKind::Experience);
                                }
                            }
                            for kind in gained {
                                self.emit(Event::PlayerCounters { player: id, kind, delta: 1, by: self.cause_ctl.get() });
                            }
                        }
                        _ => {}
                    }
                }
                Exec::Done
            }
            Effect::RemoveAnyCounters { what, n, up_to } => {
                if let Some(Sub::KindPick { ids, left, n_each, kinds, dst, up_to: _ }) = ctx.sub.clone() {
                    ctx.sub = None;
                    let idx = match self.answer.take() {
                        Some(Choice::Pick { indices }) => indices.first().copied(),
                        _ => None,
                    };
                    let stop = idx.map_or(false, |i| i >= kinds.len());
                    let pick = if stop { None } else { idx.and_then(|i| kinds.get(i)).copied().or_else(|| kinds.first().copied()) };
                    let (ids, left) = if stop { (ids[1..].to_vec(), n_each) } else { (ids, left) };
                    return self.counter_kind_job(ctx, source, controller, ids, left, n_each, dst, pick, *up_to);
                }
                self.counters_removed.set(0);
                let (os, k) = {
                    let c = ec!();
                    (self.sel_objs(what, &c), self.eval_amount(n, &c))
                };
                // a player loses counters too ("remove a counter from target opponent": the most plentiful of poison / energy / experience / rad)
                for o in &os {
                    if let Obj::Player { id } = o {
                        for _ in 0..k.max(0) {
                            let pl = &mut self.players[*id as usize];
                            let slots = [pl.poison, pl.energy, pl.experience, pl.rad];
                            let (bi, bv) = slots.iter().enumerate().max_by_key(|(_, v)| **v).map(|(i, v)| (i, *v)).unwrap_or((0, 0));
                            if bv <= 0 {
                                break;
                            }
                            match bi {
                                0 => pl.poison -= 1,
                                1 => pl.energy -= 1,
                                2 => pl.experience -= 1,
                                _ => pl.rad -= 1,
                            }
                        }
                    }
                }
                let ids: Vec<CardId> = os.into_iter().filter_map(|o| if let Obj::Card { id } = o { Some(id) } else { None }).collect();
                self.counter_kind_job(ctx, source, controller, ids, k, k, None, None, *up_to)
            }
            Effect::MoveCounters { from, to, kind, n, up_to, at_least_one, each_missing } => {
                if let Some(Sub::KindPick { ids, left, n_each, kinds, dst, up_to: sub_up }) = ctx.sub.clone() {
                    ctx.sub = None;
                    let idx = match self.answer.take() {
                        Some(Choice::Pick { indices }) => indices.first().copied(),
                        _ => None,
                    };
                    let stop = idx.map_or(false, |i| i >= kinds.len());
                    let pick = if stop { None } else { idx.and_then(|i| kinds.get(i)).copied().or_else(|| kinds.first().copied()) };
                    let (ids, left) = if stop { (ids[1..].to_vec(), n_each) } else { (ids, left) };
                    self.kind_pick_min1.set(*at_least_one);
                    let r = self.counter_kind_job(ctx, source, controller, ids, left, n_each, dst, pick, sub_up);
                    self.kind_pick_min1.set(false);
                    return r;
                }
                if *each_missing {
                    self.counters_removed.set(0);
                    let (src, dst) = {
                        let c = ec!();
                        (self.sel_cards(from, &c).first().copied(), self.sel_cards(to, &c).first().copied())
                    };
                    if let (Some(src), Some(dst)) = (src, dst) {
                        if src != dst {
                            let have: Vec<CounterKind> = self.cards[src.0 as usize].counters.iter().filter(|(_, x)| *x > 0).map(|(k, _)| *k).collect();
                            let mut moved = 0;
                            for kd in have {
                                if self.cards[dst.0 as usize].counter(kd) == 0 {
                                    self.remove_counters_from(Obj::card(src), kd, 1);
                                    self.add_counters_to(Obj::card(dst), kd, 1);
                                    moved += 1;
                                }
                            }
                            self.counters_removed.set(moved);
                        }
                    }
                    return Exec::Done;
                }
                if *up_to {
                    self.counters_removed.set(0);
                    let (srcs, dst, k) = {
                        let c = ec!();
                        (self.sel_cards(from, &c), self.sel_cards(to, &c).first().copied(), n.as_ref().map_or(i32::MAX, |a| self.eval_amount(a, &c)))
                    };
                    let Some(dst) = dst else { return Exec::Done };
                    let ids: Vec<CardId> = srcs.into_iter().filter(|s| *s != dst).collect();
                    self.kind_pick_min1.set(*at_least_one);
                    let r = self.counter_kind_job(ctx, source, controller, ids, k, k, Some(dst), *kind, true);
                    self.kind_pick_min1.set(false);
                    return r;
                }
                let (src, dst, k) = {
                    let c = ec!();
                    (self.sel_cards(from, &c).first().copied(), self.sel_cards(to, &c).first().copied(), n.as_ref().map(|a| self.eval_amount(a, &c)))
                };
                if let (Some(src), Some(dst)) = (src, dst) {
                    if src != dst {
                        if kind.is_none() && k.is_some() {
                            // the player picks the kind of each counter moved
                            return self.counter_kind_job(ctx, source, controller, vec![src], k.unwrap_or(0), k.unwrap_or(0), Some(dst), None, false);
                        }
                        let gone = self.cards[src.0 as usize].zone != Zone::Battlefield;
                        let pool = if gone { &self.cards[src.0 as usize].left_counters } else { &self.cards[src.0 as usize].counters };
                        let have: Vec<(CounterKind, u16)> = pool.iter().copied().filter(|(kd, x)| *x > 0 && kind.map_or(true, |w| w == *kd)).collect();
                        let moves: Vec<(CounterKind, i32)> = match k {
                            None => have.iter().map(|(kd, x)| (*kd, *x as i32)).collect(),
                            Some(k) => have.iter().map(|(x, a)| (*x, (*a as i32).min(k))).collect(),
                        };
                        for (kd, m) in moves {
                            if m > 0 {
                                if !gone {
                                    self.remove_counters_from(Obj::card(src), kd, m);
                                }
                                self.add_counters_to(Obj::card(dst), kd, m);
                            }
                        }
                    }
                }
                Exec::Done
            }
            Effect::NoCountersWhileSource { what } => {
                let os = {
                    let c = ec!();
                    self.sel_objs(what, &c)
                };
                let sz = self.cards[source.0 as usize].zcc;
                for o in os {
                    let zc = if let Obj::Card { id } = o { self.cards[id.0 as usize].zcc } else { 0 };
                    self.no_counters_src.push((o, zc, source, sz));
                }
                Exec::Done
            }
            Effect::RemoveAllCounters { what } => {
                self.counters_removed.set(0);
                let os = {
                    let c = ec!();
                    self.sel_objs(what, &c)
                };
                for o in os {
                    if let Obj::Card { id } = o {
                        let have: Vec<(CounterKind, u16)> = self.cards[id.0 as usize].counters.iter().copied().collect();
                        for (k, n) in have {
                            self.remove_counters_from(o, k, n as i32);
                        }
                    } else if let Obj::Player { id } = o {
                        // poison, energy, experience and rad counters on a player
                        for k in [CounterKind::Poison, CounterKind::Energy, CounterKind::Experience, CounterKind::Rad] {
                            let pl = &self.players[id as usize];
                            let n = match k {
                                CounterKind::Poison => pl.poison,
                                CounterKind::Energy => pl.energy,
                                CounterKind::Experience => pl.experience,
                                _ => pl.rad,
                            };
                            if n > 0 {
                                self.remove_counters_from(o, k, n);
                                self.counters_removed.set(self.counters_removed.get() + n);
                            }
                        }
                    }
                }
                Exec::Done
            }
            Effect::SpellEntersWithCounters { what, n, sunburst, loyalty } => {
                let (cards, k) = {
                    let c = ec!();
                    (self.sel_cards(what, &c), self.eval_amount(n, &c))
                };
                for id in cards {
                    if self.cards[id.0 as usize].zone == Zone::Stack {
                        let c = &mut self.cards[id.0 as usize];
                        if *loyalty {
                            c.rider_loyalty = c.rider_loyalty.saturating_add(k.clamp(0, 255) as u8);
                        } else if *sunburst && !self.db.def(c.def).types.has(CardType::Creature) {
                            c.rider_charge = c.rider_charge.saturating_add(k.clamp(0, 255) as u8);
                        } else {
                            c.rider_counters = c.rider_counters.saturating_add(k.clamp(0, 255) as u8);
                        }
                    }
                }
                Exec::Done
            }
            Effect::Planeswalk => {
                self.planeswalk(controller);
                Exec::Done
            }
            Effect::ChaosEnsues => {
                self.emit(Event::ChaosEnsued { player: controller });
                Exec::Done
            }
            Effect::PlanarTopToBottom => {
                if let Some(top) = self.planar_deck.pop() {
                    self.planar_deck.insert(0, top);
                }
                Exec::Done
            }
            Effect::ChaosFromPlanarDeck { n } => {
                self.chaos_from_planar_deck(controller, *n as usize);
                Exec::Done
            }
            Effect::ChaosFromPlanarDeckUntilPlane => {
                let n = self.planar_deck.iter().rev().position(|id| self.def_of(*id).types.has(CardType::Plane)).map_or(self.planar_deck.len(), |i| i + 1);
                self.chaos_from_planar_deck(controller, n);
                Exec::Done
            }
            Effect::PlaneswalkToRevealedPlanes { n, stay } => {
                self.planeswalk_to_revealed(controller, *n as usize, *stay);
                Exec::Done
            }
            Effect::PlanarTunnel { n } => {
                if let Some(Sub::PlanarTunnel { planes, rest }) = ctx.sub.clone() {
                    let ans = self.answer.take();
                    ctx.sub = None;
                    let pick = match ans {
                        Some(Choice::Pick { indices }) => indices.first().copied().and_then(|i| planes.get(i).copied()),
                        _ => planes.first().copied(),
                    };
                    let mut others = rest;
                    others.extend(planes.iter().copied().filter(|c| Some(*c) != pick));
                    self.tunnel_finish(others, pick);
                    return Exec::Done;
                }
                let (planes, rest) = self.reveal_planes(*n as usize);
                if planes.is_empty() {
                    self.tunnel_finish(rest, None);
                    return Exec::Done;
                }
                if planes.len() == 1 {
                    self.tunnel_finish(rest, planes.first().copied());
                    return Exec::Done;
                }
                ctx.sub = Some(Sub::PlanarTunnel { planes: planes.clone(), rest });
                self.pending = Some(Decision {
                    player: controller,
                    kind: DecisionKind::Pick {
                        prompt: "Put a plane card on top of your planar deck".into(),
                        source: Some(source),
                        options: planes.iter().map(|c| Opt::Obj { obj: Obj::card(*c) }).collect(),
                        min: 1,
                        max: 1,
                        cancellable: false,
                        hidden: false,
                        purpose: PickPurpose::Generic,
                    },
                });
                Exec::Wait
            }
            Effect::NoEnterFromExileThisTurn => {
                self.no_enter_from_exile_turn = self.turn + 1;
                Exec::Done
            }
            Effect::ChaoticAetherRolls => {
                self.chaotic_aether = true;
                Exec::Done
            }
            Effect::PlaneswalkRollsChaosUntilNextTurn => {
                self.fixed_point_until.push(controller);
                Exec::Done
            }
            Effect::CantAttackYouUntilPlaneswalk => {
                if !self.no_attack_until_planeswalk.contains(&controller) {
                    self.no_attack_until_planeswalk.push(controller);
                }
                Exec::Done
            }
            Effect::CantLoseUntilPlaneswalk => {
                if !self.safe_until_planeswalk.contains(&controller) {
                    self.safe_until_planeswalk.push(controller);
                }
                Exec::Done
            }
            Effect::NoSchemesNextTurn => {
                self.scheme_ban_next = true;
                Exec::Done
            }
            Effect::TripleNextScheme => {
                self.scheme_triple.push(controller);
                Exec::Done
            }
            Effect::SetSchemeInMotion => {
                self.set_scheme_in_motion(controller);
                Exec::Done
            }
            Effect::SetInMotionAgain { what } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                for id in cs {
                    let owner = self.cards[id.0 as usize].owner;
                    if self.players[owner as usize].command.contains(&id) {
                        let name = self.def_of(id).name.clone();
                        self.emit(Event::SchemeSetInMotion { player: owner, card: id, name });
                    }
                }
                Exec::Done
            }
            Effect::AbandonScheme => {
                self.abandon_scheme(source);
                Exec::Done
            }
            Effect::CreateEmblem { emblem, who, bind, until_end_of_turn } => {
                let bound: Option<Obj> = bind.as_ref().and_then(|b| {
                    let c = ec!();
                    self.sel_objs(b, &c).first().cloned()
                });
                let controller = who.as_ref().and_then(|w| {
                    let c = ec!();
                    self.sel_players(w, &c).first().copied()
                }).unwrap_or(controller);
                let Some(def) = self.db.find(emblem) else {
                    self.log(format!("unknown emblem {emblem}"));
                    return Exec::Done;
                };
                let id = self.create_card(def, controller, Zone::Command, true);
                self.cards[id.0 as usize].controller = controller;
                // "choose a creature type. You get an emblem with ...": the emblem keeps the source's choice (Oko, Lorwyn Liege)
                self.cards[id.0 as usize].chosen_type = self.cards[source.0 as usize].chosen_type;
                match bound {
                    Some(Obj::Player { id: pl }) => self.cards[id.0 as usize].chosen_player = Some(pl),
                    Some(Obj::Card { id: cd }) => self.cards[id.0 as usize].bound_card = Some(cd),
                    _ => {}
                }
                self.cards[id.0 as usize].emblem_eot = *until_end_of_turn;
                self.layers_dirty = true;
                Exec::Done
            }
            Effect::CreateToken { token, n, controller: who, tapped, attacking, attach_to, set_pt, attack_at, choose_defender } => {
                // "tapped and attacking": the controller chooses which player or planeswalker/battle (of the players already being attacked) it attacks
                let mut chosen_def: Option<Obj> = None;
                if *attacking && attack_at.is_none() && *choose_defender {
                    if let Some(Sub::DefenderPick { options }) = ctx.sub.clone() {
                        ctx.sub = None;
                        let i = match self.answer.take() {
                            Some(Choice::Pick { indices }) => indices.first().copied().unwrap_or(0),
                            _ => 0,
                        };
                        chosen_def = options.get(i).or(options.first()).copied();
                    } else if let Some(cb) = self.combat.as_ref() {
                        let mut sides: Vec<PlayerId> = vec![];
                        for a in &cb.attackers {
                            let side = match a.defender {
                                Obj::Player { id } => id,
                                Obj::Card { id } => self.defender_side(id),
                                _ => continue,
                            };
                            if !sides.contains(&side) {
                                sides.push(side);
                            }
                        }
                        let mut options: Vec<Obj> = vec![];
                        for &sd in &sides {
                            options.push(Obj::player(sd));
                            for &pid in &self.battlefield {
                                let c = &self.cards[pid.0 as usize];
                                if (c.cur.types.has(CardType::Planeswalker) || c.cur.types.has(CardType::Battle)) && self.defender_side(pid) == sd {
                                    options.push(Obj::card(pid));
                                }
                            }
                        }
                        if options.len() > 1 {
                            let ctl = ctx.item.controller;
                            ctx.sub = Some(Sub::DefenderPick { options: options.clone() });
                            self.pending = Some(Decision {
                                player: ctl,
                                kind: DecisionKind::Pick {
                                    prompt: "Choose a player or planeswalker being attacked".into(),
                                    source: Some(source),
                                    options: options.iter().map(|o| Opt::Obj { obj: *o }).collect(),
                                    min: 1,
                                    max: 1,
                                    cancellable: false,
                                    hidden: false,
                                    purpose: PickPurpose::Generic,
                                },
                            });
                            return Exec::Wait;
                        }
                    }
                }
                let pt_now: Option<(i32, i32)> = set_pt.as_ref().map(|(a, b)| {
                    let c = ec!();
                    (self.eval_amount(a, &c), self.eval_amount(b, &c))
                });
                let (ps, k, host) = {
                    let c = ec!();
                    (self.sel_players(who, &c), self.eval_amount(n, &c), attach_to.as_ref().and_then(|s| self.sel_cards(s, &c).first().copied()))
                };
                if attach_to.is_some() && host.map_or(true, |h| self.cards[h.0 as usize].zone != Zone::Battlefield) {
                    return Exec::Done;
                }
                let Some(def) = self.db.find(token) else {
                    self.log(format!("unknown token {token}"));
                    return Exec::Done;
                };
                let mut crown: Option<(PlayerId, CardId)> = None;
                let mut choice_tok: Option<(PlayerId, String)> = None;
                if let Some(Sub::CrownCopy { player, host }) = ctx.sub.clone() {
                    ctx.sub = None;
                    if let Some(Choice::YesNo { yes: true }) = self.answer.take() {
                        crown = Some((player, host));
                    }
                } else if let Some(Sub::TokenCopyPick { player, options }) = ctx.sub.clone() {
                    ctx.sub = None;
                    if let Some(Choice::Pick { indices }) = self.answer.take() {
                        if let Some(&h) = indices.first().and_then(|i| options.get(*i)) {
                            crown = Some((player, h));
                        }
                    }
                } else if let Some(Sub::TokenChoice { player, options }) = ctx.sub.clone() {
                    ctx.sub = None;
                    if let Some(Choice::Pick { indices }) = self.answer.take() {
                        if let Some(t) = indices.first().and_then(|i| options.get(*i)) {
                            choice_tok = Some((player, t.clone()));
                        }
                    }
                } else if let Some((pc, (cr, host))) = ps.iter().find_map(|&q| self.token_copy_offer(q).map(|o| (q, o))) {
                    self.cards[cr.0 as usize].token_repl_turn = self.turn;
                    ctx.sub = Some(Sub::CrownCopy { player: pc, host });
                    self.pending = Some(Decision {
                        player: pc,
                        kind: DecisionKind::YesNo { prompt: format!("Create copies of {} instead of the tokens?", self.def_of(host).name), source: Some(cr) },
                    });
                    return Exec::Wait;
                } else if let Some((pc, (cr, options))) = ps.iter().find_map(|&q| self.token_copy_pick_offer(q).map(|o| (q, o))) {
                    self.cards[cr.0 as usize].token_repl_turn = self.turn;
                    self.pending = Some(Decision {
                        player: pc,
                        kind: DecisionKind::Pick {
                            prompt: "Choose a creature to create copies of instead of the tokens".to_string(),
                            source: Some(cr),
                            options: options.iter().map(|c| Opt::Obj { obj: Obj::card(*c) }).collect(),
                            min: 0,
                            max: 1,
                            cancellable: false,
                            hidden: false,
                            purpose: PickPurpose::Generic,
                        },
                    });
                    ctx.sub = Some(Sub::TokenCopyPick { player: pc, options });
                    return Exec::Wait;
                } else if k > 0 && matches!(ctx.sub, None) {
                    if let Some((pc, (cr, options))) = ps.iter().find_map(|&q| self.token_choice_offer(q).map(|o| (q, o))) {
                        self.pending = Some(Decision {
                            player: pc,
                            kind: DecisionKind::Pick {
                                prompt: "Create one of these tokens instead? (choose none to create the original tokens)".to_string(),
                                source: Some(cr),
                                options: options.iter().map(|t| Opt::Text { text: t.clone() }).collect(),
                                min: 0,
                                max: 1,
                                cancellable: false,
                                hidden: false,
                                purpose: PickPurpose::Generic,
                            },
                        });
                        ctx.sub = Some(Sub::TokenChoice { player: pc, options });
                        return Exec::Wait;
                    }
                }
                let mut made = vec![];
                let defer_here = self.defer_events.is_none() && (ps.len() > 1 || k > 1 || (k > 0 && ps.first().map_or(false, |&q| self.replace_token_count(q, k) > 1)));
                if defer_here {
                    self.defer_events = Some(vec![]);
                }
                for p in ps {
                    let (tok_name, tok_def) = match &choice_tok {
                        Some((pc, t)) if *pc == p => match self.db.find(t) {
                            Some(d2) => (t.as_str(), d2),
                            None => (token.as_str(), def),
                        },
                        _ => (token.as_str(), def),
                    };
                    let (tname, k, extras, p) = self.replace_token_plan(p, tok_name, self.db.def(tok_def), k);
                    let main_def = match crown {
                        Some((pc, host)) if pc == p => self.cards[host.0 as usize].def,
                        _ => self.db.find(&tname).unwrap_or(tok_def),
                    };
                    let mut batches: Vec<(DefId, i32)> = vec![(main_def, k)];
                    for (name, cnt) in extras {
                        if let Some(d2) = self.db.find(&name) {
                            batches.push((d2, cnt));
                        }
                    }
                    for (bdef, cnt) in batches {
                        for _ in 0..cnt.max(0) {
                            if self.token_cap_reached() {
                                break;
                            }
                            let id = self.create_card(bdef, p, Zone::Exile, true);
                            self.cards[id.0 as usize].zone = Zone::Exile;
                            // remove from the exile list it was created into
                            self.players[p as usize].exile.retain(|x| *x != id);
                            self.cards[id.0 as usize].zone = Zone::Library;
                            self.move_card(id, Zone::Battlefield, MoveOpts { tapped: *tapped, controller: Some(p), attach_to: host, ..Default::default() });
                            self.cards[id.0 as usize].created_by = Some((source, self.cards[source.0 as usize].zcc));
                            if let Some((pw, th)) = pt_now {
                                self.add_effect(source, controller, &[id], Modif::SetPT { power: pw, toughness: th }, Duration::Forever);
                                self.cards[id.0 as usize].token_set_pt = Some((pw, th));
                            }
                            self.emit(Event::Token { card: id });
                            if *attacking {
                                let at = attack_at.as_ref().and_then(|s| {
                                    let c = ec!();
                                    self.sel_players(s, &c).first().copied()
                                });
                                match at {
                                    Some(d) => self.add_attacker_at(id, Obj::player(d)),
                                    None if chosen_def.is_some() => self.add_attacker_at(id, chosen_def.unwrap()),
                                    None => self.add_attacker_late(id, controller),
                                }
                            }
                            made.push(Obj::card(id));
                        }
                    }
                }
                if defer_here {
                    self.flush_deferred_events();
                }
                ctx.remembered = made;
                Exec::Done
            }
            Effect::CopyPermanent { what, n, except, tapped, attacking, by, attach_to, attack_at } => {
                let (cs, k, owners) = {
                    let c = ec!();
                    let owners = match by {
                        Some(sel) => self.sel_players(sel, &c),
                        None => vec![controller],
                    };
                    (self.sel_cards(what, &c), self.eval_amount(n, &c), owners)
                };
                // "create a token that's a copy of this" still works from last known information after the source left (squad, offspring)
                let cs = if cs.is_empty() && matches!(what, Sel::This) { vec![source] } else { cs };
                let mut made = vec![];
                let defer_here = self.defer_events.is_none() && (cs.len() * owners.len() > 1 || owners.first().map_or(false, |&o| self.replace_token_count(o, k) > 1));
                if defer_here {
                    self.defer_events = Some(vec![]);
                }
                for src in cs {
                  for controller in owners.iter().copied() {
                    let k = self.replace_token_count(controller, k);
                    for _ in 0..k.max(0) {
                        if self.token_cap_reached() {
                            break;
                        }
                        let def = self.cards[src.0 as usize].def;
                        // a copy of a face-down permanent copies its face-down characteristics (CR 707.2): a nameless colorless 2/2 with no abilities
                        let copy_face_down = self.cards[src.0 as usize].face_down;
                        let id = self.create_card(def, controller, Zone::Exile, true);
                        self.players[controller as usize].exile.retain(|x| *x != id);
                        self.cards[id.0 as usize].zone = Zone::Library;
                        self.cards[id.0 as usize].made_by = Some((source, self.cards[source.0 as usize].zcc));
                        self.cards[id.0 as usize].linked_card = Some(src);
                        // a copy of an Aura enters attached to a permanent it could enchant (the first legal one)
                        let host = match attach_to {
                            Some(sel) => {
                                let c = ec!();
                                self.sel_cards(sel, &c).first().copied().filter(|h| self.cards[h.0 as usize].zone == Zone::Battlefield)
                            }
                            None => self.aura_hosts(id).and_then(|hs| hs.first().copied()),
                        };
                        // entering face down (no ETB abilities, face-down characteristics); this is not a face-down turn by the controller
                        let act_turn = self.players[controller as usize].face_down_act_turn;
                        self.move_card(id, Zone::Battlefield, MoveOpts { controller: Some(controller), tapped: *tapped, attach_to: host, face_down: copy_face_down, ..Default::default() });
                        self.players[controller as usize].face_down_act_turn = act_turn;
                        if copy_face_down && self.cards[id.0 as usize].zone == Zone::Battlefield {
                            self.cards[id.0 as usize].fd_copy = true;
                            self.recompute_layers();
                        }
                        self.apply_copy_except(source, controller, id, except, &ctx.remembered);
                        // a copy of an "X/X token" copies that size too (it is the token's characteristic, not an effect on it)
                        if let (Some((pw, th)), true) = (self.cards[src.0 as usize].token_set_pt, self.cards[id.0 as usize].zone == Zone::Battlefield) {
                            self.add_effect(source, controller, &[id], Modif::SetPT { power: pw, toughness: th }, Duration::Forever);
                            self.cards[id.0 as usize].token_set_pt = Some((pw, th));
                        }
                        if except.iter().any(|x| matches!(x, CopyExcept::GrantAbility { .. })) && self.cards[id.0 as usize].zone == Zone::Battlefield {
                            // the copy's "when this enters" granted by the effect fires now (the printed ones fired when it entered)
                            self.recompute_layers();
                            self.granted_only = true;
                            crate::triggers::on_event_for(self, &Event::Moved { card: id, from: Zone::Stack, to: Zone::Battlefield }, Some(id));
                            self.granted_only = false;
                        }
                        if *attacking && self.cards[id.0 as usize].zone == Zone::Battlefield {
                            let at = attack_at.as_ref().and_then(|s| {
                                let c = ec!();
                                self.sel_players(s, &c).first().copied()
                            });
                            match at {
                                Some(d) => self.add_attacker_at(id, Obj::player(d)),
                                None => self.add_attacker_late(id, controller),
                            }
                        }
                        self.emit(Event::Token { card: id });
                        made.push(Obj::card(id));
                    }
                }
                  }
                if defer_here {
                    self.flush_deferred_events();
                }
                ctx.remembered = made;
                Exec::Done
            }
            Effect::Discard { who, n, random, filter } => {
                if let Some(Sub::Discard { players, n: ns, filter, idx }) = ctx.sub.clone() {
                    return self.continue_discard(ctx, players, ns, filter, idx);
                }
                let (ps, k) = {
                    let c = ec!();
                    (self.sel_players(who, &c), self.eval_amount(n, &c))
                };
                if *random {
                    let mut gone: Vec<Obj> = vec![];
                    for p in ps {
                        for _ in 0..k.max(0) {
                            let eligible = self.eligible_in_hand(p, filter, source);
                            if eligible.is_empty() {
                                break;
                            }
                            let i = self.rng.gen_range(0..eligible.len());
                            let id = eligible[i];
                            self.move_card(id, Zone::Graveyard, Default::default());
                            self.emit(Event::Discarded { card: id, player: p, by: self.cause_ctl.get() });
                            gone.push(Obj::card(id));
                        }
                    }
                    if !gone.is_empty() {
                        ctx.remembered = gone;
                    }
                    return Exec::Done;
                }
                let ns = vec![k.max(0); ps.len()];
                self.continue_discard(ctx, ps, ns, filter.clone().into_filter(), 0)
            }
            Effect::DiscardRemembered => {
                let ids: Vec<CardId> = ctx.remembered.iter().filter_map(|o| if let Obj::Card { id } = o { Some(*id) } else { None }).filter(|id| self.cards[id.0 as usize].zone == Zone::Hand).collect();
                for id in ids {
                    let p = self.cards[id.0 as usize].owner;
                    self.move_card(id, Zone::Graveyard, Default::default());
                    self.emit(Event::Discarded { card: id, player: p, by: self.cause_ctl.get() });
                }
                Exec::Done
            }
            Effect::Mill { who, n } => {
                let (ps, k) = {
                    let c = ec!();
                    (self.sel_players(who, &c), self.eval_amount(n, &c))
                };
                let mut milled: Vec<Obj> = vec![];
                for p in ps {
                    let mut these: Vec<CardId> = vec![];
                    for _ in 0..self.replace_mill_count(p, k) {
                        if let Some(&id) = self.players[p as usize].library.last() {
                            self.move_card(id, Zone::Graveyard, Default::default());
                            if self.cards[id.0 as usize].zone == Zone::Graveyard {
                                let (t, z) = (self.turn, self.cards[id.0 as usize].zcc);
                                self.cards[id.0 as usize].milled_turn = t;
                                self.cards[id.0 as usize].milled_zcc = z;
                            }
                            milled.push(Obj::card(id));
                            these.push(id);
                        }
                    }
                    if !these.is_empty() {
                        self.emit(Event::Milled { player: p, cards: these });
                    }
                }
                if !milled.is_empty() {
                    ctx.remembered = milled;
                }
                Exec::Done
            }
            Effect::ExileTopOfLibrary { who, n, face_down } => {
                let (ps, k) = {
                    let c = ec!();
                    (self.sel_players(who, &c), self.eval_amount(n, &c))
                };
                let mut exiled: Vec<Obj> = vec![];
                for p in ps {
                    for _ in 0..k.max(0) {
                        if let Some(&id) = self.players[p as usize].library.last() {
                            self.cur_src = Some(source);
                            self.move_card(id, Zone::Exile, MoveOpts { face_down: *face_down, ..Default::default() });
                            exiled.push(Obj::card(id));
                        }
                    }
                }
                if !exiled.is_empty() {
                    ctx.remembered = exiled;
                }
                Exec::Done
            }
            Effect::AddReplacement { what, until_your_next_turn } => {
                // the stored slots it reads are pinned as they are now (they only last for this resolution)
                let mut what = self.pin_stored(what).unwrap_or_else(|| what.clone());
                if let Replace::Damage { bind_player: Some(sel), bound_player, .. } = &mut what {
                    let c = ec!();
                    *bound_player = self.sel_players(sel, &c).first().copied();
                }
                // an {X} ability's damage bonus keeps that X ("deals that much damage plus X instead": Taii Wakeen)
                if let Replace::Damage { add_amount: Some(Amount::X), .. } = &what {
                    if let Replace::Damage { add_amount, .. } = &mut what {
                        *add_amount = Some(Amount::Fixed { n: ctx.item.x });
                    }
                }
                if let Replace::Damage { prevent_up_to, prevent_up_to_amount: Some(a), .. } = &mut what {
                    let c = ec!();
                    *prevent_up_to = Some(self.eval_amount(a, &c).max(0));
                }
                if let Replace::EntersAsCopyOf { of, bind: true, bound_of, .. } = &mut what {
                    let c = ec!();
                    *bound_of = self.sel_cards(of, &c).first().copied();
                }
                if let Replace::Damage { bind_then_target: Some(i), bound_then_target, .. } = &mut what {
                    let c = ec!();
                    *bound_then_target = c.target(*i).cloned();
                }
                if let Replace::Damage { bind_chosen_color: true, bound_color, .. } = &mut what {
                    *bound_color = self.cards[source.0 as usize].chosen_color;
                }
                if let Replace::Damage { bind_to_player: Some(sel), bound_to_player, .. } = &mut what {
                    let c = ec!();
                    *bound_to_player = self.sel_players(sel, &c).first().copied();
                }
                if let Replace::Damage { bind_source, bind_to, bound_source, bound_to, .. } = &mut what {
                    if let Some(sel) = bind_source.clone() {
                        let c = ec!();
                        *bound_source = self.sel_cards(&sel, &c).first().copied();
                    }
                    if let Some(sel) = bind_to.clone() {
                        let c = ec!();
                        *bound_to = self.sel_cards(&sel, &c).first().copied();
                    }
                }
                if let Replace::ZoneChange { bind_to, bound_to, bind_damager, bound_damager, .. } = &mut what {
                    if let Some(sel) = bind_to.clone() {
                        let c = ec!();
                        *bound_to = self.sel_cards(&sel, &c).first().copied();
                    }
                    if let Some(sel) = bind_damager.clone() {
                        let c = ec!();
                        *bound_damager = self.sel_cards(&sel, &c).first().copied();
                    }
                }
                if let Replace::Draw { bind_then_target: Some(i), bound_then_target, .. } = &mut what {
                    let c = ec!();
                    *bound_then_target = c.target(*i).cloned();
                }
                // a draw replacement created by an {X} ability keeps that X ("instead look at the top X cards": Aladdin's Lamp)
                if let Replace::Draw { instead, .. } = &mut what {
                    if !instead.is_empty() {
                        let x = ctx.item.x;
                        if let Ok(mut v) = serde_json::to_value(&*instead) {
                            fn bake(v: &mut serde_json::Value, x: i32) {
                                match v {
                                    serde_json::Value::Object(m) => {
                                        if m.len() == 1 && m.get("type").and_then(|t| t.as_str()) == Some("X") {
                                            m.insert("type".into(), "Fixed".into());
                                            m.insert("n".into(), x.into());
                                        } else {
                                            for (_, c) in m.iter_mut() {
                                                bake(c, x);
                                            }
                                        }
                                    }
                                    serde_json::Value::Array(a) => a.iter_mut().for_each(|c| bake(c, x)),
                                    _ => {}
                                }
                            }
                            bake(&mut v, x);
                            if let Ok(e) = serde_json::from_value(v) {
                                *instead = e;
                            }
                        }
                    }
                }
                if *until_your_next_turn {
                    self.long_replaces.push((source, controller, what));
                } else {
                    self.temp_replaces.push((source, controller, what));
                }
                Exec::Done
            }
            Effect::SetDoor { what, door, locked } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                for id in cs {
                    if self.cards[id.0 as usize].zone != Zone::Battlefield || !self.def_of(id).room || !(1..=2).contains(door) {
                        continue;
                    }
                    let bit = 1u8 << (*door - 1);
                    let is_unlocked = self.cards[id.0 as usize].doors & bit != 0;
                    if *locked && is_unlocked {
                        self.cards[id.0 as usize].doors &= !bit;
                        self.layers_dirty = true;
                    } else if !*locked && !is_unlocked {
                        self.cards[id.0 as usize].doors |= bit;
                        self.layers_dirty = true;
                        self.emit(Event::Unlocked { card: id, door: *door });
                        if self.cards[id.0 as usize].doors == 3 {
                            self.emit(Event::FullyUnlocked { card: id });
                        }
                    }
                }
                Exec::Done
            }
            Effect::UnlockDoor { what } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                for id in cs {
                    if self.cards[id.0 as usize].zone != Zone::Battlefield || !self.def_of(id).room {
                        continue;
                    }
                    for door in 1u8..=2 {
                        if self.cards[id.0 as usize].doors & (1 << (door - 1)) == 0 {
                            self.cards[id.0 as usize].doors |= 1 << (door - 1);
                            self.layers_dirty = true;
                            self.emit(Event::Unlocked { card: id, door });
                            if self.cards[id.0 as usize].doors == 3 {
                                self.emit(Event::FullyUnlocked { card: id });
                            }
                            break;
                        }
                    }
                }
                Exec::Done
            }
            Effect::PhaseOut { what, until_source_leaves, lock_until_your_next_turn_ends } => {
                // "until this leaves the battlefield" when it already left: nothing phases out (Out of Time destroyed in response to its trigger)
                let in_play = self.cards[source.0 as usize].zone == Zone::Battlefield || self.planes.contains(&source);
                if *until_source_leaves && (!in_play || self.cards[source.0 as usize].zcc != ctx.item.src_zcc) {
                    return Exec::Done;
                }
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                for id in cs {
                    self.phase_out(id, false);
                    if *lock_until_your_next_turn_ends {
                        self.cards[id.0 as usize].phase_lock = Some((ctx.item.controller, self.turn));
                    }
                    if *until_source_leaves && in_play {
                        self.cards[id.0 as usize].phase_hold = Some((source, self.cards[source.0 as usize].zcc));
                    }
                }
                Exec::Done
            }
            Effect::PhaseIn { what } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                for id in cs {
                    self.cards[id.0 as usize].phase_hold = None;
                    self.phase_in(id);
                }
                Exec::Done
            }
            Effect::Pair { a, b } => {
                let (x, y) = {
                    let c = ec!();
                    (self.sel_cards(a, &c).first().copied(), self.sel_cards(b, &c).first().copied())
                };
                if let (Some(x), Some(y)) = (x, y) {
                    let ok = |g: &Game, id: CardId| g.cards[id.0 as usize].zone == Zone::Battlefield && g.cards[id.0 as usize].paired.is_none() && g.cards[id.0 as usize].cur.types.has(CardType::Creature);
                    if x != y && ok(self, x) && ok(self, y) && self.cards[x.0 as usize].cur.controller == self.cards[y.0 as usize].cur.controller {
                        self.cards[x.0 as usize].paired = Some(y);
                        self.cards[y.0 as usize].paired = Some(x);
                        self.layers_dirty = true;
                    }
                }
                Exec::Done
            }
            Effect::Airbend { what, cost } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                for id in cs {
                    if self.cards[id.0 as usize].zone != Zone::Exile {
                        self.move_card(id, Zone::Exile, Default::default());
                    }
                    if self.cards[id.0 as usize].zone == Zone::Exile {
                        let owner = self.cards[id.0 as usize].owner;
                        let c = &mut self.cards[id.0 as usize];
                        c.exile_cast_cost = Some(cost.clone());
                        c.play_until = u32::MAX;
                        c.play_by = owner;
                    }
                }
                self.emit(Event::Bent { player: controller, kind: BendKind::Air });
                Exec::Done
            }
            Effect::SaddleTargets { what } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                for id in cs {
                    if self.cards[id.0 as usize].zone == Zone::Battlefield {
                        self.cards[id.0 as usize].saddled = true;
                        self.emit(Event::Saddled { card: id });
                    }
                }
                Exec::Done
            }
            Effect::Saddle => {
                self.cards[source.0 as usize].saddled = true;
                self.emit(Event::Saddled { card: source });
                Exec::Done
            }
            Effect::Tap { what } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                for id in cs {
                    self.tap_card(id);
                }
                Exec::Done
            }
            Effect::Untap { what } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                for id in cs {
                    // a stun counter replaces an untap: remove one instead (rule 122.1g)
                    if self.cards[id.0 as usize].tapped && self.cards[id.0 as usize].counter(CounterKind::Stun) > 0 {
                        if !self.stun_locked(id) {
                            self.cards[id.0 as usize].remove_counters(CounterKind::Stun, 1);
                            self.layers_dirty = true;
                        }
                        continue;
                    }
                    self.untap_card(id);
                }
                Exec::Done
            }
            Effect::Sacrifice { who, filter, n } => {
                if let Some(Sub::Sacrifice { players, n: ns, filter, idx }) = ctx.sub.clone() {
                    return self.continue_sacrifice(ctx, players, ns, filter, idx);
                }
                self.sac_max_power.set(0);
                // a selector naming specific permanents (Remembered, a target, the trigger's object): sacrifice exactly those you control
                if matches!(who, Sel::Remembered | Sel::Target { .. } | Sel::Targets | Sel::TriggerObject | Sel::TriggerSource | Sel::AttachedTo | Sel::Enchanted | Sel::All { .. }) {
                    let cs = {
                        let c = ec!();
                        self.sel_cards(who, &c)
                    };
                    // (an edict names a player with the same selectors: that goes through the normal path below)
                    if !cs.is_empty() {
                        let all = matches!(who, Sel::All { .. });
                        let mut gone = vec![];
                        for id in cs {
                            // "sacrifice it" by a delayed trigger: you can't sacrifice a permanent another player controls now (Wings of Hubris)
                            if matches!(who, Sel::AttachedTo) && matches!(ctx.item.kind, ItemKind::Delayed { .. }) && self.cards[id.0 as usize].cur.controller != controller {
                                continue;
                            }
                            if self.cards[id.0 as usize].zone == Zone::Battlefield && !self.cards[id.0 as usize].cur.cant_be_sacrificed && !self.trigger_token_barred(id) && !self.forced_sac_barred(self.cards[id.0 as usize].cur.controller) {
                                // the permanent's own controller sacrifices it ("target creature's controller sacrifices it")
                                let owner_of_it = self.cards[id.0 as usize].cur.controller;
                                self.emit(Event::Sacrificed { card: id, player: owner_of_it });
                                self.move_card(id, Zone::Graveyard, Default::default());
                                gone.push(Obj::card(id));
                            }
                        }
                        // "sacrifice all X" remembers what was sacrificed (Bringer of the Last Gift)
                        if all {
                            ctx.remembered = gone;
                        }
                        return Exec::Done;
                    }
                }
                // `who: This` means "sacrifice this permanent" (same as who: You with a `this` filter)
                if matches!(who, Sel::This) {
                    let f = Filter { this: true, ..Default::default() };
                    let p = self.cards[source.0 as usize].cur.controller;
                    // a player sacrifices only permanents they control: a creature another player took control of is not sacrificed by the old controller's delayed trigger (Crumbling Colossus)
                    if p != controller && matches!(ctx.item.kind, ItemKind::Delayed { .. }) {
                        return Exec::Done;
                    }
                    return self.continue_sacrifice(ctx, vec![p], vec![1], f, 0);
                }
                let (ps, k) = {
                    let c = ec!();
                    (self.sel_players(who, &c), self.eval_amount(n, &c))
                };
                let ns = vec![k.max(0); ps.len()];
                self.continue_sacrifice(ctx, ps, ns, filter.clone().into_filter(), 0)
            }
            Effect::Scry { who, n, .. } | Effect::Surveil { who, n } => {
                let surveil = matches!(e, Effect::Surveil { .. });
                let scry_decider: Option<&Sel> = if let Effect::Scry { decider, .. } = e { decider.as_deref() } else { None };
                if let Some(Sub::Scry { player, cards, surveil }) = ctx.sub.clone() {
                    let ans = self.answer.take();
                    ctx.sub = None;
                    // the scrying player has seen these cards: their identities (and the order they stay in) are no secret to them
                    for id in &cards {
                        self.cards[id.0 as usize].known_to |= 1u8 << (player as u32 & 7);
                    }
                    if surveil {
                        self.last_surveil_kept.set(cards.len() as i32);
                    }
                    let mut bottom_n = 0u32;
                    if let Some(Choice::Pick { indices }) = ans {
                        let chosen: Vec<CardId> = indices.iter().filter_map(|i| cards.get(*i).copied()).collect();
                        bottom_n = chosen.len() as u32;
                        if surveil {
                            self.last_surveil_kept.set((cards.len() - chosen.len()) as i32);
                            // "the card you put into your graveyard this way" (Enlightened Confidant)
                            ctx.remembered = chosen.iter().map(|c| Obj::card(*c)).collect();
                        }
                        for id in chosen {
                            if surveil {
                                self.move_card(id, Zone::Graveyard, Default::default());
                                if self.cards[id.0 as usize].zone == Zone::Graveyard {
                                    let (t, z) = (self.turn, self.cards[id.0 as usize].zcc);
                                    self.cards[id.0 as usize].surveilled_turn = t;
                                    self.cards[id.0 as usize].surveilled_zcc = z;
                                }
                            } else {
                                // to the bottom
                                self.players[player as usize].library.retain(|x| *x != id);
                                self.players[player as usize].library.insert(0, id);
                            }
                        }
                    }
                    self.players[player as usize].scried_turn = true;
                    if surveil {
                        self.players[player as usize].surveilled_turn = true;
                        self.players[player as usize].surveil_count_turn = self.players[player as usize].surveil_count_turn.saturating_add(1);
                    }
                    let bottom = if surveil { 0 } else { bottom_n };
                    self.emit(Event::Scry { player, n: cards.len() as u32, surveil, bottom });
                    return Exec::Done;
                }
                let (ps, k) = {
                    let c = ec!();
                    (self.sel_players(who, &c), self.eval_amount(n, &c))
                };
                let Some(&p) = ps.first() else { return Exec::Done };
                let k = if surveil { if k > 0 { k + self.surveil_extra_look(p) } else { k } } else { self.replace_scry_count(p, k) };
                if !surveil && k > 0 && self.scry_becomes_draw(p) {
                    for _ in 0..k {
                        self.draw_with_replacement(p, false, false);
                    }
                    self.flush_followups();
                    return Exec::Done;
                }
                let lib = &self.players[p as usize].library;
                let take = (k.max(0) as usize).min(lib.len());
                if take == 0 {
                    return Exec::Done;
                }
                let cards: Vec<CardId> = lib.iter().rev().take(take).copied().collect(); // top first
                ctx.sub = Some(Sub::Scry { player: p, cards: cards.clone(), surveil });
                let decides = match scry_decider {
                    Some(d) => {
                        let c = ec!();
                        self.sel_players(d, &c).first().copied().unwrap_or(p)
                    }
                    None => p,
                };
                self.pending = Some(Decision {
                    player: decides,
                    kind: DecisionKind::Pick {
                        prompt: if surveil { "Surveil: choose cards to put into your graveyard".into() } else { "Scry: choose cards to put on the bottom".into() },
                        source: Some(source),
                        options: cards.iter().map(|c| Opt::Obj { obj: Obj::card(*c) }).collect(),
                        min: 0,
                        max: take as u8,
                        cancellable: false,
                        hidden: true,
                        purpose: if surveil { PickPurpose::Surveil } else { PickPurpose::Scry },
                    },
                });
                Exec::Wait
            }
            Effect::Shuffle { who } => {
                let ps = {
                    let c = ec!();
                    self.sel_players(who, &c)
                };
                for p in ps {
                    self.shuffle_library(p);
                }
                Exec::Done
            }
            Effect::SearchLibrary { who, filter, n, to, shuffle, tapped, reveal: _, under_your_control, under_control_of, by_owner, instead, graveyard_too } => {
                let under_your_control_flag = *under_your_control || under_control_of.is_some();
                let enter_ctl = under_control_of.as_ref().and_then(|s| { let c = self.evalctx(ctx); self.sel_players(s, &c).first().copied() }).unwrap_or(controller);
                if let Some(Sub::Search { player, found, to, shuffle, tapped }) = ctx.sub.clone() {
                    let ans = self.answer.take();
                    ctx.sub = None;
                    let mut skip_shuffle = false;
                    if let Some(Choice::Pick { indices }) = ans {
                        let chosen: Vec<CardId> = indices.iter().filter_map(|i| found.get(*i).copied()).collect();
                        ctx.remembered = chosen.iter().map(|c| Obj::card(*c)).collect();
                        // a card taken from the graveyard means the library wasn't searched, so it isn't shuffled
                        if *graveyard_too && !chosen.is_empty() && chosen.iter().all(|c| self.cards[c.0 as usize].zone == Zone::Graveyard) {
                            skip_shuffle = true;
                        }
                        let ctl_search = self.search_controller(if *by_owner { player } else { controller });
                        // several cards put onto the battlefield by one search enter simultaneously (Scapeshift + Field of the Dead)
                        let defer_search = to == Zone::Battlefield && chosen.len() > 1 && self.defer_events.is_none();
                        if defer_search {
                            self.defer_events = Some(vec![]);
                        }
                        for id in chosen {
                            if let Some((agent, actl)) = ctl_search {
                                // Opposition Agent: the found card is exiled and its controller may play it, spending mana as though it were any color
                                self.move_card(id, Zone::Exile, MoveOpts::default());
                                let azcc = self.cards[agent.0 as usize].zcc;
                                if self.cards[id.0 as usize].zone == Zone::Exile {
                                    let c = &mut self.cards[id.0 as usize];
                                    c.play_until = u32::MAX;
                                    c.play_by = actl;
                                    c.spend_any = true;
                                    c.played_via = Some((agent, azcc));
                                }
                                continue;
                            }
                            let mut dest = to;
                            if let Some(ins) = instead {
                                let ok = ins.cond.as_ref().map_or(true, |cd| self.eval_cond(cd, &ec!()));
                                if ok {
                                    let mut f = ins.filter.clone();
                                    f.controller = PlayerFilter::Any;
                                    if self.matches_card(&f, id, source, controller) {
                                        dest = ins.to;
                                    }
                                }
                            }
                            let mut opts = MoveOpts { tapped, controller: Some(if under_your_control_flag { enter_ctl } else { player }), ..Default::default() };
                            // an Aura put onto the battlefield without being cast: its controller chooses what it enchants; with no legal permanent it stays in the library
                            let mut ask_host = false;
                            if dest == Zone::Battlefield {
                                match self.aura_host_plan(id) {
                                    AuraHostPlan::NoHost => continue,
                                    AuraHostPlan::One(h) => opts.attach_to = Some(h),
                                    AuraHostPlan::Choose => ask_host = true,
                                    AuraHostPlan::NotAura => {}
                                }
                            }
                            self.move_card(id, dest, opts);
                            if ask_host && self.cards[id.0 as usize].zone == Zone::Battlefield {
                                self.attach_pending.push(id);
                            }
                        }
                        if defer_search {
                            self.flush_deferred_events();
                        }
                    }
                    if shuffle && !skip_shuffle {
                        self.shuffle_library(player);
                    }
                    return Exec::Done;
                }
                let (ps, k) = {
                    let c = ec!();
                    (self.sel_players(who, &c), self.eval_amount(n, &c))
                };
                let Some(&p) = ps.first() else { return Exec::Done };
                // Panglacial Wurm: a card in the searcher's own library that may be cast while they search
                let searcher = if *by_owner { p } else { controller };
                // the searching player is the one a search ban (Stranglehold) applies to
                if self.cant_search_libraries(searcher) {
                    return Exec::Done;
                }
                let mut asked_more = false;
                let mut declined = false;
                if let Some(Sub::SearchCast { player, card }) = ctx.sub.clone() {
                    ctx.sub = None;
                    if let Some(Choice::YesNo { yes: true }) = self.answer.take() {
                        self.free_casts.push((player, card, true, None));
                        asked_more = true;
                    }
                    declined = !asked_more;
                }
                if !declined && (asked_more || p == searcher) {
                    if p == searcher {
                        // every Wurm already chosen during this search must still be payable together with the next one
                        let chosen: Vec<CardId> = self.free_casts.iter().filter(|(pl, c, _, _)| *pl == p && self.cards[c.0 as usize].zone == Zone::Library).map(|(_, c, _, _)| *c).collect();
                        let wurm = self.players[p as usize].library.iter().rev().copied().find(|id| {
                            if chosen.contains(id) {
                                return false;
                            }
                            if !(self.db.def(self.cards[id.0 as usize].def).abilities.iter().any(|a| matches!(a, Ability::Static { effect: StaticEffect::CastFromLibraryWhileSearching, .. })) && !self.cards[id.0 as usize].cur.types.has(CardType::Land)) {
                                return false;
                            }
                            let mut total = self.effective_cost(p, *id, None);
                            for c in &chosen {
                                let extra = self.effective_cost(p, *c, None);
                                total.shards.extend(extra.shards.iter().cloned());
                            }
                            self.can_afford(p, &total, 0)
                        });
                        if let Some(card) = wurm {
                            ctx.sub = Some(Sub::SearchCast { player: p, card });
                            self.pending = Some(Decision {
                                player: p,
                                kind: DecisionKind::YesNo { prompt: format!("Cast {} from your library?", self.def_of(card).name), source: Some(source) },
                            });
                            return Exec::Wait;
                        }
                    }
                }
                if p == controller {
                    self.players[p as usize].searched_turn = true;
                }
                self.emit(Event::Searched { player: controller });
                // Aven Mindcensor: only the top N cards of the library can be found
                let skip = self.search_top_limit(if *by_owner { p } else { controller }).map_or(0, |l| self.players[p as usize].library.len().saturating_sub(l));
                let lib_basic = self.battlefield.iter().any(|&b| {
                    let bc = &self.cards[b.0 as usize];
                    if bc.cur.no_abilities || bc.cur.controller != p {
                        return false;
                    }
                    let bd = self.db.def(bc.def);
                    bd.statics.iter().any(|&i| matches!(&bd.abilities[i as usize], Ability::Static { effect: StaticEffect::AddSupertype { supertype: Supertype::Basic, .. }, from: Some(Zone::Library), .. }))
                });
                let found: Vec<CardId> = self.players[p as usize]
                    .library
                    .iter()
                    .copied()
                    .skip(skip)
                    .filter(|id| {
                        let mut f = filter.clone();
                        f.controller = PlayerFilter::Any;
                        // Rootpath Purifier: land cards in the searcher's library are basic
                        if lib_basic && self.db.def(self.cards[id.0 as usize].def).types.has(CardType::Land) {
                            f.supertypes.retain(|s| *s != Supertype::Basic);
                        }
                        self.matches_card(&f, *id, source, controller)
                    })
                    .collect();
                let mut found = found;
                if *graveyard_too {
                    let gy: Vec<CardId> = self.players[p as usize].graveyard.iter().copied().filter(|id| {
                        let mut f = filter.clone();
                        f.controller = PlayerFilter::Any;
                        self.matches_card(&f, *id, source, controller)
                    }).collect();
                    found.extend(gy);
                }
                if found.is_empty() {
                    if *shuffle {
                        self.shuffle_library(p);
                    }
                    return Exec::Done;
                }
                ctx.sub = Some(Sub::Search { player: p, found: found.clone(), to: *to, shuffle: *shuffle, tapped: *tapped });
                self.pending = Some(Decision {
                    player: self.search_controller(if *by_owner { p } else { controller }).map_or(if *by_owner { p } else { controller }, |(_, c)| c),
                    kind: DecisionKind::Pick {
                        prompt: "Search your library".into(),
                        source: Some(source),
                        options: found.iter().map(|c| Opt::Obj { obj: Obj::card(*c) }).collect(),
                        min: 0,
                        max: (k.max(0) as usize).min(found.len()) as u8,
                        cancellable: false,
                        hidden: true,
                        purpose: PickPurpose::Search,
                    },
                });
                Exec::Wait
            }
            Effect::LookAtTop { who, n, take, filter, rest, keep_on_top, to, tapped, all, must, random_rest, rest_ordered, top_ordered, rest_top_ordered, face_down, distinct, enters_with } => {
                // the cards that were not taken: left on top, or put on the bottom / in another zone
                macro_rules! finish_rest {
                    ($rest_cards:expr, $rest_zone:expr) => {{
                        let mut left: Vec<CardId> = $rest_cards;
                        if !*keep_on_top {
                            if *random_rest {
                                left.shuffle(&mut self.rng);
                            }
                            if *rest_ordered && left.len() >= 2 && $rest_zone == Zone::Library {
                                // the controller picks the order, top card of the group first
                                ctx.sub = Some(Sub::BottomOrder { cards: left.clone() });
                                self.pending = Some(Decision {
                                    player: controller,
                                    kind: DecisionKind::Pick {
                                        prompt: "Put these cards on the bottom of the library: choose the order, topmost first".into(),
                                        source: Some(source),
                                        options: left.iter().map(|c| Opt::Obj { obj: Obj::card(*c) }).collect(),
                                        min: left.len() as u8,
                                        max: left.len() as u8,
                                        cancellable: false,
                                        hidden: true,
                                        purpose: PickPurpose::Generic,
                                    },
                                });
                                left = vec![];
                            }
                            for id in left {
                                self.move_card(id, $rest_zone, MoveOpts { bottom: true, ..Default::default() });
                            }
                        }
                    }};
                }
                if let Some(Sub::BottomOrder { cards }) = ctx.sub.clone() {
                    ctx.sub = None;
                    let mut order: Vec<CardId> = match self.answer.take() {
                        Some(Choice::Pick { indices }) => indices.iter().filter_map(|i| cards.get(*i).copied()).collect(),
                        _ => vec![],
                    };
                    for c in &cards {
                        if !order.contains(c) {
                            order.push(*c);
                        }
                    }
                    for id in order {
                        self.move_card(id, Zone::Library, MoveOpts { bottom: true, ..Default::default() });
                    }
                    return Exec::Done;
                }
                if let Some(Sub::LookTopOrder { cards, left, rest }) = ctx.sub.clone() {
                    ctx.sub = None;
                    let mut order: Vec<CardId> = match self.answer.take() {
                        Some(Choice::Pick { indices }) => indices.iter().filter_map(|i| cards.get(*i).copied()).collect(),
                        _ => vec![],
                    };
                    for c in &cards {
                        if !order.contains(c) {
                            order.push(*c);
                        }
                    }
                    finish_rest!(left, rest);
                    let owner = self.cards[cards[0].0 as usize].owner;
                    let lib = &mut self.players[owner as usize].library;
                    lib.retain(|x| !cards.contains(x));
                    // the first chosen card ends up on top (the end of the vector)
                    for c in order.into_iter().rev() {
                        lib.push(c);
                    }
                    ctx.remembered = cards.into_iter().map(Obj::card).collect();
                    return if self.pending.is_some() { Exec::Wait } else { Exec::Done };
                }
                if let Some(Sub::LookAtTop { player, cards, rest, take: _, others }) = ctx.sub.clone() {
                    let ans = self.answer.take();
                    ctx.sub = None;
                    let mut picked: Vec<CardId> = match ans {
                        Some(Choice::Pick { indices }) => indices.iter().filter_map(|i| cards.get(*i).copied()).collect(),
                        _ => vec![],
                    };
                    if let Some(dk) = distinct {
                        let key = |g: &Game, id: CardId| -> String {
                            match dk {
                                ChooseDistinct::Name => g.def_of(id).name.clone(),
                                ChooseDistinct::Power => g.def_of(id).power.unwrap_or(0).to_string(),
                                ChooseDistinct::ManaValue => g.def_of(id).mana_value.to_string(),
                                _ => String::new(),
                            }
                        };
                        let mut seen: Vec<String> = vec![];
                        picked.retain(|id| {
                            let k = key(self, *id);
                            let fresh = !seen.contains(&k);
                            seen.push(k);
                            fresh
                        });
                    }
                    if *top_ordered {
                        let mut left: Vec<CardId> = others;
                        left.extend(cards.iter().copied().filter(|c| !picked.contains(c)));
                        if picked.len() >= 2 {
                            ctx.sub = Some(Sub::LookTopOrder { cards: picked.clone(), left, rest });
                            self.pending = Some(Decision {
                                player: controller,
                                kind: DecisionKind::Pick {
                                    prompt: "Put these cards on top of the library: choose the order, top card first".into(),
                                    source: Some(source),
                                    options: picked.iter().map(|c| Opt::Obj { obj: Obj::card(*c) }).collect(),
                                    min: picked.len() as u8,
                                    max: picked.len() as u8,
                                    cancellable: false,
                                    hidden: false,
                                    purpose: PickPurpose::Generic,
                                },
                            });
                            return Exec::Wait;
                        }
                        finish_rest!(left, rest);
                        ctx.remembered = picked.into_iter().map(Obj::card).collect();
                        let _ = player;
                        return if self.pending.is_some() { Exec::Wait } else { Exec::Done };
                    }
                    // an Aura can't enchant a permanent entering at the same time: hosts are judged on the battlefield before anything enters
                    let bf_before: Vec<CardId> = self.battlefield.clone();
                    for id in &picked {
                        let mut mo = MoveOpts { tapped: *tapped, controller: Some(controller), face_down: *face_down, ..Default::default() };
                        if let (Some(k), Zone::Battlefield) = (enters_with, *to) {
                            self.cards[id.0 as usize].enter_counter_rider = Some(*k);
                        }
                        if *to == Zone::Battlefield {
                            // an Aura put onto the battlefield without being cast enters attached to a legal host (the first; none: it stays put)
                            if let Some(hosts) = self.aura_hosts(*id) {
                                if let Some(&h) = hosts.iter().find(|h| bf_before.contains(h)) {
                                    mo.attach_to = Some(h);
                                }
                            }
                        }
                        self.move_card(*id, *to, mo);
                    }
                    let mut left: Vec<CardId> = others;
                    left.extend(cards.iter().copied().filter(|c| !picked.contains(c)));
                    if *rest_top_ordered {
                        ctx.remembered = picked.into_iter().map(Obj::card).collect();
                        if left.len() >= 2 {
                            ctx.sub = Some(Sub::LookTopOrder { cards: left.clone(), left: vec![], rest });
                            self.pending = Some(Decision {
                                player: controller,
                                kind: DecisionKind::Pick {
                                    prompt: "Put the rest back on top of the library: choose the order, top card first".into(),
                                    source: Some(source),
                                    options: left.iter().map(|c| Opt::Obj { obj: Obj::card(*c) }).collect(),
                                    min: left.len() as u8,
                                    max: left.len() as u8,
                                    cancellable: false,
                                    hidden: true,
                                    purpose: PickPurpose::Generic,
                                },
                            });
                            return Exec::Wait;
                        }
                        return Exec::Done;
                    }
                    finish_rest!(left, rest);
                    ctx.remembered = picked.into_iter().map(Obj::card).collect();
                    let _ = player;
                    return if self.pending.is_some() { Exec::Wait } else { Exec::Done };
                }
                let (ps, k) = {
                    let c = ec!();
                    (self.sel_players(who, &c), self.eval_amount(n, &c))
                };
                // several players and nothing to choose ("each player exiles the top card"): look at each library in turn
                if ps.len() > 1 && (*all || *take == 0) {
                    let mut all_picked: Vec<Obj> = vec![];
                    for &q in &ps {
                        let cards: Vec<CardId> = self.players[q as usize].library.iter().rev().take(k.max(0) as usize).copied().collect();
                        let eligible: Vec<CardId> = cards.iter().copied().filter(|id| self.matches_card(filter, *id, source, controller)).collect();
                        let others: Vec<CardId> = cards.iter().copied().filter(|id| !eligible.contains(id)).collect();
                        let picked: Vec<CardId> = if *take == 0 && !*all { vec![] } else { eligible.clone() };
                        for id in &picked {
                            let mut mo = MoveOpts { tapped: *tapped, controller: Some(controller), face_down: *face_down, ..Default::default() };
                        if let (Some(k), Zone::Battlefield) = (enters_with, *to) {
                            self.cards[id.0 as usize].enter_counter_rider = Some(*k);
                        }
                        // an Aura put onto the battlefield without being cast: its controller chooses what it enchants
                        let mut ask_host = false;
                        if *to == Zone::Battlefield {
                            match self.aura_host_plan(*id) {
                                AuraHostPlan::One(h) => mo.attach_to = Some(h),
                                AuraHostPlan::Choose => ask_host = true,
                                AuraHostPlan::NoHost | AuraHostPlan::NotAura => {}
                            }
                        }
                        self.move_card(*id, *to, mo);
                        if ask_host && self.cards[id.0 as usize].zone == Zone::Battlefield {
                            self.attach_pending.push(*id);
                        }
                        }
                        let mut left = others;
                        left.extend(eligible.iter().copied().filter(|c| !picked.contains(c)));
                        finish_rest!(left, *rest);
                        all_picked.extend(picked.into_iter().map(Obj::card));
                    }
                    ctx.remembered = all_picked;
                    return Exec::Done;
                }
                let Some(&p) = ps.first() else { return Exec::Done };
                let lib = &self.players[p as usize].library;
                let cards: Vec<CardId> = lib.iter().rev().take(k.max(0) as usize).copied().collect();
                let eligible: Vec<CardId> = cards.iter().copied().filter(|id| self.matches_card(filter, *id, source, controller)).collect();
                let others: Vec<CardId> = cards.iter().copied().filter(|id| !eligible.contains(id)).collect();
                // forced takes: everything matching, or exactly as many as asked when there are no more than that
                let forced = *all || (*must && eligible.len() <= *take as usize);
                if eligible.is_empty() || (*take == 0 && !*all) || forced {
                    let picked: Vec<CardId> = if eligible.is_empty() || (*take == 0 && !*all) { vec![] } else { eligible.clone() };
                    for id in picked.iter().filter(|_| !*top_ordered) {
                        let mut mo = MoveOpts { tapped: *tapped, controller: Some(controller), face_down: *face_down, ..Default::default() };
                        if let (Some(k), Zone::Battlefield) = (enters_with, *to) {
                            self.cards[id.0 as usize].enter_counter_rider = Some(*k);
                        }
                        // an Aura put onto the battlefield without being cast: its controller chooses what it enchants
                        let mut ask_host = false;
                        if *to == Zone::Battlefield {
                            match self.aura_host_plan(*id) {
                                AuraHostPlan::One(h) => mo.attach_to = Some(h),
                                AuraHostPlan::Choose => ask_host = true,
                                AuraHostPlan::NoHost | AuraHostPlan::NotAura => {}
                            }
                        }
                        self.move_card(*id, *to, mo);
                        if ask_host && self.cards[id.0 as usize].zone == Zone::Battlefield {
                            self.attach_pending.push(*id);
                        }
                    }
                    let mut left = others;
                    left.extend(eligible.iter().copied().filter(|c| !picked.contains(c)));
                    finish_rest!(left, *rest);
                    ctx.remembered = picked.into_iter().map(Obj::card).collect();
                    return if self.pending.is_some() { Exec::Wait } else { Exec::Done };
                }
                ctx.sub = Some(Sub::LookAtTop { player: p, cards: eligible.clone(), rest: *rest, take: *take, others });
                self.pending = Some(Decision {
                    player: controller,
                    kind: DecisionKind::Pick {
                        prompt: "Choose a card".into(),
                        source: Some(source),
                        options: eligible.iter().map(|c| Opt::Obj { obj: Obj::card(*c) }).collect(),
                        min: if *must { (*take as usize).min(eligible.len()) as u8 } else { 0 },
                        max: *take,
                        cancellable: false,
                        hidden: true,
                        purpose: PickPurpose::Search,
                    },
                });
                Exec::Wait
            }
            Effect::AddMana { mana, who, until_end_of_combat, until_end_of_turn, .. } => {
                // "+1/+1 for each unspent mana" style statics read the pool
                self.layers_dirty = true;
                let controller = match who {
                    Some(w) => {
                        let c = ec!();
                        self.sel_players(w, &c).first().copied().unwrap_or(controller)
                    }
                    None => controller,
                };
                // firebending: the mana that lasts until end of combat
                if *until_end_of_combat && ctx.sub.is_none() {
                    self.emit(Event::Bent { player: controller, kind: BendKind::Fire });
                }
                let mut start = 0usize;
                let mut resume: Option<(usize, i32, i32)> = None;
                if !matches!(ctx.sub, Some(Sub::ManaColor { .. })) {
                    self.mana_pick = None;
                }
                if let Some(Sub::ManaColor { idx, per, left }) = ctx.sub.clone() {
                    ctx.sub = None;
                    let other = matches!(mana.get(idx), Some(ManaProduce::RestrictedAny { other_color: true, .. })).then_some(self.mana_pick).flatten();
                    let opts: Vec<Color> = match mana.get(idx) {
                        Some(ManaProduce::AnyColorScaled { colors, .. }) if !colors.is_empty() => colors.clone(),
                        Some(ManaProduce::AnyColorScaled { colors_of: Some(of), .. }) => self.colors_of_first(of, &ec!()),
                        Some(ManaProduce::CostOf { of }) => {
                            let h = self.cost_of_parts(of, &ec!()).1;
                            h.get(h.len().saturating_sub(left as usize)).cloned().unwrap_or_else(|| Color::ALL.to_vec())
                        }
                        _ => Color::ALL.iter().copied().filter(|c| Some(*c) != other).collect(),
                    };
                    let col = match self.answer.take() {
                        Some(Choice::Pick { indices }) => opts.get(indices.first().copied().unwrap_or(0)).copied().unwrap_or(opts[0]),
                        _ => opts[0],
                    };
                    if matches!(mana.get(idx), Some(ManaProduce::RestrictedAny { .. })) {
                        self.mana_pick = Some(col);
                    }
                    if let (Some(ManaProduce::RestrictedAny { spell, ability, .. }), false) = (mana.get(idx), self.paying_activation.get()) {
                        let r = ManaRestrict { spell: spell.clone(), ability: ability.clone(), rider: None, cost_c: false, upkeep: false, unlock: false, no_generic: false, rider_spell: None };
                        for _ in 0..per.max(0) {
                            let pl = &mut self.players[controller as usize];
                            pl.restricted.push(Some((ManaType::of(col), r.clone(), source)));
                            pl.restricted_turn.resize(pl.restricted.len() - 1, false);
                            pl.restricted_turn.push(*until_end_of_turn);
                        }
                    } else {
                    self.players[controller as usize].pool.add(ManaType::of(col), per.max(0) as u16);
                    if *until_end_of_turn {
                        self.players[controller as usize].pool_turn.add(ManaType::of(col), per.max(0) as u16);
                    }
                    if *until_end_of_combat {
                        self.players[controller as usize].pool_keep.add(ManaType::of(col), per.max(0) as u16);
                    }
                    }
                    if left > 1 {
                        resume = Some((idx, per, left - 1));
                    } else {
                        start = idx + 1;
                    }
                }
                let mut i = start;
                loop {
                    let (idx, per, left) = if let Some(r) = resume.take() {
                        r
                    } else {
                        let Some(m) = mana.get(i) else { break };
                        let idx = i;
                        i += 1;
                        match m {
                            ManaProduce::OneProduced => {
                                if let Some(t) = self.last_tapped_mana.first().copied() {
                                    self.players[controller as usize].pool.add(t, 1);
                                    if *until_end_of_turn {
                                        self.players[controller as usize].pool_turn.add(t, 1);
                                    }
                                    if *until_end_of_combat {
                                        self.players[controller as usize].pool_keep.add(t, 1);
                                    }
                                }
                                continue;
                            }
                            ManaProduce::MilledSymbols => {
                                // the ability resolves from the stack (Charmed Pendant): one mana per coloured symbol of the milled card (hybrid: its first colour)
                                if let Some(mid) = self.cost_milled.last().copied() {
                                    let shards: Vec<Shard> = self.def_of(mid).cost.shards.iter().cloned().collect();
                                    for sh in shards {
                                        let t = match sh {
                                            Shard::Color { c } | Shard::Phyrexian { c } | Shard::HybridGeneric { c } | Shard::HybridColorless { c } => ManaType::of(c),
                                            Shard::Hybrid { a, .. } | Shard::PhyrexianHybrid { a, .. } => ManaType::of(a),
                                            _ => continue,
                                        };
                                        self.players[controller as usize].pool.add(t, 1);
                                        if *until_end_of_turn {
                                            self.players[controller as usize].pool_turn.add(t, 1);
                                        }
                                    }
                                }
                                continue;
                            }
                            ManaProduce::Fixed { m, n } => {
                                self.players[controller as usize].pool.add(*m, *n as u16);
                                if *until_end_of_combat {
                                    self.players[controller as usize].pool_keep.add(*m, *n as u16);
                                }
                                if *until_end_of_turn {
                                    self.players[controller as usize].pool_turn.add(*m, *n as u16);
                                }
                                continue;
                            }
                            ManaProduce::Restricted { m, n, spell, ability, amount, rider, cost_c, upkeep, unlock, chosen, no_generic } => {
                                let m = &if *chosen { self.cards[source.0 as usize].chosen_color.map_or(*m, ManaType::of) } else { *m };
                                let n: u32 = match amount {
                                    Some(a) => {
                                        let c = ec!();
                                        self.eval_amount(a, &c).max(0) as u32
                                    }
                                    None => *n as u32,
                                };
                                if self.paying_activation.get() {
                                    // tapped to pay for something that fits the restriction: plain mana for the payment
                                    self.players[controller as usize].pool.add(*m, n as u16);
                                } else {
                                    let r = ManaRestrict { spell: spell.clone(), ability: ability.clone(), rider: *rider, cost_c: *cost_c, upkeep: *upkeep, unlock: *unlock, no_generic: *no_generic, rider_spell: None };
                                    for _ in 0..n {
                                        { let pl = &mut self.players[controller as usize]; pl.restricted.push(Some((*m, r.clone(), source))); pl.restricted_turn.resize(pl.restricted.len() - 1, false); pl.restricted_turn.push(*until_end_of_turn); }
                                    }
                                }
                                continue;
                            }
                            ManaProduce::Scaled { m, amount } => {
                                let n = {
                                    let c = ec!();
                                    self.eval_amount(amount, &c)
                                };
                                self.players[controller as usize].pool.add(*m, n.max(0) as u16);
                                if *until_end_of_combat {
                                    self.players[controller as usize].pool_keep.add(*m, n.max(0) as u16);
                                }
                                if *until_end_of_turn {
                                    self.players[controller as usize].pool_turn.add(*m, n.max(0) as u16);
                                }
                                continue;
                            }
                            ManaProduce::CostOf { of } => {
                                let (fixed, hybrids) = self.cost_of_parts(of, &ec!());
                                for (t, n) in fixed {
                                    self.players[controller as usize].pool.add(t, n);
                                    if *until_end_of_combat {
                                        self.players[controller as usize].pool_keep.add(t, n);
                                    }
                                    if *until_end_of_turn {
                                        self.players[controller as usize].pool_turn.add(t, n);
                                    }
                                }
                                if hybrids.is_empty() {
                                    continue;
                                }
                                (idx, 1, hybrids.len() as i32)
                            }
                            ManaProduce::NotedMana { .. } => {
                                let all = self.cards[source.0 as usize].noted_mana_all;
                                for (i, t) in ManaType::ALL.iter().enumerate() {
                                    if all[i] > 0 {
                                        self.players[controller as usize].pool.add(*t, all[i] as u16);
                                    }
                                }
                                continue;
                            }
                            ManaProduce::NotedType => {
                                let k = self.cards[source.0 as usize].noted_mana;
                                if k > 0 {
                                    self.players[controller as usize].pool.add(ManaType::ALL[k as usize - 1], 1);
                                }
                                continue;
                            }
                            ManaProduce::ChosenColor { n } => {
                                if let Some(c) = self.cards[source.0 as usize].chosen_color {
                                    self.players[controller as usize].pool.add(ManaType::of(c), *n as u16);
                                }
                                continue;
                            }
                            ManaProduce::SecondChosenColor { n } => {
                                if let Some(c) = self.cards[source.0 as usize].chosen_color2 {
                                    self.players[controller as usize].pool.add(ManaType::of(c), *n as u16);
                                }
                                continue;
                            }
                            ManaProduce::ChosenColorScaled { amount } => {
                                let n = {
                                    let c = ec!();
                                    self.eval_amount(amount, &c)
                                };
                                if let Some(c) = self.cards[source.0 as usize].chosen_color {
                                    self.players[controller as usize].pool.add(ManaType::of(c), n.max(0) as u16);
                                }
                                continue;
                            }
                            ManaProduce::AnyColor { n } => (idx, 1, *n as i32),
                            ManaProduce::RestrictedAny { n, one_color, amount, .. } => {
                                let n = match amount {
                                    Some(a) => {
                                        let c = ec!();
                                        self.eval_amount(a, &c)
                                    }
                                    None => *n as i32,
                                };
                                if n <= 0 {
                                    continue;
                                }
                                if *one_color { (idx, n, 1) } else { (idx, 1, n) }
                            }
                            ManaProduce::AnyColorScaled { amount, .. } => {
                                let n = {
                                    let c = ec!();
                                    self.eval_amount(amount, &c)
                                };
                                if n <= 0 {
                                    continue;
                                }
                                (idx, 1, n)
                            }
                            ManaProduce::AnyOneColor { n } => (idx, *n as i32, 1),
                            ManaProduce::AnyOneColorScaled { amount } => {
                                let n = {
                                    let c = ec!();
                                    self.eval_amount(amount, &c)
                                };
                                if n <= 0 {
                                    continue;
                                }
                                (idx, n, 1)
                            }
                            _ => (idx, 1, 1),
                        }
                    };
                    ctx.sub = Some(Sub::ManaColor { idx, per, left });
                    self.pending = Some(Decision {
                        player: controller,
                        kind: DecisionKind::Pick {
                            prompt: format!("{}: choose a color of mana", self.def_of(source).name),
                            source: Some(source),
                            options: match mana.get(idx) {
                                Some(ManaProduce::AnyColorScaled { colors, .. }) if !colors.is_empty() => colors.iter().map(|c| Opt::Color { c: *c }).collect(),
                                Some(ManaProduce::AnyColorScaled { colors_of: Some(of), .. }) => self.colors_of_first(of, &ec!()).into_iter().map(|c| Opt::Color { c }).collect(),
                                Some(ManaProduce::CostOf { of }) => {
                                    let h = self.cost_of_parts(of, &ec!()).1;
                                    h.get(h.len().saturating_sub(left as usize)).cloned().unwrap_or_else(|| Color::ALL.to_vec()).into_iter().map(|c| Opt::Color { c }).collect()
                                }
                                Some(ManaProduce::RestrictedAny { other_color: true, .. }) => Color::ALL.iter().filter(|c| Some(**c) != self.mana_pick).map(|c| Opt::Color { c: *c }).collect(),
                                _ => Color::ALL.iter().map(|c| Opt::Color { c: *c }).collect(),
                            },
                            min: 1,
                            max: 1,
                            cancellable: false,
                            hidden: false,
                            purpose: PickPurpose::Generic,
                        },
                    });
                    return Exec::Wait;
                }
                Exec::Done
            }
            Effect::GainControl { what, duration, to, as_source } => {
                let (cs, newc, own) = {
                    let c = ec!();
                    (self.sel_cards(what, &c), to.as_ref().and_then(|t| self.sel_players(t, &c).first().copied()), as_source.as_ref().and_then(|t| self.sel_cards(t, &c).first().copied()))
                };
                let source = own.unwrap_or(source);
                let prev: Vec<PlayerId> = cs.first().map(|c| vec![self.cards[c.0 as usize].cur.controller]).unwrap_or_default();
                // "for as long as you control <source>": if the source already left the battlefield, the duration is over before it begins
                if (*duration == Duration::WhileSourceControlled || *duration == Duration::WhileSourceOnBattlefield) && !self.battlefield.contains(&source) {
                    return Exec::Done;
                }
                // a spell on the stack changes controller at once (Perplexing Chimera): the stack item and the card follow
                let (stack_cs, cs): (Vec<CardId>, Vec<CardId>) = cs.into_iter().partition(|c| self.cards[c.0 as usize].zone == Zone::Stack);
                for id in stack_cs {
                    let np = newc.unwrap_or(controller);
                    for it in self.stack.iter_mut().filter(|s| s.source == id && matches!(s.kind, ItemKind::Spell { .. })) {
                        it.controller = np;
                    }
                    self.cards[id.0 as usize].cur.controller = np;
                    self.cards[id.0 as usize].controller = np;
                }
                if cs.is_empty() {
                    return Exec::Done;
                }
                self.add_effect(source, controller, &cs, Modif::Control(newc.unwrap_or(controller)), *duration);
                if *duration == Duration::WhilePrevControllerMonarch {
                    if let Some(e) = self.effects.last_mut() {
                        e.players = prev.into_iter().collect();
                    }
                }
                Exec::Done
            }
            Effect::RotateControl { filter, right, chosen } => {
                let mut plan: Vec<(CardId, PlayerId)> = vec![];
                if *chosen {
                    let mut queue: Vec<PlayerId>;
                    if let Some(Sub::RotatePick { options, queue: q, plan: pl, who }) = ctx.sub.clone() {
                        ctx.sub = None;
                        plan = pl;
                        queue = q;
                        if let Some(Choice::Pick { indices }) = self.answer.take() {
                            if let Some(id) = indices.first().and_then(|i| options.get(*i)) {
                                plan.push((*id, who));
                            }
                        }
                    } else {
                        queue = self.alive_players();
                    }
                    while !queue.is_empty() {
                        let p = queue.remove(0);
                        let q = if *right { self.previous_player(p) } else { self.next_player(p) };
                        if q == p {
                            continue;
                        }
                        let options: Vec<CardId> = self.battlefield.iter().copied().filter(|id| self.cards[id.0 as usize].cur.controller == q && self.matches_card(&Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() }, *id, source, controller)).collect();
                        if options.len() == 1 {
                            plan.push((options[0], p));
                        } else if options.len() > 1 {
                            self.pending = Some(Decision {
                                player: controller,
                                kind: DecisionKind::Pick {
                                    prompt: format!("{}: choose a permanent for {}", self.def_of(source).name, p),
                                    source: Some(source),
                                    options: options.iter().map(|c| Opt::Obj { obj: Obj::card(*c) }).collect(),
                                    min: 1,
                                    max: 1,
                                    cancellable: false,
                                    hidden: false,
                                    purpose: PickPurpose::Generic,
                                },
                            });
                            ctx.sub = Some(Sub::RotatePick { options, queue, plan, who: p });
                            return Exec::Wait;
                        }
                    }
                    for (id, p) in plan {
                        self.add_effect(source, controller, &[id], Modif::Control(p), Duration::Forever);
                    }
                    return Exec::Done;
                }
                for p in self.alive_players() {
                    let q = if *right { self.previous_player(p) } else { self.next_player(p) };
                    if q == p {
                        continue;
                    }
                    for id in self.battlefield.clone() {
                        if self.cards[id.0 as usize].cur.controller == q && self.matches_card(&Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() }, id, source, controller) {
                            plan.push((id, p));
                        }
                    }
                }
                for (id, p) in plan {
                    self.add_effect(source, controller, &[id], Modif::Control(p), Duration::Forever);
                }
                Exec::Done
            }
            Effect::ExchangeControl { a, b, duration, destroy_auras } => {
                let (xs, ys) = {
                    let c = ec!();
                    (self.sel_cards(a, &c), self.sel_cards(b, &c))
                };
                if let (Some(&x), Some(&y)) = (xs.first(), ys.first()) {
                    let (cx, cy) = (self.cards[x.0 as usize].cur.controller, self.cards[y.0 as usize].cur.controller);
                    // both permanents must still be on the battlefield: with one gone, nothing is exchanged
                    if cx != cy && self.cards[x.0 as usize].zone == Zone::Battlefield && self.cards[y.0 as usize].zone == Zone::Battlefield {
                        self.add_effect(source, controller, &[x], Modif::Control(cy), *duration);
                        self.add_effect(source, controller, &[y], Modif::Control(cx), *duration);
                        if *destroy_auras {
                            let aura = Sym::new("Aura");
                            let auras: Vec<CardId> = self.battlefield.iter().copied().filter(|b| {
                                let bc = &self.cards[b.0 as usize];
                                bc.attached_to.map_or(false, |h| h == x || h == y) && bc.cur.subtypes.contains(&aura)
                            }).collect();
                            for a in auras {
                                self.destroy(a);
                            }
                        }
                    }
                }
                Exec::Done
            }
            Effect::Fight { a, b } => {
                let (xs, ys) = {
                    let c = ec!();
                    (self.sel_cards(a, &c), self.sel_cards(b, &c))
                };
                let live = |id: CardId| { let c = &self.cards[id.0 as usize]; c.zone == Zone::Battlefield && c.cur.types.has(CardType::Creature) };
                if let (Some(&x), Some(&y)) = (xs.first(), ys.first()) {
                  if live(x) && live(y) {
                    let (px, py) = (self.power_of(x), self.power_of(y));
                    self.cards[x.0 as usize].fought_turn = self.turn;
                    self.cards[y.0 as usize].fought_turn = self.turn;
                    self.emit(Event::KeywordAction { card: x, action: KeywordAction::Fights, other: None });
                    self.emit(Event::KeywordAction { card: y, action: KeywordAction::Fights, other: None });
                    self.deal_damage(Some(x), Obj::card(y), px, false);
                    self.deal_damage(Some(y), Obj::card(x), py, false);
                  }
                }
                Exec::Done
            }
            Effect::FightEach { a, b } => {
                let (xs, ys) = {
                    let c = ec!();
                    (self.sel_cards(a, &c), self.sel_cards(b, &c))
                };
                let pairs: Vec<(CardId, CardId, i32, i32)> = xs.iter().zip(ys.iter()).map(|(&x, &y)| (x, y, self.power_of(x), self.power_of(y))).collect();
                for &(x, y, _, _) in &pairs {
                    self.cards[x.0 as usize].fought_turn = self.turn;
                    self.cards[y.0 as usize].fought_turn = self.turn;
                    self.emit(Event::KeywordAction { card: x, action: KeywordAction::Fights, other: None });
                    self.emit(Event::KeywordAction { card: y, action: KeywordAction::Fights, other: None });
                }
                for &(x, y, px, py) in &pairs {
                    self.deal_damage(Some(x), Obj::card(y), px, false);
                    self.deal_damage(Some(y), Obj::card(x), py, false);
                }
                Exec::Done
            }
            Effect::Bite { a, b } => {
                let (xs, ys) = {
                    let c = ec!();
                    (self.sel_cards(a, &c), self.sel_cards(b, &c))
                };
                if let (Some(&x), Some(&y)) = (xs.first(), ys.first()) {
                    let px = self.power_of(x);
                    self.deal_damage(Some(x), Obj::card(y), px, false);
                }
                Exec::Done
            }
            Effect::SetSubtypesOfKind { what, kind, subtypes, chosen, duration } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                if !cs.is_empty() {
                    let mut subs: Vec<Sym> = subtypes.iter().map(|s| Sym::new(s)).collect();
                    if *chosen {
                        if let Some(t) = self.cards[source.0 as usize].chosen_type {
                            subs.push(t);
                        }
                    }
                    self.add_effect(source, controller, &cs, Modif::SetSubtypesKind(*kind, subs), *duration);
                }
                Exec::Done
            }
            Effect::Animate { what, add_types, remove_types, add_subtypes, set_subtypes, set_chosen_subtype, add_chosen_subtype, set_chosen_color, power, toughness, pt_amount, colors, add_colors, keywords, add_supertypes, remove_supertypes, set_name, set_name_chosen_subtype, duration } => {
                let pt_now: Option<i32> = pt_amount.as_ref().map(|a| {
                    let c = ec!();
                    self.eval_amount(a, &c).max(0)
                });
                let (power, toughness) = if pt_now.is_some() { (&pt_now, &pt_now) } else { (power, toughness) };
                let cs = {
                    let c = ec!();
                    let mut cs = self.sel_cards(what, &c);
                    // a spell on the stack changes colour too (Lifelace, Thoughtlace)
                    if colors.is_some() {
                        for o in self.sel_objs(what, &c) {
                            if let Obj::Stack { id } = o {
                                if let Some(it) = self.stack.iter().find(|s| s.id == id) {
                                    cs.push(it.source);
                                }
                            }
                        }
                    }
                    cs
                };
                for id in cs {
                    if !add_types.is_empty() {
                        self.add_effect(source, controller, &[id], Modif::AddTypes(TypeSet::of(add_types)), *duration);
                    }
                    if !remove_types.is_empty() {
                        self.add_effect(source, controller, &[id], Modif::RemoveTypes(TypeSet::of(remove_types)), *duration);
                    }
                    for st in add_subtypes {
                        self.add_effect(source, controller, &[id], Modif::AddSubtype(Sym::new(st)), *duration);
                    }
                    if !set_subtypes.is_empty() {
                        self.add_effect(source, controller, &[id], Modif::SetSubtypes(set_subtypes.iter().map(|s| Sym::new(s)).collect()), *duration);
                    }
                    if *set_chosen_subtype {
                        if let Some(t) = self.cards[source.0 as usize].chosen_type {
                            self.add_effect(source, controller, &[id], Modif::SetSubtypes(vec![t]), *duration);
                        }
                    }
                    if *add_chosen_subtype {
                        if let Some(t) = self.cards[source.0 as usize].chosen_type {
                            self.add_effect(source, controller, &[id], Modif::AddSubtype(t), *duration);
                        }
                    }
                    if *set_chosen_color {
                        if let Some(col) = self.cards[source.0 as usize].chosen_color {
                            self.add_effect(source, controller, &[id], Modif::Colors(ColorSet::of(&[col])), *duration);
                        }
                    }
                    if let (Some(p), Some(t)) = (power, toughness) {
                        self.add_effect(source, controller, &[id], Modif::SetPT { power: *p, toughness: *t }, *duration);
                    }
                    if let Some(cols) = colors {
                        self.add_effect(source, controller, &[id], Modif::Colors(ColorSet::of(cols)), *duration);
                    }
                    if !add_colors.is_empty() {
                        self.add_effect(source, controller, &[id], Modif::AddColors(ColorSet::of(add_colors)), *duration);
                    }
                    for k in keywords {
                        self.add_effect(source, controller, &[id], Modif::Keyword(*k), *duration);
                    }
                    for st in add_supertypes {
                        self.add_effect(source, controller, &[id], Modif::AddSupertype(*st), *duration);
                    }
                    for st in remove_supertypes {
                        self.add_effect(source, controller, &[id], Modif::RemoveSupertype(*st), *duration);
                    }
                    if let Some(n) = set_name {
                        self.add_effect(source, controller, &[id], Modif::SetName(Sym::new(n)), *duration);
                    }
                    if *set_name_chosen_subtype {
                        if let Some(t) = self.cards[source.0 as usize].chosen_type {
                            self.add_effect(source, controller, &[id], Modif::SetName(t), *duration);
                        }
                    }
                }
                Exec::Done
            }
            Effect::LoseAllAbilities { what, duration } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                if !cs.is_empty() {
                    self.add_effect(source, controller, &cs, Modif::NoAbilities, *duration);
                }
                Exec::Done
            }
            Effect::Transform { what } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                let delayed = matches!(ctx.item.kind, ItemKind::Delayed { .. });
                for id in cs {
                    // 701.28f: a delayed transform does nothing if the permanent already transformed this step
                    if delayed && self.last_transform.get(&id) == Some(&(self.turn, self.step)) {
                        continue;
                    }
                    // 701.28f: an activated or triggered ability put on the stack before the permanent last transformed does nothing
                    if matches!(ctx.item.kind, ItemKind::Ability { .. } | ItemKind::Triggered { .. }) && self.last_transform_stack.get(&id).map_or(false, |&n| ctx.item.id.0 <= n) {
                        continue;
                    }
                    self.transform(id);
                    self.last_transform_stack.insert(id, self.next_stack_id);
                    self.last_transform.insert(id, (self.turn, self.step));
                }
                Exec::Done
            }
            Effect::ControlPlayerNextTurn { who, by, combat_only } => {
                let (ps, bs) = {
                    let c = ec!();
                    (self.sel_players(who, &c), by.as_ref().map(|b| self.sel_players(b, &c)))
                };
                if let Some(&v) = ps.first() {
                    let by = bs.and_then(|b| b.first().copied()).unwrap_or(controller);
                    self.next_turn_ctl.retain(|(x, _, _)| *x != v);
                    self.next_turn_ctl.push((v, by, *combat_only));
                }
                Exec::Done
            }
            Effect::Attach { what, to } => {
                let (xs, ys) = {
                    let c = ec!();
                    (self.sel_cards(what, &c), self.sel_cards(to, &c))
                };
                // every selected Aura / Equipment goes onto the first selected host ("attached to enchanted creature")
                if let Some(&y) = ys.first() {
                    for x in xs {
                        self.attach(x, Some(y));
                    }
                }
                Exec::Done
            }
            Effect::Unattach { what } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                for id in cs {
                    self.attach(id, None);
                }
                Exec::Done
            }
            Effect::ExtraTurn { who, lose_at_end } => {
                let ps = {
                    let c = ec!();
                    self.sel_players(who, &c)
                };
                for p in ps {
                    if *lose_at_end {
                        self.extra_lose_marks.push(self.extra_turns.len());
                    }
                    self.extra_turns.push(p);
                }
                Exec::Done
            }
            Effect::ExileCompareExtraTurn => {
                let mut cand = self.apnap();
                let mut guard = 0;
                while !cand.is_empty() && guard < 200 {
                    guard += 1;
                    let mut best: i64 = -1;
                    let mut tied: Vec<PlayerId> = vec![];
                    for &p in &cand {
                        if let Some(&id) = self.players[p as usize].library.last() {
                            self.cur_src = Some(source);
                            self.move_card(id, Zone::Exile, MoveOpts::default());
                            let mv = self.def_of(id).mana_value as i64;
                            if mv > best {
                                best = mv;
                                tied = vec![p];
                            } else if mv == best {
                                tied.push(p);
                            }
                        }
                    }
                    if tied.len() == 1 {
                        self.extra_turns.push(tied[0]);
                        break;
                    }
                    cand = tied;
                }
                Exec::Done
            }
            Effect::SkipNextTurn { who } => {
                let ps = {
                    let c = ec!();
                    self.sel_players(who, &c)
                };
                for p in ps {
                    self.players[p as usize].skip_turns += 1;
                }
                Exec::Done
            }
            Effect::SkipNextCombat { who, this_turn } => {
                let ps = {
                    let c = ec!();
                    self.sel_players(who, &c)
                };
                for p in ps {
                    if *this_turn && !(p == self.active && matches!(self.step, Step::Untap | Step::Upkeep | Step::Draw | Step::Main1)) {
                        continue;
                    }
                    if !*this_turn && p == self.active {
                        self.players[p as usize].skip_combat_after = self.turn;
                    }
                    self.players[p as usize].skip_combat += 1;
                }
                Exec::Done
            }
            Effect::NoteResolution => {
                let t = self.turn;
                let r = &mut self.cards[source.0 as usize].res_note;
                if r.0 != t {
                    *r = (t, 0);
                }
                r.1 = r.1.saturating_add(1);
                Exec::Done
            }
            Effect::SkipPhaseThisTurn { who, phase } => {
                let ps = {
                    let c = ec!();
                    self.sel_players(who, &c)
                };
                let bit = match phase {
                    SkipPhase::DrawStep => 1u8,
                    SkipPhase::MainPhases => 2,
                    SkipPhase::CombatPhases => 4,
                };
                for p in ps {
                    let t = self.turn;
                    let sp = &mut self.players[p as usize].skip_phases;
                    if sp.0 != t {
                        *sp = (t, 0);
                    }
                    sp.1 |= bit;
                }
                Exec::Done
            }
            Effect::LandsDontUntapNext => {
                let who = ec!().controller;
                self.players[who as usize].lands_no_untap = true;
                Exec::Done
            }
            Effect::CreaturesDontUntapNext { who } => {
                let ps = {
                    let c = ec!();
                    self.sel_players(who, &c)
                };
                for p in ps {
                    self.players[p as usize].creatures_no_untap = true;
                }
                Exec::Done
            }
            Effect::PreventDamageToYouUntilNextTurn => {
                let who = ec!().controller;
                self.players[who as usize].damage_shield = true;
                Exec::Done
            }
            Effect::SkipNextDrawStep { who } => {
                let ps = {
                    let c = ec!();
                    self.sel_players(who, &c)
                };
                for p in ps {
                    self.players[p as usize].skip_draw += 1;
                }
                Exec::Done
            }
            Effect::SkipNextUntapStep { who } => {
                let ps = {
                    let c = ec!();
                    self.sel_players(who, &c)
                };
                for p in ps {
                    self.players[p as usize].skip_untap += 1;
                }
                Exec::Done
            }
            Effect::MustBlockCreature { blocker, attacker, duration } => {
                let (bs, atk) = {
                    let c = ec!();
                    (self.sel_cards(blocker, &c), self.sel_cards(attacker, &c).first().copied())
                };
                if let Some(a) = atk {
                    self.add_effect(source, controller, &bs, Modif::MustBlockCard(a.0), *duration);
                }
                Exec::Done
            }
            Effect::GainControlOfSpell { what } => {
                let sid: Option<StackId> = {
                    let c = ec!();
                    self.sel_objs(what, &c).into_iter().find_map(|o| match o {
                        Obj::Stack { id } => Some(id),
                        Obj::Card { id } if self.cards[id.0 as usize].zone == Zone::Stack => self.stack.iter().rev().find(|s| s.source == id && !s.copy && matches!(s.kind, ItemKind::Spell { .. })).map(|s| s.id),
                        _ => None,
                    })
                };
                if let Some(item) = sid.and_then(|sid| self.stack.iter_mut().find(|s| s.id == sid)) {
                    item.controller = controller;
                    let src = item.source;
                    self.cards[src.0 as usize].cur.controller = controller;
                }
                Exec::Done
            }
            Effect::ExchangeFromHand { with } => {
                let other = {
                    let c = ec!();
                    self.sel_cards(with, &c).first().copied()
                };
                let me = source;
                let p = controller;
                if let Some(other) = other.filter(|o| *o != me && self.cards[me.0 as usize].zone == Zone::Hand) {
                    match self.cards[other.0 as usize].zone {
                        Zone::Battlefield => {
                            let old_zcc = self.cards[other.0 as usize].zcc;
                            self.move_card(other, Zone::Hand, Default::default());
                            self.move_card(me, Zone::Battlefield, Default::default());
                            let new_zcc = self.cards[me.0 as usize].zcc;
                            for item in self.stack.iter_mut() {
                                for t in item.targets.iter_mut() {
                                    if t.obj == Obj::card(other) && t.zcc == old_zcc {
                                        t.obj = Obj::card(me);
                                        t.zcc = new_zcc;
                                    }
                                }
                            }
                        }
                        Zone::Stack => {
                            let sid = self.stack.iter().rev().find(|s| s.source == other && !s.copy && matches!(s.kind, ItemKind::Spell { .. })).map(|s| s.id);
                            if let Some(sid) = sid {
                                self.remove_from_zone_lists(me);
                                {
                                    let c = &mut self.cards[me.0 as usize];
                                    c.zone = Zone::Stack;
                                    c.zcc += 1;
                                    c.cast_from = Some(Zone::Hand);
                                    c.controller = p;
                                }
                                self.reset_derived(me);
                                self.cards[me.0 as usize].cur.controller = p;
                                let (zcc, def) = (self.cards[me.0 as usize].zcc, self.cards[me.0 as usize].def);
                                if let Some(item) = self.stack.iter_mut().find(|s| s.id == sid) {
                                    item.source = me;
                                    item.src_zcc = zcc;
                                    item.def = def;
                                    item.targets.clear();
                                    item.spec_counts.clear();
                                    item.modes.clear();
                                    item.divisions.clear();
                                    item.x = 0;
                                    item.kind = ItemKind::Spell { from: Zone::Hand, kicked: false, alt: None };
                                }
                                let old_zcc = self.cards[other.0 as usize].zcc;
                                self.move_card(other, Zone::Hand, Default::default());
                                for item in self.stack.iter_mut() {
                                    for t in item.targets.iter_mut() {
                                        if t.obj == Obj::card(other) && t.zcc == old_zcc {
                                            t.obj = Obj::card(me);
                                            t.zcc = zcc;
                                        }
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
                Exec::Done
            }
            Effect::ExchangeSpellControl { spell, permanent } => {
                let (sid, perm) = {
                    let c = ec!();
                    let sid = self.sel_objs(spell, &c).into_iter().find_map(|o| match o {
                        Obj::Stack { id } => Some(id),
                        Obj::Card { id } if self.cards[id.0 as usize].zone == Zone::Stack => self.stack.iter().rev().find(|s| s.source == id && !s.copy && matches!(s.kind, ItemKind::Spell { .. })).map(|s| s.id),
                        _ => None,
                    });
                    (sid, self.sel_cards(permanent, &c).first().copied())
                };
                if let (Some(sid), Some(perm)) = (sid, perm) {
                    if self.cards[perm.0 as usize].zone == Zone::Battlefield {
                        if let Some(item) = self.stack.iter_mut().find(|s| s.id == sid) {
                            let cs = item.controller;
                            let cp = self.cards[perm.0 as usize].cur.controller;
                            if cs != cp {
                                item.controller = cp;
                                let src = item.source;
                                let copy = item.copy;
                                if !copy {
                                    self.cards[src.0 as usize].cur.controller = cp;
                                    self.cards[src.0 as usize].controller = cp;
                                }
                                self.add_effect(source, controller, &[perm], Modif::Control(cs), Duration::Forever);
                            }
                        }
                    }
                }
                Exec::Done
            }
            Effect::ChangeTargets { what, must_change, to, players_only, new_filter, each_stack, forced, random, by, exclude_yours } => {
                let controller = match by {
                    Some(sel) => {
                        let who = {
                            let c = ec!();
                            self.sel_players(sel, &c).first().copied()
                        };
                        match who {
                            Some(p) => p,
                            None => return Exec::Done,
                        }
                    }
                    None => controller,
                };
                self.retarget_must_change.set(*must_change);
                self.retarget_random.set(*random);
                self.retarget_exclude.set(*exclude_yours);
                self.retarget_players_only.set(*players_only);
                *self.retarget_filter.borrow_mut() = new_filter.clone();
                let only = to.as_ref().and_then(|t| {
                    let c = ec!();
                    self.sel_objs(t, &c).first().copied()
                });
                self.retarget_only.set(only);
                self.retarget_forced.set(*forced && only.is_some());
                if let Some(Sub::Retarget { sid, idx, options }) = ctx.sub.clone() {
                    ctx.sub = None;
                    if let Some(Choice::Pick { indices }) = self.answer.take() {
                        if let Some(o) = indices.first().and_then(|i| options.get(*i)) {
                            let mut became: Option<(Obj, CardId, PlayerId, bool, StackId)> = None;
                            if let Some(item) = self.stack.iter_mut().find(|s| s.id == sid) {
                                if let Some(t) = item.targets.get_mut(idx) {
                                    let changed = t.obj != *o;
                                    t.obj = *o;
                                    t.zcc = match o {
                                        Obj::Card { id } => self.cards[id.0 as usize].zcc,
                                        _ => 0,
                                    };
                                    if changed {
                                        became = Some((*o, item.source, item.controller, matches!(item.kind, ItemKind::Spell { .. }), item.id));
                                    }
                                }
                            }
                            // the new target "becomes the target" of the spell or ability (Emberheart Challenger's valiant)
                            if let Some((obj, by, ctl, spell, sid2)) = became {
                                match obj {
                                    Obj::Card { id } => self.emit(Event::Targeted { card: id, by, controller: ctl, spell, sid: sid2 }),
                                    Obj::Player { id } => self.emit(Event::PlayerTargeted { player: id, by, controller: ctl, spell, sid: sid2 }),
                                    _ => {}
                                }
                            }
                        }
                    }
                    return self.retarget_next(ctx, sid, idx + 1, source, controller);
                }
                if *each_stack {
                    let own = ctx.item.id;
                    // stack order is bottom first, so popping from the end handles the top item first
                    let mut ids: Vec<StackId> = self.stack.iter().filter(|s| s.id != own && !s.targets.is_empty()).map(|s| s.id).collect();
                    let first = ids.pop();
                    *self.retarget_queue.borrow_mut() = ids;
                    return match first {
                        Some(sid) => self.retarget_next(ctx, sid, 0, source, controller),
                        None => Exec::Done,
                    };
                }
                self.retarget_queue.borrow_mut().clear();
                let remembered_stack = if matches!(what, Sel::Remembered) { ctx.remembered.iter().find_map(|o| if let Obj::Stack { id } = o { Some(*id) } else { None }) } else { None };
                let sid: Option<StackId> = if remembered_stack.is_some() {
                    remembered_stack
                } else {
                    let c = ec!();
                    self.sel_objs(what, &c).into_iter().find_map(|o| match o {
                        Obj::Stack { id } => Some(id),
                        Obj::Card { id } if self.cards[id.0 as usize].zone == Zone::Stack => self.stack.iter().rev().find(|s| s.source == id && !s.copy && matches!(s.kind, ItemKind::Spell { .. })).map(|s| s.id),
                        _ => None,
                    })
                };
                match sid {
                    Some(sid) => self.retarget_next(ctx, sid, 0, source, controller),
                    None => Exec::Done,
                }
            }
            Effect::BecomeBlocked { what } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                for id in cs {
                    let mut newly = false;
                    if let Some(info) = self.combat.as_mut().and_then(|cb| cb.attackers.iter_mut().find(|x| x.card == id)) {
                        newly = !info.blocked;
                        info.blocked = true;
                    }
                    if newly {
                        self.emit(Event::BecameBlocked { attacker: id });
                    }
                }
                Exec::Done
            }
            Effect::EnterBlocking { what, attacker } => {
                let (cs, atk) = {
                    let c = ec!();
                    (self.sel_cards(what, &c), self.sel_cards(attacker, &c).first().copied())
                };
                if let Some(a) = atk {
                    for id in cs {
                        if self.cards[id.0 as usize].zone != Zone::Battlefield {
                            continue;
                        }
                        if let Some(info) = self.combat.as_mut().and_then(|cb| cb.attackers.iter_mut().find(|x| x.card == a)) {
                            if !info.blockers.contains(&id) {
                                info.blocked = true;
                                info.blockers.push(id);
                            }
                        }
                    }
                }
                Exec::Done
            }
            Effect::BecomeAttacking { what, defender_among } => {
                let mut chosen: Option<Obj> = None;
                if let Some(Sub::DefenderPick { options }) = ctx.sub.clone() {
                    ctx.sub = None;
                    let i = match self.answer.take() {
                        Some(Choice::Pick { indices }) => indices.first().copied().unwrap_or(0),
                        _ => 0,
                    };
                    chosen = options.get(i).or(options.first()).copied();
                } else if let Some(sel) = defender_among {
                    let ps = {
                        let c = ec!();
                        self.sel_players(sel, &c)
                    };
                    let mut options: Vec<Obj> = vec![];
                    if let Some(cb) = self.combat.as_ref() {
                        for a in &cb.attackers {
                            if ps.contains(&self.cards[a.card.0 as usize].cur.controller) && !options.contains(&a.defender) {
                                options.push(a.defender);
                            }
                        }
                    }
                    if options.len() == 1 {
                        chosen = options.first().copied();
                    } else if options.len() > 1 {
                        ctx.sub = Some(Sub::DefenderPick { options: options.clone() });
                        self.pending = Some(Decision {
                            player: controller,
                            kind: DecisionKind::Pick {
                                prompt: "Choose a player or planeswalker being attacked".into(),
                                source: Some(source),
                                options: options.iter().map(|o| Opt::Obj { obj: *o }).collect(),
                                min: 1,
                                max: 1,
                                cancellable: false,
                                hidden: false,
                                purpose: PickPurpose::Generic,
                            },
                        });
                        return Exec::Wait;
                    }
                }
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                for id in cs {
                    if self.cards[id.0 as usize].zone == Zone::Battlefield && self.combat.is_some() {
                        self.tap_card(id);
                        let ctl = self.cards[id.0 as usize].cur.controller;
                        if !self.combat.as_ref().map_or(false, |cb| cb.attackers.iter().any(|a| a.card == id)) {
                            match chosen {
                                Some(d) => self.add_attacker_at(id, d),
                                None => self.add_attacker_late(id, ctl),
                            }
                        }
                    }
                }
                Exec::Done
            }
            Effect::BecomeNight => {
                let was = self.day_night;
                self.day_night = Some(false);
                if was == Some(true) {
                    self.emit(Event::DayNightChanged { day: false });
                }
                Exec::Done
            }
            Effect::SetDay => {
                let was = self.day_night;
                self.day_night = Some(true);
                if was == Some(false) {
                    self.emit(Event::DayNightChanged { day: true });
                }
                Exec::Done
            }
            Effect::BecomeDay => {
                if self.day_night.is_none() {
                    self.day_night = Some(true);
                }
                Exec::Done
            }
            Effect::DamageCantBePrevented => {
                self.no_prevent_turn = self.turn;
                Exec::Done
            }
            Effect::BecomeMonarch { who } => {
                let ps = {
                    let c = ec!();
                    self.sel_players(who, &c)
                };
                if let Some(&p) = ps.first().filter(|&&p| self.monarch != Some(p) && !self.effects.iter().any(|e| matches!(e.modif, Modif::NoMonarch) && e.players.contains(&p))) {
                    self.monarch = Some(p);
                    self.layers_dirty = true;
                    self.emit(Event::Monarch { player: p });
                }
                Exec::Done
            }
            Effect::TakeInitiative { who } => {
                let ps = {
                    let c = ec!();
                    self.sel_players(who, &c)
                };
                if let Some(&p) = ps.first() {
                    // the emblem "The Initiative" follows the initiative and carries its abilities
                    if let Some(old) = self.initiative_emblem.take() {
                        self.move_card(old, Zone::Graveyard, MoveOpts::default());
                    }
                    self.initiative = Some(p);
                    if let Some(def) = self.db.find("The Initiative") {
                        let id = self.create_card(def, p, Zone::Command, true);
                        self.cards[id.0 as usize].controller = p;
                        self.initiative_emblem = Some(id);
                    }
                    self.layers_dirty = true;
                    self.emit(Event::InitiativeTaken { player: p });
                }
                Exec::Done
            }
            Effect::GainEnergy { who, n } => {
                let (ps, k) = {
                    let c = ec!();
                    (self.sel_players(who, &c), self.eval_amount(n, &c))
                };
                for p in ps {
                    if k > 0 {
                        // goes through counter replacements ("if you would get one or more {E}, you get twice that many")
                        self.add_counters_to(Obj::player(p), CounterKind::Energy, k);
                    } else {
                        self.players[p as usize].energy += k;
                        self.layers_dirty = true;
                    }
                }
                Exec::Done
            }
            Effect::AddPoison { who, n } => {
                let (ps, k) = {
                    let c = ec!();
                    (self.sel_players(who, &c), self.eval_amount(n, &c))
                };
                for p in ps {
                    let k2 = self.replace_player_counters(p, CounterKind::Poison, k);
                    self.players[p as usize].poison += k2;
                    if k2 > 0 {
                        self.emit(Event::PlayerCounters { player: p, kind: CounterKind::Poison, delta: k2, by: self.cause_ctl.get() });
                    }
                }
                Exec::Done
            }
            Effect::Reveal { what } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                for id in cs {
                    self.cards[id.0 as usize].known_to = 0xFF;
                }
                Exec::Done
            }
            Effect::LoseGame { who } => {
                let ps = {
                    let c = ec!();
                    self.sel_players(who, &c)
                };
                for p in ps {
                    self.player_loses(p, "an effect made them lose");
                }
                Exec::Done
            }
            Effect::DrawGame => {
                if self.over.is_none() {
                    self.over = Some(GameResult::Draw);
                    self.run = Run::Over;
                    self.emit(Event::Over { result: GameResult::Draw });
                }
                Exec::Done
            }
            Effect::WinGame { who } => {
                let ps = {
                    let c = ec!();
                    self.sel_players(who, &c)
                };
                if let Some(&w) = ps.first() {
                    for o in self.opponents(w) {
                        self.player_loses(o, "an opponent won the game");
                    }
                }
                Exec::Done
            }
            Effect::CounterKindsAmongOnto { among, onto } => {
                let (kinds, tokens): (Vec<CounterKind>, Vec<CardId>) = if let Some(Sub::KindsOnto { kinds, tokens }) = ctx.sub.clone() {
                    ctx.sub = None;
                    let idx = match self.answer.take() {
                        Some(Choice::Pick { indices }) => indices.first().copied().unwrap_or(0),
                        _ => 0,
                    };
                    if let (Some(k), Some(t)) = (kinds.first().copied(), tokens.get(idx).or(tokens.first()).copied()) {
                        self.add_counters_to(Obj::card(t), k, 1);
                    }
                    (kinds.into_iter().skip(1).collect(), tokens)
                } else {
                    let c = ec!();
                    let tokens: Vec<CardId> = self.sel_cards(onto, &c).into_iter().filter(|id| self.cards[id.0 as usize].zone == Zone::Battlefield).collect();
                    let mut kinds: Vec<CounterKind> = vec![];
                    for id in self.battlefield.clone() {
                        if self.matches_card(among, id, source, controller) {
                            for (k, n) in self.cards[id.0 as usize].counters.iter() {
                                if *n > 0 && !kinds.contains(k) {
                                    kinds.push(*k);
                                }
                            }
                        }
                    }
                    (kinds, tokens)
                };
                if tokens.is_empty() {
                    return Exec::Done;
                }
                let mut kinds = kinds;
                while !kinds.is_empty() && tokens.len() == 1 {
                    let k = kinds.remove(0);
                    self.add_counters_to(Obj::card(tokens[0]), k, 1);
                }
                if kinds.is_empty() {
                    return Exec::Done;
                }
                let prompt = format!("Choose which permanent gets a {:?} counter", kinds[0]);
                ctx.sub = Some(Sub::KindsOnto { kinds, tokens: tokens.clone() });
                self.pending = Some(Decision {
                    player: controller,
                    kind: DecisionKind::Pick {
                        prompt,
                        source: Some(source),
                        options: tokens.iter().map(|t| Opt::Obj { obj: Obj::card(*t) }).collect(),
                        min: 1,
                        max: 1,
                        cancellable: false,
                        hidden: false,
                        purpose: PickPurpose::Mode,
                    },
                });
                Exec::Wait
            }
            Effect::DuplicateCounter { what } => {
                if let Some(Sub::DupKind { obj, kinds }) = ctx.sub.clone() {
                    ctx.sub = None;
                    let idx = match self.answer.take() {
                        Some(Choice::Pick { indices }) => indices.first().copied().unwrap_or(0),
                        _ => 0,
                    };
                    if let Some(k) = kinds.get(idx).or(kinds.first()).copied() {
                        self.add_counters_to(obj, k, 1);
                    }
                    return Exec::Done;
                }
                let o = {
                    let c = ec!();
                    self.sel_objs(what, &c).first().copied()
                };
                let Some(o) = o else { return Exec::Done };
                let kinds: Vec<CounterKind> = match o {
                    Obj::Card { id } => self.cards[id.0 as usize].counters.iter().filter(|(_, n)| *n > 0).map(|(k, _)| *k).collect(),
                    Obj::Player { id } => {
                        let pl = &self.players[id as usize];
                        [(CounterKind::Poison, pl.poison), (CounterKind::Energy, pl.energy), (CounterKind::Experience, pl.experience), (CounterKind::Rad, pl.rad)].into_iter().filter(|(_, n)| *n > 0).map(|(k, _)| k).collect()
                    }
                    _ => vec![],
                };
                match kinds.len() {
                    0 => Exec::Done,
                    1 => {
                        self.add_counters_to(o, kinds[0], 1);
                        Exec::Done
                    }
                    _ => {
                        ctx.sub = Some(Sub::DupKind { obj: o, kinds: kinds.clone() });
                        self.pending = Some(Decision {
                            player: controller,
                            kind: DecisionKind::Pick {
                                prompt: "Choose a counter kind".into(),
                                source: Some(source),
                                options: kinds.iter().map(|k| Opt::Text { text: k.label() }).collect(),
                                min: 1,
                                max: 1,
                                cancellable: false,
                                hidden: false,
                                purpose: PickPurpose::Mode,
                            },
                        });
                        Exec::Wait
                    }
                }
            }
            Effect::Proliferate { remember } => {
                // choose any number of permanents and players with a counter; each gets one more of each kind it has
                let mut extra: u8 = 0;
                let mut resumed = false;
                if let Some(Sub::Proliferate { options, again }) = ctx.sub.clone() {
                    resumed = true;
                    ctx.sub = None;
                    let picked: Vec<Obj> = match self.answer.take() {
                        Some(Choice::Pick { indices }) => indices.iter().filter_map(|i| options.get(*i).copied()).collect(),
                        _ => vec![],
                    };
                    let mut got: Vec<Obj> = vec![];
                    for o in picked {
                        match o {
                            Obj::Card { id } => {
                                let kinds: Vec<CounterKind> = self.cards[id.0 as usize].counters.iter().map(|(k, _)| *k).collect();
                                for k in kinds {
                                    self.add_counters_to(o, k, 1);
                                }
                                got.push(o);
                            }
                            Obj::Player { id } => {
                                let mut gained: Vec<(CounterKind, i32)> = vec![];
                                let prev = self.cause_ctl.replace(Some(controller));
                                let have = {
                                    let pl = &self.players[id as usize];
                                    [(CounterKind::Poison, pl.poison > 0), (CounterKind::Energy, pl.energy > 0), (CounterKind::Experience, pl.experience > 0), (CounterKind::Rad, pl.rad > 0)]
                                };
                                for (kind, has) in have {
                                    if !has {
                                        continue;
                                    }
                                    let n = self.replace_player_counters(id, kind, 1);
                                    let pl = &mut self.players[id as usize];
                                    match kind {
                                        CounterKind::Poison => pl.poison += n,
                                        CounterKind::Energy => pl.energy += n,
                                        CounterKind::Experience => pl.experience += n,
                                        _ => pl.rad += n,
                                    }
                                    if n > 0 {
                                        gained.push((kind, n));
                                    }
                                }
                                self.cause_ctl.set(prev);
                                for (kind, n) in gained {
                                    self.emit(Event::PlayerCounters { player: id, kind, delta: n, by: self.cause_ctl.get() });
                                }
                            }
                            _ => {}
                        }
                    }
                    if *remember {
                        ctx.remembered.extend(got);
                    }
                    self.emit(Event::Proliferated { player: controller });
                    if again == 0 {
                        return Exec::Done;
                    }
                    extra = again - 1;
                }
                if !resumed {
                    if *remember {
                        ctx.remembered.clear();
                    }
                    // "If you would proliferate, proliferate twice instead" (Tekuthal): each such permanent doubles the number of rounds
                    let doublers = self
                        .battlefield
                        .iter()
                        .filter(|id| {
                            let c = &self.cards[id.0 as usize];
                            let sd = self.db.def(c.def);
                            c.cur.controller == controller && !c.cur.no_abilities && sd.statics.iter().any(|&i| matches!(&sd.abilities[i as usize], Ability::Static { effect: StaticEffect::ProliferateTwice, .. }))
                        })
                        .count()
                        .min(3);
                    extra = (1u8 << doublers) - 1;
                }
                let mut options: Vec<Obj> = self.battlefield.iter().copied().filter(|id| !self.cards[id.0 as usize].counters.is_empty()).map(Obj::card).collect();
                for p in self.alive_players() {
                    let pl = &self.players[p as usize];
                    if pl.poison > 0 || pl.energy > 0 || pl.experience > 0 || pl.rad > 0 {
                        options.push(Obj::player(p));
                    }
                }
                if options.is_empty() {
                    for _ in 0..=extra {
                        self.emit(Event::Proliferated { player: controller });
                    }
                    return Exec::Done;
                }
                ctx.sub = Some(Sub::Proliferate { options: options.clone(), again: extra });
                self.pending = Some(Decision {
                    player: controller,
                    kind: DecisionKind::Pick {
                        prompt: "Proliferate: choose any number of permanents and players with counters".into(),
                        source: Some(source),
                        options: options.iter().map(|o| Opt::Obj { obj: *o }).collect(),
                        min: 0,
                        max: options.len().min(255) as u8,
                        cancellable: false,
                        hidden: false,
                        purpose: PickPurpose::Proliferate,
                    },
                });
                Exec::Wait
            }
            Effect::Investigate { n } => {
                let k = {
                    let c = ec!();
                    self.eval_amount(n, &c)
                };
                let first = self.players[controller as usize].investigated_turn == 0;
                self.players[controller as usize].investigated_turn = self.players[controller as usize].investigated_turn.saturating_add(1);
                // a token-copy replacement (Moonlit Meditation, Mirrormind Crown) applies to the Clue too: create them through the token effect, which asks
                if k > 0 && self.token_copy_offer(controller).is_some() {
                    if let Ok(tok) = serde_json::from_str::<Effect>(&format!(r#"{{"type":"CreateToken","token":"Clue","n":{{"type":"Fixed","n":{k}}}}}"#)) {
                        self.emit(Event::Investigated { player: controller, first });
                        return Exec::Push(Frame { list: Arc::new(vec![tok]), pc: 0, repeat: 0, each: vec![], each_active: false, tbase: ctx.cur_tbase });
                    }
                }
                self.make_named_tokens("Clue", k, controller, ctx);
                self.emit(Event::Investigated { player: controller, first });
                Exec::Done
            }
            Effect::Amass { n, subtype } => {
                let k = {
                    let c = ec!();
                    self.eval_amount(n, &c)
                };
                // an Army you control gets counters (the controller chooses among several), otherwise create a 0/0 Army
                let armies: Vec<CardId> = self.battlefield.iter().copied().filter(|id| {
                    let c = &self.cards[id.0 as usize];
                    c.cur.controller == controller && (c.cur.subtypes.contains(&Sym::new("Army")) || (c.cur.kw.has(Keyword::Changeling) && c.cur.types.has(CardType::Creature)))
                }).collect();
                let mut army = armies.first().copied();
                if let Some(Sub::AmassPick { armies: opts }) = ctx.sub.clone() {
                    ctx.sub = None;
                    if let Some(Choice::Pick { indices }) = self.answer.take() {
                        army = indices.first().and_then(|i| opts.get(*i)).copied().or(army);
                    }
                } else if armies.len() > 1 {
                    ctx.sub = Some(Sub::AmassPick { armies: armies.clone() });
                    self.pending = Some(Decision {
                        player: controller,
                        kind: DecisionKind::Pick {
                            prompt: "Choose an Army you control to put the counters on".into(),
                            source: Some(source),
                            options: armies.iter().map(|c| Opt::Obj { obj: Obj::card(*c) }).collect(),
                            min: 1,
                            max: 1,
                            cancellable: false,
                            hidden: false,
                            purpose: PickPurpose::Generic,
                        },
                    });
                    return Exec::Wait;
                }
                let target = match army {
                    Some(a) => a,
                    None => {
                        self.make_named_tokens("Army", 1, controller, ctx);
                        match ctx.remembered.first() {
                            Some(Obj::Card { id }) => *id,
                            _ => return Exec::Done,
                        }
                    }
                };
                self.add_counters_to(Obj::card(target), CounterKind::PlusOnePlusOne, k);
                if let Some(st) = subtype {
                    self.add_effect(source, controller, &[target], Modif::AddSubtype(Sym::new(st.strip_suffix('s').unwrap_or(st))), Duration::Forever);
                }
                // the Army that was amassed ("the amassed Army's power")
                ctx.remembered = vec![Obj::card(target)].into_iter().collect();
                Exec::Done
            }
            Effect::LinkPlayer { what } => {
                let p = {
                    let c = ec!();
                    self.sel_players(what, &c).first().copied()
                };
                self.cards[source.0 as usize].chosen_player = p;
                self.layers_dirty = true;
                Exec::Done
            }
            Effect::Remember { what } => {
                let objs = {
                    let c = ec!();
                    self.sel_objs(what, &c)
                };
                ctx.remembered = objs.into_iter().collect();
                Exec::Done
            }
            Effect::StoreRemembered { slot, append } => {
                let mut st = self.stored_sets.borrow_mut();
                if let Some(s) = st.get_mut(*slot as usize) {
                    if *append {
                        s.extend(ctx.remembered.iter().copied());
                    } else {
                        *s = ctx.remembered.clone();
                    }
                }
                Exec::Done
            }
            Effect::RevealTop { who, n } => {
                let (ps, k) = {
                    let c = ec!();
                    (self.sel_players(who, &c), self.eval_amount(n, &c).max(0) as usize)
                };
                let mut out: Vec<Obj> = vec![];
                for p in ps {
                    let lib = &self.players[p as usize].library;
                    out.extend(lib.iter().rev().take(k).map(|c| Obj::card(*c)));
                }
                for o in &out {
                    if let Obj::Card { id } = o {
                        self.cards[id.0 as usize].known_to = 0xFF;
                    }
                }
                ctx.remembered = out;
                Exec::Done
            }
            Effect::ChangeDefender { what, to } => {
                let (cs, dest) = {
                    let c = ec!();
                    let dest = self.sel_players(to, &c).first().map(|p| Obj::player(*p)).or_else(|| self.sel_cards(to, &c).first().map(|k| Obj::card(*k)));
                    (self.sel_cards(what, &c), dest)
                };
                if let (Some(d), Some(cb)) = (dest, self.combat.as_mut()) {
                    for a in cb.attackers.iter_mut() {
                        if cs.contains(&a.card) {
                            a.defender = d;
                        }
                    }
                }
                Exec::Done
            }
            Effect::ReselectDefender { what } => {
                let mut chosen: Option<Obj> = None;
                if let Some(Sub::DefenderPick { options }) = ctx.sub.clone() {
                    ctx.sub = None;
                    let i = match self.answer.take() {
                        Some(Choice::Pick { indices }) => indices.first().copied().unwrap_or(0),
                        _ => 0,
                    };
                    chosen = options.get(i).or(options.first()).copied();
                } else {
                    let att = {
                        let c = ec!();
                        self.sel_cards(what, &c).first().copied()
                    };
                    let Some(att) = att.filter(|a| self.combat.as_ref().map_or(false, |cb| cb.attackers.iter().any(|x| x.card == *a))) else { return Exec::Done };
                    let ctl = self.cards[att.0 as usize].cur.controller;
                    let mut options: Vec<Obj> = vec![];
                    for p in 0..self.players.len() as PlayerId {
                        if p != ctl && self.alive(p) {
                            options.push(Obj::player(p));
                        }
                    }
                    for id in self.battlefield.iter().copied() {
                        let c = &self.cards[id.0 as usize];
                        let side = self.defender_side(id);
                        if (c.cur.types.has(CardType::Planeswalker) || c.cur.types.has(CardType::Battle)) && side != ctl && self.alive(side) {
                            options.push(Obj::card(id));
                        }
                    }
                    if options.len() == 1 {
                        chosen = options.first().copied();
                    } else if options.len() > 1 {
                        ctx.sub = Some(Sub::DefenderPick { options: options.clone() });
                        self.pending = Some(Decision {
                            player: controller,
                            kind: DecisionKind::Pick {
                                prompt: "Choose a player or planeswalker for the attacking creature to attack".into(),
                                source: Some(source),
                                options: options.iter().map(|o| Opt::Obj { obj: *o }).collect(),
                                min: 1,
                                max: 1,
                                cancellable: false,
                                hidden: false,
                                purpose: PickPurpose::Generic,
                            },
                        });
                        return Exec::Wait;
                    }
                }
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                if let (Some(d), Some(cb)) = (chosen, self.combat.as_mut()) {
                    if let Some(a) = cb.attackers.iter_mut().find(|a| cs.first() == Some(&a.card)) {
                        a.defender = d;
                    }
                }
                Exec::Done
            }
            Effect::AppendRemembered { what } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                for id in cs {
                    if !ctx.remembered.contains(&Obj::card(id)) {
                        ctx.remembered.push(Obj::card(id));
                    }
                }
                Exec::Done
            }
            Effect::AbilityCantBeCopied => Exec::Done,
            Effect::ClearRemembered => {
                ctx.remembered.clear();
                Exec::Done
            }
            Effect::Populate | Effect::PopulateAttacking => {
                let attacking_copy = matches!(e, Effect::PopulateAttacking);
                if let Some(Sub::Choose { options, then }) = ctx.sub.clone() {
                    let ans = self.answer.take();
                    ctx.sub = None;
                    let picked: Vec<CardId> = match ans {
                        Some(Choice::Pick { indices }) => indices.iter().filter_map(|i| options.get(*i).copied()).collect(),
                        _ => vec![],
                    };
                    return self.choose_then(ctx, picked, then);
                }
                // one candidate per distinct token the controller has; with several the controller picks which to copy
                let mut cands: Vec<CardId> = vec![];
                for id in self.battlefield.iter().copied() {
                    let c = &self.cards[id.0 as usize];
                    if c.cur.controller == controller && c.token && c.cur.types.has(CardType::Creature) && !cands.iter().any(|x| self.cards[x.0 as usize].def == c.def) {
                        cands.push(id);
                    }
                }
                if cands.is_empty() || self.token_cap_reached() {
                    return Exec::Done;
                }
                let then = vec![Effect::CopyPermanent { what: Sel::Remembered, n: Amount::Fixed { n: 1 }, except: vec![], tapped: attacking_copy, attacking: attacking_copy, by: None, attach_to: None, attack_at: None }];
                if cands.len() == 1 {
                    return self.choose_then(ctx, cands, then);
                }
                ctx.sub = Some(Sub::Choose { options: cands.clone(), then });
                self.pending = Some(Decision {
                    player: controller,
                    kind: DecisionKind::Pick {
                        prompt: format!("{}: choose a creature token to copy", self.def_of(source).name),
                        source: Some(source),
                        options: cands.iter().map(|c| Opt::Obj { obj: Obj::card(*c) }).collect(),
                        min: 1,
                        max: 1,
                        cancellable: false,
                        hidden: false,
                        purpose: PickPurpose::Generic,
                    },
                });
                Exec::Wait
            }
            Effect::CollectEvidenceChosen => {
                let n = self.cards[source.0 as usize].chosen_number.unwrap_or(0);
                ctx.remembered.clear();
                if n > 0 {
                    if let Some(cards) = self.evidence_cards(controller, n, source) {
                        for id in cards {
                            self.move_card(id, Zone::Exile, Default::default());
                            ctx.remembered.push(Obj::card(id));
                        }
                        self.emit(Event::EvidenceCollected { player: controller });
                    }
                }
                Exec::Done
            }
            Effect::StoreNumber { amount } => {
                let n = {
                    let c = ec!();
                    self.eval_amount(amount, &c)
                };
                self.cards[source.0 as usize].chosen_number = Some(n);
                Exec::Done
            }
            Effect::NoteCreatureType { of } => {
                if let Some(Sub::ChooseVal { kind: 5, texts, .. }) = ctx.sub.clone() {
                    let ans = self.answer.take();
                    ctx.sub = None;
                    if let Some(t) = if let Some(Choice::Pick { indices }) = ans { indices.first().and_then(|i| texts.get(*i)).cloned() } else { None } {
                        self.cards[source.0 as usize].noted_types.push(Sym::new(&t));
                    }
                    return Exec::Done;
                }
                let Some(card) = ({
                    let c = ec!();
                    self.sel_cards(of, &c).first().copied()
                }) else {
                    return Exec::Done;
                };
                let mut v: Vec<String> = vec![];
                if self.cards[card.0 as usize].cur.kw.has(Keyword::Changeling) {
                    for d in &self.db.defs {
                        if d.types.has(CardType::Creature) || d.types.has(CardType::Kindred) {
                            for s in &d.subtypes {
                                let n = s.name();
                                if !v.contains(&n) {
                                    v.push(n);
                                }
                            }
                        }
                    }
                } else {
                    v = self.cards[card.0 as usize].cur.subtypes.iter().map(|s| s.name()).collect();
                }
                v.sort();
                v.dedup();
                v.retain(|t| !self.cards[source.0 as usize].noted_types.contains(&Sym::new(t)));
                match v.len() {
                    0 => Exec::Done,
                    1 => {
                        self.cards[source.0 as usize].noted_types.push(Sym::new(&v[0]));
                        Exec::Done
                    }
                    _ => {
                        let name = self.def_of(source).name.clone();
                        ctx.sub = Some(Sub::ChooseVal { player: controller, kind: 5, texts: v.clone(), players: vec![] });
                        self.pending = Some(Decision {
                            player: controller,
                            kind: DecisionKind::Pick {
                                prompt: format!("{name}: note a creature type"),
                                source: Some(source),
                                options: v.iter().map(|t| Opt::Text { text: t.clone() }).collect(),
                                min: 1,
                                max: 1,
                                cancellable: false,
                                hidden: false,
                                purpose: PickPurpose::Generic,
                            },
                        });
                        Exec::Wait
                    }
                }
            }
            Effect::ExchangeTextBoxes { a, b, while_source } => {
                let (xs, ys) = {
                    let c = ec!();
                    (self.sel_cards(a, &c), self.sel_cards(b, &c))
                };
                if let (Some(&x), Some(&y)) = (xs.first(), ys.first()) {
                    if x != y {
                        let (dx, dy) = (self.cards[x.0 as usize].def, self.cards[y.0 as usize].def);
                        self.apply_text_change(x, crate::textchange::TextChange::SetAbilitiesFrom { donor: dy }, false);
                        self.apply_text_change(y, crate::textchange::TextChange::SetAbilitiesFrom { donor: dx }, false);
                        if *while_source && self.battlefield.contains(&source) {
                            self.text_swaps.push((source, x, y, dx, dy));
                        }
                    }
                }
                Exec::Done
            }
            Effect::ChangeText { what, kinds, duration, to, exclude_new } => {
                use crate::textchange::*;
                #[allow(unused)]
                let kind_of = |k: u8| match k {
                    0 => WordKind::Color,
                    1 => WordKind::Land,
                    _ => WordKind::Creature,
                };
                let code = |k: &TextKind| match k {
                    TextKind::Color => 0u8,
                    TextKind::LandType => 1,
                    TextKind::CreatureType => 2,
                };
                let (mut stage, mut target, mut kind, mut old) = (0u8, CardId(0), 0u8, String::new());
                let mut idx: Option<usize> = None;
                if let Some(Sub::TextChange { stage: s, target: t, kind: k, old: o }) = ctx.sub.clone() {
                    ctx.sub = None;
                    self.text_ask = None;
                    stage = s;
                    target = t;
                    kind = k;
                    old = o;
                    idx = match self.answer.take() {
                        Some(Choice::Pick { indices }) => indices.first().copied(),
                        _ => None,
                    };
                } else {
                    let c = ec!();
                    match self.sel_cards(what, &c).first() {
                        Some(&t) => target = t,
                        None => return Exec::Done,
                    }
                }
                let ask = |g: &mut Game, ctx: &mut ResolveCtx, stage: u8, target: CardId, kind: u8, old: &str, prompt: String, options: Vec<String>| -> Exec {
                    ctx.sub = Some(Sub::TextChange { stage, target, kind, old: old.to_string() });
                    g.text_ask = Some(TextAsk { target, stage, kind: kind_of(kind), old: old.to_string() });
                    g.pending = Some(Decision {
                        player: controller,
                        kind: DecisionKind::Pick { prompt, source: Some(source), options: options.into_iter().map(|t| Opt::Text { text: t }).collect(), min: 1, max: 1, cancellable: false, hidden: false, purpose: PickPurpose::Generic },
                    });
                    Exec::Wait
                };
                loop {
                    match stage {
                        0 | 1 => {
                            let d = self.def_of(target);
                            let avail: Vec<u8> = kinds.iter().map(|k| code(k)).filter(|k| !words_in(d, kind_of(*k)).is_empty()).collect();
                            if avail.is_empty() {
                                return Exec::Done;
                            }
                            if stage == 1 {
                                kind = avail[idx.unwrap_or(0).min(avail.len() - 1)];
                                stage = 2;
                            } else if avail.len() == 1 {
                                kind = avail[0];
                                stage = 2;
                            } else {
                                let names: Vec<String> = avail.iter().map(|k| match k { 0 => "A color word", 1 => "A basic land type", _ => "A creature type" }.to_string()).collect();
                                return ask(self, ctx, 1, target, 0, "", "Change which kind of word?".into(), names);
                            }
                        }
                        2 | 3 => {
                            let words = words_in(self.def_of(target), kind_of(kind));
                            if words.is_empty() {
                                return Exec::Done;
                            }
                            if stage == 3 {
                                old = words[idx.unwrap_or(0).min(words.len() - 1)].clone();
                                stage = 4;
                            } else if words.len() == 1 {
                                old = words[0].clone();
                                stage = 4;
                            } else {
                                let shown: Vec<String> = words.iter().map(|w| display_word(kind_of(kind), w)).collect();
                                return ask(self, ctx, 3, target, kind, "", "Replace which word?".into(), shown);
                            }
                        }
                        4 | 5 => {
                            let mut opts: Vec<String> = all_words(&self.db, kind_of(kind)).into_iter().filter(|w| *w != old && !exclude_new.contains(w)).collect();
                            if let Some(t) = to {
                                opts = vec![t.clone()];
                            }
                            if opts.is_empty() {
                                return Exec::Done;
                            }
                            if stage == 5 {
                                let new = opts[idx.unwrap_or(0).min(opts.len() - 1)].clone();
                                let new = parse_word(kind_of(kind), &new);
                                self.apply_text_change(target, TextChange::Word { kind: kind_of(kind), old: old.clone(), new }, matches!(duration, Duration::UntilEndOfTurn));
                                return Exec::Done;
                            }
                            if to.is_some() || opts.len() == 1 {
                                let new = opts[0].clone();
                                let new = parse_word(kind_of(kind), &new);
                                self.apply_text_change(target, TextChange::Word { kind: kind_of(kind), old: old.clone(), new }, matches!(duration, Duration::UntilEndOfTurn));
                                return Exec::Done;
                            }
                            let shown: Vec<String> = opts.iter().map(|w| display_word(kind_of(kind), w)).collect();
                            return ask(self, ctx, 5, target, kind, &old, "Replace it with which word?".into(), shown);
                        }
                        _ => return Exec::Done,
                    }
                }
            }
            Effect::ControlPlayerWhileResolves { who, what } => {
                let (v, cs) = {
                    let c = ec!();
                    (self.sel_players(who, &c).first().copied(), self.sel_cards(what, &c))
                };
                if let (Some(v), Some(&card)) = (v, cs.first()) {
                    if self.cards[card.0 as usize].zone == Zone::Stack {
                        self.spell_ctl.push((card, v, controller));
                    } else if self.free_casts.iter().any(|(_, c, _, _)| *c == card) {
                        // the cast happens once this resolution is over: keep the control (and the mana restriction) until then
                        self.wok_pending = Some((card, v, controller));
                    }
                }
                Exec::Done
            }
            Effect::ControlPlayerDuring { who, effects, lands_only_mana } => {
                let v = {
                    let c = ec!();
                    self.sel_players(who, &c).first().copied()
                };
                let Some(v) = v else { return Exec::Done };
                self.resolve_ctl = Some((v, controller));
                if *lands_only_mana {
                    self.land_mana_only = Some(v);
                }
                let mut list = effects.clone();
                list.push(Effect::EndPlayerControl);
                Exec::Push(Frame { list: Arc::new(list), pc: 0, repeat: 0, each: vec![], each_active: false, tbase: ctx.cur_tbase })
            }
            Effect::EndPlayerControl => {
                if self.wok_pending.is_none() {
                    self.resolve_ctl = None;
                    self.land_mana_only = None;
                }
                Exec::Done
            }
            Effect::ControlVotesThisTurn => {
                self.vote_control.retain(|(_, t)| *t == self.turn);
                self.vote_control.push((controller, self.turn));
                Exec::Done
            }
            Effect::FacesVillainousChoice { who, effects } => {
                let faced = {
                    let c = ec!();
                    self.sel_players(who, &c)
                };
                let twice = faced.first().map_or(false, |&fp| self.villainous_twice(fp));
                let mut list = effects.clone();
                if twice {
                    list.extend(effects.iter().cloned());
                }
                Exec::Push(Frame { list: Arc::new(list), pc: 0, repeat: 0, each: vec![], each_active: false, tbase: ctx.cur_tbase })
            }
            Effect::ResetPlayerNumbers => {
                self.player_numbers = vec![None; self.players.len()];
                Exec::Done
            }
            Effect::ChooseColor { who, .. } | Effect::ChooseCreatureType { who, .. } | Effect::ChooseCardType { who, .. } | Effect::ChooseNumber { who, .. } | Effect::ChoosePlayer { who, .. } => {
                if let Effect::ChooseNumber { min, max, random: true, record, .. } = e {
                    let n = if max > min { self.rng.gen_range(*min..=*max) } else { *min };
                    self.cards[source.0 as usize].chosen_number = Some(n);
                    if *record {
                        let who_p = { let c = ec!(); self.sel_players(who, &c).first().copied() };
                        self.record_number(who_p, n);
                    }
                    return Exec::Done;
                }
                if let Some(Sub::ChooseVal { kind, texts, players, player: chooser }) = ctx.sub.clone() {
                    let ans = self.answer.take();
                    ctx.sub = None;
                    let per_player = matches!(e, Effect::ChooseColor { per_player: true, .. });
                    let card = &mut self.cards[source.0 as usize];
                    match (ans, kind) {
                        (Some(Choice::Pick { indices }), 0) if per_player => {
                            if let Some(col) = indices.first().and_then(|i| Color::ALL.get(*i).copied()) {
                                card.player_colors.retain(|(p, _)| *p != chooser);
                                card.player_colors.push((chooser, col));
                            }
                        }
                        (Some(Choice::Pick { indices }), 0) => card.chosen_color = indices.first().and_then(|i| Color::ALL.get(*i).copied()),
                        (Some(Choice::Pick { indices }), 1) => {
                            card.chosen_type = indices.first().and_then(|i| texts.get(*i)).map(|t| Sym::new(t));
                            if let (Effect::ChooseCreatureType { note: true, .. }, Some(t)) = (e, card.chosen_type) {
                                card.noted_types.push(t);
                            }
                            if matches!(e, Effect::ChooseCreatureType { accumulate: true, .. }) {
                                if card.chosen_types.0 != card.zcc {
                                    card.chosen_types = (card.zcc, vec![]);
                                }
                                if let Some(t) = card.chosen_type {
                                    card.chosen_types.1.push(t);
                                }
                            }
                        }
                        (Some(Choice::Pick { indices }), 3) => card.chosen_player = indices.first().and_then(|i| players.get(*i)).copied(),
                        (Some(Choice::Pick { indices }), 6) => {
                            if let Some(n) = indices.first().and_then(|i| texts.get(*i)).and_then(|t| t.parse::<i32>().ok()) {
                                card.chosen_number = Some(n);
                                card.chosen_mask |= 1 << n;
                                if matches!(e, Effect::ChooseNumber { record: true, .. }) {
                                    self.record_number(Some(chooser), n);
                                }
                            }
                        }
                        (Some(Choice::Number { n }), 2) => {
                            card.chosen_number = Some(n);
                            if matches!(e, Effect::ChooseNumber { record: true, .. }) {
                                self.record_number(Some(chooser), n);
                            }
                        }
                        _ => {}
                    }
                    self.layers_dirty = true;
                    return Exec::Done;
                }
                let chooser = {
                    let c = ec!();
                    self.sel_players(who, &c).first().copied()
                };
                let Some(chooser) = chooser else { return Exec::Done };
                let name = self.def_of(source).name.clone();
                let (kind, kind_decision, texts, players): (u8, DecisionKind, Vec<String>, Vec<PlayerId>) = match e {
                    Effect::ChooseColor { .. } => (
                        0,
                        DecisionKind::Pick {
                            prompt: format!("{name}: choose a color"),
                            source: Some(source),
                            options: Color::ALL.iter().map(|c| Opt::Color { c: *c }).collect(),
                            min: 1,
                            max: 1,
                            cancellable: false,
                            hidden: false,
                            purpose: PickPurpose::Generic,
                        },
                        vec![],
                        vec![],
                    ),
                    Effect::ChooseCardType { excluding, .. } => {
                        let v: Vec<String> = ["Artifact", "Battle", "Creature", "Enchantment", "Instant", "Kindred", "Land", "Planeswalker", "Sorcery"].iter().map(|s| s.to_string()).filter(|s| !excluding.contains(s)).collect();
                        (
                            1,
                            DecisionKind::Pick {
                                prompt: format!("{name}: choose a card type"),
                                source: Some(source),
                                options: v.iter().map(|t| Opt::Text { text: t.clone() }).collect(),
                                min: 1,
                                max: 1,
                                cancellable: false,
                                hidden: false,
                                purpose: PickPurpose::Generic,
                            },
                            v,
                            vec![],
                        )
                    }
                    Effect::ChooseCreatureType { only, excluding, nonbasic_land_type, planeswalker_type, any_land_type, exclude_noted, .. } => {
                        let mut v: Vec<String> = vec![];
                        for d in &self.db.defs {
                            if (*planeswalker_type && d.types.has(CardType::Planeswalker)) || (!*planeswalker_type && (*nonbasic_land_type || *any_land_type) && d.types.has(CardType::Land)) || (!*planeswalker_type && !*nonbasic_land_type && !*any_land_type && (d.types.has(CardType::Creature) || d.types.has(CardType::Kindred))) {
                                for s in &d.subtypes {
                                    let n = s.name();
                                    if !v.contains(&n) {
                                        v.push(n);
                                    }
                                }
                            }
                        }
                        for t in only {
                            if !v.contains(t) {
                                v.push(t.clone());
                            }
                        }
                        if *nonbasic_land_type {
                            v.retain(|t| !["Plains", "Island", "Swamp", "Mountain", "Forest"].contains(&t.as_str()));
                        }
                        v.sort();
                        v.retain(|t| (only.is_empty() || only.contains(t)) && !excluding.contains(t) && !(*exclude_noted && self.cards[source.0 as usize].noted_types.contains(&Sym::new(t))));
                        (
                            1,
                            DecisionKind::Pick {
                                prompt: format!("{name}: choose a {} type", if *planeswalker_type { "planeswalker" } else { "creature" }),
                                source: Some(source),
                                options: v.iter().map(|t| Opt::Text { text: t.clone() }).collect(),
                                min: 1,
                                max: 1,
                                cancellable: false,
                                hidden: false,
                                purpose: PickPurpose::Generic,
                            },
                            v,
                            vec![],
                        )
                    }
                    Effect::ChooseNumber { min, max, exclude_chosen: true, .. } => {
                        let mask = self.cards[source.0 as usize].chosen_mask;
                        let v: Vec<String> = (*min..=*max).filter(|n| *n >= 0 && *n < 32 && mask & (1 << *n) == 0).map(|n| n.to_string()).collect();
                        if v.is_empty() {
                            return Exec::Done;
                        }
                        (
                            6,
                            DecisionKind::Pick { prompt: format!("{name}: choose a number"), source: Some(source), options: v.iter().map(|t| Opt::Text { text: t.clone() }).collect(), min: 1, max: 1, cancellable: false, hidden: true, purpose: PickPurpose::Generic },
                            v,
                            vec![],
                        )
                    }
                    Effect::ChooseNumber { min, max, max_amount, .. } => {
                        let hi = match max_amount {
                            Some(a) => {
                                let c = ec!();
                                self.eval_amount(a, &c).max(*min)
                            }
                            None => *max,
                        };
                        (2, DecisionKind::Number { prompt: format!("{name}: choose a number"), min: *min, max: hi, cancellable: false }, vec![], vec![])
                    }
                    _ => {
                        let opp = matches!(e, Effect::ChoosePlayer { opponent: true, .. });
                        let among = if let Effect::ChoosePlayer { among, .. } = e { *among } else { PlayerFilter::Any };
                        let excl: Vec<PlayerId> = match e {
                            Effect::ChoosePlayer { exclude: Some(x), .. } => {
                                let c = ec!();
                                self.sel_players(x, &c)
                            }
                            _ => vec![],
                        };
                        let ps: Vec<PlayerId> = if opp { self.opponents(chooser) } else { self.alive_players() }.into_iter().filter(|q| self.player_filter(among, *q, chooser) && !excl.contains(q)).collect();
                        if ps.is_empty() {
                            if matches!(e, Effect::ChoosePlayer { exclude: Some(_), .. }) {
                                self.cards[source.0 as usize].chosen_player = None;
                                self.layers_dirty = true;
                            }
                            return Exec::Done;
                        }
                        let random = matches!(e, Effect::ChoosePlayer { random: true, .. });
                        if ps.len() == 1 || random {
                            let i = if random { self.rng.gen_range(0..ps.len()) } else { 0 };
                            self.cards[source.0 as usize].chosen_player = Some(ps[i]);
                            self.layers_dirty = true;
                            return Exec::Done;
                        }
                        (
                            3,
                            DecisionKind::Pick {
                                prompt: format!("{name}: choose a player"),
                                source: Some(source),
                                options: ps.iter().map(|p| Opt::Obj { obj: Obj::player(*p) }).collect(),
                                min: 1,
                                max: 1,
                                cancellable: false,
                                hidden: false,
                                purpose: PickPurpose::Generic,
                            },
                            vec![],
                            ps,
                        )
                    }
                };
                ctx.sub = Some(Sub::ChooseVal { player: chooser, kind, texts, players });
                self.pending = Some(Decision { player: chooser, kind: kind_decision });
                Exec::Wait
            }
            Effect::ChooseFriendOrFoe => {
                let ask = |g: &mut Self, ctx: &mut ResolveCtx, who: PlayerId, rest: Vec<PlayerId>| {
                    let name = g.def_of(source).name.clone();
                    ctx.sub = Some(Sub::ChooseVal { player: controller, kind: 91, texts: vec!["Friend".into(), "Foe".into()], players: std::iter::once(who).chain(rest).collect() });
                    g.pending = Some(Decision {
                        player: controller,
                        kind: DecisionKind::Pick {
                            prompt: format!("{name}: is Player {} a friend or a foe?", who + 1),
                            source: Some(source),
                            options: vec![Opt::Text { text: "Friend".into() }, Opt::Text { text: "Foe".into() }],
                            min: 1,
                            max: 1,
                            cancellable: false,
                            hidden: false,
                            purpose: PickPurpose::Generic,
                        },
                    });
                };
                if let Some(Sub::ChooseVal { kind: 91, players, .. }) = ctx.sub.clone() {
                    ctx.sub = None;
                    let foe = matches!(self.answer.take(), Some(Choice::Pick { indices }) if indices.first() == Some(&1));
                    if let Some(&p) = players.first() {
                        self.cards[source.0 as usize].player_pick.push((p, foe));
                    }
                    if players.len() > 1 {
                        ask(self, ctx, players[1], players[2..].to_vec());
                        return Exec::Wait;
                    }
                    return Exec::Done;
                }
                self.cards[source.0 as usize].player_pick.clear();
                let mut queue = vec![];
                let mut p = controller;
                for _ in 0..self.players.len() {
                    if self.alive(p) {
                        queue.push(p);
                    }
                    p = self.next_player(p);
                }
                if queue.is_empty() {
                    return Exec::Done;
                }
                ask(self, ctx, queue[0], queue[1..].to_vec());
                Exec::Wait
            }
            Effect::BecomeChosenColors { what, duration } => {
                if let Some(Sub::ChooseVal { kind: 6, .. }) = ctx.sub.clone() {
                    ctx.sub = None;
                    if let Some(Choice::Pick { indices }) = self.answer.take() {
                        let cols: Vec<Color> = indices.iter().filter_map(|i| Color::ALL.get(*i).copied()).collect();
                        if !cols.is_empty() {
                            let cs = {
                                let c = ec!();
                                self.sel_cards(what, &c)
                            };
                            for id in cs {
                                self.add_effect(source, controller, &[id], Modif::Colors(ColorSet::of(&cols)), *duration);
                            }
                        }
                    }
                    return Exec::Done;
                }
                let live = {
                    let c = ec!();
                    !self.sel_cards(what, &c).is_empty()
                };
                if !live {
                    return Exec::Done;
                }
                let name = self.def_of(source).name.clone();
                ctx.sub = Some(Sub::ChooseVal { player: controller, kind: 6, texts: vec![], players: vec![] });
                self.pending = Some(Decision {
                    player: controller,
                    kind: DecisionKind::Pick {
                        prompt: format!("{name}: choose one or more colors"),
                        source: Some(source),
                        options: Color::ALL.iter().map(|c| Opt::Color { c: *c }).collect(),
                        min: 1,
                        max: 5,
                        cancellable: false,
                        hidden: false,
                        purpose: PickPurpose::Generic,
                    },
                });
                Exec::Wait
            }
            Effect::ChangeLandTypes { who, duration } => {
                if let Some(Sub::ChooseVal { player, kind, texts, .. }) = ctx.sub.clone() {
                    let pick = match self.answer.take() {
                        Some(Choice::Pick { indices }) => indices.first().and_then(|i| texts.get(*i)).cloned(),
                        _ => None,
                    };
                    let Some(pick) = pick else {
                        ctx.sub = None;
                        return Exec::Done;
                    };
                    let name = self.def_of(source).name.clone();
                    if kind == 5 {
                        // the first (any) land type is kept on the source while the basic type is asked
                        self.cards[source.0 as usize].chosen_type = Some(Sym::new(&pick));
                        let v: Vec<String> = ["Plains", "Island", "Swamp", "Mountain", "Forest"].iter().map(|s| s.to_string()).collect();
                        ctx.sub = Some(Sub::ChooseVal { player, kind: 6, texts: v.clone(), players: vec![] });
                        self.pending = Some(Decision {
                            player,
                            kind: DecisionKind::Pick {
                                prompt: format!("{name}: choose a basic land type"),
                                source: Some(source),
                                options: v.iter().map(|t| Opt::Text { text: t.clone() }).collect(),
                                min: 1,
                                max: 1,
                                cancellable: false,
                                hidden: false,
                                purpose: PickPurpose::Generic,
                            },
                        });
                        return Exec::Wait;
                    }
                    ctx.sub = None;
                    let first = self.cards[source.0 as usize].chosen_type.take();
                    if let Some(first) = first {
                        let lands: Vec<CardId> = self.battlefield.iter().copied().filter(|id| self.cards[id.0 as usize].cur.types.has(CardType::Land) && self.cards[id.0 as usize].cur.subtypes.contains(&first)).collect();
                        if !lands.is_empty() {
                            // 305.7: setting a basic land type removes the land's other abilities; the new type's intrinsic mana ability replaces them
                            self.add_effect(source, controller, &lands, Modif::SetSubtypesKind(SubtypeKind::Land, vec![Sym::new(&pick)]), *duration);
                            self.add_effect(source, controller, &lands, Modif::NoAbilities, *duration);
                        }
                    }
                    return Exec::Done;
                }
                let chooser = {
                    let c = ec!();
                    self.sel_players(who, &c).first().copied()
                };
                let Some(chooser) = chooser else { return Exec::Done };
                let name = self.def_of(source).name.clone();
                let mut v: Vec<String> = ["Plains", "Island", "Swamp", "Mountain", "Forest"].iter().map(|s| s.to_string()).collect();
                for d in &self.db.defs {
                    if d.types.has(CardType::Land) {
                        for s in &d.subtypes {
                            let n = s.name();
                            if !v.contains(&n) {
                                v.push(n);
                            }
                        }
                    }
                }
                v.sort();
                ctx.sub = Some(Sub::ChooseVal { player: chooser, kind: 5, texts: v.clone(), players: vec![] });
                self.pending = Some(Decision {
                    player: chooser,
                    kind: DecisionKind::Pick {
                        prompt: format!("{name}: choose a land type"),
                        source: Some(source),
                        options: v.iter().map(|t| Opt::Text { text: t.clone() }).collect(),
                        min: 1,
                        max: 1,
                        cancellable: false,
                        hidden: false,
                        purpose: PickPurpose::Generic,
                    },
                });
                Exec::Wait
            }
            Effect::SetColorsOfChoice { who, what, duration } => {
                if let Some(Sub::ChooseVal { .. }) = ctx.sub.clone() {
                    ctx.sub = None;
                    let cols: Vec<Color> = match self.answer.take() {
                        Some(Choice::Pick { indices }) => indices.iter().filter_map(|i| Color::ALL.get(*i).copied()).collect(),
                        _ => vec![Color::ALL[0]],
                    };
                    let cs = {
                        let c = ec!();
                        self.sel_cards(what, &c)
                    };
                    for id in cs {
                        self.add_effect(source, controller, &[id], Modif::Colors(ColorSet::of(&cols)), *duration);
                    }
                    return Exec::Done;
                }
                let chooser = {
                    let c = ec!();
                    self.sel_players(who, &c).first().copied()
                };
                let Some(chooser) = chooser else { return Exec::Done };
                let name = self.def_of(source).name.clone();
                ctx.sub = Some(Sub::ChooseVal { player: chooser, kind: 4, texts: vec![], players: vec![] });
                self.pending = Some(Decision {
                    player: chooser,
                    kind: DecisionKind::Pick {
                        prompt: format!("{name}: choose one or more colors"),
                        source: Some(source),
                        options: Color::ALL.iter().map(|c| Opt::Color { c: *c }).collect(),
                        min: 1,
                        max: 5,
                        cancellable: false,
                        hidden: false,
                        purpose: PickPurpose::Generic,
                    },
                });
                Exec::Wait
            }
            Effect::MoveUnder { what, player, tapped } => {
                let (cs, ps) = {
                    let c = ec!();
                    (self.sel_cards(what, &c), self.sel_players(player, &c))
                };
                let Some(&p) = ps.first() else { return Exec::Done };
                let mut moved = vec![];
                for id in cs {
                    self.move_card(id, Zone::Battlefield, MoveOpts { tapped: *tapped, controller: Some(p), ..Default::default() });
                    if self.cards[id.0 as usize].zone == Zone::Battlefield {
                        moved.push(Obj::card(id));
                    }
                }
                ctx.remembered = moved;
                Exec::Done
            }
            Effect::Exert { what } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                for id in cs {
                    if self.cards[id.0 as usize].zone == Zone::Battlefield {
                        self.cards[id.0 as usize].skip_untap = true;
                        // an exerted creature doesn't untap during its controller's next untap step (a stolen one untaps in its owner's)
                        self.cards[id.0 as usize].skip_untap_by = if id == source { Some(controller) } else { None };
                        self.emit(Event::Exerted { card: id });
                    }
                }
                Exec::Done
            }
            Effect::ShieldFromAttacks { tax } => {
                let who = ec!().controller;
                if *tax == 0 {
                    self.players[who as usize].shield_bar = true;
                } else {
                    self.players[who as usize].shield_tax += *tax;
                }
                Exec::Done
            }
            Effect::CantAttackPlayerThisTurn { player, attacker, planeswalkers, only_permanents } => {
                let (whos, ps, ctl, src) = {
                    let c = ec!();
                    (match attacker { Some(a) => self.sel_players(a, &c), None => vec![c.controller] }, self.sel_players(player, &c), c.controller, c.source)
                };
                if let Some(f) = only_permanents {
                    for who in whos {
                        let e = (who, ctl, src, (**f).clone());
                        if !self.no_attack_perms.contains(&e) {
                            self.no_attack_perms.push(e);
                        }
                    }
                    return Exec::Done;
                }
                for who in whos {
                    for &q in &ps {
                        if !self.no_attack_pairs.contains(&(who, q)) {
                            self.no_attack_pairs.push((who, q));
                        }
                        if *planeswalkers && !self.no_attack_pw_pairs.contains(&(who, q)) {
                            self.no_attack_pw_pairs.push((who, q));
                        }
                    }
                }
                Exec::Done
            }
            Effect::AttackOnlyNearestUntilNextTurn => {
                let right = self.cards[source.0 as usize].chosen_type.map_or(false, |t| t.name() == "Right");
                self.attack_nearest.retain(|e| e.0 != controller);
                self.attack_nearest.push((controller, right));
                Exec::Done
            }
            Effect::Empower { n } => {
                let k = {
                    let c = ec!();
                    self.eval_amount(n, &c)
                };
                let jace = Sym::new("Jace");
                let find = |g: &Game| g.battlefield.iter().copied().find(|id| {
                    let c = &g.cards[id.0 as usize];
                    c.token && c.cur.controller == controller && c.cur.types.has(CardType::Planeswalker) && c.cur.subtypes.contains(&jace)
                });
                let mut target = find(self);
                if target.is_none() {
                    let make = Effect::CreateToken { token: "Jace 0 Blue Planeswalker".into(), n: Amount::Fixed { n: 1 }, controller: Sel::You, tapped: false, attacking: false, attach_to: None, set_pt: None, attack_at: None, choose_defender: false };
                    match self.exec_inner(ctx, &make) {
                        Exec::Done => {}
                        other => return other,
                    }
                    target = find(self);
                }
                if let Some(t) = target {
                    if k > 0 {
                        self.add_counters_to(Obj::card(t), CounterKind::Loyalty, k);
                    }
                }
                Exec::Done
            }
            Effect::Balance { filter, hand } => {
                if let Some(Sub::Sacrifice { players, n: ns, filter, idx }) = ctx.sub.clone() {
                    return self.continue_sacrifice(ctx, players, ns, filter, idx);
                }
                if let Some(Sub::Discard { players, n: ns, filter, idx }) = ctx.sub.clone() {
                    return self.continue_discard(ctx, players, ns, filter, idx);
                }
                let ps = {
                    let c = ec!();
                    self.sel_players(&Sel::EachPlayer, &c)
                };
                let counts: Vec<i32> = ps
                    .iter()
                    .map(|&p| {
                        if *hand {
                            self.eligible_in_hand(p, filter, source).len() as i32
                        } else {
                            let mut f = filter.clone();
                            f.controller = PlayerFilter::Any;
                            self.battlefield.iter().filter(|id| self.cards[id.0 as usize].cur.controller == p && self.matches_card(&f, **id, source, p)).count() as i32
                        }
                    })
                    .collect();
                let min = counts.iter().copied().min().unwrap_or(0);
                let ns: Vec<i32> = counts.iter().map(|c| c - min).collect();
                if *hand {
                    self.continue_discard(ctx, ps, ns, filter.clone().into_filter(), 0)
                } else {
                    self.continue_sacrifice(ctx, ps, ns, filter.clone().into_filter(), 0)
                }
            }
            Effect::SkipNextUntap { what, of_targets } => {
                let (cs, step_of) = {
                    let c = ec!();
                    (self.sel_cards(what, &c), if *of_targets { self.sel_players(&Sel::Targets, &c).first().copied() } else { None })
                };
                for id in cs {
                    if self.cards[id.0 as usize].zone == Zone::Battlefield {
                        self.cards[id.0 as usize].skip_untap = true;
                        // "doesn't untap during your next untap step" on itself (exert): only the ability controller's untap step is skipped
                        self.cards[id.0 as usize].skip_untap_by = if id == source { Some(controller) } else { step_of };
                    }
                }
                Exec::Done
            }
            Effect::MarkExiled { mark } => {
                if self.cards[source.0 as usize].zone == Zone::Exile {
                    self.cards[source.0 as usize].exile_mark = Some(*mark);
                    self.cards[source.0 as usize].exile_mark_turn = self.turn;
                    self.layers_dirty = true;
                    if *mark == ExileMark::Foretold {
                        let owner = self.cards[source.0 as usize].owner;
                        self.foretold_log.push(owner);
                        self.emit(Event::Foretold { card: source });
                    }
                    if *mark == ExileMark::Plotted {
                        self.emit(Event::Plotted { card: source });
                    }
                }
                Exec::Done
            }
            Effect::SetForetellCost { reduce } => {
                if self.cards[source.0 as usize].zone == Zone::Exile {
                    self.cards[source.0 as usize].exile_cast_cost = Some(self.reduced_cost_text(source, reduce));
                }
                Exec::Done
            }
            Effect::ForetellExile { what, reduce } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                for id in cs {
                    if self.cards[id.0 as usize].zone != Zone::Hand {
                        continue;
                    }
                    self.move_card(id, Zone::Exile, Default::default());
                    if self.cards[id.0 as usize].zone == Zone::Exile {
                        let owner = self.cards[id.0 as usize].owner;
                        self.cards[id.0 as usize].exile_mark = Some(ExileMark::Foretold);
                        self.cards[id.0 as usize].exile_mark_turn = self.turn;
                        self.cards[id.0 as usize].exile_cast_cost = Some(self.reduced_cost_text(id, reduce));
                        self.foretold_log.push(owner);
                        self.emit(Event::Foretold { card: id });
                        self.layers_dirty = true;
                    }
                }
                Exec::Done
            }
            Effect::PlotCards { what } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                for id in cs {
                    if self.cards[id.0 as usize].zone == Zone::Exile {
                        let owner = self.cards[id.0 as usize].owner;
                        let turn = self.turn;
                        let c = &mut self.cards[id.0 as usize];
                        c.exile_mark = Some(ExileMark::Plotted);
                        c.exile_mark_turn = turn;
                        c.play_until = u32::MAX;
                        c.play_src = None;
                        c.play_by = owner;
                        c.play_free = true;
                        c.play_cond = Some(Condition::All { cs: vec![Condition::ExileMarked { mark: ExileMark::Plotted }, Condition::SorcerySpeed] });
                        self.emit(Event::Plotted { card: id });
                    }
                }
                Exec::Done
            }
            Effect::MarkSuspendHaste => {
                if self.cards[source.0 as usize].cur.types.has(CardType::Creature) || self.def_of(source).types.has(CardType::Creature) {
                    self.cards[source.0 as usize].free_haste = true;
                }
                Exec::Done
            }
            Effect::MarkCastHaste { what } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                for id in cs {
                    if self.def_of(id).types.has(CardType::Creature) {
                        self.cards[id.0 as usize].cast_haste = true;
                    }
                }
                Exec::Done
            }
            Effect::MarkHasteSacRider { what } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                for id in cs {
                    if self.def_of(id).types.has(CardType::Creature) {
                        self.cards[id.0 as usize].haste_sac_rider = true;
                    }
                }
                Exec::Done
            }
            Effect::BecomePrepared { what } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                for id in cs {
                    let (zone, owner, prepared, def) = {
                        let c = &self.cards[id.0 as usize];
                        (c.zone, c.owner, c.prepared_spell.is_some(), c.def)
                    };
                    let d = self.db.def(def);
                    if zone != Zone::Battlefield || prepared || !d.prepare {
                        continue;
                    }
                    let Some(back) = d.other_face else { continue };
                    let t = self.create_card(back, owner, Zone::Exile, true);
                    self.cards[t.0 as usize].front = back;
                    self.cards[t.0 as usize].prepare_of = Some(id);
                    self.cards[id.0 as usize].prepared_spell = Some(t);
                }
                Exec::Done
            }
            Effect::Unprepare { what } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                for id in cs {
                    self.unprepare(id);
                }
                Exec::Done
            }
            Effect::NameAndRevealTop { kind } => {
                let (mut queue, mut done, options) = match ctx.sub.clone() {
                    Some(Sub::NameEach { queue, done, options }) => (queue, done, options),
                    _ => {
                        let mut names = self.name_options(*kind);
                        names.sort();
                        let n = self.players.len() as u8;
                        let queue: Vec<PlayerId> = (0..n).map(|i| (controller + i) % n).filter(|p| self.alive(*p)).collect();
                        (queue, vec![], names)
                    }
                };
                if matches!(ctx.sub, Some(Sub::NameEach { .. })) {
                    ctx.sub = None;
                    let idx = match self.answer.take() {
                        Some(Choice::Pick { indices }) => indices.first().copied().unwrap_or(0),
                        _ => 0,
                    };
                    let p = queue.remove(0);
                    if let Some(n) = options.get(idx) {
                        done.push((p, n.clone()));
                    }
                }
                if !queue.is_empty() && !options.is_empty() {
                    let decider = queue[0];
                    ctx.sub = Some(Sub::NameEach { queue, done, options: options.clone() });
                    self.pending = Some(Decision {
                        player: decider,
                        kind: DecisionKind::Pick {
                            prompt: format!("{}: choose a card name", self.def_of(source).name),
                            source: Some(source),
                            options: options.into_iter().map(|t| Opt::Text { text: t }).collect(),
                            min: 1,
                            max: 1,
                            cancellable: false,
                            hidden: true,
                            purpose: PickPurpose::Generic,
                        },
                    });
                    return Exec::Wait;
                }
                // every player has named a card: each reveals the top card of their library
                let order: Vec<PlayerId> = {
                    let n = self.players.len() as u8;
                    (0..n).map(|i| (controller + i) % n).filter(|p| self.alive(*p)).collect()
                };
                for p in order {
                    let Some(&top) = self.players[p as usize].library.last() else { continue };
                    let hit = done.iter().any(|(q, n)| *q == p && *n == self.def_of(top).name);
                    if hit {
                        self.move_card(top, Zone::Hand, Default::default());
                    } else {
                        self.move_card(top, Zone::Library, MoveOpts { bottom: true, ..Default::default() });
                    }
                }
                Exec::Done
            }
            Effect::ChooseCardName { kind, who, second, from_opponent_hands } => {
                if let Some(Sub::NamePick { options }) = ctx.sub.clone() {
                    ctx.sub = None;
                    let idx = match self.answer.take() {
                        Some(Choice::Pick { indices }) => indices.first().copied().unwrap_or(0),
                        _ => 0,
                    };
                    if let Some(n) = options.get(idx) {
                        if *second {
                            self.cards[source.0 as usize].chosen_name2 = Some(n.clone());
                        } else {
                            self.cards[source.0 as usize].chosen_name = Some(n.clone());
                            self.cards[source.0 as usize].chosen_name2 = None;
                        }
                        self.layers_dirty = true;
                    }
                    return Exec::Done;
                }
                let chooser = match who {
                    Some(w) => {
                        let c = ec!();
                        self.sel_players(w, &c).first().copied().unwrap_or(controller)
                    }
                    None => controller,
                };
                let mut names: Vec<String> = if *from_opponent_hands {
                    let mut v: Vec<String> = vec![];
                    for o in self.opponents(chooser) {
                        for id in &self.players[o as usize].hand {
                            let d = self.def_of(*id);
                            if !d.has_type(CardType::Land) {
                                v.push(d.name.clone());
                            }
                        }
                    }
                    v
                } else {
                    self.name_options(*kind)
                };
                names.sort();
                names.dedup();
                if names.is_empty() {
                    return Exec::Done;
                }
                ctx.sub = Some(Sub::NamePick { options: names.clone() });
                self.pending = Some(Decision {
                    player: chooser,
                    kind: DecisionKind::Pick {
                        prompt: format!("{}: choose a card name", self.def_of(source).name),
                        source: Some(source),
                        options: names.into_iter().map(|t| Opt::Text { text: t }).collect(),
                        min: 1,
                        max: 1,
                        cancellable: false,
                        hidden: false,
                        purpose: PickPurpose::Generic,
                    },
                });
                Exec::Wait
            }
            Effect::CopyNamedCard { names, once_each } => {
                if let Some(Sub::NamePick { options }) = ctx.sub.clone() {
                    ctx.sub = None;
                    let idx = match self.answer.take() {
                        Some(Choice::Pick { indices }) => indices.first().copied().unwrap_or(0),
                        _ => 0,
                    };
                    let Some(n) = options.get(idx).cloned() else { return Exec::Done };
                    if *once_each {
                        self.cards[source.0 as usize].names_used.push(n.clone());
                    }
                    let Some(def) = self.db.find(&n) else { return Exec::Done };
                    // a copy of the card from outside the game: a token-like card that ceases to exist when it leaves the stack
                    let id = self.create_card(def, controller, Zone::Exile, true);
                    self.players[controller as usize].exile.retain(|x| *x != id);
                    ctx.remembered = vec![Obj::card(id)];
                    let cast = Effect::CastFree { what: Sel::Remembered, may: true, decline_to_bottom: false, decline_to_hand: false, exile_after: false, copy: false, pay: true, by: None, any_mana: false, lands: false, alt: None, extra_mana: None, otherwise: vec![], suspend: false, lock_spells_after: false, on_cast: vec![], after_cast: vec![], bottom_after: false, copies: None, face_choice: false };
                    return Exec::Push(Frame { list: Arc::new(vec![cast]), pc: 0, repeat: 0, each: vec![], each_active: false, tbase: ctx.cur_tbase });
                }
                let used = self.cards[source.0 as usize].names_used.clone();
                let options: Vec<String> = names.iter().filter(|n| !*once_each || !used.contains(n)).cloned().collect();
                if options.is_empty() {
                    return Exec::Done;
                }
                ctx.sub = Some(Sub::NamePick { options: options.clone() });
                self.pending = Some(Decision {
                    player: controller,
                    kind: DecisionKind::Pick {
                        prompt: format!("{}: choose a card name", self.def_of(source).name),
                        source: Some(source),
                        options: options.into_iter().map(|t| Opt::Text { text: t }).collect(),
                        min: 1,
                        max: 1,
                        cancellable: false,
                        hidden: false,
                        purpose: PickPurpose::Generic,
                    },
                });
                Exec::Wait
            }
            Effect::ExileFromZones { who, zones, filter, max } => {
                let (ps, cap) = {
                    let c = ec!();
                    (self.sel_players(who, &c), max.as_ref().map(|m| self.eval_amount(m, &c).max(0) as usize))
                };
                let mut gone: Vec<Obj> = vec![];
                for p in ps {
                    let mut f = filter.clone();
                    f.controller = PlayerFilter::Any;
                    let mut pool: Vec<CardId> = vec![];
                    for z in zones {
                        let list: Vec<CardId> = match z {
                            Zone::Hand => self.players[p as usize].hand.clone(),
                            Zone::Graveyard => self.players[p as usize].graveyard.clone(),
                            Zone::Library => self.players[p as usize].library.clone(),
                            Zone::Exile => self.players[p as usize].exile.clone(),
                            _ => vec![],
                        };
                        pool.extend(list.into_iter().filter(|id| self.matches_card(&f, *id, source, controller)));
                    }
                    if let Some(k) = cap {
                        pool.truncate(k.saturating_sub(gone.len()));
                    }
                    for id in pool {
                        if self.move_card(id, Zone::Exile, Default::default()) {
                            gone.push(Obj::card(id));
                        }
                    }
                    if zones.contains(&Zone::Library) {
                        self.shuffle_library(p);
                    }
                }
                ctx.remembered = gone;
                Exec::Done
            }
            Effect::SacrificeAny { filter, by } => {
                let controller = match by {
                    Some(sel) => {
                        let c = ec!();
                        self.sel_players(sel, &c).first().copied().unwrap_or(controller)
                    }
                    None => controller,
                };
                if let Some(Sub::SacAny { options }) = ctx.sub.clone() {
                    ctx.sub = None;
                    let mut done: Vec<Obj> = vec![];
                    let (mut tp, mut tt) = (0i32, 0i32);
                    if let Some(Choice::Pick { indices }) = self.answer.take() {
                        for i in indices {
                            if let Some(&id) = options.get(i) {
                                if self.cards[id.0 as usize].zone == Zone::Battlefield && !done.contains(&Obj::card(id)) {
                                    tp += self.power_of(id).max(0);
                                    tt += self.cards[id.0 as usize].cur.toughness.max(0);
                                    self.emit(Event::Sacrificed { card: id, player: controller });
                                    self.move_card(id, Zone::Graveyard, Default::default());
                                    done.push(Obj::card(id));
                                }
                            }
                        }
                    }
                    self.cards[source.0 as usize].sacrificed_any = done.len() as u32;
                    self.cards[source.0 as usize].sacrificed_any_power = tp;
                    self.cards[source.0 as usize].sacrificed_any_toughness = tt;
                    ctx.remembered = done;
                    return Exec::Done;
                }
                let options = self.eligible_permanents(controller, filter, source);
                ctx.remembered = vec![];
                if options.is_empty() {
                    return Exec::Done;
                }
                ctx.sub = Some(Sub::SacAny { options: options.clone() });
                self.pending = Some(Decision {
                    player: controller,
                    kind: DecisionKind::Pick {
                        prompt: format!("{}: sacrifice any number", self.def_of(source).name),
                        source: Some(source),
                        options: options.into_iter().map(|c| Opt::Obj { obj: Obj::card(c) }).collect(),
                        min: 0,
                        max: 255,
                        cancellable: false,
                        hidden: false,
                        purpose: PickPurpose::Sacrifice,
                    },
                });
                Exec::Wait
            }
            Effect::Suspect { what } | Effect::Unsuspect { what } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                let on = matches!(e, Effect::Suspect { .. });
                for id in cs {
                    if self.cards[id.0 as usize].zone == Zone::Battlefield && !(on && self.cards[id.0 as usize].cur.cant_become_suspected) {
                        self.cards[id.0 as usize].suspected = on;
                    }
                }
                self.layers_dirty = true;
                Exec::Done
            }
            Effect::RemoveFromCombat { what, unblock_sole } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                if let Some(cb) = self.combat.as_mut() {
                    for id in cs {
                        cb.attackers.retain(|a| a.card != id);
                        for a in cb.attackers.iter_mut() {
                            let had = a.blockers.contains(&id);
                            a.blockers.retain(|b| *b != id);
                            if *unblock_sole && had && a.blockers.is_empty() {
                                a.blocked = false;
                            }
                        }
                    }
                }
                Exec::Done
            }
            Effect::SwitchBlockers { what } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                if cs.len() < 2 || cs[0] == cs[1] {
                    return Exec::Done;
                }
                let (a, b) = (cs[0], cs[1]);
                let Some(cb) = self.combat.as_ref() else { return Exec::Done };
                let (Some(ai), Some(bi)) = (cb.attackers.iter().position(|x| x.card == a && x.blocked), cb.attackers.iter().position(|x| x.card == b && x.blocked)) else { return Exec::Done };
                let ab = cb.attackers[ai].blockers.clone();
                let bb = cb.attackers[bi].blockers.clone();
                if !ab.iter().all(|x| self.can_block_attacker(*x, b)) || !bb.iter().all(|x| self.can_block_attacker(*x, a)) {
                    return Exec::Done;
                }
                if let Some(cb) = self.combat.as_mut() {
                    let only_a: Vec<CardId> = ab.iter().copied().filter(|x| !bb.contains(x)).collect();
                    let only_b: Vec<CardId> = bb.iter().copied().filter(|x| !ab.contains(x)).collect();
                    cb.attackers[ai].blockers.retain(|x| !only_a.contains(x));
                    cb.attackers[bi].blockers.retain(|x| !only_b.contains(x));
                    for x in only_b {
                        cb.attackers[ai].blockers.push(x);
                    }
                    for x in only_a {
                        cb.attackers[bi].blockers.push(x);
                    }
                }
                Exec::Done
            }
            Effect::RestartGame => {
                let keep: Vec<CardId> = {
                    let c = ec!();
                    self.sel_cards(&Sel::ExiledWithThis, &c)
                };
                let keep: Vec<CardId> = keep
                    .into_iter()
                    .filter(|id| {
                        let d = self.def_of(*id);
                        !self.cards[id.0 as usize].token && !d.types.has(CardType::Instant) && !d.types.has(CardType::Sorcery) && !d.subtypes.iter().any(|s| *s == Sym::new("Aura"))
                    })
                    .collect();
                self.restart_pending = Some((controller, keep));
                Exec::Done
            }
            Effect::SetChosenCard { what, on } => {
                let (cs, holder) = {
                    let c = ec!();
                    (self.sel_cards(what, &c), on.as_ref().and_then(|o| self.sel_cards(o, &c).first().copied()).unwrap_or(source))
                };
                if let Some(&id) = cs.first() {
                    self.cards[holder.0 as usize].chosen_card = Some(id);
                    self.layers_dirty = true;
                }
                Exec::Done
            }
            Effect::CoffinExile { what } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                let Some(&id) = cs.first() else { return Exec::Done };
                if self.cards[id.0 as usize].zone != Zone::Battlefield {
                    return Exec::Done;
                }
                let aura = Sym::new("Aura");
                let auras: Vec<CardId> = self.battlefield.iter().copied().filter(|a| self.cards[a.0 as usize].attached_to == Some(id) && self.cards[a.0 as usize].cur.subtypes.contains(&aura)).collect();
                let counters: Vec<(CounterKind, u16)> = self.cards[id.0 as usize].counters.iter().copied().filter(|(_, n)| *n > 0).collect();
                self.coffin_notes.retain(|n| n.0 != source);
                let mut az: Vec<CardId> = vec![];
                for a in auras {
                    if self.move_card(a, Zone::Exile, Default::default()) {
                        az.push(a);
                    }
                }
                if self.move_card(id, Zone::Exile, Default::default()) {
                    let auras: Vec<(CardId, u32)> = az.iter().map(|a| (*a, self.cards[a.0 as usize].zcc)).collect();
                    self.coffin_notes.push((source, self.cards[source.0 as usize].zcc, id, self.cards[id.0 as usize].zcc, counters, auras));
                }
                Exec::Done
            }
            Effect::CoffinReturn => {
                let Some(pos) = self.coffin_notes.iter().position(|n| n.0 == source) else { return Exec::Done };
                let (_, _, cr, crz, counters, auras) = self.coffin_notes.remove(pos);
                let ok = self.cards[cr.0 as usize].zone == Zone::Exile && self.cards[cr.0 as usize].zcc == crz && !self.cards[cr.0 as usize].token;
                if !ok {
                    return Exec::Done;
                }
                self.move_card(cr, Zone::Battlefield, MoveOpts { tapped: true, ..Default::default() });
                if self.cards[cr.0 as usize].zone != Zone::Battlefield {
                    return Exec::Done;
                }
                for (k, n) in counters {
                    self.add_counters_to(Obj::card(cr), k, n as i32);
                }
                self.layers_dirty = true;
                for (a, z) in auras {
                    if self.cards[a.0 as usize].zone == Zone::Exile && self.cards[a.0 as usize].zcc == z && self.aura_hosts(a).map_or(false, |h| h.contains(&cr)) {
                        self.move_card(a, Zone::Battlefield, MoveOpts { attach_to: Some(cr), ..Default::default() });
                    }
                }
                Exec::Done
            }
            Effect::ReturnAsAura { what, host } => {
                let (aura, hosts) = if let Some(Sub::AsAuraHost { aura, hosts }) = ctx.sub.clone() {
                    ctx.sub = None;
                    let h = match self.answer.take() {
                        Some(Choice::Pick { indices }) => indices.first().and_then(|i| hosts.get(*i)).copied().or(hosts.first().copied()),
                        _ => hosts.first().copied(),
                    };
                    (aura, h.into_iter().collect::<Vec<_>>())
                } else {
                    let cs = {
                        let c = ec!();
                        self.sel_cards(what, &c)
                    };
                    let Some(&id) = cs.first() else { return Exec::Done };
                    if self.cards[id.0 as usize].zone != Zone::Graveyard {
                        return Exec::Done;
                    }
                    let hosts: Vec<CardId> = self.battlefield.iter().copied().filter(|h| *h != id && self.matches_card(host, *h, id, controller)).collect();
                    if hosts.len() > 1 {
                        ctx.sub = Some(Sub::AsAuraHost { aura: id, hosts: hosts.clone() });
                        self.pending = Some(Decision {
                            player: controller,
                            kind: DecisionKind::Pick {
                                prompt: format!("{}: choose what {} enchants", self.def_of(source).name, self.def_of(id).name),
                                source: Some(source),
                                options: hosts.iter().map(|c| Opt::Obj { obj: Obj::card(*c) }).collect(),
                                min: 1,
                                max: 1,
                                cancellable: false,
                                hidden: false,
                                purpose: PickPurpose::Generic,
                            },
                        });
                        return Exec::Wait;
                    }
                    (id, hosts)
                };
                if let Some(&h) = hosts.first() {
                    if self.cards[aura.0 as usize].zone == Zone::Graveyard {
                        self.move_card(aura, Zone::Battlefield, MoveOpts { attach_to: Some(h), ..Default::default() });
                        if self.cards[aura.0 as usize].zone == Zone::Battlefield {
                            self.cards[aura.0 as usize].as_aura = true;
                            self.layers_dirty = true;
                            self.recompute_layers();
                        }
                    }
                }
                Exec::Done
            }
            Effect::MayCastFromGraveyardOnce { filter, finality, subtype } => {
                self.gy_once_grants.push((controller, self.turn, filter.clone().into_filter(), source, *finality, subtype.as_ref().map(|s| Sym::new(s))));
                Exec::Done
            }
            Effect::SwapBlocking { what } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                if cs.len() < 2 || cs[0] == cs[1] {
                    return Exec::Done;
                }
                let (x, y) = (cs[0], cs[1]);
                let Some(cb) = self.combat.as_ref() else { return Exec::Done };
                let ax: Vec<usize> = (0..cb.attackers.len()).filter(|i| cb.attackers[*i].blockers.contains(&x)).collect();
                let ay: Vec<usize> = (0..cb.attackers.len()).filter(|i| cb.attackers[*i].blockers.contains(&y)).collect();
                if ax.is_empty() && ay.is_empty() {
                    return Exec::Done;
                }
                if !ay.iter().all(|i| self.can_block_attacker(x, cb.attackers[*i].card)) || !ax.iter().all(|i| self.can_block_attacker(y, cb.attackers[*i].card)) {
                    return Exec::Done;
                }
                if let Some(cb) = self.combat.as_mut() {
                    for i in &ax {
                        cb.attackers[*i].blockers.retain(|b| *b != x);
                    }
                    for i in &ay {
                        cb.attackers[*i].blockers.retain(|b| *b != y);
                    }
                    for i in &ay {
                        cb.attackers[*i].blockers.push(x);
                    }
                    for i in &ax {
                        cb.attackers[*i].blockers.push(y);
                    }
                }
                Exec::Done
            }
            Effect::SwitchPT { what, duration } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                self.add_effect(source, controller, &cs, Modif::SwitchPT, *duration);
                Exec::Done
            }
            Effect::Restrict { who, what, duration } => {
                let ps = {
                    let c = ec!();
                    self.sel_players(who, &c)
                };
                if !ps.is_empty() {
                    let ts = self.next_ts();
                    let id = self.next_effect_id;
                    self.next_effect_id += 1;
                    let modif = match what {
                        PlayerRestrict::CantCast => Modif::NoCast,
                        PlayerRestrict::OneMoreSpell => {
                            let duration = if *duration == Duration::Instant { Duration::UntilEndOfTurn } else { *duration };
                            for p in ps {
                                let id = self.next_effect_id;
                                self.next_effect_id += 1;
                                let cap = self.players[p as usize].spells_cast + 1;
                                self.effects.push(ContEffect { id, ts, source, controller, affects: SmallVec::new(), players: std::iter::once(p).collect(), modif: Modif::SpellCap(cap), duration, created_turn: self.turn });
                            }
                            return Exec::Done;
                        }
                        PlayerRestrict::CantCastCreatures => Modif::NoCastCreatures,
                        PlayerRestrict::CantCastNoncreature => Modif::NoCastNoncreature,
                        PlayerRestrict::CantAttack => Modif::NoAttack,
                        PlayerRestrict::CantCastNotFromHand => Modif::NoCastNotFromHand,
                        PlayerRestrict::CantCastInstantSorcery => Modif::NoCastInstantSorcery,
                        PlayerRestrict::CantActivateNonMana => Modif::NoActivateNonMana,
                        PlayerRestrict::CantPlayLands => Modif::NoLandPlay,
                        PlayerRestrict::CantGainLife => Modif::NoLifeGain,
                        PlayerRestrict::CantLoseLife => Modif::NoLifeLoss,
                        PlayerRestrict::CantBecomeMonarch => Modif::NoMonarch,
                        PlayerRestrict::CantPlayFromGraveyard => Modif::NoGraveyardPlay,
                        PlayerRestrict::MayPlayOthersGraveyards => Modif::PlayOthersGraveyards,
                    };
                    let duration = if *duration == Duration::Instant { Duration::UntilEndOfTurn } else { *duration };
                    self.effects.push(ContEffect { id, ts, source, controller, affects: SmallVec::new(), players: ps.into_iter().collect(), modif, duration, created_turn: self.turn });
                }
                Exec::Done
            }
            Effect::StartEngines { who } => {
                let ps = {
                    let c = ec!();
                    self.sel_players(who, &c)
                };
                for p in ps {
                    if self.players[p as usize].speed == 0 {
                        self.players[p as usize].speed = 1;
                    }
                }
                Exec::Done
            }
            Effect::IncreaseSpeed { who } => {
                let ps = {
                    let c = ec!();
                    self.sel_players(who, &c)
                };
                for p in ps {
                    if self.players[p as usize].speed < 4 {
                        self.players[p as usize].speed += 1;
                    }
                }
                Exec::Done
            }
            Effect::DecreaseSpeed { who, only_if_highest } => {
                let ps = {
                    let c = ec!();
                    self.sel_players(who, &c)
                };
                for p in ps {
                    let mine = self.players[p as usize].speed;
                    let others = self.players.iter().enumerate().filter(|(i, _)| *i != p as usize).map(|(_, q)| q.speed).max().unwrap_or(0);
                    if mine > 1 && (!*only_if_highest || mine > others) {
                        self.players[p as usize].speed -= 1;
                    }
                }
                Exec::Done
            }
            Effect::Solve => {
                if self.cards[source.0 as usize].zone == Zone::Battlefield && !self.cards[source.0 as usize].solved {
                    self.cards[source.0 as usize].solved = true;
                    self.layers_dirty = true;
                    self.emit(Event::Solved { card: source });
                }
                Exec::Done
            }
            Effect::SetMark { mark, n, add, this_turn } => {
                {
                    let v = {
                        let c = ec!();
                        self.eval_amount(n, &c)
                    };
                    let old = self.mark_of(source, *mark);
                    let turn = if *this_turn { self.turn } else { 0 };
                    let marks = &mut self.cards[source.0 as usize].marks;
                    marks.retain(|(m, _, _)| m != mark);
                    marks.push((*mark, if *add { old + v } else { v }, turn));
                    self.layers_dirty = true;
                }
                Exec::Done
            }
            Effect::Harness => {
                if self.cards[source.0 as usize].zone == Zone::Battlefield {
                    self.cards[source.0 as usize].harnessed = true;
                    self.layers_dirty = true;
                }
                Exec::Done
            }
            Effect::BecomeRenowned => {
                if !self.cards[source.0 as usize].renowned && self.cards[source.0 as usize].zone == Zone::Battlefield {
                    self.cards[source.0 as usize].renowned = true;
                    self.layers_dirty = true;
                    self.emit(Event::Renowned { card: source });
                }
                Exec::Done
            }
            Effect::ExileInsteadOfDying { what } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                for id in cs {
                    if self.cards[id.0 as usize].zone == Zone::Battlefield {
                        self.cards[id.0 as usize].exile_if_dies = true;
                    }
                }
                Exec::Done
            }
            Effect::FaceDownAs { what, add_types, add_subtypes, not_creature } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                for id in cs {
                    let c = &mut self.cards[id.0 as usize];
                    if c.zone == Zone::Battlefield && c.face_down {
                        c.fd_extra = Some(Box::new((add_types.clone(), add_subtypes.clone(), *not_creature)));
                        self.layers_dirty = true;
                    }
                }
                Exec::Done
            }
            Effect::TurnFaceDown { what } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                for id in cs {
                    let c = &mut self.cards[id.0 as usize];
                    // an exiled card can be turned face down too (The Creation of Avacyn: the searched card)
                    if matches!(c.zone, Zone::Battlefield | Zone::Exile) && !c.face_down {
                        c.face_down = true;
                        if let Some(old) = c.fd_revert.take() {
                            c.def = old;
                            self.effects.retain(|e| !(e.duration == Duration::UntilAffectedTurnedFaceDown && e.affects.iter().any(|(x, _)| *x == id)));
                        }
                        self.layers_dirty = true;
                    }
                }
                Exec::Done
            }
            Effect::TurnFaceUp { megamorph, counters, what } => {
                let cs = match what {
                    Some(w) => {
                        let c = ec!();
                        self.sel_cards(w, &c)
                    }
                    None => vec![source],
                };
                for id in cs {
                    if self.cards[id.0 as usize].face_down && self.cards[id.0 as usize].zone == Zone::Exile {
                        // a face-down exiled card is simply revealed
                        self.cards[id.0 as usize].face_down = false;
                        continue;
                    }
                    if self.cards[id.0 as usize].face_down && self.cards[id.0 as usize].zone == Zone::Battlefield {
                        if self.cant_turn_face_up(id) {
                            continue;
                        }
                        self.cards[id.0 as usize].face_down = false;
                        self.layers_dirty = true;
                        if *megamorph || *counters > 0 {
                            self.cards[id.0 as usize].add_counters(CounterKind::PlusOnePlusOne, *counters as i32 + *megamorph as i32);
                        }
                        // recompute now: the face-up abilities must exist when "turned face up" triggers are checked
                        self.recompute_layers();
                        self.queue_face_up_choice(id);
                        self.emit(Event::TurnedFaceUp { card: id });
                    }
                }
                Exec::Done
            }
            Effect::ManifestDread { who } => {
                if let Some(Sub::Dread { player, cards }) = ctx.sub.clone() {
                    ctx.sub = None;
                    let idx = match self.answer.take() {
                        Some(Choice::Pick { indices }) => indices.first().copied().unwrap_or(0),
                        _ => 0,
                    };
                    ctx.remembered = vec![];
                    let mut binned: Vec<CardId> = vec![];
                    for (i, c) in cards.iter().enumerate() {
                        if i == idx {
                            self.move_card(*c, Zone::Battlefield, MoveOpts { controller: Some(player), face_down: true, ..Default::default() });
                            ctx.remembered = vec![Obj::card(*c)];
                        } else {
                            self.move_card(*c, Zone::Graveyard, Default::default());
                            if self.cards[c.0 as usize].zone == Zone::Graveyard {
                                binned.push(*c);
                            }
                        }
                    }
                    self.emit(Event::ManifestedDread { player, cards: binned });
                    return Exec::Done;
                }
                let Some(p) = ({
                    let c = ec!();
                    self.sel_players(who, &c).first().copied()
                }) else {
                    return Exec::Done;
                };
                let lib = &self.players[p as usize].library;
                let n = lib.len().min(2);
                if n == 0 {
                    return Exec::Done;
                }
                let cards: Vec<CardId> = lib[lib.len() - n..].iter().rev().copied().collect();
                if n == 1 {
                    self.move_card(cards[0], Zone::Battlefield, MoveOpts { controller: Some(p), face_down: true, ..Default::default() });
                    ctx.remembered = vec![Obj::card(cards[0])];
                    self.emit(Event::ManifestedDread { player: p, cards: vec![] });
                    return Exec::Done;
                }
                ctx.sub = Some(Sub::Dread { player: p, cards: cards.clone() });
                self.pending = Some(Decision {
                    player: p,
                    kind: DecisionKind::Pick {
                        prompt: "Manifest dread: choose a card to manifest".into(),
                        source: Some(source),
                        options: cards.iter().map(|c| Opt::Obj { obj: Obj::card(*c) }).collect(),
                        min: 1,
                        max: 1,
                        cancellable: false,
                        hidden: false,
                        purpose: PickPurpose::Generic,
                    },
                });
                Exec::Wait
            }
            Effect::Manifest { who, controller: by, what: Some(what), include_source } => {
                let (p, mut cs) = {
                    let c = ec!();
                    (self.sel_players(who, &c).first().copied().unwrap_or(controller), self.sel_cards(what, &c))
                };
                if *include_source && !cs.contains(&source) && self.cards[source.0 as usize].zone == Zone::Battlefield {
                    cs.push(source);
                }
                for &c in &cs {
                    self.move_card(c, Zone::Exile, Default::default());
                }
                cs.retain(|id| self.cards[id.0 as usize].zone == Zone::Exile);
                {
                    use rand::seq::SliceRandom;
                    cs.shuffle(&mut self.rng);
                }
                let mut made: Vec<Obj> = vec![];
                for c in cs {
                    self.move_card(c, Zone::Battlefield, MoveOpts { controller: Some(p), face_down: true, ..Default::default() });
                    made.push(Obj::card(c));
                }
                if !made.is_empty() {
                    ctx.remembered = made;
                }
                Exec::Done
            }
            Effect::Manifest { who, controller: by, what: None, .. } => {
                let (ps, ctl) = {
                    let c = ec!();
                    (self.sel_players(who, &c), by.as_ref().and_then(|s| self.sel_players(s, &c).first().copied()))
                };
                let mut made: Vec<Obj> = vec![];
                for p in ps {
                    if let Some(&top) = self.players[p as usize].library.last() {
                        self.move_card(top, Zone::Battlefield, MoveOpts { controller: Some(ctl.unwrap_or(p)), face_down: true, ..Default::default() });
                        made.push(Obj::card(top));
                    }
                }
                if !made.is_empty() {
                    ctx.remembered = made;
                }
                Exec::Done
            }
            Effect::Learn { plain, always_ask } => {
                if !*plain {
                    let mut found = None;
                    for r in self.active_replacements() {
                        if let Replace::Learn { instead } = &r.what {
                            if r.controller == controller {
                                found = Some((r.source, r.controller, instead.clone()));
                                break;
                            }
                        }
                    }
                    if let Some((src, ctrl, instead)) = found {
                        let eff = vec![Effect::May { who: None, remember_answer: false, effects: instead, otherwise: vec![Effect::Learn { plain: true, always_ask: *always_ask }] }];
                        self.dmg_followups.borrow_mut().push((src, ctrl, eff, 1, None, None, Some(controller), vec![]));
                        self.flush_followups();
                        return Exec::Done;
                    }
                }
                let rummage = if *always_ask {
                    serde_json::from_value(serde_json::json!({"type": "May", "effects": [
                        {"type": "Discard", "who": {"type": "You"}, "n": {"type": "Fixed", "n": 1}},
                        {"type": "If", "cond": {"type": "Compare", "a": {"type": "RememberedCount"}, "cmp": {"op": "Ge", "n": 1}}, "then": [{"type": "Draw", "who": {"type": "You"}, "n": {"type": "Fixed", "n": 1}}], "else": []}]}))
                        .unwrap()
                } else {
                    Effect::MayPay {
                        payer: Sel::You,
                        cost: vec![Cost::Discard { filter: (Filter::default()).into(), n: 1, random: false }],
                        effects: vec![Effect::Draw { who: Sel::You, n: Amount::Fixed { n: 1 }, plain: false }],
                    }
                };
                // the first option: reveal a Lesson card you own from outside the game (only asked when the sideboard holds one)
                let lessons = self.players[controller as usize].sideboard.iter().any(|&id| self.def_of(id).subtypes.contains(&Sym::new("Lesson")));
                let list = if lessons {
                    let lesson = Filter { subtypes: vec!["Lesson".to_string()], ..Default::default() };
                    vec![Effect::May {
                        who: None,
                        remember_answer: false,
                        effects: vec![Effect::FetchOutsideGame { who: Sel::You, filter: lesson.into(), reveal: true, max: 1 }],
                        otherwise: vec![rummage],
                    }]
                } else {
                    vec![rummage]
                };
                Exec::Push(Frame { list: Arc::new(list), pc: 0, repeat: 0, each: vec![], each_active: false, tbase: ctx.cur_tbase })
            }
            Effect::MarkZcc => {
                let c = &mut self.cards[source.0 as usize];
                c.zcc_mark = c.zcc + 1;
                Exec::Done
            }
            Effect::Meld { with, into, attacking } => {
                let me = &self.cards[source.0 as usize];
                if me.zone != Zone::Battlefield || me.cur.controller != controller || me.owner != controller {
                    return Exec::Done;
                }
                let partner = self.battlefield.iter().copied().find(|id| {
                    let c = &self.cards[id.0 as usize];
                    *id != source && c.owner == controller && c.cur.controller == controller && self.def_of(*id).name == *with
                });
                let (Some(partner), Some(def)) = (partner, self.db.find(into)) else { return Exec::Done };
                if *attacking && !(self.is_attacking(source) && self.is_attacking(partner)) {
                    return Exec::Done;
                }
                self.move_card(source, Zone::Exile, Default::default());
                self.move_card(partner, Zone::Exile, Default::default());
                let new = self.create_card(def, controller, Zone::Exile, true);
                self.players[controller as usize].exile.retain(|x| *x != new);
                self.entering_attacking.set(*attacking);
                self.move_card(new, Zone::Battlefield, MoveOpts { controller: Some(controller), tapped: *attacking, ..Default::default() });
                self.entering_attacking.set(false);
                self.meld_parts.push((new, source, partner));
                Exec::Done
            }
            Effect::DealDamageEvenly { to, amount, source: src } => {
                let (objs, n, src_card) = {
                    let c = ec!();
                    let src_card = src.as_ref().and_then(|s| self.sel_cards(s, &c).first().copied()).unwrap_or(source);
                    (self.sel_objs(to, &c), self.eval_amount(amount, &c), src_card)
                };
                if !objs.is_empty() {
                    let each = (n / objs.len() as i32).max(0);
                    if each > 0 {
                        for o in objs {
                            self.deal_damage(Some(src_card), o, each, false);
                        }
                    }
                }
                Exec::Done
            }
            Effect::DoubleManaPool { who } => {
                let ps = {
                    let c = ec!();
                    self.sel_players(who, &c)
                };
                for p in ps {
                    let pool = &mut self.players[p as usize].pool;
                    for t in crate::mana::ManaType::ALL {
                        let n = pool.get(t);
                        pool.add(t, n);
                    }
                }
                Exec::Done
            }
            Effect::ActivateLandManaAbilities { who } => {
                let ps = {
                    let c = ec!();
                    self.sel_players(who, &c)
                };
                for p in ps {
                    let mut done: Vec<CardId> = vec![];
                    for s in self.mana_sources(p) {
                        if done.contains(&s.card) || self.cards[s.card.0 as usize].zone != Zone::Battlefield || !self.cards[s.card.0 as usize].cur.types.has(CardType::Land) || !s.cost.is_empty() || s.ability >= crate::cast::RESTRICTED_BASE && s.ability < crate::cast::INTRINSIC_BASE {
                            continue;
                        }
                        let Some(t) = s.produces.first().copied() else { continue };
                        if self.activate_mana_ability(p, s.card, s.ability, Some(t)).is_ok() {
                            done.push(s.card);
                        }
                    }
                }
                Exec::Done
            }
            Effect::DrainManaPool { who, give_to, record } => {
                let mut drained_total = 0i32;
                let ps = {
                    let c = ec!();
                    self.sel_players(who, &c)
                };
                let taker = give_to.as_ref().and_then(|s| {
                    let c = ec!();
                    self.sel_players(s, &c).first().copied()
                });
                for p in ps {
                    if taker == Some(p) {
                        // the mana lost is added right back
                        continue;
                    }
                    if let Some(q) = taker {
                        let pool = self.players[p as usize].pool;
                        for t in crate::mana::ManaType::ALL {
                            let k = pool.0[t as usize];
                            if k > 0 {
                                self.players[q as usize].pool.add(t, k);
                            }
                        }
                        let rs: Vec<_> = self.players[p as usize].restricted.drain(..).collect();
                        let mut ts: Vec<bool> = self.players[p as usize].restricted_turn.drain(..).collect();
                        ts.resize(rs.len(), false);
                        let moved: Vec<_> = rs.into_iter().zip(ts).collect();
                        for (r, turn_only) in moved {
                            if r.is_some() {
                                let pl = &mut self.players[q as usize];
                                pl.restricted.push(r);
                                pl.restricted_turn.resize(pl.restricted.len() - 1, false);
                                pl.restricted_turn.push(turn_only);
                            }
                        }
                    }
                    let to_colorless = self.mana_becomes_colorless(p);
                    let to_red = !to_colorless && self.mana_becomes_red(p);
                    let to_black = !to_colorless && !to_red && self.mana_becomes_black(p);
                    let loss_life = !to_colorless && !to_red && !to_black && self.mana_loss_costs_life(p);
                    let pl = &mut self.players[p as usize];
                    let total = pl.pool.total() as u16;
                    drained_total += total as i32;
                    pl.pool = Default::default();
                    if to_colorless {
                        pl.pool.add(crate::mana::ManaType::C, total);
                    } else if to_red {
                        pl.pool.add(crate::mana::ManaType::R, total);
                    } else if to_black {
                        pl.pool.add(crate::mana::ManaType::B, total);
                    }
                    pl.pool_keep = Default::default();
                    pl.pool_turn = Default::default();
                    pl.restricted.clear();
                    pl.restricted_turn.clear();
                    if loss_life && total > 0 {
                        self.lose_life(p, total as i32);
                    }
                }
                if *record {
                    match self.mana_drained_by.iter_mut().find(|(s, _)| *s == source) {
                        Some(e) => e.1 += drained_total,
                        None => self.mana_drained_by.push((source, drained_total)),
                    }
                }
                Exec::Done
            }
            Effect::LookAtCards { what } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                for id in cs {
                    self.cards[id.0 as usize].known_to |= 1u8 << (controller as u32 & 7);
                    self.log(format!("{} looks at {}", self.def_of(source).name, self.def_of(id).name));
                }
                Exec::Done
            }
            Effect::LookAtHand { who } => {
                let ps = {
                    let c = ec!();
                    self.sel_players(who, &c)
                };
                for p in ps {
                    for id in self.players[p as usize].hand.clone() {
                        self.cards[id.0 as usize].known_to |= 1u8 << (controller as u32 & 7);
                    }
                    self.log(format!("{} looks at player {}'s hand", self.def_of(source).name, p));
                }
                Exec::Done
            }
            Effect::Cloak { who, what: Some(what), tapped, .. } => {
                let (p, mut cs) = {
                    let c = ec!();
                    (self.sel_players(who, &c).first().copied().unwrap_or(controller), self.sel_cards(what, &c))
                };
                cs.retain(|id| matches!(self.cards[id.0 as usize].zone, Zone::Exile | Zone::Hand));
                {
                    use rand::seq::SliceRandom;
                    cs.shuffle(&mut self.rng);
                }
                let mut made: Vec<Obj> = vec![];
                for c in cs {
                    self.move_card(c, Zone::Battlefield, MoveOpts { controller: Some(p), face_down: true, tapped: *tapped, ..Default::default() });
                    if self.cards[c.0 as usize].zone == Zone::Battlefield {
                        self.cards[c.0 as usize].cloaked = true;
                        made.push(Obj::card(c));
                    }
                }
                if !made.is_empty() {
                    ctx.remembered = made;
                }
                Exec::Done
            }
            Effect::Cloak { who, controller: by, .. } => {
                let (ps, ctl) = {
                    let c = ec!();
                    (self.sel_players(who, &c), by.as_ref().and_then(|s| self.sel_players(s, &c).first().copied()))
                };
                let mut made: Vec<Obj> = vec![];
                for p in ps {
                    if let Some(&top) = self.players[p as usize].library.last() {
                        self.move_card(top, Zone::Battlefield, MoveOpts { controller: Some(ctl.unwrap_or(p)), face_down: true, ..Default::default() });
                        self.cards[top.0 as usize].cloaked = true;
                        made.push(Obj::card(top));
                    }
                }
                if !made.is_empty() {
                    ctx.remembered = made;
                }
                Exec::Done
            }
            Effect::MarkExileOnLeave { what } => {
                let cs = match what {
                    Some(w) => {
                        let c = ec!();
                        self.sel_cards(w, &c)
                    }
                    None => vec![source],
                };
                for id in cs {
                    if self.cards[id.0 as usize].zone == Zone::Battlefield {
                        self.cards[id.0 as usize].exile_on_leave = true;
                    }
                }
                Exec::Done
            }
            Effect::Monstrosity { n } => {
                if !self.cards[source.0 as usize].monstrous && self.cards[source.0 as usize].zone == Zone::Battlefield {
                    let k = {
                        let c = ec!();
                        self.eval_amount(n, &c)
                    };
                    self.add_counters_to(Obj::card(source), CounterKind::PlusOnePlusOne, k);
                    self.cards[source.0 as usize].monstrous = true;
                    self.layers_dirty = true;
                    self.emit(Event::Monstrous { card: source, n: k });
                }
                Exec::Done
            }
            Effect::FlipCoin { who, win, lose } => {
                use rand::Rng;
                let flipper = match who {
                    Some(w) => {
                        let c = ec!();
                        self.sel_players(w, &c).first().copied().unwrap_or(controller)
                    }
                    None => controller,
                };
                let heads = self.flip_coin_for(flipper);
                let list = if heads { win } else { lose };
                if list.is_empty() {
                    Exec::Done
                } else {
                    Exec::Push(Frame { list: Arc::new(list.clone()), pc: 0, repeat: 0, each: vec![], each_active: false, tbase: ctx.cur_tbase })
                }
            }
            Effect::ResetFlipTally => {
                self.flip_wins = 0;
                self.flip_count = 0;
                self.flip_lost = false;
                self.last_flip_won = true;
                self.last_cost_paid = true;
                Exec::Done
            }
            Effect::Bolster { n, remember } => {
                // several creatures tied for the least toughness: the controller chooses which one gets the counters
                let picked = if let Some(Sub::BolsterPick { options, n: k }) = ctx.sub.clone() {
                    ctx.sub = None;
                    let i = match self.answer.take() {
                        Some(Choice::Pick { indices }) => indices.first().copied().unwrap_or(0),
                        _ => 0,
                    };
                    options.get(i).or(options.first()).copied().filter(|id| self.cards[id.0 as usize].zone == Zone::Battlefield).map(|id| (id, k))
                } else {
                    let k = {
                        let c = ec!();
                        self.eval_amount(n, &c)
                    };
                    let mine: Vec<CardId> = self
                        .battlefield
                        .iter()
                        .copied()
                        .filter(|id| self.cards[id.0 as usize].cur.controller == controller && self.cards[id.0 as usize].cur.types.has(CardType::Creature))
                        .collect();
                    let least = mine.iter().map(|id| self.cards[id.0 as usize].cur.toughness).min();
                    let tied: Vec<CardId> = mine.iter().copied().filter(|id| Some(self.cards[id.0 as usize].cur.toughness) == least).collect();
                    if tied.len() > 1 {
                        ctx.sub = Some(Sub::BolsterPick { options: tied.clone(), n: k });
                        self.pending = Some(Decision {
                            player: controller,
                            kind: DecisionKind::Pick {
                                prompt: format!("Bolster {k}: choose a creature with the least toughness"),
                                source: Some(source),
                                options: tied.iter().map(|c| Opt::Obj { obj: Obj::card(*c) }).collect(),
                                min: 1,
                                max: 1,
                                cancellable: false,
                                hidden: false,
                                purpose: PickPurpose::Generic,
                            },
                        });
                        return Exec::Wait;
                    }
                    tied.first().map(|id| (*id, k))
                };
                if let Some((best, k)) = picked {
                    self.add_counters_to(Obj::card(best), CounterKind::PlusOnePlusOne, k);
                    self.layers_dirty = true;
                    if *remember {
                        ctx.remembered = vec![Obj::card(best)];
                    }
                }
                Exec::Done
            }
            Effect::RollDie { sides, per_attacked_player, outcomes, extra } => {
                use rand::Rng;
                let dice = if *per_attacked_player {
                    let mut seen: Vec<PlayerId> = vec![];
                    if let Some(cb) = &self.combat {
                        for a in cb.attackers.iter() {
                            let q = match a.defender {
                                Obj::Player { id } => id,
                                Obj::Card { id } => self.defender_side(id),
                                _ => continue,
                            };
                            if !seen.contains(&q) {
                                seen.push(q);
                            }
                        }
                    }
                    seen.len().max(1)
                } else {
                    1
                };
                let mut roll = 0;
                let dice = dice + self.extra_roll_dice(controller);
                let dice = dice + match extra {
                    Some(a) => {
                        let c = ec!();
                        self.eval_amount(a, &c).clamp(0, 20) as usize
                    }
                    None => 0,
                };
                for _ in 0..dice {
                    let forced = {
                        let mut f = self.forced_rolls.borrow_mut();
                        if f.is_empty() { None } else { Some(f.remove(0)) }
                    };
                    let r = forced.map(|r| r.clamp(1, (*sides).max(1))).unwrap_or_else(|| self.rng.gen_range(1..=(*sides).max(1)));
                    self.log(&format!("Rolled a {r} on a d{sides}"));
                    self.emit(Event::DieRolled { player: controller, n: r, sides: *sides });
                    roll = roll.max(r);
                }
                self.last_die = roll;
                self.emit(Event::DiceRolled { player: controller });
                match outcomes.iter().find(|o| roll >= o.min && roll <= o.max) {
                    Some(o) if !o.effects.is_empty() => Exec::Push(Frame { list: Arc::new(o.effects.clone()), pc: 0, repeat: 0, each: vec![], each_active: false, tbase: ctx.cur_tbase }),
                    _ => Exec::Done,
                }
            }
            Effect::RollTwoChooseOne { sides } => {
                use rand::Rng;
                if let Some(Sub::ChooseVal { kind: 90, texts, .. }) = ctx.sub.clone() {
                    let ans = self.answer.take();
                    ctx.sub = None;
                    let i = match ans {
                        Some(Choice::Pick { indices }) => indices.first().copied().unwrap_or(0).min(1),
                        _ => 0,
                    };
                    self.roll_chosen = texts[i].parse().unwrap_or(0);
                    self.roll_other = texts[1 - i].parse().unwrap_or(0);
                    return Exec::Done;
                }
                let mut rs = vec![];
                // Pixie Guide / Barbarian Class: extra dice are rolled and the lowest results are ignored, leaving two to choose from
                let extra_dice = self.extra_roll_dice(controller);
                for _ in 0..(2 + extra_dice) {
                    let forced = {
                        let mut f = self.forced_rolls.borrow_mut();
                        if f.is_empty() { None } else { Some(f.remove(0)) }
                    };
                    let r = forced.map(|r| r.clamp(1, (*sides).max(1))).unwrap_or_else(|| self.rng.gen_range(1..=(*sides).max(1)));
                    self.log(&format!("Rolled a {r} on a d{sides}"));
                    self.emit(Event::DieRolled { player: controller, n: r, sides: *sides });
                    rs.push(r);
                }
                for _ in 0..extra_dice {
                    if let Some(lowest) = rs.iter().enumerate().min_by_key(|(_, v)| **v).map(|(i, _)| i) {
                        rs.remove(lowest);
                    }
                }
                self.last_die = rs[0].max(rs[1]);
                self.emit(Event::DiceRolled { player: controller });
                if rs[0] == rs[1] {
                    self.roll_chosen = rs[0];
                    self.roll_other = rs[1];
                    return Exec::Done;
                }
                let name = self.def_of(source).name.clone();
                let texts = vec![rs[0].to_string(), rs[1].to_string()];
                ctx.sub = Some(Sub::ChooseVal { player: controller, kind: 90, texts: texts.clone(), players: vec![] });
                self.pending = Some(Decision {
                    player: controller,
                    kind: DecisionKind::Pick {
                        prompt: format!("{name}: choose one result"),
                        source: Some(source),
                        options: texts.iter().map(|t| Opt::Text { text: t.clone() }).collect(),
                        min: 1,
                        max: 1,
                        cancellable: false,
                        hidden: false,
                        purpose: PickPurpose::Generic,
                    },
                });
                Exec::Wait
            }
            Effect::Discover { n } => {
                let k = {
                    let c = ec!();
                    self.eval_amount(n, &c).max(0)
                };
                self.emit(Event::Discovered { player: controller, n: k });
                let list = vec![
                    Effect::RevealUntil {
                        who: Sel::You,
                        filter: (Filter { not_types: vec![CardType::Land], mana_value_vs: Some(AmountCmp { op: CmpOp::Le, amount: Box::new(Amount::Fixed { n: k }) }), ..Default::default() }).into(),
                        count: Amount::Fixed { n: 1 },
                        to: Zone::Exile,
                        tapped: false,
                        rest: Zone::Library,
                        rest_random: true,
                        rest_ordered: false,
                        found_to_bottom: false,
                        cascade_land: false,
                        under_your_control: false,
                    },
                    Effect::CastFree { what: Sel::Remembered, may: true, decline_to_bottom: false, decline_to_hand: true, exile_after: false, copy: false, pay: false, by: None, any_mana: false, lands: false, alt: None, extra_mana: None, otherwise: vec![], suspend: false, lock_spells_after: false, on_cast: vec![], after_cast: vec![], bottom_after: false, copies: None, face_choice: false },
                ];
                Exec::Push(Frame { list: Arc::new(list), pc: 0, repeat: 0, each: vec![], each_active: false, tbase: ctx.cur_tbase })
            }
            Effect::EmitKeywordAction { action, what } => {
                let other = what.as_ref().and_then(|w| {
                    let c = ec!();
                    self.sel_cards(w, &c).first().copied()
                });
                if self.cards[source.0 as usize].zone == Zone::Battlefield {
                    self.emit(Event::KeywordAction { card: source, action: *action, other });
                }
                Exec::Done
            }
            Effect::LockTurns { attacks, targeting, turns } => {
                let until = self.turn + (*turns as u32).max(1) - 1;
                if *attacks {
                    self.no_attack_until = self.no_attack_until.max(until);
                }
                if *targeting {
                    self.no_target_until = self.no_target_until.max(until);
                }
                Exec::Done
            }
            Effect::LoyaltyInstantThisTurn { subtype } => {
                let pl = &mut self.players[controller as usize];
                pl.loyalty_instant_turn = self.turn;
                pl.loyalty_instant_sub = subtype.as_ref().map(|s| Sym::new(s));
                Exec::Done
            }
            Effect::LoyaltyExtraThisTurn { n } => {
                let c = &mut self.cards[source.0 as usize];
                c.loyalty_extra = if c.loyalty_extra_turn == self.turn { c.loyalty_extra.saturating_add((*n).clamp(0, 255) as u8) } else { (*n).clamp(0, 255) as u8 };
                c.loyalty_extra_turn = self.turn;
                Exec::Done
            }
            Effect::LoyaltyTwiceThisTurn { all } => {
                if *all {
                    self.players[controller as usize].loyalty_all_double_turn = self.turn;
                } else {
                    self.cards[source.0 as usize].loyalty_double_turn = self.turn;
                }
                Exec::Done
            }
            Effect::Adapt { n } => {
                let k = {
                    let c = ec!();
                    self.eval_amount(n, &c)
                };
                let ignore = self.adapt_ignore.iter().position(|&x| x == source);
                if let Some(i) = ignore {
                    self.adapt_ignore.remove(i);
                }
                if ignore.is_some() || self.cards[source.0 as usize].counter(CounterKind::PlusOnePlusOne) < 1 {
                    self.add_counters_to(Obj::Card { id: source }, CounterKind::PlusOnePlusOne, k);
                }
                Exec::Done
            }
            Effect::IgnoreThisUntilEndOfTurn => {
                if !self.ignored_until_eot.contains(&source) {
                    self.ignored_until_eot.push(source);
                }
                self.layers_dirty = true;
                Exec::Done
            }
            Effect::AdaptIgnoreCounters { what } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                for c in cs {
                    if !self.adapt_ignore.contains(&c) {
                        self.adapt_ignore.push(c);
                    }
                }
                Exec::Done
            }
            Effect::ChooseAttackers => {
                self.attack_chooser = Some(controller);
                Exec::Done
            }
            Effect::ChooseBlocks { who } => {
                let (ps, ctl) = {
                    let c = ec!();
                    (self.sel_players(who, &c), c.controller)
                };
                for p in ps {
                    self.block_chooser.retain(|(d, _)| *d != p);
                    self.block_chooser.push((p, ctl));
                }
                Exec::Done
            }
            Effect::NotePileOrGuess => {
                self.pile_guess_turn += 1;
                Exec::Done
            }
            Effect::EmitExploited => {
                let victim = ctx.remembered.last().and_then(|o| if let Obj::Card { id } = o { Some(*id) } else { None });
                self.emit(Event::Exploited { card: source, victim });
                Exec::Done
            }
            Effect::Reflexive { targets: specs, effects, untargeted } => {
                let expanded_specs = self.expand_per_opponent(specs, controller);
                let specs = &expanded_specs;
                let mut chosen: Vec<Target> = vec![];
                let mut idx = 0usize;
                let mut divs: Vec<i32> = vec![];
                let mut div_answer: Option<i32> = None;
                if let Some(Sub::ReflexDivide { chosen: c, divs: d }) = ctx.sub.clone() {
                    ctx.sub = None;
                    chosen = c;
                    divs = d;
                    idx = specs.len();
                    if let Some(Choice::Number { n }) = self.answer.take() {
                        div_answer = Some(n);
                    }
                } else if let Some(Sub::Reflex { idx: i, chosen: c, options, .. }) = ctx.sub.clone() {
                    ctx.sub = None;
                    chosen = c;
                    idx = i;
                    if let Some(Choice::Pick { indices }) = self.answer.take() {
                        for k in indices {
                            if let Some(o) = options.get(k) {
                                let zcc = self.obj_zcc(o);
                                chosen.push(Target { obj: *o, zcc });
                            }
                        }
                    }
                    idx += 1;
                }
                while idx < specs.len() {
                    let spec = &specs[idx];
                    // the filter may refer to what this ability remembered (the discarded card's mana value ...)
                    *self.filter_rem.borrow_mut() = ctx.remembered.clone();
                    self.target_trig.set(true);
                    let legal = self.legal_targets(spec, source, controller);
                    self.target_trig.set(false);
                    let legal = if spec.distinct_modes {
                        let used = self.reflex_used.borrow();
                        legal.into_iter().filter(|o| !used.iter().any(|(sid, u)| *sid == ctx.item.id && u == o)).collect::<Vec<_>>()
                    } else {
                        legal
                    };
                    if legal.len() < spec.min as usize {
                        // no legal targets for a required slot: the reflexive ability is never put on the stack
                        return Exec::Done;
                    }
                    if legal.is_empty() {
                        idx += 1;
                        continue;
                    }
                    if legal.len() == spec.min as usize && spec.min == spec.max {
                        for o in &legal {
                            let zcc = self.obj_zcc(o);
                            chosen.push(Target { obj: *o, zcc });
                        }
                        idx += 1;
                        continue;
                    }
                    // "up to X other target ...": the cap can be an amount (X of the spell or ability that made this reflexive trigger)
                    let spec_max = if let Some(a) = &spec.max_amount {
                        let mut ec = EvalCtx::with_controller(source, controller);
                        ec.x = ctx.item.x;
                        ec.remembered = &ctx.remembered;
                        self.eval_amount(a, &ec).max(0).min(250) as u8
                    } else {
                        spec.max
                    };
                    ctx.sub = Some(Sub::Reflex { specs: specs.clone(), idx, chosen, options: legal.clone() });
                    self.pending = Some(Decision {
                        player: controller,
                        kind: DecisionKind::Pick {
                            prompt: format!("{}: choose target ({})", self.def_of(source).name, spec.label),
                            source: Some(source),
                            options: legal.iter().map(|o| Opt::Obj { obj: *o }).collect(),
                            min: spec.min,
                            max: spec_max.min(legal.len() as u8),
                            cancellable: false,
                            hidden: false,
                            purpose: PickPurpose::Generic,
                        },
                    });
                    return Exec::Wait;
                }
                if specs.iter().any(|s| s.distinct_modes) {
                    let mut used = self.reflex_used.borrow_mut();
                    used.retain(|(sid, _)| *sid == ctx.item.id);
                    for t in &chosen {
                        used.push((ctx.item.id, t.obj));
                    }
                }
                // "divided as you choose" on a reflexive ability: the division is announced as it is put on the stack (Blaster Hulk)
                let div_amt = effects.iter().find_map(|e| match e { Effect::DivideDamage { amount, at_cast: true, slot: None, among: None, .. } => Some(amount.clone()), _ => None });
                if let Some(amt) = div_amt {
                    let total = {
                        let mut ec = EvalCtx::with_controller(source, controller);
                        ec.x = ctx.item.x;
                        ec.remembered = &ctx.remembered;
                        self.eval_amount(&amt, &ec)
                    };
                    let nt = chosen.len();
                    while divs.len() < nt {
                        let i = divs.len();
                        let left = total - divs.iter().sum::<i32>();
                        let rest = (nt - i - 1) as i32;
                        let maxv = left - rest;
                        if rest == 0 {
                            divs.push(left.max(0));
                        } else if maxv < 1 {
                            divs.push(left.clamp(0, 1));
                        } else if let Some(n) = div_answer.take() {
                            divs.push(n.clamp(1, maxv));
                        } else {
                            ctx.sub = Some(Sub::ReflexDivide { chosen, divs });
                            self.pending = Some(Decision {
                                player: controller,
                                kind: DecisionKind::Number { prompt: format!("{}: damage to target {} of {}", self.def_of(source).name, i + 1, nt), min: 1, max: maxv, cancellable: false },
                            });
                            return Exec::Wait;
                        }
                    }
                }
                self.next_stack_id += 1;
                let n_chosen = chosen.len();
                // the reflexive ability resolves later: the stored slots it reads are pinned as they are now
                let effects = self.pin_stored(effects).unwrap_or_else(|| effects.clone());
                let item = StackItem {
                    id: StackId(self.next_stack_id),
                    controller,
                    source,
                    src_zcc: self.cards[source.0 as usize].zcc,
                    def: self.cards[source.0 as usize].def,
                    kind: ItemKind::Delayed { effects },
                    targets: chosen.into_iter().collect(),
                    // the reflexive ability keeps the X of the ability that created it
                    x: ctx.item.x,
                    modes: Default::default(),
                    ctx: ctx.item.ctx.clone(),
                    copy: false,
                    copy_except: vec![],
                    divisions: divs.into_iter().collect(),
                    spec_counts: if n_chosen > 0 && !*untargeted { SmallVec::from_slice(&[n_chosen as u8]) } else { Default::default() },
                    granted_snap: None,
            spliced: vec![],
                    remembered: ctx.remembered.clone(),
                };
                self.stack.push(item);
                Exec::Done
            }
            Effect::ChooseModes { choose_min, choose_max, modes, once_each_turn, once_ever, budget, chooser, max_amount, random, at_stack: _, allow_repeat: _, villainous } => {
                if *villainous && ctx.sub.is_none() {
                    let faced = {
                        let c = ec!();
                        chooser.as_ref().map_or(vec![controller], |s| self.sel_players(s, &c))
                    };
                    let twice = faced.first().map_or(false, |&fp| self.villainous_twice(fp));
                    if twice {
                        let mut once = e.clone();
                        if let Effect::ChooseModes { villainous, .. } = &mut once {
                            *villainous = false;
                        }
                        return Exec::Push(Frame { list: Arc::new(vec![once.clone(), once]), pc: 0, repeat: 0, each: vec![], each_active: false, tbase: ctx.cur_tbase });
                    }
                }
                if *random {
                    if modes.is_empty() {
                        return Exec::Done;
                    }
                    let i = self.rand_index(&ec!(), modes.len());
                    let list = modes[i].effects.clone();
                    if list.is_empty() {
                        return Exec::Done;
                    }
                    return Exec::Push(Frame { list: Arc::new(list), pc: 0, repeat: 0, each: vec![], each_active: false, tbase: ctx.cur_tbase });
                }
                if *budget > 0 {
                    let (mut chosen, remaining) = match ctx.sub.clone() {
                        Some(Sub::ModesBudget { chosen, remaining }) => (chosen, remaining),
                        _ => (vec![], *budget),
                    };
                    let mut remaining = remaining;
                    if matches!(ctx.sub, Some(Sub::ModesBudget { .. })) {
                        let ans = self.answer.take();
                        ctx.sub = None;
                        let afford: Vec<usize> = (0..modes.len()).filter(|i| modes[*i].paw <= remaining).collect();
                        match ans {
                            Some(Choice::Pick { indices }) if !indices.is_empty() => {
                                if let Some(i) = afford.get(indices[0]) {
                                    chosen.push(*i);
                                    remaining -= modes[*i].paw.min(remaining);
                                }
                            }
                            _ => remaining = 0,
                        }
                    }
                    let afford: Vec<usize> = (0..modes.len()).filter(|i| modes[*i].paw <= remaining && modes[*i].paw > 0).collect();
                    if remaining == 0 || afford.is_empty() {
                        let mut all: Vec<Effect> = vec![];
                        for i in chosen {
                            all.extend(modes[i].effects.iter().cloned());
                        }
                        if all.is_empty() {
                            return Exec::Done;
                        }
                        return Exec::Push(Frame { list: Arc::new(all), pc: 0, repeat: 0, each: vec![], each_active: false, tbase: ctx.cur_tbase });
                    }
                    ctx.sub = Some(Sub::ModesBudget { chosen, remaining });
                    let decider = match chooser {
                        Some(sel) => {
                            let c = ec!();
                            self.sel_players(sel, &c).first().copied().unwrap_or(controller)
                        }
                        None => controller,
                    };
                    self.pending = Some(Decision {
                        player: decider,
                        kind: DecisionKind::Pick {
                            prompt: format!("Choose a mode for {} ({} left)", self.def_of(source).name, remaining),
                            source: Some(source),
                            options: afford.iter().map(|i| Opt::Text { text: modes[*i].text.clone() }).collect(),
                            min: 0,
                            max: 1,
                            cancellable: false,
                            hidden: false,
                            purpose: PickPurpose::Mode,
                        },
                    });
                    return Exec::Wait;
                }
                if let Some(Sub::Modes { modes: progs, orig }) = ctx.sub.clone() {
                    let ans = self.answer.take();
                    ctx.sub = None;
                    let mut idx: Vec<usize> = match ans {
                        Some(Choice::Pick { indices }) => indices,
                        _ => vec![],
                    };
                    idx.sort();
                    idx.dedup();
                    if *once_each_turn || *once_ever {
                        let turn = if *once_ever { u32::MAX } else { self.turn };
                        let now = self.turn;
                        let used = &mut self.cards[source.0 as usize].modes_used;
                        used.retain(|(t, _)| *t == now || *t == u32::MAX);
                        for i in &idx {
                            if let Some(o) = orig.get(*i) {
                                used.push((turn, *o));
                            }
                        }
                    }
                    let mut all: Vec<Effect> = vec![];
                    for i in idx {
                        if let Some(p) = progs.get(i) {
                            all.extend(p.iter().cloned());
                        }
                    }
                    if all.is_empty() {
                        return Exec::Done;
                    }
                    return Exec::Push(Frame { list: Arc::new(all), pc: 0, repeat: 0, each: vec![], each_active: false, tbase: ctx.cur_tbase });
                }
                let avail: Vec<usize> = (0..modes.len())
                    .filter(|i| !(*once_each_turn || *once_ever) || !self.cards[source.0 as usize].modes_used.iter().any(|(t, m)| (*t == self.turn || *t == u32::MAX) && *m as usize == *i))
                    .collect();
                let n = avail.len();
                let max_n = match max_amount {
                    Some(a) => self.eval_amount(a, &ec!()).max(0) as usize,
                    None => *choose_max as usize,
                };
                let (lo, hi) = ((*choose_min as usize).min(n), max_n.min(n));
                if n == 0 || hi == 0 {
                    return Exec::Done;
                }
                ctx.sub = Some(Sub::Modes { modes: avail.iter().map(|i| modes[*i].effects.clone()).collect(), orig: avail.iter().map(|i| *i as u8).collect() });
                let decider = match chooser {
                    Some(sel) => {
                        let c = ec!();
                        self.sel_players(sel, &c).first().copied().unwrap_or(controller)
                    }
                    None => controller,
                };
                self.pending = Some(Decision {
                    player: decider,
                    kind: DecisionKind::Pick {
                        prompt: format!("Choose mode for {}", self.def_of(source).name),
                        source: Some(source),
                        options: avail.iter().map(|i| Opt::Text { text: modes[*i].text.clone() }).collect(),
                        min: lo as u8,
                        max: hi as u8,
                        cancellable: false,
                        hidden: false,
                        purpose: PickPurpose::Mode,
                    },
                });
                Exec::Wait
            }
            Effect::AdventureFromGraveyard { what } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                let n = self.players.len() as u32;
                for id in cs {
                    let c = &self.cards[id.0 as usize];
                    if c.zone == Zone::Graveyard {
                        let k = (c.owner as u32 + n - self.active as u32) % n;
                        self.cards[id.0 as usize].adv_gy_until = self.turn + if k == 0 { n } else { k };
                    }
                }
                Exec::Done
            }
            Effect::GrantPlayFromExile { what, window, any_mana, free, pay_life_mv, by, play_cond, affinity, exile_after, one, limit, per_type, extra_generic, land_tapped, cast_only, free_if } => {
                let (cs, grantee) = {
                    let c = ec!();
                    (self.sel_cards(what, &c), by.as_ref().and_then(|b| self.sel_players(b, &c).first().copied()).unwrap_or(controller))
                };
                let n = self.players.len() as u32;
                let until = match window {
                    PlayWindow::ThisTurn => self.turn,
                    PlayWindow::EndOfYourNextTurn => {
                        let k = (controller as u32 + n - self.active as u32) % n;
                        self.turn + if k == 0 { n } else { k }
                    }
                    PlayWindow::UntilYourNextUpkeep => {
                        let k = (controller as u32 + n - self.active as u32) % n;
                        self.turn + if k == 0 { n } else { k } - 1
                    }
                    PlayWindow::UntilYourNextEndStep => {
                        let k = (controller as u32 + n - self.active as u32) % n;
                        self.turn + if k == 0 && !matches!(self.step, Step::End | Step::Cleanup) { 0 } else if k == 0 { n } else { k }
                    }
                    PlayWindow::UntilGranteesNextEndStep => {
                        let k = (grantee as u32 + n - self.active as u32) % n;
                        self.turn + if k == 0 && !matches!(self.step, Step::End | Step::Cleanup) { 0 } else if k == 0 { n } else { k }
                    }
                    PlayWindow::UntilYourNextCombatEnd => {
                        let k = (controller as u32 + n - self.active as u32) % n;
                        self.turn + if k == 0 { n } else { k }
                    }
                    PlayWindow::Forever | PlayWindow::WhileSourceControlled | PlayWindow::UntilSourceExilesAnother => u32::MAX,
                };
                let stop_end = matches!(window, PlayWindow::UntilYourNextEndStep | PlayWindow::UntilGranteesNextEndStep);
                let stop_combat = matches!(window, PlayWindow::UntilYourNextCombatEnd);
                if matches!(window, PlayWindow::UntilSourceExilesAnother) && !cs.is_empty() {
                    let via = Some((source, self.cards[source.0 as usize].zcc));
                    for c in self.cards.iter_mut() {
                        if c.zone == Zone::Exile && c.played_via == via && c.play_until != 0 && !cs.contains(&c.id) {
                            c.play_until = 0;
                        }
                    }
                }
                let play_src = if matches!(window, PlayWindow::WhileSourceControlled) { Some(source) } else { None };
                for id in cs {
                    if (*per_type || *cast_only) && self.def_of(id).types.has(CardType::Land) {
                        continue;
                    }
                    // a card in a graveyard (Zul Ashur): a this-turn permission to cast/play it from there
                    if self.cards[id.0 as usize].zone == Zone::Graveyard && matches!(window, PlayWindow::ThisTurn) {
                        self.turn_play_grants.push((grantee, Zone::Graveyard, Filter::default(), source, self.turn, vec![id], false));
                        continue;
                    }
                    if self.cards[id.0 as usize].zone == Zone::Exile {
                        self.cards[id.0 as usize].play_per_type = *per_type;
                        self.cards[id.0 as usize].play_types_used = 0;
                        self.cards[id.0 as usize].play_until = until;
                        self.cards[id.0 as usize].play_src = play_src;
                        self.cards[id.0 as usize].play_by = grantee;
                        self.cards[id.0 as usize].play_cond = play_cond.clone();
                        self.cards[id.0 as usize].spend_any = *any_mana;
                        let free_now = *free || free_if.as_ref().map_or(false, |f| self.matches_card(f, id, source, controller));
                        self.cards[id.0 as usize].play_free = free_now || *pay_life_mv;
                        self.cards[id.0 as usize].play_life_mv = *pay_life_mv;
                        self.cards[id.0 as usize].play_affinity = affinity.clone();
                        self.cards[id.0 as usize].play_extra = *extra_generic;
                        self.cards[id.0 as usize].play_land_tapped = *land_tapped && self.def_of(id).types.has(CardType::Land);
                        self.cards[id.0 as usize].play_exile_after = *exile_after;
                        self.cards[id.0 as usize].play_stop_end = stop_end;
                        self.cards[id.0 as usize].play_stop_combat = stop_combat;
                        self.cards[id.0 as usize].play_one = *one;
                        self.cards[id.0 as usize].play_limit = *limit;
                        self.cards[id.0 as usize].play_used = 0;
                        self.cards[id.0 as usize].played_via = Some((source, self.cards[source.0 as usize].zcc));
                    }
                }
                Exec::Done
            }
            Effect::FreeCastOneThisTurn { what } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                for id in cs {
                    if self.cards[id.0 as usize].zone == Zone::Exile && !self.def_of(id).types.has(CardType::Land) {
                        self.cards[id.0 as usize].free_turn = self.turn;
                        self.cards[id.0 as usize].free_via = Some(source);
                    }
                }
                Exec::Done
            }
            Effect::PileBlocks => {
                self.pile_blocks_turn = self.turn;
                self.pile_state = None;
                Exec::Done
            }
            Effect::LimitBlockers { who, n } => {
                let ps = {
                    let c = ec!();
                    self.sel_players(who, &c)
                };
                for p in ps {
                    self.block_limits.push((p, *n));
                }
                Exec::Done
            }
            Effect::GrantProtection { what, from, duration } => {
                let (cs, ps) = {
                    let c = ec!();
                    (self.sel_cards(what, &c), self.sel_players(what, &c))
                };
                // "protection from the chosen colour / type": settled now from the choice this permanent made
                // (a source that left the battlefield uses what it had chosen); "protection from the chosen player" given to
                // another permanent or a player (Guardian Archon): the player this source chose
                let from = &match from {
                    ProtectionFrom::ChosenColor | ProtectionFrom::ChosenType | ProtectionFrom::ChosenPlayer => match self.settle_chosen_quality(source, from) {
                        Some(q) => q,
                        None => return Exec::Done,
                    },
                    ProtectionFrom::ControlledBy { p: None, who: Some(w) } => {
                        let c = ec!();
                        match self.sel_players(w, &c).first() {
                            Some(&p) => ProtectionFrom::ControlledBy { p: Some(p), who: None },
                            None => return Exec::Done,
                        }
                    }
                    other => other.clone(),
                };
                self.add_effect(source, controller, &cs, Modif::Protection(from.clone()), *duration);
                for p in ps {
                    self.player_prot.push(PlayerProt { player: p, from: from.clone(), until: *duration, owner: controller });
                }
                Exec::Done
            }
            Effect::GrantStatic { what, effect, duration, dynamic: true, .. } => {
                // the stored slots it reads are pinned as they are now (they only last for this resolution)
                let pinned = self.pin_stored(effect);
                let effect = pinned.as_ref().unwrap_or(effect);
                // a computed X in an attack / block cost is fixed when the effect resolves ("pays {X} for each attacking creature")
                let snapped;
                let effect = match effect {
                    StaticEffect::AttackCost { affects, cost, against } if cost.iter().any(|c| matches!(c, Cost::ManaAmount { amount: Amount::X })) => {
                        snapped = StaticEffect::AttackCost { affects: affects.clone(), cost: cost.iter().map(|c| if let Cost::ManaAmount { amount: Amount::X } = c { Cost::ManaAmount { amount: Amount::Fixed { n: ctx.item.x } } } else { c.clone() }).collect(), against: *against };
                        &snapped
                    }
                    StaticEffect::BlockCost { affects, cost, attacker } if cost.iter().any(|c| matches!(c, Cost::ManaAmount { amount: Amount::X })) => {
                        snapped = StaticEffect::BlockCost { affects: affects.clone(), attacker: attacker.clone(), cost: cost.iter().map(|c| if let Cost::ManaAmount { amount: Amount::X } = c { Cost::ManaAmount { amount: Amount::Fixed { n: ctx.item.x } } } else { c.clone() }).collect() };
                        &snapped
                    }
                    e => e,
                };
                // "all creatures able to block target creature do so": the static's `this` is the targeted creature
                let mut source = source;
                if matches!(effect, StaticEffect::MustBlock { attacker: Some(f), .. } if f.this) {
                    let cs = {
                        let c = ec!();
                        self.sel_cards(what, &c)
                    };
                    if let Some(&t) = cs.first() {
                        source = t;
                    }
                }
                let ts = self.next_ts();
                let id = self.next_effect_id;
                self.next_effect_id += 1;
                let duration = if *duration == Duration::Instant { Duration::UntilEndOfTurn } else { *duration };
                self.effects.push(ContEffect { id, ts, source, controller, affects: SmallVec::new(), players: SmallVec::new(), modif: Modif::DynStatic(Box::new(effect.clone())), duration, created_turn: self.turn });
                self.layers_dirty = true;
                Exec::Done
            }
            Effect::GrantStatic { what, effect, duration, .. } => {
                let pinned = self.pin_stored(effect);
                let effect = pinned.as_ref().unwrap_or(effect);
                // "for as long as you control <source>": if the source already left the battlefield, the duration is over before it begins
                if matches!(duration, Duration::WhileSourceControlled | Duration::WhileSourceOnBattlefield) && !self.battlefield.contains(&source) {
                    return Exec::Done;
                }
                // a computed X in an attack / block cost is fixed when the effect resolves ("pays {X} for each attacking creature")
                let snapped;
                let effect = match effect {
                    StaticEffect::AttackCost { affects, cost, against } if cost.iter().any(|c| matches!(c, Cost::ManaAmount { amount: Amount::X })) => {
                        snapped = StaticEffect::AttackCost { affects: affects.clone(), cost: cost.iter().map(|c| if let Cost::ManaAmount { amount: Amount::X } = c { Cost::ManaAmount { amount: Amount::Fixed { n: ctx.item.x } } } else { c.clone() }).collect(), against: *against };
                        &snapped
                    }
                    StaticEffect::BlockCost { affects, cost, attacker } if cost.iter().any(|c| matches!(c, Cost::ManaAmount { amount: Amount::X })) => {
                        snapped = StaticEffect::BlockCost { affects: affects.clone(), attacker: attacker.clone(), cost: cost.iter().map(|c| if let Cost::ManaAmount { amount: Amount::X } = c { Cost::ManaAmount { amount: Amount::Fixed { n: ctx.item.x } } } else { c.clone() }).collect() };
                        &snapped
                    }
                    StaticEffect::CostChange { filter, whose, generic, per: Some(a), not_from, color, once, per_caster, fixed: true, flashback, dash, generic_fallback, free } => {
                        let n = { let c = ec!(); self.eval_amount(a, &c) };
                        snapped = StaticEffect::CostChange { filter: filter.clone(), whose: *whose, generic: *generic, per: Some(Amount::Fixed { n }), not_from: *not_from, color: *color, once: *once, per_caster: *per_caster, fixed: false, flashback: *flashback, dash: *dash, generic_fallback: *generic_fallback, free: *free };
                        &snapped
                    }
                    e => e,
                };
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                if cs.is_empty() {
                    // a static granted to players ("you may cast spells as though they had flash this turn")
                    let ps = {
                        let c = ec!();
                        self.sel_players(what, &c)
                    };
                    if !ps.is_empty() && matches!(effect, StaticEffect::NoMaxHandSize | StaticEffect::CantLoseGame { .. } | StaticEffect::CantWinGame { .. } | StaticEffect::DamageLifeFloor { .. } | StaticEffect::PlayerHexproof { .. } | StaticEffect::PlayerHexproofFrom { .. } | StaticEffect::PlayerShroud { .. } | StaticEffect::IgnoreShroud { .. } | StaticEffect::IgnoreHexproof { .. } | StaticEffect::HandsRevealed { .. } | StaticEffect::IgnoreSearchBan | StaticEffect::ManaPersists { .. } | StaticEffect::ManaBecomesColorless { .. } | StaticEffect::CastAsThoughFlash { .. } | StaticEffect::CostChange { .. } | StaticEffect::CantBeCountered { .. } | StaticEffect::CantBeTargetedBySpells { .. } | StaticEffect::GrantSpellKeyword { .. } | StaticEffect::IgnoreLegendRule { .. } | StaticEffect::AttackCost { .. } | StaticEffect::BlockCost { .. } | StaticEffect::AlternativeManaCost { .. } | StaticEffect::TopCardVisible { .. }) {
                        let ts = self.next_ts();
                        let id = self.next_effect_id;
                        self.next_effect_id += 1;
                        let duration = if *duration == Duration::Instant { Duration::UntilEndOfTurn } else { *duration };
                        self.effects.push(ContEffect { id, ts, source, controller, affects: SmallVec::new(), players: ps.iter().copied().collect(), modif: Modif::Static(Box::new(effect.clone())), duration, created_turn: self.turn });
                    }
                    return Exec::Done;
                }
                // "gains hexproof / protection from the chosen colour" given to another permanent: settled now to the colour
                // (type, player) the SOURCE chose, not the holder's (Skrelv, Defector Mite)
                let settled;
                let effect = match effect {
                    StaticEffect::GrantAbility { affects, ability } if matches!(&**ability, Ability::Protection { from } | Ability::HexproofFrom { from } if from.iter().any(|q| matches!(q, ProtectionFrom::ChosenColor | ProtectionFrom::ChosenType | ProtectionFrom::ChosenPlayer))) => {
                        let fix = |from: &Vec<ProtectionFrom>| from.iter().filter_map(|q| self.settle_chosen_quality(source, q)).collect::<Vec<_>>();
                        let ability = match &**ability {
                            Ability::Protection { from } => Ability::Protection { from: fix(from) },
                            Ability::HexproofFrom { from } => Ability::HexproofFrom { from: fix(from) },
                            a => a.clone(),
                        };
                        settled = StaticEffect::GrantAbility { affects: affects.clone(), ability: Box::new(ability) };
                        &settled
                    }
                    e => e,
                };
                self.add_effect(source, controller, &cs, Modif::Static(Box::new(effect.clone())), *duration);
                Exec::Done
            }
            Effect::GrantProtectionSharedFrom { what, among, duration } => {
                let (cs, srcs) = {
                    let c = ec!();
                    (self.sel_cards(what, &c), self.sel_cards(among, &c))
                };
                let mut quals: Vec<ProtectionFrom> = Vec::new();
                for id in srcs {
                    for p in &self.cards[id.0 as usize].cur.protections {
                        if !quals.contains(p) {
                            quals.push(p.clone());
                        }
                    }
                }
                for q in quals {
                    self.add_effect(source, controller, &cs, Modif::Protection(q), *duration);
                }
                Exec::Done
            }
            Effect::GrantProtectionChosenColor { what, duration } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                // a source that left the battlefield (sacrificed as the cost: Floating Shield) uses the colour it had chosen
                if let Some(col) = self.chosen_color_lki(source) {
                    self.add_effect(source, controller, &cs, Modif::Protection(ProtectionFrom::Color { c: col }), *duration);
                }
                Exec::Done
            }
            Effect::Goad { what, forever, while_controlled_by_other, while_source_on_battlefield } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                for id in &cs {
                    let zc = self.cards[id.0 as usize].zcc;
                    if *while_source_on_battlefield && !*while_controlled_by_other {
                        let sz = self.cards[source.0 as usize].zcc;
                        self.goad_while_src.push((*id, zc, source, sz));
                    }
                    self.goaded.push((*id, zc, controller, if *while_controlled_by_other { u32::MAX - 1 } else if *while_source_on_battlefield { u32::MAX - 2 } else if *forever { u32::MAX } else { self.turn }));
                }
                self.add_effect(source, controller, &cs, Modif::Keyword(Keyword::MustAttack), if *while_controlled_by_other { Duration::WhileAffectedControlledByOther } else if *while_source_on_battlefield { Duration::WhileSourceOnBattlefield } else if *forever { Duration::Forever } else { Duration::UntilYourNextTurn });
                Exec::Done
            }
            Effect::LookAtFaceDown { what } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                for id in cs {
                    if self.cards[id.0 as usize].face_down {
                        self.log(format!("{} looks at the face-down card {}", self.def_of(source).name, self.def_of(id).name));
                    }
                }
                Exec::Done
            }
            Effect::CantAttackYouUntilYourNextTurn { player } => {
                let ps = {
                    let c = ec!();
                    self.sel_players(player, &c)
                };
                for q in ps {
                    if q != controller && !self.no_attack_pending.contains(&(q, controller)) {
                        self.no_attack_pending.push((q, controller));
                    }
                }
                Exec::Done
            }
            Effect::SanctuaryUntilYourNextTurn => {
                if !self.sanctuary.contains(&controller) {
                    self.sanctuary.push(controller);
                }
                Exec::Done
            }
            Effect::TapForeignLandsThisTurn => {
                self.players[controller as usize].foreign_lands_turn = self.turn + 1;
                Exec::Done
            }
            Effect::LifeManaThisTurn => {
                self.players[controller as usize].life_mana = Some((self.turn + 1, source));
                Exec::Done
            }
            Effect::CantAttackYouNextTurn { player } => {
                let ps = {
                    let c = ec!();
                    self.sel_players(player, &c)
                };
                for q in ps {
                    if !self.no_attack_next.contains(&(q, controller, false)) {
                        self.no_attack_next.push((q, controller, false));
                    }
                }
                Exec::Done
            }
            Effect::ExtraUpkeep { n } => {
                let k = {
                    let c = ec!();
                    self.eval_amount(n, &c)
                };
                self.extra_upkeep += k.max(0) as u32;
                Exec::Done
            }
            Effect::ExtraEndStep => {
                self.extra_end += 1;
                Exec::Done
            }
            Effect::ExtraBeginningPhaseAfterCombat => {
                self.extra_beginning_combat += 1;
                Exec::Done
            }
            Effect::ExtraBeginningPhase => {
                self.extra_beginning += 1;
                Exec::Done
            }
            Effect::ExtraCombat { untap, then_main } => {
                // cast after the last main phase, the main phase that follows the extra combat IS the additional one; cast earlier, one more is needed
                if *then_main && !matches!(self.step, Step::Main2 | Step::End | Step::Cleanup) {
                    self.extra_main_pending += 1;
                }
                if matches!(self.step, Step::Main2 | Step::End | Step::Cleanup) {
                    self.extra_combat_after_main += 1;
                } else {
                    self.extra_combat_now += 1;
                }
                if *untap {
                    let mine: Vec<CardId> = self.battlefield.iter().copied().filter(|id| self.cards[id.0 as usize].cur.controller == controller && self.cards[id.0 as usize].cur.types.has(CardType::Creature) && self.cards[id.0 as usize].tapped).collect();
                    for id in mine {
                        self.untap_card(id);
                    }
                }
                Exec::Done
            }
            Effect::Myriad => {
                let defender: Option<PlayerId> = self.combat.as_ref().and_then(|c| c.attackers.iter().find(|a| a.card == source)).and_then(|a| match a.defender {
                    Obj::Player { id } => Some(id),
                    Obj::Card { id } => Some(self.cards[id.0 as usize].cur.controller),
                    _ => None,
                });
                let def_card = self.cards[source.0 as usize].def;
                let mut made: Vec<Obj> = vec![];
                for opp in self.opponents(controller) {
                    if Some(opp) == defender || self.token_cap_reached() {
                        continue;
                    }
                    for _ in 0..self.replace_token_count(controller, 1) {
                        if self.token_cap_reached() {
                            break;
                        }
                        let id = self.create_card(def_card, controller, Zone::Exile, true);
                        self.players[controller as usize].exile.retain(|x| *x != id);
                        self.cards[id.0 as usize].zone = Zone::Library;
                        self.move_card(id, Zone::Battlefield, MoveOpts { controller: Some(controller), tapped: true, ..Default::default() });
                        if let Some(cb) = self.combat.as_mut() {
                            cb.attackers.push(AttackInfo { card: id, defender: Obj::Player { id: opp }, blocked: false, blockers: Default::default(), band: 0 });
                        }
                        self.emit(Event::Token { card: id });
                        made.push(Obj::card(id));
                    }
                }
                if !made.is_empty() {
                    let did = self.next_effect_id;
                    self.next_effect_id += 1;
                    self.delayed.push(DelayedTrigger {
                        id: did,
                        controller,
                        source,
                        when: DelayedWhen::EndOfCombat,
                        effects: vec![Effect::Exile { what: Sel::Remembered, return_at_end: false, return_at_owners_end: false, face_down: false, until_source_leaves: false, return_to: None, manual_return: false, return_counters: 0, return_bonus: false }],
                        created_turn: self.turn,
                        targets: vec![],
                        ctx: EventCtx::default(),
                        remembered: made,
                    });
                }
                Exec::Done
            }
            Effect::BecomeCopy { what, of, except, duration } => {
                let (targets, from) = {
                    let c = ec!();
                    (self.sel_cards(what, &c), self.sel_cards(of, &c).first().copied())
                };
                let Some(from) = from else { return Exec::Done };
                let new_def = self.cards[from.0 as usize].def;
                for id in targets {
                    if matches!(duration, Duration::UntilEndOfTurn | Duration::Instant | Duration::UntilNextEndStep) {
                        let zcc = self.cards[id.0 as usize].zcc;
                        let old = self.cards[id.0 as usize].def;
                        self.copy_reverts.push((id, zcc, old, *duration == Duration::UntilNextEndStep));
                    }
                    if *duration == Duration::UntilYourNextTurn {
                        let zcc = self.cards[id.0 as usize].zcc;
                        let old = self.cards[id.0 as usize].def;
                        self.copy_reverts_turn.push((id, zcc, old, controller));
                    }
                    if matches!(duration, Duration::WhileSourceOnBattlefield | Duration::WhileSourceAttached | Duration::WhileCopiedTapped) {
                        let zcc = self.cards[id.0 as usize].zcc;
                        let old = self.cards[id.0 as usize].def;
                        let (watch, mode) = match duration {
                            Duration::WhileCopiedTapped => (from, 2u8),
                            Duration::WhileSourceAttached => (source, 1u8),
                            _ => (source, 0u8),
                        };
                        self.copy_while_source.push((id, zcc, old, watch, self.cards[watch.0 as usize].zcc, mode));
                    }
                    self.cards[id.0 as usize].def = new_def;
                    self.apply_copy_except(source, controller, id, except, &ctx.remembered);
                    self.layers_dirty = true;
                }
                Exec::Done
            }
            Effect::CastFree { what, may, decline_to_bottom, decline_to_hand, exile_after, copy, pay, by, any_mana, lands, alt, extra_mana, otherwise, suspend, lock_spells_after, on_cast, after_cast, bottom_after, copies, face_choice } => {
                let n_copies: u8 = copies.as_ref().map_or(0, |a| { let c = ec!(); self.eval_amount(a, &c).clamp(0, 20) as u8 });
                let mut face_pick: Option<bool> = None;
                let mut resumed: Option<(PlayerId, CardId)> = None;
                if let Some(Sub::FreeCastFace { player, card }) = ctx.sub.clone() {
                    face_pick = Some(matches!(self.answer.take(), Some(Choice::YesNo { yes: true })));
                    ctx.sub = None;
                    resumed = Some((player, card));
                }
                if let Some(Sub::FreeCast { player, card }) = resumed.map(|(p, c)| Sub::FreeCast { player: p, card: c }).or_else(|| ctx.sub.clone()) {
                    let ans = if resumed.is_some() { Some(Choice::YesNo { yes: true }) } else { self.answer.take() };
                    ctx.sub = None;
                    if *face_choice && face_pick.is_none() && matches!(ans, Some(Choice::YesNo { yes: true })) {
                        if let Some(back) = self.def_of(card).other_face.map(|b| self.db.def(b)).filter(|b| !b.types.has(CardType::Land)) {
                            ctx.sub = Some(Sub::FreeCastFace { player, card });
                            self.pending = Some(Decision {
                                player,
                                kind: DecisionKind::YesNo { prompt: format!("Cast {} (back face) instead of the front face?", back.name), source: Some(source) },
                            });
                            return Exec::Wait;
                        }
                    }
                    let alt_use: Option<u8> = if face_pick == Some(true) { Some(crate::decision::FACE_ALT) } else { *alt };
                    if let Some(Choice::YesNo { yes: true }) = ans {
                        if *lands && self.cards[card.0 as usize].cur.types.has(CardType::Land) {
                            // a land that can't be played (no land drop left) counts as "if you don't"
                            if !self.play_land_free(player, card) && !otherwise.is_empty() {
                                return Exec::Push(Frame { list: Arc::new(otherwise.clone()), pc: 0, repeat: 0, each: vec![], each_active: false, tbase: ctx.cur_tbase });
                            }
                            return Exec::Done;
                        }
                        let card = if *copy { self.make_cast_copy(card, player) } else { card };
                        if *exile_after {
                            self.exile_marked.push(card);
                        }
                        if *any_mana {
                            self.cards[card.0 as usize].spend_any = true;
                        }
                        self.cards[card.0 as usize].cast_extra_mana = extra_mana.clone();
                        self.cards[card.0 as usize].cast_copies = n_copies;
                        if !after_cast.is_empty() {
                            self.cast_after.push((card, source, after_cast.clone()));
                        }
                        if *bottom_after {
                            self.cards[card.0 as usize].cast_bottom_after = true;
                        }
                        let alt_use = self.free_cast_alt(player, card, *pay, alt_use);
                        self.free_casts.push((player, card, *pay, if *suspend { self.suspend_alt(card).or(alt_use) } else { alt_use }));
                        if *lock_spells_after {
                            let ts = self.next_ts();
                            let eid = self.next_effect_id;
                            self.next_effect_id += 1;
                            let cap = self.players[player as usize].spells_cast + 1;
                            self.effects.push(ContEffect { id: eid, ts, source, controller, affects: SmallVec::new(), players: std::iter::once(player).collect(), modif: Modif::SpellCap(cap), duration: Duration::UntilEndOfTurn, created_turn: self.turn });
                        }
                    } else if *decline_to_bottom && self.cards[card.0 as usize].zone == Zone::Exile {
                        self.move_card(card, Zone::Library, MoveOpts { bottom: true, ..Default::default() });
                    } else if *decline_to_hand && self.cards[card.0 as usize].zone == Zone::Exile {
                        self.move_card(card, Zone::Hand, Default::default());
                    }
                    if !otherwise.is_empty() && !matches!(ans, Some(Choice::YesNo { yes: true })) {
                        return Exec::Push(Frame { list: Arc::new(otherwise.clone()), pc: 0, repeat: 0, each: vec![], each_active: false, tbase: ctx.cur_tbase });
                    }
                    if !on_cast.is_empty() && matches!(ans, Some(Choice::YesNo { yes: true })) {
                        return Exec::Push(Frame { list: Arc::new(on_cast.clone()), pc: 0, repeat: 0, each: vec![], each_active: false, tbase: ctx.cur_tbase });
                    }
                    return Exec::Done;
                }
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                let Some(&card) = cs.first() else {
                    // nothing to cast ("if you don't", e.g. an empty library): the fallback applies
                    if *may && !otherwise.is_empty() {
                        return Exec::Push(Frame { list: Arc::new(otherwise.clone()), pc: 0, repeat: 0, each: vec![], each_active: false, tbase: ctx.cur_tbase });
                    }
                    return Exec::Done;
                };
                let controller = match by {
                    Some(b) => {
                        let c = ec!();
                        self.sel_players(b, &c).first().copied().unwrap_or(controller)
                    }
                    None => controller,
                };
                if *may && !otherwise.is_empty() {
                    // "if you don't": a land (that can't be played) or an unaffordable card counts as not cast, without asking
                    let unplayable = (self.cards[card.0 as usize].cur.types.has(CardType::Land) && !*lands) || matches!(self.cards[card.0 as usize].zone, Zone::Stack | Zone::Battlefield) || (*pay && !self.can_afford(controller, &self.effective_cost(controller, card, if *suspend { self.suspend_alt(card).or(*alt) } else { *alt }), 0));
                    if unplayable {
                        return Exec::Push(Frame { list: Arc::new(otherwise.clone()), pc: 0, repeat: 0, each: vec![], each_active: false, tbase: ctx.cur_tbase });
                    }
                }
                if !*may {
                    // every selected card is cast (Ashiok, Nightmare Muse: up to three at once)
                    for &card in &cs {
                        if *lands && self.cards[card.0 as usize].cur.types.has(CardType::Land) && !matches!(self.cards[card.0 as usize].zone, Zone::Stack | Zone::Battlefield) {
                            self.play_land_free(controller, card);
                            continue;
                        }
                        if self.cards[card.0 as usize].cur.types.has(CardType::Land) || matches!(self.cards[card.0 as usize].zone, Zone::Stack | Zone::Battlefield) {
                            continue;
                        }
                        let card = if *copy { self.make_cast_copy(card, controller) } else { card };
                        if *exile_after {
                            self.exile_marked.push(card);
                        }
                        if *any_mana {
                            self.cards[card.0 as usize].spend_any = true;
                        }
                        self.cards[card.0 as usize].cast_extra_mana = extra_mana.clone();
                        self.cards[card.0 as usize].cast_copies = n_copies;
                        if !after_cast.is_empty() {
                            self.cast_after.push((card, source, after_cast.clone()));
                        }
                        if *bottom_after {
                            self.cards[card.0 as usize].cast_bottom_after = true;
                        }
                        let alt_f = self.free_cast_alt(controller, card, *pay, *alt);
                        self.free_casts.push((controller, card, *pay, if *suspend { self.suspend_alt(card).or(alt_f) } else { alt_f }));
                    }
                    return Exec::Done;
                }
                if (self.cards[card.0 as usize].cur.types.has(CardType::Land) && !*lands) || matches!(self.cards[card.0 as usize].zone, Zone::Stack | Zone::Battlefield) {
                    return Exec::Done;
                }
                if self.cards[card.0 as usize].zone == Zone::Exile {
                    // a spell with a required target that has no legal target can't be cast: it isn't offered (discover: it goes to hand)
                    let (specs, modal) = self.spell_specs(card);
                    let no_target = (modal.is_none() && !specs.is_empty() && !self.specs_satisfiable(&specs, card, controller)) || self.aura_hosts(card).map_or(false, |h| h.is_empty());
                    if no_target {
                        if *decline_to_hand {
                            self.move_card(card, Zone::Hand, Default::default());
                        }
                        return Exec::Done;
                    }
                }
                ctx.sub = Some(Sub::FreeCast { player: controller, card });
                self.pending = Some(Decision {
                    player: controller,
                    kind: DecisionKind::YesNo { prompt: if *pay { format!("Cast {}?", self.def_of(card).name) } else { format!("Cast {} without paying its mana cost?", self.def_of(card).name) }, source: Some(source) },
                });
                Exec::Wait
            }
            Effect::ExileSpellOnResolve => {
                if self.cards[source.0 as usize].zone == Zone::Stack && !self.exile_marked.contains(&source) {
                    self.exile_marked.push(source);
                }
                Exec::Done
            }
            Effect::CastFreeAny { what, copy, pay } => {
                let mut rest: Vec<CardId>;
                if let Some(Sub::FreeCastAny { player, card, rest: r }) = ctx.sub.clone() {
                    let ans = self.answer.take();
                    ctx.sub = None;
                    if let Some(Choice::YesNo { yes: true }) = ans {
                        let card = if *copy { self.make_cast_copy(card, player) } else { card };
                        self.free_casts.push((player, card, *pay, None));
                        // the cast card is no longer among "the rest" (Glorious Purpose)
                        ctx.remembered.retain(|o| *o != Obj::card(card));
                    }
                    rest = r;
                } else {
                    rest = {
                        let c = ec!();
                        self.sel_cards(what, &c)
                    };
                    rest.reverse();
                }
                while let Some(card) = rest.pop() {
                    let cd = &self.cards[card.0 as usize];
                    if cd.cur.types.has(CardType::Land) || matches!(cd.zone, Zone::Stack | Zone::Battlefield) {
                        continue;
                    }
                    ctx.sub = Some(Sub::FreeCastAny { player: controller, card, rest });
                    self.pending = Some(Decision {
                        player: controller,
                        kind: DecisionKind::YesNo { prompt: if *pay { format!("Cast {}?", self.def_of(card).name) } else { format!("Cast {} without paying its mana cost?", self.def_of(card).name) }, source: Some(source) },
                    });
                    return Exec::Wait;
                }
                Exec::Done
            }
            Effect::ExileUntilOpponentMonarch { what } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                for id in cs {
                    if self.move_card(id, Zone::Exile, MoveOpts::default()) {
                        self.monarch_exiles.push((id, self.cards[id.0 as usize].zcc, controller));
                    }
                }
                Exec::Done
            }
            Effect::ReturnLinkedExiles => {
                let old_zcc = self.cards[source.0 as usize].zcc.saturating_sub(1);
                let back: Vec<(CardId, u32)> = self.manual_linked.iter().filter(|(s, z, _, _)| *s == source && *z == old_zcc).map(|(_, _, c, cz)| (*c, *cz)).collect();
                self.manual_linked.retain(|(s, z, _, _)| !(*s == source && *z == old_zcc));
                for (c, cz) in back {
                    if self.cards[c.0 as usize].zone == Zone::Exile && self.cards[c.0 as usize].zcc == cz {
                        self.move_card(c, Zone::Battlefield, MoveOpts::default());
                    }
                }
                Exec::Done
            }
            Effect::ReshuffleAndReveal => {
                let order = self.apnap();
                let mut counts = vec![0usize; self.players.len()];
                let owned: Vec<CardId> = self.battlefield.clone();
                for id in &owned {
                    counts[self.cards[id.0 as usize].owner as usize] += 1;
                }
                for id in owned {
                    if self.cards[id.0 as usize].zone == Zone::Battlefield {
                        self.move_card(id, Zone::Library, Default::default());
                    }
                }
                let mut revealed: Vec<Obj> = vec![];
                for &p in &order {
                    self.shuffle_library(p);
                    let top: Vec<CardId> = self.players[p as usize].library.iter().rev().take(counts[p as usize]).copied().collect();
                    for &id in &top {
                        self.log(format!("Player {} reveals {}", p, self.def_of(id).name));
                    }
                    revealed.extend(top.into_iter().map(Obj::card));
                }
                ctx.remembered = revealed;
                Exec::Done
            }
            Effect::Explore { what } => {
                let mut queue: Vec<CardId>;
                let mut scried: Option<CardId> = None;
                if let Some(Sub::ExploreScry { creature, cards, queue: q }) = ctx.sub.clone() {
                    let ans = self.answer.take();
                    ctx.sub = None;
                    let p = self.cards[creature.0 as usize].cur.controller;
                    let mut bottom_n = 0u32;
                    if let Some(Choice::Pick { indices }) = ans {
                        bottom_n = indices.len() as u32;
                        for id in indices.iter().filter_map(|i| cards.get(*i).copied()).collect::<Vec<_>>() {
                            self.players[p as usize].library.retain(|x| *x != id);
                            self.players[p as usize].library.insert(0, id);
                        }
                    }
                    self.players[p as usize].scried_turn = true;
                    self.emit(Event::Scry { player: p, n: cards.len() as u32, surveil: false, bottom: bottom_n });
                    queue = q;
                    queue.push(creature);
                    scried = Some(creature);
                } else if let Some(Sub::ExploreKeep { card, queue: q, .. }) = ctx.sub.clone() {
                    let ans = self.answer.take();
                    ctx.sub = None;
                    if let Some(Choice::YesNo { yes: true }) = ans {
                        self.move_card(card, Zone::Graveyard, Default::default());
                    }
                    queue = q;
                } else {
                    let cs = {
                        let c = ec!();
                        self.sel_cards(what, &c)
                    };
                    queue = vec![];
                    for id in cs.into_iter().rev() {
                        for _ in 0..(1 + self.explore_extra(id)) {
                            queue.push(id);
                        }
                    }
                }
                while let Some(id) = queue.pop() {
                    let p = self.cards[id.0 as usize].cur.controller;
                    if scried.take() != Some(id) {
                        let k = self.explore_scry(id) as usize;
                        let take = k.min(self.players[p as usize].library.len());
                        if take > 0 {
                            let cards: Vec<CardId> = self.players[p as usize].library.iter().rev().take(take).copied().collect();
                            ctx.sub = Some(Sub::ExploreScry { creature: id, cards: cards.clone(), queue: queue.clone() });
                            self.pending = Some(Decision {
                                player: p,
                                kind: DecisionKind::Pick {
                                    prompt: "Scry: choose cards to put on the bottom".into(),
                                    source: Some(source),
                                    options: cards.iter().map(|c| Opt::Obj { obj: Obj::card(*c) }).collect(),
                                    min: 0,
                                    max: take as u8,
                                    cancellable: false,
                                    hidden: true,
                                    purpose: PickPurpose::Scry,
                                },
                            });
                            return Exec::Wait;
                        }
                    }
                    let revealed = self.players[p as usize].library.last().copied();
                    self.emit(Event::Explored { card: id, revealed });
                    if let Some(&top) = self.players[p as usize].library.last() {
                        if self.cards[top.0 as usize].cur.types.has(CardType::Land) {
                            self.move_card(top, Zone::Hand, Default::default());
                        } else {
                            self.add_counters_to(Obj::card(id), CounterKind::PlusOnePlusOne, 1);
                            ctx.sub = Some(Sub::ExploreKeep { player: p, card: top, queue: queue.clone() });
                            self.pending = Some(Decision {
                                player: p,
                                kind: DecisionKind::YesNo { prompt: format!("Put {} into your graveyard? (otherwise it stays on top)", self.def_of(top).name), source: Some(source) },
                            });
                            return Exec::Wait;
                        }
                    }
                }
                Exec::Done
            }
            Effect::Connive { what } => {
                let mut queue: Vec<CardId>;
                if let Some(Sub::Connive { creature, player, queue: q }) = ctx.sub.clone() {
                    let ans = self.answer.take();
                    ctx.sub = None;
                    queue = q;
                    if let Some(Choice::Pick { indices }) = ans {
                        let hand = self.players[player as usize].hand.clone();
                        if let Some(&id) = indices.first().and_then(|i| hand.get(*i)) {
                            self.connive_discard(creature, player, id);
                        }
                    }
                    self.emit(Event::Connived { card: creature });
                } else {
                    queue = {
                        let c = ec!();
                        self.sel_cards(what, &c).into_iter().rev().collect()
                    };
                }
                while let Some(creature) = queue.pop() {
                    // a creature that left the battlefield still connives (draw, discard; no counter) using last-known information
                    let p = self.cards[creature.0 as usize].cur.controller;
                    for _ in 0..self.connive_draw(creature) {
                        self.draw_with_replacement(p, false, false);
                    }
                    self.draw_with_replacement(p, false, false);
                    self.flush_followups();
                    let hand = self.players[p as usize].hand.clone();
                    if hand.is_empty() {
                        self.emit(Event::Connived { card: creature });
                        continue;
                    }
                    if hand.len() == 1 {
                        self.connive_discard(creature, p, hand[0]);
                        self.emit(Event::Connived { card: creature });
                        continue;
                    }
                    ctx.sub = Some(Sub::Connive { creature, player: p, queue: queue.clone() });
                    self.pending = Some(Decision {
                        player: p,
                        kind: DecisionKind::Pick {
                            prompt: "Connive: discard a card".to_string(),
                            source: Some(source),
                            options: hand.iter().map(|c| Opt::Obj { obj: Obj::card(*c) }).collect(),
                            min: 1,
                            max: 1,
                            cancellable: false,
                            hidden: true,
                            purpose: PickPurpose::Discard,
                        },
                    });
                    return Exec::Wait;
                }
                Exec::Done
            }
            Effect::DrawThenDiscard { who, draw, discard } => {
                if let Some(Sub::LootDiscard { player, n }) = ctx.sub.clone() {
                    let ans = self.answer.take();
                    ctx.sub = None;
                    if let Some(Choice::Pick { indices }) = ans {
                        let hand = self.players[player as usize].hand.clone();
                        for i in indices.iter().take(n.max(0) as usize) {
                            if let Some(&id) = hand.get(*i) {
                                self.move_card(id, Zone::Graveyard, Default::default());
                                self.emit(Event::Discarded { card: id, player, by: self.cause_ctl.get() });
                            }
                        }
                    }
                    return Exec::Done;
                }
                let (ps, d, x) = {
                    let c = ec!();
                    (self.sel_players(who, &c), self.eval_amount(draw, &c), self.eval_amount(discard, &c))
                };
                let Some(&p) = ps.first() else { return Exec::Done };
                for _ in 0..d.max(0) {
                    self.draw_with_replacement(p, false, false);
                }
                self.flush_followups();
                let hand = self.players[p as usize].hand.clone();
                if x <= 0 || hand.is_empty() {
                    return Exec::Done;
                }
                if hand.len() as i32 <= x {
                    for id in hand {
                        self.move_card(id, Zone::Graveyard, Default::default());
                        self.emit(Event::Discarded { card: id, player: p, by: self.cause_ctl.get() });
                    }
                    return Exec::Done;
                }
                ctx.sub = Some(Sub::LootDiscard { player: p, n: x });
                self.pending = Some(Decision {
                    player: p,
                    kind: DecisionKind::Pick {
                        prompt: format!("Discard {x} card(s)"),
                        source: Some(source),
                        options: hand.iter().map(|c| Opt::Obj { obj: Obj::card(*c) }).collect(),
                        min: x as u8,
                        max: x as u8,
                        cancellable: false,
                        hidden: true,
                        purpose: PickPurpose::Discard,
                    },
                });
                Exec::Wait
            }
            Effect::If { cond, then, otherwise } => {
                let ok = {
                    let c = ec!();
                    self.eval_cond(cond, &c)
                };
                let list = if ok { then } else { otherwise };
                if list.is_empty() {
                    Exec::Done
                } else {
                    Exec::Push(Frame { list: Arc::new(list.clone()), pc: 0, repeat: 0, each: vec![], each_active: false, tbase: ctx.cur_tbase })
                }
            }
            Effect::Vote { .. } | Effect::VoteForCards { .. } | Effect::VoteForPlayers { .. } => {
                let (queue, counts, cards, ballots) = if let Some(Sub::Vote { mut queue, mut counts, cards, mut ballots }) = ctx.sub.clone() {
                    let ans = self.answer.take();
                    ctx.sub = None;
                    let idx = match ans {
                        Some(Choice::Pick { indices }) => indices.first().copied().unwrap_or(0),
                        _ => 0,
                    };
                    // optional extra ballots (high bit set) can be declined by picking the extra "abstain" option
                    let optional = queue.first().map_or(false, |q| q & 0x80 != 0) || matches!(e, Effect::VoteForCards { optional: true, .. });
                    let voter = queue.first().map_or(0, |q| q & 0x7f);
                    // for a vote for players the picked index is an index into the voter's eligible players
                    let elig: Vec<usize> = match e {
                        Effect::VoteForPlayers { others, .. } => (0..self.players.len()).filter(|&q| !*others || q as PlayerId != voter).collect(),
                        _ => (0..counts.len()).collect(),
                    };
                    if !(optional && idx >= elig.len()) {
                        if let Some(&slot) = elig.get(idx) {
                            if let Some(c) = counts.get_mut(slot) {
                                *c += 1;
                            }
                            if !queue.is_empty() {
                                ballots.push((voter, slot));
                            }
                        }
                    }
                    if !queue.is_empty() {
                        queue.remove(0);
                    }
                    (queue, counts, cards, ballots)
                } else {
                    let cards: Vec<CardId> = match e {
                        Effect::VoteForCards { from, .. } => {
                            let c = ec!();
                            self.sel_cards(from, &c)
                        }
                        _ => vec![],
                    };
                    let n = match e {
                        Effect::Vote { options, .. } => options.len(),
                        Effect::VoteForPlayers { .. } => self.players.len(),
                        _ => cards.len(),
                    };
                    let mut order = self.apnap();
                    if let Some(i) = order.iter().position(|&q| q == controller) {
                        order.rotate_left(i);
                    }
                    if matches!(e, Effect::Vote { opponents_only: true, .. }) {
                        order.retain(|&q| q != controller);
                    }
                    if n == 0 {
                        return Exec::Done;
                    }
                    // "you may vote an additional time" statics
                    let mut with_extra: Vec<PlayerId> = vec![];
                    for &pl in &order {
                        with_extra.push(pl);
                        let extra = self
                            .battlefield
                            .iter()
                            .filter(|id| {
                                let c = &self.cards[id.0 as usize];
                                c.cur.controller == pl && !c.cur.no_abilities && self.def_of(**id).statics.iter().any(|&i| matches!(&self.def_of(**id).abilities[i as usize], Ability::Static { effect: StaticEffect::ExtraVote, .. }))
                            })
                            .count();
                        for _ in 0..extra {
                            with_extra.push(pl);
                        }
                        let opt_extra = self
                            .battlefield
                            .iter()
                            .filter(|id| {
                                let c = &self.cards[id.0 as usize];
                                c.cur.controller == pl && !c.cur.no_abilities && self.def_of(**id).statics.iter().any(|&i| matches!(&self.def_of(**id).abilities[i as usize], Ability::Static { effect: StaticEffect::OptionalExtraVote, .. }))
                            })
                            .count();
                        for _ in 0..opt_extra {
                            with_extra.push(pl | 0x80);
                        }
                    }
                    (with_extra, vec![0; n], cards, vec![])
                };
                if let Some(&voter_raw) = queue.first() {
                    let voter = voter_raw & 0x7f;
                    let mut options: Vec<Opt> = match e {
                        Effect::Vote { options, .. } => options.iter().map(|t| Opt::Text { text: t.clone() }).collect(),
                        Effect::VoteForPlayers { others, .. } => (0..self.players.len() as PlayerId).filter(|&q| !*others || q != voter).map(|q| Opt::Obj { obj: Obj::player(q) }).collect(),
                        _ => cards.iter().map(|c| Opt::Obj { obj: Obj::card(*c) }).collect(),
                    };
                    if voter_raw & 0x80 != 0 {
                        options.push(Opt::Text { text: "Don't vote an additional time".into() });
                    } else if matches!(e, Effect::VoteForCards { optional: true, .. }) {
                        options.push(Opt::Text { text: "Don't vote for a creature".into() });
                    }
                    let secret = matches!(e, Effect::Vote { secret: true, .. } | Effect::VoteForCards { secret: true, .. } | Effect::VoteForPlayers { secret: true, .. });
                    ctx.sub = Some(Sub::Vote { queue, counts, cards, ballots });
                    let decider = self.vote_control.iter().rev().find(|(_, t)| *t == self.turn).map_or(voter, |(c, _)| *c);
                    self.pending = Some(Decision {
                        player: decider,
                        kind: DecisionKind::Pick { prompt: format!("{}: vote", self.def_of(source).name), source: Some(source), options, min: 1, max: 1, cancellable: false, hidden: secret, purpose: PickPurpose::Generic },
                    });
                    return Exec::Wait;
                }
                self.last_ballots = ballots;
                self.emit(Event::VotingFinished { player: controller });
                match e {
                    Effect::Vote { .. } | Effect::VoteForPlayers { .. } => self.last_votes = counts,
                    _ => {
                        let best = counts.iter().copied().max().unwrap_or(0);
                        let all = matches!(e, Effect::VoteForCards { all_voted: true, .. });
                        ctx.remembered = if best > 0 { cards.iter().zip(counts.iter()).filter(|(_, c)| if all { **c > 0 } else { **c == best }).map(|(c, _)| Obj::card(*c)).collect() } else { vec![] };
                        self.last_card_votes = cards.iter().copied().zip(counts.iter().copied()).collect();
                    }
                }
                Exec::Done
            }
            Effect::Venture { dungeon } => {
                if ctx.sub.is_none() {
                    if self.venture_barred(controller) {
                        return Exec::Done;
                    }
                    let vt = &mut self.players[controller as usize].venture_turn;
                    *vt = if vt.0 == self.turn { (vt.0, vt.1.saturating_add(1)) } else { (self.turn, 1) };
                }
                if let Some(Sub::Venture { options, new_dungeon }) = ctx.sub.clone() {
                    ctx.sub = None;
                    let idx = match self.answer.take() {
                        Some(Choice::Pick { indices }) => indices.first().copied().unwrap_or(0),
                        _ => 0,
                    };
                    let Some(name) = options.get(idx).cloned() else { return Exec::Done };
                    self.venture_to(controller, &name, new_dungeon);
                    return Exec::Done;
                }
                // finished dungeon: its card leaves and the player starts over
                if let Some((dc, room)) = self.players[controller as usize].dungeon.clone() {
                    let next = self.room_next(dc, &room);
                    if !next.is_empty() {
                        if next.len() == 1 {
                            self.venture_to(controller, &next[0], false);
                            return Exec::Done;
                        }
                        ctx.sub = Some(Sub::Venture { options: next.clone(), new_dungeon: false });
                        self.pending = Some(Decision {
                            player: controller,
                            kind: DecisionKind::Pick {
                                prompt: "Venture: choose the next room".into(),
                                source: Some(source),
                                options: next.into_iter().map(|t| Opt::Text { text: t }).collect(),
                                min: 1,
                                max: 1,
                                cancellable: false,
                                hidden: false,
                                purpose: PickPurpose::Generic,
                            },
                        });
                        return Exec::Wait;
                    }
                    // in the last room already: that dungeon is done, start another
                    self.players[controller as usize].dungeon = None;
                    if self.cards[dc.0 as usize].zone == Zone::Command {
                        self.move_card(dc, Zone::Graveyard, MoveOpts::default());
                    }
                }
                let options: Vec<String> = match dungeon {
                    Some(d) => vec![d.clone()],
                    None => vec!["Lost Mine of Phandelver".into(), "Tomb of Annihilation".into(), "Dungeon of the Mad Mage".into()],
                };
                if options.len() == 1 {
                    self.venture_to(controller, &options[0], true);
                    return Exec::Done;
                }
                ctx.sub = Some(Sub::Venture { options: options.clone(), new_dungeon: true });
                self.pending = Some(Decision {
                    player: controller,
                    kind: DecisionKind::Pick {
                        prompt: "Venture into which dungeon?".into(),
                        source: Some(source),
                        options: options.into_iter().map(|t| Opt::Text { text: t }).collect(),
                        min: 1,
                        max: 1,
                        cancellable: false,
                        hidden: false,
                        purpose: PickPurpose::Generic,
                    },
                });
                Exec::Wait
            }
            Effect::RingTempts => {
                if let Some(Sub::RingPick { options }) = ctx.sub.clone() {
                    ctx.sub = None;
                    let idx = match self.answer.take() {
                        Some(Choice::Pick { indices }) => indices.first().copied().unwrap_or(0),
                        _ => 0,
                    };
                    if let Some(&b) = options.get(idx) {
                        self.players[controller as usize].ring_bearer = Some(b);
                        self.layers_dirty = true;
                        self.recompute_layers();
                        self.emit(Event::RingBearerChosen { player: controller, card: b });
                    }
                    self.emit(Event::RingTempted { player: controller });
                    return Exec::Done;
                }
                // the Ring gets stronger: swap the emblem for the next level's
                let lvl = (self.players[controller as usize].ring_level + 1).min(4);
                self.players[controller as usize].ring_level = lvl;
                if let Some(old) = self.players[controller as usize].ring_emblem.take() {
                    self.move_card(old, Zone::Graveyard, MoveOpts::default());
                }
                if let Some(def) = self.db.find(&format!("The Ring {lvl}")) {
                    let id = self.create_card(def, controller, Zone::Command, true);
                    self.cards[id.0 as usize].controller = controller;
                    self.players[controller as usize].ring_emblem = Some(id);
                }
                self.layers_dirty = true;
                let options: Vec<CardId> = self
                    .battlefield
                    .iter()
                    .copied()
                    .filter(|id| {
                        let c = &self.cards[id.0 as usize];
                        c.cur.controller == controller && c.cur.types.has(CardType::Creature)
                    })
                    .collect();
                match options.len() {
                    0 => {
                        self.emit(Event::RingTempted { player: controller });
                        Exec::Done
                    }
                    1 => {
                        let b = options[0];
                        self.players[controller as usize].ring_bearer = Some(b);
                        self.layers_dirty = true;
                        self.recompute_layers();
                        self.emit(Event::RingBearerChosen { player: controller, card: b });
                        self.emit(Event::RingTempted { player: controller });
                        Exec::Done
                    }
                    _ => {
                        ctx.sub = Some(Sub::RingPick { options: options.clone() });
                        self.pending = Some(Decision {
                            player: controller,
                            kind: DecisionKind::Pick {
                                prompt: "Choose your Ring-bearer".into(),
                                source: Some(source),
                                options: options.iter().map(|c| Opt::Obj { obj: Obj::card(*c) }).collect(),
                                min: 1,
                                max: 1,
                                cancellable: false,
                                hidden: false,
                                purpose: PickPurpose::Generic,
                            },
                        });
                        Exec::Wait
                    }
                }
            }
            Effect::Clash { win, lose } => {
                let (opp, mine, theirs, won, stage) = if let Some(Sub::Clash { opp, mine, theirs, won, stage }) = ctx.sub.clone() {
                    // the answer to the previous question
                    let ans = self.answer.take();
                    ctx.sub = None;
                    let (who, card) = if stage == 0 { (controller, mine) } else { (opp, theirs) };
                    if let (Some(Choice::YesNo { yes: true }), Some(c)) = (ans, card) {
                        let lib = &mut self.players[who as usize].library;
                        if let Some(i) = lib.iter().position(|&x| x == c) {
                            lib.remove(i);
                            lib.insert(0, c);
                        }
                    }
                    (opp, mine, theirs, won, stage + 1)
                } else {
                    // "clash with an opponent of your choice"
                    let opp = if let Some(Sub::ClashPick { options }) = ctx.sub.clone() {
                        ctx.sub = None;
                        let i = match self.answer.take() {
                            Some(Choice::Pick { indices }) => indices.first().copied().unwrap_or(0),
                            _ => 0,
                        };
                        options.get(i).or(options.first()).copied()
                    } else {
                        let options: Vec<PlayerId> = self.opponents(controller).into_iter().filter(|p| self.alive(*p)).collect();
                        if options.len() > 1 {
                            ctx.sub = Some(Sub::ClashPick { options: options.clone() });
                            self.pending = Some(Decision {
                                player: controller,
                                kind: DecisionKind::Pick {
                                    prompt: "Clash with which opponent?".into(),
                                    source: Some(source),
                                    options: options.iter().map(|p| Opt::Obj { obj: Obj::player(*p) }).collect(),
                                    min: 1,
                                    max: 1,
                                    cancellable: false,
                                    hidden: false,
                                    purpose: PickPurpose::Generic,
                                },
                            });
                            return Exec::Wait;
                        }
                        options.first().copied()
                    };
                    let Some(opp) = opp else { return Exec::Done };
                    // the opponent clashed with is the source's chosen player ("that player gains control": Captivating Glance)
                    self.cards[source.0 as usize].chosen_player = Some(opp);
                    let mine = self.players[controller as usize].library.last().copied();
                    let theirs = self.players[opp as usize].library.last().copied();
                    let mv = |g: &Game, c: Option<CardId>| c.map_or(-1, |c| g.def_of(c).mana_value as i32);
                    let (a, b) = (mv(self, mine), mv(self, theirs));
                    self.log(format!("clash: {} vs {}", a, b));
                    let won = a > b;
                    self.emit(Event::Clashed { player: controller, won });
                    self.emit(Event::Clashed { player: opp, won: b > a });
                    (opp, mine, theirs, won, 0)
                };
                // ask each player in turn whether to bottom their revealed card
                let mut stage = stage;
                loop {
                    let (who, card) = if stage == 0 { (controller, mine) } else { (opp, theirs) };
                    if stage > 1 {
                        break;
                    }
                    if let Some(c) = card {
                        ctx.sub = Some(Sub::Clash { opp, mine, theirs, won, stage });
                        self.pending = Some(Decision {
                            player: who,
                            kind: DecisionKind::YesNo { prompt: format!("Clash: put {} on the bottom of your library?", self.def_of(c).name), source: Some(source) },
                        });
                        return Exec::Wait;
                    }
                    stage += 1;
                }
                let list = if won { win } else { lose };
                if list.is_empty() {
                    return Exec::Done;
                }
                Exec::Push(Frame { list: Arc::new(list.clone()), pc: 0, repeat: 0, each: vec![], each_active: false, tbase: ctx.cur_tbase })
            }
            Effect::May { effects, who, otherwise, remember_answer } => {
                let decider = match who {
                    Some(sel) => {
                        let c = ec!();
                        self.sel_players(sel, &c).first().copied().unwrap_or(controller)
                    }
                    None => controller,
                };
                if ctx.sub.is_some() {
                    let ans = self.answer.take();
                    ctx.sub = None;
                    if *remember_answer {
                        self.last_cost_paid = matches!(ans, Some(Choice::YesNo { yes: true }));
                    }
                    if let Some(Choice::YesNo { yes: true }) = ans {
                        if let ItemKind::Triggered { ability, optional: true } = ctx.item.kind {
                            if ability < crate::cast::GRANTED_BASE {
                                if let Some(Ability::Triggered { limit_per_turn, .. }) = self.db.def(self.cards[source.0 as usize].def).abilities.get(ability as usize) {
                                    if *limit_per_turn > 0 {
                                        let acts = &mut self.cards[source.0 as usize].activations;
                                        match acts.iter_mut().find(|(i, _)| *i == ability) {
                                            Some((_, n)) => *n = n.saturating_add(1),
                                            None => acts.push((ability, 1)),
                                        }
                                    }
                                }
                            }
                        }
                        return Exec::Push(Frame { list: Arc::new(effects.clone()), pc: 0, repeat: 0, each: vec![], each_active: false, tbase: ctx.cur_tbase });
                    }
                    // "once each turn, you may ...": declining an optional `limit_per_turn` trigger doesn't use up the limit (a `once_per_turn` one does: Ondu Spiritdancer)
                    if !otherwise.is_empty() {
                        return Exec::Push(Frame { list: Arc::new(otherwise.clone()), pc: 0, repeat: 0, each: vec![], each_active: false, tbase: ctx.cur_tbase });
                    }
                    return Exec::Done;
                }
                // an optional limited trigger ("Do this only once each turn") is skipped when an earlier one this turn was accepted
                if let ItemKind::Triggered { ability, optional: true } = ctx.item.kind {
                    if ability < crate::cast::GRANTED_BASE {
                        if let Some(Ability::Triggered { limit_per_turn, .. }) = self.db.def(self.cards[source.0 as usize].def).abilities.get(ability as usize) {
                            if *limit_per_turn > 0 && self.cards[source.0 as usize].activations.iter().any(|(i, n)| *i == ability && *n as usize >= *limit_per_turn as usize) {
                                return Exec::Done;
                            }
                        }
                    }
                }
                // "you may mill N" can't be chosen when the library holds fewer than N cards (701.17b)
                if let [Effect::Mill { who: mw, n: Amount::Fixed { n: mn } }] = effects.as_slice() {
                    let c = ec!();
                    let ps = self.sel_players(mw, &c);
                    if ps.len() == 1 && ps[0] == decider && self.players[decider as usize].library.len() < (*mn).max(0) as usize {
                        return Exec::Done;
                    }
                }
                ctx.sub = Some(Sub::May);
                let prompt = match self.may_text(source, &ctx.item.kind) {
                    Some(t) => format!("Do you want to use {}'s effect?\n{t}", self.def_of(source).name),
                    None => format!("Do you want to use {}'s effect?", self.def_of(source).name),
                };
                self.pending = Some(Decision { player: decider, kind: DecisionKind::YesNo { prompt, source: Some(source) } });
                Exec::Wait
            }
            Effect::MayPay { payer, cost, effects } => {
                // "you may pay {X}" with X chosen by an earlier ChooseNumber: "{N}" in the mana string is the chosen number
                let cn = self.cards[source.0 as usize].chosen_number.unwrap_or(0);
                let cost_n: Vec<Cost> = cost.iter().map(|c| match c { Cost::Mana { cost } if cost.contains("{N}") => Cost::Mana { cost: cost.replace("{N}", &format!("{{{cn}}}")) }, Cost::ManaAmount { amount } => { let c = ec!(); Cost::Mana { cost: format!("{{{}}}", self.eval_amount(amount, &c).max(0)) } } Cost::PayLifeX { amount } => { let c = ec!(); Cost::PayLife { n: self.eval_amount(amount, &c).max(0) } } Cost::PayEnergyX { amount } => { let c = ec!(); Cost::PayEnergy { n: self.eval_amount(amount, &c).max(0) } } o => o.clone() }).collect();
                // madness: the cost paid takes the cost increases (and reductions) that apply to the card into account (Thalia's tax)
                let cost_n: Vec<Cost> = if self.cards[source.0 as usize].zone == Zone::Exile
                    && effects.iter().any(|e| matches!(e, Effect::CastFree { what: Sel::This, pay: false, .. }))
                    && matches!(ctx.item.kind, ItemKind::Triggered { .. })
                {
                    let eff = self.effective_cost(controller, source, None);
                    let printed = self.db.def(self.cards[source.0 as usize].def).cost.clone();
                    let gen = |m: &ManaCost| -> i32 { m.shards.iter().map(|sh| if let Shard::Generic { n } = sh { *n as i32 } else { 0 }).sum() };
                    let delta = gen(&eff) - gen(&printed);
                    cost_n.into_iter().map(|c| match c {
                        Cost::Mana { cost } if delta > 0 => Cost::Mana { cost: format!("{cost}{{{delta}}}") },
                        other => other,
                    }).collect()
                } else {
                    cost_n
                };
                let cost = &cost_n;
                let mut paid_result: Option<bool> = None;
                if let Some(Sub::PayPick { unless: false, .. }) = ctx.sub.clone() {
                    match self.continue_pay(ctx, source) {
                        Some((paid, _)) => paid_result = Some(paid),
                        None => return Exec::Wait,
                    }
                } else if let Some(Sub::MayPayTimes { payer: p, .. }) = ctx.sub.clone() {
                    // "you may pay {1}{W} any number of times": scale the cost by the chosen number and pay it
                    let k = match self.answer.take() {
                        Some(Choice::Number { n }) => n.max(0),
                        _ => 0,
                    };
                    ctx.sub = None;
                    let x_cost = Self::may_pay_has_x(cost);
                    if k <= 0 && !x_cost {
                        return Exec::Done;
                    }
                    ctx.item.x = k;
                    // filters evaluated by this spell read the X from its card ("creatures with power equal to X")
                    if matches!(self.cards[source.0 as usize].zone, Zone::Stack | Zone::Command) {
                        self.cards[source.0 as usize].x = k;
                    }
                    let scaled = if x_cost { Self::subst_may_pay_x(cost, k) } else { Self::scale_may_pay_cost(cost, k) };
                    match self.pay_with_choices(ctx, p, source, scaled, false) {
                        Some(paid) => paid_result = Some(paid),
                        None => return Exec::Wait,
                    }
                } else if let Some(Sub::MayPay { payer: p }) = ctx.sub.clone() {
                    let ans = self.answer.take();
                    ctx.sub = None;
                    if let Some(Choice::YesNo { yes: true }) = ans {
                        if Self::may_pay_has_x(cost) {
                            // "you may pay {X}{R}": X is chosen (up to what can be afforded) before paying
                            let mut max = 0;
                            for k in 1..=30 {
                                if self.can_pay_costs(p, source, &Self::subst_may_pay_x(cost, k)) { max = k } else { break }
                            }
                            ctx.sub = Some(Sub::MayPayTimes { payer: p, max });
                            self.pending = Some(Decision {
                                player: p,
                                kind: DecisionKind::Number { prompt: format!("Choose X for {}", self.def_of(source).name), min: 0, max, cancellable: false },
                            });
                            return Exec::Wait;
                        }
                        if cost.iter().any(|c| matches!(c, Cost::XTimes { .. })) {
                            let mut max = 0;
                            for k in 1..=20 {
                                if self.can_pay_costs(p, source, &Self::scale_may_pay_cost(cost, k)) { max = k } else { break }
                            }
                            ctx.sub = Some(Sub::MayPayTimes { payer: p, max });
                            self.pending = Some(Decision {
                                player: p,
                                kind: DecisionKind::Number { prompt: format!("How many times do you pay for {}?", self.def_of(source).name), min: 0, max, cancellable: false },
                            });
                            return Exec::Wait;
                        }
                        match self.pay_with_choices(ctx, p, source, cost.clone(), false) {
                            Some(paid) => paid_result = Some(paid),
                            None => return Exec::Wait,
                        }
                    } else {
                        return Exec::Done;
                    }
                }
                if let Some(paid) = paid_result {
                    if paid {
                        self.last_cost_paid = true;
                        if !self.cost_sacrificed.is_empty() {
                            ctx.remembered = std::mem::take(&mut self.cost_sacrificed).into_iter().map(Obj::card).collect();
                        }
                        return Exec::Push(Frame { list: Arc::new(effects.clone()), pc: 0, repeat: 0, each: vec![], each_active: false, tbase: ctx.cur_tbase });
                    }
                    return Exec::Done;
                }
                self.cost_sacrificed.clear();
                self.last_cost_paid = false;
                let ps = {
                    let c = ec!();
                    self.sel_players(payer, &c)
                };
                let Some(&p) = ps.first() else { return Exec::Done };
                if !self.can_pay_costs(p, source, cost) {
                    return Exec::Done;
                }
                ctx.sub = Some(Sub::MayPay { payer: p });
                self.pending = Some(Decision {
                    player: p,
                    kind: DecisionKind::YesNo { prompt: format!("Pay for {}'s effect?", self.def_of(source).name), source: Some(source) },
                });
                Exec::Wait
            }
            Effect::OwnerChoosesLibraryPosition { what } => {
                let mut queue: Vec<CardId> = if let Some(Sub::LibPos { queue }) = ctx.sub.clone() {
                    ctx.sub = None;
                    let mut q = queue;
                    if let Some(Choice::YesNo { yes }) = self.answer.take() {
                        if !q.is_empty() {
                            let id = q.remove(0);
                            if self.cards[id.0 as usize].zone == Zone::Stack {
                                self.stack.retain(|s| s.source != id || s.copy);
                            }
                            self.move_card(id, Zone::Library, MoveOpts { bottom: !yes, ..Default::default() });
                        }
                    }
                    q
                } else {
                    let c = ec!();
                    self.sel_objs(what, &c)
                        .into_iter()
                        .filter_map(|o| match o {
                            Obj::Card { id } => Some(id),
                            Obj::Stack { id } => self.stack.iter().find(|s| s.id == id && !s.copy && matches!(s.kind, ItemKind::Spell { .. })).map(|s| s.source),
                            _ => None,
                        })
                        .collect()
                };
                queue.retain(|id| self.cards[id.0 as usize].zone != Zone::Library);
                if let Some(&id) = queue.first() {
                    let owner = self.cards[id.0 as usize].owner;
                    ctx.sub = Some(Sub::LibPos { queue });
                    self.pending = Some(Decision {
                        player: owner,
                        kind: DecisionKind::YesNo { prompt: format!("Put {} on the top of its owner's library? (No = bottom)", self.def_of(id).name), source: Some(source) },
                    });
                    return Exec::Wait;
                }
                Exec::Done
            }
            Effect::PutOnTopOwnersOrder { what } => {
                let mut queue: Vec<CardId> = if let Some(Sub::OwnerTop { queue, group }) = ctx.sub.clone() {
                    ctx.sub = None;
                    let mut order: Vec<CardId> = match self.answer.take() {
                        Some(Choice::Pick { indices }) => indices.iter().filter_map(|i| group.get(*i).copied()).collect(),
                        _ => vec![],
                    };
                    for c in &group {
                        if !order.contains(c) {
                            order.push(*c);
                        }
                    }
                    // the first card chosen ends up on top: put them back to front
                    for id in order.into_iter().rev() {
                        self.move_card(id, Zone::Library, MoveOpts::default());
                    }
                    queue
                } else {
                    let c = ec!();
                    let mut v = self.sel_cards(what, &c);
                    // players in turn order starting with the active player
                    let n = self.players.len();
                    let act = self.active as usize;
                    v.sort_by_key(|id| (self.cards[id.0 as usize].owner as usize + n - act) % n);
                    v
                };
                while let Some(&first) = queue.first() {
                    let owner = self.cards[first.0 as usize].owner;
                    let group: Vec<CardId> = queue.iter().copied().filter(|id| self.cards[id.0 as usize].owner == owner).collect();
                    queue.retain(|id| self.cards[id.0 as usize].owner != owner);
                    if group.len() == 1 {
                        self.move_card(first, Zone::Library, MoveOpts::default());
                        continue;
                    }
                    ctx.sub = Some(Sub::OwnerTop { queue, group: group.clone() });
                    self.pending = Some(Decision {
                        player: owner,
                        kind: DecisionKind::Pick {
                            prompt: "Put these cards on top of your library: choose the order, topmost first".into(),
                            source: Some(source),
                            options: group.iter().map(|c| Opt::Obj { obj: Obj::card(*c) }).collect(),
                            min: group.len() as u8,
                            max: group.len() as u8,
                            cancellable: false,
                            hidden: false,
                            purpose: PickPurpose::Generic,
                        },
                    });
                    return Exec::Wait;
                }
                Exec::Done
            }
            Effect::ReorderGraveyard { who } => {
                if let Some(Sub::GyOrder { player, cards }) = ctx.sub.clone() {
                    ctx.sub = None;
                    let mut order: Vec<CardId> = match self.answer.take() {
                        Some(Choice::Pick { indices }) => indices.iter().filter_map(|i| cards.get(*i).copied()).collect(),
                        _ => vec![],
                    };
                    for c in &cards {
                        if !order.contains(c) {
                            order.push(*c);
                        }
                    }
                    // the first card chosen is the topmost: the last entry of the list
                    order.reverse();
                    let gy = &mut self.players[player as usize].graveyard;
                    gy.retain(|c| !cards.contains(c));
                    gy.extend(order);
                    return Exec::Done;
                }
                let p = {
                    let c = ec!();
                    self.sel_players(who, &c).first().copied()
                };
                let Some(p) = p else { return Exec::Done };
                let cards: Vec<CardId> = self.players[p as usize].graveyard.iter().rev().copied().collect();
                if cards.len() < 2 {
                    return Exec::Done;
                }
                ctx.sub = Some(Sub::GyOrder { player: p, cards: cards.clone() });
                self.pending = Some(Decision {
                    player: controller,
                    kind: DecisionKind::Pick {
                        prompt: "Reorder the graveyard: choose the order, topmost first".into(),
                        source: Some(source),
                        options: cards.iter().map(|c| Opt::Obj { obj: Obj::card(*c) }).collect(),
                        min: cards.len() as u8,
                        max: cards.len() as u8,
                        cancellable: false,
                        hidden: false,
                        purpose: PickPurpose::Generic,
                    },
                });
                Exec::Wait
            }
            Effect::NextSpellCascade { who } => {
                let ps = {
                    let c = ec!();
                    self.sel_players(who, &c)
                };
                for p in ps {
                    self.players[p as usize].cascade_turn = self.turn;
                }
                Exec::Done
            }
            Effect::SectorBlocking => {
                self.sector_block_turn = self.turn;
                Exec::Done
            }
            Effect::AssignSector { what } => {
                let mut queue: Vec<CardId>;
                if let Some(Sub::SectorPick { queue: q, cur }) = ctx.sub.clone() {
                    ctx.sub = None;
                    queue = q;
                    let n = match self.answer.take() {
                        Some(Choice::Pick { indices }) => indices.first().copied().unwrap_or(0) as u8 + 1,
                        _ => 1,
                    };
                    if self.cards[cur.0 as usize].zone == Zone::Battlefield {
                        self.cards[cur.0 as usize].sector = n.clamp(1, 3);
                    }
                } else {
                    let c = ec!();
                    let mut v: Vec<CardId> = self.sel_cards(what, &c).into_iter().filter(|id| {
                        let k = &self.cards[id.0 as usize];
                        k.zone == Zone::Battlefield && k.cur.types.has(CardType::Creature) && k.sector == 0
                    }).collect();
                    // the opponents of the sculptor's controller assign theirs first
                    v.sort_by_key(|id| self.cards[id.0 as usize].cur.controller == controller);
                    queue = v;
                }
                while !queue.is_empty() {
                    let cur = queue.remove(0);
                    if self.cards[cur.0 as usize].sector != 0 || self.cards[cur.0 as usize].zone != Zone::Battlefield {
                        continue;
                    }
                    let chooser = self.cards[cur.0 as usize].cur.controller;
                    ctx.sub = Some(Sub::SectorPick { queue, cur });
                    self.pending = Some(Decision {
                        player: chooser,
                        kind: DecisionKind::Pick {
                            prompt: format!("Assign {} to a sector", self.def_of(cur).name),
                            source: Some(source),
                            options: ["Alpha", "Beta", "Gamma"].iter().map(|t| Opt::Text { text: t.to_string() }).collect(),
                            min: 1,
                            max: 1,
                            cancellable: false,
                            hidden: false,
                            purpose: PickPurpose::Generic,
                        },
                    });
                    return Exec::Wait;
                }
                Exec::Done
            }
            Effect::HandSpellsCascade { who } => {
                let ps = {
                    let c = ec!();
                    self.sel_players(who, &c)
                };
                for p in ps {
                    self.players[p as usize].hand_cascade_turn = self.turn;
                }
                Exec::Done
            }
            Effect::SpendAnyTypeForOneSpell => {
                self.players[controller as usize].any_type_turn = self.turn;
                Exec::Done
            }
            Effect::GainActivatedAbilitiesOf { what, from, duration, pay_with } => {
                let (recips, froms) = {
                    let c = ec!();
                    (self.sel_cards(what, &c), self.sel_cards(from, &c))
                };
                let db = self.db.clone();
                let mut abs: Vec<Ability> = vec![];
                for fid in froms {
                    for a in &db.def(self.cards[fid.0 as usize].def).abilities {
                        if let Ability::Activated { loyalty: None, .. } = a {
                            abs.push(a.clone());
                        }
                    }
                }
                for mut a in abs {
                    if let (Some(col), Ability::Activated { cost, .. }) = (pay_with, &mut a) {
                        let cl = format!("{:?}", col);
                        for part in cost.iter_mut() {
                            if let Cost::Mana { cost: m } = part {
                                for sym in ["W", "B", "R", "G", "U"] {
                                    if sym != cl {
                                        *m = m.replace(&format!("{{{sym}}}"), &format!("{{{sym}/{cl}}}"));
                                    }
                                }
                            }
                        }
                    }
                    self.add_effect(source, controller, &recips, Modif::Static(Box::new(StaticEffect::GrantAbility { affects: (Filter::default()).into(), ability: Box::new(a) })), *duration);
                }
                Exec::Done
            }
            Effect::Ungoad { what } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                for id in &cs {
                    self.goaded.retain(|(c, _, _, _)| c != id);
                    let zcc = self.cards[id.0 as usize].zcc;
                    for e in self.effects.iter_mut() {
                        if matches!(e.modif, Modif::Keyword(Keyword::MustAttack)) && matches!(e.duration, Duration::UntilYourNextTurn | Duration::Forever) {
                            e.affects.retain(|(c, z)| !(c == id && *z == zcc));
                        }
                    }
                }
                self.effects.retain(|e| !e.affects.is_empty() || !matches!(e.modif, Modif::Keyword(Keyword::MustAttack)));
                self.layers_dirty = true;
                Exec::Done
            }
            Effect::Detain { what } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                for id in cs {
                    if self.cards[id.0 as usize].zone != Zone::Battlefield {
                        continue;
                    }
                    self.cards[id.0 as usize].detained_by = Some(controller);
                    self.add_effect(source, controller, &[id], Modif::CantAttack, Duration::UntilYourNextTurn);
                    self.add_effect(source, controller, &[id], Modif::CantBlock, Duration::UntilYourNextTurn);
                }
                Exec::Done
            }
            Effect::Haunt { what } => {
                let t = {
                    let c = ec!();
                    self.sel_cards(what, &c).first().copied()
                };
                if let Some(t) = t {
                    if self.cards[t.0 as usize].zone == Zone::Battlefield && matches!(self.cards[source.0 as usize].zone, Zone::Graveyard | Zone::Stack) {
                        self.move_card(source, Zone::Exile, Default::default());
                        self.cards[source.0 as usize].haunting = Some(t);
                    }
                }
                Exec::Done
            }
            Effect::MutateOnto { target } => {
                let m = source;
                if let Some(Sub::MutateOver { t }) = ctx.sub.clone() {
                    ctx.sub = None;
                    let over = matches!(self.answer.take(), Some(Choice::YesNo { yes: true }));
                    if self.cards[t.0 as usize].zone == Zone::Battlefield && self.cards[m.0 as usize].zone == Zone::Stack {
                        self.merge_cards(t, m, over);
                    }
                    return Exec::Done;
                }
                let t = {
                    let c = ec!();
                    self.sel_cards(target, &c).first().copied()
                };
                let Some(t) = t else { return Exec::Done };
                if self.cards[t.0 as usize].zone != Zone::Battlefield || !self.cards[t.0 as usize].cur.types.has(CardType::Creature) || self.cards[m.0 as usize].zone != Zone::Stack {
                    return Exec::Done;
                }
                ctx.sub = Some(Sub::MutateOver { t });
                self.pending = Some(Decision {
                    player: controller,
                    kind: DecisionKind::YesNo { prompt: format!("Put {} on top of {}? (No = underneath)", self.def_of(m).name, self.def_of(t).name), source: Some(m) },
                });
                Exec::Wait
            }
            Effect::MayCastFreeFromGraveyardThisTurn { what } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                for id in cs {
                    if self.cards[id.0 as usize].zone == Zone::Graveyard && !self.def_of(id).types.has(CardType::Land) {
                        let z = self.cards[id.0 as usize].zcc;
                        self.gy_free_grants.push((controller, self.turn, id, z));
                    }
                }
                Exec::Done
            }
            Effect::MayPlayThisTurn { zone, filter, of, any_mana, tapped, exile_after, snapshot } => {
                if *exile_after {
                    self.gy_exile_grants.push((controller, self.turn, filter.clone().into_filter(), source));
                }
                if let Some(of) = of {
                    let owners = {
                        let c = ec!();
                        self.sel_players(of, &c)
                    };
                    for o in owners {
                        if *zone == Zone::Hand {
                            self.hand_play_grants.push((controller, o, self.turn));
                            continue;
                        }
                        self.gy_play_grants.push((controller, o, self.turn, filter.clone().into_filter(), source, *any_mana));
                    }
                    return Exec::Done;
                }
                // "among the remembered cards" is fixed now (the filter is checked later, at cast time)
                let mut f = filter.clone();
                let mut ids: Vec<CardId> = vec![];
                if f.among_remembered {
                    f.among_remembered = false;
                    ids = ctx.remembered.iter().filter_map(|o| if let Obj::Card { id } = o { Some(*id) } else { None }).collect();
                    if ids.is_empty() {
                        return Exec::Done;
                    }
                }
                if *snapshot && ids.is_empty() {
                    let fl = f.clone().into_filter();
                    let pool: Vec<CardId> = match zone {
                        Zone::Graveyard => self.players[controller as usize].graveyard.clone(),
                        Zone::Exile => self.players[controller as usize].exile.clone(),
                        Zone::Hand => self.players[controller as usize].hand.clone(),
                        _ => vec![],
                    };
                    ids = pool.into_iter().filter(|c| self.matches_card(&fl, *c, source, controller)).collect();
                    if ids.is_empty() {
                        return Exec::Done;
                    }
                }
                self.turn_play_grants.push((controller, *zone, f.into_filter(), source, self.turn, ids, *tapped));
                Exec::Done
            }
            Effect::FetchOutsideGame { who, filter, reveal, max } => {
                let mut then = vec![];
                if *reveal {
                    then.push(Effect::Reveal { what: Sel::Remembered });
                }
                then.push(Effect::Move { what: Sel::Remembered, to: Zone::Hand, top_of_library: false, tapped: false, under_your_control: false, remember: false, transformed: false, from: None, discard: false, position: None, position_x: false, ordered: false, bottom_ordered: false, order_chooser: None, attacking: false, choose_host: false, host_creature: false, host_only: None, host_chooser: None, non_aura_if_no_host: false, face_down: false, under_control_of: None, random_order: false, enters_bare: false });
                let ch = Effect::Choose { who: who.clone(), from: None, zone: Zone::Sideboard, also_zones: vec![], filter: (Filter { owner: Some(PlayerFilter::You), ..filter.clone().into_filter() }).into(), min: 0, max: *max, random: false, min_amount: None, max_amount: None, distinct: None, accumulate: false, top: None, among: None, then };
                Exec::Push(Frame { list: Arc::new(vec![ch]), pc: 0, repeat: 0, each: vec![], each_active: false, tbase: ctx.cur_tbase })
            }
            Effect::AddCompanions { names } => {
                for n in names {
                    if let Some(def) = self.db.find(n) {
                        let id = self.create_card(def, controller, Zone::Sideboard, false);
                        self.cards[id.0 as usize].is_companion = true;
                    }
                }
                Exec::Done
            }
            Effect::GiveToSideboard { what, to } => {
                let (cs, ps) = {
                    let c = ec!();
                    (self.sel_cards(what, &c), self.sel_players(to, &c))
                };
                if let Some(&dest) = ps.first() {
                    for id in cs {
                        if self.cards[id.0 as usize].zone != Zone::Sideboard {
                            continue;
                        }
                        for pl in self.players.iter_mut() {
                            pl.sideboard.retain(|&x| x != id);
                        }
                        self.players[dest as usize].sideboard.push(id);
                        self.cards[id.0 as usize].owner = dest;
                        self.cards[id.0 as usize].cur.controller = dest;
                    }
                }
                Exec::Done
            }
            Effect::MayPlayFromOutsideGame { filter, extra_mode } => {
                match filter {
                    Some(f) => self.outside_card_grants.push((controller, self.turn, Filter { owner: Some(PlayerFilter::You), ..(**f).clone() }, source, *extra_mode)),
                    None => self.outside_grants.push((controller, self.turn)),
                }
                Exec::Done
            }
            Effect::MayCastCardThisTurn { what, exile_after, tax_unless_targets_own_creature } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                for id in cs {
                    if self.cards[id.0 as usize].zone == Zone::Graveyard && !self.def_of(id).types.has(CardType::Land) {
                        let z = self.cards[id.0 as usize].zcc;
                        self.gy_card_grants.push((controller, self.turn, id, z));
                        if *exile_after || *tax_unless_targets_own_creature > 0 {
                            self.gy_card_riders.push((id, z, self.turn, *exile_after, *tax_unless_targets_own_creature));
                        }
                    }
                }
                Exec::Done
            }
            Effect::GrantActivatedAbilitiesOf { from, to, duration } => {
                let (fs, ts) = {
                    let c = ec!();
                    (self.sel_cards(from, &c), self.sel_cards(to, &c))
                };
                let mut granted: Vec<Ability> = vec![];
                for f in fs {
                    for a in &self.def_of(f).abilities {
                        if matches!(a, Ability::Activated { .. }) {
                            granted.push(a.clone());
                        }
                    }
                }
                for a in granted {
                    for &t in &ts {
                        self.add_effect(source, controller, &[t], Modif::Static(Box::new(StaticEffect::GrantAbility { affects: (Filter::default()).into(), ability: Box::new(a.clone()) })), *duration);
                    }
                }
                Exec::Done
            }
            Effect::PlayTopOfLibraryThisTurn { whose } => {
                let owners = {
                    let c = ec!();
                    self.sel_players(whose, &c)
                };
                for o in owners {
                    self.lib_play_grants.push((controller, o, self.turn));
                }
                Exec::Done
            }
            Effect::DoubleSpellX { what } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards_or_spell(what, &c)
                };
                for id in cs {
                    if let Some(it) = self.stack.iter_mut().find(|i| i.source == id) {
                        it.x = it.x.saturating_mul(2);
                        self.cards[id.0 as usize].x = it.x;
                    }
                }
                Exec::Done
            }
            Effect::Assemble { n } => {
                let mut k = {
                    let c = ec!();
                    self.eval_amount(n, &c).max(0)
                };
                let reps: Vec<(CardId, PlayerId, FilterBox, i32)> = self.active_replacements().into_iter().filter_map(|r| if let Replace::Assemble { filter, multiply } = &r.what { Some((r.source, r.controller, filter.clone(), *multiply)) } else { None }).collect();
                for (rs, rc, f, m) in reps {
                    if rc == controller && self.matches_card(&Filter { controller: PlayerFilter::Any, ..f.clone().into_filter() }, source, rs, rc) && self.cards[source.0 as usize].cur.controller == rc {
                        k = k.saturating_mul(m);
                    }
                }
                self.players[controller as usize].contraptions += k as u32;
                Exec::Done
            }
            Effect::PlayTopFreeWhileOnTop => {
                if let Some(&top) = self.players[controller as usize].library.last() {
                    let z = self.cards[top.0 as usize].zcc;
                    self.lib_free_grants.push((controller, self.turn, top, z));
                }
                Exec::Done
            }
            Effect::MayCastForaging { filter } => {
                self.turn_play_grants.push((controller, Zone::Graveyard, filter.clone().into_filter(), source, self.turn, vec![], false));
                self.forage_grants.push((controller, self.turn, filter.clone().into_filter(), source));
                Exec::Done
            }
            Effect::Earthbend { what, n } => {
                let back = |to: Zone| Effect::AddTrigger {
                    trigger: Trigger::ChangesZone { filter: (Filter::default()).into(), from: Some(Zone::Battlefield), to: Some(to), not_to: None, also_from: vec![], cause_opponent: false, cause_you: false, destroyed: false, not_sacrificed: false, after_resolving: false, craft_cost: false },
                    effects: vec![Effect::Move { what: Sel::TriggerObject, to: Zone::Battlefield, top_of_library: false, tapped: true, under_your_control: false, remember: false, transformed: false, from: None, discard: false, position: None, position_x: false, ordered: false, bottom_ordered: false, order_chooser: None, attacking: false, choose_host: false, host_creature: false, host_only: None, host_chooser: None, non_aura_if_no_host: false, face_down: false, under_control_of: None, random_order: false, enters_bare: false }],
                    once: true,
                    until: TempUntil::Forever,
                    bind: Some(what.clone()),
                    ability: None,
                    also: vec![],
                    carry: None,
                    cond: None,
                };
                let list = vec![
                    Effect::Animate { what: what.clone(), add_types: vec![CardType::Creature], remove_types: vec![], add_subtypes: vec![], set_subtypes: vec![], set_chosen_subtype: false, add_chosen_subtype: false, set_chosen_color: false, power: Some(0), toughness: Some(0), pt_amount: None, colors: None, add_colors: vec![], keywords: vec![Keyword::Haste], add_supertypes: vec![], remove_supertypes: vec![], set_name: None, set_name_chosen_subtype: false, duration: Duration::Forever },
                    Effect::AddCounters { what: what.clone(), kind: CounterKind::PlusOnePlusOne, n: n.clone() },
                    back(Zone::Graveyard),
                    back(Zone::Exile),
                    Effect::EmitBend { kind: BendKind::Earth },
                ];
                Exec::Push(Frame { list: Arc::new(list), pc: 0, repeat: 0, each: vec![], each_active: false, tbase: ctx.cur_tbase })
            }
            Effect::Echo { cost } => {
                // echo is due once per object, at the first upkeep of whoever controls it then (it may have changed hands since it entered)
                if self.cards[source.0 as usize].zone != Zone::Battlefield || self.cards[source.0 as usize].echo_done_zcc == self.cards[source.0 as usize].zcc {
                    return Exec::Done;
                }
                self.cards[source.0 as usize].echo_done_zcc = self.cards[source.0 as usize].zcc;
                let ctrl = self.cards[source.0 as usize].controller;
                let zero = self.cards.iter().enumerate().any(|(i, c)| {
                    c.zone == Zone::Battlefield && c.controller == ctrl && self.def_of(CardId(i as _)).abilities.iter().any(|a| matches!(a, Ability::Static { effect: StaticEffect::EchoPayZero, .. }))
                });
                let cost = if zero { vec![Cost::Mana { cost: "{0}".into() }] } else { cost.clone() };
                let unless = Effect::Unless { payer: Sel::You, any_player: false, if_paid: vec![], cost, effects: vec![Effect::Sacrifice { who: Sel::This, filter: (Filter::default()).into(), n: Amount::Fixed { n: 1 } }] };
                Exec::Push(Frame { list: Arc::new(vec![unless]), pc: 0, repeat: 0, each: vec![], each_active: false, tbase: ctx.cur_tbase })
            }
            Effect::CumulativeUpkeep { cost, if_paid, if_unpaid } => {
                if self.cards[source.0 as usize].zone != Zone::Battlefield {
                    return Exec::Done;
                }
                self.add_counters_to(Obj::card(source), CounterKind::Age, 1);
                self.pay_upkeep.set(true);
                let n = self.cards[source.0 as usize].counter(CounterKind::Age) as i32;
                fn scale_up(c: &Cost, n: i32) -> Cost {
                    match c {
                        Cost::Mana { cost } => Cost::Mana { cost: cost.repeat(n as usize) },
                        Cost::PayLife { n: k } => Cost::PayLife { n: k * n },
                        Cost::OpponentGainsLife { n: k } => Cost::OpponentGainsLife { n: *k },
                        Cost::FlipCoins { n: k } => Cost::FlipCoins { n: k * n },
                        Cost::AddMana { color, n: k } => Cost::AddMana { color: *color, n: k * n },
                        Cost::OpponentCreatesToken { token, n: k } => Cost::OpponentCreatesToken { token: token.clone(), n: *k },
                        Cost::Sacrifice { filter, n: k } => Cost::Sacrifice { filter: filter.clone(), n: k * n },
                        Cost::Discard { filter, n: k, random } => Cost::Discard { filter: filter.clone(), n: k * n, random: *random },
                        Cost::ExileFromGraveyard { filter, n: k, bottom } => Cost::ExileFromGraveyard { filter: filter.clone(), n: k * n, bottom: *bottom },
                        Cost::ExileFromHand { filter, n: k, to_library } => Cost::ExileFromHand { filter: filter.clone(), n: k * n, to_library: *to_library },
                        Cost::TapPermanents { filter, n: k } => Cost::TapPermanents { filter: filter.clone(), n: k * n },
                        Cost::ExileTop { n: k } => Cost::ExileTop { n: k * n },
                        Cost::PutCounters { kind, n: k } => Cost::PutCounters { kind: *kind, n: k * n },
                        Cost::RemoveCounters { kind, n: k } => Cost::RemoveCounters { kind: *kind, n: k * n },
                        Cost::DrawCards { n: k } => Cost::DrawCards { n: k * n },
                        Cost::Mill { n: k } => Cost::Mill { n: k * n },
                        Cost::ReturnToHand { filter, n: k } => Cost::ReturnToHand { filter: filter.clone(), n: k * n },
                        // "{U} or {B}" per age counter: each alternative is scaled
                        Cost::Or { options } => Cost::Or { options: options.iter().map(|o| o.iter().map(|c| scale_up(c, n)).collect()).collect() },
                        other => other.clone(),
                    }
                }
                let scaled: Vec<Cost> = cost
                    .iter()
                    .map(|c| scale_up(c, n))
                    .flat_map(|c| match c {
                        // one payment per age counter: each counter on a permanent chosen as it is put, each "an opponent ..." to an opponent chosen for it
                        Cost::PutCountersOnChosen { .. } | Cost::ExileFromSingleGraveyard { .. } | Cost::GainControlOfChosen { .. } | Cost::OpponentGainsLife { .. } | Cost::OpponentCreatesToken { .. } => vec![c; n as usize],
                        c => vec![c],
                    })
                    .collect();
                // no age counters (Solemnity): the cost is {0}, which is paid at once
                if n == 0 {
                    return if if_paid.is_empty() { Exec::Done } else { Exec::Push(Frame { list: Arc::new(if_paid.clone()), pc: 0, repeat: 0, each: vec![], each_active: false, tbase: ctx.cur_tbase }) };
                }
                let mut unpaid = vec![Effect::Sacrifice { who: Sel::This, filter: (Filter::default()).into(), n: Amount::Fixed { n: 1 } }];
                unpaid.extend(if_unpaid.iter().cloned());
                let unless = Effect::Unless { payer: Sel::You, any_player: false, if_paid: if_paid.clone(), cost: scaled, effects: unpaid };
                Exec::Push(Frame { list: Arc::new(vec![unless]), pc: 0, repeat: 0, each: vec![], each_active: false, tbase: ctx.cur_tbase })
            }
            Effect::Unless { payer, cost, effects, any_player, if_paid } => {
                let ux = self.cards[source.0 as usize].x;
                if ctx.sub.is_none() {
                    self.cost_sacrificed.clear();
                    self.last_cost_paid = false;
                }
                let cost_x: Vec<Cost> = cost
                    .iter()
                    .map(|c| match c {
                        Cost::Mana { cost } if cost.contains("{X}") || cost.contains("{N}") => {
                            let cn = self.cards[source.0 as usize].chosen_number.unwrap_or(0);
                            Cost::Mana { cost: cost.replace("{X}", &format!("{{{ux}}}")).replace("{N}", &format!("{{{cn}}}")) }
                        }
                        Cost::ManaAmount { amount } => {
                            let c = ec!();
                            Cost::Mana { cost: format!("{{{}}}", self.eval_amount(amount, &c).max(0)) }
                        }
                        Cost::ColoredManaAmount { symbol, amount } => {
                            let c = ec!();
                            Cost::Mana { cost: symbol.repeat(self.eval_amount(amount, &c).max(0) as usize) }
                        }
                        Cost::SourceManaCost => Cost::Mana { cost: self.def_of(source).cost.to_symbols() },
                        Cost::ManaCostOf { of, reduce_generic } => {
                            let c = ec!();
                            let id = self.sel_cards(of, &c).first().copied();
                            Cost::Mana { cost: id.map_or(String::new(), |i| {
                                let mut mc = self.def_of(i).cost.clone();
                                let mut left = *reduce_generic;
                                for sh in mc.shards.iter_mut() {
                                    if let Shard::Generic { n } = sh {
                                        let r = (*n).min(left);
                                        *n -= r;
                                        left -= r;
                                    }
                                }
                                mc.shards.retain(|s| !matches!(s, Shard::Generic { n: 0 }));
                                mc.to_symbols()
                            }) }
                        }
                        Cost::PayLifeX { amount } => {
                            let c = ec!();
                            Cost::PayLife { n: self.eval_amount(amount, &c).max(0) }
                        }
                        Cost::PayEnergyX { amount } => {
                            let c = ec!();
                            Cost::PayEnergy { n: self.eval_amount(amount, &c).max(0) }
                        }
                        Cost::TakeDamageX { amount } => {
                            let c = ec!();
                            Cost::TakeDamage { n: self.eval_amount(amount, &c).max(0) }
                        }
                        o => o.clone(),
                    })
                    .collect();
                let cost = &cost_x;
                let mut paid_result: Option<bool> = None;
                if let Some(Sub::PayPick { unless: true, .. }) = ctx.sub.clone() {
                    match self.continue_pay(ctx, source) {
                        Some((paid, _)) => paid_result = Some(paid),
                        None => return Exec::Wait,
                    }
                } else if let Some(Sub::UnlessPay { payer: p }) = ctx.sub.clone() {
                    let ans = self.answer.take();
                    ctx.sub = None;
                    if let Some(Choice::YesNo { yes: true }) = ans {
                        match self.pay_with_choices(ctx, p, source, cost.clone(), true) {
                            Some(paid) => paid_result = Some(paid),
                            None => return Exec::Wait,
                        }
                    } else {
                        paid_result = Some(false);
                    }
                }
                let mut next: Option<PlayerId> = None;
                if let Some(paid) = paid_result {
                    // "any opponent may have it deal damage to them" / "any player may pay 5 life. If a player does": every player in turn order is still asked after one accepts (Vexing Devil, Dash Hopes ruling)
                    let take_dmg_only = *any_player && !if_paid.is_empty() && !cost.is_empty() && cost.iter().all(|c| matches!(c, Cost::TakeDamage { .. } | Cost::Sacrifice { .. } | Cost::TapPermanents { .. } | Cost::PayLife { .. }));
                    if paid && take_dmg_only {
                        self.last_cost_paid = true;
                    }
                    if paid && !take_dmg_only {
                        ctx.unless_rest.clear();
                        if !self.cost_sacrificed.is_empty() {
                            ctx.remembered = std::mem::take(&mut self.cost_sacrificed).into_iter().map(Obj::card).collect();
                        }
                        if !if_paid.is_empty() {
                            return Exec::Push(Frame { list: Arc::new(if_paid.clone()), pc: 0, repeat: 0, each: vec![], each_active: false, tbase: ctx.cur_tbase });
                        }
                        return Exec::Done;
                    }
                    // "unless any player pays": the next player gets the chance
                    while !ctx.unless_rest.is_empty() {
                        let q = ctx.unless_rest.remove(0);
                        if self.can_pay_costs(q, source, cost) {
                            next = Some(q);
                            break;
                        }
                    }
                    if next.is_none() {
                        if take_dmg_only && self.last_cost_paid {
                            self.last_cost_paid = false;
                            if !if_paid.is_empty() {
                                return Exec::Push(Frame { list: Arc::new(if_paid.clone()), pc: 0, repeat: 0, each: vec![], each_active: false, tbase: ctx.cur_tbase });
                            }
                            return Exec::Done;
                        }
                        return Exec::Push(Frame { list: Arc::new(effects.clone()), pc: 0, repeat: 0, each: vec![], each_active: false, tbase: ctx.cur_tbase });
                    }
                }
                let p = match next {
                    Some(p) => p,
                    None => {
                        let ps = {
                            let c = ec!();
                            self.sel_players(payer, &c)
                        };
                        let mut able: Vec<PlayerId> = if *any_player { ps.clone() } else { ps.first().copied().into_iter().collect() };
                        if !*any_player {
                            let Some(&p) = ps.first() else { return Exec::Done };
                            if !self.can_pay_costs(p, source, cost) {
                                return Exec::Push(Frame { list: Arc::new(effects.clone()), pc: 0, repeat: 0, each: vec![], each_active: false, tbase: ctx.cur_tbase });
                            }
                            able = vec![p];
                        } else {
                            able.retain(|q| self.can_pay_costs(*q, source, cost));
                            if able.is_empty() {
                                return if ps.is_empty() { Exec::Done } else { Exec::Push(Frame { list: Arc::new(effects.clone()), pc: 0, repeat: 0, each: vec![], each_active: false, tbase: ctx.cur_tbase }) };
                            }
                        }
                        ctx.unless_rest = able.split_off(1);
                        able[0]
                    }
                };
                ctx.sub = Some(Sub::UnlessPay { payer: p });
                self.pending = Some(Decision {
                    player: p,
                    kind: DecisionKind::YesNo { prompt: format!("Pay the cost for {}?", self.def_of(source).name), source: Some(source) },
                });
                Exec::Wait
            }
            Effect::Repeat { n, effects } => {
                let k = {
                    let c = ec!();
                    self.eval_amount(n, &c)
                };
                if k <= 0 {
                    return Exec::Done;
                }
                Exec::Push(Frame { list: Arc::new(effects.clone()), pc: 0, repeat: k - 1, each: vec![], each_active: false, tbase: ctx.cur_tbase })
            }
            Effect::RepeatWhile { cond, effects, max } => {
                let left = max.unwrap_or(20);
                let mut list = effects.clone();
                if left > 1 {
                    list.push(Effect::If { cond: cond.clone(), then: vec![Effect::RepeatWhile { cond: cond.clone(), effects: effects.clone(), max: Some(left - 1) }], otherwise: vec![] });
                }
                Exec::Push(Frame { list: Arc::new(list), pc: 0, repeat: 0, each: vec![], each_active: false, tbase: ctx.cur_tbase })
            }
            Effect::ForEach { objects, effects, turn_order } => {
                let mut objs = {
                    let c = ec!();
                    self.sel_objs(objects, &c)
                };
                if *turn_order {
                    let n = self.players.len() as i32;
                    let act = self.active as i32;
                    let key = |g: &Game, o: &Obj| -> i32 {
                        let seat = match o {
                            Obj::Player { id } => *id as i32,
                            Obj::Card { id } => g.cards[id.0 as usize].cur.controller as i32,
                            _ => 0,
                        };
                        (seat - act).rem_euclid(n.max(1))
                    };
                    objs.sort_by_key(|o| key(self, o));
                }
                Exec::Push(Frame { list: Arc::new(effects.clone()), pc: 0, repeat: 0, each: objs, each_active: true, tbase: ctx.cur_tbase })
            }
            Effect::AttachLegal { what, to } => {
                let (xs, ys) = {
                    let c = ec!();
                    (self.sel_cards(what, &c), self.sel_objs(to, &c))
                };
                let Some(&y) = ys.first() else { return Exec::Done };
                for x in xs {
                    if self.cards[x.0 as usize].zone != Zone::Battlefield {
                        continue;
                    }
                    let is_aura = self.cards[x.0 as usize].cur.subtypes.contains(&Sym::new("Aura"));
                    let is_equip = self.cards[x.0 as usize].cur.subtypes.contains(&Sym::new("Equipment"));
                    let xctl = self.cards[x.0 as usize].cur.controller;
                    match y {
                        Obj::Card { id: h } => {
                            if h == x || self.cards[h.0 as usize].zone != Zone::Battlefield {
                                continue;
                            }
                            if is_equip {
                                if self.cards[h.0 as usize].cur.types.has(CardType::Creature) {
                                    self.attach(x, Some(h));
                                }
                            } else if is_aura {
                                let ok = self.def_of(x).abilities.iter().find_map(|a| if let Ability::Attach { to } = a { Some(to.clone()) } else { None }).map_or(false, |f| self.matches_card(&f, h, x, xctl)) && !self.host_guarded_from_auras(h);
                                if ok {
                                    self.cards[x.0 as usize].enchanted_player = None;
                                    self.attach(x, Some(h));
                                }
                            }
                        }
                        Obj::Player { id: p } => {
                            let d = self.def_of(x);
                            let ok = is_aura && d.abilities.iter().any(|a| matches!(a, Ability::EnchantPlayer) || (matches!(a, Ability::EnchantOpponent) && p != xctl));
                            if ok {
                                self.attach(x, None);
                                self.cards[x.0 as usize].enchanted_player = Some(p);
                                self.layers_dirty = true;
                            }
                        }
                        _ => {}
                    }
                }
                Exec::Done
            }
            Effect::AttachToPlayer { what, to } => {
                let (cs, pl) = {
                    let c = ec!();
                    (self.sel_cards(what, &c), self.sel_players(to, &c).first().copied())
                };
                if let Some(pl) = pl {
                    for id in cs {
                        if self.cards[id.0 as usize].zone == Zone::Battlefield {
                            self.cards[id.0 as usize].enchanted_player = Some(pl);
                        }
                    }
                    self.layers_dirty = true;
                }
                Exec::Done
            }
            Effect::ForEachPlayer { players, effects } => {
                let objs: Vec<Obj> = {
                    let c = ec!();
                    self.sel_players(players, &c).into_iter().map(Obj::player).collect()
                };
                Exec::Push(Frame { list: Arc::new(effects.clone()), pc: 0, repeat: 0, each: objs, each_active: true, tbase: ctx.cur_tbase })
            }
            Effect::EachPlayerChooses { players, zone, filter, min, max, options, remember, then } => {
                let src = ctx.item.source;
                let mut cards: Vec<(PlayerId, Vec<CardId>)>;
                let mut named: Vec<(PlayerId, u8)>;
                let mut queue: Vec<PlayerId>;
                if let Some(Sub::HiddenChoose { options: offered, queue: q, chooser, cards: cs, named: nm }) = ctx.sub.clone() {
                    ctx.sub = None;
                    cards = cs;
                    named = nm;
                    queue = q;
                    if let Some(Choice::Pick { indices }) = self.answer.take() {
                        if !options.is_empty() {
                            named.push((chooser, indices.first().copied().unwrap_or(0).min(options.len() - 1) as u8));
                        } else {
                            let mut picked: Vec<CardId> = vec![];
                            for id in indices.iter().filter_map(|i| offered.get(*i).copied()) {
                                if !picked.contains(&id) {
                                    picked.push(id);
                                }
                            }
                            cards.push((chooser, picked));
                        }
                    }
                } else {
                    cards = vec![];
                    named = vec![];
                    let ps = {
                        let c = ec!();
                        self.sel_players(players, &c)
                    };
                    // asked one by one: the effect's controller first, then in turn order
                    queue = vec![];
                    if !ps.is_empty() {
                        let mut p = if ps.contains(&controller) { controller } else { ps[0] };
                        for _ in 0..self.players.len() {
                            if ps.contains(&p) && !queue.contains(&p) {
                                queue.push(p);
                            }
                            p = self.next_player(p);
                        }
                    }
                }
                while !queue.is_empty() {
                    let chooser = queue.remove(0);
                    if !options.is_empty() {
                        if options.len() == 1 {
                            named.push((chooser, 0));
                            continue;
                        }
                        ctx.sub = Some(Sub::HiddenChoose { options: vec![], queue, chooser, cards, named });
                        self.pending = Some(Decision {
                            player: chooser,
                            kind: DecisionKind::Pick {
                                prompt: format!("{}: choose", self.def_of(src).name),
                                source: Some(src),
                                options: options.iter().map(|t| Opt::Text { text: t.clone() }).collect(),
                                min: 1,
                                max: 1,
                                cancellable: false,
                                hidden: true,
                                purpose: PickPurpose::Generic,
                            },
                        });
                        return Exec::Wait;
                    }
                    let cand: Vec<CardId> = {
                        let pool: Vec<CardId> = match zone {
                            Zone::Hand => self.players[chooser as usize].hand.clone(),
                            Zone::Graveyard => self.players[chooser as usize].graveyard.clone(),
                            Zone::Exile => self.players[chooser as usize].exile.clone(),
                            _ => self.battlefield.iter().copied().collect(),
                        };
                        pool.into_iter().filter(|id| self.matches_card(filter, *id, src, chooser)).collect()
                    };
                    let lo = *min as usize;
                    let hi = (*max as usize).max(lo);
                    if cand.is_empty() || hi == 0 {
                        continue;
                    }
                    if cand.len() <= lo {
                        cards.push((chooser, cand));
                        continue;
                    }
                    ctx.sub = Some(Sub::HiddenChoose { options: cand.clone(), queue, chooser, cards, named });
                    self.pending = Some(Decision {
                        player: chooser,
                        kind: DecisionKind::Pick {
                            prompt: format!("{}: choose", self.def_of(src).name),
                            source: Some(src),
                            options: cand.iter().map(|c| Opt::Obj { obj: Obj::card(*c) }).collect(),
                            min: lo as u8,
                            max: hi.min(cand.len()) as u8,
                            cancellable: false,
                            hidden: matches!(zone, Zone::Hand),
                            purpose: PickPurpose::Generic,
                        },
                    });
                    return Exec::Wait;
                }
                // everybody has chosen: only now are the choices revealed, kept and applied
                for (p, picked) in &cards {
                    self.emit(Event::ChoiceRevealed { player: *p, source: src, cards: picked.clone(), option: None });
                }
                for (p, o) in &named {
                    self.emit(Event::ChoiceRevealed { player: *p, source: src, cards: vec![], option: options.get(*o as usize).cloned() });
                }
                if *remember {
                    let name = self.db.def(self.cards[source.0 as usize].def).name.clone();
                    for (p, o) in &named {
                        self.player_choices.retain(|(n, q, _)| !(n == &name && q == p));
                        self.player_choices.push((name.clone(), *p, *o));
                    }
                }
                let each: Vec<Obj> = if options.is_empty() {
                    cards.iter().filter(|(_, v)| !v.is_empty()).map(|(p, _)| Obj::player(*p)).collect()
                } else {
                    named.iter().map(|(p, _)| Obj::player(*p)).collect()
                };
                self.hidden_picks = cards;
                self.layers_dirty = true;
                if then.is_empty() || each.is_empty() {
                    return Exec::Done;
                }
                Exec::Push(Frame { list: Arc::new(then.clone()), pc: 0, repeat: 0, each, each_active: true, tbase: ctx.cur_tbase })
            }
            Effect::FlipPlayerChoices => {
                let name = self.db.def(self.cards[source.0 as usize].def).name.clone();
                for (n, _, o) in self.player_choices.iter_mut() {
                    if *n == name {
                        *o = if *o == 0 { 1 } else { 0 };
                    }
                }
                self.layers_dirty = true;
                Exec::Done
            }
            Effect::GainRebound { what } => {
                let cs = {
                    let c = ec!();
                    self.sel_cards(what, &c)
                };
                for id in cs {
                    if self.cards[id.0 as usize].zone == Zone::Stack {
                        self.cards[id.0 as usize].gained_rebound = true;
                    }
                }
                Exec::Done
            }
            Effect::AddTrigger { trigger, effects, once, until, bind, ability, also, carry, cond } => {
                // the stored slots it reads are pinned as they are now (they only last for this resolution)
                let pinned: Option<(Trigger, Vec<Trigger>, Option<Box<Condition>>, Vec<Effect>)> = self.pin_stored_as(&(trigger, also, cond, effects));
                let (trigger, also, cond, effects) = match &pinned {
                    Some((t, a, c, e)) => (t, a, c, e),
                    None => (trigger, also, cond, effects),
                };
                let bound: Vec<CardId> = match bind {
                    Some(sel) => {
                        let c = ec!();
                        self.sel_cards(sel, &c)
                    }
                    None => vec![],
                };
                if bind.is_some() && bound.is_empty() {
                    return Exec::Done;
                }
                let carried: Vec<Obj> = match carry {
                    Some(sel) => {
                        let c = ec!();
                        self.sel_objs(sel, &c).into_iter().collect()
                    }
                    None => ctx.remembered.clone(),
                };
                let did = self.next_effect_id;
                self.next_effect_id += 1;
                // a delayed trigger made by an {X} spell keeps that X ("copy that spell X times": Storm King's Thunder)
                let baked: Vec<Effect>;
                let effects = if ctx.item.x > 0 { baked = bake_x_effects(effects, ctx.item.x); &baked } else { effects };
                for trig in std::iter::once(trigger).chain(also.iter()) {
                    self.temp_triggers.push(TempTrigger {
                    trigger: trig.clone(),
                    once: *once,
                    until: *until,
                    bound: bound.clone(),
                    ability: *ability,
                    cond: cond.as_deref().cloned(),
                    d: DelayedTrigger {
                        id: did,
                        controller,
                        source,
                        when: DelayedWhen::EndOfTurn,
                        effects: effects.clone(),
                        created_turn: self.turn,
                        targets: ctx.targets.clone(),
                        ctx: ctx.item.ctx.clone(),
                        remembered: carried.clone(),
                    },
                });
                }
                Exec::Done
            }
            Effect::Delayed { at, effects } => {
                let pinned = self.pin_stored(effects);
                let effects = pinned.as_ref().unwrap_or(effects);
                let did = self.next_effect_id;
                self.next_effect_id += 1;
                let mut dctx = ctx.item.ctx.clone();
                // the source as a `this` object: a source that changes zones before the delayed effect is a new object (rule 400.7)
                if self.cards[source.0 as usize].zone == Zone::Battlefield {
                    dctx.this_zcc = Some(self.cards[source.0 as usize].zcc);
                }
                if *at == DelayedWhen::NextChosenPlayerEndStep {
                    dctx.player = self.cards[source.0 as usize].chosen_player;
                }
                if *at == DelayedWhen::NextTargetPlayerUpkeep {
                    dctx.player = ctx.targets.iter().find_map(|t| if let Obj::Player { id } = t.obj { Some(id) } else { None });
                }
                self.delayed.push(DelayedTrigger {
                    id: did,
                    controller,
                    source,
                    when: *at,
                    effects: effects.clone(),
                    created_turn: self.turn,
                    targets: ctx.targets.clone(),
                    ctx: dctx,
                    remembered: ctx.remembered.clone(),
                });
                // a spell that could not be countered goes on to resolve onto the battlefield: not tracked (Arcane Denial vs Shifting Ceratops)
                // the trigger object counts as the same object only while it stays where it was (a creature flickered before the delayed destroy is a new object)
                let obj_card = if matches!(at, DelayedWhen::EndOfCombat) { ctx.item.ctx.obj.and_then(|o| if let Obj::Card { id } = o { Some(id) } else { None }) } else { None };
                let zs: Vec<(u32, u32)> = ctx.remembered.iter().chain(obj_card.map(Obj::card).iter()).filter_map(|o| if let Obj::Card { id } = o { if self.cards[id.0 as usize].zone == Zone::Stack { None } else { Some((id.0, self.cards[id.0 as usize].zcc)) } } else { None }).collect();
                if !zs.is_empty() {
                    self.delayed_rem_zcc.push((did, zs));
                }
                Exec::Done
            }
            Effect::Choose { who, from, zone, also_zones, filter, min, max, random, min_amount, max_amount, distinct, accumulate, top, among, then } => {
                self.choose_accumulate.set(*accumulate);
                let mut acc: Vec<CardId> = vec![];
                let mut asked = false;
                let mut queue: Vec<PlayerId>;
                if let Some(Sub::ChooseEach { options, queue: q, acc: a }) = ctx.sub.clone() {
                    let ans = self.answer.take();
                    ctx.sub = None;
                    acc = a;
                    if let Some(Choice::Pick { indices }) = ans {
                        let mut picked: Vec<CardId> = vec![];
                        for c in indices.iter().filter_map(|i| options.get(*i).copied()) {
                            let key = |g: &Game, id: CardId| -> String {
                                match distinct {
                                    Some(ChooseDistinct::Name) => g.def_of(id).name.clone(),
                                    Some(ChooseDistinct::Power) => g.cards[id.0 as usize].cur.power.to_string(),
                                    Some(ChooseDistinct::ManaValue) => g.def_of(id).mana_value.to_string(),
                                    Some(ChooseDistinct::Controller) => g.cards[id.0 as usize].cur.controller.to_string(),
                                    None | Some(ChooseDistinct::SharedCreatureType) | Some(ChooseDistinct::SameToughness) => String::new(),
                                }
                            };
                            if distinct.is_some() && picked.iter().any(|p| key(self, *p) == key(self, c)) {
                                continue;
                            }
                            picked.push(c);
                        }
                        acc.extend(picked);
                    }
                    queue = q;
                    asked = true;
                } else {
                    let c = ec!();
                    queue = self.sel_players(who, &c);
                    if queue.is_empty() {
                        return Exec::Done;
                    }
                }
                loop {
                    if queue.is_empty() {
                        if acc.is_empty() && !asked {
                            return Exec::Done;
                        }
                        return self.choose_then(ctx, acc, then.clone());
                    }
                    let chooser = queue.remove(0);
                    // whose zone: the chooser's own unless `from` names other players (all of them are offered)
                    let owners: Vec<PlayerId> = match from {
                        Some(f) => {
                            let c = ec!();
                            let v = self.sel_players(f, &c);
                            if v.is_empty() { vec![chooser] } else { v }
                        }
                        // cards "exiled with this" can sit in any player's exile zone
                        None if *zone == Zone::Exile && filter.exiled_with_this => (0..self.players.len() as PlayerId).collect(),
                        None => vec![chooser],
                    };
                    // candidates: the cards in the zone that match the filter (controller relative to the chooser)
                    let mut options: Vec<CardId> = vec![];
                    let src = ctx.item.source;
                    let mut pools: Vec<CardId> = vec![];
                    if let Some(a) = among {
                        let c = ec!();
                        pools.extend(self.sel_cards(a, &c));
                        if matches!(a, Sel::LegalTargetsOf { .. }) {
                            pools.extend(self.sel_objs(a, &c).into_iter().filter_map(|o| if let Obj::Card { id } = o { Some(id) } else { None }));
                        }
                    }
                    for z in std::iter::once(zone).chain(also_zones.iter()).filter(|_| among.is_none()) {
                        match z {
                            Zone::Battlefield => pools.extend(self.battlefield.iter().copied()),
                            Zone::Hand => pools.extend(owners.iter().flat_map(|o| self.players[*o as usize].hand.clone())),
                            Zone::Graveyard => pools.extend(owners.iter().flat_map(|o| {
                                let gy = &self.players[*o as usize].graveyard;
                                match top {
                                    Some(n) => gy.iter().rev().take(*n as usize).copied().collect::<Vec<_>>(),
                                    None => gy.clone(),
                                }
                            })),
                            Zone::Exile => pools.extend(owners.iter().flat_map(|o| self.players[*o as usize].exile.clone())),
                            Zone::Library => pools.extend(owners.iter().flat_map(|o| self.players[*o as usize].library.clone())),
                            Zone::Command => pools.extend(owners.iter().flat_map(|o| self.players[*o as usize].command.clone())),
                            Zone::Sideboard => pools.extend(owners.iter().flat_map(|o| self.players[*o as usize].sideboard.clone())),
                            Zone::Stack => pools.extend(self.stack.iter().filter(|i| matches!(i.kind, ItemKind::Spell { .. }) && !i.copy).map(|i| i.source)),
                        }
                    }
                    for id in pools {
                        if self.matches_card(filter, id, src, chooser) {
                            options.push(id);
                        }
                    }
                    let min_v = match min_amount {
                        Some(a) => {
                            let c = ec!();
                            self.eval_amount(a, &c).clamp(0, 255) as usize
                        }
                        None => *min as usize,
                    };
                    let max_v = match max_amount {
                        Some(a) => {
                            let c = ec!();
                            self.eval_amount(a, &c).clamp(0, 255) as usize
                        }
                        None => *max as usize,
                    };
                    let (min, max) = (min_v, max_v.max(min_v));
                    if options.is_empty() || max == 0 {
                        continue;
                    }
                    if options.len() <= min {
                        acc.extend(options);
                        asked = true;
                        continue;
                    }
                    if *random {
                        let mut pool = options.clone();
                        for _ in 0..max.min(pool.len()) {
                            let i = rand::Rng::gen_range(&mut self.rng, 0..pool.len());
                            acc.push(pool.remove(i));
                        }
                        asked = true;
                        continue;
                    }
                    ctx.sub = Some(Sub::ChooseEach { options: options.clone(), queue, acc });
                    self.pending = Some(Decision {
                        player: chooser,
                        kind: DecisionKind::Pick {
                            prompt: format!("{}: choose", self.def_of(src).name),
                            source: Some(src),
                            options: options.iter().map(|c| Opt::Obj { obj: Obj::card(*c) }).collect(),
                            min: min as u8,
                            max: max.min(options.len()) as u8,
                            cancellable: false,
                            hidden: matches!(zone, Zone::Hand | Zone::Library) || (*zone == Zone::Sideboard && among.is_none() && owners == vec![chooser]),
                            purpose: PickPurpose::Generic,
                        },
                    });
                    return Exec::Wait;
                }
            }
            Effect::ChooseWithin { who, from, zone, also_zones, filter, kind, budget, then } => {
                let value = |g: &Game, id: CardId| -> i32 {
                    let c = &g.cards[id.0 as usize];
                    match kind {
                        BudgetKind::ManaValue => g.def_of(id).mana_value as i32,
                        BudgetKind::Power => if c.zone == Zone::Battlefield { c.cur.power } else { g.def_of(id).power.unwrap_or(0) },
                        BudgetKind::Toughness => if c.zone == Zone::Battlefield { c.cur.toughness } else { g.def_of(id).toughness.unwrap_or(0) },
                    }
                };
                let (mut acc, mut remaining, chooser): (Vec<CardId>, i32, PlayerId);
                if let Some(Sub::ChooseBudget { options, acc: a, remaining: r, chooser: ch }) = ctx.sub.clone() {
                    ctx.sub = None;
                    acc = a;
                    remaining = r;
                    chooser = ch;
                    match self.answer.take() {
                        Some(Choice::Pick { indices }) if !indices.is_empty() => {
                            if let Some(&id) = indices.first().and_then(|i| options.get(*i)) {
                                remaining -= value(self, id);
                                acc.push(id);
                            }
                        }
                        _ => return self.choose_then(ctx, acc, then.clone()),
                    }
                } else {
                    let c = ec!();
                    let Some(&ch) = self.sel_players(who, &c).first() else { return Exec::Done };
                    chooser = ch;
                    remaining = self.eval_amount(budget, &c);
                    acc = vec![];
                }
                let owners: Vec<PlayerId> = match from {
                    Some(f) => {
                        let c = ec!();
                        let v = self.sel_players(f, &c);
                        if v.is_empty() { vec![chooser] } else { v }
                    }
                    None => vec![chooser],
                };
                let src = ctx.item.source;
                let mut pools: Vec<CardId> = vec![];
                for z in std::iter::once(zone).chain(also_zones.iter()) {
                    match z {
                        Zone::Battlefield => pools.extend(self.battlefield.iter().copied()),
                        Zone::Hand => pools.extend(owners.iter().flat_map(|o| self.players[*o as usize].hand.clone())),
                        Zone::Graveyard => pools.extend(owners.iter().flat_map(|o| self.players[*o as usize].graveyard.clone())),
                        Zone::Exile => pools.extend(owners.iter().flat_map(|o| self.players[*o as usize].exile.clone())),
                        Zone::Library => pools.extend(owners.iter().flat_map(|o| self.players[*o as usize].library.clone())),
                        Zone::Command => pools.extend(owners.iter().flat_map(|o| self.players[*o as usize].command.clone())),
                        Zone::Sideboard => pools.extend(owners.iter().flat_map(|o| self.players[*o as usize].sideboard.clone())),
                        Zone::Stack => {}
                    }
                }
                let options: Vec<CardId> = pools.into_iter().filter(|id| !acc.contains(id) && value(self, *id) <= remaining && self.matches_card(filter, *id, src, chooser)).collect();
                if options.is_empty() {
                    return self.choose_then(ctx, acc, then.clone());
                }
                ctx.sub = Some(Sub::ChooseBudget { options: options.clone(), acc, remaining, chooser });
                self.pending = Some(Decision {
                    player: chooser,
                    kind: DecisionKind::Pick {
                        prompt: format!("{}: choose a card (total limit left: {remaining})", self.def_of(src).name),
                        source: Some(src),
                        options: options.iter().map(|c| Opt::Obj { obj: Obj::card(*c) }).collect(),
                        min: 0,
                        max: 1,
                        cancellable: false,
                        hidden: matches!(zone, Zone::Hand | Zone::Library),
                        purpose: PickPurpose::Generic,
                    },
                });
                Exec::Wait
            }
            Effect::RearrangeTop { who, n, decider } => {
                if let Some(Sub::Rearrange { player, cards }) = ctx.sub.clone() {
                    ctx.sub = None;
                    let order: Vec<CardId> = match self.answer.take() {
                        Some(Choice::Pick { indices }) => indices.iter().filter_map(|i| cards.get(*i).copied()).collect(),
                        _ => cards.clone(),
                    };
                    let mut order = order;
                    for c in &cards {
                        if !order.contains(c) {
                            order.push(*c);
                        }
                    }
                    let lib = &mut self.players[player as usize].library;
                    lib.retain(|x| !cards.contains(x));
                    // the first chosen card ends up on top (the end of the vector)
                    for c in order.into_iter().rev() {
                        lib.push(c);
                    }
                    return Exec::Done;
                }
                let (ps, k) = {
                    let c = ec!();
                    (self.sel_players(who, &c), self.eval_amount(n, &c))
                };
                let Some(&p) = ps.first() else { return Exec::Done };
                let who_orders = match decider {
                    Some(d) => {
                        let c = ec!();
                        self.sel_players(d, &c).first().copied().unwrap_or(controller)
                    }
                    None => controller,
                };
                let cards: Vec<CardId> = self.players[p as usize].library.iter().rev().take(k.max(0) as usize).copied().collect();
                if cards.len() < 2 {
                    return Exec::Done;
                }
                ctx.sub = Some(Sub::Rearrange { player: p, cards: cards.clone() });
                self.pending = Some(Decision {
                    player: who_orders,
                    kind: DecisionKind::Pick {
                        prompt: "Put these cards back on top of the library: choose the order, top card first".into(),
                        source: Some(source),
                        options: cards.iter().map(|c| Opt::Obj { obj: Obj::card(*c) }).collect(),
                        min: cards.len() as u8,
                        max: cards.len() as u8,
                        cancellable: false,
                        hidden: true,
                        purpose: PickPurpose::Generic,
                    },
                });
                Exec::Wait
            }
            Effect::RevealUntil { who, filter, count, to, tapped, rest, rest_random, rest_ordered, found_to_bottom, cascade_land, under_your_control } => {
                if let Some(Sub::CascadeLand { lands }) = ctx.sub.clone() {
                    ctx.sub = None;
                    if let Some(Choice::Pick { indices }) = self.answer.take() {
                        if let Some(&id) = indices.first().and_then(|i| lands.get(*i)) {
                            if self.cards[id.0 as usize].zone == Zone::Library {
                                self.move_card(id, Zone::Battlefield, MoveOpts { tapped: true, controller: Some(controller), ..Default::default() });
                            }
                        }
                    }
                    return Exec::Done;
                }
                if let Some(Sub::BottomOrder { cards }) = ctx.sub.clone() {
                    ctx.sub = None;
                    let mut order: Vec<CardId> = match self.answer.take() {
                        Some(Choice::Pick { indices }) => indices.iter().filter_map(|i| cards.get(*i).copied()).collect(),
                        _ => vec![],
                    };
                    for c in &cards {
                        if !order.contains(c) {
                            order.push(*c);
                        }
                    }
                    // the topmost of the group goes in first (each insert goes to the very bottom)
                    for id in &order {
                        let owner = self.cards[id.0 as usize].owner;
                        self.players[owner as usize].library.retain(|c| c != id);
                    }
                    for id in order.into_iter() {
                        let owner = self.cards[id.0 as usize].owner;
                        self.players[owner as usize].library.insert(0, id);
                    }
                    return Exec::Done;
                }
                let (ps, k) = {
                    let c = ec!();
                    (self.sel_players(who, &c), self.eval_amount(count, &c).max(0))
                };
                let mut found_all: Vec<Obj> = vec![];
                let single_player = ps.len() == 1;
                self.last_reveal_count.set(0);
                for p in ps {
                    let mut found: Vec<CardId> = vec![];
                    let mut others: Vec<CardId> = vec![];
                    // several cards put onto the battlefield by one reveal enter simultaneously and see each other enter (Empty the Laboratory)
                    let defer_reveal = *to == Zone::Battlefield && k > 1 && self.defer_events.is_none();
                    if defer_reveal {
                        self.defer_events = Some(vec![]);
                    }
                    while (found.len() as i32) < k {
                        // the card leaves the library list but is still "in the library" until it is moved on below
                        let Some(top) = self.players[p as usize].library.pop() else { break };
                        if self.matches_card(filter, top, source, controller) {
                            if *rest == Zone::Exile {
                                self.zone_batch.clear();
                            }
                            if !*found_to_bottom && !self.move_card(top, *to, MoveOpts { tapped: *tapped, controller: Some(if *under_your_control { controller } else { p }), ..Default::default() }) && self.cards[top.0 as usize].zone == Zone::Library {
                                // the move was barred (Grafdigger's Cage): it was revealed but stays with the rest
                                others.push(top);
                                continue;
                            }
                            found.push(top);
                        } else {
                            others.push(top);
                        }
                    }
                    if defer_reveal {
                        self.flush_deferred_events();
                    }
                    self.last_reveal_count.set(self.last_reveal_count.get() + (found.len() + others.len()) as i32);
                    // the rest: the bottom of the library (random order if asked), or another zone
                    if *found_to_bottom {
                        others.extend(found.iter().copied());
                    }
                    if *rest_random {
                        others.shuffle(&mut self.rng);
                    }
                    if *rest_ordered && *rest == Zone::Library && !*rest_random && others.len() >= 2 && single_player {
                        // the controller picks the order, topmost of the group first
                        ctx.remembered = found.iter().map(|c| Obj::card(*c)).collect();
                        ctx.sub = Some(Sub::BottomOrder { cards: others.clone() });
                        self.pending = Some(Decision {
                            player: controller,
                            kind: DecisionKind::Pick {
                                prompt: "Put the revealed cards on the bottom of the library: choose the order, topmost first".into(),
                                source: Some(source),
                                options: others.iter().map(|c| Opt::Obj { obj: Obj::card(*c) }).collect(),
                                min: others.len() as u8,
                                max: others.len() as u8,
                                cancellable: false,
                                hidden: false,
                                purpose: PickPurpose::Generic,
                            },
                        });
                        // meanwhile they sit at the bottom in their revealed order; the answer reorders them
                        for &id in others.iter() {
                            self.players[p as usize].library.insert(0, id);
                        }
                        return Exec::Wait;
                    }
                    let cascade_lands: Vec<CardId> = if *cascade_land && single_player && *rest == Zone::Library {
                        let has = self.battlefield.iter().any(|&sid| {
                            let sc = &self.cards[sid.0 as usize];
                            sc.cur.controller == p && !sc.cur.no_abilities && self.db.def(sc.def).statics.iter().any(|&i| matches!(&self.db.def(sc.def).abilities[i as usize], Ability::Static { effect: StaticEffect::CascadeLandToBattlefield, .. }))
                        });
                        if has { others.iter().copied().filter(|c| self.cards[c.0 as usize].cur.types.has(CardType::Land)).collect() } else { vec![] }
                    } else {
                        vec![]
                    };
                    for id in others {
                        if *rest == Zone::Library && *cascade_land {
                            // cascade exiles the skipped cards one at a time, then puts them on the bottom of the library
                            self.cards[id.0 as usize].zone = Zone::Library;
                            self.zone_batch.clear();
                            self.move_card(id, Zone::Exile, Default::default());
                            if self.cards[id.0 as usize].zone == Zone::Exile {
                                self.move_card(id, Zone::Library, MoveOpts { bottom: true, ..Default::default() });
                            } else {
                                self.players[p as usize].library.insert(0, id);
                            }
                        } else if *rest == Zone::Library {
                            self.players[p as usize].library.insert(0, id);
                        } else {
                            // "exile cards until ...": each card is exiled one at a time, so batched exile triggers see separate events (Laelia)
                            if *rest == Zone::Exile {
                                self.zone_batch.clear();
                            }
                            self.move_card(id, *rest, Default::default());
                        }
                    }
                    if !cascade_lands.is_empty() {
                        ctx.remembered = found.iter().map(|c| Obj::card(*c)).collect();
                        ctx.sub = Some(Sub::CascadeLand { lands: cascade_lands.clone() });
                        self.pending = Some(Decision {
                            player: controller,
                            kind: DecisionKind::Pick {
                                prompt: "You may put a land card from among the exiled cards onto the battlefield tapped".into(),
                                source: Some(source),
                                options: cascade_lands.iter().map(|c| Opt::Obj { obj: Obj::card(*c) }).collect(),
                                min: 0,
                                max: 1,
                                cancellable: true,
                                hidden: false,
                                purpose: PickPurpose::Generic,
                            },
                        });
                        return Exec::Wait;
                    }
                    found_all.extend(found.into_iter().map(Obj::card));
                }
                ctx.remembered = found_all;
                Exec::Done
            }
            Effect::PreventDamage { to, from, combat_only, amount, redirect_to, once, counters, end_step_counters, or_you, redirect_by_source } => {
                let redirect = redirect_to.as_ref().and_then(|s| {
                    let c = ec!();
                    self.sel_objs(s, &c).into_iter().next()
                });
                let (tos, froms, n) = {
                    let c = ec!();
                    (
                        to.as_ref().map(|s| self.sel_objs(s, &c)),
                        from.as_ref().map(|s| {
                            let mut v = self.sel_cards(s, &c);
                            // a spell on the stack: the source of its damage is the spell's card
                            for o in self.sel_objs(s, &c) {
                                if let Obj::Stack { id } = o {
                                    if let Some(it) = self.stack.iter().find(|i| i.id == id) {
                                        v.push(it.source);
                                    }
                                }
                            }
                            v
                        }),
                        amount.as_ref().map(|a| self.eval_amount(a, &c)),
                    )
                };
                let from_ctl: Option<Vec<PlayerId>> = match from {
                    Some(Sel::All { filter, .. }) if filter.controlled_by.is_some() && froms.is_some() => {
                        let c = ec!();
                        filter.controlled_by.as_ref().map(|s| self.sel_players(s, &c))
                    }
                    _ => None,
                };
                self.shields.push(Shield { to: tos, from: froms, from_ctl, combat_only: *combat_only, remaining: n, redirect, once: *once, counters: *counters, end_counters: end_step_counters.map(|k| (k, source, controller)), or_player: if *or_you { Some(controller) } else { None }, redirect_by: if *redirect_by_source { Some(source) } else { None } });
                Exec::Done
            }
            Effect::ExtraLandDrop { who, n } => {
                let ps = {
                    let c = ec!();
                    self.sel_players(who, &c)
                };
                for p in ps {
                    let pl = &mut self.players[p as usize];
                    pl.land_limit = pl.land_limit.saturating_add((*n).max(0) as u8);
                }
                Exec::Done
            }
            Effect::ReduceMaxHand { who, n } => {
                let ps = {
                    let c = ec!();
                    self.sel_players(who, &c)
                };
                for p in ps {
                    self.players[p as usize].max_hand -= *n;
                }
                Exec::Done
            }
            Effect::DivideDamage { at_cast: true, slot, .. } => {
                let divs = ctx.item.divisions.clone();
                // with `slot`, the division belongs to that target spec's targets only (Fiery Justice, Magma Opus)
                let (skip, take) = match slot {
                    Some(sl) => {
                        let counts = &ctx.item.spec_counts;
                        (counts.iter().take(*sl as usize).map(|c| *c as usize).sum::<usize>(), counts.get(*sl as usize).copied().unwrap_or(0) as usize)
                    }
                    None => (0, ctx.item.targets.len()),
                };
                let chosen: Vec<Obj> = ctx.item.targets.iter().skip(skip).take(take).map(|t| t.obj).collect();
                let live: Vec<Obj> = ctx.targets.iter().map(|t| t.obj).collect();
                for (o, d) in chosen.iter().zip(divs.iter()) {
                    if *d > 0 && live.contains(o) {
                        self.deal_damage(Some(source), *o, *d, false);
                    }
                }
                Exec::Done
            }
            Effect::DivideDamage { amount, slot, among, from, divider, any_number, .. } => {
                // the damage may come from another object and another player may do the dividing (Master of the Wild Hunt)
                let (source, controller) = {
                    let c = ec!();
                    (
                        from.as_ref().and_then(|s| self.sel_cards(s, &c).first().copied()).unwrap_or(source),
                        divider.as_ref().and_then(|s| self.sel_players(s, &c).first().copied()).unwrap_or(controller),
                    )
                };
                self.divide_any.set(*any_number);
                if let Some(Sub::Divide { targets, idx, left, parts, counters }) = ctx.sub.clone() {
                    ctx.sub = None;
                    let mut parts = parts;
                    let remaining_targets = targets.len() - idx;
                    let max = if *any_number { left } else { left - (remaining_targets as i32 - 1) };
                    let n = match self.answer.take() {
                        Some(Choice::Number { n }) => n.clamp(if *any_number { 0 } else { 1 }, max.max(if *any_number { 0 } else { 1 })),
                        _ => if *any_number { 0 } else { 1 },
                    };
                    parts.push(n);
                    return self.divide_next(ctx, targets, idx + 1, left - n, parts, counters, source, controller);
                }
                let (objs, total) = {
                    let c = ec!();
                    let objs: Vec<Obj> = if let Some(a) = among {
                        self.sel_objs(a, &c)
                    } else { match slot {
                        Some(s) => {
                            let base = ctx.cur_tbase.min(ctx.tidx.len());
                            let tidx = &ctx.tidx[base..];
                            let start = tidx.get(*s as usize).copied().unwrap_or(255);
                            if start == 255 {
                                vec![]
                            } else {
                                let end = tidx.get(*s as usize + 1).copied().filter(|e| *e != 255).map_or(ctx.targets.len(), |e| e as usize);
                                ctx.targets.iter().enumerate().filter(|(i, _)| *i >= start as usize && *i < end).map(|(_, t)| t.obj).collect()
                            }
                        }
                        None => ctx.targets.iter().map(|t| t.obj).collect(),
                    } };
                    (objs, self.eval_amount(amount, &c))
                };
                if objs.is_empty() || total <= 0 {
                    return Exec::Done;
                }
                self.divide_prevent.set(false);
                self.divide_next(ctx, objs, 0, total, vec![], None, source, controller)
            }
            Effect::DividePrevention { at_cast: true, .. } => {
                let divs = ctx.item.divisions.clone();
                let chosen: Vec<Obj> = ctx.item.targets.iter().map(|t| t.obj).collect();
                let live: Vec<Obj> = ctx.targets.iter().map(|t| t.obj).collect();
                for (o, d) in chosen.iter().zip(divs.iter()) {
                    if *d > 0 && live.contains(o) {
                        self.shields.push(Shield { to: Some(vec![*o]), from: None, combat_only: false, remaining: Some(*d), redirect: None, once: false, counters: false, end_counters: None, or_player: None, redirect_by: None, from_ctl: None });
                    }
                }
                Exec::Done
            }
            Effect::DividePrevention { amount, .. } => {
                if let Some(Sub::Divide { targets, idx, left, parts, counters }) = ctx.sub.clone() {
                    ctx.sub = None;
                    let mut parts = parts;
                    let remaining_targets = targets.len() - idx;
                    let max = left - (remaining_targets as i32 - 1);
                    let n = match self.answer.take() {
                        Some(Choice::Number { n }) => n.clamp(1, max.max(1)),
                        _ => 1,
                    };
                    parts.push(n);
                    self.divide_prevent.set(true);
                    return self.divide_next(ctx, targets, idx + 1, left - n, parts, counters, source, controller);
                }
                let (objs, total) = {
                    let c = ec!();
                    let objs: Vec<Obj> = ctx.targets.iter().map(|t| t.obj).collect();
                    (objs, self.eval_amount(amount, &c))
                };
                if objs.is_empty() || total <= 0 {
                    return Exec::Done;
                }
                self.divide_prevent.set(true);
                self.divide_next(ctx, objs, 0, total, vec![], None, source, controller)
            }
            Effect::ScopeTargets { base, n } => {
                if *n == 255 {
                    if let Some(f) = ctx.full_targets.take() {
                        ctx.targets = f.0;
                        ctx.tidx = f.1;
                    }
                    return Exec::Done;
                }
                let (ft, fi) = match ctx.full_targets.take() { Some(f) => (f.0, f.1), None => (ctx.targets.clone(), ctx.tidx.clone()) };
                let (b, n) = (*base as usize, *n as usize);
                let first_legal = |lo: usize, hi: usize| fi.iter().enumerate().filter(|(i, v)| *i >= lo && *i < hi && **v != 255).map(|(_, v)| *v as usize).next();
                let start = first_legal(b, b + n);
                let end = first_legal(b + n, fi.len()).unwrap_or(ft.len());
                match start {
                    Some(s) => {
                        ctx.targets = ft[s..end.max(s)].to_vec();
                        ctx.tidx = (b..(b + n).min(fi.len())).map(|i| if fi[i] == 255 { 255 } else { fi[i] - s as u8 }).collect();
                    }
                    None => {
                        ctx.targets = vec![];
                        ctx.tidx = (0..n).map(|_| 255u8).collect();
                    }
                }
                ctx.full_targets = Some(Box::new((ft, fi)));
                Exec::Done
            }
            Effect::DivideCounters { kind, at_cast: true, .. } => {
                let divs = ctx.item.divisions.clone();
                let chosen: Vec<Obj> = ctx.item.targets.iter().map(|t| t.obj).collect();
                let live: Vec<Obj> = ctx.targets.iter().map(|t| t.obj).collect();
                for (o, d) in chosen.iter().zip(divs.iter()) {
                    if let Obj::Card { id } = o {
                        if *d > 0 && live.contains(o) && self.cards[id.0 as usize].zone == Zone::Battlefield {
                            self.cards[id.0 as usize].add_counters(*kind, *d);
                            self.layers_dirty = true;
                            self.emit(Event::Counters { card: *id, kind: *kind, delta: *d, by: self.cause_ctl.get() });
                        }
                    }
                }
                Exec::Done
            }
            Effect::DivideCounters { kind, total, remove_at_cleanup, among, up_to, remove_at_end_step, take_from, .. } => {
                self.divide_cleanup.set(*remove_at_cleanup);
                self.divide_end_step.set(*remove_at_end_step);
                self.divide_from.set(take_from.as_ref().and_then(|s| {
                    let c = ec!();
                    self.sel_cards(s, &c).first().copied()
                }));
                if let Some(Sub::DivideTotal { objs, max }) = ctx.sub.clone() {
                    ctx.sub = None;
                    let lo = objs.len() as i32;
                    let n = match self.answer.take() {
                        Some(Choice::Number { n }) => n.clamp(lo, max.max(lo)),
                        _ => max.max(lo),
                    };
                    self.divide_prevent.set(false);
                    return self.divide_next(ctx, objs, 0, n, vec![], Some(*kind), source, controller);
                }
                if let Some(Sub::Divide { targets, idx, left, parts, counters }) = ctx.sub.clone() {
                    ctx.sub = None;
                    let mut parts = parts;
                    let remaining_targets = targets.len() - idx;
                    let max = left - (remaining_targets as i32 - 1);
                    let n = match self.answer.take() {
                        Some(Choice::Number { n }) => n.clamp(1, max.max(1)),
                        _ => 1,
                    };
                    parts.push(n);
                    return self.divide_next(ctx, targets, idx + 1, left - n, parts, counters, source, controller);
                }
                // "distribute N counters among ...": same flow as dividing damage, over the chosen targets
                let (objs, tot) = {
                    let c = ec!();
                    let objs: Vec<Obj> = match among {
                        Some(s) => self.sel_objs(s, &c),
                        None => ctx.targets.iter().map(|t| t.obj).collect(),
                    };
                    (objs, self.eval_amount(total, &c))
                };
                if objs.is_empty() || tot <= 0 {
                    return Exec::Done;
                }
                if *up_to && (objs.len() as i32) < tot {
                    let lo = objs.len() as i32;
                    ctx.sub = Some(Sub::DivideTotal { objs, max: tot });
                    self.pending = Some(Decision {
                        player: controller,
                        kind: DecisionKind::Number { prompt: format!("{}: how many counters to distribute (up to {})", self.def_of(source).name, tot), min: lo, max: tot, cancellable: false },
                    });
                    return Exec::Wait;
                }
                self.divide_prevent.set(false);
                self.divide_next(ctx, objs, 0, tot, vec![], Some(*kind), source, controller)
            }
            Effect::CopySpell { what, new_targets, by, except, target, each_other_target, for_opponent, even_if_gone } => {
                // "whenever you cast ... copy that spell" (storm, replicate, casualty ...) copies the spell as it was even when it has been countered since
                let even_if_gone = &(*even_if_gone || matches!(what, Sel::TriggerSource));
                let controller = match by {
                    Some(sel) => {
                        let c = ec!();
                        self.sel_players(sel, &c).first().copied().unwrap_or(controller)
                    }
                    None => controller,
                };
                self.retarget_must_change.set(false);
                self.retarget_random.set(false);
                self.retarget_exclude.set(false);
                self.retarget_queue.borrow_mut().clear();
                self.retarget_only.set(None);
                self.retarget_forced.set(false);
                self.retarget_players_only.set(false);
                *self.retarget_filter.borrow_mut() = None;
                if let Some(Sub::Retarget { sid, idx, options }) = ctx.sub.clone() {
                    ctx.sub = None;
                    if let Some(Choice::Pick { indices }) = self.answer.take() {
                        if let Some(o) = indices.first().and_then(|i| options.get(*i)) {
                            let mut became: Option<(Obj, CardId, PlayerId, bool, StackId)> = None;
                            if let Some(item) = self.stack.iter_mut().find(|s| s.id == sid) {
                                if let Some(t) = item.targets.get_mut(idx) {
                                    let changed = t.obj != *o;
                                    t.obj = *o;
                                    t.zcc = match o {
                                        Obj::Card { id } => self.cards[id.0 as usize].zcc,
                                        _ => 0,
                                    };
                                    if changed {
                                        became = Some((*o, item.source, item.controller, matches!(item.kind, ItemKind::Spell { .. }), item.id));
                                    }
                                }
                            }
                            // the new target "becomes the target" of the spell or ability (Emberheart Challenger's valiant)
                            if let Some((obj, by, ctl, spell, sid2)) = became {
                                match obj {
                                    Obj::Card { id } => self.emit(Event::Targeted { card: id, by, controller: ctl, spell, sid: sid2 }),
                                    Obj::Player { id } => self.emit(Event::PlayerTargeted { player: id, by, controller: ctl, spell, sid: sid2 }),
                                    _ => {}
                                }
                            }
                        }
                    }
                    if let Exec::Wait = self.retarget_next(ctx, sid, idx + 1, source, controller) {
                        return Exec::Wait;
                    }
                    let rest = std::mem::take(&mut ctx.copy_rest);
                    return self.copy_loop(ctx, rest, *new_targets, false, source, controller, except, None, *even_if_gone);
                }
                let mut ids: Vec<StackId> = {
                    let c = ec!();
                    self.sel_objs(what, &c)
                        .into_iter()
                        .filter_map(|o| match o {
                            Obj::Stack { id } => Some(id),
                            // a spell named by its card (TriggerObject of a cast trigger)
                            Obj::Card { id } if self.cards[id.0 as usize].zone == Zone::Stack && self.stack.iter().any(|s| s.source == id && !s.copy && matches!(s.kind, ItemKind::Spell { .. })) => self.stack.iter().rev().find(|s| s.source == id && !s.copy && matches!(s.kind, ItemKind::Spell { .. })).map(|s| s.id),
                            // a spell that has already left the stack (countered before the copy trigger resolved)
                            Obj::Card { id } if *even_if_gone && self.cards[id.0 as usize].zone != Zone::Stack && !self.stack.iter().any(|s| s.source == id && matches!(s.kind, ItemKind::Spell { .. })) && self.stack_lki_items.iter().any(|s| s.source == id && !s.copy && matches!(s.kind, ItemKind::Spell { .. })) => self.stack_lki_items.iter().rev().find(|s| s.source == id && !s.copy && matches!(s.kind, ItemKind::Spell { .. })).map(|s| s.id),
                            // an ability named by its source permanent (TriggerObject of an AbilityActivated trigger): the topmost ability of it on the stack
                            Obj::Card { id } => self.stack.iter().rev().find(|s| s.source == id && !s.copy && matches!(s.kind, ItemKind::Ability { .. })).or_else(|| self.stack.iter().rev().find(|s| s.source == id && matches!(s.kind, ItemKind::Ability { .. }))).map(|s| s.id),
                            _ => None,
                        })
                        .collect()
                };
                // "copy this spell" while it is resolving: the item is no longer on the stack, copy it from here
                let resolving_copy = matches!(what, Sel::This) && matches!(ctx.item.kind, ItemKind::Spell { .. });
                if resolving_copy {
                    self.stack.push(ctx.item.clone());
                    ids = vec![ctx.item.id];
                }
                // Twinning Staff: copying a spell once more than asked (not for a spell copying itself while it resolves)
                let extra = if resolving_copy { 0 } else { self.copy_spell_extra(controller) };
                if extra > 0 {
                    ids = ids
                        .into_iter()
                        .flat_map(|sid| if self.stack.iter().any(|s| s.id == sid && matches!(s.kind, ItemKind::Spell { .. })) { vec![sid; 1 + extra] } else { vec![sid] })
                        .collect();
                }
                ctx.copy_made.clear();
                if *each_other_target {
                    // one copy per other legal target of the (single-target) spell or ability
                    let mut made: Vec<Obj> = vec![];
                    for sid in ids {
                        let Some(item) = self.stack.iter().find(|s| s.id == sid).cloned() else { continue };
                        let (specs, modal) = if matches!(item.kind, ItemKind::Spell { .. }) {
                            self.spell_specs(item.source)
                        } else {
                            let db = self.db.clone();
                            (self.item_specs(&item, &db), None)
                        };
                        if modal.is_some() || specs.len() != 1 || item.targets.len() != 1 {
                            continue;
                        }
                        let cur = item.targets[0].obj;
                        let options: Vec<Obj> = self.legal_targets(&specs[0], item.source, item.controller).into_iter().filter(|o| *o != cur && matches!(o, Obj::Player { .. } | Obj::Card { .. } | Obj::Stack { .. })).collect();
                        for o in options {
                            self.copy_loop(ctx, vec![sid], false, false, source, controller, except, Some(o), false);
                            made.extend(std::mem::take(&mut ctx.copy_made));
                        }
                    }
                    ctx.remembered = made;
                    return Exec::Done;
                }
                if let Some(osel) = for_opponent {
                    // one copy for this opponent, aimed at them or a permanent they control (Zevlor, Elturel Exile); none when the original already targets them
                    let opp = {
                        let c = ec!();
                        self.sel_players(osel, &c).first().copied()
                    };
                    let Some(opp) = opp else { return Exec::Done };
                    for sid in ids {
                        let Some(item) = self.stack.iter().find(|s| s.id == sid).cloned() else { continue };
                        let (specs, modal) = if matches!(item.kind, ItemKind::Spell { .. }) {
                            self.spell_specs(item.source)
                        } else {
                            let db = self.db.clone();
                            (self.item_specs(&item, &db), None)
                        };
                        // a modal spell's target slots are those of its chosen modes
                        let specs = if modal.is_some() { let db = self.db.clone(); self.item_specs(&item, &db) } else { specs };
                        if item.targets.len() != 1 {
                            continue;
                        }
                        // a spell with several target slots of which only one was filled: the copy aims the filled slot
                        let slot = if specs.len() == 1 { 0 } else { match item.spec_counts.iter().position(|c| *c == 1) { Some(i) if i < specs.len() && item.spec_counts.iter().filter(|c| **c > 0).count() == 1 => i, _ => continue } };
                        let victim = match item.targets[0].obj {
                            Obj::Player { id } => Some(id),
                            Obj::Card { id } => Some(self.cards[id.0 as usize].cur.controller),
                            _ => None,
                        };
                        if victim.is_none() || victim == Some(opp) {
                            continue;
                        }
                        let options: Vec<Obj> = self
                            .legal_targets(&specs[slot], item.source, item.controller)
                            .into_iter()
                            .filter(|o| match o {
                                Obj::Player { id } => *id == opp,
                                Obj::Card { id } => self.cards[id.0 as usize].zone == Zone::Battlefield && self.cards[id.0 as usize].cur.controller == opp,
                                _ => false,
                            })
                            .collect();
                        let Some(&first) = options.first() else { continue };
                        self.copy_loop(ctx, vec![sid], false, false, source, controller, except, Some(first), false);
                        // several legal choices: the copy's controller picks the target
                        if options.len() > 1 {
                            if let Some(Obj::Stack { id: cid }) = ctx.remembered.first().copied() {
                                ctx.sub = Some(Sub::Retarget { sid: cid, idx: 0, options: options.clone() });
                                self.pending = Some(Decision {
                                    player: controller,
                                    kind: DecisionKind::Pick {
                                        prompt: format!("{}: choose the copy's target", self.def_of(source).name),
                                        source: Some(source),
                                        options: options.iter().map(|o| Opt::Obj { obj: *o }).collect(),
                                        min: 1,
                                        max: 1,
                                        cancellable: false,
                                        hidden: false,
                                        purpose: PickPurpose::Generic,
                                    },
                                });
                                return Exec::Wait;
                            }
                        }
                    }
                    return Exec::Done;
                }
                let forced = target.as_ref().and_then(|t| {
                    let c = ec!();
                    self.sel_objs(t, &c).first().copied()
                });
                self.copy_loop(ctx, ids, *new_targets && forced.is_none(), resolving_copy, source, controller, except, forced, *even_if_gone)
            }
            Effect::Dig { .. } => {
                self.log(format!("effect not implemented yet: {:?}", std::mem::discriminant(e)));
                Exec::Done
            }
        }
    }

    /// The card's mana cost with its generic part reduced by the generic part of `reduce`, as cost text (foretell costs).
    fn reduced_cost_text(&self, id: CardId, reduce: &str) -> String {
        let mut m = self.db.def(self.cards[id.0 as usize].def).cost.clone();
        if let Ok(r) = ManaCost::parse(reduce) {
            let mut left: i32 = r.shards.iter().map(|s| if let Shard::Generic { n } = s { *n as i32 } else { 0 }).sum();
            for sh in m.shards.iter_mut() {
                if let Shard::Generic { n } = sh {
                    let x = (*n as i32).min(left);
                    *n -= x as u8;
                    left -= x;
                }
            }
            m.shards.retain(|s| !matches!(s, Shard::Generic { n: 0 }));
        }
        m.display()
    }

    /// `ManaProduce::AnyColorScaled.colors_of`: the colors of the first selected card (every color when it has none).
    fn colors_of_first(&self, of: &Sel, c: &EvalCtx) -> Vec<Color> {
        let cols: Vec<Color> = self.sel_cards(of, c).first().map(|id| Color::ALL.iter().copied().filter(|col| self.def_of(*id).colors.has(*col)).collect()).unwrap_or_default();
        if cols.is_empty() { Color::ALL.to_vec() } else { cols }
    }

    /// `ManaProduce::CostOf`: the mana of the first selected permanent's mana cost that is fixed (coloured / generic / {C} symbols) and the colour choices of its hybrid symbols.
    fn cost_of_parts(&self, of: &Sel, c: &EvalCtx) -> (Vec<(ManaType, u16)>, Vec<Vec<Color>>) {
        let (mut fixed, mut hybrids) = (vec![], vec![]);
        let Some(id) = self.sel_cards(of, c).first().copied() else { return (fixed, hybrids) };
        if self.cards[id.0 as usize].cur.no_mana_cost {
            return (fixed, hybrids);
        }
        for s in self.def_of(id).cost.shards.iter() {
            match s {
                Shard::Generic { n } => fixed.push((ManaType::C, *n as u16)),
                Shard::Colorless => fixed.push((ManaType::C, 1)),
                Shard::Color { c } | Shard::Phyrexian { c } | Shard::HybridGeneric { c } | Shard::HybridColorless { c } => fixed.push((ManaType::of(*c), 1)),
                Shard::Hybrid { a, b } | Shard::PhyrexianHybrid { a, b } => hybrids.push(vec![*a, *b]),
                Shard::Snow | Shard::X => {}
            }
        }
        (fixed, hybrids)
    }

    /// Copy each spell in `ids` (the effect's controller controls the copies); with `new_targets` each copy may be retargeted, which pauses the loop.
    fn copy_loop(&mut self, ctx: &mut ResolveCtx, ids: Vec<StackId>, new_targets: bool, resolving_copy: bool, source: CardId, controller: PlayerId, except: &[CopyExcept], forced: Option<Obj>, even_if_gone: bool) -> Exec {
        let mut it = ids.into_iter();
        while let Some(sid) = it.next() {
            // a spell that was countered (or has resolved) before the copy trigger resolved is copied as it last was on the stack
            let Some(orig) = self.stack.iter().find(|s| s.id == sid).cloned().or_else(|| if !even_if_gone { None } else { self.stack_lki_items.iter().rev().find(|s| s.id == sid && matches!(s.kind, ItemKind::Spell { .. })).cloned() }) else { continue };
            if matches!(orig.kind, ItemKind::Spell { .. }) && self.def_of(orig.source).abilities.iter().any(|a| matches!(a, Ability::CantBeCopied)) {
                continue;
            }
            if let ItemKind::Ability { ability, .. } | ItemKind::Triggered { ability, .. } = &orig.kind {
                let granted = if *ability >= crate::cast::GRANTED_BASE { orig.granted_snap.clone().or_else(|| self.granted_ab(orig.source, *ability)) } else { None };
                let db = self.db.clone();
                if let Some(Ability::Activated { effects, .. } | Ability::Triggered { effects, .. }) = granted.as_deref().or_else(|| db.def(orig.def).abilities.get(*ability as usize)) {
                    if effects.iter().any(|e| matches!(e, Effect::AbilityCantBeCopied)) {
                        continue;
                    }
                }
            }
            // a copy forced onto a chosen object ("for each other creature that ability could target") is made only when that object is a legal target
            if let Some(o) = forced {
                let (specs, modal) = if matches!(orig.kind, ItemKind::Spell { .. }) {
                    self.spell_specs(orig.source)
                } else {
                    let db = self.db.clone();
                    (self.item_specs(&orig, &db), None)
                };
                if modal.is_none() && specs.len() == 1 && !self.legal_targets(&specs[0], orig.source, controller).contains(&o) {
                    continue;
                }
            }
            if resolving_copy {
                self.stack.retain(|s| s.id != sid);
            }
            self.next_stack_id += 1;
            let mut copy = orig.clone();
            copy.id = StackId(self.next_stack_id);
            copy.copy = true;
            copy.copy_except = except.to_vec();
            copy.controller = controller;
            if let Some(o) = forced {
                // every slot that aimed at the original single target (a spell with two targets on the same Golem) moves to the new one
                let old = copy.targets.first().map(|t| t.obj);
                let zcc = match o {
                    Obj::Card { id } => self.cards[id.0 as usize].zcc,
                    _ => 0,
                };
                for t in copy.targets.iter_mut().filter(|t| Some(t.obj) == old) {
                    t.obj = o;
                    t.zcc = zcc;
                }
            }
            // a copy of a spell is a spell the effect's controller controls
            self.stack.push(copy.clone());
            ctx.copy_made.push(Obj::Stack { id: copy.id });
            let is_spell = matches!(copy.kind, ItemKind::Spell { .. });
            if is_spell {
                self.emit(Event::Copied { card: copy.source, player: controller });
            }
            if new_targets {
                if let Exec::Wait = self.retarget_next(ctx, copy.id, 0, source, controller) {
                    ctx.copy_rest = it.collect();
                    return Exec::Wait;
                }
            }
        }
        ctx.remembered = std::mem::take(&mut ctx.copy_made);
        Exec::Done
    }

    /// Divide damage among targets: ask for each portion but the last (CR 601.2d is done at resolution here).
    fn divide_next(&mut self, ctx: &mut ResolveCtx, targets: Vec<Obj>, idx: usize, left: i32, parts: Vec<i32>, counters: Option<CounterKind>, source: CardId, controller: PlayerId) -> Exec {
        let n_targets = targets.len();
        let any = self.divide_any.replace(false);
        if idx + 1 < n_targets && left > if any { 0 } else { (n_targets - idx - 1) as i32 } {
            ctx.sub = Some(Sub::Divide { targets, idx, left, parts, counters });
            let max = if any { left } else { left - (n_targets - idx - 1) as i32 };
            self.divide_any.set(any);
            self.pending = Some(Decision {
                player: controller,
                kind: DecisionKind::Number { prompt: format!("{}: damage to target {} of {}", self.def_of(source).name, idx + 1, n_targets), min: if any { 0 } else { 1 }, max, cancellable: false },
            });
            return Exec::Wait;
        }
        // the last target gets the rest; targets we ran out of damage for get nothing
        let mut parts = parts;
        if idx < n_targets {
            parts.push(left);
        }
        let prevent = self.divide_prevent.replace(false);
        let mut moved_total = 0;
        for (o, d) in targets.iter().zip(parts.iter()) {
            if *d > 0 && prevent {
                self.shields.push(Shield { to: Some(vec![*o]), from: None, combat_only: false, remaining: Some(*d), redirect: None, once: false, counters: false, end_counters: None, or_player: None, redirect_by: None, from_ctl: None });
            } else if *d > 0 {
                match (counters, o) {
                    (Some(kind), Obj::Card { id }) => {
                        if self.cards[id.0 as usize].zone == Zone::Battlefield {
                            self.cards[id.0 as usize].add_counters(kind, *d);
                            moved_total += *d;
                            self.layers_dirty = true;
                            self.emit(Event::Counters { card: *id, kind, delta: *d, by: self.cause_ctl.get() });
                            if self.divide_cleanup.get() {
                                let zcc = self.cards[id.0 as usize].zcc;
                                self.cleanup_counter_removals.push((*id, zcc, kind, *d));
                            }
                            if self.divide_end_step.get() {
                                let did = self.next_effect_id;
                                self.next_effect_id += 1;
                                self.delayed.push(DelayedTrigger {
                                    id: did,
                                    controller,
                                    source,
                                    when: DelayedWhen::NextEndStep,
                                    effects: vec![Effect::RemoveCounters { what: Sel::Remembered, kind, n: Amount::Fixed { n: *d } }],
                                    created_turn: self.turn,
                                    targets: vec![],
                                    ctx: EventCtx::default(),
                                    remembered: vec![Obj::card(*id)],
                                });
                            }
                        }
                    }
                    (Some(_), _) => {}
                    (None, _) => {
                        self.deal_damage(Some(source), *o, *d, false);
                    }
                }
            }
        }
        // "move" counters (Forgotten Ancient): what was actually put on the creatures comes off the source
        if let (Some(kind), Some(from)) = (counters, self.divide_from.take()) {
            if moved_total > 0 {
                self.remove_counters_from(Obj::card(from), kind, moved_total);
            }
        }
        Exec::Done
    }

    /// Offer the next target slot of a copied spell for changing (a slot with no other legal choice is skipped).
    fn retarget_next(&mut self, ctx: &mut ResolveCtx, sid: StackId, from: usize, source: CardId, controller: PlayerId) -> Exec {
        let mut r = self.retarget_next_one(ctx, sid, from, source, controller);
        // `ChangeTargets.each_stack`: continue with the next queued stack item once this one is done
        while matches!(r, Exec::Done) {
            let next = self.retarget_queue.borrow_mut().pop();
            let Some(n) = next else { break };
            r = self.retarget_next_one(ctx, n, 0, source, controller);
        }
        r
    }

    fn retarget_next_one(&mut self, ctx: &mut ResolveCtx, sid: StackId, from: usize, source: CardId, controller: PlayerId) -> Exec {
        let Some(item) = self.stack.iter().find(|s| s.id == sid).cloned() else { return Exec::Done };
        // a copied ability retargets through the ability's own target specs
        let (specs, modal) = if matches!(item.kind, ItemKind::Spell { .. }) {
            let (sp, m) = self.spell_specs(item.source);
            // target slots that exist only when kicked (or only when not kicked) follow the copied spell's kicked state
            let kicked = self.cards[item.source.0 as usize].kicked;
            (sp.into_iter().filter(|t| t.when_kicked.map_or(true, |w| w == kicked)).collect(), m)
        } else {
            let db = self.db.clone();
            (self.item_specs(&item, &db), None)
        };
        // a copy of a modal spell keeps its modes (CR 707.10): its target slots are those of the chosen modes
        let specs = if modal.is_some() {
            let db = self.db.clone();
            self.item_specs(&item, &db)
        } else {
            specs
        };
        // a slot with several targets ("two target creatures") offers new targets for each of them
        let specs = if !item.copy && specs.len() < item.targets.len() && specs.len() == item.spec_counts.len() {
            let mut v: Vec<TargetSpec> = vec![];
            for (si, cnt) in item.spec_counts.iter().enumerate() {
                for _ in 0..*cnt {
                    v.push(specs[si].clone());
                }
            }
            v
        } else {
            specs
        };
        if specs.len() != item.targets.len() {
            return Exec::Done;
        }
        for idx in from..item.targets.len() {
            // choosing new targets for a copy is not casting or activating: Flagbearer's "must choose it" does not apply
            self.target_trig.set(true);
            let mut options = self.legal_targets(&specs[idx], item.source, item.controller);
            self.target_trig.set(false);
            let cur = item.targets[idx].obj;
            if let Some(only) = self.retarget_only.get() {
                options.retain(|o| *o == only);
            }
            if self.retarget_players_only.get() {
                options.retain(|o| matches!(o, Obj::Player { .. }));
            }
            let nf = self.retarget_filter.borrow().clone();
            if let Some(f) = nf {
                options.retain(|o| matches!(o, Obj::Card { id } if self.matches_card(&f, *id, item.source, item.controller)));
            }
            if self.retarget_exclude.get() {
                options.retain(|o| match o {
                    Obj::Player { id } => *id != controller,
                    Obj::Card { id } => self.cards[id.0 as usize].cur.controller != controller,
                    _ => true,
                });
            }
            if self.retarget_forced.get() {
                // "change the target to X if able": no choice, and nothing changes when X isn't a legal target
                if let Some(only) = self.retarget_only.get().filter(|o| options.contains(o) && *o != cur) {
                    if let Some(it) = self.stack.iter_mut().find(|s| s.id == sid) {
                        if let Some(t) = it.targets.get_mut(idx) {
                            t.obj = only;
                            t.zcc = match only {
                                Obj::Card { id } => self.cards[id.0 as usize].zcc,
                                _ => 0,
                            };
                        }
                    }
                }
                continue;
            }
            let excl = self.retarget_exclude.get();
            if !excl && !options.contains(&cur) {
                options.insert(0, cur);
            }
            if options.len() < if excl { 1 } else { 2 } {
                continue;
            }
            if self.retarget_random.get() {
                let pick = options[self.rand_index(&EvalCtx::with_controller(item.source, item.controller), options.len())];
                if let Some(t) = self.stack.iter_mut().find(|s| s.id == sid).and_then(|s| s.targets.get_mut(idx)) {
                    t.obj = pick;
                    t.zcc = match pick {
                        Obj::Card { id } => self.cards[id.0 as usize].zcc,
                        _ => 0,
                    };
                }
                continue;
            }
            // "change the target ... if able": only the other legal targets are offered (a choice is required)
            let must = self.retarget_must_change.get();
            if must {
                options.retain(|o| *o != cur);
            }
            ctx.sub = Some(Sub::Retarget { sid, idx, options: options.clone() });
            self.pending = Some(Decision {
                player: controller,
                kind: DecisionKind::Pick {
                    prompt: format!("{}: choose a new target, or none to keep it", self.def_of(item.source).name),
                    source: Some(source),
                    options: options.iter().map(|o| Opt::Obj { obj: *o }).collect(),
                    min: must as u8,
                    max: 1,
                    cancellable: false,
                    hidden: false,
                    purpose: PickPurpose::Generic,
                },
            });
            return Exec::Wait;
        }
        Exec::Done
    }

    /// Execute one effect immediately for `controller` (mana abilities' side effects). Effects that need a decision are skipped.
    pub fn run_effect_now(&mut self, source: CardId, controller: PlayerId, e: &Effect) {
        self.run_effect_with(source, controller, e, EventCtx::default());
    }

    /// Like `run_effect_now`, with the event that triggered it (so "that permanent" / "its controller" resolve).
    pub fn run_effect_with(&mut self, source: CardId, controller: PlayerId, e: &Effect, event: EventCtx) {
        self.next_stack_id += 1;
        let item = StackItem {
            id: StackId(self.next_stack_id),
            controller,
            source,
            src_zcc: self.cards[source.0 as usize].zcc,
            def: self.cards[source.0 as usize].def,
            kind: ItemKind::Ability { ability: 0, mode: None },
            targets: vec![],
            x: self.cards[source.0 as usize].x,
            modes: Default::default(),
            ctx: event,
            copy: false,
            copy_except: vec![],
            divisions: Default::default(),
            spec_counts: Default::default(),
            granted_snap: None,
            spliced: vec![],
            remembered: vec![],
        };
        let mut ctx = ResolveCtx { item, frames: vec![], remembered: vec![], targets: vec![], tidx: SmallVec::new(), sub: None, fizzled: false, cur_tbase: 0, sac_done: vec![], copy_rest: vec![], copy_made: vec![], unless_rest: vec![], full_targets: None };
        let prev_fe = self.filter_event.borrow().clone();
        *self.filter_event.borrow_mut() = Some(ctx.item.ctx.clone());
        if let Exec::Push(f) = self.exec(&mut ctx, e) {
            // nested lists (If / Repeat ...): run them to the end
            let mut stack = vec![f];
            while let Some(mut fr) = stack.pop() {
                loop {
                    while fr.pc < fr.list.len() {
                        let eff = fr.list[fr.pc].clone();
                        fr.pc += 1;
                        if let Exec::Push(n) = self.exec(&mut ctx, &eff) {
                            stack.push(n);
                        }
                    }
                    // Repeat: run the list again
                    if fr.repeat > 0 {
                        fr.repeat -= 1;
                        fr.pc = 0;
                    } else {
                        break;
                    }
                }
            }
        }
        *self.filter_event.borrow_mut() = prev_fe;
        // an "any color" mana trigger can't stop to ask (it isn't on the stack): add the color the controller's hand wants most
        let mut guard = 0;
        while matches!(ctx.sub, Some(Sub::ManaColor { .. })) && guard < 8 {
            guard += 1;
            let mut want = [0i32; 5];
            // the mana goes to `who` when given (Dawn's Reflection: the land's controller), so that player's hand decides
            let recipient = match e {
                Effect::AddMana { who: Some(w), .. } => {
                    let c = self.evalctx(&ctx);
                    self.sel_players(w, &c).first().copied().unwrap_or(controller)
                }
                _ => controller,
            };
            for &h in &self.players[recipient as usize].hand {
                for s in &self.def_of(h).cost.shards {
                    if let Shard::Color { c } = s {
                        if let Some(i) = Color::ALL.iter().position(|x| x == c) {
                            want[i] += 1;
                        }
                    }
                }
            }
            let mut best = (0..5).max_by_key(|i| (want[*i], -(*i as i32))).unwrap_or(0);
            // a colour the player named for this tap wins over the guess
            let named = { let mut h = self.mana_color_hints.borrow_mut(); if h.is_empty() { None } else { Some(h.remove(0)) } };
            if let Some(t) = named {
                if let Some(i) = Color::ALL.iter().position(|c| crate::mana::ManaType::of(*c) == t) {
                    best = i;
                }
            }
            self.pending = None;
            self.answer = Some(Choice::Pick { indices: vec![best] });
            let _ = self.exec(&mut ctx, e);
        }
        self.answer = None;
        if self.pending.is_some() && matches!(ctx.sub, Some(_)) {
            self.pending = None;
        }
    }

    /// Bind the chosen cards to `Remembered` and run the follow-up effects.
    fn choose_then(&mut self, ctx: &mut ResolveCtx, picked: Vec<CardId>, then: Vec<Effect>) -> Exec {
        if self.choose_accumulate.get() {
            ctx.remembered.extend(picked.into_iter().map(Obj::card));
        } else {
            ctx.remembered = picked.into_iter().map(Obj::card).collect();
        }
        if then.is_empty() {
            Exec::Done
        } else {
            Exec::Push(Frame { list: Arc::new(then), pc: 0, repeat: 0, each: vec![], each_active: false, tbase: ctx.cur_tbase })
        }
    }

    fn continue_discard(&mut self, ctx: &mut ResolveCtx, players: Vec<PlayerId>, ns: Vec<i32>, filter: Filter, mut idx: usize) -> Exec {
        // an answer for the current player?
        if ctx.sub.is_some() {
            let ans = self.answer.take();
            ctx.sub = None;
            if let (Some(Choice::Pick { indices }), Some(&p)) = (ans, players.get(idx)) {
                let eligible = self.eligible_in_hand(p, &filter, ctx.item.source);
                for i in indices.iter().take(ns[idx].max(0) as usize) {
                    if let Some(&id) = eligible.get(*i) {
                        self.move_card(id, Zone::Graveyard, Default::default());
                        self.emit(Event::Discarded { card: id, player: p, by: self.cause_ctl.get() });
                        ctx.sac_done.push(Obj::card(id));
                    }
                }
            }
            idx += 1;
        }
        while idx < players.len() {
            let p = players[idx];
            let n = ns[idx];
            let eligible = self.eligible_in_hand(p, &filter, ctx.item.source);
            if n <= 0 || eligible.is_empty() {
                idx += 1;
                continue;
            }
            if eligible.len() as i32 <= n {
                for id in eligible {
                    self.move_card(id, Zone::Graveyard, Default::default());
                    self.emit(Event::Discarded { card: id, player: p, by: self.cause_ctl.get() });
                    ctx.sac_done.push(Obj::card(id));
                }
                idx += 1;
                continue;
            }
            ctx.sub = Some(Sub::Discard { players: players.clone(), n: ns.clone(), filter: filter.clone(), idx });
            self.pending = Some(Decision {
                player: p,
                kind: DecisionKind::Pick {
                    prompt: format!("Discard {n} card(s)"),
                    source: Some(ctx.item.source),
                    options: eligible.iter().map(|c| Opt::Obj { obj: Obj::card(*c) }).collect(),
                    min: n as u8,
                    max: n as u8,
                    cancellable: false,
                    hidden: true,
                    purpose: PickPurpose::Discard,
                },
            });
            return Exec::Wait;
        }
        // what this discard effect discarded (nothing, when the hand is empty) replaces what was remembered before, e.g. a retrace cost's land
        ctx.remembered = std::mem::take(&mut ctx.sac_done);
        Exec::Done
    }

    fn eligible_in_graveyard(&self, p: PlayerId, filter: &Filter, source: CardId) -> Vec<CardId> {
        let mut f = filter.clone();
        f.controller = PlayerFilter::Any;
        self.players[p as usize].graveyard.iter().copied().filter(|id| self.matches_card(&f, *id, source, p)).collect()
    }

    fn eligible_in_hand(&self, p: PlayerId, filter: &Filter, source: CardId) -> Vec<CardId> {
        let mut f = filter.clone();
        f.controller = PlayerFilter::Any;
        if self.forced_discard_sac_barred(p) {
            return vec![];
        }
        self.players[p as usize].hand.iter().copied().filter(|id| self.matches_card(&f, *id, source, p)).collect()
    }

    fn continue_sacrifice(&mut self, ctx: &mut ResolveCtx, players: Vec<PlayerId>, ns: Vec<i32>, filter: Filter, mut idx: usize) -> Exec {
        if ctx.sub.is_some() {
            let ans = self.answer.take();
            ctx.sub = None;
            if let (Some(Choice::Pick { indices }), Some(&p)) = (ans, players.get(idx)) {
                let eligible = self.eligible_permanents(p, &filter, ctx.item.source);
                for i in indices.iter().take(ns[idx].max(0) as usize) {
                    if let Some(&id) = eligible.get(*i) {
                        self.sac_max_power.set(self.sac_max_power.get().max(self.power_of(id)));
                        self.emit(Event::Sacrificed { card: id, player: p });
                        self.move_card(id, Zone::Graveyard, Default::default());
                        ctx.sac_done.push(Obj::card(id));
                    }
                }
            }
            idx += 1;
        }
        while idx < players.len() {
            let p = players[idx];
            let n = ns[idx];
            let eligible = self.eligible_permanents(p, &filter, ctx.item.source);
            if n <= 0 || eligible.is_empty() {
                idx += 1;
                continue;
            }
            if eligible.len() as i32 <= n {
                for id in eligible {
                    self.sac_max_power.set(self.sac_max_power.get().max(self.power_of(id)));
                    self.emit(Event::Sacrificed { card: id, player: p });
                    self.move_card(id, Zone::Graveyard, Default::default());
                    ctx.sac_done.push(Obj::card(id));
                }
                idx += 1;
                continue;
            }
            ctx.sub = Some(Sub::Sacrifice { players: players.clone(), n: ns.clone(), filter: filter.clone(), idx });
            self.pending = Some(Decision {
                player: p,
                kind: DecisionKind::Pick {
                    prompt: format!("Sacrifice {n} permanent(s)"),
                    source: Some(ctx.item.source),
                    options: eligible.iter().map(|c| Opt::Obj { obj: Obj::card(*c) }).collect(),
                    min: n as u8,
                    max: n as u8,
                    cancellable: false,
                    hidden: false,
                    purpose: PickPurpose::Sacrifice,
                },
            });
            return Exec::Wait;
        }
        if !ctx.sac_done.is_empty() {
            ctx.remembered = std::mem::take(&mut ctx.sac_done);
        }
        Exec::Done
    }

    /// The permanents `p` may put the counters of a `PutCountersOnChosen` cost on: their own, or with `opponent` those an opponent controls.
    pub(crate) fn counter_targets(&self, p: PlayerId, filter: &Filter, source: CardId, opponent: bool) -> Vec<CardId> {
        if !opponent {
            return self.eligible_permanents(p, filter, source);
        }
        let mut f = filter.clone();
        f.controller = PlayerFilter::Any;
        self.battlefield.iter().copied().filter(|id| self.cards[id.0 as usize].cur.controller != p && self.matches_card(&f, *id, source, p)).collect()
    }

    /// The permanents `p` doesn't control that a `GainControlOfChosen` cost may take, minus those an earlier part of `costs` already takes.
    pub(crate) fn control_targets(&self, p: PlayerId, filter: &Filter, source: CardId, costs: &[Cost]) -> Vec<CardId> {
        let mut f = filter.clone();
        f.controller = PlayerFilter::Any;
        let taken: Vec<u32> = costs.iter().filter_map(|c| if let Cost::GainControlOfCard { card } = c { Some(*card) } else { None }).collect();
        self.battlefield.iter().copied().filter(|id| self.cards[id.0 as usize].cur.controller != p && !taken.contains(&id.0) && self.matches_card(&f, *id, source, p)).collect()
    }

    pub(crate) fn eligible_permanents(&self, p: PlayerId, filter: &Filter, source: CardId) -> Vec<CardId> {
        let mut f = filter.clone();
        f.controller = PlayerFilter::Any;
        if self.forced_sac_barred(p) {
            return vec![];
        }
        self.battlefield
            .iter()
            .copied()
            .filter(|id| self.cards[id.0 as usize].cur.controller == p && !self.cards[id.0 as usize].cur.cant_be_sacrificed && !self.trigger_token_barred(*id) && self.matches_card(&f, *id, source, p))
            .collect()
    }

    /// The first cost of `costs` that needs a decision from the payer (which alternative, which permanents or cards), if any.
    fn next_pay_choice(&self, p: PlayerId, source: CardId, costs: &[Cost]) -> Option<(usize, PayChoice)> {
        for (i, c) in costs.iter().enumerate() {
            match c {
                Cost::Or { options } => {
                    let ok: Vec<Vec<Cost>> = options.iter().filter(|o| self.can_pay_costs(p, source, o)).cloned().collect();
                    if ok.len() >= 2 {
                        return Some((i, PayChoice::Alternative(ok)));
                    }
                    if ok.len() == 1 {
                        return Some((i, PayChoice::Alternative(ok)));
                    }
                }
                Cost::Sacrifice { filter, n } => {
                    let el = self.eligible_permanents(p, filter, source);
                    if el.len() > *n as usize {
                        return Some((i, PayChoice::Sacrifice(el, *n as usize)));
                    }
                }
                Cost::SacrificeTotalPower { filter, power } => {
                    let el = self.eligible_permanents(p, filter, source);
                    if el.iter().map(|id| self.cards[id.0 as usize].cur.power.max(0)).sum::<i32>() >= *power {
                        return Some((i, PayChoice::SacrificePower(el, *power)));
                    }
                }
                Cost::Discard { filter, n, random: false } => {
                    let el = self.eligible_in_hand(p, filter, source);
                    if el.len() > *n as usize {
                        return Some((i, PayChoice::Discard(el, *n as usize)));
                    }
                }
                Cost::PutCountersOnChosen { filter, opponent, .. } => {
                    let el = self.counter_targets(p, filter, source, *opponent);
                    if el.len() > 1 {
                        return Some((i, PayChoice::Counters(el)));
                    }
                }
                Cost::ExileFromGraveyard { filter, n, bottom: false } => {
                    let mut f = filter.clone();
                    f.controller = PlayerFilter::Any;
                    let el: Vec<CardId> = self.players[p as usize].graveyard.iter().copied().filter(|id| self.matches_card(&f, *id, source, p)).collect();
                    if el.len() > *n as usize && *n > 0 && self.mana_picks.is_empty() {
                        return Some((i, PayChoice::Graveyard(el, *n as usize)));
                    }
                }
                Cost::GainControlOfChosen { filter } => {
                    let el = self.control_targets(p, filter, source, costs);
                    if !el.is_empty() {
                        return Some((i, PayChoice::Control(el)));
                    }
                }
                Cost::OpponentGainsLife { .. } | Cost::OpponentCreatesToken { .. } => {
                    let opps = self.cost_opponents(p);
                    if opps.len() > 1 {
                        return Some((i, PayChoice::Opponent(opps)));
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Pay `costs` for `p`, asking which alternative / permanents / cards when there is a real choice.
    /// Returns `Some(paid)` when finished, `None` when a question is pending (`ctx.sub` holds the state).
    fn pay_with_choices(&mut self, ctx: &mut ResolveCtx, p: PlayerId, source: CardId, costs: Vec<Cost>, unless: bool) -> Option<bool> {
        let Some((idx, choice)) = self.next_pay_choice(p, source, &costs) else {
            if unless {
                let saved = self.spent_track.replace([0; 6]);
                self.track_pay_spent = true;
                let ok = self.try_pay_costs(p, source, &costs);
                self.track_pay_spent = false;
                self.unless_spent = self.spent_track.replace(saved);
                return Some(ok);
            }
            return Some(self.try_pay_costs(p, source, &costs));
        };
        // a single possible alternative needs no question
        if let PayChoice::Alternative(opts) = &choice {
            if opts.len() == 1 {
                let mut next = costs.clone();
                next.splice(idx..=idx, opts[0].clone());
                return self.pay_with_choices(ctx, p, source, next, unless);
            }
        }
        if let PayChoice::Control(cards) = &choice {
            if cards.len() == 1 {
                let mut next = costs.clone();
                next[idx] = Cost::GainControlOfCard { card: cards[0].0 };
                return self.pay_with_choices(ctx, p, source, next, unless);
            }
        }
        let (prompt, options, count): (String, Vec<Opt>, usize) = match &choice {
            PayChoice::Alternative(opts) => (
                "Choose how to pay".into(),
                opts.iter().map(|o| Opt::Text { text: o.iter().map(crate::describe::cost_text).collect::<Vec<_>>().join(", ") }).collect(),
                1,
            ),
            PayChoice::Sacrifice(cards, n) => ("Choose permanents to sacrifice".into(), cards.iter().map(|c| Opt::Obj { obj: Obj::card(*c) }).collect(), *n),
            PayChoice::SacrificePower(cards, power) => (format!("Choose creatures to sacrifice with total power {power} or greater"), cards.iter().map(|c| Opt::Obj { obj: Obj::card(*c) }).collect(), 0),
            PayChoice::Discard(cards, n) => ("Choose cards to discard".into(), cards.iter().map(|c| Opt::Obj { obj: Obj::card(*c) }).collect(), *n),
            PayChoice::Counters(cards) => ("Choose a permanent to put the counters on".into(), cards.iter().map(|c| Opt::Obj { obj: Obj::card(*c) }).collect(), 1),
            PayChoice::Control(cards) => ("Choose a permanent to gain control of".into(), cards.iter().map(|c| Opt::Obj { obj: Obj::card(*c) }).collect(), 1),
            PayChoice::Graveyard(cards, n) => ("Choose cards to exile from your graveyard".into(), cards.iter().map(|c| Opt::Obj { obj: Obj::card(*c) }).collect(), *n),
            PayChoice::Opponent(players) => (format!("Choose an opponent: {}", crate::describe::cost_text(&costs[idx])), players.iter().map(|q| Opt::Obj { obj: Obj::player(*q) }).collect(), 1),
        };
        let (choice_kind, options_len) = (choice.clone(), options.len());
        ctx.sub = Some(Sub::PayPick { payer: p, costs, idx, choice, unless });
        self.pending = Some(Decision {
            player: p,
            kind: DecisionKind::Pick {
                prompt: format!("{}: {}", self.def_of(source).name, prompt),
                source: Some(source),
                options,
                min: if matches!(choice_kind, PayChoice::SacrificePower(..)) { 1 } else { count as u8 },
                max: if matches!(choice_kind, PayChoice::SacrificePower(..)) { options_len as u8 } else { count as u8 },
                cancellable: false,
                hidden: false,
                purpose: PickPurpose::Generic,
            },
        });
        None
    }

    /// Handle the answer to a `PayPick` question and keep paying. Returns `Some(paid)` when done.
    fn continue_pay(&mut self, ctx: &mut ResolveCtx, source: CardId) -> Option<(bool, bool)> {
        let Some(Sub::PayPick { payer, mut costs, idx, choice, unless }) = ctx.sub.clone() else { return Some((false, false)) };
        ctx.sub = None;
        let picked: Vec<usize> = match self.answer.take() {
            Some(Choice::Pick { indices }) => indices,
            _ => vec![],
        };
        match choice {
            PayChoice::Alternative(opts) => {
                let o = opts.get(picked.first().copied().unwrap_or(0)).cloned().unwrap_or_default();
                costs.splice(idx..=idx, o);
            }
            PayChoice::Sacrifice(cards, n) => {
                let mut chosen: Vec<u32> = picked.iter().filter_map(|i| cards.get(*i)).map(|c| c.0).collect();
                for c in cards.iter() {
                    if chosen.len() >= n {
                        break;
                    }
                    if !chosen.contains(&c.0) {
                        chosen.push(c.0);
                    }
                }
                chosen.truncate(n);
                costs[idx] = Cost::SacrificeCards { cards: chosen };
            }
            PayChoice::SacrificePower(cards, power) => {
                let mut chosen: Vec<u32> = picked.iter().filter_map(|i| cards.get(*i)).map(|c| c.0).collect();
                let pw = |c: u32, g: &Game| g.cards[c as usize].cur.power.max(0);
                let mut sorted: Vec<u32> = cards.iter().map(|c| c.0).collect();
                sorted.sort_by_key(|c| std::cmp::Reverse(pw(*c, self)));
                // a pick that falls short is topped up with the strongest remaining creatures
                for c in sorted {
                    if chosen.iter().map(|x| pw(*x, self)).sum::<i32>() >= power {
                        break;
                    }
                    if !chosen.contains(&c) {
                        chosen.push(c);
                    }
                }
                costs[idx] = Cost::SacrificeCards { cards: chosen };
            }
            PayChoice::Counters(cards) => {
                let id = picked.first().and_then(|i| cards.get(*i)).or(cards.first()).copied();
                if let (Some(id), Cost::PutCountersOnChosen { kind, n, .. }) = (id, costs[idx].clone()) {
                    costs[idx] = Cost::PutCountersOnCard { card: id.0, kind, n };
                }
            }
            PayChoice::Control(cards) => {
                let id = picked.first().and_then(|i| cards.get(*i)).or(cards.first()).copied();
                if let Some(id) = id {
                    costs[idx] = Cost::GainControlOfCard { card: id.0 };
                }
            }
            PayChoice::Opponent(players) => {
                if let Some(o) = picked.first().and_then(|i| players.get(*i)).or(players.first()).copied() {
                    costs[idx] = match costs[idx].clone() {
                        Cost::OpponentGainsLife { n } => Cost::PlayerGainsLife { player: o, n },
                        Cost::OpponentCreatesToken { token, n } => Cost::PlayerCreatesToken { player: o, token, n },
                        other => other,
                    };
                }
            }
            PayChoice::Graveyard(cards, n) => {
                // the payer's picks are exiled first; the cost pays for the rest itself
                let chosen: Vec<CardId> = picked.iter().filter_map(|i| cards.get(*i)).copied().take(n).collect();
                self.mana_picks = chosen;
            }
            PayChoice::Discard(cards, n) => {
                let mut chosen: Vec<u32> = picked.iter().filter_map(|i| cards.get(*i)).map(|c| c.0).collect();
                for c in cards.iter() {
                    if chosen.len() >= n {
                        break;
                    }
                    if !chosen.contains(&c.0) {
                        chosen.push(c.0);
                    }
                }
                chosen.truncate(n);
                costs[idx] = Cost::DiscardCards { cards: chosen };
            }
        }
        self.pay_with_choices(ctx, payer, source, costs, unless).map(|paid| (paid, unless))
    }

    /// Layer effects for "except ..." clauses of a copy.
    pub(crate) fn apply_copy_except(&mut self, source: CardId, controller: PlayerId, id: CardId, except: &[CopyExcept], remembered: &[Obj]) {
        for x in except {
            let m = match x {
                CopyExcept::NotLegendary => Modif::NotLegendary,
                CopyExcept::SetName { name } => Modif::SetName(Sym::new(name)),
                CopyExcept::NoManaCost => Modif::NoManaCost,
                CopyExcept::AddSupertype { supertype } => Modif::AddSupertype(*supertype),
                CopyExcept::GrantOwnAbility { index } => {
                    let front = self.cards[source.0 as usize].front;
                    match self.db.def(front).abilities.get(*index as usize) {
                        Some(a) => Modif::Static(Box::new(StaticEffect::GrantAbility { affects: (Filter::default()).into(), ability: Box::new(a.clone()) })),
                        None => continue,
                    }
                }
                CopyExcept::CdaPT { power, toughness } => Modif::Static(Box::new(StaticEffect::CdaPT { power: power.clone(), toughness: toughness.clone() })),
                CopyExcept::AddType { card_type } => Modif::AddTypes(TypeSet::of(&[*card_type])),
                CopyExcept::SetTypes { types, subtypes } => Modif::SetTypes(TypeSet::of(types), subtypes.iter().map(|s| Sym::new(s)).collect()),
                CopyExcept::AddSubtype { subtype } => Modif::AddSubtype(Sym::new(subtype)),
                CopyExcept::AddKeyword { keyword } => Modif::Keyword(*keyword),
                CopyExcept::SetPT { power, toughness } => Modif::SetPT { power: *power, toughness: *toughness },
                CopyExcept::SetPTAmount { power, toughness } => {
                    // "half the power of the creature that died": the effect's remembered cards are still the ones it named
                    let mut ec = crate::eval::EvalCtx::with_controller(source, controller);
                    ec.remembered = remembered;
                    Modif::SetPT { power: self.eval_amount(power, &ec), toughness: self.eval_amount(toughness, &ec) }
                }
                CopyExcept::SetPTOwn => {
                    let c = &self.cards[source.0 as usize].cur;
                    Modif::SetPT { power: c.power, toughness: c.toughness }
                }
                CopyExcept::SetColors { colors } => Modif::Colors(ColorSet::of(colors)),
                CopyExcept::AddColors { colors } => Modif::AddColors(ColorSet::of(colors)),
                CopyExcept::SetSubtypes { subtypes } => Modif::SetSubtypes(subtypes.iter().map(|s| Sym::new(s)).collect()),
                CopyExcept::GrantAbility { ability } => Modif::Static(Box::new(StaticEffect::GrantAbility { affects: (Filter::default()).into(), ability: ability.clone() })),
                CopyExcept::Vanishing { n } => {
                    let has = self.db.def(self.cards[id.0 as usize].def).abilities.iter().any(|a| matches!(a, Ability::Triggered { trigger: Trigger::BeginningOfStep { step: Step::Upkeep, .. }, effects, .. } if effects.iter().any(|e| matches!(e, Effect::RemoveCounters { kind: CounterKind::Time, .. }))));
                    if has {
                        // the copy of a creature with vanishing enters with that creature's time counters
                        let t: i32 = self.db.def(self.cards[id.0 as usize].def).abilities.iter().find_map(|a| if let Ability::EntersWithCounters { kind: CounterKind::Time, n: Amount::Fixed { n }, .. } = a { Some(*n) } else { None }).unwrap_or(0);
                        if t > 0 && self.cards[id.0 as usize].counter(CounterKind::Time) == 0 {
                            self.add_counters_to(Obj::card(id), CounterKind::Time, t);
                        }
                    } else {
                        self.add_counters_to(Obj::card(id), CounterKind::Time, *n);
                        for js in [
                            r#"{"type":"Triggered","trigger":{"type":"BeginningOfStep","step":"Upkeep","whose":"You"},"effects":[{"type":"RemoveCounters","what":{"type":"This"},"kind":"Time","n":{"type":"Fixed","n":1}}]}"#,
                            r#"{"type":"Triggered","trigger":{"type":"CounterRemoved","filter":{"this":true},"kind":"Time","last":true},"effects":[{"type":"Sacrifice","who":{"type":"You"},"filter":{"this":true},"n":{"type":"Fixed","n":1}}]}"#,
                        ] {
                            if let Ok(a) = serde_json::from_str::<Ability>(js) {
                                self.add_effect(source, controller, &[id], Modif::Static(Box::new(StaticEffect::GrantAbility { affects: (Filter::default()).into(), ability: Box::new(a) })), Duration::Forever);
                            }
                        }
                    }
                    continue;
                }
                CopyExcept::AddCountersIfControlled { .. } | CopyExcept::SameCounters => continue,
                CopyExcept::SetLoyalty { amount } => {
                    let ec = crate::eval::EvalCtx::with_controller(source, controller);
                    let n = self.eval_amount(amount, &ec).max(0);
                    let have = self.cards[id.0 as usize].counter(CounterKind::Loyalty);
                    if have > 0 {
                        self.cards[id.0 as usize].remove_counters(CounterKind::Loyalty, have);
                    }
                    if n > 0 {
                        self.add_counters_to(Obj::card(id), CounterKind::Loyalty, n);
                    }
                    continue;
                }
                CopyExcept::AddCounters { kind, n, per_x, if_creature, if_planeswalker } => {
                    let copied_types = self.db.def(self.cards[id.0 as usize].def).types;
                    if (!*if_creature || copied_types.has(CardType::Creature)) && (!*if_planeswalker || copied_types.has(CardType::Planeswalker)) {
                        let n = if *per_x { *n * self.cards[source.0 as usize].x } else { *n };
                        self.add_counters_to(Obj::card(id), *kind, n);
                    }
                    continue;
                }
            };
            self.add_effect(source, controller, &[id], m, Duration::Forever);
        }
    }

    /// Counter the spell or ability `sid` on the stack (unless it can't be countered).
    pub(crate) fn counter_stack_item(&mut self, sid: StackId, exile: bool, by: PlayerId, by_src: CardId) {
        let Some(pos) = self.stack.iter().position(|s| s.id == sid) else { return };
        let item = self.stack[pos].clone();
        if (self.cards[item.source.0 as usize].cur.kw.has(Keyword::CantBeCountered) && self.cards[item.source.0 as usize].cast_alt != Some(crate::decision::FACE_DOWN_ALT)) || (self.def_of(item.source).keywords.has(Keyword::CantBeCountered) && self.cards[item.source.0 as usize].cast_alt != Some(crate::decision::FACE_DOWN_ALT)) || self.spell_cant_be_countered(item.source, Some(by_src)) {
            return;
        }
        self.stack.remove(pos);
        let exile_play = matches!(item.kind, ItemKind::Spell { .. }) && !item.copy && self.counter_exile_play(by);
        let exile = exile || exile_play;
        self.countered_by.push((item.id, item.controller));
        if matches!(item.kind, ItemKind::Spell { .. }) {
            self.spells_countered.push((item.source, item.controller, by));
            let marked = self.exile_marked.iter().position(|c| *c == item.source);
            if let Some(i) = marked {
                self.exile_marked.remove(i);
            }
            // flashback and similar alternative costs exile the spell if it leaves the stack any way but resolving
            let alt_exiles = match &item.kind {
                ItemKind::Spell { alt: Some(i), .. } => self
                    .def_of(item.source)
                    .abilities
                    .iter()
                    .filter_map(|a| if let Ability::AlternativeCost { exile_after, .. } = a { Some(*exile_after) } else { None })
                    .nth(*i as usize)
                    .unwrap_or(false)
                    || self.cards[item.source.0 as usize].cast_exile_after,
                // a spell cast through a permission that exiles it instead of the graveyard (Maestros Ascendancy)
                ItemKind::Spell { .. } => self.cards[item.source.0 as usize].cast_exile_after,
                _ => false,
            } || matches!(&item.kind, ItemKind::Spell { from: Zone::Graveyard, .. })
                && self.db.def(item.def).abilities.iter().any(|a| matches!(a, Ability::Replace { what: Replace::ZoneChange { to: Zone::Graveyard, instead: Zone::Exile, .. }, .. }));
            let dest = if exile || marked.is_some() || alt_exiles { Zone::Exile } else { Zone::Graveyard };
            if !item.copy {
                // the countered spell is off the stack list but still a stack card: its own replacements (Nexus of Fate) apply to the move
                let prev = self.resolving_spell.replace(Some(item.source));
                self.move_card(item.source, dest, Default::default());
                self.resolving_spell.set(prev);
            }
            self.emit(Event::Countered { card: item.source, by: Some(by) });
            if exile_play && self.cards[item.source.0 as usize].zone == Zone::Exile {
                self.counter_offer = Some((by, item.source));
            }
        }
    }

    /// Does `p` control a permanent with `StaticEffect::CounterExilePlay` (Guile)?
    pub(crate) fn counter_exile_play(&self, p: PlayerId) -> bool {
        self.battlefield.iter().any(|&id| {
            let c = &self.cards[id.0 as usize];
            if c.cur.controller != p || c.cur.no_abilities {
                return false;
            }
            let d = self.db.def(c.def);
            d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::CounterExilePlay, .. }))
        })
    }

    /// After a counter: ask whether to cast the exiled spell free (see `counter_exile_play`).
    fn counter_offer_ask(&mut self, ctx: &mut ResolveCtx, source: CardId) -> bool {
        let Some((player, card)) = self.counter_offer.take() else { return false };
        if self.cards[card.0 as usize].zone != Zone::Exile {
            return false;
        }
        ctx.sub = Some(Sub::FreeCast { player, card });
        self.pending = Some(Decision { player, kind: DecisionKind::YesNo { prompt: format!("Cast {} without paying its mana cost?", self.def_of(card).name), source: Some(source) } });
        true
    }

    /// The rooms a dungeon card's room leads to.
    pub(crate) fn room_next(&self, dungeon: CardId, room: &str) -> Vec<String> {
        for a in &self.db.def(self.cards[dungeon.0 as usize].def).abilities {
            if let Ability::Room { name, next } = a {
                if name == room {
                    return next.clone();
                }
            }
        }
        vec![]
    }

    /// Enter `name`: the first room of a new dungeon card (`new_dungeon`: `name` is the dungeon's name) or a room of the current one.
    pub(crate) fn venture_to(&mut self, p: PlayerId, name: &str, new_dungeon: bool) {
        let (dc, room) = if new_dungeon {
            let Some(def) = self.db.find(name) else {
                self.log(format!("unknown dungeon {name}"));
                return;
            };
            let id = self.create_card(def, p, Zone::Command, true);
            self.cards[id.0 as usize].controller = p;
            let first = self.db.def(def).abilities.iter().find_map(|a| if let Ability::Room { name, .. } = a { Some(name.clone()) } else { None });
            let Some(first) = first else { return };
            (id, first)
        } else {
            let Some((dc, _)) = self.players[p as usize].dungeon.clone() else { return };
            (dc, name.to_string())
        };
        self.players[p as usize].dungeon = Some((dc, room.clone()));
        self.layers_dirty = true;
        let last = self.room_next(dc, &room).is_empty();
        self.emit(Event::RoomEntered { player: p, dungeon: dc, name: room });
        if last {
            self.players[p as usize].dungeons_completed = self.players[p as usize].dungeons_completed.saturating_add(1);
            let dn = self.db.def(self.cards[dc.0 as usize].def).name.clone();
            self.players[p as usize].completed_dungeon_names.push(dn);
            self.emit(Event::DungeonCompleted { player: p });
            // a completed dungeon leaves the command zone (309.7)
            self.players[p as usize].command.retain(|c| *c != dc);
            self.players[p as usize].dungeon = None;
            self.layers_dirty = true;
        }
    }

    pub fn add_effect(&mut self, source: CardId, controller: PlayerId, cards: &[CardId], modif: Modif, duration: Duration) {
        if cards.is_empty() {
            return;
        }
        // "for as long as you control <source>": a source that already left the battlefield ends the duration before it begins
        if matches!(duration, Duration::WhileSourceControlled | Duration::WhileSourceOnBattlefield) && !self.battlefield.contains(&source) {
            return;
        }
        let ts = self.next_ts();
        let id = self.next_effect_id;
        self.next_effect_id += 1;
        let affects: SmallVec<[(CardId, u32); 4]> = cards.iter().map(|c| (*c, self.cards[c.0 as usize].zcc)).collect();
        let duration = if duration == Duration::Instant { Duration::UntilEndOfTurn } else { duration };
        self.effects.push(ContEffect { id, ts, source, controller, affects, players: SmallVec::new(), modif, duration, created_turn: self.turn });
        self.layers_dirty = true;
    }

    /// A static "can't transform" applies to this permanent.
    fn cant_transform(&self, id: CardId) -> bool {
        self.battlefield.iter().any(|&sid| {
            let sc = &self.cards[sid.0 as usize];
            if sc.cur.no_abilities {
                return false;
            }
            let sd = self.db.def(sc.def);
            sd.statics.iter().any(|&i| matches!(&sd.abilities[i as usize], Ability::Static { effect: StaticEffect::CantTransform { affects }, condition, .. }
                if self.matches_card(affects, id, sid, sc.cur.controller) && condition.as_ref().map_or(true, |cd| self.eval_cond(cd, &crate::eval::EvalCtx::with_controller(sid, sc.cur.controller)))))
        })
    }

    pub fn transform(&mut self, id: CardId) {
        if self.cards[id.0 as usize].zone == Zone::Battlefield && self.cant_transform(id) {
            return;
        }
        let c = &self.cards[id.0 as usize];
        let d = self.db.def(c.def);
        // 701.28c: a permanent that is not a transforming double-faced card (a modal DFC) doesn't transform
        if c.zone == Zone::Battlefield && d.other_face.is_some() && !d.transforming {
            return;
        }
        if let Some(other) = d.other_face {
            self.cards[id.0 as usize].def = other;
            if self.cards[id.0 as usize].zone == Zone::Battlefield {
                // an Aura face enchanting a player: leaving it ends the attachment; entering one that attaches on transform asks for the player
                self.cards[id.0 as usize].enchanted_player = None;
                if self.db.def(other).abilities.iter().any(|a| matches!(a, Ability::AttachPlayerOnTransform)) && !self.attach_pending.contains(&id) {
                    self.attach_pending.push(id);
                }
            }
            // a planeswalker back face that arrives without loyalty (returned transformed) starts with its printed loyalty
            let nd = self.db.def(other);
            if let (Some(l), true) = (nd.loyalty, nd.types.has(CardType::Planeswalker)) {
                if self.cards[id.0 as usize].zone == Zone::Battlefield && self.cards[id.0 as usize].counter(CounterKind::Loyalty) == 0 {
                    self.cards[id.0 as usize].add_counters(CounterKind::Loyalty, l);
                }
            }
            self.layers_dirty = true;
            // triggers match the permanent as it is now (Cult of the Waxing Moon: "non-Human creature transforms")
            let watched = self.battlefield.iter().any(|b| {
                let bd = self.db.def(self.cards[b.0 as usize].def);
                bd.abilities.iter().any(|a| matches!(a, Ability::Triggered { trigger: Trigger::PermanentTransforms { .. }, .. }))
            });
            // the transformed permanent's own "transforms" triggers (Ashling, Rekindled) test its new face in their conditions
            let own = self.db.def(other).abilities.iter().any(|a| matches!(a, Ability::Triggered { trigger: Trigger::Transforms, .. }));
            if watched || own {
                self.recompute_layers();
            }
            self.emit(Event::Transformed { card: id });
        }
    }

    /// Safety valve against exponential token engines (Scute Swarm): no more tokens once the battlefield is enormous.
    pub fn token_cap_reached(&self) -> bool {
        self.battlefield.len() >= 600
    }

    pub fn make_named_tokens(&mut self, name: &str, n: i32, p: PlayerId, ctx: &mut ResolveCtx) {
        let Some(def) = self.db.find(name) else {
            self.log(format!("unknown token {name}"));
            return;
        };
        let mut made = vec![];
        let (tname, n, extras, p) = self.replace_token_plan(p, name, self.db.def(def), n);
        let mut batches: Vec<(DefId, i32)> = vec![(self.db.find(&tname).unwrap_or(def), n)];
        for (en, cnt) in extras {
            if let Some(d2) = self.db.find(&en) {
                batches.push((d2, cnt));
            }
        }
        for (bdef, cnt) in batches {
            for _ in 0..cnt.max(0) {
                if self.token_cap_reached() {
                    break;
                }
                let id = self.create_card(bdef, p, Zone::Exile, true);
                self.players[p as usize].exile.retain(|x| *x != id);
                self.cards[id.0 as usize].zone = Zone::Library;
                self.move_card(id, Zone::Battlefield, MoveOpts { controller: Some(p), ..Default::default() });
                self.emit(Event::Token { card: id });
                made.push(Obj::card(id));
            }
        }
        ctx.remembered = made;
    }

    /// A `MayPay` cost with an `{X}` in its mana cost.
    pub(crate) fn may_pay_has_x(cost: &[Cost]) -> bool {
        cost.iter().any(|c| matches!(c, Cost::Mana { cost } if cost.contains("{X}")))
    }

    /// The `MayPay` cost with `{X}` replaced by the chosen amount.
    pub(crate) fn subst_may_pay_x(cost: &[Cost], k: i32) -> Vec<Cost> {
        cost.iter().map(|c| match c { Cost::Mana { cost } if cost.contains("{X}") => Cost::Mana { cost: cost.replace("{X}", &if k > 0 { format!("{{{k}}}") } else { String::new() }) }, o => o.clone() }).collect()
    }

    /// Can `p` pay all these costs right now (only costs without choices beyond picking cards)?
    /// A may-pay cost paid `k` times: `XTimes` parts are scaled (mana strings repeated), the rest stay as they are.
    pub(crate) fn scale_may_pay_cost(costs: &[Cost], k: i32) -> Vec<Cost> {
        let mut out = vec![];
        for c in costs {
            match c {
                Cost::XTimes { cost } => match &**cost {
                    Cost::Mana { cost: m } => out.push(Cost::Mana { cost: m.repeat(k.max(0) as usize) }),
                    inner => out.push(Self::scale_cost(inner, k)),
                },
                o => out.push(o.clone()),
            }
        }
        out
    }

    pub fn can_pay_costs(&self, p: PlayerId, source: CardId, costs: &[Cost]) -> bool {
        let mut mana = ManaCost::default();
        for c in costs {
            match c {
                Cost::Mana { cost } => {
                    if let Ok(m) = ManaCost::parse(cost) {
                        mana.shards.extend(m.shards);
                    }
                }
                Cost::ColoredManaAmount { symbol, amount } => {
                    let n = self.life_cost(source, amount);
                    if let Ok(m) = ManaCost::parse(&symbol.repeat(n.max(0) as usize)) {
                        mana.shards.extend(m.shards);
                    }
                }
                Cost::ManaAmount { amount } => {
                    let n = self.life_cost(source, amount);
                    if let Ok(m) = ManaCost::parse(&format!("{{{n}}}")) {
                        mana.shards.extend(m.shards);
                    }
                }
                Cost::PayLifeX { amount } => {
                    let n = self.life_cost(source, amount);
                    if self.players[p as usize].life < n {
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
                Cost::PayLife { n } => {
                    if self.players[p as usize].life < *n {
                        return false;
                    }
                }
                Cost::PayHalfLife => {
                    if !self.can_pay_life(p, (self.players[p as usize].life.max(0) + 1) / 2) {
                        return false;
                    }
                }
                Cost::Discard { n, filter, .. } => {
                    if self.eligible_in_hand(p, filter, source).len() < *n as usize {
                        return false;
                    }
                }
                Cost::Sacrifice { n, filter } => {
                    if self.eligible_permanents(p, filter, source).len() < *n as usize {
                        return false;
                    }
                }
                Cost::SacrificeSelf | Cost::PutCounters { .. } => {
                    // putting a counter on this permanent (fabricate) needs it still on the battlefield
                    if self.cards[source.0 as usize].zone != Zone::Battlefield {
                        return false;
                    }
                }
                Cost::PayEnergy { n } => {
                    if self.players[p as usize].energy < *n {
                        return false;
                    }
                }
                Cost::RemoveCounters { kind, n } => {
                    if self.cards[source.0 as usize].counter(*kind) < *n {
                        return false;
                    }
                }
                Cost::RemoveAnyCounters { n } => {
                    if self.cards[source.0 as usize].counters.iter().map(|(_, x)| *x as i32).sum::<i32>() < *n {
                        return false;
                    }
                }
                Cost::RemoveAnyCountersFrom { filter, n } => {
                    if !self.any_counters_removable(filter, *n, source, p) {
                        return false;
                    }
                }
                Cost::Tap => {
                    if self.cards[source.0 as usize].tapped {
                        return false;
                    }
                }
                Cost::TapPermanents { filter, n } => {
                    if self.tappable_for_cost(p, source, filter).len() < *n as usize {
                        return false;
                    }
                }
                Cost::UntapPermanents { filter, n, opponent } => {
                    if self.untappable_for_cost(p, source, filter, *opponent).len() < *n as usize {
                        return false;
                    }
                }
                Cost::ReturnToHand { filter, n } | Cost::ExileFromBattlefield { filter, n } => {
                    if self.eligible_permanents(p, filter, source).len() < *n as usize {
                        return false;
                    }
                }
                Cost::ExileFromSingleGraveyard { filter, n, .. } => {
                    if self.single_graveyard_pools(filter, source, p, *n).is_empty() {
                        return false;
                    }
                }
                Cost::ExileFromGraveyard { filter, n, bottom } => {
                    if self.eligible_in_graveyard(p, filter, source).len() < *n as usize {
                        return false;
                    }
                }
                Cost::CollectEvidence { n } => {
                    if self.evidence_cards(p, *n, source).is_none() {
                        return false;
                    }
                }
                Cost::ExileSelf => {
                    if self.cards[source.0 as usize].zone == Zone::Stack {
                        return false;
                    }
                }
                Cost::ExileFromHand { filter, n, .. } => {
                    if self.eligible_in_hand(p, filter, source).len() < *n as usize {
                        return false;
                    }
                }
                Cost::Reveal { filter, n } => {
                    if self.players[p as usize].hand.iter().filter(|h| (**h != source || filter.this) && self.matches_card(&Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() }, **h, source, p)).count() < *n as usize {
                        return false;
                    }
                }
                Cost::RemoveCountersFrom { filter, kind, n, among, suspended } => {
                    if !self.counters_removable_from(filter, *kind, *n, *among, *suspended, source, p) {
                        return false;
                    }
                }
                Cost::PutCountersOnChosen { filter, opponent, .. } => {
                    if self.counter_targets(p, filter, source, *opponent).is_empty() {
                        return false;
                    }
                }
                Cost::GainControlOfChosen { filter } => {
                    let need = costs.iter().filter(|c| matches!(c, Cost::GainControlOfChosen { .. })).count().max(1);
                    if self.control_targets(p, filter, source, &[]).len() < need {
                        return false;
                    }
                }
                Cost::Behold { filter, n, choose_type } => {
                    if !self.beholdable(filter, source, p, *n, *choose_type) {
                        return false;
                    }
                }
                Cost::Or { options } => {
                    if !options.iter().any(|o| self.can_pay_costs(p, source, o)) {
                        return false;
                    }
                }
                _ => {}
            }
        }
        mana.is_empty() || self.can_afford(p, &mana, 0)
    }

    /// Pay costs immediately using automatic choices (used for "unless pays" effects).
    pub fn try_pay_costs(&mut self, p: PlayerId, source: CardId, costs: &[Cost]) -> bool {
        if !self.can_pay_costs(p, source, costs) {
            return false;
        }
        for c in costs {
            match c {
                Cost::Mana { cost } => {
                    let Ok(m) = ManaCost::parse(cost) else { return false };
                    let mut rem = m;
                    self.spend_pool(p, &mut rem);
                    if !rem.is_empty() {
                        let pool = self.players[p as usize].pool;
                        let Some(plan) = self.plan_payment(p, &rem, 0, pool, true) else { return false };
                        for u in plan.uses {
                            match u {
                                crate::pay::PayUse::Source { card, ability, produce } => {
                                    let _ =self.activate_mana_ability(p, card, ability, Some(produce));
                                }
                                crate::pay::PayUse::Pool(t) => {
                                    if self.track_pay_spent {
                                        self.note_spent(t);
                                    }
                                    self.players[p as usize].pool.take(t);
                                }
                            }
                        }
                        self.lose_life(p, plan.life);
                    }
                }
                Cost::ControllerDraws { n } => {
                    let ctl = self.cards[source.0 as usize].cur.controller;
                    for _ in 0..*n {
                        self.draw_with_replacement(ctl, false, false);
                    }
                    self.flush_followups();
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
                Cost::ReturnToHand { filter, n } => {
                    let el = self.eligible_permanents(p, filter, source);
                    for id in el.into_iter().take(*n as usize) {
                        self.move_card(id, Zone::Hand, Default::default());
                    }
                }
                Cost::ExileFromBattlefield { filter, n } => {
                    let el = self.eligible_permanents(p, filter, source);
                    for id in el.into_iter().take(*n as usize) {
                        self.move_card(id, Zone::Exile, Default::default());
                        self.cost_sacrificed.push(id);
                    }
                }
                Cost::ExileGraveyard => {
                    for id in self.players[p as usize].graveyard.clone() {
                        self.move_card(id, Zone::Exile, Default::default());
                    }
                }
                Cost::ExileFromSingleGraveyard { filter, n, bottom } => {
                    // no way to ask here: the first graveyard with enough cards, an opponent's before your own
                    let mut pools = self.single_graveyard_pools(filter, source, p, *n);
                    pools.sort_by_key(|v| v.first().map_or(false, |c| self.cards[c.0 as usize].owner == p));
                    if let Some(pool) = pools.first() {
                        for id in pool.iter().copied().take(*n as usize) {
                            if *bottom {
                                self.move_card(id, Zone::Library, crate::zones::MoveOpts { bottom: true, ..Default::default() });
                            } else {
                                self.move_card(id, Zone::Exile, Default::default());
                            }
                            self.cost_sacrificed.push(id);
                        }
                    }
                }
                Cost::ExileFromGraveyard { filter, n, bottom } => {
                    // the payer's own picks (PayChoice::Graveyard) first, then the rest in order
                    let elig = self.eligible_in_graveyard(p, filter, source);
                    let picks: Vec<CardId> = std::mem::take(&mut self.mana_picks).into_iter().filter(|id| elig.contains(id)).take(*n as usize).collect();
                    let left = (*n as usize).saturating_sub(picks.len());
                    let rest: Vec<CardId> = elig.into_iter().filter(|id| !picks.contains(id)).take(left).collect();
                    for id in picks.into_iter().chain(rest) {
                        if *bottom {
                            self.move_card(id, Zone::Library, crate::zones::MoveOpts { bottom: true, ..Default::default() });
                        } else {
                            self.move_card(id, Zone::Exile, Default::default());
                        }
                        self.cost_sacrificed.push(id);
                    }
                }
                Cost::CollectEvidence { n } => {
                    if let Some(cards) = self.evidence_cards(p, *n, source) {
                        for id in cards {
                            self.move_card(id, Zone::Exile, Default::default());
                            self.cost_sacrificed.push(id);
                        }
                        self.emit(Event::EvidenceCollected { player: p });
                    }
                }
                Cost::ExileSelf => {
                    self.move_card(source, Zone::Exile, Default::default());
                }
                Cost::PutSelfOnLibraryBottom => {
                    self.move_card(source, Zone::Library, crate::zones::MoveOpts { bottom: true, ..Default::default() });
                }
                Cost::ExileFromHand { filter, n, .. } => {
                    for id in self.eligible_in_hand(p, filter, source).into_iter().take(*n as usize) {
                        self.move_card(id, Zone::Exile, Default::default());
                    }
                }
                Cost::SacrificeCards { cards } => {
                    for c in cards {
                        let id = CardId(*c);
                        if self.cards[id.0 as usize].zone == Zone::Battlefield {
                            self.emit(Event::Sacrificed { card: id, player: p });
                            self.cost_sacrificed.push(id);
                            self.move_card(id, Zone::Graveyard, Default::default());
                        }
                    }
                }
                Cost::DiscardCards { cards } => {
                    for c in cards {
                        let id = CardId(*c);
                        if self.cards[id.0 as usize].zone == Zone::Hand {
                            self.move_card(id, Zone::Graveyard, Default::default());
                            self.cost_sacrificed.push(id);
                            self.emit(Event::Discarded { card: id, player: p, by: self.cause_ctl.get() });
                        }
                    }
                }
                Cost::PutCountersOnCard { card, kind, n } => {
                    if self.cards[*card as usize].zone == Zone::Battlefield {
                        self.add_counters_to(Obj::card(CardId(*card)), *kind, *n);
                    }
                }
                Cost::GainControlOfCard { card } => {
                    let id = CardId(*card);
                    if self.cards[id.0 as usize].zone == Zone::Battlefield {
                        self.add_effect(source, p, &[id], Modif::Control(p), Duration::Forever);
                    }
                }
                Cost::GainControlOfChosen { filter } => {
                    // no way to ask here: the first matching permanent
                    if let Some(id) = self.control_targets(p, filter, source, &[]).first().copied() {
                        self.add_effect(source, p, &[id], Modif::Control(p), Duration::Forever);
                    }
                }
                Cost::PutCountersOnChosen { filter, kind, n, opponent } => {
                    // no way to ask here: the first matching permanent gets them
                    if let Some(id) = self.counter_targets(p, filter, source, *opponent).first().copied() {
                        self.add_counters_to(Obj::card(id), *kind, *n);
                    }
                }
                Cost::RemoveCountersFrom { filter, kind, n, among, suspended } => {
                    // no way to ask here: take them from the first permanent that has enough
                    let mut f = filter.clone();
                    f.controller = PlayerFilter::Any;
                    let pick = self.battlefield.iter().copied().find(|id| {
                        let c = &self.cards[id.0 as usize];
                        c.cur.controller == p && c.counter(*kind) >= *n && self.matches_card(&f, *id, source, p)
                    });
                    if let Some(id) = pick {
                        self.remove_counters_from(Obj::card(id), *kind, *n);
                    }
                }
                Cost::Or { options } => {
                    // no way to ask here: pay with the first alternative that can be paid
                    if let Some(o) = options.iter().find(|o| self.can_pay_costs(p, source, o)) {
                        let o = o.clone();
                        self.try_pay_costs(p, source, &o);
                    }
                }
                Cost::PayLife { n } => {
                    self.pay_life(p, *n);
                }
                Cost::PayHalfLife => {
                    let n = (self.players[p as usize].life.max(0) + 1) / 2;
                    self.pay_life(p, n);
                }
                Cost::PayLifeX { amount } => {
                    let n = self.life_cost(source, amount);
                    self.pay_life(p, n);
                }
                Cost::TakeDamage { n } => {
                    self.deal_damage(Some(source), Obj::player(p), *n, false);
                }
                Cost::SacrificeAll { filter, .. } => {
                    let mut f = filter.clone();
                    f.controller = PlayerFilter::Any;
                    let all: Vec<CardId> = self.battlefield.iter().copied().filter(|id| self.cards[id.0 as usize].cur.controller == p && self.matches_card(&f, *id, source, p)).collect();
                    for id in all {
                        self.emit(Event::Sacrificed { card: id, player: p });
                        self.cost_sacrificed.push(id);
                        self.move_card(id, Zone::Graveyard, Default::default());
                    }
                }
                Cost::ExileSpellFromStack { filter } => {
                    // an Unless cost ("that player may exile it"): the spell the resolving source targets, else the topmost match
                    let pool = self.stack_spells_for_cost(p, source, &filter.clone().into_filter());
                    let targeted: Vec<Obj> = self.stack.iter().find(|s| s.source == source).map(|s| s.targets.iter().map(|t| t.obj).collect()).unwrap_or_default();
                    let pick = pool.iter().copied().find(|c| targeted.iter().any(|t| matches!(t, Obj::Stack { id } if self.stack.iter().any(|s| s.id == *id && s.source == *c)) || *t == Obj::card(*c))).or(pool.first().copied());
                    match pick {
                        Some(id) => {
                            self.stack.retain(|s| !(s.source == id && matches!(s.kind, ItemKind::Spell { .. }) && !s.copy));
                            self.exile_marked.retain(|c| *c != id);
                            self.cur_src = Some(source);
                            self.move_card(id, Zone::Exile, Default::default());
                        }
                        None => return false,
                    }
                }
                Cost::DrawCards { n } => {
                    // paying by drawing (Psychic Vortex's cumulative upkeep): drawing from an empty library is still attempted
                    for _ in 0..*n {
                        self.draw_with_replacement(p, false, false);
                    }
                    self.flush_followups();
                }
                Cost::DiscardHand => {
                    let hand = self.players[p as usize].hand.clone();
                    for id in hand {
                        self.move_card(id, Zone::Graveyard, Default::default());
                        self.cost_sacrificed.push(id);
                        self.emit(Event::Discarded { card: id, player: p, by: self.cause_ctl.get() });
                    }
                }
                Cost::Discard { n, filter, random } => {
                    let mut el = self.eligible_in_hand(p, filter, source);
                    if *random {
                        el.shuffle(&mut self.rng);
                    }
                    for id in el.into_iter().take(*n as usize) {
                        self.move_card(id, Zone::Graveyard, Default::default());
                        self.cost_sacrificed.push(id);
                        self.emit(Event::Discarded { card: id, player: p, by: self.cause_ctl.get() });
                    }
                }
                Cost::Sacrifice { n, filter } => {
                    let el = self.eligible_permanents(p, filter, source);
                    for id in el.into_iter().take(*n as usize) {
                        self.emit(Event::Sacrificed { card: id, player: p });
                        self.cost_sacrificed.push(id);
                        self.move_card(id, Zone::Graveyard, Default::default());
                    }
                }
                Cost::PutCounters { kind, n } => self.add_counters_to(Obj::card(source), *kind, *n),
                Cost::RemoveCounters { kind, n } => self.remove_counters_from(Obj::card(source), *kind, *n),
                Cost::RemoveAnyCounters { n } => self.remove_any_counters(source, *n),
                Cost::RemoveAnyCountersFrom { filter, n } => self.remove_any_counters_among(filter, *n, source, p),
                Cost::AddMana { color, n } => {
                    self.players[p as usize].pool.add(crate::mana::ManaType::of(*color), (*n).max(0) as u16);
                }
                Cost::PayEnergy { n } => {
                    self.players[p as usize].energy -= *n;
                    self.players[p as usize].energy_spent += *n;
                }
                Cost::Tap => self.tap_card(source),
                Cost::TapPermanents { filter, n } => {
                    let el = self.tappable_for_cost(p, source, filter);
                    for id in el.into_iter().take(*n as usize) {
                        self.tap_card(id);
                        self.cost_sacrificed.push(id);
                    }
                }
                Cost::UntapPermanents { filter, n, opponent } => {
                    let el = self.untappable_for_cost(p, source, filter, *opponent);
                    for id in el.into_iter().take(*n as usize) {
                        self.cards[id.0 as usize].tapped = false;
                        self.emit(Event::Untapped { card: id });
                    }
                }
                Cost::SacrificeSelf => {
                    self.emit(Event::Sacrificed { card: source, player: p });
                    self.move_card(source, Zone::Graveyard, Default::default());
                }
                Cost::GetPoison { n } => {
                    let k = self.replace_player_counters(p, CounterKind::Poison, *n);
                    self.players[p as usize].poison += k;
                }
                Cost::FlipCoins { n } => {
                    for _ in 0..*n {
                        self.flip_coin_for(p);
                    }
                }
                _ => {}
            }
        }
        true
    }

    /// One coin flip by `flipper` (replacement statics, tallies and the `CoinFlipped` event); true when it is won.
    pub fn flip_coin_for(&mut self, flipper: PlayerId) -> bool {
        use rand::Rng;
        let rigged = self.first_flip_heads(flipper);
        if self.players[flipper as usize].coin_flip_turn != self.turn {
            self.players[flipper as usize].coin_flip_serial = self.flip_serial;
        }
        self.players[flipper as usize].coin_flip_turn = self.turn;
        let mut heads: bool = rigged || self.rng.gen_bool(0.5);
        if self.flips_coin_twice(flipper) {
            let second: bool = self.rng.gen_bool(0.5);
            heads = heads || second;
        }
        self.last_flip_won = heads;
        self.flip_count += 1;
        if heads { self.flip_wins += 1 } else { self.flip_lost = true }
        self.log(if heads { "The coin flip is won" } else { "The coin flip is lost" });
        self.emit(Event::CoinFlipped { player: flipper, won: heads });
        heads
    }

    /// Shuffle helper for deterministic card selection.
    pub fn pick_random<T: Copy>(&mut self, v: &[T]) -> Option<T> {
        v.choose(&mut self.rng).copied()
    }
}

/// Does the oracle text have a triggered ability with an intervening "if" ("When ..., if ...,", "At the beginning of ..., if ...,")?
fn has_intervening_if(text: &str) -> bool {
    text.lines().any(|l| {
        let l = l.rsplit(" — ").next().unwrap_or(l);
        let l = l.trim_start();
        (l.starts_with("When ") || l.starts_with("Whenever ") || l.starts_with("At "))
            && l.find(", if ").map_or(false, |i| !l[..i].contains(", ") || l[..i].matches(',').count() <= 1)
    })
}


/// Number of target slots each chosen mode of a modal spell has (None when the spell isn't modal).
fn d_modes_spec_counts(db: &Arc<crate::carddb::CardDb>, item: &StackItem) -> Option<Vec<u8>> {
    let d = db.def(item.def);
    if let ItemKind::Ability { ability, .. } | ItemKind::Triggered { ability, .. } = &item.kind {
        // modes chosen as the ability went on the stack
        if let Some(Ability::Triggered { effects, .. }) | Some(Ability::Activated { effects, .. }) = d.abilities.get(*ability as usize) {
            if let Some(Effect::ChooseModes { modes, at_stack: true, .. }) = effects.first() {
                return Some(
                    item.modes
                        .iter()
                        .map(|&m| match modes[m as usize].effects.first() {
                            Some(Effect::Reflexive { targets, untargeted: false, .. }) => targets.len() as u8,
                            _ => modes[m as usize].targets.len() as u8,
                        })
                        .collect(),
                );
            }
        }
    }
    for a in d.abilities.iter() {
        if let Ability::Modal { modes, .. } = a {
            return Some(item.modes.iter().map(|&m| modes[m as usize].targets.len() as u8).collect());
        }
    }
    None
}


/// Replace every `X` amount in these effects by the fixed value `x`.
fn bake_x_effects(effects: &[Effect], x: i32) -> Vec<Effect> {
    fn bake(v: &mut serde_json::Value, x: i32) {
        match v {
            serde_json::Value::Object(m) => {
                if m.len() == 1 && m.get("type").and_then(|t| t.as_str()) == Some("X") {
                    m.insert("type".into(), "Fixed".into());
                    m.insert("n".into(), x.into());
                } else {
                    for (_, c) in m.iter_mut() {
                        bake(c, x);
                    }
                }
            }
            serde_json::Value::Array(a) => a.iter_mut().for_each(|c| bake(c, x)),
            _ => {}
        }
    }
    let Ok(mut v) = serde_json::to_value(effects) else { return effects.to_vec() };
    bake(&mut v, x);
    serde_json::from_value(v).unwrap_or_else(|_| effects.to_vec())
}

impl Game {
    /// What a "you may" question is about, shown under it: the printed line of the ability being resolved when it can be
    /// found, else the source's whole rules text when it is short (a spell, or a card whose one ability is it).
    pub(crate) fn may_text(&self, source: CardId, kind: &ItemKind) -> Option<String> {
        let d = self.def_of(source);
        let ability = match kind {
            ItemKind::Triggered { ability, .. } | ItemKind::Ability { ability, .. } if *ability < crate::cast::GRANTED_BASE => Some(*ability as usize),
            _ => None,
        };
        let flat = |t: &str| t.split_whitespace().collect::<Vec<_>>().join(" ");
        if let Some(i) = ability {
            let line = crate::view::printed_ability_text(d, i);
            if !line.is_empty() && flat(&d.oracle_text).contains(&line) {
                return Some(line);
            }
        }
        let all = flat(&d.oracle_text);
        (!all.is_empty()).then(|| crate::view::short_text(&all, 700))
    }
}
