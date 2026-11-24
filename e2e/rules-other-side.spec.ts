import { expect, test, type Page } from '@playwright/test';
import { firstRun, newPlayer, startRulesGame, TestServer, type Player } from './server';

// The other side of a double-faced card in a rules game: the server sends its picture, holding Alt turns the hover
// preview over, and the card menu's "Look at the other side" turns the card over on this screen.
const srv = new TestServer(8110);
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

async function waitForQuestion(p: Page, id: string) {
  for (let i = 0; i < 200; i++) {
    const s = await api(p, 'GET', `api/rules/games/${id}`);
    if (s.waiting) return s;
    await p.waitForTimeout(150);
  }
  throw new Error('never asked');
}

/** The hover preview's card is turned over (its 3D turn is rotated). */
const previewTurned = (p: Page) =>
  p.evaluate(() => {
    const box = [...document.querySelectorAll<HTMLElement>('div')].find((d) => d.style.zIndex === '970' && d.style.position === 'fixed');
    const turn = box?.querySelector<HTMLElement>('.cardimg .turn');
    return turn ? turn.style.transform.includes('180deg') : null;
  });

test('a double-faced card shows its other side with Alt and from the card menu', async () => {
  test.setTimeout(240_000);
  const p = alice.page;
  const name = 'Delver of Secrets // Insectile Aberration';
  const parsed = await api(p, 'POST', 'api/import/parse', { text: `Main\n10 Island\n50 ${name}` });
  const cards = parsed.matched.map((m: { key: string; section: string; qty: number }) => ({ key: m.key, section: m.section, qty: m.qty }));
  await api(p, 'POST', 'api/import/commit', { name: 'Delvers', format: 'freeform', cards });
  await startRulesGame(p, srv, 'Delvers', 1, 'Freeform');
  await p.waitForFunction(() => !!localStorage.getItem('aetherhub.rulesGame'));
  const id = (await p.evaluate(() => localStorage.getItem('aetherhub.rulesGame')))!;

  let s = await waitForQuestion(p, id);
  for (let i = 0; i < 60 && s.waiting.kind.t === 'Mulligan'; i++) {
    await api(p, 'POST', `api/rules/games/${id}/answer`, { choice: { t: 'Keep', keep: true } });
    s = await waitForQuestion(p, id);
  }
  // the front's name pictures the front, and the other face rides along
  expect(s.images[name]).toMatch(/\/0$/);
  expect(s.images['@back:' + name]).toMatch(/\/1$/);
  expect(JSON.parse(s.images['@backface:' + name]).name).toBe('Insectile Aberration');
  const me = s.view.players.findIndex((x: { name: string }) => x.name === 'alice');
  const delver = s.view.cards.find((c: { name: string; zone: string; owner: number }) => c.name === name && c.zone === 'Hand' && c.owner === me);
  expect(delver, 'a Delver in the opening hand').toBeTruthy();

  await p.reload();
  const card = p.locator(`[data-iid="${delver.id}"]`);
  await expect(card).toBeVisible({ timeout: 20_000 });
  await card.hover();
  await expect.poll(() => previewTurned(p)).toBe(false);
  await p.keyboard.down('Alt');
  await expect.poll(() => previewTurned(p)).toBe(true);
  await p.screenshot({ path: test.info().outputPath('alt-other-side.png') });
  await p.keyboard.up('Alt');
  await expect.poll(() => previewTurned(p)).toBe(false);

  // the card menu turns it over on this screen; the preview follows, and Alt then shows the side that is up
  await card.click({ button: 'right' });
  await p.getByText('Look at the other side').click();
  await expect.poll(() => card.evaluate((el) => !!el.querySelector<HTMLElement>('.cardimg .turn')?.style.transform.includes('180deg'))).toBe(true);
  await p.mouse.move(0, 0);
  await card.hover();
  await expect.poll(() => previewTurned(p)).toBe(true);
  await p.keyboard.down('Alt');
  await expect.poll(() => previewTurned(p)).toBe(false);
  await p.keyboard.up('Alt');
  await p.request.delete(srv.url + `api/rules/games/${id}`);
});
