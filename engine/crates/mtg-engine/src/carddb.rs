//! The immutable card database: card definitions compiled once at load time (never interpreted from text during play).

use crate::dsl::*;
use crate::ids::DefId;
use crate::mana::ManaCost;
use crate::types::*;
use smallvec::SmallVec;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct Def {
    pub id: DefId,
    pub name: String,
    pub cost: ManaCost,
    pub mana_value: u32,
    /// Split card: total of both halves' mana values (its mana value anywhere except on the stack); 0 for other cards.
    pub split_mv: u32,
    pub types: TypeSet,
    pub supertypes: SmallVec<[Supertype; 2]>,
    pub subtypes: SmallVec<[Sym; 4]>,
    pub colors: ColorSet,
    /// Color identity: colors of the cost, color indicator, and every mana symbol in the rules text (and the other face).
    pub identity: ColorSet,
    pub power: Option<i32>,
    pub toughness: Option<i32>,
    pub loyalty: Option<i32>,
    pub defense: Option<i32>,
    pub keywords: KeywordSet,
    pub abilities: Vec<Ability>,
    /// Indices into `abilities`, by kind (precomputed so scans don't walk every ability).
    pub triggers: SmallVec<[u16; 2]>,
    pub statics: SmallVec<[u16; 2]>,
    pub activated: SmallVec<[u16; 2]>,
    /// Activated abilities that are mana abilities (no target, produce mana, no loyalty).
    pub mana_abilities: SmallVec<[u16; 2]>,
    /// Bit i set when some trigger on this card reacts to `EventKind` i.
    pub trigger_mask: u64,
    /// Precompiled effect lists: `programs[ability][mode]` (non-modal abilities have one entry).
    pub programs: Vec<Vec<Arc<Vec<Effect>>>>,
    pub other_face: Option<DefId>,
    /// Some ability of this card is a `Replace` (so the engine only scans cards that have one).
    pub has_replace: bool,
    pub token: bool,
    /// A printed card with no mana cost (Ancestral Vision, Living End): it can't be cast normally, only by suspend / free-cast effects.
    pub no_mana_cost: bool,
    pub oracle_text: String,
    pub is_front: bool,
    /// Either face can be cast from hand (modal DFC, split card).
    pub face_castable: bool,
    /// Either face of a transforming double-faced card (layout Transform).
    pub transforming: bool,
    /// Highest chapter of a saga (0 = not a saga).
    pub saga_max: u8,
    /// Front face of an adventure card.
    pub adventure: bool,
    /// Prepare layout front face (a creature with a prepare spell).
    pub prepare: bool,
    pub room: bool,
    /// Any player may activate its activated abilities.
    pub any_player: bool,
    /// Carries `Ability::LkiSnapshot`: leaving the battlefield records the battlefield in its last-known information.
    pub lki_snapshot: bool,
    /// Carries "Start your engines!" (expanded into an enters trigger with `Effect::StartEngines`): controlling it gives a player with no speed speed 1.
    pub engines: bool,
    /// Front face of an Omen card.
    pub omen: bool,
    pub opponents_only: bool,
    /// Index of the activated ability only the attached permanent's controller may activate (`Ability::EnchantedControllerMayActivate`).
    pub enchanted_ctl_ability: Option<u16>,
    /// Entwine: a kicked cast chooses every mode.
    pub entwine: bool,
    /// Has a triggered ability that watches from another zone (graveyard, hand, exile...).
    pub zone_triggers: bool,
    /// Name shown to players (tokens named like "Beast 4-4" to tell variants apart show as "Beast").
    pub display_name: String,
}

impl Def {
    /// The keywords of this definition as a list.
    pub fn keywords_list(&self) -> Vec<Keyword> {
        Keyword::ALL.iter().copied().filter(|k| self.keywords.has(*k)).collect()
    }

    /// Recompute the per-kind ability indices, trigger mask and compiled programs after `abilities` changed.
    pub fn reindex(&mut self) {
        self.triggers.clear();
        self.statics.clear();
        self.activated.clear();
        self.mana_abilities.clear();
        self.trigger_mask = 0;
        for (i, a) in self.abilities.iter().enumerate() {
            let i = i as u16;
            match a {
                Ability::Triggered { trigger, .. } => {
                    self.triggers.push(i);
                    self.trigger_mask |= crate::triggers::trigger_kind_mask(trigger);
                }
                Ability::Static { .. } => self.statics.push(i),
                Ability::Activated { .. } => {
                    self.activated.push(i);
                    if is_mana_ability(a) {
                        self.mana_abilities.push(i);
                    }
                }
                _ => {}
            }
        }
        self.programs = self.abilities.iter().map(|a| ability_effect_lists(a).into_iter().map(|l| Arc::new(l.clone())).collect()).collect();
        self.has_replace = self.abilities.iter().any(|a| matches!(a, Ability::Replace { .. }));
        self.zone_triggers = self.abilities.iter().any(|a| matches!(a, Ability::Triggered { from: Some(z), .. } if *z != Zone::Battlefield && *z != Zone::Stack));
    }

    pub fn is_legendary(&self) -> bool {
        self.supertypes.contains(&Supertype::Legendary)
    }
    pub fn has_type(&self, t: CardType) -> bool {
        self.types.has(t)
    }
    pub fn is_permanent(&self) -> bool {
        self.types.is_permanent()
    }
}

#[derive(Debug, Clone)]
pub struct CompileError {
    pub card: String,
    pub message: String,
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.card, self.message)
    }
}

#[derive(Default, Debug)]
pub struct CardDb {
    pub defs: Vec<Def>,
    by_name: HashMap<String, DefId>,
    /// Synthetic definitions made by text-changing effects (`textchange`); ids start after `defs`. Leaked: they live as long as the process.
    extra: std::sync::RwLock<Vec<&'static Def>>,
    extra_cache: std::sync::RwLock<HashMap<(u32, String), DefId>>,
}

impl CardDb {
    /// Compile a list of card definitions. Errors are collected for every broken card (so CI reports all of them).
    pub fn build(cards: Vec<CardDef>) -> Result<Arc<CardDb>, Vec<CompileError>> {
        let mut db = CardDb::default();
        let mut errors = vec![];
        for c in cards {
            let name = c.name.clone();
            if let Err(e) = db.add(c, true) {
                errors.push(CompileError { card: name, message: e });
            }
        }
        // token names referenced by effects must exist
        for d in &db.defs {
            for t in referenced_tokens(&d.abilities) {
                if !db.by_name.contains_key(&t) {
                    errors.push(CompileError { card: d.name.clone(), message: format!("unknown token '{t}'") });
                }
            }
        }
        if errors.is_empty() {
            Ok(Arc::new(db))
        } else {
            Err(errors)
        }
    }

    /// Compile one card. Takes the card by value so its abilities move into the `Def` instead of being cloned (the database
    /// holds one `Def` per card face; a clone of every card's ability tree is gigabytes at this scale).
    fn add(&mut self, mut c: CardDef, front: bool) -> Result<DefId, String> {
        if c.abilities.iter().any(|a| matches!(a, Ability::Saga { .. })) {
            c = expand_saga(&c);
        }
        if c.abilities.iter().any(|a| matches!(a, Ability::Suspend { .. } | Ability::EntersPrepared | Ability::Devour { .. } | Ability::Bestow { .. } | Ability::Encore { .. } | Ability::Prototype { .. } | Ability::Unleash | Ability::StartYourEngines | Ability::Foretell { .. } | Ability::Plot { .. } | Ability::Warp { .. } | Ability::Unearth { .. } | Ability::Ninjutsu { .. } | Ability::Madness { .. } | Ability::Renown { .. } | Ability::Morph { .. } | Ability::Emerge { .. } | Ability::Offering { .. } | Ability::Sneak { .. })) {
            c = expand_suspend(&c);
        }
        if c.keywords.contains(&Keyword::Rebound) {
            c = expand_rebound(&c);
        }
        if c.types.contains(&CardType::Battle) && c.subtypes.iter().any(|s| s == "Siege") && !c.abilities.iter().any(|a| matches!(a, Ability::EntersChoose { kind: ChooseKind::Protector, .. })) {
            c.abilities.push(Ability::EntersChoose { kind: ChooseKind::Protector, except: vec![], cap: None });
        }
        if c.abilities.iter().any(|a| matches!(a, Ability::EntersUnderOpponent)) && !c.abilities.iter().any(|a| matches!(a, Ability::EntersChoose { kind: ChooseKind::UnderOpponent, .. })) {
            c.abilities.push(Ability::EntersChoose { kind: ChooseKind::UnderOpponent, except: vec![], cap: None });
        }
        if c.keywords.iter().any(|k| keyword_expansion(*k).is_some()) {
            c = expand_keywords(&c);
        }
        // a back face may share its name with an unrelated card (prepare spells are named after real spells): the standalone card wins the name
        let original_entwine = c.abilities.iter().any(|a| matches!(a, Ability::Entwine { .. }));
        if original_entwine {
            for a in c.abilities.iter_mut() {
                if let Ability::Entwine { cost } = a {
                    *a = Ability::Kicker { cost: cost.clone(), multi: false, buyback: false };
                }
            }
        }
        // Room: one permanent with both doors' abilities, each gated by its door; casting the back half is an alternative cost
        if front && c.layout == crate::dsl::Layout::Room && c.back.is_some() {
            let (back_abilities, back_cost) = {
                let back = c.back.as_ref().unwrap();
                (back.abilities.clone(), back.mana_cost.clone())
            };
            let gate = |a: &Ability, door: u8| -> Ability {
                let g = Condition::DoorUnlocked { door };
                let and = |old: &Option<Condition>| Some(match old {
                    Some(o) => Condition::All { cs: vec![g.clone(), o.clone()] },
                    None => g.clone(),
                });
                let mut a = a.clone();
                match &mut a {
                    Ability::Static { condition, .. } | Ability::Triggered { condition, .. } | Ability::Activated { condition, .. } | Ability::Replace { condition, .. } => *condition = and(condition),
                    _ => {}
                }
                a
            };
            let own = std::mem::take(&mut c.abilities);
            c.abilities = own.iter().map(|a| gate(a, 1)).collect();
            c.abilities.extend(back_abilities.iter().map(|a| gate(a, 2)));
            c.abilities.push(Ability::AlternativeCost { cost: vec![Cost::Mana { cost: back_cost }], from: None, exile_after: false, condition: None, effects: None, targets: None, flash: false });
        }
        if front && self.by_name.get(&c.name).map_or(false, |d| self.defs[d.0 as usize].is_front) {
            return Err("duplicate card name".into());
        }
        let id = DefId(self.defs.len() as u32);
        let cost = ManaCost::parse(&c.mana_cost)?;
        let mut kw = KeywordSet::default();
        for k in &c.keywords {
            kw.add(*k);
        }
        let colors = match &c.colors {
            Some(cs) => ColorSet::of(cs),
            None => cost.colors(),
        };
        let mut def = Def {
            id,
            name: c.name.clone(),
            mana_value: cost.mana_value(0),
            split_mv: 0,
            cost,
            types: TypeSet::of(&c.types),
            supertypes: c.supertypes.iter().copied().collect(),
            subtypes: c.subtypes.iter().map(|s| Sym::new(s)).collect(),
            colors,
            identity: identity_of(&c, colors),
            power: c.power,
            toughness: c.toughness,
            loyalty: c.loyalty,
            defense: c.defense,
            keywords: kw,
            abilities: Vec::new(), // moved in below, after the fields that read `c.abilities`
            triggers: SmallVec::new(),
            statics: SmallVec::new(),
            activated: SmallVec::new(),
            mana_abilities: SmallVec::new(),
            trigger_mask: 0,
            programs: vec![],
            other_face: None,
            has_replace: c.abilities.iter().any(|a| matches!(a, Ability::Replace { .. })),
            token: c.token,
            no_mana_cost: c.mana_cost.trim().is_empty() && !c.token && !c.types.contains(&CardType::Land) && !c.types.is_empty(),
            oracle_text: c.oracle_text.clone(),
            is_front: front,
            zone_triggers: c.abilities.iter().any(|a| matches!(a, Ability::Triggered { from: Some(z), .. } if *z != Zone::Battlefield && *z != Zone::Stack)),
            adventure: front && c.back.is_some() && c.layout == crate::dsl::Layout::Adventure,
            saga_max: c.abilities.iter().filter_map(|a| if let Ability::Triggered { trigger: Trigger::SagaChapter { chapter }, .. } = a { Some(*chapter) } else { None }).max().unwrap_or(0),
            entwine: original_entwine,
            any_player: c.abilities.iter().any(|a| matches!(a, Ability::AnyPlayerMayActivate | Ability::OnlyOpponentsMayActivate)),
            lki_snapshot: c.abilities.iter().any(|a| matches!(a, Ability::LkiSnapshot)),
            engines: c.abilities.iter().any(|a| matches!(a, Ability::Triggered { effects, .. } if effects.iter().any(|e| matches!(e, Effect::StartEngines { .. })))),
            omen: front && c.back.is_some() && c.layout == crate::dsl::Layout::Omen,
            opponents_only: c.abilities.iter().any(|a| matches!(a, Ability::OnlyOpponentsMayActivate)),
            enchanted_ctl_ability: c.abilities.iter().position(|a| matches!(a, Ability::EnchantedControllerMayActivate)).and_then(|k| c.abilities.iter().enumerate().skip(k + 1).find(|(_, a)| matches!(a, Ability::Activated { .. })).map(|(i, _)| i as u16)),
            prepare: front && c.back.is_some() && c.layout == crate::dsl::Layout::Prepare,
            room: front && c.back.is_some() && c.layout == crate::dsl::Layout::Room,
            face_castable: front && c.back.is_some() && c.layout != crate::dsl::Layout::Transform && c.layout != crate::dsl::Layout::Prepare && c.layout != crate::dsl::Layout::Room,
            transforming: front
                && c.back.is_some()
                && (c.layout == crate::dsl::Layout::Transform
                    // a modal DFC whose own text says "Transform this" (Miles Morales, Peter Parker, King T'Challa) does transform
                    || (c.layout == crate::dsl::Layout::Modal && c.abilities.iter().any(|a| serde_json::to_string(a).map_or(false, |j| j.contains("\"type\":\"Transform\""))))),
            display_name: display_name_of(&c),
        };
        def.abilities = std::mem::take(&mut c.abilities);
        for (i, a) in def.abilities.iter().enumerate() {
            let i = i as u16;
            match a {
                Ability::Triggered { trigger, .. } => {
                    def.triggers.push(i);
                    def.trigger_mask |= crate::triggers::trigger_kind_mask(trigger);
                }
                Ability::Static { .. } => def.statics.push(i),
                Ability::Activated { .. } => {
                    def.activated.push(i);
                    if is_mana_ability(a) {
                        def.mana_abilities.push(i);
                    }
                }
                _ => {}
            }
        }
        validate_abilities(&def.abilities, &c)?;
        def.programs = def
            .abilities
            .iter()
            .map(|a| ability_effect_lists(a).into_iter().map(|l| Arc::new(l.clone())).collect())
            .collect();
        if front || !self.by_name.contains_key(&c.name) {
            self.by_name.insert(c.name.clone(), id);
        }
        self.defs.push(def);
        let is_split = c.layout == crate::dsl::Layout::Split;
        if let Some(back) = c.back.take() {
            let bid = self.add(*back, false)?;
            if is_split {
                // Rule 708.4 / 202.3: outside the stack a split card's mana value is the total of both halves.
                let total = self.defs[id.0 as usize].mana_value + self.defs[bid.0 as usize].mana_value;
                self.defs[id.0 as usize].split_mv = total;
                self.defs[bid.0 as usize].split_mv = total;
            }
            self.defs[id.0 as usize].other_face = Some(bid);
            self.defs[bid.0 as usize].other_face = Some(id);
            let tr = self.defs[id.0 as usize].transforming;
            self.defs[bid.0 as usize].transforming = tr;
        }
        Ok(id)
    }

    pub fn find(&self, name: &str) -> Option<DefId> {
        self.by_name.get(name).copied().or_else(|| {
            // "Front // Back" -> front name
            name.split(" // ").next().and_then(|n| self.by_name.get(n).copied())
        })
    }

    pub fn def(&self, id: DefId) -> &Def {
        match self.defs.get(id.0 as usize) {
            Some(d) => d,
            None => self.extra.read().unwrap()[id.0 as usize - self.defs.len()],
        }
    }

    /// The definition `base` with text changes applied (made once per distinct combination).
    pub fn text_def(&self, base: DefId, changes: &[crate::textchange::TextChange]) -> DefId {
        if changes.is_empty() {
            return base;
        }
        let key = (base.0, format!("{changes:?}"));
        if let Some(id) = self.extra_cache.read().unwrap().get(&key) {
            return *id;
        }
        let mut d = crate::textchange::transformed(self, self.def(base), changes);
        let mut extra = self.extra.write().unwrap();
        let id = DefId((self.defs.len() + extra.len()) as u32);
        d.id = id;
        extra.push(Box::leak(Box::new(d)));
        self.extra_cache.write().unwrap().insert(key, id);
        id
    }

    pub fn len(&self) -> usize {
        self.defs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }
}

pub fn is_mana_ability(a: &Ability) -> bool {
    match a {
        Ability::Activated { targets, effects, loyalty, cost, .. } => {
            loyalty.is_none()
                && targets.is_empty()
                // abilities that move cards to or from a library are never mana abilities (Millikin's "mill a card" cost)
                && (!cost.iter().any(|c| matches!(c, Cost::Mill { .. })) || effects.iter().any(|e| matches!(e, Effect::AddMana { mana, .. } if mana.iter().any(|m| matches!(m, ManaProduce::MilledSymbols)))))
                // Parley (Selvala): revealing library cards and drawing makes it a stack ability
                && !effects.iter().any(|e| matches!(e, Effect::RevealTop { .. }))
                && effects.iter().any(|e| matches!(e, Effect::AddMana { .. }))
        }
        _ => false,
    }
}

fn walk_effects<'a>(effs: &'a [Effect], f: &mut impl FnMut(&'a Effect)) {
    for e in effs {
        f(e);
        match e {
            Effect::If { then, otherwise, .. } => {
                walk_effects(then, f);
                walk_effects(otherwise, f);
            }
            Effect::May { effects, .. } | Effect::MayPay { effects, .. } | Effect::Unless { effects, .. } | Effect::Repeat { effects, .. }
            | Effect::ForEach { effects, .. } | Effect::ForEachPlayer { effects, .. } | Effect::Delayed { effects, .. } | Effect::Choose { then: effects, .. } | Effect::ChooseWithin { then: effects, .. } | Effect::EachPlayerChooses { then: effects, .. } => walk_effects(effects, f),
            _ => {}
        }
    }
}

fn referenced_tokens(abilities: &[Ability]) -> Vec<String> {
    let mut out = vec![];
    for a in abilities {
        for effs in ability_effect_lists(a) {
            walk_effects(effs, &mut |e| {
                if let Effect::CreateToken { token, .. } = e {
                    out.push(token.clone());
                }
            });
        }
    }
    out
}

/// All effect lists directly owned by an ability (each mode of a modal spell counts separately).
pub fn ability_effect_lists(a: &Ability) -> Vec<&Vec<Effect>> {
    match a {
        Ability::AlternativeCost { effects: Some(effects), .. } => vec![effects],
        Ability::Spell { effects, .. }
        | Ability::Activated { effects, .. }
        | Ability::Triggered { effects, .. }
        | Ability::Replacement { effects, .. } => vec![effects],
        Ability::Modal { modes, .. } => modes.iter().map(|m| &m.effects).collect(),
        _ => vec![],
    }
}

fn max_target_idx(effs: &[Effect]) -> Option<u8> {
    let mut max: Option<u8> = None;
    let mut see = |s: &Sel| {
        if let Sel::Target { idx } = s {
            max = Some(max.map_or(*idx, |m| m.max(*idx)));
        }
    };
    walk_effects(effs, &mut |e| match e {
        Effect::DealDamage { to, .. } => see(to),
        Effect::Destroy { what, .. }
        | Effect::Exile { what, .. }
        | Effect::Move { what, .. }
        | Effect::Bounce { what }
        | Effect::CounterSpell { what, .. }
        | Effect::Pump { what, .. }
        | Effect::SetBasePT { what, .. }
        | Effect::GrantKeyword { what, .. }
        | Effect::RemoveKeyword { what, .. }
        | Effect::RemoveProtection { what, .. }
        | Effect::FaceDownAs { what, .. }
        | Effect::AddCounters { what, .. }
        | Effect::ModularCounters { what, .. }
        | Effect::RemoveCounters { what, .. }
        | Effect::Tap { what }
        | Effect::Untap { what }
        | Effect::GainControl { what, .. }
        | Effect::Transform { what }
        | Effect::Regenerate { what, .. }
        | Effect::CantRegenerate { what }
        | Effect::Reveal { what }
        | Effect::CopyPermanent { what, .. }
        | Effect::CopySpell { what, .. } => see(what),
        Effect::Draw { who, .. }
        | Effect::GainLife { who, .. }
        | Effect::LoseLife { who, .. }
        | Effect::Discard { who, .. }
        | Effect::Mill { who, .. }
        | Effect::ExileTopOfLibrary { who, .. } => see(who),
        Effect::Fight { a, b } | Effect::FightEach { a, b } | Effect::Bite { a, b } => {
            see(a);
            see(b)
        }
        Effect::Attach { what, to } => {
            see(what);
            see(to)
        }
        _ => {}
    });
    max
}

fn validate_abilities(abilities: &[Ability], c: &CardDef) -> Result<(), String> {
    for (i, a) in abilities.iter().enumerate() {
        if matches!(a, Ability::Replacement { .. }) {
            return Err("`Replacement` is not implemented; use the typed `Replace` ability".into());
        }
        // a mode of an in-effect ChooseModes can't carry targets (they would silently never be chosen)
        if let Ok(v) = serde_json::to_value(a) {
            fn bad_modes(v: &serde_json::Value) -> bool {
                match v {
                    serde_json::Value::Object(m) => {
                        if m.get("type").and_then(|t| t.as_str()) == Some("ChooseModes") {
                            if let Some(serde_json::Value::Array(modes)) = m.get("modes") {
                                if modes.iter().any(|x| x.get("targets").and_then(|t| t.as_array()).map_or(false, |t| !t.is_empty())) {
                                    return true;
                                }
                            }
                        }
                        m.values().any(bad_modes)
                    }
                    serde_json::Value::Array(a) => a.iter().any(bad_modes),
                    _ => false,
                }
            }
            if bad_modes(&v) {
                return Err(format!("ability {i}: a mode of ChooseModes has targets; targets of modes inside effects are never chosen (use Reflexive inside the mode)"));
            }
        }
        match a {
            Ability::Spell { targets, effects } | Ability::Activated { targets, effects, .. }
            | Ability::Triggered { targets, effects, .. } => {
                if let Some(m) = max_target_idx(effects) {
                    if m as usize >= targets.len() {
                        return Err(format!("ability {i} refers to target {m} but declares {} targets", targets.len()));
                    }
                }
            }
            Ability::Modal { choose_min, choose_max, modes, allow_repeat, .. } => {
                if modes.is_empty() || choose_min > choose_max || (!*allow_repeat && (*choose_max as usize) > modes.len()) {
                    return Err(format!("ability {i}: bad mode counts"));
                }
                for m in modes {
                    if let Some(x) = max_target_idx(&m.effects) {
                        if x as usize >= m.targets.len() {
                            return Err(format!("mode '{}' refers to undeclared target {x}", m.text));
                        }
                    }
                }
            }
            _ => {}
        }
        if let Ability::Activated { cost, .. } = a {
            for part in cost {
                if let Cost::Mana { cost } = part {
                    ManaCost::parse(cost)?;
                }
            }
        }
    }
    if c.types.is_empty() && !c.token {
        return Err("a card needs at least one type".into());
    }
    if c.types.contains(&CardType::Creature) && !c.token && (c.power.is_none() || c.toughness.is_none()) {
        // allowed only with a characteristic-defining ability
        let cda = abilities.iter().any(|a| matches!(a, Ability::Static { effect: StaticEffect::CdaPT { .. }, .. }));
        if !cda {
            return Err("creature without power/toughness".into());
        }
    }
    Ok(())
}

/// Color identity (CR 903.4): the card's colors plus every colored mana symbol in its text, on both faces.
fn identity_of(c: &CardDef, colors: ColorSet) -> ColorSet {
    let mut id = colors;
    fn symbols(text: &str, id: &mut ColorSet) {
        let mut rest = text;
        while let Some(i) = rest.find('{') {
            if let Some(j) = rest[i..].find('}') {
                if let Ok(mc) = ManaCost::parse(&rest[i..i + j + 1]) {
                    id.0 |= mc.colors().0;
                }
                rest = &rest[i + j + 1..];
            } else {
                break;
            }
        }
    }
    symbols(&c.oracle_text, &mut id);
    if let Some(cs) = &c.colors {
        id.0 |= ColorSet::of(cs).0;
    }
    if let Some(b) = &c.back {
        id.0 |= identity_of(b, ColorSet::NONE).0;
        symbols(&b.mana_cost, &mut id);
    }
    id
}

/// Tokens may be named with their size (and notable abilities) to keep several variants apart; players see only the creature name.
/// Turn `Saga` chapters into ordinary abilities: one lore counter per precombat main phase and one trigger per chapter number.
fn expand_saga(c: &CardDef) -> CardDef {
    let mut out = c.clone();
    let mut extra = vec![];
    let mut max = 0u8;
    for a in &c.abilities {
        if let Ability::Saga { chapters } = a {
            for ch in chapters {
                for n in &ch.numbers {
                    max = max.max(*n);
                    extra.push(Ability::Triggered {
                        trigger: Trigger::SagaChapter { chapter: *n },
                        targets: ch.targets.clone(),
                        effects: ch.effects.clone(),
                        optional: false,
                        condition: None,
                        from: None,
                        once_per_turn: false, limit_per_turn: 0, once_ever: false,
        batched: false,
        as_enters: false, no_recheck: false,
                    });
                }
            }
        }
    }
    extra.push(Ability::Triggered {
        trigger: Trigger::BeginningOfStep { step: Step::Main1, whose: PlayerFilter::You },
        targets: vec![],
        effects: vec![Effect::AddCounters { what: Sel::This, kind: CounterKind::Lore, n: Amount::Fixed { n: 1 } }],
        optional: false,
        condition: None,
        from: None,
        once_per_turn: false, limit_per_turn: 0, once_ever: false,
        batched: false,
        as_enters: false, no_recheck: false,
    });
    out.abilities.extend(extra);
    out
}

/// Rebound: a spell cast from hand is exiled as it resolves, and cast free (may) at the beginning of your next upkeep.
fn expand_rebound(c: &CardDef) -> CardDef {
    let mut out = c.clone();
    let mut any = false;
    for a in out.abilities.iter_mut() {
        if let Ability::Spell { effects, .. } = a {
            effects.push(rebound_effect());
            any = true;
        }
    }
    // a rebound spell with no other spell ability still exiles itself as it resolves (Into the Time Vortex)
    if !any && !out.abilities.iter().any(|a| matches!(a, Ability::Modal { .. })) {
        out.abilities.push(Ability::Spell { targets: vec![], effects: vec![rebound_effect()] });
    }
    out
}

/// The effect a spell with rebound ends with (also appended at resolution to spells that are granted rebound: Cast Through Time).
pub(crate) fn rebound_effect() -> Effect {
    Effect::If {
        cond: Condition::CastFrom { zone: Zone::Hand },
        then: vec![
            Effect::Move { what: Sel::This, to: Zone::Exile, top_of_library: false, tapped: false, under_your_control: false, remember: true, transformed: false, from: None, discard: false, position: None, position_x: false, ordered: false, bottom_ordered: false, order_chooser: None, attacking: false, choose_host: false, host_creature: false, host_only: None, host_chooser: None, non_aura_if_no_host: false, face_down: false, under_control_of: None, random_order: false, enters_bare: false },
            Effect::Delayed { at: DelayedWhen::NextYourUpkeep, effects: vec![Effect::CastFree { what: Sel::Remembered, may: true, decline_to_bottom: false, decline_to_hand: false, exile_after: false, copy: false, pay: false, by: None, any_mana: false, lands: false, alt: None, extra_mana: None, otherwise: vec![], suspend: false, lock_spells_after: false, on_cast: vec![], after_cast: vec![], bottom_after: false, copies: None, face_choice: false }] },
        ],
        otherwise: vec![],
    }
}

/// Turn `Suspend` into its two real abilities.
fn expand_suspend(c: &CardDef) -> CardDef {
    let mut out = c.clone();
    let instant = c.types.contains(&CardType::Instant);
    for a in &c.abilities {
        if let Ability::Renown { n } = a {
            out.abilities.push(Ability::Triggered {
                trigger: Trigger::CombatDamageToPlayer { filter: (Filter { this: true, ..Default::default() }).into(), batched: false, or_battle: false, opponents_count: false, per_controller: false },
                targets: vec![],
                effects: vec![Effect::AddCounters { what: Sel::This, kind: CounterKind::PlusOnePlusOne, n: Amount::Fixed { n: *n as i32 } }, Effect::BecomeRenowned],
                optional: false,
                condition: Some(Condition::Not { c: Box::new(Condition::Renowned) }),
                from: None,
                once_per_turn: false, limit_per_turn: 0, once_ever: false,
                batched: false,
                as_enters: false, no_recheck: false,
            });
            continue;
        }
        if let Ability::Sneak { cost } = a {
            let idx = out.abilities.iter().filter(|x| matches!(x, Ability::AlternativeCost { .. })).count() as u8;
            out.abilities.push(Ability::AlternativeCost {
                cost: vec![
                    Cost::Mana { cost: cost.clone() },
                    Cost::ReturnToHand { filter: (Filter { types: vec![CardType::Creature], controller: PlayerFilter::You, attacking: true, unblocked: true, ..Default::default() }).into(), n: 1 },
                ],
                from: None,
                exile_after: false,
                condition: Some(Condition::InStep { step: Step::DeclareBlockers }),
                effects: None,
                targets: None,
                flash: true,
            });
            out.abilities.push(Ability::Triggered {
                trigger: Trigger::EntersBattlefield { filter: (Filter { this: true, ..Default::default() }).into() },
                targets: vec![],
                effects: vec![Effect::BecomeAttacking { what: Sel::This, defender_among: None }],
                optional: false,
                condition: Some(Condition::CastWithAlt { index: Some(idx) }),
                from: None,
                once_per_turn: false, limit_per_turn: 0, once_ever: false,
                batched: false,
                as_enters: false, no_recheck: false,
            });
            continue;
        }
        if let Ability::Emerge { cost } = a {
            out.abilities.push(Ability::AlternativeCost {
                cost: vec![Cost::SacrificeReduceByManaValue { filter: (Filter { types: vec![CardType::Creature], controller: PlayerFilter::You, ..Default::default() }).into() }, Cost::Mana { cost: cost.clone() }],
                from: None,
                exile_after: false,
                condition: None,
                effects: None,
                targets: None,
                flash: false,
            });
            continue;
        }
        if let Ability::Offering { subtype } = a {
            out.abilities.push(Ability::AlternativeCost {
                cost: vec![Cost::SacrificeReduceByManaCost { filter: (Filter { types: vec![CardType::Creature], subtypes: vec![subtype.clone()], controller: PlayerFilter::You, ..Default::default() }).into() }, Cost::Mana { cost: c.mana_cost.clone() }],
                from: None,
                exile_after: false,
                condition: None,
                effects: None,
                targets: None,
                flash: true,
            });
            continue;
        }
        if let Ability::Prototype { cost, .. } = a {
            out.abilities.push(Ability::AlternativeCost { cost: vec![Cost::Mana { cost: cost.clone() }], from: None, exile_after: false, condition: None, effects: None, targets: None, flash: false });
            // keep the marker ability itself (no effect) so the layers can find the prototype values
            out.abilities.push(a.clone());
            continue;
        }
        if let Ability::Encore { cost } = a {
            out.abilities.push(Ability::Activated {
                cost: vec![Cost::Mana { cost: cost.clone() }, Cost::ExileSelf],
                targets: vec![],
                effects: vec![Effect::ForEach {
                    turn_order: false,
                    objects: Sel::EachOpponent,
                    effects: vec![
                        Effect::CopyPermanent { what: Sel::This, n: Amount::Fixed { n: 1 }, except: vec![CopyExcept::AddKeyword { keyword: Keyword::Haste }], tapped: false, attacking: false, by: None, attach_to: None, attack_at: None },
                        Effect::GrantStatic { what: Sel::Remembered, effect: StaticEffect::MustAttack { affects: (Filter::default()).into() }, duration: Duration::Forever, dynamic: false },
                        Effect::Delayed {
                            at: DelayedWhen::NextEndStep,
                            effects: vec![Effect::Move { what: Sel::Remembered, to: Zone::Graveyard, top_of_library: false, tapped: false, under_your_control: false, remember: false, transformed: false, from: Some(Zone::Battlefield), discard: false, position: None, position_x: false, ordered: false, bottom_ordered: false, order_chooser: None, attacking: false, choose_host: false, host_creature: false, host_only: None, host_chooser: None, non_aura_if_no_host: false, face_down: false, under_control_of: None, random_order: false, enters_bare: false }],
                        },
                    ],
                }],
                timing: Timing::Sorcery,
                loyalty: None,
                limit_per_turn: None,
                once: false,
                condition: None,
                from: Some(Zone::Graveyard),
                power_up: false, special_action: false,
            });
            continue;
        }
        if let Ability::Bestow { cost } = a {
            // cast as an Aura: its own target (a creature) and an empty program; the attaching is done when the spell resolves
            out.abilities.push(Ability::AlternativeCost {
                cost: vec![Cost::Mana { cost: cost.clone() }],
                from: None,
                exile_after: false,
                condition: None,
                effects: Some(vec![Effect::Nothing]),
                targets: Some(vec![TargetSpec { what: TargetKind::Permanent { filter: (Filter { types: vec![CardType::Creature], ..Default::default() }).into() }, min: 1, max: 1, label: "target creature".into(), max_x: false, exact_x: false, max_amount: None, max_total_mv: None, max_total_power: None, chooser: None, distinct: false, when_kicked: None, per_opponent: false, per_player: false, same_graveyard: false, distinct_by: None, random: false, only_controller: None, distinct_modes: false, corrupted: false }]),
                flash: false,
            });
            continue;
        }
        if let Ability::Devour { n } = a {
            out.abilities.push(Ability::Triggered {
                trigger: Trigger::EntersBattlefield { filter: (Filter { this: true, ..Default::default() }).into() },
                targets: vec![],
                effects: vec![
                    Effect::SacrificeAny { filter: (Filter { types: vec![CardType::Creature], other: true, ..Default::default() }).into(), by: None },
                    Effect::AddCounters { what: Sel::This, kind: CounterKind::PlusOnePlusOne, n: Amount::Times { a: Box::new(Amount::Fixed { n: *n as i32 }), b: Box::new(Amount::RememberedCount) } },
                ],
                optional: false,
                condition: None,
                from: None,
                once_per_turn: false, limit_per_turn: 0, once_ever: false,
                batched: false,
                as_enters: true, no_recheck: false,
            });
            continue;
        }
        if let Ability::Unleash = a {
            out.abilities.push(Ability::Triggered {
                trigger: Trigger::EntersBattlefield { filter: (Filter { this: true, ..Default::default() }).into() },
                targets: vec![],
                effects: vec![Effect::AddCounters { what: Sel::This, kind: CounterKind::PlusOnePlusOne, n: Amount::Fixed { n: 1 } }],
                optional: true,
                condition: None,
                from: None,
                once_per_turn: false, limit_per_turn: 0, once_ever: false,
                batched: false,
                as_enters: false, no_recheck: false,
            });
            out.abilities.push(Ability::Static {
                effect: StaticEffect::CantBlock { affects: (Filter { this: true, ..Default::default() }).into() },
                condition: Some(Condition::SourceHasCounter { kind: CounterKind::PlusOnePlusOne, at_least: 1 }),
                from: None,
            });
            continue;
        }
        if let Ability::StartYourEngines = a {
            out.abilities.push(Ability::Triggered {
                trigger: Trigger::EntersBattlefield { filter: (Filter { this: true, ..Default::default() }).into() },
                targets: vec![],
                effects: vec![Effect::StartEngines { who: Sel::You }],
                optional: false,
                condition: None,
                from: None,
                once_per_turn: false, limit_per_turn: 0, once_ever: false,
                batched: false,
                as_enters: false, no_recheck: false,
            });
            continue;
        }
        if let Ability::EntersPrepared = a {
            out.abilities.push(Ability::Triggered {
                trigger: Trigger::EntersBattlefield { filter: (Filter { this: true, ..Default::default() }).into() },
                targets: vec![],
                effects: vec![Effect::BecomePrepared { what: Sel::This }],
                optional: false,
                condition: None,
                from: None,
                once_per_turn: false, limit_per_turn: 0, once_ever: false,
                batched: false,
                as_enters: false, no_recheck: false,
            });
            continue;
        }
        if let Ability::Foretell { cost } = a {
            out.abilities.push(Ability::Activated {
                cost: vec![Cost::Mana { cost: "{2}".into() }],
                targets: vec![],
                effects: vec![
                    Effect::Move { what: Sel::This, to: Zone::Exile, top_of_library: false, tapped: false, under_your_control: false, remember: false, transformed: false, from: None, discard: false, position: None, position_x: false, ordered: false, bottom_ordered: false, order_chooser: None, attacking: false, choose_host: false, host_creature: false, host_only: None, host_chooser: None, non_aura_if_no_host: false, face_down: false, under_control_of: None, random_order: false, enters_bare: false },
                    Effect::MarkExiled { mark: ExileMark::Foretold },
                ],
                timing: Timing::Instant,
                loyalty: None,
                limit_per_turn: None,
                once: false,
                condition: Some(Condition::MayForetellNow),
                from: Some(Zone::Hand),
                power_up: false, special_action: false,
            });
            out.abilities.push(Ability::AlternativeCost { cost: vec![Cost::Mana { cost: cost.clone() }], from: Some(Zone::Exile), exile_after: false, condition: Some(Condition::ExileMarked { mark: ExileMark::Foretold }), effects: None, targets: None, flash: false });
            continue;
        }
        if let Ability::Plot { cost } = a {
            out.abilities.push(Ability::Activated {
                cost: vec![Cost::Mana { cost: cost.clone() }],
                targets: vec![],
                effects: vec![
                    Effect::Move { what: Sel::This, to: Zone::Exile, top_of_library: false, tapped: false, under_your_control: false, remember: false, transformed: false, from: None, discard: false, position: None, position_x: false, ordered: false, bottom_ordered: false, order_chooser: None, attacking: false, choose_host: false, host_creature: false, host_only: None, host_chooser: None, non_aura_if_no_host: false, face_down: false, under_control_of: None, random_order: false, enters_bare: false },
                    Effect::MarkExiled { mark: ExileMark::Plotted },
                ],
                timing: Timing::Sorcery,
                loyalty: None,
                limit_per_turn: None,
                once: false,
                condition: None,
                from: Some(Zone::Hand),
                power_up: false, special_action: false,
            });
            out.abilities.push(Ability::AlternativeCost { cost: vec![], from: Some(Zone::Exile), exile_after: false, condition: Some(Condition::All { cs: vec![Condition::ExileMarked { mark: ExileMark::Plotted }, Condition::SorcerySpeed] }), effects: None, targets: None, flash: false });
            continue;
        }
        if let Ability::Warp { cost } = a {
            let idx = out.abilities.iter().filter(|x| matches!(x, Ability::AlternativeCost { .. })).count() as u8;
            out.abilities.push(Ability::AlternativeCost { cost: vec![Cost::Mana { cost: cost.clone() }], from: None, exile_after: false, condition: None, effects: None, targets: None, flash: false });
            out.abilities.push(a.clone()); // marker: casting with this alternative cost "warps" the spell (void)
            out.abilities.push(Ability::Triggered {
                trigger: Trigger::EntersBattlefield { filter: (Filter { this: true, ..Default::default() }).into() },
                targets: vec![],
                effects: vec![Effect::Delayed {
                    at: DelayedWhen::NextEndStep,
                    effects: vec![
                        Effect::Move { what: Sel::This, to: Zone::Exile, top_of_library: false, tapped: false, under_your_control: false, remember: false, transformed: false, from: Some(Zone::Battlefield), discard: false, position: None, position_x: false, ordered: false, bottom_ordered: false, order_chooser: None, attacking: false, choose_host: false, host_creature: false, host_only: None, host_chooser: None, non_aura_if_no_host: false, face_down: false, under_control_of: None, random_order: false, enters_bare: false },
                        Effect::MarkExiled { mark: ExileMark::Warped },
                    ],
                }],
                optional: false,
                condition: Some(Condition::CastWithAlt { index: Some(idx) }),
                from: None,
                once_per_turn: false, limit_per_turn: 0, once_ever: false,
                batched: false,
                as_enters: false, no_recheck: false,
            });
            continue;
        }
        if let Ability::Morph { cost, megamorph, disguise, other_cost, face_up_counters } = a {
            // turning it face up: an activated ability that works while the permanent is face down
            out.abilities.push(Ability::Activated {
                cost: if cost.is_empty() { other_cost.clone() } else { std::iter::once(Cost::Mana { cost: cost.clone() }).chain(other_cost.iter().cloned()).collect() },
                targets: vec![],
                effects: vec![Effect::TurnFaceUp { megamorph: *megamorph, counters: *face_up_counters, what: None }],
                timing: Timing::Instant,
                loyalty: None,
                limit_per_turn: None,
                once: false,
                condition: Some(Condition::SourceFaceDown),
                from: None,
                power_up: false, special_action: false,
            });
            let _ = disguise;
            continue;
        }
        if let Ability::Madness { cost } = a {
            if cost.contains("{X}") || out.keywords.contains(&Keyword::Convoke) {
                // an {X} madness cost: cast through a never-offered alternative cost so X is chosen as the spell is cast and `CastWithAlt` can ask "madness cost was paid"
                let idx = out.abilities.iter().filter(|x| matches!(x, Ability::AlternativeCost { .. })).count() as u8;
                out.abilities.push(Ability::AlternativeCost { cost: vec![Cost::Mana { cost: cost.clone() }], from: Some(Zone::Graveyard), exile_after: false, condition: Some(Condition::AnyOf { cs: vec![] }), effects: None, targets: None, flash: false });
                out.abilities.push(Ability::Triggered {
                    trigger: Trigger::Discards { who: PlayerFilter::Any, this_only: true, filter: (Filter::default()).into(), cause_opponent: false },
                    targets: vec![],
                    effects: vec![Effect::CastFree { what: Sel::This, may: true, decline_to_bottom: false, decline_to_hand: false, exile_after: false, copy: false, pay: true, by: None, any_mana: false, lands: false, alt: Some(idx), extra_mana: None, otherwise: vec![], suspend: false, lock_spells_after: false, on_cast: vec![], after_cast: vec![], bottom_after: false, copies: None, face_choice: false }, Effect::Move { what: Sel::This, to: Zone::Graveyard, top_of_library: false, tapped: false, under_your_control: false, remember: false, transformed: false, from: Some(Zone::Exile), discard: false, position: None, position_x: false, ordered: false, bottom_ordered: false, order_chooser: None, attacking: false, choose_host: false, host_creature: false, host_only: None, host_chooser: None, non_aura_if_no_host: false, face_down: false, under_control_of: None, random_order: false, enters_bare: false }],
                    optional: false,
                    condition: None,
                    from: Some(Zone::Exile),
                    once_per_turn: false, limit_per_turn: 0, once_ever: false,
                    batched: false,
                    as_enters: false, no_recheck: false,
                });
                continue;
            }
            out.abilities.push(Ability::Triggered {
                trigger: Trigger::Discards { who: PlayerFilter::Any, this_only: true, filter: (Filter::default()).into(), cause_opponent: false },
                targets: vec![],
                effects: vec![Effect::MayPay {
                    payer: Sel::You,
                    cost: vec![Cost::Mana { cost: cost.clone() }],
                    effects: vec![Effect::CastFree { what: Sel::This, may: false, decline_to_bottom: false, decline_to_hand: false, exile_after: false, copy: false, pay: false, by: None, any_mana: false, lands: false, alt: None, extra_mana: None, otherwise: vec![], suspend: false, lock_spells_after: false, on_cast: vec![], after_cast: vec![], bottom_after: false, copies: None, face_choice: false }],
                }, Effect::Move { what: Sel::This, to: Zone::Graveyard, top_of_library: false, tapped: false, under_your_control: false, remember: false, transformed: false, from: Some(Zone::Exile), discard: false, position: None, position_x: false, ordered: false, bottom_ordered: false, order_chooser: None, attacking: false, choose_host: false, host_creature: false, host_only: None, host_chooser: None, non_aura_if_no_host: false, face_down: false, under_control_of: None, random_order: false, enters_bare: false }],
                optional: false,
                condition: None,
                from: Some(Zone::Exile),
                once_per_turn: false, limit_per_turn: 0, once_ever: false,
                batched: false,
                as_enters: false, no_recheck: false,
            });
            continue;
        }
        if let Ability::Ninjutsu { cost } = a {
            out.abilities.push(Ability::Activated {
                cost: vec![
                    Cost::Mana { cost: cost.clone() },
                    Cost::ReturnToHand { filter: (Filter { types: vec![CardType::Creature], controller: PlayerFilter::You, attacking: true, unblocked: true, ..Default::default() }).into(), n: 1 },
                ],
                targets: vec![],
                effects: vec![Effect::Move { what: Sel::This, to: Zone::Battlefield, top_of_library: false, tapped: true, under_your_control: false, remember: false, transformed: false, from: Some(Zone::Hand), discard: false, position: None, position_x: false, ordered: false, bottom_ordered: false, order_chooser: None, attacking: true, choose_host: false, host_creature: false, host_only: None, host_chooser: None, non_aura_if_no_host: false, face_down: false, under_control_of: None, random_order: false, enters_bare: false }],
                timing: Timing::Instant,
                loyalty: None,
                limit_per_turn: None,
                once: false,
                condition: Some(Condition::AnyOf { cs: vec![Condition::InStep { step: Step::DeclareBlockers }, Condition::InStep { step: Step::FirstStrikeDamage }, Condition::InStep { step: Step::CombatDamage }, Condition::InStep { step: Step::EndCombat }] }),
                from: Some(Zone::Hand),
                power_up: false, special_action: false,
            });
            continue;
        }
        if let Ability::Unearth { cost } = a {
            out.abilities.push(Ability::Activated {
                cost: vec![Cost::Mana { cost: cost.clone() }],
                targets: vec![],
                effects: vec![
                    Effect::Move { what: Sel::This, to: Zone::Battlefield, top_of_library: false, tapped: false, under_your_control: false, remember: false, transformed: false, from: Some(Zone::Graveyard), discard: false, position: None, position_x: false, ordered: false, bottom_ordered: false, order_chooser: None, attacking: false, choose_host: false, host_creature: false, host_only: None, host_chooser: None, non_aura_if_no_host: false, face_down: false, under_control_of: None, random_order: false, enters_bare: false },
                    Effect::GrantKeyword { what: Sel::This, keyword: Keyword::Haste, duration: Duration::Forever },
                    Effect::MarkExileOnLeave { what: None },
                    Effect::Delayed {
                        at: DelayedWhen::NextEndStep,
                        effects: vec![Effect::Move { what: Sel::This, to: Zone::Exile, top_of_library: false, tapped: false, under_your_control: false, remember: false, transformed: false, from: Some(Zone::Battlefield), discard: false, position: None, position_x: false, ordered: false, bottom_ordered: false, order_chooser: None, attacking: false, choose_host: false, host_creature: false, host_only: None, host_chooser: None, non_aura_if_no_host: false, face_down: false, under_control_of: None, random_order: false, enters_bare: false }],
                    },
                ],
                timing: Timing::Sorcery,
                loyalty: None,
                limit_per_turn: None,
                once: false,
                condition: None,
                from: Some(Zone::Graveyard),
                power_up: false, special_action: false,
            });
            continue;
        }
        let Ability::Suspend { n, cost, x } = a else { continue };
        out.abilities.push(Ability::Activated {
            cost: if *x { vec![Cost::Mana { cost: cost.clone() }, Cost::XMinimum { n: 1 }] } else { vec![Cost::Mana { cost: cost.clone() }] },
            targets: vec![],
            effects: vec![
                Effect::Move { what: Sel::This, to: Zone::Exile, top_of_library: false, tapped: false, under_your_control: false, remember: false, transformed: false, from: None, discard: false, position: None, position_x: false, ordered: false, bottom_ordered: false, order_chooser: None, attacking: false, choose_host: false, host_creature: false, host_only: None, host_chooser: None, non_aura_if_no_host: false, face_down: false, under_control_of: None, random_order: false, enters_bare: false },
                Effect::AddCounters { what: Sel::This, kind: CounterKind::Time, n: if *x { Amount::X } else { Amount::Fixed { n: *n as i32 } } },
            ],
            timing: if instant { Timing::Instant } else { Timing::Sorcery },
            loyalty: None,
            limit_per_turn: None,
                once: false,
            condition: None,
            from: Some(Zone::Hand),
            power_up: false, special_action: false,
        });
        // never offered on its own: lets an effect cast the card paying its suspend cost instead of its mana cost (`CastFree.suspend`; The Face of Boe)
        if !*x {
            out.abilities.push(Ability::AlternativeCost { cost: vec![Cost::Mana { cost: cost.clone() }], from: Some(Zone::Hand), exile_after: false, condition: Some(Condition::AnyOf { cs: vec![] }), effects: None, targets: None, flash: false });
        }
        let counters = Amount::CountersOn { of: Sel::This, kind: CounterKind::Time };
        out.abilities.push(Ability::Triggered {
            trigger: Trigger::BeginningOfStep { step: Step::Upkeep, whose: PlayerFilter::You },
            targets: vec![],
            effects: vec![Effect::RemoveCounters { what: Sel::This, kind: CounterKind::Time, n: Amount::Fixed { n: 1 } }],
            optional: false,
            condition: Some(Condition::Compare { a: counters.clone(), cmp: Cmp::Ge { n: 1 } }),
            from: Some(Zone::Exile),
            once_per_turn: false, limit_per_turn: 0, once_ever: false,
            batched: false,
            as_enters: false, no_recheck: false,
        });
        // when the last time counter is removed, however it was removed, its owner may cast it without paying its mana cost
        out.abilities.insert(0, Ability::Triggered {
            trigger: Trigger::CounterRemoved { filter: (Filter { this: true, ..Default::default() }).into(), kind: CounterKind::Time, last: true, by_you: false },
            targets: vec![],
            effects: vec![Effect::MarkSuspendHaste, Effect::CastFree { what: Sel::This, may: true, decline_to_bottom: false, decline_to_hand: false, exile_after: false, copy: false, pay: false, by: None, any_mana: false, lands: false, alt: None, extra_mana: None, otherwise: vec![], suspend: false, lock_spells_after: false, on_cast: vec![], after_cast: vec![], bottom_after: false, copies: None, face_choice: false }],
            optional: false,
            condition: None,
            from: Some(Zone::Exile),
            once_per_turn: false, limit_per_turn: 0, once_ever: false,
            batched: false,
            as_enters: false, no_recheck: false,
        });
    }
    out
}

/// Keywords that are really triggered abilities.
pub(crate) fn keyword_expansion(k: Keyword) -> Option<Ability> {
    let fx = |n: i32| Amount::Fixed { n };
    let trig = |trigger: Trigger, effects: Vec<Effect>, condition: Option<Condition>, from: Option<Zone>| Ability::Triggered { trigger, targets: vec![], effects, optional: false, condition, from, once_per_turn: false, limit_per_turn: 0, once_ever: false, batched: false, as_enters: false, no_recheck: false };
    let this_only = Filter { this: true, ..Default::default() };
    Some(match k {
        Keyword::Training => trig(
            Trigger::Attacks { filter: (this_only.clone()).into(), at_you: false, at_you_or_pw: false, at_enchanted: false, batched: false, player_only: false, at_opponent: false, pw_only: false, one_per_declaration: false },
            vec![Effect::AddCounters { what: Sel::This, kind: CounterKind::PlusOnePlusOne, n: fx(1) }, Effect::EmitKeywordAction { action: KeywordAction::Trains, what: None }],
            Some(Condition::Control {
                who: Sel::You,
                filter: (Filter { attacking: true, other: true, power_vs: Some(AmountCmp { op: CmpOp::Gt, amount: Box::new(Amount::PowerOf { of: Sel::This }) }), ..Default::default() }).into(),
                at_least: 1,
            }),
            None,
        ),
        Keyword::Annihilator1 => trig(
            Trigger::Attacks { filter: (this_only.clone()).into(), at_you: false, at_you_or_pw: false, at_enchanted: false, batched: false, player_only: false, at_opponent: false, pw_only: false, one_per_declaration: false },
            vec![Effect::Sacrifice { who: Sel::DefendingPlayer, filter: (Filter { controlled_by: Some(Box::new(Sel::DefendingPlayer)), ..Default::default() }).into(), n: fx(1) }],
            None,
            None,
        ),
        Keyword::Prowess => trig(
            Trigger::SpellCast { filter: (Filter { not_types: vec![CardType::Creature], ..Default::default() }).into(), by: PlayerFilter::You, or_copy: false },
            vec![Effect::Pump { what: Sel::This, power: fx(1), toughness: fx(1), duration: Duration::UntilEndOfTurn }],
            None,
            None,
        ),
        Keyword::Exalted => trig(
            Trigger::AttacksAlone { filter: (Filter { controller: PlayerFilter::You, ..Default::default() }).into(), player_only: false },
            vec![Effect::Pump { what: Sel::TriggerObject, power: fx(1), toughness: fx(1), duration: Duration::UntilEndOfTurn }],
            None,
            None,
        ),
        Keyword::Undying | Keyword::Persist => {
            let kind = if k == Keyword::Undying { CounterKind::PlusOnePlusOne } else { CounterKind::MinusOneMinusOne };
            trig(
                Trigger::Dies { filter: this_only.into() },
                vec![
                    Effect::Move { what: Sel::This, to: Zone::Battlefield, top_of_library: false, tapped: false, under_your_control: false, remember: true, transformed: false, from: Some(Zone::Graveyard), discard: false, position: None, position_x: false, ordered: false, bottom_ordered: false, order_chooser: None, attacking: false, choose_host: false, host_creature: false, host_only: None, host_chooser: None, non_aura_if_no_host: false, face_down: false, under_control_of: None, random_order: false, enters_bare: false },
                    Effect::AddCounters { what: Sel::Remembered, kind: if k == Keyword::Undying { CounterKind::PlusOnePlusOne } else { CounterKind::MinusOneMinusOne }, n: fx(1) },
                ],
                Some(Condition::Not { c: Box::new(Condition::SourceHasCounter { kind, at_least: 1 }) }),
                None,
            )
        }
        Keyword::Extort => trig(
            Trigger::SpellCast { filter: (Filter::default()).into(), by: PlayerFilter::You, or_copy: false },
            vec![Effect::MayPay {
                payer: Sel::You,
                cost: vec![Cost::Mana { cost: "{W/B}".into() }],
                effects: vec![Effect::LoseLife { who: Sel::EachOpponent, n: fx(1) }, Effect::GainLife { who: Sel::You, n: Amount::LifeLostThisWay }],
            }],
            None,
            None,
        ),
        Keyword::Decayed => trig(
            Trigger::Attacks { filter: (this_only.clone()).into(), at_you: false, at_you_or_pw: false, at_enchanted: false, batched: false, player_only: false, at_opponent: false, pw_only: false, one_per_declaration: false },
            vec![Effect::Delayed {
                at: DelayedWhen::EndOfCombat,
                effects: vec![Effect::Sacrifice { who: Sel::You, filter: (Filter { this: true, ..Default::default() }).into(), n: fx(1) }],
            }],
            None,
            None,
        ),
        Keyword::Battlecry => trig(
            Trigger::Attacks { filter: this_only.into(), at_you: false, at_you_or_pw: false, at_enchanted: false, batched: false, player_only: false, at_opponent: false, pw_only: false, one_per_declaration: false },
            vec![Effect::Pump {
                what: Sel::All { filter: (Filter { attacking: true, other: true, ..Default::default() }).into(), zone: None },
                power: fx(1),
                toughness: fx(0),
                duration: Duration::UntilEndOfTurn,
            }],
            None,
            None,
        ),
        Keyword::Exploit => trig(
            Trigger::EntersBattlefield { filter: (this_only.clone()).into() },
            vec![Effect::May { who: None, remember_answer: false, otherwise: vec![], effects: vec![Effect::Sacrifice { who: Sel::You, filter: (Filter { types: vec![CardType::Creature], ..Default::default() }).into(), n: Amount::Fixed { n: 1 } }, Effect::EmitExploited] }],
            None,
            None,
        ),
        Keyword::Myriad => trig(Trigger::Attacks { filter: (this_only.clone()).into(), at_you: false, at_you_or_pw: false, at_enchanted: false, batched: false, player_only: false, at_opponent: false, pw_only: false, one_per_declaration: false }, vec![Effect::May { who: None, remember_answer: false, otherwise: vec![], effects: vec![Effect::Myriad] }], None, None),
        Keyword::LivingWeapon => trig(
            Trigger::EntersBattlefield { filter: (this_only.clone()).into() },
            vec![
                Effect::CreateToken { token: "Phyrexian Germ".into(), n: fx(1), controller: Sel::You, tapped: false, attacking: false, attach_to: None, set_pt: None, attack_at: None, choose_defender: false },
                Effect::Attach { what: Sel::This, to: Sel::Remembered },
            ],
            None,
            None,
        ),
        Keyword::Riot => trig(
            Trigger::EntersBattlefield { filter: (this_only.clone()).into() },
            vec![Effect::Unless {
                payer: Sel::You,
                any_player: false,
                if_paid: vec![],
                cost: vec![Cost::PutCounters { kind: CounterKind::PlusOnePlusOne, n: 1 }],
                effects: vec![Effect::GrantKeyword { what: Sel::This, keyword: Keyword::Haste, duration: Duration::Forever }],
            }],
            None,
            None,
        ),
        // Ravenous: enters with X +1/+1 counters (see expand_keywords); draws a card when X is 5 or more
        Keyword::Ravenous => trig(
            Trigger::EntersBattlefield { filter: (this_only.clone()).into() },
            vec![Effect::Draw { who: Sel::You, n: fx(1), plain: false }],
            Some(Condition::Compare { a: Amount::X, cmp: Cmp::Ge { n: 5 } }),
            None,
        ),
        Keyword::Enlist => trig(
            Trigger::Attacks { filter: (this_only.clone()).into(), at_you: false, at_you_or_pw: false, at_enchanted: false, batched: false, player_only: false, at_opponent: false, pw_only: false, one_per_declaration: false },
            vec![Effect::Choose {
                also_zones: vec![],
                who: Sel::You,
                from: None,
                zone: Zone::Battlefield,
                filter: (Filter { types: vec![CardType::Creature], controller: PlayerFilter::You, attacking: false, tapped: Some(false), not_summoning_sick: true, other: true, ..Default::default() }).into(),
                min: 0,
                max: 1,
                random: false,
                min_amount: None,
                max_amount: None,
                distinct: None,
                accumulate: false,
                top: None,
                among: None,
                then: vec![
                    Effect::Pump { what: Sel::This, power: Amount::PowerOf { of: Sel::Remembered }, toughness: fx(0), duration: Duration::UntilEndOfTurn },
                    Effect::Tap { what: Sel::Remembered },
                    Effect::MarkEnlisted { what: Sel::This },
                ],
            }],
            None,
            None,
        ),
        Keyword::Dethrone => Ability::Triggered {
            trigger: Trigger::Attacks { filter: (this_only.clone()).into(), at_you: false, at_you_or_pw: false, at_enchanted: false, batched: false, player_only: false, at_opponent: false, pw_only: false, one_per_declaration: false },
            targets: vec![],
            effects: vec![Effect::AddCounters { what: Sel::This, kind: CounterKind::PlusOnePlusOne, n: fx(1) }],
            optional: false,
            condition: Some(Condition::AttackedPlayerHasMostLife),
            from: None,
            once_per_turn: false, limit_per_turn: 0, once_ever: false,
            batched: false,
            as_enters: false, no_recheck: true,
        },
        Keyword::Mentor => Ability::Triggered {
            trigger: Trigger::Attacks { filter: (this_only.clone()).into(), at_you: false, at_you_or_pw: false, at_enchanted: false, batched: false, player_only: false, at_opponent: false, pw_only: false, one_per_declaration: false },
            targets: vec![TargetSpec {
                what: TargetKind::Permanent {
                    filter: (Filter { attacking: true, power_vs: Some(AmountCmp { op: CmpOp::Lt, amount: Box::new(Amount::PowerOf { of: Sel::This }) }), ..Default::default() }).into(),
                },
                min: 1,
                max: 1,
                label: "target attacking creature with lesser power".into(),
                max_x: false, exact_x: false, max_amount: None, max_total_mv: None, max_total_power: None, chooser: None,
                distinct: false,
                when_kicked: None,
                per_opponent: false,
                per_player: false, same_graveyard: false, distinct_by: None,
                random: false, only_controller: None, distinct_modes: false, corrupted: false,
            }],
            effects: vec![Effect::AddCounters { what: Sel::Target { idx: 0 }, kind: CounterKind::PlusOnePlusOne, n: fx(1) }, Effect::EmitKeywordAction { action: KeywordAction::Mentors, what: Some(Sel::Target { idx: 0 }) }],
            optional: false,
            condition: None,
            from: None,
            once_per_turn: false, limit_per_turn: 0, once_ever: false,
        batched: false,
        as_enters: false, no_recheck: false,
        },
        Keyword::Evolve => trig(
            Trigger::EntersBattlefield {
                filter: (Filter {
                    types: vec![CardType::Creature],
                    controller: PlayerFilter::You,
                    other: true,
                    any_of: vec![
                        Filter { power_vs: Some(AmountCmp { op: CmpOp::Gt, amount: Box::new(Amount::PowerOf { of: Sel::This }) }), ..Default::default() },
                        Filter { toughness_vs: Some(AmountCmp { op: CmpOp::Gt, amount: Box::new(Amount::ToughnessOf { of: Sel::This }) }), ..Default::default() },
                    ],
                    ..Default::default()
                }).into(),
            },
            vec![Effect::AddCounters { what: Sel::This, kind: CounterKind::PlusOnePlusOne, n: fx(1) }, Effect::EmitKeywordAction { action: KeywordAction::Evolves, what: None }],
            None,
            None,
        ),
        Keyword::Afterlife1 => trig(Trigger::Dies { filter: this_only.into() }, vec![Effect::CreateToken { token: "Spirit".into(), n: fx(1), controller: Sel::You, tapped: false, attacking: false, attach_to: None, set_pt: None, attack_at: None, choose_defender: false }], None, None),
        Keyword::Storm => trig(
            Trigger::SpellCast { filter: this_only.into(), by: PlayerFilter::You, or_copy: false },
            vec![Effect::Repeat {
                // the storm count is every spell cast before this one this turn, by any player
                n: Amount::Plus {
                    a: Box::new(Amount::SpellsCastWhere { filter: Filter { controller: PlayerFilter::You, ..Default::default() }.into(), before_trigger: true }),
                    b: Box::new(Amount::SpellsCastWhere { filter: Filter { controller: PlayerFilter::Opponent, ..Default::default() }.into(), before_trigger: true }),
                },
                effects: vec![Effect::CopySpell { what: Sel::TriggerSource, new_targets: true, by: None, except: vec![], target: None, each_other_target: false, for_opponent: None, even_if_gone: false }],
            }],
            None,
            Some(Zone::Stack),
        ),
        Keyword::Cascade => trig(
            Trigger::SpellCast { filter: this_only.into(), by: PlayerFilter::You, or_copy: false },
            vec![
                Effect::RevealUntil {
                    who: Sel::You,
                    filter: (Filter {
                        not_types: vec![CardType::Land],
                        mana_value_vs: Some(AmountCmp { op: CmpOp::Lt, amount: Box::new(Amount::ManaValueOf { of: Sel::This }) }),
                        ..Default::default()
                    }).into(),
                    count: fx(1),
                    to: Zone::Exile,
                    tapped: false,
                    rest: Zone::Library,
                    rest_random: true,
                    rest_ordered: false,
                    found_to_bottom: false,
                    cascade_land: true,
                    under_your_control: false,
                },
                Effect::CastFree { what: Sel::Remembered, may: true, decline_to_bottom: true, decline_to_hand: false, exile_after: false, copy: false, pay: false, by: None, any_mana: false, lands: false, alt: None, extra_mana: None, otherwise: vec![], suspend: false, lock_spells_after: false, on_cast: vec![], after_cast: vec![], bottom_after: false, copies: None, face_choice: false },
            ],
            None,
            Some(Zone::Stack),
        ),
        _ => return None,
    })
}

fn expand_keywords(c: &CardDef) -> CardDef {
    let mut out = c.clone();
    for k in &c.keywords {
        if let Some(a) = keyword_expansion(*k) {
            out.abilities.push(a);
        }
        if *k == Keyword::Ravenous {
            out.abilities.push(Ability::EntersWithCounters { kind: CounterKind::PlusOnePlusOne, n: Amount::X, condition: None });
        }
    }
    out
}

fn display_name_of(c: &CardDef) -> String {
    if !c.token {
        return c.name.clone();
    }
    if let Some(n) = &c.display_name {
        return n.clone();
    }
    let n = &c.name;
    // "Beast 4-4", "Dragon 2-2 Flying"
    let bytes: Vec<&str> = n.split(' ').collect();
    if let Some(i) = bytes.iter().position(|w| {
        let mut p = w.split('-');
        matches!((p.next(), p.next(), p.next()), (Some(a), Some(b), None) if !a.is_empty() && !b.is_empty() && a.chars().all(|x| x.is_ascii_digit()) && b.chars().all(|x| x.is_ascii_digit()))
    }) {
        if i > 0 {
            return bytes[..i].join(" ");
        }
    }
    n.clone()
}
