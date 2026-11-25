import crypto from 'node:crypto';
import type { FastifyInstance, FastifyRequest } from 'fastify';
import { z } from 'zod';
import type { rules as R } from '@aetherhub/shared';
import type { RulesGame } from '@aetherhub/rules';
import { requireUser } from '../auth';
import { bad, forbidden, HttpError, notFound, type Ctx } from '../context';
import { all, get, run } from '../db/sqlite';
import { rulesEngine, rulesEngineError } from '../rulesengine';
import { formatInfo, normName } from '@aetherhub/shared';

/**
 * Rules-enforced games on the native Rust engine. The engine owns the game state; AI seats play inside it on
 * native threads. A game is its setup plus the ordered decisions, which gives undo and save/resume for free.
 */
interface SeatInfo {
  userId: number | null; // null = AI, or an open seat while the game waits for players
  open: boolean;
}
interface StoredFrame {
  seq: number;
  events: R.Event[];
  view: R.View;
  /** the seat whose answer produced this frame (the first frame after a human answer); undefined = the AI / the game itself */
  by?: number;
}
interface Session {
  id: string;
  setup: R.GameSetup;
  seats: SeatInfo[];
  game: RulesGame | null;
  status: 'waiting' | 'running';
  frames: Map<number, StoredFrame[]>;
  seq: number;
  epoch: number;
  waiting: R.Decision | null;
  /** increases with every new question, so a client can tell a fresh decision from one it already answered */
  decisionId: number;
  over: R.GameResult | null;
  /** every human seat has left the game; the AIs play on to be watched */
  out: boolean;
  /** the AI-only game paused because nobody is watching */
  watchRest?: boolean;
  error: string | null;
  warnings: Map<number, string[]>;
  touched: number;
  deckName: string;
  chain: Promise<void>;
  waiters: (() => void)[];
  /** a saved human-vs-human game waits for the other player to rejoin */
  resume: { record: R.SessionRecord; waitingFor: number } | null;
  undoRequest: { seat: number; steps: number } | null;
  /** Just undone: the question the game is back at is asked, not answered by a remembered reaction (that would redo
   *  what was undone straight away). */
  noAutoOnce?: boolean;
  /** seats whose player ended the game for themselves (conceded and left); the others play on */
  left: Set<number>;
  /** seat → where its automatic passing of priority ends (kept here: asking the engine would wait for a running AI turn) */
  passing: Map<number, R.PassUntil>;
  /** seat → answers the player asked to give automatically for the rest of this game */
  reactions: Map<number, Reaction[]>;
  reactionSeq: number;
  /** "break the rules": the game is paused while this seat puts something right by hand; nobody else can act */
  paused: { seat: number } | null;
  /** the turn number as of the last frame (kept here: asking the engine would wait for a running AI turn) */
  turn: number;
  /** a game restored after a restart: its record (JSON), replayed into the engine the first time someone opens it */
  pending: string | null;
  /** a save to the database is scheduled */
  saveTimer: NodeJS.Timeout | null;
  savedAt: number;
}

/** What `rules_games.state` holds for a game in progress (the engine's record is stored next to it). */
interface StoredSession {
  v: 1;
  setup: R.GameSetup & { _users?: (number | null)[] };
  seats: SeatInfo[];
  status: 'waiting' | 'running';
  deckName: string;
  epoch: number;
  decisionId: number;
  turn: number;
  waiting: R.Decision | null;
  out: boolean;
  warnings: [number, string[]][];
  left: number[];
  reactions: [number, Reaction[]][];
  reactionSeq: number;
  paused: { seat: number } | null;
  resume: { record: R.SessionRecord; waitingFor: number } | null;
}

function makeSession(id: string, setup: R.GameSetup, seats: SeatInfo[], deckName: string, warnings = new Map<number, string[]>()): Session {
  return {
    id,
    setup,
    seats,
    game: null,
    status: 'waiting',
    frames: new Map(),
    seq: 0,
    epoch: 0,
    waiting: null,
    decisionId: 0,
    over: null,
    out: false,
    error: null,
    warnings,
    touched: Date.now(),
    deckName,
    chain: Promise.resolve(),
    waiters: [],
    resume: null,
    undoRequest: null,
    left: new Set(),
    passing: new Map(),
    reactions: new Map(),
    reactionSeq: 0,
    paused: null,
    turn: 0,
    pending: null,
    saveTimer: null,
    savedAt: 0,
  };
}

/**
 * Run `fn` once no engine call is running or queued for this game. `fn` starts in the same tick as that check, so its
 * synchronous engine calls never wait on the lock an `advance()` holds (which would block the whole event loop).
 */
async function locked<T>(s: Session, fn: () => T | Promise<T>): Promise<T> {
  for (;;) {
    const c = s.chain;
    await c.catch(() => undefined);
    if (c === s.chain) return fn();
  }
}

/** The turn shown by a batch of frames (views carry it; a turn's first event does too), or `turn` when they show none. */
function turnOf(frames: R.Frame[], turn: number): number {
  for (const f of frames) {
    for (const v of Object.values(f.views)) if (v) turn = v.turn;
    for (const e of f.events) if (e.t === 'TurnBegin') turn = e.turn;
  }
  return turn;
}

/** "Remembered reaction" (this game only): `ignore` = pass priority whenever this is on top of the stack; `yes` / `no` =
 *  answer this card's may-question; `pick` = choose this option of this card's choice. Matched by card name and prompt. */
interface Reaction {
  id: number;
  /** ignore: pass when this is on the stack · mute: do not stop only because this ability could be activated */
  kind: 'ignore' | 'yes' | 'no' | 'pick' | 'mute';
  /** the card the stack item or question comes from */
  source: string;
  /** the stack item's text or '*' for any (ignore), `ability:<index>` (mute), or the question's prompt */
  prompt: string;
  /** pick: the option's label */
  option?: string;
}

/** The label of a pick option as the player sees it (only text and colour options can be remembered). */
function optLabel(o: R.Opt): string | null {
  return o.t === 'Text' ? o.text : o.t === 'Color' ? o.c : null;
}
const MAX_AUTO_ANSWERS = 200;

const MAX_GAMES_PER_USER = 3;
/** A game nobody has looked at for this long is autosaved (to every human's Saved games) and removed from memory. */
const IDLE_MS = 30 * 60 * 1000;
/** Games in progress are written to the database at most this often between human actions (each human action saves). */
const SAVE_MS = 2000;
/** On shutdown, how long to wait for a running AI turn before keeping the game's last save instead. */
const CLOSE_WAIT_MS = 5000;

/** Opponent decks for "no deck chosen": playable with the starter cards, whatever else is supported. */
const PRESETS: { name: string; deck: [string, number][] }[] = [
  { name: 'Green Stompy', deck: [['Forest', 16], ['Llanowar Elves', 4], ['Grizzly Bears', 4], ['Giant Spider', 4], ['Giant Growth', 4], ['Rancor', 3], ['Charging Badger', 4], ['Rampant Growth', 1]] },
  { name: 'Red Burn', deck: [['Mountain', 16], ['Lightning Bolt', 4], ['Shock', 4], ['Raging Goblin', 4], ['Hill Giant', 4], ['Flametongue Kavu', 3], ['Goblin King', 2], ['Prodigal Pyromancer', 3]] },
  { name: 'White Weenie', deck: [['Plains', 16], ['Elite Vanguard', 4], ['Youthful Knight', 4], ['Serra Angel', 3], ['Swords to Plowshares', 4], ['Glorious Anthem', 3], ['Raise the Alarm', 4], ['Isamaru, Hound of Konda', 2]] },
];

interface DeckInfo {
  name: string;
  main: [string, number][];
  commanders: string[];
  sideboard: [string, number][];
  /** The deck's companion (section 'companion'): starts outside the game and can be put into hand for {3}. */
  companion: string | null;
  unsupported: string[];
  /** Planechase: the deck's Planar section (cards the engine plays) */
  planar: [string, number][];
  /** Archenemy: the deck's Schemes section (cards the engine plays) */
  schemes: [string, number][];
}

export function rulesRoutes(r: FastifyInstance, ctx: Ctx) {
  const sessions = new Map<string, Session>();
  /** the players' names for the log and the admin page, an AI marked as one */
  const playersOf = (s: Session) => s.setup.seats.map((x) => (x.human ? x.name : `${x.name} (AI)`));
  const WATCH_IDLE_MS = 2 * 60_000;
  const db = ctx.app.db;
  /** the app is shutting down: no more watch-mode steps or scheduled saves */
  let closing = false;
  /** the shutdown save is done (the database closes next): nothing more is written */
  let stopped = false;
  const sweep = setInterval(() => {
    for (const [id, s] of sessions) {
      if (Date.now() - s.touched <= IDLE_MS) continue;
      sessions.delete(id);
      // nobody came back: keep the game in Saved games instead of losing it
      if ((s.game || s.pending) && s.status === 'running' && !s.over && !s.error)
        void locked(s, () => saveSession(s, `Autosave – ${new Date().toLocaleString('en-GB', { dateStyle: 'medium', timeStyle: 'short' })}`))
          .catch((e) => ctx.log('rules autosave failed: ' + String((e as Error).message ?? e)))
          .finally(() => unpersist(id));
      else unpersist(id);
    }
  }, 60_000);
  sweep.unref();

  // ---- keeping games in progress across restarts (table rules_games) ----

  const stateOf = (s: Session): StoredSession => ({
    v: 1,
    setup: s.setup,
    seats: s.seats,
    status: s.status,
    deckName: s.deckName,
    epoch: s.epoch,
    decisionId: s.decisionId,
    turn: s.turn,
    waiting: s.waiting,
    out: s.out,
    warnings: [...s.warnings.entries()],
    left: [...s.left],
    reactions: [...s.reactions.entries()],
    reactionSeq: s.reactionSeq,
    paused: s.paused,
    resume: s.resume,
  });

  /** Write the game to the database now (a finished or failed game is removed instead). Only call it inside `locked`
   *  or the game's chain: the engine's record is read synchronously. */
  function persist(s: Session) {
    if (stopped || sessions.get(s.id) !== s) return;
    try {
      if (s.over || s.error) return unpersist(s.id);
      const record = s.game ? s.game.recordJson() : s.pending;
      if (s.status === 'running' && !record) return;
      run(db, 'INSERT INTO rules_games (id, state, record, updated_at) VALUES (?, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET state = excluded.state, record = excluded.record, updated_at = excluded.updated_at', s.id, JSON.stringify(stateOf(s)), record, Date.now());
      s.savedAt = Date.now();
    } catch (e) {
      ctx.log(`rules game ${s.id}: saving failed: ${String((e as Error).message ?? e)}`);
    }
  }

  /** Save the game soon (several changes in a row make one write). */
  function persistSoon(s: Session) {
    if (s.saveTimer || closing) return;
    s.saveTimer = setTimeout(() => {
      void locked(s, () => {
        s.saveTimer = null;
        persist(s);
      });
    }, SAVE_MS);
    s.saveTimer.unref();
  }

  function unpersist(id: string) {
    if (stopped) return;
    try {
      run(db, 'DELETE FROM rules_games WHERE id = ?', id);
    } catch (e) {
      ctx.log(`rules game ${id}: removing its save failed: ${String((e as Error).message ?? e)}`);
    }
  }

  /** The games that were in progress when the server last stopped. Each one is listed at once; its engine state is
   *  replayed the first time someone opens it (`wake`). A record that can't be read is logged and skipped. */
  for (const row of all<{ id: string; state: string; record: string | null }>(db, 'SELECT id, state, record FROM rules_games')) {
    try {
      const st = JSON.parse(row.state) as StoredSession;
      if (st.v !== 1 || !Array.isArray(st.setup?.seats) || !Array.isArray(st.seats) || st.seats.length !== st.setup.seats.length) throw new Error('unexpected format');
      if (st.status === 'running' && !row.record) throw new Error('no engine record');
      if (row.record) JSON.parse(row.record);
      const s = makeSession(row.id, st.setup, st.seats, st.deckName ?? '', new Map(st.warnings ?? []));
      s.status = st.status;
      s.epoch = st.epoch ?? 0;
      s.decisionId = st.decisionId ?? 0;
      s.turn = st.turn ?? 0;
      s.waiting = st.waiting ?? null;
      s.out = !!st.out;
      s.left = new Set(st.left ?? []);
      s.reactions = new Map(st.reactions ?? []);
      s.reactionSeq = st.reactionSeq ?? 0;
      s.paused = st.paused ?? null;
      s.resume = st.resume ?? null;
      s.pending = st.status === 'running' ? row.record : null;
      sessions.set(s.id, s);
    } catch (e) {
      ctx.log(`rules game ${row.id} could not be restored, skipped: ${String((e as Error).message ?? e)}`);
    }
  }

  /** Shutdown: let each game finish the engine call it is in (up to CLOSE_WAIT_MS) and save it. A game whose AI is still
   *  thinking keeps its last save (taken at the last human action), so only that AI's current turn is played again.
   *  `preClose`, not `onClose`: the app's onClose closes the database, and onClose hooks run newest first. */
  r.addHook('preClose', async () => {
    closing = true;
    clearInterval(sweep);
    await Promise.all(
      [...sessions.values()].map(async (s) => {
        if (s.saveTimer) clearTimeout(s.saveTimer);
        s.saveTimer = null;
        let timer: NodeJS.Timeout | undefined;
        const saved = await Promise.race([
          locked(s, () => {
            persist(s);
            return true;
          }),
          new Promise<false>((res) => (timer = setTimeout(() => res(false), CLOSE_WAIT_MS))),
        ]);
        clearTimeout(timer);
        if (!saved) ctx.log(`rules game ${s.id}: an AI was still thinking at shutdown; keeping its last save`);
      }),
    );
    stopped = true;
    // answer the long polls now instead of holding the shutdown until they time out
    for (const s of sessions.values()) notify(s);
  });

  /** A game restored after a restart is replayed into the engine when it is first opened, and its AIs play on. */
  function wake(s: Session) {
    if (!s.pending || s.game) return;
    const json = s.pending;
    try {
      begin(s, json);
    } catch (e) {
      s.error = 'This game could not be restored: ' + String((e as Error).message ?? e).split('\n')[0];
      ctx.log(`rules game ${s.id}: ${s.error}`);
      // keep what there is in the players' Saved games, and drop it from the games in progress
      try {
        saveSession(s, `Could not be restored – ${new Date().toLocaleString('en-GB', { dateStyle: 'medium', timeStyle: 'short' })}`);
      } catch {
        /* nothing more to keep */
      }
      s.pending = null;
      unpersist(s.id);
    }
  }

  const engine = () => {
    const e = rulesEngine();
    if (!e) bad('The rules engine is not available on this server' + (rulesEngineError() ? ` (${rulesEngineError()})` : ''));
    return e!;
  };

  function deckList(deckId: number, userId: number): DeckInfo {
    const e = engine();
    const d = ctx.decks.getDeck(deckId, userId);
    if (!d) notFound('Deck not found');
    const counts = new Map<string, number>();
    const commanders: string[] = [];
    const side = new Map<string, number>();
    let companion: string | null = null;
    const unsupported = new Set<string>();
    const planar = new Map<string, number>();
    const schemes = new Map<string, number>();
    for (const c of d!.cards) {
      if (c.section === 'planar' || c.section === 'schemes') {
        const def = d!.defs[c.def];
        if (def && e.hasCard(def.name)) (c.section === 'planar' ? planar : schemes).set(def.name, ((c.section === 'planar' ? planar : schemes).get(def.name) ?? 0) + c.qty);
        continue;
      }
      if (c.section !== 'main' && c.section !== 'commander' && c.section !== 'side' && c.section !== 'companion') continue;
      const def = d!.defs[c.def];
      if (!def || def.isToken) continue;
      if (c.section === 'companion') {
        // the engine puts the companion into the sideboard on its own; deck-construction restrictions are not checked
        if (!companion && e.hasCard(def.name)) companion = def.name;
        continue;
      }
      if (c.section === 'side') {
        // sideboard cards the engine can't play are just left out; they are not in the game anyway
        if (e.hasCard(def.name)) side.set(def.name, (side.get(def.name) ?? 0) + c.qty);
        continue;
      }
      if (!e.hasCard(def.name)) {
        unsupported.add(def.name);
        continue;
      }
      if (c.section === 'commander') {
        if (!commanders.includes(def.name)) commanders.push(def.name);
        continue;
      }
      counts.set(def.name, (counts.get(def.name) ?? 0) + c.qty);
    }
    return { name: d!.name, main: [...counts.entries()], commanders, sideboard: [...side.entries()], companion, unsupported: [...unsupported], planar: [...planar.entries()], schemes: [...schemes.entries()] };
  }

  const warn = (who: string, d: DeckInfo): string | null =>
    d.unsupported.length ? `Left out of ${who} deck "${d.name}" because the rules engine doesn't support these cards yet: ${d.unsupported.join(', ')}` : null;

  function seatDeck(d: DeckInfo, commander: boolean, who: string): { deck: [string, number][]; commander: string[]; sideboard: [string, number][]; companion: string | null } {
    const total = d.main.reduce((n, [, q]) => n + q, 0) + d.commanders.length;
    if (!total) bad(`${who} deck "${d.name}" has no cards the rules engine supports yet`);
    if (commander) {
      if (!d.commanders.length) bad(`${who} deck "${d.name}" has no supported commander${d.unsupported.length ? ` (not supported yet: ${d.unsupported.join(', ')})` : ''}. Pick another deck or play a Standard game.`);
      return { deck: d.main, commander: d.commanders, sideboard: d.sideboard, companion: d.companion };
    }
    return { deck: [...d.main, ...d.commanders.map((c) => [c, 1] as [string, number])], commander: [], sideboard: d.sideboard, companion: d.companion };
  }

  const seatOf = (s: Session, userId: number) => s.seats.findIndex((x) => x.userId === userId);
  /** every seat this user holds (several in a solo game) */
  const seatsOf = (s: Session, userId: number) => s.seats.flatMap((x, i) => (x.userId === userId ? [i] : []));
  /** distinct human players (one person holding several seats counts once) */
  const humans = (s: Session) => new Set(s.seats.flatMap((x) => (x.userId !== null ? [x.userId] : []))).size;
  const isAi = (s: Session) => humans(s) === 1;
  /** this user's live games (not over, not left) */
  const mine = (uid: number) => [...sessions.values()].filter((s) => seatsOf(s, uid).some((i) => !s.left.has(i)) && !s.over && !s.error);
  const running = (uid: number) => mine(uid).length;
  const notify = (s: Session) => {
    const w = s.waiters.splice(0);
    for (const f of w) f();
    persistSoon(s);
  };

  /** Run the engine until a human must answer; store the playback frames for each human seat. */
  function pump(s: Session, by?: number): Promise<void> {
    s.chain = s.chain
      .then(async () => {
        if (!s.game) return;
        // what the humans did so far is saved before the AIs think (watching AIs alone: every few seconds)
        if (!s.out || Date.now() - s.savedAt >= SAVE_MS) persist(s);
        // a question the player told us how to answer ("remembered reactions") is answered here and the game runs on
        for (let n = 0; n <= MAX_AUTO_ANSWERS; n++) {
          const adv = await s.game.advance();
          storeFrames(s, adv, n === 0 ? by : undefined);
          if (adv.over && !s.over) {
            const names = playersOf(s);
            ctx.log(`Rules game over: ${names.join(', ')} – ${adv.over.t === 'Winner' ? `${names[adv.over.player] ?? 'a player'} won` : 'a draw'} on turn ${s.turn}`);
          }
          s.over = adv.over;
          s.out = !!adv.humans_out && !adv.over;
          // Every human has left and this step showed nothing: the engine is waiting on a seat nobody answers for.
          // Stop with an error instead of asking again for ever.
          if (s.out && !s.paused && !adv.frames.some((f) => f.events.length > 0)) throw new Error('the game is waiting for a player who has left it');
          const auto = adv.waiting && !adv.over && !s.paused && n < MAX_AUTO_ANSWERS && !s.noAutoOnce ? autoAnswer(s, adv.waiting) : null;
          s.noAutoOnce = false;
          // a question answered here is never shown as pending (nobody can answer it a second time)
          s.waiting = auto ? null : adv.waiting;
          if (!auto || !adv.waiting) break;
          s.game.answer(adv.waiting.player, auto);
        }
        if (s.waiting) s.decisionId++;
        syncPassing(s);
      })
      .catch((e) => {
        s.error = String((e as Error).message ?? e).split('\n')[0];
        ctx.log('rules game error: ' + s.error);
      })
      .finally(() => {
        notify(s);
        // watching the AIs after every human has left: keep going, one AI decision per pump, while the session lives
        // (it rests when nobody has looked for two minutes, and picks up again at the next poll)
        if (s.out && !s.over && !s.error && !s.paused && sessions.get(s.id) === s && !closing) {
          if (Date.now() - s.touched < WATCH_IDLE_MS) setTimeout(() => void pump(s), 0);
          else s.watchRest = true;
        }
      });
    return s.chain;
  }

  function storeFrames(s: Session, adv: R.Advance, by?: number) {
    s.turn = turnOf(adv.frames, s.turn);
    for (const [k, f] of adv.frames.entries()) {
      s.seq = Math.max(s.seq, f.seq);
      for (const [seatStr, view] of Object.entries(f.views)) {
        const seat = Number(seatStr);
        const list = s.frames.get(seat) ?? [];
        list.push({ seq: f.seq, events: f.events, view: view as R.View, by: k === 0 ? by : undefined });
        if (list.length > 600) list.splice(0, list.length - 600);
        s.frames.set(seat, list);
      }
    }
  }

  /** The answer a remembered reaction gives to this question, if one matches. */
  function autoAnswer(s: Session, d: R.Decision): R.Choice | null {
    const rules = s.reactions.get(d.player);
    if (!rules?.length) return null;
    const view = s.frames.get(d.player)?.at(-1)?.view;
    if (!view) return null;
    const nameOf = (id: number | null) => (id === null ? null : ([...view.cards, ...view.decision_cards].find((c) => c.id === id)?.name ?? null));
    const k = d.kind;
    if (k.t === 'Priority') {
      const top = view.stack.at(-1);
      if (top && rules.some((r) => r.kind === 'ignore' && r.source === top.name && (r.prompt === top.text || r.prompt === '*'))) return { t: 'Priority', action: null };
      // nothing to do but abilities the player said not to stop for: pass, as if they could not be used at all
      const real = k.actions.filter((a) => a.t !== 'ActivateMana');
      if (real.length && real.every((a) => a.t === 'Activate' && rules.some((r) => r.kind === 'mute' && r.source === nameOf(a.card) && r.prompt === `ability:${a.ability}`))) return { t: 'Priority', action: null };
      return null;
    }
    if (k.t === 'YesNo') {
      const src = nameOf(k.source);
      const r = rules.find((x) => (x.kind === 'yes' || x.kind === 'no') && x.source === src && x.prompt === k.prompt);
      return r ? { t: 'YesNo', yes: r.kind === 'yes' } : null;
    }
    if (k.t === 'Pick' && k.min <= 1 && k.max >= 1 && ['Mode', 'ModeRepeat', 'Generic', 'Cost'].includes(k.purpose.t)) {
      const src = nameOf(k.source);
      const r = rules.find((x) => x.kind === 'pick' && x.source === src && x.prompt === k.prompt);
      const i = r ? k.options.findIndex((o) => optLabel(o) === r.option) : -1;
      return i >= 0 ? { t: 'Pick', indices: [i] } : null;
    }
    return null;
  }

  /** A player who leaves while breaking the rules ends the pause. */
  function endPauseOf(s: Session, seat: number) {
    if (s.paused?.seat !== seat || !s.game) return;
    s.game.endBreak(seat);
    s.paused = null;
  }

  /** Refresh the cached "passing until" of every human seat (only while no engine call is running). */
  function syncPassing(s: Session) {
    if (!s.game) return;
    s.passing.clear();
    s.seats.forEach((x, i) => {
      if (x.userId === null) return;
      const p = s.game!.passing(i);
      if (p) s.passing.set(i, p);
    });
  }

  /** Start the game in the engine: new from its setup, or from a record (object or JSON) replayed. */
  function begin(s: Session, record?: R.SessionRecord | string) {
    const e = engine();
    s.game = record === undefined ? e.newGame(s.setup) : typeof record === 'string' ? e.restoreGameJson(record) : e.restoreGame(record);
    s.pending = null;
    // no engine call can be running on a game just made
    s.turn = s.game.turn;
    s.status = 'running';
    s.epoch++;
    s.frames.clear();
    s.waiting = null;
    s.over = null;
    s.out = false;
    s.error = null;
    s.seq = 0;
    void pump(s);
  }

  /** Card name → Scryfall image reference (cached). */
  const imgCache = new Map<string, string | null>();
  /** Card name → its Scryfall layout when that changes how it is drawn (split cards and Rooms are pictured sideways). */
  const layoutCache = new Map<string, string>();
  /** Card name (either face) → the picture of its other face, for double-faced cards. */
  const backCache = new Map<string, string>();
  /** Card name (either face) → its other face's name, cost, type line, text and P/T (JSON), for double-faced cards. */
  const backFaceCache = new Map<string, string>();
  /** Names of a flip card's bottom half (Rune-Tail's Essence): a permanent with this name is shown turned 180°. */
  const flippedNames = new Set<string>();
  const imageFor = (name: string): string | null => {
    if (imgCache.has(name)) return imgCache.get(name)!;
    let ref: string | null = null;
    try {
      const id = ctx.cards.matchName(name) ?? ctx.cards.matchName(name, undefined, undefined, true) ?? ctx.cards.matchBackFace(name);
      const def = id ? ctx.cards.defsByPrinting([id])['p:' + id] : undefined;
      // the face this name is: a back-face name ("Insectile Aberration") pictures the back
      const n = normName(name);
      const face = Math.max(0, def?.faces?.findIndex((f) => normName(f.name) === n) ?? 0);
      ref = def?.img?.[face] ?? def?.img?.[0] ?? null;
      if (def && def.img.length > 1) {
        backCache.set(name, def.img[face === 0 ? 1 : 0]);
        const f = def.faces?.[face === 0 ? 1 : 0];
        if (f) backFaceCache.set(name, JSON.stringify({ name: f.name, manaCost: f.manaCost, typeLine: f.typeLine, oracleText: f.oracleText, power: f.power, toughness: f.toughness, loyalty: f.loyalty, defense: f.defense }));
      }
      if (def && (def.layout === 'split' || def.layout === 'planar' || def.layout === 'flip')) layoutCache.set(name, def.layout);
      if (def?.layout === 'flip' && face === 1) flippedNames.add(name);
    } catch {
      ref = null;
    }
    imgCache.set(name, ref);
    return ref;
  };
  const tokenCache = new Map<string, string | null>();
  // pictures go by `art` (a copy looks like what it copies; a name change keeps the picture), falling back to the name
  const artOf = (c: R.CardView) => c.art || c.name;
  const tokenKey = (c: R.CardView) => `${artOf(c)}|${c.colors}`;
  const tokenImage = (c: R.CardView): string | null => {
    const key = tokenKey(c) + '|' + c.types;
    if (tokenCache.has(key)) return tokenCache.get(key)!;
    let ref: string | null = null;
    try {
      const id = ctx.cards.matchToken(artOf(c), c.base_power ?? c.power ?? undefined, c.base_toughness ?? c.toughness ?? undefined, c.colors, c.types, c.text ?? '');
      if (id) ref = ctx.cards.defsByPrinting([id])['p:' + id]?.img?.[0] ?? null;
    } catch {
      ref = null;
    }
    tokenCache.set(key, ref);
    return ref;
  };
  const imagesOf = (views: R.View[]): Record<string, string> => {
    const out: Record<string, string> = {};
    for (const v of views)
      for (const c of [...v.cards, ...v.decision_cards]) {
        if (c.face_down && c.name === 'Face-down creature') continue;
        if (c.art_token ?? c.token) {
          const key = tokenKey(c);
          if (!out[key]) {
            const ref = tokenImage(c);
            if (ref) out[key] = ref;
          }
          continue;
        }
        const name = artOf(c);
        if (out[name]) continue;
        const ref = imageFor(name);
        if (ref) out[name] = ref;
        // the layout rides along in the same map under its own key (see LAYOUT_KEY in the client's rulesView)
        const layout = layoutCache.get(name);
        if (layout) out['@layout:' + name] = layout;
        // so is the other face's picture (see BACK_KEY)
        const back = backCache.get(name);
        if (back) out['@back:' + name] = back;
        const backFace = backFaceCache.get(name);
        if (backFace) out['@backface:' + name] = backFace;
        if (flippedNames.has(name)) out['@flipped:' + name] = '1';
      }
    return out;
  };

  const stateFor = (s: Session, seat: number, since?: number) => {
    const all = s.frames.get(seat) ?? [];
    const frames = since === undefined ? all.slice(-1) : all.filter((f) => f.seq > since);
    const last = all.at(-1)?.view ?? null;
    return {
      id: s.id,
      names: s.setup.seats.map((x) => x.name),
      seat,
      /** every seat this player holds: a solo player follows the one whose decision is pending */
      mySeats: s.seats.flatMap((x, i) => (x.userId !== null && x.userId === s.seats[seat]?.userId ? [i] : [])),
      status: s.status,
      versus: isAi(s) ? 'ai' : 'pvp',
      frames,
      seq: s.seq,
      epoch: s.epoch,
      view: last,
      waiting: s.waiting && s.waiting.player === seat ? s.waiting : null,
      waitingId: s.decisionId,
      waitingFor: s.waiting ? s.waiting.player : null,
      over: s.over,
      out: s.out,
      error: s.error,
      images: imagesOf(frames.length ? frames.map((f) => f.view) : last ? [last] : []),
      notices: s.warnings.get(seat) ?? [],
      commander: s.setup.commander,
      undoRequest: s.undoRequest ? { steps: s.undoRequest.steps, mine: s.undoRequest.seat === seat } : null,
      passing: s.passing.get(seat) ?? null,
      reactions: s.reactions.get(seat) ?? [],
      paused: s.paused ? { seat: s.paused.seat, name: s.setup.seats[s.paused.seat]?.name ?? 'A player', mine: s.paused.seat === seat } : null,
    };
  };

  /**
   * The game and the seat this request acts for. A player holding several seats (a solo game) names one with
   * `?seat=`; without it, the seat whose decision is pending, else their first seat still in the game.
   */
  const own = (req: FastifyRequest): { s: Session; seat: number } => {
    const id = (req.params as { id: string }).id;
    const s = sessions.get(id);
    if (!s) notFound('Game not found');
    const seats = seatsOf(s!, req.user!.id);
    if (!seats.length) forbidden();
    // `seat` picks a seat outright; `as` is the seat the client shows, kept unless another of theirs must act
    const q = (req.query as { seat?: string; as?: string } | undefined) ?? {};
    const asked = Number(q.seat);
    const shown = Number(q.as);
    wake(s!);
    const seat = seats.includes(asked) ? asked : s!.waiting && seats.includes(s!.waiting.player) ? s!.waiting.player : seats.includes(shown) ? shown : (seats.find((i) => !s!.left.has(i)) ?? seats[0]);
    s!.touched = Date.now();
    return { s: s!, seat };
  };
  const auth = { preHandler: requireUser };

  /** Every token the engine can make (for creating one by hand while breaking the rules). */
  let tokenList: string[] | null = null;
  r.get('/api/rules/tokens', auth, async () => (tokenList ??= engine().tokenNames()));

  r.get('/api/rules/status', auth, async () => {
    const e = rulesEngine();
    return { available: !!e, cards: e?.cardCount ?? 0, error: e ? null : rulesEngineError() };
  });

  /** Names of every card the engine can play (the deck builder / lobby mark unsupported cards with this). */
  r.get('/api/rules/cards', auth, async () => ({ names: rulesEngine()?.cardNames() ?? [] }));

  /** Which cards of the given decks the engine can't play yet, and their commanders. */
  r.post('/api/rules/check', auth, async (req) => {
    const body = z.object({ deckIds: z.array(z.number().int()).max(8) }).parse(req.body);
    const out: Record<number, { name: string; unsupported: string[]; commanders: string[]; playable: number; total: number }> = {};
    for (const id of body.deckIds) {
      try {
        const d = deckList(id, req.user!.id);
        const playable = d.main.reduce((n, [, q]) => n + q, 0);
        out[id] = { name: d.name, unsupported: d.unsupported, commanders: d.commanders, playable, total: playable + d.unsupported.length };
      } catch {
        /* unknown deck */
      }
    }
    return out;
  });

  r.post('/api/rules/games', auth, async (req) => {
    const body = z
      .object({
        deckId: z.number().int(),
        gameType: z.enum(['constructed', 'commander']).default('constructed'),
        ais: z.array(z.object({ deckId: z.number().int().nullable().optional(), difficulty: z.enum(['Easy', 'Normal', 'Hard']).optional() })).max(7).default([]),
        open: z.number().int().min(0).max(7).default(0),
        seed: z.number().int().optional(),
        mulligan: z.enum(['London', 'LondonFreeFirst', 'Free', 'FreeScry3', 'Vancouver', 'Paris', 'None']).optional(),
      })
      .parse(req.body);
    const uid = req.user!.id;
    const commander = body.gameType === 'commander';
    if (running(uid) >= MAX_GAMES_PER_USER) bad('Finish or leave your running games first');
    const ais = body.ais.length || body.open ? body.ais : [{ deckId: null }];
    if (1 + ais.length + body.open < 2) bad('A game needs at least two players');
    if (1 + ais.length + body.open > 8) bad('A game has at most 8 players');
    const mineInfo = deckList(body.deckId, uid);
    const mine = seatDeck(mineInfo, commander, 'Your');
    const warnings: string[] = [];
    const w0 = warn('your', mineInfo);
    if (w0) warnings.push(w0);
    const seats: R.SeatSetup[] = [{ name: req.user!.username, human: true, deck: mine.deck, commander: mine.commander, sideboard: mine.sideboard, companion: mine.companion, difficulty: 'Normal', schemes: [] }];
    const info: SeatInfo[] = [{ userId: uid, open: false }];
    ais.forEach((a, i) => {
      const who = ais.length > 1 ? `AI ${i + 1}` : 'AI';
      let sd: { deck: [string, number][]; commander: string[]; sideboard: [string, number][]; companion: string | null };
      if (!a.deckId) {
        if (commander) bad('Choose a deck for each AI opponent in a Commander game');
        const e = engine();
        const preset = PRESETS.filter((p) => p.deck.every(([n]) => e.hasCard(n)))[i % PRESETS.length] ?? PRESETS[0];
        sd = { deck: preset.deck, commander: [], sideboard: [], companion: null };
      } else {
        const di = deckList(a.deckId, uid);
        const w = warn(`${who}'s`, di);
        if (w) warnings.push(w);
        sd = seatDeck(di, commander, `${who}'s`);
      }
      seats.push({ name: who, human: false, deck: sd.deck, commander: sd.commander, sideboard: sd.sideboard, companion: sd.companion, difficulty: a.difficulty ?? 'Normal', schemes: [] });
      info.push({ userId: null, open: false });
    });
    for (let i = 0; i < body.open; i++) {
      seats.push({ name: '(open seat)', human: true, deck: [], commander: [], sideboard: [], companion: null, difficulty: 'Normal', schemes: [] });
      info.push({ userId: null, open: true });
    }
    const s = makeSession(
      crypto.randomUUID(),
      { seed: body.seed ?? crypto.randomInt(2 ** 31 - 1), commander, mulligan: body.mulligan ?? (commander ? 'LondonFreeFirst' : 'London'), seats, planar: [] },
      info,
      mineInfo.name,
      new Map(warnings.length ? [[0, warnings]] : []),
    );
    sessions.set(s.id, s);
    if (!body.open) begin(s);
    else persistSoon(s);
    return { id: s.id };
  });

  /** The lobby's mulligan rules by their engine names. */
  const ENGINE_MULLIGAN: Record<string, R.MulliganRule> = { none: 'None', london: 'London', 'london-free': 'LondonFreeFirst', free: 'Free', 'free-scry3': 'FreeScry3', vancouver: 'Vancouver', paris: 'Paris' };

  /**
   * A rules enforced game from a lobby: humans (one user may hold several seats) and AIs, each with the deck chosen
   * in the lobby. Commander rules when the lobby's format has a command zone.
   */
  ctx.hub.starters.rules = ({ settings, seats, by }) => {
    const commander = formatInfo(settings.format).commandZone;
    for (const uid of new Set(seats.flatMap((x) => (x.userId !== null ? [x.userId] : [])))) if (running(uid) >= MAX_GAMES_PER_USER) bad('A player has too many running games: finish or leave one first');
    const setupSeats: R.SeatSetup[] = [];
    const info: SeatInfo[] = [];
    const warnings = new Map<number, string[]>();
    const note = (seat: number, w: string | null) => w && warnings.set(seat, [...(warnings.get(seat) ?? []), w]);
    const e = engine();
    let presetNo = 0;
    /** Planechase: the planar deck (the host's, or everyone's pooled, one of each card); Archenemy: each seat's schemes */
    const planarFrom: [string, number][][] = [];
    const schemesOf: [string, number][][] = [];
    seats.forEach((x, i) => {
      let sd: { deck: [string, number][]; commander: string[]; sideboard: [string, number][]; companion: string | null };
      if (x.cards) {
        // an AI drafter's deck (card names and counts); cards the engine can't play are left out
        const main = x.cards.filter(([n]) => e.hasCard(n));
        const left = x.cards.filter(([n]) => !e.hasCard(n)).map(([n]) => n);
        if (left.length) seats.forEach((y, j) => y.userId === by.id && note(j, `Left out of ${x.name}'s deck because the rules engine doesn't support these cards yet: ${left.join(', ')}`));
        sd = { deck: main, commander: [], sideboard: [], companion: null };
      } else if (x.deckId == null) {
        if (commander || x.userId !== null) bad(`Choose a deck for ${x.name}`);
        const preset = PRESETS.filter((p) => p.deck.every(([n]) => e.hasCard(n)))[presetNo++ % PRESETS.length] ?? PRESETS[0];
        sd = { deck: preset.deck, commander: [], sideboard: [], companion: null };
      } else {
        const di = deckList(x.deckId, x.userId ?? by.id);
        if (settings.planechase && (settings.planarPool !== 'host' || x.userId === by.id)) planarFrom.push(di.planar);
        schemesOf[i] = settings.archenemy ? di.schemes : [];
        const w = warn(`${x.name}'s`, di);
        // an AI's left-out cards are told to whoever started the game
        if (x.userId !== null) note(i, w);
        else seats.forEach((y, j) => y.userId === by.id && note(j, w));
        sd = seatDeck(di, commander, `${x.name}'s`);
      }
      setupSeats.push({ name: x.name, human: x.userId !== null, deck: sd.deck, commander: sd.commander, sideboard: sd.sideboard, companion: sd.companion, difficulty: x.difficulty, schemes: schemesOf[i] ?? [] });
      info.push({ userId: x.userId, open: false });
    });
    const planar = new Map<string, number>();
    for (const list of planarFrom) for (const [n] of list) planar.set(n, 1);
    const s = makeSession(
      crypto.randomUUID(),
      { seed: crypto.randomInt(2 ** 31 - 1), commander, mulligan: ENGINE_MULLIGAN[settings.mulligan] ?? (commander ? 'LondonFreeFirst' : 'London'), seats: setupSeats, planar: [...planar.entries()] },
      info,
      settings.name,
      warnings,
    );
    sessions.set(s.id, s);
    begin(s);
    ctx.log(`Rules game started: ${playersOf(s).join(', ')}`);
    return s.id;
  };

  ctx.hub.rulesAlive = (id) => {
    const s = sessions.get(id);
    return !!s && !s.over && !s.error;
  };

  ctx.hub.rulesGames = () =>
    [...sessions.values()].map((s) => ({
      id: s.id,
      kind: 'rules' as const,
      name: s.deckName,
      players: playersOf(s),
      turn: s.turn,
      lastActive: s.touched,
      state: s.over || s.error ? ('over' as const) : s.status === 'waiting' ? ('waiting' as const) : !s.game && s.pending ? ('sleeping' as const) : ('running' as const),
    }));

  /** The player's running games, so a closed tab can find its way back. */
  r.get('/api/rules/mine', auth, async (req) => {
    const uid = req.user!.id;
    return mine(uid).map((s) => {
      const seats = seatsOf(s, uid);
      return {
        id: s.id,
        opponents: s.setup.seats.filter((_, i) => !seats.includes(i)).map((x) => x.name),
        turn: s.turn,
        yourMove: !!s.waiting && seats.includes(s.waiting.player),
        versus: isAi(s) ? 'ai' : 'pvp',
        status: s.status,
        over: !!s.over,
      };
    });
  });

  /** Every rules enforced game still in progress on the server (for the Active lobbies list); `mine` = the player has a seat. */
  r.get('/api/rules/active', auth, async (req) => {
    const uid = req.user!.id;
    return [...sessions.values()]
      .filter((s) => !s.over && !s.error && !s.resume)
      .map((s) => ({
        id: s.id,
        names: s.setup.seats.map((x) => x.name),
        ais: s.setup.seats.filter((x) => !x.human).length,
        turn: s.turn,
        status: s.status,
        mine: seatsOf(s, uid).some((i) => !s.left.has(i)),
      }));
  });

  r.get('/api/rules/open', auth, async (req) =>
    [...sessions.values()]
      .filter((s) => s.status === 'waiting' && seatOf(s, req.user!.id) < 0 && (s.resume ? s.resume.waitingFor === req.user!.id : s.seats.some((x) => x.open)))
      .map((s) => ({ id: s.id, host: s.setup.seats[0].name, deck: s.deckName, resume: !!s.resume, seats: s.seats.length, free: s.seats.filter((x) => x.open).length, commander: s.setup.commander })),
  );

  r.post('/api/rules/games/:id/join', auth, async (req) => {
    const s = sessions.get((req.params as { id: string }).id);
    if (!s || s.status !== 'waiting') notFound('That game is no longer open');
    const uid = req.user!.id;
    if (s!.resume) {
      if (s!.resume.waitingFor !== uid) forbidden();
      // the saved record's seat for this user becomes theirs again
      s!.seats = s!.resume.record.setup.seats.map((x) => ({ userId: null, open: false })) as SeatInfo[];
      const rec = s!.resume.record;
      const meta = s!.setup as R.GameSetup & { _users?: (number | null)[] };
      const users: (number | null)[] = meta._users ?? [];
      s!.seats = rec.setup.seats.map((_, i) => ({ userId: users[i] ?? null, open: false }));
      s!.resume = null;
      begin(s!, rec);
      return { id: s!.id };
    }
    if (seatOf(s!, uid) >= 0) bad('You are already in this game');
    if (running(uid) >= MAX_GAMES_PER_USER) bad('Finish or leave your running games first');
    const body = z.object({ deckId: z.number().int() }).parse(req.body);
    const idx = s!.seats.findIndex((x) => x.open);
    if (idx < 0) bad('That game is full');
    const di = deckList(body.deckId, uid);
    const d = seatDeck(di, s!.setup.commander, 'Your');
    s!.setup.seats[idx] = { name: req.user!.username, human: true, deck: d.deck, commander: d.commander, sideboard: d.sideboard, companion: d.companion, difficulty: 'Normal', schemes: [] };
    s!.seats[idx] = { userId: uid, open: false };
    const w = warn('your', di);
    if (w) s!.warnings.set(idx, [w]);
    s!.touched = Date.now();
    if (!s!.seats.some((x) => x.open)) begin(s!);
    else persistSoon(s!);
    return { id: s!.id };
  });

  /** `since`: the last frame the client has (same epoch). `wait=1` holds the request until something changes (long poll). */
  r.get('/api/rules/games/:id', auth, async (req) => {
    const { s, seat } = own(req);
    if (s.watchRest) {
      s.watchRest = false;
      void pump(s);
    }
    const q = z.object({ since: z.coerce.number().int().optional(), epoch: z.coerce.number().int().optional(), wait: z.coerce.number().int().optional() }).parse(req.query ?? {});
    const since = q.epoch === s.epoch ? q.since : undefined;
    if (q.wait && since !== undefined && since >= s.seq && !s.over && !s.error) {
      await Promise.race([new Promise<void>((res) => s.waiters.push(res)), new Promise<void>((res) => setTimeout(res, 12_000))]);
    }
    return stateFor(s, seat, since);
  });

  /**
   * Answer the pending question. `decisionId` is the `waitingId` of the state the client answered: an answer to an
   * older question (the game moved on, or the answer was sent twice) is refused with 409. Clients from before it was
   * sent leave it out and are only checked against the pending seat.
   */
  r.post('/api/rules/games/:id/answer', auth, async (req) => {
    const { s } = own(req);
    // manual: tap the mana for this one spell or ability by hand ("Cast, tapping mana by hand")
    const body = z.object({ choice: z.unknown(), manual: z.boolean().optional(), decisionId: z.number().int().optional() }).parse(req.body);
    return locked(s, () => {
      // the seat is picked again now that the engine is idle: in a solo game the pending seat may have changed
      const { seat } = own(req);
      if (s.status !== 'running' || !s.game) bad('The game is not running');
      if (s.paused) bad('The game is paused while the rules are broken; finish that first');
      if (!s.waiting || s.waiting.player !== seat || (body.decisionId !== undefined && body.decisionId !== s.decisionId)) throw new HttpError(409, 'That decision is no longer pending');
      try {
        if (body.manual) s.game.manualOnce(seat);
        s.game.answer(seat, body.choice as R.Choice);
      } catch (e) {
        bad(String((e as Error).message ?? e).replace(/^.*illegal choice:\s*/i, 'Illegal choice: '));
      }
      s.waiting = null;
      void pump(s, seat);
      return { ok: true };
    });
  });

  const doUndo = (s: Session, steps: number) =>
    locked(s, async () => {
      s.undoRequest = null;
      if (!s.game) return;
      s.game.undo(steps);
      s.noAutoOnce = true;
      s.turn = s.game.turn;
      s.epoch++;
      s.frames.clear();
      s.over = null;
      s.out = false;
      s.waiting = null;
      s.seq = 0;
      await pump(s);
    });

  r.post('/api/rules/games/:id/undo', auth, async (req) => {
    const { s, seat } = own(req);
    const steps = z.object({ steps: z.number().int().min(1).max(50).default(1) }).parse(req.body ?? {}).steps;
    if (s.status !== 'running') bad('The game is not running');
    if (s.paused) bad('The game is paused while the rules are broken; finish that first');
    try {
      if (isAi(s)) await doUndo(s, steps);
      else s.undoRequest = { seat, steps };
    } catch (e) {
      bad(String((e as Error).message ?? e));
    }
    notify(s);
    return stateFor(s, seat);
  });

  r.post('/api/rules/games/:id/undo/respond', auth, async (req) => {
    const { s, seat } = own(req);
    const body = z.object({ accept: z.boolean() }).parse(req.body);
    if (!s.undoRequest || s.undoRequest.seat === seat) bad('There is no undo request for you to answer');
    if (body.accept) await doUndo(s, s.undoRequest.steps);
    else s.undoRequest = null;
    notify(s);
    return stateFor(s, seat);
  });

  /** Why a click or drag was refused (a plain-language reason): kind = play | attack | block, card and other are card ids. */
  r.get('/api/rules/games/:id/explain', auth, async (req) => {
    const { s, seat } = own(req);
    const q = z
      .object({ kind: z.enum(['play', 'attack', 'block', 'tap']), card: z.coerce.number().int().min(0).optional(), other: z.coerce.number().int().min(0).optional() })
      .parse(req.query);
    if (!s.game) bad('The game is not running');
    return locked(s, () => ({ text: s.game!.explain(seat, q.kind, q.card ?? null, q.other ?? null) }));
  });

  /** "End turn" / "pass to a phase" / "pass to a stack item": the engine passes every plain priority for you (also with
   *  things on the stack) until the given turn and step (index into the step list) or until the stack item is on top.
   *  Questions other than priority (blockers, optional costs, targets) still stop it. */
  r.post('/api/rules/games/:id/passuntil', auth, async (req) => {
    const { s, seat } = own(req);
    const body = z
      .union([z.object({ turn: z.number().int().min(0), step: z.number().int().min(0).max(12) }), z.object({ stackItem: z.number().int().min(0) }), z.object({ player: z.number().int().min(0).max(7), after: z.number().int().min(0).optional() })])
      .parse(req.body);
    if (s.status !== 'running' || !s.game) bad('The game is not running');
    if (s.paused) bad('The game is paused while the rules are broken; finish that first');
    if ('player' in body && body.player >= s.seats.length) bad('No such player');
    return locked(s, () => {
      const g = s.game!;
      if ('stackItem' in body) g.passUntilStack(seat, body.stackItem);
      else if ('player' in body) g.passUntilPlayer(seat, body.player, body.after);
      else g.passUntil(seat, body.turn, body.step);
      syncPassing(s);
      if (s.waiting && s.waiting.player === seat && s.waiting.kind.t === 'Priority' && !s.paused) {
        g.answer(seat, { t: 'Priority', action: null });
        s.waiting = null;
        void pump(s, seat);
      } else notify(s);
      return { ok: true };
    });
  });

  /** Remember how to react to something for the rest of this game (see `Reaction`). */
  r.post('/api/rules/games/:id/reactions', auth, async (req) => {
    const { s, seat } = own(req);
    const body = z
      .object({ kind: z.enum(['ignore', 'yes', 'no', 'pick', 'mute']), source: z.string().min(1).max(200), prompt: z.string().max(2000), option: z.string().max(500).optional() })
      .parse(req.body);
    if (body.kind === 'pick' && !body.option) bad('Choose the option to remember');
    if (!s.game) bad('The game is not running');
    return locked(s, () => {
      const list = (s.reactions.get(seat) ?? []).filter((x) => !(x.source === body.source && x.prompt === body.prompt));
      list.push({ id: ++s.reactionSeq, ...body });
      s.reactions.set(seat, list);
      // it may already apply to the open question
      const auto = s.waiting && s.waiting.player === seat && !s.paused ? autoAnswer(s, s.waiting) : null;
      if (auto) {
        s.game!.answer(seat, auto);
        s.waiting = null;
        void pump(s, seat);
      } else notify(s);
      return { ok: true };
    });
  });

  r.delete('/api/rules/games/:id/reactions/:rid', auth, async (req) => {
    const p = req.params as { id: string; rid: string };
    const { s, seat } = own(req);
    s.reactions.set(
      seat,
      (s.reactions.get(seat) ?? []).filter((x) => x.id !== Number(p.rid)),
    );
    notify(s);
    return { ok: true };
  });

  /** The "auto-tap lands" setting: off = the player taps lands by hand for every payment. */
  r.post('/api/rules/games/:id/autotap', auth, async (req) => {
    const { s, seat } = own(req);
    const body = z.object({ on: z.boolean() }).parse(req.body);
    if (!s.game) bad('The game is not running');
    return locked(s, () => {
      s.game!.setManualPay(seat, !body.on);
      persistSoon(s);
      return { ok: true };
    });
  });

  /** While declaring blockers: which proposed blocks the rules keep (and why others are dropped) and where damage would go. */
  r.post('/api/rules/games/:id/preview-blocks', auth, async (req) => {
    const { s } = own(req);
    const body = z.object({ blocks: z.array(z.tuple([z.number().int().min(0), z.number().int().min(0)])).max(200) }).parse(req.body);
    if (!s.game) bad('The game is not running');
    return locked(s, () => s.game!.previewBlocks(body.blocks));
  });

  /** "Break the rules": pause the game to put right something the engine got wrong (a card bug), moving cards, tapping,
   *  changing counters and life by hand; nobody (and no AI) acts until it is finished. It can start whenever a player has
   *  priority; finishing asks that priority question again from scratch. The changes are recorded with the game. */
  r.post('/api/rules/games/:id/break', auth, async (req) => {
    const { s, seat } = own(req);
    const body = z.object({ on: z.boolean() }).parse(req.body);
    if (!s.game || s.status !== 'running' || s.over) bad('The game is not running');
    return locked(s, async () => {
      if (body.on) {
        if (s.paused) bad(`${s.setup.seats[s.paused.seat]?.name ?? 'Another player'} is already breaking the rules`);
        if (!s.waiting || s.waiting.kind.t !== 'Priority')
          bad('The rules can be broken while a player has priority (when the game asks for an action, not in the middle of a question). Try again at the next priority.');
        s.paused = { seat };
        notify(s);
        return { ok: true };
      }
      if (!s.paused) return { ok: true };
      if (s.paused.seat !== seat) bad(`Only ${s.setup.seats[s.paused.seat]?.name ?? 'the player who started it'} can finish this`);
      s.game!.endBreak(seat);
      s.paused = null;
      s.waiting = null;
      await pump(s);
      return { ok: true };
    });
  });

  r.post('/api/rules/games/:id/edit', auth, async (req) => {
    const { s, seat } = own(req);
    const body = z.object({ op: z.object({ t: z.string() }).passthrough() }).parse(req.body);
    if (!s.game || s.paused?.seat !== seat) bad('Start "Break the rules" first (◇ Menu)');
    return locked(s, async () => {
      try {
        s.game!.edit(seat, body.op as unknown as R.EditOp);
      } catch (e) {
        bad(String((e as Error).message ?? e));
      }
      // show the change to everyone (the engine only reports the views again; nobody acts while paused)
      await pump(s);
      return { ok: true };
    });
  });

  /** Stop passing priority automatically. */
  r.post('/api/rules/games/:id/passuntil/cancel', auth, async (req) => {
    const { s, seat } = own(req);
    if (!s.game) bad('The game is not running');
    return locked(s, () => {
      s.game!.cancelPass(seat);
      syncPassing(s);
      notify(s);
      return { ok: true };
    });
  });

  r.post('/api/rules/games/:id/concede', auth, async (req) => {
    const { s, seat } = own(req);
    if (!s.game || s.over) bad('The game is not running');
    return locked(s, async () => {
      if (s.over) bad('The game is not running');
      endPauseOf(s, seat);
      s.game!.concede(seat);
      s.waiting = null;
      await pump(s);
      return stateFor(s, seat);
    });
  });

  r.post('/api/rules/games/:id/save', auth, async (req) => {
    const { s } = own(req);
    const body = z.object({ name: z.string().trim().min(1).max(80) }).parse(req.body);
    if (!s.game) bad('The game has not started');
    return locked(s, () => ({ id: saveSession(s, body.name).get(req.user!.id as number) ?? 0 }));
  });

  /** Store the game in every human player's Saved games; returns user id → save id. Only inside `locked`. */
  function saveSession(s: Session, name: string): Map<number, number> {
    const json = s.game ? s.game.recordJson() : s.pending;
    if (!json) return new Map();
    const record = JSON.parse(json) as R.SessionRecord;
    const users = s.seats.map((x) => x.userId);
    const cfg = JSON.stringify({ setup: record.setup, users, commands: record.commands });
    const answers = JSON.stringify(record.log);
    const now = Date.now();
    const summary = JSON.stringify({ opponent: isAi(s) ? s.setup.seats[1].name : s.setup.seats.filter((x) => x.human).map((x) => x.name).join(' vs '), versus: isAi(s) ? 'ai' : 'pvp', answers: record.log.length, turn: s.turn, deck: s.deckName });
    const ids = new Map<number, number>();
    for (const uid of users.filter((x): x is number => x !== null)) {
      const res = run(db, 'INSERT INTO rules_saves (user_id, name, cfg, answers, summary, created_at, updated_at) VALUES (?,?,?,?,?,?,?)', uid, name, cfg, answers, summary, now, now);
      ids.set(uid, Number(res.lastInsertRowid));
    }
    return ids;
  }

  r.get('/api/rules/saves', auth, async (req) =>
    all<{ id: number; name: string; summary: string; updated_at: number }>(db, 'SELECT id, name, summary, updated_at FROM rules_saves WHERE user_id = ? ORDER BY updated_at DESC', req.user!.id).map((x) => ({
      id: x.id,
      name: x.name,
      updatedAt: x.updated_at,
      ...JSON.parse(x.summary),
    })),
  );

  r.post('/api/rules/saves/:id/resume', auth, async (req) => {
    const uid = req.user!.id as number;
    const row = get<{ cfg: string; answers: string; name: string }>(db, 'SELECT cfg, answers, name FROM rules_saves WHERE id = ? AND user_id = ?', Number((req.params as { id: string }).id), uid);
    if (!row) notFound('Save not found');
    if (running(uid) >= MAX_GAMES_PER_USER) bad('Finish or leave your running games first');
    const cfg = JSON.parse(row!.cfg) as { setup?: R.GameSetup; users?: (number | null)[]; commands?: R.LoggedCommand[] };
    if (!cfg.setup || !cfg.users) bad('That save was made by the previous rules engine and can no longer be resumed');
    const record: R.SessionRecord = { setup: cfg.setup!, log: JSON.parse(row!.answers) as R.Choice[], commands: cfg.commands ?? [] };
    const users = cfg.users!;
    const other = users.find((x) => x !== null && x !== uid);
    const s = makeSession(
      crypto.randomUUID(),
      record.setup,
      users.map((u) => ({ userId: u, open: false })),
      row!.name,
    );
    sessions.set(s.id, s);
    if (other !== undefined && other !== null) {
      // human vs human: wait for the other player to rejoin (they see it in the open list)
      (s.setup as R.GameSetup & { _users?: (number | null)[] })._users = users;
      s.resume = { record, waitingFor: other };
      s.seats = users.map((u) => ({ userId: u === uid ? u : null, open: false }));
      persistSoon(s);
      return { id: s.id };
    }
    begin(s, record);
    return { id: s.id };
  });

  r.delete('/api/rules/saves/:id', auth, async (req) => {
    run(db, 'DELETE FROM rules_saves WHERE id = ? AND user_id = ?', Number((req.params as { id: string }).id), req.user!.id);
    return { ok: true };
  });

  /** End the game for this player. Against the AI (or when no other human is left) the game is removed; in a game with other
   *  players still in it, this player concedes and leaves, and the others play on. */
  r.delete('/api/rules/games/:id', auth, async (req) => {
    const id = (req.params as { id: string }).id;
    const { s, seat } = own(req);
    const others = s.seats.some((x, i) => i !== seat && x.userId !== null && !s.left.has(i));
    if (others && s.game && s.status === 'running' && !s.over && !s.error) {
      return locked(s, async () => {
        endPauseOf(s, seat);
        s.left.add(seat);
        s.game!.concede(seat);
        s.waiting = null;
        await pump(s);
        return { ok: true };
      });
    }
    sessions.delete(id);
    unpersist(id);
    // answer the requests still waiting on this game now, instead of leaving them open until they time out
    notify(s);
    return { ok: true };
  });
}
