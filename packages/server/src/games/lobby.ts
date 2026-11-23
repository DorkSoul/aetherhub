import crypto from 'node:crypto';
import type { AiLevel, EventDeck, GameSettings, LobbyChat, LobbyMember, LobbyPhase, LobbySlot, LobbySummary, LobbyView } from '@aetherhub/shared';
import { DEFAULT_SETTINGS, formatInfo, rulesMode } from '@aetherhub/shared';

export function cryptoShuffle<T>(arr: T[]): T[] {
  const a = arr.slice();
  for (let i = a.length - 1; i > 0; i--) {
    const j = crypto.randomInt(i + 1);
    [a[i], a[j]] = [a[j], a[i]];
  }
  return a;
}

export class LobbyError extends Error {}

export interface Actor {
  id: number;
  username: string;
  isAdmin: boolean;
}

export interface DeckRef {
  id: number;
  name: string;
  owner: string;
}

const AI_NAMES = ['Ava', 'Bryn', 'Cora', 'Dex', 'Edda', 'Finn', 'Gale', 'Hux'];

/**
 * A game lobby: who is in it (members), the numbered seats (each open, a human or an AI, with its own deck) and the
 * settings. Anyone in the lobby can take an open seat and pick its deck; the host (whoever created the lobby, or the
 * next member when they leave) and app admins can change settings, seat anyone and change any seat's deck. A user can
 * hold several seats (a solo game). Draft and sealed lobbies stay open after the draft as the event's home.
 */
export class Lobby {
  id = crypto.randomUUID();
  settings: GameSettings;
  password: string | null;
  members: LobbyMember[] = [];
  slots: LobbySlot[] = [];
  chat: LobbyChat[] = [];
  phase: LobbyPhase = 'setup';
  draftId: string | null = null;
  eventDecks: EventDeck[] = [];
  eventGames: LobbyView['eventGames'] = [];
  draftRev = 0;
  createdAt = Date.now();

  constructor(
    public hostId: number,
    host: { name: string; color: string },
    settings: Partial<GameSettings>,
    password: string | null,
  ) {
    const format = settings.format || DEFAULT_SETTINGS.format;
    this.settings = { ...DEFAULT_SETTINGS, life: formatInfo(format).life, ...settings, format };
    this.settings.maxPlayers = Math.min(8, Math.max(2, this.settings.maxPlayers));
    this.password = password || null;
    this.slots = Array.from({ length: this.settings.maxPlayers }, () => ({ kind: 'open' }) as LobbySlot);
    this.members.push({ userId: hostId, name: host.name, color: host.color, ready: false, connected: true });
    this.slots[0] = this.humanSlot(hostId);
  }

  get type() {
    return this.settings.type ?? 'sandbox';
  }

  /** Rules enforced (AI seats allowed): a rules game, or a draft / sealed event that is not sandbox. */
  get enforced() {
    return rulesMode(this.settings) === 'rules';
  }

  view(): LobbyView {
    return {
      id: this.id,
      hostId: this.hostId,
      settings: this.settings,
      hasPassword: !!this.password,
      members: this.members,
      slots: this.slots,
      chat: this.chat.slice(-200),
      phase: this.phase,
      draftId: this.draftId,
      eventDecks: this.eventDecks,
      eventGames: this.eventGames,
      draftRev: this.draftRev,
    };
  }

  /** Everything needed to bring a draft / sealed event lobby back after a restart. */
  toJSON() {
    return { ...this.view(), password: this.password, chat: this.chat.slice(-200), createdAt: this.createdAt };
  }

  static fromJSON(j: ReturnType<Lobby['toJSON']>): Lobby {
    const l = Object.create(Lobby.prototype) as Lobby;
    Object.assign(l, {
      id: j.id,
      hostId: j.hostId,
      settings: j.settings,
      password: j.password,
      members: j.members.map((m) => ({ ...m, connected: false })),
      slots: j.slots,
      chat: j.chat,
      phase: j.phase,
      draftId: j.draftId,
      eventDecks: j.eventDecks,
      eventGames: j.eventGames ?? [],
      draftRev: j.draftRev ?? 0,
      createdAt: j.createdAt,
    });
    return l;
  }

  summary(userId: number): LobbySummary {
    return {
      id: this.id,
      name: this.settings.name,
      hostName: this.member(this.hostId)?.name ?? '?',
      type: this.type,
      enforced: this.settings.enforced,
      seats: this.slots.filter((s) => s.kind !== 'open').length,
      maxPlayers: this.settings.maxPlayers,
      format: this.settings.format,
      bo: this.settings.bo,
      hasPassword: !!this.password,
      inLobby: !!this.member(userId),
      phase: this.phase,
    };
  }

  member(userId: number): LobbyMember | undefined {
    return this.members.find((m) => m.userId === userId);
  }

  isHost(a: Actor): boolean {
    return a.id === this.hostId || a.isAdmin;
  }

  private requireHost(a: Actor, what: string) {
    if (!this.isHost(a)) throw new LobbyError(`Only the host can ${what}`);
  }

  private requireMember(a: Actor): LobbyMember {
    const m = this.member(a.id);
    if (!m && !a.isAdmin) throw new LobbyError('You are not in the lobby');
    return m!;
  }

  private requireSetup() {
    if (this.phase !== 'setup') throw new LobbyError('The seats are fixed once the draft has started');
  }

  system(text: string) {
    this.chat.push({ who: '·', color: '#7d818b', text, system: true });
  }

  /** Back from a game: put this member in the seats they had (`before`, with their decks) where those are free;
   *  anywhere they were seated meanwhile is emptied first. */
  restoreSeats(userId: number, before: LobbySlot[]) {
    const mine = before.flatMap((s, i) => (s.kind === 'human' && s.userId === userId ? [[i, s] as const] : []));
    if (!mine.length) return;
    const now = this.seatsOf(userId);
    if (mine.some(([i]) => this.slots[i]?.kind !== 'open' && !now.includes(i))) return;
    for (const i of now) this.slots[i] = { kind: 'open' };
    for (const [i, s] of mine) {
      const m = this.member(userId)!;
      this.slots[i] = { kind: 'human', userId, name: m.name, color: m.color, deckId: s.deckId, deckName: s.deckName, deckOwner: s.deckOwner };
    }
  }

  private humanSlot(userId: number): LobbySlot {
    const m = this.member(userId)!;
    return { kind: 'human', userId, name: m.name, color: m.color, deckId: null, deckName: null, deckOwner: null };
  }

  private slotAt(i: number): LobbySlot {
    const s = this.slots[i];
    if (!s) throw new LobbyError('No such seat');
    return s;
  }

  /** Seats a member holds. */
  seatsOf(userId: number): number[] {
    return this.slots.flatMap((s, i) => (s.kind === 'human' && s.userId === userId ? [i] : []));
  }

  /** A member's readiness is lost whenever one of their seats (or the game type) changes. */
  private unready(userId: number | null) {
    for (const m of this.members) if (userId == null || m.userId === userId) m.ready = false;
  }

  join(user: Actor, color: string, password: string | null) {
    if (this.member(user.id)) return;
    if (this.password && password !== this.password && !user.isAdmin) throw new LobbyError('Wrong lobby password');
    this.members.push({ userId: user.id, name: user.username, color, ready: false, connected: true });
    // a newcomer sits in the first open seat, if there is one (they can move or stand up)
    const open = this.phase === 'setup' ? this.slots.findIndex((s) => s.kind === 'open') : -1;
    if (open >= 0) this.slots[open] = this.humanSlot(user.id);
    this.system(`${user.username} joined`);
  }

  leave(userId: number) {
    const m = this.member(userId);
    if (!m) return;
    this.members = this.members.filter((x) => x.userId !== userId);
    if (this.phase === 'setup') this.slots = this.slots.map((s) => (s.kind === 'human' && s.userId === userId ? { kind: 'open' } : s));
    this.system(`${m.name} left`);
    if (userId === this.hostId && this.members.length) {
      this.hostId = this.members[0].userId;
      this.system(`${this.members[0].name} is now the host`);
    }
  }

  kick(a: Actor, userId: number) {
    this.requireHost(a, 'remove players');
    const m = this.member(userId);
    if (!m) return;
    this.leave(userId);
    this.chat.pop();
    this.system(`${m.name} was removed by the host`);
  }

  /**
   * Put someone in seat `i`: yourself (any member, into an open seat), or, as the host, any member or an AI into any
   * seat. Taking a seat someone else just took fails, so two people can choose seats at the same time.
   */
  sit(a: Actor, i: number, who: { userId: number } | { ai: AiLevel }) {
    this.requireMember(a);
    this.requireSetup();
    const cur = this.slotAt(i);
    const host = this.isHost(a);
    if ('ai' in who) {
      if (!this.enforced) throw new LobbyError('AI players need a rules enforced game');
      if (cur.kind !== 'open' && !host) throw new LobbyError('That seat was just taken');
      const used = new Set(this.slots.flatMap((s) => (s.kind === 'ai' ? [s.name] : [])));
      const name = AI_NAMES.find((n) => !used.has(`AI ${n}`)) ?? String(i + 1);
      this.clearSlot(i);
      this.slots[i] = { kind: 'ai', name: `AI ${name}`, difficulty: who.ai, deckId: null, deckName: null, deckOwner: null, addedBy: a.id };
      this.system(`${a.username} added ${this.slots[i].kind === 'ai' ? (this.slots[i] as { name: string }).name : 'an AI'} (${who.ai}) to seat ${i + 1}`);
      return;
    }
    const target = this.member(who.userId);
    if (!target) throw new LobbyError('That player is not in the lobby');
    if (who.userId !== a.id && !host) throw new LobbyError('Only the host can seat other players');
    if (cur.kind === 'human' && cur.userId === who.userId) return;
    if (cur.kind !== 'open' && !host) throw new LobbyError('That seat was just taken');
    this.clearSlot(i);
    this.slots[i] = this.humanSlot(who.userId);
    this.unready(who.userId);
    if (who.userId !== a.id) this.system(`${a.username} seated ${target.name} in seat ${i + 1}`);
  }

  private clearSlot(i: number) {
    const cur = this.slots[i];
    if (cur?.kind === 'human') this.unready(cur.userId);
    this.slots[i] = { kind: 'open' };
  }

  /** Empty seat `i`: your own seat, or (as the host) any seat. */
  stand(a: Actor, i: number) {
    this.requireMember(a);
    this.requireSetup();
    const cur = this.slotAt(i);
    if (cur.kind === 'open') return;
    const mine = (cur.kind === 'human' && cur.userId === a.id) || (cur.kind === 'ai' && cur.addedBy === a.id);
    if (!mine && !this.isHost(a)) throw new LobbyError('Only the host can empty other players’ seats');
    if (!mine) this.system(`${a.username} emptied seat ${i + 1} (${cur.name})`);
    this.clearSlot(i);
  }

  setDifficulty(a: Actor, i: number, difficulty: AiLevel) {
    const cur = this.slotAt(i);
    if (cur.kind !== 'ai') throw new LobbyError('That seat is not an AI');
    if (cur.addedBy !== a.id && !this.isHost(a)) throw new LobbyError('Only the host can change that AI');
    cur.difficulty = difficulty;
  }

  /** The deck for seat `i`: your own seat, an AI you added, or (as the host) any seat. */
  setDeck(a: Actor, i: number, deck: DeckRef | null) {
    this.requireMember(a);
    const cur = this.slotAt(i);
    if (cur.kind === 'open') throw new LobbyError('Nobody sits there');
    const mine = (cur.kind === 'human' && cur.userId === a.id) || (cur.kind === 'ai' && cur.addedBy === a.id);
    if (!mine && !this.isHost(a)) throw new LobbyError('Only the host can choose decks for other seats');
    cur.deckId = deck?.id ?? null;
    cur.deckName = deck?.name ?? null;
    cur.deckOwner = deck?.owner ?? null;
    if (cur.kind === 'human') {
      this.unready(cur.userId);
      if (!mine) this.system(`${a.username} chose ${deck?.name ?? 'no deck'} for ${cur.name}`);
    }
  }

  /** Decks are not chosen in the lobby for a draft or sealed event (they are built from the pool). */
  needsDecks() {
    return this.type === 'sandbox' || this.type === 'rules';
  }

  setReady(userId: number, ready: boolean) {
    const m = this.member(userId);
    if (!m) throw new LobbyError('You are not in the lobby');
    if (ready && this.needsDecks()) {
      const missing = this.seatsOf(userId).filter((i) => (this.slots[i] as { deckId: number | null }).deckId == null);
      if (missing.length) throw new LobbyError(`Pick a deck for seat ${missing.map((i) => i + 1).join(', ')} first`);
    }
    m.ready = ready;
  }

  /**
   * Settings (host only). Leaving the rules enforced type while AIs are seated is refused unless `removeAi` (the client
   * asks first); fewer seats than the last filled one is refused.
   */
  updateSettings(a: Actor, patch: Partial<GameSettings> & { password?: string | null; removeAi?: boolean }) {
    this.requireHost(a, 'change settings');
    const { password, removeAi, ...rest } = patch;
    const typeChange = (rest.type !== undefined && rest.type !== this.type) || (rest.enforced !== undefined && rest.enforced !== (this.settings.enforced ?? true));
    if (typeChange) {
      this.requireSetup();
      const ais = this.slots.some((s) => s.kind === 'ai');
      if (ais && rulesMode({ ...this.settings, ...rest }) === 'sandbox') {
        if (!removeAi) throw new LobbyError('AI players need a rules enforced game');
        this.slots = this.slots.map((s) => (s.kind === 'ai' ? { kind: 'open' } : s));
      }
      this.unready(null);
    }
    if (rest.format && rest.format !== this.settings.format && rest.life == null) rest.life = formatInfo(rest.format).life;
    if (rest.maxPlayers != null) {
      const want = Math.min(8, Math.max(2, rest.maxPlayers));
      const lastFilled = this.slots.reduce((m, s, i) => (s.kind !== 'open' ? i : m), -1);
      if (want <= lastFilled) throw new LobbyError('Empty the last seats before removing them');
      if (this.phase !== 'setup' && want !== this.settings.maxPlayers) throw new LobbyError('The seats are fixed once the draft has started');
      while (this.slots.length < want) this.slots.push({ kind: 'open' });
      this.slots.length = want;
      rest.maxPlayers = want;
    }
    this.settings = { ...this.settings, ...rest };
    if (password !== undefined) this.password = password || null;
  }

  /** Swap seat `i` with the one `d` places away (host). */
  move(a: Actor, i: number, d: number) {
    this.requireHost(a, 'reorder seats');
    this.requireSetup();
    const j = i + d;
    if (!this.slots[i] || !this.slots[j]) return;
    [this.slots[i], this.slots[j]] = [this.slots[j], this.slots[i]];
  }

  /** Random turn order: the filled seats in a random order, at the front. */
  reroll(a: Actor) {
    this.requireHost(a, 're-roll the order');
    this.requireSetup();
    const filled = cryptoShuffle(this.slots.filter((s) => s.kind !== 'open'));
    this.slots = [...filled, ...this.slots.filter((s) => s.kind === 'open')];
    this.system('Host re-rolled the turn order');
  }

  /** What keeps the game from starting (empty when it can start). */
  blockers(): string[] {
    const out: string[] = [];
    const filled = this.slots.filter((s) => s.kind !== 'open');
    if (filled.length < 2) out.push('at least two seats filled');
    if (!this.enforced && filled.some((s) => s.kind === 'ai')) out.push('a rules enforced game for the AI players');
    if (this.needsDecks()) {
      const noDeck = this.slots.flatMap((s, i) => (s.kind === 'human' && s.deckId == null ? [i + 1] : []));
      if (noDeck.length) out.push(`a deck for seat ${noDeck.join(', ')}`);
    }
    const seated = new Set(this.slots.flatMap((s) => (s.kind === 'human' ? [s.userId] : [])));
    const waiting = this.members.filter((m) => seated.has(m.userId) && !m.ready).map((m) => m.name);
    if (waiting.length) out.push(`${waiting.join(', ')} to be ready`);
    return out;
  }
}
