//! Named agent kinds for benchmarks: `random`, `heur:250` (determinized search), `mcts:250` (information-set MCTS).
//! The number is the thinking budget per decision in milliseconds. Extra options: `mcts:250:noise=0.1:blunder=0.2:threads=2`.

use crate::{Agent, HeuristicAgent, MctsAgent, MctsConfig, RandomAgent};
use std::time::Duration;

pub fn make_agent(spec: &str, seed: u64) -> Result<Box<dyn Agent>, String> {
    let mut parts = spec.split(':');
    let kind = parts.next().unwrap_or("");
    let budget: u64 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(250);
    let opts: Vec<(&str, f32)> = parts.filter_map(|p| p.split_once('=')).filter_map(|(k, v)| Some((k, v.parse().ok()?))).collect();
    match kind {
        "random" => Ok(Box::new(RandomAgent::new(seed))),
        "heur" => Ok(Box::new(HeuristicAgent::new(seed, Duration::from_millis(budget)))),
        "mcts" => {
            let mut cfg = MctsConfig::new(Duration::from_millis(budget));
            for (k, v) in opts {
                match k {
                    "noise" => cfg.noise = v,
                    "blunder" => cfg.blunder = v,
                    "threads" => cfg.threads = v as usize,
                    "c" => cfg.exploration = v,
                    "horizon" => cfg.horizon_turns = v as u32,
                    "risk" => cfg.risk = v,
                    "iters" => cfg.iters = Some(v as u32),
                    "learned" => cfg.learned = v > 0.0,
                    "onnx" => cfg.onnx = v > 0.0,
                    "moreatk" => cfg.more_attacks = v > 0.0,
                    "margin" => cfg.pass_margin = v,
                    "threat" => cfg.threat = v,
                    "targets" => cfg.all_targets = v > 0.0,
                    "act" => cfg.activate = v > 0.0,
                    _ => return Err(format!("unknown option {k}")),
                }
            }
            Ok(Box::new(MctsAgent::new(seed, cfg)))
        }
        _ => Err(format!("unknown agent kind {spec}")),
    }
}
