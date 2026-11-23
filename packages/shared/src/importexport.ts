// Deck list parsing (Moxfield / Archidekt / MTGGoldfish plain text, Arena text, MTGO .dek) and export.

import type { CardDef } from './cards';
import { normalizeTag, type DeckCard, type Section } from './deck';

export type ImportFormat = 'plain' | 'arena' | 'mtgo';

export interface ParsedLine {
  line: number;
  raw: string;
  qty: number;
  name: string;
  set?: string;
  number?: string;
  foil: boolean;
  section: Section;
  /** Deck tags ("#tag", "[Tag]"). */
  tags: string[];
  /** Global tags ("#!tag"). */
  globalTags: string[];
  /** false when the line didn't look like "N Card name" at all. */
  wellFormed: boolean;
}

export interface ParseResult {
  format: ImportFormat;
  lines: ParsedLine[];
  lineCount: number;
}

const HEADERS: Record<string, Section> = {
  commander: 'commander',
  commanders: 'commander',
  companion: 'companion',
  main: 'main',
  mainboard: 'main',
  deck: 'main',
  'main deck': 'main',
  sideboard: 'side',
  side: 'side',
  maybeboard: 'maybe',
  maybe: 'maybe',
  considering: 'maybe',
  tokens: 'tokens',
  token: 'tokens',
  extras: 'tokens',
  'tokens/extras': 'tokens',
  planar: 'planar',
  planes: 'planar',
  'planar deck': 'planar',
  schemes: 'schemes',
  'scheme deck': 'schemes',
  // joke-set Attractions and stickers are not supported: imported into the maybeboard
  attractions: 'maybe',
  stickers: 'maybe',
};

export function detectFormat(text: string): ImportFormat {
  if (/<Deck[\s>]|<Cards\s/i.test(text)) return 'mtgo';
  if (/^\s*Deck\s*$/m.test(text) && !/^\s*Main(board)?\s*:?\s*$/im.test(text)) return 'arena';
  return 'plain';
}

function headerOf(line: string): Section | null {
  const h = line
    .replace(/^\/\/\s*/, '')
    .replace(/^#+\s*/, '')
    .replace(/:$/, '')
    .replace(/\s*\(\d+\)\s*$/, '')
    .trim()
    .toLowerCase();
  return HEADERS[h] ?? null;
}

function xmlAttr(tag: string, name: string): string | null {
  const m = tag.match(new RegExp(`\\b${name}\\s*=\\s*"([^"]*)"`, 'i'));
  return m ? m[1] : null;
}

function decodeXml(s: string): string {
  return s.replace(/&quot;/g, '"').replace(/&apos;/g, "'").replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&amp;/g, '&');
}

export function parseDeckText(text: string): ParseResult {
  const format = detectFormat(text);
  const lines: ParsedLine[] = [];
  const raw = text.replace(/\r\n?/g, '\n').split('\n');
  if (format === 'mtgo') {
    raw.forEach((l, i) => {
      const m = l.match(/<Cards\s[^>]*>/i);
      if (!m) return;
      const qty = parseInt(xmlAttr(m[0], 'Quantity') || '1', 10);
      const name = decodeXml(xmlAttr(m[0], 'Name') || '');
      const sb = (xmlAttr(m[0], 'Sideboard') || 'false').toLowerCase() === 'true';
      lines.push({ line: i + 1, raw: l.trim(), qty, name, foil: false, section: sb ? 'side' : 'main', tags: [], globalTags: [], wellFormed: !!name });
    });
    return { format, lines, lineCount: lines.length };
  }

  const anyHeader = raw.some((l) => l.trim() && headerOf(l.trim()));
  let section: Section = 'main';
  let seenCards = false;
  let lineCount = 0;
  raw.forEach((l, i) => {
    const line = l.trim();
    if (!line) {
      // MTGGoldfish/Arena-without-headers convention: a blank line after the main deck starts the sideboard.
      if (!anyHeader && seenCards && section === 'main') section = 'side';
      // A commander/companion block ends at a blank line; unlabelled cards after it are the main deck.
      if (seenCards && (section === 'commander' || section === 'companion')) section = 'main';
      return;
    }
    lineCount++;
    const h = headerOf(line);
    if (h) {
      section = h;
      return;
    }
    if (/^SIDEBOARD:?$/i.test(line)) {
      section = 'side';
      return;
    }
    // "1x Name (SET) 123 *F* [Tag, Tag] ^Tag^"  |  "1 Name (SET) 123 #Tag #!Global tag"  |  "1 Name"  |  "SB: 1 Name"
    let body = line;
    let sec = section;
    if (/^SB:\s*/i.test(body)) {
      sec = 'side';
      body = body.replace(/^SB:\s*/i, '');
    }
    const tags: string[] = [];
    const globalTags: string[] = [];
    const addTag = (x: string) => {
      const t = normalizeTag(x);
      const to = x.startsWith('!') ? globalTags : tags;
      if (t && !tags.includes(t) && !globalTags.includes(t)) to.push(t);
    };
    body = body.replace(/\[([^\]]*)\]/g, (_m, t: string) => {
      t.split(',').forEach((x) => addTag(x.replace(/\{.*\}$/, '')));
      return '';
    });
    // Moxfield tags run to the end of the line; card names never contain " #".
    body = body.replace(/\s#.*$/, (t) => {
      t.split(/\s#/).forEach(addTag);
      return '';
    });
    body = body.replace(/\^[^^]*\^/g, '');
    let foil = false;
    body = body.replace(/\s\*(F|E|Foil|Etched)\*/gi, () => {
      foil = true;
      return '';
    });
    body = body.trim();
    const m = body.match(/^(\d+)\s*x?\s+(.+?)(?:\s+[([<]([A-Za-z0-9]{2,6})[)\]>](?:\s+([A-Za-z0-9★\-]+))?)?$/);
    seenCards = true;
    if (!m) {
      lines.push({ line: i + 1, raw: line, qty: 1, name: body, foil, section: sec, tags, globalTags, wellFormed: false });
      return;
    }
    lines.push({
      line: i + 1,
      raw: line,
      qty: parseInt(m[1], 10),
      name: m[2].trim(),
      set: m[3] ? m[3].toLowerCase() : undefined,
      number: m[4],
      foil,
      section: sec,
      tags,
      globalTags,
      wellFormed: true,
    });
  });
  return { format, lines, lineCount };
}

// ---------------------------------------------------------------- export

export type ExportFormat = 'moxfield' | 'arena' | 'mtgo';

export interface ExportOptions {
  /** Moxfield-style " #tag" suffixes (plain text only). */
  tags?: boolean;
  /** Tags written as Moxfield global tags, " #!tag". */
  globalTags?: Set<string>;
}

function lineFor(c: DeckCard, d: CardDef, fmt: ExportFormat, o: ExportOptions): string {
  const setPart = d.set && d.collectorNumber ? ` (${d.set.toUpperCase()}) ${d.collectorNumber}` : d.set ? ` (${d.set.toUpperCase()})` : '';
  if (fmt === 'arena') return `${c.qty} ${d.name}${setPart}`;
  const tags = o.tags ? c.tags.map((t) => (o.globalTags?.has(t) ? ' #!' : ' #') + t).join('') : '';
  return `${c.qty} ${d.name}${setPart}${c.foil ? ' *F*' : ''}${tags}`;
}

function xmlEsc(s: string): string {
  return s.replace(/&/g, '&amp;').replace(/"/g, '&quot;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

export function exportDeck(cards: DeckCard[], defs: Record<string, CardDef>, fmt: ExportFormat, o: ExportOptions = {}): string {
  const by = (s: Section) => cards.filter((c) => c.section === s && defs[c.def]);
  if (fmt === 'mtgo') {
    const rows = cards
      .filter((c) => defs[c.def] && !['maybe', 'tokens', 'planar', 'schemes', 'attractions', 'stickers'].includes(c.section))
      .map((c) => `  <Cards Quantity="${c.qty}" Sideboard="${c.section === 'side' ? 'true' : 'false'}" Name="${xmlEsc(defs[c.def].name)}" />`);
    return `<?xml version="1.0" encoding="utf-8"?>\n<Deck xmlns:xsd="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">\n  <NetDeckID>0</NetDeckID>\n  <PreconstructedDeckID>0</PreconstructedDeckID>\n${rows.join('\n')}\n</Deck>\n`;
  }
  const block = (title: string, s: Section) => {
    const l = by(s).map((c) => lineFor(c, defs[c.def], fmt, o));
    return l.length ? `${title}\n${l.join('\n')}\n\n` : '';
  };
  if (fmt === 'arena') return (block('Commander', 'commander') + block('Companion', 'companion') + block('Deck', 'main') + block('Sideboard', 'side')).trim() + '\n';
  return (
    block('Commander', 'commander') +
    block('Companion', 'companion') +
    block('Main', 'main') +
    block('Sideboard', 'side') +
    block('Maybeboard', 'maybe') +
    block('Tokens', 'tokens') +
    block('Planar', 'planar') +
    block('Schemes', 'schemes') +
    block('Attractions', 'attractions') +
    block('Stickers', 'stickers')
  ).trim() + '\n';
}

// ---------------------------------------------------------------- fuzzy

/** Levenshtein distance with an early-exit bound. */
export function levenshtein(a: string, b: string, max = Infinity): number {
  if (a === b) return 0;
  if (Math.abs(a.length - b.length) > max) return max + 1;
  let prev = new Array(b.length + 1);
  let cur = new Array(b.length + 1);
  for (let j = 0; j <= b.length; j++) prev[j] = j;
  for (let i = 1; i <= a.length; i++) {
    cur[0] = i;
    let rowMin = cur[0];
    for (let j = 1; j <= b.length; j++) {
      cur[j] = Math.min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + (a[i - 1] === b[j - 1] ? 0 : 1));
      if (cur[j] < rowMin) rowMin = cur[j];
    }
    if (rowMin > max) return max + 1;
    [prev, cur] = [cur, prev];
  }
  return prev[b.length];
}
