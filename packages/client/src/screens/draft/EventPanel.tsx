import { useQuery } from '@tanstack/react-query';
import { useMemo, useState } from 'react';
import { Link } from 'react-router-dom';
import { rulesMode, type GameType, type LobbyView } from '@aetherhub/shared';
import { api } from '../../lib/api';
import { BASE } from '../../lib/base';
import { errMsg, useApp } from '../../lib/store';

export interface DraftSet {
  code: string;
  name: string;
  released: string;
  cards: number;
  icon: boolean;
  booster: 'play' | 'draft';
}

/** A set's icon (from Scryfall, cached by the server), or its code when there is none. */
export function SetIcon({ set, size = 34 }: { set: Pick<DraftSet, 'code' | 'icon'>; size?: number }) {
  const [failed, setFailed] = useState(false);
  if (!set.icon || failed)
    return (
      <span className="mono" style={{ width: size, height: size, display: 'inline-flex', alignItems: 'center', justifyContent: 'center', fontSize: Math.max(9, size * 0.3), fontWeight: 700, color: 'var(--tx3)', border: '1px solid var(--line4)', borderRadius: 6 }}>
        {set.code.toUpperCase()}
      </span>
    );
  return <img src={`${BASE}api/draft/sets/${set.code}/icon.svg`} alt="" width={size} height={size} onError={() => setFailed(true)} style={{ objectFit: 'contain', filter: 'invert(.88)' }} />;
}

/**
 * The sets of a draft or sealed event as a grid of set icons with their names. Draft: one set per pack round (up to
 * three; one set alone fills all three packs). Sealed: the six packs cycle through the chosen sets.
 */
export function SetPicker({ type, sets, disabled, onChange }: { type: GameType; sets: string[]; disabled: boolean; onChange: (sets: string[]) => void }) {
  const all = useQuery({ queryKey: ['draft-sets'], queryFn: () => api.get<DraftSet[]>('api/draft/sets'), staleTime: 300_000 });
  const [q, setQ] = useState('');
  const max = type === 'draft' ? 3 : 6;
  const byCode = useMemo(() => new Map((all.data ?? []).map((s) => [s.code, s])), [all.data]);
  const shown = (all.data ?? []).filter((s) => !q || (s.name + ' ' + s.code).toLowerCase().includes(q.toLowerCase()));
  const toggle = (code: string) => {
    if (disabled) return;
    if (sets.includes(code)) onChange(sets.filter((x) => x !== code));
    else if (sets.length < max) onChange([...sets, code]);
  };
  return (
    <div className="field">
      {type === 'draft' ? 'Sets (one per pack, up to three — one set fills all three packs)' : 'Sets for the six packs (they take turns)'}
      {sets.length > 0 && (
        <div className="row" style={{ gap: 6, flexWrap: 'wrap' }}>
          {sets.map((c, i) => {
            const s = byCode.get(c);
            return (
              <span key={c} className="row" style={{ gap: 6, padding: '3px 8px 3px 4px', borderRadius: 8, background: 'var(--acc-bg)', border: '1px solid var(--acc-bd)', fontSize: 12 }}>
                <SetIcon set={s ?? { code: c, icon: false }} size={20} />
                {type === 'draft' && sets.length > 1 ? `Pack ${i + 1}: ` : ''}
                {s?.name ?? c.toUpperCase()}
                {!disabled && (
                  <button aria-label={`Remove ${s?.name ?? c}`} onClick={() => toggle(c)} style={{ border: 0, background: 'none', color: 'var(--mu)', cursor: 'pointer', padding: 0, fontSize: 14 }}>
                    ×
                  </button>
                )}
              </span>
            );
          })}
        </div>
      )}
      {!disabled && (
        <>
          <input className="inp" placeholder="Search sets…" aria-label="Search sets" value={q} onChange={(e) => setQ(e.target.value)} />
          <div className="lg" style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(86px, 1fr))', gap: 6, maxHeight: 300, overflowY: 'auto', padding: 2 }}>
            {all.isLoading && <div style={{ color: 'var(--mu)', fontSize: 12 }}>Loading sets…</div>}
            {!all.isLoading && !shown.length && <div style={{ color: 'var(--mu)', fontSize: 12 }}>No sets{q ? ' match' : ' in the card data yet'}.</div>}
            {shown.map((s) => {
              const on = sets.includes(s.code);
              return (
                <button
                  key={s.code}
                  title={`${s.name} (${s.code.toUpperCase()}) · ${s.released.slice(0, 4)} · ${s.booster === 'play' ? 'Play Boosters' : 'Draft Boosters'}`}
                  aria-pressed={on}
                  onClick={() => toggle(s.code)}
                  style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 5, padding: '9px 4px 7px', borderRadius: 9, cursor: 'pointer', border: `1px solid ${on ? 'var(--acc)' : 'var(--line2)'}`, background: on ? 'var(--acc-bg)' : 'var(--surface)', color: 'var(--tx2)', opacity: !on && sets.length >= max ? 0.5 : 1 }}
                >
                  <SetIcon set={s} size={30} />
                  <span style={{ fontSize: 11, lineHeight: 1.25, textAlign: 'center', overflow: 'hidden', display: '-webkit-box', WebkitLineClamp: 2, WebkitBoxOrient: 'vertical' }}>{s.name}</span>
                  <span className="mono" style={{ fontSize: 9.5, color: 'var(--mu3)' }}>{s.released.slice(0, 4)}</span>
                </button>
              );
            })}
          </div>
        </>
      )}
    </div>
  );
}

/**
 * A draft / sealed event lobby after the start: the draft's progress, then everyone's decks and the event's games. The
 * players start the games they want (who plays who, 2-8 players, sandbox or rules enforced with the AI drafters).
 */
export function EventPanel({ lobby, isHost, onError }: { lobby: LobbyView; isHost: boolean; onError: (m: string) => void }) {
  const me = useApp((s) => s.me)!;
  const [chosen, setChosen] = useState<number[]>([]);
  const type = rulesMode(lobby.settings);
  const draftUrl = `/draft?id=${lobby.draftId}`;
  const L = `api/lobbies/${lobby.id}`;
  const state = useQuery({
    queryKey: ['draft', lobby.draftId, lobby.draftRev],
    queryFn: () => api.get<{ seats: { name: string; human: boolean; picked: boolean; pool: number; built: boolean }[]; round: number; pickNo: number; mode: string }>(`api/draft/games/${lobby.draftId}`),
    enabled: !!lobby.draftId && lobby.members.some((m) => m.userId === me.id) && lobby.slots.some((s) => s.kind === 'human' && s.userId === me.id),
  });
  const decks = lobby.eventDecks;
  const mine = decks.filter((d) => d.userId === me.id);
  const start = async () => {
    try {
      await api.post(`${L}/event-game`, { seats: chosen, type });
      setChosen([]);
    } catch (e) {
      onError(errMsg(e));
    }
  };

  if (lobby.phase === 'drafting')
    return (
      <div className="col" style={{ gap: 14 }}>
        <div className="row">
          <h2 className="grow" style={{ fontSize: 18 }}>
            Drafting{state.data ? ` · pack ${state.data.round}, pick ${state.data.pickNo}` : ''}
          </h2>
          <Link className="tb pri" to={draftUrl}>
            Go to the draft
          </Link>
        </div>
        <Pod seats={state.data?.seats ?? lobby.slots.flatMap((s) => (s.kind === 'open' ? [] : [{ name: s.name, human: s.kind === 'human', picked: false, pool: 0, built: false }]))} what="picked" />
      </div>
    );

  return (
    <div className="col" style={{ gap: 16 }}>
      <div className="row" style={{ flexWrap: 'wrap', gap: 10 }}>
        <h2 className="grow" style={{ fontSize: 18 }}>
          {lobby.phase === 'building' ? 'Building decks' : 'Decks'}
        </h2>
        {mine.length > 0 && (
          <Link className={'tb' + (mine.some((d) => d.deckId == null) ? ' pri' : '')} to={draftUrl}>
            {mine.some((d) => d.deckId == null) ? 'Build your deck' : 'Edit your deck'}
          </Link>
        )}
      </div>
      <div className="col" style={{ gap: 6 }}>
        {decks.map((d, i) => {
          const pickable = lobby.phase === 'event' && (d.userId === null ? !!d.deckName : d.deckId != null);
          const on = chosen.includes(i);
          return (
            <label key={i} className="row" style={{ gap: 12, padding: '10px 14px', borderRadius: 10, border: `1px solid ${on ? 'var(--acc)' : 'var(--line2)'}`, background: on ? 'var(--acc-bg)' : 'var(--surface)', cursor: pickable ? 'pointer' : 'default' }}>
              {lobby.phase === 'event' && <input type="checkbox" disabled={!pickable} checked={on} onChange={() => setChosen(on ? chosen.filter((x) => x !== i) : [...chosen, i].slice(-8))} aria-label={`Play ${d.name}`} />}
              <span className="mono" style={{ fontSize: 12, color: 'var(--mu2)', width: 14 }}>
                {i + 1}
              </span>
              <div className="grow">
                <div style={{ fontSize: 13.5, fontWeight: 600 }}>
                  {d.name}
                  {d.userId === me.id ? ' (you)' : ''}
                  {d.userId === null ? <span style={{ fontWeight: 400, color: 'var(--mu)', fontSize: 12 }}> · AI drafter</span> : null}
                </div>
                <div style={{ fontSize: 12, color: d.deckName ? 'var(--mu)' : '#e0a84f' }}>{d.deckName ?? 'Still building…'}</div>
              </div>
              {d.userId === me.id && d.deckId != null && (
                <Link className="mbtn" to={`/decks/${d.deckId}`} onClick={(e) => e.stopPropagation()}>
                  View
                </Link>
              )}
            </label>
          );
        })}
      </div>
      {lobby.phase === 'event' && (
        <div className="col" style={{ gap: 10, padding: '14px 16px', borderRadius: 12, background: 'var(--surface)', border: '1px solid var(--line2)' }}>
          <div style={{ fontSize: 13.5, fontWeight: 600 }}>New game</div>
          <div style={{ fontSize: 12.5, color: 'var(--mu)' }}>Tick who plays (2 to 8 drafters). Everyone else can watch; play as many games as you like and keep score yourselves.</div>
          <div className="row" style={{ gap: 10, flexWrap: 'wrap' }}>
            <span style={{ fontSize: 12.5, color: 'var(--mu)' }}>{type === 'rules' ? 'Rules enforced' : 'Sandbox'} game</span>
            <div className="grow" />
            <button className={'tb pri' + (chosen.length < 2 ? ' dis' : '')} disabled={chosen.length < 2 || (!isHost && !chosen.some((i) => decks[i]?.userId === me.id))} onClick={start}>
              Start game{chosen.length >= 2 ? ` (${chosen.map((i) => decks[i]?.name).join(' vs ')})` : ''}
            </button>
          </div>
        </div>
      )}
      {lobby.eventGames.length > 0 && (
        <div className="col" style={{ gap: 6 }}>
          <div style={{ fontSize: 12, textTransform: 'uppercase', letterSpacing: 0.7, color: 'var(--mu3)' }}>Games running</div>
          {lobby.eventGames.map((g) => {
            const playing = decks.some((d) => d.userId === me.id && g.names.includes(d.name));
            return (
              <div key={g.id} className="row" style={{ gap: 10, padding: '8px 12px', borderRadius: 9, background: 'var(--surface)', border: '1px solid var(--line2)', fontSize: 13 }}>
                <span className="grow">
                  {g.names.join(' vs ')} <span style={{ color: 'var(--mu)' }}>· {g.kind === 'rules' ? 'rules enforced' : 'sandbox'}</span>
                </span>
                {g.kind === 'sandbox' ? (
                  <Link className={'tb sm' + (playing ? ' pri' : '')} to={`/table/${g.id}`}>
                    {playing ? 'Rejoin' : 'Watch'}
                  </Link>
                ) : playing ? (
                  <Link className="tb sm pri" to={`/rules?id=${g.id}`}>
                    Rejoin
                  </Link>
                ) : null}
              </div>
            );
          })}
        </div>
      )}
      {state.data && lobby.phase === 'building' && <Pod seats={state.data.seats} what="built" />}
    </div>
  );
}

/** The pod in seat order (packs go left in packs 1 and 3, right in pack 2). */
function Pod({ seats, what }: { seats: { name: string; human: boolean; picked: boolean; pool: number; built: boolean }[]; what: 'picked' | 'built' }) {
  return (
    <div className="row" style={{ gap: 8, flexWrap: 'wrap' }}>
      {seats.map((s, i) => {
        const done = what === 'picked' ? s.picked : s.built;
        return (
          <div key={i} className="col" style={{ gap: 2, minWidth: 110, padding: '8px 12px', borderRadius: 9, border: `1px solid ${done ? 'var(--ok, #6fbf73)' : 'var(--line2)'}`, background: 'var(--surface)' }}>
            <span style={{ fontSize: 13, fontWeight: 600 }}>{s.name}</span>
            <span style={{ fontSize: 11.5, color: 'var(--mu)' }}>
              {s.human ? '' : 'AI · '}
              {s.pool} cards{what === 'picked' ? (done ? ' · picked' : ' · picking…') : done ? ' · deck built' : ' · building…'}
            </span>
          </div>
        );
      })}
    </div>
  );
}
