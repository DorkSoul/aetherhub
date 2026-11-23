// REST and WebSocket message shapes shared by server and client.

import type { CardDef } from './cards';
import type { PatchOp } from './diff';
import type { Intent } from './game/intents';
import type { GameSettings, Notice } from './game/types';
import type { GameView, LogView } from './game/view';
import type { UserPrefs } from './prefs';

export interface Me {
  id: number;
  username: string;
  isAdmin: boolean;
  prefs: UserPrefs;
}

export interface SetupState {
  needsAdmin: boolean;
  serverName: string;
  secure: boolean;
  lan: boolean;
  insecureRemote: boolean;
}

export interface LobbySummary {
  id: string;
  name: string;
  hostName: string;
  type: GameType;
  /** draft / sealed: rules enforced (see rulesMode) */
  enforced?: boolean;
  /** filled seats (humans and AIs) */
  seats: number;
  maxPlayers: number;
  format: string;
  bo: 1 | 3;
  hasPassword: boolean;
  inLobby: boolean;
  /** draft / sealed event: the pool is being drafted, or the decks are built and games are being played */
  phase: LobbyPhase;
}

export interface GameSummary {
  id: string;
  name: string;
  turn: number;
  hasSeat: boolean;
  spectators: boolean;
  status: 'active';
  ended: boolean;
}

export interface ServerStatus {
  serverName: string;
  cardDataUpdatedAt: number | null;
  cardCount: number;
  importing: boolean;
  /** every open lobby on the server */
  lobbies: LobbySummary[];
  /** every running sandbox game (rules games are listed by GET api/rules/mine) */
  games: GameSummary[];
}

/** A game the server is running, as the admin Server page lists it. */
export interface AdminGame {
  id: string;
  kind: 'sandbox' | 'rules';
  name: string;
  /** the players; an AI seat is marked "(AI)", a player who is not connected "(away)" */
  players: string[];
  turn: number;
  /** when someone last did something in it (ms), if known */
  lastActive: number | null;
  /** waiting for players, running, over, or restored after a restart and not opened since */
  state: 'waiting' | 'running' | 'over' | 'sleeping';
}

/** A line of the server's log (kept in memory: the most recent ones). */
export interface AdminLogLine {
  seq: number;
  at: number;
  level: string;
  msg: string;
}

/** sandbox: no rules enforced; rules: the rules engine plays the game (AI seats allowed); draft / sealed: a limited event */
export type GameType = 'sandbox' | 'rules' | 'draft' | 'sealed';
export const GAME_TYPES: { id: GameType; label: string; hint: string }[] = [
  { id: 'sandbox', label: 'Sandbox game', hint: 'No rules enforced: the players do what they like, as at a real table.' },
  { id: 'rules', label: 'Rules enforced game', hint: 'The rules engine plays the game by the real rules. AI players can take seats.' },
  { id: 'draft', label: 'Draft', hint: 'Booster draft: open packs and pass them round the table, build a 40-card deck, then play.' },
  { id: 'sealed', label: 'Sealed', hint: 'Each player opens six packs and builds a 40-card deck from them, then play.' },
];
/**
 * The lobby shows the game type as two choices: what is played (constructed, draft, sealed) and how (sandbox or rules
 * enforced). A constructed game stores both in `type` ('sandbox' / 'rules'); a draft or sealed event stores the second
 * in `enforced` (missing = rules enforced, as events were before the choice existed).
 */
export type GameKind = 'constructed' | 'draft' | 'sealed';
export type RulesMode = 'sandbox' | 'rules';
export const GAME_KINDS: { id: GameKind; label: string; hint: string }[] = [
  { id: 'constructed', label: 'Constructed', hint: 'Everyone brings a deck from their collection.' },
  { id: 'draft', label: 'Draft', hint: 'Booster draft: open packs and pass them round the table, build a 40-card deck, then play.' },
  { id: 'sealed', label: 'Sealed', hint: 'Each player opens six packs and builds a 40-card deck from them, then play.' },
];
export const RULES_MODES: { id: RulesMode; label: string; hint: string }[] = [
  { id: 'sandbox', label: 'Sandbox', hint: 'No rules enforced: the players do what they like, as at a real table. No AI players.' },
  { id: 'rules', label: 'Rules enforced', hint: 'The rules engine plays the game by the real rules. AI players can take seats.' },
];
export function gameKind(type: GameType | undefined): GameKind {
  return type === 'draft' || type === 'sealed' ? type : 'constructed';
}
export function rulesMode(s: { type?: GameType; enforced?: boolean }): RulesMode {
  const t = s.type ?? 'sandbox';
  if (t === 'draft' || t === 'sealed') return s.enforced === false ? 'sandbox' : 'rules';
  return t === 'rules' ? 'rules' : 'sandbox';
}
/** The settings patch for a game kind and rules mode. */
export function typePatch(kind: GameKind, mode: RulesMode): { type: GameType; enforced?: boolean } {
  return kind === 'constructed' ? { type: mode } : { type: kind, enforced: mode === 'rules' };
}
/** "Draft · sandbox", "Rules enforced game", ... for headers and lobby lists. */
export function gameTypeLabel(s: { type?: GameType; enforced?: boolean }): string {
  const kind = gameKind(s.type);
  const mode = rulesMode(s);
  if (kind === 'constructed') return GAME_TYPES.find((t) => t.id === mode)!.label;
  return `${GAME_KINDS.find((k) => k.id === kind)!.label} · ${mode === 'rules' ? 'rules enforced' : 'sandbox'}`;
}
export type AiLevel = 'Easy' | 'Normal' | 'Hard';
/** setup: choosing seats and decks; drafting / building: a draft or sealed event before its games; event: decks built, games are started from the lobby */
export type LobbyPhase = 'setup' | 'drafting' | 'building' | 'event';

/** Someone in the lobby (seated or not). */
export interface LobbyMember {
  userId: number;
  name: string;
  color: string;
  ready: boolean;
  connected: boolean;
}

export type LobbySlot =
  | { kind: 'open' }
  | { kind: 'human'; userId: number; name: string; color: string; deckId: number | null; deckName: string | null; deckOwner: string | null }
  | { kind: 'ai'; name: string; difficulty: AiLevel; deckId: number | null; deckName: string | null; deckOwner: string | null; addedBy: number };

export interface LobbyChat {
  who: string;
  color: string;
  text: string;
  system?: boolean;
}

/** A drafter's finished deck in a draft / sealed event lobby. */
export interface EventDeck {
  /** seat in the pod */
  seat: number;
  name: string;
  /** null = an AI drafter */
  userId: number | null;
  deckId: number | null;
  deckName: string | null;
}

export interface LobbyView {
  id: string;
  hostId: number;
  settings: GameSettings;
  hasPassword: boolean;
  members: LobbyMember[];
  /** one entry per seat, `settings.maxPlayers` long */
  slots: LobbySlot[];
  chat: LobbyChat[];
  phase: LobbyPhase;
  /** draft / sealed: the draft session (drafting and building) */
  draftId: string | null;
  /** draft / sealed event: every drafter's deck, once built */
  eventDecks: EventDeck[];
  /** games started from this event lobby that are still running */
  eventGames: { id: string; kind: 'sandbox' | 'rules'; names: string[] }[];
  /** bumped on every change to the draft (the draft screen fetches its state again) */
  draftRev: number;
}

export interface ArrowMsg {
  id: string;
  seat: number;
  color: string;
  from: { iid?: string; seat?: number };
  to: { iid?: string; seat?: number };
}

export interface UndoRequestMsg {
  id: string;
  from: number;
  fromName: string;
  label: string;
}

export interface SideboardState {
  seat: number;
  main: { def: string; qty: number }[];
  side: { def: string; qty: number }[];
  defs: Record<string, CardDef>;
  ready: boolean[];
  chooser: number | null;
  names: string[];
  wins: number[];
  game: number;
}

export type ClientMsg =
  | { t: 'hello' }
  | { t: 'takeBack' }
  | { t: 'intent'; id: number; intent: Intent }
  | { t: 'undo' }
  | { t: 'undoAnswer'; id: string; approve: boolean }
  | { t: 'arrow'; from: ArrowMsg['from']; to: ArrowMsg['to'] }
  | { t: 'arrowRemove'; id: string }
  | { t: 'ping'; iid: string }
  | { t: 'watch'; game: string }
  /** a player holding several seats of a sandbox game chooses the one they act as */
  | { t: 'seat'; seat: number }
  | { t: 'unwatch' }
  | { t: 'host'; op: 'end'; winner: number | null }
  | { t: 'host'; op: 'suspend' }
  | { t: 'host'; op: 'removeSeat'; seat: number }
  | { t: 'host'; op: 'giveSeat'; seat: number; userId: number }
  | { t: 'host'; op: 'rematch' }
  | { t: 'host'; op: 'close' }
  | { t: 'sideboard'; op: 'swap'; def: string; to: 'main' | 'side' }
  | { t: 'sideboard'; op: 'ready'; value: boolean }
  | { t: 'sideboard'; op: 'choose'; starter: number }
  | { t: 'specChat'; text: string };

export type ServerMsg =
  | { t: 'welcome'; userId: number; role: 'player' | 'spectator' | 'none'; seat: number | null; host: boolean }
  | { t: 'status'; status: ServerStatus }
  | { t: 'lobby'; lobby: LobbyView | null }
  /** the lobby this player is in started a game: go to it */
  | { t: 'started'; kind: 'sandbox' | 'rules' | 'draft'; id: string }
  | { t: 'game'; view: GameView; log: LogView[]; seq: number; host: boolean; hostSeat: number | null; undoable: number; /** the seats this player holds (several in a solo game) */ mySeats: number[] }
  | { t: 'patch'; ops: PatchOp[]; seq: number; undoable: number; host: boolean; hostSeat: number | null }
  | { t: 'log'; entries: LogView[] }
  | { t: 'logUpdate'; entry: LogView }
  | { t: 'ack'; id: number }
  | { t: 'reject'; id: number; reason: string }
  | { t: 'notice'; notice: Notice; defs?: Record<string, CardDef>; cards?: Record<string, import('./game/types').CardInst> }
  | { t: 'arrow'; arrow: ArrowMsg }
  | { t: 'arrowRemove'; id: string }
  | { t: 'ping'; iid: string; seat: number; name: string }
  | { t: 'openedElsewhere' }
  | { t: 'undoRequest'; req: UndoRequestMsg }
  | { t: 'undoResolved'; id: string; approved: boolean }
  | { t: 'gameClosed'; reason: string }
  | { t: 'sideboard'; state: SideboardState | null }
  | { t: 'error'; message: string };
