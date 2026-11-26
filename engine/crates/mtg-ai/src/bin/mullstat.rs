//! `mullstat [games]` - plays only the opening (mulligans, bottoming, scries) with the heuristic AI in every seat and prints,
//! for every mulligan rule, how many lands the kept hands hold, how many mulligans were taken and how often a player is
//! short of three lands (or holds 70%+ lands) by the third draw.
//! `MULL_DECKS=<dir of deck json>` (decks/precons): four-player Commander with those decks; without it two-player games
//! with the 60-card decks in `FUZZ_DECK_DIR` (decks/fuzz). `MULL_CUT=n` takes n lands out of every deck.

use mtg_ai::heuristic::policy;
use mtg_ai::mulligan::is_land;
use mtg_ai::*;
use mtg_engine::types::*;
use mtg_engine::*;
use std::collections::BTreeMap;
use std::sync::Arc;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let games: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(200);
    let db = mtg_cards::build_db(&mtg_cards::default_cards_dir()).unwrap();
    let commander = std::env::var("MULL_DECKS").is_ok();
    let decks: Vec<(Vec<DefId>, Vec<DefId>)> = if commander { load_precons(&db) } else { fuzz::real_decks(&db).into_iter().map(|d| (d, vec![])).collect() };
    let cut: usize = std::env::var("MULL_CUT").ok().and_then(|s| s.parse().ok()).unwrap_or(0);
    let decks: Vec<(Vec<DefId>, Vec<DefId>)> = decks
        .into_iter()
        .map(|(mut main, cmd)| {
            for _ in 0..cut {
                if let Some(i) = main.iter().position(|d| db.def(*d).types.has(CardType::Land)) {
                    main.remove(i);
                }
            }
            (main, cmd)
        })
        .collect();
    println!("{} decks, {cut} lands cut from each", decks.len());
    let players = if commander { 4 } else { 2 };
    let rules = [MulliganRule::London, MulliganRule::LondonFreeFirst, MulliganRule::FreeScry3, MulliganRule::Vancouver, MulliganRule::Paris, MulliganRule::Free];
    for rule in rules {
        // kept hands: (hand size, lands) -> count; mulligans taken -> count
        let mut kept: BTreeMap<(usize, usize), u32> = BTreeMap::new();
        let mut mulls: BTreeMap<u8, u32> = BTreeMap::new();
        let mut on_top: BTreeMap<usize, u32> = BTreeMap::new();
        let mut bad = 0u32;
        let mut total = 0u32;
        let (mut screw, mut flood, mut rejected) = (0u32, 0u32, 0u32);
        for seed in 0..games {
            let specs: Vec<DeckSpec> = (0..players)
                .map(|i| {
                    let (main, cmd) = &decks[((seed as usize) * players + i * 7 + i) % decks.len()];
                    DeckSpec { name: format!("P{i}"), main: main.clone(), commanders: cmd.clone(), sideboard: vec![], companion: None }
                })
                .collect();
            let base = if commander { Config::commander() } else { Config::standard() };
            let mut g = Game::new(db.clone(), Config { mulligan: rule, ..base }, specs, seed);
            let mut n = 0;
            loop {
                g.run();
                let Some(d) = g.decision().cloned() else { break };
                if matches!(d.kind, DecisionKind::Priority { .. }) || n > 500 {
                    break;
                }
                let c = policy(&g, &d);
                if g.apply(c).is_err() {
                    rejected += 1;
                    let _ = g.apply(g.default_choice(&d));
                }
                n += 1;
            }
            for p in 0..players {
                let pl = &g.players[p];
                let lands = pl.hand.iter().filter(|c| is_land(&g, **c)).count();
                *kept.entry((pl.hand.len(), lands)).or_default() += 1;
                *mulls.entry(pl.mulligans).or_default() += 1;
                let top = pl.library.iter().rev().take(3).filter(|c| is_land(&g, **c)).count();
                *on_top.entry(top).or_default() += 1;
                total += 1;
                // by the third draw: the hand plus the next three cards (the first player's turn-one draw counted)
                let k = 10usize.saturating_sub(pl.hand.len()).min(3);
                let seen = lands + pl.library.iter().rev().take(k).filter(|c| is_land(&g, **c)).count();
                if seen < 3 {
                    screw += 1;
                }
                if seen * 10 >= 7 * (pl.hand.len() + k) {
                    flood += 1;
                }
                if lands <= 1 || lands + 1 >= pl.hand.len() {
                    bad += 1;
                }
            }
        }
        println!("\n== {rule:?}: {total} kept hands, {bad} with <=1 land or <=1 spell ({:.1}%)", 100.0 * bad as f32 / total as f32);
        let line: Vec<String> = kept.iter().map(|((h, l), c)| format!("{h}c/{l}L:{c}")).collect();
        println!("by turn 3: {:.1}% short of three lands, {:.1}% with 70%+ lands; {rejected} answers rejected", 100.0 * screw as f32 / total as f32, 100.0 * flood as f32 / total as f32);
        println!("kept (cards/lands:count): {}", line.join(" "));
        println!("mulligans taken: {:?}", mulls);
        println!("lands in the top three after keeping: {:?}", on_top);
    }
}

fn load_precons(db: &Arc<CardDb>) -> Vec<(Vec<DefId>, Vec<DefId>)> {
    let dir = std::env::var("MULL_DECKS").unwrap();
    let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().map_or(false, |x| x == "json")).collect();
    files.sort();
    let mut out = vec![];
    for f in files {
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&f).unwrap()).unwrap();
        let ids = |k: &str| -> Vec<DefId> { v[k].as_array().map(|a| a.iter().filter_map(|x| x.as_str().and_then(|n| db.find(n))).collect()).unwrap_or_default() };
        let (main, cmd) = (ids("main"), ids("commanders"));
        if !cmd.is_empty() && main.len() >= 95 {
            out.push((main, cmd));
        }
    }
    out
}
