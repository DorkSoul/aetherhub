import type { FastifyInstance, FastifyReply, FastifyRequest } from 'fastify';
import { z } from 'zod';
import { FORMAT_BY_ID, MULLIGAN_RULES, rulesMode, type MulliganRule } from '@aetherhub/shared';
import { requireUser } from '../auth';
import { notFound, type Ctx } from '../context';
import { all, get } from '../db/sqlite';
import { LobbyError } from '../games/lobby';

const SettingsZ = z
  .object({
    name: z.string().trim().min(1).max(60),
    type: z.enum(['sandbox', 'rules', 'draft', 'sealed']),
    format: z.string(),
    life: z.number().int().min(1).max(999),
    mulligan: z.enum(MULLIGAN_RULES.map((m) => m.id) as [MulliganRule, ...MulliganRule[]]),
    bo: z.union([z.literal(1), z.literal(3)]),
    maxPlayers: z.number().int().min(2).max(8),
    spectators: z.boolean(),
    specChatShared: z.boolean(),
    ownersOnly: z.boolean(),
    planechase: z.boolean(),
    planarPool: z.enum(['host', 'all']),
    archenemy: z.boolean(),
    sets: z.array(z.string().min(1).max(12)).max(6),
    pickTimer: z.boolean(),
    enforced: z.boolean(),
  })
  .partial();

const Level = z.enum(['Easy', 'Normal', 'Hard']);

export function playRoutes(r: FastifyInstance, ctx: Ctx) {
  const auth = { preHandler: requireUser };
  const hub = ctx.hub;
  const lid = (req: FastifyRequest) => (req.params as { id: string }).id;
  const seatNo = (req: FastifyRequest) => Number((req.params as { seat: string }).seat);
  /** Runs `fn`; answers with the lobby (or what `fn` returns) and turns lobby errors into 409s. */
  const wrap = (fn: (req: FastifyRequest) => unknown) => async (req: FastifyRequest, reply: FastifyReply) => {
    try {
      const out = await fn(req);
      if (out !== undefined) return out;
      const id = (req.params as { id?: string }).id;
      return { lobby: (id && hub.lobbies.get(id)?.view()) || null };
    } catch (e) {
      if (e instanceof LobbyError) return reply.code(409).send({ error: e.message });
      throw e;
    }
  };

  r.get('/api/lobbies', auth, async (req) => [...hub.lobbies.values()].map((l) => l.summary(req.user!.id)));
  r.get('/api/lobbies/mine', auth, async (req) => hub.lobbyOf(req.user!.id)?.view() ?? null);
  r.get(
    '/api/lobbies/:id',
    auth,
    wrap((req) => {
      const l = hub.requireLobby(lid(req));
      return l.view();
    }),
  );

  r.post(
    '/api/lobbies',
    auth,
    wrap((req) => {
      const b = z.object({ settings: SettingsZ.default({}), password: z.string().max(60).nullable().optional() }).parse(req.body ?? {});
      if (b.settings.format && !FORMAT_BY_ID[b.settings.format]) throw new LobbyError('Unknown format');
      return { lobby: hub.createLobby(req.user!, b.settings, b.password ?? null).view() };
    }),
  );

  r.post(
    '/api/lobbies/:id/join',
    auth,
    wrap((req) => {
      const b = z.object({ password: z.string().max(60).nullable().optional() }).parse(req.body ?? {});
      hub.joinLobby(req.user!, lid(req), b.password ?? null);
    }),
  );

  r.post(
    '/api/lobbies/:id/leave',
    auth,
    wrap((req) => {
      hub.leaveLobby(req.user!, lid(req));
      return { lobby: null };
    }),
  );

  r.put(
    '/api/lobbies/:id/settings',
    auth,
    wrap((req) => {
      const b = SettingsZ.extend({ password: z.string().max(60).nullable().optional(), removeAi: z.boolean().optional() }).parse(req.body);
      if (b.format && !FORMAT_BY_ID[b.format]) throw new LobbyError('Unknown format');
      hub.lobbyOp(lid(req), (l) => l.updateSettings(req.user!, b));
    }),
  );

  /** Sit in a seat: yourself, or (host) another member or an AI. */
  r.post(
    '/api/lobbies/:id/seats/:seat',
    auth,
    wrap((req) => {
      const b = z.union([z.object({ userId: z.number().int() }), z.object({ ai: Level })]).parse(req.body ?? { userId: req.user!.id });
      hub.lobbyOp(lid(req), (l) => l.sit(req.user!, seatNo(req), b));
    }),
  );

  r.delete(
    '/api/lobbies/:id/seats/:seat',
    auth,
    wrap((req) => {
      hub.lobbyOp(lid(req), (l) => l.stand(req.user!, seatNo(req)));
    }),
  );

  r.put(
    '/api/lobbies/:id/seats/:seat/deck',
    auth,
    wrap((req) => {
      const b = z.object({ deckId: z.number().int().nullable() }).parse(req.body);
      const deck = hub.deckRef(b.deckId);
      hub.lobbyOp(lid(req), (l) => l.setDeck(req.user!, seatNo(req), deck));
    }),
  );

  r.put(
    '/api/lobbies/:id/seats/:seat/difficulty',
    auth,
    wrap((req) => {
      const b = z.object({ difficulty: Level }).parse(req.body);
      hub.lobbyOp(lid(req), (l) => l.setDifficulty(req.user!, seatNo(req), b.difficulty));
    }),
  );

  r.put(
    '/api/lobbies/:id/ready',
    auth,
    wrap((req) => {
      const b = z.object({ ready: z.boolean() }).parse(req.body);
      hub.lobbyOp(lid(req), (l) => l.setReady(req.user!.id, b.ready));
    }),
  );

  r.post(
    '/api/lobbies/:id/move',
    auth,
    wrap((req) => {
      const b = z.object({ seat: z.number().int().min(0).max(7), d: z.number().int().min(-7).max(7) }).parse(req.body);
      hub.lobbyOp(lid(req), (l) => l.move(req.user!, b.seat, b.d));
    }),
  );

  r.post(
    '/api/lobbies/:id/reroll',
    auth,
    wrap((req) => {
      hub.lobbyOp(lid(req), (l) => l.reroll(req.user!));
    }),
  );

  r.post(
    '/api/lobbies/:id/kick',
    auth,
    wrap((req) => {
      const b = z.object({ userId: z.number().int() }).parse(req.body);
      hub.kick(req.user!, lid(req), b.userId);
    }),
  );

  r.post(
    '/api/lobbies/:id/chat',
    auth,
    wrap((req) => {
      const b = z.object({ text: z.string().trim().min(1).max(500) }).parse(req.body);
      hub.lobbyChat(req.user!, lid(req), b.text);
    }),
  );

  /** A game from a draft / sealed event: the chosen drafters (pod seats), sandbox or rules enforced. */
  r.post(
    '/api/lobbies/:id/event-game',
    auth,
    wrap((req) => {
      const b = z.object({ seats: z.array(z.number().int().min(0).max(7)).min(2).max(8), type: z.enum(['sandbox', 'rules']).optional() }).parse(req.body);
      const l = hub.requireLobby(lid(req));
      return hub.startEventGame(req.user!, lid(req), b.seats, b.type ?? rulesMode(l.settings));
    }),
  );

  r.post(
    '/api/lobbies/:id/start',
    auth,
    wrap((req) => hub.startGame(req.user!, lid(req))),
  );

  r.get('/api/games/suspended', auth, async () => hub.suspendedGames());

  /** After a game (sandbox or rules enforced): back to its lobby (re-opened if it had closed), or leave it for the menu. */
  r.post(
    '/api/games/:id/back-to-lobby',
    auth,
    wrap((req) => ({ lobbyId: hub.backToLobby(req.user!, (req.params as { id: string }).id) })),
  );
  r.post(
    '/api/games/:id/leave-lobby',
    auth,
    wrap((req) => {
      hub.leaveAfterGame(req.user!, (req.params as { id: string }).id);
    }),
  );

  r.post(
    '/api/games/:id/load',
    auth,
    wrap((req) => {
      const g = hub.loadSuspended(req.user!, (req.params as { id: string }).id);
      return { gameId: g.id };
    }),
  );

  r.delete(
    '/api/games/:id',
    auth,
    wrap((req) => {
      hub.deleteSuspended(req.user!, (req.params as { id: string }).id);
      return { ok: true };
    }),
  );

  // ---------------------------------------------------------------- history
  r.get('/api/matches', auth, async (req) => {
    const limit = Math.min(200, Number((req.query as { limit?: string }).limit) || 100);
    const rows = all<{ id: number; name: string; format: string; bo: number; started_at: number; ended_at: number; turns: number; meta: string; winner_name: string | null }>(
      ctx.app.db,
      'SELECT * FROM matches ORDER BY ended_at DESC LIMIT ?',
      limit,
    );
    return rows.map((m) => ({
      id: m.id,
      name: m.name,
      format: m.format,
      bo: m.bo,
      startedAt: m.started_at,
      endedAt: m.ended_at,
      turns: m.turns,
      meta: JSON.parse(m.meta),
      winner: m.winner_name,
      players: all<{ seat: number; user_id: number | null; name: string; color: string; deck_name: string; result: string }>(ctx.app.db, 'SELECT * FROM match_players WHERE match_id = ? ORDER BY seat', m.id).map((p) => ({
        seat: p.seat,
        userId: p.user_id,
        name: p.user_id == null ? `(deleted) ${p.name}` : p.name,
        color: p.color,
        deck: p.deck_name,
        result: p.result,
      })),
    }));
  });

  r.get('/api/matches/:id/log', auth, async (req) => {
    const id = Number((req.params as { id: string }).id);
    if (!get(ctx.app.db, 'SELECT 1 FROM matches WHERE id = ?', id)) notFound();
    return all(ctx.app.db, 'SELECT t_offset AS t, who, color, kind, text FROM match_log WHERE match_id = ? ORDER BY idx', id);
  });

  r.get('/api/matches/:id/log.txt', auth, async (req, reply) => {
    const id = Number((req.params as { id: string }).id);
    const m = get<{ name: string; ended_at: number }>(ctx.app.db, 'SELECT name, ended_at FROM matches WHERE id = ?', id);
    if (!m) notFound();
    const rows = all<{ t: number; who: string; kind: string; text: string }>(ctx.app.db, 'SELECT t_offset AS t, who, kind, text FROM match_log WHERE match_id = ? ORDER BY idx', id);
    const fmt = (t: number) => {
      const s = Math.floor(t / 1000);
      return `${Math.floor(s / 3600)}:${String(Math.floor((s % 3600) / 60)).padStart(2, '0')}:${String(s % 60).padStart(2, '0')}`;
    };
    const text = `${m.name} — ${new Date(m.ended_at).toISOString().slice(0, 10)}\n\n` + rows.map((r2) => `${fmt(r2.t)}  ${r2.who}${r2.kind === 'chat' ? ': ' : ' '}${r2.text}`).join('\n') + '\n';
    return reply
      .header('Content-Type', 'text/plain; charset=utf-8')
      .header('Content-Disposition', `attachment; filename="match-${id}.txt"`)
      .send(text);
  });
}
