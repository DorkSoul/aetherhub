//! Game state: plain data, cheap to clone (the card database is shared behind an `Arc`).

use crate::carddb::CardDb;
use crate::decision::*;
use crate::dsl::*;
use crate::ids::*;
use crate::mana::*;
use crate::types::*;
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use std::sync::Arc;
use ts_rs::TS;

/// Characteristics after continuous effects (cached per permanent; recomputed only when something changes).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Derived {
    /// Power and toughness after layer 7b (before counters and other modifications): the "base" power and toughness of rules text.
    pub base_pt: Option<(i32, i32)>,
    pub types: TypeSet,
    pub supertypes: SmallVec<[Supertype; 2]>,
    pub subtypes: SmallVec<[Sym; 4]>,
    pub colors: ColorSet,
    pub power: i32,
    pub toughness: i32,
    pub kw: KeywordSet,
    /// Rules-effect pseudo-keywords (can't be blocked / can't block / can't attack / must attack / can't be countered)
    /// put on it by resolved spells and abilities: they are no abilities, so "loses all abilities" keeps them.
    pub effect_kw: KeywordSet,
    /// Toxic from further sources that repeats a toxic keyword the permanent already has (toxic adds up).
    pub toxic_extra: u8,
    pub controller: PlayerId,
    pub protections: SmallVec<[ProtectionFrom; 1]>,
    pub hexproof_from: SmallVec<[ProtectionFrom; 1]>,
    pub no_abilities: bool,
    /// Loses ward (Shay Cormac).
    pub no_ward: bool,
    /// With `no_abilities`: its mana abilities still work (Blood Sun).
    pub keep_mana_abilities: bool,
    /// With `no_abilities`: the basic land types give no mana ability (Imprisoned in the Moon).
    pub no_land_mana: bool,
    /// A name that replaces the card's printed name ("except its name is Gogo"); None = the printed name.
    pub name: Option<Sym>,
    /// A copy "except it has no mana cost" (embalm / eternalize tokens): its mana value is 0.
    pub no_mana_cost: bool,
    /// Has the names of all nonlegendary creature cards in addition to its own (Spy Kit).
    pub all_creature_names: bool,
    pub cant_attack: bool,
    pub dmg_by_toughness: bool,
    /// Assigns combat damage equal to toughness when toughness is greater than power (Tapestry Warden).
    pub dmg_by_toughness_gt: bool,
    /// Stations with its toughness when that is greater than its power (Tapestry Warden; `Amount::StationValue`).
    pub station_by_toughness: bool,
    /// Assigns combat damage as though its power were positive (Loot, the Anomaly).
    pub dmg_abs_power: bool,
    /// Lethal damage is its power instead of its toughness (Zilortha, Strength Incarnate).
    pub lethal_by_power: bool,
    /// Landwalk of chosen land types (any land type, not only the basic ones: Traveler's Cloak).
    pub extra_landwalk: SmallVec<[Sym; 1]>,
    /// "assigns no combat damage" (Stromgald Spy).
    pub no_combat_damage: bool,
    pub assign_as_unblocked: bool,
    pub attack_no_defender: bool,
    /// Can attack as though it had no defender, but only players who attacked its controller during their last turn (Weathered Sentinels).
    pub attack_nd_vs_attackers: bool,
    pub cant_block: bool,
    /// How many creatures beyond the first this creature may block (255: any number).
    pub extra_blocks: u8,
    /// At most this many creatures may block it (0 = no limit).
    pub max_blockers: u8,
    /// At most this many creatures may attack it each combat (planeswalkers; 0 = no limit): Tomik, Orzhov Lawmage.
    pub max_attackers: u8,
    pub min_blockers: u8,
    /// Can't be blocked unless every creature the defending player controls blocks it.
    pub block_all: bool,
    pub must_attack: bool,
    pub cant_attack_alone: bool,
    /// Can attack only if no other creature attacks (Master of Cruelties).
    pub attack_only_alone: bool,
    /// Can't attack / block unless at least this many other creatures also attack / block.
    pub attack_needs_others: u8,
    pub block_needs_others: u8,
    pub cant_block_alone: bool,
    pub cant_attack_unless_greater: bool,
    pub cant_block_unless_greater: bool,
    pub cant_be_blocked_greater: bool,
    /// Became a creature through `EnchantedAnimate` this recompute (its P/T is then set in the P/T layer).
    pub animated: bool,
    pub can_block_shadow: bool,
    pub can_block_landwalk: bool,
    /// Landwalk on this attacker doesn't restrict blockers.
    pub ignore_landwalk: u8,
    /// Can't be sacrificed.
    pub cant_be_sacrificed: bool,
    /// Can't be beamed up (returned to hand as a "beam me up" cost).
    pub cant_be_beamed: bool,
    pub cant_phase_out: bool,
    pub zero_loyalty_stays: bool,
    /// Can't become suspected.
    pub cant_become_suspected: bool,
    /// Must attack this player each combat if able.
    pub must_attack_player: Option<PlayerId>,
    /// Must attack this planeswalker/battle each combat if able (Kytheon's Gideon).
    pub must_attack_pw: Option<CardId>,
    /// Can attack as though it had haste.
    pub attack_haste: bool,
    /// Attackers this creature can't block: (filter on the attacker, the static's source, its controller).
    pub cant_block_attackers: SmallVec<[(Filter, CardId, PlayerId); 1]>,
    /// Players whose static abilities goad this creature (on top of the one-shot goads in `Game::goaded`).
    pub static_goaders: SmallVec<[PlayerId; 1]>,
    /// Blocks each combat if able.
    pub must_block: SmallVec<[(Option<FilterBox>, CardId, PlayerId); 1]>,
    /// Must be blocked if able (by at least one creature).
    pub must_be_blocked: bool,
    /// Must be blocked by at least n creatures matching the filter if able: (blocker filter, n, static source, its controller).
    pub must_be_blocked_by: SmallVec<[(Filter, u8, CardId, PlayerId); 1]>,
    /// (filter on the blocker, the static's source, its controller)
    /// (what the attacked player must / must not control, forbids-if-controls, static source, its controller)
    pub attack_restr: SmallVec<[(Filter, bool, CardId, PlayerId, bool); 1]>,
    /// (player the creature can't attack, true = only the player and not their planeswalkers / battles): from granted `CantAttackController`.
    pub attack_bar: SmallVec<[(PlayerId, bool); 1]>,
    pub cant_be_blocked_by: SmallVec<[(Filter, CardId, PlayerId); 1]>,
    /// Conditions judged with the defending player as "you": (condition, true = makes the creature unblockable / false = forbids attacking that player, static source, its controller).
    pub attacker_conds: SmallVec<[(Condition, CardId); 1]>,
    pub defender_conds: SmallVec<[(Condition, bool, CardId, PlayerId); 1]>,
    /// Abilities granted by other permanents' static abilities (index `GRANTED_BASE + i` in actions).
    pub granted: SmallVec<[std::sync::Arc<Ability>; 1]>,
    /// The permanent that granted each entry of `granted` (parallel).
    pub granted_by: SmallVec<[CardId; 1]>,
}

#[derive(Clone, Debug)]
pub struct Card {
    pub id: CardId,
    /// Current face's definition (changes on transform / copy effects).
    pub def: DefId,
    pub front: DefId,
    pub owner: PlayerId,
    pub controller: PlayerId,
    pub zone: Zone,
    /// Zone change counter: a new object each time it changes zones.
    pub zcc: u32,
    /// Turn number on which this permanent entered the battlefield / declared as an attacker (0 = never).
    pub entered_turn: u32,
    /// Length of `Game.exile_mv_log` when this permanent entered the battlefield (cards exiled earlier this turn are not noted for it).
    pub noted_from: usize,
    /// Exiled before the game for a card with this name (`Ability::PregameSideboardExile`).
    pub pregame_for: Option<Sym>,
    /// Bit n set: the number n was chosen by this source (`ChooseNumber.exclude_chosen`).
    pub chosen_mask: u32,
    /// Text-changing effects on this object (`textchange`): the change and whether it ends at end of turn. `text_base` is the definition before any of them.
    pub text_changes: Vec<(crate::textchange::TextChange, bool)>,
    pub text_base: Option<DefId>,
    /// The colour word every colour word is currently changed to by Swirl the Mists (a `Color` letter), applied to this object.
    pub text_global: Option<String>,
    /// The turn this creature last attacked because it was required to (goaded, "attacks each combat if able"); 0 if its last attack was voluntary (Firkraag).
    pub had_to_attack_turn: u32,
    /// The turn a counter was last put on this permanent, and the bit set of players who put counters on it that turn (`Filter.counter_put_by_you_this_turn`).
    pub counter_put_turn: u32,
    pub counter_put_mask: u8,
    /// It was a creature as it entered (counted in the controller's `creatures_entered`).
    pub entered_as_creature: bool,
    /// The zone it entered the battlefield from (Celes: "entered from a graveyard").
    pub entered_from: Option<Zone>,
    /// The turn this card was last discarded (mayhem).
    pub discarded_turn: Option<u32>,
    /// Mutate: the original card's definition when a mutating creature went over it (`def` is then the top card's).
    pub mut_orig: Option<DefId>,
    /// Mutate: definitions of the merged cards that are NOT on top (their abilities and keywords are granted to the permanent).
    pub mut_parts: Vec<DefId>,
    /// Mutate: the cards merged into this permanent (kept off every zone list until it leaves the battlefield).
    pub merged: Vec<CardId>,
    /// Haunt: the creature this exiled card haunts.
    pub haunting: Option<CardId>,
    pub attacked_turn: u32,
    /// How many times it was declared as an attacker on `attack_count_turn` (Moraug, Fury of Akoum).
    pub attack_count: u8,
    pub attack_count_turn: u32,
    /// How many times it regenerated on `regen_count_turn` (Spiny Starfish).
    pub regen_count: u8,
    pub regen_count_turn: u32,
    /// The turn it was last put into a graveyard by surveilling, and its zone-change count then (Eye of Duskmantle).
    pub surveilled_turn: u32,
    pub surveilled_zcc: u32,
    /// The turn it was last milled and its zone-change count then (`Filter.milled_this_turn`).
    pub milled_turn: u32,
    pub milled_zcc: u32,
    /// Turn it was drawn into a hand and the zone-change count then (`Filter.drawn_this_turn`).
    pub drawn_turn: u32,
    pub drawn_zcc: u32,
    /// Players this creature has attacked (not their planeswalkers) on `attacked_players_turn`, as a bit mask.
    pub attacked_players: u8,
    pub attacked_players_turn: u32,
    /// The turn it last fought.
    pub fought_turn: u32,
    /// It has dealt damage since it last entered its zone ("as long as it hasn't dealt damage yet").
    pub dealt_damage_ever: bool,
    /// This turn this planeswalker may activate loyalty abilities twice (Kaito, Dancing Shadow).
    pub loyalty_double_turn: u32,
    /// This turn this planeswalker may activate loyalty abilities `loyalty_extra` more times (Comet, Stellar Pup); valid while `loyalty_extra_turn` is the current turn.
    pub loyalty_extra_turn: u32,
    pub loyalty_extra: u8,
    /// The player it last attacked (a planeswalker/battle counts as its controller).
    pub attacked_player: Option<PlayerId>,
    /// The controller's `combat_serial` when this last attacked (`attacked_player` is whom).
    pub attacked_combat: u32,
    /// Turn it last attacked a battle (0 = never).
    pub attacked_battle_turn: u32,
    /// Turn this creature last blocked / was last turned face up (0 = never).
    pub blocked_turn: u32,
    /// Turn this attacker was last blocked (0 = never).
    pub was_blocked_turn: u32,
    /// (turn, other creature) for each block this creature took part in.
    pub block_pairs: Vec<(u32, CardId)>,
    /// Turn this permanent's once-each-turn play-from-zone permission was last used.
    pub may_play_used_turn: u32,
    /// Turn this permanent's once-each-turn token replacement (Mirrormind Crown) was last offered.
    pub token_repl_turn: u32,
    /// Permanent types already used this turn (`may_play_types_turn`) through a `MayPlayFrom{per_type}` permission: bitmask of `CardType as u32`.
    pub may_play_types: u32,
    pub may_play_types_turn: u32,
    /// Modes of `ChooseModes { once_each_turn }` already chosen this turn: (turn, mode index).
    pub modes_used: Vec<(u32, u8)>,
    pub face_up_turn: u32,
    pub enlisted_turn: u32,
    pub tapped: bool,
    pub face_down: bool,
    /// Extra card types / creature types this permanent has while it is face down (Death in Heaven: artifact Cyberman); cleared when it changes zones.
    pub fd_extra: Option<Box<(Vec<CardType>, Vec<String>, bool)>>,
    /// A token copy of a face-down permanent: nameless while it stays on the battlefield (CR 707.2); cleared when it changes zones.
    pub fd_copy: bool,
    pub damage: i32,
    pub deathtouch_hit: bool,
    /// Permanents that dealt damage to this one this turn (cleared at cleanup and when it enters the battlefield).
    pub damaged_by: SmallVec<[CardId; 2]>,
    /// Damage marked per source (cleared when damage is next dealt to an undamaged creature); read by `LethalNeedsSingleSource`.
    pub dmg_parts: SmallVec<[(CardId, i32); 2]>,
    /// A regeneration shield is active until end of turn.
    /// Regeneration shields (each destruction replaced uses one up).
    pub regen: u8,
    /// "The next time it would be destroyed this turn, remove all damage from it instead" (Pyramids).
    pub heal_shield: bool,
    /// With the shield: whoever gains control of the creature when it regenerates (source, new controller; Debt of Loyalty).
    pub regen_steal: Option<(CardId, PlayerId)>,
    /// The turn number until which this permanent can't be regenerated (0 = no restriction).
    pub no_regen_turn: u32,
    /// Turn in which damage to this permanent can't be prevented or redirected (`Effect::DamageLockThisTurn`); reset when it changes zones.
    pub dmg_lock_turn: u32,
    /// Creature / land type chosen as it entered.
    pub chosen_type: Option<Sym>,
    /// The second type of "choose two basic land types" (`ChooseKind::SecondBasicLandType`).
    pub chosen_type2: Option<Sym>,
    /// Types chosen by several players during one resolution (`ChooseCreatureType.accumulate`), with the zone-change count they belong to.
    pub chosen_types: (u32, Vec<Sym>),
    /// Colours chosen per player during one resolution (`ChooseColor.per_player`, Selective Obliteration).
    pub player_colors: Vec<(PlayerId, Color)>,
    /// Per-player as-enters picks (`ChooseKind::WarPeace`): true = the second option (peace). A player is listed once asked.
    pub player_pick: Vec<(PlayerId, bool)>,
    /// The type / colour this card had chosen when it last left the battlefield (last-known information for abilities that resolve after a sacrifice cost).
    pub left_chosen_type: Option<Sym>,
    pub left_chosen_color: Option<Color>,
    /// "Choose a card name": the name chosen on this card.
    pub chosen_name: Option<String>,
    /// A second chosen card name (`ChooseCardName.second`); `Filter.chosen_name` matches either name.
    pub chosen_name2: Option<String>,
    /// Names already chosen by `Effect::CopyNamedCard.once_each` (Garth One-Eye); cleared when the card leaves the battlefield.
    pub names_used: Vec<String>,
    pub chosen_color: Option<Color>,
    /// The second color of "choose two colors" (`ChooseKind::SecondColor`).
    pub chosen_color2: Option<Color>,
    /// `Effect::MarkZcc`: the card's zone-change count plus one when marked (0 = never); `Condition::ZccMarked` tests that the card has not changed zones since.
    pub zcc_mark: u32,
    /// For a token: the object (and its zone-change count then) whose effect created it (`Filter.created_by_this`).
    pub created_by: Option<(CardId, u32)>,
    /// The permanent whose `GrantPlayFromExile` let this card be played; kept while it moves from exile to the stack or battlefield (`Filter.played_via_this`).
    pub played_via: Option<(CardId, u32)>,
    /// A spell on the stack that gained rebound when it was cast (`Effect::GainRebound`).
    pub gained_rebound: bool,
    pub chosen_player: Option<PlayerId>,
    /// An emblem made with `CreateEmblem.bind`: the card it was bound to (`Sel::BoundCard`); a bound player is the emblem's `chosen_player`.
    pub bound_card: Option<CardId>,
    /// The exiled card this permanent last chose with `Effect::SetChosenCard` (`Sel::ChosenCard`; Koh, the Face Stealer). Cleared when it leaves the battlefield.
    pub chosen_card: Option<CardId>,
    /// An emblem made with `CreateEmblem.until_end_of_turn`: it ceases to exist in the cleanup step.
    pub emblem_eot: bool,
    pub chosen_player2: Option<PlayerId>,
    pub chosen_number: Option<i32>,
    /// Creature types noted for this permanent (`Effect::NoteCreatureType`, `Amount::NotedTypes`: Volo's Journal).
    pub noted_types: Vec<Sym>,
    pub counters: SmallVec<[(CounterKind, u16); 2]>,
    pub attached_to: Option<CardId>,
    /// The player this Curse / "enchant player" Aura is attached to.
    pub enchanted_player: Option<PlayerId>,
    /// A battle's protector (the opponent who defends it).
    pub protector: Option<PlayerId>,
    /// What it was attached to when it last left the battlefield (an Aura sacrificed as a cost still "remembers" its creature).
    pub left_attached_to: Option<CardId>,
    /// Counters it had when it last left the battlefield (last-known information for "for each counter on it").
    pub left_counters: SmallVec<[(CounterKind, u16); 2]>,
    /// An oil counter was removed from it this turn, while this player controlled it (cleared at the start of each turn; `Condition::OilCounterEventThisTurn`).
    pub oil_removed_by: Option<PlayerId>,
    /// Turn number the permanent entered the battlefield (summoning sickness).
    pub since: u32,
    /// Turn number its current controller got control of it (entry or a control change): "controlled continuously since the beginning of the turn" is `control_turn < turn`.
    pub control_turn: u32,
    /// Was tapped when the current turn began (set for every battlefield permanent as the turn changes, before untapping): `Condition::SourceStartedTurnUntapped`.
    pub tapped_at_turn_start: bool,
    pub ts: u64,
    pub token: bool,
    pub cur: Derived,
    pub activations: SmallVec<[(u16, u8); 1]>,
    /// Triggered abilities (by index) that already used up a `once_ever` flag since this card last changed zones.
    pub trig_ever: SmallVec<[u16; 1]>,
    pub cast_from: Option<Zone>,
    /// Cast out of sorcery timing only through its own "as though it had flash" (Armor of Thorns); reset on each cast.
    pub cast_own_flash: bool,
    /// zcc of the object whose echo cost was already handled at an upkeep (u32::MAX = never): echo is due only once per object.
    pub echo_done_zcc: u32,
    /// It left the battlefield while face down (so it has no creature types as a last-known-information source: Descendants' Fury).
    pub left_face_down: bool,
    /// Creature/permanent subtypes among the permanents its caster controlled as it was cast ("controlled a Dragon as you cast this spell"); `*` = a changeling.
    pub cast_subtypes: Vec<Sym>,
    /// The greatest power among the creatures its caster controlled as it was cast (Lifestream's Blessing).
    pub cast_power: i32,
    /// Phyrexian mana symbols paid with 2 life as it was cast (compleated).
    pub phyrexian_life: u8,
    pub kicked: bool,
    /// Cast with an offspring cost granted by `StaticEffect::GrantOffspring` paid.
    pub granted_offspring: bool,
    pub kicks: u8,
    /// Bit i set = the i-th Kicker ability was paid.
    pub kicked_mask: u8,
    /// Times each of the first four kicker abilities was paid (`Amount::KickCountAt`).
    pub kick_by: [u8; 4],
    /// A teamwork kicker (one whose cost taps creatures) was paid.
    pub teamwork: bool,
    /// Monstrosity has been used on it.
    pub monstrous: bool,
    pub renowned: bool,
    /// Harnessed designation (The Soul Stone).
    pub harnessed: bool,
    /// Suspected: has menace and can't block.
    pub suspected: bool,
    /// Token copy made by a `CopyPermanent` effect of this source (and the source's zone-change count then).
    pub made_by: Option<(CardId, u32)>,
    /// Detained by this player until the start of that player's next turn: can't attack or block, its activated abilities (mana too) can't be activated.
    pub detained_by: Option<PlayerId>,
    /// Cast with bestow: an Aura (not a creature) while attached.
    pub bestowed: bool,
    /// An Aura card put onto the battlefield with no legal host by `Effect::Move.non_aura_if_no_host`: it is a plain Enchantment (no Aura subtype, no "enchant" ability).
    pub non_aura: bool,
    /// Returned to the battlefield as an Aura enchantment by `Effect::ReturnAsAura` (Bronzehide Lion): not a creature, an Aura (`Condition::AsAura`).
    pub as_aura: bool,
    /// An extra creature type this permanent (or the spell it was) has "in addition to its other types" (Tarrian's Journal: Vampire).
    pub extra_sub: Option<Sym>,
    /// How many permanents its last `SacrificeAny` sacrificed (Devour and friends read it from other abilities).
    pub sacrificed_any: u32,
    /// Total power / toughness (last known) of the permanents its last `SacrificeAny` sacrificed (Dracoplasm).
    pub sacrificed_any_power: i32,
    pub sacrificed_any_toughness: i32,
    /// Prepare: the exiled spell copy of a prepared creature / the creature a prepare-spell copy belongs to.
    /// Mana spent to cast it, by type (W U B R G C): read by converge / sunburst / "if at least four mana was spent".
    pub mana_spent: [u8; 6],
    /// Mana spent on the most recent activation of one of this permanent's abilities, by type (read by `Effect::NoteManaSpent`).
    pub act_spent: [u8; 6],
    /// A face-down creature put onto the battlefield by Illusionary Mask: the first time it would deal or be dealt damage or become tapped it is turned face up instead.
    pub unmask: bool,
    /// The mana type noted by `Effect::NoteManaSpent` (0 = none, else `ManaType` index + 1; Jeweled Amulet).
    pub noted_mana: u8,
    /// Every mana spent by the most recent `Effect::NoteManaSpentAll`, by type (Ice Cauldron).
    pub noted_mana_all: [u8; 6],
    /// The card most recently exiled with this permanent's ability (`Filter.last_exiled_with_this`: Ice Cauldron).
    pub last_exiled_with: Option<CardId>,
    /// Snow sources that paid for this spell (each counted once; {S} and "snow mana spent").
    pub snow_spent: u8,
    /// Colors of mana produced by snow sources that paid for this spell (bit per `Color`).
    pub snow_colors: u8,
    /// Mana spent to cast this spell by kind of source ([Treasure, artifact, Cave, Desert]); one per source tapped.
    pub src_spent: [u8; 5],
    /// The permanents tapped for mana to cast this spell (Filter.paid_with_mana_from_source).
    pub paid_src: Vec<CardId>,
    /// Mana from a Treasure was spent to cast this spell.
    pub mana_from_treasure: bool,
    /// The creatures tapped to convoke this spell (read by "for each creature that convoked it").
    pub convoked: SmallVec<[CardId; 4]>,
    /// The player who paid part of this spell's cost through assist (Yume, Chronicler of Valor).
    pub assist_by: Option<PlayerId>,
    /// Additional +1/+1 counters this spell enters with because of the mana spent on it.
    pub rider_counters: u8,
    /// Charge counters this spell enters with (sunburst on a noncreature spell).
    pub rider_charge: u8,
    /// Additional loyalty counters this planeswalker spell enters with (`Effect::SpellEntersWithCounters.loyalty`).
    pub rider_loyalty: u8,
    /// Cast from a graveyard through a `MayPlayFrom{finality}` permission: enters with a finality counter.
    pub finality_rider: bool,
    /// A counter this permanent enters with, put on it by the effect that moves it onto the battlefield (`LookAtTop.enters_with`).
    pub enter_counter_rider: Option<CounterKind>,
    /// Cast with the haste-and-sacrifice rider (`Effect::MarkHasteSacRider`).
    pub haste_sac_rider: bool,
    /// Cast with a granted blitz cost: on entering it gains "When this creature dies, draw a card".
    pub blitz_rider: bool,
    /// Cast with a granted evoke cost: it is sacrificed when it enters.
    pub evoke_rider: bool,
    /// Cast with a granted sneak cost: it enters tapped and attacking.
    pub sneak_rider: bool,
    /// Gains haste until end of turn when it enters from the cast set up by `Effect::MarkCastHaste`.
    pub cast_haste: bool,
    /// Creature spell that gained riot from mana with `ManaRider::Riot`: choose a +1/+1 counter or haste as it enters.
    pub riot_rider: bool,
    /// Cast with a granted warp alternative cost: exile it at the next end step after it enters (set while on the stack).
    pub warp_rider: bool,
    /// Played or cast through a `MayPlayFrom{leave_exile_gain}` permission: on entering it gains the "exile it and you gain N life" trigger (N here).
    pub play_leave_gain: u8,
    pub prepared_spell: Option<CardId>,
    pub prepare_of: Option<CardId>,
    /// The player who exiled this card with an effect that tags its exiler (`Effect::SetExiledBy`; Shared Fate: `MayPlayFrom.exiler`).
    pub exiled_by: Option<PlayerId>,
    /// This (copy) card was cast as a prepare spell (`Filter.prepared_spell`).
    pub cast_prepared: bool,
    /// A token created by `CopyPermanent.link_source`: the card it was copied from (Hofri Ghostforge; `Sel::LinkedCard`).
    pub linked_card: Option<CardId>,
    /// Unearthed: exiled instead of going anywhere else when it leaves the battlefield.
    pub exile_on_leave: bool,
    /// Gained suspend from an effect while in exile ("it gains suspend").
    pub granted_suspend: bool,
    /// Put onto the battlefield face down by Cloak: has ward {2} while face down.
    pub cloaked: bool,
    /// "If it would die this turn, exile it instead" (wears off at cleanup).
    pub exile_if_dies: bool,
    /// Which alternative cost it was cast with (evoke, overload ...).
    pub cast_alt: Option<u8>,
    /// Cast with a granted alternative cost that exiles the spell when it leaves the stack.
    pub cast_exile_after: bool,
    /// "If that spell would be put into a graveyard, put it on the bottom of its owner's library instead" (`Effect::CastFree.bottom_after`): applies while the card is on the stack.
    pub cast_bottom_after: bool,
    /// The permanent whose mana (with `ManaRider::ExileSpell`) was spent on this spell: the spell is exiled with it if the controller accepts (Forger's Foundry).
    pub exile_link: Option<CardId>,
    /// Cast from the sideboard through `MayPlayFromOutsideGame { extra_mode }`: a modal spell may choose one additional mode.
    pub extra_mode: bool,
    /// Airbend: the owner may cast this card from exile for this mana cost (set by Effect::Airbend; cleared when it leaves exile).
    pub exile_cast_cost: Option<String>,
    /// Exiled "on an adventure": castable from exile as its front face.
    pub on_adventure: bool,
    /// Who cast the Adventure half that put this card on an adventure (may differ from the owner: Robber of the Rich).
    pub adventure_by: Option<PlayerId>,
    /// Last turn number through which `play_by` may play / cast this card from exile (0 = no permission).
    pub play_until: u32,
    /// With a `WhileSourceControlled` play window: the permission lapses when this permanent is no longer controlled by `play_by`.
    pub play_src: Option<CardId>,
    /// Extra condition on the play-from-exile permission (`GrantPlayFromExile.play_cond`).
    pub play_cond: Option<crate::dsl::Condition>,
    /// Foretold / plotted / warped exile mark and the turn it was set (cast permission starts on a later turn).
    pub exile_mark: Option<crate::dsl::ExileMark>,
    pub exile_mark_turn: u32,
    /// Cast from suspend: has haste while on the battlefield.
    pub free_haste: bool,
    /// "Doesn't untap during its controller's next untap step."
    pub skip_untap: bool,
    /// With `skip_untap` from exert: the player who exerted it; the skip only applies to that player's untap step (a stolen exerted creature untaps for its owner).
    pub skip_untap_by: Option<PlayerId>,
    pub play_by: PlayerId,
    /// Cast from exile spending mana as though it were mana of any type.
    pub spend_any: bool,
    /// Copies to make when this card is cast by `Effect::CastFree.copies` (consumed by `finish_cast`).
    pub cast_copies: u8,
    /// Power of the card / permanent beheld as this spell's cost was paid (Dragon's Fire, Monstrous Emergence); cleared when it leaves the stack.
    pub beheld_power: Option<i32>,
    /// The card may be cast from exile without paying its mana cost (GrantPlayFromExile free).
    pub play_free: bool,
    /// The exile-cast permission costs life equal to the card's mana value instead of mana.
    pub play_life_mv: bool,
    /// Cast from exile with affinity for permanents matching this filter (GrantPlayFromExile.affinity).
    pub play_affinity: Option<crate::dsl::FilterBox>,
    /// A token created with a set size ("an X/X token"): the size is part of what a copy of it copies (populate, Rite of Replication).
    pub token_set_pt: Option<(i32, i32)>,
    /// Generic mana more a spell cast through the exile permission costs (GrantPlayFromExile.extra_generic).
    pub play_extra: u8,
    /// A land played through the exile permission enters tapped (GrantPlayFromExile.land_tapped).
    pub play_land_tapped: bool,
    /// A spell cast through the exile permission is exiled instead of going to the graveyard (GrantPlayFromExile.exile_after).
    pub play_exile_after: bool,
    /// The exile-play permission stops when the end step of its last turn begins (`PlayWindow::UntilYourNextEndStep`).
    pub play_stop_end: bool,
    /// The exile-play permission stops after the end of combat step of its last turn (`PlayWindow::UntilYourNextCombatEnd`).
    pub play_stop_combat: bool,
    /// Turn number through which its owner may cast this graveyard card as an Adventure (`Effect::AdventureFromGraveyard`; 0 = no permission).
    pub adv_gy_until: u32,
    /// The exile-play permission is one of several of which only one may be used (`GrantPlayFromExile.one`).
    pub play_one: bool,
    /// `GrantPlayFromExile.limit`: at most this many of the granted cards may be played (0 = no limit); `play_used` counts those already cast or played (kept on every card of the group).
    pub play_limit: u8,
    pub play_used: u8,
    /// The exile-play permission allows one spell per nonland card type among the granted cards (`GrantPlayFromExile.per_type`): the card types already used up (bit per `CardType`, shared by the group).
    pub play_per_type: bool,
    pub play_types_used: u32,
    /// `Effect::FreeCastOneThisTurn`: the card may be cast from exile without paying its mana cost during this turn number (0 = not offered).
    pub free_turn: u32,
    /// The permanent whose `FreeCastOneThisTurn` offer this is (casting one card ends the offer on the others).
    pub free_via: Option<CardId>,
    /// Played from the graveyard through a `MayPlayThisTurn { tapped }` permission: it enters the battlefield tapped.
    pub enter_tapped_once: bool,
    /// The card this permanent was before it became a copy "until turned face down" (`EntersAsCopy.until_turned_face_down`).
    pub fd_revert: Option<DefId>,
    /// `Effect::NoteResolution`: the turn and how many times the source noted a resolution that turn.
    pub res_note: (u32, u8),
    /// A face-up choice (copy / attach) is being made because the permanent was just turned face up, not because it entered.
    pub turnup_choice: bool,
    /// Extra mana added to the cost of the next cast of this card (CastFree.extra_mana: Ogre Battlecaster); consumed by `begin_cast`.
    pub cast_extra_mana: Option<String>,
    /// Phased out: still on the battlefield per the rules but treated as if it didn't exist (not in `Game::battlefield`).
    pub phased_out: bool,
    /// Phased out because the permanent it is attached to phased out (it phases in with it).
    pub phased_indirect: bool,
    /// Soulbond partner.
    pub paired: Option<CardId>,
    /// The sector (1 alpha, 2 beta, 3 gamma; 0 none) this creature was assigned to by a space sculptor (Space Beleren).
    pub sector: u8,
    /// Room doors unlocked (bit 0 = door 1, bit 1 = door 2).
    pub doors: u8,
    /// Phased out until the source that did it leaves (no phasing in at untap).
    pub phase_hold: Option<(CardId, u32)>,
    /// Can't phase in until the end of this player's next turn after the stored turn (Teferi, Timeless Voyager).
    pub phase_lock: Option<(PlayerId, u32)>,
    /// Toughness of the creature sacrificed to pay this spell's emerge cost (read by its enters trigger).
    pub emerge_toughness: i32,
    /// As-enters choices already made (1 pay life, 2 reveal, 4 copy, 8 choose).
    pub enter_done: u16,
    /// Total power and toughness of the cards exiled by `EntersExileAny` as this entered.
    pub enter_exiled_pt: (i32, i32),
    /// The permanent whose ability exiled this card (and its zone-change counter then).
    pub exiled_with: Option<(CardId, u32)>,
    /// The permanent this one put onto the battlefield and is linked to (and that permanent's zone-change count then): Diabolic Servitude. Reset when this enters the battlefield.
    pub linked: Option<(CardId, u32)>,
    /// Further permanents linked by later `LinkRemembered` effects (Seraph returns several creatures); read with `Filter.linked_with_this` too.
    pub linked_extra: Vec<(CardId, u32)>,
    /// This land entered the battlefield by being played (reset whenever it enters).
    pub land_played: bool,
    pub saddled: bool,
    /// Exhaust abilities already used (by ability index).
    pub exhausted: Vec<u16>,
    pub x: i32,
    /// Cards the cast paid as a cost (a kicker discard), read by this permanent's enters-with-counters amount (Dralnu's Pet).
    pub cast_remembered: Vec<Obj>,
    pub is_commander: bool,
    /// A commander put into a graveyard or exile since state-based actions were last checked: its owner may put it into
    /// the command zone (CR 903.9a).
    pub cmd_offer: bool,
    /// A card in its owner's sideboard that is their chosen companion (CR 702.139): `Action::Companion` puts it into hand for {3}.
    pub is_companion: bool,
    /// A token that has left the battlefield: it no longer exists and can't move anywhere.
    pub ceased: bool,
    pub cast_count: u32,
    pub revealed: bool,
    /// Players (bit per seat) who know which card this is although it is in a hidden zone: it was public earlier (bounced, tucked), revealed, or looked at. The owner always knows their own cards. Used by the AI to sample only what it cannot know.
    pub known_to: u8,
}

impl Card {
    pub fn counter(&self, k: CounterKind) -> i32 {
        self.counters.iter().find(|(c, _)| *c == k).map_or(0, |(_, n)| *n as i32)
    }
    pub fn add_counters(&mut self, k: CounterKind, n: i32) {
        if n <= 0 {
            return;
        }
        if let Some(e) = self.counters.iter_mut().find(|(c, _)| *c == k) {
            e.1 = e.1.saturating_add(n.min(u16::MAX as i32) as u16);
        } else {
            self.counters.push((k, n.min(u16::MAX as i32) as u16));
        }
    }
    pub fn remove_counters(&mut self, k: CounterKind, n: i32) -> i32 {
        let have = self.counter(k);
        let take = n.min(have).max(0);
        if take > 0 {
            if k == CounterKind::Oil {
                self.oil_removed_by = Some(self.cur.controller);
            }
            if let Some(e) = self.counters.iter_mut().find(|(c, _)| *c == k) {
                e.1 -= take as u16;
            }
            self.counters.retain(|(_, n)| *n > 0);
        }
        take
    }
}

/// Where an automatic pass of priority (`Player::pass_until`) ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum PassUntil {
    /// until this turn and step are reached
    Step { turn: u32, step: Step },
    /// until this stack item is the top of the stack (or has left it)
    StackTop { item: StackId },
    /// until a turn of `player` later than turn `after` has started
    PlayerTurn { player: PlayerId, after: u32 },
}

#[derive(Clone, Debug)]
pub struct Player {
    pub id: PlayerId,
    pub name: String,
    pub life: i32,
    pub library: Vec<CardId>, // top = last
    pub hand: Vec<CardId>,
    pub graveyard: Vec<CardId>,
    pub exile: Vec<CardId>,
    pub command: Vec<CardId>,
    /// Archenemy: this player's scheme deck (top = last), face down outside every zone list.
    pub scheme_deck: Vec<CardId>,
    /// Team (Archenemy: the archenemy alone against everyone else); players of one team are not opponents.
    pub team: u8,
    /// Planechase: planar die rolls this player made this turn (each costs {N} for N rolls before it).
    pub planar_rolls: u32,
    /// Cards outside the game (the deck's sideboard), visible to the owner only.
    pub sideboard: Vec<CardId>,
    pub pool: ManaPool,
    /// Part of `pool` that survives step changes until the end of combat (firebending).
    pub pool_keep: ManaPool,
    /// Mana that stays until the end of the turn (Brazen Collector).
    pub pool_turn: ManaPool,
    pub poison: i32,
    pub energy: i32,
    /// Energy paid or lost so far this turn (Blaster Hulk, Izzet Generatorium).
    pub energy_spent: i32,
    pub experience: i32,
    pub rad: i32,
    pub lands_played: u8,
    pub land_limit: u8,
    pub spells_cast: u32,
    pub spells_cast_last: u32,
    /// Spells this player has cast this game.
    pub spells_cast_game: u32,
    /// The turn number on which this player last cast a spell face down (0 = never).
    pub face_down_cast_turn: u32,
    /// The turn number on which a permanent last entered face down under this player's control, or this player last turned a permanent face up (0 = never; Oblivious Bookworm).
    pub face_down_act_turn: u32,
    /// The turn number on which this player last played a land or cast a spell from anywhere other than their hand (0 = never).
    pub nonhand_turn: u32,
    pub life_gained: i32,
    /// How many separate life-gain events this player has had this turn ("the first time each turn you gain life").
    pub life_gain_events: u32,
    pub life_lost: i32,
    /// Life lost during the previous turn.
    pub life_lost_last: i32,
    /// Life total as the current turn began (Sengir, the Dark Baron).
    pub life_turn_start: i32,
    /// Number of cards in the player's deck when the game began (library, and commanders), for `Amount::StartingDeckSize`.
    pub starting_deck: i32,
    /// Turn number through which this player may spend mana as though it were mana of any type for one spell (`Effect::SpendAnyTypeForOneSpell`; North Star); 0 = no allowance.
    pub any_type_turn: u32,
    /// Turn number on which the next spell this player casts has cascade (`Effect::NextSpellCascade`; Bigger on the Inside); 0 = none.
    pub cascade_turn: u32,
    /// Turn number on which every spell this player casts from their hand has cascade (`Effect::HandSpellsCascade`; Yidris); 0 = none.
    pub hand_cascade_turn: u32,
    /// Was dealt damage this turn.
    pub damaged: bool,
    /// Bit per `BendKind` (water, earth, fire, air) of the kinds of bending this player did on turn `bend_turn`.
    pub bend_mask: u8,
    pub bend_turn: u32,
    /// Total damage dealt to the player this turn.
    pub damage_taken_turn: i32,
    /// Cards this player discarded this turn.
    pub discarded_turn: i32,
    /// Cards this player cycled this turn (cycling abilities only, not channel).
    pub cycled_turn: i32,
    /// Game turn number in which this player was last dealt combat damage (0: never); with `prev_turn` for `Condition::NoCombatDamageSinceYourLastTurn`.
    pub combat_dmg_turn: u32,
    /// Game turn number in which this player last cast a spell (0: never); with `prev_turn` for `Condition::NoOpponentSpellSinceYourLastTurn`.
    pub spell_cast_turn: u32,
    /// Turn number of this player's current / most recent turn and of the one before it.
    pub cur_turn: u32,
    pub prev_turn: u32,
    /// Committed a crime this turn (targeted an opponent, their permanent or their graveyard card).
    pub crime: bool,
    /// "Start your engines!": 0 = no speed, 1..=4 (4 is max speed).
    pub speed: u8,
    /// The city's blessing (Ascend): gained once the player controls ten or more permanents; never lost.
    pub city_blessing: bool,
    /// Storied: an enduring story, latched once the player controls three or more artifacts, legendaries and/or Sagas; never lost.
    pub enduring_story: bool,
    /// The Ring: how many times it has tempted this player (0..=4 shown), the chosen Ring-bearer and the emblem for the current level.
    /// Venture: the dungeon card (in the command zone) the player is in and the name of the room they are in; dungeons completed so far.
    pub dungeon: Option<(CardId, String)>,
    pub dungeons_completed: u8,
    /// `Game.turn` (+1) of the turn this player may tap lands they don't control for mana (`Effect::TapForeignLandsThisTurn`); 0 = never.
    pub foreign_lands_turn: u32,
    /// `(Game.turn + 1, source card)` of the turn this player may pay 1 life for {C} any time they could activate a mana ability (`Effect::LifeManaThisTurn`).
    pub life_mana: Option<(u32, CardId)>,
    /// The color chosen before the game began (`Ability::PregameChooseColor`).
    pub pregame_color: Option<Color>,
    /// Names of the dungeons this player has completed.
    pub completed_dungeon_names: Vec<String>,
    pub ring_level: u8,
    pub ring_bearer: Option<CardId>,
    pub ring_emblem: Option<CardId>,
    /// Total mana this player has spent this turn (for "expend").
    pub mana_spent_turn: u32,
    /// Mana added by spells / triggers that may only pay for certain things; `None` once spent. Emptied with the pool.
    pub restricted: Vec<Option<(ManaType, crate::dsl::ManaRestrict, CardId)>>,
    /// Mana floating in the pool came (partly) from a Treasure tapped by hand: spending any of it counts as "mana from a Treasure was spent".
    pub pool_treasure: bool,
    /// "Until your next turn": creatures can't attack this player (or their planeswalkers) at all (`shield_bar`), or only by paying `shield_tax` generic each.
    pub shield_bar: bool,
    pub shield_tax: i32,
    /// Parallel to `restricted`: slots added `until_end_of_turn` survive step changes.
    pub restricted_turn: Vec<bool>,
    /// Speed already increased this turn.
    pub speed_up: bool,
    pub drew_from_empty: bool,
    pub lost: bool,
    pub conceded: bool,
    pub mulligans: u8,
    pub kept: bool,
    /// Free mulligan + scry 3: cards of the three already put on the bottom while deciding this hand (the peek is the
    /// top `3 - scry_bottomed`; a mulligan sends only those with the hand).
    pub scry_bottomed: u8,
    /// The player declined to use Serum Powder for the current keep decision.
    pub powder_declined: bool,
    /// The player declined to begin the game with Gemstone Caverns.
    pub luck_declined: bool,
    pub max_hand: i32,
    pub commander_damage: SmallVec<[(CardId, i32); 2]>,
    pub commander_cast: SmallVec<[(CardId, u32); 2]>,
    /// Pass priority automatically when there is nothing to do (the engine skips the question).
    pub auto_skip: bool,
    pub allow_cancel: bool,
    /// Ask for each mana payment step instead of paying automatically.
    pub manual_pay: bool,
    /// Ask for the mana payment of the next spell or ability only ("cast, tapping mana by hand"); cleared once it is paid or cancelled.
    pub manual_once: bool,
    /// Steps (bit per `Step`) where this player wants priority on their own turn / on others' turns when the stack is empty.
    pub stops_mine: u16,
    pub stops_opp: u16,
    /// Ask for priority at every step, even with nothing to do.
    pub full_control: bool,
    /// Offer "tap this for mana" as a priority action (humans: a land can be tapped at instant speed with nothing to spend the mana on).
    /// It never counts as something to do, so it never holds priority by itself.
    pub offer_mana: bool,
    /// "End turn" / "pass to a phase" / "pass to a stack item": pass every plain priority automatically (also while
    /// the stack has things on it) until the target is reached. Decisions other than priority still stop it.
    pub pass_until: Option<PassUntil>,
    pub skip_turns: u8,
    /// Draw steps this player skips (Fatigue).
    pub skip_draw: u8,
    pub skip_untap: u8,
    /// Combat phases this player skips (Blinding Angel, Stonehorn Dignitary, Moment of Silence).
    pub skip_combat: u8,
    /// A queued combat skip applies only to turns after this one ("skips all combat phases of their *next* turn" cast during their own turn).
    pub skip_combat_after: u32,
    /// Phases skipped for the whole of one turn (`Effect::SkipPhaseThisTurn`; Fatespinner): the turn number and a bit set (1 draw step, 2 main phases, 4 combat phases).
    pub skip_phases: (u32, u8),
    pub cards_drawn_turn: u32,
    /// Turn of this player's latest venture and how many ventures they made that turn (`StaticEffect::VentureLimit`).
    pub venture_turn: (u32, u8),
    /// The last card this player drew (`Filter.last_drawn`).
    pub last_drawn: Option<u32>,
    /// The latest turn of their own on which this player cast a spell or put a nontoken permanent onto the battlefield (`Condition::ActedLastTurn`).
    pub act_turn: u32,
    /// The turn number on which this player last drew a card (`Condition::DrewLastTurn`).
    pub drew_turn: u32,
    pub attacked_turn: bool,
    /// Searched their own library this turn (Archive Trap).
    pub searched_turn: bool,
    /// Times the player has investigated this turn (Erdwal Illuminator: "for the first time each turn").
    pub investigated_turn: u8,
    /// The turn (if any) in which all of this player's planeswalkers may activate loyalty abilities twice.
    pub loyalty_all_double_turn: u32,
    /// The turn (if any) in which this player may activate loyalty abilities at instant speed (restricted to the given planeswalker subtype), Jace's Machinations.
    pub loyalty_instant_turn: u32,
    /// The turn in which this player last flipped coins (Two-Headed Coin: only the first flip each turn is rigged).
    pub coin_flip_turn: u32,
    /// The resolution (`Game.flip_serial`) of this player's first coin flip this turn: coins flipped together by one effect are one set (Two-Headed Coin).
    pub coin_flip_serial: u64,
    pub loyalty_instant_sub: Option<Sym>,
    /// Scried or surveilled this turn / surveilled this turn.
    pub scried_turn: bool,
    /// Dilemmas this player faced this turn (modes chosen for a spell or ability).
    pub dilemmas_turn: u32,
    /// This player has begun an extra turn this game (H.M.S. Bounty).
    pub began_extra_turn: bool,
    /// Activated a loyalty ability this turn.
    pub loyalty_turn: bool,
    /// Activated an exhaust ability this turn.
    pub exhaust_turn: bool,
    /// Has activated an equip ability this turn (`StaticEffect::EquipFirstFree`).
    pub equip_turn: bool,
    /// "Lands you control don't untap during your next untap step": cleared when that player's upkeep begins.
    pub lands_no_untap: bool,
    /// "Creatures you control don't untap during your next untap step": cleared when that player's upkeep begins.
    pub creatures_no_untap: bool,
    /// "Until your next turn, prevent all damage that would be dealt to you": cleared when that player's turn starts.
    pub damage_shield: bool,
    pub surveilled_turn: bool,
    pub surveil_count_turn: u8,
    /// Number of the turn on which this player's most recent turn began (summoning sickness).
    pub last_turn_start: u32,
    /// The turn number on which the turn before `last_turn_start` began.
    pub prev_turn_start: u32,
    pub hand_turn_start: i32,
    /// Untapped lands this player controlled as their turn began (before untapping): Power Surge.
    pub lands_untapped_turn_start: i32,
    /// Turns this player has taken so far (including the current one).
    pub turns_taken: u32,
    /// Lands that entered the battlefield under this player's control this turn.
    pub lands_entered: u32,
    /// Creatures that entered the battlefield under this player's control this turn / during the previous turn.
    pub creatures_entered: u32,
    pub creatures_entered_last: u32,
    /// Bit mask of the players this player attacked (declared an attacker against) during their most recent turn (Weathered Sentinels).
    pub attacked_last_turn: u8,
    /// Number of combat phases this player has begun on their own turns (`Card.attacked_combat` compares against it: Territorial Hellkite).
    pub combat_serial: u32,
    /// Contraptions this player has assembled (the engine has no Contraption deck; see `Effect::Assemble`).
    pub contraptions: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Target {
    pub obj: Obj,
    pub zcc: u32,
}

/// What happened to trigger an ability (so "that creature", "the damage dealt" can be referred to).
#[derive(Clone, Debug, Default)]
pub struct EventCtx {
    pub obj: Option<Obj>,
    /// The source of the event (damage source, spell cast).
    pub source: Option<CardId>,
    pub player: Option<PlayerId>,
    pub amount: i32,
    /// The replaced damage was combat damage (`DealDamageAgain.keep_combat`).
    pub dmg_combat: bool,
    pub lki: Option<Box<Lki>>,
    /// Batched `ChangesZone` triggers: the total last-known power of the cards moved in the batch (`Amount::EventBatchPower`).
    pub batch_power: i32,
    /// Each attacker in a batched attack trigger with its power when it attacked (`Amount::EventAttackersPower`).
    pub card_powers: Vec<(CardId, i32)>,
    /// The permanent that was dealt damage (IsDealtDamage triggers); `obj` is the damage source there.
    pub damaged: Option<CardId>,
    /// For a granted ability: the permanent that granted it (Sel::GrantSource).
    pub granted_by: Option<CardId>,
    /// The kind of counter in a counter-added event.
    pub counter_kind: Option<CounterKind>,
    /// A batched counter-added trigger: every kind of counter put in the batch (`Effect::AddCountersOfEventKinds`).
    pub counter_kinds: Vec<CounterKind>,
    /// A batched event's cards (the creature cards milled together): `Sel::EventCards`.
    pub cards: Vec<CardId>,
    /// The trigger fired because a permanent entered the battlefield (Strict Proctor).
    pub entering: bool,
    /// The trigger fired because its own source attacked (Firebender Ascension).
    pub attacking: bool,
    /// The stack item whose targeting fired the trigger (`Sel::TriggerStackItem`).
    pub stack: Option<StackId>,
    /// `TapsForMana`: the colors of the mana produced (bit per `Color`), for `Condition::SharesColorWithEventMana`.
    pub mana_colors: u8,
    /// Timestamp (`Game::ts`) when the triggered ability triggered (`Effect::Destroy.existing_at_trigger`).
    pub ts: u64,
    /// A delayed trigger's source when the delayed trigger was created: its zone-change counter then. A `this` condition is false once the source changed zones (a new object).
    pub this_zcc: Option<u32>,
}

/// Last known information about an object that has left the battlefield.
#[derive(Clone, Debug)]
pub struct Lki {
    pub card: CardId,
    pub derived: Derived,
    pub controller: PlayerId,
    pub counters: SmallVec<[(CounterKind, u16); 2]>,
    pub def: DefId,
    pub token: bool,
    pub face_down: bool,
    pub attached_to: Option<CardId>,
    /// Permanents that were attached to it when it left.
    pub attachments: SmallVec<[CardId; 2]>,
    /// Battlefield permanents (with their controllers) when it left, plus those that left in the same batch; only recorded for cards with `Ability::LkiSnapshot`.
    pub snapshot: Vec<(CardId, PlayerId)>,
}

#[derive(Clone, Debug)]
pub enum ItemKind {
    Spell { from: Zone, kicked: bool, alt: Option<u8> },
    Ability { ability: u16, mode: Option<u8> },
    Triggered { ability: u16, optional: bool },
    Delayed { effects: Vec<Effect> },
}

#[derive(Clone, Debug)]
pub struct StackItem {
    pub id: StackId,
    pub controller: PlayerId,
    pub source: CardId,
    pub src_zcc: u32,
    pub def: DefId,
    pub kind: ItemKind,
    pub targets: Vec<Target>,
    pub x: i32,
    pub modes: SmallVec<[u8; 3]>,
    pub ctx: EventCtx,
    pub copy: bool,
    /// For a copy of a permanent spell: changes its token gets ("except it isn't legendary").
    pub copy_except: Vec<CopyExcept>,
    /// Chosen division of damage (divide-as-you-choose) per target.
    pub divisions: SmallVec<[i32; 4]>,
    /// Number of targets chosen for each target spec, in order.
    pub spec_counts: SmallVec<[u8; 4]>,
    pub remembered: Vec<Obj>,
    /// A granted ability on the stack keeps itself, so it still resolves after the permanent that granted it is gone (sacrificed as a cost).
    pub granted_snap: Option<std::sync::Arc<Ability>>,
    /// Cards spliced onto this spell (their spell effects run after its own, with their targets after its own).
    pub spliced: Vec<DefId>,
}

#[derive(Clone, Debug)]
pub struct ContEffect {
    pub id: u32,
    pub ts: u64,
    pub source: CardId,
    pub controller: PlayerId,
    pub affects: SmallVec<[(CardId, u32); 4]>,
    pub players: SmallVec<[PlayerId; 4]>,
    pub modif: Modif,
    pub duration: Duration,
    pub created_turn: u32,
}

/// Protection a player has been granted (The One Ring): until their next turn, end of turn, or for good.
#[derive(Clone, Debug)]
pub struct PlayerProt {
    pub player: PlayerId,
    pub from: ProtectionFrom,
    pub until: Duration,
    /// Whose turn counts for `UntilYourNextTurn`.
    pub owner: PlayerId,
}

/// A damage prevention shield created by `Effect::PreventDamage`.
#[derive(Clone, Debug)]
pub struct Shield {
    pub to: Option<Vec<Obj>>,
    pub from: Option<Vec<CardId>>,
    pub combat_only: bool,
    /// Points left to prevent (None = all damage).
    pub remaining: Option<i32>,
    /// Prevented damage is dealt to this object instead.
    pub redirect: Option<Obj>,
    /// Only the next matching damage event is prevented (all of it), then the shield is gone.
    pub once: bool,
    /// Each point prevented puts a +1/+1 counter on the shielded object (Temper).
    pub counters: bool,
    /// Each point prevented to a creature puts a counter of this kind on it at the next end step (the shield's source and controller come along).
    pub end_counters: Option<(CounterKind, CardId, PlayerId)>,
    /// The shield also covers damage to this player.
    pub or_player: Option<PlayerId>,
    /// The redirected damage is dealt by this card (non-combat) instead of the original source.
    pub redirect_by: Option<CardId>,
    /// "Creatures target player controls": also covers creatures those players come to control (or that enter) later this turn (Encircling Fissure).
    pub from_ctl: Option<Vec<PlayerId>>,
}

#[derive(Clone, Debug)]
pub enum Modif {
    Pump { power: i32, toughness: i32 },
    SetPT { power: i32, toughness: i32 },
    SetP { power: i32 },
    SetT { toughness: i32 },
    Keyword(Keyword),
    RemoveKeyword(Keyword),
    RemoveProtection(ProtectionFrom),
    Control(PlayerId),
    AddTypes(TypeSet),
    RemoveTypes(TypeSet),
    SetTypes(TypeSet, Vec<Sym>),
    Colors(ColorSet),
    NoAbilities,
    CantBlock,
    CantAttack,
    /// Players: can't gain life this turn, etc.
    NoLifeGain,
    NoMonarch,
    NoLifeLoss,
    /// Players: can't cast spells for the duration.
    NoCast,
    /// This creature must block the given attacker if able.
    MustBlockCard(u32),
    /// No more spells once the player has cast this many this turn (`PlayerRestrict::OneMoreSpell`).
    SpellCap(u32),
    NoCastCreatures,
    NoCastNoncreature,
    /// Players: can't attack with creatures (`PlayerRestrict::CantAttack`).
    NoAttack,
    NoCastNotFromHand,
    NoCastInstantSorcery,
    NoActivateNonMana,
    NoLandPlay,
    NoGraveyardPlay,
    PlayOthersGraveyards,
    /// Power and toughness are switched.
    SwitchPT,
    ExtraLandDrop(u8),
    AddSubtype(Sym),
    AddColors(ColorSet),
    NotLegendary,
    SetName(Sym),
    NoManaCost,
    MustAttackPlayer(PlayerId),
    MustAttackPermanent(CardId),
    AddSupertype(Supertype),
    RemoveSupertype(Supertype),
    SetSubtypes(Vec<Sym>),
    SetSubtypesKind(crate::dsl::SubtypeKind, Vec<Sym>),
    Protection(ProtectionFrom),
    /// A layered static effect (can't block, can't be blocked by ..., must attack, extra blockers) applied to the affected cards for a duration.
    Static(Box<crate::dsl::StaticEffect>),
    /// Like `Static`, but the static's own `affects` filter is evaluated continuously (also for permanents that arrive later).
    DynStatic(Box<crate::dsl::StaticEffect>),
}

#[derive(Clone, Debug)]
pub struct DelayedTrigger {
    pub id: u32,
    pub controller: PlayerId,
    pub source: CardId,
    pub when: DelayedWhen,
    pub effects: Vec<Effect>,
    pub created_turn: u32,
    pub targets: Vec<Target>,
    pub ctx: EventCtx,
    pub remembered: Vec<Obj>,
}

/// A temporary triggered ability created by Effect::AddTrigger.
#[derive(Clone, Debug)]
pub struct TempTrigger {
    pub trigger: Trigger,
    pub once: bool,
    pub until: TempUntil,
    /// When non-empty, only events about one of these permanents fire it.
    pub bound: Vec<CardId>,
    /// A dormant triggered ability of the source card that goes on the stack (with its own targets) when this fires.
    pub ability: Option<u16>,
    /// Intervening "if" checked when the trigger event happens.
    pub cond: Option<Condition>,
    pub d: DelayedTrigger,
}

#[derive(Clone, Debug)]
pub struct PendingTrigger {
    pub controller: PlayerId,
    pub source: CardId,
    pub src_zcc: u32,
    pub def: DefId,
    pub ability: u16,
    pub ctx: EventCtx,
    pub ts: u64,
    /// The granted ability as it was when the trigger fired (the source may have changed zones before the trigger goes on the stack).
    pub granted: Option<std::sync::Arc<Ability>>,
}

#[derive(Clone, Debug)]
pub struct AttackInfo {
    pub card: CardId,
    pub defender: Obj,
    pub blocked: bool,
    pub blockers: SmallVec<[CardId; 2]>,
    /// Banding: attackers with the same non-zero band number attack as a band.
    pub band: u8,
}

#[derive(Clone, Debug, Default)]
pub struct Combat {
    pub attackers: Vec<AttackInfo>,
    pub first_strike_done: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Config {
    pub starting_life: i32,
    pub hand_size: u8,
    pub commander: bool,
    pub mulligan: MulliganRule,
    pub commander_damage: i32,
}

/// A face of the planar die: one chaos face, one planeswalk face, four blank faces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum PlanarFace {
    Chaos,
    Planeswalk,
    Blank,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum MulliganRule {
    #[default]
    London,
    /// London, but the first mulligan is free.
    LondonFreeFirst,
    None,
    /// Mulligan as often as you like: always a fresh full hand, nothing goes to the bottom (casual / testing).
    Free,
    /// One card fewer each time; keeping with fewer cards than the starting hand size lets you scry 1.
    Vancouver,
    /// One card fewer each time, no scry.
    Paris,
    /// Mulligan as often as you like, scrying the top three cards of the library while deciding (put any on the bottom,
    /// order the rest); mulligan → the hand and the cards still on top go to the bottom and a fresh seven is drawn.
    FreeScry3,
}

impl Config {
    pub fn standard() -> Config {
        Config { starting_life: 20, hand_size: 7, commander: false, mulligan: MulliganRule::London, commander_damage: 21 }
    }
    pub fn commander() -> Config {
        Config { starting_life: 40, hand_size: 7, commander: true, mulligan: MulliganRule::LondonFreeFirst, commander_damage: 21 }
    }
}

/// A deck to start a game with.
#[derive(Clone, Debug)]
pub struct DeckSpec {
    pub name: String,
    pub main: Vec<DefId>,
    pub commanders: Vec<DefId>,
    pub sideboard: Vec<DefId>,
    /// The sideboard card (by definition) that is the player's chosen companion; it must also appear in `sideboard`.
    pub companion: Option<DefId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "t")]
#[ts(export)]
pub enum Event {
    GameStart,
    /// A message for a player: the rules changed or refused something they asked for (a block that was ignored, an attacker that is forced to attack ...).
    Notice { player: PlayerId, text: String },
    TurnBegin { turn: u32, player: PlayerId },
    StepBegin { step: Step, player: PlayerId },
    Moved { card: CardId, from: Zone, to: Zone },
    Tapped { card: CardId },
    Regenerated { card: CardId },
    Untapped { card: CardId },
    Damage { source: Option<CardId>, target: Obj, amount: i32, combat: bool },
    Life { player: PlayerId, delta: i32 },
    Drew { player: PlayerId, card: CardId },
    Cast { card: CardId, player: PlayerId },
    /// A spell was copied (by `player`, who controls the copy); `card` is the copied spell's card.
    Copied { card: CardId, player: PlayerId },
    /// `life`: the life paid as part of the activation cost (Verrak, Warped Sengir reads it as `Amount::EventAmount`).
    Activated { card: CardId, ability: u16, player: PlayerId, life: i32, mana: bool },
    /// A triggered ability was put on the stack; `entering` = it triggered because a permanent entered.
    AbilityTriggered { id: StackId, source: CardId, player: PlayerId, entering: bool, attacking: bool },
    /// A spell or ability was put onto the stack (spells, activated and triggered abilities); `targets` = how many targets it has.
    StackPut { id: StackId, targets: u8 },
    Saddled { card: CardId },
    Resolved { card: CardId },
    /// A spell was countered; `by` = the controller of the spell or ability that countered it (None: ward / other).
    Countered { card: CardId, by: Option<PlayerId> },
    Fizzled { card: CardId },
    Attacked { card: CardId, defender: Obj },
    Blocked { blocker: CardId, attacker: CardId },
    Unblocked { card: CardId },
    /// An attacking creature became blocked without a blocker (Choking Vines).
    BecameBlocked { attacker: CardId },
    Counters { card: CardId, kind: CounterKind, delta: i32, by: Option<PlayerId> },
    Token { card: CardId },
    Shuffled { player: PlayerId },
    Lost { player: PlayerId, reason: String },
    Over { result: GameResult },
    Attach { card: CardId, to: Option<CardId> },
    Revealed { card: CardId },
    Scry { player: PlayerId, n: u32, surveil: bool, bottom: u32 },
    Mana { player: PlayerId, mana: ManaType },
    Control { card: CardId, player: PlayerId },
    Transformed { card: CardId },
    /// A permanent entered the battlefield as its back face ("enters transformed").
    EnteredTransformed { card: CardId },
    ControlLost { card: CardId, from: PlayerId, to: PlayerId },
    TurnedFaceUp { card: CardId },
    Mulligan { player: PlayerId, keep: bool },
    LandPlayed { card: CardId, player: PlayerId, from_hand: bool, from: Zone },
    Monarch { player: PlayerId },
    Discarded { card: CardId, player: PlayerId, by: Option<PlayerId> },
    /// A permanent was tapped for mana (after the mana was added).
    ManaTapped { card: CardId, player: PlayerId },
    /// A card was cycled (discarded to pay a "discard this card" activation).
    Cycled { card: CardId, player: PlayerId },
    /// A permanent became the target of a spell or ability from `by`.
    Targeted { card: CardId, by: CardId, controller: PlayerId, spell: bool, sid: StackId },
    /// A player became the target of a spell or ability (`by` = its source card).
    PlayerTargeted { player: PlayerId, by: CardId, controller: PlayerId, spell: bool, sid: StackId },
    Crime { player: PlayerId },
    Unlocked { card: CardId, door: u8 },
    FullyUnlocked { card: CardId },
    PhasedOut { card: CardId },
    PhasedIn { card: CardId },
    /// Day became night (`day: false`) or night became day (`day: true`).
    DayNightChanged { day: bool },
    Sacrificed { card: CardId, player: PlayerId },
    /// A creature with exploit sacrificed a creature as it entered.
    Exploited { card: CardId, victim: Option<CardId> },
    KeywordAction { card: CardId, action: crate::dsl::KeywordAction, other: Option<CardId> },
    Monstrous { card: CardId, n: i32 },
    Mutated { card: CardId },
    /// A card was plotted (exiled with the plotted mark).
    Plotted { card: CardId },
    Foretold { card: CardId },
    /// A creature was exerted (by the player who controls it).
    Exerted { card: CardId },
    /// A Saga's final chapter ability finished resolving (the Saga is `card`).
    SagaFinalChapter { card: CardId },
    /// A player chose modes for a spell or ability (Star Trek: "you face a dilemma").
    FacedDilemma { player: PlayerId },
    Renowned { card: CardId },
    /// `player` took part in a clash and won (or lost) it.
    Clashed { player: PlayerId, won: bool },
    VotingFinished { player: PlayerId },
    /// A creature explored; `revealed` is the card revealed (None with an empty library).
    Explored { card: CardId, revealed: Option<CardId> },
    /// A player investigated (`first`: it was their first investigate this turn).
    Investigated { player: PlayerId, first: bool },
    /// A player collected evidence (paid a collect-evidence cost).
    EvidenceCollected { player: PlayerId },
    /// A player paid life as a cost.
    LifePaid { player: PlayerId, n: i32 },
    /// A creature connived (drew, discarded and maybe got a counter).
    Connived { card: CardId },
    EnergyGained { player: PlayerId, n: i32 },
    /// A player discovered N (`Effect::Discover`).
    Discovered { player: PlayerId, n: i32 },
    /// Counters were put on (or given to) a player (poison, energy, experience or rad); `by` is the player whose spell or ability did it.
    PlayerCounters { player: PlayerId, kind: CounterKind, delta: i32, by: Option<PlayerId> },
    DieRolled { player: PlayerId, n: i32, sides: i32 },
    /// Planechase: the planar die was rolled (face: chaos, planeswalk or blank).
    PlanarDieRolled { player: PlayerId, face: PlanarFace },
    /// Planechase: chaos ensued ({CHAOS} rolled, or an effect said so); the face-up planes' chaos abilities trigger.
    ChaosEnsued { player: PlayerId },
    /// Planechase: the planar controller is about to planeswalk away from these planes.
    PlaneswalkingAway { player: PlayerId, from: Vec<CardId> },
    /// Planechase: the planar controller planeswalked to this plane or phenomenon.
    Planeswalked { player: PlayerId, to: CardId },
    /// Archenemy: a scheme was set in motion (its name is public from now on, though a one-shot scheme is face down again
    /// at the bottom of its deck by the time the players see the next view).
    SchemeSetInMotion { player: PlayerId, card: CardId, name: String },
    /// What a player chose with `Effect::EachPlayerChooses`, revealed once every player has chosen (`option`: a named choice).
    ChoiceRevealed { player: PlayerId, source: CardId, cards: Vec<CardId>, option: Option<String> },
    /// A player finished one roll of one or more dice (after every `DieRolled` of that roll).
    DiceRolled { player: PlayerId },
    /// A player gave a gift: a spell cast with its gift promised (an empty `Kicker`) resolved.
    GiftGiven { player: PlayerId },
    Milled { player: PlayerId, cards: Vec<CardId> },
    /// A player manifested dread; `cards` are the ones put into the graveyard.
    ManifestedDread { player: PlayerId, cards: Vec<CardId> },
    /// A player waterbent, earthbent, firebent or airbent.
    Bent { player: PlayerId, kind: crate::dsl::BendKind },
    CoinFlipped { player: PlayerId, won: bool },
    Proliferated { player: PlayerId },
    Crewed { card: CardId, player: PlayerId },
    /// A creature was tapped to pay a Crew (`saddle: false`) or Saddle (`true`) cost of `vehicle`.
    CrewPaid { creature: CardId, vehicle: CardId, saddle: bool },
    /// A player started searching a library.
    Searched { player: PlayerId },
    Unattached { card: CardId, from: CardId },
    /// The Ring tempted `player`.
    RingTempted { player: PlayerId },
    /// `player` took the initiative.
    InitiativeTaken { player: PlayerId },
    /// `player` entered the room `name` of the dungeon card `dungeon`.
    RoomEntered { player: PlayerId, dungeon: CardId, name: String },
    /// `player` completed a dungeon.
    DungeonCompleted { player: PlayerId },
    /// `player` chose `card` as their Ring-bearer.
    RingBearerChosen { player: PlayerId, card: CardId },
    /// `player` has now spent `n` total mana this turn ("expend N" triggers once per number crossed).
    Expended { player: PlayerId, n: u32 },
    Log { text: String },
}

/// The control-flow state: where the game is, so a clone can resume from any decision.
#[derive(Clone, Debug)]
pub enum Run {
    /// Mulligans before turn 1.
    PreGame { stage: PreGame },
    /// Entering a step: turn-based actions, then priority (or straight on).
    BeginStep,
    Priority { player: PlayerId, passes: u8 },
    Cast(Box<crate::cast::CastCtx>),
    Resolve(Box<crate::resolve::ResolveCtx>),
    Attackers,
    Blockers { idx: usize },
    DamageAssign(Box<crate::combat::DamageCtx>),
    Cleanup,
    /// Asking for an as-enters choice; priority resumes for `player` afterwards.
    EnterChoice { card: CardId, kind: crate::dsl::ChooseKind, options: Vec<String>, player: PlayerId, passes: u8 },
    /// Asking whether to pay life for a shock land.
    EnterPay { card: CardId, life: i32, player: PlayerId, passes: u8 },
    EnterMayTap { card: CardId, player: PlayerId, passes: u8 },
    /// Asking whether to move a card on to where an optional zone-change replacement allows (Library of Leng).
    OptZone { card: CardId, instead: Zone, bottom: bool, player: PlayerId, passes: u8 },
    /// The owner picks which of several zone-change replacements applied (its index in `cands`).
    ZoneChoice { card: CardId, cands: Vec<(CardId, PlayerId, Vec<Effect>)>, player: PlayerId, passes: u8 },
    /// Asking which permanent gets the counter of `EntersCounterOnChosen`.
    EnterCounter { card: CardId, options: Vec<CardId>, player: PlayerId, passes: u8 },
    /// Asking which graveyard cards (`options` non-empty) or which counter kind (`options` empty, `left` still to choose) for `EntersExileXCounters`.
    EnterExile { card: CardId, options: Vec<CardId>, left: u8, player: PlayerId, passes: u8 },
    /// Asking which graveyard cards to exile for `EntersExileAny`.
    EnterExileAny { card: CardId, options: Vec<CardId>, player: PlayerId, passes: u8 },
    /// Asking whether to pay the as-enters cost of `EntersUnlessCost`.
    EnterUnless { card: CardId, player: PlayerId, passes: u8 },
    /// Asking whether to reveal a card from hand for a reveal-land.
    EnterReveal { card: CardId, player: PlayerId, passes: u8 },
    /// Asking which permanent a Clone enters as a copy of.
    EnterCopy { card: CardId, options: Vec<CardId>, player: PlayerId, passes: u8 },
    /// Asking which creature a just-turned-face-up Aura attaches to (`Ability::TurnedFaceUpMayAttach`).
    EnterAttach { card: CardId, options: Vec<CardId>, player: PlayerId, passes: u8 },
    /// Untap step: asking whether to keep optional-untap permanents tapped, one at a time (`asked`: the first card's question is pending).
    UntapChoose { queue: Vec<CardId>, asked: bool },
    /// Draw step: asking whether to skip it (Fasting); `gain` life if so.
    DrawSkipAsk { gain: i32 },
    /// Untap step: the active player is choosing the permanent type that can untap (`StaticEffect::UntapOnlyChosenType`).
    UntapPickType { options: Vec<CardType> },
    UntapLimitPick { ids: Vec<CardId> },
    /// Asking which player a permanent that just transformed into a Curse is attached to (`Ability::AttachPlayerOnTransform`); `resume`: the priority state to return to (None: the start of a step).
    AttachPlayerAsk { card: CardId, options: Vec<PlayerId>, resume: Option<(PlayerId, u8)> },
    /// An Aura put onto the battlefield without being cast, with several legal permanents to enchant: its controller chooses.
    AttachHostAsk { card: CardId, options: Vec<CardId>, resume: Option<(PlayerId, u8)> },
    /// The attack declaration doesn't meet a requirement and there is more than one way to meet it: the attacking player
    /// chooses (`card` None: which creature attacks for "attack with at least one creature"; Some: what that creature attacks).
    AttackFixAsk { attacks: Vec<(CardId, Obj)>, card: Option<CardId>, options: Vec<Obj> },
    /// Asking which player gets the mana of a `player_choice` mana ability that `player` is tapping by hand.
    ManaGiftAsk { card: CardId, ability: u16, produce: Option<crate::mana::ManaType>, options: Vec<PlayerId>, player: PlayerId, passes: u8 },
    /// Asking which cards a mana ability that `player` is tapping by hand exiles from the graveyard (Sunken Palace).
    ManaPickAsk { card: CardId, ability: u16, produce: Option<crate::mana::ManaType>, options: Vec<CardId>, player: PlayerId, passes: u8 },
    Over,
}

#[derive(Clone, Debug)]
pub enum PreGame {
    /// Colour choices of commanders with `Ability::PregameChooseColor`, per seat, before the mulligans.
    Color { idx: usize },
    Mulligan { idx: usize },
    Bottom { idx: usize, n: u8 },
    /// Vancouver mulligan: scry 1 after keeping a smaller hand (seat `idx` in turn order).
    Scry { idx: usize },
    /// Serum Powder: asked before the keep decision of seat `idx`.
    Powder { idx: usize },
    /// Cards with `PregameSideboardExile`: the yes / no question for the `k`-th such card in the library of seat `idx`.
    Sideboard { idx: usize, k: usize },
    /// ... and the choice of sideboard cards to exile.
    SideboardPick { idx: usize, k: usize },
    /// Gemstone Caverns: begin the game with it on the battlefield (seat `idx`).
    Begin { idx: usize },
    /// Gemstone Caverns: exile a card from the hand (seat `idx`).
    BeginExile { idx: usize },
}

#[derive(Clone)]
pub struct Game {
    pub db: Arc<CardDb>,
    pub cfg: Config,
    pub players: Vec<Player>,
    pub cards: Vec<Card>,
    pub battlefield: Vec<CardId>,
    pub stack: Vec<StackItem>,
    /// Definitions of spells cast this turn, kept after they leave the stack (a countered spell's mana value is still known).
    pub stack_lki: Vec<(StackId, DefId)>,
    /// The card behind each countered / resolved spell in `stack_lki` (so its mana spent and so on can still be read).
    pub stack_lki_src: Vec<(StackId, CardId)>,
    /// The announced X of each spell in `stack_lki` (its mana value as it last existed on the stack).
    pub stack_lki_x: Vec<(StackId, i32)>,
    /// Every spell cast this turn as it was put on the stack (a copy can be made of a spell that was countered before the copy trigger resolved).
    pub stack_lki_items: Vec<StackItem>,
    /// The ChangeTargets effect being resolved must change the target (Swerve).
    pub retarget_must_change: std::cell::Cell<bool>,
    /// `ChangeTargets.random`: new targets are picked at random, nobody is asked.
    pub retarget_random: std::cell::Cell<bool>,
    pub retarget_exclude: std::cell::Cell<bool>,
    pub retarget_only: std::cell::Cell<Option<Obj>>,
    pub retarget_forced: std::cell::Cell<bool>,
    /// Stack items still waiting to be retargeted by a `ChangeTargets { each_stack: true }` (popped from the end).
    pub retarget_queue: std::cell::RefCell<Vec<StackId>>,
    /// How many cards the last `RevealUntil` revealed (`Amount::RevealedCount`: Madcap Experiment).
    pub last_reveal_count: std::cell::Cell<i32>,
    /// The spell whose cost a `CostChange.per` amount is being evaluated for (`Amount::SameNameInGraveyardAsSpell`).
    pub costing_card: std::cell::Cell<Option<CardId>>,
    pub retarget_players_only: std::cell::Cell<bool>,
    pub retarget_filter: std::cell::RefCell<Option<FilterBox>>,
    pub turn: u32,
    pub active: PlayerId,
    pub step: Step,
    /// Turns already queued (extra turns), most recent last.
    pub extra_turns: Vec<PlayerId>,
    /// Positions in `extra_turns` of turns after which their player loses the game (Final Fortune).
    pub extra_lose_marks: Vec<usize>,
    /// The player who loses the game at the beginning of this turn's end step.
    pub lose_end_turn: Option<PlayerId>,
    /// Whose regular (non-extra) turn came last; turn order resumes after this player when extra turns end.
    pub regular_active: PlayerId,
    /// The current turn is an extra turn (also true when the extra turn is taken by the player who just had a turn).
    pub in_extra_turn: bool,
    pub rng: ChaCha8Rng,
    pub ts: u64,
    pub next_stack_id: u32,
    pub next_effect_id: u32,
    pub effects: Vec<ContEffect>,
    pub delayed: Vec<DelayedTrigger>,
    pub pending_triggers: Vec<PendingTrigger>,
    /// Creatures that dealt combat damage to a player this turn (freerunning, "if an Assassin dealt combat damage to a player this turn").
    pub combat_damagers: Vec<CardId>,
    /// Each (creature, its controller at the time) that dealt combat damage to a player this turn, even if it has since left the battlefield.
    pub combat_damagers_ctl: Vec<(CardId, PlayerId)>,
    /// Creatures that will adapt as though they had no +1/+1 counters the next time they adapt this turn.
    pub adapt_ignore: Vec<CardId>,
    /// (attacking player, defending player) pairs barred this turn by `Effect::CantAttackPlayerThisTurn`.
    pub no_attack_pairs: Vec<(PlayerId, PlayerId)>,
    /// (player, right) restrictions from `Effect::AttackOnlyNearestUntilNextTurn`: until that player's next turn every player may attack only their nearest opponent in that direction.
    pub attack_nearest: Vec<(PlayerId, bool)>,
    /// Pairs (attacking player, defending player) that also can't attack the defender's planeswalkers this turn.
    pub no_attack_pw_pairs: Vec<(PlayerId, PlayerId)>,
    /// (attacking player, effect controller, effect source, filter): the attacker can't attack permanents matching the filter this turn (`CantAttackPlayerThisTurn.only_permanents`).
    pub no_attack_perms: Vec<(PlayerId, PlayerId, CardId, Filter)>,
    /// (attacking player, defending player, active): barred from attacking during the attacking player's next turn (`Effect::CantAttackYouNextTurn`); becomes active when that turn begins and is dropped when it ends.
    pub no_attack_next: Vec<(PlayerId, PlayerId, bool)>,
    /// (attacker, defender) pairs barred until the defender's next turn begins (`Effect::CantAttackYouUntilYourNextTurn`).
    pub no_attack_pending: Vec<(PlayerId, PlayerId)>,
    /// Players who can't be attacked (themselves, not their planeswalkers) except by creatures with flying and/or islandwalk until their next turn begins (`Effect::SanctuaryUntilYourNextTurn`).
    pub sanctuary: Vec<PlayerId>,
    /// The active player declined to skip their draw step (Fasting); consumed when the draw step begins again.
    pub draw_skip_declined: bool,
    /// The untap-step type choice (`UntapOnlyChosenType`) was just made: the re-entered step must not ask again.
    pub untap_pick_done: bool,
    pub untap_limit_done: bool,
    pub untap_prefer: Option<Vec<CardId>>,
    /// This untap step only untaps permanents of this type (`UntapOnlyChosenType`).
    pub untap_only: Option<CardType>,
    /// (player whose creatures must attack, player they must attack, turn): `Effect::TauntAllCreatures`.
    pub taunts: Vec<(PlayerId, PlayerId, u32)>,
    /// (player whose creatures must all attack, turn): `Effect::AttackAllThisTurn`.
    pub attack_all: Vec<(PlayerId, u32)>,
    /// (controlled player, controller): `Effect::ControlPlayerNextTurn` waiting for the controlled player's next turn.
    pub next_turn_ctl: Vec<(PlayerId, PlayerId, bool)>,
    /// (controlled player, controller) for the turn in progress: decisions of the controlled player are surfaced to the controller.
    pub turn_ctl: Option<(PlayerId, PlayerId)>,
    /// The controlled turn lasts only for the combat phase (`ControlPlayerNextTurn { combat_only }`).
    pub turn_ctl_combat: bool,
    /// (controlled player, controller) while a `ControlPlayerDuring` effect resolves.
    pub resolve_ctl: Option<(PlayerId, PlayerId)>,
    /// The player who can use only the mana abilities of lands (Word of Command, while they play the chosen card).
    pub land_mana_only: Option<PlayerId>,
    /// The text-change question being asked (for the AI).
    pub text_ask: Option<crate::textchange::TextAsk>,
    /// (spell, controlled player, controller): the controller controls the player while that spell resolves (Word of Command).
    pub spell_ctl: Vec<(CardId, PlayerId, PlayerId)>,
    /// The spell whose resolution `resolve_ctl` was set for.
    pub resolve_ctl_item: Option<CardId>,
    /// (card, controlled player, controller): a cast forced by Word of Command that has not been made yet; the control lasts until it is.
    pub wok_pending: Option<(CardId, PlayerId, PlayerId)>,
    /// (decision_log length, controlled player) of the pending decision that was handed to the controller of a controlled turn.
    pub remapped_at: Option<(usize, PlayerId)>,
    /// (source, player, turn): `Effect::IgnoreSourceEffect`.
    pub effect_ignored: Vec<(CardId, PlayerId, u32)>,
    /// Last turn number through which creatures can't attack / players and permanents can't be targeted by spells or activated abilities (`Effect::LockTurns`; 0 = none).
    pub no_attack_until: u32,
    pub no_target_until: u32,
    /// Set while choosing or checking targets of a triggered ability (exempt from `no_target_until`).
    pub target_trig: std::cell::Cell<bool>,
    /// Permanents whose effects are ignored until end of turn (Volrath's Curse: `Effect::IgnoreThisUntilEndOfTurn`, read by `Condition::IgnoredUntilEndOfTurn`).
    pub ignored_until_eot: Vec<CardId>,
    /// (defending player, player who declares that player's blockers) for this turn.
    pub block_chooser: Vec<(PlayerId, PlayerId)>,
    /// The player who declares the active player's attackers this turn instead of them (Master Warcraft).
    pub attack_chooser: Option<PlayerId>,
    /// Every damage event this turn (after prevention), for "was dealt damage this turn" / "dealt damage to you this turn" / excess damage filters.
    pub dmg_log: Vec<DmgRec>,
    /// Players dealt noncombat damage during the previous turn (Command the Stage).
    pub noncombat_dmg_last: Vec<PlayerId>,
    /// Every damage event of the whole game by a source card: (source, target, combat). Never cleared ("a player it has dealt damage to this game").
    pub dmg_game: Vec<(CardId, Obj, bool)>,
    /// How many times each activated / triggered ability of each source has begun resolving this turn ("if this is the second time this ability has resolved this turn").
    pub ability_res: std::collections::HashMap<(CardId, u16), u32>,
    /// The count for the ability currently resolving (including this resolution).
    pub cur_ability_res: u32,
    pub temp_triggers: Vec<TempTrigger>,
    pub combat: Option<Combat>,
    pub run: Run,
    pub pending: Option<Decision>,
    pub answer: Option<Choice>,
    pub events: Vec<Event>,
    pub over: Option<GameResult>,
    pub layers_dirty: bool,
    /// Sources of static abilities whose effects already started to apply in an earlier layer of this pass; they keep applying in later layers even if the ability is removed meanwhile (rule 613.6: Humility, Opalescence).
    pub layer_started: std::cell::RefCell<Vec<CardId>>,
    /// Cards outside the battlefield currently carry an extra chosen creature type (reset on the next recompute).
    pub zone_sub_dirty: bool,
    pub monarch: Option<PlayerId>,
    pub initiative: Option<PlayerId>,
    pub day_night: Option<bool>,
    /// Damage can't be prevented during this turn number (0 = never).
    pub no_prevent_turn: u32,
    /// Bands the attacking player wants to form with the next attack declaration (banding); consumed by it.
    pub attack_bands: Vec<Vec<CardId>>,
    /// Void: a nonland permanent left the battlefield this turn / a spell was warped this turn.
    pub nonland_left_turn: bool,
    pub warped_this_turn: bool,
    /// (source of a once-per-turn granted alternative cost, turn) already used.
    pub alt_grants_used: Vec<(CardId, u32)>,
    /// Result of the most recent die roll.
    pub last_die: i32,
    /// Planechase: the shared planar deck (top = last), face down outside every zone list. The face-up plane(s) are in
    /// the planar controller's (active player's) command zone; `planes` lists them.
    pub planar_deck: Vec<CardId>,
    pub planes: Vec<CardId>,
    /// Chaotic Aether: each blank roll of the planar die is a {CHAOS} roll until a player planeswalks away from a plane.
    pub chaotic_aether: bool,
    /// `turn + 1` of the turn in which cards can't enter the battlefield from exile (Bad Wolf Bay); 0 = never.
    pub no_enter_from_exile_turn: u32,
    /// Fixed Point in Time: players whose next turn ends the effect; while non-empty a planeswalk roll of the planar die is chaos instead.
    pub fixed_point_until: Vec<PlayerId>,
    /// Players that creatures can't attack until a player planeswalks away from a plane (Agyrem).
    pub no_attack_until_planeswalk: Vec<PlayerId>,
    /// Players who can't lose (and whose opponents can't win) until a player planeswalks away from a plane (Celestine Reef).
    pub safe_until_planeswalk: Vec<PlayerId>,
    /// Archenemy: the archenemy already set a scheme in motion this turn (the turn-based action happens once a turn).
    pub scheme_this_turn: bool,
    /// Archenemy: schemes can't be set in motion during the next turn (All in Good Time); moved into `scheme_this_turn` when it starts.
    pub scheme_ban_next: bool,
    /// Archenemy: players whose next scheme set in motion becomes three (Plots That Span Centuries); one entry per use.
    pub scheme_triple: Vec<PlayerId>,
    /// Planechase: the planes and phenomena the planar controller planeswalked to this turn (Unyaro).
    pub planeswalked_to_this_turn: Vec<CardId>,
    /// Archenemy is on (scheme decks were set up): one-shot schemes are checked by the state-based actions.
    pub archenemy: bool,
    /// Chosen / other result of the last `Effect::RollTwoChooseOne`.
    pub roll_chosen: i32,
    pub roll_other: i32,
    /// While costing a face-down cast, the card being cast (it counts as face down for cost-changing filters).
    pub face_down_cast: std::cell::Cell<Option<CardId>>,
    /// The card whose cast cost is being worked out from a non-stack zone: `Filter.cast_from` reads the zone it is in now, not a stale `cast_from` (Aven Interrupter's tax on a plotted or flashed-back spell).
    pub cost_probe: std::cell::Cell<Option<CardId>>,
    /// A Move with `attacking` is in progress: the permanent joins combat before its enters event so triggers see it attacking.
    pub entering_attacking: std::cell::Cell<bool>,
    /// Set while a Move with `enters_bare` runs; the moved card is recorded in `bare_entered`.
    pub entering_bare: std::cell::Cell<bool>,
    /// (card, zcc) of permanents that entered with no abilities: their own enters triggers don't fire.
    pub bare_entered: std::cell::RefCell<Vec<(CardId, u32)>>,
    /// The source of a replacement that sent a card to exile ("exile it instead"): the card counts as exiled with that permanent.
    pub repl_exile_src: std::cell::Cell<Option<CardId>>,
    /// (creature tapped, Vehicle/Mount it crewed or saddled, turn): who paid Crew / Saddle costs.
    pub crew_log: Vec<(CardId, CardId, u32)>,
    /// (permanent tapped, source whose ability cost it paid via Cost::TapPermanents, turn).
    pub tap_cost_log: Vec<(CardId, CardId, u32)>,
    /// The spell card being put into its final zone as it finishes resolving (its own `Replace` abilities from the stack still apply).
    pub resolving_spell: std::cell::Cell<Option<CardId>>,
    /// The spell whose effects are running right now and its chosen targets (it has left the stack while it resolves): filters that read "a spell with a single target" still see it.
    pub exec_spell: std::cell::RefCell<Option<(CardId, Vec<Target>)>>,
    /// The resolving spell is being put away after resolving normally (not countered, not fizzled).
    pub resolved_ok: std::cell::Cell<bool>,
    /// Mana types the most recent mana ability produced (read by `ManaProduce::OneProduced`).
    pub last_tapped_mana: Vec<crate::mana::ManaType>,
    /// Total amount of mana the latest mana ability produced (after multipliers); read as the `EventAmount` of `TapsForMana`.
    pub last_tapped_mana_n: i32,
    /// Result of the most recent coin flip (true = won), read by `Condition::LastFlipWon`.
    pub last_flip_won: bool,
    /// Coin flips won / any flip lost since the last `Effect::ResetFlipTally` (`Amount::FlipsWon`, `Condition::FlipLost`).
    pub flip_wins: u32,
    pub flip_lost: bool,
    /// Whether the most recent `Effect::MayPay` was paid, read by `Condition::LastCostPaid`.
    pub last_cost_paid: bool,
    /// Cards whose damage this turn stops regeneration of the damaged creature, with the turn (`Effect::CantRegenerateDamagedBy`).
    pub no_regen_dealers: Vec<(CardId, u32)>,
    /// Life each player paid in the last `Effect::PayLifeRounds`, by player index.
    pub life_rounds: Vec<i32>,
    /// Coin flips since the last `ResetFlipTally`.
    pub flip_count: i32,
    /// What each player recorded with `Effect::PayTally`, by player index.
    pub pay_tally: Vec<i32>,
    /// A `StaticEffect::ColorlessEverywhere` permanent is on the battlefield.
    pub colorless_all: bool,
    /// Colors (bit mask) a `StaticEffect::ColorAddedEverywhere` permanent adds to every card outside the battlefield and every spell.
    pub color_all: u8,
    /// Bit mask of players whose creature spells and creature cards outside the battlefield are also artifacts (`StaticEffect::CreaturesAreArtifactsEverywhere`).
    pub artifact_creatures: u8,
    /// Bit per player: a `NonlandCardsWhiteOutsideBattlefield` permanent of theirs is on the battlefield (nonland cards they own outside the battlefield are white).
    pub white_nonland: u8,
    /// The effect being resolved belongs to a triggered ability (for `StaticEffect::TriggersCantSacExileOwnTokens`).
    pub cause_trigger: std::cell::Cell<bool>,
    pub first_player: PlayerId,
    /// `Effect::RestartGame` waiting for the current step to finish: (the player who restarts it, the non-Aura permanent cards exiled with the source).
    pub restart_pending: Option<(PlayerId, Vec<CardId>)>,
    /// Cards that go onto the battlefield under that player's control as the restarted game begins.
    pub restart_return: Vec<(PlayerId, CardId)>,
    pub lands_entered_turn: u32,
    /// Permanents that entered the battlefield this turn: (controller, card types). Survives them leaving.
    pub entered_log: Vec<(PlayerId, TypeSet, Vec<Sym>, bool, CardId, bool)>,
    /// Creatures declared as attackers this turn (their controllers).
    pub attackers_log: Vec<(PlayerId, Vec<Sym>, bool)>,
    /// Controllers of the tokens declared as attackers this turn (`Amount::AttackedThisTurn.token`).
    pub token_attackers_log: Vec<PlayerId>,
    /// Players who attacked with a commander this turn (one entry per attacking commander).
    pub commander_attackers_log: Vec<PlayerId>,
    /// Same attackers with the player each attacked (or the controller of the attacked planeswalker/battle): (attacker controller, defender, subtypes, changeling).
    pub defended_log: Vec<(PlayerId, PlayerId, Vec<Sym>, bool)>,
    /// Counters put on permanents this turn: (permanent, its controller, kind, who put them, was a creature).
    pub counters_log: Vec<(CardId, PlayerId, CounterKind, Option<PlayerId>, bool, i32)>,
    pub creatures_died_turn: u32,
    /// Spells / abilities this turn that made players pile or guess (`Effect::NotePileOrGuess`).
    pub pile_guess_turn: u32,
    /// The current element of the innermost running `ForEach` (`Sel::EachObject`).
    pub each_current: Vec<Obj>,
    /// The picks of the last `Effect::EachPlayerChooses`, per chooser (read with `Sel::ChosenByEach` while its `then` runs).
    pub hidden_picks: Vec<(PlayerId, Vec<CardId>)>,
    /// Each player's last named choice made with `Effect::EachPlayerChooses { options, remember: true }`: (source card name, player, option index).
    /// Kept for the whole game so a plane keeps "last chose" across planeswalking away and back (Two Streams Facility).
    pub player_choices: Vec<(String, PlayerId, u8)>,
    /// The emblem "The Initiative" that carries its abilities.
    pub initiative_emblem: Option<CardId>,
    /// Vote tallies of the last `Effect::Vote`.
    pub last_votes: Vec<i32>,
    /// Votes each card received in the last `Effect::VoteForCards`.
    pub last_card_votes: Vec<(CardId, i32)>,
    /// Numbers recorded per player by `ChooseNumber { record }`.
    pub player_numbers: Vec<Option<i32>>,
    /// (player choosing every vote, turn): `Effect::ControlVotesThisTurn`.
    pub vote_control: Vec<(PlayerId, u32)>,
    /// Who voted for which option (index) in the last vote.
    pub last_ballots: Vec<(PlayerId, usize)>,
    /// Creatures that died this turn, by their controller when they died.
    pub creatures_died_by: Vec<u32>,
    /// Cards that left each player's graveyard this turn (index = owner).
    pub left_graveyard_by: Vec<u32>,
    /// Creature cards that left each player's graveyard this turn.
    pub left_graveyard_creature_by: Vec<u32>,
    /// Delayed trigger id -> zone-change counts of its remembered cards when it was created (a card that changed zones since is a new object).
    pub delayed_rem_zcc: Vec<(u32, Vec<(u32, u32)>)>,
    /// Cards put into a graveyard this turn: (card, its zone-change counter then, owner, zone it came from).
    pub gy_entries: Vec<(CardId, u32, PlayerId, Zone)>,
    /// Cards put into exile this turn (non-token), with their owner.
    pub exile_entries: Vec<(CardId, PlayerId)>,
    /// Mana value of each non-token card put into exile this turn, in order (Bell Borca, Spectral Sergeant: the numbers noted).
    pub exile_mv_log: Vec<i32>,
    /// Length of `gy_entries` when the current turn's combat began (Storrev).
    pub combat_gy_start: usize,
    /// Card types (bit set) of each permanent sacrificed this turn as it was when sacrificed, with the sacrificing player (tokens included).
    pub sacrificed_types: Vec<(PlayerId, u32)>,
    /// Bit per player: tapped a land for mana this turn (Desolation).
    pub land_mana_turn: u32,
    /// Bit per player: a noncreature permanent they controlled was destroyed this turn by a spell or ability an opponent controlled (Cobra Trap).
    pub nc_destroyed_by_opp: u32,
    /// Permanents sacrificed this turn: (card, sacrificing player).
    pub sacrificed_turn: Vec<(CardId, PlayerId)>,
    /// Tokens created this turn by each player (index = controller).
    pub tokens_made_by: Vec<u32>,
    /// Treasure tokens created this turn by each player (index = controller).
    pub treasures_made_by: Vec<u32>,
    /// Permanents put into their owner's hand from the battlefield this turn: (card, owner).
    pub bounced_turn: Vec<(CardId, PlayerId)>,
    /// Permanents that left the battlefield this turn: (card, controller when it left).
    pub left_bf_turn: Vec<(CardId, PlayerId)>,
    /// Power each permanent had as it left the battlefield this turn (Sword of the Ages: total power of the creatures sacrificed).
    pub left_bf_power: Vec<(CardId, i32)>,
    /// Keywords a permanent had when it left the battlefield this turn (last-known information for damage it deals afterwards: lifelink, deathtouch).
    pub left_bf_kw: Vec<(CardId, KeywordSet)>,
    /// Tokens put into a graveyard from the battlefield this turn: (controller, owner, types as they last existed).
    pub token_gy_turn: Vec<(PlayerId, PlayerId, TypeSet)>,
    /// A permanent with an oil counter on it was put into a graveyard this turn.
    pub oil_died_turn: bool,
    /// Creatures that died this turn: (card, controller when it died).
    pub died_turn: Vec<(CardId, PlayerId)>,
    /// Last-known power of each creature in `died_turn` (same indices).
    pub died_power: Vec<i32>,
    /// Whether each creature in `died_turn` was modified when it died (a counter on it, or Equipment / its controller's Aura attached).
    pub died_modified: Vec<bool>,
    /// Number of combat phases begun this turn.
    pub combats_this_turn: u8,
    /// Players attacked this turn: (attacking player, defending player).
    pub attacked_players_turn: Vec<(PlayerId, PlayerId)>,
    /// Blocks declared this turn: (blocker, attacker, attacker's zone-change counter, the attacker's controller when it became blocked); the latest block of a pair wins (`Effect::DestroyBlockedBy`; Glyph of Reincarnation).
    pub block_log: Vec<(CardId, CardId, u32, PlayerId)>,
    /// Recent attacks on players (not planeswalkers): (turn number, attacking player, defending player).
    pub attacked_player_log: Vec<(u32, PlayerId, PlayerId)>,
    pub spells_this_turn: u32,
    /// Spells cast this turn: (caster, card).
    pub spells_log: Vec<(PlayerId, CardId)>,
    /// Players (bit per player index) who already got a poison counter under a `PoisonOnlyOne` replacement this turn (Melira, the Living Cure).
    pub poison_capped: std::cell::Cell<u32>,
    /// Players who cast a spell from a graveyard or activated an ability of a card in a graveyard this turn.
    pub graveyard_use_log: Vec<PlayerId>,
    /// Players who cast an instant or sorcery from a graveyard this turn (one entry per cast; Sevinne, the Chronoclasm).
    pub gy_instant_sorcery_log: Vec<PlayerId>,
    /// Players who foretold a card this turn (Ranar the Ever-Watchful).
    pub foretold_log: Vec<PlayerId>,
    /// Players who activated an ability of an artifact this turn (`AbilityCostChange.first_per_turn`).
    pub artifact_act_log: Vec<PlayerId>,
    /// Players who this turn activated an ability that targeted a creature they control, during their own turn (Professor Hojo).
    pub creature_tgt_act_log: Vec<PlayerId>,
    /// Players who activated a power-up ability this turn, one entry per activation (Advancing the Spirit).
    pub power_up_act_log: Vec<PlayerId>,
    /// Names of every spell each player has cast this game (never cleared; "you've cast another spell named X this game").
    pub spells_cast_names: Vec<(PlayerId, String)>,
    /// Every decision answered so far (seed + this log = the whole game).
    pub decision_log: Vec<Choice>,
    pub lki_slot: Option<Lki>,
    /// True while a permanent is being exiled to pay the cost of a craft ability (`Trigger::ChangesZone.craft_cost`).
    pub craft_exile: std::cell::Cell<bool>,
    /// Last-known information of the most recent permanent that left the battlefield (kept after the move events).
    pub last_lki: Option<Lki>,
    /// Last-known information of every permanent that left the battlefield this turn (capped), so a trigger can read its own source after others left too.
    pub left_lki: Vec<Lki>,
    /// (source, ability, player) batched combat-damage triggers already fired in this damage step.
    pub batch_seen: Vec<(CardId, u16, PlayerId)>,
    /// Batched attack triggers per attacked planeswalker / battle: (source, ability, attacked permanent).
    pub batch_seen_pw: Vec<(CardId, u16, CardId)>,
    /// Permanents sacrificed as a cost since the current resolution began ("sacrifice a creature: ... its power").
    pub cost_sacrificed: Vec<CardId>,
    /// The player a `player_choice` mana ability gives its mana to (set for the one activation by `Run::ManaGiftAsk`).
    pub mana_gift: Option<PlayerId>,
    /// Cards the player picked to pay a mana ability's graveyard-exile cost (Sunken Palace); used by the next activation, else the cheapest are taken.
    pub mana_picks: Vec<CardId>,
    /// The cards milled to pay the last `Cost::Mill` (read by `ManaProduce::MilledSymbols`).
    pub cost_milled: Vec<CardId>,
    /// Mana spent by the latest "unless ... pays" payment, by `ManaType`.
    pub unless_spent: [u8; 6],
    /// While true, `try_pay_costs` records the mana it spends into `spent_track`.
    pub track_pay_spent: bool,
    /// Batched triggers that already fired for the current group of simultaneous events.
    pub zone_batch: Vec<(CardId, u16)>,
    /// (source, source zcc, exiled card, exiled zcc): cards exiled "until this leaves the battlefield".
    /// While true, permanents leaving the battlefield are recorded in `batch_left` so look-back triggers see simultaneous departures.
    pub batch_active: bool,
    /// Moves to a graveyard seen so far in the current simultaneous batch (see `triggers::on_event_for`).
    pub batch_gy_events: Vec<Event>,
    /// While true, `on_event_for` replays earlier batch moves for graveyard-functioning zone-change abilities only.
    pub gy_replay: bool,
    /// True while a copy of a spell is resolving (it was not cast, so `CastFrom` is false for it).
    pub copy_resolving: std::cell::Cell<bool>,
    /// The spell currently being paid for / checked for affordability (restricted mana counts only for matching spells).
    pub pay_for: std::cell::Cell<Option<CardId>>,
    /// While affordability of a cast with this alternative cost (`Some(alt)`) is probed, `Condition::CastWithAlt` of the `pay_for` spell reads it instead of the spell's `cast_alt`.
    pub pay_alt_probe: std::cell::Cell<Option<Option<u8>>>,
    /// The cost being paid contains a {C} symbol (restricted mana with `cost_c` then counts).
    pub pay_cost_has_c: std::cell::Cell<bool>,
    /// Permanents a cost under evaluation must sacrifice (all eligible ones are needed): they can't also be tapped for mana.
    pub sac_reserve: std::cell::RefCell<Vec<CardId>>,
    /// Mana of each type spent on the cost currently being paid (stored on the spell when it is cast).
    pub spent_track: std::cell::Cell<[u8; 6]>,
    pub snow_track: std::cell::Cell<u8>,
    pub snow_colors_track: std::cell::Cell<u8>,
    pub src_track: std::cell::Cell<[u8; 5]>,
    pub src_cards_track: std::cell::RefCell<Vec<CardId>>,
    /// An effect marked `unpreventable` is dealing damage right now.
    pub force_unprevent: std::cell::Cell<bool>,
    /// Set while `Effect::DealDamageAgain.plain` deals damage: `Replace::Damage.redirect_optional` replacements are skipped.
    pub skip_opt_redirect: std::cell::Cell<bool>,
    pub treasure_paid: std::cell::Cell<bool>,
    /// `pay_for` is the source of an activated ability being paid for (not a spell).
    pub pay_is_ability: std::cell::Cell<bool>,
    /// A cumulative-upkeep payment is being resolved: restricted mana marked `upkeep` may pay it (Snowfall).
    pub pay_upkeep: std::cell::Cell<bool>,
    /// The mana cost of unlocking a Room door is being paid: restricted mana marked `unlock` may pay it (Smoky Lounge).
    pub pay_unlock: std::cell::Cell<bool>,
    pub pay_is_equip: std::cell::Cell<bool>,
    pub pay_is_power_up: std::cell::Cell<bool>,
    pub pay_is_foretell: std::cell::Cell<bool>,
    /// While paying for an activated ability: whether its cost contains {X} (restricted mana "only for costs containing {X}").
    pub pay_ability_has_x: std::cell::Cell<bool>,
    /// A restricted-mana source is being tapped as part of paying a cost (its mana goes straight into the pool, the restriction was checked by the planner).
    pub paying_activation: std::cell::Cell<bool>,
    /// Riders of the mana spent on the spell currently being paid for (applied to the spell when it is cast).
    pub pay_riders: std::cell::RefCell<Vec<crate::dsl::ManaRider>>,
    /// The mana sources that gave `ManaRider::ExileSpell` riders (parallel to the ExileSpell entries of `pay_riders`).
    pub pay_rider_srcs: std::cell::RefCell<Vec<CardId>>,
    /// One entry per unit of rider mana produced while paying: its rider, colour and spell filter, so units that end up unspent keep the rider while they float.
    pub pay_rider_units: std::cell::RefCell<Vec<(crate::dsl::ManaRider, ManaType, Option<FilterBox>)>>,
    /// Mana of an "any colour" copy rider (Primal Wellspring) was spent: the copy is made even if the spell is gone by then.
    pub copy_rider_always: std::cell::Cell<bool>,
    /// While set, `effective_cost` assumes target-dependent cost increases apply (used to detect that a per-target affordability check is needed).
    pub assume_target_tax: std::cell::Cell<bool>,
    /// Controllers of spells that were countered (so "its controller" still works after it left the stack).
    pub countered_by: Vec<(StackId, PlayerId)>,
    /// Spells countered this turn: (the spell's card, its controller, the controller of what countered it).
    pub spells_countered: Vec<(CardId, PlayerId, PlayerId)>,
    /// The zone a spell is being cast from while its legality is checked (for `Condition::CastFrom`).
    pub cast_hint: std::cell::Cell<Option<Zone>>,
    /// The trigger event whose targets are being chosen/validated (so filters can compare against "that permanent").
    /// Cards the resolving effect lets a player cast for free once it has finished.
    /// Permanents that are temporarily a copy of something else: (card, zcc, original definition), restored at end of turn.
    /// When each permanent last transformed (turn, step).
    pub last_transform: std::collections::HashMap<CardId, (u32, Step)>,
    /// `next_stack_id` when a card last transformed: an ability put on the stack earlier can't transform it again (701.28f).
    pub last_transform_stack: std::collections::HashMap<CardId, u32>,
    /// Additional combat phases queued: before the post-combat main phase / after it.
    /// Goaded creatures: (creature, zcc, goader, turn the goad was applied).
    pub goaded: Vec<(CardId, u32, PlayerId, u32)>,
    /// Goads that last while a source stays on the battlefield (turn marker `u32::MAX - 2`): (creature, zcc, source, source zcc).
    pub goad_while_src: Vec<(CardId, u32, CardId, u32)>,
    /// Objects that can't get counters while a source stays on the battlefield: (object, object's zone-change count (0 for a player), source, source zcc) (Suncleanser).
    pub no_counters_src: Vec<(Obj, u32, CardId, u32)>,
    pub extra_combat_now: u8,
    /// Additional upkeep steps still to happen (Ninth Doctor, Paradox Haze; Obeka: after the combat phase, then back to `upkeep_return`).
    pub extra_upkeep: u32,
    pub upkeep_return: Option<Step>,
    /// Additional beginning phases queued for after the post-combat main phase; `in_extra_beginning` while one is running.
    pub extra_beginning: u8,
    pub in_extra_beginning: bool,
    /// Additional beginning phases queued for right after the combat phase (Cyclonus, Cybertronian Fighter); the step to go to once that phase's draw step is over.
    pub extra_beginning_combat: u8,
    pub extra_beginning_return: Option<Step>,
    /// Upkeep steps begun this turn ("the first upkeep each turn").
    pub upkeeps_this_turn: u32,
    /// Additional end steps still to happen this turn (Y'shtola Rhul), and end steps begun this turn.
    pub extra_end: u32,
    pub end_steps_this_turn: u32,
    /// Who was the monarch when the current turn began (`Condition::WasMonarchAtTurnStart`).
    pub monarch_turn_start: Option<PlayerId>,
    pub extra_combat_after_main: u8,
    /// Extra combats queued with `then_main`: when the extra combat ends, one more post-combat main phase will follow the next one.
    pub extra_main_pending: u8,
    pub extra_main_repeat: u8,
    pub copy_reverts: Vec<(CardId, u32, DefId, bool)>,
    /// Copies that end as their controller's next turn begins (`Effect::BecomeCopy` with `UntilYourNextTurn`): (copy, its zcc, old definition, the controller).
    pub copy_reverts_turn: Vec<(CardId, u32, DefId, PlayerId)>,
    /// Copies that last while their source stays on the battlefield: (copy, its zcc, old definition, source, source zcc).
    pub copy_while_source: Vec<(CardId, u32, DefId, CardId, u32, u8)>,
    pub free_casts: Vec<(PlayerId, CardId, bool, Option<u8>)>,
    /// `CastFree.after_cast`: (card being cast, the effect's source, effects to run once it has been cast).
    pub cast_after: Vec<(CardId, CardId, Vec<Effect>)>,
    /// A spell just exiled by a `CounterExilePlay` replacement: its card may be cast free by this player.
    pub counter_offer: Option<(PlayerId, CardId)>,
    /// Cipher: (encoded spell card, the creature it is encoded on, that creature's zone-change counter).
    pub encoded: Vec<(CardId, CardId, u32)>,
    /// Meld: (the melded permanent, its two halves in exile).
    pub meld_parts: Vec<(CardId, CardId, CardId)>,
    /// Cards cast for free with "exile it instead of putting it into a graveyard".
    pub exile_marked: Vec<CardId>,
    /// The next `begin_cast` costs no mana.
    pub free_cast_now: std::cell::Cell<bool>,
    /// Cost reduction left over after the generic part of the last computed cost: it reduces {X} (Affinity on an X spell).
    pub x_red: std::cell::Cell<u32>,
    /// Cost reduction of the last `effective_cost` that found no generic mana to reduce (it can reduce a kicker added later: Vine Gecko).
    pub red_overflow: std::cell::Cell<u32>,
    /// Coloured cost reductions of the last `effective_cost` that found no coloured mana to remove (Edgewalker with a chosen additional mana cost).
    pub color_left: std::cell::RefCell<Vec<(Color, i32)>>,
    pub filter_rem: std::cell::RefCell<Vec<Obj>>,
    /// Targets chosen by earlier `Reflexive`s of the same stack item (`TargetSpec.distinct_modes`).
    pub reflex_used: std::cell::RefCell<Vec<(StackId, Obj)>>,
    /// The chosen targets of the effect being resolved (filters can compare against "the target spell's mana value").
    pub filter_targets: std::cell::RefCell<(Vec<Target>, Vec<u8>)>,
    pub filter_event: std::cell::RefCell<Option<EventCtx>>,
    /// Follow-up effects of damage replacements (`Replace::Damage.then`) waiting to be put on the stack: (source of the replacement, its controller, effects, amount removed, damage source).
    /// Cards drawn since `DrawRemember` last cleared it.
    /// Damage dealt this turn per source card (Brightflame: "damage dealt this way").
    pub dmg_by_source: std::cell::RefCell<std::collections::HashMap<CardId, i32>>,
    /// Counters taken off cards since the last `RemoveAnyCounters` began (Hex Parasite: "for each counter removed this way").
    pub counters_removed: std::cell::Cell<i32>,
    /// While a `MoveCounters { at_least_one }` kind pick is pending: the player may not stop before one counter has moved.
    pub kind_pick_min1: std::cell::Cell<bool>,
    /// Set while a `DividePrevention` is being divided (shields instead of damage).
    pub divide_prevent: std::cell::Cell<bool>,
    pub divide_cleanup: std::cell::Cell<bool>,
    pub divide_end_step: std::cell::Cell<bool>,
    /// Counters to take off again at the next cleanup step (creature, zone-change count, kind, how many).
    pub cleanup_counter_removals: Vec<(CardId, u32, CounterKind, i32)>,
    /// Set while a permanent is being destroyed (so a zone-change trigger can tell destroyed from sacrificed).
    pub cause_destroy: std::cell::Cell<bool>,
    /// The permanent last sacrificed and its zone-change counter at that moment (a zone-change trigger can tell "sacrificed" from other ways to the graveyard).
    pub last_sacrificed: std::cell::Cell<Option<(CardId, u32)>>,
    /// Set while a creature is sacrificed to pay a spell's emerge cost (`Trigger::Sacrificed.emerge`).
    pub emerge_sac: std::cell::Cell<bool>,
    /// Permanents that just transformed into an Aura face with `Ability::AttachPlayerOnTransform` and still need their controller to choose the player they enchant.
    pub attach_pending: Vec<CardId>,
    /// Creatures that had first or double strike when first-strike combat damage was dealt this combat (they deal regular damage only while they still have double strike).
    pub fs_dealers: Vec<CardId>,
    /// Greatest last-known power among the permanents the latest `Effect::Sacrifice` sacrificed (`Amount::GreatestPowerSacrificed`).
    pub sac_max_power: std::cell::Cell<i32>,
    pub draw_log: std::cell::RefCell<Vec<CardId>>,
    /// `Effect::DivideDamage.any_number` in force while its division is being asked.
    pub divide_any: std::cell::Cell<bool>,
    pub prevent_batch: std::cell::RefCell<Option<Vec<CardId>>>,
    /// Draw replacements already applied on the way to the draw being looked at (each applies once per chain: two Blood Scriveners).
    pub draw_chain: std::cell::RefCell<Vec<CardId>>,
    /// The chain each waiting follow-up (by its position in `dmg_followups`) carries.
    pub followup_chains: std::cell::RefCell<Vec<(usize, Vec<CardId>)>>,
    pub dmg_combat_now: std::cell::Cell<bool>,
    /// The X the player chose for a mana ability with an `XTimes` cost (`Choice::Mana.x`), read while that ability is paid.
    pub mana_x_hint: std::cell::Cell<Option<i32>>,
    /// Colours the player named for the triggered mana abilities of the tap in progress (`Choice::Mana.picks`).
    pub mana_color_hints: std::cell::RefCell<Vec<crate::mana::ManaType>>,
    /// The colour the player asked a tapped land to produce (`Choice::Mana.produce`); picks between several mana-type replacements that apply to one tap.
    pub mana_produce_hint: std::cell::Cell<Option<crate::mana::ManaType>>,
    /// Where the damaged player chose to redirect damage when several redirects apply (`Effect::DealDamage`).
    pub redirect_pick: std::cell::Cell<Option<CardId>>,
    pub dmg_followups: std::cell::RefCell<Vec<(CardId, PlayerId, Vec<Effect>, i32, Option<CardId>, Option<CardId>, Option<PlayerId>, Vec<Target>)>>,
    /// Per player: a permanent they controlled left the battlefield this turn (revolt).
    pub revolt: Vec<bool>,
    /// Per player: a permanent card was put into their graveyard this turn (descended).
    pub descended: Vec<bool>,
    /// Damage prevention shields (until end of turn).
    pub shields: Vec<Shield>,
    /// While true, event processing considers only abilities granted to permanents (used right after a token copy gets granted abilities).
    pub granted_only: bool,
    /// Phased-out permanents (still zone Battlefield).
    pub phased: Vec<CardId>,
    /// The source of the effect being executed (tags cards it exiles).
    pub cur_src: Option<CardId>,
    /// An effect said "end the turn": handled when the resolving spell or ability finishes.
    pub end_turn_pending: bool,
    pub end_combat_pending: bool,
    /// The controller of the spell or ability whose effect is resolving right now (None outside effect resolution).
    pub cause_ctl: std::cell::Cell<Option<PlayerId>>,
    /// A card is being cycled right now (the discard of a cycling cost): `Replace::ZoneChange.not_cycled`.
    pub cycling_move: std::cell::Cell<bool>,
    /// The nonland card whose cycling ability is resolving its draw (read by `Replace::Draw.cycling_other_nonland`).
    pub draw_cycled: std::cell::Cell<Option<CardId>>,
    /// The source of the spell or ability whose effect is resolving right now (None outside effect resolution).
    pub cause_src: std::cell::Cell<Option<CardId>>,
    /// The targets of the spell or ability whose effect is resolving right now (empty outside effect resolution).
    pub cause_targets: std::cell::RefCell<Vec<Obj>>,
    /// Set while attackers are being tapped as they are declared ("if it isn't being declared as an attacker").
    pub declaring_attacker: std::cell::Cell<bool>,
    /// The Choose effect being resolved adds to Remembered instead of replacing it.
    pub choose_accumulate: std::cell::Cell<bool>,
    /// Test hook: forced die results consumed in order by `Effect::RollDie`.
    pub forced_rolls: std::cell::RefCell<Vec<i32>>,
    /// Sets saved by `Effect::StoreRemembered` (slots 0-3). Emptied as each stack item begins to resolve.
    pub stored_sets: std::cell::RefCell<[Vec<Obj>; 4]>,
    /// Snapshots of stored slots taken when a lasting object (granted static, delayed / added trigger, replacement) was made:
    /// slot id `4 + i` reads entry `i` (`Game::pin_stored`). At most `PIN_MAX` entries; past that the oldest is reused.
    pub pinned_sets: Vec<Vec<Obj>>,
    /// Next entry of `pinned_sets` to reuse once it is full.
    pub pinned_next: u16,
    /// Turn number on which creatures can be blocked only by creatures in their own sector (Space Beleren +1); 0 = never.
    pub sector_block_turn: u32,
    /// A wall-clock limit for `run` (set on the AI's simulation copies): once passed, `run` stops and reports `Status::Over` without a result.
    pub abort_at: Option<std::time::Instant>,
    /// Text-box exchanges that last while their source stays on the battlefield: (source, first, second, first's original definition, second's).
    pub text_swaps: Vec<(CardId, CardId, CardId, DefId, DefId)>,
    /// The colour picked for the last `RestrictedAny` mana group of the `AddMana` effect being resolved (read by `RestrictedAny.other_color`).
    pub mana_pick: Option<Color>,
    /// Replacement effects created by `Effect::AddReplacement` (source, controller, effect); cleared at cleanup.
    pub temp_replaces: Vec<(CardId, PlayerId, Replace)>,
    /// Replacement effects that last until their controller's next turn (`AddReplacement.until_your_next_turn`).
    pub long_replaces: Vec<(CardId, PlayerId, Replace)>,
    /// Cards outside the battlefield that currently hold an ability granted by a static (cleared at each layer recompute).
    pub zone_granted: Vec<CardId>,
    /// Indices into `temp_replaces` used up by a `once` replacement (removed after the damage event).
    /// (damage dealt, excess damage) of the most recent damage event.
    pub last_damage: std::cell::Cell<(i32, i32)>,
    /// The life actually lost by the most recent `Effect::LoseLife` (`Amount::LifeLostThisWay`).
    pub last_life_lost: std::cell::Cell<i32>,
    /// Cards the last surveil left on top of the library.
    pub last_surveil_kept: std::cell::Cell<i32>,
    /// (player, zone, filter, source, turn) permissions from `Effect::MayPlayThisTurn`.
    pub turn_play_grants: Vec<(PlayerId, Zone, Filter, CardId, u32, Vec<CardId>, bool)>,
    /// (player, n): the player can't declare more than n different blockers for the rest of this combat (`Effect::LimitBlockers`).
    pub block_limits: Vec<(PlayerId, u8)>,
    /// `Effect::PileBlocks` (Camouflage): the turn in which blockers are declared as random piles instead (0 = none).
    pub pile_blocks_turn: u32,
    /// The piles a defending player has chosen so far for `pile_blocks_turn` (player, piles).
    pub pile_state: Option<(PlayerId, Vec<Vec<CardId>>)>,
    /// The card a `DivideCounters { take_from }` takes its counters from.
    pub divide_from: std::cell::Cell<Option<CardId>>,
    /// (player, turn, filter, source) graveyard casts that must forage (`Effect::MayCastForaging`).
    pub forage_grants: Vec<(PlayerId, u32, Filter, CardId)>,
    /// (caster, graveyard owner, turn, filter, source, any mana type) casts from another player's graveyard (`Effect::MayPlayThisTurn.of`).
    /// (caster, turn, filter, source): spells cast from the graveyard through such a grant are exiled instead of going to the graveyard (`MayPlayThisTurn.exile_after`).
    pub gy_exile_grants: Vec<(PlayerId, u32, Filter, CardId)>,
    /// (caster, turn, card, its zcc): graveyard cards the caster may cast this turn without paying their mana cost, exiled instead of going to the graveyard (`Effect::MayCastFreeFromGraveyardThisTurn`).
    pub gy_free_grants: Vec<(PlayerId, u32, CardId, u32)>,
    /// (caster, turn, card, its zcc): graveyard cards (any graveyard) the caster may cast this turn paying their costs (`Effect::MayCastCardThisTurn`).
    pub gy_card_grants: Vec<(PlayerId, u32, CardId, u32)>,
    /// Riders on `gy_card_grants` entries: (card, its zcc, turn, exile instead of graveyard, extra generic unless the spell targets a creature its caster controls).
    pub gy_card_riders: Vec<(CardId, u32, u32, bool, u8)>,
    /// (caster, turn, filter, source, finality, extra subtype): the caster may cast ONE spell matching the filter from their graveyard this turn (`Effect::MayCastFromGraveyardOnce`).
    pub gy_once_grants: Vec<(PlayerId, u32, Filter, CardId, bool, Option<Sym>)>,
    /// Tawnos's Coffin: (coffin, its zcc, exiled creature, its zcc, counters noted on it, exiled Auras with their zccs) (`Effect::CoffinExile` / `CoffinReturn`).
    /// Ertai's Meddling: (exiled card, its zcc, the player whose upkeeps count it down, the spell as it was on the stack) (`Effect::ExileSpellDelayed`).
    pub meddled: Vec<(CardId, u32, PlayerId, StackItem)>,
    /// The game's turn order runs backwards (`Effect::ReverseTurnOrder`).
    pub turn_reversed: bool,
    pub coffin_notes: Vec<(CardId, u32, CardId, u32, Vec<(CounterKind, u16)>, Vec<(CardId, u32)>)>,
    pub gy_play_grants: Vec<(PlayerId, PlayerId, u32, Filter, CardId, bool)>,
    /// (caster, library owner, turn): the caster may play the top card of that player's library this turn, spending mana as though it were mana of any color (`Effect::PlayTopOfLibraryThisTurn`; Xanathar, Guild Kingpin).
    /// Mana lost per source this turn through `DrainManaPool { record: true }`.
    pub mana_drained_by: Vec<(CardId, i32)>,
    pub lib_play_grants: Vec<(PlayerId, PlayerId, u32)>,
    /// `PlayTopFreeWhileOnTop`: (player, turn, card, zone-change count): that card may be played free while it is on top of its owner's library.
    pub lib_free_grants: Vec<(PlayerId, u32, CardId, u32)>,
    /// (grantee, hand owner, turn): the grantee may play lands and cast spells from the owner's hand this turn (`MayPlayThisTurn { zone: Hand, of }`: Sen Triplets).
    pub hand_play_grants: Vec<(PlayerId, PlayerId, u32)>,
    /// (player, turn): the player may play cards they own from outside the game (their sideboard) this turn (`Effect::MayPlayFromOutsideGame`; Wish).
    pub outside_grants: Vec<(PlayerId, u32)>,
    /// (player, turn, filter, source, extra_mode): one matching card from the player's sideboard may be played this turn (`MayPlayFromOutsideGame` with a filter); used up by the play.
    pub outside_card_grants: Vec<(PlayerId, u32, Filter, CardId, bool)>,
    /// Counters are being put by a modular trigger (`Effect::ModularCounters`).
    pub modular_cause: std::cell::Cell<bool>,
    /// Counters are being put on as a cost (loyalty abilities): only "if you would put" replacements (those with a `by` filter) apply, not "if an effect would" ones (Doubling Season).
    pub counters_as_cost: std::cell::Cell<bool>,
    /// A creature is being tapped to pay a teamwork cost.
    pub teamwork_tap: std::cell::Cell<bool>,
    pub consumed_temp: std::cell::RefCell<Vec<usize>>,
    /// (index into `temp_replaces`, remaining shield amount) after a `prevent_up_to` shield absorbed damage.
    pub reduced_temp: std::cell::RefCell<Vec<(usize, i32)>>,
    /// > 0 while redirected damage is being dealt: redirect shields don't apply again (no loops).
    pub redirecting: u8,
    /// Counts stack-item resolutions (coins flipped within one resolution are one set).
    pub flip_serial: u64,
    /// Set while a combat damage step deals damage: a once-only full redirect keeps applying to the rest of that simultaneous damage.
    pub combat_batch: bool,
    /// Life lost during the current simultaneous combat damage, per player; emitted as one event afterwards.
    pub life_batch: Vec<(PlayerId, i32)>,
    /// The once-only redirect shields (their `to` and `redirect`) that already applied in the current combat damage step.
    pub batch_once_used: Vec<(Option<Vec<Obj>>, Option<Obj>)>,
    pub player_prot: Vec<PlayerProt>,
    pub batch_left: Vec<(CardId, PlayerId)>,
    /// Members of `batch_left` that had lost all abilities when they left (a face-down permanent): they see no look-back triggers.
    pub batch_noabil: Vec<CardId>,
    /// Permanents being put onto the battlefield together by one effect: their replacement effects don't apply to each other (Living Death with Grumgully / Containment Priest).
    pub entering_batch: Vec<CardId>,
    /// While set, enter-the-battlefield and token-creation events are queued here and matched against triggers together afterwards (tokens created at once see each other enter).
    pub defer_events: Option<Vec<Event>>,
    /// Replacement abilities granted to members of `batch_left` as they left (The Masamune's extra death trigger still applies to simultaneous deaths).
    pub batch_granted: Vec<(CardId, std::sync::Arc<Ability>)>,
    pub linked_exiles: Vec<(CardId, u32, CardId, u32, Zone)>,
    /// (exiled card, its zcc, counters, the source's controller): +1/+1 counters a card gets when it returns from a linked exile under that player's control (`Effect::Exile.return_counters`).
    pub linked_counters: Vec<(CardId, u32, i32, PlayerId)>,
    /// Like `linked_exiles`, but returned by `Effect::ReturnLinkedExiles` instead of when the source leaves.
    pub manual_linked: Vec<(CardId, u32, CardId, u32)>,
    /// (exiled card, its zcc, the exiling effect's controller): cards `Effect::ExileUntilOpponentMonarch` exiled; they return when an opponent of that controller becomes the monarch.
    pub monarch_exiles: Vec<(CardId, u32, PlayerId)>,
    /// A state-based-action question (legend rule) is waiting for an answer.
    pub sba_waiting: bool,
    /// Permanents that entered with an "as this enters, choose" ability still waiting for the choice.
    pub enter_choices: Vec<CardId>,
    /// Enter events of permanents entering as a copy (Clone): matched against triggers only after the copy choice is made.
    pub copy_deferred: Vec<(CardId, Zone)>,
    /// Permanents entering "under the control of an opponent of your choice": the player who chooses (who would have controlled it).
    pub under_opponent_chooser: Vec<(CardId, PlayerId)>,
    /// Optional zone-change replacements found by `replace_zone` for the move in progress: (card, instead, bottom).
    pub opt_zone_tmp: std::cell::RefCell<Vec<(CardId, Zone, bool)>>,
    /// Several zone-change replacements applied to one card (Nemata / Rest in Peace): the candidates (source, controller, follow-up effects) found while moving it.
    pub zone_choice_tmp: std::cell::RefCell<Vec<(CardId, Vec<(CardId, PlayerId, Vec<Effect>)>)>>,
    /// The same, queued with the card's zone-change counter until its owner has chosen.
    pub zone_choice_q: Vec<(CardId, u32, Vec<(CardId, PlayerId, Vec<Effect>)>)>,
    /// Cards that moved where an optional replacement could have sent them elsewhere, waiting for their owner's answer: (card, zcc after the move, instead, bottom).
    pub opt_zone_q: Vec<(CardId, u32, Zone, bool)>,
    /// The action list of the pending priority decision (`Choice::Priority` indexes into it).
    pub last_actions: Vec<Action>,
    pub steps_taken: u64,
    pub max_steps: u64,
}

impl Game {
    pub fn card(&self, id: CardId) -> &Card {
        &self.cards[id.0 as usize]
    }
    pub fn card_mut(&mut self, id: CardId) -> &mut Card {
        &mut self.cards[id.0 as usize]
    }
    pub fn player(&self, id: PlayerId) -> &Player {
        &self.players[id as usize]
    }
    pub fn player_mut(&mut self, id: PlayerId) -> &mut Player {
        &mut self.players[id as usize]
    }
    pub fn def_of(&self, id: CardId) -> &crate::carddb::Def {
        self.db.def(self.cards[id.0 as usize].def)
    }
    pub fn n_players(&self) -> usize {
        self.players.len()
    }
    pub fn alive(&self, p: PlayerId) -> bool {
        !self.players[p as usize].lost
    }
    pub fn alive_players(&self) -> Vec<PlayerId> {
        (0..self.players.len() as PlayerId).filter(|p| self.alive(*p)).collect()
    }
    /// The next living player in turn order after `p`.
    pub fn previous_player(&self, p: PlayerId) -> PlayerId {
        let n = self.players.len() as PlayerId;
        let step = if self.turn_reversed { 1 } else { n - 1 };
        let mut q = p;
        for _ in 0..n {
            q = (q + step) % n;
            if self.alive(q) {
                return q;
            }
        }
        p
    }
    pub fn next_player(&self, p: PlayerId) -> PlayerId {
        let n = self.players.len() as PlayerId;
        let step = if self.turn_reversed { n - 1 } else { 1 };
        let mut q = p;
        for _ in 0..n {
            q = (q + step) % n;
            if self.alive(q) {
                return q;
            }
        }
        p
    }
    /// `id` is a creature on the battlefield that its controller chose as their Ring-bearer.
    pub fn is_ring_bearer(&self, id: CardId) -> bool {
        let c = &self.cards[id.0 as usize];
        c.zone == Zone::Battlefield && self.players.get(c.cur.controller as usize).map_or(false, |p| p.ring_bearer == Some(id))
    }
    pub fn opponents(&self, p: PlayerId) -> Vec<PlayerId> {
        self.alive_players().into_iter().filter(|q| self.is_opponent(*q, p)).collect()
    }
    /// Players on different teams (every player is their own team except in Archenemy).
    pub fn is_opponent(&self, a: PlayerId, b: PlayerId) -> bool {
        a != b && self.players.get(a as usize).zip(self.players.get(b as usize)).map_or(true, |(x, y)| x.team != y.team)
    }
    /// Players in APNAP order starting with the active player.
    pub fn apnap(&self) -> Vec<PlayerId> {
        let mut v = vec![];
        let mut q = self.active;
        for _ in 0..self.players.len() {
            if self.alive(q) {
                v.push(q);
            }
            q = (q + 1) % self.players.len() as PlayerId;
        }
        v
    }
    pub fn emit(&mut self, e: Event) {
        if let Event::Counters { card, kind, delta, by } = &e {
            if *delta > 0 {
                let c = &self.cards[card.0 as usize];
                self.counters_log.push((*card, c.cur.controller, *kind, *by, c.cur.types.has(CardType::Creature), *delta));
            }
        }
        // statics that look at tapped state (Paradise Druid) must be recomputed
        if matches!(e, Event::Tapped { .. } | Event::Untapped { .. } | Event::Monarch { .. }) {
            self.layers_dirty = true;
        }
        if let Event::Tapped { card } = &e {
            if self.cards[card.0 as usize].unmask {
                self.unmask_creature(*card);
            }
        }
        if let Event::Bent { player, kind } = &e {
            let turn = self.turn;
            let p = &mut self.players[*player as usize];
            if p.bend_turn != turn {
                p.bend_turn = turn;
                p.bend_mask = 0;
            }
            p.bend_mask |= 1 << (*kind as u8);
        }
        if let Event::LandPlayed { player, from_hand: false, .. } = &e {
            self.players[*player as usize].nonhand_turn = self.turn;
        }
        if let Event::Sacrificed { card, .. } = &e {
            self.last_sacrificed.set(Some((*card, self.cards[card.0 as usize].zcc)));
        }
        if let Event::Attacked { card, defender } = &e {
            let atk = self.cards[card.0 as usize].cur.controller;
            let def = match defender {
                Obj::Player { id } => Some(*id),
                Obj::Card { id } => Some(self.cards[id.0 as usize].cur.controller),
                _ => None,
            };
            if let Obj::Player { id } = defender {
                let t = self.turn;
                if !self.attacked_player_log.contains(&(t, atk, *id)) {
                    self.attacked_player_log.retain(|e| e.0 + 16 >= t);
                    self.attacked_player_log.push((t, atk, *id));
                }
            }
            // attacking a planeswalker isn't attacking its controller (Faramir, Prince of Ithilien)
            if let (Some(d), Obj::Player { .. }) = (def, defender) {
                if !self.attacked_players_turn.contains(&(atk, d)) {
                    self.attacked_players_turn.push((atk, d));
                }
            }
        }
        if let Event::Blocked { blocker, attacker } = &e {
            let (bz, bc) = (self.cards[attacker.0 as usize].zcc, self.cards[attacker.0 as usize].cur.controller);
            self.block_log.push((*blocker, *attacker, bz, bc));
            self.cards[blocker.0 as usize].blocked_turn = self.turn;
            self.cards[attacker.0 as usize].was_blocked_turn = self.turn;
            let t = self.turn;
            self.cards[blocker.0 as usize].block_pairs.retain(|p| p.0 == t);
            self.cards[attacker.0 as usize].block_pairs.retain(|p| p.0 == t);
            self.cards[blocker.0 as usize].block_pairs.push((t, *attacker));
            self.cards[attacker.0 as usize].block_pairs.push((t, *blocker));
        }
        if let Event::TurnedFaceUp { card } = &e {
            self.cards[card.0 as usize].face_up_turn = self.turn;
            let ctl = self.cards[card.0 as usize].controller as usize;
            self.players[ctl].face_down_act_turn = self.turn;
        }
        if let Event::Token { card } = &e {
            let p = self.cards[card.0 as usize].cur.controller as usize;
            if self.tokens_made_by.len() <= p {
                self.tokens_made_by.resize(p + 1, 0);
            }
            self.tokens_made_by[p] += 1;
            if self.db.def(self.cards[card.0 as usize].def).name == "Treasure" {
                if self.treasures_made_by.len() <= p {
                    self.treasures_made_by.resize(p + 1, 0);
                }
                self.treasures_made_by[p] += 1;
            }
        }
        if let Event::Sacrificed { card, player } = &e {
            self.sacrificed_turn.push((*card, *player));
            self.sacrificed_types.push((*player, self.cards[card.0 as usize].cur.types.0));
        }
        if let Event::Discarded { card, player, .. } = &e {
            // a card with madness is discarded into exile, not the graveyard (its madness trigger then casts it or puts it into the graveyard)
            if self.cards[card.0 as usize].zone == Zone::Graveyard && self.discard_zone(*card) == Zone::Exile {
                let c = *card;
                self.move_card(c, Zone::Exile, Default::default());
            }
            self.cards[card.0 as usize].discarded_turn = Some(self.turn);
            if let Some(p) = self.players.get_mut(*player as usize) {
                p.discarded_turn += 1;
            }
        }
        let monarch_now = if let Event::Monarch { player } = &e { Some(*player) } else { None };
        if monarch_now.is_some() {
            self.layers_dirty = true;
        }
        // a permanent entering as a copy is what it copies only after the choice: enters triggers wait for it
        if let Event::Moved { card, from, to: Zone::Battlefield } = &e {
            let c = &self.cards[card.0 as usize];
            let under_choice = c.enter_done & 8 == 0
                && self.db.def(c.def).abilities.iter().any(|a| matches!(a, crate::dsl::Ability::EntersChoose { kind: crate::dsl::ChooseKind::UnderOpponent, .. }))
                && self.under_opponent_chooser.iter().find(|(x, _)| x == card).map_or(false, |(_, p)| self.opponents(*p).into_iter().filter(|q| self.alive(*q)).count() > 1);
            if !c.face_down && self.enter_choices.contains(card) && (under_choice || self.db.def(c.def).abilities.iter().any(|a| (c.enter_done & 4 == 0 && matches!(a, crate::dsl::Ability::EntersAsCopy { until_turned_face_down: false, .. })) || (c.enter_done & 32 == 0 && matches!(a, crate::dsl::Ability::EntersUnlessCost { .. })))) {
                self.copy_deferred.push((*card, *from));
                self.events.push(e);
                return;
            }
        }
        if let Some(q) = self.defer_events.as_mut() {
            if matches!(e, Event::Moved { to: Zone::Battlefield, .. } | Event::Token { .. }) {
                q.push(e.clone());
                self.events.push(e);
                return;
            }
        }
        crate::triggers::on_event(self, &e);
        self.events.push(e);
        // cards exiled "until an opponent becomes the monarch" come back
        if let Some(np) = monarch_now {
            if !self.monarch_exiles.is_empty() {
                let back: Vec<(CardId, u32)> = self.monarch_exiles.iter().filter(|(_, _, ctl)| *ctl != np).map(|(c, z, _)| (*c, *z)).collect();
                self.monarch_exiles.retain(|(_, _, ctl)| *ctl == np);
                for (c, z) in back {
                    if self.cards[c.0 as usize].zone == Zone::Exile && self.cards[c.0 as usize].zcc == z {
                        self.move_card(c, Zone::Battlefield, crate::zones::MoveOpts::default());
                    }
                }
            }
        }
    }
    /// Match the queued simultaneous enter events against triggers (see `defer_events`).
    pub fn flush_deferred_events(&mut self) {
        if let Some(q) = self.defer_events.take() {
            for e in q {
                crate::triggers::on_event(self, &e);
            }
        }
    }
    pub fn log(&mut self, text: impl Into<String>) {
        self.events.push(Event::Log { text: text.into() });
    }
    pub fn next_ts(&mut self) -> u64 {
        self.ts += 1;
        self.ts
    }
    pub fn decision(&self) -> Option<&Decision> {
        self.pending.as_ref()
    }
    pub fn is_over(&self) -> bool {
        self.over.is_some()
    }
    pub fn drain_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }
}

impl std::fmt::Debug for Game {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Game(turn {}, {:?}, active {}, {} cards)", self.turn, self.step, self.active, self.cards.len())
    }
}

/// One damage event of this turn (see `Game::dmg_log`).
#[derive(Clone, Debug)]
pub struct DmgRec {
    pub source: Option<CardId>,
    pub target: Obj,
    pub amount: i32,
    pub combat: bool,
    /// Damage beyond lethal dealt to a creature by this event.
    pub excess: i32,
    /// The controller of a creature / planeswalker target when it was dealt the damage (None for players and other permanents).
    pub target_ctrl: Option<PlayerId>,
    /// The controller of the damage source when it dealt the damage.
    pub source_ctrl: Option<PlayerId>,
    /// Which combat phase of the turn it was dealt in (`Game::combats_this_turn` at the time; "this combat": Glen Elendra).
    pub combat_no: u8,
}

impl Derived {
    /// Add a keyword; a second source of the same `Toxic N` stacks instead of collapsing into the one flag.
    pub fn add_keyword(&mut self, k: Keyword) {
        let n = match k {
            Keyword::Toxic1 => 1,
            Keyword::Toxic2 => 2,
            Keyword::Toxic3 => 3,
            Keyword::Toxic4 => 4,
            Keyword::Toxic5 => 5,
            Keyword::Toxic9 => 9,
            _ => 0,
        };
        if n > 0 && self.kw.has(k) {
            self.toxic_extra = self.toxic_extra.saturating_add(n);
        } else {
            self.kw.add(k);
        }
    }
}
