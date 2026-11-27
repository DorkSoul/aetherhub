import { describe, expect, it } from 'vitest';
import { entryOf } from './serverlog';

describe('the framework logger lines kept in the activity log', () => {
  it('keeps warnings and errors with their stack, and leaves out info lines, request lines and errors already logged', () => {
    expect(entryOf(JSON.stringify({ level: 30, msg: 'listening' }))).toBeNull();
    expect(entryOf(JSON.stringify({ level: 50, msg: 'request failed', req: { method: 'GET', url: '/api/x' } }))).toBeNull();
    expect(entryOf(JSON.stringify({ level: 50, msg: 'boom', logged: true }))).toBeNull();
    expect(entryOf('not json')).toBeNull();
    expect(entryOf(JSON.stringify({ level: 40, msg: 'slow' }))).toEqual({ msg: 'slow', cat: 'server', level: 'warn', detail: null });
    expect(entryOf(JSON.stringify({ level: 60, msg: 'Uncaught exception', err: { message: 'x is undefined', stack: 'Error: x is undefined\n    at f' } }))).toEqual({
      msg: 'Uncaught exception: x is undefined',
      cat: 'server',
      level: 'error',
      detail: 'Error: x is undefined\n    at f',
    });
  });
});
