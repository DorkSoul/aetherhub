import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import type { FastifyInstance } from 'fastify';
import { z } from 'zod';
import { FORMAT_BY_ID } from '@aetherhub/shared';
import { hashPassword, requireAdmin, tempPassword } from '../auth';
import { config } from '../config';
import { bad, notFound, type Ctx } from '../context';
import { AppDb } from '../db/app';
import { all, get, run, tx } from '../db/sqlite';
import { NewPw, Username } from './auth';

export function adminRoutes(r: FastifyInstance, ctx: Ctx) {
  const opts = { preHandler: requireAdmin };

  r.get('/api/admin/users', opts, async () => {
    return all(
      ctx.app.db,
      `SELECT u.id, u.username, u.is_admin AS isAdmin, u.created_at AS createdAt, u.last_seen_at AS lastSeenAt,
        (SELECT COUNT(*) FROM decks d WHERE d.owner_id = u.id) AS decks,
        (SELECT COUNT(*) FROM custom_cards c WHERE c.creator_id = u.id) AS customCards
       FROM users u ORDER BY u.username`,
    ).map((u) => ({ ...u, isAdmin: !!u.isAdmin, online: ctx.hub.isConnected(u.id as number) }));
  });

  r.post('/api/admin/users', opts, async (req, reply) => {
    const b = z.object({ username: Username, admin: z.boolean().optional() }).parse(req.body);
    if (get(ctx.app.db, 'SELECT 1 FROM users WHERE username = ?', b.username)) return reply.code(409).send({ error: 'That username is taken' });
    const pw = tempPassword();
    const res = run(ctx.app.db, 'INSERT INTO users (username, pw_hash, is_admin, created_at) VALUES (?, ?, ?, ?)', b.username, await hashPassword(pw), b.admin ? 1 : 0, Date.now());
    return { id: Number(res.lastInsertRowid), username: b.username, tempPassword: pw };
  });

  r.post('/api/admin/users/:id/reset', opts, async (req) => {
    const id = Number((req.params as { id: string }).id);
    if (!get(ctx.app.db, 'SELECT 1 FROM users WHERE id = ?', id)) notFound('No such user');
    const pw = tempPassword();
    run(ctx.app.db, 'UPDATE users SET pw_hash = ? WHERE id = ?', await hashPassword(pw), id);
    run(ctx.app.db, 'DELETE FROM sessions WHERE user_id = ?', id);
    return { tempPassword: pw };
  });

  // Admins can rename anyone and set a password of their choosing (e.g. one agreed in person).
  r.put('/api/admin/users/:id', opts, async (req, reply) => {
    const id = Number((req.params as { id: string }).id);
    const b = z.object({ username: Username.optional(), password: NewPw.optional() }).parse(req.body);
    if (!get(ctx.app.db, 'SELECT 1 FROM users WHERE id = ?', id)) notFound('No such user');
    if (b.username && get(ctx.app.db, 'SELECT 1 FROM users WHERE username = ? AND id != ?', b.username, id)) return reply.code(409).send({ error: 'That username is taken' });
    const pwHash = b.password ? await hashPassword(b.password) : null;
    tx(ctx.app.db, () => {
      if (b.username) run(ctx.app.db, 'UPDATE users SET username = ? WHERE id = ?', b.username, id);
      if (pwHash) {
        run(ctx.app.db, 'UPDATE users SET pw_hash = ? WHERE id = ?', pwHash, id);
        // A new password signs the user out everywhere, except the admin changing their own.
        if (id !== req.user!.id) run(ctx.app.db, 'DELETE FROM sessions WHERE user_id = ?', id);
      }
    });
    return { ok: true };
  });

  r.post('/api/admin/users/:id/role', opts, async (req) => {
    const id = Number((req.params as { id: string }).id);
    const b = z.object({ admin: z.boolean() }).parse(req.body);
    if (!b.admin) {
      const admins = get<{ n: number }>(ctx.app.db, 'SELECT COUNT(*) AS n FROM users WHERE is_admin = 1')!.n;
      const isAdm = get<{ is_admin: number }>(ctx.app.db, 'SELECT is_admin FROM users WHERE id = ?', id)?.is_admin;
      if (isAdm && admins <= 1) bad('The last admin cannot be demoted');
    }
    run(ctx.app.db, 'UPDATE users SET is_admin = ? WHERE id = ?', b.admin ? 1 : 0, id);
    return { ok: true };
  });

  r.delete('/api/admin/users/:id', opts, async (req) => {
    const id = Number((req.params as { id: string }).id);
    const mode = (req.query as { mode?: string }).mode === 'delete' ? 'delete' : 'transfer';
    if (id === req.user!.id) bad("You can't delete yourself");
    const u = get<{ username: string; is_admin: number }>(ctx.app.db, 'SELECT username, is_admin FROM users WHERE id = ?', id);
    if (!u) notFound('No such user');
    tx(ctx.app.db, () => {
      if (mode === 'transfer') {
        run(ctx.app.db, 'UPDATE decks SET owner_id = ?, folder_id = NULL WHERE owner_id = ?', req.user!.id, id);
        run(ctx.app.db, 'UPDATE custom_cards SET creator_id = ? WHERE creator_id = ?', req.user!.id, id);
      } else {
        run(ctx.app.db, 'DELETE FROM decks WHERE owner_id = ?', id);
        run(ctx.app.db, 'DELETE FROM custom_cards WHERE creator_id = ?', id);
      }
      run(ctx.app.db, 'DELETE FROM users WHERE id = ?', id);
    });
    return { ok: true };
  });

  r.get('/api/admin/warnings', opts, async () => all(ctx.app.db, 'SELECT id, username, ip, at FROM http_warnings WHERE dismissed = 0 ORDER BY at DESC LIMIT 20'));
  r.post('/api/admin/warnings/dismiss', opts, async () => {
    run(ctx.app.db, 'UPDATE http_warnings SET dismissed = 1');
    return { ok: true };
  });

  r.get('/api/admin/settings', opts, async () => ({
    serverName: ctx.app.setting('serverName', 'Aetherhub'),
    defaultFormat: ctx.app.setting('defaultFormat', 'commander'),
    defaultMaxPlayers: parseInt(ctx.app.setting('defaultMaxPlayers', '2'), 10),
    env: { port: config.port, basePath: config.basePath, dataDir: config.dataDir, heartbeatSeconds: config.heartbeatMs / 1000, uploadLimitMb: config.uploadLimit / 1048576, version: config.version, node: process.version, host: os.hostname() },
  }));

  r.put('/api/admin/settings', opts, async (req) => {
    const b = z
      .object({ serverName: z.string().trim().min(1).max(60).optional(), defaultFormat: z.string().optional(), defaultMaxPlayers: z.number().int().min(2).max(8).optional() })
      .parse(req.body);
    if (b.serverName) ctx.app.setSetting('serverName', b.serverName);
    if (b.defaultFormat && FORMAT_BY_ID[b.defaultFormat]) ctx.app.setSetting('defaultFormat', b.defaultFormat);
    if (b.defaultMaxPlayers) ctx.app.setSetting('defaultMaxPlayers', String(b.defaultMaxPlayers));
    ctx.hub.broadcastStatus();
    return { ok: true };
  });

  r.get('/api/admin/games', opts, async () => ctx.hub.adminGames());

  r.get('/api/admin/log', opts, async (req) => {
    const f = z
      .object({
        before: z.coerce.number().int().min(0).optional(),
        after: z.coerce.number().int().min(0).optional(),
        cat: z.string().max(20).optional(),
        level: z.enum(['warn', 'error']).optional(),
        user: z.string().max(60).optional(),
        q: z.string().max(200).optional(),
        limit: z.coerce.number().int().min(1).max(1000).optional(),
      })
      .parse(req.query);
    return { entries: ctx.activity.query(f), users: ctx.activity.users() };
  });

  r.get('/api/admin/carddata', opts, async () => ({ ...ctx.carddata.status(), cache: ctx.images.cacheStats(), prefetch: ctx.images.prefetch, queued: ctx.images.queued() }));

  r.post('/api/admin/carddata/refresh', opts, async () => {
    if (ctx.carddata.importing) bad('An import is already running');
    ctx.carddata
      .refresh(true)
      .then(() => ctx.hub.broadcastStatus())
      .catch((e) => ctx.log('Card data refresh failed: ' + e.message, { cat: 'cards', level: 'error' }));
    return { ok: true };
  });

  r.get('/api/admin/precons', opts, async () => ({ enabled: ctx.decks.precons.enabled(), available: ctx.decks.precons.available() }));

  r.put('/api/admin/precons', opts, async (req) => {
    const b = z.object({ enabled: z.boolean() }).parse(req.body);
    if (b.enabled && !ctx.decks.precons.available()) bad('This server has no preconstructed deck list');
    ctx.decks.precons.setEnabled(b.enabled);
    if (b.enabled) ctx.decks.precons.list().catch((e) => ctx.log('Preconstructed decks: ' + e.message, { level: 'warn' }));
    ctx.hub.broadcastStatus();
    return { ok: true };
  });

  r.post('/api/admin/images/prefetch', opts, async () => {
    if (!ctx.cards.db) bad('No card data yet');
    if (config.imageOffline) bad('Image downloads are turned off on this server (IMAGE_OFFLINE)');
    const ids = ctx.cards.db
      .prepare('SELECT p.id, p.image_faces FROM oracles o JOIN printings p ON p.id = o.default_id WHERE o.paper = 1')
      .all()
      .map((x) => [String(x.id), String(x.image_faces)]);
    void ctx.images.prefetchAll(ids);
    // the missing images are counted before prefetchAll first waits, so the totals are ready here
    return { ok: true, total: ctx.images.prefetch.total, cached: ctx.images.prefetch.cached };
  });

  r.post('/api/admin/images/prefetch/pause', opts, async () => {
    ctx.images.pausePrefetch();
    return { ok: true };
  });

  r.post('/api/admin/images/clear', opts, async () => {
    ctx.images.clear();
    return { ok: true };
  });

  r.get('/api/admin/backups', opts, async () => AppDb.listBackups());

  r.post('/api/admin/backups', opts, async (req) => ctx.app.backup(`Manual backup by ${req.user!.username}`));

  r.post('/api/admin/backups/:file/restore', opts, async (req) => {
    if (ctx.hub.rooms.size) bad('Suspend or end the running games first');
    const file = (req.params as { file: string }).file;
    ctx.app.restore(file);
    return { ok: true };
  });

  r.get('/api/admin/export', opts, async (req, reply) => {
    const tmp = path.join(os.tmpdir(), `aetherhub-export-${Date.now()}.sqlite`);
    ctx.app.exportTo(tmp);
    const buf = fs.readFileSync(tmp);
    fs.rmSync(tmp, { force: true });
    const d = new Date().toISOString().slice(0, 10);
    return reply
      .header('Content-Type', 'application/vnd.sqlite3')
      .header('Content-Disposition', `attachment; filename="aetherhub-${d}.sqlite"`)
      .send(buf);
  });
}
