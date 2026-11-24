//! Scenario tests: a small, engine-agnostic JSON format describing a board, some plays and what must be true afterwards.
//! Cards are verified by scenarios.
//!
//! ```json
//! { "name": "Bolt kills Bears",
//!   "players": [ { "hand": ["Lightning Bolt"], "battlefield": ["Mountain"] }, { "battlefield": ["Grizzly Bears"] } ],
//!   "steps": [ { "cast": "Lightning Bolt", "targets": ["Grizzly Bears"] }, { "resolve": true } ],
//!   "expect": { "players": [ { "graveyard": ["Lightning Bolt"] }, { "graveyard": ["Grizzly Bears"], "life": 20 } ] } }
//! ```

use mtg_engine::types::*;
use mtg_engine::*;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum CardSpec {
    Name(String),
    Detailed {
        name: String,
        #[serde(default)]
        tapped: bool,
        #[serde(default)]
        counters: BTreeMap<String, u16>,
        #[serde(default)]
        sick: bool,
        /// Enters the battlefield face down (a 2/2 with no abilities).
        #[serde(default)]
        face_down: bool,
        /// An exiled card that was exiled before the game for the card with this name ("exiled with cards named ...").
        #[serde(default)]
        pregame_for: Option<String>,
    },
}

impl CardSpec {
    fn name(&self) -> &str {
        match self {
            CardSpec::Name(n) => n,
            CardSpec::Detailed { name, .. } => name,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct PlayerSetup {
    #[serde(default)]
    pub life: Option<i32>,
    /// Starting energy counters.
    #[serde(default)]
    pub energy: Option<i32>,
    /// The colour this player chose before the game (a "choose a color before the game begins" commander): "W", "U", "B", "R" or "G".
    #[serde(default)]
    pub pregame_color: Option<String>,
    /// Starting poison counters.
    #[serde(default)]
    pub poison: Option<i32>,
    /// Starting rad counters.
    #[serde(default)]
    pub rad: Option<i32>,
    #[serde(default)]
    pub hand: Vec<CardSpec>,
    #[serde(default)]
    pub battlefield: Vec<CardSpec>,
    #[serde(default)]
    pub graveyard: Vec<CardSpec>,
    #[serde(default)]
    pub exile: Vec<CardSpec>,
    /// Top of the library first.
    #[serde(default)]
    pub library: Vec<CardSpec>,
    /// No filler is put under the listed library cards: the library holds exactly these ("fewer than four cards in library" rulings).
    #[serde(default)]
    pub exact_library: bool,
    #[serde(default)]
    pub command: Vec<CardSpec>,
    /// Cards outside the game (the sideboard).
    #[serde(default)]
    pub sideboard: Vec<CardSpec>,
    /// The name of the sideboard card that is this player's chosen companion.
    #[serde(default)]
    pub companion: Option<String>,
    /// Archenemy: this player's scheme deck (first listed = top); a player with schemes is the archenemy (teams are set).
    #[serde(default)]
    pub schemes: Vec<String>,
}

/// Anything a step can name: a player ("p0", "p1"), a card ("Grizzly Bears", "p1:Grizzly Bears"), or a spell on the stack.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Ref {
    Name(String),
    Stack { stack: String },
    /// The named spell itself (not a trigger or copy ability that carries its card name).
    Spell { spell: String },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Answer {
    Text(String),
    Index(usize),
    Number(i64),
    Bool(bool),
    List(Vec<Answer>),
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Common {
    /// Seat that acts (default 0).
    #[serde(default)]
    pub player: Option<u8>,
    #[serde(default)]
    pub targets: Vec<Ref>,
    /// Answers to the questions the step triggers, in order (modes, searches, discards, yes/no, X ...).
    #[serde(default)]
    pub answers: Vec<Answer>,
    /// Use the engine's default for any question not covered by `answers`.
    #[serde(default)]
    pub auto: bool,
    /// Targets already picked for a target question this step (so a name given twice can't be satisfied by one object).
    #[serde(skip)]
    pub used_targets: Vec<Obj>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attacker {
    pub card: String,
    /// "p1" or a planeswalker name.
    pub at: String,
    /// Attackers with the same band number attack as a band (banding).
    #[serde(default)]
    pub band: Option<u8>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockPair {
    pub blocker: String,
    pub attacker: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step_ {
    #[serde(flatten)]
    pub common: Common,
    #[serde(default)]
    pub cast: Option<String>,
    /// Cast using the n-th alternative cost.
    #[serde(default)]
    pub alt: Option<u8>,
    /// Cast / play the card's other face (modal double-faced and split cards).
    #[serde(default)]
    pub face: bool,
    #[serde(default)]
    pub play_land: Option<String>,
    /// Activate an ability of a permanent: name, with `ability` = index among its non-mana activated abilities (default 0).
    #[serde(default)]
    pub activate: Option<String>,
    #[serde(default)]
    pub ability: Option<usize>,
    /// Unlock a door (`door`: 1 or 2) of a Room you control.
    #[serde(default)]
    pub unlock: Option<String>,
    #[serde(default)]
    pub door: Option<u8>,
    /// Take the companion special action: pay {3} to put the named companion from the sideboard into hand.
    #[serde(default)]
    pub companion: Option<String>,
    /// Planechase: roll the planar die (the special action), forcing the face: "chaos", "planeswalk" or "blank".
    #[serde(default)]
    pub planar_die: Option<String>,
    /// Pass priority until the stack is empty.
    #[serde(default)]
    pub resolve: Option<bool>,
    /// Pass priority once.
    #[serde(default)]
    pub pass: Option<bool>,
    /// Pass until this step of the current (or next) turn, stopping at the first priority there.
    #[serde(default)]
    pub advance_to: Option<Step>,
    /// With `advance_to`: stop at the first priority of that step even when triggers are on the stack (so a scenario can respond to a beginning-of-step trigger).
    #[serde(default)]
    pub hold: Option<bool>,
    #[serde(default)]
    pub attack: Option<Vec<Attacker>>,
    #[serde(default)]
    pub block: Option<Vec<BlockPair>>,
    /// Tap a permanent for mana (floating).
    #[serde(default)]
    pub tap_for_mana: Option<String>,
    /// Color to produce when tapping for mana of a chosen color (W U B R G); default: the first the source allows.
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub concede: Option<bool>,
    /// The action (cast / play_land / activate) must NOT be legal right now; nothing is done.
    #[serde(default)]
    pub illegal: Option<bool>,
    #[serde(default)]
    pub set_life: Option<i32>,
    /// Set the acting player's speed (0..=4) before continuing.
    #[serde(default)]
    pub set_speed: Option<u8>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct PlayerExpect {
    pub life: Option<i32>,
    pub hand: Option<Vec<String>>,
    pub battlefield: Option<Vec<String>>,
    pub graveyard: Option<Vec<String>>,
    pub exile: Option<Vec<String>>,
    pub command: Option<Vec<String>>,
    pub sideboard: Option<Vec<String>>,
    pub hand_count: Option<usize>,
    pub library_count: Option<usize>,
    /// The top cards of the library, topmost first (only as many as listed are compared).
    pub library_top: Option<Vec<String>>,
    pub poison: Option<i32>,
    pub energy: Option<i32>,
    pub rad: Option<i32>,
    /// Contraptions the player has assembled.
    pub contraptions: Option<u32>,
    /// The player's speed (0 to 4).
    pub speed: Option<u8>,
    /// Battlefield contains at least these.
    #[serde(default)]
    pub battlefield_has: Vec<String>,
    #[serde(default)]
    pub battlefield_lacks: Vec<String>,
    #[serde(default)]
    pub graveyard_has: Vec<String>,
    #[serde(default)]
    pub pool: Option<BTreeMap<String, u16>>,
    /// Restricted mana still floating (type letter -> units), added by spells / triggers.
    #[serde(default)]
    pub restricted_pool: Option<BTreeMap<String, u16>>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Expect {
    #[serde(default)]
    pub players: Vec<PlayerExpect>,
    pub stack_empty: Option<bool>,
    pub step: Option<Step>,
    pub turn: Option<u32>,
    /// Seat that won; use -1 for "the game is not over".
    pub winner: Option<i32>,
    /// card name -> counter kind -> count
    #[serde(default)]
    pub counters: BTreeMap<String, BTreeMap<String, i32>>,
    /// card name -> [power, toughness] (current)
    #[serde(default)]
    pub pt: BTreeMap<String, [i32; 2]>,
    /// card name -> its current colors as letters ("W","U","B","R","G"); an empty list means colorless
    #[serde(default)]
    pub colors: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub tapped: BTreeMap<String, bool>,
    /// card name -> keywords it must currently have
    #[serde(default)]
    pub keywords: BTreeMap<String, Vec<Keyword>>,
    /// card name -> name of the permanent it is attached to
    #[serde(default)]
    pub attached: BTreeMap<String, String>,
    /// battle name -> seat of its protector
    #[serde(default)]
    pub protector: BTreeMap<String, u8>,
    /// card name -> subtypes it must currently have ("Wall", "Avatar" ...)
    #[serde(default)]
    pub subtypes: BTreeMap<String, Vec<String>>,
    /// card name -> subtypes it must currently NOT have
    #[serde(default)]
    pub not_subtypes: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub name: String,
    #[serde(default)]
    pub commander: bool,
    #[serde(default)]
    pub players: Vec<PlayerSetup>,
    /// Step the game starts in (default Main1, player 0's turn).
    #[serde(default)]
    pub start: Option<Step>,
    #[serde(default)]
    pub seed: Option<u64>,
    /// Forced results for the next die rolls, in order (then real random rolls): `"rolls": [20, 3]`.
    #[serde(default)]
    pub rolls: Vec<i32>,
    /// Planechase: the face-up plane(s) (in the active player's command zone) and the planar deck (first listed = top).
    #[serde(default)]
    pub planes: Vec<CardSpec>,
    #[serde(default)]
    pub planar_deck: Vec<String>,
    pub steps: Vec<Step_>,
    #[serde(default)]
    pub expect: Expect,
}

pub fn load_scenarios(text: &str) -> Result<Vec<Scenario>, String> {
    match serde_json::from_str::<Vec<Scenario>>(text) {
        Ok(v) => Ok(v),
        Err(_) => serde_json::from_str::<Scenario>(text).map(|s| vec![s]).map_err(|e| e.to_string()),
    }
}

fn counter_kind(s: &str) -> Option<CounterKind> {
    let lower = s.to_lowercase();
    // "-1/-1" must not collapse into "11" (+1/+1) once the signs are stripped
    if lower.trim_start().starts_with('-') && lower.contains('1') {
        return Some(CounterKind::MinusOneMinusOne);
    }
    let n = lower.replace(['+', '/', '-', ' ', '_'], "");
    Some(match n.as_str() {
        "p1p1" | "11" | "plusoneplusone" | "plus1plus1" => CounterKind::PlusOnePlusOne,
        "m1m1" | "minusoneminusone" | "minus1minus1" => CounterKind::MinusOneMinusOne,
        "loyalty" => CounterKind::Loyalty,
        "defense" => CounterKind::Defense,
        "charge" => CounterKind::Charge,
        "poison" => CounterKind::Poison,
        "energy" => CounterKind::Energy,
        "experience" => CounterKind::Experience,
        "time" => CounterKind::Time,
        "lore" => CounterKind::Lore,
        "other" => CounterKind::Other,
        "acorn" => CounterKind::Acorn,
        "arrow" => CounterKind::Arrow,
        "arrowhead" => CounterKind::Arrowhead,
        "bait" => CounterKind::Bait,
        "blessing" => CounterKind::Blessing,
        "bloodline" => CounterKind::Bloodline,
        "bloodstain" => CounterKind::Bloodstain,
        "book" => CounterKind::Book,
        "burden" => CounterKind::Burden,
        "carrion" => CounterKind::Carrion,
        "cell" => CounterKind::Cell,
        "chorus" => CounterKind::Chorus,
        "component" => CounterKind::Component,
        "conqueror" => CounterKind::Conqueror,
        "contested" => CounterKind::Contested,
        "credit" => CounterKind::Credit,
        "cube" => CounterKind::Cube,
        "currency" => CounterKind::Currency,
        "descent" => CounterKind::Descent,
        "devotion" => CounterKind::Devotion,
        "dread" => CounterKind::Dread,
        "elixir" => CounterKind::Elixir,
        "ember" => CounterKind::Ember,
        "eon" => CounterKind::Eon,
        "feeding" => CounterKind::Feeding,
        "fellowship" => CounterKind::Fellowship,
        "filibuster" => CounterKind::Filibuster,
        "film" => CounterKind::Film,
        "fire" => CounterKind::Fire,
        "flame" => CounterKind::Flame,
        "foreshadow" => CounterKind::Foreshadow,
        "fury" => CounterKind::Fury,
        "gem" => CounterKind::Gem,
        "ghostform" => CounterKind::Ghostform,
        "growth" => CounterKind::Growth,
        "harmony" => CounterKind::Harmony,
        "hatchling" => CounterKind::Hatchling,
        "healing" => CounterKind::Healing,
        "hoofprint" => CounterKind::Hoofprint,
        "hope" => CounterKind::Hope,
        "hourglass" => CounterKind::Hourglass,
        "incarnation" => CounterKind::Incarnation,
        "incubation" => CounterKind::Incubation,
        "ingenuity" => CounterKind::Ingenuity,
        "intervention" => CounterKind::Intervention,
        "invasion" => CounterKind::Invasion,
        "invitation" => CounterKind::Invitation,
        "javelin" => CounterKind::Javelin,
        "knowledge" => CounterKind::Knowledge,
        "landmark" => CounterKind::Landmark,
        "loot" => CounterKind::Loot,
        "mining" => CounterKind::Mining,
        "muster" => CounterKind::Muster,
        "nest" => CounterKind::Nest,
        "night" => CounterKind::Night,
        "ore" => CounterKind::Ore,
        "palliation" => CounterKind::Palliation,
        "petal" => CounterKind::Petal,
        "phyresis" => CounterKind::Phyresis,
        "plague" => CounterKind::Plague,
        "plot" => CounterKind::Plot,
        "possession" => CounterKind::Possession,
        "pressure" => CounterKind::Pressure,
        "eruption" => CounterKind::Eruption,
        "pupa" => CounterKind::Pupa,
        "rally" => CounterKind::Rally,
        "rev" => CounterKind::Rev,
        "ribbon" => CounterKind::Ribbon,
        "ritual" => CounterKind::Ritual,
        "scream" => CounterKind::Scream,
        "shell" => CounterKind::Shell,
        "silver" => CounterKind::Silver,
        "skewer" => CounterKind::Skewer,
        "slumber" => CounterKind::Slumber,
        "soot" => CounterKind::Soot,
        "story" => CounterKind::Story,
        "supply" => CounterKind::Supply,
        "task" => CounterKind::Task,
        "theft" => CounterKind::Theft,
        "tower" => CounterKind::Tower,
        "training" => CounterKind::Training,
        "treasure" => CounterKind::Treasure,
        "unity" => CounterKind::Unity,
        "unlock" => CounterKind::Unlock,
        "valor" => CounterKind::Valor,
        "vitality" => CounterKind::Vitality,
        "winch" => CounterKind::Winch,
        "scroll" => CounterKind::Scroll,
        "exposure" => CounterKind::Exposure,
        "shield" => CounterKind::Shield,
        "stun" => CounterKind::Stun,
        "flying" => CounterKind::Flying,
        "level" => CounterKind::Level,
        "age" => CounterKind::Age,
        "fade" => CounterKind::Fade,
        "quest" => CounterKind::Quest,
        "verse" => CounterKind::Verse,
        "firststrike" => CounterKind::FirstStrike,
        "doublestrike" => CounterKind::DoubleStrike,
        "vigilance" => CounterKind::Vigilance,
        "deathtouch" => CounterKind::Deathtouch,
        "lifelink" => CounterKind::Lifelink,
        "reach" => CounterKind::Reach,
        "trample" => CounterKind::Trample,
        "menace" => CounterKind::Menace,
        "hexproof" => CounterKind::Hexproof,
        "indestructible" => CounterKind::Indestructible,
        "haste" => CounterKind::Haste,
        "finality" => CounterKind::Finality,
        "ice" => CounterKind::Ice,
        "corpse" => CounterKind::Corpse,
        "isolation" => CounterKind::Isolation,
        "crystal" => CounterKind::Crystal,
        "study" => CounterKind::Study,
        "hit" => CounterKind::Hit,
        "echo" => CounterKind::Echo,
        "oil" => CounterKind::Oil,
        "rad" => CounterKind::Rad,
        "storage" => CounterKind::Storage,
        "depletion" => CounterKind::Depletion,
        "ki" => CounterKind::Ki,
        "bounty" => CounterKind::Bounty,
        "flood" => CounterKind::Flood,
        "spore" => CounterKind::Spore,
        "soul" => CounterKind::Soul,
        "infection" => CounterKind::Infection,
        "page" => CounterKind::Page,
        "blood" => CounterKind::Blood,
        "wind" => CounterKind::Wind,
        "plan" => CounterKind::Plan,
        "memory" => CounterKind::Memory,
        "doom" => CounterKind::Doom,
        "net" => CounterKind::Net,
        "coin" => CounterKind::Coin,
        "dream" => CounterKind::Dream,
        "egg" => CounterKind::Egg,
        "wage" => CounterKind::Wage,
        "fuse" => CounterKind::Fuse,
        "brick" => CounterKind::Brick,
        "pin" => CounterKind::Pin,
        "rope" => CounterKind::Rope,
        "fetch" => CounterKind::Fetch,
        "mine" => CounterKind::Mine,
        "aim" => CounterKind::Aim,
        "corruption" => CounterKind::Corruption,
        "vortex" => CounterKind::Vortex,
        "manifestation" => CounterKind::Manifestation,
        "judgment" => CounterKind::Judgment,
        "collection" => CounterKind::Collection,
        "ingredient" => CounterKind::Ingredient,
        "p1p0" | "10" | "plusoneplusnothing" => CounterKind::PlusOnePlusZero,
        "p0p1" | "01" => CounterKind::PlusZeroPlusOne,
        "p2p2" | "22" => CounterKind::PlusTwoPlusTwo,
        "p0p2" | "02" => CounterKind::PlusZeroPlusTwo,
        "m0m1" => CounterKind::MinusZeroMinusOne,
        "m0m2" => CounterKind::MinusZeroMinusTwo,
        "m1m0" => CounterKind::MinusOneMinusZero,
        "m2m2" => CounterKind::MinusTwoMinusTwo,
        "fate" => CounterKind::Fate,
        "takeover" => CounterKind::Takeover,
        "hone" => CounterKind::Hone,
        "promotion" => CounterKind::Promotion,
        "influence" => CounterKind::Influence,
        "stash" => CounterKind::Stash,
        "refine" => CounterKind::Refine,
        "hunger" => CounterKind::Hunger,
        "glyph" => CounterKind::Glyph,
        "pain" => CounterKind::Pain,
        "petrification" => CounterKind::Petrification,
        "spite" => CounterKind::Spite,
        "awakening" => CounterKind::Awakening,
        "peace" => CounterKind::Peace,
        "saurian" => CounterKind::Saurian,
        "gold" => CounterKind::Gold,
        "sleep" => CounterKind::Sleep,
        "blight" => CounterKind::Blight,
        "phylactery" => CounterKind::Phylactery,
        "enlightened" => CounterKind::Enlightened,
        "void" => CounterKind::Void,
        "vow" => CounterKind::Vow,
        "tide" => CounterKind::Tide,
        "hatching" => CounterKind::Hatching,
        "duty" => CounterKind::Duty,
        "luck" => CounterKind::Luck,
        "polyp" => CounterKind::Polyp,
        "intel" => CounterKind::Intel,
        "aegis" => CounterKind::Aegis,
        "impostor" => CounterKind::Impostor,
        "suspect" => CounterKind::Suspect,
        "divinity" => CounterKind::Divinity,
        "matrix" => CounterKind::Matrix,
        "strife" => CounterKind::Strife,
        "rejection" => CounterKind::Rejection,
        "despair" => CounterKind::Despair,
        "hour" => CounterKind::Hour,
        "music" => CounterKind::Music,
        "prey" => CounterKind::Prey,
        "velocity" => CounterKind::Velocity,
        "croak" => CounterKind::Croak,
        "necrodermis" => CounterKind::Necrodermis,
        "bore" => CounterKind::Bore,
        "blaze" => CounterKind::Blaze,
        "shadow" => CounterKind::ShadowCounter,
        "fungus" => CounterKind::Fungus,
        "m2m1" => CounterKind::MinusTwoMinusOne,
        "p1p2" => CounterKind::PlusOnePlusTwo,
        "wish" => CounterKind::Wish,
        "brain" => CounterKind::Brain,
        "marker" => CounterKind::Marker,
        "feather" => CounterKind::Feather,
        "sleight" => CounterKind::Sleight,
        "rust" => CounterKind::Rust,
        "reprieve" => CounterKind::Reprieve,
        "revival" => CounterKind::Revival,
        "voyage" => CounterKind::Voyage,
        "mire" => CounterKind::Mire,
        "magnet" => CounterKind::Magnet,
        "exalted" => CounterKind::Exalted,
        "everything" => CounterKind::Everything,
        "cage" => CounterKind::Cage,
        "omen" => CounterKind::Omen,
        "shred" => CounterKind::Shred,
        "bribery" => CounterKind::Bribery,
        "decayed" => CounterKind::Decayed,
        "slime" => CounterKind::Slime,
        "trap" => CounterKind::Trap,
        "eyeball" => CounterKind::Eyeball,
        "delay" => CounterKind::Delay,
        "paralyzation" => CounterKind::Paralyzation,
        "death" => CounterKind::Death,
        "point" => CounterKind::Point,
        "p2p0" => CounterKind::PlusTwoPlusZero,
        _ => return None,
    })
}

struct Runner {
    g: Game,
    trace: Vec<String>,
    /// Set while checking an `illegal` step with explicit targets: a named target that no target slot used makes the action illegal.
    strict_targets: bool,
    /// Set while checking an `illegal` attack step: reaching the declaration must not run into a later turn (when no
    /// creature can attack, the engine skips the declare attackers decision, and the next one may be turns later).
    same_turn_attack: bool,
    /// The attack or block declaration submitted while checking an `illegal` step (with the bands it asked for).
    declared: Option<(Choice, Vec<Vec<CardId>>)>,
}

type R<T> = Result<T, String>;

impl Runner {
    fn card_matches(&self, id: CardId, owner: Option<u8>, name: &str) -> bool {
        let c = self.g.card(id);
        self.g.card_name(id) == name && owner.map_or(true, |o| c.owner == o || c.cur.controller == o)
    }

    fn find(&self, spec: &str, zones: &[Zone]) -> R<CardId> {
        let (owner, name) = match spec.split_once(':') {
            Some((p, n)) if p.starts_with('p') && p[1..].parse::<u8>().is_ok() => (Some(p[1..].parse::<u8>().unwrap()), n),
            _ => (None, spec),
        };
        for z in zones {
            if let Some(c) = self.g.cards.iter().find(|c| c.zone == *z && self.card_matches(c.id, owner, name)) {
                return Ok(c.id);
            }
        }
        Err(format!("no card '{spec}' in {zones:?}"))
    }

    fn any_zone() -> [Zone; 7] {
        [Zone::Battlefield, Zone::Stack, Zone::Hand, Zone::Graveyard, Zone::Exile, Zone::Command, Zone::Library]
    }

    fn player_ref(s: &str) -> Option<u8> {
        s.strip_prefix('p').and_then(|n| n.parse().ok())
    }

    fn resolve_ref(&self, r: &Ref) -> R<Obj> {
        match r {
            Ref::Spell { spell } => self
                .g
                .stack
                .iter()
                .rev()
                .find(|s| self.g.db.def(s.def).name == *spell && matches!(s.kind, mtg_engine::ItemKind::Spell { .. }))
                .map(|s| Obj::Stack { id: s.id })
                .ok_or_else(|| format!("no '{spell}' spell on the stack")),
            Ref::Stack { stack } => self
                .g
                .stack
                .iter()
                .rev()
                .find(|s| self.g.db.def(s.def).name == *stack)
                .map(|s| Obj::Stack { id: s.id })
                .ok_or_else(|| format!("no '{stack}' on the stack")),
            Ref::Name(n) => {
                if let Some(p) = Self::player_ref(n) {
                    if (p as usize) < self.g.players.len() {
                        return Ok(Obj::player(p));
                    }
                }
                self.find(n, &Self::any_zone()).map(Obj::card)
            }
        }
    }

    fn label_of(&self, o: &Opt) -> String {
        match o {
            Opt::Text { text } => text.clone(),
            Opt::Color { c } => format!("{c:?}"),
            Opt::Obj { obj } => match obj {
                Obj::Player { id } => format!("p{id}"),
                Obj::Card { id } => format!("{} (p{}, {:?})", self.g.card_name(*id), self.g.card(*id).owner, self.g.card(*id).zone),
                Obj::Stack { id } => self.g.stack.iter().find(|s| s.id == *id).map_or("stack item".into(), |s| format!("{} (stack)", self.g.db.def(s.def).name)),
            },
        }
    }

    fn option_index(&self, options: &[Opt], want: &Answer) -> R<Vec<usize>> {
        self.option_index_used(options, want, &[])
    }

    /// `taken`: options already picked by earlier entries of the same list (so "Forest","Forest" picks two different Forests).
    fn option_index_used(&self, options: &[Opt], want: &Answer, taken: &[usize]) -> R<Vec<usize>> {
        match want {
            Answer::Index(i) => Ok(vec![*i]),
            Answer::List(l) => {
                let mut v: Vec<usize> = vec![];
                for a in l {
                    let got = self.option_index_used(options, a, &v)?;
                    v.extend(got);
                }
                Ok(v)
            }
            Answer::Text(t) => {
                // match by player ref, card name (optionally "pN:Name") or text
                let used: Vec<usize> = taken.to_vec();
                // an exact text option wins over a substring match ("Soldier" vs "Shi'ar Soldier")
                if let Some(i) = options.iter().enumerate().position(|(i, o)| !used.contains(&i) && matches!(o, Opt::Text { text } if text == t)) {
                    return Ok(vec![i]);
                }
                for (i, o) in options.iter().enumerate() {
                    if used.contains(&i) {
                        continue;
                    }
                    let hit = match o {
                        Opt::Text { text } => text == t || text.contains(t.as_str()),
                        Opt::Color { c } => format!("{c:?}") == *t,
                        Opt::Obj { obj } => match obj {
                            Obj::Player { id } => Self::player_ref(t) == Some(*id),
                            Obj::Card { id } => {
                                let (owner, name) = match t.split_once(':') {
                                    Some((p, n)) if Self::player_ref(p).is_some() => (Self::player_ref(p), n),
                                    _ => (None, t.as_str()),
                                };
                                self.card_matches(*id, owner, name)
                            }
                            Obj::Stack { id } => self.g.stack.iter().any(|s| s.id == *id && self.g.db.def(s.def).name == *t),
                        },
                    };
                    if hit {
                        return Ok(vec![i]);
                    }
                }
                Err(format!("answer '{t}' matches none of: {}", options.iter().map(|o| self.label_of(o)).collect::<Vec<_>>().join("; ")))
            }
            other => Err(format!("answer {other:?} doesn't fit a choice between cards")),
        }
    }

    /// Respond to a non-priority decision using the step's targets / answers.
    fn answer_decision(&mut self, d: &Decision, c: &mut Common) -> R<()> {
        let choice = match &d.kind {
            DecisionKind::Pick { options, min, max, purpose, .. } => {
                let is_target = matches!(purpose, PickPurpose::Targets { .. });
                let picks: Vec<usize> = if is_target && !c.targets.is_empty() {
                    let mut out = vec![];
                    let take = (*max as usize).max(1);
                    for _ in 0..take.min(c.targets.len()) {
                        let t = c.targets.remove(0);
                        let obj = self.resolve_ref(&t)?;
                        let idx = options
                            .iter()
                            .position(|o| matches!(o, Opt::Obj { obj: x } if *x == obj))
                            .ok_or_else(|| format!("target {t:?} is not a legal choice; options: {}", options.iter().map(|o| self.label_of(o)).collect::<Vec<_>>().join("; ")))?;
                        // naming the same card twice ("Grizzly Bears", "Grizzly Bears") picks two different copies
                        let idx = if out.contains(&idx) {
                            let label = self.label_of(&options[idx]);
                            options.iter().enumerate().position(|(j, o)| !out.contains(&j) && self.label_of(o) == label).unwrap_or(idx)
                        } else {
                            idx
                        };
                        if let Opt::Obj { obj } = &options[idx] {
                            c.used_targets.push(*obj);
                        }
                        out.push(idx);
                    }
                    out
                } else if !c.answers.is_empty() {
                    let a = c.answers.remove(0);
                    self.option_index(options, &a)?
                } else if c.auto || (*min as usize) == 0 {
                    return self.apply(self.g.default_choice(d));
                } else {
                    return Err(format!("unexpected question: {} (options: {})", d_prompt(d), options.iter().map(|o| self.label_of(o)).collect::<Vec<_>>().join("; ")));
                };
                Choice::Pick { indices: picks }
            }
            // the commander's way back to the command zone (CR 903.9): answered by "commander: yes" / "commander: no",
            // else yes, so a scenario needn't mention it
            DecisionKind::YesNo { prompt, .. } if prompt.starts_with("Put your commander ") => match c.answers.first() {
                Some(Answer::Text(t)) if t == "commander: yes" || t == "commander: no" => {
                    let yes = t == "commander: yes";
                    c.answers.remove(0);
                    Choice::YesNo { yes }
                }
                _ => Choice::YesNo { yes: true },
            },
            DecisionKind::YesNo { prompt, .. } => match c.answers.first() {
                Some(Answer::Bool(b)) => {
                    let b = *b;
                    c.answers.remove(0);
                    Choice::YesNo { yes: b }
                }
                Some(Answer::Text(t)) if t == "yes" || t == "no" => {
                    let yes = t == "yes";
                    c.answers.remove(0);
                    Choice::YesNo { yes }
                }
                _ if c.auto => self.g.default_choice(d),
                _ => return Err(format!("unexpected yes/no question: {prompt}")),
            },
            DecisionKind::Number { prompt, min, max, .. } => match c.answers.first() {
                Some(Answer::Number(_)) | Some(Answer::Index(_)) => {
                    let n = match c.answers.first() {
                        Some(Answer::Number(n)) => *n as i32,
                        Some(Answer::Index(i)) => *i as i32,
                        _ => 0,
                    };
                    c.answers.remove(0);
                    if n < *min || n > *max {
                        return Err(format!("{prompt}: {n} not in {min}..={max}"));
                    }
                    Choice::Number { n }
                }
                _ if c.auto => self.g.default_choice(d),
                _ => return Err(format!("unexpected number question: {prompt}")),
            },
            // an explicit source for the payment: answer "tap:<card name>#<ability index>" (the ability index is the card's own ability number, as in `tap_for_mana` abilities)
            DecisionKind::PayMana { sources, .. } if matches!(c.answers.first(), Some(Answer::Text(t)) if t.starts_with("tap:")) => {
                let Some(Answer::Text(t)) = c.answers.first().cloned() else { unreachable!() };
                c.answers.remove(0);
                let spec = t.trim_start_matches("tap:");
                let (name, ab) = spec.rsplit_once('#').ok_or_else(|| format!("bad payment source '{t}' (want tap:<card>#<ability>)"))?;
                let card = self.find(name.trim(), &[Zone::Battlefield])?;
                let ability = ab.trim().parse::<u16>().map_err(|e| e.to_string())?;
                // a source the payment doesn't offer (already tapped...) is refused by the engine: that answer
                // is used up without paying anything and the payment question comes back for the next answer
                if !sources.iter().any(|s| s.card == card && s.ability == ability) {
                    return Ok(());
                }
                Choice::Pay { answer: PayAnswer::Tap { card, ability, produce: None } }
            }
            DecisionKind::PayMana { .. } => Choice::Pay { answer: PayAnswer::Auto },
            // an explicit division: answers like "Grizzly Bears=2,p1=4" (name=damage, comma separated)
            DecisionKind::AssignDamage { .. } if matches!(c.answers.first(), Some(Answer::Text(t)) if t.contains('=')) => {
                let Some(Answer::Text(t)) = c.answers.first().cloned() else { unreachable!() };
                c.answers.remove(0);
                let mut assign = vec![];
                for part in t.split(',') {
                    let (name, n) = part.split_once('=').ok_or_else(|| format!("bad damage assignment '{part}'"))?;
                    let obj = self.resolve_ref(&Ref::Name(name.trim().to_string()))?;
                    assign.push((obj, n.trim().parse::<i32>().map_err(|e| e.to_string())?));
                }
                Choice::Damage { assign }
            }
            DecisionKind::AssignDamage { .. } | DecisionKind::Order { .. } | DecisionKind::Bottom { .. } | DecisionKind::Mulligan { .. } => self.g.default_choice(d),
            DecisionKind::Attackers { .. } => self.g.default_choice(d),
            DecisionKind::Blockers { .. } => Choice::Blockers { blocks: vec![] },
            DecisionKind::Priority { .. } => return Err("internal: priority passed to answer_decision".into()),
        };
        self.apply(choice)
    }

    fn apply(&mut self, c: Choice) -> R<()> {
        if self.same_turn_attack && matches!(c, Choice::Attackers { .. } | Choice::Blockers { .. }) {
            self.declared = Some((c.clone(), self.g.attack_bands.clone()));
        }
        self.g.apply(c).map_err(|e| e.to_string())
    }

    /// Run until a decision is pending.
    fn next(&mut self) -> R<Option<Decision>> {
        match self.g.run() {
            Status::Over => Ok(None),
            Status::NeedDecision => Ok(self.g.decision().cloned()),
        }
    }

    /// Answer decisions until the next priority decision for `player` (answering others via `c`).
    fn until_priority(&mut self, c: &mut Common) -> R<Option<Decision>> {
        for _ in 0..200 {
            let Some(d) = self.next()? else { return Ok(None) };
            if matches!(d.kind, DecisionKind::Priority { .. }) {
                return Ok(Some(d));
            }
            self.answer_decision(&d, c)?;
        }
        Err("too many decisions without reaching priority".into())
    }

    fn priority_for(&mut self, player: u8, c: &mut Common) -> R<Vec<Action>> {
        // players holding priority before `player` pass it along (only while something is on the stack to respond to)
        let mut passed = 0;
        loop {
            match self.until_priority(c)? {
                Some(Decision { player: p, kind: DecisionKind::Priority { actions, .. } }) if p == player => return Ok(actions),
                Some(Decision { player: p, .. }) if !self.g.stack.is_empty() && passed + 1 < self.g.players.len() => {
                    let _ = p;
                    self.apply(Choice::Priority { action: None })?;
                    passed += 1;
                }
                Some(Decision { player: p, .. }) => {
                    return Err(format!("expected priority for player {player} but player {p} holds it (step {:?}, turn {})", self.g.step, self.g.turn))
                }
                None => return Err("the game ended unexpectedly".into()),
            }
        }
    }

    /// An `illegal` attack or block step names a declaration that must not stand as written. The engine may refuse it,
    /// drop attackers or blockers that break a restriction, or add creatures that a requirement forces in (goad, "must
    /// be blocked by all able creatures", banding fixes): the step passes unless combat ends up exactly as declared.
    fn illegal_declaration_verdict(&self) -> R<()> {
        let Some((declared, bands)) = &self.declared else { return Ok(()) };
        let attackers = self.g.combat.as_ref().map_or(&[][..], |c| &c.attackers[..]);
        let sorted = |mut v: Vec<String>| {
            v.sort();
            v
        };
        match declared {
            Choice::Attackers { attacks } => {
                let want = sorted(attacks.iter().map(|(c, d)| format!("{c:?}>{d:?}")).collect());
                let got = sorted(attackers.iter().map(|a| format!("{:?}>{:?}", a.card, a.defender)).collect());
                let want_bands = sorted(bands.iter().filter(|b| b.len() > 1).map(|b| sorted(b.iter().map(|c| format!("{c:?}")).collect()).join(",")).collect());
                let mut groups: std::collections::BTreeMap<u8, Vec<String>> = Default::default();
                for a in attackers.iter().filter(|a| a.band != 0) {
                    groups.entry(a.band).or_default().push(format!("{:?}", a.card));
                }
                let got_bands = sorted(groups.into_values().filter(|b| b.len() > 1).map(|b| sorted(b).join(",")).collect());
                if want == got && want_bands == got_bands {
                    let names: Vec<String> = attacks.iter().map(|(c, _)| self.g.card_name(*c).to_string()).collect();
                    return Err(format!("expected the attack by '{}' to be illegal, but it stands as declared", names.join("', '")));
                }
                Ok(())
            }
            Choice::Blockers { blocks } => {
                let want = sorted(blocks.iter().map(|(b, a)| format!("{b:?}>{a:?}")).collect());
                let got = sorted(attackers.iter().flat_map(|a| a.blockers.iter().map(move |b| format!("{b:?}>{:?}", a.card))).collect());
                if want == got {
                    let names: Vec<String> = blocks.iter().map(|(b, a)| format!("{} → {}", self.g.card_name(*b), self.g.card_name(*a))).collect();
                    return Err(format!("expected the block '{}' to be illegal, but it stands as declared", names.join("', '")));
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn do_step(&mut self, s: &mut Step_) -> R<()> {
        let player = s.common.player.unwrap_or(0);
        if s.illegal == Some(true) && (s.block.is_some() || s.attack.is_some()) {
            // an illegal declaration: the engine must refuse it or change it (see illegal_declaration_verdict)
            s.illegal = None;
            let saved = self.g.clone();
            self.declared = None;
            self.same_turn_attack = true;
            let r = self.do_step(s);
            self.same_turn_attack = false;
            s.illegal = Some(true);
            // `apply` only stores the declaration; the engine checks it (and drops illegal attackers / blockers) when it
            // next runs, so run it before looking at who is in combat.
            let r = r.and_then(|()| self.next().map(|_| ()));
            let verdict = if r.is_err() { Ok(()) } else { self.illegal_declaration_verdict() };
            // the step is only an assertion: go back to the state before the declaration
            self.g = saved;
            return verdict;
        }
        if s.illegal == Some(true) && (!s.common.targets.is_empty() || s.common.answers.iter().any(|a| matches!(a, Answer::Number(_) | Answer::Index(_)))) && (s.cast.is_some() || s.activate.is_some()) {
            // illegal with explicit targets: try the action with exactly those targets; the engine must reject it
            s.illegal = None;
            let saved = self.g.clone();
            let saved_answers = s.common.answers.clone();
            self.strict_targets = true;
            let r = self.do_step(s);
            self.strict_targets = false;
            s.common.answers = saved_answers;
            s.illegal = Some(true);
            self.g = saved;
            return match r {
                Err(_) => Ok(()),
                Ok(()) => Err("expected the action with these targets to be illegal, but it was accepted".to_string()),
            };
        }
        if s.illegal == Some(true) {
            let actions = self.priority_for(player, &mut s.common)?;
            let found = actions.iter().any(|a| match a {
                Action::Cast { card, alt } => s.cast.as_deref() == Some(&*self.g.card_name(*card)) && (*alt == Some(mtg_engine::decision::FACE_ALT)) == s.face && (s.face || s.alt.map_or(*alt != Some(mtg_engine::decision::FACE_DOWN_ALT), |a| *alt == Some(a))),
                Action::PlayLand { card } => !s.face && s.play_land.as_deref() == Some(&*self.g.card_name(*card)),
                Action::PlayLandFace { card } => s.face && s.play_land.as_deref() == Some(&*self.g.card_name(*card)),
                Action::Companion { card } => s.companion.as_deref() == Some(&*self.g.card_name(*card)),
                Action::RollPlanarDie => s.planar_die.is_some(),
                Action::Activate { .. } => s.activate.as_deref().map_or(false, |n| self.find_activation(std::slice::from_ref(a), n, s.ability.unwrap_or(0)).is_some()),
                _ => false,
            });
            if found {
                return Err(format!("expected the action to be illegal, but it is legal: {}", self.describe_actions(&actions)));
            }
            return Ok(());
        }
        if let Some(name) = s.cast.clone() {
            let actions = self.priority_for(player, &mut s.common)?;
            let alt = if s.face { Some(mtg_engine::decision::FACE_ALT) } else { s.alt };
            let face = s.face;
            let idx = actions.iter().position(|a| matches!(a, Action::Cast { card, alt: a2 } if self.g.card_name(*card) == name && (alt.is_none() || *a2 == alt) && (face || *a2 != Some(mtg_engine::decision::FACE_ALT))));
            let idx = idx.ok_or_else(|| format!("'{name}' can't be cast by player {player} now; legal actions: {}", self.describe_actions(&actions)))?;
            // explicit payment sources ("tap:<card>#<ability>" answers) need the engine to ask how to pay
            if s.common.answers.iter().any(|a| matches!(a, Answer::Text(t) if t.starts_with("tap:"))) {
                self.g.players[player as usize].manual_pay = true;
            }
            self.apply(Choice::Priority { action: Some(idx) })?;
            self.settle_cast(&mut s.common)?;
            self.g.players[player as usize].manual_pay = false;
        } else if let Some(name) = s.play_land.clone() {
            let actions = self.priority_for(player, &mut s.common)?;
            let face = s.face;
            let idx = actions
                .iter()
                .position(|a| match a {
                    Action::PlayLand { card } => !face && self.g.card_name(*card) == name,
                    Action::PlayLandFace { card } => face && self.g.card_name(*card) == name,
                    _ => false,
                })
                .ok_or_else(|| format!("can't play '{name}'"))?;
            self.apply(Choice::Priority { action: Some(idx) })?;
        } else if let Some(face) = s.planar_die.clone() {
            let actions = self.priority_for(player, &mut s.common)?;
            let idx = actions.iter().position(|a| matches!(a, Action::RollPlanarDie)).ok_or_else(|| format!("can't roll the planar die; legal actions: {}", self.describe_actions(&actions)))?;
            let v = match face.as_str() {
                "chaos" => 1,
                "planeswalk" => 2,
                "blank" => 3,
                f => return Err(format!("planar_die face must be chaos, planeswalk or blank, not {f}")),
            };
            self.g.forced_rolls.borrow_mut().insert(0, v);
            self.apply(Choice::Priority { action: Some(idx) })?;
            self.settle_cast(&mut s.common)?;
        } else if let Some(name) = s.companion.clone() {
            let actions = self.priority_for(player, &mut s.common)?;
            let idx = actions.iter().position(|a| matches!(a, Action::Companion { card } if self.g.card_name(*card) == name));
            let idx = idx.ok_or_else(|| format!("can't take the companion action for '{name}'; legal actions: {}", self.describe_actions(&actions)))?;
            self.apply(Choice::Priority { action: Some(idx) })?;
            self.settle_cast(&mut s.common)?;
        } else if let Some(name) = s.unlock.clone() {
            let actions = self.priority_for(player, &mut s.common)?;
            let door = s.door.unwrap_or(1);
            let idx = actions.iter().position(|a| matches!(a, Action::UnlockDoor { card, door: d } if self.g.card_name(*card) == name && *d == door));
            let idx = idx.ok_or_else(|| format!("can't unlock door {door} of '{name}'; legal actions: {}", self.describe_actions(&actions)))?;
            self.apply(Choice::Priority { action: Some(idx) })?;
            self.settle_cast(&mut s.common)?;
        } else if let Some(name) = s.activate.clone() {
            let actions = self.priority_for(player, &mut s.common)?;
            let want = s.ability.unwrap_or(0);
            // a manifested creature card is turned face up by paying its mana cost (a special action, not an ability)
            let found = self.find_activation(&actions, &name, want).or_else(|| actions.iter().position(|a| matches!(a, Action::TurnFaceUp { card } if self.g.card_name(*card) == name)));
            let idx = found.ok_or_else(|| format!("can't activate ability #{want} of '{name}'; legal actions: {}", self.describe_actions(&actions)))?;
            if s.common.answers.iter().any(|a| matches!(a, Answer::Text(t) if t.starts_with("tap:"))) {
                self.g.players[player as usize].manual_pay = true;
            }
            self.apply(Choice::Priority { action: Some(idx) })?;
            self.settle_cast(&mut s.common)?;
            self.g.players[player as usize].manual_pay = false;
        } else if let Some(name) = s.tap_for_mana.clone() {
            self.priority_for(player, &mut s.common)?;
            let mut id = self.find(&name, &[Zone::Battlefield])?;
            // naming a permanent twice taps a second copy
            if self.g.card(id).tapped {
                let (owner, bare) = match name.split_once(':') {
                    Some((p, n)) if p.starts_with('p') && p[1..].parse::<u8>().is_ok() => (Some(p[1..].parse::<u8>().unwrap()), n),
                    _ => (None, name.as_str()),
                };
                if let Some(c) = self.g.cards.iter().find(|c| c.zone == Zone::Battlefield && !c.tapped && self.card_matches(c.id, owner, bare)) {
                    id = c.id;
                }
            }
            let srcs: Vec<_> = self.g.mana_sources(player).into_iter().filter(|m| m.card == id).collect();
            let produce = s.color.as_deref().and_then(|c| match c {
                "W" => Some(mtg_engine::mana::ManaType::W),
                "U" => Some(mtg_engine::mana::ManaType::U),
                "B" => Some(mtg_engine::mana::ManaType::B),
                "R" => Some(mtg_engine::mana::ManaType::R),
                "G" => Some(mtg_engine::mana::ManaType::G),
                "C" => Some(mtg_engine::mana::ManaType::C),
                _ => None,
            });
            let want_t = produce;
            let src = srcs.iter().find(|m| s.ability.map_or(false, |a| m.ability as usize == a as usize)).or_else(|| srcs.iter().find(|m| want_t.map_or(true, |t| m.outputs.iter().any(|o| o.iter().any(|(x, _)| *x == t))))).or(srcs.first()).ok_or_else(|| format!("'{name}' can't be tapped for mana right now"))?;
            let mana_x = s.common.answers.iter().find_map(|a| match a { Answer::Number(n) => Some(*n as i32), Answer::Index(n) => Some(*n as i32), _ => None });
            let picks: Vec<mtg_engine::mana::ManaType> = s.common.answers.iter().filter_map(|a| match a {
                Answer::Text(t) => match t.as_str() { "W" => Some(mtg_engine::mana::ManaType::W), "U" => Some(mtg_engine::mana::ManaType::U), "B" => Some(mtg_engine::mana::ManaType::B), "R" => Some(mtg_engine::mana::ManaType::R), "G" => Some(mtg_engine::mana::ManaType::G), _ => None },
                _ => None,
            }).collect();
            self.apply(Choice::Mana { card: id, ability: src.ability, produce, x: mana_x, picks })?;
            // "a player of your choice adds ...": the question about who gets the mana is answered by this step's answers
            if !s.common.answers.is_empty() && (self.g.mana_gift_ability(player, id, src.ability) || self.g.mana_pick_options(player, id, src.ability).is_some()) {
                self.until_priority(&mut s.common)?;
            }
        } else if s.resolve == Some(true) && s.hold == Some(true) {
            self.resolve_top(&mut s.common)?;
        } else if s.resolve == Some(true) {
            self.resolve_all(&mut s.common)?;
        } else if s.pass == Some(true) {
            let _ = self.priority_for(player, &mut s.common)?;
            self.apply(Choice::Priority { action: None })?;
        } else if let Some(step) = s.advance_to {
            self.advance_to(step, s.hold == Some(true), &mut s.common)?;
        } else if let Some(attackers) = s.attack.clone() {
            self.to_attackers(&mut s.common)?;
            let mut attacks = vec![];
            let mut bands: std::collections::BTreeMap<u8, Vec<CardId>> = Default::default();
            for a in attackers {
                let mut card = self.find(&a.card, &[Zone::Battlefield])?;
                // naming the same card twice declares two different copies as attackers
                if attacks.iter().any(|(c, _): &(CardId, Obj)| *c == card) {
                    let (owner, name) = match a.card.split_once(':') {
                        Some((p, n)) if p.starts_with('p') && p[1..].parse::<u8>().is_ok() => (Some(p[1..].parse::<u8>().unwrap()), n.to_string()),
                        _ => (None, a.card.clone()),
                    };
                    if let Some(c) = self.g.cards.iter().find(|c| c.zone == Zone::Battlefield && self.card_matches(c.id, owner, &name) && !attacks.iter().any(|(x, _): &(CardId, Obj)| *x == c.id)) {
                        card = c.id;
                    }
                }
                if let Some(b) = a.band {
                    bands.entry(b).or_default().push(card);
                }
                let def = match Self::player_ref(&a.at) {
                    Some(p) => Obj::player(p),
                    None => Obj::card(self.find(&a.at, &[Zone::Battlefield])?),
                };
                attacks.push((card, def));
            }
            self.g.attack_bands = bands.into_values().collect();
            self.apply(Choice::Attackers { attacks })?;
        } else if let Some(blocks) = s.block.clone() {
            let asked = self.to_blockers(&mut s.common)?;
            let mut pairs = vec![];
            for b in blocks {
                pairs.push((self.find(&b.blocker, &[Zone::Battlefield])?, self.find(&b.attacker, &[Zone::Battlefield])?));
            }
            if asked {
                self.apply(Choice::Blockers { blocks: pairs })?;
            } else if !pairs.is_empty() {
                return Err("no creature can block, but blocks were declared".into());
            }
        } else if s.concede == Some(true) {
            self.g.concede(player);
        } else if let Some(l) = s.set_life {
            // a declared block is only validated when the engine next runs: do that before changing life
            if self.g.answer.is_some() {
                self.next()?;
            }
            self.g.players[player as usize].life = l;
            self.g.layers_dirty = true;
            self.g.recompute_layers();
        } else if let Some(sp) = s.set_speed {
            self.g.players[player as usize].speed = sp.min(4);
            self.g.layers_dirty = true;
            self.g.recompute_layers();
        } else {
            return Err("empty step".into());
        }
        Ok(())
    }

    /// The action index of the `want`-th printed non-mana activated ability of the named card (stable, independent of what is currently legal).
    fn find_activation(&self, actions: &[Action], name: &str, want: usize) -> Option<usize> {
        // an ability granted by another permanent: give 32768 (+ its index among the granted abilities)
        if want >= 32768 {
            return actions.iter().position(|a| matches!(a, Action::Activate { card, ability } if self.g.card_name(*card) == name && *ability as usize == want));
        }
        // printed ability indices of that card (any copy), in order, skipping mana abilities
        let id = actions.iter().find_map(|a| match a {
            Action::Activate { card, .. } if self.g.card_name(*card) == name => Some(*card),
            _ => None,
        })?;
        let d = self.g.def_of(id);
        let printed: Vec<u16> = d.activated.iter().copied().filter(|i| !d.mana_abilities.contains(i)).collect();
        let target = *printed.get(want)?;
        actions.iter().position(|a| matches!(a, Action::Activate { card, ability } if self.g.card_name(*card) == name && *ability == target))
    }

    fn describe_actions(&self, actions: &[Action]) -> String {
        actions
            .iter()
            .map(|a| match a {
                Action::PlayLand { card } => format!("play {}", self.g.card_name(*card)),
                Action::PlayLandFace { card } => format!("play back face of {}", self.g.card_name(*card)),
                Action::Cast { card, .. } => format!("cast {}", self.g.card_name(*card)),
                Action::Activate { card, ability } => format!("activate {}#{ability}", self.g.card_name(*card)),
                Action::ActivateMana { card, .. } => format!("mana {}", self.g.card_name(*card)),
                Action::TurnFaceUp { card } => format!("unmorph {}", self.g.card_name(*card)),
                Action::Companion { card } => format!("companion {}", self.g.card_name(*card)),
                Action::UnlockDoor { card, door } => format!("unlock door {door} of {}", self.g.card_name(*card)),
                Action::RollPlanarDie => "roll the planar die".to_string(),
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// After choosing an action: answer the casting questions (modes, targets, X, costs) until priority returns.
    fn settle_cast(&mut self, c: &mut Common) -> R<()> {
        self.until_priority(c)?;
        if !c.targets.is_empty() {
            // a target the engine chose by itself (only one legal choice) is fine if the spell ended up with it
            let mut on_stack: Vec<Obj> = self.g.stack.iter().flat_map(|s| s.targets.iter().map(|t| t.obj)).collect();
            let used = std::mem::take(&mut c.used_targets);
            if self.strict_targets {
                for u in used {
                    if let Some(pos) = on_stack.iter().position(|x| *x == u) {
                        on_stack.remove(pos);
                    }
                }
            }
            let mut left = vec![];
            for t in std::mem::take(&mut c.targets) {
                let obj = self.resolve_ref(&t)?;
                if self.strict_targets {
                    // an `illegal` step names each target slot: one object named twice needs two slots
                    if let Some(pos) = on_stack.iter().position(|x| *x == obj) {
                        on_stack.remove(pos);
                    } else {
                        left.push(t);
                    }
                } else if !on_stack.contains(&obj) {
                    left.push(t);
                }
            }
            let warded = self.g.events.iter().rev().take(40).any(|e| matches!(e, mtg_engine::Event::Log { text } if text.starts_with("ward:")));
            if !left.is_empty() && !warded {
                return Err(format!("{} target(s) were not used: {:?}", left.len(), left));
            }
        }
        if !c.answers.is_empty() {
            return Err(format!("{} answer(s) were not used: {:?}", c.answers.len(), c.answers));
        }
        Ok(())
    }

    fn resolve_all(&mut self, c: &mut Common) -> R<()> {
        for _ in 0..500 {
            if self.g.is_over() {
                return self.finish_unused(c);
            }
            let Some(d) = self.next()? else { return Ok(()) };
            match &d.kind {
                DecisionKind::Priority { .. } => {
                    if self.g.stack.is_empty() {
                        return self.finish_unused(c);
                    }
                    self.apply(Choice::Priority { action: None })?;
                }
                _ => self.answer_decision(&d, c)?,
            }
        }
        Err("the stack never emptied".into())
    }

    /// `resolve` with `hold`: resolve only the item on top of the stack now, stopping at the next priority (a trigger it caused stays on the stack).
    fn resolve_top(&mut self, c: &mut Common) -> R<()> {
        let top = self.g.stack.last().map(|i| i.id);
        for _ in 0..500 {
            if self.g.is_over() {
                return self.finish_unused(c);
            }
            let Some(d) = self.next()? else { return Ok(()) };
            match &d.kind {
                DecisionKind::Priority { .. } => {
                    if self.g.stack.last().map(|i| i.id) != top {
                        return self.finish_unused(c);
                    }
                    self.apply(Choice::Priority { action: None })?;
                }
                _ => self.answer_decision(&d, c)?,
            }
        }
        Err("the top item never resolved".into())
    }

    fn finish_unused(&self, c: &Common) -> R<()> {
        if !c.targets.is_empty() {
            return Err(format!("unused targets: {:?}", c.targets));
        }
        if !c.answers.is_empty() {
            return Err(format!("unused answers: {:?}", c.answers));
        }
        Ok(())
    }

    /// Pass priority until the first priority at `step` that comes after the current moment (this turn if the step is still
    /// ahead, otherwise the next turn's).
    fn advance_to(&mut self, step: Step, hold: bool, c: &mut Common) -> R<()> {
        let (start_turn, start_step) = (self.g.turn, self.g.step as u8);
        // a step seen again after other steps (an extra combat after the second main phase, a second main phase) is later too
        let mut left_start = false;
        for _ in 0..2000 {
            let Some(d) = self.next()? else { return Ok(()) };
            match &d.kind {
                DecisionKind::Priority { .. } => {
                    left_start |= self.g.turn != start_turn || self.g.step as u8 != start_step;
                    let later = left_start || (self.g.step as u8) > start_step;
                    if self.g.step == step && (hold || self.g.stack.is_empty()) && later {
                        return Ok(());
                    }
                    self.apply(Choice::Priority { action: None })?;
                }
                _ => self.answer_decision(&d, c)?,
            }
        }
        Err(format!("never reached {step:?}"))
    }

    fn to_attackers(&mut self, c: &mut Common) -> R<()> {
        let turn = self.g.turn;
        // (turn, active player, a combat of that turn was still ahead): an attack step must not roll over a combat of the
        // attacking player in which the engine offered no declaration (the attack was refused or nothing could attack)
        let combat_ahead = (self.g.step as u8) < Step::DeclareAttackers as u8 || self.g.extra_combat_now > 0 || self.g.extra_combat_after_main > 0;
        let mut turns: Vec<(u32, u8, bool)> = vec![(turn, self.g.active, combat_ahead)];
        for _ in 0..2000 {
            if self.same_turn_attack && self.g.turn != turn {
                return Err(format!("no declare attackers decision in turn {turn}"));
            }
            let Some(d) = self.next()? else { return Err(format!("game ended before attackers ({:?}; turn {} step {:?})", self.g.over, self.g.turn, self.g.step)) };
            if turns.last().map_or(false, |t| t.0 != self.g.turn) {
                turns.push((self.g.turn, self.g.active, true));
            }
            match &d.kind {
                DecisionKind::Attackers { .. } => {
                    let (now, ap) = (self.g.turn, self.g.active);
                    if let Some(t) = turns.iter().find(|t| t.0 < now && t.1 == ap && t.2) {
                        return Err(format!(
                            "no declare attackers decision in turn {} (the attack was refused or nothing could attack there); it would roll over to turn {now}: use an `advance_to` first for a later turn's attack",
                            t.0
                        ));
                    }
                    return Ok(());
                }
                DecisionKind::Priority { .. } => self.apply(Choice::Priority { action: None })?,
                _ => self.answer_decision(&d, c)?,
            }
        }
        Err("never reached the declare attackers decision".into())
    }

    /// Returns false when no blockers decision will come (nothing can block).
    fn to_blockers(&mut self, c: &mut Common) -> R<bool> {
        for _ in 0..2000 {
            let Some(d) = self.next()? else { return Err("game ended before blockers".into()) };
            match &d.kind {
                DecisionKind::Blockers { .. } => return Ok(true),
                DecisionKind::Priority { .. } if (self.g.step as u8) >= Step::DeclareBlockers as u8 => return Ok(false),
                DecisionKind::Priority { .. } => self.apply(Choice::Priority { action: None })?,
                _ => self.answer_decision(&d, c)?,
            }
        }
        Err("never reached the declare blockers decision".into())
    }
}

fn d_prompt(d: &Decision) -> String {
    match &d.kind {
        DecisionKind::Pick { prompt, .. } => prompt.clone(),
        other => format!("{other:?}"),
    }
}

fn names_in(g: &Game, ids: &[CardId]) -> Vec<String> {
    let mut v: Vec<String> = ids.iter().map(|i| g.card_name(*i).to_string()).collect();
    v.sort();
    v
}

fn sorted(v: &[String]) -> Vec<String> {
    let mut v = v.to_vec();
    v.sort();
    v
}

/// Run one scenario against the compiled cards.
pub fn run_scenario(db: &Arc<CardDb>, s: &Scenario) -> Result<(), String> {
    let n = s.players.len().max(2);
    let mut g = Game::new_blank(db.clone(), n, s.seed.unwrap_or(1));
    *g.forced_rolls.borrow_mut() = s.rolls.clone();
    if s.commander {
        g.cfg.commander = true;
    }
    let add = |g: &mut Game, p: u8, zone: Zone, specs: &[CardSpec]| -> Result<(), String> {
        for spec in specs {
            if db.find(spec.name()).is_none() {
                return Err(format!("unknown card '{}'", spec.name()));
            }
            let id = g.add_card(spec.name(), p, zone);
            // shock lands placed by the setup start untapped and are not asked
            let shock = g.def_of(id).abilities.iter().any(|a| matches!(a, mtg_engine::dsl::Ability::EntersPayLifeOrTapped { .. } | mtg_engine::dsl::Ability::EntersRevealOrTapped { .. } | mtg_engine::dsl::Ability::EntersMayTapped { .. }));
            if shock && g.enter_choices.contains(&id) {
                g.enter_choices.retain(|c| *c != id);
                g.card_mut(id).tapped = false;
            }
            // a permanent placed by the setup has been around: its echo cost was already dealt with
            // (a `sick` permanent just entered: its echo is still due)
            if zone == Zone::Battlefield && !matches!(spec, CardSpec::Detailed { sick: true, .. }) {
                let z = g.card(id).zcc;
                g.card_mut(id).echo_done_zcc = z;
            }
            if let CardSpec::Detailed { pregame_for: Some(n), .. } = spec {
                g.card_mut(id).pregame_for = Some(mtg_engine::types::Sym::new(n));
            }
            if let CardSpec::Detailed { tapped, counters, sick, face_down, .. } = spec {
                if *face_down {
                    g.card_mut(id).face_down = true;
                    g.layers_dirty = true;
                }
                if *tapped {
                    g.card_mut(id).tapped = true;
                }
                if *sick {
                    // it just came under its controller's control: sick until that player's next turn begins
                    g.card_mut(id).since = 1;
                }
                for (k, v) in counters {
                    let kind = counter_kind(k).ok_or_else(|| format!("unknown counter '{k}'"))?;
                    // explicit counters set the amount (replacing any it entered with)
                    let have = g.card(id).counter(kind);
                    g.card_mut(id).remove_counters(kind, have);
                    g.card_mut(id).add_counters(kind, *v as i32);
                }
            }
        }
        Ok(())
    };
    for (i, ps) in s.players.iter().enumerate() {
        let p = i as u8;
        if let Some(c) = ps.pregame_color.as_deref() {
            g.players[i].pregame_color = match c {
                "W" => Some(Color::W),
                "U" => Some(Color::U),
                "B" => Some(Color::B),
                "R" => Some(Color::R),
                "G" => Some(Color::G),
                _ => None,
            };
            g.layers_dirty = true;
        }
        if let Some(e) = ps.energy {
            g.players[i].energy = e;
        }
        if let Some(e) = ps.poison {
            g.players[i].poison = e;
        }
        if let Some(e) = ps.rad {
            g.players[i].rad = e;
        }
        if let Some(l) = ps.life {
            g.players[i].life = l;
            g.players[i].life_turn_start = l;
        }
        add(&mut g, p, Zone::Hand, &ps.hand)?;
        add(&mut g, p, Zone::Battlefield, &ps.battlefield)?;
        add(&mut g, p, Zone::Graveyard, &ps.graveyard)?;
        add(&mut g, p, Zone::Exile, &ps.exile)?;
        add(&mut g, p, Zone::Command, &ps.command)?;
        add(&mut g, p, Zone::Sideboard, &ps.sideboard)?;
        if let Some(cn) = &ps.companion {
            let id = g.players[i].sideboard.iter().copied().find(|&c| g.card_name(c) == cn.as_str()).ok_or_else(|| format!("companion '{cn}' is not in the sideboard"))?;
            g.card_mut(id).is_companion = true;
        }
        // filler goes underneath so the listed cards are on top (first listed = top)
        for _ in 0..(if ps.exact_library { 0 } else { 20 }) {
            g.add_card("Forest", p, Zone::Library);
        }
        for spec in ps.library.iter().rev() {
            add(&mut g, p, Zone::Library, std::slice::from_ref(spec))?;
        }
        g.players[i].starting_deck = (g.players[i].library.len() + g.players[i].hand.len()) as i32;
        for id in g.players[i].command.clone() {
            // emblems, planes and schemes are not commanders
            let t = g.def_of(id).types;
            if !g.def_of(id).token && !t.has(CardType::Plane) && !t.has(CardType::Phenomenon) && !t.has(CardType::Scheme) {
                g.card_mut(id).is_commander = true;
            }
        }
    }
    // Planechase: planar deck and face-up planes; Archenemy: scheme decks and teams
    for name in s.planar_deck.iter().rev() {
        let id = g.add_card(name, g.active, Zone::Command);
        let a = g.active as usize;
        g.players[a].command.retain(|x| *x != id);
        g.planar_deck.push(id);
    }
    for spec in &s.planes {
        let id = g.add_card(spec.name(), g.active, Zone::Command);
        g.planes.push(id);
        if let CardSpec::Detailed { counters, .. } = spec {
            for (k, v) in counters {
                let kind = counter_kind(k).ok_or_else(|| format!("unknown counter '{k}'"))?;
                g.card_mut(id).add_counters(kind, *v as i32);
            }
        }
    }
    let arch: Vec<usize> = s.players.iter().enumerate().filter(|(_, p)| !p.schemes.is_empty()).map(|(i, _)| i).collect();
    for (i, ps) in s.players.iter().enumerate() {
        for name in ps.schemes.iter().rev() {
            let id = g.add_card(name, i as u8, Zone::Command);
            g.players[i].command.retain(|x| *x != id);
            g.players[i].scheme_deck.push(id);
        }
    }
    if !arch.is_empty() {
        g.archenemy = true;
        for p in 0..n {
            g.players[p].team = if arch.contains(&p) { 0 } else { 1 };
        }
    }
    // players beyond the listed ones also get a library
    for p in s.players.len()..n {
        for _ in 0..20 {
            g.add_card("Forest", p as u8, Zone::Library);
        }
    }
    // triggers caused by setting the board up (enters-the-battlefield of placed permanents) don't happen
    g.pending_triggers.clear();
    g.events.clear();
    // cards the scenario placed in graveyards (or sacrificed / made) while setting up did not happen "this turn"
    g.gy_entries.clear();
    g.exile_entries.clear();
    g.left_graveyard_by.clear();
    g.left_graveyard_creature_by.clear();
    g.sacrificed_turn.clear();
    g.tokens_made_by.clear();
    g.bounced_turn.clear();
    g.left_bf_turn.clear();
    g.died_turn.clear();
    g.attacked_players_turn.clear();
    g.layers_dirty = true;
    // lands placed by the setup did not "enter this turn"
    for p in g.players.iter_mut() {
        p.lands_entered = 0;
        p.creatures_entered = 0;
        p.creatures_entered_last = 0;
    }
    g.lands_entered_turn = 0;
    for c in g.cards.iter_mut() {
        c.entered_turn = 0;
    }
    g.entered_log.clear();
    g.attackers_log.clear();
    g.start_at(0, s.start.unwrap_or(Step::Main1));
    let mut r = Runner { g, trace: vec![], strict_targets: false, same_turn_attack: false, declared: None };
    for (i, step) in s.steps.iter().enumerate() {
        let mut st = step.clone();
        r.trace.push(format!("step {i}"));
        r.do_step(&mut st).map_err(|e| format!("step {i} ({}): {e}", describe_step(step)))?;
    }
    // let triggers waiting on the engine settle
    r.g.run();
    check_expect(&r.g, &s.expect)
}

fn describe_step(s: &Step_) -> String {
    if let Some(c) = &s.cast {
        format!("cast {c}")
    } else if let Some(c) = &s.play_land {
        format!("play {c}")
    } else if let Some(c) = &s.activate {
        format!("activate {c}")
    } else if s.resolve == Some(true) {
        "resolve".into()
    } else if s.pass == Some(true) {
        "pass".into()
    } else if let Some(st) = s.advance_to {
        format!("advance to {st:?}")
    } else if s.attack.is_some() {
        "attack".into()
    } else if s.block.is_some() {
        "block".into()
    } else {
        "…".into()
    }
}

fn cmp(errs: &mut Vec<String>, i: usize, what: &str, got: Vec<String>, want: &Vec<String>) {
    if got != sorted(want) {
        errs.push(format!("player {i} {what}: expected {:?}, got {:?}", sorted(want), got));
    }
}

fn check_expect(g: &Game, e: &Expect) -> Result<(), String> {
    let mut errs: Vec<String> = vec![];
    for (i, pe) in e.players.iter().enumerate() {
        let Some(p) = g.players.get(i) else {
            errs.push(format!("no player {i}"));
            continue;
        };
        let bf: Vec<CardId> = g.battlefield.iter().copied().filter(|c| g.card(*c).cur.controller == i as u8).collect();
        let bf_names = names_in(g, &bf);
        if let Some(l) = pe.life {
            if p.life != l {
                errs.push(format!("player {i} life: expected {l}, got {}", p.life));
            }
        }
        if let Some(w) = &pe.hand {
            cmp(&mut errs, i, "hand", names_in(g, &p.hand), w);
        }
        if let Some(w) = &pe.battlefield {
            cmp(&mut errs, i, "battlefield", bf_names.clone(), w);
        }
        if let Some(w) = &pe.graveyard {
            cmp(&mut errs, i, "graveyard", names_in(g, &p.graveyard), w);
        }
        if let Some(w) = &pe.exile {
            cmp(&mut errs, i, "exile", names_in(g, &p.exile), w);
        }
        if let Some(w) = &pe.command {
            cmp(&mut errs, i, "command zone", names_in(g, &p.command), w);
        }
        if let Some(w) = &pe.sideboard {
            cmp(&mut errs, i, "sideboard", names_in(g, &p.sideboard), w);
        }
        if let Some(n) = pe.hand_count {
            if p.hand.len() != n {
                errs.push(format!("player {i} hand size: expected {n}, got {}", p.hand.len()));
            }
        }
        if let Some(n) = pe.library_count {
            // the scenario adds 20 filler cards per library
            if p.library.len() != n + 20 && p.library.len() != n {
                errs.push(format!("player {i} library: expected {n} (+20 filler), got {}", p.library.len()));
            }
        }
        if let Some(top) = &pe.library_top {
            let got: Vec<String> = p.library.iter().rev().take(top.len()).map(|&c| g.def_of(c).name.clone()).collect();
            if &got != top {
                errs.push(format!("player {i} library top: expected {top:?}, got {got:?}"));
            }
        }
        if let Some(n) = pe.poison {
            if p.poison != n {
                errs.push(format!("player {i} poison: expected {n}, got {}", p.poison));
            }
        }
        if let Some(n) = pe.speed {
            if p.speed != n {
                errs.push(format!("player {i} speed: expected {n}, got {}", p.speed));
            }
        }
        if let Some(n) = pe.contraptions {
            if p.contraptions != n {
                errs.push(format!("player {i} contraptions: expected {n}, got {}", p.contraptions));
            }
        }
        if let Some(n) = pe.rad {
            if p.rad != n {
                errs.push(format!("player {i} rad: expected {n}, got {}", p.rad));
            }
        }
        if let Some(n) = pe.energy {
            if p.energy != n {
                errs.push(format!("player {i} energy: expected {n}, got {}", p.energy));
            }
        }
        for (k, n) in pe.battlefield_has.iter().enumerate() {
            // naming a card twice means at least two copies
            let want = pe.battlefield_has[..=k].iter().filter(|m| *m == n).count();
            if bf_names.iter().filter(|m| *m == n).count() < want {
                errs.push(format!("player {i} battlefield lacks {n}: {:?}", bf_names));
            }
        }
        // a face-down permanent has no name: compare against what the battlefield shows
        let bf_visible: Vec<String> = bf.iter().map(|i| g.view_name(&g.view(*i))).collect();
        for n in &pe.battlefield_lacks {
            if bf_visible.contains(n) {
                errs.push(format!("player {i} battlefield unexpectedly has {n}"));
            }
        }
        for n in &pe.graveyard_has {
            if !names_in(g, &p.graveyard).contains(n) {
                errs.push(format!("player {i} graveyard lacks {n}: {:?}", names_in(g, &p.graveyard)));
            }
        }
        if let Some(rp) = &pe.restricted_pool {
            for (k, v) in rp {
                let t = match k.as_str() {
                    "W" => mtg_engine::mana::ManaType::W,
                    "U" => mtg_engine::mana::ManaType::U,
                    "B" => mtg_engine::mana::ManaType::B,
                    "R" => mtg_engine::mana::ManaType::R,
                    "G" => mtg_engine::mana::ManaType::G,
                    _ => mtg_engine::mana::ManaType::C,
                };
                let have = p.restricted.iter().flatten().filter(|(ty, _, _)| *ty == t).count() as u16;
                if have != *v {
                    errs.push(format!("player {i} restricted pool {k}: expected {v}, got {have}"));
                }
            }
        }
        if let Some(pool) = &pe.pool {
            for (k, v) in pool {
                let t = match k.as_str() {
                    "W" => mtg_engine::mana::ManaType::W,
                    "U" => mtg_engine::mana::ManaType::U,
                    "B" => mtg_engine::mana::ManaType::B,
                    "R" => mtg_engine::mana::ManaType::R,
                    "G" => mtg_engine::mana::ManaType::G,
                    _ => mtg_engine::mana::ManaType::C,
                };
                if p.pool.get(t) != *v {
                    errs.push(format!("player {i} pool {k}: expected {v}, got {}", p.pool.get(t)));
                }
            }
        }
    }
    if let Some(b) = e.stack_empty {
        if g.stack.is_empty() != b {
            errs.push(format!("stack_empty: expected {b}, stack has {} item(s)", g.stack.len()));
        }
    }
    if let Some(st) = e.step {
        if g.step != st {
            errs.push(format!("step: expected {st:?}, got {:?}", g.step));
        }
    }
    if let Some(t) = e.turn {
        if g.turn != t {
            errs.push(format!("turn: expected {t}, got {}", g.turn));
        }
    }
    if let Some(w) = e.winner {
        let got: i32 = match &g.over {
            Some(GameResult::Winner { player }) => *player as i32,
            Some(GameResult::Draw) => -2,
            None => -1,
        };
        if got != w {
            errs.push(format!("winner: expected {w}, got {got}"));
        }
    }
    // "p1:Walking Ballista" names the one that player controls
    let find_bf = |spec: &str| {
        let (ctl, name) = match spec.split_once(':') {
            Some((p, n)) if p.starts_with('p') && p[1..].parse::<u8>().is_ok() => (Some(p[1..].parse::<u8>().unwrap()), n),
            _ => (None, spec),
        };
        g.battlefield.iter().chain(g.planes.iter()).copied().find(|c| g.card_name(*c) == name && ctl.map_or(true, |o| g.card(*c).cur.controller == o))
    };
    for (name, kinds) in &e.counters {
        match find_bf(name) {
            None => errs.push(format!("counters: '{name}' is not on the battlefield")),
            Some(id) => {
                for (k, n) in kinds {
                    match counter_kind(k) {
                        None => errs.push(format!("unknown counter '{k}'")),
                        Some(kind) => {
                            if g.card(id).counter(kind) != *n {
                                errs.push(format!("{name} {k} counters: expected {n}, got {}", g.card(id).counter(kind)));
                            }
                        }
                    }
                }
            }
        }
    }
    for (name, [p, t]) in &e.pt {
        match find_bf(name) {
            None => errs.push(format!("pt: '{name}' is not on the battlefield")),
            Some(id) => {
                let c = g.card(id);
                if (c.cur.power, c.cur.toughness) != (*p, *t) {
                    errs.push(format!("{name} P/T: expected {p}/{t}, got {}/{}", c.cur.power, c.cur.toughness));
                }
            }
        }
    }
    for (name, want) in &e.colors {
        match find_bf(name) {
            None => errs.push(format!("colors: '{name}' is not on the battlefield")),
            Some(id) => {
                let cs = g.card(id).cur.colors;
                let mut have: Vec<String> = ["W", "U", "B", "R", "G"].iter().zip([Color::W, Color::U, Color::B, Color::R, Color::G]).filter(|(_, c)| cs.has(*c)).map(|(l, _)| l.to_string()).collect();
                let mut want = want.clone();
                have.sort();
                want.sort();
                if have != want {
                    errs.push(format!("{name} colors: expected {want:?}, got {have:?}"));
                }
            }
        }
    }
    for (name, want) in &e.tapped {
        match find_bf(name) {
            None => errs.push(format!("tapped: '{name}' is not on the battlefield")),
            Some(id) => {
                if g.card(id).tapped != *want {
                    errs.push(format!("{name} tapped: expected {want}"));
                }
            }
        }
    }
    for (name, kws) in &e.keywords {
        match find_bf(name) {
            None => errs.push(format!("keywords: '{name}' is not on the battlefield")),
            Some(id) => {
                for k in kws {
                    if !g.card(id).cur.kw.has(*k) {
                        errs.push(format!("{name} should have {k:?}"));
                    }
                }
            }
        }
    }
    for (map, want) in [(&e.subtypes, true), (&e.not_subtypes, false)] {
        for (name, subs) in map {
            match find_bf(name) {
                None => errs.push(format!("subtypes: '{name}' is not on the battlefield")),
                Some(id) => {
                    for sub in subs {
                        let card = g.card(id);
                        // a changeling is every creature type
                        let has = card.cur.subtypes.iter().any(|x| *x == mtg_engine::types::Sym::new(sub))
                            || (card.cur.kw.has(mtg_engine::types::Keyword::Changeling) && card.cur.types.has(mtg_engine::types::CardType::Creature) && mtg_engine::dsl::SubtypeKind::Creature.contains(sub.as_str()));
                        if has != want {
                            errs.push(format!("{name} should {}have subtype {sub}", if want { "" } else { "not " }));
                        }
                    }
                }
            }
        }
    }
    for (name, host) in &e.attached {
        // any permanent with that name attached to any permanent with the host's name satisfies it
        let present = find_bf(name).is_some() && find_bf(host).is_some();
        let ok = g.battlefield.iter().copied().any(|a| {
            g.card_name(a) == name.as_str() && g.card(a).attached_to.map_or(false, |h| g.card_name(h) == host.as_str())
        });
        if !present {
            errs.push(format!("attached: '{name}' or '{host}' is not on the battlefield"));
        } else if !ok {
            errs.push(format!("{name} should be attached to {host}"));
        }
    }
    for (name, seat) in &e.protector {
        match find_bf(name) {
            None => errs.push(format!("protector: '{name}' is not on the battlefield")),
            Some(b) if g.card(b).protector != Some(*seat) => errs.push(format!("{name} should be protected by player {seat}, is {:?}", g.card(b).protector)),
            _ => {}
        }
    }
    if errs.is_empty() {
        Ok(())
    } else {
        Err(errs.join("\n  "))
    }
}
