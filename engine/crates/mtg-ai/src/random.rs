use crate::agent::Agent;
use mtg_engine::*;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

/// Plays random legal moves. Used for fuzzing and as the baseline opponent.
pub struct RandomAgent {
    rng: ChaCha8Rng,
    /// Probability of passing when something could be done.
    pub pass_chance: f64,
}

impl RandomAgent {
    pub fn new(seed: u64) -> Self {
        RandomAgent { rng: ChaCha8Rng::seed_from_u64(seed), pass_chance: 0.25 }
    }
}

impl Agent for RandomAgent {
    fn decide(&mut self, game: &Game, d: &Decision) -> Choice {
        match &d.kind {
            DecisionKind::Mulligan { .. } => Choice::Keep { keep: true },
            DecisionKind::Priority { actions, .. } => {
                if actions.is_empty() {
                    return Choice::Priority { action: None };
                }
                // always play lands
                if let Some(i) = actions.iter().position(|a| matches!(a, Action::PlayLand { .. })) {
                    return Choice::Priority { action: Some(i) };
                }
                if self.rng.gen_bool(self.pass_chance) {
                    return Choice::Priority { action: None };
                }
                Choice::Priority { action: Some(self.rng.gen_range(0..actions.len())) }
            }
            DecisionKind::Pick { options, min, max, .. } => {
                let max = (*max as usize).min(options.len());
                let min = (*min as usize).min(max);
                let k = if max == min { min } else { self.rng.gen_range(min..=max) };
                let mut idx: Vec<usize> = (0..options.len()).collect();
                idx.shuffle(&mut self.rng);
                idx.truncate(k);
                Choice::Pick { indices: idx }
            }
            DecisionKind::Number { min, max, .. } => Choice::Number { n: self.rng.gen_range(*min..=(*max).min(*min + 5)) },
            DecisionKind::YesNo { .. } => Choice::YesNo { yes: self.rng.gen_bool(0.5) },
            DecisionKind::Attackers { options, .. } => {
                let mut attacks = vec![];
                for o in options {
                    if self.rng.gen_bool(0.5) {
                        let mut targets: Vec<Obj> = o.players.iter().map(|p| Obj::player(*p)).collect();
                        targets.extend(o.permanents.iter().map(|c| Obj::card(*c)));
                        if let Some(t) = targets.choose(&mut self.rng) {
                            attacks.push((o.card, *t));
                        }
                    }
                }
                Choice::Attackers { attacks }
            }
            DecisionKind::Blockers { options } => {
                let mut blocks = vec![];
                for o in options {
                    if self.rng.gen_bool(0.5) {
                        if let Some(a) = o.can_block.choose(&mut self.rng) {
                            blocks.push((o.card, *a));
                        }
                    }
                }
                Choice::Blockers { blocks }
            }
            _ => game.default_choice(d),
        }
    }
}
