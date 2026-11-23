// Every request to Scryfall goes through here, so its rules hold in one place
// (https://scryfall.com/docs/api/rate-limits):
//  - api.scryfall.com: at most 10 requests/second (2/second for card search/named/random/collection).
//    Calls are serialised and spaced 500 ms apart, which covers even the strictest endpoints.
//  - *.scryfall.io (images, bulk files) has no rate limit, but callers still pace themselves.
//  - A 429 limits access for 30 seconds and must never be ignored: after one, no request to that host
//    is sent until the Retry-After time (at least 30 s) has passed.
//  - Every request carries an accurate User-Agent and an Accept header.
import { config } from './config';

const API_SPACING_MS = 500;
const MIN_BACKOFF_MS = 30_000;

/** Per-host time before which no request may be sent (after a 429 / 503). */
const blockedUntil = new Map<string, number>();
let apiChain: Promise<unknown> = Promise.resolve();
let lastApiCall = 0;

export class ScryfallBackoffError extends Error {
  constructor(public host: string, public until: number) {
    super(`Scryfall asked us to slow down — waiting until ${new Date(until).toLocaleTimeString()} before contacting ${host} again`);
  }
}

/** Milliseconds until requests to this host are allowed again (0 = now). */
export function backoffRemaining(host: string): number {
  return Math.max(0, (blockedUntil.get(host) ?? 0) - Date.now());
}

function retryAfterMs(res: Response): number {
  const h = res.headers.get('retry-after');
  const secs = h ? parseInt(h, 10) : NaN;
  if (Number.isFinite(secs)) return Math.max(MIN_BACKOFF_MS, secs * 1000);
  const date = h ? Date.parse(h) : NaN;
  if (Number.isFinite(date)) return Math.max(MIN_BACKOFF_MS, date - Date.now());
  return MIN_BACKOFF_MS;
}

/**
 * fetch() for Scryfall hosts. Adds the required headers, refuses to send while the host is backing off,
 * and starts a backoff when Scryfall answers 429 (or 503, its overload signal).
 */
export async function scryfallFetch(url: string, init: RequestInit & { accept?: string } = {}): Promise<Response> {
  const host = new URL(url).host;
  const wait = backoffRemaining(host);
  if (wait > 0) throw new ScryfallBackoffError(host, Date.now() + wait);
  const { accept, headers, ...rest } = init;
  const res = await fetch(url, { ...rest, headers: { ...(headers as Record<string, string>), 'User-Agent': config.userAgent, Accept: accept || '*/*' } });
  if (res.status === 429 || res.status === 503) {
    const until = Date.now() + retryAfterMs(res);
    blockedUntil.set(host, Math.max(until, blockedUntil.get(host) ?? 0));
    void res.body?.cancel();
    throw new ScryfallBackoffError(host, until);
  }
  return res;
}

/** A call to api.scryfall.com: serialised and spaced so the API rate limits can never be exceeded. */
export function scryfallApi(path: string, init: RequestInit = {}): Promise<Response> {
  const run = async () => {
    const wait = lastApiCall + API_SPACING_MS - Date.now();
    if (wait > 0) await new Promise((r) => setTimeout(r, wait));
    lastApiCall = Date.now();
    return scryfallFetch(`https://api.scryfall.com${path}`, { accept: 'application/json', ...init });
  };
  const p = apiChain.then(run, run);
  apiChain = p.catch(() => undefined);
  return p;
}

/** Test hook. */
export function _resetScryfall() {
  blockedUntil.clear();
  apiChain = Promise.resolve();
  lastApiCall = 0;
}
