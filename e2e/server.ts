import { spawn, type ChildProcess } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { expect, type Browser, type BrowserContext, type Page } from '@playwright/test';

const ROOT = path.resolve(path.dirname(new URL(import.meta.url).pathname), '..');

export class TestServer {
  proc: ChildProcess | null = null;
  dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'aetherhub-e2e-'));
  constructor(public port: number) {}
  get url() {
    return `http://127.0.0.1:${this.port}/`;
  }
  async start() {
    this.proc = spawn(process.execPath, [path.join(ROOT, 'packages/server/dist/index.js')], {
      env: { ...process.env, PORT: String(this.port), DATA_DIR: this.dataDir, NO_AUTO_DOWNLOAD: '1', IMAGE_OFFLINE: '1', SEED_CARDS: path.join(ROOT, 'fixtures/cards-fixture.jsonl'), LOG_LEVEL: 'warn', RULES_CARDS: path.join(ROOT, 'cards') },
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    this.proc.stderr?.on('data', (d) => process.stderr.write(d));
    for (let i = 0; i < 100; i++) {
      try {
        const r = await fetch(this.url + 'api/health');
        if (r.ok) {
          // wait for the fixture import
          for (let j = 0; j < 50; j++) {
            const st = await fetch(this.url + 'api/setup/state').then((x) => x.json());
            if (st) break;
            await new Promise((res) => setTimeout(res, 100));
          }
          await new Promise((res) => setTimeout(res, 400));
          return;
        }
      } catch {
        /* not up yet */
      }
      await new Promise((res) => setTimeout(res, 150));
    }
    throw new Error('server did not start');
  }
  async stop() {
    if (!this.proc) return;
    const p = this.proc;
    this.proc = null;
    await new Promise<void>((res) => {
      p.once('exit', () => res());
      p.kill('SIGTERM');
      setTimeout(() => {
        p.kill('SIGKILL');
        res();
      }, 5000);
    });
  }
  cleanup() {
    fs.rmSync(this.dataDir, { recursive: true, force: true });
  }
}

export interface Player {
  ctx: BrowserContext;
  page: Page;
  name: string;
  errors: string[];
}

export async function newPlayer(browser: Browser, name: string): Promise<Player> {
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
  const page = await ctx.newPage();
  const errors: string[] = [];
  page.on('pageerror', (e) => errors.push(e.message));
  return { ctx, page, name, errors };
}

export async function firstRun(p: Player, srv: TestServer) {
  await p.page.goto(srv.url);
  await p.page.getByLabel('Username').fill(p.name);
  await p.page.getByLabel('Password', { exact: true }).fill('hunter22');
  await p.page.getByLabel('Confirm password').fill('hunter22');
  await p.page.getByRole('button', { name: /Create admin/ }).click();
  await expect(p.page.getByText('What are we playing?')).toBeVisible();
}

/** Creates users via the admin API and signs each player in. */
export async function createAndLogin(admin: Player, srv: TestServer, players: Player[]) {
  for (const p of players) {
    const r = await admin.page.request.post(srv.url + 'api/admin/users', { data: { username: p.name } });
    const { tempPassword } = await r.json();
    await p.page.goto(srv.url);
    await p.page.getByLabel('Username').fill(p.name);
    await p.page.getByLabel('Password', { exact: true }).fill(tempPassword);
    await p.page.getByRole('button', { name: 'Sign in' }).click();
    await expect(p.page.getByText('What are we playing?')).toBeVisible();
  }
}

const LIST = `Commander
1 Atraxa, Praetors' Voice

Main
1 Sol Ring
1 Arcane Signet
1 Swords to Plowshares
1 Counterspell
1 Cultivate
1 Rhystic Study
1 Evolution Sage
1 Llanowar Elves
1 Serra Angel
12 Forest
12 Island
10 Plains

Sideboard
1 Lightning Bolt
1 Shivan Dragon

Tokens
1 Treasure

Planar
1 Tazeem
1 Naya
1 The Aether Flues`;

/** Imports a deck through the API as this player and returns its id. */
export async function makeDeck(p: Player, srv: TestServer, name = 'Test Atraxa', list = LIST): Promise<number> {
  const parsed = await (await p.page.request.post(srv.url + 'api/import/parse', { data: { text: list } })).json();
  if (parsed.review.length) throw new Error('unmatched: ' + parsed.review.map((r: { raw: string }) => r.raw).join(', '));
  const cards = parsed.matched.map((m: { key: string; section: string; qty: number }) => ({ key: m.key, section: m.section, qty: m.qty }));
  const d = await (await p.page.request.post(srv.url + 'api/import/commit', { data: { name, format: 'commander', cards } })).json();
  return d.id;
}

export async function api(p: Player, srv: TestServer, method: 'get' | 'post' | 'put' | 'delete', url: string, data?: unknown) {
  const r = await p.page.request[method](srv.url + url, data !== undefined ? { data } : undefined);
  const body = await r.text();
  if (!r.ok()) throw new Error(`${method} ${url} → ${r.status()} ${body}`);
  return body ? JSON.parse(body) : null;
}

/** Visible card element for a card that is currently rendered, by index within an area. */
export function tableCards(p: Page) {
  return p.locator('[data-iid]');
}

export async function cardBoxes(p: Page) {
  return p.locator('[data-iid]').evaluateAll((els) =>
    els.map((e) => {
      const r = e.getBoundingClientRect();
      return { iid: (e as HTMLElement).dataset.iid!, x: r.x, y: r.y, w: r.width, h: r.height, back: !!e.querySelector('img[alt=""]'), name: e.querySelector('img')?.getAttribute('alt') || e.querySelector('svg')?.getAttribute('aria-label') || '' };
    }),
  );
}

/** Choose `option` in one of the app's dropdowns (ui/Select), found by its label. */
export async function pickOption(p: Page, label: string, option: string) {
  await p.getByRole('button', { name: label, exact: true }).click();
  await p.getByRole('option', { name: new RegExp('^' + option.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')) }).first().click();
}

/**
 * Start a rules enforced game through the lobby, as a player would: New game, game type and format, `ais` AI players
 * (preset decks), your deck from the deck picker, ready, start. Ends on the rules game screen.
 */
export async function startRulesGame(p: Page, srv: TestServer, deck: string, ais: number, format = 'Modern') {
  await p.goto(srv.url);
  await p.getByRole('button', { name: /New game/ }).click();
  await p.waitForURL('**/lobby/*');
  await pickOption(p, 'Rules', 'Rules enforced');
  await pickOption(p, 'Format', format);
  // new lobbies start with 2 seats: add seats until there is an open one for each AI
  const addAi = p.getByRole('button', { name: 'Add AI' });
  await expect(addAi.first()).toBeVisible();
  for (let n = await addAi.count(); n < ais; n++) {
    await p.getByRole('button', { name: 'More seats' }).click();
    await expect(addAi).toHaveCount(n + 1);
  }
  for (let i = 0; i < ais; i++) await addAi.first().click();
  await p.getByRole('button', { name: 'Select deck' }).first().click();
  await p.locator('.dcard', { hasText: deck }).first().getByRole('button', { name: 'Select deck' }).click();
  await p.getByRole('button', { name: 'Ready up' }).click();
  await p.getByRole('button', { name: 'Start game' }).click();
  await p.waitForURL('**/rules?id=*');
}
