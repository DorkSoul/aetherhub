import type { LogEntry } from './activity';

/**
 * The web framework logger's destination: stdout, plus its warnings and errors into the activity log (through the
 * sink the app sets). Errors already logged for a request (marked `logged`) and per-request lines are left to the
 * activity log's request hooks.
 */
let sink: ((e: LogEntry) => void) | null = null;

export function setLogSink(fn: ((e: LogEntry) => void) | null): void {
  sink = fn;
}

/** The activity log entry for one line of the logger's JSON output, or null when it isn't kept. */
export function entryOf(raw: string): LogEntry | null {
  let o: { level?: number; msg?: string; err?: { message?: string; stack?: string }; req?: unknown; logged?: boolean };
  try {
    o = JSON.parse(raw);
  } catch {
    return null;
  }
  if ((o.level ?? 30) < 40 || o.req || o.logged) return null;
  const err = o.err?.message && o.err.message !== o.msg ? `: ${o.err.message}` : '';
  return { msg: `${o.msg ?? ''}${err}`, cat: 'server', level: (o.level ?? 40) >= 50 ? 'error' : 'warn', detail: o.err?.stack ?? null };
}

export const logStream = {
  write(chunk: string): void {
    process.stdout.write(chunk);
    for (const raw of chunk.split('\n')) {
      const e = raw.trim() ? entryOf(raw) : null;
      if (e) sink?.(e);
    }
  },
};
