//! `smoke [--all | names...]` - smoke-test cards (never crash, hang or corrupt the game).

fn main() {
    // deeply nested card definitions (Cultivator Colossus ...) need more than rayon's default 2 MiB worker stack
    let _ = rayon::ThreadPoolBuilder::new().stack_size(64 << 20).build_global();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let all = args.is_empty() || args.iter().any(|a| a == "--all");
    let db = if all {
        match mtg_cards::build_db(&mtg_cards::default_cards_dir()) {
            Ok(d) => d,
            Err(e) => {
                for x in e {
                    eprintln!("{x}");
                }
                std::process::exit(2)
            }
        }
    } else {
        // specific cards: load just those and the cards they mention (the whole pool is gigabytes of start-up for one card)
        mtg_cards::build_db_subset(&mtg_cards::default_cards_dir(), &args, &[]).0
    };
    let names: Vec<String> = if args.is_empty() || args.iter().any(|a| a == "--all") {
        db.defs.iter().filter(|d| d.is_front && !d.token).map(|d| d.name.clone()).collect()
    } else {
        args
    };
    // keep panic messages out of the way: failures are reported per card
    std::panic::set_hook(Box::new(|_| {}));
    // every card is independent: run them across all cores (results keep the card order)
    use rayon::prelude::*;
    let run = |db: &std::sync::Arc<mtg_engine::CardDb>| {
        let results: Vec<_> = names.par_iter().map(|n| mtg_ai::smoke::smoke_card(db, n)).collect();
        results
    };
    let mut results = run(&db);
    if !all && results.iter().any(|r| r.is_err()) {
        // the subset is built by scanning names: confirm a failure against the whole pool (tolerating other cards' problems)
        drop(db);
        let full = mtg_cards::build_db_lenient(&mtg_cards::default_cards_dir()).0;
        results = run(&full);
    }
    let (mut ok, mut bad, mut unexercised) = (0, 0, vec![]);
    for (n, r) in names.iter().zip(results) {
        match r {
            Ok(r) => {
                ok += 1;
                // cards with no legal way to be played outside their draft (Regicide: no color was chosen, so it has no legal target)
                if !r.exercised && n != "Regicide" {
                    unexercised.push(n.clone());
                }
            }
            Err(e) => {
                bad += 1;
                println!("FAIL {e}");
            }
        }
    }
    println!("{ok} ok, {bad} failed, {} never cast/activated/entered play", unexercised.len());
    if std::env::var("VERBOSE").is_ok() && !unexercised.is_empty() {
        println!("never exercised: {}", unexercised.join(", "));
    }
    if bad > 0 {
        std::process::exit(1);
    }
}
