import { expect, test } from '@playwright/test';
import { api, createAndLogin, firstRun, makeDeck, newPlayer, TestServer, type Player, pickOption } from './server';

const srv = new TestServer(8101);
let alice: Player;
let bob: Player;

test.describe.configure({ mode: 'serial' });

test.beforeAll(async ({ browser }) => {
  await srv.start();
  alice = await newPlayer(browser, 'alice');
  bob = await newPlayer(browser, 'bob');
});

test.afterAll(async () => {
  await srv.stop();
  srv.cleanup();
});

test('first run creates the admin, admin creates a user with a temp password', async () => {
  await firstRun(alice, srv);
  await alice.page.goto(srv.url + 'settings/users');
  await alice.page.getByPlaceholder('New username').fill('cass');
  await alice.page.getByRole('button', { name: 'Create user' }).click();
  await expect(alice.page.getByText(/Temporary password: \w{4}-\w{4}-\w{4}/)).toBeVisible();
  await createAndLogin(alice, srv, [bob]);
});

test('login is rate limited after five failures', async ({ request }) => {
  for (let i = 0; i < 4; i++) expect((await request.post(srv.url + 'api/auth/login', { data: { username: 'cass', password: 'nope' } })).status()).toBe(401);
  const r = await request.post(srv.url + 'api/auth/login', { data: { username: 'cass', password: 'nope' } });
  expect(r.status()).toBe(429);
  expect((await r.json()).error).toMatch(/Too many attempts/);
});

test('import matches single-slash names and flavor names by set + number', async () => {
  const text = '1 Bushi Tenderfoot / Kenzo the Hardhearted (CHK) 2\n1 Ring of the Sun (WHO) 836\n1 Lightning Bolt (NOPE) 1';
  const r = await (await alice.page.request.post(srv.url + 'api/import/parse', { data: { text } })).json();
  expect(r.review).toEqual([]);
  expect(r.matched.map((m: { resolvedName: string }) => m.resolvedName)).toEqual(['Bushi Tenderfoot // Kenzo the Hardhearted', 'Sol Ring', 'Lightning Bolt']);
});

test('import with review, then edit in the deck editor', async () => {
  const p = alice.page;
  await p.goto(srv.url + 'import');
  await p.getByLabel('Paste deck list').fill("Commander\n1 Atraxa, Praetor's Voice\n\nMain\n1 Sol Ring (C21) 263\n1 Counterspel\n20 Forest\n\nSideboard\n1 Lightning Bolt\n\nTokens\n1 Treasure");
  await expect(p.getByText('Detected:')).toBeVisible();
  await expect(p.getByText('Did you mean').first()).toBeVisible();
  await p.getByRole('button', { name: 'Counterspell', exact: true }).click();
  await p.getByRole('button', { name: "Atraxa, Praetors' Voice", exact: true }).click();
  await expect(p.getByText('Ready. Card images download')).toBeVisible();
  await p.getByRole('button', { name: /^Import \d+ cards/ }).click();
  await expect(p.getByText('Mana curve')).toBeVisible();
  await expect(p.getByRole('heading', { name: 'Atraxa (imported)' })).toBeVisible();
  // warnings: Lightning Bolt is sideboard (no CI check), deck size warning
  await expect(p.getByText(/of 100 cards/).first()).toBeVisible();
  // add a card through search
  await p.getByLabel('Search cards to add').fill('t:creature c:u');
  await p.getByRole('button', { name: '+ Main' }).first().click();
  await expect(p.getByText(/Creatures/).first()).toBeVisible();
  // hover preview beside the card (clicking elsewhere closes the search results)
  await p.getByRole('heading', { name: 'Atraxa (imported)' }).click();
  await p.locator('.dtile').first().hover();
  await expect(p.locator('div[style*="z-index: 970"]')).toBeVisible();
  await p.mouse.move(5, 5);
  // right-click menu: add copies, then move one copy to the sideboard
  const sol = p.locator('.dtile').filter({ has: p.locator('img[alt="Sol Ring"], svg[aria-label="Sol Ring"]') });
  await sol.click({ button: 'right' });
  await p.getByPlaceholder('How many?').first().fill('2');
  await p.getByRole('button', { name: 'Add', exact: true }).click();
  await expect(sol.getByText('3×')).toBeVisible();
  await sol.click({ button: 'right' });
  await p.getByText('Move 1 to').click();
  await p.getByText('Sideboard', { exact: true }).click();
  await expect(sol.getByText('2×')).toBeVisible();
  await expect(p.getByRole('button', { name: /^Side 2/ })).toBeVisible();
  // switch views
  for (const v of ['Stacks', 'Text', 'Table', 'Visual']) await p.getByRole('button', { name: v, exact: true }).click();
  // export
  await p.getByRole('button', { name: 'Export' }).click();
  await p.getByRole('button', { name: 'Arena', exact: true }).click();
  await expect(p.getByLabel('Exported deck list')).toHaveValue(/Deck\n/);
  await p.keyboard.press('Escape');
  // sample hand
  await p.getByRole('button', { name: 'Sample hand' }).click();
  await expect(p.getByText(/to keep/)).toBeVisible();
  await p.getByRole('button', { name: 'Mulligan (London)' }).click();
  await expect(p.getByText(/put 1 on the bottom/)).toBeVisible();
  await p.keyboard.press('Escape');
  // card detail: change quantity, add a tag
  await p.locator('.dtile').filter({ hasText: '' }).nth(1).click();
  await p.getByRole('dialog').getByRole('button', { name: '#ramp' }).click();
  await expect(p.getByRole('dialog').getByRole('button', { name: '#ramp' })).toHaveAttribute('aria-pressed', 'true');
  await p.keyboard.press('Escape');
});

test('tags: bulk tag from the menu, filter, group, rename, export, delete', async () => {
  const p = alice.page;
  const tiles = p.locator('.dtile');
  const bar = p.getByRole('group', { name: 'Filter by tag' });
  // select two cards, tag them both from the right-click menu
  await tiles.nth(2).click({ modifiers: ['Control'] });
  await tiles.nth(3).click({ modifiers: ['Control'] });
  await expect(p.getByText('2 selected')).toBeVisible();
  await tiles.nth(3).click({ button: 'right' });
  // hovering an item with ▸ opens its submenu beside the menu; both stay open
  await p.getByRole('menuitem', { name: /Tag these cards/ }).hover();
  const menus = p.getByRole('menu');
  await expect(menus).toHaveCount(2);
  await expect(menus.first().getByText('Add 1')).toBeVisible();
  const sub = menus.last();
  await sub.getByPlaceholder('e.g. mana rock').fill('#Mana Rock');
  await sub.getByRole('button', { name: 'Add', exact: true }).click();
  await expect(sub.getByRole('button', { name: '#mana rock', exact: true })).toHaveAttribute('aria-pressed', 'true');
  // a deck-only tag, from the same submenu
  await sub.getByPlaceholder('e.g. mana rock').fill('opener');
  await sub.getByLabel('This deck only').check();
  await sub.getByRole('button', { name: 'Add', exact: true }).click();
  await expect(sub.getByRole('button', { name: '#opener · deck', exact: true })).toHaveAttribute('aria-pressed', 'true');
  // a third level: All tags ▸ opens beside the submenu
  await sub.getByRole('menuitem', { name: /All tags/ }).hover();
  await expect(menus).toHaveCount(3);
  await expect(menus.last().getByText('Global · in all your decks')).toBeVisible();
  // moving back onto a plain item in the root closes the submenus
  await menus.first().getByText('Add 1').hover();
  await expect(menus).toHaveCount(1);
  await p.keyboard.press('Escape');
  await expect(tiles.locator('.tagc', { hasText: '#mana rock' })).toHaveCount(2);
  // filter by the tag, then clear
  const all = await tiles.count();
  await bar.getByRole('button', { name: /^#mana rock/ }).click();
  await expect(tiles).toHaveCount(2);
  await bar.getByRole('button', { name: /^All/ }).click();
  await expect(tiles).toHaveCount(all);
  // group by tag: a card with two tags shows under both
  await pickOption(p, 'Group by', 'Tag');
  await expect(p.getByRole('heading', { name: '#mana rock' })).toBeVisible();
  await expect(p.getByRole('heading', { name: 'Untagged' })).toBeVisible();
  // rename in Manage tags, give it a colour
  await bar.getByRole('button', { name: 'Manage tags…' }).click();
  await p.getByRole('dialog').locator('div.row', { hasText: '#mana rock' }).first().getByRole('button', { name: 'Rename' }).click();
  await p.getByLabel('Rename #mana rock').fill('rocks');
  await p.getByLabel('Rename #mana rock').press('Enter');
  await expect(p.getByRole('dialog').getByText('#rocks')).toBeVisible();
  await p.getByLabel('Colour for #rocks').click();
  await p.getByLabel('Colour #7db4ea').click();
  await p.keyboard.press('Escape');
  await expect(bar.getByRole('button', { name: /^#rocks/ })).toHaveCSS('background-color', 'rgb(125, 180, 234)');
  await expect(p.getByRole('heading', { name: '#rocks' })).toBeVisible();
  // export keeps tags in Moxfield syntax
  await p.getByRole('button', { name: 'Export' }).click();
  await expect(p.getByLabel('Exported deck list')).toHaveValue(/ #!rocks/);
  await expect(p.getByLabel('Exported deck list')).toHaveValue(/ #opener/);
  await p.getByLabel('Include tags').uncheck();
  await expect(p.getByLabel('Exported deck list')).not.toHaveValue(/#!?rocks/);
  await p.keyboard.press('Escape');
  // delete the tag: the cards stay, the tag goes
  await bar.getByRole('button', { name: 'Manage tags…' }).click();
  const row = p.getByRole('dialog').locator('div.row', { hasText: '#rocks' }).first();
  await row.getByRole('button', { name: 'Delete' }).click();
  await p.getByRole('button', { name: 'Delete tag' }).click();
  await expect(p.getByRole('dialog').getByText('#rocks')).toHaveCount(0);
  await p.keyboard.press('Escape');
  await expect(tiles.locator('.tagc', { hasText: '#rocks' })).toHaveCount(0);
  await pickOption(p, 'Group by', 'Type');
});

test('tags: a global tag follows the card into another deck; recent tags come first', async () => {
  const p = alice.page;
  const decks = await api(alice, srv, 'get', 'api/decks');
  const src = await api(alice, srv, 'get', `api/decks/${decks.find((d: { name: string }) => d.name === 'Atraxa (imported)').id}`);
  const sol = src.cards.find((c: { def: string }) => src.defs[c.def].name === 'Sol Ring');
  await api(alice, srv, 'post', `api/decks/${src.id}/cards/tags`, { cardIds: [sol.id], add: ['staple'] });
  const id = (await api(alice, srv, 'post', 'api/decks', { name: 'Tag follower' })).id;
  await api(alice, srv, 'post', `api/decks/${id}/cards`, { key: sol.def, section: 'main' });
  await p.goto(srv.url + 'decks/' + id);
  await expect(p.locator('.dtile .tagc', { hasText: '#staple' })).toBeVisible();
  // the card dialog offers recent tags first, the newest being the one just used
  await p.locator('.dtile').first().click();
  const chips = p.getByRole('dialog').locator('button.tagc');
  await expect(chips.first()).toHaveText('#staple');
  expect(await chips.count()).toBeLessThanOrEqual(13);
  await p.keyboard.press('Escape');
  await api(alice, srv, 'delete', `api/decks/${id}`);
});

test('friends can view and clone but not edit; pins sort first', async () => {
  const decks = await api(bob, srv, 'get', 'api/decks');
  const id = decks[0].id;
  await bob.page.goto(srv.url + `decks/${id}`);
  await expect(bob.page.getByRole('button', { name: 'Clone' })).toBeVisible();
  await expect(bob.page.getByLabel('Search cards to add')).toHaveCount(0);
  await bob.page.getByRole('button', { name: 'Clone' }).click();
  await expect(bob.page.getByLabel('Search cards to add')).toBeVisible();
  await bob.page.goto(srv.url + 'decks');
  await bob.page.getByRole('button', { name: /Pin as favourite/ }).first().click();
  await bob.page.getByRole('button', { name: /Pinned/ }).click();
  await expect(bob.page.getByText('1 deck')).toBeVisible();
});

test('custom cards show up in search with a custom tag', async () => {
  const p = bob.page;
  await p.goto(srv.url + 'decks');
  await p.getByRole('button', { name: /Custom cards/ }).click();
  await p.getByRole('button', { name: '+ New custom card' }).click();
  await p.getByLabel('Name', { exact: true }).fill('Grovetender Hydra');
  await p.getByLabel('Cost, e.g. {2}{G}{G}').fill('{3}{G}{G}');
  await p.getByRole('button', { name: 'Save' }).click();
  await expect(p.getByText(/by bob/)).toBeVisible();
  const r = await api(alice, srv, 'get', 'api/cards/search?q=grovetender');
  expect(r.cards[0].custom).toBe(true);
});

test('deleting a deck from the editor asks first', async () => {
  const p = alice.page;
  const id = await makeDeck(alice, srv, 'Doomed deck');
  await p.goto(srv.url + 'decks/' + id);
  await p.getByRole('button', { name: 'Delete', exact: true }).click();
  await expect(p.getByText('Delete this deck?')).toBeVisible();
  // Cancel is focused, so Enter doesn't delete.
  await p.keyboard.press('Enter');
  await expect(p.getByText('Delete this deck?')).toHaveCount(0);
  await expect(p.getByRole('heading', { name: 'Doomed deck' })).toBeVisible();
  await p.getByRole('button', { name: 'Delete', exact: true }).click();
  await p.getByRole('button', { name: 'Delete deck' }).last().click();
  await expect(p).toHaveURL(/\/decks$/);
  expect((await p.request.get(srv.url + 'api/decks/' + id)).status()).toBe(404);
});

test('admin settings panes render and backups can be taken', async () => {
  const p = alice.page;
  for (const pane of ['profile', 'views', 'hotkeys', 'sounds', 'users', 'cards', 'server', 'backups']) {
    await p.goto(srv.url + `settings/${pane}`);
    await expect(p.locator('main h2').first()).toBeVisible();
  }
  await p.getByRole('button', { name: 'Back up now' }).click();
  await expect(p.getByText(/Manual backup by alice/)).toBeVisible();
  // accent colour applies immediately
  await p.goto(srv.url + 'settings/profile');
  await p.getByRole('button', { name: 'Green' }).click();
  await expect.poll(() => p.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue('--acc').trim())).toBe('#4fae5c');
  expect(alice.errors).toEqual([]);
  expect(bob.errors).toEqual([]);
});

test('admins can rename users and set passwords; users can rename themselves', async ({ browser }) => {
  const created = await api(alice, srv, 'post', 'api/admin/users', { username: 'dora' });
  const p = alice.page;
  await p.goto(srv.url + 'settings/users');
  const row = p.locator('div.row', { has: p.getByText('dora', { exact: true }) }).last();
  await row.getByRole('button', { name: 'Edit' }).click();
  await p.getByLabel('Username', { exact: true }).fill('dorothy');
  await p.getByLabel('New password', { exact: true }).fill('secret12');
  await p.getByLabel('Repeat new password').fill('secret99');
  await expect(p.getByText("The passwords don't match.")).toBeVisible();
  await expect(p.getByRole('button', { name: 'Save' })).toBeDisabled();
  await p.getByLabel('Repeat new password').fill('secret12');
  await p.getByRole('button', { name: 'Save' }).click();
  await expect(p.getByText('dorothy', { exact: true })).toBeVisible();

  // The old name and the temporary password no longer work; the new ones do.
  const ctx = await browser.newContext();
  expect((await ctx.request.post(srv.url + 'api/auth/login', { data: { username: 'dora', password: created.tempPassword } })).status()).toBe(401);
  expect((await ctx.request.post(srv.url + 'api/auth/login', { data: { username: 'dorothy', password: 'secret12' } })).ok()).toBe(true);

  // Dorothy renames herself from Settings; a wrong password is refused.
  const page = await ctx.newPage();
  await page.goto(srv.url + 'settings/profile');
  await page.getByLabel('Username', { exact: true }).fill('dot');
  await page.getByLabel('Password to confirm').fill('wrong-pass');
  await page.getByRole('button', { name: 'Change username' }).click();
  await expect(page.getByText('Your password is wrong')).toBeVisible();
  await page.getByLabel('Password to confirm').fill('secret12');
  await page.getByRole('button', { name: 'Change username' }).click();
  await expect(page.getByText("You're now dot.")).toBeVisible();
  // Names are unique (case-insensitive).
  expect((await ctx.request.put(srv.url + 'api/me/username', { data: { username: 'ALICE', password: 'secret12' } })).status()).toBe(409);
  expect((await (await ctx.request.get(srv.url + 'api/me')).json()).username).toBe('dot');
  await ctx.close();
});
