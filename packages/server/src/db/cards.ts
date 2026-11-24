import fs from 'node:fs';
import path from 'node:path';
import readline from 'node:readline';
import type { Readable } from 'node:stream';
import type { CardDef, CardFace, RelatedPart, SearchQuery, SearchTerm } from '@aetherhub/shared';
import { levenshtein, normName, parseColors, parseSearch, RARITY_CODES } from '@aetherhub/shared';
import { all, get, openDb, run, type DB } from './sqlite';

const SCHEMA = `
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS printings (
  id TEXT PRIMARY KEY,
  oracle_id TEXT,
  name TEXT NOT NULL,
  norm_name TEXT NOT NULL,
  front_norm TEXT NOT NULL,
  lang TEXT,
  set_code TEXT,
  set_name TEXT,
  set_type TEXT,
  collector_number TEXT,
  released_at TEXT,
  layout TEXT,
  mana_cost TEXT,
  cmc REAL,
  type_line TEXT,
  oracle_text TEXT,
  power TEXT,
  toughness TEXT,
  loyalty TEXT,
  defense TEXT,
  colors TEXT,
  color_identity TEXT,
  rarity TEXT,
  legalities TEXT,
  usd REAL,
  eur REAL,
  promo INTEGER,
  digital INTEGER,
  paper INTEGER,
  image_status TEXT,
  image_faces INTEGER,
  faces TEXT,
  all_parts TEXT,
  is_token INTEGER,
  nonstd INTEGER
);
`;

const POST = `
CREATE INDEX IF NOT EXISTS p_oracle ON printings(oracle_id, released_at);
CREATE INDEX IF NOT EXISTS p_set ON printings(set_code, collector_number);
CREATE TABLE oracles (
  oracle_id TEXT PRIMARY KEY,
  default_id TEXT NOT NULL,
  name TEXT NOT NULL,
  norm_name TEXT NOT NULL,
  front_norm TEXT NOT NULL,
  layout TEXT,
  mana_cost TEXT,
  cmc REAL,
  type_line TEXT,
  oracle_text TEXT,
  power TEXT,
  toughness TEXT,
  colors TEXT,
  color_identity TEXT,
  rarity TEXT,
  legalities TEXT,
  is_token INTEGER,
  paper INTEGER
);
INSERT INTO oracles
SELECT oracle_id, id, name, norm_name, front_norm, layout, mana_cost, cmc, type_line, oracle_text, power, toughness, colors, color_identity, rarity, legalities, is_token, paper_any
FROM (
  SELECT p.*, MAX(p.paper) OVER (PARTITION BY oracle_id) AS paper_any,
    ROW_NUMBER() OVER (
      PARTITION BY oracle_id
      ORDER BY (lang = 'en') DESC,
        (promo = 0 AND digital = 0 AND paper = 1 AND image_status IN ('highres_scan', 'lowres')) DESC,
        nonstd ASC,
        (set_type NOT IN ('memorabilia', 'funny', 'alchemy')) DESC,
        (image_status IN ('highres_scan', 'lowres')) DESC,
        released_at DESC
    ) AS rn
  FROM printings p
  WHERE oracle_id IS NOT NULL AND layout NOT IN ('art_series', 'vanguard')
) WHERE rn = 1;
CREATE INDEX o_norm ON oracles(norm_name);
CREATE INDEX o_front ON oracles(front_norm);
CREATE VIRTUAL TABLE oracles_fts USING fts5(name, type_line, oracle_text, content='oracles', content_rowid='rowid', tokenize='unicode61 remove_diacritics 2');
INSERT INTO oracles_fts(rowid, name, type_line, oracle_text) SELECT rowid, name, type_line, oracle_text FROM oracles;
`;

interface ScryCard {
  id: string;
  oracle_id?: string;
  name: string;
  lang?: string;
  released_at?: string;
  layout: string;
  image_status?: string;
  image_uris?: Record<string, string>;
  mana_cost?: string;
  cmc?: number;
  type_line?: string;
  oracle_text?: string;
  power?: string;
  toughness?: string;
  loyalty?: string;
  defense?: string;
  colors?: string[];
  color_identity?: string[];
  legalities?: Record<string, string>;
  games?: string[];
  promo?: boolean;
  border_color?: string;
  frame_effects?: string[];
  full_art?: boolean;
  textless?: boolean;
  promo_types?: string[];
  digital?: boolean;
  set?: string;
  set_name?: string;
  set_type?: string;
  collector_number?: string;
  rarity?: string;
  prices?: { usd?: string | null; usd_foil?: string | null; eur?: string | null; eur_foil?: string | null };
  card_faces?: {
    name: string;
    mana_cost?: string;
    type_line?: string;
    oracle_text?: string;
    power?: string;
    toughness?: string;
    loyalty?: string;
    defense?: string;
    colors?: string[];
    oracle_id?: string;
    image_uris?: Record<string, string>;
  }[];
  all_parts?: { id: string; name: string; component: string; type_line: string }[];
}

export interface ImportProgress {
  stage: 'downloading' | 'importing' | 'indexing' | 'done' | 'error';
  rows: number;
  bytes: number;
  totalBytes: number;
  message?: string;
}

function toRow(c: ScryCard): unknown[] | null {
  if (c.layout === 'art_series') return null;
  const f0 = c.card_faces?.[0];
  const oracleId = c.oracle_id ?? f0?.oracle_id ?? null;
  const faces = c.card_faces?.map((f) => ({
    name: f.name,
    manaCost: f.mana_cost || '',
    typeLine: f.type_line || '',
    oracleText: f.oracle_text || '',
    power: f.power,
    toughness: f.toughness,
    loyalty: f.loyalty,
    defense: f.defense,
  }));
  const imageFaces = c.card_faces && c.card_faces.every((f) => f.image_uris) ? c.card_faces.length : c.image_uris ? 1 : 0;
  const typeLine = c.type_line || (faces ? faces.map((f) => f.typeLine).join(' // ') : '');
  const oracleText = c.oracle_text ?? (faces ? faces.map((f) => f.oracleText).join('\n//\n') : '');
  const manaCost = c.mana_cost ?? f0?.mana_cost ?? '';
  const isToken = c.layout === 'token' || c.layout === 'double_faced_token' || c.layout === 'emblem' || /\bToken\b|\bEmblem\b/.test(typeLine) || c.set_type === 'token' ? 1 : 0;
  const colors = (c.colors ?? f0?.colors ?? []).join('');
  const price = (a?: string | null, b?: string | null) => {
    const v = a ?? b;
    return v ? parseFloat(v) : null;
  };
  return [
    c.id,
    oracleId,
    c.name,
    normName(c.name),
    normName(c.name.split(' // ')[0]),
    c.lang || 'en',
    c.set || '',
    c.set_name || '',
    c.set_type || '',
    c.collector_number || '',
    c.released_at || '',
    c.layout,
    manaCost,
    c.cmc ?? 0,
    typeLine,
    oracleText,
    c.power ?? f0?.power ?? null,
    c.toughness ?? f0?.toughness ?? null,
    c.loyalty ?? f0?.loyalty ?? null,
    c.defense ?? f0?.defense ?? null,
    colors,
    (c.color_identity || []).join(''),
    c.rarity || 'common',
    JSON.stringify(c.legalities || {}),
    price(c.prices?.usd, c.prices?.usd_foil),
    price(c.prices?.eur, c.prices?.eur_foil),
    c.promo ? 1 : 0,
    c.digital ? 1 : 0,
    (c.games || []).includes('paper') ? 1 : 0,
    c.image_status || 'missing',
    imageFaces,
    faces && faces.length > 1 ? JSON.stringify(faces) : null,
    c.all_parts ? JSON.stringify(c.all_parts.map((p) => ({ id: p.id, name: p.name, component: p.component, typeLine: p.type_line }))) : null,
    isToken,
    nonStandard(c),
  ];
}

const ODD_SET_TYPES = new Set(['masterpiece', 'memorabilia', 'funny', 'alchemy', 'promo', 'treasure_chest', 'minigame', 'box', 'vanguard']);
/** 1 for printings that aren't a "normal" frame (showcase, borderless, Secret Lair …) so they rank after normal ones. */
function nonStandard(c: ScryCard): number {
  if (c.set_type && ODD_SET_TYPES.has(c.set_type)) return 2;
  if (c.border_color === 'borderless' || c.full_art || c.textless) return 1;
  if ((c.frame_effects || []).some((f) => ['showcase', 'extendedart', 'etched', 'inverted', 'fullart'].includes(f))) return 1;
  if ((c.promo_types || []).length) return 1;
  return 0;
}

/** Streams Scryfall bulk data (JSON array or JSONL) into a new card DB file. */
export async function importBulk(input: Readable, outFile: string, onProgress?: (p: ImportProgress) => void, totalBytes = 0): Promise<number> {
  fs.rmSync(outFile, { force: true });
  for (const s of ['-wal', '-shm']) fs.rmSync(outFile + s, { force: true });
  const db = openDb(outFile);
  db.exec('PRAGMA journal_mode = OFF; PRAGMA synchronous = OFF;');
  db.exec(SCHEMA);
  const ins = db.prepare(`INSERT OR REPLACE INTO printings VALUES (${new Array(35).fill('?').join(',')})`);
  let rows = 0;
  let bytes = 0;
  input.on('data', (b: Buffer) => {
    bytes += b.length;
  });
  const rl = readline.createInterface({ input, crlfDelay: Infinity });
  db.exec('BEGIN');
  let lastReport = 0;
  for await (let line of rl) {
    line = line.trim();
    if (!line || line === '[' || line === ']') continue;
    if (line.startsWith('[')) line = line.slice(1);
    if (line.endsWith(',')) line = line.slice(0, -1);
    if (line.endsWith(']')) line = line.slice(0, -1);
    let c: ScryCard;
    try {
      c = JSON.parse(line);
    } catch {
      continue;
    }
    const r = toRow(c);
    if (!r) continue;
    ins.run(...(r as never[]));
    rows++;
    if (rows % 5000 === 0) {
      db.exec('COMMIT; BEGIN');
      const now = Date.now();
      if (onProgress && now - lastReport > 300) {
        lastReport = now;
        onProgress({ stage: 'importing', rows, bytes, totalBytes });
      }
    }
  }
  db.exec('COMMIT');
  onProgress?.({ stage: 'indexing', rows, bytes, totalBytes });
  db.exec(POST);
  run(db, 'INSERT OR REPLACE INTO meta VALUES (?, ?)', 'imported_at', String(Date.now()));
  run(db, 'INSERT OR REPLACE INTO meta VALUES (?, ?)', 'rows', String(rows));
  db.exec('PRAGMA journal_mode = WAL; ANALYZE; VACUUM;');
  db.close();
  return rows;
}

// ------------------------------------------------------------------ query side

interface PRow {
  id: string;
  oracle_id: string | null;
  name: string;
  lang: string;
  set_code: string;
  set_name: string;
  collector_number: string;
  released_at: string;
  layout: string;
  mana_cost: string;
  cmc: number;
  type_line: string;
  oracle_text: string;
  power: string | null;
  toughness: string | null;
  loyalty: string | null;
  defense: string | null;
  colors: string;
  color_identity: string;
  rarity: string;
  legalities: string;
  usd: number | null;
  eur: number | null;
  promo: number;
  digital: number;
  image_status: string;
  image_faces: number;
  faces: string | null;
  all_parts: string | null;
  is_token: number;
}

export function rowToDef(r: PRow): CardDef {
  const faces = r.faces ? (JSON.parse(r.faces) as CardFace[]) : undefined;
  const img: string[] = [];
  for (let i = 0; i < Math.max(0, r.image_faces); i++) img.push(`p/${r.id}/${i}`);
  return {
    key: 'p:' + r.id,
    printingId: r.id,
    oracleId: r.oracle_id || undefined,
    name: r.name,
    layout: r.layout,
    manaCost: r.mana_cost || '',
    cmc: r.cmc || 0,
    typeLine: r.type_line || '',
    oracleText: r.oracle_text || '',
    power: r.power ?? undefined,
    toughness: r.toughness ?? undefined,
    loyalty: r.loyalty ?? undefined,
    defense: r.defense ?? undefined,
    colors: r.colors ? r.colors.split('') : [],
    colorIdentity: r.color_identity ? r.color_identity.split('') : [],
    faces,
    set: r.set_code,
    setName: r.set_name,
    collectorNumber: r.collector_number,
    rarity: r.rarity,
    releasedAt: r.released_at,
    img,
    isToken: !!r.is_token,
    legalities: JSON.parse(r.legalities || '{}'),
    usd: r.usd,
    eur: r.eur,
    allParts: r.all_parts ? (JSON.parse(r.all_parts) as RelatedPart[]) : undefined,
  };
}

function like(v: string): string {
  return '%' + v.replace(/[%_\\]/g, (m) => '\\' + m) + '%';
}

function colorSql(col: string, v: string, op: string, identity: boolean, params: unknown[]): string {
  const want = parseColors(v);
  if (want == null) return '0';
  if (want === 'M') return `length(${col}) > 1`;
  if (want.length === 0) return `${col} = ''`;
  const others = ['W', 'U', 'B', 'R', 'G'].filter((c) => !want.includes(c));
  // Colours are single letters, so they can be inlined safely without parameters.
  const sup = want.map((c) => `instr(${col}, '${c}') > 0`).join(' AND ');
  const sub = others.length ? others.map((c) => `instr(${col}, '${c}') = 0`).join(' AND ') : '1';
  void params;
  const mode = op === ':' ? (identity ? '<=' : '>=') : op;
  switch (mode) {
    case '=':
      return `(${sup} AND ${sub})`;
    case '>=':
      return `(${sup})`;
    case '<=':
      return `(${sub})`;
    case '>':
      return `(${sup} AND NOT (${sub}))`;
    case '<':
      return `(${sub} AND NOT (${sup}))`;
    case '!=':
      return `NOT (${sup} AND ${sub})`;
    default:
      return `(${sup})`;
  }
}

function numOp(op: string): string {
  return op === ':' ? '=' : op;
}

/** Compiles one search term. Terms on `p.` are printing-level (set, rarity); the rest apply to oracles `o.`. */
function termSql(t: SearchTerm, params: unknown[], plevel: { used: boolean }): string {
  let sql: string;
  if (t.kind === 'name') {
    params.push(like(normName(t.value)));
    sql = `o.norm_name LIKE ? ESCAPE '\\'`;
  } else {
    const v = t.value;
    switch (t.field) {
      case 't':
        params.push(like(v));
        sql = `o.type_line LIKE ? ESCAPE '\\'`;
        break;
      case 'o':
        params.push(like(v.replace(/~/g, '')));
        sql = `o.oracle_text LIKE ? ESCAPE '\\'`;
        break;
      case 'c':
        sql = colorSql('o.colors', v, t.op, false, params);
        break;
      case 'id':
        sql = colorSql('o.color_identity', v, t.op, true, params);
        break;
      case 'mv':
        params.push(parseFloat(v) || 0);
        sql = `o.cmc ${numOp(t.op)} ?`;
        break;
      case 'pow':
        params.push(parseFloat(v) || 0);
        sql = `(o.power GLOB '[0-9]*' AND CAST(o.power AS REAL) ${numOp(t.op)} ?)`;
        break;
      case 'tou':
        params.push(parseFloat(v) || 0);
        sql = `(o.toughness GLOB '[0-9]*' AND CAST(o.toughness AS REAL) ${numOp(t.op)} ?)`;
        break;
      case 'r': {
        plevel.used = true;
        params.push(RARITY_CODES[v] || v);
        sql = `p.rarity = ?`;
        break;
      }
      case 's':
        plevel.used = true;
        params.push(v);
        sql = `p.set_code = ?`;
        break;
      case 'f':
        params.push(v.replace(/[^a-z]/g, ''));
        sql = `json_extract(o.legalities, '$.' || ?) IN ('legal', 'restricted')`;
        break;
      case 'is':
        switch (v) {
          case 'commander':
            sql = `((o.type_line LIKE '%Legendary%' AND (o.type_line LIKE '%Creature%' OR o.type_line LIKE '%Planeswalker%')) OR o.oracle_text LIKE '%can be your commander%')`;
            break;
          case 'token':
            sql = 'o.is_token = 1';
            break;
          case 'legendary':
            sql = `o.type_line LIKE '%Legendary%'`;
            break;
          case 'dfc':
          case 'mdfc':
          case 'transform':
            sql = `o.layout IN ('transform', 'modal_dfc', 'double_faced_token', 'meld', 'reversible_card')`;
            break;
          case 'split':
            sql = `o.layout = 'split'`;
            break;
          case 'flip':
            sql = `o.layout = 'flip'`;
            break;
          case 'permanent':
            sql = `o.type_line NOT LIKE '%Instant%' AND o.type_line NOT LIKE '%Sorcery%'`;
            break;
          case 'spell':
            sql = `o.type_line NOT LIKE '%Land%'`;
            break;
          default:
            sql = '0';
        }
        break;
      default:
        sql = '1';
    }
  }
  return t.neg ? `NOT (${sql})` : sql;
}

export interface SearchOpts {
  limit?: number;
  offset?: number;
  tokens?: boolean;
}

export class CardDb {
  db: DB | null = null;
  file: string;
  private names: { norm: string; name: string; oracle: string }[] | null = null;
  private defCache = new Map<string, CardDef>();

  constructor(file: string) {
    this.file = file;
    this.reopen();
  }

  reopen() {
    this.db?.close();
    this.db = null;
    this.names = null;
    this.defCache.clear();
    if (fs.existsSync(this.file)) {
      this.db = openDb(this.file);
    }
  }

  close() {
    this.db?.close();
    this.db = null;
  }

  get ready(): boolean {
    return !!this.db;
  }

  private d(): DB {
    if (!this.db) throw new Error('Card data has not been imported yet');
    return this.db;
  }

  meta(key: string): string | null {
    if (!this.db) return null;
    return get<{ value: string }>(this.db, 'SELECT value FROM meta WHERE key = ?', key)?.value ?? null;
  }

  stats() {
    if (!this.db) return { printings: 0, oracles: 0, importedAt: null as number | null };
    const p = get<{ n: number }>(this.db, 'SELECT COUNT(*) AS n FROM printings')!.n;
    const o = get<{ n: number }>(this.db, 'SELECT COUNT(*) AS n FROM oracles')!.n;
    const at = this.meta('imported_at');
    return { printings: p, oracles: o, importedAt: at ? parseInt(at, 10) : null };
  }

  search(q: string, opts: SearchOpts = {}): { cards: CardDef[]; total: number } {
    if (!this.db) return { cards: [], total: 0 };
    const query: SearchQuery = parseSearch(q);
    const params: unknown[] = [];
    const pparams: unknown[] = [];
    const plevel = { used: false };
    const wantsTokens =
      opts.tokens || query.some((g) => g.some((t) => t.kind === 'field' && ((t.field === 'is' && t.value === 'token') || (t.field === 't' && /token|emblem|dungeon/.test(t.value)))));
    // Split each AND-group into oracle-level and printing-level conditions.
    const oGroups: string[] = [];
    const pGroups: string[] = [];
    for (const g of query) {
      const o: string[] = [];
      const p: string[] = [];
      for (const t of g) {
        const pl = { used: false };
        const tmp: unknown[] = [];
        const s = termSql(t, tmp, pl);
        if (pl.used) {
          p.push(s);
          pparams.push(...tmp);
          plevel.used = true;
        } else {
          o.push(s);
          params.push(...tmp);
        }
      }
      oGroups.push(o.length ? o.join(' AND ') : '1');
      pGroups.push(p.length ? p.join(' AND ') : '1');
    }
    const where: string[] = [];
    if (oGroups.length) where.push('(' + oGroups.map((x) => `(${x})`).join(' OR ') + ')');
    where.push(wantsTokens ? (opts.tokens ? 'o.is_token = 1' : '1') : 'o.is_token = 0');
    where.push('o.paper = 1');
    const pcond = plevel.used ? '(' + pGroups.map((x) => `(${x})`).join(' OR ') + ')' : '1';
    const firstName = query[0]?.find((t) => t.kind === 'name')?.value;
    const order: string[] = [];
    const oparams: unknown[] = [];
    if (firstName) {
      oparams.push(normName(firstName), normName(firstName) + '%');
      order.push('(o.norm_name = ?) DESC', '(o.norm_name LIKE ?) DESC');
    }
    order.push('o.name');
    const limit = Math.min(opts.limit ?? 60, 200);
    const offset = opts.offset ?? 0;
    const pidExpr = plevel.used
      ? `(SELECT p.id FROM printings p WHERE p.oracle_id = o.oracle_id AND ${pcond} ORDER BY p.released_at DESC LIMIT 1)`
      : 'o.default_id';
    const sql = `SELECT pid FROM (SELECT ${pidExpr} AS pid, o.norm_name, o.name FROM oracles o WHERE ${where.join(' AND ')}) o WHERE pid IS NOT NULL`;
    const allParams = plevel.used ? [...pparams, ...params] : params;
    const total = get<{ n: number }>(this.d(), `SELECT COUNT(*) AS n FROM (${sql})`, ...allParams)!.n;
    const rows = all<{ pid: string }>(this.d(), `${sql} ORDER BY ${order.join(', ')} LIMIT ${limit} OFFSET ${offset}`, ...allParams, ...oparams);
    const defs = this.defsByPrinting(rows.map((r) => r.pid));
    return { cards: rows.map((r) => defs['p:' + r.pid]).filter(Boolean), total };
  }

  autocomplete(q: string, limit = 10): { name: string; key: string }[] {
    if (!this.db || q.trim().length < 2) return [];
    const n = normName(q);
    return all<{ name: string; default_id: string }>(
      this.d(),
      `SELECT name, default_id FROM oracles WHERE is_token = 0 AND paper = 1 AND (norm_name LIKE ? OR front_norm LIKE ?)
       ORDER BY (norm_name LIKE ?) DESC, length(name), name LIMIT ?`,
      n + '%',
      '%' + n + '%',
      n + '%',
      limit,
    ).map((r) => ({ name: r.name, key: 'p:' + r.default_id }));
  }

  /** Cards whose name contains the query (for rulings search), best matches first. */
  oraclesByName(q: string, limit = 30): { oracleId: string; name: string; defaultId: string }[] {
    if (!this.db || q.trim().length < 2) return [];
    const n = normName(q);
    return all<{ oracle_id: string; name: string; default_id: string }>(
      this.d(),
      // Whole-word match: "station" finds Station Monitor, not Devastation Tide.
      `SELECT oracle_id, name, default_id FROM oracles WHERE is_token = 0 AND (norm_name LIKE ? OR norm_name LIKE ? OR norm_name LIKE ?)
       ORDER BY (norm_name = ? OR front_norm = ?) DESC, (norm_name LIKE ?) DESC, length(name), name LIMIT ?`,
      n + '%',
      '% ' + n + '%',
      '%-' + n + '%',
      n,
      n,
      n + '%',
      limit,
    ).map((r) => ({ oracleId: r.oracle_id, name: r.name, defaultId: r.default_id }));
  }

  /** Name and default printing for each oracle id. */
  oracleInfo(ids: string[]): Record<string, { name: string; defaultId: string }> {
    const out: Record<string, { name: string; defaultId: string }> = {};
    if (!this.db) return out;
    for (let i = 0; i < ids.length; i += 500) {
      const chunk = ids.slice(i, i + 500);
      for (const r of all<{ oracle_id: string; name: string; default_id: string }>(this.db, `SELECT oracle_id, name, default_id FROM oracles WHERE oracle_id IN (${chunk.map(() => '?').join(',')})`, ...chunk))
        out[r.oracle_id] = { name: r.name, defaultId: r.default_id };
    }
    return out;
  }

  defsByPrinting(ids: string[]): Record<string, CardDef> {
    const out: Record<string, CardDef> = {};
    const need: string[] = [];
    for (const id of ids) {
      const c = this.defCache.get(id);
      if (c) out['p:' + id] = c;
      else need.push(id);
    }
    if (need.length && this.db) {
      for (let i = 0; i < need.length; i += 500) {
        const chunk = need.slice(i, i + 500);
        const rows = all<PRow>(this.db, `SELECT * FROM printings WHERE id IN (${chunk.map(() => '?').join(',')})`, ...chunk);
        for (const r of rows) {
          const d = rowToDef(r);
          if (this.defCache.size > 20000) this.defCache.clear();
          this.defCache.set(r.id, d);
          out[d.key] = d;
        }
      }
    }
    return out;
  }

  defaultPrinting(oracleId: string): string | null {
    if (!this.db) return null;
    return get<{ default_id: string }>(this.db, 'SELECT default_id FROM oracles WHERE oracle_id = ?', oracleId)?.default_id ?? null;
  }

  printings(oracleId: string): CardDef[] {
    if (!this.db) return [];
    const rows = all<PRow>(
      this.db,
      // Release order (newest first), with a stable order inside a set so the list never shifts between opens.
      `SELECT * FROM printings WHERE oracle_id = ? AND (paper = 1 OR digital = 0) ORDER BY released_at DESC, set_code, CAST(collector_number AS INTEGER), collector_number LIMIT 300`,
      oracleId,
    );
    return rows.map(rowToDef);
  }

  /**
   * A printing of the token that best fits what the rules engine made: the same name (a trailing " Token" dropped),
   * then the same colours, power/toughness and creature types. Null when Scryfall has no such token.
   */
  matchToken(name: string, power: number | undefined, toughness: number | undefined, colorMask: number | undefined, types: string, text = ''): string | null {
    if (!this.db) return null;
    const base = name.replace(/\s+Token$/i, '');
    const n = normName(base);
    const rows = all<{ default_id: string; colors: string; power: string | null; toughness: string | null; type_line: string; oracle_text: string | null }>(
      this.db,
      `SELECT default_id, colors, power, toughness, type_line, oracle_text FROM oracles WHERE is_token = 1 AND (norm_name = ? OR front_norm = ?)`,
      n,
      n,
    );
    if (!rows.length) return null;
    const letters = 'WUBRG';
    const want = colorMask === undefined ? null : [...letters].filter((_, i) => colorMask & (1 << i)).join('');
    const subtypes = (types.split('—')[1] ?? '').trim().split(/\s+/).filter(Boolean);
    let best = rows[0];
    let bestScore = -1;
    for (const r of rows) {
      const mine = [...(r.colors ?? '')].sort((a, b) => letters.indexOf(a) - letters.indexOf(b)).join('');
      let score = 0;
      if (want !== null && mine === want) score += 4;
      if (power !== undefined && r.power === String(power) && r.toughness === String(toughness)) score += 2;
      if (subtypes.length && subtypes.every((t) => r.type_line.includes(t))) score += 1;
      // the same rules text (a 0/0 Construct that grows with your artifacts is not the vanilla Construct)
      const want_text = text.replace(/\s+/g, ' ').trim().toLowerCase();
      const has_text = (r.oracle_text ?? '').replace(/\s+/g, ' ').trim().toLowerCase();
      if (want_text && has_text && (want_text === has_text || has_text.includes(want_text) || want_text.includes(has_text))) score += 3;
      else if (!want_text && !has_text) score += 1;
      if (!r.type_line.includes('//')) score += 0.5; // a one-sided token over a double-sided one
      if (score > bestScore) {
        best = r;
        bestScore = score;
      }
    }
    return best.default_id;
  }

  /**
   * Name lookup for imports. A set + collector number pins the printing; it also resolves printings whose
   * list name is a flavor name (e.g. Godzilla or Final Fantasy crossover names) that isn't the oracle name.
   */
  matchName(name: string, set?: string, number?: string, preferToken = false): string | null {
    if (!this.db) return null;
    // Moxfield and others write double-faced / split / adventure names with a single slash.
    const n = normName(name.replace(/\s+\/{1,2}\s+/g, ' // '));
    const ord = preferToken ? 'is_token DESC, (type_line LIKE \'%//%\')' : 'is_token';
    const o =
      get<{ oracle_id: string; default_id: string }>(this.db, `SELECT oracle_id, default_id FROM oracles WHERE norm_name = ? ORDER BY ${ord} LIMIT 1`, n) ||
      get<{ oracle_id: string; default_id: string }>(this.db, `SELECT oracle_id, default_id FROM oracles WHERE front_norm = ? ORDER BY ${ord} LIMIT 1`, n);
    const exact =
      set && number
        ? get<{ id: string; oracle_id: string | null }>(
            this.db,
            'SELECT id, oracle_id FROM printings WHERE set_code = ? AND collector_number = ? COLLATE NOCASE LIMIT 1',
            set.toLowerCase(),
            number,
          )
        : undefined;
    // The exact printing wins unless the name clearly points at a different card.
    if (exact && (!o || exact.oracle_id === o.oracle_id)) return exact.id;
    if (!o) {
      // Printed names / flavor names aren't indexed; try a printing-level exact name.
      const p = get<{ id: string }>(this.db, 'SELECT id FROM printings WHERE norm_name = ? OR front_norm = ? ORDER BY released_at DESC LIMIT 1', n, n);
      return p?.id ?? null;
    }
    if (set) {
      const p = get<{ id: string }>(this.db, 'SELECT id FROM printings WHERE oracle_id = ? AND set_code = ? ORDER BY nonstd, collector_number LIMIT 1', o.oracle_id, set.toLowerCase());
      if (p) return p.id;
    }
    return o.default_id;
  }

  /** The default printing of the card whose back face is named `name` ("Insectile Aberration" → Delver of Secrets). */
  matchBackFace(name: string): string | null {
    if (!this.db) return null;
    const n = normName(name).replace(/[\\%_]/g, (m) => '\\' + m);
    return get<{ default_id: string }>(this.db, "SELECT default_id FROM oracles WHERE norm_name LIKE ? ESCAPE '\\' ORDER BY is_token LIMIT 1", '% // ' + n)?.default_id ?? null;
  }

  /** Top suggestions for an unmatched import line. */
  suggest(name: string, n = 3): string[] {
    if (!this.db) return [];
    if (!this.names) {
      this.names = all<{ norm_name: string; name: string; oracle_id: string }>(this.db, 'SELECT norm_name, name, oracle_id FROM oracles WHERE is_token = 0 AND paper = 1').map((r) => ({
        norm: r.norm_name,
        name: r.name,
        oracle: r.oracle_id,
      }));
    }
    const q = normName(name.split(/\s+\/{1,2}\s+/)[0]);
    const scored: { name: string; d: number }[] = [];
    let worst = Infinity;
    for (const x of this.names) {
      const front = x.norm.split(' // ')[0];
      const bound = Math.min(worst, Math.max(3, Math.floor(q.length / 2)));
      let d = levenshtein(q, front, bound);
      if (x.norm.startsWith(q) || front.includes(q)) d = Math.min(d, 1 + Math.abs(front.length - q.length) / 10);
      if (d > bound) continue;
      scored.push({ name: x.name, d });
      if (scored.length > 50) {
        scored.sort((a, b) => a.d - b.d);
        scored.length = 20;
        worst = scored[scored.length - 1].d;
      }
    }
    scored.sort((a, b) => a.d - b.d || a.name.length - b.name.length);
    return Array.from(new Set(scored.map((s) => s.name))).slice(0, n);
  }
}

export function cardsDbPath(dataDir: string): string {
  return path.join(dataDir, 'cards.sqlite');
}
