import fs from 'node:fs';
import type { CardDef, Deck, DeckCard, DeckSummary, PreconInfo, Section } from '@aetherhub/shared';
import { checkLegality, COLORS, FORMATS } from '@aetherhub/shared';
import type { AppDb } from './db/app';
import type { CardDb } from './db/cards';

/** One deck of decks/precons.json (written by scripts/precons.mjs). */
interface PreconEntry {
  id: number;
  file: string;
  name: string;
  code: string;
  type: string;
  date: string | null;
  format: string;
  /** [section, qty, name, set code, collector number] */
  cards: [Section, number, string, string, string][];
}

interface Resolved {
  cards: DeckCard[];
  defs: Record<string, CardDef>;
  /** cards the card database doesn't have, by name */
  missing: string[];
  /** the formats the deck is legal in, the one it is listed under first */
  formats: string[];
}

/** Formats a bundled deck is tested against: the ones with a legality list. */
const CHECKED = FORMATS.filter((f) => f.legalityKey);

export const PRECON_OWNER = 'Pre-constructed';

/**
 * The official preconstructed decks bundled with the server, served as read-only decks with negative ids. They are
 * never stored in the decks table, so they don't show up in deck lists, user counts or backups. Listing them is
 * switched on by an admin; looking one up by id always works, so games and lobbies that use one keep working.
 */
export class PreconService {
  private entries: Map<number, PreconEntry> | null = null;
  private resolved = new Map<number, Resolved>();
  private summaries: DeckSummary[] | null = null;
  private building: Promise<DeckSummary[]> | null = null;
  /** card data import time the resolved cards belong to */
  private cardsAt: number | null = null;

  constructor(
    private app: AppDb,
    private cards: CardDb,
    private file: string,
  ) {}

  static isPreconId(id: number): boolean {
    return id < 0;
  }

  enabled(): boolean {
    return this.app.setting('precons', '0') === '1';
  }

  setEnabled(on: boolean) {
    this.app.setSetting('precons', on ? '1' : '0');
  }

  /** Number of decks in the bundled file (0 when it is missing). */
  available(): number {
    return this.load().size;
  }

  private load(): Map<number, PreconEntry> {
    if (!this.entries) {
      this.entries = new Map();
      try {
        const data = JSON.parse(fs.readFileSync(this.file, 'utf8')) as { decks: PreconEntry[] };
        for (const d of data.decks) this.entries.set(d.id, d);
      } catch {
        /* no bundled file: no precons */
      }
    }
    return this.entries;
  }

  /** Drops resolved cards when the card data was re-imported (printing ids can change). */
  private checkCardData() {
    const at = this.cards.stats().importedAt;
    if (at !== this.cardsAt) {
      this.cardsAt = at;
      this.resolved.clear();
      this.summaries = null;
      this.building = null;
    }
  }

  private resolve(e: PreconEntry): Resolved {
    this.checkCardData();
    let r = this.resolved.get(e.id);
    if (r) return r;
    const cards: DeckCard[] = [];
    const missing: string[] = [];
    for (const [i, [section, qty, name, set, number]] of e.cards.entries()) {
      const id = this.cards.matchName(name, set, number);
      if (!id) {
        missing.push(name);
        continue;
      }
      cards.push({ id: i + 1, section, def: 'p:' + id, qty, foil: false, tags: [], addedAt: 0, frontAsset: null, backAsset: null });
    }
    const defs = this.cards.defsByPrinting(cards.map((c) => c.def.slice(2)));
    const found = cards.filter((c) => defs[c.def]);
    r = { cards: found, defs, missing, formats: PreconService.legalFormats(e.format, found, defs, missing.length > 0) };
    this.resolved.set(e.id, r);
    return r;
  }

  /**
   * Every format the deck passes the legality check in without a warning. A deck with cards the database doesn't have
   * stays in its own format only. A freeform deck is listed under the first legal format instead (Standard before
   * Pioneer before Modern ...), since that is the narrowest one.
   */
  static legalFormats(format: string, cards: DeckCard[], defs: Record<string, CardDef>, incomplete: boolean): string[] {
    const legal = incomplete ? [] : CHECKED.filter((f) => !checkLegality(f.id, cards, defs).warnings.length).map((f) => f.id as string);
    const own = format === 'freeform' && legal.length ? legal[0] : format;
    return [own, ...legal.filter((f) => f !== own)];
  }

  private info(e: PreconEntry, r: Resolved): PreconInfo {
    return { type: e.type, set: e.code, released: e.date, missing: r.missing, formats: r.formats };
  }

  private summary(e: PreconEntry): DeckSummary {
    const r = this.resolve(e);
    const { cards, defs } = r;
    const cmd = cards.filter((c) => c.section === 'commander');
    const src = cmd.length ? cmd : cards.filter((c) => c.section === 'main');
    const cols = new Set<string>();
    src.forEach((c) => defs[c.def]?.colorIdentity.forEach((x) => cols.add(x)));
    const cover = src[0] && defs[src[0].def]?.img[0];
    return {
      id: e.id,
      ownerId: 0,
      ownerName: PRECON_OWNER,
      name: e.name,
      format: r.formats[0],
      folderId: null,
      colors: COLORS.filter((c) => cols.has(c)),
      cardCount: cards.filter((c) => c.section === 'main' || c.section === 'commander').reduce((n, c) => n + c.qty, 0),
      warnings: 0,
      coverImg: cover || null,
      pinned: false,
      updatedAt: e.date ? Date.parse(e.date) : 0,
      precon: this.info(e, r),
    };
  }

  /**
   * Every bundled deck, newest first. Empty until the card data is imported. The first call resolves every deck's
   * cards (a few seconds of database lookups), in small batches so the server keeps answering in between.
   */
  async list(): Promise<DeckSummary[]> {
    this.checkCardData();
    if (!this.cards.db) return [];
    if (this.summaries) return this.summaries;
    if (!this.building) {
      const at = this.cardsAt;
      this.building = (async () => {
        const out: DeckSummary[] = [];
        for (const [i, e] of [...this.load().values()].entries()) {
          if (i % 20 === 0) await new Promise((r) => setImmediate(r));
          if (this.cardsAt !== at) throw new Error('Card data changed while listing preconstructed decks');
          out.push(this.summary(e));
        }
        if (this.cardsAt === at) this.summaries = out;
        return out;
      })().finally(() => {
        this.building = null;
      });
    }
    return this.building;
  }

  getDeck(id: number): Deck | null {
    const e = this.load().get(id);
    if (!e) return null;
    const { cards, defs } = this.resolve(e);
    return { ...this.summary(e), notes: '', tags: [], recentTags: [], coverKey: null, sleeveAsset: null, createdAt: 0, cards: cards.map((c) => ({ ...c, tags: [] })), defs };
  }

  /** The deck's name and cards, for games and for copying into someone's decks. */
  contents(id: number): { name: string; format: string; cards: DeckCard[]; defs: Record<string, CardDef> } | null {
    const e = this.load().get(id);
    if (!e) return null;
    const { cards, defs, formats } = this.resolve(e);
    return { name: e.name, format: formats[0], cards, defs };
  }
}
