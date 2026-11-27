import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { afterAll, beforeEach, describe, expect, it, vi } from 'vitest';

const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'aetherhub-images-'));
process.env.DATA_DIR = dir;
delete process.env.IMAGE_OFFLINE;

const { ImageCache } = await import('./images');
const { SetCatalog } = await import('./sets');
const { _resetScryfall } = await import('./scryfall');

const id = (n: number) => `${String(n).padStart(8, '0')}-0000-4000-8000-000000000000`;
let requests: string[] = [];
let inFlight = 0;
let maxInFlight = 0;

beforeEach(() => {
  _resetScryfall();
  requests = [];
  inFlight = 0;
  maxInFlight = 0;
  vi.stubGlobal('fetch', async (url: string) => {
    requests.push(String(url));
    inFlight++;
    maxInFlight = Math.max(maxInFlight, inFlight);
    await new Promise((r) => setTimeout(r, 20));
    inFlight--;
    if (String(url).endsWith('.svg')) return new Response('<svg/>', { status: 200 });
    return new Response(Buffer.alloc(1000), { status: 200, headers: { 'content-type': 'image/jpeg' } });
  });
});

afterAll(() => {
  vi.unstubAllGlobals();
  fs.rmSync(dir, { recursive: true, force: true });
});

const until = async (ok: () => boolean) => {
  for (let i = 0; i < 300 && !ok(); i++) await new Promise((r) => setTimeout(r, 20));
};

describe('prefetching every image', () => {
  it('counts only the missing images, and reports what it downloaded', async () => {
    const c = new ImageCache();
    fs.mkdirSync(path.dirname(c.file(id(1), 0, 'normal')), { recursive: true });
    fs.writeFileSync(c.file(id(1), 0, 'normal'), 'cached');
    await c.prefetchAll([
      [id(1), '1'],
      [id(2), '2'],
      [id(3), '1'],
    ]);
    expect(c.prefetch).toMatchObject({ running: false, paused: false, total: 3, done: 3, downloaded: 3, failed: 0, bytes: 3000, cached: 1 });
    expect(requests).toHaveLength(3);
  });

  it('pauses at once (queued downloads are dropped), and resuming fetches only what is still missing', async () => {
    const c = new ImageCache();
    const ids = Array.from({ length: 40 }, (_, i) => [id(100 + i), '1']);
    const run = c.prefetchAll(ids);
    await until(() => c.prefetch.done >= 3);
    c.pausePrefetch();
    await run;
    expect(c.prefetch.paused).toBe(true);
    const got = c.prefetch.downloaded;
    expect(got).toBeLessThan(40);
    expect(c.prefetch.failed).toBe(0);
    await c.prefetchAll(ids);
    expect(c.prefetch).toMatchObject({ paused: false, total: 40 - got, cached: got });
    expect(c.prefetch.downloaded).toBe(40 - got);
  });

  it("keeps a player's request for an image the prefetch had queued when it is paused", async () => {
    const c = new ImageCache();
    const ids = Array.from({ length: 30 }, (_, i) => [id(200 + i), '1']);
    const run = c.prefetchAll(ids);
    const wanted = c.get(id(229), 0, 'normal');
    c.pausePrefetch();
    await run;
    expect(await wanted).toBe(c.file(id(229), 0, 'normal'));
  });
});

describe('set icons', () => {
  it('are downloaded one at a time and only once each, however many are asked for at once', async () => {
    const app = { setting: () => '', setSetting: () => {} };
    const sets = new SetCatalog(app as never, {} as never);
    const meta = (sets as unknown as { meta: Map<string, { icon_svg_uri: string }> }).meta;
    for (const code of ['aaa', 'bbb', 'ccc']) meta.set(code, { icon_svg_uri: `https://svgs.scryfall.io/sets/${code}.svg` });
    const icons = await Promise.all(['aaa', 'bbb', 'ccc', 'aaa', 'bbb'].map((code) => sets.icon(code)));
    expect(icons.every((s) => s === '<svg/>')).toBe(true);
    expect(requests.filter((u) => u.endsWith('.svg'))).toHaveLength(3);
    expect(maxInFlight).toBe(1);
  });
});
