import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useState } from 'react';
import { Link, useNavigate, useSearchParams } from 'react-router-dom';
import { formatName } from '@aetherhub/shared';
import { api, download } from '../lib/api';
import { SoloSavesList } from './soloSaves';
import { errMsg, useApp } from '../lib/store';
import { askConfirm } from '../ui/dialogs';
import { Chevron, Avatar, Confirm, shortDate } from '../ui/ui';
import { duration, type MatchSummary } from './MainMenu';

interface Suspended {
  id: string;
  name: string;
  suspendedBy: string | null;
  suspendedAt: number | null;
  turn: number;
  format: string;
  bo: number;
  game: number;
  wins: number[];
  players: { userId: number; name: string; color: string }[];
}

function fmtT(ms: number) {
  const s = Math.floor(ms / 1000);
  return `${Math.floor(s / 3600)}:${String(Math.floor((s % 3600) / 60)).padStart(2, '0')}`;
}

export default function Games() {
  const [params, setParams] = useSearchParams();
  const tab = params.get('tab') === 'load' ? 'load' : 'history';
  const sel = params.get('match') ? Number(params.get('match')) : null;
  const me = useApp((s) => s.me)!;
  const toast = useApp((s) => s.toast);
  const qc = useQueryClient();
  const nav = useNavigate();
  const matches = useQuery({ queryKey: ['matches', 'all'], queryFn: () => api.get<MatchSummary[]>('api/matches?limit=200') });
  const suspended = useQuery({ queryKey: ['suspended'], queryFn: () => api.get<Suspended[]>('api/games/suspended') });
  const current = (matches.data || []).find((m) => m.id === sel) || matches.data?.[0];
  const log = useQuery({ queryKey: ['match-log', current?.id], queryFn: () => api.get<{ t: number; who: string; color: string; kind: string; text: string }[]>(`api/matches/${current!.id}/log`), enabled: !!current });
  const [filter, setFilter] = useState<'all' | 'act' | 'chat'>('all');
  const [del, setDel] = useState<Suspended | null>(null);

  return (
    <div className="screen">
      <header className="page-header" style={{ gap: 24 }}>
        <Link to="/" className="tb" aria-label="Back to main menu">
          <Chevron />Menu
        </Link>
        <h1>{tab === 'load' ? 'Load game' : 'Match history'}</h1>
      </header>
      {tab === 'load' ? (
        <div className="col lg" style={{ padding: 28, gap: 14, maxWidth: 980, overflowY: 'auto' }}>
          <h2 style={{ fontSize: 18, margin: 0 }}>Multiplayer games</h2>
          <div style={{ fontSize: 13, color: 'var(--mu)' }}>Sandbox games suspended by their host. Loading one reopens it; players rejoin their own seats by account.</div>
          {(suspended.data || []).map((g) => {
            const canLoad = me.isAdmin || g.players.some((p) => p.userId === me.id);
            return (
              <div key={g.id} className="row" style={{ gap: 18, padding: '18px 20px', borderRadius: 12, background: 'var(--surface)', border: '1px solid var(--line2)', flexWrap: 'wrap' }}>
                <div style={{ width: 48, height: 48, borderRadius: 10, background: 'var(--acc-bg)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
                  <svg style={{ stroke: 'var(--acc)' }} width="22" height="22" viewBox="0 0 24 24" fill="none" strokeWidth="1.8" aria-hidden="true">
                    <rect x="6" y="5" width="4" height="14" rx="1" />
                    <rect x="14" y="5" width="4" height="14" rx="1" />
                  </svg>
                </div>
                <div className="grow">
                  <div className="display" style={{ fontSize: 19 }}>
                    {g.name}
                  </div>
                  <div style={{ fontSize: 12.5, color: 'var(--mu)', marginTop: 3 }}>
                    Suspended {g.suspendedAt ? new Date(g.suspendedAt).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' }) : ''} by {g.suspendedBy} · {formatName(g.format)} · turn {g.turn}
                    {g.bo === 3 ? ` · game ${g.game} of 3 · ${g.players.map((p, i) => `${p.name} ${g.wins[i] || 0}`).join('–')}` : ''} · autosaved
                  </div>
                  <div className="row" style={{ gap: 6, marginTop: 8, flexWrap: 'wrap' }}>
                    {g.players.map((p) => (
                      <span key={p.userId} className="row" style={{ gap: 5, fontSize: 12, padding: '2px 8px', borderRadius: 10, background: 'var(--surface3)' }}>
                        <span className="dot" style={{ background: p.color }} />
                        {p.name}
                        {p.userId === me.id ? ' (you)' : ''}
                      </span>
                    ))}
                  </div>
                </div>
                {canLoad && (
                  <button className="tb" onClick={() => setDel(g)}>
                    Delete
                  </button>
                )}
                <button
                  className="tb pri"
                  disabled={!canLoad}
                  onClick={async () => {
                    try {
                      await api.post(`api/games/${g.id}/load`);
                      qc.invalidateQueries({ queryKey: ['suspended'] });
                      nav(`/table/${g.id}`);
                    } catch (e) {
                      toast(errMsg(e));
                    }
                  }}
                >
                  Load game
                </button>
              </div>
            );
          })}
          {suspended.data && !suspended.data.length && <div style={{ padding: '20px 0', color: 'var(--mu2)' }}>No suspended games.</div>}
          <RulesGames />
          <h2 style={{ fontSize: 18, margin: '14px 0 0' }}>Saved solo games (this browser)</h2>
          <div style={{ fontSize: 13, color: 'var(--mu)' }}>Solo games saved in your browser. New solo games are played from New game: take several seats yourself.</div>
          <SoloSavesList empty={<div style={{ padding: '20px 0', color: 'var(--mu2)' }}>No saved solo games.</div>} />
        </div>
      ) : (
        <div style={{ flexGrow: 1, display: 'flex', minHeight: 0, flexWrap: 'wrap' }}>
          <div className="lg" style={{ width: 440, maxWidth: '100%', flexShrink: 0, borderRight: '1px solid var(--line)', overflowY: 'auto' }}>
            {(matches.data || []).map((m) => {
              const on = m.id === current?.id;
              const w = m.players.find((p) => p.name.replace('(deleted) ', '') === m.winner);
              return (
                <div key={m.id} onClick={() => setParams({ tab: 'history', match: String(m.id) })} style={{ padding: '15px 22px', borderBottom: '1px solid var(--line)', background: on ? 'var(--surface)' : 'transparent', borderLeft: `3px solid ${on ? 'var(--acc)' : 'transparent'}`, cursor: 'pointer' }}>
                  <div className="row" style={{ alignItems: 'baseline', gap: 8 }}>
                    <span className="grow" style={{ fontSize: 14, fontWeight: 600 }}>
                      {m.name}
                    </span>
                    <span className="mono" style={{ fontSize: 11.5, color: 'var(--mu2)' }}>
                      {shortDate(m.endedAt)}
                    </span>
                  </div>
                  <div style={{ fontSize: 12.5, color: 'var(--mu)', marginTop: 4 }}>
                    {formatName(m.format)} · {m.bo === 3 ? 'Best of 3 · ' : ''}
                    {m.players.length} players · {duration(m.endedAt - m.startedAt)}
                    {m.meta.planechase ? ' · Planechase on' : ''}
                  </div>
                  <div style={{ fontSize: 12.5, marginTop: 4, color: w?.color || 'var(--tx3)' }}>Winner: {m.winner || '—'}</div>
                </div>
              );
            })}
            {matches.data && !matches.data.length && <div style={{ padding: 30, color: 'var(--mu2)' }}>No matches yet.</div>}
          </div>
          {current && (
            <div className="lg col" style={{ flex: '1 1 400px', overflowY: 'auto', padding: '24px 30px', gap: 18, minWidth: 0 }}>
              <div className="row" style={{ alignItems: 'flex-start', gap: 14 }}>
                <div className="grow">
                  <h1 style={{ fontSize: 26 }}>
                    {current.name} · {shortDate(current.endedAt)}
                  </h1>
                  <div style={{ fontSize: 13, color: 'var(--mu)', marginTop: 4 }}>
                    {formatName(current.format)} · {current.players.length} players · {duration(current.endedAt - current.startedAt)} · {current.turns} turns
                  </div>
                </div>
                <button className="tb" onClick={() => download(`api/matches/${current.id}/log.txt`, `match-${current.id}.txt`).catch((e) => toast(errMsg(e)))}>
                  Export log (.txt)
                </button>
              </div>
              <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(260px, 1fr))', gap: 10 }}>
                {current.players.map((p) => {
                  const win = /^Winner|^Won/.test(p.result);
                  return (
                    <div key={p.seat} className="row" style={{ gap: 12, padding: '12px 14px', borderRadius: 11, background: 'var(--surface)', border: `1px solid ${win ? '#2f5a3c' : 'var(--line2)'}` }}>
                      <Avatar name={p.name} color={p.userId == null ? '#6f737c' : p.color} />
                      <div className="grow">
                        <div style={{ fontSize: 14, fontWeight: 600 }}>{p.name}</div>
                        <div style={{ fontSize: 12.5, color: 'var(--mu)', marginTop: 2 }}>{p.deck}</div>
                      </div>
                      <span style={{ fontSize: 12.5, fontWeight: 600, color: win ? 'var(--ok)' : 'var(--mu)' }}>{p.result}</span>
                    </div>
                  );
                })}
              </div>
              <div className="row">
                <h2 className="grow" style={{ fontSize: 18 }}>
                  Full log <span style={{ fontSize: 12.5, color: 'var(--mu2)', fontFamily: 'var(--font)', fontWeight: 400 }}>· unredacted, every card named</span>
                </h2>
                {(
                  [
                    ['all', 'All'],
                    ['act', 'Players'],
                    ['chat', 'Chat'],
                  ] as const
                ).map(([k, l]) => (
                  <button key={k} className={'mbtn' + (filter === k ? ' on' : '')} onClick={() => setFilter(k)}>
                    {l}
                  </button>
                ))}
              </div>
              <div className="mono" style={{ background: 'var(--panel)', border: '1px solid var(--line2)', borderRadius: 12, padding: '12px 16px', fontSize: 12.5, lineHeight: 1.75 }}>
                {(log.data || [])
                  .filter((l) => filter === 'all' || (filter === 'chat' ? l.kind === 'chat' : l.kind !== 'chat' && l.who !== 'Server'))
                  .map((l, i) => (
                    <div key={i} className="row" style={{ gap: 12, alignItems: 'flex-start' }}>
                      <span style={{ color: 'var(--mu4)', width: 44, flexShrink: 0 }}>{fmtT(l.t)}</span>
                      <span style={{ color: l.color, width: 70, flexShrink: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{l.who}</span>
                      <span style={{ color: l.who === 'Server' ? 'var(--mu)' : 'var(--tx2)' }}>{l.kind === 'chat' ? `“${l.text}”` : l.text}</span>
                    </div>
                  ))}
              </div>
            </div>
          )}
        </div>
      )}
      {del && (
        <Confirm
          title="Delete suspended game?"
          text={`“${del.name}” will be removed. This cannot be undone.`}
          confirm="Delete"
          danger
          onNo={() => setDel(null)}
          onYes={async () => {
            try {
              await api.del(`api/games/${del.id}`);
              qc.invalidateQueries({ queryKey: ['suspended'] });
            } catch (e) {
              toast(errMsg(e));
            }
            setDel(null);
          }}
        />
      )}
    </div>
  );
}

const ROW: React.CSSProperties = { gap: 14, padding: '14px 18px', borderRadius: 12, background: 'var(--surface)', border: '1px solid var(--line2)', flexWrap: 'wrap' };

/** Rules enforced games: the player's running ones, saved ones, and two-player saves waiting for them to rejoin. */
function RulesGames() {
  const nav = useNavigate();
  const toast = useApp((s) => s.toast);
  const running = useQuery({ queryKey: ['rules-mine'], queryFn: () => api.get<{ id: string; opponents: string[]; turn: number; yourMove: boolean; versus: 'ai' | 'pvp'; status: 'waiting' | 'running' }[]>('api/rules/mine'), refetchInterval: 5000 });
  const saves = useQuery({ queryKey: ['rules-saves'], queryFn: () => api.get<{ id: number; name: string; opponent: string; turn: number | null; answers: number }[]>('api/rules/saves') });
  const waiting = useQuery({ queryKey: ['rules-open'], queryFn: () => api.get<{ id: string; host: string; resume?: boolean }[]>('api/rules/open'), refetchInterval: 5000 });
  const rejoinable = (waiting.data ?? []).filter((g) => g.resume);
  const go = (id: string) => nav(`/rules?id=${id}`);
  const tryIt = async (f: () => Promise<void>) => {
    try {
      await f();
    } catch (e) {
      toast(errMsg(e));
    }
  };
  return (
    <>
      <h2 style={{ fontSize: 18, margin: '14px 0 0' }}>Rules enforced games</h2>
      <div style={{ fontSize: 13, color: 'var(--mu)' }}>A running game is removed and autosaved after 30 minutes unattended.</div>
      {(running.data ?? []).map((g) => (
        <div key={g.id} className="row" style={ROW}>
          <div className="grow">
            <div className="display" style={{ fontSize: 17 }}>vs {g.opponents.join(', ') || 'yourself'}</div>
            <div style={{ fontSize: 12.5, color: 'var(--mu)' }}>
              Running · {g.status === 'waiting' ? 'waiting for players' : `turn ${g.turn}`}
              {g.yourMove ? ' · your move' : ''}
              {g.versus === 'pvp' ? ' · other players' : ''}
            </div>
          </div>
          <button
            className="tb"
            onClick={() =>
              tryIt(async () => {
                const ok = await askConfirm(g.versus === 'pvp' ? { title: 'End this game for you?', text: 'You concede and leave; the other players play on.', confirm: 'Concede and leave', danger: true } : { title: 'End this game?', text: 'It is not saved.', confirm: 'End game', danger: true });
                if (!ok) return;
                await api.del(`api/rules/games/${g.id}`);
                void running.refetch();
              })
            }
          >
            End
          </button>
          <button className="tb pri" onClick={() => go(g.id)}>
            Rejoin
          </button>
        </div>
      ))}
      {rejoinable.map((g) => (
        <div key={g.id} className="row" style={ROW}>
          <div className="grow">
            <div className="display" style={{ fontSize: 17 }}>{g.host} resumed a saved game</div>
            <div style={{ fontSize: 12.5, color: 'var(--mu)' }}>Waiting for you to rejoin your seat</div>
          </div>
          <button
            className="tb pri"
            onClick={() =>
              tryIt(async () => {
                await api.post(`api/rules/games/${g.id}/join`, {});
                go(g.id);
              })
            }
          >
            Rejoin saved game
          </button>
        </div>
      ))}
      {(saves.data ?? []).map((x) => (
        <div key={x.id} className="row" style={ROW}>
          <div className="grow">
            <div className="display" style={{ fontSize: 17 }}>{x.name}</div>
            <div style={{ fontSize: 12.5, color: 'var(--mu)' }}>
              Saved · vs {x.opponent} · {x.answers} moves{x.turn ? ` · turn ${x.turn}` : ''}
            </div>
          </div>
          <button
            className="tb"
            onClick={() =>
              tryIt(async () => {
                await api.del(`api/rules/saves/${x.id}`);
                void saves.refetch();
              })
            }
          >
            Delete
          </button>
          <button
            className="tb pri"
            onClick={() =>
              tryIt(async () => {
                const r = await api.post<{ id: string }>(`api/rules/saves/${x.id}/resume`);
                go(r.id);
              })
            }
          >
            Resume
          </button>
        </div>
      ))}
      {!running.data?.length && !saves.data?.length && !rejoinable.length && <div style={{ padding: '12px 0', color: 'var(--mu2)' }}>No running or saved rules enforced games.</div>}
    </>
  );
}
