import { describe, expect, it } from 'vitest';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { nativePath, RulesEngine, type Choice, type Decision, type GameSetup } from './index';

const cards = join(fileURLToPath(import.meta.url), '../../../../cards');
const native = nativePath();
// CI sets REQUIRE_NATIVE so a missing addon fails the run instead of silently skipping every engine test.
if (!native && process.env.REQUIRE_NATIVE) throw new Error('REQUIRE_NATIVE is set but the rules engine addon was not found (set RULES_NATIVE)');

const deck = (colors: 'GR' | 'WB'): [string, number][] =>
  colors === 'GR'
    ? [['Forest', 8], ['Mountain', 8], ['Grizzly Bears', 4], ['Hill Giant', 4], ['Llanowar Elves', 4], ['Lightning Bolt', 4], ['Giant Growth', 4], ['Raging Goblin', 4]]
    : [['Plains', 8], ['Swamp', 8], ['Elite Vanguard', 4], ['Serra Angel', 3], ['Typhoid Rats', 4], ['Vampire Nighthawk', 3], ['Terror', 4], ['Swords to Plowshares', 3], ['Glorious Anthem', 3]];

function setup(): GameSetup {
  return {
    seed: 7,
    commander: false,
    mulligan: 'London',
    planar: [],
    combat_stops: true,
    seats: [
      { name: 'Human', human: true, deck: deck('GR'), commander: [], sideboard: [], companion: null, schemes: [], difficulty: 'Normal' },
      // Easy AIs: these tests check the game and session plumbing, not how well the AI plays (Normal / Hard think 250 / 600 ms
      // per decision, which makes a whole game take 20+ seconds)
      { name: 'Bot 1', human: false, deck: deck('WB'), commander: [], sideboard: [], companion: null, schemes: [], difficulty: 'Easy' },
      { name: 'Bot 2', human: false, deck: deck('GR'), commander: [], sideboard: [], companion: null, schemes: [], difficulty: 'Easy' },
      { name: 'Bot 3', human: false, deck: deck('WB'), commander: [], sideboard: [], companion: null, schemes: [], difficulty: 'Easy' },
    ],
  };
}

/** The simplest legal answer for any question: keep, pass, no blocks. */
function simple(d: Decision): Choice {
  const k = d.kind;
  switch (k.t) {
    case 'Mulligan': return { t: 'Keep', keep: true };
    case 'Bottom': return { t: 'Bottom', cards: k.hand.slice(0, k.n) };
    case 'Priority': {
      const land = k.actions.findIndex((a) => a.t === 'PlayLand');
      return { t: 'Priority', action: land >= 0 ? land : null };
    }
    case 'Pick': return { t: 'Pick', indices: Array.from({ length: k.min }, (_, i) => i) };
    case 'Number': return { t: 'Number', n: k.min };
    case 'YesNo': return { t: 'YesNo', yes: false };
    case 'PayMana': return { t: 'Pay', answer: { t: 'Auto' } };
    case 'Attackers': return { t: 'Attackers', attacks: [] };
    case 'Blockers': return { t: 'Blockers', blocks: [] };
    case 'AssignDamage': return { t: 'Damage', assign: [] };
    case 'Order': return { t: 'Order', order: k.items.map((_, i) => i) };
  }
}

describe.skipIf(!native)('native rules engine', () => {
  it('loads the card pool', () => {
    const e = RulesEngine.load(cards);
    expect(e.cardCount).toBeGreaterThan(40);
    expect(e.hasCard('Lightning Bolt')).toBe(true);
    expect(e.hasCard('No Such Card')).toBe(false);
  });

  it('rejects unknown cards with a list', () => {
    const e = RulesEngine.load(cards);
    const s = setup();
    s.seats[0].deck.push(['Definitely Not A Card', 1]);
    expect(() => e.newGame(s)).toThrow(/Definitely Not A Card/);
  });

  it('plays 1 human + 3 AI to completion without blocking the event loop', async () => {
    const e = RulesEngine.load(cards);
    const g = e.newGame(setup());
    let ticks = 0;
    const timer = setInterval(() => ticks++, 5);
    let frames = 0;
    let over = false;
    for (let i = 0; i < 3000 && !over; i++) {
      const adv = await g.advance();
      frames += adv.frames.length;
      over = adv.over !== null;
      if (adv.waiting) g.answer(adv.waiting.player, simple(adv.waiting));
    }
    clearInterval(timer);
    expect(over).toBe(true);
    expect(frames).toBeGreaterThan(50);
    expect(ticks).toBeGreaterThan(0);
  }, 60_000);

  it('saves and restores a game (seed + decisions)', async () => {
    const e = RulesEngine.load(cards);
    const g = e.newGame(setup());
    for (let i = 0; i < 40; i++) {
      const adv = await g.advance();
      if (adv.over || !adv.waiting) break;
      g.answer(adv.waiting.player, simple(adv.waiting));
    }
    const rec = g.record();
    const g2 = e.restoreGame(rec);
    // both continue from the same saved point and reach the same question
    await g.advance();
    await g2.advance();
    const a = g.view(0);
    const b = g2.view(0);
    expect(b.turn).toBe(a.turn);
    expect(b.players.map((p) => p.life)).toEqual(a.players.map((p) => p.life));
    expect(b.cards.length).toBe(a.cards.length);
  }, 60_000);

  it('refuses answers with ids that name nothing instead of crashing the process', async () => {
    const e = RulesEngine.load(cards);
    const g = e.newGame(setup());
    g.setManualPay(0, true);
    let sawPay = false;
    let sawBlock = false;
    for (let i = 0; i < 3000 && !(sawPay && sawBlock); i++) {
      const adv = await g.advance();
      if (adv.over || !adv.waiting) break;
      const d = adv.waiting;
      const k = d.kind;
      if (d.player === 0 && k.t === 'PayMana' && !sawPay) {
        sawPay = true;
        expect(() => g.answer(0, { t: 'Pay', answer: { t: 'Tap', card: 999_999, ability: 0, produce: null } })).toThrow();
        // the question is still open: a legal answer goes on as before
        g.answer(0, { t: 'Pay', answer: { t: 'Auto' } });
        continue;
      }
      if (d.player === 0 && k.t === 'Blockers' && !sawBlock) {
        sawBlock = true;
        expect(() => g.answer(0, { t: 'Blockers', blocks: [[999_999, 0]] })).toThrow();
        g.answer(0, { t: 'Blockers', blocks: [] });
        continue;
      }
      if (d.player === 0 && k.t === 'Priority' && !k.actions.some((a) => a.t === 'PlayLand')) {
        // cast what we can, so payments (and creatures to block with) come up
        const cast = k.actions.findIndex((a) => a.t === 'Cast');
        if (cast >= 0) {
          g.answer(0, { t: 'Priority', action: cast });
          continue;
        }
      }
      g.answer(d.player, simple(d));
    }
    expect(sawPay).toBe(true);
    // the game still works after the refused answers
    await expect(g.advance()).resolves.toBeDefined();
    expect(g.view(0).turn).toBeGreaterThan(0);
  }, 60_000);

  it('hides opponents\' hands', async () => {
    const e = RulesEngine.load(cards);
    const g = e.newGame(setup());
    const adv = await g.advance();
    const v = adv.frames.at(-1)!.views[0]!;
    expect(v.cards.filter((c) => c.zone === 'Hand' && c.owner !== 0)).toHaveLength(0);
    expect(v.cards.filter((c) => c.zone === 'Library')).toHaveLength(0);
    expect(v.players[1].hand).toBeGreaterThan(0);
  });
});
