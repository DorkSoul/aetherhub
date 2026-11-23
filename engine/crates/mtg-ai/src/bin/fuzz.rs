//! `fuzz [games] [first_seed] [max_decisions]` - play random games and report any failure with its seed.

fn main() {
    // deeply nested card definitions need more than rayon's default 2 MiB worker stack
    let _ = rayon::ThreadPoolBuilder::new().stack_size(64 << 20).build_global();
    let args: Vec<String> = std::env::args().collect();
    let games: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(200);
    let first: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);
    let max: u64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(20_000);
    let db = mtg_cards::build_db(&mtg_cards::default_cards_dir()).unwrap_or_else(|e| {
        eprintln!("card errors: {e:#?}");
        std::process::exit(2)
    });
    if std::env::var_os("FUZZ_PROF").is_some() {
        std::thread::spawn(|| loop {
            std::thread::sleep(std::time::Duration::from_secs(5));
            eprintln!("--- prof\n{}", mtg_engine::prof::report());
        });
    }
    let t = std::time::Instant::now();
    let rep = mtg_ai::fuzz::fuzz(&db, games, first, max);
    println!(
        "{} games, {} finished, {} hit the decision limit, {} decisions, {} failures ({:.1}s)",
        rep.games,
        rep.finished,
        rep.timed_out,
        rep.decisions,
        rep.failures.len(),
        t.elapsed().as_secs_f32()
    );
    for f in rep.failures.iter().take(20) {
        println!("FAIL seed={} players={}: {}", f.seed, f.players, f.message);
        if std::env::var_os("FUZZ_TRACE").is_some() {
            for d in f.decisions.iter().rev().take(6).rev() {
                println!("  last decision: {d:?}");
            }
        }
    }
    if !rep.failures.is_empty() {
        std::process::exit(1);
    }
}
