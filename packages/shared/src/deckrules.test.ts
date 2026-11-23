import { describe, expect, it } from 'vitest';
import type { CardDef } from './cards';
import { checkLegality, type DeckCard } from './deck';

let n = 0;
function def(name: string, typeLine: string, ci: string[], extra: Partial<CardDef> = {}): CardDef {
  return { key: name, name, layout: 'normal', manaCost: '', cmc: 3, typeLine, oracleText: '', colors: ci, colorIdentity: ci, img: [], legalities: { commander: 'legal' }, ...extra } as CardDef;
}
function deck(cmd: CardDef, cards: [CardDef, number][], filler = 0): { cards: DeckCard[]; defs: Record<string, CardDef> } {
  const defs: Record<string, CardDef> = { [cmd.key]: cmd };
  const out: DeckCard[] = [{ id: ++n, def: cmd.key, qty: 1, section: 'commander' } as DeckCard];
  for (const [d, q] of cards) {
    defs[d.key] = d;
    out.push({ id: ++n, def: d.key, qty: q, section: 'main' } as DeckCard);
  }
  if (filler) {
    const w = def('Wastes', 'Basic Land', []);
    defs[w.key] = w;
    out.push({ id: ++n, def: w.key, qty: filler, section: 'main' } as DeckCard);
  }
  return { cards: out, defs };
}
const warn = (d: ReturnType<typeof deck>) => checkLegality('commander', d.cards, d.defs);

const plains = def('Plains', 'Basic Land — Plains', ['W']);
const angelR = def('Red Angel', 'Creature — Angel', ['R']);
const goblinR = def('Red Goblin', 'Creature — Goblin', ['R']);

describe('commander deck rules (Rulebreaker and friends)', () => {
  it('an ordinary commander keeps colour identity', () => {
    const r = warn(deck(def('Mono Black', 'Legendary Creature — Human', ['B']), [[angelR, 1], [plains, 1]], 97));
    expect(r.warnings.some((w) => w.startsWith('Red Angel'))).toBe(true);
    expect(r.warnings.some((w) => w.startsWith('Plains'))).toBe(true);
    expect(r.notes).toEqual([]);
  });

  it('Seluma: Angels of any colour and any basic lands, other cards still checked', () => {
    const seluma = def('Seluma, Light of Aysen', 'Legendary Creature — Angel Warrior', ['B']);
    const r = warn(deck(seluma, [[angelR, 1], [plains, 1], [goblinR, 1]], 96));
    expect(r.warnings.filter((w) => w.startsWith('Red Angel') || w.startsWith('Plains'))).toEqual([]);
    expect(r.warnings.some((w) => w.startsWith('Red Goblin'))).toBe(true);
    expect(r.notes[0]).toMatch(/Seluma.*Angel/);
    // the playtest commander itself is not flagged as "not legal"
    const notLegal = { ...seluma, legalities: { commander: 'not_legal' } } as CardDef;
    expect(warn(deck(notLegal, [], 99)).warnings).toEqual([]);
  });

  it('The Everforger: artifact creatures and Equipment', () => {
    const ever = def('The Everforger', 'Legendary Artifact Creature — Construct', []);
    const golem = def('Blue Golem', 'Artifact Creature — Golem', ['U']);
    const sword = def('Green Sword', 'Artifact — Equipment', ['G']);
    const relic = def('Red Relic', 'Artifact', ['R']);
    const r = warn(deck(ever, [[golem, 1], [sword, 1], [relic, 1]], 96));
    expect(r.warnings.filter((w) => /Golem|Sword/.test(w))).toEqual([]);
    expect(r.warnings.some((w) => w.startsWith('Red Relic'))).toBe(true);
  });

  it('Maular: creatures of mana value 7 or more', () => {
    const maular = def('Maular, the Next Evolution', 'Legendary Creature — Dinosaur Mutant', ['G']);
    const big = def('Big Blue', 'Creature — Leviathan', ['U'], { cmc: 8 });
    const small = def('Small Blue', 'Creature — Fish', ['U'], { cmc: 2 });
    const r = warn(deck(maular, [[big, 1], [small, 1]], 97));
    expect(r.warnings.some((w) => w.startsWith('Big Blue'))).toBe(false);
    expect(r.warnings.some((w) => w.startsWith('Small Blue'))).toBe(true);
  });

  it('Grizzlegom: any land cards', () => {
    const g = def('Grizzlegom, Hurloon Hero', 'Legendary Creature — Minotaur Warrior', ['R']);
    const dual = def('Blue White Dual', 'Land', ['W', 'U']);
    const r = warn(deck(g, [[dual, 1], [plains, 1]], 97));
    expect(r.warnings.filter((w) => /Dual|Plains/.test(w))).toEqual([]);
  });

  it('Whtz: no maximum deck size, minimum still 100', () => {
    const whtz = def('Whtz, the Bibliophile', 'Legendary Creature — Homunculus', ['U']);
    expect(warn(deck(whtz, [], 249)).deckWarnings).toEqual([]);
    expect(warn(deck(whtz, [], 80)).deckWarnings[0]).toMatch(/81 of 100/);
  });

  it('The Zonian Brawler: 60 to 100 cards, life 30', () => {
    const z = def('The Zonian Brawler', 'Legendary Creature — Human', ['R']);
    const r = warn(deck(z, [], 59));
    expect(r.deckWarnings).toEqual([]);
    expect(r.life).toBe(30);
    expect(warn(deck(z, [], 40)).deckWarnings[0]).toMatch(/60–100/);
  });

  it('Tolabow: instants and sorceries may add one colour, the same one for all', () => {
    const t = def('Tolabow, Loch Rascal', 'Legendary Creature — Otter', ['U', 'R']);
    const shock = def('Black Shock', 'Instant', ['B']);
    const bolt = def('Black Bolt', 'Sorcery', ['B']);
    const growth = def('Green Growth', 'Instant', ['G']);
    const bear = def('Black Bear', 'Creature — Bear', ['B']);
    const ok = warn(deck(t, [[shock, 1], [bolt, 1], [bear, 1]], 96));
    expect(ok.warnings.filter((w) => /Shock|Bolt/.test(w))).toEqual([]);
    expect(ok.warnings.some((w) => w.startsWith('Black Bear'))).toBe(true);
    const two = warn(deck(t, [[shock, 1], [growth, 1]], 97));
    expect(two.warnings.some((w) => /Only one extra color/.test(w))).toBe(true);
  });

  it('unknown future Rulebreaker text is read generically', () => {
    const c = def('Some New Commander', 'Legendary Creature — Elf', ['G'], { oracleText: 'Rulebreaker — A deck with this commander can have Goblin cards of any color identity and any basic land cards.' });
    const r = warn(deck(c, [[goblinR, 1], [plains, 1], [angelR, 1]], 96));
    expect(r.warnings.filter((w) => /Goblin|Plains/.test(w))).toEqual([]);
    expect(r.warnings.some((w) => w.startsWith('Red Angel'))).toBe(true);
  });

  it('copy-limit cards were already fine (Rat Colony)', () => {
    const rat = def('Rat Colony', 'Creature — Rat', ['B'], { oracleText: 'A deck can have any number of cards named Rat Colony.' });
    const r = warn(deck(def('Rat King', 'Legendary Creature — Rat', ['B']), [[rat, 99]]));
    expect(r.warnings).toEqual([]);
  });
});
