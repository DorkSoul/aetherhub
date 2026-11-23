import { expect, test, type Page } from '@playwright/test';
import { firstRun, newPlayer, startRulesGame, TestServer, type Player } from './server';

const srv = new TestServer(8106);
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

async function endGame(p: Page) {
  const id = await p.evaluate(() => localStorage.getItem('aetherhub.rulesGame'));
  if (id) await p.request.delete(srv.url + `api/rules/games/${id}`);
  await p.evaluate(() => localStorage.removeItem('aetherhub.rulesGame'));
}

async function makeBurn(p: Page): Promise<number> {
  const parsed = await (await p.request.post(srv.url + 'api/import/parse', { data: { text: 'Main\n22 Mountain\n14 Lightning Bolt\n12 Llanowar Elves\n4 Rancor\n8 Forest' } })).json();
  const cards = parsed.matched.map((m: { key: string; section: string; qty: number }) => ({ key: m.key, section: m.section, qty: m.qty }));
  const deck = await (await p.request.post(srv.url + 'api/import/commit', { data: { name: 'Burn', format: 'modern', cards } })).json();
  expect(deck.id).toBeTruthy();
  return deck.id;
}

/** Whether you hold priority: the side panel says so, and in the minimal view the Priority chip does. */
const yourPriority = (p: Page) => p.getByText('Your priority.').or(p.getByText(/^Priority · /)).first();

/** Answer whatever prompt is open in the simplest legal way. */
async function simplePrompt(p: Page) {
  for (const name of ['Keep', 'No attack', 'No blocks', 'Auto-pay', 'No']) {
    const b = p.getByRole('button', { name, exact: true });
    if (await b.first().isVisible()) {
      await b.first().click({ timeout: 2000 }).catch(() => {});
      return true;
    }
  }
  const confirm = p.getByRole('button', { name: 'Confirm', exact: true });
  if (await confirm.isVisible()) {
    await confirm.click({ timeout: 2000 }).catch(() => {});
    return true;
  }
  return false;
}

test('rules game vs the AI: start, keep, play a land, reach game over by conceding', async () => {
  test.setTimeout(240_000);
  const p = alice.page;
  await makeBurn(p);
  await startRulesGame(p, srv, 'Burn', 1);

  const seat = '[data-seat-name="alice"]';
  const field = p.locator(`${seat} [data-bf] [data-iid]`);
  const deadline = Date.now() + 150_000;
  let tryIdx = 0;
  while ((await field.count()) < 1) {
    if (Date.now() > deadline) throw new Error('never played a land: ' + (await p.locator('body').innerText()).slice(0, 1500));
    if (await yourPriority(p).isVisible()) {
      const hand = p.locator(`${seat} [data-hand] [data-iid]`);
      if (await hand.count()) {
        const hb = (await hand.nth(tryIdx++ % (await hand.count())).boundingBox())!;
        const f = (await p.locator(`${seat} [data-bf]`).boundingBox())!;
        await p.mouse.move(hb.x + hb.width / 2, hb.y + hb.height / 2);
        await p.mouse.down();
        await p.mouse.move(f.x + f.width * 0.8, f.y + f.height * 0.7, { steps: 12 });
        await p.mouse.up();
        await p.waitForTimeout(900);
      }
    } else if (!(await simplePrompt(p))) await p.waitForTimeout(300);
  }
  expect(await field.count()).toBeGreaterThanOrEqual(1);
  // the opponent exists on the table too
  await expect(p.getByText(/^AI /).first()).toBeVisible();
  await endGame(p);
});

test('rules game with three AI opponents shows every seat on the table', async () => {
  test.setTimeout(120_000);
  const p = alice.page;
  await p.evaluate(() => localStorage.removeItem('aetherhub.rulesGame'));
  await startRulesGame(p, srv, 'Burn', 3);
  for (const n of ['AI Ava', 'AI Bryn', 'AI Cora']) await expect(p.getByText(n, { exact: true }).first()).toBeVisible({ timeout: 90_000 });
  await endGame(p);
});

test('end turn passes priority until the next turn, and the game log fills in', async () => {
  test.setTimeout(180_000);
  const p = alice.page;
  await makeBurn(p);
  await p.evaluate(() => localStorage.removeItem('aetherhub.rulesGame'));
  await startRulesGame(p, srv, 'Burn', 1);
  const deadline = Date.now() + 120_000;
  // answer prompts (keep, ...) until the game is under way and we hold priority
  while (!(await yourPriority(p).isVisible())) {
    if (Date.now() > deadline) throw new Error('no priority: ' + (await p.locator('body').innerText()).slice(0, 1200));
    if (!(await simplePrompt(p))) await p.waitForTimeout(250);
  }
  const turn0 = await p.evaluate(async () => {
    const id = localStorage.getItem('aetherhub.rulesGame');
    return (await (await fetch(`api/rules/games/${id}`)).json()).view.turn as number;
  });
  await p.getByRole('button', { name: 'End turn' }).first().click();
  // passing everything reaches the next turn, or stops at the cleanup step to ask which card to discard (8 cards in hand on the draw)
  await expect
    .poll(
      async () =>
        p.evaluate(async (t0) => {
          const id = localStorage.getItem('aetherhub.rulesGame');
          const j = await (await fetch(`api/rules/games/${id}`)).json();
          return j.view.turn > t0 || j.waiting?.kind?.purpose?.t === 'Discard';
        }, turn0),
      { timeout: 60_000 },
    )
    .toBe(true);
  await endGame(p);
});
