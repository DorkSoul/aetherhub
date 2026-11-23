import { useEffect, type ReactNode } from 'react';
import type { CardDef, DeckCard, DeckStats, GroupBy, SortBy } from '@aetherhub/shared';
import { PIP_COLORS, RARITY_NAME } from '@aetherhub/shared';
import { useApp } from '../../lib/store';
import { Select } from '../../ui/Select';
import { CardImage, Chevron, ManaSymbol, Pips, previewProps, toggleTurned, TurnButton, usePreview, useTurned } from '../../ui/ui';

/** Card views shared by the deck builder and draft / sealed deck building: the toolbar (size, preview, view, group,
 *  sort), the four views of grouped cards, and the stats side panel. Size, preview and view are user prefs. */

export interface CardGroup {
  title: string;
  cards: DeckCard[];
  count: number;
}

export const VIEW_CSS = `.dtile{cursor:pointer;transition:transform .12s}.dtile:hover{transform:translateY(-3px)}.dstk{transition:transform .12s}.dstk:hover{transform:translateX(6px)}.drow:hover{background:var(--surface2)}.res:hover{background:var(--surface4)}`;

export const GROUP_OPTIONS: [GroupBy, string][] = [
  ['type', 'Type'],
  ['mv', 'Mana value'],
  ['color', 'Color'],
  ['ci', 'Color identity'],
  ['rarity', 'Rarity'],
  ['set', 'Set'],
  ['tag', 'Tag'],
  ['section', 'Section'],
  ['none', 'Nothing'],
];
export const SORT_OPTIONS: [SortBy, string][] = [
  ['name', 'Name'],
  ['mv', 'Mana value'],
  ['color', 'Color'],
  ['type', 'Type'],
  ['rarity', 'Rarity'],
  ['price', 'Price'],
  ['added', 'Date added'],
];

/** The hover preview follows the deck builder's preview prefs while the screen is open. */
export function useDeckPreview() {
  const on = useApp((s) => s.me?.prefs.deckPreview ?? true);
  const size = useApp((s) => s.me?.prefs.deckPreviewSize || 100);
  useEffect(() => {
    usePreview.setState({ scale: size / 100, enabled: on });
    return () => usePreview.setState({ scale: 1, enabled: true });
  }, [on, size]);
}

/** Size, preview, view, group and sort controls (a fragment for a toolbar row). */
export function ViewControls({
  groupBy,
  onGroupBy,
  sortBy,
  onSortBy,
  groupOptions = GROUP_OPTIONS,
  sortOptions = SORT_OPTIONS,
}: {
  groupBy: GroupBy;
  onGroupBy: (v: GroupBy) => void;
  sortBy: SortBy;
  onSortBy: (v: SortBy) => void;
  groupOptions?: [GroupBy, string][];
  sortOptions?: [SortBy, string][];
}) {
  const prefs = useApp((s) => s.me!.prefs);
  const setPrefs = useApp((s) => s.setPrefs);
  const view = prefs.deckView;
  const Z = prefs.deckZoom || 1;
  const pvOn = prefs.deckPreview;
  const pvSize = prefs.deckPreviewSize || 100;
  return (
    <>
      <div className="row" style={{ gap: 6 }} role="group" aria-label="Card size">
        <span style={{ fontSize: 12, color: 'var(--mu2)' }}>Size</span>
        <button className="sq" aria-label="Smaller cards" title="Smaller cards — fit more on screen" onClick={() => setPrefs({ deckZoom: Math.max(0.5, Math.round((Z - 0.1) * 100) / 100) })}>
          −
        </button>
        <input type="range" min={50} max={200} step={5} value={Math.round(Z * 100)} onChange={(e) => setPrefs({ deckZoom: parseInt(e.target.value, 10) / 100 })} aria-label="Card size" style={{ width: 64, accentColor: 'var(--acc)' }} />
        <button className="sq" aria-label="Bigger cards" onClick={() => setPrefs({ deckZoom: Math.min(2, Math.round((Z + 0.1) * 100) / 100) })}>
          +
        </button>
        <span className="mono" style={{ fontSize: 11.5, color: 'var(--mu)', width: 34 }}>
          {Math.round(Z * 100)}%
        </span>
      </div>
      <div className="row" style={{ gap: 6 }} role="group" aria-label="Card preview">
        <button className={'tb sm' + (pvOn ? ' on' : '')} aria-pressed={pvOn} title="Show a large preview when hovering a card" onClick={() => setPrefs({ deckPreview: !pvOn })}>
          Preview {pvOn ? 'on' : 'off'}
        </button>
        <input
          type="range"
          min={60}
          max={180}
          step={10}
          value={pvSize}
          disabled={!pvOn}
          onChange={(e) => setPrefs({ deckPreviewSize: parseInt(e.target.value, 10) })}
          aria-label="Preview size"
          title="Preview size"
          style={{ width: 64, accentColor: 'var(--acc)', opacity: pvOn ? 1 : 0.4 }}
        />
        <span className="mono" style={{ fontSize: 11.5, color: 'var(--mu)', width: 38, opacity: pvOn ? 1 : 0.4 }}>
          {pvSize}%
        </span>
      </div>
      <div className="segs" role="group" aria-label="View">
        {(
          [
            ['visual', 'Visual'],
            ['stacks', 'Stacks'],
            ['text', 'Text'],
            ['table', 'Table'],
          ] as const
        ).map(([v, l]) => (
          <button key={v} className={'seg' + (view === v ? ' on' : '')} onClick={() => setPrefs({ deckView: v })}>
            {l}
          </button>
        ))}
      </div>
      <label className="row" style={{ fontSize: 12, color: 'var(--mu2)', gap: 6 }}>
        Group
        <Select aria-label="Group by" value={groupBy} onChange={onGroupBy} options={groupOptions.map(([value, label]) => ({ value, label }))} />
      </label>
      <label className="row" style={{ fontSize: 12, color: 'var(--mu2)', gap: 6 }}>
        Sort
        <Select aria-label="Sort by" value={sortBy} onChange={onSortBy} options={sortOptions.map(([value, label]) => ({ value, label }))} />
      </label>
    </>
  );
}

/** Per-card extras a screen adds to the shared views. */
export interface ViewDecor {
  /** legality problems: a red ring and badge, red name */
  reasons?: (c: DeckCard) => string[];
  selected?: (c: DeckCard) => boolean;
  /** the def to show (art overrides); defaults to `defs[c.def]` */
  shown?: (c: DeckCard) => CardDef | undefined;
  /** extra overlays on a visual tile (tags, foil) */
  tileExtra?: (c: DeckCard, zoom: number) => ReactNode;
  /** after the name in the text view */
  nameExtra?: (c: DeckCard) => ReactNode;
  /** a line under the hover preview */
  previewExtra?: (c: DeckCard) => ReactNode;
  /** the table's Tags cell; the column is left out without it */
  tags?: (c: DeckCard) => ReactNode;
  /** prefix of the keys that remember turned double-faced cards */
  turnPrefix?: string;
}

/** Grouped cards in the user's chosen view (visual / stacks / text / table) at the chosen size. */
export function CardGroupsView({
  groups,
  defs,
  groupBy,
  onClick,
  onContextMenu,
  decor = {},
}: {
  groups: CardGroup[];
  defs: Record<string, CardDef>;
  groupBy: GroupBy;
  onClick?: (e: React.MouseEvent, c: DeckCard) => void;
  onContextMenu?: (e: React.MouseEvent, c: DeckCard) => void;
  decor?: ViewDecor;
}) {
  const view = useApp((s) => s.me!.prefs.deckView);
  const Z = useApp((s) => s.me!.prefs.deckZoom || 1);
  const turned = useTurned((s) => s.m);
  const reasons = (c: DeckCard) => decor.reasons?.(c) ?? [];
  const selected = (c: DeckCard) => decor.selected?.(c) ?? false;
  const shown = (c: DeckCard) => (decor.shown ? decor.shown(c) : defs[c.def]);
  const tk = (c: DeckCard) => (decor.turnPrefix ?? 'dc:') + c.id;
  const pv = (c: DeckCard) => previewProps(shown(c), decor.previewExtra?.(c) ?? null, 0, tk(c));
  const handlers = (c: DeckCard) => ({ onClick: onClick ? (e: React.MouseEvent) => onClick(e, c) : undefined, onContextMenu: onContextMenu ? (e: React.MouseEvent) => onContextMenu(e, c) : undefined });
  const cursor = onClick ? 'pointer' : 'default';

  if (view === 'stacks')
    return (
      <div className="row" style={{ gap: 18, alignItems: 'flex-start', flexWrap: 'wrap' }}>
        {groups.map((g) => {
          const sw = Math.round(160 * Z);
          const sh = Math.round(224 * Z);
          const step = Math.round(34 * Z);
          return (
            <section key={g.title} style={{ width: Math.round(168 * Z) }}>
              <div className="row" style={{ alignItems: 'baseline', gap: 6, marginBottom: 8 }}>
                <h2 className="row" style={{ fontSize: 15, gap: 5 }}>
                  {groupSymbols(groupBy, g.title, 15)}
                  {g.title}
                </h2>
                <span className="mono" style={{ fontSize: 11.5, color: 'var(--mu2)' }}>
                  ({g.count})
                </span>
              </div>
              <div style={{ position: 'relative', height: (g.cards.length - 1) * step + sh }}>
                {g.cards.map((c, i) => {
                  const bad = reasons(c).length > 0;
                  const d = shown(c);
                  return (
                    <div key={c.id} className="dstk" title={bad ? reasons(c).join(' · ') : undefined} {...handlers(c)} {...pv(c)} style={{ position: 'absolute', left: 0, top: i * step, width: sw, height: sh, borderRadius: 9, cursor, boxShadow: `${selected(c) ? '0 0 0 3px var(--acc)' : bad ? '0 0 0 3px #e2604f' : '0 0 0 0 transparent'}, 0 -2px 6px rgba(0,0,0,.35)` }}>
                      <CardImage def={d} face={turned[tk(c)] ? 1 : 0} style={{ width: '100%', height: '100%', borderRadius: 9 }} />
                      <TurnButton def={d} onTurn={() => toggleTurned(tk(c))} size={Math.min(22, step - 8)} top={Math.round((step - Math.min(22, step - 8)) / 2)} right={c.qty > 1 ? 40 : 6} />
                      {c.qty > 1 && <div style={{ position: 'absolute', right: 6, top: 4, fontSize: 11, fontWeight: 700, background: '#ebe8e2', color: '#15130f', borderRadius: 8, padding: '1px 6px' }}>{c.qty}×</div>}
                    </div>
                  );
                })}
              </div>
            </section>
          );
        })}
      </div>
    );

  if (view === 'text')
    return (
      <div style={{ columnCount: Z < 0.8 ? 4 : Z > 1.1 ? 2 : 3, columnGap: 36 }}>
        {groups.map((g) => (
          <section key={g.title} style={{ breakInside: 'avoid', marginBottom: 18 }}>
            <h2 style={{ fontSize: 15, marginBottom: 6, display: 'flex', alignItems: 'center', gap: 5 }}>
              {groupSymbols(groupBy, g.title, 15)}
              {g.title}{' '}
              <span className="mono" style={{ fontSize: 11.5, color: 'var(--mu2)', fontWeight: 500 }}>
                {g.count}
              </span>
            </h2>
            {g.cards.map((c) => {
              const d = defs[c.def];
              const bad = reasons(c).length > 0;
              return (
                <div key={c.id} className="drow row" title={bad ? reasons(c).join(' · ') : undefined} {...handlers(c)} {...pv(c)} style={{ gap: 8, padding: '3px 6px', borderRadius: 5, cursor, fontSize: 13, background: selected(c) ? 'var(--acc-bg)' : undefined }}>
                  <span className="mono" style={{ color: 'var(--mu)', width: 18 }}>
                    {c.qty}
                  </span>
                  <span className="grow" style={{ color: bad ? 'var(--danger-tx)' : 'var(--tx)' }}>
                    {d?.name}
                    {decor.nameExtra?.(c)}
                  </span>
                  <TurnButton inline def={shown(c)} onTurn={() => toggleTurned(tk(c))} size={18} />
                  <Pips cost={d?.manaCost || ''} size={13} />
                </div>
              );
            })}
          </section>
        ))}
      </div>
    );

  if (view === 'table') {
    const rows = Array.from(new Set(groups.flatMap((g) => g.cards)));
    if (!rows.length) return null;
    const heads = ['Qty', 'Name', 'Cost', 'Type', 'MV', 'Set', 'Rarity', ...(decor.tags ? ['Tags'] : []), 'USD'];
    return (
      <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: 13 }}>
        <thead>
          <tr style={{ textAlign: 'left', color: 'var(--mu2)', fontSize: 11.5, textTransform: 'uppercase', letterSpacing: 0.5 }}>
            {heads.map((h) => (
              <th key={h} style={{ padding: 8, borderBottom: '1px solid var(--line)', fontWeight: 500, textAlign: h === 'USD' ? 'right' : 'left' }}>
                {h}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((c) => {
            const d = defs[c.def];
            const bad = reasons(c).length > 0;
            const td = { padding: '7px 8px', borderBottom: '1px solid var(--surface2)' };
            return (
              <tr key={c.id} className="drow" style={{ cursor, background: selected(c) ? 'var(--acc-bg)' : undefined }} {...handlers(c)} {...pv(c)}>
                <td className="mono" style={{ ...td, color: 'var(--mu)' }}>
                  {c.qty}
                </td>
                <td style={{ ...td, color: bad ? 'var(--danger-tx)' : 'var(--tx)', fontWeight: 500 }}>
                  <span className="row" style={{ gap: 6 }}>
                    {d?.name}
                    <TurnButton inline def={shown(c)} onTurn={() => toggleTurned(tk(c))} size={18} />
                  </span>
                </td>
                <td style={td}>
                  <Pips cost={d?.manaCost || ''} size={13} />
                </td>
                <td style={{ ...td, color: 'var(--tx3)' }}>{d?.typeLine}</td>
                <td className="mono" style={td}>
                  {d?.cmc}
                </td>
                <td className="mono" style={{ ...td, color: 'var(--tx3)' }}>
                  {d?.set?.toUpperCase()}
                </td>
                <td style={{ ...td, color: 'var(--tx3)' }}>{RARITY_NAME[d?.rarity || ''] || ''}</td>
                {decor.tags && <td style={td}>{decor.tags(c)}</td>}
                <td className="mono" style={{ ...td, textAlign: 'right' }}>
                  {d?.usd != null ? '$' + (d.usd * c.qty).toFixed(2) : '—'}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    );
  }

  // visual
  const tileW = Math.round(144 * Z);
  const tileH = Math.round(202 * Z);
  return (
    <div className="col" style={{ gap: 22 }}>
      {groups.map((g) => (
        <section key={g.title}>
          <div className="row" style={{ alignItems: 'baseline', gap: 8, marginBottom: 10, borderBottom: '1px solid var(--line)', paddingBottom: 6 }}>
            <h2 className="row" style={{ fontSize: 17, gap: 6 }}>
              {groupSymbols(groupBy, g.title, 17)}
              {g.title}
            </h2>
            <span className="mono" style={{ fontSize: 12, color: 'var(--mu2)' }}>
              ({g.count})
            </span>
          </div>
          <div style={{ display: 'flex', flexWrap: 'wrap', gap: Math.max(3, Math.round(5 * Z)) }}>
            {g.cards.map((c) => {
              const bad = reasons(c).length > 0;
              const d = shown(c);
              return (
                <div key={c.id} className="dtile" title={bad ? reasons(c).join(' · ') : undefined} {...handlers(c)} {...pv(c)} style={{ width: tileW, height: tileH, position: 'relative', borderRadius: 8, cursor, boxShadow: selected(c) ? '0 0 0 3px var(--acc)' : bad ? '0 0 0 3px #e2604f' : '0 6px 14px rgba(0,0,0,.35)' }}>
                  <CardImage def={d} face={turned[tk(c)] ? 1 : 0} style={{ width: '100%', height: '100%', borderRadius: 8 }} />
                  <TurnButton def={d} onTurn={() => toggleTurned(tk(c))} size={Math.round(Math.max(20, Math.min(32, tileW * 0.17)))} />
                  {c.qty > 1 && <div style={{ position: 'absolute', left: 4, top: 26 * Z, fontSize: 11.5, fontWeight: 700, background: '#ebe8e2', color: '#15130f', borderRadius: 9, padding: '2px 7px', boxShadow: '0 1px 4px rgba(0,0,0,.5)' }}>{c.qty}×</div>}
                  {bad && <div style={{ position: 'absolute', right: 4, top: 26 * Z, width: 20, height: 20, borderRadius: 10, background: '#e2604f', color: '#fff', fontSize: 12, fontWeight: 700, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>!</div>}
                  {decor.tileExtra?.(c, Z)}
                </div>
              );
            })}
          </div>
        </section>
      ))}
    </div>
  );
}

/** The collapsible right-hand panel (open state is the `deckPanel` pref); hidden on small screens. */
export function SidePanel({ children }: { children: ReactNode }) {
  const open = useApp((s) => s.me!.prefs.deckPanel !== false);
  const setPrefs = useApp((s) => s.setPrefs);
  return (
    <>
      <button className="hide-sm panel-tab" onClick={() => setPrefs({ deckPanel: !open })} title={open ? 'Hide the side panel' : 'Show the side panel'} aria-label={open ? 'Hide the side panel' : 'Show the side panel'} aria-expanded={open}>
        <Chevron dir={open ? 'right' : 'left'} size={13} />
      </button>
      {open && (
        <aside className="lg hide-sm" style={{ width: 340, flexShrink: 0, borderLeft: '1px solid var(--line)', background: 'var(--panel)', padding: '18px 20px', overflowY: 'auto', display: 'flex', flexDirection: 'column', gap: 22 }}>
          {children}
        </aside>
      )}
    </>
  );
}

/** Card / land / average MV tiles, the mana curve, colour pips and price. */
export function StatsSections({ stats, cardsLabel }: { stats: DeckStats; cardsLabel?: string | number }) {
  return (
    <>
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 8 }}>
        {[
          ['Cards', cardsLabel ?? (stats.size ? `${stats.total}/${stats.size}` : stats.total)],
          ['Lands', stats.lands],
          ['Avg MV', stats.avgMv.toFixed(2)],
        ].map(([l, v]) => (
          <div key={l as string} style={{ background: 'var(--surface)', border: '1px solid var(--line2)', borderRadius: 9, padding: '9px 10px' }}>
            <div style={{ fontSize: 11, color: 'var(--mu2)' }}>{l}</div>
            <div className="display" style={{ fontSize: 22, marginTop: 2 }}>
              {v}
            </div>
          </div>
        ))}
      </div>
      <section>
        <h3 style={{ fontFamily: 'var(--font)', fontSize: 13, marginBottom: 10 }}>
          Mana curve <span style={{ color: 'var(--mu2)', fontWeight: 400 }}>· nonland</span>
        </h3>
        <div style={{ height: 120, display: 'flex', alignItems: 'flex-end', gap: 6, borderBottom: '1px solid var(--line)' }}>
          {stats.curve.map((n, i) => (
            <div key={i} style={{ flexGrow: 1, flexBasis: 0, display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'flex-end', height: '100%', gap: 4 }}>
              <span className="mono" style={{ fontSize: 10.5, color: 'var(--tx3)' }}>
                {n}
              </span>
              <div style={{ width: '100%', height: Math.round((n / (Math.max(...stats.curve) || 1)) * 92), background: 'var(--acc)', borderRadius: '3px 3px 0 0' }} />
            </div>
          ))}
        </div>
        <div className="row" style={{ gap: 6, marginTop: 5 }}>
          {stats.curve.map((_, i) => (
            <span key={i} className="mono" style={{ flexGrow: 1, flexBasis: 0, textAlign: 'center', fontSize: 10.5, color: 'var(--mu2)' }}>
              {i === 7 ? '7+' : i}
            </span>
          ))}
        </div>
      </section>
      <section>
        <h3 style={{ fontFamily: 'var(--font)', fontSize: 13, marginBottom: 10 }}>Color pips</h3>
        <div className="col" style={{ gap: 7 }}>
          {['W', 'U', 'B', 'R', 'G'].map((c) => {
            const n = stats.pips[c];
            const max = Math.max(...Object.values(stats.pips)) || 1;
            return (
              <div key={c} className="row" style={{ gap: 8, fontSize: 12 }}>
                <ManaSymbol c={c} size={16} title={null} />
                <span style={{ width: 48, color: 'var(--tx3)' }}>{{ W: 'White', U: 'Blue', B: 'Black', R: 'Red', G: 'Green' }[c]}</span>
                <div className="grow" style={{ height: 8, borderRadius: 4, background: 'var(--surface2)', overflow: 'hidden' }}>
                  <div style={{ width: `${(n / max) * 100}%`, height: '100%', background: c === 'W' ? '#d6cbac' : c === 'B' ? '#8a7f78' : PIP_COLORS[c][0] }} />
                </div>
                <span className="mono" style={{ width: 22, textAlign: 'right', color: 'var(--tx3)' }}>
                  {n}
                </span>
              </div>
            );
          })}
        </div>
      </section>
      <section>
        <h3 style={{ fontFamily: 'var(--font)', fontSize: 13, marginBottom: 10 }}>
          Price <span style={{ color: 'var(--mu2)', fontWeight: 400 }}>· from Scryfall bulk data</span>
        </h3>
        <div className="row" style={{ gap: 8 }}>
          {[
            ['USD', '$' + stats.usd.toFixed(2)],
            ['EUR', '€' + stats.eur.toFixed(2)],
          ].map(([l, v]) => (
            <div key={l} className="grow" style={{ background: 'var(--surface)', border: '1px solid var(--line2)', borderRadius: 9, padding: '9px 10px' }}>
              <div style={{ fontSize: 11, color: 'var(--mu2)' }}>{l}</div>
              <div className="mono" style={{ fontSize: 17, fontWeight: 600 }}>
                {v}
              </div>
            </div>
          ))}
        </div>
      </section>
    </>
  );
}

const COLOR_GROUP: Record<string, string> = { White: 'W', Blue: 'U', Black: 'B', Red: 'R', Green: 'G', Colorless: 'C' };
/** Mana symbols in front of a colour / colour-identity group heading ("Green", "WUB"). */
export function groupSymbols(by: GroupBy, title: string, size: number) {
  const syms = by === 'color' && COLOR_GROUP[title] ? [COLOR_GROUP[title]] : by === 'ci' && /^[WUBRG]+$/.test(title) ? title.split('') : by === 'ci' && title === 'Colorless' ? ['C'] : [];
  if (!syms.length) return null;
  return (
    <span style={{ display: 'inline-flex', gap: 2 }}>
      {syms.map((c, i) => (
        <ManaSymbol key={i} c={c} size={size} title={null} />
      ))}
    </span>
  );
}
