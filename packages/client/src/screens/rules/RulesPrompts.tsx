import { useEffect, useRef, useState } from 'react';
import type { CardDef, rules as R } from '@aetherhub/shared';
import { CardImage, previewProps, useMenu } from '../../ui/ui';
import { useTable } from '../table/store';
import { ManaChip, ManaCostText, POOL_COLORS, RichLabel } from './mana';
import type { RCard, RView } from './rulesView';
import { tapChoices } from './rulesView';

type Choice = R.Choice;
type Opt = R.Opt;
type Obj = R.Obj;

/** What an engine option points at, resolved against the current view. */
interface Resolved {
  card?: RCard;
  player?: number;
  stack?: number;
  label: string;
}
export function resolveOpt(o: Opt, v: RView, names: string[]): Resolved {
  if (o.t === 'Text') return { label: o.text };
  if (o.t === 'Color') return { label: ({ W: 'White', U: 'Blue', B: 'Black', R: 'Red', G: 'Green' } as Record<string, string>)[String(o.c)] ?? String(o.c) };
  const ob = o.obj;
  if (ob.kind === 'Player') return { player: ob.id, label: names[ob.id] ?? `Player ${ob.id + 1}` };
  if (ob.kind === 'Stack') {
    const s = v.stack.find((x) => x.id === ob.id);
    return { stack: ob.id, label: s ? `${s.name}${s.is_spell ? '' : ' (ability)'}` : 'a spell or ability' };
  }
  const card = v.cards.find((c) => c.id === ob.id) ?? v.decision_cards.find((c) => c.id === ob.id);
  return { card, label: card?.name ?? 'a card' };
}

const onTable = (c: RCard | undefined, me: number): boolean => !!c && (c.zone === 'Battlefield' || (c.zone === 'Hand' && c.owner === me));

/** `back`: the other face's picture of a double-faced card (the image map's `@back:` entry). */
function defOf(c: RCard, img?: string, back?: string): CardDef {
  return {
    key: 'o:' + c.id, name: c.name, layout: 'normal', manaCost: c.cost, cmc: c.mana_value, typeLine: c.types, oracleText: c.text, colors: [], colorIdentity: [], img: img ? (back ? [img, back] : [img]) : [],
    power: c.base_power !== null ? String(c.base_power) : undefined, toughness: c.base_toughness !== null ? String(c.base_toughness) : undefined,
  };
}

function PickGrid({ cards, sel, images, toggle, max }: { cards: { idx: number; card: RCard }[]; sel: number[]; images: Record<string, string>; toggle: (i: number, max: number) => void; max: number }) {
  const [q, setQ] = useState('');
  const shown = cards.filter((o) => !q || `${o.card.name} ${o.card.types} ${o.card.text}`.toLowerCase().includes(q.toLowerCase()));
  return (
    <div>
      {cards.length > 8 && <input className="inp" placeholder="Filter by name, type or text…" value={q} onChange={(e) => setQ(e.target.value)} style={{ width: '100%', marginBottom: 8 }} />}
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(118px, 1fr))', gap: '12px 10px', maxHeight: '58vh', overflowY: 'auto', padding: 2 }}>
        {shown.map(({ idx, card }) => {
          const def = defOf(card, images[card.name], images['@back:' + card.name]);
          const on = sel.includes(idx);
          return (
            <button data-option
              key={idx}
              type="button"
              onClick={() => toggle(idx, max)}
              {...previewProps(def, undefined, 0, 'o:' + card.id)}
              style={{ all: 'unset', cursor: 'pointer', display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 4, padding: 4, borderRadius: 9, outline: on ? '2px solid var(--acc)' : '2px solid transparent', background: on ? 'rgba(var(--acc-rgb),.14)' : undefined }}
              aria-pressed={on}
            >
              <CardImage def={def} style={{ width: 110, height: Math.round((110 * 88) / 63), borderRadius: 6 }} />
              <span style={{ fontSize: 11.5, textAlign: 'center', lineHeight: 1.2 }}>{card.name}</span>
            </button>
          );
        })}
      </div>
      {!shown.length && <div style={{ color: 'var(--mu)', padding: 8 }}>Nothing matches.</div>}
    </div>
  );
}

/** True when the pick should be shown as a spoiler of cards instead of clicked on the table. */
export function isGridPick(d: R.Decision, v: RView, names: string[]): boolean {
  if (d.kind.t !== 'Pick') return false;
  const rs = d.kind.options.map((o) => resolveOpt(o, v, names));
  return rs.length > 0 && rs.every((r) => r.card) && !rs.every((r) => onTable(r.card, v.you));
}

function DamagePrompt({ d, v, names, onAnswer }: { d: R.Decision; v: RView; names: string[]; onAnswer: (c: Choice) => void }) {
  const k = d.kind as Extract<R.DecisionKind, { t: 'AssignDamage' }>;
  const nameOfCard = (id: number) => v.cards.find((c) => c.id === id)?.name ?? 'creature';
  const defName = k.defender ? (k.defender.kind === 'Player' ? names[k.defender.id] : nameOfCard((k.defender as { id: number }).id)) : '';
  const lethalFirst = () => {
    let left = k.total;
    const m: Record<string, number> = {};
    k.blockers.forEach(([id, lethal], i) => {
      const last = i === k.blockers.length - 1 && !k.defender;
      const n = last ? left : Math.min(left, lethal);
      m[String(id)] = n;
      left -= n;
    });
    if (k.defender) m.d = left;
    return m;
  };
  const [amt, setAmt] = useState<Record<string, number>>(lethalFirst);
  const sum = Object.values(amt).reduce((a, b) => a + b, 0);
  const set = (key: string, val: number) => setAmt((m) => ({ ...m, [key]: Math.max(0, Math.min(k.total, Math.floor(val) || 0)) }));
  const problem = sum !== k.total ? `Assign exactly ${k.total} (assigned ${sum}).` : '';
  const row = (key: string, name: string, hint?: string) => (
    <div key={key} className="row" style={{ gap: 8, alignItems: 'center', marginBottom: 4 }}>
      <span style={{ flex: 1, minWidth: 0 }}>
        {name}
        {hint && <span style={{ color: 'var(--mu)', fontSize: 12 }}> · {hint}</span>}
      </span>
      <button className="tb sm" onClick={() => set(key, (amt[key] ?? 0) - 1)} aria-label={`Less damage to ${name}`}>−</button>
      <input type="number" min={0} max={k.total} value={amt[key] ?? 0} onChange={(e) => set(key, Number(e.target.value))} style={{ width: 56 }} aria-label={`Damage to ${name}`} />
      <button className="tb sm" onClick={() => set(key, (amt[key] ?? 0) + 1)} aria-label={`More damage to ${name}`}>+</button>
    </div>
  );
  return (
    <div style={{ background: 'var(--surface)', border: '1px solid var(--acc)', borderRadius: 12, padding: 16 }}>
      <h3 className="display" style={{ margin: '0 0 6px', fontSize: 17 }}>Assign {nameOfCard(k.attacker)}'s combat damage ({k.total})</h3>
      <div style={{ color: 'var(--mu)', fontSize: 12.5, marginBottom: 8 }}>Divide the damage among the blockers{k.defender ? ' and the defender' : ''}. Lethal damage is shown for each blocker.</div>
      {k.blockers.map(([id, lethal]) => row(String(id), nameOfCard(id), `lethal ${lethal}`))}
      {k.defender && row('d', defName, 'defending')}
      {problem && <div style={{ color: 'var(--bad, #e4787a)', fontSize: 12.5, margin: '4px 0' }}>{problem}</div>}
      <div className="row" style={{ gap: 8, marginTop: 8 }}>
        <button data-primary
          className="tb pri"
          disabled={!!problem}
          onClick={() => onAnswer({ t: 'Damage', assign: [...k.blockers.map(([id]) => [{ kind: 'Card', id } as Obj, amt[String(id)] ?? 0] as [Obj, number]), ...(k.defender ? [[k.defender, amt.d ?? 0] as [Obj, number]] : [])].filter(([, n]) => n > 0) })}
        >
          Confirm
        </button>
        <button className="tb" onClick={() => setAmt(lethalFirst())}>Lethal first</button>
      </div>
    </div>
  );
}

export function PromptPanel({ d, v, names, images, onAnswer, onExplain, onPreviewBlocks }: { d: R.Decision; v: RView; names: string[]; images: Record<string, string>; onAnswer: (c: Choice) => void; onExplain?: (kind: 'play' | 'attack' | 'block' | 'tap', card: number | null, other: number | null) => void; /** what proposed blocks would do (kept / dropped blocks, where damage goes) */ onPreviewBlocks?: (blocks: [number, number][]) => Promise<R.BlockPreview[]> }) {
  const k = d.kind;
  const me = v.you;
  const [sel, setSel] = useState<number[]>([]);
  const [num, setNum] = useState<number | null>(null);
  const mouse = useRef({ x: 300, y: 300 });
  useEffect(() => {
    const f = (e: MouseEvent) => (mouse.current = { x: e.clientX, y: e.clientY });
    window.addEventListener('mousemove', f);
    return () => window.removeEventListener('mousemove', f);
  }, []);
  /** attackers: who each chosen attacker goes after; `armed` is the creature (or 'all') waiting for a click on who it attacks */
  const [defOf2, setDefOf2] = useState<Record<number, Obj>>({});
  const defRef = useRef(defOf2);
  defRef.current = defOf2;
  const [armed, setArmed] = useState<number | 'all' | null>(null);
  const armedRef = useRef(armed);
  armedRef.current = armed;
  /** blockers: each of your creatures with the attackers it blocks */
  const [blk, setBlk] = useState<Record<number, number[]>>({});
  /** the engine's reading of the proposed blocks: which it keeps, which it drops (and why), where each attacker's damage goes */
  const [preview, setPreview] = useState<R.BlockPreview[] | null>(null);
  useEffect(() => {
    if (k.t !== 'Blockers' || !onPreviewBlocks) return;
    const blocks = Object.entries(blk).flatMap(([b, as]) => as.map((a) => [Number(b), a] as [number, number]));
    let live = true;
    const t = setTimeout(() => {
      onPreviewBlocks(blocks)
        .then((p) => live && setPreview(p))
        .catch(() => live && setPreview(null));
    }, 80);
    return () => {
      live = false;
      clearTimeout(t);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [blk, k.t]);
  const selRef = useRef<number[]>([]);
  selRef.current = sel;
  const key = JSON.stringify(k).slice(0, 400);
  useEffect(() => {
    setSel([]);
    setNum(null);
    setDefOf2({});
    setArmed(null);
    setBlk({});
  }, [key]);

  const pickOpts = k.t === 'Pick' ? k.options.map((o) => resolveOpt(o, v, names)) : [];
  const tablePick = k.t === 'Pick' && pickOpts.length > 0 && pickOpts.some((r) => onTable(r.card, me));
  const gridPick = k.t === 'Pick' && isGridPick(d, v, names);

  type AttackOpt = Extract<R.Decision['kind'], { t: 'Attackers' }>['options'][number];
  // attackers: who an option may attack, and sending the armed creature (or all of them) at a defender
  const sameObj = (a: Obj, b: Obj) => a.kind === b.kind && (a as { id: number }).id === (b as { id: number }).id;
  const canAttack = (o: AttackOpt, ob: Obj) => (ob.kind === 'Player' ? o.players.includes(ob.id) : ob.kind === 'Card' ? o.permanents.includes(ob.id) : false);
  const defendersOf = (o: AttackOpt): Obj[] => [...o.players.map((id) => ({ kind: 'Player', id }) as Obj), ...o.permanents.map((id) => ({ kind: 'Card', id }) as Obj)];
  const attackOpts = k.t === 'Attackers' ? k.options : [];
  const assignAttack = (target: Obj) => {
    const a = armedRef.current;
    if (a === null) return;
    if (a === 'all') {
      const ok = attackOpts.filter((o) => canAttack(o, target));
      if (!ok.length) return;
      setDefOf2((m) => ({ ...m, ...Object.fromEntries(ok.map((o) => [o.card, target])) }));
    } else {
      const o = attackOpts.find((x) => x.card === a);
      if (!o) return;
      if (!canAttack(o, target)) return void onExplain?.('attack', a, target.kind === 'Card' ? target.id : null);
      setDefOf2((m) => ({ ...m, [a]: target }));
    }
    setArmed(null);
  };
  const assignRef = useRef(assignAttack);
  assignRef.current = assignAttack;

  // targets, attackers and blockers are clicked on the table itself
  useEffect(() => {
    if (k.t === 'Pick' && tablePick) {
      const { max, min } = k;
      const byCard = new Map<string, number>();
      pickOpts.forEach((r, i) => {
        if (r.card && onTable(r.card, me)) byCard.set(String(r.card.id), i);
      });
      useTable.setState({
        pickIds: [...byCard.keys()],
        pick: (iid) => {
          const i = byCard.get(iid);
          if (i === undefined) return;
          if (max === 1 && min >= 1) return onAnswer({ t: 'Pick', indices: [i] });
          setSel((s) => (s.includes(i) ? s.filter((x) => x !== i) : max === 1 ? [i] : s.length < max ? [...s, i] : s));
        },
      });
    } else if (k.t === 'Bottom') {
      const { hand, n } = k;
      useTable.setState({
        pickIds: hand.map(String),
        pick: (iid) => {
          const id = Number(iid);
          setSel((s) => (s.includes(id) ? s.filter((x) => x !== id) : s.length < n ? [...s, id] : s));
        },
      });
    } else if (k.t === 'Attackers') {
      const opts = k.options;
      const walkers = new Set(opts.flatMap((o) => o.permanents));
      useTable.setState({
        pickIds: [...opts.map((o) => o.card), ...walkers].map(String),
        pickDenied: (iid) => {
          const id = Number(iid);
          if (v.cards.find((c) => c.id === id)?.controller === me) onExplain?.('attack', id, null);
        },
        pick: (iid) => {
          const id = Number(iid);
          const o = opts.find((x) => x.card === id);
          if (!o) {
            // a planeswalker or battle: the armed creature attacks it
            if (walkers.has(id)) assignRef.current({ kind: 'Card', id });
            return;
          }
          // an attacker clicked again comes out of the attack
          if (defRef.current[id]) {
            setDefOf2(({ [id]: _drop, ...rest }) => rest);
            return setArmed(null);
          }
          // only one thing it could attack: no need to click it
          const defs = defendersOf(o);
          if (defs.length === 1) {
            setDefOf2((m) => ({ ...m, [id]: defs[0] }));
            return setArmed(null);
          }
          setArmed((a) => (a === id ? null : id));
        },
      });
    } else if (k.t === 'PayMana') {
      const bySource = new Map<string, typeof k.sources>();
      for (const s of k.sources) bySource.set(String(s.card), [...(bySource.get(String(s.card)) ?? []), s]);
      const usable = POOL_COLORS.filter((c, i) => (k.pool[i] ?? 0) > 0);
      useTable.setState({
        pickIds: [...bySource.keys()],
        pick: (iid) => {
          const srcs = bySource.get(iid);
          if (!srcs) return;
          const options = srcs.flatMap(tapChoices);
          if (options.length === 1) return onAnswer({ t: 'Pay', answer: { t: 'Tap', card: options[0].s.card, ability: options[0].s.ability, produce: options[0].m } });
          useMenu.getState().open(mouse.current.x + 2, mouse.current.y + 2, () =>
            options.map(({ s, m, text }) => ({ kind: 'item' as const, label: `Add ${text}`, run: () => onAnswer({ t: 'Pay', answer: { t: 'Tap', card: s.card, ability: s.ability, produce: m } }) })),
          );
        },
        payPool: { usable: [...usable], pay: (color) => onAnswer({ t: 'Pay', answer: { t: 'Pool', mana: color as R.ManaType } }) },
      });
    } else if (k.t === 'Blockers') {
      const opts = k.options;
      const mine = new Set(opts.map((o) => o.card));
      const attackers = new Set(opts.flatMap((o) => o.can_block));
      useTable.setState({
        pickIds: [...mine, ...attackers].map(String),
        pickDenied: (iid) => {
          const id = Number(iid);
          const b = selRef.current[0];
          const ownIt = v.cards.find((c) => c.id === id)?.controller === me;
          if (ownIt) onExplain?.('block', id, null);
          else onExplain?.('block', b ?? null, id);
        },
        pick: (iid) => {
          const id = Number(iid);
          if (mine.has(id) && !(selRef.current[0] !== undefined && attackers.has(id) && selRef.current[0] !== id)) return setSel((s) => (s[0] === id ? [] : [id]));
          const b = selRef.current[0];
          if (b === undefined || !attackers.has(id)) return;
          const o = opts.find((x) => x.card === b);
          if (!o || !o.can_block.includes(id)) {
            onExplain?.('block', b, id);
            return void setSel([]);
          }
          setBlk((m) => {
            const cur = m[b] ?? [];
            const next = cur.includes(id) ? [] : [id]; // a creature blocks one attacker
            const { [b]: _drop, ...rest } = m;
            return next.length ? { ...rest, [b]: next } : rest;
          });
          setSel([]);
        },
      });
    }
    return () => useTable.setState({ pick: null, pickIds: [], pickDenied: null, pickSeat: null, pickSeats: [], sel: [], payPool: null });
  }, [key]);
  useEffect(() => {
    if (k.t === 'Blockers') useTable.setState({ sel: sel.map(String) });
    else if (k.t === 'Pick' && tablePick) useTable.setState({ sel: sel.map((i) => String(pickOpts[i]?.card?.id)) });
    else if (k.t === 'Bottom') useTable.setState({ sel: sel.map(String) });
  }, [sel]);
  useEffect(() => {
    if (k.t !== 'Attackers') return;
    useTable.setState({ sel: [...Object.keys(defOf2), ...(typeof armed === 'number' ? [String(armed)] : [])] });
    // with a creature armed, the players it can attack are outlined and clicking their side sends it at them
    const seats = armed === null ? [] : [...new Set((armed === 'all' ? attackOpts : attackOpts.filter((o) => o.card === armed)).flatMap((o) => o.players))];
    useTable.setState({ pickSeats: seats, pickSeat: seats.length ? (seat) => assignRef.current({ kind: 'Player', id: seat }) : null });
  }, [defOf2, armed, key]);
  // Arrows for what is being declared, redrawn as the choices change
  useEffect(() => {
    if (k.t === 'Attackers') {
      useTable.setState({
        liveArrows: Object.entries(defOf2).map(([a, def]) => ({ id: a, kind: 'attack' as const, from: { iid: a }, to: def.kind === 'Player' ? { seat: def.id } : { iid: String((def as { id: number }).id) }, draft: true })),
      });
    } else if (k.t === 'Blockers') {
      const toOf = (t: Obj) => (t.kind === 'Player' ? { seat: t.id } : { iid: String((t as { id: number }).id) });
      const dropped = new Set((preview ?? []).flatMap((p) => p.dropped.map(([b]) => `${b}>${p.attacker}`)));
      useTable.setState({
        liveArrows: [
          // the blocks as chosen; ones the rules would drop are grey
          ...Object.entries(blk).flatMap(([b, as]) => as.map((a) => ({ id: b + '>' + a, kind: 'block' as const, from: { iid: b }, to: { iid: String(a) }, draft: true, illegal: dropped.has(`${b}>${a}`) }))),
          // and where each attacker's damage would go with these blocks
          ...(preview ?? []).flatMap((p) => p.damage_to.map((t, i) => ({ id: `${p.attacker}>${i}`, kind: 'attack' as const, from: { iid: String(p.attacker) }, to: toOf(t), draft: true }))),
        ],
      });
    }
    return () => useTable.setState({ liveArrows: [] });
  }, [defOf2, blk, key, preview]);

  const toggle = (i: number, max = 99) => setSel((s) => (s.includes(i) ? s.filter((x) => x !== i) : max === 1 ? [i] : s.length < max ? [...s, i] : s));
  const box: React.CSSProperties = { background: 'var(--surface)', border: '1px solid var(--acc)', borderRadius: 12, padding: 16 };
  const cardName = (id: number) => v.cards.find((c) => c.id === id)?.name ?? 'a card';

  if (k.t === 'PayMana') {
    const pool = Object.fromEntries(POOL_COLORS.map((c, i) => [c, k.pool[i] ?? 0]));
    const usable = POOL_COLORS.filter((c) => pool[c] > 0);
    return (
      <div style={box}>
        <h3 className="display" style={{ margin: '0 0 8px', fontSize: 18 }}>Pay for {k.what}</h3>
        <div style={{ marginBottom: 8, display: 'flex', alignItems: 'center', gap: 8 }}>
          <span style={{ fontSize: 12, color: 'var(--mu)' }}>Still to pay</span>
          <ManaCostText cost={k.remaining} size={24} />
        </div>
        <div style={{ fontSize: 12.5, color: 'var(--tx3)', marginBottom: 8 }}>Click a land on the table to tap it, or click mana in your pool.</div>
        <div style={{ display: 'flex', gap: 8, marginBottom: 10 }}>
          {POOL_COLORS.map((c) => (
            <ManaChip key={c} sym={c} size={26} count={pool[c]} dim={!pool[c]} hot={usable.includes(c)} onClick={usable.includes(c) ? () => onAnswer({ t: 'Pay', answer: { t: 'Pool', mana: c as R.ManaType } }) : undefined} />
          ))}
        </div>
        <button className="tb pri" data-autopay onClick={() => onAnswer({ t: 'Pay', answer: { t: 'Auto' } })} title="Let the engine tap lands and spend pool mana for the rest">Auto-pay</button>{' '}
        {k.can_pay_life && <><button className="tb" onClick={() => onAnswer({ t: 'Pay', answer: { t: 'Life' } })}>Pay 2 life</button>{' '}</>}
        <button className="tb" onClick={() => onAnswer({ t: 'Cancel' })} title="Cancel the spell; mana already spent on it is refunded">Cancel</button>
      </div>
    );
  }

  if (k.t === 'Mulligan') {
    return (
      <div style={box}>
        <h3 style={{ margin: '0 0 8px' }}>Keep this hand?{k.mulligans ? ` (mulligan ${k.mulligans})` : ''}</h3>
        <div style={{ display: 'flex', flexWrap: 'wrap', marginBottom: 8 }}>
          {k.hand.map((id) => (
            <div key={id} style={{ margin: 3, padding: '4px 10px', border: '1px solid var(--line2)', borderRadius: 8, background: 'var(--surface2)' }}>{cardName(id)}</div>
          ))}
        </div>
        {k.peek.length > 0 && (
          <>
            <div style={{ fontSize: 12.5, color: 'var(--mu)', margin: '2px 0 4px' }}>
              Scry {k.peek.length === 3 ? 3 : `(${k.peek.length} left on top)`}: top of your library, top card first. Put any on the bottom and order the rest before you keep; a mulligan puts your hand and the {k.peek.length === 1 ? 'card' : `${k.peek.length} cards`} still on top on the bottom.
            </div>
            <div style={{ display: 'flex', flexWrap: 'wrap', gap: 10, marginBottom: 10 }}>
              {k.peek.map((id, i) => {
                const c = v.decision_cards.find((x) => x.id === id);
                const def = c ? defOf(c, images[c.name], images['@back:' + c.name]) : null;
                const swap = (j: number) => {
                  const order = k.peek.map((_, x) => x);
                  [order[i], order[j]] = [order[j], order[i]];
                  onAnswer({ t: 'Order', order });
                };
                return (
                  <div key={id} style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 4 }}>
                    {def ? (
                      <div {...previewProps(def, undefined, 0, 'o:' + id)}>
                        <CardImage def={def} style={{ width: 110, height: Math.round((110 * 88) / 63), borderRadius: 6 }} />
                      </div>
                    ) : (
                      <div style={{ width: 110, padding: '4px 6px', border: '1px dashed var(--line4)', borderRadius: 8 }}>a card</div>
                    )}
                    <span style={{ fontSize: 11.5, textAlign: 'center', maxWidth: 110, lineHeight: 1.2 }}>
                      {i + 1}. {c?.name ?? 'a card'}
                    </span>
                    <div style={{ display: 'flex', gap: 3, marginTop: 'auto' }}>
                      <button className="tb sm" disabled={i === 0} title="Move up (closer to the top)" onClick={() => swap(i - 1)}>◀</button>
                      <button className="tb sm" title="Put this card on the bottom of your library" onClick={() => onAnswer({ t: 'Pick', indices: [i] })}>Bottom</button>
                      <button className="tb sm" disabled={i === k.peek.length - 1} title="Move down (further from the top)" onClick={() => swap(i + 1)}>▶</button>
                    </div>
                  </div>
                );
              })}
            </div>
          </>
        )}
        <button className="tb pri" data-primary data-option onClick={() => onAnswer({ t: 'Keep', keep: true })}>Keep</button>{' '}
        <button className="tb" data-option onClick={() => onAnswer({ t: 'Keep', keep: false })}>Mulligan</button>
      </div>
    );
  }

  if (k.t === 'Bottom') {
    return (
      <div style={box}>
        <h3 style={{ margin: '0 0 8px' }}>Put {k.n} card{k.n === 1 ? '' : 's'} on the bottom of your library</h3>
        <div style={{ color: 'var(--mu)', fontSize: 12.5, marginBottom: 8 }}>Click cards in your hand to choose them. Chosen: {sel.length}/{k.n}{sel.length ? ` — ${sel.map(cardName).join(', ')}` : ''}.</div>
        <button className="tb pri" data-primary disabled={sel.length !== k.n} onClick={() => onAnswer({ t: 'Bottom', cards: sel })}>Confirm</button>
      </div>
    );
  }

  if (k.t === 'YesNo') {
    // the engine puts what a "you may" is about (the ability's printed text) on the lines after the question
    const [question, ...about] = k.prompt.split('\n');
    return (
      <div style={box}>
        <h3 className="display" style={{ margin: about.length ? '0 0 6px' : '0 0 10px', fontSize: 18 }}>{question}</h3>
        {about.length > 0 && <p style={{ margin: '0 0 10px', fontSize: 13, lineHeight: 1.45, color: 'var(--tx2)', borderLeft: '2px solid var(--line4)', paddingLeft: 8 }}>{about.join(' ')}</p>}
        <button className="tb pri" data-primary data-option onClick={() => onAnswer({ t: 'YesNo', yes: true })}>Yes</button>{' '}
        <button className="tb" data-option onClick={() => onAnswer({ t: 'YesNo', yes: false })}>No</button>
      </div>
    );
  }

  if (k.t === 'Number') {
    const val = Math.min(k.max, Math.max(k.min, num ?? k.min));
    return (
      <div style={box}>
        <h3 style={{ margin: '0 0 8px' }}>{k.prompt}</h3>
        <input type="number" min={k.min} max={k.max} value={val} onChange={(e) => setNum(Number(e.target.value))} style={{ width: 80 }} />{' '}
        <button className="tb pri" data-primary onClick={() => onAnswer({ t: 'Number', n: val })}>Confirm</button>
        {k.cancellable && <>{' '}<button className="tb" onClick={() => onAnswer({ t: 'Cancel' })} title="Back out: the cast or ability is undone">✕ Cancel</button></>}
      </div>
    );
  }

  if (k.t === 'Pick' && k.purpose.t === 'ModeRepeat') {
    return <RepeatPick prompt={k.prompt} options={k.options.map((o) => resolveOpt(o, v, names).label)} min={k.min} max={k.max} cancellable={k.cancellable} onAnswer={onAnswer} />;
  }

  if (k.t === 'Pick') {
    const { prompt, min, max, cancellable } = k;
    const players = pickOpts.map((r, i) => ({ r, i })).filter(({ r }) => r.player !== undefined);
    const others = pickOpts.map((r, i) => ({ r, i })).filter(({ r }) => r.player === undefined && !(tablePick && onTable(r.card, me)) && !gridPick);
    return (
      <div style={box}>
        <h3 className="display" style={{ margin: '0 0 6px', fontSize: 17 }}>{prompt}{min !== 1 || max !== 1 ? ` (choose ${min === max ? min : `${min}–${max}`})` : ''}</h3>
        {tablePick && (
          <div style={{ color: 'var(--mu)', fontSize: 12.5, marginBottom: 8 }}>
            Click {max === 1 ? 'one' : 'the'} card{max === 1 ? '' : 's'} on the table to choose{max === 1 ? ' it' : ' them'}.
            {sel.length ? ` Chosen: ${sel.map((i) => pickOpts[i]?.label).join(', ')}.` : ''}
          </div>
        )}
        {gridPick && <PickGrid cards={pickOpts.map((r, idx) => ({ idx, card: r.card! }))} sel={sel} images={images} toggle={toggle} max={max} />}
        {(players.length > 0 || others.length > 0) && (
          <div style={{ display: 'flex', flexWrap: 'wrap', marginBottom: 6 }}>
            {[...players, ...others].map(({ r, i }) => (
              <button key={i} data-option className={sel.includes(i) ? 'tb pri' : 'tb'} style={{ margin: 3 }} onClick={() => (max === 1 && min >= 1 ? onAnswer({ t: 'Pick', indices: [i] }) : toggle(i, max))}>
                <RichLabel text={r.label} />
              </button>
            ))}
          </div>
        )}
        <div className="row" style={{ gap: 8 }}>
          {(max > 1 || min < 1 || gridPick) && (
            <button className="tb pri" data-primary disabled={sel.length < min} onClick={() => onAnswer({ t: 'Pick', indices: sel })}>
              {sel.length === 0 && min === 0 ? 'None' : 'Confirm'}
            </button>
          )}
          {cancellable &&
            (k.purpose.t === 'Targets' && k.purpose.optional ? (
              <button className="tb" onClick={() => onAnswer({ t: 'Cancel' })} title="This ability is optional: don't use it (it isn't put on the stack)">
                Don’t use it
              </button>
            ) : (
              <button className="tb" onClick={() => onAnswer({ t: 'Cancel' })} title="Back out: the cast or ability is undone, and anything tapped as a cost untaps">✕ Cancel</button>
            ))}
        </div>
      </div>
    );
  }

  if (k.t === 'AssignDamage') return <DamagePrompt d={d} v={v} names={names} onAnswer={onAnswer} />;

  if (k.t === 'Order') {
    return <OrderPrompt prompt={k.prompt} items={k.items.map((o) => resolveOpt(o, v, names).label)} onAnswer={(order) => onAnswer({ t: 'Order', order })} />;
  }

  if (k.t === 'Attackers') {
    const defName = (o: Obj) => (o.kind === 'Player' ? names[o.id] : o.kind === 'Card' ? cardName(o.id) : '?');
    const chosen = Object.entries(defOf2);
    // several things to attack: a creature is clicked, then who it attacks; otherwise one click attacks
    const choose = k.options.some((o) => defendersOf(o).length > 1);
    const walkers = k.options.some((o) => o.permanents.length > 0);
    // who the armed creature (or all of them) can go after, as small buttons for when that side isn't on screen
    const targets: Obj[] = [];
    for (const o of armed === null ? [] : armed === 'all' ? k.options : k.options.filter((x) => x.card === armed)) for (const t of defendersOf(o)) if (!targets.some((x) => sameObj(x, t))) targets.push(t);
    const hint =
      armed !== null
        ? `Now click who ${armed === 'all' ? 'they all attack' : `${cardName(armed)} attacks`}: a player's battlefield or name panel${walkers ? ', or a planeswalker or battle' : ''}.`
        : choose
          ? 'Click a creature, then the player it attacks (their battlefield or name panel). Click an attacker again to take it out.'
          : 'Click your creatures to attack; click one again to take it out.';
    return (
      <div style={{ ...box, padding: '10px 12px' }}>
        <div style={{ display: 'flex', alignItems: 'baseline', gap: 8, marginBottom: 4 }}>
          <h3 className="display" style={{ margin: 0, fontSize: 15 }}>Declare attackers</h3>
          {chosen.length > 0 && <span style={{ fontSize: 12, color: 'var(--mu)' }}>{chosen.length} attacking</span>}
        </div>
        <div style={{ color: 'var(--mu)', fontSize: 12, marginBottom: 6, lineHeight: 1.35 }}>{hint}</div>
        {armed !== null && targets.length > 1 && (
          <div style={{ display: 'flex', flexWrap: 'wrap', gap: 4, marginBottom: 6 }}>
            {targets.map((t) => (
              <button key={t.kind + (t as { id: number }).id} className="tb sm" onClick={() => assignAttack(t)}>→ {defName(t)}</button>
            ))}
          </div>
        )}
        <div className="row" style={{ gap: 6, flexWrap: 'wrap' }}>
          <button className="tb pri sm" data-primary onClick={() => onAnswer({ t: 'Attackers', attacks: chosen.map(([a, def]) => [Number(a), def] as [number, Obj]) })}>
            {chosen.length ? `Attack with ${chosen.length}` : 'No attack'}
          </button>
          {k.options.length > 1 &&
            (choose ? (
              <button className={'tb sm' + (armed === 'all' ? ' on' : '')} title="Send every creature that can attack at the player you click next" onClick={() => setArmed((a) => (a === 'all' ? null : 'all'))}>All…</button>
            ) : (
              <button className="tb sm" onClick={() => setDefOf2(Object.fromEntries(k.options.flatMap((o) => (defendersOf(o)[0] ? [[o.card, defendersOf(o)[0]]] : []))))}>All attack</button>
            ))}
          {chosen.length > 0 && <button className="tb sm" onClick={() => { setDefOf2({}); setArmed(null); }}>Clear</button>}
        </div>
      </div>
    );
  }

  if (k.t === 'Blockers') {
    const pairs = Object.entries(blk).flatMap(([b, as]) => as.map((a) => [Number(b), a] as const));
    const dropped = pairs.map(([b, a]) => ({ b, a, why: preview?.find((p) => p.attacker === a)?.dropped.find(([x]) => x === b)?.[1] })).filter((x) => x.why);
    return (
      <div style={{ ...box, padding: '10px 12px' }}>
        <div style={{ display: 'flex', alignItems: 'baseline', gap: 8, marginBottom: 4 }}>
          <h3 className="display" style={{ margin: 0, fontSize: 15 }}>Declare blockers</h3>
          {pairs.length > 0 && <span style={{ fontSize: 12, color: 'var(--mu)' }}>{pairs.length} blocking</span>}
        </div>
        <div style={{ color: 'var(--mu)', fontSize: 12, marginBottom: 6, lineHeight: 1.35 }}>
          {sel.length ? `Now click the attacker ${cardName(sel[0])} blocks.` : 'Click one of your creatures, then the attacker it blocks. Do it again to take a block back.'}
        </div>
        {dropped.map(({ b, a, why }) => (
          <div key={b + '>' + a} style={{ color: 'var(--warn, #d9a441)', fontSize: 12, marginBottom: 4 }}>⚠ {cardName(b)} can't block {cardName(a)}: {why}. That block will be dropped.</div>
        ))}
        <div className="row" style={{ gap: 6 }}>
          <button className="tb pri sm" data-primary onClick={() => onAnswer({ t: 'Blockers', blocks: pairs.map(([b, a]) => [b, a] as [number, number]) })}>{pairs.length ? `Confirm ${pairs.length === 1 ? 'block' : 'blocks'}` : 'No blocks'}</button>
          {pairs.length > 0 && <button className="tb sm" onClick={() => { setBlk({}); setSel([]); }}>Clear</button>}
        </div>
      </div>
    );
  }

  return null;
}

/** Choosing modes where the same mode may be taken several times: a counter per mode. */
function RepeatPick({ prompt, options, min, max, cancellable, onAnswer }: { prompt: string; options: string[]; min: number; max: number; cancellable: boolean; onAnswer: (c: Choice) => void }) {
  const [counts, setCounts] = useState<number[]>(options.map(() => 0));
  const total = counts.reduce((a, b) => a + b, 0);
  const set = (i: number, d: number) => setCounts((c) => c.map((x, j) => (j === i ? Math.max(0, x + d) : x)));
  return (
    <div style={{ background: 'var(--surface)', border: '1px solid var(--acc)', borderRadius: 12, padding: 16 }}>
      <h3 className="display" style={{ margin: '0 0 6px', fontSize: 17 }}>{prompt} (choose {min === max ? min : `${min}–${max}`}; a mode may be chosen more than once)</h3>
      {options.map((o, i) => (
        <div key={i} className="row" style={{ gap: 8, alignItems: 'center', marginBottom: 4 }}>
          <span style={{ flex: 1 }}><RichLabel text={o} /></span>
          <button className="tb sm" disabled={counts[i] === 0} onClick={() => set(i, -1)} aria-label={`Fewer: ${o}`}>−</button>
          <b style={{ minWidth: 18, textAlign: 'center' }}>{counts[i]}</b>
          <button className="tb sm" disabled={total >= max} onClick={() => set(i, 1)} aria-label={`More: ${o}`}>+</button>
        </div>
      ))}
      <div className="row" style={{ gap: 8, marginTop: 8 }}>
        <button className="tb pri" data-primary disabled={total < min || total > max} onClick={() => onAnswer({ t: 'Pick', indices: counts.flatMap((n, i) => Array(n).fill(i)) })}>Confirm</button>
        {cancellable && <button className="tb" onClick={() => onAnswer({ t: 'Cancel' })}>✕ Cancel</button>}
      </div>
    </div>
  );
}

function OrderPrompt({ prompt, items, onAnswer }: { prompt: string; items: string[]; onAnswer: (order: number[]) => void }) {
  const [order, setOrder] = useState(items.map((_, i) => i));
  const move = (from: number, to: number) => setOrder((o) => { const n = o.slice(); const [x] = n.splice(from, 1); n.splice(to, 0, x); return n; });
  return (
    <div style={{ background: 'var(--surface)', border: '1px solid var(--acc)', borderRadius: 12, padding: 16 }}>
      <h3 style={{ margin: '0 0 8px' }}>{prompt}</h3>
      {order.map((idx, pos) => (
        <div key={idx} className="row" style={{ gap: 6, marginBottom: 4 }}>
          <span style={{ flex: 1 }}>{pos + 1}. {items[idx]}</span>
          <button className="tb sm" disabled={pos === 0} onClick={() => move(pos, pos - 1)}>↑</button>
          <button className="tb sm" disabled={pos === order.length - 1} onClick={() => move(pos, pos + 1)}>↓</button>
        </div>
      ))}
      <button className="tb pri" data-primary onClick={() => onAnswer(order)}>Confirm order</button>
    </div>
  );
}
