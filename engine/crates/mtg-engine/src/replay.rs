//! Determinism: a game is its configuration, decks, seed and the ordered list of decisions answered.

use crate::decision::*;
use crate::state::*;
use crate::types::*;
use std::sync::Arc;

struct Fnv(u64);
impl Fnv {
    fn new() -> Fnv {
        Fnv(0xcbf29ce484222325)
    }
    fn u(&mut self, v: u64) {
        for b in v.to_le_bytes() {
            self.0 ^= b as u64;
            self.0 = self.0.wrapping_mul(0x100000001b3);
        }
    }
    fn i(&mut self, v: i64) {
        self.u(v as u64)
    }
}

impl Game {
    /// A hash of everything that matters in the game state (not of the control-flow bookkeeping).
    pub fn state_hash(&self) -> u64 {
        let mut h = Fnv::new();
        h.u(self.turn as u64);
        h.u(self.active as u64);
        h.u(self.step as u64);
        h.u(self.stack.len() as u64);
        for s in &self.stack {
            h.u(s.source.0 as u64);
            h.i(s.x as i64);
            for t in &s.targets {
                h.u(t.zcc as u64);
            }
        }
        for p in &self.players {
            h.i(p.life as i64);
            h.i(p.poison as i64);
            h.u(p.lost as u64);
            for z in [&p.library, &p.hand, &p.graveyard, &p.exile, &p.command] {
                h.u(z.len() as u64);
                for c in z.iter() {
                    h.u(c.0 as u64);
                }
            }
            for n in p.pool.0 {
                h.u(n as u64);
            }
        }
        for id in &self.battlefield {
            let c = &self.cards[id.0 as usize];
            h.u(id.0 as u64);
            h.u(c.tapped as u64);
            h.i(c.damage as i64);
            h.i(c.cur.power as i64);
            h.i(c.cur.toughness as i64);
            h.u(c.cur.controller as u64);
            for (k, n) in &c.counters {
                h.u(*k as u64);
                h.u(*n as u64);
            }
            h.u(c.attached_to.map_or(u64::MAX, |a| a.0 as u64));
        }
        h.u(self.ts);
        h.0
    }

    /// Rebuild a game by replaying the decisions. Stops (without error) if the log runs out.
    pub fn replay(db: Arc<crate::CardDb>, cfg: Config, decks: Vec<DeckSpec>, seed: u64, log: &[Choice]) -> Result<Game, String> {
        let mut g = Game::new(db, cfg, decks, seed);
        g.replay_onto(log)?;
        Ok(g)
    }

    /// Feed a list of decisions into a freshly created game.
    pub fn replay_onto(&mut self, log: &[Choice]) -> Result<(), String> {
        for (i, c) in log.iter().enumerate() {
            match self.run() {
                Status::Over => return Ok(()),
                Status::NeedDecision => {}
            }
            self.apply(c.clone()).map_err(|e| format!("decision {i} rejected on replay: {e}"))?;
        }
        self.run();
        Ok(())
    }
}

/// Everything needed to recreate a game from nothing.
#[derive(Clone, Debug)]
pub struct GameRecord {
    pub cfg: Config,
    pub decks: Vec<DeckSpec>,
    pub seed: u64,
    pub log: Vec<Choice>,
}

impl GameRecord {
    pub fn replay(&self, db: Arc<crate::CardDb>) -> Result<Game, String> {
        Game::replay(db, self.cfg.clone(), self.decks.clone(), self.seed, &self.log)
    }

    /// The record with the last `n` decisions removed (undo).
    pub fn undo(&self, n: usize) -> GameRecord {
        let mut r = self.clone();
        let keep = r.log.len().saturating_sub(n);
        r.log.truncate(keep);
        r
    }
}

#[allow(dead_code)]
fn _types(_: Zone) {}
