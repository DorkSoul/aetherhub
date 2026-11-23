import { useQuery, useQueryClient } from '@tanstack/react-query';
import { Select } from '../../ui/Select';
import { useEffect, useRef, useState } from 'react';
import { Link, useNavigate, useSearchParams } from 'react-router-dom';
import type { Deck, ParsedLine, Section } from '@aetherhub/shared';
import { FORMATS, SECTION_NAMES } from '@aetherhub/shared';
import { api } from '../../lib/api';
import { errMsg, useApp } from '../../lib/store';
import { Chevron, useDebounced } from '../../ui/ui';

interface Matched extends ParsedLine {
  key: string;
  resolvedName?: string;
  resolvedSet?: string;
}
interface Review extends ParsedLine {
  suggestions: { name: string; key: string }[];
}
interface ParseRes {
  format: 'plain' | 'arena' | 'mtgo';
  lineCount: number;
  matched: Matched[];
  review: Review[];
}

const FMT_LABEL = { plain: 'Plain text', arena: 'Arena text', mtgo: 'MTGO .dek' };
const SEC_ORDER: Section[] = ['commander', 'companion', 'main', 'side', 'maybe', 'tokens', 'planar', 'schemes'];

export default function Import() {
  const [params] = useSearchParams();
  const targetId = params.get('deck') ? Number(params.get('deck')) : null;
  const target = useQuery({ queryKey: ['deck', targetId], queryFn: () => api.get<Deck>(`api/decks/${targetId}`), enabled: !!targetId });
  const toast = useApp((s) => s.toast);
  const qc = useQueryClient();
  const nav = useNavigate();
  const [text, setText] = useState('');
  const [name, setName] = useState('Imported deck');
  const [format, setFormat] = useState('commander');
  const [picks, setPicks] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);
  const file = useRef<HTMLInputElement>(null);
  const dt = useDebounced(text, 400);
  const parsed = useQuery({ queryKey: ['import-parse', dt], queryFn: () => api.post<ParseRes>('api/import/parse', { text: dt }), enabled: dt.trim().length > 0 });
  const p = dt.trim() ? parsed.data : undefined;

  // Default the deck name to the commander once the list is parsed.
  useEffect(() => {
    const cmd = p?.matched.find((m) => m.section === 'commander');
    if (cmd && !targetId && name === 'Imported deck') setName(`${(cmd.resolvedName || cmd.name).split(',')[0]} (imported)`);
  }, [p]);

  useEffect(() => {
    if (target.data) {
      setName(target.data.name);
      setFormat(target.data.format);
    }
  }, [target.data]);

  const rkey = (r: Review) => `${r.line}|${r.raw}`;
  const resolved: { key: string; section: Section; qty: number; foil: boolean; tags: string[]; globalTags: string[] }[] = [];
  let skipped = 0;
  let open = 0;
  for (const r of p?.review || []) {
    const pick = picks[rkey(r)];
    if (pick === '__skip') skipped++;
    else if (pick) resolved.push({ key: pick, section: r.section, qty: r.qty, foil: r.foil, tags: r.tags, globalTags: r.globalTags ?? [] });
    else open++;
  }
  const rows = [...(p?.matched || []).map((m) => ({ key: m.key, section: m.section, qty: m.qty, foil: m.foil, tags: m.tags, globalTags: m.globalTags ?? [], name: m.resolvedName || m.name, set: m.resolvedSet })), ...resolved.map((r) => ({ ...r, name: (p?.review || []).flatMap((x) => x.suggestions).find((s) => s.key === r.key)?.name || '', set: '' }))];
  const cards = rows.reduce((s, r) => s + r.qty, 0);

  const commit = async () => {
    setBusy(true);
    try {
      const cmd = rows.find((r) => r.section === 'commander');
      const finalName = name === 'Imported deck' && cmd?.name && !targetId ? `${cmd.name.split(',')[0]} (imported)` : name;
      const d = await api.post<Deck>('api/import/commit', { name: finalName, format, deckId: targetId, cards: rows.map((r) => ({ key: r.key, section: r.section, qty: r.qty, foil: r.foil, tags: r.tags, globalTags: r.globalTags })) });
      qc.invalidateQueries({ queryKey: ['decks'] });
      qc.setQueryData(['deck', d.id], d);
      nav(`/decks/${d.id}`);
    } catch (e) {
      toast(errMsg(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="screen">
      <header className="page-header">
        <Link to={targetId ? `/decks/${targetId}` : '/decks'} className="tb" aria-label="Back to decks">
          <Chevron />{targetId ? 'Deck' : 'Decks'}
        </Link>
        <h1>{targetId ? `Import into “${target.data?.name ?? '…'}”` : 'Import deck'}</h1>
        <div className="grow" />
        <span className="hide-sm" style={{ fontSize: 12.5, color: 'var(--mu2)' }}>
          Plain text (Moxfield, Archidekt, MTGGoldfish), Arena text or an MTGO .dek file. No URL import.
        </span>
      </header>
      <div style={{ flexGrow: 1, display: 'flex', gap: 24, padding: '22px 28px 26px', minHeight: 0, flexWrap: 'wrap', overflowY: 'auto' }}>
        <section className="col" style={{ flex: '1 1 460px', maxWidth: 560, gap: 12, minHeight: 360 }}>
          <div className="row">
            <input className="inp grow" aria-label="Deck name" value={name} onChange={(e) => setName(e.target.value)} />
            <Select aria-label="Format" value={format} onChange={setFormat} options={FORMATS.map((f) => ({ value: f.id as string, label: f.name }))} />
            <button className="tb" onClick={() => file.current?.click()}>
              Open .dek file…
            </button>
            <input
              ref={file}
              type="file"
              accept=".dek,.txt,.xml,text/plain"
              hidden
              onChange={async (e) => {
                const f = e.target.files?.[0];
                if (!f) return;
                setText(await f.text());
                setPicks({});
                if (name === 'Imported deck') setName(f.name.replace(/\.(dek|txt|xml)$/i, ''));
              }}
            />
          </div>
          <div className="row" style={{ fontSize: 12, color: 'var(--mu)', gap: 8 }}>
            <span>Detected:</span>
            <span style={{ padding: '3px 8px', borderRadius: 6, background: 'var(--acc-bg)', color: 'var(--acc-tx)', border: '1px solid var(--acc-bd)' }}>{p ? FMT_LABEL[p.format] : '—'}</span>
            <span style={{ marginLeft: 'auto' }}>{p ? `${p.lineCount} lines · sections, (SET) number and *F* recognised` : parsed.isFetching ? 'Reading…' : ''}</span>
          </div>
          <textarea
            aria-label="Paste deck list"
            value={text}
            onChange={(e) => setText(e.target.value)}
            spellCheck={false}
            placeholder={'Commander\n1 Atraxa, Praetors\' Voice (C16) 28\n\nMain\n1 Sol Ring (C21) 263\n1 Arcane Signet *F*\n…'}
            className="inp mono lg"
            style={{ flexGrow: 1, minHeight: 300, fontSize: 12.5, lineHeight: 1.6, padding: '14px 16px', resize: 'none' }}
          />
        </section>
        <section className="col" style={{ flex: '1 1 480px', gap: 14, minWidth: 0 }}>
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)', gap: 10 }}>
            {[
              ['Cards matched', cards, 'var(--tx)'],
              ['Needs review', open, open ? 'var(--danger-tx)' : 'var(--ok)'],
              ['Resolved', resolved.length, 'var(--ok)'],
              ['Skipped', skipped, 'var(--mu)'],
            ].map(([l, v, c]) => (
              <div key={l as string} className="card-box" style={{ borderRadius: 10, padding: '11px 14px' }}>
                <div style={{ fontSize: 11.5, color: 'var(--mu2)' }}>{l}</div>
                <div className="display" style={{ fontSize: 26, color: c as string, marginTop: 2 }}>
                  {v}
                </div>
              </div>
            ))}
          </div>
          <div className="row" style={{ alignItems: 'baseline' }}>
            <h2 style={{ fontSize: 18 }}>Needs review</h2>
            <span style={{ fontSize: 12.5, color: 'var(--mu)' }}>Lines that didn't match a card. Pick a suggestion, or skip the line.</span>
          </div>
          <div className="lg" style={{ border: '1px solid var(--line2)', borderRadius: 12, background: 'var(--panel)', overflowY: 'auto', maxHeight: 360 }}>
            {(p?.review || []).map((r) => {
              const pick = picks[rkey(r)];
              return (
                <div key={rkey(r)} className="col" style={{ padding: '13px 16px', borderBottom: '1px solid var(--line)', gap: 8, background: pick ? 'rgba(108,194,138,.06)' : 'transparent' }}>
                  <div className="row">
                    <span className="mono" style={{ fontSize: 11, color: 'var(--mu3)', width: 52 }}>
                      line {r.line}
                    </span>
                    <span className="mono" style={{ fontSize: 13, color: pick === '__skip' ? 'var(--mu3)' : 'var(--danger-tx)', textDecoration: pick === '__skip' ? 'line-through' : 'none' }}>
                      {r.raw}
                    </span>
                    <span style={{ marginLeft: 'auto', fontSize: 12, color: pick === '__skip' ? 'var(--mu2)' : pick ? 'var(--ok)' : 'var(--danger-tx)' }}>
                      {pick === '__skip' ? 'Skipped' : pick ? '→ ' + r.suggestions.find((s) => s.key === pick)?.name : 'Unmatched'}
                    </span>
                  </div>
                  <div className="row" style={{ gap: 6, paddingLeft: 62, flexWrap: 'wrap' }}>
                    {r.suggestions.length > 0 && <span style={{ fontSize: 11.5, color: 'var(--mu2)' }}>Did you mean</span>}
                    {r.suggestions.map((s) => (
                      <button key={s.key} className={'mbtn' + (pick === s.key ? ' on' : '')} onClick={() => setPicks({ ...picks, [rkey(r)]: s.key })}>
                        {s.name}
                      </button>
                    ))}
                    <button className="mbtn" style={{ marginLeft: 6 }} onClick={() => setPicks({ ...picks, [rkey(r)]: pick === '__skip' ? '' : '__skip' })}>
                      {pick === '__skip' ? 'Undo skip' : 'Skip line'}
                    </button>
                  </div>
                </div>
              );
            })}
            {p && !p.review.length && <div style={{ padding: 26, textAlign: 'center', fontSize: 13, color: 'var(--ok)' }}>Every line matched a card.</div>}
            {!p && <div style={{ padding: 26, textAlign: 'center', fontSize: 13, color: 'var(--mu2)' }}>Paste a list to begin.</div>}
          </div>
          <div className="row" style={{ alignItems: 'baseline' }}>
            <h2 style={{ fontSize: 18 }}>Matched</h2>
            <span style={{ fontSize: 12.5, color: 'var(--mu)' }}>Grouped by the section headers in your list</span>
          </div>
          <div className="lg" style={{ display: 'flex', gap: 14, alignItems: 'flex-start', flexWrap: 'wrap', overflowY: 'auto', maxHeight: 320 }}>
            {SEC_ORDER.filter((s) => rows.some((r) => r.section === s)).map((s) => {
              const list = rows.filter((r) => r.section === s);
              return (
                <div key={s} className="card-box" style={{ flex: '1 1 200px', minWidth: 0, borderRadius: 10, padding: '10px 12px' }}>
                  <div style={{ fontSize: 12.5, fontWeight: 600, marginBottom: 6 }}>
                    {SECTION_NAMES[s]}{' '}
                    <span className="mono" style={{ color: 'var(--mu2)', fontWeight: 400 }}>
                      {list.reduce((a, r) => a + r.qty, 0)}
                    </span>
                  </div>
                  {list.map((m, i) => (
                    <div key={i} className="row" style={{ fontSize: 12, color: 'var(--tx3)', gap: 6, padding: '2px 0' }}>
                      <span className="mono" style={{ color: 'var(--mu2)', width: 20 }}>
                        {m.qty}
                      </span>
                      <span className="grow" style={{ whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>
                        {m.name}
                      </span>
                      <span className="mono" style={{ fontSize: 11, color: 'var(--mu3)' }}>
                        {m.set?.toUpperCase()}
                      </span>
                      {m.foil && <span style={{ fontSize: 10, color: 'var(--acc-tx)' }}>foil</span>}
                    </div>
                  ))}
                </div>
              );
            })}
          </div>
          <div className="row" style={{ paddingTop: 4 }}>
            <span className="grow" style={{ fontSize: 12.5, color: 'var(--mu)' }}>
              {open ? `${open} line${open === 1 ? '' : 's'} still need a decision — or import now and they are left out.` : cards ? 'Ready. Card images download in the background after saving.' : ''}
            </span>
            <Link to={targetId ? `/decks/${targetId}` : '/decks'} className="tb">
              Cancel
            </Link>
            <button className="tb pri" onClick={commit} disabled={busy || !cards}>
              {targetId ? `Replace with ${cards} cards` : `Import ${cards} cards`}
            </button>
          </div>
        </section>
      </div>
    </div>
  );
}
