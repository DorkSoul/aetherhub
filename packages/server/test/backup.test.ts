import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { afterAll, describe, expect, it } from 'vitest';

const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'aetherhub-backup-'));
process.env.DATA_DIR = dir;
const { AppDb } = await import('../src/db/app');
const { get, run } = await import('../src/db/sqlite');

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

describe('backups', () => {
  const app = new AppDb();
  app.db.exec('CREATE TABLE marker (v INTEGER)');
  const marker = () => get<{ v: number }>(app.db, 'SELECT v FROM marker')?.v;

  afterAll(() => {
    app.db.close();
    fs.rmSync(dir, { recursive: true, force: true });
  });

  it('restores the oldest of the kept backups (the safety backup must not prune it first)', async () => {
    run(app.db, 'INSERT INTO marker (v) VALUES (1)');
    const oldest = app.backup('first').file;
    for (let v = 2; v <= 5; v++) {
      await sleep(5);
      run(app.db, 'UPDATE marker SET v = ?', v);
      app.backup(`backup ${v}`);
    }
    expect(AppDb.listBackups()).toHaveLength(5);
    expect(AppDb.listBackups().at(-1)!.file).toBe(oldest);
    run(app.db, 'UPDATE marker SET v = 99');

    await sleep(5);
    app.restore(oldest);
    expect(marker()).toBe(1);
    // the database is usable and the state before the restore was kept as a backup
    run(app.db, 'UPDATE marker SET v = 7');
    expect(marker()).toBe(7);
    expect(AppDb.listBackups()[0].reason).toBe(`Before restoring ${oldest}`);
    expect(fs.existsSync(app.file + '.old')).toBe(false);
    expect(fs.existsSync(app.file + '.restore')).toBe(false);
  });

  it('refuses a broken backup and keeps the live database', () => {
    fs.writeFileSync(path.join(AppDb.backupDir(), 'broken.sqlite'), 'not a database');
    expect(() => app.restore('broken.sqlite')).toThrow(/not a readable database/);
    expect(marker()).toBe(7);
  });
});
