//! `tournament <pairs> <players> <kind> <kind> ...` - agent kinds (see `kinds.rs`) play each other in PAIRS of games.
//! A pair is two games on the same decks and the same shuffles with the kinds' seats rotated, so every kind plays
//! every deck and every seat once. The honest sample size is the number of pairs one kind sweeps (wins both games);
//! split pairs are decided by luck and carry no information. With two kinds the report gives the sweeps, an exact
//! sign test on them and an Elo estimate from them.
//!
//! Decks: `TOURNEY_DECKS=real` uses the decks in `decks/fuzz` (coherent, interactive), default random decks.
//! `TOURNEY_MIRROR=1` gives every seat the same list. `TOURNEY_MAXTURN=n` ends a game at turn n with no winner (stalemates are slow and say nothing about skill). `TOURNEY_LONE=1` (4 seats) plays one seat of the first kind against the second kind in every other seat, the lone seat rotating; reports its win rate against the fair 1/players. `TOURNEY_SEED0=n` shifts every seed (fresh decks and shuffles for confirmation runs). `TOURNEY_JOBS=n` plays n pairs at once (use `threads=1` and
//! `iters=N` agents then, so CPU contention cannot change how hard they think). `TOURNEY_VERBOSE=1` prints each pair.

use mtg_ai::*;
use mtg_engine::*;
use rand::{Rng, SeedableRng};
use rayon::prelude::*;
use std::time::{Duration, Instant};

struct GameOut {
    winner: Option<usize>,
    kinds: Vec<usize>,
    think: Vec<(Duration, u64, Duration)>,
    turns: u32,
}

fn play(db: &std::sync::Arc<CardDb>, decks: &[Vec<DefId>], kinds: &[String], seed: u64, seat_kinds: &[usize], pair: u64) -> GameOut {
    let players = decks.len();
    let k = kinds.len();
    let specs: Vec<DeckSpec> = decks.iter().enumerate().map(|(i, d)| DeckSpec { name: format!("P{i}"), main: d.clone(), commanders: vec![], sideboard: vec![], companion: None }).collect();
    let mut g = Game::new(db.clone(), Config::standard(), specs, seed);
    let kind_of = |seat: usize| seat_kinds[seat];
    let mut agents: Vec<Box<dyn Agent>> = (0..players).map(|s| make_agent(&kinds[kind_of(s)], seed * 131 + s as u64 + 7 * pair).unwrap()).collect();
    let mut think = vec![(Duration::ZERO, 0u64, Duration::ZERO); k];
    let mut n = 0u64;
    let max_turn: u32 = std::env::var("TOURNEY_MAXTURN").ok().and_then(|v| v.parse().ok()).unwrap_or(u32::MAX);
    loop {
        if g.run() == Status::Over || n > 12_000 || g.turn > max_turn {
            break;
        }
        let d = g.decision().cloned().unwrap();
        let s = d.player as usize;
        let t = Instant::now();
        let c = agents[s].decide(&g, &d);
        let el = t.elapsed();
        let e = &mut think[kind_of(s)];
        e.0 += el;
        e.1 += 1;
        e.2 = e.2.max(el);
        if g.apply(c).is_err() {
            g.apply(Choice::Default).unwrap();
        }
        n += 1;
    }
    let winner = match &g.over {
        Some(GameResult::Winner { player }) => Some(kind_of(*player as usize)),
        _ => None,
    };
    GameOut { winner, kinds: (0..players).map(kind_of).collect(), think, turns: g.turn }
}

/// Two-sided exact binomial (p = 1/2) probability of a split at least this uneven.
fn sign_test(a: u64, b: u64) -> f64 {
    let n = a + b;
    if n == 0 {
        return 1.0;
    }
    let k = a.min(b);
    let mut p = 0.0;
    let mut c = 1.0f64; // C(n, 0)
    for i in 0..=k {
        p += c;
        c = c * (n - i) as f64 / (i + 1) as f64;
    }
    (2.0 * p / 2f64.powi(n as i32)).min(1.0)
}

fn main() {
    let _ = rayon::ThreadPoolBuilder::new().stack_size(64 << 20).num_threads(std::env::var("TOURNEY_JOBS").ok().and_then(|s| s.parse().ok()).unwrap_or(1)).build_global();
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!("usage: tournament <pairs> <players> <kind> <kind> ...   e.g. tournament 100 2 mcts:150 heur:150");
        std::process::exit(2);
    }
    let pairs: u64 = args[1].parse().unwrap();
    let players: usize = args[2].parse().unwrap();
    let kinds: Vec<String> = args[3..].to_vec();
    let k = kinds.len();
    let db = mtg_cards::build_db(&mtg_cards::default_cards_dir()).unwrap();
    let real: Vec<Vec<DefId>> = if std::env::var("TOURNEY_DECKS").map_or(false, |v| v == "real") {
        std::env::set_var("FUZZ_DECK_DIR", std::env::var("TOURNEY_DECK_DIR").unwrap_or_else(|_| format!("{}/decks/fuzz", std::env::var("AETHERHUB_ROOT").unwrap_or_else(|_| ".".into()))));
        let r = fuzz::real_decks(&db);
        assert!(!r.is_empty(), "no decks found (set TOURNEY_DECK_DIR)");
        r
    } else {
        vec![]
    };
    let mirror = std::env::var_os("TOURNEY_MIRROR").is_some();
    let seed0: u64 = std::env::var("TOURNEY_SEED0").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    let verbose = std::env::var_os("TOURNEY_VERBOSE").is_some();
    let t0 = Instant::now();
    let lone = std::env::var_os("TOURNEY_LONE").is_some();
    let results: Vec<Vec<GameOut>> = (0..pairs)
        .into_par_iter()
        .map(|pi| {
            let seed = pi + 1 + seed0;
            let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(seed + 5000);
            let mut decks: Vec<Vec<DefId>> = (0..players)
                .map(|_| if real.is_empty() { fuzz::random_deck(&db, &mut rng, 40) } else { real[rng.gen_range(0..real.len())].clone() })
                .collect();
            if mirror {
                for i in 1..players {
                    decks[i] = decks[0].clone();
                }
            }
            if lone {
                // one seat of the first kind against the rest of the second, the lone seat rotating over every seat
                return (0..players)
                    .map(|r| {
                        let sk: Vec<usize> = (0..players).map(|s| if s == r { 0 } else { 1 }).collect();
                        play(&db, &decks, &kinds, seed, &sk, pi * 8 + r as u64)
                    })
                    .collect();
            }
            let a = play(&db, &decks, &kinds, seed, &(0..players).map(|s| s % k).collect::<Vec<_>>(), pi);
            let b = play(&db, &decks, &kinds, seed, &(0..players).map(|s| (s + 1) % k).collect::<Vec<_>>(), pi);
            if verbose {
                eprintln!("pair {pi}: winners {:?} / {:?}  turns {} / {}  [{:.0}s]", a.winner.map(|w| &kinds[w]), b.winner.map(|w| &kinds[w]), a.turns, b.turns, t0.elapsed().as_secs_f32());
            }
            vec![a, b]
        })
        .collect();
    if lone {
        report_lone(&results, &kinds, players, t0.elapsed());
        return;
    }
    let results: Vec<(GameOut, GameOut)> = results.into_iter().map(|mut v| { let b = v.pop().unwrap(); let a = v.pop().unwrap(); (a, b) }).collect();
    let mut wins = vec![0f64; k];
    let mut seats = vec![0u64; k];
    let mut think = vec![(Duration::ZERO, 0u64, Duration::ZERO); k];
    let mut no_winner = 0;
    let mut swept = vec![0u64; k];
    let mut split = 0u64;
    for (a, b) in &results {
        for g in [a, b] {
            for &kd in &g.kinds {
                seats[kd] += 1;
            }
            match g.winner {
                Some(w) => wins[w] += 1.0,
                None => no_winner += 1,
            }
            for (i, t) in g.think.iter().enumerate() {
                think[i].0 += t.0;
                think[i].1 += t.1;
                think[i].2 = think[i].2.max(t.2);
            }
        }
        match (a.winner, b.winner) {
            (Some(x), Some(y)) if x == y => swept[x] += 1,
            _ => split += 1,
        }
    }
    println!("{pairs} pairs ({} games), {players} players, {no_winner} games without a winner, {:.0}s", pairs * 2, t0.elapsed().as_secs_f32());
    for i in 0..k {
        println!(
            "{:<34} wins {:>5.0}/{:<4}  swept pairs {:>4}  think avg {:.1} ms max {:.0} ms",
            kinds[i],
            wins[i],
            pairs * 2,
            swept[i],
            think[i].0.as_secs_f64() * 1000.0 / think[i].1.max(1) as f64,
            think[i].2.as_secs_f64() * 1000.0
        );
    }
    println!("split pairs (decided by luck, no information): {split}");
    if k == 2 {
        let (a, b) = (swept[0], swept[1]);
        let n = a + b;
        if n > 0 {
            let p = a as f64 / n as f64;
            let elo = if a == 0 || b == 0 { 9999.0 * (p - 0.5).signum() } else { 400.0 * (p / (1.0 - p)).log10() };
            let se = (p * (1.0 - p) / n as f64).sqrt();
            println!("{} vs {}: sweeps {a}-{b}, share {:.1}% +/- {:.1}%, sign test p = {:.3}, Elo {:+.0}", kinds[0], kinds[1], 100.0 * p, 196.0 * se, sign_test(a, b), elo);
        }
    }
}

/// Exact two-sided binomial probability of a result at least as unlikely as `wins` out of `n` with success chance `p0`.
fn binom_two_sided(wins: u64, n: u64, p0: f64) -> f64 {
    let mut pmf = vec![0f64; n as usize + 1];
    let (mut c, q) = (1f64, 1.0 - p0);
    for k in 0..=n {
        pmf[k as usize] = c * p0.powi(k as i32) * q.powi((n - k) as i32);
        c = c * (n - k) as f64 / (k + 1) as f64;
    }
    let mine = pmf[wins as usize];
    pmf.iter().filter(|x| **x <= mine * (1.0 + 1e-9)).sum::<f64>().min(1.0)
}

/// `TOURNEY_LONE`: the first kind holds one seat among players of the second kind; fair share is 1 / players.
fn report_lone(results: &[Vec<GameOut>], kinds: &[String], players: usize, elapsed: Duration) {
    let (mut wins, mut games, mut none) = (0u64, 0u64, 0u64);
    for g in results.iter().flatten() {
        games += 1;
        match g.winner {
            Some(0) => wins += 1,
            Some(_) => {}
            None => none += 1,
        }
    }
    let p0 = 1.0 / players as f64;
    let share = wins as f64 / games.max(1) as f64;
    let se = (p0 * (1.0 - p0) / games.max(1) as f64).sqrt();
    println!("{games} games ({} decks x {players} seats), {none} without a winner, {:.0}s", results.len(), elapsed.as_secs_f32());
    println!(
        "lone {} among {} x{}: won {wins}/{games} = {:.1}% (fair {:.1}%, +/- {:.1}% at 95%), p = {:.3}",
        kinds[0],
        kinds.get(1).map(|s| s.as_str()).unwrap_or("?"),
        players - 1,
        100.0 * share,
        100.0 * p0,
        196.0 * se,
        binom_two_sided(wins, games, p0)
    );
    let decided = games - none;
    if decided > 0 {
        println!(
            "among the {decided} games that had a winner: {wins} = {:.1}% (fair {:.1}%), p = {:.6}; {:.0}% of games had no winner",
            100.0 * wins as f64 / decided as f64,
            100.0 * p0,
            binom_two_sided(wins, decided, p0),
            100.0 * none as f64 / games as f64
        );
    }
}
