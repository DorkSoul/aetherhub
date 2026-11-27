//! What a client may see: the full public state plus the viewer's own hidden information, nothing else.

use crate::decision::*;
use crate::describe::*;
use crate::ids::*;
use crate::mana::ManaPool;
use crate::state::*;
use crate::types::*;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AbilityView {
    pub index: u16,
    pub text: String,
    pub mana: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CardView {
    pub id: CardId,
    pub name: String,
    pub zone: Zone,
    pub owner: PlayerId,
    pub controller: PlayerId,
    pub tapped: bool,
    pub face_down: bool,
    pub token: bool,
    pub power: Option<i32>,
    pub toughness: Option<i32>,
    /// Printed (before effects and counters).
    pub base_power: Option<i32>,
    pub base_toughness: Option<i32>,
    pub types: String,
    /// Color mask W=1 U=2 B=4 R=8 G=16.
    pub colors: u8,
    pub cost: String,
    pub mana_value: u32,
    pub text: String,
    pub loyalty: Option<i32>,
    pub counters: Vec<(CounterKind, u16)>,
    pub attached_to: Option<CardId>,
    pub summoning_sick: bool,
    pub commander: bool,
    pub keywords: Vec<Keyword>,
    pub abilities: Vec<AbilityView>,
    /// The card's own alternative costs in order (a `Cast` action's `alt` below 100 indexes this): "{U}, Exile a blue card", "from graveyard: {2}{R}".
    pub alt_costs: Vec<String>,
    /// Alternative costs other objects grant it, for the viewer's pending choice (a `Cast` action's `alt` of 100 or more indexes
    /// this at `alt - 100`): "{0} (Omniscience)", "pay life equal to its mana value (Bolas's Citadel)". Empty unless it can be cast that way now.
    pub granted_alts: Vec<String>,
    /// The name of what this object is copying (a clone that entered as a copy, or became one; a token copy of a card).
    pub copy_of: Option<String>,
    /// The name to find this object's picture by: the copied card's for a copy, else its current face's (a name-changing effect doesn't change the picture).
    pub art: String,
    /// `art` names a token (look its picture up among tokens).
    pub art_token: bool,
    pub damage: i32,
    /// A battle's protector (the opponent who defends it); the table shows the battle on that player's side.
    pub protector: Option<PlayerId>,
    /// The player an Aura enchants (a Curse); the table shows it on that player's side.
    pub enchanted_player: Option<PlayerId>,
    /// A Room on the battlefield: its unlocked doors (bit 0 = the left door, bit 1 = the right door).
    #[serde(default)]
    pub doors: Option<u8>,
    /// Notes put on it while breaking the rules ("can't block"); one lasting until end of turn ends with " · EOT".
    #[serde(default)]
    pub notes: Vec<String>,
    /// Made not to untap during its controller's untap step while breaking the rules.
    #[serde(default)]
    pub no_untap: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PlayerView {
    pub id: PlayerId,
    pub name: String,
    pub life: i32,
    pub poison: i32,
    pub energy: i32,
    pub experience: i32,
    pub hand: u32,
    pub library: u32,
    pub pool: ManaPool,
    pub lost: bool,
    pub monarch: bool,
    pub initiative: bool,
    pub commander_damage: Vec<(CardId, i32)>,
    pub commander_tax: Vec<(CardId, u32)>,
    pub mulligans: u8,
    /// Archenemy: cards left in this player's scheme deck (0: not an archenemy, or the deck ran out).
    #[serde(default)]
    pub schemes: u32,
    /// Team (Archenemy); players of one team are not opponents.
    #[serde(default)]
    pub team: u8,
    /// Planechase: planar die rolls this turn (the next roll costs this much).
    #[serde(default)]
    pub planar_rolls: u32,
    /// The library in order, top first, while it is shown to the viewer during a rule break (else empty).
    #[serde(default)]
    pub library_shown: Vec<CardId>,
    #[serde(default)]
    pub city_blessing: bool,
    /// Times the Ring has tempted this player (0-4).
    #[serde(default)]
    pub ring: u8,
    /// The dungeon this player is in and the room they are in.
    #[serde(default)]
    pub dungeon: Option<(String, String)>,
    /// Labelled markers made while breaking the rules: (label, value).
    #[serde(default)]
    pub markers: Vec<(String, i32)>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DecisionAbout {
    pub source: CardId,
    pub name: String,
    /// The spell's Oracle text, or the ability's printed text ("{1}, {T}, Sacrifice ...: ...").
    pub text: String,
    pub is_spell: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct StackView {
    pub id: StackId,
    pub source: CardId,
    pub controller: PlayerId,
    pub name: String,
    pub text: String,
    pub targets: Vec<Obj>,
    pub is_spell: bool,
    pub x: i32,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CombatView {
    pub attacker: CardId,
    pub defender: Obj,
    pub blockers: Vec<CardId>,
    /// Where this attacker's combat damage goes with the default assignment (a preview: blockers in order until each has
    /// lethal damage, the rest to the defender with trample). Empty when it deals no damage.
    pub damage_to: Vec<Obj>,
}

/// A preview of a proposed block (nothing changes): the blocks the rules keep, the ones they drop and why, and where the
/// attacker's combat damage would go.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct BlockPreview {
    pub attacker: CardId,
    pub blockers: Vec<CardId>,
    /// (blocker, why the rules drop this block)
    pub dropped: Vec<(CardId, String)>,
    pub damage_to: Vec<Obj>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct View {
    pub you: PlayerId,
    pub turn: u32,
    pub step: Step,
    pub active: PlayerId,
    pub players: Vec<PlayerView>,
    pub cards: Vec<CardView>,
    pub stack: Vec<StackView>,
    pub combat: Vec<CombatView>,
    pub day_night: Option<bool>,
    pub over: Option<GameResult>,
    /// The question waiting for this player, if any.
    pub decision: Option<Decision>,
    /// Cards named by the viewer's pending question that they couldn't otherwise see (library searches, scry, surveil).
    pub decision_cards: Vec<CardView>,
    /// The spell or ability the viewer's pending question belongs to (being cast or activated, or resolving).
    #[serde(default)]
    pub decision_about: Option<DecisionAbout>,
    /// Permanents the viewer could tap for mana right now.
    pub mana_sources: Vec<ManaSourceOpt>,
    /// Planechase: cards left in the planar deck (the face-up planes are command-zone cards).
    #[serde(default)]
    pub planar_deck: u32,
    /// Labelled markers on the game made while breaking the rules: (label, value).
    #[serde(default)]
    pub markers: Vec<(String, i32)>,
    /// Who is being asked (any player), so the UI can show "waiting for X".
    pub waiting_for: Option<PlayerId>,
}

impl Game {
    /// What a stack item does, as the stack and the game log show it: a spell's Oracle text, an ability's printed text.
    pub(crate) fn stack_item_text(&self, s: &StackItem) -> String {
        let d = self.db.def(s.def);
        match &s.kind {
            ItemKind::Spell { .. } => d.oracle_text.clone(),
            ItemKind::Ability { ability, .. } | ItemKind::Triggered { ability, .. } => printed_ability_text(d, *ability as usize),
            ItemKind::Delayed { .. } => {
                let all = d.oracle_text.split_whitespace().collect::<Vec<_>>().join(" ");
                if all.is_empty() { "Delayed trigger".into() } else { format!("Delayed trigger: {}", short_text(&all, 700)) }
            }
        }
    }

    /// Is `id` the top card of a library that `viewer` may look at (`TopCardVisible`: the controller of the static, or everyone when the card is played revealed)?
    fn revealed_top(&self, id: CardId, viewer: PlayerId) -> bool {
        let owner = self.card(id).owner;
        if self.players[owner as usize].library.last() != Some(&id) {
            return false;
        }
        let ok = |ctl: PlayerId, e: &crate::dsl::StaticEffect| match e {
            crate::dsl::StaticEffect::TopCardVisible { all, each_library } => (ctl == owner || *each_library) && (*all || *each_library || viewer == owner),
            _ => false,
        };
        // granted to a player for a while ("until end of turn, you may look at the top card of your library": Gwenom)
        if self.effects.iter().any(|e| e.players.contains(&owner) && matches!(&e.modif, crate::state::Modif::Static(b) if ok(owner, b))) {
            return true;
        }
        // permanents (printed or granted: Glowcap Lantern) and command-zone planes / emblems (Skybreen, Windriddle Palaces)
        self.battlefield.iter().copied().chain(self.emblem_srcs()).any(|b| {
            let c = self.card(b);
            if c.cur.no_abilities {
                return false;
            }
            let ctl = self.src_ctrl(b);
            let in_command = c.zone == Zone::Command;
            let d = self.def_of(b);
            let granted = if in_command { None } else { Some(c.cur.granted.iter().map(|g| &**g)) };
            d.statics.iter().map(|&i| &d.abilities[i as usize]).chain(granted.into_iter().flatten()).any(|a| match a {
                crate::dsl::Ability::Static { effect, from, .. } if from.unwrap_or(Zone::Battlefield) == c.zone => ok(ctl, effect),
                _ => false,
            })
        })
    }

    fn card_view(&self, id: CardId, viewer: PlayerId, force: bool) -> Option<CardView> {
        let c = self.card(id);
        let d = self.def_of(id);
        // visibility
        let shown = |z: Zone| self.edit_shown.contains(&(viewer, c.owner, z));
        let visible = match c.zone {
            Zone::Hand => c.owner == viewer || shown(Zone::Hand),
            Zone::Library => self.revealed_top(id, viewer) || shown(Zone::Library),
            Zone::Sideboard => c.owner == viewer,
            Zone::Exile => !c.face_down || c.owner == viewer,
            // the planar deck and scheme decks are face down: kept in the command zone but in no player's command list
            Zone::Command => self.players.iter().any(|p| p.command.contains(&id)),
            _ => true,
        };
        if !visible && !force {
            return None;
        }
        let masked = c.face_down && c.zone == Zone::Battlefield && c.cur.controller != viewer;
        let on_bf = c.zone == Zone::Battlefield;
        let cur = &c.cur;
        let is_creature = cur.types.has(CardType::Creature);
        if masked {
            return Some(CardView {
                id,
                name: "Face-down creature".into(),
                zone: c.zone,
                owner: c.owner,
                controller: cur.controller,
                tapped: c.tapped,
                face_down: true,
                token: c.token,
                power: Some(2),
                toughness: Some(2),
                base_power: Some(2),
                base_toughness: Some(2),
                types: "Creature".into(),
                colors: 0,
                cost: String::new(),
                mana_value: 0,
                text: String::new(),
                loyalty: None,
                counters: c.counters.to_vec(),
                attached_to: c.attached_to,
                summoning_sick: false,
                commander: false,
                keywords: vec![],
                abilities: vec![],
                alt_costs: vec![],
                granted_alts: vec![],
                copy_of: None,
                art: String::new(),
                art_token: false,
                damage: c.damage,
                protector: c.protector,
                enchanted_player: c.enchanted_player,
                doors: None,
                notes: vec![],
                no_untap: false,
            });
        }
        // what it is a copy of: a permanent that entered as or became a copy (its definition is another card's), or a token copy of a card.
        // Its own faces and text-changed versions of them aren't copies.
        let copy_src: Option<crate::ids::DefId> = {
            let printed = self.db.def(c.front);
            let base = c.text_base.unwrap_or(c.def);
            let own = |x: crate::ids::DefId| x == c.front || printed.other_face == Some(x);
            // a melded permanent (made as a token of the meld card) and a mutated pile (topped by the mutating card) aren't copies
            if c.face_down || !c.mut_parts.is_empty() || self.meld_parts.iter().any(|m| m.0 == id) {
                None
            } else if !own(c.def) && !own(base) && self.db.def(base).display_name != printed.display_name {
                Some(base)
            } else if c.token && !printed.token {
                Some(base)
            } else {
                None
            }
        };
        let kws: Vec<Keyword> = Keyword::ALL.iter().copied().filter(|k| cur.kw.has(*k)).collect();
        let mut oracle = OracleAbilities::new(&d.oracle_text);
        let abilities: Vec<AbilityView> = d
            .abilities
            .iter()
            .enumerate()
            .filter_map(|(i, a)| match a {
                crate::dsl::Ability::Activated { .. } => {
                    let text = view_ability_text(a);
                    let text = oracle.take(&text).unwrap_or(text);
                    Some(AbilityView { index: i as u16, text, mana: d.mana_abilities.contains(&(i as u16)) })
                }
                _ => None,
            })
            .chain(cur.granted.iter().enumerate().filter_map(|(k, a)| match &**a {
                crate::dsl::Ability::Activated { .. } => Some(AbilityView { index: crate::cast::GRANTED_BASE + k as u16, text: ability_text(a), mana: crate::carddb::is_mana_ability(a) }),
                _ => None,
            }))
            .collect();
        Some(CardView {
            id,
            // the name it has now: a copy "except its name is ..." (Chameleon, Master of Disguise) keeps its own
            name: match cur.name {
                Some(n) if on_bf => n.name(),
                _ => d.display_name.clone(),
            },
            zone: c.zone,
            owner: c.owner,
            controller: if on_bf { cur.controller } else { c.owner },
            tapped: c.tapped,
            face_down: c.face_down,
            token: c.token,
            power: is_creature.then_some(cur.power),
            toughness: is_creature.then_some(cur.toughness),
            base_power: d.power,
            base_toughness: d.toughness,
            types: type_line(&cur.supertypes, cur.types, &cur.subtypes),
            colors: cur.colors.0,
            cost: d.cost.display(),
            mana_value: d.mana_value,
            text: d.oracle_text.clone(),
            loyalty: cur.types.has(CardType::Planeswalker).then(|| c.counter(CounterKind::Loyalty)),
            counters: c.counters.to_vec(),
            attached_to: c.attached_to,
            summoning_sick: on_bf && is_creature && !self.can_use_tap_symbol(id),
            commander: c.is_commander,
            keywords: kws,
            abilities,
            alt_costs: d
                .abilities
                .iter()
                .filter_map(|a| match a {
                    crate::dsl::Ability::AlternativeCost { cost, from, .. } => {
                        let text = if cost.is_empty() { "no mana".to_string() } else { cost.iter().map(cost_text).collect::<Vec<_>>().join(", ") };
                        Some(match from {
                            Some(z) => format!("from {}: {text}", format!("{z:?}").to_lowercase()),
                            None => text,
                        })
                    }
                    _ => None,
                })
                .collect(),
            granted_alts: self.granted_alt_labels(id, viewer),
            copy_of: copy_src.map(|x| self.db.def(x).display_name.clone()),
            art: copy_src.map_or_else(|| d.display_name.clone(), |x| self.db.def(x).display_name.clone()),
            // by the definition, not the object: a melded permanent is made as a token but pictured as its card
            art_token: self.db.def(copy_src.unwrap_or(c.def)).token,
            damage: c.damage,
            protector: c.protector,
            enchanted_player: c.enchanted_player,
            doors: (on_bf && d.room).then_some(c.doors),
            notes: self.edit_notes.iter().filter(|(x, _, until)| *x == id && until.map_or(true, |t| t == self.turn)).map(|(_, t, until)| if until.is_some() { format!("{t} · EOT") } else { t.clone() }).collect(),
            no_untap: self.edit_no_untap.contains(&id),
        })
    }

    /// The game as `viewer` may see it.
    pub fn view_for(&self, viewer: PlayerId) -> View {
        let mut cards = vec![];
        for c in &self.cards {
            if c.token && c.zone != Zone::Battlefield && c.zone != Zone::Stack {
                continue;
            }
            if let Some(v) = self.card_view(c.id, viewer, false) {
                // cards parked outside every list (ceased tokens, spell copies) are not shown
                cards.push(v);
            }
        }
        let players = self
            .players
            .iter()
            .map(|p| PlayerView {
                id: p.id,
                name: p.name.clone(),
                life: p.life,
                poison: p.poison,
                energy: p.energy,
                experience: p.experience,
                hand: p.hand.len() as u32,
                library: p.library.len() as u32,
                pool: p.pool,
                lost: p.lost,
                monarch: self.monarch == Some(p.id),
                initiative: self.initiative == Some(p.id),
                commander_damage: p.commander_damage.to_vec(),
                commander_tax: p.commander_cast.to_vec(),
                mulligans: p.mulligans,
                schemes: p.scheme_deck.len() as u32,
                team: p.team,
                planar_rolls: p.planar_rolls,
                library_shown: if self.edit_shown.contains(&(viewer, p.id, Zone::Library)) { p.library.iter().rev().copied().collect() } else { vec![] },
                city_blessing: p.city_blessing,
                ring: p.ring_level,
                dungeon: p.dungeon.as_ref().map(|(dc, room)| (self.def_of(*dc).name.clone(), room.clone())),
                markers: self.edit_markers.iter().filter(|(q, _, _)| *q == Some(p.id)).map(|(_, l, v)| (l.clone(), *v)).collect(),
            })
            .collect();
        let stack = self
            .stack
            .iter()
            .map(|s| {
                let d = self.db.def(s.def);
                let is_spell = matches!(s.kind, ItemKind::Spell { .. });
                let text = self.stack_item_text(s);
                StackView {
                    id: s.id,
                    source: s.source,
                    controller: s.controller,
                    name: d.name.clone(),
                    text,
                    targets: s.targets.iter().map(|t| t.obj).collect(),
                    is_spell,
                    x: s.x,
                }
            })
            .collect();
        let combat = self
            .combat
            .as_ref()
            .map(|cb| cb.attackers.iter().map(|a| CombatView { attacker: a.card, defender: a.defender, blockers: a.blockers.to_vec(), damage_to: self.damage_targets(a.card, a.defender, a.blocked, &a.blockers) }).collect())
            .unwrap_or_default();
        let decision = self.pending.clone().filter(|d| d.player == viewer);
        let mut decision_cards = vec![];
        // free mulligan + scry 3: the top of the library the player looks at while deciding
        if let Some(Decision { kind: DecisionKind::Mulligan { peek, .. }, .. }) = &decision {
            for id in peek {
                if let Some(v) = self.card_view(*id, viewer, true) {
                    decision_cards.push(v);
                }
            }
        }
        if let Some(Decision { kind: DecisionKind::Pick { options, .. }, .. }) = &decision {
            for o in options {
                if let Opt::Obj { obj: Obj::Card { id } } = o {
                    if !cards.iter().any(|c: &CardView| c.id == *id) {
                        if let Some(v) = self.card_view(*id, viewer, true) {
                            decision_cards.push(v);
                        }
                    }
                }
            }
        }
        let mana_sources = if decision.is_some() { self.mana_sources(viewer) } else { vec![] };
        // the spell or ability the question is part of: being cast / activated, or resolving
        let decision_about = decision.as_ref().and_then(|_| {
            let item = match &self.run {
                Run::Cast(c) => &c.item,
                Run::Resolve(r) => &r.item,
                _ => return None,
            };
            let d = self.db.def(item.def);
            let (text, is_spell) = match &item.kind {
                ItemKind::Spell { .. } => (d.oracle_text.clone(), true),
                ItemKind::Ability { ability, .. } | ItemKind::Triggered { ability, .. } => (printed_ability_text(d, *ability as usize), false),
                ItemKind::Delayed { .. } => ("Delayed trigger".into(), false),
            };
            Some(DecisionAbout { source: item.source, name: d.name.clone(), text, is_spell })
        });
        View {
            you: viewer,
            turn: self.turn,
            step: self.step,
            active: self.active,
            players,
            cards,
            stack,
            combat,
            day_night: self.day_night,
            over: self.over.clone(),
            decision,
            decision_cards,
            decision_about,
            mana_sources,
            planar_deck: self.planar_deck.len() as u32,
            markers: self.edit_markers.iter().filter(|(q, _, _)| q.is_none()).map(|(_, l, v)| (l.clone(), *v)).collect(),
            waiting_for: self.pending.as_ref().map(|d| d.player),
        }
    }
}

/// The activated-ability lines of a card's Oracle text ("cost: effect"), matched in order to the card's activated
/// abilities so menus and prompts show the printed wording instead of the generated summary (which shortens effects
/// it can't describe to "…").
struct OracleAbilities {
    lines: Vec<(Vec<String>, String)>,
    next: usize,
}

impl OracleAbilities {
    fn new(oracle: &str) -> Self {
        let lines = oracle
            .lines()
            .filter_map(|l| {
                let l = l.trim();
                let (cost, _) = l.split_once(": ")?;
                // not a quoted ability another object gets, a chapter or a long sentence that merely holds a colon
                if cost.contains('"') || cost.contains(". ") || cost.len() > 120 || cost.is_empty() || l.starts_with('•') {
                    return None;
                }
                Some((cost_key(cost), strip_reminder(l)))
            })
            .collect();
        OracleAbilities { lines, next: 0 }
    }

    /// The Oracle line for the next activated ability whose generated text is `generated`, when its cost matches one.
    fn take(&mut self, generated: &str) -> Option<String> {
        let (cost, _) = generated.split_once(": ")?;
        let key = cost_key(cost);
        let j = (self.next..self.lines.len()).find(|&j| self.lines[j].0 == key)?;
        self.next = j + 1;
        Some(self.lines[j].1.clone())
    }
}

/// A cost compared loosely: each comma-separated part is its mana / tap symbols, or else its first word ("sacrifice"),
/// with a loyalty cost's sign made plain ("−2" and "-2").
fn cost_key(cost: &str) -> Vec<String> {
    cost.split(", ")
        .map(|part| {
            let part = part.trim().replace('\u{2212}', "-");
            if part.starts_with('{') {
                part.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_uppercase()
            } else if part.starts_with(['+', '-']) || part.chars().all(|c| c.is_ascii_digit()) {
                part.trim_start_matches('+').to_string()
            } else {
                part.split_whitespace().next().unwrap_or("").to_lowercase()
            }
        })
        .collect()
}

/// The line without its reminder text in parentheses.
fn strip_reminder(line: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for ch in line.chars() {
        match ch {
            '(' => depth += 1,
            ')' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(ch),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The printed text of ability `idx` of `d` when it can be found: an activated ability's Oracle line (see
/// `OracleAbilities`), or the Oracle line of the n-th triggered ability when the card has as many trigger lines
/// ("When / Whenever / At ...") as triggered abilities. Otherwise the generated text.
pub fn printed_ability_text(d: &crate::carddb::Def, idx: usize) -> String {
    use crate::dsl::{Ability, Trigger};
    let Some(a) = d.abilities.get(idx) else { return String::new() };
    // a generated text that says nothing ("…") gives way to the card's own text (cut short on a very long card)
    let generated = |a: &Ability| {
        let t = ability_text(a);
        let all = d.oracle_text.split_whitespace().collect::<Vec<_>>().join(" ");
        if (t.is_empty() || t.contains('…')) && !all.is_empty() { short_text(&all, 700) } else { t }
    };
    if let Ability::Triggered { trigger: Trigger::SagaChapter { chapter }, .. } = a {
        return saga_chapter_line(&d.oracle_text, *chapter).unwrap_or_else(|| generated(a));
    }
    if is_saga_lore_trigger(d, a) {
        return "Add a lore counter.".into();
    }
    match a {
        Ability::Activated { .. } => {
            let mut oracle = OracleAbilities::new(&d.oracle_text);
            let mut found = None;
            for (i, b) in d.abilities.iter().enumerate().take(idx + 1) {
                if matches!(b, Ability::Activated { .. }) {
                    let t = oracle.take(&view_ability_text(b));
                    if i == idx {
                        found = t;
                    }
                }
            }
            found.or_else(|| activated_line_loose(&d.oracle_text, &view_ability_text(a))).unwrap_or_else(|| {
                let t = view_ability_text(a);
                if t.contains('…') { generated(a) } else { t }
            })
        }
        Ability::Triggered { .. } => {
            let lines: Vec<String> = d
                .oracle_text
                .lines()
                .map(strip_reminder)
                .filter(|l| l.starts_with("When ") || l.starts_with("Whenever ") || l.starts_with("At "))
                .collect();
            // a Saga's chapters and its lore counter have lines of their own
            let trig: Vec<usize> = d
                .abilities
                .iter()
                .enumerate()
                .filter(|(_, b)| matches!(b, Ability::Triggered { .. }) && !matches!(b, Ability::Triggered { trigger: Trigger::SagaChapter { .. }, .. }) && !is_saga_lore_trigger(d, b))
                .map(|(i, _)| i)
                .collect();
            match trig.iter().position(|&i| i == idx) {
                Some(k) if lines.len() == trig.len() => lines[k].clone(),
                _ => trigger_line_by_event(&lines, a).unwrap_or_else(|| generated(a)),
            }
        }
        _ => generated(a),
    }
}

/// `t` cut to at most `max` bytes at a word boundary, with "…" when it was cut.
pub(crate) fn short_text(t: &str, max: usize) -> String {
    if t.len() <= max {
        return t.to_string();
    }
    let mut end = max;
    while !t.is_char_boundary(end) {
        end -= 1;
    }
    let cut = &t[..end];
    format!("{}…", cut.rsplit_once(' ').map_or(cut, |(a, _)| a))
}

/// An activated ability's Oracle line found loosely, when the costs don't line up: the line holding the same mana symbols
/// (a keyword line, "Morph {2}{W}{B}{G}"), else the one whose cost starts with the same word ("Sacrifice ...").
fn activated_line_loose(oracle: &str, generated: &str) -> Option<String> {
    let (cost, _) = generated.split_once(": ")?;
    let first = cost.split(", ").next().unwrap_or("");
    let mana: String = first.split_inclusive('}').filter(|p| p.starts_with('{')).collect();
    let lines: Vec<String> = oracle.lines().map(strip_reminder).collect();
    if !mana.is_empty() && mana != "{T}" {
        if let Some(l) = lines.iter().find(|l| l.contains(&mana)) {
            return Some(l.clone());
        }
    }
    let word = first.split_whitespace().next()?.to_lowercase();
    if word.starts_with('{') {
        return None;
    }
    lines.into_iter().find(|l| l.split_once(':').map_or(false, |(c, _)| c.to_lowercase().starts_with(&word)))
}

/// A triggered ability's Oracle line picked by its event when the lines can't be paired one to one: the only trigger
/// line naming the event ("enters", "dies", "attacks", "upkeep" ...).
fn trigger_line_by_event(lines: &[String], a: &crate::dsl::Ability) -> Option<String> {
    let crate::dsl::Ability::Triggered { trigger, .. } = a else { return None };
    let dbg = format!("{trigger:?}");
    let variant = dbg.split(|c: char| !c.is_alphanumeric()).next().unwrap_or("");
    let mut words: Vec<String> = vec![];
    let mut cur = String::new();
    for ch in variant.chars() {
        if ch.is_uppercase() && !cur.is_empty() {
            words.push(std::mem::take(&mut cur));
        }
        cur.push(ch.to_ascii_lowercase());
    }
    words.push(cur);
    if variant == "BeginningOfStep" {
        words = match () {
            _ if dbg.contains("Upkeep") => vec!["upkeep".into()],
            _ if dbg.contains("step: End") => vec!["end step".into()],
            _ if dbg.contains("step: Draw") => vec!["draw step".into()],
            _ if dbg.contains("Combat") => vec!["combat".into()],
            _ if dbg.contains("Main") => vec!["main phase".into()],
            _ => vec![],
        };
    }
    const FILLER: [&str; 12] = ["battlefield", "or", "to", "player", "creature", "you", "of", "a", "the", "any", "trigger", "beginning"];
    let stems: Vec<String> = words.iter().filter(|w| !FILLER.contains(&w.as_str()) && w.len() >= 3).map(|w| w.chars().take(5).collect()).collect();
    if stems.is_empty() {
        return None;
    }
    let score = |l: &String| {
        let ll = l.to_lowercase();
        let head = ll.split(',').next().unwrap_or(&ll).to_string();
        stems.iter().filter(|s| head.contains(s.as_str())).count()
    };
    let best = lines.iter().map(score).max().unwrap_or(0);
    if best == 0 {
        return None;
    }
    let hits: Vec<&String> = lines.iter().filter(|l| score(l) == best).collect();
    (hits.len() == 1).then(|| hits[0].clone())
}

/// The Oracle line of a Saga's chapter `n` ("II, III — Until your next turn, ...").
fn saga_chapter_line(oracle: &str, n: u8) -> Option<String> {
    let roman = |r: &str| -> Option<u8> {
        let v = |c| match c { 'I' => Some(1), 'V' => Some(5), 'X' => Some(10), _ => None };
        let ds: Vec<u8> = r.trim().chars().map(v).collect::<Option<_>>()?;
        let mut total: i32 = 0;
        for (i, &x) in ds.iter().enumerate() {
            if ds.get(i + 1).map_or(false, |&y| y > x) { total -= x as i32 } else { total += x as i32 }
        }
        (total > 0 && !ds.is_empty()).then_some(total as u8)
    };
    oracle.lines().map(strip_reminder).find(|l| {
        l.split_once(" \u{2014} ").map_or(false, |(nums, _)| nums.split(',').filter_map(roman).any(|x| x == n))
    })
}

/// The lore-counter trigger a Saga is given (`expand_saga`): add a lore counter at the beginning of your precombat main phase.
fn is_saga_lore_trigger(d: &crate::carddb::Def, a: &crate::dsl::Ability) -> bool {
    use crate::dsl::{Ability, Effect, Trigger};
    let saga = d.abilities.iter().any(|b| matches!(b, Ability::Triggered { trigger: Trigger::SagaChapter { .. }, .. }));
    saga && matches!(a, Ability::Triggered { trigger: Trigger::BeginningOfStep { step: Step::Main1, .. }, effects, .. }
        if matches!(effects.as_slice(), [Effect::AddCounters { kind: CounterKind::Lore, .. }]))
}

/// An activated ability's text for the table; the special actions generated from keywords (foretell, plot, warp) say their keyword.
fn view_ability_text(a: &crate::dsl::Ability) -> String {
    use crate::dsl::{Ability, Effect, ExileMark};
    if let Ability::Activated { cost, effects, .. } = a {
        let mark = effects.iter().find_map(|e| if let Effect::MarkExiled { mark } = e { Some(*mark) } else { None });
        let word = match mark {
            Some(ExileMark::Foretold) => Some("Foretell"),
            Some(ExileMark::Plotted) => Some("Plot"),
            Some(ExileMark::Warped) => Some("Warp"),
            None => None,
        };
        if let Some(w) = word {
            return format!("{w} ({}: exile it from your hand)", cost.iter().map(cost_text).collect::<Vec<_>>().join(", "));
        }
    }
    ability_text(a)
}

impl Game {
    /// Labels for the granted alternative costs of `id` when `viewer`'s pending priority choice can cast it with one.
    fn granted_alt_labels(&self, id: CardId, viewer: PlayerId) -> Vec<String> {
        let granted = match &self.pending {
            Some(Decision { player, kind: DecisionKind::Priority { actions, .. } }) if *player == viewer => {
                actions.iter().any(|a| matches!(a, Action::Cast { card, alt: Some(i) } if *card == id && *i >= crate::cast::GRANT_ALT_BASE && *i < FACE_DOWN_ALT))
            }
            _ => false,
        };
        if !granted {
            return vec![];
        }
        let c = self.card(id);
        self.granted_alt_costs(viewer, id, c.zone)
            .into_iter()
            .map(|(src, cost, _, life, extra, _)| {
                if cost.is_empty() && !life {
                    return String::new();
                }
                let mut parts: Vec<String> = vec![];
                if life {
                    parts.push("pay life equal to its mana value".into());
                } else if cost == "own" {
                    parts.push("its mana cost".into());
                } else {
                    parts.push(cost);
                }
                parts.extend(extra.iter().map(cost_text));
                format!("{} ({})", parts.join(", "), self.def_of(src).display_name)
            })
            .collect()
    }
}

#[cfg(test)]
mod oracle_ability_tests {
    use super::*;

    #[test]
    fn activated_abilities_take_their_oracle_lines() {
        let mut o = OracleAbilities::new("{T}: Add {C}.\n{1}, {T}, Sacrifice this land: You may cast spells this turn as though they had flash.");
        assert_eq!(o.take("{T}: Add {C}").as_deref(), Some("{T}: Add {C}."));
        assert_eq!(o.take("{1}, {T}, Sacrifice this: …").as_deref(), Some("{1}, {T}, Sacrifice this land: You may cast spells this turn as though they had flash."));
        // planeswalker loyalty, reminder text, a quoted granted ability that isn't this card's own
        let mut o = OracleAbilities::new("Creatures you control have \"{T}: Add {G}.\"\n+1: Scry 1. (Look at the top card.)\n\u{2212}2: Draw a card.\n0: Untap it.");
        assert_eq!(o.take("+1: Scry 1").as_deref(), Some("+1: Scry 1."));
        assert_eq!(o.take("-2: Draw 1").as_deref(), Some("\u{2212}2: Draw a card."));
        assert_eq!(o.take("+0: …").as_deref(), Some("0: Untap it."));
        // no matching line (a keyword ability such as equip): the generated text stays
        let mut o = OracleAbilities::new("Equip {2}");
        assert_eq!(o.take("{2}: Attach to target creature you control"), None);
    }
}
