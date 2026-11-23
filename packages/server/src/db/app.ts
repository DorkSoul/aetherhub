import fs from 'node:fs';
import path from 'node:path';
import { config, dataPath } from '../config';
import { MIGRATIONS } from './migrations';
import { all, get, openDb, run, type DB } from './sqlite';

export interface BackupInfo {
  file: string;
  reason: string;
  bytes: number;
  at: number;
}

const KEEP_BACKUPS = 5;

export class AppDb {
  db: DB;
  file: string;

  constructor(file = dataPath('app.sqlite')) {
    this.file = file;
    fs.mkdirSync(path.dirname(file), { recursive: true });
    const existed = fs.existsSync(file);
    this.db = openDb(file);
    this.migrate(existed);
  }

  private currentVersion(): number {
    this.db.exec('CREATE TABLE IF NOT EXISTS schema_migrations (id INTEGER PRIMARY KEY, name TEXT NOT NULL, applied_at INTEGER NOT NULL)');
    const r = get<{ v: number | null }>(this.db, 'SELECT MAX(id) AS v FROM schema_migrations');
    return r?.v ?? 0;
  }

  migrate(existed: boolean) {
    const cur = this.currentVersion();
    const pending = MIGRATIONS.filter((m) => m.id > cur);
    if (!pending.length) return;
    if (existed && cur > 0) {
      const last = pending[pending.length - 1];
      this.backup(`Before migration ${String(cur).padStart(4, '0')} → ${String(last.id).padStart(4, '0')} (v${config.version})`);
    }
    for (const m of pending) {
      this.db.exec('BEGIN');
      try {
        this.db.exec(m.sql);
        run(this.db, 'INSERT INTO schema_migrations (id, name, applied_at) VALUES (?, ?, ?)', m.id, m.name, Date.now());
        this.db.exec('COMMIT');
      } catch (e) {
        this.db.exec('ROLLBACK');
        throw e;
      }
    }
  }

  // -------------------------------------------------------------- backups

  static backupDir(): string {
    const d = dataPath('backups');
    fs.mkdirSync(d, { recursive: true });
    return d;
  }

  private static indexFile(): string {
    return path.join(AppDb.backupDir(), 'index.json');
  }

  private static readIndex(): Record<string, { reason: string; at: number }> {
    try {
      return JSON.parse(fs.readFileSync(AppDb.indexFile(), 'utf8'));
    } catch {
      return {};
    }
  }

  backup(reason: string): BackupInfo {
    const d = new Date();
    const pad = (n: number) => String(n).padStart(2, '0');
    let name = `aetherhub_${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}_${pad(d.getHours())}${pad(d.getMinutes())}`;
    const dir = AppDb.backupDir();
    let file = path.join(dir, name + '.sqlite');
    for (let i = 2; fs.existsSync(file); i++) file = path.join(dir, `${name}_${i}.sqlite`);
    name = path.basename(file);
    this.db.exec(`VACUUM INTO '${file.replace(/'/g, "''")}'`);
    const idx = AppDb.readIndex();
    idx[name] = { reason, at: Date.now() };
    fs.writeFileSync(AppDb.indexFile(), JSON.stringify(idx, null, 2));
    AppDb.prune();
    return { file: name, reason, bytes: fs.statSync(file).size, at: idx[name].at };
  }

  static listBackups(): BackupInfo[] {
    const dir = AppDb.backupDir();
    const idx = AppDb.readIndex();
    return fs
      .readdirSync(dir)
      .filter((f) => f.endsWith('.sqlite'))
      .map((f) => {
        const st = fs.statSync(path.join(dir, f));
        return { file: f, reason: idx[f]?.reason || 'Backup', bytes: st.size, at: idx[f]?.at || st.mtimeMs };
      })
      .sort((a, b) => b.at - a.at);
  }

  static prune() {
    const list = AppDb.listBackups();
    const idx = AppDb.readIndex();
    for (const b of list.slice(KEEP_BACKUPS)) {
      fs.rmSync(path.join(AppDb.backupDir(), b.file), { force: true });
      delete idx[b.file];
    }
    fs.writeFileSync(AppDb.indexFile(), JSON.stringify(idx, null, 2));
  }

  /** Replaces the live database with a backup. Caller must make sure no game is running. */
  restore(file: string) {
    if (!/^[\w.-]+\.sqlite$/.test(file)) throw new Error('Bad backup name');
    const src = path.join(AppDb.backupDir(), file);
    if (!fs.existsSync(src)) throw new Error('Backup not found');
    // Stage a private copy first: the safety backup below prunes old backups and may delete `src` itself.
    const staged = this.file + '.restore';
    const rmDb = (f: string) => {
      for (const suffix of ['', '-wal', '-shm']) fs.rmSync(f + suffix, { force: true });
    };
    rmDb(staged);
    fs.copyFileSync(src, staged);
    try {
      const t = openDb(staged);
      try {
        const ok = get<{ quick_check: string }>(t, 'PRAGMA quick_check');
        if (ok?.quick_check !== 'ok') throw new Error(ok?.quick_check || 'check failed');
      } finally {
        t.close();
      }
    } catch (e) {
      rmDb(staged);
      throw new Error(`That backup is not a readable database (${(e as Error).message})`);
    }
    this.backup(`Before restoring ${file}`);
    // Swap the files by renaming, and put the old database back if anything fails, so the server never ends up
    // without a database.
    const old = this.file + '.old';
    rmDb(old);
    this.db.close();
    try {
      for (const suffix of ['-wal', '-shm']) fs.rmSync(this.file + suffix, { force: true });
      fs.renameSync(this.file, old);
      fs.renameSync(staged, this.file);
      this.db = openDb(this.file);
      this.migrate(true);
    } catch (e) {
      try {
        this.db.close();
      } catch {
        /* not open */
      }
      if (fs.existsSync(old)) {
        rmDb(this.file);
        fs.renameSync(old, this.file);
      }
      this.db = openDb(this.file);
      rmDb(staged);
      throw e;
    }
    rmDb(old);
    rmDb(staged);
  }

  exportTo(file: string) {
    fs.rmSync(file, { force: true });
    this.db.exec(`VACUUM INTO '${file.replace(/'/g, "''")}'`);
  }

  // -------------------------------------------------------------- settings

  setting(key: string, fallback = ''): string {
    return get<{ value: string }>(this.db, 'SELECT value FROM server_settings WHERE key = ?', key)?.value ?? fallback;
  }

  setSetting(key: string, value: string) {
    run(this.db, 'INSERT INTO server_settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value', key, value);
  }

  settings(): Record<string, string> {
    return Object.fromEntries(all<{ key: string; value: string }>(this.db, 'SELECT key, value FROM server_settings').map((r) => [r.key, r.value]));
  }
}
