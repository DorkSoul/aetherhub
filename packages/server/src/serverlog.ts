/** The server's most recent log lines, kept in memory for the admin Server page. Every line still goes to stdout. */
export interface LogLine {
  seq: number;
  at: number;
  level: string;
  msg: string;
}

const MAX_LINES = 1000;
const LEVELS: Record<number, string> = { 10: 'trace', 20: 'debug', 30: 'info', 40: 'warn', 50: 'error', 60: 'fatal' };
const lines: LogLine[] = [];
let seq = 0;

/** Keeps one line of the logger's JSON output (a line that isn't JSON is kept as it is). */
export function addLogLine(raw: string): void {
  let line: LogLine;
  try {
    const o = JSON.parse(raw) as { level?: number; time?: number; msg?: string; err?: { message?: string }; req?: { method?: string; url?: string } };
    const req = o.req?.url ? ` (${o.req.method ?? ''} ${o.req.url})` : '';
    const err = o.err?.message && o.err.message !== o.msg ? `: ${o.err.message}` : '';
    line = { seq: ++seq, at: o.time ?? Date.now(), level: LEVELS[o.level ?? 30] ?? String(o.level), msg: `${o.msg ?? ''}${err}${req}` };
  } catch {
    line = { seq: ++seq, at: Date.now(), level: 'info', msg: raw };
  }
  lines.push(line);
  if (lines.length > MAX_LINES) lines.splice(0, lines.length - MAX_LINES);
}

/** The kept lines after `after` (a line's `seq`), oldest first. */
export function recentLog(after = 0): LogLine[] {
  return lines.filter((l) => l.seq > after);
}

/** The logger's destination: stdout, and the in-memory lines above. */
export const logStream = {
  write(chunk: string): void {
    process.stdout.write(chunk);
    for (const raw of chunk.split('\n')) if (raw.trim()) addLogLine(raw);
  },
};
