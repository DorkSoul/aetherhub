declare global {
  interface Window {
    __AETHER_BASE__?: string;
  }
}

/** Base path the app is served under ("/" or "/sub/"). */
export const BASE = window.__AETHER_BASE__ || '/';

export function url(p: string): string {
  return BASE + p.replace(/^\//, '');
}

export function imgUrl(ref: string | undefined | null, size: 'normal' | 'large' = 'normal'): string | null {
  if (!ref) return null;
  if (ref.startsWith('a/')) return url(`img/${ref}`);
  return url(`img/${ref}/${size}`);
}

export const CARD_BACK = url('card-back.webp');
