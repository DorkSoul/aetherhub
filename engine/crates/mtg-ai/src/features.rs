//! A learned position evaluation: a few dozen features of a position as seen by one seat, and a logistic model of
//! "this seat wins" fitted on self-play (`datagen` records positions, `fit` fits `src/eval_weights.json`).

use crate::eval::permanent_value;
use mtg_engine::types::*;
use mtg_engine::*;
use std::sync::OnceLock;

/// Per-seat measures: life, board worth, creature count, power, lands, hand, planeswalkers, graveyard, untapped mana,
/// evasive power, total toughness, quality keywords, tokens, castable hand, lands in hand, mana invested, instants in hand.
const M: usize = 17;
/// Each measure for me, the average opponent and the best opponent, then extras.
pub const N: usize = M * 3 + 7;

fn seat_measures(g: &Game, q: PlayerId) -> [f32; M] {
    let pl = g.player(q);
    let (mut board, mut creatures, mut power, mut lands, mut pws) = (0f32, 0f32, 0f32, 0f32, 0f32);
    let (mut untapped, mut evasive, mut toughness, mut quality, mut tokens, mut invested) = (0f32, 0f32, 0f32, 0f32, 0f32, 0f32);
    for &id in &g.battlefield {
        let c = g.card(id);
        if c.cur.controller != q {
            continue;
        }
        board += permanent_value(g, id);
        let d = g.def_of(id);
        if c.token {
            tokens += 1.0;
        }
        if !c.tapped && (c.cur.types.has(CardType::Land) || !d.mana_abilities.is_empty()) {
            untapped += 1.0;
        }
        if c.cur.types.has(CardType::Creature) {
            creatures += 1.0;
            power += c.cur.power.max(0) as f32;
            toughness += c.cur.toughness.max(0) as f32;
            if c.cur.kw.has(Keyword::Flying) {
                evasive += c.cur.power.max(0) as f32;
            }
            for k in [Keyword::Deathtouch, Keyword::FirstStrike, Keyword::DoubleStrike, Keyword::Lifelink] {
                if c.cur.kw.has(k) {
                    quality += 1.0;
                }
            }
        }
        if c.cur.types.has(CardType::Land) {
            lands += 1.0;
        } else {
            invested += d.mana_value as f32;
        }
        if c.cur.types.has(CardType::Planeswalker) {
            pws += 1.0;
        }
    }
    let (mut castable, mut hand_lands, mut instants) = (0f32, 0f32, 0f32);
    for &id in &pl.hand {
        let c = g.card(id);
        if c.cur.types.has(CardType::Land) {
            hand_lands += 1.0;
            continue;
        }
        if g.def_of(id).mana_value as f32 <= lands + 1.0 {
            castable += 1.0;
        }
        if c.cur.types.has(CardType::Instant) {
            instants += 1.0;
        }
    }
    [
        ((pl.life.clamp(0, 60) + 1) as f32).ln(),
        board / 20.0,
        creatures / 5.0,
        power / 10.0,
        lands / 8.0,
        pl.hand.len() as f32 / 7.0,
        pws,
        (pl.graveyard.len() as f32 / 20.0).min(2.0),
        untapped / 8.0,
        evasive / 5.0,
        toughness / 10.0,
        quality / 3.0,
        tokens / 5.0,
        castable / 4.0,
        hand_lands / 4.0,
        invested / 20.0,
        instants / 3.0,
    ]
}

pub fn features(g: &Game, me: PlayerId) -> Vec<f32> {
    let mine = seat_measures(g, me);
    let opps = g.opponents(me);
    let n = opps.len().max(1) as f32;
    let theirs: Vec<[f32; M]> = opps.iter().map(|o| seat_measures(g, *o)).collect();
    let mut f = Vec::with_capacity(N);
    for m in 0..M {
        let avg = theirs.iter().map(|t| t[m]).sum::<f32>() / n;
        let best = theirs.iter().map(|t| t[m]).fold(f32::MIN, f32::max);
        f.push(mine[m]);
        f.push(avg);
        f.push(if theirs.is_empty() { 0.0 } else { best });
    }
    let life = g.player(me).life as f32;
    let min_opp_life = opps.iter().map(|o| g.player(*o).life).min().unwrap_or(20) as f32;
    f.push(if life <= 5.0 { 1.0 } else { 0.0 });
    f.push(if min_opp_life <= 5.0 { 1.0 } else { 0.0 });
    f.push(if g.monarch == Some(me) { 1.0 } else { 0.0 });
    f.push((g.player(me).library.len().min(8) as f32 - 8.0) / 8.0);
    f.push(g.player(me).poison as f32 / 5.0);
    f.push(if g.active == me { 1.0 } else { 0.0 });
    f.push(1.0); // bias
    debug_assert_eq!(f.len(), N);
    f
}

fn weights() -> &'static Option<Vec<f32>> {
    static W: OnceLock<Option<Vec<f32>>> = OnceLock::new();
    W.get_or_init(|| {
        let text = include_str!("eval_weights.json");
        let v: Vec<f32> = serde_json::from_str(text).ok()?;
        if v.len() == N {
            Some(v)
        } else {
            None
        }
    })
}

pub fn has_weights() -> bool {
    weights().is_some()
}

/// Probability-like score in -1..1 (2 * P(win) - 1); falls back to the hand-written evaluation without fitted weights.
pub fn learned_eval(g: &Game, me: PlayerId) -> f32 {
    if let Some(r) = &g.over {
        return match r {
            GameResult::Winner { player } if *player == me => 1.0,
            GameResult::Winner { .. } => -1.0,
            GameResult::Draw => 0.0,
        };
    }
    if g.player(me).lost {
        return -1.0;
    }
    match weights() {
        Some(w) => {
            let f = features(g, me);
            let z: f32 = f.iter().zip(w).map(|(a, b)| a * b).sum();
            2.0 / (1.0 + (-z).exp()) - 1.0
        }
        None => crate::eval::evaluate(g, me),
    }
}

// ------------------------------------------------------------------ ONNX evaluation

type Plan = std::sync::Arc<tract_onnx::prelude::TypedRunnableModel>;

fn onnx_plan() -> &'static Option<Plan> {
    use tract_onnx::prelude::*;
    static P: OnceLock<Option<Plan>> = OnceLock::new();
    P.get_or_init(|| {
        let bytes: &[u8] = include_bytes!("eval_mlp.onnx");
        let model = tract_onnx::onnx().model_for_read(&mut std::io::Cursor::new(bytes)).ok()?;
        let model = model.with_input_fact(0, f32::fact([1, N]).into()).ok()?;
        model.into_optimized().ok()?.into_runnable().ok()
    })
}

pub fn has_onnx() -> bool {
    onnx_plan().is_some()
}

/// The MLP's win logit for a feature vector (None if the model failed to load or does not match `N`).
pub fn onnx_logit(f: &[f32]) -> Option<f32> {
    use tract_onnx::prelude::*;
    let plan = onnx_plan().as_ref()?;
    let input = tract_ndarray::Array2::from_shape_vec((1, f.len()), f.to_vec()).ok()?;
    let out = plan.run(tvec!(Tensor::from(input).into())).ok()?;
    out[0].to_plain_array_view::<f32>().ok()?.iter().next().copied()
}

/// `learned_eval` with the ONNX multilayer network in place of the linear model (2 * P(win) - 1).
pub fn onnx_eval(g: &Game, me: PlayerId) -> f32 {
    if let Some(r) = &g.over {
        return match r {
            GameResult::Winner { player } if *player == me => 1.0,
            GameResult::Winner { .. } => -1.0,
            GameResult::Draw => 0.0,
        };
    }
    if g.player(me).lost {
        return -1.0;
    }
    match onnx_logit(&features(g, me)) {
        Some(z) => 2.0 / (1.0 + (-z).exp()) - 1.0,
        None => crate::eval::evaluate(g, me),
    }
}
