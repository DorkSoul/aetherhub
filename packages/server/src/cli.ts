// Admin CLI: `npm run reset-password -- <username>` (or `docker exec aetherhub npm run reset-password -- <username>`).
import fs from 'node:fs';
import { hashPassword, tempPassword } from './auth';
import { cardsDbPath, importBulk } from './db/cards';
import { config } from './config';
import { AppDb } from './db/app';
import { get, run } from './db/sqlite';

const [cmd, ...args] = process.argv.slice(2);

async function main() {
  if (cmd === 'reset-password') {
    const name = args[0];
    if (!name) throw new Error('Usage: reset-password <username>');
    const app = new AppDb();
    const u = get<{ id: number; username: string }>(app.db, 'SELECT id, username FROM users WHERE username = ?', name);
    if (!u) throw new Error(`No user named "${name}" in ${config.dataDir}`);
    const pw = tempPassword();
    run(app.db, 'UPDATE users SET pw_hash = ? WHERE id = ?', await hashPassword(pw), u.id);
    run(app.db, 'DELETE FROM sessions WHERE user_id = ?', u.id);
    console.log(`Temporary password for ${u.username}: ${pw}`);
    console.log('Sign in with it, then change it under Settings → Profile & table.');
    app.db.close();
    return;
  }
  if (cmd === 'import-cards') {
    const file = args[0];
    if (!file || !fs.existsSync(file)) throw new Error('Usage: import-cards <default-cards.json|jsonl>');
    const out = cardsDbPath(config.dataDir);
    const next = out.replace(/\.sqlite$/, '.next.sqlite');
    const t = Date.now();
    const rows = await importBulk(fs.createReadStream(file), next);
    for (const s2 of ['', '-wal', '-shm']) fs.rmSync(out + s2, { force: true });
    fs.renameSync(next, out);
    console.log(`Imported ${rows} printings into ${out} in ${((Date.now() - t) / 1000).toFixed(1)} s. Restart the server to use them.`);
    return;
  }
  console.log('Commands:\n  reset-password <username>   print a temporary password for a user\n  import-cards <file>         import a Scryfall Default Cards bulk file (restart the server afterwards)');
}

main().catch((e) => {
  console.error(e.message);
  process.exit(1);
});
