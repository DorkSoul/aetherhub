//! Headless, deterministic Magic: The Gathering rules engine.
//!
//! The engine never blocks: `Game::run()` advances until it needs an answer (`Game::decision()`), and
//! `Game::apply(choice)` supplies it. Everything that decides (human, AI, network, test script) is an agent.
pub mod carddb;
pub mod cast;
pub mod combat;
pub mod coverage;
pub mod damage;
pub mod describe;
pub mod decision;
pub mod driver;
pub mod edit;
pub mod dsl;
pub mod eval;
pub mod ids;
pub mod invariants;
pub mod layers;
pub mod mana;
pub mod pay;
pub mod planechase;
pub mod prof;
pub mod polarity;
pub mod replace;
pub mod replay;
pub mod resolve;
pub mod sba;
pub mod setup;
pub mod state;
pub mod triggers;
pub mod turn;
pub mod view;
pub mod textchange;
pub mod explain;
pub mod types;
pub mod zones;

pub use carddb::{CardDb, CompileError, Def};
pub use decision::*;
pub use edit::EditOp;
pub use ids::*;
pub use replay::GameRecord;
pub use state::*;
