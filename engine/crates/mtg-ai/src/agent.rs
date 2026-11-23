use mtg_engine::*;

/// Anything that answers the engine's questions.
pub trait Agent: Send {
    fn decide(&mut self, game: &Game, d: &Decision) -> Choice;
}

#[derive(Debug, Clone)]
pub struct PlayOutcome {
    pub result: Option<GameResult>,
    pub decisions: u64,
    pub turns: u32,
}

/// Drive a game to completion with one agent per seat. `check` runs after every decision (invariants in the fuzzer).
pub fn play_game(
    game: &mut Game,
    agents: &mut [Box<dyn Agent>],
    max_decisions: u64,
    mut check: impl FnMut(&Game) -> Result<(), String>,
) -> Result<PlayOutcome, String> {
    let mut n = 0u64;
    loop {
        match game.run() {
            Status::Over => break,
            Status::NeedDecision => {}
        }
        let d = game.decision().cloned().ok_or("run() asked for a decision but none is pending")?;
        let mut choice = agents[d.player as usize].decide(game, &d);
        if game.apply(choice.clone()).is_err() {
            // an agent gave an illegal answer; fall back to a safe default so the game continues
            choice = Choice::Default;
            game.apply(choice).map_err(|e| format!("default choice rejected for {:?}: {e}", d.kind))?;
        }
        check(game)?;
        n += 1;
        if n >= max_decisions {
            return Ok(PlayOutcome { result: None, decisions: n, turns: game.turn });
        }
    }
    Ok(PlayOutcome { result: game.over.clone(), decisions: n, turns: game.turn })
}
