//! Text-changing effects (CR 612): colour words, basic land types and creature types in an object's text (and type line) are replaced.
//!
//! A card whose text was changed gets a synthetic `Def` (a copy of its definition with the rewritten abilities); the engine swaps `Card.def` to it
//! and back again when the effect ends or the object changes zones. Synthetic defs live in `CardDb::extra`.

use crate::carddb::{CardDb, Def};
use crate::dsl::*;
use crate::ids::*;
use crate::types::*;
use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub enum TextChange {
    /// Replace one colour word ("W".."G"), basic land type or creature type with another.
    Word { kind: WordKind, old: String, new: String },
    /// Every colour word becomes this one (Swirl the Mists).
    AllColors { to: String },
    /// The object's text box becomes that of the donor definition (Deadpool, Trading Card).
    SetAbilitiesFrom { donor: DefId },
}

/// The question a text-changing effect is asking right now (read by the AI to rank the options): `stage` 1 = which kind of word,
/// 3 = which word to replace (of `kind`), 5 = the new word (replacing `old`).
#[derive(Clone, Debug, PartialEq)]
pub struct TextAsk {
    pub target: CardId,
    pub stage: u8,
    pub kind: WordKind,
    pub old: String,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum WordKind {
    Color,
    Land,
    Creature,
}

pub const LAND_TYPES: [&str; 5] = ["Plains", "Island", "Swamp", "Mountain", "Forest"];
pub const COLOR_LETTERS: [&str; 5] = ["W", "U", "B", "R", "G"];
pub const COLOR_NAMES: [&str; 5] = ["white", "blue", "black", "red", "green"];

/// JSON keys of the DSL that hold colour words (mana symbols such as `AddMana.m` are not words and are left alone).
const COLOR_KEYS: [&str; 8] = ["color", "or_color", "colors", "add_colors", "all_colors", "not_colors", "c", "chosen_color"];
/// JSON keys that hold subtype words.
const SUBTYPE_KEYS: [&str; 6] = ["subtypes", "not_subtypes", "set_subtypes", "add_subtypes", "only", "subtype"];

fn walk_strings(v: &mut Value, key: &str, parent_type: &str, f: &mut dyn FnMut(&str, &str, &mut String)) {
    match v {
        Value::String(s) => f(key, parent_type, s),
        Value::Array(a) => {
            for x in a {
                walk_strings(x, key, parent_type, f);
            }
        }
        Value::Object(m) => {
            let t = m.get("type").and_then(|t| t.as_str()).unwrap_or("").to_string();
            for (k, x) in m.iter_mut() {
                let k = k.clone();
                walk_strings(x, &k, &t, f);
            }
        }
        _ => {}
    }
}

fn walk_ro(v: &Value, key: &str, parent_type: &str, f: &mut dyn FnMut(&str, &str, &str)) {
    match v {
        Value::String(s) => f(key, parent_type, s),
        Value::Array(a) => {
            for x in a {
                walk_ro(x, key, parent_type, f);
            }
        }
        Value::Object(m) => {
            let t = m.get("type").and_then(|t| t.as_str()).unwrap_or("");
            for (k, x) in m {
                walk_ro(x, k, t, f);
            }
        }
        _ => {}
    }
}

fn walk_word(kind: WordKind, key: &str, parent_type: &str, s: &str) -> Option<String> {
    match kind {
        WordKind::Color => (COLOR_KEYS.contains(&key) && COLOR_LETTERS.contains(&s)).then(|| s.to_string()),
        WordKind::Land => {
            if (SUBTYPE_KEYS.contains(&key) || (key == "name" && parent_type == "Subtype")) && LAND_TYPES.contains(&s) {
                Some(s.to_string())
            } else {
                s.strip_suffix("walk").filter(|w| LAND_TYPES.contains(w)).map(|w| w.to_string())
            }
        }
        WordKind::Creature => ((SUBTYPE_KEYS.contains(&key) || (key == "name" && parent_type == "Subtype")) && !LAND_TYPES.contains(&s) && s != "Aura" && s != "Equipment").then(|| s.to_string()),
    }
}

/// The words of `kind` that appear in the text (and type line) of `d`: colour letters, land type names or creature types.
pub fn words_in(d: &Def, kind: WordKind) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for a in &d.abilities {
        if let Ok(v) = serde_json::to_value(a) {
            walk_ro(&v, "", "", &mut |k, t, s| {
                if let Some(w) = walk_word(kind, k, t, s) {
                    if !out.contains(&w) {
                        out.push(w);
                    }
                }
            });
        }
    }
    for k in d.keywords_list() {
        if kind == WordKind::Land {
            if let Some(w) = format!("{k:?}").strip_suffix("walk").filter(|w| LAND_TYPES.contains(w)) {
                if !out.contains(&w.to_string()) {
                    out.push(w.to_string());
                }
            }
        }
    }
    for s in &d.subtypes {
        let n = s.name().to_string();
        let is_land = LAND_TYPES.contains(&n.as_str());
        let creature_like = d.types.has(CardType::Creature) || d.types.has(CardType::Kindred);
        let ok = match kind {
            WordKind::Land => is_land,
            WordKind::Creature => !is_land && creature_like,
            WordKind::Color => false,
        };
        if ok && !out.contains(&n) {
            out.push(n);
        }
    }
    out
}

fn replace_word(d: &mut Def, kind: WordKind, old: &str, new: &str) {
    for a in d.abilities.iter_mut() {
        let Ok(mut v) = serde_json::to_value(&*a) else { continue };
        let mut changed = false;
        walk_strings(&mut v, "", "", &mut |k, t, s| {
            if let Some(w) = walk_word(kind, k, t, s) {
                if w == old {
                    *s = if kind == WordKind::Land && s.ends_with("walk") { format!("{new}walk") } else { new.to_string() };
                    changed = true;
                }
            }
        });
        if changed {
            if let Ok(na) = serde_json::from_value::<Ability>(v) {
                *a = na;
            }
        }
    }
    if kind != WordKind::Color {
        let mut subs: Vec<Sym> = vec![];
        for s in d.subtypes.iter() {
            let n = if s.name() == old { Sym::new(new) } else { *s };
            if !subs.contains(&n) {
                subs.push(n);
            }
        }
        d.subtypes = subs.into_iter().collect();
    }
    if kind == WordKind::Land {
        let from: Result<Keyword, _> = serde_json::from_value(Value::String(format!("{old}walk")));
        let to: Result<Keyword, _> = serde_json::from_value(Value::String(format!("{new}walk")));
        if let (Ok(f), Ok(t)) = (from, to) {
            if d.keywords.has(f) {
                d.keywords.remove(f);
                d.keywords.add(t);
            }
        }
    }
}

/// A copy of `base` with the changes applied in order.
pub fn transformed(db: &CardDb, base: &Def, changes: &[TextChange]) -> Def {
    let mut d = base.clone();
    for ch in changes {
        match ch {
            TextChange::Word { kind, old, new } => replace_word(&mut d, *kind, old, new),
            TextChange::AllColors { to } => {
                for c in COLOR_LETTERS.iter() {
                    if c != to {
                        replace_word(&mut d, WordKind::Color, c, to);
                    }
                }
            }
            TextChange::SetAbilitiesFrom { donor } => {
                let dd = db.def(*donor);
                d.abilities = dd.abilities.clone();
                d.keywords = dd.keywords;
            }
        }
    }
    d.reindex();
    d
}

/// Display names of the words of a kind that can be chosen as the new word.
pub fn all_words(db: &CardDb, kind: WordKind) -> Vec<String> {
    match kind {
        WordKind::Color => COLOR_LETTERS.iter().map(|s| s.to_string()).collect(),
        WordKind::Land => LAND_TYPES.iter().map(|s| s.to_string()).collect(),
        WordKind::Creature => {
            let mut v: Vec<String> = vec![];
            for d in &db.defs {
                if d.types.has(CardType::Creature) || d.types.has(CardType::Kindred) {
                    for s in &d.subtypes {
                        let n = s.name().to_string();
                        if !LAND_TYPES.contains(&n.as_str()) && !v.contains(&n) {
                            v.push(n);
                        }
                    }
                }
            }
            v.sort();
            v
        }
    }
}

pub fn display_word(kind: WordKind, w: &str) -> String {
    match kind {
        WordKind::Color => COLOR_LETTERS.iter().position(|c| *c == w).map_or(w.to_string(), |i| COLOR_NAMES[i].to_string()),
        _ => w.to_string(),
    }
}

pub fn parse_word(kind: WordKind, shown: &str) -> String {
    match kind {
        WordKind::Color => COLOR_NAMES.iter().position(|c| *c == shown).map_or(shown.to_string(), |i| COLOR_LETTERS[i].to_string()),
        _ => shown.to_string(),
    }
}

impl crate::state::Game {
    /// Rebuild `id`'s definition from its base definition and its text changes (its own and Swirl the Mists's).
    pub(crate) fn refresh_text(&mut self, id: CardId) {
        let c = &self.cards[id.0 as usize];
        let Some(base) = c.text_base else { return };
        let mut changes: Vec<TextChange> = c.text_changes.iter().map(|(ch, _)| ch.clone()).collect();
        if let Some(to) = &c.text_global {
            changes.push(TextChange::AllColors { to: to.clone() });
        }
        let new = self.db.text_def(base, &changes);
        let on_stack = c.zone == Zone::Stack;
        self.cards[id.0 as usize].def = new;
        if on_stack {
            for it in self.stack.iter_mut() {
                if it.source == id {
                    it.def = new;
                }
            }
        }
        if changes.is_empty() {
            self.cards[id.0 as usize].text_base = None;
        }
        self.layers_dirty = true;
    }

    /// Try a one-word text change on this (cloned) game and refresh the layers: used by the AI to see what a change would do.
    pub fn try_change_word(&mut self, id: CardId, kind: WordKind, old: &str, new: &str) {
        self.apply_text_change(id, TextChange::Word { kind, old: old.to_string(), new: parse_word(kind, new) }, false);
        self.recompute_layers();
    }

    /// Try an exchange of text boxes between two objects on this (cloned) game.
    pub fn try_exchange_text(&mut self, a: CardId, b: CardId) {
        let (da, db) = (self.cards[a.0 as usize].def, self.cards[b.0 as usize].def);
        self.apply_text_change(a, TextChange::SetAbilitiesFrom { donor: db }, false);
        self.apply_text_change(b, TextChange::SetAbilitiesFrom { donor: da }, false);
        self.recompute_layers();
    }

    /// Apply a text change to a permanent or spell.
    pub(crate) fn apply_text_change(&mut self, id: CardId, ch: TextChange, until_eot: bool) {
        let c = &mut self.cards[id.0 as usize];
        if c.text_base.is_none() {
            c.text_base = Some(c.def);
        }
        c.text_changes.push((ch, until_eot));
        self.refresh_text(id);
    }

    /// At the end of the turn the "until end of turn" text changes stop.
    pub(crate) fn expire_text_changes(&mut self) {
        let ids: Vec<CardId> = self.cards.iter().filter(|c| c.text_changes.iter().any(|(_, e)| *e)).map(|c| c.id).collect();
        for id in ids {
            self.cards[id.0 as usize].text_changes.retain(|(_, e)| !*e);
            if self.cards[id.0 as usize].text_changes.is_empty() && self.cards[id.0 as usize].text_global.is_none() {
                if let Some(b) = self.cards[id.0 as usize].text_base.take() {
                    self.cards[id.0 as usize].def = b;
                    for it in self.stack.iter_mut() {
                        if it.source == id {
                            it.def = b;
                        }
                    }
                    self.layers_dirty = true;
                }
            } else {
                self.refresh_text(id);
            }
        }
    }

    /// Swirl the Mists: every permanent and spell has its colour words changed to the colour chosen for the newest one on the battlefield.
    pub(crate) fn sync_global_text(&mut self) {
        let want: Option<String> = self
            .battlefield
            .iter()
            .rev()
            .find_map(|&id| {
                let c = &self.cards[id.0 as usize];
                let d = self.db.def(c.def);
                if !c.cur.no_abilities && d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], Ability::Static { effect: StaticEffect::AllColorWordsChosen, .. })) {
                    c.chosen_color.map(|col| COLOR_LETTERS[Color::ALL.iter().position(|x| *x == col).unwrap_or(0)].to_string())
                } else {
                    None
                }
            });
        let ids: Vec<CardId> = self.battlefield.iter().copied().chain(self.stack.iter().map(|i| i.source)).collect();
        for id in ids {
            if self.cards[id.0 as usize].text_global != want {
                self.cards[id.0 as usize].text_global = want.clone();
                if want.is_some() && self.cards[id.0 as usize].text_base.is_none() {
                    let d = self.cards[id.0 as usize].def;
                    self.cards[id.0 as usize].text_base = Some(d);
                }
                self.refresh_text(id);
            }
        }
    }
}
