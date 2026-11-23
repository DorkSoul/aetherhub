import { useQuery } from '@tanstack/react-query';
import { Link } from 'react-router-dom';
import type { GameSummary, LobbySummary } from '@aetherhub/shared';
import { formatName, gameKind, gameTypeLabel } from '@aetherhub/shared';
import { api } from '../lib/api';
import { useApp } from '../lib/store';
import { Modal } from '../ui/ui';

export interface ActiveRulesGame {
  id: string;
  names: string[];
  ais: number;
  turn: number;
  status: 'waiting' | 'running';
  mine: boolean;
}

export const useActiveRulesGames = (enabled = true) =>
  useQuery({ queryKey: ['rules-active'], queryFn: () => api.get<ActiveRulesGame[]>('api/rules/active'), refetchInterval: 10_000, enabled });

const rowStyle: React.CSSProperties = { gap: 14, padding: '10px 12px 10px 16px', borderRadius: 10, background: 'var(--surface)', border: '1px solid var(--line2)', flexWrap: 'wrap' };
const head: React.CSSProperties = { fontSize: 12, textTransform: 'uppercase', letterSpacing: 0.7, color: 'var(--mu3)', margin: '6px 0 2px' };

function Row({ title, sub, children }: { title: React.ReactNode; sub: React.ReactNode; children?: React.ReactNode }) {
  return (
    <div className="row" style={rowStyle}>
      <div className="grow" style={{ minWidth: 180 }}>
        <div style={{ fontSize: 13.5, fontWeight: 600 }}>{title}</div>
        <div style={{ fontSize: 12, color: 'var(--mu)', marginTop: 2 }}>{sub}</div>
      </div>
      {children}
    </div>
  );
}

function lobbySub(l: LobbySummary) {
  return `${gameTypeLabel(l)} · ${l.hostName} is hosting · ${l.seats} of ${l.maxPlayers} seats${gameKind(l.type) === 'constructed' ? ` · ${formatName(l.format)} · Best of ${l.bo}` : ''}${l.hasPassword ? ' · password' : ''}`;
}

/**
 * Everything running on the server, for players who arrive late: lobbies still being set up (join and take a seat),
 * draft / sealed events under way (join the lobby to watch and follow its games) and games in progress (spectate a
 * sandbox game when it allows spectators; rules enforced games are for their players only).
 */
export function ActiveLobbies({ onClose, onJoin }: { onClose: () => void; onJoin: (l: LobbySummary) => void }) {
  const status = useApp((s) => s.status);
  const me = useApp((s) => s.me)!;
  const lobbies = status?.lobbies ?? [];
  const games = status?.games ?? [];
  const rules = useActiveRulesGames();
  const setup = lobbies.filter((l) => l.phase === 'setup');
  const events = lobbies.filter((l) => l.phase !== 'setup');
  const rulesGames = rules.data ?? [];
  const empty = !lobbies.length && !games.length && !rulesGames.length;

  const joinBtn = (l: LobbySummary, label: string) =>
    l.inLobby ? (
      <Link to={`/lobby/${l.id}`} className="tb pri" onClick={onClose}>
        Back to lobby
      </Link>
    ) : (
      <button className="tb pri" onClick={() => onJoin(l)}>
        {label}
      </button>
    );

  const sandboxBtn = (g: GameSummary) =>
    g.hasSeat ? (
      <Link to={`/table/${g.id}`} className="tb pri" onClick={onClose}>
        Rejoin seat
      </Link>
    ) : g.spectators || me.isAdmin ? (
      <Link to={`/table/${g.id}`} className="tb" onClick={onClose}>
        Spectate
      </Link>
    ) : (
      <span style={{ fontSize: 12, color: 'var(--mu2)' }}>No spectators</span>
    );

  return (
    <Modal title="Active lobbies" onClose={onClose} width={640}>
      <div className="col" style={{ padding: 20, gap: 8 }}>
        {empty && <div style={{ padding: '16px 0', color: 'var(--mu2)', textAlign: 'center' }}>Nothing is running right now. Start one with New game.</div>}
        {setup.length > 0 && <div style={head}>Open lobbies</div>}
        {setup.map((l) => (
          <Row key={l.id} title={l.name} sub={lobbySub(l)}>
            {joinBtn(l, l.seats < l.maxPlayers ? 'Join' : 'Join to watch')}
          </Row>
        ))}
        {events.length > 0 && <div style={head}>Draft and sealed events in progress</div>}
        {events.map((l) => (
          <Row key={l.id} title={`${l.name} · ${l.phase === 'drafting' ? 'drafting' : l.phase === 'building' ? 'building decks' : 'playing'}`} sub={lobbySub(l)}>
            {joinBtn(l, 'Join to watch')}
          </Row>
        ))}
        {(games.length > 0 || rulesGames.length > 0) && <div style={head}>Games in progress</div>}
        {games.map((g) => (
          <Row key={g.id} title={`${g.name} · ${g.ended ? 'game over' : `turn ${g.turn}`}`} sub={`Sandbox game · ${g.hasSeat ? 'you have a seat' : g.spectators ? 'spectators allowed' : 'no spectators'}`}>
            {sandboxBtn(g)}
          </Row>
        ))}
        {rulesGames.map((g) => (
          <Row key={g.id} title={g.names.join(' vs ')} sub={`Rules enforced game · ${g.status === 'waiting' ? 'waiting for players' : `turn ${g.turn}`}${g.ais ? ` · ${g.ais} AI` : ''}`}>
            {g.mine ? (
              <Link to={`/rules?id=${g.id}`} className="tb pri" onClick={onClose}>
                Rejoin
              </Link>
            ) : (
              <span style={{ fontSize: 12, color: 'var(--mu2)' }}>Players only</span>
            )}
          </Row>
        ))}
      </div>
    </Modal>
  );
}
