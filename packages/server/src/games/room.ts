import crypto from 'node:crypto';
import type WebSocket from 'ws';
import type { ArrowMsg, CardDef, CardInst, ClientMsg, GameSettings, GameState, GameSummary, GameView, LogEntry, LogView, Notice, Section, SeatInput, ServerMsg, SideboardState, Viewer } from '@aetherhub/shared';
import { createGame, diff, IntentError, IntentSchema, logFor, PLAYER_COLORS, viewFor } from '@aetherhub/shared';
import type { AppDb } from '../db/app';
import { get, run, tx } from '../db/sqlite';
import type { DeckService } from '../decks';
import type { ImageCache } from '../images';
import { cryptoRng, LiveGame, type GameMeta } from './live';

export interface HubUser {
  id: number;
  username: string;
  isAdmin: boolean;
}

export interface Conn {
  id: number;
  ws: WebSocket;
  user: HubUser;
  alive: boolean;
  /** the sandbox game this connection is watching (null: none) */
  room: GameRoom | null;
  /** False when a newer tab of the same account took the seat. */
  primary: boolean;
  created: number;
  lastView: GameView | null;
  role: 'player' | 'spectator' | 'none';
  /** the seat this connection acts as (a player holding several seats chooses it) */
  seat: number | null;
}

interface SbSeat {
  userId: number;
  deckId: number | null;
  main: Map<string, number>;
  side: Map<string, number>;
  other: { def: string; section: Section; qty: number }[];
  defs: Record<string, CardDef>;
}

interface SbStage {
  seats: SbSeat[];
  ready: boolean[];
  chooser: number | null;
  starter: number | null;
}

interface UndoReq {
  id: string;
  seat: number;
  userId: number;
  toSeq: number;
  label: string;
}

/** What a room needs from the hub that owns it. */
export interface RoomHost {
  conns: Set<Conn>;
  app: AppDb;
  decks: DeckService;
  images: ImageCache;
  logger: (m: string) => void;
  send(c: Conn, m: ServerMsg): void;
  isConnected(userId: number): boolean;
  prefsOf(userId: number): import('@aetherhub/shared').UserPrefs;
  broadcastStatus(): void;
  /** the room's game was closed or suspended and should be forgotten */
  dropRoom(room: GameRoom): void;
  /** a game started from a draft / sealed event lobby ended */
  onRoomEnded?(room: GameRoom): void;
}

export interface SeatPick {
  userId: number;
  name: string;
  deckId: number | null;
  cards?: { def: string; section: Section; qty: number }[];
  defs?: Record<string, CardDef>;
}

export function seatInputs(h: RoomHost, list: SeatPick[]) {
  const used = new Set<string>();
  const defs: Record<string, CardDef> = {};
  const seats: SeatInput[] = list.map((s) => {
    const prefs = h.prefsOf(s.userId);
    let color = prefs.playerColor;
    if (used.has(color)) color = PLAYER_COLORS.find((c) => !used.has(c)) || color;
    used.add(color);
    const deck = s.deckId != null ? h.decks.forGame(s.deckId) : null;
    if (deck) Object.assign(defs, deck.defs);
    if (s.defs) Object.assign(defs, s.defs);
    return {
      userId: s.userId,
      name: s.name,
      color,
      deckId: s.deckId,
      deckName: deck?.name ?? 'No deck',
      sleeve: deck?.sleeve ?? null,
      playmat: prefs.playmat ? 'a/' + prefs.playmat : null,
      autoUntap: prefs.autoUntap,
      autoDraw: prefs.autoDraw,
      cards: s.cards ?? deck?.cards ?? [],
    };
  });
  return { seats, defs };
}

export function buildGame(h: RoomHost, settings: GameSettings, seats: SeatInput[], defs: Record<string, CardDef>, starter: number, match: GameState['match'] | undefined, id: string = crypto.randomUUID()): GameState {
  const state = createGame({ id, settings, seats, defs, rng: cryptoRng, now: Date.now(), starter, match, hostSeat: 0 });
  h.images.warm(Object.values(state.defs).flatMap((d) => d.img));
  return state;
}

/**
 * One running sandbox game (no rules enforced) and everyone watching it. Several rooms run at once; a player can hold
 * several seats of one game (a solo game) and chooses which one they act as.
 */
export class GameRoom {
  private sb: SbStage | null = null;
  private undoReqs = new Map<string, UndoReq>();
  /** the draft / sealed event lobby this game was started from */
  eventLobby: string | null = null;

  constructor(
    private h: RoomHost,
    public game: LiveGame,
  ) {}

  get id() {
    return this.game.id;
  }

  private conns(): Conn[] {
    return [...this.h.conns].filter((c) => c.room === this);
  }

  private userConns(userId: number): Conn[] {
    return this.conns().filter((c) => c.user.id === userId);
  }

  summary(userId: number): GameSummary {
    const g = this.game;
    return { id: g.id, name: g.state.settings.name, turn: g.state.turn.n, hasSeat: this.seatsOfUser(userId).length > 0, spectators: g.state.settings.spectators, status: 'active', ended: g.state.stage === 'ended' };
  }

  seatsOfUser(userId: number): number[] {
    return this.game.state.seats.flatMap((s, i) => (s.userId === userId ? [i] : []));
  }

  hasPlayer(userId: number) {
    return this.seatsOfUser(userId).length > 0;
  }

  private markConnected(userId: number, on: boolean) {
    const seats = this.seatsOfUser(userId);
    if (!seats.length || seats.every((i) => this.game.state.seats[i].connected === on)) return false;
    this.game.system((s) => seats.forEach((i) => (s.seats[i].connected = on)));
    return true;
  }

  onDisconnect(c: Conn) {
    if (this.hasPlayer(c.user.id) && !this.userConns(c.user.id).length) {
      if (this.markConnected(c.user.id, false)) this.pushGame();
    }
    // A newer tab closing hands the seat back to the remaining one.
    if (c.primary) {
      const rest = this.userConns(c.user.id);
      if (rest.length && !rest.some((x) => x.primary)) {
        const newest = rest.sort((a, b) => b.created - a.created)[0];
        newest.primary = true;
        this.sendFullGame(newest);
      }
    }
  }

  handle(c: Conn, m: ClientMsg) {
    switch (m.t) {
      case 'takeBack':
        this.takeSeat(c);
        break;
      case 'seat': {
        if (!this.seatsOfUser(c.user.id).includes(m.seat)) return;
        c.seat = m.seat;
        c.role = 'player';
        c.lastView = null;
        this.sendFullGame(c);
        if (this.sb) this.sendSideboard();
        break;
      }
      case 'intent':
        this.onIntent(c, m.id, m.intent);
        break;
      case 'undo':
        this.onUndo(c);
        break;
      case 'undoAnswer':
        this.onUndoAnswer(c, m.id, m.approve);
        break;
      case 'arrow':
        this.onArrow(c, m.from, m.to);
        break;
      case 'arrowRemove':
        this.broadcastGame({ t: 'arrowRemove', id: m.id });
        break;
      case 'ping':
        this.broadcastGame({ t: 'ping', iid: m.iid, seat: c.seat ?? -1, name: c.user.username });
        break;
      case 'host':
        this.onHost(c, m);
        break;
      case 'sideboard':
        this.onSideboard(c, m);
        break;
      case 'specChat':
        if (c.role === 'spectator') this.onIntent(c, 0, { type: 'chat', text: m.text });
        break;
    }
  }

  private effectiveHostUser(): number | null {
    const g = this.game;
    if (this.h.isConnected(g.meta.hostUserId)) return g.meta.hostUserId;
    const seats = g.state.seats;
    const hs = seats.findIndex((s) => s.userId === g.meta.hostUserId);
    for (let k = 1; k <= seats.length; k++) {
      const s = seats[(Math.max(0, hs) + k) % seats.length];
      if (this.h.isConnected(s.userId) && !s.conceded) return s.userId;
    }
    return g.meta.hostUserId;
  }

  isHost(user: HubUser): boolean {
    return user.isAdmin || this.effectiveHostUser() === user.id;
  }

  private hostSeat(): number | null {
    const u = this.effectiveHostUser();
    return u == null ? null : (this.seatsOfUser(u)[0] ?? null);
  }

  watch(c: Conn) {
    const g = this.game;
    const seats = this.seatsOfUser(c.user.id);
    if (!seats.length && !g.state.settings.spectators && !c.user.isAdmin) {
      this.h.send(c, { t: 'gameClosed', reason: 'Spectators are not allowed in this game' });
      return;
    }
    if (c.room && c.room !== this) c.room.unwatch(c);
    c.room = this;
    c.role = seats.length ? 'player' : 'spectator';
    // keep the seat this tab chose; otherwise the seat whose turn it is (or whose mulligan is open), else the first
    if (!seats.length) c.seat = null;
    else if (c.seat == null || !seats.includes(c.seat)) {
      const s = g.state;
      const deciding = s.stage === 'mulligan' ? seats.find((i) => !s.seats[i].mulligan?.kept && !s.seats[i].conceded) : seats.find((i) => i === s.turn.active);
      c.seat = deciding ?? seats[0];
    }
    if (seats.length) {
      // Newest connection wins the seats.
      for (const o of this.userConns(c.user.id)) {
        if (o !== c && o.primary) {
          o.primary = false;
          this.h.send(o, { t: 'openedElsewhere' });
        }
      }
      c.primary = true;
      if (this.markConnected(c.user.id, true)) this.pushGame(c);
    }
    this.sendFullGame(c);
    if (this.sb) this.sendSideboard();
    for (const r of this.undoReqs.values()) if (this.isHost(c.user)) this.h.send(c, { t: 'undoRequest', req: { id: r.id, from: r.seat, fromName: g.state.seats[r.seat]?.name ?? '?', label: r.label } });
  }

  unwatch(c: Conn) {
    c.room = null;
    c.lastView = null;
    c.role = 'none';
    this.onDisconnect(c);
  }

  private takeSeat(c: Conn) {
    for (const o of this.userConns(c.user.id)) if (o !== c && o.primary) this.h.send(o, { t: 'openedElsewhere' });
    for (const o of this.userConns(c.user.id)) o.primary = o === c;
    this.watch(c);
  }

  private viewerOf(c: Conn): Viewer {
    return c.role === 'player' && c.seat != null ? c.seat : 'spectator';
  }

  /** The view for a connection: a player holding several seats sees all their hands. */
  private viewFor(c: Conn): GameView {
    const v = this.viewerOf(c);
    return viewFor(this.game.state, v, v === 'spectator' ? [] : this.seatsOfUser(c.user.id).filter((i) => i !== v));
  }

  private logView(entries: LogEntry[], viewer: Viewer): LogView[] {
    const shared = this.game.state.settings.specChatShared ?? true;
    return entries.map((e) => logFor(e, viewer, shared)).filter(Boolean) as LogView[];
  }

  private sendFullGame(c: Conn) {
    const g = this.game;
    const v = this.viewerOf(c);
    const view = this.viewFor(c);
    c.lastView = view;
    this.h.send(c, {
      t: 'game',
      view,
      log: this.logView(g.log.slice(-400), v),
      seq: g.seq,
      host: this.isHost(c.user),
      hostSeat: this.hostSeat(),
      undoable: c.seat != null ? g.undoableCount(c.seat) : 0,
      mySeats: this.seatsOfUser(c.user.id),
    });
  }

  /** Pushes a diff of the current state to every watching connection (except `skip`). */
  pushGame(skip?: Conn) {
    const g = this.game;
    const cache = new Map<string, GameView>();
    const hs = this.hostSeat();
    for (const c of this.conns()) {
      if (c === skip || !c.primary) continue;
      const key = `${this.viewerOf(c)}|${c.role === 'player' ? c.user.id : ''}`;
      let view = cache.get(key);
      if (!view) {
        view = this.viewFor(c);
        cache.set(key, view);
      }
      if (!c.lastView) {
        this.sendFullGame(c);
        continue;
      }
      const ops = diff(c.lastView, view);
      c.lastView = view;
      this.h.send(c, { t: 'patch', ops, seq: g.seq, undoable: c.seat != null ? g.undoableCount(c.seat) : 0, host: this.isHost(c.user), hostSeat: hs });
    }
  }

  private pushLog(added: LogEntry[], updated: LogEntry[]) {
    for (const c of this.conns()) {
      if (!c.primary) continue;
      const v = this.viewerOf(c);
      const a = this.logView(added, v);
      if (a.length) this.h.send(c, { t: 'log', entries: a });
      for (const u of this.logView(updated, v)) this.h.send(c, { t: 'logUpdate', entry: u });
    }
  }

  private broadcastGame(m: ServerMsg) {
    for (const c of this.conns()) if (c.primary) this.h.send(c, m);
  }

  private sendNotices(notices: Notice[]) {
    const g = this.game;
    for (const n of notices) {
      for (const c of this.conns()) {
        if (!c.primary) continue;
        const to = n.to;
        const isTarget = to === 'all' || (c.role === 'player' && c.seat === to);
        if (!isTarget) continue;
        if (n.kind === 'cards') {
          const cards: Record<string, CardInst> = {};
          const defs: Record<string, CardDef> = {};
          for (const iid of n.iids) {
            const card = g.state.cards[iid];
            if (!card) continue;
            cards[iid] = card;
            if (g.state.defs[card.def]) defs[card.def] = g.state.defs[card.def];
          }
          this.h.send(c, { t: 'notice', notice: n, cards, defs });
        } else this.h.send(c, { t: 'notice', notice: n });
      }
    }
  }

  private onIntent(c: Conn, id: number, raw: unknown) {
    const g = this.game;
    if (!c.primary) return this.h.send(c, { t: 'reject', id, reason: 'This tab is not controlling your seat' });
    const parsed = IntentSchema.safeParse(raw);
    if (!parsed.success) return this.h.send(c, { t: 'reject', id, reason: 'Invalid action' });
    const intent = parsed.data;
    const seat = c.role === 'player' && c.seat != null && this.seatsOfUser(c.user.id).includes(c.seat) ? c.seat : null;
    const actor = seat ?? -1;
    if (actor < 0 && intent.type !== 'chat' && !c.user.isAdmin) return this.h.send(c, { t: 'reject', id, reason: 'Spectators can only chat' });
    try {
      const before = g.state.seats.map((s) => s.conceded);
      const res = g.apply(actor, c.user.id, seat != null ? g.state.seats[seat].name : c.user.username, intent, (keys) => this.h.decks.resolveDefs(keys));
      if (intent.type === 'concede' && seat != null && !before[seat]) {
        g.meta.concededTurn[seat] = g.state.turn.n;
        g.saveMeta();
      }
      this.h.send(c, { t: 'ack', id });
      this.pushGame();
      this.pushLog(res.added, res.updated);
      this.sendNotices(res.notices);
      if (intent.type === 'passTurn') this.h.broadcastStatus();
    } catch (e) {
      if (e instanceof IntentError) this.h.send(c, { t: 'reject', id, reason: e.message });
      else {
        this.h.logger('Intent failed: ' + (e as Error).stack);
        this.h.send(c, { t: 'reject', id, reason: 'Something went wrong' });
      }
    }
  }

  private onArrow(c: Conn, from: ArrowMsg['from'], to: ArrowMsg['to']) {
    const g = this.game;
    const seat = c.seat;
    const color = seat != null ? g.state.seats[seat].color : '#a3a7b0';
    const arrow: ArrowMsg = { id: crypto.randomUUID(), seat: seat ?? -1, color, from, to };
    this.broadcastGame({ t: 'arrow', arrow });
    const name = (x: ArrowMsg['to']) => (x.iid ? (g.state.cards[x.iid] && !g.state.cards[x.iid].faceDown ? g.state.defs[g.state.cards[x.iid].def]?.name : 'a card') : x.seat != null ? g.state.seats[x.seat]?.name : 'somewhere');
    const res = g.addLogs(seat ?? -1, seat != null ? g.state.seats[seat].name : c.user.username, [{ kind: 'act', text: `pointed an arrow from ${name(from)} to ${name(to)}` }]);
    this.pushLog(res.added, res.updated);
  }

  // ------------------------------------------------------------------ undo / rewind

  private onUndo(c: Conn) {
    const g = this.game;
    const seat = c.seat;
    if (seat == null || c.role !== 'player') return;
    const mine = this.seatsOfUser(c.user.id);
    const idx = g.lastOwnEvent(seat);
    if (idx < 0) return this.h.send(c, { t: 'notice', notice: { to: seat, kind: 'toast', text: 'Nothing of yours to undo' } });
    const ev = g.events[idx];
    // actions by this player's other seats (a solo game) don't need anyone's approval
    const others = g.events.slice(idx + 1).filter((e) => !mine.includes(e.actor) && e.intent.type !== 'chat');
    if (!others.length || this.isHost(c.user)) {
      this.doRewind(ev.seq, seat, ev.label, others.length);
      return;
    }
    const req: UndoReq = { id: crypto.randomUUID(), seat, userId: c.user.id, toSeq: ev.seq, label: `${ev.label} (would also undo ${others.length} action${others.length === 1 ? '' : 's'} by others)` };
    this.undoReqs.set(req.id, req);
    for (const x of this.conns()) if (this.isHost(x.user)) this.h.send(x, { t: 'undoRequest', req: { id: req.id, from: seat, fromName: g.state.seats[seat].name, label: req.label } });
    this.h.send(c, { t: 'notice', notice: { to: seat, kind: 'toast', text: 'Undo needs the host’s approval — asked' } });
  }

  private onUndoAnswer(c: Conn, id: string, approve: boolean) {
    const g = this.game;
    const r = this.undoReqs.get(id);
    if (!r) return;
    if (!this.isHost(c.user)) return;
    this.undoReqs.delete(id);
    this.broadcastGame({ t: 'undoResolved', id, approved: approve });
    if (!approve) {
      const res = g.addLogs(-1, 'Server', [{ kind: 'system', text: `The host declined ${g.state.seats[r.seat]?.name}'s undo` }]);
      this.pushLog(res.added, res.updated);
      return;
    }
    if (!g.events.some((e) => e.seq === r.toSeq)) return;
    this.doRewind(r.toSeq, r.seat, r.label, 1, c.user.username);
  }

  private doRewind(toSeq: number, seat: number, label: string, othersUndone: number, approvedBy?: string) {
    const g = this.game;
    g.rewind(toSeq);
    const name = g.state.seats[seat]?.name ?? 'Someone';
    const res = g.addLogs(seat, name, [
      { kind: 'act', text: `rewound: ${label.toLowerCase()}${othersUndone ? (approvedBy ? ` · approved by ${approvedBy}` : ' (as host)') : ' (nobody else had acted, so no approval needed)'}` },
    ]);
    this.pushGame();
    this.pushLog(res.added, res.updated);
  }

  // ------------------------------------------------------------------ host powers

  private onHost(c: Conn, m: Extract<ClientMsg, { t: 'host' }>) {
    const g = this.game;
    if (!this.isHost(c.user)) return this.h.send(c, { t: 'error', message: 'Only the host can do that' });
    switch (m.op) {
      case 'end':
        this.endGame(m.winner, c.user.username);
        break;
      case 'suspend':
        this.suspend(c.user.username);
        break;
      case 'removeSeat': {
        const s = g.state.seats[m.seat];
        if (!s) return;
        g.system((st) => {
          st.seats[m.seat].conceded = true;
          st.seats[m.seat].connected = false;
        });
        const res = g.addLogs(-1, 'Server', [{ kind: 'system', text: `${c.user.username} removed ${s.name} from the game` }]);
        this.pushGame();
        this.pushLog(res.added, res.updated);
        break;
      }
      case 'giveSeat': {
        const s = g.state.seats[m.seat];
        const u = get<{ id: number; username: string }>(this.h.app.db, 'SELECT id, username FROM users WHERE id = ?', m.userId);
        if (!s || !u) return;
        const old = s.name;
        g.system((st) => {
          st.seats[m.seat].userId = u.id;
          st.seats[m.seat].name = u.username;
          st.seats[m.seat].connected = this.h.isConnected(u.id);
          st.seats[m.seat].conceded = false;
        });
        const res = g.addLogs(-1, 'Server', [{ kind: 'system', text: `${c.user.username} gave ${old}'s seat to ${u.username}` }]);
        for (const x of this.h.conns) if (x.user.id === u.id && x.room === this) this.watch(x);
        // the old holder's tabs follow their remaining seats (or become spectators)
        for (const x of this.conns()) if (x.seat === m.seat && x.user.id !== u.id) this.watch(Object.assign(x, { seat: null }));
        this.pushGame();
        this.pushLog(res.added, res.updated);
        this.h.broadcastStatus();
        break;
      }
      case 'close':
        this.closeGame('closed');
        break;
      case 'rematch': {
        if (g.state.stage !== 'ended') return this.h.send(c, { t: 'error', message: 'End the game first' });
        this.nextGame(true);
        break;
      }
    }
  }

  private finishMatch(winner: number | null) {
    const g = this.game;
    const s = g.state;
    const now = Date.now();
    const bo = s.settings.bo;
    const db = this.h.app.db;
    tx(db, () => {
      const r = run(
        db,
        'INSERT INTO matches (game_id, name, format, bo, started_at, ended_at, turns, meta, winner_name) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)',
        g.id,
        s.settings.name,
        s.settings.format,
        bo,
        g.meta.matchStartedAt,
        now,
        s.turn.n,
        JSON.stringify({ players: s.seats.length, planechase: s.settings.planechase, archenemy: s.settings.archenemy, dungeons: s.settings.dungeons, games: s.match.results.length, type: s.settings.type ?? 'sandbox' }),
        winner != null ? s.seats[winner].name : null,
      );
      const mid = Number(r.lastInsertRowid);
      s.seats.forEach((p, i) => {
        let result = '—';
        if (bo === 3) {
          const w = s.match.wins[i] || 0;
          const l = s.match.results.filter((x) => x != null && x !== i).length;
          result = `${i === winner ? 'Won' : 'Lost'} ${w}–${l}`;
        } else if (i === winner) result = 'Winner';
        else if (p.conceded) result = g.meta.concededTurn[i] ? `Conceded turn ${g.meta.concededTurn[i]}` : 'Conceded';
        else if (winner != null) result = 'Lost';
        run(db, 'INSERT INTO match_players (match_id, seat, user_id, name, color, deck_name, result) VALUES (?, ?, ?, ?, ?, ?, ?)', mid, i, p.userId, p.name, p.color, p.deckName, result);
      });
      g.meta.matchLog.forEach((l, i) => {
        run(db, 'INSERT INTO match_log (match_id, idx, t_offset, who, color, kind, text) VALUES (?, ?, ?, ?, ?, ?, ?)', mid, i, l.t, l.who, l.color, l.kind, l.text);
      });
    });
  }

  endGame(winner: number | null, by: string) {
    const g = this.game;
    const s = g.state;
    g.system((st) => {
      st.stage = 'ended';
      st.winner = winner;
      if (winner != null) st.match.wins[winner] = (st.match.wins[winner] || 0) + 1;
      st.match.results.push(winner);
    });
    const res = g.addLogs(-1, 'Server', [{ kind: 'system', text: `${by} ended the game${winner != null ? ` · winner: ${s.seats[winner].name}` : ''}` }]);
    this.pushGame();
    this.pushLog(res.added, res.updated);
    const st = g.state;
    const matchOver = st.settings.bo === 1 || st.match.wins.some((w) => w >= 2) || st.match.results.length >= 3;
    if (matchOver) {
      const mw = st.settings.bo === 1 ? winner : st.match.wins.findIndex((w) => w === Math.max(...st.match.wins));
      this.finishMatch(mw != null && mw >= 0 ? mw : null);
      g.setStatus('finished');
      g.meta.matchLog.push({ t: Date.now() - g.meta.matchStartedAt, who: 'Server', color: '#8f939c', kind: 'system', text: 'Match recorded' });
      this.h.onRoomEnded?.(this);
    } else this.startSideboarding(winner);
    this.h.broadcastStatus();
  }

  /** Closes the game (finished or not). */
  closeGame(reason: string) {
    const g = this.game;
    if (g.state.stage !== 'ended') g.setStatus('finished');
    this.sb = null;
    this.undoReqs.clear();
    for (const c of this.conns()) {
      this.h.send(c, { t: 'gameClosed', reason });
      c.room = null;
      c.lastView = null;
      c.role = 'none';
      c.seat = null;
    }
    this.h.dropRoom(this);
    this.h.onRoomEnded?.(this);
    this.h.broadcastStatus();
  }

  suspend(by: string) {
    const g = this.game;
    const res = g.addLogs(-1, 'Server', [{ kind: 'system', text: `${by} suspended the game` }]);
    this.pushLog(res.added, res.updated);
    g.setStatus('suspended', by);
    this.sb = null;
    for (const c of this.conns()) {
      this.h.send(c, { t: 'gameClosed', reason: 'suspended' });
      c.room = null;
      c.lastView = null;
    }
    this.h.dropRoom(this);
    this.h.broadcastStatus();
  }

  // ------------------------------------------------------------------ Bo3 sideboarding

  private startSideboarding(lastWinner: number | null) {
    const g = this.game;
    const s = g.state;
    const seats: SbSeat[] = s.seats.map((p) => {
      const deck = p.deckId != null ? this.h.decks.forGame(p.deckId) : null;
      const main = new Map<string, number>();
      const side = new Map<string, number>();
      const other: SbSeat['other'] = [];
      for (const c of deck?.cards ?? []) {
        if (c.section === 'main') main.set(c.def, (main.get(c.def) || 0) + c.qty);
        else if (c.section === 'side') side.set(c.def, (side.get(c.def) || 0) + c.qty);
        else other.push(c);
      }
      return { userId: p.userId, deckId: p.deckId, main, side, other, defs: deck?.defs ?? {} };
    });
    // The loser of the last game chooses who starts (multiplayer: the first non-winner in turn order).
    const chooser = s.seats.findIndex((_p, i) => i !== lastWinner);
    this.sb = { seats, ready: s.seats.map(() => false), chooser: chooser >= 0 ? chooser : null, starter: null };
    const res = g.addLogs(-1, 'Server', [{ kind: 'system', text: `Game ${s.match.game} over · sideboarding before game ${s.match.game + 1} (saved decks are not changed)` }]);
    this.pushLog(res.added, res.updated);
    this.sendSideboard();
  }

  private sendSideboard() {
    const g = this.game;
    const sb = this.sb;
    for (const c of this.conns()) {
      if (!c.primary) continue;
      if (!sb) {
        this.h.send(c, { t: 'sideboard', state: null });
        continue;
      }
      const seat = c.role === 'player' ? c.seat : null;
      const mine = seat != null ? sb.seats[seat] : null;
      const state: SideboardState = {
        seat: seat ?? -1,
        main: mine ? [...mine.main].map(([def, qty]) => ({ def, qty })) : [],
        side: mine ? [...mine.side].map(([def, qty]) => ({ def, qty })) : [],
        defs: mine ? mine.defs : {},
        ready: sb.ready,
        chooser: sb.chooser,
        names: g.state.seats.map((s) => s.name),
        wins: g.state.match.wins,
        game: g.state.match.game,
      };
      this.h.send(c, { t: 'sideboard', state });
    }
  }

  private onSideboard(c: Conn, m: Extract<ClientMsg, { t: 'sideboard' }>) {
    const sb = this.sb;
    const g = this.game;
    if (!sb) return;
    const seat = c.role === 'player' && c.seat != null && this.seatsOfUser(c.user.id).includes(c.seat) ? c.seat : null;
    if (m.op === 'choose') {
      if ((seat == null || !this.seatsOfUser(c.user.id).includes(sb.chooser ?? -1)) && !this.isHost(c.user)) return;
      sb.starter = m.starter;
      const res = g.addLogs(seat ?? -1, c.user.username, [{ kind: 'act', text: `chose ${g.state.seats[m.starter]?.name} to start game ${g.state.match.game + 1}` }]);
      this.pushLog(res.added, res.updated);
    } else if (seat != null) {
      const me = sb.seats[seat];
      if (m.op === 'swap') {
        const from = m.to === 'main' ? me.side : me.main;
        const to = m.to === 'main' ? me.main : me.side;
        const n = from.get(m.def) || 0;
        if (n <= 0) return;
        if (n === 1) from.delete(m.def);
        else from.set(m.def, n - 1);
        to.set(m.def, (to.get(m.def) || 0) + 1);
        sb.ready[seat] = false;
      } else if (m.op === 'ready') sb.ready[seat] = m.value;
    }
    if (sb.ready.every((r, i) => r || g.state.seats[i].conceded) && sb.starter != null) {
      this.nextGame(false);
      return;
    }
    this.sendSideboard();
  }

  private nextGame(rematch: boolean) {
    const g = this.game;
    const old = g.state;
    const sb = this.sb;
    const list: SeatPick[] = old.seats.map((p, i) => {
      if (sb && !rematch) {
        const x = sb.seats[i];
        const cards = [...x.other, ...[...x.main].map(([def, qty]) => ({ def, qty, section: 'main' as Section })), ...[...x.side].map(([def, qty]) => ({ def, qty, section: 'side' as Section }))];
        return { userId: p.userId, name: p.name, deckId: p.deckId, cards, defs: x.defs };
      }
      return { userId: p.userId, name: p.name, deckId: p.deckId };
    });
    const inputs = seatInputs(this.h, list);
    const match = rematch ? undefined : { game: old.match.game + 1, wins: old.match.wins.slice(), results: old.match.results.slice() };
    const starter = sb?.starter ?? 0;
    // The next game of the match (or the rematch) keeps the room's id: the hub's room map, the players' URLs and the
    // resume-after-restart lookup all use it. LiveGame.create replaces the finished game's rows under that id.
    const state = buildGame(this.h, old.settings, inputs.seats, inputs.defs, rematch ? 0 : starter, match, g.id);
    const meta: GameMeta = rematch ? { hostUserId: g.meta.hostUserId, matchStartedAt: Date.now(), matchLog: [], concededTurn: {} } : { ...g.meta, concededTurn: {} };
    this.game = LiveGame.create(this.h.app, state, meta);
    this.sb = null;
    for (const c of this.conns()) c.lastView = null;
    this.sendSideboard();
    this.afterStart();
  }

  /** Log the start, mark who is connected, and send the table to everyone already watching or seated. */
  afterStart() {
    const g = this.game;
    const s = g.state;
    const order = s.seats.map((x) => x.name).join(', ');
    g.addLogs(-1, 'Server', [
      {
        kind: 'system',
        text: `Game ${s.match.game > 1 ? s.match.game + ' ' : ''}started · turn order ${order} · ${s.seats[s.turn.active].name} goes first${s.settings.mulligan !== 'none' ? ' · mulligan: ' + s.settings.mulligan : ''}`,
      },
    ]);
    g.system((st) => st.seats.forEach((x) => (x.connected = this.h.isConnected(x.userId))));
    this.h.broadcastStatus();
    for (const c of this.h.conns) if (c.room === this || (!c.room && this.hasPlayer(c.user.id))) this.watch(c);
  }
}
