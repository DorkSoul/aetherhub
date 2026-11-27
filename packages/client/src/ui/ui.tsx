import { forwardRef, useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode } from 'react';
import { create } from 'zustand';
import type { CardDef } from '@aetherhub/shared';
import { MANA_SYMBOL } from './manaSymbols';
import { faceSideways, FRAME_COLORS, frameKey, manaSymbols, meldResult, PIP_COLORS, turnKind } from '@aetherhub/shared';
import { CARD_BACK, imgUrl } from '../lib/base';
import { useApp } from '../lib/store';
import { RichLabel } from '../screens/rules/mana';

// ------------------------------------------------------------------ small pieces

/** A stroke chevron that sits centred in a flex button (text arrows like ← sit on the baseline and look off-centre). */
export function Chevron({ dir = 'left', size = 14, style }: { dir?: 'left' | 'right' | 'up' | 'down'; size?: number; style?: React.CSSProperties }) {
  const rot = { left: 0, up: 90, right: 180, down: 270 }[dir];
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" style={{ flexShrink: 0, display: 'block', transform: rot ? `rotate(${rot}deg)` : undefined, ...style }}>
      <path d="M15 5l-7 7 7 7" />
    </svg>
  );
}

export function Logo({ size = 26 }: { size?: number }) {
  return (
    <svg style={{ stroke: 'var(--acc)', flexShrink: 0 }} width={size} height={size} viewBox="0 0 24 24" fill="none" strokeWidth="1.6" aria-hidden="true">
      <path d="M12 2l9 10-9 10-9-10z" />
      <path d="M12 7l4.5 5-4.5 5-4.5-5z" />
    </svg>
  );
}

export function Switch({ on, onChange, label }: { on: boolean; onChange: (v: boolean) => void; label: string }) {
  return (
    <button className="sw" role="switch" aria-checked={on} aria-label={label} onClick={() => onChange(!on)}>
      <span />
    </button>
  );
}

export function Seg<T extends string | number>({ value, options, onChange, label }: { value: T; options: [T, string][]; onChange: (v: T) => void; label?: string }) {
  return (
    <div className="segs" role="group" aria-label={label}>
      {options.map(([v, l]) => (
        <button key={String(v)} className={'seg' + (v === value ? ' on' : '')} aria-pressed={v === value} onClick={() => onChange(v)}>
          {l}
        </button>
      ))}
    </div>
  );
}

/** A number with − / + buttons that keep stepping while held, and a value that can be clicked to type a number. */
export function NumberField({ value, min, max, step, set, format, label, width = 46, small }: { value: number; min: number; max?: number; /** the next value going up (dir 1) or down (dir -1) from v */ step: (v: number, dir: 1 | -1) => number; set: (v: number) => void; format?: (v: number) => string; label: string; width?: number; small?: boolean }) {
  const clamp = (v: number) => Math.min(max ?? Infinity, Math.max(min, v));
  const [typing, setTyping] = useState<string | null>(null);
  // while a button is held the value moves locally (the owner may only re-render later)
  const cur = useRef(value);
  cur.current = value;
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const stop = () => {
    if (timer.current) clearTimeout(timer.current);
    timer.current = null;
  };
  useEffect(() => stop, []);
  const bump = (dir: 1 | -1) => {
    const n = clamp(step(cur.current, dir));
    cur.current = n;
    set(n);
  };
  const hold = (dir: 1 | -1) => {
    stop();
    bump(dir);
    let k = 0;
    const again = () => {
      bump(dir);
      k++;
      // speeds up the longer it is held
      timer.current = setTimeout(again, k > 20 ? 25 : k > 8 ? 45 : 70);
    };
    timer.current = setTimeout(again, 400);
  };
  const commit = () => {
    if (typing === null) return;
    const n = Number(typing.replace(',', '.'));
    if (Number.isFinite(n)) set(clamp(n));
    setTyping(null);
  };
  const cls = small ? 'sq sm' : 'sq';
  const btn = (dir: 1 | -1) => (
    <button
      className={cls}
      aria-label={`${dir > 0 ? 'Increase' : 'Decrease'} ${label}`}
      onPointerDown={(e) => {
        if (e.button !== 0) return;
        e.preventDefault();
        hold(dir);
      }}
      onPointerUp={stop}
      onPointerLeave={stop}
      onPointerCancel={stop}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault();
          bump(dir);
        }
      }}
    >
      {dir > 0 ? '+' : '−'}
    </button>
  );
  return (
    <div className="row" style={{ gap: 6 }}>
      {btn(-1)}
      {typing !== null ? (
        <input
          className="inp mono"
          autoFocus
          aria-label={label}
          value={typing}
          onChange={(e) => setTyping(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => {
            e.stopPropagation();
            if (e.key === 'Enter') commit();
            else if (e.key === 'Escape') setTyping(null);
          }}
          style={{ width, height: small ? 24 : 28, padding: '0 4px', textAlign: 'center', fontSize: 12 }}
        />
      ) : (
        <button className="mono" title="Click to type a value" onClick={() => setTyping(String(value))} style={{ all: 'unset', cursor: 'text', minWidth: width, textAlign: 'center', fontSize: 12, borderBottom: '1px dashed var(--line5)' }}>
          {format ? format(value) : String(value)}
        </button>
      )}
      {btn(1)}
    </div>
  );
}

const COLOR_NAME: Record<string, string> = { W: 'White', U: 'Blue', B: 'Black', R: 'Red', G: 'Green', C: 'Colorless' };

/** A real mana symbol (W U B R G C) as artwork, the same symbols the rules game's mana pool uses. */
export function ManaSymbol({ c, size = 16, title, style }: { c: string; size?: number; title?: string | null; style?: React.CSSProperties }) {
  const art = MANA_SYMBOL[c];
  return (
    <span
      role={title === null ? undefined : 'img'}
      aria-label={title === null ? undefined : (title ?? COLOR_NAME[c] ?? c)}
      title={title === null ? undefined : (title ?? COLOR_NAME[c] ?? c)}
      style={{ display: 'inline-block', flexShrink: 0, width: size, height: size, borderRadius: '50%', background: art ? `${art} center / 100% 100% no-repeat` : PIP_COLORS[c]?.[0] || '#c9ccd1', boxShadow: '0 1px 1.5px rgba(0,0,0,.55)', ...style }}
    />
  );
}

export function Pips({ cost, size = 14 }: { cost: string; size?: number }) {
  const syms = manaSymbols(cost);
  if (!syms.length) return null;
  return (
    <span style={{ display: 'inline-flex', gap: 2 }}>
      {syms.map((s, i) => {
        if (MANA_SYMBOL[s]) return <ManaSymbol key={i} c={s} size={size} title={`{${s}}`} />;
        const single = PIP_COLORS[s];
        const hy = !single && s.split('/').find((x) => PIP_COLORS[x]);
        const [bg, fg] = single || (hy ? PIP_COLORS[hy] : ['#c9ccd1', '#1b1d22']);
        return (
          <span key={i} className="pip" style={{ width: size, height: size, borderRadius: size / 2, background: bg, color: fg, fontSize: size * 0.64 }} title={`{${s}}`}>
            {single ? '' : s.replace('/P', 'ᵖ').slice(0, 3)}
          </span>
        );
      })}
    </span>
  );
}

export function ColorDots({ colors, size = 16 }: { colors: string[]; size?: number }) {
  return (
    <span style={{ display: 'inline-flex', gap: 3 }}>
      {colors.map((c) => (
        <ManaSymbol key={c} c={c} size={size} />
      ))}
      {!colors.length && <ManaSymbol c="C" size={size} />}
    </span>
  );
}

export function Avatar({ name, color, size = 32 }: { name: string; color: string; size?: number }) {
  const n = name.replace(/^\(deleted\) /, '');
  return (
    <span style={{ width: size, height: size, borderRadius: size / 2, background: color, color: '#15130f', fontWeight: 700, fontSize: size * 0.42, display: 'inline-flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0 }}>
      {n[0]?.toUpperCase()}
    </span>
  );
}

export function Toasts() {
  const toasts = useApp((s) => s.toasts);
  return (
    <div className="toasts" role="status" aria-live="polite">
      {toasts.map((t) => (
        <div key={t.id} className="toast" style={{ borderColor: t.color || undefined }}>
          {t.text}
        </div>
      ))}
    </div>
  );
}

export function Modal({ title, onClose, children, width = 640, foot, head }: { title: ReactNode; onClose?: () => void; children: ReactNode; width?: number; foot?: ReactNode; head?: ReactNode }) {
  const scrim = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!onClose) return;
    // Only the topmost modal closes, so Escape in a modal opened from another leaves the one beneath open.
    const k = (e: KeyboardEvent) => {
      const all = document.querySelectorAll('.scrim');
      if (e.key === 'Escape' && all[all.length - 1] === scrim.current) onClose();
    };
    window.addEventListener('keydown', k);
    return () => window.removeEventListener('keydown', k);
  }, [onClose]);
  return (
    <div ref={scrim} className="scrim" onMouseDown={(e) => e.target === e.currentTarget && onClose?.()}>
      <div className="modal" role="dialog" aria-label={typeof title === 'string' ? title : undefined} style={{ width }}>
        <div className="modal-head">
          <h2>{title}</h2>
          {head}
          {onClose && (
            <button className="tb sm" onClick={onClose}>
              Close
            </button>
          )}
        </div>
        <div className="lg" style={{ overflowY: 'auto', minHeight: 0 }}>
          {children}
        </div>
        {foot && <div className="modal-foot">{foot}</div>}
      </div>
    </div>
  );
}

export function Confirm({ title, text, confirm, danger, onYes, onNo }: { title: string; text: ReactNode; confirm: string; danger?: boolean; onYes: () => void; onNo: () => void }) {
  return (
    <Modal
      title={title}
      onClose={onNo}
      width={460}
      foot={
        <>
          {/* Destructive confirmations focus Cancel, so pressing Enter never deletes by accident. */}
          <button className="tb" onClick={onNo} autoFocus={danger}>
            Cancel
          </button>
          <button className={'tb ' + (danger ? 'danger' : 'pri')} onClick={onYes} autoFocus={!danger}>
            {confirm}
          </button>
        </>
      }
    >
      <div style={{ padding: '16px 20px', fontSize: 13.5, color: 'var(--tx3)', lineHeight: 1.5 }}>{text}</div>
    </Modal>
  );
}

// ------------------------------------------------------------------ card rendering

/** Generated frame for cards without an image (custom cards) or while an image fails to load. Scales with its box. */
export function CardFrame({ def }: { def: CardDef }) {
  const f = FRAME_COLORS[frameKey(def.colors, def.typeLine)] || FRAME_COLORS.C;
  const pt = def.power != null ? `${def.power}/${def.toughness}` : def.loyalty ?? '';
  const t = def.typeLine;
  return (
    <svg viewBox="0 0 200 280" width="100%" height="100%" style={{ display: 'block' }} aria-label={def.name}>
      <rect width="200" height="280" rx="10" fill="#15130f" />
      <rect x="7" y="7" width="186" height="266" rx="6" fill={f.bg} />
      <foreignObject x="12" y="12" width="176" height="256">
        <div style={{ width: 176, height: 256, display: 'flex', flexDirection: 'column', gap: 4, fontFamily: 'var(--font)' }}>
          <div style={{ height: 22, flexShrink: 0, background: f.bar, borderRadius: 4, display: 'flex', alignItems: 'center', gap: 4, padding: '0 5px', boxShadow: 'inset 0 0 0 1px rgba(0,0,0,.25)' }}>
            <div style={{ flexGrow: 1, minWidth: 0, fontFamily: 'var(--display)', fontWeight: 700, fontSize: 12.5, color: '#1c1813', whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>{def.name}</div>
            <Pips cost={def.manaCost} size={13} />
          </div>
          <div
            style={{
              height: 108,
              flexShrink: 0,
              borderRadius: 3,
              backgroundColor: f.art,
              backgroundImage: 'repeating-linear-gradient(135deg, rgba(255,255,255,.05) 0 2px, transparent 2px 9px)',
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'center',
              color: 'rgba(255,255,255,.4)',
              fontSize: 40,
            }}
          >
            {/Creature/.test(t) ? '◉' : /Land/.test(t) ? '▲' : /Instant|Sorcery/.test(t) ? 'ϟ' : /Artifact/.test(t) ? '⬡' : '✦'}
          </div>
          <div style={{ height: 20, flexShrink: 0, background: f.bar, borderRadius: 4, display: 'flex', alignItems: 'center', padding: '0 5px', fontSize: 9.5, fontWeight: 600, color: '#1c1813', whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>
            {t}
          </div>
          <div style={{ flexGrow: 1, minHeight: 0, background: '#f3eee3', borderRadius: 3, padding: '5px 6px', fontSize: 9, lineHeight: 1.35, color: '#221d16', overflow: 'hidden', whiteSpace: 'pre-line' }}>{def.oracleText}</div>
          <div style={{ height: 12, flexShrink: 0, display: 'flex', alignItems: 'center', fontSize: 8, letterSpacing: 0.4, color: f.foot }}>
            {def.custom ? 'CUSTOM' : (def.set || '').toUpperCase()} · {(def.rarity || 'common').toUpperCase()}
            {pt && (
              <span style={{ marginLeft: 'auto', background: f.bar, color: '#1c1813', borderRadius: 4, padding: '0 6px', fontFamily: 'var(--display)', fontWeight: 700, fontSize: 12 }}>{pt}</span>
            )}
          </div>
        </div>
      </foreignObject>
    </svg>
  );
}

/** A card face as its own def, so the generated frame fallback shows the right name / cost / text. */
function faceDef(def: CardDef, face: number): CardDef {
  const f = def.faces?.[face];
  if (!f || face === 0) return def;
  return { ...def, name: f.name, manaCost: f.manaCost, typeLine: f.typeLine, oracleText: f.oracleText, power: f.power, toughness: f.toughness, loyalty: f.loyalty };
}

function FaceImg({ def, ref_, size, back, sleeve, alt }: { def?: CardDef | null; ref_?: string; size: 'normal' | 'large'; back?: boolean; sleeve?: string | null; alt?: string }) {
  const [failed, setFailed] = useState<string | null>(null);
  const src = back || !def ? (sleeve ? imgUrl(sleeve) : CARD_BACK) : imgUrl(ref_, size);
  if (!back && def && (!ref_ || failed === src)) return <CardFrame def={def} />;
  return <img src={src || CARD_BACK} alt={back || !def ? 'Card back' : alt || def.name} draggable={false} loading="lazy" onError={() => setFailed(src)} />;
}

/**
 * A card image. `face` 1 shows the other side: double-faced and meld cards turn over (3D), flip cards rotate 180°.
 * Changing `face` animates. `flipped` is the table's flip-card state and counts as face 1.
 */
export function CardImage({
  def,
  face = 0,
  back,
  sleeve,
  size = 'normal',
  style,
  className,
  flipped,
}: {
  def?: CardDef | null;
  face?: number;
  back?: boolean;
  sleeve?: string | null;
  size?: 'normal' | 'large';
  style?: CSSProperties;
  className?: string;
  flipped?: boolean;
}) {
  const kind = back || !def ? null : turnKind(def);
  const cls = 'cardimg ' + (className || '');
  if (!def || back || !kind) {
    return (
      <div className={cls} style={style}>
        <FaceImg def={def} ref_={def ? def.img[face] ?? def.img[0] : undefined} size={size} back={back} sleeve={sleeve} />
      </div>
    );
  }
  const turned = face > 0 || !!flipped;
  if (kind === 'flip') {
    return (
      <div className={cls} style={style}>
        <div className="turn" style={{ width: '100%', height: '100%', transform: turned ? 'rotate(180deg)' : 'none' }}>
          <FaceImg def={def} ref_={def.img[0]} size={size} />
        </div>
      </div>
    );
  }
  const meld = kind === 'meld' ? meldResult(def) : undefined;
  const backRef = meld ? `p/${meld.id}/0` : def.img[Math.max(1, face)] ?? def.img[1];
  const backDef = meld ? { ...def, name: meld.name, typeLine: meld.typeLine, manaCost: '', oracleText: '', power: undefined, toughness: undefined, img: [backRef] } : faceDef(def, Math.max(1, face));
  const side: CSSProperties = { position: 'absolute', inset: 0, backfaceVisibility: 'hidden', WebkitBackfaceVisibility: 'hidden', borderRadius: 'inherit', overflow: 'hidden' };
  return (
    <div className={cls} style={{ ...style, overflow: 'visible', background: 'transparent' }}>
      <div className="turn" style={{ position: 'relative', width: '100%', height: '100%', borderRadius: 'inherit', transformStyle: 'preserve-3d', transform: turned ? 'perspective(1400px) rotateY(180deg)' : 'perspective(1400px) rotateY(0deg)' }}>
        <div style={{ ...side, background: '#15130f' }}>
          <FaceImg def={def} ref_={def.img[0]} size={size} />
        </div>
        <div style={{ ...side, background: '#15130f', transform: 'rotateY(180deg)' }}>
          <FaceImg def={backDef} ref_={backRef} size={size} />
        </div>
      </div>
    </div>
  );
}

// ------------------------------------------------------------------ turning cards over

/** Local "show the other side" toggles, keyed by whatever identifies the card on screen (deck card id, table iid …). */
export const useTurned = create<{ m: Record<string, boolean> }>(() => ({ m: {} }));
export function toggleTurned(key: string) {
  const m = useTurned.getState().m;
  useTurned.setState({ m: { ...m, [key]: !m[key] } });
}

const TURN_LABEL = { dfc: 'Turn over', flip: 'Flip (rotate 180°)', meld: 'Show the melded card' } as const;

/** Round turn-over icon placed on a card. Swallows pointer events so it never starts a drag or selects the card. */
export function TurnButton({
  def,
  onTurn,
  size = 26,
  top = '40%',
  right = '5%',
  inline,
}: {
  def: CardDef | null | undefined;
  onTurn: () => void;
  size?: number;
  top?: string | number;
  right?: string | number;
  /** In a text row instead of on top of a card image. */
  inline?: boolean;
}) {
  const kind = turnKind(def);
  if (!kind) return null;
  const stop = (e: React.SyntheticEvent) => e.stopPropagation();
  return (
    <button
      className="turnbtn"
      title={TURN_LABEL[kind]}
      aria-label={TURN_LABEL[kind]}
      onPointerDown={stop}
      onMouseDown={stop}
      onDoubleClick={stop}
      onContextMenu={stop}
      onClick={(e) => {
        e.stopPropagation();
        onTurn();
      }}
      style={inline ? { width: size, height: size, flexShrink: 0 } : { position: 'absolute', right, top, width: size, height: size, zIndex: 5 }}
    >
      <svg viewBox="0 0 24 24" width="62%" height="62%" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round">
        {kind === 'flip' ? (
          <>
            <path d="M20 12a8 8 0 1 1-2.34-5.66" />
            <path d="M20 4v5h-5" />
          </>
        ) : (
          <>
            <path d="M4 10a8 8 0 0 1 14.5-3.5M20 14a8 8 0 0 1-14.5 3.5" />
            <path d="M19 3v4h-4M5 21v-4h4" />
          </>
        )}
      </svg>
    </button>
  );
}

/** CardImage with a turn-over icon whose state is local (deck editor, zone lists). The hover preview follows it. */
export function TurnableCard({ def, turnKey, style, size, className, iconSize }: { def?: CardDef | null; turnKey: string; style?: CSSProperties; size?: 'normal' | 'large'; className?: string; iconSize?: number }) {
  const turned = useTurned((s) => !!s.m[turnKey]);
  const w = typeof style?.width === 'number' ? style.width : 120;
  return (
    <div style={{ position: 'relative', width: style?.width, height: style?.height, flexShrink: 0 }}>
      <CardImage def={def} face={turned ? 1 : 0} size={size} className={className} style={{ ...style, width: '100%', height: '100%' }} />
      <TurnButton def={def} onTurn={() => toggleTurned(turnKey)} size={iconSize ?? Math.round(Math.max(20, Math.min(34, w * 0.17)))} />
    </div>
  );
}

// ------------------------------------------------------------------ hover preview

interface PreviewState {
  /** Set by the table: preview size and whether the popup is on (Detailed view shows cards in the side panel instead). */
  scale: number;
  enabled: boolean;
  def: CardDef | null;
  rect: DOMRect | null;
  extra: ReactNode;
  face: number;
  /** useTurned key of the hovered card; its local toggle turns the preview over too. */
  turnKey: string | null;
  /** Drawn over the card at the preview's size (the table's effect / counter / P/T chips). */
  overlay: ((width: number) => ReactNode) | null;
  frozen: boolean;
  /** The element being hovered (when the caller passed it): the preview hides once the pointer is over anything else,
   *  even when that element went away under the pointer and never got a mouseleave. */
  anchor: Element | null;
  /** `at`: the hovered element (preferred) or just its rectangle. */
  show: (def: CardDef | null | undefined, at: DOMRect | Element, extra?: ReactNode, face?: number, turnKey?: string | null, overlay?: ((width: number) => ReactNode) | null) => void;
  hide: (force?: boolean) => void;
  /** Updates the face when the previewed card was turned by a game action. */
  setFace: (key: string, face: number) => void;
}

export const usePreview = create<PreviewState>((set, get) => ({
  scale: 1,
  enabled: true,
  def: null,
  rect: null,
  extra: null,
  face: 0,
  turnKey: null,
  overlay: null,
  frozen: false,
  anchor: null,
  show: (def, at, extra, face = 0, turnKey = null, overlay = null) => {
    if (get().frozen) return;
    const anchor = at instanceof Element ? at : null;
    const rect = anchor ? anchor.getBoundingClientRect() : (at as DOMRect);
    set({ def: def ?? null, rect, anchor, extra: extra ?? null, face, turnKey, overlay });
  },
  hide: (force) => {
    if (get().frozen && !force) return;
    set({ def: null, rect: null, anchor: null, extra: null, frozen: false, turnKey: null, overlay: null });
  },
  setFace: (key, face) => {
    if (get().def?.key === key) set({ face });
  },
}));

/** Global hover preview placed beside the hovered element; flips away from screen edges; hold Shift to freeze, hold Alt to see
 *  the other side of a double-faced, flip or meld card. */
export function HoverPreview() {
  const { def, rect, extra, face: baseFace, turnKey, overlay, frozen, scale, enabled } = usePreview();
  const turned = useTurned((s) => (turnKey ? !!s.m[turnKey] : false));
  const [alt, setAlt] = useState(false);
  useEffect(() => {
    const down = (e: KeyboardEvent) => {
      if (e.key === 'Shift' && usePreview.getState().def) usePreview.setState({ frozen: true });
      if (e.key === 'Alt') {
        setAlt(true);
        // a bare Alt would otherwise move focus to the browser's menu bar (and the blur below hides the preview)
        if (usePreview.getState().def) e.preventDefault();
      }
    };
    const up = (e: KeyboardEvent) => {
      if (e.key === 'Shift') usePreview.setState({ frozen: false });
      if (e.key === 'Alt') {
        setAlt(false);
        if (usePreview.getState().def) e.preventDefault();
      }
    };
    // a missed mouseleave (the card was played, moved or re-drawn under the pointer) must not leave the preview up:
    // hide it as soon as the pointer is over something that isn't the hovered element (or outside its rectangle)
    const move = (e: PointerEvent) => {
      setAlt(e.altKey);
      const st = usePreview.getState();
      if (!st.def || st.frozen || e.shiftKey) return;
      const t = e.target;
      const over = st.anchor
        ? st.anchor.isConnected && t instanceof Node && st.anchor.contains(t)
        : !!st.rect && e.clientX >= st.rect.left - 4 && e.clientX <= st.rect.right + 4 && e.clientY >= st.rect.top - 4 && e.clientY <= st.rect.bottom + 4;
      if (!over) st.hide();
    };
    // Shift released in another window never sends keyup here: don't stay frozen
    const blur = () => {
      setAlt(false);
      usePreview.getState().hide(true);
    };
    window.addEventListener('keydown', down);
    window.addEventListener('keyup', up);
    window.addEventListener('pointermove', move, { passive: true });
    window.addEventListener('blur', blur);
    return () => {
      window.removeEventListener('keydown', down);
      window.removeEventListener('keyup', up);
      window.removeEventListener('pointermove', move);
      window.removeEventListener('blur', blur);
    };
  }, []);
  if (!enabled || !def || !rect) return null;
  const kind = turnKind(def);
  // Alt turns the preview over too (and back, when the card is already turned)
  const other = turned !== (alt && !!kind);
  const face = other ? (baseFace ? 0 : 1) : baseFace;
  // A meld result and a flipped card keep the front's orientation.
  const sideways = faceSideways(def, kind === 'dfc' ? face : 0);
  const W = Math.round(250 * scale);
  const H = Math.round(350 * scale);
  const boxW = sideways ? H : W;
  const boxH = sideways ? W : H;
  const vw = window.innerWidth;
  const vh = window.innerHeight;
  const x = rect.right + 14 + H < vw ? rect.right + 14 : Math.max(4, rect.left - 14 - boxW);
  const y = Math.max(8, Math.min(rect.top - 40, vh - Math.max(boxH, H) - 90));
  return (
    <div style={{ position: 'fixed', left: x, top: y, zIndex: 970, pointerEvents: 'none', display: 'flex', flexDirection: 'column', gap: 6, width: boxW }}>
      <div className="turn" style={{ width: boxW, height: boxH, position: 'relative' }}>
        <div className="turn" style={{ width: W, height: H, position: 'absolute', left: (boxW - W) / 2, top: (boxH - H) / 2, transform: sideways ? 'rotate(90deg)' : 'none' }}>
          <CardImage key={def.key} def={def} face={face} size="large" style={{ width: W, height: H, boxShadow: '0 18px 40px rgba(0,0,0,.65)', borderRadius: '4.75% / 3.4%' }} />
          {overlay?.(W)}
        </div>
      </div>
      {extra && <div style={{ background: 'var(--surface3)', border: '1px solid var(--line4)', borderRadius: 8, padding: '7px 9px', fontSize: 12, lineHeight: 1.45, color: 'var(--tx2)', whiteSpace: 'pre-line' }}>{extra}</div>}
      {frozen && <div style={{ fontSize: 11, color: 'var(--mu)' }}>Frozen — release Shift</div>}
    </div>
  );
}

/** Props to spread on anything that should show the hover preview. */
export function previewProps(def: CardDef | null | undefined, extra?: ReactNode, face = 0, turnKey?: string) {
  return {
    onMouseEnter: (e: React.MouseEvent) => usePreview.getState().show(def, e.currentTarget as HTMLElement, extra, face, turnKey ?? null),
    onMouseLeave: () => usePreview.getState().hide(),
  };
}

// ------------------------------------------------------------------ context menu

export type MenuItem =
  | { kind: 'head'; label: string }
  | { kind: 'sep' }
  | { kind: 'item'; label: string; hint?: string; danger?: boolean; run: () => void; keep?: boolean; disabled?: boolean; checked?: boolean; /** more choices for this item: hovering opens them beside it, clicking still runs the item */ sub?: () => MenuItem[] }
  | { kind: 'step'; label: string; value: ReactNode; dec: () => void; inc: () => void }
  | { kind: 'number'; label: string; value: number; min: number; max?: number; step: (v: number, dir: 1 | -1) => number; set: (v: number) => void; format?: (v: number) => string }
  | { kind: 'chips'; label: string; chips: { label: string; on?: boolean; run: () => void }[] }
  | { kind: 'input'; label: string; placeholder?: string; submit: (v: string, checked: boolean) => void; button?: string; checkbox?: string }
  | { kind: 'dots'; label: string; dots: { label: string; color: string; run: () => void }[] }
  | { kind: 'sub'; label: string; items: () => MenuItem[] };

interface MenuState {
  x: number;
  y: number;
  build: (() => MenuItem[]) | null;
  tick: number;
  open: (x: number, y: number, build: () => MenuItem[]) => void;
  close: () => void;
  refresh: () => void;
}

export const useMenu = create<MenuState>((set) => ({
  x: 0,
  y: 0,
  build: null,
  tick: 0,
  open: (x, y, build) => {
    usePreview.getState().hide(true);
    set({ x, y, build });
  },
  close: () => set({ build: null }),
  refresh: () => set((s) => ({ tick: s.tick + 1 })),
}));

function MenuInput({ it }: { it: Extract<MenuItem, { kind: 'input' }> }) {
  const [v, setV] = useState('');
  const [checked, setChecked] = useState(false);
  const go = () => {
    if (v.trim()) it.submit(v.trim(), checked);
    setV('');
  };
  return (
    <div style={{ padding: '2px 9px 8px' }}>
      <div style={{ fontSize: 11, color: 'var(--mu2)', marginBottom: 5 }}>{it.label}</div>
      <div className="row" style={{ gap: 5 }}>
        <input
          className="inp"
          style={{ height: 26, fontSize: 12, flexGrow: 1 }}
          placeholder={it.placeholder}
          value={v}
          onChange={(e) => setV(e.target.value)}
          onKeyDown={(e) => {
            e.stopPropagation();
            if (e.key === 'Enter') go();
          }}
        />
        <button className="chipb" style={{ height: 26, borderRadius: 6 }} onClick={go}>
          {it.button || 'Add'}
        </button>
      </div>
      {it.checkbox && (
        <label className="row" style={{ gap: 6, fontSize: 11.5, color: 'var(--mu)', marginTop: 6 }}>
          <input type="checkbox" checked={checked} onChange={() => setChecked(!checked)} /> {it.checkbox}
        </label>
      )}
    </div>
  );
}

function MenuList({ items, close, onSub, openSub }: { items: MenuItem[]; close: () => void; onSub: (label: string, el: HTMLElement) => void; openSub?: string }) {
  return (
    <>
      {items.map((it, i) => {
        switch (it.kind) {
          case 'head':
            return (
              <div key={i} className="mhead">
                <RichLabel text={it.label} />
              </div>
            );
          case 'sep':
            return <div key={i} className="msep" />;
          case 'item':
            return (
              <button
                key={i}
                className={'mi' + (it.danger ? ' danger' : '') + (it.sub && openSub === it.label ? ' open' : '')}
                role={it.checked === undefined ? 'menuitem' : 'menuitemcheckbox'}
                aria-checked={it.checked}
                aria-haspopup={it.sub ? 'menu' : undefined}
                aria-expanded={it.sub ? openSub === it.label : undefined}
                data-sub={it.sub ? it.label : undefined}
                disabled={it.disabled}
                style={it.disabled ? { opacity: 0.5 } : undefined}
                onClick={() => {
                  if (!it.keep) close();
                  it.run();
                }}
              >
                <span><RichLabel text={it.label} /></span>
                {it.hint && <span className="hint">{it.hint}</span>}
                {it.sub && <span className="hint">▸</span>}
                {it.checked !== undefined && (
                  <span className="sw mini" aria-checked={it.checked}>
                    <span />
                  </span>
                )}
              </button>
            );
          case 'sub':
            return (
              <button key={i} className={'mi' + (openSub === it.label ? ' open' : '')} role="menuitem" aria-haspopup="menu" aria-expanded={openSub === it.label} data-sub={it.label} onClick={(e) => onSub(it.label, e.currentTarget)}>
                <span><RichLabel text={it.label} /></span>
                <span className="hint">▸</span>
              </button>
            );
          case 'step':
            return (
              <div key={i} className="row" style={{ height: 30, padding: '0 9px', fontSize: 12.5, gap: 6 }}>
                <span className="grow">{it.label}</span>
                <button className="sq sm" onClick={it.dec} aria-label={`Decrease ${it.label}`}>
                  −
                </button>
                <span className="mono" style={{ minWidth: 26, textAlign: 'center', fontSize: 12 }}>
                  {it.value}
                </span>
                <button className="sq sm" onClick={it.inc} aria-label={`Increase ${it.label}`}>
                  +
                </button>
              </div>
            );
          case 'number':
            return (
              <div key={i} className="row" style={{ height: 32, padding: '0 9px', fontSize: 12.5, gap: 6 }}>
                <span className="grow">{it.label}</span>
                <NumberField small value={it.value} min={it.min} max={it.max} step={it.step} set={it.set} format={it.format} label={it.label} />
              </div>
            );
          case 'chips':
            return (
              <div key={i} style={{ padding: '4px 9px 6px' }}>
                <div style={{ fontSize: 11, color: 'var(--mu2)', marginBottom: 5 }}>{it.label}</div>
                <div style={{ display: 'flex', flexWrap: 'wrap', gap: 4 }}>
                  {it.chips.map((c, j) => (
                    <button key={j} className={'chipb' + (c.on ? ' on' : '')} aria-pressed={!!c.on} onClick={c.run}>
                      {c.label}
                    </button>
                  ))}
                </div>
              </div>
            );
          case 'dots':
            return (
              <div key={i} style={{ padding: '4px 9px 6px' }}>
                <div style={{ fontSize: 11, color: 'var(--mu2)', marginBottom: 5 }}>{it.label}</div>
                <div style={{ display: 'flex', flexWrap: 'wrap', gap: 4 }}>
                  {it.dots.map((d, j) => (
                    <button
                      key={j}
                      className="chipb"
                      onClick={() => {
                        close();
                        d.run();
                      }}
                    >
                      <span className="dot" style={{ background: d.color, marginRight: 5 }} />
                      {d.label}
                    </button>
                  ))}
                </div>
              </div>
            );
          case 'input':
            return <MenuInput key={i} it={it} />;
        }
      })}
    </>
  );
}

interface OpenSub {
  label: string;
  /** The parent item's rect: the submenu opens beside it. */
  anchor: DOMRect;
}

/** Cascading menu, desktop style: hovering an item with ▸ opens its submenu beside it and every level stays open. */
export function ContextMenu() {
  const { x, y, build, close } = useMenu();
  useMenu((s) => s.tick);
  const [subs, setSubs] = useState<OpenSub[]>([]);
  const subsRef = useRef(subs);
  subsRef.current = subs;
  const panels = useRef<(HTMLDivElement | null)[]>([]);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => {
    setSubs([]);
  }, [build]);
  useEffect(() => () => clearTimeout(timer.current), []);
  useEffect(() => {
    if (!build) return;
    const onDown = (e: MouseEvent) => {
      if (!panels.current.some((p) => p?.contains(e.target as Node))) close();
    };
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && close();
    const t = setTimeout(() => window.addEventListener('mousedown', onDown), 0);
    window.addEventListener('keydown', onKey);
    return () => {
      clearTimeout(t);
      window.removeEventListener('mousedown', onDown);
      window.removeEventListener('keydown', onKey);
    };
  }, [build, close]);
  if (!build) return null;

  const levels: { items: MenuItem[]; anchor: DOMRect | null }[] = [{ items: build(), anchor: null }];
  for (const o of subs) {
    const it = levels[levels.length - 1].items.find((i) => (i.kind === 'sub' || (i.kind === 'item' && !!i.sub)) && i.label === o.label);
    const more = it?.kind === 'sub' ? it.items() : it?.kind === 'item' && it.sub ? it.sub() : null;
    if (!more) break;
    levels.push({ items: more, anchor: o.anchor });
  }
  /** Set which submenu hangs off level `depth` (null = none). Opening is quick; closing waits, so a diagonal move into the submenu doesn't close it. */
  const want = (depth: number, label: string | null, el: HTMLElement | null, now = false) => {
    const cur = subsRef.current;
    clearTimeout(timer.current);
    if (label ? cur[depth]?.label === label : cur.length <= depth) return;
    const apply = () => setSubs((c) => (label && el ? [...c.slice(0, depth), { label, anchor: el.getBoundingClientRect() }] : c.slice(0, depth)));
    if (now) apply();
    else timer.current = setTimeout(apply, label ? 110 : 280);
  };
  return (
    <>
      {levels.map((l, d) => (
        <MenuPanel
          key={d + ':' + (subs[d - 1]?.label ?? '')}
          ref={(el) => {
            panels.current[d] = el;
          }}
          x={x}
          y={y}
          anchor={l.anchor}
          parent={d ? panels.current[d - 1] : null}
          onMouseOver={(e) => {
            const el = (e.target as HTMLElement).closest<HTMLElement>('[data-sub]');
            want(d, el?.dataset.sub ?? null, el);
          }}
        >
          <MenuList items={l.items} close={close} openSub={subs[d]?.label} onSub={(label, el) => want(d, label, el, true)} />
        </MenuPanel>
      ))}
    </>
  );
}

const MenuPanel = forwardRef<HTMLDivElement, { x: number; y: number; anchor: DOMRect | null; parent: HTMLDivElement | null; onMouseOver: (e: React.MouseEvent) => void; children: ReactNode }>(function MenuPanel({ x, y, anchor, parent, onMouseOver, children }, fwd) {
  const ref = useRef<HTMLDivElement | null>(null);
  const [pos, setPos] = useState<{ x: number; y: number } | null>(null);
  // Every render: the content can change size (chips toggled, tags added) while open.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    const W = window.innerWidth;
    const H = window.innerHeight;
    let nx = x;
    let ny = y;
    if (anchor && parent) {
      const p = parent.getBoundingClientRect();
      nx = p.right - 3;
      if (nx + r.width > W - 4) nx = p.left - r.width + 3;
      ny = anchor.top - 5;
    }
    nx = Math.max(4, Math.min(nx, W - r.width - 4));
    ny = Math.max(4, Math.min(ny, H - r.height - 4));
    if (!pos || pos.x !== nx || pos.y !== ny) setPos({ x: nx, y: ny });
  });
  return (
    <div
      ref={(el) => {
        ref.current = el;
        if (typeof fwd === 'function') fwd(el);
      }}
      className="menu lg"
      role="menu"
      style={{ left: pos?.x ?? x, top: pos?.y ?? y, visibility: pos ? 'visible' : 'hidden', maxHeight: window.innerHeight - 8 }}
      onContextMenu={(e) => e.preventDefault()}
      onMouseOver={onMouseOver}
    >
      {children}
    </div>
  );
});

export function openMenuAt(e: React.MouseEvent | MouseEvent, build: () => MenuItem[]) {
  e.preventDefault();
  e.stopPropagation();
  useMenu.getState().open(e.clientX + 2, e.clientY + 2, build);
}

export function openMenuBelow(el: HTMLElement, build: () => MenuItem[]) {
  const r = el.getBoundingClientRect();
  useMenu.getState().open(r.left, r.bottom + 6, build);
}

// ------------------------------------------------------------------ misc

/** Phone-sized screens: narrow (the `.hide-sm` / `.show-sm` breakpoint) or short (a phone held sideways). */
const COMPACT_QUERY = '(max-width: 760px), (max-height: 500px)';

export function useCompact(): boolean {
  const [m, setM] = useState(() => typeof matchMedia !== 'undefined' && matchMedia(COMPACT_QUERY).matches);
  useEffect(() => {
    const q = matchMedia(COMPACT_QUERY);
    const on = () => setM(q.matches);
    on();
    q.addEventListener('change', on);
    return () => q.removeEventListener('change', on);
  }, []);
  return m;
}

export function useDebounced<T>(v: T, ms: number): T {
  const [d, setD] = useState(v);
  useEffect(() => {
    const t = setTimeout(() => setD(v), ms);
    return () => clearTimeout(t);
  }, [v, ms]);
  return d;
}

export function timeAgo(t: number | null | undefined): string {
  if (!t) return 'never';
  const s = (Date.now() - t) / 1000;
  if (s < 60) return 'just now';
  if (s < 3600) return `${Math.floor(s / 60)} min ago`;
  if (s < 86400) return `${Math.floor(s / 3600)} hour${s < 7200 ? '' : 's'} ago`;
  const d = Math.floor(s / 86400);
  if (d === 1) return 'yesterday';
  if (d < 7) return `${d} days ago`;
  if (d < 30) return `${Math.floor(d / 7)} week${d < 14 ? '' : 's'} ago`;
  return `${Math.floor(d / 30)} month${d < 60 ? '' : 's'} ago`;
}

export function shortDate(t: number): string {
  return new Date(t).toLocaleDateString(undefined, { month: 'short', day: '2-digit' });
}
