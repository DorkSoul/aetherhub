// Static card data shared by the deckbuilder, the search API and the game engine.

export interface CardFace {
  name: string;
  manaCost: string;
  typeLine: string;
  oracleText: string;
  power?: string;
  toughness?: string;
  loyalty?: string;
  defense?: string;
}

export interface RelatedPart {
  id: string;
  name: string;
  component: string;
  typeLine: string;
}

/**
 * A card definition. `key` identifies it inside decks and games:
 *  - `p:<printingId>` a Scryfall printing
 *  - `c:<customCardId>` a server-wide custom card
 *  - `t:<name>|<pt>|<colors>` an ad-hoc custom token made at the table
 */
export interface CardDef {
  key: string;
  printingId?: string;
  oracleId?: string;
  customId?: number;
  name: string;
  layout: string;
  manaCost: string;
  cmc: number;
  typeLine: string;
  oracleText: string;
  power?: string;
  toughness?: string;
  loyalty?: string;
  defense?: string;
  colors: string[];
  colorIdentity: string[];
  faces?: CardFace[];
  set?: string;
  setName?: string;
  collectorNumber?: string;
  rarity?: string;
  releasedAt?: string;
  /** Image references, one per face that has its own image: `p/<id>/<face>` or `a/<assetId>`. Empty = render a frame. */
  img: string[];
  isToken?: boolean;
  legalities?: Record<string, string>;
  usd?: number | null;
  eur?: number | null;
  allParts?: RelatedPart[];
  custom?: boolean;
}

export const COLORS = ['W', 'U', 'B', 'R', 'G'] as const;
export type Color = (typeof COLORS)[number];
export const COLOR_NAMES: Record<string, string> = { W: 'White', U: 'Blue', B: 'Black', R: 'Red', G: 'Green', C: 'Colorless' };

export function manaSymbols(cost: string | undefined | null): string[] {
  const out: string[] = [];
  String(cost || '').replace(/\{([^}]+)\}/g, (_m, s: string) => {
    out.push(s);
    return '';
  });
  return out;
}

export function manaValue(cost: string | undefined | null): number {
  let n = 0;
  for (const s of manaSymbols(cost)) {
    if (/^\d+$/.test(s)) n += parseInt(s, 10);
    else if (s === 'X' || s === 'Y' || s === 'Z') n += 0;
    else if (/^2\//.test(s)) n += 2;
    else n += 1;
  }
  return n;
}

/** Counts coloured pips; hybrid symbols count toward each of their colours. */
export function pipCounts(cost: string | undefined | null): Record<string, number> {
  const out: Record<string, number> = { W: 0, U: 0, B: 0, R: 0, G: 0 };
  for (const s of manaSymbols(cost)) for (const c of COLORS) if (s.includes(c)) out[c]++;
  return out;
}

export function isLand(def: Pick<CardDef, 'typeLine'>): boolean {
  const front = def.typeLine.split('//')[0];
  return /\bLand\b/.test(front) && !/\bCreature\b/.test(front);
}

export function isBasicLand(def: Pick<CardDef, 'typeLine'>): boolean {
  return /\bBasic\b/.test(def.typeLine) && /\bLand\b/.test(def.typeLine);
}

export function frontFace(def: CardDef): CardFace {
  if (def.faces && def.faces.length) return def.faces[0];
  return def;
}

export function hasBackImage(def: CardDef): boolean {
  return def.img.length > 1;
}

/** Card is shown rotated 90° in previews (split, battles, rooms, planes). */
/** Whether a card face is printed landscape (split cards and Rooms, planes, the front of a battle). */
export function faceSideways(def: CardDef, face = 0): boolean {
  if (def.layout === 'planar') return true;
  if (def.layout === 'split') return !/Aftermath/i.test(def.oracleText);
  const t = def.faces?.[face]?.typeLine ?? def.typeLine.split(' // ')[face] ?? def.typeLine;
  return /\bBattle\b/.test(t);
}

export function isSideways(def: CardDef): boolean {
  return faceSideways(def, 0);
}

/** The card a meld piece becomes, when this card is one of the halves. */
export function meldResult(def: CardDef): RelatedPart | undefined {
  if (def.layout !== 'meld') return undefined;
  return def.allParts?.find((p) => p.component === 'meld_result' && p.name !== def.name);
}

/**
 * How a card shows its other side: `dfc` turns over to a second image (transform, modal DFC, battle, custom back),
 * `flip` rotates 180° (Kamigawa flip cards), `meld` turns over to the melded card. null = one side only.
 */
export function turnKind(def: CardDef | null | undefined): 'dfc' | 'flip' | 'meld' | null {
  if (!def) return null;
  if (def.img.length > 1) return 'dfc';
  if (def.layout === 'flip') return 'flip';
  if (meldResult(def)) return 'meld';
  return null;
}

export function isFlipLayout(def: CardDef): boolean {
  return def.layout === 'flip';
}

/**
 * Whether the rules allow this card as a commander: a legendary creature, a legendary Vehicle or Spacecraft
 * with a power/toughness box (allowed since Edge of Eternities, 2025), a Background (next to a
 * "Choose a Background" commander), or a card that says it can be your commander. In Brawl any legendary
 * planeswalker also qualifies, and Oathbreaker has its own rule. Only used for warnings — anyone can put any card in the command zone.
 */
export function canBeCommander(def: CardDef, formatId?: string): boolean {
  if (/can be your commander/i.test(def.oracleText)) return true;
  const f = frontFace(def);
  const t = f.typeLine || def.typeLine;
  // Oathbreaker: any planeswalker, plus an instant or sorcery as the signature spell.
  if (formatId === 'oathbreaker') return /\b(Planeswalker|Instant|Sorcery)\b/.test(t);
  if (!/\bLegendary\b/.test(t)) return false;
  if (/\bCreature\b/.test(t) || /\bBackground\b/.test(t)) return true;
  if (/\b(Vehicle|Spacecraft)\b/.test(t) && (f.power ?? def.power) != null) return true;
  if (/\bPlaneswalker\b/.test(t) && (formatId === 'brawl' || formatId === 'standardbrawl')) return true;
  return false;
}

export function normName(s: string): string {
  return s
    .normalize('NFD')
    .replace(/[̀-ͯ]/g, '')
    .toLowerCase()
    .replace(/[’‘`´]/g, "'")
    .replace(/[^a-z0-9_' /,-]/g, ' ')
    .replace(/\s+/g, ' ')
    .trim();
}

export function typeGroup(typeLine: string): string {
  const t = typeLine.split('//')[0];
  if (/\bLand\b/.test(t) && !/\bCreature\b/.test(t)) return 'Lands';
  if (/\bCreature\b/.test(t)) return 'Creatures';
  if (/\bPlaneswalker\b/.test(t)) return 'Planeswalkers';
  if (/\bBattle\b/.test(t)) return 'Battles';
  if (/\bInstant\b/.test(t)) return 'Instants';
  if (/\bSorcery\b/.test(t)) return 'Sorceries';
  if (/\bArtifact\b/.test(t)) return 'Artifacts';
  if (/\bEnchantment\b/.test(t)) return 'Enchantments';
  return 'Other';
}

export const TYPE_ORDER = [
  'Commander',
  'Companion',
  'Creatures',
  'Planeswalkers',
  'Battles',
  'Instants',
  'Sorceries',
  'Artifacts',
  'Enchantments',
  'Lands',
  'Other',
];

export const RARITY_ORDER: Record<string, number> = { common: 0, uncommon: 1, rare: 2, mythic: 3, special: 4, bonus: 4 };
export const RARITY_NAME: Record<string, string> = {
  common: 'Common',
  uncommon: 'Uncommon',
  rare: 'Rare',
  mythic: 'Mythic',
  special: 'Special',
  bonus: 'Bonus',
};

export function basePT(def: CardDef | undefined, faceIndex = 0): [number, number] | null {
  if (!def) return null;
  const f = def.faces && def.faces[faceIndex] ? def.faces[faceIndex] : def;
  if (f.power == null || f.toughness == null) return null;
  const p = parseInt(f.power, 10);
  const t = parseInt(f.toughness, 10);
  return [isNaN(p) ? 0 : p, isNaN(t) ? 0 : t];
}
