import { expect, test, type Page } from '@playwright/test';
import { firstRun, newPlayer, startRulesGame, TestServer, type Player } from './server';

// A copy on the table and in the hover preview: Chameleon, Master of Disguise copies a Soldier token and keeps its own name.
const srv = new TestServer(8108);
let alice: Player;

test.describe.configure({ mode: 'serial' });

test.beforeAll(async ({ browser }) => {
  await srv.start();
  alice = await newPlayer(browser, 'alice');
  await firstRun(alice, srv);
});
test.afterAll(async () => {
  await srv.stop();
  srv.cleanup();
});

async function api(p: Page, method: 'GET' | 'POST', path: string, data?: unknown) {
  const r = method === 'GET' ? await p.request.get(srv.url + path) : await p.request.post(srv.url + path, { data });
  expect(r.ok(), `${method} ${path}: ${await r.text()}`).toBeTruthy();
  return r.json();
}

/** Waits until this seat is asked something, and returns the game state. */
async function waitForQuestion(p: Page, id: string) {
  for (let i = 0; i < 200; i++) {
    const s = await api(p, 'GET', `api/rules/games/${id}`);
    if (s.waiting) return s;
    await p.waitForTimeout(150);
  }
  throw new Error('never asked');
}

test('a copy shows what it copies, on the table and in the hover preview', async () => {
  test.setTimeout(240_000);
  const p = alice.page;
  const parsed = await api(p, 'POST', 'api/import/parse', { text: 'Main\n30 Island\n30 Chameleon, Master of Disguise' });
  const cards = parsed.matched.map((m: { key: string; section: string; qty: number }) => ({ key: m.key, section: m.section, qty: m.qty }));
  await api(p, 'POST', 'api/import/commit', { name: 'Copycat', format: 'freeform', cards });
  await startRulesGame(p, srv, 'Copycat', 1, 'Freeform');
  await p.waitForFunction(() => !!localStorage.getItem('aetherhub.rulesGame'));
  const id = (await p.evaluate(() => localStorage.getItem('aetherhub.rulesGame')))!;
  expect(id).toBeTruthy();

  // keep, then pass until this seat has priority in its own main phase
  for (let i = 0; i < 60; i++) {
    const s = await waitForQuestion(p, id);
    const k = s.waiting.kind;
    if (k.t === 'Mulligan') await api(p, 'POST', `api/rules/games/${id}/answer`, { choice: { t: 'Keep', keep: true } });
    else if (k.t === 'Priority' && s.view.active === s.view.players.findIndex((x: { name: string }) => x.name === 'alice') && /Main/.test(String(s.view.step))) break;
    else await api(p, 'POST', `api/rules/games/${id}/answer`, { choice: k.t === 'Priority' ? { t: 'Priority', action: null } : { t: 'Default' } });
  }

  // break the rules: four Treasures to pay with, and a Soldier to copy
  const me = (await api(p, 'GET', `api/rules/games/${id}`)).view.players.findIndex((x: { name: string }) => x.name === 'alice');
  await api(p, 'POST', `api/rules/games/${id}/break`, { on: true });
  await api(p, 'POST', `api/rules/games/${id}/edit`, { op: { t: 'CreateToken', player: me, name: 'Treasure', n: 4, tapped: false } });
  await api(p, 'POST', `api/rules/games/${id}/edit`, { op: { t: 'CreateToken', player: me, name: 'Soldier', n: 1, tapped: false } });
  await api(p, 'POST', `api/rules/games/${id}/break`, { on: false });

  // cast Chameleon, pay automatically, let it resolve, and copy the Soldier
  let s = await waitForQuestion(p, id);
  const cham = s.view.cards.find((c: { name: string; zone: string }) => c.name === 'Chameleon, Master of Disguise' && c.zone === 'Hand');
  const cast = s.waiting.kind.actions.findIndex((a: { t: string; card: number; alt: number | null }) => a.t === 'Cast' && a.card === cham.id && a.alt === null);
  expect(cast).toBeGreaterThanOrEqual(0);
  await api(p, 'POST', `api/rules/games/${id}/answer`, { choice: { t: 'Priority', action: cast } });
  for (let i = 0; i < 20; i++) {
    s = await waitForQuestion(p, id);
    const k = s.waiting.kind;
    const onField = s.view.cards.find((c: { id: number; zone: string }) => c.id === cham.id && c.zone === 'Battlefield');
    if (onField && k.t === 'Priority') break;
    if (k.t === 'PayMana') await api(p, 'POST', `api/rules/games/${id}/answer`, { choice: { t: 'Pay', answer: { t: 'Auto' } } });
    else if (k.t === 'YesNo') await api(p, 'POST', `api/rules/games/${id}/answer`, { choice: { t: 'YesNo', yes: true } });
    else if (k.t === 'Pick') {
      const soldier = s.view.cards.find((c: { name: string; zone: string }) => c.name === 'Soldier' && c.zone === 'Battlefield');
      const i2 = k.options.findIndex((o: { t: string; obj?: { kind: string; id: number } }) => o.obj?.id === soldier?.id);
      await api(p, 'POST', `api/rules/games/${id}/answer`, { choice: { t: 'Pick', indices: [Math.max(0, i2)] } });
    } else await api(p, 'POST', `api/rules/games/${id}/answer`, { choice: k.t === 'Priority' ? { t: 'Priority', action: null } : { t: 'Default' } });
  }
  const v = s.view.cards.find((c: { id: number }) => c.id === cham.id);
  expect(v.zone).toBe('Battlefield');
  expect(v.name).toBe('Chameleon, Master of Disguise');
  expect(v.copy_of).toBe('Soldier');
  expect(v.art_token).toBe(true);

  // the table shows the chips, and the hover preview shows the same ones
  await p.reload();
  const card = p.locator(`[data-iid="${cham.id}"]`);
  await expect(card).toBeVisible({ timeout: 20_000 });
  await expect(card.getByText('Copy of Soldier')).toBeVisible();
  await expect(card.getByText('Named Chameleon, Master of Disguise')).toBeVisible();
  await card.hover();
  await p.waitForTimeout(400);
  await p.screenshot({ path: test.info().outputPath('copy-table-and-preview.png') });
  await expect(p.getByText('Copy of Soldier')).toHaveCount(2);
  await p.request.delete(srv.url + `api/rules/games/${id}`);
});
