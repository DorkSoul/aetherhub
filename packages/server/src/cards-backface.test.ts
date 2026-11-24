import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

process.env.DATA_DIR = fs.mkdtempSync(path.join(os.tmpdir(), 'aether-backface-'));

const { CardDb, importBulk } = await import('./db/cards');

const fixture = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../../fixtures/cards-fixture.jsonl');
let db: InstanceType<typeof CardDb>;

describe('back-face names', () => {
  beforeAll(async () => {
    const file = path.join(process.env.DATA_DIR!, 'cards.sqlite');
    await importBulk(fs.createReadStream(fixture), file);
    db = new CardDb(file);
  });
  afterAll(() => db.close());

  it('finds the card a back-face name belongs to', () => {
    const id = db.matchBackFace('Insectile Aberration');
    expect(id).toBeTruthy();
    const def = db.defsByPrinting([id!])['p:' + id];
    expect(def.name).toBe('Delver of Secrets // Insectile Aberration');
    expect(def.img).toHaveLength(2);
    expect(def.faces?.[1].name).toBe('Insectile Aberration');
  });

  it('finds nothing for a front-face or unknown name', () => {
    expect(db.matchBackFace('Delver of Secrets')).toBeNull();
    expect(db.matchBackFace('No Such Card 100%_')).toBeNull();
  });
});
