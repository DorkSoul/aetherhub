// The Free view's camera: the table is one "world" of a fixed size (whatever the window or layout), shown through a
// pan/zoom transform that starts out fitting the whole table on screen. Mouse wheel zooms around the pointer; W A S D / arrow keys and right-drag pan.
// The camera can move until a corner of the table reaches the middle of the screen; what shows
// beyond the table is plain border.
import { useCallback, useEffect, useRef, useState } from 'react';

export interface Cam {
  x: number;
  y: number;
  k: number;
}

export const IDENTITY: Cam = { x: 0, y: 0, k: 1 };
export const MIN_ZOOM = 0.1;
export const MAX_ZOOM = 3;
const KEY_SPEED = 900; // screen px per second
const WASD: Record<string, [number, number]> = { w: [0, 1], a: [1, 0], s: [0, -1], d: [-1, 0] };
const ARROWS: Record<string, [number, number]> = { ArrowUp: [0, 1], ArrowLeft: [1, 0], ArrowDown: [0, -1], ArrowRight: [-1, 0] };

export interface World {
  w: number;
  h: number;
}

/** Keeps the middle of the screen over the table, so at most half the screen is border. */
function clamp(c: Cam, W: number, H: number, world: World): Cam {
  const k = Math.max(MIN_ZOOM, Math.min(MAX_ZOOM, c.k));
  return { k, x: Math.max(W / 2 - world.w * k, Math.min(W / 2, c.x)), y: Math.max(H / 2 - world.h * k, Math.min(H / 2, c.y)) };
}

/** The camera that shows the whole table, centred. */
export function fitCam(size: { w: number; h: number }, world: World): Cam {
  const k = Math.max(MIN_ZOOM, Math.min(MAX_ZOOM, Math.min(size.w / world.w, size.h / world.h)));
  return { k, x: (size.w - world.w * k) / 2, y: (size.h - world.h * k) / 2 };
}

function typing(t: EventTarget | null): boolean {
  const el = t as HTMLElement | null;
  return !!el && (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA' || el.tagName === 'SELECT' || el.isContentEditable);
}

/** True for a plain W/A/S/D press the camera owns (Shift+D etc. still reach the game hotkeys). */
export function isPanKey(e: KeyboardEvent): boolean {
  if (e.key in ARROWS) return !e.ctrlKey && !e.metaKey && !e.altKey;
  return e.key.toLowerCase() in WASD && !e.shiftKey && !e.ctrlKey && !e.metaKey && !e.altKey;
}

/** `el` is the play-area element (state, not a ref, so listeners attach once it exists). */
export function useCamera(enabled: boolean, el: HTMLElement | null, size: { w: number; h: number }, world: World) {
  const [cam, setCamState] = useState<Cam>(IDENTITY);
  const camRef = useRef<Cam>(IDENTITY);
  const sizeRef = useRef(size);
  sizeRef.current = size;
  const worldRef = useRef(world);
  worldRef.current = world;
  const [panning, setPanning] = useState(false);
  /** false until the player pans or zooms; until then the camera keeps fitting the table as the window changes */
  const touched = useRef(false);

  const setCam = useCallback((c: Cam | ((c: Cam) => Cam), auto = false) => {
    if (!auto) touched.current = true;
    const next = clamp(typeof c === 'function' ? c(camRef.current) : c, sizeRef.current.w, sizeRef.current.h, worldRef.current);
    camRef.current = next;
    setCamState(next);
  }, []);

  /** Zooms by a factor around a point in viewport coordinates (default: the middle of the screen). */
  const zoomBy = useCallback(
    (f: number, px = sizeRef.current.w / 2, py = sizeRef.current.h / 2) =>
      setCam((c) => {
        const k = Math.max(MIN_ZOOM, Math.min(MAX_ZOOM, c.k * f));
        return { k, x: px - ((px - c.x) * k) / c.k, y: py - ((py - c.y) * k) / c.k };
      }),
    [setCam],
  );
  const reset = useCallback(() => {
    touched.current = false;
    setCam(fitCam(sizeRef.current, worldRef.current), true);
  }, [setCam]);

  // Leaving the Free view puts the table back where it belongs; resizing keeps the camera legal.
  useEffect(() => {
    touched.current = false;
    setCam(enabled ? fitCam(sizeRef.current, worldRef.current) : IDENTITY, true);
  }, [enabled, world.w, world.h, setCam]);
  useEffect(() => {
    setCam(!touched.current && enabled ? fitCam(size, world) : (c) => c, true);
  }, [size.w, size.h, setCam]);

  // Mouse wheel zooms around the pointer.
  useEffect(() => {
    if (!enabled || !el) return;
    const wheel = (e: WheelEvent) => {
      // a scrollable panel under the pointer (a seat's counters and mana pool) scrolls instead of zooming
      const sc = (e.target as HTMLElement | null)?.closest?.('[data-left-scroll]') as HTMLElement | null;
      if (sc && sc.scrollHeight > sc.clientHeight) return;
      e.preventDefault();
      const r = el.getBoundingClientRect();
      const f = Math.pow(1.0015, -Math.max(-120, Math.min(120, e.deltaY * (e.deltaMode === 1 ? 33 : 1))));
      zoomBy(f, e.clientX - r.left, e.clientY - r.top);
    };
    el.addEventListener('wheel', wheel, { passive: false });
    return () => el.removeEventListener('wheel', wheel);
  }, [enabled, el, zoomBy]);

  // W A S D / arrow keys pan smoothly while held.
  useEffect(() => {
    if (!enabled) return;
    const held = new Map<string, [number, number]>();
    let raf = 0;
    let last = 0;
    const tick = (t: number) => {
      const dt = Math.min(0.05, (t - last) / 1000);
      last = t;
      let dx = 0;
      let dy = 0;
      for (const [x, y] of held.values()) {
        dx += x;
        dy += y;
      }
      if (dx || dy) setCam((c) => ({ ...c, x: c.x + dx * KEY_SPEED * dt, y: c.y + dy * KEY_SPEED * dt }));
      raf = held.size ? requestAnimationFrame(tick) : 0;
    };
    const down = (e: KeyboardEvent) => {
      if (typing(e.target) || !isPanKey(e)) return;
      e.preventDefault();
      const dir = ARROWS[e.key] ?? WASD[e.key.toLowerCase()];
      held.set(e.key.toLowerCase(), dir);
      if (!raf) {
        last = performance.now();
        raf = requestAnimationFrame(tick);
      }
    };
    const up = (e: KeyboardEvent) => {
      held.delete(e.key.toLowerCase());
    };
    const blur = () => held.clear();
    window.addEventListener('keydown', down);
    window.addEventListener('keyup', up);
    window.addEventListener('blur', blur);
    return () => {
      window.removeEventListener('keydown', down);
      window.removeEventListener('keyup', up);
      window.removeEventListener('blur', blur);
      cancelAnimationFrame(raf);
    };
  }, [enabled, setCam]);

  // Right-drag pans. A right-click that doesn't move still opens the usual menu: on systems that fire
  // the context menu on press, it is held back and replayed on release.
  useEffect(() => {
    if (!enabled || !el) return;
    let drag: { sx: number; sy: number; ox: number; oy: number; moved: boolean; menu: { target: EventTarget; x: number; y: number } | null } | null = null;
    let suppressUntil = 0;
    let replaying = false;
    const onDown = (e: PointerEvent) => {
      if (e.button !== 2 || (e.target as HTMLElement).closest?.('[data-nopan]')) return;
      drag = { sx: e.clientX, sy: e.clientY, ox: camRef.current.x, oy: camRef.current.y, moved: false, menu: null };
    };
    const onMove = (e: PointerEvent) => {
      if (!drag) return;
      if (!drag.moved && Math.hypot(e.clientX - drag.sx, e.clientY - drag.sy) < 6) return;
      if (!drag.moved) setPanning(true);
      drag.moved = true;
      setCam((c) => ({ ...c, x: drag!.ox + e.clientX - drag!.sx, y: drag!.oy + e.clientY - drag!.sy }));
    };
    const onUp = (e: PointerEvent) => {
      if (!drag || e.button !== 2) return;
      const d = drag;
      drag = null;
      setPanning(false);
      if (d.moved) suppressUntil = Date.now() + 400;
      else if (d.menu) {
        replaying = true;
        d.menu.target.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: d.menu.x, clientY: d.menu.y, button: 2, view: window }));
        replaying = false;
      }
    };
    const onMenu = (e: MouseEvent) => {
      if (replaying) return;
      if (drag) {
        // Pressed but not released yet: wait to see whether this is a click or a pan.
        e.preventDefault();
        e.stopPropagation();
        drag.menu = { target: e.target!, x: e.clientX, y: e.clientY };
      } else if (Date.now() < suppressUntil) {
        e.preventDefault();
        e.stopPropagation();
      }
    };
    el.addEventListener('pointerdown', onDown, true);
    el.addEventListener('contextmenu', onMenu, true);
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
    return () => {
      el.removeEventListener('pointerdown', onDown, true);
      el.removeEventListener('contextmenu', onMenu, true);
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
    };
  }, [enabled, el, setCam]);

  return { cam, camRef, setCam, zoomBy, reset, panning };
}
