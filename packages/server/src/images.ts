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
  /** queued by "prefetch every image" (dropped when it is paused) */
  prefetch?: boolean;
}

/** Progress of "prefetch every image": `total` images were missing when it started, `done` of them are handled. */
export interface PrefetchState {
  running: boolean;
  /** stopped by an admin before it finished */
  paused: boolean;
  total: number;
  done: number;
  /** of `done`: downloaded now (the rest Scryfall had no image for, or the download failed) */
  downloaded: number;
  failed: number;
  /** bytes downloaded by this run */
  bytes: number;
  /** images that were already cached when it started */
  cached: number;
}

/** On-disk cache of Scryfall card images with a polite download queue. */
export class ImageCache {
  private queue: Job[] = [];
  private inflight = new Map<string, Promise<string | null>>();
  /** the queued job behind each in-flight key, so a player's request can take over a prefetch download */
  private jobs = new Map<string, Job>();
  private running = 0;
  private lastStart = 0;
  private stats: { files: number; bytes: number } | null = null;
  prefetch: PrefetchState = { running: false, paused: false, total: 0, done: 0, downloaded: 0, failed: 0, bytes: 0, cached: 0 };
  private cancelPrefetch = false;
  /** told when "prefetch every image" ends (finished, paused or cleared), for the activity log */
  onPrefetchEnd: (s: PrefetchState) => void = () => {};

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
  get(id: string, face: number, size: ImgSize, priority = true, prefetch = false): Promise<string | null> {
    if (!/^[0-9a-f-]{36}$/.test(id) || face < 0 || face > 1) return Promise.resolve(null);
    const f = this.file(id, face, size);
    if (fs.existsSync(f)) return Promise.resolve(f);
    if (config.imageOffline) return Promise.resolve(null);
    const key = `${id}-${face}-${size}`;
    const existing = this.inflight.get(key);
    if (existing) {
      // a player wants an image the prefetch has queued: it is theirs now (pausing won't drop it), and it goes first
      const j = this.jobs.get(key);
      if (j && !prefetch && j.prefetch) {
        j.prefetch = false;
        const i = this.queue.indexOf(j);
        if (priority && i > 0) this.queue.unshift(...this.queue.splice(i, 1));
      }
      return existing;
    }
    const p = new Promise<string | null>((resolve) => {
      const job: Job = { id, face, size, resolve, tries: 0, prefetch };
      this.jobs.set(key, job);
      if (priority) this.queue.unshift(job);
      else this.queue.push(job);
    });
    this.inflight.set(key, p);
    p.finally(() => {
      this.inflight.delete(key);
      this.jobs.delete(key);
    });
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
    if (job.prefetch) this.prefetch.bytes += buf.length;
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
    for (const j of this.queue) j.resolve(null);
    this.queue = [];
    fs.rmSync(dataPath('images'), { recursive: true, force: true });
    this.stats = { files: 0, bytes: 0 };
    this.prefetch = { running: false, paused: false, total: 0, done: 0, downloaded: 0, failed: 0, bytes: 0, cached: 0 };
  }

  /**
   * Downloads the normal-size image of every listed card face that isn't cached yet, at low priority (players'
   * requests go first). `ids` are [printing id, face count] pairs. Stops when paused or cleared.
   */
  async prefetchAll(ids: string[][]) {
    if (this.prefetch.running) return;
    this.cancelPrefetch = false;
    const missing: [string, number][] = [];
    let cached = 0;
    for (const [id, faces] of ids) {
      const n = Math.min(2, parseInt(faces, 10) || 1);
      for (let f = 0; f < n; f++) {
        if (fs.existsSync(this.file(id, f, 'normal'))) cached++;
        else missing.push([id, f]);
      }
    }
    this.prefetch = { running: true, paused: false, total: missing.length, done: 0, downloaded: 0, failed: 0, bytes: 0, cached };
    const state = this.prefetch;
    const pending: Promise<unknown>[] = [];
    try {
      for (const [id, face] of missing) {
        if (this.cancelPrefetch) break;
        // Keep the queue shallow so interactive requests stay fast.
        while (this.queue.length > 50 && !this.cancelPrefetch) await new Promise((r) => setTimeout(r, 200));
        if (this.cancelPrefetch) break;
        pending.push(
          this.get(id, face, 'normal', false, true).then((f) => {
            // a download dropped by pausing isn't done: resuming fetches it
            if (this.prefetch !== state || (!f && this.cancelPrefetch)) return;
            state.done++;
            if (f) state.downloaded++;
            else state.failed++;
          }),
        );
      }
      await Promise.all(pending);
    } finally {
      state.running = false;
      state.paused = this.cancelPrefetch && state.done < state.total;
      this.onPrefetchEnd(state);
    }
  }

  /** Pauses "prefetch every image": nothing new is queued and its queued downloads are dropped. Images already
   * downloaded stay; starting it again carries on with the ones still missing. */
  pausePrefetch() {
    if (!this.prefetch.running) return;
    this.cancelPrefetch = true;
    const keep: Job[] = [];
    for (const j of this.queue) {
      if (j.prefetch) j.resolve(null);
      else keep.push(j);
    }
    this.queue = keep;
  }
}
