import { useQuery } from '@tanstack/react-query';
import { useState } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import type { DeckSummary, LobbyView } from '@aetherhub/shared';
import { formatName, gameKind, gameTypeLabel } from '@aetherhub/shared';
import { ActiveLobbies, useActiveRulesGames } from './ActiveLobbies';
import { useSoloSaves } from './soloSaves';
import { api } from '../lib/api';
import { imgUrl } from '../lib/base';
import { errMsg, useApp } from '../lib/store';
import { socket } from '../lib/ws';
import { Avatar, ColorDots, Logo, Modal, shortDate } from '../ui/ui';

export interface MatchSummary {
  id: number;
  name: string;
  format: string;
  bo: number;
  startedAt: number;
  endedAt: number;
  turns: number;
  winner: string | null;
  meta: { players: number; planechase?: boolean; games?: number };
  players: { seat: number; userId: number | null; name: string; color: string; deck: string; result: string }[];
}

export function duration(ms: number): string {
  const m = Math.round(ms / 60000);
  return m >= 60 ? `${Math.floor(m / 60)}h ${m % 60}m` : `${m}m`;
}

export function signOut() {
  api.post('api/auth/logout').finally(() => {
    socket.close();
    location.href = location.pathname.replace(/[^/]*$/, '');
  });
}

function MenuRow({ to, onClick, title, sub, icon }: { to?: string; onClick?: () => void; title: string; sub: string; icon: React.ReactNode }) {
  const style: React.CSSProperties = { display: 'flex', alignItems: 'center', gap: 18, padding: '16px 20px', borderRadius: 12, border: '1px solid var(--line2)', background: 'var(--surface)', color: 'var(--tx)', textAlign: 'left', cursor: 'pointer', font: 'inherit', width: '100%' };
  const Tag = (p: { children: React.ReactNode }) =>
    onClick ? (
      <button type="button" className="mrow" style={style} onClick={onClick}>
        {p.children}
      </button>
    ) : (
      <Link to={to!} className="mrow" style={style}>
        {p.children}
      </Link>
    );
  return (
    <Tag>
      <div style={{ width: 44, height: 44, borderRadius: 10, background: 'var(--acc-bg)', display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0 }}>
        <svg style={{ stroke: 'var(--acc)' }} width="22" height="22" viewBox="0 0 24 24" fill="none" strokeWidth="1.7" aria-hidden="true">
          {icon}
        </svg>
      </div>
      <div className="grow">
        <div className="display" style={{ fontSize: 21 }}>
          {title}
        </div>
        <div style={{ fontSize: 13, color: 'var(--mu)', marginTop: 2 }}>{sub}</div>
      </div>
    </Tag>
  );
}

export function MainMenu() {
  const me = useApp((s) => s.me)!;
  const status = useApp((s) => s.status);
  const toast = useApp((s) => s.toast);
  const wsConnected = useApp((s) => s.wsConnected);
  const nav = useNavigate();
  const [pwFor, setPwFor] = useState<string | null>(null);
  const [pw, setPw] = useState('');
  const [showActive, setShowActive] = useState(false);
  const decks = useQuery({ queryKey: ['decks'], queryFn: () => api.get<DeckSummary[]>('api/decks') });
  const suspended = useQuery({ queryKey: ['suspended'], queryFn: () => api.get<unknown[]>('api/games/suspended') });
  const soloSaves = useSoloSaves();
  const mine = (decks.data || []).filter((d) => d.ownerId === me.id);
  const pinned = mine.filter((d) => d.pinned);
  const allLobbies = status?.lobbies ?? [];
  const allGames = status?.games ?? [];
  // the banners up top are the player's own; Active lobbies lists everything on the server
  const lobbies = allLobbies.filter((l) => l.inLobby);
  const games = allGames.filter((g) => g.hasSeat);
  const activeRules = useActiveRulesGames();
  const activeCount = allLobbies.length + allGames.length + (activeRules.data?.length ?? 0);
  const rulesGames = useQuery({ queryKey: ['rules-mine'], queryFn: () => api.get<{ id: string; opponents: string[]; turn: number; yourMove: boolean; status: string }[]>('api/rules/mine'), refetchInterval: 10_000 });
  const rulesSaves = useQuery({ queryKey: ['rules-saves'], queryFn: () => api.get<unknown[]>('api/rules/saves') });

  const joinLobby = async (id: string, password: string | null) => {
    try {
      await api.post(`api/lobbies/${id}/join`, { password });
      setPwFor(null);
      setShowActive(false);
      nav(`/lobby/${id}`);
    } catch (e) {
      if (/password/i.test(errMsg(e)) && !pwFor) setPwFor(id);
      else toast(errMsg(e));
    }
  };
  const newGame = async () => {
    try {
      const r = await api.post<{ lobby: LobbyView }>('api/lobbies', { settings: {} });
      useApp.setState({ lobby: r.lobby });
      nav(`/lobby/${r.lobby.id}`);
    } catch (e) {
      toast(errMsg(e));
    }
  };
  const loadCount = (suspended.data?.length ?? 0) + (soloSaves.data?.length ?? 0) + (rulesSaves.data?.length ?? 0) + (rulesGames.data?.length ?? 0);

  return (
    <div className="screen" style={{ overflowY: 'auto' }}>
      <style>{`.mrow{transition:background .15s,border-color .15s}.mrow:hover{background:var(--surface2)!important;border-color:var(--line5)!important}`}</style>
      <header className="page-header" style={{ padding: '0 40px', gap: 16 }}>
        <Logo />
        <div className="display" style={{ fontSize: 19 }}>
          Aetherhub
        </div>
        <div style={{ fontSize: 12, color: 'var(--mu2)', marginLeft: -8 }} title="Aetherhub version">
          v{__APP_VERSION__}
        </div>
        {status?.serverName && status.serverName.trim().toLowerCase() !== 'aetherhub' && (
          <div className="hide-sm" style={{ fontSize: 12, color: 'var(--mu2)', padding: '3px 8px', border: '1px solid var(--line3)', borderRadius: 6 }}>
            {status.serverName}
          </div>
        )}
        <div className="grow" />
        <div className="row hide-sm" style={{ gap: 8, fontSize: 12, color: 'var(--mu)' }}>
          <span className="dot" style={{ background: wsConnected ? '#6cc28a' : '#e0a84f' }} />
          {!wsConnected
            ? 'Connecting…'
            : status?.importing
            ? 'Importing card data…'
            : status?.cardDataUpdatedAt
              ? `Server online · card data updated ${shortDate(status.cardDataUpdatedAt)}`
              : 'Server online · no card data yet'}
        </div>
        <div className="hide-sm" style={{ width: 1, height: 26, background: 'var(--line3)', margin: '0 8px' }} />
        <div className="row">
          <Avatar name={me.username} color="var(--acc)" />
          <div className="col" style={{ lineHeight: 1.2 }}>
            <span style={{ fontSize: 13, fontWeight: 600 }}>{me.username}</span>
            <span style={{ fontSize: 11, color: 'var(--mu2)' }}>{me.isAdmin ? 'Admin' : 'Player'}</span>
          </div>
          <button className="lnk" style={{ fontSize: 12, marginLeft: 10 }} onClick={signOut}>
            Sign out
          </button>
        </div>
      </header>

      {(lobbies.length > 0 || games.length > 0 || (rulesGames.data?.length ?? 0) > 0) && (
        <div className="col" style={{ margin: '20px 40px 0', gap: 10 }}>
          {lobbies.map((l) => (
            <div key={l.id} className="row" style={{ minHeight: 60, padding: '8px 12px 8px 20px', borderRadius: 12, background: 'var(--acc-bg)', border: '1px solid var(--acc-bd)', gap: 16, flexWrap: 'wrap' }}>
              <div style={{ width: 10, height: 10, borderRadius: 5, background: 'var(--acc)', boxShadow: '0 0 0 4px rgba(var(--acc-rgb),.18)' }} />
              <div className="grow">
                <div style={{ fontSize: 14, fontWeight: 600, color: 'var(--acc-tx)' }}>
                  {l.phase === 'setup' ? 'Lobby open' : l.phase === 'event' ? 'Event in progress' : l.type === 'draft' ? 'Drafting' : 'Building decks'} · {l.name}
                </div>
                <div style={{ fontSize: 12, color: 'var(--acc-mu)' }}>
                  {gameTypeLabel(l)} · {l.hostName} is hosting · {l.seats} of {l.maxPlayers} seats filled{gameKind(l.type) === 'constructed' ? ` · ${formatName(l.format)} · Best of ${l.bo}` : ''}
                  {l.hasPassword ? ' · password protected' : ''}
                </div>
              </div>
              {l.inLobby ? (
                <Link to={`/lobby/${l.id}`} className="tb pri">
                  Back to lobby
                </Link>
              ) : (
                <button className="tb pri" onClick={() => (l.hasPassword ? setPwFor(l.id) : joinLobby(l.id, null))}>
                  Join lobby
                </button>
              )}
            </div>
          ))}
          {games.map((game) => (
            <div key={game.id} className="row" style={{ minHeight: 60, padding: '8px 12px 8px 20px', borderRadius: 12, background: '#13191a', border: '1px solid #22514c', gap: 16, flexWrap: 'wrap' }}>
              <div style={{ width: 10, height: 10, borderRadius: 5, background: '#4fb3a9', boxShadow: '0 0 0 4px rgba(79,179,169,.18)' }} />
              <div className="grow">
                <div style={{ fontSize: 14, fontWeight: 600, color: '#bfe6e1' }}>
                  {game.ended ? 'Game over' : 'Sandbox game'} · {game.name} · Turn {game.turn}
                </div>
                <div style={{ fontSize: 12, color: '#93b8b3' }}>{game.ended ? 'Waiting for the host to start a rematch or close it' : `${game.hasSeat ? 'You have a seat in this game' : 'You are not seated'} · ${game.spectators ? 'spectators allowed' : 'no spectators'}`}</div>
              </div>
              {(game.spectators || me.isAdmin) && !game.hasSeat && (
                <Link to={`/table/${game.id}`} className="tb">
                  Spectate
                </Link>
              )}
              {game.hasSeat && (
                <Link to={`/table/${game.id}`} className="tb pri">
                  Rejoin seat
                </Link>
              )}
            </div>
          ))}
          {(rulesGames.data ?? []).map((g) => (
            <div key={g.id} className="row" style={{ minHeight: 60, padding: '8px 12px 8px 20px', borderRadius: 12, background: '#13191a', border: '1px solid #22514c', gap: 16, flexWrap: 'wrap' }}>
              <div style={{ width: 10, height: 10, borderRadius: 5, background: g.yourMove ? 'var(--acc)' : '#4fb3a9' }} />
              <div className="grow">
                <div style={{ fontSize: 14, fontWeight: 600, color: '#bfe6e1' }}>
                  Rules game · vs {g.opponents.join(', ') || 'yourself'}
                </div>
                <div style={{ fontSize: 12, color: '#93b8b3' }}>
                  {g.status === 'waiting' ? 'Waiting for players' : `Turn ${g.turn}`}
                  {g.yourMove ? ' · your move' : ''}
                </div>
              </div>
              <Link to={`/rules?id=${g.id}`} className="tb pri">
                Rejoin
              </Link>
            </div>
          ))}
        </div>
      )}

      <div style={{ flexGrow: 1, display: 'flex', justifyContent: 'center', padding: '36px 16px 40px' }}>
        <div style={{ width: 560, maxWidth: '100%', display: 'flex', flexDirection: 'column', gap: 22 }}>
          <div style={{ textAlign: 'center' }}>
            <div style={{ fontSize: 12, letterSpacing: 1.6, textTransform: 'uppercase', color: 'var(--mu)' }}>Welcome back, {me.username}</div>
            <h1 style={{ margin: '8px 0 0', fontSize: 56, lineHeight: 1, letterSpacing: -0.5 }}>What are we playing?</h1>
          </div>
          <nav style={{ display: 'flex', flexDirection: 'column', gap: 10 }} aria-label="Main menu">
            <MenuRow
              to="/decks"
              title="Deckbuilder"
              sub={`${mine.length} deck${mine.length === 1 ? '' : 's'} · ${pinned.length} pinned · import from Moxfield, Arena, MTGO`}
              icon={
                <>
                  <rect x="7" y="3" width="12" height="16" rx="2" />
                  <path d="M5 6v13a2 2 0 002 2h9" />
                </>
              }
            />
            <MenuRow
              onClick={newGame}
              title="New game"
              sub="Open a lobby for 1–8 players · constructed, draft or sealed"
              icon={
                <>
                  <circle cx="12" cy="12" r="9" />
                  <path d="M12 8v8M8 12h8" />
                </>
              }
            />
            <MenuRow
              onClick={() => setShowActive(true)}
              title="Active lobbies"
              sub={
                activeCount
                  ? `${allLobbies.length} lobb${allLobbies.length === 1 ? 'y' : 'ies'} · ${allGames.length + (activeRules.data?.length ?? 0)} game${allGames.length + (activeRules.data?.length ?? 0) === 1 ? '' : 's'} in progress · join a seat or watch`
                  : 'Nothing running right now · join or watch lobbies and games others started'
              }
              icon={
                <>
                  <circle cx="9" cy="8" r="3" />
                  <path d="M3 20a6 6 0 0112 0" />
                  <circle cx="17" cy="9" r="2.5" />
                  <path d="M16 14.5a5 5 0 016 5" />
                </>
              }
            />
            <MenuRow
              to="/games?tab=load"
              title="Load game"
              sub={loadCount ? `${loadCount} game${loadCount === 1 ? '' : 's'} to continue` : 'Suspended, saved and running games'}
              icon={<path d="M3 7a2 2 0 012-2h4l2 2h8a2 2 0 012 2v8a2 2 0 01-2 2H5a2 2 0 01-2-2z" />}
            />
            <MenuRow
              to="/games?tab=history"
              title="Match history"
              sub="Players, decks, winners and full logs"
              icon={
                <>
                  <circle cx="12" cy="12" r="9" />
                  <path d="M12 7v5l3 2" />
                </>
              }
            />
            <MenuRow
              to="/rulings"
              title="Rulings"
              sub="Search official card rulings by card name or by what they say"
              icon={
                <>
                  <path d="M12 3v18M5 7h14" />
                  <path d="M5 7l-3 7a3 3 0 006 0zM19 7l-3 7a3 3 0 006 0z" />
                  <path d="M8 21h8" />
                </>
              }
            />
            <MenuRow
              to="/settings"
              title="Settings"
              sub={me.isAdmin ? 'Views, hotkeys, sounds · Admin: users, card data, backups' : 'Views, hotkeys, sounds, password'}
              icon={
                <>
                  <path d="M4 6h10M18 6h2M4 12h4M12 12h8M4 18h12" />
                  <circle cx="16" cy="6" r="2" />
                  <circle cx="10" cy="12" r="2" />
                  <circle cx="18" cy="18" r="2" />
                </>
              }
            />
          </nav>
        </div>
      </div>
      {showActive && <ActiveLobbies onClose={() => setShowActive(false)} onJoin={(l) => (l.hasPassword ? setPwFor(l.id) : joinLobby(l.id, null))} />}
      {pwFor && (
        <Modal
          title="Lobby password"
          onClose={() => setPwFor(null)}
          width={400}
          foot={
            <button className="tb pri" onClick={() => pwFor && joinLobby(pwFor, pw)}>
              Join
            </button>
          }
        >
          <div style={{ padding: 20 }}>
            <input className="inp" style={{ width: '100%' }} type="password" autoFocus value={pw} onChange={(e) => setPw(e.target.value)} onKeyDown={(e) => e.key === 'Enter' && pwFor && joinLobby(pwFor, pw)} />
          </div>
        </Modal>
      )}
    </div>
  );
}

export function DeckArt({ deck, height }: { deck: DeckSummary; height: number }) {
  const src = imgUrl(deck.coverImg, 'normal');
  return (
    <div
      style={{
        height,
        position: 'relative',
        backgroundColor: '#2a2d34',
        backgroundImage: src ? `url("${src}")` : 'repeating-linear-gradient(135deg, rgba(255,255,255,.05) 0 2px, transparent 2px 10px)',
        backgroundSize: src ? '140% auto' : undefined,
        backgroundPosition: src ? 'center 18%' : undefined,
      }}
    >
      <div style={{ position: 'absolute', left: 12, bottom: 10 }}>
        <ColorDots colors={deck.colors} size={16} />
      </div>
    </div>
  );
}
