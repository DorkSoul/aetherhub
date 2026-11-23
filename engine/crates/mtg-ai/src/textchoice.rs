//! Answers to the questions of text-changing effects and to number questions that a plain "first option / maximum" would get wrong.
//!
//! A text change is judged by trying it on a clone of the game: the change in the board's worth (power and toughness, keywords,
//! lords) plus what the evaluation can't see: evasion and protection words (landwalk and protection from a color are worth more
//! when the opponents have lands of that type / permanents of that color) and drawbacks printed on a text box.

use crate::eval::player_score;
use mtg_engine::textchange::*;
use mtg_engine::types::*;
use mtg_engine::*;

const COLORS: [Color; 5] = [Color::W, Color::U, Color::B, Color::R, Color::G];

/// Worth of each opposing permanent that a matching word makes relevant (landwalk, protection, "destroy target red creature").
const RELEVANCE: f32 = 0.6;

/// How well `me` stands against the opponents, in the evaluation's life-point units.
fn margin(g: &Game, me: PlayerId) -> f32 {
    let opps = g.opponents(me);
    if opps.is_empty() {
        return player_score(g, me);
    }
    let avg = opps.iter().map(|o| player_score(g, *o)).sum::<f32>() / opps.len() as f32;
    player_score(g, me) - avg
}

/// How many permanents of `players` match the word.
fn relevance(g: &Game, players: &[PlayerId], kind: WordKind, word: &str) -> f32 {
    let color = COLOR_LETTERS.iter().position(|c| *c == word).map(|i| COLORS[i]);
    g.battlefield
        .iter()
        .filter(|id| {
            let c = g.card(**id);
            if !players.contains(&c.cur.controller) {
                return false;
            }
            match kind {
                WordKind::Color => color.map_or(false, |col| c.cur.colors.has(col)),
                WordKind::Land | WordKind::Creature => c.cur.subtypes.iter().any(|s| s.name() == word),
            }
        })
        .count() as f32
}

/// Is the word used for evasion / protection by this object (so the opponents' board decides what it is worth)?
fn evasive_use(g: &Game, target: CardId, kind: WordKind, word: &str) -> bool {
    let c = g.card(target);
    if c.zone == Zone::Stack {
        return true;
    }
    match kind {
        WordKind::Land => g.def_of(target).keywords_list().iter().any(|k| format!("{k:?}").strip_suffix("walk") == Some(word)),
        WordKind::Color => COLOR_LETTERS.iter().position(|l| *l == word).map_or(false, |i| {
            c.cur.protections.iter().chain(c.cur.hexproof_from.iter()).any(|p| matches!(p, mtg_engine::dsl::ProtectionFrom::Color { c } if *c == COLORS[i]))
        }),
        WordKind::Creature => false,
    }
}

struct Ctx<'a> {
    g: &'a Game,
    me: PlayerId,
    target: CardId,
    base: f32,
    friendly: bool,
}

impl<'a> Ctx<'a> {
    fn new(g: &'a Game, me: PlayerId, target: CardId) -> Self {
        Ctx { g, me, target, base: margin(g, me), friendly: g.card(target).cur.controller == me }
    }

    /// How much better (for `me`) the position gets when `old` becomes `new` on the target.
    fn score(&self, kind: WordKind, old: &str, new: &str) -> f32 {
        let mut sim = self.g.clone();
        sim.text_ask = None;
        sim.try_change_word(self.target, kind, old, new);
        let mut s = margin(&sim, self.me) - self.base;
        if evasive_use(self.g, self.target, kind, old) {
            let opps = self.g.opponents(self.me);
            let mine = [self.me];
            s += RELEVANCE
                * if self.friendly {
                    relevance(self.g, &opps, kind, new) - relevance(self.g, &opps, kind, old)
                } else {
                    relevance(self.g, &mine, kind, old) - relevance(self.g, &mine, kind, new)
                };
        }
        s
    }

    /// Words worth trying as the new word.
    fn news(&self, kind: WordKind, old: &str) -> Vec<String> {
        match kind {
            WordKind::Color => COLOR_LETTERS.iter().filter(|w| **w != old).map(|w| w.to_string()).collect(),
            WordKind::Land => LAND_TYPES.iter().filter(|w| **w != old).map(|w| w.to_string()).collect(),
            WordKind::Creature => {
                let mut v: Vec<String> = vec![];
                for &id in &self.g.battlefield {
                    let c = self.g.card(id);
                    if c.cur.types.has(CardType::Creature) {
                        for s in &c.cur.subtypes {
                            let n = s.name().to_string();
                            if n != old && !LAND_TYPES.contains(&n.as_str()) && !v.contains(&n) {
                                v.push(n);
                            }
                        }
                    }
                }
                if let Some(f) = all_words(&self.g.db, kind).into_iter().find(|w| w != old && !v.contains(w)) {
                    v.push(f);
                }
                v
            }
        }
    }

    fn olds(&self, kind: WordKind) -> Vec<String> {
        words_in(self.g.def_of(self.target), kind)
    }

    /// The best change still possible once `kind` (and `old`) are fixed.
    fn best(&self, kind: WordKind, old: Option<&str>) -> f32 {
        let mut best = f32::MIN;
        for o in self.olds(kind) {
            if old.map_or(false, |x| x != o) {
                continue;
            }
            for n in self.news(kind, &o) {
                best = best.max(self.score(kind, &o, &n));
            }
        }
        best
    }
}

fn order(scores: Vec<f32>) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..scores.len()).collect();
    idx.sort_by(|a, b| scores[*b].partial_cmp(&scores[*a]).unwrap_or(std::cmp::Ordering::Equal));
    idx
}

/// Option indices of a text-change question, best first (None when the decision isn't one).
pub fn rank_text_pick(g: &Game, me: PlayerId, d: &Decision) -> Option<Vec<usize>> {
    let ask = g.text_ask.as_ref()?;
    let DecisionKind::Pick { options, .. } = &d.kind else { return None };
    let c = Ctx::new(g, me, ask.target);
    let texts: Vec<&str> = options.iter().map(|o| if let Opt::Text { text } = o { text.as_str() } else { "" }).collect();
    let scores: Vec<f32> = match ask.stage {
        1 => texts
            .iter()
            .map(|t| {
                let kind = match *t {
                    "A color word" => WordKind::Color,
                    "A basic land type" => WordKind::Land,
                    _ => WordKind::Creature,
                };
                c.best(kind, None)
            })
            .collect(),
        3 => {
            let words = c.olds(ask.kind);
            (0..texts.len()).map(|i| words.get(i).map_or(f32::MIN, |w| c.best(ask.kind, Some(w)))).collect()
        }
        5 => {
            let known = c.news(ask.kind, &ask.old);
            let all = all_words(&g.db, ask.kind);
            let mut off_board: Option<f32> = None;
            texts
                .iter()
                .map(|t| {
                    let raw = all.iter().find(|w| display_word(ask.kind, w) == *t).cloned().unwrap_or_else(|| parse_word(ask.kind, t));
                    if known.contains(&raw) {
                        c.score(ask.kind, &ask.old, &raw)
                    } else {
                        // a creature type nobody has: every such word is worth the same
                        *off_board.get_or_insert_with(|| c.score(ask.kind, &ask.old, &raw))
                    }
                })
                .collect()
        }
        _ => return None,
    };
    Some(order(scores))
}

/// Printed drawbacks of a text box (the evaluation counts every ability as a plus): life loss for its controller, sacrifice of itself.
fn drawbacks(def: &carddb::Def) -> f32 {
    let mut pen = 0.0;
    for a in &def.abilities {
        let Ok(v) = serde_json::to_string(a) else { continue };
        if v.contains("\"type\":\"Triggered\"") {
            if v.contains("\"type\":\"LoseLife\",\"who\":{\"type\":\"You\"}") {
                pen += 4.0;
            }
            if v.contains("\"type\":\"Sacrifice\"") && v.contains("\"type\":\"This\"") {
                pen += 3.0;
            }
        }
    }
    pen
}

/// "Exchange text boxes" (Deadpool): rank the creatures to exchange with, best first. Looks at what each text box is worth to its new owner.
pub fn rank_exchange(g: &Game, me: PlayerId, d: &Decision) -> Option<Vec<usize>> {
    let DecisionKind::Pick { options, source: Some(src), .. } = &d.kind else { return None };
    let has = g.def_of(*src).abilities.iter().any(|a| serde_json::to_string(a).map_or(false, |s| s.contains("ExchangeTextBoxes")));
    if !has || g.card(*src).zone != Zone::Battlefield {
        return None;
    }
    let base = margin(g, me);
    let sign = |id: CardId| if g.card(id).cur.controller == me { 1.0 } else { -1.0 };
    let scores: Vec<f32> = options
        .iter()
        .map(|o| {
            let Opt::Obj { obj: Obj::Card { id } } = o else { return f32::MIN };
            let mut sim = g.clone();
            sim.try_exchange_text(*src, *id);
            let mut s = margin(&sim, me) - base;
            // drawbacks follow the text box to its new controller
            for (x, other) in [(*src, *id), (*id, *src)] {
                s += sign(x) * (drawbacks(g.def_of(x)) - drawbacks(g.def_of(other)));
            }
            s
        })
        .collect();
    Some(order(scores))
}

/// The cheap answer to a number question, by what it asks (used in playouts and as the tie-break of the simulated answer).
pub fn number_default(prompt: &str, min: i32, max: i32) -> i32 {
    let p = prompt.to_lowercase();
    let n = if p.contains("pay how much life")
        || p.contains("bidding")
        || p.contains("top the high bid")
        || p.contains("remove how many counters")
        || p.starts_with("assist")
        || p.contains("hide how many")
        || p.contains("choose a number")
    {
        min
    } else {
        // X, kicker and replicate counts, extra mana, divided damage and counters: more is better, the question already caps it at what can be afforded
        max
    };
    n.clamp(min, max)
}

/// The question is how much life to pay (or bid).
pub fn pays_life(prompt: &str) -> bool {
    let p = prompt.to_lowercase();
    p.contains("pay how much life") || p.contains("bidding") || p.contains("top the high bid") || p.contains(" life")
}

/// Values worth simulating for a number question.
pub fn number_candidates(min: i32, max: i32, default: i32) -> Vec<i32> {
    let mut v: Vec<i32> = vec![default];
    if max - min <= 8 {
        v.extend(min..=max);
    } else {
        let span = max - min;
        for n in [min, min + 1, min + 2, min + 3, min + span / 4, min + span / 2, max - span / 4, max - 1, max] {
            v.push(n);
        }
    }
    v.retain(|n| *n >= min && *n <= max);
    let mut seen = vec![];
    v.retain(|n| {
        if seen.contains(n) {
            false
        } else {
            seen.push(*n);
            true
        }
    });
    v
}
