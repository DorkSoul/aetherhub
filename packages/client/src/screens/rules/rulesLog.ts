import type { rules as R } from '@aetherhub/shared';
import { COUNTER_NAMES, roundOf } from './rulesView';

type Ev = R.Event;
type View = R.View;

/** Everything a game log needs across frames: the players' names and the names of cards seen before (cards that are
 * hidden again later, like a sacrificed creature's controller shuffling it away, keep their name). */
export interface LogContext {
  names: string[];
  known: Map<number, string>;
  owners?: Map<number, number>;
}

/** Costs seen before a spell or ability is cast or activated: they are paid first, so they arrive first. */
interface Payment {
  player: number | null;
  mana: string[];
  manaFrom: number[];
  tapped: number[];
  sacrificed: number[];
  discarded: number[];
  exiled: number[];
  life: number;
  counters: string[];
  crew: number[];
}

const emptyPayment = (): Payment => ({ player: null, mana: [], manaFrom: [], tapped: [], sacrificed: [], discarded: [], exiled: [], life: 0, counters: [], crew: [] });
const paymentEmpty = (p: Payment) => !p.mana.length && !p.manaFrom.length && !p.tapped.length && !p.sacrificed.length && !p.discarded.length && !p.exiled.length && !p.life && !p.counters.length && !p.crew.length;

/** A spell or ability being put on the stack: its line is written when the frame ends, once targets are known. */
interface Action {
  line: number;
  base: string;
  source: number;
  sid: number | null;
  text: string;
  x: number;
  targets: string[];
  pay: Payment | null;
}

const MANA_ORDER = ['W', 'U', 'B', 'R', 'G', 'C'];
const list = (xs: string[]) => (xs.length <= 1 ? (xs[0] ?? '') : `${xs.slice(0, -1).join(', ')} and ${xs[xs.length - 1]}`);
const plural = (n: number, one: string, many = one + 's') => `${n} ${n === 1 ? one : many}`;
/** a counter kind as players read it: "+1/+1", "loyalty", "oil" */
export const counterName = (k: string) => COUNTER_NAMES[k] ?? k.replace(/([a-z])([A-Z])/g, '$1 $2').toLowerCase();

/**
 * Readable game log lines for one frame of engine events: who cast or activated what, the ability's text, its targets
 * and what was paid for it, and what happened as a result (damage, life, cards moving, counters, power changes).
 * `prev` is the board before the frame (to name cards that left it and to see power / toughness change).
 */
export function describeFrame(events: Ev[], view: View, prev: View | null, ctx: LogContext): string[] {
  const owners = (ctx.owners ??= new Map());
  for (const c of [...(prev?.cards ?? []), ...view.cards]) {
    if (c.name) ctx.known.set(c.id, c.name);
    owners.set(c.id, c.owner);
  }
  const findCard = (id: number) => view.cards.find((c) => c.id === id) ?? prev?.cards.find((c) => c.id === id);
  const nameOf = (id: number) => findCard(id)?.name || ctx.known.get(id) || null;
  const card = (id: number) => nameOf(id) ?? 'a hidden card';
  const who = (p: number) => ctx.names[p] ?? `Player ${p + 1}`;
  const obj = (o: R.Obj): string => (o.kind === 'Player' ? who(o.id) : o.kind === 'Card' ? card(o.id) : 'a spell or ability');
  const ownerOf = (id: number) => findCard(id)?.owner ?? owners.get(id) ?? null;

  const out: string[] = [];
  const actions: Action[] = [];
  let pay = emptyPayment();
  /** the spell or ability whose StackPut and targets are still to come */
  const cur: { open: Action | null } = { open: null };
  let draws: { player: number; cards: number[] } | null = null;
  const sacrificedNow = new Set<number>();
  // life totals as the frame goes, to show what each change leaves
  const life = (prev ?? view).players.map((pl) => pl.life);
  const skip = new Set<number>();
  const used = new Set<number>();
  const landPlayed = new Set(events.flatMap((e) => (e.t === 'LandPlayed' ? [e.card] : [])));
  const attackers = new Set(events.flatMap((e) => (e.t === 'Attacked' ? [e.card] : [])));

  const flushDraws = () => {
    if (!draws) return;
    const known = draws.cards.map(nameOf);
    out.push(known.every(Boolean) ? `${who(draws.player)} draws ${list(known as string[])}` : `${who(draws.player)} draws ${plural(draws.cards.length, 'card')}`);
    draws = null;
  };
  /** Costs that were not followed by a spell or ability: what they were on their own. */
  const flushPayment = () => {
    const p = pay;
    pay = emptyPayment();
    if (paymentEmpty(p)) return;
    const by = p.player !== null ? who(p.player) : 'Someone';
    if (p.mana.length) out.push(`${by} adds ${manaText(p.mana)}${p.manaFrom.length ? ` (tapping ${list(p.manaFrom.map(card))})` : ''}`);
    for (const id of p.tapped) out.push(`${card(id)} becomes tapped`);
    for (const id of p.crew) out.push(`${card(id)} is tapped to crew`);
    for (const id of p.sacrificed) out.push(`${who(ownerOf(id) ?? p.player ?? 0)} sacrifices ${card(id)}`);
    for (const id of p.discarded) out.push(`${by} discards ${card(id)}`);
    for (const id of p.exiled) out.push(`${card(id)} is exiled from a graveyard`);
    if (p.life) out.push(`${by} pays ${p.life} life`);
    for (const c of p.counters) out.push(c);
  };
  const startAction = (base: string, source: number, sid: number | null) => {
    flushDraws();
    const p = pay;
    pay = emptyPayment();
    cur.open = { line: out.length, base, source, sid, text: '', x: 0, targets: [], pay: paymentEmpty(p) ? null : p };
    actions.push(cur.open);
    out.push(base);
  };
  const payText = (a: Action) => {
    const p = a.pay;
    if (!p) return '';
    const parts: string[] = [];
    if (p.mana.length) parts.push(`paying ${manaText(p.mana)}${p.manaFrom.length ? ` (tapping ${list(p.manaFrom.map(card))})` : ''}`);
    const tapped = p.tapped.filter((id) => id !== a.source && !p.manaFrom.includes(id));
    if (tapped.length) parts.push(`tapping ${list(tapped.map(card))}`);
    if (p.crew.length) parts.push(`tapping ${list(p.crew.map(card))} to crew`);
    const sacrificed = p.sacrificed.filter((id) => id !== a.source);
    if (sacrificed.length) parts.push(`sacrificing ${list(sacrificed.map(card))}`);
    else if (p.sacrificed.length) parts.push('sacrificing it');
    if (p.discarded.length) parts.push(`discarding ${list(p.discarded.map(card))}`);
    if (p.exiled.length) parts.push(`exiling ${list(p.exiled.map(card))} from a graveyard`);
    if (p.life) parts.push(`paying ${p.life} life`);
    for (const c of p.counters) parts.push(c.replace(/^Removed/, 'removing'));
    return parts.length ? ` — ${list(parts)}` : '';
  };
  /** An event that is not a cost: costs gathered so far belong to nothing, and the open action is complete. */
  const settle = () => {
    flushPayment();
    cur.open = null;
  };
  const say = (line: string | null) => {
    if (line === null) return;
    flushDraws();
    settle();
    out.push(line);
  };

  for (const [i, e] of events.entries()) {
    // damage to a player comes with the life it costs (just before or after it): one line for both
    if (skip.has(i)) continue;
    if (e.t === 'Damage' && e.target.kind === 'Player' && e.amount > 0) {
      const p = e.target.id;
      const j = [i - 1, i + 1].find((k) => {
        const l = events[k];
        return l?.t === 'Life' && l.player === p && l.delta === -e.amount && !skip.has(k) && !used.has(k);
      });
      if (j !== undefined) {
        if (j > i) skip.add(j);
        life[p] = (life[p] ?? 0) - e.amount;
        say(`${e.source !== null ? card(e.source) : 'Something'} deals ${e.amount}${e.combat ? ' combat' : ''} damage to ${who(p)} (${life[p]} life left)`);
        continue;
      }
    }
    if (e.t === 'Life' && e.delta < 0) {
      const d = events[i + 1];
      if (d?.t === 'Damage' && d.target.kind === 'Player' && d.target.id === e.player && d.amount === -e.delta) {
        used.add(i);
        continue;
      }
    }
    if (e.t === 'Attacked') {
      // one line per declared attack: every attacker declared in this frame, grouped by what it attacks
      if (events.findIndex((x) => x.t === 'Attacked') !== i) continue;
      const run = events.filter((x): x is Extract<Ev, { t: 'Attacked' }> => x.t === 'Attacked');
      const byDefender = new Map<string, number[]>();
      for (const a of run) byDefender.set(obj(a.defender), [...(byDefender.get(obj(a.defender)) ?? []), a.card]);
      const attacker = findCard(run[0].card)?.controller;
      for (const [def, ids] of byDefender) say(`${attacker !== undefined ? who(attacker) : 'Someone'} attacks ${def} with ${counted(ids.map(card))}`);
      continue;
    }
    if (e.t === 'Life') {
      life[e.player] = (life[e.player] ?? 0) + e.delta;
      say(e.delta > 0 ? `${who(e.player)} gains ${e.delta} life (now ${life[e.player]})` : `${who(e.player)} loses ${-e.delta} life (now ${life[e.player]})`);
      continue;
    }
    // a one-shot scheme goes back into its deck hidden: remember its name
    if (e.t === 'SchemeSetInMotion') ctx.known.set(e.card, e.name);
    if (e.t !== 'Drew') flushDraws();
    switch (e.t) {
      // ---- costs (and mana) come before the spell or ability they pay for
      case 'Tapped':
        if (!attackers.has(e.card)) pay.tapped.push(e.card);
        break;
      case 'Mana':
        pay.player = e.player;
        pay.mana.push(e.mana);
        break;
      case 'ManaTapped':
        pay.player = e.player;
        pay.manaFrom.push(e.card);
        pay.tapped = pay.tapped.filter((id) => id !== e.card);
        break;
      case 'LifePaid':
        pay.player = e.player;
        pay.life += e.n;
        break;
      case 'Sacrificed':
        pay.player ??= e.player;
        pay.sacrificed.push(e.card);
        sacrificedNow.add(e.card);
        break;
      case 'Discarded':
        if (e.by !== null && e.by !== e.player) {
          say(`${who(e.player)} discards ${card(e.card)}`);
          break;
        }
        pay.player ??= e.player;
        pay.discarded.push(e.card);
        break;
      case 'CrewPaid':
        pay.crew.push(e.creature);
        pay.tapped = pay.tapped.filter((id) => id !== e.creature);
        break;
      case 'Counters':
        if (e.delta < 0) pay.counters.push(`Removed ${plural(-e.delta, `${counterName(e.kind)} counter`)} from ${card(e.card)}`);
        else say(`${card(e.card)} gets ${plural(e.delta, `${counterName(e.kind)} counter`)}`);
        break;
      case 'Activated':
        // a mana ability's mana and taps were gathered above
        if (e.mana) break;
        startAction(`${who(e.player)} activates ${card(e.card)}`, e.card, null);
        break;
      case 'Cast':
        startAction(`${who(e.player)} casts ${card(e.card)}`, e.card, null);
        break;
      case 'AbilityTriggered':
        flushPayment();
        startAction(`${card(e.source)} (${who(e.player)}) triggers`, e.source, e.id);
        break;
      case 'StackPut': {
        const o = cur.open;
        const a = o && (o.sid === null || o.sid === e.id) ? o : null;
        if (a) {
          a.sid = e.id;
          a.text = e.text;
          a.x = e.x;
        }
        break;
      }
      case 'Targeted':
      case 'PlayerTargeted': {
        const a = actions.find((x) => x.sid === e.sid);
        const name = e.t === 'Targeted' ? card(e.card) : who(e.player);
        if (a) a.targets.push(name);
        break;
      }
      case 'Expended':
      case 'Crime':
      case 'Shuffled':
      case 'StepBegin':
      case 'Untapped':
      case 'Unblocked':
      case 'BecameBlocked':
      case 'Resolved':
      case 'GameStart':
      case 'DiceRolled':
      case 'Cycled':
        break;
      case 'Drew':
        if (draws && draws.player !== e.player) flushDraws();
        settle();
        draws ??= { player: e.player, cards: [] };
        draws.cards.push(e.card);
        break;
      case 'Moved':
        if (e.from === 'Graveyard' && e.to === 'Exile' && !cur.open) {
          pay.exiled.push(e.card);
          break;
        }
        if (sacrificedNow.has(e.card) && e.to === 'Graveyard') break;
        say(moveLine(e, card, findCard, landPlayed, who, ownerOf));
        break;
      default:
        say(eventLine(e, card, who, obj, ctx.names.length));
    }
  }
  flushDraws();
  flushPayment();
  for (const a of actions) {
    out[a.line] = `${a.base}${a.text ? `: “${a.text}”` : ''}${a.x ? ` (X = ${a.x})` : ''}${a.targets.length ? `, targeting ${list(a.targets)}` : ''}${payText(a)}`;
  }
  if (prev) out.push(...sizeChanges(prev, view));
  return collapse(out);
}

/** "A, A, B" → "2× A and B" */
function counted(names: string[]): string {
  const n = new Map<string, number>();
  for (const x of names) n.set(x, (n.get(x) ?? 0) + 1);
  return list([...n].map(([x, k]) => (k > 1 ? `${k}× ${x}` : x)));
}

/** The same line several times in a row (three identical creatures dealing damage) is written once, with a count. */
function collapse(lines: string[]): string[] {
  const out: string[] = [];
  let k = 0;
  for (let i = 0; i < lines.length; i++) {
    k++;
    if (lines[i + 1] === lines[i] && !/life left|\(now /.test(lines[i])) continue;
    out.push(k > 1 ? `${lines[i]} (×${k})` : lines[i]);
    k = 0;
  }
  return out;
}

function manaText(mana: string[]): string {
  return [...mana].sort((a, b) => MANA_ORDER.indexOf(a) - MANA_ORDER.indexOf(b)).map((m) => `{${m}}`).join('');
}

function moveLine(
  e: Extract<Ev, { t: 'Moved' }>,
  card: (id: number) => string,
  findCard: (id: number) => R.CardView | undefined,
  landPlayed: Set<number>,
  who: (p: number) => string,
  ownerOf: (id: number) => number | null,
): string | null {
  const n = card(e.card);
  const owner = ownerOf(e.card);
  const whose = owner !== null ? `${who(owner)}'s` : 'its owner\'s';
  const creature = /Creature/.test(findCard(e.card)?.types ?? '');
  const key = `${e.from}>${e.to}`;
  switch (key) {
    case 'Battlefield>Graveyard': return creature ? `${n} dies` : `${n} is put into ${whose} graveyard`;
    case 'Battlefield>Exile': return `${n} is exiled`;
    case 'Battlefield>Hand': return `${n} returns to ${whose} hand`;
    case 'Battlefield>Library': return `${n} is put into ${whose} library`;
    case 'Battlefield>Command': return `${n} goes to the command zone`;
    case 'Stack>Graveyard':
    case 'Hand>Stack':
    case 'Command>Stack':
    case 'Stack>Battlefield':
      return null;
    case 'Stack>Exile': return `${n} is exiled as it leaves the stack`;
    case 'Stack>Hand': return `${n} returns to ${whose} hand from the stack`;
    case 'Stack>Library': return `${n} is put into ${whose} library from the stack`;
    case 'Hand>Battlefield': return landPlayed.has(e.card) ? null : `${n} is put onto the battlefield from ${whose} hand`;
    case 'Hand>Exile': return `${n} is exiled from ${whose} hand`;
    case 'Hand>Library': return `${n} is put from ${whose} hand into their library`;
    case 'Hand>Graveyard': return null;
    case 'Graveyard>Battlefield': return `${n} returns from ${whose} graveyard to the battlefield`;
    case 'Graveyard>Hand': return `${n} returns from ${whose} graveyard to their hand`;
    case 'Graveyard>Library': return `${n} is put from ${whose} graveyard into their library`;
    case 'Graveyard>Exile': return `${n} is exiled from ${whose} graveyard`;
    case 'Graveyard>Stack': return null;
    case 'Library>Hand': return `${n} is put into ${whose} hand from their library`;
    case 'Library>Battlefield': return `${n} enters the battlefield from ${whose} library`;
    case 'Library>Exile': return `${n} is exiled from ${whose} library`;
    case 'Library>Graveyard': return null;
    case 'Exile>Battlefield': return `${n} returns from exile to the battlefield`;
    case 'Exile>Hand': return `${n} is put into ${whose} hand from exile`;
    case 'Exile>Graveyard': return `${n} is put into ${whose} graveyard from exile`;
    case 'Exile>Stack': return null;
    case 'Command>Battlefield': return `${n} enters the battlefield from the command zone`;
    default: return e.to === 'Battlefield' ? `${n} enters the battlefield` : null;
  }
}

function eventLine(e: Ev, card: (id: number) => string, who: (p: number) => string, obj: (o: R.Obj) => string, players: number): string | null {
  switch (e.t) {
    case 'Notice': return `⚠ ${who(e.player)}: ${e.text}`;
    case 'TurnBegin': return `— ${who(e.player)}'s turn ${roundOf(e.turn, players)} —`;
    case 'LandPlayed': return `${who(e.player)} plays ${card(e.card)}${e.from_hand ? '' : ` from their ${e.from.toLowerCase()}`}`;
    case 'Copied': return `${who(e.player)} copies ${card(e.card)}`;
    case 'Countered': return `${card(e.card)} is countered${e.by !== null ? ` by ${who(e.by)}` : ''}`;
    case 'Fizzled': return `${card(e.card)} does nothing: its targets are no longer legal`;
    case 'Attacked': return `${card(e.card)} attacks ${obj(e.defender)}`;
    case 'Blocked': return `${card(e.blocker)} blocks ${card(e.attacker)}`;
    case 'Damage': return e.amount > 0 ? `${e.source !== null ? card(e.source) : 'Something'} deals ${e.amount}${e.combat ? ' combat' : ''} damage to ${obj(e.target)}` : null;
    case 'Life': return e.delta > 0 ? `${who(e.player)} gains ${e.delta} life` : `${who(e.player)} loses ${-e.delta} life`;
    case 'Token': return `A ${card(e.card)} token is created`;
    case 'Regenerated': return `${card(e.card)} regenerates`;
    case 'Lost': return `${who(e.player)} loses the game (${e.reason})`;
    case 'Mulligan': return e.keep ? `${who(e.player)} keeps their hand` : `${who(e.player)} takes a mulligan`;
    case 'Over': return e.result.t === 'Winner' ? `${who(e.result.player)} wins the game` : 'The game is a draw';
    case 'Scry': return e.surveil ? `${who(e.player)} surveils ${e.n} (${e.bottom} into the graveyard)` : `${who(e.player)} scries ${e.n} (${e.bottom} to the bottom)`;
    case 'Attach': return e.to !== null ? `${card(e.card)} is attached to ${card(e.to)}` : null;
    case 'Unattached': return `${card(e.card)} becomes unattached from ${card(e.from)}`;
    case 'Revealed': return `${card(e.card)} is revealed`;
    case 'Control': return `${who(e.player)} gains control of ${card(e.card)}`;
    case 'ControlLost': return `${card(e.card)} changes control from ${who(e.from)} to ${who(e.to)}`;
    case 'Transformed': return `${card(e.card)} transforms`;
    case 'EnteredTransformed': return `${card(e.card)} enters transformed`;
    case 'TurnedFaceUp': return `${card(e.card)} is turned face up`;
    case 'Monarch': return `${who(e.player)} becomes the monarch`;
    case 'Saddled': return `${card(e.card)} becomes saddled`;
    case 'Crewed': return `${who(e.player)} crews ${card(e.card)}`;
    case 'Exploited': return e.victim !== null ? `${card(e.card)} exploits ${card(e.victim)}` : null;
    case 'KeywordAction': return `${card(e.card)} ${e.action.toLowerCase()}${e.other !== null ? ` ${card(e.other)}` : ''}`;
    case 'Monstrous': return `${card(e.card)} becomes monstrous (${e.n})`;
    case 'Mutated': return `${card(e.card)} mutates`;
    case 'Plotted': return `${card(e.card)} is plotted`;
    case 'Foretold': return `${card(e.card)} is foretold`;
    case 'Exerted': return `${card(e.card)} is exerted`;
    case 'Solved': return `${card(e.card)} is solved`;
    case 'SagaFinalChapter': return `${card(e.card)} reaches its final chapter`;
    case 'FacedDilemma': return `${who(e.player)} faces a villainous choice`;
    case 'Renowned': return `${card(e.card)} becomes renowned`;
    case 'Clashed': return `${who(e.player)} clashes and ${e.won ? 'wins' : 'loses'}`;
    case 'VotingFinished': return 'The vote is over';
    case 'Explored': return `${card(e.card)} explores${e.revealed !== null ? ` (reveals ${card(e.revealed)})` : ''}`;
    case 'Investigated': return `${who(e.player)} investigates`;
    case 'EvidenceCollected': return `${who(e.player)} collects evidence`;
    case 'Connived': return `${card(e.card)} connives`;
    case 'EnergyGained': return `${who(e.player)} gets ${e.n} energy`;
    case 'Discovered': return `${who(e.player)} discovers ${e.n}`;
    case 'PlayerCounters': return e.delta > 0 ? `${who(e.player)} gets ${plural(e.delta, `${counterName(e.kind)} counter`)}` : `${who(e.player)} loses ${plural(-e.delta, `${counterName(e.kind)} counter`)}`;
    case 'DieRolled': return `${who(e.player)} rolls a d${e.sides}: ${e.n}`;
    case 'CoinFlipped': return `${who(e.player)} flips a coin and ${e.won ? 'wins' : 'loses'}`;
    case 'Proliferated': return `${who(e.player)} proliferates`;
    case 'Searched': return `${who(e.player)} searches their library`;
    case 'Milled': return e.cards.length ? `${who(e.player)} mills ${list(e.cards.map(card))}` : null;
    case 'ManifestedDread': return `${who(e.player)} manifests dread`;
    case 'GiftGiven': return `${who(e.player)} gives a gift`;
    case 'Bent': return `${who(e.player)} ${e.kind.toLowerCase()}bends`;
    case 'RingTempted': return `The Ring tempts ${who(e.player)}`;
    case 'RingBearerChosen': return `${card(e.card)} becomes ${who(e.player)}'s Ring-bearer`;
    case 'InitiativeTaken': return `${who(e.player)} takes the initiative`;
    case 'RoomEntered': return `${who(e.player)} enters ${e.name} (${card(e.dungeon)})`;
    case 'DungeonCompleted': return `${who(e.player)} completes a dungeon`;
    case 'Unlocked': return `${card(e.card)}: the ${e.door === 0 ? 'left' : 'right'} door is unlocked`;
    case 'FullyUnlocked': return `${card(e.card)} is fully unlocked`;
    case 'PhasedOut': return `${card(e.card)} phases out`;
    case 'PhasedIn': return `${card(e.card)} phases in`;
    case 'DayNightChanged': return e.day ? 'It becomes day' : 'It becomes night';
    case 'PlanarDieRolled': return `${who(e.player)} rolls the planar die: ${e.face === 'Chaos' ? 'chaos' : e.face === 'Planeswalk' ? 'planeswalk' : 'blank'}`;
    case 'ChaosEnsued': return 'Chaos ensues';
    case 'PlaneswalkingAway': return null;
    case 'Planeswalked': return `${who(e.player)} planeswalks to ${card(e.to)}`;
    case 'SchemeSetInMotion': return `${who(e.player)} sets ${e.name} in motion`;
    case 'ChoiceRevealed': return `${who(e.player)} chose ${e.option ?? (e.cards.length ? list(e.cards.map(card)) : 'nothing')} (${card(e.source)})`;
    case 'Log': return e.text;
    default: return null;
  }
}

/** Creatures whose power or toughness changed during the frame (pump spells, anthems, -X/-X effects). */
function sizeChanges(prev: View, view: View): string[] {
  const before = new Map(prev.cards.map((c) => [c.id, c]));
  const lines: string[] = [];
  for (const c of view.cards) {
    const was = before.get(c.id);
    if (!was || c.zone !== 'Battlefield' || was.zone !== 'Battlefield' || c.power === null || c.toughness === null || was.power === null || was.toughness === null) continue;
    if (c.power !== was.power || c.toughness !== was.toughness) lines.push(`${c.name || 'A face-down creature'} is now ${c.power}/${c.toughness} (was ${was.power}/${was.toughness})`);
  }
  return lines.length > 8 ? [...lines.slice(0, 8), `…and ${lines.length - 8} more creatures changed size`] : lines;
}
