import { basePT, type CardDef, type CardView } from '@aetherhub/shared';

/** The power/toughness to draw on a permanent, or null when its picture already shows it. */
export function ptOf(c: CardView, def: CardDef | undefined): { txt: string; delta: number } | null {
  // no printed P/T to compare with: always show the current one, uncoloured (delta 0)
  if (def?.unprintedPT) return { txt: `${c.pt.p + c.ptEot.p}/${c.pt.t + c.ptEot.t}`, delta: 0 };
  const b = basePT(def, c.face);
  if (!b) return null;
  const counters = (c.counters['+1/+1'] || 0) - (c.counters['-1/-1'] || 0);
  // rules games: the engine's power/toughness (the def's) already includes counters, so don't add them again
  const net = !!def?.key.startsWith('f:');
  const d = net ? 0 : counters;
  const p = b[0] + d + c.pt.p + c.ptEot.p;
  const t = b[1] + d + c.pt.t + c.ptEot.t;
  const delta = p - b[0] + (t - b[1]);
  if (!delta) return null;
  return { txt: `${p}/${t}`, delta };
}
