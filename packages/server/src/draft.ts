/**
 * Booster draft and sealed by the real procedure:
 *  - boosters as they are printed: Play Boosters (14 cards, sets from Murders at Karlov Manor, 2024, on) or Draft Boosters
 *    (15 cards, older sets), with colour-balanced commons, the rare slot (mythic about 1 in 8 / 1 in 7), wildcards and a
 *    land slot;
 *  - a pod of 2-8 drafters (humans and AI drafters) passing left, right, left, everyone picking at the same time, with an
 *    optional pick timer (official paper times) that picks for a player who runs out of time;
 *  - sealed: six boosters per player;
 *  - AI drafters that rate cards, read which colours are flowing, watch their curve, and build a 40-card deck.
 * The whole state serialises to JSON, so a draft survives a server restart.
 */

export interface PoolCard {
  /** printing key, `p:<id>` */
  key: string;
  name: string;
  rarity: 'common' | 'uncommon' | 'rare' | 'mythic' | string;
  cmc: number;
  /** colour letters, e.g. "WU" */
  colors: string;
  /** colour identity letters (lands that make coloured mana have them) */
  identity: string;
  typeLine: string;
  text: string;
  manaCost: string;
  power: string | null;
  toughness: string | null;
}

/** One set's cards as they can come out of a booster. */
export interface SetPool {
  code: string;
  name: string;
  released: string;
  cards: PoolCard[];
  /** basic lands printed in the set (the land slot) */
  basics: PoolCard[];
}

/** Small deterministic generator (mulberry32) whose state can be saved, so drafts replay from a seed and survive restarts. */
export class Rng {
  constructor(public state: number) {}
  next(): number {
    let a = (this.state = (this.state + 0x6d2b79f5) >>> 0);
    a = Math.imul(a ^ (a >>> 15), a | 1);
    a ^= a + Math.imul(a ^ (a >>> 7), a | 61);
    return ((a ^ (a >>> 14)) >>> 0) / 4294967296;
  }
}

/** Plain-function form of the seeded generator. */
export function rng(seed: number): () => number {
  const r = new Rng(seed >>> 0);
  return () => r.next();
}

function pickRandom<T>(list: T[], r: () => number): T {
  return list[Math.floor(r() * list.length)];
}

const COLORS = ['W', 'U', 'B', 'R', 'G'];
const isLand = (c: PoolCard) => /\bLand\b/.test(c.typeLine);
const isBasic = (c: PoolCard) => /\bBasic\b/.test(c.typeLine) && isLand(c);
const isCreature = (c: PoolCard) => /\bCreature\b/.test(c.typeLine);

/** Play Boosters replaced Draft and Set Boosters with Murders at Karlov Manor (February 2024). */
export function boosterKind(released: string): 'play' | 'draft' {
  return released >= '2024-02-09' ? 'play' : 'draft';
}

/** A rarity drawn with these weights (from the published booster odds, rounded). */
function weighted(r: () => number, w: [string, number][]): string {
  const total = w.reduce((n, [, x]) => n + x, 0);
  let x = r() * total;
  for (const [k, v] of w) {
    if ((x -= v) < 0) return k;
  }
  return w[w.length - 1][0];
}

/**
 * One booster. Draft Booster: 10 commons, 3 uncommons, 1 rare or mythic (1 in 8), 1 land. Play Booster: 7 commons,
 * 3 uncommons, 1 rare or mythic (about 1 in 7), 1 wildcard of any rarity, 1 more wildcard (the traditional foil slot),
 * 1 land. Commons cover every colour when the set allows; no card twice in a pack while the set allows.
 */
export function makeBooster(set: SetPool | PoolCard[], r: () => number, kind?: 'play' | 'draft'): PoolCard[] {
  const s: SetPool = Array.isArray(set) ? { code: '', name: '', released: '', cards: set, basics: [] } : set;
  const k = kind ?? boosterKind(s.released);
  const nonBasic = s.cards.filter((c) => !isBasic(c));
  const pool = nonBasic.length ? nonBasic : s.cards;
  const by = (rar: string) => pool.filter((c) => c.rarity === rar);
  const rarities: Record<string, PoolCard[]> = { common: by('common'), uncommon: by('uncommon'), rare: by('rare'), mythic: by('mythic') };
  const pack: PoolCard[] = [];
  const add = (list: PoolCard[]) => {
    const src = list.length ? list : pool;
    let c = pickRandom(src, r);
    for (let tries = 0; tries < 15 && pack.some((x) => x.name === c.name); tries++) c = pickRandom(src, r);
    pack.push(c);
  };
  const rare = () => (rarities.mythic.length && r() < (k === 'play' ? 1 / 7 : 1 / 8) ? rarities.mythic : rarities.rare.length ? rarities.rare : rarities.mythic);
  const ofRarity = (rar: string) => (rar === 'mythic' ? (rarities.mythic.length ? rarities.mythic : rarities.rare) : rarities[rar]);
  // commons: one of each colour first (print sheets are collated so a pack has every colour), then any
  const nCommons = k === 'play' ? 7 : 10;
  const commons = rarities.common.filter((c) => !isLand(c) || !c.identity);
  for (const col of COLORS) {
    if (pack.length >= nCommons) break;
    const mono = commons.filter((c) => c.colors === col && !pack.some((x) => x.name === c.name));
    if (mono.length) pack.push(pickRandom(mono, r));
  }
  while (pack.length < nCommons) add(commons.length ? commons : rarities.common);
  for (let i = 0; i < 3; i++) add(rarities.uncommon);
  add(rare());
  if (k === 'play') {
    add(ofRarity(weighted(r, [['common', 41.7], ['uncommon', 41.7], ['rare', 14.6], ['mythic', 2]])));
    add(ofRarity(weighted(r, [['common', 66.7], ['uncommon', 25], ['rare', 6.9], ['mythic', 1.4]])));
  }
  // the land slot: a basic of the set, or one of its common lands
  const lands = s.basics.length ? s.basics : rarities.common.filter(isLand);
  if (lands.length) pack.push(pickRandom(lands, r));
  else add(rarities.common);
  return pack;
}

// ---------------------------------------------------------------------------------------------------- card ratings

/** How good a card is in limited, before colours and the drafter's deck (roughly a 0-5 scale). */
export function baseRating(c: PoolCard): number {
  let s = c.rarity === 'mythic' ? 3.6 : c.rarity === 'rare' ? 3.2 : c.rarity === 'uncommon' ? 2.2 : 1.4;
  const t = c.text.toLowerCase();
  // removal and interaction
  if (/(destroy|exile) target (creature|nonland permanent|permanent|artifact or creature|creature or planeswalker)/.test(t)) s += 1.5;
  else if (/deals? (\d+|x) damage to (any target|target creature|target creature or planeswalker|each creature)/.test(t)) s += 1.2;
  else if (/target creature (gets|an opponent controls gets) -\d+\/-\d+|fights? (target|another target|up to one target)|target creature can't (attack|block)|tap target creature.*doesn't untap/.test(t)) s += 0.8;
  if (/counter target spell/.test(t)) s += 0.3;
  if (/destroy all creatures|each creature gets -|deals \d+ damage to each creature/.test(t)) s += 0.6;
  // card advantage
  if (/draw (two|three|x) cards/.test(t)) s += 0.6;
  else if (/draw a card/.test(t)) s += 0.3;
  if (/create (a|an|two|three|x) .*creature tokens?/.test(t)) s += 0.4;
  // evasion and keywords
  if (/\bflying\b/.test(t)) s += 0.6;
  if (/\b(menace|trample|lifelink|deathtouch|first strike|double strike|hexproof|ward)\b/.test(t)) s += 0.25;
  if (/\bvigilance\b|\bhaste\b|\breach\b/.test(t)) s += 0.1;
  // body for the cost
  const p = parseInt(c.power ?? '', 10);
  const th = parseInt(c.toughness ?? '', 10);
  if (isCreature(c) && !Number.isNaN(p) && !Number.isNaN(th)) s += Math.max(-0.6, ((p + th) / (c.cmc + 1) - 1.6) * 0.5);
  if (c.cmc >= 6) s -= 0.5 * (c.cmc - 5);
  // lands: dual lands help splashing a little, utility lands are rarely picked early
  if (isLand(c)) s = isBasic(c) ? 0 : c.identity.length >= 2 ? 1.4 : 0.6;
  // auras and situational cards
  if (/^enchant creature/m.test(t) && !/\bflash\b/.test(t)) s -= 0.3;
  return s;
}

export class AiDrafter {
  weight: Record<string, number> = Object.fromEntries(COLORS.map((c) => [c, 0]));
  /** colours that kept coming in packs passed to this drafter (signals): the quality left in packs it saw */
  open: Record<string, number> = Object.fromEntries(COLORS.map((c) => [c, 0]));
  picks: PoolCard[] = [];
  constructor(private r: () => number) {}

  /** The two colours this drafter is drafting (decided after a few picks, from what it took and what flows). */
  main(): string[] {
    if (this.picks.length < 5) return [];
    return [...COLORS].sort((a, b) => this.score(b) - this.score(a)).slice(0, 2);
  }

  private score(col: string): number {
    return this.weight[col] + this.open[col] * 0.35;
  }

  rate(c: PoolCard): number {
    let s = baseRating(c) + this.r() * 0.3;
    const cols = [...c.colors];
    const main = this.main();
    if (main.length) {
      if (!cols.length) s += isLand(c) ? (c.identity.split('').every((x) => main.includes(x)) && c.identity ? 0.8 : 0) : 0.4;
      else if (cols.every((x) => main.includes(x))) s += 1.4;
      else if (cols.some((x) => main.includes(x))) s -= 0.7;
      else s -= this.picks.length > 15 ? 3 : 1.8;
      // curve and creature count late in the draft
      if (this.picks.length >= 18 && cols.every((x) => main.includes(x))) {
        const creatures = this.picks.filter((x) => isCreature(x) && [...x.colors].every((y) => main.includes(y))).length;
        if (isCreature(c) && creatures < 14) s += 0.4;
        const twos = this.picks.filter((x) => x.cmc === 2 && !isLand(x)).length;
        if (c.cmc === 2 && twos < 5) s += 0.3;
        if (c.cmc >= 5 && this.picks.filter((x) => x.cmc >= 5).length >= 5) s -= 0.6;
      }
    } else if (cols.length >= 2) s -= 0.4; // stay flexible early
    return s;
  }

  /** Read the pack before picking: strong cards of a colour still in it mean that colour is open. */
  notice(pack: PoolCard[], pickNo: number) {
    if (pickNo < 2) return;
    for (const c of pack) if (c.colors.length === 1 && baseRating(c) >= 2.6) this.open[c.colors] += (baseRating(c) - 2.4) * (pickNo / 10);
  }

  pick(pack: PoolCard[], pickNo = 0): number {
    this.notice(pack, pickNo);
    let best = 0;
    let bestScore = -Infinity;
    pack.forEach((c, i) => {
      const s = this.rate(c);
      if (s > bestScore) {
        bestScore = s;
        best = i;
      }
    });
    this.take(pack[best]);
    return best;
  }

  take(c: PoolCard) {
    this.picks.push(c);
    const w = baseRating(c) / 2;
    for (const x of c.colors) this.weight[x] = (this.weight[x] ?? 0) + w;
  }

  toJSON() {
    return { weight: this.weight, open: this.open, picks: this.picks.map((p) => p.key) };
  }
}

export interface BuiltDeck {
  /** the cards played, from the pool (keys, one per copy) */
  main: string[];
  /** basic land name → count */
  basics: Record<string, number>;
  colors: string[];
}

const BASIC_OF: Record<string, string> = { W: 'Plains', U: 'Island', B: 'Swamp', R: 'Mountain', G: 'Forest' };

/**
 * A 40-card deck from a pool: the best two colours (by the quality of their playables), 23 spells (about 15 creatures,
 * a sensible curve), the pool's dual lands in those colours, and basics split by mana symbols to 17 lands.
 * `playable` keeps cards the rules engine cannot play out of the deck (AI decks play rules enforced games).
 */
export function buildDeck(pool: PoolCard[], playable: (c: PoolCard) => boolean = () => true): BuiltDeck {
  const usable = pool.filter((c) => !isBasic(c) && playable(c));
  const fits = (c: PoolCard, cols: string[]) => [...c.colors].every((x) => cols.includes(x));
  let best: { cols: string[]; score: number } = { cols: ['W', 'U'], score: -Infinity };
  for (let i = 0; i < 5; i++)
    for (let j = i + 1; j < 5; j++) {
      const cols = [COLORS[i], COLORS[j]];
      const top = usable
        .filter((c) => !isLand(c) && fits(c, cols))
        .map(baseRating)
        .sort((a, b) => b - a)
        .slice(0, 23);
      const score = top.reduce((n, x) => n + x, 0) + (top.length < 23 ? (top.length - 23) * 2 : 0);
      if (score > best.score) best = { cols, score };
    }
  const cols = best.cols;
  const spells = usable.filter((c) => !isLand(c) && fits(c, cols)).sort((a, b) => baseRating(b) - baseRating(a));
  const main: PoolCard[] = [];
  const creatures = spells.filter(isCreature);
  // about 15 creatures first, then the best of the rest; keep the expensive end of the curve in check
  for (const c of creatures) if (main.length < 15) main.push(c);
  for (const c of spells) {
    if (main.length >= 23) break;
    if (main.includes(c)) continue;
    if (c.cmc >= 6 && main.filter((x) => x.cmc >= 6).length >= 2) continue;
    main.push(c);
  }
  // still short (a thin pool): any playable cards, colourless first
  for (const c of usable.filter((x) => !isLand(x) && !main.includes(x)).sort((a, b) => a.colors.length - b.colors.length || baseRating(b) - baseRating(a))) {
    if (main.length >= 23) break;
    main.push(c);
  }
  const duals = usable.filter((c) => isLand(c) && c.identity.length >= 1 && [...c.identity].every((x) => cols.includes(x))).slice(0, 3);
  main.push(...duals);
  const pips: Record<string, number> = { [cols[0]]: 0, [cols[1]]: 0 };
  for (const c of main) for (const m of c.manaCost.match(/\{([WUBRG])\}/g) ?? []) if (m[1] in pips) pips[m[1]]++;
  const nBasics = Math.max(0, 40 - main.length);
  const total = pips[cols[0]] + pips[cols[1]] || 1;
  const a = Math.round((nBasics * pips[cols[0]]) / total);
  const basics: Record<string, number> = {};
  if (a) basics[BASIC_OF[cols[0]]] = a;
  if (nBasics - a) basics[BASIC_OF[cols[1]]] = nBasics - a;
  return { main: main.map((c) => c.key), basics, colors: cols };
}

/** Basic lands for a limited deck: the five colours by mana symbols among the chosen cards (17 lands with the nonbasics). */
export function suggestBasics(cards: PoolCard[], lands = 17): Record<string, number> {
  const pips: Record<string, number> = {};
  for (const c of cards) for (const m of c.manaCost.match(/\{([WUBRG])\}/g) ?? []) pips[m[1]] = (pips[m[1]] ?? 0) + 1;
  const n = Math.max(0, lands - cards.filter(isLand).length);
  const total = Object.values(pips).reduce((x, y) => x + y, 0);
  const out: Record<string, number> = {};
  if (!total) return n ? { Plains: Math.ceil(n / 2), Island: Math.floor(n / 2) } : {};
  let given = 0;
  const cols = Object.keys(pips).sort((a, b) => pips[b] - pips[a]);
  cols.forEach((c, i) => {
    const k = i === cols.length - 1 ? n - given : Math.round((n * pips[c]) / total);
    if (k > 0) out[BASIC_OF[c]] = k;
    given += k;
  });
  return out;
}

// ---------------------------------------------------------------------------------------------------- the pod

export interface Seat {
  userId: number | null;
  name: string;
  pool: PoolCard[];
  /** the pack in front of the seat right now */
  pack: PoolCard[];
  ai?: AiDrafter;
  /** a pick for the current round is made (waiting for the others) */
  picked: boolean;
  /** the deck built from the pool: a saved deck (humans) or the AI's deck */
  deckId: number | null;
  built: BuiltDeck | null;
}

export type Phase = 'waiting' | 'drafting' | 'building';

/** Seconds for each pick of a 15-card pack (official paper draft timing); a 14-card pack skips the first. */
const PICK_SECONDS = [40, 40, 35, 30, 25, 25, 20, 20, 15, 10, 10, 5, 5, 5, 5];

export class Draft {
  seats: Seat[] = [];
  phase: Phase = 'waiting';
  round = 0; // pack number 0..2
  pickNo = 0; // pick within the pack
  /** when the current pick runs out (null: no timer, or not drafting) */
  deadline: number | null = null;
  private packs: PoolCard[][][] = []; // [round][seat] boosters
  private r: Rng;
  private rf: () => number;

  constructor(
    public sets: SetPool[],
    public mode: 'draft' | 'sealed',
    seed: number,
    public size = 8,
    /** a pick timer (seconds scale with the pack size); off: wait as long as it takes */
    public timed = false,
  ) {
    this.r = new Rng(seed >>> 0);
    this.rf = () => this.r.next();
  }

  addHuman(userId: number, name: string) {
    if (this.phase !== 'waiting') throw new Error('The draft has already started');
    if (this.seats.length >= this.size) throw new Error('The pod is full');
    if (this.seats.some((s) => s.userId === userId)) return;
    this.seats.push({ userId, name, pool: [], pack: [], picked: false, deckId: null, built: null });
  }

  addAi(name: string) {
    if (this.phase !== 'waiting') throw new Error('The draft has already started');
    this.seats.push({ userId: null, name, pool: [], pack: [], ai: new AiDrafter(this.rf), picked: false, deckId: null, built: null });
  }

  /** Start. Without explicit AI seats a draft fills up to `size` with AI drafters. */
  start(fill = true) {
    if (this.phase !== 'waiting') return;
    if (fill && this.mode === 'draft' && !this.seats.some((s) => s.ai)) {
      const humans = this.seats.length;
      for (let i = humans; i < this.size; i++) this.addAi(`Bot ${i - humans + 1}`);
    }
    if (this.mode === 'sealed') {
      // six boosters per player, straight into the pool
      for (const s of this.seats) for (let k = 0; k < 6; k++) s.pool.push(...makeBooster(this.sets[k % this.sets.length], this.rf));
      this.toBuilding();
      return;
    }
    this.packs = [0, 1, 2].map((rd) => this.seats.map(() => makeBooster(this.sets[rd % this.sets.length], this.rf)));
    this.phase = 'drafting';
    this.openRound();
  }

  private toBuilding(playable?: (c: PoolCard) => boolean) {
    this.phase = 'building';
    this.deadline = null;
    for (const s of this.seats) if (s.ai) s.built = buildDeck(s.pool, playable);
  }

  /** The AI decks only use cards the rules engine can play (set before the draft ends). */
  playable: (c: PoolCard) => boolean = () => true;

  private openRound() {
    this.pickNo = 0;
    this.seats.forEach((s, i) => {
      s.pack = this.packs[this.round][i];
      s.picked = false;
    });
    this.aiPick();
    this.setDeadline();
  }

  private setDeadline() {
    if (!this.timed || this.phase !== 'drafting') return (this.deadline = null);
    const left = Math.max(...this.seats.map((s) => s.pack.length));
    const secs = PICK_SECONDS[Math.max(0, PICK_SECONDS.length - left)] ?? 5;
    this.deadline = Date.now() + secs * 1000;
  }

  /** Everything the bots do for this pick (they never wait). */
  private aiPick() {
    for (const s of this.seats) {
      if (s.ai && s.pack.length && !s.picked) {
        const i = s.ai.pick(s.pack, this.pickNo);
        s.pool.push(s.pack[i]);
        s.pack = s.pack.filter((_, k) => k !== i);
        s.picked = true;
      }
    }
  }

  /** A human takes a card from the pack in front of them. */
  pick(userId: number, key: string, seatNo?: number) {
    if (this.phase !== 'drafting') throw new Error('Not drafting');
    const s = seatNo != null ? this.seats[seatNo] : this.seats.find((x) => x.userId === userId && !x.picked) ?? this.seats.find((x) => x.userId === userId);
    if (!s || s.userId !== userId) throw new Error('You are not in this draft');
    if (s.picked) throw new Error('You already picked; waiting for the others');
    const i = s.pack.findIndex((c) => c.key === key);
    if (i < 0) throw new Error('That card is not in your pack');
    s.pool.push(s.pack[i]);
    s.pack = s.pack.filter((_, k) => k !== i);
    s.picked = true;
    if (this.seats.every((x) => x.picked)) this.advance();
  }

  /** The pick timer ran out: the best card (as an AI would rate it for that player's picks) is taken for whoever has not picked. */
  tick(now = Date.now()): boolean {
    if (this.phase !== 'drafting' || this.deadline == null || now < this.deadline) return false;
    for (const s of this.seats) {
      if (s.picked || !s.pack.length) continue;
      const shadow = new AiDrafter(this.rf);
      for (const c of s.pool) shadow.take(c);
      const i = shadow.pick(s.pack, this.pickNo);
      s.pool.push(s.pack[i]);
      s.pack = s.pack.filter((_, k) => k !== i);
      s.picked = true;
    }
    this.advance();
    return true;
  }

  /** All seats have picked: pass the packs (left in packs 1 and 3, right in pack 2) and let the bots pick. */
  private advance() {
    this.pickNo++;
    const n = this.seats.length;
    if (this.seats.every((s) => !s.pack.length)) {
      this.round++;
      if (this.round >= 3) {
        this.toBuilding(this.playable);
        return;
      }
      this.openRound();
      return;
    }
    const dir = this.round % 2 === 0 ? 1 : -1;
    const packs = this.seats.map((s) => s.pack);
    this.seats.forEach((s, i) => {
      s.pack = packs[(i - dir + n) % n];
      s.picked = false;
    });
    this.aiPick();
    // a pack of one card left is taken without asking
    for (const s of this.seats) {
      if (!s.picked && s.pack.length === 1) {
        s.pool.push(s.pack[0]);
        s.pack = [];
        s.picked = true;
      }
    }
    if (this.seats.every((x) => x.picked)) this.advance();
    else this.setDeadline();
  }

  /** Everything needed to restore the draft after a restart. */
  toJSON() {
    const cards = new Map<string, PoolCard>();
    const ref = (c: PoolCard) => (cards.set(c.key, c), c.key);
    const seats = this.seats.map((s) => ({ userId: s.userId, name: s.name, pool: s.pool.map(ref), pack: s.pack.map(ref), ai: s.ai ? (s.ai.picks.forEach(ref), s.ai.toJSON()) : null, picked: s.picked, deckId: s.deckId, built: s.built }));
    const packs = this.packs.map((rd) => rd.map((p) => p.map(ref)));
    return { v: 1, mode: this.mode, size: this.size, timed: this.timed, phase: this.phase, round: this.round, pickNo: this.pickNo, deadline: this.deadline, rng: this.r.state, sets: this.sets.map((s) => ({ code: s.code, name: s.name, released: s.released })), cards: [...cards.values()], seats, packs };
  }

  static fromJSON(j: ReturnType<Draft['toJSON']>): Draft {
    const d = new Draft(j.sets.map((s) => ({ ...s, cards: [], basics: [] })), j.mode, 0, j.size, j.timed);
    d.r.state = j.rng;
    const cards = new Map(j.cards.map((c) => [c.key, c]));
    const get = (k: string) => cards.get(k)!;
    d.phase = j.phase;
    d.round = j.round;
    d.pickNo = j.pickNo;
    // a restart while drafting gives the current pick a fresh timer
    d.deadline = j.deadline != null ? Math.max(j.deadline, Date.now() + 15_000) : null;
    d.packs = j.packs.map((rd) => rd.map((p) => p.map(get)));
    d.seats = j.seats.map((s) => {
      let ai: AiDrafter | undefined;
      if (s.ai) {
        ai = new AiDrafter(d.rf);
        ai.weight = s.ai.weight;
        ai.open = s.ai.open;
        ai.picks = s.ai.picks.map(get);
      }
      return { userId: s.userId, name: s.name, pool: s.pool.map(get), pack: s.pack.map(get), ai, picked: s.picked, deckId: s.deckId, built: s.built };
    });
    return d;
  }
}
