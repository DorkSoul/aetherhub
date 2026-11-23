//! Node binding: the rules engine as a native addon. Everything crosses as JSON strings so the TypeScript side
//! stays typed by the ts-rs generated definitions. AI thinking runs on the libuv thread pool, never on the event loop.

use mtg_ai::{GameSetup, Session, SessionRecord};
use mtg_engine::{CardDb, Choice};
use napi::bindgen_prelude::*;
use napi_derive::napi;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex, MutexGuard};

fn err<E: std::fmt::Display>(e: E) -> Error {
    Error::from_reason(e.to_string())
}

/// Run engine code so that a panic becomes a JS error instead of aborting the Node process (napi does not catch
/// unwinds: a panic in a #[napi] method or an async task's `compute` would otherwise take the whole server down).
/// A game whose engine panicked keeps a poisoned lock, so every later call on it fails with an error too.
fn guard<T>(f: impl FnOnce() -> Result<T>) -> Result<T> {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|p| {
        let msg = p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_else(|| "unknown panic".into());
        Err(Error::from_reason(format!("rules engine error: {msg}")))
    })
}

fn lock(m: &Mutex<Session>) -> Result<MutexGuard<'_, Session>> {
    m.lock().map_err(|_| err("rules engine error: this game failed earlier and can't continue"))
}

/// The compiled card database.
#[napi]
pub struct Engine {
    db: Arc<CardDb>,
}

#[napi]
impl Engine {
    /// Load every card under `dir` (one JSON file per card) and compile them.
    #[napi(factory)]
    pub fn load(dir: String) -> Result<Engine> {
        guard(|| {
            let (db, notes) = mtg_cards::build_db_playable(std::path::Path::new(&dir));
            if !notes.is_empty() {
                eprintln!("rules engine: {} card file(s) left out of the playable pool", notes.len());
            }
            Ok(Engine { db })
        })
    }

    #[napi]
    pub fn card_count(&self) -> Result<u32> {
        guard(|| Ok(self.db.defs.iter().filter(|d| d.is_front && !d.token).count() as u32))
    }

    /// Is there a definition for this card name?
    #[napi]
    pub fn has_card(&self, name: String) -> Result<bool> {
        guard(|| Ok(self.db.find(&name).map_or(false, |id| !self.db.def(id).token)))
    }

    /// Names that have a definition (front faces, tokens excluded).
    #[napi]
    pub fn card_names(&self) -> Result<Vec<String>> {
        guard(|| Ok(self.db.defs.iter().filter(|d| d.is_front && !d.token).map(|d| d.name.clone()).collect()))
    }

    /// The engine's token definitions, by name (for "break the rules": create a token a card failed to make).
    #[napi]
    pub fn token_names(&self) -> Result<Vec<String>> {
        guard(|| {
            let mut v: Vec<String> = self.db.defs.iter().filter(|d| d.token && d.is_front).map(|d| d.name.clone()).collect();
            v.sort();
            v.dedup();
            Ok(v)
        })
    }

    /// Start a game from a `GameSetup` JSON document.
    #[napi]
    pub fn new_game(&self, setup_json: String) -> Result<Game> {
        guard(|| {
            let setup: GameSetup = serde_json::from_str(&setup_json).map_err(err)?;
            let s = Session::new(self.db.clone(), setup).map_err(err)?;
            Ok(Game { inner: Arc::new(Mutex::new(s)) })
        })
    }

    /// Rebuild a saved game from a `SessionRecord` JSON document.
    #[napi]
    pub fn restore_game(&self, record_json: String) -> Result<Game> {
        guard(|| {
            let rec: SessionRecord = serde_json::from_str(&record_json).map_err(err)?;
            let s = Session::restore(self.db.clone(), rec).map_err(err)?;
            Ok(Game { inner: Arc::new(Mutex::new(s)) })
        })
    }
}

#[napi]
pub struct Game {
    inner: Arc<Mutex<Session>>,
}

pub struct AdvanceTask {
    inner: Arc<Mutex<Session>>,
}

impl Task for AdvanceTask {
    type Output = String;
    type JsValue = String;

    fn compute(&mut self) -> Result<String> {
        guard(|| {
            let mut s = lock(&self.inner)?;
            let adv = s.advance();
            serde_json::to_string(&adv).map_err(err)
        })
    }

    fn resolve(&mut self, _env: Env, output: String) -> Result<String> {
        Ok(output)
    }
}

#[napi]
impl Game {
    /// Run until a human seat must answer or the game ends. Returns an `Advance` JSON document.
    #[napi(ts_return_type = "Promise<string>")]
    pub fn advance(&self) -> AsyncTask<AdvanceTask> {
        AsyncTask::new(AdvanceTask { inner: self.inner.clone() })
    }

    /// Give a seat's answer (a `Choice` JSON document). Call `advance()` afterwards.
    #[napi]
    pub fn answer(&self, seat: u32, choice_json: String) -> Result<()> {
        guard(|| {
            let c: Choice = serde_json::from_str(&choice_json).map_err(err)?;
            lock(&self.inner)?.answer(seat as u8, c).map_err(err)
        })
    }

    /// The game as a seat sees it (`View` JSON).
    #[napi]
    pub fn view(&self, seat: u32) -> Result<String> {
        guard(|| {
            let v = lock(&self.inner)?.view(seat as u8);
            serde_json::to_string(&v).map_err(err)
        })
    }

    /// `SessionRecord` JSON (setup + every decision) for saving.
    #[napi]
    pub fn record(&self) -> Result<String> {
        guard(|| {
            let r = lock(&self.inner)?.record();
            serde_json::to_string(&r).map_err(err)
        })
    }

    /// Plain-language reason why a click or drag (`kind`: play / attack / block) was refused.
    #[napi]
    pub fn explain(&self, seat: u32, kind: String, card: Option<u32>, other: Option<u32>) -> Result<String> {
        guard(|| {
            Ok(lock(&self.inner)?.explain(seat as u8, &kind, card, other))
        })
    }

    /// Where a seat wants priority (bit masks over steps, Untap=bit 0 ... Cleanup=bit 12).
    #[napi]
    pub fn set_stops(&self, seat: u32, mine: u32, opp: u32, full_control: bool) -> Result<()> {
        guard(|| {
            lock(&self.inner)?.set_stops(seat as u8, mine as u16, opp as u16, full_control);
            Ok(())
        })
    }

    /// Pass priority for a seat until `turn` / step index (Untap = 0 ... Cleanup = 12) is reached.
    #[napi]
    pub fn pass_until(&self, seat: u32, turn: u32, step: u32) -> Result<()> {
        guard(|| {
            use mtg_engine::types::Step::*;
            let steps = [Untap, Upkeep, Draw, Main1, BeginCombat, DeclareAttackers, DeclareBlockers, FirstStrikeDamage, CombatDamage, EndCombat, Main2, End, Cleanup];
            let st = *steps.get(step as usize).ok_or_else(|| err("bad step"))?;
            lock(&self.inner)?.pass_until(seat as u8, turn, st);
            Ok(())
        })
    }

    /// Pass priority for a seat until the stack item (`StackView.id`) is on top of the stack, or has left it.
    #[napi]
    pub fn pass_until_stack(&self, seat: u32, item: u32) -> Result<()> {
        guard(|| {
            lock(&self.inner)?.pass_until_stack(seat as u8, mtg_engine::StackId(item));
            Ok(())
        })
    }

    /// Pass priority for a seat until `player`'s next turn starts.
    #[napi]
    pub fn pass_until_player(&self, seat: u32, player: u32) -> Result<()> {
        guard(|| {
            let mut g = lock(&self.inner)?;
            if player as usize >= g.game.players.len() {
                return Err(err("bad player"));
            }
            g.pass_until_player(seat as u8, player as u8);
            Ok(())
        })
    }

    /// Tap lands by hand for every payment (`on`), or let the engine pay.
    #[napi]
    pub fn set_manual_pay(&self, seat: u32, on: bool) -> Result<()> {
        guard(|| {
            lock(&self.inner)?.set_manual_pay(seat as u8, on);
            Ok(())
        })
    }

    /// Tap lands by hand for the next payment only.
    #[napi]
    pub fn manual_once(&self, seat: u32) -> Result<()> {
        guard(|| {
            lock(&self.inner)?.manual_once(seat as u8);
            Ok(())
        })
    }

    /// `BlockPreview[]` JSON for proposed blocks given as `[[blocker, attacker], ...]` JSON.
    #[napi]
    pub fn preview_blocks(&self, blocks_json: String) -> Result<String> {
        guard(|| {
            let blocks: Vec<(u32, u32)> = serde_json::from_str(&blocks_json).map_err(err)?;
            let blocks: Vec<(mtg_engine::CardId, mtg_engine::CardId)> = blocks.into_iter().map(|(b, a)| (mtg_engine::CardId(b), mtg_engine::CardId(a))).collect();
            let p = lock(&self.inner)?.preview_blocks(&blocks);
            serde_json::to_string(&p).map_err(err)
        })
    }

    /// "Break the rules": apply a direct change (`EditOp` JSON).
    #[napi]
    pub fn edit(&self, seat: u32, op_json: String) -> Result<()> {
        guard(|| {
            let op: mtg_engine::EditOp = serde_json::from_str(&op_json).map_err(err)?;
            lock(&self.inner)?.edit(seat as u8, op).map_err(err)
        })
    }

    /// Finish a rule break: the open priority question is asked again.
    #[napi]
    pub fn end_break(&self, seat: u32) -> Result<()> {
        guard(|| {
            lock(&self.inner)?.end_break(seat as u8);
            Ok(())
        })
    }

    /// Stop passing priority automatically for a seat.
    #[napi]
    pub fn cancel_pass(&self, seat: u32) -> Result<()> {
        guard(|| {
            lock(&self.inner)?.cancel_pass(seat as u8);
            Ok(())
        })
    }

    /// Whether a seat is passing priority automatically: `PassUntil` JSON, or null.
    #[napi]
    pub fn passing(&self, seat: u32) -> Result<String> {
        guard(|| {
            let p = lock(&self.inner)?.passing(seat as u8);
            serde_json::to_string(&p).map_err(err)
        })
    }

    #[napi]
    pub fn concede(&self, seat: u32) -> Result<()> {
        guard(|| {
            lock(&self.inner)?.concede(seat as u8);
            Ok(())
        })
    }

    /// Undo the last `n` answers of human seats.
    #[napi]
    pub fn undo(&self, n: u32) -> Result<()> {
        guard(|| {
            lock(&self.inner)?.undo(n as usize).map_err(err)
        })
    }

    #[napi]
    pub fn turn(&self) -> Result<u32> {
        guard(|| {
            Ok(lock(&self.inner)?.game.turn)
        })
    }
}
