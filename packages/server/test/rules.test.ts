import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import type { FastifyInstance } from 'fastify';
import { nativePath, RulesGame } from '@aetherhub/rules';
import type { Ctx } from '../src/context';

const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'aetherhub-rules-'));
process.env.DATA_DIR = dir;
process.env.NO_AUTO_DOWNLOAD = '1';
process.env.SEED_CARDS = path.resolve(__dirname, '../../../fixtures/cards-fixture.jsonl');
process.env.CLIENT_DIR = '/nonexistent';
process.env.IMAGE_OFFLINE = '1';
process.env.RULES_CARDS = path.resolve(__dirname, '../../../cards');

const hasNative = !!nativePath();
// CI sets REQUIRE_NATIVE so a missing addon fails the run instead of silently skipping every rules game test.
if (!hasNative && process.env.REQUIRE_NATIVE) throw new Error('REQUIRE_NATIVE is set but the rules engine addon was not found (set RULES_NATIVE)');
let f: FastifyInstance;
let ctx: Ctx;

/**
 * Every other engine call takes the lock a running `advance()` (an AI turn) holds, on the Node main thread: made while an
 * advance is in flight, it would stall the whole server until the AI is done. Watch every game for that, and count the
 * synchronous turn getter (the running-games lists must not use it).
 */
const inflight = new WeakMap<object, number>();
let advancing = 0;
const violations: string[] = [];
let turnCalls = 0;
function guardEngineCalls() {
  const P = RulesGame.prototype as unknown as Record<string, unknown>;
  const advance = P.advance as (this: object) => Promise<unknown>;
  P.advance = async function (this: object) {
    inflight.set(this, (inflight.get(this) ?? 0) + 1);
    advancing++;
    try {
      return await advance.call(this);
    } finally {
      inflight.set(this, inflight.get(this)! - 1);
      advancing--;
    }
  };
  for (const k of Object.getOwnPropertyNames(P)) {
    if (k === 'constructor' || k === 'advance') continue;
    const d = Object.getOwnPropertyDescriptor(P, k)!;
    if (d.get) {
      const get = d.get;
      Object.defineProperty(P, k, {
        ...d,
        get(this: object) {
          if (k === 'turn') turnCalls++;
          if (inflight.get(this)) violations.push(k);
          return get.call(this);
        },
      });
    } else if (typeof d.value === 'function') {
      const fn = d.value as (...a: unknown[]) => unknown;
      P[k] = function (this: object, ...a: unknown[]) {
        if (inflight.get(this)) violations.push(k);
        return fn.apply(this, a);
      };
    }
  }
}

beforeAll(async () => {
  if (!hasNative) return;
  guardEngineCalls();
  const { buildApp } = await import('../src/app');
  const built = await buildApp();
  f = built.f;
  ctx = built.ctx;
  await built.ctx.carddata.importFile(process.env.SEED_CARDS!);
});
afterAll(async () => {
  await f?.close();
  fs.rmSync(dir, { recursive: true, force: true });
});

async function req(method: string, url: string, body?: unknown, cookie?: string) {
  const r = await f.inject({ method: method as 'GET', url, payload: body as object, headers: cookie ? { cookie } : {} });
  let json: any = null;
  try {
    json = r.body ? JSON.parse(r.body) : null;
  } catch {
    json = r.body;
  }
  return { status: r.statusCode, json, cookie: (r.headers['set-cookie'] as string | undefined)?.split(';')[0] };
}

/** The simplest legal answer to any question: play lands, otherwise pass; no attacks, no blocks. */
function simple(d: any): unknown {
  const k = d.kind;
  switch (k.t) {
    case 'Mulligan': return { t: 'Keep', keep: true };
    case 'Bottom': return { t: 'Bottom', cards: k.hand.slice(0, k.n) };
    case 'Priority': {
      const land = k.actions.findIndex((a: any) => a.t === 'PlayLand');
      return { t: 'Priority', action: land >= 0 ? land : null };
    }
    case 'Pick': return { t: 'Pick', indices: Array.from({ length: k.min }, (_: unknown, i: number) => i) };
    case 'Number': return { t: 'Number', n: k.min };
    case 'YesNo': return { t: 'YesNo', yes: false };
    case 'PayMana': return { t: 'Pay', answer: { t: 'Auto' } };
    case 'Attackers': return { t: 'Attackers', attacks: [] };
    case 'Blockers': return { t: 'Blockers', blocks: [] };
    case 'AssignDamage': return { t: 'Damage', assign: [] };
    default: return { t: 'Default' };
  }
}

/** Polls the game and answers every question for the listed players until it is over. */
async function drive(id: string, cookies: string[], until?: (s: any) => boolean): Promise<any> {
  const deadline = Date.now() + 120_000;
  let since: number | undefined;
  let epoch: number | undefined;
  for (;;) {
    let last: any = null;
    let progressed = false;
    for (const c of cookies) {
      const q = since !== undefined ? `?since=${since}&epoch=${epoch}` : '';
      const s = (await req('GET', `/api/rules/games/${id}${q}`, undefined, c)).json;
      last = s;
      if (s.error) throw new Error(s.error);
      if (s.over || (until && until(s))) return s;
      if (s.waiting) {
        const r = await req('POST', `/api/rules/games/${id}/answer`, { choice: simple(s.waiting), decisionId: s.waitingId }, c);
        expect(r.status, JSON.stringify(r.json)).toBe(200);
        progressed = true;
      }
    }
    if (!progressed) await new Promise((r) => setTimeout(r, 20));
    if (Date.now() > deadline) throw new Error('timed out ' + JSON.stringify(last?.waiting));
  }
}

let cookie = '';
let cookie2 = '';
let deckId = 0;
let deckId2 = 0;

describe.skipIf(!hasNative)('rules games (native engine)', () => {
  it('reports the engine as available', async () => {
    const reg = await req('POST', '/api/setup/admin', { username: 'alice', password: 'hunter22' });
    cookie = reg.cookie!;
    const st = await req('GET', '/api/rules/status', undefined, cookie);
    expect(st.json.available, JSON.stringify(st.json)).toBe(true);
    expect(st.json.cards).toBeGreaterThan(40);
  });

  it('creates decks and checks which cards the engine supports', async () => {
    const mk = async (name: string, text: string, ck: string) => {
      const p = await req('POST', '/api/import/parse', { text }, ck);
      const cards = p.json.matched.map((m: any) => ({ key: m.key, section: m.section, qty: m.qty }));
      const d = await req('POST', '/api/import/commit', { name, format: 'modern', cards }, ck);
      expect(d.status, JSON.stringify(d.json)).toBe(200);
      return d.json.id as number;
    };
    deckId = await mk('Burn', 'Main\n22 Mountain\n14 Lightning Bolt\n12 Llanowar Elves\n4 Rancor', cookie);
    const chk = await req('POST', '/api/rules/check', { deckIds: [deckId] }, cookie);
    expect(chk.json[deckId].unsupported).toEqual([]);
    expect(chk.json[deckId].playable).toBe(52);
  });

  it('plays a full game vs the AI, then undoes, saves and resumes it', async () => {
    const g = await req('POST', '/api/rules/games', { deckId, seed: 7, ais: [{ deckId: null }] }, cookie);
    expect(g.status, JSON.stringify(g.json)).toBe(200);
    const id = g.json.id;
    const done = await drive(id, [cookie]);
    expect(done.over).toBeTruthy();
    expect(done.view.players.length).toBe(2);
    expect(done.view.cards.some((c: any) => c.zone === 'Battlefield')).toBe(true);

    // undo the last two answers: the game is live again, and still ends
    const u = await req('POST', `/api/rules/games/${id}/undo`, { steps: 2 }, cookie);
    expect(u.status, JSON.stringify(u.json)).toBe(200);
    const again = await drive(id, [cookie]);
    expect(again.over).toBeTruthy();

    // save mid-way and resume
    const u2 = await req('POST', `/api/rules/games/${id}/undo`, { steps: 4 }, cookie);
    expect(u2.status).toBe(200);
    const mid = await drive(id, [cookie], (s) => !!s.waiting);
    expect(mid.waiting).toBeTruthy();
    const sv = await req('POST', `/api/rules/games/${id}/save`, { name: 'midgame' }, cookie);
    expect(sv.status, JSON.stringify(sv.json)).toBe(200);
    expect((await req('GET', '/api/rules/saves', undefined, cookie)).json[0].name).toBe('midgame');
    expect((await req('DELETE', `/api/rules/games/${id}`, undefined, cookie)).status).toBe(200);
    const rs = await req('POST', `/api/rules/saves/${sv.json.id}/resume`, {}, cookie);
    expect(rs.status, JSON.stringify(rs.json)).toBe(200);
    const fin = await drive(rs.json.id, [cookie]);
    expect(fin.over).toBeTruthy();
  });

  it('passes priority until a target, cancels it, remembers reactions, lists running games, and replays it all on undo', async () => {
    const g = await req('POST', '/api/rules/games', { deckId, seed: 5, ais: [{ deckId: null }] }, cookie);
    expect(g.status, JSON.stringify(g.json)).toBe(200);
    const id = g.json.id;
    const first = await drive(id, [cookie], (s) => !!s.waiting);
    expect(first.waiting.kind.t).toBe('Mulligan');
    expect((await req('GET', '/api/rules/mine', undefined, cookie)).json.map((x: any) => x.id)).toContain(id);

    // a pass set while another question is open is reported, and cancelling clears it
    expect((await req('POST', `/api/rules/games/${id}/passuntil`, { turn: 9, step: 0 }, cookie)).status).toBe(200);
    expect((await req('GET', `/api/rules/games/${id}`, undefined, cookie)).json.passing).toEqual({ Step: { turn: 9, step: 'Untap' } });
    expect((await req('POST', `/api/rules/games/${id}/passuntil/cancel`, {}, cookie)).status).toBe(200);
    expect((await req('GET', `/api/rules/games/${id}`, undefined, cookie)).json.passing).toBeNull();

    // remembered reactions are listed and can be forgotten
    expect((await req('POST', `/api/rules/games/${id}/reactions`, { kind: 'ignore', source: 'Lightning Bolt', prompt: 'x' }, cookie)).status).toBe(200);
    const withR = (await req('GET', `/api/rules/games/${id}`, undefined, cookie)).json;
    expect(withR.reactions).toHaveLength(1);
    expect((await req('DELETE', `/api/rules/games/${id}/reactions/${withR.reactions[0].id}`, undefined, cookie)).status).toBe(200);
    expect((await req('GET', `/api/rules/games/${id}`, undefined, cookie)).json.reactions).toHaveLength(0);

    // end turn: no plain priority is asked before turn 4, then the game plays on to the end
    expect((await req('POST', `/api/rules/games/${id}/passuntil`, { turn: 4, step: 0 }, cookie)).status).toBe(200);
    const done = await drive(id, [cookie], (s) => {
      if (s.waiting?.kind.t === 'Priority') expect(s.view.turn).toBeGreaterThanOrEqual(4);
      return false;
    });
    expect(done.over).toBeTruthy();
    // undo replays the recorded pass, and the game still ends
    expect((await req('POST', `/api/rules/games/${id}/undo`, { steps: 1 }, cookie)).status).toBe(200);
    expect((await drive(id, [cookie])).over).toBeTruthy();
    expect((await req('DELETE', `/api/rules/games/${id}`, undefined, cookie)).status).toBe(200);
    expect((await req('GET', '/api/rules/mine', undefined, cookie)).json.map((x: any) => x.id)).not.toContain(id);
  });

  it('breaks the rules: pauses, applies edits that undo and save keep, then carries on', async () => {
    const g = await req('POST', '/api/rules/games', { deckId, seed: 9, ais: [{ deckId: null }] }, cookie);
    const id = g.json.id;
    const pri = await drive(id, [cookie], (s) => s.waiting?.kind.t === 'Priority');
    const life = pri.view.players[0].life;
    expect((await req('POST', `/api/rules/games/${id}/break`, { on: true }, cookie)).status).toBe(200);
    const paused = (await req('GET', `/api/rules/games/${id}`, undefined, cookie)).json;
    expect(paused.paused).toMatchObject({ seat: 0, mine: true });
    // nobody acts while paused
    expect((await req('POST', `/api/rules/games/${id}/answer`, { choice: { t: 'Priority', action: null } }, cookie)).status).toBe(400);
    expect((await req('POST', `/api/rules/games/${id}/edit`, { op: { t: 'Life', player: 0, delta: 7 } }, cookie)).status).toBe(200);
    const hand = paused.view.cards.find((c: any) => c.zone === 'Hand' && c.owner === 0);
    expect((await req('POST', `/api/rules/games/${id}/edit`, { op: { t: 'Move', card: hand.id, to: 'Graveyard' } }, cookie)).status).toBe(200);
    const edited = (await req('GET', `/api/rules/games/${id}`, undefined, cookie)).json;
    expect(edited.view.players[0].life).toBe(life + 7);
    expect(edited.view.cards.find((c: any) => c.id === hand.id).zone).toBe('Graveyard');
    expect((await req('POST', `/api/rules/games/${id}/edit`, { op: { t: 'Life', player: 9, delta: 1 } }, cookie)).status).toBe(400);
    // tokens a card failed to make: a named token, and a token copy
    const tokens = (await req('GET', '/api/rules/tokens', undefined, cookie)).json;
    expect(tokens).toContain('Soldier');
    expect((await req('POST', `/api/rules/games/${id}/edit`, { op: { t: 'CreateToken', player: 0, name: 'Soldier', n: 2 } }, cookie)).status).toBe(200);
    const withTokens = (await req('GET', `/api/rules/games/${id}`, undefined, cookie)).json;
    const soldiers = withTokens.view.cards.filter((c: any) => c.name === 'Soldier' && c.zone === 'Battlefield' && c.token);
    expect(soldiers).toHaveLength(2);
    expect((await req('POST', `/api/rules/games/${id}/edit`, { op: { t: 'TokenCopy', card: soldiers[0].id, player: 1 } }, cookie)).status).toBe(200);
    const copied = (await req('GET', `/api/rules/games/${id}`, undefined, cookie)).json;
    expect(copied.view.cards.filter((c: any) => c.name === 'Soldier' && c.zone === 'Battlefield' && c.controller === 1)).toHaveLength(1);
    expect((await req('POST', `/api/rules/games/${id}/break`, { on: false }, cookie)).status).toBe(200);
    const after = await drive(id, [cookie], (s) => !!s.waiting);
    expect(after.paused).toBeNull();
    // saving and resuming replays the edits
    const sv = await req('POST', `/api/rules/games/${id}/save`, { name: 'broken' }, cookie);
    expect(sv.status).toBe(200);
    expect((await req('DELETE', `/api/rules/games/${id}`, undefined, cookie)).status).toBe(200);
    const rs = await req('POST', `/api/rules/saves/${sv.json.id}/resume`, {}, cookie);
    expect(rs.status, JSON.stringify(rs.json)).toBe(200);
    const back = await drive(rs.json.id, [cookie], (s) => !!s.waiting);
    expect(back.view.cards.find((c: any) => c.id === hand.id).zone).not.toBe('Hand');
    expect((await drive(rs.json.id, [cookie])).over).toBeTruthy();
  });

  it('plays a 4-player commander-less game against three AIs', async () => {
    // Easy AIs: this checks a 4-player game end to end, not AI strength (Normal thinks 250 ms per decision: 20+ seconds a game)
    const g = await req('POST', '/api/rules/games', { deckId, seed: 11, ais: [{ difficulty: 'Easy' }, { difficulty: 'Easy' }, { difficulty: 'Easy' }] }, cookie);
    expect(g.status, JSON.stringify(g.json)).toBe(200);
    // the admin Server page lists it while it runs
    const listed = (await req('GET', '/api/admin/games', undefined, cookie)).json as { id: string; kind: string; players: string[]; state: string }[];
    const mine = listed.find((x) => x.id === g.json.id);
    expect(mine).toMatchObject({ kind: 'rules', state: 'running' });
    expect(mine!.players.filter((p) => p.endsWith('(AI)'))).toHaveLength(3);
    expect(Array.isArray((await req('GET', '/api/admin/log', undefined, cookie)).json)).toBe(true);
    const done = await drive(g.json.id, [cookie]);
    expect(done.view.players.length).toBe(4);
    expect(done.over).toBeTruthy();
  }, 60_000);

  it('lets two humans play each other via an open seat', async () => {
    const reg = await req('POST', '/api/admin/users', { username: 'friend' }, cookie);
    expect(reg.status, JSON.stringify(reg.json)).toBe(200);
    const login = await req('POST', '/api/auth/login', { username: 'friend', password: reg.json.tempPassword });
    cookie2 = login.cookie!;
    deckId2 = await (async () => {
      const p = await req('POST', '/api/import/parse', { text: 'Main\n20 Plains\n16 Swords to Plowshares\n12 Serra Angel' }, cookie2);
      const cards = p.json.matched.map((m: any) => ({ key: m.key, section: m.section, qty: m.qty }));
      return (await req('POST', '/api/import/commit', { name: 'Angels', format: 'modern', cards }, cookie2)).json.id as number;
    })();
    const g = await req('POST', '/api/rules/games', { deckId, open: 1, seed: 3 }, cookie);
    expect(g.status).toBe(200);
    const open = await req('GET', '/api/rules/open', undefined, cookie2);
    expect(open.json.map((x: any) => x.id)).toContain(g.json.id);
    const j = await req('POST', `/api/rules/games/${g.json.id}/join`, { deckId: deckId2 }, cookie2);
    expect(j.status, JSON.stringify(j.json)).toBe(200);
    const done = await drive(g.json.id, [cookie, cookie2]);
    expect(done.over).toBeTruthy();
    // each player sees only their own hand
    const a = (await req('GET', `/api/rules/games/${g.json.id}`, undefined, cookie)).json.view;
    expect(a.cards.filter((c: any) => c.zone === 'Hand' && c.owner !== 0)).toHaveLength(0);
  });

  it('starts a rules game from a lobby against an AI', async () => {
    const made = await req('POST', '/api/lobbies', { settings: { name: 'Lobby rules', type: 'rules', format: 'modern', maxPlayers: 2 } }, cookie);
    expect(made.status, JSON.stringify(made.json)).toBe(200);
    const L = `/api/lobbies/${made.json.lobby.id}`;
    expect((await req('POST', `${L}/seats/1`, { ai: 'Easy' }, cookie)).status).toBe(200);
    await req('PUT', `${L}/seats/0/deck`, { deckId }, cookie);
    await req('PUT', `${L}/seats/1/deck`, { deckId: deckId2 }, cookie);
    await req('PUT', `${L}/ready`, { ready: true }, cookie);
    const st = await req('POST', `${L}/start`, {}, cookie);
    expect(st.status, JSON.stringify(st.json)).toBe(200);
    expect(st.json.kind).toBe('rules');
    const done = await drive(st.json.id, [cookie]);
    expect(done.over).toBeTruthy();
    expect(done.names[1]).toMatch(/^AI /);
  }, 60_000);

  it('plays a solo rules game: one player holds both seats and answers for whichever must act', async () => {
    const made = await req('POST', '/api/lobbies', { settings: { name: 'Solo rules', type: 'rules', format: 'modern', maxPlayers: 2 } }, cookie);
    const L = `/api/lobbies/${made.json.lobby.id}`;
    const me = (await req('GET', '/api/me', undefined, cookie)).json.id;
    expect((await req('POST', `${L}/seats/1`, { userId: me }, cookie)).status).toBe(200);
    await req('PUT', `${L}/seats/0/deck`, { deckId }, cookie);
    await req('PUT', `${L}/seats/1/deck`, { deckId: deckId2 }, cookie);
    await req('PUT', `${L}/ready`, { ready: true }, cookie);
    const st = await req('POST', `${L}/start`, {}, cookie);
    expect(st.status, JSON.stringify(st.json)).toBe(200);
    const first = (await req('GET', `/api/rules/games/${st.json.id}`, undefined, cookie)).json;
    expect(first.mySeats).toEqual([0, 1]);
    // asking for the other seat explicitly works too
    expect((await req('GET', `/api/rules/games/${st.json.id}?seat=1`, undefined, cookie)).json.seat).toBe(1);
    const done = await drive(st.json.id, [cookie]);
    expect(done.over).toBeTruthy();
  }, 60_000);

  it('lists running games from cached data while the AIs think, never waiting on the engine', async () => {
    // Normal AIs think about 250 ms a decision: the watched game below is inside advance() most of the time
    const g = await req('POST', '/api/rules/games', { deckId, seed: 13, ais: [{}, {}] }, cookie);
    expect(g.status, JSON.stringify(g.json)).toBe(200);
    const id = g.json.id;
    await drive(id, [cookie], (s) => !!s.waiting);
    // the player concedes; the two AIs play on to be watched, one AI decision per advance
    expect((await req('POST', `/api/rules/games/${id}/concede`, {}, cookie)).status).toBe(200);
    turnCalls = 0;
    let busy = 0;
    let start: number | null = null;
    let turn = 0;
    const deadline = Date.now() + 40_000;
    while (Date.now() < deadline && (busy < 3 || start === null || turn <= start)) {
      const during = advancing > 0;
      const mine = await req('GET', '/api/rules/mine', undefined, cookie);
      const active = await req('GET', '/api/rules/active', undefined, cookie);
      if (during && advancing > 0) busy++;
      expect(mine.status).toBe(200);
      expect(mine.json.find((x: any) => x.id === id)).toBeTruthy();
      turn = active.json.find((x: any) => x.id === id)?.turn ?? 0;
      start ??= turn;
      // keep watching (an unwatched AI-only game rests)
      expect((await req('GET', `/api/rules/games/${id}`, undefined, cookie)).json.error).toBeNull();
      await new Promise((r) => setTimeout(r, 30));
    }
    expect(busy, 'the lists were asked while an AI was thinking').toBeGreaterThanOrEqual(3);
    expect(turn, 'the listed turn follows the game').toBeGreaterThan(start!);
    expect(turnCalls, 'the lists do not ask the engine for the turn').toBe(0);
    expect((await req('DELETE', `/api/rules/games/${id}`, undefined, cookie)).status).toBe(200);
  }, 60_000);

  it('refuses an answer to a question that is no longer pending', async () => {
    const g = await req('POST', '/api/rules/games', { deckId, seed: 21, ais: [{ difficulty: 'Easy' }] }, cookie);
    const id = g.json.id;
    const st = await drive(id, [cookie], (s) => !!s.waiting);
    const choice = simple(st.waiting);
    const stale = await req('POST', `/api/rules/games/${id}/answer`, { choice, decisionId: st.waitingId - 1 }, cookie);
    expect(stale.status, JSON.stringify(stale.json)).toBe(409);
    expect(stale.json.error).toMatch(/no longer pending/);
    expect((await req('POST', `/api/rules/games/${id}/answer`, { choice, decisionId: st.waitingId }, cookie)).status).toBe(200);
    // the same answer sent twice: the second one is refused, whether or not the next question is out yet
    expect((await req('POST', `/api/rules/games/${id}/answer`, { choice, decisionId: st.waitingId }, cookie)).status).toBe(409);
    const next = await drive(id, [cookie], (s) => !!s.waiting);
    expect(next.waitingId).toBeGreaterThan(st.waitingId);
    expect((await req('DELETE', `/api/rules/games/${id}`, undefined, cookie)).status).toBe(200);
  });

  it('keeps games in progress across a restart: same id, same position, still playable', async () => {
    const g = await req('POST', '/api/rules/games', { deckId, seed: 17, ais: [{ difficulty: 'Easy' }] }, cookie);
    expect(g.status, JSON.stringify(g.json)).toBe(200);
    const id = g.json.id;
    const before = await drive(id, [cookie], (s) => s.waiting?.kind.t === 'Priority' && s.view.turn >= 2);
    const hand = (v: any) => v.cards.filter((c: any) => c.zone === 'Hand' && c.owner === 0).map((c: any) => c.id).sort();
    // a damaged record must not stop the server from starting
    const { run } = await import('../src/db/sqlite');
    run(ctx.app.db, "INSERT INTO rules_games (id, state, record, updated_at) VALUES ('damaged', '{not json', NULL, 0)");

    await f.close();
    const { buildApp } = await import('../src/app');
    const built = await buildApp();
    f = built.f;
    ctx = built.ctx;

    // listed at once (before the engine has replayed it), finished games are not brought back
    const listed = (await req('GET', '/api/rules/mine', undefined, cookie)).json;
    expect(listed.find((x: any) => x.id === id)).toMatchObject({ turn: before.view.turn, yourMove: true });
    expect((await req('GET', '/api/rules/active', undefined, cookie)).json.map((x: any) => x.id)).toEqual([id]);
    const after = await drive(id, [cookie], (s) => !!s.waiting);
    expect(after.view.turn).toBe(before.view.turn);
    expect(after.waiting.kind.t).toBe('Priority');
    expect(hand(after.view)).toEqual(hand(before.view));
    expect((await drive(id, [cookie])).over).toBeTruthy();
  }, 60_000);

  it('never made a synchronous engine call while an AI turn was running', () => {
    expect(violations).toEqual([]);
  });
});
