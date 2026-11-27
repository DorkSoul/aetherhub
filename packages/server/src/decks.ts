import type { CardDef, Deck, DeckCard, DeckSummary, DeckTag, RecentTag, Section, TagScope } from '@aetherhub/shared';
import { checkLegality, COLORS, manaSymbols, manaValue, MAX_CARD_TAGS, MAX_DECK_TAGS, normalizeTag, RECENT_TAGS, SECTIONS } from '@aetherhub/shared';
import type { AppDb } from './db/app';
import type { CardDb } from './db/cards';
import { all, get, run, tx } from './db/sqlite';
import { PreconService } from './precons';

/** Bump when checkLegality changes what it warns about, so the cached warning counts are recomputed at startup. */
const LEGALITY_VERSION = '2026-11-21b commander deck rules';

export interface CustomRow {
  id: number;
  creator_id: number | null;
  name: string;
  mana_cost: string;
  type_line: string;
  power: string | null;
  toughness: string | null;
  loyalty: string | null;
  oracle_text: string;
  colors: string;
  rarity: string;
  image_asset: number | null;
  created_at: number;
  updated_at: number;
}

/** Normalised, deduplicated and capped. */
function cleanTags(tags: string[]): string[] {
  return Array.from(new Set(tags.map(normalizeTag).filter(Boolean))).slice(0, MAX_CARD_TAGS);
}

export function customDef(r: CustomRow): CardDef {
  const fromCost = COLORS.filter((c) => manaSymbols(r.mana_cost).some((s) => s.includes(c)));
  const colors = r.colors ? r.colors.split('') : fromCost;
  return {
    key: 'c:' + r.id,
    customId: r.id,
    name: r.name,
    layout: 'normal',
    manaCost: r.mana_cost,
    cmc: manaValue(r.mana_cost),
    typeLine: r.type_line,
    oracleText: r.oracle_text,
    power: r.power ?? undefined,
    toughness: r.toughness ?? undefined,
    loyalty: r.loyalty ?? undefined,
    colors,
    colorIdentity: colors,
    rarity: r.rarity,
    set: 'custom',
    setName: 'Custom card',
    img: r.image_asset ? ['a/' + r.image_asset] : [],
    custom: true,
    isToken: /\bToken\b/.test(r.type_line),
  };
}

interface CardRow {
  id: number;
  deck_id: number;
  section: string;
  card_key: string;
  qty: number;
  foil: number;
  tags: string;
  added_at: number;
  front_asset: number | null;
  back_asset: number | null;
}

interface DeckRow {
  id: number;
  owner_id: number;
  owner_name: string;
  name: string;
  format: string;
  folder_id: number | null;
  notes: string;
  cover_key: string | null;
  sleeve_asset: number | null;
  tags: string;
  colors: string;
  card_count: number;
  warnings: number;
  created_at: number;
  updated_at: number;
  pinned?: number;
}

export class DeckService {
  constructor(
    private app: AppDb,
    private cards: CardDb,
    readonly precons: PreconService,
  ) {}

  resolveDefs(keys: Iterable<string>): Record<string, CardDef> {
    const p: string[] = [];
    const c: number[] = [];
    for (const k of keys) {
      if (k.startsWith('p:')) p.push(k.slice(2));
      else if (k.startsWith('c:')) c.push(parseInt(k.slice(2), 10));
    }
    const out = this.cards.defsByPrinting(p);
    if (c.length) {
      for (const r of all<CustomRow>(this.app.db, `SELECT * FROM custom_cards WHERE id IN (${c.map(() => '?').join(',')})`, ...c)) {
        const d = customDef(r);
        out[d.key] = d;
      }
    }
    return out;
  }

  private toCard(r: CardRow): DeckCard {
    return {
      id: r.id,
      section: r.section as Section,
      def: r.card_key,
      qty: r.qty,
      foil: !!r.foil,
      tags: JSON.parse(r.tags || '[]'),
      addedAt: r.added_at,
      frontAsset: r.front_asset,
      backAsset: r.back_asset,
    };
  }

  private coverImg(r: DeckRow, cards: DeckCard[], defs: Record<string, CardDef>): string | null {
    const key = r.cover_key || cards.find((c) => c.section === 'commander')?.def || cards.find((c) => c.section === 'main')?.def;
    return key && defs[key]?.img[0] ? defs[key].img[0] : null;
  }

  /** Applies the owner's "apply to all my decks" image overrides to cards without their own. */
  private applyUserOverrides(ownerId: number, cards: DeckCard[], defs: Record<string, CardDef>) {
    const rows = all<{ oracle_id: string; front_asset: number | null; back_asset: number | null }>(this.app.db, 'SELECT * FROM user_overrides WHERE user_id = ?', ownerId);
    if (!rows.length) return;
    const by = new Map(rows.map((r) => [r.oracle_id, r]));
    for (const c of cards) {
      if (c.frontAsset || c.backAsset) continue;
      const o = defs[c.def]?.oracleId && by.get(defs[c.def].oracleId!);
      if (o) {
        c.frontAsset = o.front_asset;
        c.backAsset = o.back_asset;
      }
    }
  }

  getDeck(id: number, viewerId: number): Deck | null {
    if (PreconService.isPreconId(id)) return this.precons.getDeck(id);
    const r = get<DeckRow>(
      this.app.db,
      `SELECT d.*, u.username AS owner_name, (SELECT 1 FROM deck_pins p WHERE p.deck_id = d.id AND p.user_id = ?) AS pinned
       FROM decks d JOIN users u ON u.id = d.owner_id WHERE d.id = ?`,
      viewerId,
      id,
    );
    if (!r) return null;
    const cards = all<CardRow>(this.app.db, 'SELECT * FROM deck_cards WHERE deck_id = ? ORDER BY id', id).map((x) => this.toCard(x));
    const keys = new Set(cards.map((c) => c.def));
    if (r.cover_key) keys.add(r.cover_key);
    const defs = this.resolveDefs(keys);
    this.applyUserOverrides(r.owner_id, cards, defs);
    this.applyGlobalTags(r.owner_id, cards, defs);
    return {
      id: r.id,
      ownerId: r.owner_id,
      ownerName: r.owner_name,
      name: r.name,
      format: r.format,
      folderId: r.folder_id,
      colors: r.colors ? r.colors.split('') : [],
      cardCount: r.card_count,
      warnings: r.warnings,
      coverImg: this.coverImg(r, cards, defs),
      pinned: !!r.pinned,
      updatedAt: r.updated_at,
      notes: r.notes,
      tags: this.allTags(r.id, r.owner_id),
      recentTags: this.recentTags(viewerId),
      coverKey: r.cover_key,
      sleeveAsset: r.sleeve_asset,
      createdAt: r.created_at,
      cards,
      defs,
    };
  }

  listDecks(viewerId: number): DeckSummary[] {
    const rows = all<DeckRow & { cover_img: string | null }>(
      this.app.db,
      `SELECT d.*, u.username AS owner_name, (SELECT 1 FROM deck_pins p WHERE p.deck_id = d.id AND p.user_id = ?) AS pinned,
        COALESCE(d.cover_key, (SELECT card_key FROM deck_cards c WHERE c.deck_id = d.id AND c.section = 'commander' ORDER BY id LIMIT 1),
                 (SELECT card_key FROM deck_cards c WHERE c.deck_id = d.id AND c.section = 'main' ORDER BY id LIMIT 1)) AS cover_img
       FROM decks d JOIN users u ON u.id = d.owner_id ORDER BY d.updated_at DESC`,
      viewerId,
    );
    const defs = this.resolveDefs(rows.map((r) => r.cover_img).filter(Boolean) as string[]);
    return rows.map((r) => ({
      id: r.id,
      ownerId: r.owner_id,
      ownerName: r.owner_name,
      name: r.name,
      format: r.format,
      folderId: r.folder_id,
      colors: r.colors ? r.colors.split('') : [],
      cardCount: r.card_count,
      warnings: r.warnings,
      coverImg: r.cover_img ? (defs[r.cover_img]?.img[0] ?? null) : null,
      pinned: !!r.pinned,
      updatedAt: r.updated_at,
    }));
  }

  /**
   * Recomputes every deck's cached warning count when the legality rules changed (bump LEGALITY_VERSION with them),
   * without touching when the decks were edited.
   */
  refreshWarnings() {
    if (this.app.setting('legalityVersion', '') === LEGALITY_VERSION) return;
    for (const { id } of all<{ id: number }>(this.app.db, 'SELECT id FROM decks')) this.recompute(id, false);
    this.app.setSetting('legalityVersion', LEGALITY_VERSION);
  }

  /** Recomputes cached summary columns (colours, count, warnings). */
  recompute(deckId: number, touch = true) {
    const d = get<DeckRow>(this.app.db, `SELECT d.*, '' AS owner_name FROM decks d WHERE id = ?`, deckId);
    if (!d) return;
    const cards = all<CardRow>(this.app.db, 'SELECT * FROM deck_cards WHERE deck_id = ?', deckId).map((x) => this.toCard(x));
    const defs = this.resolveDefs(new Set(cards.map((c) => c.def)));
    const leg = checkLegality(d.format, cards, defs);
    const cmd = cards.filter((c) => c.section === 'commander');
    const src = cmd.length ? cmd : cards.filter((c) => c.section === 'main');
    const cols = new Set<string>();
    src.forEach((c) => defs[c.def]?.colorIdentity.forEach((x) => cols.add(x)));
    const count = cards.filter((c) => c.section === 'main' || c.section === 'commander').reduce((s, c) => s + c.qty, 0);
    run(
      this.app.db,
      'UPDATE decks SET colors = ?, card_count = ?, warnings = ?, updated_at = ? WHERE id = ?',
      COLORS.filter((c) => cols.has(c)).join(''),
      count,
      leg.warnings.length,
      touch ? Date.now() : d.updated_at,
      deckId,
    );
  }

  canEdit(deckId: number, user: { id: number; isAdmin: boolean }): boolean {
    const r = get<{ owner_id: number }>(this.app.db, 'SELECT owner_id FROM decks WHERE id = ?', deckId);
    return !!r && (r.owner_id === user.id || user.isAdmin);
  }

  create(ownerId: number, name: string, format: string, folderId: number | null = null): number {
    const now = Date.now();
    const r = run(this.app.db, 'INSERT INTO decks (owner_id, name, format, folder_id, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)', ownerId, name, format, folderId, now, now);
    return Number(r.lastInsertRowid);
  }

  update(id: number, patch: { name?: string; format?: string; notes?: string; folderId?: number | null; coverKey?: string | null; sleeveAsset?: number | null }) {
    const sets: string[] = [];
    const vals: unknown[] = [];
    const add = (col: string, v: unknown) => {
      sets.push(`${col} = ?`);
      vals.push(v);
    };
    if (patch.name != null) add('name', patch.name);
    if (patch.format != null) add('format', patch.format);
    if (patch.notes != null) add('notes', patch.notes);
    if (patch.folderId !== undefined) add('folder_id', patch.folderId);
    if (patch.coverKey !== undefined) add('cover_key', patch.coverKey);
    if (patch.sleeveAsset !== undefined) add('sleeve_asset', patch.sleeveAsset);
    if (!sets.length) return;
    add('updated_at', Date.now());
    run(this.app.db, `UPDATE decks SET ${sets.join(', ')} WHERE id = ?`, ...vals, id);
    this.recompute(id);
  }

  addCard(deckId: number, key: string, section: Section, qty = 1, opts: { foil?: boolean; tags?: string[] } = {}): number {
    if (!SECTIONS.includes(section)) throw new Error('Bad section');
    const ex = get<{ id: number }>(this.app.db, 'SELECT id FROM deck_cards WHERE deck_id = ? AND card_key = ? AND section = ? AND foil = ?', deckId, key, section, opts.foil ? 1 : 0);
    let id: number;
    if (ex) {
      run(this.app.db, 'UPDATE deck_cards SET qty = qty + ? WHERE id = ?', qty, ex.id);
      id = ex.id;
    } else {
      const r = run(
        this.app.db,
        'INSERT INTO deck_cards (deck_id, section, card_key, qty, foil, tags, added_at) VALUES (?, ?, ?, ?, ?, ?, ?)',
        deckId,
        section,
        key,
        qty,
        opts.foil ? 1 : 0,
        JSON.stringify(cleanTags(opts.tags || [])),
        Date.now(),
      );
      id = Number(r.lastInsertRowid);
    }
    return id;
  }

  updateCard(deckId: number, cardId: number, p: { qty?: number; section?: Section; foil?: boolean; tags?: string[]; key?: string; frontAsset?: number | null; backAsset?: number | null }) {
    const row = get<CardRow>(this.app.db, 'SELECT * FROM deck_cards WHERE id = ? AND deck_id = ?', cardId, deckId);
    if (!row) throw new Error('Card not in deck');
    if (p.qty != null && p.qty <= 0) {
      run(this.app.db, 'DELETE FROM deck_cards WHERE id = ?', cardId);
      return;
    }
    if (p.section && !SECTIONS.includes(p.section)) throw new Error('Bad section');
    // Moving onto an entry that already holds this card (same printing, section and foil) merges the two.
    const key = p.key ?? row.card_key;
    const section = p.section ?? row.section;
    const foil = p.foil != null ? (p.foil ? 1 : 0) : row.foil;
    if (key !== row.card_key || section !== row.section || foil !== row.foil) {
      const ex = get<{ id: number }>(this.app.db, 'SELECT id FROM deck_cards WHERE deck_id = ? AND card_key = ? AND section = ? AND foil = ? AND id != ?', deckId, key, section, foil, cardId);
      if (ex) {
        tx(this.app.db, () => {
          run(this.app.db, 'UPDATE deck_cards SET qty = qty + ? WHERE id = ?', p.qty ?? row.qty, ex.id);
          run(this.app.db, 'DELETE FROM deck_cards WHERE id = ?', cardId);
        });
        return;
      }
    }
    run(
      this.app.db,
      'UPDATE deck_cards SET qty = ?, section = ?, foil = ?, tags = ?, card_key = ?, front_asset = ?, back_asset = ? WHERE id = ?',
      p.qty ?? row.qty,
      p.section ?? row.section,
      p.foil != null ? (p.foil ? 1 : 0) : row.foil,
      p.tags ? JSON.stringify(cleanTags(p.tags)) : row.tags,
      p.key ?? row.card_key,
      p.frontAsset !== undefined ? p.frontAsset : row.front_asset,
      p.backAsset !== undefined ? p.backAsset : row.back_asset,
      cardId,
    );
  }

  removeCard(deckId: number, cardId: number) {
    run(this.app.db, 'DELETE FROM deck_cards WHERE id = ? AND deck_id = ?', cardId, deckId);
  }

  replaceCards(deckId: number, cards: { key: string; section: Section; qty: number; foil?: boolean; tags?: string[] }[]) {
    tx(this.app.db, () => {
      run(this.app.db, 'DELETE FROM deck_cards WHERE deck_id = ?', deckId);
      for (const c of cards) this.addCard(deckId, c.key, c.section, c.qty, { foil: c.foil, tags: c.tags });
    });
    this.recompute(deckId);
  }

  /** Whether a deck (one of the users' or a bundled precon) exists. */
  exists(deckId: number): boolean {
    if (PreconService.isPreconId(deckId)) return !!this.precons.contents(deckId);
    return !!get(this.app.db, 'SELECT 1 FROM decks WHERE id = ?', deckId);
  }

  clone(deckId: number, newOwner: number): number {
    if (PreconService.isPreconId(deckId)) return this.clonePrecon(deckId, newOwner);
    return tx(this.app.db, () => {
      const d = get<DeckRow>(this.app.db, `SELECT *, '' AS owner_name FROM decks WHERE id = ?`, deckId);
      if (!d) throw new Error('Deck not found');
      const now = Date.now();
      const r = run(
        this.app.db,
        'INSERT INTO decks (owner_id, name, format, notes, tags, cover_key, sleeve_asset, colors, card_count, warnings, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)',
        newOwner,
        d.owner_id === newOwner ? `${d.name} (copy)` : d.name,
        d.format,
        d.notes,
        d.tags,
        d.cover_key,
        d.sleeve_asset,
        d.colors,
        d.card_count,
        d.warnings,
        now,
        now,
      );
      const id = Number(r.lastInsertRowid);
      run(
        this.app.db,
        'INSERT INTO deck_cards (deck_id, section, card_key, qty, foil, tags, added_at, front_asset, back_asset) SELECT ?, section, card_key, qty, foil, tags, added_at, front_asset, back_asset FROM deck_cards WHERE deck_id = ?',
        id,
        deckId,
      );
      // Someone else's copy keeps the owner's global tags, as its own deck tags.
      if (d.owner_id !== newOwner) {
        const rows = this.rowsWithKeys(id);
        const cards = rows.map(({ row }) => ({ ...this.toCard(row), tags: [] as string[] }));
        this.applyGlobalTags(d.owner_id, cards, this.resolveDefs(new Set(cards.map((c) => c.def))));
        const byId = new Map(cards.map((c) => [c.id, c.tags]));
        this.mapCardTags(id, (tags, cid) => [...tags, ...(byId.get(cid) || [])]);
        const used = new Set(cards.flatMap((c) => c.tags));
        const list = this.deckTags(id);
        const extra = this.library(d.owner_id).filter((t) => used.has(t.name) && !list.some((x) => x.name === t.name));
        if (extra.length) this.saveDeckTags(id, [...list, ...extra]);
      }
      return id;
    });
  }

  // ---------------------------------------------------------------- tags
  // Deck tags live on deck_cards.tags, listed (order, colour) in decks.tags. Global tags belong to the deck's owner:
  // user_tags lists them, user_card_tags ties one to a card (by oracle id), so it shows on that card in all their decks.

  private ownerOf(deckId: number): number {
    return get<{ owner_id: number }>(this.app.db, 'SELECT owner_id FROM decks WHERE id = ?', deckId)!.owner_id;
  }

  private deckTags(deckId: number): { name: string; color: string | null }[] {
    return JSON.parse(get<{ tags: string }>(this.app.db, 'SELECT tags FROM decks WHERE id = ?', deckId)?.tags || '[]');
  }

  private saveDeckTags(deckId: number, tags: { name: string; color: string | null }[]) {
    run(this.app.db, 'UPDATE decks SET tags = ?, updated_at = ? WHERE id = ?', JSON.stringify(tags.slice(0, MAX_DECK_TAGS).map((t) => ({ name: t.name, color: t.color }))), Date.now(), deckId);
  }

  private library(userId: number): { name: string; color: string | null }[] {
    return all<{ name: string; color: string | null }>(this.app.db, 'SELECT name, color FROM user_tags WHERE user_id = ? ORDER BY sort, name', userId);
  }

  private saveLibrary(userId: number, tags: { name: string; color: string | null }[]) {
    run(this.app.db, 'DELETE FROM user_tags WHERE user_id = ?', userId);
    tags.slice(0, MAX_DECK_TAGS * 3).forEach((t, i) => run(this.app.db, 'INSERT OR IGNORE INTO user_tags (user_id, name, color, sort) VALUES (?, ?, ?, ?)', userId, t.name, t.color, i));
  }

  /** What a global tag is attached to: the card's oracle id, so every printing gets it. */
  private cardKey(def: CardDef | undefined, key: string): string {
    return def?.oracleId ? 'o:' + def.oracleId : key;
  }

  private rowsWithKeys(deckId: number, ids?: Set<number>) {
    const rows = all<CardRow>(this.app.db, 'SELECT * FROM deck_cards WHERE deck_id = ?', deckId).filter((r) => !ids || ids.has(r.id));
    const defs = this.resolveDefs(new Set(rows.map((r) => r.card_key)));
    return rows.map((r) => ({ row: r, key: this.cardKey(defs[r.card_key], r.card_key) }));
  }

  /** Adds the owner's global tags to the cards (in place). */
  private applyGlobalTags(ownerId: number, cards: DeckCard[], defs: Record<string, CardDef>) {
    const by = new Map<string, string[]>();
    for (const r of all<{ card: string; tag: string }>(this.app.db, 'SELECT card, tag FROM user_card_tags WHERE user_id = ?', ownerId)) by.set(r.card, [...(by.get(r.card) || []), r.tag]);
    if (!by.size) return;
    for (const c of cards) {
      const g = by.get(this.cardKey(defs[c.def], c.def));
      if (g) c.tags = Array.from(new Set([...c.tags, ...g]));
    }
  }

  private recentTags(userId: number): RecentTag[] {
    return all<RecentTag>(this.app.db, 'SELECT name, scope FROM user_recent_tags WHERE user_id = ? ORDER BY used_at DESC LIMIT ?', userId, RECENT_TAGS);
  }

  private touchRecent(userId: number, tags: RecentTag[]) {
    const now = Date.now();
    tags.forEach((t, i) =>
      run(this.app.db, 'INSERT INTO user_recent_tags (user_id, name, scope, used_at) VALUES (?, ?, ?, ?) ON CONFLICT(user_id, name) DO UPDATE SET scope = excluded.scope, used_at = excluded.used_at', userId, t.name, t.scope, now + i),
    );
    run(this.app.db, 'DELETE FROM user_recent_tags WHERE user_id = ? AND name NOT IN (SELECT name FROM user_recent_tags WHERE user_id = ? ORDER BY used_at DESC LIMIT 50)', userId, userId);
  }

  /** This deck's own tags, then the owner's global tags. */
  private allTags(deckId: number, ownerId: number): DeckTag[] {
    const own = this.deckTags(deckId);
    const names = new Set(own.map((t) => t.name));
    return [...own.map((t) => ({ ...t, scope: 'deck' as const })), ...this.library(ownerId).filter((t) => !names.has(t.name)).map((t) => ({ ...t, scope: 'global' as const }))];
  }

  /** A tag's scope in this deck; `fallback` for a brand-new tag. */
  private scopeOf(deckId: number, ownerId: number, name: string, fallback: TagScope): TagScope {
    if (this.deckTags(deckId).some((t) => t.name === name)) return 'deck';
    if (this.library(ownerId).some((t) => t.name === name)) return 'global';
    if (all<{ tags: string }>(this.app.db, 'SELECT tags FROM deck_cards WHERE deck_id = ?', deckId).some((r) => JSON.parse(r.tags || '[]').includes(name))) return 'deck';
    return fallback;
  }

  /** Rewrites the deck tags of every card in the deck (only rows that change are written). */
  private mapCardTags(deckId: number, fn: (tags: string[], cardId: number) => string[]) {
    for (const r of all<{ id: number; tags: string }>(this.app.db, 'SELECT id, tags FROM deck_cards WHERE deck_id = ?', deckId)) {
      const before: string[] = JSON.parse(r.tags || '[]');
      const after = cleanTags(fn(before, r.id));
      if (after.join('\n') !== before.join('\n')) run(this.app.db, 'UPDATE deck_cards SET tags = ? WHERE id = ?', JSON.stringify(after), r.id);
    }
  }

  /** Sets order and colours: deck tags on the deck, global tags on the owner's list. */
  setTags(deckId: number, tags: { name: string; color: string | null; scope: TagScope }[]) {
    const seen = new Set<string>();
    const clean: { name: string; color: string | null; scope: TagScope }[] = [];
    for (const t of tags) {
      const name = normalizeTag(t.name);
      if (!name || seen.has(name)) continue;
      seen.add(name);
      clean.push({ name, color: t.color || null, scope: t.scope });
    }
    tx(this.app.db, () => {
      this.saveDeckTags(deckId, clean.filter((t) => t.scope === 'deck'));
      const owner = this.ownerOf(deckId);
      const global = clean.filter((t) => t.scope === 'global');
      const kept = new Set(global.map((t) => t.name));
      // Global tags left out (not sent) keep their place at the end.
      this.saveLibrary(owner, [...global, ...this.library(owner).filter((t) => !kept.has(t.name))]);
    });
  }

  /**
   * Adds and removes tags on some cards. A tag keeps its scope; a new one is global unless asked otherwise.
   * Adding or removing a global tag changes that card in all the owner's decks.
   */
  tagCards(deckId: number, actorId: number, cardIds: number[], add: { name: string; scope?: TagScope }[], remove: string[]) {
    const owner = this.ownerOf(deckId);
    tx(this.app.db, () => {
      const adds: RecentTag[] = [];
      for (const a of add) {
        const name = normalizeTag(a.name);
        if (name && !adds.some((x) => x.name === name)) adds.push({ name, scope: this.scopeOf(deckId, owner, name, a.scope ?? 'global') });
      }
      const rm = new Set(remove.map(normalizeTag));
      const ids = new Set(cardIds);
      const deckAdd = adds.filter((t) => t.scope === 'deck').map((t) => t.name);
      const globalAdd = adds.filter((t) => t.scope === 'global').map((t) => t.name);
      this.mapCardTags(deckId, (tags, id) => (ids.has(id) ? [...tags.filter((t) => !rm.has(t)), ...deckAdd] : tags));
      for (const { key } of this.rowsWithKeys(deckId, ids)) {
        for (const t of rm) run(this.app.db, 'DELETE FROM user_card_tags WHERE user_id = ? AND card = ? AND tag = ?', owner, key, t);
        for (const t of globalAdd) run(this.app.db, 'INSERT OR IGNORE INTO user_card_tags (user_id, card, tag) VALUES (?, ?, ?)', owner, key, t);
      }
      const list = this.deckTags(deckId);
      const newDeck = deckAdd.filter((t) => !list.some((x) => x.name === t));
      if (newDeck.length) this.saveDeckTags(deckId, [...list, ...newDeck.map((name) => ({ name, color: null }))]);
      const lib = this.library(owner);
      const newGlobal = globalAdd.filter((t) => !lib.some((x) => x.name === t));
      if (newGlobal.length) this.saveLibrary(owner, [...lib, ...newGlobal.map((name) => ({ name, color: null }))]);
      if (adds.length) this.touchRecent(actorId, adds);
    });
  }

  /** Makes one card's tags (deck and global) exactly `tags`. */
  setCardTags(deckId: number, actorId: number, cardId: number, tags: string[]) {
    const c = this.getDeck(deckId, actorId)?.cards.find((x) => x.id === cardId);
    if (!c) throw new Error('Card not in deck');
    const want = cleanTags(tags);
    this.tagCards(
      deckId,
      actorId,
      [cardId],
      want.filter((t) => !c.tags.includes(t)).map((name) => ({ name })),
      c.tags.filter((t) => !want.includes(t)),
    );
  }

  /** Renames a tag (a global one in all the owner's decks). Renaming onto an existing tag merges the two. */
  renameTag(deckId: number, from: string, to: string) {
    const f = normalizeTag(from);
    const t = normalizeTag(to);
    if (!f || !t || f === t) return;
    const owner = this.ownerOf(deckId);
    tx(this.app.db, () => {
      if (this.scopeOf(deckId, owner, f, 'deck') === 'deck') {
        this.mapCardTags(deckId, (tags) => tags.map((x) => (x === f ? t : x)));
        const list = this.deckTags(deckId);
        this.saveDeckTags(deckId, list.some((x) => x.name === t) ? list.filter((x) => x.name !== f) : list.map((x) => (x.name === f ? { ...x, name: t } : x)));
        return;
      }
      run(this.app.db, 'UPDATE OR IGNORE user_card_tags SET tag = ? WHERE user_id = ? AND tag = ?', t, owner, f);
      run(this.app.db, 'DELETE FROM user_card_tags WHERE user_id = ? AND tag = ?', owner, f);
      const lib = this.library(owner);
      this.saveLibrary(owner, lib.some((x) => x.name === t) ? lib.filter((x) => x.name !== f) : lib.map((x) => (x.name === f ? { ...x, name: t } : x)));
      run(this.app.db, 'UPDATE OR IGNORE user_recent_tags SET name = ? WHERE user_id = ? AND name = ?', t, owner, f);
      run(this.app.db, 'DELETE FROM user_recent_tags WHERE user_id = ? AND name = ?', owner, f);
    });
  }

  /** Removes a tag from every card (a global one from all the owner's decks) and from its list. */
  deleteTag(deckId: number, name: string) {
    const n = normalizeTag(name);
    const owner = this.ownerOf(deckId);
    tx(this.app.db, () => {
      if (this.scopeOf(deckId, owner, n, 'deck') === 'global') {
        run(this.app.db, 'DELETE FROM user_card_tags WHERE user_id = ? AND tag = ?', owner, n);
        this.saveLibrary(owner, this.library(owner).filter((x) => x.name !== n));
        run(this.app.db, 'DELETE FROM user_recent_tags WHERE user_id = ? AND name = ?', owner, n);
      }
      this.mapCardTags(deckId, (tags) => tags.filter((x) => x !== n));
      this.saveDeckTags(deckId, this.deckTags(deckId).filter((x) => x.name !== n));
    });
  }

  /**
   * Switches a tag between global and this-deck-only without losing anything:
   * deck → global moves this deck's uses onto the cards; global → deck copies it onto the cards in every deck of the owner.
   */
  setScope(deckId: number, name: string, scope: TagScope) {
    const n = normalizeTag(name);
    const owner = this.ownerOf(deckId);
    if (!n || this.scopeOf(deckId, owner, n, 'deck') === scope) return;
    tx(this.app.db, () => {
      if (scope === 'global') {
        const color = this.deckTags(deckId).find((x) => x.name === n)?.color ?? null;
        for (const { row, key } of this.rowsWithKeys(deckId)) if (JSON.parse(row.tags || '[]').includes(n)) run(this.app.db, 'INSERT OR IGNORE INTO user_card_tags (user_id, card, tag) VALUES (?, ?, ?)', owner, key, n);
        this.mapCardTags(deckId, (tags) => tags.filter((x) => x !== n));
        this.saveDeckTags(deckId, this.deckTags(deckId).filter((x) => x.name !== n));
        const lib = this.library(owner);
        if (!lib.some((x) => x.name === n)) this.saveLibrary(owner, [...lib, { name: n, color }]);
      } else {
        const color = this.library(owner).find((x) => x.name === n)?.color ?? null;
        const keys = new Set(all<{ card: string }>(this.app.db, 'SELECT card FROM user_card_tags WHERE user_id = ? AND tag = ?', owner, n).map((r) => r.card));
        for (const d of all<{ id: number }>(this.app.db, 'SELECT id FROM decks WHERE owner_id = ?', owner)) {
          const ids = new Set(this.rowsWithKeys(d.id).filter((x) => keys.has(x.key)).map((x) => x.row.id));
          if (!ids.size && d.id !== deckId) continue;
          this.mapCardTags(d.id, (tags, id) => (ids.has(id) ? [...tags, n] : tags));
          const list = this.deckTags(d.id);
          if (!list.some((x) => x.name === n)) this.saveDeckTags(d.id, [...list, { name: n, color }]);
        }
        run(this.app.db, 'DELETE FROM user_card_tags WHERE user_id = ? AND tag = ?', owner, n);
        this.saveLibrary(owner, this.library(owner).filter((x) => x.name !== n));
      }
      run(this.app.db, 'UPDATE user_recent_tags SET scope = ? WHERE user_id = ? AND name = ?', scope, owner, n);
    });
  }

  /** Attaches global tags from an import to the deck owner's cards. */
  addGlobalTags(deckId: number, cards: { key: string; tags: string[] }[]) {
    const owner = this.ownerOf(deckId);
    const defs = this.resolveDefs(new Set(cards.map((c) => c.key)));
    const names = new Set<string>();
    tx(this.app.db, () => {
      for (const c of cards)
        for (const t of cleanTags(c.tags)) {
          run(this.app.db, 'INSERT OR IGNORE INTO user_card_tags (user_id, card, tag) VALUES (?, ?, ?)', owner, this.cardKey(defs[c.key], c.key), t);
          names.add(t);
        }
      const lib = this.library(owner);
      const fresh = [...names].filter((t) => !lib.some((x) => x.name === t));
      if (fresh.length) this.saveLibrary(owner, [...lib, ...fresh.map((name) => ({ name, color: null }))]);
    });
  }

  remove(deckId: number) {
    run(this.app.db, 'DELETE FROM decks WHERE id = ?', deckId);
  }

  setPinned(userId: number, deckId: number, pinned: boolean) {
    if (pinned) run(this.app.db, 'INSERT OR IGNORE INTO deck_pins (user_id, deck_id) VALUES (?, ?)', userId, deckId);
    else run(this.app.db, 'DELETE FROM deck_pins WHERE user_id = ? AND deck_id = ?', userId, deckId);
  }

  setUserOverride(userId: number, oracleId: string, front: number | null, back: number | null) {
    if (!front && !back) run(this.app.db, 'DELETE FROM user_overrides WHERE user_id = ? AND oracle_id = ?', userId, oracleId);
    else
      run(
        this.app.db,
        'INSERT INTO user_overrides (user_id, oracle_id, front_asset, back_asset) VALUES (?, ?, ?, ?) ON CONFLICT(user_id, oracle_id) DO UPDATE SET front_asset = excluded.front_asset, back_asset = excluded.back_asset',
        userId,
        oracleId,
        front,
        back,
      );
  }

  private clonePrecon(deckId: number, newOwner: number): number {
    const p = this.precons.contents(deckId);
    if (!p) throw new Error('Deck not found');
    const id = this.create(newOwner, p.name, p.format);
    this.replaceCards(
      id,
      p.cards.map((c) => ({ key: c.def, section: c.section, qty: c.qty, foil: false, tags: [] })),
    );
    return id;
  }

  /** Deck contents for a game: defs with image overrides folded in (as variant keys) so everyone sees them. */
  forGame(deckId: number, cardsOverride?: { def: string; section: Section; qty: number }[]): { name: string; sleeve: string | null; cards: { def: string; section: Section; qty: number }[]; defs: Record<string, CardDef> } | null {
    if (PreconService.isPreconId(deckId)) {
      const p = this.precons.contents(deckId);
      return p && { name: p.name, sleeve: null, cards: cardsOverride ?? p.cards.map((c) => ({ def: c.def, section: c.section, qty: c.qty })), defs: { ...p.defs } };
    }
    const d = get<DeckRow>(this.app.db, `SELECT *, '' AS owner_name FROM decks WHERE id = ?`, deckId);
    if (!d) return null;
    const rows = all<CardRow>(this.app.db, 'SELECT * FROM deck_cards WHERE deck_id = ?', deckId).map((x) => this.toCard(x));
    const defs = this.resolveDefs(new Set(rows.map((c) => c.def)));
    this.applyUserOverrides(d.owner_id, rows, defs);
    const out: { def: string; section: Section; qty: number }[] = [];
    for (const c of rows) {
      let key = c.def;
      const base = defs[key];
      if (!base) continue;
      if (c.frontAsset || c.backAsset) {
        key = `${c.def}|o${c.frontAsset || 0}.${c.backAsset || 0}`;
        const img = base.img.slice();
        if (c.frontAsset) img[0] = 'a/' + c.frontAsset;
        if (c.backAsset) img[1] = 'a/' + c.backAsset;
        defs[key] = { ...base, key, img };
      }
      out.push({ def: key, section: c.section, qty: c.qty });
    }
    return { name: d.name, sleeve: d.sleeve_asset ? 'a/' + d.sleeve_asset : null, cards: cardsOverride ?? out, defs };
  }
}
