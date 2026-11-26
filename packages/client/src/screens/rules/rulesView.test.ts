import { describe, expect, it } from 'vitest';
import { ptOf } from '../table/pt';
import { layoutBoard, markTightPiles, optionsOf, toGameView, type Placed, type RCard, type RView } from './rulesView';

const player = (name: string) => ({
  name, life: 20, library: 0, hand: 0, schemes: 0, poison: 0, energy: 0, experience: 0, pool: [0, 0, 0, 0, 0, 0], commander_tax: [], commander_damage: [], lost: false, monarch: false, initiative: false,
});

const card = (id: number, name: string, types: string, controller: number, over: Partial<RCard> = {}): RCard =>
  ({
    id, name, zone: 'Battlefield', owner: controller, controller, tapped: false, face_down: false, token: false, power: null, toughness: null, base_power: null, base_toughness: null,
    types, colors: 0, cost: '', mana_value: 0, text: '', loyalty: null, counters: [], attached_to: null, summoning_sick: false, commander: false, keywords: [], abilities: [],
    alt_costs: [], granted_alts: [], copy_of: null, art: name, art_token: false, damage: 0, protector: null, enchanted_player: null, doors: null, ...over,
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

describe('power and toughness on the table', () => {
  const shownPT = (c: RCard) => {
    const { view } = toGameView(snapOf([c]), {}, ['A', 'B'], 0, 'g', undefined, new Map(), new Map());
    const inst = view.cards[String(c.id)];
    return ptOf(inst, view.defs[inst.def])?.txt ?? null;
  };

  it('a */* creature always shows its current P/T', () => {
    expect(shownPT(card(1, 'Old Stickfingers', 'Legendary Creature — Horror', 0, { power: 2, toughness: 2 }))).toBe('2/2');
  });

  it('an animated land shows its P/T', () => {
    expect(shownPT(card(1, 'Mutavault', 'Land Creature — Shapeshifter', 0, { power: 2, toughness: 2 }))).toBe('2/2');
  });

  it('a printed P/T shows only when changed', () => {
    expect(shownPT(card(1, 'Bear', 'Creature — Bear', 0, { power: 2, toughness: 2, base_power: 2, base_toughness: 2 }))).toBe(null);
    expect(shownPT(card(1, 'Bear', 'Creature — Bear', 0, { power: 3, toughness: 3, base_power: 2, base_toughness: 2 }))).toBe('3/3');
    expect(shownPT(card(1, 'Lhurgoyf', 'Creature — Lhurgoyf', 0, { power: 2, toughness: 3, base_power: 0, base_toughness: 1 }))).toBe('2/3');
  });
});

describe('flip cards', () => {
  const images = { 'Rune-Tail, Kitsune Ascendant': 'p/x/0', "Rune-Tail's Essence": 'p/x/0', '@layout:Rune-Tail, Kitsune Ascendant': 'flip', "@layout:Rune-Tail's Essence": 'flip', "@flipped:Rune-Tail's Essence": '1' };
  const flipped = (c: RCard) => toGameView(snapOf([c]), images, ['A', 'B'], 0, 'g', undefined, new Map(), new Map()).view.cards[String(c.id)].flipped;

  it('a flipped permanent is turned 180°, the unflipped one is not', () => {
    expect(flipped(card(1, "Rune-Tail's Essence", 'Legendary Enchantment', 0))).toBe(true);
    expect(flipped(card(1, 'Rune-Tail, Kitsune Ascendant', 'Legendary Creature — Fox Monk', 0, { power: 2, toughness: 2 }))).toBe(false);
  });

  it('only on the battlefield', () => {
    expect(flipped(card(1, "Rune-Tail's Essence", 'Legendary Enchantment', 0, { zone: 'Exile' }))).toBe(false);
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
    expect(sorted.piles).toEqual([{ ids: [1, 2, 3], mode: 'auto' }]);
    expect(layoutBoard(pile, 2, sorted.piles, sorted.pos).pos).toEqual(sorted.pos);
  });

  it('leaves out a card attached to another, which is drawn under its host', () => {
    const cards = [bear(1), card(2, 'Rancor', 'Enchantment — Aura', 0, { attached_to: 1 })];
    const { pos } = layoutBoard(cards, 2, [], new Map([[1, { x: 0.3, y: 0 }], [2, { x: 0.3, y: 0.5 }]]));
    expect(pos.has(2)).toBe(false);
    expect(pos.get(1)).toEqual({ x: 0.3, y: 0 });
  });

  it('fans a pile out until it holds 7 cards, then lays them exactly on top of each other; new copies join it', () => {
    const goblins = (n: number) => Array.from({ length: n }, (_, i) => bear(i + 1, 'Goblin'));
    const six = layoutBoard(goblins(6), 2, [], new Map());
    expect(six.piles).toEqual([{ ids: [1, 2, 3, 4, 5, 6], mode: 'auto' }]);
    expect(new Set([1, 2, 3, 4, 5, 6].map((id) => six.pos.get(id)!.x)).size).toBe(6);
    // a seventh copy arrives: it joins the pile, which now lies in one spot
    const seven = layoutBoard(goblins(7), 2, six.piles, six.pos);
    expect(seven.piles).toEqual([{ ids: [1, 2, 3, 4, 5, 6, 7], mode: 'auto' }]);
    expect(new Set([1, 2, 3, 4, 5, 6, 7].map((id) => seven.pos.get(id)!.x)).size).toBe(1);
    // one the player dragged out of the pile stays out
    expect(layoutBoard(goblins(7), 2, [{ ids: [1, 2, 3, 4, 5, 6], mode: 'auto' }], six.pos, new Set([7])).piles).toEqual([{ ids: [1, 2, 3, 4, 5, 6], mode: 'auto' }]);
  });

  it('keeps the style the player chose: a pile of two stacked with a count, a big pile fanned out', () => {
    const cards = [bear(1, 'Goblin'), bear(2, 'Goblin'), ...Array.from({ length: 8 }, (_, i) => bear(10 + i, 'Elf'))];
    const elves = Array.from({ length: 8 }, (_, i) => 10 + i);
    const { pos } = layoutBoard(cards, 2, [{ ids: [1, 2], mode: 'tight' }, { ids: elves, mode: 'fan' }], new Map());
    expect(pos.get(1)).toEqual(pos.get(2));
    expect(new Set(elves.map((id) => pos.get(id)!.x)).size).toBe(8);
  });

  it('puts the count on the top card of a tight pile only', () => {
    const cards = [bear(1, 'Goblin'), bear(2, 'Goblin'), bear(3, 'Goblin'), bear(4, 'Elf'), bear(5, 'Elf')];
    const { view } = toGameView(snapOf(cards), {}, ['A', 'B'], 0, 'g', undefined, new Map(), new Map());
    markTightPiles(view, [{ ids: [1, 2, 3], mode: 'tight' }, { ids: [4, 5], mode: 'fan' }]);
    const bf = view.seats[0].zones.battlefield;
    const top = [...bf].filter((i) => ['1', '2', '3'].includes(i)).pop()!;
    expect(view.cards[top].stacked).toBe(3);
    expect(bf.filter((i) => view.cards[i].stacked).length).toBe(1);
  });
});

describe('Rooms', () => {
  it('names the door being cast or unlocked, with its cost and text', () => {
    const reminder = '(You may cast either half. That door unlocks on the battlefield. As a sorcery, you may pay the mana cost of a locked door to unlock it.)';
    const room = card(1, 'Dazzling Theater // Prop Room', 'Enchantment — Room', 0, {
      zone: 'Hand', cost: '{3}{W}', alt_costs: ['{2}{W}'],
      text: `Creature spells you cast have convoke.\n${reminder}\n//\nUntap each creature you control during each other player's untap step.\n${reminder}`,
    });
    const v = snapOf([room]);
    const d = { player: 0, kind: { t: 'Priority', stack_len: 0, actions: [{ t: 'Cast', card: 1, alt: null }, { t: 'Cast', card: 1, alt: 0 }, { t: 'UnlockDoor', card: 1, door: 2 }] } } as never;
    expect(optionsOf(d, v)!.map((o) => o.label)).toEqual([
      'Cast Dazzling Theater ({3}{W}): Creature spells you cast have convoke.',
      "Cast Prop Room ({2}{W}): Untap each creature you control during each other player's untap step.",
      "Unlock Prop Room ({2}{W}): Untap each creature you control during each other player's untap step.",
    ]);
  });

  it('marks the locked doors of a Room on the battlefield', () => {
    const room = card(1, 'Dazzling Theater // Prop Room', 'Enchantment — Room', 0, { doors: 1 });
    const { view } = toGameView(snapOf([room]), {}, ['A', 'B'], 0, 'g', undefined, new Map(), new Map());
    expect(view.cards['1'].locked).toEqual([2]);
    expect(view.cards['1'].effects.map((e) => e.k)).toContain('Locked: Prop Room');
    const open = toGameView(snapOf([{ ...room, doors: 3 }]), {}, ['A', 'B'], 0, 'g', undefined, new Map(), new Map()).view;
    expect(open.cards['1'].locked).toBeUndefined();
  });
});
