import { expect, test, type Page } from '@playwright/test';
import { api, firstRun, makeDeck, newPlayer, TestServer, type Player } from './server';

// Solo games: one player takes several seats of a lobby (sandbox) and plays them all.
const srv = new TestServer(8104);
let alice: Player;
let deckA = 0;
let deckB = 0;

test.describe.configure({ mode: 'serial' });

test.beforeAll(async ({ browser }) => {
  await srv.start();
  alice = await newPlayer(browser, 'alice');
  await firstRun(alice, srv);
  deckA = await makeDeck(alice, srv, 'Deck A');
  deckB = await makeDeck(alice, srv, 'Deck B');
});

test.afterAll(async () => {
  await srv.stop();
  srv.cleanup();
});

/** A sandbox lobby with alice in both seats (Deck A, Deck B), started from the lobby screen. Seats are named "alice 1" / "alice 2". */
async function startSolo(p: Page, mulligan: string) {
  const me = await api(alice, srv, 'get', 'api/me');
  const made = await api(alice, srv, 'post', 'api/lobbies', { settings: { name: 'Solo', format: 'commander', maxPlayers: 2, mulligan } });
  const L = `api/lobbies/${made.lobby.id}`;
  await api(alice, srv, 'post', `${L}/seats/1`, { userId: me.id });
  await api(alice, srv, 'put', `${L}/seats/0/deck`, { deckId: deckA });
  await api(alice, srv, 'put', `${L}/seats/1/deck`, { deckId: deckB });
  await api(alice, srv, 'put', `${L}/ready`, { ready: true });
  await p.goto(srv.url + `lobby/${made.lobby.id}`);
  await p.getByRole('button', { name: 'Start game' }).click();
  await p.waitForURL('**/table/*');
}

const hand = (p: Page, seat: string) => p.locator(`[data-seat-name="${seat}"] [data-hand] [data-iid]`);
const control = (p: Page) => p.getByRole('button', { name: 'Controlling seat' });

test('solo game: one person holds every seat, control follows the mulligans and the turn', async () => {
  const p = alice.page;
  // This test covers the automatic draw on a new turn, which is off by default.
  await p.request.put(srv.url + 'api/me/prefs', { data: { autoDraw: true } });
  await startSolo(p, 'london');
  await expect(control(p)).toContainText('seat 1');
  // London mulligans for both seats: control moves on by itself after each keep.
  await p.getByRole('button', { name: 'Mulligan', exact: true }).click();
  await p.getByRole('button', { name: 'Keep 6' }).click();
  await hand(p, 'alice 1').first().click();
  await p.getByRole('button', { name: 'Keep', exact: true }).click();
  await expect(control(p)).toContainText('seat 2');
  await expect(p.getByText('Mulligan · alice 2')).toBeVisible();
  await p.getByRole('button', { name: 'Mulligan', exact: true }).click();
  await p.getByRole('button', { name: 'Keep 6' }).click();
  await hand(p, 'alice 2').first().click();
  await p.getByRole('button', { name: 'Keep', exact: true }).click();
  // Everyone kept: control returns to the starting player and the game begins.
  await expect(p.getByText(/Opening hand|Waiting for/)).toHaveCount(0);
  await expect(control(p)).toContainText('seat 1');
  await expect(hand(p, 'alice 1')).toHaveCount(6);
  await expect(hand(p, 'alice 2')).toHaveCount(6);
  // Both hands are yours, so both are face up.
  await expect(p.locator('[data-hand] [data-iid] img[alt=""]')).toHaveCount(0);

  // Pass the turn as seat 1 → control follows to seat 2, who auto-drew.
  await expect(p.locator('body > div[aria-hidden="true"]')).toHaveCount(0);
  await p.getByRole('button', { name: 'Pass turn' }).click();
  await expect(control(p)).toContainText('seat 2');
  await expect(hand(p, 'alice 2')).toHaveCount(7);

  // Act as seat 2: draw goes to that hand, then undo it.
  await p.mouse.move(700, 400);
  await p.keyboard.press('d');
  await expect(hand(p, 'alice 2')).toHaveCount(8);
  await p.getByRole('button', { name: /^Undo/ }).click();
  await expect(hand(p, 'alice 2')).toHaveCount(7);

  // Switch seats by hand.
  await control(p).click();
  await p.getByRole('option', { name: /seat 1/ }).click();
  await expect(control(p)).toContainText('seat 1');
  await p.request.put(srv.url + 'api/me/prefs', { data: { autoDraw: false } });
  await closeGame(p);
  expect(alice.errors).toEqual([]);
});

test('solo game with free mulligans + scry 3: always seven, see the top three, all ten go to the bottom', async () => {
  const p = alice.page;
  await startSolo(p, 'free-scry3');
  for (let i = 1; i <= 3; i++) {
    await p.getByRole('button', { name: 'Mulligan (10 to the bottom)' }).click();
    await expect(p.getByText(`${i} taken · Free mulligans + scry 3`)).toBeVisible();
    await expect(hand(p, 'alice 1')).toHaveCount(7);
  }
  // The scry 3 happens while deciding, before keeping; keeping then moves on to the next seat.
  await p.getByRole('button', { name: 'Scry 3', exact: true }).click();
  await expect(p.getByRole('button', { name: 'Top' })).toHaveCount(3);
  await p.getByRole('button', { name: 'Done', exact: true }).click();
  await p.getByRole('button', { name: 'Keep 7' }).click();
  await expect(p.getByText('Mulligan · alice 2')).toBeVisible();
  await closeGame(p);
  expect(alice.errors).toEqual([]);
});

test('Free view: wheel zoom, WASD pan (Shift+D still draws), fit, preview off by default', async () => {
  const p = alice.page;
  await p.request.put(srv.url + 'api/me/prefs', { data: { layout: 'free' } });
  await startSolo(p, 'none');
  const zoom = p.locator('[data-nopan] .mbtn');
  // the starting zoom is the table fitted to the viewport (100% only on a big enough window)
  await expect(zoom).toHaveText(/^\d+%$/);
  const fitted = (await zoom.textContent())!;
  const mine = hand(p, 'alice 1');
  await expect(mine).toHaveCount(7);
  // Hover preview is off by default in this view.
  await mine.first().hover();
  await p.waitForTimeout(300);
  await expect(p.locator('div[style*="z-index: 970"]')).toHaveCount(0);
  // Wheel zooms.
  await p.mouse.move(600, 500);
  await p.mouse.wheel(0, -240);
  await expect(zoom).not.toHaveText(fitted);
  // Plain D pans instead of drawing; Shift+D draws.
  await p.keyboard.down('d');
  await p.waitForTimeout(200);
  await p.keyboard.up('d');
  await expect(mine).toHaveCount(7);
  await p.keyboard.press('Shift+D');
  await expect(mine).toHaveCount(8);
  // Clicking the zoom level fits the table again.
  await zoom.click();
  await expect(zoom).toHaveText(fitted);
  await p.request.put(srv.url + 'api/me/prefs', { data: { layout: 'grid' } });
  await closeGame(p);
  expect(alice.errors).toEqual([]);
});

test('box-select cards in your hand, then drag them together or move them from the menu', async () => {
  const p = alice.page;
  await startSolo(p, 'none');
  const seat = '[data-seat-name="alice 1"]';
  const mine = p.locator(`${seat} [data-hand] [data-iid]`);
  await expect(mine).toHaveCount(7);
  await expect(p.locator('body > div[aria-hidden="true"]')).toHaveCount(0);

  // Drag a box from empty hand space (right of the cards) leftwards over the last three cards.
  const strip = (await p.locator(`${seat} [data-hand]`).boundingBox())!;
  const c4 = (await mine.nth(4).boundingBox())!;
  const y = c4.y + c4.height / 2;
  await p.mouse.move(strip.x + strip.width - 20, y);
  await p.mouse.down();
  await p.mouse.move(c4.x + c4.width / 2, y + 4, { steps: 10 });
  await p.mouse.up();
  const picked = await mine.evaluateAll((els) => els.filter((e) => /var\(--acc\)/.test((e as HTMLElement).style.boxShadow)).map((e) => (e as HTMLElement).dataset.iid));
  expect(picked).toHaveLength(3);

  // Drag one of them: all three move to the battlefield together.
  const one = p.locator(`[data-iid="${picked[0]}"]`);
  const b1 = (await one.boundingBox())!;
  await p.mouse.move(b1.x + b1.width / 2, b1.y + b1.height / 2);
  await p.mouse.down();
  await p.mouse.move(b1.x + 20, b1.y - 80, { steps: 5 });
  await p.mouse.move(500, strip.y - 120, { steps: 10 });
  await p.mouse.up();
  await expect(mine).toHaveCount(4);

  // Box two more, then Move to → Graveyard from the right-click menu moves both.
  const c2 = (await mine.nth(2).boundingBox())!;
  await p.mouse.move(strip.x + strip.width - 20, c2.y + c2.height / 2);
  await p.mouse.down();
  await p.mouse.move(c2.x + c2.width / 2, c2.y + c2.height / 2 + 4, { steps: 10 });
  await p.mouse.up();
  await mine.nth(3).click({ button: 'right' });
  await expect(p.getByText('2 selected')).toBeVisible();
  await p.getByRole('menuitem', { name: /Move to/ }).click();
  await p.getByRole('menuitem', { name: 'Graveyard (discard)' }).click();
  await expect(mine).toHaveCount(2);
  await expect(p.locator('[data-seat="0"] [data-pile="graveyard"]')).toContainText('2');
  await closeGame(p);
  expect(alice.errors).toEqual([]);
});

/** Leave the table; the game keeps running (several games can run at once), so the next test simply starts another. */
async function closeGame(p: Page) {
  await p.goto(srv.url);
}
