import { useInfiniteQuery, useQuery, useQueryClient } from '@tanstack/react-query';
import { Select } from '../../ui/Select';
import { useEffect, useMemo, useRef, useState } from 'react';
import { Link, useNavigate, useParams } from 'react-router-dom';
import type { CardDef, Deck, DeckCard, GroupBy, RecentTag, Section, SortBy } from '@aetherhub/shared';
import { canBeCommander, checkLegality, deckStats, deckTagNames, tagScope, FORMATS, formatInfo, groupCards, SECTION_NAMES } from '@aetherhub/shared';
import { api } from '../../lib/api';
import { imgUrl } from '../../lib/base';
import { errMsg, useApp } from '../../lib/store';
import { SearchResults } from './SearchResults';
import { Chevron, CardImage, Confirm, openMenuAt, openMenuBelow, timeAgo, useDebounced, useMenu, type MenuItem } from '../../ui/ui';
import { CardGroupsView, SidePanel, StatsSections, useDeckPreview, ViewControls, VIEW_CSS } from './deckViews';
import { CardDetail, ExportModal, ProxyModal, SampleHand, SCOPE_TITLE, shownDef, tagChoices, tagClass, tagColorOf, TagManager, tagStyle, type TagAdd } from './deckModals';

/** Tag filter value for cards with no tags. */
const UNTAGGED = '\u0000untagged';

type Board = 'main' | 'side' | 'maybe' | 'tokens' | 'companion' | 'planar' | 'schemes';
const BOARD_SECTIONS: Record<Board, Section[]> = {
  main: ['commander', 'main'],
  side: ['side'],
  maybe: ['maybe'],
  tokens: ['tokens'],
  companion: ['companion'],
  planar: ['planar'],
  schemes: ['schemes'],
};

/** The deck picker (lobby): the deck is read-only and the header offers Select deck and Sample hand. */
export interface DeckPickMode {
  onSelect: (deck: Deck) => void;
  onBack: () => void;
}

export default function DeckEditor() {
  const { id } = useParams();
  return <DeckView deckId={Number(id)} />;
}

export function DeckView({ deckId, pick }: { deckId: number; pick?: DeckPickMode }) {
  const me = useApp((s) => s.me)!;
  const setPrefs = useApp((s) => s.setPrefs);
  const toast = useApp((s) => s.toast);
  const qc = useQueryClient();
  const nav = useNavigate();
  const q = useQuery({ queryKey: ['deck', deckId], queryFn: () => api.get<Deck>(`api/decks/${deckId}`) });
  const folders = useQuery({ queryKey: ['folders'], queryFn: () => api.get<{ id: number; name: string }[]>('api/folders') });
  const deck = q.data;
  const [board, setBoard] = useState<Board>('main');
  const [groupBy, setGroupBy] = useState<GroupBy>('type');
  const [sortBy, setSortBy] = useState<SortBy>('name');
  const [hideWarn, setHideWarn] = useState(false);
  const [modal, setModal] = useState<null | 'hand' | 'proxy' | 'export' | 'delete' | 'tags' | { card: number }>(null);
  const [tagFilter, setTagFilter] = useState<string | null>(null);
  const [sel, setSel] = useState<number[]>([]);
  const [editName, setEditName] = useState<string | null>(null);
  const [search, setSearch] = useState('');
  const [resultsOpen, setResultsOpen] = useState(false);
  const searchBox = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const down = (e: MouseEvent) => {
      if (searchBox.current && !searchBox.current.contains(e.target as Node)) setResultsOpen(false);
    };
    window.addEventListener('mousedown', down);
    return () => window.removeEventListener('mousedown', down);
  }, []);
  const [showMore, setShowMore] = useState(false);
  const dq = useDebounced(search, 220);
  const searchView = me.prefs.deckSearchView === 'list' ? 'list' : 'visual';
  const results = useInfiniteQuery({
    queryKey: ['search', searchView, dq],
    queryFn: ({ pageParam }) => api.get<{ cards: CardDef[]; total: number }>(`api/cards/search?limit=${searchView === 'visual' ? 60 : 20}&page=${pageParam}&q=${encodeURIComponent(dq)}`),
    initialPageParam: 0,
    getNextPageParam: (last, pages) => (pages.reduce((n, pg) => n + pg.cards.length, 0) < last.total && last.cards.length ? pages.length : undefined),
    enabled: dq.trim().length >= 2,
  });
  const found = results.data?.pages.flatMap((pg) => pg.cards) ?? [];
  useDeckPreview();
  const editable = !pick && !!deck && !deck.precon && (deck.ownerId === me.id || me.isAdmin);
  const [notes, setNotes] = useState<string | null>(null);

  useEffect(() => {
    setNotes(null);
    setTagFilter(null);
  }, [deckId]);

  const setDeck = (d: Deck) => {
    qc.setQueryData(['deck', deckId], d);
    qc.invalidateQueries({ queryKey: ['decks'] });
  };
  const mutate = async (p: Promise<Deck>) => {
    try {
      setDeck(await p);
    } catch (e) {
      toast(errMsg(e));
    }
  };
  const addCard = (key: string, section: Section) => mutate(api.post<Deck>(`api/decks/${deckId}/cards`, { key, section, qty: 1 }));
  const patchCard = (cardId: number, p: object) => mutate(api.patch<Deck>(`api/decks/${deckId}/cards/${cardId}`, p));
  const updateDeck = (p: object) => mutate(api.put<Deck>(`api/decks/${deckId}`, p));

  const legality = useMemo(() => (deck ? checkLegality(deck.format, deck.cards, deck.defs) : null), [deck]);
  const stats = useMemo(() => (deck ? deckStats(deck.format, deck.cards, deck.defs) : null), [deck]);
  const suggested = useMemo(() => {
    if (!deck) return [];
    const have = new Set(deck.cards.filter((c) => c.section === 'tokens').map((c) => deck.defs[c.def]?.name));
    const out = new Map<string, { key: string; name: string; from: string }>();
    for (const c of deck.cards) {
      if (!['main', 'commander', 'companion'].includes(c.section)) continue;
      const d = deck.defs[c.def];
      for (const p of d?.allParts || []) {
        if ((p.component === 'token' || /Emblem|Dungeon/.test(p.typeLine)) && p.name !== d.name && !have.has(p.name) && !out.has(p.name)) out.set(p.name, { key: 'p:' + p.id, name: p.name, from: d.name });
      }
    }
    return [...out.values()];
  }, [deck]);

  if (q.isError) return <div style={{ padding: 40 }}>Deck not found. <Link to="/decks">Back to decks</Link></div>;
  if (!deck || !legality || !stats) return <div style={{ padding: 40, color: 'var(--mu)' }}>Loading deck…</div>;

  const f = formatInfo(deck.format);
  const tagNames = deckTagNames(deck.tags, deck.cards);
  const onBoard = deck.cards.filter((c) => BOARD_SECTIONS[board].includes(c.section));
  // A filter on a tag that was since renamed or deleted shows everything.
  const activeTag = tagFilter === UNTAGGED || (tagFilter != null && tagNames.includes(tagFilter)) ? tagFilter : null;
  const hasTag = (c: DeckCard, t: string | null) => (t == null ? true : t === UNTAGGED ? !c.tags.length : c.tags.includes(t));
  const boardCards = onBoard.filter((c) => hasTag(c, activeTag));
  const groups = groupCards(boardCards, deck.defs, groupBy, sortBy, tagNames);
  const boardTagCount = (t: string | null) => onBoard.filter((c) => hasTag(c, t)).reduce((s, c) => s + c.qty, 0);
  const boardCount = (b: Board) => deck.cards.filter((c) => BOARD_SECTIONS[b].includes(c.section)).reduce((s, c) => s + c.qty, 0);
  const extraBoards: Board[] = (['companion', 'planar', 'schemes'] as Board[]).filter((b) => showMore || boardCount(b) > 0 || board === b);
  const reasonsOf = (c: DeckCard) => legality.cards[c.id] || [];
  const coverDef = deck.coverKey ? deck.defs[deck.coverKey] : deck.cards.find((c) => c.section === 'commander') ? deck.defs[deck.cards.find((c) => c.section === 'commander')!.def] : undefined;
  const targetSection: Section = board === 'main' ? 'main' : (board as Section);

  const clickCard = (e: React.MouseEvent, c: DeckCard) => {
    if (e.ctrlKey || e.metaKey || e.shiftKey) {
      setSel((s) => (s.includes(c.id) ? s.filter((x) => x !== c.id) : [...s, c.id]));
      return;
    }
    setModal({ card: c.id });
  };
  // ---------------------------------------------------------------- right-click menu
  /** Right-clicking a selected card acts on the whole selection. */
  const menuTargets = (c: DeckCard) => (sel.includes(c.id) && sel.length > 1 ? deck.cards.filter((x) => sel.includes(x.id)) : [c]);
  const run = async (fn: (c: DeckCard) => Promise<Deck>, cards: DeckCard[]) => {
    try {
      let d: Deck | null = null;
      for (const c of cards) d = await fn(c);
      if (d) setDeck(d);
    } catch (e) {
      toast(errMsg(e));
    }
    setSel((s) => s.filter((id) => !cards.some((c) => c.id === id)));
  };
  const setQty = (cards: DeckCard[], delta: number | 'all') => run((c) => api.patch<Deck>(`api/decks/${deckId}/cards/${c.id}`, { qty: delta === 'all' ? 0 : Math.max(0, c.qty + delta) }), cards);
  const moveTo = (cards: DeckCard[], section: Section) => run((c) => api.patch<Deck>(`api/decks/${deckId}/cards/${c.id}`, { section }), cards.filter((c) => c.section !== section));
  /** Splits one copy off into another section. */
  const moveOne = (c: DeckCard, section: Section) =>
    run(async (x) => {
      await api.post<Deck>(`api/decks/${deckId}/cards`, { key: x.def, section, qty: 1 });
      return api.patch<Deck>(`api/decks/${deckId}/cards/${x.id}`, { qty: x.qty - 1 });
    }, [c]);
  // ---------------------------------------------------------------- tags
  /** Reads the deck from the query cache so an open menu shows tags changed while it was open. */
  const liveDeck = () => qc.getQueryData<Deck>(['deck', deckId]) ?? deck;
  const tagCards = async (ids: number[], add: TagAdd[], remove: string[]) => {
    await mutate(api.post<Deck>(`api/decks/${deckId}/cards/tags`, { cardIds: ids, add, remove }));
    useMenu.getState().refresh();
  };
  const tagItems = (ids: number[]): MenuItem[] => {
    const d = liveDeck();
    const cs = d.cards.filter((c) => ids.includes(c.id));
    const { recent, onCards } = tagChoices(d, cs);
    const chip = (t: RecentTag) => {
      const on = cs.length > 0 && cs.every((c) => c.tags.includes(t.name));
      const some = !on && cs.some((c) => c.tags.includes(t.name));
      return { label: '#' + t.name + (t.scope === 'deck' ? ' · deck' : '') + (some ? ' (some)' : ''), on, run: () => tagCards(ids, on ? [] : [t], on ? [t.name] : []) };
    };
    const used = Array.from(new Set(cs.flatMap((c) => c.tags)));
    const known = d.tags.map((t) => ({ name: t.name, scope: t.scope }));
    const unlisted = deckTagNames(d.tags, d.cards)
      .filter((n) => !d.tags.some((t) => t.name === n))
      .map((name) => ({ name, scope: 'deck' as const }));
    const I: MenuItem[] = [{ kind: 'chips', label: `Recent tags${cs.length > 1 ? ` · toggles on all ${cs.length}` : ''}`, chips: recent.map(chip) }];
    if (onCards.length) I.push({ kind: 'chips', label: 'Also on ' + (cs.length > 1 ? 'these cards' : 'this card'), chips: onCards.map(chip) });
    I.push({ kind: 'input', label: 'New tag · global unless ticked', placeholder: 'e.g. mana rock', button: 'Add', checkbox: 'This deck only', submit: (v, deckOnly) => tagCards(ids, [{ name: v, scope: deckOnly ? 'deck' : 'global' }], []) });
    I.push({ kind: 'sep' });
    I.push({
      kind: 'sub',
      label: 'All tags',
      items: () => {
        const glob = known.filter((t) => t.scope === 'global');
        const own = [...known.filter((t) => t.scope === 'deck'), ...unlisted];
        const out: MenuItem[] = [];
        if (glob.length) out.push({ kind: 'chips', label: 'Global · in all your decks', chips: glob.map(chip) });
        if (own.length) out.push({ kind: 'chips', label: 'This deck only', chips: own.map(chip) });
        if (!out.length) out.push({ kind: 'head', label: 'No tags yet' });
        return out;
      },
    });
    if (used.length) I.push({ kind: 'item', label: cs.length > 1 ? 'Remove all tags from these' : 'Remove all tags', danger: true, run: () => tagCards(ids, [], used) });
    I.push({ kind: 'item', label: 'Manage tags…', run: () => setModal('tags') });
    return I;
  };
  const MOVE_ORDER: Section[] = ['main', 'side', 'maybe', 'commander', 'companion', 'tokens', 'planar', 'schemes'];
  const boardName = (s: Section) => (s === 'tokens' ? 'Tokens/Extras' : s === 'side' ? 'Sideboard' : s === 'maybe' ? 'Maybeboard' : SECTION_NAMES[s]);
  const cardMenu = (c: DeckCard): MenuItem[] => {
    const cards = menuTargets(c);
    const one = cards.length === 1;
    const d = deck.defs[c.def];
    const I: MenuItem[] = [{ kind: 'head', label: one ? `${d?.name || 'Card'} · ${c.qty}× in ${boardName(c.section)}` : `${cards.length} selected cards` }];
    if (!editable) {
      I.push({ kind: 'item', label: 'Card details…', run: () => setModal({ card: c.id }) });
      return I;
    }
    const num = (v: string) => {
      const n = parseInt(v, 10);
      return Number.isFinite(n) && n > 0 ? Math.min(n, 999) : 0;
    };
    I.push({ kind: 'item', label: 'Add 1', hint: '+1', run: () => setQty(cards, 1) });
    I.push({ kind: 'input', label: 'Add X', placeholder: 'How many?', button: 'Add', submit: (v) => num(v) && setQty(cards, num(v)) });
    I.push({ kind: 'item', label: 'Remove 1', hint: '−1', run: () => setQty(cards, -1) });
    I.push({ kind: 'input', label: 'Remove X', placeholder: 'How many?', button: 'Remove', submit: (v) => num(v) && setQty(cards, -num(v)) });
    I.push({ kind: 'item', label: one ? 'Remove all copies' : 'Remove from deck', danger: true, run: () => setQty(cards, 'all') });
    I.push({ kind: 'sep' });
    if (one && c.section === 'commander') I.push({ kind: 'item', label: 'No longer commander', hint: '→ Main', run: () => moveTo(cards, 'main') });
    else if (cards.some((x) => x.section !== 'commander')) {
      // Never restricted: any card can go in the command zone. Legality warnings flag unusual picks.
      const odd = cards.some((x) => deck.defs[x.def] && !canBeCommander(deck.defs[x.def], deck.format));
      I.push({ kind: 'item', label: one ? 'Make commander' : 'Make commanders', hint: !f.commandZone ? 'no command zone' : odd ? 'unusual' : undefined, run: () => moveTo(cards, 'commander') });
    }
    if (one && c.section === 'companion') I.push({ kind: 'item', label: 'No longer companion', hint: '→ Main', run: () => moveTo(cards, 'main') });
    else if (one && d && /\bCompanion\b/.test(d.oracleText)) I.push({ kind: 'item', label: 'Make companion', run: () => moveTo(cards, 'companion') });
    I.push({ kind: 'sub', label: one && c.qty > 1 ? 'Move all to' : 'Move to', items: () => MOVE_ORDER.filter((s) => !(one && s === c.section)).map((s) => ({ kind: 'item', label: boardName(s), run: () => moveTo(cards, s) })) });
    if (one && c.qty > 1) I.push({ kind: 'sub', label: 'Move 1 to', items: () => MOVE_ORDER.filter((s) => s !== c.section).map((s) => ({ kind: 'item', label: boardName(s), run: () => moveOne(c, s) })) });
    I.push({ kind: 'sep' });
    const ids = cards.map((x) => x.id);
    I.push({ kind: 'sub', label: one ? 'Tags' : 'Tag these cards', items: () => tagItems(ids) });
    if (one) {
      I.push({ kind: 'item', label: c.foil ? 'Foil: on' : 'Foil: off', hint: 'toggle', run: () => patchCard(c.id, { foil: !c.foil }) });
      I.push({ kind: 'item', label: 'Use as deck cover', run: () => updateDeck({ coverKey: c.def }) });
      I.push({ kind: 'item', label: 'Card details…', run: () => setModal({ card: c.id }) });
    }
    return I;
  };
  const onCardCtx = (e: React.MouseEvent, c: DeckCard) => openMenuAt(e, () => cardMenu(c));

  const cardExtra = (c: DeckCard) => {
    const r = reasonsOf(c);
    return r.length ? r.join(' · ') : null;
  };


  return (
    <div className="screen">
      <style>{VIEW_CSS}</style>
      <header className="page-header" style={{ minHeight: 68, padding: '8px 24px', background: 'var(--panel)' }}>
        {pick ? (
          <button className="tb" aria-label="Back to the decks" onClick={pick.onBack}>
            <Chevron />Decks
          </button>
        ) : (
          <Link to="/decks" className="tb" aria-label="Back to deck list">
            <Chevron />Decks
          </Link>
        )}
        <div
          title={coverDef ? `Cover card: ${coverDef.name}` : 'Cover card'}
          style={{ width: 44, height: 44, borderRadius: 8, flexShrink: 0, backgroundColor: '#7d6224', backgroundImage: coverDef?.img[0] ? `url("${imgUrl(coverDef.img[0])}")` : undefined, backgroundSize: '160% auto', backgroundPosition: 'center 20%' }}
        />
        <div className="col" style={{ gap: 3, minWidth: 0 }}>
          <div className="row">
            {editName != null ? (
              <input
                className="inp"
                autoFocus
                value={editName}
                onChange={(e) => setEditName(e.target.value)}
                onBlur={() => {
                  if (editName.trim() && editName !== deck.name) updateDeck({ name: editName.trim() });
                  setEditName(null);
                }}
                onKeyDown={(e) => e.key === 'Enter' && (e.target as HTMLInputElement).blur()}
                style={{ fontSize: 18, fontFamily: 'var(--display)' }}
              />
            ) : (
              <h1 style={{ fontSize: 23, cursor: editable ? 'text' : 'default' }} onClick={() => editable && setEditName(deck.name)} title={editable ? 'Click to rename' : undefined}>
                {deck.name}
              </h1>
            )}
            <Select aria-label="Format" value={deck.format} disabled={!editable} onChange={(v) => updateDeck({ format: v })} style={{ height: 26, fontSize: 12 }} options={FORMATS.map((x) => ({ value: x.id, label: x.name }))} menuWidth={180} />
          </div>
          <div style={{ fontSize: 12, color: 'var(--mu)' }}>
            {deck.precon ? `${deck.precon.type} · ${deck.precon.set}${deck.precon.released ? ` (${deck.precon.released.slice(0, 4)})` : ''}` : `by ${deck.ownerName}`} · {stats.total} cards · starting life {legality.life ?? f.life} · {f.commandZone ? 'command zone on' : 'no command zone'}
            {deck.precon ? '' : ` · edited ${timeAgo(deck.updatedAt)}`}
          </div>
        </div>
        <div className="grow" />
        {legality.warnings.length > 0 && (
          <button className="tb" onClick={() => setHideWarn(!hideWarn)} style={{ borderColor: '#7a3a33', color: '#f5b3a8' }}>
            <span className="dot" style={{ background: 'var(--danger)' }} />
            {hideWarn ? 'Show ' : ''}
            {legality.warnings.length} warning{legality.warnings.length === 1 ? '' : 's'}
          </button>
        )}
        {pick ? (
          <>
            <button className="tb" onClick={() => setModal('hand')}>
              Sample hand
            </button>
            <button className="tb pri" onClick={() => pick.onSelect(deck)}>
              Select deck
            </button>
          </>
        ) : (
          <>
        {editable && (
          <Link to={`/import?deck=${deck.id}`} className="tb">
            Import
          </Link>
        )}
        <button className="tb" onClick={() => setModal('export')}>
          Export
        </button>
        <button className="tb" onClick={() => setModal('proxy')}>
          Proxy PDF
        </button>
        <button className="tb" onClick={() => setModal('hand')}>
          Sample hand
        </button>
        {!editable && (
          <button
            className="tb"
            onClick={async () => {
              const d = await api.post<Deck>(`api/decks/${deck.id}/clone`);
              qc.invalidateQueries({ queryKey: ['decks'] });
              nav(`/decks/${d.id}`);
              toast(deck.precon ? 'Copied into your decks' : 'Cloned into your decks');
            }}
          >
            {deck.precon ? 'Copy to my decks' : 'Clone'}
          </button>
        )}
        {editable && (
          <button className="tb danger" onClick={() => setModal('delete')} title="Delete this deck">
            Delete
          </button>
        )}
        <Link to={`/decks/${deck.id}/playtest`} className="tb pri">
          Playtest
        </Link>
          </>
        )}
      </header>
      {legality.notes.length > 0 && (
        <div style={{ flexShrink: 0, margin: '12px 24px 0', padding: '8px 14px', borderRadius: 10, background: 'var(--acc-bg)', border: '1px solid var(--acc-bd)', fontSize: 12.5, color: 'var(--acc-tx)' }}>
          <strong>Deck rules from your commander</strong> · {legality.notes.join(' · ')}
        </div>
      )}
      {legality.warnings.length > 0 && !hideWarn && (
        <div style={{ flexShrink: 0, margin: '12px 24px 0', padding: '10px 14px', borderRadius: 10, background: 'var(--danger-bg)', border: '1px solid var(--danger-bd)', display: 'flex', gap: 14, alignItems: 'flex-start', maxHeight: 110, overflowY: 'auto' }} className="lg">
          <div style={{ fontSize: 12.5, fontWeight: 600, color: '#f5b3a8', whiteSpace: 'nowrap' }}>Deck warnings</div>
          <div style={{ display: 'flex', flexWrap: 'wrap', gap: '6px 16px', flexGrow: 1 }}>
            {legality.warnings.map((w, i) => (
              <div key={i} style={{ fontSize: 12.5, color: 'var(--danger-tx)' }}>
                · {w}
              </div>
            ))}
          </div>
          <div style={{ fontSize: 11.5, color: '#b58e88', whiteSpace: 'nowrap' }}>Nothing is ever blocked</div>
        </div>
      )}
      <div className="row" style={{ minHeight: 58, flexShrink: 0, padding: '8px 24px', gap: 10, flexWrap: 'wrap' }}>
        {editable && (
          <div ref={searchBox} style={{ position: 'relative', width: 270 }}>
            <input
              className="inp"
              aria-label="Search cards to add"
              placeholder="Add cards… name or t:creature c:g mv:3 o:proliferate"
              value={search}
              onChange={(e) => {
                setSearch(e.target.value);
                setResultsOpen(true);
              }}
              onFocus={() => setResultsOpen(true)}
              onKeyDown={(e) => {
                if (e.key === 'Escape') setSearch('');
                if (e.key === 'Enter' && found[0]) addCard(found[0].key, targetSection);
              }}
              style={{ width: '100%', height: 36, paddingLeft: 34 }}
            />
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="var(--mu2)" strokeWidth="2" style={{ position: 'absolute', left: 11, top: 10 }} aria-hidden="true">
              <circle cx="11" cy="11" r="7" />
              <path d="M20 20l-3.5-3.5" />
            </svg>
            {dq.trim().length >= 2 && search && resultsOpen && (
              <SearchResults
                anchor={searchBox}
                query={search}
                cards={found}
                total={results.data?.pages[0]?.total ?? 0}
                fetching={results.isFetching}
                hasMore={!!results.hasNextPage}
                loadMore={() => void results.fetchNextPage()}
                view={searchView}
                onView={(v) => setPrefs({ deckSearchView: v })}
                target={targetSection}
                commandZone={f.commandZone}
                onAdd={addCard}
                onClose={() => setResultsOpen(false)}
              />
            )}
          </div>
        )}
        <div className="segs" role="group" aria-label="Board">
          {(['main', 'side', 'maybe', 'tokens'] as Board[]).map((b) => (
            <button key={b} className={'seg' + (board === b ? ' on' : '')} onClick={() => setBoard(b)}>
              {b === 'main' ? 'Main' : b === 'side' ? 'Side' : b === 'maybe' ? 'Maybe' : 'Tokens/Extras'} {boardCount(b)}
            </button>
          ))}
          {extraBoards.map((b) => (
            <button key={b} className={'seg' + (board === b ? ' on' : '')} onClick={() => setBoard(b)}>
              {SECTION_NAMES[b as Section]} {boardCount(b)}
            </button>
          ))}
          <button className="seg" onClick={() => setShowMore(!showMore)} title={showMore ? 'Hide the extra sections' : 'Companion, Planar, Schemes'} aria-label={showMore ? 'Hide the extra sections' : 'Show more sections'} style={{ display: 'inline-flex', alignItems: 'center' }}>
            <Chevron dir={showMore ? 'left' : 'right'} size={13} />
          </button>
        </div>
        <div className="grow" />
        <ViewControls groupBy={groupBy} onGroupBy={setGroupBy} sortBy={sortBy} onSortBy={setSortBy} />
      </div>
      {tagNames.length > 0 && (
        <div className="row" role="group" aria-label="Filter by tag" style={{ flexShrink: 0, padding: '0 24px 10px', gap: 6, flexWrap: 'wrap' }}>
          <span style={{ fontSize: 12, color: 'var(--mu2)', marginRight: 2 }}>Tags</span>
          <button className={'chipb' + (activeTag == null ? ' on' : '')} aria-pressed={activeTag == null} onClick={() => setTagFilter(null)}>
            All {boardTagCount(null)}
          </button>
          {tagNames.map((t) => {
            const n = boardTagCount(t);
            return (
              <button key={t} className={tagClass(tagScope(deck.tags, t), activeTag === t ? ' sel' : n ? '' : ' off')} aria-pressed={activeTag === t} style={tagStyle(tagColorOf(deck, t))} onClick={() => setTagFilter(activeTag === t ? null : t)} title={`${activeTag === t ? 'Show all cards' : `Only cards tagged #${t}`} · ${SCOPE_TITLE[tagScope(deck.tags, t)]}`}>
                #{t} <span style={{ opacity: 0.75, fontWeight: 400 }}>{n}</span>
              </button>
            );
          })}
          <button className={'chipb' + (activeTag === UNTAGGED ? ' on' : '')} aria-pressed={activeTag === UNTAGGED} onClick={() => setTagFilter(activeTag === UNTAGGED ? null : UNTAGGED)}>
            Untagged {boardTagCount(UNTAGGED)}
          </button>
          {editable && (
            <button className="lnk" style={{ fontSize: 12, marginLeft: 4 }} onClick={() => setModal('tags')}>
              Manage tags…
            </button>
          )}
        </div>
      )}
      <div style={{ flexGrow: 1, display: 'flex', minHeight: 0, borderTop: '1px solid var(--line)' }}>
        <div className="lg" style={{ flexGrow: 1, overflowY: 'auto', padding: '18px 24px 40px', minWidth: 0 }}>
          {sel.length > 0 && (
            <div className="row" style={{ marginBottom: 12, fontSize: 12.5, color: 'var(--mu)' }}>
              {sel.length} selected (Ctrl-click to add more)
              {editable && (
                <button className="mbtn" onClick={(e) => openMenuBelow(e.currentTarget, () => tagItems(sel))}>
                  Tag selected…
                </button>
              )}
              <button className="mbtn" onClick={() => setModal('proxy')}>
                Proxy selected
              </button>
              <button className="mbtn" onClick={() => setSel([])}>
                Clear
              </button>
            </div>
          )}
          {!boardCards.length && activeTag != null && onBoard.length > 0 && (
            <div style={{ padding: 60, textAlign: 'center', color: 'var(--mu2)', fontSize: 13.5 }}>
              No cards on this board {activeTag === UNTAGGED ? 'are untagged' : `are tagged #${activeTag}`}.{' '}
              <button className="lnk" onClick={() => setTagFilter(null)}>
                Show all
              </button>
            </div>
          )}
          {!onBoard.length && (
            <div style={{ padding: 60, textAlign: 'center', color: 'var(--mu2)', fontSize: 13.5 }}>
              {board === 'side' ? 'Sideboard is empty. ' : 'Nothing here yet. '}
              {editable ? 'Add cards with the search box, or change a card’s section.' : ''}
            </div>
          )}
          <CardGroupsView
            groups={groups}
            defs={deck.defs}
            groupBy={groupBy}
            onClick={clickCard}
            onContextMenu={onCardCtx}
            decor={{
              reasons: reasonsOf,
              selected: (c) => sel.includes(c.id),
              shown: (c) => shownDef(c, deck.defs),
              previewExtra: cardExtra,
              tileExtra: (c, Z) => (
                <>
                  {c.tags.length > 0 && (
                    <div className="row" style={{ position: 'absolute', left: 6, bottom: 6, right: c.foil ? 40 : 6, gap: 3, flexWrap: 'wrap-reverse', overflow: 'hidden', maxHeight: 40 }}>
                      {c.tags.slice(0, Z < 0.8 ? 1 : 3).map((t) => (
                        <span key={t} className="tagc img" style={tagStyle(tagColorOf(deck, t))}>
                          #{t}
                        </span>
                      ))}
                      {c.tags.length > (Z < 0.8 ? 1 : 3) && (
                        <span className="tagc img" title={c.tags.map((t) => '#' + t).join(' ')}>
                          +{c.tags.length - (Z < 0.8 ? 1 : 3)}
                        </span>
                      )}
                    </div>
                  )}
                  {c.foil && <div style={{ position: 'absolute', right: 6, bottom: 6, fontSize: 10, background: 'rgba(21,19,15,.9)', color: '#e0a84f', borderRadius: 4, padding: '1px 5px' }}>foil</div>}
                </>
              ),
              nameExtra: (c) =>
                c.tags.map((t) => <span key={t} title={'#' + t} aria-label={'#' + t} style={{ display: 'inline-block', width: 7, height: 7, borderRadius: 4, marginLeft: 5, verticalAlign: 1, background: tagColorOf(deck, t) || 'var(--acc)' }} />),
              tags: (c) => (
                <span className="row" style={{ gap: 3, flexWrap: 'wrap' }}>
                  {c.tags.map((t) => (
                    <span key={t} className={tagClass(tagScope(deck.tags, t))} title={SCOPE_TITLE[tagScope(deck.tags, t)]} style={tagStyle(tagColorOf(deck, t))}>
                      #{t}
                    </span>
                  ))}
                </span>
              ),
            }}
          />
        </div>
        <SidePanel>
          <StatsSections stats={stats} />
          <section style={{ background: 'var(--surface)', border: '1px solid var(--line2)', borderRadius: 10, padding: 12 }}>
            <h3 style={{ fontFamily: 'var(--font)', fontSize: 13, marginBottom: 4 }}>Suggested tokens</h3>
            <div style={{ fontSize: 12, color: 'var(--mu)', lineHeight: 1.5 }}>
              {suggested.length ? `From your cards’ related parts: ${suggested.map((s) => `${s.name} (${s.from})`).join(', ')}.` : 'All suggested tokens are in Tokens/Extras.'}
            </div>
            {editable && suggested.length > 0 && (
              <button
                className="tb"
                style={{ marginTop: 10 }}
                onClick={async () => {
                  await mutate(api.post<Deck>(`api/decks/${deckId}/cards/bulk`, { cards: suggested.map((s) => ({ key: s.key, section: 'tokens', qty: 1 })) }));
                  setBoard('tokens');
                }}
              >
                Add all to Tokens/Extras
              </button>
            )}
          </section>
          <section>
            <h3 style={{ fontFamily: 'var(--font)', fontSize: 13, marginBottom: 8 }}>Notes</h3>
            <textarea
              aria-label="Deck notes"
              className="inp"
              readOnly={!editable}
              value={notes ?? deck.notes}
              onChange={(e) => setNotes(e.target.value)}
              onBlur={() => notes != null && notes !== deck.notes && updateDeck({ notes })}
              style={{ width: '100%', height: 84, fontSize: 12.5 }}
            />
          </section>
          {editable && (
            <section className="col" style={{ gap: 8 }}>
              <h3 style={{ fontFamily: 'var(--font)', fontSize: 13 }}>Deck settings</h3>
              <label className="field">
                Cover card
                <Select aria-label="Cover card" value={deck.coverKey || ''} onChange={(v) => updateDeck({ coverKey: v || null })} options={[{ value: '', label: 'Commander / first card' }, ...Array.from(new Set(deck.cards.map((c) => c.def))).map((k) => ({ value: k, label: deck.defs[k]?.name ?? k }))]} />
              </label>
              {deck.ownerId === me.id && (
                <label className="field">
                  Folder
                  <Select aria-label="Folder" value={deck.folderId ?? ''} onChange={(v) => updateDeck({ folderId: v === '' ? null : Number(v) })} options={[{ value: '' as number | '', label: 'No folder' }, ...(folders.data || []).map((f) => ({ value: f.id as number | '', label: f.name }))]} />
                </label>
              )}
              <SleevePicker deck={deck} onSet={(id) => updateDeck({ sleeveAsset: id })} />
            </section>
          )}
        </SidePanel>
      </div>
      {modal === 'hand' && <SampleHand deck={deck} onClose={() => setModal(null)} />}
      {modal === 'proxy' && <ProxyModal deck={deck} selected={sel} onClose={() => setModal(null)} />}
      {modal === 'export' && <ExportModal deck={deck} onClose={() => setModal(null)} />}
      {modal === 'tags' && <TagManager deck={deck} onDeck={setDeck} onClose={() => setModal(null)} />}
      {modal === 'delete' && (
        <Confirm
          title="Delete this deck?"
          text={
            <>
              <strong style={{ color: 'var(--tx)' }}>{deck.name}</strong> and everything in it (sideboard, maybeboard, notes) will be permanently deleted
              {deck.ownerId !== me.id ? ` from ${deck.ownerName || 'its owner'}'s decks` : ''}. Others will no longer be able to use it. Past match history keeps the deck's name. This can't be undone.
            </>
          }
          confirm="Delete deck"
          danger
          onNo={() => setModal(null)}
          onYes={async () => {
            try {
              await api.del(`api/decks/${deck.id}`);
              qc.invalidateQueries({ queryKey: ['decks'] });
              toast(`Deleted “${deck.name}”`);
              nav('/decks');
            } catch (e) {
              toast(errMsg(e));
              setModal(null);
            }
          }}
        />
      )}
      {modal && typeof modal === 'object' && deck.cards.find((c) => c.id === modal.card) && (
        <CardDetail
          deck={deck}
          card={deck.cards.find((c) => c.id === modal.card)!}
          reasons={reasonsOf(deck.cards.find((c) => c.id === modal.card)!)}
          editable={editable}
          onPatch={(p) => patchCard(modal.card, p)}
          onTags={(add, remove) => tagCards([modal.card], add, remove)}
          onOverrideAll={(oracleId, front, back) => mutate(api.post<Deck>(`api/decks/${deckId}/override-all`, { oracleId, front, back }))}
          onClose={() => setModal(null)}
        />
      )}
    </div>
  );
}

function SleevePicker({ deck, onSet }: { deck: Deck; onSet: (id: number | null) => void }) {
  const toast = useApp((s) => s.toast);
  const ref = useRef<HTMLInputElement>(null);
  return (
    <div className="row">
      <CardImage back sleeve={deck.sleeveAsset ? 'a/' + deck.sleeveAsset : null} style={{ width: 40, height: 56, borderRadius: 4 }} />
      <div className="col grow" style={{ gap: 4 }}>
        <span style={{ fontSize: 12, color: 'var(--mu)' }}>Card back (sleeve) · seen by all players</span>
        <div className="row" style={{ gap: 6 }}>
          <button className="mbtn" onClick={() => ref.current?.click()}>
            Upload…
          </button>
          {deck.sleeveAsset && (
            <button className="mbtn" onClick={() => onSet(null)}>
              Default
            </button>
          )}
        </div>
      </div>
      <input
        ref={ref}
        type="file"
        accept="image/*"
        hidden
        onChange={async (e) => {
          const f = e.target.files?.[0];
          if (!f) return;
          try {
            onSet(await api.upload(f, 'sleeve'));
          } catch (x) {
            toast(errMsg(x));
          }
        }}
      />
    </div>
  );
}
