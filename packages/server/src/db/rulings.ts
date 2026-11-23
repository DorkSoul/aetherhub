import fs from 'node:fs';
import path from 'node:path';
import readline from 'node:readline';
import type { Readable } from 'node:stream';
import { all, get, openDb, run, type DB } from './sqlite';

export interface Ruling {
  oracleId: string;
  source: string;
  date: string;
  comment: string;
}

const SCHEMA = `
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE rulings (oracle_id TEXT NOT NULL, source TEXT, published_at TEXT, comment TEXT NOT NULL);
`;
const POST = `
CREATE INDEX r_oracle ON rulings(oracle_id, published_at);
CREATE VIRTUAL TABLE rulings_fts USING fts5(comment, content='rulings', content_rowid='rowid', tokenize='unicode61 remove_diacritics 2');
INSERT INTO rulings_fts(rowid, comment) SELECT rowid, comment FROM rulings;
`;

/** Streams Scryfall's rulings bulk file (JSON array or JSONL) into a new rulings DB file. */
export async function importRulings(input: Readable, outFile: string, onRows?: (n: number) => void): Promise<number> {
  for (const s of ['', '-wal', '-shm']) fs.rmSync(outFile + s, { force: true });
  const db = openDb(outFile);
  db.exec('PRAGMA journal_mode = OFF; PRAGMA synchronous = OFF;');
  db.exec(SCHEMA);
  const ins = db.prepare('INSERT INTO rulings VALUES (?, ?, ?, ?)');
  let rows = 0;
  db.exec('BEGIN');
  for await (let line of readline.createInterface({ input, crlfDelay: Infinity })) {
    line = line.trim();
    if (!line || line === '[' || line === ']') continue;
    if (line.startsWith('[')) line = line.slice(1);
    if (line.endsWith(',')) line = line.slice(0, -1);
    if (line.endsWith(']')) line = line.slice(0, -1);
    let r: { oracle_id?: string; source?: string; published_at?: string; comment?: string };
    try {
      r = JSON.parse(line);
    } catch {
      continue;
    }
    if (!r.oracle_id || !r.comment) continue;
    ins.run(r.oracle_id, r.source || '', r.published_at || '', r.comment);
    if (++rows % 10000 === 0) {
      db.exec('COMMIT; BEGIN');
      onRows?.(rows);
    }
  }
  db.exec('COMMIT');
  db.exec(POST);
  run(db, 'INSERT INTO meta VALUES (?, ?)', 'imported_at', String(Date.now()));
  db.exec('PRAGMA journal_mode = WAL; ANALYZE; VACUUM;');
  db.close();
  return rows;
}

/** Turns free text into a safe FTS5 query: every word must appear (prefix match on the last one). */
function ftsQuery(q: string): string | null {
  // Split like the index does (apostrophes break words: "Urza's" → urza + s), then drop 1-letter bits.
  const words = q
    .toLowerCase()
    .split(/[^\p{L}\p{N}]+/u)
    .filter((w) => w.length > 1);
  if (!words.length) return null;
  return words.map((w, i) => `"${w}"${i === words.length - 1 ? '*' : ''}`).join(' ');
}

export class RulingsDb {
  private db: DB | null = null;

  constructor(public file: string) {
    this.reopen();
  }

  reopen() {
    this.db?.close();
    this.db = null;
    if (!fs.existsSync(this.file)) return;
    try {
      const db = openDb(this.file);
      get(db, 'SELECT 1 FROM rulings LIMIT 1');
      this.db = db;
    } catch {
      this.db = null;
    }
  }

  close() {
    this.db?.close();
    this.db = null;
  }

  get ready(): boolean {
    return !!this.db;
  }

  count(): number {
    return this.db ? get<{ n: number }>(this.db, 'SELECT COUNT(*) AS n FROM rulings')!.n : 0;
  }

  forOracle(oracleId: string): Ruling[] {
    if (!this.db) return [];
    return all<{ oracle_id: string; source: string; published_at: string; comment: string }>(
      this.db,
      'SELECT oracle_id, source, published_at, comment FROM rulings WHERE oracle_id = ? ORDER BY published_at, rowid',
      oracleId,
    ).map((r) => ({ oracleId: r.oracle_id, source: r.source, date: r.published_at, comment: r.comment }));
  }

  /** How many rulings each card has (for the given oracle ids). */
  counts(oracleIds: string[]): Record<string, number> {
    if (!this.db || !oracleIds.length) return {};
    const out: Record<string, number> = {};
    for (let i = 0; i < oracleIds.length; i += 500) {
      const ids = oracleIds.slice(i, i + 500);
      for (const r of all<{ oracle_id: string; n: number }>(this.db, `SELECT oracle_id, COUNT(*) AS n FROM rulings WHERE oracle_id IN (${ids.map(() => '?').join(',')}) GROUP BY oracle_id`, ...ids)) out[r.oracle_id] = r.n;
    }
    return out;
  }

  /** Full-text search over ruling text. Returns matches grouped by card, most matches first. */
  searchText(q: string, limitCards = 40): { oracleId: string; rulings: Ruling[] }[] {
    const fq = this.db && ftsQuery(q);
    if (!this.db || !fq) return [];
    const rows = all<{ oracle_id: string; source: string; published_at: string; comment: string }>(
      this.db,
      `SELECT r.oracle_id, r.source, r.published_at, r.comment FROM rulings_fts f JOIN rulings r ON r.rowid = f.rowid WHERE rulings_fts MATCH ? ORDER BY rank LIMIT 600`,
      fq,
    );
    const by = new Map<string, Ruling[]>();
    for (const r of rows) {
      const list = by.get(r.oracle_id) || [];
      list.push({ oracleId: r.oracle_id, source: r.source, date: r.published_at, comment: r.comment });
      by.set(r.oracle_id, list);
    }
    return [...by.entries()].slice(0, limitCards).map(([oracleId, rulings]) => ({ oracleId, rulings }));
  }

  importedAt(): number | null {
    if (!this.db) return null;
    const v = get<{ value: string }>(this.db, "SELECT value FROM meta WHERE key = 'imported_at'")?.value;
    return v ? parseInt(v, 10) : null;
  }
}

export function rulingsDbPath(dataDir: string): string {
  return path.join(dataDir, 'rulings.sqlite');
}
