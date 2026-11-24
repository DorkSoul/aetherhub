//! Triggered abilities: events are matched against the triggers of permanents; matches wait in
//! `pending_triggers` until a player would receive priority, then go on the stack.

use crate::dsl::*;
use crate::eval::*;
use crate::ids::*;
use crate::state::*;
use crate::types::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum EventKind {
    Moved = 0,
    Damage,
    Step,
    Cast,
    Attack,
    Block,
    Tap,
    Untap,
    Draw,
    LifeGain,
    LifeLoss,
    CounterAdd,
    Token,
    Attach,
    Transform,
    Control,
    PlayerLost,
    LandPlayed,
    Shuffle,
    Monarch,
    Discard,
    Sacrifice,
    Monstrous,
    Scry,
    State,
    Cycle,
    Crime,
    Unlock,
    Phase,
    DayNight,
    Activate,
    Saddle,
    Target,
    ManaTap,
    Clash,
    Vote,
    Misc,
    Expend,
    Countered,
    Ring,
    Room,
}

/// Bits of every event kind a trigger listens to (`Trigger::Either` listens to both sides).
pub fn trigger_kind_mask(t: &Trigger) -> u64 {
    match t {
        Trigger::Either { a, b } => trigger_kind_mask(a) | trigger_kind_mask(b),
        _ => 1 << event_kind_of(t) as u64,
    }
}

/// Does the trigger listen to events of this kind?
pub fn trigger_listens(t: &Trigger, kind: EventKind) -> bool {
    trigger_kind_mask(t) & (1 << kind as u64) != 0
}

pub fn event_kind_of(t: &Trigger) -> EventKind {
    use Trigger::*;
    match t {
        Either { a, .. } => event_kind_of(a),
        EntersBattlefield { .. } | Dies { .. } | LeavesBattlefield { .. } | ChangesZone { .. } | Landfall => EventKind::Moved,
        Attacks { .. } | AttacksAlone { .. } | AttacksDifferentPlayers { .. } | YouAttack { .. } | PlayerAttacksOpponent { .. } => EventKind::Attack,
        Blocks { .. } | BecomesBlocked { .. } | AttackerUnblocked { .. } => EventKind::Block,
        CommitsCrime { .. } => EventKind::Crime,
        DayNightChanged { .. } => EventKind::DayNight,
        UnlocksDoor { .. } | FullyUnlocksRoom { .. } => EventKind::Unlock,
        PhasesOut { .. } | PhasesIn { .. } => EventKind::Phase,
        DealsCombatDamage { .. } | DealsDamage { .. } | IsDealtDamage { .. } | CombatDamageToPlayer { .. } | DamageToChosenPlayer { .. } => EventKind::Damage,
        BeginningOfStep { .. } | FirstUpkeepOfGame | EndOfCombat => EventKind::Step,
        SpellCast { .. } | SpellCopied { .. } | SpellCastTargets { .. } | SpellCastTeamwork { .. } => EventKind::Cast,
        SpellCountered { .. } => EventKind::Countered,
        BecomesRenowned { .. } | Mutates { .. } | BecomesPlotted { .. } | BecomesForetold { .. } | Exerted { .. } | CaseSolved { .. } => EventKind::Monstrous,
        SagaFinalChapter { .. } => EventKind::Misc,
        FacesDilemma { .. } => EventKind::Misc,
        Regenerated { .. } => EventKind::Misc,
        Scried { .. } => EventKind::Scry,
        BecomesTapped { .. } => EventKind::Tap,
        BecomesUntapped { .. } => EventKind::Untap,
        BecomesTarget { .. } | PlayerBecomesTarget { .. } | YouOrPermanentBecomesTarget { .. } => EventKind::Target,
        StackItemPut { .. } => EventKind::Misc,
        Draws { .. } => EventKind::Draw,
        Discards { .. } => EventKind::Discard,
        GainsLife { .. } => EventKind::LifeGain,
        LosesLife { .. } => EventKind::LifeLoss,
        CounterAdded { .. } | CounterRemoved { .. } | SagaChapter { .. } | SagaFinalChapterTriggers { .. } => EventKind::CounterAdd,
        Sacrificed { .. } => EventKind::Sacrifice,
        Attached => EventKind::Attach,
        AttachedToThis { .. } | AttachedToPermanent { .. } => EventKind::Attach,
        Transforms | PermanentTransforms { .. } | TurnedFaceUp { .. } => EventKind::Transform,
        LosesControl { .. } | GainsControl { .. } => EventKind::Control,
        LandPlayed { .. } => EventKind::LandPlayed,
        Shuffles { .. } => EventKind::Shuffle,
        BecomesMonarch | PlayerBecomesMonarch { .. } => EventKind::Monarch,
        Cycled { .. } => EventKind::Cycle,
        AbilityActivated { .. } => EventKind::Activate,
        AbilityTriggeredByEntering { .. } | AbilityTriggeredByAttacking { .. } => EventKind::Misc,
        SearchesLibrary { .. } => EventKind::Misc,
        BecomesSaddled { .. } => EventKind::Saddle,
        TapsForMana { .. } => EventKind::ManaTap,
        TokenCreated { .. } => EventKind::Token,
        PlayerDies { .. } => EventKind::PlayerLost,
        Exploited | ExploitedBy { .. } => EventKind::Sacrifice,
        WinsClash { .. } | Clashes { .. } => EventKind::Clash,
        VotingFinished => EventKind::Vote,
        KeywordActionDone { .. } | Discovers { .. } | Investigates { .. } | CollectsEvidence { .. } | PaysLife { .. } | Explores { .. } | Connives { .. } | GainsEnergy { .. } | PlayerCounterAdded { .. } | Milled { .. } | ManifestsDread { .. } | Bends { .. } | RollsDie { .. } | RollsChaos | RollsBlank | RollsPlanarDie | PlaneswalkTo | PlaneswalkFrom | SetInMotion | AnotherSchemeSetInMotion { .. } | GiftGiven { .. } | FlipsCoin { .. } | Proliferates { .. } | BecomesCrewed | Crews { .. } | BecomesUnattached => EventKind::Misc,
        Expend { .. } => EventKind::Expend,
        RingTempts { .. } | RingBearerChosen { .. } => EventKind::Ring,
        TakesInitiative { .. } => EventKind::Room,
        EntersRoom { .. } | CompletesDungeon { .. } => EventKind::Room,
        BecomesMonstrous { .. } => EventKind::Monstrous,
        State { .. } => EventKind::State,
    }
}

fn kind_of_event(e: &Event) -> Option<EventKind> {
    Some(match e {
        Event::Moved { .. } => EventKind::Moved,
        Event::Damage { .. } => EventKind::Damage,
        Event::StepBegin { .. } => EventKind::Step,
        Event::Cast { .. } | Event::Copied { .. } => EventKind::Cast,
        Event::Countered { .. } => EventKind::Countered,
        Event::Attacked { .. } => EventKind::Attack,
        Event::Blocked { .. } | Event::Unblocked { .. } | Event::BecameBlocked { .. } => EventKind::Block,
        Event::Tapped { .. } => EventKind::Tap,
        Event::Regenerated { .. } => EventKind::Misc,
        Event::Untapped { .. } => EventKind::Untap,
        Event::Drew { .. } => EventKind::Draw,
        Event::Life { delta, .. } => {
            if *delta > 0 {
                EventKind::LifeGain
            } else {
                EventKind::LifeLoss
            }
        }
        Event::Counters { .. } => EventKind::CounterAdd,
        Event::Token { .. } => EventKind::Token,
        Event::Attach { .. } => EventKind::Attach,
        Event::Transformed { .. } | Event::EnteredTransformed { .. } | Event::TurnedFaceUp { .. } => EventKind::Transform,
        Event::ControlLost { .. } => EventKind::Control,
        Event::Lost { .. } => EventKind::PlayerLost,
        Event::LandPlayed { .. } => EventKind::LandPlayed,
        Event::Shuffled { .. } => EventKind::Shuffle,
        Event::Monarch { .. } => EventKind::Monarch,
        Event::Discarded { .. } => EventKind::Discard,
        Event::Sacrificed { .. } | Event::Exploited { .. } => EventKind::Sacrifice,
        Event::Monstrous { .. } | Event::Renowned { .. } | Event::Mutated { .. } | Event::Plotted { .. } | Event::Foretold { .. } | Event::Exerted { .. } | Event::Solved { .. } => EventKind::Monstrous,
        Event::Scry { .. } => EventKind::Scry,
        Event::Clashed { .. } => EventKind::Clash,
        Event::VotingFinished { .. } => EventKind::Vote,
        Event::Bent { .. } | Event::Discovered { .. } | Event::KeywordAction { .. } | Event::Investigated { .. } | Event::EvidenceCollected { .. } | Event::LifePaid { .. } | Event::Explored { .. } | Event::Connived { .. } | Event::EnergyGained { .. } | Event::PlayerCounters { .. } | Event::DieRolled { .. } | Event::DiceRolled { .. } | Event::PlanarDieRolled { .. } | Event::ChaosEnsued { .. } | Event::PlaneswalkingAway { .. } | Event::Planeswalked { .. } | Event::SchemeSetInMotion { .. } | Event::ChoiceRevealed { .. } | Event::GiftGiven { .. } | Event::Milled { .. } | Event::ManifestedDread { .. } | Event::CoinFlipped { .. } | Event::Proliferated { .. } | Event::Crewed { .. } | Event::CrewPaid { .. } | Event::Searched { .. } | Event::Unattached { .. } | Event::SagaFinalChapter { .. } | Event::FacedDilemma { .. } => EventKind::Misc,
        Event::Expended { .. } => EventKind::Expend,
        Event::RingTempted { .. } | Event::RingBearerChosen { .. } => EventKind::Ring,
        Event::RoomEntered { .. } | Event::DungeonCompleted { .. } | Event::InitiativeTaken { .. } => EventKind::Room,
        Event::Cycled { .. } => EventKind::Cycle,
        Event::Activated { .. } => EventKind::Activate,
        Event::AbilityTriggered { .. } | Event::StackPut { .. } => EventKind::Misc,
        Event::Crime { .. } => EventKind::Crime,
        Event::DayNightChanged { .. } => EventKind::DayNight,
        Event::Unlocked { .. } | Event::FullyUnlocked { .. } => EventKind::Unlock,
        Event::PhasedOut { .. } | Event::PhasedIn { .. } => EventKind::Phase,
        Event::Saddled { .. } => EventKind::Saddle,
        Event::ManaTapped { .. } => EventKind::ManaTap,
        Event::Targeted { .. } | Event::PlayerTargeted { .. } => EventKind::Target,
        _ => return None,
    })
}

fn subject_card(e: &Event) -> Option<CardId> {
    match e {
        Event::Moved { card, .. }
        | Event::Cast { card, .. }
        | Event::Attacked { card, .. }
        | Event::Tapped { card }
        | Event::Regenerated { card }
        | Event::Untapped { card }
        | Event::Token { card }
        | Event::Attach { card, .. }
        | Event::Transformed { card }
        | Event::EnteredTransformed { card }
        | Event::ControlLost { card, .. }
        | Event::TurnedFaceUp { card }
        | Event::LandPlayed { card, .. }
        | Event::Discarded { card, .. }
        | Event::Cycled { card, .. }
        | Event::Activated { card, .. }
        | Event::Saddled { card }
        | Event::PhasedOut { card }
        | Event::Unlocked { card, .. }
        | Event::FullyUnlocked { card }
        | Event::PhasedIn { card }
        | Event::ManaTapped { card, .. }
        | Event::Targeted { card, .. }
        | Event::Sacrificed { card, .. }
        | Event::Monstrous { card, .. }
        | Event::Mutated { card }
        | Event::Plotted { card }
        | Event::Foretold { card }
        | Event::Exerted { card }
        | Event::Solved { card }
        | Event::SagaFinalChapter { card }
        | Event::Renowned { card }
        | Event::Counters { card, .. } => Some(*card),
        Event::Blocked { blocker, .. } => Some(*blocker),
        Event::Unblocked { card } => Some(*card),
        Event::BecameBlocked { attacker } => Some(*attacker),
        Event::Damage { source, .. } => *source,
        _ => None,
    }
}

pub fn on_event(g: &mut Game, e: &Event) {
    on_event_for(g, e, None)
}

/// Like `on_event`, but only `only`'s own abilities are considered (a Clone's copied enters-the-battlefield triggers).
pub fn on_event_for(g: &mut Game, e: &Event, only: Option<CardId>) {
    on_event_for_inner(g, e, only);
    // simultaneous moves to a graveyard (state-based actions): a card that functions in the graveyard also sees the cards that went there just before it in the
    // same batch (Centaur Vinecrasher dying with a land), so replay those earlier moves to its own abilities
    if only.is_none() && g.batch_active {
        if let Event::Moved { card, to: Zone::Graveyard, .. } = e {
            let card = *card;
            let earlier: Vec<Event> = g.batch_gy_events.clone();
            if !earlier.is_empty() && g.cards[card.0 as usize].zone == Zone::Graveyard {
                g.gy_replay = true;
                for ev in &earlier {
                    on_event_for_inner(g, ev, Some(card));
                }
                g.gy_replay = false;
            }
            g.batch_gy_events.push(e.clone());
        }
    }
}

fn on_event_for_inner(g: &mut Game, e: &Event, only: Option<CardId>) {
    let Some(kind) = kind_of_event(e) else { return };
    let bit = 1u64 << kind as u64;
    let db = g.db.clone();
    let mut candidates: Vec<CardId> = match only {
        Some(c) => vec![c],
        None => g.battlefield.clone(),
    };
    if let Some(c) = subject_card(e) {
        if !candidates.contains(&c) {
            candidates.push(c);
        }
    }
    // an exploit creature that sacrificed itself is already in the graveyard
    if let Event::Exploited { card, victim: Some(v) } = e {
        if card == v && !candidates.contains(card) {
            candidates.push(*card);
        }
    }
    // abilities that watch from the graveyard / hand / exile (Bloodghast)
    for pl in g.players.iter().filter(|_| only.is_none()) {
        for &c in pl.graveyard.iter().chain(pl.hand.iter()).chain(pl.exile.iter()).chain(pl.command.iter()) {
            let d = db.def(g.cards[c.0 as usize].def);
            if d.zone_triggers && d.trigger_mask & bit != 0 && !candidates.contains(&c) {
                candidates.push(c);
            }
        }
    }
    // permanents that left the battlefield in the same batch still see this departure
    if let Event::Moved { from: Zone::Battlefield, card: m, .. } = e {
        for (c, _) in g.batch_left.clone() {
            if c != *m && !candidates.contains(&c) {
                candidates.push(c);
            }
        }
    }
    // an Equipment that left the battlefield while attached still sees its own unattach (Grafted Exoskeleton)
    if let Event::Unattached { card, .. } = e {
        if !candidates.contains(card) {
            candidates.push(*card);
        }
    }
    // damage events also concern the damaged permanent
    if let Event::Damage { target: Obj::Card { id }, .. } = e {
        if !candidates.contains(id) {
            candidates.push(*id);
        }
    }
    let mut found: Vec<PendingTrigger> = vec![];
    let mut once_marks: Vec<(CardId, u16)> = vec![];
    let mut ever_marks: Vec<(CardId, u16)> = vec![];
    let mut immediate: Vec<(CardId, PlayerId, u16, EventCtx)> = vec![];
    if g.granted_only {
        candidates.clear();
    }
    for src in candidates {
        let card = &g.cards[src.0 as usize];
        // a permanent that just left the battlefield triggers as it last was (a transformed double-faced card keeps its back face)
        let leaving_def = match (e, g.lki_slot.as_ref()) {
            (Event::Moved { card: m, from: Zone::Battlefield, .. }, Some(l)) if *m == src && l.card == src => Some(l.def),
            _ => None,
        };
        let card_def = leaving_def.unwrap_or(card.def);
        let def = db.def(card_def);
        if def.trigger_mask & bit == 0 {
            continue;
        }
        for &ai in &def.triggers {
            let Ability::Triggered { trigger, condition, from, once_per_turn, limit_per_turn, once_ever, batched, .. } = &def.abilities[ai as usize] else { continue };
            if !trigger_listens(trigger, kind) {
                continue;
            }
            // is the source in a zone where this ability is active?
            let need = from.unwrap_or(Zone::Battlefield);
            if g.gy_replay && (need != Zone::Graveyard || !matches!(trigger, Trigger::ChangesZone { .. })) {
                continue;
            }
            // an Aura's "when enchanted creature dies" trigger from the graveyard (Screams from Within, Takklemaggot): the Aura is still on the battlefield, just unattached, when the
            // creature dies and follows into the graveyard in the same round of state-based actions, before the trigger goes on the stack (the trigger's filter picks the host)
            let aura_follows_host = need == Zone::Graveyard && card.zone == Zone::Battlefield && matches!(e, Event::Moved { from: Zone::Battlefield, to: Zone::Graveyard, .. }) && card.attached_to.is_none() && card.cur.subtypes.contains(&Sym::new("Aura"));
            let in_zone = card.zone == need || aura_follows_host;
            let batch_ctrl = g.batch_left.iter().find(|(c, _)| *c == src).map(|(_, p)| *p);
            let leaves_look_back = (matches!(e, Event::Moved { card: m, from: Zone::Battlefield, .. } if *m == src)
                || (batch_ctrl.is_some() && matches!(e, Event::Moved { from: Zone::Battlefield, .. })))
                && need == Zone::Battlefield;
            // "whenever land cards are put into your graveyard from anywhere" (Sand Scout) is not a leaves-the-battlefield ability:
            // a source that goes to the graveyard in the same batch as the cards does not see them
            let leaves_look_back = leaves_look_back
                && !(*batched && card.zone != Zone::Battlefield && matches!(trigger, Trigger::ChangesZone { from: None, to: Some(Zone::Graveyard), .. }) && !matches!(e, Event::Moved { card: m, .. } if *m == src));
            let cast_self = matches!(e, Event::Cast { card: m, .. } if *m == src) && need == Zone::Stack;
            // "when this card is put into a graveyard from your hand / the stack / your library" (Hostility, Blightsteel Colossus): the card has already moved on when the event is seen
            let zone_exit_self = matches!(need, Zone::Stack | Zone::Hand | Zone::Library) && matches!(e, Event::Moved { card: m, from, .. } if *m == src && *from == need);
            // a creature with exploit that sacrificed itself still "exploits a creature" (it has left the battlefield by then)
            let exploit_self = matches!(e, Event::Exploited { card: m, victim: Some(v) } if *m == src && *v == src);
            // an Equipment destroyed while attached is unattached as it leaves (Grafted Exoskeleton): the event is seen after the move
            let unattach_self = need == Zone::Battlefield && matches!(e, Event::Unattached { card: m, .. } if *m == src);
            if !(in_zone || leaves_look_back || cast_self || zone_exit_self || exploit_self || unattach_self) {
                continue;
            }
            // a graveyard ability doesn't see another permanent dying at the same time the source itself died (Syrix, Carrier of the Flame)
            if need == Zone::Graveyard && card.zone == Zone::Graveyard && batch_ctrl.is_some() && !matches!(trigger, Trigger::ChangesZone { to: Some(Zone::Graveyard), .. }) && matches!(e, Event::Moved { card: m, from: Zone::Battlefield, .. } if *m != src) {
                continue;
            }
            if card.cur.no_abilities && card.zone == Zone::Battlefield {
                continue;
            }
            if matches!(e, Event::Moved { card: m, to: Zone::Battlefield, .. } if *m == src) && g.bare_entered.borrow().iter().any(|&(c, z)| c == src && z == card.zcc) {
                continue;
            }
            // Yixlid Jailer: cards in graveyards have no abilities, so graveyard triggers (and "when put into a graveyard from ...") don't happen
            if g.graveyard_abilities_off()
                && ((in_zone && need == Zone::Graveyard && card.zone == Zone::Graveyard)
                    || (zone_exit_self && matches!(e, Event::Moved { to: Zone::Graveyard, .. })))
            {
                continue;
            }
            if !in_zone && batch_ctrl.is_some() && g.batch_noabil.contains(&src) {
                continue;
            }
            // a permanent that had lost all its abilities when it left doesn't see its own leaves-the-battlefield triggers
            if leaves_look_back && !in_zone && matches!(e, Event::Moved { card: m, .. } if *m == src) && g.lki_slot.as_ref().map_or(false, |l| l.card == src && l.derived.no_abilities) {
                continue;
            }
            let ctrl = if let (Event::ControlLost { card: m, from, to }, true) = (e, card.zone == Zone::Battlefield) {
                if *m == src && matches!(trigger, Trigger::GainsControl { .. }) { *to } else if *m == src { *from } else { card.cur.controller }
            } else if card.zone == Zone::Battlefield {
                card.cur.controller
            } else if leaves_look_back {
                match batch_ctrl {
                    Some(p) if !matches!(e, Event::Moved { card: m, .. } if *m == src) => p,
                    _ => g.lki_slot.as_ref().map_or(card.owner, |l| l.controller),
                }
            } else {
                card.owner
            };
            if let Some(mut ctx) = match_trigger(g, trigger, e, src, ctrl) {
                if let Some(cond) = condition {
                    let ec = EvalCtx { source: src, controller: ctrl, x: g.cards[src.0 as usize].x, targets: &[], tidx: &[], event: &ctx, remembered: &[] };
                    // filters inside the condition (same_name_as TriggerObject ...) read the event
                    *g.filter_event.borrow_mut() = Some(ctx.clone());
                    let ok = g.eval_cond(cond, &ec);
                    *g.filter_event.borrow_mut() = None;
                    if !ok {
                        continue;
                    }
                }
                if matches!(trigger, Trigger::BecomesBlocked { batched: true, .. }) {
                    let key = (src, ai | 0x2000, 255);
                    if g.batch_seen.contains(&key) {
                        continue;
                    }
                    g.batch_seen.push(key);
                }
                if let (Trigger::PlayerAttacksOpponent { .. }, Some(att), Some(Obj::Player { id: def })) = (trigger, ctx.player, ctx.obj) {
                    // one trigger per attacking player and attacked opponent for the whole declaration
                    let key = (src, ai | 0x1000 | ((att as u16) << 8), def);
                    if g.batch_seen.contains(&key) {
                        continue;
                    }
                    g.batch_seen.push(key);
                }
                if matches!(trigger, Trigger::YouAttack { .. }) {
                    // one trigger per attack declaration
                    let key = (src, ai | 0x4000, 255);
                    if g.batch_seen.contains(&key) {
                        continue;
                    }
                    g.batch_seen.push(key);
                }
                if let (Trigger::Attacks { batched: true, one_per_declaration, .. }, Some(pl)) = (trigger, ctx.player) {
                    // one trigger per attacked player / planeswalker / battle for the whole declaration; the attackers join the batch's `EventCards`
                    let pw = if *one_per_declaration { None } else { match (ctx.obj, g.combat.as_ref()) {
                        (Some(Obj::Card { id: a }), Some(cb)) => cb.attackers.iter().find(|x| x.card == a).and_then(|x| match x.defender {
                            Obj::Card { id } => Some(id),
                            _ => None,
                        }),
                        _ => None,
                    } };
                    // `one_per_declaration`: attackers at different defending players still make one trigger ("whenever you attack")
                    let key = (src, ai | 0x8000, if *one_per_declaration { 255 } else { pl });
                    let seen = match pw {
                        Some(pw) => g.batch_seen_pw.contains(&(src, ai, pw)),
                        None => g.batch_seen.contains(&key),
                    };
                    // the attacker's power when it attacked (last known information for "their total power", Vulpine Harvester)
                    let pw_power = match ctx.obj { Some(Obj::Card { id }) => g.power_of(id), _ => 0 };
                    if seen {
                        if let (Some(Obj::Card { id }), Some(pt)) = (ctx.obj, g.pending_triggers.iter_mut().find(|t| t.source == src && t.ability == ai && (*one_per_declaration || t.ctx.player == Some(pl)) && t.ctx.damaged == pw)) {
                            if !pt.ctx.cards.contains(&id) {
                                pt.ctx.cards.push(id);
                                pt.ctx.batch_power += pw_power;
                                pt.ctx.card_powers.push((id, pw_power));
                            }
                        }
                        continue;
                    }
                    match pw {
                        Some(pw) => g.batch_seen_pw.push((src, ai, pw)),
                        None => g.batch_seen.push(key),
                    }
                    ctx.damaged = pw;
                    if let Some(Obj::Card { id }) = ctx.obj {
                        ctx.cards.push(id);
                        ctx.batch_power += pw_power;
                        ctx.card_powers.push((id, pw_power));
                    }
                }
                if let (Trigger::IsDealtDamage { batched: true, .. }, Event::Damage { combat: true, .. }, Some(dmgd)) = (trigger, e, ctx.damaged) {
                    // simultaneous combat damage to the same permanent: one trigger, the amounts add up
                    if g.batch_seen_pw.contains(&(src, ai, dmgd)) {
                        if let Some(pt) = g.pending_triggers.iter_mut().find(|t| t.source == src && t.ability == ai && t.ctx.damaged == Some(dmgd)) {
                            pt.ctx.amount += ctx.amount;
                        }
                        continue;
                    }
                    g.batch_seen_pw.push((src, ai, dmgd));
                }
                if let (Trigger::DealsDamage { opponents_count: true, .. }, Event::Damage { combat: false, .. }) = (trigger, e) {
                    ctx.amount = 1;
                }
                if let (Trigger::CombatDamageToPlayer { opponents_count: true, .. } | Trigger::DealsDamage { opponents_count: true, .. }, Event::Damage { combat: true, .. }, Some(pl)) = (trigger, e, ctx.player) {
                    // one trigger for the whole damage step; EventAmount counts the different opponents dealt damage
                    if g.batch_seen.contains(&(src, ai | 0x1000, pl)) {
                        continue;
                    }
                    g.batch_seen.push((src, ai | 0x1000, pl));
                    if g.batch_seen.contains(&(src, ai | 0x1000, 255)) {
                        if let Some(pt) = g.pending_triggers.iter_mut().find(|t| t.source == src && t.ability == ai) {
                            pt.ctx.amount += 1;
                        }
                        continue;
                    }
                    g.batch_seen.push((src, ai | 0x1000, 255));
                    ctx.amount = 1;
                }
                if let (Trigger::CombatDamageToPlayer { batched: true, opponents_count: false, per_controller, .. }, Some(pl)) = (trigger, ctx.player) {
                    let ctl_of = |o: Option<Obj>| if let Some(Obj::Card { id }) = o { g.cards[id.0 as usize].cur.controller } else { 255 };
                    let bk = if *per_controller { pl.wrapping_add(ctl_of(ctx.obj).wrapping_mul(8)).wrapping_add(16) } else { pl };
                    if g.batch_seen.contains(&(src, ai, bk)) {
                        // the same simultaneous damage: add it to the trigger already waiting
                        if let Some(pt) = g.pending_triggers.iter_mut().find(|t| t.source == src && t.ability == ai && t.ctx.player == Some(pl) && (!*per_controller || ctl_of(t.ctx.cards.first().map(|c| Obj::card(*c))) == ctl_of(ctx.obj))) {
                            pt.ctx.amount += ctx.amount;
                            if let Some(Obj::Card { id }) = ctx.obj {
                                if !pt.ctx.cards.contains(&id) {
                                    pt.ctx.cards.push(id);
                                }
                            }
                        }
                        continue;
                    }
                    g.batch_seen.push((src, ai, bk));
                    if let Some(Obj::Card { id }) = ctx.obj {
                        ctx.cards.push(id);
                    }
                }
                // "deals damage to one or more ...": damage one source deals at the same time (several targets of one effect, or one
                // combat damage step) folds into the trigger already waiting; EventAmount is the total (Justice)
                if *batched && matches!(trigger, Trigger::DealsDamage { per_source: true, .. } | Trigger::DealsCombatDamage { to: DamageTo::Any, .. }) {
                    if let Some(pt) = g.pending_triggers.iter_mut().find(|t| t.source == src && t.ability == ai && t.ctx.source == ctx.source && g.zone_batch.contains(&(src, ai))) {
                        pt.ctx.amount += ctx.amount;
                        continue;
                    }
                    g.zone_batch.push((src, ai));
                } else if *batched {
                    if g.zone_batch.contains(&(src, ai)) {
                        // "one or more cards discarded": EventAmount is how many (like batched combat damage, the later events fold into the waiting trigger)
                        if let Some(pt) = g.pending_triggers.iter_mut().find(|t| t.source == src && t.ability == ai) {
                            if let (Trigger::CounterAdded { .. }, Some(k)) = (trigger, ctx.counter_kind) {
                                if !pt.ctx.counter_kinds.contains(&k) {
                                    pt.ctx.counter_kinds.push(k);
                                }
                            }
                            if matches!(trigger, Trigger::Discards { .. }) {
                                pt.ctx.amount += ctx.amount;
                            } else if matches!(trigger, Trigger::ChangesZone { .. }) && !pt.ctx.cards.iter().any(|c| Some(Obj::card(*c)) == ctx.obj) {
                                pt.ctx.amount += 1;
                                pt.ctx.batch_power += ctx.lki.as_ref().map_or(0, |l| l.derived.power);
                            }
                            // the later events of the batch join the set the trigger sees (`Sel::EventCards`)
                            if let Some(Obj::Card { id }) = ctx.obj {
                                if !pt.ctx.cards.contains(&id) {
                                    pt.ctx.cards.push(id);
                                }
                            }
                        }
                        continue;
                    }
                    g.zone_batch.push((src, ai));
                    if matches!(trigger, Trigger::ChangesZone { .. }) && ctx.amount == 0 {
                        ctx.amount = 1;
                    }
                    if matches!(trigger, Trigger::ChangesZone { .. }) {
                        ctx.batch_power = ctx.lki.as_ref().map_or(0, |l| l.derived.power);
                    }
                    if ctx.cards.is_empty() {
                        if let Some(Obj::Card { id }) = ctx.obj {
                            ctx.cards.push(id);
                        }
                    }
                }
                if *once_ever {
                    if card.trig_ever.contains(&ai) || ever_marks.contains(&(src, ai)) {
                        continue;
                    }
                    ever_marks.push((src, ai));
                }
                if *once_per_turn {
                    if card.activations.iter().any(|(i, _)| *i == ai) || once_marks.contains(&(src, ai)) {
                        continue;
                    }
                    once_marks.push((src, ai));
                }
                if *limit_per_turn > 0 {
                    let optional = matches!(&def.abilities[ai as usize], Ability::Triggered { optional: true, .. });
                    let used = card.activations.iter().find(|(i, _)| *i == ai).map_or(0, |(_, n)| *n as usize) + if optional { 0 } else { once_marks.iter().filter(|m| **m == (src, ai)).count() };
                    if used >= *limit_per_turn as usize {
                        continue;
                    }
                    // an optional limited trigger ("you may ... Do this only once each turn") uses the limit only when it is accepted on resolution
                    if !optional {
                        once_marks.push((src, ai));
                    }
                }
                if matches!(trigger, Trigger::TapsForMana { on_stack: false, .. }) {
                    immediate.push((src, ctrl, ai, ctx));
                    continue;
                }
                let ts = g.ts;
                let copies = if matches!(trigger, Trigger::CounterAdded { per_counter: true, .. }) { ctx.amount.max(1) } else { 1 };
                for _ in 1..copies {
                    found.push(PendingTrigger { controller: ctrl, source: src, src_zcc: g.cards[src.0 as usize].zcc, def: card_def, ability: ai, ctx: ctx.clone(), ts, granted: None });
                }
                found.push(PendingTrigger { controller: ctrl, source: src, src_zcc: g.cards[src.0 as usize].zcc, def: card_def, ability: ai, ctx, ts, granted: None });
            }
        }
    }
    // triggered mana abilities don't use the stack
    for (src, ctrl, ai, ectx) in immediate {
        if crate::coverage::ON.load(std::sync::atomic::Ordering::Relaxed) {
            crate::coverage::record(g.cards[src.0 as usize].def, ai);
        }
        let effects = match db.def(g.cards[src.0 as usize].def).abilities.get(ai as usize) {
            Some(Ability::Triggered { effects, .. }) => effects.clone(),
            _ => continue,
        };
        for eff in &effects {
            g.run_effect_with(src, ctrl, eff, ectx.clone());
        }
    }
    for (c, ai) in once_marks {
        let acts = &mut g.cards[c.0 as usize].activations;
        match acts.iter_mut().find(|(i, _)| *i == ai) {
            Some((_, n)) => *n = n.saturating_add(1),
            None => acts.push((ai, 1)),
        }
    }
    for (c, ai) in ever_marks {
        g.cards[c.0 as usize].trig_ever.push(ai);
    }
    // triggered abilities granted by other permanents (Cayth: other creatures have fabricate)
    let mut immediate_granted: Vec<(CardId, PlayerId, Vec<Effect>, EventCtx)> = vec![];
    let mut granted_srcs: Vec<CardId> = g.battlefield.clone();
    if let Event::Moved { card: m, from: Zone::Battlefield, .. } = e {
        if g.cards[m.0 as usize].zone != Zone::Battlefield {
            granted_srcs.push(*m);
        }
    }
    // a triggered ability granted to a card in hand / graveyard / exile (Molecule Man: miracle) watches events about that card
    if g.layers_dirty && matches!(e, Event::Drew { .. }) {
        g.recompute_layers();
    }
    if let Some(sc) = subject_card(e).or(match e {
        Event::Drew { card, .. } => Some(*card),
        _ => None,
    }) {
        let c = &g.cards[sc.0 as usize];
        if c.zone != Zone::Battlefield && !c.cur.granted.is_empty() && !granted_srcs.contains(&sc) {
            granted_srcs.push(sc);
        }
    }
    // a triggered ability granted to an exiled card (Uvilda's refine counters) watches the steps of the game
    if matches!(e, Event::StepBegin { .. }) && only.is_none() {
        for pl in g.players.iter() {
            for &c in pl.exile.iter() {
                if !g.cards[c.0 as usize].cur.granted.is_empty() && !granted_srcs.contains(&c) {
                    granted_srcs.push(c);
                }
            }
        }
    }
    if g.granted_only {
        granted_srcs = only.into_iter().collect();
    }
    if g.gy_replay {
        granted_srcs.clear();
    }
    for src in granted_srcs {
        let card = &g.cards[src.0 as usize];
        if card.cur.granted.is_empty() || (card.cur.no_abilities && !card.face_down) {
            continue;
        }
        let ctrl = if card.zone == Zone::Battlefield { card.cur.controller } else { g.lki_slot.as_ref().map_or(card.owner, |l| l.controller) };
        let granted: Vec<std::sync::Arc<Ability>> = card.cur.granted.iter().cloned().collect();
        let granted_by: Vec<CardId> = card.cur.granted_by.iter().copied().collect();
        for (k, a) in granted.iter().enumerate() {
            let Ability::Triggered { trigger, condition, once_per_turn, limit_per_turn, effects: granted_effects, from: gfrom, .. } = &**a else { continue };
            if let Some(z) = gfrom {
                if matches!(z, Zone::Hand | Zone::Graveyard | Zone::Exile) && g.cards[src.0 as usize].zone != *z {
                    continue;
                }
            }
            if !trigger_listens(trigger, kind) {
                continue;
            }
            if let Some(mut ctx) = match_trigger(g, trigger, e, src, ctrl) {
                ctx.granted_by = granted_by.get(k).copied();
                if let Some(cond) = condition {
                    let ec = EvalCtx { source: src, controller: ctrl, x: g.cards[src.0 as usize].x, targets: &[], tidx: &[], event: &ctx, remembered: &[] };
                    if !g.eval_cond(cond, &ec) {
                        continue;
                    }
                }
                // simultaneous combat damage to the same permanent triggers a granted ability once (Fungus Sliver)
                if let (Trigger::IsDealtDamage { batched: true, .. }, Event::Damage { combat: true, .. }, Some(dmgd)) = (trigger, e, ctx.damaged) {
                    let idx = crate::cast::GRANTED_BASE + k as u16;
                    if g.batch_seen_pw.contains(&(src, idx, dmgd)) {
                        if let Some(pt) = g.pending_triggers.iter_mut().find(|t| t.source == src && t.ability == idx && t.ctx.damaged == Some(dmgd)) {
                            pt.ctx.amount += ctx.amount;
                        }
                        continue;
                    }
                    g.batch_seen_pw.push((src, idx, dmgd));
                }
                // a granted triggered mana ability does not use the stack either
                if matches!(trigger, Trigger::TapsForMana { on_stack: false, .. }) {
                    immediate_granted.push((src, ctrl, granted_effects.clone(), ctx));
                    continue;
                }
                if *once_per_turn {
                    let idx = crate::cast::GRANTED_BASE + k as u16;
                    if g.cards[src.0 as usize].activations.iter().any(|(i, _)| *i == idx) {
                        continue;
                    }
                    g.cards[src.0 as usize].activations.push((idx, 1));
                }
                if *limit_per_turn > 0 {
                    let idx = crate::cast::GRANTED_BASE + k as u16;
                    let acts = &mut g.cards[src.0 as usize].activations;
                    match acts.iter_mut().find(|(i, _)| *i == idx) {
                        Some((_, n)) if *n >= *limit_per_turn => continue,
                        Some((_, n)) => *n += 1,
                        None => acts.push((idx, 1)),
                    }
                }
                let ts = g.ts;
                found.push(PendingTrigger { controller: ctrl, source: src, src_zcc: g.cards[src.0 as usize].zcc, def: g.cards[src.0 as usize].def, ability: crate::cast::GRANTED_BASE + k as u16, ctx, ts, granted: Some(a.clone()) });
            }
        }
    }
    for (src, ctrl, effects, ectx) in immediate_granted {
        for eff in &effects {
            g.run_effect_with(src, ctrl, eff, ectx.clone());
        }
    }
    // temporary triggered abilities (Effect::AddTrigger)
    if !g.temp_triggers.is_empty() && !g.gy_replay {
        let temps: Vec<(Vec<CardId>, Trigger, DelayedTrigger, bool, Option<u16>, Option<Condition>)> = g.temp_triggers.iter().filter(|t| trigger_listens(&t.trigger, kind)).map(|t| (t.bound.clone(), t.trigger.clone(), t.d.clone(), t.once, t.ability, t.cond.clone())).collect();
        let mut temp_pending: Vec<PendingTrigger> = vec![];
        let mut fired_once: Vec<u32> = vec![];
        let mut items: Vec<DelayedTrigger> = vec![];
        let mut immediate_temp: Vec<DelayedTrigger> = vec![];
        let mut once_step_src: Vec<CardId> = vec![];
        for (bound, trig, d, once, tab, tcond) in temps {
            *g.filter_targets.borrow_mut() = (d.targets.clone(), (0..d.targets.len() as u8).collect());
            let matched = match_trigger(g, &trig, e, d.source, d.controller);
            *g.filter_targets.borrow_mut() = (vec![], vec![]);
            if let Some(ctx) = matched {
                if g.temp_triggers.iter().any(|t| t.d.id == d.id && t.until == TempUntil::EndOfCarriedPlayersNextTurn) {
                    let owner = d.remembered.first().and_then(|o| if let Obj::Player { id } = o { Some(*id) } else { None });
                    if ctx.player != owner {
                        continue;
                    }
                }
                if !bound.is_empty() {
                    let about = |o: Option<Obj>| matches!(o, Some(Obj::Card { id }) if bound.contains(&id));
                    // "whenever it is dealt damage" is about the damaged permanent, not the damage source
                    let ok = if matches!(trig, Trigger::IsDealtDamage { .. }) {
                        ctx.damaged.map_or(false, |s| bound.contains(&s))
                    } else {
                        about(ctx.obj) || ctx.source.map_or(false, |s| bound.contains(&s))
                    };
                    if !ok {
                        continue;
                    }
                }
                // "deal combat damage to a player" (one or more): simultaneous damage triggers a temporary ability once per player
                if let (Trigger::CombatDamageToPlayer { batched: true, .. }, Some(pl)) = (&trig, ctx.player) {
                    let key = (d.source, 0x2000 | (d.id as u16 & 0xfff), pl);
                    if g.batch_seen.contains(&key) {
                        continue;
                    }
                    g.batch_seen.push(key);
                }
                if let Some(cd) = &tcond {
                    if !g.eval_cond(cd, &EvalCtx::with_controller(d.source, d.controller)) {
                        continue;
                    }
                }
                // one-shot "at the beginning of that step" abilities added by separate resolutions of the same source each wait for their own step (two Moraug landfalls: one untap per additional combat)
                if once && matches!(trig, Trigger::BeginningOfStep { step: Step::BeginCombat, .. }) {
                    if once_step_src.contains(&d.source) {
                        continue;
                    }
                    once_step_src.push(d.source);
                }
                if let Some(ai) = tab {
                    let ts = g.ts;
                    temp_pending.push(PendingTrigger { controller: d.controller, source: d.source, src_zcc: g.cards[d.source.0 as usize].zcc, def: g.cards[d.source.0 as usize].def, ability: ai, ctx, ts, granted: None });
                    if once {
                        fired_once.push(d.id);
                    }
                    continue;
                }
                let mut d2 = d.clone();
                d2.ctx = ctx;
                // a temporary triggered mana ability does not use the stack either (A Display of My Dark Power)
                if matches!(trig, Trigger::TapsForMana { on_stack: false, .. }) {
                    immediate_temp.push(d2);
                    continue;
                }
                items.push(d2);
                if once {
                    fired_once.push(d.id);
                }
            }
        }
        g.temp_triggers.retain(|t| !fired_once.contains(&t.d.id));
        g.pending_triggers.extend(temp_pending);
        for d in items {
            g.push_delayed_item(d);
        }
        for d in immediate_temp {
            for eff in &d.effects {
                g.run_effect_with(d.source, d.controller, eff, d.ctx.clone());
            }
        }
    }
    if let Event::Moved { to: Zone::Battlefield, .. } = e {
        for t in found.iter_mut() {
            t.ctx.entering = true;
        }
    }
    if let Event::Attacked { card, .. } = e {
        for t in found.iter_mut() {
            t.ctx.attacking = t.source == *card;
        }
    }
    // Torpor Orb-style statics: triggers caused by a permanent entering / dying don't happen
    if !found.is_empty() {
        let supp: Vec<(CardId, PlayerId, TriggerCause, FilterBox, PlayerFilter)> = g
            .battlefield
            .iter()
            .flat_map(|&sid| {
                let c = &g.cards[sid.0 as usize];
                let d = g.db.def(c.def);
                let on = !c.cur.no_abilities;
                d.statics
                    .iter()
                    .filter_map(move |&i| match &d.abilities[i as usize] {
                        Ability::Static { effect: StaticEffect::SuppressTriggers { event, cause, whose }, .. } if on => Some((sid, c.cur.controller, *event, cause.clone(), *whose)),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        if !supp.is_empty() {
            if let Event::Moved { from, to, .. } = e {
                let ev = if *to == Zone::Battlefield {
                    Some(TriggerCause::EntersBattlefield)
                } else if *from == Zone::Battlefield && *to == Zone::Graveyard {
                    Some(TriggerCause::Dies)
                } else {
                    None
                };
                if let Some(ev) = ev {
                    found.retain(|t| {
                        let Some(Obj::Card { id }) = t.ctx.obj else { return true };
                        // "as this enters" abilities (devour) are replacement effects, not triggers: nothing suppresses them
                        if matches!(g.db.def(t.def).abilities.get(t.ability as usize), Some(Ability::Triggered { as_enters: true, .. })) {
                            return true;
                        }
                        !supp.iter().any(|(sid, sctl, sev, flt, whose)| {
                            *sev == ev && g.player_filter(*whose, t.controller, *sctl) && obj_matches(g, flt, id, *sid, *sctl, g.lki_slot.as_ref().filter(|l| l.card == id))
                        })
                    });
                }
            }
        }
    }
    // Panharmonicon-style effects: the trigger happens additional times
    if g.any_replacements() {
        let mut extra: Vec<PendingTrigger> = vec![];
        for t in &found {
            let cause = match (t.ctx.obj, e) {
                (Some(Obj::Card { id }), Event::Moved { from, to, .. }) => {
                    let ev = if *to == Zone::Battlefield {
                        Some(TriggerCause::EntersBattlefield)
                    } else if *from == Zone::Battlefield && *to == Zone::Graveyard {
                        Some(TriggerCause::Dies)
                    } else if *from == Zone::Battlefield {
                        Some(TriggerCause::LeavesBattlefield)
                    } else {
                        None
                    };
                    ev.map(|ev| (id, ev))
                }
                (Some(Obj::Card { id }), Event::Attacked { .. }) => Some((id, TriggerCause::Attacks)),
                (Some(Obj::Card { id }), Event::TurnedFaceUp { .. }) => Some((id, TriggerCause::TurnedFaceUp)),
                (_, Event::RoomEntered { dungeon, .. }) => Some((*dungeon, TriggerCause::EntersRoom)),
                (_, Event::Damage { source: Some(s), target: Obj::Player { .. }, combat: true, .. }) => Some((*s, TriggerCause::CombatDamageToPlayer)),
                (_, Event::Damage { target: Obj::Card { id }, amount, .. }) if *amount > 0 => Some((*id, TriggerCause::CreatureDealtDamage)),
                (_, Event::Drew { .. }) => Some((t.source, TriggerCause::Draws)),
                (_, Event::Cast { card, player } | Event::Copied { card, player })
                    if *player == t.controller && (g.def_of(*card).types.has(CardType::Instant) || g.def_of(*card).types.has(CardType::Sorcery)) =>
                {
                    Some((t.source, TriggerCause::CastOrCopyInstantSorcery))
                }
                (_, Event::Life { delta, player }) if *delta > 0 && *player == t.controller => Some((t.source, TriggerCause::GainsLife)),
                _ => None,
            };
            if let Some((id, ev)) = cause {
                for _ in 0..g.trigger_extra(id, ev, t.controller, t.source) {
                    extra.push(t.clone());
                }
            }
            for _ in 0..g.trigger_extra(t.source, TriggerCause::AnyTrigger, t.controller, t.source) {
                extra.push(t.clone());
            }
        }
        found.extend(extra);
    }
    g.pending_triggers.extend(found);
}

fn obj_matches(g: &Game, f: &Filter, id: CardId, src: CardId, ctrl: PlayerId, lki: Option<&Lki>) -> bool {
    match lki {
        Some(l) if l.card == id => g.matches_view(f, &g.view_lki(l), src, ctrl),
        _ => g.matches_card(f, id, src, ctrl),
    }
}

/// `obj_matches` with the trigger's own event visible to the filter (`PowerOf TriggerObject` inside `power_vs` ...).
fn obj_matches_ev(g: &Game, f: &Filter, id: CardId, src: CardId, ctrl: PlayerId, ev: &EventCtx) -> bool {
    *g.filter_event.borrow_mut() = Some(ev.clone());
    let r = obj_matches(g, f, id, src, ctrl, None);
    *g.filter_event.borrow_mut() = None;
    r
}

/// `obj_matches` with the trigger's event visible to the filter, with last-known information for the object.
fn obj_matches_ev_lki(g: &Game, f: &Filter, id: CardId, src: CardId, ctrl: PlayerId, ev: &EventCtx, lki: Option<&Lki>) -> bool {
    *g.filter_event.borrow_mut() = Some(ev.clone());
    let r = obj_matches(g, f, id, src, ctrl, lki);
    *g.filter_event.borrow_mut() = None;
    r
}

fn dmg_to_ok(g: &Game, to: DamageTo, target: &Obj, ctrl: PlayerId, src: CardId) -> bool {
    match (to, target) {
        (DamageTo::Any, _) => true,
        (DamageTo::Player, Obj::Player { .. }) => true,
        (DamageTo::PlayerOrBattle, Obj::Player { .. }) => true,
        (DamageTo::PlayerOrBattle, Obj::Card { id }) => g.cards[id.0 as usize].cur.types.has(CardType::Battle),
        (DamageTo::You, Obj::Player { id }) => *id == ctrl,
        (DamageTo::YouOrYourPermanent, Obj::Player { id }) => *id == ctrl,
        (DamageTo::YouOrYourPermanent, Obj::Card { id }) => g.cards[id.0 as usize].zone == Zone::Battlefield && g.cards[id.0 as usize].cur.controller == ctrl,
        (DamageTo::YouOrYourPlaneswalker, Obj::Player { id }) => *id == ctrl,
        (DamageTo::YouOrYourPlaneswalker, Obj::Card { id }) => {
            let c = &g.cards[id.0 as usize];
            c.zone == Zone::Battlefield && c.cur.controller == ctrl && c.cur.types.has(CardType::Planeswalker)
        }
        (DamageTo::Opponent, Obj::Player { id }) => *id != ctrl,
        (DamageTo::EnchantedPlayer, Obj::Player { id }) => g.cards[src.0 as usize].enchanted_player == Some(*id),
        (DamageTo::ChosenPlayer, Obj::Player { id }) => g.cards[src.0 as usize].chosen_player == Some(*id),
        (DamageTo::Creature, Obj::Card { id }) => g.cards[id.0 as usize].cur.types.has(CardType::Creature),
        (DamageTo::Planeswalker, Obj::Card { id }) => g.cards[id.0 as usize].cur.types.has(CardType::Planeswalker),
        (DamageTo::Battle, Obj::Card { id }) => g.cards[id.0 as usize].cur.types.has(CardType::Battle),
        _ => false,
    }
}

fn match_trigger(g: &Game, t: &Trigger, e: &Event, src: CardId, ctrl: PlayerId) -> Option<EventCtx> {
    let lki = g.lki_slot.as_ref();
    let ctx = |obj: Option<Obj>, player: Option<PlayerId>, amount: i32| EventCtx { obj, source: None, player, amount, dmg_combat: false, batch_power: 0, card_powers: Vec::new(), lki: lki.map(|l| Box::new(l.clone())), damaged: None, granted_by: None, counter_kind: None, counter_kinds: Vec::new(), cards: Vec::new(), entering: false, attacking: false, stack: None, mana_colors: 0, ts: 0, this_zcc: None };
    let with_src = |mut c: EventCtx, s: Option<CardId>| {
        c.source = s;
        c
    };
    if let Trigger::Either { a, b } = t {
        return match_trigger(g, a, e, src, ctrl).or_else(|| match_trigger(g, b, e, src, ctrl));
    }
    match (t, e) {
        (Trigger::EntersBattlefield { filter }, Event::Moved { card, to: Zone::Battlefield, .. }) => {
            let c = ctx(Some(Obj::card(*card)), Some(g.cards[card.0 as usize].cur.controller), 0);
            obj_matches_ev_lki(g, filter, *card, src, ctrl, &c, None).then(|| c)
        }
        (Trigger::Landfall, Event::Moved { card, to: Zone::Battlefield, .. }) => {
            let c = &g.cards[card.0 as usize];
            (c.cur.types.has(CardType::Land) && c.cur.controller == ctrl).then(|| ctx(Some(Obj::card(*card)), None, 0))
        }
        (Trigger::Dies { filter }, Event::Moved { card, from: Zone::Battlefield, to: Zone::Graveyard }) => {
            // the trigger's player is the controller of the creature that died (last known)
            let dead_ctrl = lki.map_or(g.cards[card.0 as usize].controller, |l| l.controller);
            let c = ctx(Some(Obj::card(*card)), Some(dead_ctrl), 0);
            obj_matches_ev_lki(g, filter, *card, src, ctrl, &c, lki).then(|| c)
        }
        (Trigger::LeavesBattlefield { filter }, Event::Moved { card, from: Zone::Battlefield, to }) if *to != Zone::Battlefield => {
            let c = ctx(Some(Obj::card(*card)), None, 0);
            obj_matches_ev_lki(g, filter, *card, src, ctrl, &c, lki).then(|| c)
        }
        (Trigger::ChangesZone { filter, from, to, not_to, also_from, cause_opponent, cause_you, destroyed, not_sacrificed, after_resolving, craft_cost }, Event::Moved { card, from: f, to: tt }) => {
            if *craft_cost && !g.craft_exile.get() {
                return None;
            }
            if *after_resolving && !(g.resolved_ok.get() && g.resolving_spell.get() == Some(*card)) {
                return None;
            }
            if from.map_or(false, |z| z != *f && !also_from.contains(f)) || to.map_or(false, |z| z != *tt) || not_to.map_or(false, |z| z == *tt) {
                return None;
            }
            let cause = g.cause_ctl.get();
            if (*cause_opponent && !cause.map_or(false, |c| c != ctrl)) || (*cause_you && cause != Some(ctrl)) || (*destroyed && !g.cause_destroy.get()) {
                return None;
            }
            if *not_sacrificed {
                let z = g.cards[card.0 as usize].zcc;
                if g.last_sacrificed.get().map_or(false, |(c, k)| c == *card && (k == z || k + 1 == z)) {
                    return None;
                }
            }
            // "a creature card is put into a graveyard": the card's own types decide, not what the permanent was while animated (a Genju-animated land is a land card)
            if *tt == Zone::Graveyard && *f == Zone::Battlefield && to.is_some() {
                let ft = filter.clone().into_filter();
                if !ft.types.is_empty() && !g.cards[card.0 as usize].token && g.db.def(g.cards[src.0 as usize].def).oracle_text.contains(" card") && !ft.types.iter().any(|t| g.db.def(g.cards[card.0 as usize].def).types.has(*t)) {
                    return None;
                }
            }
            obj_matches(g, filter, *card, src, ctrl, lki).then(|| ctx(Some(Obj::card(*card)), if *cause_opponent { cause } else { None }, 0))
        }
        (Trigger::Attacks { filter, at_you, at_you_or_pw, at_enchanted, player_only, at_opponent, pw_only, .. }, Event::Attacked { card, defender }) => {
            let mut ok = (!*pw_only || matches!(defender, Obj::Card { id } if g.cards[id.0 as usize].cur.controller == ctrl)) && (!*at_you_or_pw || match defender { Obj::Player { id } => *id == ctrl, Obj::Card { id } => g.cards[id.0 as usize].cur.controller == ctrl, _ => false }) && (!*player_only || matches!(defender, Obj::Player { .. })) && (!*at_you || matches!(defender, Obj::Player { id } if *id == ctrl));
            if *at_opponent {
                let dp = match defender {
                    Obj::Player { id } => Some(*id),
                    Obj::Card { id } => Some(g.cards[id.0 as usize].cur.controller),
                    _ => None,
                };
                ok = ok && dp.map_or(false, |d| d != ctrl);
            }
            if *at_enchanted {
                let ep = g.cards[src.0 as usize].enchanted_player;
                let dp = match defender {
                    Obj::Player { id } => Some(*id),
                    Obj::Card { id } => Some(g.cards[id.0 as usize].cur.controller),
                    _ => None,
                };
                ok = ok && ep.is_some() && ep == dp && matches!(defender, Obj::Player { .. });
            }
            (ok && obj_matches(g, filter, *card, src, ctrl, None)).then(|| {
                let dp = match defender {
                    Obj::Player { id } => Some(*id),
                    Obj::Card { id } => Some(g.cards[id.0 as usize].cur.controller),
                    _ => None,
                };
                with_src(ctx(Some(Obj::card(*card)), dp, 0), Some(*card))
            })
        }
        (Trigger::YouAttack { who, min, max, filter, players_only, min_total_power }, Event::Attacked { card, defender }) => {
            let attacker_ctl = g.cards[card.0 as usize].cur.controller;
            let n = g.combat.as_ref().map_or(0, |c| c.attackers.iter().filter(|a| (!*players_only || matches!(a.defender, Obj::Player { .. })) && g.matches_card(&Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() }, a.card, src, ctrl)).count());
            if *players_only && !matches!(defender, Obj::Player { .. }) {
                return None;
            }
            if !g.matches_card(&Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() }, *card, src, ctrl) {
                return None;
            }
            if *min_total_power > 0 {
                let total: i32 = g.combat.as_ref().map_or(0, |c| c.attackers.iter().filter(|a| (!*players_only || matches!(a.defender, Obj::Player { .. })) && g.matches_card(&Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() }, a.card, src, ctrl)).map(|a| g.cards[a.card.0 as usize].cur.power.max(0)).sum());
                if total < *min_total_power {
                    return None;
                }
            }
            (g.player_filter_src(src, *who, attacker_ctl, ctrl) && n >= *min as usize && max.map_or(true, |m| n <= m as usize)).then(|| {
                // the creatures declared as attackers (Sel::EventCards), even ones removed from combat before the trigger resolves
                let mut cx = ctx(Some(Obj::card(*card)), Some(attacker_ctl), n as i32);
                if let Some(cb) = g.combat.as_ref() {
                    cx.cards = cb.attackers.iter().filter(|a| (!*players_only || matches!(a.defender, Obj::Player { .. })) && g.matches_card(&Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() }, a.card, src, ctrl)).map(|a| a.card).collect();
                }
                with_src(cx, Some(*card))
            })
        }
        (Trigger::PlayerAttacksOpponent { who }, Event::Attacked { card, defender }) => {
            let attacker_ctl = g.cards[card.0 as usize].cur.controller;
            match defender {
                Obj::Player { id } if *id != ctrl && g.player_filter_src(src, *who, attacker_ctl, ctrl) => Some(with_src(ctx(Some(Obj::player(*id)), Some(attacker_ctl), 0), Some(*card))),
                _ => None,
            }
        }
        (Trigger::AttacksAlone { filter, player_only }, Event::Attacked { card, defender }) => {
            if *player_only && !matches!(defender, Obj::Player { .. }) {
                return None;
            }
            let alone = g.combat.as_ref().map_or(false, |c| c.attackers.len() == 1);
            (alone && obj_matches(g, filter, *card, src, ctrl, None)).then(|| with_src(ctx(Some(Obj::card(*card)), None, 0), Some(*card)))
        }
        (Trigger::AttacksDifferentPlayers { filter }, Event::Attacked { card, defender }) => {
            let Obj::Player { id: mine } = defender else { return None };
            let other = g.combat.as_ref().map_or(false, |c| c.attackers.iter().any(|a| a.card != *card && matches!(a.defender, Obj::Player { id } if id != *mine)));
            (other && obj_matches(g, filter, *card, src, ctrl, None)).then(|| with_src(ctx(Some(Obj::card(*card)), Some(*mine), 0), Some(*card)))
        }
        (Trigger::PhasesOut { filter }, Event::PhasedOut { card }) | (Trigger::PhasesIn { filter }, Event::PhasedIn { card }) => obj_matches(g, filter, *card, src, ctrl, None).then(|| ctx(Some(Obj::card(*card)), None, 0)),
        (Trigger::FullyUnlocksRoom { filter }, Event::FullyUnlocked { card }) => (g.cards[card.0 as usize].cur.controller == ctrl && obj_matches(g, filter, *card, src, ctrl, None)).then(|| ctx(Some(Obj::card(*card)), None, 0)),
        (Trigger::UnlocksDoor { door }, Event::Unlocked { card, door: d }) => (*card == src && door.map_or(true, |x| x == *d)).then(|| ctx(Some(Obj::card(*card)), None, 0)),
        (Trigger::DayNightChanged { to_day }, Event::DayNightChanged { day }) => to_day.map_or(true, |d| d == *day).then(|| ctx(None, None, 0)),
        (Trigger::CommitsCrime { who }, Event::Crime { player }) => g.player_filter_src(src, *who, *player, ctrl).then(|| ctx(None, Some(*player), 0)),
        (Trigger::AttackerUnblocked { filter }, Event::Unblocked { card }) => obj_matches(g, filter, *card, src, ctrl, None).then(|| with_src(ctx(Some(Obj::card(*card)), None, 0), Some(*card))),
        (Trigger::Blocks { filter, once, blocked }, Event::Blocked { blocker, attacker }) => {
            if let Some(bf) = blocked {
                if !obj_matches(g, bf, *attacker, src, ctrl, None) {
                    return None;
                }
            }
            if *once && g.combat.as_ref().map_or(false, |c| c.attackers.iter().find(|a| a.blockers.contains(blocker)).map_or(false, |a| a.card != *attacker)) {
                return None;
            }
            let dp = Some(g.cards[blocker.0 as usize].cur.controller);
            let c = with_src(ctx(Some(Obj::card(*attacker)), dp, 0), Some(*blocker));
            obj_matches_ev(g, filter, *blocker, src, ctrl, &c).then(|| c)
        }
        (Trigger::BecomesBlocked { filter, by_each, blocker: bf, .. }, Event::Blocked { blocker, attacker }) => {
            if let Some(bf) = bf {
                if !obj_matches(g, bf, *blocker, src, ctrl, None) {
                    return None;
                }
            }
            let first = *by_each
                || g
                    .combat
                    .as_ref()
                    .and_then(|c| c.attackers.iter().find(|a| a.card == *attacker))
                    .map_or(false, |a| a.blockers.first() == Some(blocker));
            if !first {
                return None;
            }
            let c = with_src(ctx(Some(Obj::card(*blocker)), None, 0), Some(*attacker));
            obj_matches_ev(g, filter, *attacker, src, ctrl, &c).then(|| c)
        }
        (Trigger::BecomesBlocked { filter, blocker: None, .. }, Event::BecameBlocked { attacker }) => {
            let c = with_src(ctx(None, None, 0), Some(*attacker));
            obj_matches_ev(g, filter, *attacker, src, ctrl, &c).then(|| c)
        }
        (Trigger::DealsCombatDamage { filter, to, to_filter, to_owner }, Event::Damage { source: Some(s), target, amount, combat: true }) => {
            (dmg_to_ok(g, *to, target, ctrl, src)
                && (!*to_owner || matches!(target, Obj::Player { id } if *id == g.cards[s.0 as usize].owner))
                && to_filter.as_ref().map_or(true, |tf| matches!(target, Obj::Card { id } if obj_matches(g, tf, *id, src, ctrl, None)))
                && obj_matches(g, filter, *s, src, ctrl, None))
                .then(|| {
                    let pl = if let Obj::Player { id } = target { Some(*id) } else { None };
                    let mut c = with_src(ctx(Some(*target), pl, *amount), Some(*s));
                    if let Obj::Card { id } = target {
                        c.damaged = Some(*id);
                    }
                    c
                })
        }
        (Trigger::CombatDamageToPlayer { filter, opponents_count, .. }, Event::Damage { source: Some(s), target: Obj::Player { id }, amount, combat: true }) => {
            if *opponents_count && *id == ctrl {
                return None;
            }
            obj_matches(g, filter, *s, src, ctrl, None).then(|| with_src(ctx(Some(Obj::card(*s)), Some(*id), *amount), Some(*s)))
        }
        (Trigger::CombatDamageToPlayer { filter, or_battle: true, .. }, Event::Damage { source: Some(s), target: Obj::Card { id }, amount, combat: true }) if g.cards[id.0 as usize].cur.types.has(CardType::Battle) => {
            let owner = g.cards[id.0 as usize].cur.controller;
            obj_matches(g, filter, *s, src, ctrl, None).then(|| with_src(ctx(Some(Obj::card(*s)), Some(owner), *amount), Some(*s)))
        }
        (Trigger::DealsDamage { filter, to, combat, opponents_count, first_to_player_each_turn, .. }, Event::Damage { source: Some(s), target, amount, combat: is_combat }) => {
            let first_ok = !*first_to_player_each_turn || {
                let mut to_players = g.dmg_log.iter().filter(|r| r.source == Some(*s) && r.amount > 0 && matches!(r.target, Obj::Player { .. }));
                match to_players.next() {
                    Some(f) => f.target == *target && 1 + to_players.filter(|r| r.target == *target).count() == 1,
                    None => false,
                }
            };
            (first_ok && combat.map_or(true, |c| c == *is_combat) && (!*opponents_count || matches!(target, Obj::Player { id } if *id != ctrl)) && dmg_to_ok(g, *to, target, ctrl, src) && obj_matches(g, filter, *s, src, ctrl, None)).then(|| {
                let pl = if let Obj::Player { id } = target { Some(*id) } else { None };
                let mut c = with_src(ctx(Some(*target), pl, *amount), Some(*s));
                // `TriggerDamaged` is the permanent that was dealt the damage
                if let Obj::Card { id } = target {
                    c.damaged = Some(*id);
                }
                c
            })
        }
        (Trigger::IsDealtDamage { filter, combat: want_combat, excess, .. }, Event::Damage { source, target: Obj::Card { id }, amount, combat, .. }) => {
            // with `excess` the event amount is the excess damage (beyond lethal) of this event
            let ex = if *excess { g.dmg_log.iter().rev().find(|r| r.source == *source && r.target == Obj::card(*id) && r.amount == *amount && r.combat == *combat).map_or(0, |r| r.excess) } else { *amount };
            (want_combat.map_or(true, |w| w == *combat) && (!*excess || ex > 0) && obj_matches(g, filter, *id, src, ctrl, None))
            .then(|| {
                // `TriggerPlayer` is the controller of the damaged permanent
                let owner = g.cards[id.0 as usize].cur.controller;
                let mut c = with_src(ctx(source.map(Obj::card), Some(owner), ex), *source);
                c.damaged = Some(*id);
                c
            })
        }
        (Trigger::BeginningOfStep { step, whose }, Event::StepBegin { step: s, player }) => {
            (step == s && g.player_filter_src(src, *whose, *player, ctrl)).then(|| ctx(None, Some(*player), 0))
        }
        (Trigger::FirstUpkeepOfGame, Event::StepBegin { step: Step::Upkeep, player }) => (g.turn <= 1).then(|| ctx(None, Some(*player), 0)),
        (Trigger::EndOfCombat, Event::StepBegin { step: Step::EndCombat, player }) => Some(ctx(None, Some(*player), 0)),
        (Trigger::SpellCountered { filter, by: PlayerFilter::Any }, Event::Countered { card, by: None }) => {
            obj_matches(g, filter, *card, src, ctrl, None).then(|| with_src(ctx(Some(Obj::card(*card)), None, 0), Some(*card)))
        }
        (Trigger::SpellCountered { filter, by }, Event::Countered { card, by: Some(p) }) => {
            (g.player_filter_src(src, *by, *p, ctrl) && obj_matches(g, filter, *card, src, ctrl, None)).then(|| with_src(ctx(Some(Obj::card(*card)), Some(*p), 0), Some(*card)))
        }
        (Trigger::SpellCast { filter, by, .. }, Event::Cast { card, player }) => {
            (g.player_filter_src(src, *by, *player, ctrl) && obj_matches(g, filter, *card, src, ctrl, None))
                .then(|| with_src(ctx(Some(Obj::card(*card)), Some(*player), g.players[*player as usize].spells_cast as i32), Some(*card)))
        }
        (Trigger::SpellCast { filter, by, or_copy: true }, Event::Copied { card, player }) => {
            (g.player_filter_src(src, *by, *player, ctrl) && obj_matches(g, filter, *card, src, ctrl, None))
                .then(|| with_src(ctx(Some(Obj::card(*card)), Some(*player), 0), Some(*card)))
        }
        (Trigger::SpellCastTeamwork { filter, by }, Event::Cast { card, player }) => {
            (g.cards[card.0 as usize].teamwork && g.player_filter_src(src, *by, *player, ctrl) && obj_matches(g, filter, *card, src, ctrl, None))
                .then(|| with_src(ctx(Some(Obj::card(*card)), Some(*player), g.players[*player as usize].spells_cast as i32), Some(*card)))
        }
        (Trigger::SpellCopied { filter, by }, Event::Copied { card, player }) => {
            (g.player_filter_src(src, *by, *player, ctrl) && obj_matches(g, filter, *card, src, ctrl, None))
                .then(|| with_src(ctx(Some(Obj::card(*card)), Some(*player), 0), Some(*card)))
        }
        (Trigger::SpellCastTargets { filter, by, target, only }, Event::Cast { card, player }) => {
            if !(g.player_filter_src(src, *by, *player, ctrl) && obj_matches(g, filter, *card, src, ctrl, None)) {
                return None;
            }
            let hit = g.stack.iter().rev().find(|s| s.source == *card).map_or(false, |s| {
                (!*only || (!s.targets.is_empty() && s.targets.iter().all(|t| t.obj == s.targets[0].obj))) && s.targets.iter().any(|t| matches!(t.obj, Obj::Card { id } if g.cards[id.0 as usize].zone == Zone::Battlefield && g.matches_card(target, id, src, ctrl)))
            });
            // the permanents the spell targeted as it was cast (a later retarget does not change which ones the trigger refers to)
            let tcards: Vec<CardId> = g.stack.iter().rev().find(|s| s.source == *card).map(|s| s.targets.iter().filter_map(|t| if let Obj::Card { id } = t.obj { Some(id) } else { None }).collect()).unwrap_or_default();
            hit.then(|| {
                let mut cx = with_src(ctx(Some(Obj::card(*card)), Some(*player), 0), Some(*card));
                cx.cards = tcards;
                cx
            })
        }
        (Trigger::Scried { who, surveil, first_this_turn, to_bottom }, Event::Scry { player, n, surveil: is_surveil, bottom }) => {
            if *to_bottom {
                return (!*is_surveil && *bottom > 0 && g.player_filter_src(src, *who, *player, ctrl)).then(|| ctx(None, Some(*player), *bottom as i32));
            }
            (g.player_filter_src(src, *who, *player, ctrl) && surveil.map_or(true, |s| s == *is_surveil) && (!*first_this_turn || (*is_surveil && g.players[*player as usize].surveil_count_turn == 1))).then(|| ctx(None, Some(*player), *n as i32))
        }
        (Trigger::BecomesRenowned { filter }, Event::Renowned { card }) => {
            obj_matches(g, filter, *card, src, ctrl, None).then(|| ctx(Some(Obj::card(*card)), None, 0))
        }
        (Trigger::FacesDilemma { who }, Event::FacedDilemma { player }) => g.player_filter_src(src, *who, *player, ctrl).then(|| ctx(None, Some(*player), g.players[*player as usize].dilemmas_turn as i32)),
        (Trigger::SagaFinalChapter { filter }, Event::SagaFinalChapter { card }) => obj_matches(g, filter, *card, src, ctrl, lki).then(|| ctx(Some(Obj::card(*card)), None, 0)),
        (Trigger::Exerted { filter }, Event::Exerted { card }) => obj_matches(g, filter, *card, src, ctrl, None).then(|| ctx(Some(Obj::card(*card)), None, 0)),
        (Trigger::CaseSolved { filter }, Event::Solved { card }) => obj_matches(g, filter, *card, src, ctrl, None).then(|| ctx(Some(Obj::card(*card)), None, 0)),
        (Trigger::BecomesForetold { filter }, Event::Foretold { card }) => obj_matches(g, filter, *card, src, ctrl, None).then(|| ctx(Some(Obj::card(*card)), None, 0)),
        (Trigger::BecomesPlotted { filter }, Event::Plotted { card }) => obj_matches(g, filter, *card, src, ctrl, None).then(|| ctx(Some(Obj::card(*card)), None, 0)),
        (Trigger::Mutates { filter }, Event::Mutated { card }) => obj_matches(g, filter, *card, src, ctrl, None).then(|| ctx(Some(Obj::card(*card)), None, 0)),
        (Trigger::BecomesMonstrous { filter }, Event::Monstrous { card, n }) => {
            obj_matches(g, filter, *card, src, ctrl, None).then(|| ctx(Some(Obj::card(*card)), None, *n))
        }
        (Trigger::Expend { n, who }, Event::Expended { player, n: m }) => (*n == *m && g.player_filter_src(src, *who, *player, ctrl)).then(|| ctx(None, Some(*player), *m as i32)),
        (Trigger::EntersRoom { name }, Event::RoomEntered { player, dungeon, name: n }) => (*dungeon == src && name == n).then(|| ctx(None, Some(*player), 0)),
        (Trigger::CompletesDungeon { who }, Event::DungeonCompleted { player }) => g.player_filter_src(src, *who, *player, ctrl).then(|| ctx(None, Some(*player), 0)),
        (Trigger::TakesInitiative { who }, Event::InitiativeTaken { player }) => g.player_filter_src(src, *who, *player, ctrl).then(|| ctx(None, Some(*player), 0)),
        (Trigger::RingTempts { who }, Event::RingTempted { player }) => g.player_filter_src(src, *who, *player, ctrl).then(|| ctx(None, Some(*player), 0)),
        (Trigger::RingBearerChosen { who }, Event::RingBearerChosen { player, card }) => g.player_filter_src(src, *who, *player, ctrl).then(|| ctx(Some(Obj::card(*card)), Some(*player), 0)),
        (Trigger::Bends { who, kinds }, Event::Bent { player, kind }) => (g.player_filter_src(src, *who, *player, ctrl) && (kinds.is_empty() || kinds.contains(kind))).then(|| ctx(None, Some(*player), 1)),
        (Trigger::ManifestsDread { who }, Event::ManifestedDread { player, cards }) => g.player_filter_src(src, *who, *player, ctrl).then(|| {
            let mut c = ctx(None, Some(*player), cards.len() as i32);
            c.cards = cards.clone();
            c
        }),
        (Trigger::Milled { who, filter }, Event::Milled { player, cards }) => {
            if !g.player_filter_src(src, *who, *player, ctrl) {
                return None;
            }
            let hits: Vec<CardId> = cards.iter().copied().filter(|c| obj_matches(g, filter, *c, src, ctrl, None)).collect();
            (!hits.is_empty()).then(|| {
                let mut c = ctx(Some(Obj::card(hits[0])), Some(*player), hits.len() as i32);
                c.cards = hits.clone();
                c
            })
        }
        (Trigger::RollsDie { who, highest, batched: false }, Event::DieRolled { player, n, sides }) => (g.player_filter_src(src, *who, *player, ctrl) && (!*highest || n == sides)).then(|| ctx(None, Some(*player), *n)),
        // planes and schemes: "you" is the planar controller / the archenemy, who controls the card in their command zone
        (Trigger::RollsChaos, Event::ChaosEnsued { player }) => (*player == ctrl).then(|| ctx(None, Some(*player), 0)),
        (Trigger::RollsBlank, Event::PlanarDieRolled { player, face: PlanarFace::Blank }) => (*player == ctrl).then(|| ctx(None, Some(*player), 0)),
        (Trigger::RollsPlanarDie, Event::PlanarDieRolled { player, .. }) => (*player == ctrl).then(|| ctx(None, Some(*player), 0)),
        (Trigger::PlaneswalkTo, Event::Planeswalked { player, to }) => (*to == src).then(|| ctx(None, Some(*player), 0)),
        (Trigger::PlaneswalkFrom, Event::PlaneswalkingAway { player, from }) => from.contains(&src).then(|| ctx(None, Some(*player), 0)),
        (Trigger::SetInMotion, Event::SchemeSetInMotion { player, card, .. }) => (*card == src).then(|| ctx(None, Some(*player), 0)),
        (Trigger::AnotherSchemeSetInMotion { non_ongoing }, Event::SchemeSetInMotion { player, card, .. }) => (*card != src && *player == ctrl && (!*non_ongoing || !g.def_of(*card).supertypes.contains(&Supertype::Ongoing))).then(|| ctx(Some(Obj::Card { id: *card }), Some(*player), 0)),
        (Trigger::GiftGiven { who }, Event::GiftGiven { player }) => g.player_filter_src(src, *who, *player, ctrl).then(|| ctx(None, Some(*player), 0)),
        (Trigger::RollsDie { who, highest: false, batched: true }, Event::DiceRolled { player }) => g.player_filter_src(src, *who, *player, ctrl).then(|| ctx(None, Some(*player), g.last_die)),
        (Trigger::FlipsCoin { who, won }, Event::CoinFlipped { player, won: w }) => (won.map_or(true, |x| x == *w) && g.player_filter_src(src, *who, *player, ctrl)).then(|| ctx(None, Some(*player), 0)),
        (Trigger::PlayerCounterAdded { who, placed_by }, Event::PlayerCounters { player, delta, by, .. }) if *delta > 0 && placed_by.map_or(true, |pf| by.map_or(false, |b| g.player_filter_src(src, pf, b, ctrl))) => g.player_filter_src(src, *who, *player, ctrl).then(|| ctx(None, Some(*player), *delta)),
        (Trigger::Discovers { who }, Event::Discovered { player, n }) => g.player_filter_src(src, *who, *player, ctrl).then(|| ctx(None, Some(*player), *n)),
        (Trigger::GainsEnergy { who }, Event::EnergyGained { player, n }) => g.player_filter_src(src, *who, *player, ctrl).then(|| ctx(None, Some(*player), *n)),
        (Trigger::KeywordActionDone { action, filter }, Event::KeywordAction { card, action: a, other }) if action == a => obj_matches(g, filter, *card, src, ctrl, None).then(|| ctx(Some(Obj::card(other.unwrap_or(*card))), Some(g.cards[card.0 as usize].cur.controller), 0)),
        (Trigger::Investigates { who, first_this_turn }, Event::Investigated { player, first }) => (g.player_filter_src(src, *who, *player, ctrl) && (!*first_this_turn || *first)).then(|| ctx(None, Some(*player), 0)),
        (Trigger::CollectsEvidence { who }, Event::EvidenceCollected { player }) => g.player_filter_src(src, *who, *player, ctrl).then(|| ctx(None, Some(*player), 0)),
        (Trigger::PaysLife { who }, Event::LifePaid { player, n }) => g.player_filter_src(src, *who, *player, ctrl).then(|| ctx(None, Some(*player), *n)),
        (Trigger::Explores { filter, revealed }, Event::Explored { card, revealed: rv }) => {
            let rv_ok = revealed.as_ref().map_or(true, |f| rv.map_or(false, |r| g.matches_card(f, r, src, ctrl)));
            (rv_ok && obj_matches(g, filter, *card, src, ctrl, None)).then(|| ctx(Some(Obj::card(*card)), None, 0))
        }
        (Trigger::Connives { filter }, Event::Connived { card }) => obj_matches(g, filter, *card, src, ctrl, None).then(|| ctx(Some(Obj::card(*card)), None, 0)),
        (Trigger::Proliferates { who }, Event::Proliferated { player }) => g.player_filter_src(src, *who, *player, ctrl).then(|| ctx(None, Some(*player), 0)),
        (Trigger::Crews { filter, vehicle, saddle }, Event::CrewPaid { creature, vehicle: veh, saddle: sd }) => {
            if saddle.map_or(false, |s| s != *sd) || !obj_matches(g, filter, *creature, src, ctrl, None) || !obj_matches(g, vehicle, *veh, src, ctrl, None) {
                return None;
            }
            let mut c = ctx(Some(Obj::card(*creature)), None, 0);
            c.source = Some(*veh);
            Some(c)
        }
        (Trigger::SearchesLibrary { who }, Event::Searched { player }) => g.player_filter_src(src, *who, *player, ctrl).then(|| ctx(None, Some(*player), 0)),
        (Trigger::BecomesCrewed, Event::Crewed { card, player }) if *card == src => Some(ctx(Some(Obj::card(*card)), Some(*player), 0)),
        (Trigger::BecomesUnattached, Event::Unattached { card, from }) if *card == src => Some(ctx(Some(Obj::card(*from)), None, 0)),
        (Trigger::VotingFinished, Event::VotingFinished { player }) => Some(ctx(None, Some(*player), 0)),
        (Trigger::Clashes { who }, Event::Clashed { player, won }) => g.player_filter_src(src, *who, *player, ctrl).then(|| ctx(None, Some(*player), *won as i32)),
        (Trigger::WinsClash { who }, Event::Clashed { player, won: true }) => g.player_filter_src(src, *who, *player, ctrl).then(|| ctx(None, Some(*player), 0)),
        (Trigger::ExploitedBy { filter, victim: vf }, Event::Exploited { card, victim }) => (obj_matches(g, filter, *card, src, ctrl, None) && victim.map_or(true, |v| obj_matches(g, vf, v, src, ctrl, None))).then(|| ctx(Some(Obj::card(victim.unwrap_or(*card))), None, 0)),
        (Trigger::Exploited, Event::Exploited { card, victim }) => (*card == src).then(|| ctx(Some(Obj::card(victim.unwrap_or(*card))), None, 0)),
        (Trigger::BecomesTapped { filter, by_effect_of, not_declared_attacker, teamwork, first_this_turn }, Event::Tapped { card }) => {
            if *first_this_turn && g.cards[card.0 as usize].taps_turn != (g.turn, 1) {
                return None;
            }
            if *not_declared_attacker && g.declaring_attacker.get() {
                return None;
            }
            if *teamwork && !g.teamwork_tap.get() {
                return None;
            }
            let cause_ok = by_effect_of.map_or(true, |pf| g.cause_ctl.get().map_or(false, |p| g.player_filter_src(src, pf, p, ctrl)));
            (cause_ok && obj_matches(g, filter, *card, src, ctrl, None)).then(|| ctx(Some(Obj::card(*card)), None, 0))
        }
        (Trigger::Regenerated { filter }, Event::Regenerated { card }) => obj_matches(g, filter, *card, src, ctrl, None).then(|| ctx(Some(Obj::card(*card)), None, 0)),
        (Trigger::BecomesUntapped { filter }, Event::Untapped { card }) => {
            obj_matches(g, filter, *card, src, ctrl, None).then(|| ctx(Some(Obj::card(*card)), None, 0))
        }
        (Trigger::Draws { who, this_card, first_this_turn, nth_this_turn, not_first_in_draw_step, chosen_name }, Event::Drew { player, card }) => {
            let drawn = g.players[*player as usize].cards_drawn_turn;
            let first_in_draw_step = g.step == Step::Draw && g.active == *player && drawn == 1;
            ((!*this_card || *card == src)
                && (!*first_this_turn || drawn == 1)
                && nth_this_turn.map_or(true, |n| drawn as u32 == n)
                && !(*not_first_in_draw_step && first_in_draw_step)
                && (!*chosen_name || g.matches_card(&Filter { chosen_name: true, ..Default::default() }, *card, src, ctrl))
                && g.player_filter_src(src, *who, *player, ctrl))
            .then(|| ctx(Some(Obj::card(*card)), Some(*player), 1))
        }
        (Trigger::TapsForMana { filter, produced, produced_chosen_color, .. }, Event::ManaTapped { card, player }) => {
            if let Some(t) = produced {
                if !g.last_tapped_mana.contains(t) {
                    return None;
                }
            }
            if *produced_chosen_color {
                match g.cards[src.0 as usize].chosen_color {
                    Some(c) if g.last_tapped_mana.contains(&crate::mana::ManaType::of(c)) => {}
                    _ => return None,
                }
            }
            obj_matches(g, filter, *card, src, ctrl, None).then(|| {
                let mut e = ctx(Some(Obj::card(*card)), Some(*player), g.last_tapped_mana_n);
                for t in &g.last_tapped_mana {
                    if let Some(col) = Color::ALL.iter().find(|c| crate::mana::ManaType::of(**c) == *t) {
                        e.mana_colors |= col.bit();
                    }
                }
                e
            })
        }
        (Trigger::AbilityActivated { filter, by, exhaust, no_tap_cost, boast, loyalty, power_up, targets_creature_or_player, ninjutsu, targets_only_this, loyalty_removed_min, has_x_cost, sacrificed, mana_spent_min, incl_mana }, Event::Activated { card, player, ability, life, mana }) => {
            if *mana && !*incl_mana {
                return None;
            }
            if let Some(m) = mana_spent_min {
                if g.cards[card.0 as usize].act_spent.iter().map(|&n| n as u32).sum::<u32>() < *m as u32 {
                    return None;
                }
            }
            if *sacrificed {
                let granted = if *ability >= crate::cast::GRANTED_BASE { g.granted_ab(*card, *ability) } else { None };
                let ab = granted.as_deref().or_else(|| g.db.def(g.cards[card.0 as usize].def).abilities.get(*ability as usize));
                if !matches!(ab, Some(Ability::Activated { cost, .. }) if cost.iter().any(|c| matches!(c, Cost::SacrificeSelf | Cost::SacrificeTotalPower { .. } | Cost::Sacrifice { .. } | Cost::SacrificeDistinctNames { .. } | Cost::SacrificeAmount { .. } | Cost::SacrificeReduceByManaValue { .. } | Cost::SacrificeReduceByManaCost { .. } | Cost::SacrificeAll { .. }))) {
                    return None;
                }
            }
            if *has_x_cost {
                let granted = if *ability >= crate::cast::GRANTED_BASE { g.granted_ab(*card, *ability) } else { None };
                let ab = granted.as_deref().or_else(|| g.db.def(g.cards[card.0 as usize].def).abilities.get(*ability as usize));
                if !matches!(ab, Some(Ability::Activated { cost, .. }) if cost.iter().any(|c| matches!(c, Cost::Mana { cost } if cost.contains("{X}")) || matches!(c, Cost::XTimes { .. }))) {
                    return None;
                }
            }
            if *targets_only_this
                && !g.stack.iter().rev().find(|s| s.source == *card && matches!(s.kind, ItemKind::Ability { .. })).map_or(false, |s| s.targets.len() == 1 && s.targets[0].obj == Obj::card(src))
            {
                return None;
            }
            if *ninjutsu {
                let granted = if *ability >= crate::cast::GRANTED_BASE { g.granted_ab(*card, *ability) } else { None };
                let ab = granted.as_deref().or_else(|| g.db.def(g.cards[card.0 as usize].def).abilities.get(*ability as usize));
                if !matches!(ab, Some(Ability::Activated { cost, .. }) if cost.iter().any(|c| matches!(c, Cost::ReturnToHand { filter, .. } if filter.attacking && filter.unblocked))) {
                    return None;
                }
            }
            if *targets_creature_or_player
                && !g.stack.iter().rev().find(|s| s.source == *card && matches!(s.kind, ItemKind::Ability { .. })).map_or(false, |s| {
                    s.targets.iter().any(|t| match t.obj {
                        Obj::Player { .. } => true,
                        Obj::Card { id } => g.cards[id.0 as usize].zone == Zone::Battlefield && g.cards[id.0 as usize].cur.types.has(CardType::Creature),
                        _ => false,
                    })
                })
            {
                return None;
            }
            if *power_up {
                let granted = if *ability >= crate::cast::GRANTED_BASE { g.granted_ab(*card, *ability) } else { None };
                let ab = granted.as_deref().or_else(|| g.db.def(g.cards[card.0 as usize].def).abilities.get(*ability as usize));
                if !matches!(ab, Some(Ability::Activated { power_up: true, .. })) {
                    return None;
                }
            }
            if let Some(m) = loyalty_removed_min {
                let granted = if *ability >= crate::cast::GRANTED_BASE { g.granted_ab(*card, *ability) } else { None };
                let ab = granted.as_deref().or_else(|| g.db.def(g.cards[card.0 as usize].def).abilities.get(*ability as usize));
                if !matches!(ab, Some(Ability::Activated { loyalty: Some(l), .. }) if *l <= -(*m as i32)) {
                    return None;
                }
            }
            if *loyalty {
                let granted = if *ability >= crate::cast::GRANTED_BASE { g.granted_ab(*card, *ability) } else { None };
                let ab = granted.as_deref().or_else(|| g.db.def(g.cards[card.0 as usize].def).abilities.get(*ability as usize));
                if !matches!(ab, Some(Ability::Activated { loyalty: Some(_), .. })) {
                    return None;
                }
            }
            if *boast {
                let granted = if *ability >= crate::cast::GRANTED_BASE { g.granted_ab(*card, *ability) } else { None };
                let ab = granted.as_deref().or_else(|| g.db.def(g.cards[card.0 as usize].def).abilities.get(*ability as usize));
                if !matches!(ab, Some(Ability::Activated { limit_per_turn: Some(1), condition: Some(c), .. }) if format!("{c:?}").contains("attacked_this_turn: true")) {
                    return None;
                }
            }
            if *no_tap_cost {
                let granted = if *ability >= crate::cast::GRANTED_BASE { g.granted_ab(*card, *ability) } else { None };
                let ab = granted.as_deref().or_else(|| g.db.def(g.cards[card.0 as usize].def).abilities.get(*ability as usize));
                if !matches!(ab, Some(Ability::Activated { cost, .. }) if !cost.iter().any(|c| matches!(c, Cost::Tap))) {
                    return None;
                }
            }
            if *exhaust && !matches!(g.db.def(g.cards[card.0 as usize].def).abilities.get(*ability as usize), Some(Ability::Activated { once: true, .. })) {
                return None;
            }
            // "an ability of an artifact, creature, or land" means a permanent: abilities that work from hand or graveyard (cycling) don't count
            if !filter.types.is_empty() {
                let d = g.db.def(g.cards[card.0 as usize].def);
                if matches!(d.abilities.get(*ability as usize), Some(Ability::Activated { from: Some(z), .. }) if *z != Zone::Battlefield) {
                    return None;
                }
            }
            (g.player_filter_src(src, *by, *player, ctrl) && obj_matches(g, filter, *card, src, ctrl, g.last_lki.as_ref().filter(|l| g.cards[card.0 as usize].zone != Zone::Battlefield && l.card == *card))).then(|| ctx(Some(Obj::card(*card)), Some(*player), *life))
        }
        (Trigger::AbilityTriggeredByAttacking { by, filter }, Event::AbilityTriggered { id, source, player, attacking: true, .. }) => {
            (g.player_filter_src(src, *by, *player, ctrl) && obj_matches(g, filter, *source, src, ctrl, None)).then(|| ctx(Some(Obj::Stack { id: *id }), Some(*player), 0))
        }
        (Trigger::AbilityTriggeredByEntering { by, filter }, Event::AbilityTriggered { id, source, player, entering: true, .. }) => {
            (g.player_filter_src(src, *by, *player, ctrl) && obj_matches(g, filter, *source, src, ctrl, None)).then(|| ctx(Some(Obj::Stack { id: *id }), Some(*player), 0))
        }
        (Trigger::StackItemPut { single_target, has_targets }, Event::StackPut { id, targets }) => {
            ((!*single_target || *targets == 1) && (!*has_targets || *targets > 0)).then(|| {
                let mut c = ctx(Some(Obj::Stack { id: *id }), None, 0);
                c.stack = Some(*id);
                c
            })
        }
        (Trigger::BecomesSaddled { filter }, Event::Saddled { card }) => obj_matches(g, filter, *card, src, ctrl, None).then(|| ctx(Some(Obj::card(*card)), None, 0)),
        (Trigger::Cycled { who, this_card }, Event::Cycled { card, player }) => {
            ((!*this_card || *card == src) && g.player_filter_src(src, *who, *player, ctrl)).then(|| ctx(Some(Obj::card(*card)), Some(*player), 1))
        }
        (Trigger::PlayerBecomesTarget { who, by: by_pl, spell_only }, Event::PlayerTargeted { player, by, controller, spell, sid }) if spell_only.map_or(true, |s| s == *spell) => {
            (g.player_filter_src(src, *who, *player, ctrl) && g.player_filter_src(src, *by_pl, *controller, ctrl)).then(|| { let mut c = with_src(ctx(None, Some(*controller), 0), Some(*by)); c.stack = Some(*sid); c })
        }
        (Trigger::DamageToChosenPlayer { combat }, Event::Damage { source, target: Obj::Player { id }, amount, combat: is_combat }) if combat.map_or(true, |w| w == *is_combat) => {
            let s = &g.cards[src.0 as usize];
            (s.chosen_player == Some(*id) || s.chosen_player2 == Some(*id)).then(|| with_src(ctx(source.map(Obj::card), Some(*id), *amount), *source))
        }
        (Trigger::YouOrPermanentBecomesTarget { filter, by: by_pl }, Event::PlayerTargeted { player, by, controller, sid, .. }) if *player == ctrl => {
            g.player_filter_src(src, *by_pl, *controller, ctrl).then(|| { let mut c = with_src(ctx(None, Some(*controller), 0), Some(*by)); c.stack = Some(*sid); let _ = filter; c })
        }
        (Trigger::YouOrPermanentBecomesTarget { filter, by: by_pl }, Event::Targeted { card, by, controller, sid, .. }) => {
            if g.cards[card.0 as usize].zone != Zone::Battlefield || !g.player_filter_src(src, *by_pl, *controller, ctrl) || !obj_matches(g, filter, *card, src, ctrl, None) {
                return None;
            }
            // one trigger per spell or ability: the player target (if any) fires it, else only the first matching permanent
            let item = g.stack.iter().find(|s| s.id == *sid)?;
            if item.targets.iter().any(|t| t.obj == Obj::Player { id: ctrl }) {
                return None;
            }
            let first = item.targets.iter().find_map(|t| match t.obj {
                Obj::Card { id } if g.cards[id.0 as usize].zone == Zone::Battlefield && obj_matches(g, filter, id, src, ctrl, None) => Some(id),
                _ => None,
            });
            (first == Some(*card)).then(|| { let mut c = with_src(ctx(Some(Obj::card(*card)), Some(*controller), 0), Some(*by)); c.stack = Some(*sid); c })
        }
        (Trigger::BecomesTarget { filter, by: by_pl, spell_only, activated_only, source, only }, Event::Targeted { card, by, controller, spell, sid }) if spell_only.map_or(true, |s| s == *spell) => {
            if *only && !g.stack.iter().any(|it| it.id == *sid && !it.targets.is_empty() && it.targets.iter().all(|t| t.obj == Obj::card(*card))) {
                return None;
            }
            if *activated_only && !g.stack.iter().any(|it| it.id == *sid && matches!(it.kind, ItemKind::Ability { .. })) {
                return None;
            }
            // a creature SPELL on the stack is not a creature that "becomes the target" (Monk Gyatso must not airbend a targeted spell)
            if g.cards[card.0 as usize].zone == Zone::Stack && !filter.is_spell {
                return None;
            }
            (g.player_filter_src(src, *by_pl, *controller, ctrl) && source.as_ref().map_or(true, |f| obj_matches(g, f, *by, src, ctrl, None)) && obj_matches(g, filter, *card, src, ctrl, None)).then(|| { let mut c = with_src(ctx(Some(Obj::card(*card)), Some(*controller), 0), Some(*by)); c.stack = Some(*sid); c })
        }
        (Trigger::Discards { who, this_only, filter, cause_opponent }, Event::Discarded { card, player, by }) => {
            if (*this_only && *card != src) || (*cause_opponent && !by.map_or(false, |b| b != *player)) {
                return None;
            }
            (g.player_filter_src(src, *who, *player, ctrl) && obj_matches(g, filter, *card, src, ctrl, None)).then(|| ctx(Some(Obj::card(*card)), Some(*player), 1))
        }
        (Trigger::GainsLife { who, cause }, Event::Life { player, delta }) if *delta > 0 => {
            if let Some(f) = cause {
                // Only a spell can "cause" life gain: a triggered or delayed ability's life gain (e.g. cycling
                // Renewed Faith) is not caused by the instant or sorcery it came from.
                if g.cause_trigger.get() {
                    return None;
                }
                let Some(c) = g.cause_src.get() else { return None };
                if !obj_matches(g, f, c, src, ctrl, None) {
                    return None;
                }
            }
            g.player_filter_src(src, *who, *player, ctrl).then(|| ctx(None, Some(*player), *delta))
        }
        (Trigger::LosesLife { who, first_this_turn, exactly }, Event::Life { player, delta }) if *delta < 0 && exactly.map_or(true, |e| e == -*delta) && (!*first_this_turn || g.players[*player as usize].life_lost == -*delta) => {
            g.player_filter_src(src, *who, *player, ctrl).then(|| ctx(None, Some(*player), -*delta))
        }
        (Trigger::CounterRemoved { filter, kind, last, by_you }, Event::Counters { card, kind: k, delta, by }) if k == kind && *delta < 0 => {
            if *last && g.cards[card.0 as usize].counter(*kind) > 0 {
                return None;
            }
            if *by_you && *by != Some(ctrl) {
                return None;
            }
            obj_matches(g, filter, *card, src, ctrl, None).then(|| ctx(Some(Obj::card(*card)), None, -*delta))
        }
        (Trigger::CounterAdded { filter, kind, placed_by, first_this_turn, .. }, Event::Counters { card, kind: k, delta, by }) if kind.map_or(true, |x| x == *k) && *delta > 0 && (!*first_this_turn || g.counters_log.iter().filter(|t| t.0 == *card && t.2 == *k).count() <= 1) && placed_by.map_or(true, |pf| by.map_or(false, |b| g.player_filter_src(src, pf, b, ctrl))) => {
            obj_matches(g, filter, *card, src, ctrl, None).then(|| ctx(Some(Obj::card(*card)), None, *delta)).map(|mut c| {
                c.counter_kind = Some(*k);
                c.counter_kinds.push(*k);
                c
            })
        }
        (Trigger::SagaChapter { chapter }, Event::Counters { card, kind: CounterKind::Lore, delta, .. }) if *delta > 0 && *card == src => {
            let total = g.cards[card.0 as usize].counter(CounterKind::Lore);
            (total - *delta < *chapter as i32 && *chapter as i32 <= total).then(|| ctx(None, None, *chapter as i32))
        }
        (Trigger::SagaFinalChapterTriggers { filter }, Event::Counters { card, kind: CounterKind::Lore, delta, .. }) if *delta > 0 => {
            let max = g.db.def(g.cards[card.0 as usize].def).saga_max as i32;
            let total = g.cards[card.0 as usize].counter(CounterKind::Lore);
            (max > 0 && total - *delta < max && max <= total && obj_matches(g, filter, *card, src, ctrl, lki)).then(|| ctx(Some(Obj::card(*card)), None, 0))
        }
        (Trigger::TokenCreated { filter }, Event::Token { card }) => {
            obj_matches(g, filter, *card, src, ctrl, None).then(|| ctx(Some(Obj::card(*card)), None, 0))
        }
        (Trigger::Sacrificed { filter, emerge }, Event::Sacrificed { card, player }) => {
            ((!*emerge || g.emerge_sac.get() || g.sacrificed_while_casting_emerge()) && obj_matches(g, filter, *card, src, ctrl, lki)).then(|| ctx(Some(Obj::card(*card)), Some(*player), 0))
        }
        (Trigger::GainsControl { filter }, Event::ControlLost { card, to, from }) if *to == ctrl && *from != ctrl => obj_matches(g, filter, *card, src, ctrl, None).then(|| ctx(Some(Obj::card(*card)), Some(*to), 0)),
        (Trigger::LosesControl { filter }, Event::ControlLost { card, from, .. }) if *from == ctrl => obj_matches(g, filter, *card, src, ctrl, None).then(|| ctx(Some(Obj::card(*card)), Some(*from), 0)),
        (Trigger::Attached, Event::Attach { card, to: Some(t) }) if *card == src => Some(ctx(Some(Obj::card(*t)), None, 0)),
        (Trigger::AttachedToPermanent { attachment, host, mv_le }, Event::Attach { card, to: Some(t) }) => {
            (obj_matches(g, attachment, *card, src, ctrl, None)
                && obj_matches(g, host, *t, src, ctrl, None)
                && (!*mv_le || g.def_of(*t).mana_value <= g.def_of(*card).mana_value))
            .then(|| with_src(ctx(Some(Obj::card(*t)), None, 0), Some(*card)))
        }
        (Trigger::AttachedToThis { filter }, Event::Attach { card, to: Some(t) }) if *t == src => obj_matches(g, filter, *card, src, ctrl, None).then(|| ctx(Some(Obj::card(*card)), None, 0)),
        (Trigger::Transforms, Event::Transformed { card }) if *card == src => Some(ctx(Some(Obj::card(*card)), None, 0)),
        (Trigger::PermanentTransforms { filter, .. }, Event::Transformed { card }) => obj_matches(g, filter, *card, src, ctrl, None).then(|| ctx(Some(Obj::card(*card)), None, 0)),
        (Trigger::PermanentTransforms { filter, or_enters: true }, Event::EnteredTransformed { card }) => obj_matches(g, filter, *card, src, ctrl, None).then(|| ctx(Some(Obj::card(*card)), None, 0)),
        (Trigger::TurnedFaceUp { filter }, Event::TurnedFaceUp { card }) => obj_matches(g, filter, *card, src, ctrl, None).then(|| ctx(Some(Obj::card(*card)), None, 0)),
        (Trigger::LandPlayed { who, filter, not_from_hand, from: fz }, Event::LandPlayed { card, player, from_hand, from }) => {
            (g.player_filter_src(src, *who, *player, ctrl) && !(*not_from_hand && *from_hand) && fz.map_or(true, |z| z == *from) && obj_matches(g, filter, *card, src, ctrl, None)).then(|| ctx(Some(Obj::card(*card)), Some(*player), 0))
        }
        (Trigger::Shuffles { who }, Event::Shuffled { player }) => g.player_filter_src(src, *who, *player, ctrl).then(|| ctx(None, Some(*player), 0)),
        (Trigger::BecomesMonarch, Event::Monarch { player }) => (*player == ctrl).then(|| ctx(None, Some(*player), 0)),
        (Trigger::PlayerBecomesMonarch { who }, Event::Monarch { player }) => g.player_filter_src(src, *who, *player, ctrl).then(|| ctx(None, Some(*player), 0)),
        (Trigger::PlayerDies { who }, Event::Lost { player, .. }) => g.player_filter_src(src, *who, *player, ctrl).then(|| ctx(Some(Obj::player(*player)), Some(*player), 0)),
        _ => None,
    }
}

impl Game {
    /// Take the next triggered ability waiting to be put on the stack (active player's first, then by timestamp).
    pub fn pop_next_trigger(&mut self) -> Option<PendingTrigger> {
        if self.pending_triggers.is_empty() {
            return None;
        }
        let apnap = self.apnap();
        let (i, _) = self
            .pending_triggers
            .iter()
            .enumerate()
            .min_by_key(|(_, t)| (apnap.iter().position(|p| *p == t.controller).unwrap_or(99), t.ts))
            .unwrap();
        Some(self.pending_triggers.remove(i))
    }
}
