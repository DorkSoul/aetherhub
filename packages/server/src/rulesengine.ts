import fs from 'node:fs';
import { nativePath, RulesEngine } from '@aetherhub/rules';
import { config } from './config';

let engine: RulesEngine | null | undefined;
let loadError: string | null = null;

/** The rules engine, loaded once; null when the native module or card files are missing. */
export function rulesEngine(): RulesEngine | null {
  if (engine !== undefined) return engine;
  try {
    if (!nativePath()) throw new Error('native module not built');
    if (!fs.existsSync(config.rulesCards)) throw new Error(`card directory ${config.rulesCards} not found`);
    engine = RulesEngine.load(config.rulesCards);
  } catch (e) {
    loadError = (e as Error).message;
    engine = null;
  }
  return engine;
}

export function rulesEngineError(): string | null {
  rulesEngine();
  return loadError;
}
