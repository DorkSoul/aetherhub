import crypto from 'node:crypto';
import type { FastifyInstance, FastifyRequest } from 'fastify';
import { z } from 'zod';
import type { CardDef } from '@aetherhub/shared';
import { requireUser } from '../auth';
import { bad, forbidden, notFound, type Ctx } from '../context';
import { all, get, run } from '../db/sqlite';
import { buildDeck, Draft, suggestBasics, type PoolCard, type SetPool } from '../draft';
import { Lobby } from '../games/lobby';
import { rulesEngine } from '../rulesengine';
import { SetCatalog } from '../sets';

/** A draft or sealed event: the pod's draft, owned by a lobby (the event's home). */
interface Session {
  id: string;
  lobbyId: string;
  draft: Draft;
}

const BASICS = ['Plains', 'Island', 'Swamp', 'Mountain', 'Forest'];

export function draftRoutes(r: FastifyInstance, ctx: Ctx) {
  const sessions = new Map<string, Session>();
  const catalog = new SetCatalog(ctx.app, ctx.cards, ctx.log);
  const db = ctx.app.db;
  const hub = ctx.hub;
  const auth = { preHandler: requireUser };

  // ------------------------------------------------------------------ persistence (a restart keeps the event)

  function save(s: Session) {
    const l = hub.lobbies.get(s.lobbyId);
    if (!l) return;
    run(db, 'INSERT INTO draft_events (id, lobby, draft, updated_at) VALUES (?, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET lobby = excluded.lobby, draft = excluded.draft, updated_at = excluded.updated_at', s.id, JSON.stringify(l.toJSON()), JSON.stringify(s.draft.toJSON()), Date.now());
  }
  hub.onEventLobby = (l) => {
    const s = l.draftId ? sessions.get(l.draftId) : undefined;
    if (s) save(s);
  };
  hub.onLobbyClosed = (l) => {
    if (!l.draftId) return;
    sessions.delete(l.draftId);
    run(db, 'DELETE FROM draft_events WHERE id = ?', l.draftId);
  };
  for (const row of all<{ id: string; lobby: string; draft: string }>(db, 'SELECT id, lobby, draft FROM draft_events')) {
    try {
      const l = Lobby.fromJSON(JSON.parse(row.lobby));
      const d = Draft.fromJSON(JSON.parse(row.draft));
      d.playable = playable;
      sessions.set(row.id, { id: row.id, lobbyId: l.id, draft: d });
      hub.lobbies.set(l.id, l);
    } catch (e) {
      ctx.log(`Draft event ${row.id} could not be restored: ${(e as Error).message}`, { cat: 'game', level: 'error' });
    }
  }

  /** Something changed in a draft: save it, move the lobby's phase along, tell the lobby's members. */
  function changed(s: Session) {
    const l = hub.lobbies.get(s.lobbyId);
    if (l) {
      if (s.draft.phase === 'building' && l.phase === 'drafting') {
        l.phase = 'building';
        l.system('The draft is over: build your decks (40 cards or more)');
      }
      syncEventDecks(s, l);
      l.draftRev++;
      hub.lobbyOp(l.id, () => {});
    }
    save(s);
  }

  /** The event lobby's list of decks; once every human drafter has a deck, the games can start. */
  function syncEventDecks(s: Session, l: Lobby) {
    if (s.draft.phase !== 'building') return;
    l.eventDecks = s.draft.seats.map((x, i) => ({
      seat: i,
      name: x.name,
      userId: x.userId,
      deckId: x.deckId,
      deckName: x.userId === null ? (x.built ? `${x.name}’s deck (${x.built.colors.join('')})` : null) : x.deckId != null ? (get<{ name: string }>(db, 'SELECT name FROM decks WHERE id = ?', x.deckId)?.name ?? 'Deck') : null,
    }));
    if (l.phase === 'building' && s.draft.seats.every((x) => x.userId === null || x.deckId != null)) {
      l.phase = 'event';
      l.system('Every deck is built: start games from the lobby (pick the players and sandbox or rules enforced)');
    }
  }

  // the pick timer
  const timer = setInterval(() => {
    for (const s of sessions.values()) if (s.draft.tick()) changed(s);
  }, 1000);
  timer.unref();

  /** Cards the rules engine can play (AI drafters build their decks from these, as their games are rules enforced). */
  function playable(c: PoolCard) {
    const e = rulesEngine();
    return !e || e.hasCard(c.name);
  }

  // ------------------------------------------------------------------ the cards of a set

  function poolOf(code: string): SetPool {
    if (!ctx.cards.db) bad('Card data has not been imported yet');
    const printed = catalog.printedSize(code);
    const rows = all<{ id: string; name: string; rarity: string; cmc: number; colors: string | null; color_identity: string | null; type_line: string; oracle_text: string | null; mana_cost: string | null; power: string | null; toughness: string | null; set_name: string; released_at: string; collector_number: string }>(
      ctx.cards.db!,
      `SELECT id, name, rarity, cmc, colors, color_identity, type_line, oracle_text, mana_cost, power, toughness, set_name, released_at, collector_number FROM printings
       WHERE set_code = ? AND paper = 1 AND is_token = 0 AND promo = 0 AND nonstd = 0 AND (lang = 'en' OR lang IS NULL)
       ORDER BY CAST(collector_number AS INTEGER), collector_number`,
      code,
    );
    const seen = new Set<string>();
    const cards: PoolCard[] = [];
    const basics: PoolCard[] = [];
    for (const x of rows) {
      // collector numbers past the numbered set are bonus sheets and variants, not booster cards
      const n = parseInt(x.collector_number, 10);
      if (printed && !Number.isNaN(n) && n > printed && !/Basic Land/.test(x.type_line)) continue;
      if (seen.has(x.name)) continue;
      seen.add(x.name);
      const c: PoolCard = { key: 'p:' + x.id, name: x.name, rarity: x.rarity, cmc: x.cmc ?? 0, colors: x.colors ?? '', identity: x.color_identity ?? '', typeLine: x.type_line, text: x.oracle_text ?? '', manaCost: x.mana_cost ?? '', power: x.power, toughness: x.toughness };
      (/^Basic Land/.test(x.type_line) ? basics : cards).push(c);
    }
    if (!cards.length) bad(`No cards found for set ${code}`);
    return { code, name: rows[0]?.set_name ?? code, released: rows[0]?.released_at ?? '', cards, basics };
  }

  const defsOf = (cards: { key: string }[]): Record<string, CardDef> => ctx.cards.defsByPrinting([...new Set(cards.map((c) => c.key.replace(/^p:/, '')))]);

  // ------------------------------------------------------------------ starting an event from a lobby

  hub.starters.draft = ({ lobby }) => {
    const type = lobby.type === 'sealed' ? 'sealed' : 'draft';
    const codes = (lobby.settings.sets ?? []).map((c) => c.toLowerCase());
    if (!codes.length) bad('Choose the set (or sets) first');
    const sets = codes.map(poolOf);
    const draft = new Draft(sets, type, crypto.randomInt(2 ** 31 - 1), lobby.slots.filter((x) => x.kind !== 'open').length, lobby.settings.pickTimer === true);
    draft.playable = playable;
    // the pod sits in the lobby's seat order
    for (const slot of lobby.slots) {
      if (slot.kind === 'human') draft.seats.push({ userId: slot.userId, name: slot.name, pool: [], pack: [], picked: false, deckId: null, built: null });
      else if (slot.kind === 'ai') draft.addAi(slot.name);
    }
    draft.start(false);
    const s: Session = { id: crypto.randomUUID(), lobbyId: lobby.id, draft };
    sessions.set(s.id, s);
    lobby.draftId = s.id;
    if (draft.phase === 'building') {
      lobby.phase = 'building';
      syncEventDecks(s, lobby);
    }
    save(s);
    return s.id;
  };

  /** The cards an AI drafter plays (for a rules enforced game started from the event). */
  hub.eventDeckCards = (draftId, seat) => {
    const s = sessions.get(draftId);
    const x = s?.draft.seats[seat];
    if (!x?.built) return null;
    const keys = new Map<string, number>();
    const byKey = new Map(x.pool.map((c) => [c.key, c]));
    for (const k of x.built.main) keys.set(k, (keys.get(k) ?? 0) + 1);
    const main: [string, number][] = [...keys].map(([k, n]) => [byKey.get(k)!.name, n]);
    for (const [name, n] of Object.entries(x.built.basics)) main.push([name, n]);
    return main;
  };

  // ------------------------------------------------------------------ the player's view

  const own = (req: FastifyRequest): { s: Session; seat: number } => {
    const id = (req.params as { id: string }).id;
    const s = sessions.get(id);
    if (!s) notFound('Draft not found');
    const uid = req.user!.id;
    const mine = s!.draft.seats.flatMap((x, i) => (x.userId === uid ? [i] : []));
    if (!mine.length) forbidden();
    const asked = Number((req.query as { seat?: string } | undefined)?.seat ?? (req.body as { seat?: number } | undefined)?.seat);
    const seat = mine.includes(asked) ? asked : (mine.find((i) => !s!.draft.seats[i].picked && s!.draft.phase === 'drafting') ?? mine.find((i) => s!.draft.seats[i].deckId == null) ?? mine[0]);
    return { s: s!, seat };
  };

  const view = (c: PoolCard) => ({ key: c.key, name: c.name, rarity: c.rarity, cmc: c.cmc, colors: c.colors, typeLine: c.typeLine, manaCost: c.manaCost });

  const state = (s: Session, seat: number, uid: number) => {
    const d = s.draft;
    const me = d.seats[seat];
    const visible = [...me.pack, ...me.pool];
    return {
      id: s.id,
      lobbyId: s.lobbyId,
      mode: d.mode,
      phase: d.phase,
      sets: d.sets.map((x) => ({ code: x.code, name: x.name })),
      round: d.round + 1,
      pickNo: d.pickNo + 1,
      timed: d.timed,
      /** milliseconds left for the current pick */
      timeLeft: d.deadline != null ? Math.max(0, d.deadline - Date.now()) : null,
      seats: d.seats.map((x) => ({ name: x.name, human: x.userId !== null, picked: x.picked, pool: x.pool.length, built: x.userId === null ? !!x.built : x.deckId != null })),
      seat,
      mySeats: d.seats.flatMap((x, i) => (x.userId === uid ? [i] : [])),
      /** passing direction this pack: left (packs 1 and 3) or right (pack 2) */
      passing: d.round % 2 === 0 ? 'left' : 'right',
      pack: me.picked ? [] : me.pack.map(view),
      picked: me.picked,
      waitingOn: d.phase === 'drafting' ? d.seats.filter((x) => x.userId !== null && !x.picked).map((x) => x.name) : [],
      pool: me.pool.map(view),
      deckId: me.deckId,
      defs: defsOf(visible),
    };
  };

  r.get('/api/draft/sets', auth, async () => catalog.list());

  r.get('/api/draft/sets/:code/icon.svg', async (req, reply) => {
    const svg = await catalog.icon((req.params as { code: string }).code.toLowerCase());
    if (!svg) return reply.code(404).send();
    return reply.header('Content-Type', 'image/svg+xml').header('Cache-Control', 'public, max-age=604800').send(svg);
  });

  r.get('/api/draft/games/:id', auth, async (req) => {
    const { s, seat } = own(req);
    return state(s, seat, req.user!.id);
  });

  r.post('/api/draft/games/:id/pick', auth, async (req) => {
    const { s, seat } = own(req);
    const b = z.object({ key: z.string(), seat: z.number().int().optional() }).parse(req.body);
    try {
      s.draft.pick(req.user!.id, b.key, seat);
    } catch (e) {
      bad((e as Error).message);
    }
    changed(s);
    return state(s, seat, req.user!.id);
  });

  /** A suggested 40-card deck from this player's pool (the "Build for me" button). */
  r.get('/api/draft/games/:id/suggest', auth, async (req) => {
    const { s, seat } = own(req);
    const b = buildDeck(s.draft.seats[seat].pool);
    return b;
  });

  /** Basic lands for the chosen cards (17 lands with the nonbasics). */
  r.post('/api/draft/games/:id/lands', auth, async (req) => {
    const { s, seat } = own(req);
    const b = z.object({ main: z.array(z.string()).max(120) }).parse(req.body);
    const by = new Map(s.draft.seats[seat].pool.map((c) => [c.key, c]));
    return suggestBasics(b.main.map((k) => by.get(k)).filter((c): c is PoolCard => !!c));
  });

  /**
   * Save the deck: the chosen pool cards (one key per copy) and basic lands in the main deck, the rest of the pool as
   * the sideboard, format Limited. Saving again (rebuilding between games) updates the same deck.
   */
  r.post('/api/draft/games/:id/deck', auth, async (req) => {
    const { s, seat } = own(req);
    if (s.draft.phase !== 'building') bad('Finish drafting first');
    const b = z
      .object({
        seat: z.number().int().optional(),
        name: z.string().trim().min(1).max(80),
        main: z.array(z.string()).max(120),
        basics: z.record(z.string(), z.number().int().min(0).max(60)).default({}),
      })
      .parse(req.body);
    const x = s.draft.seats[seat];
    const left = new Map<string, number>();
    for (const c of x.pool) left.set(c.key, (left.get(c.key) ?? 0) + 1);
    for (const k of b.main) {
      const n = left.get(k) ?? 0;
      if (n <= 0) bad('A chosen card is not in your pool (or you chose more copies than you have)');
      left.set(k, n - 1);
    }
    const count = (keys: string[]) => {
      const m = new Map<string, number>();
      for (const k of keys) m.set(k, (m.get(k) ?? 0) + 1);
      return [...m];
    };
    const cards: { key: string; section: 'main' | 'side'; qty: number }[] = [...count(b.main).map(([key, qty]) => ({ key, section: 'main' as const, qty })), ...[...left].filter(([, n]) => n > 0).map(([key, qty]) => ({ key, section: 'side' as const, qty }))];
    for (const [name, qty] of Object.entries(b.basics)) {
      if (!qty) continue;
      if (!BASICS.includes(name)) bad(`Unknown basic land ${name}`);
      // a basic printed in one of the event's sets when there is one
      const id = s.draft.sets.map((st) => get<{ id: string }>(ctx.cards.db!, "SELECT id FROM printings WHERE set_code = ? AND name = ? AND nonstd = 0 LIMIT 1", st.code, name)?.id).find(Boolean) ?? ctx.cards.matchName(name);
      if (!id) bad(`No printing of ${name} in the card data`);
      cards.push({ key: 'p:' + id, section: 'main', qty });
    }
    if (!b.main.length && !Object.values(b.basics).some(Boolean)) bad('Pick some cards for the deck');
    let deckId = x.deckId;
    if (deckId != null && ctx.decks.getDeck(deckId, req.user!.id)) ctx.decks.update(deckId, { name: b.name, format: 'limited' });
    else deckId = ctx.decks.create(req.user!.id, b.name, 'limited');
    ctx.decks.replaceCards(deckId, cards);
    ctx.images.warm(Object.values(ctx.decks.resolveDefs(cards.map((c) => c.key))).flatMap((d) => d.img));
    x.deckId = deckId;
    changed(s);
    return { deckId };
  });
}
