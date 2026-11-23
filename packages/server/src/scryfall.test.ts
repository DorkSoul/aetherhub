import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

process.env.DATA_DIR = fs.mkdtempSync(path.join(os.tmpdir(), 'aether-scry-'));

const { _resetScryfall, backoffRemaining, ScryfallBackoffError, scryfallApi, scryfallFetch } = await import('./scryfall');
const { ImageCache } = await import('./images');

const ok = (body = '{}') => new Response(body, { status: 200 });
const tooMany = (retryAfter?: string) => new Response('slow down', { status: 429, headers: retryAfter ? { 'Retry-After': retryAfter } : {} });

describe('Scryfall client', () => {
  let calls: { url: string; at: number; headers: Record<string, string> }[];
  let reply: () => Response;
  beforeEach(() => {
    _resetScryfall();
    calls = [];
    reply = () => ok();
    vi.stubGlobal('fetch', async (url: string | URL, init?: RequestInit) => {
      calls.push({ url: String(url), at: Date.now(), headers: (init?.headers || {}) as Record<string, string> });
      return reply();
    });
  });
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  it('sends a User-Agent and an Accept header', async () => {
    await scryfallApi('/bulk-data/default-cards');
    expect(calls[0].headers['User-Agent']).toMatch(/Aetherhub/);
    expect(calls[0].headers.Accept).toBe('application/json');
  });

  it('spaces API calls at least 500 ms apart, even when fired together', async () => {
    await Promise.all([scryfallApi('/a'), scryfallApi('/b'), scryfallApi('/c')]);
    expect(calls).toHaveLength(3);
    expect(calls[1].at - calls[0].at).toBeGreaterThanOrEqual(490);
    expect(calls[2].at - calls[1].at).toBeGreaterThanOrEqual(490);
  });

  it('after a 429, sends nothing to that host for at least 30 s', async () => {
    reply = () => tooMany();
    await expect(scryfallApi('/bulk-data/default-cards')).rejects.toBeInstanceOf(ScryfallBackoffError);
    expect(backoffRemaining('api.scryfall.com')).toBeGreaterThan(29_000);
    reply = () => ok();
    await expect(scryfallApi('/bulk-data/default-cards')).rejects.toBeInstanceOf(ScryfallBackoffError);
    expect(calls).toHaveLength(1);
    // Other hosts are unaffected.
    await scryfallFetch('https://cards.scryfall.io/normal/front/a/b/x.jpg');
    expect(calls).toHaveLength(2);
  });

  it('honours a Retry-After longer than 30 s', async () => {
    reply = () => tooMany('120');
    await expect(scryfallFetch('https://cards.scryfall.io/x.jpg')).rejects.toBeInstanceOf(ScryfallBackoffError);
    expect(backoffRemaining('cards.scryfall.io')).toBeGreaterThan(119_000);
  });

  it('the image queue pauses on a 429 and resumes the same jobs afterwards', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'Date'] });
    let n = 0;
    reply = () => (++n === 1 ? tooMany() : ok('jpegdata'));
    const cache = new ImageCache();
    const ids = ['0000000a-0000-4000-8000-000000000001', '0000000a-0000-4000-8000-000000000002', '0000000a-0000-4000-8000-000000000003'];
    const got = ids.map((id) => cache.get(id, 0, 'normal'));
    await vi.advanceTimersByTimeAsync(1_000);
    // Only the first request went out (and got the 429); the rest are held.
    expect(calls).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(20_000);
    expect(calls).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(15_000);
    const files = await Promise.all(got);
    expect(files.every((f) => f && fs.existsSync(f))).toBe(true);
    expect(calls).toHaveLength(4);
    expect(calls[1].at - calls[0].at).toBeGreaterThanOrEqual(30_000);
  });
});
