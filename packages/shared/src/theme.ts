// App accent colours and player palette.

export interface Accent {
  id: string;
  name: string;
  group: 'mtg' | 'more';
  hex: string;
  tx?: string;
  mu?: string;
  hi?: string;
  bd?: string;
  ink?: string;
}

export const ACCENTS: Accent[] = [
  { id: 'white', name: 'White', group: 'mtg', hex: '#e6d9b3' },
  { id: 'blue', name: 'Blue', group: 'mtg', hex: '#4a95e0' },
  { id: 'black', name: 'Black', group: 'mtg', hex: '#3d3634', tx: '#d8cfca', mu: '#9d9490', hi: '#5a514d', bd: '#4d4542', ink: '#ffffff' },
  { id: 'red', name: 'Red', group: 'mtg', hex: '#e3573f' },
  { id: 'green', name: 'Green', group: 'mtg', hex: '#4fae5c' },
  { id: 'gold', name: 'Gold', group: 'more', hex: '#e0a84f' },
  { id: 'silver', name: 'Silver', group: 'more', hex: '#a7b1bf' },
  { id: 'teal', name: 'Teal', group: 'more', hex: '#35b3a6' },
  { id: 'orange', name: 'Orange', group: 'more', hex: '#ec8a33' },
  { id: 'pink', name: 'Pink', group: 'more', hex: '#e46f9a' },
];
export const ACCENT_DEFAULT = 'blue';

export function hexRgb(h: string): [number, number, number] {
  const n = parseInt(h.slice(1), 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

export function mixHex(a: string, b: string, t: number): string {
  const x = hexRgb(a);
  const y = hexRgb(b);
  return '#' + x.map((v, i) => ('0' + Math.round(v + (y[i] - v) * t).toString(16)).slice(-2)).join('');
}

export function hexA(h: string, a: number): string {
  const [r, g, b] = hexRgb(h);
  return `rgba(${r},${g},${b},${a})`;
}

function lum(h: string): number {
  return hexRgb(h)
    .map((v) => {
      v /= 255;
      return v <= 0.03928 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4);
    })
    .reduce((s, v, i) => s + v * [0.2126, 0.7152, 0.0722][i], 0);
}

/** CSS custom properties for an accent, relative to a page background. */
export function accentVars(id: string, bg = '#101114'): Record<string, string> {
  const a = ACCENTS.find((x) => x.id === id) || ACCENTS.find((x) => x.id === ACCENT_DEFAULT)!;
  const L = lum(a.hex);
  const ink = (L + 0.05) / (0.008 + 0.05) >= 1.05 / (L + 0.05) ? '#15130f' : '#ffffff';
  return {
    '--acc': a.hex,
    '--acc-hi': a.hi || mixHex(a.hex, '#ffffff', 0.18),
    '--acc-tx': a.tx || mixHex(a.hex, '#ffffff', 0.45),
    '--acc-mu': a.mu || mixHex(a.hex, '#8f939c', 0.5),
    '--acc-ink': a.ink || ink,
    '--acc-bg': mixHex(bg, a.hex, 0.14),
    '--acc-bd': a.bd || mixHex(bg, a.hex, 0.4),
    '--acc-rgb': hexRgb(a.tx ? a.hi! : a.hex).join(','),
  };
}

export const PLAYER_COLORS = ['#e0a84f', '#4fb3a9', '#9b8cf0', '#e0708f', '#a3c85a', '#56c1e8', '#e88d3a', '#b8bcc6'];

export const PIP_COLORS: Record<string, [string, string]> = {
  W: ['#f3ecd6', '#2b261b'],
  U: ['#4a86c5', '#ffffff'],
  B: ['#3a3431', '#f1ece4'],
  R: ['#d0573d', '#ffffff'],
  G: ['#4c8a4f', '#ffffff'],
};

export const FRAME_COLORS: Record<string, { bg: string; bar: string; art: string; foot: string }> = {
  W: { bg: '#d6cbac', bar: '#efe7d2', art: '#8c7f5f', foot: '#3a3226' },
  U: { bg: '#3d6b9c', bar: '#d3e2f0', art: '#2a4a6e', foot: 'rgba(255,255,255,.85)' },
  B: { bg: '#3f3935', bar: '#dcd4ca', art: '#29231f', foot: 'rgba(255,255,255,.8)' },
  R: { bg: '#b1523b', bar: '#f0d7cb', art: '#7a3222', foot: 'rgba(255,255,255,.85)' },
  G: { bg: '#467a4a', bar: '#d6e5cf', art: '#2f5533', foot: 'rgba(255,255,255,.85)' },
  M: { bg: '#c19a47', bar: '#f3e5c2', art: '#7d6224', foot: '#2e2410' },
  C: { bg: '#8d939c', bar: '#e4e7eb', art: '#5b6068', foot: '#1e2126' },
  L: { bg: '#86705a', bar: '#ece2d5', art: '#574636', foot: 'rgba(255,255,255,.85)' },
};

/** Frame colour key for a card: single colour, M for multicolour, L for land, C for colourless. */
export function frameKey(colors: string[], typeLine: string): string {
  if (colors.length > 1) return 'M';
  if (colors.length === 1) return colors[0];
  if (/\bLand\b/.test(typeLine)) return 'L';
  return 'C';
}
