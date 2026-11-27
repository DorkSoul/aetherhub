import { describe, expect, it } from 'vitest';
import type { rules as R } from '@aetherhub/shared';
import { describeFrame, type LogContext } from './rulesLog';

type Card = { id: number; name: string; zone?: string; controller?: number; owner?: number; types?: string; power?: number | null; toughness?: number | null };

/** A board with just what the log reads: cards (name, zone, controller, types, power / toughness) and life totals. */
function board(cards: Card[], life = [20, 20]): R.View {
  return {
    cards: cards.map((c) => ({ zone: 'Battlefield', controller: 0, owner: c.owner ?? c.controller ?? 0, types: '', power: null, toughness: null, ...c })),
    players: life.map((l) => ({ life: l })),
  } as unknown as R.View;
}
const ctx = (): LogContext => ({ names: ['Alice', 'Bot'], known: new Map() });
const ev = (e: unknown) => e as R.Event;

describe('the rules game log', () => {
  it('says what a spell targets and how it was paid for', () => {
    const v = board([
      { id: 1, name: 'Mountain' },
      { id: 2, name: 'Lightning Bolt', zone: 'Graveyard' },
      { id: 3, name: 'Grizzly Bears', controller: 1, types: 'Creature — Bear' },
    ]);
    const lines = describeFrame(
      [
        ev({ t: 'Tapped', card: 1 }),
        ev({ t: 'Mana', player: 0, mana: 'R' }),
        ev({ t: 'ManaTapped', card: 1, player: 0 }),
        ev({ t: 'Activated', card: 1, ability: 0, player: 0, life: 0, mana: true }),
        ev({ t: 'Cast', card: 2, player: 0 }),
        ev({ t: 'StackPut', id: 7, targets: 1, text: '', x: 0 }),
        ev({ t: 'Targeted', card: 3, by: 2, controller: 0, spell: true, sid: 7 }),
        ev({ t: 'Damage', source: 2, target: { kind: 'Card', id: 3 }, amount: 3, combat: false }),
        ev({ t: 'Moved', card: 3, from: 'Battlefield', to: 'Graveyard' }),
      ],
      v,
      null,
      ctx(),
    );
    expect(lines).toEqual(['Alice casts Lightning Bolt, targeting Grizzly Bears — paying {R} (tapping Mountain)', 'Lightning Bolt deals 3 damage to Grizzly Bears', 'Grizzly Bears dies']);
  });

  it('names which ability was activated, with its text, X, targets and every cost', () => {
    const v = board([
      { id: 1, name: 'Swamp' },
      { id: 4, name: 'Blood Pet', zone: 'Graveyard', types: 'Creature' },
      { id: 5, name: 'Altar of Doom' },
    ]);
    const lines = describeFrame(
      [
        ev({ t: 'Mana', player: 0, mana: 'B' }),
        ev({ t: 'ManaTapped', card: 1, player: 0 }),
        ev({ t: 'Sacrificed', card: 4, player: 0 }),
        ev({ t: 'Moved', card: 4, from: 'Battlefield', to: 'Graveyard' }),
        ev({ t: 'LifePaid', player: 0, n: 2 }),
        ev({ t: 'Activated', card: 5, ability: 1, player: 0, life: 2, mana: false }),
        ev({ t: 'StackPut', id: 9, targets: 1, text: '{X}{B}, Sacrifice a creature, Pay 2 life: Target player loses X life.', x: 3 }),
        ev({ t: 'PlayerTargeted', player: 1, by: 5, controller: 0, spell: false, sid: 9 }),
      ],
      v,
      null,
      ctx(),
    );
    expect(lines).toEqual(['Alice activates Altar of Doom: “{X}{B}, Sacrifice a creature, Pay 2 life: Target player loses X life.” (X = 3), targeting Bot — paying {B} (tapping Swamp), sacrificing Blood Pet and paying 2 life']);
  });

  it('shows what a triggered ability does, and life changes with the total they leave', () => {
    const v = board([{ id: 6, name: 'Soul Warden' }], [21, 20]);
    const lines = describeFrame(
      [ev({ t: 'AbilityTriggered', id: 3, source: 6, player: 0, entering: true, attacking: false }), ev({ t: 'StackPut', id: 3, targets: 0, text: 'Whenever another creature enters, you gain 1 life.', x: 0 }), ev({ t: 'Life', player: 0, delta: 1 })],
      v,
      board([{ id: 6, name: 'Soul Warden' }]),
      ctx(),
    );
    expect(lines).toEqual(['Soul Warden (Alice) triggers: “Whenever another creature enters, you gain 1 life.”', 'Alice gains 1 life (now 21)']);
  });

  it('groups an attack, and puts damage to a player and the life it costs on one line', () => {
    const cards = [
      { id: 10, name: 'Goblin', controller: 1, types: 'Creature' },
      { id: 11, name: 'Goblin', controller: 1, types: 'Creature' },
      { id: 12, name: 'Ogre', controller: 1, types: 'Creature' },
    ];
    const lines = describeFrame(
      [
        ev({ t: 'Tapped', card: 10 }),
        ev({ t: 'Attacked', card: 10, defender: { kind: 'Player', id: 0 } }),
        ev({ t: 'Tapped', card: 11 }),
        ev({ t: 'Attacked', card: 11, defender: { kind: 'Player', id: 0 } }),
        ev({ t: 'Tapped', card: 12 }),
        ev({ t: 'Attacked', card: 12, defender: { kind: 'Player', id: 0 } }),
        ev({ t: 'Damage', source: 12, target: { kind: 'Player', id: 0 }, amount: 3, combat: true }),
        ev({ t: 'Life', player: 0, delta: -3 }),
      ],
      board(cards, [17, 20]),
      board(cards),
      ctx(),
    );
    expect(lines).toEqual(['Bot attacks Alice with 2× Goblin and Ogre', 'Ogre deals 3 combat damage to Alice (17 life left)']);
  });

  it('names your own draws, counts hidden ones, and notices creatures changing size', () => {
    const before = board([{ id: 20, name: 'Grizzly Bears', types: 'Creature', power: 2, toughness: 2 }]);
    const after = board([
      { id: 20, name: 'Grizzly Bears', types: 'Creature', power: 5, toughness: 5 },
      { id: 21, name: 'Giant Growth', zone: 'Hand' },
    ]);
    const lines = describeFrame([ev({ t: 'Drew', player: 0, card: 21 }), ev({ t: 'Drew', player: 1, card: 30 }), ev({ t: 'Drew', player: 1, card: 31 })], after, before, ctx());
    expect(lines).toEqual(['Alice draws Giant Growth', 'Bot draws 2 cards', 'Grizzly Bears is now 5/5 (was 2/2)']);
  });

  it('writes costs that paid for nothing on their own (mana added, an edict sacrifice)', () => {
    const v = board([{ id: 1, name: 'Forest' }, { id: 2, name: 'Elf', zone: 'Graveyard', controller: 1, owner: 1, types: 'Creature' }]);
    const lines = describeFrame(
      [ev({ t: 'Mana', player: 0, mana: 'G' }), ev({ t: 'ManaTapped', card: 1, player: 0 }), ev({ t: 'Resolved', card: 99 }), ev({ t: 'Sacrificed', card: 2, player: 1 }), ev({ t: 'Moved', card: 2, from: 'Battlefield', to: 'Graveyard' })],
      v,
      null,
      ctx(),
    );
    expect(lines).toEqual(['Alice adds {G} (tapping Forest)', 'Bot sacrifices Elf']);
  });

  it('keeps the name of a card that is hidden again later', () => {
    const c = ctx();
    describeFrame([], board([{ id: 40, name: 'Llanowar Elves', types: 'Creature' }]), null, c);
    expect(describeFrame([ev({ t: 'Moved', card: 40, from: 'Battlefield', to: 'Library' })], board([]), null, c)).toEqual(["Llanowar Elves is put into Alice's library"]);
  });
});
