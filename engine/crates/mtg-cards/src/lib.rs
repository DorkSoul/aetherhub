//! Loading, validating and compiling card definitions (one JSON file per card).

use mtg_engine::dsl::CardDef;
use mtg_engine::{CardDb, CompileError};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub mod scenario;
pub mod tokens;

#[derive(Debug)]
pub struct LoadError {
    pub file: PathBuf,
    pub message: String,
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.file.display(), self.message)
    }
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            // scenario tests and notes live beside the cards but are not cards
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if matches!(name, "tests" | "queue" | "blocked" | "wontdo" | "notes") {
                continue;
            }
            collect_files(&p, out);
        } else if p.extension().map_or(false, |e| e == "json") && p.file_name().map_or(false, |n| n != "schema.json" && n != "manifest.json") {
            out.push(p);
        }
    }
}

/// Parse one card file: it holds one card or an array of cards.
fn parse_card_file(text: &str) -> Result<Vec<CardDef>, String> {
    match serde_json::from_str::<CardDef>(text) {
        Ok(c) => Ok(vec![c]),
        Err(_) => serde_json::from_str::<Vec<CardDef>>(text).map_err(|e| e.to_string()),
    }
}

/// Read every card file under `dir`. A file may hold one card or an array of cards.
pub fn load_dir(dir: &Path) -> (Vec<(PathBuf, CardDef)>, Vec<LoadError>) {
    let mut files = vec![];
    collect_files(dir, &mut files);
    let mut cards = vec![];
    let mut errors = vec![];
    for f in files {
        let text = match std::fs::read_to_string(&f) {
            Ok(t) => t,
            Err(e) => {
                errors.push(LoadError { file: f, message: e.to_string() });
                continue;
            }
        };
        match parse_card_file(&text) {
            Ok(v) => cards.extend(v.into_iter().map(|c| (f.clone(), c))),
            Err(message) => errors.push(LoadError { file: f, message }),
        }
    }
    (cards, errors)
}

/// File-name form of a card name (`Aang, Air Nomad` -> `Aang_Air_Nomad`): runs of non-alphanumerics become one `_`.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    let mut last_sep = true;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            last_sep = false;
        } else if !last_sep {
            out.push('_');
            last_sep = true;
        }
    }
    out.trim_matches('_').to_string()
}

/// ASCII-fold common accented Latin letters ("Éomer" -> "Eomer"). File stems are inconsistent about accents (some drop the letter,
/// some fold it), so lookups try `slug(name)` and `slug(fold_accents(name))`.
pub fn fold_accents(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' | 'ā' => 'a',
            'Á' | 'À' | 'Â' | 'Ä' | 'Ã' | 'Å' | 'Ā' => 'A',
            'é' | 'è' | 'ê' | 'ë' | 'ē' => 'e',
            'É' | 'È' | 'Ê' | 'Ë' | 'Ē' => 'E',
            'í' | 'ì' | 'î' | 'ï' | 'ī' => 'i',
            'Í' | 'Ì' | 'Î' | 'Ï' => 'I',
            'ó' | 'ò' | 'ô' | 'ö' | 'õ' | 'ō' => 'o',
            'Ó' | 'Ò' | 'Ô' | 'Ö' | 'Õ' => 'O',
            'ú' | 'ù' | 'û' | 'ü' | 'ū' => 'u',
            'Ú' | 'Ù' | 'Û' | 'Ü' => 'U',
            'ñ' => 'n',
            'ç' => 'c',
            other => other,
        })
        .collect()
}

/// Compile `defs`, dropping cards that don't compile (reported in `notes`) and retrying. `reload` re-reads the same card list:
/// a build consumes its input (so the database is the only copy in memory), so a retry starts again from disk instead of
/// keeping a clone of every card around for the rare failure.
fn compile_dropping_bad(mut defs: Vec<CardDef>, reload: &dyn Fn() -> Vec<CardDef>, notes: &mut Vec<String>) -> Arc<CardDb> {
    let mut all_bad: std::collections::HashSet<String> = std::collections::HashSet::new();
    loop {
        match CardDb::build(defs) {
            Ok(db) => return db,
            Err(errs) => {
                let before = all_bad.len();
                for e in &errs {
                    notes.push(e.to_string());
                    all_bad.insert(e.card.clone());
                }
                defs = reload();
                defs.retain(|d| !all_bad.contains(&d.name));
                if all_bad.len() == before {
                    // nothing removable (e.g. a built-in token): give up with the tokens only
                    return CardDb::build(tokens::builtin_tokens()).unwrap_or_else(|_| CardDb::build(vec![]).unwrap());
                }
            }
        }
    }
}

/// Compile the cards in `dir` plus the built-in tokens into a database.
pub fn build_db(dir: &Path) -> Result<Arc<CardDb>, Vec<String>> {
    let (cards, load_errors) = load_dir(dir);
    let mut errors: Vec<String> = load_errors.iter().map(|e| e.to_string()).collect();
    let mut defs: Vec<CardDef> = tokens::builtin_tokens();
    defs.extend(cards.into_iter().map(|(_, c)| c));
    match CardDb::build(defs) {
        Ok(db) if errors.is_empty() => Ok(db),
        Ok(_) => Err(errors),
        Err(es) => {
            errors.extend(es.iter().map(|e: &CompileError| e.to_string()));
            Err(errors)
        }
    }
}

/// Like `build_db`, but cards that don't compile are left out (and reported) instead of failing the whole build.
/// Used by the per-card tools so one broken file can't block testing of the others.
pub fn build_db_lenient(dir: &Path) -> (Arc<CardDb>, Vec<String>) {
    let gather = || -> (Vec<CardDef>, Vec<String>) {
        let (cards, load_errors) = load_dir(dir);
        let mut defs: Vec<CardDef> = tokens::builtin_tokens();
        defs.extend(cards.into_iter().map(|(_, c)| c));
        (defs, load_errors.iter().map(|e| e.to_string()).collect())
    };
    let (defs, mut notes) = gather();
    let db = compile_dropping_bad(defs, &|| gather().0, &mut notes);
    (db, notes)
}

/// Cards the engine, the scenario runner and the smoke test look up by a literal name, so every subset database holds them.
pub const IMPLICIT_CARDS: &[&str] = &["Plains", "Island", "Swamp", "Mountain", "Forest", "Wastes", "The Initiative", "Grizzly Bears", "Hill Giant", "Serra Angel", "Lightning Bolt", "Divination"];

/// A database holding only the cards named in `names` (card names or file stems) and every card they mention, found by
/// scanning the JSON strings of those cards and of `extra_texts` (scenario files) for names of other card files. A single-card
/// test then needs a few MB instead of the whole pool. The result is only as complete as that name scan: callers should
/// confirm a failure against `build_db_lenient` before reporting it.
pub fn build_db_subset(dir: &Path, names: &[String], extra_texts: &[String]) -> (Arc<CardDb>, Vec<String>) {
    let mut files = vec![];
    collect_files(dir, &mut files);
    // file stems in slug form ("Merfolk_Wizard_1-1_Blue" -> "Merfolk_Wizard_1_1_Blue"), sorted so a front-face name can find "Front_Back"
    let mut by_stem: std::collections::BTreeMap<String, Vec<PathBuf>> = std::collections::BTreeMap::new();
    for f in files {
        if let Some(stem) = f.file_stem().and_then(|s| s.to_str()) {
            by_stem.entry(slug(stem)).or_default().push(f);
        }
    }
    fn strings(v: &serde_json::Value, out: &mut Vec<String>) {
        match v {
            serde_json::Value::String(s) => out.push(s.clone()),
            serde_json::Value::Array(a) => a.iter().for_each(|x| strings(x, out)),
            serde_json::Value::Object(m) => m.values().for_each(|x| strings(x, out)),
            _ => {}
        }
    }
    let mut wanted: Vec<String> = names.to_vec();
    wanted.extend(IMPLICIT_CARDS.iter().map(|s| s.to_string()));
    for t in extra_texts {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(t) {
            strings(&v, &mut wanted);
        }
    }
    let mut taken: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    let mut cards: Vec<CardDef> = vec![];
    let mut notes = vec![];
    while let Some(w) = wanted.pop() {
        // a card's file is its slugged name; "Front // Back" may also be named by its front face alone, and a lone face name
        // ("Fire") may be the front of a split / double-faced file ("Fire_Ice"): take a few prefix matches when nothing matches exactly
        let full = slug(&w);
        let front = slug(w.split(" // ").next().unwrap_or(&w));
        let (ffull, ffront) = (slug(&fold_accents(&w)), slug(&fold_accents(w.split(" // ").next().unwrap_or(&w))));
        let mut paths: Vec<&PathBuf> = vec![];
        if let Some(p) = by_stem.get(&full).or_else(|| by_stem.get(&front)).or_else(|| by_stem.get(&ffull)).or_else(|| by_stem.get(&ffront)) {
            paths.extend(p);
        } else if !full.is_empty() {
            let prefix = format!("{full}_");
            let hits: Vec<&Vec<PathBuf>> = by_stem.range(prefix.clone()..).take_while(|(k, _)| k.starts_with(&prefix)).map(|(_, v)| v).take(9).collect();
            if hits.len() <= 8 {
                paths.extend(hits.into_iter().flatten());
            }
        }
        for p in paths {
            if !taken.insert(p.clone()) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(p) else { continue };
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                strings(&v, &mut wanted);
            }
            match parse_card_file(&text) {
                Ok(v) => cards.extend(v),
                Err(message) => notes.push(format!("{}: {message}", p.display())),
            }
        }
    }
    let mut defs: Vec<CardDef> = tokens::builtin_tokens();
    let have: std::collections::HashSet<String> = defs.iter().map(|d| d.name.clone()).collect();
    defs.extend(cards.into_iter().filter(|c| !have.contains(&c.name)));
    let snapshot = defs.clone();
    let db = compile_dropping_bad(defs, &|| snapshot.clone(), &mut notes);
    (db, notes)
}

/// The playable card pool: every card file that compiles and has no note in `blocked/` (a card with a note there stays
/// out of the pool). Broken files are reported, not fatal. Used by the server.
pub fn build_db_playable(dir: &Path) -> (Arc<CardDb>, Vec<String>) {
    let blocked: std::collections::HashSet<String> = std::fs::read_dir(dir.join("blocked"))
        .map(|rd| rd.filter_map(|e| e.ok()).filter_map(|e| e.path().file_stem().map(|s| s.to_string_lossy().to_string())).collect())
        .unwrap_or_default();
    let gather = || -> (Vec<CardDef>, Vec<String>) {
        let (cards, load_errors) = load_dir(dir);
        let mut notes: Vec<String> = load_errors.iter().map(|e| e.to_string()).collect();
        let mut defs: Vec<CardDef> = tokens::builtin_tokens();
        for (_, c) in cards {
            if !c.token && blocked.contains(&slug(&c.name)) {
                notes.push(format!("{}: left out (known issue, see cards/blocked)", c.name));
                continue;
            }
            defs.push(c);
        }
        (defs, notes)
    };
    let (defs, mut notes) = gather();
    let db = compile_dropping_bad(defs, &|| gather().0, &mut notes);
    (db, notes)
}

/// JSON Schema for card files (generated from the DSL types).
pub fn schema_json() -> String {
    let schema = schemars::schema_for!(CardDef);
    serde_json::to_string_pretty(&schema).unwrap()
}

/// Repository `cards/` directory (relative to this crate).
pub fn default_cards_dir() -> PathBuf {
    // `AETHERHUB_CARDS_DIR` points the tools at another card tree (mutation testing works on throw-away copies)
    if let Some(d) = std::env::var_os("AETHERHUB_CARDS_DIR") {
        return PathBuf::from(d);
    }
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../cards")
}
