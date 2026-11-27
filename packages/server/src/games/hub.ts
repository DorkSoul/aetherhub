import type WebSocket from 'ws';
import type { AdminGame, AiLevel, ClientMsg, GameSettings, GameState, LobbySlot, LobbyView, ServerMsg, ServerStatus } from '@aetherhub/shared';
import { mergePrefs } from '@aetherhub/shared';
import type { LogFn } from '../activity';
import type { AppDb } from '../db/app';
import type { CardDb } from '../db/cards';
import { all, get, run } from '../db/sqlite';
import type { DeckService } from '../decks';
import type { ImageCache } from '../images';
import { PRECON_OWNER, PreconService } from '../precons';
import { LiveGame, type GameMeta } from './live';
import { Lobby, LobbyError, type DeckRef } from './lobby';
import { buildGame, GameRoom, seatInputs, type Conn, type HubUser, type RoomHost } from './room';

export type { HubUser } from './room';

/** One seat of a rules enforced game started from a lobby. */
export interface RulesSeatStart {
  userId: number | null;
  name: string;
  deckId: number | null;
  difficulty: AiLevel;
  /** the deck as card names and counts instead of a saved deck (an AI drafter's deck) */
  cards?: [string, number][];
}

/** Hooks the rules and draft routes register, so a lobby can start those kinds of game. */
export interface GameStarters {
  rules?: (o: { settings: GameSettings; seats: RulesSeatStart[]; by: HubUser }) => string;
  draft?: (o: { lobby: Lobby; by: HubUser }) => string;
}

/**
 * Every lobby and running sandbox game on the server, and every browser connection. Several lobbies and games run at
 * once; a user is in at most one lobby at a time.
 */
/** How long a user has to stay disconnected before the log says they went offline. */
const OFFLINE_AFTER_MS = 30_000;

export class GameHub implements RoomHost {
  lobbies = new Map<string, Lobby>();
  rooms = new Map<string, GameRoom>();
  conns = new Set<Conn>();
  starters: GameStarters = {};
  /** Where each started game's players go back to: a normal lobby as it was when it started (it closed then and is
   *  re-opened by the first player back, `reopened`), or the draft / sealed event lobby that stays open. */
  private afterGame = new Map<string, { at: number; event: string | null; settings?: GameSettings; password?: string | null; slots?: LobbySlot[]; reopened?: string | null }>();
  /** a draft / sealed event lobby changed (saved so a restart keeps it) */
  onEventLobby?: (l: Lobby) => void;
  /** a lobby closed (its event, if any, is deleted) */
  onLobbyClosed?: (l: Lobby) => void;
  /** an AI drafter's deck as card names and counts */
  eventDeckCards?: (draftId: string, seat: number) => [string, number][] | null;
  /** whether a rules game is still running (event lobbies list their running games) */
  rulesAlive?: (id: string) => boolean;
  /** the rules games the server holds, for the admin Server page */
  rulesGames?: () => AdminGame[];
  private nextConnId = 1;
  /** users logged as online, and the pending "went offline" entries of those whose last tab closed */
  private online = new Set<number>();
  private offlineTimers = new Map<number, NodeJS.Timeout>();
  private heartbeat: NodeJS.Timeout | null = null;
  statusExtra: () => Pick<ServerStatus, 'serverName' | 'cardDataUpdatedAt' | 'cardCount' | 'importing' | 'precons'> = () => ({ serverName: 'Aetherhub', cardDataUpdatedAt: null, cardCount: 0, importing: false, precons: false });

  constructor(
    public app: AppDb,
    public cards: CardDb,
    public decks: DeckService,
    public images: ImageCache,
    public logger: LogFn = () => {},
  ) {}

  init(heartbeatMs: number) {
    for (const row of all<{ id: string }>(this.app.db, "SELECT id FROM games WHERE status = 'active' ORDER BY updated_at DESC")) {
      // One corrupt game row must not stop the server from booting.
      try {
        const g = LiveGame.load(this.app, row.id);
        if (!g) continue;
        g.system((s) => s.seats.forEach((x) => (x.connected = false)));
        this.rooms.set(g.id, new GameRoom(this, g));
        this.logger(`Resumed game ${g.state.settings.name} at event ${g.seq}`);
      } catch (e) {
        this.logger(`Game ${row.id} could not be resumed: ${(e as Error).message}`, { level: 'error', detail: (e as Error).stack });
      }
    }
    this.heartbeat = setInterval(() => {
      for (const c of this.conns) {
        if (!c.alive) {
          c.ws.terminate();
          continue;
        }
        c.alive = false;
        try {
          c.ws.ping();
        } catch {
          /* closed */
        }
      }
    }, heartbeatMs);
    this.heartbeat.unref();
  }

  stop() {
    if (this.heartbeat) clearInterval(this.heartbeat);
    for (const t of this.offlineTimers.values()) clearTimeout(t);
    this.offlineTimers.clear();
    for (const c of this.conns) c.ws.close();
  }

  // ------------------------------------------------------------------ connections

  send(c: Conn, m: ServerMsg) {
    if (c.ws.readyState === 1) c.ws.send(JSON.stringify(m));
  }

  private userConns(userId: number): Conn[] {
    return [...this.conns].filter((c) => c.user.id === userId);
  }

  isConnected(userId: number): boolean {
    return this.userConns(userId).length > 0;
  }

  prefsOf(userId: number) {
    const r = get<{ json: string }>(this.app.db, 'SELECT json FROM user_prefs WHERE user_id = ?', userId);
    return mergePrefs(r ? JSON.parse(r.json) : null);
  }

  attach(ws: WebSocket, user: HubUser) {
    const c: Conn = { id: this.nextConnId++, ws, user, alive: true, room: null, primary: true, created: Date.now(), lastView: null, role: 'none', seat: null };
    clearTimeout(this.offlineTimers.get(user.id));
    this.offlineTimers.delete(user.id);
    if (!this.online.has(user.id)) {
      this.online.add(user.id);
      this.logger('Came online', { cat: 'auth', user: user.username });
    }
    this.conns.add(c);
    // ws emits 'error' for an oversized frame (maxPayload), invalid UTF-8 or a malformed frame; without a listener
    // Node treats it as an uncaught exception and the whole server exits. Drop just this connection ('close' follows).
    ws.on('error', (e) => {
      this.logger(`WebSocket error: ${e.message}`, { level: 'warn', cat: 'server', user: user.username });
      ws.terminate();
    });
    ws.on('pong', () => (c.alive = true));
    ws.on('message', (data) => {
      c.alive = true;
      let msg: ClientMsg;
      try {
        msg = JSON.parse(String(data));
      } catch {
        return;
      }
      try {
        this.handle(c, msg);
      } catch (e) {
        this.send(c, { t: 'error', message: (e as Error).message });
      }
    });
    ws.on('close', () => {
      this.conns.delete(c);
      c.room?.onDisconnect(c);
      this.lobbyPresence(user.id);
      // a reload or a dropped connection that comes back soon isn't worth an entry
      if (!this.isConnected(user.id) && this.online.has(user.id)) {
        clearTimeout(this.offlineTimers.get(user.id));
        const t = setTimeout(() => {
          this.offlineTimers.delete(user.id);
          if (this.isConnected(user.id)) return;
          this.online.delete(user.id);
          this.logger('Went offline', { cat: 'auth', user: user.username });
        }, OFFLINE_AFTER_MS);
        t.unref();
        this.offlineTimers.set(user.id, t);
      }
    });
    this.send(c, { t: 'status', status: this.status(user.id) });
    this.sendLobby(c);
    this.lobbyPresence(user.id);
  }

  handle(c: Conn, m: ClientMsg) {
    switch (m.t) {
      case 'hello':
        this.send(c, { t: 'status', status: this.status(c.user.id) });
        this.sendLobby(c);
        break;
      case 'watch': {
        const room = this.rooms.get(m.game);
        if (!room) {
          this.send(c, { t: 'gameClosed', reason: 'none' });
          return;
        }
        room.watch(c);
        break;
      }
      case 'unwatch':
        c.room?.unwatch(c);
        break;
      default:
        c.room?.handle(c, m);
    }
  }

  // ------------------------------------------------------------------ status

  status(userId: number): ServerStatus {
    return {
      ...this.statusExtra(),
      lobbies: [...this.lobbies.values()].map((l) => l.summary(userId)),
      games: [...this.rooms.values()].map((r) => r.summary(userId)),
    };
  }

  broadcastStatus() {
    for (const c of this.conns) this.send(c, { t: 'status', status: this.status(c.user.id) });
  }

  dropRoom(room: GameRoom) {
    if (this.rooms.delete(room.id)) this.logger(`Sandbox game ended: ${room.game.state.settings.name}`);
  }

  /** Every game the server is running (sandbox and rules), for the admin Server page. */
  adminGames(): AdminGame[] {
    const sandbox = [...this.rooms.values()].map((r): AdminGame => {
      const st = r.game.state;
      return {
        id: r.id,
        kind: 'sandbox',
        name: st.settings.name,
        players: st.seats.map((x) => x.name + (x.connected ? '' : ' (away)')),
        turn: st.turn.n,
        lastActive: r.game.events.at(-1)?.at ?? null,
        state: st.stage === 'ended' ? 'over' : 'running',
      };
    });
    return [...sandbox, ...(this.rulesGames?.() ?? [])];
  }

  /** The lobby this user is in. */
  lobbyOf(userId: number): Lobby | undefined {
    return [...this.lobbies.values()].find((l) => l.member(userId));
  }

  private sendLobby(c: Conn) {
    const l = this.lobbyOf(c.user.id);
    this.send(c, { t: 'lobby', lobby: l ? l.view() : null });
  }

  private broadcastLobby(l: Lobby) {
    // an event lobby lists only the games still running
    if (l.eventGames.length) l.eventGames = l.eventGames.filter((g) => (g.kind === 'sandbox' ? this.rooms.has(g.id) : (this.rulesAlive?.(g.id) ?? true)));
    if (l.phase !== 'setup' && this.lobbies.has(l.id)) this.onEventLobby?.(l);
    const v: LobbyView = l.view();
    for (const c of this.conns) if (l.member(c.user.id)) this.send(c, { t: 'lobby', lobby: v });
    this.broadcastStatus();
  }

  /** Mark a member connected or not (shown in the lobby) when their last tab closes or a tab opens. */
  private lobbyPresence(userId: number) {
    const l = this.lobbyOf(userId);
    const m = l?.member(userId);
    if (!l || !m) return;
    const on = this.isConnected(userId);
    if (m.connected === on) return;
    m.connected = on;
    this.broadcastLobby(l);
  }

  // ------------------------------------------------------------------ lobbies (called from REST)

  requireLobby(id: string): Lobby {
    const l = this.lobbies.get(id);
    if (!l) throw new LobbyError('That lobby is closed');
    return l;
  }

  createLobby(user: HubUser, settings: Partial<GameSettings>, password: string | null) {
    const old = this.lobbyOf(user.id);
    if (old) this.leaveLobby(user, old.id);
    const defaults: Partial<GameSettings> = {
      format: this.app.setting('defaultFormat', 'commander'),
      maxPlayers: parseInt(this.app.setting('defaultMaxPlayers', '2'), 10) || 2,
      mulligan: 'london',
      name: `${user.username}’s game`,
    };
    const l = new Lobby(user.id, { name: user.username, color: this.prefsOf(user.id).playerColor }, { ...defaults, ...settings }, password);
    l.members[0].connected = this.isConnected(user.id);
    this.lobbies.set(l.id, l);
    this.broadcastLobby(l);
    return l;
  }

  /** Run a change on a lobby and tell its members; an empty lobby closes (an event lobby keeps running games). */
  lobbyOp(id: string, fn: (l: Lobby) => void) {
    const l = this.requireLobby(id);
    fn(l);
    if (!l.members.length && !l.eventGames.length) {
      this.lobbies.delete(l.id);
      this.onLobbyClosed?.(l);
    }
    this.broadcastLobby(l);
    return l;
  }

  joinLobby(user: HubUser, id: string, password: string | null) {
    const l = this.requireLobby(id);
    const old = this.lobbyOf(user.id);
    if (old && old !== l) this.leaveLobby(user, old.id);
    this.lobbyOp(id, (x) => x.join(user, this.prefsOf(user.id).playerColor, password));
    const m = l.member(user.id);
    if (m) m.connected = this.isConnected(user.id);
  }

  leaveLobby(user: HubUser, id: string) {
    this.lobbyOp(id, (l) => l.leave(user.id));
    for (const c of this.userConns(user.id)) this.send(c, { t: 'lobby', lobby: null });
  }

  kick(user: HubUser, id: string, target: number) {
    this.lobbyOp(id, (l) => l.kick(user, target));
    for (const c of this.userConns(target)) this.send(c, { t: 'lobby', lobby: null });
  }

  lobbyChat(user: HubUser, id: string, text: string) {
    this.lobbyOp(id, (l) => {
      const m = l.member(user.id);
      if (!m) throw new LobbyError('You are not in the lobby');
      l.chat.push({ who: m.name, color: m.color, text: text.slice(0, 500) });
    });
  }

  deckRef(deckId: number | null): DeckRef | null {
    if (deckId == null) return null;
    if (PreconService.isPreconId(deckId)) {
      const p = this.decks.precons.contents(deckId);
      if (!p) throw new LobbyError('Deck not found');
      return { id: deckId, name: p.name, owner: PRECON_OWNER };
    }
    const r = get<DeckRef>(this.app.db, 'SELECT d.id, d.name, u.username AS owner FROM decks d JOIN users u ON u.id = d.owner_id WHERE d.id = ?', deckId);
    if (!r) throw new LobbyError('Deck not found');
    return r;
  }

  // ------------------------------------------------------------------ starting games

  private tellStarted(l: Lobby, kind: 'sandbox' | 'rules' | 'draft', id: string, users: number[]) {
    for (const c of this.conns) if (users.includes(c.user.id)) this.send(c, { t: 'started', kind, id });
  }

  /** Start the lobby's game (host only). A normal lobby closes; a draft / sealed event lobby stays as the event's home. */
  startGame(user: HubUser, id: string): { kind: 'sandbox' | 'rules' | 'draft'; id: string } {
    const l = this.requireLobby(id);
    if (!l.isHost(user)) throw new LobbyError('Only the host can start the game');
    const type = l.type;
    if (type === 'draft' || type === 'sealed') {
      if (l.phase !== 'setup') throw new LobbyError('The event has already started');
      if (!l.slots.some((s) => s.kind === 'human')) throw new LobbyError('Nobody is seated');
      // sealed can be practised alone; a draft needs a pod of two or more (AI drafters count)
      const block = l.blockers().filter((b) => !b.startsWith('at least two') || type === 'draft');
      if (block.length) throw new LobbyError(`Waiting for ${block.join(', ')}`);
      if (!this.starters.draft) throw new LobbyError('Draft and sealed are not available on this server');
      const draftId = this.starters.draft({ lobby: l, by: user });
      l.draftId = draftId;
      l.phase = type === 'draft' ? 'drafting' : 'building';
      l.system(type === 'draft' ? 'The draft has started' : 'The sealed pools are open: build your decks');
      this.broadcastLobby(l);
      this.tellStarted(l, 'draft', draftId, l.members.map((m) => m.userId));
      return { kind: 'draft', id: draftId };
    }
    const block = l.blockers();
    if (block.length) throw new LobbyError(`Waiting for ${block.join(', ')}`);
    const seats = l.slots.filter((s) => s.kind !== 'open').map((s) => ({ ...s }));
    const users = [...new Set(seats.flatMap((s) => (s.kind === 'human' ? [s.userId] : [])))];
    // one player in several seats (a solo game): number the seats so the logs can tell them apart
    for (const u of users) {
      const mine = seats.filter((s) => s.kind === 'human' && s.userId === u) as { name: string }[];
      if (mine.length > 1) mine.forEach((s, k) => (s.name = `${s.name} ${k + 1}`));
    }
    let out: { kind: 'sandbox' | 'rules'; id: string };
    if (type === 'rules') {
      if (!this.starters.rules) throw new LobbyError('The rules engine is not available on this server');
      const rid = this.starters.rules({
        settings: l.settings,
        by: user,
        seats: seats.map((s) => (s.kind === 'human' ? { userId: s.userId, name: s.name, deckId: s.deckId, difficulty: 'Normal' as AiLevel } : { userId: null, name: (s as { name: string }).name, deckId: s.deckId, difficulty: (s as { difficulty: AiLevel }).difficulty })),
      });
      out = { kind: 'rules', id: rid };
    } else {
      const room = this.startSandbox(
        l.settings,
        l.hostId,
        seats.map((s) => ({ userId: (s as { userId: number }).userId, name: (s as { name: string }).name, deckId: s.deckId })),
      );
      out = { kind: 'sandbox', id: room.id };
    }
    this.lobbies.delete(l.id);
    this.rememberLobby(out.id, { event: null, settings: { ...l.settings }, password: l.password, slots: l.slots.map((x) => ({ ...x })), reopened: null });
    for (const c of this.conns) if (l.member(c.user.id)) this.send(c, { t: 'lobby', lobby: null });
    this.tellStarted(l, out.kind, out.id, users);
    this.broadcastStatus();
    return out;
  }

  private rememberLobby(gameId: string, v: { event: string | null; settings?: GameSettings; password?: string | null; slots?: LobbySlot[]; reopened?: string | null }) {
    const now = Date.now();
    for (const [k, x] of this.afterGame) if (now - x.at > 24 * 3600_000) this.afterGame.delete(k);
    this.afterGame.set(gameId, { at: now, ...v });
  }

  /**
   * "Back to lobby" after a game: the event lobby it came from, or its own lobby re-opened as it was (same settings,
   * the AIs and their decks; this player back in their seats with their decks, the other players' seats kept free for
   * them). The other players who come back join that same lobby. Returns the lobby id.
   */
  backToLobby(user: HubUser, gameId: string): string {
    const a = this.afterGame.get(gameId);
    if (!a) throw new LobbyError('The lobby of that game is no longer available: start a new one from the main menu');
    const target = a.event ?? a.reopened ?? null;
    let l = target ? this.lobbies.get(target) : undefined;
    if (a.event && !l) throw new LobbyError('That event lobby has closed');
    if (l) {
      if (!l.member(user.id)) this.joinLobby(user, l.id, l.password);
      if (a.slots) this.lobbyOp(l.id, (x) => x.restoreSeats(user.id, a.slots!));
      return l.id;
    }
    const old = this.lobbyOf(user.id);
    if (old) this.leaveLobby(user, old.id);
    l = new Lobby(user.id, { name: user.username, color: this.prefsOf(user.id).playerColor }, a.settings!, a.password ?? null);
    l.members[0].connected = this.isConnected(user.id);
    l.slots = a.slots!.map((x) => (x.kind === 'ai' ? { ...x } : ({ kind: 'open' } as LobbySlot)));
    l.restoreSeats(user.id, a.slots!);
    if (!l.seatsOf(user.id).length) {
      const open = l.slots.findIndex((x) => x.kind === 'open');
      if (open >= 0) l.sit(user, open, { userId: user.id });
    }
    l.system(`${user.username} is back from the game`);
    a.reopened = l.id;
    this.lobbies.set(l.id, l);
    this.broadcastLobby(l);
    return l.id;
  }

  /** "Main menu" after a game: leave the lobby it re-opened (which closes when nobody else is in it). */
  leaveAfterGame(user: HubUser, gameId: string) {
    const a = this.afterGame.get(gameId);
    const l = a?.reopened ? this.lobbies.get(a.reopened) : undefined;
    if (l?.member(user.id)) this.leaveLobby(user, l.id);
  }

  /** A sandbox game for these seats (the same user may hold several); returns its room. */
  startSandbox(settings: GameSettings, hostUserId: number, list: { userId: number; name: string; deckId: number | null; cards?: { def: string; section: import('@aetherhub/shared').Section; qty: number }[] }[], eventLobby?: string): GameRoom {
    const now = Date.now();
    const meta: GameMeta = { hostUserId, matchStartedAt: now, matchLog: [], concededTurn: {} };
    const inputs = seatInputs(this, list);
    const state = buildGame(this, { ...settings, type: 'sandbox' }, inputs.seats, inputs.defs, 0, undefined);
    const room = new GameRoom(this, LiveGame.create(this.app, state, meta));
    room.eventLobby = eventLobby ?? null;
    this.rooms.set(room.id, room);
    this.logger(`Sandbox game started: ${settings.name} (${list.map((x) => x.name).join(', ')})`);
    room.afterStart();
    return room;
  }

  /**
   * A game from a draft / sealed event: the chosen drafters (pod seats) play their event decks, sandbox or rules
   * enforced (AI drafters only in rules enforced games). Any member can start a game they play in; the host any game.
   */
  startEventGame(user: HubUser, id: string, podSeats: number[], type: 'sandbox' | 'rules') {
    const l = this.requireLobby(id);
    if (l.phase !== 'event' && l.phase !== 'building') throw new LobbyError('The decks are not built yet');
    const seats = [...new Set(podSeats)].map((i) => l.eventDecks[i]);
    if (seats.some((x) => !x)) throw new LobbyError('No such drafter');
    if (seats.length < 2 || seats.length > 8) throw new LobbyError('A game needs 2 to 8 players');
    if (!l.isHost(user) && !seats.some((x) => x.userId === user.id)) throw new LobbyError('Only the host can start a game you are not playing in');
    const missing = seats.filter((x) => (x.userId === null ? !x.deckName : x.deckId == null));
    if (missing.length) throw new LobbyError(`${missing.map((x) => x.name).join(', ')} has no deck yet`);
    if (type === 'sandbox' && seats.some((x) => x.userId === null)) throw new LobbyError('AI drafters can only play rules enforced games');
    const settings: GameSettings = { ...l.settings, type, format: 'limited', life: 20, maxPlayers: seats.length, planechase: false, archenemy: false };
    const names = seats.map((x) => x.name);
    let out: { kind: 'sandbox' | 'rules'; id: string };
    if (type === 'rules') {
      if (!this.starters.rules) throw new LobbyError('The rules engine is not available on this server');
      const rid = this.starters.rules({
        settings,
        by: user,
        seats: seats.map((x) =>
          x.userId === null ? { userId: null, name: x.name, deckId: null, difficulty: 'Normal' as AiLevel, cards: this.eventDeckCards?.(l.draftId!, x.seat) ?? undefined } : { userId: x.userId, name: x.name, deckId: x.deckId, difficulty: 'Normal' as AiLevel },
        ),
      });
      out = { kind: 'rules', id: rid };
    } else {
      const room = this.startSandbox(settings, user.id, seats.map((x) => ({ userId: x.userId!, name: x.name, deckId: x.deckId })), l.id);
      out = { kind: 'sandbox', id: room.id };
    }
    l.eventGames.push({ id: out.id, kind: out.kind, names });
    this.rememberLobby(out.id, { event: l.id });
    l.system(`${user.username} started a ${type === 'rules' ? 'rules enforced' : 'sandbox'} game: ${names.join(' vs ')}`);
    this.broadcastLobby(l);
    this.tellStarted(l, out.kind, out.id, [...new Set(seats.flatMap((x) => (x.userId !== null ? [x.userId] : [])))]);
    return out;
  }

  onRoomEnded(room: GameRoom) {
    if (!room.eventLobby) return;
    const l = this.lobbies.get(room.eventLobby);
    if (!l) return;
    l.eventGames = l.eventGames.filter((g) => g.id !== room.id);
    this.broadcastLobby(l);
  }

  // ------------------------------------------------------------------ suspended games

  loadSuspended(user: HubUser, id: string) {
    const row = get<{ status: string }>(this.app.db, 'SELECT status FROM games WHERE id = ?', id);
    if (!row || row.status !== 'suspended') throw new LobbyError('That game is not suspended');
    const g = LiveGame.load(this.app, id);
    if (!g) throw new LobbyError('Could not load the game');
    const inGame = g.state.seats.some((s) => s.userId === user.id);
    if (!inGame && !user.isAdmin) throw new LobbyError('Only players of that game can load it');
    g.setStatus('active');
    g.system((s) => s.seats.forEach((x) => (x.connected = false)));
    g.addLogs(-1, 'Server', [{ kind: 'system', text: `${user.username} loaded the suspended game` }]);
    const room = new GameRoom(this, g);
    this.rooms.set(g.id, room);
    this.broadcastStatus();
    return g;
  }

  suspendedGames() {
    return all<{ id: string; name: string; settings: string; meta: string; suspended_by: string | null; suspended_at: number | null; seq: number }>(
      this.app.db,
      "SELECT id, name, settings, meta, suspended_by, suspended_at, seq FROM games WHERE status = 'suspended' ORDER BY suspended_at DESC",
    ).map((r) => {
      const snap = get<{ state: string }>(this.app.db, 'SELECT state FROM game_snapshots WHERE game_id = ? ORDER BY seq DESC LIMIT 1', r.id);
      const st = snap ? (JSON.parse(snap.state) as GameState) : null;
      return {
        id: r.id,
        name: r.name,
        suspendedBy: r.suspended_by,
        suspendedAt: r.suspended_at,
        turn: st?.turn.n ?? 0,
        format: st?.settings.format ?? '',
        bo: st?.settings.bo ?? 1,
        game: st?.match.game ?? 1,
        wins: st?.match.wins ?? [],
        players: st?.seats.map((s) => ({ userId: s.userId, name: s.name, color: s.color })) ?? [],
      };
    });
  }

  deleteSuspended(user: HubUser, id: string) {
    const g = this.suspendedGames().find((x) => x.id === id);
    if (!g) throw new LobbyError('Not found');
    if (!user.isAdmin && !g.players.some((p) => p.userId === user.id)) throw new LobbyError('Only players of that game can delete it');
    run(this.app.db, 'DELETE FROM games WHERE id = ?', id);
  }

  /** The room of a running sandbox game (tests and the REST API). */
  room(id: string): GameRoom | undefined {
    return this.rooms.get(id);
  }
}
