import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { Select } from '../../ui/Select';
import type { CardDef, CardView, GameView, Rect, SeatArea, ZoneName } from '@aetherhub/shared';
import { faceSideways, formatInfo, formatName, hexA, inRect, isTopRowSeat, PHASES, turnKind } from '@aetherhub/shared';
import { imgUrl } from '../../lib/base';
import { useApp } from '../../lib/store';
import { socket } from '../../lib/ws';
import { RulingsList } from '../../ui/rulings';
import { askConfirm } from '../../ui/dialogs';
import { CardImage, Modal, openMenuAt, openMenuBelow, timeAgo, toggleTurned, useCompact, useMenu, usePreview, useTurned, type MenuItem } from '../../ui/ui';
import { keyName } from '../settings/Settings';
import { bfCardRect, computeGeo, dropFrac, freeWorld, handRects, hitTest, pzOf, TABH, TOPBAR, type Geo, type Hit } from './geometry';
import { battlefieldMenu, cardLabel, cardMenu, diceMenu, handMenu, hostMenu, libraryMenu, playerMenu } from './menus';
import { TableModals } from './modals';
import { isPanKey, useCamera } from './camera';
import { markDragged, useCardFlights } from './flights';
import { ManaPoolBox } from '../rules/mana';
import { ChatInput, dungeonLabel, LifeBox, LiveOverlays, LogFilters, LogList, Overlays, PhaseStrip, Pile, pileThumb, ptOf, seatTint, TCard } from './parts';
import { mySeat, useTable, type PanZoom } from './store';
import { AfterGameButtons } from '../afterGame';

interface Drag {
  iid: string;
  from: { seat: number; zone: ZoneName };
  sx: number;
  sy: number;
  x: number;
  y: number;
  offX: number;
  offY: number;
  w: number;
  h: number;
  moved: boolean;
  ctrl: boolean;
  shift: boolean;
  tapped: boolean;
  showFace: boolean;
}

interface CardPlace {
  c: CardView;
  seat: number;
  zone: ZoneName;
  rect: Rect; // relative to the table area
  local: Rect; // relative to its container (bf or hand)
  z: number;
}

const send = (i: Parameters<ReturnType<typeof useTable.getState>['send']>[0]) => useTable.getState().send(i);

function defOf(v: GameView, c: CardView | undefined): CardDef | undefined {
  return c && !c.hidden && c.def ? v.defs[c.def] : undefined;
}

export function TableView({ onExit, title }: { onExit: () => void; title?: string }) {
  const v = useTable((s) => s.view);
  const log = useTable((s) => s.log);
  const sel = useTable((s) => s.sel);
  const pz = useTable((s) => s.pz);
  const pings = useTable((s) => s.pings);
  const arrows = useTable((s) => s.arrows);
  const combatArrows = useTable((s) => s.combatArrows);
  const liveArrows = useTable((s) => s.liveArrows);
  const host = useTable((s) => s.host);
  const undoable = useTable((s) => s.undoable);
  const tabOpp = useTable((s) => s.tabOpp);
  const logOpen = useTable((s) => s.logOpen);
  const logFilter = useTable((s) => s.logFilter);
  const mullBottom = useTable((s) => s.mullBottom);
  const mode = useTable((s) => s.mode);
  const rules = useTable((s) => s.rules);
  const payPool = useTable((s) => s.payPool);
  /** players a chosen attacker can attack: their side is outlined and clicking it picks them */
  const pickSeats = useTable((s) => s.pickSeats);
  const solo = useTable((s) => s.solo);
  const mySeats = useTable((s) => s.mySeats);
  const soloSave = useTable((s) => s.soloSave);
  const [leaving, setLeaving] = useState(false);
  // Leaving a solo game that was never saved asks first; saved games autosave, so they just go.
  const exit = () => (soloSave && soloSave.id == null && soloSave.dirty ? setLeaving(true) : onExit());
  const prefs = useApp((s) => s.me!.prefs);
  const setPrefs = useApp((s) => s.setPrefs);
  const areaRef = useRef<HTMLDivElement>(null);
  const reducedMotion = prefs.reduceMotion || (typeof matchMedia !== 'undefined' && matchMedia('(prefers-reduced-motion: reduce)').matches);
  useCardFlights(v, areaRef, !reducedMotion);
  const [size, setSize] = useState({ w: 1200, h: 800 });
  const free = prefs.layout === 'free';
  const [areaEl, setAreaEl] = useState<HTMLDivElement | null>(null);
  const setArea = useCallback((el: HTMLDivElement | null) => {
    areaRef.current = el;
    setAreaEl(el);
  }, []);
  // The Free view's table is a fixed size for the number of seats (not tied to the window or to the Grid / Tab layout); you zoom and pan around it.
  const world = useMemo(() => (free ? freeWorld(v?.seats.length ?? 2) : size), [free, v?.seats.length, size]);
  const camera = useCamera(free, areaEl, size, world);
  const camRef = camera.camRef;
  const [drag, setDrag] = useState<Drag | null>(null);
  const dragRef = useRef<Drag | null>(null);
  const [box, setBox] = useState<{ seat: number; zone: 'battlefield' | 'hand'; x0: number; y0: number; x1: number; y1: number } | null>(null);
  const [arrowDraft, setArrowDraft] = useState<{ from: { iid?: string; seat?: number }; x1: number; y1: number; x2: number; y2: number } | null>(null);
  const [offer, setOffer] = useState<{ iids: string[]; x: number; y: number } | null>(null);
  const [pending, setPending] = useState<Record<string, { seat: number; x: number; y: number }>>({});
  const [panelW, setPanelW] = useState(prefs.panelW);
  const [tabHover, setTabHover] = useState<number | null>(null);
  const spaceDown = useRef(false);
  const pan = useRef<{ seat: number; sx: number; sy: number; ox: number; oy: number } | null>(null);
  const longPress = useRef<number | null>(null);
  // Phone-sized screens: compact top bar, no side panel, one opponent at a time, tap a card to see it.
  const compact = useCompact();
  /** The last press on a card was a touch: no hover preview (it would stick), a tap opens the card sheet instead. */
  const touchPress = useRef(false);
  const [peek, setPeek] = useState<{ iid: string; at: number } | null>(null);
  const detailed = prefs.panel === 'detailed' && !compact;
  const panelWidth = detailed ? panelW : 0;

  useLayoutEffect(() => {
    const el = areaRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setSize({ w: el.clientWidth, h: el.clientHeight }));
    ro.observe(el);
    setSize({ w: el.clientWidth, h: el.clientHeight });
    return () => ro.disconnect();
  }, [detailed, !!v]);

  // The global hover preview follows the table's view settings while the table is open.
  useEffect(() => {
    usePreview.setState({ scale: (prefs.previewSize || 100) / 100, enabled: !detailed && (prefs.layout === 'free' ? prefs.freePreview : prefs.preview) });
    return () => usePreview.setState({ scale: 1, enabled: true });
  }, [prefs.previewSize, prefs.preview, prefs.freePreview, prefs.layout, detailed]);

  // Pending optimistic positions are cleared when the server view catches up.
  useEffect(() => {
    setPending({});
  }, [v]);

  const me = v ? mySeat(v) : null;
  const turned = useTurned((s) => s.m);
  // A phone is too narrow for the grid's side-by-side seats, so it always shows one opponent at a time.
  const layout = free ? 'grid' : compact ? 'tab' : prefs.layout;
  const tabLayout = layout === 'tab' && !!v && v.seats.length > 2;
  const g: Geo | null = useMemo(() => (v ? computeGeo(v, world.w, world.h, layout, tabOpp) : null), [v, world.w, world.h, layout, tabOpp]);
  // How far each seat's info column is scrolled; drops onto its piles hit where they are drawn.
  const [infoScroll, setInfoScroll] = useState<Record<number, number>>({});
  const gHit: Geo | null = useMemo(() => {
    if (!g || !Object.values(infoScroll).some(Boolean)) return g;
    const areas: Geo['areas'] = {};
    for (const s of g.order) {
      const A = g.areas[s];
      const sc = infoScroll[s] || 0;
      const piles = { ...A.piles };
      for (const z of Object.keys(piles) as (keyof typeof piles)[]) {
        const r = { ...piles[z], y: piles[z].y - sc };
        // scrolled out of view: not a drop target
        piles[z] = r.y < A.y || r.y + r.h > A.y + A.h ? { x: -1e6, y: -1e6, w: 0, h: 0 } : r;
      }
      areas[s] = { ...A, piles };
    }
    return { ...g, areas };
  }, [g, infoScroll]);

  // ------------------------------------------------------------------ card placement
  const places = useMemo(() => {
    const out: CardPlace[] = [];
    const byIid = new Map<string, CardPlace>();
    if (!v || !g) return { out, byIid };
    for (const s of g.order) {
      const A = g.areas[s];
      const seat = v.seats[s];
      const P = pzOf(pz, s);
      seat.zones.battlefield.forEach((iid, i) => {
        let c = v.cards[iid];
        if (!c) return;
        const pend = pending[iid];
        if (pend && pend.seat === s) c = { ...c, x: pend.x, y: pend.y };
        const r = bfCardRect(A, c, P);
        const p: CardPlace = { c, seat: s, zone: 'battlefield', rect: r, local: { x: r.x - A.bf.x, y: r.y - A.bf.y, w: r.w, h: r.h }, z: 10 + i * 2 };
        out.push(p);
        byIid.set(iid, p);
      });
      // Attached cards sit under their host with the sides flush, showing below it as if the host lay on top of them
      // (above it when there is no room below, so nothing is cut off at the edge of the battlefield).
      const hung = new Map<string, number>();
      for (const p of out) {
        if (p.seat !== s || !p.c.attachedTo) continue;
        const host = byIid.get(p.c.attachedTo);
        if (!host || host.seat !== s) continue;
        const k = (hung.get(p.c.attachedTo) ?? 0) + 1;
        hung.set(p.c.attachedTo, k);
        const step = host.rect.h * 0.24 * k;
        const dy = host.rect.y + host.rect.h + step <= A.bf.y + A.bf.h - 2 ? step : -step;
        p.rect = { ...host.rect, y: host.rect.y + dy };
        p.local = { ...host.local, y: host.local.y + dy };
        p.z = host.z - k;
      }
      const hand = seat.zones.hand;
      const hr = handRects(A, hand.length);
      hand.forEach((iid, i) => {
        const c = v.cards[iid];
        if (!c || !hr[i]) return;
        const p: CardPlace = { c, seat: s, zone: 'hand', rect: hr[i], local: { x: hr[i].x - A.hand.x, y: hr[i].y - A.hand.y, w: hr[i].w, h: hr[i].h }, z: 200 + i };
        out.push(p);
        byIid.set(iid, p);
      });
    }
    return { out, byIid };
  }, [v, g, pz, pending]);

  // The card sheet closes when its card leaves the battlefield / hand.
  useEffect(() => {
    if (peek && !places.byIid.has(peek.iid)) setPeek(null);
  }, [peek, places]);

  // ------------------------------------------------------------------ helpers
  // Screen → table coordinates (undoes the Free view's camera).
  const pt = useCallback((e: { clientX: number; clientY: number }) => {
    const r = areaRef.current!.getBoundingClientRect();
    const c = camRef.current;
    return { x: (e.clientX - r.left - c.x) / c.k, y: (e.clientY - r.top - c.y) / c.k };
  }, []);

  /** Text under a card preview. Effects, counters and changed P/T are drawn on the preview itself (live); the text keeps the rest. */
  const previewExtra = (p: CardPlace, def: CardDef): string | undefined => {
    if (!v) return undefined;
    const extra: string[] = [];
    const ptx = ptOf(p.c, def);
    if (ptx && def.power != null && p.zone === 'battlefield') extra.push(`Printed P/T ${def.power}/${def.toughness}`);
    if (p.c.faceDown) extra.push('Face down — only you can see it');
    if (p.c.attachedTo && v.cards[p.c.attachedTo]) extra.push('Attached to ' + cardLabel(v, v.cards[p.c.attachedTo]));
    return extra.length ? extra.join('\n') : undefined;
  };

  const showPreview = (e: React.MouseEvent, p: CardPlace) => {
    if (!v) return;
    useTable.setState({ hover: p.c.iid, lastHover: defOf(v, p.c) ? p.c.iid : useTable.getState().lastHover });
    if (dragRef.current?.moved || touchPress.current) return;
    // Follows the current view's setting (Grid/Tab and Free have separate preview switches).
    if (detailed || !usePreview.getState().enabled) return;
    const def = defOf(v, p.c);
    if (!def) return;
    const iid = p.c.iid;
    const onBf = p.zone === 'battlefield';
    usePreview.getState().show(def, e.currentTarget as HTMLElement, previewExtra(p, def), p.c.flipped ? 1 : p.c.face, iid, (w) => <LiveOverlays iid={iid} w={w} onBf={onBf} />);
  };

  /** Turn-over icon: on the battlefield it transforms / flips the card for everyone; elsewhere it's a local look at the other side. */
  const turnCard = (p: CardPlace) => {
    if (!v) return;
    const def = defOf(v, p.c);
    const kind = turnKind(def);
    if (def && p.zone === 'battlefield' && me != null && kind === 'dfc' && (def.faces?.length || 0) > 1) {
      send({ type: 'transform', iid: p.c.iid });
      usePreview.getState().setFace(def.key, (p.c.face + 1) % def.faces!.length);
    } else if (def && p.zone === 'battlefield' && me != null && kind === 'flip') {
      send({ type: 'flip', iid: p.c.iid });
      usePreview.getState().setFace(def.key, p.c.flipped ? 0 : 1);
    } else toggleTurned(p.c.iid);
  };

  // ------------------------------------------------------------------ pointer: cards
  const onCardDown = (e: React.PointerEvent, p: CardPlace) => {
    if (!v) return;
    e.stopPropagation();
    if (e.button === 1) {
      e.preventDefault();
      socket.send({ t: 'ping', iid: p.c.iid });
      if (mode === 'local') useTable.setState({ pings: { ...useTable.getState().pings, [p.c.iid]: Date.now() } });
      return;
    }
    if (e.button !== 0) return;
    const P = pt(e);
    touchPress.current = e.pointerType === 'touch';
    if (e.pointerType === 'touch') {
      longPress.current = window.setTimeout(() => {
        dragRef.current = null;
        setDrag(null);
        openMenuAt(e.nativeEvent, () => menuFor(p));
      }, 550);
    }
    if (e.altKey) {
      setArrowDraft({ from: { iid: p.c.iid }, x1: p.rect.x + p.rect.w / 2, y1: p.rect.y + p.rect.h / 2, x2: P.x, y2: P.y });
      return;
    }
    const def = defOf(v, p.c);
    const d: Drag = {
      iid: p.c.iid,
      from: { seat: p.seat, zone: p.zone },
      sx: P.x,
      sy: P.y,
      x: P.x,
      y: P.y,
      offX: P.x - p.rect.x,
      offY: P.y - p.rect.y,
      w: p.rect.w,
      h: p.rect.h,
      moved: false,
      ctrl: e.ctrlKey || e.metaKey,
      shift: e.shiftKey,
      tapped: p.zone === 'battlefield' && p.c.tapped,
      showFace: !!def && !p.c.faceDown,
    };
    dragRef.current = d;
    setDrag(d);
  };

  const onPileDown = (e: React.PointerEvent, seat: number, zone: 'library' | 'graveyard' | 'exile' | 'command', r: Rect) => {
    if (!v || e.button !== 0) return;
    const list = v.seats[seat].zones[zone];
    if (!list.length) return;
    const iid = zone === 'library' || zone === 'command' ? list[0] : list[list.length - 1];
    const c = v.cards[iid];
    const P = pt(e);
    const th = r.h - 6;
    const tw = (th * 63) / 88;
    const d: Drag = { iid, from: { seat, zone }, sx: P.x, sy: P.y, x: P.x, y: P.y, offX: tw / 2, offY: th / 2, w: tw, h: th, moved: false, ctrl: false, shift: false, tapped: false, showFace: !!defOf(v, c) && !c.faceDown };
    dragRef.current = d;
    setDrag(d);
  };

  const menuFor = (p: CardPlace) => {
    if (!v) return [];
    if (p.zone === 'hand' && p.c.hidden) return handMenu(p.seat);
    const acts = useTable.getState().rules?.abilities(p.c.iid) ?? [];
    const base = cardMenu(p.c.iid);
    if (!acts.length) return base;
    return [{ kind: 'head' as const, label: p.zone === 'hand' ? 'Play' : 'Activated effects' }, ...acts.map((a) => ({ kind: 'item' as const, label: a.label, run: a.run, sub: a.sub })), { kind: 'sep' as const }, ...base];
  };

  // ------------------------------------------------------------------ pointer: battlefield background
  const touches = useRef<Map<number, { x: number; y: number }>>(new Map());
  const pinch = useRef<{ seat: number; d0: number; k0: number } | null>(null);
  const onBfDown = (e: React.PointerEvent, seat: number) => {
    const P = pt(e);
    if (e.pointerType === 'touch') {
      touches.current.set(e.pointerId, P);
      if (touches.current.size === 2) {
        const [a, b] = [...touches.current.values()];
        pinch.current = { seat, d0: Math.hypot(a.x - b.x, a.y - b.y) || 1, k0: pzOf(useTable.getState().pz, seat).k };
        setBox(null);
        return;
      }
    }
    if (e.button === 1 || (e.button === 0 && spaceDown.current)) {
      e.preventDefault();
      const cur = pzOf(useTable.getState().pz, seat);
      pan.current = { seat, sx: P.x, sy: P.y, ox: cur.x, oy: cur.y };
      return;
    }
    if (e.button !== 0) return;
    setOffer(null);
    setBox({ seat, zone: 'battlefield', x0: P.x, y0: P.y, x1: P.x, y1: P.y });
  };

  /** Box-select in your own hand, starting on empty space in the hand strip. */
  const onHandDown = (e: React.PointerEvent, seat: number) => {
    if (e.button !== 0 || seat !== me || e.pointerType === 'touch') return;
    const P = pt(e);
    setOffer(null);
    setBox({ seat, zone: 'hand', x0: P.x, y0: P.y, x1: P.x, y1: P.y });
  };

  const onBfWheel = (e: React.WheelEvent, A: SeatArea) => {
    if (free || (!e.ctrlKey && !e.metaKey)) return;
    e.preventDefault();
    const P = pt(e);
    const cur = pzOf(useTable.getState().pz, A.seat);
    const k = Math.max(0.5, Math.min(2.5, cur.k * (e.deltaY < 0 ? 1.1 : 1 / 1.1)));
    const lx = P.x - A.bf.x;
    const ly = P.y - A.bf.y;
    const nx = lx - ((lx - cur.x) * k) / cur.k;
    const ny = ly - ((ly - cur.y) * k) / cur.k;
    useTable.setState({ pz: { ...useTable.getState().pz, [A.seat]: { x: nx, y: ny, k } } });
  };

  // Global move/up listeners while dragging / box-selecting / drawing an arrow.
  useEffect(() => {
    let raf = 0;
    let last: PointerEvent | null = null;
    const move = (e: PointerEvent) => {
      last = e;
      if (raf) return;
      raf = requestAnimationFrame(() => {
        raf = 0;
        if (!last || !areaRef.current) return;
        const P = pt(last);
        if (pinch.current && last.pointerType === 'touch' && touches.current.has(last.pointerId)) {
          touches.current.set(last.pointerId, P);
          const [a, b] = [...touches.current.values()];
          if (a && b) {
            const k = Math.max(0.5, Math.min(2.5, (pinch.current.k0 * Math.hypot(a.x - b.x, a.y - b.y)) / pinch.current.d0));
            const cur = pzOf(useTable.getState().pz, pinch.current.seat);
            useTable.setState({ pz: { ...useTable.getState().pz, [pinch.current.seat]: { ...cur, k } } });
          }
          return;
        }
        if (pan.current) {
          const p = pan.current;
          const cur = pzOf(useTable.getState().pz, p.seat);
          useTable.setState({ pz: { ...useTable.getState().pz, [p.seat]: { ...cur, x: p.ox + P.x - p.sx, y: p.oy + P.y - p.sy } } });
          return;
        }
        const d = dragRef.current;
        if (d) {
          const moved = d.moved || Math.hypot(P.x - d.sx, P.y - d.sy) > 4;
          if (moved && longPress.current) {
            clearTimeout(longPress.current);
            longPress.current = null;
          }
          if (moved && !d.moved) usePreview.getState().hide(true);
          const nd = { ...d, x: P.x, y: P.y, moved };
          dragRef.current = nd;
          setDrag(nd);
        }
        setBox((b) => (b ? { ...b, x1: P.x, y1: P.y } : b));
        setArrowDraft((a) => (a ? { ...a, x2: P.x, y2: P.y } : a));
      });
    };
    const up = (e: PointerEvent) => {
      touches.current.delete(e.pointerId);
      if (pinch.current) {
        if (touches.current.size < 2) pinch.current = null;
        return;
      }
      if (longPress.current) {
        clearTimeout(longPress.current);
        longPress.current = null;
      }
      if (pan.current) {
        pan.current = null;
        return;
      }
      const P = areaRef.current ? pt(e) : { x: 0, y: 0 };
      const d = dragRef.current;
      if (d) {
        dragRef.current = null;
        setDrag(null);
        if (!d.moved) clickCard(d, e);
        else drop(d, P.x, P.y, e.shiftKey);
      }
      setBox((b) => {
        if (b) finishBox(b);
        return null;
      });
      setArrowDraft((a) => {
        if (a) finishArrow(a, e, P);
        return null;
      });
    };
    // The browser took the touch over (it became a scroll): drop whatever was in progress.
    const cancel = (e: PointerEvent) => {
      touches.current.delete(e.pointerId);
      if (touches.current.size < 2) pinch.current = null;
      if (longPress.current) {
        clearTimeout(longPress.current);
        longPress.current = null;
      }
      pan.current = null;
      dragRef.current = null;
      setDrag(null);
      setBox(null);
      setArrowDraft(null);
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up);
    window.addEventListener('pointercancel', cancel);
    return () => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', up);
      window.removeEventListener('pointercancel', cancel);
      cancelAnimationFrame(raf);
    };
  });

  /** A double-click (or Shift+click in a rules game) on a card: tap / activate it on the battlefield, play it from the hand. */
  const activateCard = (iid: string, zone: ZoneName) => {
    if (!v) return;
    if (zone === 'battlefield') send({ type: 'tap', iids: [iid] });
    else if (zone === 'hand' && me != null && defOf(v, v.cards[iid])) send({ type: 'move', iids: [iid], seat: me, zone: 'battlefield', x: 0.45, y: isTopRowSeat(me, v.seats.length) ? 0.6 : 0.35, ...(rules ? { activate: true } : {}) });
  };

  const clickCard = (d: Drag, e: PointerEvent) => {
    const st = useTable.getState();
    if (st.mullBottom && d.from.zone === 'hand' && d.from.seat === me) {
      const mb = st.mullBottom;
      useTable.setState({ mullBottom: mb.includes(d.iid) ? mb.filter((x) => x !== d.iid) : [...mb, d.iid] });
      return;
    }
    if (st.pick && st.pickIds.includes(d.iid)) {
      // Touch: someone else's card (an attacker to block, a target) opens large first, with Choose on the sheet;
      // your own (lands to pay with, your blockers) are chosen at once.
      if (e.pointerType === 'touch' && v && d.from.seat !== me && defOf(v, v.cards[d.iid])) {
        usePreview.getState().hide(true);
        setPeek({ iid: d.iid, at: Date.now() });
        return;
      }
      st.pick(d.iid);
      return;
    }
    // choosing who an attacker attacks: a click on any card on that player's battlefield counts as that player
    if (st.pickSeat && d.from.zone === 'battlefield' && st.pickSeats.includes(d.from.seat)) {
      st.pickSeat(d.from.seat);
      return;
    }
    // a question is asking for a pick and this card isn't one of the choices: say why
    if (st.pick && st.pickDenied && d.from.zone === 'battlefield') st.pickDenied(d.iid);
    if (d.from.zone === 'library' || d.from.zone === 'graveyard' || d.from.zone === 'exile' || d.from.zone === 'command') return;
    // Rules game: Shift+click does what a double-click does, so lands can be tapped quickly one after another
    if (rules && e.shiftKey && !e.ctrlKey && !e.metaKey && (d.from.zone === 'battlefield' || (d.from.zone === 'hand' && d.from.seat === me))) {
      activateCard(d.iid, d.from.zone);
      return;
    }
    const ctrl = e.ctrlKey || e.metaKey;
    if (ctrl) useTable.setState({ sel: st.sel.includes(d.iid) ? st.sel.filter((x) => x !== d.iid) : [...st.sel, d.iid] });
    else useTable.setState({ sel: [d.iid] });
    // Touch has no hover: one tap opens the card sheet (big preview + actions).
    if (e.pointerType === 'touch' && v && defOf(v, v.cards[d.iid])) {
      usePreview.getState().hide(true);
      setPeek({ iid: d.iid, at: Date.now() });
    }
  };

  // Where a card dropped at x goes in a hand: after every other card whose middle is left of x.
  const handSlotAt = (seat: number, moving: string[], x: number): number => {
    if (!v || !g) return 0;
    const hand = v.seats[seat].zones.hand;
    const rects = handRects(g.areas[seat], hand.length);
    return hand.filter((iid, k) => !moving.includes(iid) && !!rects[k] && rects[k].x + rects[k].w / 2 < x).length;
  };

  const drop = (d: Drag, x: number, y: number, shift: boolean) => {
    if (!v || !g) return;
    const h = hitTest(gHit!, x, y);
    if (!h) return;
    const st = useTable.getState();
    const ids = st.sel.includes(d.iid) && st.sel.length > 1 ? st.sel.slice() : [d.iid];
    // Dragged cards already land where they were dropped; don't fly them in again.
    markDragged(ids);
    if (h.t === 'bf') {
      const A = g.areas[h.seat];
      const P = pzOf(st.pz, h.seat);
      const cw = A.card.w * P.k;
      const ch = A.card.h * P.k;
      const positions: { x: number; y: number }[] = [];
      ids.forEach((iid, k) => {
        const pl = places.byIid.get(iid);
        const c = v.cards[iid];
        let px: number;
        let py: number;
        if (iid === d.iid) {
          px = x - d.offX * (cw / d.w);
          py = y - d.offY * (ch / d.h);
        } else if (pl && pl.zone === 'battlefield' && pl.seat === h.seat && d.from.zone === 'battlefield' && d.from.seat === h.seat) {
          px = pl.rect.x + (x - d.sx);
          py = pl.rect.y + (y - d.sy);
        } else {
          px = x - d.offX * (cw / d.w) + k * 14;
          py = y - d.offY * (ch / d.h) + k * 8;
        }
        const tapped = !!(pl && pl.zone === 'battlefield' && c?.tapped);
        positions.push(dropFrac(A, px, py, P, tapped));
      });
      const pend: Record<string, { seat: number; x: number; y: number }> = {};
      ids.forEach((iid, i) => (pend[iid] = { seat: h.seat, ...positions[i] }));
      setPending((p) => ({ ...p, ...pend }));
      send({ type: 'move', iids: ids, seat: h.seat, zone: 'battlefield', positions });
    } else if (h.t === 'hand') {
      const hand = v.seats[h.seat].zones.hand;
      const at = handSlotAt(h.seat, ids, x);
      const rest = hand.filter((i) => !ids.includes(i));
      const next = [...rest.slice(0, at), ...ids, ...rest.slice(at)];
      if (next.length === hand.length && next.every((i, k) => i === hand[k])) return;
      send({ type: 'move', iids: ids, seat: h.seat, zone: 'hand', pos: at + 1 });
    } else if (h.t === 'pile') {
      if (d.from.zone === h.zone && d.from.seat === h.seat) return;
      send({ type: 'move', iids: ids, seat: h.seat, zone: h.zone, pos: h.zone === 'library' ? (shift ? 'bottom' : 'top') : undefined });
    }
    if (ids.length > 1) useTable.setState({ sel: ids });
  };

  const finishBox = (b: { seat: number; zone: 'battlefield' | 'hand'; x0: number; y0: number; x1: number; y1: number }) => {
    if (!v) return;
    const x0 = Math.min(b.x0, b.x1);
    const x1 = Math.max(b.x0, b.x1);
    const y0 = Math.min(b.y0, b.y1);
    const y1 = Math.max(b.y0, b.y1);
    if (x1 - x0 < 4 && y1 - y0 < 4) {
      // a plain click on a player's battlefield while choosing who an attacker attacks
      const st = useTable.getState();
      if (b.zone === 'battlefield' && st.pickSeat && st.pickSeats.includes(b.seat)) return st.pickSeat(b.seat);
      if (st.pick) return; // keep the cards a question has picked so far highlighted
      useTable.setState({ sel: [] });
      return;
    }
    if (b.zone === 'hand') {
      // Hand cards overlap, so any card the box touches counts.
      const ids = places.out.filter((p) => p.zone === 'hand' && p.seat === b.seat && p.rect.x < x1 && p.rect.x + p.rect.w > x0 && p.rect.y < y1 && p.rect.y + p.rect.h > y0).map((p) => p.c.iid);
      const mb = useTable.getState().mullBottom;
      // Choosing mulligan cards to put on the bottom: the box adds to that pick instead.
      if (mb) useTable.setState({ mullBottom: Array.from(new Set([...mb, ...ids])) });
      else useTable.setState({ sel: ids });
      return;
    }
    const ids = places.out.filter((p) => p.zone === 'battlefield' && p.seat === b.seat && inRect(p.rect.x + p.rect.w / 2, p.rect.y + p.rect.h / 2, { x: x0, y: y0, w: x1 - x0, h: y1 - y0 })).map((p) => p.c.iid);
    useTable.setState({ sel: ids });
    const cs = ids.map((i) => v.cards[i]);
    if (ids.length > 1 && cs.every((c) => c && !c.faceDown && !c.hidden && c.def === cs[0].def)) setOffer({ iids: ids, x: x1, y: y1 });
  };

  const finishArrow = (a: NonNullable<typeof arrowDraft>, e: PointerEvent, P: { x: number; y: number }) => {
    if (!g) return;
    const el = document.elementFromPoint(e.clientX, e.clientY)?.closest('[data-iid]') as HTMLElement | null;
    let to: { iid?: string; seat?: number } | null = null;
    if (el?.dataset.iid && el.dataset.iid !== a.from.iid) to = { iid: el.dataset.iid };
    else {
      const h = hitTest(gHit!, P.x, P.y);
      if (h) to = { seat: h.seat };
    }
    if (!to || Math.hypot(a.x2 - a.x1, a.y2 - a.y1) < 20) return;
    if (mode === 'local') useTable.setState({ arrows: [...useTable.getState().arrows, { id: String(Math.random()), seat: 0, color: v!.seats[0].color, from: a.from, to, at: Date.now() }] });
    else socket.send({ t: 'arrow', from: a.from, to });
  };

  // ------------------------------------------------------------------ keyboard
  useEffect(() => {
    const down = (e: KeyboardEvent) => {
      if (e.key === ' ' && !(e.target instanceof HTMLInputElement) && !(e.target instanceof HTMLTextAreaElement)) spaceDown.current = true;
      const t = e.target as HTMLElement;
      if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.tagName === 'SELECT' || t.isContentEditable)) return;
      if (!v) return;
      const k = keyName(e);
      if (!k) return;
      // Free view: plain W A S D / arrows pan the camera; Shift+key still triggers a hotkey bound to it.
      if (prefs.layout === 'free' && isPanKey(e)) return;
      const hk = prefs.hotkeys;
      const st = useTable.getState();
      if (e.key === 'Escape') {
        useTable.setState({ sel: [], modal: null });
        setOffer(null);
        rules?.escape?.();
        return;
      }
      // Rules game: the sandbox actions (draw, untap all, shuffle) don't exist; E ends the turn, S sorts your board, ? lists the shortcuts
      if (rules) {
        if (k === hk.undo) {
          e.preventDefault();
          doUndo();
        } else if (k === hk.pass && me != null) {
          e.preventDefault();
          send({ type: 'passTurn' });
        } else if (k === hk.tap) {
          const targets = st.sel.length ? st.sel : st.hover ? [st.hover] : [];
          const bf = targets.filter((i) => v.seats.some((s) => s.zones.battlefield.includes(i)));
          if (bf.length) send({ type: 'tap', iids: bf });
        } else if (k === 'E' && me != null) rules.endTurn();
        else if (k === 'S' && me != null) rules.sortBoard(me);
        else if (k === 'B' && me != null) rules.toggleBreak();
        else if (e.key === '?') openMenuAt({ clientX: window.innerWidth - 340, clientY: TOPBAR + 4 } as React.MouseEvent, () => shortcutsMenu(prefs.hotkeys));
        return;
      }
      if (e.key === 'Delete' || e.key === 'Backspace') {
        const tok = st.sel.filter((i) => v.cards[i]?.token);
        if (tok.length) send({ type: 'delete', iids: tok });
        return;
      }
      if (k === hk.undo) {
        e.preventDefault();
        doUndo();
      } else if (k === hk.tap) {
        const targets = st.sel.length ? st.sel : st.hover ? [st.hover] : [];
        const bf = targets.filter((i) => v.seats.some((s) => s.zones.battlefield.includes(i)));
        if (bf.length) send({ type: 'tap', iids: bf, tapped: !v.cards[bf[0]].tapped });
      } else if (k === hk.draw && me != null) send({ type: 'draw', seat: me, n: 1 });
      else if (k === hk.untap && me != null) send({ type: 'untapAll', seat: me });
      else if (k === hk.shuffle && me != null) send({ type: 'shuffle', seat: me });
      else if (k === hk.pass && me != null) {
        e.preventDefault();
        send({ type: 'passTurn' });
      } else return;
    };
    const up = (e: KeyboardEvent) => {
      if (e.key === ' ') spaceDown.current = false;
    };
    window.addEventListener('keydown', down);
    window.addEventListener('keyup', up);
    return () => {
      window.removeEventListener('keydown', down);
      window.removeEventListener('keyup', up);
    };
  });

  const doUndo = () => {
    if (rules) return rules.undo();
    if (mode === 'local') useTable.getState().send({ type: 'chat', text: '__undo__' });
    else socket.send({ t: 'undo' });
  };

  if (!v || !g) return <div style={{ padding: 40, color: 'var(--mu)' }}>Waiting for the table…</div>;

  // ------------------------------------------------------------------ render
  const active = v.turn.active;
  const ap = v.seats[active];
  const spectator = me == null;
  const hot: Hit | null = drag?.moved ? hitTest(gHit ?? g, drag.x, drag.y) : null;
  const dragIds = drag?.moved ? (sel.includes(drag.iid) && sel.length > 1 ? sel : [drag.iid]) : [];
  const planar = v.settings.planechase || v.table.planes.length > 0;
  // a rules game: the engine runs Planechase and Archenemy (the planar die is in the action list), so no sandbox controls
  const specials = !rules && (planar || v.settings.dungeons || v.settings.archenemy || v.settings.attractions || v.seats.some((s) => s.archenemy));

  // Ghost: exactly where the card will land.
  let ghost: { r: Rect; rot: number; face: boolean; def?: CardDef; sleeve: string | null } | null = null;
  if (drag?.moved) {
    const c = v.cards[drag.iid];
    let r: Rect = { x: drag.x - drag.offX, y: drag.y - drag.offY, w: drag.w, h: drag.h };
    let rot = 0;
    let face = drag.showFace;
    if (hot?.t === 'bf') {
      const A = g.areas[hot.seat];
      const P = pzOf(pz, hot.seat);
      const fr = dropFrac(A, drag.x - drag.offX * ((A.card.w * P.k) / drag.w), drag.y - drag.offY * ((A.card.h * P.k) / drag.h), P, drag.tapped);
      r = bfCardRect(A, { ...(c as CardView), x: fr.x, y: fr.y, tapped: drag.tapped }, P);
      rot = drag.tapped ? 90 : 0;
      face = drag.showFace;
    } else if (hot?.t === 'hand') {
      const hs = v.seats[hot.seat];
      const others = hs.zones.hand.filter((i) => i !== drag.iid);
      const hl = handRects(g.areas[hot.seat], others.length + 1);
      const slot = hl[handSlotAt(hot.seat, [drag.iid], drag.x)];
      if (slot) r = slot;
      face = drag.showFace && (hot.seat === me || !!me);
    } else if (hot?.t === 'pile') {
      const th = hot.r.h - 6;
      r = { x: hot.r.x + 5, y: hot.r.y + 3, w: (th * 63) / 88, h: th };
      face = drag.showFace && hot.zone !== 'library';
    }
    ghost = { r, rot, face, def: defOf(v, c), sleeve: v.seats[c?.owner ?? 0]?.sleeve ?? null };
  }

  const cardPos = (iid: string) => places.byIid.get(iid)?.rect;
  const seatCenter = (s: number) => {
    const A = g.areas[s];
    return A ? { x: A.x + 52, y: A.y + 65 } : null;
  };

  /** Compact top bar: every button of the full bar, as one dropdown. */
  const barMenu = (): MenuItem[] => {
    const st = useTable.getState();
    const r = st.rules;
    const items: MenuItem[] = [{ kind: 'head', label: `${title || v.settings.name} · ${formatName(v.settings.format)}` }];
    if (!spectator) {
      items.push({ kind: 'item', label: 'Undo', hint: r ? undefined : String(st.undoable), disabled: !!r && !r.canUndo, run: doUndo });
      if (r) items.push({ kind: 'item', label: r.ending ? 'Stop passing' : 'End turn', hint: r.ending ? 'passing…' : undefined, run: r.ending ? r.cancelPass : r.endTurn });
    }
    if (solo)
      items.push({
        kind: 'sub',
        label: `Playing as ${v.seats[solo.seat]?.name ?? ''}`,
        items: () =>
          v.seats.map((s2, i) => ({
            kind: 'item' as const,
            label: s2.name + (v.turn.active === i ? ' · turn' : ''),
            checked: solo.seat === i,
            run: () => {
              useTable.setState({ solo: { ...solo, seat: i }, sel: [], mullBottom: null });
              useTable.getState().republish?.();
            },
          })),
      });
    if (soloSave && (soloSave.dirty || soloSave.id == null)) items.push({ kind: 'item', label: soloSave.saving ? 'Saving…' : 'Save', disabled: soloSave.saving, run: () => void soloSave.save() });
    if (r) items.push({ kind: 'sub', label: 'Game', items: rulesGameMenu });
    items.push({ kind: 'sep' });
    items.push({ kind: 'item', label: 'Log & chat', checked: st.logOpen, run: () => useTable.setState({ logOpen: !useTable.getState().logOpen }) });
    if (tabLayout) items.push({ kind: 'sub', label: 'Show opponent', items: () => v.seats.flatMap((p, i) => (i === me || (me == null && i === 0) ? [] : [{ kind: 'item' as const, label: `${p.name} · ♥ ${p.life}${active === i ? ' · turn' : ''}`, checked: g.order.includes(i), run: () => useTable.setState({ tabOpp: i }) }])) });
    items.push({ kind: 'sub', label: 'Settings', items: viewMenu });
    items.push({ kind: 'sub', label: 'Dice', items: diceMenu });
    if (specials) items.push({ kind: 'item', label: planar ? 'Planechase' : 'Specials', run: () => useTable.setState({ modal: { kind: 'specials' } }) });
    if (!spectator && !r) items.push({ kind: 'item', label: 'Tokens', run: () => useTable.setState({ modal: { kind: 'tokens', seat: me! } }) });
    if (prefs.barDay || v.global.day) items.push({ kind: 'item', label: v.global.day === 'night' ? '☾ Night → Day' : '☀ Day → Night', run: () => send({ type: 'day', value: v.global.day === 'day' ? 'night' : 'day' }) });
    if ((host || mode === 'local') && !r) items.push({ kind: 'sub', label: mode === 'local' ? 'Playtest' : 'Host', items: hostMenu });
    if (!r) items.push({ kind: 'sep' }, { kind: 'item', label: 'Leave to main menu', run: exit });
    return items;
  };

  /** Rules game: the ◇ Menu dropdown (game-level actions that don't need to sit in the bar). */
  const rulesGameMenu = (): MenuItem[] => {
    const r = useTable.getState().rules;
    if (!r) return [];
    const items: MenuItem[] = [{ kind: 'head', label: title || v.settings.name }];
    if (!spectator) items.push({ kind: 'item', label: 'Save game', run: r.save });
    if (!spectator)
      items.push(
        r.breakRules.active
          ? r.breakRules.mine
            ? { kind: 'item', label: 'Finish breaking the rules', run: r.toggleBreak }
            : { kind: 'item', label: `Paused by ${r.breakRules.by}`, disabled: true, run: () => {} }
          : { kind: 'item', label: 'Break the rules…', hint: 'B', run: r.toggleBreak },
      );
    items.push({
      kind: 'sub',
      label: `Remembered reactions${r.reactions.length ? ` (${r.reactions.length})` : ''}`,
      items: () => {
        const cur = useTable.getState().rules?.reactions ?? [];
        return cur.length
          ? [{ kind: 'head' as const, label: 'Click one to forget it' }, ...cur.map((x) => ({ kind: 'item' as const, label: x.label, hint: 'forget', run: x.forget }))]
          : [{ kind: 'head' as const, label: 'None yet: right-click a stack item, or use “Remember my answer” under a question' }];
      },
    });
    items.push({ kind: 'sep' });
    if (!spectator) {
      items.push({ kind: 'item', label: 'Concede', danger: true, run: r.concede });
      items.push({ kind: 'item', label: 'End game', hint: 'not saved', danger: true, run: r.endGame });
      items.push({ kind: 'sep' });
    }
    items.push({ kind: 'item', label: 'Back to main menu', hint: 'game keeps running', run: exit });
    return items;
  };

  return (
    <div style={{ position: 'fixed', inset: 0, background: 'var(--bg-table)', userSelect: 'none', WebkitUserSelect: 'none', WebkitTouchCallout: 'none', overflow: 'hidden', touchAction: 'manipulation' }} onContextMenu={(e) => e.preventDefault()}>
      <style>{`.tping{animation:tping 1.1s ease-out 2}@keyframes tping{0%{box-shadow:0 0 0 0 rgba(255,255,255,.95)}100%{box-shadow:0 0 0 22px rgba(255,255,255,0)}}.tarrow{animation:tfade 10s linear forwards}@keyframes tfade{0%,75%{opacity:1}100%{opacity:0}}.pile:hover{background:var(--surface4)!important}.ic:hover{background:var(--surface4)!important}.lifebox:hover{border-color:var(--line5)!important}.rspin{animation:rspin .8s linear infinite}@keyframes rspin{to{transform:rotate(360deg)}}.reduce-motion .tping,.reduce-motion .tarrow{animation:none}`}</style>
      {/* Top bar */}
      {compact ? (
        <div className="row" style={{ position: 'absolute', left: 0, top: 0, right: 0, height: TOPBAR, padding: '0 8px', background: 'var(--surface)', borderBottom: '1px solid var(--line3)', gap: 6, zIndex: 600, overflowX: 'auto', overflowY: 'hidden', scrollbarWidth: 'none', overscrollBehaviorX: 'contain', touchAction: 'pan-x' }}>
          <button className="tb sm" style={{ flexShrink: 0, fontSize: 16, padding: '0 11px' }} onClick={(e) => openMenuBelow(e.currentTarget, barMenu)} aria-label="Game menu">
            ☰
          </button>
          <div className="row" style={{ height: 30, padding: '0 9px', borderRadius: 8, background: 'var(--surface2)', border: `1px solid ${ap?.color}`, gap: 6, fontSize: 12, whiteSpace: 'nowrap', flexShrink: 0 }}>
            <span style={{ color: 'var(--mu2)' }}>T{v.turn.n}</span>
            <span className="dot" style={{ background: ap?.color }} />
            <span style={{ fontWeight: 600, maxWidth: 90, overflow: 'hidden', textOverflow: 'ellipsis' }}>{active === me && !solo ? 'You' : ap?.name}</span>
            <span style={{ color: 'var(--mu)' }}>{PHASES[v.turn.phase]}</span>
          </div>
          <PhaseStrip compact phase={v.turn.phase} onPick={(i) => (rules ? rules.passTo(i) : send({ type: 'phase', phase: i }))} target={rules?.endPhase ?? null} />
          {rules?.status && <RulesStatus text={rules.status} />}
          <span className="grow" />
          {!spectator && rules?.ending && (
            <button className="tb sm" style={{ flexShrink: 0 }} onClick={rules.cancelPass} title="Stop passing priority">
              Passing… ✕
            </button>
          )}
          {!spectator && (
            <button className="tb sm pri" style={{ flexShrink: 0 }} onClick={() => send({ type: 'passTurn' })}>
              {rules ? 'Pass' : 'Pass turn'}
            </button>
          )}
        </div>
      ) : (
      <div className="row" style={{ position: 'absolute', left: 0, top: 0, right: 0, height: TOPBAR, padding: '0 12px', background: 'var(--surface)', borderBottom: '1px solid var(--line3)', gap: 10, zIndex: 600 }}>
        <button className="tb sm" onClick={(e) => (rules ? openMenuBelow(e.currentTarget, rulesGameMenu) : exit())} aria-label={rules ? 'Game menu' : 'Back to main menu'} aria-haspopup={rules ? 'menu' : undefined}>
          ◇ Menu{rules ? ' ▾' : ''}
        </button>
        <div className="col hide-sm" style={{ width: 190, lineHeight: 1.2, minWidth: 0 }}>
          <div className="display" style={{ fontSize: 15, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>
            {title || v.settings.name}
          </div>
          <div style={{ fontSize: 11, color: 'var(--mu2)' }}>
            {formatName(v.settings.format)}
            {mode === 'local' ? '' : ` · Bo${v.settings.bo}`}
            {v.settings.bo === 3 ? ` · game ${v.match.game}` : ''} · {v.seats.length} seat{v.seats.length === 1 ? '' : 's'}
            {rules ? ' · rules enforced' : mode === 'local' ? ' · local' : spectator ? ' · spectating' : host ? ' · you host' : ''}
          </div>
        </div>
        <div className="row grow" style={{ justifyContent: 'center', gap: 10, minWidth: 0, overflow: 'hidden' }}>
          <div className="row" style={{ height: 30, padding: '0 12px', borderRadius: 8, background: 'var(--surface2)', border: `1px solid ${ap?.color}`, gap: 8, fontSize: 12, whiteSpace: 'nowrap', flexShrink: 0 }}>
            <span style={{ color: 'var(--mu2)' }}>Turn {v.turn.n}</span>
            <span className="dot" style={{ background: ap?.color }} />
            <span style={{ fontWeight: 600, maxWidth: 140, overflow: 'hidden', textOverflow: 'ellipsis' }}>{active === me && !solo ? 'Your turn' : ap?.name}</span>
          </div>
          <PhaseStrip phase={v.turn.phase} onPick={(i) => (rules ? rules.passTo(i) : send({ type: 'phase', phase: i }))} target={rules?.endPhase ?? null} />
          {rules?.status && <RulesStatus text={rules.status} />}
        </div>
        {!solo && mySeats.length > 1 && (
          <Select
            aria-label="Controlling seat"
            title="Which of your seats you are acting as"
            value={me ?? mySeats[0]}
            onChange={(n) => {
              useTable.setState({ sel: [], mullBottom: null });
              socket.send({ t: 'seat', seat: n });
            }}
            style={{ height: 30, fontSize: 12, borderColor: me != null ? v.seats[me]?.color : undefined }}
            options={mySeats.map((i) => ({ value: i, label: `As ${v.seats[i]?.name ?? i + 1} (seat ${i + 1})${v.turn.active === i ? ' · turn' : ''}`, icon: <span className="dot" style={{ background: v.seats[i]?.color }} /> }))}
          />
        )}
        {solo && (
          <div className="row" style={{ gap: 6 }}>
            <label className="row" style={{ gap: 6, fontSize: 12, color: 'var(--mu)' }} title="Which player you are acting as">
              <Select
                aria-label="Controlling seat"
                value={solo.seat}
                onChange={(n) => {
                  useTable.setState({ solo: { ...solo, seat: n }, sel: [], mullBottom: null });
                  useTable.getState().republish?.();
                }}
                style={{ height: 30, fontSize: 12, borderColor: v.seats[solo.seat]?.color }}
                options={v.seats.map((s2, i) => ({ value: i, label: `As ${s2.name}${v.turn.active === i ? ' · turn' : ''}`, icon: <span className="dot" style={{ background: s2.color }} /> }))}
              />
            </label>
            {soloSave && (
              <button
                className={'tb sm' + (soloSave.dirty || soloSave.id == null ? '' : ' on')}
                disabled={soloSave.saving}
                onClick={() => void soloSave.save()}
                title={soloSave.id != null ? `Saved as “${soloSave.name}” — changes autosave` : 'Save this solo game to continue it later'}
              >
                {soloSave.saving ? 'Saving…' : soloSave.id == null ? 'Save' : soloSave.dirty ? 'Save' : 'Saved ✓'}
              </button>
            )}
          </div>
        )}
        {!detailed && (
          <button className={'tb sm' + (logOpen ? ' on' : '')} onClick={() => useTable.setState({ logOpen: !logOpen })} aria-pressed={logOpen}>
            Log &amp; chat
          </button>
        )}
        <button className="tb sm" onClick={(e) => openMenuBelow(e.currentTarget, viewMenu)}>
          Settings
        </button>
        {rules && (
          <button className="tb sm" onClick={(e) => openMenuBelow(e.currentTarget, () => shortcutsMenu(prefs.hotkeys))} title="Keyboard and mouse shortcuts (?)">
            Shortcuts
          </button>
        )}
        {prefs.barDice && (
          <button className="tb sm" onClick={(e) => openMenuBelow(e.currentTarget, diceMenu)}>
            Dice
          </button>
        )}
        {specials && (
          <button className="tb sm" onClick={() => useTable.setState({ modal: { kind: 'specials' } })}>
            {planar ? 'Planechase' : 'Specials'}
          </button>
        )}
        {!spectator && !rules && (
          <button className="tb sm" onClick={() => useTable.setState({ modal: { kind: 'tokens', seat: me! } })}>
            Tokens
          </button>
        )}
        {(prefs.barDay || v.global.day) && (
          <button className="tb sm" title="Global day/night marker" onClick={() => send({ type: 'day', value: v.global.day === 'day' ? 'night' : 'day' })}>
            {v.global.day === 'night' ? '☾ Night' : '☀ Day'}
          </button>
        )}
        {(host || mode === 'local') && !rules && (
          <button className="tb sm" onClick={(e) => openMenuBelow(e.currentTarget, hostMenu)}>
            {mode === 'local' ? 'Playtest' : 'Host'}
          </button>
        )}
        {!spectator && (
          <>
            <button className="tb sm" onClick={doUndo} disabled={!!rules && !rules.canUndo} title={`Undo your last action (${prefs.hotkeys.undo})`}>
              Undo
              {!rules && (
                <span className="mono" style={{ color: 'var(--mu2)', fontSize: 10 }}>
                  {undoable}
                </span>
              )}
            </button>
            <button className="tb sm pri" onClick={() => send({ type: 'passTurn' })} title={prefs.hotkeys.pass}>
              {rules ? 'Pass priority' : 'Pass turn'}
            </button>
            {rules && (
              <button className={'tb sm' + (rules.ending ? ' pri' : '')} onClick={rules.ending ? rules.cancelPass : rules.endTurn} title="Pass priority until the turn ends, even when something goes on the stack; stops only for a question you must answer (blockers, a cost to pay, a choice). Esc stops it.">
                {rules.ending ? (rules.endPhase != null ? `Passing to ${PHASES[rules.endPhase]}… (cancel)` : 'Ending turn… (cancel)') : 'End turn'}
              </button>
            )}
          </>
        )}
      </div>
      )}
      {leaving && soloSave && (
        <Modal
          title="Leave without saving?"
          onClose={() => setLeaving(false)}
          width={460}
          foot={
            <>
              <button className="tb" onClick={() => setLeaving(false)} autoFocus>
                Stay
              </button>
              <button className="tb danger" onClick={onExit}>
                Leave
              </button>
              <button
                className="tb pri"
                onClick={async () => {
                  if (await soloSave.save()) onExit();
                }}
              >
                Save &amp; leave
              </button>
            </>
          }
        >
          <div style={{ padding: '16px 20px', fontSize: 13.5, color: 'var(--tx3)', lineHeight: 1.5 }}>This solo game hasn't been saved. Save it to continue later from Solo game or Load game.</div>
        </Modal>
      )}

      {/* Tab strip */}
      {tabLayout && (
        <div className="row" style={{ position: 'absolute', left: 0, top: TOPBAR, width: size.w, height: TABH, background: 'var(--area)', borderBottom: '1px solid var(--line3)', padding: '0 8px', gap: 4, zIndex: 5, alignItems: 'stretch', overflowX: 'auto', overflowY: 'hidden', scrollbarWidth: 'none' }}>
          {v.seats.map((p, i) =>
            i === me || (me == null && i === 0) ? null : (
              <button
                key={i}
                onClick={() => useTable.setState({ tabOpp: i })}
                onMouseEnter={() => setTabHover(i)}
                onMouseLeave={() => setTabHover(null)}
                className="row"
                style={{ border: 0, borderBottom: `2px solid ${g.order.includes(i) ? p.color : 'transparent'}`, background: g.order.includes(i) ? 'var(--surface2)' : 'transparent', color: 'var(--tx)', padding: '0 14px', gap: 8, fontSize: 12, cursor: 'pointer', flexShrink: 0, whiteSpace: 'nowrap' }}
              >
                <span className="dot" style={{ background: p.color }} />
                <b>{p.name}</b>
                <span style={{ color: 'var(--mu)' }}>♥ {p.life}</span>
                <span style={{ color: 'var(--mu2)' }}>Hand {p.zones.hand.length}</span>
                {(p.counters.poison || 0) > 0 && <span style={{ color: '#9bc76a' }}>Poison {p.counters.poison}</span>}
                {active === i && <span style={{ fontSize: 10, color: 'var(--acc-ink)', background: p.color, borderRadius: 4, padding: '1px 5px', fontWeight: 600 }}>TURN</span>}
              </button>
            ),
          )}
        </div>
      )}
      {tabLayout && tabHover != null && !g.order.includes(tabHover) && (
        <div style={{ position: 'absolute', left: 12, top: TOPBAR + TABH + 6, zIndex: 700, background: 'var(--surface3)', border: '1px solid var(--line4)', borderRadius: 10, padding: 10, maxWidth: 560, boxShadow: 'var(--shadow)' }}>
          <div style={{ fontSize: 12, marginBottom: 6 }}>
            {v.seats[tabHover].name}'s battlefield · {v.seats[tabHover].zones.battlefield.length} cards
          </div>
          <div className="row" style={{ flexWrap: 'wrap', gap: 4 }}>
            {v.seats[tabHover].zones.battlefield.slice(0, 40).map((iid) => {
              const c = v.cards[iid];
              return <CardImage key={iid} def={defOf(v, c)} back={!defOf(v, c) || c.faceDown} style={{ width: 44, height: 62, borderRadius: 3, transform: c.tapped ? 'rotate(90deg)' : undefined }} />;
            })}
          </div>
        </div>
      )}

      {/* Table area */}
      <div
        ref={setArea}
        style={{
          position: 'absolute',
          left: 0,
          top: TOPBAR,
          right: panelWidth,
          bottom: 0,
          overflow: free ? 'hidden' : undefined,
          // Free view: past the table's edges is plain border.
          background: free ? (prefs.theme === 'light' ? '#fff' : '#000') : undefined,
          cursor: camera.panning ? 'grabbing' : undefined,
        }}
      >
        {/* The table "world": transformed by the camera in the Free view, identity otherwise. */}
        <div
          style={{
            position: 'absolute',
            left: 0,
            top: 0,
            width: world.w,
            height: world.h,
            transformOrigin: '0 0',
            transform: free ? `translate(${camera.cam.x}px, ${camera.cam.y}px) scale(${camera.cam.k})` : undefined,
            background: free ? 'var(--bg-table)' : undefined,
            boxShadow: free ? '0 0 0 1px var(--line3)' : undefined,
          }}
        >
        {g.order.map((s) => {
          const A = g.areas[s];
          const p = v.seats[s];
          const tint = seatTint(p, prefs.tintPanels, prefs.theme === 'light', prefs.tableTheme);
          const isActive = active === s;
          const P = pzOf(pz, s);
          const mat = imgUrl(p.playmat);
          const chips: { label: string; val: string | number; color: string; inc?: () => void; dec?: () => void; title?: string }[] = [];
          const showZeroCmdTax = formatInfo(v.settings.format).commandZone;
          if (v.global.monarch === s) chips.push({ label: 'Monarch', val: 1, color: 'var(--acc)', title: 'This player is the Monarch' });
          if (v.global.initiative === s) chips.push({ label: 'Initiative', val: 1, color: '#56c1e8', title: 'This player has the Initiative' });
          if (p.citysBlessing) chips.push({ label: "City's blessing", val: 1, color: '#e0a84f', title: "This player has the city's blessing" });
          if ((p.counters.poison || 0) > 0) chips.push({ label: 'Poison', val: p.counters.poison, color: '#9bc76a', inc: () => send({ type: 'playerCounter', seat: s, key: 'poison', delta: 1 }), dec: () => send({ type: 'playerCounter', seat: s, key: 'poison', delta: -1 }) });
          for (const cmd of Object.keys(p.cmdTax)) {
            const tax = p.cmdTax[cmd] || 0;
            if (showZeroCmdTax || tax !== 0)
              chips.push({
                label: Object.keys(p.cmdTax).length > 1 ? `Tax · ${cardLabel(v, v.cards[cmd]).split(/[ ,]/)[0]}` : 'Cmdr tax',
                val: '+' + tax,
                color: 'var(--acc-tx)',
                title: 'Click +2 · right-click −2 (auto +2 when it leaves the command zone)',
                inc: () => send({ type: 'cmdTax', seat: s, iid: cmd, delta: 2 }),
                dec: () => send({ type: 'cmdTax', seat: s, iid: cmd, delta: -2 }),
              });
          }
          for (const [src, n] of Object.entries(p.cmdDmg))
            chips.push({ label: `Cmdr dmg · ${cardLabel(v, v.cards[src]).split(/[ ,]/)[0]}`, val: n, color: '#e0708f', inc: () => send({ type: 'cmdDmg', seat: s, source: src, delta: 1 }), dec: () => send({ type: 'cmdDmg', seat: s, source: src, delta: -1 }) });
          for (const [k, col] of [
            ['experience', '#9b8cf0'],
            ['energy', '#56c1e8'],
            ['rad', '#a3c85a'],
            ['tickets', '#e0a84f'],
          ] as const)
            if (p.counters[k]) chips.push({ label: k[0].toUpperCase() + k.slice(1), val: p.counters[k], color: col, inc: () => send({ type: 'playerCounter', seat: s, key: k, delta: 1 }), dec: () => send({ type: 'playerCounter', seat: s, key: k, delta: -1 }) });
          if (p.ring) chips.push({ label: 'Ring tempts', val: p.ring, color: '#e0a84f', inc: () => send({ type: 'ring', seat: s, delta: 1 }), dec: () => send({ type: 'ring', seat: s, delta: -1 }) });
          for (const m of v.global.markers.filter((x) => x.seat === s)) chips.push({ label: m.label, val: m.value, color: 'var(--tx3)', inc: () => send({ type: 'customMarker', seat: s, label: m.label, delta: 1 }), dec: () => send({ type: 'customMarker', seat: s, label: m.label, delta: -1 }) });
          const dl = dungeonLabel(p);
          const isMe = s === me;
          // Cards outside the game sit in a pile under the command zone, for their owner only.
          const showSide = p.zones.sideboard.length > 0 && (isMe || me == null);
          const chipTop = (showSide ? A.piles.sideboard : A.piles.command).y + A.piles.command.h + 8 - A.y;
          // Rules games: sizes are only how your screen draws the table, so every seat has them (the AI's too).
          const sizeBtns = isMe && !rules;
          const setLayout = (patch: { handFrac?: number; cardScale?: number }) => (rules ? rules.layout(s, patch) : send({ type: 'layout', ...patch }));
          // an archenemy's scheme row: the face-up schemes, and in a rules game the size of the face-down scheme deck
          const sideRow = p.zones.schemes.length > 0 || (!!rules && p.archenemy);
          const bottomH = 8 + (sizeBtns ? 52 : 0) + (sideRow ? 24 : 0);
          const chipsH = (rules && p.manaPool ? 76 : 0) + chips.length * 25 + (dl ? 26 : 0);
          const infoH = Math.max(A.h, chipTop + chipsH + bottomH + 4);
          const infoScrolls = infoH > A.h;
          const hot2 = (z: string) => hot?.t === 'pile' && hot.seat === s && hot.zone === z;
          return (
            <div key={s} data-seat={s} data-seat-name={p.name}>
              <div style={{ position: 'absolute', left: A.x, top: A.y, width: A.w, height: A.h, borderRight: '1px solid var(--line3)', borderBottom: A.top ? '2px solid var(--line4)' : 0, background: 'var(--area)' }} />
              {/* battlefield */}
              <div
                data-bf
                onPointerDown={(e) => onBfDown(e, s)}
                onWheel={(e) => onBfWheel(e, A)}
                onContextMenu={(e) => openMenuAt(e, () => battlefieldMenu(s, rules ? { setLayout: (patch) => rules.layout(s, patch), sort: () => rules.sortBoard(s), createToken: rules.breakRules.mine ? () => rules.openTokens(s) : undefined } : undefined))}
                style={{
                  position: 'absolute',
                  left: A.bf.x,
                  top: A.bf.y,
                  width: A.bf.w,
                  height: A.bf.h,
                  overflow: 'hidden',
                  // a playmat image the player uploaded wins over the table theme
                  ...(mat ? { backgroundColor: hexA(p.color, 0.045), backgroundImage: `linear-gradient(rgba(14,15,18,.35), rgba(14,15,18,.35)), url("${mat}")`, backgroundSize: 'cover', backgroundPosition: 'center' } : tint.mat),
                  touchAction: 'none',
                  cursor: pickSeats.includes(s) ? 'crosshair' : undefined,
                }}
              >
                {places.out
                  .filter((pl) => pl.seat === s && pl.zone === 'battlefield')
                  .map((pl) => {
                    const def = defOf(v, pl.c);
                    return (
                      <TCard
                        key={pl.c.iid}
                        c={pl.c}
                        def={def}
                        rect={pl.local}
                        rot={(pl.c.tapped ? 90 : 0) + (pl.c.flipped ? 180 : 0) + (prefs.flipOpponents && A.top ? 180 : 0)}
                        showFace={!!def && !pl.c.faceDown}
                        sleeve={v.seats[pl.c.owner]?.sleeve ?? null}
                        z={sel.includes(pl.c.iid) ? pl.z + 100 : pl.z}
                        selected={sel.includes(pl.c.iid)}
                        dim={!p.connected || p.conceded}
                        pinged={!!pings[pl.c.iid]}
                        hiddenForDrag={dragIds.includes(pl.c.iid)}
                        foreignColor={pl.c.owner !== s ? v.seats[pl.c.owner]?.color : null}
                        peek={pl.c.faceDown && !!def}
                        onBf
                        turned={!!turned[pl.c.iid]}
                        onTurn={() => turnCard(pl)}
                        transition={!prefs.reduceMotion}
                        onDown={(e) => onCardDown(e, pl)}
                        onCtx={(e) => openMenuAt(e, () => menuFor(pl))}
                        onDbl={() => activateCard(pl.c.iid, 'battlefield')}
                        onEnter={(e) => showPreview(e, pl)}
                        onLeave={(e) => !e.shiftKey && usePreview.getState().hide()}
                      />
                    );
                  })}
                {P.k !== 1 && (
                  <button className="mbtn" style={{ position: 'absolute', right: 6, top: 6, zIndex: 400 }} onPointerDown={(e) => e.stopPropagation()} onClick={() => useTable.setState({ pz: { ...pz, [s]: { x: 0, y: 0, k: 1 } } })}>
                    Fit all · {Math.round(P.k * 100)}%
                  </button>
                )}
              </div>
              {/* hand */}
              <div
                data-hand
                onPointerDown={(e) => onHandDown(e, s)}
                onContextMenu={(e) => openMenuAt(e, () => handMenu(s))}
                style={{ position: 'absolute', left: A.hand.x, top: A.hand.y, width: A.hand.w, height: A.hand.h, background: tint.hand, borderTop: A.top ? 0 : '1px dashed var(--line)', borderBottom: A.top ? '1px dashed var(--line)' : 0 }}
              >
                <div style={{ position: 'absolute', right: 8, [A.top ? 'bottom' : 'top']: 3, fontSize: 10.5, color: 'var(--mu3)', pointerEvents: 'none', zIndex: 300 }}>
                  {isMe ? 'Your hand' : `${p.name}'s hand`} · {p.zones.hand.length}
                  {me != null && p.handViewers.includes(me) ? ' · viewing (granted)' : ''}
                  {p.handRevealAll ? ' · revealed' : ''}
                </div>
                {places.out
                  .filter((pl) => pl.seat === s && pl.zone === 'hand')
                  .map((pl) => {
                    const def = defOf(v, pl.c);
                    return (
                      <TCard
                        key={pl.c.iid}
                        c={pl.c}
                        def={def}
                        rect={pl.local}
                        rot={0}
                        showFace={!!def}
                        sleeve={v.seats[pl.c.owner]?.sleeve ?? null}
                        z={pl.z}
                        selected={sel.includes(pl.c.iid)}
                        dim={false}
                        pinged={!!pings[pl.c.iid]}
                        hiddenForDrag={dragIds.includes(pl.c.iid)}
                        foreignColor={pl.c.owner !== s && def ? v.seats[pl.c.owner]?.color : null}
                        peek={false}
                        onBf={false}
                        transition={false}
                        mulliganPick={!!mullBottom?.includes(pl.c.iid)}
                        turned={!!turned[pl.c.iid]}
                        onTurn={() => turnCard(pl)}
                        onDown={(e) => onCardDown(e, pl)}
                        onCtx={(e) => openMenuAt(e, () => menuFor(pl))}
                        onDbl={() => activateCard(pl.c.iid, 'hand')}
                        onEnter={(e) => showPreview(e, pl)}
                        onLeave={(e) => !e.shiftKey && usePreview.getState().hide()}
                      />
                    );
                  })}
              </div>
              {pickSeats.includes(s) && (
                <div data-attack-target={s} style={{ position: 'absolute', left: A.x, top: A.y, width: A.w, height: A.h, border: '2px solid var(--bad, #e4787a)', boxShadow: 'inset 0 0 24px rgba(228,120,122,.25)', pointerEvents: 'none', zIndex: 450 }} />
              )}
              {/* info column: scrolls when it is taller than the seat (phones, landscape) */}
              <div
                data-info={s}
                // choosing who an attacker attacks: the whole panel picks this player, and its life box, piles and chips don't react
                onPointerDownCapture={(e) => {
                  const st = useTable.getState();
                  if (st.pickSeat && st.pickSeats.includes(s)) e.stopPropagation();
                }}
                onClickCapture={(e) => {
                  const st = useTable.getState();
                  if (!st.pickSeat || !st.pickSeats.includes(s)) return;
                  e.preventDefault();
                  e.stopPropagation();
                  st.pickSeat(s);
                }}
                onScroll={(e) => {
                  const top = e.currentTarget.scrollTop;
                  setInfoScroll((m) => (m[s] === top ? m : { ...m, [s]: top }));
                }}
                style={{ position: 'absolute', left: A.x, top: A.y, width: 104, height: A.h, background: tint.info, borderRight: '1px solid var(--line2)', overflowX: 'hidden', overflowY: infoScrolls ? 'auto' : 'hidden', overscrollBehavior: 'contain', scrollbarWidth: 'none' }}
              >
                <div style={{ position: 'relative', height: infoH, cursor: pickSeats.includes(s) ? 'crosshair' : undefined }}>
                  <div className="ic row" onClick={(e) => openMenuAt(e, () => playerMenu(s))} onContextMenu={(e) => openMenuAt(e, () => playerMenu(s))} title="Player menu" style={{ position: 'absolute', left: 6, top: 7, width: 92, height: 24, borderRadius: 6, padding: '0 5px', gap: 5, cursor: 'pointer' }}>
                    <span style={{ width: 9, height: 9, borderRadius: 5, background: p.color, flexShrink: 0 }} />
                    <span style={{ fontSize: 12.5, fontWeight: 600, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis', flexGrow: 1 }}>
                      {p.name}
                      {isMe ? ' (you)' : ''}
                    </span>
                  </div>
                  <LifeBox seat={p} life={p.life} hist={p.lifeHist} wheel={!free} onDelta={(d) => send({ type: 'life', seat: s, delta: d })} onSet={(n) => send({ type: 'life', seat: s, set: n })} />
                  <div className="col" data-left-scroll style={{ position: 'absolute', left: 4, top: chipTop, bottom: bottomH, width: 96, gap: 3, overflowY: infoScrolls ? 'visible' : 'auto', overscrollBehavior: 'contain', scrollbarWidth: 'thin' }}>
                    {rules && p.manaPool && <ManaPoolBox pool={p.manaPool} usable={isMe ? payPool?.usable ?? [] : []} pay={isMe ? payPool?.pay ?? null : null} label={p.name} />}
                    {chips.map((ch, i) => (
                      <div
                        key={i}
                        className="ic row"
                        title={ch.title || (ch.inc ? 'Click +1 · right-click −1' : undefined)}
                        onClick={ch.inc}
                        onContextMenu={(e) => {
                          e.preventDefault();
                          e.stopPropagation();
                          ch.dec?.();
                        }}
                        style={{ height: 22, flexShrink: 0, borderRadius: 6, background: 'var(--surface2)', border: '1px solid var(--line)', padding: '0 6px', gap: 4, fontSize: 10.5, cursor: ch.inc ? 'pointer' : undefined }}
                      >
                        <span className="grow" style={{ color: ch.color, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>
                          {ch.label}
                        </span>
                        <span className="mono" style={{ fontWeight: 600 }}>
                          {ch.val}
                        </span>
                      </div>
                    ))}
                    {dl && (
                      <div className="ic" onClick={(e) => openMenuAt(e, () => playerMenu(s))} title="Dungeon" style={{ fontSize: 10.5, padding: '3px 6px', borderRadius: 6, background: 'var(--surface2)', border: '1px solid var(--line)', cursor: 'pointer', color: 'var(--tx3)' }}>
                        ⌂ {dl}
                      </div>
                    )}
                  </div>
                  {/* piles */}
                  {(showSide ? (['library', 'graveyard', 'exile', 'command', 'sideboard'] as const) : (['library', 'graveyard', 'exile', 'command'] as const)).map((z) => {
                    const r = A.piles[z];
                    const lr = { ...r, x: r.x - A.x, y: r.y - A.y };
                    const list = p.zones[z];
                    const top = z === 'library' || z === 'command' ? list[0] : list[list.length - 1];
                    const topCard = top ? v.cards[top] : undefined;
                    const hiddenTop = !topCard || topCard.hidden || topCard.faceDown;
                    const cmdName = z === 'command' && topCard && !topCard.hidden ? cardLabel(v, topCard).split(',')[0] : null;
                    return (
                      <Pile
                        key={z}
                        r={lr}
                        touchAction={infoScrolls ? 'pan-y' : 'none'}
                        label={z === 'library' ? 'Library' : z === 'graveyard' ? 'Graveyard' : z === 'exile' ? 'Exile' : z === 'sideboard' ? 'Sideboard' : 'Command'}
                        count={z === 'command' ? (cmdName ? (list.length > 1 ? `${cmdName} +${list.length - 1}` : cmdName) : list.length ? String(list.length) : '—') : String(list.length)}
                        thumb={hiddenTop ? (p.sleeve ? imgUrl(p.sleeve) : null) : pileThumb(v, top)}
                        back={z === 'library' ? hiddenTop : !!topCard && hiddenTop}
                        hot={hot2(z)}
                        title={z === 'library' ? (p.revealTop && topCard && !topCard.hidden ? `Top revealed: ${cardLabel(v, topCard)} · click for actions` : 'Click for library actions · drag to draw') : `Click to open${z === 'command' ? ' · drag to cast' : ''}`}
                        onDown={(e) => (z === 'sideboard' ? undefined : onPileDown(e, s, z, r))}
                        // a library whose top card is revealed: hovering shows that card, clicking opens it like a graveyard
                        onEnter={z === 'library' && p.revealTop && topCard && !topCard.hidden ? (e) => showPreview(e, { c: topCard, seat: s, zone: 'library', rect: lr, local: lr, z: 0 }) : undefined}
                        onLeave={z === 'library' && p.revealTop && topCard && !topCard.hidden ? (e) => !e.shiftKey && usePreview.getState().hide() : undefined}
                        onClick={(e) => {
                          if (drag?.moved) return;
                          if (z === 'library' && rules && p.revealTop && topCard && !topCard.hidden) useTable.setState({ modal: { kind: 'zone', seat: s, zone: 'library' } });
                          else if (z === 'library') openMenuAt(e, () => libraryMenu(s));
                          else useTable.setState({ modal: { kind: 'zone', seat: s, zone: z } });
                        }}
                        onCtx={(e) => (z === 'library' ? openMenuAt(e, () => libraryMenu(s)) : (e.preventDefault(), useTable.setState({ modal: { kind: 'zone', seat: s, zone: z } })))}
                      />
                    );
                  })}
                  {(sideRow || sizeBtns) && (
                    <div className="col" style={{ position: 'absolute', left: 8, bottom: 8, width: 88, gap: 4 }}>
                      {sideRow && (
                        <div className="row" style={{ gap: 4 }}>
                          <button
                            className="mbtn grow"
                            style={{ height: 20, fontSize: 10 }}
                            disabled={p.zones.schemes.length === 0}
                            title={`${p.zones.schemes.length} face-up scheme(s)${rules ? ` · ${p.zones.schemeDeck.length} left in the scheme deck` : ''}`}
                            onClick={() => useTable.setState({ modal: { kind: 'zone', seat: s, zone: 'schemes' } })}
                          >
                            Schemes {p.zones.schemes.length}{rules ? ` · ${p.zones.schemeDeck.length}` : ''}
                          </button>
                        </div>
                      )}
                      {sizeBtns && (
                        <div className="col" style={{ gap: 4 }} title={rules ? 'How big this player\'s cards and hand are drawn on your screen' : 'Only you can change these; everyone sees the result'}>
                          <div className="row" style={{ gap: 4, fontSize: 10, color: 'var(--mu2)' }}>
                            <span className="grow" title={`Card size ${Math.round(p.cardScale * 100)}%`}>
                              Cards
                            </span>
                            <button className="sq sm" aria-label="Smaller cards" onClick={() => setLayout({ cardScale: Math.max(0.6, Math.round((p.cardScale - 0.1) * 10) / 10) })}>
                              −
                            </button>
                            <button className="sq sm" aria-label="Bigger cards" onClick={() => setLayout({ cardScale: Math.min(1.6, Math.round((p.cardScale + 0.1) * 10) / 10) })}>
                              +
                            </button>
                          </div>
                          <div className="row" style={{ gap: 4, fontSize: 10, color: 'var(--mu2)' }}>
                            <span className="grow">Hand</span>
                            <button className="sq sm" aria-label="Smaller hand" onClick={() => setLayout({ handFrac: Math.max(0.16, p.handFrac - 0.04) })}>
                              −
                            </button>
                            <button className="sq sm" aria-label="Bigger hand" onClick={() => setLayout({ handFrac: Math.min(0.44, p.handFrac + 0.04) })}>
                              +
                            </button>
                          </div>
                        </div>
                      )}
                    </div>
                  )}
                </div>
              </div>
              <div style={{ position: 'absolute', left: A.x, top: A.y, width: A.w, height: A.h, pointerEvents: 'none', boxShadow: isActive ? `inset 0 0 0 2px ${hexA(p.color, 0.9)}, inset 0 0 26px ${hexA(p.color, 0.18)}` : 'none', zIndex: 150 }} />
              {(!p.connected || p.conceded) && (
                <div className="col" style={{ position: 'absolute', left: A.x, top: A.y, width: A.w, height: A.h, background: 'rgba(14,15,18,.62)', alignItems: 'center', justifyContent: 'center', gap: 10, zIndex: 180, pointerEvents: p.conceded ? 'none' : 'auto' }}>
                  <div style={{ fontSize: 13, color: 'var(--tx3)' }}>{p.conceded ? (rules ? `${p.name} is out of the game · their cards left with them` : `${p.name} conceded · cards stay in place`) : `${p.name} disconnected · reconnecting automatically`}</div>
                  {!p.connected && !p.conceded && host && mode === 'online' && (
                    <div className="row" style={{ gap: 6 }}>
                      <button className="mbtn" onClick={() => useTable.setState({ modal: { kind: 'giveSeat', seat: s } })}>
                        Give seat to…
                      </button>
                      <button className="mbtn" onClick={() => void askConfirm({ title: `Remove ${p.name} from the game?`, confirm: 'Remove', danger: true }).then((ok) => ok && socket.send({ t: 'host', op: 'removeSeat', seat: s }))}>
                        Remove
                      </button>
                    </div>
                  )}
                </div>
              )}
            </div>
          );
        })}

        {/* planes in play (small) */}
        {v.table.planes[0] && v.cards[v.table.planes[0]] && (
          <div
            title={rules ? `Current plane · ${v.table.planarDeck.length} cards in the planar deck · click to roll the planar die` : 'Current plane · click for Planechase'}
            onClick={(e) => {
              if (!rules) return useTable.setState({ modal: { kind: 'specials' } });
              // a rules game: the planar die roll is an action of the plane (offered when you may roll)
              const r = e.currentTarget.getBoundingClientRect();
              const acts = rules.abilities(v.table.planes[0]);
              useMenu.getState().open(r.left, r.bottom + 4, () =>
                acts.length ? acts.map((a) => ({ kind: 'item' as const, label: a.label, run: a.run })) : [{ kind: 'item' as const, label: 'You can roll the planar die on your turn, when you could cast a sorcery', disabled: true, run: () => {} }],
              );
            }}
            style={{ position: 'absolute', right: 10, top: '50%', transform: 'translateY(-50%)', zIndex: 170, cursor: 'pointer', width: 112, height: 80, overflow: 'hidden', borderRadius: 8, boxShadow: '0 6px 18px rgba(0,0,0,.5)' }}
            onMouseEnter={(e) => usePreview.getState().show(v.defs[v.cards[v.table.planes[0]].def], e.currentTarget)}
            onMouseLeave={() => usePreview.getState().hide()}
          >
            <CardImage def={v.defs[v.cards[v.table.planes[0]].def]} style={{ width: 80, height: 112, transform: 'rotate(90deg) translate(-16px, -16px)', transformOrigin: '40px 40px' }} />
          </div>
        )}

        {/* drop highlight */}
        {hot && hot.t !== 'info' && <div style={{ position: 'absolute', left: hot.r.x, top: hot.r.y, width: hot.r.w, height: hot.r.h, border: '2px dashed var(--acc)', borderRadius: 6, background: 'rgba(var(--acc-rgb),.07)', pointerEvents: 'none', zIndex: 350 }} />}

        {/* box select */}
        {box && (
          <div style={{ position: 'absolute', left: Math.min(box.x0, box.x1), top: Math.min(box.y0, box.y1), width: Math.abs(box.x1 - box.x0), height: Math.abs(box.y1 - box.y0), border: '1px solid var(--acc)', background: 'rgba(var(--acc-rgb),.1)', pointerEvents: 'none', zIndex: 400 }} />
        )}

        {/* stack offer */}
        {offer && sel.length === offer.iids.length && (
          <div className="row" onPointerDown={(e) => e.stopPropagation()} style={{ position: 'absolute', left: Math.max(4, Math.min(offer.x - 150, world.w - 380)), top: Math.min(offer.y + 6, world.h - 44), zIndex: 610, gap: 4, padding: 4, borderRadius: 9, background: 'var(--surface3)', border: '1px solid var(--line4)', boxShadow: '0 10px 24px rgba(0,0,0,.5)' }}>
            <button
              className="tb sm pri"
              onClick={() => {
                send({ type: 'stack', iids: offer.iids });
                useTable.setState({ sel: [offer.iids[0]] });
                setOffer(null);
              }}
            >
              Stack {offer.iids.reduce((n, i) => n + (v.cards[i]?.pile || 1), 0)} {cardLabel(v, v.cards[offer.iids[0]])} into a pile
            </button>
            <button className="tb sm" onClick={() => setOffer(null)}>
              Not now
            </button>
          </div>
        )}

        {/* drag ghost */}
        {ghost && (
          <div style={{ position: 'absolute', left: ghost.r.x, top: ghost.r.y, width: ghost.r.w, height: ghost.r.h, zIndex: 650, pointerEvents: 'none', borderRadius: 6, boxShadow: '0 14px 30px rgba(0,0,0,.6)', transform: ghost.rot ? `rotate(${ghost.rot}deg)` : undefined, transition: 'width .08s, height .08s' }}>
            <CardImage def={ghost.face ? ghost.def : undefined} back={!ghost.face} sleeve={ghost.sleeve} style={{ width: '100%', height: '100%', borderRadius: 6 }} />
            {dragIds.length > 1 && <div style={{ position: 'absolute', right: -8, top: -8, fontSize: 11, fontWeight: 700, padding: '2px 7px', borderRadius: 9, background: 'var(--acc)', color: 'var(--acc-ink)' }}>+{dragIds.length - 1}</div>}
          </div>
        )}

        {/* arrows */}
        <svg width={world.w} height={world.h} style={{ position: 'absolute', left: 0, top: 0, pointerEvents: 'none', zIndex: 620 }} aria-hidden="true">
          {arrows.map((a) => {
            const f = a.from.iid ? cardPos(a.from.iid) : null;
            const t = a.to.iid ? cardPos(a.to.iid) : null;
            const p1 = f ? { x: f.x + f.w / 2, y: f.y + f.h / 2 } : a.from.seat != null ? seatCenter(a.from.seat) : null;
            const p2 = t ? { x: t.x + t.w / 2, y: t.y + t.h / 2 } : a.to.seat != null ? seatCenter(a.to.seat) : null;
            if (!p1 || !p2) return null;
            const ang = Math.atan2(p2.y - p1.y, p2.x - p1.x);
            const L = 16;
            const head = `${p2.x},${p2.y} ${p2.x - L * Math.cos(ang - 0.45)},${p2.y - L * Math.sin(ang - 0.45)} ${p2.x - L * Math.cos(ang + 0.45)},${p2.y - L * Math.sin(ang + 0.45)}`;
            return (
              <g key={a.id} className="tarrow">
                <line
                  x1={p1.x}
                  y1={p1.y}
                  x2={p2.x}
                  y2={p2.y}
                  stroke={a.color}
                  strokeWidth={5}
                  strokeLinecap="round"
                  style={{ pointerEvents: 'stroke', cursor: 'context-menu' }}
                  onContextMenu={(e) => {
                    e.preventDefault();
                    if (mode === 'local') useTable.setState({ arrows: useTable.getState().arrows.filter((x) => x.id !== a.id) });
                    else socket.send({ t: 'arrowRemove', id: a.id });
                  }}
                />
                <polygon points={head} fill={a.color} />
              </g>
            );
          })}
          {[...combatArrows.filter((a) => !liveArrows.some((l) => l.kind === a.kind && l.from.iid === a.from.iid)), ...liveArrows].map((a) => {
            const f = a.from.iid ? cardPos(a.from.iid) : null;
            const t = a.to.iid ? cardPos(a.to.iid) : null;
            const p1 = f ? { x: f.x + f.w / 2, y: f.y + f.h / 2 } : a.from.seat != null ? seatCenter(a.from.seat) : null;
            const p2 = t ? { x: t.x + t.w / 2, y: t.y + t.h / 2 } : a.to.seat != null ? seatCenter(a.to.seat) : null;
            if (!p1 || !p2) return null;
            const col = a.illegal ? '#8b8f98' : a.kind === 'attack' ? '#e5484d' : '#3b9eff';
            const ang = Math.atan2(p2.y - p1.y, p2.x - p1.x);
            // stop at the target's edge so the head isn't hidden under the card
            const end = t ? { x: p2.x - Math.cos(ang) * Math.min(t.w, t.h) * 0.4, y: p2.y - Math.sin(ang) * Math.min(t.w, t.h) * 0.4 } : p2;
            const L = 18;
            const head = `${end.x},${end.y} ${end.x - L * Math.cos(ang - 0.42)},${end.y - L * Math.sin(ang - 0.42)} ${end.x - L * Math.cos(ang + 0.42)},${end.y - L * Math.sin(ang + 0.42)}`;
            return (
              <g key={a.kind + a.id + (a.draft ? 'd' : '') + (a.illegal ? 'x' : '')} opacity={a.illegal ? 0.6 : a.draft ? 0.75 : 1}>
                <line x1={p1.x} y1={p1.y} x2={end.x} y2={end.y} stroke="#000" strokeOpacity={0.45} strokeWidth={8} strokeLinecap="round" />
                <line x1={p1.x} y1={p1.y} x2={end.x} y2={end.y} stroke={col} strokeWidth={5} strokeLinecap="round" strokeDasharray={a.illegal ? '3 7' : a.draft ? '9 7' : undefined} />
                <polygon points={head} fill={col} stroke="#000" strokeOpacity={0.45} strokeWidth={1.5} />
              </g>
            );
          })}
          {arrowDraft && <line x1={arrowDraft.x1} y1={arrowDraft.y1} x2={arrowDraft.x2} y2={arrowDraft.y2} stroke="var(--acc)" strokeWidth={3} strokeDasharray="7 6" strokeLinecap="round" />}
        </svg>
        </div>

        {free && (
          <div className="row" data-nopan style={{ position: 'absolute', right: 12, bottom: 12, zIndex: 630, gap: 4, padding: 4, borderRadius: 10, background: 'var(--surface3)', border: '1px solid var(--line4)', boxShadow: 'var(--shadow)' }} onPointerDown={(e) => e.stopPropagation()}>
            <button className="sq sm" aria-label="Zoom out" onClick={() => camera.zoomBy(1 / 1.25)}>
              −
            </button>
            <button className="mbtn" onClick={camera.reset} title="Fit the whole table on screen" style={{ minWidth: 58 }}>
              {Math.round(camera.cam.k * 100)}%
            </button>
            <button className="sq sm" aria-label="Zoom in" onClick={() => camera.zoomBy(1.25)}>
              +
            </button>
            <span className="hide-sm" style={{ fontSize: 11, color: 'var(--mu2)', padding: '0 6px' }}>
              Wheel zoom · WASD / arrows / right-drag pan · click % to fit
            </span>
          </div>
        )}

        {v.stage === 'ended' && (
          <div className="row" style={{ position: 'absolute', left: '50%', top: '50%', transform: 'translate(-50%, -50%)', zIndex: 640, padding: '16px 22px', borderRadius: 14, background: 'var(--surface3)', border: '1px solid var(--acc)', boxShadow: 'var(--shadow)', fontSize: 15, flexWrap: 'wrap', justifyContent: 'center' }}>
            <b className="display" style={{ fontSize: 22 }}>{v.winner != null && v.winner === mySeat(v) ? 'You won!' : 'Game over'}</b>
            {v.winner != null ? ` · ${v.seats[v.winner].name} won` : ''}
            {v.settings.bo === 3 ? ` · match ${v.seats.map((s, i) => `${s.name} ${v.match.wins[i] || 0}`).join('–')}` : ''}
            {mode !== 'local' && <AfterGameButtons gameId={v.id} small />}
            {host && (
              <>
                <button className="tb sm" onClick={() => socket.send({ t: 'host', op: 'rematch' })}>
                  Rematch
                </button>
                <button className="tb sm" onClick={() => socket.send({ t: 'host', op: 'close' })}>
                  Close game
                </button>
              </>
            )}
          </div>
        )}
      </div>

      {/* Log drawer (minimal) */}
      {!detailed && logOpen && (
        <div className="col" onPointerDown={(e) => e.stopPropagation()} style={{ position: 'absolute', right: compact ? 8 : 12, top: TOPBAR + 8, width: compact ? 'calc(100vw - 16px)' : 380, height: 440, maxHeight: 'calc(100vh - 70px)', zIndex: 560, background: 'var(--surface3)', border: '1px solid var(--line4)', borderRadius: 12, boxShadow: '0 16px 40px rgba(0,0,0,.55)', overflow: 'hidden' }}>
          <div className="row" style={{ gap: 4, padding: '9px 10px', borderBottom: '1px solid var(--line)' }}>
            <span className="grow" style={{ fontSize: 12.5, fontWeight: 600 }}>
              Log &amp; chat
            </span>
            <LogFilters value={logFilter} onChange={(f) => useTable.setState({ logFilter: f })} />
          </div>
          <LogList log={log} filter={logFilter} seats={v.seats} />
          <ChatInput onSend={(t) => (spectator ? socket.send({ t: 'specChat', text: t }) : send({ type: 'chat', text: t }))} />
        </div>
      )}

      {detailed && <DetailedPanel width={panelW} onWidth={(w) => setPanelW(w)} onWidthDone={(w) => setPrefs({ panelW: w })} />}
      {peek && places.byIid.get(peek.iid) && (
        <CardSheet
          p={places.byIid.get(peek.iid)!}
          openedAt={peek.at}
          me={me}
          extra={previewExtra}
          onClose={() => setPeek(null)}
          onTurn={turnCard}
          onMenu={(p, x, y) => {
            setPeek(null);
            useMenu.getState().open(x, y, () => menuFor(p));
          }}
          onPlay={(p) => {
            setPeek(null);
            if (me != null) send({ type: 'move', iids: [p.c.iid], seat: me, zone: 'battlefield', x: 0.45, y: isTopRowSeat(me, v.seats.length) ? 0.6 : 0.35 });
          }}
        />
      )}
      <TableModals />
    </div>
  );
}

// ------------------------------------------------------------------ touch card sheet

/** Touch: a tapped card shown large with its common actions (there is no hover, double-click or right-click). */
function CardSheet({ p, openedAt, me, extra, onClose, onTurn, onMenu, onPlay }: { p: CardPlace; openedAt: number; me: number | null; extra: (p: CardPlace, def: CardDef) => string | undefined; onClose: () => void; onTurn: (p: CardPlace) => void; onMenu: (p: CardPlace, x: number, y: number) => void; onPlay: (p: CardPlace) => void }) {
  const v = useTable((s) => s.view)!;
  // The game is asking you to choose cards (blocking, targets…) and this is one of them.
  const pick = useTable((s) => (s.pick && s.pickIds.includes(p.c.iid) ? s.pick : null));
  const picked = useTable((s) => s.sel.includes(p.c.iid));
  const turned = useTurned((s) => !!s.m[p.c.iid]);
  const def = defOf(v, p.c);
  if (!def) return null;
  const baseFace = p.c.flipped ? 1 : p.c.face;
  const face = turned ? (baseFace ? 0 : 1) : baseFace;
  const sideways = faceSideways(def, turnKind(def) === 'dfc' ? face : 0);
  // As large as fits with room for the buttons: below the card, or beside it on a phone held sideways.
  const wide = window.innerWidth > window.innerHeight;
  const W = Math.round(wide ? Math.min(360, ((window.innerHeight - 40) * 63) / 88, (window.innerWidth - 220) * (sideways ? 63 / 88 : 1)) : Math.min(360, (window.innerWidth - 40) * (sideways ? 63 / 88 : 1), ((window.innerHeight - 190) * 63) / 88));
  const H = Math.round((W * 88) / 63);
  const boxW = sideways ? H : W;
  const boxH = sideways ? W : H;
  const onBf = p.zone === 'battlefield';
  const note = extra(p, def);
  // The tap that opened the sheet also produces a click; ignore it so the sheet doesn't close at once.
  const close = () => Date.now() - openedAt > 350 && onClose();
  return (
    <div onPointerDown={(e) => e.stopPropagation()} onClick={close} style={{ position: 'fixed', inset: 0, zIndex: 940, background: 'rgba(8,9,11,.72)', display: 'flex', flexDirection: wide ? 'row' : 'column', alignItems: 'center', justifyContent: 'center', gap: 12, padding: 16 }}>
      <div style={{ width: boxW, height: boxH, position: 'relative', flexShrink: 0 }}>
        <div style={{ width: W, height: H, position: 'absolute', left: (boxW - W) / 2, top: (boxH - H) / 2, transform: sideways ? 'rotate(90deg)' : undefined }}>
          <CardImage key={def.key} def={def} face={face} size="large" style={{ width: W, height: H, boxShadow: '0 18px 40px rgba(0,0,0,.65)', borderRadius: '4.75% / 3.4%' }} />
          <LiveOverlays iid={p.c.iid} w={W} onBf={onBf} />
        </div>
      </div>
      <div className="col" style={{ gap: 12, alignItems: 'center', maxWidth: wide ? 200 : undefined }}>
      {note && <div style={{ maxWidth: wide ? 200 : boxW, background: 'var(--surface3)', border: '1px solid var(--line4)', borderRadius: 8, padding: '7px 9px', fontSize: 12, lineHeight: 1.45, color: 'var(--tx2)', whiteSpace: 'pre-line' }}>{note}</div>}
      <div className="row" style={{ gap: 8, flexWrap: 'wrap', justifyContent: 'center' }} onClick={(e) => e.stopPropagation()}>
        {pick && (
          <button
            className="tb pri"
            onClick={() => {
              pick(p.c.iid);
              onClose();
            }}
          >
            {picked ? 'Unchoose' : 'Choose'}
          </button>
        )}
        {onBf && me != null && !pick && (
          <button className="tb pri" onClick={() => send({ type: 'tap', iids: [p.c.iid] })}>
            {p.c.tapped ? 'Untap' : 'Tap'}
          </button>
        )}
        {p.zone === 'hand' && p.seat === me && !pick && (
          <button className="tb pri" onClick={() => onPlay(p)}>
            Play
          </button>
        )}
        {turnKind(def) && (
          <button className="tb" onClick={() => onTurn(p)}>
            Turn over
          </button>
        )}
        <button className="tb" onClick={(e) => onMenu(p, e.clientX, e.clientY)}>
          Actions…
        </button>
        <button className="tb" onClick={onClose}>
          Close
        </button>
      </div>
      </div>
    </div>
  );
}

// ------------------------------------------------------------------ settings menu

function soloItems(): MenuItem[] {
  const so = useTable.getState().solo;
  if (!so) return [];
  const set = (patch: Partial<typeof so>) => {
    useTable.setState({ solo: { ...so, ...patch } });
    useTable.getState().republish?.();
    useMenu.getState().refresh();
  };
  const sv = useTable.getState().soloSave;
  const saveItems: MenuItem[] = sv
    ? [
        { kind: 'sep' },
        { kind: 'head', label: sv.id != null ? `Saved as “${sv.name}”${sv.savedAt ? ' · ' + timeAgo(sv.savedAt) : ''}` : 'Not saved yet' },
        { kind: 'item', label: sv.id != null ? 'Save now' : 'Save game', hint: sv.id != null ? 'autosaves' : undefined, run: () => void sv.save() },
        { kind: 'input', label: sv.id != null ? 'Save as a new copy' : 'Save with a name', placeholder: sv.name || 'Name this game', button: 'Save', submit: (name) => void sv.save({ asNew: sv.id != null, name }) },
        ...(sv.id != null ? [{ kind: 'input', label: 'Rename', placeholder: sv.name, button: 'Rename', submit: (name: string) => void sv.save({ name }) } as MenuItem] : []),
      ]
    : [];
  return [
    { kind: 'head', label: 'Solo game' },
    { kind: 'item', label: 'Control follows the turn', checked: so.follow, run: () => set({ follow: !so.follow }), keep: true },
    { kind: 'item', label: 'Show all hands', checked: so.revealAll, run: () => set({ revealAll: !so.revealAll }), keep: true },
    ...saveItems,
    { kind: 'sep' },
  ];
}

/** Rules game: what the game is doing while you wait, with a spinner, so it never looks frozen. */
function RulesStatus({ text }: { text: string }) {
  const busy = !/^\d+ queued$/.test(text);
  return (
    <span className="row" role="status" aria-live="polite" style={{ height: 30, padding: '0 10px', borderRadius: 8, background: 'var(--surface2)', border: '1px solid var(--line4)', gap: 7, fontSize: 14, whiteSpace: 'nowrap', flexShrink: 0, color: 'var(--tx2, var(--tx))' }}>
      {busy && <span className="rspin" aria-hidden style={{ width: 12, height: 12, borderRadius: '50%', border: '2px solid var(--line5)', borderTopColor: 'var(--acc)', display: 'inline-block' }} />}
      {text}
    </span>
  );
}

/** Rules game: every keyboard and mouse shortcut (read-only list; the configurable keys are set in Settings → Hotkeys). */
function shortcutsMenu(hk: Record<string, string>): MenuItem[] {
  const row = (label: string, keys: string): MenuItem => ({ kind: 'item', label, hint: keys, run: () => {}, keep: true });
  return [
    { kind: 'head', label: 'Mouse' },
    row('Play a card / tap a land / activate', 'Double-click'),
    row('Same as a double-click (fast tapping)', 'Shift+click'),
    row('Add a card to the selection', 'Ctrl+click'),
    row('Card actions, sort a board, sizes', 'Right-click'),
    row('Pass priority until that phase', 'Click a phase'),
    row('Pass until that stack item is on top', 'Click a stack item'),
    { kind: 'sep' },
    { kind: 'head', label: 'Keyboard' },
    row('Pass priority', hk.pass),
    row('End turn', 'E'),
    row('Undo', hk.undo),
    row('Tap / activate the hovered or selected card', hk.tap),
    row('Sort your board', 'S'),
    row('Break the rules / finish', 'B'),
    row('Stop passing, clear queued actions', 'Esc'),
    { kind: 'sep' },
    { kind: 'head', label: 'While a question is open' },
    row('Confirm (the main button)', 'Enter'),
    row('Choose the n-th option', '1 – 9'),
    row('Auto-pay mana', 'A'),
    { kind: 'sep' },
    row('Show this list', '?'),
  ];
}

/** Seconds-per-action steps: 0.01 below 0.1 s, 0.1 up to 2 s, 0.5 above. */
function paceStep(v: number, dir: 1 | -1): number {
  const unit = dir > 0 ? (v < 0.1 ? 0.01 : v < 2 ? 0.1 : 0.5) : v <= 0.1 ? 0.01 : v <= 2 ? 0.1 : 0.5;
  return Math.round((v + dir * unit) * 100) / 100;
}

function viewMenu() {
  const app = useApp.getState();
  const p = app.me!.prefs;
  const set = (patch: Parameters<typeof app.setPrefs>[0]) => {
    app.setPrefs(patch);
    useMenu.getState().refresh();
  };
  const me = mySeat(useTable.getState().view);
  const seat = me === null ? null : useTable.getState().view!.seats[me];
  // Changes this game's seat and remembers it as the default for future games
  const turn = (key: 'autoUntap' | 'autoDraw', value: boolean) => {
    send({ type: key, value });
    set({ [key]: value });
  };
  return [
    ...soloItems(),
    { kind: 'head' as const, label: 'Layout' },
    { kind: 'item' as const, label: (p.layout === 'grid' ? '✓ ' : '   ') + 'Grid — everyone at once', run: () => set({ layout: 'grid' }) },
    { kind: 'item' as const, label: (p.layout === 'tab' ? '✓ ' : '   ') + 'Tab — one opponent at a time', run: () => set({ layout: 'tab' }) },
    { kind: 'item' as const, label: (p.layout === 'free' ? '✓ ' : '   ') + 'Free — pan & zoom the whole table', run: () => set({ layout: 'free' }) },
    { kind: 'sep' as const },
    { kind: 'head' as const, label: 'Panel' },
    { kind: 'item' as const, label: (p.panel === 'minimal' ? '✓ ' : '   ') + 'Minimal — popup preview', run: () => set({ panel: 'minimal' }) },
    { kind: 'item' as const, label: (p.panel === 'detailed' ? '✓ ' : '   ') + 'Detailed — side panel', run: () => set({ panel: 'detailed' }) },
    { kind: 'sep' as const },
    p.layout === 'free'
      ? { kind: 'item' as const, label: 'Hover preview (Free view)', checked: p.freePreview, run: () => set({ freePreview: !p.freePreview }), keep: true }
      : { kind: 'item' as const, label: 'Hover preview', checked: p.preview, run: () => set({ preview: !p.preview }), keep: true },
    { kind: 'number' as const, label: 'Preview size', value: p.previewSize, min: 60, max: 180, step: (x: number, dir: 1 | -1) => x + dir * 10, set: (x: number) => set({ previewSize: Math.round(x) }), format: (x: number) => `${x}%` },
    ...(useTable.getState().rules
      ? [
          { kind: 'sep' as const },
          { kind: 'head' as const, label: 'Rules game' },
          { kind: 'item' as const, label: 'Auto-tap lands to pay for spells', checked: p.autoTap, run: () => set({ autoTap: !p.autoTap }), keep: true },
          { kind: 'item' as const, label: 'Auto-sort the battlefields', checked: p.autoSort, run: () => set({ autoSort: !p.autoSort }), keep: true },
          { kind: 'item' as const, label: "Slow down other players' actions", checked: p.aiPaceOn, run: () => set({ aiPaceOn: !p.aiPaceOn }), keep: true },
          ...(p.aiPaceOn ? [{ kind: 'number' as const, label: 'Seconds per action', value: p.aiPaceSeconds, min: 0.01, step: paceStep, set: (x: number) => set({ aiPaceSeconds: Math.round(x * 100) / 100 }), format: (x: number) => `${x}s` }] : []),
        ]
      : []),
    { kind: 'sep' as const },
    { kind: 'head' as const, label: 'Top bar buttons' },
    { kind: 'item' as const, label: 'Dice', checked: p.barDice, run: () => set({ barDice: !p.barDice }), keep: true },
    { kind: 'item' as const, label: 'Day / Night', checked: p.barDay, run: () => set({ barDay: !p.barDay }), keep: true },
    { kind: 'sep' as const },
    { kind: 'item' as const, label: "Rotate opponents' cards 180°", checked: p.flipOpponents, run: () => set({ flipOpponents: !p.flipOpponents }), keep: true },
    { kind: 'item' as const, label: 'Tint panels in player colour', checked: p.tintPanels, run: () => set({ tintPanels: !p.tintPanels }), keep: true },
    { kind: 'sep' as const },
    { kind: 'head' as const, label: 'Table' },
    { kind: 'item' as const, label: 'Classic — lit in each player\u2019s colour', checked: p.tableTheme === 'classic', run: () => set({ tableTheme: 'classic' }), keep: true },
    { kind: 'item' as const, label: 'Felt — green baize', checked: p.tableTheme === 'felt', run: () => set({ tableTheme: 'felt' }), keep: true },
    { kind: 'item' as const, label: 'Playmats — a mat in each player\u2019s colour', checked: p.tableTheme === 'playmats', run: () => set({ tableTheme: 'playmats' }), keep: true },
    ...(seat
      ? [
          { kind: 'sep' as const },
          { kind: 'head' as const, label: 'Turn start' },
          { kind: 'item' as const, label: 'Auto untap', checked: seat.autoUntap, run: () => turn('autoUntap', !seat.autoUntap), keep: true },
          { kind: 'item' as const, label: 'Auto draw', checked: seat.autoDraw, run: () => turn('autoDraw', !seat.autoDraw), keep: true },
        ]
      : []),
  ];
}

// ------------------------------------------------------------------ detailed side panel

/** Stable ref callback: the rules game draws its prompts into this element. */
const setRulesSlot = (el: HTMLElement | null) => {
  if (useTable.getState().rulesSlot !== el) useTable.setState({ rulesSlot: el });
};

function DetailedPanel({ width, onWidth, onWidthDone }: { width: number; onWidth: (w: number) => void; onWidthDone: (w: number) => void }) {
  const v = useTable((s) => s.view)!;
  const log = useTable((s) => s.log);
  const lastHover = useTable((s) => s.lastHover);
  const hover = useTable((s) => s.hover);
  const logFilter = useTable((s) => s.logFilter);
  const panelTab = useTable((s) => s.panelTab);
  const rulesOn = useTable((s) => !!s.rules);
  const rulesAttention = useTable((s) => !!s.rules?.attention);
  const prefs = useApp((s) => s.me!.prefs);
  const me = mySeat(v);
  const [drag, setDrag] = useState(false);
  const [weights, setWeights] = useState<number[]>(() => {
    try {
      const w = JSON.parse(localStorage.getItem('aetherhub.panelWeights') || 'null');
      if (Array.isArray(w) && w.length === prefs.slots.length) return w;
    } catch {
      /* ignore */
    }
    return prefs.slots.map((s) => (s === 'spoiler' ? 3 : 2));
  });
  useEffect(() => {
    if (weights.length !== prefs.slots.length) setWeights(prefs.slots.map((s) => (s === 'spoiler' ? 3 : 2)));
  }, [prefs.slots, weights.length]);
  useEffect(() => {
    if (!drag) return;
    let w = width;
    const move = (e: PointerEvent) => {
      w = Math.round(Math.max(280, Math.min(560, window.innerWidth - e.clientX)));
      onWidth(w);
    };
    const up = () => {
      setDrag(false);
      onWidthDone(w);
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up);
    return () => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', up);
    };
  }, [drag]);
  const iid = hover && v.cards[hover] && defOf(v, v.cards[hover]) ? hover : lastHover;
  const c = iid ? v.cards[iid] : undefined;
  const def = defOf(v, c);
  const spectator = me == null;
  const sendChat = (t: string) => (spectator ? socket.send({ t: 'specChat', text: t }) : useTable.getState().send({ type: 'chat', text: t }));

  const section = (kind: string, i: number) => {
    if (kind === 'spoiler')
      return (
        <div className="col" style={{ flexGrow: 1, alignItems: 'center', justifyContent: 'center', gap: 8, padding: 10, minHeight: 0 }}>
          {def && c ? (
            // split cards, Rooms, battles and planes are read sideways: the preview turns (the card on the table does not)
            faceSideways(def, turnKind(def) === 'dfc' ? c.face : 0) ? (
              <div style={{ position: 'relative', width: '100%', maxWidth: 420, aspectRatio: '88 / 63' }}>
                <div style={{ position: 'absolute', left: '50%', top: '50%', width: `${(63 / 88) * 100}%`, height: `${(88 / 63) * 100}%`, transform: 'translate(-50%, -50%) rotate(90deg)' }}>
                  <CardImage def={def} face={c.face} flipped={c.flipped} size="large" style={{ width: '100%', height: '100%', borderRadius: '4.75% / 3.4%', boxShadow: '0 10px 28px rgba(0,0,0,.5)' }} />
                </div>
              </div>
            ) : (
              <div style={{ position: 'relative', height: '100%', maxHeight: 340, aspectRatio: '63 / 88' }}>
                <CardImage def={def} face={c.face} flipped={c.flipped} size="large" style={{ width: '100%', height: '100%', borderRadius: '4.75% / 3.4%', boxShadow: '0 10px 28px rgba(0,0,0,.5)' }} />
                <Overlays c={c} def={def} w={(Math.min(340, 300) * 63) / 88} onBf />
              </div>
            )
          ) : (
            <div style={{ fontSize: 12, color: 'var(--mu3)' }}>Hover a card to see it here</div>
          )}
        </div>
      );
    // A tab strip only offers what doesn't already have its own section.
    const tabOpts = (['game', 'log', 'players', 'rules'] as const).filter((k) => (k === 'game' ? !!rulesOn : !prefs.slots.includes(k)));
    const tab = kind === 'tabs' ? (tabOpts.includes(panelTab) ? panelTab : tabOpts[0] ?? 'log') : kind;
    return (
      <div className="col" style={{ flexGrow: 1, minHeight: 0 }}>
        {kind === 'tabs' && (
          <div className="row" style={{ height: 36, flexShrink: 0, borderBottom: '1px solid var(--line)', padding: '0 6px', gap: 2, alignItems: 'flex-end' }} role="tablist">
            {(
              [
                ['game', 'Game'],
                ['log', 'Log & chat'],
                ['players', 'Players'],
                ['rules', 'Rules text'],
              ] as const
            ).filter(([k]) => tabOpts.includes(k)).map(([k, l]) => (
              <button key={k} onClick={() => useTable.setState({ panelTab: k })} style={{ height: 32, border: 0, borderBottom: `2px solid ${tab === k ? 'var(--acc)' : 'transparent'}`, background: 'transparent', color: tab === k ? 'var(--tx)' : 'var(--mu2)', fontSize: 12, fontWeight: 600, padding: '0 12px', cursor: 'pointer' }}>
                {l}
                {k === 'game' && tab !== 'game' && rulesAttention && <span className="dot" style={{ background: 'var(--acc)', marginLeft: 6 }} aria-label="needs attention" />}
              </button>
            ))}
          </div>
        )}
        {tab === 'game' && <div ref={setRulesSlot} className="col" style={{ flexGrow: 1, minHeight: 0, overflowY: 'auto', padding: 10, gap: 10 }} />}
        {tab === 'log' && (
          <>
            <div className="row" style={{ gap: 4, padding: '8px 10px 4px' }}>
              <LogFilters value={logFilter} onChange={(f) => useTable.setState({ logFilter: f })} />
            </div>
            <LogList log={log} filter={logFilter} seats={v.seats} />
            <ChatInput onSend={sendChat} />
          </>
        )}
        {tab === 'players' && (
          <div className="lg col" style={{ flexGrow: 1, overflowY: 'auto', padding: '10px 12px', gap: 8 }}>
            <div style={{ fontSize: 11, color: 'var(--mu2)', textTransform: 'uppercase', letterSpacing: 0.6 }}>Turn order (clockwise)</div>
            {v.seats.map((p, si) => (
              <div key={si} className="row" style={{ gap: 10, padding: '9px 10px', borderRadius: 9, background: v.turn.active === si ? hexA(p.color, 0.1) : 'var(--surface2)', border: '1px solid var(--line)' }}>
                <span className="mono" style={{ fontSize: 11, color: 'var(--mu2)', width: 14 }}>
                  {si + 1}
                </span>
                <span className="dot" style={{ background: p.color }} />
                <div className="grow">
                  <div style={{ fontSize: 13, fontWeight: 600 }}>
                    {p.name}
                    {si === me ? ' (you)' : ''}
                  </div>
                  <div style={{ fontSize: 11, color: 'var(--mu)' }}>
                    Hand {p.zones.hand.length} · Library {p.zones.library.length} · Poison {p.counters.poison || 0}
                    {Object.keys(p.cmdTax).length ? ` · Tax +${Object.values(p.cmdTax).reduce((a, b) => a + b, 0)}` : ''}
                    {v.global.monarch === si ? ' · Monarch' : ''}
                    {v.global.initiative === si ? ' · Initiative' : ''}
                    {!p.connected ? ' · disconnected' : ''}
                    {p.conceded ? ' · conceded' : ''}
                  </div>
                </div>
                <span className="display" style={{ fontSize: 22 }}>
                  {p.life}
                </span>
              </div>
            ))}
          </div>
        )}
        {tab === 'rules' && (
          <div className="lg col" style={{ flexGrow: 1, padding: '14px 16px', gap: 8, overflowY: 'auto' }}>
            {def && c ? (
              <>
                <div className="display" style={{ fontSize: 19 }}>
                  {cardLabel(v, c)}
                </div>
                <div style={{ fontSize: 12, color: 'var(--mu)' }}>
                  {def.faces?.[c.face]?.typeLine || def.typeLine}
                  {def.power != null ? ` · ${def.faces?.[c.face]?.power ?? def.power}/${def.faces?.[c.face]?.toughness ?? def.toughness}` : ''}
                </div>
                <div style={{ fontSize: 13, lineHeight: 1.55, color: 'var(--tx2)', whiteSpace: 'pre-line' }}>{def.faces?.[c.face]?.oracleText || def.oracleText || '—'}</div>
                <div style={{ fontSize: 12, color: 'var(--acc-tx)', whiteSpace: 'pre-line' }}>
                  {c.effects.length ? 'Effects: ' + c.effects.map((e) => e.k).join(', ') + '\n' : ''}
                  {Object.entries(c.counters)
                    .filter(([, n]) => n)
                    .map(([k, n]) => `Counter: ${k} ×${n}`)
                    .join('\n')}
                </div>
                {!def.custom && <RulingsList oracleId={def.oracleId} link={false} />}
              </>
            ) : (
              <div style={{ fontSize: 13, color: 'var(--mu)' }}>Hover any card to read its rules text here.</div>
            )}
          </div>
        )}
        {void i}
      </div>
    );
  };

  const total = weights.reduce((a, b) => a + b, 0);
  return (
    <div className="col" onPointerDown={(e) => e.stopPropagation()} style={{ position: 'absolute', right: 0, top: TOPBAR, bottom: 0, width, background: 'var(--surface)', borderLeft: '1px solid var(--line3)', zIndex: 500 }}>
      <div
        onPointerDown={(e) => {
          e.preventDefault();
          setDrag(true);
        }}
        title="Drag to resize the side panel"
        role="separator"
        aria-orientation="vertical"
        aria-label="Resize side panel"
        style={{ position: 'absolute', left: -5, top: 0, width: 10, height: '100%', cursor: 'ew-resize', zIndex: 5, display: 'flex', alignItems: 'center', justifyContent: 'center' }}
      >
        <span style={{ width: 4, height: 44, borderRadius: 2, background: drag ? 'var(--acc)' : 'var(--line5)' }} />
      </div>
      {prefs.slots.map((kind, i) => (
        <div key={i} className="col" style={{ flexBasis: 0, flexGrow: weights[i] / total, minHeight: 80, borderBottom: i < prefs.slots.length - 1 ? '1px solid var(--line)' : 0, position: 'relative' }}>
          {section(kind, i)}
          {i < prefs.slots.length - 1 && (
            <div
              title="Drag to resize sections"
              onPointerDown={(e) => {
                e.preventDefault();
                const startY = e.clientY;
                const start = weights.slice();
                const H = (e.currentTarget.parentElement?.parentElement?.clientHeight || 800) / total;
                const move = (ev: PointerEvent) => {
                  const d = (ev.clientY - startY) / H;
                  const w = start.slice();
                  w[i] = Math.max(0.5, start[i] + d);
                  w[i + 1] = Math.max(0.5, start[i + 1] - d);
                  setWeights(w);
                };
                const up = () => {
                  window.removeEventListener('pointermove', move);
                  window.removeEventListener('pointerup', up);
                  setWeights((w) => {
                    try {
                      localStorage.setItem('aetherhub.panelWeights', JSON.stringify(w));
                    } catch {
                      /* ignore */
                    }
                    return w;
                  });
                };
                window.addEventListener('pointermove', move);
                window.addEventListener('pointerup', up);
              }}
              style={{ position: 'absolute', left: 0, right: 0, bottom: -4, height: 8, cursor: 'ns-resize', zIndex: 4 }}
            />
          )}
        </div>
      ))}
    </div>
  );
}

export type { PanZoom };
