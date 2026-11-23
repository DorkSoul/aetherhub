// Table geometry. All values are pixels of the table viewport.

export const INFO_W = 104;
export const ROWS: Record<number, [number, number]> = { 1: [0, 1], 2: [1, 1], 3: [2, 1], 4: [2, 2], 5: [3, 2], 6: [3, 3], 7: [4, 3], 8: [4, 4] };

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface SeatSlot extends Rect {
  seat: number;
  top: boolean;
}

export interface SeatArea extends SeatSlot {
  hand: Rect;
  bf: Rect;
  card: { w: number; h: number };
  piles: Record<'library' | 'graveyard' | 'exile' | 'command' | 'sideboard', Rect>;
}

/** Seat order relative to the viewer: you are always bottom-left, seats run clockwise. */
export function gridSlots(n: number, viewer: number, x0: number, y0: number, W: number, H: number): SeatSlot[] {
  const rel = (r: number) => (viewer + r) % n;
  const [t, b] = ROWS[n] || [1, 1];
  const slots: SeatSlot[] = [];
  const top: number[] = [];
  for (let i = 1; i <= t; i++) top.push(i);
  const bot = [0];
  for (let i = n - 1; i > t; i--) bot.push(i);
  const hTop = t ? Math.floor(H / 2) : 0;
  top.forEach((r, i) => slots.push({ seat: rel(r), x: x0 + Math.round((i * W) / t), y: y0, w: Math.round(W / t), h: hTop, top: true }));
  bot.forEach((r, i) => slots.push({ seat: rel(r), x: x0 + Math.round((i * W) / b), y: y0 + hTop, w: Math.round(W / b), h: H - hTop, top: false }));
  return slots;
}

export function tabSlots(n: number, viewer: number, opp: number, x0: number, y0: number, W: number, H: number, tabH: number): SeatSlot[] {
  if (n < 2) return gridSlots(n, viewer, x0, y0, W, H);
  const half = Math.floor(H / 2);
  return [
    { seat: opp, x: x0, y: y0 + tabH, w: W, h: half - tabH, top: true },
    { seat: viewer, x: x0, y: y0 + half, w: W, h: H - half, top: false },
  ];
}

export function seatArea(s: SeatSlot, handFrac: number, cardScale: number): SeatArea {
  const cx = s.x + INFO_W;
  const cw = s.w - INFO_W;
  const hh = Math.round(s.h * handFrac);
  const hand = s.top ? { x: cx, y: s.y, w: cw, h: hh } : { x: cx, y: s.y + s.h - hh, w: cw, h: hh };
  const bf = s.top ? { x: cx, y: s.y + hh, w: cw, h: s.h - hh } : { x: cx, y: s.y, w: cw, h: s.h - hh };
  const ch = Math.max(24, Math.round(Math.min(bf.h * 0.34 * cardScale, bf.h - 16)));
  const cwd = Math.round((ch * 63) / 88);
  const pileY = s.y + 100;
  const pileH = Math.max(26, Math.min(38, Math.floor((s.h - 110) / 7)));
  const piles = {
    library: { x: s.x + 8, y: pileY, w: INFO_W - 16, h: pileH },
    graveyard: { x: s.x + 8, y: pileY + (pileH + 5), w: INFO_W - 16, h: pileH },
    exile: { x: s.x + 8, y: pileY + 2 * (pileH + 5), w: INFO_W - 16, h: pileH },
    command: { x: s.x + 8, y: pileY + 3 * (pileH + 5), w: INFO_W - 16, h: pileH },
    sideboard: { x: s.x + 8, y: pileY + 4 * (pileH + 5), w: INFO_W - 16, h: pileH },
  };
  return { ...s, hand, bf, card: { w: cwd, h: ch }, piles };
}

/**
 * Hand layout: grow to fill, then shrink just enough to fit each extra card until the cards are
 * as small as they would be with one more row, then wrap to 2 rows, then 3 rows, then overlap.
 */
export function handLayout(n: number, R: Rect): Rect[] {
  const out: Rect[] = [];
  if (!n) return out;
  const pad = 8;
  const gap = 6;
  const aw = R.w - 2 * pad;
  const ah = R.h - 2 * pad;
  const maxH = Math.min(ah, 260);
  const rowH = (rows: number) => Math.min(maxH, (ah - (rows - 1) * gap) / rows);
  const floorH = rowH(3);
  let best: { rows: number; ch: number } | null = null;
  for (let rows = 1; rows <= 3; rows++) {
    const per = Math.ceil(n / rows);
    const fitW = (((aw - (per - 1) * gap) / per) * 88) / 63;
    const ch = Math.min(rowH(rows), fitW);
    if (ch >= floorH - 1e-6 && (!best || ch > best.ch + 1e-6)) best = { rows, ch };
  }
  if (best) {
    const { rows, ch } = best;
    const cw = (ch * 63) / 88;
    const per = Math.max(Math.ceil(n / rows), Math.floor((aw + gap) / (cw + gap) + 1e-6));
    for (let i = 0; i < n; i++) {
      const r = Math.floor(i / per);
      const col = i % per;
      out.push({ x: R.x + pad + col * (cw + gap), y: R.y + pad + r * (ch + gap), w: cw, h: ch });
    }
    return out;
  }
  const ch = floorH;
  const cw = (ch * 63) / 88;
  const per = Math.ceil(n / 3);
  const step = per > 1 ? Math.min(cw + gap, (aw - cw) / (per - 1)) : 0;
  for (let i = 0; i < n; i++) {
    const r = Math.floor(i / per);
    const col = i % per;
    out.push({ x: R.x + pad + col * step, y: R.y + pad + r * (ch + gap), w: cw, h: ch });
  }
  return out;
}

/**
 * Top-left pixel position of a battlefield card from its normalised position. A tapped card's footprint is rotated.
 * Stored y counts from the far edge, so y = 1 is always next to the seat's hand: on a top seat the battlefield is mirrored.
 */
export function bfPos(A: Pick<SeatArea, 'bf' | 'card'> & { top?: boolean }, c: { x: number; y: number; tapped: boolean }): { x: number; y: number } {
  const w = A.card.w;
  const h = A.card.h;
  const fw = c.tapped ? h : w;
  const fh = c.tapped ? w : h;
  const cx = A.bf.x + 6 + fw / 2 + c.x * Math.max(0, A.bf.w - 12 - fw);
  const cy = A.bf.y + 6 + fh / 2 + (A.top ? 1 - c.y : c.y) * Math.max(0, A.bf.h - 12 - fh);
  return { x: cx - w / 2, y: cy - h / 2 };
}

export function bfFrac(A: Pick<SeatArea, 'bf' | 'card'> & { top?: boolean }, px: number, py: number, tapped: boolean): { x: number; y: number } {
  const w = A.card.w;
  const h = A.card.h;
  const fw = tapped ? h : w;
  const fh = tapped ? w : h;
  const cx = px + w / 2;
  const cy = py + h / 2;
  const clamp = (v: number) => Math.max(0, Math.min(1, v));
  return {
    x: clamp((cx - fw / 2 - A.bf.x - 6) / Math.max(1, A.bf.w - 12 - fw)),
    y: A.top ? 1 - clamp((cy - fh / 2 - A.bf.y - 6) / Math.max(1, A.bf.h - 12 - fh)) : clamp((cy - fh / 2 - A.bf.y - 6) / Math.max(1, A.bf.h - 12 - fh)),
  };
}

export function inRect(x: number, y: number, r: Rect | null | undefined): boolean {
  return !!r && x >= r.x && x <= r.x + r.w && y >= r.y && y <= r.y + r.h;
}
