use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A seat at the table (0-based).
pub type PlayerId = u8;

/// Index of a card instance in the game's arena (cards, tokens, copies).
#[derive(Copy, Clone, Eq, PartialEq, Hash, Ord, PartialOrd, Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CardId(pub u32);

/// Index of a card definition in the shared, immutable card database.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Ord, PartialOrd, Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DefId(pub u32);

/// Identity of an object on the stack (a spell shares its card's id space; abilities get their own).
#[derive(Copy, Clone, Eq, PartialEq, Hash, Ord, PartialOrd, Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct StackId(pub u32);

/// Anything a spell or ability can point at.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "kind")]
#[ts(export)]
pub enum Obj {
    Card { id: CardId },
    Player { id: PlayerId },
    Stack { id: StackId },
}

impl Obj {
    pub fn card(id: CardId) -> Obj {
        Obj::Card { id }
    }
    pub fn player(id: PlayerId) -> Obj {
        Obj::Player { id }
    }
}
