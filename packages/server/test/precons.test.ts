import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import type { FastifyInstance } from 'fastify';

const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'aetherhub-precons-'));
const PRECON = -12345;
const BURN = -23456;
const preconsFile = path.join(dir, 'precons.json');
fs.writeFileSync(
  preconsFile,
  JSON.stringify({
    source: 'test',
    decks: [
      {
        id: PRECON,
        file: 'Gisela_TEST',
        name: 'Angelic Test',
        code: 'TST',
        type: 'Commander Deck',
        date: '2024-01-02',
        format: 'commander',
        cards: [
          ['commander', 1, 'Gisela, the Broken Blade', 'INR', '24'],
          ['main', 1, 'Sol Ring', 'ECC', '57'],
          ['main', 30, 'Forest', 'BLB', '280'],
          ['main', 1, 'No Such Card Anywhere', 'XXX', '1'],
        ],
      },
      {
        id: BURN,
        file: 'Burn_TEST',
        name: 'Burn Test',
        code: 'TST',
        type: 'Theme Deck',
        date: '2024-01-01',
        format: 'freeform',
        cards: [
          ['main', 4, 'Lightning Bolt', 'SLD', '901'],
          ['main', 56, 'Mountain', '2ED', '298'],
        ],
      },
    ],
  }),
);
process.env.DATA_DIR = dir;
process.env.NO_AUTO_DOWNLOAD = '1';
process.env.SEED_CARDS = path.resolve(__dirname, '../../../fixtures/cards-fixture.jsonl');
process.env.CLIENT_DIR = '/nonexistent';
process.env.IMAGE_OFFLINE = '1';
process.env.PRECONS_FILE = preconsFile;

let f: FastifyInstance;
let ctx: Awaited<ReturnType<typeof import('../src/app').buildApp>>['ctx'];
let admin = '';
let bob = '';

async function req(method: string, url: string, body?: unknown, cookie?: string) {
  const r = await f.inject({ method: method as 'GET', url, payload: body as object, headers: cookie ? { cookie } : {} });
  return { status: r.statusCode, json: r.body ? JSON.parse(r.body) : null, cookie: (r.headers['set-cookie'] as string | undefined)?.split(';')[0] };
}

beforeAll(async () => {
  const { buildApp } = await import('../src/app');
  const built = await buildApp();
  f = built.f;
  ctx = built.ctx;
  await built.ctx.carddata.importFile(process.env.SEED_CARDS!);
  admin = (await req('POST', '/api/setup/admin', { username: 'alice', password: 'hunter22' })).cookie!;
  const pw = (await req('POST', '/api/admin/users', { username: 'bob' }, admin)).json.tempPassword;
  bob = (await req('POST', '/api/auth/login', { username: 'bob', password: pw })).cookie!;
});

afterAll(async () => {
  await f?.close();
  fs.rmSync(dir, { recursive: true, force: true });
});

describe('preconstructed decks', () => {
  it('are listed only after an admin switches them on, and never among the users’ decks', async () => {
    expect((await req('GET', '/api/precons', undefined, bob)).json).toEqual([]);
    expect((await req('GET', '/api/admin/precons', undefined, admin)).json).toEqual({ enabled: false, available: 2 });
    expect((await req('PUT', '/api/admin/precons', { enabled: true }, bob)).status).toBe(403);
    expect((await req('PUT', '/api/admin/precons', { enabled: true }, admin)).status).toBe(200);
    const list = (await req('GET', '/api/precons', undefined, bob)).json;
    expect(list).toHaveLength(2);
    expect(list[0]).toMatchObject({ id: PRECON, name: 'Angelic Test', format: 'commander', cardCount: 32, colors: ['W'], precon: { type: 'Commander Deck', set: 'TST', missing: ['No Such Card Anywhere'], formats: ['commander'] } });
    expect((await req('GET', '/api/decks', undefined, bob)).json.some((d: { id: number }) => d.id === PRECON)).toBe(false);
  });

  it('list every format they are legal in, under the narrowest one when they have no format of their own', async () => {
    const burn = (await req('GET', '/api/precons', undefined, bob)).json.find((d: { id: number }) => d.id === BURN);
    expect(burn.format).toBe('modern');
    expect(burn.precon.formats).toEqual(['modern', 'legacy', 'vintage', 'pauper']);
    expect(ctx.decks.precons.contents(BURN)!.format).toBe('modern');
  });

  it('open read-only, cannot be pinned, and copy into the viewer’s decks', async () => {
    const d = (await req('GET', `/api/decks/${PRECON}`, undefined, bob)).json;
    expect(d.precon.set).toBe('TST');
    expect(d.cards.find((c: { section: string }) => c.section === 'commander')).toBeTruthy();
    expect((await req('PUT', `/api/decks/${PRECON}`, { name: 'Mine now' }, admin)).status).toBe(403);
    expect((await req('POST', `/api/decks/${PRECON}/pin`, { pinned: true }, bob)).status).toBe(400);
    const copy = (await req('POST', `/api/decks/${PRECON}/clone`, {}, bob)).json;
    expect(copy.id).toBeGreaterThan(0);
    expect(copy).toMatchObject({ ownerName: 'bob', name: 'Angelic Test', format: 'commander', cardCount: 32 });
    expect(copy.precon).toBeUndefined();
  });

  it('can be chosen for a game: the lobby finds it and the game gets its cards', () => {
    expect(ctx.hub.deckRef(PRECON)).toEqual({ id: PRECON, name: 'Angelic Test', owner: 'Pre-constructed' });
    const g = ctx.decks.forGame(PRECON)!;
    expect(g.cards.find((c) => c.section === 'commander' && g.defs[c.def].name === 'Gisela, the Broken Blade')).toBeTruthy();
    expect(g.cards.filter((c) => c.section === 'main').reduce((n, c) => n + c.qty, 0)).toBe(31);
    expect(() => ctx.hub.deckRef(-999)).toThrow('Deck not found');
  });

  it('switched off: hidden from the list, but still found by id (games and lobbies that use one keep working)', async () => {
    await req('PUT', '/api/admin/precons', { enabled: false }, admin);
    expect((await req('GET', '/api/precons', undefined, bob)).json).toEqual([]);
    expect((await req('GET', `/api/decks/${PRECON}`, undefined, bob)).status).toBe(200);
  });
});
