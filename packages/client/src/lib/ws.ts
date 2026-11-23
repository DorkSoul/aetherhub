import type { ClientMsg, ServerMsg } from '@aetherhub/shared';
import { BASE } from './base';

type Listener = (m: ServerMsg) => void;

/** One WebSocket per tab, reconnecting with backoff. */
class Socket {
  private ws: WebSocket | null = null;
  private listeners = new Set<Listener>();
  private queue: ClientMsg[] = [];
  private retry = 0;
  private closedByUs = false;
  /** the sandbox game this tab watches (re-sent after a reconnect) */
  private watching: string | null = null;
  connected = false;
  onState: ((c: boolean) => void) | null = null;

  connect() {
    this.closedByUs = false;
    if (this.ws && this.ws.readyState <= 1) return;
    const proto = location.protocol === 'https:' ? 'wss' : 'ws';
    const ws = new WebSocket(`${proto}://${location.host}${BASE}ws`);
    this.ws = ws;
    ws.onopen = () => {
      this.retry = 0;
      this.connected = true;
      this.onState?.(true);
      ws.send(JSON.stringify({ t: 'hello' }));
      if (this.watching) ws.send(JSON.stringify({ t: 'watch', game: this.watching }));
      for (const m of this.queue.splice(0)) ws.send(JSON.stringify(m));
    };
    ws.onmessage = (e) => {
      let m: ServerMsg;
      try {
        m = JSON.parse(e.data);
      } catch {
        return;
      }
      for (const l of this.listeners) l(m);
    };
    ws.onclose = () => {
      this.connected = false;
      this.onState?.(false);
      if (this.closedByUs) return;
      const wait = Math.min(10000, 500 * 2 ** this.retry++);
      setTimeout(() => this.connect(), wait);
    };
  }

  close() {
    this.closedByUs = true;
    this.ws?.close();
  }

  send(m: ClientMsg) {
    if (m.t === 'watch') this.watching = m.game;
    if (m.t === 'unwatch') this.watching = null;
    if (this.ws && this.ws.readyState === 1) this.ws.send(JSON.stringify(m));
    else if (m.t !== 'watch' && m.t !== 'unwatch') this.queue.push(m);
  }

  on(l: Listener): () => void {
    this.listeners.add(l);
    return () => this.listeners.delete(l);
  }
}

export const socket = new Socket();
