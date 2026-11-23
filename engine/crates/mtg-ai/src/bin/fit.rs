//! `fit <data.csv> <out.json> [l2]` - L2-regularised logistic regression by full-batch gradient descent with momentum.
//! Rows are grouped by game for a held-out check (every 5th game is held out); prints held-out log-loss and accuracy.

use std::io::BufRead;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let l2: f64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0.001);
    let file = std::io::BufReader::new(std::fs::File::open(&args[1]).unwrap());
    let (mut train, mut test): (Vec<(Vec<f64>, f64)>, Vec<(Vec<f64>, f64)>) = (vec![], vec![]);
    for line in file.lines().map_while(Result::ok) {
        let p: Vec<&str> = line.split(',').collect();
        if p.len() < 6 {
            continue;
        }
        let game: u64 = p[0].parse().unwrap();
        let y: f64 = p[3].parse().unwrap();
        let x: Vec<f64> = p[4..].iter().map(|s| s.parse().unwrap()).collect();
        if game % 5 == 4 { test.push((x, y)) } else { train.push((x, y)) }
    }
    let n = train[0].0.len();
    let mut w = vec![0f64; n];
    let mut v = vec![0f64; n];
    let sig = |z: f64| 1.0 / (1.0 + (-z).exp());
    let lr = 0.5;
    for it in 0..3000 {
        let mut grad = vec![0f64; n];
        for (x, y) in &train {
            let z: f64 = x.iter().zip(&w).map(|(a, b)| a * b).sum();
            let e = sig(z) - y;
            for i in 0..n {
                grad[i] += e * x[i];
            }
        }
        for i in 0..n {
            grad[i] = grad[i] / train.len() as f64 + if i + 1 < n { l2 * w[i] } else { 0.0 };
            v[i] = 0.9 * v[i] - lr * grad[i];
            w[i] += v[i];
        }
        if it % 500 == 0 {
            eprintln!("iter {it}: held-out {:?}", score(&test, &w));
        }
    }
    println!("train {:?} held-out {:?} ({} train rows, {} test rows)", score(&train, &w), score(&test, &w), train.len(), test.len());
    std::fs::write(&args[2], format!("[{}]", w.iter().map(|x| format!("{x:.5}")).collect::<Vec<_>>().join(","))).unwrap();
}

fn score(rows: &[(Vec<f64>, f64)], w: &[f64]) -> (f64, f64) {
    let (mut ll, mut ok) = (0.0, 0.0);
    for (x, y) in rows {
        let p = 1.0 / (1.0 + (-x.iter().zip(w).map(|(a, b)| a * b).sum::<f64>()).exp());
        let p = p.clamp(1e-6, 1.0 - 1e-6);
        ll -= y * p.ln() + (1.0 - y) * (1.0 - p).ln();
        if (p > 0.5) == (*y > 0.5) {
            ok += 1.0;
        }
    }
    (ll / rows.len().max(1) as f64, ok / rows.len().max(1) as f64)
}
