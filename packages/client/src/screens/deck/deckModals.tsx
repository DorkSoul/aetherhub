import { useQuery } from '@tanstack/react-query';
import { Select } from '../../ui/Select';
import { useEffect, useMemo, useRef, useState } from 'react';
import type { CardDef, Deck, DeckCard, DeckTag, ExportFormat, RecentTag, Section, TagScope } from '@aetherhub/shared';
import { deckTagNames, DEFAULT_TAGS, exportDeck, normalizeTag, RECENT_TAGS, SECTION_NAMES, TAG_COLORS, tagScope, turnKind } from '@aetherhub/shared';
import { api, download, saveBlob } from '../../lib/api';
import { errMsg, useApp } from '../../lib/store';
import { RulingsList } from '../../ui/rulings';
import { CardImage, Modal, TurnableCard, TurnButton, useDebounced } from '../../ui/ui';

/** Inline style for a `.tagc` chip in a tag's colour (none = accent). */
export function tagStyle(color: string | null | undefined): React.CSSProperties | undefined {
  return color ? ({ '--tagc': color } as React.CSSProperties) : undefined;
}

export const tagColorOf = (deck: Deck, name: string) => deck.tags.find((t) => t.name === name)?.color ?? null;

export type TagAdd = { name: string; scope?: TagScope };

export const SCOPE_TITLE: Record<TagScope, string> = { global: 'Global tag: follows this card into all your decks', deck: 'Deck tag: only in this deck' };

/** `.tagc` class for a tag, marking deck-only tags. */
export const tagClass = (scope: TagScope, extra = '') => 'tagc' + (scope === 'deck' ? ' deck' : '') + extra;

/**
 * Tags to offer for some cards: the 13 most recently used (topped up with this deck's tags, the global ones,
 * then the defaults), plus any other tags already on those cards so they can be removed.
 */
export function tagChoices(deck: Deck, cards: DeckCard[]): { recent: RecentTag[]; onCards: RecentTag[] } {
  const recent: RecentTag[] = [];
  const scopeOf = (name: string, fallback: TagScope) => deck.tags.find((t) => t.name === name)?.scope ?? fallback;
  const push = (name: string, scope: TagScope) => {
    if (recent.length < RECENT_TAGS && !recent.some((x) => x.name === name)) recent.push({ name, scope: scopeOf(name, scope) });
  };
  deck.recentTags.forEach((t) => push(t.name, t.scope));
  deckTagNames(deck.tags, deck.cards).forEach((n) => push(n, tagScope(deck.tags, n)));
  deck.tags.forEach((t) => push(t.name, t.scope));
  DEFAULT_TAGS.forEach((n) => push(n, 'global'));
  const onCards = Array.from(new Set(cards.flatMap((c) => c.tags)))
    .filter((n) => !recent.some((x) => x.name === n))
    .map((name) => ({ name, scope: tagScope(deck.tags, name) }));
  return { recent, onCards };
}

/** The def to show for a deck card, with its image overrides applied. */
export function shownDef(c: DeckCard, defs: Record<string, CardDef>): CardDef | undefined {
  const d = defs[c.def];
  if (!d || (!c.frontAsset && !c.backAsset)) return d;
  const img = d.img.slice();
  if (c.frontAsset) img[0] = 'a/' + c.frontAsset;
  if (c.backAsset) img[1] = 'a/' + c.backAsset;
  return { ...d, img };
}

export function CardDetail({
  deck,
  card,
  reasons,
  editable,
  onPatch,
  onTags,
  onOverrideAll,
  onClose,
}: {
  deck: Deck;
  card: DeckCard;
  reasons: string[];
  editable: boolean;
  onPatch: (p: Partial<DeckCard> & { key?: string }) => Promise<void>;
  onTags: (add: TagAdd[], remove: string[]) => Promise<void>;
  onOverrideAll: (oracleId: string, front: number | null, back: number | null) => Promise<void>;
  onClose: () => void;
}) {
  const toast = useApp((s) => s.toast);
  const def = deck.defs[card.def];
  const shown = shownDef(card, deck.defs);
  const printings = useQuery({ queryKey: ['printings', def?.oracleId], queryFn: () => api.get<CardDef[]>(`api/cards/printings/${def!.oracleId}`), enabled: !!def?.oracleId });
  const [face, setFace] = useState<0 | 1>(0);
  const [allDecks, setAllDecks] = useState(false);
  const [pick, setPick] = useState(false);
  const [gallery, setGallery] = useState(false);
  const [urlIn, setUrlIn] = useState<string | null>(null);
  const [tagIn, setTagIn] = useState('');
  const [deckOnly, setDeckOnly] = useState(false);
  const [sq, setSq] = useState(def?.name || '');
  const dsq = useDebounced(sq, 300);
  const search = useQuery({ queryKey: ['override-search', dsq], queryFn: () => api.get<{ cards: CardDef[] }>(`api/cards/search?limit=24&q=${encodeURIComponent(dsq)}`), enabled: pick && dsq.length > 1 });
  const file = useRef<HTMLInputElement>(null);
  const [showFace, setShowFace] = useState(0);
  const printList = useRef<HTMLDivElement>(null);
  const scrolledTo = useRef(false);
  // Once, when the list first loads: bring the current printing into view. Never again, so the list stays put.
  useEffect(() => {
    const el = printList.current;
    if (scrolledTo.current || !el || !printings.data) return;
    scrolledTo.current = true;
    const on = el.querySelector<HTMLElement>('[data-on]');
    if (on) el.scrollTop = on.offsetTop - el.clientHeight / 2 + on.clientHeight / 2;
  }, [printings.data]);
  if (!def) return null;
  const choices = tagChoices(deck, [card]);
  const tags = [...choices.recent, ...choices.onCards];

  const setOverride = async (assetId: number | null) => {
    const patch = face === 0 ? { frontAsset: assetId } : { backAsset: assetId };
    if (allDecks && def.oracleId) await onOverrideAll(def.oracleId, face === 0 ? assetId : card.frontAsset ?? null, face === 1 ? assetId : card.backAsset ?? null);
    else await onPatch(patch);
  };
  const fromUrl = async (u: string) => {
    try {
      const r = await api.post<{ id: number }>('api/assets/url', { url: u, kind: 'override' });
      await setOverride(r.id);
      toast('Image override saved');
    } catch (e) {
      toast(errMsg(e));
    }
  };

  return (
    <Modal title={def.name} onClose={onClose} width={960}>
      <div className="row" style={{ alignItems: 'stretch', flexWrap: 'wrap' }}>
        <div style={{ width: 330, background: 'var(--panel)', padding: 26, display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 14 }}>
          <div style={{ position: 'relative' }}>
            <CardImage def={shown} face={showFace} size="large" style={{ width: 270, height: 378, borderRadius: 13, boxShadow: reasons.length ? '0 0 0 3px #e2604f' : '0 12px 30px rgba(0,0,0,.5)' }} />
            <TurnButton def={shown} onTurn={() => setShowFace(showFace ? 0 : 1)} size={36} />
          </div>
          {turnKind(shown) && (
            <button className="tb sm" onClick={() => setShowFace(showFace ? 0 : 1)}>
              ⟲ {turnKind(shown) === 'flip' ? (showFace ? 'Rotate back' : 'Rotate 180°') : turnKind(shown) === 'meld' ? (showFace ? 'Show front' : 'Show melded card') : `Turn to ${showFace ? 'front' : 'back'}`}
            </button>
          )}
          {reasons.length > 0 && <div className="warnbox" style={{ width: '100%', fontSize: 12, display: 'block' }}>{reasons.join(' · ')}</div>}
          {!def.custom && (
            <div style={{ width: '100%' }}>
              <RulingsList oracleId={def.oracleId} maxHeight={320} />
            </div>
          )}
        </div>
        <div className="col" style={{ flex: '1 1 400px', padding: '22px 24px', gap: 16, minWidth: 0 }}>
          <div style={{ fontSize: 12.5, color: 'var(--mu)' }}>{def.typeLine}</div>
          <div style={{ fontSize: 13, color: 'var(--tx3)', whiteSpace: 'pre-line', lineHeight: 1.5 }}>{def.oracleText}</div>
          {editable ? (
            <>
              <div className="row" style={{ gap: 18, flexWrap: 'wrap' }}>
                <div className="row" style={{ gap: 8 }}>
                  <span style={{ fontSize: 12, color: 'var(--mu2)' }}>Quantity</span>
                  <button className="sq" aria-label="Fewer" onClick={() => onPatch({ qty: card.qty - 1 }).then(() => card.qty <= 1 && onClose())}>
                    −
                  </button>
                  <span className="mono" style={{ width: 22, textAlign: 'center' }}>
                    {card.qty}
                  </span>
                  <button className="sq" aria-label="More" onClick={() => onPatch({ qty: card.qty + 1 })}>
                    +
                  </button>
                </div>
                <label className="row" style={{ gap: 8, fontSize: 12, color: 'var(--mu2)' }}>
                  Section
                  <Select aria-label="Section" value={card.section} onChange={(v) => onPatch({ section: v })} options={(Object.keys(SECTION_NAMES) as Section[]).map((s) => ({ value: s, label: SECTION_NAMES[s] }))} />
                </label>
                <label className="row" style={{ gap: 6, fontSize: 12, color: 'var(--mu)' }}>
                  <input type="checkbox" checked={card.foil} onChange={() => onPatch({ foil: !card.foil })} /> Foil (stored only)
                </label>
              </div>
              <div>
                <div style={{ fontSize: 12, color: 'var(--mu2)', marginBottom: 6 }}>
                  Tags <span style={{ color: 'var(--mu4)' }}>· recently used · global tags follow this card into all your decks</span>
                </div>
                <div style={{ display: 'flex', flexWrap: 'wrap', gap: 5, alignItems: 'center' }}>
                  {tags.map((t) => {
                    const on = card.tags.includes(t.name);
                    return (
                      <button key={t.name} className={tagClass(t.scope, on ? '' : ' off')} title={SCOPE_TITLE[t.scope]} aria-pressed={on} style={tagStyle(tagColorOf(deck, t.name))} onClick={() => onTags(on ? [] : [t], on ? [t.name] : [])}>
                        #{t.name}
                      </button>
                    );
                  })}
                  <input
                    className="inp"
                    style={{ height: 26, width: 120, fontSize: 12 }}
                    placeholder="+ new tag"
                    aria-label="New tag"
                    value={tagIn}
                    onChange={(e) => setTagIn(e.target.value)}
                    onKeyDown={(e) => {
                      const t = normalizeTag(tagIn);
                      if (e.key === 'Enter' && t) {
                        onTags([{ name: t, scope: deckOnly ? 'deck' : 'global' }], []);
                        setTagIn('');
                      }
                    }}
                  />
                  <label className="row" style={{ gap: 5, fontSize: 11.5, color: 'var(--mu)' }}>
                    <input type="checkbox" checked={deckOnly} onChange={() => setDeckOnly(!deckOnly)} /> This deck only
                  </label>
                </div>
              </div>
              <div>
                <div className="row" style={{ fontSize: 12, color: 'var(--mu2)', marginBottom: 6, gap: 8 }}>
                  <span>
                    Printing <span style={{ color: 'var(--mu4)' }}>· default is the newest non-promo printing with a normal image</span>
                  </span>
                  <button className="tb sm" style={{ marginLeft: 'auto' }} disabled={!printings.data} onClick={() => setGallery(true)}>
                    View all{printings.data ? ` (${printings.data.length})` : ''}
                  </button>
                </div>
                {gallery && printings.data && (
                  <PrintingsGallery
                    name={def.name}
                    printings={printings.data}
                    current={card.def}
                    onPick={(key) => {
                      setGallery(false);
                      if (key !== card.def) onPatch({ key });
                    }}
                    onClose={() => setGallery(false)}
                  />
                )}
                {/* Release order, never re-sorted, so picking a printing doesn't move your place in the list. */}
                <div ref={printList} className="lg" style={{ position: 'relative', maxHeight: 170, overflowY: 'auto', border: '1px solid var(--line)', borderRadius: 9 }}>
                  {(printings.data || [def]).map((p) => {
                    const on = p.key === card.def;
                    return (
                      <button
                        key={p.key}
                        data-on={on || undefined}
                        onClick={() => onPatch({ key: p.key })}
                        className="row"
                        style={{ width: '100%', border: 0, borderBottom: '1px solid var(--line)', background: on ? 'var(--surface4)' : 'transparent', color: 'var(--tx)', padding: '8px 12px', fontSize: 12.5, cursor: 'pointer', textAlign: 'left', gap: 12 }}
                      >
                        <span style={{ width: 14, height: 14, borderRadius: 7, border: `2px solid ${on ? 'var(--acc)' : 'var(--line5)'}`, background: on ? 'var(--acc)' : 'transparent', flexShrink: 0 }} />
                        <span className="mono" style={{ width: 50 }}>
                          {p.set?.toUpperCase()}
                        </span>
                        <span className="grow" style={{ color: 'var(--tx3)' }}>
                          {p.setName} #{p.collectorNumber}
                          {p.releasedAt ? ` · ${p.releasedAt.slice(0, 4)}` : ''}
                        </span>
                        <span className="mono" style={{ color: 'var(--mu)' }}>
                          {p.usd != null ? `$${p.usd.toFixed(2)}` : '—'}
                        </span>
                      </button>
                    );
                  })}
                </div>
              </div>
              <div className="card-box" style={{ borderRadius: 10, padding: 12, background: 'var(--surface2)' }}>
                <div style={{ fontSize: 12.5, fontWeight: 600, marginBottom: 8 }}>
                  Image override <span style={{ fontWeight: 400, color: 'var(--mu2)' }}>· front and back set separately · cached by the server</span>
                </div>
                <div className="row" style={{ gap: 8, flexWrap: 'wrap' }}>
                  <div className="segs">
                    <button className={'seg' + (face === 0 ? ' on' : '')} onClick={() => setFace(0)}>
                      Front{card.frontAsset ? ' ●' : ''}
                    </button>
                    <button className={'seg' + (face === 1 ? ' on' : '')} onClick={() => setFace(1)}>
                      Back{card.backAsset ? ' ●' : ''}
                    </button>
                  </div>
                  <button className="mbtn" onClick={() => setPick(!pick)}>
                    Pick from Scryfall search
                  </button>
                  <button className="mbtn" onClick={() => setUrlIn('')}>
                    Paste image URL
                  </button>
                  <button className="mbtn" onClick={() => file.current?.click()}>
                    Upload file (≤10 MB)
                  </button>
                  {(face === 0 ? card.frontAsset : card.backAsset) && (
                    <button className="mbtn" onClick={() => setOverride(null)}>
                      Remove
                    </button>
                  )}
                  <label className="row" style={{ gap: 6, fontSize: 12, color: 'var(--mu)', marginLeft: 'auto' }}>
                    <input type="checkbox" checked={allDecks} onChange={() => setAllDecks(!allDecks)} /> Apply to all my decks
                  </label>
                  <input
                    ref={file}
                    type="file"
                    accept="image/*"
                    hidden
                    onChange={async (e) => {
                      const f = e.target.files?.[0];
                      if (!f) return;
                      try {
                        await setOverride(await api.upload(f, 'override'));
                      } catch (x) {
                        toast(errMsg(x));
                      }
                    }}
                  />
                </div>
                {urlIn != null && (
                  <div className="row" style={{ marginTop: 10 }}>
                    <input className="inp grow" placeholder="https://…" value={urlIn} onChange={(e) => setUrlIn(e.target.value)} autoFocus />
                    <button className="tb" onClick={() => urlIn && fromUrl(urlIn).then(() => setUrlIn(null))}>
                      Use image
                    </button>
                  </div>
                )}
                {pick && (
                  <div style={{ marginTop: 10 }}>
                    <input className="inp" style={{ width: '100%' }} value={sq} onChange={(e) => setSq(e.target.value)} placeholder="Search any card…" />
                    <div className="lg" style={{ display: 'flex', gap: 8, flexWrap: 'wrap', maxHeight: 220, overflowY: 'auto', marginTop: 8 }}>
                      {(search.data?.cards || [])
                        .filter((c) => c.printingId)
                        .map((c) => (
                          <button
                            key={c.key}
                            title={`${c.name} (${c.set?.toUpperCase()})`}
                            style={{ border: 0, padding: 0, background: 'none', cursor: 'pointer' }}
                            onClick={() => {
                              const id = c.printingId!;
                              fromUrl(`https://cards.scryfall.io/large/${face && c.img.length > 1 ? 'back' : 'front'}/${id[0]}/${id[1]}/${id}.jpg`);
                              setPick(false);
                            }}
                          >
                            <CardImage def={c} face={face && c.img.length > 1 ? 1 : 0} style={{ width: 80, height: 112, borderRadius: 5 }} />
                          </button>
                        ))}
                    </div>
                  </div>
                )}
              </div>
            </>
          ) : (
            <div className="row" style={{ fontSize: 12.5, color: 'var(--mu)', gap: 5, flexWrap: 'wrap' }}>
              {card.qty}× · {SECTION_NAMES[card.section]} · {def.setName}
              {card.tags.map((t) => (
                <span key={t} className={tagClass(tagScope(deck.tags, t))} title={SCOPE_TITLE[tagScope(deck.tags, t)]} style={tagStyle(tagColorOf(deck, t))}>
                  #{t}
                </span>
              ))}
            </div>
          )}
        </div>
      </div>
    </Modal>
  );
}

/** Every printing of a card as images, in release order, to compare and pick one. */
function PrintingsGallery({ name, printings, current, onPick, onClose }: { name: string; printings: CardDef[]; current: string; onPick: (key: string) => void; onClose: () => void }) {
  const [turned, setTurned] = useState<Record<string, boolean>>({});
  const grid = useRef<HTMLDivElement>(null);
  // Bring the current printing into view on open.
  useEffect(() => {
    grid.current?.querySelector<HTMLElement>('[data-on]')?.scrollIntoView({ block: 'center' });
  }, []);
  return (
    <Modal title={`${name} · ${printings.length} printing${printings.length === 1 ? '' : 's'}`} onClose={onClose} width={1100}>
      <div ref={grid} style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(180px, 1fr))', gap: 16, padding: 20 }}>
        {printings.map((p) => {
          const on = p.key === current;
          return (
            // A div, not a button: the turn button for two-faced cards sits inside it.
            <div
              key={p.key}
              role="button"
              tabIndex={0}
              data-on={on || undefined}
              aria-pressed={on}
              title={`${p.setName} #${p.collectorNumber}`}
              onClick={() => onPick(p.key)}
              onKeyDown={(e) => e.target === e.currentTarget && (e.key === 'Enter' || e.key === ' ') && (e.preventDefault(), onPick(p.key))}
              className="col"
              style={{ cursor: 'pointer', gap: 6, minWidth: 0 }}
            >
              <div style={{ position: 'relative', width: '100%', aspectRatio: '63 / 88' }}>
                <CardImage def={p} face={turned[p.key] ? 1 : 0} style={{ width: '100%', height: '100%', borderRadius: 9, boxShadow: on ? '0 0 0 3px var(--acc)' : '0 6px 14px rgba(0,0,0,.35)' }} />
                <TurnButton def={p} onTurn={() => setTurned((t) => ({ ...t, [p.key]: !t[p.key] }))} size={28} />
              </div>
              <div className="row" style={{ gap: 6, fontSize: 12 }}>
                <span className="mono" style={{ color: on ? 'var(--acc)' : 'var(--tx3)' }}>
                  {p.set?.toUpperCase()} #{p.collectorNumber}
                </span>
                <span className="mono" style={{ marginLeft: 'auto', color: 'var(--mu)' }}>
                  {p.usd != null ? `$${p.usd.toFixed(2)}` : '—'}
                </span>
              </div>
              <div style={{ fontSize: 11.5, color: 'var(--mu2)', whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis', marginTop: -4 }}>
                {p.setName}
                {p.releasedAt ? ` · ${p.releasedAt.slice(0, 4)}` : ''}
              </div>
            </div>
          );
        })}
      </div>
    </Modal>
  );
}

function shuffle<T>(a: T[]): T[] {
  const r = a.slice();
  for (let i = r.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [r[i], r[j]] = [r[j], r[i]];
  }
  return r;
}

export function SampleHand({ deck, onClose }: { deck: Deck; onClose: () => void }) {
  const lib = useMemo(() => deck.cards.filter((c) => c.section === 'main').flatMap((c) => Array.from({ length: c.qty }, () => c)), [deck]);
  const deal = (m: number) => {
    const s = shuffle(lib);
    return { hand: s.slice(0, 7), rest: s.slice(7), mulls: m };
  };
  const [st, setSt] = useState(() => deal(0));
  return (
    <Modal
      title={
        <>
          Sample hand{' '}
          <span style={{ fontSize: 13, color: 'var(--mu)', fontFamily: 'var(--font)', fontWeight: 400 }}>
            · {7 - st.mulls} to keep{st.mulls ? ` · mulligan ${st.mulls} (put ${st.mulls} on the bottom)` : ''} · {st.rest.length} left in library
          </span>
        </>
      }
      onClose={onClose}
      width={1180}
      head={
        <>
          <button className="tb" onClick={() => setSt(deal(Math.min(6, st.mulls + 1)))}>
            Mulligan (London)
          </button>
          <button className="tb" onClick={() => st.rest.length && setSt({ ...st, hand: [...st.hand, st.rest[0]], rest: st.rest.slice(1) })}>
            Draw 1
          </button>
          <button className="tb pri" onClick={() => setSt(deal(0))}>
            New hand
          </button>
        </>
      }
    >
      <div style={{ padding: '26px 20px', display: 'flex', gap: 12, justifyContent: 'center', flexWrap: 'wrap' }}>
        {st.hand.map((c, i) => (
          <TurnableCard key={i} turnKey={`hand:${st.mulls}:${i}:${c.id}`} def={shownDef(c, deck.defs)} style={{ width: 150, height: 210, borderRadius: 9, boxShadow: '0 8px 20px rgba(0,0,0,.45)' }} />
        ))}
        {!lib.length && <div style={{ color: 'var(--mu)' }}>The main deck is empty.</div>}
      </div>
    </Modal>
  );
}

export function ProxyModal({ deck, selected, onClose }: { deck: Deck; selected: number[]; onClose: () => void }) {
  const toast = useApp((s) => s.toast);
  const [paper, setPaper] = useState<'A4' | 'Letter'>('A4');
  const [scope, setScope] = useState<'deck' | 'sel'>(selected.length ? 'sel' : 'deck');
  const [cut, setCut] = useState(true);
  const [bleed, setBleed] = useState(false);
  const [busy, setBusy] = useState(false);
  const chosen = scope === 'sel' ? deck.cards.filter((c) => selected.includes(c.id)) : deck.cards.filter((c) => ['commander', 'companion', 'main'].includes(c.section));
  const count = chosen.reduce((s, c) => s + c.qty, 0);
  const cells = chosen.flatMap((c) => Array.from({ length: c.qty }, () => c)).slice(0, 9);
  return (
    <Modal
      title="Proxy PDF"
      onClose={onClose}
      width={760}
      foot={
        <>
          <button className="tb" onClick={onClose}>
            Cancel
          </button>
          <button
            className="tb pri"
            disabled={busy || !count}
            onClick={async () => {
              setBusy(true);
              try {
                await download(`api/decks/${deck.id}/proxy`, `${deck.name} proxies.pdf`, { paper, cutLines: cut, bleed, cardIds: scope === 'sel' ? selected : undefined });
                onClose();
              } catch (e) {
                toast(errMsg(e));
              } finally {
                setBusy(false);
              }
            }}
          >
            {busy ? 'Generating…' : 'Generate PDF'}
          </button>
        </>
      }
    >
      <div className="row" style={{ alignItems: 'stretch', flexWrap: 'wrap' }}>
        <div style={{ width: 300, background: 'var(--panel)', padding: 22, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
          <div style={{ width: 212, height: paper === 'A4' ? 300 : 274, background: '#f7f5f0', borderRadius: 3, padding: '12px 10px', display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: bleed ? 2 : 5, alignContent: 'start', boxShadow: '0 10px 24px rgba(0,0,0,.5)' }}>
            {cells.map((c, i) => (
              <div key={i} style={{ aspectRatio: '63 / 88', outline: cut ? '1px dashed #b6b0a4' : 'none', outlineOffset: 1 }}>
                <CardImage def={shownDef(c, deck.defs)} style={{ width: '100%', height: '100%', borderRadius: 2 }} />
              </div>
            ))}
          </div>
        </div>
        <div className="col" style={{ flex: '1 1 300px', padding: '22px 24px', gap: 14 }}>
          <div style={{ fontSize: 12.5, color: 'var(--mu)', lineHeight: 1.5 }}>
            3×3 per sheet at 63×88 mm. Uses your image overrides. {count} cards → {Math.ceil(count / 9)} {paper} sheet{Math.ceil(count / 9) === 1 ? '' : 's'}.
          </div>
          <div className="row">
            <span style={{ width: 90, fontSize: 12.5, color: 'var(--tx3)' }}>Paper</span>
            {(['A4', 'Letter'] as const).map((p) => (
              <button key={p} className={'mbtn' + (paper === p ? ' on' : '')} onClick={() => setPaper(p)}>
                {p}
              </button>
            ))}
          </div>
          <div className="row">
            <span style={{ width: 90, fontSize: 12.5, color: 'var(--tx3)' }}>Cards</span>
            <button className={'mbtn' + (scope === 'deck' ? ' on' : '')} onClick={() => setScope('deck')}>
              Whole deck
            </button>
            <button className={'mbtn' + (scope === 'sel' ? ' on' : '')} onClick={() => setScope('sel')} disabled={!selected.length} title={selected.length ? '' : 'Ctrl-click cards in the deck to select them'}>
              Selected cards ({selected.length})
            </button>
          </div>
          <label className="row" style={{ gap: 8, fontSize: 12.5, color: 'var(--tx3)' }}>
            <input type="checkbox" checked={cut} onChange={() => setCut(!cut)} /> Cut lines
          </label>
          <label className="row" style={{ gap: 8, fontSize: 12.5, color: 'var(--tx3)' }}>
            <input type="checkbox" checked={bleed} onChange={() => setBleed(!bleed)} /> Bleed (1 mm)
          </label>
        </div>
      </div>
    </Modal>
  );
}

export function ExportModal({ deck, onClose }: { deck: Deck; onClose: () => void }) {
  const [fmt, setFmt] = useState<ExportFormat>('moxfield');
  const [copied, setCopied] = useState(false);
  const hasTags = deck.cards.some((c) => c.tags.length);
  const [withTags, setWithTags] = useState(hasTags);
  const text = exportDeck(deck.cards, deck.defs, fmt, { tags: withTags, globalTags: new Set(deck.tags.filter((t) => t.scope === 'global').map((t) => t.name)) });
  const ext = fmt === 'mtgo' ? '.dek' : '.txt';
  return (
    <Modal
      title="Export"
      onClose={onClose}
      width={640}
      foot={
        <>
          <button
            className="tb"
            onClick={() => {
              navigator.clipboard?.writeText(text).then(() => setCopied(true));
            }}
          >
            {copied ? 'Copied' : 'Copy'}
          </button>
          <button className="tb pri" onClick={() => saveBlob(new Blob([text], { type: 'text/plain' }), deck.name + ext)}>
            Download {ext}
          </button>
        </>
      }
    >
      <div className="col" style={{ padding: '14px 20px', gap: 12 }}>
        <div className="row" style={{ gap: 6 }}>
          {(
            [
              ['moxfield', 'Plain text (Moxfield)'],
              ['arena', 'Arena'],
              ['mtgo', 'MTGO .dek'],
            ] as const
          ).map(([k, l]) => (
            <button key={k} className={'mbtn' + (fmt === k ? ' on' : '')} onClick={() => (setFmt(k), setCopied(false))}>
              {l}
            </button>
          ))}
          {fmt === 'moxfield' && hasTags && (
            <label className="row" style={{ gap: 6, fontSize: 12, color: 'var(--mu)', marginLeft: 'auto' }}>
              <input type="checkbox" checked={withTags} onChange={() => (setWithTags(!withTags), setCopied(false))} /> Include tags
            </label>
          )}
        </div>
        <textarea readOnly aria-label="Exported deck list" value={text} className="inp mono lg" style={{ height: 320, fontSize: 12, resize: 'none' }} />
      </div>
    </Modal>
  );
}

/** Tag lists for this deck: global tags (the owner's, in all their decks) and this deck's own. Order, colour, scope, rename, delete. */
export function TagManager({ deck, onDeck, onClose }: { deck: Deck; onDeck: (d: Deck) => void; onClose: () => void }) {
  const toast = useApp((s) => s.toast);
  const used = deckTagNames(deck.tags, deck.cards);
  const globals = deck.tags.filter((t) => t.scope === 'global').map((t) => t.name);
  const locals = [...deck.tags.filter((t) => t.scope === 'deck').map((t) => t.name), ...used.filter((n) => !deck.tags.some((t) => t.name === n))];
  const all = [...globals, ...locals];
  const [renaming, setRenaming] = useState<{ from: string; to: string } | null>(null);
  const [painting, setPainting] = useState<string | null>(null);
  const [deleting, setDeleting] = useState<string | null>(null);
  const [newTag, setNewTag] = useState('');
  const [newDeckOnly, setNewDeckOnly] = useState(false);
  const call = async (p: Promise<Deck>) => {
    try {
      onDeck(await p);
    } catch (e) {
      toast(errMsg(e));
    }
  };
  const full = (): DeckTag[] => [...globals.map((name) => ({ name, color: tagColorOf(deck, name), scope: 'global' as const })), ...locals.map((name) => ({ name, color: tagColorOf(deck, name), scope: 'deck' as const }))];
  const saveList = (tags: DeckTag[]) => call(api.put<Deck>(`api/decks/${deck.id}/tags`, { tags }));
  const move = (t: string, d: number) => {
    const l = full();
    const i = l.findIndex((x) => x.name === t);
    const j = i + d;
    if (j < 0 || j >= l.length || l[j].scope !== l[i].scope) return;
    [l[i], l[j]] = [l[j], l[i]];
    saveList(l);
  };
  const copies = (t: string) => deck.cards.filter((c) => c.tags.includes(t)).reduce((s, c) => s + c.qty, 0);
  const rename = () => {
    if (!renaming) return;
    const to = normalizeTag(renaming.to);
    if (to && to !== renaming.from) call(api.post<Deck>(`api/decks/${deck.id}/tags/rename`, { from: renaming.from, to }));
    setRenaming(null);
  };
  const add = () => {
    const t = normalizeTag(newTag);
    if (t && !all.includes(t)) saveList([...full(), { name: t, color: null, scope: newDeckOnly ? 'deck' : 'global' }]);
    setNewTag('');
  };

  const row = (t: string, scope: TagScope, i: number, list: string[]) => {
    const color = tagColorOf(deck, t);
    const merge = renaming?.from === t && normalizeTag(renaming.to) !== t && all.includes(normalizeTag(renaming.to));
    const n = copies(t);
    return (
      <div key={t} style={{ borderBottom: '1px solid var(--line)' }}>
        <div className="row" style={{ gap: 8, padding: '8px 20px' }}>
          <div className="col" style={{ gap: 1 }}>
            <button className="lnk" style={{ fontSize: 10, lineHeight: 1, color: 'var(--mu2)' }} aria-label={`Move #${t} up`} disabled={i === 0} onClick={() => move(t, -1)}>
              ▲
            </button>
            <button className="lnk" style={{ fontSize: 10, lineHeight: 1, color: 'var(--mu2)' }} aria-label={`Move #${t} down`} disabled={i === list.length - 1} onClick={() => move(t, 1)}>
              ▼
            </button>
          </div>
          <button className="sq sm" title="Colour" aria-label={`Colour for #${t}`} onClick={() => setPainting(painting === t ? null : t)} style={{ background: color || undefined }}>
            {color ? '' : '·'}
          </button>
          {renaming?.from === t ? (
            <input
              className="inp"
              autoFocus
              aria-label={`Rename #${t}`}
              value={renaming.to}
              onChange={(e) => setRenaming({ from: t, to: e.target.value })}
              onKeyDown={(e) => {
                if (e.key === 'Enter') rename();
                if (e.key === 'Escape') (e.stopPropagation(), setRenaming(null));
              }}
              onBlur={() => setRenaming(null)}
              style={{ height: 26, width: 150, fontSize: 12.5 }}
            />
          ) : (
            <span className={tagClass(scope)} style={{ ...tagStyle(color), height: 22, fontSize: 12 }}>
              #{t}
            </span>
          )}
          <span style={{ fontSize: 12, color: merge ? 'var(--acc-tx)' : 'var(--mu2)' }}>{merge ? `Enter merges into #${normalizeTag(renaming!.to)}` : `${n} here`}</span>
          <div className="grow" />
          <button className="mbtn" title={scope === 'global' ? 'Copy it onto the cards in each of your decks as a deck tag, and stop it following cards around' : 'Make it follow these cards into all your decks'} onClick={() => call(api.post<Deck>(`api/decks/${deck.id}/tags/scope`, { name: t, scope: scope === 'global' ? 'deck' : 'global' }))}>
            {scope === 'global' ? 'Make deck-only' : 'Make global'}
          </button>
          <button className="mbtn" onClick={() => setRenaming({ from: t, to: t })}>
            Rename
          </button>
          <button className="mbtn" style={{ color: 'var(--danger-tx)' }} onClick={() => setDeleting(deleting === t ? null : t)}>
            Delete
          </button>
        </div>
        {painting === t && (
          <div className="row" style={{ gap: 6, padding: '0 20px 10px 58px' }}>
            {TAG_COLORS.map((c) => (
              <button key={c} className="sq sm" aria-label={`Colour ${c}`} onClick={() => (saveList(full().map((x) => (x.name === t ? { ...x, color: c } : x))), setPainting(null))} style={{ background: c, borderColor: c === color ? 'var(--tx)' : c }} />
            ))}
            <button className="mbtn" onClick={() => (saveList(full().map((x) => (x.name === t ? { ...x, color: null } : x))), setPainting(null))}>
              No colour
            </button>
          </div>
        )}
        {deleting === t && (
          <div className="row" style={{ gap: 8, padding: '0 20px 10px 58px', fontSize: 12.5, color: 'var(--danger-tx)', flexWrap: 'wrap' }}>
            {scope === 'global' ? `Remove #${t} from every card in all your decks?` : `Remove #${t} from ${n} card${n === 1 ? '' : 's'}?`} The cards stay.
            <button className="tb sm danger" onClick={() => (call(api.post<Deck>(`api/decks/${deck.id}/tags/delete`, { name: t })), setDeleting(null))}>
              Delete tag
            </button>
            <button className="mbtn" onClick={() => setDeleting(null)}>
              Cancel
            </button>
          </div>
        )}
      </div>
    );
  };
  const head = (title: string, note: string) => (
    <div style={{ padding: '12px 20px 6px', borderBottom: '1px solid var(--line)' }}>
      <div style={{ fontSize: 13, fontWeight: 600 }}>{title}</div>
      <div style={{ fontSize: 12, color: 'var(--mu2)', marginTop: 2 }}>{note}</div>
    </div>
  );

  return (
    <Modal title="Manage tags" onClose={onClose} width={680}>
      {head('Global tags', 'They belong to the deck’s owner and follow a card into all their decks. New tags are global unless you tick “This deck only”.')}
      {!globals.length && <div style={{ padding: '14px 20px', color: 'var(--mu2)', fontSize: 12.5, borderBottom: '1px solid var(--line)' }}>No global tags yet.</div>}
      {globals.map((t, i) => row(t, 'global', i, globals))}
      {head('This deck only', 'Tags that stay in this deck.')}
      {!locals.length && <div style={{ padding: '14px 20px', color: 'var(--mu2)', fontSize: 12.5, borderBottom: '1px solid var(--line)' }}>No deck-only tags.</div>}
      {locals.map((t, i) => row(t, 'deck', i, locals))}
      <div className="row" style={{ gap: 8, padding: '12px 20px', flexWrap: 'wrap' }}>
        <input className="inp grow" aria-label="New tag" placeholder="New tag, e.g. mana rock" value={newTag} onChange={(e) => setNewTag(e.target.value)} onKeyDown={(e) => e.key === 'Enter' && add()} style={{ height: 30 }} />
        <label className="row" style={{ gap: 6, fontSize: 12, color: 'var(--mu)' }}>
          <input type="checkbox" checked={newDeckOnly} onChange={() => setNewDeckOnly(!newDeckOnly)} /> This deck only
        </label>
        <button className="tb" onClick={add} disabled={!normalizeTag(newTag)}>
          Add tag
        </button>
      </div>
    </Modal>
  );
}
