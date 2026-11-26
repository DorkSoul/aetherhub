//! Opening hands: whether the AI keeps or mulligans, which cards it puts on the bottom after a London mulligan and how it
//! uses the scry 3 of the free mulligan.
//!
//! A hand is judged as it will be after the bottoming (seven cards seen, six kept after one London mulligan):
//! - seven cards: 3-5 lands, or 2 lands with a cheap mana source or two plays of mana value 2 or less; 0-1 and 6-7 lands
//!   go back;
//! - six cards: 2-4 lands; five cards: 2-4 lands, or 1 land with two cheap plays; four or fewer: always kept;
//! - with the Vancouver and Paris mulligans the smaller hands are drawn as they come instead of chosen from seven, so
//!   they are kept more readily: six cards with 2-5 lands, five cards with 1-4.
//!
//! When the next mulligan costs nothing (free mulligans, free + scry 3, the first mulligan in Commander) only good hands
//! are kept: 3-4 lands; 5 lands with a play of mana value 2 or less (with the scry, which can bottom further lands, any
//! 5 lands); 2 lands with a cheap mana source or, with the scry, a land among the scried cards. After four free
//! mulligans the normal seven-card rule applies and after eight any hand is kept.

use mtg_engine::types::*;
use mtg_engine::*;

/// A land, or a modal double-faced card that can be played as one.
pub fn is_land(g: &Game, c: CardId) -> bool {
    if g.card(c).cur.types.has(CardType::Land) {
        return true;
    }
    let d = g.def_of(c);
    d.face_castable && d.other_face.map_or(false, |o| g.db.def(o).types.has(CardType::Land))
}

/// A nonland card of mana value 2 or less with a mana ability (Sol Ring, Signets, Llanowar Elves).
fn is_ramp(g: &Game, c: CardId) -> bool {
    let d = g.def_of(c);
    !is_land(g, c) && d.mana_value <= 2 && !d.mana_abilities.is_empty()
}

struct Shape {
    cards: usize,
    lands: usize,
    ramp: usize,
    cheap: usize,
}

fn shape(g: &Game, hand: &[CardId]) -> Shape {
    let lands = hand.iter().filter(|c| is_land(g, **c)).count();
    let ramp = hand.iter().filter(|c| is_ramp(g, **c)).count();
    let cheap = hand.iter().filter(|c| !is_land(g, **c) && g.def_of(**c).mana_value <= 2).count();
    Shape { cards: hand.len(), lands, ramp, cheap }
}

/// Whether a hand of this shape is worth keeping. `strict`: the next mulligan is free. `scry`: the free mulligan's scry
/// is used on this hand (extra lands can go to the bottom), `land_on_top`: a land is among the scried cards. `drawn`:
/// a smaller hand was drawn as it is (Vancouver, Paris) rather than chosen from seven.
fn good(s: &Shape, strict: bool, scry: bool, land_on_top: bool, drawn: bool) -> bool {
    let l = s.lands;
    let two_ok = s.ramp >= 1 || land_on_top;
    if strict {
        let five_ok = s.cards >= 7 && (scry || s.cheap >= 1);
        return (3..=4).contains(&l) || (l == 5 && five_ok) || (l == 2 && two_ok);
    }
    match s.cards {
        7.. => (3..=5).contains(&l) || (l == 2 && (two_ok || s.cheap >= 2)),
        6 if drawn => (2..=5).contains(&l),
        6 => (2..=4).contains(&l),
        5 if drawn => (1..=4).contains(&l),
        5 => (2..=4).contains(&l) || (l == 1 && s.cheap >= 2),
        _ => true,
    }
}

/// How many cards a player who keeps after `mulligans` mulligans puts on the bottom.
fn to_bottom(g: &Game, mulligans: u8) -> usize {
    match g.cfg.mulligan {
        MulliganRule::London => mulligans as usize,
        MulliganRule::LondonFreeFirst => mulligans.saturating_sub(1) as usize,
        _ => 0,
    }
}

/// Keep (true) or mulligan this hand. `peek`: the scried top cards of the free mulligan + scry 3, top first.
pub fn keep(g: &Game, hand: &[CardId], mulligans: u8, peek: &[CardId]) -> bool {
    let rule = g.cfg.mulligan;
    let unlimited = matches!(rule, MulliganRule::Free | MulliganRule::FreeScry3);
    if unlimited && mulligans >= 8 {
        return true;
    }
    let next_free = unlimited || (rule == MulliganRule::LondonFreeFirst && mulligans == 0);
    let strict = next_free && !(unlimited && mulligans >= 4);
    let out = choose_bottom(g, hand, to_bottom(g, mulligans));
    let kept: Vec<CardId> = hand.iter().filter(|c| !out.contains(c)).copied().collect();
    let land_on_top = peek.iter().any(|c| is_land(g, *c));
    let drawn = matches!(rule, MulliganRule::Vancouver | MulliganRule::Paris);
    good(&shape(g, &kept), strict, !peek.is_empty(), land_on_top, drawn)
}

/// The cards to put on the bottom after keeping: lands above about half of the hand that stays, else the most
/// expensive spells (cheap mana sources last).
pub fn choose_bottom(g: &Game, hand: &[CardId], n: usize) -> Vec<CardId> {
    let mut cards: Vec<CardId> = hand.to_vec();
    let stay = hand.len().saturating_sub(n);
    let max_lands = ((stay + 1) / 2).clamp(2, 4);
    let mut out = vec![];
    for _ in 0..n.min(hand.len()) {
        let lands = cards.iter().filter(|c| is_land(g, **c)).count();
        let spell = cards
            .iter()
            .enumerate()
            .filter(|(_, c)| !is_land(g, **c))
            .max_by_key(|(_, c)| (!is_ramp(g, **c), g.def_of(**c).mana_value))
            .map(|(i, _)| i);
        let i = if lands > max_lands || spell.is_none() { cards.iter().position(|c| is_land(g, *c)).unwrap_or(0) } else { spell.unwrap() };
        out.push(cards.remove(i));
    }
    out
}

/// Scry with `lands` lands available (hand, plus the battlefield during the game): the indices of `seen` to put on the
/// bottom. Lands past five go; when short of four lands, spells above the land count go to dig for lands; spells far
/// above the land count always go.
pub fn scry_bottom(g: &Game, lands: usize, seen: &[CardId]) -> Vec<usize> {
    let seen_lands = seen.iter().filter(|c| is_land(g, **c)).count();
    let short = lands + seen_lands < 4;
    let mut kept_lands = 0;
    let mut out = vec![];
    for (i, c) in seen.iter().enumerate() {
        if is_land(g, *c) {
            if lands + kept_lands >= 5 {
                out.push(i);
            } else {
                kept_lands += 1;
            }
        } else {
            let mv = g.def_of(*c).mana_value as usize;
            if mv > lands + 3 || (short && mv > lands.max(2) && !is_ramp(g, *c)) {
                out.push(i);
            }
        }
    }
    out
}

/// The answer to a mulligan question: under free + scry 3 the scry comes first (once, on a hand it keeps), then the keep.
pub fn decide(g: &Game, hand: &[CardId], mulligans: u8, peek: &[CardId]) -> Choice {
    if g.cfg.mulligan == MulliganRule::FreeScry3 && peek.len() < 3 {
        // the scry was already made on this hand, which means it is being kept
        return Choice::Keep { keep: true };
    }
    if !keep(g, hand, mulligans, peek) {
        return Choice::Keep { keep: false };
    }
    if !peek.is_empty() {
        let lands = hand.iter().filter(|c| is_land(g, **c)).count();
        let out = scry_bottom(g, lands, peek);
        if !out.is_empty() {
            return Choice::Pick { indices: out };
        }
    }
    Choice::Keep { keep: true }
}
