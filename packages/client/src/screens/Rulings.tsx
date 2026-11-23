import { useQuery } from '@tanstack/react-query';
import { useEffect, useState } from 'react';
import { Link, useSearchParams } from 'react-router-dom';
import type { CardDef } from '@aetherhub/shared';
import { api } from '../lib/api';
import { RulingItem, type Ruling } from '../ui/rulings';
import { Chevron, previewProps, TurnableCard, useDebounced } from '../ui/ui';

interface Result {
  oracleId: string;
  name: string;
  key: string;
  match: 'name' | 'text';
  rulings: Ruling[];
  def: CardDef | null;
}

const EXAMPLES = ['Urza, Lord Protector', 'station', 'commander tax', 'mana value of a split card', 'copy of a double-faced'];

/** Search Scryfall's official rulings by card name or by what the ruling says. */
export default function Rulings() {
  const [params, setParams] = useSearchParams();
  const [q, setQ] = useState(params.get('q') || '');
  const dq = useDebounced(q.trim(), 250);
  useEffect(() => {
    setParams(dq ? { q: dq } : {}, { replace: true });
  }, [dq, setParams]);
  const res = useQuery({
    queryKey: ['rulings-search', dq],
    queryFn: () => api.get<{ ready: boolean; results: Result[] }>(`api/rulings/search?q=${encodeURIComponent(dq)}`),
    enabled: dq.length >= 2,
    placeholderData: (prev) => prev,
  });
  const words = dq.toLowerCase().split(/\s+/).filter((w) => w.length > 1);
  const results = dq.length >= 2 ? res.data?.results ?? [] : [];
  const byName = results.filter((r) => r.match === 'name');
  const byText = results.filter((r) => r.match === 'text');

  const block = (r: Result) => (
    <article key={r.oracleId} className="row" style={{ gap: 18, alignItems: 'flex-start', padding: '16px 18px', borderRadius: 12, background: 'var(--surface)', border: '1px solid var(--line2)' }}>
      <div {...previewProps(r.def, undefined, 0, 'rs:' + r.oracleId)}>
        <TurnableCard def={r.def} turnKey={'rs:' + r.oracleId} style={{ width: 110, height: 154, borderRadius: 7 }} />
      </div>
      <div className="grow" style={{ minWidth: 0 }}>
        <div className="row" style={{ gap: 8, alignItems: 'baseline' }}>
          <h2 style={{ fontSize: 18, margin: 0 }}>{r.name}</h2>
          <span className="mono" style={{ fontSize: 11.5, color: 'var(--mu2)' }}>
            {r.rulings.length} ruling{r.rulings.length === 1 ? '' : 's'}
            {r.match === 'text' ? ' matching' : ''}
          </span>
        </div>
        {r.def && <div style={{ fontSize: 12, color: 'var(--mu)', marginTop: 2 }}>{r.def.typeLine}</div>}
        <ul style={{ margin: '6px 0 0', padding: 0 }}>
          {r.rulings.map((x, i) => (
            <RulingItem key={i} r={x} highlight={r.match === 'text' ? words : undefined} />
          ))}
        </ul>
      </div>
    </article>
  );

  return (
    <div className="screen">
      <header className="page-header" style={{ gap: 24 }}>
        <Link to="/" className="tb" aria-label="Back to main menu">
          <Chevron />Menu
        </Link>
        <h1 style={{ fontSize: 26, margin: 0 }}>Rulings</h1>
        <div className="grow" />
      </header>
      <div className="lg" style={{ overflowY: 'auto', flexGrow: 1 }}>
        <div className="col" style={{ maxWidth: 900, margin: '0 auto', padding: '22px 24px 60px', gap: 16 }}>
          <input
            className="inp lg"
            autoFocus
            value={q}
            onChange={(e) => setQ(e.target.value)}
            placeholder="Card name, or words from a ruling…"
            aria-label="Search rulings"
            style={{ width: '100%' }}
          />
          <div style={{ fontSize: 12.5, color: 'var(--mu)' }}>
            Official Wizards of the Coast rulings and Scryfall notes, updated weekly with the card data. Search a card's name to see all its rulings, or search for words to find every ruling
            that mentions them.
          </div>
          {res.data && !res.data.ready && <div className="warnbox">Rulings haven't been downloaded yet. An admin can fetch them with Settings → Card data → Refresh now.</div>}
          {dq.length < 2 && (
            <div className="row" style={{ gap: 6, flexWrap: 'wrap' }}>
              <span style={{ fontSize: 12.5, color: 'var(--mu2)' }}>Try:</span>
              {EXAMPLES.map((e) => (
                <button key={e} className="mbtn" onClick={() => setQ(e)}>
                  {e}
                </button>
              ))}
            </div>
          )}
          {dq.length >= 2 && res.data?.ready && !results.length && !res.isFetching && <div style={{ color: 'var(--mu)', padding: 30, textAlign: 'center' }}>No rulings found for “{dq}”.</div>}
          {byName.length > 0 && (
            <>
              <h3 style={{ fontSize: 13, color: 'var(--mu2)', textTransform: 'uppercase', letterSpacing: 0.6, margin: '6px 0 0' }}>Cards named “{dq}”</h3>
              {byName.map(block)}
            </>
          )}
          {byText.length > 0 && (
            <>
              <h3 style={{ fontSize: 13, color: 'var(--mu2)', textTransform: 'uppercase', letterSpacing: 0.6, margin: '6px 0 0' }}>Rulings mentioning “{dq}”</h3>
              {byText.map(block)}
            </>
          )}
        </div>
      </div>
    </div>
  );
}
