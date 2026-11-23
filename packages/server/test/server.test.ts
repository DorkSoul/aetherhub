import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import WebSocket from 'ws';
import type { FastifyInstance } from 'fastify';
import type { ServerMsg } from '@aetherhub/shared';

const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'aetherhub-test-'));
process.env.DATA_DIR = dir;
process.env.NO_AUTO_DOWNLOAD = '1';
process.env.SEED_CARDS = path.resolve(__dirname, '../../../fixtures/cards-fixture.jsonl');
process.env.CLIENT_DIR = '/nonexistent';
process.env.IMAGE_OFFLINE = '1';

let f: FastifyInstance;
let base = '';
let ctxRef: Awaited<ReturnType<typeof import('../src/app').buildApp>>['ctx'];

async function req(method: string, url: string, body?: unknown, cookie?: string) {
  const r = await f.inject({ method: method as 'GET', url, payload: body as object, headers: cookie ? { cookie } : {} });
  return { status: r.statusCode, json: r.body ? (() => { try { return JSON.parse(r.body); } catch { return r.body; } })() : null, cookie: (r.headers['set-cookie'] as string | undefined)?.split(';')[0], raw: r };
}

beforeAll(async () => {
  const { buildApp } = await import('../src/app');
  const built = await buildApp();
  f = built.f;
  ctxRef = built.ctx;
  await built.ctx.carddata.importFile(process.env.SEED_CARDS!);
  await f.listen({ port: 0, host: '127.0.0.1' });
  const addr = f.server.address() as { port: number };
  base = `127.0.0.1:${addr.port}`;
});

afterAll(async () => {
  await f?.close();
  fs.rmSync(dir, { recursive: true, force: true });
});

let admin = '';
let bob = '';

describe('auth', () => {
  it('first run creates the admin', async () => {
    const s = await req('GET', '/api/setup/state');
    expect(s.json.needsAdmin).toBe(true);
    expect(s.json.lan).toBe(true);
    const r = await req('POST', '/api/setup/admin', { username: 'alice', password: 'hunter22' });
    expect(r.status).toBe(200);
    expect(r.json.isAdmin).toBe(true);
    admin = r.cookie!;
    expect((await req('POST', '/api/setup/admin', { username: 'x', password: 'hunter22' })).status).toBe(409);
  });

  it('admin creates users with a temp password; login works; lockout after 5 fails', async () => {
    const c = await req('POST', '/api/admin/users', { username: 'bob' }, admin);
    expect(c.json.tempPassword).toMatch(/^\w{4}-\w{4}-\w{4}$/);
    const ok = await req('POST', '/api/auth/login', { username: 'bob', password: c.json.tempPassword });
    expect(ok.status).toBe(200);
    bob = ok.cookie!;
    for (let i = 0; i < 4; i++) expect((await req('POST', '/api/auth/login', { username: 'bob', password: 'nope' })).status).toBe(401);
    const locked = await req('POST', '/api/auth/login', { username: 'bob', password: 'nope' });
    expect(locked.status).toBe(429);
    expect(locked.json.error).toMatch(/Too many attempts/);
    expect((await req('GET', '/api/admin/users', undefined, bob)).status).toBe(403);
  });

  it('a new (spoofed) address per attempt does not get around the login limit', async () => {
    const c = await req('POST', '/api/admin/users', { username: 'spoofy' }, admin);
    const ok = c.json.tempPassword as string;
    const login = (n: number, password: string) =>
      f.inject({ method: 'POST', url: '/api/auth/login', payload: { username: 'spoofy', password }, headers: { 'x-forwarded-for': `203.0.113.${n}`, 'cf-connecting-ip': `198.51.100.${n}` } });
    const codes: number[] = [];
    for (let i = 1; i <= 20; i++) codes.push((await login(i, 'nope')).statusCode);
    expect(codes.slice(0, 19).every((x) => x === 401)).toBe(true);
    expect(codes[19]).toBe(429);
    // locked for every address now, even with the right password
    expect((await login(99, ok)).statusCode).toBe(429);
  });

  it('only proxies on private networks may set the client address', async () => {
    const direct = await f.inject({ method: 'GET', url: '/api/setup/state', remoteAddress: '203.0.113.7', headers: { 'x-forwarded-for': '10.0.0.5' } });
    expect(direct.json().lan).toBe(false);
    const viaProxy = await f.inject({ method: 'GET', url: '/api/setup/state', remoteAddress: '172.18.0.3', headers: { 'x-forwarded-for': '203.0.113.7' } });
    expect(viaProxy.json().lan).toBe(false);
    const cf = await f.inject({ method: 'GET', url: '/api/setup/state', remoteAddress: '203.0.113.7', headers: { 'cf-connecting-ip': '192.168.1.10' } });
    expect(cf.json().lan).toBe(false);
  });

  it('cannot demote the last admin; prefs persist', async () => {
    const users = (await req('GET', '/api/admin/users', undefined, admin)).json;
    const me = users.find((u: { username: string }) => u.username === 'alice');
    expect((await req('POST', `/api/admin/users/${me.id}/role`, { admin: false }, admin)).status).toBe(400);
    const p = await req('PUT', '/api/me/prefs', { accent: 'green', playerColor: '#4fb3a9' }, bob);
    expect(p.json.accent).toBe('green');
    expect((await req('GET', '/api/me', undefined, bob)).json.prefs.playerColor).toBe('#4fb3a9');
  });
});

describe('cards and decks', () => {
  let deckId = 0;
  it('searches the card DB', async () => {
    const r = await req('GET', '/api/cards/search?q=' + encodeURIComponent('t:instant c:r'), undefined, admin);
    expect(r.json.cards.some((c: { name: string }) => c.name === 'Lightning Bolt')).toBe(true);
    const t = await req('GET', '/api/cards/search?tokens=1&q=treasure', undefined, admin);
    expect(t.json.cards[0].isToken).toBe(true);
  });

  it('imports a list with review suggestions', async () => {
    const text = "Commander\n1 Atraxa, Praetor's Voice\n\nMain\n1 Sol Ring (C21) 263\n1 Counterspel\n30 Forest\n5 Island\n\nTokens\n1 Treasure";
    const p = await req('POST', '/api/import/parse', { text }, admin);
    expect(p.json.format).toBe('plain');
    const names = p.json.review.map((r: { name: string }) => r.name);
    expect(names).toContain('Counterspel');
    const rv = p.json.review.find((r: { name: string }) => r.name === 'Counterspel');
    expect(rv.suggestions[0].name).toBe('Counterspell');
    const atraxa = p.json.review.find((r: { name: string }) => r.name.startsWith('Atraxa'));
    expect(atraxa.suggestions[0].name).toBe("Atraxa, Praetors' Voice");
    const cards = [
      ...p.json.matched.map((m: { key: string; section: string; qty: number }) => ({ key: m.key, section: m.section, qty: m.qty })),
      { key: rv.suggestions[0].key, section: 'main', qty: 1 },
      { key: atraxa.suggestions[0].key, section: 'commander', qty: 1 },
    ];
    const c = await req('POST', '/api/import/commit', { name: 'Atraxa test', format: 'commander', cards }, admin);
    expect(c.status).toBe(200);
    deckId = c.json.id;
    expect(c.json.cards.length).toBe(6);
    expect(c.json.warnings).toBeGreaterThan(0);
  });

  it('edits, clones, exports and pins', async () => {
    const d = (await req('GET', `/api/decks/${deckId}`, undefined, admin)).json;
    const sol = d.cards.find((c: { def: string }) => d.defs[c.def].name === 'Sol Ring');
    const u = await req('PATCH', `/api/decks/${deckId}/cards/${sol.id}`, { qty: 2, tags: ['ramp'] }, admin);
    expect(u.json.cards.find((c: { id: number }) => c.id === sol.id).qty).toBe(2);
    expect((await req('PATCH', `/api/decks/${deckId}/cards/${sol.id}`, { qty: 1 }, bob)).status).toBe(403);
    const cl = await req('POST', `/api/decks/${deckId}/clone`, {}, bob);
    expect(cl.json.ownerName).toBe('bob');
    const ex = await req('GET', `/api/decks/${deckId}/export?fmt=arena`, undefined, bob);
    expect(ex.raw.body).toContain('Deck\n');
    await req('POST', `/api/decks/${deckId}/pin`, { pinned: true }, admin);
    const list = (await req('GET', '/api/decks', undefined, admin)).json;
    expect(list.find((x: { id: number }) => x.id === deckId).pinned).toBe(true);
  });

  it('tags: global tags follow a card into other decks, deck tags stay; rename, merge, scope, delete, recents', async () => {
    type D = { id: number; cards: { id: number; def: string; tags: string[] }[]; defs: Record<string, { name: string }>; tags: { name: string; color: string | null; scope: string }[]; recentTags: { name: string; scope: string }[] };
    const d: D = (await req('GET', `/api/decks/${deckId}`, undefined, admin)).json;
    const byName = (deck: D, n: string) => deck.cards.find((c) => deck.defs[c.def].name === n)!;
    const sol = byName(d, 'Sol Ring');
    const forest = byName(d, 'Forest');
    // global by default, deck-only on request
    let r = await req('POST', `/api/decks/${deckId}/cards/tags`, { cardIds: [sol.id, forest.id], add: ['#Mana  Rock', { name: 'Early', scope: 'deck' }] }, admin);
    expect(r.status).toBe(200);
    expect(byName(r.json, 'Sol Ring').tags).toEqual(expect.arrayContaining(['mana rock', 'early']));
    expect(r.json.tags).toEqual(expect.arrayContaining([{ name: 'early', color: null, scope: 'deck' }, { name: 'mana rock', color: null, scope: 'global' }]));
    expect(r.json.recentTags.slice(0, 2).map((t: { name: string }) => t.name).sort()).toEqual(['early', 'mana rock']);
    expect((await req('POST', `/api/decks/${deckId}/cards/tags`, { cardIds: [sol.id], add: ['x'] }, bob)).status).toBe(403);
    // a second deck with Sol Ring picks up its global tags (the earlier #ramp too), not the deck one
    const other = (await req('POST', '/api/decks', { name: 'Second' }, admin)).json;
    const o = (await req('POST', `/api/decks/${other.id}/cards`, { key: sol.def, section: 'main' }, admin)).json;
    expect(byName(o, 'Sol Ring').tags.sort()).toEqual(['mana rock', 'ramp']);
    const tagsIn = async (id: number) => byName((await req('GET', `/api/decks/${id}`, undefined, admin)).json, 'Sol Ring').tags.filter((t: string) => t !== 'ramp').sort();
    // removing a global tag in one deck removes it everywhere
    await req('POST', `/api/decks/${other.id}/cards/tags`, { cardIds: [byName(o, 'Sol Ring').id], remove: ['mana rock'] }, admin);
    expect(await tagsIn(deckId)).toEqual(['early']);
    expect(byName(r.json, 'Forest').tags).toContain('mana rock');
    // PATCH tags sets the card's tags exactly, keeping scopes
    r = await req('PATCH', `/api/decks/${deckId}/cards/${sol.id}`, { tags: ['early', 'mana rock'] }, admin);
    expect(await tagsIn(deckId)).toEqual(['early', 'mana rock']);
    // colours and order per scope; bad colours refused
    r = await req('PUT', `/api/decks/${deckId}/tags`, { tags: [{ name: 'mana rock', color: '#7db4ea', scope: 'global' }, { name: 'early', color: null, scope: 'deck' }] }, admin);
    expect(r.json.tags.find((t: { name: string }) => t.name === 'mana rock')).toEqual({ name: 'mana rock', color: '#7db4ea', scope: 'global' });
    expect((await req('PUT', `/api/decks/${deckId}/tags`, { tags: [{ name: 'x', color: 'red;', scope: 'deck' }] }, admin)).status).toBe(400);
    // renaming a global tag renames it in every deck
    r = await req('POST', `/api/decks/${deckId}/tags/rename`, { from: 'mana rock', to: 'rocks' }, admin);
    expect(byName(r.json, 'Forest').tags).toContain('rocks');
    expect(await tagsIn(other.id)).toEqual(['rocks']);
    // renaming a deck tag onto another deck tag merges
    await req('POST', `/api/decks/${deckId}/cards/tags`, { cardIds: [sol.id], add: [{ name: 'opener', scope: 'deck' }] }, admin);
    r = await req('POST', `/api/decks/${deckId}/tags/rename`, { from: 'opener', to: 'early' }, admin);
    expect(byName(r.json, 'Sol Ring').tags.filter((t: string) => t === 'early')).toHaveLength(1);
    // global → deck copies the tag onto the cards in every deck, so nothing disappears
    r = await req('POST', `/api/decks/${deckId}/tags/scope`, { name: 'rocks', scope: 'deck' }, admin);
    expect(r.json.tags.find((t: { name: string }) => t.name === 'rocks').scope).toBe('deck');
    expect(byName(r.json, 'Forest').tags).toContain('rocks');
    expect(await tagsIn(other.id)).toEqual(['rocks']);
    // deck → global
    r = await req('POST', `/api/decks/${deckId}/tags/scope`, { name: 'early', scope: 'global' }, admin);
    expect(r.json.tags.find((t: { name: string }) => t.name === 'early').scope).toBe('global');
    expect(await tagsIn(other.id)).toEqual(['early', 'rocks']);
    // someone else's clone keeps the owner's global tags as its own deck tags
    const cl = await req('POST', `/api/decks/${deckId}/clone`, {}, bob);
    expect(byName(cl.json, 'Sol Ring').tags).toContain('early');
    expect(cl.json.tags.find((t: { name: string }) => t.name === 'early').scope).toBe('deck');
    // export writes #!global and #deck
    const ex = await req('GET', `/api/decks/${deckId}/export?tags=1`, undefined, admin);
    expect(ex.raw.body).toMatch(/Sol Ring.* #!early/);
    expect(ex.raw.body).toMatch(/Forest.* #rocks/);
    // deleting a global tag removes it from all decks
    r = await req('POST', `/api/decks/${deckId}/tags/delete`, { name: 'early' }, admin);
    expect(r.json.tags.some((t: { name: string }) => t.name === 'early')).toBe(false);
    expect(await tagsIn(other.id)).toEqual(['rocks']);
    await req('DELETE', `/api/decks/${other.id}`, undefined, admin);
  });

  it('import: #!tags become global tags, #tags deck tags', async () => {
    const text = '1 Sol Ring (C21) 263 #ramp #!staple';
    const p = await req('POST', '/api/import/parse', { text }, admin);
    const m = p.json.matched[0];
    expect(m).toMatchObject({ tags: ['ramp'], globalTags: ['staple'] });
    const c = await req('POST', '/api/import/commit', { name: 'Imp', format: 'commander', cards: [{ key: m.key, section: 'main', qty: 1, tags: m.tags, globalTags: m.globalTags }] }, admin);
    expect(c.json.cards[0].tags).toEqual(expect.arrayContaining(['ramp', 'staple']));
    expect(c.json.tags.find((t: { name: string }) => t.name === 'staple').scope).toBe('global');
    await req('DELETE', `/api/decks/${c.json.id}`, undefined, admin);
  });

  it('custom cards appear in search as custom', async () => {
    await req('POST', '/api/custom-cards', { name: 'Grovetender Hydra', manaCost: '{3}{G}{G}', typeLine: 'Creature — Hydra', power: '0', toughness: '0', oracleText: 'Enters with five +1/+1 counters.' }, bob);
    const r = await req('GET', '/api/cards/search?q=grovetender', undefined, admin);
    expect(r.json.cards[0].custom).toBe(true);
  });

  it('makes a proxy PDF', async () => {
    const r = await req('POST', `/api/decks/${deckId}/proxy`, { paper: 'A4', cutLines: true, bleed: true, cardIds: [] }, admin);
    expect(r.status).toBe(400);
  });
});

function connect(cookie: string): Promise<{ ws: WebSocket; msgs: ServerMsg[]; wait: (pred: (m: ServerMsg) => boolean, ms?: number) => Promise<ServerMsg> }> {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(`ws://${base}/ws`, { headers: { cookie } });
    const msgs: ServerMsg[] = [];
    const waiters: { pred: (m: ServerMsg) => boolean; res: (m: ServerMsg) => void }[] = [];
    ws.on('message', (d) => {
      const m = JSON.parse(String(d)) as ServerMsg;
      msgs.push(m);
      for (const w of waiters.slice()) if (w.pred(m)) {
        waiters.splice(waiters.indexOf(w), 1);
        w.res(m);
      }
    });
    ws.on('open', () =>
      resolve({
        ws,
        msgs,
        wait: (pred, ms = 3000) =>
          new Promise((res, rej) => {
            const hit = msgs.find(pred);
            if (hit) return res(hit);
            const t = setTimeout(() => rej(new Error('timeout; got: ' + msgs.map((x) => x.t + (x.t === 'reject' ? ':' + (x as { reason: string }).reason : x.t === 'log' ? ':' + (x as { entries: { text: string }[] }).entries.map((e) => e.text).join('|') : '')).join(', '))), ms);
            waiters.push({ pred, res: (m) => (clearTimeout(t), res(m)) });
          }),
      }),
    );
    ws.on('error', reject);
  });
}

describe('lobby and game over websockets', () => {
  it('runs a 2 player game with redaction, undo and suspend/load', async () => {
    const deckA = (await req('GET', '/api/decks', undefined, admin)).json[0].id;
    const made = await req('POST', '/api/lobbies', { settings: { name: 'Test', format: 'commander' } }, admin);
    expect(made.status).toBe(200);
    const L = `/api/lobbies/${made.json.lobby.id}`;
    expect((await req('POST', `${L}/join`, {}, bob)).status).toBe(200);
    // the newcomer sits in the first open seat
    expect((await req('GET', L, undefined, bob)).json.slots[1]).toMatchObject({ kind: 'human', name: 'bob' });
    await req('PUT', `${L}/seats/0/deck`, { deckId: deckA }, admin);
    await req('PUT', `${L}/seats/1/deck`, { deckId: deckA }, bob);
    expect((await req('POST', `${L}/start`, {}, admin)).status).toBe(409);
    await req('PUT', `${L}/ready`, { ready: true }, admin);
    await req('PUT', `${L}/ready`, { ready: true }, bob);
    const a = await connect(admin);
    const b = await connect(bob);
    const st = await req('POST', `${L}/start`, {}, admin);
    expect(st.status).toBe(200);
    expect(st.json.kind).toBe('sandbox');
    const gid = st.json.id;
    a.ws.send(JSON.stringify({ t: 'watch', game: gid }));
    b.ws.send(JSON.stringify({ t: 'watch', game: gid }));
    const ga = (await a.wait((m) => m.t === 'game')) as Extract<ServerMsg, { t: 'game' }>;
    const gb = (await b.wait((m) => m.t === 'game')) as Extract<ServerMsg, { t: 'game' }>;
    const seatA = ga.view.seats.findIndex((s) => s.name === 'alice');
    const seatB = 1 - seatA;
    const aHand = ga.view.seats[seatA].zones.hand;
    expect(aHand).toHaveLength(7);
    expect(ga.view.cards[aHand[0]].hidden).toBeFalsy();
    expect(gb.view.cards[aHand[0]].hidden).toBe(true);
    expect(ga.view.seats[seatA].zones.command).toHaveLength(1);

    // Draw: private names for alice, count for bob.
    a.ws.send(JSON.stringify({ t: 'intent', id: 1, intent: { type: 'draw', seat: seatA, n: 1 } }));
    await a.wait((m) => m.t === 'ack' && m.id === 1);
    const la = (await a.wait((m) => m.t === 'log' && m.entries.some((e) => e.text.startsWith('drew')))) as Extract<ServerMsg, { t: 'log' }>;
    const lb = (await b.wait((m) => m.t === 'log' && m.entries.some((e) => e.text.startsWith('drew')))) as Extract<ServerMsg, { t: 'log' }>;
    expect(la.entries.find((e) => e.text.startsWith('drew'))!.priv).toBe(true);
    expect(lb.entries.find((e) => e.text.startsWith('drew'))!.text).toBe('drew 1 card');

    // Life merges into one line.
    for (let i = 0; i < 3; i++) a.ws.send(JSON.stringify({ t: 'intent', id: 10 + i, intent: { type: 'life', seat: seatB, delta: -1 } }));
    const upd = (await b.wait((m) => m.t === 'logUpdate' && /40 → 37/.test(m.entry.text))) as Extract<ServerMsg, { t: 'logUpdate' }>;
    expect(upd.entry.text).toMatch(/life 40 → 37/);

    // Bob acts, then alice's undo needs the host... alice is the host, so it applies directly.
    b.ws.send(JSON.stringify({ t: 'intent', id: 20, intent: { type: 'playerCounter', seat: seatB, key: 'poison', delta: 1 } }));
    await b.wait((m) => m.t === 'ack' && m.id === 20);
    a.ws.send(JSON.stringify({ t: 'undo' }));
    await a.wait((m) => m.t === 'log' && m.entries.some((e) => e.text.startsWith('rewound')));

    // Bob's undo of his own last action: nothing after, so instant.
    b.ws.send(JSON.stringify({ t: 'intent', id: 21, intent: { type: 'tap', iids: [] } }));
    await b.wait((m) => m.t === 'reject' && m.id === 21);

    // Suspend and load back.
    a.ws.send(JSON.stringify({ t: 'host', op: 'suspend' }));
    await b.wait((m) => m.t === 'gameClosed');
    const sus = (await req('GET', '/api/games/suspended', undefined, bob)).json;
    expect(sus).toHaveLength(1);
    const load = await req('POST', `/api/games/${sus[0].id}/load`, {}, bob);
    expect(load.status).toBe(200);
    a.msgs.length = 0;
    a.ws.send(JSON.stringify({ t: 'watch', game: gid }));
    const again = (await a.wait((m) => m.t === 'game')) as Extract<ServerMsg, { t: 'game' }>;
    expect(again.view.seats[seatA].zones.hand).toHaveLength(8);

    // End with a winner → match history.
    a.ws.send(JSON.stringify({ t: 'host', op: 'end', winner: seatB }));
    await a.wait((m) => m.t === 'log' && m.entries.some((e) => /winner/.test(e.text)));
    const matches = (await req('GET', '/api/matches', undefined, admin)).json;
    expect(matches[0].winner).toBe('bob');
    expect(matches[0].players.find((p: { name: string }) => p.name === 'bob').result).toBe('Winner');
    const log = (await req('GET', `/api/matches/${matches[0].id}/log`, undefined, admin)).json;
    expect(log.some((l: { text: string }) => /drew (?!1 card)/.test(l.text))).toBe(true);
    a.ws.send(JSON.stringify({ t: 'host', op: 'close' }));
    await a.wait((m) => m.t === 'gameClosed' && m.reason === 'closed');
    a.ws.close();
    b.ws.close();
    expect(ctxRef.hub.rooms.size).toBe(0);
  });

  it('lets one player hold several seats (solo), refuses AIs in a sandbox game, and keeps seat races fair', async () => {
    const deckA = (await req('GET', '/api/decks', undefined, admin)).json[0].id;
    const made = await req('POST', '/api/lobbies', { settings: { name: 'Solo', format: 'commander', maxPlayers: 3 } }, admin);
    const L = `/api/lobbies/${made.json.lobby.id}`;
    // AI players need a rules enforced game
    expect((await req('POST', `${L}/seats/1`, { ai: 'Normal' }, admin)).status).toBe(409);
    // sit in a second seat yourself
    expect((await req('POST', `${L}/seats/1`, { userId: (await req('GET', '/api/me', undefined, admin)).json.id }, admin)).status).toBe(200);
    // somebody else cannot take a seat that is taken
    await req('POST', `${L}/join`, {}, bob);
    const v = (await req('GET', L, undefined, bob)).json;
    expect(v.slots[2]).toMatchObject({ kind: 'human', name: 'bob' });
    expect((await req('POST', `${L}/seats/1`, { userId: v.slots[2].userId }, bob)).status).toBe(409);
    await req('POST', `${L}/leave`, {}, bob);
    await req('PUT', `${L}/seats/0/deck`, { deckId: deckA }, admin);
    await req('PUT', `${L}/seats/1/deck`, { deckId: deckA }, admin);
    await req('PUT', `${L}/ready`, { ready: true }, admin);
    const st = await req('POST', `${L}/start`, {}, admin);
    expect(st.status).toBe(200);
    const a = await connect(admin);
    a.ws.send(JSON.stringify({ t: 'watch', game: st.json.id }));
    const g = (await a.wait((m) => m.t === 'game')) as Extract<ServerMsg, { t: 'game' }>;
    expect(g.mySeats).toEqual([0, 1]);
    // act as the other seat
    a.msgs.length = 0;
    a.ws.send(JSON.stringify({ t: 'seat', seat: 1 }));
    const g2 = (await a.wait((m) => m.t === 'game')) as Extract<ServerMsg, { t: 'game' }>;
    expect(g2.view.cards[g2.view.seats[1].zones.hand[0]].hidden).toBeFalsy();
    // a rematch keeps the room's id, so the room is found, listed once and really removed when closed
    a.ws.send(JSON.stringify({ t: 'host', op: 'end', winner: 0 }));
    await a.wait((m) => m.t === 'log' && m.entries.some((e) => /winner/.test(e.text)));
    a.msgs.length = 0;
    a.ws.send(JSON.stringify({ t: 'host', op: 'rematch' }));
    const g3 = (await a.wait((m) => m.t === 'game')) as Extract<ServerMsg, { t: 'game' }>;
    expect(g3.view.stage).not.toBe('ended');
    expect(g3.view.id).toBe(st.json.id);
    expect([...ctxRef.hub.rooms.keys()]).toEqual([st.json.id]);
    a.ws.send(JSON.stringify({ t: 'host', op: 'close' }));
    await a.wait((m) => m.t === 'gameClosed');
    expect(ctxRef.hub.rooms.size).toBe(0);
    a.ws.close();
  });

  it('runs several lobbies at once; switching away from rules asks before removing AIs', async () => {
    const one = await req('POST', '/api/lobbies', { settings: { name: 'One', type: 'rules' } }, admin);
    const two = await req('POST', '/api/lobbies', { settings: { name: 'Two' } }, bob);
    expect(one.status).toBe(200);
    expect(two.status).toBe(200);
    expect((await req('GET', '/api/lobbies', undefined, admin)).json.map((l: { name: string }) => l.name).sort()).toEqual(['One', 'Two']);
    const L = `/api/lobbies/${one.json.lobby.id}`;
    expect((await req('POST', `${L}/seats/1`, { ai: 'Hard' }, admin)).status).toBe(200);
    expect((await req('PUT', `${L}/settings`, { type: 'sandbox' }, admin)).status).toBe(409);
    const ok = await req('PUT', `${L}/settings`, { type: 'sandbox', removeAi: true }, admin);
    expect(ok.status).toBe(200);
    expect(ok.json.lobby.slots[1]).toEqual({ kind: 'open' });
    // only the host changes settings
    await req('POST', `${L}/join`, {}, bob);
    expect((await req('PUT', `${L}/settings`, { name: 'Mine now' }, bob)).status).toBe(409);
    await req('POST', `${L}/leave`, {}, bob);
    await req('POST', `${L}/leave`, {}, admin);
    await req('POST', `/api/lobbies/${two.json.lobby.id}/leave`, {}, bob);
    expect((await req('GET', '/api/lobbies', undefined, admin)).json).toEqual([]);
  });
});

describe('backups', () => {
  it('backs up and lists', async () => {
    const r = await req('POST', '/api/admin/backups', {}, admin);
    expect(r.json.reason).toMatch(/Manual backup by alice/);
    expect((await req('GET', '/api/admin/backups', undefined, admin)).json.length).toBeGreaterThan(0);
  });
});

describe('after a game', () => {
  it('back to lobby re-opens it with the seats and decks; main menu leaves it, and it closes when empty', async () => {
    const deckA = (await req('GET', '/api/decks', undefined, admin)).json[0].id;
    const made = await req('POST', '/api/lobbies', { settings: { name: 'Again', format: 'commander' } }, admin);
    const L = `/api/lobbies/${made.json.lobby.id}`;
    await req('POST', `${L}/join`, {}, bob);
    await req('PUT', `${L}/seats/0/deck`, { deckId: deckA }, admin);
    await req('PUT', `${L}/seats/1/deck`, { deckId: deckA }, bob);
    await req('PUT', `${L}/ready`, { ready: true }, admin);
    await req('PUT', `${L}/ready`, { ready: true }, bob);
    const st = await req('POST', `${L}/start`, {}, admin);
    expect(st.status).toBe(200);
    const gid = st.json.id;
    // bob is back first: the lobby re-opens with bob in his seat and admin's seat kept free
    const back = await req('POST', `/api/games/${gid}/back-to-lobby`, {}, bob);
    expect(back.status).toBe(200);
    const R = `/api/lobbies/${back.json.lobbyId}`;
    let lv = (await req('GET', R, undefined, bob)).json;
    expect(lv.name ?? lv.settings?.name).toBeTruthy();
    expect(lv.slots[0]).toMatchObject({ kind: 'open' });
    expect(lv.slots[1]).toMatchObject({ kind: 'human', name: 'bob', deckId: deckA });
    // admin comes back to the same lobby and the same seat
    const back2 = await req('POST', `/api/games/${gid}/back-to-lobby`, {}, admin);
    expect(back2.json.lobbyId).toBe(back.json.lobbyId);
    lv = (await req('GET', R, undefined, admin)).json;
    expect(lv.slots[0]).toMatchObject({ kind: 'human', name: 'alice', deckId: deckA });
    // main menu: each leaves; the lobby closes with the last one
    await req('POST', `/api/games/${gid}/leave-lobby`, {}, admin);
    expect((await req('GET', '/api/lobbies', undefined, bob)).json.some((x: { id: string }) => x.id === back.json.lobbyId)).toBe(true);
    await req('POST', `/api/games/${gid}/leave-lobby`, {}, bob);
    expect((await req('GET', '/api/lobbies', undefined, bob)).json.some((x: { id: string }) => x.id === back.json.lobbyId)).toBe(false);
  });
});
