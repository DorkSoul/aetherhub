import { expect, test } from '@playwright/test';
import { api, createAndLogin, firstRun, makeDeck, newPlayer, TestServer, type Player } from './server';

const srv = new TestServer(8103);
let table = 'table';
let alice: Player;
let bob: Player;

test.describe.configure({ mode: 'serial' });

test.beforeAll(async ({ browser }) => {
  await srv.start();
  alice = await newPlayer(browser, 'alice');
  bob = await newPlayer(browser, 'bob');
  await firstRun(alice, srv);
  await createAndLogin(alice, srv, [bob]);
  const d1 = await makeDeck(alice, srv, 'Alice deck');
  const d2 = await makeDeck(bob, srv, 'Bob deck');
  const made = await api(alice, srv, 'post', 'api/lobbies', { settings: { name: 'Bo3', bo: 3, format: 'commander' } });
  const L = `api/lobbies/${made.lobby.id}`;
  await api(bob, srv, 'post', `${L}/join`, {});
  for (const [p, d, seat] of [
    [alice, d1, 0],
    [bob, d2, 1],
  ] as const) {
    await api(p, srv, 'put', `${L}/seats/${seat}/deck`, { deckId: d });
    await api(p, srv, 'put', `${L}/ready`, { ready: true });
  }
  const started = await api(alice, srv, 'post', `${L}/start`, {});
  table = `table/${started.id}`;
});

test.afterAll(async () => {
  await srv.stop();
  srv.cleanup();
});

test('Bo3: sideboard swap between games, loser picks who starts', async () => {
  await alice.page.goto(srv.url + table);
  await bob.page.goto(srv.url + table);
  await expect(alice.page.locator('[data-seat-name="alice"] [data-life]')).toBeVisible();
  // bob wins game 1
  await alice.page.getByRole('button', { name: 'Host' }).click();
  await alice.page.getByRole('menuitem', { name: 'End game…' }).click();
  await alice.page.getByRole('button', { name: 'bob' }).click();
  await alice.page.getByRole('button', { name: 'End game', exact: true }).click();
  await expect(alice.page.getByText(/Sideboarding · game 2 of 3/)).toBeVisible();
  await expect(bob.page.getByText(/Sideboarding · game 2 of 3/)).toBeVisible();
  // alice swaps Lightning Bolt into the main deck
  await alice.page.getByRole('button', { name: '← Main' }).first().click();
  await expect(alice.page.getByText(/Sideboard · 1/)).toBeVisible();
  await expect(alice.page.getByText(/You lost the last game — choose who starts/)).toBeVisible();
  await alice.page.getByRole('button', { name: 'alice', exact: true }).click();
  await alice.page.getByRole('button', { name: 'Ready for next game' }).click();
  await bob.page.getByRole('button', { name: 'Ready for next game' }).click();
  await expect(alice.page.getByText(/Sideboarding/)).toHaveCount(0);
  await expect(alice.page.getByText(/game 2/).first()).toBeVisible();
  await expect(alice.page.getByText('Your turn')).toBeVisible();
  // the saved deck is unchanged
  const decks = await api(alice, srv, 'get', 'api/decks');
  const mine = decks.find((d: { name: string }) => d.name === 'Alice deck');
  const deck = await api(alice, srv, 'get', `api/decks/${mine.id}`);
  const side = deck.cards.filter((c: { section: string }) => c.section === 'side');
  expect(side.length).toBe(2);
  expect(alice.errors).toEqual([]);
  expect(bob.errors).toEqual([]);
});
