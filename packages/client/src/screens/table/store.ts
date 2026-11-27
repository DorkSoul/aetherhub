import type { MenuItem } from '../../ui/ui';
import { create } from 'zustand';
import type { ArrowMsg, CardDef, CardInst, GameView, Intent, LogView, Notice, SideboardState, UndoRequestMsg } from '@aetherhub/shared';
import { applyPatch } from '@aetherhub/shared';

export interface CardsNotice {
  notice: Extract<Notice, { kind: 'cards' }>;
  cards: Record<string, CardInst>;
  defs: Record<string, CardDef>;
}

export interface PanZoom {
  x: number;
  y: number;
  k: number;
}

/** Rules games: an arrow for who attacks what / who blocks whom (from the engine's combat, or the choices being made right now). */
export interface CombatArrow {
  id: string;
  kind: 'attack' | 'block';
  from: { iid?: string; seat?: number };
  to: { iid?: string; seat?: number };
  /** still being chosen (not declared yet) */
  draft?: boolean;
  /** a proposed block the rules would drop (menace, "can't block" ...): drawn grey */
  illegal?: boolean;
}

export type TableModal =
  | { kind: 'zone'; seat: number; zone: 'library' | 'graveyard' | 'exile' | 'command' | 'sideboard' | 'schemes' | 'stickers' }
  | { kind: 'cards'; data: CardsNotice }
  | { kind: 'tokens'; seat: number }
  | { kind: 'commander'; iid: string }
  | { kind: 'end' }
  | { kind: 'giveSeat'; seat: number }
  | { kind: 'specials' }
  | { kind: 'attach'; iid: string; mode: 'attach' | 'meld' }
  | { kind: 'counter'; iids: string[] }
  | { kind: 'nth'; iid: string }
  | { kind: 'x'; title: string; run: (n: number) => void };

/** A card to show in the details window: `iid` follows a card on the table, `def` is used as is (cards in a look or reveal). */
export type CardDetailsTarget = { iid: string; def?: undefined } | { iid?: string; def: CardDef };

export interface TableState {
  mode: 'online' | 'local';
  view: GameView | null;
  log: LogView[];
  seq: number;
  host: boolean;
  hostSeat: number | null;
  undoable: number;
  closed: string | null;
  openedElsewhere: boolean;
  arrows: (ArrowMsg & { at: number })[];
  combatArrows: CombatArrow[];
  liveArrows: CombatArrow[];
  pings: Record<string, number>;
  modal: TableModal | null;
  reveal: CardsNotice | null;
  /** Card details opened by this player only; kept apart from `modal` so it opens on top of a zone or a pending choice. */
  details: CardDetailsTarget | null;
  undoRequests: UndoRequestMsg[];
  sideboard: SideboardState | null;
  sel: string[];
  hover: string | null;
  lastHover: string | null;
  pz: Record<number, PanZoom>;
  tabOpp: number | null;
  logOpen: boolean;
  logFilter: 'all' | 'act' | 'chat';
  panelTab: 'log' | 'players' | 'rules' | 'game';
  /** Rules games in the detailed view: where the game's prompts are drawn (the side panel's Game tab) */
  rulesSlot: HTMLElement | null;
  mullBottom: string[] | null;
  /** Set by the driver: sends an intent (online: over the socket; local: applies it). */
  send: (i: Intent) => void;
  /** Rules game: while set, clicking one of these cards calls pick(iid) instead of selecting it (targets, attackers, blockers). */
  pick: ((iid: string) => void) | null;
  pickIds: string[];
  /** Rules game: called when a battlefield card that can't be picked is clicked (explains why). */
  pickDenied: ((iid: string) => void) | null;
  /** Rules game: while set, clicking one of these players (their battlefield, a card on it, or their side panel) calls pickSeat(seat): who a chosen attacker attacks. */
  pickSeat: ((seat: number) => void) | null;
  pickSeats: number[];
  /** Rules game: while paying for a spell, the pool colors that can pay and how to spend one. */
  payPool: { usable: string[]; pay: (color: string) => void } | null;
  rules: RulesHooks | null;
  /** Local playtest only. */
  restart: (() => void) | null;
  /** Local solo game: which seat you are acting as, and view options. Re-render the view with republish(). */
  solo: SoloOpts | null;
  /** Online game: every seat this player holds (several in a solo game played on the server). */
  mySeats: number[];
  republish: (() => void) | null;
  /** Local solo game saving: where it is saved (id null = never saved) and how to save. */
  soloSave: SoloSaveState | null;
  applyPatch: (ops: Parameters<typeof applyPatch>[1]) => void;
}

/** What a rules-engine game (screens/rules/RulesGame.tsx) adds to the shared table. */
export interface RulesHooks {
  undo: () => void;
  save: () => void;
  canUndo: boolean;
  endTurn: () => void;
  /** stop passing priority automatically */
  cancelPass: () => void;
  /** "break the rules": the game is paused for a fix by hand (by: who, when it isn't you) */
  breakRules: { active: boolean; mine: boolean; by: string | null };
  toggleBreak: () => void;
  /** while breaking the rules: open the token picker for a seat's board */
  openTokens: (seat: number) => void;
  /** answers given automatically for the rest of this game (right-click a stack item, "Remember my answer") */
  reactions: { id: number; label: string; forget: () => void }[];
  ending: boolean;
  /** pass priority until the given phase (index into PHASES) */
  passTo: (phase: number) => void;
  /** re-lay one battlefield (that seat's permanents) into the three automatic rows, piling up identical cards */
  sortBoard: (seat: number) => void;
  /** concede (asks first) */
  concede: () => void;
  /** end the game for this player (asks first): removes it against the AI, concedes and leaves with other players in it */
  endGame: () => void;
  /** card / hand size for any seat (only how this screen draws it) */
  layout: (seat: number, patch: { handFrac?: number; cardScale?: number }) => void;
  /** phase being passed to, if any */
  endPhase: number | null;
  /** the card's activated abilities that can be used right now */
  abilities: (iid: string) => { label: string; run: () => void; sub?: () => MenuItem[] }[];
  /** pass priority until that seat's next turn starts */
  passToPlayer?: (seat: number) => void;
  /** what the game is doing while you wait ("AI 2 is thinking…", "Sending…", "2 queued"), or null */
  status?: string | null;
  /** Esc: stop passing priority, drop queued actions */
  escape?: () => void;
  /** something on the side panel's Game tab needs a look (a question, the stack) while another tab is showing */
  attention?: boolean;
}

export interface SoloOpts {
  seat: number;
  count: number;
  follow: boolean;
  revealAll: boolean;
}

export interface SoloSaveState {
  id: number | null;
  name: string;
  savedAt: number | null;
  saving: boolean;
  /** Changes since the last save. */
  dirty: boolean;
  save: (opts?: { asNew?: boolean; name?: string }) => Promise<boolean>;
}

export const useTable = create<TableState>((set, get) => ({
  mode: 'online',
  view: null,
  log: [],
  seq: 0,
  host: false,
  hostSeat: null,
  undoable: 0,
  closed: null,
  openedElsewhere: false,
  arrows: [],
  combatArrows: [],
  liveArrows: [],
  pings: {},
  modal: null,
  reveal: null,
  details: null,
  undoRequests: [],
  sideboard: null,
  sel: [],
  hover: null,
  lastHover: null,
  pz: {},
  tabOpp: null,
  logOpen: false,
  logFilter: 'all',
  panelTab: 'log',
  rulesSlot: null,
  mullBottom: null,
  send: () => {},
  pick: null,
  pickIds: [],
  pickDenied: null,
  pickSeat: null,
  pickSeats: [],
  payPool: null,
  rules: null,
  restart: null,
  solo: null,
  mySeats: [],
  republish: null,
  soloSave: null,
  applyPatch: (ops) => {
    const v = get().view;
    if (!v) return;
    set({ view: applyPatch(v, ops) });
  },
}));

export function resetTable() {
  useTable.setState({
    view: null,
    log: [],
    seq: 0,
    closed: null,
    openedElsewhere: false,
    arrows: [],
    combatArrows: [],
    liveArrows: [],
    pings: {},
    modal: null,
    reveal: null,
    details: null,
    undoRequests: [],
    sideboard: null,
    sel: [],
    hover: null,
    lastHover: null,
    pz: {},
    mullBottom: null,
  });
}

/** Viewer's own seat, or null for spectators. */
export function mySeat(v: GameView | null): number | null {
  return v && typeof v.viewer === 'number' ? v.viewer : null;
}
