import { describe, expect, it } from 'vitest';
import { addLogLine, recentLog } from './serverlog';

describe('the in-memory server log', () => {
  it('keeps the logger lines readable, with their level and time, and lists the ones after a given line', () => {
    const before = recentLog().at(-1)?.seq ?? 0;
    addLogLine(JSON.stringify({ level: 30, time: 1000, msg: 'Rules game started: alice, Bot (AI)' }));
    addLogLine(JSON.stringify({ level: 50, time: 2000, msg: 'request failed', err: { message: 'boom' }, req: { method: 'GET', url: '/api/x' } }));
    addLogLine('not json');
    const lines = recentLog(before);
    expect(lines.map((l) => [l.level, l.msg])).toEqual([
      ['info', 'Rules game started: alice, Bot (AI)'],
      ['error', 'request failed: boom (GET /api/x)'],
      ['info', 'not json'],
    ]);
    expect(lines[0].at).toBe(1000);
    expect(recentLog(lines[1].seq).map((l) => l.msg)).toEqual(['not json']);
  });

  it('keeps only the most recent 1,000 lines', () => {
    for (let i = 0; i < 1100; i++) addLogLine(JSON.stringify({ level: 30, time: i, msg: `line ${i}` }));
    const all = recentLog();
    expect(all).toHaveLength(1000);
    expect(all.at(-1)!.msg).toBe('line 1099');
  });
});
