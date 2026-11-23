// A subset of Scryfall search syntax: t: o: c: id: mv: pow: tou: r: s: is: f:
// Terms are ANDed; `or` between terms creates alternatives; `-` negates; quotes group words.

import type { CardDef } from './cards';
import { COLORS } from './cards';

export type Op = ':' | '=' | '>=' | '<=' | '>' | '<' | '!=';

export type SearchTerm =
  | { kind: 'name'; value: string; neg: boolean }
  | { kind: 'field'; field: Field; op: Op; value: string; neg: boolean };

export type Field = 't' | 'o' | 'c' | 'id' | 'mv' | 'pow' | 'tou' | 'r' | 's' | 'is' | 'f';

/** Disjunctive normal form: OR of AND-groups. */
export type SearchQuery = SearchTerm[][];

const ALIASES: Record<string, Field> = {
  t: 't',
  type: 't',
  o: 'o',
  oracle: 'o',
  text: 'o',
  c: 'c',
  color: 'c',
  colour: 'c',
  id: 'id',
  identity: 'id',
  ci: 'id',
  mv: 'mv',
  cmc: 'mv',
  manavalue: 'mv',
  pow: 'pow',
  power: 'pow',
  tou: 'tou',
  toughness: 'tou',
  r: 'r',
  rarity: 'r',
  s: 's',
  set: 's',
  e: 's',
  edition: 's',
  is: 'is',
  f: 'f',
  format: 'f',
  legal: 'f',
};

function tokenize(q: string): string[] {
  const out: string[] = [];
  let i = 0;
  while (i < q.length) {
    if (/\s/.test(q[i])) {
      i++;
      continue;
    }
    let tok = '';
    while (i < q.length && !/\s/.test(q[i])) {
      if (q[i] === '"') {
        i++;
        while (i < q.length && q[i] !== '"') tok += q[i++];
        i++;
      } else tok += q[i++];
    }
    out.push(tok);
  }
  return out;
}

export function parseSearch(input: string): SearchQuery {
  const groups: SearchTerm[][] = [[]];
  for (let tok of tokenize(input)) {
    if (tok.toLowerCase() === 'or') {
      groups.push([]);
      continue;
    }
    let neg = false;
    if (tok.startsWith('-') && tok.length > 1) {
      neg = true;
      tok = tok.slice(1);
    }
    const m = tok.match(/^([a-zA-Z]+)(:|>=|<=|!=|=|>|<)(.+)$/);
    const field = m ? ALIASES[m[1].toLowerCase()] : undefined;
    if (m && field) groups[groups.length - 1].push({ kind: 'field', field, op: m[2] as Op, value: m[3].toLowerCase(), neg });
    else groups[groups.length - 1].push({ kind: 'name', value: tok.toLowerCase(), neg });
  }
  return groups.filter((g) => g.length > 0);
}

const COLOR_WORDS: Record<string, string> = {
  white: 'w',
  blue: 'u',
  black: 'b',
  red: 'r',
  green: 'g',
  colorless: 'c',
  azorius: 'wu',
  dimir: 'ub',
  rakdos: 'br',
  gruul: 'rg',
  selesnya: 'gw',
  orzhov: 'wb',
  izzet: 'ur',
  golgari: 'bg',
  boros: 'rw',
  simic: 'gu',
  esper: 'wub',
  grixis: 'ubr',
  jund: 'brg',
  naya: 'rgw',
  bant: 'gwu',
  abzan: 'wbg',
  jeskai: 'urw',
  sultai: 'bgu',
  mardu: 'rwb',
  temur: 'gur',
  wubrg: 'wubrg',
};

/** Parses a colour value into a set of upper-case colour letters, or 'C' for colourless. Multicolour 'm' returns null. */
export function parseColors(v: string): string[] | 'M' | null {
  const w = COLOR_WORDS[v] ?? v;
  if (w === 'm' || w === 'multicolor') return 'M';
  if (!/^[wubrgc]+$/.test(w)) return null;
  if (w.includes('c')) return [];
  return Array.from(new Set(w.toUpperCase().split(''))).filter((c) => (COLORS as readonly string[]).includes(c));
}

export const RARITY_CODES: Record<string, string> = { c: 'common', u: 'uncommon', r: 'rare', m: 'mythic', s: 'special' };

function cmpNum(a: number, op: Op, b: number): boolean {
  switch (op) {
    case '>':
      return a > b;
    case '<':
      return a < b;
    case '>=':
      return a >= b;
    case '<=':
      return a <= b;
    case '!=':
      return a !== b;
    default:
      return a === b;
  }
}

function colorMatch(have: string[], want: string[] | 'M', op: Op, identity: boolean): boolean {
  if (want === 'M') return have.length > 1;
  const H = new Set(have);
  const sup = want.every((c) => H.has(c));
  const sub = have.every((c) => want.includes(c));
  if (want.length === 0) return have.length === 0; // colourless
  switch (op) {
    case '=':
      return sup && sub;
    case '>=':
      return sup;
    case '<=':
      return sub;
    case '>':
      return sup && !sub;
    case '<':
      return sub && !sup;
    case '!=':
      return !(sup && sub);
    default:
      return identity ? sub : sup; // Scryfall: c: means "at least", id: means "within"
  }
}

function isMatch(def: CardDef, v: string): boolean {
  switch (v) {
    case 'commander':
      return (/Legendary/.test(def.typeLine) && /Creature|Planeswalker/.test(def.typeLine)) || /can be your commander/i.test(def.oracleText);
    case 'token':
      return !!def.isToken;
    case 'legendary':
      return /Legendary/.test(def.typeLine);
    case 'dfc':
    case 'mdfc':
    case 'transform':
      return def.img.length > 1;
    case 'split':
      return def.layout === 'split';
    case 'flip':
      return def.layout === 'flip';
    case 'custom':
      return !!def.custom;
    case 'permanent':
      return !/Instant|Sorcery/.test(def.typeLine);
    case 'spell':
      return !/\bLand\b/.test(def.typeLine);
    default:
      return false;
  }
}

/** In-memory matcher (used for custom cards and tests). The server compiles the same AST to SQL for the card DB. */
export function matchTerm(def: CardDef, t: SearchTerm): boolean {
  let r: boolean;
  if (t.kind === 'name') r = def.name.toLowerCase().includes(t.value);
  else {
    const v = t.value;
    switch (t.field) {
      case 't':
        r = def.typeLine.toLowerCase().includes(v);
        break;
      case 'o':
        r = def.oracleText.toLowerCase().includes(v) || (def.faces || []).some((f) => f.oracleText.toLowerCase().includes(v));
        break;
      case 'c': {
        const want = parseColors(v);
        r = want == null ? false : colorMatch(def.colors, want, t.op, false);
        break;
      }
      case 'id': {
        const want = parseColors(v);
        r = want == null ? false : colorMatch(def.colorIdentity, want, t.op, true);
        break;
      }
      case 'mv':
        r = cmpNum(def.cmc, t.op, parseFloat(v));
        break;
      case 'pow':
        r = def.power != null && cmpNum(parseFloat(def.power), t.op, parseFloat(v));
        break;
      case 'tou':
        r = def.toughness != null && cmpNum(parseFloat(def.toughness), t.op, parseFloat(v));
        break;
      case 'r': {
        const want = RARITY_CODES[v] || v;
        r = (def.rarity || '').startsWith(want);
        break;
      }
      case 's':
        r = (def.set || '').toLowerCase() === v;
        break;
      case 'is':
        r = isMatch(def, v);
        break;
      case 'f': {
        if (def.custom) r = true;
        else {
          const l = def.legalities?.[v];
          r = l === 'legal' || l === 'restricted';
        }
        break;
      }
      default:
        r = true;
    }
  }
  return t.neg ? !r : r;
}

export function matchQuery(def: CardDef, q: SearchQuery): boolean {
  if (!q.length) return true;
  return q.some((g) => g.every((t) => matchTerm(def, t)));
}
