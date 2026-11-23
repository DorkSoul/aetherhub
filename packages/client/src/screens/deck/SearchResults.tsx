import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import type { CardDef, Section } from '@aetherhub/shared';
import { PIP_COLORS, SECTION_NAMES } from '@aetherhub/shared';
import { CardImage, Pips, previewProps, Switch } from '../../ui/ui';

export interface SearchResultsProps {
  /** the search box the panel hangs under (the visual panel spans the window from its bottom edge) */
  anchor: React.RefObject<HTMLDivElement | null>;
  query: string;
  cards: CardDef[];
  total: number;
  fetching: boolean;
  hasMore: boolean;
  loadMore: () => void;
  view: 'visual' | 'list';
  onView: (v: 'visual' | 'list') => void;
  target: Section;
  commandZone: boolean;
  onAdd: (key: string, section: Section) => void;
  onClose: () => void;
}

const SYNTAX = 'syntax t: o: c: id: mv: pow: tou: r: s: is: f:';

/**
 * Card search results in the deck builder. Visual (the default): a panel the full width of the window, from under
 * the search bar to the bottom of the screen, with a grid of card images that loads more as it scrolls. List: the
 * compact dropdown of names, types and costs.
 */
export function SearchResults(p: SearchResultsProps) {
  const [top, setTop] = useState(0);
  useLayoutEffect(() => {
    const place = () => setTop(Math.round((p.anchor.current?.getBoundingClientRect().bottom ?? 100) + 8));
    place();
    window.addEventListener('resize', place);
    return () => window.removeEventListener('resize', place);
  }, [p.anchor, p.view]);
  const sentinel = useRef<HTMLDivElement>(null);
  const { hasMore, fetching, loadMore } = p;
  useEffect(() => {
    const el = sentinel.current;
    if (!el || !hasMore) return;
    const io = new IntersectionObserver((es) => {
      if (es.some((e) => e.isIntersecting) && !fetching) loadMore();
    });
    io.observe(el);
    return () => io.disconnect();
  }, [hasMore, fetching, loadMore, p.view]);

  const targetLabel = p.target === 'main' ? 'Main' : SECTION_NAMES[p.target];
  const header = (
    <div className="row" style={{ fontSize: 11.5, color: 'var(--mu2)', padding: '6px 8px', gap: 10, flexWrap: 'wrap' }}>
      <span className="grow">{p.fetching && !p.cards.length ? 'Searching…' : p.cards.length ? `${p.total} match${p.total === 1 ? '' : 'es'} · ${SYNTAX}` : `No cards match “${p.query}”`}</span>
      <label className="row" style={{ gap: 7, cursor: 'pointer', color: 'var(--tx3)', fontSize: 12 }}>
        Card images
        <Switch on={p.view === 'visual'} onChange={(v) => p.onView(v ? 'visual' : 'list')} label="Show card images" />
      </label>
      {p.view === 'visual' && (
        <button className="tb sm" onClick={p.onClose}>
          Close
        </button>
      )}
    </div>
  );
  const more = p.hasMore && (
    <div ref={sentinel} style={{ padding: 14, textAlign: 'center', fontSize: 12, color: 'var(--mu2)' }}>
      {p.fetching ? 'Loading more…' : ''}
    </div>
  );

  if (p.view === 'list')
    return (
      <div className="lg" style={{ position: 'absolute', left: 0, top: 42, width: 560, maxWidth: '90vw', maxHeight: 420, overflowY: 'auto', background: 'var(--surface2)', border: '1px solid var(--line4)', borderRadius: 10, boxShadow: '0 16px 36px rgba(0,0,0,.55)', zIndex: 50, padding: 5 }}>
        {header}
        {p.cards.map((c) => (
          <div key={c.key} className="res row" style={{ gap: 10, padding: '7px 8px', borderRadius: 7 }} {...previewProps(c)}>
            <div style={{ width: 4, height: 28, borderRadius: 2, background: c.colors.length === 1 ? PIP_COLORS[c.colors[0]][0] : c.colors.length > 1 ? '#c19a47' : '#8d939c' }} />
            <div className="grow">
              <div className="row" style={{ fontSize: 13, fontWeight: 500, gap: 6 }}>
                {c.name}
                {c.custom && <span style={{ fontSize: 10, color: 'var(--acc-ink)', background: '#9b8cf0', borderRadius: 4, padding: '0 5px' }}>custom</span>}
              </div>
              <div style={{ fontSize: 11.5, color: 'var(--mu)' }}>{c.typeLine}</div>
            </div>
            <Pips cost={c.manaCost} />
            <AddButtons c={c} {...p} targetLabel={targetLabel} />
          </div>
        ))}
        {more}
      </div>
    );

  return (
    <div className="lg col" style={{ position: 'fixed', left: 0, right: 0, top, bottom: 0, background: 'var(--surface2)', borderTop: '1px solid var(--line4)', boxShadow: '0 -6px 24px rgba(0,0,0,.35)', zIndex: 50 }} role="dialog" aria-label="Card search results">
      <div style={{ padding: '4px 16px 0', flexShrink: 0 }}>{header}</div>
      <div className="lg" style={{ flexGrow: 1, overflowY: 'auto', padding: '6px 16px 24px', minHeight: 0 }}>
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(168px, 1fr))', gap: 14 }}>
          {p.cards.map((c) => (
            <div key={c.key} className="col" style={{ gap: 6 }}>
              <button title={`Add ${c.name} to ${targetLabel}`} onClick={() => p.onAdd(c.key, p.target)} style={{ padding: 0, border: 0, background: 'none', cursor: 'pointer', position: 'relative' }} {...previewProps(c)}>
                <CardImage def={c} style={{ width: '100%', aspectRatio: '63 / 88', height: 'auto', borderRadius: '4.75% / 3.4%' }} />
                {c.custom && <span style={{ position: 'absolute', left: 6, top: 6, fontSize: 10, color: 'var(--acc-ink)', background: '#9b8cf0', borderRadius: 4, padding: '0 5px' }}>custom</span>}
              </button>
              <div className="row" style={{ gap: 4, justifyContent: 'center', flexWrap: 'wrap' }}>
                <AddButtons c={c} {...p} targetLabel={targetLabel} />
              </div>
            </div>
          ))}
        </div>
        {more}
      </div>
    </div>
  );
}

function AddButtons({ c, target, targetLabel, commandZone, onAdd }: { c: CardDef; target: Section; targetLabel: string; commandZone: boolean; onAdd: (key: string, section: Section) => void }) {
  return (
    <>
      <button className="mbtn" onClick={() => onAdd(c.key, target)}>
        + {targetLabel}
      </button>
      {target === 'main' && (
        <button className="mbtn" onClick={() => onAdd(c.key, 'maybe')}>
          + Maybe
        </button>
      )}
      {target === 'main' && commandZone && (
        <button className="mbtn" onClick={() => onAdd(c.key, 'commander')} title="Add as commander">
          ★
        </button>
      )}
    </>
  );
}
