import { describe, expect, it } from 'vitest';
import { layoutBoard, toGameView, type Placed, type RCard, type RView } from './rulesView';

const player = (name: string) => ({
  name, life: 20, library: 0, hand: 0, schemes: 0, poison: 0, energy: 0, experience: 0, pool: [0, 0, 0, 0, 0, 0], commander_tax: [], commander_damage: [], lost: false, monarch: false, initiative: false,
});

const card = (id: number, name: string, types: string, controller: number, over: Partial<RCard> = {}): RCard =>
  ({
    id, name, zone: 'Battlefield', owner: controller, controller, tapped: false, face_down: false, token: false, power: null, toughness: null, base_power: null, base_toughness: null,
    types, colors: 0, cost: '', mana_value: 0, text: '', loyalty: null, counters: [], attached_to: null, summoning_sick: false, commander: false, keywords: [], abilities: [],
    alt_costs: [], granted_alts: [], copy_of: null, art: name, art_token: false, damage: 0, protector: null, enchanted_player: null, ...over,
  }) as RCard;

const snapOf = (cards: RCard[]) => ({ players: [player('A'), player('B')], cards, stack: [], planar_deck: 0, active: 0, turn: 1, step: 'Main1', day_night: null }) as unknown as RView;

const sideOf = (cards: RCard[], name: string): number => {
  const { view } = toGameView(snapOf(cards), {}, ['A', 'B'], 0, 'g', undefined, new Map(), new Map());
  const id = String(cards.find((c) => c.name === name)!.id);
  return view.seats.findIndex((s) => s.zones.battlefield.includes(id));
};

describe('which side of the table a permanent sits on', () => {
  it('a Curse sits with the player it enchants', () => {
    expect(sideOf([card(1, 'Curse of Opulence', 'Enchantment — Aura Curse', 0, { enchanted_player: 1 })], 'Curse of Opulence')).toBe(1);
  });

  it("an Aura on an opponent's creature sits with that creature", () => {
    const cards = [card(1, 'Bear', 'Creature — Bear', 1, { power: 2, toughness: 2 }), card(2, 'In Too Deep', 'Enchantment — Aura', 0, { attached_to: 1 })];
    expect(sideOf(cards, 'In Too Deep')).toBe(1);
  });

  it('a battle sits with its protector', () => {
    expect(sideOf([card(1, 'Invasion of Ergamon', 'Battle — Siege', 0, { protector: 1 })], 'Invasion of Ergamon')).toBe(1);
  });

  it('an Aura on its own creature stays with its controller', () => {
    const cards = [card(1, 'Bear', 'Creature — Bear', 0, { power: 2, toughness: 2 }), card(2, 'Rancor', 'Enchantment — Aura', 0, { attached_to: 1 })];
    expect(sideOf(cards, 'Rancor')).toBe(0);
  });
});

describe('the hand', () => {
  const hand = (ids: number[]) => ids.map((id) => card(id, `Card ${id}`, 'Instant', 0, { zone: 'Hand' }));
  const shown = (ids: number[], order: number[]) => toGameView(snapOf(hand(ids)), {}, ['A', 'B'], 0, 'g', undefined, new Map(), new Map(), undefined, undefined, order).view.seats[0].zones.hand;

  it('keeps the order the player gave it, and a drawn card goes at the end', () => {
    const order = [30, 10, 20];
    // the engine lists a hand in its own order; a new card (15) sorts between the others there
    expect(shown([10, 15, 20, 30], order)).toEqual(['30', '10', '20', '15']);
    expect(order).toEqual([30, 10, 20, 15]);
    // a card that leaves drops out, the rest keep their places
    expect(shown([15, 20, 30], order)).toEqual(['30', '20', '15']);
  });
});

describe('sorting a battlefield', () => {
  const bear = (id: number, name = `Bear ${id}`) => card(id, name, 'Creature — Bear', 0, { power: 2, toughness: 2 });
  const land = (id: number) => card(id, `Land ${id}`, 'Land', 0);

  it('keeps the left-to-right order and moves an already tidy board very little', () => {
    const cards = [bear(1, 'Zebra'), bear(2, 'Ape'), bear(3, 'Moose')];
    const current: Placed = new Map([[1, { x: 0.1, y: 0 }], [2, { x: 0.5, y: 0 }], [3, { x: 0.8, y: 0 }]]);
    const { pos } = layoutBoard(cards, 2, [], current);
    expect(pos.get(1)!.x).toBeLessThan(pos.get(2)!.x);
    expect(pos.get(2)!.x).toBeLessThan(pos.get(3)!.x);
    for (const id of [1, 2, 3]) expect(Math.abs(pos.get(id)!.x - current.get(id)!.x)).toBeLessThan(0.05);
  });

  it('puts a card in the wrong row in its own row at about the same place, and pushes apart only cards that overlap', () => {
    const cards = [bear(1), bear(2), land(3)];
    const current: Placed = new Map([[1, { x: 0.4, y: 0 }], [2, { x: 0.41, y: 0.1 }], [3, { x: 0.6, y: 0.2 }]]);
    const { pos } = layoutBoard(cards, 2, [], current);
    expect(pos.get(3)).toMatchObject({ y: 1 });
    expect(Math.abs(pos.get(3)!.x - 0.6)).toBeLessThan(0.05);
    expect(pos.get(1)!.y).toBe(0);
    expect(pos.get(2)!.y).toBe(0);
    expect(Math.abs(pos.get(1)!.x - 0.4)).toBeLessThan(0.05);
    expect(pos.get(2)!.x).toBeGreaterThan(pos.get(1)!.x + 0.05);
  });

  it('sorting twice changes nothing', () => {
    const cards = [bear(1), bear(2), bear(4), land(3)];
    const current: Placed = new Map([[1, { x: 0.3, y: 0.2 }], [2, { x: 0.31, y: 0 }], [3, { x: 0.9, y: 0.7 }], [4, { x: 1, y: 0 }]]);
    const first = layoutBoard(cards, 2, [], current).pos;
    const second = layoutBoard(cards, 2, [], first).pos;
    expect(second).toEqual(first);
  });

  it('sorting twice changes nothing when cards are crowded at the right edge or next to a pile', () => {
    const edge = [bear(1), bear(2)];
    const atEdge: Placed = new Map([[1, { x: 0.99, y: 0 }], [2, { x: 1, y: 0 }]]);
    const once = layoutBoard(edge, 2, [], atEdge).pos;
    expect(layoutBoard(edge, 2, [], once).pos).toEqual(once);

    const pile = [bear(1, 'Goblin'), bear(2, 'Goblin'), bear(3, 'Goblin'), bear(4)];
    const near: Placed = new Map([[1, { x: 0.2, y: 0 }], [2, { x: 0.2, y: 0 }], [3, { x: 0.2, y: 0 }], [4, { x: 0.21, y: 0 }]]);
    const sorted = layoutBoard(pile, 2, [], near);
    expect(sorted.piles).toEqual([[1, 2, 3]]);
    expect(layoutBoard(pile, 2, sorted.piles, sorted.pos).pos).toEqual(sorted.pos);
  });

  it('leaves out a card attached to another, which is drawn under its host', () => {
    const cards = [bear(1), card(2, 'Rancor', 'Enchantment — Aura', 0, { attached_to: 1 })];
    const { pos } = layoutBoard(cards, 2, [], new Map([[1, { x: 0.3, y: 0 }], [2, { x: 0.3, y: 0.5 }]]));
    expect(pos.has(2)).toBe(false);
    expect(pos.get(1)).toEqual({ x: 0.3, y: 0 });
  });
});
