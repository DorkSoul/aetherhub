//! Agents that decide for players: random (fuzzing), heuristic and search-based AI.

pub mod agent;
pub mod determinize;
pub mod eval;
pub mod features;
pub mod fuzz;
pub mod heuristic;
pub mod kinds;
pub mod mcts;
pub mod random;
pub mod session;
pub mod smoke;
pub mod textchoice;

pub use agent::{play_game, Agent, PlayOutcome};
pub use heuristic::HeuristicAgent;
pub use kinds::make_agent;
pub use mcts::{MctsAgent, MctsConfig};
pub use random::RandomAgent;
pub use session::*;
