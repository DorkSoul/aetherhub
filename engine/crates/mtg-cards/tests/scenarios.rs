//! Runs every scenario file under `cards/tests/` against the compiled card pool.

use mtg_cards::scenario::{load_scenarios, run_scenario};

#[test]
fn all_card_scenarios_pass() {
    let dir = mtg_cards::default_cards_dir();
    let db = mtg_cards::build_db(&dir).unwrap_or_else(|e| panic!("card errors: {e:?}"));
    let mut files: Vec<_> = std::fs::read_dir(dir.join("tests")).unwrap().filter_map(|e| e.ok()).map(|e| e.path()).collect();
    files.sort();
    let (mut n, mut failures) = (0, vec![]);
    for f in files {
        // a card with a note in cards/blocked/ has a known issue: its scenarios document it and may fail
        if dir.join("blocked").join(f.file_name().unwrap()).exists() {
            continue;
        }
        let text = std::fs::read_to_string(&f).unwrap();
        let list = match load_scenarios(&text) {
            Ok(l) => l,
            Err(e) => {
                failures.push(format!("{}: invalid scenario file: {e}", f.display()));
                continue;
            }
        };
        for s in list {
            n += 1;
            if let Err(e) = run_scenario(&db, &s) {
                failures.push(format!("{} / {}: {e}", f.file_stem().unwrap().to_string_lossy(), s.name));
            }
        }
    }
    assert!(failures.is_empty(), "{} of {n} scenarios failed:\n{}", failures.len(), failures.join("\n"));
    assert!(n > 10);
}

/// Runs one scenario (JSON list with a single entry) on a database of just the cards it mentions.
fn run_one(json: &str) -> Result<(), String> {
    let (db, _) = mtg_cards::build_db_subset(&mtg_cards::default_cards_dir(), &[], &[json.to_string()]);
    let list = load_scenarios(json).unwrap();
    run_scenario(&db, &list[0])
}

/// A (non-illegal) attack step must not roll over a combat of the attacking player in which no declaration came
/// (here the only creature is tapped), but an attack in a later turn after an `advance_to` still works.
#[test]
fn runner_attack_does_not_roll_over_a_refused_combat() {
    let rolled = r#"[{"name": "x", "players": [{"battlefield": [{"name": "Grizzly Bears", "tapped": true}]}, {}],
        "steps": [{"attack": [{"card": "Grizzly Bears", "at": "p1"}]}, {"block": []}, {"advance_to": "Main2"}],
        "expect": {"players": [{}, {"life": 18}]}}]"#;
    let e = run_one(rolled).expect_err("the attack rolled over to turn 3");
    assert!(e.contains("no declare attackers decision in turn 1"), "{e}");
    let explicit = r#"[{"name": "x", "players": [{"battlefield": [{"name": "Grizzly Bears", "tapped": true}]}, {}],
        "steps": [{"advance_to": "Main1", "player": 1}, {"advance_to": "Main1"}, {"attack": [{"card": "Grizzly Bears", "at": "p1"}]}, {"block": []}, {"advance_to": "Main2"}],
        "expect": {"turn": 3, "players": [{}, {"life": 18}]}}]"#;
    run_one(explicit).unwrap();
    // the opponent's creature attacking in their next turn is no roll-over
    let theirs = r#"[{"name": "x", "players": [{}, {"battlefield": ["Grizzly Bears"]}],
        "steps": [{"attack": [{"card": "Grizzly Bears", "at": "p0"}]}, {"block": []}, {"advance_to": "Main2"}],
        "expect": {"turn": 2, "players": [{"life": 18}, {}]}}]"#;
    run_one(theirs).unwrap();
}

/// Setup `"sick": true` is real summoning sickness: it wears off when its controller's next turn begins.
#[test]
fn runner_sick_wears_off() {
    let s = r#"[{"name": "x", "players": [{"battlefield": [{"name": "Grizzly Bears", "sick": true}]}, {}],
        "steps": [{"attack": [{"card": "Grizzly Bears", "at": "p1"}], "illegal": true},
                  {"advance_to": "Main1", "player": 1}, {"advance_to": "Main1"}, {"attack": [{"card": "Grizzly Bears", "at": "p1"}]}, {"block": []}, {"advance_to": "Main2"}],
        "expect": {"turn": 3, "players": [{}, {"life": 18}]}}]"#;
    run_one(s).unwrap();
}

/// `advance_to` from the second main phase reaches an additional combat of the same turn.
#[test]
fn runner_advance_to_reaches_extra_combat_after_main2() {
    let s = r#"[{"name": "x", "players": [{"hand": ["Relentless Assault"], "battlefield": ["Mountain", "Mountain", "Mountain", "Mountain"]}, {}],
        "steps": [{"advance_to": "Main2"}, {"cast": "Relentless Assault"}, {"resolve": true}, {"advance_to": "BeginCombat"}],
        "expect": {"turn": 1, "step": "BeginCombat"}}]"#;
    run_one(s).unwrap();
}
