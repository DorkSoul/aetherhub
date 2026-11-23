//! Records which `(card, ability)` pairs actually resolved, so a test run can list the abilities no scenario ever exercises
//! (`cards coverage`). Off unless `ON` is set; the cost when off is one relaxed atomic load per resolving stack item.
use crate::carddb::CardDb;
use crate::dsl::Ability;
use crate::state::{ItemKind, StackItem};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

pub static ON: AtomicBool = AtomicBool::new(false);
static HITS: Mutex<Vec<(u32, u16)>> = Mutex::new(Vec::new());

/// Note the ability a resolving (not fizzled) stack item came from.
pub fn note(item: &StackItem, db: &CardDb) {
    let d = db.def(item.def);
    let idx = match &item.kind {
        ItemKind::Spell { alt, .. } => {
            let alt_with_effects = alt.and_then(|ai| d.abilities.iter().enumerate().filter(|(_, a)| matches!(a, Ability::AlternativeCost { effects: Some(_), .. })).nth(ai as usize).map(|(i, _)| i));
            alt_with_effects.or_else(|| d.abilities.iter().position(|a| matches!(a, Ability::Spell { .. } | Ability::Modal { .. })))
        }
        ItemKind::Ability { ability, .. } | ItemKind::Triggered { ability, .. } if *ability < crate::cast::GRANTED_BASE => Some(*ability as usize),
        _ => None,
    };
    if let Some(i) = idx {
        if let Ok(mut h) = HITS.lock() {
            h.push((item.def.0, i as u16));
        }
    }
}

/// Note an ability that ran without using the stack (a triggered mana ability).
pub fn record(def: crate::ids::DefId, ability: u16) {
    if let Ok(mut h) = HITS.lock() {
        h.push((def.0, ability));
    }
}

/// The hits since the last call.
pub fn take() -> HashSet<(u32, u16)> {
    HITS.lock().map(|mut h| std::mem::take(&mut *h).into_iter().collect()).unwrap_or_default()
}
