import type { CardDef } from './cards';

/**
 * Deck-building rules a commander changes: Rulebreaker (Magic: The Gathering — Mystery Booster 2 / playtest cards,
 * "A deck with this commander can have Angel cards of any color identity and any basic land cards") and older
 * "If this is your commander, your deck can …" cards. The legality check asks every commander of a deck for its
 * modifier and lets through what it allows.
 */
export interface DeckRuleMod {
  /** the commander's name, for "allowed by …" notes */
  source: string;
  /** what it allows, in a few words */
  summary: string;
  /** this card may be in the deck whatever its colour identity */
  anyIdentity?: (d: CardDef) => boolean;
  /** basic land cards of any colour identity */
  anyBasics?: boolean;
  /** this card may add exactly one colour outside the commander's identity, the same one for every such card (Tolabow) */
  oneExtraColor?: (d: CardDef) => boolean;
  /** no maximum deck size (the minimum stays) */
  noMaxSize?: boolean;
  /** a different minimum deck size */
  minSize?: number;
  /** a different starting life total */
  life?: number;
}

const front = (d: CardDef) => d.faces?.[0]?.typeLine ?? d.typeLine;
const types = (d: CardDef) => (d.faces?.length ? d.faces.map((f) => f.typeLine).join(' // ') : d.typeLine);
const has = (d: CardDef, re: RegExp) => re.test(types(d));
const isLandCard = (d: CardDef) => /\bLand\b/.test(front(d));
const isPermanentCard = (d: CardDef) => /\b(Artifact|Battle|Creature|Enchantment|Land|Planeswalker)\b/.test(front(d));
const num = (v: string | undefined) => {
  const n = parseFloat(v ?? '');
  return isNaN(n) ? -Infinity : n;
};

/** Known cards, by name. */
const KNOWN: Record<string, Omit<DeckRuleMod, 'source'>> = {
  'The Everforger': { summary: 'artifact creature and Equipment cards of any color identity, any basic lands', anyIdentity: (d) => has(d, /\bArtifact\b[^/]*\bCreature\b|\bCreature\b[^/]*\bArtifact\b/) || has(d, /\bEquipment\b/), anyBasics: true },
  'Seluma, Light of Aysen': { summary: 'Angel cards of any color identity, any basic lands', anyIdentity: (d) => has(d, /\bAngel\b/), anyBasics: true },
  'Valko Indorian': { summary: 'Phyrexian cards of any color identity, any basic lands', anyIdentity: (d) => has(d, /\bPhyrexian\b/), anyBasics: true },
  'The Unluckiest Planeswalker': { summary: 'Aura cards of any color identity, any basic lands', anyIdentity: (d) => has(d, /\bAura\b/), anyBasics: true },
  'Maular, the Next Evolution': { summary: 'creature cards with mana value 7 or greater of any color identity, any basic lands', anyIdentity: (d) => has(d, /\bCreature\b/) && d.cmc >= 7, anyBasics: true },
  'Grizzlegom, Hurloon Hero': { summary: 'any land cards', anyIdentity: isLandCard, anyBasics: true },
  'Whtz, the Bibliophile': { summary: 'no maximum deck size', noMaxSize: true },
  'Tolabow, Loch Rascal': { summary: 'instants and sorceries may add one chosen color, any basic lands', oneExtraColor: (d) => has(d, /\b(Instant|Sorcery)\b/), anyBasics: true },
  // Mystery Booster playtest versions
  'Arvad of the Weatherlight': { summary: 'legendary permanents of any color', anyIdentity: (d) => /\bLegendary\b/.test(front(d)) && isPermanentCard(d) },
  'Valko Indorian, Researcher': { summary: 'artifacts and enchantments of any color', anyIdentity: (d) => has(d, /\b(Artifact|Enchantment)\b/) },
  'Hadran, Naya Sunseeder': { summary: 'creature cards with power 4 or greater of any color', anyIdentity: (d) => has(d, /\bCreature\b/) && num(d.faces?.[0]?.power ?? d.power) >= 4 },
  'Daxiver, Izzet Electromancer': { summary: 'instant and sorcery cards of any color', anyIdentity: (d) => has(d, /\b(Instant|Sorcery)\b/) },
  // older "if this is your commander" cards
  'The Paradise Bird': { summary: 'cards of any color identity', anyIdentity: () => true, anyBasics: true },
  'The Waffle Restaurant': { summary: 'cards that can create Food tokens of any color identity, any basic lands', anyIdentity: (d) => /\bcreates?\b[^.]*\bFood\b/i.test(d.oracleText), anyBasics: true },
  'The Covert Blue Mage': { summary: 'no maximum deck size', noMaxSize: true },
  'The Zonian Brawler': { summary: 'minimum deck size 60, starting life 30', minSize: 60, life: 30 },
};

/** Generic reading of Rulebreaker text for cards not in the table (future printings). */
function fromText(d: CardDef): Omit<DeckRuleMod, 'source'> | null {
  const t = d.oracleText;
  if (!/Rulebreaker|is your commander|with this commander/i.test(t)) return null;
  const m: Omit<DeckRuleMod, 'source'> = { summary: '' };
  const parts: string[] = [];
  if (/no maximum deck size|any number of cards beyond the maximum deck size/i.test(t)) (m.noMaxSize = true), parts.push('no maximum deck size');
  if (/any basic land cards/i.test(t)) (m.anyBasics = true), parts.push('any basic lands');
  if (/can have any land cards/i.test(t)) (m.anyIdentity = isLandCard), parts.push('any land cards');
  if (/can include cards of any color identity/i.test(t)) (m.anyIdentity = () => true), parts.push('cards of any color identity');
  // "can have Angel cards of any color identity", "can have artifact creature and Equipment cards of any color identity"
  const typ = t.match(/can have ((?:[A-Z]?[a-z]+)(?:(?:,| and| or) (?:[A-Z]?[a-z]+))*) cards of any color identity/);
  if (typ && !m.anyIdentity) {
    const words = typ[1].split(/,| and | or /).map((w) => w.trim()).filter(Boolean);
    m.anyIdentity = (c) => words.some((w) => w.split(' ').every((x) => new RegExp(`\\b${x}\\b`, 'i').test(types(c))));
    parts.push(`${typ[1]} cards of any color identity`);
  }
  if (!parts.length) return null;
  m.summary = parts.join(', ');
  return m;
}

/** The deck-building modifiers of a deck's commanders (empty for ordinary commanders). */
export function commanderDeckRules(commanders: CardDef[]): DeckRuleMod[] {
  const out: DeckRuleMod[] = [];
  for (const d of commanders) {
    const m = KNOWN[d.name] ?? fromText(d);
    if (m) out.push({ source: d.name, ...m });
  }
  return out;
}
