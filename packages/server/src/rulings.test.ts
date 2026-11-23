import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { Readable } from 'node:stream';
import { describe, expect, it } from 'vitest';
import { importRulings, RulingsDb } from './db/rulings';

const lines = [
  { oracle_id: 'a', source: 'wotc', published_at: '2022-10-14', comment: 'The mana value of a melded permanent is the sum of the mana values of its front faces.' },
  { oracle_id: 'a', source: 'wotc', published_at: '2022-10-14', comment: 'Urza’s first ability can’t reduce colored mana costs.' },
  { oracle_id: 'b', source: 'scryfall', published_at: '2025-07-25', comment: 'Station abilities put charge counters on the Spacecraft.' },
];

describe('rulings', () => {
  it('imports JSONL (and JSON arrays) and searches by text', async () => {
    const file = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'aether-rul-')), 'rulings.sqlite');
    // Mix both bulk formats: the first line opens a JSON array, the rest are JSONL.
    const text = '[' + lines.map((l) => JSON.stringify(l)).join(',\n') + ']\n';
    expect(await importRulings(Readable.from([text]), file)).toBe(3);
    const db = new RulingsDb(file);
    expect(db.ready).toBe(true);
    expect(db.forOracle('a').map((r) => r.date)).toEqual(['2022-10-14', '2022-10-14']);
    expect(db.counts(['a', 'b', 'zzz'])).toEqual({ a: 2, b: 1 });
    expect(db.searchText('charge counters').map((r) => r.oracleId)).toEqual(['b']);
    expect(db.searchText('melded').map((r) => r.oracleId)).toEqual(['a']);
    // Prefix match on the last word, apostrophes and punctuation are safe in queries.
    expect(db.searchText('Urza’s first abil').map((r) => r.oracleId)).toEqual(['a']);
    expect(db.searchText('"; DROP TABLE rulings; --')).toEqual([]);
    db.close();
  });
});
