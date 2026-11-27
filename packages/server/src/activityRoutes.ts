import type { FastifyInstance, FastifyRequest } from 'fastify';
import type { LogCategory } from '@aetherhub/shared';
import { clientIp } from './auth';
import { config } from './config';
import type { Ctx } from './context';
import { get } from './db/sqlite';

/** Names looked up before the handler runs, so an entry can still name a deck or user the request deletes. */
interface Names {
  deck?: string;
  card?: string;
  target?: string;
  lobby?: string;
  folder?: string;
  custom?: string;
}

interface RequestLog {
  names: Names;
  /** the error message the response carried */
  error?: string;
  /** stack of a server error */
  stack?: string;
}

declare module 'fastify' {
  interface FastifyRequest {
    activity?: RequestLog;
  }
}

interface Ev {
  body: Record<string, unknown>;
  query: Record<string, string>;
  params: Record<string, string>;
  names: Names;
  status: number;
  /** the signed-in user's name (or the name a sign-in was attempted with) */
  who: string | null;
}

/** A readable line for one kind of request: the message, or null to leave it out of the log. */
type Describe = (e: Ev) => string | null;

const q = (s: unknown) => `“${String(s ?? '?')}”`;
const deck = (e: Ev) => q(e.names.deck ?? `deck ${e.params.id}`);
const lobby = (e: Ev) => q(e.names.lobby ?? 'a lobby');
const fields = (b: Record<string, unknown>, hide: string[] = []) =>
  Object.keys(b)
    .filter((k) => !hide.includes(k))
    .join(', ');

/** Requests that are part of playing (every move, pick and keystroke): too many to log one by one. */
const SKIP = new Set([
  'PUT /api/me/prefs',
  'POST /api/import/parse',
  'POST /api/cards/defs',
  'POST /api/rules/check',
  'POST /api/rules/games/:id/answer',
  'POST /api/rules/games/:id/undo',
  'POST /api/rules/games/:id/undo/respond',
  'POST /api/rules/games/:id/passuntil',
  'POST /api/rules/games/:id/passuntil/cancel',
  'POST /api/rules/games/:id/reactions',
  'DELETE /api/rules/games/:id/reactions/:rid',
  'POST /api/rules/games/:id/autotap',
  'POST /api/rules/games/:id/preview-blocks',
  'POST /api/draft/games/:id/pick',
  'POST /api/draft/games/:id/lands',
  'POST /api/lobbies/:id/chat',
  'POST /api/lobbies/:id/move',
  // solo games autosave every few seconds
  'PUT /api/solo/saves/:id',
]);

const ROUTES: Record<string, [LogCategory, Describe]> = {
  // sign-ins and your own account
  'POST /api/setup/admin': ['auth', (e) => `Created the first admin account ${q(e.body.username)}`],
  'POST /api/auth/login': ['auth', (e) => (e.status === 200 ? 'Signed in' : e.status === 429 ? 'Sign-in blocked: too many failed attempts' : 'Failed sign-in: wrong username or password')],
  'POST /api/auth/logout': ['auth', () => 'Signed out'],
  'PUT /api/me/password': ['users', () => 'Changed their password'],
  'PUT /api/me/username': ['users', (e) => `Renamed themselves to ${q(e.body.username)}`],

  // admin: accounts
  'POST /api/admin/users': ['users', (e) => `Created user ${q(e.body.username)}${e.body.admin ? ' (admin)' : ''}`],
  'POST /api/admin/users/:id/reset': ['users', (e) => `Reset the password of ${q(e.names.target)}`],
  'PUT /api/admin/users/:id': ['users', (e) => `Changed user ${q(e.names.target)}: ${[e.body.username && `renamed to ${q(e.body.username)}`, e.body.password && 'set a new password'].filter(Boolean).join(', ')}`],
  'POST /api/admin/users/:id/role': ['users', (e) => `Made ${q(e.names.target)} ${e.body.admin ? 'an admin' : 'a regular user'}`],
  'DELETE /api/admin/users/:id': ['users', (e) => `Deleted user ${q(e.names.target)} (their decks and custom cards were ${e.query.mode === 'delete' ? 'deleted' : 'transferred'})`],

  // admin: server
  'POST /api/admin/warnings/dismiss': ['admin', () => 'Dismissed the insecure-connection warnings'],
  'PUT /api/admin/settings': ['admin', (e) => `Changed server settings (${fields(e.body)})`],
  'POST /api/admin/carddata/refresh': ['admin', () => 'Started a card data refresh'],
  'PUT /api/admin/precons': ['admin', (e) => (e.body.enabled ? 'Added the preconstructed decks' : 'Removed the preconstructed decks')],
  'POST /api/admin/images/prefetch': ['admin', () => 'Started prefetching every card image'],
  'POST /api/admin/images/clear': ['admin', () => 'Cleared the image cache'],
  'POST /api/admin/backups': ['admin', () => 'Made a backup'],
  'POST /api/admin/backups/:file/restore': ['admin', (e) => `Restored the backup ${q(e.params.file)}`],
  'GET /api/admin/export': ['admin', () => 'Downloaded a copy of the database'],

  // decks
  'POST /api/decks': ['decks', (e) => `Created deck ${q(e.body.name ?? 'New deck')}`],
  'PUT /api/decks/:id': ['decks', (e) => (e.body.name && e.body.name !== e.names.deck ? `Renamed deck ${deck(e)} to ${q(e.body.name)}` : `Changed deck ${deck(e)} (${fields(e.body)})`)],
  'DELETE /api/decks/:id': ['decks', (e) => `Deleted deck ${deck(e)}`],
  'POST /api/decks/:id/cards': ['decks', (e) => `Added ${e.names.card ? q(e.names.card) : 'a card'} to ${deck(e)}${e.body.section && e.body.section !== 'main' ? ` (${e.body.section})` : ''}`],
  'POST /api/decks/:id/cards/bulk': ['decks', (e) => `Changed several cards in ${deck(e)}`],
  'PATCH /api/decks/:id/cards/:cardId': ['decks', (e) => `Changed ${q(e.names.card ?? 'a card')} in ${deck(e)} (${fields(e.body)})`],
  'DELETE /api/decks/:id/cards/:cardId': ['decks', (e) => `Removed ${q(e.names.card ?? 'a card')} from ${deck(e)}`],
  'POST /api/decks/:id/cards/tags': ['decks', (e) => `Tagged cards in ${deck(e)}`],
  'PUT /api/decks/:id/tags': ['decks', (e) => `Edited the tags of ${deck(e)}`],
  'POST /api/decks/:id/tags/rename': ['decks', (e) => `Renamed a tag in ${deck(e)}`],
  'POST /api/decks/:id/tags/scope': ['decks', (e) => `Changed a tag's scope in ${deck(e)}`],
  'POST /api/decks/:id/tags/delete': ['decks', (e) => `Deleted a tag in ${deck(e)}`],
  'POST /api/decks/:id/clone': ['decks', (e) => `Copied deck ${deck(e)} into their decks`],
  'POST /api/decks/:id/pin': ['decks', (e) => `${e.body.pinned ? 'Pinned' : 'Unpinned'} deck ${deck(e)}`],
  'POST /api/decks/:id/override-all': ['decks', (e) => `Set a card image for all their decks (from ${deck(e)})`],
  'POST /api/decks/:id/proxy': ['decks', (e) => `Made a proxy PDF of ${deck(e)}`],
  'POST /api/import/commit': ['decks', (e) => `Imported deck ${q(e.body.name)} (${Array.isArray(e.body.cards) ? e.body.cards.length : 0} entries)`],
  'POST /api/folders': ['decks', (e) => `Created folder ${q(e.body.name)}`],
  'PUT /api/folders/:id': ['decks', (e) => `Renamed folder ${q(e.names.folder)} to ${q(e.body.name)}`],
  'DELETE /api/folders/:id': ['decks', (e) => `Deleted folder ${q(e.names.folder)}`],
  'POST /api/custom-cards': ['decks', (e) => `Created custom card ${q(e.body.name)}`],
  'PUT /api/custom-cards/:id': ['decks', (e) => `Edited custom card ${q(e.names.custom)}`],
  'DELETE /api/custom-cards/:id': ['decks', (e) => `Deleted custom card ${q(e.names.custom)}`],
  'POST /api/assets': ['decks', () => 'Uploaded an image'],
  'POST /api/assets/url': ['decks', () => 'Added an image from a link'],

  // lobbies
  'POST /api/lobbies': ['lobby', (e) => `Opened lobby ${q((e.body.settings as { name?: string } | undefined)?.name ?? 'a new lobby')}`],
  'POST /api/lobbies/:id/join': ['lobby', (e) => `Joined lobby ${lobby(e)}`],
  'POST /api/lobbies/:id/leave': ['lobby', (e) => `Left lobby ${lobby(e)}`],
  'PUT /api/lobbies/:id/settings': ['lobby', (e) => `Changed the settings of lobby ${lobby(e)} (${fields(e.body, ['password']) || 'password'})`],
  'POST /api/lobbies/:id/seats/:seat': ['lobby', (e) => (e.body.ai ? `Added a ${e.body.ai} AI to seat ${+e.params.seat + 1} in ${lobby(e)}` : e.names.target ? `Seated ${q(e.names.target)} in seat ${+e.params.seat + 1} of ${lobby(e)}` : `Sat in seat ${+e.params.seat + 1} of ${lobby(e)}`)],
  'DELETE /api/lobbies/:id/seats/:seat': ['lobby', (e) => `Cleared seat ${+e.params.seat + 1} of ${lobby(e)}`],
  'PUT /api/lobbies/:id/seats/:seat/deck': ['lobby', (e) => (e.names.deck ? `Chose deck ${q(e.names.deck)} for seat ${+e.params.seat + 1} in ${lobby(e)}` : `Cleared the deck of seat ${+e.params.seat + 1} in ${lobby(e)}`)],
  'PUT /api/lobbies/:id/seats/:seat/difficulty': ['lobby', (e) => `Set the AI in seat ${+e.params.seat + 1} of ${lobby(e)} to ${e.body.difficulty}`],
  'PUT /api/lobbies/:id/ready': ['lobby', (e) => `${e.body.ready ? 'Ready' : 'Not ready'} in ${lobby(e)}`],
  'POST /api/lobbies/:id/reroll': ['lobby', (e) => `Shuffled the seats of ${lobby(e)}`],
  'POST /api/lobbies/:id/kick': ['lobby', (e) => `Removed ${q(e.names.target)} from ${lobby(e)}`],
  'POST /api/lobbies/:id/event-game': ['game', (e) => `Started an event game in ${lobby(e)}`],
  'POST /api/lobbies/:id/start': ['game', (e) => `Started the game in lobby ${lobby(e)}`],

  // games
  'POST /api/games/:id/back-to-lobby': ['lobby', () => 'Went back to the lobby after a game'],
  'POST /api/games/:id/leave-lobby': ['lobby', () => 'Left the lobby after a game'],
  'POST /api/games/:id/load': ['game', () => 'Loaded a suspended game'],
  'DELETE /api/games/:id': ['game', () => 'Deleted a suspended game'],
  // a started game is logged with its players by the rules routes; only a refused start is logged here
  'POST /api/rules/games': ['game', (e) => (e.status < 400 ? null : 'Started a rules game')],
  'POST /api/rules/games/:id/join': ['game', () => 'Joined a rules game'],
  'POST /api/rules/games/:id/break': ['game', () => 'Broke the rules in a rules game'],
  'POST /api/rules/games/:id/edit': ['game', () => 'Changed the board in a rules game (breaking the rules)'],
  'POST /api/rules/games/:id/concede': ['game', () => 'Conceded a rules game'],
  'POST /api/rules/games/:id/save': ['game', () => 'Saved a rules game'],
  'POST /api/rules/saves/:id/resume': ['game', () => 'Resumed a saved rules game'],
  'DELETE /api/rules/saves/:id': ['game', () => 'Deleted a saved rules game'],
  'DELETE /api/rules/games/:id': ['game', () => 'Ended a rules game'],
  'POST /api/solo/saves': ['game', (e) => `Saved a solo game${e.body.name ? ` as ${q(e.body.name)}` : ''}`],
  'PATCH /api/solo/saves/:id': ['game', (e) => `Renamed a solo save to ${q(e.body.name)}`],
  'DELETE /api/solo/saves/:id': ['game', () => 'Deleted a solo save'],
  'POST /api/draft/games/:id/deck': ['game', (e) => `Built their draft deck${e.body.name ? ` ${q(e.body.name)}` : ''}`],
};

const MUTATING = new Set(['POST', 'PUT', 'PATCH', 'DELETE']);

/** Fastify hooks that write one activity log entry per state-changing request (and per failed one). */
export function registerActivityLog(f: FastifyInstance, ctx: Ctx) {
  const prefix = config.basePath.replace(/\/$/, '');
  const routeOf = (req: FastifyRequest) => {
    const url = req.routeOptions.url ?? '';
    return url.startsWith(prefix + '/api/') ? url.slice(prefix.length) : url;
  };
  const deckName = (id: number) => (id < 0 ? ctx.decks.precons.contents(id)?.name : get<{ name: string }>(ctx.app.db, 'SELECT name FROM decks WHERE id = ?', id)?.name);
  const userName = (id: unknown) => get<{ username: string }>(ctx.app.db, 'SELECT username FROM users WHERE id = ?', Number(id))?.username;

  f.addHook('preHandler', async (req) => {
    if (!MUTATING.has(req.method) && req.method !== 'GET') return;
    const route = routeOf(req);
    if (!route.startsWith('/api/') || SKIP.has(`${req.method} ${route}`)) return;
    const names: Names = {};
    req.activity = { names };
    if (!MUTATING.has(req.method)) return;
    const p = (req.params ?? {}) as Record<string, string>;
    const b = (req.body && typeof req.body === 'object' ? req.body : {}) as Record<string, unknown>;
    try {
      if (route.startsWith('/api/decks/:id')) {
        names.deck = deckName(Number(p.id));
        if (p.cardId) {
          const key = get<{ card_key: string }>(ctx.app.db, 'SELECT card_key FROM deck_cards WHERE id = ?', Number(p.cardId))?.card_key;
          if (key) names.card = ctx.decks.resolveDefs([key])[key]?.name;
        } else if (typeof b.key === 'string') names.card = ctx.decks.resolveDefs([b.key])[b.key]?.name;
      }
      if (route.startsWith('/api/admin/users/:id')) names.target = userName(p.id);
      if (route.startsWith('/api/lobbies/:id')) {
        names.lobby = ctx.hub.lobbies.get(p.id)?.settings.name;
        if (typeof b.userId === 'number' && b.userId !== req.user?.id) names.target = userName(b.userId);
        if (typeof b.deckId === 'number') names.deck = deckName(b.deckId);
      }
      if (route.startsWith('/api/folders/:id')) names.folder = get<{ name: string }>(ctx.app.db, 'SELECT name FROM folders WHERE id = ?', Number(p.id))?.name;
      if (route.startsWith('/api/custom-cards/:id')) names.custom = get<{ name: string }>(ctx.app.db, 'SELECT name FROM custom_cards WHERE id = ?', Number(p.id))?.name;
    } catch {
      /* names are a nicety */
    }
  });

  f.addHook('onSend', async (req, reply, payload) => {
    if (req.activity && !req.activity.error && reply.statusCode >= 400 && typeof payload === 'string') {
      try {
        req.activity.error = (JSON.parse(payload) as { error?: string }).error;
      } catch {
        /* not JSON */
      }
    }
    return payload;
  });

  f.addHook('onResponse', async (req, reply) => {
    const a = req.activity;
    if (!a) return;
    const status = reply.statusCode;
    const route = routeOf(req);
    const key = `${req.method} ${route}`;
    const known = ROUTES[key];
    // reads are only logged when they fail on the server's side (or are listed, like the database download)
    if (req.method === 'GET' && !known && status < 500) return;
    const body = (req.body && typeof req.body === 'object' ? req.body : {}) as Record<string, unknown>;
    // a sign-in is about the account being tried, whoever this browser is already signed in as
    const signIn = route === '/api/auth/login' || route === '/api/setup/admin';
    const who = (signIn && typeof body.username === 'string' ? body.username : null) ?? req.user?.username ?? null;
    const ev: Ev = { body, query: (req.query ?? {}) as Record<string, string>, params: (req.params ?? {}) as Record<string, string>, names: a.names, status, who };
    let msg: string | null;
    try {
      msg = known ? known[1](ev) : `${req.method} ${route}`;
    } catch {
      msg = `${req.method} ${route}`;
    }
    if (msg == null) return;
    const failed = status >= 400 && key !== 'POST /api/auth/login';
    if (failed) msg += ` (failed, ${status}${a.error ? `: ${a.error}` : ''})`;
    ctx.log(msg, {
      cat: known?.[0] ?? 'request',
      level: status >= 500 ? 'error' : status >= 400 ? 'warn' : 'info',
      user: who,
      ip: clientIp(req),
      detail: a.stack ?? null,
    });
  });
}
