import { useQuery, useQueryClient } from '@tanstack/react-query';
import { Select } from '../../ui/Select';
import { useMemo, useRef, useState } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import type { CardDef, Deck, DeckSummary } from '@aetherhub/shared';
import { FORMATS, formatName, PLAYER_COLORS } from '@aetherhub/shared';
import { api } from '../../lib/api';
import { errMsg, useApp } from '../../lib/store';
import { Chevron, ManaSymbol, Avatar, CardImage, Modal, previewProps, timeAgo } from '../../ui/ui';
import { DeckArt } from '../MainMenu';

interface Folder {
  id: number;
  name: string;
}
interface CustomEntry {
  def: CardDef;
  creatorId: number | null;
  creator: string | null;
  decks: number;
}

export function ownerColor(name: string): string {
  let h = 0;
  for (const c of name) h = (h * 31 + c.charCodeAt(0)) >>> 0;
  return PLAYER_COLORS[h % PLAYER_COLORS.length];
}

/** The deck picker (lobby): the library without editing; each deck has Select deck, and opening one shows it read-only. */
export interface DeckLibraryPick {
  title: string;
  /** the lobby's format: decks of other formats are marked */
  format?: string;
  onSelect: (deck: DeckSummary) => void;
  onOpen: (deckId: number) => void;
  onClose: () => void;
}

export default function DeckList() {
  return <DeckLibrary />;
}

export function DeckLibrary({ pick }: { pick?: DeckLibraryPick }) {
  const me = useApp((s) => s.me)!;
  const toast = useApp((s) => s.toast);
  const qc = useQueryClient();
  const nav = useNavigate();
  const decks = useQuery({ queryKey: ['decks'], queryFn: () => api.get<DeckSummary[]>('api/decks') });
  const folders = useQuery({ queryKey: ['folders'], queryFn: () => api.get<Folder[]>('api/folders') });
  const customs = useQuery({ queryKey: ['custom-cards'], queryFn: () => api.get<CustomEntry[]>('api/custom-cards') });
  const [view, setView] = useState<string>('all');
  const [q, setQ] = useState('');
  const [fmt, setFmt] = useState('all');
  const [owner, setOwner] = useState('all');
  const [colors, setColors] = useState<string[]>([]);
  const [editCustom, setEditCustom] = useState<CardDef | 'new' | null>(null);
  const [newFolder, setNewFolder] = useState<string | null>(null);
  const all = decks.data || [];
  const owners = Array.from(new Set(all.map((d) => d.ownerName))).sort();

  const list = useMemo(() => {
    const ql = q.toLowerCase();
    return all
      .filter((d) => {
        if (view === 'pinned' && !d.pinned) return false;
        if (view === 'mine' && d.ownerId !== me.id) return false;
        if (view === 'shared' && d.ownerId === me.id) return false;
        if (view.startsWith('f:') && (d.ownerId !== me.id || d.folderId !== +view.slice(2))) return false;
        if (ql && !d.name.toLowerCase().includes(ql)) return false;
        if (fmt !== 'all' && d.format !== fmt) return false;
        if (owner !== 'all' && d.ownerName !== owner) return false;
        if (colors.length && !colors.every((c) => d.colors.includes(c))) return false;
        return true;
      })
      .sort((a, b) => Number(b.pinned) - Number(a.pinned) || b.updatedAt - a.updatedAt);
  }, [all, view, q, fmt, owner, colors, me.id]);

  const refresh = () => qc.invalidateQueries({ queryKey: ['decks'] });
  const count = (fn: (d: DeckSummary) => boolean) => all.filter(fn).length;
  const navItem = (key: string, label: string, icon: string, n: number, iconColor = 'var(--mu3)') => (
    <button key={key} className={'nav' + (view === key ? ' on' : '')} onClick={() => setView(key)}>
      <span style={{ width: 16, textAlign: 'center', color: iconColor }}>{icon}</span>
      <span className="grow">{label}</span>
      <span className="mono" style={{ fontSize: 11.5, color: 'var(--mu3)' }}>
        {n}
      </span>
    </button>
  );

  const create = async () => {
    try {
      const d = await api.post<Deck>('api/decks', { name: 'New deck', format: 'commander', folderId: view.startsWith('f:') ? +view.slice(2) : null });
      refresh();
      nav(`/decks/${d.id}`);
    } catch (e) {
      toast(errMsg(e));
    }
  };

  return (
    <div className="screen">
      <style>{`.dcard{transition:border-color .15s}.dcard:hover{border-color:var(--line5)!important}.dcard:hover .acts{opacity:1!important}`}</style>
      {pick ? (
        <header className="page-header">
          <button className="tb" aria-label="Back to the lobby" onClick={pick.onClose}>
            <Chevron />Lobby
          </button>
          <h1>{pick.title}</h1>
        </header>
      ) : (
        <header className="page-header">
          <Link to="/" className="tb" aria-label="Back to main menu">
            <Chevron />Menu
          </Link>
          <h1>Deckbuilder</h1>
          <div className="grow" />
          <Link to="/import" className="tb">
            Import deck
          </Link>
          <button className="tb pri" onClick={create}>
            + New deck
          </button>
        </header>
      )}
      <div style={{ flexGrow: 1, display: 'flex', minHeight: 0 }}>
        <aside className="hide-sm lg" style={{ width: 240, flexShrink: 0, borderRight: '1px solid var(--line)', padding: '18px 14px', display: 'flex', flexDirection: 'column', gap: 4, overflowY: 'auto' }}>
          <div className="nav-head">Library</div>
          {navItem('all', 'All decks', '▦', all.length)}
          {navItem('pinned', 'Pinned', '★', count((d) => d.pinned), 'var(--acc)')}
          {navItem('mine', 'My decks', '◧', count((d) => d.ownerId === me.id))}
          <div className="nav-head">Your folders</div>
          {(folders.data || []).map((f) => navItem('f:' + f.id, f.name, '▸', count((d) => d.ownerId === me.id && d.folderId === f.id)))}
          {pick ? null : newFolder != null ? (
            <input
              className="inp"
              autoFocus
              placeholder="Folder name"
              value={newFolder}
              onChange={(e) => setNewFolder(e.target.value)}
              onBlur={() => setNewFolder(null)}
              onKeyDown={async (e) => {
                if (e.key === 'Escape') setNewFolder(null);
                if (e.key === 'Enter' && newFolder.trim()) {
                  await api.post('api/folders', { name: newFolder.trim() });
                  setNewFolder(null);
                  qc.invalidateQueries({ queryKey: ['folders'] });
                }
              }}
            />
          ) : (
            <button className="nav" onClick={() => setNewFolder('')}>
              <span style={{ width: 16, textAlign: 'center' }}>+</span>New folder
            </button>
          )}
          <div className="nav-head">Others</div>
          {navItem('shared', 'Others’ decks', '◎', count((d) => d.ownerId !== me.id))}
          {!pick && (
            <>
              <div className="nav-head">Server</div>
              {navItem('custom', 'Custom cards', '✦', customs.data?.length ?? 0)}
            </>
          )}
        </aside>
        <main className="lg" style={{ flexGrow: 1, overflowY: 'auto', padding: '20px 28px 40px', minWidth: 0 }}>
          {view !== 'custom' ? (
            <>
              <div className="row" style={{ marginBottom: 18, flexWrap: 'wrap' }}>
                <Select
                  className="show-sm"
                  aria-label="Deck folder"
                  value={view}
                  onChange={setView}
                  options={[
                    { value: 'all', label: 'All decks' },
                    { value: 'pinned', label: 'Pinned' },
                    { value: 'mine', label: 'My decks' },
                    ...(folders.data || []).map((f) => ({ value: 'f:' + f.id, label: `Folder: ${f.name}` })),
                    { value: 'shared', label: 'Others’ decks' },
                    ...(pick ? [] : [{ value: 'custom', label: 'Custom cards' }]),
                  ]}
                />
                <input className="inp" aria-label="Search decks" placeholder="Search decks by name…" value={q} onChange={(e) => setQ(e.target.value)} style={{ width: 280 }} />
                <Select aria-label="Format filter" value={fmt} onChange={setFmt} options={[{ value: 'all', label: 'All formats' }, ...FORMATS.map((f) => ({ value: f.id as string, label: f.name }))]} />
                <Select aria-label="Owner filter" value={owner} onChange={setOwner} options={[{ value: 'all', label: 'Any owner' }, ...owners.map((o) => ({ value: o, label: o + (o === me.username ? ' (you)' : '') }))]} />
                <div className="row" style={{ gap: 5, marginLeft: 4 }} role="group" aria-label="Color filter">
                  {['W', 'U', 'B', 'R', 'G'].map((c) => {
                    const on = colors.includes(c);
                    return (
                      <button
                        key={c}
                        aria-label={`Filter ${c}`}
                        aria-pressed={on}
                        onClick={() => setColors(on ? colors.filter((x) => x !== c) : [...colors, c])}
                        style={{ width: 30, height: 30, borderRadius: 15, cursor: 'pointer', background: 'transparent', border: `2px solid ${on ? 'var(--acc)' : 'transparent'}`, opacity: on || !colors.length ? 1 : 0.45, padding: 0, display: 'inline-flex', alignItems: 'center', justifyContent: 'center' }}
                      >
                        <ManaSymbol c={c} size={24} title={null} />
                      </button>
                    );
                  })}
                </div>
                <div className="grow" />
                <span style={{ fontSize: 12.5, color: 'var(--mu2)' }}>
                  {list.length} deck{list.length === 1 ? '' : 's'}
                </span>
              </div>
              <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(220px, 1fr))', gap: 16 }}>
                {list.map((d) => {
                  const mine = d.ownerId === me.id;
                  return (
                    <div key={d.id} className="dcard" style={{ position: 'relative', borderRadius: 12, border: '1px solid var(--line2)', background: 'var(--surface)', overflow: 'hidden', display: 'flex', flexDirection: 'column' }}>
                      {pick ? (
                        <button aria-label={`Look at ${d.name}`} onClick={() => pick.onOpen(d.id)} style={{ position: 'absolute', inset: 0, zIndex: 1, border: 0, background: 'transparent', cursor: 'pointer' }} />
                      ) : (
                        <Link to={`/decks/${d.id}`} aria-label={`Open ${d.name}`} style={{ position: 'absolute', inset: 0, zIndex: 1 }} />
                      )}
                      <div style={{ position: 'relative' }}>
                        <DeckArt deck={d} height={128} />
                        <div style={{ position: 'absolute', left: 12, top: 10, fontSize: 11, color: '#f1ece4', background: 'rgba(16,17,20,.72)', borderRadius: 5, padding: '2px 7px' }}>{formatName(d.format)}</div>
                        <button
                          aria-label={d.pinned ? 'Unpin (pinned decks are offered first in the lobby)' : 'Pin as favourite'}
                          title={d.pinned ? 'Unpin (pinned decks are offered first in the lobby)' : 'Pin as favourite'}
                          onClick={async () => {
                            await api.post(`api/decks/${d.id}/pin`, { pinned: !d.pinned });
                            refresh();
                          }}
                          style={{ position: 'absolute', right: 10, top: 10, zIndex: 2, width: 30, height: 30, borderRadius: 8, border: 0, background: 'rgba(16,17,20,.7)', cursor: 'pointer', display: 'flex', alignItems: 'center', justifyContent: 'center' }}
                        >
                          <svg width="16" height="16" viewBox="0 0 24 24" style={{ fill: d.pinned ? 'var(--acc)' : 'none', stroke: d.pinned ? 'var(--acc)' : '#cfccc5' }} strokeWidth="1.8" aria-hidden="true">
                            <path d="M12 3l2.7 5.6 6.1.9-4.4 4.3 1 6.1L12 17l-5.4 2.9 1-6.1-4.4-4.3 6.1-.9z" />
                          </svg>
                        </button>
                      </div>
                      <div style={{ padding: '12px 14px', display: 'flex', flexDirection: 'column', gap: 4, flexGrow: 1 }}>
                        <div style={{ fontSize: 14.5, fontWeight: 600 }}>{d.name}</div>
                        <div style={{ fontSize: 12, color: 'var(--mu)' }}>
                          {d.cardCount} cards · edited {timeAgo(d.updatedAt)}
                        </div>
                        <div className="row" style={{ gap: 6, marginTop: 4 }}>
                          <Avatar name={d.ownerName} color={ownerColor(d.ownerName)} size={18} />
                          <span style={{ fontSize: 12, color: 'var(--tx3)' }}>{mine ? 'You' : d.ownerName}</span>
                          {d.warnings > 0 && (
                            <span title="Has legality warnings" style={{ marginLeft: 'auto', fontSize: 11, color: 'var(--danger-tx)' }}>
                              ! {d.warnings} warning{d.warnings === 1 ? '' : 's'}
                            </span>
                          )}
                        </div>
                      </div>
                      {pick ? (
                        <div className="acts row" style={{ position: 'relative', zIndex: 2, gap: 6, padding: '0 14px 12px' }}>
                          <button className="tb sm pri" onClick={() => pick.onSelect(d)}>
                            Select deck
                          </button>
                          <button className="mbtn" onClick={() => pick.onOpen(d.id)}>
                            View
                          </button>
                          {pick.format && d.format !== pick.format && <span style={{ marginLeft: 'auto', fontSize: 11, color: '#e0a84f' }}>{formatName(d.format)}</span>}
                        </div>
                      ) : (
                      <div className="acts row" style={{ position: 'relative', zIndex: 2, gap: 6, padding: '0 14px 12px', opacity: 0.75 }}>
                        <Link to={`/decks/${d.id}`} className="mbtn">
                          {mine || me.isAdmin ? 'Edit' : 'View'}
                        </Link>
                        <button
                          className="mbtn"
                          onClick={async () => {
                            await api.post(`api/decks/${d.id}/clone`);
                            refresh();
                            toast(`Cloned “${d.name}” into your decks`);
                          }}
                        >
                          Clone
                        </button>
                      </div>
                      )}
                    </div>
                  );
                })}
              </div>
              {!list.length && <div style={{ padding: 80, textAlign: 'center', color: 'var(--mu2)', fontSize: 14 }}>{all.length ? 'No decks match these filters.' : 'No decks yet — make one or import a list.'}</div>}
            </>
          ) : (
            <>
              <div className="row" style={{ marginBottom: 14 }}>
                <div className="grow" style={{ fontSize: 13, color: 'var(--mu)' }}>
                  Server-wide custom cards show up in search with a “custom” tag and are always legal. The creator and admins can edit them.
                </div>
                <button className="tb pri" onClick={() => setEditCustom('new')}>
                  + New custom card
                </button>
              </div>
              <div style={{ display: 'flex', flexWrap: 'wrap', gap: 18 }}>
                {(customs.data || []).map((c) => (
                  <div key={c.def.key} className="col" style={{ gap: 8, width: 180 }}>
                    <button onClick={() => (c.creatorId === me.id || me.isAdmin) && setEditCustom(c.def)} style={{ border: 0, padding: 0, background: 'none', cursor: 'pointer' }} {...previewProps(c.def)}>
                      <CardImage def={c.def} style={{ width: 180, height: 252, borderRadius: 10, boxShadow: '0 8px 20px rgba(0,0,0,.4)' }} />
                    </button>
                    <div style={{ fontSize: 12, color: 'var(--mu)' }}>
                      by {c.creator || 'unknown'} · {c.decks ? `in ${c.decks} deck${c.decks === 1 ? '' : 's'}` : 'unused'}
                    </div>
                  </div>
                ))}
              </div>
            </>
          )}
        </main>
      </div>
      {editCustom && <CustomCardEditor def={editCustom === 'new' ? null : editCustom} onClose={() => setEditCustom(null)} onSaved={() => qc.invalidateQueries({ queryKey: ['custom-cards'] })} />}
    </div>
  );
}

export function CustomCardEditor({ def, onClose, onSaved }: { def: CardDef | null; onClose: () => void; onSaved: () => void }) {
  const toast = useApp((s) => s.toast);
  const [f, setF] = useState({
    name: def?.name || '',
    manaCost: def?.manaCost || '',
    typeLine: def?.typeLine || 'Creature — ',
    power: def?.power || '',
    toughness: def?.toughness || '',
    oracleText: def?.oracleText || '',
    rarity: def?.rarity || 'rare',
    imageAsset: def?.img[0]?.startsWith('a/') ? parseInt(def.img[0].slice(2), 10) : (null as number | null),
  });
  const file = useRef<HTMLInputElement>(null);
  const preview: CardDef = {
    key: 'preview',
    name: f.name || 'Card name',
    layout: 'normal',
    manaCost: f.manaCost,
    cmc: 0,
    typeLine: f.typeLine,
    oracleText: f.oracleText,
    power: f.power || undefined,
    toughness: f.toughness || undefined,
    colors: ['W', 'U', 'B', 'R', 'G'].filter((c) => f.manaCost.includes(c)),
    colorIdentity: [],
    rarity: f.rarity,
    img: f.imageAsset ? ['a/' + f.imageAsset] : [],
    custom: true,
  };
  const save = async () => {
    const body = { ...f, power: f.power || null, toughness: f.toughness || null };
    try {
      if (def?.customId) await api.put(`api/custom-cards/${def.customId}`, body);
      else await api.post('api/custom-cards', body);
      onSaved();
      onClose();
    } catch (e) {
      toast(errMsg(e));
    }
  };
  const inp = (k: keyof typeof f, label: string, w?: number) => (
    <label className="field" style={{ width: w }}>
      {label}
      <input className="inp" value={String(f[k] ?? '')} onChange={(e) => setF({ ...f, [k]: e.target.value })} />
    </label>
  );
  return (
    <Modal
      title={def ? 'Edit custom card' : 'New custom card'}
      onClose={onClose}
      width={760}
      foot={
        <>
          {def?.customId && (
            <button
              className="tb danger"
              style={{ marginRight: 'auto' }}
              onClick={async () => {
                await api.del(`api/custom-cards/${def.customId}`);
                onSaved();
                onClose();
              }}
            >
              Delete
            </button>
          )}
          <button className="tb" onClick={onClose}>
            Cancel
          </button>
          <button className="tb pri" onClick={save} disabled={!f.name.trim()}>
            Save
          </button>
        </>
      }
    >
      <div className="row" style={{ padding: 20, gap: 22, alignItems: 'flex-start', flexWrap: 'wrap' }}>
        <CardImage def={preview} style={{ width: 220, height: 308, borderRadius: 12, flexShrink: 0 }} />
        <div className="col grow" style={{ gap: 12, minWidth: 260 }}>
          {inp('name', 'Name')}
          <div className="row">
            {inp('manaCost', 'Cost, e.g. {2}{G}{G}')}
            <label className="field">
              Rarity
              <Select aria-label="Rarity" value={f.rarity} onChange={(v) => setF({ ...f, rarity: v })} options={['common', 'uncommon', 'rare', 'mythic'].map((r) => ({ value: r, label: r[0].toUpperCase() + r.slice(1) }))} />
            </label>
          </div>
          {inp('typeLine', 'Type line')}
          <div className="row">
            {inp('power', 'Power', 90)}
            {inp('toughness', 'Toughness', 90)}
          </div>
          <label className="field">
            Rules text
            <textarea className="inp" rows={5} value={f.oracleText} onChange={(e) => setF({ ...f, oracleText: e.target.value })} />
          </label>
          <div className="row">
            <input
              ref={file}
              type="file"
              accept="image/*"
              hidden
              onChange={async (e) => {
                const x = e.target.files?.[0];
                if (!x) return;
                try {
                  setF({ ...f, imageAsset: await api.upload(x, 'card') });
                } catch (e2) {
                  toast(errMsg(e2));
                }
              }}
            />
            <button className="tb" onClick={() => file.current?.click()}>
              Upload image (≤10 MB)
            </button>
            {f.imageAsset && (
              <button className="tb" onClick={() => setF({ ...f, imageAsset: null })}>
                Use generated frame
              </button>
            )}
          </div>
        </div>
      </div>
    </Modal>
  );
}
