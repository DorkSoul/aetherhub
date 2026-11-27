import { useQuery } from '@tanstack/react-query';
import { useEffect, useMemo, useState } from 'react';
import type { CardDef, CardInst, CardView, ZoneName } from '@aetherhub/shared';
import { DUNGEON_NAMES, DUNGEONS, FRAME_COLORS, MULLIGAN_RULES, ZONE_LABEL } from '@aetherhub/shared';
import { api } from '../../lib/api';
import { socket } from '../../lib/ws';
import { RulingsList } from '../../ui/rulings';
import { CardImage, Modal, previewProps, TurnableCard, useDebounced } from '../../ui/ui';
import { cardLabel, locateView } from './menus';
import { mySeat, useTable, type CardsNotice } from './store';

const send = (i: Parameters<ReturnType<typeof useTable.getState>['send']>[0]) => useTable.getState().send(i);
const close = () => useTable.setState({ modal: null });

function CardTile({ c, def, w = 130, children }: { c: CardInst | CardView; def?: CardDef; w?: number; children?: React.ReactNode }) {
  const ctrs = Object.entries(c.counters || {}).filter(([, v]) => v);
  return (
    <div className="col" style={{ gap: 6, alignItems: 'center' }}>
      <div style={{ position: 'relative' }} {...previewProps(def, undefined, 0, 'z:' + c.iid)}>
        {def ? <TurnableCard def={def} turnKey={'z:' + c.iid} style={{ width: w, height: Math.round((w * 88) / 63), borderRadius: 7 }} /> : <CardImage back style={{ width: w, height: Math.round((w * 88) / 63), borderRadius: 7 }} />}
        {ctrs.length > 0 && <div style={{ position: 'absolute', left: 4, bottom: 4, fontSize: 10, fontWeight: 600, background: '#2f7a45', color: '#fff', borderRadius: 7, padding: '1px 6px' }}>{ctrs.map(([k, v]) => `${k} ×${v}`).join(' ')}</div>}
        {c.faceDown && <div style={{ position: 'absolute', right: 4, top: 4, fontSize: 10, background: '#15130f', color: '#ebe8e2', borderRadius: 4, padding: '1px 5px' }}>face down</div>}
      </div>
      {children}
    </div>
  );
}

// ------------------------------------------------------------------ zone viewer (graveyard, exile, command, sideboard…)

export function ZoneModal({ seat, zone }: { seat: number; zone: ZoneName }) {
  const v = useTable((s) => s.view)!;
  const me = mySeat(v);
  const [q, setQ] = useState('');
  const p = v.seats[seat];
  const rules = useTable((s) => s.rules);
  // a library only shows the card that is revealed (the rest are face down placeholders)
  const shownLib = zone === 'library' && p.zones.library.length > 0 && p.zones.library.every((iid) => v.cards[iid] && !v.cards[iid].hidden);
  // a library is listed top first; other zones newest first
  const list = (zone === 'library' ? [...p.zones[zone]] : [...p.zones[zone]].reverse()).filter((iid) => zone !== 'library' || (v.cards[iid] && !v.cards[iid].hidden));
  const rows = list.filter((iid) => {
    const c = v.cards[iid];
    const d = c && v.defs[c.def];
    return !q || (d && `${d.name} ${d.typeLine}`.toLowerCase().includes(q.toLowerCase()));
  });
  return (
    <Modal
      title={zone === 'library' ? `${seat === me ? 'Your' : p.name + "'s"} library · ${shownLib ? `${list.length} cards, top first` : 'top card'}` : `${seat === me ? 'Your' : p.name + "'s"} ${ZONE_LABEL[zone].replace(/^the /, '')} · ${list.length}`}
      onClose={close}
      width={1000}
      head={<input className="inp" placeholder="Filter by name or type…" value={q} onChange={(e) => setQ(e.target.value)} style={{ width: 240 }} />}
    >
      <div style={{ padding: '18px 20px', display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(140px, 1fr))', gap: '16px 14px' }}>
        {rows.map((iid) => {
          const c = v.cards[iid];
          const def = c && !c.hidden ? v.defs[c.def] : undefined;
          const acts: [string, () => void][] = [];
          const mv = (to: ZoneName, s: number, extra = {}) => () => send({ type: 'move', iids: [iid], seat: s, zone: to, ...extra });
          if (rules) {
            // a rules game: only what the rules allow (play or cast it, or use one of its abilities); the sandbox moves don't apply
            // every legal way to use it is listed by name; only when there is none does a plain "Play / cast" ask the engine why not
            const ways = rules.abilities(iid);
            for (const a of ways) acts.push([a.label, () => (a.run(), close())]);
            if (!ways.length) acts.push(['Play / cast', () => (mv('battlefield', me ?? c.owner, { x: 0.5, y: 0.4 })(), close())]);
          } else {
          if (zone !== 'hand') acts.push(['Hand', mv('hand', me ?? c.owner)]);
          acts.push(['Battlefield', mv('battlefield', me ?? c.owner, { x: 0.5, y: 0.4 })]);
          if (zone !== 'library') acts.push(['Library', mv('library', c.owner, { pos: 'top' })]);
          if (zone !== 'graveyard') acts.push(['Graveyard', mv('graveyard', c.owner)]);
          if (zone !== 'exile') acts.push(['Exile', mv('exile', c.owner)]);
          if (zone !== 'command' && c.commander) acts.push(['Command', mv('command', c.owner)]);
          if (zone === 'schemes') acts.push(['Abandon', mv('schemeDeck', c.owner, { pos: 'bottom' })]);
          }
          return (
            <CardTile key={iid} c={c} def={def}>
              <div className="row" style={{ flexWrap: 'wrap', gap: 4, justifyContent: 'center', width: 140 }}>
                {acts.map(([l, run]) => (
                  <button key={l} className="mbtn" onClick={run} style={{ whiteSpace: 'normal', height: 'auto', minHeight: 26, maxWidth: 140, lineHeight: 1.25, padding: '4px 8px', textAlign: 'center' }}>
                    {l}
                  </button>
                ))}
              </div>
              {c.faceDown && me != null && c.peekers.includes(me) && v.seats.some((_, si) => !c.peekers.includes(si)) && (
                <div className="row" style={{ gap: 4, flexWrap: 'wrap', justifyContent: 'center', width: 140 }}>
                  <span style={{ fontSize: 10.5, color: 'var(--mu2)' }}>Let peek:</span>
                  {v.seats.map((s2, si) =>
                    c.peekers.includes(si) ? null : (
                      <button key={si} className="mbtn" onClick={() => send({ type: 'grantPeek', iid, seat: si })}>
                        {s2.name}
                      </button>
                    ),
                  )}
                </div>
              )}
              {!rules && (zone === 'exile' || zone === 'graveyard') && (
                <div className="row" style={{ gap: 4 }}>
                  <button className="mbtn" title="Add a time counter" onClick={() => send({ type: 'counter', iids: [iid], counter: 'time', delta: 1 })}>
                    +time
                  </button>
                  <button className="mbtn" onClick={() => send({ type: 'counter', iids: [iid], counter: 'time', delta: -1 })}>
                    −time
                  </button>
                </div>
              )}
            </CardTile>
          );
        })}
        {!rows.length && <div style={{ gridColumn: '1 / -1', padding: 40, textAlign: 'center', color: 'var(--mu2)', fontSize: 13 }}>{zone === 'library' ? 'No card of this library is revealed.' : 'Nothing here.'}</div>}
      </div>
    </Modal>
  );
}

// ------------------------------------------------------------------ look / scry / search / reveal

type Dest = 'top' | 'bottom' | 'hand' | 'graveyard' | 'exile' | 'battlefield';

export function CardsModal({ data }: { data: CardsNotice }) {
  const v = useTable((s) => s.view)!;
  const n = data.notice;
  const [order, setOrder] = useState(n.iids.map((iid) => ({ iid, dest: 'top' as Dest })));
  const [q, setQ] = useState('');
  const me = mySeat(v);
  if (n.mode === 'search') {
    const still = n.iids.filter((iid) => v.seats[n.seat]?.zones[n.zone].includes(iid));
    const rows = still.filter((iid) => {
      const d = data.defs[data.cards[iid]?.def];
      return !q || (d && `${d.name} ${d.typeLine}`.toLowerCase().includes(q.toLowerCase()));
    });
    return (
      <Modal
        title={`${n.title} (searching) · ${still.length}`}
        onClose={close}
        width={1000}
        head={
          <>
            <input className="inp" placeholder="Filter by name or type…" value={q} onChange={(e) => setQ(e.target.value)} style={{ width: 240 }} autoFocus />
            {n.zone === 'library' && (
              <button
                className="tb sm"
                onClick={() => {
                  send({ type: 'shuffle', seat: n.seat });
                  close();
                }}
              >
                Close &amp; shuffle
              </button>
            )}
          </>
        }
      >
        <div style={{ padding: '18px 20px', display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(140px, 1fr))', gap: '16px 14px' }}>
          {rows.map((iid) => {
            const c = data.cards[iid];
            const def = data.defs[c.def];
            const mv = (to: ZoneName, s: number, extra = {}) => () => send({ type: 'move', iids: [iid], seat: s, zone: to, ...extra });
            return (
              <CardTile key={iid} c={c} def={def}>
                <div className="row" style={{ flexWrap: 'wrap', gap: 4, justifyContent: 'center', width: 140 }}>
                  <button className="mbtn" onClick={mv('hand', me ?? c.owner)}>
                    Hand
                  </button>
                  <button className="mbtn" onClick={mv('battlefield', n.seat, { x: 0.5, y: 0.4 })}>
                    Battlefield
                  </button>
                  <button className="mbtn" onClick={mv('graveyard', c.owner)}>
                    Graveyard
                  </button>
                  <button className="mbtn" onClick={mv('exile', c.owner)}>
                    Exile
                  </button>
                  <button className="mbtn" onClick={mv('library', c.owner, { pos: 'top' })}>
                    Top
                  </button>
                  <button className="mbtn" onClick={mv('library', c.owner, { pos: 'bottom' })}>
                    Bottom
                  </button>
                </div>
              </CardTile>
            );
          })}
          {!rows.length && <div style={{ gridColumn: '1 / -1', padding: 40, textAlign: 'center', color: 'var(--mu2)' }}>Nothing here.</div>}
        </div>
      </Modal>
    );
  }
  const dests: [Dest, string][] =
    n.mode === 'scry'
      ? [
          ['top', 'Top'],
          ['bottom', 'Bottom'],
        ]
      : [
          ['top', 'Top'],
          ['bottom', 'Bottom'],
          ['hand', 'Hand'],
          ['graveyard', 'Graveyard'],
          ['exile', 'Exile'],
          ['battlefield', 'Battlefield'],
        ];
  const moveIdx = (i: number, d: number) => {
    const j = i + d;
    if (j < 0 || j >= order.length) return;
    const o = order.slice();
    [o[i], o[j]] = [o[j], o[i]];
    setOrder(o);
  };
  return (
    <Modal
      title={n.title}
      onClose={close}
      width={1060}
      foot={
        <>
          <button className="tb" onClick={close}>
            Cancel
          </button>
          <button
            className="tb pri"
            onClick={() => {
              send({ type: 'lookResolve', seat: n.seat, mode: n.mode === 'reveal' ? 'reveal' : n.mode === 'scry' ? 'scry' : 'look', items: order });
              close();
            }}
          >
            Done
          </button>
        </>
      }
    >
      <div style={{ padding: '10px 20px 0', fontSize: 12.5, color: 'var(--mu)' }}>Order left → right is top → down. Cards sent to the bottom keep this order.</div>
      <div className="row" style={{ padding: '22px 20px', gap: 18, justifyContent: 'center', flexWrap: 'wrap', alignItems: 'flex-start' }}>
        {order.map((x, i) => {
          const c = data.cards[x.iid];
          const def = c ? data.defs[c.def] : undefined;
          return (
            <div key={x.iid} className="col" style={{ alignItems: 'center', gap: 8, width: 170 }}>
              <div {...previewProps(def, undefined, 0, 'z:' + x.iid)}>
                <TurnableCard def={def} turnKey={'z:' + x.iid} style={{ width: 150, height: 210, borderRadius: 8 }} />
              </div>
              <div className="row" style={{ gap: 4 }}>
                <button className="sq sm" onClick={() => moveIdx(i, -1)} aria-label="Move earlier">
                  ◂
                </button>
                <span style={{ fontSize: 11, color: 'var(--mu2)', width: 50, textAlign: 'center' }}>#{i + 1}</span>
                <button className="sq sm" onClick={() => moveIdx(i, 1)} aria-label="Move later">
                  ▸
                </button>
              </div>
              <div className="row" style={{ flexWrap: 'wrap', gap: 4, justifyContent: 'center' }}>
                {dests.map(([d, l]) => (
                  <button key={d} className={'mbtn' + (x.dest === d ? ' on' : '')} onClick={() => setOrder(order.map((o, j) => (j === i ? { ...o, dest: d } : o)))}>
                    {l}
                  </button>
                ))}
              </div>
            </div>
          );
        })}
      </div>
    </Modal>
  );
}

/** Non-blocking popup when someone reveals cards. */
export function RevealPopup({ data }: { data: CardsNotice }) {
  useEffect(() => {
    const t = setTimeout(() => useTable.setState({ reveal: null }), 6000);
    return () => clearTimeout(t);
  }, [data]);
  return (
    <div style={{ position: 'fixed', right: 16, bottom: 16, zIndex: 900, background: 'var(--surface3)', border: '1px solid var(--line4)', borderRadius: 12, boxShadow: 'var(--shadow)', padding: 12, maxWidth: 560 }}>
      <div className="row" style={{ marginBottom: 8 }}>
        <span className="grow" style={{ fontSize: 13, fontWeight: 600 }}>
          {data.notice.title}
        </span>
        <button className="mbtn" onClick={() => useTable.setState({ reveal: null })}>
          ✕
        </button>
      </div>
      <div className="row" style={{ gap: 8, flexWrap: 'wrap' }}>
        {data.notice.iids.map((iid) => {
          const d = data.defs[data.cards[iid]?.def];
          return (
            <div key={iid} {...previewProps(d)}>
              <CardImage def={d} style={{ width: 100, height: 140, borderRadius: 6 }} />
            </div>
          );
        })}
      </div>
    </div>
  );
}

// ------------------------------------------------------------------ tokens

export function TokensModal({ seat }: { seat: number }) {
  const v = useTable((s) => s.view)!;
  const [q, setQ] = useState('');
  const dq = useDebounced(q, 250);
  const res = useQuery({ queryKey: ['tokens', dq], queryFn: () => api.get<{ cards: CardDef[] }>(`api/cards/search?tokens=1&limit=48&q=${encodeURIComponent(dq || 't:token')}`), enabled: useTable.getState().mode === 'online' || true });
  const [cu, setCu] = useState({ name: 'Spirit', pt: '1/1', colors: ['W'] as string[] });
  const [count, setCount] = useState(1);
  const deckTokens = v.seats[seat].tokenDefs.map((k) => v.defs[k]).filter(Boolean);
  const make = (def: string) => {
    send({ type: 'createToken', seat, def, count });
    close();
  };
  const unique = useMemo(() => {
    const seen = new Set<string>();
    return (res.data?.cards || []).filter((c) => {
      const k = c.name + c.typeLine + c.power + c.toughness + c.oracleText;
      if (seen.has(k)) return false;
      seen.add(k);
      return true;
    });
  }, [res.data]);
  return (
    <Modal
      title={`Create token${seat !== mySeat(v) ? ` for ${v.seats[seat].name}` : ''}`}
      onClose={close}
      width={960}
      head={
        <>
          <span style={{ fontSize: 12, color: 'var(--mu)' }}>Count</span>
          <button className="sq sm" onClick={() => setCount(Math.max(1, count - 1))}>
            −
          </button>
          <span className="mono">{count}</span>
          <button className="sq sm" onClick={() => setCount(Math.min(30, count + 1))}>
            +
          </button>
          <input className="inp" aria-label="Search tokens" placeholder="Search tokens and emblems…" value={q} onChange={(e) => setQ(e.target.value)} style={{ width: 240 }} autoFocus />
        </>
      }
    >
      {deckTokens.length > 0 && (
        <div className="row" style={{ padding: '12px 20px', background: 'var(--surface2)', borderBottom: '1px solid var(--line)', flexWrap: 'wrap' }}>
          <span className="grow" style={{ fontSize: 12.5, color: 'var(--tx3)' }}>
            From <b>{v.seats[seat].deckName}</b> · Tokens/Extras section: {deckTokens.map((d) => d.name).join(', ')}
          </span>
          {deckTokens.map((d) => (
            <button key={d.key} className="mbtn" onClick={() => make(d.key)}>
              {d.name}
            </button>
          ))}
          <button
            className="tb sm"
            onClick={() => {
              deckTokens.forEach((d) => send({ type: 'createToken', seat, def: d.key, count: 1 }));
              close();
            }}
          >
            Add all {deckTokens.length}
          </button>
        </div>
      )}
      <div style={{ padding: '18px 20px', display: 'flex', gap: 14, flexWrap: 'wrap' }}>
        {unique.map((d) => (
          <button key={d.key} onClick={() => make(d.key)} title="Create on the battlefield" className="col" style={{ border: '1px solid var(--line)', background: 'var(--surface2)', borderRadius: 10, padding: 8, cursor: 'pointer', alignItems: 'center', gap: 6 }} {...previewProps(d)}>
            <CardImage def={d} style={{ width: 110, height: 154, borderRadius: 6 }} />
            <span style={{ fontSize: 12, color: 'var(--tx)', maxWidth: 110, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
              {d.name}
              {d.power != null ? ` ${d.power}/${d.toughness}` : ''}
            </span>
          </button>
        ))}
        {res.isFetching && <div style={{ color: 'var(--mu)' }}>Searching…</div>}
      </div>
      <div className="row" style={{ padding: '14px 20px', borderTop: '1px solid var(--line)', flexWrap: 'wrap' }}>
        <span style={{ fontSize: 12.5, fontWeight: 600 }}>Custom token</span>
        <input className="inp" aria-label="Token name" value={cu.name} onChange={(e) => setCu({ ...cu, name: e.target.value })} style={{ width: 170, height: 30 }} />
        <input className="inp" aria-label="Power/toughness" value={cu.pt} onChange={(e) => setCu({ ...cu, pt: e.target.value })} style={{ width: 64, height: 30, textAlign: 'center' }} />
        <div className="row" style={{ gap: 4 }}>
          {['W', 'U', 'B', 'R', 'G', 'C'].map((c) => {
            const on = c === 'C' ? !cu.colors.length : cu.colors.includes(c);
            return (
              <button
                key={c}
                aria-label={`Color ${c}`}
                aria-pressed={on}
                onClick={() => setCu({ ...cu, colors: c === 'C' ? [] : on ? cu.colors.filter((x) => x !== c) : [...cu.colors, c] })}
                style={{ width: 24, height: 24, borderRadius: 12, background: FRAME_COLORS[c].bg, border: `2px solid ${on ? 'var(--acc)' : '#15130f'}`, cursor: 'pointer' }}
              />
            );
          })}
        </div>
        <div className="grow" />
        <button
          className="tb pri"
          onClick={() => {
            send({ type: 'createToken', seat, count, custom: { name: cu.name || 'Token', pt: cu.pt, colors: cu.colors as ('W' | 'U' | 'B' | 'R' | 'G')[] } });
            close();
          }}
        >
          Create
        </button>
      </div>
    </Modal>
  );
}

// ------------------------------------------------------------------ small dialogs

export function CommanderPrompt({ iid }: { iid: string }) {
  const v = useTable((s) => s.view)!;
  const c = v.cards[iid];
  const loc = locateView(v, iid);
  if (!c || !loc || loc.zone === 'command') return null;
  return (
    <Modal
      title="Move to command zone?"
      onClose={close}
      width={440}
      foot={
        <>
          <button className="tb" onClick={close}>
            Leave it there
          </button>
          <button
            className="tb pri"
            autoFocus
            onClick={() => {
              send({ type: 'move', iids: [iid], seat: c.owner, zone: 'command' });
              close();
            }}
          >
            Move to command zone
          </button>
        </>
      }
    >
      <div style={{ padding: '16px 20px', fontSize: 13.5, color: 'var(--tx3)', lineHeight: 1.5 }}>
        {cardLabel(v, c)} went to your {ZONE_LABEL[loc.zone]}. Commanders can go to the command zone instead.
      </div>
    </Modal>
  );
}

export function EndGameModal() {
  const v = useTable((s) => s.view)!;
  const [winner, setWinner] = useState<number | null>(null);
  return (
    <Modal
      title="End the game"
      onClose={close}
      width={460}
      foot={
        <>
          <button className="tb" onClick={close}>
            Cancel
          </button>
          <button
            className="tb pri"
            onClick={() => {
              socket.send({ t: 'host', op: 'end', winner });
              close();
            }}
          >
            End game
          </button>
        </>
      }
    >
      <div className="col" style={{ padding: '16px 20px', gap: 8 }}>
        <div style={{ fontSize: 13, color: 'var(--mu)' }}>Optionally mark a winner. The match goes to history{v.settings.bo === 3 ? '; in Best of 3 the next game starts after sideboarding' : ''}.</div>
        {[null, ...v.seats.map((_, i) => i)].map((i) => (
          <button key={String(i)} onClick={() => setWinner(i)} className="row" style={{ padding: '10px 12px', borderRadius: 9, border: `1px solid ${winner === i ? 'var(--acc)' : 'var(--line5)'}`, background: winner === i ? 'var(--acc-bg)' : 'var(--surface2)', color: 'var(--tx)', cursor: 'pointer' }}>
            {i == null ? (
              'No winner'
            ) : (
              <>
                <span className="dot" style={{ background: v.seats[i].color }} />
                {v.seats[i].name}
              </>
            )}
          </button>
        ))}
      </div>
    </Modal>
  );
}

export function GiveSeatModal({ seat }: { seat: number }) {
  const v = useTable((s) => s.view)!;
  const users = useQuery({ queryKey: ['users'], queryFn: () => api.get<{ id: number; username: string }[]>('api/users') });
  const seated = new Set(v.seats.map((s) => s.userId));
  return (
    <Modal title={`Give ${v.seats[seat].name}'s seat to…`} onClose={close} width={420}>
      <div className="col" style={{ padding: 16, gap: 6 }}>
        {(users.data || [])
          .filter((u) => !seated.has(u.id))
          .map((u) => (
            <button
              key={u.id}
              className="tb"
              onClick={() => {
                socket.send({ t: 'host', op: 'giveSeat', seat, userId: u.id });
                close();
              }}
            >
              {u.username}
            </button>
          ))}
      </div>
    </Modal>
  );
}

export function RulingsModal({ iid }: { iid: string }) {
  const v = useTable((s) => s.view);
  const c = v?.cards[iid];
  const def = c && v ? v.defs[c.def] : undefined;
  if (!def) return null;
  return (
    <Modal title={`Rulings · ${def.name}`} onClose={close} width={620}>
      <div className="row" style={{ padding: '16px 20px', gap: 18, alignItems: 'flex-start' }}>
        <TurnableCard def={def} turnKey={'r:' + iid} style={{ width: 180, height: 252, borderRadius: 9 }} />
        <div className="grow" style={{ minWidth: 0 }}>
          <RulingsList oracleId={def.oracleId} maxHeight={460} link={false} />
        </div>
      </div>
    </Modal>
  );
}

export function AttachModal({ iid, mode }: { iid: string; mode: 'attach' | 'meld' }) {
  const v = useTable((s) => s.view)!;
  const loc = locateView(v, iid);
  const all = v.seats.flatMap((s) => s.zones.battlefield).filter((i) => i !== iid);
  return (
    <Modal title={mode === 'attach' ? `Attach ${cardLabel(v, v.cards[iid])} to…` : `Meld ${cardLabel(v, v.cards[iid])} with…`} onClose={close} width={900}>
      <div style={{ padding: 18, display: 'flex', flexWrap: 'wrap', gap: 10 }}>
        {all
          .filter((i) => mode === 'attach' || locateView(v, i)?.seat === loc?.seat)
          .map((i) => {
            const c = v.cards[i];
            const d = c && !c.hidden ? v.defs[c.def] : undefined;
            return (
              <button
                key={i}
                style={{ border: 0, padding: 0, background: 'none', cursor: 'pointer' }}
                title={cardLabel(v, c)}
                onClick={() => {
                  send(mode === 'attach' ? { type: 'attach', iid, to: i } : { type: 'meld', iid, with: i });
                  close();
                }}
              >
                <CardImage def={d} back={!d} style={{ width: 90, height: 126, borderRadius: 6 }} />
              </button>
            );
          })}
      </div>
    </Modal>
  );
}

export function NumberModal({ title, onDone }: { title: string; onDone: (n: number) => void }) {
  const [n, setN] = useState('3');
  return (
    <Modal
      title={title}
      onClose={close}
      width={360}
      foot={
        <button
          className="tb pri"
          onClick={() => {
            const x = parseInt(n, 10);
            if (x > 0) onDone(x);
            close();
          }}
        >
          OK
        </button>
      }
    >
      <div style={{ padding: 20 }}>
        <input className="inp" type="number" min={1} autoFocus value={n} onChange={(e) => setN(e.target.value)} onKeyDown={(e) => e.key === 'Enter' && (onDone(parseInt(n, 10) || 1), close())} style={{ width: '100%' }} />
      </div>
    </Modal>
  );
}

// ------------------------------------------------------------------ specials (planechase, archenemy, dungeons, attractions)

export function SpecialsModal() {
  const v = useTable((s) => s.view)!;
  const me = mySeat(v);
  const planeDefs = v.table.planes.map((iid) => (v.cards[iid] ? v.defs[v.cards[iid].def] : undefined)).filter((d): d is NonNullable<typeof d> => !!d);
  const rollCost = v.table.planarRolls ?? 0;
  const encounter = v.table.encounter ? v.defs[v.cards[v.table.encounter]?.def] : undefined;
  return (
    <Modal title="Special game objects" onClose={close} width={1000}>
      <div className="col" style={{ padding: 20, gap: 22 }}>
        {(v.settings.planechase || v.table.planes.length > 0) && (
          <section className="col" style={{ gap: 10 }}>
            <h3 style={{ fontSize: 17 }}>Planechase</h3>
            <div className="row" style={{ gap: 18, alignItems: 'flex-start', flexWrap: 'wrap' }}>
              {planeDefs.length ? (
                planeDefs.map((d) => (
                  <div key={d.key} {...previewProps(d)} style={{ width: 308, height: 220, position: 'relative' }}>
                    <CardImage def={d} style={{ width: 220, height: 308, position: 'absolute', left: 44, top: -44, transform: 'rotate(90deg)', borderRadius: 10 }} />
                  </div>
                ))
              ) : (
                <div style={{ color: 'var(--mu)' }}>No plane in play. Add Planar cards to your deck’s Planar section.</div>
              )}
              <div className="col" style={{ gap: 8 }}>
                {planeDefs.map((d) => (
                  <div key={d.key} className="col" style={{ gap: 4 }}>
                    <div style={{ fontSize: 13 }}>{d.name}</div>
                    <div style={{ fontSize: 12.5, color: 'var(--mu)', maxWidth: 420, whiteSpace: 'pre-line' }}>{d.oracleText}</div>
                  </div>
                ))}
                {encounter && (
                  <div style={{ fontSize: 12.5, padding: '8px 10px', borderRadius: 8, background: 'var(--acc-bg)', border: '1px solid var(--acc-bd)', maxWidth: 420 }}>
                    Phenomenon: resolve {encounter.name}, then planeswalk on.
                  </div>
                )}
                <div className="row" style={{ flexWrap: 'wrap' }}>
                  <button className="tb pri" onClick={() => send({ type: 'planarRoll' })} title="The first roll each turn is free; each roll after it costs one more generic mana">
                    Roll planar die{rollCost ? ` (pay {${rollCost}})` : ' (free)'}
                  </button>
                  <button className="tb" onClick={() => send({ type: 'planeswalk' })}>
                    {encounter ? 'Done — planeswalk on' : 'Planeswalk'}
                  </button>
                  <button className="tb" onClick={() => send({ type: 'planeswalk', extra: true })} title="Turn over one more plane and keep the current ones (Spatial Merging, Interplanar Tunnel)">
                    Add another plane
                  </button>
                  <span style={{ fontSize: 12, color: 'var(--mu2)' }}>{v.table.planarDeck.length} in the planar deck</span>
                </div>
              </div>
            </div>
          </section>
        )}
        {v.seats.some((s) => s.archenemy) && (
          <section className="col" style={{ gap: 10 }}>
            <h3 style={{ fontSize: 17 }}>Archenemy</h3>
            {v.seats.map((s, i) =>
              s.archenemy ? (
                <div key={i} className="col" style={{ gap: 8 }}>
                  <div className="row">
                    <span className="dot" style={{ background: s.color }} />
                    <b>{s.name}</b>
                    <span style={{ fontSize: 12, color: 'var(--mu)' }}>
                      {s.zones.schemeDeck.length} schemes left · {s.zones.schemes.length} in play
                    </span>
                    <button className="tb sm" onClick={() => send({ type: 'setInMotion', seat: i })} title="Also done for you at the start of the archenemy's first main phase; one-shot schemes go back to the bottom">
                      Set a scheme in motion
                    </button>
                    <span style={{ fontSize: 11.5, color: 'var(--mu2)' }}>Archenemy · {s.life} life · everyone else is one team</span>
                  </div>
                  <div className="row" style={{ gap: 10, flexWrap: 'wrap' }}>
                    {s.zones.schemes.map((iid) => {
                      const d = v.defs[v.cards[iid]?.def];
                      return (
                        <div key={iid} className="col" style={{ gap: 4, alignItems: 'center' }}>
                          <div {...previewProps(d)}>
                            <CardImage def={d} style={{ width: 110, height: 154, borderRadius: 6 }} />
                          </div>
                          <button className="mbtn" onClick={() => send({ type: 'move', iids: [iid], seat: i, zone: 'schemeDeck', pos: 'bottom' })}>
                            Abandon
                          </button>
                        </div>
                      );
                    })}
                  </div>
                </div>
              ) : null,
            )}
          </section>
        )}
        {(v.settings.dungeons || v.seats.some((s) => s.dungeon)) && (
          <section className="col" style={{ gap: 10 }}>
            <h3 style={{ fontSize: 17 }}>Dungeons</h3>
            {v.seats.map((s, i) => {
              const rooms = s.dungeon ? DUNGEONS[s.dungeon.name] : null;
              return (
                <div key={i} className="row" style={{ flexWrap: 'wrap', alignItems: 'flex-start' }}>
                  <span className="row" style={{ width: 120 }}>
                    <span className="dot" style={{ background: s.color }} />
                    {s.name}
                  </span>
                  {rooms && s.dungeon ? (
                    <div className="row" style={{ gap: 4, flexWrap: 'wrap' }}>
                      <b style={{ fontSize: 12.5 }}>{s.dungeon.name}:</b>
                      {rooms.map((r, ri) => (
                        <button key={ri} className={'mbtn' + (s.dungeon!.room === ri ? ' on' : '')} onClick={() => send({ type: 'dungeon', seat: i, name: s.dungeon!.name, room: ri })} title={r.next.length ? `Leads to ${r.next.map((x) => rooms[x].name).join(' or ')}` : 'Last room'}>
                          {r.name}
                        </button>
                      ))}
                      <button className="mbtn" onClick={() => send({ type: 'dungeon', seat: i, name: null, room: 0 })}>
                        Complete
                      </button>
                    </div>
                  ) : (
                    <div className="row" style={{ gap: 4, flexWrap: 'wrap' }}>
                      {DUNGEON_NAMES.map((n) => (
                        <button key={n} className="mbtn" onClick={() => send({ type: 'dungeon', seat: i, name: n, room: 0 })}>
                          Venture into {n}
                        </button>
                      ))}
                    </div>
                  )}
                </div>
              );
            })}
          </section>
        )}
        {v.settings.attractions && (
          <section className="col" style={{ gap: 10 }}>
            <h3 style={{ fontSize: 17 }}>Attractions &amp; stickers</h3>
            {v.seats.map((s, i) => (
              <div key={i} className="col" style={{ gap: 6 }}>
                <div className="row">
                  <span className="dot" style={{ background: s.color }} />
                  <b>{s.name}</b>
                  <span style={{ fontSize: 12, color: 'var(--mu)' }}>{s.zones.attractionDeck.length} attractions in deck</span>
                  {i === me && (
                    <>
                      <button className="tb sm" onClick={() => send({ type: 'openAttraction', seat: i })}>
                        Open an Attraction
                      </button>
                      <button className="tb sm" onClick={() => send({ type: 'rollVisit', seat: i })}>
                        Roll to visit
                      </button>
                    </>
                  )}
                </div>
                {s.zones.stickers.length > 0 && (
                  <div className="row" style={{ gap: 8, flexWrap: 'wrap' }}>
                    {s.zones.stickers.map((iid) => {
                      const d = v.defs[v.cards[iid]?.def];
                      return (
                        <div key={iid} {...previewProps(d)}>
                          <CardImage def={d} style={{ width: 90, height: 126, borderRadius: 6 }} />
                        </div>
                      );
                    })}
                  </div>
                )}
              </div>
            ))}
          </section>
        )}
      </div>
    </Modal>
  );
}

// ------------------------------------------------------------------ prompts & stage bars

export function Prompts() {
  const v = useTable((s) => s.view);
  const undo = useTable((s) => s.undoRequests);
  const me = v ? mySeat(v) : null;
  if (!v) return null;
  const reqs = v.requests.filter((r) => r.to === me);
  if (!reqs.length && !undo.length) return null;
  return (
    <div className="col" style={{ position: 'fixed', left: '50%', top: 60, transform: 'translateX(-50%)', gap: 8, zIndex: 930 }}>
      {reqs.map((r) => (
        <div key={r.id} className="row" style={{ padding: '10px 14px', borderRadius: 10, background: 'var(--surface3)', border: '1px solid var(--acc)', boxShadow: 'var(--shadow)', fontSize: 13 }}>
          <span className="dot" style={{ background: v.seats[r.from]?.color }} />
          <b>{v.seats[r.from]?.name}</b> asks to see your hand
          <button className="tb sm pri" onClick={() => send({ type: 'answerHandView', id: r.id, grant: true })}>
            Allow (live)
          </button>
          <button className="tb sm" onClick={() => send({ type: 'answerHandView', id: r.id, grant: false })}>
            Decline
          </button>
        </div>
      ))}
      {undo.map((u) => (
        <div key={u.id} className="row" style={{ padding: '10px 14px', borderRadius: 10, background: 'var(--surface3)', border: '1px solid var(--acc)', boxShadow: 'var(--shadow)', fontSize: 13 }}>
          <b>{u.fromName}</b> wants to undo: {u.label}
          <button className="tb sm pri" onClick={() => (socket.send({ t: 'undoAnswer', id: u.id, approve: true }), useTable.setState({ undoRequests: useTable.getState().undoRequests.filter((x) => x.id !== u.id) }))}>
            Approve rewind
          </button>
          <button className="tb sm" onClick={() => (socket.send({ t: 'undoAnswer', id: u.id, approve: false }), useTable.setState({ undoRequests: useTable.getState().undoRequests.filter((x) => x.id !== u.id) }))}>
            Decline
          </button>
        </div>
      ))}
    </div>
  );
}

export function MulliganBar() {
  const v = useTable((s) => s.view)!;
  const bottom = useTable((s) => s.mullBottom);
  const solo = useTable((s) => s.solo);
  const me = mySeat(v);
  if (v.stage !== 'mulligan' || me == null) return null;
  const s = v.seats[me];
  const m = s.mulligan;
  if (!m || m.kept || s.conceded) {
    const waiting = v.seats.filter((x) => x.mulligan && !x.mulligan.kept && !x.conceded).map((x) => x.name);
    if (!waiting.length) return null;
    // Solo: you are every player (or hold several seats online), so offer to switch to the next one instead of waiting.
    const mine = useTable.getState().mySeats;
    const next = v.seats.findIndex((x, i) => x.mulligan && !x.mulligan.kept && !x.conceded && (solo || mine.includes(i)));
    return (
      <div className="row" style={{ position: 'fixed', left: '50%', bottom: 18, transform: 'translateX(-50%)', zIndex: 920, padding: '10px 16px', borderRadius: 10, background: 'var(--surface3)', border: '1px solid var(--line4)', fontSize: 13, gap: 10 }}>
        Waiting for {waiting.join(', ')} to keep…
        {next >= 0 && (
          <button
            className="tb sm pri"
            onClick={() => {
              if (solo) {
                useTable.setState({ solo: { ...solo, seat: next }, sel: [], mullBottom: null });
                useTable.getState().republish?.();
              } else {
                useTable.setState({ sel: [], mullBottom: null });
                socket.send({ t: 'seat', seat: next });
              }
            }}
          >
            Decide for {v.seats[next].name}
          </button>
        )}
      </div>
    );
  }
  const rule = v.settings.mulligan;
  const need = rule === 'london' ? m.count : rule === 'london-free' ? Math.max(0, m.count - 1) : 0;
  const picking = bottom != null;
  return (
    <div className="row" style={{ position: 'fixed', left: '50%', bottom: 18, transform: 'translateX(-50%)', zIndex: 920, padding: '10px 14px', borderRadius: 12, background: 'var(--surface3)', border: '1px solid var(--acc)', boxShadow: 'var(--shadow)', fontSize: 13, flexWrap: 'wrap' }}>
      <b>Mulligan{solo || useTable.getState().mySeats.length > 1 ? ` · ${s.name}` : ''}</b>
      <span style={{ color: 'var(--mu)' }}>
        {m.count ? `${m.count} taken` : 'Opening hand'} · {MULLIGAN_RULES.find((r) => r.id === rule)?.label ?? rule}
      </span>
      {picking ? (
        <>
          <span>
            Click {need} card{need === 1 ? '' : 's'} in {solo ? `${s.name}'s` : 'your'} hand to put on the bottom ({bottom.length}/{need})
          </span>
          <button
            className="tb sm pri"
            disabled={bottom.length !== need}
            onClick={() => {
              send({ type: 'keep', bottom });
              useTable.setState({ mullBottom: null });
            }}
          >
            Keep
          </button>
          <button className="tb sm" onClick={() => useTable.setState({ mullBottom: null })}>
            Back
          </button>
        </>
      ) : (
        <>
          {rule === 'free-scry3' && (m.scryLeft ?? 3) > 0 && (
            <button className="tb sm" title="Scry the top cards of your library now: put any on the bottom, the rest back on top in any order" onClick={() => send({ type: 'look', seat: me, n: m.scryLeft ?? 3, mode: 'scry' })}>
              {m.scried ? `Scry again (${m.scryLeft ?? 3} on top)` : 'Scry 3'}
            </button>
          )}
          <button className="tb sm" onClick={() => send({ type: 'mulligan' })}>
            {rule === 'free-scry3' ? `Mulligan (${s.zones.hand.length + Math.min(m.scryLeft ?? 3, s.zones.library.length)} to the bottom)` : 'Mulligan'}
          </button>
          <button className="tb sm pri" onClick={() => (need ? useTable.setState({ mullBottom: [] }) : send({ type: 'keep', bottom: [] }))}>
            Keep {s.zones.hand.length - need}
          </button>
        </>
      )}
    </div>
  );
}

export function SideboardModal() {
  const sb = useTable((s) => s.sideboard);
  const v = useTable((s) => s.view);
  if (!sb || !v) return null;
  const me = sb.seat;
  const mine = me >= 0;
  const row = (list: { def: string; qty: number }[], to: 'main' | 'side') =>
    list
      .slice()
      .sort((a, b) => (sb.defs[a.def]?.name || '').localeCompare(sb.defs[b.def]?.name || ''))
      .map((c) => (
        <div key={c.def} className="row" style={{ fontSize: 12.5, gap: 6, padding: '2px 0' }} {...previewProps(sb.defs[c.def])}>
          <span className="mono" style={{ width: 22, color: 'var(--mu)' }}>
            {c.qty}
          </span>
          <span className="grow">{sb.defs[c.def]?.name}</span>
          <button className="mbtn" onClick={() => socket.send({ t: 'sideboard', op: 'swap', def: c.def, to })}>
            {to === 'side' ? '→ Side' : '← Main'}
          </button>
        </div>
      ));
  return (
    <Modal title={`Sideboarding · game ${sb.game + 1} of 3`} width={900}>
      <div className="col" style={{ padding: 20, gap: 14 }}>
        <div style={{ fontSize: 13, color: 'var(--mu)' }}>
          Score: {sb.names.map((n, i) => `${n} ${sb.wins[i] || 0}`).join(' · ')}. Swaps only change this match, never your saved deck.
        </div>
        {mine && (
          <div className="row" style={{ alignItems: 'flex-start', gap: 20, flexWrap: 'wrap' }}>
            <div className="col grow card-box lg" style={{ padding: 12, maxHeight: 380, overflowY: 'auto', minWidth: 280 }}>
              <b style={{ marginBottom: 6 }}>Main · {sb.main.reduce((s, c) => s + c.qty, 0)}</b>
              {row(sb.main, 'side')}
            </div>
            <div className="col grow card-box lg" style={{ padding: 12, maxHeight: 380, overflowY: 'auto', minWidth: 280 }}>
              <b style={{ marginBottom: 6 }}>Sideboard · {sb.side.reduce((s, c) => s + c.qty, 0)}</b>
              {row(sb.side, 'main')}
            </div>
          </div>
        )}
        {sb.chooser != null && (me === sb.chooser || useTable.getState().host) && (
          <div className="row" style={{ flexWrap: 'wrap' }}>
            <span style={{ fontSize: 13 }}>{me === sb.chooser ? 'You lost the last game — choose who starts:' : `${sb.names[sb.chooser]} chooses who starts (you can choose as host):`}</span>
            {sb.names.map((n, i) => (
              <button key={i} className="mbtn" onClick={() => socket.send({ t: 'sideboard', op: 'choose', starter: i })}>
                {n}
              </button>
            ))}
          </div>
        )}
        <div className="row">
          <span className="grow" style={{ fontSize: 12.5, color: 'var(--mu)' }}>
            Ready: {sb.names.filter((_, i) => sb.ready[i]).join(', ') || 'nobody yet'}
          </span>
          {mine && (
            <button className={'tb ' + (sb.ready[me] ? '' : 'pri')} onClick={() => socket.send({ t: 'sideboard', op: 'ready', value: !sb.ready[me] })}>
              {sb.ready[me] ? 'Ready ✓' : 'Ready for next game'}
            </button>
          )}
        </div>
      </div>
    </Modal>
  );
}

export function TableModals() {
  const modal = useTable((s) => s.modal);
  const reveal = useTable((s) => s.reveal);
  const v = useTable((s) => s.view);
  if (!v) return null;
  let body: React.ReactNode = null;
  if (modal) {
    switch (modal.kind) {
      case 'zone':
        body = <ZoneModal seat={modal.seat} zone={modal.zone} />;
        break;
      case 'cards':
        body = <CardsModal data={modal.data} />;
        break;
      case 'tokens':
        body = <TokensModal seat={modal.seat} />;
        break;
      case 'commander':
        body = <CommanderPrompt iid={modal.iid} />;
        break;
      case 'end':
        body = <EndGameModal />;
        break;
      case 'giveSeat':
        body = <GiveSeatModal seat={modal.seat} />;
        break;
      case 'specials':
        body = <SpecialsModal />;
        break;
      case 'attach':
        body = <AttachModal iid={modal.iid} mode={modal.mode} />;
        break;
      case 'rulings':
        body = <RulingsModal iid={modal.iid} />;
        break;
      case 'nth':
        body = <NumberModal title="Put it Nth from the top — N =" onDone={(n) => send({ type: 'move', iids: [modal.iid], seat: v.cards[modal.iid]?.owner ?? 0, zone: 'library', pos: n })} />;
        break;
      case 'x':
        body = <NumberModal title={modal.title} onDone={modal.run} />;
        break;
      default:
        body = null;
    }
  }
  return (
    <>
      {body}
      {reveal && <RevealPopup data={reveal} />}
      <SideboardModal />
      <Prompts />
      <MulliganBar />
    </>
  );
}

