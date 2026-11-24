import { expect, test, type Page } from '@playwright/test';
import { api, cardBoxes, createAndLogin, firstRun, makeDeck, newPlayer, TestServer, type Player } from './server';

const srv = new TestServer(8102);
let lobbyId = '';
/** the table's path (table/<game id>), known once the game starts */
let table = 'table';
let alice: Player;
let bob: Player;
let cass: Player;
let dev: Player;

test.describe.configure({ mode: 'serial' });

const seatSel = (name: string) => `[data-seat-name="${name}"]`;

async function pileCount(p: Page, seat: string, pile: string): Promise<string> {
  return (await p.locator(`${seatSel(seat)} [data-pile="${pile}"] .mono`).textContent())!.trim();
}

async function handCards(p: Page, seat: string) {
  return p.locator(`${seatSel(seat)} [data-hand] [data-iid]`);
}

async function openLog(p: Page) {
  const btn = p.getByRole('button', { name: 'Log & chat' });
  if (!(await btn.getAttribute('aria-pressed'))?.includes('true')) await btn.click();
}

test.beforeAll(async ({ browser }) => {
  await srv.start();
  alice = await newPlayer(browser, 'alice');
  bob = await newPlayer(browser, 'bob');
  cass = await newPlayer(browser, 'cass');
  dev = await newPlayer(browser, 'dev');
  await firstRun(alice, srv);
  await createAndLogin(alice, srv, [bob, cass, dev]);
  const d1 = await makeDeck(alice, srv, 'Alice deck');
  const d2 = await makeDeck(bob, srv, 'Bob deck');
  const d3 = await makeDeck(cass, srv, 'Cass deck');
  const made = await api(alice, srv, 'post', 'api/lobbies', { settings: { name: 'E2E', maxPlayers: 4, mulligan: 'london', planechase: true, planarPool: 'all' } });
  lobbyId = made.lobby.id;
  // newcomers sit in the first open seat: alice 1, bob 2, cass 3
  await api(bob, srv, 'post', `api/lobbies/${lobbyId}/join`, {});
  await api(cass, srv, 'post', `api/lobbies/${lobbyId}/join`, {});
  for (const [p, d, seat] of [
    [alice, d1, 0],
    [bob, d2, 1],
    [cass, d3, 2],
  ] as const) {
    await api(p, srv, 'put', `api/lobbies/${lobbyId}/seats/${seat}/deck`, { deckId: d });
    await api(p, srv, 'put', `api/lobbies/${lobbyId}/ready`, { ready: true });
  }
});

test.afterAll(async () => {
  await srv.stop();
  srv.cleanup();
});

test('the lobby banner and start take everyone to the table', async () => {
  await alice.page.goto(srv.url + `lobby/${lobbyId}`);
  await expect(alice.page.getByRole('button', { name: 'Start game' })).toBeVisible();
  await bob.page.goto(srv.url);
  await alice.page.getByRole('button', { name: 'Start game' }).click();
  await alice.page.waitForURL('**/table/*');
  await bob.page.waitForURL('**/table/*');
  table = alice.page.url().replace(srv.url, '');
  await cass.page.goto(srv.url + table);
  await expect(cass.page.locator(`${seatSel('cass')} [data-life]`)).toBeVisible();
});

test('London mulligan: take one, bottom one, keep', async () => {
  const p = alice.page;
  await expect(p.getByText('Opening hand')).toBeVisible();
  await p.getByRole('button', { name: 'Mulligan', exact: true }).click();
  await expect(p.getByText(/1 taken/)).toBeVisible();
  await p.getByRole('button', { name: 'Keep 6' }).click();
  await (await handCards(p, 'alice')).first().click();
  await p.getByRole('button', { name: 'Keep', exact: true }).click();
  await expect(await handCards(p, 'alice')).toHaveCount(6);
  for (const x of [bob, cass]) await x.page.getByRole('button', { name: 'Keep 7' }).click();
  await expect(p.getByText('Opening hand')).toHaveCount(0);
});

test('drawing is private in the log; others see a count', async () => {
  await alice.page.keyboard.press('d');
  await expect(await handCards(alice.page, 'alice')).toHaveCount(7);
  await openLog(alice.page);
  await openLog(bob.page);
  await expect(alice.page.getByText('only you').last()).toBeVisible();
  await expect(bob.page.getByText('drew 1 card').last()).toBeVisible();
});

test('dragging a card to the battlefield is seen by the others', async () => {
  const p = alice.page;
  const hand = await handCards(p, 'alice');
  const box = (await hand.first().boundingBox())!;
  const bf = (await p.locator(`${seatSel('alice')} [data-bf]`).boundingBox())!;
  await p.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await p.mouse.down();
  await p.mouse.move(bf.x + bf.width / 2, bf.y + bf.height / 2, { steps: 10 });
  await p.mouse.up();
  await expect(p.locator(`${seatSel('alice')} [data-bf] [data-iid]`)).toHaveCount(1);
  await expect(bob.page.locator(`${seatSel('alice')} [data-bf] [data-iid]`)).toHaveCount(1);
  // the opponent sees its face
  const faces = await cardBoxes(bob.page);
  expect(faces.some((c) => !c.back && c.y < 450)).toBeTruthy();
});

test('a face-down card: its controller may look at it on their screen, an opponent sees only the back', async () => {
  const p = alice.page;
  const card = p.locator(`${seatSel('alice')} [data-bf] [data-iid]`).first();
  const iid = await card.getAttribute('data-iid');
  await card.click({ button: 'right' });
  await p.getByRole('menuitem', { name: 'Turn face down' }).click();
  const shown = (pg: Page) => pg.locator(`[data-iid="${iid}"] .cardimg`).count();
  await expect.poll(() => shown(p)).toBe(0);
  await card.click({ button: 'right' });
  await p.getByRole('menuitemcheckbox', { name: /Look at the face-down card/ }).click();
  await expect.poll(() => shown(p)).toBe(1);
  // bob only sees the card back (his view has no def for it, so he has no look either)
  await expect(bob.page.locator(`[data-iid="${iid}"]`)).toBeVisible();
  expect(await shown(bob.page)).toBe(0);
  // turning it back over ends the look (and turning it face up shows the face as usual)
  await card.click({ button: 'right' });
  await p.getByRole('menuitemcheckbox', { name: /Look at the face-down card/ }).click();
  await expect.poll(() => shown(p)).toBe(0);
  await card.click({ button: 'right' });
  await p.getByRole('menuitem', { name: 'Turn face up' }).click();
  await expect.poll(() => shown(p)).toBe(1);
});

test("drawing from an opponent's library goes to the clicker's hand", async () => {
  const p = alice.page;
  const before = await pileCount(p, 'bob', 'library');
  await p.locator(`${seatSel('bob')} [data-pile="library"]`).click();
  await p.getByRole('menuitem', { name: /Draw 1 \(to your hand\)/ }).click();
  await expect(await handCards(p, 'alice')).toHaveCount(7);
  await expect.poll(() => pileCount(p, 'bob', 'library')).toBe(String(Number(before) - 1));
});

test('hand-view request and approval', async () => {
  const b = bob.page;
  await b.getByRole('button', { name: 'Log & chat' }).click();
  await b.locator(`${seatSel('alice')} [data-hand]`).click({ button: 'right', position: { x: 500, y: 12 } });
  await b.getByRole('menuitem', { name: 'Request to view hand' }).click();
  await expect(alice.page.getByText('asks to see your hand').first()).toBeVisible();
  await alice.page.getByRole('button', { name: 'Allow (live)' }).click();
  await expect(b.getByText('viewing (granted)')).toBeVisible();
  const boxes = await cardBoxes(b);
  expect(boxes.filter((c) => c.y < 200 && c.x < 1100 && !c.back).length).toBeGreaterThan(0);
});

test('arrows are visible to other players', async () => {
  const p = alice.page;
  const card = (await p.locator(`${seatSel('alice')} [data-bf] [data-iid]`).first().boundingBox())!;
  const target = (await p.locator(`${seatSel('bob')} [data-life]`).boundingBox())!;
  await p.keyboard.down('Alt');
  await p.mouse.move(card.x + card.width / 2, card.y + card.height / 2);
  await p.mouse.down();
  await p.mouse.move(target.x + 20, target.y + 20, { steps: 8 });
  await p.mouse.up();
  await p.keyboard.up('Alt');
  await expect(bob.page.locator('g.tarrow')).toHaveCount(1);
});

test('commander prompt when a commander dies', async () => {
  const p = alice.page;
  await p.locator(`${seatSel('alice')} [data-pile="command"]`).click();
  await p.getByRole('button', { name: 'Graveyard' }).first().click();
  await expect(p.getByText('Move to command zone?')).toBeVisible();
  await p.getByRole('button', { name: 'Move to command zone' }).click();
  await expect.poll(() => pileCount(p, 'alice', 'command')).toMatch(/Atraxa/);
});

test('undo that would rewind other players needs host approval', async () => {
  const b = bob.page;
  await b.locator(`${seatSel('bob')} [data-life]`).click();
  await expect(b.locator(`${seatSel('bob')} [data-life]`)).toContainText('41');
  // cass acts after bob
  await cass.page.locator(`${seatSel('cass')} [data-life]`).click({ button: 'right' });
  await expect(cass.page.locator(`${seatSel('cass')} [data-life]`)).toContainText('39');
  await b.getByRole('button', { name: /^Undo/ }).click();
  await expect(alice.page.getByText(/wants to undo/)).toBeVisible();
  await alice.page.getByRole('button', { name: 'Approve rewind' }).click();
  await expect(b.locator(`${seatSel('bob')} [data-life]`)).toContainText('40');
  await expect(b.locator(`${seatSel('cass')} [data-life]`)).toContainText('40');
});

test('spectators see every hand', async () => {
  await dev.page.goto(srv.url + table);
  await expect(dev.page.getByText(/spectating/)).toBeVisible();
  const boxes = await cardBoxes(dev.page);
  expect(boxes.filter((c) => c.back).length).toBe(0);
});

test('a second tab takes the seat; the first can take it back', async () => {
  const second = await bob.ctx.newPage();
  await second.goto(srv.url + table);
  await expect(bob.page.getByText('Opened elsewhere')).toBeVisible();
  await bob.page.getByRole('button', { name: 'Take back' }).click();
  await expect(second.getByText('Opened elsewhere')).toBeVisible();
  await second.close();
});

test('Planechase: roll the planar die', async () => {
  const p = alice.page;
  await p.getByRole('button', { name: 'Planechase' }).click();
  await p.getByRole('button', { name: 'Roll planar die' }).click();
  await p.keyboard.press('Escape');
  await openLog(p);
  await expect(p.getByText(/rolled the planar die/).last()).toBeVisible();
});

test('host powers pass to the next player when the host leaves', async () => {
  await expect(bob.page.getByRole('button', { name: 'Host' })).toHaveCount(0);
  await alice.page.goto(srv.url);
  await expect(bob.page.getByRole('button', { name: 'Host' })).toBeVisible();
  await alice.page.goto(srv.url + table);
  await expect(alice.page.getByRole('button', { name: 'Host' })).toBeVisible();
});

test('autosave: restart the server and resume', async () => {
  const handBefore = await (await handCards(alice.page, 'alice')).count();
  await srv.stop();
  await srv.start();
  await alice.page.goto(srv.url + table);
  await expect(alice.page.locator(`${seatSel('alice')} [data-life]`)).toBeVisible();
  await expect(await handCards(alice.page, 'alice')).toHaveCount(handBefore);
  await expect(alice.page.locator(`${seatSel('alice')} [data-bf] [data-iid]`)).toHaveCount(1);
});

test('ending the game records match history', async () => {
  const p = alice.page;
  await p.getByRole('button', { name: 'Host' }).click();
  await p.getByRole('menuitem', { name: 'End game…' }).click();
  await p.getByRole('button', { name: 'cass' }).click();
  await p.getByRole('button', { name: 'End game', exact: true }).click();
  await expect(p.getByText('Game over')).toBeVisible();
  await p.getByRole('button', { name: 'Close game' }).click();
  await expect(p.getByText('Game closed')).toBeVisible();
  await p.goto(srv.url + 'games?tab=history');
  await expect(p.getByText('Winner: cass', { exact: true })).toBeVisible();
  await expect(p.getByText(/Full log/)).toBeVisible();
  for (const x of [alice, bob, cass, dev]) expect(x.errors).toEqual([]);
});
