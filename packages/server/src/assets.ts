import crypto from 'node:crypto';
import type { LookupAddress, LookupOptions } from 'node:dns';
import dns from 'node:dns/promises';
import fs from 'node:fs';
import http from 'node:http';
import https from 'node:https';
import net from 'node:net';
import path from 'node:path';
import sharp from 'sharp';
import { config, dataPath } from './config';
import type { AppDb } from './db/app';
import { get, run } from './db/sqlite';
import { isPrivateTarget } from './lan';
import { scryfallFetch } from './scryfall';

export type AssetKind = 'playmat' | 'card' | 'override' | 'sleeve';

const MAX_DIM: Record<AssetKind, number> = { playmat: 2400, card: 1000, override: 1000, sleeve: 800 };

export function uploadsDir(): string {
  const d = dataPath('uploads');
  fs.mkdirSync(d, { recursive: true });
  return d;
}

export function assetFile(sha: string): string {
  return path.join(uploadsDir(), sha + '.webp');
}

/** Re-encodes any image to WebP, dedupes by content hash and records it. Returns the asset id. */
export async function storeImage(app: AppDb, input: Buffer, kind: AssetKind, ownerId: number | null): Promise<number> {
  if (input.length > config.uploadLimit) throw new Error('Images are limited to 10 MB');
  let img = sharp(input, { failOn: 'error', limitInputPixels: 80_000_000 }).rotate();
  const meta = await img.metadata();
  if (!meta.width || !meta.height) throw new Error('Not an image');
  const max = MAX_DIM[kind];
  if (meta.width > max || meta.height > max) img = img.resize({ width: max, height: max, fit: 'inside' });
  const out = await img.webp({ quality: 86 }).toBuffer({ resolveWithObject: true });
  const sha = crypto.createHash('sha256').update(out.data).digest('hex');
  const existing = get<{ id: number }>(app.db, 'SELECT id FROM assets WHERE sha256 = ?', sha);
  if (existing) return existing.id;
  fs.writeFileSync(assetFile(sha), out.data);
  const r = run(app.db, 'INSERT INTO assets (sha256, kind, bytes, width, height, owner_id, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)', sha, kind, out.data.length, out.info.width, out.info.height, ownerId, Date.now());
  return Number(r.lastInsertRowid);
}

export function assetPath(app: AppDb, id: number): string | null {
  const row = get<{ sha256: string }>(app.db, 'SELECT sha256 FROM assets WHERE id = ?', id);
  if (!row) return null;
  const f = assetFile(row.sha256);
  return fs.existsSync(f) ? f : null;
}

/** dns.lookup for http(s).get that refuses private / reserved addresses. The connection uses the address checked
 *  here, so a DNS answer that changes between a check and the connect (DNS rebinding) can't reach the LAN. */
function guardedLookup(hostname: string, options: LookupOptions, cb: (err: Error | null, address?: string | LookupAddress[], family?: number) => void) {
  dns.lookup(hostname, { all: true, family: options.family }).then(
    (addrs) => {
      if (!addrs.length || addrs.some((a) => isPrivateTarget(a.address))) return cb(new Error('That address is not allowed'));
      if (options.all) cb(null, addrs);
      else cb(null, addrs[0].address, addrs[0].family);
    },
    (e: Error) => cb(e),
  );
}

function getGuarded(u: URL): Promise<http.IncomingMessage> {
  return new Promise((resolve, reject) => {
    const mod = u.protocol === 'https:' ? https : http;
    const req = mod.get(u, { lookup: guardedLookup as unknown as net.LookupFunction, headers: { 'User-Agent': config.userAgent, Accept: 'image/*' }, timeout: 20_000 }, resolve);
    req.on('timeout', () => req.destroy(new Error('Download timed out')));
    req.on('error', reject);
  });
}

async function readCapped(body: AsyncIterable<Uint8Array>): Promise<Buffer> {
  const chunks: Uint8Array[] = [];
  let total = 0;
  for await (const value of body) {
    total += value.length;
    if (total > config.uploadLimit) throw new Error('Images are limited to 10 MB');
    chunks.push(value);
  }
  return Buffer.concat(chunks);
}

/** Fetches a user-supplied image URL with an SSRF guard and a size cap. */
export async function fetchImageUrl(url: string): Promise<Buffer> {
  let u: URL;
  try {
    u = new URL(url);
  } catch {
    throw new Error('Not a valid URL');
  }
  if (u.protocol !== 'https:' && u.protocol !== 'http:') throw new Error('Only http(s) URLs are allowed');
  const host = u.hostname.replace(/^\[|\]$/g, '');
  if (net.isIP(host) && isPrivateTarget(host)) throw new Error('That address is not allowed');
  const check = (status: number, type: string, len: number) => {
    if (status < 200 || status >= 300) throw new Error(`Download failed (${status})`);
    if (!type.startsWith('image/')) throw new Error('The URL is not an image');
    if (len > config.uploadLimit) throw new Error('Images are limited to 10 MB');
  };
  // Scryfall URLs (e.g. picking another printing's art) go through the shared backoff / header rules.
  if (/(^|\.)scryfall\.(com|io)$/i.test(host)) {
    const res = await scryfallFetch(u.href, { redirect: 'error', accept: 'image/*', signal: AbortSignal.timeout(20_000) });
    check(res.status, res.headers.get('content-type') || '', parseInt(res.headers.get('content-length') || '0', 10));
    return readCapped(res.body as unknown as AsyncIterable<Uint8Array>);
  }
  const res = await getGuarded(u);
  const overall = setTimeout(() => res.destroy(new Error('Download timed out')), 20_000);
  try {
    check(res.statusCode ?? 0, String(res.headers['content-type'] || ''), parseInt(String(res.headers['content-length'] || '0'), 10));
    return await readCapped(res);
  } finally {
    clearTimeout(overall);
    res.destroy();
  }
}
