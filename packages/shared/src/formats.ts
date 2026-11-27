export type FormatId =
  | 'commander'
  | 'standard'
  | 'pioneer'
  | 'modern'
  | 'legacy'
  | 'vintage'
  | 'pauper'
  | 'brawl'
  | 'standardbrawl'
  | 'oathbreaker'
  | 'freeform'
  | 'cube'
  | 'limited';

export interface FormatInfo {
  id: FormatId;
  name: string;
  life: number;
  commandZone: boolean;
  singleton: boolean;
  /** Exact deck size (commander-style) or minimum size (constructed); null = no size rule. */
  deckSize: number | null;
  exactSize: boolean;
  maxSideboard: number | null;
  /** Key into Scryfall `legalities`; null = no legality checks. */
  legalityKey: string | null;
}

export const FORMATS: FormatInfo[] = [
  { id: 'commander', name: 'Commander', life: 40, commandZone: true, singleton: true, deckSize: 100, exactSize: true, maxSideboard: null, legalityKey: 'commander' },
  { id: 'standard', name: 'Standard', life: 20, commandZone: false, singleton: false, deckSize: 60, exactSize: false, maxSideboard: 15, legalityKey: 'standard' },
  { id: 'pioneer', name: 'Pioneer', life: 20, commandZone: false, singleton: false, deckSize: 60, exactSize: false, maxSideboard: 15, legalityKey: 'pioneer' },
  { id: 'modern', name: 'Modern', life: 20, commandZone: false, singleton: false, deckSize: 60, exactSize: false, maxSideboard: 15, legalityKey: 'modern' },
  { id: 'legacy', name: 'Legacy', life: 20, commandZone: false, singleton: false, deckSize: 60, exactSize: false, maxSideboard: 15, legalityKey: 'legacy' },
  { id: 'vintage', name: 'Vintage', life: 20, commandZone: false, singleton: false, deckSize: 60, exactSize: false, maxSideboard: 15, legalityKey: 'vintage' },
  { id: 'pauper', name: 'Pauper', life: 20, commandZone: false, singleton: false, deckSize: 60, exactSize: false, maxSideboard: 15, legalityKey: 'pauper' },
  { id: 'brawl', name: 'Brawl', life: 25, commandZone: true, singleton: true, deckSize: 100, exactSize: true, maxSideboard: null, legalityKey: 'brawl' },
  { id: 'standardbrawl', name: 'Standard Brawl', life: 25, commandZone: true, singleton: true, deckSize: 60, exactSize: true, maxSideboard: null, legalityKey: 'standardbrawl' },
  { id: 'oathbreaker', name: 'Oathbreaker', life: 20, commandZone: true, singleton: true, deckSize: 60, exactSize: true, maxSideboard: null, legalityKey: 'oathbreaker' },
  { id: 'freeform', name: 'Freeform', life: 20, commandZone: false, singleton: false, deckSize: null, exactSize: false, maxSideboard: null, legalityKey: null },
  { id: 'cube', name: 'Cube', life: 20, commandZone: false, singleton: false, deckSize: null, exactSize: false, maxSideboard: null, legalityKey: null },
  // draft and sealed: at least 40 cards from the pool plus basic lands; the rest of the pool is the sideboard
  { id: 'limited', name: 'Limited', life: 20, commandZone: false, singleton: false, deckSize: 40, exactSize: false, maxSideboard: null, legalityKey: null },
];

export const FORMAT_BY_ID: Record<string, FormatInfo> = Object.fromEntries(FORMATS.map((f) => [f.id, f]));

export function formatInfo(id: string | null | undefined): FormatInfo {
  return FORMAT_BY_ID[id || 'commander'] || FORMAT_BY_ID.freeform;
}

export function formatName(id: string | null | undefined): string {
  return formatInfo(id).name;
}

