// Mana symbols for rules games: the mana pool box shown on the table and the cost text in the payment prompt.
import { MANA_SYMBOL } from '../../ui/manaSymbols';
const COLORS: Record<string, { bg: string; fg: string; name: string }> = {
  W: { bg: '#f4efd0', fg: '#4a4630', name: 'White' },
  U: { bg: '#4a8fd9', fg: '#fff', name: 'Blue' },
  B: { bg: '#3d3a42', fg: '#e8e2ee', name: 'Black' },
  R: { bg: '#d9553a', fg: '#fff', name: 'Red' },
  G: { bg: '#3f9a55', fg: '#fff', name: 'Green' },
  C: { bg: '#b9b9bf', fg: '#2a2a2e', name: 'Colorless' },
};
export const POOL_COLORS = ['W', 'U', 'B', 'R', 'G', 'C'] as const;

/** One mana symbol (a colored circle with its letter, or the amount for generic mana), optionally with a count badge. */
export function ManaChip({ sym, count, size = 22, dim, hot, onClick, title }: { sym: string; count?: number; size?: number; dim?: boolean; hot?: boolean; onClick?: () => void; title?: string }) {
  const c = COLORS[sym];
  const generic = !c;
  const art = MANA_SYMBOL[sym];
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={!onClick}
      title={title ?? (c ? c.name : sym)}
      aria-label={title ?? (c ? c.name : sym)}
      style={{
        position: 'relative', width: size, height: size, borderRadius: '50%', border: art ? 0 : hot ? '2px solid var(--acc)' : '1px solid rgba(0,0,0,.45)', padding: 0,
        background: art ? `${art} center / 100% 100% no-repeat` : c ? c.bg : '#c9c6bf', color: c ? c.fg : '#2a2a2e', fontSize: Math.round(size * 0.52), fontWeight: 700, lineHeight: 1, display: 'inline-flex',
        alignItems: 'center', justifyContent: 'center', opacity: dim ? (art ? 0.6 : 0.35) : 1, cursor: onClick ? 'pointer' : 'default',
        boxShadow: hot ? '0 0 0 2px var(--acc), 0 0 8px var(--acc)' : art ? '0 1px 2px rgba(0,0,0,.6)' : 'none',
      }}
    >
      {art ? null : generic ? sym.replace(/[{}]/g, '') : sym}
      {count !== undefined && count > 0 && (
        <span style={{ position: 'absolute', right: -4, bottom: -5, minWidth: 14, height: 14, borderRadius: 7, background: 'var(--surface)', color: 'var(--tx)', border: '1px solid var(--line2)', fontSize: 9.5, fontWeight: 700, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
          {count}
        </span>
      )}
    </button>
  );
}

/** "{1}{R}{R/P}" as symbols. */
export function ManaCostText({ cost, size = 22 }: { cost: string; size?: number }) {
  const parts = cost.match(/\{[^}]+\}/g) ?? (cost ? [cost] : []);
  if (!parts.length) return <span>{'{0}'}</span>;
  return (
    <span style={{ display: 'inline-flex', gap: 3, verticalAlign: 'middle', flexWrap: 'wrap' }}>
      {parts.map((p, i) => {
        const inner = p.replace(/[{}]/g, '');
        return <ManaChip key={i} sym={inner.length === 1 && COLORS[inner] ? inner : inner.replace('/P', '').length === 1 && COLORS[inner.replace('/P', '')] ? inner.replace('/P', '') : inner} size={size} title={p} />;
      })}
    </span>
  );
}

/** The mana pool shown under a seat's command zone: each color's mana (tapping lands fills it), clickable while paying for a spell. */
export function ManaPoolBox({ pool, usable, pay, label }: { pool: Record<string, number>; usable: string[]; pay: ((c: string) => void) | null; label: string }) {
  return (
    <div
      data-mana-pool={label}
      style={{ flexShrink: 0, display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 6, justifyItems: 'center', padding: '6px 4px', borderRadius: 7, background: 'var(--surface2)', border: `1px solid ${pay && usable.length ? 'var(--acc)' : 'var(--line)'}` }}
      title="Mana pool: tapping a land adds its mana here; click a color to spend it while paying for a spell"
    >
      {POOL_COLORS.map((c) => {
        const n = pool[c] ?? 0;
        const can = !!pay && usable.includes(c);
        return <ManaChip key={c} sym={c} count={n} size={24} dim={n === 0} hot={can} onClick={can ? () => pay!(c) : undefined} />;
      })}
    </div>
  );
}

/** A mana symbol inline in text (no button, so it can sit inside one). */
function ManaGlyph({ inner, size }: { inner: string; size: number }) {
  const sym = COLORS[inner] ? inner : COLORS[inner.replace('/P', '')] ? inner.replace('/P', '') : inner;
  const c = COLORS[sym];
  const art = MANA_SYMBOL[sym];
  return (
    <span
      title={`{${inner}}`}
      style={{
        display: 'inline-flex', alignItems: 'center', justifyContent: 'center', width: size, height: size, borderRadius: '50%', verticalAlign: '-0.2em', flexShrink: 0,
        background: art ? `${art} center / 100% 100% no-repeat` : c ? c.bg : '#c9c6bf', color: c ? c.fg : '#2a2a2e', fontSize: Math.round(size * 0.55), fontWeight: 700, lineHeight: 1,
        border: art ? 0 : '1px solid rgba(0,0,0,.45)', boxShadow: art ? '0 1px 2px rgba(0,0,0,.6)' : 'none',
      }}
    >
      {art ? null : inner}
    </span>
  );
}

/** Text with its {W}{U}{3} symbols drawn as real mana symbols. More than 3 of the same symbol in a row shows as one symbol and a count. */
export function RichLabel({ text, size = 16 }: { text: string; size?: number }) {
  if (!text.includes('{')) return <>{text}</>;
  const out: React.ReactNode[] = [];
  const re = /(?:\{([^}]+)\})+/g;
  let last = 0;
  let key = 0;
  for (let m = re.exec(text); m; m = re.exec(text)) {
    if (m.index > last) out.push(text.slice(last, m.index));
    const syms = m[0].match(/\{[^}]+\}/g)!.map((s) => s.slice(1, -1));
    const runs: [string, number][] = [];
    for (const s of syms) {
      const r = runs[runs.length - 1];
      if (r && r[0] === s) r[1]++;
      else runs.push([s, 1]);
    }
    out.push(
      <span key={key++} style={{ display: 'inline-flex', alignItems: 'center', gap: 2, verticalAlign: 'middle' }}>
        {runs.flatMap(([s, n], i) =>
          n > 3
            ? [<span key={i} style={{ display: 'inline-flex', alignItems: 'center', gap: 1 }}><ManaGlyph inner={s} size={size} /><span style={{ fontSize: '0.85em' }}>×{n}</span></span>]
            : Array.from({ length: n }, (_, j) => <ManaGlyph key={`${i}.${j}`} inner={s} size={size} />),
        )}
      </span>,
    );
    last = m.index + m[0].length;
  }
  if (last < text.length) out.push(text.slice(last));
  return <>{out}</>;
}
