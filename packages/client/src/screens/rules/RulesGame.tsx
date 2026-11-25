import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { Link, useNavigate, useSearchParams } from 'react-router-dom';
import type { rules as R } from '@aetherhub/shared';
import { api, ApiError } from '../../lib/api';
import { useApp } from '../../lib/store';
import { askConfirm, askText } from '../../ui/dialogs';
import { Chevron, useCompact, useMenu, type MenuItem } from '../../ui/ui';
import { RichLabel } from './mana';
import { AfterGameButtons } from '../afterGame';
import { unmarkDragged } from '../table/flights';
import { resetTable, useTable } from '../table/store';
import { TableView } from '../table/TableView';
import { isGridPick, PromptPanel } from './RulesPrompts';
import { AUTO_SLOT, counterKindOf, tapChoices, describeEvent, frameDelay, gestureOption, isPermanentCard, layoutBoard, logViews, onlyUntaps, optionsOf, PHASE_INDEX, PILE_DX, PHASE_STEP, sides, stepIndex, toGameView, type Placed, type Remembered, type RView } from './rulesView';

// Wire shapes of the rules-game API (packages/server/src/routes/rules.ts).
interface Frame {
  seq: number;
  events: R.Event[];
  view: RView;
  /** the seat whose answer produced this frame */
  by?: number;
}
interface State {
  id: string;
  names: string[];
  seat: number;
  /** every seat this player holds (several in a solo game) */
  mySeats?: number[];
  status: 'waiting' | 'running';
  versus: 'ai' | 'pvp';
  frames: Frame[];
  seq: number;
  epoch: number;
  view: RView | null;
  waiting: R.Decision | null;
  waitingId: number;
  waitingFor: number | null;
  over: R.GameResult | null;
  /** every human has left the game and the AIs play on (watching) */
  out?: boolean;
  error: string | null;
  images: Record<string, string>;
  notices: string[];
  commander: boolean;
  undoRequest: { steps: number; mine: boolean } | null;
  /** where this player's automatic passing of priority ends ("end turn", a phase, a stack item), or null */
  passing: R.PassUntil | null;
  /** answers given automatically for the rest of this game */
  reactions: Reaction[];
  /** "break the rules": the game is paused while this player fixes it by hand */
  paused: { seat: number; name: string; mine: boolean } | null;
}
interface Reaction {
  id: number;
  /** ignore: pass when this is on the stack (prompt '*' = any of the card's stack items) · mute: don't stop just because this ability can be used */
  kind: 'ignore' | 'yes' | 'no' | 'pick' | 'mute';
  source: string;
  prompt: string;
  option?: string;
}
const reactionLabel = (r: Reaction) =>
  r.kind === 'mute' ? `Don’t stop for ${r.source}’s ability` : r.kind === 'ignore' ? `Pass on ${r.source}` : r.kind === 'pick' ? `${r.source}: always “${r.option}”` : `${r.source}: always ${r.kind === 'yes' ? 'Yes' : 'No'}`;

/** Breaking the rules: pick one of the engine's tokens to put onto a board (a token a card failed to make). */
function TokenPicker({ seat, name, onPick, onClose }: { seat: number; name: string; onPick: (token: string, n: number, tapped: boolean) => void; onClose: () => void }) {
  const list = useQuery({ queryKey: ['rules-tokens'], queryFn: () => api.get<string[]>('api/rules/tokens'), staleTime: Infinity });
  const [q, setQ] = useState('');
  const [n, setN] = useState(1);
  const [tapped, setTapped] = useState(false);
  const words = q.toLowerCase().split(/\s+/).filter(Boolean);
  const shown = (list.data ?? []).filter((t) => words.every((w) => t.toLowerCase().includes(w))).slice(0, 120);
  return (
    <div style={{ position: 'fixed', inset: 0, zIndex: 960, background: 'rgba(0,0,0,.45)', display: 'flex', alignItems: 'center', justifyContent: 'center' }} onClick={onClose}>
      <div className="card-box col" style={{ width: 460, maxWidth: 'calc(100vw - 28px)', maxHeight: '80vh', padding: 16, gap: 10 }} onClick={(e) => e.stopPropagation()} role="dialog" aria-label="Create a token">
        <h3 className="display" style={{ margin: 0, fontSize: 18 }}>Create a token for {name}</h3>
        <input className="inp" autoFocus placeholder="Search tokens: soldier, treasure, 1/1 white…" value={q} onChange={(e) => setQ(e.target.value)} onKeyDown={(e) => e.key === 'Escape' && onClose()} />
        <div className="row" style={{ gap: 12, fontSize: 13 }}>
          <label className="row" style={{ gap: 6 }}>
            How many
            <input className="inp mono" type="number" min={1} max={50} value={n} onChange={(e) => setN(Math.max(1, Math.min(50, Number(e.target.value) || 1)))} style={{ width: 64 }} />
          </label>
          <label className="row" style={{ gap: 6 }}>
            <input type="checkbox" checked={tapped} onChange={(e) => setTapped(e.target.checked)} /> Tapped
          </label>
        </div>
        <div className="col" style={{ overflowY: 'auto', gap: 2, minHeight: 120 }}>
          {list.isLoading && <div style={{ color: 'var(--mu)' }}>Loading…</div>}
          {shown.map((t) => (
            <button key={t} className="mi" style={{ textAlign: 'left' }} onClick={() => onPick(t, n, tapped)}>
              {t}
            </button>
          ))}
          {list.data && !shown.length && <div style={{ color: 'var(--mu)' }}>No token matches.</div>}
        </div>
        <div className="row" style={{ justifyContent: 'flex-end' }}>
          <button className="tb" onClick={onClose}>
            Close
          </button>
        </div>
      </div>
    </div>
  );
}

/** "Remember my answer" under a may-question or a choice of modes: give the same answer automatically for the rest of the game. */
function RememberRow({ d, v, onRemember }: { d: R.Decision; v: RView; onRemember: (r: Omit<Reaction, 'id'>) => void }) {
  const k = d.kind;
  const source = k.t === 'YesNo' || k.t === 'Pick' ? k.source : null;
  const name = source === null ? null : ([...v.cards, ...v.decision_cards].find((c) => c.id === source)?.name ?? null);
  if (!name) return null;
  let opts: { label: string; r: Omit<Reaction, 'id'> }[] = [];
  if (k.t === 'YesNo')
    opts = [
      { label: 'Yes', r: { kind: 'yes', source: name, prompt: k.prompt } },
      { label: 'No', r: { kind: 'no', source: name, prompt: k.prompt } },
    ];
  else if (k.t === 'Pick' && k.min <= 1 && k.max >= 1 && ['Mode', 'ModeRepeat', 'Generic', 'Cost'].includes(k.purpose.t))
    opts = k.options.flatMap((o) => {
      const label = o.t === 'Text' ? o.text : o.t === 'Color' ? o.c : null;
      return label ? [{ label, r: { kind: 'pick' as const, source: name, prompt: k.prompt, option: label } }] : [];
    });
  if (!opts.length) return null;
  return (
    <details className="card-box" style={{ padding: '6px 12px', fontSize: 13, color: 'var(--mu)' }}>
      <summary style={{ cursor: 'pointer' }}>Remember my answer for {name}…</summary>
      <div className="row" style={{ gap: 6, flexWrap: 'wrap', marginTop: 6 }}>
        <span>Always answer (this game):</span>
        {opts.map((o) => (
          <button key={o.label} className="tb sm" onClick={() => onRemember(o.r)}>
            {o.label}
          </button>
        ))}
      </div>
    </details>
  );
}

function FloatingBox({ storeKey, children, hideBtn, width = 420, at }: { storeKey: string; children: React.ReactNode; hideBtn?: React.ReactNode; width?: number; /** where it sits until dragged (default: the middle of the screen) */ at?: React.CSSProperties }) {
  const read = (): { x: number; y: number } | null => {
    try {
      const v = JSON.parse(localStorage.getItem(storeKey) ?? 'null');
      return v && typeof v.x === 'number' && typeof v.y === 'number' ? v : null;
    } catch {
      return null;
    }
  };
  const [pos, setPos] = useState<{ x: number; y: number } | null>(read);
  const ref = useRef<HTMLDivElement>(null);
  const grab = useRef<{ dx: number; dy: number } | null>(null);
  useEffect(() => setPos(read()), [storeKey]);
  const clamp = (x: number, y: number) => {
    const w = ref.current?.offsetWidth ?? 400;
    return { x: Math.round(Math.min(Math.max(60 - w, x), window.innerWidth - 60)), y: Math.round(Math.min(Math.max(0, y), window.innerHeight - 40)) };
  };
  // keep it on screen when the window shrinks
  useEffect(() => {
    const f = () => setPos((p) => (p ? clamp(p.x, p.y) : p));
    window.addEventListener('resize', f);
    return () => window.removeEventListener('resize', f);
  }, []);
  const save = (p: { x: number; y: number } | null) => {
    try {
      if (p) localStorage.setItem(storeKey, JSON.stringify(p));
      else localStorage.removeItem(storeKey);
    } catch {
      /* storage unavailable: the box simply starts centred next time */
    }
  };
  const place: React.CSSProperties = pos ? { left: pos.x, top: pos.y } : (at ?? { left: '50%', top: '50%', transform: 'translate(-50%, -50%)' });
  /** where a press started; it becomes a drag only after the pointer moves a few pixels, so clicks on cards and tiles still work */
  const press = useRef<{ x: number; y: number; id: number } | null>(null);
  const interactive = (t: EventTarget | null) => t instanceof Element && !!t.closest('button, input, select, textarea, a, label, [data-nodrag], [role="button"]');
  return (
    <div
      ref={ref}
      style={{ position: 'fixed', ...place, width, maxWidth: 'calc(100vw - 28px)', maxHeight: '88vh', overflowY: 'auto', zIndex: 900, display: 'flex', flexDirection: 'column', gap: 6, boxShadow: '0 10px 40px rgba(0,0,0,.45)', borderRadius: 12, cursor: 'move' }}
      onPointerDown={(e) => {
        if (e.button !== 0 || interactive(e.target)) return;
        press.current = { x: e.clientX, y: e.clientY, id: e.pointerId };
      }}
      onPointerMove={(e) => {
        const p = press.current;
        if (p && !grab.current && Math.hypot(e.clientX - p.x, e.clientY - p.y) > 4) {
          const box = ref.current!.getBoundingClientRect();
          grab.current = { dx: p.x - box.left, dy: p.y - box.top };
          ref.current!.setPointerCapture(p.id);
        }
        if (grab.current) setPos(clamp(e.clientX - grab.current.dx, e.clientY - grab.current.dy));
      }}
      onPointerUp={() => {
        press.current = null;
        if (!grab.current) return;
        grab.current = null;
        setPos((p) => (save(p), p));
      }}
      onPointerCancel={() => {
        press.current = null;
        grab.current = null;
      }}
      onDoubleClick={(e) => {
        if (interactive(e.target)) return;
        setPos(null);
        save(null);
      }}
    >
      {hideBtn && <div style={{ position: 'absolute', right: 8, top: 8, zIndex: 1 }}>{hideBtn}</div>}
      {children}
    </div>
  );
}

/** Minimal view: three separate indicators (whose turn to act, what is on the stack, messages) instead of one icon. */
function StatusChips({ priority, stackCount, stackEmpty, noteCount, open, setOpen, onPanel, right }: { priority: boolean; stackCount: number; stackEmpty: boolean; noteCount: number; open: 'stack' | 'notes' | null; setOpen: (v: 'stack' | 'notes' | null) => void; onPanel: () => void; right: number }) {
  const chip = (on: boolean, extra?: React.CSSProperties): React.CSSProperties => ({ borderColor: on ? 'var(--acc)' : undefined, color: on ? 'var(--tx)' : 'var(--mu)', ...extra });
  return (
    <div style={{ position: 'fixed', right, top: 62, zIndex: 901, display: 'flex', gap: 6 }}>
      <span className="tb sm" style={{ ...chip(priority), cursor: 'default' }} title={priority ? (stackEmpty ? 'You have priority and the stack is empty' : 'You have priority: the stack is not empty, so you can respond') : 'You do not have priority'}>
        {priority && <span className="dot" style={{ background: 'var(--acc)', marginRight: 6 }} />}
        Priority{priority ? (stackEmpty ? ' · empty stack' : ' · respond?') : ''}
      </span>
      <button className="tb sm" aria-pressed={open === 'stack'} style={chip(stackCount > 0)} title="The stack" onClick={() => setOpen(open === 'stack' ? null : 'stack')}>
        Stack{stackCount > 0 ? ` ${stackCount}` : ''}
      </button>
      <button className="tb sm" aria-pressed={open === 'notes'} style={chip(noteCount > 0)} title="Messages and warnings (cards the AI plays badly, undo requests, ...)" onClick={() => setOpen(open === 'notes' ? null : 'notes')}>
        ⓘ{noteCount > 0 ? ` ${noteCount}` : ''}
      </button>
      <button className="tb sm" style={chip(false)} title="Open the side panel" aria-label="Open the game panel" onClick={onPanel}>
        ▤
      </button>
    </div>
  );
}

/** What the rules engine can't do with a deck: cards it has no definition for (left out of the game) and, in a Commander game, a missing commander. */


/** The open game is remembered in localStorage, so a reload or a new tab goes straight back to it. */
const GAME_KEY = 'aetherhub.rulesGame';
function readGameId(): string | null {
  try {
    return localStorage.getItem(GAME_KEY);
  } catch {
    return null;
  }
}
function writeGameId(id: string | null) {
  try {
    if (id) localStorage.setItem(GAME_KEY, id);
    else localStorage.removeItem(GAME_KEY);
  } catch {
    /* storage unavailable: the running-games list still finds it */
  }
}

/** A playback unit: the view to show and what happened to get there. */
type Shown = { view: RView; events: R.Event[]; /** produced by this player's own answer: shown at once */ own?: boolean };

export default function RulesGame() {
  const compact = useCompact();
  // Phones have no side panel: the messages open over the table instead.
  const detailedView = useApp((s) => s.me?.prefs.panel === 'detailed') && !compact;
  const autoSort = useApp((s) => s.me?.prefs.autoSort ?? false);
  const [msgOpen, setMsgOpen] = useState(false);
  const [chipOpen, setChipOpen] = useState<'stack' | 'notes' | null>(null);
  /** Phones: the prompt box covers the opponent's board, so it can be tucked away (until the next prompt). */
  const [promptHidden, setPromptHidden] = useState(false);
  /** breaking the rules: the board a token is being created on */
  const [tokenSeat, setTokenSeat] = useState<number | null>(null);
  const slot = useTable((st) => st.rulesSlot);
  // Minimal view: the log drawer opens at the top right, so the chips and boxes move to its left instead of covering it
  const logOpen = useTable((st) => st.logOpen);
  const qc = useQueryClient();
  const nav = useNavigate();
  const [params, setParams] = useSearchParams();
  const urlId = params.get('id');
  const [gameId, setGameId] = useState<string | null>(() => urlId ?? readGameId());
  // a game opened by link (the lobby, Load game, the home page) replaces the remembered one
  useEffect(() => {
    if (urlId && urlId !== gameId) {
      writeGameId(urlId);
      setGameId(urlId);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [urlId]);
  useEffect(() => {
    if (gameId) writeGameId(gameId);
  }, [gameId]);
  /** the seat on screen: a player holding several seats (solo) follows the one that must act */
  const seatRef = useRef<number | null>(null);
  const [raw, setState] = useState<State | null>(null);
  const [err, setErr] = useState('');
  /** the centred win / lose message was closed */
  const [overClosed, setOverClosed] = useState(false);
  // An error message fades on its own, so a stale one can never stay stuck on the table.
  useEffect(() => {
    if (!err) return;
    const t = setTimeout(() => setErr(''), 6000);
    return () => clearTimeout(t);
  }, [err]);
  const busy = useRef(false);
  /** Bumped whenever the game on screen changes: an answer to a request made for an earlier game is ignored. */
  const pollGen = useRef(0);
  // Playback: the engine plays AI turns in an instant, so the server keeps every step as a frame and they are shown one by one here.
  const queue = useRef<Shown[]>([]);
  const shownRef = useRef<Shown | null>(null);
  const [shown, setShown] = useState<Shown | null>(null);
  const [ready, setReady] = useState(true);
  const seqRef = useRef<number | null>(null);
  const epochRef = useRef<number | null>(null);
  const imgs = useRef<Record<string, string>>({});
  const placed = useRef<Placed>(new Map());
  /** piles of permanents the player made ("Group identical", "Stack into a pile") or that sorting made: kept together by sorting */
  const groups = useRef<number[][]>([]);
  /** where the player left each permanent, kept across undo / replay (which rebuild the game and so forget `placed`) */
  const memory = useRef<Remembered>(new Map());
  /** the game log, built from the events as the frames are shown */
  const logLines = useRef<string[]>([]);
  /** names of cards the log saw once that are hidden again (a one-shot scheme back at the bottom of its deck) */
  const knownNames = useRef(new Map<number, string>());
  // Card / hand size per seat: only how this screen draws the table.
  const sizes = useRef<Record<number, { handFrac?: number; cardScale?: number }>>({});
  /** cards the player just dropped on the table from their hand: shown there at once, until the engine accepts the play or it is refused */
  const optimistic = useRef<Placed>(new Map());
  /** the order of this player's hand as they arranged it; cards that arrive go at the end */
  const handOrder = useRef<number[]>([]);
  /** plays made while the game was busy (the AI acting, the server working): tried in order at this player's next priority */
  const actionQueue = useRef<{ intent: Parameters<NonNullable<ReturnType<typeof useTable.getState>['send']>>[0]; card: number }[]>([]);
  const [queueTick, setQueueTick] = useState(0);
  /** true while a queued play is being sent, so a refusal drops the rest of the queue */
  const fromQueue = useRef(false);
  /** requests to the server in flight (answers, passes): "Sending…" */
  const [sending, setSending] = useState(0);
  /** the decision the player last answered; a late poll that still carries it must not bring it back */
  const lastAnswered = useRef(0);
  const holding = useRef(new Set<number>());
  const [layoutTick, setLayoutTick] = useState(0);
  const waitingId = raw?.waiting ? raw.waitingId : 0;
  useEffect(() => {
    setPromptHidden(false);
  }, [waitingId]);
  // Detailed view: a new decision brings the side panel's Game tab forward
  useEffect(() => {
    if (waitingId) useTable.setState({ panelTab: 'game' });
  }, [waitingId]);
  /** set the moment End turn / a phase / a stack item is clicked, until the server's own `passing` takes over */
  const [endingLocal, setEnding] = useState(false);
  const [endPhaseLocal, setEndPhase] = useState<number | null>(null);
  const passing = raw?.passing ?? null;
  const ending = endingLocal || !!passing;
  const passingTurn = passing && 'Step' in passing ? passing.Step : null;
  const endPhase = endPhaseLocal ?? (passingTurn && raw?.view && passingTurn.turn === raw.view.turn ? (PHASE_INDEX[String(passingTurn.step)] ?? null) : null);
  const passingItem = passing && 'StackTop' in passing ? passing.StackTop.item : null;
  const state = useMemo(
    () =>
      raw
        ? {
            ...raw,
            view: shown?.view ?? raw.view,
            images: imgs.current,
            waiting: ready && raw.waiting && raw.waitingId !== lastAnswered.current ? raw.waiting : null,
            over: ready ? raw.over : null,
          }
        : null,
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [raw, shown, ready],
  );

  const resetPlayback = () => {
    queue.current = [];
    shownRef.current = null;
    setShown(null);
    imgs.current = {};
    placed.current.clear();
    optimistic.current.clear();
    actionQueue.current = [];
    logLines.current = [];
    setEnding(false);
    setEndPhase(null);
  };

  const poll = useCallback(async () => {
    if (!gameId || busy.current) return;
    busy.current = true;
    const gen = pollGen.current;
    try {
      const hint = seatRef.current !== null ? `as=${seatRef.current}` : '';
      const q = epochRef.current !== null ? `?since=${seqRef.current}&epoch=${epochRef.current}&wait=1${hint ? '&' + hint : ''}` : hint ? `?${hint}` : '';
      const r = await api.get<State>(`api/rules/games/${gameId}${q}`);
      // the game was ended or swapped while this (long) request was open: its answer belongs to the old game
      if (gen !== pollGen.current) return;
      // a solo player's view moves to the seat that must act now: start that seat's playback afresh
      if (seatRef.current !== null && r.seat !== seatRef.current && (r.mySeats?.length ?? 0) > 1) {
        resetPlayback();
        lastAnswered.current = 0;
        useApp.getState().toast(`Now playing as ${r.names[r.seat] ?? 'seat ' + (r.seat + 1)}`);
      }
      seatRef.current = r.seat;
      if (epochRef.current !== r.epoch) {
        // a new game (or undo / resume): drop whatever was queued from the old one
        resetPlayback();
        epochRef.current = r.epoch;
      }
      seqRef.current = r.seq;
      imgs.current = { ...imgs.current, ...r.images };
      if (r.frames.length) {
        queue.current.push(...r.frames.map((f) => ({ view: f.view, events: f.events, own: f.by === r.seat })));
        setReady(false);
      }
      setState(r);
    } catch (e) {
      // only a game that is gone (or not yours) is forgotten: a dropped connection, a server restart or the request a reload
      // cancels must not lose the game, so those just wait and poll again
      if (gen !== pollGen.current) return;
      const status = e instanceof ApiError ? e.status : 0;
      if (status === 404 || status === 403) {
        setErr((e as Error).message);
        writeGameId(null);
        setGameId(null);
      } else await new Promise((r) => setTimeout(r, 1500));
    } finally {
      if (gen === pollGen.current) busy.current = false;
    }
  }, [gameId]);

  useEffect(() => {
    // a request still open for the previous game must neither block this game's polling nor overwrite it
    pollGen.current++;
    busy.current = false;
    // question numbers start again in every game: forgetting the last one answered keeps the new game's first
    // question (keep or mulligan) from being taken for one already answered
    lastAnswered.current = 0;
    epochRef.current = null;
    seqRef.current = null;
    setReady(true);
    resetPlayback();
    memory.current.clear();
    groups.current = [];
  }, [gameId]);

  useEffect(() => {
    if (!gameId) return;
    let stop = false;
    const loop = async () => {
      while (!stop) {
        await poll();
        // the server holds the request until something changes (long poll); this only paces failures and the first call
        await new Promise((r) => setTimeout(r, epochRef.current === null ? 100 : 30));
      }
    };
    void loop();
    return () => {
      stop = true;
    };
  }, [gameId, poll]);

  useEffect(() => {
    if (!gameId) return;
    let stop = false;
    let timer: ReturnType<typeof setTimeout>;
    const names = () => stateRef.current?.names ?? [];
    const tick = () => {
      if (stop) return;
      let next = queue.current.shift();
      const prefs = useApp.getState().me?.prefs;
      const pace = prefs?.aiPaceOn ? Math.max(0.01, prefs.aiPaceSeconds) * 1000 : null;
      // a run of single-card untaps (untap step) is shown as one frame, unless the player asked to see every action
      while (pace === null && next && queue.current.length && shownRef.current && onlyUntaps(next.view, queue.current[0].view)) next = queue.current.shift();
      if (!next) {
        setReady(true);
        timer = setTimeout(tick, 60);
        return;
      }
      for (const e of next.events) {
        // the rules changed or refused something this player asked for (a block that was ignored, a forced attacker ...)
        if (e.t === 'Notice' && e.player === stateRef.current?.seat) useApp.getState().toast(e.text);
        const line = describeEvent(e, next.view, names(), knownNames.current);
        if (line) logLines.current.push(line);
      }
      if (logLines.current.length > 400) logLines.current.splice(0, logLines.current.length - 400);
      const prev = shownRef.current;
      shownRef.current = next;
      setShown(next);
      // never skip a step; a long backlog just plays a little quicker. The player's own action shows at once; with
      // "slow down other players' actions" on, every other frame waits exactly that long (one action at a time).
      const backlog = queue.current.length;
      const following = queue.current[0];
      const delay = following?.own ? 0 : pace !== null ? pace : Math.max(60, frameDelay(prev?.view ?? null, next.view) / (backlog > 30 ? 2 : 1));
      timer = setTimeout(tick, delay);
    };
    tick();
    return () => {
      stop = true;
      clearTimeout(timer);
    };
  }, [gameId]);

  const answer = async (choice: R.Choice, manual = false) => {
    const cur = stateRef.current;
    if (!cur?.waiting || !gameId || !raw) return;
    lastAnswered.current = cur.waitingId;
    setState({ ...raw, waiting: null });
    setSending((n) => n + 1);
    try {
      // decisionId: the server refuses the answer if the game has moved on to another question since
      await api.post(`api/rules/games/${gameId}/answer`, manual ? { choice, manual, decisionId: cur.waitingId } : { choice, decisionId: cur.waitingId });
    } catch (e) {
      lastAnswered.current = 0;
      const msg = (e as Error).message;
      // A stale click (the game moved on, e.g. skip-until ran while the AI was thinking) is not an error worth showing:
      // the next poll brings the real state.
      if (/no longer pending/i.test(msg)) return;
      setErr(msg);
      setState((s) => (s ? { ...s, waiting: cur.waiting } : s));
    } finally {
      setSending((n) => n - 1);
    }
  };
  /** Drop every queued play (one was refused or undone, so the plan behind the rest may have changed). */
  const clearQueue = (why: string | null) => {
    const q = actionQueue.current.splice(0);
    if (!q.length) return;
    for (const { card } of q) {
      unmarkDragged([String(card)]);
      optimistic.current.delete(card);
      placed.current.delete(card);
    }
    setQueueTick((t) => t + 1);
    setLayoutTick((t) => t + 1);
    if (why) useApp.getState().toast(`${why}: ${q.length} queued action${q.length === 1 ? '' : 's'} dropped.`);
  };
  const clearQueueRef = useRef(clearQueue);
  clearQueueRef.current = clearQueue;
  const respondUndo = async (accept: boolean) => {
    if (!gameId) return;
    try {
      setState(await api.post<State>(`api/rules/games/${gameId}/undo/respond`, { accept }));
    } catch (e) {
      setErr((e as Error).message);
    }
  };
  const undo = async () => {
    if (!gameId) return;
    try {
      setState(await api.post<State>(`api/rules/games/${gameId}/undo`, { steps: 1 }));
    } catch (e) {
      setErr((e as Error).message);
    }
  };
  const save = async () => {
    if (!gameId) return;
    const name = await askText({ title: 'Save this game', label: 'Name for the saved game', initial: new Date().toLocaleString(), confirm: 'Save' });
    if (!name) return;
    try {
      await api.post(`api/rules/games/${gameId}/save`, { name });
      void qc.invalidateQueries({ queryKey: ['rules-saves'] });
      setErr('');
    } catch (e) {
      setErr((e as Error).message);
    }
  };
  const concede = async () => {
    if (!gameId || !(await askConfirm({ title: 'Concede this game?', confirm: 'Concede', danger: true }))) return;
    try {
      await api.post(`api/rules/games/${gameId}/concede`);
    } catch (e) {
      setErr((e as Error).message);
    }
  };
  /** End the game for this player: removes it against the AI; with other players still in it, concedes and leaves. */
  const leave = async () => {
    if (gameId) await api.del(`api/rules/games/${gameId}`).catch(() => {});
    writeGameId(null);
    setGameId(null);
    setState(null);
    setParams({});
    void qc.invalidateQueries({ queryKey: ['rules-mine'] });
  };
  const endGame = async () => {
    const pvp = stateRef.current?.versus === 'pvp' && !stateRef.current?.over;
    const ok = await askConfirm(pvp ? { title: 'End the game for you?', text: 'You concede and leave; the other players play on.', confirm: 'Concede and leave', danger: true } : { title: 'End this game?', text: 'It is not saved.', confirm: 'End game', danger: true });
    if (!ok) return;
    await leave();
  };
  /** Back to the main menu; the game keeps running and can be rejoined from the home page or Load game. */
  const backToMenu = () => {
    writeGameId(null);
    setGameId(null);
    setState(null);
    void qc.invalidateQueries({ queryKey: ['rules-mine'] });
    nav('/');
  };
  /** Start or finish "break the rules". */
  const toggleBreakRef = useRef<() => Promise<void>>(async () => {});
  toggleBreakRef.current = async () => {
    const cur = stateRef.current;
    if (!gameId || !cur) return;
    const on = !cur.paused;
    if (
      on &&
      !(await askConfirm({
        title: 'Break the rules?',
        confirm: 'Break the rules',
        text: (
          <>
            <p style={{ margin: '0 0 10px' }}>The game pauses (no one, and no AI, can act) so you can put right something the rules engine got wrong: drag cards to any zone, tap or untap them, change counters and life, draw or shuffle. Choose Finish when done and the game carries on from there.</p>
            <p style={{ margin: 0 }}>The changes are kept with the game (undo and saving keep them). Please report the bug so it can be fixed.</p>
          </>
        ),
      }))
    )
      return;
    try {
      await api.post(`api/rules/games/${gameId}/break`, { on });
    } catch (e) {
      useApp.getState().toast((e as Error).message);
    }
  };
  const rememberRef = useRef<(r: Omit<Reaction, 'id'>) => void>(() => {});
  rememberRef.current = (r) => {
    if (!gameId) return;
    api
      .post(`api/rules/games/${gameId}/reactions`, r)
      .then(() => useApp.getState().toast(`Remembered for this game: ${reactionLabel({ ...r, id: 0 })}. Forget it from ◇ Menu → Remembered reactions.`))
      .catch((e: Error) => setErr(e.message));
  };
  const forgetRef = useRef<(id: number) => void>(() => {});
  forgetRef.current = (id) => {
    if (gameId) api.del(`api/rules/games/${gameId}/reactions/${id}`).catch((e: Error) => setErr(e.message));
  };
  /** Right-click on a stack item: pass until it is on top, or always pass when it is on the stack. */
  const stackMenuRef = useRef<(e: React.MouseEvent, item: R.StackView) => void>(() => {});
  stackMenuRef.current = (e, item) =>
    useMenu.getState().open(e.clientX + 2, e.clientY + 2, () => [
      { kind: 'head', label: item.name },
      { kind: 'item', label: 'Pass until this is on top', run: () => passRef.current({ stackItem: item.id }) },
      { kind: 'item', label: `Always pass when ${item.name} is on the stack`, hint: 'this game', run: () => rememberRef.current({ kind: 'ignore', source: item.name, prompt: item.text }) },
    ]);
  const concedeRef = useRef(concede);
  concedeRef.current = concede;
  const endGameRef = useRef(endGame);
  endGameRef.current = endGame;

  /** What the game is doing while this player waits, shown in the top bar so it never looks frozen. */
  const statusText = (() => {
    if (!state || state.over || state.error || state.status !== 'running') return null;
    const q = actionQueue.current.length;
    const queued = q ? ` · ${q} queued` : '';
    if (state.waiting) return q ? `${q} queued` : null;
    if (sending > 0) return 'Sending…' + queued;
    if (state.out) return 'Watching the AIs play on';
    if (!ready) return 'Showing moves…' + queued;
    const wf = state.waitingFor;
    if (wf !== null && wf !== state.seat) return (state.versus === 'ai' ? `${state.names[wf] ?? 'The AI'} is thinking…` : `Waiting for ${state.names[wf] ?? 'another player'}…`) + queued;
    return 'Working…' + queued;
  })();

  // keep the app's table in step with the engine's view; gestures on it become rules actions
  const mouse = useRef({ x: 200, y: 200 });
  useEffect(() => {
    const f = (e: MouseEvent) => (mouse.current = { x: e.clientX, y: e.clientY });
    window.addEventListener('mousemove', f);
    return () => window.removeEventListener('mousemove', f);
  }, []);
  const stateRef = useRef<typeof state>(null);
  stateRef.current = state;
  const undoRef = useRef<() => void>(() => {});
  const saveRef = useRef<() => void>(() => {});
  const answerRef = useRef<(c: R.Choice) => void>(() => {});
  answerRef.current = (c) => void answer(c);
  undoRef.current = () => void undo();
  saveRef.current = () => void save();
  const passRef = useRef<(target: { turn: number; step: number } | { stackItem: number } | { player: number; after?: number }) => void>(() => {});
  passRef.current = (target) => {
    if (!gameId) return;
    setEnding(true);
    setSending((n) => n + 1);
    api
      .post(`api/rules/games/${gameId}/passuntil`, target)
      .then(() => setEnding(false))
      .catch((e) => {
        setEnding(false);
        setEndPhase(null);
        setErr((e as Error).message);
      })
      .finally(() => setSending((n) => n - 1));
  };
  const cancelPassRef = useRef<() => void>(() => {});
  cancelPassRef.current = () => {
    setEnding(false);
    setEndPhase(null);
    if (gameId) api.post(`api/rules/games/${gameId}/passuntil/cancel`).catch((e) => setErr((e as Error).message));
  };

  /** Ask the engine why something was refused and show the reason. */
  const explainRef = useRef<(kind: 'play' | 'attack' | 'block' | 'tap', card: number | null, other: number | null, fallback: string) => void>(() => {});
  explainRef.current = (kind, card, other, fallback) => {
    if (!gameId) return void useApp.getState().toast(fallback);
    const q = new URLSearchParams({ kind });
    if (card !== null) q.set('card', String(card));
    if (other !== null) q.set('other', String(other));
    api
      .get<{ text: string }>(`api/rules/games/${gameId}/explain?${q}`)
      .then((r) => useApp.getState().toast(r.text))
      .catch(() => useApp.getState().toast(fallback));
  };

  /** Tap a permanent for floating mana (needs a priority or payment question to be open). */
  const tapMana = (cardId: number, ability: number, produce: R.ManaType | null) => {
    const w = stateRef.current?.waiting;
    if (!w) return explainRef.current('tap', cardId, null, "You can't tap for mana right now");
    if (w.kind.t === 'PayMana') return answerRef.current({ t: 'Pay', answer: { t: 'Tap', card: cardId, ability, produce } });
    // choosing what to give up for a cost (a land to sacrifice...): mana abilities may still be used first (CR 601.2g)
    const costPick = w.kind.t === 'Pick' && w.kind.purpose.t === 'Cost';
    if (w.kind.t !== 'Priority' && !costPick) return explainRef.current('tap', cardId, null, "You can't tap for mana right now");
    answerRef.current({ t: 'Mana', card: cardId, ability, produce, x: null, picks: [] });
  };
  const manaMenu = (cardId: number) => {
    const v = stateRef.current?.waiting ? stateRef.current.view : null;
    const srcs = v?.decision ? v.mana_sources.filter((s) => s.card === cardId) : [];
    return srcs.flatMap(tapChoices);
  };

  /** Sorts one side of the table (see `layoutBoard`); returns true when a card moved. */
  const sortSeat = (v: RView, seat: number): boolean => {
    const sideOf = sides(v.cards);
    const mine = v.cards.filter((c) => c.zone === 'Battlefield' && sideOf(c) === seat && !optimistic.current.has(c.id));
    const { pos, piles } = layoutBoard(mine, v.players.length, groups.current, placed.current);
    const onField = new Set(v.cards.filter((c) => c.zone === 'Battlefield').map((c) => c.id));
    groups.current = [...groups.current, ...piles].map((g) => g.filter((id) => onField.has(id))).filter((g) => g.length > 1);
    let moved = false;
    for (const [id, p] of pos) {
      const was = placed.current.get(id);
      if (!was || Math.abs(was.x - p.x) > 1e-6 || Math.abs(was.y - p.y) > 1e-6) moved = true;
      placed.current.set(id, p);
      // remembered so that undo / replay put it back here
      const name = mine.find((c) => c.id === id)?.name;
      if (name !== undefined) memory.current.set(id, { ...p, name });
    }
    return moved;
  };

  useEffect(() => {
    if (!gameId || !state?.view) return;
    const v = state.view;
    const queuedIds = new Map(actionQueue.current.map((q, i) => [q.card, i + 1]));
    const { view } = toGameView(v, state.images ?? {}, state.names ?? [], state.seat, state.id, undefined, placed.current, optimistic.current, memory.current, queuedIds, handOrder.current);
    // Auto-sort: every side of the table is kept sorted (after the new permanents got their first spot above)
    if (autoSort) {
      let moved = false;
      for (let seat = 0; seat < v.players.length; seat++) moved = sortSeat(v, seat) || moved;
      if (moved)
        for (const c of v.cards) {
          const p = placed.current.get(c.id);
          if (c.zone === 'Battlefield' && p && view.cards[String(c.id)]) Object.assign(view.cards[String(c.id)], p);
        }
    }
    view.settings.format = state.commander ? 'commander' : 'standard';
    view.seats.forEach((seat, i) => {
      const sz = sizes.current[i];
      if (sz?.handFrac != null) seat.handFrac = sz.handFrac;
      if (sz?.cardScale != null) seat.cardScale = sz.cardScale;
    });
    const options = optionsOf(state.waiting, v);
    useTable.setState({
      // each attacker points where its damage will go: the defender, or its blockers (and the defender too with trample)
      combatArrows: v.combat.flatMap((c) => [
        ...(c.damage_to?.length ? c.damage_to : [c.defender]).map((t, i) => ({ id: `${c.attacker}>${i}`, kind: 'attack' as const, from: { iid: String(c.attacker) }, to: t.kind === 'Player' ? { seat: t.id } : { iid: String((t as { id: number }).id) } })),
        ...c.blockers.map((b) => ({ id: String(b), kind: 'block' as const, from: { iid: String(b) }, to: { iid: String(c.attacker) } })),
      ]),
      mode: 'local',
      view,
      log: logViews(logLines.current),
      host: false,
      hostSeat: null,
      undoable: 0,
      restart: null,
      rules: {
        undo: () => void undoRef.current(),
        save: () => void saveRef.current(),
        canUndo: !state.undoRequest && v.turn > 0,
        endTurn: () => {
          const cur = stateRef.current?.view;
          if (cur) passRef.current({ turn: cur.turn + 1, step: 0 });
        },
        cancelPass: () => cancelPassRef.current(),
        // the turn on screen when clicked: the request can reach the engine after that player's turn has begun
        passToPlayer: (seat) => {
          const cur = stateRef.current?.view;
          passRef.current(cur ? { player: seat, after: cur.turn } : { player: seat });
        },
        breakRules: { by: state.paused ? (state.paused.mine ? null : state.paused.name) : null, active: !!state.paused, mine: !!state.paused?.mine },
        toggleBreak: () => void toggleBreakRef.current(),
        openTokens: (seat) => setTokenSeat(seat),
        reactions: (state.reactions ?? []).map((r) => ({ id: r.id, label: reactionLabel(r), forget: () => forgetRef.current(r.id) })),
        escape: () => {
          if (stateRef.current?.passing || endingLocal) cancelPassRef.current();
          clearQueueRef.current(null);
        },
        status: statusText,
        ending,
        endPhase,
        passTo: (i) => {
          const cur = stateRef.current?.view;
          if (!cur || state.over || state.error) return;
          // a phase that has already started this turn means that phase of the next turn
          const later = i > (PHASE_INDEX[String(cur.step)] ?? 0);
          setEndPhase(later ? i : null);
          passRef.current({ turn: later ? cur.turn : cur.turn + 1, step: stepIndex(PHASE_STEP[i]) });
        },
        layout: (seat, patch) => {
          sizes.current[seat] = { ...sizes.current[seat], ...patch };
          setLayoutTick((t) => t + 1);
        },
        sortBoard: (seat) => {
          sortSeat(v, seat);
          setLayoutTick((t) => t + 1);
        },
        concede: () => void concedeRef.current(),
        endGame: () => void endGameRef.current(),
        attention: detailedView && !slot && (!!(state.waiting && state.waiting.kind.t !== 'Priority') || v.stack.length > 0),
        abilities: (iid) => {
          if (state.paused) return [];
          const id = Number(iid);
          const acts = state.waiting?.kind.t === 'Priority' ? state.waiting.kind.actions : [];
          const cardName = state.view?.cards.find((c) => c.id === id)?.name ?? null;
          const card = state.view?.cards.find((c) => c.id === id);
          const manaSrcs = manaMenu(id);
          // a mana ability the "Tap: add" entries below already offer isn't listed a second time
          const covered = (o: { action: number }) => {
            const a = acts[o.action];
            return a?.t === 'ActivateMana' && manaSrcs.some(({ s }) => s.card === a.card && s.ability === a.ability);
          };
          const opts = (options ?? []).filter((o) => o.cardId === id && o.kind === 'ability' && !covered(o)).map((o) => {
            const run = () => answerRef.current({ t: 'Priority', action: o.action });
            const a = acts[o.action];
            if (!a || a.t !== 'Activate' || !cardName) return { label: o.label, run };
            // more ways to use it, and how the game treats it from now on (the same choices a stack item offers)
            const key = `ability:${a.ability}`;
            const sub = (): MenuItem[] => {
              const rs = stateRef.current?.reactions ?? [];
              const muted = rs.find((r) => r.kind === 'mute' && r.source === cardName && r.prompt === key);
              const ignored = rs.find((r) => r.kind === 'ignore' && r.source === cardName && r.prompt === '*');
              return [
                { kind: 'item', label: 'Activate', run },
                { kind: 'item', label: 'Activate, tapping mana by hand', run: () => void answer({ t: 'Priority', action: o.action }, true) },
                { kind: 'sep' },
                { kind: 'head', label: 'For the rest of this game' },
                { kind: 'item', label: 'Don’t stop to offer this ability', hint: 'skip priority', checked: !!muted, run: () => (muted ? forgetRef.current(muted.id) : rememberRef.current({ kind: 'mute', source: cardName, prompt: key })) },
                { kind: 'item', label: `Always pass when ${cardName} is on the stack`, checked: !!ignored, run: () => (ignored ? forgetRef.current(ignored.id) : rememberRef.current({ kind: 'ignore', source: cardName, prompt: '*' })) },
              ];
            };
            return { label: o.label, run, sub };
          });
          // a spell in the hand: cast it normally, or tap its mana by hand (choose which lands pay for it)
          const casts = (options ?? []).filter((o) => o.cardId === id && o.kind === 'spell').flatMap((o) => [
            { label: o.label, run: () => answerRef.current({ t: 'Priority', action: o.action }) },
            { label: `${o.label} (tap mana by hand)`, run: () => void answer({ t: 'Priority', action: o.action }, true) },
          ]);
          const plays = (options ?? []).filter((o) => o.cardId === id && o.kind === 'land').map((o) => ({ label: o.label, run: () => answerRef.current({ t: 'Priority', action: o.action }) }));
          // each mana ability by its text; one that can make several kinds of mana gets an entry per kind
          const mana = manaSrcs.map(({ s, m, text }) => {
            const full = card?.abilities.find((x) => x.index === s.ability)?.text;
            const several = manaSrcs.filter((x) => x.s.ability === s.ability).length > 1;
            const cost = full?.split(': ')[0];
            return { label: full && !several ? full : `${cost ?? '{T}'}: Add ${text}`, run: () => tapMana(s.card, s.ability, m) };
          });
          return [...plays, ...casts, ...opts, ...mana];
        },
      },
      send: (intent) => {
        const st = stateRef.current;
        if (intent.type === 'layout') {
          sizes.current[state.seat] = { ...sizes.current[state.seat], ...(intent.handFrac != null ? { handFrac: intent.handFrac } : {}), ...(intent.cardScale != null ? { cardScale: intent.cardScale } : {}) };
          setLayoutTick((t) => t + 1);
          return;
        }
        // rearranging the hand only changes how this screen shows it
        if (intent.type === 'move' && intent.zone === 'hand' && st?.view && intent.seat === st.seat) {
          const ids = (intent.iids ?? []).map(Number);
          if (ids.length && ids.every((id) => handOrder.current.includes(id))) {
            const rest = handOrder.current.filter((id) => !ids.includes(id));
            rest.splice(Math.min(rest.length, typeof intent.pos === 'number' ? intent.pos - 1 : rest.length), 0, ...ids);
            handOrder.current = rest;
            setLayoutTick((t) => t + 1);
            return;
          }
        }
        // Breaking the rules: the table works like the sandbox, and each gesture becomes a direct change to the game
        if (st?.paused?.mine && st.view) {
          const zoneOf: Record<string, R.Zone> = { battlefield: 'Battlefield', hand: 'Hand', graveyard: 'Graveyard', exile: 'Exile', library: 'Library', command: 'Command' };
          const byId = (iid: string) => (/^\d+$/.test(iid) ? st.view!.cards.find((c) => c.id === Number(iid)) : undefined);
          const sideOf = sides(st.view.cards);
          const ops: R.EditOp[] = [];
          if (intent.type === 'move') {
            (intent.iids ?? []).forEach((iid: string, k: number) => {
              const c = byId(iid);
              const to = zoneOf[intent.zone as string];
              if (!c || !to) return;
              const pos = intent.positions?.[k] ?? (intent.x !== undefined && intent.y !== undefined ? { x: intent.x, y: intent.y } : null);
              if (to === 'Battlefield' && pos) placed.current.set(c.id, { x: pos.x, y: pos.y });
              if (c.zone === 'Battlefield' && to === 'Battlefield') {
                if (intent.seat !== undefined && intent.seat !== c.controller && intent.seat !== sideOf(c)) ops.push({ t: 'Control', card: c.id, player: intent.seat });
              } else ops.push({ t: 'Move', card: c.id, to, bottom: intent.pos === 'bottom', controller: to === 'Battlefield' ? (intent.seat ?? null) : null });
            });
            setLayoutTick((t) => t + 1);
          } else if (intent.type === 'tap') {
            for (const iid of intent.iids ?? []) {
              const c = byId(iid);
              if (c?.zone === 'Battlefield') ops.push({ t: 'Tap', card: c.id, tapped: intent.tapped ?? !c.tapped });
            }
          } else if (intent.type === 'untapAll') {
            for (const c of st.view.cards) if (c.zone === 'Battlefield' && c.controller === intent.seat && c.tapped) ops.push({ t: 'Tap', card: c.id, tapped: false });
          } else if (intent.type === 'counter') {
            for (const iid of intent.iids ?? []) {
              const c = byId(iid);
              if (c) ops.push({ t: 'Counters', card: c.id, kind: counterKindOf(intent.counter) as R.CounterKind, delta: intent.delta });
            }
          } else if (intent.type === 'life') {
            const cur = st.view.players[intent.seat]?.life ?? 0;
            const delta = intent.delta ?? (intent.set !== undefined ? intent.set - cur : 0);
            if (delta) ops.push({ t: 'Life', player: intent.seat, delta });
          } else if (intent.type === 'draw') {
            for (let k = 0; k < intent.n; k++) ops.push({ t: 'Draw', player: intent.seat });
          } else if (intent.type === 'shuffle') ops.push({ t: 'Shuffle', player: intent.seat });
          else if (intent.type === 'copy') {
            const c = byId(intent.iid);
            if (c) ops.push({ t: 'TokenCopy', card: c.id, player: c.zone === 'Battlefield' ? c.controller : st.seat, n: 1 });
          }
          else if (intent.type !== 'concede') return void useApp.getState().toast("That isn't available while breaking the rules: move cards, tap them, change counters and life, draw or shuffle.");
          if (intent.type !== 'concede') {
            if (gameId) void (async () => {
              for (const op of ops) await api.post(`api/rules/games/${gameId}/edit`, { op }).catch((e: Error) => useApp.getState().toast(e.message));
            })();
            return;
          }
        }
        // The table menu already asked "Concede this game?"; it is a game-level action, not a card gesture.
        if (intent.type === 'concede') {
          if (gameId) api.post(`api/rules/games/${gameId}/concede`).catch((e: Error) => setErr(e.message));
          return;
        }
        // Rearranging permanents is only bookkeeping on the table, never a game action: always allowed, anywhere.
        if (intent.type === 'move' && intent.zone === 'battlefield' && st?.view) {
          const ids = (intent.iids ?? []).map((i: string) => (/^\d+$/.test(i) ? Number(i) : -1));
          const onField = (id: number) => st.view!.cards.some((c) => c.id === id && c.zone === 'Battlefield');
          if (ids.length && ids.every(onField)) {
            // dragging one card out of a pile takes it out of the pile
            if (ids.length === 1) groups.current = groups.current.map((g) => g.filter((x) => x !== ids[0])).filter((g) => g.length > 1);
            ids.forEach((id: number, k: number) => {
              const pos = intent.positions?.[k] ?? (intent.x !== undefined && intent.y !== undefined ? { x: intent.x, y: intent.y } : null);
              if (pos) placed.current.set(id, { x: pos.x, y: pos.y });
            });
            setLayoutTick((t) => t + 1);
            return;
          }
        }
        // "Group identical" / "Stack into a pile": lay copies of a card on top of each other, slightly offset. Only table layout, never a game action.
        if ((intent.type === 'group' || intent.type === 'stack') && st?.view) {
          const ids: number[] = intent.type === 'stack' ? (intent.iids ?? []).map(Number) : [Number(intent.iid)];
          const first = st.view.cards.find((c) => c.id === ids[0]);
          if (first) {
            const group = intent.type === 'stack' ? ids : st.view.cards.filter((c) => c.zone === 'Battlefield' && c.controller === first.controller && c.name === first.name && c.token === first.token).map((c) => c.id);
            const base = placed.current.get(ids[0]) ?? { x: 0.5, y: 0.5 };
            group.forEach((id, k) => placed.current.set(id, { x: Math.min(1, base.x + k * PILE_DX), y: base.y }));
            groups.current = [...groups.current.map((g) => g.filter((x) => !group.includes(x))).filter((g) => g.length > 1), group];
            setLayoutTick((t) => t + 1);
          }
          return;
        }
        // The game is busy (the AI is acting, the server is working, moves are being shown): a play or a tap is queued and
        // tried at this player's next priority. The others may act first; Undo takes it back if that changes the plan.
        if ((intent.type === 'move' || intent.type === 'tap') && st?.view && !st.waiting && !st.over && !st.error && !fromQueue.current && st.status === 'running') {
          const id = Number(intent.iids?.[0]);
          const card = st.view.cards.find((c) => c.id === id);
          const fromHand = intent.type === 'move' && intent.zone === 'battlefield' && card?.zone === 'Hand' && card.owner === st.seat;
          const onField = intent.type === 'tap' && card?.zone === 'Battlefield' && card.controller === st.seat;
          if (card && (fromHand || onField) && !actionQueue.current.some((q) => q.card === id && q.intent.type === intent.type)) {
            actionQueue.current.push({ intent, card: id });
            if (fromHand && isPermanentCard(card)) {
              const pos = intent.positions?.[0] ?? AUTO_SLOT;
              optimistic.current.set(id, { x: pos.x, y: pos.y });
              if (pos.x >= 0) placed.current.set(id, { x: pos.x, y: pos.y });
            }
            setQueueTick((t) => t + 1);
            setLayoutTick((t) => t + 1);
            return;
          }
        }
        // while paying for a spell, lands are tapped by clicking them (one click = one tap), so a double-click's second tap is ignored
        if (st?.waiting?.kind.t === 'PayMana') return;
        // tapping a land for mana: floating mana in your pool
        if (intent.type === 'tap' && st?.view) {
          const id = Number(intent.iids?.[0]);
          const srcs = manaMenu(id);
          if (!srcs.length && Number.isFinite(id) && st.view.cards.some((c) => c.id === id && c.zone === 'Battlefield')) {
            // not a mana source right now: if the card has no other use either, say why it can't be tapped
            const other = (optionsOf(st.waiting ?? null, st.view ?? null) ?? []).some((o) => o.cardId === id && o.kind === 'ability');
            if (!other) return explainRef.current('tap', id, null, "That can't be tapped for mana right now");
          }
          if (srcs.length) {
            const distinct = srcs.filter((a, i) => srcs.findIndex((b) => b.text === a.text && b.s.ability === a.s.ability) === i);
            // a mana source with another ability that can be used right now (a Spacecraft's Station, an equip...) asks which one
            const others = (optionsOf(st.waiting ?? null, st.view ?? null) ?? [])
              .filter((o) => o.cardId === id && o.kind === 'ability' && !(st.waiting?.kind.t === 'Priority' && st.waiting.kind.actions[o.action]?.t === 'ActivateMana'))
              .filter((o, i, all) => all.findIndex((x) => x.label === o.label) === i);
            if (distinct.length === 1 && !others.length) return tapMana(distinct[0].s.card, distinct[0].s.ability, distinct[0].m);
            return useMenu.getState().open(mouse.current.x + 2, mouse.current.y + 2, () => [
              ...distinct.map(({ s, m, text }) => ({ kind: 'item' as const, label: `Add ${text}`, run: () => tapMana(s.card, s.ability, m) })),
              ...others.map((o) => ({ kind: 'item' as const, label: o.label, run: () => answerRef.current({ t: 'Priority', action: o.action }) })),
            ]);
          }
        }
        const opts = optionsOf(st?.waiting ?? null, st?.view ?? null);
        const g = gestureOption(intent as { type: string; iids?: string[]; zone?: string }, opts);
        // A card dragged from the hand onto the table is shown where it was dropped straight away; the engine then accepts the
        // play (the card stays) or it is illegal (the card flies back to the hand).
        const dropped: [number, { x: number; y: number }][] = [];
        if (intent.type === 'move' && intent.zone === 'battlefield' && st?.view) {
          (intent.iids ?? []).forEach((i: string, k: number) => {
            const card = /^\d+$/.test(i) ? st.view!.cards.find((c) => c.id === Number(i) && c.zone === 'Hand' && c.owner === st.seat) : undefined;
            if (!card) return;
            // a drag keeps its drop point; a double-click play of a permanent goes straight to the first free slot
            const pos = intent.positions?.[k] ?? (isPermanentCard(card) ? AUTO_SLOT : null);
            if (pos) dropped.push([card.id, { x: pos.x, y: pos.y }]);
          });
        }
        const showDropped = () => {
          dropped.forEach(([id, pos]) => {
            optimistic.current.set(id, pos);
            if (pos.x >= 0) placed.current.set(id, pos);
          });
          if (dropped.length) setLayoutTick((t) => t + 1);
        };
        const sendBack = () => {
          unmarkDragged(dropped.map(([id]) => String(id)));
          dropped.forEach(([id]) => {
            holding.current.delete(id);
            optimistic.current.delete(id);
            placed.current.delete(id);
          });
          setLayoutTick((t) => t + 1);
        };
        // a refused play: ask the engine why (it knows about timing, land drops, costs, "can't" effects)
        const refuse = (message: string) => {
          if (fromQueue.current) clearQueueRef.current('A queued play was not allowed');
          const id = Number((intent as { iids?: string[] }).iids?.[0]);
          if (message === "That card can't be played or used right now" && Number.isFinite(id)) explainRef.current('play', id, null, message);
          else if (message === "You can't do that right now") {
            const w = st?.waiting;
            const waitingFor = st?.view?.waiting_for;
            if (w?.kind.t === 'PayMana') useApp.getState().toast('Pay for the spell first: click your lands or mana pool, press Auto-pay, or cancel it.');
            else if (w) useApp.getState().toast('Answer the open question first (see the prompt on the table).');
            else if (waitingFor != null && waitingFor !== st?.seat) useApp.getState().toast(`It isn't your decision: the game is waiting for ${st?.names?.[waitingFor] ?? 'another player'}.`);
            else useApp.getState().toast("The game is between decisions: wait until it asks you for priority.");
          } else useApp.getState().toast(message);
        };
        // the card only lands on the table for a land or a spell (not for foretell, cycling and other abilities used from the hand)
        const lands = (index: number | null) => index !== null && opts?.find((o) => o.action === index)?.kind !== 'ability';
        if ('index' in g) {
          if (lands(g.index)) showDropped();
          answerRef.current({ t: 'Priority', action: g.index });
        } else if ('choices' in g)
          useMenu.getState().open(mouse.current.x + 2, mouse.current.y + 2, () =>
            g.choices.map((c) => ({
              kind: 'item' as const,
              label: c.label,
              run: () => {
                if (lands(c.index)) showDropped();
                answerRef.current({ t: 'Priority', action: c.index });
              },
            })),
          );
        else if (dropped.length) {
          // not a legal play: the card lands, the move is refused, and it flies back
          dropped.forEach(([id]) => holding.current.add(id));
          showDropped();
          refuse(g.message);
          setTimeout(sendBack, 650);
        } else refuse(g.message);
      },
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [gameId, state, layoutTick, ending, endPhase, endingLocal, detailedView, slot, queueTick, statusText, autoSort]);

  // A queued play is tried as soon as this player has priority again (one per question, in order)
  useEffect(() => {
    if (!state?.view || state.waiting?.kind.t !== 'Priority' || !actionQueue.current.length) return;
    const next = actionQueue.current.shift()!;
    setQueueTick((t) => t + 1);
    fromQueue.current = true;
    try {
      useTable.getState().send(next.intent);
    } finally {
      fromQueue.current = false;
    }
  }, [state]);
  useEffect(() => {
    if (!gameId) resetTable();
    return () => {
      resetTable();
      useTable.setState({ rules: null });
    };
  }, [gameId]);

  // The "auto-tap lands" setting goes to the engine when the game starts and whenever it changes
  const autoTap = useApp((s) => s.me?.prefs.autoTap ?? true);
  const runningNow = raw?.status === 'running';
  useEffect(() => {
    if (!gameId || !runningNow) return;
    api.post(`api/rules/games/${gameId}/autotap`, { on: autoTap }).catch(() => {});
  }, [gameId, runningNow, autoTap]);

  // Prompt shortcuts: Enter = the main button, 1-9 = the n-th choice, A = auto-pay
  useEffect(() => {
    if (!gameId) return;
    const down = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement | null;
      if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.tagName === 'SELECT' || t.isContentEditable)) return;
      if (e.ctrlKey || e.metaKey || e.altKey) return;
      const root = document.querySelector('[data-rules-prompt]');
      if (!root) return;
      const enabled = (sel: string) => [...root.querySelectorAll<HTMLButtonElement>(sel)].filter((b) => !b.disabled);
      let target: HTMLButtonElement | undefined;
      if (e.key === 'Enter') target = enabled('[data-primary]').at(-1);
      else if (/^[1-9]$/.test(e.key)) target = enabled('[data-option]')[Number(e.key) - 1];
      else if (e.key === 'a' || e.key === 'A') target = enabled('[data-autopay]')[0];
      if (!target) return;
      e.preventDefault();
      e.stopPropagation();
      target.click();
    };
    window.addEventListener('keydown', down, true);
    return () => window.removeEventListener('keydown', down, true);
  }, [gameId]);

  // "End turn" / "pass to phase" end when the engine gives us a question again that is not a plain priority at the target
  useEffect(() => {
    if (!state) return;
    if (state.over || state.error || (state.waiting && !state.passing)) {
      setEnding(false);
      setEndPhase(null);
    }
  }, [state]);

  // Dropped cards stay on the table while the engine works. Once the card has left the hand it is the engine's; if the engine is
  // asking for a new action and the card never left the hand, the play did not happen (cancelled payment, no mana...): send it back.
  useEffect(() => {
    if (!optimistic.current.size || !state?.view) return;
    let changed = false;
    const queued = new Set(actionQueue.current.map((q) => q.card));
    let reversed = false;
    for (const id of [...optimistic.current.keys()]) {
      if (holding.current.has(id) || queued.has(id)) continue;
      const card = state.view.cards.find((c) => c.id === id);
      if (!card) {
        optimistic.current.delete(id);
        placed.current.delete(id);
        changed = true;
      } else if (card.zone !== 'Hand') optimistic.current.delete(id);
      else if (state.waiting?.kind.t === 'Priority' || state.over || state.error) {
        unmarkDragged([String(id)]);
        optimistic.current.delete(id);
        placed.current.delete(id);
        changed = true;
        reversed = true;
      }
    }
    if (changed) setLayoutTick((t) => t + 1);
    // the play did not happen: what was queued after it may no longer make sense
    if (reversed) clearQueueRef.current('A play did not go through');
  }, [state]);

  if (!gameId)
    return (
      <div className="screen" style={{ alignItems: 'center', justifyContent: 'center', gap: 14 }}>
        <h1 style={{ fontSize: 26 }}>No rules game open</h1>
        <div style={{ color: 'var(--mu)', fontSize: 13.5, maxWidth: 460, textAlign: 'center' }}>
          Start one from New game (choose Rules enforced as the game type), or continue a running or saved game from Load game.
          {err ? ` ${err}` : ''}
        </div>
        <div className="row">
          <Link to="/" className="tb">
            Main menu
          </Link>
          <Link to="/games?tab=load" className="tb pri">
            Load game
          </Link>
        </div>
      </div>
    );

  const v = state?.view;
  const inPanel = detailedView && !!slot;
  const rightEdge = !compact && !detailedView && logOpen ? 404 : 14;
  const bar: React.CSSProperties = inPanel ? { display: 'flex', flexDirection: 'column', gap: 10 } : { position: 'fixed', right: rightEdge, top: 62, width: 400, maxWidth: 'calc(100vw - 28px)', maxHeight: '70vh', overflowY: 'auto', zIndex: 900, display: 'flex', flexDirection: 'column', gap: 10 };
  return (
    <>
      {tokenSeat !== null && state?.paused?.mine && (
        <TokenPicker
          seat={tokenSeat}
          name={state.names[tokenSeat] ?? 'this player'}
          onClose={() => setTokenSeat(null)}
          onPick={(name, n, tapped) => {
            setTokenSeat(null);
            if (gameId) api.post(`api/rules/games/${gameId}/edit`, { op: { t: 'CreateToken', player: tokenSeat, name, n, tapped } }).catch((e: Error) => useApp.getState().toast(e.message));
          }}
        />
      )}
      {state?.paused && (
        <div role="status" className="row" style={{ position: 'fixed', left: '50%', top: 56, transform: 'translateX(-50%)', zIndex: 950, gap: 12, padding: '10px 16px', borderRadius: 10, background: 'var(--surface3)', border: '2px solid var(--warn, #d9a441)', boxShadow: '0 10px 30px rgba(0,0,0,.5)', fontSize: 14, maxWidth: 'calc(100vw - 28px)' }}>
          <span>
            <b>Rules paused.</b> {state.paused.mine ? 'Fix the game by hand: drag cards to any zone, tap them, change counters and life (right-click for more).' : `${state.paused.name} is fixing the game by hand; it carries on when they finish.`}
          </span>
          {state.paused.mine && (
            <button className="tb sm pri" onClick={() => void toggleBreakRef.current()}>
              Finish
            </button>
          )}
        </div>
      )}
      {v ? (
        <TableView onExit={backToMenu} title="Rules game" />
      ) : (
        <div className="screen">
          <header className="page-header">
            <button className="tb" onClick={leave}><Chevron />Leave game</button>
          </header>
          <div style={{ padding: 28, color: 'var(--mu)' }}>{state?.status === 'waiting' ? (state.versus === 'pvp' ? 'Waiting for players to join…' : 'Waiting…') : 'Starting the engine…'}</div>
        </div>
      )}
      {(() => {
        const stack = v?.stack ?? [];
        const stackBox = stack.length > 0 && (
          <div className="card-box" style={{ padding: 12, fontSize: 13 }}>
            <div className="display" style={{ fontSize: 15 }}>Stack</div>
            {[...stack].reverse().map((x, i) => (
              <button
                key={x.id}
                className="stack-item"
                onClick={() => (passingItem === x.id ? cancelPassRef.current() : passRef.current({ stackItem: x.id }))}
                onContextMenu={(e) => {
                  e.preventDefault();
                  stackMenuRef.current(e, x);
                }}
                title={i === 0 ? 'On top: resolves next' : passingItem === x.id ? 'Passing until this is on top — click to stop' : 'Click: pass priority until this is on top of the stack (right-click for more)'}
                style={{ all: 'unset', boxSizing: 'border-box', display: 'block', width: '100%', marginTop: 4, padding: '4px 6px', borderRadius: 7, cursor: 'pointer', outline: passingItem === x.id ? '2px dashed var(--acc)' : undefined }}
              >
                <b>{x.name}</b> <span style={{ color: 'var(--mu)' }}>({state?.names[x.controller] ?? 'player'})</span>
                {x.text && <div title={x.text} style={{ color: 'var(--tx3)', fontSize: 12, display: '-webkit-box', WebkitLineClamp: 4, WebkitBoxOrient: 'vertical', overflow: 'hidden' }}>{x.text}</div>}
              </button>
            ))}
          </div>
        );
        const notesBox = state?.notices && state.notices.length > 0 && (
          <div style={{ fontSize: 14, color: 'var(--mu)' }}>
            {state.notices.slice(-6).map((n, i) => (
              <div key={i}>ⓘ {n}</div>
            ))}
          </div>
        );
        const winner = state?.over?.t === 'Winner' ? (state.names[state.over.player] ?? 'A player') : null;
        const passiveEvents = (
          <>
            {state?.undoRequest && !state.undoRequest.mine && (
              <div className="card-box" style={{ padding: 14 }}>
                Your opponent asks to undo the last {state.undoRequest.steps} move{state.undoRequest.steps === 1 ? '' : 's'}.{' '}
                <button className="tb pri" onClick={() => respondUndo(true)}>Accept</button>{' '}
                <button className="tb" onClick={() => respondUndo(false)}>Decline</button>
              </div>
            )}
            {state?.undoRequest?.mine && <div className="card-box" style={{ padding: 12, color: 'var(--mu)' }}>Waiting for your opponent to accept the undo…</div>}
            {state?.error && <div className="card-box" style={{ padding: 12, color: 'var(--bad, #e4787a)' }}>Engine error: {state.error}</div>}
            {state?.out && !state.over && (
              <div className="card-box" style={{ padding: 12, fontSize: 14 }}>
                You're out of the game. The AIs play on so you can watch (Settings → slow down AI actions to follow along). Undo to go back, or end it from the menu.
              </div>
            )}
            {state?.over && (
              <div className="card-box" style={{ padding: 14 }}>
                <h2 className="display" style={{ margin: 0, fontSize: 20 }}>
                  Game over — {winner ? `${winner} wins` : 'draw'} (round {v ? Math.ceil(v.turn / Math.max(1, state.names.length)) : 0})
                </h2>
              </div>
            )}
          </>
        );
        const passive = (
          <>
            {passiveEvents}
            {stackBox}
            {notesBox}
          </>
        );
        const priority = state?.waiting?.kind.t === 'Priority';
        const hint = priority && (
          <div className="card-box" style={{ padding: '10px 14px', fontSize: 14, color: 'var(--tx3)' }}>
            <b style={{ color: 'var(--tx)' }}>Your priority.</b>{' '}
            {compact
              ? 'Tap a card in your hand and press Play (or drag it) to cast it, tap a land and press Tap to add its mana, or press Pass. End turn is in the ☰ menu.'
              : 'Drag or double-click a card in your hand to play or cast it, double-click a land to add its mana to your pool, or press Pass priority / End turn.'}
          </div>
        );
        const decision = state?.waiting && v && !state.paused && state.waiting.kind.t !== 'Priority' && (
          <div data-rules-prompt className="col" style={{ gap: 6 }}>
            {v.decision_about && state.waiting.kind.t !== 'Mulligan' && <AboutLine about={v.decision_about} />}
            <PromptPanel d={state.waiting} v={v} names={state.names} images={state.images} onAnswer={answer} onExplain={(kind, card, other) => explainRef.current(kind, card, other, "That can't be done right now")} onPreviewBlocks={(blocks) => api.post<R.BlockPreview[]>(`api/rules/games/${gameId}/preview-blocks`, { blocks })} />
            <RememberRow d={state.waiting} v={v} onRemember={(r) => rememberRef.current(r)} />
          </div>
        );
        const errMsg = err && <p style={{ color: 'var(--bad, #e4787a)' }}>{err}</p>;
        // On a computer the question floats in the middle of the screen, can be dragged, and remembers where it was put for this game
        // the result of the game, in the middle of the screen until closed
        const overBox = state?.over && !overClosed && (
          <div role="dialog" aria-label="Game over" style={{ position: 'fixed', left: '50%', top: '50%', transform: 'translate(-50%, -50%)', zIndex: 960, padding: '20px 26px', borderRadius: 14, background: 'var(--surface3)', border: '1px solid var(--acc)', boxShadow: 'var(--shadow)', textAlign: 'center', maxWidth: 'calc(100vw - 32px)' }}>
            <h2 className="display" style={{ margin: '0 0 6px', fontSize: 28 }}>
              {state.over.t === 'Winner' ? (state.over.player === state.seat ? 'You won!' : state.names.length > 2 ? `${winner} wins` : 'You lost') : 'Draw'}
            </h2>
            <div style={{ color: 'var(--mu)', fontSize: 13, marginBottom: 12 }}>
              {state.over.t === 'Winner' && state.over.player !== state.seat && state.names.length > 2 ? 'You lost. ' : ''}Game over after round {v ? Math.ceil(v.turn / Math.max(1, state.names.length)) : 0}.
            </div>
            <div className="row" style={{ justifyContent: 'center', flexWrap: 'wrap', gap: 8 }}>
              {gameId && <AfterGameButtons gameId={gameId} />}
              <button className="tb" onClick={() => setOverClosed(true)} title="Stay and look at the board">Close</button>
            </div>
          </div>
        );
        const w = state?.waiting;
        // declaring attackers or blockers: a small box over the top opponent's hand (clear of the name panels, which are click targets), kept apart from where other questions are dragged, so the board stays clickable
        const combat = w?.kind.t === 'Attackers' || w?.kind.t === 'Blockers';
        const boxWidth = combat ? 340 : w && v && isGridPick(w, v, state.names) ? 880 : w?.kind.t === 'Mulligan' && w.kind.peek.length > 0 ? 540 : 420;
        const floating =
          !compact && decision ? (
            <FloatingBox storeKey={`aetherhub.rules${combat ? 'Combat' : 'Prompt'}.${gameId}`} width={boxWidth} at={combat ? { left: 'calc(50% + 130px)', top: 54 } : undefined}>
              {decision}
            </FloatingBox>
          ) : null;
        if (inPanel)
          return (
            <>
              {floating}
              {overBox}
              {createPortal(
                <div style={bar}>
                  {passive}
                  {compact ? decision : null}
                  {hint}
                  {errMsg}
                </div>,
                slot!,
              )}
            </>
          );
        // Detailed view showing another tab (no Game slot): only the question floats; the Game tab gets a dot instead of chips over the panel.
        if (detailedView)
          return (
            <>
              {floating}
              {overBox}
              {errMsg && <div style={{ position: 'fixed', left: '50%', bottom: 90, transform: 'translateX(-50%)', zIndex: 900 }}>{errMsg}</div>}
            </>
          );
        // Panel off: decisions still float over the table; everything else collapses into three indicators (priority, stack, messages).
        const noteCount = (state?.notices?.length ?? 0) + (state?.undoRequest ? 1 : 0) + (state?.over || state?.error ? 1 : 0);
        return (
          <>
            {v && (
              <StatusChips
                priority={!!priority}
                stackCount={stack.length}
                stackEmpty={stack.length === 0}
                noteCount={noteCount}
                open={chipOpen}
                setOpen={setChipOpen}
                right={rightEdge}
                onPanel={() => {
                  if (compact) return setMsgOpen(!msgOpen);
                  void useApp.getState().setPrefs({ panel: 'detailed' });
                  useTable.setState({ panelTab: 'game' });
                }}
              />
            )}
            {floating}
            {overBox}
            {v && (chipOpen === 'stack' || chipOpen === 'notes' || state?.undoRequest || state?.error || state?.over) && (
              <div style={{ ...bar, top: 100, maxHeight: '55vh' }}>
                {state?.undoRequest || state?.error || state?.over ? passiveEvents : null}
                {chipOpen === 'stack' && (stackBox || <div className="card-box" style={{ padding: 12, fontSize: 13, color: 'var(--mu)' }}>The stack is empty.</div>)}
                {chipOpen === 'notes' && (notesBox || <div className="card-box" style={{ padding: 12, fontSize: 13, color: 'var(--mu)' }}>No messages.</div>)}
              </div>
            )}
            {compact && decision && promptHidden && (
              <button className="tb sm pri" onClick={() => setPromptHidden(false)} style={{ position: 'fixed', right: 14, top: 100, zIndex: 900 }}>
                Show prompt ▾
              </button>
            )}
            {((compact && decision) || errMsg || (compact && msgOpen)) && !(compact && promptHidden && !msgOpen) && (
              <div style={{ ...bar, top: 100, ...(compact ? { maxHeight: '55vh' } : {}) }}>
                {compact && decision && (
                  <button className="tb sm" onClick={() => setPromptHidden(true)} style={{ alignSelf: 'flex-end', flexShrink: 0 }} title="Hide this box to see the table; the prompt stays open">
                    Hide ▴
                  </button>
                )}
                {compact && decision}
                {errMsg}
                {compact && msgOpen && (
                  <>
                    {passive}
                    {hint}
                  </>
                )}
              </div>
            )}
          </>
        );
      })()}
    </>
  );
}

/** Which spell or ability a question belongs to, with its text, so a card with several abilities is unambiguous. */
function AboutLine({ about }: { about: R.DecisionAbout }) {
  return (
    <div className="card-box" style={{ padding: '7px 12px', fontSize: 12.5, color: 'var(--tx3)', maxWidth: 520 }}>
      <b style={{ color: 'var(--tx)' }}>{about.name}</b>
      {about.text && (
        <>
          {about.is_spell ? ' — ' : ': '}
          <RichLabel text={about.text.replace(/\n/g, ' ')} />
        </>
      )}
    </div>
  );
}
