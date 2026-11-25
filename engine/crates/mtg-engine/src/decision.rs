//! Everything the engine can ask, and every answer it accepts. All agents (human, AI, network, test script) speak this.

use crate::ids::*;
use crate::mana::{ManaPool, ManaType};
use crate::types::*;
use crate::polarity::Polarity;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Decision {
    pub player: PlayerId,
    pub kind: DecisionKind,
}

/// `Action::Cast.alt` value meaning "cast the other face".
pub const FACE_ALT: u8 = 255;
/// `Action::Cast` alt: cast a card with Morph / Disguise face down as a 2/2 for {3}.
pub const FACE_DOWN_ALT: u8 = 254;

/// Something the player can do with priority.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "t")]
#[ts(export)]
pub enum Action {
    PlayLand { card: CardId },
    /// Play the card's other face as a land (modal double-faced cards).
    PlayLandFace { card: CardId },
    Cast { card: CardId, alt: Option<u8> },
    Activate { card: CardId, ability: u16 },
    /// Tap a permanent for mana outside of paying a cost (floating mana).
    ActivateMana { card: CardId, ability: u16 },
    TurnFaceUp { card: CardId },
    /// Unlock a locked door of a Room you control by paying its mana cost (sorcery speed).
    UnlockDoor { card: CardId, door: u8 },
    /// Companion special action (CR 702.139): pay {3} to put your companion from your sideboard into your hand (sorcery timing, once).
    Companion { card: CardId },
    /// Planechase special action (CR 901.9): the active player rolls the planar die at sorcery speed, paying {N} for the
    /// N rolls they made before it this turn.
    RollPlanarDie,
}

/// One choice in a `Pick` decision.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "t")]
#[ts(export)]
pub enum Opt {
    Obj { obj: Obj },
    Text { text: String },
    Color { c: Color },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AttackerOpt {
    pub card: CardId,
    /// Defending players that can be attacked.
    pub players: Vec<PlayerId>,
    /// Planeswalkers / battles that can be attacked.
    pub permanents: Vec<CardId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct BlockerOpt {
    pub card: CardId,
    pub can_block: Vec<CardId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ManaSourceOpt {
    pub card: CardId,
    pub ability: u16,
    /// Every mana type this activation could produce (for display).
    pub produces: Vec<ManaType>,
    /// The alternatives for what one activation yields (a choice of color gives several): each is a multiset of mana.
    pub outputs: Vec<Vec<(ManaType, u8)>>,
    /// Mana the ability itself costs to activate (filter lands: "{1}, {T}: Add {W}{U}"); empty for most.
    pub cost: String,
    /// Mana from this source can only pay for spells matching the filter.
    #[serde(skip)]
    #[ts(skip)]
    pub restrict: Option<crate::dsl::ManaRestrict>,
    /// Produced by a snow permanent (can pay {S}).
    #[serde(skip)]
    #[ts(skip)]
    pub snow: bool,
}

/// Why a `Pick` is being asked (so agents and UIs can treat target choices, discards and searches differently).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "t")]
#[ts(export)]
pub enum PickPurpose {
    /// `optional`: the targets of a "you may" triggered ability; a `Cancel` answer means "don't use it" and the trigger is
    /// left off the stack (the question offers that instead of backing out of a cast).
    Targets {
        polarity: Polarity,
        #[serde(default)]
        optional: bool,
    },
    Mode,
    /// Like `Mode`, but the same option may be picked more than once (the answer may repeat indices).
    ModeRepeat,
    Discard,
    Sacrifice,
    Search,
    Scry,
    Surveil,
    Legend,
    /// Choosing what to give up to pay a cost (sacrifice / discard / tap...).
    Cost,
    /// Proliferate: choose any number; the default answer is all of them.
    Proliferate,
    Generic,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "t")]
#[ts(export)]
pub enum DecisionKind {
    /// London mulligan: keep this hand or take another. `peek`: the top cards of the library the player also sees
    /// while deciding (free mulligan + scry 3: the cards of the scry still on top), top first. While `peek` is not
    /// empty the player may also answer `Pick` (indices into `peek`: put those on the bottom, in that order) or
    /// `Order` (a permutation of `peek`, first = new top); the decision is then asked again.
    Mulligan {
        hand: Vec<CardId>,
        mulligans: u8,
        #[serde(default)]
        peek: Vec<CardId>,
    },
    /// Put `n` cards from the hand on the bottom of the library.
    Bottom { hand: Vec<CardId>, n: u8 },
    Priority { actions: Vec<Action>, stack_len: u32 },
    /// Choose between `min` and `max` of the options (modes, targets, discards, searches, colors...).
    Pick {
        prompt: String,
        source: Option<CardId>,
        options: Vec<Opt>,
        min: u8,
        max: u8,
        /// The player may back out of the cast / activation (only while casting).
        cancellable: bool,
        /// The cards are hidden from the opponents (library search, discard from hand...).
        hidden: bool,
        purpose: PickPurpose,
    },
    Number { prompt: String, min: i32, max: i32, cancellable: bool },
    YesNo { prompt: String, source: Option<CardId> },
    PayMana {
        what: String,
        remaining: String,
        pool: ManaPool,
        sources: Vec<ManaSourceOpt>,
        can_pay_life: bool,
    },
    Attackers { options: Vec<AttackerOpt>, must_attack: Vec<CardId> },
    Blockers { options: Vec<BlockerOpt> },
    /// Divide combat damage among blockers / the defender.
    AssignDamage {
        attacker: CardId,
        total: i32,
        /// (blocker, lethal damage needed)
        blockers: Vec<(CardId, i32)>,
        /// Defender that trample (or an unblocked attacker) can hit.
        defender: Option<Obj>,
        trample: bool,
    },
    /// Put these in the order you want them (triggers, scry piles...). `first` is the top / first to resolve.
    Order { prompt: String, items: Vec<Opt> },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "t")]
#[ts(export)]
pub enum PayAnswer {
    /// Tap a source for its mana.
    Tap { card: CardId, ability: u16, produce: Option<ManaType> },
    /// Spend mana already in the pool.
    Pool { mana: ManaType },
    /// Pay 2 life for a Phyrexian symbol.
    Life,
    /// Let the engine finish paying.
    Auto,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "t")]
#[ts(export)]
pub enum Choice {
    Keep { keep: bool },
    Bottom { cards: Vec<CardId> },
    /// Take the i-th action of the priority list; `None` passes priority.
    Priority { action: Option<usize> },
    /// Indices into the options of a `Pick`.
    Pick { indices: Vec<usize> },
    Number { n: i32 },
    YesNo { yes: bool },
    Pay { answer: PayAnswer },
    Attackers { attacks: Vec<(CardId, Obj)> },
    Blockers { blocks: Vec<(CardId, CardId)> },
    Damage { assign: Vec<(Obj, i32)> },
    Order { order: Vec<usize> },
    /// Tap a permanent for mana while holding priority (floating mana; doesn't use the stack or end the decision).
    Mana { card: CardId, ability: u16, produce: Option<ManaType>, #[serde(default)] x: Option<i32>, /// Colours for an "any colour" triggered mana ability that fires from this tap (it cannot stop to ask), in order.
        #[serde(default)] picks: Vec<ManaType> },
    /// Back out of a cast or activation.
    Cancel,
    /// Let the engine decide (used by the auto-pass / default agents).
    Default,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "t")]
#[ts(export)]
pub enum GameResult {
    Winner { player: PlayerId },
    Draw,
}

#[derive(Clone, Debug, thiserror::Error)]
#[error("illegal choice: {0}")]
pub struct IllegalChoice(pub String);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// A decision is waiting (see `Game::decision`).
    NeedDecision,
    Over,
}
