//! Random-deck, random-play fuzzing: no crash, no hang, no inconsistent state.

use crate::agent::{play_game, Agent};
use crate::random::RandomAgent;
use mtg_engine::types::*;
use mtg_engine::*;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct FuzzFailure {
    pub seed: u64,
    pub players: usize,
    pub message: String,
    pub decisions: Vec<Choice>,
}

#[derive(Debug, Default, Clone)]
pub struct FuzzReport {
    pub games: u64,
    pub finished: u64,
    pub timed_out: u64,
    pub decisions: u64,
    pub failures: Vec<FuzzFailure>,
}

/// Build a random two-color deck: basic lands plus random on-color (or colorless) cards from the pool.
pub fn random_deck(db: &Arc<CardDb>, rng: &mut ChaCha8Rng, size: usize) -> Vec<DefId> {
    let basics: Vec<(Color, DefId)> = [("Plains", Color::W), ("Island", Color::U), ("Swamp", Color::B), ("Mountain", Color::R), ("Forest", Color::G)]
        .iter()
        .filter_map(|(n, c)| db.find(n).map(|d| (*c, d)))
        .collect();
    let mut chosen = basics.clone();
    chosen.shuffle(rng);
    chosen.truncate(2);
    let colors = ColorSet::of(&chosen.iter().map(|(c, _)| *c).collect::<Vec<_>>());
    let pool: Vec<DefId> = db
        .defs
        .iter()
        .filter(|d| {
            d.is_front
                && !d.token
                && !d.supertypes.contains(&Supertype::Basic)
                && !d.types.has(CardType::Land)
                && (d.colors.0 & !colors.0) == 0
        })
        .map(|d| d.id)
        .collect();
    let lands = size * 2 / 5;
    let mut deck = vec![];
    for i in 0..lands {
        deck.push(chosen[i % chosen.len().max(1)].1);
    }
    while deck.len() < size && !pool.is_empty() {
        deck.push(pool[rng.gen_range(0..pool.len())]);
    }
    deck
}


/// Decks from `FUZZ_DECK_DIR` (text files of `N Card Name` lines; unknown cards are skipped). Empty when the variable is unset.
pub fn real_decks(db: &Arc<CardDb>) -> Vec<Vec<DefId>> {
    let Some(dir) = std::env::var_os("FUZZ_DECK_DIR") else { return vec![] };
    let mut out = vec![];
    let mut files: Vec<_> = std::fs::read_dir(dir).map(|r| r.filter_map(|e| e.ok()).map(|e| e.path()).collect()).unwrap_or_default();
    files.sort();
    for f in files {
        let Ok(text) = std::fs::read_to_string(&f) else { continue };
        let mut deck = vec![];
        for line in text.lines() {
            let Some((n, name)) = line.trim().split_once(' ') else { continue };
            let Ok(n) = n.parse::<usize>() else { continue };
            if let Some(d) = db.find(name.trim()) {
                deck.extend(std::iter::repeat(d).take(n));
            }
        }
        if deck.len() >= 30 {
            out.push(deck);
        }
    }
    out
}

pub fn fuzz_one(db: &Arc<CardDb>, seed: u64, players: usize, max_decisions: u64) -> Result<(bool, u64), FuzzFailure> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed ^ 0x9e3779b97f4a7c15);
    let real = real_decks(db);
    let commander = real.is_empty() && players > 2 && rng.gen_bool(0.5);
    let decks: Vec<DeckSpec> = (0..players)
        .map(|i| {
            let main = if real.is_empty() { random_deck(db, &mut rng, if commander { 60 } else { 40 }) } else { real[rng.gen_range(0..real.len())].clone() };
            DeckSpec { name: format!("P{i}"), main, commanders: vec![], sideboard: vec![], companion: None }
        })
        .collect();
    let mut cfg = if commander { Config::commander() } else { Config::standard() };
    // every eighth game uses free mulligan + scry 3, so its pre-game scry is played too
    cfg.mulligan = if seed % 8 == 3 { MulliganRule::FreeScry3 } else { MulliganRule::London };
    let mut game = Game::new(db.clone(), cfg, decks, seed);
    // a stalemate (nobody can act, e.g. Omen Machine with empty hands) would otherwise advance thousands of turns without a single decision
    game.max_steps = 150_000;
    let mut agents: Vec<Box<dyn Agent>> = (0..players).map(|i| Box::new(RandomAgent::new(seed.wrapping_add(i as u64 * 7919))) as Box<dyn Agent>).collect();
    let trace = std::env::var_os("FUZZ_TRACE").is_some();
    let log_start = game.decision_log.len();
    let started = std::time::Instant::now();
    let r = catch_unwind(AssertUnwindSafe(|| play_game(&mut game, &mut agents, max_decisions, |g| {
        if trace {
            eprintln!("{:?} turn={} step={:?} active={} stack={}", g.events.iter().rev().take(std::env::var("FUZZ_TRACE_N").ok().and_then(|s| s.parse().ok()).unwrap_or(4)).collect::<Vec<_>>(), g.turn, g.step, g.active, g.stack.len());
            eprintln!("BF {:?}", g.battlefield.iter().map(|c| g.def_of(*c).name.to_string()).collect::<Vec<_>>());
        }
        // runaway boards (huge token/activation engines) are cut off rather than ground through
        if started.elapsed().as_secs() >= 10 {
            return Err("__deadline".into());
        }
        g.check_invariants()?;
        g.check_sbas_settled()
    })));
    let fail = |message: String, g: &Game| FuzzFailure { seed, players, message, decisions: g.decision_log[log_start..].to_vec() };
    match r {
        Ok(Ok(out)) => Ok((out.result.is_some(), out.decisions)),
        Ok(Err(e)) if e == "__deadline" => Ok((false, 0)),
        Ok(Err(e)) => Err(fail(e, &game)),
        Err(p) => {
            let msg = if let Some(s) = p.downcast_ref::<&str>() {
                s.to_string()
            } else if let Some(s) = p.downcast_ref::<String>() {
                s.clone()
            } else {
                "panic".into()
            };
            Err(fail(format!("panic: {msg}"), &game))
        }
    }
}

pub fn fuzz(db: &Arc<CardDb>, games: u64, first_seed: u64, max_decisions: u64) -> FuzzReport {
    use rayon::prelude::*;
    let results: Vec<(u64, usize, Result<(bool, u64), FuzzFailure>)> = (0..games)
        .into_par_iter()
        .map(|i| {
            let seed = first_seed + i;
            let players = 2 + (seed % 3) as usize; // 2, 3 or 4
            (seed, players, fuzz_one(db, seed, players, max_decisions))
        })
        .collect();
    let mut rep = FuzzReport::default();
    for (_, _, r) in results {
        rep.games += 1;
        match r {
            Ok((finished, d)) => {
                rep.decisions += d;
                if finished {
                    rep.finished += 1
                } else {
                    rep.timed_out += 1
                }
            }
            Err(f) => rep.failures.push(f),
        }
    }
    rep
}

/// A Commander deck from the bundled preconstructed deck list: its name ("Name (SET)") and card names, one per copy.
pub struct CommanderPrecon {
    pub name: String,
    pub commanders: Vec<String>,
    pub main: Vec<String>,
}

/// The Commander decks in `decks/precons.json` (written by scripts/precons.mjs): every deck with a commander.
pub fn commander_precons(path: &str) -> Vec<CommanderPrecon> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{path}: {e}"));
    let mut out = vec![];
    for d in v["decks"].as_array().into_iter().flatten() {
        let (mut commanders, mut main) = (vec![], vec![]);
        for c in d["cards"].as_array().into_iter().flatten() {
            let (Some(section), Some(qty), Some(name)) = (c[0].as_str(), c[1].as_u64(), c[2].as_str()) else { continue };
            let list = match section {
                "commander" => &mut commanders,
                "main" => &mut main,
                _ => continue,
            };
            list.extend(std::iter::repeat(name.to_string()).take(qty as usize));
        }
        if !commanders.is_empty() {
            out.push(CommanderPrecon { name: format!("{} ({})", d["name"].as_str().unwrap_or("?"), d["code"].as_str().unwrap_or("?")), commanders, main });
        }
    }
    out
}
