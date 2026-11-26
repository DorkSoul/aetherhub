import { expect, test, type Page } from '@playwright/test';
import { firstRun, newPlayer, startRulesGame, TestServer, type Player } from './server';

// Where the table puts things in a rules game: an Aura on an opponent's creature and a Curse on an opponent sit on that
// opponent's side; the hand keeps the order the player gives it (a drawn card goes at the end); sorting a tidy board
// moves nothing.
const srv = new TestServer(8109);
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

type Card = { id: number; name: string; zone: string; owner: number; controller: number };

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

/** Casts a card from the hand and answers its questions (targets by `pick`) until it is on the battlefield. */
async function cast(p: Page, id: string, card: Card, pick: (o: { t: string; obj?: { kind: string; id: number } }) => boolean) {
  let s = await waitForQuestion(p, id);
  const action = s.waiting.kind.actions.findIndex((a: { t: string; card: number; alt: number | null }) => a.t === 'Cast' && a.card === card.id && a.alt === null);
  expect(action, `cast ${card.name}`).toBeGreaterThanOrEqual(0);
  await api(p, 'POST', `api/rules/games/${id}/answer`, { choice: { t: 'Priority', action } });
  for (let i = 0; i < 30; i++) {
    s = await waitForQuestion(p, id);
    const k = s.waiting.kind;
    if (s.view.cards.some((c: Card) => c.id === card.id && c.zone === 'Battlefield') && k.t === 'Priority') return;
    if (k.t === 'PayMana') await api(p, 'POST', `api/rules/games/${id}/answer`, { choice: { t: 'Pay', answer: { t: 'Auto' } } });
    else if (k.t === 'Pick') await api(p, 'POST', `api/rules/games/${id}/answer`, { choice: { t: 'Pick', indices: [Math.max(0, k.options.findIndex(pick))] } });
    else await api(p, 'POST', `api/rules/games/${id}/answer`, { choice: k.t === 'Priority' ? { t: 'Priority', action: null } : { t: 'Default' } });
  }
  throw new Error(`${card.name} never resolved`);
}

/** The ids of the hand cards on this seat's side, left to right. */
async function handOrder(p: Page, seat: number): Promise<string[]> {
  const cards = p.locator(`[data-seat="${seat}"] [data-iid]:not([data-bf] [data-iid])`);
  const out: { iid: string; x: number }[] = [];
  for (const el of await cards.all()) {
    const iid = await el.getAttribute('data-iid');
    const box = await el.boundingBox();
    if (iid && box && /^\d+$/.test(iid)) out.push({ iid, x: box.x });
  }
  return out.sort((a, b) => a.x - b.x).map((c) => c.iid);
}

test('Auras and Curses on an opponent sit on their side, the hand keeps its order, and sorting a sorted board again moves nothing', async () => {
  test.setTimeout(300_000);
  const p = alice.page;
  const parsed = await api(p, 'POST', 'api/import/parse', { text: 'Main\n20 Island\n20 Rancor\n20 Curse of Disturbance' });
  const cards = parsed.matched.map((m: { key: string; section: string; qty: number }) => ({ key: m.key, section: m.section, qty: m.qty }));
  await api(p, 'POST', 'api/import/commit', { name: 'Curses', format: 'freeform', cards });
  await startRulesGame(p, srv, 'Curses', 1, 'Freeform');
  await p.waitForFunction(() => !!localStorage.getItem('aetherhub.rulesGame'));
  const id = (await p.evaluate(() => localStorage.getItem('aetherhub.rulesGame')))!;

  // keep, then pass until this seat has priority in its own main phase
  for (let i = 0; i < 60; i++) {
    const s = await waitForQuestion(p, id);
    const k = s.waiting.kind;
    if (k.t === 'Mulligan') await api(p, 'POST', `api/rules/games/${id}/answer`, { choice: { t: 'Keep', keep: true } });
    else if (k.t === 'Priority' && s.view.active === s.view.players.findIndex((x: { name: string }) => x.name === 'alice') && /Main/.test(String(s.view.step))) break;
    else await api(p, 'POST', `api/rules/games/${id}/answer`, { choice: k.t === 'Priority' ? { t: 'Priority', action: null } : { t: 'Default' } });
  }

  // break the rules: Treasures to pay with, a Soldier for the opponent, and one of each spell in hand
  let s = await api(p, 'GET', `api/rules/games/${id}`);
  const me = s.view.players.findIndex((x: { name: string }) => x.name === 'alice');
  const opp = 1 - me;
  const edit = (op: unknown) => api(p, 'POST', `api/rules/games/${id}/edit`, { op });
  await api(p, 'POST', `api/rules/games/${id}/break`, { on: true });
  await edit({ t: 'CreateToken', player: me, name: 'Treasure', n: 6, tapped: false });
  await edit({ t: 'CreateToken', player: opp, name: 'Soldier', n: 1, tapped: false });
  // draw until both spells are in hand
  for (let i = 0; i < 30; i++) {
    s = await api(p, 'GET', `api/rules/games/${id}`);
    const has = (name: string) => s.view.cards.some((c: Card) => c.name === name && c.zone === 'Hand' && c.owner === me);
    if (has('Rancor') && has('Curse of Disturbance')) break;
    await edit({ t: 'Draw', player: me });
  }
  await api(p, 'POST', `api/rules/games/${id}/break`, { on: false });

  s = await api(p, 'GET', `api/rules/games/${id}`);
  const soldier: Card = s.view.cards.find((c: Card) => c.name === 'Soldier' && c.zone === 'Battlefield');
  const aura: Card = s.view.cards.find((c: Card) => c.name === 'Rancor' && c.zone === 'Hand');
  const curse: Card = s.view.cards.find((c: Card) => c.name === 'Curse of Disturbance' && c.zone === 'Hand');
  await cast(p, id, aura, (o) => o.obj?.id === soldier.id && o.obj?.kind !== 'Player');
  await cast(p, id, curse, (o) => o.obj?.kind === 'Player' && o.obj.id === opp);
  s = await api(p, 'GET', `api/rules/games/${id}`);
  expect(s.view.cards.find((c: { id: number }) => c.id === aura.id).attached_to).toBe(soldier.id);
  expect(s.view.cards.find((c: { id: number }) => c.id === curse.id).enchanted_player).toBe(opp);

  await p.reload();
  const onSide = (seat: number, card: Card) => p.locator(`[data-seat="${seat}"] [data-bf] [data-iid="${card.id}"]`);
  await expect(onSide(opp, aura)).toBeVisible({ timeout: 20_000 });
  await expect(onSide(opp, curse)).toBeVisible();
  await expect(onSide(me, aura)).toHaveCount(0);
  await p.screenshot({ path: test.info().outputPath('aura-and-curse-on-opponent.png') });

  // drag the leftmost hand card to the right end of the hand
  const before = await handOrder(p, me);
  expect(before.length).toBeGreaterThanOrEqual(3);
  const first = (await p.locator(`[data-iid="${before[0]}"]`).boundingBox())!;
  const last = (await p.locator(`[data-iid="${before[before.length - 1]}"]`).boundingBox())!;
  await p.mouse.move(first.x + first.width / 2, first.y + first.height / 2);
  await p.mouse.down();
  await p.mouse.move(first.x + first.width / 2 + 20, first.y + first.height / 2, { steps: 4 });
  await p.mouse.move(last.x + last.width + 10, last.y + last.height / 2, { steps: 12 });
  await p.mouse.up();
  const moved = [...before.slice(1), before[0]];
  await expect.poll(() => handOrder(p, me)).toEqual(moved);

  // a drawn card goes at the end; the rest keep the order the player gave them
  await api(p, 'POST', `api/rules/games/${id}/break`, { on: true });
  await edit({ t: 'Draw', player: me });
  await api(p, 'POST', `api/rules/games/${id}/break`, { on: false });
  await expect.poll(async () => (await handOrder(p, me)).length).toBe(moved.length + 1);
  const after = await handOrder(p, me);
  expect(after.slice(0, -1)).toEqual(moved);

  // the first sort stacks the six Treasures into a pile; sorting the sorted board again moves nothing
  const mine = p.locator(`[data-seat="${me}"] [data-bf] [data-iid]`);
  const boxes = async () => Promise.all((await mine.all()).map(async (el) => ({ iid: await el.getAttribute('data-iid'), box: await el.boundingBox() })));
  await p.locator('body').press('s');
  await p.waitForTimeout(800);
  const was = await boxes();
  expect(was.length).toBeGreaterThan(0);
  await p.locator('body').press('s');
  await p.waitForTimeout(800);
  for (const { iid, box } of await boxes()) {
    const old = was.find((w) => w.iid === iid)!.box!;
    expect(Math.abs(box!.x - old.x) + Math.abs(box!.y - old.y), `card ${iid} moved`).toBeLessThan(2);
  }

  // the Treasures can lie exactly on top of each other instead, with their count on the top one
  s = await api(p, 'GET', `api/rules/games/${id}`);
  const treasures: Card[] = s.view.cards.filter((c: Card) => c.name === 'Treasure' && c.zone === 'Battlefield' && c.controller === me);
  const n = treasures.length;
  expect(n).toBeGreaterThanOrEqual(2);
  await onSide(me, treasures[0]).click({ button: 'right' });
  await p.getByText(`Group identical: stacked with count (×${n})`).click();
  await expect(p.locator(`[data-seat="${me}"] [data-bf]`).getByText(`×${n}`, { exact: true })).toBeVisible();
  const spread = async () => {
    const spots = await Promise.all(treasures.map(async (t) => (await onSide(me, t).boundingBox())!));
    return Math.max(...spots.map((b) => Math.abs(b.x - spots[0].x) + Math.abs(b.y - spots[0].y)));
  };
  await expect.poll(spread).toBeLessThan(2);
  await p.screenshot({ path: test.info().outputPath('treasures-stacked.png') });

  // the admin Server page lists the game and shows the server log
  await p.goto(srv.url + 'settings/server');
  await expect(p.getByRole('heading', { name: 'Running games' })).toBeVisible({ timeout: 20_000 });
  await expect(p.getByText(/alice, .*\(AI\)/)).toBeVisible();
  await expect(p.getByText('in progress', { exact: false }).first()).toBeVisible();
  await expect(p.getByRole('heading', { name: 'Server log' })).toBeVisible();
  await p.screenshot({ path: test.info().outputPath('server-page.png'), fullPage: true });
  await p.request.delete(srv.url + `api/rules/games/${id}`);
});

test("breaking the rules, the AI's hand and library can be looked at, and are hidden again afterwards", async () => {
  test.setTimeout(300_000);
  const p = alice.page;
  await startRulesGame(p, srv, 'Curses', 1, 'Freeform');
  await p.waitForFunction(() => !!localStorage.getItem('aetherhub.rulesGame'));
  const id = (await p.evaluate(() => localStorage.getItem('aetherhub.rulesGame')))!;
  for (let i = 0; i < 60; i++) {
    const s = await waitForQuestion(p, id);
    const k = s.waiting.kind;
    if (k.t === 'Mulligan') await api(p, 'POST', `api/rules/games/${id}/answer`, { choice: { t: 'Keep', keep: true } });
    else if (k.t === 'Priority') break;
    else await api(p, 'POST', `api/rules/games/${id}/answer`, { choice: { t: 'Default' } });
  }
  let s = await api(p, 'GET', `api/rules/games/${id}`);
  const me = s.view.players.findIndex((x: { name: string }) => x.name === 'alice');
  const opp = 1 - me;
  let oppHand = s.view.players[opp].hand;
  await api(p, 'POST', `api/rules/games/${id}/break`, { on: true });
  await p.reload();
  const hand = p.locator(`[data-seat="${opp}"] [data-hand]`);
  await expect(hand).toBeVisible({ timeout: 20_000 });
  // taking a random card from the AI's hand puts it into this player's hand
  const mine = s.view.players[me].hand;
  await hand.click({ button: 'right', position: { x: 20, y: 20 } });
  await p.getByText('Take a random card').click();
  await expect.poll(async () => (await api(p, 'GET', `api/rules/games/${id}`)).view.players[me].hand, { timeout: 10_000 }).toBe(mine + 1);
  s = await api(p, 'GET', `api/rules/games/${id}`);
  expect(s.view.players[opp].hand).toBe(oppHand - 1);
  oppHand -= 1;
  await hand.click({ button: 'right', position: { x: 20, y: 20 } });
  await p.getByText('Request to view hand').click();
  // the AI agrees at once: its cards are face up (real ids), with no extra face-down ones
  await expect.poll(async () => (await handOrder(p, opp)).length, { timeout: 10_000 }).toBe(oppHand);
  await expect(p.locator(`[data-seat="${opp}"] [data-iid^="H"]`)).toHaveCount(0);
  s = await api(p, 'GET', `api/rules/games/${id}`);
  expect(s.view.cards.filter((c: Card) => c.owner === opp && c.zone === 'Hand')).toHaveLength(oppHand);
  // the library, in order
  await api(p, 'POST', `api/rules/games/${id}/edit`, { op: { t: 'Show', player: opp, zone: 'Library' } });
  s = await api(p, 'GET', `api/rules/games/${id}`);
  expect(s.view.players[opp].library_shown).toHaveLength(s.view.players[opp].library);
  await api(p, 'POST', `api/rules/games/${id}/break`, { on: false });
  s = await api(p, 'GET', `api/rules/games/${id}`);
  expect(s.view.cards.filter((c: Card) => c.owner === opp && c.zone === 'Hand')).toHaveLength(0);
  expect(s.view.players[opp].library_shown).toHaveLength(0);
});
