import fs from 'node:fs';
import path from 'node:path';
import { Readable } from 'node:stream';
import zlib from 'node:zlib';
import { config, dataPath } from './config';
import type { AppDb } from './db/app';
import { CardDb, importBulk, type ImportProgress } from './db/cards';
import { importRulings, type RulingsDb } from './db/rulings';
import { scryfallApi, scryfallFetch } from './scryfall';

const WEEK = 7 * 86400_000;

export class CardDataService {
  progress: ImportProgress | null = null;
  private busy = false;
  private timer: NodeJS.Timeout | null = null;

  constructor(
    private app: AppDb,
    public cards: CardDb,
    public rulings: RulingsDb,
    private log: (msg: string) => void = () => {},
  ) {}

  get importing(): boolean {
    return this.busy;
  }

  status() {
    const s = this.cards.stats();
    return {
      ...s,
      importing: this.busy,
      progress: this.progress,
      lastCheck: parseInt(this.app.setting('carddata.lastCheck', '0'), 10) || null,
      bulkUpdatedAt: this.app.setting('carddata.bulkUpdatedAt', '') || null,
      bulkName: this.app.setting('carddata.bulkName', '') || null,
      nextCheck: (parseInt(this.app.setting('carddata.lastCheck', '0'), 10) || Date.now()) + WEEK,
      lastError: this.app.setting('carddata.lastError', '') || null,
      rulings: this.rulings.count(),
      rulingsUpdatedAt: this.rulings.importedAt(),
    };
  }

  /** Imports from a local file (seed). */
  async importFile(file: string) {
    if (this.busy) throw new Error('An import is already running');
    this.busy = true;
    try {
      const size = fs.statSync(file).size;
      this.progress = { stage: 'importing', rows: 0, bytes: 0, totalBytes: size };
      await this.runImport(fs.createReadStream(file), size);
      this.app.setSetting('carddata.bulkName', path.basename(file));
      this.app.setSetting('carddata.lastCheck', String(Date.now()));
      // A seed dropped into the data volume is no longer needed once imported.
      if (path.resolve(path.dirname(file)) === path.resolve(config.dataDir)) {
        fs.rmSync(file, { force: true });
        this.log(`Removed ${path.basename(file)} after import`);
      }
    } finally {
      this.busy = false;
    }
  }

  private async runImport(stream: Readable, total: number) {
    const next = dataPath('cards.next.sqlite');
    const rows = await importBulk(stream, next, (p) => (this.progress = p), total);
    if (rows < 100) throw new Error(`Import produced only ${rows} cards`);
    this.cards.close();
    for (const s of ['', '-wal', '-shm']) fs.rmSync(this.cards.file + s, { force: true });
    fs.renameSync(next, this.cards.file);
    this.cards.reopen();
    this.progress = { stage: 'done', rows, bytes: total, totalBytes: total };
    this.log(`Card data imported: ${rows} printings`);
  }

  /** Looks up a Scryfall bulk file (via the rate-limited API) and opens a decompressed stream of it. */
  private async openBulk(type: 'default-cards' | 'rulings') {
    const meta = await scryfallApi(`/bulk-data/${type}`, { signal: AbortSignal.timeout(30_000) });
    if (!meta.ok) throw new Error(`Scryfall bulk-data API returned ${meta.status}`);
    const info = (await meta.json()) as { download_uri?: string; jsonl_download_uri?: string; updated_at: string; size?: number; compressed_size?: number };
    // Scryfall has served both a JSON array (download_uri) and gzipped JSONL (jsonl_download_uri).
    const uri = info.jsonl_download_uri || info.download_uri;
    if (!uri) throw new Error('Scryfall bulk-data response has no download link');
    return {
      info,
      uri,
      open: async () => {
        const res = await scryfallFetch(uri);
        if (!res.ok || !res.body) throw new Error(`Download failed (${res.status})`);
        const total = parseInt(res.headers.get('content-length') || '0', 10) || info.compressed_size || info.size || 0;
        let stream: Readable = Readable.fromWeb(res.body as never);
        // fetch only decodes Content-Encoding; a .gz file served as-is needs gunzip here.
        const gz = /\.gz$/.test(new URL(uri).pathname) && !/gzip/i.test(res.headers.get('content-encoding') || '');
        if (gz) stream = stream.pipe(zlib.createGunzip());
        return { stream, total: gz ? 0 : total };
      },
    };
  }

  /** Imports rulings into a fresh file and swaps it in. */
  private async runRulingsImport(stream: Readable) {
    const next = dataPath('rulings.next.sqlite');
    const rows = await importRulings(stream, next);
    if (rows < 100) throw new Error(`Rulings import produced only ${rows} rulings`);
    this.rulings.close();
    for (const s of ['', '-wal', '-shm']) fs.rmSync(this.rulings.file + s, { force: true });
    fs.renameSync(next, this.rulings.file);
    this.rulings.reopen();
    this.log(`Rulings imported: ${rows}`);
  }

  /** Downloads Scryfall's rulings file (weekly, with the card data). Failures are logged, never fatal. */
  async refreshRulings(force = false) {
    try {
      const b = await this.openBulk('rulings');
      if (!force && b.info.updated_at === this.app.setting('rulings.bulkUpdatedAt') && this.rulings.ready) return false;
      const { stream } = await b.open();
      await this.runRulingsImport(stream);
      this.app.setSetting('rulings.bulkUpdatedAt', b.info.updated_at);
      return true;
    } catch (e) {
      this.log('Rulings download failed: ' + (e as Error).message);
      return false;
    }
  }

  /** Imports a local rulings file (seed). */
  async importRulingsFile(file: string) {
    await this.runRulingsImport(fs.createReadStream(file));
    if (path.resolve(path.dirname(file)) === path.resolve(config.dataDir)) fs.rmSync(file, { force: true });
  }

  /** Downloads the current Default Cards bulk file and imports it. */
  async refresh(force = true) {
    if (this.busy) throw new Error('An import is already running');
    this.busy = true;
    this.progress = { stage: 'downloading', rows: 0, bytes: 0, totalBytes: 0 };
    try {
      const b = await this.openBulk('default-cards');
      const info = b.info;
      const uri = b.uri;
      this.app.setSetting('carddata.lastCheck', String(Date.now()));
      if (!force && info.updated_at === this.app.setting('carddata.bulkUpdatedAt') && this.cards.ready) {
        this.progress = null;
        void this.refreshRulings(false);
        return false;
      }
      const { stream, total } = await b.open();
      // Progress is by row count when the byte total refers to compressed data.
      await this.runImport(stream, total);
      this.app.setSetting('carddata.bulkUpdatedAt', info.updated_at);
      this.app.setSetting('carddata.bulkName', path.basename(new URL(uri).pathname));
      this.app.setSetting('carddata.lastError', '');
      this.busy = false;
      await this.refreshRulings(force);
      return true;
    } catch (e) {
      this.progress = { stage: 'error', rows: 0, bytes: 0, totalBytes: 0, message: (e as Error).message };
      this.app.setSetting('carddata.lastError', (e as Error).message);
      throw e;
    } finally {
      this.busy = false;
    }
  }

  /** Finds a seed file: SEED_CARDS, or default-cards-*.json(l) in the data dir or the working directory. */
  findSeed(pattern = /^default-cards.*\.jsonl?$/): string | null {
    if (pattern.source.includes('default-cards') && config.seedFile && fs.existsSync(config.seedFile)) return config.seedFile;
    for (const dir of [config.dataDir, process.cwd()]) {
      try {
        const f = fs
          .readdirSync(dir)
          .filter((x) => pattern.test(x))
          .sort()
          .pop();
        if (f) return path.join(dir, f);
      } catch {
        /* ignore */
      }
    }
    return null;
  }

  /** Called at startup: seed if empty, then check weekly. */
  start() {
    const boot = async () => {
      if (!this.cards.ready) {
        const seed = this.findSeed();
        if (seed) {
          this.log(`No card data yet — importing ${seed}`);
          await this.importFile(seed);
        } else if (!config.noAutoDownload) {
          this.log('No card data yet — downloading from Scryfall');
          await this.refresh(true);
        }
      }
      if (!this.rulings.ready) {
        const seed = this.findSeed(/^rulings.*\.jsonl?$/);
        if (seed) {
          this.log(`No rulings yet — importing ${seed}`);
          await this.importRulingsFile(seed).catch((e) => this.log('Rulings import failed: ' + (e as Error).message));
        } else if (!config.noAutoDownload) await this.refreshRulings(true);
      }
    };
    boot()
      .catch((e) => this.log('Card data import failed: ' + (e as Error).message))
      .finally(() => {
        const tick = () => {
          const last = parseInt(this.app.setting('carddata.lastCheck', '0'), 10) || 0;
          if (!config.noAutoDownload && Date.now() - last > WEEK && !this.busy) this.refresh(false).catch((e) => this.log('Weekly card check failed: ' + e.message));
        };
        tick();
        this.timer = setInterval(tick, 6 * 3600_000);
        this.timer.unref();
      });
  }

  stop() {
    if (this.timer) clearInterval(this.timer);
  }
}
