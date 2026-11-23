import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useEffect, useRef, useState } from 'react';
import { Link, useNavigate, useParams, useSearchParams } from 'react-router-dom';
import type { CardDef, Deck, GameState, Intent, LogView, MulliganRule, Section, SeatInput } from '@aetherhub/shared';
import { applyIntent, createGame, DEFAULT_SETTINGS, formatInfo, IntentError, logFor, PLAYER_COLORS, resolveIntent, viewFor, type Rng } from '@aetherhub/shared';
import { api } from '../../lib/api';
import { errMsg, useApp } from '../../lib/store';
import { usePreview } from '../../ui/ui';
import { handleNotice, onNewLog, refreshMenus } from './driver';
import { resetTable, useTable, type SoloOpts } from './store';
import { TableView } from './TableView';

const rng: Rng = {
  int: (n) => {
    if (n <= 1) return 0;
    const a = new Uint32Array(1);
    crypto.getRandomValues(a);
    return a[0] % n;
  },
};

export interface LocalConfig {
  title: string;
  deckIds: number[];
  format: string;
  life: number;
  mulligan: MulliganRule;
  /** Continue a saved solo game instead of starting fresh. */
  resume?: SoloSaveData;
}

/** What a saved solo game holds (see the server's /api/solo/saves). */
export interface SoloSaveData {
  id: number;
  name: string;
  state: GameState;
  log: LogView[];
  solo: SoloOpts | null;
}

const AUTOSAVE_MS = 2500;

/** Builds seat input from a deck, folding image overrides into variant card keys. */
function seatFromDeck(deck: Deck, i: number, name: string, color: string, prefs: { playmat: number | null; autoUntap: boolean; autoDraw: boolean }, defs: Record<string, CardDef>): SeatInput {
  Object.assign(defs, deck.defs);
  const cards: SeatInput['cards'] = deck.cards.map((c) => {
    if (!c.frontAsset && !c.backAsset) return { def: c.def, section: c.section as Section, qty: c.qty };
    const key = `${c.def}|o${c.frontAsset || 0}.${c.backAsset || 0}`;
    const img = defs[c.def].img.slice();
    if (c.frontAsset) img[0] = 'a/' + c.frontAsset;
    if (c.backAsset) img[1] = 'a/' + c.backAsset;
    defs[key] = { ...defs[c.def], key, img };
    return { def: key, section: c.section as Section, qty: c.qty };
  });
  return {
    userId: -1 - i,
    name,
    color,
    deckId: deck.id,
    deckName: deck.name,
    sleeve: deck.sleeveAsset ? 'a/' + deck.sleeveAsset : null,
    playmat: i === 0 && prefs.playmat ? 'a/' + prefs.playmat : null,
    autoUntap: prefs.autoUntap,
    autoDraw: prefs.autoDraw,
    cards,
  };
}

/**
 * A game that runs entirely in the browser with the shared engine. One person controls every seat;
 * nothing is saved and the server's one-active-game slot is untouched.
 */
function LocalGame({ config, onExit }: { config: LocalConfig; onExit: () => void }) {
  const me = useApp((s) => s.me)!;
  const [decks, setDecks] = useState<Deck[] | null>(null);
  const [err, setErr] = useState('');
  const state = useRef<GameState | null>(null);
  const undo = useRef<{ state: GameState; seat: number }[]>([]);
  const logId = useRef(1);

  const loadDecks = async () => {
    const ids = Array.from(new Set(config.deckIds));
    const list = await Promise.all(ids.map((id) => api.get<Deck>(`api/decks/${id}`)));
    return config.deckIds.map((id) => list.find((d) => d.id === id)!);
  };
  useEffect(() => {
    // A saved game carries everything it needs; its decks are only loaded if you restart it.
    if (config.resume) return setDecks([]);
    loadDecks()
      .then(setDecks)
      .catch((e) => setErr(errMsg(e)));
  }, [config]);

  useEffect(() => {
    if (!decks) return;
    const toast = useApp.getState().toast;
    const solo = config.resume ? config.resume.state.seats.length > 1 : decks.length > 1;
    let fresh = decks;
    const current = () => useTable.getState().solo?.seat ?? 0;
    const publish = () => {
      const s = state.current!;
      const seat = current();
      const v = viewFor(s, seat);
      // "Show all hands": the controlled seat's perspective, but every card face visible.
      const all = useTable.getState().solo?.revealAll;
      useTable.setState({ view: all ? { ...v, cards: s.cards, defs: s.defs } : v, undoable: undo.current.length });
      refreshMenus();
    };
    const pushLogs = (drafts: ReturnType<typeof applyIntent>['logs'], actor: number) => {
      const s = state.current!;
      const entries: LogView[] = drafts.map((d) => logFor({ ...d, id: logId.current++, at: Date.now(), actor, actorName: s.seats[actor]?.name ?? me.username }, 'spectator', true)!);
      onNewLog(entries);
    };
    // ---------------------------------------------------------------- saving (solo only)
    let saveTimer: number | undefined;
    const setSave = (patch: Partial<NonNullable<ReturnType<typeof useTable.getState>['soloSave']>>) => {
      const cur = useTable.getState().soloSave;
      if (cur) useTable.setState({ soloSave: { ...cur, ...patch } });
    };
    const defaultName = () => {
      const s = state.current!;
      const names = Array.from(new Set(s.seats.map((x) => x.deckName)));
      const when = new Date().toLocaleString(undefined, { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' });
      return `${names.join(' vs ')} · ${when}`.slice(0, 80);
    };
    const save = async (opts: { asNew?: boolean; name?: string } = {}): Promise<boolean> => {
      const cur = useTable.getState().soloSave;
      if (!state.current || !cur) return false;
      window.clearTimeout(saveTimer);
      const name = (opts.name || cur.name || defaultName()).trim().slice(0, 80);
      const body = { name, state: state.current, log: useTable.getState().log.slice(-1500), solo: useTable.getState().solo };
      setSave({ saving: true });
      try {
        const r = cur.id != null && !opts.asNew ? await api.put<{ id: number; updatedAt: number }>(`api/solo/saves/${cur.id}`, body) : await api.post<{ id: number; updatedAt: number }>('api/solo/saves', body);
        setSave({ id: r.id, name, savedAt: r.updatedAt, saving: false, dirty: false });
        return true;
      } catch (e) {
        setSave({ saving: false });
        toast('Could not save: ' + errMsg(e));
        return false;
      }
    };
    /** After any change: mark unsaved, and autosave games that already have a save slot. */
    const changed = () => {
      const cur = useTable.getState().soloSave;
      if (!cur) return;
      if (!cur.dirty) setSave({ dirty: true });
      if (cur.id != null) {
        window.clearTimeout(saveTimer);
        saveTimer = window.setTimeout(() => void save(), AUTOSAVE_MS);
      }
    };

    const restore = (r: SoloSaveData) => {
      state.current = structuredClone(r.state);
      undo.current = [];
      logId.current = r.log.reduce((m, e) => Math.max(m, e.id), 0) + 1;
      const n = r.state.seats.length;
      const so = r.solo && r.solo.seat < n ? r.solo : { seat: r.state.turn.active, count: n, follow: true, revealAll: true };
      useTable.setState({ log: r.log, solo: n > 1 ? so : null, soloSave: { id: r.id, name: r.name, savedAt: Date.now(), saving: false, dirty: false, save } });
      pushLogs([{ kind: 'system', text: `Loaded saved solo game “${r.name}”.` }], -1);
      publish();
    };

    const build = async (restart = false) => {
      if (!restart && config.resume) return restore(config.resume);
      if (!fresh.length) {
        try {
          fresh = await loadDecks();
        } catch (e) {
          return toast("Can't restart: " + errMsg(e));
        }
      }
      const decks = fresh;
      const defs: Record<string, CardDef> = {};
      const seats = decks.map((d, i) =>
        seatFromDeck(d, i, solo ? `P${i + 1} · ${d.name}` : me.username, i === 0 ? me.prefs.playerColor : PLAYER_COLORS.filter((c) => c !== me.prefs.playerColor)[(i - 1) % 7], { playmat: me.prefs.playmat, autoUntap: me.prefs.autoUntap, autoDraw: me.prefs.autoDraw }, defs),
      );
      state.current = createGame({
        id: 'local',
        settings: { ...DEFAULT_SETTINGS, name: config.title, format: config.format, life: config.life, mulligan: config.mulligan, maxPlayers: decks.length, spectators: false },
        seats,
        defs,
        rng,
        now: Date.now(),
      });
      undo.current = [];
      // Restarting keeps the save slot, so the next save overwrites it.
      const slot = useTable.getState().soloSave;
      useTable.setState({
        log: [],
        solo: solo ? { seat: 0, count: decks.length, follow: true, revealAll: true } : null,
        soloSave: solo ? { id: slot?.id ?? null, name: slot?.name ?? '', savedAt: slot?.savedAt ?? null, saving: false, dirty: !!slot?.id, save } : null,
      });
      pushLogs([{ kind: 'system', text: solo ? `Solo game started with ${decks.length} seats — you control everyone. Save it from the Settings menu to continue later.` : `Playtest started with ${decks[0].name}. Nothing here is saved.` }], -1);
      publish();
    };
    const send = async (intent: Intent) => {
      const s = state.current;
      if (!s) return;
      if (intent.type === 'chat' && intent.text === '__undo__') {
        const prev = undo.current.pop();
        if (!prev) return toast('Nothing to undo');
        state.current = prev.state;
        pushLogs([{ kind: 'act', text: 'rewound the last action' }], prev.seat);
        publish();
        changed();
        return;
      }
      const actor = current();
      const ctx = { actor, actorName: s.seats[actor]?.name ?? me.username, now: Date.now() };
      try {
        const resolved = resolveIntent(s, ctx, intent, rng);
        if (intent.type === 'createToken' && intent.def && !s.defs[intent.def]) resolved.defs = await api.post<Record<string, CardDef>>('api/cards/defs', { keys: [intent.def] });
        const next = structuredClone(state.current!);
        const res = applyIntent(next, ctx, intent, resolved);
        if (intent.type !== 'chat') undo.current = [...undo.current.slice(-120), { state: state.current!, seat: actor }];
        state.current = next;
        // Follow the turn: after passing, take control of the new active player.
        const so = useTable.getState().solo;
        if (so && so.follow && intent.type === 'passTurn') useTable.setState({ solo: { ...so, seat: next.turn.active } });
        // Mulligans: once a seat keeps, hand control to the next seat that still has to decide
        // (and to the starting player when everyone has kept), so the game never waits on nobody.
        if (so && intent.type === 'keep') {
          const n = next.seats.length;
          const waiting = Array.from({ length: n - 1 }, (_, k) => (actor + 1 + k) % n).find((i) => next.seats[i].mulligan && !next.seats[i].mulligan!.kept && !next.seats[i].conceded);
          const seat = next.stage === 'mulligan' && waiting != null ? waiting : next.stage !== 'mulligan' ? next.turn.active : so.seat;
          if (seat !== so.seat) {
            useTable.setState({ solo: { ...so, seat }, sel: [], mullBottom: null });
            if (next.stage === 'mulligan') toast(`Now deciding for ${next.seats[seat].name}`);
          }
        }
        publish();
        pushLogs(res.logs, actor);
        changed();
        for (const n of res.notices) {
          if (n.kind === 'yourTurn' && solo) continue;
          if (n.kind === 'cards') {
            const cards = Object.fromEntries(n.iids.map((i) => [i, next.cards[i]]));
            const cdefs = Object.fromEntries(n.iids.map((i) => [next.cards[i].def, next.defs[next.cards[i].def]]));
            handleNotice({ ...n, to: typeof n.to === 'number' ? current() : n.to } as typeof n, cards, cdefs);
          } else if ('to' in n && typeof n.to === 'number' && n.kind !== 'handRequest') handleNotice({ ...n, to: current() } as typeof n);
          else handleNotice(n);
        }
      } catch (e) {
        toast(e instanceof IntentError ? e.message : errMsg(e));
      }
    };
    resetTable();
    useTable.setState({ mode: 'local', host: true, hostSeat: 0, send: (i) => void send(i), restart: () => void build(true), republish: publish });
    void build();
    // Closing or reloading the tab: flush a pending autosave (best effort) and warn about unsaved games.
    const beforeUnload = (e: BeforeUnloadEvent) => {
      const cur = useTable.getState().soloSave;
      if (!cur?.dirty) return;
      if (cur.id != null) void save();
      else e.preventDefault();
    };
    window.addEventListener('beforeunload', beforeUnload);
    return () => {
      window.removeEventListener('beforeunload', beforeUnload);
      // Leaving the table: a saved game gets its last changes written straight away.
      if (useTable.getState().soloSave?.dirty && useTable.getState().soloSave?.id != null) void save();
      window.clearTimeout(saveTimer);
      usePreview.getState().hide(true);
      resetTable();
      useTable.setState({ mode: 'online', restart: null, solo: null, republish: null, soloSave: null });
    };
  }, [decks]);

  if (err) return <div style={{ padding: 40 }}>{err}</div>;
  return <TableView onExit={onExit} title={config.title} />;
}

/** Single-deck playtest from the deck editor. */
export default function Playtest() {
  const { id } = useParams();
  const nav = useNavigate();
  const deck = useQuery({ queryKey: ['deck', Number(id)], queryFn: () => api.get<Deck>(`api/decks/${id}`) });
  const [config, setConfig] = useState<LocalConfig | null>(null);
  useEffect(() => {
    if (deck.data && !config) setConfig({ title: `Playtest · ${deck.data.name}`, deckIds: [deck.data.id], format: deck.data.format, life: formatInfo(deck.data.format).life, mulligan: 'london' });
  }, [deck.data, config]);
  if (deck.isError) return <div style={{ padding: 40 }}>Deck not found.</div>;
  if (!config) return <div style={{ padding: 40, color: 'var(--mu)' }}>Loading…</div>;
  return <LocalGame config={config} onExit={() => nav(`/decks/${id}`)} />;
}

/**
 * A solo game saved in this browser: /solo?save=<id> continues it. New solo games
 * are started from New game by taking several seats.
 */
export function SoloGame() {
  const [config, setConfig] = useState<LocalConfig | null>(null);
  const [params, setParams] = useSearchParams();
  const saveId = params.get('save');
  const qc = useQueryClient();
  const [loadErr, setLoadErr] = useState('');
  useEffect(() => {
    if (!saveId) return;
    setLoadErr('');
    api
      .get<SoloSaveData & { updatedAt: number }>(`api/solo/saves/${saveId}`)
      .then((r) =>
        setConfig({
          title: r.name,
          deckIds: r.state.seats.map((x) => x.deckId ?? 0),
          format: r.state.settings.format,
          life: r.state.settings.life,
          mulligan: r.state.settings.mulligan,
          resume: r,
        }),
      )
      .catch((e) => setLoadErr(errMsg(e)));
  }, [saveId]);

  if (config)
    return (
      <LocalGame
        key={saveId ?? 'new'}
        config={config}
        onExit={() => {
          setConfig(null);
          if (saveId) setParams({}, { replace: true });
          // The list may have gained a newer autosave.
          setTimeout(() => qc.invalidateQueries({ queryKey: ['solo-saves'] }), 400);
        }}
      />
    );
  if (saveId && !loadErr) return <div style={{ padding: 40, color: 'var(--mu)' }}>Loading saved game…</div>;
  return (
    <div className="screen" style={{ alignItems: 'center', justifyContent: 'center', gap: 14 }}>
      <h1 style={{ fontSize: 26 }}>Solo games are played from New game</h1>
      <div style={{ color: 'var(--mu)', fontSize: 13.5, maxWidth: 480, textAlign: 'center' }}>
        Open a lobby and take several seats yourself (sandbox or rules enforced). Solo games saved in this browser are under Load game.
        {loadErr ? ` ${loadErr}` : ''}
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
}
