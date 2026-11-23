import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

/** Every dropdown in the app uses ui/Select.tsx, never the browser's <select> (which looks generic and can't be themed). */
describe('dropdowns', () => {
  it('no native <select> in the client', () => {
    const root = join(import.meta.dirname, '../../client/src');
    const bad: string[] = [];
    const walk = (dir: string) => {
      for (const f of readdirSync(dir)) {
        const p = join(dir, f);
        if (statSync(p).isDirectory()) walk(p);
        else if (p.endsWith('.tsx') && !p.endsWith('Select.tsx') && /<select[\s>]/.test(readFileSync(p, 'utf8'))) bad.push(p);
      }
    };
    walk(root);
    expect(bad).toEqual([]);
  });
});
