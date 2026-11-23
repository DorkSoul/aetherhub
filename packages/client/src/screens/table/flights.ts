// Animates cards that change zone without being dragged (auto-draw on the turn pass, "To graveyard"
// from a menu, mills, another player's actions …). Each view update is diffed by card id; a moved
// card flies from where it was to where it landed. Piles (library, graveyard, exile, command) are single
// widgets, so a card shrinks into or grows out of its pile.
import { useLayoutEffect, useRef, type RefObject } from 'react';
import type { GameView } from '@aetherhub/shared';
import { SEAT_ZONES } from '@aetherhub/shared';
import { CARD_BACK, imgUrl } from '../../lib/base';

const DURATION = 480;
const STAGGER = 70;
const MAX_FLIGHTS = 10;
/** Zones that draw each card on the table (everything else is a pile widget). */
const RENDERED = new Set(['hand', 'battlefield']);
const PILES = new Set(['library', 'graveyard', 'exile', 'command', 'sideboard']);

interface Loc {
  seat: number;
  zone: string;
}
interface Snapshot {
  gameId: string;
  locs: Map<string, Loc>;
  cards: Map<string, DOMRect>;
  piles: Map<string, DOMRect>;
  at: number;
}

/** Cards the user just dragged themselves: they already landed where they were dropped. */
const dragged = new Map<string, number>();
export function markDragged(ids: string[]) {
  const until = Date.now() + 3000;
  for (const id of ids) dragged.set(id, until);
}
/** A drop that was refused: the card should now fly back instead of being treated as already placed. */
export function unmarkDragged(ids: string[]) {
  for (const id of ids) dragged.delete(id);
}

function locsOf(v: GameView): Map<string, Loc> {
  const m = new Map<string, Loc>();
  v.seats.forEach((s, seat) => {
    for (const z of SEAT_ZONES) for (const iid of s.zones[z] || []) m.set(iid, { seat, zone: z });
  });
  return m;
}

function measure(root: HTMLElement): { cards: Map<string, DOMRect>; piles: Map<string, DOMRect> } {
  const cards = new Map<string, DOMRect>();
  root.querySelectorAll<HTMLElement>('[data-iid]').forEach((el) => {
    const r = el.getBoundingClientRect();
    if (r.width > 0) cards.set(el.dataset.iid!, r);
  });
  const piles = new Map<string, DOMRect>();
  root.querySelectorAll<HTMLElement>('[data-seat]').forEach((seatEl) => {
    seatEl.querySelectorAll<HTMLElement>('[data-pile]').forEach((p) => {
      const r = p.getBoundingClientRect();
      if (r.width > 0) piles.set(`${seatEl.dataset.seat}:${p.dataset.pile}`, r);
    });
  });
  return { cards, piles };
}

/** A card-shaped box inside a (wide) pile widget, at its thumbnail end. */
function cardBoxIn(r: DOMRect): { x: number; y: number; w: number; h: number } {
  if (r.width / r.height < 0.9) return { x: r.left, y: r.top, w: r.width, h: r.height };
  const h = r.height * 0.9;
  const w = (h * 63) / 88;
  return { x: r.left + 6, y: r.top + (r.height - h) / 2, w, h };
}

function imageFor(v: GameView, prev: GameView | null, iid: string): string {
  for (const view of [v, prev]) {
    const c = view?.cards[iid];
    const def = c && !c.hidden && !c.faceDown ? view!.defs[c.def] : undefined;
    const ref = def?.img[c!.face] ?? def?.img[0];
    if (ref) return imgUrl(ref) || CARD_BACK;
  }
  const owner = v.cards[iid]?.owner ?? prev?.cards[iid]?.owner;
  const sleeve = owner != null ? v.seats[owner]?.sleeve : null;
  return sleeve ? imgUrl(sleeve) || CARD_BACK : CARD_BACK;
}

function fly(src: { x: number; y: number; w: number; h: number }, dst: { x: number; y: number; w: number; h: number }, img: string, delay: number, rotateTo: number, intoPile: boolean, onLand?: () => void) {
  const g = document.createElement('div');
  g.setAttribute('aria-hidden', 'true');
  Object.assign(g.style, {
    position: 'fixed',
    left: `${dst.x}px`,
    top: `${dst.y}px`,
    width: `${dst.w}px`,
    height: `${dst.h}px`,
    zIndex: '900',
    pointerEvents: 'none',
    borderRadius: `${Math.max(3, dst.w * 0.05)}px`,
    backgroundImage: `url("${img}")`,
    backgroundSize: 'cover',
    backgroundPosition: 'center',
    backgroundColor: '#15130f',
    boxShadow: '0 10px 26px rgba(0,0,0,.55)',
    transformOrigin: 'center',
    opacity: '0',
  } satisfies Partial<CSSStyleDeclaration>);
  document.body.appendChild(g);
  const dx = src.x + src.w / 2 - (dst.x + dst.w / 2);
  const dy = src.y + src.h / 2 - (dst.y + dst.h / 2);
  const s = src.w / dst.w;
  // Between two small pile widgets the card would be a speck: let it grow to a readable size mid-flight.
  const mid = Math.max(1 + (s - 1) * 0.35, Math.min(2.4, 70 / dst.w));
  const a = g.animate(
    [
      { transform: `translate(${dx}px, ${dy}px) scale(${s})`, opacity: 1 },
      { transform: `translate(${dx * 0.35}px, ${dy * 0.35 - 18}px) scale(${mid})`, opacity: 1, offset: 0.55 },
      { transform: `translate(0, 0) scale(1) rotate(${rotateTo}deg)`, opacity: intoPile ? 0.35 : 1 },
    ],
    { duration: DURATION, delay, easing: 'cubic-bezier(.25,.7,.3,1)', fill: 'both' },
  );
  const done = () => {
    g.remove();
    onLand?.();
  };
  a.onfinish = done;
  a.oncancel = done;
}

/**
 * Call from the table component. Diffs each new view against the last and flies moved cards.
 * `enabled` is false for reduced motion.
 */
export function useCardFlights(v: GameView | null, root: RefObject<HTMLElement | null>, enabled: boolean) {
  const last = useRef<{ snap: Snapshot; view: GameView } | null>(null);

  // Keep a fresh picture of where everything is (throttled), so the next update knows where cards came from.
  useLayoutEffect(() => {
    const el = root.current;
    if (!el || !v) return;
    const prev = last.current;
    const now = Date.now();
    const viewChanged = !prev || prev.view !== v;
    if (viewChanged && prev && enabled && prev.snap.gameId === v.id && !document.hidden) animate(prev, v, el);
    if (viewChanged || now - (prev?.snap.at ?? 0) > 120) {
      const { cards, piles } = measure(el);
      last.current = { view: v, snap: { gameId: v.id, locs: viewChanged || !prev ? locsOf(v) : prev.snap.locs, cards, piles, at: now } };
    }
  });
}

function animate(prev: { snap: Snapshot; view: GameView }, v: GameView, root: HTMLElement) {
  const next = locsOf(v);
  const old = prev.snap.locs;
  const now = Date.now();
  const moves: { iid: string; from: Loc; to: Loc; fromIid?: string; pop?: boolean }[] = [];
  // face-down placeholders for an opponent's hand: one disappears when they play a card from it
  const gone = [...old].filter(([iid, l]) => l.zone === 'hand' && /^H\d+:/.test(iid) && !next.has(iid));
  for (const [iid, to] of next) {
    const from = old.get(iid);
    if ((dragged.get(iid) ?? 0) > now) continue;
    if (from) {
      if (from.seat === to.seat && from.zone === to.zone) continue;
      moves.push({ iid, from, to });
    } else if (to.zone === 'hand' && old.size) {
      // Library cards get new ids when shuffled (e.g. a mulligan), so a fresh card in hand came from the library.
      moves.push({ iid, from: { seat: to.seat, zone: 'library' }, to });
    } else if (to.zone === 'battlefield' && old.size) {
      // a card that appears on the table: it came out of the opponent's hand if one of their face-down cards just went missing, otherwise it is new (a token)
      const k = gone.findIndex(([id]) => id.startsWith(`H${to.seat}:`));
      if (k >= 0) {
        const [fromIid, fromLoc] = gone.splice(k, 1)[0];
        moves.push({ iid, from: fromLoc, to, fromIid });
      } else moves.push({ iid, from: { seat: to.seat, zone: 'battlefield' }, to, pop: true });
    }
  }
  // Hand cards shuffled away (mulligan) vanish under new ids: send them to their owner's library.
  for (const [iid, from] of old) {
    if (next.has(iid) || from.zone !== 'hand' || (dragged.get(iid) ?? 0) > now) continue;
    if ((v.seats[from.seat]?.zones.library.length ?? 0) > (prev.view.seats[from.seat]?.zones.library.length ?? 0)) moves.push({ iid, from, to: { seat: from.seat, zone: 'library' } });
  }
  for (const [id, until] of dragged) if (until < now) dragged.delete(id);
  if (!moves.length) return;

  const cur = measure(root);
  let n = 0;
  for (const m of moves) {
    if (n >= MAX_FLIGHTS) break;
    const fromRect = RENDERED.has(m.from.zone) ? prev.snap.cards.get(m.fromIid ?? m.iid) : PILES.has(m.from.zone) ? prev.snap.piles.get(`${m.from.seat}:${m.from.zone}`) ?? cur.piles.get(`${m.from.seat}:${m.from.zone}`) : undefined;
    const toEl = RENDERED.has(m.to.zone) ? root.querySelector<HTMLElement>(`[data-iid="${CSS.escape(m.iid)}"]`) : null;
    const toRect = toEl ? toEl.getBoundingClientRect() : PILES.has(m.to.zone) ? cur.piles.get(`${m.to.seat}:${m.to.zone}`) : undefined;
    if ((!fromRect && !m.pop) || !toRect || !toRect.width) continue;
    // a new card (token) grows out of its own spot
    const src = m.pop ? { x: toRect.left + toRect.width * 0.25, y: toRect.top + toRect.height * 0.25, w: toRect.width * 0.5, h: toRect.height * 0.5 } : RENDERED.has(m.from.zone) ? { x: fromRect!.left, y: fromRect!.top, w: fromRect!.width, h: fromRect!.height } : cardBoxIn(fromRect!);
    let dst = toEl ? { x: toRect.left, y: toRect.top, w: toRect.width, h: toRect.height } : cardBoxIn(toRect);
    // A tapped card's box is its rotated footprint: fly an upright card there and turn it on landing.
    const tapped = m.to.zone === 'battlefield' && !!v.cards[m.iid]?.tapped;
    if (tapped) dst = { x: dst.x + (dst.w - dst.h) / 2, y: dst.y + (dst.h - dst.w) / 2, w: dst.h, h: dst.w };
    if (src.w < 4 || dst.w < 4) continue;
    const delay = n * STAGGER;
    // Hide the real card until the flying copy lands on it.
    toEl?.animate([{ opacity: 0 }, { opacity: 0 }], { duration: DURATION + delay, fill: 'none' });
    fly(src, dst, imageFor(v, prev.view, m.iid), delay, tapped ? 90 : 0, !toEl);
    n++;
  }
}
