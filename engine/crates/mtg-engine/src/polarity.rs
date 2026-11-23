//! Reasoning about effect primitives instead of cards: is a target chosen from the opponent's things (hostile)
//! or from your own (friendly)? The AI uses this for every card without any per-card hints.

use crate::dsl::*;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum Polarity {
    /// Aim at opponents and their things (damage, destroy, exile, discard...).
    Hostile,
    /// Aim at yourself and your things (pump, protection, draw...).
    Friendly,
    Neutral,
}

fn negative(a: &Amount) -> bool {
    matches!(a, Amount::Fixed { n } if *n < 0)
}

fn refers_to(s: &Sel, idx: u8) -> bool {
    match s {
        Sel::Target { idx: i } => *i == idx,
        Sel::ControllerOf { of } | Sel::OwnerOf { of } | Sel::OwnersOf { of, .. } => refers_to(of, idx),
        Sel::Targets => true,
        _ => false,
    }
}

/// Polarity of the `idx`-th target of an ability whose effects are `effects`.
pub fn polarity_of(effects: &[Effect], idx: u8) -> Polarity {
    let mut hostile = 0;
    let mut friendly = 0;
    walk(effects, idx, &mut hostile, &mut friendly);
    match (hostile, friendly) {
        (h, f) if h > f => Polarity::Hostile,
        (h, f) if f > h => Polarity::Friendly,
        _ => Polarity::Neutral,
    }
}

fn walk(effects: &[Effect], idx: u8, h: &mut i32, f: &mut i32) {
    for e in effects {
        match e {
            Effect::DealDamage { to, .. } if refers_to(to, idx) => *h += 1,
            Effect::Destroy { what, .. } | Effect::Exile { what, .. } | Effect::Bounce { what } | Effect::CounterSpell { what, .. }
            | Effect::Tap { what } | Effect::GainControl { what, .. } | Effect::Transform { what }
                if refers_to(what, idx) =>
            {
                *h += 1
            }
            Effect::Discard { who, .. } | Effect::LoseLife { who, .. } | Effect::Mill { who, .. } | Effect::ExileTopOfLibrary { who, .. } | Effect::Sacrifice { who, .. }
                if refers_to(who, idx) =>
            {
                *h += 1
            }
            Effect::Pump { what, power, toughness, .. } | Effect::SetBasePT { what, power, toughness, .. } if refers_to(what, idx) => {
                if negative(power) || negative(toughness) {
                    *h += 1
                } else {
                    *f += 1
                }
            }
            Effect::ModularCounters { what, .. } if refers_to(what, idx) => *f += 1,
            Effect::AddCounters { what, kind, .. } if refers_to(what, idx) => {
                if matches!(kind, crate::types::CounterKind::MinusOneMinusOne | crate::types::CounterKind::Stun) {
                    *h += 1
                } else {
                    *f += 1
                }
            }
            Effect::RemoveKeyword { what, .. } if refers_to(what, idx) => *h += 1,
            Effect::GrantKeyword { what, .. } | Effect::Untap { what } | Effect::Regenerate { what, .. } | Effect::CantRegenerate { what } | Effect::Attach { to: what, .. }
                if refers_to(what, idx) =>
            {
                *f += 1
            }
            Effect::GainLife { who, .. } | Effect::Draw { who, .. } | Effect::Scry { who, .. } | Effect::DrawThenDiscard { who, .. }
                if refers_to(who, idx) =>
            {
                *f += 1
            }
            Effect::Move { what, to, .. } if refers_to(what, idx) => {
                if matches!(to, crate::types::Zone::Hand | crate::types::Zone::Battlefield) {
                    *f += 1
                } else {
                    *h += 1
                }
            }
            Effect::Fight { a, b } | Effect::FightEach { a, b } | Effect::Bite { a, b } => {
                if refers_to(b, idx) {
                    *h += 1
                }
                if refers_to(a, idx) {
                    *f += 1
                }
            }
            Effect::If { then, otherwise, .. } => {
                walk(then, idx, h, f);
                walk(otherwise, idx, h, f);
            }
            Effect::May { effects, .. } | Effect::Unless { effects, .. } | Effect::Repeat { effects, .. } | Effect::ForEach { effects, .. } | Effect::ForEachPlayer { effects, .. }
            | Effect::Delayed { effects, .. } => walk(effects, idx, h, f),
            _ => {}
        }
    }
}
