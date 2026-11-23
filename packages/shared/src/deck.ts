import type { CardDef } from './cards';
import { canBeCommander, isBasicLand, isLand, manaValue, pipCounts, typeGroup, COLOR_NAMES, RARITY_ORDER, RARITY_NAME, TYPE_ORDER } from './cards';
import { formatInfo } from './formats';
import { commanderDeckRules } from './deckrules';

export const SECTIONS = ['commander', 'companion', 'main', 'side', 'maybe', 'tokens', 'planar', 'schemes', 'attractions', 'stickers'] as const;
export type Section = (typeof SECTIONS)[number];

export const SECTION_NAMES: Record<Section, string> = {
  commander: 'Commander',
  companion: 'Companion',
  main: 'Main',
  side: 'Side',
  maybe: 'Maybeboard',
  tokens: 'Tokens/Extras',
  planar: 'Planar',
  schemes: 'Schemes',
  attractions: 'Attractions',
  stickers: 'Stickers',
};

/** Sections that are never flagged for legality and never shuffled into the library. */
export const EXTRA_SECTIONS: Section[] = ['maybe', 'tokens', 'planar', 'schemes', 'attractions', 'stickers'];

export interface DeckCard {
  id: number;
  section: Section;
  def: string; // CardDef key
  qty: number;
  foil: boolean;
  tags: string[];
  addedAt: number;
  frontAsset?: number | null;
  backAsset?: number | null;
}

export interface DeckSummary {
  id: number;
  ownerId: number;
  ownerName: string;
  name: string;
  format: string;
  folderId: number | null;
  colors: string[];
  cardCount: number;
  warnings: number;
  coverImg: string | null;
  pinned: boolean;
  updatedAt: number;
}

/** global: the owner's tag, follows the card into all their decks (Moxfield "#!tag"). deck: this deck only ("#tag"). */
export type TagScope = 'global' | 'deck';

/** A known tag: this deck's own tags, then the owner's global tags. Cards can also carry deck tags not listed yet. */
export interface DeckTag {
  name: string;
  color: string | null;
  scope: TagScope;
}

export interface RecentTag {
  name: string;
  scope: TagScope;
}

export interface Deck extends DeckSummary {
  notes: string;
  tags: DeckTag[];
  /** The viewer's most recently used tags, newest first. */
  recentTags: RecentTag[];
  coverKey: string | null;
  sleeveAsset: number | null;
  createdAt: number;
  cards: DeckCard[];
  defs: Record<string, CardDef>;
}

export const DEFAULT_TAGS = ['ramp', 'draw', 'removal', 'interaction', 'counters', 'finisher', 'combo'];
export const MAX_TAG_LEN = 30;
export const MAX_CARD_TAGS = 20;
export const MAX_DECK_TAGS = 100;
export const RECENT_TAGS = 13;
/** Tag chip colours; all light enough for dark text in either theme. */
export const TAG_COLORS = ['#e0b155', '#e8935a', '#e4787a', '#e69ac4', '#b39bf0', '#7db4ea', '#62c9bb', '#8ccb7d', '#b8b3a8'];

/** "#Mana  Rock" → "mana rock". Also strips Moxfield's "#!" global-tag marker. */
export function normalizeTag(s: string): string {
  return s
    .replace(/\s+/g, ' ')
    .trim()
    .replace(/^#*!?\s*/, '')
    .toLowerCase()
    .slice(0, MAX_TAG_LEN)
    .trim();
}

/** The tags in use in a deck: its own tags (in their order), global tags on its cards (in their order), then any others A–Z. */
export function deckTagNames(tags: DeckTag[], cards: DeckCard[]): string[] {
  const used = new Set(cards.flatMap((c) => c.tags));
  const listed = tags.filter((t) => t.scope === 'deck' || used.has(t.name)).map((t) => t.name);
  const have = new Set(listed);
  return [...listed, ...Array.from(used).filter((t) => !have.has(t)).sort()];
}

/** A tag's scope in a deck. Unlisted tags can only be on the deck's own cards. */
export function tagScope(tags: DeckTag[], name: string): TagScope {
  return tags.find((t) => t.name === name)?.scope ?? 'deck';
}

// ---------------------------------------------------------------- legality

export interface LegalityResult {
  /** Reasons per DeckCard id. */
  cards: Record<number, string[]>;
  /** Deck-level warnings, including per-card ones prefixed with the card name. */
  warnings: string[];
  deckWarnings: string[];
  /** Deck rules the commander changes (Rulebreaker and similar): "Seluma, Light of Aysen: Angel cards of any color identity, …". */
  notes: string[];
  /** Starting life when a commander changes it (The Zonian Brawler). */
  life?: number;
}

function copyLimitExempt(def: CardDef): number | null {
  if (isBasicLand(def)) return Infinity;
  if (/A deck can have any number of cards named/i.test(def.oracleText)) return Infinity;
  const m = def.oracleText.match(/A deck can have up to (\w+) cards named/i);
  if (m) {
    const words: Record<string, number> = { two: 2, three: 3, four: 4, five: 5, six: 6, seven: 7, eight: 8, nine: 9, ten: 10 };
    return words[m[1].toLowerCase()] ?? (parseInt(m[1], 10) || 4);
  }
  return null;
}

/**
 * Planechase and Archenemy decks: a planar deck has at least 10 cards, all with different names, and at most 2
 * phenomena; a scheme deck has at least 20 schemes, at most 2 of any one (checked only when the section has cards).
 */
function extraDeckWarnings(cards: DeckCard[], defs: Record<string, CardDef>): string[] {
  const out: string[] = [];
  const planar = cards.filter((c) => c.section === 'planar');
  if (planar.length) {
    const n = planar.reduce((k, c) => k + c.qty, 0);
    if (n < 10) out.push(`Planar deck has ${n} cards (at least 10)`);
    const byName = new Map<string, number>();
    for (const c of planar) if (defs[c.def]) byName.set(defs[c.def].name, (byName.get(defs[c.def].name) ?? 0) + c.qty);
    const dup = [...byName].filter(([, q]) => q > 1).map(([nm]) => nm);
    if (dup.length) out.push(`Planar deck: one copy of each card (${dup.join(', ')})`);
    const ph = planar.filter((c) => /\bPhenomenon\b/.test(defs[c.def]?.typeLine ?? '')).reduce((k, c) => k + c.qty, 0);
    if (ph > 2) out.push(`Planar deck has ${ph} phenomena (at most 2)`);
  }
  const schemes = cards.filter((c) => c.section === 'schemes');
  if (schemes.length) {
    const n = schemes.reduce((k, c) => k + c.qty, 0);
    if (n < 20) out.push(`Scheme deck has ${n} cards (at least 20)`);
    const byName = new Map<string, number>();
    for (const c of schemes) if (defs[c.def]) byName.set(defs[c.def].name, (byName.get(defs[c.def].name) ?? 0) + c.qty);
    const many = [...byName].filter(([, q]) => q > 2).map(([nm]) => nm);
    if (many.length) out.push(`Scheme deck: at most 2 of each scheme (${many.join(', ')})`);
  }
  return out;
}

export function checkLegality(format: string, cards: DeckCard[], defs: Record<string, CardDef>): LegalityResult {
  const f = formatInfo(format);
  const out: LegalityResult = { cards: {}, warnings: [], deckWarnings: [], notes: [] };
  const add = (c: DeckCard, r: string) => {
    (out.cards[c.id] = out.cards[c.id] || []).push(r);
  };
  // no legality list (freeform, cube, limited): only the deck size is checked
  if (!f.legalityKey) {
    const main = cards.filter((c) => c.section === 'main' || c.section === 'commander').reduce((n, c) => n + c.qty, 0);
    if (f.deckSize != null && main < f.deckSize) out.deckWarnings.push(`${main} of ${f.deckSize} cards`);
    out.deckWarnings.push(...extraDeckWarnings(cards, defs));
    out.warnings.push(...out.deckWarnings);
    return out;
  }

  const commanders = cards.filter((c) => c.section === 'commander');
  const ci = new Set<string>();
  commanders.forEach((c) => defs[c.def]?.colorIdentity.forEach((x) => ci.add(x)));
  // Rulebreaker and other commanders that change the deck-building rules
  const mods = f.commandZone ? commanderDeckRules(commanders.map((c) => defs[c.def]).filter((d): d is CardDef => !!d)) : [];
  out.notes = mods.map((m) => `${m.source}: ${m.summary}`);
  out.life = mods.find((m) => m.life != null)?.life;
  /** the outside colour each "one extra colour" card (Tolabow) uses: they must all agree */
  const extra = new Map<number, string>();

  // copies by card name across deck sections that count
  const counted = cards.filter((c) => !EXTRA_SECTIONS.includes(c.section));
  const byName: Record<string, number> = {};
  counted.forEach((c) => {
    const d = defs[c.def];
    if (d) byName[d.name] = (byName[d.name] || 0) + c.qty;
  });

  for (const c of counted) {
    const d = defs[c.def];
    if (!d || d.custom || d.isToken) continue;
    const leg = d.legalities?.[f.legalityKey];
    if (leg === 'banned') add(c, `Banned in ${f.name}`);
    // a commander that changes the deck rules (Rulebreaker) only exists on casual playtest cards Scryfall lists as not
    // legal: playing it as the commander is the point, so it is not flagged
    else if ((leg === 'not_legal' || leg == null) && !(c.section === 'commander' && mods.some((m) => m.source === d.name))) add(c, `Not legal in ${f.name}`);
    else if (leg === 'restricted' && (byName[d.name] || 0) > 1) add(c, `Restricted in ${f.name} — 1 copy allowed`);

    if (f.commandZone && commanders.length && c.section !== 'side') {
      const bad = d.colorIdentity.filter((x) => !ci.has(x));
      const allowed = bad.length > 0 && mods.some((m) => m.anyIdentity?.(d) || (m.anyBasics && isBasicLand(d)));
      if (bad.length && !allowed && bad.length === 1 && mods.some((m) => m.oneExtraColor?.(d))) extra.set(c.id, bad[0]);
      else if (bad.length && !allowed) add(c, `Outside your commander's color identity (${bad.map((x) => COLOR_NAMES[x]).join(', ')})`);
    }
    const n = byName[d.name] || 0;
    const exempt = copyLimitExempt(d);
    if (f.singleton) {
      const lim = exempt ?? 1;
      if (n > lim) add(c, `${n} copies — singleton format allows ${lim === Infinity ? 'any' : lim}`);
    } else {
      const lim = exempt ?? 4;
      if (n > lim) add(c, `More than ${lim} copies`);
    }
    if (!f.commandZone && c.section === 'commander') add(c,`No command zone in ${f.name} — counts as a main deck card`);
    if (f.commandZone && c.section === 'commander' && !canBeCommander(d, f.id)) add(c, 'Not normally allowed as a commander (needs to be a legendary creature or say it can be your commander)');
  }
  // Tolabow-style: the cards may add one colour, but it has to be the same colour for all of them
  const extraColors = new Set(extra.values());
  if (extraColors.size > 1)
    for (const c of counted) if (extra.has(c.id)) add(c, `Only one extra color is allowed (this deck uses ${[...extraColors].map((x) => COLOR_NAMES[x]).join(', ')})`);
  // Dedupe reasons
  for (const k of Object.keys(out.cards)) out.cards[+k] = Array.from(new Set(out.cards[+k]));

  // Deck size
  const mainCount = cards.filter((c) => c.section === 'main' || c.section === 'commander').reduce((s, c) => s + c.qty, 0);
  if (f.deckSize != null) {
    const min = mods.find((m) => m.minSize != null)?.minSize ?? f.deckSize;
    const noMax = mods.some((m) => m.noMaxSize);
    if (min < f.deckSize && f.exactSize) {
      if (mainCount < min || mainCount > f.deckSize) out.deckWarnings.push(`${mainCount} cards (${min}–${f.deckSize} allowed)`);
    } else if (f.exactSize && !noMax && mainCount !== f.deckSize) out.deckWarnings.push(`${mainCount} of ${f.deckSize} cards`);
    else if ((!f.exactSize || noMax) && mainCount < min) out.deckWarnings.push(`${mainCount} of ${min} cards${noMax && f.exactSize ? ' (no maximum)' : ''}`);
  }
  if (f.maxSideboard != null) {
    const side = cards.filter((c) => c.section === 'side').reduce((s, c) => s + c.qty, 0);
    if (side > f.maxSideboard) out.deckWarnings.push(`Sideboard has ${side} cards (max ${f.maxSideboard})`);
  }
  if (f.commandZone && commanders.length === 0) out.deckWarnings.push('No commander chosen');
  out.deckWarnings.push(...extraDeckWarnings(cards, defs));

  for (const c of cards) {
    const r = out.cards[c.id];
    if (r && defs[c.def]) out.warnings.push(`${defs[c.def].name}: ${r.join(' · ')}`);
  }
  out.warnings.push(...out.deckWarnings);
  return out;
}

// ---------------------------------------------------------------- stats

export interface DeckStats {
  total: number;
  lands: number;
  nonland: number;
  avgMv: number;
  curve: number[]; // 0..7+
  pips: Record<string, number>;
  usd: number;
  eur: number;
  size: number | null;
  colors: string[];
}

export function deckStats(format: string, cards: DeckCard[], defs: Record<string, CardDef>): DeckStats {
  const f = formatInfo(format);
  const main = cards.filter((c) => c.section === 'main' || c.section === 'commander');
  const s: DeckStats = { total: 0, lands: 0, nonland: 0, avgMv: 0, curve: [0, 0, 0, 0, 0, 0, 0, 0], pips: { W: 0, U: 0, B: 0, R: 0, G: 0 }, usd: 0, eur: 0, size: f.deckSize, colors: [] };
  let mvSum = 0;
  const cols = new Set<string>();
  for (const c of main) {
    const d = defs[c.def];
    if (!d) continue;
    s.total += c.qty;
    s.usd += (d.usd || 0) * c.qty;
    s.eur += (d.eur || 0) * c.qty;
    d.colorIdentity.forEach((x) => cols.add(x));
    if (isLand(d)) {
      s.lands += c.qty;
      continue;
    }
    s.nonland += c.qty;
    const mv = d.cmc ?? manaValue(d.manaCost);
    mvSum += mv * c.qty;
    s.curve[Math.min(7, Math.floor(mv))] += c.qty;
    const cost = d.faces ? d.faces.map((x) => x.manaCost).join('') : d.manaCost;
    const p = pipCounts(cost);
    for (const k of Object.keys(p)) s.pips[k] += p[k] * c.qty;
  }
  s.avgMv = s.nonland ? mvSum / s.nonland : 0;
  s.colors = ['W', 'U', 'B', 'R', 'G'].filter((c) => cols.has(c));
  return s;
}

// ---------------------------------------------------------------- grouping & sorting

export type GroupBy = 'type' | 'mv' | 'color' | 'ci' | 'rarity' | 'set' | 'tag' | 'section' | 'none';
export type SortBy = 'name' | 'mv' | 'color' | 'type' | 'rarity' | 'price' | 'added';

function colorKey(d: CardDef): string {
  const c = d.colors;
  return c.length === 0 ? 'Z' : c.length > 1 ? 'Y' + c.join('') : c[0];
}

export function sortCards(list: DeckCard[], defs: Record<string, CardDef>, by: SortBy): DeckCard[] {
  const name = (a: DeckCard, b: DeckCard) => (defs[a.def]?.name || '').localeCompare(defs[b.def]?.name || '');
  const fns: Record<SortBy, (a: DeckCard, b: DeckCard) => number> = {
    name,
    mv: (a, b) => (defs[a.def]?.cmc ?? 0) - (defs[b.def]?.cmc ?? 0) || name(a, b),
    color: (a, b) => colorKey(defs[a.def]).localeCompare(colorKey(defs[b.def])) || name(a, b),
    type: (a, b) => (defs[a.def]?.typeLine || '').localeCompare(defs[b.def]?.typeLine || '') || name(a, b),
    rarity: (a, b) => (RARITY_ORDER[defs[b.def]?.rarity || ''] ?? 0) - (RARITY_ORDER[defs[a.def]?.rarity || ''] ?? 0) || name(a, b),
    price: (a, b) => (defs[b.def]?.usd || 0) * b.qty - (defs[a.def]?.usd || 0) * a.qty || name(a, b),
    added: (a, b) => b.addedAt - a.addedAt || name(a, b),
  };
  return list.slice().sort(fns[by]);
}

export function groupKey(c: DeckCard, d: CardDef | undefined, by: GroupBy): string {
  if (!d) return 'Unknown';
  if (c.section === 'commander' && by !== 'section') return 'Commander';
  if (c.section === 'companion' && by !== 'section') return 'Companion';
  switch (by) {
    case 'type':
      return typeGroup(d.typeLine);
    case 'mv':
      return isLand(d) ? 'Lands' : 'MV ' + (d.cmc >= 7 ? '7+' : String(Math.floor(d.cmc)));
    case 'color':
      if (isLand(d)) return 'Lands';
      return d.colors.length === 0 ? 'Colorless' : d.colors.length > 1 ? 'Multicolor' : COLOR_NAMES[d.colors[0]];
    case 'ci':
      return d.colorIdentity.length ? d.colorIdentity.join('') : 'Colorless';
    case 'rarity':
      return RARITY_NAME[d.rarity || ''] || 'Other';
    case 'set':
      return (d.set || 'custom').toUpperCase();
    case 'tag':
      return groupKeys(c, d, by)[0];
    case 'section':
      return SECTION_NAMES[c.section];
    default:
      return 'All cards';
  }
}

/** Every group a card belongs in. Only tag grouping puts a card in more than one: one group per tag, like Moxfield. */
export function groupKeys(c: DeckCard, d: CardDef | undefined, by: GroupBy): string[] {
  if (by !== 'tag' || !d || c.section === 'commander' || c.section === 'companion') return [groupKey(c, d, by)];
  return c.tags.length ? c.tags.map((t) => '#' + t) : ['Untagged'];
}

/** `tagOrder` sets the order of tag groups (the deck's tag list); other groups sort A–Z. */
export function groupCards(list: DeckCard[], defs: Record<string, CardDef>, by: GroupBy, sort: SortBy, tagOrder: string[] = []) {
  const sorted = sortCards(list, defs, sort);
  const map = new Map<string, DeckCard[]>();
  for (const c of sorted) {
    for (const k of groupKeys(c, defs[c.def], by)) {
      if (!map.has(k)) map.set(k, []);
      map.get(k)!.push(c);
    }
  }
  const tagIdx = (k: string) => (k.startsWith('#') ? tagOrder.indexOf(k.slice(1)) : -1);
  const keys = Array.from(map.keys()).sort((a, b) => {
    const fixed = ['Commander', 'Companion'];
    if (fixed.includes(a) || fixed.includes(b)) return (fixed.includes(a) ? fixed.indexOf(a) : 9) - (fixed.includes(b) ? fixed.indexOf(b) : 9);
    if (a === 'Untagged' || b === 'Untagged') return a === 'Untagged' ? 1 : -1;
    const ta = tagIdx(a);
    const tb = tagIdx(b);
    if (ta >= 0 || tb >= 0) return ta < 0 ? 1 : tb < 0 ? -1 : ta - tb;
    const ia = TYPE_ORDER.indexOf(a);
    const ib = TYPE_ORDER.indexOf(b);
    if (ia >= 0 && ib >= 0) return ia - ib;
    if (a === 'Lands') return 1;
    if (b === 'Lands') return -1;
    return a.localeCompare(b, undefined, { numeric: true });
  });
  return keys.map((k) => ({ title: k, cards: map.get(k)!, count: map.get(k)!.reduce((s, c) => s + c.qty, 0) }));
}
