import crypto from 'node:crypto';
import { hash, verify } from '@node-rs/argon2';
import type { FastifyReply, FastifyRequest } from 'fastify';
import { config } from './config';
import type { AppDb } from './db/app';
import { get, run } from './db/sqlite';
import { isLanIp } from './lan';

export interface SessionUser {
  id: number;
  username: string;
  isAdmin: boolean;
}

declare module 'fastify' {
  interface FastifyRequest {
    user: SessionUser | null;
  }
}

export const COOKIE = 'aetherhub_session';

export function hashPassword(pw: string): Promise<string> {
  return hash(pw, { memoryCost: 19456, timeCost: 2, parallelism: 1 });
}

export function verifyPassword(h: string, pw: string): Promise<boolean> {
  return verify(h, pw).catch(() => false);
}

export function tempPassword(): string {
  const a = 'abcdefghjkmnpqrstuvwxyz23456789';
  const pick = (n: number) => Array.from(crypto.randomBytes(n), (b) => a[b % a.length]).join('');
  return `${pick(4)}-${pick(4)}-${pick(4)}`;
}

function sha(s: string): string {
  return crypto.createHash('sha256').update(s).digest('hex');
}

/** The client's address. `req.ip` already follows X-Forwarded-For from trusted proxies only (`config.trustProxy`);
 *  headers such as CF-Connecting-IP are not used, since any client can send them. */
export function clientIp(req: FastifyRequest): string {
  return req.ip;
}

export function isSecure(req: FastifyRequest): boolean {
  return req.protocol === 'https';
}

export function createSession(app: AppDb, req: FastifyRequest, reply: FastifyReply, userId: number) {
  const token = crypto.randomBytes(32).toString('base64url');
  const exp = Date.now() + config.sessionDays * 86400_000;
  const ip = clientIp(req);
  run(app.db, 'INSERT INTO sessions (token_hash, user_id, expires_at, created_ip, last_ip, user_agent) VALUES (?, ?, ?, ?, ?, ?)', sha(token), userId, exp, ip, ip, String(req.headers['user-agent'] || '').slice(0, 200));
  setCookie(req, reply, token, exp);
}

function setCookie(req: FastifyRequest, reply: FastifyReply, token: string, exp: number) {
  reply.setCookie(COOKIE, token, {
    path: config.basePath,
    httpOnly: true,
    sameSite: 'lax',
    secure: isSecure(req),
    expires: new Date(exp),
  });
}

export function destroySession(app: AppDb, req: FastifyRequest, reply: FastifyReply) {
  const t = req.cookies[COOKIE];
  if (t) run(app.db, 'DELETE FROM sessions WHERE token_hash = ?', sha(t));
  reply.clearCookie(COOKIE, { path: config.basePath });
}

/** Resolves the session cookie. Sliding expiry: refreshed at most once a day. */
export function userFromToken(app: AppDb, token: string | undefined, ip?: string): SessionUser | null {
  if (!token) return null;
  const row = get<{ user_id: number; expires_at: number; username: string; is_admin: number }>(
    app.db,
    'SELECT s.user_id, s.expires_at, u.username, u.is_admin FROM sessions s JOIN users u ON u.id = s.user_id WHERE s.token_hash = ?',
    sha(token),
  );
  if (!row) return null;
  const now = Date.now();
  if (row.expires_at < now) {
    run(app.db, 'DELETE FROM sessions WHERE token_hash = ?', sha(token));
    return null;
  }
  if (row.expires_at - now < (config.sessionDays - 1) * 86400_000) {
    run(app.db, 'UPDATE sessions SET expires_at = ?, last_ip = ? WHERE token_hash = ?', now + config.sessionDays * 86400_000, ip ?? null, sha(token));
    run(app.db, 'UPDATE users SET last_seen_at = ? WHERE id = ?', now, row.user_id);
  }
  return { id: row.user_id, username: row.username, isAdmin: !!row.is_admin };
}

// ---------------------------------------------------------------- login rate limit

/**
 * Failed logins are counted three ways, so neither a new address per attempt nor a new username per attempt gets
 * unlimited guesses:
 *  - address + username: 5 fails lock that pair for a minute (the friendly limit for typos)
 *  - username from any address: 20 fails within 15 minutes lock the username for 15 minutes
 *  - address with any username: 30 fails within 15 minutes lock the address for 15 minutes
 */
const RULES = [
  { key: (ip: string, u: string) => `p|${ip}|${u}`, max: 5, windowMs: 15 * 60_000, lockMs: 60_000 },
  { key: (_ip: string, u: string) => `u|${u}`, max: 20, windowMs: 15 * 60_000, lockMs: 15 * 60_000 },
  { key: (ip: string, _u: string) => `i|${ip}`, max: 30, windowMs: 15 * 60_000, lockMs: 15 * 60_000 },
];
const fails = new Map<string, { n: number; first: number; until: number }>();
let lastSweep = 0;

function sweep(now: number) {
  if (now - lastSweep < 60_000) return;
  lastSweep = now;
  for (const [k, f] of fails) if (f.until <= now && now - f.first > 15 * 60_000) fails.delete(k);
}

/** Milliseconds until this address may try this username again (0 = now). */
export function loginLocked(ip: string, username: string): number {
  const now = Date.now();
  const u = username.toLowerCase();
  let wait = 0;
  for (const r of RULES) {
    const f = fails.get(r.key(ip, u));
    if (f && f.until > now) wait = Math.max(wait, f.until - now);
  }
  return wait;
}

export function recordLoginFail(ip: string, username: string) {
  const now = Date.now();
  const u = username.toLowerCase();
  sweep(now);
  for (const r of RULES) {
    const k = r.key(ip, u);
    let f = fails.get(k);
    if (!f || now - f.first > r.windowMs) f = { n: 0, first: now, until: f?.until ?? 0 };
    f.n++;
    if (f.n >= r.max) {
      f.until = now + r.lockMs;
      f.n = 0;
      f.first = now;
    }
    fails.set(k, f);
  }
}

export function clearLoginFails(ip: string, username: string) {
  fails.delete(RULES[0].key(ip, username.toLowerCase()));
}

/** Checking a password against this when the username doesn't exist makes both cases take as long. */
let dummyHash: Promise<string> | null = null;
export function verifyDummy(pw: string): Promise<boolean> {
  dummyHash ??= hashPassword(crypto.randomBytes(16).toString('hex'));
  return dummyHash.then((h) => verifyPassword(h, pw)).then(() => false);
}

export function recordHttpWarning(app: AppDb, req: FastifyRequest, userId: number, username: string) {
  const ip = clientIp(req);
  if (isSecure(req) || isLanIp(ip)) return;
  run(app.db, 'INSERT INTO http_warnings (user_id, username, ip, at) VALUES (?, ?, ?, ?)', userId, username, ip, Date.now());
}

export async function requireUser(req: FastifyRequest, reply: FastifyReply) {
  if (!req.user) return reply.code(401).send({ error: 'Not signed in' });
}

export async function requireAdmin(req: FastifyRequest, reply: FastifyReply) {
  if (!req.user) return reply.code(401).send({ error: 'Not signed in' });
  if (!req.user.isAdmin) return reply.code(403).send({ error: 'Admins only' });
}
