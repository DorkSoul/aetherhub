import type { CardView, GameView, Rect, SeatArea } from '@aetherhub/shared';
import { bfFrac, bfPos, gridSlots, handLayout, inRect, seatArea, tabSlots } from '@aetherhub/shared';
import type { PanZoom } from './store';

export const TOPBAR = 48;
export const TABH = 40;

export interface Geo {
  areas: Record<number, SeatArea>;
  order: number[];
  tab: boolean;
  width: number;
  height: number;
}

export function computeGeo(v: GameView, W: number, H: number, layout: 'grid' | 'tab' | 'free', tabOpp: number | null): Geo {
  const n = v.seats.length;
  const viewer = typeof v.viewer === 'number' ? v.viewer : 0;
  const tab = layout === 'tab' && n > 2;
  const opp = tabOpp != null && tabOpp !== viewer && tabOpp < n ? tabOpp : (viewer + 1) % n;
  const slots = tab ? tabSlots(n, viewer, opp, 0, 0, W, H, TABH) : gridSlots(n, viewer, 0, 0, W, H);
  const areas: Record<number, SeatArea> = {};
  for (const s of slots) {
    const seat = v.seats[s.seat];
    areas[s.seat] = seatArea(s, seat.handFrac, seat.cardScale);
  }
  return { areas, order: slots.map((s) => s.seat), tab, width: W, height: H };
}

/** The Free view's table size: fixed for the number of seats, large enough that every seat has its normal room however you zoom. */
export function freeWorld(seats: number): { w: number; h: number } {
  if (seats <= 2) return { w: 1800, h: 1100 };
  if (seats <= 4) return { w: 2600, h: 1500 };
  if (seats <= 6) return { w: 3200, h: 1800 };
  return { w: 3600, h: 2000 };
}

export function pzOf(pz: Record<number, PanZoom>, seat: number): PanZoom {
  return pz[seat] || { x: 0, y: 0, k: 1 };
}

/** Local battlefield geometry (origin at the battlefield's top-left) for positioning cards. */
export function localBf(A: SeatArea) {
  return { bf: { x: 0, y: 0, w: A.bf.w, h: A.bf.h }, card: A.card, top: A.top };
}

/** Screen-space rect of a battlefield card (before rotation), including pan/zoom. */
export function bfCardRect(A: SeatArea, c: CardView, pz: PanZoom): Rect {
  const p = bfPos(localBf(A), c);
  return { x: A.bf.x + pz.x + p.x * pz.k, y: A.bf.y + pz.y + p.y * pz.k, w: A.card.w * pz.k, h: A.card.h * pz.k };
}

/** Normalised battlefield position for a card whose top-left would land at screen point (px, py). */
export function dropFrac(A: SeatArea, px: number, py: number, pz: PanZoom, tapped: boolean) {
  const lx = (px - A.bf.x - pz.x) / pz.k;
  const ly = (py - A.bf.y - pz.y) / pz.k;
  return bfFrac(localBf(A), lx, ly, tapped);
}

export function handRects(A: SeatArea, n: number): Rect[] {
  return handLayout(n, A.hand);
}

export type Hit = { t: 'pile'; seat: number; zone: 'library' | 'graveyard' | 'exile' | 'command' | 'sideboard'; r: Rect } | { t: 'hand'; seat: number; r: Rect } | { t: 'bf'; seat: number; r: Rect } | { t: 'info'; seat: number; r: Rect };

export function hitTest(g: Geo, x: number, y: number): Hit | null {
  for (const s of g.order) {
    const A = g.areas[s];
    for (const z of ['library', 'graveyard', 'exile', 'command', 'sideboard'] as const) if (inRect(x, y, A.piles[z])) return { t: 'pile', seat: s, zone: z, r: A.piles[z] };
    if (inRect(x, y, A.hand)) return { t: 'hand', seat: s, r: A.hand };
    if (inRect(x, y, A.bf)) return { t: 'bf', seat: s, r: A.bf };
    if (inRect(x, y, { x: A.x, y: A.y, w: 104, h: A.h })) return { t: 'info', seat: s, r: { x: A.x, y: A.y, w: 104, h: A.h } };
  }
  return null;
}
