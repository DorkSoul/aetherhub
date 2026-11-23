import { describe, expect, it } from 'vitest';
import type { CardDef } from './cards';
import { canBeCommander, manaValue, normName, pipCounts } from './cards';
import { checkLegality, deckStats, deckTagNames, groupCards, normalizeTag, type DeckCard } from './deck';
import { applyPatch, diff } from './diff';
import { detectFormat, exportDeck, levenshtein, parseDeckText } from './importexport';
import { matchQuery, parseSearch } from './search';
import { accentVars } from './theme';
import { bfFrac, bfPos, handLayout, gridSlots } from './game/layout';

function def(name: string, o: Partial<CardDef> = {}): CardDef {
  return { key: 'p:' + name, name, layout: 'normal', manaCost: '', cmc: 0, typeLine: 'Creature', oracleText: '', colors: [], colorIdentity: [], img: [], legalities: {}, ...o };
}

describe('mana helpers', () => {
  it('computes mana value and pips', () => {
    expect(manaValue('{2}{W}{U}')).toBe(4);
    expect(manaValue('{X}{R}')).toBe(1);
    expect(pipCounts('{W/U}{G}')).toEqual({ W: 1, U: 1, B: 0, R: 0, G: 1 });
    expect(normName("Praetors’ Voice")).toBe("praetors' voice");
  });
});

describe('import parsing', () => {
  it('parses Moxfield text with sections, set codes and foil', () => {
    const p = parseDeckText('Commander\n1 Atraxa, Praetors\' Voice (C16) 28\n\nMain\n1x Sol Ring (C21) 263\n1 Arcane Signet (CMM) 364 *F*\n10 Forest\nSideboard\n2 Negate [Counter]\nTokens\n1 Treasure');
    expect(p.format).toBe('plain');
    const [cmd, sol, sig, forest, neg, tr] = p.lines;
    expect(cmd).toMatchObject({ section: 'commander', qty: 1, name: "Atraxa, Praetors' Voice", set: 'c16', number: '28' });
    expect(sol).toMatchObject({ section: 'main', name: 'Sol Ring', set: 'c21' });
    expect(sig.foil).toBe(true);
    expect(forest).toMatchObject({ qty: 10, name: 'Forest' });
    expect(neg).toMatchObject({ section: 'side', qty: 2, tags: ['counter'] });
    expect(tr.section).toBe('tokens');
  });
  it('parses Arena exports', () => {
    const t = 'Commander\n1 Krenko, Mob Boss (DDT) 52\n\nDeck\n4 Lightning Bolt (2XM) 141\n\nSideboard\n1 Negate (M20) 69';
    expect(detectFormat(t)).toBe('arena');
    const p = parseDeckText(t);
    expect(p.lines.map((l) => l.section)).toEqual(['commander', 'main', 'side']);
  });
  it('parses MTGGoldfish style (blank line = sideboard)', () => {
    const p = parseDeckText('4 Lightning Bolt\n20 Mountain\n\n2 Smash to Smithereens');
    expect(p.lines[2].section).toBe('side');
  });
  it('a blank line ends a commander block', () => {
    const p = parseDeckText("Commander\n1 Atraxa, Praetors' Voice\n\n1 Sol Ring\n30 Island\n\nTokens\n1 Treasure");
    expect(p.lines.map((l) => l.section)).toEqual(['commander', 'main', 'main', 'tokens']);
  });
  it('parses MTGO .dek XML', () => {
    const p = parseDeckText('<?xml version="1.0"?>\n<Deck>\n  <Cards CatID="1" Quantity="4" Sideboard="false" Name="Lightning Bolt" />\n  <Cards Quantity="2" Name="Lightning Helix" Sideboard="true" />\n</Deck>');
    expect(p.format).toBe('mtgo');
    expect(p.lines).toHaveLength(2);
    expect(p.lines[1]).toMatchObject({ qty: 2, name: 'Lightning Helix', section: 'side' });
  });
  it('flags malformed lines', () => {
    expect(parseDeckText('what is this').lines[0].wellFormed).toBe(false);
  });
  it('exports round-trip', () => {
    const defs = { 'p:a': def('Sol Ring', { key: 'p:a', set: 'c21', collectorNumber: '263' }) };
    const cards: DeckCard[] = [{ id: 1, section: 'main', def: 'p:a', qty: 1, foil: true, tags: [], addedAt: 0 }];
    const txt = exportDeck(cards, defs, 'moxfield');
    expect(txt).toContain('1 Sol Ring (C21) 263 *F*');
    expect(parseDeckText(txt).lines[0]).toMatchObject({ name: 'Sol Ring', foil: true });
    expect(exportDeck(cards, defs, 'mtgo')).toContain('Name="Sol Ring"');
    expect(exportDeck(cards, defs, 'arena')).toContain('Deck\n1 Sol Ring (C21) 263');
  });
  it('parses Moxfield #tags and exports them round-trip', () => {
    const p = parseDeckText('1 Sol Ring (C21) 263 *F* #Ramp #!Mana  Rock\n1 Arcane Signet [Ramp, Fixing{top}] #ramp');
    expect(p.lines[0]).toMatchObject({ name: 'Sol Ring', set: 'c21', number: '263', foil: true, tags: ['ramp'], globalTags: ['mana rock'] });
    expect(p.lines[1]).toMatchObject({ name: 'Arcane Signet', tags: ['ramp', 'fixing'] });
    const defs = { 'p:a': def('Sol Ring', { key: 'p:a', set: 'c21', collectorNumber: '263' }) };
    const cards: DeckCard[] = [{ id: 1, section: 'main', def: 'p:a', qty: 1, foil: false, tags: ['ramp', 'mana rock'], addedAt: 0 }];
    expect(exportDeck(cards, defs, 'moxfield')).toContain('1 Sol Ring (C21) 263\n');
    const txt = exportDeck(cards, defs, 'moxfield', { tags: true, globalTags: new Set(['mana rock']) });
    expect(txt).toContain('1 Sol Ring (C21) 263 #ramp #!mana rock');
    expect(parseDeckText(txt).lines[0]).toMatchObject({ name: 'Sol Ring', number: '263', tags: ['ramp'], globalTags: ['mana rock'] });
    expect(exportDeck(cards, defs, 'arena', { tags: true })).not.toContain('#');
  });
  it('normalizes tags', () => {
    expect(normalizeTag('  #Mana   Rock ')).toBe('mana rock');
    expect(normalizeTag('#!Staple')).toBe('staple');
    expect(normalizeTag('x'.repeat(40))).toHaveLength(30);
  });
  it('levenshtein', () => {
    expect(levenshtein('lightnig bolt', 'lightning bolt')).toBe(1);
    expect(levenshtein('abc', 'xyz', 1)).toBeGreaterThan(1);
  });
});

describe('search', () => {
  const bolt = def('Lightning Bolt', { typeLine: 'Instant', colors: ['R'], colorIdentity: ['R'], cmc: 1, oracleText: 'deals 3 damage', rarity: 'common', set: 'lea', legalities: { modern: 'legal' } });
  const atraxa = def("Atraxa, Praetors' Voice", { typeLine: 'Legendary Creature — Angel', colors: ['W', 'U', 'B', 'G'], colorIdentity: ['W', 'U', 'B', 'G'], cmc: 4, power: '4', toughness: '4', rarity: 'mythic' });
  const q = (s: string, d: CardDef) => matchQuery(d, parseSearch(s));
  it('matches fields', () => {
    expect(q('t:instant c:r', bolt)).toBe(true);
    expect(q('mv<=1 o:damage', bolt)).toBe(true);
    expect(q('-t:instant', bolt)).toBe(false);
    expect(q('id:wubg is:commander', atraxa)).toBe(true);
    expect(q('id:wu', atraxa)).toBe(false);
    expect(q('c:m pow>=4', atraxa)).toBe(true);
    expect(q('r:m', atraxa)).toBe(true);
    expect(q('f:modern', bolt)).toBe(true);
    expect(q('f:standard', bolt)).toBe(false);
    expect(q('s:lea bolt', bolt)).toBe(true);
    expect(q('"praetors\' voice"', atraxa)).toBe(true);
    expect(q('t:goblin or t:instant', bolt)).toBe(true);
  });
});

describe('legality', () => {
  const card = (id: number, name: string, section: DeckCard['section'], qty = 1): DeckCard => ({ id, section, def: 'p:' + name, qty, foil: false, tags: [], addedAt: 0 });
  const defs: Record<string, CardDef> = {
    'p:Atraxa': def('Atraxa', { key: 'p:Atraxa', colorIdentity: ['W', 'U', 'B', 'G'], legalities: { commander: 'legal', vintage: 'legal', modern: 'legal' } }),
    'p:Crypt': def('Crypt', { key: 'p:Crypt', legalities: { commander: 'banned', vintage: 'restricted', legacy: 'banned', modern: 'not_legal' } }),
    'p:Bolt': def('Bolt', { key: 'p:Bolt', colorIdentity: ['R'], legalities: { commander: 'legal', modern: 'legal', vintage: 'legal' } }),
    'p:Forest': def('Forest', { key: 'p:Forest', typeLine: 'Basic Land — Forest', legalities: { commander: 'legal', modern: 'legal' } }),
    'p:Rats': def('Rats', { key: 'p:Rats', oracleText: 'A deck can have any number of cards named Rats.', legalities: { commander: 'legal', modern: 'legal' } }),
  };
  it('commander rules', () => {
    const r = checkLegality('commander', [card(1, 'Atraxa', 'commander'), card(2, 'Crypt', 'main'), card(3, 'Bolt', 'main', 2), card(4, 'Forest', 'main', 30), card(5, 'Rats', 'main', 20)], defs);
    expect(r.cards[2]).toContain('Banned in Commander');
    expect(r.cards[3]).toContain("Outside your commander's color identity (Red)");
    expect(r.cards[3]).toContain('2 copies — singleton format allows 1');
    expect(r.cards[4]).toBeUndefined();
    expect(r.cards[5]).toBeUndefined();
    expect(r.deckWarnings).toContain('54 of 100 cards');
  });
  it('constructed rules', () => {
    const r = checkLegality('modern', [card(1, 'Atraxa', 'commander'), card(2, 'Bolt', 'main', 5), card(3, 'Crypt', 'main')], defs);
    expect(r.cards[1]).toContain('No command zone in Modern — counts as a main deck card');
    expect(r.cards[2]).toContain('More than 4 copies');
    expect(r.cards[3]).toContain('Not legal in Modern');
    const v = checkLegality('vintage', [card(3, 'Crypt', 'main', 2)], defs);
    expect(v.cards[3]).toContain('Restricted in Vintage — 1 copy allowed');
  });
  it('freeform and cube never warn; maybe/tokens never flagged', () => {
    expect(checkLegality('cube', [card(2, 'Crypt', 'main', 9)], defs).warnings).toEqual([]);
    expect(checkLegality('freeform', [card(2, 'Crypt', 'main', 9)], defs).warnings).toEqual([]);
    expect(checkLegality('commander', [card(1, 'Atraxa', 'commander'), card(2, 'Crypt', 'maybe')], defs).cards[2]).toBeUndefined();
  });
  it('stats and grouping', () => {
    const d2 = { ...defs, 'p:Bolt': { ...defs['p:Bolt'], cmc: 1, manaCost: '{R}', typeLine: 'Instant' } };
    const cards = [card(1, 'Atraxa', 'commander'), card(3, 'Bolt', 'main', 3), card(4, 'Forest', 'main', 5)];
    const s = deckStats('commander', cards, d2);
    expect(s.total).toBe(9);
    expect(s.lands).toBe(5);
    expect(s.pips.R).toBe(3);
    const g = groupCards(cards, d2, 'type', 'name');
    expect(g.map((x) => x.title)).toEqual(['Commander', 'Instants', 'Lands']);
  });
  it('tag grouping puts a card under each of its tags, in the deck tag order', () => {
    const tagged = [card(1, 'Atraxa', 'commander'), { ...card(3, 'Bolt', 'main', 3), tags: ['removal', 'burn'] }, { ...card(2, 'Rats', 'main'), tags: ['burn'] }, card(4, 'Forest', 'main', 5)];
    // a global tag no card here uses is left out
    const order = deckTagNames([{ name: 'removal', color: null, scope: 'global' }, { name: 'unused', color: null, scope: 'global' }], tagged);
    expect(order).toEqual(['removal', 'burn']);
    const g = groupCards(tagged, defs, 'tag', 'name', order);
    expect(g.map((x) => [x.title, x.count])).toEqual([
      ['Commander', 1],
      ['#removal', 3],
      ['#burn', 4],
      ['Untagged', 5],
    ]);
  });
});

describe('diff', () => {
  it('round-trips', () => {
    const a = { x: 1, y: { z: [1, 2], w: 'a' }, gone: true };
    const b = { x: 2, y: { z: [1, 2, 3], w: 'a' }, added: { q: 1 } };
    const out = applyPatch(a, diff(a, b));
    expect(out).toEqual(b);
    expect(a.x).toBe(1);
  });
});

describe('theme and layout', () => {
  it('accent vars', () => {
    expect(accentVars('blue')['--acc']).toBe('#4a95e0');
    expect(accentVars('black')['--acc-ink']).toBe('#ffffff');
  });
  it('grid puts the viewer bottom-left and runs clockwise', () => {
    const s = gridSlots(5, 2, 0, 0, 1000, 800);
    const bottom = s.filter((x) => !x.top);
    const top = s.filter((x) => x.top);
    expect(bottom[0].seat).toBe(2);
    expect(top.map((x) => x.seat)).toEqual([3, 4, 0]);
    expect(bottom.map((x) => x.seat)).toEqual([2, 1]);
  });
  it('hand layout wraps then overlaps', () => {
    const R = { x: 0, y: 0, w: 600, h: 200 };
    expect(handLayout(3, R).every((r) => r.y === 8)).toBe(true);
    const many = handLayout(40, R);
    expect(many).toHaveLength(40);
    expect(Math.max(...many.map((r) => r.x + r.w))).toBeLessThanOrEqual(600);
  });
  it('hand shrinks a little per card before wrapping to another row', () => {
    const R = { x: 0, y: 0, w: 600, h: 200 };
    const rows = (n: number) => new Set(handLayout(n, R).map((r) => r.y)).size;
    const h = (n: number) => handLayout(n, R)[0].h;
    const h2 = (184 - 6) / 2;
    const h3 = (184 - 12) / 3;
    let prevRows = 1;
    let prevH = h(1);
    for (let n = 2; n <= 60; n++) {
      const r = rows(n);
      expect(r).toBeGreaterThanOrEqual(prevRows);
      expect(h(n)).toBeLessThanOrEqual(prevH + 1e-9);
      // Still one row while the cards are bigger than two-row size; still two rows while bigger than three-row size.
      if (h(n) > h2 + 1e-6) expect(r).toBe(1);
      else if (h(n) > h3 + 1e-6) expect(r).toBeLessThanOrEqual(2);
      expect(h(n)).toBeGreaterThanOrEqual(h3 - 1e-6);
      prevRows = r;
      prevH = h(n);
    }
    // 5 cards fit a single row at full height; 6 shrink slightly but stay on one row.
    expect(rows(6)).toBe(1);
    expect(h(6)).toBeLessThan(h(5));
    expect(h(6)).toBeGreaterThan(h2);
  });
});

describe('canBeCommander', () => {
  const card = (typeLine: string, extra: Partial<CardDef> = {}): CardDef => ({ key: 'x', name: 'X', layout: 'normal', manaCost: '', cmc: 0, typeLine, oracleText: '', colors: [], colorIdentity: [], img: [], ...extra });
  it('accepts legendary creatures and cards that say so', () => {
    expect(canBeCommander(card('Legendary Creature — Human Artificer'))).toBe(true);
    expect(canBeCommander(card('Legendary Planeswalker — Teferi', { oracleText: 'Teferi can be your commander.' }))).toBe(true);
  });
  it('accepts legendary Vehicles and Spacecraft with power/toughness', () => {
    expect(canBeCommander(card('Legendary Artifact — Spacecraft', { power: '7', toughness: '15' }))).toBe(true);
    expect(canBeCommander(card('Legendary Artifact — Vehicle', { power: '4', toughness: '4' }))).toBe(true);
    expect(canBeCommander(card('Legendary Artifact — Spacecraft'))).toBe(false);
  });
  it('accepts Backgrounds, and planeswalkers only in Brawl', () => {
    expect(canBeCommander(card('Legendary Enchantment — Background'))).toBe(true);
    expect(canBeCommander(card('Legendary Planeswalker — Jace'))).toBe(false);
    expect(canBeCommander(card('Legendary Planeswalker — Jace'), 'brawl')).toBe(true);
  });
  it('uses the Oathbreaker rule in Oathbreaker', () => {
    expect(canBeCommander(card('Planeswalker — Jace'), 'oathbreaker')).toBe(true);
    expect(canBeCommander(card('Instant'), 'oathbreaker')).toBe(true);
    expect(canBeCommander(card('Legendary Creature — Elf'), 'oathbreaker')).toBe(false);
  });
  it('rejects ordinary cards', () => {
    expect(canBeCommander(card('Artifact'))).toBe(false);
    expect(canBeCommander(card('Legendary Enchantment'))).toBe(false);
  });
});

describe('battlefield positions are measured from the hand', () => {
  const area = { bf: { x: 0, y: 0, w: 800, h: 400 }, card: { w: 60, h: 84 } };
  it('y = 1 is next to the hand on both a bottom and a top seat', () => {
    const low = bfPos({ ...area, top: false }, { x: 0.5, y: 1, tapped: false });
    const high = bfPos({ ...area, top: true }, { x: 0.5, y: 1, tapped: false });
    expect(low.y).toBeGreaterThan(200); // bottom seat: hand below the battlefield
    expect(high.y).toBeLessThan(100); // top seat: hand above it
  });
  it('round-trips through bfFrac', () => {
    for (const top of [false, true]) {
      const p = bfPos({ ...area, top }, { x: 0.3, y: 0.8, tapped: false });
      const f = bfFrac({ ...area, top }, p.x, p.y, false);
      expect(f.x).toBeCloseTo(0.3, 3);
      expect(f.y).toBeCloseTo(0.8, 3);
    }
  });
});
