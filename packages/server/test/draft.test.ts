import { describe, expect, it } from 'vitest';
import { AiDrafter, baseRating, boosterKind, buildDeck, Draft, makeBooster, rng, suggestBasics, type PoolCard, type SetPool } from '../src/draft';

const card = (i: number, rarity: string, colors: string, extra: Partial<PoolCard> = {}): PoolCard => ({
  key: `p:${i}`,
  name: `Card ${i}`,
  rarity,
  cmc: 2 + (i % 4),
  colors,
  identity: colors,
  typeLine: 'Creature — Test',
  text: '',
  manaCost: colors ? `{${2 + (i % 4) - 1}}{${colors[0]}}` : '{2}',
  power: '2',
  toughness: '2',
  ...extra,
});
const CARDS: PoolCard[] = [
  ...Array.from({ length: 80 }, (_, i) => card(i, 'common', 'WUBRG'[i % 5])),
  ...Array.from({ length: 40 }, (_, i) => card(100 + i, 'uncommon', 'WUBRG'[i % 5])),
  ...Array.from({ length: 20 }, (_, i) => card(200 + i, 'rare', 'WUBRG'[i % 5])),
  ...Array.from({ length: 6 }, (_, i) => card(300 + i, 'mythic', 'WUBRG'[i % 5])),
];
const BASICS: PoolCard[] = ['Plains', 'Island', 'Swamp', 'Mountain', 'Forest'].map((n, i) => card(900 + i, 'common', '', { name: n, typeLine: 'Basic Land — ' + n, identity: 'WUBRG'[i], manaCost: '' }));
const OLD: SetPool = { code: 'old', name: 'Old Set', released: '2019-01-01', cards: CARDS, basics: BASICS };
const NEW: SetPool = { code: 'new', name: 'New Set', released: '2025-01-01', cards: CARDS, basics: BASICS };

describe('boosters', () => {
  it('Draft Booster (before 2024): 15 cards, 10 commons with every colour, 3 uncommons, 1 rare or mythic, 1 basic land', () => {
    expect(boosterKind(OLD.released)).toBe('draft');
    for (let seed = 1; seed < 20; seed++) {
      const b = makeBooster(OLD, rng(seed));
      expect(b).toHaveLength(15);
      expect(b.filter((c) => c.rarity === 'rare' || c.rarity === 'mythic')).toHaveLength(1);
      expect(b.filter((c) => c.rarity === 'uncommon')).toHaveLength(3);
      expect(b.filter((c) => /Basic Land/.test(c.typeLine))).toHaveLength(1);
      const commons = b.filter((c) => c.rarity === 'common' && !/Basic/.test(c.typeLine));
      expect(commons).toHaveLength(10);
      for (const col of 'WUBRG') expect(commons.some((c) => c.colors === col)).toBe(true);
      expect(new Set(b.map((c) => c.name)).size).toBe(15);
    }
  });
  it('Play Booster (2024 on): 14 cards with two wildcards and a land', () => {
    expect(boosterKind(NEW.released)).toBe('play');
    const b = makeBooster(NEW, rng(3));
    expect(b).toHaveLength(14);
    expect(b.filter((c) => c.rarity === 'common' && !/Basic/.test(c.typeLine)).length).toBeGreaterThanOrEqual(7);
    expect(b.filter((c) => /Basic Land/.test(c.typeLine))).toHaveLength(1);
  });
  it('mythics come about one pack in eight, deterministic per seed, and a tiny pool still makes a pack', () => {
    let mythic = 0;
    for (let s = 0; s < 800; s++) if (makeBooster(OLD, rng(s)).some((c) => c.rarity === 'mythic')) mythic++;
    expect(mythic).toBeGreaterThan(60);
    expect(mythic).toBeLessThan(150);
    expect(makeBooster(OLD, rng(5)).map((c) => c.key)).toEqual(makeBooster(OLD, rng(5)).map((c) => c.key));
    expect(makeBooster(CARDS.slice(0, 3), rng(2), 'draft').length).toBeGreaterThan(10);
  });
});

describe('AI drafters', () => {
  it('rate removal, evasion and rares above vanilla commons', () => {
    const bolt = card(1, 'common', 'R', { text: 'Lightning deals 3 damage to any target.', typeLine: 'Instant' });
    const flyer = card(4, 'common', 'U', { text: 'Flying' });
    const vanilla = card(2, 'common', 'R');
    expect(baseRating(bolt)).toBeGreaterThan(baseRating(vanilla));
    expect(baseRating(flyer)).toBeGreaterThan(baseRating(vanilla));
    expect(baseRating(card(3, 'rare', 'R'))).toBeGreaterThan(baseRating(vanilla));
  });
  it('commit to their colours after a few picks, and read what is flowing', () => {
    const ai = new AiDrafter(rng(1));
    for (let i = 0; i < 6; i++) ai.take(card(i, 'common', 'R'));
    expect(ai.main()).toContain('R');
    const pack = [card(50, 'common', 'U'), card(51, 'common', 'R')];
    expect(pack[ai.pick(pack, 6)].colors).toBe('R');
    // strong green cards keep coming late: green becomes one of its colours
    const reader = new AiDrafter(rng(2));
    for (let i = 0; i < 5; i++) reader.take(card(i, 'common', 'W'));
    for (let p = 2; p < 10; p++) reader.notice([card(600 + p, 'rare', 'G', { text: 'Flying' })], p);
    expect(reader.main()).toContain('G');
  });
  it('build a 40-card two-colour deck with about 17 lands from a pool', () => {
    const pool = Array.from({ length: 45 }, (_, i) => CARDS[(i * 7) % CARDS.length]);
    const d = buildDeck(pool);
    const n = d.main.length + Object.values(d.basics).reduce((a, b) => a + b, 0);
    expect(n).toBe(40);
    expect(d.colors).toHaveLength(2);
    expect(d.main.length).toBeGreaterThanOrEqual(22);
    expect(Object.keys(d.basics).length).toBeGreaterThan(0);
    // cards the rules engine can't play stay out
    const none = buildDeck(pool, (c) => c.colors !== 'W');
    const byKey = new Map(pool.map((c) => [c.key, c]));
    expect(none.main.every((k) => byKey.get(k)!.colors !== 'W')).toBe(true);
  });
  it('suggest basic lands by the mana symbols of the chosen cards', () => {
    const chosen = [card(1, 'common', 'R'), card(2, 'common', 'R'), card(3, 'common', 'G')];
    const b = suggestBasics(chosen);
    expect((b.Mountain ?? 0) + (b.Forest ?? 0)).toBe(17);
    expect(b.Mountain).toBeGreaterThan(b.Forest);
  });
});

describe('draft pod', () => {
  it('runs a full draft with one human and seven bots, passing left-right-left', () => {
    const d = new Draft([OLD], 'draft', 11);
    d.addHuman(1, 'alice');
    d.start();
    expect(d.phase).toBe('drafting');
    expect(d.seats).toHaveLength(8);
    let picks = 0;
    while (d.phase === 'drafting') {
      const me = d.seats[0];
      expect(me.pack.length).toBe(15 - d.pickNo);
      d.pick(1, me.pack[0].key);
      picks++;
      expect(picks).toBeLessThanOrEqual(45);
    }
    // the last card of each pack is taken automatically
    expect(picks).toBe(42);
    expect(d.phase).toBe('building');
    for (const s of d.seats) expect(s.pool).toHaveLength(45);
    // the bots built decks
    for (const s of d.seats.slice(1)) expect(s.built?.main.length).toBeGreaterThan(20);
  });
  it('a pod of exactly the lobby seats (two humans, one AI) waits for every human before passing', () => {
    const d = new Draft([OLD], 'draft', 3, 3);
    d.addHuman(1, 'a');
    d.addHuman(2, 'b');
    d.addAi('AI Ava');
    d.start(false);
    expect(d.seats).toHaveLength(3);
    d.pick(1, d.seats[0].pack[0].key);
    expect(() => d.pick(1, d.seats[0].pack[0]?.key ?? 'x')).toThrow(/already picked/);
    expect(d.seats[1].picked).toBe(false);
    d.pick(2, d.seats[1].pack[0].key);
    expect(d.pickNo).toBe(1);
  });
  it('one player can hold several seats of the pod (a solo draft)', () => {
    const d = new Draft([OLD], 'draft', 9, 2);
    d.addHuman(1, 'alice 1');
    d.seats.push({ userId: 1, name: 'alice 2', pool: [], pack: [], picked: false, deckId: null, built: null });
    d.start(false);
    d.pick(1, d.seats[0].pack[0].key, 0);
    d.pick(1, d.seats[1].pack[0].key, 1);
    expect(d.pickNo).toBe(1);
  });
  it('the pick timer picks for a player who runs out of time', () => {
    const d = new Draft([OLD], 'draft', 5, 2, true);
    d.addHuman(1, 'slow');
    d.addAi('AI Bryn');
    d.start(false);
    expect(d.deadline).not.toBeNull();
    expect(d.tick(Date.now())).toBe(false);
    expect(d.tick(d.deadline! + 1)).toBe(true);
    expect(d.seats[0].pool).toHaveLength(1);
    expect(d.pickNo).toBe(1);
  });
  it('sealed gives each player six boosters', () => {
    const d = new Draft([OLD], 'sealed', 4);
    d.addHuman(1, 'alice');
    d.start();
    expect(d.phase).toBe('building');
    expect(d.seats).toHaveLength(1);
    expect(d.seats[0].pool).toHaveLength(90);
  });
  it('rejects picking a card that is not in the pack', () => {
    const d = new Draft([OLD], 'draft', 3);
    d.addHuman(1, 'a');
    d.start();
    expect(() => d.pick(1, 'p:nope')).toThrow(/not in your pack/);
  });
  it('saves to JSON and carries on after a restore', () => {
    const d = new Draft([OLD], 'draft', 21, 3);
    d.addHuman(1, 'a');
    d.addAi('AI Cora');
    d.addAi('AI Dex');
    d.start(false);
    d.pick(1, d.seats[0].pack[0].key);
    const back = Draft.fromJSON(JSON.parse(JSON.stringify(d.toJSON())));
    expect(back.pickNo).toBe(d.pickNo);
    expect(back.seats[0].pool.map((c) => c.key)).toEqual(d.seats[0].pool.map((c) => c.key));
    expect(back.seats[1].ai?.picks).toHaveLength(d.seats[1].ai!.picks.length);
    while (back.phase === 'drafting') back.pick(1, back.seats[0].pack[0].key);
    expect(back.phase).toBe('building');
  });
});
