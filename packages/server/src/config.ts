import fs from 'node:fs';
import path from 'node:path';

function defaultDataDir(): string {
  if (process.env.DATA_DIR) return path.resolve(process.env.DATA_DIR);
  try {
    if (fs.existsSync('/data') && fs.statSync('/data').isDirectory()) return '/data';
  } catch {
    /* ignore */
  }
  return path.resolve('data');
}

function normBase(b: string | undefined): string {
  let s = (b || '/').trim();
  if (!s.startsWith('/')) s = '/' + s;
  if (!s.endsWith('/')) s += '/';
  return s;
}

/**
 * Whose X-Forwarded-For / X-Forwarded-Proto to believe. By default only proxies on loopback or a private network
 * (a tunnel client such as Newt or cloudflared on the Docker host), so a client on the internet can't choose its
 * own IP address (that would get around the login limits). `false` trusts nobody; `true` trusts any peer (only
 * safe when nothing but the proxy can reach the port); anything else is a comma-separated list of IPs / CIDRs.
 */
function trustProxy(v: string | undefined): boolean | string {
  const s = (v ?? '').trim();
  if (!s) return 'loopback,linklocal,uniquelocal';
  if (s === 'false') return false;
  if (s === 'true') return true;
  return s;
}

export const config = {
  port: parseInt(process.env.PORT || '8080', 10),
  host: process.env.HOST || '0.0.0.0',
  basePath: normBase(process.env.BASE_PATH),
  dataDir: defaultDataDir(),
  trustProxy: trustProxy(process.env.TRUST_PROXY),
  userAgent: process.env.SCRYFALL_UA || 'Aetherhub/1.0 (self-hosted MTG table)',
  seedFile: process.env.SEED_CARDS || '',
  /** Skip the automatic bulk download when no card DB exists (tests, offline dev). */
  noAutoDownload: process.env.NO_AUTO_DOWNLOAD === '1',
  /** Never contact Scryfall for images (tests / offline). */
  imageOffline: process.env.IMAGE_OFFLINE === '1',
  imageConcurrency: parseInt(process.env.IMAGE_CONCURRENCY || '4', 10),
  clientDir: process.env.CLIENT_DIR || '',
  /** Directory of rules-engine card definitions (one JSON file per card). */
  rulesCards: process.env.RULES_CARDS || path.resolve('cards'),
  uploadLimit: 10 * 1024 * 1024,
  heartbeatMs: 25_000,
  sessionDays: 30,
  version: '1.0.4',
};

export function dataPath(...p: string[]): string {
  return path.join(config.dataDir, ...p);
}
