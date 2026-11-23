import { useQuery } from '@tanstack/react-query';
import { useState, type ReactNode } from 'react';
import { Link } from 'react-router-dom';
import { api } from '../lib/api';
import { Pips } from './ui';

export interface Ruling {
  oracleId: string;
  source: string;
  date: string;
  comment: string;
}

/** Ruling text with {T}, {2}{U} … drawn as mana symbols, and optional search words highlighted. */
export function RulingText({ text, highlight }: { text: string; highlight?: string[] }) {
  const parts = text.split(/((?:\{[^}]+\})+)/g);
  const words = (highlight || []).filter((w) => w.length > 1).map((w) => w.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'));
  const re = words.length ? new RegExp(`(${words.join('|')})`, 'gi') : null;
  const mark = (s: string, k: number): ReactNode =>
    re
      ? s.split(re).map((p, i) =>
          i % 2 ? (
            <mark key={`${k}-${i}`} style={{ background: 'rgba(var(--acc-rgb),.28)', color: 'inherit', borderRadius: 3, padding: '0 1px' }}>
              {p}
            </mark>
          ) : (
            p
          ),
        )
      : s;
  return (
    <>
      {parts.map((p, i) =>
        i % 2 ? (
          <span key={i} style={{ display: 'inline-flex', verticalAlign: '-2px', margin: '0 1px' }}>
            <Pips cost={p} size={13} />
          </span>
        ) : (
          mark(p, i)
        ),
      )}
    </>
  );
}

export function RulingItem({ r, highlight }: { r: Ruling; highlight?: string[] }) {
  return (
    <li style={{ padding: '8px 0', borderBottom: '1px solid var(--line)', listStyle: 'none' }}>
      <div style={{ fontSize: 11, color: 'var(--mu2)', marginBottom: 3 }}>
        {r.date}
        {r.source && r.source !== 'wotc' ? ` · ${r.source === 'scryfall' ? 'Scryfall note' : r.source}` : ''}
      </div>
      <div style={{ fontSize: 13, lineHeight: 1.5, color: 'var(--tx2)' }}>
        <RulingText text={r.comment} highlight={highlight} />
      </div>
    </li>
  );
}

/** Official rulings for one card, with a filter when there are many. */
/** `link` shows a link to the Rulings page (off on the table, where leaving would interrupt the game). */
export function RulingsList({ oracleId, maxHeight, title = true, link = true }: { oracleId: string | undefined; maxHeight?: number; title?: boolean; link?: boolean }) {
  const [filter, setFilter] = useState('');
  const q = useQuery({
    queryKey: ['rulings', oracleId],
    queryFn: () => api.get<{ rulings: Ruling[]; ready: boolean }>(`api/cards/rulings/${oracleId}`),
    enabled: !!oracleId,
    staleTime: 3600_000,
  });
  if (!oracleId) return null;
  const all = q.data?.rulings ?? [];
  const words = filter.trim().toLowerCase().split(/\s+/).filter(Boolean);
  const shown = words.length ? all.filter((r) => words.every((w) => r.comment.toLowerCase().includes(w))) : all;
  return (
    <section aria-label="Rulings">
      {title && (
        <div className="row" style={{ gap: 8, marginBottom: 4 }}>
          <h3 style={{ fontSize: 14, margin: 0 }}>Rulings</h3>
          <span className="mono" style={{ fontSize: 11.5, color: 'var(--mu2)' }}>
            {q.isLoading ? '…' : all.length}
          </span>
          <div className="grow" />
          {link && (
            <Link to="/rulings" style={{ fontSize: 12 }}>
              Search all rulings
            </Link>
          )}
        </div>
      )}
      {all.length > 5 && <input className="inp" value={filter} onChange={(e) => setFilter(e.target.value)} placeholder="Filter these rulings…" aria-label="Filter rulings" style={{ width: '100%', height: 30, fontSize: 12.5, margin: '4px 0' }} />}
      {q.data && !q.data.ready && <div style={{ fontSize: 12.5, color: 'var(--mu)' }}>Rulings haven't been downloaded yet.</div>}
      {q.data?.ready && !all.length && <div style={{ fontSize: 12.5, color: 'var(--mu)', padding: '4px 0' }}>No official rulings for this card.</div>}
      <ul style={{ margin: 0, padding: 0, maxHeight, overflowY: maxHeight ? 'auto' : undefined }}>
        {shown.map((r, i) => (
          <RulingItem key={i} r={r} highlight={words} />
        ))}
      </ul>
      {words.length > 0 && all.length > 0 && !shown.length && <div style={{ fontSize: 12.5, color: 'var(--mu)' }}>No rulings mention that.</div>}
    </section>
  );
}
