//! `cards validate` compiles every card under `cards/`; `cards schema` prints the JSON Schema; `cards stats` summarises.

use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("validate");
    let dir = args.get(2).map(PathBuf::from).unwrap_or_else(mtg_cards::default_cards_dir);
    match cmd {
        "schema" => println!("{}", mtg_cards::schema_json()),
        "check" => {
            // compile only the given card files (plus the tokens): fast feedback while writing cards
            let files: Vec<PathBuf> = args.iter().skip(2).map(PathBuf::from).collect();
            let mut defs = mtg_cards::tokens::builtin_tokens();
            let mut bad = 0;
            for f in &files {
                match std::fs::read_to_string(f) {
                    Err(e) => {
                        eprintln!("{}: {e}", f.display());
                        bad += 1;
                    }
                    Ok(t) => match serde_json::from_str::<mtg_engine::dsl::CardDef>(&t) {
                        Ok(c) => defs.push(c),
                        Err(e) => {
                            eprintln!("{}: {e}", f.display());
                            bad += 1;
                        }
                    },
                }
            }
            // referenced tokens may live in other card files: add every other card as context
            let (all, _) = mtg_cards::load_dir(&mtg_cards::default_cards_dir());
            let names: std::collections::HashSet<String> = defs.iter().map(|d| d.name.clone()).collect();
            for (_, c) in all {
                if !names.contains(&c.name) {
                    defs.push(c);
                }
            }
            match mtg_engine::CardDb::build(defs) {
                Ok(_) => {}
                Err(errs) => {
                    let wanted: Vec<String> = files
                        .iter()
                        .filter_map(|f| std::fs::read_to_string(f).ok())
                        .filter_map(|t| serde_json::from_str::<mtg_engine::dsl::CardDef>(&t).ok().map(|c| c.name))
                        .collect();
                    for e in errs.iter().filter(|e| wanted.contains(&e.card)) {
                        eprintln!("{e}");
                        bad += 1;
                    }
                }
            }
            if bad > 0 {
                eprintln!("{bad} problem(s)");
                std::process::exit(1);
            }
            println!("ok: {} file(s) compile", files.len());
        }
        "coverage" => {
            // `cards coverage [out.tsv]`: run every card's scenarios and list the spell / triggered / activated abilities that never
            // resolved in any of them (nothing checks those). Static and replacement abilities are not tracked here.
            use mtg_engine::dsl::Ability;
            // `cards coverage <out.tsv|-> [Card_Name...]`: with names, only those cards' scenario files are run
            let out_path = args.get(2).cloned().filter(|p| p != "-");
            let only_cards: Vec<String> = args.iter().skip(3).cloned().collect();
            let cards_dir = mtg_cards::default_cards_dir();
            let db = match mtg_cards::build_db(&cards_dir) {
                Ok(db) => db,
                Err(e) => {
                    e.iter().for_each(|x| eprintln!("{x}"));
                    std::process::exit(1)
                }
            };
            // abilities the card file itself declares (keyword expansions are appended after them)
            let mut declared: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
            for (_, c) in mtg_cards::load_dir(&cards_dir).0 {
                declared.insert(c.name.clone(), if c.abilities.iter().any(|a| matches!(a, Ability::Saga { .. })) { usize::MAX } else { c.abilities.len() });
                if let Some(b) = &c.back {
                    declared.insert(b.name.clone(), b.abilities.len());
                }
            }
            let mut by_slug: std::collections::HashMap<String, mtg_engine::DefId> = std::collections::HashMap::new();
            for d in db.defs.iter().filter(|d| d.is_front && !d.token) {
                by_slug.insert(mtg_cards::slug(&d.name), d.id);
                by_slug.insert(mtg_cards::slug(&mtg_cards::fold_accents(&d.name)), d.id);
                // double-faced / split cards: the test file is named "Front_Back"
                if let Some(b) = d.other_face {
                    by_slug.insert(mtg_cards::slug(&format!("{} {}", d.name, db.def(b).name)), d.id);
                    by_slug.insert(mtg_cards::slug(&mtg_cards::fold_accents(&format!("{} {}", d.name, db.def(b).name))), d.id);
                }
            }
            let mut files: Vec<_> = std::fs::read_dir(cards_dir.join("tests")).map(|r| r.filter_map(|e| e.ok()).map(|e| e.path()).collect()).unwrap_or_default();
            files.sort();
            mtg_engine::coverage::ON.store(true, std::sync::atomic::Ordering::Relaxed);
            let (mut cards, mut cards_gap, mut total, mut missed, mut untested) = (0usize, 0usize, 0usize, 0usize, vec![]);
            let mut rows: Vec<String> = vec![];
            let kind_of = |a: &Ability| match a {
                Ability::Spell { .. } => Some("Spell"),
                Ability::Modal { .. } => Some("Modal"),
                Ability::Triggered { .. } => Some("Triggered"),
                Ability::Activated { .. } => Some("Activated"),
                _ => None,
            };
            let mut tested: std::collections::HashSet<mtg_engine::DefId> = std::collections::HashSet::new();
            for f in &files {
                let stem = f.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                if !only_cards.is_empty() && !only_cards.iter().any(|o| o == stem || mtg_cards::slug(o) == mtg_cards::slug(stem)) {
                    continue;
                }
                let Some(&front) = by_slug.get(&mtg_cards::slug(stem)) else { continue };
                tested.insert(front);
                let text = std::fs::read_to_string(f).unwrap_or_default();
                let _ = mtg_engine::coverage::take();
                if let Ok(list) = mtg_cards::scenario::load_scenarios(&text) {
                    for s in list {
                        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| mtg_cards::scenario::run_scenario(&db, &s)));
                    }
                }
                let hits = mtg_engine::coverage::take();
                let faces: Vec<mtg_engine::DefId> = std::iter::once(front).chain(db.def(front).other_face).collect();
                cards += 1;
                let mut gap = false;
                for id in faces {
                    let d = db.def(id);
                    let limit = declared.get(&d.name).copied().unwrap_or(usize::MAX);
                    for (i, a) in d.abilities.iter().enumerate().take(limit) {
                        let Some(kind) = kind_of(a) else { continue };
                        if d.mana_abilities.contains(&(i as u16)) {
                            continue;
                        }
                        total += 1;
                        // a Room's back half is merged into the front definition after the front's own abilities
                        let merged_room_hit = db.def(front).room && id != front && declared.get(&db.def(front).name).map_or(false, |&fl| fl != usize::MAX && hits.contains(&(front.0, (fl + i) as u16)));
                        if hits.contains(&(id.0, i as u16)) || merged_room_hit {
                            continue;
                        }
                        missed += 1;
                        gap = true;
                        let brief: String = serde_json::to_string(a).unwrap_or_default().chars().take(140).collect();
                        rows.push(format!("{}\t{}\t{}\t{}\t{}", db.def(front).name, d.name, i, kind, brief));
                    }
                }
                if gap {
                    cards_gap += 1;
                }
            }
            for d in db.defs.iter().filter(|d| d.is_front && !d.token) {
                if !tested.contains(&d.id) {
                    untested.push(d.name.clone());
                }
            }
            println!("{cards} cards with scenarios; {cards_gap} have at least one spell/triggered/activated ability no scenario resolves ({missed} of {total} abilities)");
            if only_cards.is_empty() {
                println!("{} cards have no scenario file at all", untested.len());
            } else {
                // with a card filter, show the gaps directly
                for r in &rows {
                    println!("UNCOVERED {r}");
                }
            }
            if let Some(p) = out_path {
                let mut body = String::from("card\tface\tability\tkind\tjson\n");
                body.push_str(&rows.join("\n"));
                std::fs::write(&p, body).expect("write report");
                std::fs::write(format!("{p}.untested"), untested.join("\n")).expect("write untested");
                println!("wrote {p} and {p}.untested");
            }
        }
        "test" => {
            // run scenario files: `cards test` for all, or `cards test <name>...` for the scenarios of those cards
            let only_given = args.len() > 2;
            let cards_dir = mtg_cards::default_cards_dir();
            let tests_dir = cards_dir.join("tests");
            let only: Vec<String> = args.iter().skip(2).cloned().collect();
            let mut files: Vec<_> = std::fs::read_dir(&tests_dir).map(|r| r.filter_map(|e| e.ok()).map(|e| e.path()).collect()).unwrap_or_default();
            files.sort();
            // (stem, scenario file text) for the files asked for
            let wanted: Vec<(String, String)> = files
                .iter()
                .filter_map(|f| {
                    let stem = f.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
                    if !only.is_empty() && !only.iter().any(|o| o == &stem || mtg_cards::slug(o) == stem) {
                        return None;
                    }
                    Some((stem, std::fs::read_to_string(f).unwrap_or_default()))
                })
                .collect();
            let run = |db: &std::sync::Arc<mtg_engine::CardDb>| -> (usize, Vec<String>) {
                let (mut pass, mut failures) = (0, vec![]);
                for (stem, text) in &wanted {
                    match mtg_cards::scenario::load_scenarios(text) {
                        Err(e) => failures.push(format!("FAIL {stem}: invalid scenario file: {e}")),
                        Ok(list) => {
                            for s in list {
                                match mtg_cards::scenario::run_scenario(db, &s) {
                                    Ok(()) => pass += 1,
                                    Err(e) => failures.push(format!("FAIL {stem} / {}: {e}", s.name)),
                                }
                            }
                        }
                    }
                }
                (pass, failures)
            };
            let (pass, failures) = if only_given {
                // testing specific cards: load just those cards and the cards they mention (a few MB, not the whole pool)
                let texts: Vec<String> = wanted.iter().map(|(_, t)| t.clone()).collect();
                let (db, _) = mtg_cards::build_db_subset(&cards_dir, &only, &texts);
                // a panic (an unknown card the name scan missed) counts as a failure, to be confirmed on the full pool
                let (pass, failures) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&db))).unwrap_or_else(|_| (0, vec!["panic in subset database".to_string()]));
                drop(db);
                if failures.is_empty() {
                    (pass, failures)
                } else {
                    // the subset is built by scanning names, so confirm a failure on the full pool (other cards' problems must not get in the way)
                    eprintln!("note: failed on the subset database; re-running on the full pool");
                    for f in failures.iter().take(3) {
                        eprintln!("  subset: {}", f.chars().take(200).collect::<String>());
                    }
                    run(&mtg_cards::build_db_lenient(&cards_dir).0)
                }
            } else {
                match mtg_cards::build_db(&cards_dir) {
                    Ok(db) => run(&db),
                    Err(e) => {
                        for x in e {
                            eprintln!("{x}");
                        }
                        std::process::exit(1)
                    }
                }
            };
            for f in &failures {
                eprintln!("{f}");
            }
            let fail = failures.len();
            println!("{pass} scenario(s) passed, {fail} failed");
            if fail > 0 {
                std::process::exit(1);
            }
        }
        "status" => status(&args),
        "validate" | "stats" => match mtg_cards::build_db(&dir) {
            Ok(db) => {
                println!("ok: {} definitions compiled from {}", db.len(), dir.display());
            }
            Err(errors) => {
                for e in &errors {
                    eprintln!("{e}");
                }
                eprintln!("{} error(s)", errors.len());
                std::process::exit(1);
            }
        },
        other => {
            eprintln!("unknown command {other}; use validate | schema | stats");
            std::process::exit(2);
        }
    }
}

fn slug(n: &str) -> String {
    n.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect::<String>().split('_').filter(|x| !x.is_empty()).collect::<Vec<_>>().join("_")
}

/// `cards status [queue.txt] [--write]`: where every card of the queue stands (missing / blocked / validated / tested / failing).
fn status(args: &[String]) {
    use std::collections::BTreeMap;
    let root = mtg_cards::default_cards_dir();
    let queue_path = args.iter().skip(2).find(|a| !a.starts_with("--")).map(PathBuf::from).unwrap_or_else(|| root.join("queue/priority.txt"));
    let write = args.iter().any(|a| a == "--write");
    let names: Vec<String> = std::fs::read_to_string(&queue_path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split('\t').next())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let db = mtg_cards::build_db(&root);
    let db = match db {
        Ok(d) => Some(d),
        Err(e) => {
            eprintln!("warning: some cards don't compile ({} problem(s)); first: {}", e.len(), e.first().cloned().unwrap_or_default());
            None
        }
    };
    // blocked notes
    let mut blocked: BTreeMap<String, Vec<String>> = BTreeMap::new();
    if let Ok(rd) = std::fs::read_dir(root.join("blocked")) {
        for e in rd.filter_map(|e| e.ok()) {
            if let Ok(t) = std::fs::read_to_string(e.path()) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&t) {
                    let name = v["name"].as_str().unwrap_or("").to_string();
                    let needs = v["needs"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
                    blocked.insert(name, needs);
                }
            }
        }
    }
    // scenario results per test file
    let mut results: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    if let Some(db) = &db {
        if let Ok(rd) = std::fs::read_dir(root.join("tests")) {
            for e in rd.filter_map(|e| e.ok()) {
                let stem = e.path().file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
                let text = std::fs::read_to_string(e.path()).unwrap_or_default();
                match mtg_cards::scenario::load_scenarios(&text) {
                    Ok(list) => {
                        let pass = list.iter().filter(|s| mtg_cards::scenario::run_scenario(db, s).is_ok()).count();
                        results.insert(slug(&stem), (pass, list.len()));
                    }
                    Err(_) => {
                        results.insert(slug(&stem), (0, 1));
                    }
                }
            }
        }
    }
    let mut out: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    let mut needs: BTreeMap<String, usize> = BTreeMap::new();
    for n in &names {
        let compiled = db.as_ref().map_or(false, |d| d.find(n).is_some());
        let sc = results.get(&slug(n)).copied();
        let status = if compiled {
            match sc {
                Some((p, t)) if p == t && t > 0 => "tested",
                Some(_) => "failing",
                None => "validated",
            }
        } else if blocked.contains_key(n) {
            "blocked"
        } else {
            "missing"
        };
        *counts.entry(status).or_default() += 1;
        let mut entry = serde_json::json!({ "status": status });
        if let Some((p, t)) = sc {
            entry["scenarios"] = serde_json::json!({ "passed": p, "total": t });
        }
        if let Some(nd) = blocked.get(n) {
            entry["needs"] = serde_json::json!(nd);
            if status == "blocked" {
                for x in nd {
                    *needs.entry(x.clone()).or_default() += 1;
                }
            }
        }
        out.insert(n.clone(), entry);
    }
    let mut needs_sorted: Vec<(String, usize)> = needs.into_iter().collect();
    needs_sorted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let doc = serde_json::json!({ "queue": names.len(), "counts": counts, "needs": needs_sorted, "cards": out });
    if write {
        let p = root.join("manifest.json");
        std::fs::write(&p, serde_json::to_string_pretty(&doc).unwrap() + "\n").unwrap();
        println!("wrote {}", p.display());
    }
    println!("{} cards in {}: {}", names.len(), queue_path.display(), serde_json::to_string(&counts).unwrap());
    for (k, n) in needs_sorted.iter().take(25) {
        println!("  needs {k}: {n}");
    }
}
