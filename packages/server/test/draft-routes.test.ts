import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import type { FastifyInstance } from 'fastify';

const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'aetherhub-draft-'));
process.env.DATA_DIR = dir;
process.env.NO_AUTO_DOWNLOAD = '1';
// the fixture plus a made-up 75-card expansion, so a set can be drafted
const seed = path.join(dir, 'cards.jsonl');
const RARITY = (i: number) => (i < 45 ? 'common' : i < 63 ? 'uncommon' : i < 72 ? 'rare' : 'mythic');
const synthetic = Array.from({ length: 75 }, (_, i) => {
  const col = 'WUBRG'[i % 5];
  return JSON.stringify({
    object: 'card', id: `00000000-0000-4000-8000-${String(i).padStart(12, '0')}`, oracle_id: `10000000-0000-4000-8000-${String(i).padStart(12, '0')}`,
    name: `Testling ${i}`, lang: 'en', released_at: '2020-01-01', layout: 'normal', mana_cost: `{${1 + (i % 4)}}{${col}}`, cmc: 2 + (i % 4),
    type_line: i % 4 === 3 ? 'Instant' : 'Creature — Test', oracle_text: i % 4 === 3 ? 'Testling deals 3 damage to any target.' : '', power: '2', toughness: '2',
    colors: [col], color_identity: [col], legalities: {}, games: ['paper'], set: 'tst', set_name: 'Test Set', set_type: 'expansion',
    collector_number: String(i + 1), rarity: RARITY(i), image_status: 'missing', digital: false, promo: false, prices: {},
  });
});
const basics = ['Plains', 'Island', 'Swamp', 'Mountain', 'Forest'].map((n, i) =>
  JSON.stringify({ object: 'card', id: `00000000-0000-4000-9000-${String(i).padStart(12, '0')}`, oracle_id: `10000000-0000-4000-9000-${String(i).padStart(12, '0')}`, name: n, lang: 'en', released_at: '2020-01-01', layout: 'normal', mana_cost: '', cmc: 0, type_line: `Basic Land — ${n}`, oracle_text: '', colors: [], color_identity: ['WUBRG'[i]], legalities: {}, games: ['paper'], set: 'tst', set_name: 'Test Set', set_type: 'expansion', collector_number: String(200 + i), rarity: 'common', image_status: 'missing', digital: false, promo: false, prices: {} }),
);
fs.writeFileSync(seed, fs.readFileSync(path.resolve(__dirname, '../../../fixtures/cards-fixture.jsonl'), 'utf8').trimEnd() + '\n' + [...synthetic, ...basics].join('\n') + '\n');
process.env.SEED_CARDS = seed;
process.env.CLIENT_DIR = '/nonexistent';
process.env.IMAGE_OFFLINE = '1';

let f: FastifyInstance;
beforeAll(async () => {
  const { buildApp } = await import('../src/app');
  const built = await buildApp();
  f = built.f;
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

describe('draft and sealed events from a lobby', () => {
  let cookie = '';
  let me = 0;
  let setCode = '';

  it('lists sets (with icons when known)', async () => {
    cookie = (await req('POST', '/api/setup/admin', { username: 'alice', password: 'hunter22' })).cookie!;
    me = (await req('GET', '/api/me', undefined, cookie)).json.id;
    const sets = await req('GET', '/api/draft/sets', undefined, cookie);
    expect(sets.status).toBe(200);
    setCode = sets.json.find((x: { code: string }) => x.code === 'tst')?.code;
    expect(setCode).toBe('tst');
    expect(sets.json[0]).toHaveProperty('booster');
  });

  it('drafts in a pod of the lobby seats (alice twice and an AI), builds a deck, then starts a game from the event', async () => {
    if (!setCode) return;
    const made = await req('POST', '/api/lobbies', { settings: { name: 'Draft night', type: 'draft', sets: [setCode], maxPlayers: 3, pickTimer: false } }, cookie);
    expect(made.status, JSON.stringify(made.json)).toBe(200);
    const L = `/api/lobbies/${made.json.lobby.id}`;
    await req('POST', `${L}/seats/1`, { userId: me }, cookie);
    expect((await req('POST', `${L}/seats/2`, { ai: 'Normal' }, cookie)).status).toBe(200);
    await req('PUT', `${L}/ready`, { ready: true }, cookie);
    const st0 = await req('POST', `${L}/start`, {}, cookie);
    expect(st0.status, JSON.stringify(st0.json)).toBe(200);
    expect(st0.json.kind).toBe('draft');
    const id = st0.json.id;
    expect((await req('GET', L, undefined, cookie)).json.phase).toBe('drafting');
    let st = (await req('GET', `/api/draft/games/${id}`, undefined, cookie)).json;
    expect(st.seats).toHaveLength(3);
    expect(st.mySeats).toEqual([0, 1]);
    for (let n = 0; n < 200 && st.phase === 'drafting'; n++) {
      // pick for whichever of alice's seats still has to
      const seat = st.mySeats.find((i: number) => !st.seats[i].picked) ?? st.seat;
      const view = (await req('GET', `/api/draft/games/${id}?seat=${seat}`, undefined, cookie)).json;
      if (!view.pack.length) break;
      st = (await req('POST', `/api/draft/games/${id}/pick`, { key: view.pack[0].key, seat }, cookie)).json;
    }
    expect(st.phase).toBe('building');
    expect((await req('GET', L, undefined, cookie)).json.phase).toBe('building');
    // build both of alice's decks from a suggestion, with basics
    for (const seat of [0, 1]) {
      const pool = (await req('GET', `/api/draft/games/${id}?seat=${seat}`, undefined, cookie)).json.pool;
      expect(pool.length).toBeGreaterThan(30);
      const sug = (await req('GET', `/api/draft/games/${id}/suggest?seat=${seat}`, undefined, cookie)).json;
      const deck = await req('POST', `/api/draft/games/${id}/deck`, { seat, name: `Drafted ${seat}`, main: sug.main, basics: sug.basics }, cookie);
      expect(deck.status, JSON.stringify(deck.json)).toBe(200);
      // cannot take more copies than were drafted
      const cheat = await req('POST', `/api/draft/games/${id}/deck`, { seat, name: 'Cheat', main: Array(30).fill(pool[0].key) }, cookie);
      expect(cheat.status).toBe(400);
    }
    const decks = (await req('GET', '/api/decks', undefined, cookie)).json;
    const drafted = decks.find((d: any) => d.name === 'Drafted 0');
    expect(drafted.format).toBe('limited');
    // every human built a deck: the event lobby lists the decks (the AI's too) and games can start
    const lobby = (await req('GET', L, undefined, cookie)).json;
    expect(lobby.phase).toBe('event');
    expect(lobby.eventDecks).toHaveLength(3);
    expect(lobby.eventDecks[2].deckName).toMatch(/AI .*deck/);
    // an AI drafter cannot play a sandbox game
    expect((await req('POST', `${L}/event-game`, { seats: [0, 2], type: 'sandbox' }, cookie)).status).toBe(409);
    const game = await req('POST', `${L}/event-game`, { seats: [0, 1], type: 'sandbox' }, cookie);
    expect(game.status, JSON.stringify(game.json)).toBe(200);
    expect(game.json.kind).toBe('sandbox');
    const after = (await req('GET', L, undefined, cookie)).json;
    expect(after.eventGames.map((g: any) => g.id)).toContain(game.json.id);
  });

  it('sealed opens six boosters for each player and goes straight to deck building', async () => {
    if (!setCode) return;
    const made = await req('POST', '/api/lobbies', { settings: { name: 'Sealed', type: 'sealed', sets: [setCode], maxPlayers: 2 } }, cookie);
    const L = `/api/lobbies/${made.json.lobby.id}`;
    await req('PUT', `${L}/ready`, { ready: true }, cookie);
    const st0 = await req('POST', `${L}/start`, {}, cookie);
    expect(st0.status, JSON.stringify(st0.json)).toBe(200);
    const st = (await req('GET', `/api/draft/games/${st0.json.id}`, undefined, cookie)).json;
    expect(st.phase).toBe('building');
    expect(st.pool.length).toBeGreaterThanOrEqual(84);
  });
});
