#!/usr/bin/env node
// Builds decks/precons.json, the bundled list of official preconstructed decks, from MTGJSON (https://mtgjson.com).
//
// usage: node scripts/precons.mjs [dir]
//   dir: an unpacked MTGJSON AllDeckFiles archive. Without it, DeckList.json and every included deck file are
//   downloaded (cached in $TMPDIR/aetherhub-precons, so a re-run only fetches new decks).
//
// Output: {"source": ..., "decks": [...]}, one deck per line. Each deck is
//   {id, file, name, code, type, date, format, cards: [[section, qty, name, set, number], ...]}
// `id` is negative and derived from the MTGJSON file name, so a deck keeps its id when the file is regenerated
// (lobbies and saved games refer to decks by id).
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const BASE = 'https://mtgjson.com/api/v5';
const OUT = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', 'decks', 'precons.json');

/** Complete decks meant to be played as they come. Card bundles, half decks, land packs and promos are left out. */
const TYPES = new Set([
  'Commander Deck',
  'MTGO Commander Deck',
  'Historic Brawl Precon Deck',
  'Brawl Deck',
  'Theme Deck',
  'MTGO Theme Deck',
  'Intro Pack',
  'Planeswalker Deck',
  'Duel Deck',
  'MTGO Duel Deck',
  'Duel Of The Planeswalkers Deck',
  'Challenger Deck',
  'Pioneer Challenger Deck',
  'Event Deck',
  'Modern Event Deck',
  'Game Night Deck',
  'Clash Pack',
  'Guild Kit',
  'Premium Deck',
  'Enhanced Deck',
  'Advanced Deck',
  'Starter Deck',
  'Starter Kit',
  'Spellslinger Starter Kit',
  'Welcome Deck',
  'Sample Deck',
  'Halfdeck',
  'Arena Starter Deck',
  'Arena Starter Kit',
  'Arena Promotional Deck',
  'World Championship Deck',
  'Pro Tour Deck',
  'Planechase Deck',
  'Archenemy Deck',
  'Challenge Deck',
  'Dandan Deck',
]);

const SECTIONS = [
  ['commander', 'commander'],
  ['main', 'mainBoard'],
  ['side', 'sideBoard'],
  ['planar', 'planes'],
  ['schemes', 'schemes'],
];

function formatOf(type, commanders, mainCount) {
  if (type === 'Brawl Deck') return 'standardbrawl';
  if (type === 'Historic Brawl Precon Deck') return 'brawl';
  if (commanders && mainCount + commanders >= 95) return 'commander';
  return 'freeform';
}

/** FNV-1a: a stable id in -1..-2^30 for a deck file name. */
function idOf(file) {
  let h = 0x811c9dc5;
  for (const ch of file) {
    h ^= ch.codePointAt(0);
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return -((h & 0x3fffffff) + 1);
}

async function fetchJson(url, cacheFile) {
  if (cacheFile && fs.existsSync(cacheFile) && fs.statSync(cacheFile).size) return JSON.parse(fs.readFileSync(cacheFile, 'utf8'));
  const res = await fetch(url, { headers: { 'User-Agent': 'aetherhub-precon-import' } });
  if (!res.ok) throw new Error(`${url}: HTTP ${res.status}`);
  const text = await res.text();
  if (cacheFile) fs.writeFileSync(cacheFile, text);
  await new Promise((r) => setTimeout(r, 100));
  return JSON.parse(text);
}

async function loadDecks(dir) {
  if (dir) {
    return fs
      .readdirSync(dir)
      .filter((f) => f.endsWith('.json'))
      .map((f) => ({ file: f.slice(0, -5), data: JSON.parse(fs.readFileSync(path.join(dir, f), 'utf8')).data }))
      .filter((d) => TYPES.has(d.data.type));
  }
  const cache = path.join(os.tmpdir(), 'aetherhub-precons');
  fs.mkdirSync(cache, { recursive: true });
  const list = (await fetchJson(`${BASE}/DeckList.json`)).data.filter((d) => TYPES.has(d.type));
  const out = [];
  for (const [i, d] of list.entries()) {
    out.push({ file: d.fileName, data: (await fetchJson(`${BASE}/decks/${d.fileName}.json`, path.join(cache, d.fileName + '.json'))).data });
    if ((i + 1) % 100 === 0) console.log(`${i + 1} / ${list.length}`);
  }
  return out;
}

const src = process.argv[2];
const meta = await fetchJson(`${BASE}/Meta.json`).then((m) => m.data, () => null);
const decks = [];
const ids = new Map();
for (const { file, data: d } of await loadDecks(src)) {
  const cards = [];
  for (const [section, key] of SECTIONS) for (const c of d[key] || []) cards.push([section, c.count || 1, c.name, c.setCode, c.number]);
  const count = (s) => cards.filter((c) => c[0] === s).reduce((n, c) => n + c[1], 0);
  if (!count('main') && !count('commander')) continue;
  const id = idOf(file);
  if (ids.has(id)) throw new Error(`id clash: ${file} and ${ids.get(id)}`);
  ids.set(id, file);
  decks.push({ id, file, name: d.name, code: d.code, type: d.type, date: d.releaseDate ?? null, format: formatOf(d.type, count('commander'), count('main')), cards });
}
decks.sort((a, b) => (b.date ?? '').localeCompare(a.date ?? '') || a.name.localeCompare(b.name));
const source = `MTGJSON ${meta?.version ?? '(version unknown)'}`;
fs.writeFileSync(OUT, `{"source":${JSON.stringify(source)},"decks":[\n${decks.map((d) => JSON.stringify(d)).join(',\n')}\n]}\n`);
console.log(`${decks.length} decks written to ${path.relative(process.cwd(), OUT)} (${(fs.statSync(OUT).size / 1e6).toFixed(1)} MB)`);
