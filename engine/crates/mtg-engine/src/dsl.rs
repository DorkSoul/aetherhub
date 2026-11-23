//! The card definition DSL. Cards are JSON documents (one per card) that deserialize into `CardDef`.
//! Everything a card can do is built from the small set of primitives here; nothing is interpreted from text at play time.

use crate::types::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

fn battlefield_zone() -> Zone {
    Zone::Battlefield
}
fn is_battlefield_zone(z: &Zone) -> bool {
    *z == Zone::Battlefield
}
fn is_false(b: &bool) -> bool {
    !*b
}
fn is_zero(n: &i32) -> bool {
    *n == 0
}
fn one() -> i32 {
    1
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CardDef {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub mana_cost: String,
    pub types: Vec<CardType>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supertypes: Vec<Supertype>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subtypes: Vec<String>,
    /// Color indicator (overrides the colors of the mana cost).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colors: Option<Vec<Color>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub power: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toughness: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loyalty: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub defense: Option<i32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keywords: Vec<Keyword>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub abilities: Vec<Ability>,
    /// Other face of a double-faced / modal / flip card; the front face is the card itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub back: Option<Box<CardDef>>,
    /// How the two faces relate: transform (default; the back is only reached by transforming), modal (either face can be cast
    /// or played from hand) or split (either half can be cast from hand; in the graveyard/hand only the first half is modelled).
    #[serde(default, skip_serializing_if = "is_default_layout")]
    pub layout: Layout,
    #[serde(default, skip_serializing_if = "is_false")]
    pub token: bool,
    /// The name shown to players when it differs from `name` (a token file named "Elemental X-X Red Trample" shows as "Elemental";
    /// a named token such as "Beau"). Without it a token named like "Beast 4-4 Trample" shows everything before the "4-4".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Reference text only (shown in the UI); never interpreted.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub oracle_text: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Layout {
    #[default]
    Transform,
    Modal,
    Split,
    /// Adventure: the back face is the adventure spell; when it resolves the card is exiled and may later be cast as its front face.
    Adventure,
    /// Omen: the back face is a sorcery/instant "omen spell"; when it resolves (not countered, not fizzled) its card is shuffled into its owner's library instead of going to the graveyard.
    Omen,
    /// Prepare: the back face is a sorcery/instant "prepare spell". It can't be cast from hand; while the creature is prepared
    /// (`Effect::BecomePrepared`) a copy of the spell waits in exile and may be cast, which unprepares the creature.
    Prepare,
    /// Room: a two-door enchantment. The front's abilities belong to door 1, the back's (in `back.abilities`) to door 2; cast either half
    /// (that door unlocks), and unlock the other for its mana cost as a sorcery. Abilities only work while their door is unlocked.
    Room,
}

fn is_default_layout(l: &Layout) -> bool {
    *l == Layout::Transform
}

/// A change to a token copy (copiable values, so they are permanent).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum CopyExcept {
    NotLegendary,
    /// "except its name is X" (the copy keeps this name: legend rule, name filters).
    SetName { name: String },
    /// "except it has no mana cost" (its mana value is 0): embalm / eternalize tokens.
    NoManaCost,
    /// "except it's legendary" / "snow in addition to its other types".
    AddSupertype { supertype: Supertype },
    /// "except it has this ability": the ability at that index of the SOURCE's printed card (no need to nest the ability inside itself).
    GrantOwnAbility { index: u16 },
    /// "except its power and toughness are each equal to ...": a characteristic-defining ability the copy has (Colossal Clone Master).
    CdaPT { power: Amount, toughness: Amount },
    AddType { card_type: CardType },
    /// "except it's a Food artifact and it loses all other card types" (Shelob, Child of Ungoliant): the copy's card types and subtypes become exactly these.
    SetTypes { types: Vec<CardType>, subtypes: Vec<String> },
    AddSubtype { subtype: String },
    AddKeyword { keyword: Keyword },
    SetPT { power: i32, toughness: i32 },
    /// "its power is equal to ... and its toughness is equal to ...": amounts worked out when the copy is made (Benthic Anomaly: totals of a `StoreRemembered` set).
    SetPTAmount { power: Amount, toughness: Amount },
    /// "its power and toughness are equal to this creature's power and toughness" (the copying permanent's own, prototype included: Hulking Metamorph).
    SetPTOwn,
    SetColors { colors: Vec<Color> },
    /// "in addition to its other colors" (Invasion of Amonkhet // Lazotep Convert).
    AddColors { colors: Vec<Color> },
    /// Replace the copy's subtypes ("except it's a 4/4 black Zombie").
    SetSubtypes { subtypes: Vec<String> },
    /// "except it has this ability" (the ability is added to the copy).
    GrantAbility { ability: Box<Ability> },
    /// "except it enters with N counters" (counters are put on the copy right away).
    /// "except it has vanishing N if that creature doesn't have vanishing" (Flesh Duplicate): `n` time counters and the upkeep / last-counter abilities, unless the copied card already has vanishing.
    Vanishing { n: i32 },
    /// "except it enters with N counters if you controlled the copied permanent" (Undercover Operative: a shield counter); only for `EntersAsCopy`.
    AddCountersIfControlled { kind: CounterKind, n: i32 },
    /// "it enters with additional counters on it equal to the same number and kinds of counters the copied permanent has" (Dominion Saboteur); only for `EntersAsCopy`.
    SameCounters,
    /// "The copy has starting loyalty N" (Ob Nixilis, the Adversary): when a copy of a planeswalker spell becomes a token, its loyalty counters are set to the amount, evaluated with the copied spell's event context (`PowerOf` `TriggerObject` = the creature sacrificed to pay its casualty).
    SetLoyalty { amount: Amount },
    AddCounters {
        kind: CounterKind,
        n: i32,
        /// `n` counters for each X paid for the source ("enters with X additional +1/+1 counters": Altered Ego).
        #[serde(default, skip_serializing_if = "is_false")]
        per_x: bool,
        /// Only if the copy is a creature ("if it's a creature, it enters with two +1/+1 counters": Moritte of the Frost).
        #[serde(default, skip_serializing_if = "is_false")]
        if_creature: bool,
        /// Only if the copy is a planeswalker ("it enters with an additional loyalty counter": Spark Double).
        #[serde(default, skip_serializing_if = "is_false")]
        if_planeswalker: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SagaChapter {
    /// Chapter numbers sharing this ability ("II, III" = [2, 3]).
    pub numbers: Vec<u8>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub targets: Vec<TargetSpec>,
    pub effects: Vec<Effect>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum CmpOp {
    Eq,
    Le,
    Ge,
    Lt,
    Gt,
}

impl CmpOp {
    pub fn test(self, v: i32, w: i32) -> bool {
        match self {
            CmpOp::Eq => v == w,
            CmpOp::Le => v <= w,
            CmpOp::Ge => v >= w,
            CmpOp::Lt => v < w,
            CmpOp::Gt => v > w,
        }
    }
}

/// "A player who was dealt combat damage this turn by `at_least` or more different creatures matching `filter`" (Admiral Beckett Brass: three Pirates).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HitByCount {
    pub filter: FilterBox,
    pub at_least: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AmountCmp {
    pub op: CmpOp,
    pub amount: Box<Amount>,
}

/// The four kinds of bending (`Trigger::Bends`, `Event::Bent`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export)]
pub enum BendKind {
    Water,
    Earth,
    Fire,
    Air,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum PlayerCounterKind {
    Energy,
    Poison,
    Experience,
    Rad,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum SkipPhase {
    DrawStep,
    MainPhases,
    CombatPhases,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum ChooseKind {
    /// "As this enters, pay any amount of life" (Nameless Race): the amount (0 up to `EntersChoose.cap`) is stored as the chosen number, read with `Amount::ChosenNumber`.
    PayLife,
    CreatureType,
    Color,
    BasicLandType,
    /// "Choose Island or Swamp" (Roots of Life): stored like `BasicLandType` in `Card.chosen_type`.
    IslandOrSwamp,
    /// The second basic land type of "choose two basic land types" (after a `BasicLandType` choice; must differ from the first): stored in `Card.chosen_type2` (Illusionary Terrain).
    SecondBasicLandType,
    /// Any land type (basic or not: Desert, Gate, Locus, Urza's ...; stored in `chosen_type`: Shimmer).
    LandType,
    /// "As this enters, exile an instant or sorcery card from your graveyard" (Living Lore): the card is tagged as exiled with this permanent (`Filter.exiled_with_this`, `Amount::HighestManaValue`); no prompt when there is no such card.
    ExileSpellFromGraveyard,
    /// A planeswalker type (any planeswalker subtype printed in the card database; stored in `chosen_type`, `Filter.chosen_type` matches subtypes: Deification).
    PlaneswalkerType,
    /// A card type (Artifact, Battle, Creature, Enchantment, Instant, Land, Planeswalker, Sorcery): `Filter.chosen_type` then matches card types.
    CardType,
    /// Read ahead: choose a chapter number; the Saga enters with that many lore counters and only that chapter triggers.
    SagaChapter,
    /// Choose an opponent (stored like a chosen player: `Sel::ChosenPlayer`; with one opponent it is chosen automatically).
    Opponent,
    /// Choose a player (any player, you included).
    Player,
    /// A Siege's protector: the opponent its controller chooses as it enters (CR 310.11a). Added to every Siege when the
    /// card loads; with one opponent it is chosen automatically.
    Protector,
    /// "Enters under the control of an opponent of your choice" (Xantcha, Sleeper Agent): the player who would have
    /// controlled it chooses the opponent. Added to every `EntersUnderOpponent` card when the card loads.
    UnderOpponent,
    /// The second player of "choose two players" (after a `Player` choice; must differ from the first): stored in `Card.chosen_player2`, used by `Replace::Damage.between_chosen_players`.
    SecondPlayer,
    /// The second color of "choose two colors" (after a `Color` choice; must differ from the first): stored in `Card.chosen_color2` (Seal of the Guildpact).
    SecondColor,
    /// Choose odd or even (stored in `chosen_type` as "Odd"/"Even"; `Filter.mana_value_chosen_parity` tests it: Gollum, Riddle Master).
    OddEven,
    /// Choose Khans or Dragons (stored in `chosen_type`; `Condition::ChosenTypeIs` reads it: Frontier Siege).
    KhansDragons,
    /// Choose "5/1" or "1/5" (stored in `chosen_type`; `Condition::ChosenTypeIs` reads it: Aquamorph Entity). Asked again when a face-down permanent with it is turned face up.
    FiveOneOrOneFive,
    /// Choose Elemental, Elf, Faerie, Giant, Goblin, Kithkin, Merfolk or Treefolk (stored in `chosen_type`; Eclipsed Realms).
    EclipsedTypes,
    /// "As this enters, choose 2, 3, or 4 at random": nobody is asked; the number is stored as the chosen number (Haktos the Unscarred; `Filter.mana_value_not_chosen_number`).
    RandomTwoToFour,
    /// Choose left or right (stored in `chosen_type` as "Left"/"Right"; Left = next in turn order, Right = previous: Pramikon, Sky Rampart).
    LeftRight,
    /// Choose Mardu or Jeskai (stored in `chosen_type`; `Condition::ChosenTypeIs` reads it: Windcrag Siege).
    MarduJeskai,
    /// "Choose a card type shared among two exiled cards used to craft it" (Apex Observatory): only card types that at least two cards exiled with this permanent share can be chosen; stored like `CardType`.
    SharedCraftType,
    /// "As this enters, each player chooses war or peace" (Archangel of Strife): the controller first, then each other player in turn order; stored per player in `Card.player_pick`, read by `Filter.controller_chose` ("War" / "Peace").
    WarPeace,
}

// ---------------------------------------------------------------- abilities

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum Ability {
    /// The effect of an instant or sorcery (or an Aura/Equipment attach, etc.).
    Spell {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        targets: Vec<TargetSpec>,
        effects: Vec<Effect>,
    },
    /// "Rather than pay {2} for each previous time you've cast this spell from the command zone, pay 2 life that many times" (Liesa, Shroud of Dusk).
    CommanderTaxInLife,
    /// "X" that is not paid in the mana cost but fixed when the spell is cast / an ability of this card is activated: `Amount::X` in its effects reads this value
    /// (Monstrous Onslaught: X = greatest power among creatures you control; Undercity Upheaval: creature cards in your graveyard).
    XFromAmount { amount: Amount },
    /// Choose one (or more) of several spell modes; each mode has its own targets.
    Modal {
        choose_min: u8,
        choose_max: u8,
        modes: Vec<Mode>,
        /// "You may choose the same mode more than once" (Eldrazi Confluence).
        #[serde(default, skip_serializing_if = "is_false")]
        allow_repeat: bool,
        /// "Choose X" (Doomsday Confluence): X is announced first and exactly X modes are then chosen (`choose_min`/`choose_max` are ignored).
        #[serde(default, skip_serializing_if = "is_false")]
        x_modes: bool,
        /// Escalate: this cost is paid once for each mode chosen beyond the first.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        escalate: Vec<Cost>,
        /// "If <condition>, you may choose both / all": while the condition holds (evaluated for the caster when the modes are chosen) the maximum becomes this number.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_if: Option<(Condition, u8)>,
        /// While the condition holds the MINIMUM number of modes becomes this ("if teamwork was paid, choose both" is mandatory).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min_if: Option<(Condition, u8)>,
        /// "Choose any number of modes that total exactly N": the chosen modes' `paw` values (used as weights) must add up to exactly this (Pick Your Poison).
        #[serde(default, skip_serializing_if = "is_zero_u8")]
        total_paw: u8,
    },
    Activated {
        cost: Vec<Cost>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        targets: Vec<TargetSpec>,
        effects: Vec<Effect>,
        #[serde(default)]
        timing: Timing,
        /// Planeswalker loyalty ability: the cost is a loyalty change (positive adds).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        loyalty: Option<i32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit_per_turn: Option<u8>,
        /// Exhaust: this ability can be activated only once per object (reset when it leaves the battlefield).
        #[serde(default, skip_serializing_if = "is_false")]
        once: bool,
        /// "Activate only if ..." (checked when activating).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        condition: Option<Condition>,
        /// Zone the ability can be activated from (default: battlefield).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<Zone>,
        /// A power-up ability (Marvel "Power-up" ability word); `Trigger::AbilityActivated.power_up` matches only these.
        #[serde(default, skip_serializing_if = "is_false")]
        power_up: bool,
        /// A special action (CR 116.2): activating it never uses the stack, so it resolves at once (paying {U} to end a Gliding Licid's effect).
        #[serde(default, skip_serializing_if = "is_false")]
        special_action: bool,
    },
    Triggered {
        trigger: Trigger,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        targets: Vec<TargetSpec>,
        effects: Vec<Effect>,
        /// "you may" - the controller decides on resolution.
        #[serde(default, skip_serializing_if = "is_false")]
        optional: bool,
        /// Intervening "if" clause, checked on trigger and again on resolution.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        condition: Option<Condition>,
        /// Zone the source must be in for the trigger to be watching (default: battlefield).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<Zone>,
        /// "This ability triggers only once each turn."
        #[serde(default, skip_serializing_if = "is_false")]
        once_per_turn: bool,
        /// "This ability triggers only twice each turn" (Nadu, Winged Wisdom): at most this many times each turn (0 = no limit); counted per permanent.
        #[serde(default, skip_serializing_if = "is_zero_u8")]
        limit_per_turn: u8,
        /// "This ability triggers only once" (ever, while this object stays in the zone it triggered in: Acrobatic Cheerleader).
        #[serde(default, skip_serializing_if = "is_false")]
        once_ever: bool,
    /// "One or more ...": simultaneous events (cards leaving together, several tokens created at once) trigger it only once.
        #[serde(default, skip_serializing_if = "is_false")]
        batched: bool,
        /// "As this enters ...": the trigger resolves before state-based actions are checked, so a 0/0 body isn't killed while it waits (Wood Elemental, Mimeoplasm).
        #[serde(default, skip_serializing_if = "is_false")]
        as_enters: bool,
        /// The condition is checked only when the ability triggers, never again on resolution (an event-dependent condition the card encodes against the entering / cast object: Valakut's "five other Mountains", a per-colour trigger set).
        #[serde(default, skip_serializing_if = "is_false")]
        no_recheck: bool,
    },
    Static {
        effect: StaticEffect,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        condition: Option<Condition>,
        /// Zone the source must be in for this to apply (default: battlefield).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<Zone>,
    },
    /// Suspend N—cost: from hand, pay `cost` to exile this with N time counters; at the start of each of your upkeeps remove one,
    /// and when the last is gone cast it without paying its mana cost. (Expanded into an activated and a triggered ability when the card loads.)
    /// `x`: Suspend X ({X}{B}{B}{B}, X can't be 0): `cost` holds the X symbol and the card gets X time counters.
    Suspend { n: u8, cost: String, #[serde(default, skip_serializing_if = "is_false")] x: bool },
    /// Foretell {cost}: on your turn pay {2} to exile this from your hand (a special action); on a later turn cast it from exile for `cost`.
    /// Expanded into an activated ability and an alternative cost when the card loads.
    Foretell { cost: String },
    /// "This creature enters prepared" (expanded into an enters trigger with `BecomePrepared`).
    EntersPrepared,
    /// Old Companion (The Companion of the Wilds): if this is your companion, you may cast it from your sideboard without first paying {3}.
    OldCompanion,
    /// "If this is your commander, choose a color before the game begins" (Clara Oswald): asked in the pre-game for a commander in the command zone; stored in `Player.pregame_color` and read by `StaticEffect::SetColorsPregame`.
    PregameChooseColor,
    /// Devour N: as this enters you may sacrifice any number of other creatures; it gets N +1/+1 counters for each (done when it enters).
    Devour { n: u8 },
    /// "Any player may activate this ability": every non-mana activated ability of this permanent can be activated by any player (they pay the cost
    /// and are "you" for the ability's effects).
    AnyPlayerMayActivate,
    /// Marks a permanent whose leaves-the-battlefield abilities read `Amount::LkiCount`: when it leaves, the battlefield (plus permanents that left in the same batch) is recorded in its last-known information (Kodama of the Center Tree).
    LkiSnapshot,
    /// "Only your opponents may activate this ability" (Detention Vortex, Knight of the Holy Nimbus): every non-mana activated ability of this permanent
    /// can be activated only by its controller's opponents (they pay the cost and are "you" for the effects).
    OnlyOpponentsMayActivate,
    /// Marks the NEXT activated ability of this permanent as one only the controller of the permanent it is attached to may activate (they pay the cost and are "you" for it),
    /// instead of this permanent's own controller (Volrath's Curse). Other abilities keep their normal activators.
    EnchantedControllerMayActivate,
    /// Umbra armor (on an Aura): if the enchanted permanent would be destroyed, instead remove all damage from it and destroy this Aura.
    UmbraArmor,
    /// Bestow {cost}: may be cast for `cost` as an Aura that enchants a creature (and becomes a creature again when unattached). The card's other
    /// abilities are written normally (an `EnchantedGets` static gives the bonus while it is attached).
    Bestow { cost: String },
    /// Encore {cost}: from the graveyard (sorcery speed), pay `cost` and exile this card: for each opponent create a token copy that has haste and must
    /// attack (this engine does not pin it to that opponent), sacrificed at the beginning of the next end step.
    Encore { cost: String },
    /// Prototype {cost} P/T: may be cast for `cost` as a smaller creature (the color and power/toughness of the prototype) - written as an
    /// alternative cost; on the battlefield it has the prototype P/T and the colors of the prototype cost.
    Prototype { cost: String, power: i32, toughness: i32 },
    /// Unleash: it may enter with a +1/+1 counter; it can't block as long as it has a +1/+1 counter.
    Unleash,
    /// A room of a dungeon card (a token card in the command zone): `next` names the rooms it leads to. The room's effect is a normal
    /// `Triggered` ability with `Trigger::EntersRoom { name }` and `from: Command` on the same card.
    Room { name: String, next: Vec<String> },
    /// "Start your engines!": when this enters, if you have no speed your speed becomes 1 (expanded into an enters trigger). Speed then increases by
    /// 1 (max 4) the first time each turn an opponent loses life during your turn. Use `Condition::MaxSpeed` for "Max speed — ..." abilities.
    StartYourEngines,
    /// Plot {cost}: at sorcery speed pay `cost` to exile this from your hand; on a later turn cast it from exile without paying its mana cost, at sorcery speed.
    Plot { cost: String },
    /// Warp {cost}: this may be cast from hand for `cost`; if so, exile it at the next end step, and it may be cast from exile on a later turn for its mana cost.
    Warp { cost: String },
    /// Ninjutsu {cost}: from hand, pay `cost` and return an unblocked attacker you control to its owner's hand: this enters tapped and attacking
    /// (activate during the declare-blockers step or later in combat). Expanded into an activated ability when the card loads.
    Ninjutsu { cost: String },
    /// Compleated: if Phyrexian mana in this planeswalker's cost was paid with life, it enters with two fewer loyalty counters for each such symbol.
    Compleated,
    /// "Spend only mana of the chosen color to activate this ability" (Throne of Eldraine): the generic mana of every activated ability of this card is paid with mana of the color it chose as it entered.
    ActivationManaOnlyChosenColor,
    /// Mana of any type can be spent to activate this permanent's abilities (Sharkey, Tyrant of the Shire): colored and colorless costs are paid with any mana.
    SpendAnyTypeForOwnAbilities,
    /// "Spend only <color> mana on X" (Crypt Rats, Drain Life, Crimson Hellkite): the {X} of this card's spell cost and of its activated abilities can only be paid with mana of that color.
    XOnlyColor {
        color: Color,
        /// "Spend only black and/or red mana on X" (Soul Burn): X may be paid with either color.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        or_color: Option<Color>,
    },
    /// "Spend only colored mana on X. No more than one mana of each color may be spent this way" (Emblazoned Golem's kicker {X}): X is paid with that many mana of different colors
    /// (the maximum is the number of distinct colors the player can pay; when several color sets work the player picks which).
    XColoredDistinct,
    /// "X can't be greater than <amount>" (Open the Way: the number of players); checked when X is chosen.
    XCap { amount: Amount },
    /// "As an additional cost to cast this spell, choose a number between min and max" (Liquid Fire): asked on cast, stored as the card's chosen number (read with `Amount::ChosenNumber`).
    ChooseNumberOnCast { min: i32, max: i32 },
    /// "As an additional cost to cast this spell, you may remove any number of `kind` counters from among creatures you control. This spell costs {per} less to cast for each counter removed this way" (Hierophant Bio-Titan): asked on cast; the counters come off the creatures you control (most plentiful first).
    RemoveCountersReduceCost { kind: CounterKind, per: i32 },
    /// "Choose a creature type" announced while casting this spell, before its targets (Aphetto Dredging): stored as the spell's chosen type (`Filter.chosen_type` in its target filters).
    ChooseCreatureTypeOnCast,
    /// "You can't spend mana to cast this spell" (Hogaak, Arisen Necropolis): the whole cost must be paid with convoke / improvise / delve and other non-mana costs.
    CantSpendManaToCast,
    /// "...for each <thing> you controlled as you cast this spell": `amount` is evaluated (source = this card, controller = the caster) as the spell is cast and
    /// stored as the card's chosen number (read with `Amount::ChosenNumber`; Lethal Exploit).
    NoteOnCast { amount: Amount },
    /// While this card is in a graveyard, name-based counting treats it as a card with this name (Pardic Firecat: "Flame Burst").
    GraveyardAlias { name: String },
    /// Sneak {cost}: cast it during the declare blockers step by returning an unblocked attacker you control to its owner's hand; the creature enters tapped and attacking. Expanded into an `AlternativeCost` (flash, `Condition::CastWithAlt`) + an enters trigger.
    Sneak { cost: String },
    /// Emerge {cost}: cast it by sacrificing a creature and paying `cost` reduced (generic part) by that creature's mana value. Expanded into an `AlternativeCost`.
    Emerge { cost: String },
    /// "<Subtype> offering": cast it at instant speed by sacrificing a creature of that subtype and paying the difference in mana costs. Expanded into an `AlternativeCost`.
    Offering { subtype: String },
    /// Madness {cost}: when this is discarded, its owner may pay `cost` to cast it (approximated: it stays in the graveyard and is cast from there). Expanded into a triggered ability.
    Madness { cost: String },
    /// "Enchant player" (Curses): the spell targets a player and the permanent enters attached to that player (`PlayerFilter::Enchanted`, `Sel::EnchantedPlayer`).
    EnchantPlayer,
    /// "As this permanent transforms into <this face>, attach it to a player" (Curse of Leeches): put on the Aura face next to `EnchantPlayer`. When the permanent transforms into this face
    /// its controller chooses the player it is attached to (opponents are offered first) before any player can act; it is not asked when the Aura is cast.
    AttachPlayerOnTransform,
    /// "Enchant opponent" (Psychic Possession, Tenuous Truce): like EnchantPlayer, but only an opponent can be enchanted.
    EnchantOpponent,
    /// Unearth {cost}: from the graveyard (sorcery speed) return this to the battlefield with haste; exile it at the next end step or if it would leave.
    Unearth { cost: String },
    /// Morph / Megamorph / Disguise {cost}: this may be cast face down as a 2/2 for {3}, and turned face up for `cost`.
    Morph {
        cost: String,
        #[serde(default)]
        megamorph: bool,
        #[serde(default)]
        disguise: bool,
        /// "As this creature is turned face up, put N +1/+1 counters on it" (Hooded Hydra).
        #[serde(default, skip_serializing_if = "is_zero_u8")]
        face_up_counters: u8,
        /// Extra turn-face-up costs that are not mana ("Morph—Discard a card", "Morph—Reveal a green card in your hand"). With these and an
        /// empty `cost` string, the card is turned face up for only these costs.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        other_cost: Vec<Cost>,
    },
    /// Renown N: when this deals combat damage to a player, if it isn't renowned, put N +1/+1 counters on it and it becomes renowned.
    Renown { n: u8 },
    /// Saga chapters ("I", "II, III" ...): the saga enters with a lore counter, gets another at the beginning of your precombat main
    /// phase, each chapter ability triggers as the counters reach its number, and it is sacrificed after the last chapter resolves.
    Saga { chapters: Vec<SagaChapter> },
    /// Kicker / multikicker: an optional additional cost. `Condition::Kicked` / `Amount::KickCount` read the result.
    Kicker {
        cost: Vec<Cost>,
        #[serde(default, skip_serializing_if = "is_false")]
        multi: bool,
        /// This kicker is a buyback cost ("buyback costs cost {2} less": `StaticEffect::BuybackCostChange`).
        #[serde(default, skip_serializing_if = "is_false")]
        buyback: bool,
    },
    /// Splice onto `onto`: while you cast a matching spell you may reveal this card from your hand and pay `cost`; the spell gets this card's spell effects
    /// (and targets) added after its own. The card stays in your hand. Give the card a normal `Spell` ability for its text.
    Splice {
        onto: FilterBox,
        cost: Vec<Cost>,
    },
    /// Strive: the spell costs `cost` (a mana string such as "{2}{W}") more for each target beyond the first.
    Strive { cost: String },
    /// "This spell costs N life more to cast for each target" (Phyrexian Purge): the life is paid with the other costs once the targets are chosen, and the number of targets is capped by the life you can pay.
    LifePerTarget { n: i32 },
    /// Entwine {cost}: on a Modal spell, an optional additional cost that makes the spell choose ALL its modes (executed in printed order).
    /// Expanded into a one-time `Kicker` that auto-chooses every mode.
    Entwine { cost: Vec<Cost> },
    /// "Cast this spell only if / during ...": the spell can only be cast while the condition holds.
    CastOnlyIf { condition: Condition },
    /// An additional cost to cast this spell.
    AdditionalCost { cost: Vec<Cost> },
    /// Alternative way to cast (flashback, an alternative cost, ...).
    AlternativeCost {
        cost: Vec<Cost>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<Zone>,
        /// Exile the spell instead of putting it anywhere else when it would leave the stack.
        #[serde(default, skip_serializing_if = "is_false")]
        exile_after: bool,
        /// The alternative cost can only be used while this holds (Blasphemous Edict: thirteen or more creatures).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        condition: Option<Condition>,
        /// Overload and similar: cast this way the spell has no targets and does these effects instead of its normal ones.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        effects: Option<Vec<Effect>>,
        /// Targets for the alternative effects (an overload-style cost that still needs targets: tiered spells).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        targets: Option<Vec<TargetSpec>>,
        /// Cast this way, the spell can be cast at instant speed (sneak: combined with a `condition` on the step; "flash if you pay {2} more").
        #[serde(default, skip_serializing_if = "is_false")]
        flash: bool,
    },
    /// Aura / Equipment attach rule: "Enchant creature" / "Equip {2}".
    Attach { to: FilterBox },
    /// "This Equipment can be attached only to a creature with toughness 4 or greater": when the host stops matching, the Equipment becomes unattached.
    AttachOnlyTo { to: FilterBox },
    /// "Enchant creature card in a graveyard" (Animate Dead, Dance of the Dead, Necromancy): the Aura spell targets a card in a graveyard matching `to`;
    /// as it resolves that card is put onto the battlefield under the Aura's controller and the Aura enters attached to it (it then behaves as an ordinary Aura).
    AttachGraveyard {
        to: FilterBox,
        /// The creature card enters tapped (Dance of the Dead).
        #[serde(default, skip_serializing_if = "is_false")]
        tapped: bool,
        /// The card is NOT returned: the Aura enters the battlefield attached to the card in the graveyard and stays attached while that card remains in a graveyard and matches `to`
        /// (Spellweaver Volute: "Enchant instant card in a graveyard"). `Effect::Attach` can move it to another such card.
        #[serde(default, skip_serializing_if = "is_false")]
        stay: bool,
    },
    /// Deprecated placeholder (never implemented); use `Replace`. Cards using it fail to compile on purpose.
    Replacement {
        event: ReplacedEvent,
        effects: Vec<Effect>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        condition: Option<Condition>,
    },
    /// A static replacement or prevention effect (CR 614/615) that modifies events of the game.
    Replace {
        what: Replace,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        condition: Option<Condition>,
        /// The replacement works while the card is in this zone instead of on the battlefield (`"Graveyard"`: dredge).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<Zone>,
    },
    /// "This enters under the control of an opponent of your choice" (Captive Audience, Pendant of Prosperity): the first living opponent of whoever would control it.
    EntersUnderOpponent,
    /// "This enters tapped" (unless the condition holds when it enters: checklands, fastlands ...).
    EntersTapped {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unless: Option<Condition>,
    },
    /// "As this enters, remove all counters from all permanents" (Thief of Blood): run before `EntersWithCounters`, whose `Amount::CountersRemoved` is the total.
    EntersRemoveAllCounters,
    /// "This spell can't be copied" (Choreographed Sparks): copy effects skip it on the stack.
    CantBeCopied,
    EntersWithCounters {
        kind: CounterKind,
        n: Amount,
        /// Only when this holds as it enters (escape: "if it escaped").
        #[serde(default, skip_serializing_if = "Option::is_none")]
        condition: Option<Condition>,
    },
    /// "As this enters, choose a creature type / color / basic land type": remembered on the permanent for `Filter.chosen_*` and `ManaProduce::ChosenColor`.
    EntersChoose {
        kind: ChooseKind,
        /// Colors that can't be chosen ("a color other than red").
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        except: Vec<Color>,
        /// With `ChooseKind::PayLife`: the most life that can be paid (also limited by the controller's life total).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cap: Option<Box<Amount>>,
    },
    /// Gain-land style: "As this enters, you may reveal a card matching the filter from your hand. If you don't, it enters tapped."
    EntersRevealOrTapped {
        filter: FilterBox,
        /// "... or you control a <filter> permanent": no reveal needed then.
        #[serde(default, skip_serializing_if = "is_false")]
        or_control: bool,
    },
    /// Clone: "You may have this enter as a copy of any permanent matching the filter on the battlefield" (plus exceptions).
    EntersAsCopy {
        filter: FilterBox,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        except: Vec<CopyExcept>,
        /// Where the copied card is: the battlefield (default) or a graveyard ("a creature card in a graveyard": Body Double) or exile (The Master, Formed Anew; `filter` then matches cards there).
        #[serde(default = "battlefield_zone", skip_serializing_if = "is_battlefield_zone")]
        zone: Zone,
        /// The chosen graveyard card is exiled as this enters (Body Double).
        #[serde(default, skip_serializing_if = "is_false")]
        exile_copied: bool,
        /// "You may have this enter tapped as a copy of ...": it enters tapped only if it copied something (Vesuva, Echoing Deeps, Callidus Assassin).
        #[serde(default, skip_serializing_if = "is_false")]
        tapped: bool,
        /// The copy choice is only offered while this holds (Protean Raider: "Raid — if you attacked this turn, you may have this enter as a copy").
        #[serde(default, skip_serializing_if = "Option::is_none")]
        condition: Option<Condition>,
        /// The permanent is a copy only until end of turn, then reverts (Cursed Mirror); the copied permanent's enters triggers still happen.
        #[serde(default, skip_serializing_if = "is_false")]
        until_end_of_turn: bool,
        /// "When you do, tap the copied creature": the copied permanent is tapped and linked to this one (`Filter.linked_with_this`; pair with a `DoesntUntap` static): Wall of Stolen Identity.
        #[serde(default, skip_serializing_if = "is_false")]
        tap_and_link_copied: bool,
        /// With `zone: Graveyard`: choose TWO matching cards (pick them in this order: the one to copy, then the other); both are exiled and this enters with +1/+1 counters equal to the other card's power (The Mimeoplasm). Choosing fewer than two copies nothing.
        #[serde(default, skip_serializing_if = "is_false")]
        exile_two_power_counters: bool,
        /// If it entered as a copy, it gains haste until end of turn (The Fourteenth Doctor).
        #[serde(default, skip_serializing_if = "is_false")]
        haste_until_eot: bool,
        /// It is a copy only until it is turned face down (Vesuvan Shapeshifter): the original card comes back and the "except" abilities end. The choice is offered again whenever it is turned face up.
        #[serde(default, skip_serializing_if = "is_false")]
        until_turned_face_down: bool,
    },
    /// "As this Aura is turned face up, you may attach it to a creature" (Gift of Doom): an as-turned-face-up choice among the permanents matching the filter.
    TurnedFaceUpMayAttach { to: FilterBox },
    /// Leyline: "If this card is in your opening hand, you may begin the game with it on the battlefield" (always taken).
    OpeningHand,
    /// Serum Powder: "Any time you could mulligan and this card is in your hand, you may exile all the cards from your hand, then draw that many cards" (asked before each keep / mulligan decision).
    MulliganExile,
    /// Conspiracy "Before you shuffle your deck to start the game, you may reveal this card from your deck and exile `min` or more cards matching `filter` from your sideboard"
    /// (the sideboard stands for the cards you drafted that aren't in your deck). The exiled cards are "exiled with cards named" this one (`Filter.exiled_with_this` matches them).
    PregameSideboardExile { filter: FilterBox, min: u8 },
    /// Gemstone Caverns: "If this card is in your opening hand and you're not the starting player, you may begin the game with it on the battlefield with a luck counter on it. If you do, exile a card from your hand."
    OpeningHandLuck,
    /// Chancellor: "You may reveal this card from your opening hand. If you do, at the beginning of the first upkeep, <effects>" (always revealed). The effects' source is the revealed card.
    OpeningHandReveal { effects: Vec<Effect>, /// The effects happen at the beginning of your first main phase instead of the first upkeep (Chancellor of the Tangle).
        #[serde(default, skip_serializing_if = "is_false")] main_phase: bool },
    /// Shock land: "As this enters, you may pay N life. If you don't, it enters tapped."
    EntersPayLifeOrTapped { life: i32 },
    /// "If this would enter, <pay cost> instead. If you do, put it onto the battlefield; if you don't, put it into its owner's graveyard" (Mox Diamond: discard a land; Lotus Vale: sacrifice two untapped lands; Heart of Yavimaya, Kjeldoran Outpost). An as-enters choice; unpaid, the permanent goes to the graveyard.
    EntersUnlessCost { cost: Vec<Cost> },
    /// "You may have this enter tapped. If you do, <then>" (Mariposa Military Base): an as-enters choice; `then` runs when it enters tapped.
    EntersMayTapped {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        then: Vec<Effect>,
    },
    /// "As this enters, put a <kind> counter on a permanent you control matching the filter" (Phylactery Lich: an artifact): an as-enters choice, made before any
    /// state trigger can see the permanent (automatic with one candidate, nothing happens with none).
    EntersCounterOnChosen { filter: FilterBox, kind: CounterKind },
    /// "As this enters, exile X cards matching the filter from your graveyard (X = this spell's X). If you can't, put it into its owner's graveyard instead. For each card exiled this way it enters with
    /// your choice of one counter of the listed kinds" (Frankenstein's Monster): you choose the cards (automatic when exactly X match), then a counter kind per card (automatic with one kind).
    EntersExileXCounters { filter: FilterBox, kinds: Vec<CounterKind> },
    /// "As this enters, exile any number of <filter> cards from your graveyard" (Sutured Ghoul): the controller picks any number; the total power and toughness of the exiled cards is read with `Amount::EnterExiledPower` / `EnterExiledToughness`.
    EntersExileAny { filter: FilterBox },
    /// "As this enters, sacrifice all <filter> you control" (Overlaid Terrain): done before the permanent can be responded to; the filter is relative to the controller.
    EntersSacrificeAll { filter: FilterBox },
    /// "As this enters, choose a permanent matching the filter": this permanent is linked to it (`Filter.linked_with_this`, which still finds it after this has left: Dauntless Bodyguard).
    /// `optional`: "you may choose" (Scheming Fence): the player may pick nothing and then nothing is linked.
    EntersLinkChosen { filter: FilterBox, #[serde(default, skip_serializing_if = "is_false")] optional: bool },
    Ward { cost: Vec<Cost> },
    Protection { from: Vec<ProtectionFrom> },
    /// "Hexproof from white": can't be the target of spells or abilities from matching sources that opponents control.
    HexproofFrom { from: Vec<ProtectionFrom> },
}

/// How long a play-from-exile permission lasts.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export)]
pub enum PlayWindow {
    /// Until the end of this turn.
    ThisTurn,
    /// Until the end of the controller's next turn.
    EndOfYourNextTurn,
    /// Until the beginning of the controller's next upkeep (through the turn before their next turn).
    UntilYourNextUpkeep,
    /// Until the beginning of the controller's next end step (this turn's if it has not begun yet): Dragonhawk, Fate's Tempest.
    UntilYourNextEndStep,
    /// Until the beginning of the next end step of the player who gets the permission (`by`), not the effect's controller (Casey & Raph, Hotheads).
    UntilGranteesNextEndStep,
    /// Until the end of combat on the controller's next turn (Brazen Cannonade).
    UntilYourNextCombatEnd,
    /// For as long as it stays in exile.
    Forever,
    /// For as long as the effect's controller still controls the source permanent ("you may play it for as long as you control this creature": Gwen Stacy, Intet, Taster of Wares).
    WhileSourceControlled,
    /// Until the source exiles another card (with this same ability): granting it to new cards revokes the permission this source gave earlier (Unstable Amulet).
    UntilSourceExilesAnother,
}

fn zone_hand() -> Zone {
    Zone::Hand
}
fn is_zone_hand(z: &Zone) -> bool {
    *z == Zone::Hand
}

fn one_i32() -> i32 {
    1
}

/// Events a replacement effect can modify. `multiply` is applied first, then `add`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum Replace {
    /// "If you would gain life, you gain twice that much instead" (multiply 2) / "gain that much +1" (add 1).
    GainLife {
        who: PlayerFilter,
        #[serde(default = "one_i32")]
        multiply: i32,
        #[serde(default)]
        add: i32,
        /// Do these effects INSTEAD of gaining life (put on the stack after the current effect): `Amount::EventAmount` = the life that would have been gained,
        /// `Sel::TriggerPlayer` = the player who would gain it ("draw that many cards instead": Nefarious Lich).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        instead: Vec<Effect>,
    },
    LoseLife { who: PlayerFilter, #[serde(default = "one_i32")] multiply: i32, #[serde(default)] add: i32 },
    /// "If you would learn, you may instead ..." (Retriever Phoenix, from the graveyard): `Effect::Learn` offers these `instead` effects to the controller (source = the replacing card);
    /// declining learns normally.
    Learn {
        #[serde(default)]
        instead: Vec<Effect>,
    },
    /// "If a Rigger you control would assemble a Contraption, it assembles two Contraptions instead" (Steamflogger Boss): `Effect::Assemble` by a source matching `filter` (relative to the replacing permanent's controller) assembles `multiply` times as many.
    Assemble { filter: FilterBox, multiply: i32 },
    /// "If you would draw a card, draw two cards instead" (multiply 2).
    Draw {
        who: PlayerFilter,
        /// Leave the first card drawn in each of the player's draw steps alone.
        #[serde(default)]
        except_first_in_draw_step: bool,
        #[serde(default = "one_i32")]
        multiply: i32,
        #[serde(default)]
        add: i32,
        /// Do these effects INSTEAD of drawing (`Sel::TriggerPlayer` = the player who would draw). They run (on the stack) right after the current effect finishes.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        instead: Vec<Effect>,
        /// With `instead`: the controller may decline (then the player draws normally).
        #[serde(default, skip_serializing_if = "is_false")]
        optional: bool,
        /// Used up the first time it applies (only meaningful in `AddReplacement`).
        #[serde(default, skip_serializing_if = "is_false")]
        once: bool,
        /// With `instead`: only applies when ONE draw effect asks for at least this many cards (Alms Collector: 2); the whole draw is replaced.
        #[serde(default, skip_serializing_if = "is_zero_i32")]
        min_cards: i32,
        /// Only applies when the player has already drawn a card this turn ("can't draw more than one card each turn": `multiply: 0`).
        #[serde(default, skip_serializing_if = "is_false")]
        after_first: bool,
        /// Only applies to the player's regular draw-step draw ("if you would draw a card during your draw step": Island Sanctuary); with `instead`.
        #[serde(default, skip_serializing_if = "is_false")]
        draw_step_only: bool,
        /// With `Effect::AddReplacement`: the ability's target slot this index picks is carried to the `instead` effects as their `Sel::Target{idx:0}` (Words of War).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bind_then_target: Option<u8>,
        #[serde(skip)]
        bound_then_target: Option<crate::state::Target>,
        /// Only applies to a draw caused by the cycling ability of ANOTHER nonland card (Unpredictable Cyclone); with `instead`, `Sel::TriggerObject` is the cycled card.
        #[serde(default, skip_serializing_if = "is_false")]
        cycling_other_nonland: bool,
    },
    /// Tokens created by `who` (Doubling Season, Anointed Procession: multiply 2).
    /// With `types` / `subtypes` it only applies to tokens that have one of those card types and all of those subtypes ("artifact tokens", "Food tokens", "creature tokens").
    /// `instead_token` creates that token card instead; `extra_token` creates additional tokens of that card (`extra_per_token`: as many as the tokens being created, else one per event).
    /// Cards `who` would mill: n * multiply + add instead (The Water Crystal: add 4; Bruvac: multiply 2). Applied once, never re-triggers itself.
    Mill {
        who: PlayerFilter,
        #[serde(default = "one_i32")]
        multiply: i32,
        #[serde(default)]
        add: i32,
    },
    /// Cards `who` would scry (not surveil): n + add instead when n > 0 (Kenessos, Priest of Thassa: add 1).
    Scry {
        who: PlayerFilter,
        #[serde(default)]
        add: i32,
        /// "If you would scry a number of cards, draw that many cards instead" (Eligeth, Crossroads Augur).
        #[serde(default, skip_serializing_if = "is_false")]
        draw_instead: bool,
    },
    CreateTokens {
        who: PlayerFilter,
        #[serde(default = "one_i32")]
        multiply: i32,
        #[serde(default)]
        add: i32,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        types: Vec<CardType>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        subtypes: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        instead_token: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        extra_token: Option<String>,
        #[serde(default, skip_serializing_if = "is_false")]
        extra_per_token: bool,
        /// Several additional token cards (Academy Manufactor: "instead create one of each"); each is created like `extra_token`.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        extra_tokens: Vec<String>,
        /// The tokens are created under the replacement's controller instead (Crafty Cutpurse, with `who: Opponent` and `AddReplacement`).
        #[serde(default, skip_serializing_if = "is_false")]
        control_instead: bool,
        /// Mirrormind Crown: the first time each turn `who` would create tokens, they may instead create that many copies of the creature this source is attached to (`Card.token_repl_turn`; asked in `Effect::CreateToken`).
        #[serde(default, skip_serializing_if = "is_false")]
        copy_equipped: bool,
        /// Esix, Fractal Bloom: the first time each of the controller's turns `who` would create tokens, they may instead choose a creature other than the source and create that many copies of it (`Card.token_repl_turn`).
        #[serde(default, skip_serializing_if = "is_false")]
        copy_chosen: bool,
        /// Jinnie Fay: whenever `who` would create tokens they may instead create that many of ONE of these token cards (the player picks, or declines; asked in `Effect::CreateToken`).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        instead_choice: Vec<String>,
    },
    /// Counters put on permanents matching `on` that `who` controls (Hardened Scales: add 1, kind +1/+1).
    AddCounters {
        who: PlayerFilter,
        #[serde(default)]
        on: FilterBox,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<CounterKind>,
        #[serde(default = "one_i32")]
        multiply: i32,
        #[serde(default)]
        add: i32,
        /// Also applies to counters put on players (Winding Constrictor: "...or on you"; Solemnity with `prevent`-style multiply 0): `who` is then the player receiving them.
        #[serde(default, skip_serializing_if = "is_false")]
        players: bool,
        /// Only counters put by a player matching this filter (relative to the replacement's controller): Vorinclex "if an opponent would put counters".
        #[serde(default, skip_serializing_if = "Option::is_none")]
        by: Option<PlayerFilter>,
        /// Half as many, rounded down (applied after `multiply` / `add`).
        #[serde(default, skip_serializing_if = "is_false")]
        halve: bool,
        /// Only counters put by a modular trigger (`Effect::ModularCounters`): Zabaz, the Glimmerwasp.
        #[serde(default, skip_serializing_if = "is_false")]
        modular: bool,
    },
    /// Damage dealt by sources matching `source` to `to` (multiply 2 doubles it; prevent stops it entirely).
    Damage {
        source: FilterBox,
        #[serde(default)]
        to: DamageTo,
        #[serde(default = "one_i32")]
        multiply: i32,
        #[serde(default)]
        add: i32,
        #[serde(default)]
        prevent: bool,
        /// Some(true): only combat damage; Some(false): only noncombat damage.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        combat: Option<bool>,
        /// Only damage caused by a spell or ability that targets the recipient (Silhouette).
        #[serde(default, skip_serializing_if = "is_false")]
        cause_targets_recipient: bool,
        /// Only damage dealt to the permanent that has this replacement effect.
        #[serde(default, skip_serializing_if = "is_false")]
        to_this: bool,
        /// Only applies to damage of at most / at least this amount ("damage of 3 or less", "7 or more").
        #[serde(default, skip_serializing_if = "Option::is_none")]
        at_most: Option<i32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        at_least: Option<i32>,
        /// Only damage dealt to a PERMANENT matching this filter (relative to the replacement's controller / source): "creatures you control",
        /// "a permanent an opponent controls", "enchanted creature" (`attached_by_source`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to_filter: Option<FilterBox>,
        /// With `to_filter`: the replacement also covers damage to its controller ("you and/or creatures you control").
        #[serde(default, skip_serializing_if = "is_false")]
        to_filter_or_you: bool,
        /// The damage becomes exactly this much ("it deals 3 damage instead").
        #[serde(default, skip_serializing_if = "Option::is_none")]
        set_to: Option<i32>,
        /// Effects that happen after the damage was changed (put on the stack): `Amount::EventAmount` = the amount that was prevented/removed,
        /// `Sel::TriggerSource` = the damage source ("prevent that damage; put that many counters ...", "mill that many").
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        then: Vec<Effect>,
        /// `then` also runs when the damage is not changed (`EventAmount` = the damage dealt): "that source deals that much damage to you and this deals that much to its controller" (Eye for an Eye).
        #[serde(default, skip_serializing_if = "is_false")]
        then_always: bool,
        /// The effect replaces the damage without being prevention ("put that many counters on it instead": Phytohydra): it applies even when the damage can't be prevented.
        #[serde(default, skip_serializing_if = "is_false")]
        instead: bool,
        /// `then` runs once for all damage dealt to this at the same time (Phantom Centaur: simultaneous blockers remove one counter).
        #[serde(default, skip_serializing_if = "is_false")]
        once_per_batch: bool,
        /// With `Effect::AddReplacement`: the replacement is used up the first time it applies ("the next time ... this turn").
        #[serde(default, skip_serializing_if = "is_false")]
        once: bool,
        /// With `Effect::AddReplacement`: only damage dealt BY the creature this selector picks when the effect resolves (a target).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bind_source: Option<Sel>,
        /// With `Effect::AddReplacement`: only damage dealt TO the permanent this selector picks when the effect resolves ("target creature").
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bind_to: Option<Sel>,
        /// A shield: prevents (at most) this much damage in total, then is used up ("prevent the next 3 damage").
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prevent_up_to: Option<i32>,
        /// `prevent_up_to` computed when `Effect::AddReplacement` resolves ("prevent the next X damage, X = your devotion").
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prevent_up_to_amount: Option<Amount>,
        /// The damage becomes this fraction of itself, rounded down ("halve that damage, rounded down" = 2).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        divide_by: Option<i32>,
        /// Prevents this fraction of the damage, rounded down ("prevent half of that damage, rounded down" = 2); the rest is dealt.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prevent_fraction: Option<i32>,
        /// Damage smaller than this computed amount becomes exactly that amount instead; larger damage is left alone (Ojer Axonil: "less than Ojer's power ... deals damage equal to Ojer's power instead").
        #[serde(default, skip_serializing_if = "Option::is_none")]
        raise_to_amount: Option<Amount>,
        /// Adds this computed amount to the damage ("plus X"); evaluated for the replacement's source and controller.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        add_amount: Option<Amount>,
        /// Prevents this computed amount of the damage ("prevent X of that damage"); the rest is dealt.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prevent_amount: Option<Amount>,
        /// For damage to a player: it can't take the player below this life total (Worship: 1).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        leave_at_least: Option<i32>,
        /// The replacement only applies while this holds (evaluated for the replacement's source and controller).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        condition: Option<Condition>,
        #[serde(skip)]
        bound_source: Option<crate::ids::CardId>,
        #[serde(skip)]
        bound_to: Option<crate::ids::CardId>,
        /// Only damage dealt to a player or to a permanent that player controls: the player is the one this selector picks when an `AddReplacement` resolves, else the source's chosen player (Lightning, Army of One; Sawhorn Nemesis).
        #[serde(default, skip_serializing_if = "is_false")]
        to_chosen_player_or_permanents: bool,
        /// Only damage dealt by a source controlled by one of the source's two chosen players (`chosen_player`, `chosen_player2`) to the other one or a permanent the other controls (Bitter Feud).
        #[serde(default, skip_serializing_if = "is_false")]
        between_chosen_players: bool,
        /// With `Effect::AddReplacement` and `to_chosen_player_or_permanents`: the player to bind.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bind_player: Option<Sel>,
        #[serde(skip)]
        bound_player: Option<crate::ids::PlayerId>,
        /// With `Effect::AddReplacement`: only damage dealt TO the player this selector picks (use with `bind_to` for "any target": the permanent or the player).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bind_to_player: Option<Sel>,
        #[serde(skip)]
        bound_to_player: Option<crate::ids::PlayerId>,
        /// With `Effect::AddReplacement`: only damage from sources of the colour the source's controller chose (`ChooseColor` earlier in the same effect list), snapshotted when the effect resolves ("sources of the color of your choice").
        #[serde(default, skip_serializing_if = "is_false")]
        bind_chosen_color: bool,
        #[serde(skip)]
        bound_color: Option<Color>,
        /// With `Effect::AddReplacement`: the spell's target slot this index picks is carried to the `then` effects as their `Sel::Target{idx:0}` ("deals that much damage to any target": Divine Deflection).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bind_then_target: Option<u8>,
        #[serde(skip)]
        bound_then_target: Option<crate::state::Target>,
        /// The damage is dealt to this object instead (selector evaluated for the replacement's source and controller; Martyrs of Korlis, Ancient Adamantoise): the new recipient is still hit by replacements and keeps the damage's source and combat-ness.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        redirect_to: Option<Sel>,
        /// With `redirect_to`: the controller MAY have the damage dealt to the new recipient instead (Blood of the Martyr). The damage is held back and a follow-up
        /// asks; when declined it is dealt as originally planned (a non-redirecting second pass; damage is no longer simultaneous with the event).
        #[serde(default, skip_serializing_if = "is_false")]
        redirect_optional: bool,
    },
    /// A card that would go from `from` (any zone if omitted) to `to` goes to `instead` (Rest in Peace).
    ZoneChange {
        #[serde(default)]
        filter: FilterBox,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<Zone>,
        to: Zone,
        instead: Zone,
        /// Whose cards (their owner relative to the source's controller).
        #[serde(default)]
        whose: PlayerFilter,
        /// A library destination is the bottom of the library ("put it on the bottom of its owner's library instead").
        #[serde(default, skip_serializing_if = "is_false")]
        bottom: bool,
        /// Effects that go on the stack after the card moved ("exile it instead. When you do ..."). `Sel::TriggerObject` / `TriggerSource` = the card that moved; `TriggerPlayer` = its owner.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        then: Vec<Effect>,
        /// Only when a spell or ability controlled by a player other than the card's owner causes the move ("if a spell or ability an opponent controls causes you to discard this card").
        #[serde(default, skip_serializing_if = "is_false")]
        cause_opponent: bool,
        /// Only when a spell or ability (an effect, not a cost or the cleanup step) causes the move ("if an effect causes you to discard a card": Library of Leng).
        #[serde(default, skip_serializing_if = "is_false")]
        cause_effect: bool,
        /// "You may ... instead": the owner is asked after the move (at the next priority) and the card is moved on from `to` to `instead` when they accept.
        #[serde(default, skip_serializing_if = "is_false")]
        optional: bool,
        /// With `Effect::AddReplacement`: only the one card this selector picks when the effect resolves ("that spell", "target creature").
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bind_to: Option<Sel>,
        /// With `Effect::AddReplacement`: used up the first time it applies.
        #[serde(default, skip_serializing_if = "is_false")]
        once: bool,
        /// Only when a spell is put into its destination as it resolves (not when countered or fizzled): "if that spell would be put into a graveyard as it resolves".
        #[serde(default, skip_serializing_if = "is_false")]
        on_resolve: bool,
        #[serde(skip)]
        bound_to: Option<crate::ids::CardId>,
        /// With `Effect::AddReplacement`: only cards dealt damage this turn by the one card this selector picks when the effect resolves ("a creature dealt damage by the targeted creature": Runesword).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bind_damager: Option<Box<Sel>>,
        #[serde(skip)]
        bound_damager: Option<crate::ids::CardId>,
        /// Not when the card is being cycled (the discard of a cycling cost): Abandoned Sarcophagus "and it wasn't cycled".
        #[serde(default, skip_serializing_if = "is_false")]
        not_cycled: bool,
        /// Not when the card comes from the battlefield ("permanent, spell, or card not on the battlefield": Sanctifier en-Vec).
        #[serde(default, skip_serializing_if = "is_false")]
        not_from_battlefield: bool,
    },
    /// "If this would be destroyed, regenerate it" (Mossbridge Troll, Knight of the Holy Nimbus, enchanted land of Crackling Emergence): matching permanents regenerate instead of being destroyed.
    /// Respects "can't be regenerated". Not triggered by sacrifice or lethal-damage state-based destruction that can't be regenerated.
    DestroyRegenerate { filter: FilterBox },
    /// "If enchanted land would be destroyed, instead sacrifice this Aura and that land gains indestructible until end of turn" (Harmonious Emergence): a matching
    /// permanent that would be destroyed (not one that is indestructible, and not sacrificed) is not destroyed; the replacement's source is sacrificed and the permanent gains indestructible until end of turn.
    DestroyInsteadSacSource { filter: FilterBox },
    /// "If a matching permanent would be destroyed, instead remove all +1/+1 counters from it and all damage dealt to it is healed" (Dr. McCoy, Surly Surgeon).
    DestroyInsteadRemoveCounters { filter: FilterBox },
    /// Matching permanents enter tapped.
    EntersTapped { filter: FilterBox },
    /// "If a creature you control would explore, instead it explores, then it explores again" (Topography Tracker): matching creatures explore `extra` more times.
    /// `scry`: the controller first scries that many (Twists and Turns: "instead you scry 1, then that creature explores").
    Explore { filter: FilterBox, extra: u8, #[serde(default, skip_serializing_if = "is_zero_u8")] scry: u8 },
    /// "If a creature you control would connive, instead you draw `draw` card(s), then that creature connives" (Leader, Super-Genius).
    Connive { filter: FilterBox, draw: u8 },
    /// "As a matching permanent enters, it becomes a P/T <subtype> creature in addition to its other types" (Displaced Dinosaurs: historic permanents become 7/7 Dinosaur creatures).
    EntersAsCreature { filter: FilterBox, subtype: String, power: i32, toughness: i32 },
    /// Matching permanents enter untapped (Horizon Explorer, Archelos): overrides every other enters-tapped effect.
    EntersUntapped { filter: FilterBox },
    /// Matching permanents that would enter under an opponent's control (of this ability's controller) enter under this ability's controller's control instead (Gather Specimens; use with `AddReplacement`).
    EntersControlled { filter: FilterBox },
    /// Matching permanents enter with extra counters.
    /// `once`: used up the first time it applies (only meaningful in `AddReplacement`: "the next creature spell you cast this turn enters with ...").
    EntersWithCounters {
        filter: FilterBox,
        kind: CounterKind,
        n: Amount,
        #[serde(default, skip_serializing_if = "is_false")]
        once: bool,
    },
    /// "If you would lose the game, instead ...": the controller of the replacement doesn't lose; the listed changes happen at once
    /// (Exquisite Archangel, The Golden Throne, Lich's Mirror, Stunning Reversal via `AddReplacement`, which is used up when it applies).
    LoseGame {
        /// Exile the source of this replacement.
        #[serde(default, skip_serializing_if = "is_false")]
        exile_source: bool,
        /// Shuffle your hand, graveyard and all permanents you own into your library.
        #[serde(default, skip_serializing_if = "is_false")]
        shuffle_all: bool,
        /// Then draw this many cards.
        #[serde(default, skip_serializing_if = "is_zero_u8")]
        draw: u8,
        /// Your life total becomes this (`starting_life`: the format's starting life total instead).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        life: Option<i32>,
        #[serde(default, skip_serializing_if = "is_false")]
        starting_life: bool,
    },
    /// Matching permanents enter the battlefield as a copy of the permanent `of` selects (Essence of the Wild: "creatures you control enter as a copy of this creature"; Infinite Reflection: the enchanted creature).
    /// With `Effect::AddReplacement`: `bind` evaluates `of` when the effect resolves (the chosen target) and keeps that card; `once` uses the replacement up the first time it applies
    /// ("the next time one or more creatures or planeswalkers enter this turn, they enter as copies of the chosen creature": Mystic Reflection).
    EntersAsCopyOf {
        filter: FilterBox,
        of: Sel,
        #[serde(default, skip_serializing_if = "is_false")]
        bind: bool,
        #[serde(default, skip_serializing_if = "is_false")]
        once: bool,
        #[serde(skip)]
        bound_of: Option<crate::ids::CardId>,
    },
    /// Triggered abilities of permanents you control that trigger because of a matching event trigger `additional` extra times (Panharmonicon).
    TriggerExtra {
        /// The permanent entering / dying that caused the trigger.
        cause: FilterBox,
        event: TriggerCause,
        #[serde(default = "one_i32")]
        additional: i32,
        /// Only triggered abilities of the permanent with this replacement or of an emblem its controller owns count (The Masamune's granted ability).
        #[serde(default, skip_serializing_if = "is_false")]
        own_or_emblem: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum TriggerCause {
    EntersBattlefield,
    Dies,
    LeavesBattlefield,
    /// The attacking creature caused the trigger (Wulfgar, Isshin).
    Attacks,
    /// The permanent turned face up caused the trigger (Panoptic Projektor).
    TurnedFaceUp,
    /// A player drawing a card caused the trigger; `cause` filters the permanent whose ability triggers (Krang).
    Draws,
    /// The trigger controller gaining life caused the trigger; `cause` filters the permanent whose ability triggers (Dr. Beverly Crusher).
    GainsLife,
    /// A creature dealing combat damage to a player caused the trigger; `cause` filters that creature (Felix Five-Boots).
    CombatDamageToPlayer,
    /// A room being entered caused the trigger; `cause` filters the dungeon card (Dungeon Delver: a dungeon you own).
    EntersRoom,
    /// The trigger controller casting or copying an instant or sorcery spell caused the trigger; `cause` filters the permanent whose ability triggers (Veyran, Voice of Duality).
    CastOrCopyInstantSorcery,
    /// ANY triggered ability of a permanent matching `cause` (the permanent whose ability triggers is the cause).
    AnyTrigger,
    /// A creature being dealt damage caused the trigger; `cause` filters the damaged creature (Wayta, Trainer Prodigy).
    CreatureDealtDamage,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DieOutcome {
    pub min: i32,
    pub max: i32,
    pub effects: Vec<Effect>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Mode {
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub targets: Vec<TargetSpec>,
    pub effects: Vec<Effect>,
    /// Spree: an additional cost paid when this mode is chosen.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cost: Vec<Cost>,
    /// Pawprints of this mode when the `ChooseModes` has a `budget` ("Choose up to five {P} worth of modes").
    #[serde(default, skip_serializing_if = "is_zero_u8")]
    pub paw: u8,
}

fn is_zero_u8(n: &u8) -> bool {
    *n == 0
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Timing {
    /// Any time the player has priority.
    #[default]
    Instant,
    /// Main phase, empty stack, your turn.
    Sorcery,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum ProtectionFrom {
    Color { c: Color },
    Type { t: CardType },
    Everything,
    /// Sources that aren't spells: activated and triggered abilities (hexproof from activated and triggered abilities, Volatile Stormdrake).
    Abilities,
    Colorless,
    /// Sources that are NOT this color (colorless included): hexproof from nongreen sources (Gaea's Revenge).
    NotColor { c: Color },
    /// Creature sources that are NOT of this creature type ("protection from non-Human creatures": Spare from Evil).
    NonSubtype { name: String },
    Monocolored,
    Multicolored,
    /// Protection from a creature type ("protection from Wizards").
    Subtype { name: String },
    /// Protection from the player chosen "as this enters" (`EntersChoose` kind `Opponent` / `ChoosePlayer`): sources that player controls (True-Name Nemesis).
    ChosenPlayer,
    /// Protection from each opponent of the protected object's controller: sources controlled by anyone else (Cliffside Rescuer, Absolute Virtue).
    Opponents,
    /// Protection from the colour chosen "as this enters" (Pentarch Ward, Iron Maiden-style chosen colours): sources of that colour.
    ChosenColor,
    /// Protection from the creature type / card type chosen "as this enters" (Riders of Gavony, Serra's Emissary): sources of that subtype or card type.
    ChosenType,
    /// Protection from sources matching a filter (mana value 3 or less, a supertype, an odd/even mana value, ...); the filter is read with the protected permanent as its source.
    Matching { filter: Box<Filter> },
    /// Sources whose mana value equals the mana value of an artifact the protected permanent's controller controls ("protection from each mana value among artifacts you control": Rebbec, Architect of Ascension).
    ArtifactManaValues,
    /// Protection from everything controlled by one player: `who` selects that player when `GrantProtection` resolves (Eon Frolicker: the targeted opponent); it is then stored as `p`.
    ControlledBy {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        p: Option<u8>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        who: Option<Box<Sel>>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum ReplacedEvent {
    Dies,
    Damage,
    Draw,
    EntersBattlefield,
    GainLife,
    Destroy,
    LeavesBattlefield,
}

// ---------------------------------------------------------------- costs

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum Cost {
    Mana { cost: String },
    Tap,
    /// Planeswalker "-X": remove X loyalty counters (X chosen when activating).
    RemoveLoyaltyX,
    /// Remove X counters of `kind` from this permanent (X is chosen as the cost is paid; `Amount::X` in the effects).
    RemoveCountersX { kind: CounterKind },
    /// "Remove X counters from a permanent you control that matches `filter`" (Moxite Refinery): X is announced first, then one matching permanent holding at least X counters in total is chosen and X counters (most plentiful kind first) come off it. The chosen permanent is `Remembered`.
    RemoveXCountersFromOne { filter: FilterBox },
    /// "Add {R}" as a cost (Braid of Fire's cumulative upkeep): adds `n` mana of `color` to your pool; scaled by cumulative upkeep.
    AddMana { color: Color, n: i32 },
    /// Roll a die with `sides` sides as a cost ("{6}, Roll a d8"); the result is the ability's `Amount::X` (Clay Golem).
    RollDie { sides: i32 },
    /// Tap any number of untapped permanents you control matching `filter` whose total power is at least `power` (Teamwork N, Crew-like: chosen as the cost is paid).
    TapTotalPower { filter: FilterBox, power: i32 },
    /// Sacrifice any number of permanents you control matching `filter` whose total power is at least `power` (Phyrexian Dreadnought's "unless you sacrifice ..."; chosen as the cost is paid).
    SacrificeTotalPower { filter: FilterBox, power: i32 },
    /// Tap `n` untapped creatures matching `filter` that all share a creature type (Weight of Conscience).
    TapShareType { filter: FilterBox, n: i32 },
    /// Discard `n` cards from hand matching `filter` that all have the same name (Sphinx of the Chimes).
    DiscardSameName { filter: FilterBox, n: i32 },
    /// Discard `n` cards from hand matching `filter` that all have different names (Ormos, Archive Keeper).
    DiscardDistinctNames { filter: FilterBox, n: i32 },
    /// Reveal `n` cards from your hand that share a color (Illuminated Folio); nothing moves.
    RevealShareColor { n: i32 },
    /// Generic mana sized by an Amount evaluated when the cost is paid ("unless its controller pays {X} where X = the cards in your hand", Phantasmal Sphere, Thassa's Rebuff). In an activated ability it is added to the cost once the targets are chosen, so it may read them (Reflecting Mirror: twice the targeted spell's mana value).
    ManaAmount { amount: Amount },
    /// Like `ManaAmount` but paid in repeated `symbol` (e.g. "{G}"): Cyclone's "{G} for each wind counter". Used in `Unless`.
    ColoredManaAmount { symbol: String, amount: Amount },
    /// Pay the mana cost printed on the ability's source ("unless you pay its mana cost": Pendrell Flux). Only inside `Unless`.
    SourceManaCost,
    /// Pay the mana cost printed on the selected card ("unless they pay that creature's mana cost": Tariff). Only inside `Unless`.
    /// `reduce_generic`: the generic part is reduced by this much ("its mana cost reduced by {2}": Flash).
    ManaCostOf { of: Sel, #[serde(default, skip_serializing_if = "is_zero_u8")] reduce_generic: u8 },
    /// Pay the printed mana cost of the permanent this one is attached to ("Pay enchanted creature's mana cost": Merseine). Activated abilities only.
    AttachedManaCost,
    /// Put `n` counters of `kind` on a permanent you control matching `filter`, chosen as the cost is paid (Blight N: `-1/-1` counters on a creature you control).
    /// `opponent`: the permanent is one an opponent controls instead (Sheltering Ancient's cumulative upkeep).
    PutCountersOnChosen { filter: FilterBox, kind: CounterKind, n: i32, #[serde(default, skip_serializing_if = "is_false")] opponent: bool },
    /// Gain control (for good) of a permanent matching `filter` that you don't control, chosen as the cost is paid (Herald of Leshrac's cumulative upkeep: a land).
    GainControlOfChosen { filter: FilterBox },
    /// Remove ALL counters from this permanent (of `kind`, or of every kind when absent); `Amount::X` in the effects is the number removed (Molten Hydra, Jar of Eyeballs).
    RemoveAllCounters {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<CounterKind>,
    },
    /// "Discard X cards" / "Sacrifice X creatures" / "Pay X life" ...: the inner cost is paid X times, X being the spell's / ability's X
    /// (announced together with any {X} in the mana cost; chosen from 0 up to what can be paid when there is no {X} in the mana cost).
    XTimes { cost: Box<Cost> },
    /// Marker (costs nothing): the X chosen for this ability must be at least `n` ("X can't be 0"). Put it next to the X cost.
    XMinimum { n: u8 },
    /// Marker (costs nothing), only in `AlternativeManaCost.extra`: casting with this granted alternative cost is a warp cast (the permanent is exiled at the next end step and may be cast from exile later: Tannuk, Steadfast Second).
    Warp,
    /// Marker (costs nothing), only in `AlternativeManaCost.extra`: a blitz cast. The cost is reduced by {1} for each time its caster cast their commander from the command zone this game, the permanent gains haste and "When this creature dies, draw a card", and it is sacrificed at the next end step (Henzie "Toolbox" Torre).
    Blitz,
    /// Marker (costs nothing), only in `AlternativeManaCost.extra`: an evoke cast; the permanent is sacrificed when it enters (Ashling, the Limitless).
    Evoke,
    /// Marker (costs nothing), only in `AlternativeManaCost.extra`: a spell cast with this granted alternative cost may be cast as though it had flash (Primal Prayers).
    AltFlash,
    /// Marker (costs nothing), only in `AlternativeManaCost.extra`: a sneak cast; the permanent enters tapped and attacking (Ninja Teen: creature cards in your graveyard have sneak).
    SneakEntry,
    /// Collect evidence N: exile cards from your graveyard with total mana value N or more (the cards are chosen automatically, biggest first).
    /// Use it as an `AdditionalCost` or inside a `Kicker` (then `Condition::Kicked` = evidence was collected).
    CollectEvidence { n: i32 },
    /// Collect evidence X, where X is the total mana value of the permanents the spell targets (Urgent Necropsy): paid after the targets are chosen; the spell can't be cast when the graveyard can't cover X.
    CollectEvidenceTargets,
    /// Exile any number of `filter` cards from your graveyard with `n` or more `color` mana symbols among their mana costs
    /// (chosen automatically, most symbols first; they become Remembered). Baron Helmut Zemo.
    ExileSymbolsFromGraveyard { filter: FilterBox, color: Color, n: i32 },
    /// Crew N: tap any number of untapped creatures you control with total power N or more (chosen automatically, biggest first).
    Crew { power: i32 },
    Untap,
    PayLife { n: i32 },
    /// Pay half your life, rounded up (Temporal Extortion): worked out for the paying player when the cost is paid.
    PayHalfLife,
    /// Pay life equal to a computed amount, evaluated with the cost's source as `This` ("pay life equal to its mana value": `ManaValueOf This`).
    PayLifeX { amount: Amount },
    /// "has this deal N damage to you" (Browbeat): the payer is dealt N damage by the source (always payable).
    TakeDamage { n: i32 },
    /// Like `TakeDamage`, but the damage is an amount worked out when the cost is paid (Combustion Man: his power; Craig Boone: his quest counters); in `Unless` only.
    TakeDamageX { amount: Amount },
    /// Sacrifice this permanent.
    SacrificeSelf,
    /// Exert this creature as a cost: it skips its controller's next untap step (Hope Tender: "{1}, {T}, Exert this creature").
    ExertSelf,
    /// In a granted ability: sacrifice the permanent that granted it ("Sacrifice this Equipment"; `SacrificeSelf` would sacrifice the creature).
    SacrificeGrantSource,
    /// In a granted ability: exile the permanent that granted it (The Dominion Bracelet). Counts as a "sacrifice self" cost for `AbilityCostChange.sac_self_only`.
    ExileGrantSource,
    /// In a granted ability: remove ALL counters (of `kind`, or every kind) from the permanent that granted it; `Amount::X` is the number removed (Hankyu).
    RemoveAllCountersGrantSource {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<CounterKind>,
    },
    Sacrifice {
        filter: FilterBox,
        #[serde(default = "one")]
        n: i32,
    },
    /// Sacrifice `n` permanents matching the filter whose names are pairwise different ("sacrifice three artifacts and/or tokens with different names": Transmutation Font).
    SacrificeDistinctNames { filter: FilterBox, n: i32 },
    /// Sacrifice a number of permanents computed when the cost is paid ("sacrifice half the lands you control, rounded up": `Half` of a `Count`). Additional costs only.
    SacrificeAmount { filter: FilterBox, amount: Amount },
    /// Waterbend {n}: pay `n` generic mana; each of them may instead be paid by tapping an untapped artifact or creature you control (convoke/improvise-like, also for abilities and additional costs).
    Waterbend { n: i32 },
    /// Convoke-like: pay `n` generic mana; each of them may instead be paid by tapping an untapped creature you control (Heirloom Epic: "for each mana in this ability's activation cost, you may tap an untapped creature you control rather than pay that mana").
    TapCreaturesOrMana { n: i32 },
    /// Only in `AlternativeManaCost.extra`: Waterbend {X} where X is the mana value of the spell being cast (Hama, the Bloodbender).
    WaterbendMV,
    /// Emerge: sacrifice a permanent matching the filter; the generic part of the spell's mana cost is reduced by its mana value. Chosen (and paid) before the mana.
    SacrificeReduceByManaValue { filter: FilterBox },
    /// Offering: sacrifice a permanent matching the filter; its mana cost is subtracted from the spell's mana cost (generic first by its generic part, colored shards by matching colored shards).
    SacrificeReduceByManaCost { filter: FilterBox },
    /// Harmonize: tap an untapped permanent matching the filter (summoning sickness doesn't matter); the generic part of the spell's mana cost is reduced by its power.
    TapReduceByPower { filter: FilterBox },
    /// Discard your whole hand (possibly zero cards): Bomat Courier, Subira.
    DiscardHand,
    /// Sacrifice every permanent you control that matches the filter (Kaervek's Spite: all permanents; Tomb of Urami: all lands); always payable.
    SacrificeAll {
        filter: FilterBox,
        /// As an additional spell cost: the sacrificed permanents' total power (last known, as they were sacrificed) becomes the spell's X, read with `Amount::X` (Soulblast).
        #[serde(default, skip_serializing_if = "is_false")]
        power_to_x: bool,
    },
    Discard {
        #[serde(default)]
        filter: FilterBox,
        #[serde(default = "one")]
        n: i32,
        /// "Discard a card at random" as a cost.
        #[serde(default, skip_serializing_if = "is_false")]
        random: bool,
    },
    DiscardSelf,
    ExileSelf,
    /// Exile a spell you control that matches `filter` from the stack (Nivmagus Elemental: "Exile an instant or sorcery spell you control"); it doesn't resolve and isn't countered. The spell is Remembered.
    ExileSpellFromStack { filter: FilterBox },
    /// "Put this on the bottom of its owner's library" as a cost.
    PutSelfOnLibraryBottom,
    ExileFromGraveyard {
        #[serde(default)]
        filter: FilterBox,
        #[serde(default = "one")]
        n: i32,
        /// Put the cards on the bottom of your library instead of exiling them ("put two cards from your graveyard on the bottom of your library").
        #[serde(default, skip_serializing_if = "is_false")]
        bottom: bool,
    },
    /// Exile a number of cards matching `filter` from your graveyard computed when the cost is paid ("exile cards from your graveyard equal to the number of opponents you have": Araumi of the Dead Tide). Zero is payable.
    ExileFromGraveyardAmount { filter: FilterBox, amount: Amount },
    /// Exile all cards from your graveyard (Grip of Amnesia's "unless its controller exiles all cards from their graveyard"; always payable).
    ExileGraveyard,
    /// Exile any number of other cards from your graveyard with `n` or more card types among them (Nethergoyf's escape); the player picks the cards.
    ExileGraveyardTypes { n: i32 },
    /// Exile any number of cards matching `filter` from your graveyard with total mana value `n` or greater; the player picks the cards (The Capitoline Triad).
    ExileGraveyardMv { filter: FilterBox, n: i32 },
    /// Exile one or more other permanents you control matching `filter` whose total mana value is exactly X; X is announced first, the player then picks the permanents (Fabrication Foundry: `Filter.mana_value_vs` with `Amount::X` limits the target).
    ExileTotalMvX { filter: FilterBox },
    /// Craft materials: exile one or more permanents you control matching `bf` and/or cards in your graveyard matching `gy`, in any mix; the player picks them (Paleontologist's Pick-Axe: "one or more creatures you control and/or creature cards from your graveyard").
    /// `min`: at least this many materials ("four or more"; 0 or 1 = one or more).
    ExileCraftMaterials { bf: FilterBox, gy: FilterBox, #[serde(default, skip_serializing_if = "is_zero_u8")] min: u8, #[serde(default, skip_serializing_if = "is_false")] share_type: bool },
    /// Exile `n` cards matching `filter` that all come from one graveyard, any player's (Night Soil: "Exile two creature cards from a single graveyard").
    ExileFromSingleGraveyard {
        #[serde(default)]
        filter: FilterBox,
        #[serde(default = "one")]
        n: i32,
        /// Put the cards on the bottom of their owners' libraries instead (Jotun Grunt's cumulative upkeep); a cumulative upkeep repeats the cost once per age counter.
        #[serde(default, skip_serializing_if = "is_false")]
        bottom: bool,
    },
    /// Exile a card matching `filter` from your graveyard and pay its mana cost (Back from the Brink); the exiled card is Remembered.
    ExileFromGraveyardPayCost { filter: FilterBox },
    /// Put a card from exile matching `filter` into its owner's graveyard (Shelob, Dread Weaver: "Put a creature card exiled with Shelob into its owner's graveyard"; use `exiled_with_this`). The card is Remembered.
    PutExiledIntoGraveyard { filter: FilterBox },
    ExileFromHand {
        #[serde(default)]
        filter: FilterBox,
        #[serde(default = "one")]
        n: i32,
        /// Put the card(s) on top of your library instead of exiling them (Leashling).
        #[serde(default, skip_serializing_if = "is_false")]
        to_library: bool,
    },
    /// Tap n untapped permanents you control that match the filter.
    TapPermanents {
        filter: FilterBox,
        #[serde(default = "one")]
        n: i32,
    },
    /// Untap n tapped permanents you control that match the filter (Halo Fountain); `opponent`: ones an opponent controls instead (Benthic Explorers).
    UntapPermanents {
        filter: FilterBox,
        #[serde(default = "one")]
        n: i32,
        #[serde(default, skip_serializing_if = "is_false")]
        opponent: bool,
    },
    ReturnToHand {
        filter: FilterBox,
        #[serde(default = "one")]
        n: i32,
    },
    /// Exile `n` permanents you control that match the filter (craft materials, champion). They become `Remembered` for the ability's effects.
    ExileFromBattlefield {
        filter: FilterBox,
        #[serde(default = "one")]
        n: i32,
    },
    /// Unattach a permanent matching `filter` that is attached to this permanent (for abilities granted by an Equipment: "Unattach Leonin Bola" costs the creature's granted ability the Equipment's attachment; use filter name = the Equipment).
    UnattachAttached { filter: FilterBox },
    /// Unattach this permanent from what it is attached to ("Unattach Leonin Bola").
    UnattachSelf,
    /// Behold: reveal a card matching the filter from your hand or choose a matching permanent you control (nothing moves; payable if either exists).
    Behold {
        filter: FilterBox,
        /// How many distinct cards / permanents must be beheld ("Behold three Elementals"); default 1.
        #[serde(default = "one_u8")]
        n: u8,
        /// A creature type is chosen as the cost is paid and the `n` beheld cards must all have it (Celestial Reunion); stored as the spell's `chosen_type`.
        #[serde(default, skip_serializing_if = "is_false")]
        choose_type: bool,
    },
    /// "Choose a creature you control or a warped creature card you own in exile" (Close Encounter): payable if one exists; the one with the greatest power is chosen
    /// (the best choice for a "damage equal to its power" spell) and its power is read back with `Amount::BeheldPower`.
    ChooseCreatureOrWarped,
    /// Remove `n` counters of `kind` from one permanent you control that matches the filter ("remove a +1/+1 counter from a creature you control"). The chosen permanent is `Remembered`.
    RemoveCountersFrom {
        filter: FilterBox,
        kind: CounterKind,
        #[serde(default = "one")]
        n: i32,
        /// "Remove N counters from among creatures you control": the N counters may come from several permanents (taken automatically, no choice) instead of all from one.
        #[serde(default, skip_serializing_if = "is_false")]
        among: bool,
        /// The counter may instead come from a card you own in exile that holds counters of the kind (a suspended card: "a permanent you control or suspended card you own", Rift Elemental).
        #[serde(default, skip_serializing_if = "is_false")]
        suspended: bool,
    },
    RemoveCounters {
        kind: CounterKind,
        #[serde(default = "one")]
        n: i32,
    },
    /// Remove `n` counters of any kind(s) from this permanent (taken automatically, most plentiful kind first): "remove a counter from this creature".
    RemoveAnyCounters {
        #[serde(default = "one")]
        n: i32,
    },
    /// Remove `n` counters of any kind(s), in total, from permanents you control that match `filter` (taken automatically, most plentiful kind first).
    RemoveAnyCountersFrom {
        filter: FilterBox,
        #[serde(default = "one")]
        n: i32,
    },
    PutCounters {
        kind: CounterKind,
        #[serde(default = "one")]
        n: i32,
    },
    Mill { n: i32 },
    /// Draw `n` cards as a cost (Psychic Vortex: cumulative upkeep—Draw a card).
    DrawCards { n: i32 },
    /// Exile the top `n` cards of your library as a cost (Arc-Slogger, Phyrexian Devourer, Royal Herbalist).
    ExileTop { n: i32 },
    /// "Have <the source's controller> draw `n` cards" as a cost paid by someone else (Development: "unless any opponent has you draw a card"): the controller of the cost's source draws.
    ControllerDraws { n: i32 },
    /// An opponent of the payer's choice gains `n` life as a cost (Wall of Shards: cumulative upkeep).
    OpponentGainsLife { n: i32 },
    /// Each other player gains `n` life as a cost (Skyshroud Cutter's alternative cost).
    EachOtherPlayerGainsLife { n: i32 },
    /// An opponent of the payer's choice creates `n` of the named token as a cost ("allow an opponent to create a 1/1 Survivor": Varchild's War-Riders).
    OpponentCreatesToken { token: String, n: i32 },
    PayEnergy { n: i32 },
    /// "Get n poison counters" as a cost (Ward—Get five poison counters: The Serpent Society); always payable.
    GetPoison { n: i32 },
    /// Flip `n` coins as a cost (Karplusan Minotaur's cumulative upkeep, scaled by age counters); always payable. Each flip fires the usual coin-flip triggers.
    FlipCoins { n: i32 },
    /// Pay energy equal to an amount (evaluated against the source and chosen targets): Behemoth of Vault 0, Confiscation Coup.
    PayEnergyX { amount: Amount },
    Reveal {
        #[serde(default)]
        filter: FilterBox,
        /// Reveal this many matching cards from your hand (`XTimes` scales it).
        #[serde(default = "one")]
        n: i32,
    },
    /// The cost is chosen from several alternatives by the player ("...or ...").
    Or { options: Vec<Vec<Cost>> },
    /// Internal: sacrifice exactly these permanents (the payer already chose them).
    #[serde(skip)]
    SacrificeCards { cards: Vec<u32> },
    /// Internal: discard exactly these cards.
    #[serde(skip)]
    DiscardCards { cards: Vec<u32> },
    /// Internal (after the payer chose): put the counters on exactly this permanent.
    #[serde(skip)]
    PutCountersOnCard { card: u32, kind: CounterKind, n: i32 },
    /// Internal (after the payer chose the opponent of `OpponentGainsLife`): this player gains `n` life.
    #[serde(skip)]
    PlayerGainsLife { player: u8, n: i32 },
    /// Internal (after the payer chose the opponent of `OpponentCreatesToken`): this player creates the tokens.
    #[serde(skip)]
    PlayerCreatesToken { player: u8, token: String, n: i32 },
    /// Internal (after the payer chose): gain control of exactly this permanent.
    #[serde(skip)]
    GainControlOfCard { card: u32 },
}

// ---------------------------------------------------------------- targeting & filters

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TargetSpec {
    pub what: TargetKind,
    #[serde(default = "one_u8")]
    pub min: u8,
    #[serde(default = "one_u8")]
    pub max: u8,
    /// Free-form label shown in the UI ("target creature").
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    /// The most targets is X ("up to X target creatures"); `max` is ignored.
    #[serde(default, skip_serializing_if = "is_false")]
    pub max_x: bool,
    /// With `max_x`: exactly X targets (minimum X too) instead of "up to X".
    #[serde(default, skip_serializing_if = "is_false")]
    pub exact_x: bool,
    /// With `exact_x` (and no `max_x`): exactly N targets (Strength of the Tajuru: KickCount + 1).
    /// "up to N target creatures" where N is a computed amount (Archipelagore: `TimesMutated`; evaluated when the targets are chosen); `max` is ignored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_amount: Option<Amount>,
    /// The chosen targets' total mana value can't exceed this ("any number of target artifacts with total mana value 10 or less"); targets beyond the budget (in the order picked) are dropped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_total_mv: Option<Amount>,
    /// Same for total power ("any number of target creature cards with total power 6 or less").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_total_power: Option<Amount>,
    /// The player who chooses this slot's targets instead of the controller ("target creature of an opponent's choice": `{"type":"TriggerPlayer"}`, `ChosenPlayer`, `EachOpponent`'s first ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chooser: Option<Sel>,
    /// "another target": must differ from the targets chosen for the earlier slots of the same spell or ability.
    #[serde(default, skip_serializing_if = "is_false")]
    pub distinct: bool,
    /// The slot exists only if the spell was kicked / the gift was promised (`true`) or only if it was not (`false`): "if the gift was promised, target ...".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when_kicked: Option<bool>,
    /// One separate target slot per opponent ("for each opponent, up to one target creature that player controls"): `min`/`max` apply to each opponent's slot; the chosen targets are all in `Targets`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub per_opponent: bool,
    /// Like `per_opponent` but also one slot for the controller ("for each player, up to one target creature that player controls"; the controller's slot comes first).
    #[serde(default, skip_serializing_if = "is_false")]
    pub per_player: bool,
    /// "cards from a single graveyard": every chosen card must belong to the same player's graveyard (picks from other graveyards than the first are dropped).
    #[serde(default, skip_serializing_if = "is_false")]
    pub same_graveyard: bool,
    /// The chosen targets must all differ in this respect ("with different names", "each with a different mana value"); picks that repeat an earlier pick's value are dropped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distinct_by: Option<ChooseDistinct>,
    /// The target is chosen at random among the legal ones as the spell or ability is put on the stack ("target creature an opponent controls chosen at random": Scab-Clan Giant).
    #[serde(default, skip_serializing_if = "is_false")]
    pub random: bool,
    /// (engine use) the target must be controlled by this player; set when a `per_opponent` slot is expanded.
    #[serde(skip)]
    #[schemars(skip)]
    pub only_controller: Option<u8>,
    /// In a `Reflexive` inside a `ChooseModes`: the targets can't repeat anything an earlier mode of the same resolution targeted ("each mode must target a different player": Shadrix Silverquill).
    #[serde(default, skip_serializing_if = "is_false")]
    pub distinct_modes: bool,
    /// With `per_opponent`: a slot exists only for opponents with three or more poison counters as the spell is cast (corrupted: Geth's Summons).
    #[serde(default, skip_serializing_if = "is_false")]
    pub corrupted: bool,
}

fn is_one_u8(n: &u8) -> bool {
    *n == 1
}
fn zone_battlefield() -> Zone {
    Zone::Battlefield
}
fn one_u8() -> u8 {
    1
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum TargetKind {
    /// A permanent on the battlefield matching the filter.
    Permanent { filter: FilterBox },
    /// Any target: creature, planeswalker, battle or player.
    AnyTarget,
    /// A player or a planeswalker ("target player or planeswalker").
    PlayerOrPlaneswalker,
    /// An opponent or a planeswalker ("target opponent or planeswalker").
    OpponentOrPlaneswalker,
    /// Any one of these kinds of target ("target artifact, creature, planeswalker or opponent": a Permanent kind plus `Player {Opponent}`).
    AnyOf { kinds: Vec<TargetKind> },
    Player { filter: PlayerFilter },
    /// A card in a zone (graveyard targets and the like).
    CardIn { zone: Zone, filter: FilterBox, whose: PlayerFilter },
    /// A spell on the stack matching the filter.
    Spell { filter: FilterBox },
    /// A spell or ability on the stack. The filter's target predicates (`single_target`, `spell_has_targets`, `spell_targets`, `spell_targets_you`) apply to either kind;
    /// the rest of the filter applies to spells (via the spell card).
    StackObject {
        #[serde(default)]
        filter: FilterBox,
    },
    /// An activated and/or triggered ability on the stack (Stifle, "copy target activated or triggered ability you control").
    /// `controller` filters who controls it; `filter` filters its source permanent.
    StackAbility {
        #[serde(default)]
        controller: PlayerFilter,
        #[serde(default)]
        kind: StackAbilityKind,
        #[serde(default)]
        filter: FilterBox,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum StackAbilityKind {
    #[default]
    Any,
    Activated,
    Triggered,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum PlayerFilter {
    #[default]
    Any,
    You,
    Opponent,
    /// You or a teammate (Archenemy: the archenemy's opponents are one team; otherwise only you; Valor's Reach).
    Team,
    /// The player this card enchants (Curses).
    Enchanted,
    /// The owner of the source card ("at the beginning of that player's next declare attackers step" after exiling the source: Noctis, Heir Apparent).
    SourceOwner,
    /// An opponent of the controller who controls a creature attacking the enchanted player or a planeswalker/battle they control (Curse of Verbosity).
    AttackingEnchanted,
    /// A player with the most life, or tied for the most (among all players).
    MostLife,
    /// A player with the least life, or tied for the least.
    LeastLife,
    /// An opponent of the controller with at least two fewer creature cards in their graveyard than the controller (Keeper of the Dead).
    TwoFewerGraveyardCreaturesThanYou,
    /// The active player ("target opponent whose turn it is").
    Active,
    /// A player controlling the most lands, or tied for the most (Tectonic Hellion).
    MostLands,
    /// A player controlling the fewest creatures, or tied (Gor Muldrak).
    FewestCreatures,
    /// A player controlling the most creatures, or tied (Wild Mammoth, No Witnesses).
    MostCreatures,
    /// A player controlling the most Wizards, or tied (Thoughtbound Primoc).
    MostWizards,
    /// A player with the most cards in hand, or tied (Sokenzan Renegade).
    MostCardsInHand,
    /// A player who attacked this turn (declared at least one attacker).
    AttackedThisTurn,
    /// An opponent who attacked this turn.
    OpponentAttackedThisTurn,
    /// An opponent who attacked the controller or a planeswalker the controller controls this turn (Sandswirl Wanderglyph).
    AttackedYouThisTurn,
    /// An opponent who searched their own library this turn.
    OpponentSearchedLibraryThisTurn,
    /// An opponent who cast a spell this turn.
    OpponentCastSpellThisTurn,
    /// A player who cast a spell this turn.
    CastSpellThisTurn,
    /// A player who cast a sorcery spell this turn (Backdraft).
    CastSorceryThisTurn,
    /// A player who cast a nonartifact spell this turn (Ethersworn Canonist).
    CastNonartifactSpellThisTurn,
    /// A player who cast a non-Phyrexian spell this turn (Phyrexian Censor).
    CastNonPhyrexianSpellThisTurn,
    /// An opponent with the most life, or tied, among the opponents (Keeper of the Flame).
    OpponentMostLife,
    /// A player with more life than the controller.
    MoreLifeThanYou,
    /// An opponent of the ACTIVE player with more life than them (Oath of Mages: "target player who has more life than they do and is their opponent").
    OpponentWithMoreLifeThanActive,
    /// An opponent of the ACTIVE player with more cards in hand than them (Oath of Scholars).
    OpponentWithMoreCardsThanActive,
    /// An opponent of the ACTIVE player who controls more lands than them (Oath of Lieges).
    OpponentWithMoreLandsThanActive,
    /// An opponent of the ACTIVE player who controls more creatures than them (Oath of Druids).
    OpponentWithMoreCreaturesThanActive,
    /// An opponent of the ACTIVE player whose graveyard has fewer creature cards than theirs (Oath of Ghouls).
    OpponentWithFewerGraveyardCreaturesThanActive,
    /// A player whose life total is less than half their starting life total (Anya, Merciless Angel).
    LifeBelowHalfStarting,
    /// A player whose graveyard holds seven or more cards (Master's Councillors: "for each graveyard with seven or more cards in it").
    GraveyardSevenOrMore,
    /// Lost life this turn (Rix Maadi Guildmage).
    LostLifeThisTurn,
    /// A player who controls more creatures than the controller.
    MoreCreaturesThanYou,
    /// A player who controls more artifacts than the controller (Ward of Bones).
    MoreArtifactsThanYou,
    /// A player who controls more enchantments than the controller (Ward of Bones).
    MoreEnchantmentsThanYou,
    /// A player who controls more lands than the controller (Ward of Bones).
    MoreLandsThanYou,
    /// A player who controls more permanents than each other player, unless they have ignored the source's effect this turn with `Effect::IgnoreSourceEffect` (Damping Engine).
    MorePermanentsThanEachOther,
    /// A player with more cards in hand than the controller.
    MoreCardsInHandThanYou,
    /// A player with at least two more cards in hand than the controller.
    TwoMoreCardsInHandThanYou,
    /// A player who was dealt damage this turn (any source).
    DealtDamageThisTurn,
    /// A player who was dealt combat damage this turn.
    DealtCombatDamageThisTurn,
    /// A player who was dealt noncombat damage this turn.
    DealtNoncombatDamageThisTurn,
    /// A player who was dealt damage this turn by this source (Wicked Akuba).
    DealtDamageByThisThisTurn,
    /// A player who was dealt combat damage this turn by this source (Steel Hellkite, Hope of Ghirapur).
    DealtCombatDamageByThisThisTurn,
    /// A player who was ever dealt damage by this source this game (Diseased Vermin).
    EverDamagedByThis,
    /// An opponent (of the effect's controller) who was ever dealt damage by this source this game (The Fallen: "each opponent ... it has dealt damage to this game").
    OpponentEverDamagedByThis,
    /// An opponent who was ever dealt combat damage by a creature with this source's name (Gollum, Obsessed Stalker: "each opponent dealt combat damage this game by a creature named ...").
    OpponentEverCombatDamagedBySameName,
    /// A player who was ever dealt combat damage by this source this game.
    EverCombatDamagedByThis,
    /// A player who was ever dealt combat damage this game by a creature with the same name as this source (Gollum, Obsessed Stalker).
    EverCombatDamagedBySameName,
    /// Any player except the one this Curse / Aura enchants ("any number of target players other than that player").
    OtherThanEnchanted,
    /// Any player except the player who caused the trigger ("choose another target player": The Lord of Pain). Judged while targets of a triggered ability are chosen.
    OtherThanTriggerPlayer,
    /// Exactly the player who caused the trigger ("target card they own in exile" after combat damage to that player: Time Reaper). Judged while a triggered ability's targets are chosen / it resolves.
    TriggerPlayer,
    /// Any player except this source's owner ("target player other than this artifact's owner", Crown of Doom).
    NotOwner,
    /// Any player except the active player (Dosan the Falling Leaf: "players can cast spells only during their own turns").
    NotActive,
    /// The player this source is attacking (or the controller of the planeswalker / battle it attacks): "defending player" for an attacking source.
    DefendingPlayerOfThis,
    /// The player the creature this Aura enchants is attacking (or the controller of the planeswalker / battle it attacks): "defending player" for "whenever enchanted creature attacks" (Chorale of the Void).
    DefendingPlayerOfAttached,
    /// The controller of the permanent this Aura / Equipment is attached to ("the upkeep of enchanted creature's controller", Soul Tithe, Venarian Gold).
    ControllerOfAttached,
    /// The player this source chose as it entered ("the chosen player's upkeep", Black Vise, Energy Vortex).
    ChosenPlayer,
    /// The player to the LEFT of the controller ("you"): the next player in turn order who is still in the game. In a creature filter's
    /// `controller` it is relative to whoever the filter is judged for, e.g. the chooser of `EachPlayerChooses` ("a creature controlled by the player to their left").
    LeftOfYou,
    /// A player whose last named choice for this source's card (`EachPlayerChooses { options, remember: true }`) was the FIRST option (Two Streams Facility: green anchor).
    ChoseFirstOption,
    /// Like `ChoseFirstOption` for the SECOND option (red waterfall).
    ChoseSecondOption,
}

/// A `Filter` held on the heap. `Filter` is large (about 1 KB), so holding it inline would make every `Effect`/`Condition`/`Trigger`
/// several KB. An empty filter (the common default) is `None` and costs no allocation.
/// Derefs to `Filter` for reading and writing; JSON and schema are exactly those of `Filter`.
#[derive(Clone, Default)]
pub struct FilterBox(Option<Box<Filter>>);

static DEFAULT_FILTER: std::sync::OnceLock<Filter> = std::sync::OnceLock::new();

impl std::ops::Deref for FilterBox {
    type Target = Filter;
    fn deref(&self) -> &Filter {
        match &self.0 {
            Some(b) => b,
            None => DEFAULT_FILTER.get_or_init(Filter::default),
        }
    }
}
impl std::ops::DerefMut for FilterBox {
    fn deref_mut(&mut self) -> &mut Filter {
        self.0.get_or_insert_with(Default::default)
    }
}
impl FilterBox {
    /// An owned copy of the filter (what `..filter.clone()` struct updates and by-value parameters want).
    pub fn into_filter(self) -> Filter {
        match self.0 {
            Some(b) => *b,
            None => Filter::default(),
        }
    }
}
impl From<Filter> for FilterBox {
    fn from(f: Filter) -> Self {
        if f == Filter::default() {
            FilterBox(None)
        } else {
            FilterBox(Some(Box::new(f)))
        }
    }
}
impl std::fmt::Debug for FilterBox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        (**self).fmt(f)
    }
}
impl PartialEq for FilterBox {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}
impl Serialize for FilterBox {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        (**self).serialize(s)
    }
}
impl<'de> Deserialize<'de> for FilterBox {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Filter::deserialize(d).map(FilterBox::from)
    }
}
impl JsonSchema for FilterBox {
    fn is_referenceable() -> bool {
        Filter::is_referenceable()
    }
    fn schema_name() -> String {
        Filter::schema_name()
    }
    fn schema_id() -> std::borrow::Cow<'static, str> {
        Filter::schema_id()
    }
    fn json_schema(g: &mut schemars::gen::SchemaGenerator) -> schemars::schema::Schema {
        Filter::json_schema(g)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Filter {
    /// Matches exactly this card (set by effects that single out a chosen object, e.g. "target creature blocks target creature"); never written in card JSON.
    #[serde(skip)]
    pub only_card: Option<u32>,
    /// In a mana restriction's `ability` filter: "an ability of a [type] source" also covers abilities of cards in hand or graveyard (cycling), not just permanents.
    #[serde(default, skip_serializing_if = "is_false")]
    pub any_object: bool,
    /// Any of these types (empty = any type).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub types: Vec<CardType>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub not_types: Vec<CardType>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supertypes: Vec<Supertype>,
    /// Has none of these supertypes ("nonbasic", "nonlegendary").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub not_supertypes: Vec<Supertype>,
    /// Owned by a player matching this (relative to the source's controller): "you own", "an opponent owns".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<PlayerFilter>,
    /// Attacking the source's controller or a planeswalker/battle they control (not another player).
    #[serde(default, skip_serializing_if = "is_false")]
    pub attacking_you: bool,
    /// Attacking the source's controller themself (not their planeswalkers/battles): Arachnogenesis.
    #[serde(default, skip_serializing_if = "is_false")]
    pub attacking_you_player: bool,
    /// With `attacking_player`: only creatures attacking a player themself, not a planeswalker or battle they control (Landroval, Horizon Witness).
    #[serde(default, skip_serializing_if = "is_false")]
    pub attacking_player_direct: bool,
    /// Attacking one of the source controller's opponents themself, not a planeswalker or battle (Oviya, Automech Artisan).
    #[serde(default, skip_serializing_if = "is_false")]
    pub attacking_opponent_player: bool,
    /// Attacking the player the source's controller last chose (or a planeswalker/battle they control): Beckoning Will-o'-Wisp.
    #[serde(default, skip_serializing_if = "is_false")]
    pub attacking_chosen_player: bool,
    /// Attacking the player the source Aura enchants (Curse of Hospitality: "creatures attacking enchanted player").
    #[serde(default, skip_serializing_if = "is_false")]
    pub attacking_enchanted_player: bool,
    /// The source is attacking and this is the source itself or a creature attacking in the same band as it (Camel: "this creature and creatures banded with this creature").
    #[serde(default, skip_serializing_if = "is_false")]
    pub banded_with_attacking_source: bool,
    /// A spell cast using mana produced by the source permanent ("a permanent spell using mana produced by this land").
    #[serde(default, skip_serializing_if = "is_false")]
    pub paid_with_mana_from_source: bool,
    /// An attacking creature that is attacking someone other than the source's controller and their planeswalkers/battles ("attacks a player other than you").
    #[serde(default, skip_serializing_if = "is_false")]
    pub not_attacking_you: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subtypes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub not_subtypes: Vec<String>,
    /// Has any of these colors.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub colors: Vec<Color>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub not_colors: Vec<Color>,
    /// Has every one of these colors ("both white and black").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub all_colors: Vec<Color>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub colorless: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub multicolored: bool,
    /// Has at least one activated ability (printed or granted; mana and loyalty abilities count): craft with "nonlands with activated abilities" (The Enigma Jewel).
    #[serde(default, skip_serializing_if = "is_false")]
    pub has_activated_ability: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub monocolored: bool,
    /// Exactly this many colors (Meeting of the Five: exactly three).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_count: Option<u8>,
    /// Controlled continuously since the beginning of this turn (Keldon Twilight, Siren's Call, Nettling Imp).
    #[serde(default, skip_serializing_if = "is_false")]
    pub controlled_since_turn_start: bool,
    /// Attacked the source's controller (or a planeswalker/battle they control) this turn, even if it is no longer attacking (Jabari's Influence).
    #[serde(default, skip_serializing_if = "is_false")]
    pub attacked_you_this_turn: bool,
    /// Currently attacking a battle (War-Trained Slasher, Thrashing Frontliner, Rampaging Geoderm).
    #[serde(default, skip_serializing_if = "is_false")]
    pub attacking_battle: bool,
    /// Attacked a battle this turn (War Historian).
    #[serde(default, skip_serializing_if = "is_false")]
    pub attacked_battle_this_turn: bool,
    /// Not exactly this many colors (Invasion of Ravnica: "isn't exactly two colors").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_color_count: Option<u8>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keywords: Vec<Keyword>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub not_keywords: Vec<Keyword>,
    #[serde(default)]
    pub controller: PlayerFilter,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tapped: Option<bool>,
    /// Face-down (`true`) / face-up (`false`) permanents or spells (morph, disguise, manifest, cloak).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub face_down: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<bool>,
    /// `true`: the card has a morph (or megamorph / disguise) ability in its printed text; `false`: it has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_morph: Option<bool>,
    /// `true`: the card has a disguise ability printed (a morph that is not a plain morph); `false`: it has none (Expose the Culprit).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_disguise: Option<bool>,
    /// `true`: a spell that was kicked (any kicker paid); `false`: not kicked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kicked: Option<bool>,
    /// `true`: the card has an Adventure half (printed); `false`: it has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_adventure: Option<bool>,
    /// `true`: a double-faced card (transforming or modal, it has another face); `false`: a single-faced card.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub double_faced: Option<bool>,
    /// `true`: a goaded creature (one-shot or continuous goad); `false`: not goaded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goaded: Option<bool>,
    /// `true`: the object has no abilities (no printed abilities or keywords, or it lost them); `false`: it has some.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_abilities: Option<bool>,
    /// The card is in exile and was exiled by this ability's source ("exiled with this"): for Choose / Count / CantCast filters.
    #[serde(default, skip_serializing_if = "is_false")]
    pub exiled_with_this: bool,
    /// A token created by an effect of this ability's source (while it is the same object): "tokens created with this creature" (Tetravus).
    #[serde(default, skip_serializing_if = "is_false")]
    pub created_by_this: bool,
    /// A card the source's `GrantPlayFromExile` permission lets the controller play, matched while it is being cast or played from exile: "when you play a card this way" (Fires of Mount Doom).
    #[serde(default, skip_serializing_if = "is_false")]
    pub played_via_this: bool,
    /// The permanent is attached to one of the cards selected here (Fires of Mount Doom: "Equipment attached to that creature", `Target`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attached_to: Option<Box<Sel>>,
    /// The card is the permanent this source linked itself to with `Effect::LinkRemembered` (on the battlefield, or just left it: "the creature put onto the battlefield with this").
    #[serde(default, skip_serializing_if = "is_false")]
    pub linked_with_this: bool,
    /// The card is NOT a permanent this source linked itself to with `LinkRemembered` (Fight or Flight: "only creatures in the chosen pile can attack").
    #[serde(default, skip_serializing_if = "is_false")]
    pub not_linked_with_this: bool,
    /// The card shares no land type with any land its searcher / `src_controller` controls (Hiveheart Shaman).
    #[serde(default, skip_serializing_if = "is_false")]
    pub no_land_type_shared_with_yours: bool,
    /// Soulbond: `true` = paired with a creature, `false` = unpaired.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paired: Option<bool>,
    /// Soulbond: this creature if it is paired, and the creature paired with the source ("each of those creatures").
    #[serde(default, skip_serializing_if = "is_false")]
    pub paired_with_this: bool,
    /// "Modified": the creature has a counter on it, is equipped, or is enchanted by an Aura its controller controls.
    #[serde(default, skip_serializing_if = "is_false")]
    pub modified: bool,
    /// Not the ability's own source ("another", "other").
    #[serde(default, skip_serializing_if = "is_false")]
    pub other: bool,
    /// Must be the source itself.
    #[serde(default, skip_serializing_if = "is_false")]
    pub this: bool,
    /// The card is (or, as a spell being cast, was cast from) this zone / is not in this zone ("spells you cast from your graveyard",
    /// "spells cast from anywhere other than your hand").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cast_from: Option<Zone>,
    /// The card is the very top card of its owner's graveyard, whatever it is (Bösium Strip; unlike `graveyard_top`, which looks at the top matching card).
    #[serde(default, skip_serializing_if = "is_false")]
    pub graveyard_top_card: bool,
    /// The card's name is the name chosen on the source ("spells with the chosen name").
    #[serde(default, skip_serializing_if = "is_false")]
    pub chosen_name: bool,
    /// The permanent is suspected.
    #[serde(default, skip_serializing_if = "is_false")]
    pub suspected: bool,
    /// The token was created by the source ("tokens created with this enchantment", Dual Nature; made by `CopyPermanent`). Still matches after the source left the battlefield.
    #[serde(default, skip_serializing_if = "is_false")]
    pub created_by_source: bool,
    /// The permanent is its controller's Ring-bearer.
    #[serde(default, skip_serializing_if = "is_false")]
    pub ring_bearer: bool,
    /// The card is a commander (use `owner` for "your commander"; a commander creature is `types:[Creature]` + this).
    #[serde(default, skip_serializing_if = "is_false")]
    pub commander: bool,
    /// The card is NOT a commander.
    #[serde(default, skip_serializing_if = "is_false")]
    pub not_commander: bool,
    /// The card's mana cost contains {X}.
    #[serde(default, skip_serializing_if = "is_false")]
    pub has_x: bool,
    /// A modal spell: a card with a "choose one / two / ..." spell ability (Riku of Many Paths: "whenever you cast a modal spell").
    #[serde(default, skip_serializing_if = "is_false")]
    pub modal_spell: bool,
    /// The card's mana cost contains a Phyrexian mana symbol ({H}: Rage Extractor).
    #[serde(default, skip_serializing_if = "is_false")]
    pub has_phyrexian_mana: bool,
    /// A permanent whose back face is up because it transformed (not a modal / adventure face): Mutagen Connoisseur.
    #[serde(default, skip_serializing_if = "is_false")]
    pub transformed: bool,
    /// A renowned creature (Enshrouding Mist: "if it's renowned").
    #[serde(default, skip_serializing_if = "is_false")]
    pub renowned: bool,
    /// The card's rules text has a keyword line starting with this word ("flashback", "devoid", "freerunning", "cycling", "equip"; case-insensitive; also in a comma list): for keywords that are not in the Keyword enum.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_keyword_word: Option<String>,
    /// The card's rules text has no keyword line starting with this word (the opposite of `has_keyword_word`: Bohn, Beguiling Balladeer "without foretell").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lacks_keyword_word: Option<Box<String>>,
    /// The permanent currently has at least one protection quality (printed or granted): Concerted Effort.
    #[serde(default, skip_serializing_if = "is_false")]
    pub has_protection: bool,
    /// An Aura card (any zone) that could enchant the filter's source permanent: it has an enchant restriction the source meets (Bruna, Light of Alabaster).
    #[serde(default, skip_serializing_if = "is_false")]
    pub aura_could_enchant_source: bool,
    /// A creature in the sector the source chose (`ChooseCreatureType` with `only` Alpha / Beta / Gamma stores it on the source; Space Beleren).
    #[serde(default, skip_serializing_if = "is_false")]
    pub in_chosen_sector: bool,
    /// The permanent currently has protection from every colour (from each of the five colours, from monocolored and multicolored sources, or from everything): Escaped Shapeshifter.
    #[serde(default, skip_serializing_if = "is_false")]
    pub protection_any_color: bool,
    /// The permanent has no protection from the source (so the source Aura could legally enchant it): "a creature at random this Aura can enchant".
    #[serde(default, skip_serializing_if = "is_false")]
    pub can_be_enchanted_by_source: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_cast_from: Option<Zone>,
    /// true = the permanent / spell was cast; false = it was not cast (put onto the battlefield or onto the stack by an effect, a copy).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub was_cast: Option<bool>,
    /// It was cast for its warp cost (Full Bore: "if that creature was cast for its warp cost").
    #[serde(default, skip_serializing_if = "is_false")]
    pub cast_for_warp: bool,
    /// This spell was the nth spell cast this turn by any player (1-based): "the second spell cast this turn" (Second Guess).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nth_spell_this_turn: Option<u32>,
    /// This spell is the first spell cast this turn by its caster that matches the rest of this filter ("their first multicolored spell each turn").
    #[serde(default, skip_serializing_if = "is_false")]
    pub first_by_caster_this_turn: bool,
    /// This card has the same name as a spell that was cast this turn (Twinning Glass).
    #[serde(default, skip_serializing_if = "is_false")]
    pub same_name_as_cast_spell: bool,
    /// This card has the same name as a nontoken permanent on the battlefield (Cornered Market: "spells with the same name as a nontoken permanent"; the card itself does not count).
    #[serde(default, skip_serializing_if = "is_false")]
    pub same_name_as_nontoken_permanent: bool,
    /// The permanent was cast with an offspring cost granted by `StaticEffect::GrantOffspring` paid (Zinnia, Valley's Voice).
    #[serde(default, skip_serializing_if = "is_false")]
    pub granted_offspring: bool,
    /// Is exactly one colour and it is the colour its controller chose with `ChooseColor.per_player` on the source (Selective Obliteration).
    #[serde(default, skip_serializing_if = "is_false")]
    pub only_controllers_chosen_color: bool,
    /// Is NOT (exactly one colour and that colour is the colour its controller chose with `ChooseColor.per_player` on the source): Selective Obliteration's "unless it's only the color its controller chose".
    #[serde(default, skip_serializing_if = "is_false")]
    pub not_only_controllers_chosen_color: bool,
    /// This card is in exile and foretold (Alrund, God of the Cosmos: "each foretold card you own in exile").
    #[serde(default, skip_serializing_if = "is_false")]
    pub foretold: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub power: Option<Cmp>,
    /// The card's PRINTED power / toughness (before counters and effects) compared: "creatures with base power and toughness 1/1".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_power: Option<Cmp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_toughness: Option<Cmp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toughness: Option<Cmp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mana_value: Option<Cmp>,
    /// Mana value parity: true = even (zero is even), false = odd.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mana_value_even: Option<bool>,
    /// Mana value has the parity chosen by the source's `ChooseKind::OddEven` (zero is even).
    #[serde(default, skip_serializing_if = "is_false")]
    pub mana_value_chosen_parity: bool,
    /// Mana value differs from the source's chosen number (`ChooseKind::RandomTwoToFour`: "protection from each mana value other than the chosen number").
    #[serde(default, skip_serializing_if = "is_false")]
    pub mana_value_not_chosen_number: bool,
    /// The card has two or more card types (Rendmaw, Creaking Nest).
    #[serde(default, skip_serializing_if = "is_false")]
    pub multi_type: bool,
    /// A card in its owner's hand that was drawn this turn and hasn't left the hand since (Sylvan Library).
    #[serde(default, skip_serializing_if = "is_false")]
    pub drawn_this_turn: bool,
    /// The card is the last card its owner drew this turn and is still in their hand (Jandor's Ring).
    #[serde(default, skip_serializing_if = "is_false")]
    pub last_drawn: bool,
    /// A planeswalker that had a loyalty ability activated this turn (Cut Short).
    #[serde(default, skip_serializing_if = "is_false")]
    pub loyalty_activated_this_turn: bool,
    /// The card is currently in this zone (Ashes of the Abhorrent: `CantActivate` / `CantCastCards` limited to graveyard cards).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_zone: Option<Zone>,
    /// Power parity: true = even (zero is even), false = odd (Zimone's Hypothesis).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub power_even: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub attacking: bool,
    /// It is attacking alone (the only attacking creature) or blocking alone (the only blocking creature); still true while it dies in combat (Thijarian Witness).
    #[serde(default, skip_serializing_if = "is_false")]
    pub combat_alone: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub blocking: bool,
    /// Is blocking the source creature ("creatures blocking this creature").
    #[serde(default, skip_serializing_if = "is_false")]
    pub blocking_source: bool,
    /// Is blocked by the source creature (an attacker blocked by this blocker).
    #[serde(default, skip_serializing_if = "is_false")]
    pub blocked_by_source: bool,
    /// Is blocking the creature this Equipment / Aura is attached to ("one of those creatures" for the equipped creature: Godsend).
    #[serde(default, skip_serializing_if = "is_false")]
    pub blocking_attached: bool,
    /// Is blocked by the creature this Equipment / Aura is attached to (Godsend).
    #[serde(default, skip_serializing_if = "is_false")]
    pub blocked_by_attached: bool,
    /// Is an attacking creature in the same band as the source (not the source itself): Icatian Skirmishers.
    #[serde(default, skip_serializing_if = "is_false")]
    pub banded_with_source: bool,
    /// Is blocking a creature held in this stored slot ("creatures blocking that creature"; fill the slot with StoreRemembered).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocking_stored: Option<u8>,
    /// Tapped to pay a Crew / Saddle cost of the source this turn ("target creature that crewed it this turn").
    #[serde(default, skip_serializing_if = "is_false")]
    pub crewed_source_this_turn: bool,
    /// Tapped this turn to pay a "tap an untapped X you control" cost of the source's abilities.
    #[serde(default, skip_serializing_if = "is_false")]
    pub tapped_for_source_cost_this_turn: bool,
    /// Blocked or was blocked by a creature held in this stored slot this turn (per-card `block_pairs`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_pair_stored: Option<u8>,
    /// Is blocked by a creature held in this stored slot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_by_stored: Option<u8>,
    /// Has no counters of any kind.
    #[serde(default, skip_serializing_if = "is_false")]
    pub no_counters: bool,
    /// Can be sacrificed: not under a "can't be sacrificed" effect (set on sacrifice-cost filters by `sac_cost_filter`).
    #[serde(default, skip_serializing_if = "is_false")]
    pub sacrificable: bool,
    /// Can be returned to hand to pay a "beam me up" cost: not under a "can't be beamed up" effect (Marooned, Scotty).
    #[serde(default, skip_serializing_if = "is_false")]
    pub beamable: bool,
    /// Not an attacking creature ("nonattacking creature").
    #[serde(default, skip_serializing_if = "is_false")]
    pub not_attacking: bool,
    /// Not a blocking creature.
    #[serde(default, skip_serializing_if = "is_false")]
    pub not_blocking: bool,
    /// Has the same name as the selected card(s) ("search for cards with the same name as that creature": `{"type":"Target","idx":0}`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub same_name_as: Option<Box<Sel>>,
    /// Does NOT have the same name as any of the selected cards ("a creature with a different name": Yenna).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_same_name_as: Option<Box<Sel>>,
    /// With `exiled_with_this`: leaves out the first (oldest) card exiled with the source that has the same name as the selected card, but not later ones with that name (Spellweaver Helix: "copy the other one").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skip_first_same_name_as: Option<Box<Sel>>,
    /// The permanent's controller controls a permanent matching this filter ("a creature whose controller controls a snow Swamp": snow swampwalk as a `CantBeBlockedBy`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controller_controls: Option<Box<Filter>>,
    /// The permanent's controller controls at most this many lands ("as long as that player controls three or fewer lands": Sheltering Prayers).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controller_lands_max: Option<Box<u32>>,
    /// The permanent's controller picked this word ("War" / "Peace") in the source's per-player as-enters choice (`ChooseKind::WarPeace`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controller_chose: Option<Box<String>>,
    /// The permanent's controller was dealt combat damage this turn by at least that many different creatures matching the inner filter (Admiral Beckett Brass).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controller_hit_by: Option<Box<HitByCount>>,
    /// No permanent matching this filter (evaluated from the source controller's view, e.g. `{"token":true,"controller":"You"}`) has the same name as this one ("doesn't have the same name as a token you control": The Apprentice's Folly).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_permanent_same_name: Option<Box<Filter>>,
    /// Power plus toughness compared with a computed amount (Wild Pair: the same total as the entered creature).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_pt_vs: Option<AmountCmp>,
    /// Not one of the selected objects ("other than that creature": `TriggerObject`); judged when targets are chosen too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub except: Option<Box<Sel>>,
    /// An attacker attacking the same player or planeswalker/battle as one of the selected attackers (Kitesail Skirmisher, Jin Sakai).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attacking_same_as: Option<Box<Sel>>,
    /// An attacker whose defending player is one of the selected players (Echoing Assault, Ordruun Mentor: "attacking that player"); a planeswalker/battle counts as its controller.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attacking_player: Option<Box<Sel>>,
    /// Must be one of the selected objects ("attached to that creature": `attached_host: {only: Target 0}`); judged when targets are chosen too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub only: Option<Box<Sel>>,
    /// The creature was dealt damage this turn by one of the selected cards (Touch of Moonglove: `{"type":"Target","idx":0}` of the spell that created the temporary trigger).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub damaged_by: Option<Box<Sel>>,
    /// An Aura / Equipment attached to a permanent matching this filter (Miracle Worker: "Aura attached to a creature you control"; filter relative to the ability's controller).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attached_host: Option<Box<Filter>>,
    /// The card is one of the current targets of the selected spell on the stack (Orvar: "one of those permanents"; use `TriggerObject`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub targeted_by: Option<Box<Sel>>,
    /// The card is another legal target of the selected spell on the stack (Beamsplitter Mage: "another creature that spell could target"); its current targets are excluded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legal_target_of: Option<Box<Sel>>,
    /// Shares at least one creature type with any of the selected cards ("a creature card that shares a creature type with this creature": `{"type":"This"}`, kinship).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shares_type_with: Option<Box<Sel>>,
    /// With `shares_type_with`: the card must share a creature type with EACH of the selected cards, not just any one (Cryptic Gateway).
    #[serde(default, skip_serializing_if = "is_false")]
    pub shares_type_with_each: bool,
    /// Shares NO creature type with any of the selected cards (Radagast the Brown).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_shares_type_with: Option<Box<Sel>>,
    /// No creature on the battlefield has lower toughness (Purging Scythe: "the creature with the least toughness"; ties all match).
    #[serde(default, skip_serializing_if = "is_false")]
    pub least_toughness: bool,
    /// Shares at least one color with any of the selected cards ("a creature that shares a color with it": Ringsight, Rassilon conspire).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shares_color_with: Option<Box<Sel>>,
    /// Shares at least one color with the spell most recently cast this turn (Mana Maze); colorless or no spell cast yet: no match.
    #[serde(default, skip_serializing_if = "is_false")]
    pub shares_color_last_spell: bool,
    /// Is NOT the permanent the source is attached to ("other than enchanted creature": Sporogenic Infection, Due Diligence).
    #[serde(default, skip_serializing_if = "is_false")]
    pub not_attached_by_source: bool,
    /// Controlled by the player this source chose (`ChoosePlayer`): "target creature that player controls".
    #[serde(default, skip_serializing_if = "is_false")]
    pub controlled_by_chosen_player: bool,
    /// Has a mana ability (an activated, non-loyalty, targetless ability that adds mana): Magewright's Stone, Moonsilver Key, Power Sink.
    #[serde(default, skip_serializing_if = "is_false")]
    pub has_mana_ability: bool,
    /// Could produce {C} through any mana ability (Break the Ice).
    #[serde(default, skip_serializing_if = "is_false")]
    pub could_produce_colorless: bool,
    /// Has an activated ability that is not a mana ability (Ravager Wurm, Tazri).
    #[serde(default, skip_serializing_if = "is_false")]
    pub has_nonmana_activated: bool,
    /// Has NO mana ability ("artifacts without mana abilities": Midnight Arsonist).
    #[serde(default, skip_serializing_if = "is_false")]
    pub no_mana_ability: bool,
    /// Has an activated ability whose cost includes {T} ("artifact with a {T} ability": Magewright's Stone).
    #[serde(default, skip_serializing_if = "is_false")]
    pub has_tap_ability: bool,
    /// An Aura card whose restriction is plain "enchant creature" (Attach to exactly Creature, no other conditions): Tallowisp.
    #[serde(default, skip_serializing_if = "is_false")]
    pub enchants_creature: bool,
    /// A permanent with fading (it has the fade-counter entry ability of a fading card): Parallax Inhibitor.
    #[serde(default, skip_serializing_if = "is_false")]
    pub fading: bool,
    /// A spell (or permanent) for which mana from an artifact source was spent to cast it (Shadow the Hedgehog's Chaos Control).
    #[serde(default, skip_serializing_if = "is_false")]
    pub artifact_mana_spent: bool,
    /// The permanent shares a color with the mana spent to activate the source's most recent activated ability (Protective Sphere).
    #[serde(default, skip_serializing_if = "is_false")]
    pub shares_color_with_activation_mana: bool,
    /// The card's mana cost could be paid by some amount of (or all of) the mana spent to activate the source's most recent activated ability (Illusionary Mask).
    #[serde(default, skip_serializing_if = "is_false")]
    pub payable_by_activation_mana: bool,
    /// The creature attacked this turn because it was required to (goaded or otherwise forced; Firkraag, Cunning Instigator).
    #[serde(default, skip_serializing_if = "is_false")]
    pub had_to_attack: bool,
    /// The source's controller put a counter on this permanent this turn (La Forge, Thoughtful as Ever).
    #[serde(default, skip_serializing_if = "is_false")]
    pub counter_put_by_you_this_turn: bool,
    /// A prepare spell copy cast from exile (Codie, Ravenous Codex: "a prepared spell"); `Card.cast_prepared`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub prepared_spell: bool,
    /// Shares at least one CARD type (artifact, creature, instant ...) with any of the selected cards ("a card that shares a card type with that spell": `{"type":"TriggerObject"}`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shares_card_type_with: Option<Box<Sel>>,
    /// Could produce at least one type of mana (W U B R G C) that any of the selected permanents could produce, over all their mana abilities (Mana Web: `{"type":"TriggerObject"}`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub could_produce_mana_of: Option<Box<Sel>>,
    /// An Aura card whose "enchant ..." restriction (`Ability::Attach`) lets it legally enchant at least one of the selected permanents (Academy Researchers, Danitha, Aura Graft).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub can_enchant: Option<Box<Sel>>,
    /// At least one of these card types is PRINTED on the card (looks through face-down: Hauntwoods Shrieker's "if it's a creature card").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub printed_types: Vec<CardType>,
    /// None of these card types is PRINTED on the card (layer-independent: "each noncreature artifact is an artifact creature with P/T equal to its mana value" must keep matching after the type change).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub not_printed_types: Vec<CardType>,
    /// None of these subtypes is PRINTED on the card (layer-independent: Bludgeon Brawl's "non-Equipment artifact" must keep matching after the Equipment type is added).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub not_printed_subtypes: Vec<String>,
    /// At least one of these subtypes is PRINTED on the card ("if it's a Samurai card": Oathkeeper still returns a Samurai that Lignify made a Treefolk).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub printed_subtypes: Vec<String>,
    /// A permanent that at least one of the selected Auras could legally enchant ("attach it to another permanent it can enchant": Aura Graft, Simic Guildmage).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enchantable_by: Option<Box<Sel>>,
    /// The permanent's controller is one of these players (`{"type":"DefendingPlayer"}`, `{"type":"TriggerPlayer"}`, a targeted player ...):
    /// "target creature defending player controls", "creature controlled by the damaged player".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controlled_by: Option<Box<Sel>>,
    /// Controlled by a player NOT among the selected players.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_controlled_by: Option<Box<Sel>>,
    /// The card's owner is one of these players (`{"type":"EachObject"}` inside `ForEachPlayer`: "all permanents they own", The Great Aurora).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owned_by: Option<Box<Sel>>,
    /// The permanent's defender side (a battle's protector, else its controller) is one of these players: "planeswalker that player controls or battle that player protects" (Rampaging Raptor).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub defended_by: Option<Box<Sel>>,
    /// Only cards in the current Remembered set ("choose one of the cards milled this way").
    #[serde(default, skip_serializing_if = "is_false")]
    pub among_remembered: bool,
    /// Not the object of the trigger event being targeted for ("return up to one OTHER target creature card" from a dies trigger: Illicit Masquerade).
    #[serde(default, skip_serializing_if = "is_false")]
    pub not_trigger_object: bool,
    /// Only cards NOT in the current Remembered set ("each nonland permanent that wasn't chosen"; combine with `zone` filters in `All`).
    #[serde(default, skip_serializing_if = "is_false")]
    pub not_among_remembered: bool,
    /// Is one of the cards saved in this stored slot (`StoreRemembered`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub among_stored: Option<u8>,
    /// Is NOT one of the cards saved in this stored slot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_among_stored: Option<u8>,
    /// An attacking creature that no creature has blocked (Ninjutsu).
    #[serde(default, skip_serializing_if = "is_false")]
    pub unblocked: bool,
    /// Attacking creatures that have been blocked.
    #[serde(default, skip_serializing_if = "is_false")]
    pub blocked_attacker: bool,
    /// Fought this turn.
    #[serde(default, skip_serializing_if = "is_false")]
    pub fought_this_turn: bool,
    /// Entered the battlefield from this zone ("entered from a graveyard").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entered_from: Option<Zone>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub enchanted_or_equipped: bool,
    /// Has an Aura (or other enchantment) attached.
    #[serde(default, skip_serializing_if = "is_false")]
    pub enchanted: bool,
    /// Has a permanent matching this filter attached (relative to the source: "enchanted by an Aura you control" = `{subtypes:[Aura], controller:You}`, "another Aura attached" = `{subtypes:[Aura], other:true}`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_attached: Option<Box<Filter>>,
    /// Has no Aura (or other enchantment) attached (Winds of Rath, Time Elemental).
    #[serde(default, skip_serializing_if = "is_false")]
    pub not_enchanted: bool,
    /// Has an Equipment attached.
    #[serde(default, skip_serializing_if = "is_false")]
    pub equipped: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub damaged: bool,
    /// Was dealt damage this turn by the source of the ability ("a creature dealt damage by this creature this turn dies"). Also works for a dying creature.
    #[serde(default, skip_serializing_if = "is_false")]
    pub damaged_by_source: bool,
    /// Was dealt damage this turn by any source the ability's controller controlled ("a creature dealt damage this turn by a source you controlled": Etching of Kumano). Also works for a dying creature.
    #[serde(default, skip_serializing_if = "is_false")]
    pub damaged_by_your_source: bool,
    /// Was dealt damage this turn by a source the ability's controller controlled that matches this filter ("dealt damage this turn by a Spider you controlled": Shelob, Child of Ungoliant). Also works for a dying creature.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub damaged_by_your_matching: Option<Box<Filter>>,
    /// Was dealt damage this turn by the permanent the source is attached to ("a creature dealt damage by enchanted creature this turn").
    #[serde(default, skip_serializing_if = "is_false")]
    pub damaged_by_attached: bool,
    /// Dealt damage this turn to the source of the ability ("each creature that dealt damage to this creature this turn": Giant Albatross; works when the source has died).
    #[serde(default, skip_serializing_if = "is_false")]
    pub dealt_damage_to_source: bool,
    /// Dealt combat damage this turn (to any player or creature).
    #[serde(default, skip_serializing_if = "is_false")]
    pub dealt_combat_damage_this_turn: bool,
    /// Dealt any damage this turn (combat or not, to anything): "a creature that dealt damage this turn".
    #[serde(default, skip_serializing_if = "is_false")]
    pub dealt_damage_this_turn: bool,
    /// The creature's own toughness is greater than its own power (Bedrock Tortoise, Immobilizer Eldrazi).
    #[serde(default, skip_serializing_if = "is_false")]
    pub toughness_gt_power: bool,
    /// The permanent was ever dealt damage by the source this game (The Fallen's planeswalkers).
    #[serde(default, skip_serializing_if = "is_false")]
    pub damaged_by_source_ever: bool,
    /// A card in a graveyard that is one of the topmost N cards (most recently put there) of its owner's graveyard that match the rest of this filter ("exile the top card of your graveyard": 1; Spinning Darkness: black cards, 3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_of_graveyard: Option<u8>,
    /// The creature's current power is greater than its base (printed) power (Baird, Kutzil).
    #[serde(default, skip_serializing_if = "is_false")]
    pub power_gt_base: bool,
    /// The creature's current (or last-known) power differs from its base (printed) power (Jason Bright).
    #[serde(default, skip_serializing_if = "is_false")]
    pub power_ne_base: bool,
    /// Was dealt damage this turn by any source (even if it has healed or regenerated since).
    #[serde(default, skip_serializing_if = "is_false")]
    pub was_dealt_damage_this_turn: bool,
    /// Was dealt noncombat damage this turn (Grisly Sigil).
    #[serde(default, skip_serializing_if = "is_false")]
    pub was_dealt_noncombat_damage_this_turn: bool,
    /// Was dealt excess damage (more than lethal) this turn.
    #[serde(default, skip_serializing_if = "is_false")]
    pub was_dealt_excess_damage_this_turn: bool,
    /// Dealt damage to the source's controller this turn ("a creature that dealt damage to you this turn").
    #[serde(default, skip_serializing_if = "is_false")]
    pub dealt_damage_to_you_this_turn: bool,
    /// Dealt damage this turn to a player who is an opponent of the source's controller now (Whirling Dervish: control changes matter).
    #[serde(default, skip_serializing_if = "is_false")]
    pub dealt_damage_to_opponent_this_turn: bool,
    /// Dealt combat damage to the source's controller this turn.
    #[serde(default, skip_serializing_if = "is_false")]
    pub dealt_combat_damage_to_you_this_turn: bool,
    /// Dealt combat damage to a player this turn (Glen Elendra).
    #[serde(default, skip_serializing_if = "is_false")]
    pub dealt_combat_damage_to_player_this_turn: bool,
    /// Dealt combat damage to a player during the current combat phase ("this combat": Glen Elendra; differs from "this turn" with extra combats).
    #[serde(default, skip_serializing_if = "is_false")]
    pub dealt_combat_damage_to_player_this_combat: bool,
    /// Blocked this turn.
    #[serde(default, skip_serializing_if = "is_false")]
    pub blocked_this_turn: bool,
    /// Did NOT block this turn.
    #[serde(default, skip_serializing_if = "is_false")]
    pub not_blocked_this_turn: bool,
    /// Was blocked this turn (an attacker that a creature blocked).
    #[serde(default, skip_serializing_if = "is_false")]
    pub was_blocked_this_turn: bool,
    /// Blocked or was blocked by a creature matching this filter this turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_pair_this_turn: Option<Box<Filter>>,
    /// Is currently blocking an attacking creature that matches this filter (Sting, the Glinting Dagger).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocking_matching: Option<Box<Filter>>,
    /// Is an attacking creature currently blocked by a creature matching this filter (Sting, the Glinting Dagger).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_by_matching: Option<Box<Filter>>,
    /// Currently has this exact ability, printed or granted (Musician: "if it doesn't have that ability, it gains it", with `Not`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_ability: Option<Box<Ability>>,
    /// Was turned face up this turn.
    #[serde(default, skip_serializing_if = "is_false")]
    pub turned_face_up_this_turn: bool,
    /// Enlisted a creature this turn (Aradesh, the Founder).
    #[serde(default, skip_serializing_if = "is_false")]
    pub enlisted_this_turn: bool,
    /// Entered the battlefield this turn.
    #[serde(default, skip_serializing_if = "is_false")]
    pub entered_this_turn: bool,
    /// Entered the battlefield without being played (a land that was put onto the battlefield by an effect: Deep Gnome Terramancer).
    #[serde(default, skip_serializing_if = "is_false")]
    pub not_played: bool,
    /// Not being moved right now by an effect of this source ("if it wasn't put onto the battlefield with this ability": Kodama of the East Tree; read while the zone-change event is handled).
    #[serde(default, skip_serializing_if = "is_false")]
    pub not_put_by_this: bool,
    /// Entered the battlefield since the end of its source controller's previous turn (Premature Burial).
    #[serde(default, skip_serializing_if = "is_false")]
    pub entered_since_your_last_turn_ended: bool,
    /// A card that was put into a graveyard by surveilling this turn and has stayed there (Eye of Duskmantle).
    #[serde(default, skip_serializing_if = "is_false")]
    pub surveilled_this_turn: bool,
    /// A card that was put into a graveyard this turn (and may since have left it).
    #[serde(default, skip_serializing_if = "is_false")]
    pub put_in_graveyard_this_turn: bool,
    /// ... and it came from the battlefield.
    #[serde(default, skip_serializing_if = "is_false")]
    pub put_in_graveyard_from_battlefield_this_turn: bool,
    /// ... and it came from somewhere OTHER than the battlefield (hand, library, stack).
    #[serde(default, skip_serializing_if = "is_false")]
    pub put_in_graveyard_not_from_battlefield_this_turn: bool,
    /// A card that was put into a graveyard from a library this turn (milled; "cards milled this turn").
    #[serde(default, skip_serializing_if = "is_false")]
    pub put_in_graveyard_from_library_this_turn: bool,
    /// A card that was milled (by an instruction using "mill") this turn and is still in the graveyard it went to (Tato Farmer).
    #[serde(default, skip_serializing_if = "is_false")]
    pub milled_this_turn: bool,
    /// A card that was NOT put into a graveyard during this combat phase (Storrev: "that wasn't put there this combat").
    #[serde(default, skip_serializing_if = "is_false")]
    pub not_put_in_graveyard_this_combat: bool,
    /// A card discarded this turn (also matches cycled cards).
    #[serde(default, skip_serializing_if = "is_false")]
    pub discarded_this_turn: bool,
    /// The creature could use {T} abilities (it is not summoning sick): enlist.
    #[serde(default, skip_serializing_if = "is_false")]
    pub not_summoning_sick: bool,
    /// A spell with exactly one target (Deflection, Swerve, Ricochet Trap: "target spell with a single target").
    #[serde(default, skip_serializing_if = "is_false")]
    pub single_target: bool,
    /// The card was put into exile this turn (Maralen, Fae Ascendant).
    #[serde(default, skip_serializing_if = "is_false")]
    pub exiled_this_turn: bool,
    /// A spell or ability (`TargetKind::StackObject`) whose only target is a single permanent or player (Radiant Performer).
    #[serde(default, skip_serializing_if = "is_false")]
    pub single_permanent_or_player_target: bool,
    /// A spell on the stack whose only target is a single opponent of the source's controller or a single permanent an opponent of the source's controller controls (Zevlor, Elturel Exile).
    #[serde(default, skip_serializing_if = "is_false")]
    pub spell_targets_single_opp_object: bool,
    /// An Aura / Curse attached to the source's controller ("a Curse attached to you": Lynde, Cheerful Tormentor).
    #[serde(default, skip_serializing_if = "is_false")]
    pub enchanting_you: bool,
    /// The card most recently exiled with the source ("the last card exiled with this artifact": Ice Cauldron).
    #[serde(default, skip_serializing_if = "is_false")]
    pub last_exiled_with_this: bool,
    /// A spell on the stack that would destroy a land the source's controller controls if it resolved now: a `Destroy` among its effects aimed at such a land (by target or by an "all" filter) (Equinox).
    #[serde(default, skip_serializing_if = "is_false")]
    pub spell_would_destroy_your_land: bool,
    /// A spell on the stack with at least one target that is a player or a permanent on the battlefield ("if that spell targets a permanent or player": Shiko and Narset, Unified).
    #[serde(default, skip_serializing_if = "is_false")]
    pub spell_targets_permanent_or_player: bool,
    /// A spell on the stack that has at least one target ("a spell with one or more targets": Wyll's Reversal).
    #[serde(default, skip_serializing_if = "is_false")]
    pub spell_has_targets: bool,
    /// A spell on the stack with a target (a permanent/card) matching this filter ("a spell that targets a permanent you control", "targets a commander you control").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spell_targets: Option<Box<Filter>>,
    /// A spell that targets a spell or ability on the stack that itself targets a permanent matching this filter (Not of This World: "targets a spell or ability that targets a creature you control with power 7 or greater").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spell_targets_stack: Option<Box<Filter>>,
    /// A spell on the stack that targets the source's controller ("target spell that targets you": Mirror Sheen).
    #[serde(default, skip_serializing_if = "is_false")]
    pub spell_targets_you: bool,
    /// A spell on the stack that targets an opponent of the source's controller (player target; Danitha, Spear of Agony).
    #[serde(default, skip_serializing_if = "is_false")]
    pub spell_targets_opponent: bool,
    /// A spell on the stack that targets the player the source Aura enchants (Infectious Curse: "spells you cast that target enchanted player").
    #[serde(default, skip_serializing_if = "is_false")]
    pub spell_targets_enchanted_player: bool,
    /// A spell on the stack that has targets and targets only players (Rebound).
    #[serde(default, skip_serializing_if = "is_false")]
    pub spell_targets_only_players: bool,
    /// In a restricted-mana `ability` filter: only when paying to activate an equip ability ("spend this mana only to activate equip abilities").
    #[serde(default, skip_serializing_if = "is_false")]
    pub equip_ability: bool,
    /// In a restricted-mana `ability` filter: only when paying to activate a power-up ability (`Activated.power_up`; Quinjet Technician).
    #[serde(default, skip_serializing_if = "is_false")]
    pub power_up_ability: bool,
    /// In a restricted-mana `ability` filter: only when paying to foretell a card from hand (a special action: Karfell Harbinger).
    #[serde(default, skip_serializing_if = "is_false")]
    pub foretell_ability: bool,
    /// A spell that is not the target of an ability from another source with the same name as the filter's source (Goblin Artisans).
    #[serde(default, skip_serializing_if = "is_false")]
    pub not_targeted_by_same_name_ability: bool,
    /// A card in a graveyard that is the topmost (most recently put there) among the cards of that graveyard matching the rest of this filter (Necratog: "the top creature card of your graveyard").
    #[serde(default, skip_serializing_if = "is_false")]
    pub graveyard_top: bool,
    /// The card is haunted by the source (haunt: "when the creature it haunts dies" - use with `Dies` in a trigger with `from: Exile`).
    #[serde(default, skip_serializing_if = "is_false")]
    pub haunted_by_this: bool,
    /// Was declared as an attacker this turn (also after combat ended).
    #[serde(default, skip_serializing_if = "is_false")]
    pub attacked_this_turn: bool,
    /// Was NOT declared as an attacker this turn.
    #[serde(default, skip_serializing_if = "is_false")]
    pub not_attacked_this_turn: bool,
    /// Some(true): the creature couldn't attack this turn (not its controller's turn, or no legal attack right now: summoning sick, defender, can't-attack effects). Some(false): it could (Season of the Witch).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub couldnt_attack: Option<bool>,
    /// Attacked during its controller's most recent turn (the turn one full round ago; checked before it attacks again this turn).
    #[serde(default, skip_serializing_if = "is_false")]
    pub attacked_last_own_turn: bool,
    /// The creature attacked or blocked since the source controller's last upkeep (this turn's upkeep counts as the last one outside their own turn): Shape of the Wiitigo.
    #[serde(default, skip_serializing_if = "is_false")]
    pub attacked_or_blocked_since_upkeep: bool,
    /// Mana value compared with a computed amount (`X`, a creature's power, life gained ...), evaluated for the ability's source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mana_value_vs: Option<AmountCmp>,
    /// Mana value no greater than the number of cards in the graveyard of this object's own controller (Drown in the Loch: per candidate, not per ability).
    #[serde(default, skip_serializing_if = "is_false")]
    pub mv_le_controller_graveyard: bool,
    /// Mana value is odd / even (Soundwave, Superior Captain: "a spell with an odd mana value"; 0 is even).
    #[serde(default, skip_serializing_if = "is_false")]
    pub mv_odd: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub mv_even: bool,
    /// Power / toughness compared with a computed amount (X, another creature's power, cards in hand ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub power_vs: Option<AmountCmp>,
    /// Number of colors compared with a computed amount (Evolving Door: exactly the stored count plus one).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_count_vs: Option<AmountCmp>,
    /// Power and toughness are not equal (Gilt-Leaf Winnower).
    #[serde(default, skip_serializing_if = "is_false")]
    pub pt_unequal: bool,
    /// You (the source's controller) put a counter of this kind on it this turn (Kid Loki).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub counter_put_by_you: Option<CounterKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toughness_vs: Option<AmountCmp>,
    /// Power plus toughness is at most this (Cut Down: "total power and toughness 5 or less").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_pt_max: Option<i32>,
    /// Has a counter of this kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_counter: Option<CounterKind>,
    /// With `has_counter`: at least this many counters of that kind ("three or more +1/+1 counters"; default 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub counter_min: Option<i32>,
    /// Has at least one counter of any kind.
    #[serde(default, skip_serializing_if = "is_false")]
    pub has_any_counter: bool,
    /// Has no counter of this kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_has_counter: Option<CounterKind>,
    /// The object must have ALL of these types ("artifact creature"); `types` means any of.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub all_types: Vec<CardType>,
    /// Also required: matches at least one of these filters (OR): "attacking or blocking", "basic land or Gate".
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub any_of: Vec<Filter>,
    /// Is the permanent the source (an Aura or Equipment) is attached to ("equipped creature", "enchanted land").
    #[serde(default, skip_serializing_if = "is_false")]
    pub attached_by_source: bool,
    /// Is an Aura / Equipment attached to the source ("Auras attached to this creature", "equipment attached to it"). Count it for "+1/+1 for each Aura attached".
    #[serde(default, skip_serializing_if = "is_false")]
    pub attached_to_source: bool,
    /// Is attached to a permanent matching this filter ("an Aura attached to a creature", "Equipment attached to a land you control").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attached_to_matching: Option<Box<Filter>>,
    /// Is attached to one of the selected permanents ("destroy all Auras attached to that creature": `{"type":"TriggerObject"}`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attached_to_sel: Option<Box<Sel>>,
    /// The card is a spell on the stack, or one being cast from hand (not a permanent or an ability): "protection from spells that are one or more colors".
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_spell: bool,
    /// The card is a spell on the stack that its caster does not own ("a spell they don't own": Gonti, Night Minister).
    #[serde(default, skip_serializing_if = "is_false")]
    pub cast_by_non_owner: bool,
    /// Has the creature type the source chose as it entered ("creatures of the chosen type").
    #[serde(default, skip_serializing_if = "is_false")]
    pub chosen_type: bool,
    /// Is the colour the source chose as it entered.
    #[serde(default, skip_serializing_if = "is_false")]
    pub chosen_color: bool,
    /// Doesn't have the creature type the source chose ("creatures not of the chosen type").
    #[serde(default, skip_serializing_if = "is_false")]
    pub not_chosen_type: bool,
    /// Has any of the creature types chosen with `ChooseCreatureType { accumulate: true }` on this source (this resolution).
    #[serde(default, skip_serializing_if = "is_false")]
    pub chosen_type_any: bool,
    /// Has none of the creature types chosen with `ChooseCreatureType { accumulate: true }` on this source (Harsh Mercy).
    #[serde(default, skip_serializing_if = "is_false")]
    pub not_chosen_type_any: bool,
    /// Isn't the colour the source chose.
    #[serde(default, skip_serializing_if = "is_false")]
    pub not_chosen_color: bool,
    /// Isn't named this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_name: Option<String>,
    /// Doesn't share a name with a Curse attached to the source's enchanted player (Curse of Misfortunes: `SearchLibrary`).
    #[serde(default, skip_serializing_if = "is_false")]
    pub name_not_curse_on_enchanted: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "op", deny_unknown_fields)]
pub enum Cmp {
    Eq { n: i32 },
    Le { n: i32 },
    Ge { n: i32 },
    Lt { n: i32 },
    Gt { n: i32 },
}

impl Cmp {
    pub fn test(&self, v: i32) -> bool {
        match *self {
            Cmp::Eq { n } => v == n,
            Cmp::Le { n } => v <= n,
            Cmp::Ge { n } => v >= n,
            Cmp::Lt { n } => v < n,
            Cmp::Gt { n } => v > n,
        }
    }
}

// ---------------------------------------------------------------- amounts and conditions

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum Amount {
    Fixed { n: i32 },
    /// The result of the most recent die roll.
    DieRoll,
    /// The result chosen by the last `RollTwoChooseOne`.
    RollChosen,
    /// The result not chosen by the last `RollTwoChooseOne`.
    RollOther,
    /// The X chosen when casting / activating.
    X,
    /// The X of the spell that caused this trigger ("whenever you cast a spell with {X} in its mana cost ... X counters": Zaxara).
    TriggerSpellX,
    /// The number of modes chosen for the spell that fired a cast trigger ("the number of times you chose a mode for that spell": Riku of Many Paths).
    TriggerSpellModes,
    /// A player's counters of one kind (energy / poison / experience).
    PlayerCounters { who: Sel, kind: PlayerCounterKind },
    /// The SUM of the selected players' counters of a kind (Vishgraz: "for each poison counter your opponents have").
    PlayerCountersTotal { who: Sel, kind: PlayerCounterKind },
    /// How many of the selected players have at least `at_least` counters of a kind (Glissa's Retriever: opponents with three or more poison counters).
    PlayersWithCounters { who: Sel, kind: PlayerCounterKind, at_least: i32 },
    /// How many times the spell was kicked (multikicker); 1 for a single kicker.
    KickCount,
    /// How many times the `index`-th kicker ability (in printed order) was paid (Primitive Justice: "for each additional {1}{G} you paid").
    KickCountAt { index: u8 },
    /// A player's speed (0 to 4).
    Speed { who: Sel },
    /// Party size: how many of Cleric, Rogue, Warrior, Wizard are represented among the creatures the player controls (0..=4).
    PartySize { who: Sel },
    /// How many objects are `Remembered` (the creatures sacrificed to devour, the cards discarded this way ...).
    RememberedCount,
    /// How many cards were drawn since the last `Effect::StartDrawCount` ("a card drawn this way": Explosion of Riches).
    DrawnSinceStart,
    /// How many `Remembered` cards have the same mana value as another `Remembered` card (Vault 21: House Gambit: a Treasure for each such revealed card).
    RememberedSharedManaValue,
    /// Permanents matching `filter` (its controller test is relative to the leaving permanent's controller) that were on the battlefield when the triggering permanent left it, together with those leaving in the same batch (needs `Ability::LkiSnapshot` on the card; 0 otherwise): soulshift X for Kodama of the Center Tree.
    LkiCount { filter: FilterBox },
    /// Cards in `of` that left the battlefield this turn under the control of one of these players (Deadly Tempest: `of` a stored slot, `who` the iterated player).
    /// `filter`: only cards matching it count ("for each land sacrificed this way": Wave of Vitriol).
    LeftBattlefieldControlledBy { of: Sel, who: Sel, #[serde(default, skip_serializing_if = "Option::is_none")] filter: Option<Box<Filter>> },
    /// How many permanents the selected card's last "sacrifice any number" sacrificed (readable from its other abilities: Devour).
    SacrificedAnyCount { of: Sel },
    /// The total power / toughness (as they were when sacrificed) of the permanents the selected card's last "sacrifice any number" sacrificed (Dracoplasm).
    SacrificedAnyTotalPower { of: Sel },
    SacrificedAnyTotalToughness { of: Sel },
    /// Coin flips won since the last `Effect::ResetFlipTally`.
    FlipsWon,
    /// Dungeons the player has completed.
    DungeonsCompleted { who: Sel },
    /// Differently named dungeons the player has completed (Ellywick Tumblestrum's emblem).
    DistinctDungeonsCompleted { who: Sel },
    /// Votes the option (0-based index into the last `Vote`'s options) received.
    Votes { option: u8 },
    /// The votes the selected players received in the last `Effect::VoteForPlayers` (summed).
    VotesReceived { who: Sel },
    /// The number the first selected player recorded with `ChooseNumber { record }` (0 if none).
    PlayerNumber { who: Sel },
    /// The highest number recorded with `ChooseNumber { record }` (0 if none).
    HighestPlayerNumber,
    /// The votes the selected cards received in the last `Effect::VoteForCards` (summed).
    CardVotes { of: Sel },
    /// Mana spent to cast the selected spell / permanent (0 when it was not cast or cast for free).
    ManaSpent { of: Sel },
    /// Mana of one type (W, U, B, R, G or C) spent to cast the selected spell / permanent ("adamant", "if {B} was spent", "for each {R} spent").
    ManaSpentOf { of: Sel, mana: crate::mana::ManaType },
    /// Mana of the listed types spent on the most recent "unless ... pays" payment (cumulative upkeep: "for each {B} or {R} spent this way").
    LastPaymentManaSpent { mana: Vec<crate::mana::ManaType> },
    /// How many snow sources paid for the spell ("if at least {S}{S} was spent", Graven Lore): each snow permanent tapped for mana counts once.
    SnowManaSpent { of: Sel },
    /// How many of the spell's colors had snow-source mana of that color spent on it (Boreal Outrider).
    SnowColorsMatching { of: Sel },
    /// What a creature adds when tapped to station: its power, or its toughness if it has that option and toughness is greater (Tapestry Warden).
    StationValue { of: Sel },
    /// How many mana of the spell's cost came from sources of one kind (Marut: Treasures; Coin of Mastery: artifacts; Bat Colony: Caves); each source tapped counts once.
    ManaSpentFromSource { of: Sel, source: ManaSourceKind },
    /// Number of different colors among the mana spent to cast the selected spell / permanent (converge, sunburst).
    ColorsSpent { of: Sel },
    /// Distinct colors of mana spent to activate the source's most recent activated ability (Paragon of Modernity).
    ActivationColorsSpent,
    /// How many colors the selected card / spell / permanent is (last known for one that left): Crystal, Ancient Cornucopia, Evolving Door.
    ColorsOf { of: Sel },
    /// Number of creatures tapped to convoke the selected spell / permanent (Ancient Imperiosaur, Knight-Errant of Eos).
    ConvokedCount { of: Sel },
    /// Number of permanents (or cards in a zone) matching a filter.
    /// Count matching cards. `player`: count only those controlled (permanents) / owned (other zones) by these players instead of using the
    /// filter's controller ("lands the defending player controls", "creatures target player controls").
    Count { filter: FilterBox, #[serde(default, skip_serializing_if = "Option::is_none")] zone: Option<Zone>, #[serde(default, skip_serializing_if = "Option::is_none")] player: Option<Sel> },
    /// The most matching cards any ONE of the selected players has (controlled for permanents, owned in other zones): "an opponent controls more lands than you" is
    /// `Compare` of this with `who: EachOpponent` against your own `Count`. (A plain `Count` with the filter's controller `Opponent` adds all opponents together.)
    CountMaxBy { filter: FilterBox, #[serde(default, skip_serializing_if = "Option::is_none")] zone: Option<Zone>, who: Sel },
    PowerOf { of: Sel },
    /// The base (copiable) power of the first selected card: its printed power, or that of the card it is a copy of; no counters or effects (Curie, Emergent Intelligence).
    BasePowerOf { of: Sel },
    /// The total toxic value of the first selected permanent: its toxic keywords added up (Goliath Hatchery's corrupted draw).
    ToxicValue { of: Sel },
    /// Current power minus printed (base) power of the selected creature, may be negative (Sovereign Okinec Ahau).
    PowerMinusBasePower { of: Sel },
    /// The power of the card revealed / permanent chosen as this spell's `Behold` cost was paid (the best candidate is taken); 0 if none.
    BeheldPower,
    ToughnessOf { of: Sel },
    ManaValueOf { of: Sel },
    LifeOf { who: Sel },
    /// The format's starting life total (20, or 40 in Commander).
    StartingLife,
    /// The number of cards in the first selected player's deck when the game began (Whtz, the Bibliophile).
    StartingDeckSize { who: Sel },
    HandSize { who: Sel },
    /// Cards in the player's hand when their current turn began (Mindstorm Crown).
    HandSizeAtTurnStart { who: Sel },
    /// The untapped lands the player controlled as their turn began (Power Surge).
    UntappedLandsAtTurnStart { who: Sel },
    /// The number chosen on the source.
    ChosenNumber,
    GraveyardSize { who: Sel },
    /// Creatures that entered the battlefield under these players' control this turn / during the previous turn.
    CreaturesEnteredThisTurn { who: Sel },
    /// `other`: not counting the source itself ("another creature entered last turn": Ephara).
    CreaturesEnteredLastTurn { who: Sel, #[serde(default, skip_serializing_if = "std::ops::Not::not")] other: bool },
    /// The largest graveyard among the selected players ("an opponent has eight or more cards in their graveyard": compare to 8).
    GraveyardSizeMax { who: Sel },
    /// The largest hand among the selected players (Whispering Madness: the greatest number of cards a player discards from a whole hand).
    HandSizeMax { who: Sel },
    /// The smallest hand among the selected players ("an opponent has no cards in hand": compare to 0).
    HandSizeMin { who: Sel },
    /// All the selected players' hands added together ("the total number of cards in all players' hands": Realm Seekers).
    HandSizeTotal { who: Sel },
    /// The highest life total among the selected players ("an opponent has more life than you": compare to your life).
    LifeOfMax { who: Sel },
    /// The lowest life total among the selected players ("you have more life than an opponent").
    LifeOfMin { who: Sel },
    /// Counters of every kind on the selected players added together (Lumbering Megasloth: "each counter among players and permanents").
    AllPlayerCounters { who: Sel },
    /// The total life the selected players paid with the last `Effect::PayLifeRounds` (use inside `ForEachPlayer` with `EachObject`).
    LifePaidRounds { who: Sel },
    /// The total the selected players have recorded with `Effect::PayTally` (Liege of the Hollows: the mana each player paid).
    PayTally { who: Sel },
    CountersOn { of: Sel, kind: CounterKind },
    /// Counters of every kind on all the selected permanents added together (Nardole, Luxior, Ferrafor).
    AllCountersOn { of: Sel },
    /// The printed bushido value of the selected creatures, summed: each "whenever this blocks" trigger of the card that pumps itself by a fixed amount counts as that many points (bushido N is written as such a pair of triggers; Takeno, Samurai General).
    BushidoValue { of: Sel },
    /// 2 raised to the power of the amount (Mathemagics), capped at 2^30.
    PowerOfTwo { of: Box<Amount> },
    /// Coin flips made since the last `ResetFlipTally` (Game of Chaos: the stake is 2 to the power of this minus one).
    CoinFlipsInSeries,
    /// What the player bid or hid in the last `Effect::BidLife` / `Effect::HideNumbers`, by player (summed over the selected players; Pain's Reward, Goblin Game).
    BidOf { who: Sel },    /// How many cards the selector picks (Mantle of the Ancients: `AttachmentsOf`).
    CountSel { of: Sel },
    /// Damage / life / amount of the event that triggered this ability.
    EventAmount,
    Plus { a: Box<Amount>, b: Box<Amount> },
    Minus { a: Box<Amount>, b: Box<Amount> },
    Times { a: Box<Amount>, b: Box<Amount> },
    Half { a: Box<Amount>, round_up: bool },
    /// `a` divided by `n`, rounded down ("extra turns equal to counters removed divided by five").
    DivideBy { a: Box<Amount>, n: i32 },
    /// `a` modulo `n` (non-negative): `Modulo{a, 2}` is 0 for even, 1 for odd.
    Modulo { a: Box<Amount>, n: i32 },
    /// Total power of the remembered permanents / their total toughness (sum over `Remembered`).
    RememberedTotalPower,
    /// Total last-known power of the cards in the batch that fired a batched `ChangesZone` trigger from the battlefield (The Skullspore Nexus).
    EventBatchPower,
    /// Total power of the attackers in a batched attack trigger: the current power of those still on the battlefield, the power they had when they attacked for those that left (Vulpine Harvester).
    EventAttackersPower,
    /// The greatest mana value among the selected cards (`Stored` / `EventCards` / `Remembered`).
    HighestManaValueOf { of: Sel },
    /// The greatest mana value among the cards put into exile this turn since the source entered the battlefield, 0 if none (Bell Borca, Spectral Sergeant: "the greatest number noted for it this turn").
    NotedExileMv,
    /// The lowest mana value among the selected cards (Stronghold Gambit).
    LowestManaValueOf { of: Sel },
    RememberedTotalToughness,
    /// Number of differently named tokens among the permanents matching `filter` (a creature token is named for its creature types, any other token by its name): Neriv, Crackling Vanguard.
    DistinctTokenNames { filter: FilterBox },
    /// Total power / toughness of the cards exiled by this permanent's `EntersExileAny` as it entered (Sutured Ghoul).
    EnterExiledPower,
    EnterExiledToughness,
    /// Sum of the mana values of the remembered cards, wherever they are now (discarded as a cost: Nahiri's Wrath).
    RememberedTotalManaValue,
    /// The mana value of the permanent most recently sacrificed to pay a cost of this ability (0 when none; Slobad, Iron Goblin). In a mana ability that sacrifices, the most valuable eligible permanent is sacrificed.
    SacrificedCostManaValue,
    Max { a: Box<Amount>, b: Box<Amount> },
    Min { a: Box<Amount>, b: Box<Amount> },
    /// Highest / lowest value among the filtered permanents.
    /// The greatest power among the cards matching `filter` (on the battlefield, or in `zone` for "creature cards in your graveyard").
    HighestPower {
        filter: FilterBox,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        zone: Option<Zone>,
    },
    /// The greatest power among the creatures the spell's caster controlled as it was cast (Lifestream's Blessing); 0 for anything but a spell.
    GreatestPowerAtCast,
    HighestToughness {
        filter: FilterBox,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        zone: Option<Zone>,
    },
    /// Greatest mana value among the matching cards on the battlefield, or in `zone` ("among cards in your graveyard").
    HighestManaValue {
        filter: FilterBox,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        zone: Option<Zone>,
    },
    /// The least power among the cards matching `filter` (0 when none).
    LowestPower {
        filter: FilterBox,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        zone: Option<Zone>,
    },
    /// The least toughness among the cards matching `filter` (0 when none; The Dining Car).
    LowestToughness { filter: FilterBox },
    /// The least mana value among the permanents matching `filter` (0 when none; Culling Scales).
    LowestManaValue { filter: FilterBox },
    /// How many different colors the permanents matching `filter` have among them, 0-5 (Conqueror's Flail).
    /// With `spells_cast`, also counts the colors of the spells the controller has cast this turn (First Family).
    DistinctColors {
        filter: FilterBox,
        #[serde(default, skip_serializing_if = "is_false")]
        spells_cast: bool,
    },
    TotalPower { filter: FilterBox },
    /// Total toughness of the matching permanents (Orysa: creatures you control with total toughness 10 or greater).
    TotalToughness { filter: FilterBox },
    CardsDrawnThisTurn { who: Sel },
    /// Lands these players have played this turn (not lands put onto the battlefield by effects): Fastbond.
    LandsPlayedThisTurn { who: Sel },
    LandsEnteredThisTurn { who: Sel },
    /// Counters of this kind you put on creatures under your control this turn (even if those creatures left since).
    CountersPutThisTurn { kind: CounterKind },
    LifeGainedThisTurn { who: Sel },
    /// How many separate life-gain events the selected players have had this turn (Attended Healer: "first time each turn" = events == 1 when the trigger is checked).
    LifeGainEventsThisTurn { who: Sel },
    /// Permanents that entered the battlefield under the selected players' control this turn (even if they have left since), optionally only these card types / without these types.
    /// How many cards the most recent `RevealUntil` revealed (the found cards and the rest).
    RevealedCount,
    /// Cards in the controller's graveyard with the same name as the spell whose cost is being reduced (Locket of Yesterdays; only meaningful in `CostChange.per`).
    SameNameInGraveyardAsSpell,
    EnteredThisTurn {
        who: Sel,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        types: Vec<CardType>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        not_types: Vec<CardType>,
        /// Only permanents with at least one of these subtypes (Cleaving Reaper: Angel or Berserker; Cloudspire Coordinator: Mount or Vehicle).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        subtypes: Vec<String>,
        /// Not counting the source itself ("another Elf entered").
        #[serde(default, skip_serializing_if = "is_false")]
        other: bool,
        /// Only nontoken permanents (Gyome, Master Chef).
        #[serde(default, skip_serializing_if = "is_false")]
        nontoken: bool,
        /// Only permanents of at least one of these (printed) colors (Permafrost Trap: green creatures).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        colors: Vec<Color>,
        /// Only permanents with at least one of these (printed) keywords (Flying Drone: another creature with flying).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        keywords: Vec<Keyword>,
    },
    /// Creatures that attacked this turn under the selected players' control (even if they have left since).
    AttackedThisTurn {
        who: Sel,
        /// Count only tokens ("you attacked with a token this turn": Neyali, Suns' Vanguard).
        #[serde(default, skip_serializing_if = "is_false")]
        token: bool,
        /// Only creatures with one of these subtypes (Deepway Navigator: Merfolk).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        subtypes: Vec<String>,
        /// Count only commanders ("during any turn you attacked with a commander": Neriv, Crackling Vanguard).
        #[serde(default, skip_serializing_if = "is_false")]
        commander: bool,
    },
    /// Life lost this turn (damage included), summed over the selected players.
    LifeLostThisTurn { who: Sel },
    /// Damage dealt to these players this turn (all sources; unlike LifeLostThisTurn it ignores infect/life-loss that isn't damage).
    DamageTakenThisTurn { who: Sel },
    /// Unspent mana in the selected players' pools (of one `color`, or all types when absent): Omnath, Locus of Mana.
    /// The greatest mana value among the spells matching `filter` cast this turn (by you unless the filter's controller says otherwise): Rootha.
    HighestManaValueSpellsCast { filter: FilterBox },
    /// The total mana value of the spells matching `filter` cast this turn (by you unless the filter's controller says otherwise; `other: true` leaves out the source): Call Forth the Tempest.
    TotalManaValueSpellsCast { filter: FilterBox },
    /// How many spells matching `filter` the selected players cast this turn (Magebane Lizard: noncreature spells "they've" cast).
    SpellsCastByWhere { who: Sel, filter: FilterBox },
    /// Damage dealt this turn (after prevention) to the first selected permanent by sources the first selected player controlled when they dealt it (Grothama, All-Devouring).
    DamageToPermanentByPlayer { of: Sel, by: Sel },
    /// Damage dealt this turn to the selected players (after prevention); `combat` limits it to combat / noncombat damage.
    DamageToPlayersThisTurn {
        who: Sel,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        combat: Option<bool>,
        /// Only damage from sources matching this filter (Reverse Polarity: artifacts).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<FilterBox>,
    },
    /// The X of the spell on the stack that `of` selects (a triggering spell).
    XOf { of: Sel },
    /// `amount` worked out once for each selected player (with "you" = that player) and combined: the highest by default (Adamaro: the opponent with the
    /// most cards in hand; Arbiter of Knollridge: the highest life total; Windfall: the greatest hand size), or the lowest / the sum.
    OverPlayers {
        who: Sel,
        amount: Box<Amount>,
        #[serde(default)]
        how: PlayerAgg,
    },
    /// How many of the selected players match a player filter (Gnoll War Band: opponents dealt damage this turn; Tymna: opponents dealt combat damage).
    PlayersWhere { who: Sel, filter: PlayerFilter },
    /// How many different sources matching `from` (by their controller when they dealt it) dealt damage this turn, to the `to` players (any recipient when absent);
    /// `combat` limits it to combat / noncombat damage (Inferno Trap: creatures that dealt damage to you; Case of the Burning Masks).
    DistinctDamageSources {
        #[serde(default)]
        from: FilterBox,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to: Option<Sel>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        combat: Option<bool>,
    },
    /// The most damage a single source matching `from` dealt this turn (Dragon Cultist: "a source you controlled dealt 5 or more damage").
    MostDamageBySingleSource {
        #[serde(default)]
        from: FilterBox,
    },
    /// How many colors the selected card (or spell, by its source card) has.
    ColorCountOf { of: Sel },
    /// How many times the resolving activated / triggered ability (of its source) has resolved this turn, counting this resolution: 2 = "the second time this ability has resolved this turn".
    AbilityResolutionsThisTurn,
    /// The number of targets of the spell being cast (only meaningful in a cost change's `per`: Hinata, Battlefield Thaumaturge).
    /// With `filter`: only the targets that are permanents matching it ("for each creature it targets").
    SpellTargetCount {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filter: Option<FilterBox>,
    },
    /// The number of targets chosen for the spell or ability on the stack that `of` selects (a triggering spell: Voracious Bibliophile, Arcee).
    /// With `filter`: only the distinct permanents among them that match it ("that many +1/+1 counters" for a spell that targets creatures or Vehicles you control: Arcee, Sharpshooter).
    TargetCountOf {
        of: Sel,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filter: Option<FilterBox>,
    },
    /// Total damage dealt this turn (after prevention) by sources matching `from` (their controller when they dealt it is judged against `from.controller`) to anything;
    /// `combat` limits it to combat / noncombat damage (Temple of Power: "red sources you controlled dealt 4 or more noncombat damage this turn").
    DamageDealtBySources {
        from: FilterBox,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        combat: Option<bool>,
    },
    /// Energy these players paid or lost this turn.
    EnergySpentThisTurn { who: Sel },
    ManaInPool { who: Sel, #[serde(default, skip_serializing_if = "Option::is_none")] color: Option<crate::mana::ManaType> },
    /// The total damage dealt to the selected permanent(s) this turn (any source).
    DamageDealtToThisTurn {
        of: Sel,
        /// Only damage from sources matching this filter (Blazing Effigy: "by other sources named Blazing Effigy").
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<FilterBox>,
    },
    /// The excess damage (beyond lethal) dealt to the selected permanent(s) this turn, even if they have died since.
    ExcessDamageDealtToThisTurn { of: Sel },
    /// How many times the selected creature has been declared as an attacker this turn (Moraug, Fury of Akoum; use `each` in a `Modify` static).
    TimesAttackedThisTurn { of: Sel },
    /// How many times the selected permanent has regenerated this turn (Spiny Starfish).
    TimesRegeneratedThisTurn { of: Sel },
    /// The damage actually dealt by the most recent damage event ("gain life equal to the damage dealt this way", "that much damage").
    LastDamageDealt,
    /// The life actually lost by the most recent `LoseLife` effect ("you gain life equal to the life lost this way": extort, with replacement effects applied).
    LifeLostThisWay,
    /// The greatest amount of damage any single source dealt to one permanent or player in one damage event this turn (Impact Resonance).
    GreatestDamageThisTurn,
    /// The toughness of the creature sacrificed to cast this permanent for its emerge cost (Adipose Offspring).
    EmergeSacrificedToughness,
    /// How many creatures crewed this Vehicle this turn, including ones that have since left the battlefield (Luxurious Locomotive).
    CrewedItThisTurn,
    /// How many cards the last surveil left on top of the library (Starving Revenant: "for each card you put on top").
    SurveilKeptOnTop,
    /// How many counters the most recent `RemoveAnyCounters` effect actually removed (Hex Parasite).
    CountersRemoved,
    /// The total damage the source (`This`) has dealt this turn ("you gain life equal to the damage dealt this way" for a spell cast once).
    DamageDealtByThisThisTurn,
    /// The most damage any single sorcery spell cast this turn by the first selected player has dealt this turn (Backdraft: wrap in `Half`).
    GreatestSorceryDamage { who: Sel },
    /// The excess damage (more than lethal for a creature / loyalty for a planeswalker) of the most recent damage event.
    LastExcessDamage,
    /// Number of opponents you have (players still in the game).
    OpponentCount,
    /// Permanents matching `filter` (any controller) attached to the player this permanent enchants ("the number of Curses attached to them": Curse of Thirst).
    AttachedToEnchantedPlayer { filter: FilterBox },
    /// Cards these players discarded this turn ("if you discarded a card this turn": `Compare` this `Ge 1`).
    CardsDiscardedThisTurn { who: Sel },
    /// Cards these players cycled this turn (cycling abilities only; channel and other discard-this abilities do not count): Spellpyre Phoenix.
    CardsCycledThisTurn { who: Sel },
    /// How many times this permanent has mutated (the number of cards merged into it).
    TimesMutated,
    /// Domain: the number of basic land types (Plains, Island, Swamp, Mountain, Forest) among lands the selected players control.
    Domain { who: Sel },
    /// Number of different power values among the matching cards (coven: three or more creatures with different powers).
    DistinctPowers {
        filter: FilterBox,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        zone: Option<Zone>,
    },
    /// Number of different mana values among the matching cards (Lunar Insight).
    DistinctManaValues {
        filter: FilterBox,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        zone: Option<Zone>,
    },
    /// Number of different mana costs (as written: {2}{B} and {1}{B}{B} differ) among the matching cards (Embodiment of Agonies).
    DistinctManaCosts {
        filter: FilterBox,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        zone: Option<Zone>,
    },
    /// Number of different creature types among the matching cards (Valiant Changeling: creatures you control; a Changeling counts for every type present).
    DistinctSubtypes {
        filter: FilterBox,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        zone: Option<Zone>,
    },
    /// The greatest number of matching cards that share one creature type (Skemfar Shadowsage: "the greatest number of creatures you control that have a creature type in common").
    GreatestSharedSubtype {
        filter: FilterBox,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        zone: Option<Zone>,
    },
    /// Total number of counters of every kind on the matching permanents (Hydra Trainer: counters on permanents you control).
    AllCounters { filter: FilterBox },
    /// Turns the player has taken this game, counting the current one (Serra Avenger, Starting Town: "your first, second, or third turn").
    TurnsTaken { who: Sel },
    /// How many players the selector picks (`PlayersWhere{MostCreatures}` == 1: "a player controls more creatures than each other player").
    PlayersIn { who: Sel },
    /// Evaluate `amount` once for each player in `who` (with that player as "you") and take the greatest, or the least when `least`
    /// ("the greatest number of creatures a player controls", "a library has twenty or fewer cards"). 0 for no players.
    ExtremeOverPlayers {
        who: Sel,
        amount: Box<Amount>,
        #[serde(default, skip_serializing_if = "is_false")]
        least: bool,
    },
    /// The greatest number of matching cards that share one name (Sceptre of Eternal Glory: three or more lands with the same name).
    GreatestSharedName {
        filter: FilterBox,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        zone: Option<Zone>,
    },
    /// Number of different card names among the matching cards ("Field of the Dead": lands with different names).
    DistinctNames {
        filter: FilterBox,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        zone: Option<Zone>,
    },
    /// Sum of the mana values of the matching cards.
    TotalManaValue {
        filter: FilterBox,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        zone: Option<Zone>,
    },
    /// Colored mana symbols in the mana cost of the first selected card (chroma of a revealed / milled card); `colors` limits which colors count (empty = any).
    ColorSymbols {
        of: Sel,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        colors: Vec<Color>,
    },
    /// How many creatures are blocking the source creature (rampage), or 0 when it isn't blocked.
    BlockersOfThis,
    /// How many creatures are blocking the triggering attacker (`TriggerSource` of a `BecomesBlocked` trigger); granted rampage (Rapid Fire).
    BlockersOfTrigger,
    /// How many of the source's two chosen colors (`ChooseKind::Color` and `SecondColor`) the spell being cast is (Seal of the Guildpact).
    ChosenColorsOfSpell,
    /// Number of different players being attacked this combat (melee).
    OpponentsAttacked,
    /// Total number of times you have cast your commander(s) from the command zone this game (storm-style "commander casts").
    CommanderCasts,
    /// The greatest mana value among the commanders the selected players own that are on the battlefield or in the command zone (0 for none; Imposing Grandeur).
    GreatestCommanderManaValue { who: Sel },
    /// The greatest power among the permanents the last `Sacrifice` effect of this ability sacrificed (last-known power; Shadowgrange Archfiend).
    GreatestPowerSacrificed,
    /// How many of these keywords at least one matching battlefield permanent has ("the number of abilities from among flying, first strike ... found among creatures you control, counting each once": Odric, Blood-Cursed).
    DistinctKeywords { filter: FilterBox, keywords: Vec<Keyword> },
    /// How many creature types the first selected permanent has (Diligent Zookeeper: "+1/+1 for each of its creature types").
    CreatureTypeCount { of: Sel },
    /// How many card types the selected cards have among them (Polluted Cistern: `of: EventCards`, the batch that triggered).
    DistinctTypesOf { of: Sel },
    DistinctTypes {
        filter: FilterBox,
        /// Count among cards in this zone of every player (battlefield when absent).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        zone: Option<Zone>,
        /// Count only permanent types (artifact, battle, creature, enchantment, land, planeswalker): "permanent types among cards in your graveyard" (Korvold, Gleeful Glutton).
        #[serde(default, skip_serializing_if = "is_false")]
        permanent_only: bool,
    },
    /// How many card types the cards matching any of the `parts` (filter + zone, battlefield when absent) have among them, counting each type once ("card types among permanents you control and/or cards in your graveyard": Happily Ever After).
    DistinctTypesUnion { parts: Vec<(Filter, Option<Zone>)> },
    /// How many creature types are noted on the first selected permanent (`Effect::NoteCreatureType`; Volo's Journal).
    NotedTypes { of: Sel },
    /// Spells the selected players have cast this game (Once Upon a Time: the first spell of the game).
    SpellsCastThisGame { who: Sel },
    /// How many players have lost the game (Rampant Frogantua, Hot Pursuit).
    PlayersLost,
    /// Spells matching `filter`, controlled by a player matching `whose`, countered this turn by a spell or ability controlled by a player matching `by` (both relative to the source's controller): Summoning Trap.
    SpellsCounteredThisTurn {
        #[serde(default)]
        filter: FilterBox,
        #[serde(default = "you_filter")]
        whose: PlayerFilter,
        #[serde(default)]
        by: PlayerFilter,
    },
    /// Number of times a thing happened this turn.
    SpellsCastThisTurn { who: Sel },
    /// The total mana lost to this source's `DrainManaPool { record: true }` effects this turn (Pygmy Hippo).
    ManaDrained,
    /// Spells cast this turn that match `filter` (its `controller` says whose; default: yours): Rionya, Sentinel Tower, Sorcerer Class.
    SpellsCastWhere {
        filter: FilterBox,
        /// Count only spells cast before the triggering spell (Sentinel Tower: "cast before that spell this turn").
        #[serde(default, skip_serializing_if = "is_false")]
        before_trigger: bool,
    },
    /// Spells cast during the previous turn (werewolves): summed over `who`.
    SpellsCastLastTurn { who: Sel },
    /// The most spells any single player cast during the previous turn.
    MostSpellsCastLastTurn,
    /// Creatures that died this turn (all players).
    CreaturesDiedThisTurn,
    /// Dilemmas you faced this turn (modes chosen for your spells and abilities).
    DilemmasFacedThisTurn,
    /// Spells and abilities this turn that made players guess or group cards or permanents into piles (`Effect::NotePileOrGuess`).
    PileGuessAbilitiesThisTurn,
    /// Creatures that died this turn under the ability controller's control.
    YourCreaturesDiedThisTurn,
    /// Number of cards that left your graveyard this turn.
    CardsLeftYourGraveyardThisTurn,
    /// How many different opponents you attacked this turn (their planeswalkers and battles count as attacking them).
    OpponentsYouAttackedThisTurn,
    /// Permanents that left the battlefield this turn (controller when it left matching `whose`, card matching `filter`).
    PermanentsLeftBattlefieldThisTurn {
        #[serde(default)]
        whose: PlayerFilter,
        #[serde(default)]
        filter: FilterBox,
        /// Only cards now in this zone (Vren: `Exile` = "exiled from the battlefield this turn").
        #[serde(default, skip_serializing_if = "Option::is_none")]
        now_in: Option<Zone>,
    },
    /// Creatures that dealt combat damage to a player this turn (controller at that time matching `whose`), counted even if they have since left the battlefield (Excess).
    CreaturesDealtCombatDamageThisTurn {
        #[serde(default)]
        whose: PlayerFilter,
    },
    /// Total last-known power of the creatures that died this turn (controller when it died matching `whose`, card matching `filter`): Genesis of the Daleks.
    DiedThisTurnTotalPower {
        #[serde(default)]
        whose: PlayerFilter,
        #[serde(default)]
        filter: FilterBox,
    },
    /// Creatures that died this turn (controller when it died matching `whose`, card matching `filter`: subtype for the Zubera).
    CreaturesDiedThisTurnWhere {
        #[serde(default)]
        whose: PlayerFilter,
        #[serde(default)]
        filter: FilterBox,
    },
    /// Life lost during the previous turn by these players.
    LifeLostLastTurn { who: Sel },
    /// The selected players' life totals as the current turn began, summed (Sengir, the Dark Baron: `TriggerPlayer`).
    LifeAtTurnStart { who: Sel },
    /// Number of permanents returned from the battlefield to their owner's hand this turn (owner matching `whose` relative to the controller; `filter` on the card).
    PermanentsReturnedToHandThisTurn {
        #[serde(default)]
        whose: PlayerFilter,
        #[serde(default)]
        filter: FilterBox,
    },
    /// Tokens created this turn by players matching `who` ("if you created a token this turn": compare to 0).
    TokensCreatedThisTurn { who: Sel },
    /// Treasure tokens the selected players created this turn (Vazi, Keen Negotiator).
    TreasuresCreatedThisTurn { who: Sel },
    /// How many card types the permanents sacrificed this turn by a player matching `whose` have among them (each type once; types as the card last had them): Korvold, Gleeful Glutton.
    SacrificedTypesThisTurn {
        #[serde(default)]
        whose: PlayerFilter,
    },
    /// Number of permanents sacrificed this turn by a player matching `whose` (relative to the controller) that match `filter` (a Food, a creature ...).
    SacrificedThisTurn {
        #[serde(default)]
        whose: PlayerFilter,
        #[serde(default)]
        filter: FilterBox,
    },
    /// Number of cards put into a graveyard this turn that belong to a player matching `whose` (relative to the controller), optionally only those that came
    /// from the battlefield, and matching `filter` ("nonland permanent cards put into your graveyard from the battlefield this turn").
    CardsPutIntoGraveyardThisTurn {
        #[serde(default)]
        whose: PlayerFilter,
        /// Count the cards of these players instead of `whose` (e.g. the target player: `{"type":"Target","idx":0}`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        of: Option<Sel>,
        #[serde(default)]
        from_battlefield: bool,
        /// With `from_battlefield`: match `whose` against the controller it had when it left, not its owner.
        #[serde(default, skip_serializing_if = "is_false")]
        by_controller: bool,
        /// Also count tokens ("an artifact or creature was put into a graveyard from the battlefield": Ichor Shade).
        #[serde(default, skip_serializing_if = "is_false")]
        tokens: bool,
        #[serde(default)]
        filter: FilterBox,
    },
    /// Number of cards put into exile this turn that belong to a player matching `whose` and match `filter` (Ennis, Debate Moderator).
    CardsPutIntoExileThisTurn {
        #[serde(default)]
        whose: PlayerFilter,
        #[serde(default)]
        filter: FilterBox,
    },
    /// Devotion: mana symbols of these colors in the mana costs of permanents the controller controls.
    Devotion { colors: Vec<Color> },
    /// Devotion to the colour chosen as this permanent entered (Nyx Lotus: add that much mana of the chosen colour).
    DevotionToChosenColor,
    /// Total number of counters of `kind` on all battlefield permanents matching `filter` ("the total number of +1/+1 counters on creatures you control").
    TotalCounters { filter: FilterBox, kind: CounterKind },
    /// Total number of counters of every kind on all battlefield permanents matching `filter` (Lux Artillery).
    TotalCountersAllKinds { filter: FilterBox },
    /// Number of colours in the colour identity of the controller's commander(s) (War Room).
    CommanderColorCount,
    /// Total unlocked doors among battlefield Rooms matching `filter` (e.g. Rooms you control: Rampaging Soulrager).
    UnlockedDoors { filter: FilterBox },
    /// Different names among the unlocked doors of the battlefield Rooms matching `filter` (Central Elevator // Promising Stairs: "eight or more different names among unlocked doors of Rooms you control").
    DistinctDoorNames { filter: FilterBox },
    /// Cards matching `filter` above the source card in its owner's graveyard (more recently put there: Nether Shadow); 0 when the source isn't in a graveyard.
    CardsAboveInGraveyard { filter: FilterBox },
    /// 1 when the card DIRECTLY above the source card in its owner's graveyard matches `filter`, else 0 (Death Spark).
    CardDirectlyAboveInGraveyard { filter: FilterBox },
    /// How many of the controller's opponents were dealt combat damage this turn by a source matching `filter` (Estinien Varlineau: "Estinien or a Dragon"); the source may have left the battlefield since.
    OpponentsDealtCombatDamageBy { filter: FilterBox },
    /// How many different kinds of counters are on the battlefield permanents matching `filter` ("the number of counter kinds among creatures you control").
    DistinctCounterKinds { filter: FilterBox },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum NameKind {
    #[default]
    Any,
    Nonland,
    Creature,
    Artifact,
    /// Not a basic land name.
    NotBasicLand,
    NonartifactNonland,
    /// A noncreature, nonland card name (Council of the Absolute).
    NoncreatureNonland,
    /// A land card name (Petrified Hamlet).
    Land,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum ChooseDistinct {
    Name,
    Power,
    ManaValue,
    /// Different controllers ("two target creatures controlled by different players"): permanents only.
    Controller,
    /// The chosen targets must share a creature type (changelings share every type): Selective Snare, "creatures of the creature type of your choice". Target slots only.
    SharedCreatureType,
    /// The chosen targets must all have the same toughness (V.A.T.S.: "any number of target creatures with equal toughness"). Target slots only.
    SameToughness,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum PlayerRestrict {
    CantCast,
    /// The player can cast only one more spell this turn (Irencrag Feat): spells already cast don't count.
    OneMoreSpell,
    /// The player can't cast creature spells.
    CantCastCreatures,
    /// The player can't cast noncreature spells.
    CantCastNoncreature,
    /// The player can't attack with creatures (Know Evil).
    CantAttack,
    /// The player can't cast spells from anywhere other than their hand (Avatar's Wrath).
    CantCastNotFromHand,
    /// The player can't cast instant or sorcery spells (Abeyance).
    CantCastInstantSorcery,
    /// The player can't activate abilities that aren't mana abilities (Abeyance).
    CantActivateNonMana,
    /// The player can't play lands (the land-play special action only).
    CantPlayLands,
    CantGainLife,
    CantLoseLife,
    /// The player can't become the monarch (Jared Carthalion, True Heir: this turn).
    CantBecomeMonarch,
    /// The player can't play lands or cast spells from their graveyard (Shaman's Trance).
    CantPlayFromGraveyard,
    /// The player may play lands and cast spells from other players' graveyards as though in their own (Shaman's Trance).
    MayPlayOthersGraveyards,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum ExileMark {
    Foretold,
    Plotted,
    Warped,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum Condition {
    /// The source has an activated ability besides the ones granted with this very condition (Tazri, Stalwart Survivor: "Activate only if this creature has another activated ability"); printed or granted, mana abilities included.
    SourceHasOtherActivatedAbility,
    /// It is an extra turn (the active player is not the one whose regular turn this is).
    ExtraTurn,
    /// The controller has begun an extra turn this game (H.M.S. Bounty: "haven't begun an extra turn").
    BegunExtraTurn,
    /// Mana from a Treasure was spent to cast the selected spell ("if mana from a Treasure was spent to cast it").
    ManaFromTreasure { of: Sel },
    /// A pseudo-random pick among `of` options (fixed by the game state while triggers are collected, so abilities sharing one event agree): true when option `pick` (0-based) is the one chosen. Several triggers of one source, each with a different `pick`, make "choose one at random" fixed as the trigger goes on the stack (Umaro, Raging Yeti).
    RandomPick { of: u8, pick: u8 },
    /// The most recent coin flip was won.
    LastFlipWon,
    /// Any coin flip since the last `ResetFlipTally` was lost (Fiery Gambit).
    FlipLost,
    /// The most recent `MayPay` was paid (false when declined or unaffordable): the loop test of "you may pay {B}. If you do, repeat this process" (Shrouded Lore).
    LastCostPaid,
    /// In a `TapsForMana` trigger: a selected card shares a color with the mana the permanent produced (C.A.M.P.: "shares a color with the mana that land produced").
    SharesColorWithEventMana { with: Sel },
    Not { c: Box<Condition> },
    /// This permanent is face down.
    SourceFaceDown,
    /// The source's `chosen_type` (an as-enters choice such as `ChooseKind::KhansDragons`) is exactly `value`.
    ChosenTypeIs { value: String },
    /// This permanent's underlying card is a creature card (a face-down permanent can be turned face up; Etrata, Deadly Fugitive).
    SourceCardIsCreature,
    All { cs: Vec<Condition> },
    AnyOf { cs: Vec<Condition> },
    LifeAtLeast { who: Sel, n: i32 },
    /// Some player was dealt at least `n` combat damage this turn (total over all sources; Sidequest: Play Blitzball).
    PlayerDealtCombatDamage { n: i32 },
    /// Every selected player has waterbent, earthbent, firebent and airbent this turn (`Player.bend_mask`; Avatar Aang).
    BentAllFour { who: Sel },
    /// Every selected player's number from the last `Effect::HideNumbers` is the lowest of all players' numbers (ties count as lowest).
    BidIsLowest { who: Sel },
    /// Every selected player's number from the last `Effect::HideNumbers` is the highest of all players' numbers (Wheel of Misfortune).
    BidIsHighest { who: Sel },
    /// Every selected player activated a mana ability of a land this turn (Desolation: inside `ForEachPlayer` with `Sel::EachObject`).
    TappedLandForManaThisTurn { who: Sel },
    LifeAtMost { who: Sel, n: i32 },
    Control { who: Sel, filter: FilterBox, at_least: i32 },
    /// The selected players control at least `at_least` permanents matching `filter` that all share a creature type (changelings share every type; Graxiplon).
    ControlSharingType { who: Sel, filter: FilterBox, at_least: i32 },
    /// The controller has not yet cast a face-down spell (morph / disguise) this turn (Kadena, Slinking Sorcerer: "the first face-down creature spell you cast each turn").
    NoFaceDownSpellCastThisTurn,
    /// A permanent entered the battlefield face down under the controller's control this turn, or the controller turned a permanent face up this turn, even if it has left since (Oblivious Bookworm).
    FaceDownActivityThisTurn,
    YourTurn,
    /// The source has run `Effect::NoteResolution` at least `n` times this turn, this resolution included (Prowl, Pursuit Vehicle: "if this is the second time this ability has resolved this turn").
    ResolvedTimesThisTurn { n: u8 },
    OpponentsTurn,
    /// An oil counter was removed from a permanent the source's controller controlled this turn, or a permanent with an oil counter on it was put into a graveyard this turn (Churning Reservoir).
    OilCounterEventThisTurn,
    InStep { step: Step },
    /// It is a combat phase (any step from beginning of combat to end of combat); pair with `YourTurn` for "your combat phase" (Great Train Heist).
    InCombatPhase,
    /// The source is tapped / untapped.
    SourceTapped,
    /// The source was untapped when the current turn began (before the untap step) and has been on the battlefield since then (Rasputin Dreamweaver).
    SourceStartedTurnUntapped,
    /// The source's effects are being ignored until end of turn (`Effect::IgnoreThisUntilEndOfTurn`; Volrath's Curse: `Not` of this on its statics).
    IgnoredUntilEndOfTurn,
    /// The source has dealt damage since it entered its current zone ("as long as it hasn't dealt damage yet" = `Not` of this).
    SourceHasDealtDamage,
    /// The source dealt damage to a player other than its controller this turn (Dunerider Outlaw: "if this creature dealt damage to an opponent this turn"); judged against the current controller.
    SourceDamagedOpponentThisTurn,
    /// The source has been under its controller's control continuously since the beginning of that player's most recent turn (not "summoning sick"): Rocket Launcher.
    SourceControlledSinceTurnStart,
    /// An opponent of the controller was dealt noncombat damage during the previous turn.
    OpponentDealtNoncombatDamageLastTurn,
    /// The source is monstrous.
    Monstrous,
    /// The source is renowned.
    Renowned,
    /// The source is harnessed (The Soul Stone).
    Harnessed,
    /// The controller has max speed (speed 4).
    MaxSpeed,
    /// The controller has the city's blessing.
    CityBlessing,
    /// The controller has an enduring story (Storied: Fíli, Kíli, Bombur; latched by a state check in sba.rs).
    EnduringStory,
    /// The source is suspected.
    IsSuspected,
    /// The source is its controller's Ring-bearer.
    IsRingBearer,
    /// The controller has the initiative.
    HaveInitiative,
    /// Some player selected by `who` has the initiative ("if you or a player you're attacking has the initiative": Undercellar Sweep, with `PlayersAttackedBy`).
    PlayerHasInitiative { who: Sel },
    /// The player the trigger is about (the damaged player, the player who cast ...) is the ability's controller.
    TriggerPlayerIsYou,
    /// The triggering spell had part of its cost paid by another player through assist (Yume, Chronicler of Valor).
    TriggerSpellAssisted,
    /// The trigger's player (e.g. the player who lost the game) was attacked this turn by a creature the ability's controller controlled that had one of `subtypes` (any creature if empty; Ramses, Assassin Lord).
    /// One or more of the players the trigger player (the attacker, in a `YouAttack` trigger) is attacking has a poison counter (Norn's Decree; attacked planeswalkers' and battles' controllers count as attacked).
    TriggerPlayerAttackedPoisoned,
    TriggerPlayerAttackedBy {
        #[serde(default)]
        subtypes: Vec<String>,
    },
    /// The source card has not changed zones since `Effect::MarkZcc` ran on it (Mangara's Blessing).
    ZccMarked,
    /// A player other than the ability's controller attacked the controller (the player, not their planeswalkers) during that player's last turn (Avenge).
    AnyPlayerAttackedYouLastTurn,
    /// The controller cast a spell or put a nontoken permanent onto the battlefield during their last turn (Arboria; `Game::last_act_turn`).
    ActedLastTurn,
    /// The controller drew a card during the previous turn ("if you drew a card last turn": Mine Is the Only Truth).
    DrewLastTurn,
    /// Wraps `c`: in a `CantAttackWhileDefender` static it restricts attacking players only, not their planeswalkers or battles (Arboria).
    PlayerAttackOnly { c: Box<Condition> },
    /// The trigger's player is the active player ("if it isn't that player's turn" is `Not` of this).
    TriggerPlayerIsActive,
    /// The controller hasn't been dealt combat damage since their last turn (Marchesa, Resolute Monarch).
    NoCombatDamageSinceYourLastTurn,
    /// No opponent of the controller has cast a spell since the controller's last turn ended (I Bask in Your Silent Awe).
    NoOpponentSpellSinceYourLastTurn,
    /// The trigger's player attacked the ability's controller (the player, not their planeswalkers) during that player's last turn (O-Kagachi, Vengeful Kami).
    TriggerPlayerAttackedYouLastTurn,
    /// The trigger's player attacked the ability's controller (the player, not their planeswalkers) this turn (Faramir, Prince of Ithilien, via a delayed trigger created for the chosen player).
    TriggerPlayerAttackedYouThisTurn,
    /// The trigger's player has an opponent other than the ability's controller who is not being attacked (no creature is attacking them or a planeswalker/battle they control) in the current combat (Suppressor Skyguard).
    TriggerPlayerHasOtherOpponentNotAttacked,
    /// The controller has completed at least this many dungeons.
    CompletedDungeon { at_least: u8 },
    /// The controller has completed the dungeon with this name (Acererak the Archlich: "if you haven't completed Tomb of Annihilation").
    CompletedNamedDungeon { name: String },
    /// The Ring has tempted the controller at least this many times.
    RingTempted { at_least: u8 },
    /// After a vote: this option got more votes than every other option (`strict: true`), or at least as many as every other option (`strict: false`, "most votes or tied").
    MostVotes { option: u8, strict: bool },
    SourceHasCounter { kind: CounterKind, at_least: i32 },
    Compare { a: Amount, cmp: Cmp },
    /// Compares two amounts: `a op b` (e.g. mana spent greater than this creature's power).
    CompareAmounts { a: Amount, op: CmpOp, b: Amount },
    Threshold,
    /// A creature or planeswalker controlled (when damaged) by a player matching `controlled_by` was dealt excess damage this turn (Rith, Liberated Primeval).
    ExcessDamageDealtThisTurn { #[serde(default)] controlled_by: PlayerFilter },
    /// A selected permanent was dealt damage this turn by a source controlled by the ability's controller (when it dealt the damage) that matches `filter`
    /// ("if a Giant, Wizard, or spell you controlled dealt damage to it this turn": Aegar, the Freezing Flame).
    DamagedThisTurnByYour { what: Sel, filter: FilterBox },
    /// The source's abilities have been activated at least this many times this turn (Dragon Whelp: "if this ability has been activated four or more times this turn").
    SourceActivationsThisTurn { at_least: u8 },
    /// The source's abilities have been activated fewer times this turn than the amount (Withering Wisps: no more than the number of snow Swamps).
    SourceActivationsBelow { limit: Amount },
    /// The controller has played a land this turn.
    PlayedLandThisTurn,
    Delirium,
    /// The controller's graveyard holds at least this many different card types (Winter, Misanthropic Guide stacks one delta per type above three).
    GraveyardCardTypes { at_least: u8 },
    Metalcraft,
    Monarch { who: Sel },
    CastThisTurn { filter: FilterBox, at_least: i32 },
    /// The controller cast a spell from a graveyard or activated an ability of a card in a graveyard this turn (Laboratory Drudge).
    UsedGraveyardThisTurn,
    /// The controller's cast of an instant or sorcery from a graveyard is their first such cast this turn, counting spells cast before the source was on the battlefield (Sevinne, the Chronoclasm).
    FirstInstantSorceryFromGraveyardThisTurn,
    /// You played a land or cast a spell this turn from anywhere other than your hand (Spider-Man 2099).
    PlayedOrCastFromNonHandThisTurn,
    /// The controller has not foretold a card yet this turn (Ranar the Ever-Watchful).
    NoForetoldThisTurn,
    /// It is your turn, or you control a permanent with `StaticEffect::ForetellAnyTime` (the generated foretell special action's timing).
    MayForetellNow,
    /// The source has dealt combat damage (to anything) at some point this game (Ruric Thar, Magecrusher).
    SourceEverDealtCombatDamage,
    /// This spell/permanent was cast using its n-th alternative cost (None = any alternative cost): evoke, overload, ...
    CastWithAlt { index: Option<u8> },
    /// The source is an exiled card with this mark (foretold / plotted / warped) that was marked on an earlier turn.
    ExileMarked { mark: ExileMark },
    /// The source creature is prepared.
    SourcePrepared,
    /// It is main phase of the player's turn with an empty stack ("any time you could cast a sorcery").
    SorcerySpeed,
    /// The source spell was cast out of sorcery timing only through its own "as though it had flash" (no other effect gave it flash).
    CastWithOwnFlash,
    /// Some player's library has `n` or fewer cards (Shelldock Isle: "if a library has twenty or fewer cards in it").
    LibraryAtMost { n: u32 },
    /// This permanent is saddled (until end of turn).
    Saddled,
    /// This creature is paired (soulbond).
    Paired,
    /// Door `door` (1 or 2) of this Room is unlocked (added automatically to every ability of a Room).
    DoorUnlocked { door: u8 },
    /// Both doors of this Room are unlocked ("fully unlocked").
    FullyUnlocked,
    /// At least one of the selected players was dealt damage this turn (bloodthirst: `who: EachOpponent`).
    DamagedThisTurn { who: Sel },
    /// The selected players were dealt damage this turn by a source matching `filter` (judged as the card now; Suffocation: a red instant or sorcery spell).
    DamagedThisTurnBy { who: Sel, filter: FilterBox },
    /// A noncreature permanent the controller controlled was destroyed this turn by a spell or ability an opponent controlled (Cobra Trap).
    NoncreatureDestroyedByOpponentThisTurn,
    /// This card was discarded this turn (mayhem: `AlternativeCost {from: Graveyard, condition: ThisDiscardedThisTurn}`).
    ThisDiscardedThisTurn,
    /// A creature matching the filter dealt combat damage to a player this turn (freerunning: `{"types":["Creature"],"subtypes":["Assassin"],"controller":"You"}`; or `commander:true`).
    DealtCombatDamageThisTurn { filter: FilterBox },
    /// A selected permanent dealt combat damage to a creature this turn (Thirsting Axe: `what: AttachedTo`).
    DealtCombatDamageToCreature { what: Sel },
    /// The controller committed a crime this turn.
    CommittedCrime,
    /// True if, for at least one player in `who` (evaluated first), `cond` holds with that player as the controller
    /// ("an opponent has eight or more cards in their graveyard": `who: EachOpponent`, `cond` reads `You`). Use `Not` for "no opponent ...".
    ForSomePlayer { who: Sel, cond: Box<Condition> },
    Revolt,
    /// A permanent card was put into your graveyard from anywhere this turn.
    Descended,
    Raid,
    /// You surveilled this turn.
    SurveilledThisTurn,
    /// You controlled a permanent with this subtype as this spell (or the permanent it became) was cast.
    ControlledSubtypeAtCast { subtype: String },
    /// You activated a loyalty ability this turn.
    LoyaltyActivatedThisTurn,
    /// You scried or surveilled this turn.
    ScriedOrSurveilledThisTurn,
    /// A creature died this turn (morbid).
    Morbid,
    /// A creature you controlled died this turn.
    YourCreatureDied,
    /// At least one card left your graveyard this turn (Gau, Essence Anchor).
    CardLeftYourGraveyardThisTurn,
    /// At least one creature card left your graveyard this turn (Syrix, Carrier of the Flame).
    CreatureCardLeftYourGraveyardThisTurn,
    /// A counter of this kind was put on a permanent under your control this turn (Fairgrounds Trumpeter).
    CounterPutOnYourPermanentThisTurn { kind: CounterKind },
    /// You put a counter of this kind on a creature this turn (Sigardian Paladin).
    CounterPutByYouOnCreatureThisTurn { kind: CounterKind },
    /// You put a counter of any kind on a creature this turn (Lasting Tarfire).
    AnyCounterPutByYouOnCreatureThisTurn,
    /// This is the first upkeep step of the turn (Paradox Haze).
    FirstUpkeepThisTurn,
    /// Planechase: the planar controller planeswalked to this plane this turn (Unyaro).
    PlaneswalkedToThisTurn,
    /// The current end step is the first one of this turn (Y'shtola Rhul).
    FirstEndStepThisTurn,
    /// The selected player was the monarch as the turn began (Knights of the Black Rose).
    WasMonarchAtTurnStart { who: Sel },
    /// This is the first time counters were put on the trigger's permanent this turn (Stalwart Successor).
    FirstCountersOnTriggerObjectThisTurn,
    /// A counter (any kind) was put on this permanent this turn (Wakka, Devoted Guardian).
    CounterPutOnThisThisTurn,
    /// Exactly one player has the most life ("a player has more life than each other player").
    UniqueMostLife,
    /// It is the first combat phase of the turn (not an extra combat).
    FirstCombatThisTurn,
    /// Exactly one player has the least life.
    UniqueLeastLife,
    /// The spell was cast from this zone (or kicked, etc.: see `Kicked`).
    CastFrom { zone: Zone },
    /// The permanent / spell was cast (from any zone) rather than put onto the battlefield some other way ("enters with ... if you cast it": Nine-Lives Familiar).
    WasCast,
    /// The resolving spell is a copy (a copy was not cast: no addendum / "if you cast it" bonus).
    ResolvingAsCopy,
    /// The permanent that caused the trigger (an entering permanent) was cast rather than put onto the battlefield some other way (Light-Paws, Emperor's Voice: "if you cast it").
    EventObjectWasCast,
    /// The controller has cast another spell with this source's name this game (Approach of the Second Sun; the spell itself counts as one).
    CastAnotherSameNameThisGame,
    Kicked,
    /// The i-th Kicker ability of the spell (0 = the first one printed) was paid ("if it was kicked with its {1}{R} kicker").
    KickedWith { index: u8 },
    TargetIs { idx: u8, filter: FilterBox },
    /// The chosen target number `idx` is a player ("if a player was dealt damage this way", "if the target was a player").
    TargetIsPlayer { idx: u8 },
    /// The chosen target number `idx` is a player matching the filter relative to the controller ("if an opponent created a token this way").
    TargetPlayerMatches { idx: u8, filter: PlayerFilter },
    /// The effect's controller (for an ability anyone may activate: the activator) matches the player filter, judged with the source (Damping Engine: `MorePermanentsThanEachOther`).
    ControllerMatches { filter: PlayerFilter },
    /// The source is an Aura card that returned to the battlefield as a non-Aura enchantment (`Effect::Move.non_aura_if_no_host`).
    NonAura,
    /// This permanent was returned as an Aura by `Effect::ReturnAsAura` (Bronzehide Lion).
    AsAura,
    /// Every `Remembered` card matches the filter (false when nothing is remembered): "if you discarded a nonland card" (Connive).
    RememberedIs { filter: FilterBox },
    /// At least one `Remembered` card matches the filter ("if a creature card was discarded this way": Mog, Moogle Warrior).
    AnyRememberedIs { filter: FilterBox },
    /// Some `Remembered` card owned by a player matching `who` has the greatest mana value among the `Remembered` cards, or is tied for it
    /// ("if you discarded the card with the greatest mana value among those cards or tied for greatest": Cait, Cage Brawler).
    RememberedGreatestManaValueBy { who: PlayerFilter },
    /// Every card selected by `what` (at least one) matches the filter ("if the top card of your library is a creature card": `TopOfLibrary`).
    SelIs { what: Sel, filter: FilterBox },
    /// The first selected player is among the players saved in stored slot `slot` (`Remember` + `StoreRemembered`): "each opponent who didn't" (Kynaios and Tiro of Meletis).
    StoredHasPlayer { slot: u8, who: Sel },
    /// The source card is in its owner's graveyard and the card directly above it (put into the graveyard right after it) matches the filter (Krovikan Horror).
    GraveyardCardAboveSource { filter: FilterBox },
    IsDay,
    IsNight,
    /// The player the triggering creature attacked (not a planeswalker / battle) has the most life or is tied for the most life (dethrone).
    AttackedPlayerHasMostLife,
    /// "if no opponent has more life than that player" (only the controller's opponents are compared)
    AttackedPlayerHasMostLifeAmongOpponents,
    /// The triggering attacking creature is attacking a player (not a planeswalker or battle): Master of Cruelties.
    TriggerAttackerAttacksPlayer,
    /// None of the cards in the triggering batch (`Sel::EventCards`) had mana spent on them: they weren't cast, or were cast without paying mana (Satoru, the Infiltrator).
    EventCardsNoManaSpent,
    /// Void: a nonland permanent left the battlefield this turn (any player's), or a spell was warped this turn.
    Void,
}

/// Who or what an effect acts on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum Sel {
    /// The i-th chosen target (of this ability / mode).
    Target { idx: u8 },
    /// All chosen targets.
    Targets,
    /// All the targets chosen for target slot `idx` (Spinning Wheel Kick: "each of X target creatures" after the first slot).
    SlotTargets { idx: u8 },
    /// The ability's source object.
    This,
    /// Your commander card(s), in any zone (Path of Ancestry: "shares a creature type with your commander").
    YourCommanders,
    /// Your commanders that are on the battlefield or in the command zone (Visions of Dominance).
    YourCommandersInPlay,
    /// The controller of the ability.
    You,
    /// The owner of the source.
    Owner,
    /// Each opponent of the controller.
    EachOpponent,
    /// The players this source's controller marked friend (`foe: false`) or foe (`foe: true`) with `Effect::ChooseFriendOrFoe`, in turn order from the controller.
    PlayersPicked { foe: bool },
    /// Every player matching a player filter ("each player dealt combat damage by this creature this turn").
    PlayersWhere { filter: PlayerFilter },
    /// Each opponent of the trigger's player ("each of that player's opponents").
    EachOpponentOfTriggerPlayer,
    /// Every player other than the trigger's player ("each other player" when a player did the thing).
    EachPlayerExceptTriggerPlayer,
    /// The active player ("the active player controls", Siren's Call, Nettling Imp).
    ActivePlayer,
    /// The monarch, if any ("creatures the monarch controls": Azure Fleet Admiral).
    Monarch,
    /// Each opponent of the controller other than the trigger's player ("each other opponent": Kediss, Super State, Hydra Omnivore).
    EachOpponentExceptTriggerPlayer,
    /// Players who voted for the option with this index in the last vote.
    VotersFor { option: u8 },
    /// You and each player who voted for a choice you voted for (last vote).
    VotersLikeController,
    /// The controller's opponents who voted for a choice the controller also voted for (Erestor of the Council).
    OpponentVotersLikeController,
    /// The players whose recorded `ChooseNumber { record }` number is the highest.
    HighestNumberPlayers,
    /// Each player who voted for a choice you did not vote for (last vote).
    VotersUnlikeController,
    EachPlayer,
    /// Every player except the controller(s) of the selected objects ("each player other than its controller": Fractured Identity).
    EachPlayerExcept { of: Box<Sel> },
    /// Every player except the players `a` and `b` select ("every player but you and the chosen player": Choose Your Champion: `You`, `ChosenPlayer`).
    EachPlayerExceptBoth { a: Box<Sel>, b: Box<Sel> },
    /// Inside the `then` of `Effect::EachPlayerChooses`: the cards the current chooser (`Sel::EachObject`) picked, in the order picked (`index: Some(n)` = only the n-th pick, 0-based).
    ChosenByEach { #[serde(default, skip_serializing_if = "Option::is_none")] index: Option<u8> },
    /// Each opponent of the controller except the player(s) the inner selection names ("each of your other opponents" after "target opponent": Feed the Machine).
    EachOpponentExcept { of: Box<Sel> },
    /// One card of the inner selection, chosen at random (seeded RNG): "destroy one of them at random" (Wild Swing, `of: Targets`).
    RandomOf { of: Box<Sel> },
    /// From the selected cards: the one(s) the effect's controller owns plus every other card with a lesser mana value than the greatest of those ("the card you own exiled this way and each other card exiled this way with lesser mana value than it": Triple Triad). Empty when the controller owns none.
    OwnedAndLesserMv { of: Box<Sel> },
    /// A random opponent.
    RandomOpponent,
    /// A random opponent other than the player this Aura enchants (Maddening Hex).
    RandomOpponentNotEnchanted,
    /// A random player (you and the opponents alike): "choose a player at random".
    RandomPlayer,
    /// One object chosen at random with equal chances among all creatures, planeswalkers and players ("2 damage to any target chosen at random").
    RandomAnyTarget,
    /// Every player with the highest life total (ties included: "the player with the most life or tied for most life").
    PlayerMostLife,
    /// Every player with the lowest life total (ties included).
    PlayerLeastLife,
    /// The player being attacked / defending.
    DefendingPlayer,
    /// You and the defending player ("you and defending player each draw a card": Cait, Cage Brawler).
    YouAndDefendingPlayer,
    /// What the selected attacking creature is attacking: the player, or the planeswalker / battle ("damage to the player or planeswalker it's attacking").
    AttackedBy { of: Box<Sel> },
    /// Every player the selected creature(s) attacked this turn ("each player this creature attacked this turn"): Angel of Destiny.
    PlayersAttackedBy { of: Box<Sel> },
    /// Every player the selected creature(s) dealt combat damage to this turn ("that player": Glen Elendra).
    PlayersDealtCombatDamageBy { of: Box<Sel> },
    /// The permanents attached to the selected permanent(s) that match `filter` (Auras / Equipment on a target creature: "return target creature and all Auras attached to it", "unattach Equipment attached to target creature").
    AttachmentsOf { of: Box<Sel>, #[serde(default)] filter: FilterBox },
    /// All objects matching the filter (on the battlefield unless `zone` says otherwise).
    All { filter: FilterBox, #[serde(default, skip_serializing_if = "Option::is_none")] zone: Option<Zone> },
    /// All phased-out permanents matching the filter (Time and Tide).
    PhasedOut { #[serde(default)] filter: FilterBox },
    /// The phased-out permanents held out by this source (`PhaseOut.until_source_leaves`): The Pandorica phases them in when it untaps.
    PhaseHeldBySource,
    /// All cards matching the filter in one zone of a particular player ("target player's graveyard").
    AllOfPlayer {
        player: Box<Sel>,
        zone: Zone,
        #[serde(default)]
        filter: FilterBox,
        /// On the battlefield: select permanents the player OWNS rather than controls.
        #[serde(default)]
        owned: bool,
    },
    /// The object (or player) that caused the trigger to fire.
    TriggerObject,
    /// The cards of a batched trigger event (the matching cards milled together).
    EventCards,
    /// The player who caused the trigger to fire.
    TriggerPlayer,
    /// The player who paid part of the triggering spell's cost through assist (Yume, Chronicler of Valor).
    AssistingPlayer,
    /// The player chosen on the source ("choose a player").
    ChosenPlayer,
    /// The permanent bound to this emblem by `CreateEmblem.bind` (kept even after it left the battlefield, so a replacement bound to it never matches anything else).
    BoundCard,
    /// The exiled card this permanent last chose with `Effect::SetChosenCard`, while it is still in exile (Koh, the Face Stealer).
    ChosenCard,
    /// The card a token copy was made from (`Card.linked_card`), if it is still in exile: "return the exiled card to its owner's graveyard" (Hofri Ghostforge).
    LinkedCard,
    /// With two chosen players (`ChooseKind::Player` + `SecondPlayer`): the one that is NOT the event's player (Sower of Discord: "the other chosen player").
    OtherChosenPlayer,
    /// The source of the event that caused the trigger (the creature that dealt damage, the spell that was cast).
    TriggerSource,
    /// The stack item (spell or ability) whose targeting fired a `BecomesTarget` / `PlayerBecomesTarget` trigger; `CounterSpell` counters it even when it is an ability.
    TriggerStackItem,
    /// The owner of the card with the greatest mana value among the selected cards; nobody when two or more of them tie for greatest (Psychic Battle: `of` = the revealed cards, `Remembered` after `RevealTop`).
    OwnerOfGreatestMv { of: Box<Sel> },
    /// The source permanent / card of the selected stack objects (the artifact whose ability was targeted: Ouphe Vandals); still found after the object was countered.
    StackSource { of: Box<Sel> },
    /// Every activated and triggered ability on the stack controlled by the effect's controller (mana abilities never are; the resolving ability is not on the stack any more: Ulalek, Fused Atrocity).
    StackAbilities,
    /// The permanents that dealt damage to the selected cards this turn (Brine Hag: "each creature that dealt damage to it this turn").
    DamagedThisBy { of: Box<Sel> },
    /// The permanent that was dealt damage (IsDealtDamage triggers).
    TriggerDamaged,
    /// Inside an ability granted by a static / effect: the permanent that granted it (the Equipment or Aura; `This` is the creature).
    GrantSource,
    /// What the source is attached to.
    AttachedTo,
    /// What each selected Aura / Equipment is attached to ("the creature that Aura is attached to": Crown of the Ages, Enchantment Alteration).
    HostOf { of: Box<Sel> },
    /// The creature each selected creature is paired with (soulbond): Joint Assault.
    PairedWith { of: Box<Sel> },
    /// The permanent this Aura/Equipment is attached to, or the creature it enchants.
    Enchanted,
    /// The player the source enchants (a Curse).
    EnchantedPlayer,
    /// The controller of the i-th target.
    ControllerOf { of: Box<Sel> },
    /// The controller (when it dealt the damage) of the most recent source matching `filter` that dealt damage this turn to a player selected by `to` (Suffocation).
    LastDamagerController { to: Box<Sel>, filter: FilterBox },
    /// The owner of the target.
    OwnerOf { of: Box<Sel> },
    /// Every distinct owner of the selected cards ("each player who discarded a card this way": the owners of the remembered discarded cards).
    /// `filter`: only the cards matching it count (Scythe Specter: `mana_value_vs` = `HighestManaValueOf` of `Remembered`).
    OwnersOf { of: Box<Sel>, #[serde(default)] filter: FilterBox },
    /// Whatever was chosen/created by an earlier effect of this ability (token just created, card exiled...).
    Remembered,
    /// The remembered cards from position `from` on (0-based, in the order they were remembered), at most `count` of them: `{from:0,count:1}` = the first one, `{from:1}` = all but the first (Flickerform).
    RememberedSlice { from: u8, #[serde(default, skip_serializing_if = "Option::is_none")] count: Option<u8> },
    /// The remembered cards that match `filter` ("an instant or sorcery card from among those exiled cards": Chandra, Hope's Beacon).
    RememberedWhere { #[serde(default)] filter: FilterBox },
    /// A set saved earlier in this resolution with `StoreRemembered` (slot 0-3): a second/third remembered set (piles, "the rest").
    Stored { slot: u8 },
    /// The cards in exile that this permanent exiled ("exiled with this", hideaway, linked exile). Cleared when the card leaves exile or this permanent changes zones.
    ExiledWithThis,
    /// The creatures (still on the battlefield) that were tapped to convoke the selected spell / permanent (Venerated Loxodon: "each creature that convoked it").
    ConvokedCreatures { of: Box<Sel> },
    /// The top `n` cards of each selected player's library (not moved; `n` defaults to 1).
    TopOfLibrary { who: Box<Sel>, #[serde(default = "one_u8")] n: u8, /// Only cards matching this count (the topmost CREATURE card of a graveyard / library).
        #[serde(default)] filter: FilterBox },
    /// The top `n` cards of each selected player's graveyard (the most recently put there first).
    TopOfGraveyard { who: Box<Sel>, #[serde(default = "one_u8")] n: u8, /// Only cards matching this count (the topmost CREATURE card of a graveyard / library).
        #[serde(default)] filter: FilterBox },
    /// The oldest `n` cards of each selected player's graveyard (the bottom card first).
    BottomOfGraveyard { who: Box<Sel>, #[serde(default = "one_u8")] n: u8, #[serde(default)] filter: FilterBox },
    /// The bottom `n` cards of each selected player's library.
    BottomOfLibrary { who: Box<Sel>, #[serde(default = "one_u8")] n: u8, /// A computed number of cards instead of `n` ("exile that many cards from the bottom of your library").
        #[serde(default, skip_serializing_if = "Option::is_none")] amount: Option<Box<Amount>>, /// Only cards matching this count (the topmost CREATURE card of a graveyard / library).
        #[serde(default)] filter: FilterBox },
    /// The permanents (matching `filter`, relative to the effect's controller) and, with `players`, players that the first target of the given spell on the stack could legally target
    /// (Radiate, Mirrorwing Dragon): with `except_current` the spell's present targets are left out. Use as the `objects` of a `ForEach` around `CopySpell {target: EachObject}`.
    LegalTargetsOf {
        spell: Box<Sel>,
        #[serde(default)]
        filter: FilterBox,
        #[serde(default)]
        players: bool,
        #[serde(default)]
        except_current: bool,
    },
    /// The current targets of a spell on the stack (Exterminator Magmarch: "another opponent than the one controlling the targeted permanent").
    SpellTargets { spell: Box<Sel>, #[serde(default)] filter: FilterBox },
    /// Inside `ForEach`: the current element (unlike `Remembered`, effects such as Discard/Mill/Choose inside the loop do not replace it).
    EachObject,
    /// The controller's Ring-bearer.
    RingBearer,
    /// The player to the left of the controller.
    NextPlayer,
    /// The player to the controller's right (the previous player in turn order): Hithlain Rope.
    PreviousPlayer,
    /// The player next to the first selected player in the direction the source chose (`ChooseCreatureType` with `only: ["Left", "Right"]`; Left = next in turn order, Right = previous). `reverse`: the other way round (Order of Succession).
    DirectionNeighbor { of: Box<Sel>, #[serde(default, skip_serializing_if = "is_false")] reverse: bool },
}

// ---------------------------------------------------------------- triggers

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum Trigger {
    /// Whenever a permanent matching the filter enters the battlefield.
    EntersBattlefield { filter: FilterBox },
    /// Whenever a creature (or other permanent) matching the filter dies.
    Dies { filter: FilterBox },
    LeavesBattlefield { filter: FilterBox },
    /// Zone change of a matching card from `from` to `to`.
    ChangesZone {
        filter: FilterBox,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<Zone>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to: Option<Zone>,
        /// Any destination except this zone ("leaves the battlefield without dying": `from: Battlefield`, `not_to: Graveyard`; Dour Port-Mage with `batched`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        not_to: Option<Zone>,
        /// More zones the card may come from besides `from` ("from graveyards and/or the battlefield": `from: Graveyard`, `also_from: [Battlefield]`); pair with `batched` for "one or more".
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        also_from: Vec<Zone>,
        /// Only when a spell or ability controlled by someone other than this trigger's controller moved it (Sacred Ground, Karmic Justice); the trigger player is that cause.
        #[serde(default, skip_serializing_if = "is_false")]
        cause_opponent: bool,
        /// Only when a spell or ability controlled by this trigger's controller moved it ("a spell or ability you control exiles ...": Hero of Bretagard).
        #[serde(default, skip_serializing_if = "is_false")]
        cause_you: bool,
        /// Only when it was destroyed (not sacrificed, not state-based damage-free moves).
        #[serde(default, skip_serializing_if = "is_false")]
        destroyed: bool,
        /// Only when it wasn't sacrificed (Urza's Miter: "if it wasn't sacrificed").
        #[serde(default, skip_serializing_if = "is_false")]
        not_sacrificed: bool,
        /// Only when a spell card was put there as the final step of resolving without being countered or fizzled ("when this spell card is put into a graveyard after resolving": Haunt on sorceries).
        #[serde(default, skip_serializing_if = "is_false")]
        after_resolving: bool,
        /// Only when it was exiled from the battlefield as a cost of a craft ability (an activated ability whose costs exile this card and the crafted permanent: Market Gnome).
        #[serde(default, skip_serializing_if = "is_false")]
        craft_cost: bool,
    },
    Attacks {
        filter: FilterBox,
        /// Only when it attacks you (not your planeswalkers or battles).
        #[serde(default, skip_serializing_if = "is_false")]
        at_you: bool,
        /// Only when it attacks you or a planeswalker/battle you control ("attacks you or a planeswalker you control").
        #[serde(default, skip_serializing_if = "is_false")]
        at_you_or_pw: bool,
        /// Only when it attacks the player this Curse enchants (not a planeswalker or battle that player controls).
        #[serde(default, skip_serializing_if = "is_false")]
        at_enchanted: bool,
        /// "Whenever one or more creatures attack": once per defending player per combat, not once per attacker.
        #[serde(default, skip_serializing_if = "is_false")]
        batched: bool,
        /// Only attacks on a player (not on a planeswalker or battle): "whenever you attack a player".
        #[serde(default, skip_serializing_if = "is_false")]
        player_only: bool,
        /// Only when it attacks an opponent of the trigger's controller (or a planeswalker/battle one controls): "attacks one of your opponents".
        #[serde(default, skip_serializing_if = "is_false")]
        at_opponent: bool,
        /// Only attacks on a planeswalker/battle the trigger's controller controls, never on the player (Oath of Kaya, Mila).
        #[serde(default, skip_serializing_if = "is_false")]
        pw_only: bool,
        /// With `batched`: one trigger for the whole declaration even when the attackers go at different planeswalkers (Mila: "one or more planeswalkers you control"); without it each attacked planeswalker gets its own (Oath of Kaya).
        #[serde(default, skip_serializing_if = "is_false")]
        one_per_declaration: bool,
    },
    /// "Whenever a player attacks one of your opponents": once per attacking player (matching `who`, relative to the trigger's controller) and attacked opponent
    /// of the trigger's controller (planeswalkers and battles don't count) per combat. `TriggerPlayer` = the attacking player, `TriggerObject` = the attacked player.
    PlayerAttacksOpponent { #[serde(default)] who: PlayerFilter },
    /// A spell matching `filter` is countered by a spell or ability controlled by a player matching `by` (relative to the trigger's controller): Baral, Lullmage Mentor.
    SpellCountered {
        #[serde(default)]
        filter: FilterBox,
        #[serde(default = "you_filter")]
        by: PlayerFilter,
    },
    /// `player_only`: it must be attacking a player, not a planeswalker or battle ("attacks a player alone": Yuriko, Blade of the Mighty).
    AttacksAlone { filter: FilterBox, #[serde(default, skip_serializing_if = "is_false")] player_only: bool },
    /// "Whenever this creature and another creature attack different players": the matching creature attacks a player and another attacking creature attacks a different player (not a planeswalker or battle; Canal Courier).
    AttacksDifferentPlayers { filter: FilterBox },
    /// "Whenever you attack" / "whenever a player attacks with N or more creatures": once per attack declaration by a player matching `who`, if at least `min` creatures attacked.
    /// The trigger object is the first attacking creature.
    YouAttack {
        #[serde(default = "you_filter")]
        who: PlayerFilter,
        #[serde(default = "one_u8")]
        min: u8,
        /// At most this many attackers ("attack with exactly two creatures": min 2, max 2).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<u8>,
        /// Count only attackers matching this ("attack with one or more Zombies"); the trigger needs at least `min` of them.
        #[serde(default)]
        filter: FilterBox,
        /// Only attackers that attack a player (not a planeswalker or battle) count ("one or more Devils you control attack one or more players": Zurzoth).
        #[serde(default, skip_serializing_if = "is_false")]
        players_only: bool,
        /// The counted attackers' total power must be at least this ("attack with creatures with total power 6 or greater": Ugin's Mastery).
        #[serde(default, skip_serializing_if = "is_zero_i32")]
        min_total_power: i32,
    },
    /// A creature matching `filter` blocks. Fires once per blocked attacker unless `once` ("whenever this blocks" with no "a creature": one trigger however many attackers it blocks).
    Blocks {
        filter: FilterBox,
        #[serde(default, skip_serializing_if = "is_false")]
        once: bool,
        /// Also requires the blocked attacker to match ("whenever this blocks a nonblack creature").
        #[serde(default, skip_serializing_if = "Option::is_none")]
        blocked: Option<FilterBox>,
    },
    /// An attacking creature matching `filter` isn't blocked (after blockers are declared; "attacks and isn't blocked").
    AttackerUnblocked { filter: FilterBox },
    /// "When you unlock this door": a door of this Room becomes unlocked (`door` 1 or 2; absent = either).
    /// A Room (matching `filter`) you control becomes fully unlocked (eerie).
    FullyUnlocksRoom { filter: FilterBox },
    UnlocksDoor { #[serde(default, skip_serializing_if = "Option::is_none")] door: Option<u8> },
    /// A matching permanent phases out / phases in.
    PhasesOut { filter: FilterBox },
    PhasesIn { filter: FilterBox },
    /// A matching player commits a crime (targets an opponent, an opponent's permanent or a card in an opponent's graveyard).
    CommitsCrime { #[serde(default)] who: PlayerFilter },
    /// "Whenever day becomes night or night becomes day" (`to_day: true` = only night becoming day, `false` = only day becoming night).
    DayNightChanged { #[serde(default)] to_day: Option<bool> },
    BecomesBlocked {
        filter: FilterBox,
        /// Triggers for every blocking creature ("becomes blocked by a creature"), not once per attacker.
        #[serde(default, skip_serializing_if = "is_false")]
        by_each: bool,
        /// Also requires the blocking creature to match ("becomes blocked by a nonblack creature").
        #[serde(default, skip_serializing_if = "Option::is_none")]
        blocker: Option<FilterBox>,
        /// "Whenever one or more creatures you control become blocked": one trigger per declare-blockers step, however many attackers were blocked.
        #[serde(default, skip_serializing_if = "is_false")]
        batched: bool,
    },
    /// Combat damage dealt by a matching source.
    DealsCombatDamage {
        filter: FilterBox,
        #[serde(default)]
        to: DamageTo,
        /// The damaged permanent must match this filter ("deals combat damage to a non-Wall creature": Acidic Dagger).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to_filter: Option<Box<Filter>>,
        /// The damaged player must be the owner of the damage source ("deals combat damage to its owner": The Beast, Deathless Prince).
        #[serde(default, skip_serializing_if = "is_false")]
        to_owner: bool,
    },
    DealsDamage {
        filter: FilterBox,
        #[serde(default)]
        to: DamageTo,
        /// Some(true): only combat damage; Some(false): only noncombat damage.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        combat: Option<bool>,
        /// With `batched`: damage is folded only per damage source ("a red creature or spell deals damage": Justice), not across all matching sources ("one or more Pirates").
        #[serde(default, skip_serializing_if = "is_false")]
        per_source: bool,
        /// "one or more ... deal damage to your opponents, for each opponent dealt damage": combat damage of one damage step folds into ONE trigger whose `EventAmount` is the number of different opponents dealt damage (like `CombatDamageToPlayer.opponents_count`); noncombat damage triggers separately with `EventAmount` 1 (Malcolm, Keen-Eyed Navigator).
        #[serde(default, skip_serializing_if = "is_false")]
        opponents_count: bool,
        /// Only the first time this turn the damage source dealt damage to a player (Stensia: "for the first time each turn").
        #[serde(default, skip_serializing_if = "is_false")]
        first_to_player_each_turn: bool,
    },
    /// Damage dealt to a matching permanent or player.
    /// `combat`: Some(true) = only combat damage, Some(false) = only noncombat damage (absent = any).
    IsDealtDamage {
        filter: FilterBox,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        combat: Option<bool>,
        /// Combat damage dealt at the same time by several sources is one trigger whose `EventAmount` is the total (Innocent Bystander: "dealt 3 or more damage").
        #[serde(default, skip_serializing_if = "is_false")]
        batched: bool,
        /// Only damage beyond lethal ("is dealt excess damage": Aegar, the Freezing Flame); `EventAmount` is then the excess (Toralf, God of Fury).
        #[serde(default, skip_serializing_if = "is_false")]
        excess: bool,
    },
    BeginningOfStep {
        step: Step,
        #[serde(default)]
        whose: PlayerFilter,
    },
    /// "At the beginning of the first upkeep of the game" (Two Streams Facility). Use with `from: "Command"` on a plane that starts face up.
    FirstUpkeepOfGame,
    SpellCast {
        filter: FilterBox,
        #[serde(default)]
        by: PlayerFilter,
        /// Also whenever a matching spell is copied ("cast or copy": magecraft, Prismari Apprentice ...).
        #[serde(default)]
        or_copy: bool,
    },
    /// Whenever a matching player casts a matching spell using teamwork (a Kicker whose cost taps creatures: `Cost::TapTotalPower`) (Virtual Assistant).
    SpellCastTeamwork {
        filter: FilterBox,
        #[serde(default)]
        by: PlayerFilter,
    },
    /// Whenever a matching player copies a matching spell (only copies, not casts: The Twelfth Doctor).
    SpellCopied {
        filter: FilterBox,
        #[serde(default)]
        by: PlayerFilter,
    },
    /// Whenever a matching player casts a spell (matching `filter`) that targets a permanent matching `target` (heroic: `target: {this: true}`).
    SpellCastTargets {
        filter: FilterBox,
        #[serde(default)]
        by: PlayerFilter,
        target: FilterBox,
        /// The spell's ONLY target is such a permanent ("a spell that targets only this creature"): it has exactly one target and it matches.
        #[serde(default)]
        only: bool,
    },
    /// This permanent becomes renowned.
    BecomesRenowned { filter: FilterBox },
    /// This creature (matching the filter) mutates (a mutating creature spell merges with it).
    Mutates { filter: FilterBox },
    /// This card (or a matching one) was plotted: exiled from hand with the Plot special action (Longhorn Sharpshooter; use `from: Exile`).
    BecomesPlotted { filter: FilterBox },
    /// A card (matching the filter, e.g. `owner: You`) is foretold: exiled from hand by the foretell special action or a foretelling effect (Dream Devourer).
    BecomesForetold { filter: FilterBox },
    /// A creature matching `filter` is exerted ("whenever you exert a creature": `controller: You`); the trigger object is the exerted creature.
    Exerted { filter: FilterBox },
    /// "Whenever the final chapter ability of a Saga (matching `filter`) resolves" (Narci, Tom Bombadil). The trigger object is the Saga.
    SagaFinalChapter { filter: FilterBox },
    /// "Whenever the final chapter ability of a Saga (matching `filter`) triggers" (Historian's Boon): fires as the lore counter reaching the last chapter is added. The trigger object is the Saga.
    SagaFinalChapterTriggers { filter: FilterBox },
    /// A player faces a dilemma: chooses one or more modes for a spell or ability (Star Trek). `EventAmount` is how many they have faced this turn.
    FacesDilemma {
        #[serde(default)]
        who: PlayerFilter,
    },
    /// A player scries or surveils (`surveil`: Some(false) scry only, Some(true) surveil only, None either).
    Scried {
        #[serde(default)]
        who: PlayerFilter,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        surveil: Option<bool>,
        /// Only the first surveil of the turn (Whispering Snitch: "whenever you surveil for the first time each turn"; use with `surveil: true`).
        #[serde(default, skip_serializing_if = "is_false")]
        first_this_turn: bool,
        /// Only when the scry put one or more cards on the bottom; `EventAmount` is how many (The Temporal Anchor).
        #[serde(default, skip_serializing_if = "is_false")]
        to_bottom: bool,
    },
    BecomesTapped {
        filter: FilterBox,
        /// Only when an effect (spell/ability resolution) controlled by a player matching this (relative to the trigger's controller) tapped it - not costs, attacks or the untap step ("whenever you tap an untapped creature an opponent controls").
        #[serde(default, skip_serializing_if = "Option::is_none")]
        by_effect_of: Option<PlayerFilter>,
        /// Not when it is tapped as it is declared as an attacker (Verity Circle).
        #[serde(default, skip_serializing_if = "is_false")]
        not_declared_attacker: bool,
        /// Only when it was tapped to pay a teamwork cost (Agent Maria Hill).
        #[serde(default, skip_serializing_if = "is_false")]
        teamwork: bool,
    },
    BecomesUntapped { filter: FilterBox },
    /// A regeneration shield (or "regenerate instead" replacement) was used on a matching permanent ("when it regenerates this way", Matopi Golem).
    Regenerated { filter: FilterBox },
    /// `by`: who controls the spell / ability doing the targeting ("an opponent's spell or ability targets ~": `by: Opponent`).
    /// A player matching `who` (relative to this permanent's controller; `Enchanted` = the enchanted player) becomes the target of a spell or ability controlled by a player matching `by`.
    /// Whenever a spell or ability (spell, activated or triggered ability; not delayed triggers or copies) is put onto the stack; `TriggerStackItem` is that item.
    /// `single_target`: only if it has exactly one target (Grip of Chaos).
    StackItemPut {
        #[serde(default, skip_serializing_if = "is_false")]
        single_target: bool,
        /// Only if it has at least one target ("whenever a player chooses one or more targets": Psychic Battle).
        #[serde(default, skip_serializing_if = "is_false")]
        has_targets: bool,
    },
    /// "Whenever you and/or at least one permanent you control becomes the target of a spell or ability an opponent controls": one trigger per spell or ability
    /// (the first matching permanent target counts only when the player is not targeted too). `TriggerPlayer` is the controller of the targeting spell or ability.
    YouOrPermanentBecomesTarget {
        filter: FilterBox,
        #[serde(default)]
        by: PlayerFilter,
    },
    /// Damage (any, or only combat / noncombat) is dealt to one of the two players chosen on the source (Sower of Discord); `TriggerPlayer` is the damaged player, `EventAmount` the damage.
    DamageToChosenPlayer {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        combat: Option<bool>,
    },
    PlayerBecomesTarget {
        #[serde(default)]
        who: PlayerFilter,
        #[serde(default)]
        by: PlayerFilter,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        spell_only: Option<bool>,
    },
    BecomesTarget {
        filter: FilterBox,
        #[serde(default)]
        by: PlayerFilter,
        /// Some(true): only targeted by a spell; Some(false): only by an ability.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        spell_only: Option<bool>,
        /// Only an activated ability (not a spell or a triggered ability) counts (Professor Hojo).
        #[serde(default, skip_serializing_if = "is_false")]
        activated_only: bool,
        /// The spell / ability's source card must match this filter ("an instant or sorcery spell": `types`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<FilterBox>,
        /// Only when the spell or ability targets only the matching object ("an ability that targets only it": Agrus Kos, Eternal Soldier): every target of the stack object is that permanent.
        #[serde(default, skip_serializing_if = "is_false")]
        only: bool,
    },
    /// A matching player draws a card. `this_card`: only when this very card is drawn (miracle: use `from: Hand`); `first_this_turn`: only the first card drawn that turn.
    Draws {
        who: PlayerFilter,
        #[serde(default, skip_serializing_if = "is_false")]
        this_card: bool,
        #[serde(default, skip_serializing_if = "is_false")]
        first_this_turn: bool,
        /// Only the Nth card drawn this turn (2 = "the second card each player draws each turn").
        #[serde(default, skip_serializing_if = "Option::is_none")]
        nth_this_turn: Option<u32>,
        /// Not the first card drawn during that player's own draw step ("except the first one they draw in their draw step").
        #[serde(default, skip_serializing_if = "is_false")]
        not_first_in_draw_step: bool,
        /// Only when the drawn card has the name chosen for the source (`Effect::ChooseCardName`: Booby Trap).
        #[serde(default, skip_serializing_if = "is_false")]
        chosen_name: bool,
    },
    Discards {
        who: PlayerFilter,
        /// Only when this very card is the one discarded (Madness).
        #[serde(default, skip_serializing_if = "is_false")]
        this_only: bool,
        /// What kind of card was discarded ("whenever you discard an artifact card").
        #[serde(default)]
        filter: FilterBox,
        /// Only when a spell or ability controlled by a player other than the discarding player causes the discard ("whenever an opponent's spell or ability causes you to discard").
        #[serde(default, skip_serializing_if = "is_false")]
        cause_opponent: bool,
    },
    GainsLife {
        who: PlayerFilter,
        /// Only when the life is gained because of a spell or ability matching the filter, read from the object resolving right now ("whenever a white instant or sorcery spell causes you to gain life": Firesong and Sunspeaker).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cause: Option<FilterBox>,
    },
    LosesLife {
        who: PlayerFilter,
        /// Only the first time that player loses life this turn (Vengeful Warchief).
        #[serde(default, skip_serializing_if = "is_false")]
        first_this_turn: bool,
        /// Only when the player lost exactly this much life in the event ("one or more opponents each lose exactly 1 life": Ob Nixilis, Captive Kingpin; use `batched` on the ability).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exactly: Option<i32>,
    },
    /// `kind` omitted: counters of any kind.
    CounterAdded {
        filter: FilterBox,
        /// Only counters put there by a spell or ability controlled by this player ("a counter you put"); relative to the trigger's controller.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        placed_by: Option<PlayerFilter>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<CounterKind>,
        /// Only when this is the first time counters of this kind were put on that permanent this turn, counting placements before the source entered (Botanical Brawler).
        #[serde(default, skip_serializing_if = "is_false")]
        first_this_turn: bool,
        /// Triggers once for each counter put on at the same time ("whenever a +1/+1 counter is put on ..."; Bloodcrazed Hoplite ruling).
        #[serde(default, skip_serializing_if = "is_false")]
        per_counter: bool,
    },
    /// Counters were put on (or given to) a matching player (poison, energy, experience, rad); `EventAmount` = how many (All Will Be One).
    PlayerCounterAdded {
        #[serde(default)]
        who: PlayerFilter,
        /// Only counters put there by a spell or ability controlled by this player; relative to the trigger's controller.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        placed_by: Option<PlayerFilter>,
    },
    /// One or more counters of `kind` were removed from a matching permanent or card (e.g. a suspended card in exile with `from: Exile`); `last`: only when that removed the last counter of the kind.
    CounterRemoved {
        filter: FilterBox,
        kind: CounterKind,
        #[serde(default, skip_serializing_if = "is_false")]
        last: bool,
        /// Only when the trigger's controller removed the counter ("when you remove the last counter": Divine Intervention).
        #[serde(default, skip_serializing_if = "is_false")]
        by_you: bool,
    },
    Sacrificed {
        filter: FilterBox,
        /// Only when the permanent is sacrificed to pay the emerge cost of a spell being cast (Foul Emissary).
        #[serde(default, skip_serializing_if = "is_false")]
        emerge: bool,
    },
    /// When this becomes attached.
    Attached,
    /// Whenever a permanent matching `filter` (an Aura) becomes attached to this permanent (Bramble Elemental, Brood Keeper). TriggerObject = the attached permanent.
    AttachedToThis { filter: FilterBox },
    /// Whenever a permanent matching `attachment` becomes attached to a permanent matching `host` (anything, not just this one). TriggerObject = the host, TriggerSource = the attached permanent. `mv_le`: the host's mana value must be at most the attachment's (Eriette, the Beguiler).
    AttachedToPermanent {
        attachment: FilterBox,
        host: FilterBox,
        #[serde(default, skip_serializing_if = "is_false")]
        mv_le: bool,
    },
    Transforms,
    /// When you lose control of a permanent matching the filter (Gustha's Scepter, Khârn the Betrayer). A source that is itself the permanent triggers for its former controller.
    LosesControl { filter: FilterBox },
    /// You gain control of a matching permanent (Risky Move: "when you gain control of this enchantment"); the ability is controlled by the new controller.
    GainsControl { filter: FilterBox },
    /// When a permanent matching the filter transforms (Cult of the Waxing Moon, Corruption of Towashi).
    /// `or_enters`: also when a matching permanent enters the battlefield transformed (Corruption of Towashi).
    PermanentTransforms { filter: FilterBox, #[serde(default, skip_serializing_if = "is_false")] or_enters: bool },
    /// When a permanent matching the filter is turned face up.
    TurnedFaceUp { filter: FilterBox },
    /// A land is played (a land drop). `filter` tests the land; `not_from_hand`: only when it was played from somewhere other than a hand (Courser of Kruphix, Shadow of the Goblin).
    LandPlayed {
        who: PlayerFilter,
        #[serde(default)]
        filter: FilterBox,
        #[serde(default, skip_serializing_if = "is_false")]
        not_from_hand: bool,
        /// Only when the land was played from this zone (Exile, Library, Graveyard).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<Zone>,
    },
    /// A player's library is shuffled (Cosi's Trickster, Widespread Panic).
    Shuffles { who: PlayerFilter },
    BecomesMonarch,
    /// A matching player becomes the monarch (Garland, Royal Kidnapper: "whenever an opponent becomes the monarch"). `Sel::TriggerPlayer` = that player.
    PlayerBecomesMonarch { who: PlayerFilter },
    Landfall,
    Cycled {
        who: PlayerFilter,
        /// "When you cycle THIS card".
        #[serde(default, skip_serializing_if = "is_false")]
        this_card: bool,
    },
    TokenCreated { filter: FilterBox },
    /// Whenever a matching player activates an ability (of a permanent matching `filter`) that isn't a mana ability.
    AbilityActivated {
        filter: FilterBox,
        #[serde(default)]
        by: PlayerFilter,
        /// Mana abilities count too ("whenever you activate an ability of a creature" without the "that isn't a mana ability" rider: Elrond, Moon-Reader).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        incl_mana: bool,
        /// Only exhaust abilities (an `Activated` ability with `once: true`).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        exhaust: bool,
        /// Only abilities whose activation cost has no {T} ("activates an artifact's ability without {T} in its activation cost": Haunting Wind, Powerleech).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        no_tap_cost: bool,
        /// Only boast abilities (once each turn, only if the creature attacked this turn).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        boast: bool,
        /// Only loyalty abilities ("activate a loyalty ability": Repeated Reverberation).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        loyalty: bool,
        /// Only power-up abilities (`Activated.power_up`).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        power_up: bool,
        /// Only abilities with at least one target that is a creature or a player (Ertha Jo, Frontier Mentor).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        targets_creature_or_player: bool,
        /// Only ninjutsu abilities (activated from hand, returning an unblocked attacker to hand as a cost: Satoru Umezawa).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        ninjutsu: bool,
        /// Only abilities that target only this permanent (exactly one target, the trigger's source: Bill Potts).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        targets_only_this: bool,
        /// Only loyalty abilities whose cost removes at least this many loyalty counters (Way of the Mind Sculptor: 2).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        loyalty_removed_min: Option<u8>,
        /// Only abilities with {X} in their activation cost (Magus Lucea Kane).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        has_x_cost: bool,
        /// Only abilities with a sacrifice among their costs ("if one or more permanents were sacrificed to activate it": Ashnod the Uncaring).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        sacrificed: bool,
        /// Only abilities activated by spending at least this much mana (Dynaheir, Invoker Adept: 4).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mana_spent_min: Option<u8>,
    },
    /// Either trigger event ("whenever you cast ... or activate ..."): one trigger, so "this ability triggers only once each turn" covers both (Bill Potts). `TriggerObject` is whatever `a` / `b` give.
    Either { a: Box<Trigger>, b: Box<Trigger> },
    /// Whenever a triggered ability (controlled per `by`, source matching `filter`) is put on the stack because a permanent entering caused it to trigger (Strict Proctor). TriggerObject is the ability.
    AbilityTriggeredByEntering {
        #[serde(default)]
        by: PlayerFilter,
        #[serde(default)]
        filter: FilterBox,
    },
    /// Whenever a triggered ability (controlled per `by`, source matching `filter`) is put on the stack because its own source attacked, i.e. it triggered from that creature's attack (Firebender Ascension). TriggerObject is the ability.
    AbilityTriggeredByAttacking {
        #[serde(default)]
        by: PlayerFilter,
        #[serde(default)]
        filter: FilterBox,
    },
    /// A player (matching `who`, relative to the source's controller) searches a library (a SearchLibrary effect starts).
    SearchesLibrary { who: PlayerFilter },
    /// Whenever this Mount becomes saddled (or, with `any`, whenever a matching Mount does).
    BecomesSaddled { filter: FilterBox },
    /// A player loses the game (`who`: which players; `Enchanted` for Curse of Vengeance).
    PlayerDies {
        #[serde(default)]
        who: PlayerFilter,
    },
    CombatDamageToPlayer {
        filter: FilterBox,
        /// "One or more creatures ... deal combat damage to a player": once per damaged player per damage step.
        #[serde(default)]
        batched: bool,
        /// Also fires for combat damage dealt to a battle ("to a player or battle": Zurgo and Ojutai). The battle's controller stands in as the damaged player for batching.
        #[serde(default, skip_serializing_if = "is_false")]
        or_battle: bool,
        /// "One or more creatures deal combat damage to one or more of your opponents": ONE trigger per damage step (not per player), only damage to opponents of the controller counts, and
        /// `Amount::EventAmount` is the number of different opponents dealt damage (Hordewing Skaab). Implies `batched`.
        #[serde(default, skip_serializing_if = "is_false")]
        opponents_count: bool,
        /// With `batched`: fold the damage per CONTROLLER of the damaging creatures (as well as per damaged player), so each controller's creatures give their own trigger ("whenever one or more creatures an opponent controls deal combat damage to you, that opponent ...": Norn's Decree; read the controller with `Sel::ControllerOf{TriggerObject}`).
        #[serde(default, skip_serializing_if = "is_false")]
        per_controller: bool,
    },
    Exploited,
    /// "Whenever a creature (matching filter) exploits a creature" - fires for any exploiting creature.
    ExploitedBy {
        filter: FilterBox,
        /// Restricts the sacrificed creature ("exploits a nontoken creature").
        #[serde(default)]
        victim: FilterBox,
    },
    /// Whenever a player (matching `who`) wins a clash.
    WinsClash { who: PlayerFilter },
    /// A matching player clashes (either result); `Amount::EventAmount` is 1 if they won, else 0.
    Clashes { who: PlayerFilter },
    /// Whenever players finish voting (the Vote effect completed).
    VotingFinished,
    /// A matching creature explores (the Explore effect resolved for it). `revealed`: the revealed card must match ("explores a land card" / "a nonland card").
    Explores {
        filter: FilterBox,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        revealed: Option<FilterBox>,
    },
    /// A player matching `who` investigates (`first_this_turn`: only their first investigate each turn: Erdwal Illuminator).
    Investigates {
        #[serde(default = "you_filter")]
        who: PlayerFilter,
        #[serde(default, skip_serializing_if = "is_false")]
        first_this_turn: bool,
    },
    /// A player matching `who` collects evidence.
    CollectsEvidence {
        #[serde(default = "you_filter")]
        who: PlayerFilter,
    },
    /// A player matching `who` pays life (EventAmount = the life paid): Font of Agonies.
    PaysLife {
        #[serde(default = "you_filter")]
        who: PlayerFilter,
    },
    /// A creature matching `filter` performs a keyword action (trains / evolves / mentors); the trigger object is the mentored creature for `Mentors`, else the creature itself.
    KeywordActionDone { action: KeywordAction, filter: FilterBox },
    /// A matching creature connives (the Connive effect resolved for it).
    Connives { filter: FilterBox },
    /// A player gets one or more {E} (energy counters).
    GainsEnergy { who: PlayerFilter },
    /// A player matching `who` discovers (`Effect::Discover`); `EventAmount` is the discover value N (Curator of Sun's Creation).
    Discovers { who: PlayerFilter },
    /// One or more cards matching `filter` are milled from a player's library (batched per Mill effect; `EventAmount` = how many matched).
    Milled { who: PlayerFilter, #[serde(default)] filter: FilterBox },
    /// A player matching `who` bends: waterbends (pays a waterbend cost), earthbends, firebends (firebending mana is added) or airbends, one of `kinds` (empty = any kind) (Avatar Aang).
    Bends { #[serde(default = "you_filter")] who: PlayerFilter, #[serde(default, skip_serializing_if = "Vec::is_empty")] kinds: Vec<BendKind> },
    /// A player matching `who` manifests dread (the Manifest dread effect finished); `Sel::EventCards` = the card(s) put into the graveyard this way (Paranormal Analyst).
    ManifestsDread { #[serde(default = "you_filter")] who: PlayerFilter },
    /// A player (matching `who`) rolls a die; `EventAmount` is the result.
    /// `highest`: only when the natural result is the die's highest ("whenever you roll a die's highest natural result": Netherese Puzzle-Ward).
    /// `batched`: "whenever you roll one or more dice": once per roll (all the dice of one roll together, however many the roll replacement added) instead of once per die (Barbarian Class).
    RollsDie { who: PlayerFilter, #[serde(default, skip_serializing_if = "is_false")] highest: bool, #[serde(default, skip_serializing_if = "is_false")] batched: bool },
    /// A player (matching `who`) gives a gift: a spell they cast with the gift promised (a `Kicker` with an empty cost) resolves without being countered or fizzling (Jolly Gerbils).
    GiftGiven { who: PlayerFilter },
    /// A player (matching `who`) flips a coin; `won` limits to won / lost flips (None = any).
    FlipsCoin { who: PlayerFilter, #[serde(default, skip_serializing_if = "Option::is_none")] won: Option<bool> },
    /// A player proliferates (the Proliferate effect completed).
    Proliferates { who: PlayerFilter },
    /// This Vehicle becomes crewed (its Crew cost was paid).
    BecomesCrewed,
    /// Planechase: "Whenever you roll {CHAOS}" on a plane (also when chaos ensues). Use with `from: "Command"`.
    RollsChaos,
    /// Planechase: "Whenever you roll a blank on the planar die" (Pompeii). Use with `from: "Command"`.
    RollsBlank,
    /// Planechase: "Whenever you roll the planar die" (any face; Stairs to Infinity). Use with `from: "Command"`.
    RollsPlanarDie,
    /// Planechase: "When you planeswalk to this plane" / a phenomenon's "When you encounter". Use with `from: "Command"`.
    PlaneswalkTo,
    /// Planechase: "When you planeswalk away from this plane". Use with `from: "Command"`.
    PlaneswalkFrom,
    /// Archenemy: "When you set this scheme in motion". Use with `from: "Command"`.
    SetInMotion,
    /// Archenemy: "Whenever you set a scheme in motion" (another scheme than this one); `non_ongoing`: only a non-ongoing scheme (My Laughter Echoes). TriggerObject = that scheme.
    AnotherSchemeSetInMotion {
        #[serde(default, skip_serializing_if = "is_false")]
        non_ongoing: bool,
    },
    /// A creature (matching `filter`) is tapped to pay a Crew / Saddle cost of a Vehicle / Mount (matching `vehicle`); `saddle`: only Saddle (true) / only Crew (false). TriggerObject = the creature, TriggerSource = the Vehicle / Mount.
    Crews {
        #[serde(default)]
        filter: FilterBox,
        #[serde(default)]
        vehicle: FilterBox,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        saddle: Option<bool>,
    },
    /// This attachment becomes unattached from a permanent.
    BecomesUnattached,
    /// Whenever the Ring tempts a player (matching `who`).
    RingTempts { who: PlayerFilter },
    /// Whenever a player (matching `who`) takes the initiative.
    TakesInitiative { who: PlayerFilter },
    /// This dungeon card's room `name` was entered (put it on a dungeon card as `from: Command`).
    EntersRoom { name: String },
    /// Whenever a player (matching `who`) completes a dungeon.
    CompletesDungeon { who: PlayerFilter },
    /// Whenever a player (matching `who`) chooses a creature as their Ring-bearer (the creature is the trigger object).
    RingBearerChosen { who: PlayerFilter },
    /// Expend N: whenever a player (matching `who`) spends their Nth total mana of the turn to cast spells.
    Expend { n: u32, who: PlayerFilter },
    /// This creature becomes monstrous (a matching creature, with `filter`).
    BecomesMonstrous { filter: FilterBox },
    EndOfCombat,
    /// Fires when its condition is true (state trigger).
    State { condition: Condition },
    /// "Whenever <a matching permanent> is tapped for mana": a triggered mana ability, resolved immediately (Wild Growth, Utopia Sprawl ...).
    TapsForMana {
        filter: FilterBox,
        /// Only when the mana produced includes this type (Forsaken Monument: colorless).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        produced: Option<crate::mana::ManaType>,
        /// Only when the mana produced includes the color this permanent chose (Gauntlet of Power).
        #[serde(default, skip_serializing_if = "is_false")]
        produced_chosen_color: bool,
        /// Not a mana ability: the trigger goes on the stack (Burning Earth).
        #[serde(default, skip_serializing_if = "is_false")]
        on_stack: bool,
    },
    /// A saga's chapter `chapter` (lore counters reaching that number).
    SagaChapter { chapter: u8 },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum PlayerAgg {
    #[default]
    Max,
    Min,
    Sum,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum DamageTo {
    #[default]
    Any,
    Player,
    Creature,
    Planeswalker,
    /// A battle (Siege).
    Battle,
    /// A player or a battle (Rankle and Torbran).
    PlayerOrBattle,
    You,
    Opponent,
    /// You or a permanent you control (Flameblade Angel); only used by `DealsDamage` triggers.
    YouOrYourPermanent,
    /// You or a planeswalker you control (Vengeful Pharaoh).
    YouOrYourPlaneswalker,
    /// The player a Curse enchants.
    EnchantedPlayer,
    /// The player linked to the source (`Effect::LinkPlayer` / "choose target opponent"): Great Train Heist.
    ChosenPlayer,
}

// ---------------------------------------------------------------- effects

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export)]
pub enum Duration {
    Instant,
    UntilEndOfTurn,
    UntilYourNextTurn,
    /// Until the beginning of the effect controller's next upkeep (Xenic Poltergeist).
    UntilYourNextUpkeep,
    /// Until the end of the effect controller's next upkeep step ("until the end of your next upkeep": Halfdane); an effect made during an upkeep lasts through the following one of that player.
    UntilEndOfYourNextUpkeep,
    UntilNextEndStep,
    /// Until the end of the effect controller's next turn ("until the end of your next turn").
    UntilEndOfYourNextTurn,
    /// Until the end of the current combat phase ("can't block this combat", provoke requirements).
    UntilEndOfCombat,
    Forever,
    /// As long as the source stays on the battlefield.
    WhileSourceOnBattlefield,
    /// As long as the effect's controller still controls the source ("for as long as you control this").
    WhileSourceControlled,
    /// As long as the source stays on the battlefield and tapped ("for as long as this creature remains tapped").
    WhileSourceTapped,
    /// Only with `BecomeCopy`: as long as the permanent copied (`of`) stays on the battlefield and tapped (Zygon Infiltrator: "for as long as that creature remains tapped").
    WhileCopiedTapped,
    /// As long as the source stays on the battlefield and tapped AND every affected permanent's power is no greater than the source's power (Old Man of the Sea).
    WhileSourceTappedAffectedPowerLe,
    /// As long as the effect's controller still controls the source AND the source stays tapped ("for as long as you control this and it remains tapped").
    WhileSourceControlledTapped,
    /// Only with `Effect::GainControl`: for as long as the affected permanents' previous controller is the monarch (Garland, Royal Kidnapper: "for as long as they're the monarch").
    WhilePrevControllerMonarch,
    /// As long as the source Aura / Equipment stays attached to something ("for as long as this Equipment remains attached to it": Blade of Shared Souls).
    WhileSourceAttached,
    /// As long as the effect's source Aura / Equipment is attached to the affected permanent itself ("for as long as that Aura is attached to it": Eriette, the Beguiler, with `GainControl.as_source`).
    WhileSourceAttachedToAffected,
    /// As long as every affected permanent is still on the battlefield and tapped (Braided Net: "for as long as it remains tapped").
    WhileAffectedTapped,
    /// As long as the affected permanent is enchanted by an Aura (Rootwater Matriarch: "for as long as that creature is enchanted").
    WhileAffectedEnchanted,
    /// Until the start of the untap step of the affected permanent's controller (Orcish Farmer: "until its controller's next untap step").
    UntilAffectedUntaps,
    /// As long as the affected permanent has a counter of this kind on it (Immortal Obligation: "for as long as that creature has a duty counter on it").
    WhileAffectedHasCounter(CounterKind),
    /// Until the end of the next turn of the affected permanent's controller ("during its controller's next turn").
    UntilEndOfAffectedControllersNextTurn,
    /// Until the untap step of the affected player's next turn ("until that player's next turn": Teferi's Reproach); for player restrictions and `GrantProtection` on players.
    UntilAffectedPlayersNextTurn,
    /// Planechase: until a player planeswalks (player restrictions: Eloren Wilds).
    UntilPlaneswalk,
    /// Until the end of the affected player's next turn ("during that player's next turn": Azor, the Lawbringer); for player restrictions.
    UntilEndOfAffectedPlayersNextTurn,
    /// Until the source card is cast from exile (Shattered Seraph); it ends at that cast, whatever else happens to the card.
    UntilSourceCast,
    /// Until a player casts a creature spell (Soul Sculptor); it ends as the spell is cast.
    UntilCreatureSpellCast,
    /// Until the end of the next combat phase of the affected permanent's controller ("attacks during its controller's next combat phase if able": Sizzling Soloist).
    UntilAffectedControllersCombatEnd,
    /// As long as every affected permanent is controlled by a player other than the effect's controller (Vislor Turlough: goaded "for as long as they control it").
    WhileAffectedControlledByOther,
    /// Until the affected permanent is turned face down (Vesuvan Shapeshifter's copy exceptions).
    UntilAffectedTurnedFaceDown,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum Effect {
    /// Planechase: the planar controller planeswalks (the face-up planes go to the bottom, the next one is turned over).
    Planeswalk,
    /// Planechase: chaos ensues (the face-up planes' {CHAOS} abilities trigger).
    ChaosEnsues,
    /// Planechase: reveal the top `n` cards of the planar deck; each revealed card's chaos abilities trigger (as if chaos ensued for it), then the cards go on the bottom of the planar deck (Pools of Becoming).
    ChaosFromPlanarDeck { n: u8 },
    /// Planechase: reveal cards from the top of the planar deck until a plane card is revealed; chaos ensues on that plane, then all revealed cards go to the bottom (The Fertile Lands of Saulvinia).
    ChaosFromPlanarDeckUntilPlane,
    /// Planechase: put the top card of the planar deck on the bottom (Stairs to Infinity: reveal it, then "you may" via `optional`).
    PlanarTopToBottom,
    /// Planechase: each blank roll of the planar die is a {CHAOS} roll until a player planeswalks away from a plane (Chaotic Aether).
    ChaoticAetherRolls,
    /// Planechase: until the controller's next turn, a planeswalk roll of the planar die makes chaos ensue instead (Fixed Point in Time).
    PlaneswalkRollsChaosUntilNextTurn,
    /// Planechase: reveal cards from the top of the planar deck until `n` plane cards are revealed and planeswalk to all of them at once; the other revealed cards go to the bottom (Spatial Merging).
    PlaneswalkToRevealedPlanes {
        n: u8,
        /// "Except don't planeswalk away from any plane": the face-up planes stay (Norn's Seedcore).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        stay: bool,
    },
    /// Planechase: reveal cards from the top of the planar deck until `n` plane cards are revealed; the controller puts one of the planes on top and the rest go to the bottom in a random order (Interplanar Tunnel).
    PlanarTunnel { n: u8 },
    /// Cards can't enter the battlefield from exile this turn (Bad Wolf Bay).
    NoEnterFromExileThisTurn,
    /// Planechase: creatures can't attack the controller until a player planeswalks away from a plane (Agyrem).
    CantAttackYouUntilPlaneswalk,
    /// Planechase: until a player planeswalks away from a plane, the controller can't lose the game and their opponents can't win (Celestine Reef).
    CantLoseUntilPlaneswalk,
    /// Archenemy: schemes can't be set in motion during the next turn (All in Good Time).
    NoSchemesNextTurn,
    /// Archenemy: the next time the controller would set a scheme in motion, they set three in motion instead (Plots That Span Centuries).
    TripleNextScheme,
    /// Archenemy: the controller sets the top scheme of their scheme deck in motion.
    SetSchemeInMotion,
    /// Archenemy: set the selected scheme (still face up in a command zone) in motion again: its "when you set this scheme in motion" abilities trigger again (My Laughter Echoes).
    SetInMotionAgain { what: Sel },
    /// Archenemy: abandon this ongoing scheme (it goes to the bottom of its owner's scheme deck).
    AbandonScheme,
    DealDamage {
        to: Sel,
        amount: Amount,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<Sel>,
        /// "This damage can't be prevented" (Lightning Surge, Pinpoint Avalanche): ignores prevention shields, replacements and protection for this one effect.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        unpreventable: bool,
        /// "Creatures dealt damage this way can't be regenerated this turn" (Flamebreak): creatures actually dealt damage by this effect.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        no_regen: bool,
    },
    /// "Deals `amount` damage to target creature. If excess damage would be dealt to it this way, instead that damage is dealt to that creature's controller" (Megatron, Tyrant):
    /// the creature gets at most lethal damage, its controller the rest. `Amount::LastExcessDamage` afterwards reads the excess (`LastDamageDealt` the part dealt to the creature).
    DealDamageExcessToController { to: Sel, amount: Amount, #[serde(default, skip_serializing_if = "Option::is_none")] source: Option<Sel> },
    /// Each card selected by `sources` deals `amount` damage to every selected recipient ("each artifact creature you control deals 1 damage to that opponent": Missy).
    EachDealsDamage { sources: Sel, to: Sel, amount: Amount },
    /// The controller bends (records the kind in `Player.bend_mask` and emits `Event::Bent`): put it after the other effects of an earthbend-like action.
    EmitBend { kind: BendKind },
    /// Deals the damage amount of the triggering event (`EventAmount`) from its source (`TriggerSource`) to `to`. Used by the follow-ups of an optional damage redirect
    /// (`Replace::Damage.redirect_optional`); `plain` skips optional redirects so the declined damage is not offered again; `combat` keeps the damage's combat-ness.
    DealDamageAgain {
        to: Sel,
        #[serde(default, skip_serializing_if = "is_false")]
        combat: bool,
        #[serde(default, skip_serializing_if = "is_false")]
        plain: bool,
        /// Deal it as combat damage when the replaced damage was combat damage (Pariah redirecting combat damage).
        #[serde(default, skip_serializing_if = "is_false")]
        keep_combat: bool,
        /// The damage is reduced by this amount first (an optional prevention of X: Battletide Alchemist, `Replace::Damage` with `redirect_optional` and `prevent_amount`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        less: Option<Amount>,
    },
    /// Damage divided as the controller chooses among the targets.
    DivideDamage {
        amount: Amount,
        /// Divide among only the targets chosen for this target spec of the ability (Magma Opus: slot 0; its slot-1 targets are tapped instead).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        slot: Option<u8>,
        /// The division is chosen as the spell is cast (each target gets at least 1, in target order, the last target the rest) and kept on the stack item;
        /// at resolution an illegal target's share is simply lost (Conflagrate). `amount` is read at cast (X is known by then).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        at_cast: bool,
        /// Divide among these objects instead of the ability's targets (untargeted: Master of the Wild Hunt's Wolves).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        among: Option<Sel>,
        /// The damage comes from this object instead of the source (the target creature in Master of the Wild Hunt).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<Sel>,
        /// The first player this selects divides the damage instead of the controller ("as its controller chooses").
        #[serde(default, skip_serializing_if = "Option::is_none")]
        divider: Option<Sel>,
        /// "among any number of" the objects: a part may be 0 (the division is not forced to give each object at least 1).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        any_number: bool,
    },
    /// "Prevent the next N damage that would be dealt this turn to any number of targets, divided as you choose" (Remedy, Pollen Remedy, Angel of Salvation): one shield per chosen target.
    DividePrevention {
        amount: Amount,
        /// The shields are divided as the spell is cast (Remedy); a target that is illegal on resolution gets none.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        at_cast: bool,
    },
    /// Damage divided evenly among the targets, rounded down (Fireball): each target takes amount / (number of targets).
    DealDamageEvenly {
        to: Sel,
        amount: Amount,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<Sel>,
    },
    /// The players lose all unspent mana (their mana pools empty). `give_to`: the first player this selects adds the mana lost this way (Drain Power).
    DrainManaPool { who: Sel, #[serde(default, skip_serializing_if = "Option::is_none")] give_to: Option<Sel>, /// The mana lost is noted for the source until end of turn, read with `Amount::ManaDrained` (Pygmy Hippo).
 #[serde(default, skip_serializing_if = "is_false")] record: bool },
    /// Each selected player activates a mana ability of each land they control (the first one that needs no mana, its first colour; Drain Power).
    ActivateLandManaAbilities { who: Sel },
    /// Double the amount of each type of unspent mana the players have (Doubling Cube).
    DoubleManaPool { who: Sel },
    /// Look at the players' hands (information only; nothing changes).
    LookAtHand { who: Sel },
    /// The controller privately looks at the selected cards (a face-down creature: Aven Soulgazer). Nothing moves; the look is only logged, like `LookAtHand`.
    LookAtCards { what: Sel },
    /// Distribute `total` counters among the chosen targets (the controller picks the split, at least one each).
    /// `remove_at_cleanup`: for each counter put on a creature this way, remove one from it at the beginning of the next cleanup step (Bounty of the Hunt).
    DivideCounters {
        kind: CounterKind,
        total: Amount,
        #[serde(default, skip_serializing_if = "is_false")]
        remove_at_cleanup: bool,
        /// Divide among these objects instead of the ability's targets (untargeted: "among any number of creatures you control", usually `Remembered` after a `Choose`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        among: Option<Sel>,
        /// "Distribute up to N": the controller first chooses the total (at least one per object, at most `total`).
        #[serde(default, skip_serializing_if = "is_false")]
        up_to: bool,
        /// "For each counter put on a creature this way, remove a counter from it at the beginning of the next end step" (a delayed trigger per creature; Shambling Swarm).
        #[serde(default, skip_serializing_if = "is_false")]
        remove_at_end_step: bool,
        /// "Move" semantics: as many counters as were actually put on the objects are removed from the first card this selects (Forgotten Ancient).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        take_from: Option<Sel>,
        /// The division is chosen as the spell is cast (kept on the stack item; a target that is illegal at resolution loses its share): Abzan Charm.
        #[serde(default, skip_serializing_if = "is_false")]
        at_cast: bool,
    },
    /// Learn: the controller may reveal a Lesson card from their sideboard and take it (asked only when the sideboard holds a Lesson), otherwise may discard a card to draw a card. A `Replace::Learn` of the controller's ("if you would learn, ... instead") is offered first;
    /// `plain` skips it.
    Learn {
        #[serde(default, skip_serializing_if = "is_false")]
        plain: bool,
        /// The rummage half is always offered as a plain yes/no "may" (even with an empty hand) instead of only when a card can be discarded.
        #[serde(default, skip_serializing_if = "is_false")]
        always_ask: bool,
    },
    Draw {
        who: Sel,
        #[serde(default = "one_amount")]
        n: Amount,
        /// Skip "draw ... instead" replacement effects (the draws done BY such a replacement must not trigger it again).
        #[serde(default, skip_serializing_if = "is_false")]
        plain: bool,
    },
    /// Draw cards, and the cards drawn become `Remembered` ("draw a card and reveal it; if it isn't a land, discard it").
    DrawRemember { who: Sel, n: Amount },
    /// Start counting the cards drawn from now on (read with `Amount::DrawnSinceStart`).
    StartDrawCount,
    GainLife { who: Sel, n: Amount },
    LoseLife { who: Sel, n: Amount },
    SetLife { who: Sel, n: Amount },
    /// Each selected card in exile that doesn't have suspend gains suspend ("if it doesn't have suspend, it gains suspend"): at its owner's upkeep it loses a time counter, and when the last is removed it is cast without paying its mana cost (a creature gains haste).
    GrantSuspend { what: Sel },
    /// "Target spell's controller exiles it with N delay counters on it" (Ertai's Meddling): the spell leaves the stack without being countered. At the beginning of each of that player's upkeeps
    /// while the card is still exiled a delay counter comes off; with none left the player puts a copy of the original spell (same targets, modes and X) onto the stack.
    ExileSpellDelayed { what: Sel, counters: Amount },
    /// "Reverse the game's turn order" (Aeon Engine): `Game.turn_reversed` flips; the next player / previous player (turn order, priority, Left / Right) swap places.
    ReverseTurnOrder,
    /// Cipher: the resolving spell card is exiled encoded on a creature you choose (optional; put it last in the spell's effects). Whenever that creature deals combat damage to a player, its controller casts a copy of the encoded card for free.
    Encode,
    /// Remember that the source card is the same object now (for `Condition::ZccMarked`): "if this card is still in the graveyard" delayed returns (Mangara's Blessing).
    MarkZcc,
    /// Meld: if you own and control this permanent and a permanent named `with`, exile both, then put the melded permanent (the card named `into`, written as an ordinary card) onto the battlefield. When it leaves the battlefield both halves go to that zone.
    /// With `attacking` (Mishra, Claimed by Gix): both halves must be attacking, and the melded permanent enters tapped and attacking.
    Meld {
        with: String,
        into: String,
        #[serde(default, skip_serializing_if = "is_false")]
        attacking: bool,
    },
    /// Time travel: for each suspended card you own and each permanent you control with a time counter, you may add or remove one time counter (asked per object; all changes happen together at the end).
    TimeTravel,
    /// End the turn: exile every spell on the stack, abilities cease to exist, combat ends, and the game skips to the cleanup step.
    /// "End the combat phase": exile all spells on the stack, remove all attackers and blockers from combat, and go to the postcombat main phase (Mandate of Peace).
    EndCombatPhase,
    EndTheTurn,
    /// Count one resolution of the source's ability this turn (read with `Condition::ResolvedTimesThisTurn`).
    NoteResolution,
    /// Two players exchange life totals (`a` and `b` each select a player; each ends with the other's old total).
    ExchangeLife { a: Sel, b: Sel },
    /// Destroy every creature that was blocked by the selected creature this turn (they can't be regenerated), then, with `reanimate`, for each one that died this way
    /// the controller puts a creature card from the graveyard of the player who controlled it the last time it became blocked by that creature onto the battlefield under its owner's control (Glyph of Reincarnation).
    DestroyBlockedBy {
        by: Sel,
        #[serde(default, skip_serializing_if = "is_false")]
        reanimate: bool,
    },
    Destroy {
        what: Sel,
        #[serde(default, skip_serializing_if = "is_false")]
        no_regeneration: bool,
        /// The cards that were actually destroyed become `Remembered` ("return each creature destroyed this way").
        #[serde(default, skip_serializing_if = "is_false")]
        remember: bool,
        /// In a triggered ability: only permanents that were already on the battlefield (with the same timestamp) when the ability triggered are destroyed (Drudge Spell).
        #[serde(default, skip_serializing_if = "is_false")]
        existing_at_trigger: bool,
        /// With `remember`: only creatures that actually died (went to the graveyard) are remembered, not ones a replacement effect exiled ("X is the number of creatures that died this way": Hellfire).
        #[serde(default, skip_serializing_if = "is_false")]
        died_only: bool,
    },
    Exile {
        what: Sel,
        /// Return it at the beginning of the next end step (flicker).
        #[serde(default, skip_serializing_if = "is_false")]
        return_at_end: bool,
        /// Return it at the beginning of the next end step of its owner ("return that card to the battlefield under its owner's control at the beginning of that player's next end step": The Eternal Wanderer).
        #[serde(default, skip_serializing_if = "is_false")]
        return_at_owners_end: bool,
        #[serde(default, skip_serializing_if = "is_false")]
        face_down: bool,
        #[serde(default, skip_serializing_if = "is_false")]
        until_source_leaves: bool,
        /// With `until_source_leaves`: the zone the card goes back to (default: the battlefield; Brain Maggot: Hand).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        return_to: Option<Zone>,
        /// With `until_source_leaves`: don't return the cards when the source leaves; a later `ReturnLinkedExiles` (in the source's leaves-the-battlefield trigger) does (Day of the Dragons: sacrifice first, then return).
        #[serde(default, skip_serializing_if = "is_false")]
        manual_return: bool,
        /// With `until_source_leaves` returning to the battlefield: a card that comes back under the source controller's control gets this many +1/+1 counters (Lagrella, the Magpie).
        #[serde(default, skip_serializing_if = "is_zero_i32")]
        return_counters: i32,
        /// With `return_at_end`: each card returns with an additional +1/+1 counter if it is a creature and an additional loyalty counter if it is a planeswalker (Semester's End).
        #[serde(default, skip_serializing_if = "is_false")]
        return_bonus: bool,
    },
    /// Tag the selected exiled cards as exiled by the selected player (the first one): `MayPlayFrom.exiler` then lets that player play them (Shared Fate).
    SetExiledBy { what: Sel, who: Sel },
    /// Return the cards this (departed) source exiled with a `manual_return` Exile to the battlefield.
    ReturnLinkedExiles,
    /// Exile the selected permanents until an opponent of the effect's controller becomes the monarch (Palace Jailer); they return under their owners' control even if the source has left.
    ExileUntilOpponentMonarch { what: Sel },
    /// Move cards between zones.
    Move {
        what: Sel,
        to: Zone,
        #[serde(default, skip_serializing_if = "is_false")]
        top_of_library: bool,
        #[serde(default, skip_serializing_if = "is_false")]
        tapped: bool,
        #[serde(default, skip_serializing_if = "is_false")]
        under_your_control: bool,
        /// The cards enter the battlefield under the control of the first player this selects ("return it under an opponent's control": `EachOpponent`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        under_control_of: Option<Sel>,
        /// Several cards put on the library move in a random order ("put the rest on the bottom of your library in a random order").
        #[serde(default, skip_serializing_if = "is_false")]
        random_order: bool,
        /// The cards have no abilities as they enter, so their enter-the-battlefield triggers don't fire (Xu-Ifit).
        #[serde(default, skip_serializing_if = "is_false")]
        enters_bare: bool,
        /// The moved cards become `Remembered` (for a following GrantPlayFromExile, counters ...).
        #[serde(default, skip_serializing_if = "is_false")]
        remember: bool,
        /// Enters the battlefield transformed (back face up): "exile it, then return it transformed".
        #[serde(default, skip_serializing_if = "is_false")]
        transformed: bool,
        /// Only cards currently in this zone move ("shuffle it into its owner's library" does nothing if it already died).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<Zone>,
        /// Moving a card from hand to the graveyard counts as discarding it (raises the discard event).
        #[serde(default, skip_serializing_if = "is_false")]
        discard: bool,
        /// Into a library at this position from the top (0 = top, 1 = second from the top ...); needs `top_of_library`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        position: Option<u8>,
        /// The position is the ability's X ("just beneath the top X cards": Unexpectedly Absent); overrides `position`.
        #[serde(default, skip_serializing_if = "is_false")]
        position_x: bool,
        /// Several cards put on top of a library: the controller chooses their order ("in any order"; answer: one Pick of all of them, topmost first).
        #[serde(default, skip_serializing_if = "is_false")]
        ordered: bool,
        /// Several cards put on the bottom of a library: the controller chooses their order (answer: one Pick of all of them, topmost first); Lim-Dûl's Vault.
        #[serde(default, skip_serializing_if = "is_false")]
        bottom_ordered: bool,
        /// With `bottom_ordered` (or `ordered`): the player who chooses the order (default: the effect's controller). Morphic Tide: each owner
        /// orders their own cards (`EachObject` inside `ForEachPlayer`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        order_chooser: Option<Box<Sel>>,
        /// Enters the battlefield attacking (the same defender as the first attacker; Ninjutsu, Warcry Phoenix).
        #[serde(default, skip_serializing_if = "is_false")]
        attacking: bool,
        /// An Aura card put onto the battlefield (not cast): its controller chooses what it enchants (none legal: it stays put). Without this the Aura enters unattached (a later `Attach` effect places it).
        #[serde(default, skip_serializing_if = "is_false")]
        choose_host: bool,
        /// With `choose_host`: only a creature may be chosen as the host ("Only creatures can be enchanted this way": Retether). An Aura with no creature host stays where it is.
        #[serde(default, skip_serializing_if = "is_false")]
        host_creature: bool,
        /// With `choose_host`: the returned Aura can only enchant a permanent in this selection, and an Equipment card returned this way enters attached to it
        /// (the SBA unattaches an Equipment whose `AttachOnlyTo` doesn't match). Nothing returns if the selection is empty (Unfinished Business: `Remembered`).  An Aura with several legal hosts in the selection lets its controller choose (Storm Herald).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        host_only: Option<Box<Sel>>,
        /// With `choose_host`: the player who chooses what the Aura enchants (default: the effect's controller). Takklemaggot: the dead creature's controller (`ControllerOf` the trigger object).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        host_chooser: Option<Sel>,
        /// With `choose_host`: when the Aura has no legal host it still returns, as a non-Aura enchantment (it ceases to be an Aura and loses its "enchant" ability); `Condition::NonAura` is then true for it (Takklemaggot).
        #[serde(default, skip_serializing_if = "is_false")]
        non_aura_if_no_host: bool,
        /// Enters the battlefield face down (a 2/2 with no abilities).
        #[serde(default, skip_serializing_if = "is_false")]
        face_down: bool,
    },
    Bounce { what: Sel },
    /// For each selected permanent in turn, find a card with the same name in your library (a name can only be found once per selected permanent), put all the found cards
    /// onto the battlefield at the same time (`tapped`), then shuffle ("for each of those permanents, you may search ... for a card with the same name": Clarion Ultimatum).
    SearchSameNames { of: Sel, #[serde(default, skip_serializing_if = "is_false")] tapped: bool },
    /// Return the selected cards from the graveyard to the battlefield with `n` counters of `kind` on them (`n` is worked out before they move, so it can read
    /// their last-known counters: "return it ... with one fewer revival counter on it": Nine-Lives Familiar).
    ReturnWithCounters { what: Sel, kind: CounterKind, n: Amount },
    /// Counter every activated and triggered ability on the stack (Summary Dismissal).
    CounterAllAbilities,
    /// "Starting with you, each player may pay any amount of life. Repeat this process until no one pays life" (Plague of Vermin): rounds of every player (the controller first, then turn order)
    /// choosing a number of life to pay (0 to their life total) until a whole round passes with nobody paying; each player's total is read back with `Amount::LifePaidRounds`.
    PayLifeRounds,
    /// "Redistribute any number of players' life totals" (Reverse the Sands): the controller picks any number of players, then gives each of them one of the chosen players' current
    /// life totals (each total used once; the last player gets what is left); players then gain or lose the difference.
    RedistributeLife,
    /// Join forces: "starting with you, each player may pay any amount of mana" (Minds Aglow). The controller, then the other players in turn order, each choose a number from 0 to the generic mana
    /// they can pay (a player who can pay none isn't asked) and pay it (sources picked automatically); each player's payment is read back with `Amount::LifePaidRounds` (the total: all players).
    JoinForces,
    /// A life auction (Pain's Reward): the controller opens with a bid of 0 to their life total, then in turn order each player whose life exceeds the high bid may top it
    /// (a number at most the high bid passes); it ends when the high bid stands. The high bidder is Remembered (a player) and `Amount::BidOf` gives their bid. Nobody loses life by bidding.
    /// `opening`: the controller's fixed opening bid instead of choosing one (Illicit Auction: 0).
    /// `with`: only the controller and the selected players bid (Mages' Contest: "you and target spell's controller bid life"); everyone alive bids when absent.
    BidLife { #[serde(default, skip_serializing_if = "Option::is_none")] opening: Option<i32>, #[serde(default, skip_serializing_if = "Option::is_none")] with: Option<Sel> },
    /// "Each player hides at least `min` items, then all reveal" (Goblin Game): each player in turn order (the controller first) secretly chooses a number from `min` to their life total (at least `min`);
    /// the numbers are read with `Amount::BidOf` and `Condition::BidIsLowest`.
    HideNumbers { min: i32 },    /// Record that each selected player paid `amount` (added to their running tally, read with `Amount::PayTally`); `reset` first zeroes every player's tally. Use it in a `MayPay`'s effects inside `ForEachPlayer`
    /// so later effects can use what every player paid (Liege of the Hollows).
    PayTally {
        who: Sel,
        #[serde(default = "zero_amount")]
        amount: Amount,
        #[serde(default, skip_serializing_if = "is_false")]
        reset: bool,
    },
    /// The selected spells on the stack (the triggering spell of a `SpellCast` trigger) gain rebound: if cast from the hand, they are exiled as they resolve and cast free at your next upkeep (Ojer Pakpatiq).
    GainRebound { what: Sel },
    /// Counter every activated and triggered ability on the stack controlled by an opponent of the source's controller (Kadena's Silencer).
    CounterOpponentAbilities,
    /// Counter all spells and all abilities your opponents control; the source card of each object actually countered becomes `Remembered` (Glen Elendra's Answer: then one token per `RememberedCount`).
    CounterOpposingAll,
    CounterSpell {
        what: Sel,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unless_pays: Option<Vec<Cost>>,
        #[serde(default, skip_serializing_if = "is_false")]
        exile: bool,
        /// The countered spells' cards become `Remembered` (Hinder / Memory Lapse: then `Move Remembered` to the library).
        #[serde(default, skip_serializing_if = "is_false")]
        remember: bool,
        /// Remember the targeted spells' cards even when they were not countered (Arcane Denial: "its controller may draw ..." happens regardless).
        #[serde(default, skip_serializing_if = "is_false")]
        remember_targeted: bool,
        /// Also the abilities on the stack that match `what`'s controller filter (Whirlwind Denial: "for each spell and ability your opponents control").
        #[serde(default, skip_serializing_if = "is_false")]
        include_abilities: bool,
    },
    Pump {
        what: Sel,
        power: Amount,
        toughness: Amount,
        duration: Duration,
    },
    /// Set base power/toughness.
    SetBasePT {
        what: Sel,
        power: Amount,
        toughness: Amount,
        duration: Duration,
    },
    /// Set base power only (toughness untouched): Arni Brokenbrow, Belligerent Yearling.
    SetBasePower {
        what: Sel,
        power: Amount,
        duration: Duration,
    },
    /// Set base toughness only (power untouched): Wall of Tombstones, Sentinel ("base toughness becomes 1 plus ...", `Forever`).
    SetBaseToughness {
        what: Sel,
        toughness: Amount,
        duration: Duration,
    },
    GrantKeyword {
        what: Sel,
        keyword: Keyword,
        duration: Duration,
    },
    RemoveKeyword {
        what: Sel,
        keyword: Keyword,
        duration: Duration,
    },
    /// The selected permanents lose one protection quality ("loses protection from black": Cephalid Snitch); only that exact `from` entry is removed.
    RemoveProtection {
        what: Sel,
        from: ProtectionFrom,
        duration: Duration,
    },
    AddCounters {
        what: Sel,
        kind: CounterKind,
        n: Amount,
    },
    RemoveCounters {
        what: Sel,
        kind: CounterKind,
        n: Amount,
    },
    /// Put counters of the same kind as the triggering counter-added event on the selection ("put that many counters of that kind").
    AddCountersOfEventKind { what: Sel, n: Amount },
    /// The controller of the selected spell reveals their hand, then discards each card with the same name as a card spliced onto that spell (Minamo's Meddling).
    /// Put BEFORE the counter effect: the spell's splice record is gone once it leaves the stack.
    DiscardSplicedNames { of: Sel },
    /// Put one counter of EACH kind put in the triggering (batched) counter-added event on the selection ("one of each of those kinds": Aragorn, Company Leader).
    AddCountersOfEventKinds { what: Sel },
    /// Double the counters on each selected permanent: only `kind` if given, else every kind it has ("double the number of each kind of counter on it").
    DoubleCounters {
        what: Sel,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<CounterKind>,
    },
    /// "For each kind of counter on it, give it another counter of that kind" (Powerful Broker): +1 of every kind the permanent or player already has.
    AddOneOfEachCounter { what: Sel },
    /// "Choose a counter on [the first selected permanent]. Put an additional counter of that kind on it" (Ichormoon Gauntlet): the controller picks among the kinds already there (automatic with one; nothing with none).
    AddCounterOfChosenKind { what: Sel },
    /// "Choose a kind of counter on a creature you control. Put a counter of that kind on each other creature you control" (Contractual Safeguard): the controller picks one of
    /// the selected permanents that has counters (automatic with one), then a kind among its counters (automatic with one); each OTHER selected permanent gets one counter of that kind.
    SpreadCounterKind { among: Sel },
    /// "Choose a counter on a permanent you control. Put a counter of that kind on target permanent you control if it doesn't have a counter of that kind on it" (Aven Courier):
    /// the controller picks a kind among the counters on the `among` permanents (automatic with one); each `to` permanent lacking that kind gets one.
    ChooseCounterKindAdd { among: Sel, to: Sel },
    /// "For each selected permanent, choose a counter on it. You may put an additional counter of that kind on that permanent" (The Caves of Androzani): in turn, the controller picks a kind
    /// among the permanent's counters (automatic with one kind), then answers yes/no.
    BumpChosenCounterKind { what: Sel },
    /// Put `n` +1/+1 counters on the selected permanents as part of a modular trigger: `Replace::AddCounters.modular` replacements apply (Zabaz, the Glimmerwasp).
    ModularCounters { what: Sel, n: Amount },
    /// Link this permanent to the first `Remembered` permanent on the battlefield (Diabolic Servitude: "the creature put onto the battlefield with this"); read with `Filter.linked_with_this`.
    LinkRemembered,
    /// Forget every permanent this source linked itself to with `LinkRemembered` (Giant Oyster, when it untaps or leaves).
    UnlinkAll,
    /// "For each kind of counter on target permanent, put another counter of that kind on it or remove one from it" (Quarry Hauler): the controller answers yes (add) or no (remove) once per kind present, in turn.
    AdjustEachCounterKind { what: Sel },
    /// "Choose a counter on target permanent or suspended card. Remove that counter from it or put another of those counters on it" (Clockspinning): the controller picks one kind of counter on the first selected permanent / exiled card, then adds one more of that kind or removes one.
    ChooseCounterAdjust { what: Sel },
    /// The selected creatures must attack the selected player this combat / for the duration if able ("Ruhan attacks that player each combat if able", Taunt).
    MustAttackPlayer { what: Sel, player: Sel, duration: Duration },
    /// "Choose an opponent at random that this creature didn't attack during your last combat. This creature attacks that player this combat if able. If you can't choose an opponent this way, tap this creature" (Territorial Hellkite).
    AttackRandomOpponentNotLast,
    /// All creatures the selected player controls (also ones that arrive later) must attack `player` this turn if able (Taunt, from a delayed trigger at the target's upkeep).
    TauntAllCreatures { who: Sel, player: Sel },
    /// Each creature the selected players control attacks this turn if able ("during target player's next turn, each creature that player controls attacks if able": Rowan Kenrith, run from a `Delayed` at that player's upkeep).
    AttackAllThisTurn { who: Sel },
    /// The effect's controller ignores this source's player-filtered effect (`PlayerFilter::MorePermanentsThanEachOther`) until end of turn (Damping Engine: "may sacrifice a permanent to ignore this effect until end of turn").
    IgnoreSourceEffect,
    /// The selected creatures must attack the selected planeswalker/battle if able for the duration (Gideon, Battle-Forged +2).
    MustAttackPermanent { what: Sel, permanent: Sel, duration: Duration },
    /// Remove every counter (of every kind) from each selected permanent.
    RemoveAllCounters { what: Sel },
    /// The selected permanents or players can't have counters put on them / get counters for as long as the source stays on the battlefield (Suncleanser).
    NoCountersWhileSource { what: Sel },
    /// Remove up to `n` counters of any kind(s) in total from each selected permanent (most plentiful kind first).
    RemoveAnyCounters {
        what: Sel,
        n: Amount,
        /// "Remove up to N": the player may stop early (asked before each counter).
        #[serde(default, skip_serializing_if = "is_false")]
        up_to: bool,
    },
    /// Move counters from the first selected permanent to the first selected `to`: `n` counters (all when absent) of `kind` (every kind when absent; with an `n` and no kind, the most plentiful kind).
    MoveCounters {
        from: Sel,
        to: Sel,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<CounterKind>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        n: Option<Amount>,
        /// "Move any number of counters from among [the selected permanents] onto `to`": `from` selects every source (the destination is skipped); for each, the player repeatedly picks a counter kind to move or stops (up to `n` each, unlimited when absent).
        #[serde(default, skip_serializing_if = "is_false")]
        up_to: bool,
        /// With `up_to`: the player must move at least one counter when the source has any ("move one or more counters": Goldberry, River-Daughter); `Amount::CountersRemoved` is then the number moved.
        #[serde(default, skip_serializing_if = "is_false")]
        at_least_one: bool,
        /// Instead of a chosen amount: move ONE counter of each kind that is on the source and not on the destination ("a counter of each kind not on Goldberry"); `Amount::CountersRemoved` is the number moved.
        #[serde(default, skip_serializing_if = "is_false")]
        each_missing: bool,
    },
    CreateToken {
        /// Name of a token card definition.
        token: String,
        #[serde(default = "one_amount")]
        n: Amount,
        #[serde(default = "you")]
        controller: Sel,
        #[serde(default, skip_serializing_if = "is_false")]
        tapped: bool,
        #[serde(default, skip_serializing_if = "is_false")]
        attacking: bool,
        /// The token (an Aura such as a Role) enters attached to this permanent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attach_to: Option<Sel>,
        /// The token's base power / toughness computed when it is created ("X/X token where X is ..."); replaces the token definition's.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        set_pt: Option<(Amount, Amount)>,
        /// With `attacking`: the token attacks the first player this selects (Ellie, Brick Master: "attacking that opponent") instead of the first attacker's defender.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attack_at: Option<Sel>,
        /// With `attacking`: the controller chooses which attacked player / planeswalker / battle the token attacks (Mordor Trebuchet).
        #[serde(default, skip_serializing_if = "is_false")]
        choose_defender: bool,
    },
    /// You get an emblem: a token card definition (no types) whose abilities work from the command zone (`from: Command`).
    CreateEmblem {
        emblem: String,
        /// The player who gets the emblem instead of the controller (Ob Nixilis Reignited: "target opponent gets an emblem").
        #[serde(default, skip_serializing_if = "Option::is_none")]
        who: Option<Sel>,
        /// The first selected object (a permanent or a player) is bound to the emblem: read back inside its abilities with `Sel::BoundCard` (a permanent) or `Sel::ChosenPlayer` (a player). Guardian Angel.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bind: Option<Sel>,
        /// The emblem ceases to exist in the cleanup step ("until end of turn, you may pay {1} any time you could cast an instant").
        #[serde(default, skip_serializing_if = "is_false")]
        until_end_of_turn: bool,
    },
    /// Create a token that is a copy of a permanent.
    CopyPermanent {
        what: Sel,
        #[serde(default = "one_amount")]
        n: Amount,
        /// "... except it's an artifact in addition", "except it isn't legendary", "it gains haste".
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        except: Vec<CopyExcept>,
        /// The token copy enters tapped.
        #[serde(default, skip_serializing_if = "is_false")]
        tapped: bool,
        /// The token copy enters attacking (under the effect's controller, attacking the same player / permanent as normal late attackers).
        #[serde(default, skip_serializing_if = "is_false")]
        attacking: bool,
        /// The player who controls the token copies instead of the effect's controller ("target player creates a token that's a copy of ..."; one token per player selected).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        by: Option<Sel>,
        /// A token copy of an Aura / Equipment enters attached to the first permanent this selects (`EachObject` inside a `ForEach`: Three Dog, Galaxy News DJ); default: the first legal host for an Aura.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attach_to: Option<Sel>,
        /// With `attacking`: the copy attacks the first player this selects ("for each opponent, ... attacking that opponent": `EachObject` inside a `ForEachPlayer`; Mardu Siegebreaker).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attack_at: Option<Box<Sel>>,
    },
    /// Copy a spell on the stack (the copy is controlled by the effect's controller); `new_targets`: they may choose new targets.
    CopySpell {
        what: Sel,
        #[serde(default, skip_serializing_if = "is_false")]
        new_targets: bool,
        /// The player who controls (and, with `new_targets`, retargets) the copies instead of the effect's controller
        /// ("its controller may copy this spell": `{"type":"ControllerOf","of":{"type":"Target","idx":0}}`; demonstrate: an opponent).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        by: Option<Sel>,
        /// "except ..." clauses for a copy of a permanent spell (it becomes a token with these changes: not legendary, a name, added types ...).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        except: Vec<CopyExcept>,
        /// Each copy's first target is forced to be this object ("the copy targets that token", Zada: one copy per creature under ForEach); no retarget choice is offered.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<Sel>,
        /// One copy for each other permanent or player the spell / ability (which has a single target) could target; each copy targets a different one (Radiant Performer).
        #[serde(default, skip_serializing_if = "is_false")]
        each_other_target: bool,
        /// Make one copy of the (single-target) spell or ability for the first selected opponent: the copy's controller chooses that player or a permanent they control as its target.
        /// No copy when that player is the spell's target or controls its target permanent (Zevlor: run it under `ForEachPlayer` over the opponents).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        for_opponent: Option<Sel>,
        /// The copy is made even when the spell was countered (left the stack) before this resolves ("copy that spell" triggers whose rulings say so: Swarm Intelligence, Thousand-Year Storm).
        #[serde(default, skip_serializing_if = "is_false")]
        even_if_gone: bool,
    },
    /// The remembered cards still in their owners' hands are discarded at the same time (each fires `Discarded`); they stay remembered (Mog, Moogle Warrior).
    DiscardRemembered,
    /// Note the type of mana spent to pay this ability's activation cost on the source (the first type paid); read by `ManaProduce::NotedType` (Jeweled Amulet).
    NoteManaSpent,
    /// Note the type and amount of all the mana spent to pay this ability's activation cost on the source; read by `ManaProduce::NotedMana` (Ice Cauldron).
    NoteManaSpentAll,
    Discard {
        who: Sel,
        #[serde(default = "one_amount")]
        n: Amount,
        #[serde(default, skip_serializing_if = "is_false")]
        random: bool,
        #[serde(default)]
        filter: FilterBox,
    },
    Mill {
        who: Sel,
        n: Amount,
    },
    /// The selected creatures count as having enlisted a creature this turn (set by the Enlist keyword).
    MarkEnlisted { what: Sel },
    /// Mark the selected spell (a face-down creature spell cast by Illusionary Mask): the creature it becomes is turned face up instead of dealing / being dealt damage or becoming tapped, once.
    MarkUnmask { what: Sel },
    /// Creatures dealt damage this turn by the selected card (now or later this turn) can't be regenerated this turn (Runesword).
    CantRegenerateDamagedBy { by: Sel },
    Tap { what: Sel },
    Untap { what: Sel },
    /// Soulbond: pair the two creatures (both must be unpaired creatures controlled by the same player).
    Pair { a: Sel, b: Sel },
    /// The selected permanents phase out (they, and anything attached to them, phase in again during their controller's next untap step).
    /// Unlock a locked door of each selected Room (the lowest locked door first).
    UnlockDoor { what: Sel },
    /// Lock (`locked: true`) or unlock one specific door (1 or 2) of each selected Room, if it isn't already in that state (Marina Vendrell, Keys to the House: four modes).
    SetDoor { what: Sel, door: u8, locked: bool },
    PhaseOut {
        what: Sel,
        /// Oubliette / Out of Time: they stay phased out (they don't phase in during untap) until this source leaves the battlefield.
        #[serde(default, skip_serializing_if = "is_false")]
        until_source_leaves: bool,
        /// Teferi, Timeless Voyager: they can't phase in until the end of the controller's next turn.
        #[serde(default, skip_serializing_if = "is_false")]
        lock_until_your_next_turn_ends: bool,
    },
    /// The selected phased-out permanents phase in (Time and Tide, The Pandorica).
    PhaseIn { what: Sel },
    /// This Mount becomes saddled until end of turn (Saddle N = `Activated` with `Cost::Crew {power: N}`, sorcery timing, and this effect).
    Saddle,
    /// The selected Mounts become saddled until end of turn ("target Mount you control becomes saddled until end of turn": Guidelight Matrix, Alacrian Armory).
    SaddleTargets { what: Sel },
    /// Airbend: exile the selected permanents/cards; their OWNER may cast each from exile for `cost` (a mana string, e.g. "{2}") for as long as it stays there.
    Airbend { what: Sel, cost: String },
    Sacrifice {
        who: Sel,
        #[serde(default)]
        filter: FilterBox,
        #[serde(default = "one_amount")]
        n: Amount,
    },
    /// `decider`: the player who chooses which cards go to the bottom when it isn't the library's owner (Ransack: the caster decides for the target player's library).
    Scry { who: Sel, n: Amount, #[serde(default, skip_serializing_if = "Option::is_none")] decider: Option<Box<Sel>> },
    Surveil { who: Sel, n: Amount },
    Shuffle { who: Sel },
    SearchLibrary {
        who: Sel,
        filter: FilterBox,
        #[serde(default = "one_amount")]
        n: Amount,
        #[serde(default = "to_hand")]
        to: Zone,
        #[serde(default = "yes", skip_serializing_if = "is_true")]
        shuffle: bool,
        #[serde(default, skip_serializing_if = "is_false")]
        tapped: bool,
        #[serde(default, skip_serializing_if = "is_false")]
        reveal: bool,
        /// The found card enters under the control of whoever controls this effect (searching an opponent's library: Inevitable Betrayal).
        #[serde(default, skip_serializing_if = "is_false")]
        under_your_control: bool,
        /// The found card enters under this player's control (Yavimaya Dryad: "under target player's control").
        #[serde(default, skip_serializing_if = "Option::is_none")]
        under_control_of: Option<Sel>,
        /// The player whose library it is makes the choice ("that player searches their library"), not the effect's controller.
        #[serde(default, skip_serializing_if = "is_false")]
        by_owner: bool,
        /// A found card that matches `instead.filter` (judged against the source) goes to `instead.to` rather than `to`, when `instead.cond` holds (Celestial Reunion).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        instead: Option<Box<SearchInstead>>,
        /// "Search your library and/or graveyard": matching cards in the searcher's graveyard can be found too; the library is shuffled only if the card came from the library (Captain Kirk, Boldly Going).
        #[serde(default, skip_serializing_if = "is_false")]
        graveyard_too: bool,
    },
    /// Look at the top n cards; put some in hand / the rest on bottom / graveyard.
    LookAtTop {
        who: Sel,
        n: Amount,
        take: u8,
        filter: FilterBox,
        rest: Zone,
        /// The cards that aren't taken stay on top of the library instead of going to `rest`.
        #[serde(default, skip_serializing_if = "is_false")]
        keep_on_top: bool,
        /// Where the taken cards go (default: the hand).
        #[serde(default = "zone_hand", skip_serializing_if = "is_zone_hand")]
        to: Zone,
        /// The taken cards enter the battlefield tapped.
        #[serde(default, skip_serializing_if = "is_false")]
        tapped: bool,
        /// Take every matching card (no choice).
        #[serde(default, skip_serializing_if = "is_false")]
        all: bool,
        /// The choice is mandatory: exactly `take` cards (or all matching if fewer).
        #[serde(default, skip_serializing_if = "is_false")]
        must: bool,
        /// The rest go to `rest` in a random order.
        #[serde(default, skip_serializing_if = "is_false")]
        random_rest: bool,
        /// The controller chooses the order the rest go to the bottom of the library in ("in any order").
        #[serde(default, skip_serializing_if = "is_false")]
        rest_ordered: bool,
        /// The chosen cards stay on top of the library and the controller orders them ("put those cards on top in any order", Munda, Ambush Leader).
        #[serde(default, skip_serializing_if = "is_false")]
        top_ordered: bool,
        /// The cards that aren't taken stay on top of the library and the controller puts them back in any order (Dimir Machinations; use with `keep_on_top`).
        #[serde(default, skip_serializing_if = "is_false")]
        rest_top_ordered: bool,
        /// The taken cards are put into `to` (exile) face down (The Dragon-Kami Reborn).
        #[serde(default, skip_serializing_if = "is_false")]
        face_down: bool,
        /// The taken cards must all differ in this respect ("any number of cards with different powers": Rip, Spawn Hunter); a later duplicate pick is dropped.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        distinct: Option<ChooseDistinct>,
        /// The taken cards enter the battlefield with this counter already on them (Arbiter of the Ideal's manifestation counter).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        enters_with: Option<CounterKind>,
    },
    AddMana {
        mana: Vec<ManaProduce>,
        /// The mana doesn't empty at the end of steps: it lasts until the end of combat (firebending).
        #[serde(default, skip_serializing_if = "is_false")]
        until_end_of_combat: bool,
        /// The mana doesn't empty as steps and phases end this turn ("you don't lose this mana as steps and phases end").
        #[serde(default, skip_serializing_if = "is_false")]
        until_end_of_turn: bool,
        /// The player who gets the mana (default: the effect controller). Vernal Bloom: TriggerPlayer.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        who: Option<Sel>,
        /// "A player of your choice adds ..." in a mana ability (Victory Chimes): when the permanent is tapped for floating mana by hand (`Choice::Mana`) the controller is asked which player gets the mana; when it is tapped to pay a cost, or by the AI, the controller gets it.
        #[serde(default, skip_serializing_if = "is_false")]
        player_choice: bool,
    },
    GainControl {
        what: Sel,
        duration: Duration,
        /// The player who gains control (default: the effect's controller).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to: Option<Sel>,
        /// The control effect belongs to this object instead of the ability's source (with `Duration::WhileSourceAttachedToAffected`: the Aura whose attachment the control lasts for; Eriette, the Beguiler).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        as_source: Option<Sel>,
    },
    /// Every player gains control (permanently) of all permanents matching `filter` controlled by the next player to the left (`right: false`) or right (`right: true`) at once (Aminatou, the Fateshifter).
    /// `chosen`: instead of every matching permanent, the effect's controller picks ONE matching permanent of the neighbour for each player (Inniaz, the Gale Force); all changes happen together.
    RotateControl { filter: FilterBox, right: bool, #[serde(default, skip_serializing_if = "is_false")] chosen: bool },
    /// Exchange control of two permanents (each one's controller gains control of the other).
    /// `destroy_auras`: when the exchange happens, destroy all Auras attached to the two permanents (Gauntlets of Chaos).
    ExchangeControl { a: Sel, b: Sel, duration: Duration, #[serde(default, skip_serializing_if = "is_false")] destroy_auras: bool },
    /// "You control target player during that player's next turn" (Mindslaver, Construct a Cosmic Cube): the first selected player's decisions during their next turn are made by the effect's controller.
    /// The game surfaces those decisions with `Decision.player` set to the controller (`Game.turn_ctl`, `Game.remapped_at`).
    /// `by`: the controlling player when it is not the effect's controller (Cruel Entertainment). `combat_only`: only the next combat phase (Secret of Bloodbending).
    /// "You control target player until this finishes resolving" (Word of Command): while `effects` run, the first selected player's decisions are made by the effect's controller.
    /// Change the text of the first selected permanent or spell (CR 612): replace one colour word, basic land type or creature type (`kinds` says which may be chosen) with another.
    /// The controller picks the old word among those that appear in the text and the new one (or `to` is fixed; `exclude_new` can't be chosen). Lasts `duration` (`Forever` / `UntilEndOfTurn`).
    ChangeText { what: Sel, kinds: Vec<TextKind>, duration: Duration, #[serde(default, skip_serializing_if = "Option::is_none")] to: Option<String>, #[serde(default, skip_serializing_if = "Vec::is_empty")] exclude_new: Vec<String> },
    /// The two selected permanents exchange text boxes (Deadpool, Trading Card).
    /// `while_source`: the exchange ends when the source permanent leaves the battlefield (Exchange of Words).
    ExchangeTextBoxes { a: Sel, b: Sel, #[serde(default, skip_serializing_if = "is_false")] while_source: bool },
    /// `lands_only_mana`: while `effects` run the player can use only mana abilities of lands (Word of Command).
    ControlPlayerDuring { who: Sel, effects: Vec<Effect>, #[serde(default, skip_serializing_if = "is_false")] lands_only_mana: bool },
    /// The effect's controller also controls the first selected player while the selected spell (on the stack now) resolves (Word of Command's cast spell).
    ControlPlayerWhileResolves { who: Sel, what: Sel },
    /// Internal: the end of a `ControlPlayerDuring`.
    EndPlayerControl,
    ControlPlayerNextTurn { who: Sel, #[serde(default, skip_serializing_if = "Option::is_none")] by: Option<Sel>, #[serde(default, skip_serializing_if = "is_false")] combat_only: bool },
    Fight { a: Sel, b: Sel },
    /// Simultaneous fights: the i-th selected creature of `a` fights the i-th of `b` (surplus entries on either side are left out); every power is read before any damage is dealt (Ezuri's Predation).
    FightEach { a: Sel, b: Sel },
    /// `a` deals damage equal to its power to `b`.
    Bite { a: Sel, b: Sel },
    Transform { what: Sel },
    /// "Becomes a 2/2 Elemental creature with haste until end of turn" (manlands, vehicles, animated artifacts).
    /// The selected permanents lose all abilities for the duration (Nightcreep: lands that become Swamps).
    LoseAllAbilities { what: Sel, duration: Duration },
    /// Until the duration, the selected permanents lose all subtypes of the group and gain `subtypes` (Ego Erasure: creature types).
    SetSubtypesOfKind {
        what: Sel,
        kind: SubtypeKind,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        subtypes: Vec<String>,
        /// Use the type this source chose (`ChooseCreatureType` with `only` basic land types earlier in the same effect list; Unstable Frontier).
        #[serde(default, skip_serializing_if = "is_false")]
        chosen: bool,
        duration: Duration,
    },
    Animate {
        what: Sel,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        add_types: Vec<CardType>,
        /// Card types it loses ("becomes a creature ... it's no longer an enchantment": Lurking Evil).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        remove_types: Vec<CardType>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        add_subtypes: Vec<String>,
        /// Replaces ALL its subtypes ("becomes an Island", "becomes a Goblin").
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        set_subtypes: Vec<String>,
        /// Replaces all its subtypes with the type this source chose (`ChooseCreatureType` earlier in the same effect list; Mistform cycle).
        #[serde(default, skip_serializing_if = "is_false")]
        set_chosen_subtype: bool,
        /// Adds the creature type / basic land type this source chose to its subtypes ("is the chosen type in addition to its other types").
        #[serde(default, skip_serializing_if = "is_false")]
        add_chosen_subtype: bool,
        /// Replaces its colors with the color this source chose ("becomes the color of your choice", `ChooseColor` earlier in the same effect list).
        #[serde(default, skip_serializing_if = "is_false")]
        set_chosen_color: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        power: Option<i32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        toughness: Option<i32>,
        /// Base power and toughness X/X from an amount worked out as the effect resolves (Cactus Preserve: the greatest mana value among your commanders); overrides `power` / `toughness`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pt_amount: Option<Amount>,
        /// Replaces its colors.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        colors: Option<Vec<Color>>,
        /// Gains these colors in addition to its others ("a black Zombie in addition to its other colors and types").
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        add_colors: Vec<Color>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        keywords: Vec<Keyword>,
        /// Gains these supertypes ("becomes legendary", "becomes snow").
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        add_supertypes: Vec<Supertype>,
        /// Loses these supertypes ("is no longer snow").
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        remove_supertypes: Vec<Supertype>,
        /// Becomes named this.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        set_name: Option<String>,
        /// Becomes named after the creature type this source chose (Volrath's Laboratory tokens).
        #[serde(default, skip_serializing_if = "is_false")]
        set_name_chosen_subtype: bool,
        duration: Duration,
    },
    /// `steal`: whoever resolves it gains control of the creature if the shield is used to regenerate it (Debt of Loyalty).
    Regenerate {
        what: Sel,
        #[serde(default, skip_serializing_if = "is_false")]
        steal: bool,
    },
    /// "It can't be regenerated this turn."
    CantRegenerate { what: Sel },
    /// "Damage that would be dealt to that creature this turn can't be prevented or dealt instead to another permanent or player" (Whippoorwill).
    DamageLockThisTurn { what: Sel },
    /// The next time each selected permanent would be destroyed this turn, all damage marked on it is removed instead (Pyramids).
    DestroyHealShield { what: Sel },
    /// `remember`: the permanents that received a counter this way become `Remembered` (Ripples of Potential).
    /// "Choose a counter on target permanent or player. Give that permanent or player another counter of that kind": the controller picks one of the kinds the first selected object has (players: poison, energy, experience) and one more of it is added (Animation Module).
    DuplicateCounter { what: Sel },
    Proliferate {
        #[serde(default, skip_serializing_if = "is_false")]
        remember: bool,
    },
    Investigate { n: Amount },
    /// Amass N ("Amass Orcs N": `subtype` is added to the Army permanently).
    Amass {
        n: Amount,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subtype: Option<String>,
    },
    Explore { what: Sel },
    /// Each player shuffles all permanents they own into their library (tokens cease to exist), then reveals that many cards from the top of their library; the revealed cards (still on top of the libraries) are `Remembered`, to be moved on by following effects (Morphic Tide: `Move` them from the Library in waves).
    ReshuffleAndReveal,
    /// Each selected creature connives, one at a time: its controller draws a card, then discards a card; if a nonland card was discarded the creature gets a +1/+1 counter.
    Connive { what: Sel },
    /// Myriad: for each opponent other than the defending player, a token copy of the source enters tapped and attacking that player; exiled at end of combat.
    Myriad,
    /// Goad: until your next turn the creature attacks each combat if able and attacks a player other than you if able.
    Goad {
        what: Sel,
        /// Goaded for the rest of the game (Rendmaw, Creaking Nest).
        #[serde(default, skip_serializing_if = "is_false")]
        forever: bool,
        /// Goaded only for as long as someone other than the goader controls it (Vislor Turlough); supersedes `forever`.
        #[serde(default, skip_serializing_if = "is_false")]
        while_controlled_by_other: bool,
        /// Goaded only while the source stays on the battlefield (Hot Pursuit); supersedes `forever`.
        #[serde(default, skip_serializing_if = "is_false")]
        while_source_on_battlefield: bool,
    },
    /// The effect's controller looks at the selected face-down permanents (information only; Smoke Teller).
    LookAtFaceDown { what: Sel },
    /// The selected players can't attack the effect's controller or permanents that player controls during their next turn (Willie Lumpkin, Postman).
    CantAttackYouNextTurn { player: Sel },
    /// The creatures the selected players control can't attack the effect's controller or planeswalkers / battles that player controls until the controller's next turn begins (Orzhov Advokist).
    CantAttackYouUntilYourNextTurn { player: Sel },
    /// Until your next turn you can't be attacked except by creatures with flying and/or islandwalk (Island Sanctuary); planeswalkers you control can still be attacked.
    SanctuaryUntilYourNextTurn,
    /// Until end of turn the controller may tap lands they don't control for mana; that mana can be spent only to cast spells (Piracy; `Player.foreign_lands_turn`).
    TapForeignLandsThisTurn,
    /// Until end of turn, any time the controller could activate a mana ability they may pay 1 life to add {C} (Channel; `Player.life_mana`, virtual mana sources in `mana_sources_for_cost`).
    LifeManaThisTurn,
    /// "Choose a color": the chooser picks; remembered on the source for `Filter.chosen_color` and `GrantProtectionChosenColor`.
    /// `per_player`: the choice is stored per chooser on the source (kept for every player in a `ForEachPlayer`) for `Filter.only_controllers_chosen_color` / `not_only_controllers_chosen_color` (Selective Obliteration); it does not set the source's own chosen colour.
    ChooseColor { who: Sel, #[serde(default, skip_serializing_if = "is_false")] per_player: bool },
    /// The controller chooses one or more colors; the selected permanents become exactly those colors for `duration` (Prismwake Merrow: "the color or colors of your choice").
    BecomeChosenColors { what: Sel, duration: Duration },
    /// "Becomes the color or colors of your choice": `who` picks one to five colors, which replace the colors of the selected permanents (Dream Coat).
    SetColorsOfChoice { who: Sel, what: Sel, duration: Duration },
    /// "Choose a land type and a basic land type. Each land of the first chosen type becomes the second chosen type until end of turn" (Vision Charm): `who` picks any land subtype
    /// (basic or not), then a basic land type; every land with the first type loses its land types and abilities for the duration and gets the second (with its intrinsic mana ability).
    ChangeLandTypes { who: Sel, duration: Duration },
    /// "Choose a creature type": remembered on the source for `Filter.chosen_type`.
    ChooseCreatureType {
        who: Sel,
        /// Only these creature types may be chosen (empty = any): "choose Human, Merfolk or Goblin".
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        only: Vec<String>,
        /// These creature types may not be chosen ("choose a creature type other than Wall").
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        excluding: Vec<String>,
        /// Choose a nonbasic land type instead (any subtype of a land card other than the five basic types): March from Velis Vel. `Filter.chosen_type` then matches lands of that type.
        #[serde(default, skip_serializing_if = "is_false")]
        nonbasic_land_type: bool,
        /// Choose any land type (basic or not) instead (Illusionary Presence); `Filter.chosen_type` matches lands of that type.
        #[serde(default, skip_serializing_if = "is_false")]
        any_land_type: bool,
        /// Choose a planeswalker type (any subtype of a planeswalker card) instead: `Filter.chosen_type` then matches planeswalkers of that type (Leori, Sparktouched Hunter).
        #[serde(default, skip_serializing_if = "is_false")]
        planeswalker_type: bool,
        /// Keep every choice made on this source's resolution (one per player, in a `ForEachPlayer`) instead of replacing the last one; `Filter.chosen_type_any` matches any of them (Patriarch's Bidding).
        #[serde(default, skip_serializing_if = "is_false")]
        accumulate: bool,
        /// Types this source already noted (`note`) can't be chosen ("a creature type that hasn't been noted for this Saga": Long List of the Ents).
        #[serde(default, skip_serializing_if = "is_false")]
        exclude_noted: bool,
        /// The chosen type is also noted on this source (`Card.noted_types`, kept until the source leaves the battlefield).
        #[serde(default, skip_serializing_if = "is_false")]
        note: bool,
    },
    /// "Note one of its creature types that hasn't been noted for this permanent": the source's controller picks one creature type of the first selected card (a spell: every creature type for a changeling)
    /// that this source hasn't noted yet and the source records it (read back with `Amount::NotedTypes`). Nothing happens when there is none.
    NoteCreatureType { of: Sel },
    /// Choose a card type (stored like the creature type: `Filter.chosen_type` / `not_chosen_type` match the card type).
    ChooseCardType {
        who: Sel,
        /// These card types may not be chosen ("choose a card type other than creature").
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        excluding: Vec<String>,
    },
    /// "Choose a number between min and max": read back with `Amount::ChosenNumber`.
    /// Remember a number worked out now, read back later with `Amount::ChosenNumber` ("draws cards equal to the greatest number discarded this way": store it before the discard).
    StoreNumber { amount: Amount },
    /// "Collect evidence X" at resolution, X being the chosen number (`ChooseNumber` first): exiles cards from the controller's graveyard with total mana value X or more (biggest first) and makes them Remembered; nothing is remembered when X is 0 or the graveyard can't cover X (Incinerator of the Guilty).
    CollectEvidenceChosen,
    ChooseNumber {
        who: Sel,
        min: i32,
        max: i32,
        /// A computed maximum instead of `max` ("choose a number up to your life total / the number of cards in hand").
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_amount: Option<Amount>,
        /// Nobody is asked: a number from `min` to `max` is picked at random (the seeded RNG, no die roll: "choose 1, 2, or 3 at random", Tibalt's Trickery).
        #[serde(default, skip_serializing_if = "is_false")]
        random: bool,
        /// Also record the number for the chooser (`Game.player_numbers`): read back with `Amount::PlayerNumber` / `Amount::HighestPlayerNumber` / `Sel::HighestNumberPlayers`
        /// ("each player secretly chooses a number", Menacing Ogre; the choices are made one after another but the decisions are hidden). Run `ResetPlayerNumbers` first.
        #[serde(default, skip_serializing_if = "is_false")]
        record: bool,
        /// Numbers this source already chose with `exclude_chosen` can't be chosen again ("a number between 1 and 5 that hasn't been chosen": The Toymaker's Trap). Nothing happens when none is left.
        #[serde(default, skip_serializing_if = "is_false")]
        exclude_chosen: bool,
    },
    /// Forget the numbers recorded by `ChooseNumber { record }`.
    ResetPlayerNumbers,
    /// "You choose how each player votes this turn" (Illusion of Choice): every ballot this turn is cast by the effect's controller.
    ControlVotesThisTurn,
    /// The first selected player becomes this source's linked player, read back with `Sel::ChosenPlayer` ("target player loses 3 life. When this leaves the battlefield, that player gains 3 life").
    LinkPlayer { what: Sel },
    /// "Choose a player" (`opponent` limits it to the chooser's opponents): read back with `Sel::ChosenPlayer`.
    ChoosePlayer {
        who: Sel,
        #[serde(default, skip_serializing_if = "is_false")]
        opponent: bool,
        /// Only players matching this (relative to the chooser) may be chosen: `MostLife` ("choose a player with the most life or tied").
        #[serde(default)]
        among: PlayerFilter,
        /// The player is chosen at random (equal chances) instead of by `who`'s decision ("choose a player at random").
        #[serde(default, skip_serializing_if = "is_false")]
        random: bool,
        /// Players selected by this (usually `Stored{slot}` holding earlier choices) can't be chosen: "choose a second player" (Gluntch, the Bestower).
        /// When nobody is left, the source's chosen player is cleared.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exclude: Option<Sel>,
    },
    /// The selected permanents gain protection from something for a while.
    GrantProtection { what: Sel, from: ProtectionFrom, duration: Duration },
    /// The selected permanents gain every protection quality that any permanent selected by `among` currently has (Concerted Effort: "gain protection if a creature you control has protection").
    GrantProtectionSharedFrom { what: Sel, among: Sel, duration: Duration },
    /// The selected permanents gain a static effect for `duration` ("target creature can't block this turn", "can't be blocked by creatures
    /// with power 2 or less this turn", "must attack", "can block any number of creatures"). The effect's own `affects` filter is ignored;
    /// supported: CantBlock, CantAttack, MustAttack, CantBeBlockedBy, CanBlockExtra, GrantKeyword, GrantAbility, Modify.
    /// With `dynamic: true` the effect's own `affects` filter is used instead of `what` and also covers permanents that arrive later this turn
    /// ("creatures can't block this turn", "creatures your opponents control can't block").
    GrantStatic {
        what: Sel,
        effect: StaticEffect,
        duration: Duration,
        #[serde(default, skip_serializing_if = "is_false")]
        dynamic: bool,
    },
    /// The selected permanents gain protection from the color chosen on this source.
    GrantProtectionChosenColor { what: Sel, duration: Duration },
    /// The selected permanents don't untap during their controller's next untap step.
    /// With `of_targets`, the skip is that targeted player's next untap step for every selected permanent (Blinding Beam), so another player's "untap during each other player's untap step" effects (Seedborn Muse) don't undo it.
    SkipNextUntap {
        what: Sel,
        #[serde(default)]
        of_targets: bool,
    },
    /// Put the selected cards onto the battlefield under the control of the first selected player ("puts target creature card onto the battlefield under their control"); they become `Remembered`.
    MoveUnder {
        what: Sel,
        player: Sel,
        #[serde(default, skip_serializing_if = "is_false")]
        tapped: bool,
    },
    /// Exert the selected creatures: they skip their controller's next untap step, and "whenever you exert a creature" triggers fire.
    Exert { what: Sel },
    /// Until your next turn, creatures can't attack you or planeswalkers you control (`tax` 0), or can't unless their controller pays `tax` generic for each.
    ShieldFromAttacks {
        #[serde(default, skip_serializing_if = "is_zero_i32")]
        tax: i32,
    },
    /// This turn the effect's controller can't attack the selected players (not their planeswalkers): "You can't attack that player this turn" (Call for Aid).
    /// `attacker`: these players are barred (from attacking `player`) instead of the controller; `planeswalkers`: they can't attack the planeswalkers `player` controls either
    /// ("they can't attack you or planeswalkers you control this turn": Sandswirl Wanderglyph).
    CantAttackPlayerThisTurn {
        player: Sel,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attacker: Option<Sel>,
        #[serde(default, skip_serializing_if = "is_false")]
        planeswalkers: bool,
        /// Instead of barring the players: the barred attackers can't attack only the permanents `player` controls that match this filter ("creatures they control can't attack Jaces you control this turn": Jace, Multiverse Architect).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        only_permanents: Option<Box<Filter>>,
    },
    /// "Until your next turn, each player may attack only the nearest opponent in the last chosen direction and planeswalkers controlled by that opponent" (Teyo, Geometric Tactician): the direction is
    /// the one this source chose with `ChooseCreatureType { only: ["Left", "Right"] }` just before (Left = next in turn order, Right = previous). A later choice by the same player replaces it.
    AttackOnlyNearestUntilNextTurn,
    /// "Empower Jace N": put `n` loyalty counters on a Jace token you control; if you control none, first create a blue Jace planeswalker token (token "Jace 0 Blue Planeswalker": "[-1]: Surveil 1", "[-3]: Draw a card").
    Empower { n: Amount },
    /// Balance clause: each player sacrifices (or, with `hand`, discards) down to the count of the player who has the fewest matching permanents (or cards in hand) (Magus of the Balance).
    Balance {
        #[serde(default)]
        filter: FilterBox,
        #[serde(default, skip_serializing_if = "is_false")]
        hand: bool,
    },
    /// Internal (inserted when several modes of one spell are chosen): scope this resolution's targets to the target slots `base..base+n` of the current mode, so `Targets` names only that mode's targets; `n == 255` restores them all.
    ScopeTargets { base: u8, n: u8 },
    /// Suspend: a creature cast from suspend gains haste (as long as it stays on the battlefield).
    MarkSuspendHaste,
    /// The selected creature cards gain haste until end of turn if they are cast next and enter the battlefield (Alaundo the Seer: "if you cast a creature spell this way, it gains haste until end of turn"; set just before a `CastFree`).
    MarkCastHaste { what: Sel },
    /// The selected cards (about to be cast) carry a rider: if the spell is a creature that enters, it gains haste and "At the beginning of the end step, sacrifice this creature" (Strago and Relm). Put it before the `CastFree`.
    MarkHasteSacRider { what: Sel },
    /// Mark the source (an exiled card) as foretold / plotted / warped this turn.
    MarkExiled { mark: ExileMark },
    /// The source (an exiled card) can be cast for its mana cost reduced by `reduce` (generic part only) on a later turn: the foretell cost granted by Bohn, Beguiling Balladeer. Use after `MarkExiled Foretold`.
    SetForetellCost { reduce: String },
    /// Exile each selected card from its owner's hand face down; it becomes foretold with a foretell cost equal to its mana cost reduced by `reduce` (generic part only), castable on a later turn (Ethereal Valkyrie).
    ForetellExile { what: Sel, reduce: String },
    /// The selected exiled cards become plotted: their owner may cast each one for free at sorcery speed on a later turn (Make Your Own Luck).
    PlotCards { what: Sel },
    /// Prepare: the creatures become prepared (a copy of their prepare spell appears in exile, castable while it stays prepared).
    BecomePrepared { what: Sel },
    /// The creatures stop being prepared (their spell copy ceases to exist).
    Unprepare { what: Sel },
    /// If this would leave the battlefield, exile it instead (unearth).
    MarkExileOnLeave {
        /// The permanents to mark (the ability's source when absent): "if that creature would leave the battlefield, exile it instead" for a creature returned by this effect.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        what: Option<Sel>,
    },
    /// The selected permanents are exiled instead if they would die this turn (Ob Nixilis's Cruelty, Incendiary Flow ...).
    ExileInsteadOfDying { what: Sel },
    /// Adapt N: if this creature has no +1/+1 counters (or a one-shot `AdaptIgnoreCounters` is pending for it), put N +1/+1 counters on it; the one-shot is used up either way.
    Adapt { n: Amount },
    /// The next time each selected creature adapts this turn, it adapts as though it had no +1/+1 counters (Biomancer's Familiar).
    AdaptIgnoreCounters { what: Sel },
    /// The source permanent's effects are ignored until end of turn (`Condition::IgnoredUntilEndOfTurn` is true for it; Volrath's Curse).
    IgnoreThisUntilEndOfTurn,
    /// The effect's controller makes the block declarations of the selected players this turn ("you choose how those creatures block": Brutal Hordechief).
    ChooseBlocks { who: Sel },
    /// The effect's controller makes the attack declaration of the active player this turn ("you choose which creatures attack this turn": Master Warcraft).
    ChooseAttackers,
    /// Raises the exploit event for this source (used by the Exploit keyword expansion: "when this creature exploits a creature").
    EmitExploited,
    /// This spell or ability makes players guess or group cards / permanents into piles: counted once for `Amount::PileGuessAbilitiesThisTurn` (The Celestial Toymaker).
    NotePileOrGuess,
    /// Through the next `turns` - 1 turns as well as this one (Peace Talks: 2): creatures can't attack (`attacks`) and players and permanents can't be targeted by spells or activated abilities (`targeting`; triggered abilities still can).
    LockTurns {
        #[serde(default, skip_serializing_if = "is_false")]
        attacks: bool,
        #[serde(default, skip_serializing_if = "is_false")]
        targeting: bool,
        turns: u8,
    },
    /// This turn the source planeswalker's loyalty abilities may be activated `n` more times (Comet, Stellar Pup: "you may activate Comet's loyalty ability two more times this turn").
    LoyaltyExtraThisTurn { n: i32 },
    /// This turn you may activate loyalty abilities of your planeswalkers twice rather than once (Kaito, Dancing Shadow).
    LoyaltyTwiceThisTurn {
        /// All planeswalkers you control (Urza Assembles the Titans) rather than only the source.
        #[serde(default, skip_serializing_if = "is_false")]
        all: bool,
    },
    /// Until end of turn you may activate loyalty abilities of your planeswalkers (with `subtype`, if given: "Jace planeswalkers") on any player's turn any time you could cast an instant (Jace's Machinations).
    LoyaltyInstantThisTurn {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subtype: Option<String>,
    },
    /// The keyword ability just performed its action ("whenever this creature trains / evolves / mentors a creature"): `what` is the creature that was mentored.
    /// Discover N: exile cards from the top of your library until you exile a nonland card with mana value N or less; cast it without paying its mana cost or put it into your hand;
    /// put the rest on the bottom in a random order. Fires `Trigger::Discovers` with `EventAmount` = N.
    Discover { n: Amount },
    EmitKeywordAction {
        action: KeywordAction,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        what: Option<Sel>,
    },
    /// "For each kind of counter among permanents `among` (checked as this resolves), put a counter of that kind on one of `onto`" (Exotic Pets: `onto` is `Remembered`, the new tokens).
    /// The controller picks the permanent separately for each kind (no question when there is only one).
    CounterKindsAmongOnto { among: FilterBox, onto: Sel },
    /// General Jarkeld: of the first two selected blocked attackers, if every blocker of each could block the other one, each creature blocking exactly one of them
    /// (of those two) stops blocking it and blocks the other.
    SwitchBlockers { what: Sel },
    /// Sorrow's Path: of the first two selected blocking creatures, if each could block every attacker the other blocks, both are removed from combat and then each blocks all the attackers the other was blocking.
    SwapBlocking { what: Sel },
    /// Tawnos's Coffin: exile the first selected creature and every Aura attached to it (all exiled with the source) and note the creature's counters. `CoffinReturn` brings them back.
    CoffinExile { what: Sel },
    /// Tawnos's Coffin: return the creature exiled by this source's `CoffinExile` to the battlefield under its owner's control tapped with the noted counters, then the other exiled cards attached to it (an Aura that can't enchant it stays exiled).
    CoffinReturn,
    /// Bronzehide Lion: return the selected card from the graveyard to the battlefield as a non-creature Aura enchantment (`Card.as_aura`) enchanting a permanent matching `host` (relative to the effect's controller; the controller picks among several).
    /// With no legal host it stays where it is.
    ReturnAsAura { what: Sel, host: FilterBox },
    /// Tarrian's Journal: the controller may cast one spell matching `filter` from their graveyard this turn. `finality`: it enters with a finality counter; `subtype`: it is also that creature type.
    MayCastFromGraveyardOnce { filter: FilterBox, #[serde(default, skip_serializing_if = "is_false")] finality: bool, #[serde(default, skip_serializing_if = "Option::is_none")] subtype: Option<String> },
    /// Restart the game (CR 727; Karn Liberated): every card is put back in its owner's library, new opening hands are drawn and the game begins again with the non-Aura permanent cards exiled with the source left in exile; they enter the battlefield under the controller's control as the new game begins. The controller goes first. Emblems and tokens are gone; life, counters and all other game state are reset.
    RestartGame,
    /// The source remembers the first selected card as its "last chosen card" (`Sel::ChosenCard`).
    /// `on`: the first selected permanent remembers it instead of the source (Timothar's Bat token remembers the exiled Vampire).
    SetChosenCard { what: Sel, #[serde(default, skip_serializing_if = "Option::is_none")] on: Option<Sel> },
    /// This becomes renowned.
    BecomeRenowned,
    /// This becomes harnessed (a designation; lost when it leaves the battlefield; The Soul Stone).
    Harness,
    /// Start your engines!: if the players have no speed, their speed becomes 1.
    StartEngines { who: Sel },
    /// The selected players are restricted for a while ("players can't cast spells this turn", "you can't gain life this turn").
    SwitchPT { what: Sel, duration: Duration },
    /// Choose a card name (stored on the source; `Filter.chosen_name` matches cards with that name). `who` chooses (default: the controller).
    /// The choices offered are the names of the cards in this game that fit `kind`.
    ChooseCardName {
        #[serde(default)]
        kind: NameKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        who: Option<Sel>,
        /// The name is stored as a second chosen name (`Card.chosen_name2`) and `Filter.chosen_name` matches either one (Null Chamber: you and an opponent each choose a name). A choice without `second` clears the second name.
        #[serde(default, skip_serializing_if = "is_false")]
        second: bool,
        /// Only the names of nonland cards in the hands of the chooser's opponents can be chosen ("choose the name of a nonland card revealed this way": Alhammarret, High Arbiter).
        #[serde(default, skip_serializing_if = "is_false")]
        from_opponent_hands: bool,
    },
    /// "Choose a card name from among <names>. Create a copy of the card with the chosen name. You may cast the copy" (Garth One-Eye): the controller picks one of `names`
    /// (with `once_each`, not one this source chose before: `Card.names_used`), a card with that name is created outside the game, and the controller may cast it paying its costs.
    CopyNamedCard {
        names: Vec<String>,
        #[serde(default, skip_serializing_if = "is_false")]
        once_each: bool,
    },
    /// Each player (the controller first, in turn order) secretly chooses a card name; then each player reveals the top card of their library and puts it into
    /// their hand if it has the name they chose, otherwise on the bottom (Conundrum Sphinx).
    NameAndRevealTop {
        #[serde(default)]
        kind: NameKind,
    },
    /// Search the selected players' zones for cards matching the filter (e.g. `chosen_name`) and exile them (up to `max`); they become `Remembered`.
    /// The library is shuffled when searched.
    ExileFromZones {
        who: Sel,
        zones: Vec<Zone>,
        filter: FilterBox,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<Amount>,
    },
    /// The selected creatures become suspected (menace, can't block) / stop being suspected.
    /// The controller sacrifices any number of permanents they control matching the filter (they choose); the sacrificed ones become `Remembered`
    /// (`Amount::RememberedCount` counts them). Devour, "sacrifice any number of ...".
    /// `by`: that player sacrifices and chooses instead of the controller (Jund's devour for the creature's controller).
    SacrificeAny {
        filter: FilterBox,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        by: Option<Box<Sel>>,
    },
    Suspect { what: Sel },
    Unsuspect { what: Sel },
    /// The selected creatures are removed from combat.
    /// `unblock_sole`: attackers whose only blockers were the removed creatures become unblocked (Balduvian Warlord, False Orders).
    RemoveFromCombat { what: Sel, #[serde(default, skip_serializing_if = "std::ops::Not::not")] unblock_sole: bool },
    Restrict { who: Sel, what: PlayerRestrict, duration: Duration },
    /// The Ring tempts you: the Ring gets its next level (max 4: each level adds its ability through the emblem "The Ring N"), then the controller
    /// chooses a creature they control as their Ring-bearer. The Ring-bearer is legendary and can't be blocked by creatures with greater power.
    RingTempts,
    /// Venture into the dungeon: enter the first room of a dungeon (the named one, or the player's choice of Lost Mine of Phandelver / Tomb of Annihilation /
    /// Dungeon of the Mad Mage) or, if already in a dungeon, advance to a next room (the dungeon is completed after its last room).
    Venture { dungeon: Option<String> },
    /// Voting: starting with the controller, each player votes for one of the named options. The tallies are readable afterwards through
    /// `Amount::Votes { option }` and `Condition::MostVotes` (council's dilemma: `Repeat { n: Votes }`; will of the council: `MostVotes`).
    /// `opponents_only`: only the controller's opponents vote (Prisoner's Dilemma). `secret`: the ballots are hidden from the other players (Secret council).
    Vote { options: Vec<String>, #[serde(default, skip_serializing_if = "is_false")] opponents_only: bool, #[serde(default, skip_serializing_if = "is_false")] secret: bool },
    /// Voting for objects: each player votes for one of the selected cards. Afterwards `Remembered` is every card with the most votes or tied for most.
    /// `secret`: hidden ballots. `optional`: a voter may vote for no creature ("up to one"). `all_voted`: afterwards `Remembered` is every card with at least one vote (not only the leaders).
    /// The vote count of each card is `Amount::CardVotes`.
    VoteForCards { from: Sel, #[serde(default, skip_serializing_if = "is_false")] secret: bool, #[serde(default, skip_serializing_if = "is_false")] optional: bool, #[serde(default, skip_serializing_if = "is_false")] all_voted: bool },
    /// Each player votes for a player (`others`: not for themselves). The votes each player received are `Amount::VotesReceived`; `Sel::VotersFor` has the player id as its `option`.
    VoteForPlayers { #[serde(default, skip_serializing_if = "is_false")] others: bool, #[serde(default, skip_serializing_if = "is_false")] secret: bool },
    /// Clash with an opponent of the controller's choice (recorded as the source's chosen player, `Sel::ChosenPlayer`): each player reveals the top card of their library, may put it on the bottom; the player with the
    /// higher mana value wins. `win` runs if the controller won, `lose` otherwise (a tie is a loss for both).
    Clash {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        win: Vec<Effect>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        lose: Vec<Effect>,
    },
    /// The players' speed increases by 1 (max 4).
    IncreaseSpeed { who: Sel },
    /// The players' speed decreases by 1, not below 1 (Spikeshell Harrier). `only_if_highest`: only a player whose speed is greater than every other player's.
    DecreaseSpeed { who: Sel, #[serde(default, skip_serializing_if = "is_false")] only_if_highest: bool },
    /// Monstrosity N: if this isn't monstrous, put N +1/+1 counters on it and it becomes monstrous.
    Monstrosity { n: Amount },
    /// Flip a coin: `win` runs when the controller wins the flip, `lose` otherwise.
    FlipCoin {
        /// The player who flips (default: the controller).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        who: Option<Sel>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        win: Vec<Effect>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        lose: Vec<Effect>,
    },
    /// Bolster N: put N +1/+1 counters on the creature you control with the least toughness (the first of any tie).
    /// `remember`: the creature that got the counters becomes `Remembered` ("the chosen creature gains trample", Optimus Prime, Autobot Leader).
    Bolster {
        n: Amount,
        #[serde(default, skip_serializing_if = "is_false")]
        remember: bool,
    },
    /// Roll a die with `sides` sides: the first outcome whose `min..=max` contains the result runs its effects; `Amount::DieRoll` reads the result.
    RollDie {
        sides: i32,
        /// Roll one die for each player being attacked (a player, or a player whose planeswalker or battle is attacked) and keep only the highest result; `DieRolled` fires for each die (Iron Mastiff).
        #[serde(default, skip_serializing_if = "is_false")]
        per_attacked_player: bool,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        outcomes: Vec<DieOutcome>,
        /// Additional dice rolled at the same time ("plus an additional d6 for each mana from Treasures spent": Mr. House); each fires `DieRolled`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        extra: Option<Box<Amount>>,
    },
    /// Roll two dice with `sides` sides and choose one result (Grave Endeavor): `Amount::RollChosen` is the chosen result, `Amount::RollOther` the other one.
    /// Each die fires `DieRolled`; the controller is asked only when the two results differ.
    RollTwoChooseOne { sides: i32 },
    /// "For each player, choose friend or foe" (Zndrsplt's Judgment): the controller picks for every living player in turn order starting with themselves; the picks are kept on the source
    /// (`Card.player_pick`, true = foe) and read with `Sel::PlayersPicked`.
    ChooseFriendOrFoe,
    /// A reflexive trigger ("When you do, ..."): the controller chooses targets now and the effects go on the stack as a new ability.
    /// `untargeted`: the "targets" are plain choices (Season of Weaving's "choose a creature"), so the effects still run when the choice has left.
    Reflexive { targets: Vec<TargetSpec>, effects: Vec<Effect>, #[serde(default, skip_serializing_if = "is_false")] untargeted: bool },
    /// A modal triggered / activated ability: the controller chooses `choose_min..=choose_max` modes as it resolves, then the chosen modes'
    /// effects run in printed order. Modes with targets aren't supported here (use Choose inside the mode).
    ChooseModes {
        choose_min: u8,
        choose_max: u8,
        modes: Vec<Mode>,
        /// Each mode can be chosen only once per turn by this permanent ("choose one that hasn't been chosen this turn": Gala Greeters, Lita, Genku).
        #[serde(default, skip_serializing_if = "is_false")]
        once_each_turn: bool,
        /// Each mode can be chosen only once per permanent for the rest of the game ("choose one that hasn't been chosen": Henrika Domnathi, Gandalf the Grey, Kimoyo Beads).
        #[serde(default, skip_serializing_if = "is_false")]
        once_ever: bool,
        /// "Choose up to N {P} worth of modes; you may choose the same mode more than once": modes are picked one at a time, each costing its `paw`, until the budget is used up or the player stops. `choose_min`/`choose_max` are ignored.
        #[serde(default, skip_serializing_if = "is_zero_u8")]
        budget: u8,
        /// The player who chooses the mode(s) instead of the controller ("target opponent chooses one": pile choice, "an opponent chooses one of them").
        #[serde(default, skip_serializing_if = "Option::is_none")]
        chooser: Option<Sel>,
        /// "Choose up to X modes, where X is ...": when set, the maximum is this amount (evaluated as the modes are chosen) instead of `choose_max`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_amount: Option<Amount>,
        /// "Choose one at random": the engine picks a single mode from the game state (no die roll, so no RollsDie triggers); nobody decides.
        #[serde(default, skip_serializing_if = "is_false")]
        random: bool,
        /// The modes (and the targets of the chosen modes) are chosen as the ability is put on the stack, like a modal spell (CR 700.2): the only effect of its ability, each mode being plain effects or one `Reflexive` whose targets become the mode's targets.
        #[serde(default, skip_serializing_if = "is_false")]
        at_stack: bool,
        /// With `at_stack`: "you may choose the same mode more than once" (I Will Savor Your Agony); each repeat gets its own targets.
        #[serde(default, skip_serializing_if = "is_false")]
        allow_repeat: bool,
        /// This is a villainous choice ("faces a villainous choice"): an opponent of a player controlling `StaticEffect::VillainousChoiceTwice` makes it an additional time.
        #[serde(default, skip_serializing_if = "is_false")]
        villainous: bool,
    },
    /// "You may play / cast the selected exiled cards": the controller gets permission for the window.
    /// `any_mana`: mana of any type can be spent to cast the card (its colored / colorless costs are paid with any mana).
    /// `free`: the card may be cast without paying its mana cost.
    /// The owner of each selected Adventure card in a graveyard may cast it as an Adventure (its Adventure half only) from there until the end of their next turn (Hildibrand Manderville).
    AdventureFromGraveyard { what: Sel },
    /// "Until end of turn, you may cast target instant or sorcery card from your graveyard without paying its mana cost. If that spell would be put into your graveyard, exile it instead" (Sins of the Past).
    MayCastFreeFromGraveyardThisTurn { what: Sel },
    GrantPlayFromExile {
        what: Sel,
        window: PlayWindow,
        #[serde(default, skip_serializing_if = "is_false")]
        any_mana: bool,
        #[serde(default, skip_serializing_if = "is_false")]
        free: bool,
        /// Instead of its mana cost the card costs life equal to its mana value ("you may cast it, paying life equal to its mana value rather than paying its mana cost").
        #[serde(default, skip_serializing_if = "is_false")]
        pay_life_mv: bool,
        /// The player who gets the permission (default: the effect's controller): "target opponent may cast it".
        #[serde(default, skip_serializing_if = "Option::is_none")]
        by: Option<Sel>,
        /// The permission only applies while this holds (checked each time the card is played, with the grantee as controller): "During any turn you attacked with three or more creatures, you may play that card".
        #[serde(default, skip_serializing_if = "Option::is_none")]
        play_cond: Option<Condition>,
        /// The spell cast this way has affinity for permanents matching this filter ("it has affinity for artifacts": Chiss-Goria).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        affinity: Option<FilterBox>,
        /// "If a spell cast this way would be put into your graveyard, exile it instead."
        #[serde(default, skip_serializing_if = "is_false")]
        exile_after: bool,
        /// "You may play ONE of those cards": once one of the selected cards is cast or played, the others lose the permission (Nashi, Moon Sage's Scion).
        #[serde(default, skip_serializing_if = "is_false")]
        one: bool,
        /// "You may play up to N of those cards" (March of Reckless Joy: 2): once N of the selected cards have been cast or played, the others lose the permission.
        #[serde(default, skip_serializing_if = "is_zero_u8")]
        limit: u8,
        /// "For each nonland card type, you may cast one spell of that type from among those cards" (Aminatou's Augury): a spell cast uses up one of its card types for the whole group; lands get no permission. A multi-type spell uses the type the fewest other granted cards have.
        #[serde(default, skip_serializing_if = "is_false")]
        per_type: bool,
        /// A spell cast through this permission costs this much generic mana more (Lightstall Inquisitor: {1} more).
        #[serde(default, skip_serializing_if = "is_zero_u8")]
        extra_generic: u8,
        /// A land played through this permission enters tapped (Lightstall Inquisitor).
        #[serde(default, skip_serializing_if = "is_false")]
        land_tapped: bool,
        /// "You may cast that card" (not "play"): lands get no permission (Grenzo, Havoc Raiser).
        #[serde(default, skip_serializing_if = "is_false")]
        cast_only: bool,
        /// Only cards matching this filter may be cast without paying their mana cost ("you may cast Equipment spells this way without paying their mana costs": Nahiri, Forged in Fury).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        free_if: Option<Box<Filter>>,
    },
    /// "Until end of turn, you may cast a spell from among the selected exiled cards without paying its mana cost": ONE nonland card of them may be cast for free this turn
    /// (casting one ends the offer for the others). The cards keep any other play permission they have (Court of Locthwain).
    FreeCastOneThisTurn { what: Sel },
    /// "Each opponent can't block with more than one creature this combat": the selected players can't declare more than `n` different blocking creatures for the rest of this combat (Mirri, Weatherlight Duelist).
    LimitBlockers { who: Sel, n: u8 },
    /// "This turn, instead of declaring blockers, each defending player chooses any number of creatures they control and divides them into a number of piles equal to the number of
    /// creatures attacking them ... Assign each pile to a different one of those attacking creatures at random. Each creature in a pile that can block the creature that pile is assigned to does so" (Camouflage).
    PileBlocks,
    /// Additional upkeep steps for the active turn (Ninth Doctor, Paradox Haze); used during combat they come after the combat phase (Obeka).
    ExtraUpkeep { n: Amount },
    /// An additional beginning phase (untap, upkeep, draw) after the post-combat main phase; the end step follows it (Sphinx of the Second Sun).
    ExtraBeginningPhase,
    /// An additional beginning phase (untap, upkeep, draw) right after the current combat phase, before the post-combat main phase (Cyclonus, Cybertronian Fighter).
    /// There is an additional end step after the current one (Y'shtola Rhul).
    ExtraEndStep,
    ExtraBeginningPhaseAfterCombat,
    /// An additional combat phase after this one (after the post-combat main phase if resolved then). `untap`: untap all creatures you control now.
    ExtraCombat {
        #[serde(default, skip_serializing_if = "is_false")]
        untap: bool,
        /// "...followed by an additional main phase": a second post-combat main phase after the extra combat (modelled as Combat, Combat, Main, Main).
        #[serde(default, skip_serializing_if = "is_false")]
        then_main: bool,
    },
    /// `what` becomes a copy of `of` (the first selected card), for `duration` (Forever or UntilEndOfTurn), plus exceptions.
    BecomeCopy {
        what: Sel,
        of: Sel,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        except: Vec<CopyExcept>,
        duration: Duration,
    },
    /// Cast the (first) selected card without paying its mana cost, as the resolving effect finishes (cascade). `may`: the controller is asked first.
    CastFree {
        what: Sel,
        #[serde(default, skip_serializing_if = "is_false")]
        may: bool,
        /// If the controller declines, the card goes to the bottom of its owner's library (cascade).
        #[serde(default, skip_serializing_if = "is_false")]
        decline_to_bottom: bool,
        /// If the controller declines, the card goes to hand instead (discover).
        #[serde(default, skip_serializing_if = "is_false")]
        decline_to_hand: bool,
        /// "If that spell would be put into a graveyard, exile it instead" (it is exiled when it resolves or is countered).
        #[serde(default, skip_serializing_if = "is_false")]
        exile_after: bool,
        /// Cast a copy of the card instead of the card itself (the original stays where it is): "copy that card and you may cast the copy".
        #[serde(default, skip_serializing_if = "is_false")]
        copy: bool,
        /// The card is cast paying its mana cost (and other costs) instead of for free ("you may cast it, paying its mana cost").
        #[serde(default, skip_serializing_if = "is_false")]
        pay: bool,
        /// The player who casts the card (default: the effect's controller): "its owner may cast it", "target opponent may cast it without paying".
        #[serde(default, skip_serializing_if = "Option::is_none")]
        by: Option<Sel>,
        /// "Mana of any type can be spent to cast that spell" (Quistis Trepe, Tinybones the Pickpocket).
        #[serde(default, skip_serializing_if = "is_false")]
        any_mana: bool,
        /// A selected land card is played instead (uses a land drop; nothing happens when none is left): "you may play one of those cards without paying its mana cost" (Vault 112: Sadistic Simulation).
        #[serde(default, skip_serializing_if = "is_false")]
        lands: bool,
        /// Cast using the card's n-th `AlternativeCost` ability (with `pay: true`: madness / miracle with an {X} cost; the spell reads `Condition::CastWithAlt`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        alt: Option<u8>,
        /// Mana added to the cost of this cast ("by paying {R}{R} in addition to its other costs": Ogre Battlecaster); use with `pay: true`.
        /// A leading `-` ("-{2}") instead reduces the generic part of the cost ("that copy costs {2} less to cast": God-Eternal Kefnet).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        extra_mana: Option<String>,
        /// "If you don't": done when the card is not cast because the controller declines, or it is a land that can't be played, or (with `pay`) its cost can't be afforded (Chandra, Torch of Defiance).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        otherwise: Vec<Effect>,
        /// Cast the card paying its suspend cost rather than its mana cost (use with `pay: true`; The Face of Boe). The card needs a `Suspend` ability.
        #[serde(default, skip_serializing_if = "is_false")]
        suspend: bool,
        /// With `may`: when the controller accepts, they can't cast additional spells this turn beyond this one (Conduit of Worlds). A cast that then fails to start leaves the one-more-spell allowance.
        #[serde(default, skip_serializing_if = "is_false")]
        lock_spells_after: bool,
        /// Run when the controller accepts the (`may`) cast, right before the spell is cast ("if you cast a spell this way, put a +1/+1 counter on it": Helmut Zemo).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        on_cast: Vec<Effect>,
        /// Run (as a trigger above the spell) right after the spell has actually been cast through this effect ("if you do, convert Soundwave": cast triggers see the cast before the effects change anything).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        after_cast: Vec<Effect>,
        /// "If that spell would be put into a graveyard, put it on the bottom of its owner's library instead" (Kylox's Voltstrider).
        #[serde(default, skip_serializing_if = "is_false")]
        bottom_after: bool,
        /// Copy the spell this many times as soon as it is cast (a copy trigger above it on the stack; each copy may get new targets): Finale of Promise.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        copies: Option<Amount>,
        /// A modal double-faced card with a castable (non-land) back face: the caster is asked which face to cast (Jadzi, Oracle of Arcavios). With `pay`, an `extra_mana` starting with `=` ("={1}") is the whole cost paid instead of the mana cost.
        #[serde(default, skip_serializing_if = "is_false")]
        face_choice: bool,
    },
    /// "You may cast any number of spells from among" the selected cards without paying their mana costs: the controller answers yes/no for each nonland card, then every accepted card is cast in turn, all before any of the spells resolves (Etali, Primal Storm).
    CastFreeAny {
        what: Sel,
        /// Each accepted card is copied and the copy is cast instead (the card stays where it is): Wizard's Spellbook's "copy each card exiled with this artifact".
        #[serde(default, skip_serializing_if = "is_false")]
        copy: bool,
        /// The accepted cards are cast paying their mana costs ("you may cast any number of the copies": The Tale of Tamiyo).
        #[serde(default, skip_serializing_if = "is_false")]
        pay: bool,
    },
    /// Internal (from `ManaRider::ExileSpell`): the source spell is exiled instead of going to the graveyard as it resolves, linked to the permanent whose mana was spent on it.
    ExileSpellOnResolve,
    Populate,
    /// Populate, and the new token enters tapped and attacking (Ghired, Conclave Exile).
    PopulateAttacking,
    /// Each selected creature that isn't in a sector is assigned to one by its controller; the source's opponents choose first (the space sculptor rule: Space Beleren).
    AssignSector { what: Sel },
    /// Until end of turn, creatures can be blocked only by creatures in the same sector (Space Beleren +1).
    SectorBlocking,
    /// Save `Remembered` into a numbered slot (0-3) for `Sel::Stored` / `Filter.among_stored`; with `append` add to what the slot holds.
    StoreRemembered {
        slot: u8,
        #[serde(default, skip_serializing_if = "is_false")]
        append: bool,
    },
    /// `Remembered` = the selected objects, without moving them ("remember the triggering attacker" before a later StoreRemembered / Filter.blocking_stored).
    Remember { what: Sel },
    /// Reveal (remember) the top `n` cards of a library WITHOUT moving them: `Remembered` = those cards ("reveal the top five cards of your library"). Reordering / moving them is done by later effects.
    RevealTop { who: Sel, n: Amount },
    /// The selected attacking creatures now attack the first selected player / planeswalker / battle instead (a token "attacking that player": CreateToken attacking:true, then this).
    ChangeDefender { what: Sel, to: Sel },
    /// The effect's controller reselects what the first selected attacking creature attacks: any player other than that creature's controller, or a planeswalker / battle they control or protect (the current defender stays an option; Misleading Signpost). Auto-picks when only one option.
    ReselectDefender { what: Sel },
    /// Empty `Remembered` (a later Choose that picks nothing would otherwise leave an earlier set in place).
    ClearRemembered,
    /// Marker (does nothing): the ability this effect is in can't be copied (Gogo, Master of Mimicry: "This ability can't be copied").
    AbilityCantBeCopied,
    /// Add the selected cards to the end of `Remembered` (Flickerform: the enchanted creature, then the Auras attached to it).
    AppendRemembered { what: Sel },
    /// The creature / permanent spell selected (`TriggerObject` in a cast trigger) enters with `n` additional +1/+1 counters (Wildgrowth Archaic, Runadi).
    SpellEntersWithCounters {
        what: Sel,
        n: Amount,
        /// Sunburst: a spell that isn't a creature enters with charge counters instead of +1/+1 counters (Solar Array).
        #[serde(default, skip_serializing_if = "is_false")]
        sunburst: bool,
        /// The counters are loyalty counters instead (Dark Intimations: "that planeswalker enters with an additional loyalty counter").
        #[serde(default, skip_serializing_if = "is_false")]
        loyalty: bool,
    },
    Attach { what: Sel, to: Sel },
    /// Attach each selected Aura / Equipment to the first selected permanent or player only where that is legal (an Aura's enchant restriction, an Equipment only to a creature, a player only for "Enchant player" Auras); the others stay put (Ardenn, Intrepid Archaeologist).
    AttachLegal { what: Sel, to: Sel },
    /// Move the selected Auras onto the first selected player (Archnemesis, Maddening Hex: "attach this Aura to that player").
    AttachToPlayer { what: Sel, to: Sel },
    Unattach { what: Sel },
    SkipNextTurn { who: Sel },
    /// The players skip their next combat phase (`this_turn`: only a combat phase still to come this turn, by the active player).
    SkipNextCombat { who: Sel, #[serde(default, skip_serializing_if = "is_false")] this_turn: bool },
    /// The selected player skips every instance of the draw step, of the main phases, or of the combat phases for the rest of this turn (Fatespinner).
    SkipPhaseThisTurn { who: Sel, phase: SkipPhase },
    /// The players skip their next draw step (Fatigue; "if you searched your library this way, skip your draw step this turn" during upkeep).
    SkipNextDrawStep { who: Sel },
    /// The selected players skip their next untap step (Shisato, Whispering Hunter): nothing untaps and phasing doesn't happen.
    SkipNextUntapStep { who: Sel },
    /// "Lands you control don't untap during your next untap step."
    LandsDontUntapNext,
    /// Creatures each selected player controls don't untap during that player's next untap step (Imaginary Threats); cleared when their upkeep begins.
    CreaturesDontUntapNext { who: Sel },
    /// "Until your next turn, prevent all damage that would be dealt to you."
    PreventDamageToYouUntilNextTurn,
    /// "You may change the targets of target spell" / "choose new targets for target spell": the effect's controller may pick a new legal target for each target slot of a spell on the stack (a slot with no other legal choice is skipped).
    /// `blocker` blocks `attacker` this turn if able (Hunt Down, Feral Contest).
    MustBlockCreature { blocker: Sel, attacker: Sel, duration: Duration },
    /// You gain control of a spell on the stack (Aethersnatch, Invert Polarity): it resolves under your control, so a permanent spell enters under it.
    GainControlOfSpell { what: Sel },
    /// Exchange control of a spell on the stack and a permanent (Sudden Substitution): the spell's controller becomes the permanent's controller, and the permanent goes to the spell's old controller for good.
    ExchangeSpellControl { spell: Sel, permanent: Sel },
    /// Exchange this card (in its owner's hand) with the first selected permanent (battlefield) or spell (stack) you control (Red Herring): this card takes its place and the other goes to the hand.
    /// Spells and abilities that targeted the exchanged permanent now target this card.
    ExchangeFromHand { with: Sel },
    /// Change the target of a spell (`must_change`: the target must be changed to another legal one when there is one -- Swerve, Ricochet Trap).
    ChangeTargets {
        what: Sel,
        #[serde(default, skip_serializing_if = "is_false")]
        must_change: bool,
        /// The only new target on offer is this object (Mizzium Meddler: "change a target to this creature"), when it is a legal target.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to: Option<Sel>,
        /// The new target must be a player (Reflecting Mirror: "the new target must be a player").
        #[serde(default, skip_serializing_if = "is_false")]
        players_only: bool,
        /// The new target must be a permanent matching this filter (Meddle: "another creature"); players are not offered.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        new_filter: Option<FilterBox>,
        /// Offer a new target for every other spell and ability on the stack that has targets (Boltbender: "any number of other spells and/or abilities"); `what` is ignored. The effect's own ability is skipped; modal spells are left alone.
        #[serde(default, skip_serializing_if = "is_false")]
        each_stack: bool,
        /// With `to`: the target is changed to that object without asking, when it is a legal target ("change the target to enchanted creature if able": Captured by the Consulate).
        #[serde(default, skip_serializing_if = "is_false")]
        forced: bool,
        /// Reselect each target at random from among all legal targets (the current one included); nobody chooses (Grip of Chaos).
        #[serde(default, skip_serializing_if = "is_false")]
        random: bool,
        /// The player who chooses the new targets (default: the effect's controller). When the selection is empty nobody changes anything (Psychic Battle: `OwnerOfGreatestMv`, empty on a tie).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        by: Option<Sel>,
        /// The new target can't be the effect's controller or a permanent they control; a slot with no such legal target keeps its current one (Chef's Kiss).
        #[serde(default, skip_serializing_if = "is_false")]
        exclude_yours: bool,
    },
    /// "If it's neither day nor night, it becomes day."
    BecomeDay,
    /// "It becomes night." (even when it was day)
    BecomeNight,
    /// "It becomes day." (even when it was night; The Celestus)
    SetDay,
    /// The creature becomes tapped and attacking (the same defender as the first attacker; Sneak, "put onto the battlefield tapped and attacking").
    /// With `defender_among`, the effect's controller picks the defender (a player, planeswalker or battle) among those attacked by creatures the selected players control
    /// ("choose a player or planeswalker that opponent is attacking"; Tahngarth, First Mate).
    BecomeAttacking { what: Sel, #[serde(default, skip_serializing_if = "Option::is_none")] defender_among: Option<Sel> },
    /// The creatures put onto the battlefield "blocking" the first selected attacker (Flash Foliage, Brimaz): they join its blockers without a Blocked event.
    EnterBlocking { what: Sel, attacker: Sel },
    /// The selected attacking creatures become blocked (no creature blocks them; Curtain of Light, Choking Vines): they deal no combat damage to the defender unless they have trample.
    BecomeBlocked { what: Sel },
    /// "Damage can't be prevented this turn."
    DamageCantBePrevented,
    /// `lose_at_end`: the player loses the game at the beginning of that extra turn's end step (Final Fortune, Last Chance).
    ExtraTurn { who: Sel, #[serde(default, skip_serializing_if = "is_false")] lose_at_end: bool },
    /// Each player exiles the top card of their library; the player whose card has the greatest mana value takes an extra turn after this one. If two or more are tied for greatest, only the tied players repeat the process until the tie is broken (Timesifter). Players without a card take no part.
    ExileCompareExtraTurn,
    BecomeMonarch { who: Sel },
    TakeInitiative { who: Sel },
    GainEnergy { who: Sel, n: Amount },
    AddPoison { who: Sel, n: Amount },
    Reveal { what: Sel },
    LoseGame { who: Sel },
    WinGame { who: Sel },
    /// The game ends in a draw (Divine Intervention, Celestial Convergence); nothing that stops a player from losing prevents it.
    DrawGame,
    DrawThenDiscard { who: Sel, draw: Amount, discard: Amount },
    Dig { who: Sel, n: Amount },
    /// Exile the top `n` cards of the selected players' libraries ("exiles the top half of their library, rounded up": Ulamog, the Defiler). The exiled cards are remembered.
    ExileTopOfLibrary { who: Sel, n: Amount, #[serde(default)] face_down: bool },
    /// Look at the top n cards of a library and put them back in any order (Ponder, Sensei's Top).
    /// `decider`: the player who looks at and orders the cards when it isn't the effect's controller (Tahngarth's Glare: the library's owner orders it).
    RearrangeTop { who: Sel, n: Amount, #[serde(default, skip_serializing_if = "Option::is_none")] decider: Option<Box<Sel>> },
    /// Reveal cards from the top of a library until `count` cards matching the filter are revealed: those go to `to`, the rest
    /// to `rest` (a library: the bottom, in random order if `rest_random`). The found cards become `Remembered`.
    RevealUntil {
        who: Sel,
        filter: FilterBox,
        #[serde(default = "one_amount")]
        count: Amount,
        to: Zone,
        #[serde(default, skip_serializing_if = "is_false")]
        tapped: bool,
        rest: Zone,
        #[serde(default, skip_serializing_if = "is_false")]
        rest_random: bool,
        /// The controller chooses the order the rest go to the bottom of the library in (one Pick of all of them, topmost first; single-player reveals only).
        #[serde(default, skip_serializing_if = "is_false")]
        rest_ordered: bool,
        /// The found card is not moved to `to`: it goes to the bottom of the library together with the rest (random order with `rest_random`), but stays `Remembered`
        /// ("put the revealed cards on the bottom in a random order. When you reveal a nonland card this way, ...": Calibrated Blast, Amplifire).
        #[serde(default, skip_serializing_if = "is_false")]
        found_to_bottom: bool,
        /// Cascade: if the controller controls a `StaticEffect::CascadeLandToBattlefield`, they may put a land card from among the revealed cards onto the battlefield tapped before deciding on the found card (Averna, the Chaos Bloom).
        #[serde(default, skip_serializing_if = "is_false")]
        cascade_land: bool,
        /// The found cards enter the battlefield under the effect controller's control instead of the library owner's ("put it onto the battlefield under your control": Dance, Pathetic Marionette).
        #[serde(default, skip_serializing_if = "is_false")]
        under_your_control: bool,
    },
    /// Prevent damage until end of turn: all of it (`amount` None) or the next `amount` points; only damage dealt to `to` and/or by `from`
    /// (anything when absent); `combat_only`: only combat damage.
    PreventDamage {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to: Option<Sel>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<Sel>,
        #[serde(default, skip_serializing_if = "is_false")]
        combat_only: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        amount: Option<Amount>,
        /// The prevented damage is dealt to this object instead (a redirect shield: Shimian Night Stalker, Harsh Judgment-style).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        redirect_to: Option<Sel>,
        /// Only the next damage event matching `to`/`from` is prevented ("the next time a source of your choice would deal damage...").
        #[serde(default, skip_serializing_if = "is_false")]
        once: bool,
        /// For each point of damage this shield prevents, put a +1/+1 counter on the object it protects (Temper).
        #[serde(default, skip_serializing_if = "is_false")]
        counters: bool,
        /// For each point prevented from damage dealt to a creature, put a counter of this kind on it at the beginning of the next end step (Scars of the Veteran: +0/+1).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        end_step_counters: Option<CounterKind>,
        /// The shield also covers damage to its controller ("to you and/or permanents you control", "this creature and/or you": Harm's Way, Glarecaster).
        #[serde(default, skip_serializing_if = "is_false")]
        or_you: bool,
        /// With `redirect_to`: the redirected damage is dealt by the source of this effect (non-combat) rather than by the original source ("Refraction Trap deals that much damage to any target").
        #[serde(default, skip_serializing_if = "is_false")]
        redirect_by_source: bool,
    },
    /// The player may play `n` more lands this turn.
    ExtraLandDrop { who: Sel, n: i32 },
    /// "Your maximum hand size is reduced by `n` for the rest of the game" (cumulative; Inspired Idea).
    ReduceMaxHand { who: Sel, n: i32 },
    /// A player chooses between `min` and `max` cards in a zone that match the filter, then `then` runs with `Remembered` bound to the choice.
    /// The filter's controller is relative to the chooser ("a land you control" = `controller: You`). Nothing eligible: nothing happens.
    Choose {
        who: Sel,
        /// Whose zone the cards are in, when not the chooser's own (Duress: the target opponent's hand).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<Sel>,
        zone: Zone,
        /// More zones the choice is made among ("a card from their hand or graveyard": `zone: Hand, also_zones: [Graveyard]`).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        also_zones: Vec<Zone>,
        #[serde(default)]
        filter: FilterBox,
        #[serde(default)]
        min: u8,
        #[serde(default = "one_u8")]
        max: u8,
        /// Nobody is asked: `max` of the candidates are picked at random ("a card at random from their hand", "two of them at random").
        #[serde(default, skip_serializing_if = "is_false")]
        random: bool,
        /// A computed minimum / maximum instead of the fixed `min` / `max` ("reveals X cards", "choose up to X nonland cards").
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min_amount: Option<Amount>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_amount: Option<Amount>,
        /// The chosen cards must all differ in this respect ("any number of cards with different names / different powers"); a later duplicate pick is dropped.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        distinct: Option<ChooseDistinct>,
        /// The chosen cards are ADDED to `Remembered` instead of replacing it (several Choose effects building one set: "one land of each basic land type").
        #[serde(default, skip_serializing_if = "is_false")]
        accumulate: bool,
        /// Only the topmost N cards of a graveyard zone are offered ("one of the top two cards of your graveyard": Phyrexian Grimoire).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        top: Option<u8>,
        /// The candidates are exactly these cards (filtered) instead of a zone: "one of those targets" (Aerith Rescue Mission: `Targets`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        among: Option<Sel>,
        then: Vec<Effect>,
    },
    /// Choose any number of cards whose TOTAL mana value / power / toughness is at most `budget` ("any number of creature cards with total mana value 6 or less").
    /// The chooser picks one card at a time among those that still fit; stop with an empty pick. `Remembered` = the chosen cards.
    ChooseWithin {
        who: Sel,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<Sel>,
        zone: Zone,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        also_zones: Vec<Zone>,
        #[serde(default)]
        filter: FilterBox,
        kind: BudgetKind,
        budget: Amount,
        then: Vec<Effect>,
    },
    // ---- control flow
    If {
        cond: Condition,
        then: Vec<Effect>,
        #[serde(default, skip_serializing_if = "Vec::is_empty", rename = "else")]
        otherwise: Vec<Effect>,
    },
    /// A replacement/prevention effect for the rest of this turn ("this turn each creature you control enters with an additional +1/+1 counter").
    AddReplacement {
        what: Replace,
        /// Lasts until the controller's next turn begins instead of until end of turn (Arlinn, the Pack's Hope).
        #[serde(default, skip_serializing_if = "is_false")]
        until_your_next_turn: bool,
    },
    /// The selected player(s) face a villainous choice made by `effects` (a `May` / `Unless` that is their choice): done an additional time when an opponent of the
    /// player facing it controls `StaticEffect::VillainousChoiceTwice` (The Valeyard).
    FacesVillainousChoice { who: Sel, effects: Vec<Effect> },
    /// The controller may do these effects.
    May {
        effects: Vec<Effect>,
        /// Done when the player declines.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        otherwise: Vec<Effect>,
        /// The player who decides instead of the controller ("its controller may copy this spell").
        #[serde(default, skip_serializing_if = "Option::is_none")]
        who: Option<Sel>,
        /// Record the answer for `Condition::LastCostPaid` (true when the player accepted): "you may repeat this process" (Kindle the Carnage).
        #[serde(default, skip_serializing_if = "is_false")]
        remember_answer: bool,
    },
    /// "You may pay <cost>. If you do, <effects>": `payer` is asked, and if they pay, the effects happen.
    MayPay { payer: Sel, cost: Vec<Cost>, effects: Vec<Effect> },
    /// "unless <player> pays <cost>": otherwise do the effects.
    /// `any_player`: every selected player in turn may pay ("unless any player pays"); the first to pay stops the effects.
    Unless { payer: Sel, #[serde(default, skip_serializing_if = "std::ops::Not::not")] any_player: bool, cost: Vec<Cost>, effects: Vec<Effect>, #[serde(default, skip_serializing_if = "Vec::is_empty")] if_paid: Vec<Effect> },
    /// Cumulative upkeep: put an age counter on this permanent, then its controller pays `cost` once per age counter (mana / life / discard /
    /// sacrifice amounts are multiplied) or sacrifices it. Put it in an upkeep trigger.
    /// `if_paid` runs once the upkeep was paid (Balduvian Fallen reads `Amount::LastPaymentManaSpent`).
    /// `if_unpaid` runs after the sacrifice when the upkeep was not paid (Thought Lash: exile your library).
    CumulativeUpkeep { cost: Vec<Cost>, #[serde(default, skip_serializing_if = "Vec::is_empty")] if_paid: Vec<Effect>, #[serde(default, skip_serializing_if = "Vec::is_empty")] if_unpaid: Vec<Effect> },
    /// Echo payment: its controller pays `cost` or sacrifices this permanent. If that player controls a permanent with `StaticEffect::EchoPayZero`, the cost is {0} instead.
    Echo { cost: Vec<Cost> },
    Repeat { n: Amount, effects: Vec<Effect> },
    /// Start a new coin-flip series: zero the won-flip count and lost flag, and make `LastFlipWon` / `LastCostPaid` true so a `RepeatWhile` loop starts.
    ResetFlipTally,
    /// Run `effects`, then again while `cond` holds (do-while; at most `max` passes, default 20): "flip a coin until you lose".
    RepeatWhile {
        cond: Condition,
        effects: Vec<Effect>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<u8>,
    },
    /// Do the effects for each object matching a selector, with `Remembered` bound to it.
    /// `turn_order`: the objects are processed in turn order starting with the active player (a permanent counts as its controller; Kaboom!).
    ForEach { objects: Sel, effects: Vec<Effect>, #[serde(default, skip_serializing_if = "is_false")] turn_order: bool },
    /// Do the effects once for each selected PLAYER (in turn order); inside, `Sel::EachObject` is that player ("for each defending player", "each opponent, in turn").
    ForEachPlayer { players: Sel, effects: Vec<Effect> },
    /// "Each player chooses ... simultaneously" (phenomena such as Caught in a Parallel Universe, Two Streams Facility). The players are asked ONE BY ONE
    /// (in the order `players` selects: the effect's controller first, then turn order), but nothing is applied and nothing is shown to the others
    /// until EVERY player has chosen: each answer is only kept per chooser, no token is made, no card moves, no event names a pick. A later chooser can
    /// therefore never pick something an earlier chooser's result would create. Afterwards `then` runs once per chooser (ForEachPlayer style:
    /// `Sel::EachObject` = that chooser, `Sel::ChosenByEach { index }` = their picks in order; a chooser who picked nothing is skipped).
    /// Card choice: `zone` + `filter` (controller etc. relative to the chooser, so `controller: LeftOfYou` = "controlled by the player to their left"),
    /// `min..=max` picks (an ordered list: the first pick is the "first" creature); nothing eligible: that player picks nothing.
    /// Named choice: with `options` (non-empty) each player picks one of the texts instead; `remember: true` stores it per player and source card name for the
    /// rest of the game (read with `PlayerFilter::ChoseFirstOption` / `ChoseSecondOption`, flipped by `Effect::FlipPlayerChoices`); `then` is then run per chooser too.
    EachPlayerChooses {
        players: Sel,
        #[serde(default = "zone_battlefield")]
        zone: Zone,
        #[serde(default)]
        filter: FilterBox,
        #[serde(default)]
        min: u8,
        #[serde(default = "one_u8")]
        max: u8,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        options: Vec<String>,
        #[serde(default, skip_serializing_if = "is_false")]
        remember: bool,
        #[serde(default)]
        then: Vec<Effect>,
    },
    /// Every player's remembered named choice for this source card's name (`EachPlayerChooses { remember: true }`) becomes the other option ("each player who last chose green anchor chooses red waterfall, and vice versa").
    FlipPlayerChoices,
    /// Create a delayed triggered ability.
    Delayed { at: DelayedWhen, effects: Vec<Effect> },
    /// Create a temporary triggered ability that watches for events ("until end of turn, whenever ...", "when you next cast ..."):
    /// when `trigger` matches, `effects` go on the stack. `once` removes it after it fires.
    AddTrigger {
        trigger: Trigger,
        effects: Vec<Effect>,
        #[serde(default)]
        once: bool,
        #[serde(default)]
        until: TempUntil,
        /// Only for events about these permanents (chosen when the spell/ability resolves): "target creature gains 'whenever this deals combat damage ...' until end of turn".
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bind: Option<Sel>,
        /// Index (in this card's `abilities` array) of a dormant `Triggered` ability (give it `from: "Command"` so it never triggers by itself) that goes on the stack
        /// when the temporary trigger fires, choosing its targets then ("target creature gets +1/+1 counters ..." granted triggers: Heroic Sacrifice, Roads Go Ever, Ever On). `effects` is then unused.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ability: Option<u16>,
        /// Further triggers sharing this one's single delayed ability: whichever happens first fires it, and with `once` all are removed
        /// ("when you next cast an instant, cast a sorcery, or activate a loyalty ability": Repeated Reverberation).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        also: Vec<Trigger>,
        /// The trigger's `Remembered` is this selection, evaluated now, instead of the current remembered set ("when that token leaves the battlefield, put the exiled card into your hand": Ugin, the Ineffable).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        carry: Option<Box<Sel>>,
        /// Intervening "if": checked when the trigger event happens (nothing goes on the stack if false), evaluated for the effect's controller (Fight for the Throne).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cond: Option<Box<Condition>>,
    },
    /// Turn this permanent face up (morph); `megamorph` also puts a +1/+1 counter on it.
    /// Turn matching face-up permanents face down (a 2/2 creature with no name, types or abilities); `TurnFaceUp` (morph) turns a face-down permanent back.
    TurnFaceDown { what: Sel },
    /// The selected face-down permanents have these extra card types and creature types for as long as they stay face down ("they're 2/2 Cyberman artifact creatures": Death in Heaven; put after a `Move` with `remember`).
    FaceDownAs {
        what: Sel,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        add_types: Vec<CardType>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        add_subtypes: Vec<String>,
        /// The face-down permanent is not a creature: it has only `add_types` (Yedora, Grave Gardener: "It's a Forest land").
        #[serde(default, skip_serializing_if = "is_false")]
        not_creature: bool,
    },
    TurnFaceUp {
        #[serde(default)]
        megamorph: bool,
        /// Extra +1/+1 counters put on it as it is turned face up.
        #[serde(default, skip_serializing_if = "is_zero_u8")]
        counters: u8,
        /// Turn these face-down permanents face up instead of this one (Ixidor, Break Open, Kaust).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        what: Option<Sel>,
    },
    /// Manifest: put the top card of the player's library onto the battlefield face down as a 2/2 creature.
    Manifest {
        who: Sel,
        /// Who controls the manifested permanent when it isn't the library's owner ("you manifest the top card of that player's library").
        #[serde(default, skip_serializing_if = "Option::is_none")]
        controller: Option<Sel>,
        /// Manifest these cards instead of the library top: they are exiled, shuffled into a random order, then manifested under `who`'s control (Ghastly Conscription).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        what: Option<Box<Sel>>,
        /// With `what`: the source itself is manifested along with those cards (Jeskai Infiltrator: "exile it and the top card of your library in a face-down pile ... then manifest those cards").
        #[serde(default, skip_serializing_if = "is_false")]
        include_source: bool,
    },
    /// Cloak: like Manifest (the top card of the player's library face down as a 2/2) but the face-down permanent has ward {2}. The new permanent is Remembered.
    Cloak {
        who: Sel,
        /// Who controls the cloaked permanent when it isn't the library's owner ("cloak the top card of that player's library": Etrata, Deadly Fugitive).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        controller: Option<Sel>,
        /// Cloak these (exiled, or in hand: Vannifar, Evolved Enigma) cards instead of the library top: shuffled into a random order, then cloaked under `who`'s control (Become Anonymous).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        what: Option<Box<Sel>>,
        /// The cloaked permanents enter tapped.
        #[serde(default, skip_serializing_if = "is_false")]
        tapped: bool,
    },
    /// Manifest dread: look at the top two cards of your library, manifest one of them (face down 2/2) and put the other into your graveyard.
    ManifestDread { who: Sel },
    /// "Its owner puts it on their choice of the top or bottom of their library": each card's OWNER decides (yes = top, no = bottom).
    /// Works on permanents, cards in other zones and spells on the stack (target spell or creature).
    OwnerChoosesLibraryPosition { what: Sel },
    /// Put the selected cards on top of their owners' libraries; an owner who gets several of them chooses their order (one Pick, topmost first; Harmonic Convergence).
    PutOnTopOwnersOrder { what: Sel },
    /// The controller reorders the graveyard of the first selected player as they choose (one Pick, topmost first; skipped for fewer than two cards): Fossil Find.
    ReorderGraveyard { who: Sel },
    /// "For one spell this turn, you may spend mana as though it were mana of any type" (North Star): the controller's next spell this turn with a colored or colorless mana symbol uses the allowance up.
    SpendAnyTypeForOneSpell,
    /// "The next spell they cast this turn has cascade": the selected players' next spell cast this turn gains cascade (Bigger on the Inside).
    NextSpellCascade { who: Sel },
    /// Until end of turn, each spell the player casts from their hand has cascade (Yidris, Maelstrom Wielder).
    HandSpellsCascade { who: Sel },
    /// The selected permanents gain every activated (non-loyalty) ability printed on the `from` cards for `duration` (Grell Philosopher). `pay_with`: each colored symbol
    /// of a granted ability's mana cost can also be paid with that color of mana ("you may spend blue mana as though it were mana of any color to activate those abilities").
    GainActivatedAbilitiesOf {
        what: Sel,
        from: Sel,
        duration: Duration,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pay_with: Option<Color>,
    },
    /// Each selected creature stops being goaded (one-shot goads of any player are removed; a static goad is not): Serene Sleuth.
    Ungoad { what: Sel },
    /// Detain: until your next turn the permanents can't attack or block and their activated abilities (mana abilities too) can't be activated.
    Detain { what: Sel },
    /// Haunt: exile this card (from the graveyard) haunting the target creature `what`. The card's own `from: Exile` triggers can use
    /// `Filter.haunted_by_this` ("when the creature it haunts dies").
    Haunt { what: Sel },
    /// Mutate: (the effect of the mutate `AlternativeCost`) merge this creature spell with the target creature (a non-Human creature you own).
    /// The caster chooses whether the spell goes over (yes) or under (no) the target. If the target is no longer legal the spell resolves as a creature.
    MutateOnto { target: Sel },
    /// Until end of turn you may play (cast / play as a land) cards from `zone` (your graveyard / library top / exile ...) matching `filter`
    /// ("until end of turn, you may cast creature spells from your graveyard").
    /// With `of`, the cards come from the graveyard of that player instead of yours; `any_mana` lets you spend mana as though it were mana of any type to cast them (Whispersteel Dagger).
    MayPlayThisTurn {
        zone: Zone,
        filter: FilterBox,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        of: Option<Sel>,
        #[serde(default, skip_serializing_if = "is_false")]
        any_mana: bool,
        /// "If you do, it enters tapped": a permanent played or cast from the graveyard through this permission enters tapped (Kaldring, the Rimestaff).
        #[serde(default, skip_serializing_if = "is_false")]
        tapped: bool,
        /// "If a spell cast this way would be put into a graveyard, exile it instead" (graveyard permission only; Bösium Strip).
        #[serde(default, skip_serializing_if = "is_false")]
        exile_after: bool,
        /// The permission is for the matching cards in `zone` right now, not for ones that arrive later this turn (Kethis, the Hidden Hand).
        #[serde(default, skip_serializing_if = "is_false")]
        snapshot: bool,
    },
    /// Until end of turn the controller may play the top card of the selected players' libraries, spending mana as though it were mana of any color to cast them (Xanathar, Guild Kingpin).
    PlayTopOfLibraryThisTurn { whose: Sel },
    /// Until end of turn, for as long as the card now on top of your library stays there, you may play it without paying its mana cost (X is 0): Temporal Aperture (after `Shuffle` and `RevealTop`).
    PlayTopFreeWhileOnTop,
    /// The source assembles `n` Contraptions (no Contraption deck exists in this engine, so they are only counted in `Player.contraptions`; `Replace::Assemble` multiplies `n`): Steady-Handed Mook.
    Assemble { n: Amount },
    /// Double the value of X of the selected spell on the stack ("double the value of X": Unbound Flourishing; `what` is `TriggerObject` for a cast trigger).
    DoubleSpellX { what: Sel },
    /// Until end of turn you may cast creature spells matching `filter` from your graveyard by foraging in addition to their other costs
    /// (forage: exile three other cards from your graveyard or sacrifice a Food); such a creature enters with a finality counter (Osteomancer Adept).
    MayCastForaging { filter: FilterBox },
    /// Until end of turn the controller may cast each selected creature / nonland card in a graveyard (any graveyard), paying its costs (Havengul Lich).
    MayCastCardThisTurn {
        what: Sel,
        /// "If that spell would be put into your graveyard, exile it instead" (Mavinda).
        #[serde(default, skip_serializing_if = "is_false")]
        exile_after: bool,
        /// The spell costs this much generic mana more unless it targets a creature the caster controls (Mavinda, Students' Advocate: 8).
        #[serde(default, skip_serializing_if = "is_zero_u8")]
        tax_unless_targets_own_creature: u8,
    },
    /// The `to` permanents gain every activated ability printed on each card `from` selects (a card on the stack or in a graveyard counts) for `duration` (Havengul Lich: "gains all activated abilities of that card until end of turn").
    GrantActivatedAbilitiesOf { from: Sel, to: Sel, duration: Duration },
    /// Earthbend N: `what` (a land you control) becomes a 0/0 haste creature (still a land) with N +1/+1 counters; when it dies or is exiled it returns
    /// to the battlefield tapped. Expands to Animate + AddCounters + two bound AddTriggers.
    Earthbend { what: Sel, n: Amount },
    /// "You may [reveal] a <filter> card you own from outside the game and put it into your hand" (the Wishes, Mastermind's Acquisition): `who` may take up to `max`
    /// cards matching `filter` from their own sideboard into hand (optionally revealing them). No sideboard or no match: nothing happens.
    FetchOutsideGame {
        who: Sel,
        #[serde(default)]
        filter: FilterBox,
        /// The card is revealed as it is put into the hand.
        #[serde(default, skip_serializing_if = "is_false")]
        reveal: bool,
        #[serde(default = "one_u8")]
        max: u8,
    },
    /// "You may play a card you own from outside the game this turn" (Wish): until end of turn the controller may play lands and cast spells from their own sideboard.
    /// With `filter`, instead only ONE matching card (Frontier Explorer: a basic Plains; Your Wish Is My Command: an instant or sorcery) may be played / cast from the sideboard this turn and the permission is used up by it;
    /// `extra_mode`: a modal spell cast that way may have one additional mode chosen. Filtered grants allow lands and spells alike.
    MayPlayFromOutsideGame {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filter: Option<Box<Filter>>,
        #[serde(default, skip_serializing_if = "is_false")]
        extra_mode: bool,
    },
    /// "Add <cards> as your companions" (The Keeper of Kaldra): creates each named card in the controller's sideboard as one of their companions ({3} puts it into hand, once each).
    AddCompanions { names: Vec<String> },
    /// "Give an opponent a card you own from outside the game" (Collector Protector): each selected sideboard card moves into the first selected player's sideboard (and becomes theirs).
    GiveToSideboard { what: Sel, to: Sel },
    /// A one-shot effect that does nothing itself (reminder-only abilities).
    Nothing,
}

/// How long a temporary triggered ability (Effect::AddTrigger) lasts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum TempUntil {
    #[default]
    EndOfTurn,
    /// Until the start of its controller's next turn.
    YourNextTurn,
    /// Until the end of its controller's next turn (Season of the Bold).
    EndOfYourNextTurn,
    /// Until the end of the next turn of the player in the trigger's first `Remembered` object (give `carry: DefendingPlayer`); it fires only for events caused by that player (Nuka-Nuke Launcher).
    EndOfCarriedPlayersNextTurn,
    /// Until the end of the current combat (Love on the Battlefield).
    EndOfCombat,
    Forever,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum BudgetKind {
    ManaValue,
    Power,
    Toughness,
}

/// Which group of subtypes a "loses all creature types" / "nonbasic lands are Islands" effect replaces; the rest of the subtypes stay.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export)]
pub enum SubtypeKind {
    Creature,
    Land,
}

impl SubtypeKind {
    /// Does this subtype belong to the group (anything that isn't an artifact / enchantment / land / spell / battle type counts as a creature type)?
    pub fn contains(self, name: &str) -> bool {
        const LAND: &[&str] = &["Plains", "Island", "Swamp", "Mountain", "Forest", "Desert", "Gate", "Lair", "Locus", "Cave", "Sphere", "Tower", "Urza's", "Mine", "Power-Plant", "Cloud", "Town", "Planet"];
        const OTHER: &[&str] = &[
            "Equipment", "Vehicle", "Food", "Clue", "Treasure", "Blood", "Gold", "Powerstone", "Map", "Incubator", "Junk", "Fortification", "Contraption", "Attraction", "Spacecraft", "Lander", "Shard", "Aura", "Saga", "Class", "Curse", "Shrine", "Background", "Role", "Case", "Room", "Cartouche", "Arcane", "Lesson", "Trap", "Adventure", "Omen", "Siege", "Mount",
        ];
        let land = LAND.contains(&name);
        match self {
            SubtypeKind::Land => land,
            SubtypeKind::Creature => !land && !OTHER.contains(&name),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export)]
pub enum KeywordAction {
    Trains,
    Evolves,
    Mentors,
    /// A creature fights (emitted for each fighter; `Trigger::KeywordActionDone` with this action).
    Fights,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum DelayedWhen {
    NextEndStep,
    NextUpkeep,
    /// The beginning of the controller's next upkeep.
    NextYourUpkeep,
    /// The beginning of the controller's next precombat main phase.
    NextYourMainPhase,
    /// The beginning of the controller's next end step (this turn's if it has not begun yet): Dragonhawk, Fate's Tempest.
    NextYourEndStep,
    /// As the controller untaps in their next untap step, before untapping (the effects happen at once, no stack): Undiscovered Paradise.
    NextYourUntap,
    /// The beginning of the next upkeep of the owner of the source card (Phytotitan).
    NextOwnerUpkeep,
    /// The beginning of the next upkeep of the player the creating trigger was about (its `TriggerPlayer`: the player a creature dealt damage to; Sabertooth Cobra).
    NextTriggerPlayerUpkeep,
    /// The beginning of the next upkeep of the first player the ability targeted, fixed when the delayed trigger is created; `TriggerPlayer` is that player (Plague Reaver: "under that player's control at the beginning of their next upkeep").
    NextTargetPlayerUpkeep,
    /// The beginning of the draw step of the player the creating trigger was about (its `TriggerPlayer`: the player a creature dealt damage to; Nafs Asp).
    NextTriggerPlayerDrawStep,
    /// The beginning of the end step of the player the source chose (`ChoosePlayer`/`ChooseKind::Opponent`), fixed when the delayed trigger is created; `TriggerPlayer` is that player (Faramir, Prince of Ithilien).
    NextChosenPlayerEndStep,
    /// The beginning of the end step of the player stored in the delayed trigger's `ctx.player` (set when it is created: the owner of the exiled card; The Eternal Wanderer).
    NextCtxPlayerEndStep,
    /// The beginning of the controller's next declare attackers step, after attackers are declared (Meandering Towershell). The step is not skipped for it even when nothing attacks.
    NextYourDeclareAttackers,
    EndOfCombat,
    EndOfTurn,
}

/// What restricted mana may pay for (see `ManaProduce::Restricted`).
#[derive(Clone, Debug, PartialEq)]
pub struct ManaRestrict {
    pub spell: Option<FilterBox>,
    pub ability: Option<FilterBox>,
    pub rider: Option<ManaRider>,
    /// Also payable toward a cost that contains {C} (mana cost or ability mana cost).
    pub cost_c: bool,
    /// Only payable toward a cumulative upkeep cost (Snowfall).
    pub upkeep: bool,
    /// Also payable toward unlocking a door of a Room (Smoky Lounge: "Room spells and unlock doors").
    pub unlock: bool,
    /// The mana can't be spent to pay generic mana costs (Jegantha, the Wellspring).
    pub no_generic: bool,
    /// The rider only applies when the mana is spent on a spell matching this filter (Scaled Nurturer: a Dragon creature spell); the mana itself is unrestricted.
    pub rider_spell: Option<FilterBox>,
}

/// What happens to a spell that mana with a rider is spent on ("that spell can't be countered", "it gains haste", "it enters with an additional +1/+1 counter").
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub enum ManaRider {
    CantBeCountered,
    /// The permanent spell gains haste until end of turn.
    Haste,
    /// The permanent spell gains haste for as long as it stays on the battlefield (Hall of the Bandit Lord: no duration is stated).
    HasteForever,
    /// The permanent spell enters with an additional +1/+1 counter (once per mana spent).
    PlusOneCounter,
    /// The permanent spell enters with an additional +1/+1 counter (once per mana spent) and gains hexproof until its caster's next turn (Jade Orb of Dragonkind).
    PlusOneCounterHexproof,
    /// The commander spell enters with additional +1/+1 counters equal to the number of times it has been cast from the command zone this game, this cast included (Opal Palace).
    PlusCountersCommanderCasts,
    /// "When that mana is spent to cast ..., scry N": the caster scries (a trigger above the spell on the stack).
    Scry(u8),
    /// "... draw a card" (N cards).
    Draw(u8),
    /// "... copy that spell. You may choose new targets for the copy."
    CopySpell,
    /// "... scry X, where X is the number of times you've cast your commander from the command zone" (Study Hall).
    ScryCommanderCasts,
    /// "... you gain N life" (Scaled Nurturer).
    GainLife(u8),
    /// "... you may exile that spell instead of putting it into its owner's graveyard as it resolves" (Forger's Foundry): the card is exiled with the permanent that made the mana.
    ExileSpell,
    /// "... it gains riot" (Domri, Chaos Bringer): a creature spell enters with its controller's choice of a +1/+1 counter or haste.
    Riot,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "t", deny_unknown_fields)]
pub enum ManaProduce {
    Fixed { m: crate::mana::ManaType, n: u8 },
    /// `n` mana that can only be spent to cast spells matching the filter ("Spend this mana only to cast creature spells").
    /// It is only counted when paying for such a spell.
    /// `spell` limits it to spells matching the filter; `ability` (a filter on the SOURCE of the ability) also lets it pay to activate abilities of
    /// matching sources ("spend this mana only to cast creature spells or activate abilities of creature sources"). Give only `ability` for
    /// "only to activate abilities" (empty filter = any source).
    Restricted {
        m: crate::mana::ManaType,
        n: u8,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        spell: Option<FilterBox>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ability: Option<FilterBox>,
        /// A computed amount instead of the fixed `n` ("add X mana, spend it only on ...").
        #[serde(default, skip_serializing_if = "Option::is_none")]
        amount: Option<Amount>,
        /// What happens to the spell this mana is spent on.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rider: Option<ManaRider>,
        /// "... or pay a cost that contains {C}": the mana also pays any cost with a {C} symbol.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        cost_c: bool,
        /// "Spend this mana only to pay cumulative upkeep costs" (Snowfall); give no `spell`/`ability`.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        upkeep: bool,
        /// "... or unlock doors": the mana also pays the mana cost of unlocking a door of a Room (Smoky Lounge).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        unlock: bool,
        /// The mana is of the colour this permanent chose as it entered instead of `m` (Throne of Eldraine).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        chosen: bool,
        /// "This mana can't be spent to pay generic mana costs" (Jegantha, the Wellspring).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        no_generic: bool,
    },
    /// `n` unrestricted mana of type `m`; when it is spent on a spell matching `spell` (any spell if absent) that spell gets the rider
    /// ("Add {R}. If that mana is spent on a Dragon creature spell, it gains haste"). Only mana tapped while paying carries the rider.
    Rider {
        m: crate::mana::ManaType,
        n: u8,
        rider: ManaRider,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        spell: Option<FilterBox>,
    },
    /// `n` mana of any colour that can only be spent as `Restricted` mana can (`spell` / `ability` filters). Each mana is a separate colour choice
    /// ("add two mana in any combination of colors, spend only on creature spells"); with `one_color` all `n` share one chosen colour.
    RestrictedAny {
        n: u8,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        spell: Option<FilterBox>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ability: Option<FilterBox>,
        #[serde(default)]
        one_color: bool,
        /// The colour chosen must differ from the one chosen for the previous `RestrictedAny` group of the same `AddMana` (Open the Omenpaths: "two mana of any one color and two mana of any other color").
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        other_color: bool,
        /// When set, the number of mana is this amount instead of `n` (Klauth, Metamorphosis, Crucible of the Spirit Dragon; effects only).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        amount: Option<Amount>,
    },
    /// One mana of any of this permanent's own colours (none if it is colourless), spent only to activate abilities of sources matching `ability` (Tazri, Stalwart Survivor).
    OwnColorRestricted { ability: FilterBox },
    /// One mana of any color.
    AnyColor { n: u8 },
    /// `n` mana of any one color, with a rider when it is spent on a spell matching `spell` (Primal Wellspring: "when that mana is spent to cast an instant or sorcery spell, copy that spell").
    AnyColorRider {
        n: u8,
        rider: ManaRider,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        spell: Option<FilterBox>,
    },
    /// One mana of any color that a land an opponent controls could produce (Exotic Orchard).
    AnyColorOpponentsCould,
    /// One mana of a type the permanent sacrificed to pay this ability's cost could produce (Squandered Resources).
    AnyTypeOfSacrificed,
    /// One mana of a type the permanent untapped to pay this ability's cost could produce (Benthic Explorers).
    AnyTypeOfUntapped,
    /// `n` mana of one chosen color (Gilded Lotus: three of any one color).
    AnyOneColor { n: u8 },
    /// A computed amount of one fixed type (Empowered Autogenerator: X mana where X = charge counters; Temple: two when ...).
    Scaled { m: crate::mana::ManaType, amount: Amount },
    /// X mana of any one color, X computed.
    AnyOneColorScaled { amount: Amount },
    /// `amount` mana, each of which may be a different color ("add X mana in any combination of colors", "... of {G} and/or {W}": `colors` limits the choice, empty = any color).
    AnyColorScaled {
        amount: Amount,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        colors: Vec<Color>,
        /// Instead of `colors`: the choice is limited to the colors of the first selected card ("in any combination of its colors": Omnath, Locus of All).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        colors_of: Option<Box<Sel>>,
    },
    /// One mana of a color a permanent you control could produce (Fellwar Stone...).
    AnyColorYouCould,
    /// One mana of a color that a permanent you control matching `filter` could produce (Gond Gate: Gates).
    AnyColorYouCouldFrom { filter: FilterBox, /// "any type" (Reflecting Pool, Naga Vitalist): colorless counts too.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")] colorless: bool,
        /// How many mana of the chosen type (Incubation Druid: 3).
        #[serde(default = "one_u8", skip_serializing_if = "is_one_u8")] n: u8 },
    /// One mana of a color among the permanents you control matching `filter` (Mox Amber: legendary creatures and planeswalkers); no mana if there are none.
    AnyColorAmong { filter: FilterBox, /// Look at cards in this zone of yours instead of the battlefield (The Grey Havens: legendary creature cards in your graveyard).
        #[serde(default, skip_serializing_if = "Option::is_none")] zone: Option<Zone> },
    /// One mana of a color of a card exiled with this permanent (Chrome Mox's imprint); no mana if there is none.
    AnyColorOfExiledWith,
    /// One additional mana of a type the land that was just tapped for mana produced (Mana Flare, Heartbeat of Spring, Dictate of Karametra): used by a `TapsForMana` trigger. With several produced types the first is used.
    OneProduced,
    /// One mana of a color from this card's color identity (Commander's Sphere).
    AnyCommanderColor,
    /// One mana of a color from your commander's color identity, with a rider when spent on a spell matching `spell` (Study Hall, Path of Ancestry).
    AnyCommanderColorRider {
        rider: ManaRider,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        spell: Option<FilterBox>,
    },
    /// Mana equal to the mana cost of the first selected permanent: one mana of each coloured symbol's colour, colourless for generic and {C} symbols, nothing for {X};
    /// a hybrid symbol lets the controller choose one of its colours (Elemental Resonance).
    CostOf { of: Sel },
    /// One mana of each coloured symbol in the mana cost of the card this ability's `Cost::Mill` just milled (hybrid symbols: one of either colour; generic, {C}, {X} and snow give nothing; Charmed Pendant).
    MilledSymbols,
    /// One mana of the type last noted by `Effect::NoteManaSpent` on this permanent; no mana if none was noted.
    NotedType,
    /// The type and amount of mana last noted by `Effect::NoteManaSpentAll` on this permanent, spendable only on spells matching `spell` (Ice Cauldron: `last_exiled_with_this`).
    NotedMana { #[serde(default, skip_serializing_if = "Option::is_none")] spell: Option<FilterBox> },
    /// `n` mana of the colour this permanent chose as it entered.
    ChosenColor { n: u8 },
    /// `n` mana of the second colour this permanent chose as it entered (`ChooseKind::SecondColor`; Cryptic Spires: one ability per circled colour).
    SecondChosenColor { n: u8 },
    /// Choose a color as the ability is activated; add mana of it equal to the controller's devotion to that color (Nyx Lotus).
    DevotionOneColor,
    /// A computed amount of mana of the colour this permanent chose as it entered (Nyx Lotus: devotion to the chosen colour).
    ChosenColorScaled { amount: Amount },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum StaticEffect {
    /// "Spend only mana produced by creatures to cast this spell": while this card is being cast, only mana from sources matching `filter` (tapped
    /// for it, or restricted mana they produced) pays for it; mana already floating in the pool can't be spent (its source isn't tracked): Myr Superion.
    SpendOnlyManaFrom { filter: FilterBox },
    /// Matching spells (cast by players matching `whose`, relative to the source's controller) can't cost less than `min` mana in total, after every
    /// other cost change, alternative cost and chosen X: the shortfall is added as generic mana (Trinisphere: `min` 3, condition `Not SourceTapped`).
    CostMinimum {
        #[serde(default)]
        filter: FilterBox,
        #[serde(default)]
        whose: PlayerFilter,
        min: u8,
    },
    /// The affected creatures can attack as though they had haste.
    AttackAsThoughHaste { affects: FilterBox },
    /// The affected creatures' {T} / {Q} abilities can be activated as though they had haste (Tyvar, Thousand-Year Elixir); attacking is not covered.
    ActivateAsThoughHaste { affects: FilterBox },
    /// Winter Orb: while it applies, matching players can't untap more than `max` permanents matching `filter` during their untap steps.
    /// Torpor Orb / Hushbringer / Elesh Norn: triggered abilities of permanents controlled by `whose` (relative to this permanent's controller) that
    /// would trigger because a permanent matching `cause` entered / died (`event`) don't trigger. A trigger's own source counts as the cause when it is the permanent.
    SuppressTriggers {
        event: TriggerCause,
        #[serde(default)]
        cause: FilterBox,
        #[serde(default)]
        whose: PlayerFilter,
    },
    /// During each player's untap step (while the condition holds), that player chooses one of `options` and can untap only permanents of that type this step (Storage Matrix: artifact, creature or land; use the condition "this is untapped").
    UntapOnlyChosenType { options: Vec<CardType> },
    UntapLimit {
        who: PlayerFilter,
        filter: FilterBox,
        max: u8,
    },
    /// Matching permanents can't be the target of spells (matching `spell`, default any spell; any controller). Abilities may still target them
    /// (Dense Foliage, Anti-Magic Aura; Bartel Runeaxe / Tetsuo Umezawa: `spell` = Aura spells).
    /// Spells cast by `whose` that target a matching permanent cost this much additional life to cast (Terror of the Peaks).
    ExtraLifeToTargetSpells { affects: FilterBox, whose: PlayerFilter, life: i32 },
    CantBeTargetedBySpells { affects: FilterBox, #[serde(default)] spell: FilterBox },
    /// Matching permanents can't be the target of spells or abilities (of any controller, the permanent's own included) whose source matches `source`
    /// ("blue spells or abilities from blue sources": Spellbane Centaur, `source` = `{colors:[Blue]}`).
    /// `abilities_only`: spells are not stopped, only abilities from matching sources ("abilities from artifact sources": Artifact Ward).
    CantBeTargetedBySource { affects: FilterBox, source: FilterBox, #[serde(default, skip_serializing_if = "is_false")] abilities_only: bool },
    /// Flagbearer: while an opponent of this permanent's controller chooses targets as part of casting a spell or activating an ability, they must choose at least one Flagbearer
    /// on the battlefield if able (Coalition Honor Guard). Enforced per target slot: a slot where a Flagbearer is a legal target only offers Flagbearers (a spell with several such slots must aim all of them at Flagbearers).
    FlagbearerMustBeTargeted,
    /// Matching permanents can't be the target of spells or abilities that can target only permanents of this subtype
    /// (a `Permanent` target slot whose filter requires `subtype`: Wall of Shadows, "only Walls").
    CantBeTargetedByOnlySubtype { affects: FilterBox, subtype: String },
    /// "Cards in graveyards can't be the targets of spells or abilities" (Dennick, Underworld Cerberus).
    GraveyardCardsCantBeTargeted,
    /// Matching permanents on the battlefield and matching cards in graveyards can't be the targets of spells or abilities controlled by
    /// this permanent's controller's opponents (Tomik, Distinguished Advokist).
    CantBeTargetedByOpponents { affects: FilterBox },
    /// "Your opponents can't play land cards from graveyards" (players matching `who` can't play lands from a graveyard; Tomik).
    CantPlayLandsFromGraveyard { who: PlayerFilter },
    /// Matching permanents (and, with `players`, this ability's controller's opponents) can be targeted "as though they didn't have hexproof"
    /// (Kaya, Bane of the Dead: `own_only` = only spells and abilities this ability's controller controls; Nowhere to Run: `ignore_ward` too).
    IgnoreHexproof {
        affects: FilterBox,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        players: bool,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        own_only: bool,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        ignore_ward: bool,
    },
    /// Granted to a player (GrantStatic on a target player): that player's spells and abilities may target matching permanents (filter relative
    /// to the granting source) as though they didn't have shroud (Autumn Willow).
    IgnoreShroud { affects: FilterBox },
    /// Granted to a player with `GrantStatic`: that player ignores "can't search libraries" effects (Leonin Arbiter's pay-{2} ability).
    IgnoreSearchBan,
    /// "You may pay {0} rather than pay the echo cost for permanents you control" (Thick-Skinned Goblin); read by `Effect::Echo`.
    EchoPayZero,
    /// You may foretell cards from your hand on any player's turn (Cosmos Charger); read by the generated foretell action.
    ForetellAnyTime,
    /// Matching creatures can't crew Vehicles (Intercessor's Arrest, Bound in Gold).
    CantCrew { affects: FilterBox },
    /// Matching permanents can't transform (Immerwolf, Bound by Moonsilver).
    CantTransform { affects: FilterBox },
    /// While voting, the controller votes an additional time (Ballot Broker, Brago's Representative, Tivit, Seller of Secrets).
    ExtraVote,
    /// "You may vote an additional time": the extra ballot can be declined (Ballot Broker, Tivit, Seller of Secrets).
    OptionalExtraVote,
    /// "If an opponent would face a villainous choice, they face that choice an additional time" (The Valeyard).
    VillainousChoiceTwice,
    /// "All instances of color words in the text of spells and permanents are changed to the chosen color word" (Swirl the Mists; the colour is the source's chosen colour).
    AllColorWordsChosen,
    /// Affected permanents get +power/+toughness.
    Modify {
        affects: FilterBox,
        power: Amount,
        toughness: Amount,
        /// Evaluate `power` / `toughness` once per affected permanent, with `Sel::EachObject` = that permanent
        /// ("+1/+1 for each other creature you control with the same name as it": Mirror Box, Coat of Arms, Toxrill, Bruenor).
        #[serde(default, skip_serializing_if = "is_false")]
        each: bool,
    },
    GrantKeyword { affects: FilterBox, keyword: Keyword },
    /// Affected creatures lose `keyword` (and, applied after grants, can't keep it: Archetype of Courage / Mystic Decree).
    LoseKeyword { affects: FilterBox, keyword: Keyword },
    /// The affected permanents lose hexproof, indestructible, protection, shroud and ward (Shay Cormac).
    LoseDefenses { affects: FilterBox },
    /// Cards in graveyards lose all abilities (Yixlid Jailer): no flashback / graveyard alternative costs, no graveyard activated abilities.
    GraveyardLoseAbilities,
    RemoveAbilities { affects: FilterBox, /// "lose all abilities except mana abilities" (Blood Sun).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")] except_mana: bool,
        /// Also loses the mana abilities its basic land types give, but keeps the types (Imprisoned in the Moon).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")] no_land_mana: bool },
    SetBasePT { affects: FilterBox, power: i32, toughness: i32 },
    /// Base power and toughness from amounts evaluated continuously (Aettir and Priwen: X/X where X is your life total).
    /// `each`: the amounts are evaluated per affected permanent (`Sel::EachObject` = that permanent; March of the Machines, Starfield of Nyx).
    SetBasePTAmount { affects: FilterBox, power: Amount, toughness: Amount, #[serde(default, skip_serializing_if = "is_false")] each: bool },
    /// Only the base power ("has base power 4"); toughness stays as it is (counters/pumps still apply).
    SetBasePower { affects: FilterBox, power: i32 },
    /// Only the base toughness.
    SetBaseToughness { affects: FilterBox, toughness: i32 },
    /// Characteristic-defining ability: base power and toughness from an amount.
    CdaPT { power: Amount, toughness: Amount },
    AddType { affects: FilterBox, types: Vec<CardType>, subtypes: Vec<String> },
    /// Matching permanents gain a supertype (Legendary: Leyline of Singularity; Snow: Glittering Frost).
    AddSupertype { affects: FilterBox, supertype: Supertype },
    /// Matching permanents lose a supertype (Melting: lands are no longer snow).
    RemoveSupertype { affects: FilterBox, supertype: Supertype },
    SetTypes { affects: FilterBox, types: Vec<CardType>, subtypes: Vec<String> },
    /// Matching permanents lose these card types and creature/other subtypes (Princess Yue, Veiled Crocodile, Luxior).
    RemoveTypes { affects: FilterBox, types: Vec<CardType>, #[serde(default, skip_serializing_if = "Vec::is_empty")] subtypes: Vec<String> },
    /// Matching permanents get this name (Psychic Paper, Witness Protection, Princess Yue).
    SetName { affects: FilterBox, name: String },
    /// Matching permanents get the card name and creature type this source chose (`ChooseCardName` + `ChooseCreatureType` effects): Psychic Paper. Their other creature types are replaced.
    SetChosenNameAndType { affects: FilterBox },
    /// The affected objects have the names of all nonlegendary creature cards in addition to their own (Spy Kit); honoured by name filters (`name`, `chosen_name`, `same_name_as`, `not_same_name_as`).
    AllCreatureNames { affects: FilterBox },
    SetColors { affects: FilterBox, colors: Vec<Color> },
    AddColors { affects: FilterBox, colors: Vec<Color> },
    /// The attached permanent gets the modification (Auras, Equipment).
    EnchantedGets {
        power: Amount,
        toughness: Amount,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        keywords: Vec<Keyword>,
    },
    /// "As long as enchanted permanent isn't a creature, it's a creature (in addition to its types) with power and toughness `power`/`toughness`" (Animate Artifact).
    /// Judged in the type layer against the types it has then, so the P/T layer still applies after it has become a creature.
    EnchantedAnimate { power: Amount, toughness: Amount },
    /// Enchanted permanent can't attack and/or block (Pacifism).
    EnchantedCant { attack: bool, block: bool },
    /// Activated abilities of matching permanents (that aren't mana abilities) can't be activated (Detainment Spell, Pithing Needle style).
    CantActivate {
        affects: FilterBox,
        /// Mana abilities too ("its activated abilities can't be activated" includes them; Pithing Needle's don't).
        #[serde(default)]
        mana: bool,
        /// Only abilities with {T} in their costs (Katabatic Winds, Serra Bestiary).
        #[serde(default, skip_serializing_if = "is_false")]
        tap_only: bool,
        /// Only power-up abilities (Kang, the Conqueror: "during that turn, power-up abilities can't be activated"; use a dynamic `GrantStatic` with `affects: {}`).
        #[serde(default, skip_serializing_if = "is_false")]
        power_up_only: bool,
    },
    /// Matching face-down permanents can't be turned face up (morph, manifest, cloak or an effect): Karlov Watchdog (with a `YourTurn` condition).
    CantTurnFaceUp { affects: FilterBox },
    /// If the controller would copy a spell one or more times, they copy it that many times plus one more (Twinning Staff).
    CopySpellExtra {
        #[serde(default)]
        who: PlayerFilter,
    },
    /// Matching permanents have this ability (activated, mana or triggered) in addition to their own (Chromatic Lantern).
    GrantAbility { affects: FilterBox, ability: Box<Ability> },
    /// Matching permanents have every protection quality printed on a card exiled with the source (Eater of Virtue: "the same is true for protection").
    GrantExiledProtections { affects: FilterBox },
    /// Matching permanents have every protection quality printed on a creature card in any graveyard (Cairn Wanderer: "the same is true for protection").
    GrantGraveyardProtections { affects: FilterBox },
    /// Matching permanents have flying, first strike, double strike, deathtouch, haste, hexproof, indestructible, lifelink, menace, reach, trample, vigilance and protection
    /// whenever a creature card in exile with a `kind` counter has it (Rayami, First of the Fallen).
    ShareKeywordsFromExiledCounter { affects: FilterBox, kind: CounterKind },
    /// Matching permanents have protection from each colour that is not in the colour identity of the static's controller's commander(s) (Commander's Plate).
    GrantNonIdentityProtections { affects: FilterBox },
    /// "Has all activated abilities of <the cards selected by from>" (Dark Impostor, Myr Welder, Necrotic Ooze, Mirran Safehouse); the abilities are those printed on the selected cards.
    /// `non_mana`: only the activated abilities that are not mana abilities (Sharkey, Tyrant of the Shire).
    GrantAbilitiesFrom { affects: FilterBox, from: Sel, #[serde(default, skip_serializing_if = "is_false")] mana_only: bool, #[serde(default, skip_serializing_if = "is_false")] non_mana: bool, /// "You may activate each of those abilities only once each turn" (Locus of Enlightenment): the granted copies get `limit_per_turn: 1`.
 #[serde(default, skip_serializing_if = "is_false")] once_each_turn: bool,
 /// Grant the LOYALTY abilities of the selected cards instead of their other activated abilities (Nicol Bolas, Dragon-God: "has all loyalty abilities of all other planeswalkers").
 #[serde(default, skip_serializing_if = "is_false")] loyalty_only: bool,
    /// Grant the triggered abilities of the selected cards too (Koh, the Face Stealer: "all activated and triggered abilities").
    #[serde(default, skip_serializing_if = "is_false")] triggered: bool,
    /// A selected land with basic land types also grants the mana ability those types give it (Manascape Refractor: "all activated abilities of all lands").
    #[serde(default, skip_serializing_if = "is_false")] intrinsic_land_mana: bool },
    /// "If you would roll one or more dice, instead roll that many dice plus one and ignore the lowest roll" (Barbarian Class): the controller's die rolls get one extra die and only the highest counts.
    RollExtraDieIgnoreLowest,
    /// Buyback costs cost this much more / less (generic part only; Memory Crystal: `generic: -2`). Applies to every player's buyback kickers (`Kicker.buyback`).
    BuybackCostChange { generic: i32 },
    /// Spells matching the filter cost this much more / less to cast.
    CostChange {
        filter: FilterBox,
        whose: PlayerFilter,
        generic: i32,
        /// Multiply `generic` by this amount ("costs {1} less for each artifact you control"; Affinity, Blasphemous Act).
        /// A spell's own CostChange (`filter.this`) applies while it is in hand, on the stack or in the command zone.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        per: Option<Amount>,
        /// Does not apply to spells cast from this zone ("spells you cast from anywhere other than your hand": `not_from: Hand`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        not_from: Option<Zone>,
        /// The change applies to the coloured mana of this colour in the cost instead of the generic part: a negative `generic` removes that many coloured shards ("costs {R} less to cast"), a POSITIVE one adds them ("white spells cost {W} more to cast").
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color: Option<Color>,
        /// Used up by the first matching spell the player casts (only meaningful in `GrantStatic`: "the next spell you cast this turn costs {1} less").
        #[serde(default, skip_serializing_if = "is_false")]
        once: bool,
        /// `per` is counted from the CASTER's point of view ("for each artifact its controller controls": Hum of the Radix) instead of the static's controller.
        #[serde(default, skip_serializing_if = "is_false")]
        per_caster: bool,
        /// In `GrantStatic`: `per` is evaluated once when the effect resolves and fixed for its duration (Commander Liara Portyr: "where X is the number of players being attacked").
        #[serde(default, skip_serializing_if = "is_false")]
        fixed: bool,
        /// Applies only to spells cast for their flashback cost (a printed Flashback alternative cost cast from the graveyard: Catalyst Stone).
        #[serde(default, skip_serializing_if = "is_false")]
        flashback: bool,
        /// Applies only to spells cast for their dash alternative cost (Warbringer: "Dash costs you pay cost {2} less").
        #[serde(default, skip_serializing_if = "is_false")]
        dash: bool,
        /// With `color` and a reduction: when the cost has no coloured mana of that colour left, the reduction takes off generic mana instead ("This can reduce generic costs": Aang, Master of Elements).
        #[serde(default, skip_serializing_if = "is_false")]
        generic_fallback: bool,
        /// In `GrantStatic` with `once`: the matching spell is cast without paying its mana cost (Apex Observatory: "the next spell you cast this turn of the chosen type"). Extra costs still apply.
        #[serde(default, skip_serializing_if = "is_false")]
        free: bool,
    },
    /// "As an additional cost to cast <spells matching filter>, you may pay <cost>. Those spells cost {C} less to cast if you paid this way" (Defiler of Dreams).
    /// The caster is asked once per cast; paying adds `cost` to the spell's additional costs and removes one `reduce` coloured shard from its mana cost (only the coloured mana is reduced).
    OptionalSpellCost {
        filter: FilterBox,
        #[serde(default = "you_filter")]
        whose: PlayerFilter,
        cost: Vec<Cost>,
        reduce: Color,
    },
    /// "As an additional cost to cast spells matching `filter`, you may pay any amount of mana. If you do, that permanent enters with that many additional +1/+1 counters" (Chorus of the Conclave).
    /// The caster is asked for the amount (0 to the most they can afford on top of the spell's cost) before the spell's other choices.
    SpellManaForCounters {
        filter: FilterBox,
        #[serde(default = "you_filter")]
        whose: PlayerFilter,
    },
    /// "As long as this isn't on the battlefield, it's a P/T `subtype` creature in addition to its other types" (Grist, the Hunger Tide): applies to the card in every zone but the battlefield (hand, library, graveyard, exile, stack, command zone).
    NonBattlefieldCreature { subtype: String, power: i32, toughness: i32 },
    /// "You may spend mana from snow sources as though it were mana of any color to cast <spells matching filter>" (Draugr Necromancer: exiled opponents' cards with ice counters).
    /// The `filter` is judged against the spell being cast; `whose` is the caster, relative to this permanent's controller.
    SpendSnowManaAsAnyColor {
        filter: FilterBox,
        #[serde(default = "you_filter")]
        whose: PlayerFilter,
    },
    /// Damage marked on this creature isn't removed during cleanup steps (Ancient Adamantoise).
    DamageStaysAtCleanup,
    /// Counters remain on this card as it moves to any zone other than a player's hand or library (Me, the Immortal).
    CountersRemain,
    /// "You may spend `from` mana as though it were `to` mana" (Sunglasses of Urza): each {to} / {to/P} symbol of a cost the controller pays can also be paid with `from` mana
    /// (planned as a `{to/from}` hybrid; other hybrid symbols are left alone).
    SpendManaAsColor { from: Color, to: Color },
    /// "As long as the top card of your graveyard is a creature card, this creature has the full text of that card": while the top card of its controller's graveyard is a creature card, this permanent has that card's name, types, colors, power, toughness, keywords and
    /// triggered / activated / static abilities in addition to its own printed abilities (Volrath's Shapeshifter). Its mana cost / mana value stay its own.
    TextOfTopGraveyardCreature,
    /// Colored mana that spells and abilities the controller controls would add is white mana instead (False Dawn: grant with `GrantStatic` `dynamic` until end of turn). Applies to mana abilities and to `AddMana`; the mana-source planner shows white only.
    ColoredManaBecomesWhite,
    /// "For each {C} in a cost, you may pay 2 life rather than pay that mana" (K'rrik, Son of Yawgmoth): every `color` symbol of a cost the controller pays is treated as a Phyrexian symbol.
    PayLifeForMana { color: Color },
    /// "You may spend mana as though it were mana of any color (or type) to cast spells matching `filter`" (Oath of Nissa, Vizier of the Menagerie, Mycosynth Lattice): the spell's colored / colorless costs are paid with any mana.
    SpendManaAsAnyType {
        #[serde(default = "you_filter")]
        whose: PlayerFilter,
        #[serde(default)]
        filter: FilterBox,
        /// Instead of spells: mana spent to activate abilities of the battlefield permanents matching `filter` (Nathan Drake: permanents you control but don't own).
        #[serde(default, skip_serializing_if = "is_false")]
        abilities: bool,
    },
    /// Activated abilities (not mana abilities) of the permanents matching `filter` cost `generic` more / less to activate when activated by a player matching `whose`
    /// (relative to the static's controller). `per` multiplies `generic`. `min_one`: not below one mana in total ("can't reduce its total cost below one mana").
    /// Only the generic part of a mana cost is reduced. Equip, crew and other special abilities count as activated abilities.
    AbilityCostChange {
        filter: FilterBox,
        #[serde(default)]
        whose: PlayerFilter,
        generic: i32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        per: Option<Amount>,
        #[serde(default, skip_serializing_if = "is_false")]
        min_one: bool,
        /// Only equip abilities ("Equip abilities you activate cost {1} less"): an activated ability whose first effect attaches this Equipment.
        #[serde(default, skip_serializing_if = "is_false")]
        equip_only: bool,
        /// Only abilities whose first target is a permanent matching this filter (relative to this static's source: "Equip abilities that target this creature").
        /// Settled once the targets are chosen.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<FilterBox>,
        /// Instead of `generic`: remove these mana symbols from the ability's cost (each coloured / hybrid symbol that is present, generic reducing the generic part).
        /// "Reduce the cost by its mana cost if it entered this turn" (power-up): give the card's own mana cost with an `entered_this_turn` condition.
        /// Applies to every activated ability of the matching permanents, so use it on cards whose only activated ability is the one reduced.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        remove: Option<String>,
        /// Only abilities with a "Sacrifice this" cost ("{7}, {T}, Sacrifice this creature: ... costs {1} less ...": Qiqirn Merchant).
        #[serde(default, skip_serializing_if = "is_false")]
        sac_self_only: bool,
        /// Only boast abilities (limit once per turn, conditioned on the creature having attacked: Dragonkin Berserker).
        #[serde(default, skip_serializing_if = "is_false")]
        boast_only: bool,
        /// Only foretell special actions (an ability that exiles its card from hand and marks it foretold: Ranar the Ever-Watchful).
        #[serde(default, skip_serializing_if = "is_false")]
        foretell_only: bool,
        /// Only exhaust abilities (an activated ability marked `once: true`; Boom Scholar).
        #[serde(default, skip_serializing_if = "is_false")]
        once_only: bool,
        /// The matching abilities' mana cost becomes {0} ("you may pay {0} rather than pay cycling costs": New Perspectives); `generic` is ignored.
        #[serde(default, skip_serializing_if = "is_false")]
        free: bool,
        /// Only the first activated ability of an artifact that each affected player activates each turn (Tezzeret, Betrayer of Flesh; `Game.artifact_act_log`).
        #[serde(default, skip_serializing_if = "is_false")]
        first_per_turn: bool,
        /// Only plot special actions (an ability that exiles its card from hand and marks it plotted: Doc Aurlock, Grizzled Genius).
        #[serde(default, skip_serializing_if = "is_false")]
        plot_only: bool,
        /// Only power-up abilities (`Activated.power_up`; Hulk, Gamma Goliath).
        #[serde(default, skip_serializing_if = "is_false")]
        power_up_only: bool,
        /// Only the first power-up ability each affected player activates each turn (Advancing the Spirit, with `free`; `Game.power_up_act_log`).
        #[serde(default, skip_serializing_if = "is_false")]
        first_power_up_per_turn: bool,
        /// With `target`: only the first ability each affected player activates during their own turn that targets a creature they control (Professor Hojo; `Game.creature_tgt_act_log`).
        #[serde(default, skip_serializing_if = "is_false")]
        first_creature_target_per_turn: bool,
        /// Only abilities of cards in this zone, whichever zone that is ("activated abilities of cards in your graveyard": Convergence of Dominion); the filter's controller test is relative to the static's controller as usual.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        in_zone: Option<Zone>,
        /// Only while the affected player hasn't cycled a card this turn ("the first card you cycle each turn": Gavi, Nest Warden, with `free`).
        #[serde(default, skip_serializing_if = "is_false")]
        first_cycle_per_turn: bool,
    },
    /// Unlocking a Room's doors (the special action) costs `generic` more (positive) or less (negative) for the matching players, relative to the static's controller (Inquisitive Glimmer).
    UnlockCostChange {
        #[serde(default = "you_filter")]
        whose: PlayerFilter,
        generic: i32,
    },
    /// Activated abilities (not mana abilities) of the permanents matching `filter` cost the additional `cost` parts to activate (Brutal Suppression: "Sacrifice a land").
    /// With `per_symbol`, the cost is paid once for each mana symbol of that colour in the ability's mana cost (Drought: a Sacrifice-a-Swamp cost for each {B}); no such symbol, no extra cost.
    AbilityExtraCost {
        filter: FilterBox,
        cost: Vec<Cost>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        per_symbol: Option<Color>,
    },
    /// Spells matching `filter` cast by players matching `whose` cost the additional `cost` parts (Sacrifice parts are repeated once for each `per_symbol` mana symbol in the spell's mana cost when given: Drought; no such symbol, no extra cost).
    SpellExtraCost {
        filter: FilterBox,
        #[serde(default)]
        whose: PlayerFilter,
        cost: Vec<Cost>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        per_symbol: Option<Color>,
    },
    /// "You may spend white mana as though it were mana of any color. You may spend other mana only as though it were colorless mana" (Celestial Dawn): applies to the controller's payments.
    SpendWhiteAsAnyOtherAsColorless,
    /// Nonland cards (and spells) the controller owns that aren't on the battlefield are white (Celestial Dawn; pair with a `SetColors` static for the permanents; `Game.white_nonland`).
    NonlandCardsWhiteOutsideBattlefield,
    /// Every card outside the battlefield and every spell is colorless (Mycosynth Lattice); use a `SetColors` static for the permanents.
    ColorlessEverywhere,
    /// Triggered abilities (and delayed triggered abilities) the controller controls can't cause that player to sacrifice or exile creature tokens they control (The Master, Multiplied).
    TriggersCantSacExileOwnTokens,
    /// Spells matching `filter` (cast by players matching `whose`, from `from` if given, hand by default) can be cast by paying `cost` (a mana string, "{0}" = free) rather than their mana cost
    /// (Jodah, Leyline of Mutation, Rooftop Storm, Darksteel Monolith). `once_per_turn`: only the first time each turn this source is used.
    AlternativeManaCost {
        filter: FilterBox,
        #[serde(default)]
        whose: PlayerFilter,
        cost: String,
        #[serde(default, skip_serializing_if = "is_false")]
        once_per_turn: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<Zone>,
        /// Also pay life equal to the spell's mana value (use `cost: "{0}"`): "pay life equal to its mana value rather than pay its mana cost".
        #[serde(default, skip_serializing_if = "is_false")]
        pay_life_mv: bool,
        /// Further (non-mana) costs paid along with `cost`: "rather than pay the mana cost, discard a card" (Dream Halls: cost "{0}", extra [Discard]); "by paying 3 life in addition" (Noctis, from Graveyard).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        extra: Vec<Cost>,
        /// The spell cast this way is exiled instead of going to the graveyard when it leaves the stack (flashback-style grants: Katilda and Lier, Backdraft Hellkite). `cost: "own"` = the card's own mana cost.
        #[serde(default, skip_serializing_if = "is_false")]
        exile_after: bool,
        /// Prowl: only while a creature you controlled that shares a creature type with the spell dealt combat damage to a player this turn (Hunting Velociraptor).
        #[serde(default, skip_serializing_if = "is_false")]
        prowl: bool,
    },
    /// "If you would proliferate, proliferate twice instead" (Tekuthal, Inquiry Dominus); each such permanent you control doubles the number of proliferate rounds.
    ProliferateTwice,
    /// This planeswalker / battle can't be attacked (put a condition on the static: The Aetherspark while attached to a creature).
    CantBeAttacked,
    /// Players can't do something.
    CantCast { who: PlayerFilter },
    /// The matching players can't activate abilities that aren't mana abilities or loyalty abilities, from any zone (Overwhelming Splendor).
    PlayerCantActivate { who: PlayerFilter },
    /// The matching players can cast spells only at times they could cast a sorcery (Teferi, Mage of Zhalfir; Teferi, Time Raveler).
    CastOnlyAtSorcerySpeed { who: PlayerFilter },
    /// The matching players can't play lands (cast spells and put lands onto the battlefield other ways are unaffected); use a condition for "while ten or more lands".
    /// With `filter`, only lands matching it (relative to the source; `chosen_name` for "lands with the chosen name", Conjurer's Ban) can't be played.
    CantPlayLands { who: PlayerFilter, #[serde(default, skip_serializing_if = "Option::is_none")] filter: Option<FilterBox> },
    /// The matching players can't search libraries (Mindlock Orb: everyone; Ashiok: your opponents): their SearchLibrary effects find nothing.
    CantSearchLibraries { who: PlayerFilter },
    /// The matching players skip their untap steps (Stasis): no untapping and no phasing.
    SkipUntapStep { who: PlayerFilter },
    /// Spells and abilities your opponents control can't cause you (the controller) to discard cards or sacrifice permanents (Tamiyo, Collector of Tales).
    OpponentsCantForceDiscardSacrifice,
    /// Spells and abilities your opponents control can't cause you to sacrifice permanents (Tajuru Preserver); discards and your own costs are unaffected.
    OpponentsCantForceSacrifice,
    /// Loyalty abilities of the planeswalkers matching `filter`, activated by a player matching `whose`, cost `delta` extra loyalty (an additional [+1]: Carth the Lion).
    /// The cost is paid as printed, then `delta` counters are added; the ability's own loyalty cost is still needed to activate.
    LoyaltyCostChange {
        #[serde(default)]
        filter: FilterBox,
        #[serde(default)]
        whose: PlayerFilter,
        delta: i32,
    },
    /// At most `n` creatures can attack the players matching `defending` (relative to the source's controller) each combat, counted together (Crawlspace: You, 2; Caverns of Despair: Any, 2). Extra attackers are dropped from the end of the declaration.
    MaxAttackers { defending: PlayerFilter, n: u8 },
    /// The matching players must attack with at least one creature each combat if able (Seeker of Slaanesh); if they declare none, the first able attacker is added.
    MustAttackOne { who: PlayerFilter },
    /// If a creature controlled by a matching player attacks, all their creatures must attack if able (War's Toll): after the declaration, every other able creature is added, attacking the same player as the first attacker when it can.
    MustAttackAllIfAny { who: PlayerFilter },
    /// "If a creature with a `kind` counter on it attacks, all creatures with `kind` counters on them attack if able" (Magnetic Web): after the declaration every other able creature of the attacking player with such a counter is added, attacking the same player as the first such attacker when it can.
    MustAttackIfCounteredAttacks { kind: CounterKind },
    /// Stun counters can't be removed from the matching permanents (Fear of Sleep Paralysis): they stay tapped through untap steps and untap effects, and `{Q}` can't be paid.
    StunCountersCantBeRemoved { affects: FilterBox },
    /// Matching permanents and spells are colourless sources of damage (Ghostly Flame): damage-source colour checks (protection from a colour, colourless, monocoloured, multicoloured) see no colour; targeting, blocking and enchanting still see the real colours.
    ColorlessDamageSources { affects: FilterBox },
    /// "If a creature you control attacks, this creature also attacks if able" (Ekundu Cyclops): applied to the attack declaration once at least one creature attacks.
    AttacksIfAnyAttacks,
    /// "No more than N creatures can block each combat" (Dueling Grounds, Caverns of Despair, Silent Arbiter): a combat-wide cap on blocking creatures; later blockers are dropped.
    MaxBlockersTotal { n: u8 },
    /// "The attacking player chooses how each creature blocks each combat" (Invasion Plans): the active player answers the block declaration for every defending player.
    AttackerChoosesBlocks,
    /// Matching spells can't be cast by the players (`who`): "spells with the chosen name can't be cast".
    /// `caster_relative`: the filter is evaluated as if the casting player controlled the source, so "You" in an `Amount` inside it
    /// (`mana_value_vs`) means the caster (Lavinia, Azorius Renegade: noncreature spells with mana value above the lands the caster controls).
    CantCastCards { who: PlayerFilter, filter: FilterBox, #[serde(default, skip_serializing_if = "std::ops::Not::not")] caster_relative: bool },
    CantAttack { affects: FilterBox },
    /// While the source is untapped, attacking doesn't cause creatures its controller controls to tap (Johan; granted with `GrantStatic` `dynamic` until end of combat).
    AttackersDontTap,
    /// Affected creatures count as having `bonus` more power when tapped for Crew / Saddle costs.
    /// `crew_only`: counts for Crew but not Saddle ("crews Vehicles as though its power were N greater": Reckoner Bankbuster's Pilot).
    CrewPowerBonus { affects: FilterBox, bonus: i32, #[serde(default)] crew_only: bool },
    /// Affected creatures tap for Crew / Saddle using their toughness rather than their power (Giant Ox, Interface Ace).
    CrewUsesToughness { affects: FilterBox },
    /// Affected creatures count as having three times their power when tapped for Crew or Station (Sulu, Heroic Helmsman).
    CrewStationTriple { affects: FilterBox },
    /// This permanent can't be equipped: Equipment can't be attached to it and falls off (Goblin Brawler).
    CantBeEquipped,
    /// On an Aura: the permanent it enchants can't be enchanted by other Auras (Consecrate Land): other Auras can't target it and ones already attached go to the graveyard.
    CantBeEnchantedByOthers,
    /// "If <who> would begin an extra turn, that player skips that turn instead" (Stranglehold, Gerrard's Hourglass Pendant).
    SkipExtraTurns { who: PlayerFilter },
    /// Matching blockers can block attackers matching `attacker` as though they had reach (Dragon Hunter: Dragons).
    BlockAsReachVs { affects: FilterBox, attacker: FilterBox },
    /// All damage is dealt as though its source had wither (Everlasting Torment).
    AllDamageWither,
    /// Replace only the affected permanents' subtypes of one group (creature types or land types) with `subtypes`, keeping card types and the other subtypes (Ichthyomorphosis: creature types := Fish; Harbinger of the Seas: land types := Island; empty = lose them).
    SetSubtypesOfKind { affects: FilterBox, kind: SubtypeKind, subtypes: Vec<String> },
    /// Mana abilities of this permanent cost an additional `n` life to activate (Thran Portal).
    ManaAbilityLifeCost { n: i32 },
    /// The affected permanents' creature types become those of the last creature card exiled with the source, plus `keep` ("it's still a Shapeshifter": Duplicant). Nothing changes while no creature card is exiled with it.
    CreatureTypesOfLastExiled { affects: FilterBox, #[serde(default)] keep: Vec<String> },
    /// Tapped creatures matching `affects` can block as though they were untapped (Masako the Humorless).
    CanBlockWhileTapped { affects: FilterBox },
    /// Matching Vehicles have crew `n` instead of their printed crew number (Kotori, Pilot Prodigy: "Vehicles you control have crew 2").
    CrewCostOverride { affects: FilterBox, n: i32 },
    /// Matching creatures can't attack unless another attacking creature matching `other` also attacks (Scarred Puma).
    CantAttackUnlessOther { affects: FilterBox, other: FilterBox },
    /// The matching players may activate Equip abilities any time they could cast an instant (Leonin Shikari).
    EquipAtInstantSpeed { who: PlayerFilter },
    /// All affected permanents are the color this source chose ("All nonland permanents are the chosen color"; Shifting Sky).
    SetColorsChosen { affects: FilterBox },
    /// Matching permanents are the color their owner chose before the game (`Ability::PregameChooseColor`); colorless if none was chosen (Clara Oswald).
    SetColorsPregame { affects: FilterBox },
    /// Matching permanents are also the color this source chose (Painter's Servant; pair with `ColorAddedEverywhere` for cards outside the battlefield and spells).
    AddColorsChosen { affects: FilterBox },
    /// Every card that isn't on the battlefield, and every spell, is also the color this source chose (Painter's Servant; `Game.color_all`).
    ColorAddedEverywhere,
    /// Creature spells this source's controller controls and creature cards they own outside the battlefield are also artifacts (Biotransference; pair with an `AddType` static for the battlefield; `Game.artifact_creatures`).
    CreaturesAreArtifactsEverywhere,
    /// Affected creatures assign combat damage equal to their toughness rather than their power.
    CombatDamageByToughness { affects: FilterBox },
    /// "While you're searching your library, you may cast this card from your library" (Panglacial Wurm); read by `Effect::SearchLibrary`.
    CastFromLibraryWhileSearching,
    /// Affected creatures with toughness greater than power assign combat damage equal to their toughness (Tapestry Warden).
    CombatDamageByToughnessIfGreater { affects: FilterBox },
    /// Affected creatures with toughness greater than power station using their toughness (Tapestry Warden; spacecraft read `Amount::StationValue`).
    StationByToughness { affects: FilterBox },
    /// Affected creatures with negative power assign combat damage as though their power were positive (Loot, the Anomaly).
    CombatDamageAbsPower { affects: FilterBox },
    /// Lethal damage dealt to the affected creatures is determined by their power rather than their toughness (Zilortha, Strength Incarnate; a creature with power 0 or less is destroyed by any damage).
    LethalByPower { affects: FilterBox },
    /// Affected creatures have landwalk of the land type this source chose (`EntersChoose` `LandType`; Traveler's Cloak).
    LandwalkChosen { affects: FilterBox },
    /// Affected creatures assign no combat damage (grant it with a timed `GrantStatic`: Stromgald Spy).
    AssignNoCombatDamage { affects: FilterBox },
    /// Affected creatures with defender can attack as though they didn't have defender.
    AttackAsThoughNoDefender { affects: FilterBox },
    /// Affected creatures with defender can attack players who attacked their controller during their last turn as though they didn't have defender (Weathered Sentinels).
    AttackAsThoughNoDefenderVsAttackers { affects: FilterBox },
    /// Affected creatures may assign their combat damage as though they weren't blocked (all of it to the player / planeswalker they attack). The damage-assignment question lists each blocker with 0 lethal damage and the defender as a trample target.
    AssignCombatDamageAsUnblocked { affects: FilterBox },
    /// "You may assign this creature's combat damage divided as you choose among defending player and/or any number of creatures they control" (Butcher Orgg): on the card itself, unblocked case.
    DivideCombatDamageAmongDefenders,
    /// "Rather than the attacking player, you assign the combat damage of each creature attacking you. You can divide that creature's combat damage as you choose among any of the creatures blocking it" (Defensive Formation): the controller of this permanent, as a defending player, makes the damage assignment of blocked attackers (no lethal-damage requirement, no trample).
    DefenderAssignsCombatDamage,
    /// "Creatures you control can boast twice during each of your turns rather than once" (Birgi, God of Storytelling): boast abilities of the controller's creatures can be activated twice a turn.
    BoastTwice,
    /// "Trample over planeswalkers" on the card itself: combat damage it deals to a planeswalker it attacks beyond the planeswalker's loyalty goes to that planeswalker's controller (Thrasta, Tempest's Roar).
    TrampleOverPlaneswalkers,
    /// "Can't be blocked by more than N creature(s)" (the affected attackers).
    MaxBlockers { affects: FilterBox, n: u8 },
    /// "No more than N creatures can attack this planeswalker each combat" for the matching permanents (Tomik, Orzhov Lawmage): later attackers declared against it are dropped.
    MaxAttackersOnThis { affects: FilterBox, n: u8 },
    /// "can't be blocked except by N or more creatures": a block with fewer than `n` blockers on the creature is not allowed.
    MinBlockers { affects: FilterBox, n: u8 },
    /// "Can't be blocked unless all creatures defending player controls block it."
    CantBeBlockedUnlessAllBlock { affects: FilterBox },
    /// "Can't be destroyed by lethal damage unless lethal damage dealt by a single source is marked on it" (Ogre Enforcer). Deathtouch still destroys.
    LethalNeedsSingleSource,
    /// "Can't attack unless defending player controls an Island" (`if_controls: false`) / "can't attack if defending player controls an untapped
    /// land" (`if_controls: true`): `defender_controls` is matched against the permanents the attacked player (or planeswalker's controller) controls.
    /// The affected creatures can't attack their owner or planeswalkers/battles their owner controls (Alexios, Xantcha).
    CantAttackOwner { affects: FilterBox },
    /// The affected creatures can't attack this permanent's controller (nor, unless `players_only`, planeswalkers/battles that player controls).
    CantAttackController {
        affects: FilterBox,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        players_only: bool,
    },
    CantAttackPlayer {
        affects: FilterBox,
        defender_controls: FilterBox,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        if_controls: bool,
        /// The restriction applies only to attacking the player, not that player's planeswalkers / battles ("can't attack you").
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        players_only: bool,
    },
    CantBlock { affects: FilterBox },
    /// The affected creatures can't block attackers matching `attacker` ("can't block non-Spirit creatures"; use `not_subtypes` etc. on the attacker filter).
    CantBlockAttackers { affects: FilterBox, attacker: FilterBox },
    /// "Can block an additional creature each combat" (`extra: 1`); `extra: 0` means any number of creatures. Its damage is dealt lethal-first in declaration order.
    CanBlockExtra {
        affects: FilterBox,
        #[serde(default)]
        extra: u8,
    },
    /// "Can block an additional creature each combat for each ..." (Kemba's Legion): the number of extra blocks is an amount evaluated continuously.
    CanBlockExtraAmount { affects: FilterBox, extra: Amount },
    CantGainLife { who: PlayerFilter },
    MustAttack { affects: FilterBox },
    /// The affected creatures attack this source (a planeswalker) each combat if able (Gideon Jura; use via `GrantStatic dynamic`).
    MustAttackSource { affects: FilterBox },
    /// The affected creatures attack the source's controller each combat if able (Trove of Temptation); `enchanted_controller`: the controller of the permanent the source is attached to (Public Enemy).
    MustAttackController {
        affects: FilterBox,
        #[serde(default, skip_serializing_if = "is_false")]
        enchanted_controller: bool,
    },
    /// The affected creatures can't attack unless another creature also attacks ("can't attack alone"); combine with a static condition for "unless ...".
    CantAttackAlone { affects: FilterBox },
    /// The matching creatures can attack only alone: if one of them is declared with other attackers, only it attacks (Master of Cruelties).
    CanOnlyAttackAlone { affects: FilterBox },
    /// The affected creatures can't attack unless at least `n` other creatures also attack (Orcish Conscripts).
    CantAttackUnlessOthers { affects: FilterBox, n: u8 },
    /// The affected creatures can't block unless at least `n` other creatures also block (Orcish Conscripts).
    CantBlockUnlessOthers { affects: FilterBox, n: u8 },
    /// The affected creatures can't attack a player they have already attacked this turn (Bloodthirster, Port Razer); attacking that player's planeswalkers is still allowed.
    CantAttackPlayerAgain { affects: FilterBox },
    /// Each player may attack only the nearest opponent in the direction this source chose (`ChooseKind::LeftRight`) and planeswalkers / battles that opponent controls (Pramikon, Sky Rampart).
    AttackOnlyNearestInDirection,
    /// The affected creatures can't attack unless a creature with greater power also attacks (Okk).
    CantAttackUnlessGreaterPower { affects: FilterBox },
    /// The affected creatures can't block unless a creature with greater power also blocks (Okk).
    CantBlockUnlessGreaterPower { affects: FilterBox },
    /// The affected creatures can't block unless another creature also blocks ("can't block alone").
    CantBlockAlone { affects: FilterBox },
    /// The affected creatures can block creatures with shadow (as though they had / the attackers lacked shadow). They can still block creatures without shadow.
    CanBlockShadow { affects: FilterBox },
    /// The affected creatures can block creatures with landwalk as though those creatures didn't have landwalk (Street Savvy).
    CanBlockLandwalk { affects: FilterBox },
    /// The affected creatures can be blocked as though they didn't have landwalk ("Islandwalk creatures can be blocked as though they didn't have islandwalk").
    IgnoreLandwalk {
        affects: FilterBox,
        /// Only this landwalk keyword is ignored (Crevasse: Mountainwalk); without it all of the creature's landwalk is.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        only: Option<Keyword>,
    },
    /// The matching permanents can't be sacrificed (neither as a cost nor by an effect).
    CantBeSacrificed { affects: FilterBox },
    /// The matching permanents can't be returned to hand to pay a "beam me up" cost (Marooned: "can't be beamed up"); filters with `beamable` skip them.
    CantBeBeamedUp { affects: FilterBox },
    /// The matching permanents can't phase out (Spatial Binding, via GrantStatic until your next upkeep).
    CantPhaseOut { affects: FilterBox },
    /// The matching planeswalkers aren't put into the graveyard for having 0 loyalty (Sanctum Lurker).
    ZeroLoyaltyStays { affects: FilterBox },
    /// The matching permanents can't become suspected (Airtight Alibi).
    CantBecomeSuspected { affects: FilterBox },
    /// Permanents can't phase in (Disciple of Caelus Nin).
    CantPhaseIn,
    /// Matching cards in `from` can't be put onto the battlefield (Kunoros, Hound of Athreos: creature cards in graveyards).
    CantEnterBattlefieldFrom { from: Zone, filter: FilterBox },
    /// Matching cards can't enter the battlefield from any zone (Worms of the Earth: lands). Tokens are not covered.
    CantEnterBattlefield { filter: FilterBox },
    /// When damage is dealt to this creature, the damage already marked on it is healed first (Wolverine, Fierce Fighter).
    HealOnDamaged,
    /// Information only (no rules effect in the engine; clients may show it): the matching players play with their hand revealed (Telepathy, Seer's Vision).
    HandsRevealed { who: PlayerFilter },
    /// Information only: the controller may look at the top card of their library any time (Radar Sense, Glarb) / plays with it revealed (`all`: every player's).
    TopCardVisible {
        #[serde(default)]
        all: bool,
        /// Every player's library, not only the controller's ("Players play with the top card of their libraries revealed": Wizened Snitches).
        #[serde(default, skip_serializing_if = "is_false")]
        each_library: bool,
    },
    /// Continuously goaded (while the static applies): attacks each combat if able and attacks a player other than the static's controller if able.
    Goaded { affects: FilterBox },
    /// The creatures block each combat if able.
    MustBlock {
        affects: FilterBox,
        /// Only attackers matching this (relative to the static's source): "all creatures able to block this creature do so" = `{"this":true}`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attacker: Option<FilterBox>,
    },
    /// The creatures must be blocked if able (at least one creature that can block them does).
    MustBeBlocked { affects: FilterBox },
    /// The creatures can't be blocked while `cond` holds, judged with the defending player (the blocker's controller) as "you" ("can't be blocked as long as defending player controls an enchantment").
    CantBeBlockedWhileDefender { affects: FilterBox, cond: Condition },
    /// The creatures can't attack a player (or planeswalker of a player) for whom `cond` holds, judged with that defending player as "you" ("can't attack unless defending player is poisoned" = `Not` poisoned).
    CantAttackWhileDefender { affects: FilterBox, cond: Condition },
    /// The creatures can't block an attacker whose controller satisfies `cond`, judged with the attacking player as "you" ("can't block unless you control more creatures than attacking player" = `Not` of that).
    CantBlockWhileAttacker { affects: FilterBox, cond: Condition },
    /// The creatures must be blocked by at least `n` creatures matching `by` if able ("must be blocked by a Dalek if able", "by two or more creatures if able").
    MustBeBlockedBy {
        affects: FilterBox,
        #[serde(default)]
        by: FilterBox,
        #[serde(default = "one_u8")]
        n: u8,
    },
    /// The matching cards (on the battlefield, spells on the stack, and cards in other zones) are also of the creature type this permanent chose as it entered (Arcane Adaptation, Leyline of Transformation, Adaptive Automaton).
    /// `zone`: only cards currently in that zone (Ashes of the Fallen: Graveyard); default every zone.
    /// `fixed`: add that fixed subtype to matching cards that are NOT on the battlefield (hand, graveyard, exile, library; `zone` still narrows) instead of the chosen type (Dune Chanter: "land cards you own that aren't on the battlefield are Deserts"); pair with an `AddType` for the battlefield.
    AddChosenSubtype {
        affects: FilterBox,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        zone: Option<Zone>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fixed: Option<String>,
        /// The matching cards' creature types BECOME the chosen type (their other creature types are lost) instead of adding to them (Conspiracy).
        #[serde(default, skip_serializing_if = "is_false")]
        replace: bool,
    },
    /// The matching permanents are also of the SECOND basic land type this permanent chose (`ChooseKind::SecondBasicLandType`). `affects` picks them, usually with `chosen_type` for the first type (Illusionary Terrain: basic lands of the first chosen type are the second).
    AddSecondChosenType { affects: FilterBox },
    /// Creatures matching the filter can't be blocked except by ...
    CantBeBlockedBy { affects: FilterBox, by: FilterBox },
    /// The affected creatures can't be blocked by creatures with greater power than their own (Tadeas, Juniper Ascendant; grant with `GrantStatic`).
    CantBeBlockedByGreaterPower { affects: FilterBox },
    MaxHandSize { n: i32 },
    /// The maximum hand size of the matching players is `n` (Cursed Rack: `ChosenPlayer`). Judged with `player_filter_src`.
    MaxHandSizeFor { who: PlayerFilter, n: i32 },
    NoMaxHandSize,
    /// Matching players don't lose unspent mana as steps and phases end (Upwelling: all mana; Omnath/Fangorn: `colors` [G]; Leyline Tyrant: [R]). Empty `colors` = every type.
    ManaPersists {
        #[serde(default)]
        who: PlayerFilter,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        colors: Vec<crate::mana::ManaType>,
    },
    /// This permanent can't have more than `max` counters of `kind` on it; counters put on it beyond that are not added (Rasputin Dreamweaver).
    MaxCounters { kind: CounterKind, max: u32 },
    /// Mana matching players would lose (as steps and phases end, or from a "lose all unspent mana" effect) becomes colorless instead (Horizon Stone).
    ManaBecomesColorless {
        #[serde(default)]
        who: PlayerFilter,
    },
    /// "If you would get one or more poison counters, instead you get one poison counter and you can't get additional poison counters this turn" (Melira, the Living Cure).
    PoisonOnlyOne {
        #[serde(default = "you_filter")]
        who: PlayerFilter,
    },
    /// Mana matching players would lose (as steps and phases end, or from a "lose all unspent mana" effect) becomes black instead (Omnath, Locus of All).
    ManaBecomesBlack {
        #[serde(default)]
        who: PlayerFilter,
    },
    /// Mana matching players would lose (as steps and phases end, or from a "lose all unspent mana" effect) becomes red instead (Ozai, the Phoenix King).
    ManaBecomesRed {
        #[serde(default)]
        who: PlayerFilter,
    },
    /// A matching player losing unspent mana (as a step or phase ends, or from a "lose all unspent mana" effect) loses that much life (Yurlok of Scorch Thrash).
    ManaLossCostsLife {
        #[serde(default)]
        who: PlayerFilter,
    },
    /// "You may play lands / cast spells from your graveyard" (Crucible of Worlds, Muldrotha) or "from the top of your library" (Mystic Forge, Oracle of Mul Daya).
    /// The `filter` limits which cards; the controller of this permanent may play them as if from hand.
    MayPlayFrom {
        zone: Zone,
        filter: FilterBox,
        /// The permission belongs to the active player (whoever's turn it is) instead of this permanent's controller; the filter is judged from that player's side ("a spell from among the cards they don't own exiled with Ian Malcolm").
        #[serde(default, skip_serializing_if = "is_false")]
        active_player: bool,
        /// Every player has the permission for the cards THEY exiled (`Card.exiled_by`, set with `Effect::SetExiledBy`), not just this permanent's controller (Shared Fate).
        #[serde(default, skip_serializing_if = "is_false")]
        exiler: bool,
        /// This permission can be used only once each turn (by this permanent): "once during each of your turns, you may cast a ... from your graveyard".
        #[serde(default, skip_serializing_if = "is_false")]
        once_per_turn: bool,
        /// A permanent spell cast from the graveyard through this permission enters with a finality counter (Leonardo, Sewer Samurai).
        #[serde(default, skip_serializing_if = "is_false")]
        finality: bool,
        /// Once each turn per permanent type (Muldrotha, the Gravetide): each play/cast through this permission uses up one permanent type of the card
        /// (a multi-type card uses the first unused one in the order land, artifact, enchantment, planeswalker, battle, creature).
        #[serde(default, skip_serializing_if = "is_false")]
        per_type: bool,
        /// Once each turn per kind: one land AND one spell (Coram, the Undertaker: "you may play a land and cast a spell from among ...").
        #[serde(default, skip_serializing_if = "is_false")]
        per_kind: bool,
        /// With `zone: Graveyard`: the cards in every player's graveyard are offered, not just your own (Coram, the Undertaker).
        #[serde(default, skip_serializing_if = "is_false")]
        any_graveyard: bool,
        /// With `zone: Library`: the top card of every player's library is offered, not just your own (Windriddle Palaces).
        #[serde(default, skip_serializing_if = "is_false")]
        any_library: bool,
        /// A permanent played or cast through this permission gains "When this permanent is put into a graveyard from the battlefield, exile it and you gain N life"
        /// (Serra Paragon: 2). Pair with `once_per_turn`, which is also used up by playing a land through the permission.
        #[serde(default, skip_serializing_if = "is_zero_u8")]
        leave_exile_gain: u8,
        /// A permanent played or cast through this permission gains "If this permanent would leave the battlefield, exile it instead of putting it anywhere else" (The Eighth Doctor; pair with `once_per_turn`).
        #[serde(default, skip_serializing_if = "is_false")]
        leave_exile: bool,
        /// Casting through this permission costs this much life in addition to the other costs (Festival of Embers: 1). Not applied to flashback or other alternative-cost casts.
        #[serde(default, skip_serializing_if = "is_zero_u8")]
        extra_life: u8,
        /// Casting through this permission requires sacrificing a creature in addition to the other costs, and a spell cast this way is exiled instead of going to the graveyard (Maestros Ascendancy).
        #[serde(default, skip_serializing_if = "is_false")]
        sac_exile: bool,
        /// A spell cast through this permission is exiled instead of going to the graveyard when it would (Kess, Dissident Mage; flashback and other alternative-cost casts are not affected).
        #[serde(default, skip_serializing_if = "is_false")]
        exile_after: bool,
        /// A spell cast from exile through this permission is cast without paying its mana cost (Maralen, Fae Ascendant).
        #[serde(default, skip_serializing_if = "is_false")]
        free: bool,
        /// Extra costs paid in addition to the other costs when casting through this permission (Falco Spara: remove a counter from a creature you control). Not applied to alternative-cost casts.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        extra_cost: Vec<Cost>,
        /// "When they do, exile the top card of their library": whoever plays a land / casts a spell through this permission exiles the top card of their own library, tagged as exiled with this permanent (Share the Spoils).
        #[serde(default, skip_serializing_if = "is_false")]
        exile_top: bool,
        /// A card played or cast through this permission is marked as played via this source, so the source's own triggers can
        /// tell (`Filter.played_via_this` on `SpellCast` / `LandPlayed`: The Matrix of Time, "whenever you play a land or cast a spell
        /// from among cards exiled with ~").
        #[serde(default, skip_serializing_if = "is_false")]
        mark_played: bool,
    },
    /// While a matching player searches their library, this permanent's controller makes the search's choices, and the found cards are exiled instead;
    /// the controller may play them for as long as they remain exiled, spending mana as though it were any color (Opposition Agent).
    ControlSearches { who: PlayerFilter },
    /// Matching players can't venture into the dungeon more than once each turn (Keen-Eared Sentry: `who` Opponent).
    VentureLimit { who: PlayerFilter },
    /// "You have hexproof" (`who` relative to the controller): opponents' spells and abilities can't target those players.
    PlayerHexproof { who: PlayerFilter },
    /// "You have hexproof from <quality>" (Veil of Summer, granted with `GrantStatic` to players): opponents' spells and abilities from sources with that quality can't target those players. Only honoured as a granted effect.
    PlayerHexproofFrom { who: PlayerFilter, from: ProtectionFrom },
    /// "You have shroud" (Ivory Mask): no spell or ability, even your own, can target those players.
    PlayerShroud { who: PlayerFilter },
    /// "You have protection from ..." (Absolute Virtue: `Opponents`): the players can't be targeted or dealt damage by matching sources.
    PlayerProtection { who: PlayerFilter, from: ProtectionFrom },
    /// "Players have no maximum hand size" / "your opponents have no maximum hand size".
    HandSizeUnlimited { who: PlayerFilter },
    /// Matching players can't lose the game (Platinum Angel: `You` for its controller).
    CantLoseGame { who: PlayerFilter },
    /// If damage dealt to a matching planeswalker would remove all its loyalty counters, all but one are removed instead (Deification: use `condition` "you control a creature").
    LoyaltyDamageFloor { affects: FilterBox },
    /// Matching players can't win the game (Courageous Resolve: opponents; Platinum Angel: "your opponents can't win"). The game goes on while the last player left can't win.
    CantWinGame { who: PlayerFilter },
    /// "Damage that would reduce your life total to less than 1 reduces it to 1 instead" (Angel's Grace; granted to players until end of turn).
    DamageLifeFloor { who: PlayerFilter },
    /// Damage that would reduce the player's life total to less than `floor` reduces it to `floor` instead (Elderscale Wurm); the damage is still dealt in full.
    DamageLifeFloorAt { who: PlayerFilter, floor: i32 },
    /// "If your life total would be reduced to 0 or less, instead transform this creature and your life total becomes 3. Then if this creature didn't transform this way, you lose the game" (Enduring Angel). Applied when state-based actions find the life total at 0 or less.
    LifeZeroTransform,
    /// "Damage doesn't cause you to lose life" (Archon of Coronation while monarch); other life loss still happens.
    DamageNoLifeLoss { who: PlayerFilter },
    /// "You don't lose the game for having 0 or less life" (other ways of losing still work).
    NoLoseAtZeroLife { who: PlayerFilter },
    /// "You may cast <matching spells> as though they had flash."
    /// `once` (in `GrantStatic` to a player): used up by the first matching spell cast.
    CastAsThoughFlash {
        filter: FilterBox,
        whose: PlayerFilter,
        #[serde(default, skip_serializing_if = "is_false")]
        once: bool,
        /// Only if the spell targets a permanent matching this filter (Flash Photography: "if it targets a permanent you control"). Cast at a time it could not
        /// otherwise be cast, the spell's first target slot is limited to such permanents. Only for a card's own ability (`filter.this`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        if_targets: Option<Box<Filter>>,
    },
    /// Matching players can't pay life (Platinum Emperion).
    CantPayLife { who: PlayerFilter },
    /// Matching players who would pay life (while their library has at least that many cards) exile that many cards from the top of their library instead (Ashiok, Wicked Manipulator).
    PayLifeExilesLibrary { who: PlayerFilter },
    /// Players can't sacrifice creatures to pay costs (Angel of Jubilation).
    CantSacrificeCreaturesForCosts,
    /// Nonland permanents can't be sacrificed to pay costs of spells or abilities (Yasharn, Implacable Earth); effects that sacrifice are unaffected.
    CantSacrificeNonlandForCosts,
    /// The legend rule doesn't apply to matching permanents (Mirror Box, Mirror Gallery, Spider-Verse).
    IgnoreLegendRule { affects: FilterBox },
    /// Matching permanents don't untap during their controller's untap step (Intruder Alarm, Dehydration, Grimgrin).
    DoesntUntap { filter: FilterBox },
    /// Granted via `GrantStatic` for a duration: the permanent can't become tapped (every tap path checks `Game::can_become_tapped`); with `except_attacking` it still taps when declared as an attacker (Ood Sphere). `affects` is ignored.
    CantBecomeTapped { #[serde(default)] affects: FilterBox, #[serde(default, skip_serializing_if = "std::ops::Not::not")] except_attacking: bool },
    /// Mana produced by matching permanents' mana abilities is of type `to` instead (Contamination: B; Ritual of Subdual: C; Pale Moon). With `from` set only that type is replaced. Granted via GrantStatic the filter is ignored.
    ManaTypeReplace { filter: FilterBox, to: crate::mana::ManaType, #[serde(default)] from: Option<crate::mana::ManaType>, /// "instead of any other type and amount" (Contamination): a tap produces exactly one mana of `to`.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")] single: bool,
        /// "one mana of a color of your choice instead of any other type and amount" (Harvest Mage): a tap produces exactly one mana of the color the tapping player picks (`to` is ignored; the payment planner treats the land as a source of any color).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")] any_color: bool,
        /// `to` is ignored: the color chosen on the source by its latest `ChooseColor` is used (Hall of Gemstone). Only honoured as a granted dynamic static.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")] to_chosen: bool },
    /// "If a land is tapped for two or more mana, it produces {C} instead of any other type and amount" (Damping Sphere): affects every player's lands.
    LandManaCappedToColorless,
    /// Your devotion to each color and each combination of colors is increased by `n` (Altar of the Pantheon).
    DevotionBonus { n: i32 },
    /// Matching permanents (you control) also untap during each other player's untap step (Murkfiend Liege, Ivorytusk Fortress, Ohabi Caleria).
    UntapDuringOthers { filter: FilterBox },
    /// A matching permanent that would untap during its controller's untap step has all `kind` counters removed instead (Freyalise's Winds).
    UntapRemoveCounters { filter: FilterBox, kind: CounterKind },
    /// A matching permanent that would untap during its controller's untap step instead gets `n` `kind` counters and stays tapped (Edge of Malacol).
    UntapAddCounters { filter: FilterBox, kind: CounterKind, n: u8 },
    /// A matching permanent that would untap during its controller's untap step instead has one `kind` counter removed, and untaps only if a counter was removed (Bewitching Leechcraft; use `attached_by_source` on an Aura).
    UntapPayCounter { filter: FilterBox, kind: CounterKind },
    /// "Damage can't be prevented" (Leyline of Punishment): prevention effects and protection do nothing while this is on the battlefield.
    DamageCantBePrevented,
    /// "Combat damage dealt by creatures you control can't be prevented" / "damage dealt by sources you control ...": prevention does nothing for damage from sources matching `filter` (read with this permanent as the source).
    SourceDamageCantBePrevented { filter: FilterBox, #[serde(default)] combat_only: bool },
    /// "As you cascade, you may put a land card from among the exiled cards onto the battlefield tapped" (Averna, the Chaos Bloom): read by `RevealUntil.cascade_land`.
    CascadeLandToBattlefield,
    /// "Skip your draw step" (Necropotence, Dragon Appeasement): the matching players skip their draw steps.
    SkipDrawStep { who: PlayerFilter },
    /// "You may look at an additional `n` cards each time you surveil" (Enhanced Surveillance).
    SurveilExtraLook { who: PlayerFilter, n: i32 },
    /// "You gain life rather than lose life from radiation" (Strong, the Brutish Thespian).
    RadiationGainsLife { who: PlayerFilter },
    /// "If you would begin your draw step, you may skip that step instead. If you do, you gain `gain` life" (Fasting).
    MaySkipDrawStep { who: PlayerFilter, gain: i32 },
    /// "Players skip their upkeep steps" (Eon Hub): the matching players skip their upkeep.
    SkipUpkeep { who: PlayerFilter },
    /// "You can't play lands from your hand" (Experimental Frenzy): only land plays from the hand are stopped.
    CantPlayLandsFromHand { who: PlayerFilter },
    /// "You may activate loyalty abilities of planeswalkers you control twice each turn rather than only once" (Oath of Teferi).
    LoyaltyTwice,
    /// You may activate loyalty abilities of planeswalkers you control on any player's turn any time you could cast an instant (Teferi's Talent emblem; emblem statics run from the command zone).
    LoyaltyAtInstantSpeed,
    /// Instant and sorcery spells you control have rebound (Cast Through Time; a spell with its own rebound is unchanged).
    GrantRebound,
    /// Instant and sorcery spells (of every player) have rebound; works from a face-up plane (Trail of the Mage-Rings).
    GrantReboundAll,
    /// The affected permanents can't be enchanted: new Auras can't target them; Auras already attached stay (Guardian Beast).
    NoNewAuras { affects: FilterBox },
    /// Players other than the static's controller can't gain control of the affected permanents (Guardian Beast).
    NoControlChangeByOthers { affects: FilterBox },
    /// This permanent can't be enchanted: Auras can't target it and ones attached to it fall off (the Tetravite token).
    CantBeEnchanted,
    /// "During your turn, as long as you haven't activated an exhaust ability this turn, you may activate exhaust abilities as though they haven't been activated" (Elvish Refueler).
    ExhaustReactivate,
    /// "Each power-up ability of permanents you control can be activated an additional time" (Wonder Man, Hollywood Hero): a once-only `power_up` ability may be activated twice.
    PowerUpTwice,
    /// "If an opponent would search a library, that player searches the top `n` cards of that library instead" (Aven Mindcensor): searches by players matching `who` only see the top `n` cards.
    SearchTopOnly { who: PlayerFilter, n: u8 },
    /// "You may choose not to untap this during your untap step": its controller is asked each untap step (the default answer untaps it).
    OptionalUntap { affects: FilterBox },
    /// "If you would flip a coin, instead flip two coins and ignore one" (Krark's Thumb): the controller's flips are won if either coin is (the better result is kept).
    CoinFlipTwice,
    /// "The first time you flip one or more coins each turn, those coins come up heads and you win those flips" (Edgar, King of Figaro): the controller's first coin flip each turn is won.
    FirstCoinFlipHeads,
    /// Matching spells its controller casts have replicate with a replicate cost equal to their mana cost (Ian Chesterton). Offered as a multikicker-like payment
    /// (`KickCount` = times paid) in the cast; the copies are made by a `SpellCast` trigger on the granting permanent (`Repeat` of `KickCount` with `CopySpell`).
    GrantReplicate {
        affects: FilterBox,
        /// The replicate cost as a mana string (Threefold Signal: `{3}`); without it the cost equals the spell's mana cost.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cost: Option<String>,
    },
    /// Matching spells its controller casts have offspring `cost` (Zinnia, Valley's Voice): an optional extra cost, offered like a kicker (`KickCount` = 1 when paid).
    /// The 1/1 token copy is made by an `EntersBattlefield` trigger on the granting permanent whose filter has `granted_offspring: true`.
    GrantOffspring { affects: FilterBox, cost: String },
    /// Matching spells its controller casts have conspire (Wort, the Raidmother): an optional extra cost of tapping two untapped creatures you control that share a color
    /// with the spell, offered like a kicker (`KickCount` = 1 when paid). The copy is made by a `SpellCast` trigger on the granting permanent (`Repeat` of `KickCount` with `CopySpell`).
    GrantConspire { affects: FilterBox },
    /// Matching spells its controller casts have casualty `n` (Anhelo, the Painter): an optional extra cost of sacrificing a creature with power `n` or greater, offered like a kicker
    /// (`KickCount` = 1 when paid). `first_each_turn`: only the first matching spell that player casts each turn. The copy is made by a `SpellCast` trigger on the granting permanent (`Repeat` of `KickCount` with `CopySpell`).
    GrantCasualty {
        affects: FilterBox,
        n: i32,
        #[serde(default, skip_serializing_if = "is_false")]
        first_each_turn: bool,
    },
    /// Matching spells the player casts have convoke / improvise / delve ("the next spell you cast this turn has improvise").
    /// Only meaningful in `GrantStatic` to a player; `once`: used up by the first matching spell cast.
    GrantSpellKeyword {
        filter: FilterBox,
        keyword: Keyword,
        #[serde(default, skip_serializing_if = "is_false")]
        once: bool,
    },
    /// Matching spells can't be countered.
    /// `once` (in `GrantStatic` to a player): applies to the first matching spell cast only.
    CantBeCountered {
        filter: FilterBox,
        #[serde(default, skip_serializing_if = "is_false")]
        once: bool,
        /// Only against counterspells / abilities from a source matching this ("can't be countered by blue or black spells": Autumn's Veil; `GrantStatic` to a player only).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        by: Option<FilterBox>,
    },
    /// Creatures can't attack you (or planeswalkers you control) unless their controller pays this much for each (Propaganda: 2).
    AttackTax {
        generic: i32,
        /// Ghostly Prison: only attacks against the controller, not their planeswalkers.
        #[serde(default)]
        only_player: bool,
        /// Onakke Oathkeeper: only attacks against the controller's planeswalkers / battles, not the player.
        #[serde(default, skip_serializing_if = "is_false")]
        only_planeswalkers: bool,
        /// A computed tax instead of `generic` ("unless pays {X} for each, X = the cards in your hand / your life ..."), evaluated for the static's controller.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        amount: Option<Amount>,
    },
    /// Matching creatures can't attack unless their controller pays this cost (checked per attacker as attackers are declared): "enchanted creature can't attack unless its controller pays {3}", Brainwash, Norn's Annex (use ManaAmount for computed amounts).
    AttackCost {
        affects: FilterBox,
        cost: Vec<Cost>,
        /// Only when attacking a player matching this (relative to the static's controller): "creatures can't attack YOU unless ..." is `You`.
        #[serde(default)]
        against: PlayerFilter,
    },
    /// Matching creatures can't block unless their controller pays this cost (per blocker).
    BlockCost {
        affects: FilterBox,
        cost: Vec<Cost>,
        /// Only when blocking an attacker matching this filter (relative to the static's controller): Heat Wave "can't block creatures you control unless ...".
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attacker: Option<FilterBox>,
    },
    /// Creatures you control can attack as though they had haste etc. are expressed as GrantKeyword.
    /// You control the permanent (Control Magic-style).
    ControlEnchanted,
    /// The current monarch controls the enchanted permanent (Fealty to the Realm); nobody's control changes while there is no monarch.
    ControlEnchantedByMonarch,
    /// Your opponents can't gain life / cast spells during your turn, etc.
    OpponentsCantCastDuringYourTurn,
    /// During combat (beginning of combat through end of combat) players can't cast spells or activate abilities that aren't mana abilities (turning face up is a special action and still allowed; Yuriko, Blade of the Mighty).
    NoSpellsOrAbilitiesInCombat,
    /// During combat (beginning of combat through end of combat) players can't cast spells; abilities are unaffected (Basandra, Battle Seraph).
    NoSpellsInCombat,
    /// If a spell or ability you control would counter a spell, instead exile that spell and you may play that card without paying its mana cost (Guile; the offer is made by `Effect::CounterSpell`).
    CounterExilePlay,
    /// You may pay {0} rather than the equip cost of the first equip ability you activate during each of your turns (Forge Anew; only the mana part of an equip cost is replaced).
    EquipFirstFree,
    /// Like `EquipFirstFree` but on any turn, not only the controller's own ("the first equip ability you activate each turn": Bruenor Battlehammer).
    EquipFirstFreeEachTurn,
    /// You may play additional lands.
    AdditionalLandDrop {
        n: i32,
        /// Who gets the extra land drops (default: the permanent's controller; Rites of Flourishing / Ghirapur Orrery: Any).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        who: Option<PlayerFilter>,
    },
    /// Matching players' maximum hand size changes by `delta` (Gnat Miser: opponents -1; Jin-Gitaxias, Core Augur: opponents -7).
    MaxHandSizeDelta { who: PlayerFilter, delta: i32 },
    /// Colored mana of this kind is produced by lands you control (Mana Reflection ...), kept coarse.
    ManaMultiplier {
        factor: i32,
        /// Only mana from basic lands is multiplied (Virtue of Strength).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        basic_land: bool,
        /// Applies to mana every player makes, not just its controller's (Lavaleaper).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        all_players: bool,
    },
}

fn you_filter() -> PlayerFilter {
    PlayerFilter::You
}
fn zero_amount() -> Amount {
    Amount::Fixed { n: 0 }
}
fn one_amount() -> Amount {
    Amount::Fixed { n: 1 }
}
fn you() -> Sel {
    Sel::You
}
fn to_hand() -> Zone {
    Zone::Hand
}
fn yes() -> bool {
    true
}
fn is_true(b: &bool) -> bool {
    *b
}

#[allow(dead_code)]
fn _unused(_: &i32) -> bool {
    is_zero(&0)
}

fn is_zero_i32(n: &i32) -> bool {
    *n == 0
}

/// `Effect::SearchLibrary.instead`: found cards matching the filter go to another zone when the condition holds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SearchInstead {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cond: Option<Condition>,
    pub filter: FilterBox,
    pub to: Zone,
}

/// A kind of mana source for `Amount::ManaSpentFromSource`.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Serialize, Deserialize, JsonSchema)]
pub enum ManaSourceKind {
    Treasure,
    Artifact,
    Cave,
    Desert,
    /// A creature tapped for mana (Inga and Esika: "three or more mana from creatures").
    Creature,
}

/// Which kind of word a `ChangeText` effect may replace.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum TextKind {
    Color,
    LandType,
    CreatureType,
}
