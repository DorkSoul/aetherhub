import { ACCENT_DEFAULT, PLAYER_COLORS } from './theme';

export type PanelSlot = 'log' | 'players' | 'rules' | 'tabs' | 'spoiler';
export type HotkeyAction = 'tap' | 'draw' | 'untap' | 'shuffle' | 'undo' | 'pass';

export const HOTKEY_LABELS: Record<HotkeyAction, string> = {
  tap: 'Tap / untap',
  draw: 'Draw a card',
  untap: 'Untap all',
  shuffle: 'Shuffle library',
  undo: 'Undo',
  pass: 'Pass turn',
};

export const DEFAULT_HOTKEYS: Record<HotkeyAction, string> = { tap: 'T', draw: 'D', untap: 'U', shuffle: 'S', undo: 'Ctrl+Z', pass: 'Space' };

export interface AlertPrefs {
  turn: boolean;
  requests: boolean;
  pings: boolean;
  chat: boolean;
  actions: boolean;
}

export interface UserPrefs {
  accent: string;
  theme: 'dark' | 'light';
  playerColor: string;
  tintPanels: boolean;
  /** How battlefields and hand strips are drawn: the player-colour wash, green baize, or a coloured playmat per player. */
  tableTheme: 'classic' | 'felt' | 'playmats';
  /** grid: everyone at once · tab: one opponent at a time · free: the grid with a pan/zoom camera. */
  layout: 'grid' | 'tab' | 'free';
  panel: 'minimal' | 'detailed';
  sections: 2 | 3 | 4;
  slots: PanelSlot[];
  panelW: number;
  preview: boolean;
  previewSize: number;
  /** Hover preview in the Free view (off by default there). */
  freePreview: boolean;
  barDice: boolean;
  barDay: boolean;
  hotkeys: Record<HotkeyAction, string>;
  volume: number;
  alerts: AlertPrefs;
  reduceMotion: boolean;
  /** Turn the opponents' battlefield cards 180° so they read the right way up for you (the hover preview is always upright). */
  flipOpponents: boolean;
  autoUntap: boolean;
  /** Draw a card automatically when your turn starts (off by default). */
  autoDraw: boolean;
  playmat: number | null;
  deckView: 'visual' | 'stacks' | 'text' | 'table';
  deckZoom: number;
  /** Hover preview in the deck builder (separate from the table's). */
  deckPreview: boolean;
  deckPreviewSize: number;
  /** Deck builder: the right side panel (stats, deck settings) is open. */
  deckPanel: boolean;
  /** Deck builder card search: card images (visual spoiler) or a compact list. */
  deckSearchView: 'visual' | 'list';
  /** Rules game: show the other players' actions one at a time, at most one per `aiPaceSeconds` (off by default). */
  aiPaceOn: boolean;
  aiPaceSeconds: number;
  /** Rules game: tap lands automatically to pay for spells (off = always tap by hand). */
  autoTap: boolean;
  /** Rules game: keep every battlefield sorted into its rows as cards come and go (off by default). */
  autoSort: boolean;
}

export const DEFAULT_PREFS: UserPrefs = {
  accent: ACCENT_DEFAULT,
  theme: 'dark',
  playerColor: PLAYER_COLORS[0],
  tintPanels: true,
  tableTheme: 'classic',
  layout: 'grid',
  panel: 'minimal',
  sections: 3,
  slots: ['tabs', 'log', 'spoiler'],
  panelW: 340,
  preview: true,
  previewSize: 100,
  freePreview: false,
  barDice: false,
  barDay: false,
  hotkeys: DEFAULT_HOTKEYS,
  volume: 70,
  alerts: { turn: true, requests: true, pings: true, chat: true, actions: false },
  reduceMotion: false,
  flipOpponents: false,
  autoUntap: true,
  autoDraw: false,
  playmat: null,
  deckView: 'visual',
  deckZoom: 1,
  deckPreview: true,
  deckPreviewSize: 100,
  deckPanel: true,
  deckSearchView: 'visual',
  aiPaceOn: false,
  aiPaceSeconds: 0.1,
  autoTap: true,
  autoSort: false,
};

export function defaultSlots(n: 2 | 3 | 4): PanelSlot[] {
  return n === 2 ? ['tabs', 'spoiler'] : n === 3 ? ['tabs', 'log', 'spoiler'] : ['log', 'players', 'rules', 'spoiler'];
}

export function mergePrefs(p: Partial<UserPrefs> | null | undefined): UserPrefs {
  const o = { ...DEFAULT_PREFS, ...(p || {}) };
  o.hotkeys = { ...DEFAULT_HOTKEYS, ...(p?.hotkeys || {}) };
  o.alerts = { ...DEFAULT_PREFS.alerts, ...(p?.alerts || {}) };
  return o;
}
