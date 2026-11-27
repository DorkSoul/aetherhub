import type { CardDef, CardView, GameView, Intent, ZoneName } from '@aetherhub/shared';
import { basePT, DUNGEON_NAMES, DUNGEONS, turnKind } from '@aetherhub/shared';
import { useApp } from '../../lib/store';
import { socket } from '../../lib/ws';
import { askConfirm } from '../../ui/dialogs';
import type { MenuItem } from '../../ui/ui';
import { toggleTurned, useMenu, useTurned } from '../../ui/ui';
import { mySeat, useTable, type CardDetailsTarget } from './store';

export const EFFECTS = ['Flying', 'Haste', 'Hexproof', 'Indestructible', 'Trample', 'Lifelink', 'Vigilance', 'Deathtouch'];
let menuX = 3;

const st = () => useTable.getState();
const view = () => st().view!;
const send = (i: Intent) => st().send(i);
const refresh = () => useMenu.getState().refresh();

export function locateView(v: GameView, iid: string): { seat: number; zone: ZoneName; index: number } | null {
  for (let s = 0; s < v.seats.length; s++) {
    for (const z of Object.keys(v.seats[s].zones) as ZoneName[]) {
      const i = v.seats[s].zones[z].indexOf(iid);
      if (i >= 0) return { seat: s, zone: z, index: i };
    }
  }
  return null;
}

export function cardLabel(v: GameView, c: CardView | undefined, seat?: number): string {
  if (!c) return 'Card';
  if (c.hidden || !v.defs[c.def]) return c.faceDown ? 'Face-down card' : 'Hidden card';
  const d = v.defs[c.def];
  const name = d.faces && d.img.length > 1 ? d.faces[c.face]?.name || d.name : d.name;
  return name + (seat != null && c.owner !== seat ? ` · owned by ${v.seats[c.owner]?.name}` : '');
}

function xStep(): MenuItem {
  return {
    kind: 'step',
    label: 'X =',
    value: menuX,
    dec: () => {
      menuX = Math.max(1, menuX - 1);
      refresh();
    },
    inc: () => {
      menuX = Math.min(30, menuX + 1);
      refresh();
    },
  };
}

/** Selected cards if the clicked card is part of a multi-selection, else just it. */
function targets(iid: string): string[] {
  const sel = st().sel;
  return sel.includes(iid) && sel.length > 1 ? sel : [iid];
}

/** The card a details window shows: a card on the table only while this player can see it (never a face-down card
 *  they may not look at), or a def handed over by a look or reveal window. */
export function detailsCard(v: GameView, t: CardDetailsTarget): { def: CardDef; card?: CardView } | null {
  const c = t.iid != null ? v.cards[t.iid] : undefined;
  const live = c && !c.hidden ? v.defs[c.def] : undefined;
  if (t.def) return { def: t.def, card: live?.key === t.def.key ? c : undefined };
  return live ? { def: live, card: c } : null;
}

/** "Card details…" for a card on the table this player can see, or null. */
export function detailsItem(v: GameView, iid: string): MenuItem | null {
  if (!detailsCard(v, { iid })) return null;
  return { kind: 'item', label: 'Card details…', hint: 'only you', run: () => useTable.setState({ details: { iid } }) };
}

/** The menu of a card shown in a window (zone, look, reveal): its details. */
export function detailsMenu(def: CardDef | undefined, target: CardDetailsTarget): MenuItem[] {
  if (!def) return [];
  return [
    { kind: 'head', label: def.name },
    { kind: 'item', label: 'Card details…', hint: 'only you', run: () => useTable.setState({ details: target }) },
  ];
}

export function cardMenu(iid: string): MenuItem[] {
  const v = view();
  const c = v.cards[iid];
  const loc = locateView(v, iid);
  if (!c || !loc) return [];
  const me = mySeat(v);
  const I: MenuItem[] = [];
  const ids = targets(iid);
  const multi = ids.length > 1;
  const o = c.owner;
  const item = (label: string, run: () => void, hint?: string, danger?: boolean): MenuItem => ({ kind: 'item', label, run, hint, danger });
  const toZone = (zone: ZoneName, extra: Partial<Extract<Intent, { type: 'move' }>> = {}) => {
    // Menu moves go to each card's owner's zone.
    const byOwner = new Map<number, string[]>();
    for (const i of ids) {
      const cc = v.cards[i];
      if (!cc) continue;
      byOwner.set(cc.owner, [...(byOwner.get(cc.owner) || []), i]);
    }
    for (const [seat, list] of byOwner) send({ type: 'move', iids: list, seat, zone, ...extra });
  };
  const nth = item('Nth from the top of library…', () => useTable.setState({ modal: { kind: 'nth', iid } }));
  const library = (): MenuItem[] => [item('Top of library', () => toZone('library', { pos: 'top' })), item('Bottom of library', () => toZone('library', { pos: 'bottom' })), ...(multi ? [] : [nth])];

  if (loc.zone === 'battlefield') {
    I.push({ kind: 'head', label: multi ? `${ids.length} selected` : cardLabel(v, c, loc.seat) });
    I.push(item(c.tapped ? 'Untap' : 'Tap', () => send({ type: 'tap', iids: ids, tapped: !c.tapped }), 'T'));
    I.push(item(c.faceDown ? 'Turn face up' : 'Turn face down', () => send({ type: 'faceDown', iids: ids, value: !c.faceDown })));
    I.push({ kind: 'sep' });
    I.push({
      kind: 'sub',
      label: 'Move to…',
      items: () => {
        const M: MenuItem[] = [item('Hand', () => toZone('hand'), "owner's"), item('Graveyard', () => toZone('graveyard')), item('Exile', () => toZone('exile')), { kind: 'sep' }, ...library(), { kind: 'sep' }, item('Command zone', () => toZone('command'))];
        const others = v.seats.map((s, i) => ({ s, i })).filter((x) => x.i !== loc.seat);
        if (others.length)
          M.push(
            { kind: 'sep' },
            { kind: 'dots', label: 'Give control to', dots: others.map((x) => ({ label: x.s.name, color: x.s.color, run: () => send({ type: 'move', iids: ids, seat: x.i, zone: 'battlefield', x: c.x, y: 1 - c.y }) })) },
          );
        return M;
      },
    });
    I.push({ kind: 'sub', label: 'Counters…', items: () => counterItems(iid, ids) });
    if (!multi) I.push({ kind: 'sub', label: 'Effects and P/T…', items: () => effectItems(iid) });
    const piles = pileItems(iid, ids);
    if (piles.length) I.push({ kind: 'sub', label: 'Pile…', items: () => pileItems(iid, ids) });
    I.push({ kind: 'sub', label: 'More…', items: () => moreMenu(iid, ids) });
    if (ids.every((i) => v.cards[i]?.token)) I.push({ kind: 'sep' }, item(multi ? 'Delete tokens' : 'Delete token', () => send({ type: 'delete', iids: ids }), 'Del', true));
  } else if (loc.zone === 'hand') {
    I.push({ kind: 'head', label: multi ? `${ids.length} selected` : cardLabel(v, c) });
    I.push(item('Play to battlefield', () => send({ type: 'move', iids: ids, seat: me ?? loc.seat, zone: 'battlefield', x: 0.45, y: 0.4 }), 'dbl-click'));
    I.push(item('Play face down', () => send({ type: 'move', iids: ids, seat: me ?? loc.seat, zone: 'battlefield', x: 0.6, y: 0.4, faceDown: true })));
    I.push({ kind: 'sep' });
    I.push({
      kind: 'sub',
      label: 'Move to…',
      items: () => [item('Graveyard (discard)', () => toZone('graveyard')), item('Exile', () => toZone('exile')), item('Exile face down', () => toZone('exile', { faceDown: true })), { kind: 'sep' }, ...library(), { kind: 'sep' }, item('Command zone', () => toZone('command'))],
    });
    I.push(item('Reveal to everyone', () => send({ type: 'reveal', iids: ids })));
  } else {
    I.push({ kind: 'head', label: cardLabel(v, c) });
    I.push({
      kind: 'sub',
      label: 'Move to…',
      items: () =>
        (
          [
            ['hand', 'Hand'],
            ['battlefield', 'Battlefield'],
            ['graveyard', 'Graveyard'],
            ['exile', 'Exile'],
            ['library', 'Top of library'],
            ['command', 'Command zone'],
          ] as [ZoneName, string][]
        )
          .filter(([z]) => z !== loc.zone)
          .map(([z, l]) => item(l, () => send({ type: 'move', iids: ids, seat: z === 'hand' || z === 'battlefield' ? (me ?? o) : o, zone: z }))),
    });
  }
  if (!multi) {
    const info = [detailsItem(v, iid), lookItem(iid)].filter((x): x is MenuItem => !!x);
    if (info.length) I.push({ kind: 'sep' }, ...info);
  }
  return I;
}

/** Counters on the clicked card (or +1/+1 on every selected card), and a counter of any type. */
function counterItems(iid: string, ids: string[]): MenuItem[] {
  const v = view();
  const c = v.cards[iid];
  if (!c) return [];
  const def = v.defs[c.def];
  const step = (label: string, counter: string, value: string | number): MenuItem => ({ kind: 'step', label, value, dec: () => send({ type: 'counter', iids: ids, counter, delta: -1 }), inc: () => send({ type: 'counter', iids: ids, counter, delta: 1 }) });
  const I: MenuItem[] = [];
  if (ids.length > 1) {
    I.push(step('+1/+1 (each)', '+1/+1', '±'), step('−1/−1 (each)', '-1/-1', '±'));
  } else {
    if (basePT(def, c.face) || c.hidden) I.push(step('+1/+1', '+1/+1', c.counters['+1/+1'] || 0), step('−1/−1', '-1/-1', c.counters['-1/-1'] || 0));
    const others = Object.keys(c.counters).filter((k) => k !== '+1/+1' && k !== '-1/-1');
    for (const k of Array.from(new Set(['charge', ...others]))) I.push(step(`${k[0].toUpperCase()}${k.slice(1)}`, k, c.counters[k] || 0));
  }
  I.push({ kind: 'sep' }, { kind: 'input', label: 'Counter of any type', placeholder: 'e.g. loyalty, time, oil', button: '+1', submit: (t) => send({ type: 'counter', iids: ids, counter: t.toLowerCase(), delta: 1 }) });
  return I;
}

/** Keywords, custom effects and power/toughness changes on one card. */
function effectItems(iid: string): MenuItem[] {
  const v = view();
  const c = v.cards[iid];
  if (!c) return [];
  const custom = c.effects.map((e) => e.k).filter((k) => !EFFECTS.includes(k));
  const sign = (n: number) => `${n >= 0 ? '+' : ''}${n}`;
  return [
    {
      kind: 'chips',
      label: 'Effects (stay until removed · click again to remove)',
      chips: [...EFFECTS, ...custom].map((k) => ({ label: k + (c.effects.find((e) => e.k === k)?.eot ? ' · EOT' : ''), on: c.effects.some((e) => e.k === k), run: () => send({ type: 'effect', iid, k }) })),
    },
    { kind: 'input', label: 'Custom effect', placeholder: 'e.g. Can’t block, Goaded', submit: (t) => send({ type: 'effect', iid, k: t }) },
    { kind: 'input', label: 'Effect until end of turn', placeholder: 'e.g. +2/+0, Menace', submit: (t) => send({ type: 'effect', iid, k: t, eot: true }) },
    { kind: 'sep' },
    { kind: 'step', label: 'P/T until end of turn', value: sign(c.ptEot.p), dec: () => send({ type: 'ptMod', iid, p: -1, t: -1, eot: true }), inc: () => send({ type: 'ptMod', iid, p: 1, t: 1, eot: true }) },
    { kind: 'step', label: 'P/T modifier (lasting)', value: sign(c.pt.p), dec: () => send({ type: 'ptMod', iid, p: -1, t: -1, eot: false }), inc: () => send({ type: 'ptMod', iid, p: 1, t: 1, eot: false }) },
    { kind: 'step', label: 'Power only (EOT)', value: `${c.ptEot.p}`, dec: () => send({ type: 'ptMod', iid, p: -1, t: 0, eot: true }), inc: () => send({ type: 'ptMod', iid, p: 1, t: 0, eot: true }) },
  ];
}

/** Piling identical cards together, splitting and ungrouping piles. Empty when none applies. */
function pileItems(iid: string, ids: string[]): MenuItem[] {
  const v = view();
  const c = v.cards[iid];
  const loc = c && locateView(v, iid);
  if (!c || !loc) return [];
  const multi = ids.length > 1;
  const I: MenuItem[] = [];
  const item = (label: string, run: () => void): MenuItem => ({ kind: 'item', label, run });
  const bf = v.seats[loc.seat].zones.battlefield;
  const rulesGame = !!st().rules;
  if (multi && ids.every((i) => bf.includes(i) && v.cards[i]?.def === c.def && !v.cards[i]?.faceDown)) {
    // a rules game keeps every card: they can lie fanned out or exactly on top of each other with a count
    if (rulesGame) {
      I.push(item(`Pile ${ids.length} selected: fanned out`, () => send({ type: 'stack', iids: ids, tight: false })));
      I.push(item(`Pile ${ids.length} selected: stacked with count (×${ids.length})`, () => send({ type: 'stack', iids: ids, tight: true })));
    } else I.push(item(`Stack ${ids.length} selected into a pile`, () => send({ type: 'stack', iids: ids })));
  }
  if (multi) return I;
  const copies = bf.filter((i) => v.cards[i]?.def === c.def && v.cards[i]?.token === c.token).length;
  if (rulesGame && copies > 1) {
    I.push(item('Group identical: fanned out', () => send({ type: 'group', iid, tight: false })));
    I.push(item(`Group identical: stacked with count (×${copies})`, () => send({ type: 'group', iid, tight: true })));
  } else if (c.pile > 1) {
    I.push(item('Split one off the pile', () => send({ type: 'split', iid, n: 1 })));
    I.push({ kind: 'input', label: `Tap N of the ${c.pile}`, placeholder: 'N', button: 'Tap', submit: (t) => parseInt(t, 10) > 0 && send({ type: 'split', iid, n: parseInt(t, 10), tap: true }) });
    I.push(item('Ungroup pile', () => send({ type: 'ungroup', iid })));
  } else if (c.token && copies > 1) I.push(item('Group identical', () => send({ type: 'group', iid })));
  return I;
}


/** The `useTurned` key of a card on the table: a face-down card's look at its face is kept apart from turning a face-up
 *  card over, so the card isn't shown turned once it is turned face up. */
export function turnedKey(c: CardView): string {
  return c.faceDown ? 'fd:' + c.iid : c.iid;
}

/** "Look at the other side": a local look (only on this screen) at a double-faced card's other face, or at the face of a
 *  face-down card you are allowed to see. Cards you may not see have no def here, so they get no item. */
function lookItem(iid: string): MenuItem | null {
  const v = view();
  const c = v.cards[iid];
  const def = c && !c.hidden ? v.defs[c.def] : undefined;
  if (!c || !def) return null;
  const key = turnedKey(c);
  const checked = !!useTurned.getState().m[key];
  const run = () => toggleTurned(key);
  if (c.faceDown) return locateView(v, iid)?.zone === 'battlefield' ? { kind: 'item', label: 'Look at the face-down card', hint: 'only you', checked, run } : null;
  const kind = turnKind(def);
  if (!kind) return null;
  return { kind: 'item', label: kind === 'meld' ? 'Look at the melded card' : 'Look at the other side', hint: 'hold Alt', checked, run };
}

function moreMenu(iid: string, ids: string[]): MenuItem[] {
  const v = view();
  const c = v.cards[iid];
  const def = v.defs[c?.def];
  const me = mySeat(v);
  const I: MenuItem[] = [];
  if (!c) return I;
  const multi = ids.length > 1;
  // in a rules game the engine transforms and flips cards; the look at the other side is in the card menu
  const rules = !!st().rules;
  if (!rules && def?.faces && def.faces.length > 1 && turnKind(def) === 'dfc') I.push({ kind: 'item', label: 'Transform / flip over', run: () => send({ type: 'transform', iid }) });
  if (!rules && def?.layout === 'flip') I.push({ kind: 'item', label: c.flipped ? 'Unflip (rotate back)' : 'Flip (rotate 180°)', run: () => send({ type: 'flip', iid }) });
  if (c.attachedTo) I.push({ kind: 'item', label: 'Detach', run: () => send({ type: 'attach', iid, to: null }) });
  else I.push({ kind: 'item', label: 'Attach to…', run: () => useTable.setState({ modal: { kind: 'attach', iid, mode: 'attach' } }) });
  I.push({ kind: 'item', label: 'Meld with…', run: () => useTable.setState({ modal: { kind: 'attach', iid, mode: 'meld' } }) });
  I.push({ kind: 'item', label: 'Reveal to everyone', run: () => send({ type: 'reveal', iids: ids }) });
  I.push({ kind: 'sep' });
  if (!multi) I.push({ kind: 'item', label: 'Create copy', run: () => send({ type: 'copy', iid }) });
  if (!multi) I.push({ kind: 'item', label: 'Ping', hint: 'Middle-click', run: () => socket.send({ t: 'ping', iid }) });
  I.push({ kind: 'item', label: 'Draw arrow', hint: 'Alt-drag', run: () => useApp.getState().toast('Hold Alt and drag from a card to a card or player') });
  if (!multi) I.push({ kind: 'item', label: "Doesn't untap", checked: c.noUntap, run: () => send({ type: 'noUntap', iid, value: !c.noUntap }) });
  if (c.faceDown && me != null && (c.peekers.includes(me) || c.owner === me)) {
    const others = v.seats.map((s, i) => ({ s, i })).filter((x) => !c.peekers.includes(x.i));
    if (others.length) I.push({ kind: 'dots', label: 'Let a player peek', dots: others.map((x) => ({ label: x.s.name, color: x.s.color, run: () => send({ type: 'grantPeek', iid, seat: x.i }) })) });
  }
  return I;
}

export function libraryMenu(seat: number): MenuItem[] {
  const v = view();
  const p = v.seats[seat];
  const me = mySeat(v);
  const mine = seat === me;
  const I: MenuItem[] = [{ kind: 'head', label: `${mine ? 'Your' : p.name + "'s"} library · ${p.zones.library.length}` }, xStep()];
  const it = (label: string, run: () => void, hint?: string): MenuItem => ({ kind: 'item', label, run, hint });
  if (me != null) {
    I.push(it(mine ? 'Draw 1' : 'Draw 1 (to your hand)', () => send({ type: 'draw', seat, n: 1 }), mine ? 'D' : ''));
    I.push(it(`Draw X (${menuX})`, () => send({ type: 'draw', seat, n: menuX })));
    I.push(it('Draw from the bottom', () => send({ type: 'draw', seat, n: 1, bottom: true })));
    I.push({ kind: 'sep' });
    I.push(it('Scry 1', () => send({ type: 'look', seat, n: 1, mode: 'scry' })));
    I.push(it(`Scry X (${menuX})`, () => send({ type: 'look', seat, n: menuX, mode: 'scry' })));
    I.push(it(`Look at top X (${menuX})`, () => send({ type: 'look', seat, n: menuX, mode: 'look' }), 'private'));
    I.push(it(`Reveal top X (${menuX})`, () => send({ type: 'look', seat, n: menuX, mode: 'reveal' })));
  }
  I.push(it('Reveal top card', () => send({ type: 'revealTop', seat })));
  I.push(it(`${p.revealTop ? 'Stop playing' : 'Play'} with top revealed`, () => send({ type: 'toggleRevealTop', seat })));
  I.push({ kind: 'sep' });
  I.push(it(`Mill X (${menuX})`, () => send({ type: 'mill', seat, n: menuX })));
  I.push(it(`Exile top X (${menuX})`, () => send({ type: 'exileTop', seat, n: menuX, faceDown: false })));
  I.push(it(`Exile top X face down (${menuX})`, () => send({ type: 'exileTop', seat, n: menuX, faceDown: true })));
  I.push(it('Random card to your hand', () => send({ type: 'randomCard', seat, zone: 'library', to: 'hand' })));
  I.push({ kind: 'sep' });
  if (me != null) I.push(it('Search…', () => send({ type: 'search', seat })));
  I.push(it('Shuffle', () => send({ type: 'shuffle', seat }), mine ? 'S' : ''));
  return I;
}

export function handMenu(seat: number): MenuItem[] {
  const v = view();
  const p = v.seats[seat];
  const me = mySeat(v);
  const I: MenuItem[] = [];
  if (seat === me) {
    I.push({ kind: 'head', label: `Your hand · ${p.zones.hand.length}` });
    I.push({ kind: 'item', label: p.handRevealAll ? 'Stop revealing hand' : 'Reveal hand to everyone', run: () => send({ type: 'revealHand', value: !p.handRevealAll }) });
    const others = v.seats.map((x, i) => ({ x, i })).filter((o) => o.i !== seat && !p.handViewers.includes(o.i));
    if (others.length && !p.handRevealAll) I.push({ kind: 'dots', label: 'Reveal hand to one player', dots: others.map((o) => ({ label: o.x.name, color: o.x.color, run: () => send({ type: 'revealHand', value: true, seat: o.i }) })) });
    if (p.handViewers.length)
      I.push({ kind: 'dots', label: 'Revoke hand view', dots: p.handViewers.map((i) => ({ label: v.seats[i]?.name, color: v.seats[i]?.color, run: () => send({ type: 'revokeHandView', seat: i }) })) });
    I.push({ kind: 'item', label: 'Random discard', run: () => send({ type: 'randomCard', seat, zone: 'hand', to: 'graveyard' }) });
    I.push({ kind: 'item', label: 'Draw 1', run: () => send({ type: 'draw', seat, n: 1 }), hint: 'D' });
    I.push({ kind: 'item', label: 'Shuffle hand into library', run: () => {
      send({ type: 'move', iids: p.zones.hand.slice(), seat, zone: 'library', pos: 'top' });
      send({ type: 'shuffle', seat });
    } });
  } else {
    I.push({ kind: 'head', label: `${p.name}'s hand · ${p.zones.hand.length}` });
    if (me != null) {
      const viewing = p.handViewers.includes(me);
      const pending = v.requests.some((r) => r.from === me && r.to === seat);
      if (viewing) I.push({ kind: 'item', label: 'Stop viewing', run: () => send({ type: 'stopViewing', seat }) });
      else if (pending) I.push({ kind: 'item', label: 'Request pending…', run: () => {}, disabled: true });
      else I.push({ kind: 'item', label: 'Request to view hand', run: () => send({ type: 'requestHandView', seat }) });
      I.push({ kind: 'item', label: 'Random discard', run: () => send({ type: 'randomCard', seat, zone: 'hand', to: 'graveyard' }) });
      I.push({ kind: 'item', label: 'Take a random card', run: () => send({ type: 'randomCard', seat, zone: 'hand', to: 'hand' }) });
    }
  }
  return I;
}

/** In a rules game the board menu has no sandbox actions (tokens, untap all); it sorts that one board and sets how big its cards and hand are drawn. */
export interface RulesBoardOpts {
  setLayout: (patch: { handFrac?: number; cardScale?: number }) => void;
  sort: () => void;
  /** while breaking the rules: pick a token to create on this board */
  createToken?: () => void;
}

export function battlefieldMenu(seat: number, rules?: RulesBoardOpts): MenuItem[] {
  const v = view();
  const p = v.seats[seat];
  const me = mySeat(v);
  const fit: MenuItem = {
    kind: 'item',
    label: 'Fit all (reset pan & zoom)',
    run: () => {
      const pz = { ...st().pz };
      delete pz[seat];
      useTable.setState({ pz });
    },
    hint: 'local',
  };
  const head: MenuItem = { kind: 'head', label: seat === me ? 'Your battlefield' : `${p.name}'s battlefield` };
  const selectAll: MenuItem = { kind: 'item', label: 'Select all', run: () => useTable.setState({ sel: p.zones.battlefield.slice() }) };
  if (rules) {
    // the menu stays open while stepping: rebuild it once the table has redrawn with the new size
    const sort = rules.sort;
    const set = rules.setLayout;
    const setLayout = (patch: { handFrac?: number; cardScale?: number }) => {
      set(patch);
      setTimeout(() => useMenu.getState().refresh(), 0);
    };
    const round = (x: number, k: number) => Math.round(x * k) / k;
    return [
      head,
      ...(rules.createToken ? [{ kind: 'item' as const, label: 'Create token…', hint: 'rule break', run: rules.createToken }] : []),
      { kind: 'item', label: 'Sort this board', hint: seat === me ? 'S' : '', run: sort },
      { kind: 'step', label: 'Card size', value: `${Math.round(p.cardScale * 100)}%`, dec: () => setLayout({ cardScale: Math.max(0.6, round(p.cardScale - 0.1, 10)) }), inc: () => setLayout({ cardScale: Math.min(1.6, round(p.cardScale + 0.1, 10)) }) },
      { kind: 'step', label: 'Hand size', value: `${Math.round(p.handFrac * 100)}%`, dec: () => setLayout({ handFrac: Math.max(0.16, round(p.handFrac - 0.04, 100)) }), inc: () => setLayout({ handFrac: Math.min(0.44, round(p.handFrac + 0.04, 100)) }) },
      { kind: 'sep' },
      selectAll,
      fit,
    ];
  }
  return [
    head,
    { kind: 'item', label: 'Create token…', run: () => useTable.setState({ modal: { kind: 'tokens', seat } }) },
    { kind: 'item', label: 'Untap all', run: () => send({ type: 'untapAll', seat }), hint: seat === me ? 'U' : '' },
    selectAll,
    fit,
  ];
}

export function playerMenu(seat: number): MenuItem[] {
  const v = view();
  const p = v.seats[seat];
  const me = mySeat(v);
  const I: MenuItem[] = [{ kind: 'head', label: p.name + (seat === me ? ' (you)' : '') }];
  const pass = useTable.getState().rules?.passToPlayer;
  if (pass && !p.conceded) I.push({ kind: 'item', label: seat === me ? 'Skip until my next turn starts' : `Skip until ${p.name}\u2019s turn starts`, hint: 'passes priority', run: () => pass(seat) }, { kind: 'sep' });
  const stepC = (key: string, label: string): MenuItem => ({ kind: 'step', label, value: p.counters[key] || 0, dec: () => send({ type: 'playerCounter', seat, key, delta: -1 }), inc: () => send({ type: 'playerCounter', seat, key, delta: 1 }) });
  I.push({ kind: 'step', label: 'Life', value: p.life, dec: () => send({ type: 'life', seat, delta: -1 }), inc: () => send({ type: 'life', seat, delta: 1 }) });
  I.push(stepC('poison', 'Poison'), stepC('energy', 'Energy'), stepC('experience', 'Experience'), stepC('rad', 'Rad'), stepC('tickets', 'Tickets'));
  for (const cmd of Object.keys(p.cmdTax)) {
    const nm = v.cards[cmd] ? cardLabel(v, v.cards[cmd]) : 'Commander';
    I.push({ kind: 'step', label: `Tax · ${nm.split(',')[0]}`, value: '+' + p.cmdTax[cmd], dec: () => send({ type: 'cmdTax', seat, iid: cmd, delta: -2 }), inc: () => send({ type: 'cmdTax', seat, iid: cmd, delta: 2 }) });
  }
  // Commander damage taken from each other player's commanders.
  v.seats.forEach((o, oi) => {
    if (oi === seat) return;
    for (const cmd of Object.keys(o.cmdTax)) {
      const nm = v.cards[cmd] ? cardLabel(v, v.cards[cmd]).split(',')[0] : 'Commander';
      I.push({ kind: 'step', label: `Cmdr dmg · ${nm}`, value: p.cmdDmg[cmd] || 0, dec: () => send({ type: 'cmdDmg', seat, source: cmd, delta: -1 }), inc: () => send({ type: 'cmdDmg', seat, source: cmd, delta: 1 }) });
    }
  });
  I.push({ kind: 'sep' });
  I.push({ kind: 'item', label: v.global.monarch === seat ? 'Is the Monarch (remove)' : 'Make Monarch', run: () => send({ type: 'marker', key: 'monarch', seat: v.global.monarch === seat ? null : seat }) });
  I.push({ kind: 'item', label: v.global.initiative === seat ? 'Has the Initiative (remove)' : 'Give the Initiative', run: () => send({ type: 'marker', key: 'initiative', seat: v.global.initiative === seat ? null : seat }) });
  I.push({ kind: 'item', label: p.citysBlessing ? "Remove the city's blessing" : "Give the city's blessing", run: () => send({ type: 'citysBlessing', seat, value: !p.citysBlessing }) });
  I.push({ kind: 'step', label: 'The Ring tempts (level)', value: p.ring, dec: () => send({ type: 'ring', seat, delta: -1 }), inc: () => send({ type: 'ring', seat, delta: 1 }) });
  if (v.settings.dungeons || p.dungeon) {
    if (p.dungeon) {
      const rooms = DUNGEONS[p.dungeon.name] || [];
      const next = rooms[p.dungeon.room]?.next || [];
      I.push({ kind: 'head', label: `${p.dungeon.name} · ${rooms[p.dungeon.room]?.name}` });
      for (const n of next) I.push({ kind: 'item', label: `Venture → ${rooms[n].name}`, run: () => send({ type: 'dungeon', seat, name: p.dungeon!.name, room: n }) });
      I.push({ kind: 'item', label: next.length ? 'Leave dungeon' : 'Complete dungeon', run: () => send({ type: 'dungeon', seat, name: null, room: 0 }) });
    } else I.push({ kind: 'dots', label: 'Venture into…', dots: DUNGEON_NAMES.map((n) => ({ label: n, color: '#8f939c', run: () => send({ type: 'dungeon', seat, name: n, room: 0 }) })) });
  }
  const mk = v.global.markers.filter((m) => m.seat === seat);
  for (const m of mk) I.push({ kind: 'step', label: m.label, value: m.value, dec: () => send({ type: 'customMarker', seat, label: m.label, delta: -1 }), inc: () => send({ type: 'customMarker', seat, label: m.label, delta: 1 }) });
  I.push({ kind: 'input', label: 'Add a marker', placeholder: 'e.g. Speed, Blessing', submit: (t) => send({ type: 'customMarker', seat, label: t }) });
  if (mk.length) I.push({ kind: 'chips', label: 'Remove marker', chips: mk.map((m) => ({ label: m.label, run: () => send({ type: 'customMarker', seat, label: m.label, remove: true }) })) });
  if (seat === me) {
    I.push({ kind: 'sep' });
    I.push({ kind: 'item', label: 'Untap automatically at turn start', checked: p.autoUntap, run: () => send({ type: 'autoUntap', value: !p.autoUntap }), keep: true });
    I.push({ kind: 'item', label: 'Draw automatically at turn start', checked: p.autoDraw, run: () => send({ type: 'autoDraw', value: !p.autoDraw }), keep: true });
    if (!p.conceded) I.push({ kind: 'item', label: 'Concede', run: () => void askConfirm({ title: 'Concede this game?', text: 'Your cards stay on the table.', confirm: 'Concede', danger: true }).then((ok) => ok && send({ type: 'concede' })), danger: true });
  }
  if (st().host && seat !== me && st().mode === 'online') {
    I.push({ kind: 'sep' });
    I.push({ kind: 'item', label: 'Give seat to…', run: () => useTable.setState({ modal: { kind: 'giveSeat', seat } }) });
    I.push({ kind: 'item', label: 'Remove from game', danger: true, run: () => void askConfirm({ title: `Remove ${p.name} from the game?`, confirm: 'Remove', danger: true }).then((ok) => ok && socket.send({ t: 'host', op: 'removeSeat', seat })) });
  }
  return I;
}

export function diceMenu(): MenuItem[] {
  const v = view();
  const I: MenuItem[] = [{ kind: 'head', label: 'Randomness · crypto RNG on server' }];
  for (const d of [6, 20, 4, 8, 10, 12, 100]) I.push({ kind: 'item', label: `Roll d${d}`, run: () => send({ type: 'roll', sides: d }) });
  I.push({ kind: 'item', label: 'Flip a coin', run: () => send({ type: 'coin' }) });
  if (v.settings.planechase || v.table.planes.length) I.push({ kind: 'item', label: 'Roll planar die', run: () => send({ type: 'planarRoll' }) });
  return I;
}

export function hostMenu(): MenuItem[] {
  const v = view();
  const I: MenuItem[] = [{ kind: 'head', label: st().mode === 'local' ? 'Playtest' : 'Host' }];
  const restart = st().restart;
  if (restart) {
    I.push({ kind: 'item', label: 'Restart playtest (reshuffle)', run: restart });
    return I;
  }
  if (v.stage !== 'ended') {
    I.push({ kind: 'item', label: 'End game…', run: () => useTable.setState({ modal: { kind: 'end' } }) });
    if (st().mode === 'online') I.push({ kind: 'item', label: 'Suspend game', run: () => void askConfirm({ title: 'Suspend the game?', text: 'Everyone returns to the menu; it can be loaded later.', confirm: 'Suspend' }).then((ok) => ok && socket.send({ t: 'host', op: 'suspend' })) });
  } else {
    I.push({ kind: 'item', label: 'Rematch (same seats and decks)', run: () => socket.send({ t: 'host', op: 'rematch' }) });
    I.push({ kind: 'item', label: 'Close game', run: () => socket.send({ t: 'host', op: 'close' }) });
  }
  return I;
}
