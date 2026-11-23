import crypto from 'node:crypto';
import type { ApplyCtx, CardDef, GameState, Intent, LogDraft, LogEntry, Notice, Resolved, Rng } from '@aetherhub/shared';
import { applyIntent, IntentError, resolveIntent } from '@aetherhub/shared';
import type { AppDb } from '../db/app';
import { all, get, run, tx } from '../db/sqlite';

export const cryptoRng: Rng = { int: (n) => (n <= 1 ? 0 : crypto.randomInt(n)) };

export interface EventRec {
  seq: number;
  actor: number;
  actorUser: number | null;
  intent: Intent;
  resolved: Resolved;
  label: string;
  at: number;
}

export interface GameMeta {
  hostUserId: number;
  matchStartedAt: number;
  /** Full, unredacted log lines across all games of the match. */
  matchLog: { t: number; who: string; color: string; kind: string; text: string }[];
  concededTurn: Record<number, number>;
}

const SNAPSHOT_EVERY = 50;
const LIFE_MERGE_MS = 6000;

export const INTENT_LABEL: Partial<Record<Intent['type'], string>> = {
  move: 'Move',
  tap: 'Tap',
  untapAll: 'Untap all',
  draw: 'Draw',
  shuffle: 'Shuffle',
  life: 'Life change',
  passTurn: 'Pass turn',
  createToken: 'Create token',
  counter: 'Counter',
  mill: 'Mill',
  lookResolve: 'Library order',
  mulligan: 'Mulligan',
  keep: 'Keep',
};

export function labelFor(i: Intent): string {
  return INTENT_LABEL[i.type] || i.type.replace(/([A-Z])/g, ' $1').replace(/^./, (c) => c.toUpperCase());
}

export class LiveGame {
  events: EventRec[] = [];
  log: LogEntry[] = [];
  private nextLogId = 1;

  constructor(
    private app: AppDb,
    public id: string,
    public state: GameState,
    public seq: number,
    public meta: GameMeta,
  ) {}

  static create(app: AppDb, state: GameState, meta: GameMeta): LiveGame {
    const g = new LiveGame(app, state.id, state, 0, meta);
    const now = Date.now();
    tx(app.db, () => {
      run(app.db, 'DELETE FROM games WHERE id = ?', state.id);
      run(
        app.db,
        'INSERT INTO games (id, status, name, host_id, settings, meta, seq, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, 0, ?, ?)',
        state.id,
        'active',
        state.settings.name,
        meta.hostUserId,
        JSON.stringify(state.settings),
        JSON.stringify(meta),
        now,
        now,
      );
      run(app.db, 'INSERT INTO game_snapshots (game_id, seq, state) VALUES (?, 0, ?)', state.id, JSON.stringify(state));
    });
    return g;
  }

  /** Rebuilds from the newest snapshot plus the events after it. */
  static load(app: AppDb, id: string): LiveGame | null {
    const row = get<{ meta: string; seq: number }>(app.db, 'SELECT meta, seq FROM games WHERE id = ?', id);
    if (!row) return null;
    const meta = JSON.parse(row.meta) as GameMeta;
    const first = get<{ state: string }>(app.db, 'SELECT state FROM game_snapshots WHERE game_id = ? AND seq = 0', id);
    if (!first) return null;
    const g = new LiveGame(app, id, JSON.parse(first.state), 0, meta);
    g.events = all<{ seq: number; actor: number; actor_user: number | null; intent: string; resolved: string; label: string; at: number }>(
      app.db,
      'SELECT * FROM game_events WHERE game_id = ? ORDER BY seq',
      id,
    ).map((e) => ({ seq: e.seq, actor: e.actor, actorUser: e.actor_user, intent: JSON.parse(e.intent), resolved: JSON.parse(e.resolved), label: e.label, at: e.at }));
    const snap = get<{ seq: number; state: string }>(app.db, 'SELECT seq, state FROM game_snapshots WHERE game_id = ? ORDER BY seq DESC LIMIT 1', id)!;
    g.state = JSON.parse(snap.state);
    g.seq = snap.seq;
    for (const e of g.events.filter((x) => x.seq > snap.seq)) g.replay(e);
    g.log = all<{ entry: string }>(app.db, 'SELECT entry FROM game_log WHERE game_id = ? ORDER BY id', id).map((r) => JSON.parse(r.entry));
    g.nextLogId = (g.log[g.log.length - 1]?.id ?? 0) + 1;
    return g;
  }

  private replay(e: EventRec) {
    const next = structuredClone(this.state);
    try {
      applyIntent(next, { actor: e.actor, actorName: this.state.seats[e.actor]?.name ?? 'Someone', now: e.at }, e.intent, e.resolved);
      this.state = next;
    } catch {
      /* an event that fails on replay is skipped */
    }
    this.seq = e.seq;
  }

  /** Applies an intent. Throws IntentError when it isn't allowed. */
  apply(actor: number, actorUser: number | null, actorName: string, intent: Intent, extraDefs?: (keys: string[]) => Record<string, CardDef>) {
    const ctx: ApplyCtx = { actor, actorName, now: Date.now() };
    const resolved = resolveIntent(this.state, ctx, intent, cryptoRng);
    if (intent.type === 'createToken' && intent.def && !this.state.defs[intent.def]) {
      if (!/^[pc]:/.test(intent.def)) throw new IntentError('Unknown token');
      resolved.defs = extraDefs?.([intent.def]) ?? {};
      if (!resolved.defs[intent.def]) throw new IntentError('Unknown token');
    }
    const next = structuredClone(this.state);
    const res = applyIntent(next, ctx, intent, resolved);
    this.state = next;
    this.seq++;
    const ev: EventRec = { seq: this.seq, actor, actorUser, intent, resolved, label: labelFor(intent), at: ctx.now };
    this.events.push(ev);
    tx(this.app.db, () => {
      run(
        this.app.db,
        'INSERT INTO game_events (game_id, seq, actor, actor_user, intent, resolved, label, at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)',
        this.id,
        ev.seq,
        actor,
        actorUser,
        JSON.stringify(intent),
        JSON.stringify(resolved),
        ev.label,
        ev.at,
      );
      if (this.seq % SNAPSHOT_EVERY === 0) run(this.app.db, 'INSERT OR REPLACE INTO game_snapshots (game_id, seq, state) VALUES (?, ?, ?)', this.id, this.seq, JSON.stringify(this.state));
      run(this.app.db, 'UPDATE games SET seq = ?, updated_at = ? WHERE id = ?', this.seq, ev.at, this.id);
    });
    const { added, updated } = this.addLogs(actor, actorName, res.logs, intent.type === 'chat' && actor < 0);
    return { added, updated, notices: res.notices as Notice[] };
  }

  /** Mutations that aren't player actions (connection status, seat hand-over). Saved as a snapshot, not an event. */
  system(mut: (s: GameState) => void) {
    const next = structuredClone(this.state);
    mut(next);
    this.state = next;
    run(this.app.db, 'INSERT OR REPLACE INTO game_snapshots (game_id, seq, state) VALUES (?, ?, ?)', this.id, this.seq, JSON.stringify(this.state));
  }

  addLogs(actor: number, actorName: string, drafts: LogDraft[], spectatorChat = false): { added: LogEntry[]; updated: LogEntry[] } {
    const added: LogEntry[] = [];
    const updated: LogEntry[] = [];
    const now = Date.now();
    for (const d of drafts) {
      const last = this.log[this.log.length - 1];
      if (d.kind === 'life' && d.life && last && last.kind === 'life' && last.life && last.actor === actor && last.life.target === d.life.target && now - last.at < LIFE_MERGE_MS) {
        last.life = { target: d.life.target, from: last.life.from, to: d.life.to };
        last.text = d.text.replace(/\d+ → (-?\d+)$/, `${last.life.from} → $1`);
        last.at = now;
        run(this.app.db, 'UPDATE game_log SET entry = ? WHERE game_id = ? AND id = ?', JSON.stringify(last), this.id, last.id);
        this.meta.matchLog[this.meta.matchLog.length - 1] = { ...this.meta.matchLog[this.meta.matchLog.length - 1], text: last.text };
        updated.push(last);
        continue;
      }
      const e: LogEntry = { ...d, id: this.nextLogId++, at: now, actor: actor >= 0 ? actor : null, actorName };
      if (spectatorChat && d.kind === 'chat') e.channel = 'spectators';
      this.log.push(e);
      added.push(e);
      run(this.app.db, 'INSERT INTO game_log (game_id, id, entry) VALUES (?, ?, ?)', this.id, e.id, JSON.stringify(e));
      const seatColor = actor >= 0 ? this.state.seats[actor]?.color : '#8f939c';
      this.meta.matchLog.push({ t: now - this.meta.matchStartedAt, who: actor >= 0 || d.kind === 'chat' ? actorName : 'Server', color: seatColor || '#8f939c', kind: d.kind, text: d.text });
    }
    if (added.length || updated.length) this.saveMeta();
    return { added, updated };
  }

  saveMeta() {
    run(this.app.db, 'UPDATE games SET meta = ? WHERE id = ?', JSON.stringify(this.meta), this.id);
  }

  /** Index of the most recent undoable event by `seat`, or -1. */
  lastOwnEvent(seat: number): number {
    for (let i = this.events.length - 1; i >= 0; i--) {
      const e = this.events[i];
      if (e.actor === seat && e.intent.type !== 'chat') return i;
    }
    return -1;
  }

  undoableCount(seat: number): number {
    return this.events.filter((e) => e.actor === seat && e.intent.type !== 'chat').length;
  }

  /** Rewinds the whole game to just before event `seq`. Chat events after it are kept. */
  rewind(toSeq: number) {
    const keep = this.events.filter((e) => e.seq < toSeq);
    const replayChat = this.events.filter((e) => e.seq > toSeq && e.intent.type === 'chat');
    const snap = get<{ seq: number; state: string }>(this.app.db, 'SELECT seq, state FROM game_snapshots WHERE game_id = ? AND seq < ? ORDER BY seq DESC LIMIT 1', this.id, toSeq);
    if (!snap) throw new IntentError('Nothing to rewind to');
    // Connection flags reflect reality, not history.
    const connected = this.state.seats.map((s) => s.connected);
    this.state = JSON.parse(snap.state);
    this.seq = snap.seq;
    for (const e of keep.filter((x) => x.seq > snap.seq)) this.replay(e);
    this.state.seats.forEach((s, i) => (s.connected = connected[i] ?? s.connected));
    this.events = keep;
    tx(this.app.db, () => {
      run(this.app.db, 'DELETE FROM game_events WHERE game_id = ? AND seq >= ?', this.id, toSeq);
      run(this.app.db, 'DELETE FROM game_snapshots WHERE game_id = ? AND seq >= ?', this.id, toSeq);
      run(this.app.db, 'INSERT OR REPLACE INTO game_snapshots (game_id, seq, state) VALUES (?, ?, ?)', this.id, this.seq, JSON.stringify(this.state));
      run(this.app.db, 'UPDATE games SET seq = ? WHERE id = ?', this.seq, this.id);
    });
    // Re-number chat events so they are not lost (they don't change the board).
    for (const c of replayChat) {
      this.seq++;
      const ev = { ...c, seq: this.seq };
      this.events.push(ev);
      run(this.app.db, 'INSERT INTO game_events (game_id, seq, actor, actor_user, intent, resolved, label, at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)', this.id, ev.seq, ev.actor, ev.actorUser, JSON.stringify(ev.intent), '{}', ev.label, ev.at);
    }
  }

  setStatus(status: 'active' | 'suspended' | 'finished', by?: string) {
    run(
      this.app.db,
      'UPDATE games SET status = ?, suspended_by = ?, suspended_at = ?, updated_at = ? WHERE id = ?',
      status,
      status === 'suspended' ? (by ?? null) : null,
      status === 'suspended' ? Date.now() : null,
      Date.now(),
      this.id,
    );
    if (status !== 'active') run(this.app.db, 'INSERT OR REPLACE INTO game_snapshots (game_id, seq, state) VALUES (?, ?, ?)', this.id, this.seq, JSON.stringify(this.state));
  }
}
