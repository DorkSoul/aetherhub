import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import type { FastifyInstance } from 'fastify';
import type { AdminLogLine } from '@aetherhub/shared';

const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'aetherhub-activity-'));
process.env.DATA_DIR = dir;
process.env.NO_AUTO_DOWNLOAD = '1';
process.env.SEED_CARDS = path.resolve(__dirname, '../../../fixtures/cards-fixture.jsonl');
process.env.CLIENT_DIR = '/nonexistent';
process.env.IMAGE_OFFLINE = '1';

let f: FastifyInstance;
let admin = '';
let bob = '';
let bobPw = '';

async function start() {
  const { buildApp } = await import('../src/app');
  f = (await buildApp()).f;
}
async function req(method: string, url: string, body?: unknown, cookie?: string) {
  const r = await f.inject({ method: method as 'GET', url, payload: body as object, headers: cookie ? { cookie } : {} });
  return { status: r.statusCode, json: r.body ? JSON.parse(r.body) : null, cookie: (r.headers['set-cookie'] as string | undefined)?.split(';')[0] };
}
async function log(query = ''): Promise<AdminLogLine[]> {
  return (await req('GET', '/api/admin/log' + query, undefined, admin)).json.entries;
}

beforeAll(async () => {
  await start();
  admin = (await req('POST', '/api/setup/admin', { username: 'alice', password: 'hunter22' })).cookie!;
});

afterAll(async () => {
  await f?.close();
  fs.rmSync(dir, { recursive: true, force: true });
});

describe('the activity log', () => {
  it('records accounts and sign-ins, never passwords', async () => {
    bobPw = (await req('POST', '/api/admin/users', { username: 'bob' }, admin)).json.tempPassword;
    expect((await req('POST', '/api/auth/login', { username: 'bob', password: 'wrong-one' })).status).toBe(401);
    bob = (await req('POST', '/api/auth/login', { username: 'bob', password: bobPw })).cookie!;
    const entries = await log();
    const line = (msg: string) => entries.find((e) => e.msg === msg);
    expect(line('Created the first admin account “alice”')).toMatchObject({ cat: 'auth', user: 'alice' });
    expect(line('Created user “bob”')).toMatchObject({ cat: 'users', user: 'alice', level: 'info' });
    expect(line('Failed sign-in: wrong username or password')).toMatchObject({ cat: 'auth', user: 'bob', level: 'warn' });
    expect(line('Signed in')).toMatchObject({ user: 'bob', level: 'info' });
    // tried from a browser already signed in as alice: the entry names the account being tried
    await req('POST', '/api/auth/login', { username: 'ghost', password: 'nope' }, admin);
    expect((await log('?user=ghost'))[0]).toMatchObject({ msg: 'Failed sign-in: wrong username or password', level: 'warn' });
    expect(entries[0].ip).toBeTruthy();
    const all = JSON.stringify(entries);
    expect(all).not.toContain(bobPw);
    expect(all).not.toContain('wrong-one');
    expect(all).not.toContain('hunter22');
  });

  it('names the decks it records, even one that was deleted, and records refused requests as warnings', async () => {
    const id = (await req('POST', '/api/decks', { name: 'Elf Ball', format: 'freeform' }, bob)).json.id;
    await req('PUT', `/api/decks/${id}`, { name: 'Elf Storm' }, bob);
    expect((await req('DELETE', `/api/decks/${id}`, undefined, (await req('POST', '/api/admin/users', { username: 'cass' }, admin)).cookie)).status).toBe(401);
    await req('DELETE', `/api/decks/${id}`, undefined, bob);
    const msgs = (await log('?cat=decks')).map((e) => e.msg);
    expect(msgs.slice(0, 4)).toEqual(['Deleted deck “Elf Storm”', 'Deleted deck “Elf Storm” (failed, 401: Not signed in)', 'Renamed deck “Elf Ball” to “Elf Storm”', 'Created deck “Elf Ball”']);
    await req('PUT', '/api/admin/settings', { serverName: 'Nope' }, bob);
    const refused = (await log('?level=warn')).find((e) => e.msg.startsWith('Changed server settings'));
    expect(refused).toMatchObject({ user: 'bob', level: 'warn' });
    expect(refused!.msg).toContain('failed, 403');
  });

  it('says whether a deleted user\'s decks were deleted or transferred, and caps an over-long attempted username', async () => {
    const mk = async (u: string) => (await req('POST', '/api/admin/users', { username: u }, admin)).json.id;
    await req('DELETE', `/api/admin/users/${await mk('dora')}?mode=delete`, undefined, admin);
    await req('DELETE', `/api/admin/users/${await mk('tim')}`, undefined, admin);
    const msgs = (await log('?cat=users')).map((e) => e.msg);
    expect(msgs).toContain('Deleted user “dora” (their decks and custom cards were deleted)');
    expect(msgs).toContain('Deleted user “tim” (their decks and custom cards were transferred)');
    await req('POST', '/api/auth/login', { username: 'x'.repeat(5000), password: 'nope' });
    expect((await log('?cat=auth'))[0].user).toHaveLength(60);
  });

  it('filters by user and text, pages with before / after, and is for admins only', async () => {
    const bobs = await log('?user=bob');
    expect(bobs.length).toBeGreaterThan(0);
    expect(bobs.every((e) => e.user === 'bob')).toBe(true);
    expect((await log('?q=' + encodeURIComponent('Elf Storm'))).every((e) => e.msg.includes('Elf Storm'))).toBe(true);
    const all = await log();
    const older = await log(`?before=${all[2].id}&limit=2`);
    expect(older.map((e) => e.id)).toEqual([all[3].id, all[4].id]);
    expect((await log(`?after=${all[1].id}`)).map((e) => e.id)).toEqual([all[0].id]);
    expect((await req('GET', '/api/admin/log', undefined, bob)).status).toBe(403);
  });

  it('survives a restart, and records the server stopping and starting', async () => {
    const before = (await log())[0].id;
    await f.close();
    await start();
    admin = (await req('POST', '/api/auth/login', { username: 'alice', password: 'hunter22' })).cookie!;
    const entries = await log();
    expect(entries.some((e) => e.id === before)).toBe(true);
  });
});
