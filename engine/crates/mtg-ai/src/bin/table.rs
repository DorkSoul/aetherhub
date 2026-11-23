//! With `TABLE_DECKS=<dir of deck json>` (decks/precons) it plays Commander (40 life) with those decks, four at a time, every deck at least once when `games` is omitted.
//! `table [games] [budget_ms] [players]` (env `TABLE_SEED` first seed, `TABLE_GAME_SECS` wall-clock limit per game (default 300), `TABLE_LOG` log dir, default `target/table-logs`) - every seat is a heuristic AI (default 4 players, free-for-all); prints each game's outcome and any illegal answers.

use mtg_ai::*;
use mtg_engine::*;
use rand::SeedableRng;
use std::io::Write;
use std::time::{Duration, Instant};

fn main() {
    let _ = rayon::ThreadPoolBuilder::new().stack_size(64 << 20).build_global();
    let args: Vec<String> = std::env::args().collect();
    let precons = load_precons_from_env();
    let default_games = if precons.is_empty() { 4 } else { (precons.len() as u64 + 3) / 4 };
    let games: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(default_games);
    let budget: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(200);
    let players: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(4);
    let db = mtg_cards::build_db(&mtg_cards::default_cards_dir()).unwrap();
    let mut illegal: std::collections::BTreeMap<String, u64> = Default::default();
    let (mut finished, mut stuck) = (0, 0);
    let first: u64 = std::env::var("TABLE_SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(0);
    let dir = std::path::PathBuf::from(std::env::var("TABLE_LOG").unwrap_or_else(|_| "target/table-logs".into()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut deck_stats: std::collections::BTreeMap<String, (u32, u32)> = Default::default();
    let order: Vec<usize> = {
        use rand::seq::SliceRandom;
        let mut o: Vec<usize> = (0..precons.len()).collect();
        o.shuffle(&mut rand_chacha::ChaCha8Rng::seed_from_u64(99));
        o
    };
    let commander_mode = !precons.is_empty();
    let game_secs: u64 = std::env::var("TABLE_GAME_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(300);
    let decision_limit: u64 = if commander_mode { 60_000 } else { 30_000 };
    for seed in first..first + games {
        // one log per game, written and flushed line by line: a crash, freeze or loop leaves everything up to the last decision on disk
        let mut log = std::fs::File::create(dir.join(format!("game-{seed}.log"))).unwrap();
        let _ = writeln!(log, "seed {seed} players {players} budget {budget}ms");
        let t = Instant::now();
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(seed + 5000);
        let pod: Vec<usize> = (0..players).map(|i| order[((seed as usize) * players + i) % order.len().max(1)]).collect();
        let decks: Vec<DeckSpec> = if commander_mode {
            pod.iter().map(|&k| precons[k].spec(&db)).collect()
        } else {
            (0..players).map(|i| DeckSpec { name: format!("P{i}"), main: fuzz::random_deck(&db, &mut rng, 40), commanders: vec![], sideboard: vec![], companion: None }).collect()
        };
        for (i, d) in decks.iter().enumerate() {
            let _ = writeln!(log, "seat {i}: {} ({} cards, {} commander(s))", d.name, d.main.len(), d.commanders.len());
        }
        let mut g = Game::new(db.clone(), if commander_mode { Config::commander() } else { Config::standard() }, decks, seed);
        let mut ais: Vec<HeuristicAgent> = (0..players).map(|i| HeuristicAgent::new(seed * 7 + i as u64, Duration::from_millis(budget))).collect();
        let mut n = 0u64;
        let mut notices = 0;
        let mut seen_events = 0usize;
        let mut attacked = vec![0u32; players];
        loop {
            if g.run() == Status::Over || n > decision_limit || t.elapsed().as_secs() > game_secs {
                if g.over.is_none() {
                    let _ = writeln!(log, "STOPPED unfinished after {n} decisions, {}s", t.elapsed().as_secs());
                }
                break;
            }
            if g.events.len() < seen_events {
                seen_events = 0;
            }
            for e in g.events.iter().skip(seen_events) {
                if matches!(e, Event::Notice { .. }) {
                    notices += 1;
                }
                let line = format!("{e:?}");
                let _ = writeln!(log, "  event {}", named(&g, &line).chars().take(260).collect::<String>());
            }
            seen_events = g.events.len();
            let d = g.decision().cloned().unwrap();
            let _ = writeln!(log, "#{n} t{} {:?} active p{} life {:?} stack {} | p{} asked {}", g.turn, g.step, g.active, (0..players).map(|p| g.player(p as PlayerId).life).collect::<Vec<_>>(), g.stack.len(), d.player, named(&g, &short(&d.kind)));
            let c = ais[d.player as usize].decide(&g, &d);
            let _ = writeln!(log, "    -> {}", named(&g, &format!("{c:?}")));
            if let Choice::Attackers { attacks } = &c {
                for (_, o) in attacks {
                    if let Obj::Player { id: p } = o {
                        attacked[*p as usize] += 1;
                    }
                }
            }
            if let Err(e) = g.apply(c) {
                let _ = writeln!(log, "    REJECTED ({e:?}); using the default instead");
                let k = format!("{:?}", d.kind);
                let k = k.split(|ch: char| ch == ' ' || ch == '{' || ch == '(').next().unwrap_or("").to_string();
                *illegal.entry(k).or_insert(0u64) += 1;
                g.apply(Choice::Default).unwrap();
            }
            n += 1;
        }
        let lives: Vec<i32> = (0..players).map(|p| g.player(p as PlayerId).life).collect();
        if g.over.is_some() {
            finished += 1;
        } else {
            stuck += 1;
        }
        for (i, &k) in pod.iter().enumerate().filter(|_| commander_mode) {
            let e = deck_stats.entry(precons[k].name.clone()).or_default();
            e.0 += 1;
            if matches!(&g.over, Some(GameResult::Winner { player }) if *player as usize == i) {
                e.1 += 1;
            }
        }
        let _ = writeln!(log, "END turn {} result {:?} lives {lives:?} decisions {n}", g.turn, g.over);
        println!("game {seed}: turn {} result {:?} lives {lives:?} attackers-sent-at-player {attacked:?} decisions {n} notices {notices} {:.1}s", g.turn, g.over, t.elapsed().as_secs_f32());
    }
    if commander_mode {
        let seen = deck_stats.len();
        println!("{seen} of {} precon decks played; wins/games per deck:", precons.len());
        for (name, (played, won)) in &deck_stats {
            println!("  {name}: {won}/{played}");
        }
    }
    println!("{games} games of {players} AI seats: {finished} finished, {stuck} hit the decision limit. illegal AI answers: {illegal:?}");
}

/// Replace every `CardId(n)` in a debug string with the card's name, so the log reads without a lookup.
fn named(g: &Game, s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find("CardId(") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 7..];
        match after.find(')').and_then(|j| after[..j].parse::<u32>().ok().map(|n| (n, j))) {
            Some((n, j)) if (n as usize) < g.cards.len() => {
                out.push_str(&format!("{}#{n}", g.card_name(CardId(n))));
                rest = &after[j + 1..];
            }
            _ => {
                out.push_str("CardId(");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// A decision's kind and prompt on one line (long option lists cut).
fn short(k: &DecisionKind) -> String {
    let s = format!("{k:?}");
    if s.len() > 300 { format!("{}...", s.chars().take(300).collect::<String>()) } else { s }
}

/// A precon read from `decks/precons/*.json`.
struct Precon {
    name: String,
    commanders: Vec<String>,
    main: Vec<String>,
}

impl Precon {
    fn spec(&self, db: &CardDb) -> DeckSpec {
        let ids = |names: &[String]| -> Vec<DefId> { names.iter().filter_map(|n| db.find(n)).collect() };
        DeckSpec { name: self.name.clone(), main: ids(&self.main), commanders: ids(&self.commanders), sideboard: vec![], companion: None }
    }
}

/// `TABLE_DECKS=<dir>`: every deck file in it that has all its commanders in the card database; the coverage of the rest is printed.
fn load_precons_from_env() -> Vec<Precon> {
    let Ok(dir) = std::env::var("TABLE_DECKS") else { return vec![] };
    let db = mtg_cards::build_db(&mtg_cards::default_cards_dir()).unwrap();
    let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().map_or(false, |x| x == "json")).collect();
    files.sort();
    let mut out = vec![];
    let (mut skipped, mut partial) = (0, 0);
    for f in files {
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&f).unwrap()).unwrap();
        let strings = |k: &str| -> Vec<String> { v[k].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default() };
        let p = Precon { name: format!("{} ({})", v["name"].as_str().unwrap_or("?"), v["code"].as_str().unwrap_or("?")), commanders: strings("commanders"), main: strings("main") };
        let missing: Vec<&String> = p.commanders.iter().chain(p.main.iter()).filter(|n| db.find(n).is_none()).collect();
        if p.commanders.is_empty() || p.commanders.iter().any(|n| db.find(n).is_none()) {
            println!("SKIP {}: commander not in the card database ({:?})", p.name, missing);
            skipped += 1;
            continue;
        }
        if !missing.is_empty() {
            let mut m: Vec<&&String> = missing.iter().collect();
            m.dedup();
            println!("PARTIAL {}: {} card(s) not in the card database: {:?}", p.name, missing.len(), m);
            partial += 1;
        }
        out.push(p);
    }
    println!("{} precon decks loaded ({partial} with missing cards, {skipped} skipped)", out.len());
    out
}
