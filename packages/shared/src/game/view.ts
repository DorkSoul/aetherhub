// Per-viewer redaction of game state and log.

import type { CardDef } from '../cards';
import type { CardInst, GameState, LogEntry, ZoneName } from './types';
import { SEAT_ZONES } from './types';

export type Viewer = number | 'spectator';

export interface CardView extends CardInst {
  hidden?: boolean;
  /** The top card of a pile whose cards lie exactly on top of each other: how many cards the pile holds (display only). */
  stacked?: number;
}

export interface GameView extends Omit<GameState, 'cards'> {
  cards: Record<string, CardView>;
  viewer: Viewer;
}

/** Can `viewer` see the face of card `c`, sitting in `zone` of seat `seat`? */
export function canSee(state: GameState, viewer: Viewer, c: CardInst, seat: number, zone: ZoneName | 'planarDeck' | 'planes'): boolean {
  if (viewer === 'spectator') return true;
  switch (zone) {
    case 'battlefield':
    case 'exile':
      return !c.faceDown || c.peekers.includes(viewer);
    case 'graveyard':
    case 'command':
    case 'schemes':
    case 'planes':
      return true;
    case 'hand': {
      const s = state.seats[seat];
      return seat === viewer || s.handRevealAll || s.handViewers.includes(viewer);
    }
    case 'library': {
      const s = state.seats[seat];
      return s.revealTop && s.zones.library[0] === c.iid;
    }
    case 'sideboard':
      return seat === viewer;
    default:
      return false;
  }
}

/**
 * What `viewer` sees of the game. `alsoSeats`: other seats the same player holds (a solo game on the server), whose
 * hidden cards they see too, as at a real table where you play every hand yourself.
 */
export function viewFor(state: GameState, viewer: Viewer, alsoSeats: number[] = []): GameView {
  const cards: Record<string, CardView> = {};
  const defs: Record<string, CardDef> = {};
  const addDef = (k: string) => {
    if (state.defs[k]) defs[k] = state.defs[k];
  };
  const place = (iid: string, seat: number, zone: ZoneName | 'planarDeck' | 'planes') => {
    const c = state.cards[iid];
    if (!c) return;
    if (canSee(state, viewer, c, seat, zone) || alsoSeats.some((v) => canSee(state, v, c, seat, zone))) {
      cards[iid] = c;
      addDef(c.def);
      if (c.melded && state.cards[c.melded]) {
        cards[c.melded] = state.cards[c.melded];
        addDef(state.cards[c.melded].def);
      }
    } else if (zone === 'battlefield' || zone === 'exile') {
      // Face-down permanents keep their public state; only identity is hidden.
      cards[iid] = { ...c, def: '', hidden: true, peekers: [] };
    } else {
      cards[iid] = { ...blank(c), hidden: true };
    }
  };
  state.seats.forEach((s, si) => {
    for (const z of SEAT_ZONES) for (const iid of s.zones[z]) place(iid, si, z);
    s.tokenDefs.forEach(addDef);
  });
  for (const iid of state.table.planarDeck) place(iid, -1, 'planarDeck');
  for (const iid of state.table.planes) place(iid, -1, 'planes');
  return { ...state, cards, defs, viewer };
}

function blank(c: CardInst): CardInst {
  return {
    iid: c.iid,
    def: '',
    owner: c.owner,
    faceDown: false,
    tapped: false,
    x: 0,
    y: 0,
    face: 0,
    flipped: false,
    counters: {},
    effects: [],
    pt: { p: 0, t: 0 },
    ptEot: { p: 0, t: 0 },
    pile: c.pile,
    noUntap: false,
    token: c.token,
    attachedTo: null,
    melded: null,
    commander: c.commander,
    companion: c.companion,
    peekers: [],
  };
}

export interface LogView {
  id: number;
  at: number;
  actor: number | null;
  actorName: string;
  kind: LogEntry['kind'];
  text: string;
  priv: boolean;
}

export function logFor(e: LogEntry, viewer: Viewer, spectatorChatShared: boolean): LogView | null {
  if (e.channel === 'spectators' && viewer !== 'spectator' && !spectatorChatShared) return null;
  const full = viewer === 'spectator' || !e.privateTo || e.privateTo.includes(viewer);
  return {
    id: e.id,
    at: e.at,
    actor: e.actor,
    actorName: e.actorName,
    kind: e.kind,
    text: full ? e.text : (e.redacted ?? e.text),
    priv: !!(full && e.redacted && viewer !== 'spectator' && e.redacted !== e.text),
  };
}
