import { memo, useEffect, useRef, useState, type CSSProperties } from 'react';
import type { CardDef, CardView, GameView, LogView, Rect, Seat } from '@aetherhub/shared';
import { basePT, DUNGEONS, FRAME_COLORS, frameKey, hexA, mixHex, PHASES } from '@aetherhub/shared';
import { CARD_BACK, imgUrl } from '../../lib/base';
import { CardImage, TurnButton } from '../../ui/ui';
import { useTable } from './store';

// ------------------------------------------------------------------ card

export interface TCardProps {
  c: CardView;
  def?: CardDef;
  rect: Rect;
  rot: number;
  showFace: boolean;
  sleeve: string | null;
  z: number;
  selected: boolean;
  dim: boolean;
  pinged: boolean;
  hiddenForDrag: boolean;
  foreignColor: string | null;
  peek: boolean;
  onBf: boolean;
  transition: boolean;
  mulliganPick?: boolean;
  /** Local "show the other side" toggle (hand cards, meld pieces, cards you can't act on). */
  turned?: boolean;
  onTurn?: () => void;
}

export function ptOf(c: CardView, def: CardDef | undefined): { txt: string; delta: number } | null {
  const b = basePT(def, c.face);
  if (!b) return null;
  const counters = (c.counters['+1/+1'] || 0) - (c.counters['-1/-1'] || 0);
  // rules games: the engine's power/toughness (the def's) already includes counters, so don't add them again
  const net = !!def?.key.startsWith('f:');
  const d = net ? 0 : counters;
  const p = b[0] + d + c.pt.p + c.ptEot.p;
  const t = b[1] + d + c.pt.t + c.ptEot.t;
  const delta = p - b[0] + (t - b[1]);
  if (!delta) return null;
  return { txt: `${p}/${t}`, delta };
}

/** Effect / counter / P/T / pile overlays in a 90×126 box scaled to the card. */
export function Overlays({ c, def, w, onBf }: { c: CardView; def?: CardDef; w: number; onBf: boolean }) {
  const effs = c.effects.map((e) => e.k + (e.eot ? ' · EOT' : ''));
  if (c.noUntap) effs.push("Doesn't untap");
  if (c.token && def && !def.isToken) effs.push('Copy');
  const ctrs = Object.entries(c.counters)
    .filter(([, v]) => v)
    .map(([k, v]) => (k === '+1/+1' ? `+1/+1 ×${v}` : k === '-1/-1' ? `−1/−1 ×${v}` : `${k} ${v}`));
  const pt = onBf && !c.faceDown ? ptOf(c, def) : null;
  const chip: CSSProperties = { fontSize: 6, lineHeight: '7px', padding: '0 2.5px', borderRadius: 2.3, background: 'rgba(28,30,35,.92)', color: 'var(--acc-tx)', border: '1px solid rgba(var(--acc-rgb),.5)' };
  return (
    <div style={{ position: 'absolute', left: 0, top: 0, width: 90, height: 126, transform: `scale(${w / 90})`, transformOrigin: '0 0', pointerEvents: 'none' }}>
      {effs.length > 0 && (
        <div style={{ position: 'absolute', left: 2, right: 2, top: 2, display: 'flex', flexWrap: 'wrap', gap: 2 }}>
          {effs.map((e, i) => (
            <div key={i} style={chip}>
              {e}
            </div>
          ))}
        </div>
      )}
      {ctrs.length > 0 && (
        <div style={{ position: 'absolute', left: 2, bottom: 2, display: 'flex', flexDirection: 'column', alignItems: 'flex-start', gap: 2 }}>
          {ctrs.map((k, i) => (
            <div key={i} style={{ ...chip, fontWeight: 600 }}>
              {k}
            </div>
          ))}
        </div>
      )}
      {pt && (
        <div style={{ ...chip, position: 'absolute', right: 4.5, bottom: 4, minWidth: 15.5, height: 9, display: 'flex', alignItems: 'center', justifyContent: 'center', fontFamily: 'var(--display)', fontWeight: 700, color: pt.delta > 0 ? '#8fdca6' : '#f5a397', borderColor: pt.delta > 0 ? 'rgba(143,220,166,.6)' : 'rgba(245,163,151,.6)' }}>
          {pt.txt}
        </div>
      )}
      {c.pile > 1 && <div style={{ position: 'absolute', right: 3, top: 3, fontSize: 11, fontWeight: 700, padding: '2px 6px', borderRadius: 9, background: 'var(--acc)', color: 'var(--acc-ink)', boxShadow: '0 1px 3px rgba(0,0,0,.6)' }}>×{c.pile}</div>}
      {onBf && c.faceDown && <div style={{ position: 'absolute', right: 3, bottom: 3, fontSize: 10, fontWeight: 600, background: '#15130f', color: '#ebe8e2', borderRadius: 4, padding: '1px 4px' }}>2/2</div>}
    </div>
  );
}

/** Overlays for a card by id, kept current from the table state (used on the hover preview). */
export function LiveOverlays({ iid, w, onBf }: { iid: string; w: number; onBf: boolean }) {
  const c = useTable((s) => s.view?.cards[iid]);
  const def = useTable((s) => (c ? s.view?.defs[c.def] : undefined));
  if (!c) return null;
  return <Overlays c={c} def={def} w={w} onBf={onBf} />;
}

/** How far each card of a pile peeks out to the right of the one above it (a fraction of the card's width). */
const PILE_FAN = 0.18;

export const TCard = memo(function TCard(p: TCardProps & { onDown: (e: React.PointerEvent) => void; onCtx: (e: React.MouseEvent) => void; onDbl: () => void; onEnter: (e: React.MouseEvent) => void; onLeave: (e: React.MouseEvent) => void }) {
  const { c, def, rect, rot } = p;
  const rad = Math.max(3, Math.round(rect.w * 0.05));
  // a face-down card you may look at, turned up on this screen only (the card menu's "Look at the face-down card")
  const look = p.peek && !!p.turned && !!def;
  // a permanent turned over (or turned up) on this screen only is outlined, so it isn't taken for transformed
  const localLook = look || (p.onBf && !!p.turned && p.showFace);
  return (
    <div
      className={p.pinged ? 'tping' : undefined}
      data-iid={c.iid}
      onPointerDown={p.onDown}
      onContextMenu={p.onCtx}
      onDoubleClick={p.onDbl}
      onMouseEnter={p.onEnter}
      onMouseLeave={p.onLeave}
      style={{
        position: 'absolute',
        left: rect.x,
        top: rect.y,
        width: rect.w,
        height: rect.h,
        transform: rot ? `rotate(${rot}deg)` : undefined,
        transition: p.transition ? 'transform .16s ease, left .12s, top .12s' : undefined,
        zIndex: p.z,
        borderRadius: rad,
        boxShadow: p.mulliganPick ? '0 0 0 3px #e0a84f, 0 0 14px rgba(224,168,79,.5)' : p.selected ? '0 0 0 2px var(--acc), 0 0 14px rgba(var(--acc-rgb),.5)' : '0 2px 6px rgba(0,0,0,.45)',
        opacity: p.hiddenForDrag ? 0 : p.dim ? 0.55 : 1,
        cursor: 'grab',
        touchAction: 'none',
      }}
    >
      {/* a pile of identical cards fans out sideways: up to four copies peek out to the right, behind the top card */}
      {p.onBf &&
        c.pile > 1 &&
        Array.from({ length: Math.min(c.pile - 1, 4) }, (_, i) => Math.min(c.pile - 1, 4) - i).map((k) => (
          <div key={k} aria-hidden style={{ position: 'absolute', left: k * rect.w * PILE_FAN, top: 0, width: '100%', height: '100%', zIndex: -1, borderRadius: rad, boxShadow: '0 2px 6px rgba(0,0,0,.45)', overflow: 'hidden', pointerEvents: 'none' }}>
            {p.showFace && def ? (
              <CardImage def={def} face={c.face} style={{ width: '100%', height: '100%', borderRadius: rad }} />
            ) : (
              <img src={p.sleeve ? imgUrl(p.sleeve) || CARD_BACK : CARD_BACK} alt="" draggable={false} style={{ width: '100%', height: '100%', display: 'block', objectFit: 'cover', borderRadius: rad }} />
            )}
          </div>
        ))}
      {(p.showFace || look) && def ? (
        // A flipped card on the battlefield is rotated by the wrapper (together with tapping), not here.
        <CardImage def={def} face={p.turned && !look ? (c.face ? 0 : 1) : c.face} style={{ width: '100%', height: '100%', borderRadius: rad, ...(localLook ? { outline: '2px dashed rgba(var(--acc-rgb),.85)', outlineOffset: -2 } : {}) }} />
      ) : (
        <img src={p.sleeve ? imgUrl(p.sleeve) || CARD_BACK : CARD_BACK} alt="" draggable={false} style={{ width: '100%', height: '100%', display: 'block', objectFit: 'cover', borderRadius: rad }} />
      )}
      <Overlays c={c} def={def} w={rect.w} onBf={p.onBf} />
      {p.foreignColor && <div title="Owned by another player" style={{ position: 'absolute', left: 4, top: 4, width: 10, height: 10, borderRadius: 5, background: p.foreignColor, boxShadow: '0 0 0 2px #121317' }} />}
      {p.showFace && def && p.onTurn && rect.w >= 40 && <TurnButton def={def} onTurn={p.onTurn} size={Math.round(Math.max(14, Math.min(26, rect.w * 0.2)))} />}
      {p.peek && !look && rect.w > 60 && <div style={{ position: 'absolute', left: 3, top: 3, fontSize: 9, background: 'rgba(21,19,15,.85)', color: 'var(--acc-tx)', borderRadius: 4, padding: '1px 4px' }}>hover to peek</div>}
    </div>
  );
});

// ------------------------------------------------------------------ life box

export function LifeBox({ seat, life, hist, onDelta, onSet, wheel = true }: { seat: Seat; life: number; hist: number[]; onDelta: (d: number) => void; onSet: (v: number) => void; wheel?: boolean }) {
  const [edit, setEdit] = useState<string | null>(null);
  const hold = useRef<{ t?: number; i?: number }>({});
  const stop = () => {
    clearTimeout(hold.current.t);
    clearInterval(hold.current.i);
    hold.current = {};
  };
  useEffect(() => stop, []);
  const start = (d: number) => {
    stop();
    onDelta(d);
    hold.current.t = window.setTimeout(() => {
      hold.current.i = window.setInterval(() => onDelta(d), 90);
    }, 420);
  };
  void seat;
  return (
    <div
      data-life
      className="lifebox"
      title={`Left-click +1 · right-click −1 · hold to repeat · Shift ±5 · scroll · Ctrl-click to type a number\nHistory: ${hist.slice(-10).join(' → ')}`}
      onPointerDown={(e) => {
        e.stopPropagation();
        if (edit != null) return;
        if (e.button === 0 && (e.ctrlKey || e.metaKey)) {
          e.preventDefault();
          setEdit(String(life));
          return;
        }
        if (e.button !== 0 && e.button !== 2) return;
        start((e.button === 0 ? 1 : -1) * (e.shiftKey ? 5 : 1));
      }}
      onPointerUp={stop}
      onPointerLeave={stop}
      onContextMenu={(e) => e.preventDefault()}
      // In the Free view the wheel zooms the camera, so it never changes life by accident.
      onWheel={wheel ? (e) => onDelta(e.deltaY < 0 ? 1 : -1) : undefined}
      data-nopan
      style={{ position: 'absolute', left: 8, top: 36, width: 88, height: 58, borderRadius: 9, background: 'var(--surface2)', border: '1px solid var(--line3)', display: 'flex', alignItems: 'center', justifyContent: 'center', cursor: 'pointer', userSelect: 'none' }}
    >
      {edit == null ? (
        <span className="display" style={{ fontSize: 30, color: life <= 10 ? '#f08a7a' : 'var(--tx)', pointerEvents: 'none' }}>
          {life}
        </span>
      ) : (
        <input
          aria-label="Exact life total"
          autoFocus
          value={edit}
          onChange={(e) => setEdit(e.target.value)}
          onPointerDown={(e) => e.stopPropagation()}
          onKeyDown={(e) => {
            e.stopPropagation();
            if (e.key === 'Enter') (e.target as HTMLInputElement).blur();
            if (e.key === 'Escape') setEdit(null);
          }}
          onBlur={() => {
            const n = parseInt(edit, 10);
            if (!isNaN(n) && n !== life) onSet(n);
            setEdit(null);
          }}
          style={{ width: 60, height: 34, borderRadius: 6, border: '1px solid var(--acc)', background: 'var(--area)', color: 'var(--tx)', fontSize: 20, textAlign: 'center' }}
        />
      )}
    </div>
  );
}

// ------------------------------------------------------------------ piles

/** `touchAction: 'pan-y'` lets a finger scroll the column the pile sits in (a drag that starts vertically scrolls instead). */
export function Pile({ r, label, count, thumb, back, hot, title, onDown, onClick, onCtx, onEnter, onLeave, touchAction = 'none' }: { r: Rect; label: string; count: string; thumb: string | null; back: boolean; hot: boolean; title: string; onDown: (e: React.PointerEvent) => void; onClick: (e: React.MouseEvent) => void; onCtx: (e: React.MouseEvent) => void; onEnter?: (e: React.MouseEvent) => void; onLeave?: (e: React.MouseEvent) => void; touchAction?: 'none' | 'pan-y' }) {
  return (
    <div
      className="pile"
      data-pile={label.toLowerCase()}
      title={title}
      onPointerDown={onDown}
      onClick={onClick}
      onContextMenu={onCtx}
      onMouseEnter={onEnter}
      onMouseLeave={onLeave}
      style={{ position: 'absolute', left: r.x, top: r.y, width: r.w, height: r.h, borderRadius: 7, background: 'var(--surface2)', border: `1px solid ${hot ? 'var(--acc)' : 'var(--line)'}`, display: 'flex', alignItems: 'center', gap: 6, padding: '0 6px', zIndex: 6, overflow: 'hidden', cursor: 'pointer', touchAction }}
    >
      <div style={{ width: 15, height: 21, flexShrink: 0, borderRadius: 2, background: back ? `url("${thumb || CARD_BACK}") center/cover` : thumb ? `url("${thumb}") center 20%/cover` : '#26282e', boxShadow: '0 0 0 1px rgba(0,0,0,.5)' }} />
      <div className="col grow" style={{ lineHeight: 1.15, minWidth: 0 }}>
        <span style={{ fontSize: 10, color: 'var(--mu2)', whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>{label}</span>
        <span className="mono" style={{ fontSize: 12.5, fontWeight: 600, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>
          {count}
        </span>
      </div>
    </div>
  );
}

// ------------------------------------------------------------------ top bar pieces

/** Compact: each phase is its first letter (the strip reads U U D M C M E in turn order; the full name is the label). */
export function PhaseStrip({ phase, onPick, target = null, compact = false }: { phase: number; onPick: (i: number) => void; target?: number | null; compact?: boolean }) {
  return (
    <div className="row" style={{ gap: 2, background: 'var(--surface2)', border: '1px solid var(--line)', borderRadius: 8, padding: 2, flexShrink: 0 }} role="group" aria-label="Phase">
      {PHASES.map((ph, i) => (
        <button
          key={ph}
          onClick={() => onPick(i)}
          title={compact ? ph : undefined}
          aria-label={ph}
          style={{ height: compact ? 26 : 24, minWidth: compact ? 24 : undefined, padding: compact ? 0 : '0 7px', border: 0, outline: target === i ? '1px dashed var(--acc)' : undefined, borderRadius: 6, background: phase === i ? 'var(--acc)' : 'transparent', color: phase === i ? 'var(--acc-ink)' : 'var(--mu)', fontWeight: phase === i ? 600 : 400, fontSize: compact ? 12 : 11, cursor: 'pointer', whiteSpace: 'nowrap' }}
        >
          {compact ? ph[0] : ph}
        </button>
      ))}
    </div>
  );
}

// ------------------------------------------------------------------ log

export function LogList({ log, filter, seats }: { log: LogView[]; filter: 'all' | 'act' | 'chat'; seats: Seat[] }) {
  const end = useRef<HTMLDivElement>(null);
  const list = log.filter((l) => filter === 'all' || (filter === 'chat' ? l.kind === 'chat' : l.kind !== 'chat')).slice(-300);
  useEffect(() => {
    end.current?.scrollIntoView({ block: 'end' });
  }, [list.length]);
  return (
    <div className="lg" style={{ flexGrow: 1, overflowY: 'auto', padding: '4px 12px', minHeight: 0 }}>
      {list.map((l) => {
        const col = l.actor != null ? seats[l.actor]?.color : l.kind === 'chat' ? 'var(--mu)' : 'var(--mu2)';
        const d = new Date(l.at);
        return (
          <div key={l.id} className="row" style={{ gap: 7, fontSize: 12, lineHeight: 1.45, padding: '3px 0', alignItems: 'flex-start' }}>
            <span className="mono" style={{ color: 'var(--mu4)', fontSize: 10, paddingTop: 2, flexShrink: 0 }}>
              {String(d.getHours()).padStart(2, '0')}:{String(d.getMinutes()).padStart(2, '0')}
            </span>
            <div style={{ minWidth: 0 }}>
              <span style={{ color: col, fontWeight: 600 }}>
                {l.actorName}
                {l.kind === 'chat' ? ': ' : ' '}
              </span>
              <span style={{ color: l.kind === 'chat' ? 'var(--tx)' : l.kind === 'system' ? 'var(--mu2)' : 'var(--tx3)' }}>{l.text}</span>
              {l.priv && (
                <span title="Other players see this line without card names" style={{ fontSize: 9.5, color: 'var(--mu)', border: '1px solid var(--line5)', borderRadius: 4, padding: '0 4px', marginLeft: 5 }}>
                  only you
                </span>
              )}
            </div>
          </div>
        );
      })}
      <div ref={end} />
    </div>
  );
}

export function ChatInput({ onSend, placeholder = 'Message the table… (Enter)' }: { onSend: (t: string) => void; placeholder?: string }) {
  const [v, setV] = useState('');
  return (
    <div style={{ padding: '8px 10px', borderTop: '1px solid var(--line)' }}>
      <input
        className="inp"
        aria-label="Chat message"
        placeholder={placeholder}
        value={v}
        onChange={(e) => setV(e.target.value)}
        onKeyDown={(e) => {
          e.stopPropagation();
          if (e.key === 'Enter' && v.trim()) {
            onSend(v.trim());
            setV('');
          }
        }}
        style={{ width: '100%', height: 32, fontSize: 12.5 }}
      />
    </div>
  );
}

export function LogFilters({ value, onChange }: { value: 'all' | 'act' | 'chat'; onChange: (v: 'all' | 'act' | 'chat') => void }) {
  return (
    <>
      {(
        [
          ['all', 'All'],
          ['act', 'Actions'],
          ['chat', 'Chat'],
        ] as const
      ).map(([k, l]) => (
        <button key={k} className={'mbtn' + (value === k ? ' on' : '')} style={{ height: 24, fontSize: 11 }} onClick={() => onChange(k)}>
          {l}
        </button>
      ))}
    </>
  );
}

// ------------------------------------------------------------------ helpers

/** Fine grain for the baize table theme (an inline SVG noise tile). */
const FELT_GRAIN = `url("data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' width='180' height='180'><filter id='n'><feTurbulence type='fractalNoise' baseFrequency='0.95' numOctaves='2' stitchTiles='stitch'/><feColorMatrix values='0 0 0 0 1  0 0 0 0 1  0 0 0 0 1  0 0 0 0.11 0'/></filter><rect width='180' height='180' filter='url(%23n)'/></svg>")`;

/** How one seat's surfaces are painted: the info column, the hand strip and the battlefield.
 *  `theme` is the player's table theme: `classic` washes each battlefield in its player's colour, `felt` is green baize
 *  for everyone, `playmats` gives every player a mat of their own colour. */
export function seatTint(seat: Seat, tint: boolean, light = false, theme: 'classic' | 'felt' | 'playmats' = 'classic'): { info: string; hand: string; mat: CSSProperties } {
  const c = seat.color;
  // the info column: the player's colour at the top, fading out two thirds of the way down
  const infoBase = light ? '#ffffff' : '#0b101a';
  const info = tint ? `linear-gradient(180deg, ${mixHex(infoBase, c, light ? 0.22 : 0.26)} 0%, ${light ? mixHex(infoBase, c, 0.06) : infoBase} 62%)` : 'var(--surface)';
  if (theme === 'felt')
    return {
      info,
      hand: 'linear-gradient(180deg, #0b1a14 0%, #08120e 100%)',
      mat: {
        backgroundColor: '#10251c',
        backgroundImage: `${FELT_GRAIN}, radial-gradient(95% 85% at 50% 46%, rgba(255,255,255,.075) 0%, transparent 62%), radial-gradient(120% 110% at 50% 50%, transparent 45%, rgba(0,0,0,.42) 100%)`,
        backgroundSize: '180px 180px, auto, auto',
        backgroundPosition: '0 0, center, center',
      },
    };
  if (theme === 'playmats')
    return {
      info,
      hand: tint ? `linear-gradient(180deg, ${mixHex('#08090b', c, 0.12)} 0%, #08090b 100%)` : 'linear-gradient(180deg, #101114 0%, #08090b 100%)',
      mat: {
        backgroundColor: mixHex('#0b0c0f', c, 0.24),
        backgroundImage: 'radial-gradient(115% 105% at 50% 48%, transparent 38%, rgba(0,0,0,.5) 100%), repeating-linear-gradient(45deg, rgba(255,255,255,.022) 0 2px, transparent 2px 6px), repeating-linear-gradient(-45deg, rgba(0,0,0,.05) 0 2px, transparent 2px 6px)',
        backgroundSize: 'auto',
        backgroundPosition: 'center',
      },
    };
  if (light)
    return {
      info,
      hand: tint ? mixHex('#f0efe9', c, 0.12) : 'var(--bg-table)',
      mat: { backgroundColor: hexA(c, 0.07), backgroundImage: 'radial-gradient(rgba(255,255,255,.045) 1px, transparent 1.3px)', backgroundSize: '20px 20px', backgroundPosition: 'center' },
    };
  // classic: the dotted mat, lit from the middle in the player's colour and never falling to black at the edges
  return {
    info,
    hand: tint ? `linear-gradient(180deg, ${mixHex('#101216', c, 0.15)} 0%, #0f1013 100%)` : 'var(--bg-table)',
    mat: {
      backgroundColor: '#131519',
      backgroundImage: `radial-gradient(rgba(255,255,255,.045) 1px, transparent 1.3px), radial-gradient(85% 75% at 50% 45%, ${hexA(c, 0.17)} 0%, ${hexA(c, 0.055)} 78%)`,
      backgroundSize: '20px 20px, auto',
      backgroundPosition: 'center',
    },
  };
}

export function pileThumb(v: GameView, iid: string | undefined): string | null {
  if (!iid) return null;
  const c = v.cards[iid];
  const d = c && !c.hidden ? v.defs[c.def] : undefined;
  if (!d) return null;
  return d.img[0] ? imgUrl(d.img[0]) : null;
}

export function frameColor(d?: CardDef): string {
  return d ? FRAME_COLORS[frameKey(d.colors, d.typeLine)].bg : '#26282e';
}

export function dungeonLabel(s: Seat): string | null {
  if (!s.dungeon) return null;
  const r = DUNGEONS[s.dungeon.name]?.[s.dungeon.room];
  return `${s.dungeon.name.split(' ')[0]} · ${r?.name ?? ''}`;
}
