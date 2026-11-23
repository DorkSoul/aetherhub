import { useEffect } from 'react';
import type { GameView } from '@aetherhub/shared';
import { Link, useNavigate, useParams } from 'react-router-dom';
import { play } from '../../lib/sound';
import { useApp } from '../../lib/store';
import { socket } from '../../lib/ws';
import { usePreview } from '../../ui/ui';
import { handleNotice, onNewLog, refreshMenus } from './driver';
import { mySeat, resetTable, useTable } from './store';
import { TableView } from './TableView';

let nextId = 1;

/**
 * A solo game on the server (one player in several seats): control moves on by itself, as in a solo game — during the
 * mulligans to your next seat that has not kept, then with the turn to whichever of your seats is active.
 */
function followSeat(before: GameView | null) {
  const { view: v, mySeats: mine } = useTable.getState();
  if (!v || mine.length < 2) return;
  const cur = mySeat(v);
  let want: number | null = null;
  if (v.stage === 'mulligan') {
    const open = (i: number) => !v.seats[i].mulligan?.kept && !v.seats[i].conceded;
    if (cur == null || !open(cur)) want = mine.find(open) ?? (mine.includes(v.turn.active) ? v.turn.active : null);
  } else if (mine.includes(v.turn.active) && (before?.stage === 'mulligan' || before?.turn.active !== v.turn.active)) want = v.turn.active;
  if (want != null && want !== cur) {
    socket.send({ t: 'seat', seat: want });
    useApp.getState().toast(`Now playing as ${v.seats[want]?.name}`);
  }
}

export default function Table() {
  const nav = useNavigate();
  const { id: gameId } = useParams();
  const closed = useTable((s) => s.closed);
  const elsewhere = useTable((s) => s.openedElsewhere);
  const hasView = useTable((s) => !!s.view);

  useEffect(() => {
    resetTable();
    useTable.setState({
      mode: 'online',
      restart: null,
      send: (intent) => socket.send({ t: 'intent', id: nextId++, intent }),
    });
    const toast = useApp.getState().toast;
    const off = socket.on((m) => {
      const st = useTable.getState();
      switch (m.t) {
        case 'game': {
          const before = st.view;
          useTable.setState({ view: m.view, log: m.log, seq: m.seq, host: m.host, hostSeat: m.hostSeat, undoable: m.undoable, closed: null, openedElsewhere: false, mySeats: m.mySeats ?? [] });
          refreshMenus();
          followSeat(before);
          break;
        }
        case 'patch': {
          const before = st.view;
          st.applyPatch(m.ops);
          useTable.setState({ seq: m.seq, undoable: m.undoable, host: m.host, hostSeat: m.hostSeat });
          refreshMenus();
          followSeat(before);
          break;
        }
        case 'log':
          onNewLog(m.entries);
          break;
        case 'logUpdate':
          useTable.setState({ log: st.log.map((l) => (l.id === m.entry.id ? m.entry : l)) });
          break;
        case 'reject':
          toast(m.reason);
          break;
        case 'notice':
          handleNotice(m.notice, m.cards, m.defs);
          break;
        case 'arrow': {
          const me = mySeat(st.view);
          useTable.setState({ arrows: [...st.arrows, { ...m.arrow, at: Date.now() }] });
          setTimeout(() => useTable.setState({ arrows: useTable.getState().arrows.filter((a) => a.id !== m.arrow.id) }), 10_000);
          const tgt = m.arrow.to;
          const mine = (tgt.seat != null && tgt.seat === me) || (tgt.iid && st.view?.cards[tgt.iid]?.owner === me);
          if (mine && m.arrow.seat !== me && useApp.getState().me?.prefs.alerts.pings) play('ping');
          break;
        }
        case 'arrowRemove':
          useTable.setState({ arrows: st.arrows.filter((a) => a.id !== m.id) });
          break;
        case 'ping': {
          useTable.setState({ pings: { ...st.pings, [m.iid]: Date.now() } });
          setTimeout(() => {
            const p = { ...useTable.getState().pings };
            delete p[m.iid];
            useTable.setState({ pings: p });
          }, 2300);
          const me = mySeat(st.view);
          if (st.view?.cards[m.iid]?.owner === me && m.seat !== me && useApp.getState().me?.prefs.alerts.pings) play('ping');
          break;
        }
        case 'openedElsewhere':
          useTable.setState({ openedElsewhere: true });
          break;
        case 'undoRequest':
          useTable.setState({ undoRequests: [...st.undoRequests.filter((u) => u.id !== m.req.id), m.req] });
          if (useApp.getState().me?.prefs.alerts.requests) play('request');
          break;
        case 'undoResolved':
          useTable.setState({ undoRequests: st.undoRequests.filter((u) => u.id !== m.id) });
          break;
        case 'gameClosed':
          useTable.setState({ closed: m.reason, view: m.reason === 'none' ? null : st.view });
          break;
        case 'sideboard':
          useTable.setState({ sideboard: m.state });
          break;
        case 'error':
          toast(m.message);
          break;
      }
    });
    if (gameId) socket.send({ t: 'watch', game: gameId });
    else useTable.setState({ closed: 'none' });
    return () => {
      off();
      socket.send({ t: 'unwatch' });
      usePreview.getState().hide(true);
      resetTable();
    };
  }, [gameId]);

  if (closed)
    return (
      <div className="screen" style={{ alignItems: 'center', justifyContent: 'center', gap: 14 }}>
        <h1 style={{ fontSize: 28 }}>{closed === 'suspended' ? 'Game suspended' : closed === 'none' ? 'No game is running' : closed === 'closed' ? 'Game closed' : closed}</h1>
        <div style={{ color: 'var(--mu)' }}>{closed === 'suspended' ? 'It is saved and can be loaded again from Load game.' : closed === 'closed' ? 'The match is in history.' : ''}</div>
        <div className="row">
          <Link to="/" className="tb">
            Main menu
          </Link>
          {closed === 'suspended' && (
            <Link to="/games?tab=load" className="tb pri">
              Load game
            </Link>
          )}
          {closed === 'closed' && (
            <Link to="/games?tab=history" className="tb pri">
              Match history
            </Link>
          )}
        </div>
      </div>
    );

  return (
    <>
      {hasView ? <TableView onExit={() => nav('/')} /> : <div style={{ padding: 40, color: 'var(--mu)' }}>Joining the table…</div>}
      {elsewhere && (
        <div className="scrim" style={{ zIndex: 2000 }}>
          <div className="modal col" style={{ padding: 24, gap: 14, width: 420 }}>
            <h2 style={{ fontSize: 20 }}>Opened elsewhere</h2>
            <div style={{ fontSize: 13.5, color: 'var(--tx3)' }}>Your seat is being controlled from another tab or device. The newest connection controls the seat.</div>
            <div className="row" style={{ justifyContent: 'flex-end' }}>
              <button className="tb" onClick={() => nav('/')}>
                Main menu
              </button>
              <button className="tb pri" onClick={() => socket.send({ t: 'takeBack' })}>
                Take back
              </button>
            </div>
          </div>
        </div>
      )}
    </>
  );
}
