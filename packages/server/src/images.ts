import fs from 'node:fs';
import path from 'node:path';
import { config, dataPath } from './config';
import { backoffRemaining, ScryfallBackoffError, scryfallFetch } from './scryfall';

const IMG_HOST = 'cards.scryfall.io';
/** Gap between download starts. cards.scryfall.io has no rate limit; this just keeps us a polite neighbour. */
const SPACING_MS = 60;

export type ImgSize = 'normal' | 'large';

interface Job {
  id: string;
  face: number;
  size: ImgSize;
  resolve: (f: string | null) => void;
  tries: number;
}

/** On-disk cache of Scryfall card images with a polite download queue. */
export class ImageCache {
  private queue: Job[] = [];
  private inflight = new Map<string, Promise<string | null>>();
  private running = 0;
  private lastStart = 0;
  private stats: { files: number; bytes: number } | null = null;
  prefetch = { running: false, done: 0, total: 0 };
  private cancelPrefetch = false;

  dir(size: ImgSize): string {
    return dataPath('images', size);
  }

  file(id: string, face: number, size: ImgSize): string {
    return path.join(this.dir(size), id.slice(0, 2), `${id}-${face}.jpg`);
  }

  static url(id: string, face: number, size: ImgSize): string {
    return `https://cards.scryfall.io/${size}/${face ? 'back' : 'front'}/${id[0]}/${id[1]}/${id}.jpg`;
  }

  /** Returns the cached file path, downloading on a miss. Null when Scryfall has no such image. */
  get(id: string, face: number, size: ImgSize, priority = true): Promise<string | null> {
    if (!/^[0-9a-f-]{36}$/.test(id) || face < 0 || face > 1) return Promise.resolve(null);
    const f = this.file(id, face, size);
    if (fs.existsSync(f)) return Promise.resolve(f);
    if (config.imageOffline) return Promise.resolve(null);
    const key = `${id}-${face}-${size}`;
    const existing = this.inflight.get(key);
    if (existing) return existing;
    const p = new Promise<string | null>((resolve) => {
      const job = { id, face, size, resolve, tries: 0 };
      if (priority) this.queue.unshift(job);
      else this.queue.push(job);
    });
    this.inflight.set(key, p);
    p.finally(() => this.inflight.delete(key));
    this.pump();
    return p;
  }

  queued(): number {
    return this.queue.length + this.running;
  }

  private resumeTimer: NodeJS.Timeout | null = null;

  private pump() {
    // Scryfall asked us to back off: start nothing until the wait is over, then carry on with the same queue.
    const blocked = backoffRemaining(IMG_HOST);
    if (blocked > 0) {
      if (!this.resumeTimer && this.queue.length) {
        this.resumeTimer = setTimeout(() => {
          this.resumeTimer = null;
          this.pump();
        }, blocked + 50);
        this.resumeTimer.unref?.();
      }
      return;
    }
    while (this.running < config.imageConcurrency && this.queue.length) {
      const job = this.queue.shift()!;
      this.running++;
      const wait = Math.max(0, this.lastStart + SPACING_MS - Date.now());
      this.lastStart = Date.now() + wait;
      setTimeout(() => {
        this.download(job)
          .then((f) => job.resolve(f))
          .catch((e) => {
            // A backoff isn't the image's fault: put it back at the front without using up a retry.
            if (e instanceof ScryfallBackoffError) return void this.queue.unshift(job);
            job.tries++;
            if (job.tries < 3) setTimeout(() => (this.queue.push(job), this.pump()), 1000 * 2 ** job.tries);
            else job.resolve(null);
          })
          .finally(() => {
            this.running--;
            this.pump();
          });
      }, wait);
    }
  }

  private async download(job: Job): Promise<string | null> {
    const res = await scryfallFetch(ImageCache.url(job.id, job.face, job.size), { accept: 'image/*', signal: AbortSignal.timeout(30_000) });
    if (res.status === 404) return null;
    if (!res.ok) throw new Error('HTTP ' + res.status);
    const buf = Buffer.from(await res.arrayBuffer());
    const f = this.file(job.id, job.face, job.size);
    fs.mkdirSync(path.dirname(f), { recursive: true });
    const tmp = f + '.tmp' + process.pid;
    fs.writeFileSync(tmp, buf);
    fs.renameSync(tmp, f);
    if (this.stats) {
      this.stats.files++;
      this.stats.bytes += buf.length;
    }
    return f;
  }

  /** Queues images for a list of image refs (`p/<id>/<face>`), both sizes, at low priority. */
  warm(refs: string[]) {
    for (const r of refs) {
      const m = r.match(/^p\/([0-9a-f-]{36})\/(\d)$/);
      if (!m) continue;
      this.get(m[1], +m[2], 'normal', false);
      this.get(m[1], +m[2], 'large', false);
    }
  }

  cacheStats(): { files: number; bytes: number } {
    if (this.stats) return this.stats;
    let files = 0;
    let bytes = 0;
    const walk = (d: string) => {
      if (!fs.existsSync(d)) return;
      for (const e of fs.readdirSync(d, { withFileTypes: true })) {
        const p = path.join(d, e.name);
        if (e.isDirectory()) walk(p);
        else if (e.name.endsWith('.jpg')) {
          files++;
          bytes += fs.statSync(p).size;
        }
      }
    };
    walk(dataPath('images'));
    this.stats = { files, bytes };
    return this.stats;
  }

  clear() {
    this.cancelPrefetch = true;
    this.queue = [];
    fs.rmSync(dataPath('images'), { recursive: true, force: true });
    this.stats = { files: 0, bytes: 0 };
  }

  async prefetchAll(ids: string[][]) {
    if (this.prefetch.running) return;
    this.cancelPrefetch = false;
    this.prefetch = { running: true, done: 0, total: ids.length };
    try {
      for (const [id, faces] of ids) {
        if (this.cancelPrefetch) break;
        const n = parseInt(faces, 10) || 1;
        const jobs: Promise<unknown>[] = [];
        for (let f = 0; f < n; f++) jobs.push(this.get(id, f, 'normal', false));
        // Keep the queue shallow so interactive requests stay fast.
        while (this.queue.length > 50) await new Promise((r) => setTimeout(r, 200));
        void Promise.all(jobs).then(() => this.prefetch.done++);
      }
    } finally {
      this.prefetch.running = false;
    }
  }
}
