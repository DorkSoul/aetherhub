import type { CardDef } from '../cards';
import type { Section } from '../deck';
import { emptyZones, makeCard, makePerm } from './engine';
import type { GameSettings, GameState, Rng, Seat } from './types';

export interface SeatInput {
  userId: number;
  name: string;
  color: string;
  deckId: number | null;
  deckName: string;
  sleeve: string | null;
  playmat: string | null;
  autoUntap: boolean;
  autoDraw?: boolean;
  cards: { def: string; section: Section; qty: number }[];
}

export const DEFAULT_SETTINGS: GameSettings = {
  name: 'Friday Commander',
  type: 'sandbox',
  format: 'commander',
  life: 40,
  mulligan: 'none',
  bo: 1,
  maxPlayers: 4,
  spectators: true,
  specChatShared: true,
  ownersOnly: false,
  dungeons: false,
  planechase: false,
  planarPool: 'all',
  archenemy: false,
};

function newSeat(i: SeatInput, life: number): Seat {
  return {
    userId: i.userId,
    name: i.name,
    color: i.color,
    deckId: i.deckId,
    deckName: i.deckName,
    sleeve: i.sleeve,
    playmat: i.playmat,
    life,
    lifeHist: [life],
    counters: { poison: 0 },
    cmdTax: {},
    cmdDmg: {},
    zones: emptyZones(),
    handFrac: 0.27,
    cardScale: 1,
    connected: true,
    conceded: false,
    revealTop: false,
    handViewers: [],
    handRevealAll: false,
    autoUntap: i.autoUntap,
    autoDraw: !!i.autoDraw,
    dungeon: null,
    ring: 0,
    citysBlessing: false,
    mulligan: null,
    tokenDefs: [],
    archenemy: false,
  };
}

/** A shuffled copy of `arr`. */
function shuffled<T>(arr: T[], rng: Rng): T[] {
  return makePerm(rng, arr.length).map((i) => arr[i]);
}

/**
 * Builds a fresh game. `order` lists the seat inputs in turn order (seat 0 goes first unless `starter` is given).
 * Libraries are shuffled, commanders/companions go to the command zone, everyone draws 7.
 * Cards of hidden decks (library, scheme and planar decks) are created in shuffled order, so their ids
 * (`c1`, `c2`, …) say nothing about the deck list's order or which cards are copies of each other.
 */
export function createGame(opts: {
  id: string;
  settings: GameSettings;
  seats: SeatInput[];
  defs: Record<string, CardDef>;
  rng: Rng;
  now: number;
  starter?: number;
  match?: GameState['match'];
  hostSeat?: number;
}): GameState {
  const { settings, rng } = opts;
  const state: GameState = {
    v: 1,
    id: opts.id,
    settings,
    seats: opts.seats.map((s) => newSeat(s, settings.life)),
    cards: {},
    defs: { ...opts.defs },
    table: { planarDeck: [], planes: [] },
    turn: { active: opts.starter ?? 0, n: 1, phase: 0 },
    stage: settings.mulligan === 'none' ? 'play' : 'mulligan',
    global: { monarch: null, initiative: null, day: null, markers: [] },
    requests: [],
    match: opts.match ?? { game: 1, wins: opts.seats.map(() => 0), results: [] },
    winner: null,
    nextIid: 1,
    startedAt: opts.now,
  };
  const planar: { def: string; seat: number }[] = [];
  opts.seats.forEach((inp, si) => {
    const seat = state.seats[si];
    const library: string[] = [];
    const schemes: string[] = [];
    for (const e of inp.cards) {
      if (!state.defs[e.def]) continue;
      for (let k = 0; k < e.qty; k++) {
        switch (e.section) {
          case 'main':
            library.push(e.def);
            break;
          case 'commander':
          case 'companion': {
            const cmd = makeCard(state, e.def, si, { commander: e.section === 'commander', companion: e.section === 'companion' });
            seat.zones.command.push(cmd.iid);
            if (cmd.commander) seat.cmdTax[cmd.iid] = 0;
            break;
          }
          case 'side':
            seat.zones.sideboard.push(makeCard(state, e.def, si).iid);
            break;
          case 'tokens':
            if (!seat.tokenDefs.includes(e.def)) seat.tokenDefs.push(e.def);
            break;
          case 'planar':
            if (settings.planechase && (settings.planarPool === 'all' || si === (opts.hostSeat ?? 0))) planar.push({ def: e.def, seat: si });
            break;
          case 'schemes':
            if (settings.archenemy) {
              schemes.push(e.def);
              seat.archenemy = true;
            }
            break;
          default:
            break;
        }
      }
    }
    // Two independent shuffles: one for the order the ids are handed out in, one for the deck order itself.
    seat.zones.library = shuffled(shuffled(library, rng).map((def) => makeCard(state, def, si).iid), rng);
    seat.zones.schemeDeck = shuffled(shuffled(schemes, rng).map((def) => makeCard(state, def, si).iid), rng);
    for (let k = 0; k < 7 && seat.zones.library.length; k++) seat.zones.hand.push(seat.zones.library.shift()!);
    if (state.stage === 'mulligan') seat.mulligan = { count: 0, kept: false };
  });
  // the archenemy starts at 40 life (and their opponents at the format's life)
  for (const seat of state.seats) if (seat.archenemy) {
    seat.life = Math.max(seat.life, 40);
    seat.lifeHist = [seat.life];
  }
  state.table.planarDeck = shuffled(shuffled(planar, rng).map((p) => makeCard(state, p.def, p.seat).iid), rng);
  if (state.table.planarDeck.length) {
    state.table.planes.push(state.table.planarDeck.shift()!);
  }
  // Prune defs that aren't used by any card or token list, so views stay small.
  const used = new Set<string>();
  for (const c of Object.values(state.cards)) used.add(c.def);
  for (const s of state.seats) s.tokenDefs.forEach((d) => used.add(d));
  for (const k of Object.keys(state.defs)) if (!used.has(k)) delete state.defs[k];
  return state;
}
