import type { CardDef } from '../cards';

export const SEAT_ZONES = [
  'library',
  'hand',
  'battlefield',
  'graveyard',
  'exile',
  'command',
  'sideboard',
  'schemeDeck',
  'schemes',
  'attractionDeck',
  'stickers',
] as const;
export type ZoneName = (typeof SEAT_ZONES)[number];

export const ZONE_LABEL: Record<ZoneName, string> = {
  library: 'library',
  hand: 'hand',
  battlefield: 'the battlefield',
  graveyard: 'graveyard',
  exile: 'exile',
  command: 'the command zone',
  sideboard: 'sideboard',
  schemeDeck: 'scheme deck',
  schemes: 'schemes in play',
  attractionDeck: 'attraction deck',
  stickers: 'sticker sheets',
};

/** Zones whose card identities are hidden from other players by default. */
export const HIDDEN_ZONES: ZoneName[] = ['library', 'hand', 'sideboard', 'schemeDeck', 'attractionDeck'];

export const PHASES = ['Untap', 'Upkeep', 'Draw', 'Main', 'Combat', 'Main 2', 'End'];

export type MulliganRule = 'none' | 'london' | 'london-free' | 'free' | 'free-scry3' | 'vancouver' | 'paris';

export const MULLIGAN_RULES: { id: MulliganRule; label: string; hint: string }[] = [
  { id: 'none', label: 'No mulligan phase', hint: 'Everyone draws 7 and play starts. Players can still shuffle and redraw by hand.' },
  { id: 'london', label: 'London', hint: 'Draw 7 each time, then put one card on the bottom per mulligan taken. Current official rule.' },
  {
    id: 'london-free',
    label: 'London, free first mulligan',
    hint: 'As London, but the first mulligan is free (no card to the bottom). Official multiplayer / Commander rule.',
  },
  { id: 'free', label: 'Free mulligans', hint: 'Mulligan as many times as you like: shuffle your hand back and draw a fresh 7, never putting cards on the bottom. Casual / testing rule.' },
  {
    id: 'free-scry3',
    label: 'Free mulligans + scry 3',
    hint: 'Mulligan as many times as you like, scrying the top 3 cards of your library while you decide (any to the bottom, the rest in any order). Keep; or put your hand and the scried cards still on top on the bottom and draw a fresh 7.',
  },
  { id: 'vancouver', label: 'Vancouver', hint: 'Draw one fewer card each time; if you keep with fewer than 7, scry 1. The official rule before London.' },
  { id: 'paris', label: 'Paris', hint: 'Draw one fewer card each time, no scry. The official rule before Vancouver.' },
];

export interface CardInst {
  iid: string;
  def: string;
  owner: number;
  faceDown: boolean;
  tapped: boolean;
  /** Normalised 0..1 position inside the battlefield's free space. */
  x: number;
  y: number;
  /** Transformed face index for double-faced cards. */
  face: number;
  /** Kamigawa flip cards: rotated 180°. */
  flipped: boolean;
  counters: Record<string, number>;
  effects: { k: string; eot: boolean }[];
  pt: { p: number; t: number };
  ptEot: { p: number; t: number };
  pile: number;
  noUntap: boolean;
  token: boolean;
  attachedTo: string | null;
  melded: string | null;
  commander: boolean;
  companion: boolean;
  /** Seats that may see this face-down card. */
  peekers: number[];
  /** A Room's locked doors (1 = left, 2 = right); absent when none is locked or the card isn't a Room. */
  locked?: number[];
}

export interface Seat {
  userId: number;
  name: string;
  color: string;
  deckId: number | null;
  deckName: string;
  sleeve: string | null;
  playmat: string | null;
  life: number;
  lifeHist: number[];
  counters: Record<string, number>; // poison, energy, experience, rad, tickets, ...
  cmdTax: Record<string, number>; // commander iid -> tax
  cmdDmg: Record<string, number>; // source commander iid -> damage taken
  zones: Record<ZoneName, string[]>;
  handFrac: number;
  cardScale: number;
  connected: boolean;
  conceded: boolean;
  revealTop: boolean;
  handViewers: number[];
  handRevealAll: boolean;
  autoUntap: boolean;
  /** Draw a card when your turn starts. Missing in older saves = off. */
  autoDraw?: boolean;
  dungeon: { name: string; room: number } | null;
  ring: number;
  citysBlessing: boolean;
  /** `scryLeft`: free mulligans + scry 3, cards of the scry still on top (the rest were put on the bottom while deciding);
   *  `scried`: the scry for this hand is done (no scry after keeping). */
  mulligan: { count: number; kept: boolean; scryLeft?: number; scried?: boolean } | null;
  /** Card def keys from the deck's Tokens/Extras section. */
  tokenDefs: string[];
  archenemy: boolean;
  /** Rules games: mana currently in the pool by color (W U B R G C). */
  manaPool?: Record<string, number>;
}

export interface GameSettings {
  name: string;
  /** sandbox / rules / draft / sealed (see GAME_TYPES); older saved games have none and are sandbox */
  type?: import('../api').GameType;
  /** draft / sealed: set codes, one per pack round (draft) or the sets the packs come from (sealed) */
  sets?: string[];
  /** draft / sealed: rules enforced games (AI drafters allowed); false = sandbox games only. Missing = true. */
  enforced?: boolean;
  /** draft: a pick timer (official paper times); off (the default) waits as long as it takes */
  pickTimer?: boolean;
  format: string;
  life: number;
  mulligan: MulliganRule;
  bo: 1 | 3;
  maxPlayers: number;
  spectators: boolean;
  specChatShared: boolean;
  ownersOnly: boolean;
  dungeons: boolean;
  planechase: boolean;
  planarPool: 'host' | 'all';
  archenemy: boolean;
  /** no longer offered (joke sets are not supported); kept so older saved games load */
  attractions?: boolean;
}

export interface Marker {
  id: string;
  seat: number | null;
  label: string;
  value: number;
}

export interface HandViewRequest {
  id: string;
  from: number;
  to: number;
}

export interface GameState {
  v: 1;
  id: string;
  settings: GameSettings;
  seats: Seat[];
  cards: Record<string, CardInst>;
  defs: Record<string, CardDef>;
  table: {
    planarDeck: string[];
    planes: string[];
    /** planar die rolls this turn (each roll after the first costs one more mana) */
    planarRolls?: number;
    /** a phenomenon being encountered: resolve it, then planeswalk on */
    encounter?: string | null;
  };
  turn: { active: number; n: number; phase: number };
  stage: 'mulligan' | 'play' | 'ended';
  global: { monarch: number | null; initiative: number | null; day: 'day' | 'night' | null; markers: Marker[] };
  requests: HandViewRequest[];
  match: { game: number; wins: number[]; results: (number | null)[] };
  winner: number | null;
  nextIid: number;
  startedAt: number;
}

export interface LogDraft {
  kind: 'act' | 'chat' | 'life' | 'system';
  text: string;
  /** Text shown to seats not in privateTo (card names removed). */
  redacted?: string;
  privateTo?: number[];
  /** For life merging. */
  life?: { target: number; from: number; to: number };
  channel?: 'spectators';
}

export interface LogEntry extends LogDraft {
  id: number;
  at: number;
  actor: number | null;
  actorName: string;
}

export type Notice =
  | { to: number | 'all'; kind: 'toast'; text: string; color?: string }
  | { to: number | 'all'; kind: 'sound'; sound: 'shuffle' | 'draw' | 'tap' | 'dice' | 'turn' | 'ping' }
  | { to: number; kind: 'commanderPrompt'; iid: string }
  | { to: number | 'all'; kind: 'cards'; mode: 'look' | 'scry' | 'reveal' | 'search'; seat: number; zone: ZoneName; iids: string[]; title: string }
  | { to: number; kind: 'handRequest'; from: number; id: string }
  | { to: number; kind: 'vancouverScry'; /** cards to scry (Vancouver 1, free mulligan + scry 3) */ n?: number }
  | { to: number | 'all'; kind: 'yourTurn'; seat: number };

export interface Rng {
  int(maxExclusive: number): number;
}

/** Randomness resolved before reducing, stored with the event so replays are deterministic. */
export interface Resolved {
  perm?: number[];
  perms?: Record<string, number[]>;
  index?: number;
  value?: number;
  defs?: Record<string, CardDef>;
}
