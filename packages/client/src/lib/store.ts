import { create } from 'zustand';
import type { LobbyView, Me, ServerStatus, UserPrefs } from '@aetherhub/shared';
import { accentVars, mergePrefs } from '@aetherhub/shared';
import { api } from './api';
import { setVolume } from './sound';

export interface Toast {
  id: number;
  text: string;
  color?: string;
}

interface AppState {
  me: Me | null;
  status: ServerStatus | null;
  lobby: LobbyView | null;
  toasts: Toast[];
  wsConnected: boolean;
  setMe: (m: Me | null) => void;
  setPrefs: (p: Partial<UserPrefs>) => void;
  toast: (text: string, color?: string) => void;
}

let toastId = 1;
let saveTimer: number | null = null;
let pending: Partial<UserPrefs> = {};

export const useApp = create<AppState>((set, get) => ({
  me: null,
  status: null,
  lobby: null,
  toasts: [],
  wsConnected: false,
  setMe: (me) => {
    set({ me });
    if (me) applyPrefs(me.prefs);
  },
  setPrefs: (p) => {
    const me = get().me;
    if (!me) return;
    const prefs = mergePrefs({ ...me.prefs, ...p });
    set({ me: { ...me, prefs } });
    applyPrefs(prefs);
    pending = { ...pending, ...p };
    if (saveTimer) clearTimeout(saveTimer);
    saveTimer = window.setTimeout(() => {
      const body = pending;
      pending = {};
      api.put('api/me/prefs', body).catch(() => get().toast('Could not save settings'));
    }, 400);
  },
  toast: (text, color) => {
    const id = toastId++;
    set((s) => ({ toasts: [...s.toasts, { id, text, color }].slice(-5) }));
    setTimeout(() => set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })), 3200);
  },
}));

export function applyPrefs(p: UserPrefs) {
  const root = document.documentElement;
  root.dataset.theme = p.theme;
  const vars = accentVars(p.accent, p.theme === 'light' ? '#f5f4ef' : '#101114');
  for (const [k, v] of Object.entries(vars)) root.style.setProperty(k, v);
  root.classList.toggle('reduce-motion', p.reduceMotion);
  setVolume(p.volume);
  try {
    localStorage.setItem('aetherhub.accent', p.accent);
    localStorage.setItem('aetherhub.theme', p.theme);
  } catch {
    /* storage unavailable */
  }
}

/** Paint the cached accent before we know who is signed in. */
export function applyCachedAccent() {
  try {
    const a = localStorage.getItem('aetherhub.accent') || 'blue';
    const t = localStorage.getItem('aetherhub.theme') === 'light' ? 'light' : 'dark';
    document.documentElement.dataset.theme = t;
    for (const [k, v] of Object.entries(accentVars(a, t === 'light' ? '#f5f4ef' : '#101114'))) document.documentElement.style.setProperty(k, v);
  } catch {
    /* ignore */
  }
}

export function errMsg(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}
