// Turns a rules-game view (from the native engine) into the GameView the app's table renders, and maps the table's gestures
// onto rules actions (a card dropped on the battlefield is played, double-clicking a permanent taps it for mana, ...).
import type { CardDef, CardFace, CardInst, CardView, GameView, LogView, Seat, rules as R } from '@aetherhub/shared';

export type RView = R.View;
export type RCard = R.CardView;
export type REvent = R.Event;

const COLORS = ['#4a90e2', '#e2724a', '#58b368', '#b36fd1', '#d1b84a', '#4ac1c1', '#d14a8a', '#8a8a8a'];

export const STEP_NAMES = ['Untap', 'Upkeep', 'Draw', 'Main1', 'BeginCombat', 'DeclareAttackers', 'DeclareBlockers', 'FirstStrikeDamage', 'CombatDamage', 'EndCombat', 'Main2', 'End', 'Cleanup'] as const;
/** Index of an engine step in the engine's step list (what `passuntil` takes). */
export const stepIndex = (s: string): number => Math.max(0, STEP_NAMES.indexOf(s as (typeof STEP_NAMES)[number]));
/** The table's top-bar phases: untap, upkeep, draw, main 1, combat, main 2, end. */
export const PHASE_INDEX: Record<string, number> = {
  Untap: 0, Upkeep: 1, Draw: 2, Main1: 3, BeginCombat: 4, DeclareAttackers: 4, DeclareBlockers: 4, FirstStrikeDamage: 4, CombatDamage: 4, EndCombat: 4, Main2: 5, End: 6, Cleanup: 6,
};
/** Engine step to stop at for each top-bar phase. */
export const PHASE_STEP = ['Untap', 'Upkeep', 'Draw', 'Main1', 'BeginCombat', 'Main2', 'End'] as const;

// A spell on the stack is shown on its controller's battlefield (tagged "On the stack") so it never vanishes between the hand and where it ends up.
const ZONES: Record<string, 'battlefield' | 'graveyard' | 'exile' | 'command' | 'hand' | 'sideboard' | 'schemes'> = { Battlefield: 'battlefield', Stack: 'battlefield', Graveyard: 'graveyard', Exile: 'exile', Command: 'command', Hand: 'hand', Sideboard: 'sideboard' };

const COUNTER_NAMES: Record<string, string> = {
  PlusOnePlusOne: '+1/+1', MinusOneMinusOne: '-1/-1', Loyalty: 'loyalty', Defense: 'defense', Charge: 'charge', Poison: 'poison', Energy: 'energy', Experience: 'experience', Time: 'time', Lore: 'lore',
  Shield: 'shield', Stun: 'stun', Flying: 'flying', Level: 'level', Age: 'age', Fade: 'fade', Quest: 'quest', Verse: 'verse', Other: 'counter',
};

/** The engine's counter kind for a table counter name ("+1/+1" → PlusOnePlusOne, "oil" → Oil). */
export function counterKindOf(name: string): string {
  const known = Object.entries(COUNTER_NAMES).find(([, n]) => n === name)?.[0];
  return known ?? name.replace(/(^|[\s_-])(\w)/g, (_, __, c: string) => c.toUpperCase()).replace(/[^A-Za-z]/g, '');
}

function inst(iid: string, def: string, owner: number, over: Partial<CardInst> = {}): CardView {
  return {
    iid, def, owner, faceDown: false, tapped: false, x: 0.5, y: 0.5, face: 0, flipped: false, counters: {}, effects: [], pt: { p: 0, t: 0 }, ptEot: { p: 0, t: 0 },
    pile: 0, noUntap: false, token: false, attachedTo: null, melded: null, commander: false, companion: false, peekers: [], ...over,
  };
}

/** Where each permanent sits on its controller's battlefield; kept between views so cards stay where they are (or where the player dragged them). */
export type Placed = Map<number, { x: number; y: number }>;
export type Remembered = Map<number, { x: number; y: number; name: string }>;

/** Default board card size: small enough that the battlefield holds three rows of cards (creatures, other, lands) with room between them. */
export const BOARD_CARD_SCALE = 0.8;
/** Sideways offset between the cards of a pile of identical permanents (a fraction of the board width): piles fan out horizontally. */
export const PILE_DX = 0.014;
/** A pile made by sorting lies exactly on top of itself, with a count, from this many cards. */
export const TIGHT_PILE = 7;
/** A pile of permanents kept together on the table: `fan` partly covered, `tight` exactly on top of each other with a
 *  count, `auto` (made by sorting) fanned until it holds `TIGHT_PILE` cards, then tight. */
export type Pile = { ids: number[]; mode: 'auto' | 'fan' | 'tight' };
export const pileTight = (p: Pile, n = p.ids.length): boolean => p.mode === 'tight' || (p.mode === 'auto' && n >= TIGHT_PILE);
/** What makes permanents identical for piling: same name, token or not, face-down or not, same power and toughness. */
const pileKey = (c: RCard): string => `${c.name}|${c.token}|${c.face_down}|${c.power}/${c.toughness}`;
const ROW_Y = { creature: 0, other: 0.5, land: 1 };

/** A board card's size in board units (positions run 0..1 over the room the card can move in): `pitch` the width of a
 *  tapped card plus a gap, `wide` an untapped card's width plus a gap, `tall` an untapped card's height plus a gap. */
function slotGeom(seats: number): { pitch: number; wide: number; tall: number } {
  const W = typeof window === 'undefined' ? 1400 : window.innerWidth;
  const H = typeof window === 'undefined' ? 900 : window.innerHeight;
  const cols = seats <= 2 ? 1 : 2;
  const rows = seats <= 2 ? seats : Math.ceil(seats / 2);
  const bfW = Math.max(400, W / cols - 140);
  const bfH = ((H - 48) / rows) * (1 - 0.28);
  const ch = Math.min(bfH * 0.34 * BOARD_CARD_SCALE, bfH - 16);
  const w = (ch * 63) / 88;
  const xs = Math.max(1, bfW - 12 - w);
  // a tapped card is as wide as the card is tall; leave a gap so it never covers its neighbour
  return { pitch: (ch + 10) / xs, wide: (w + 8) / xs, tall: (ch + 4) / Math.max(1, bfH - 12 - ch) };
}

function slotPitch(seats: number): number {
  return slotGeom(seats).pitch;
}

/** Finds the seat whose side of the table a permanent sits on, as a real table would show it: a battle's protector, a
 *  Curse's enchanted player, an attached permanent's host's side (an Aura on an opponent's creature lies with that
 *  creature), else its controller. */
export function sides(cards: readonly RCard[]): (c: RCard) => number {
  const byId = new Map(cards.map((c) => [c.id, c]));
  const side = (c: RCard, depth: number): number => {
    if (c.protector != null) return c.protector;
    if (c.enchanted_player != null) return c.enchanted_player;
    const host = c.attached_to != null && depth < 8 ? byId.get(c.attached_to) : undefined;
    if (host && host.zone === 'Battlefield') return side(host, depth + 1);
    return c.controller;
  };
  return (c) => side(c, 0);
}

const rowY = (c: { types: string }) => (/Land/.test(c.types) ? ROW_Y.land : /Creature/.test(c.types) ? ROW_Y.creature : ROW_Y.other);

/** Gives permanents that have no position yet a free spot: the first free slot in their row, then in the other rows, then
 *  anywhere on the board; only a completely full board puts a card over another, offset so the one below still shows. */
function settle(list: RCard[], placed: Placed, seatCount: number, memory?: Remembered): void {
  const here = new Set(list.map((c) => c.id));
  for (const id of [...placed.keys()]) if (!here.has(id)) placed.delete(id);
  const sideOf = sides(list);
  const byController = new Map<number, RCard[]>();
  for (const c of list) byController.set(sideOf(c), [...(byController.get(sideOf(c)) ?? []), c]);
  for (const cards of byController.values()) {
    const { pitch, wide, tall } = slotGeom(seatCount);
    // the spot overlaps a card already there (wherever the player put it): closer than a card's width across (a tapped
    // card's when either is tapped) and a card's height down
    let tappedNew = false;
    const taken = (x: number, y: number) =>
      cards.some((o) => {
        const p = placed.get(o.id);
        return !!p && Math.abs(p.x - x) < (o.tapped || tappedNew ? pitch : wide) && Math.abs(p.y - y) < tall;
      });
    const scanRow = (y: number): number | null => {
      for (let x = 0; x <= 1 + 1e-9; x += pitch / 4) if (!taken(Math.min(1, x), y)) return Math.min(1, x);
      return null;
    };
    for (const c of cards) {
      tappedNew = c.tapped;
      if (placed.has(c.id)) continue;
      // Undo / replay rebuilds the game: a card that was already laid out by the player goes back where they left it
      const m = memory?.get(c.id);
      if (m && m.name === c.name) {
        placed.set(c.id, { x: m.x, y: m.y });
        continue;
      }
      const y0 = rowY(c);
      let spot: { x: number; y: number } | null = null;
      for (const y of [y0, ...[ROW_Y.creature, ROW_Y.other, ROW_Y.land].filter((r) => r !== y0)]) {
        const x = scanRow(y);
        if (x !== null) {
          spot = { x, y };
          break;
        }
      }
      // anywhere on the board (cards the player moved can leave gaps between the rows)
      for (let y = 0; !spot && y <= 1 + 1e-9; y += 0.0625) {
        const x = scanRow(Math.min(1, y));
        if (x !== null) spot = { x, y: Math.min(1, y) };
      }
      if (!spot) {
        // no room at all: on top of a card in its row, shifted so that something is visibly underneath
        const base = cards.map((o) => placed.get(o.id)).find((p) => p && Math.abs(p.y - y0) < 0.2) ?? { x: 0, y: y0 };
        let p = { x: base.x, y: base.y };
        const same = (q: { x: number; y: number }) => cards.some((o) => {
          const r = placed.get(o.id);
          return !!r && Math.abs(r.x - q.x) < 0.005 && Math.abs(r.y - q.y) < 0.005;
        });
        for (let k = 0; k < 40 && same(p); k++) p = { x: Math.min(1, p.x + PILE_DX), y: Math.max(0, Math.min(1, p.y + (y0 >= 0.5 ? -0.03 : 0.03))) };
        spot = p;
      }
      placed.set(c.id, spot);
    }
  }
}

/** "Sort this board": the three automatic rows (creatures, other permanents, lands), with every pile kept together and
 *  laid out as one wide unit. `groups` are the piles there already: a copy of a pile's card that is in no pile joins it
 *  (unless the player took it out of a pile: `loose`), and three or more copies of the same card that are in no pile become
 *  a new pile (`auto`). A tight pile (see `pileTight`) takes one slot, its cards exactly on top of each other. Each card
 *  moves as little as it can: it goes to its row at about where it is now, cards keep their left-to-right order, and only
 *  cards that would overlap are pushed apart; cards with no position yet go after the others in their row. Sorting a
 *  sorted board changes nothing. Returns the position of every card except attached ones, and the piles of this board as
 *  they are now (with the copies that joined them, and the new ones). */
export function layoutBoard(all: RCard[], seatCount: number, groups: Pile[], current?: Placed, loose?: Set<number>): { pos: Map<number, { x: number; y: number }>; piles: Pile[] } {
  const pitch = slotPitch(seatCount);
  // an Aura or Equipment is drawn under its host, so it takes no place of its own in a row
  const here = new Set(all.map((c) => c.id));
  const cards = all.filter((c) => c.attached_to == null || !here.has(c.attached_to));
  const byId = new Map(cards.map((c) => [c.id, c]));
  const used = new Set<number>();
  const piles: Pile[] = [];
  const pileOf = new Map<string, Pile>();
  for (const g of groups) {
    const ids = g.ids.filter((id) => byId.has(id) && !used.has(id));
    if (ids.length < 2) continue;
    const p = { ids, mode: g.mode };
    piles.push(p);
    ids.forEach((id) => used.add(id));
    const k = pileKey(byId.get(ids[0])!);
    if (!pileOf.has(k)) pileOf.set(k, p);
  }
  const same = new Map<string, number[]>();
  for (const c of cards) {
    if (used.has(c.id)) continue;
    const k = pileKey(c);
    const p = pileOf.get(k);
    if (p && !loose?.has(c.id)) {
      p.ids.push(c.id);
      used.add(c.id);
      continue;
    }
    same.set(k, [...(same.get(k) ?? []), c.id]);
  }
  for (const ids of same.values()) {
    if (ids.length < 3) continue;
    piles.push({ ids, mode: 'auto' });
    ids.forEach((id) => used.add(id));
  }
  const units: { u: number[]; tight: boolean }[] = piles.map((p) => ({ u: p.ids, tight: pileTight(p) }));
  for (const c of cards) if (!used.has(c.id)) units.push({ u: [c.id], tight: false });
  // where a unit is now: its leftmost card (none for a card that has no position yet)
  const at = (u: number[]): number | null => {
    const xs = u.map((id) => current?.get(id)).filter((p): p is { x: number; y: number } => !!p && p.x >= 0).map((p) => p.x);
    return xs.length ? Math.min(...xs) : null;
  };
  const pos = new Map<number, { x: number; y: number }>();
  for (const y of [ROW_Y.creature, ROW_Y.other, ROW_Y.land]) {
    const row = units
      .filter(({ u }) => rowY(byId.get(u[0])!) === y)
      .map(({ u, tight }) => ({ u, tight, x: at(u) }))
      .sort((a, b) => (a.x ?? Infinity) - (b.x ?? Infinity) || byId.get(a.u[0])!.name.localeCompare(byId.get(b.u[0])!.name) || a.u[0] - b.u[0]);
    if (!row.length) continue;
    // each unit takes a slot plus the fan of its pile (none for a tight pile)
    const dx = row.map(({ tight }) => (tight ? 0 : PILE_DX));
    const fan = row.map(({ u }, i) => (u.length - 1) * dx[i]);
    const advance = fan.map((f) => pitch + f);
    const needed = advance.slice(0, -1).reduce((a, b) => a + b, 0) + fan[fan.length - 1];
    const xs: number[] = [];
    if (needed > 1) {
      // a row that doesn't fit is squeezed evenly
      const f = 1 / needed;
      let x = 0;
      row.forEach((_, i) => {
        xs.push(x);
        x += advance[i] * f;
      });
      row.forEach(({ u }, i) => u.forEach((id, k) => pos.set(id, { x: Math.min(1, xs[i] + k * dx[i] * f), y })));
      continue;
    }
    // where each unit is, pushed right past the one before it, then back left from the edge
    row.forEach(({ x }, i) => {
      const prev = i > 0 ? xs[i - 1] + advance[i - 1] : 0;
      xs.push(Math.max(prev, x ?? prev));
    });
    for (let i = row.length - 1; i >= 0; i--) xs[i] = Math.min(xs[i], i === row.length - 1 ? 1 - fan[i] : xs[i + 1] - advance[i]);
    row.forEach(({ u }, i) => u.forEach((id, k) => pos.set(id, { x: Math.max(0, Math.min(1, xs[i] + k * dx[i])), y })));
  }
  return { pos, piles };
}

/** Puts the count on the top card of every tight pile (the one drawn last on its battlefield). Display only. */
export function markTightPiles(view: GameView, piles: Pile[]): void {
  for (const p of piles) {
    const iids = p.ids.map(String).filter((iid) => view.cards[iid]);
    if (iids.length < 2 || !pileTight(p, iids.length)) continue;
    for (const s of view.seats) {
      const bf = s.zones.battlefield;
      const top = iids.filter((iid) => bf.includes(iid)).sort((a, b) => bf.indexOf(b) - bf.indexOf(a))[0];
      if (top) view.cards[top].stacked = iids.filter((iid) => bf.includes(iid)).length;
    }
  }
}

/** The engine counts every player's turn (turn 1 = first player, turn 2 = second); the table shows rounds: turn 1 for everyone, then turn 2. */
export function roundOf(turn: number, players: number): number {
  return turn <= 0 ? 0 : Math.floor((turn - 1) / Math.max(1, players)) + 1;
}

const POOL_KEYS = ['W', 'U', 'B', 'R', 'G', 'C'] as const;
export function poolOf(p: R.ManaPool): Record<string, number> {
  const out: Record<string, number> = {};
  POOL_KEYS.forEach((k, i) => (out[k] = p[i] ?? 0));
  return out;
}

/** The name a card's picture goes by: a copy looks like what it copies, and a name-changing effect keeps the picture. */
const ART = (c: { name: string; art?: string }) => c.art || c.name;
const TOKEN_KEY = (c: { name: string; art?: string; colors: number }) => `${ART(c)}|${c.colors}`;
/** Plane and phenomenon cards (Planechase). */
const PLANAR = /\b(Plane|Phenomenon)\b/;
/** The server sends a card's layout (split / Room, planar) in the image map under this key, so previews turn sideways. */
const LAYOUT_KEY = (name: string) => `@layout:${name}`;
/** The picture of a double-faced card's other face rides in the image map under this key. */
const BACK_KEY = (name: string) => `@back:${name}`;
/** ... and its name, cost, type line, text and P/T (a JSON `CardFace`) under this one. */
const BACK_FACE_KEY = (name: string) => `@backface:${name}`;
/** Set for the bottom half's name of a flip card (Rune-Tail's Essence): that permanent is shown turned 180°. */
const FLIPPED_KEY = (name: string) => `@flipped:${name}`;

function parseFace(json: string | undefined): CardFace | undefined {
  if (!json) return undefined;
  try {
    return JSON.parse(json) as CardFace;
  } catch {
    return undefined;
  }
}

/** An optimistic entry with this position has no drop point (a double-click): it takes the first free slot in its row. */
export const AUTO_SLOT = { x: -1, y: -1 } as const;

/** True for a card that stays on the battlefield when played (shown there at once); instants and sorceries are not. */
export function isPermanentCard(c: { types: string }): boolean {
  return /\b(Land|Creature|Artifact|Enchantment|Planeswalker|Battle)\b/.test(c.types) && !/\b(Instant|Sorcery)\b/.test(c.types);
}

export function toGameView(snap: RView, images: Record<string, string>, names: string[], me: number, gameId: string, wins: number[] | undefined, placed: Placed, optimistic: Placed, memory?: Remembered, queued?: Map<number, number>, handOrder?: number[]): { view: GameView } {
  const defs: Record<string, CardDef> = {};
  const cards: Record<string, CardView> = {};
  const defFor = (c: RCard): string => {
    const creature = c.power !== null;
    // no printed P/T (Old Stickfingers' */*, an animated land): the def carries none and the instance carries the whole P/T
    const unprinted = creature && (c.base_power === null || c.base_toughness === null);
    const bp = creature && !unprinted ? c.base_power : null;
    const bt = creature && !unprinted ? c.base_toughness : null;
    const art = (c.art_token ?? c.token) ? images[TOKEN_KEY(c)] : images[ART(c)];
    // img[0] is the face that is up, img[1] the other face (so the card can be turned over to look at it)
    const back = (c.art_token ?? c.token) ? undefined : images[BACK_KEY(ART(c))];
    const backFace = back ? parseFace(images[BACK_FACE_KEY(ART(c))]) : undefined;
    const key = `f:${c.name}|${ART(c)}|${unprinted ? '*' : creature ? bp + '/' + bt : ''}|${c.colors}|${c.token ? 't' : ''}`;
    if (!defs[key]) {
      defs[key] = {
        key, name: c.name, layout: (images[LAYOUT_KEY(ART(c))] as CardDef['layout'] | undefined) ?? 'normal', manaCost: c.cost, cmc: c.mana_value, typeLine: c.types, oracleText: c.text, colors: [], colorIdentity: [], img: art ? (back ? [art, back] : [art]) : [],
        // a token copy says "Copy of ..." itself; the table's generic "Copy" badge is for sandbox tokens made from cards
        isToken: c.token,
        power: bp !== null ? String(bp) : undefined, toughness: bt !== null ? String(bt) : undefined,
        ...(unprinted ? { unprintedPT: true } : {}),
      };
      // faces[0] is the face that is up, as the engine sees it now; faces[1] the printed other face
      if (backFace) {
        const d = defs[key];
        defs[key].faces = [{ name: d.name, manaCost: d.manaCost, typeLine: d.typeLine, oracleText: d.oracleText, power: d.power, toughness: d.toughness }, backFace];
      }
    }
    return key;
  };
  const seats: Seat[] = snap.players.map((p, i) => {
    const zones: Seat['zones'] = { library: [], hand: [], battlefield: [], graveyard: [], exile: [], command: [], sideboard: [], schemeDeck: [], schemes: [], attractionDeck: [], stickers: [] };
    for (let k = 0; k < p.library; k++) {
      const iid = `L${i}:${k}`;
      cards[iid] = { ...inst(iid, '', i), faceDown: true, hidden: true };
      zones.library.push(iid);
    }
    // Archenemy: the scheme deck is face down (only its size is known)
    for (let k = 0; k < p.schemes; k++) {
      const iid = `S${i}:${k}`;
      cards[iid] = { ...inst(iid, '', i), faceDown: true, hidden: true };
      zones.schemeDeck.push(iid);
    }
    const counters: Record<string, number> = {};
    if (p.poison) counters.poison = p.poison;
    if (p.energy) counters.energy = p.energy;
    if (p.experience) counters.experience = p.experience;
    return {
      userId: i, name: names[i] ?? p.name, color: COLORS[i % COLORS.length], deckId: null, deckName: '', sleeve: null, playmat: null, life: p.life, lifeHist: [],
      counters, cmdTax: Object.fromEntries(p.commander_tax.map(([cid, n]) => [String(cid), n])), cmdDmg: Object.fromEntries(p.commander_damage.map(([cid, n]) => [String(cid), n])),
      zones, handFrac: 0.28, cardScale: BOARD_CARD_SCALE, connected: true, conceded: p.lost, revealTop: false,
      manaPool: poolOf(p.pool), handViewers: [], handRevealAll: false, autoUntap: false, dungeon: null, ring: 0, citysBlessing: false, mulligan: null, tokenDefs: [], archenemy: p.schemes > 0 || snap.cards.some((c) => c.zone === 'Command' && c.owner === i && /\bScheme\b/.test(c.types)),
    };
  });
  const bf = new Map<number, RCard[]>();
  const stackSources = new Set(snap.stack.filter((s) => s.is_spell).map((s) => s.source));
  const sideOf = sides(snap.cards);
  for (const c of snap.cards) {
    // the top card of a library that is played revealed (Oracle of Mul Daya) or that its owner may look at (Radar Sense): shown face up on the pile
    if (c.zone === 'Library') {
      const seat = seats[c.owner];
      if (!seat) continue;
      const iid = String(c.id);
      cards[iid] = inst(iid, defFor(c), c.owner);
      delete cards[`L${c.owner}:0`];
      if (seat.zones.library.length) seat.zones.library[0] = iid;
      else seat.zones.library.push(iid);
      seat.revealTop = true;
      continue;
    }
    let z = ZONES[c.zone];
    if (!z) continue;
    // A card the player just dropped on the table is shown there at once, before the engine has ruled on it; if it was
    // not a legal play the entry is removed and the card flies back to the hand.
    const dropped = c.zone === 'Hand' && c.owner === me ? optimistic.get(c.id) : undefined;
    if (dropped) {
      z = 'battlefield';
      // a drop point is kept; a double-click play gets the first free slot (settle, below) and keeps it once it resolves
      if (dropped.x >= 0) placed.set(c.id, dropped);
    }
    const iid = String(c.id);
    const counters: Record<string, number> = {};
    for (const [k, n] of c.counters) counters[COUNTER_NAMES[k] ?? k] = (counters[COUNTER_NAMES[k] ?? k] ?? 0) + n;
    const creature = c.power !== null;
    // an opponent's face-down permanent (the engine masks it: no picture) gets no def, like any card you may not see
    const masked = c.face_down && !c.art;
    cards[iid] = inst(iid, masked ? '' : defFor(c), c.owner, {
      tapped: c.tapped, flipped: z === 'battlefield' && !masked && !!images[FLIPPED_KEY(ART(c))], counters, attachedTo: c.attached_to != null ? String(c.attached_to) : null, faceDown: c.face_down, token: c.token, commander: c.commander,
      pt: creature ? (c.base_power === null || c.base_toughness === null ? { p: c.power ?? 0, t: c.toughness ?? 0 } : { p: (c.power ?? 0) - c.base_power, t: (c.toughness ?? 0) - c.base_toughness }) : { p: 0, t: 0 },
      effects: [
        ...(c.zone === 'Stack' && stackSources.has(c.id) ? [{ k: 'On the stack', eot: false }] : queued?.has(c.id) ? [{ k: `Queued #${queued.get(c.id)}`, eot: false }] : []),
        ...(c.copy_of ? [{ k: `Copy of ${c.copy_of}`, eot: false }] : []),
        // a name that isn't the one on the picture (a copy "except its name is ...", or a name-changing effect)
        ...(c.name !== ART(c) ? [{ k: `Named ${c.name}`, eot: false }] : []),
      ],
    });
    const seat = seats[z === 'battlefield' ? sideOf(c) : c.owner];
    if (!seat) continue;
    // face-up planes are the table's (the top bar), face-up schemes go in the archenemy's scheme row
    if (c.zone === 'Command' && PLANAR.test(c.types)) continue;
    if (c.zone === 'Command' && /\bScheme\b/.test(c.types)) z = 'schemes';
    seat.zones[z].push(iid);
    if (z === 'battlefield') bf.set(sideOf(c), [...(bf.get(sideOf(c)) ?? []), c]);
  }
  // the viewer's hand keeps the order they gave it (updated in place); a card that arrives goes at the end
  if (handOrder && seats[me]) {
    const inHand = snap.cards.filter((c) => c.zone === 'Hand' && c.owner === me).map((c) => c.id);
    const have = new Set(inHand);
    const kept = handOrder.filter((id) => have.has(id));
    const known = new Set(kept);
    handOrder.splice(0, handOrder.length, ...kept, ...inHand.filter((id) => !known.has(id)));
    const shown = new Set(seats[me].zones.hand);
    seats[me].zones.hand = [...handOrder.map(String).filter((iid) => shown.has(iid)), ...seats[me].zones.hand.filter((iid) => !handOrder.includes(Number(iid)))];
  }
  settle([...bf.values()].flat(), placed, snap.players.length, memory);
  if (memory) for (const list of bf.values()) for (const c of list) if (placed.has(c.id)) memory.set(c.id, { ...placed.get(c.id)!, name: c.name });
  for (const list of bf.values()) for (const c of list) Object.assign(cards[String(c.id)], placed.get(c.id));
  snap.players.forEach((p, i) => {
    if (i === me) return;
    for (let k = 0; k < p.hand; k++) {
      const iid = `H${i}:${k}`;
      cards[iid] = { ...inst(iid, '', i), faceDown: true, hidden: true };
      seats[i].zones.hand.push(iid);
    }
  });
  const planarDeck: string[] = [];
  for (let k = 0; k < snap.planar_deck; k++) {
    const iid = `P:${k}`;
    cards[iid] = { ...inst(iid, '', 0), faceDown: true, hidden: true };
    planarDeck.push(iid);
  }
  const planes = snap.cards.filter((c) => c.zone === 'Command' && PLANAR.test(c.types)).map((c) => String(c.id));
  const monarch = snap.players.findIndex((p) => p.monarch);
  const initiative = snap.players.findIndex((p) => p.initiative);
  const view: GameView = {
    v: 1, id: gameId,
    settings: { name: 'Rules game', type: 'rules', format: 'rules', life: 20, mulligan: 'none', bo: 1, maxPlayers: seats.length, spectators: false, specChatShared: false, ownersOnly: false, dungeons: false, planechase: planes.length > 0, planarPool: 'host', archenemy: seats.some((st) => st.archenemy) },
    // Planechase: the face-up plane(s) are command-zone cards; the top bar shows the first one
    seats, cards, defs, table: { planarDeck, planes },
    turn: { active: Math.max(0, snap.active), n: roundOf(snap.turn, snap.players.length), phase: PHASE_INDEX[snap.step] ?? 3 },
    stage: 'play',
    global: { monarch: monarch >= 0 ? monarch : null, initiative: initiative >= 0 ? initiative : null, day: snap.day_night === null ? null : snap.day_night ? 'day' : 'night', markers: [] },
    requests: [],
    match: { game: 1, wins: wins ?? seats.map(() => 0), results: [] }, winner: null, nextIid: 1, startedAt: 0, viewer: me,
  };
  return { view };
}

export interface ROption {
  action: number;
  cardId: number;
  kind: 'land' | 'spell' | 'ability';
  label: string;
}

/** The priority actions as labelled options. */
export function optionsOf(d: R.Decision | null, v: RView | null): ROption[] | null {
  if (!d || d.kind.t !== 'Priority' || !v) return null;
  const byId = new Map(v.cards.map((c) => [c.id, c]));
  return d.kind.actions.map((a, action) => {
    const c = 'card' in a ? byId.get(a.card) : undefined;
    const name = c?.name ?? 'card';
    switch (a.t) {
      case 'PlayLand': return { action, cardId: a.card, kind: 'land' as const, label: `Play ${name}` };
      case 'PlayLandFace': return { action, cardId: a.card, kind: 'land' as const, label: `Play the other face of ${name}` };
      case 'Cast': return { action, cardId: a.card, kind: 'spell' as const, label: castLabel(name, a.alt, c) };
      case 'Activate': return { action, cardId: a.card, kind: 'ability' as const, label: `${name}: ${c?.abilities.find((x) => x.index === a.ability)?.text ?? 'activate'}` };
      case 'ActivateMana': return { action, cardId: a.card, kind: 'ability' as const, label: `${name}: ${c?.abilities.find((x) => x.index === a.ability)?.text ?? 'add mana'}` };
      case 'TurnFaceUp': return { action, cardId: a.card, kind: 'ability' as const, label: `Turn ${name} face up` };
      case 'Companion': return { action, cardId: a.card, kind: 'ability' as const, label: `Put your companion ${name} into your hand ({3})` };
      case 'UnlockDoor': return { action, cardId: a.card, kind: 'ability' as const, label: `Unlock door ${a.door + 1} of ${name}` };
      case 'RollPlanarDie': {
        const cost = v.players[v.you]?.planar_rolls ?? 0;
        const plane = v.cards.find((x) => x.zone === 'Command' && /\b(Plane|Phenomenon)\b/.test(x.types));
        return { action, cardId: plane?.id ?? -1, kind: 'ability' as const, label: `Roll the planar die ${cost ? `({${cost}})` : '(free this turn)'}` };
      }
    }
  });
}

/** What tapping a mana source can yield, one entry per different result: a land that adds {B}{G} at once is one entry, one that
 *  adds {B} or {G} is two. `m` is the colour sent with the tap. */
export function tapChoices(s: R.ManaSourceOpt): { s: R.ManaSourceOpt; m: R.ManaType; text: string }[] {
  const outs = s.outputs.filter((o) => o.length);
  const firsts = outs.map((o) => o[0][0]);
  if (!outs.length || new Set(firsts).size !== outs.length) return s.produces.map((m) => ({ s, m, text: `{${m}}` }));
  return outs.map((o) => ({ s, m: o[0][0], text: o.map(([t, n]) => `{${t}}`.repeat(n)).join('') }));
}

/** "Cast X", or how it is cast: the other face, face down, or for which alternative cost (flashback, evoke, a foretold card's cost...). */
function castLabel(name: string, alt: number | null, c: R.CardView | undefined): string {
  if (alt === null) return `Cast ${name}`;
  if (alt === 255) return `Cast the other face of ${name}`;
  if (alt === 254) return `Cast ${name} face down`;
  // its own alternative costs below 100; ones other objects grant it from 100 up ("{0} (Omniscience)")
  const text = alt < 100 ? c?.alt_costs?.[alt] : c?.granted_alts?.[alt - 100] || undefined;
  if (!text) return `Cast ${name} (alternative cost)`;
  const m = /^from (\w+): (.*)$/.exec(text);
  return m ? `Cast ${name} from ${m[1]} for ${m[2]}` : `Cast ${name} for ${text}`;
}

/** What a table gesture means in a rules game: the index of the priority action to take, or a message. */
export function gestureOption(intent: { type: string; iids?: string[]; zone?: string; activate?: boolean }, options: ROption[] | null): { index: number | null } | { choices: { index: number; label: string }[] } | { message: string } {
  if (!options) return { message: "You can't do that right now" };
  if (intent.type === 'passTurn') return { index: null };
  const iid = intent.iids?.[0];
  const id = iid && /^\d+$/.test(iid) ? Number(iid) : null;
  if (id === null) return { message: "That's a hidden card (a library or an opponent's hand): you can't act on it." };
  // a drag onto the table means play / cast it; a double-click (or Shift+click) on a hand card means any way to use it from there,
  // so a card with foretell, plot, cycling, channel or similar asks which one
  const want: ROption['kind'][] = intent.type === 'tap' ? ['ability'] : intent.type === 'move' && intent.zone === 'battlefield' && !intent.activate ? ['land', 'spell'] : ['spell', 'land', 'ability'];
  if (intent.type === 'move' && intent.zone !== 'battlefield') return { message: "You can't move cards between zones by hand in a rules game: play lands, cast spells and use abilities, and the rules move the cards." };
  if (intent.type !== 'tap' && intent.type !== 'move') return { message: `That table action ("${intent.type}") isn't available in a rules game: the rules engine changes the game, and the table only rearranges cards.` };
  const matches = options.filter((o) => o.cardId === id && want.includes(o.kind));
  if (!matches.length) return { message: "That card can't be played or used right now" };
  // several lands/abilities that do the same thing are interchangeable; only ask when the actions differ
  const distinct = matches.filter((o, i) => matches.findIndex((m) => m.label === o.label) === i);
  if (distinct.length === 1) return { index: distinct[0].action };
  return { choices: distinct.map((o) => ({ index: o.action, label: o.label })) };
}

/** True when `b` differs from `a` only by cards turning from tapped to untapped (the untap step is shown as one frame). */
export function onlyUntaps(a: RView, b: RView): boolean {
  if (a.turn !== b.turn || a.step !== b.step || a.cards.length !== b.cards.length || a.stack.length !== b.stack.length) return false;
  if (a.players.some((p, i) => p.life !== b.players[i]?.life)) return false;
  let untapped = 0;
  const before = new Map(a.cards.map((c) => [c.id, c]));
  const sideA = sides(a.cards);
  const sideB = sides(b.cards);
  for (const c of b.cards) {
    const was = before.get(c.id);
    if (!was || was.zone !== c.zone || sideA(was) !== sideB(c)) return false;
    if (was.tapped && !c.tapped) untapped++;
    else if (was.tapped !== c.tapped) return false;
  }
  return untapped > 0;
}

/** How long to show a frame before the next one: card movement and the stack get time to animate, bare phase changes and draws pass quickly. */
export function frameDelay(prev: RView | null, next: RView): number {
  if (!prev) return 0;
  // before the game starts (opening hands, mulligans) there is nothing to watch but the deal: keep it brisk
  if (next.turn === 0) return 110;
  const before = new Map(prev.cards.map((c) => [c.id, c]));
  const sidePrev = sides(prev.cards);
  const sideNext = sides(next.cards);
  let big = false;
  let draws = 0;
  for (const c of next.cards) {
    const was = before.get(c.id);
    if (!was) {
      if (c.zone === 'Hand') draws++; // a card appearing in the hand is a draw
      else big = true;
    } else if (was.zone !== c.zone || sidePrev(was) !== sideNext(c)) big = true;
  }
  for (const c of prev.cards) if (!next.cards.some((n) => n.id === c.id) && c.zone !== 'Hand') big = true;
  if (big || prev.stack.length !== next.stack.length) return 750;
  if (draws) return 170;
  const detail = (s: RView) => s.cards.map((c) => `${c.tapped ? 1 : 0}${c.power}${c.toughness}${c.counters.length}${c.damage}`).join(',') + s.players.map((p) => p.life).join(',');
  if (detail(prev) !== detail(next)) return 380;
  return 140;
}

/** A readable line for an engine event (for the game log). */
export function describeEvent(e: REvent, v: RView, names: string[], known?: Map<number, string>): string | null {
  // `known`: names learned from earlier events for cards that are hidden again (a one-shot scheme back in its deck)
  if (e.t === 'SchemeSetInMotion') known?.set(e.card, e.name);
  const card = (id: number) => v.cards.find((c) => c.id === id)?.name ?? known?.get(id) ?? 'a card';
  const who = (p: number) => names[p] ?? `Player ${p + 1}`;
  const obj = (o: R.Obj): string => (o.kind === 'Player' ? who(o.id) : o.kind === 'Card' ? card(o.id) : 'a spell');
  switch (e.t) {
    case 'Notice': return `⚠ ${who(e.player)}: ${e.text}`;
    case 'TurnBegin': return `— ${who(e.player)}'s turn ${roundOf(e.turn, names.length)} —`;
    case 'Cast': return `${who(e.player)} casts ${card(e.card)}`;
    case 'LandPlayed': return `${who(e.player)} plays ${card(e.card)}`;
    case 'Activated': return `${who(v.cards.find((c) => c.id === e.card)?.controller ?? e.player)} activates ${card(e.card)}`;
    case 'Moved':
      if (e.from === 'Battlefield' && e.to === 'Graveyard') return `${card(e.card)} dies`;
      if (e.from === 'Battlefield' && e.to === 'Exile') return `${card(e.card)} is exiled`;
      if (e.from === 'Battlefield' && e.to === 'Hand') return `${card(e.card)} returns to hand`;
      if (e.from === 'Stack' && e.to === 'Graveyard') return null;
      if (e.to === 'Battlefield' && e.from !== 'Stack' && e.from !== 'Hand') return `${card(e.card)} enters the battlefield`;
      return null;
    case 'Resolved': return null;
    case 'Countered': return `${card(e.card)} is countered`;
    case 'Fizzled': return `${card(e.card)} fizzles (no legal targets)`;
    case 'Attacked': return `${card(e.card)} attacks ${obj(e.defender)}`;
    case 'Blocked': return `${card(e.blocker)} blocks ${card(e.attacker)}`;
    case 'Damage': return e.amount > 0 ? `${e.source !== null ? card(e.source) : 'Damage'} deals ${e.amount} to ${obj(e.target)}` : null;
    case 'Life': return e.delta > 0 ? `${who(e.player)} gains ${e.delta} life` : `${who(e.player)} loses ${-e.delta} life`;
    case 'Token': return `${card(e.card)} token is created`;
    case 'Counters': return `${e.delta > 0 ? 'Added' : 'Removed'} ${Math.abs(e.delta)} ${COUNTER_NAMES[e.kind] ?? e.kind} counter(s) ${e.delta > 0 ? 'to' : 'from'} ${card(e.card)}`;
    case 'Lost': return `${who(e.player)} loses the game (${e.reason})`;
    case 'Mulligan': return e.keep ? `${who(e.player)} keeps` : `${who(e.player)} takes a mulligan`;
    case 'Shuffled': return null;
    case 'Over': return e.result.t === 'Winner' ? `${who(e.result.player)} wins` : 'The game is a draw';
    case 'Scry': return `${who(e.player)} scries ${e.n}`;
    case 'Attach': return e.to !== null ? `${card(e.card)} attaches to ${card(e.to)}` : null;
    case 'Transformed': return `${card(e.card)} transforms`;
    case 'Monarch': return `${who(e.player)} becomes the monarch`;
    case 'Discarded': return `${who(e.player)} discards ${card(e.card)}`;
    case 'Sacrificed': return `${who(e.player)} sacrifices ${card(e.card)}`;
    case 'PlanarDieRolled': return `${who(e.player)} rolls the planar die: ${e.face === 'Chaos' ? 'chaos' : e.face === 'Planeswalk' ? 'planeswalk' : 'blank'}`;
    case 'ChaosEnsued': return 'Chaos ensues';
    case 'Planeswalked': return `${who(e.player)} planeswalks to ${card(e.to)}`;
    case 'SchemeSetInMotion': return `${who(e.player)} sets ${e.name} in motion`;
    case 'ChoiceRevealed': return `${who(e.player)} chose ${e.option ?? (e.cards.length ? e.cards.map(card).join(' and ') : 'nothing')} (${card(e.source)})`;
    default: return null;
  }
}

export function logViews(lines: string[]): LogView[] {
  return [...lines].reverse().map((t, i) => ({ id: i, at: 0, actor: null, actorName: '', kind: 'act', text: t, priv: false }));
}
