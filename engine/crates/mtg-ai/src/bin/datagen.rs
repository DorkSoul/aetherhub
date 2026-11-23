//! `datagen <games> <players> <out.csv> [kind]` - self-play games; at the first decision of every turn one row per seat:
//! `game,turn,seat,won,f0..fN`. `won` is 1 for the winner (games with no winner are dropped).
//! Half the games use the real decks mirrored, half random decks. `TOURNEY_JOBS=n` games at once.

use mtg_ai::features::features;
use mtg_ai::*;
use mtg_engine::*;
use rand::{Rng, SeedableRng};
use rayon::prelude::*;
use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let games: u64 = args[1].parse().unwrap();
    let players: usize = args[2].parse().unwrap();
    let out = &args[3];
    let kind = args.get(4).cloned().unwrap_or_else(|| "mcts:100:threads=1:iters=60".into());
    let jobs = std::env::var("TOURNEY_JOBS").ok().and_then(|s| s.parse().ok()).unwrap_or(1);
    let _ = rayon::ThreadPoolBuilder::new().stack_size(64 << 20).num_threads(jobs).build_global();
    let db = mtg_cards::build_db(&mtg_cards::default_cards_dir()).unwrap();
    std::env::set_var("FUZZ_DECK_DIR", format!("{}/decks/fuzz", std::env::var("AETHERHUB_ROOT").unwrap_or_else(|_| ".".into())));
    let real = fuzz::real_decks(&db);
    let seed0: u64 = std::env::var("DATAGEN_SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(0);
    let rows: Vec<String> = (0..games)
        .into_par_iter()
        .flat_map(|gi| {
            let seed = seed0 + gi + 1;
            let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(seed + 77_000);
            let decks: Vec<Vec<DefId>> = if gi % 2 == 0 && !real.is_empty() {
                let d = real[rng.gen_range(0..real.len())].clone();
                vec![d; players]
            } else {
                (0..players).map(|_| fuzz::random_deck(&db, &mut rng, 40)).collect()
            };
            let specs: Vec<DeckSpec> = decks.into_iter().enumerate().map(|(i, d)| DeckSpec { name: format!("P{i}"), main: d, commanders: vec![], sideboard: vec![], companion: None }).collect();
            let mut g = Game::new(db.clone(), Config::standard(), specs, seed);
            let mut agents: Vec<Box<dyn Agent>> = (0..players).map(|s| make_agent(&kind, seed * 131 + s as u64).unwrap()).collect();
            let mut recs: Vec<(u32, u8, Vec<f32>)> = vec![];
            let mut last_turn = 0u32;
            let mut n = 0u64;
            loop {
                if g.run() == Status::Over || n > 12_000 {
                    break;
                }
                let d = g.decision().cloned().unwrap();
                if g.turn != last_turn {
                    last_turn = g.turn;
                    if g.turn >= 2 {
                        for s in 0..players {
                            recs.push((g.turn, s as u8, features(&g, s as PlayerId)));
                        }
                    }
                }
                let c = agents[d.player as usize].decide(&g, &d);
                if g.apply(c).is_err() {
                    g.apply(Choice::Default).unwrap();
                }
                n += 1;
            }
            let Some(GameResult::Winner { player }) = g.over.clone() else { return vec![] };
            recs.into_iter()
                .map(|(turn, seat, f)| {
                    let won = if seat as PlayerId == player { 1 } else { 0 };
                    let fs: Vec<String> = f.iter().map(|x| format!("{x:.4}")).collect();
                    format!("{gi},{turn},{seat},{won},{}", fs.join(","))
                })
                .collect::<Vec<_>>()
        })
        .collect();
    let mut file = std::fs::File::create(out).unwrap();
    for r in &rows {
        writeln!(file, "{r}").unwrap();
    }
    eprintln!("{} rows from {games} games -> {out}", rows.len());
}
