import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useEffect, useMemo, useState } from 'react';
import { Link, useSearchParams } from 'react-router-dom';
import type { CardDef, Deck, DeckCard, GroupBy, SortBy } from '@aetherhub/shared';
import { deckStats, groupCards } from '@aetherhub/shared';
import { api } from '../lib/api';
import { errMsg, useApp } from '../lib/store';
import { Select } from '../ui/Select';
import { Chevron, ManaSymbol } from '../ui/ui';
import { CardGroupsView, GROUP_OPTIONS, SidePanel, SORT_OPTIONS, StatsSections, useDeckPreview, ViewControls, VIEW_CSS } from './deck/deckViews';

interface PoolCardV {
  key: string;
  name: string;
  rarity: string;
  cmc: number;
  colors: string;
  typeLine: string;
  manaCost: string;
}
interface DraftState {
  id: string;
  lobbyId: string;
  mode: 'draft' | 'sealed';
  phase: 'waiting' | 'drafting' | 'building';
  sets: { code: string; name: string }[];
  round: number;
  pickNo: number;
  timed: boolean;
  timeLeft: number | null;
  seats: { name: string; human: boolean; picked: boolean; pool: number; built: boolean }[];
  seat: number;
  mySeats: number[];
  passing: 'left' | 'right';
  pack: PoolCardV[];
  picked: boolean;
  waitingOn: string[];
  pool: PoolCardV[];
  deckId: number | null;
  defs: Record<string, CardDef>;
}

const BASICS: [string, string][] = [
  ['Plains', 'W'],
  ['Island', 'U'],
  ['Swamp', 'B'],
  ['Mountain', 'R'],
  ['Forest', 'G'],
];
const defOf = (s: DraftState, c: PoolCardV) => s.defs[c.key] ?? s.defs[c.key.replace(/^p:/, '')] ?? null;
/** Grouping and sorting offered for a pool (tags and deck sections don't apply; "Date added" is pick order). */
const DRAFT_GROUPS = GROUP_OPTIONS.filter(([v]) => v !== 'tag' && v !== 'section');
const DRAFT_SORTS = SORT_OPTIONS.map(([v, l]): [SortBy, string] => [v, v === 'added' ? 'Latest pick' : l]);

/** Pool cards as deck cards for the shared views: copies of one card are merged (id = the first pool index), and
 *  `idx` maps that id back to the pool indices of all copies. `addedAt` is the pick number. */
function asDeckCards(list: { c: PoolCardV; i: number }[]) {
  const byKey = new Map<string, DeckCard>();
  const idx = new Map<number, number[]>();
  for (const { c, i } of list) {
    const have = byKey.get(c.key);
    if (have) {
      have.qty++;
      have.addedAt = Math.max(have.addedAt, i);
      idx.get(have.id)!.push(i);
    } else {
      byKey.set(c.key, { id: i, section: 'main', def: c.key, qty: 1, foil: false, tags: [], addedAt: i });
      idx.set(i, [i]);
    }
  }
  return { cards: [...byKey.values()], idx };
}

/** Card defs by pool key; a card the server sent no def for gets a minimal one from the pool entry. */
function useDefs(s: DraftState) {
  return useMemo(() => {
    const out: Record<string, CardDef> = {};
    for (const c of [...s.pool, ...s.pack])
      out[c.key] ??= defOf(s, c) ?? { key: c.key, name: c.name, layout: 'normal', manaCost: c.manaCost, cmc: c.cmc, typeLine: c.typeLine, oracleText: '', colors: [...c.colors], colorIdentity: [...c.colors], rarity: c.rarity, img: [] };
    return out;
  }, [s]);
}

/** A draft or sealed event's draft and deck building (/draft?id=…). The event itself lives in its lobby. */
export default function Draft() {
  const [params, setParams] = useSearchParams();
  const id = params.get('id');
  const seatParam = params.get('seat');
  const lobbyRev = useApp((s) => (s.lobby && s.lobby.draftId === id ? s.lobby.draftRev : null));
  const [groupBy, setGroupBy] = useState<GroupBy>('mv');
  const [sortBy, setSortBy] = useState<SortBy>('color');
  useDeckPreview();
  const q = useQuery({
    queryKey: ['draft', id, seatParam, lobbyRev],
    queryFn: () => api.get<DraftState>(`api/draft/games/${id}${seatParam != null ? `?seat=${seatParam}` : ''}`),
    enabled: !!id,
    // the lobby pushes every change; this only covers a missed one
    refetchInterval: (query) => (query.state.data?.phase === 'drafting' ? 4000 : false),
    retry: false,
  });
  const s = q.data;
  if (!id)
    return (
      <div className="screen" style={{ alignItems: 'center', justifyContent: 'center', gap: 14 }}>
        <h1 style={{ fontSize: 26 }}>Draft and sealed are played from New game</h1>
        <div style={{ color: 'var(--mu)', fontSize: 13.5 }}>Open a lobby and choose Draft or Sealed as the game type.</div>
        <Link to="/" className="tb">
          Main menu
        </Link>
      </div>
    );
  const title = s ? `${s.mode === 'draft' ? 'Draft' : 'Sealed'} · ${[...new Set(s.sets.map((x) => x.name))].join(' / ')}` : 'Draft';
  const view = { groupBy, sortBy };
  return (
    <div className="screen">
      <style>{VIEW_CSS}</style>
      <header className="page-header" style={{ gap: 16, flexWrap: 'wrap' }}>
        <Link to={s ? `/lobby/${s.lobbyId}` : '/'} className="tb" aria-label="Back to the lobby">
          <Chevron />
          Lobby
        </Link>
        <h1 style={{ fontSize: 24, margin: 0 }}>{title}</h1>
        <div className="grow" />
        {s && s.mySeats.length > 1 && (
          <Select
            aria-label="Drafting as"
            value={s.seat}
            onChange={(v) => setParams({ id, seat: String(v) })}
            style={{ height: 30, fontSize: 12 }}
            options={s.mySeats.map((i) => ({ value: i, label: `As ${s.seats[i].name}${s.phase === 'drafting' && !s.seats[i].picked ? ' · to pick' : ''}` }))}
          />
        )}
      </header>
      {(s?.phase === 'drafting' || s?.phase === 'building') && (
        <div className="row" style={{ minHeight: 52, flexShrink: 0, padding: '8px 24px', gap: 10, flexWrap: 'wrap' }}>
          <div className="grow" />
          <ViewControls groupBy={groupBy} onGroupBy={setGroupBy} sortBy={sortBy} onSortBy={setSortBy} groupOptions={DRAFT_GROUPS} sortOptions={DRAFT_SORTS} />
        </div>
      )}
      {s?.phase === 'drafting' ? (
        <Picking s={s} view={view} />
      ) : s?.phase === 'building' ? (
        <Build key={`${s.id}:${s.seat}`} s={s} view={view} />
      ) : (
        <Body>
          {q.error && <div className="warnbox">{errMsg(q.error)}</div>}
          {!s && !q.error && <div style={{ color: 'var(--mu)' }}>Loading…</div>}
        </Body>
      )}
    </div>
  );
}

/** The scrolling page body, with the stats side panel on the right when given. */
function Body({ children, panel }: { children: React.ReactNode; panel?: React.ReactNode }) {
  return (
    <div style={{ flexGrow: 1, display: 'flex', minHeight: 0, borderTop: '1px solid var(--line)' }}>
      <div className="lg" style={{ flexGrow: 1, overflowY: 'auto', minWidth: 0 }}>
        <div className="col" style={{ padding: '18px 24px 60px', gap: 16 }}>
          {children}
        </div>
      </div>
      {panel && <SidePanel>{panel}</SidePanel>}
    </div>
  );
}

interface ViewOpts {
  groupBy: GroupBy;
  sortBy: SortBy;
}

function Countdown({ ms }: { ms: number }) {
  const [end] = useState(() => Date.now() + ms);
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 250);
    return () => clearInterval(t);
  }, []);
  const left = Math.max(0, Math.ceil((end - now) / 1000));
  return (
    <span className="mono" style={{ padding: '3px 10px', borderRadius: 8, background: left <= 5 ? 'var(--danger-bg)' : 'var(--surface2)', border: `1px solid ${left <= 5 ? 'var(--danger-bd)' : 'var(--line3)'}`, fontSize: 13 }}>
      {left}s
    </span>
  );
}

/** Stats of everything picked so far (the side panel while drafting). */
function PoolStats({ s }: { s: DraftState }) {
  const defs = useDefs(s);
  const stats = useMemo(() => deckStats('limited', asDeckCards(s.pool.map((c, i) => ({ c, i }))).cards, defs), [s.pool, defs]);
  return (
    <>
      <div style={{ fontSize: 12.5, color: 'var(--mu)' }}>Your picks so far</div>
      <StatsSections stats={stats} cardsLabel={stats.total} />
    </>
  );
}

function Picking({ s, view }: { s: DraftState; view: ViewOpts }) {
  const qc = useQueryClient();
  const toast = useApp((x) => x.toast);
  const defs = useDefs(s);
  const [busy, setBusy] = useState(false);
  const pick = async (key: string) => {
    if (busy || s.picked) return;
    setBusy(true);
    try {
      await api.post<DraftState>(`api/draft/games/${s.id}/pick`, { key, seat: s.seat });
      void qc.invalidateQueries({ queryKey: ['draft', s.id] });
    } catch (e) {
      toast(errMsg(e));
    } finally {
      setBusy(false);
    }
  };
  const pack = useMemo(() => groupCards(asDeckCards(s.pack.map((c, i) => ({ c, i }))).cards, defs, view.groupBy, view.sortBy), [s.pack, defs, view.groupBy, view.sortBy]);
  const picks = useMemo(() => groupCards(asDeckCards(s.pool.map((c, i) => ({ c, i }))).cards, defs, view.groupBy, view.sortBy), [s.pool, defs, view.groupBy, view.sortBy]);
  return (
    <Body panel={<PoolStats s={s} />}>
      <div className="row" style={{ gap: 12, flexWrap: 'wrap' }}>
        <span style={{ fontSize: 15, fontWeight: 600 }}>
          Pack {s.round} · pick {s.pickNo}
        </span>
        <span style={{ fontSize: 13, color: 'var(--mu)' }}>passing {s.passing === 'left' ? '← left' : 'right →'}</span>
        {s.timeLeft != null && !s.picked && <Countdown key={`${s.round}:${s.pickNo}:${s.seat}`} ms={s.timeLeft} />}
        <div className="grow" />
        <span style={{ fontSize: 13, color: 'var(--mu)' }}>{s.picked ? `Waiting for ${s.waitingOn.join(', ') || 'the others'}…` : 'Click a card to take it'}</span>
      </div>
      <div style={{ opacity: busy || s.picked ? 0.6 : 1 }}>
        <CardGroupsView groups={pack} defs={defs} groupBy={view.groupBy} onClick={busy || s.picked ? undefined : (_e, c) => void pick(c.def)} decor={{ turnPrefix: 'dp:' }} />
      </div>
      <Seats s={s} />
      <div className="col" style={{ gap: 8 }}>
        <h2 style={{ fontSize: 16 }}>Your picks ({s.pool.length})</h2>
        {s.pool.length ? <CardGroupsView groups={picks} defs={defs} groupBy={view.groupBy} decor={{ turnPrefix: 'dl:' }} /> : <div style={{ color: 'var(--mu)', fontSize: 13 }}>Nothing picked yet.</div>}
      </div>
    </Body>
  );
}

function Seats({ s }: { s: DraftState }) {
  return (
    <div className="row" style={{ gap: 6, flexWrap: 'wrap', fontSize: 12 }}>
      {s.seats.map((x, i) => (
        <span key={i} style={{ padding: '3px 9px', borderRadius: 7, border: `1px solid ${i === s.seat ? 'var(--acc)' : 'var(--line3)'}`, color: x.picked ? 'var(--tx2)' : 'var(--mu)' }}>
          {x.name}
          {x.human ? '' : ' (AI)'} {x.picked ? '✓' : '…'}
        </span>
      ))}
    </div>
  );
}

function Build({ s, view }: { s: DraftState; view: ViewOpts }) {
  const toast = useApp((x) => x.toast);
  const qc = useQueryClient();
  const defs = useDefs(s);
  /** pool indices in the deck */
  const [inDeck, setInDeck] = useState<Set<number>>(new Set());
  const [basics, setBasics] = useState<Record<string, number>>({});
  const [name, setName] = useState(`${s.mode === 'draft' ? 'Draft' : 'Sealed'} · ${s.sets[0]?.name ?? ''}`.trim());
  const [colorFilter, setColorFilter] = useState<string[]>([]);
  const [saving, setSaving] = useState(false);
  const [dirty, setDirty] = useState(false);
  const [loaded, setLoaded] = useState(false);

  // a deck saved before (rebuilding between games): start from it
  useEffect(() => {
    if (loaded) return;
    if (s.deckId == null) return setLoaded(true);
    api
      .get<Deck>(`api/decks/${s.deckId}`)
      .then((d) => {
        setName(d.name);
        const free = new Map<string, number[]>();
        s.pool.forEach((c, i) => free.set(c.key, [...(free.get(c.key) ?? []), i]));
        const picked = new Set<number>();
        const b: Record<string, number> = {};
        for (const c of d.cards.filter((x) => x.section === 'main')) {
          const def = d.defs[c.def];
          const basic = BASICS.find(([n]) => n === def?.name && /Basic/.test(def.typeLine));
          if (basic) b[basic[0]] = (b[basic[0]] ?? 0) + c.qty;
          else
            for (let k = 0; k < c.qty; k++) {
              const i = free.get(c.def)?.shift();
              if (i != null) picked.add(i);
            }
        }
        setInDeck(picked);
        setBasics(b);
      })
      .finally(() => setLoaded(true));
  }, [s.deckId, loaded, s.pool]);

  const deck = useMemo(() => s.pool.map((c, i) => ({ c, i })).filter((x) => inDeck.has(x.i)), [s.pool, inDeck]);
  const side = useMemo(
    () => s.pool.map((c, i) => ({ c, i })).filter((x) => !inDeck.has(x.i) && (!colorFilter.length || [...x.c.colors].some((col) => colorFilter.includes(col)) || (!x.c.colors && colorFilter.includes('C')))),
    [s.pool, inDeck, colorFilter],
  );
  const deckCards = useMemo(() => asDeckCards(deck), [deck]);
  const sideCards = useMemo(() => asDeckCards(side), [side]);
  const deckGroups = useMemo(() => groupCards(deckCards.cards, defs, view.groupBy, view.sortBy), [deckCards, defs, view.groupBy, view.sortBy]);
  const sideGroups = useMemo(() => groupCards(sideCards.cards, defs, view.groupBy, view.sortBy), [sideCards, defs, view.groupBy, view.sortBy]);
  const nBasics = Object.values(basics).reduce((a, b) => a + b, 0);
  const stats = useMemo(() => {
    const st = deckStats('limited', deckCards.cards, defs);
    return { ...st, total: st.total + nBasics, lands: st.lands + nBasics };
  }, [deckCards, defs, nBasics]);
  const total = stats.total;
  const creatures = deck.filter((x) => /\bCreature\b/.test(x.c.typeLine)).length;
  const lands = stats.lands;
  const toggle = (i: number) => {
    setInDeck((cur) => {
      const n = new Set(cur);
      if (n.has(i)) n.delete(i);
      else n.add(i);
      return n;
    });
    setDirty(true);
  };
  /** Moves one copy of a merged card between the deck and the pool. */
  const moveOne = (idx: Map<number, number[]>) => (_e: React.MouseEvent, c: DeckCard) => {
    const all = idx.get(c.id);
    if (all?.length) toggle(all[all.length - 1]);
  };
  const setBasic = (n: string, v: number) => {
    setBasics({ ...basics, [n]: Math.max(0, Math.min(60, v)) });
    setDirty(true);
  };
  const suggest = async () => {
    try {
      const r = await api.get<{ main: string[]; basics: Record<string, number> }>(`api/draft/games/${s.id}/suggest?seat=${s.seat}`);
      const free = new Map<string, number[]>();
      s.pool.forEach((c, i) => free.set(c.key, [...(free.get(c.key) ?? []), i]));
      setInDeck(new Set(r.main.map((k) => free.get(k)?.shift()).filter((x): x is number => x != null)));
      setBasics(r.basics);
      setDirty(true);
    } catch (e) {
      toast(errMsg(e));
    }
  };
  const suggestLands = async () => {
    try {
      setBasics(await api.post<Record<string, number>>(`api/draft/games/${s.id}/lands?seat=${s.seat}`, { main: deck.map((x) => x.c.key) }));
      setDirty(true);
    } catch (e) {
      toast(errMsg(e));
    }
  };
  const save = async () => {
    setSaving(true);
    try {
      await api.post(`api/draft/games/${s.id}/deck`, { seat: s.seat, name, main: deck.map((x) => x.c.key), basics });
      setDirty(false);
      toast('Deck saved — it is in your deck list too');
      void qc.invalidateQueries({ queryKey: ['draft', s.id] });
    } catch (e) {
      toast(errMsg(e));
    } finally {
      setSaving(false);
    }
  };
  return (
    <Body
      panel={
        <>
          <div style={{ fontSize: 12.5, color: 'var(--mu)' }}>Your deck, basic lands included</div>
          <StatsSections stats={stats} />
        </>
      }
    >
      <div className="row" style={{ gap: 12, flexWrap: 'wrap', padding: '12px 14px', borderRadius: 12, background: 'var(--surface)', border: '1px solid var(--line2)' }}>
        <input
          className="inp"
          aria-label="Deck name"
          value={name}
          onChange={(e) => {
            setName(e.target.value);
            setDirty(true);
          }}
          style={{ width: 240 }}
        />
        <span style={{ fontSize: 13.5 }}>
          <b style={{ color: total < 40 ? '#e0a84f' : 'var(--tx)' }}>{total}</b> cards · {creatures} creatures · {lands} lands
          {total < 40 ? <span style={{ color: '#e0a84f' }}> · at least 40 needed</span> : null}
        </span>
        <div className="grow" />
        <button className="tb" onClick={suggest} title="A deck from your best two colours, with lands">
          Build for me
        </button>
        <button className={'tb pri' + (saving ? ' dis' : '')} onClick={save} disabled={saving || !name.trim() || total === 0}>
          {saving ? 'Saving…' : s.deckId != null && !dirty ? 'Saved ✓' : 'Save deck'}
        </button>
        {s.deckId != null && !dirty && (
          <Link to={`/lobby/${s.lobbyId}`} className="tb">
            Back to the lobby
          </Link>
        )}
      </div>
      <div className="row" style={{ gap: 10, flexWrap: 'wrap', alignItems: 'center' }}>
        <span style={{ fontSize: 13, color: 'var(--tx3)' }}>Basic lands</span>
        {BASICS.map(([n, c]) => (
          <div key={n} className="row" style={{ gap: 4 }}>
            <ManaSymbol c={c} size={18} title={n} />
            <button className="sq" aria-label={`One fewer ${n}`} onClick={() => setBasic(n, (basics[n] ?? 0) - 1)}>
              −
            </button>
            <span className="mono" style={{ width: 18, textAlign: 'center', fontSize: 13 }}>
              {basics[n] ?? 0}
            </span>
            <button className="sq" aria-label={`One more ${n}`} onClick={() => setBasic(n, (basics[n] ?? 0) + 1)}>
              +
            </button>
          </div>
        ))}
        <button className="tb sm" onClick={suggestLands} title="17 lands, split by the mana symbols of the cards in the deck">
          Suggest lands
        </button>
      </div>
      <div className="col" style={{ gap: 8 }}>
        <h2 style={{ fontSize: 16 }}>
          Deck ({deck.length} + {nBasics} basics) <span style={{ fontSize: 12, color: 'var(--mu)', fontWeight: 400 }}>· click a card to move it to the sideboard</span>
        </h2>
        {deck.length ? <CardGroupsView groups={deckGroups} defs={defs} groupBy={view.groupBy} onClick={moveOne(deckCards.idx)} decor={{ turnPrefix: 'dd:' }} /> : <div style={{ color: 'var(--mu)', fontSize: 13 }}>Click cards in your pool to add them, or Build for me.</div>}
      </div>
      <div className="col" style={{ gap: 8 }}>
        <div className="row" style={{ gap: 8, flexWrap: 'wrap' }}>
          <h2 className="grow" style={{ fontSize: 16 }}>
            Pool / sideboard ({s.pool.length - inDeck.size}) <span style={{ fontSize: 12, color: 'var(--mu)', fontWeight: 400 }}>· click a card to put it in the deck</span>
          </h2>
          {['W', 'U', 'B', 'R', 'G', 'C'].map((c) => {
            const on = colorFilter.includes(c);
            return (
              <button
                key={c}
                aria-label={`Show ${c}`}
                aria-pressed={on}
                onClick={() => setColorFilter(on ? colorFilter.filter((x) => x !== c) : [...colorFilter, c])}
                style={{ width: 28, height: 28, borderRadius: 14, border: `2px solid ${on ? 'var(--acc)' : 'transparent'}`, background: 'transparent', padding: 0, cursor: 'pointer', opacity: on || !colorFilter.length ? 1 : 0.45, display: 'inline-flex', alignItems: 'center', justifyContent: 'center' }}
              >
                <ManaSymbol c={c} size={22} title={null} />
              </button>
            );
          })}
        </div>
        <CardGroupsView groups={sideGroups} defs={defs} groupBy={view.groupBy} onClick={moveOne(sideCards.idx)} decor={{ turnPrefix: 'ds:' }} />
      </div>
    </Body>
  );
}
