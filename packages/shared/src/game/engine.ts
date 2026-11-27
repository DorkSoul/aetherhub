// The game reducer. Pure apart from mutating the state it is given: the server clones the state,
// resolves randomness (see resolveIntent), then calls applyIntent. Replaying the same events with the
// same `resolved` data reproduces the same state, which is what undo/rewind relies on.

import type { CardDef } from '../cards';
import { DUNGEONS } from './dungeons';
import type { Intent } from './intents';
import type { CardInst, GameState, LogDraft, Notice, Resolved, Rng, Seat, ZoneName } from './types';
import { HIDDEN_ZONES, SEAT_ZONES, ZONE_LABEL } from './types';

export class IntentError extends Error {}
function fail(msg: string): never {
  throw new IntentError(msg);
}

export interface ApplyCtx {
  actor: number; // seat index, or -1 for a spectator / unseated admin
  actorName: string;
  now: number;
}

export interface ApplyResult {
  logs: LogDraft[];
  notices: Notice[];
}

// ------------------------------------------------------------------ helpers

export interface Loc {
  seat: number; // -1 for table zones (planar)
  zone: ZoneName | 'planarDeck' | 'planes';
  index: number;
  card: CardInst;
}

export function locate(state: GameState, iid: string): Loc | null {
  const card = state.cards[iid];
  if (!card) return null;
  for (let s = 0; s < state.seats.length; s++) {
    const zones = state.seats[s].zones;
    for (const z of SEAT_ZONES) {
      const i = zones[z].indexOf(iid);
      if (i >= 0) return { seat: s, zone: z, index: i, card };
    }
  }
  for (const z of ['planarDeck', 'planes'] as const) {
    const i = state.table[z].indexOf(iid);
    if (i >= 0) return { seat: -1, zone: z, index: i, card };
  }
  return null;
}

function zoneArr(state: GameState, seat: number, zone: Loc['zone']): string[] {
  if (seat < 0) return state.table[zone as 'planarDeck' | 'planes'];
  return state.seats[seat].zones[zone as ZoneName];
}

export function emptyZones(): Record<ZoneName, string[]> {
  return Object.fromEntries(SEAT_ZONES.map((z) => [z, [] as string[]])) as unknown as Record<ZoneName, string[]>;
}

export function makeCard(state: GameState, def: string, owner: number, o: Partial<CardInst> = {}): CardInst {
  const c: CardInst = {
    iid: 'c' + state.nextIid++,
    def,
    owner,
    faceDown: false,
    tapped: false,
    x: 0.45,
    y: 0.4,
    face: 0,
    flipped: false,
    counters: {},
    effects: [],
    pt: { p: 0, t: 0 },
    ptEot: { p: 0, t: 0 },
    pile: 1,
    noUntap: false,
    token: false,
    attachedTo: null,
    melded: null,
    commander: false,
    companion: false,
    peekers: [],
    ...o,
  };
  state.cards[c.iid] = c;
  return c;
}

export function cardName(state: GameState, c: CardInst): string {
  const d = state.defs[c.def];
  if (!d) return 'a card';
  if (d.faces && d.faces.length > 1 && d.img.length > 1) return d.faces[c.face]?.name || d.name;
  if (d.faces && d.layout === 'flip') return d.faces[c.flipped ? 1 : 0]?.name || d.name;
  return d.name;
}

function isPublicZone(zone: Loc['zone']): boolean {
  return !HIDDEN_ZONES.includes(zone as ZoneName) && zone !== 'planarDeck';
}

function seatName(state: GameState, s: number): string {
  return state.seats[s]?.name ?? 'someone';
}

/** "their" when the seat is the actor, otherwise "Bryn's". */
function poss(state: GameState, ctx: ApplyCtx, s: number): string {
  return s === ctx.actor ? 'their' : `${seatName(state, s)}'s`;
}

function list(names: string[]): string {
  if (names.length <= 1) return names[0] || '';
  if (names.length > 4) return `${names.length} cards`;
  return names.slice(0, -1).join(', ') + ' and ' + names[names.length - 1];
}

function plural(n: number, s: string): string {
  return `${n} ${s}${n === 1 ? '' : 's'}`;
}

function requireSeat(ctx: ApplyCtx): number {
  if (ctx.actor < 0) fail('Only seated players can do that');
  return ctx.actor;
}

function checkSeat(state: GameState, s: number) {
  if (!state.seats[s]) fail('No such seat');
}

function removeFromZone(state: GameState, loc: Loc) {
  zoneArr(state, loc.seat, loc.zone).splice(loc.index, 1);
}

function resetOnLeave(state: GameState, c: CardInst) {
  c.tapped = false;
  c.effects = [];
  c.pt = { p: 0, t: 0 };
  c.ptEot = { p: 0, t: 0 };
  c.counters = {};
  c.flipped = false;
  c.face = 0;
  if (c.attachedTo) c.attachedTo = null;
  for (const o of Object.values(state.cards)) if (o.attachedTo === c.iid) o.attachedTo = null;
}

interface MoveOpts {
  x?: number;
  y?: number;
  pos?: 'top' | 'bottom' | number;
  faceDown?: boolean;
}

interface MoveInfo {
  card: CardInst;
  from: Loc;
  wasFaceDown: boolean;
}

/** Moves one card. Returns info for logging. Notices (commander prompt) are pushed into `notices`. */
export function moveCard(state: GameState, ctx: ApplyCtx, iid: string, toSeat: number, toZone: ZoneName, o: MoveOpts, notices: Notice[]): MoveInfo {
  const loc = locate(state, iid);
  if (!loc) fail('Card not found');
  checkSeat(state, toSeat);
  const c = loc.card;
  const wasFaceDown = c.faceDown;
  // Dungeons and the Initiative switch on by themselves the first time a card that uses them is played
  if (!state.settings.dungeons && (toZone === 'battlefield' || (toZone === 'graveyard' && loc.zone === 'hand')) && !o.faceDown && usesDungeons(state.defs[c.def])) {
    state.settings.dungeons = true;
    notices.push({ to: 'all', kind: 'toast', text: `${state.defs[c.def].name} uses dungeons: venture and the Initiative are now in the Specials menu` });
  }
  removeFromZone(state, loc);
  const sameZone = loc.seat === toSeat && loc.zone === toZone;
  if (loc.zone === 'battlefield' && toZone !== 'battlefield') resetOnLeave(state, c);
  if (toZone === 'battlefield') {
    c.x = o.x ?? (sameZone ? c.x : 0.45);
    c.y = o.y ?? (sameZone ? c.y : 0.4);
    if (!sameZone) c.faceDown = !!o.faceDown;
    else if (o.faceDown != null) c.faceDown = o.faceDown;
    if (c.faceDown) c.peekers = Array.from(new Set([...c.peekers, toSeat, ctx.actor].filter((s) => s >= 0)));
    // Moving an attached card by itself detaches it; things attached to a card follow it to another battlefield.
    if (sameZone) c.attachedTo = null;
    else if (loc.zone === 'battlefield') {
      for (const other of Object.values(state.cards)) {
        if (other.attachedTo !== c.iid) continue;
        const ol = locate(state, other.iid);
        if (!ol || ol.zone !== 'battlefield') continue;
        removeFromZone(state, ol);
        state.seats[toSeat].zones.battlefield.push(other.iid);
        other.x = c.x;
        other.y = c.y;
      }
    }
  } else if (toZone === 'exile' && o.faceDown) {
    c.faceDown = true;
    c.peekers = Array.from(new Set([c.owner, ctx.actor].filter((s) => s >= 0)));
  } else {
    c.faceDown = false;
    c.peekers = [];
  }
  const dest = state.seats[toSeat].zones[toZone];
  if (toZone === 'library' || toZone === 'schemeDeck') {
    if (o.pos === 'bottom') dest.push(iid);
    else if (typeof o.pos === 'number') dest.splice(Math.min(dest.length, o.pos - 1), 0, iid);
    else dest.unshift(iid);
  } else dest.push(iid);

  // Melded pairs travel together when they leave the battlefield.
  if (c.melded && loc.zone === 'battlefield' && toZone !== 'battlefield') {
    const partner = state.cards[c.melded];
    c.melded = null;
    if (partner) {
      partner.melded = null;
      state.seats[toSeat].zones[toZone].push(partner.iid);
    }
  }

  // Commander tax and "move to command zone?" prompt.
  if (c.commander && loc.zone === 'command' && toZone !== 'command') {
    const owner = state.seats[c.owner];
    owner.cmdTax[c.iid] = (owner.cmdTax[c.iid] || 0) + 2;
  }
  if (c.commander && ['graveyard', 'exile', 'hand', 'library'].includes(toZone) && !(loc.zone === toZone && loc.seat === toSeat))
    notices.push({ to: c.owner, kind: 'commanderPrompt', iid: c.iid });
  return { card: c, from: loc, wasFaceDown };
}

function describeMoves(state: GameState, ctx: ApplyCtx, moves: MoveInfo[], toSeat: number, toZone: ZoneName, faceDown?: boolean, pos?: MoveOpts['pos']): LogDraft | null {
  if (!moves.length) return null;
  const pub: string[] = [];
  const priv: string[] = [];
  let hiddenCount = 0;
  let revealedFd = false;
  const privateTo = new Set<number>([ctx.actor]);
  for (const m of moves) {
    const nm = cardName(state, m.card);
    const fromPublic = isPublicZone(m.from.zone) && !(m.wasFaceDown && m.from.zone !== 'battlefield');
    const toPublic = isPublicZone(toZone) && !m.card.faceDown;
    const fromFaceDownVisible = m.wasFaceDown && m.from.zone === 'battlefield' && toZone !== 'battlefield';
    if ((fromPublic && !m.wasFaceDown) || toPublic || fromFaceDownVisible) {
      pub.push(nm);
      if (fromFaceDownVisible) revealedFd = true;
    } else {
      priv.push(nm);
      hiddenCount++;
      if (m.from.zone === 'hand' || m.from.zone === 'library') privateTo.add(m.from.seat);
      if (toZone === 'hand') privateTo.add(toSeat);
    }
  }
  const f = moves[0].from;
  const sameSeatBf = f.zone === 'battlefield' && toZone === 'battlefield';
  let verb: string;
  const where = (z: ZoneName) =>
    z === 'battlefield'
      ? toSeat === ctx.actor
        ? 'the battlefield'
        : `${seatName(state, toSeat)}'s battlefield`
      : z === 'command'
        ? toSeat === ctx.actor
          ? 'the command zone'
          : `${seatName(state, toSeat)}'s command zone`
        : `${poss(state, ctx, toSeat)} ${ZONE_LABEL[z]}`;
  const posTxt = (p?: MoveOpts['pos']) => (toZone === 'library' ? (p === 'bottom' ? 'the bottom of ' : typeof p === 'number' && p > 1 ? `position ${p} from the top of ` : 'the top of ') : '');
  const names = (arr: string[]) => list(arr);
  const all = [...pub, ...priv];
  if (toZone === 'battlefield' && sameSeatBf && f.seat !== toSeat) verb = `gave control of ${names(pub.length ? pub : ['a face-down card'])} to ${seatName(state, toSeat)}`;
  else if (toZone === 'battlefield' && f.zone === 'hand') verb = faceDown ? `played ${names(all)} face down` : `played ${names(all)}`;
  else if (toZone === 'hand' && f.zone === 'library') {
    const src = f.seat === ctx.actor ? '' : ` from ${seatName(state, f.seat)}'s library`;
    const into = toSeat === ctx.actor ? '' : ` into ${seatName(state, toSeat)}'s hand`;
    return {
      kind: 'act',
      text: `drew ${names(all)}${src}${into}`,
      redacted: `drew ${plural(moves.length, 'card')}${src}${into}`,
      privateTo: Array.from(privateTo),
    };
  } else if (toZone === 'exile' && faceDown) verb = `exiled ${names(all)} face down`;
  else verb = `put ${names(all)} ${toZone === 'battlefield' ? 'onto' : 'into'} ${posTxt(pos)}${where(toZone)}`;
  if (hiddenCount === 0) return { kind: 'act', text: verb + (revealedFd ? ' (revealed: it was face down)' : '') };
  // Some names are private.
  const red = verb.replace(names(all), pub.length ? `${names(pub)} and ${plural(hiddenCount, 'card')}` : plural(hiddenCount, 'card'));
  return { kind: 'act', text: verb, redacted: red, privateTo: Array.from(privateTo) };
}

function ownersOnlyCheck(state: GameState, ctx: ApplyCtx, iids: string[]) {
  if (!state.settings.ownersOnly || ctx.actor < 0) return;
  for (const iid of iids) {
    const l = locate(state, iid);
    if (!l) continue;
    if (l.card.owner !== ctx.actor && l.seat !== ctx.actor) fail('Only the owner can touch that card in this game');
  }
}

function shuffleInPlace(arr: string[], perm: number[] | undefined) {
  if (!perm || perm.length !== arr.length) return;
  const copy = arr.slice();
  for (let i = 0; i < arr.length; i++) arr[i] = copy[perm[i]];
}

/** Re-keys every card in a library so clients can't follow hidden cards through a shuffle. */
function rekey(state: GameState, seat: number) {
  const lib = state.seats[seat].zones.library;
  for (let i = 0; i < lib.length; i++) {
    const c = state.cards[lib[i]];
    delete state.cards[lib[i]];
    c.iid = 'c' + state.nextIid++;
    state.cards[c.iid] = c;
    lib[i] = c.iid;
  }
}

export function makePerm(rng: Rng, n: number): number[] {
  const p = Array.from({ length: n }, (_, i) => i);
  for (let i = n - 1; i > 0; i--) {
    const j = rng.int(i + 1);
    [p[i], p[j]] = [p[j], p[i]];
  }
  return p;
}

// ------------------------------------------------------------------ randomness

/** Resolves randomness for an intent. Server-side only (crypto RNG); the result is stored with the event. */
export function resolveIntent(state: GameState, ctx: ApplyCtx, intent: Intent, rng: Rng): Resolved {
  switch (intent.type) {
    case 'shuffle':
      return { perm: makePerm(rng, state.seats[intent.seat]?.zones.library.length ?? 0) };
    case 'mulligan': {
      const s = state.seats[ctx.actor];
      if (!s) return {};
      return { perm: makePerm(rng, s.zones.library.length + s.zones.hand.length) };
    }
    case 'randomCard': {
      const n = state.seats[intent.seat]?.zones[intent.zone].length ?? 0;
      return { index: n ? rng.int(n) : -1 };
    }
    case 'roll':
      return { value: 1 + rng.int(intent.sides) };
    case 'coin':
      return { value: rng.int(2) };
    case 'planarRoll':
      return { value: rng.int(6) };
    default:
      return {};
  }
}

// ------------------------------------------------------------------ apply

export function applyIntent(state: GameState, ctx: ApplyCtx, intent: Intent, resolved: Resolved): ApplyResult {
  const logs: LogDraft[] = [];
  const notices: Notice[] = [];
  const log = (text: string, extra: Partial<LogDraft> = {}) => logs.push({ kind: 'act', text, ...extra });
  /** Logs a line that names cards. A face-down card is named only to the players who may peek at it (and to
   *  spectators); everyone else reads "a face-down card". `line` builds the text from a naming function. */
  const logNamed = (cs: CardInst[], line: (name: (c: CardInst) => string) => string) => {
    const full = line((c) => cardName(state, c));
    const fd = cs.filter((c) => c.faceDown);
    if (!fd.length) return log(full);
    const redacted = line((c) => (c.faceDown ? 'a face-down card' : cardName(state, c)));
    const privateTo = fd.map((c) => c.peekers).reduce((a, b) => a.filter((x) => b.includes(x)));
    log(redacted, { text: full, redacted, privateTo });
  };
  const card = (iid: string) => state.cards[iid] || fail('Card not found');
  const onBf = (iid: string) => {
    const l = locate(state, iid);
    if (!l) fail('Card not found');
    return l;
  };

  switch (intent.type) {
    // ---------------------------------------------------------- cards
    case 'move': {
      ownersOnlyCheck(state, ctx, intent.iids);
      const moves: MoveInfo[] = [];
      intent.iids.forEach((iid, i) => {
        const p = intent.positions?.[i];
        const before = locate(state, iid);
        if (!before) return;
        // reordering a battlefield or a hand is not logged
        const sameZone = (before.zone === 'battlefield' || before.zone === 'hand') && before.zone === intent.zone && before.seat === intent.seat;
        const m = moveCard(state, ctx, iid, intent.seat, intent.zone, { x: p?.x ?? intent.x, y: p?.y ?? intent.y, pos: intent.pos, faceDown: intent.faceDown }, notices);
        if (!sameZone) moves.push(m);
      });
      // a hand position: the cards go there together, in the order given (counted among the cards that stay)
      if (intent.zone === 'hand' && typeof intent.pos === 'number') {
        const hand = state.seats[intent.seat].zones.hand;
        const moved = intent.iids.filter((iid) => hand.includes(iid));
        const rest = hand.filter((iid) => !moved.includes(iid));
        rest.splice(Math.min(rest.length, intent.pos - 1), 0, ...moved);
        hand.splice(0, hand.length, ...rest);
      }
      const d = describeMoves(state, ctx, moves, intent.seat, intent.zone, intent.faceDown, intent.pos);
      if (d) logs.push(d);
      if (moves.some((m) => m.from.zone === 'library' && intent.zone === 'hand')) notices.push({ to: 'all', kind: 'sound', sound: 'draw' });
      break;
    }
    case 'toFront': {
      for (const iid of intent.iids) {
        const l = locate(state, iid);
        if (l && l.zone === 'battlefield') {
          const arr = state.seats[l.seat].zones.battlefield;
          arr.splice(l.index, 1);
          arr.push(iid);
        }
      }
      break;
    }
    case 'tap': {
      ownersOnlyCheck(state, ctx, intent.iids);
      const names: string[] = [];
      let tapped = true;
      for (const iid of intent.iids) {
        const l = onBf(iid);
        if (l.zone !== 'battlefield') continue;
        l.card.tapped = intent.tapped ?? !l.card.tapped;
        tapped = l.card.tapped;
        names.push(l.card.faceDown ? 'a face-down card' : cardName(state, l.card) + (l.card.pile > 1 ? ` (×${l.card.pile})` : ''));
      }
      if (names.length) log(`${tapped ? 'tapped' : 'untapped'} ${names.length > 3 ? plural(names.length, 'permanent') : list(names)}`);
      notices.push({ to: 'all', kind: 'sound', sound: 'tap' });
      break;
    }
    case 'untapAll': {
      checkSeat(state, intent.seat);
      if (state.settings.ownersOnly && intent.seat !== ctx.actor) fail('Only the owner can do that in this game');
      for (const iid of state.seats[intent.seat].zones.battlefield) {
        const c = card(iid);
        if (!c.noUntap) c.tapped = false;
      }
      log(`untapped all ${intent.seat === ctx.actor ? 'their' : seatName(state, intent.seat) + "'s"} permanents`);
      notices.push({ to: 'all', kind: 'sound', sound: 'tap' });
      break;
    }
    case 'faceDown': {
      ownersOnlyCheck(state, ctx, intent.iids);
      for (const iid of intent.iids) {
        const l = onBf(iid);
        const nm = cardName(state, l.card);
        l.card.faceDown = intent.value;
        if (intent.value) {
          l.card.peekers = Array.from(new Set([l.seat, ctx.actor, l.card.owner].filter((s) => s >= 0)));
          log('turned a card face down', { text: `turned ${nm} face down`, redacted: 'turned a card face down', privateTo: l.card.peekers });
        } else {
          l.card.peekers = [];
          log(`turned ${nm} face up`);
        }
      }
      break;
    }
    case 'transform': {
      ownersOnlyCheck(state, ctx, [intent.iid]);
      const c = card(intent.iid);
      const d = state.defs[c.def];
      const before = cardName(state, c);
      const faces = d?.faces?.length || 1;
      if (faces < 2) fail('This card has only one face');
      c.face = (c.face + 1) % faces;
      logNamed([c], (name) => (c.faceDown ? `transformed ${name(c)}` : `transformed ${before} into ${name(c)}`));
      break;
    }
    case 'flip': {
      ownersOnlyCheck(state, ctx, [intent.iid]);
      const c = card(intent.iid);
      c.flipped = !c.flipped;
      logNamed([c], (name) => `flipped ${name(c)}`);
      break;
    }
    case 'attach': {
      ownersOnlyCheck(state, ctx, [intent.iid]);
      const c = card(intent.iid);
      if (intent.to) {
        if (intent.to === intent.iid) fail("Can't attach a card to itself");
        const host = onBf(intent.to);
        const me = onBf(intent.iid);
        if (host.zone !== 'battlefield' || me.zone !== 'battlefield') fail('Both cards must be on the battlefield');
        c.attachedTo = intent.to;
        logNamed([c, host.card], (name) => `attached ${name(c)} to ${name(host.card)}`);
      } else {
        c.attachedTo = null;
        logNamed([c], (name) => `detached ${name(c)}`);
      }
      break;
    }
    case 'meld': {
      ownersOnlyCheck(state, ctx, [intent.iid, intent.with]);
      const a = onBf(intent.iid);
      const b = onBf(intent.with);
      if (a.zone !== 'battlefield' || b.zone !== 'battlefield') fail('Both cards must be on the battlefield');
      removeFromZone(state, b);
      a.card.melded = b.card.iid;
      a.card.face = 1;
      logNamed([a.card, b.card], (name) => `melded ${name(a.card)} with ${name(b.card)}`);
      break;
    }
    case 'reveal': {
      const names = intent.iids.map((i) => cardName(state, card(i)));
      const l = locate(state, intent.iids[0]);
      log(`revealed ${list(names)}${l ? ` from ${poss(state, ctx, l.seat)} ${ZONE_LABEL[l.zone as ZoneName] || l.zone}` : ''}`);
      if (l && l.seat >= 0) notices.push({ to: 'all', kind: 'cards', mode: 'reveal', seat: l.seat, zone: l.zone as ZoneName, iids: intent.iids, title: `${ctx.actorName} revealed` });
      break;
    }
    case 'copy': {
      const l = onBf(intent.iid);
      // A copy of a face-down card is face-down too, and only the original's peekers may look at it.
      const fd = l.card.faceDown;
      const c = makeCard(state, l.card.def, l.seat >= 0 ? l.seat : 0, { token: true, x: Math.min(1, l.card.x + 0.08), y: l.card.y, face: l.card.face, faceDown: fd, peekers: fd ? [...l.card.peekers] : [] });
      state.seats[l.seat].zones.battlefield.push(c.iid);
      logNamed([l.card], (name) => `created a token copy of ${name(l.card)}`);
      break;
    }
    case 'counter': {
      ownersOnlyCheck(state, ctx, intent.iids);
      const names: string[] = [];
      let now = 0;
      for (const iid of intent.iids) {
        const c = card(iid);
        now = Math.max(0, (c.counters[intent.counter] || 0) + intent.delta);
        if (now) c.counters[intent.counter] = now;
        else delete c.counters[intent.counter];
        names.push(c.faceDown ? 'a face-down card' : cardName(state, c));
      }
      const n = Math.abs(intent.delta);
      log(`${intent.delta > 0 ? 'put' : 'removed'} ${n === 1 ? 'a' : n} ${intent.counter} counter${n === 1 ? '' : 's'} ${intent.delta > 0 ? 'on' : 'from'} ${list(names)}${intent.iids.length === 1 ? ` (now ${now})` : ''}`);
      break;
    }
    case 'ptMod': {
      ownersOnlyCheck(state, ctx, [intent.iid]);
      const c = card(intent.iid);
      const tgt = intent.eot ? c.ptEot : c.pt;
      tgt.p += intent.p;
      tgt.t += intent.t;
      const f = (n: number) => (n >= 0 ? '+' + n : '−' + Math.abs(n));
      logNamed([c], (name) => `gave ${name(c)} ${f(intent.p)}/${f(intent.t)}${intent.eot ? ' until end of turn' : ''}`);
      break;
    }
    case 'effect': {
      ownersOnlyCheck(state, ctx, [intent.iid]);
      const c = card(intent.iid);
      const i = c.effects.findIndex((e) => e.k === intent.k);
      if (i >= 0 || intent.remove) {
        if (i >= 0) c.effects.splice(i, 1);
        logNamed([c], (name) => `removed “${intent.k}” from ${name(c)}`);
      } else {
        c.effects.push({ k: intent.k, eot: !!intent.eot });
        logNamed([c], (name) => `gave ${name(c)} “${intent.k}”${intent.eot ? ' until end of turn' : ''}`);
      }
      break;
    }
    case 'noUntap': {
      const c = card(intent.iid);
      c.noUntap = intent.value;
      logNamed([c], (name) => `marked ${name(c)} ${intent.value ? "as doesn't untap" : 'to untap normally'}`);
      break;
    }
    case 'delete': {
      ownersOnlyCheck(state, ctx, intent.iids);
      const names: string[] = [];
      for (const iid of intent.iids) {
        const l = locate(state, iid);
        if (!l) continue;
        if (!l.card.token) fail('Only tokens can be deleted');
        removeFromZone(state, l);
        for (const o of Object.values(state.cards)) if (o.attachedTo === iid) o.attachedTo = null;
        delete state.cards[iid];
        names.push((l.card.pile > 1 ? l.card.pile + ' ' : '') + cardName(state, l.card));
      }
      if (names.length) log(`deleted ${list(names)} token${names.length === 1 && !names[0].match(/^\d/) ? '' : 's'}`);
      break;
    }
    case 'group': {
      const l = onBf(intent.iid);
      if (l.zone !== 'battlefield') fail('Only cards on the battlefield can be grouped');
      const arr = state.seats[l.seat].zones.battlefield;
      const same = arr.filter((i) => i !== intent.iid && state.cards[i].def === l.card.def && state.cards[i].token === l.card.token && !state.cards[i].faceDown);
      let n = l.card.pile;
      for (const i of same) {
        n += state.cards[i].pile;
        arr.splice(arr.indexOf(i), 1);
        delete state.cards[i];
      }
      l.card.pile = n;
      logNamed([l.card], (name) => `grouped ${n} ${name(l.card)} into a pile`);
      break;
    }
    case 'stack': {
      const locs = intent.iids.map(onBf);
      const first = locs[0];
      if (locs.some((x) => x.zone !== 'battlefield' || x.seat !== first.seat || x.card.def !== first.card.def || x.card.faceDown)) fail('Only identical face-up cards on one battlefield can be stacked');
      const arr = state.seats[first.seat].zones.battlefield;
      let n = first.card.pile;
      for (const x of locs.slice(1)) {
        n += x.card.pile;
        arr.splice(arr.indexOf(x.card.iid), 1);
        delete state.cards[x.card.iid];
      }
      first.card.pile = n;
      arr.splice(arr.indexOf(first.card.iid), 1);
      arr.push(first.card.iid);
      log(`stacked ${n} ${cardName(state, first.card)} into a pile`);
      break;
    }
    case 'split': {
      const l = onBf(intent.iid);
      const n = Math.min(intent.n ?? 1, l.card.pile - 1);
      if (n < 1) fail('Nothing to split');
      l.card.pile -= n;
      const c = makeCard(state, l.card.def, l.card.owner, {
        token: l.card.token,
        pile: n,
        x: Math.min(1, l.card.x + 0.08),
        y: l.card.y,
        tapped: intent.tap ? true : l.card.tapped,
        counters: { ...l.card.counters },
        effects: l.card.effects.map((e) => ({ ...e })),
      });
      zoneArr(state, l.seat, l.zone).push(c.iid);
      logNamed([l.card], (name) => (intent.tap ? `tapped ${n} of the ${name(l.card)} pile` : `split ${n} ${name(l.card)} off the pile`));
      break;
    }
    case 'ungroup': {
      const l = onBf(intent.iid);
      const n = l.card.pile;
      if (n < 2) fail('Not a pile');
      l.card.pile = 1;
      for (let i = 1; i < n; i++) {
        const c = makeCard(state, l.card.def, l.card.owner, { token: l.card.token, x: Math.min(1, l.card.x + i * 0.05), y: l.card.y, tapped: l.card.tapped });
        zoneArr(state, l.seat, l.zone).push(c.iid);
      }
      logNamed([l.card], (name) => `ungrouped ${n} ${name(l.card)}`);
      break;
    }
    case 'grantPeek': {
      const c = card(intent.iid);
      if (!c.faceDown) fail('The card is face up');
      if (!c.peekers.includes(ctx.actor)) fail('You can only share a card you may look at');
      if (!c.peekers.includes(intent.seat)) c.peekers.push(intent.seat);
      log(`let ${seatName(state, intent.seat)} look at a face-down card`);
      break;
    }

    // ---------------------------------------------------------- library
    case 'draw': {
      const me = requireSeat(ctx);
      checkSeat(state, intent.seat);
      if (state.settings.ownersOnly && intent.seat !== me) fail('Only the owner can do that in this game');
      const lib = state.seats[intent.seat].zones.library;
      const moves: MoveInfo[] = [];
      for (let i = 0; i < intent.n && lib.length; i++) {
        const iid = intent.bottom ? lib[lib.length - 1] : lib[0];
        moves.push(moveCard(state, ctx, iid, me, 'hand', {}, notices));
      }
      if (!moves.length) {
        log(`tried to draw from ${poss(state, ctx, intent.seat)} empty library`);
        break;
      }
      const d = describeMoves(state, ctx, moves, me, 'hand')!;
      if (intent.bottom) {
        d.text += ' (from the bottom)';
        if (d.redacted) d.redacted += ' (from the bottom)';
      }
      logs.push(d);
      notices.push({ to: 'all', kind: 'sound', sound: 'draw' });
      break;
    }
    case 'look': {
      const me = requireSeat(ctx);
      checkSeat(state, intent.seat);
      const lib = state.seats[intent.seat].zones.library;
      const top = lib.slice(0, intent.n);
      if (intent.mode === 'reveal') {
        const names = top.map((i) => cardName(state, card(i)));
        log(`revealed the top ${plural(top.length, 'card')} of ${poss(state, ctx, intent.seat)} library: ${names.join(', ')}`);
        notices.push({ to: 'all', kind: 'cards', mode: 'reveal', seat: intent.seat, zone: 'library', iids: top, title: `${ctx.actorName} revealed the top ${top.length}` });
        notices.push({ to: me, kind: 'cards', mode: 'look', seat: intent.seat, zone: 'library', iids: top, title: `Reveal top ${top.length}` });
      } else {
        log(`${intent.mode === 'scry' ? 'is scrying' : 'is looking at the top'} ${top.length} of ${poss(state, ctx, intent.seat)} library`);
        notices.push({ to: me, kind: 'cards', mode: intent.mode, seat: intent.seat, zone: 'library', iids: top, title: intent.mode === 'scry' ? `Scry ${top.length}` : `Look at the top ${top.length}` });
      }
      break;
    }
    case 'lookResolve': {
      const me = requireSeat(ctx);
      checkSeat(state, intent.seat);
      const lib = state.seats[intent.seat].zones.library;
      const ids = intent.items.map((x) => x.iid);
      const topSet = new Set(lib.slice(0, ids.length));
      if (!ids.every((i) => topSet.has(i))) fail('The top of the library changed — look again');
      for (const i of ids) lib.splice(lib.indexOf(i), 1);
      const tops: string[] = [];
      const bots: string[] = [];
      const moved: string[] = [];
      const privMoved: string[] = [];
      for (const it of intent.items) {
        if (it.dest === 'top') tops.push(it.iid);
        else if (it.dest === 'bottom') bots.push(it.iid);
      }
      state.seats[intent.seat].zones.library = [...tops, ...lib, ...bots];
      // free mulligans + scry 3: a scry of your own library while deciding on a hand counts as that hand's scry
      const mm = state.seats[me].mulligan;
      if (intent.mode === 'scry' && intent.seat === me && state.stage === 'mulligan' && state.settings.mulligan === 'free-scry3' && mm && !mm.kept) {
        mm.scryLeft = Math.max(0, (mm.scryLeft ?? 3) - bots.length);
        mm.scried = true;
      }
      for (const it of intent.items) {
        if (it.dest === 'top' || it.dest === 'bottom') continue;
        const c = card(it.iid);
        // temporarily put it back so moveCard can find it
        state.seats[intent.seat].zones.library.unshift(it.iid);
        const toSeat = it.dest === 'hand' ? me : c.owner;
        moveCard(state, ctx, it.iid, toSeat, it.dest, { x: 0.5, y: 0.4 }, notices);
        (it.dest === 'hand' ? privMoved : moved).push(`${cardName(state, c)} → ${ZONE_LABEL[it.dest]}`);
      }
      const whose = poss(state, ctx, intent.seat);
      if (intent.mode === 'scry') log(`scried ${ids.length} (${tops.length} on top, ${bots.length} on the bottom)`);
      else {
        const all = [...moved, ...privMoved];
        const txt = `looked at the top ${ids.length} of ${whose} library${all.length ? ': ' + all.join(', ') : ''}${tops.length || bots.length ? ` · ${tops.length} on top, ${bots.length} on the bottom` : ''}`;
        const red = `looked at the top ${ids.length} of ${whose} library${moved.length ? ': ' + moved.join(', ') : ''}${privMoved.length ? `${moved.length ? ',' : ':'} ${plural(privMoved.length, 'card')} → hand` : ''}`;
        log(txt, { redacted: red, privateTo: [me] });
      }
      break;
    }
    case 'mill': {
      checkSeat(state, intent.seat);
      if (state.settings.ownersOnly && intent.seat !== ctx.actor) fail('Only the owner can do that in this game');
      const lib = state.seats[intent.seat].zones.library;
      const names: string[] = [];
      for (let i = 0; i < intent.n && lib.length; i++) {
        const c = card(lib[0]);
        moveCard(state, ctx, c.iid, c.owner, 'graveyard', {}, notices);
        names.push(cardName(state, c));
      }
      log(`milled ${names.length} from ${poss(state, ctx, intent.seat)} library: ${names.join(', ')}`);
      break;
    }
    case 'exileTop': {
      checkSeat(state, intent.seat);
      if (state.settings.ownersOnly && intent.seat !== ctx.actor) fail('Only the owner can do that in this game');
      const lib = state.seats[intent.seat].zones.library;
      const names: string[] = [];
      for (let i = 0; i < intent.n && lib.length; i++) {
        const c = card(lib[0]);
        moveCard(state, ctx, c.iid, c.owner, 'exile', { faceDown: intent.faceDown }, notices);
        names.push(cardName(state, c));
      }
      if (intent.faceDown)
        log(`exiled the top ${names.length} of ${poss(state, ctx, intent.seat)} library face down: ${names.join(', ')}`, {
          redacted: `exiled the top ${names.length} of ${poss(state, ctx, intent.seat)} library face down`,
          privateTo: [ctx.actor, intent.seat].filter((s) => s >= 0),
        });
      else log(`exiled the top ${names.length} of ${poss(state, ctx, intent.seat)} library: ${names.join(', ')}`);
      break;
    }
    case 'shuffle': {
      checkSeat(state, intent.seat);
      if (state.settings.ownersOnly && intent.seat !== ctx.actor) fail('Only the owner can do that in this game');
      shuffleInPlace(state.seats[intent.seat].zones.library, resolved.perm);
      rekey(state, intent.seat);
      log(`shuffled ${poss(state, ctx, intent.seat)} library`);
      notices.push({ to: 'all', kind: 'sound', sound: 'shuffle' });
      break;
    }
    case 'revealTop': {
      checkSeat(state, intent.seat);
      const top = state.seats[intent.seat].zones.library[0];
      if (!top) fail('The library is empty');
      log(`revealed ${cardName(state, card(top))} from the top of ${poss(state, ctx, intent.seat)} library`);
      notices.push({ to: 'all', kind: 'cards', mode: 'reveal', seat: intent.seat, zone: 'library', iids: [top], title: `Top of ${seatName(state, intent.seat)}'s library` });
      break;
    }
    case 'toggleRevealTop': {
      checkSeat(state, intent.seat);
      const s = state.seats[intent.seat];
      s.revealTop = !s.revealTop;
      log(`${s.revealTop ? 'is now playing with the top of' : 'stopped revealing the top of'} ${poss(state, ctx, intent.seat)} library${s.revealTop ? ' revealed' : ''}`);
      break;
    }
    case 'search': {
      const me = requireSeat(ctx);
      checkSeat(state, intent.seat);
      const z = intent.zone ?? 'library';
      if (state.settings.ownersOnly && intent.seat !== me && HIDDEN_ZONES.includes(z)) fail('Only the owner can do that in this game');
      const ids = state.seats[intent.seat].zones[z].slice();
      if (HIDDEN_ZONES.includes(z)) log(`is searching ${poss(state, ctx, intent.seat)} ${ZONE_LABEL[z]}`);
      notices.push({ to: me, kind: 'cards', mode: 'search', seat: intent.seat, zone: z, iids: ids, title: `${intent.seat === me ? 'Your' : seatName(state, intent.seat) + "'s"} ${ZONE_LABEL[z]}` });
      break;
    }
    case 'randomCard': {
      checkSeat(state, intent.seat);
      const arr = state.seats[intent.seat].zones[intent.zone];
      const i = resolved.index ?? -1;
      if (i < 0 || i >= arr.length) {
        log(`picked a random card from ${poss(state, ctx, intent.seat)} ${ZONE_LABEL[intent.zone]}, but it was empty`);
        break;
      }
      const c = card(arr[i]);
      const nm = cardName(state, c);
      if (intent.to) {
        const toSeat = intent.to === 'hand' ? (intent.zone === 'hand' && intent.seat !== ctx.actor ? requireSeat(ctx) : c.owner) : c.owner;
        const m = moveCard(state, ctx, c.iid, toSeat, intent.to, {}, notices);
        const verb =
          intent.zone === 'hand' && intent.to === 'graveyard'
            ? `made ${intent.seat === ctx.actor ? 'themselves' : seatName(state, intent.seat)} discard at random: ${nm}`
            : intent.zone === 'hand' && intent.to === 'hand'
              ? `took a random card from ${seatName(state, intent.seat)}'s hand`
              : `moved a random card (${nm}) from ${poss(state, ctx, intent.seat)} ${ZONE_LABEL[intent.zone]} to ${ZONE_LABEL[intent.to]}`;
        if (intent.zone === 'hand' && intent.to === 'hand') log(`${verb} (${nm})`, { redacted: verb, privateTo: [ctx.actor, intent.seat] });
        else log(verb);
        void m;
      } else {
        const hidden = HIDDEN_ZONES.includes(intent.zone);
        const t = `picked ${nm} at random from ${poss(state, ctx, intent.seat)} ${ZONE_LABEL[intent.zone]}`;
        if (hidden) log(t, { redacted: `picked a random card from ${poss(state, ctx, intent.seat)} ${ZONE_LABEL[intent.zone]}`, privateTo: [ctx.actor] });
        else log(t);
      }
      notices.push({ to: 'all', kind: 'sound', sound: 'dice' });
      break;
    }

    // ---------------------------------------------------------- hands
    case 'revealHand': {
      const me = requireSeat(ctx);
      const s = state.seats[me];
      if (intent.seat != null) {
        checkSeat(state, intent.seat);
        if (intent.value && !s.handViewers.includes(intent.seat)) s.handViewers.push(intent.seat);
        if (!intent.value) s.handViewers = s.handViewers.filter((x) => x !== intent.seat);
        log(intent.value ? `revealed their hand to ${seatName(state, intent.seat)} (live until revoked)` : `stopped revealing their hand to ${seatName(state, intent.seat)}`);
        break;
      }
      s.handRevealAll = intent.value;
      log(intent.value ? 'revealed their hand to everyone' : 'stopped revealing their hand');
      break;
    }
    case 'requestHandView': {
      const me = requireSeat(ctx);
      checkSeat(state, intent.seat);
      if (intent.seat === me) fail('That is your own hand');
      if (state.seats[intent.seat].handViewers.includes(me)) fail('You can already see that hand');
      if (!state.requests.some((r) => r.from === me && r.to === intent.seat)) {
        const id = `hv${me}-${intent.seat}`;
        state.requests.push({ id, from: me, to: intent.seat });
        notices.push({ to: intent.seat, kind: 'handRequest', from: me, id });
      }
      log(`asked to see ${seatName(state, intent.seat)}'s hand`);
      break;
    }
    case 'answerHandView': {
      const me = requireSeat(ctx);
      const r = state.requests.find((x) => x.id === intent.id);
      if (!r) fail('That request is gone');
      if (r.to !== me) fail('Not your request to answer');
      state.requests = state.requests.filter((x) => x.id !== intent.id);
      if (intent.grant) {
        if (!state.seats[me].handViewers.includes(r.from)) state.seats[me].handViewers.push(r.from);
        log(`let ${seatName(state, r.from)} view their hand (live until revoked)`);
        notices.push({ to: r.from, kind: 'toast', text: `${seatName(state, me)} approved — you can see their hand` });
      } else {
        log(`declined ${seatName(state, r.from)}'s request to see their hand`);
        notices.push({ to: r.from, kind: 'toast', text: `${seatName(state, me)} declined your request` });
      }
      break;
    }
    case 'revokeHandView': {
      const me = requireSeat(ctx);
      const s = state.seats[me];
      s.handViewers = intent.seat == null ? [] : s.handViewers.filter((v) => v !== intent.seat);
      log(intent.seat == null ? 'revoked all hand views' : `stopped letting ${seatName(state, intent.seat)} see their hand`);
      break;
    }
    case 'stopViewing': {
      const me = requireSeat(ctx);
      checkSeat(state, intent.seat);
      const s = state.seats[intent.seat];
      s.handViewers = s.handViewers.filter((v) => v !== me);
      log(`stopped viewing ${seatName(state, intent.seat)}'s hand`);
      break;
    }

    // ---------------------------------------------------------- players
    case 'life': {
      checkSeat(state, intent.seat);
      const s = state.seats[intent.seat];
      const from = s.life;
      if (intent.set != null) s.life = intent.set;
      else s.life += intent.delta ?? 0;
      if (s.life === from) break;
      s.lifeHist.push(s.life);
      if (s.lifeHist.length > 50) s.lifeHist.splice(0, s.lifeHist.length - 50);
      logs.push({
        kind: 'life',
        text: `set ${intent.seat === ctx.actor ? 'their' : seatName(state, intent.seat) + "'s"} life ${from} → ${s.life}`,
        life: { target: intent.seat, from, to: s.life },
      });
      break;
    }
    case 'playerCounter': {
      checkSeat(state, intent.seat);
      const s = state.seats[intent.seat];
      const v = Math.max(0, (s.counters[intent.key] || 0) + intent.delta);
      s.counters[intent.key] = v;
      log(`set ${intent.seat === ctx.actor ? 'their' : seatName(state, intent.seat) + "'s"} ${intent.key} to ${v}`);
      break;
    }
    case 'cmdTax': {
      checkSeat(state, intent.seat);
      const s = state.seats[intent.seat];
      const v = Math.max(0, (s.cmdTax[intent.iid] || 0) + intent.delta);
      s.cmdTax[intent.iid] = v;
      log(`set ${poss(state, ctx, intent.seat)} commander tax${state.cards[intent.iid] ? ' for ' + cardName(state, state.cards[intent.iid]) : ''} to +${v}`);
      break;
    }
    case 'cmdDmg': {
      checkSeat(state, intent.seat);
      const s = state.seats[intent.seat];
      const v = Math.max(0, (s.cmdDmg[intent.source] || 0) + intent.delta);
      if (v) s.cmdDmg[intent.source] = v;
      else delete s.cmdDmg[intent.source];
      const src = state.cards[intent.source];
      log(`set commander damage to ${intent.seat === ctx.actor ? 'themselves' : seatName(state, intent.seat)} from ${src ? cardName(state, src) : 'a commander'} to ${v}`);
      break;
    }
    case 'concede': {
      const me = requireSeat(ctx);
      state.seats[me].conceded = true;
      log('conceded — their cards stay on the table');
      break;
    }
    case 'layout': {
      const me = requireSeat(ctx);
      const s = state.seats[me];
      if (intent.handFrac != null) s.handFrac = Math.round(intent.handFrac * 100) / 100;
      if (intent.cardScale != null) s.cardScale = Math.round(intent.cardScale * 100) / 100;
      break;
    }
    case 'autoUntap': {
      const me = requireSeat(ctx);
      state.seats[me].autoUntap = intent.value;
      break;
    }
    case 'autoDraw': {
      const me = requireSeat(ctx);
      state.seats[me].autoDraw = intent.value;
      break;
    }
    case 'marker': {
      if (intent.seat != null) checkSeat(state, intent.seat);
      state.global[intent.key] = intent.seat;
      const what = intent.key === 'monarch' ? 'the Monarch' : 'the Initiative';
      if (intent.seat == null) log(`removed ${what}`);
      else log(intent.seat === ctx.actor ? `became ${intent.key === 'monarch' ? 'the Monarch' : 'the holder of the Initiative'}` : `gave ${what} to ${seatName(state, intent.seat)}`);
      if (intent.key === 'initiative' && intent.seat != null) {
        const s = state.seats[intent.seat];
        if (!s.dungeon || s.dungeon.name !== 'Undercity') {
          s.dungeon = { name: 'Undercity', room: 0 };
          log(`${seatName(state, intent.seat)} ventured into the Undercity (Secret Entrance)`);
        }
      }
      break;
    }
    case 'day': {
      state.global.day = intent.value;
      log(intent.value ? `made it ${intent.value}` : 'removed the day/night marker');
      break;
    }
    case 'citysBlessing': {
      checkSeat(state, intent.seat);
      state.seats[intent.seat].citysBlessing = intent.value;
      log(`${intent.value ? 'gave' : 'removed'} the city's blessing ${intent.value ? 'to' : 'from'} ${seatName(state, intent.seat)}`);
      break;
    }
    case 'ring': {
      checkSeat(state, intent.seat);
      const s = state.seats[intent.seat];
      s.ring = Math.max(0, Math.min(4, s.ring + intent.delta));
      log(`set ${poss(state, ctx, intent.seat)} ring level to ${s.ring}${intent.delta > 0 ? ' (the Ring tempts)' : ''}`);
      break;
    }
    case 'customMarker': {
      const id = `${intent.seat ?? 't'}:${intent.label.toLowerCase()}`;
      const m = state.global.markers.find((x) => x.id === id);
      if (intent.remove) {
        state.global.markers = state.global.markers.filter((x) => x.id !== id);
        log(`removed the “${intent.label}” marker`);
      } else if (m) {
        m.value += intent.delta ?? 1;
        log(`set the “${intent.label}” marker to ${m.value}`);
      } else {
        state.global.markers.push({ id, seat: intent.seat, label: intent.label, value: intent.delta ?? 0 });
        log(`added a “${intent.label}” marker${intent.seat != null ? ' to ' + seatName(state, intent.seat) : ' to the table'}`);
      }
      break;
    }
    case 'dungeon': {
      checkSeat(state, intent.seat);
      const s = state.seats[intent.seat];
      if (!intent.name) {
        s.dungeon = null;
        log(`completed or removed ${poss(state, ctx, intent.seat)} dungeon`);
        break;
      }
      const rooms = DUNGEONS[intent.name];
      if (!rooms) fail('Unknown dungeon');
      s.dungeon = { name: intent.name, room: Math.min(intent.room, rooms.length - 1) };
      log(`${intent.seat === ctx.actor ? 'ventured' : `moved ${seatName(state, intent.seat)}`} into ${rooms[s.dungeon.room].name} (${intent.name})`);
      break;
    }

    // ---------------------------------------------------------- turn
    case 'phase': {
      const before = state.turn.phase;
      state.turn.phase = intent.phase;
      enteredPhase(state, before, logs, notices, ctx.actorName);
      break;
    }
    case 'passTurn': {
      const n = state.seats.length;
      const cur = state.turn.active;
      let next = (cur + 1) % n;
      for (let k = 0; k < n && (state.seats[next].conceded || !state.seats[next].connected) && next !== cur; k++) next = (next + 1) % n;
      for (const c of Object.values(state.cards)) {
        c.effects = c.effects.filter((e) => !e.eot);
        c.ptEot = { p: 0, t: 0 };
      }
      state.turn.active = next;
      state.turn.n += 1;
      state.turn.phase = 0;
      state.table.planarRolls = 0;
      log(`passed the turn to ${seatName(state, next)}`);
      const p = state.seats[next];
      // Turn-start automation is per player: untap (on by default) and draw (off by default).
      if (p.autoUntap) for (const iid of p.zones.battlefield) if (!state.cards[iid].noUntap) state.cards[iid].tapped = false;
      if (p.autoDraw) {
        const top = p.zones.library[0];
        const did = p.autoUntap ? 'untapped and drew' : 'drew';
        if (top) {
          const c = state.cards[top];
          moveCard(state, { ...ctx, actor: next }, top, next, 'hand', {}, notices);
          logs.push({ kind: 'act', text: `${p.name} ${did} ${cardName(state, c)}`, redacted: `${p.name} ${did} 1 card`, privateTo: [next] });
        } else log(`${p.name} ${p.autoUntap ? 'untapped' : 'could not draw'} (library is empty)`);
        state.turn.phase = 3;
      } else if (p.autoUntap) {
        log(`${p.name} untapped`);
        // Not drawing automatically: start in the draw step so the draw isn't forgotten.
        state.turn.phase = 2;
      }
      notices.push({ to: next, kind: 'yourTurn', seat: next });
      enteredPhase(state, 0, logs, notices, ctx.actorName);
      break;
    }
    case 'roll': {
      log(`rolled a d${intent.sides}: ${resolved.value}`);
      notices.push({ to: 'all', kind: 'toast', text: `${ctx.actorName} rolled d${intent.sides} → ${resolved.value}` });
      notices.push({ to: 'all', kind: 'sound', sound: 'dice' });
      break;
    }
    case 'coin': {
      const r = resolved.value === 0 ? 'heads' : 'tails';
      log(`flipped a coin: ${r}`);
      notices.push({ to: 'all', kind: 'toast', text: `${ctx.actorName} flipped a coin → ${r}` });
      notices.push({ to: 'all', kind: 'sound', sound: 'dice' });
      break;
    }
    case 'planarRoll': {
      // the first roll in a turn is free; each roll after it costs one more generic mana (paid by hand)
      const cost = state.table.planarRolls ?? 0;
      state.table.planarRolls = cost + 1;
      const face = resolved.value === 4 ? 'chaos' : resolved.value === 5 ? 'planeswalk' : 'blank';
      log(`rolled the planar die${cost ? ` (paid {${cost}})` : ''}: ${face}`);
      notices.push({ to: 'all', kind: 'sound', sound: 'dice' });
      if (face === 'chaos') {
        const chaos = state.table.planes.map((iid) => chaosText(state.defs[state.cards[iid]?.def])).filter(Boolean);
        notices.push({ to: 'all', kind: 'toast', text: chaos.length ? `{CHAOS} → ${chaos.join(' · ')}` : 'Planar die → chaos' });
        if (chaos.length) logs.push({ kind: 'system', text: `Chaos: ${chaos.join(' · ')}` });
      } else notices.push({ to: 'all', kind: 'toast', text: `Planar die → ${face}` });
      if (face === 'planeswalk') planeswalk(state, logs, notices);
      break;
    }
    case 'planeswalk': {
      planeswalk(state, logs, notices, intent.extra);
      break;
    }
    case 'setInMotion': {
      setInMotion(state, intent.seat, logs, notices, ctx.actorName);
      break;
    }

    // ---------------------------------------------------------- tokens
    case 'createToken': {
      checkSeat(state, intent.seat);
      let key = intent.def;
      if (intent.custom) {
        const cu = intent.custom;
        key = `t:${cu.name}|${cu.pt}|${cu.colors.join('')}`;
        if (!state.defs[key]) {
          const pt = cu.pt.split('/');
          state.defs[key] = {
            key,
            name: cu.name,
            layout: 'token',
            manaCost: '',
            cmc: 0,
            typeLine: `Token ${cu.type || 'Creature'} — ${cu.name}`,
            oracleText: '',
            power: pt.length === 2 ? pt[0] : undefined,
            toughness: pt.length === 2 ? pt[1] : undefined,
            colors: cu.colors,
            colorIdentity: cu.colors,
            img: [],
            isToken: true,
            custom: true,
          } satisfies CardDef;
        }
      } else if (key && resolved.defs?.[key]) state.defs[key] = resolved.defs[key];
      if (!key || !state.defs[key]) fail('Unknown token');
      const n = intent.count ?? 1;
      const already = state.seats[intent.seat].zones.battlefield.filter((i) => state.cards[i]?.token).length;
      for (let i = 0; i < n; i++) {
        const k = already + i;
        const c = makeCard(state, key, intent.seat, { token: true, x: intent.x ?? 0.5 + ((k * 0.07) % 0.45), y: intent.y ?? 0.4 + (Math.floor(k / 7) % 3) * 0.12 });
        state.seats[intent.seat].zones.battlefield.push(c.iid);
      }
      const nm = state.defs[key].name + (state.defs[key].power != null ? ` ${state.defs[key].power}/${state.defs[key].toughness}` : '');
      log(`created ${n === 1 ? 'a' : n} ${nm} token${n === 1 ? '' : 's'}${intent.seat !== ctx.actor ? ` for ${seatName(state, intent.seat)}` : ''}`);
      break;
    }

    // ---------------------------------------------------------- mulligan
    case 'mulligan': {
      const me = requireSeat(ctx);
      const s = state.seats[me];
      const m = (s.mulligan = s.mulligan || { count: 0, kept: false });
      if (m.kept) fail('You already kept');
      if (state.settings.mulligan === 'free-scry3') {
        // the hand and the scried cards still on top go to the bottom in a random order (any already put on the bottom
        // stay where they are), then a fresh seven
        const gone = [...s.zones.hand, ...s.zones.library.splice(0, m.scryLeft ?? 3)];
        m.scryLeft = undefined;
        m.scried = undefined;
        const perm = resolved.perm ?? [];
        const order = gone.map((iid, i) => ({ iid, k: perm[i] ?? i })).sort((a, b) => a.k - b.k);
        s.zones.hand = [];
        s.zones.library.push(...order.map((x) => x.iid));
        rekey(state, me);
        m.count += 1;
        for (let i = 0; i < 7 && s.zones.library.length; i++) s.zones.hand.push(s.zones.library.shift()!);
        log(`took a mulligan (${m.count}): put ${gone.length} cards on the bottom and drew 7`);
        break;
      }
      const all = [...s.zones.hand, ...s.zones.library];
      s.zones.hand = [];
      s.zones.library = all;
      shuffleInPlace(s.zones.library, resolved.perm);
      rekey(state, me);
      m.count += 1;
      const rule = state.settings.mulligan;
      const draw = rule === 'vancouver' || rule === 'paris' ? Math.max(0, 7 - m.count) : 7;
      for (let i = 0; i < draw && s.zones.library.length; i++) s.zones.hand.push(s.zones.library.shift()!);
      log(`took a mulligan (${m.count}) and drew ${draw}`);
      notices.push({ to: 'all', kind: 'sound', sound: 'shuffle' });
      break;
    }
    case 'keep': {
      const me = requireSeat(ctx);
      const s = state.seats[me];
      const m = (s.mulligan = s.mulligan || { count: 0, kept: false });
      if (m.kept) fail('You already kept');
      const rule = state.settings.mulligan;
      const need = rule === 'london' ? m.count : rule === 'london-free' ? Math.max(0, m.count - 1) : 0;
      if (intent.bottom.length !== need) fail(`Put ${need} card${need === 1 ? '' : 's'} on the bottom`);
      for (const iid of intent.bottom) {
        const i = s.zones.hand.indexOf(iid);
        if (i < 0) fail('That card is not in your hand');
        s.zones.hand.splice(i, 1);
        s.zones.library.push(iid);
      }
      m.kept = true;
      log(m.count ? `kept ${s.zones.hand.length} after ${plural(m.count, 'mulligan')}${need ? ` (put ${need} on the bottom)` : ''}` : 'kept 7');
      if (rule === 'vancouver' && m.count > 0 && s.zones.hand.length < 7) notices.push({ to: me, kind: 'vancouverScry' });
      if (rule === 'free-scry3' && !m.scried && (m.scryLeft ?? 3) > 0 && s.zones.library.length) notices.push({ to: me, kind: 'vancouverScry', n: m.scryLeft ?? 3 });
      if (state.seats.every((x) => x.conceded || x.mulligan?.kept)) {
        state.stage = 'play';
        logs.push({ kind: 'system', text: 'Everyone has kept. Play begins.' });
      }
      break;
    }

    case 'chat': {
      logs.push({ kind: 'chat', text: intent.text });
      break;
    }
    default: {
      const t: never = intent;
      fail(`Unknown action ${(t as { type: string }).type}`);
    }
  }
  return { logs, notices };
}

/**
 * Planeswalk: the face-up plane(s) go to the bottom of the planar deck and the top card is turned over. `extra` keeps
 * the current planes and adds one (Spatial Merging, Interplanar Tunnel). A phenomenon is encountered: its text is shown,
 * and the planar controller planeswalks on once it is resolved.
 */
function planeswalk(state: GameState, logs: LogDraft[], notices: Notice[], extra = false) {
  const t = state.table;
  if (!t.planarDeck.length && !t.planes.length) throw new IntentError('There is no planar deck');
  if (!extra) {
    t.planarDeck.push(...t.planes);
    t.planes = [];
  }
  const next = t.planarDeck.shift();
  if (next) t.planes.push(next);
  const def = next ? state.defs[state.cards[next]?.def] : undefined;
  const phenomenon = !!def && /\bPhenomenon\b/.test(def.typeLine);
  t.encounter = phenomenon ? next! : null;
  logs.push({ kind: 'act', text: `${extra ? 'added another plane:' : phenomenon ? 'encountered' : 'planeswalked to'} ${next ? cardName(state, state.cards[next]) : 'nowhere'}` });
  if (phenomenon && def) notices.push({ to: 'all', kind: 'toast', text: `Phenomenon: ${def.name} — ${def.oracleText.split('\n')[0]} Then planeswalk on.` });
}

/** The chaos ability of a plane ("Whenever you roll {CHAOS}, …"). */
function chaosText(d: CardDef | undefined): string {
  if (!d) return '';
  const m = d.oracleText.match(/Whenever you roll \{CHAOS\}, ([^\n]+)/);
  return m ? `${d.name}: ${m[1]}` : '';
}

/**
 * Set the top scheme of an archenemy's scheme deck in motion. One-shot schemes set in motion earlier go to the
 * bottom first (ongoing schemes stay until abandoned).
 */
function setInMotion(state: GameState, seat: number, logs: LogDraft[], notices: Notice[], by: string) {
  checkSeat(state, seat);
  const s = state.seats[seat];
  const done = s.zones.schemes.filter((iid) => !/\bOngoing\b/.test(state.defs[state.cards[iid]?.def]?.typeLine ?? ''));
  for (const iid of done) {
    s.zones.schemes.splice(s.zones.schemes.indexOf(iid), 1);
    s.zones.schemeDeck.push(iid);
    state.cards[iid].faceDown = false;
  }
  const top = s.zones.schemeDeck[0];
  if (!top) throw new IntentError('The scheme deck is empty');
  s.zones.schemeDeck.shift();
  s.zones.schemes.push(top);
  logs.push({ kind: 'act', text: `${by === s.name ? '' : `${s.name}: `}set ${cardName(state, state.cards[top])} in motion` });
  notices.push({ to: 'all', kind: 'cards', mode: 'reveal', seat, zone: 'schemes', iids: [top], title: 'Scheme set in motion' });
}

/** Turn-structure automation: an archenemy sets a scheme in motion at the start of their first main phase. */
function enteredPhase(state: GameState, before: number, logs: LogDraft[], notices: Notice[], by: string) {
  const MAIN = 3;
  const s = state.seats[state.turn.active];
  if (state.turn.phase === MAIN && before < MAIN && s?.archenemy && state.settings.archenemy && s.zones.schemeDeck.length) setInMotion(state, state.turn.active, logs, notices, by);
}

export function isTopRowSeat(seat: number, n: number): boolean {
  const rows: Record<number, [number, number]> = { 1: [0, 1], 2: [1, 1], 3: [2, 1], 4: [2, 2], 5: [3, 2], 6: [3, 3], 7: [4, 3], 8: [4, 4] };
  return seat >= 1 && seat <= (rows[n] || [1, 1])[0];
}

export type { Seat };

/** Cards that venture into the dungeon, take the Initiative or complete dungeons. */
export function usesDungeons(d: CardDef | undefined): boolean {
  if (!d) return false;
  const text = [d.oracleText, ...(d.faces ?? []).map((f) => f.oracleText ?? '')].join('\n');
  return /venture into the dungeon|the initiative|complete(d)? a dungeon|Undercity/i.test(text);
}
