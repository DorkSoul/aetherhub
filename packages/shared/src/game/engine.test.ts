import { describe, expect, it } from 'vitest';
import type { CardDef } from '../cards';
import { applyIntent, IntentError, locate, resolveIntent, type ApplyCtx } from './engine';
import type { Intent } from './intents';
import { createGame, DEFAULT_SETTINGS, type SeatInput } from './setup';
import type { GameState, Rng } from './types';
import { logFor, viewFor } from './view';

function def(key: string, name: string, o: Partial<CardDef> = {}): CardDef {
  return { key, name, layout: 'normal', manaCost: '', cmc: 0, typeLine: 'Creature — Test', oracleText: '', colors: [], colorIdentity: [], img: [], power: '2', toughness: '2', ...o };
}

const DEFS: Record<string, CardDef> = {
  'p:bear': def('p:bear', 'Grizzly Bears'),
  'p:bolt': def('p:bolt', 'Lightning Bolt', { typeLine: 'Instant', power: undefined, toughness: undefined }),
  'p:cmd': def('p:cmd', 'Atraxa', { typeLine: 'Legendary Creature — Angel' }),
  'p:tok': def('p:tok', 'Goblin', { typeLine: 'Token Creature — Goblin', isToken: true }),
  'p:dfc': def('p:dfc', 'Delver of Secrets', { layout: 'transform', img: ['p/x/0', 'p/x/1'], faces: [
    { name: 'Delver of Secrets', manaCost: '{U}', typeLine: 'Creature', oracleText: '', power: '1', toughness: '1' },
    { name: 'Insectile Aberration', manaCost: '', typeLine: 'Creature', oracleText: 'Flying', power: '3', toughness: '2' },
  ] }),
};

// Deterministic RNG for tests.
function seqRng(seed = 1): Rng {
  let s = seed;
  return { int: (n) => ((s = (s * 1103515245 + 12345) % 2147483648), s % n) };
}

function seatIn(i: number, cards: SeatInput['cards']): SeatInput {
  return { userId: i + 1, name: ['Alice', 'Bob', 'Cass'][i], color: '#fff', deckId: i + 1, deckName: 'Deck', sleeve: null, playmat: null, autoUntap: true, autoDraw: true, cards };
}

function game(players = 2, settings: Partial<typeof DEFAULT_SETTINGS> = {}): GameState {
  const cards: SeatInput['cards'] = [
    { def: 'p:bear', section: 'main', qty: 20 },
    { def: 'p:bolt', section: 'main', qty: 10 },
    { def: 'p:cmd', section: 'commander', qty: 1 },
    { def: 'p:bear', section: 'side', qty: 2 },
    { def: 'p:tok', section: 'tokens', qty: 1 },
  ];
  return createGame({ id: 'g', settings: { ...DEFAULT_SETTINGS, ...settings }, seats: Array.from({ length: players }, (_, i) => seatIn(i, cards)), defs: DEFS, rng: seqRng(), now: 0 });
}

function act(state: GameState, actor: number, intent: Intent) {
  const ctx: ApplyCtx = { actor, actorName: state.seats[actor]?.name ?? 'spec', now: 1 };
  const resolved = resolveIntent(state, ctx, intent, seqRng(7));
  if (intent.type === 'createToken' && intent.def) resolved.defs = { [intent.def]: DEFS[intent.def] };
  return applyIntent(state, ctx, intent, resolved);
}

describe('setup', () => {
  it('shuffles, deals 7 and places commanders', () => {
    const g = game();
    const s = g.seats[0];
    expect(s.zones.hand).toHaveLength(7);
    expect(s.zones.library).toHaveLength(23);
    expect(s.zones.command).toHaveLength(1);
    expect(g.cards[s.zones.command[0]].commander).toBe(true);
    expect(s.zones.sideboard).toHaveLength(2);
    expect(s.tokenDefs).toEqual(['p:tok']);
    expect(s.life).toBe(40);
    expect(g.stage).toBe('play');
  });
  it("card ids give away neither the deck list's order nor the library order", () => {
    // 30 bears listed before 30 bolts: ids handed out in list order would put all bears on c1..c30.
    const cards: SeatInput['cards'] = [
      { def: 'p:bear', section: 'main', qty: 30 },
      { def: 'p:bolt', section: 'main', qty: 30 },
    ];
    const g = createGame({ id: 'g', settings: DEFAULT_SETTINGS, seats: [seatIn(0, cards), seatIn(1, cards)], defs: DEFS, rng: seqRng(5), now: 0 });
    for (const s of g.seats) {
      const all = [...s.zones.hand, ...s.zones.library];
      const byId = all.slice().sort((a, b) => Number(a.slice(1)) - Number(b.slice(1)));
      const firstHalf = byId.slice(0, 30).map((i) => g.cards[i].def);
      expect(firstHalf.filter((d) => d === 'p:bear').length).toBeLessThan(25);
      expect(firstHalf.filter((d) => d === 'p:bear').length).toBeGreaterThan(5);
      // and sorting by id doesn't give the draw order either
      expect(byId).not.toEqual(all);
    }
  });
  it('enters mulligan stage when a rule is chosen', () => {
    expect(game(2, { mulligan: 'london' }).stage).toBe('mulligan');
  });
});

describe('moves and logging', () => {
  it('plays a card from hand to the battlefield and names it', () => {
    const g = game();
    const iid = g.seats[0].zones.hand[0];
    const r = act(g, 0, { type: 'move', iids: [iid], seat: 0, zone: 'battlefield', x: 0.2, y: 0.3 });
    expect(g.seats[0].zones.battlefield).toContain(iid);
    expect(g.cards[iid].x).toBe(0.2);
    expect(r.logs[0].text).toMatch(/^played (Grizzly Bears|Lightning Bolt)$/);
    expect(r.logs[0].redacted).toBeUndefined();
  });

  it('draw is private: the drawer sees names, others see a count', () => {
    const g = game();
    const r = act(g, 0, { type: 'draw', seat: 0, n: 2 });
    expect(g.seats[0].zones.hand).toHaveLength(9);
    const e = { ...r.logs[0], id: 1, at: 1, actor: 0, actorName: 'Alice' };
    expect(logFor(e, 0, true)!.text).toMatch(/drew .+ and .+/);
    expect(logFor(e, 0, true)!.priv).toBe(true);
    expect(logFor(e, 1, true)!.text).toBe('drew 2 cards');
    expect(logFor(e, 'spectator', true)!.text).toMatch(/drew .+ and .+/);
  });

  it("drawing from an opponent's library goes to the clicker's hand", () => {
    const g = game();
    const top = g.seats[1].zones.library[0];
    act(g, 0, { type: 'draw', seat: 1, n: 1 });
    expect(g.seats[0].zones.hand).toContain(top);
    expect(g.cards[top].owner).toBe(1);
  });

  it('menu moves go to the owner zone; leaving the battlefield resets state', () => {
    const g = game();
    const iid = g.seats[0].zones.hand[0];
    act(g, 0, { type: 'move', iids: [iid], seat: 1, zone: 'battlefield' });
    act(g, 1, { type: 'tap', iids: [iid] });
    act(g, 1, { type: 'counter', iids: [iid], counter: '+1/+1', delta: 2 });
    expect(g.cards[iid].tapped).toBe(true);
    act(g, 1, { type: 'move', iids: [iid], seat: g.cards[iid].owner, zone: 'graveyard' });
    expect(g.seats[0].zones.graveyard).toContain(iid);
    expect(g.cards[iid].tapped).toBe(false);
    expect(g.cards[iid].counters).toEqual({});
  });

  it('library positions: top, bottom and Nth', () => {
    const g = game();
    const [a, b, c] = g.seats[0].zones.hand;
    act(g, 0, { type: 'move', iids: [a], seat: 0, zone: 'library', pos: 'top' });
    act(g, 0, { type: 'move', iids: [b], seat: 0, zone: 'library', pos: 'bottom' });
    act(g, 0, { type: 'move', iids: [c], seat: 0, zone: 'library', pos: 3 });
    const lib = g.seats[0].zones.library;
    expect(lib[0]).toBe(a);
    expect(lib[lib.length - 1]).toBe(b);
    expect(lib[2]).toBe(c);
  });

  it('hands: cards go where they are dropped, a draw goes at the end, and reordering is not logged', () => {
    const g = game();
    const hand = g.seats[0].zones.hand;
    const [a, b, c, d] = hand;
    const r = act(g, 0, { type: 'move', iids: [d, c], seat: 0, zone: 'hand', pos: 1 });
    expect(hand.slice(0, 4)).toEqual([d, c, a, b]);
    expect(r.logs).toEqual([]);
    act(g, 0, { type: 'draw', seat: 0, n: 1 });
    const drawn = hand[hand.length - 1];
    expect([a, b, c, d]).not.toContain(drawn);
    const bf = g.seats[0].zones.battlefield;
    act(g, 0, { type: 'move', iids: [a], seat: 0, zone: 'battlefield' });
    expect(bf).toContain(a);
    act(g, 0, { type: 'move', iids: [a], seat: 0, zone: 'hand', pos: 2 });
    expect(hand.slice(0, 3)).toEqual([d, a, c]);
  });
});

describe('face-down cards in the log', () => {
  it('names a face-down card only to its peekers', () => {
    const g = game();
    const iid = g.seats[0].zones.hand[0];
    act(g, 0, { type: 'move', iids: [iid], seat: 0, zone: 'battlefield', faceDown: true });
    const name = g.defs[g.cards[iid].def].name;
    const entry = (r: ReturnType<typeof act>) => ({ ...r.logs[0], id: 1, at: 1, actor: 0, actorName: 'Alice' });
    for (const intent of [
      { type: 'ptMod', iid, p: 1, t: 1, eot: true },
      { type: 'effect', iid, k: 'Flying' },
      { type: 'noUntap', iid, value: true },
      { type: 'copy', iid },
    ] as Intent[]) {
      const e = entry(act(g, 0, intent));
      expect(logFor(e, 0, true)!.text).toContain(name);
      expect(logFor(e, 1, true)!.text).not.toContain(name);
      expect(logFor(e, 1, true)!.text).toContain('a face-down card');
    }
    // the token copy of a face-down card is face-down as well
    const copy = g.seats[0].zones.battlefield.find((i) => g.cards[i].token)!;
    expect(g.cards[copy].faceDown).toBe(true);
  });
});

describe('commanders', () => {
  it('adds tax when leaving the command zone and prompts on death', () => {
    const g = game();
    const cmd = g.seats[0].zones.command[0];
    act(g, 0, { type: 'move', iids: [cmd], seat: 0, zone: 'battlefield' });
    expect(g.seats[0].cmdTax[cmd]).toBe(2);
    const r = act(g, 0, { type: 'move', iids: [cmd], seat: 0, zone: 'graveyard' });
    expect(r.notices).toContainEqual({ to: 0, kind: 'commanderPrompt', iid: cmd });
  });
});

describe('shuffle and randomness', () => {
  it('re-keys library cards on shuffle', () => {
    const g = game();
    const before = g.seats[0].zones.library.slice();
    act(g, 0, { type: 'shuffle', seat: 0 });
    const after = g.seats[0].zones.library;
    expect(after).toHaveLength(before.length);
    expect(after.some((i) => before.includes(i))).toBe(false);
  });

  it('replaying with the same resolved data gives an identical state', () => {
    const a = game();
    const b = JSON.parse(JSON.stringify(a)) as GameState;
    const ctx: ApplyCtx = { actor: 0, actorName: 'Alice', now: 1 };
    const intent: Intent = { type: 'shuffle', seat: 0 };
    const res = resolveIntent(a, ctx, intent, seqRng(99));
    applyIntent(a, ctx, intent, res);
    applyIntent(b, ctx, intent, res);
    expect(b).toEqual(a);
  });
});

describe('turns', () => {
  it('passes the turn: untaps (except doesn\'t-untap), draws, expires EOT effects, skips conceded', () => {
    const g = game(3);
    const [x, y] = g.seats[1].zones.hand;
    act(g, 1, { type: 'move', iids: [x, y], seat: 1, zone: 'battlefield' });
    act(g, 1, { type: 'tap', iids: [x, y] });
    act(g, 1, { type: 'noUntap', iid: y, value: true });
    act(g, 1, { type: 'effect', iid: x, k: 'Haste', eot: true });
    act(g, 1, { type: 'ptMod', iid: x, p: 2, t: 2, eot: true });
    const hand = g.seats[1].zones.hand.length;
    act(g, 0, { type: 'passTurn' });
    expect(g.turn.active).toBe(1);
    expect(g.cards[x].tapped).toBe(false);
    expect(g.cards[y].tapped).toBe(true);
    expect(g.cards[x].effects).toEqual([]);
    expect(g.cards[x].ptEot).toEqual({ p: 0, t: 0 });
    expect(g.seats[1].zones.hand.length).toBe(hand + 1);
    act(g, 2, { type: 'concede' });
    act(g, 1, { type: 'passTurn' });
    expect(g.turn.active).toBe(0);
  });
  it('auto draw can be turned off: untap still happens and the turn starts in the draw step', () => {
    const g = game(2);
    act(g, 1, { type: 'autoDraw', value: false });
    const [x] = g.seats[1].zones.hand;
    act(g, 1, { type: 'move', iids: [x], seat: 1, zone: 'battlefield' });
    act(g, 1, { type: 'tap', iids: [x] });
    const hand = g.seats[1].zones.hand.length;
    act(g, 0, { type: 'passTurn' });
    expect(g.cards[x].tapped).toBe(false);
    expect(g.seats[1].zones.hand.length).toBe(hand);
    expect(g.turn.phase).toBe(2);
    // Both off: nothing happens automatically.
    act(g, 0, { type: 'autoUntap', value: false });
    act(g, 0, { type: 'autoDraw', value: false });
    act(g, 1, { type: 'passTurn' });
    expect(g.turn.phase).toBe(0);
  });
});

describe('piles and tokens', () => {
  it('creates, stacks, splits and deletes tokens', () => {
    const g = game();
    act(g, 0, { type: 'createToken', seat: 0, def: 'p:tok', count: 3 });
    const toks = g.seats[0].zones.battlefield.filter((i) => g.cards[i].token);
    expect(toks).toHaveLength(3);
    act(g, 0, { type: 'stack', iids: toks });
    const pile = g.seats[0].zones.battlefield.find((i) => g.cards[i].token)!;
    expect(g.cards[pile].pile).toBe(3);
    act(g, 0, { type: 'split', iid: pile, n: 2, tap: true });
    const tapped = g.seats[0].zones.battlefield.find((i) => g.cards[i].token && g.cards[i].tapped)!;
    expect(g.cards[tapped].pile).toBe(2);
    expect(g.cards[pile].pile).toBe(1);
    act(g, 0, { type: 'delete', iids: [pile, tapped] });
    expect(g.seats[0].zones.battlefield.filter((i) => g.cards[i].token)).toHaveLength(0);
  });
  it('refuses to delete non-tokens', () => {
    const g = game();
    const iid = g.seats[0].zones.hand[0];
    expect(() => act(g, 0, { type: 'delete', iids: [iid] })).toThrow(IntentError);
  });
  it('creates custom tokens with an ad-hoc definition', () => {
    const g = game();
    act(g, 0, { type: 'createToken', seat: 0, custom: { name: 'Spirit', pt: '1/1', colors: ['W'] } });
    const t = g.seats[0].zones.battlefield.map((i) => g.cards[i]).find((c) => c.token)!;
    expect(g.defs[t.def].name).toBe('Spirit');
  });
});

describe('redaction', () => {
  it('hides opponent hands and libraries, shows your own', () => {
    const g = game();
    const v1 = viewFor(g, 1);
    const myHand = g.seats[1].zones.hand[0];
    const theirHand = g.seats[0].zones.hand[0];
    expect(v1.cards[myHand].hidden).toBeFalsy();
    expect(v1.cards[theirHand].hidden).toBe(true);
    expect(v1.cards[theirHand].def).toBe('');
    expect(v1.cards[g.seats[1].zones.library[0]].hidden).toBe(true);
    const spec = viewFor(g, 'spectator');
    expect(spec.cards[theirHand].hidden).toBeFalsy();
  });
  it('face-down cards are hidden to others but visible to the controller', () => {
    const g = game();
    const iid = g.seats[0].zones.hand[0];
    act(g, 0, { type: 'move', iids: [iid], seat: 0, zone: 'battlefield', faceDown: true });
    expect(viewFor(g, 0).cards[iid].def).toBe(g.cards[iid].def);
    expect(viewFor(g, 1).cards[iid].hidden).toBe(true);
    const r = act(g, 0, { type: 'move', iids: [iid], seat: 0, zone: 'graveyard' });
    expect(r.logs[0].text).toMatch(/revealed: it was face down/);
  });
  it('hand-view grants', () => {
    const g = game();
    act(g, 1, { type: 'requestHandView', seat: 0 });
    const req = g.requests[0];
    act(g, 0, { type: 'answerHandView', id: req.id, grant: true });
    expect(viewFor(g, 1).cards[g.seats[0].zones.hand[0]].hidden).toBeFalsy();
    act(g, 0, { type: 'revokeHandView' });
    expect(viewFor(g, 1).cards[g.seats[0].zones.hand[0]].hidden).toBe(true);
  });
  it('revealTop exposes only the top card', () => {
    const g = game();
    act(g, 0, { type: 'toggleRevealTop', seat: 0 });
    const v = viewFor(g, 1);
    expect(v.cards[g.seats[0].zones.library[0]].hidden).toBeFalsy();
    expect(v.cards[g.seats[0].zones.library[1]].hidden).toBe(true);
  });
});

describe('library actions', () => {
  it('scry: order top and bottom', () => {
    const g = game();
    const [a, b, c] = g.seats[0].zones.library;
    act(g, 0, { type: 'lookResolve', seat: 0, mode: 'scry', items: [{ iid: c, dest: 'top' }, { iid: a, dest: 'bottom' }, { iid: b, dest: 'top' }] });
    const lib = g.seats[0].zones.library;
    expect(lib.slice(0, 2)).toEqual([c, b]);
    expect(lib[lib.length - 1]).toBe(a);
  });
  it('rejects a stale look resolution', () => {
    const g = game();
    const deep = g.seats[0].zones.library[10];
    expect(() => act(g, 0, { type: 'lookResolve', seat: 0, mode: 'scry', items: [{ iid: deep, dest: 'top' }] })).toThrow(IntentError);
  });
  it('mill and exile top face down', () => {
    const g = game();
    act(g, 0, { type: 'mill', seat: 1, n: 3 });
    expect(g.seats[1].zones.graveyard).toHaveLength(3);
    const r = act(g, 0, { type: 'exileTop', seat: 1, n: 1, faceDown: true });
    const ex = g.cards[g.seats[1].zones.exile[0]];
    expect(ex.faceDown).toBe(true);
    expect(ex.peekers.sort()).toEqual([0, 1]);
    expect(r.logs[0].privateTo).toContain(0);
    expect(viewFor(g, 1).cards[ex.iid].hidden).toBeFalsy();
  });
});

describe('mulligans', () => {
  it('London: draw 7, bottom one per mulligan', () => {
    const g = game(2, { mulligan: 'london' });
    act(g, 0, { type: 'mulligan' });
    expect(g.seats[0].zones.hand).toHaveLength(7);
    expect(() => act(g, 0, { type: 'keep', bottom: [] })).toThrow(IntentError);
    act(g, 0, { type: 'keep', bottom: [g.seats[0].zones.hand[0]] });
    expect(g.seats[0].zones.hand).toHaveLength(6);
    act(g, 1, { type: 'keep', bottom: [] });
    expect(g.stage).toBe('play');
  });
  it('free first mulligan', () => {
    const g = game(2, { mulligan: 'london-free' });
    act(g, 0, { type: 'mulligan' });
    act(g, 0, { type: 'keep', bottom: [] });
    expect(g.seats[0].zones.hand).toHaveLength(7);
  });
  it('free mulligans: unlimited, always 7, nothing to the bottom', () => {
    const g = game(2, { mulligan: 'free' });
    for (let i = 0; i < 12; i++) {
      act(g, 0, { type: 'mulligan' });
      expect(g.seats[0].zones.hand).toHaveLength(7);
    }
    const lib = g.seats[0].zones.library.length;
    act(g, 0, { type: 'keep', bottom: [] });
    expect(g.seats[0].zones.hand).toHaveLength(7);
    expect(g.seats[0].zones.library).toHaveLength(lib);
    expect(g.seats[0].mulligan).toEqual({ count: 12, kept: true });
  });
  it('Vancouver draws one fewer and asks for a scry', () => {
    const g = game(2, { mulligan: 'vancouver' });
    act(g, 0, { type: 'mulligan' });
    expect(g.seats[0].zones.hand).toHaveLength(6);
    const r = act(g, 0, { type: 'keep', bottom: [] });
    expect(r.notices.some((n) => n.kind === 'vancouverScry')).toBe(true);
  });
  it('free mulligans + scry 3: the hand and the top three go to the bottom, keep asks for a scry 3', () => {
    const g = game(2, { mulligan: 'free-scry3' });
    // hidden cards get new ids when they move, so compare what they are
    const defs = (ids: string[]) => ids.map((i) => g.cards[i].def).sort();
    const before = defs([...g.seats[0].zones.hand, ...g.seats[0].zones.library.slice(0, 3)]);
    const size = g.seats[0].zones.library.length;
    act(g, 0, { type: 'mulligan' });
    const lib = g.seats[0].zones.library;
    expect(g.seats[0].zones.hand).toHaveLength(7);
    expect(lib).toHaveLength(size);
    expect(defs(lib.slice(-10))).toEqual(before);
    const r = act(g, 0, { type: 'keep', bottom: [] });
    expect(r.notices.find((n) => n.kind === 'vancouverScry')).toMatchObject({ n: 3 });
  });
  it('free mulligans + scry 3: scry while deciding; a mulligan then moves only the hand and the cards still on top', () => {
    const g = game(2, { mulligan: 'free-scry3' });
    const defs = (ids: string[]) => ids.map((i) => g.cards[i].def).sort();
    const top = g.seats[0].zones.library.slice(0, 3);
    act(g, 0, { type: 'look', seat: 0, n: 3, mode: 'scry' });
    act(g, 0, { type: 'lookResolve', seat: 0, mode: 'scry', items: [{ iid: top[2], dest: 'top' }, { iid: top[0], dest: 'top' }, { iid: top[1], dest: 'bottom' }] });
    expect(g.seats[0].zones.library.slice(0, 2)).toEqual([top[2], top[0]]);
    expect(g.seats[0].mulligan).toMatchObject({ scryLeft: 2, scried: true });
    const bottomed = g.cards[top[1]].def;
    const before = defs([...g.seats[0].zones.hand, top[2], top[0]]);
    act(g, 0, { type: 'mulligan' });
    const lib = g.seats[0].zones.library;
    expect(defs(lib.slice(-9))).toEqual(before);
    expect(g.cards[lib[lib.length - 10]].def).toBe(bottomed);
    expect(g.seats[0].mulligan).toMatchObject({ count: 1, kept: false });
    expect(g.seats[0].mulligan?.scryLeft).toBeUndefined();
    // scry the new hand's three, then keep: no second scry
    act(g, 0, { type: 'look', seat: 0, n: 3, mode: 'scry' });
    const t2 = g.seats[0].zones.library.slice(0, 3);
    act(g, 0, { type: 'lookResolve', seat: 0, mode: 'scry', items: t2.map((iid) => ({ iid, dest: 'top' as const })) });
    const r = act(g, 0, { type: 'keep', bottom: [] });
    expect(r.notices.some((n) => n.kind === 'vancouverScry')).toBe(false);
  });
  it('Paris draws one fewer without a scry', () => {
    const g = game(2, { mulligan: 'paris' });
    act(g, 0, { type: 'mulligan' });
    act(g, 0, { type: 'mulligan' });
    expect(g.seats[0].zones.hand).toHaveLength(5);
    const r = act(g, 0, { type: 'keep', bottom: [] });
    expect(r.notices.some((n) => n.kind === 'vancouverScry')).toBe(false);
  });
});

describe('misc', () => {
  it('owners-only blocks touching other players cards', () => {
    const g = game(2, { ownersOnly: true });
    const iid = g.seats[0].zones.hand[0];
    act(g, 0, { type: 'move', iids: [iid], seat: 0, zone: 'battlefield' });
    expect(() => act(g, 1, { type: 'tap', iids: [iid] })).toThrow(IntentError);
  });
  it('transform switches faces and names', () => {
    const g = game();
    const c = g.cards[g.seats[0].zones.hand[0]];
    c.def = 'p:dfc';
    g.defs['p:dfc'] = DEFS['p:dfc'];
    act(g, 0, { type: 'move', iids: [c.iid], seat: 0, zone: 'battlefield' });
    const r = act(g, 0, { type: 'transform', iid: c.iid });
    expect(r.logs[0].text).toBe('transformed Delver of Secrets into Insectile Aberration');
  });
  it('initiative ventures into the Undercity', () => {
    const g = game();
    act(g, 0, { type: 'marker', key: 'initiative', seat: 1 });
    expect(g.global.initiative).toBe(1);
    expect(g.seats[1].dungeon).toEqual({ name: 'Undercity', room: 0 });
  });
  it('locate finds cards', () => {
    const g = game();
    const iid = g.seats[1].zones.graveyard[0] ?? g.seats[1].zones.hand[0];
    expect(locate(g, iid)?.seat).toBe(1);
  });
});

describe('planechase and archenemy', () => {
  const PDEFS: Record<string, CardDef> = {
    ...DEFS,
    ...Object.fromEntries(
      Array.from({ length: 10 }, (_, i) => [`p:plane${i}`, def(`p:plane${i}`, `Plane ${i}`, { layout: 'planar', typeLine: 'Plane — Test', oracleText: `Static ${i}.\nWhenever you roll {CHAOS}, draw ${i} cards.`, power: undefined, toughness: undefined })]),
    ),
    'p:phen': def('p:phen', 'Chaotic Aether', { layout: 'planar', typeLine: 'Phenomenon', oracleText: 'When you encounter Chaotic Aether, each blank roll counts as chaos.', power: undefined, toughness: undefined }),
    'p:scheme': def('p:scheme', 'Your Fate Is Thrice Sealed', { typeLine: 'Scheme', oracleText: 'When you set this scheme in motion, reveal the top three cards.', power: undefined, toughness: undefined }),
    'p:ongoing': def('p:ongoing', 'Every Hope Shall Vanish', { typeLine: 'Ongoing Scheme', oracleText: '(An ongoing scheme remains face up.)', power: undefined, toughness: undefined }),
  };
  const special = (settings: Partial<typeof DEFAULT_SETTINGS>, extra: SeatInput['cards'][number][]) =>
    createGame({
      id: 'g',
      settings: { ...DEFAULT_SETTINGS, ...settings },
      seats: [0, 1].map((i) => seatIn(i, [{ def: 'p:bear', section: 'main', qty: 20 }, ...(i === 0 ? extra : [])])),
      defs: PDEFS,
      rng: seqRng(),
      now: 0,
    });
  const act2 = (g: GameState, actor: number, intent: Intent, value?: number) => {
    const ctx: ApplyCtx = { actor, actorName: g.seats[actor].name, now: 1 };
    const resolved = resolveIntent(g, ctx, intent, seqRng(7));
    if (value != null) resolved.value = value;
    return applyIntent(g, ctx, intent, resolved);
  };

  it('the planar die costs one more each roll in a turn, and resets on the next turn; chaos shows the plane’s chaos text', () => {
    const g = special({ planechase: true }, Array.from({ length: 10 }, (_, i) => ({ def: `p:plane${i}`, section: 'planar' as const, qty: 1 })));
    expect(g.table.planes).toHaveLength(1);
    const r1 = act2(g, 0, { type: 'planarRoll' }, 4);
    expect(r1.notices.some((n) => n.kind === 'toast' && /\{CHAOS\} → Plane \d: draw/.test(n.text))).toBe(true);
    expect(g.table.planarRolls).toBe(1);
    const r2 = act2(g, 0, { type: 'planarRoll' }, 1);
    expect(r2.logs.some((e) => /paid \{1\}/.test(e.text))).toBe(true);
    act2(g, 0, { type: 'passTurn' });
    expect(g.table.planarRolls).toBe(0);
  });

  it('a phenomenon is encountered, then planeswalking moves on; extra planes stay together', () => {
    const g = special({ planechase: true }, [{ def: 'p:phen', section: 'planar', qty: 1 }, ...Array.from({ length: 9 }, (_, i) => ({ def: `p:plane${i}`, section: 'planar' as const, qty: 1 }))]);
    // put the phenomenon on top of the planar deck
    const ph = Object.values(g.cards).find((c) => c.def === 'p:phen')!.iid;
    g.table.planarDeck = [ph, ...g.table.planarDeck.filter((x) => x !== ph)];
    g.table.planes = g.table.planes.filter((x) => x !== ph);
    act2(g, 0, { type: 'planeswalk' });
    expect(g.table.encounter).toBe(ph);
    act2(g, 0, { type: 'planeswalk' });
    expect(g.table.encounter).toBeNull();
    act2(g, 0, { type: 'planeswalk', extra: true });
    expect(g.table.planes).toHaveLength(2);
  });

  it('the archenemy has 40 life and sets a scheme in motion at the start of their first main phase; one-shot schemes go to the bottom', () => {
    const g = special({ archenemy: true, life: 20 }, [
      { def: 'p:scheme', section: 'schemes', qty: 10 },
      { def: 'p:ongoing', section: 'schemes', qty: 10 },
    ]);
    expect(g.seats[0].archenemy).toBe(true);
    expect(g.seats[0].life).toBe(40);
    expect(g.seats[1].life).toBe(20);
    g.turn.active = 0;
    act2(g, 0, { type: 'phase', phase: 3 });
    expect(g.seats[0].zones.schemes).toHaveLength(1);
    // the next set in motion moves a one-shot scheme back to the bottom first
    const first = g.seats[0].zones.schemes[0];
    const oneShot = g.cards[first].def === 'p:scheme';
    act2(g, 0, { type: 'setInMotion', seat: 0 });
    if (oneShot) expect(g.seats[0].zones.schemeDeck.at(-1)).toBe(first);
    else expect(g.seats[0].zones.schemes).toContain(first);
  });
});
