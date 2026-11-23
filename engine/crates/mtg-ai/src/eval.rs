//! Static evaluation: how good is this position for a player? Built from characteristics, not card names.

use mtg_engine::dsl::{Ability, Amount, Effect, PlayerFilter, Sel, Trigger};
use mtg_engine::types::*;
use mtg_engine::*;

/// Life its controller loses every turn to a trigger that does nothing else (Deadpool, Trading Card; not Dark Confidant or Bitterblossom, which get something for it).
pub fn pure_drain(d: &carddb::Def) -> i32 {
    d.triggers
        .iter()
        .filter_map(|&i| d.abilities.get(i as usize))
        .map(|a| match a {
            Ability::Triggered { trigger: Trigger::BeginningOfStep { step: Step::Upkeep | Step::End, whose: PlayerFilter::You }, effects, optional: false, .. }
                if !effects.is_empty() && effects.iter().all(|e| matches!(e, Effect::LoseLife { who: Sel::You, n: Amount::Fixed { .. } })) =>
            {
                effects.iter().map(|e| if let Effect::LoseLife { n: Amount::Fixed { n }, .. } = e { *n } else { 0 }).sum()
            }
            _ => 0,
        })
        .sum()
}

/// What a permanent's own drain costs its controller: little while the life total is healthy, more as it falls, and a lot once the next upkeep would kill.
fn drain_cost(g: &Game, id: CardId, drain: i32) -> f32 {
    let life = g.player(g.card(id).cur.controller).life;
    if life <= drain {
        return 50.0;
    }
    // two upkeeps from death: get rid of it
    if life <= drain * 2 {
        return 20.0;
    }
    let turns = ((life / drain.max(1)) as f32).min(4.0);
    let per_life = 0.6 + (20 - life).max(0) as f32 * 0.1;
    drain as f32 * turns * per_life
}

/// Worth of a permanent in "life point" units.
pub fn permanent_value(g: &Game, id: CardId) -> f32 {
    let c = g.card(id);
    let d = g.def_of(id);
    let cur = &c.cur;
    let mut v = 0.0f32;
    let abilities = d.triggers.len() + d.activated.len() + d.statics.len();
    if cur.types.has(CardType::Creature) {
        let p = cur.power.max(0) as f32;
        let t = cur.toughness.max(0) as f32;
        v += 1.0 + p * 1.2 + t * 0.8;
        let kw = cur.kw;
        if kw.has(Keyword::Flying) {
            v += 0.6 * p + 0.3;
        }
        if kw.has(Keyword::FirstStrike) {
            v += 0.35 * p + 0.2;
        }
        if kw.has(Keyword::DoubleStrike) {
            v += 0.9 * p + 0.3;
        }
        if kw.has(Keyword::Deathtouch) {
            v += 1.2;
        }
        if kw.has(Keyword::Lifelink) {
            v += 0.5 * p;
        }
        if kw.has(Keyword::Trample) {
            v += 0.25 * p;
        }
        if kw.has(Keyword::Menace) {
            v += 0.4 * p;
        }
        if kw.has(Keyword::Hexproof) || kw.has(Keyword::Shroud) {
            v += 0.8;
        }
        if kw.has(Keyword::Indestructible) {
            v += 1.5 + 0.3 * t;
        }
        if kw.has(Keyword::Vigilance) {
            v += 0.15 * p;
        }
        if kw.has(Keyword::Reach) {
            v += 0.2;
        }
        if kw.has(Keyword::Defender) {
            v -= 0.8 * p;
        }
        if cur.cant_attack && cur.cant_block {
            v *= 0.1;
        } else if cur.cant_attack {
            v *= 0.6;
        }
        v += 0.5 * abilities as f32;
    } else if cur.types.has(CardType::Land) {
        v += 1.6;
    } else if cur.types.has(CardType::Planeswalker) {
        v += 3.0 + 0.6 * c.counter(CounterKind::Loyalty) as f32 + 0.8 * abilities as f32;
    } else {
        v += 1.0 + 0.8 * abilities as f32;
        if !d.mana_abilities.is_empty() {
            v += 1.0;
        }
    }
    if c.token {
        v *= 0.9;
    }
    if !cur.no_abilities {
        let drain = pure_drain(d);
        if drain > 0 {
            v -= drain_cost(g, id, drain);
        }
    }
    v
}

pub fn player_score(g: &Game, p: PlayerId) -> f32 {
    let pl = g.player(p);
    if pl.lost {
        return -1000.0;
    }
    let life = pl.life as f32;
    // life matters more as it gets low
    let life_score = if life >= 20.0 { 10.0 + (life - 20.0) * 0.15 } else if life > 0.0 { life * 0.5 + (1.0 - life / 20.0) * -2.0 } else { -50.0 };
    let mut s = life_score;
    for &id in &g.battlefield {
        if g.card(id).cur.controller == p {
            s += permanent_value(g, id);
        }
    }
    s += pl.hand.len() as f32 * 1.1;
    s -= pl.poison as f32 * 1.5;
    if pl.library.len() < 4 {
        s -= (4 - pl.library.len()) as f32 * 2.0;
    }
    s += pl.graveyard.len() as f32 * 0.03;
    if g.monarch == Some(p) {
        s += 2.0;
    }
    s
}

/// Roughly -1 (certain loss) to 1 (certain win) for `me`.
pub fn evaluate(g: &Game, me: PlayerId) -> f32 {
    evaluate_w(g, me, 0.5)
}

/// `evaluate` with the weight of the strongest opponent (the rest goes to the average): more than 0.5 plays against the leader.
pub fn evaluate_w(g: &Game, me: PlayerId, best_w: f32) -> f32 {
    if let Some(r) = &g.over {
        return match r {
            GameResult::Winner { player } if *player == me => 1.0,
            GameResult::Winner { .. } => -1.0,
            GameResult::Draw => 0.0,
        };
    }
    let mine = player_score(g, me);
    let opps = g.opponents(me);
    if opps.is_empty() {
        return 1.0;
    }
    let scores: Vec<f32> = opps.iter().map(|o| player_score(g, *o)).collect();
    let best = scores.iter().cloned().fold(f32::MIN, f32::max);
    let avg = scores.iter().sum::<f32>() / scores.len() as f32;
    let diff = mine - (best_w * best + (1.0 - best_w) * avg);
    (diff / 25.0).tanh()
}
