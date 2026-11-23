# Card definitions

Every playable card is one JSON file in this tree (`cards/<first letter>/<Name_with_underscores>.json`). The engine compiles them
at load time into typed ability trees; nothing is interpreted from rules text during play. `cards/schema.json` is the
machine-readable schema (generated: `engine/target/release/cards schema > cards/schema.json`); this file is the human guide.

**Rule of thumb: be faithful, not approximate.** If a card can't be expressed exactly with the current DSL, do not write an
approximation; add the missing primitive to the engine first (see "When you can't express a card").

## Tools (build once: `cd engine && cargo build --release`)

| Command | What it does |
|---|---|
| `engine/target/release/cards check <files...>` | compile just these files; prints every problem (fast, use it constantly) |
| `engine/target/release/cards test [Card_Name...]` | run scenario tests (`cards/tests/<Card_Name>.json`) |
| `engine/target/release/smoke "<Card Name>" ...` | random-play smoke test (never crash/hang/corrupt) |
| `engine/target/release/cards validate` | compile every card |
| `engine/target/release/cards status <names.txt>` | status of each card named in the file (missing / blocked / validated / tested / failing) |

A card is **done** when its file compiles, its scenario tests pass, and `smoke` is green.

## File anatomy

```json
{
  "name": "Llanowar Elves",
  "mana_cost": "{G}",
  "types": ["Creature"],
  "subtypes": ["Elf", "Druid"],
  "power": 1, "toughness": 1,
  "abilities": [
    { "type": "Activated", "cost": [{ "type": "Tap" }],
      "effects": [{ "type": "AddMana", "mana": [{ "t": "Fixed", "m": "G", "n": 1 }] }] }
  ],
  "oracle_text": "{T}: Add {G}."
}
```

* `name` must equal the Oracle name exactly. Double-faced cards put the back face in `"back": { ... }` (a full card definition).
* `types`: Artifact, Battle, Creature, Enchantment, Instant, Kindred, Land, Planeswalker, Sorcery, ... `supertypes`: Basic, Legendary, Snow, World.
* `keywords`: only the simple ones the engine implements natively (Flying, FirstStrike, DoubleStrike, Deathtouch, Haste, Hexproof,
  Indestructible, Lifelink, Menace, Reach, Trample, Vigilance, Defender, Flash, Shroud, Fear, Intimidate, Skulk, Infect, Wither,
  Changeling, Unblockable ...; see the `Keyword` enum in the schema). Keywords with a cost or a trigger (Equip, Ward, Cycling, Kicker,
  Flashback, Prowess ...) are written as ordinary abilities - see the patterns below.
* `oracle_text`: copy the Oracle text (it is shown to players; never interpreted).
* Planeswalkers: `"loyalty": 4`; each loyalty ability is `Activated` with `"loyalty": +1` and an empty `cost`.
* Tokens a card makes are named in `CreateToken.token`; the standard ones (Soldier, Spirit, Zombie, Goblin, Treasure, Clue, Food, Blood, ...)
  live in the engine (`engine/crates/mtg-cards/src/tokens.rs`). A token that isn't there yet: add it as a card file with `"token": true`.

## Ability patterns (copy from the existing cards in `cards/core/`)

* **Spell** (instant/sorcery): `{ "type": "Spell", "targets": [...], "effects": [...] }`. Modal: `{ "type": "Modal", "choose_min": 1, "choose_max": 1, "modes": [{ "text": "...", "targets": [...], "effects": [...] }] }`.
* **Activated**: `{ "type": "Activated", "cost": [...], "targets": [...], "effects": [...], "timing": "Sorcery"|"Instant" }`.
  Mana abilities are Activated abilities whose only effect is `AddMana` with no targets.
* **Triggered**: `{ "type": "Triggered", "trigger": {...}, "effects": [...], "optional": true|false, "condition": {...}, "targets": [...] }`.
  `"filter": { "this": true }` makes a trigger about the card itself; `"other": true` excludes it; `"controller": "You"` etc.
* **Static**: `{ "type": "Static", "effect": { "type": "Modify", "affects": {filter}, "power": {"type":"Fixed","n":1}, "toughness": {...} } }`;
  also `GrantKeyword`, `SetBasePT`, `CdaPT`, `AddType`, `SetColors`, `CostChange`, `EnchantedGets`, `EnchantedCant`, ...
* **Aura**: `{ "type": "Attach", "to": {filter} }` plus `Static` effects using `EnchantedGets` / `EnchantedCant` (subtype `Aura`).
* **Equipment**: subtype `Equipment`, `Static EnchantedGets`, and `Equip {N}` = `Activated`, `"timing": "Sorcery"`, cost `[{"type":"Mana","cost":"{N}"}]`,
  one target (`Permanent` creature you control), effect `Attach { what: This, to: Target 0 }`.
* **Additional cost** to cast: `{ "type": "AdditionalCost", "cost": [...] }`; **alternative cost** (flashback etc.): `AlternativeCost { cost, from, exile_after }`.
* **Enters tapped / with counters**: `EntersTapped`, `EntersWithCounters { kind, n }`.

## More primitives (see the `Test ...` cards in `cards/core/` and their scenarios for working examples)

* **Replacement / prevention effects**: `{ "type": "Replace", "what": {...}, "condition": {...} }` with `what` one of `GainLife`, `LoseLife`, `Draw` (`except_first_in_draw_step`),
  `CreateTokens`, `AddCounters` (`who`, `on` filter, optional `kind`), `Damage` (`source` filter, `to`, `multiply`/`add`/`prevent`), `ZoneChange` (`from`, `to`, `instead`),
  `EntersTapped`, `EntersWithCounters`, `TriggerExtra` (Panharmonicon: `cause` filter + `event` EntersBattlefield/Dies/LeavesBattlefield). `multiply` applies first, then `add`.
* **Granted abilities**: static `{ "type": "GrantAbility", "affects": {filter}, "ability": { ...an Activated (incl. mana) ability... } }` (Chromatic Lantern); triggered abilities can be granted the same way.
* **Choose cards**: effect `{ "type": "Choose", "who": ..., "zone": "Battlefield", "filter": {...}, "min": 1, "max": 1, "then": [ ...effects using Sel Remembered... ] }`
  (bounce lands: choose a land you control, then `Bounce` `Remembered`; `controller` in the filter is relative to the chooser).
* **"You may pay X. If you do, ..."**: effect `MayPay { payer, cost: [...], effects: [...] }`. **"unless pays"**: `Unless`.
* **Activation restrictions**: `Activated.condition` ("activate only if you control five or more lands"). **Optional triggers**: `"optional": true` on a Triggered ability works (the controller is asked on resolution).
* **Mana**: `AddMana` entries `Fixed {m, n}`, `Scaled {m, amount}` (n computed), `AnyOneColor {n}` (n mana of one chosen color), `AnyOneColorScaled {amount}`, `AnyColor {n}`,
  `AnyCommanderColor` (commander's color identity), `AnyColorOpponentsCould` (colors lands an opponent controls could make). A mana ability may have a mana cost (filter lands: `{1},{T}: Add {W}{U}`).
  Lands with basic land types (granted by effects too) automatically get the intrinsic mana ability.
* **Equipment/Aura triggers**: `Filter.attached_by_source: true` = "the permanent this Equipment/Aura is attached to" (for "whenever equipped creature attacks").
* Static `NoMaxHandSize`, `MaxHandSize {n}`, `AdditionalLandDrop {n}`, `CostChange`, `CdaPT`, `EnchantedGets`, `EnchantedCant`
* Also: `Modal.allow_repeat`; `Filter.any_of` (OR) and `Filter.all_types` ("artifact creature"); `Replace` is the way to do "enters tapped unless" via `EntersTapped {unless}`; Amounts `HighestToughness`, `HighestManaValue`, `TotalPower`, `CardsDrawnThisTurn`, `LandsEnteredThisTurn`, `LifeGainedThisTurn`; `Sel TriggerSource` (the creature that dealt damage / the spell cast);
  statics `HandSizeUnlimited {who}`, `CantLoseGame {who}`, `CantBeCountered {filter}`, `AttackTax {generic}`; landwalk keywords; granted `Protection`; a mana ability may also have other effects (they happen immediately); tokens named "Beast 4-4" display as "Beast".
* **Scenario steps** can say `"illegal": true` (the cast/play_land/activate must not be legal) and `answers` may list repeated card names.

## Vocabulary cheat-sheet

* `Sel` (what an effect acts on): `Target {idx}`, `Targets`, `This`, `You`, `Owner`, `EachOpponent`, `EachPlayer`, `All {filter, zone}`,
  `TriggerObject`, `TriggerPlayer`, `AttachedTo`, `ControllerOf {of}`, `OwnerOf {of}`, `Remembered` (what an earlier effect in the same ability created or exiled).
* `TargetSpec`: `{ "what": {"type":"Permanent","filter":{...}} | {"type":"AnyTarget"} | {"type":"Player","filter":"Opponent"} | {"type":"CardIn","zone":"Graveyard","filter":{...},"whose":"You"} | {"type":"Spell","filter":{}}, "min":1, "max":1, "label":"..." }`.
* `Filter`: `types`, `not_types`, `subtypes`, `colors`, `not_colors`, `controller` (`Any`|`You`|`Opponent`), `tapped`, `token`, `other`, `this`, `power`/`toughness`/`mana_value` (`{"op":"Ge","n":4}`), `keywords`, `attacking`, `blocking`, `has_counter` ...
* `Amount`: `Fixed {n}`, `X`, `Count {filter, zone}`, `PowerOf {of}`, `LifeOf`, `HandSize`, `CountersOn`, `EventAmount`, `Plus`/`Minus`/`Times`/`Half`/`Max`/`Min`.
* `Condition`: `Not`, `All`, `AnyOf`, `LifeAtLeast`, `Control {who, filter, at_least}`, `YourTurn`, `Threshold`, `Delirium`, `Metalcraft`, `Compare {a, cmp}` ...
* `Trigger`: `EntersBattlefield`, `Dies`, `LeavesBattlefield`, `Attacks`, `Blocks`, `DealsCombatDamage`, `BeginningOfStep`, `SpellCast`, `Draws`, `GainsLife`, `Landfall`, `CounterAdded`, `TokenCreated`, ...
* `Effect`: DealDamage, Draw, GainLife, LoseLife, Destroy, Exile, Move, Bounce, CounterSpell, Pump, SetBasePT, GrantKeyword, AddCounters, CreateToken, CopyPermanent,
  Discard, Mill, Tap, Untap, Sacrifice, Scry, Surveil, SearchLibrary, LookAtTop, AddMana, GainControl, Fight, Bite, Transform, Attach, Proliferate, Investigate, Amass,
  If, May, Unless, Repeat, ForEach, Delayed, ... (see `cards/schema.json` for every field).

Costs: `Mana {cost: "{2}{R}"}`, `Tap`, `PayLife`, `SacrificeSelf`, `Sacrifice {filter,n}`, `Discard`, `ExileFromGraveyard`, `TapPermanents`, `RemoveCounters`, ...

## Scenario tests

Every card needs scenarios in `cards/tests/<Card_Name>.json` (same underscored name as its card file; a JSON list). Use the existing files as models; the format is documented at the top of `engine/crates/mtg-cards/src/scenario.rs`.

```json
[{ "name": "kills a creature",
   "players": [ { "hand": ["Lightning Bolt"], "battlefield": ["Mountain"] }, { "battlefield": ["Grizzly Bears"] } ],
   "steps": [ { "cast": "Lightning Bolt", "targets": ["Grizzly Bears"] }, { "resolve": true } ],
   "expect": { "players": [ { "graveyard": ["Lightning Bolt"] }, { "graveyard": ["Grizzly Bears"] } ] } }]
```

Steps: `cast` (+`targets`, `answers`, `alt`), `"illegal": true` (with cast/play_land/activate: the action must NOT be legal), `play_land`, `activate` (+`ability` = index among the card's printed non-mana activated abilities, in order), `resolve`, `pass`, `advance_to`, `attack`, `block`, `tap_for_mana`, `set_life`, `concede`.
`answers` answers questions in order (mode index, card names for a search/discard, `true`/`false`, X). Use `player: 1` to act as the other seat.
Write the *rules text's* expectations from the Oracle text and rulings - not from what the implementation does. Test the main effect and one boundary
(illegal target, "up to", a replacement, a condition false/true). Players start with 20 life and 20 filler cards in each library; `players[i].hand_count` etc. are checked only if given.


## When you can't express a card

Don't approximate. If the DSL can't express a card exactly, leave it out and add the missing primitive to the engine
(`engine/crates/mtg-engine/src/dsl.rs`) first. A card can be kept out of play temporarily by putting a file with its
underscored name in `cards/blocked/`: the server skips it and its scenarios may fail.

## Quality checklist (per card)

1. Name, cost, types, subtypes, P/T, loyalty exactly as Oracle. 2. Every ability in the Oracle text is present (don't drop reminder-only text, but don't model reminder text).
3. Targets/`optional`/`condition` match the text ("up to one", "you may", "if"). 4. `cards check` clean. 5. Scenarios pass. 6. `smoke` green.
