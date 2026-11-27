import { expect, test } from '@playwright/test';
import { firstRun, newPlayer, pickOption, TestServer, type Player } from './server';

const srv = new TestServer(8111);
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

const nav = (p: Player['page']) => p.locator('aside');

test('preconstructed decks appear in their own tab once an admin adds them, and never in All decks', async () => {
  const p = alice.page;
  await p.goto(srv.url + 'decks');
  await expect(nav(p).getByRole('button', { name: /My decks/ })).toHaveClass(/\bon\b/);
  await expect(nav(p).getByRole('button', { name: /Pre-constructed/ })).toHaveCount(0);

  await p.goto(srv.url + 'settings/cards');
  await p.getByRole('button', { name: 'Add decks' }).click();
  await expect(p.getByRole('button', { name: 'Remove' })).toBeVisible();

  await p.goto(srv.url + 'decks');
  await expect(nav(p).getByRole('button', { name: /Pre-constructed/ })).toBeVisible();
  const labels = await nav(p).locator('button.nav .grow').allTextContents();
  expect(labels.slice(0, 5)).toEqual(['My decks', 'Others’ decks', 'All decks', 'Pinned', 'Pre-constructed']);
  await nav(p).getByRole('button', { name: /Pre-constructed/ }).click();
  await expect(p.locator('.dcard')).toHaveCount(2);
  await expect(p.locator('.dcard', { hasText: 'Angelic Test' })).toContainText('Commander Deck');
  await expect(p.locator('.dcard', { hasText: 'Angelic Test' }).getByLabel(/Pin/)).toHaveCount(0);

  await nav(p).getByRole('button', { name: /All decks/ }).click();
  await expect(p.locator('.dcard', { hasText: 'Angelic Test' })).toHaveCount(0);
});

test('a preconstructed deck opens read-only and can be copied into your decks', async () => {
  const p = alice.page;
  await p.goto(srv.url + 'decks');
  await nav(p).getByRole('button', { name: /Pre-constructed/ }).click();
  await p.locator('.dcard', { hasText: 'Elvish Test' }).getByRole('link', { name: 'View' }).click();
  await expect(p.getByText('Theme Deck · TST')).toBeVisible();
  await expect(p.getByRole('button', { name: 'Delete' })).toHaveCount(0);
  await p.getByRole('button', { name: 'Copy to my decks' }).click();
  await expect(p).toHaveURL(/\/decks\/\d+$/);
  await expect(p.getByText('by alice')).toBeVisible();
});

test('the lobby deck picker offers them, and a game starts with one', async () => {
  const p = alice.page;
  await p.goto(srv.url);
  await p.getByRole('button', { name: /New game/ }).click();
  await p.waitForURL('**/lobby/*');
  await pickOption(p, 'Rules', 'Rules enforced');
  await pickOption(p, 'Format', 'Freeform');
  await p.getByRole('button', { name: 'Add AI' }).first().click();
  await p.getByRole('button', { name: 'Select deck' }).first().click();
  const picker = p.locator('.deck-picker');
  await picker.locator('aside').getByRole('button', { name: /Pre-constructed/ }).click();
  await picker.locator('.dcard', { hasText: 'Elvish Test' }).getByRole('button', { name: 'Select deck' }).click();
  await expect(p.getByText('Elvish Test')).toBeVisible();
  await p.getByRole('button', { name: 'Ready up' }).click();
  await p.getByRole('button', { name: 'Start game' }).click();
  await p.waitForURL('**/rules?id=*');
  expect(alice.errors).toEqual([]);
});

test('the server log shows what happened, and filters by category', async () => {
  const p = alice.page;
  await p.goto(srv.url + 'settings/server');
  const log = p.getByRole('log', { name: 'Server log entries' });
  await expect(log.getByText('Added the preconstructed decks')).toBeVisible();
  await expect(log.getByText('Copied deck “Elvish Test” into their decks')).toBeVisible();
  await expect(log.getByText('Started the game in lobby')).toBeVisible();
  await pickOption(p, 'Log category', 'Decks');
  await expect(log.getByText('Added the preconstructed decks')).toHaveCount(0);
  await expect(log.getByText('Copied deck “Elvish Test” into their decks')).toBeVisible();
  await log.getByText('Copied deck “Elvish Test” into their decks').click();
  await expect(log.getByText(/from 127\.0\.0\.1/)).toBeVisible();
  expect(alice.errors).toEqual([]);
});
