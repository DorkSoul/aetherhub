import path from 'node:path';
import type { AdminLogLine, LogCategory, LogLevel } from '@aetherhub/shared';
import { all, get, openDb, run, type DB } from './db/sqlite';

/** Entries older than this are deleted. */
const MAX_AGE_MS = 90 * 24 * 3600_000;
/** At most this many entries are kept (the oldest go first). */
const MAX_ROWS = 200_000;

export interface LogEntry {
  msg: string;
  cat?: LogCategory;
  level?: LogLevel;
  /** who did it (a username, kept as text so the entry outlives the account) */
  user?: string | null;
  ip?: string | null;
  /** extra lines shown when the entry is expanded (a stack trace, what changed) */
  detail?: string | null;
}

/** How the rest of the server writes to the log: a message, or a message with its category, level and user. */
export type LogFn = (msg: string, entry?: Omit<LogEntry, 'msg'>) => void;

export interface LogQuery {
  /** entries older than this id (the next page down) */
  before?: number;
  /** entries newer than this id (new ones since the last poll) */
  after?: number;
  cat?: string;
  /** 'warn' = warnings and errors, 'error' = errors only */
  level?: string;
  user?: string;
  /** text to look for in the message, the detail or the user */
  q?: string;
  limit?: number;
}

export function logDbPath(dataDir: string): string {
  return path.join(dataDir, 'log.sqlite');
}

/**
 * The server's activity log: sign-ins, account changes, decks, lobbies and games, admin actions, card data imports,
 * and errors. Kept in its own database file (not the app database, so backups, exports and restores leave it alone)
 * and echoed to stdout. Writing never throws: a full disk or a locked file must not fail the request being logged.
 */
export class ActivityLog {
  private db: DB | null = null;
  private timer: NodeJS.Timeout | null = null;

  constructor(file: string, private echo = true) {
    try {
      this.db = openDb(file);
      this.db.exec(`
        CREATE TABLE IF NOT EXISTS entries (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          at INTEGER NOT NULL,
          level TEXT NOT NULL,
          cat TEXT NOT NULL,
          user TEXT,
          ip TEXT,
          msg TEXT NOT NULL,
          detail TEXT
        );
        CREATE INDEX IF NOT EXISTS entries_cat ON entries(cat, id);
        CREATE INDEX IF NOT EXISTS entries_user ON entries(user, id);
      `);
      this.prune();
      this.timer = setInterval(() => this.prune(), 6 * 3600_000);
      this.timer.unref();
    } catch (e) {
      this.db = null;
      process.stderr.write(`Activity log unavailable: ${(e as Error).message}\n`);
    }
  }

  add(e: LogEntry) {
    const at = Date.now();
    const level = e.level ?? 'info';
    const cat = e.cat ?? 'server';
    if (this.echo) process.stdout.write(`${new Date(at).toISOString()} ${level.padEnd(5)} [${cat}]${e.user ? ` ${e.user.slice(0, 60)}:` : ''} ${e.msg.slice(0, 2000)}\n`);
    if (!this.db) return;
    try {
      run(this.db, 'INSERT INTO entries (at, level, cat, user, ip, msg, detail) VALUES (?, ?, ?, ?, ?, ?, ?)', at, level, cat, e.user?.slice(0, 60) ?? null, e.ip?.slice(0, 64) ?? null, e.msg.slice(0, 2000), e.detail?.slice(0, 20000) ?? null);
    } catch {
      /* logging must never break the caller */
    }
  }

  /** A LogFn writing to this log. */
  fn(defaults: Omit<LogEntry, 'msg'> = {}): LogFn {
    return (msg, entry) => this.add({ ...defaults, ...entry, msg });
  }

  /** Newest first: one page of entries matching the filters. */
  query(f: LogQuery = {}): AdminLogLine[] {
    if (!this.db) return [];
    const where: string[] = [];
    const args: unknown[] = [];
    const add = (sql: string, ...v: unknown[]) => {
      where.push(sql);
      args.push(...v);
    };
    if (f.before) add('id < ?', f.before);
    if (f.after) add('id > ?', f.after);
    if (f.cat) add('cat = ?', f.cat);
    if (f.level === 'warn') add("level IN ('warn', 'error')");
    if (f.level === 'error') add("level = 'error'");
    if (f.user) add('user = ? COLLATE NOCASE', f.user);
    if (f.q) {
      const like = `%${f.q.replace(/[\\%_]/g, (m) => '\\' + m)}%`;
      add("(msg LIKE ? ESCAPE '\\' OR detail LIKE ? ESCAPE '\\' OR user LIKE ? ESCAPE '\\')", like, like, like);
    }
    const limit = Math.min(Math.max(f.limit ?? 200, 1), 1000);
    return all<AdminLogLine>(this.db, `SELECT id, at, level, cat, user, ip, msg, detail FROM entries ${where.length ? 'WHERE ' + where.join(' AND ') : ''} ORDER BY id DESC LIMIT ${limit}`, ...args);
  }

  /** Usernames that appear in the log, for the filter. */
  users(): string[] {
    if (!this.db) return [];
    return all<{ user: string }>(this.db, 'SELECT DISTINCT user FROM entries WHERE user IS NOT NULL ORDER BY user COLLATE NOCASE LIMIT 500').map((r) => r.user);
  }

  prune() {
    if (!this.db) return;
    try {
      run(this.db, 'DELETE FROM entries WHERE at < ?', Date.now() - MAX_AGE_MS);
      const last = get<{ id: number }>(this.db, 'SELECT MAX(id) AS id FROM entries')?.id ?? 0;
      run(this.db, 'DELETE FROM entries WHERE id <= ?', last - MAX_ROWS);
    } catch {
      /* try again next time */
    }
  }

  close() {
    if (this.timer) clearInterval(this.timer);
    this.timer = null;
    try {
      this.db?.close();
    } catch {
      /* already closed */
    }
    this.db = null;
  }
}
