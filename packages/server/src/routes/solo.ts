import type { FastifyInstance, FastifyRequest } from 'fastify';
import { z } from 'zod';
import { requireUser } from '../auth';
import { bad, notFound, type Ctx } from '../context';
import { all, get, run } from '../db/sqlite';

/** Solo games run in the browser; these routes only store and return snapshots of them, per user. */
const MAX_BYTES = 8 * 1024 * 1024;
const MAX_SAVES = 50;

const SaveZ = z.object({
  name: z.string().trim().min(1).max(80),
  // The full game state plus the log and view options; the shared engine validates it again on load.
  state: z
    .object({
      v: z.literal(1),
      seats: z.array(z.object({ name: z.string(), color: z.string(), life: z.number(), deckName: z.string().optional() }).passthrough()).min(1).max(8),
      turn: z.object({ n: z.number(), active: z.number() }).passthrough(),
      settings: z.object({ format: z.string() }).passthrough(),
    })
    .passthrough(),
  log: z.array(z.unknown()).max(1500).default([]),
  solo: z.unknown().optional(),
});
type SaveBody = z.infer<typeof SaveZ>;

interface Row {
  id: number;
  name: string;
  summary: string;
  data: string;
  created_at: number;
  updated_at: number;
}

function summaryOf(b: SaveBody) {
  return {
    format: b.state.settings.format,
    turn: b.state.turn.n,
    active: b.state.turn.active,
    seats: b.state.seats.map((s) => ({ name: s.name, deckName: s.deckName ?? '', color: s.color, life: s.life })),
  };
}

export function soloRoutes(r: FastifyInstance, ctx: Ctx) {
  const auth = { preHandler: requireUser, bodyLimit: MAX_BYTES };
  const db = ctx.app.db;
  const own = (req: FastifyRequest) => {
    const id = Number((req.params as { id: string }).id);
    const row = get<Row>(db, 'SELECT * FROM solo_saves WHERE id = ? AND user_id = ?', id, req.user!.id);
    if (!row) notFound();
    return row!;
  };

  r.get('/api/solo/saves', auth, async (req) =>
    all<Row>(db, 'SELECT id, name, summary, created_at, updated_at FROM solo_saves WHERE user_id = ? ORDER BY updated_at DESC', req.user!.id).map((x) => ({
      id: x.id,
      name: x.name,
      ...JSON.parse(x.summary),
      createdAt: x.created_at,
      updatedAt: x.updated_at,
    })),
  );

  r.get('/api/solo/saves/:id', auth, async (req) => {
    const row = own(req);
    return { id: row.id, name: row.name, updatedAt: row.updated_at, ...JSON.parse(row.data) };
  });

  r.post('/api/solo/saves', auth, async (req) => {
    const b = SaveZ.parse(req.body);
    const n = get<{ n: number }>(db, 'SELECT COUNT(*) AS n FROM solo_saves WHERE user_id = ?', req.user!.id)!.n;
    if (n >= MAX_SAVES) bad(`You can keep up to ${MAX_SAVES} saved solo games — delete an old one first`);
    const now = Date.now();
    const res = run(db, 'INSERT INTO solo_saves (user_id, name, summary, data, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)', req.user!.id, b.name, JSON.stringify(summaryOf(b)), JSON.stringify({ state: b.state, log: b.log, solo: b.solo }), now, now);
    return { id: Number(res.lastInsertRowid), updatedAt: now };
  });

  r.put('/api/solo/saves/:id', auth, async (req) => {
    const row = own(req);
    const b = SaveZ.parse(req.body);
    const now = Date.now();
    run(db, 'UPDATE solo_saves SET name = ?, summary = ?, data = ?, updated_at = ? WHERE id = ?', b.name, JSON.stringify(summaryOf(b)), JSON.stringify({ state: b.state, log: b.log, solo: b.solo }), now, row.id);
    return { id: row.id, updatedAt: now };
  });

  r.patch('/api/solo/saves/:id', auth, async (req) => {
    const row = own(req);
    const b = z.object({ name: z.string().trim().min(1).max(80) }).parse(req.body);
    run(db, 'UPDATE solo_saves SET name = ? WHERE id = ?', b.name, row.id);
    return { ok: true };
  });

  r.delete('/api/solo/saves/:id', auth, async (req) => {
    const row = own(req);
    run(db, 'DELETE FROM solo_saves WHERE id = ?', row.id);
    return { ok: true };
  });
}
