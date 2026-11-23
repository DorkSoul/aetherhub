import type { FastifyInstance } from 'fastify';
import { z } from 'zod';
import type { Me, SetupState, UserPrefs } from '@aetherhub/shared';
import { mergePrefs } from '@aetherhub/shared';
import {
  clearLoginFails,
  clientIp,
  createSession,
  destroySession,
  hashPassword,
  isSecure,
  loginLocked,
  recordHttpWarning,
  recordLoginFail,
  requireUser,
  verifyDummy,
  verifyPassword,
} from '../auth';
import { bad, type Ctx } from '../context';
import { get, run } from '../db/sqlite';
import { isLanIp } from '../lan';

const Creds = z.object({ username: z.string().trim().min(1).max(40), password: z.string().min(1).max(200) });
const NewPw = z.string().min(6, 'Passwords need at least 6 characters').max(200);
const Username = z
  .string()
  .trim()
  .min(2, 'Usernames need at least 2 characters')
  .max(32)
  .regex(/^[\w .'-]+$/, 'Letters, numbers, spaces and . _ - only');

export function prefsOf(ctx: Ctx, userId: number): UserPrefs {
  const r = get<{ json: string }>(ctx.app.db, 'SELECT json FROM user_prefs WHERE user_id = ?', userId);
  return mergePrefs(r ? JSON.parse(r.json) : null);
}

export function meOf(ctx: Ctx, u: { id: number; username: string; isAdmin: boolean }): Me {
  return { id: u.id, username: u.username, isAdmin: u.isAdmin, prefs: prefsOf(ctx, u.id) };
}

export function authRoutes(r: FastifyInstance, ctx: Ctx) {
  const needsAdmin = () => !get(ctx.app.db, 'SELECT 1 FROM users WHERE is_admin = 1 LIMIT 1');

  r.get('/api/setup/state', async (req): Promise<SetupState> => {
    const ip = clientIp(req);
    const lan = isLanIp(ip);
    return { needsAdmin: needsAdmin(), serverName: ctx.app.setting('serverName', 'Aetherhub'), secure: isSecure(req), lan, insecureRemote: !lan && !isSecure(req) };
  });

  r.post('/api/setup/admin', async (req, reply) => {
    if (!needsAdmin()) return reply.code(409).send({ error: 'An admin already exists' });
    const b = z.object({ username: Username, password: NewPw }).parse(req.body);
    const h = await hashPassword(b.password);
    const res = run(ctx.app.db, 'INSERT INTO users (username, pw_hash, is_admin, created_at, last_seen_at) VALUES (?, ?, 1, ?, ?)', b.username, h, Date.now(), Date.now());
    const id = Number(res.lastInsertRowid);
    createSession(ctx.app, req, reply, id);
    return meOf(ctx, { id, username: b.username, isAdmin: true });
  });

  r.post('/api/auth/login', async (req, reply) => {
    const b = Creds.parse(req.body);
    const ip = clientIp(req);
    const wait = loginLocked(ip, b.username);
    if (wait) return reply.code(429).send({ error: `Too many attempts. Try again in ${Math.ceil(wait / 1000)} seconds.` });
    const u = get<{ id: number; username: string; pw_hash: string; is_admin: number }>(ctx.app.db, 'SELECT * FROM users WHERE username = ?', b.username);
    const ok = u ? await verifyPassword(u.pw_hash, b.password) : await verifyDummy(b.password);
    if (!u || !ok) {
      recordLoginFail(ip, b.username);
      const w = loginLocked(ip, b.username);
      if (w) return reply.code(429).send({ error: `Too many attempts. Try again in ${Math.ceil(w / 1000)} seconds.` });
      return reply.code(401).send({ error: 'Wrong username or password.' });
    }
    clearLoginFails(ip, b.username);
    run(ctx.app.db, 'UPDATE users SET last_seen_at = ? WHERE id = ?', Date.now(), u.id);
    recordHttpWarning(ctx.app, req, u.id, u.username);
    createSession(ctx.app, req, reply, u.id);
    return meOf(ctx, { id: u.id, username: u.username, isAdmin: !!u.is_admin });
  });

  r.post('/api/auth/logout', async (req, reply) => {
    destroySession(ctx.app, req, reply);
    return { ok: true };
  });

  r.get('/api/me', { preHandler: requireUser }, async (req) => meOf(ctx, req.user!));

  r.put('/api/me/password', { preHandler: requireUser }, async (req, reply) => {
    const b = z.object({ current: z.string(), next: NewPw }).parse(req.body);
    const u = get<{ pw_hash: string }>(ctx.app.db, 'SELECT pw_hash FROM users WHERE id = ?', req.user!.id)!;
    if (!(await verifyPassword(u.pw_hash, b.current))) return reply.code(400).send({ error: 'Your current password is wrong' });
    run(ctx.app.db, 'UPDATE users SET pw_hash = ? WHERE id = ?', await hashPassword(b.next), req.user!.id);
    return { ok: true };
  });

  // Rename yourself. Needs your password so a signed-in browser left open can't do it.
  r.put('/api/me/username', { preHandler: requireUser }, async (req, reply) => {
    const b = z.object({ username: Username, password: z.string().min(1).max(200) }).parse(req.body);
    const u = get<{ pw_hash: string }>(ctx.app.db, 'SELECT pw_hash FROM users WHERE id = ?', req.user!.id)!;
    if (!(await verifyPassword(u.pw_hash, b.password))) return reply.code(400).send({ error: 'Your password is wrong' });
    if (get(ctx.app.db, 'SELECT 1 FROM users WHERE username = ? AND id != ?', b.username, req.user!.id)) return reply.code(409).send({ error: 'That username is taken' });
    run(ctx.app.db, 'UPDATE users SET username = ? WHERE id = ?', b.username, req.user!.id);
    return { ok: true, username: b.username };
  });

  r.put('/api/me/prefs', { preHandler: requireUser }, async (req) => {
    const patch = z.record(z.unknown()).parse(req.body);
    const cur = prefsOf(ctx, req.user!.id);
    const next = mergePrefs({ ...cur, ...(patch as Partial<UserPrefs>) });
    if (typeof next.panelW !== 'number' || next.panelW < 280 || next.panelW > 560) next.panelW = 340;
    if (JSON.stringify(next).length > 20000) bad('Preferences too large');
    run(ctx.app.db, 'INSERT INTO user_prefs (user_id, json) VALUES (?, ?) ON CONFLICT(user_id) DO UPDATE SET json = excluded.json', req.user!.id, JSON.stringify(next));
    return next;
  });

  r.get('/api/status', { preHandler: requireUser }, async (req) => ctx.hub.status(req.user!.id));

  r.get('/api/users', { preHandler: requireUser }, async () => {
    return ctx.app.db.prepare('SELECT id, username FROM users ORDER BY username').all();
  });
}

export { Username, NewPw };
