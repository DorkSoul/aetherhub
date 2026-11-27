import fs from 'node:fs';
import path from 'node:path';
import type { LogFn } from './activity';
import { config, dataPath } from './config';
import type { AppDb } from './db/app';
import type { CardDb } from './db/cards';
import { all } from './db/sqlite';
import { scryfallApi, scryfallFetch } from './scryfall';

/** What the server knows about a set from Scryfall's /sets list (fetched once a week). */
interface ScryfallSet {
  code: string;
  name: string;
  set_type: string;
  released_at?: string;
  card_count: number;
  printed_size?: number;
  icon_svg_uri?: string;
  digital?: boolean;
}

export interface DraftSet {
  code: string;
  name: string;
  released: string;
  /** distinct cards that can come in a booster */
  cards: number;
  type: string;
  /** the set's icon can be served (cached, or fetchable) */
  icon: boolean;
  /** the numbered main set size: higher collector numbers are bonus sheets, not booster cards */
  printedSize: number | null;
  /** Play Booster (2024 on) or Draft Booster */
  booster: 'play' | 'draft';
}

const WEEK = 7 * 24 * 3600_000;
const SET_TYPES = ['expansion', 'core', 'masters', 'draft_innovation'];
/** Products in those set types that are not opened as draft boosters (The List, Commander-only Mystery Boosters, Jumpstart). */
const NOT_DRAFTABLE = new Set(['plst', 'mbc', 'jmp', 'j22', 'j25', 'fj25', 'ajmp', 'sjm']);

/**
 * Sets that can be drafted, with their icons: the sets in the card data that have enough booster cards, joined with
 * Scryfall's /sets list (a single free API call, cached in the app database). Icons are fetched once into the data
 * directory and served from there, so a LAN server keeps showing them offline.
 */
export class SetCatalog {
  private meta = new Map<string, ScryfallSet>();
  private loading: Promise<void> | null = null;

  constructor(
    private app: AppDb,
    private cards: CardDb,
    private log: LogFn = () => {},
  ) {
    try {
      const saved = JSON.parse(app.setting('scryfallSets', '{}')) as { sets?: ScryfallSet[] };
      for (const s of saved.sets ?? []) this.meta.set(s.code, s);
    } catch {
      /* none yet */
    }
  }

  /** Refresh the Scryfall set list when it is older than a week (in the background; never when offline). */
  refresh(): Promise<void> {
    if (config.imageOffline) return Promise.resolve();
    const fetchedAt = Number(this.app.setting('scryfallSetsAt', '0'));
    if (this.meta.size && Date.now() - fetchedAt < WEEK) return Promise.resolve();
    this.loading ??= (async () => {
      try {
        const res = await scryfallApi('/sets');
        if (!res.ok) throw new Error(`HTTP ${res.status}`);
        const body = (await res.json()) as { data: ScryfallSet[] };
        const keep = body.data.map(({ code, name, set_type, released_at, card_count, printed_size, icon_svg_uri, digital }) => ({ code, name, set_type, released_at, card_count, printed_size, icon_svg_uri, digital }));
        this.meta = new Map(keep.map((s) => [s.code, s]));
        this.app.setSetting('scryfallSets', JSON.stringify({ sets: keep }));
        this.app.setSetting('scryfallSetsAt', String(Date.now()));
      } catch (e) {
        this.log(`Set list not refreshed: ${(e as Error).message}`, { cat: 'cards', level: 'warn' });
      } finally {
        this.loading = null;
      }
    })();
    return this.loading;
  }

  /** Paper sets with at least 40 booster cards, newest first. */
  list(): DraftSet[] {
    if (!this.cards.db) return [];
    void this.refresh();
    const rows = all<{ set_code: string; set_name: string; set_type: string; released: string; n: number }>(
      this.cards.db,
      `SELECT set_code, MAX(set_name) AS set_name, MAX(set_type) AS set_type, MAX(released_at) AS released, COUNT(DISTINCT name) AS n FROM printings
       WHERE set_type IN (${SET_TYPES.map(() => '?').join(',')}) AND paper = 1 AND is_token = 0 AND digital = 0 AND promo = 0 AND nonstd = 0
       GROUP BY set_code HAVING COUNT(DISTINCT name) >= 40 ORDER BY released DESC`,
      ...SET_TYPES,
    );
    return rows
      .filter((r) => !NOT_DRAFTABLE.has(r.set_code) && !/Commander|^The List/i.test(this.meta.get(r.set_code)?.name ?? r.set_name))
      .map((r) => {
      const m = this.meta.get(r.set_code);
      return {
        code: r.set_code,
        name: m?.name ?? r.set_name,
        released: m?.released_at ?? r.released,
        cards: r.n,
        type: r.set_type,
        icon: !!m?.icon_svg_uri || fs.existsSync(this.iconFile(r.set_code)),
        printedSize: m?.printed_size ?? null,
        booster: ((m?.released_at ?? r.released) >= '2024-02-09' ? 'play' : 'draft') as 'play' | 'draft',
      };
      })
      .sort((a, b) => b.released.localeCompare(a.released));
  }

  printedSize(code: string): number | null {
    return this.meta.get(code)?.printed_size ?? null;
  }

  private iconFile(code: string) {
    return dataPath('set-icons', `${code.replace(/[^a-z0-9]/gi, '')}.svg`);
  }

  /** The set's icon (SVG text), fetched once from Scryfall and kept in the data directory; null when unknown or offline. */
  async icon(code: string): Promise<string | null> {
    const file = this.iconFile(code);
    if (fs.existsSync(file)) return fs.readFileSync(file, 'utf8');
    const uri = this.meta.get(code)?.icon_svg_uri;
    if (!uri || config.imageOffline) return null;
    try {
      const res = await scryfallFetch(uri, { accept: 'image/svg+xml' });
      if (!res.ok) return null;
      const svg = await res.text();
      if (!svg.includes('<svg')) return null;
      fs.mkdirSync(path.dirname(file), { recursive: true });
      fs.writeFileSync(file, svg);
      return svg;
    } catch {
      return null;
    }
  }
}
