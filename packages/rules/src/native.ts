import { existsSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

/** The native addon's classes (everything crosses as JSON strings; see index.ts for the typed wrapper). */
export interface NativeGame {
  advance(): Promise<string>;
  answer(seat: number, choiceJson: string): void;
  view(seat: number): string;
  record(): string;
  setStops(seat: number, mine: number, opp: number, fullControl: boolean): void;
  passUntil(seat: number, turn: number, step: number): void;
  explain(seat: number, kind: string, card: number | null, other: number | null): string;
  passUntilStack(seat: number, item: number): void;
  passUntilPlayer(seat: number, player: number, after?: number | null): void;
  cancelPass(seat: number): void;
  setManualPay(seat: number, on: boolean): void;
  edit(seat: number, opJson: string): void;
  endBreak(seat: number): void;
  manualOnce(seat: number): void;
  previewBlocks(blocksJson: string): string;
  passing(seat: number): string;
  concede(seat: number): void;
  undo(n: number): void;
  turn(): number;
}
export interface NativeEngine {
  cardCount(): number;
  hasCard(name: string): boolean;
  cardNames(): string[];
  tokenNames(): string[];
  newGame(setupJson: string): NativeGame;
  restoreGame(recordJson: string): NativeGame;
}
export interface NativeModule {
  Engine: { load(dir: string): NativeEngine };
}

const FILES = ['libmtg_node.so', 'libmtg_node.dylib', 'mtg_node.dll', 'mtg_node.node'];

/** Where the compiled addon lives: $RULES_NATIVE, or the cargo release build in the repository. */
export function nativePath(): string | null {
  const env = process.env.RULES_NATIVE;
  if (env && existsSync(env)) return env;
  let dir = dirname(fileURLToPath(import.meta.url));
  for (let i = 0; i < 8; i++) {
    for (const rel of ['engine/target/release', 'engine/target/debug', 'native']) {
      for (const f of FILES) {
        const p = join(dir, rel, f);
        if (existsSync(p)) return p;
      }
    }
    dir = dirname(dir);
  }
  return null;
}

let cached: NativeModule | null = null;
export function loadNative(): NativeModule {
  if (cached) return cached;
  const p = nativePath();
  if (!p) throw new Error('rules engine native module not found; build it with `cargo build --release -p mtg-node` in engine/ or set RULES_NATIVE');
  const m = { exports: {} as unknown } as NodeJS.Module;
  process.dlopen(m, p);
  cached = m.exports as NativeModule;
  return cached;
}
