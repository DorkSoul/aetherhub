import type { rules as R } from '@aetherhub/shared';
import { loadNative, type NativeEngine, type NativeGame } from './native';

export { nativePath } from './native';
export type GameSetup = R.GameSetup;
export type SessionRecord = R.SessionRecord;
export type Advance = R.Advance;
export type Frame = R.Frame;
export type View = R.View;
export type Choice = R.Choice;
export type Decision = R.Decision;
export type PassUntil = R.PassUntil;

/** The compiled card database and the entry point for starting games. */
export class RulesEngine {
  private constructor(private native: NativeEngine) {}

  static load(cardsDir: string): RulesEngine {
    return new RulesEngine(loadNative().Engine.load(cardsDir));
  }
  get cardCount(): number {
    return this.native.cardCount();
  }
  hasCard(name: string): boolean {
    return this.native.hasCard(name);
  }
  cardNames(): string[] {
    return this.native.cardNames();
  }
  /** The engine's token definitions by name ("break the rules": create a token a card failed to make). */
  tokenNames(): string[] {
    return this.native.tokenNames();
  }
  newGame(setup: GameSetup): RulesGame {
    return new RulesGame(this.native.newGame(JSON.stringify(setup)));
  }
  restoreGame(record: SessionRecord): RulesGame {
    return this.restoreGameJson(JSON.stringify(record));
  }
  /** Restore from a `SessionRecord` JSON document (as `RulesGame.recordJson` gives it). Replays the whole log synchronously. */
  restoreGameJson(recordJson: string): RulesGame {
    return new RulesGame(this.native.restoreGame(recordJson));
  }
}

/**
 * One game. `advance()` runs on a worker thread and holds the game's lock for the whole call (every AI decision until a
 * human must answer); every other method is synchronous and takes the same lock on the calling thread, so calling one
 * while an `advance()` is in flight blocks the Node event loop until that AI turn ends. Callers serialize them.
 */
export class RulesGame {
  constructor(private g: NativeGame) {}
  /** Plays AI seats (off the event loop) until a human must answer or the game ends. */
  async advance(): Promise<Advance> {
    return JSON.parse(await this.g.advance()) as Advance;
  }
  answer(seat: number, choice: Choice): void {
    this.g.answer(seat, JSON.stringify(choice));
  }
  view(seat: number): View {
    return JSON.parse(this.g.view(seat)) as View;
  }
  record(): SessionRecord {
    return JSON.parse(this.recordJson()) as SessionRecord;
  }
  /** The record as the engine's JSON document (for storing it without parsing it first). */
  recordJson(): string {
    return this.g.record();
  }
  /** Where a seat wants priority: bit masks over steps (bit i = the i-th step, Untap = 0) for its own and for opponents' turns. */
  setStops(seat: number, mine: number, opp: number, fullControl = false): void {
    this.g.setStops(seat, mine, opp, fullControl);
  }
  /** Why a click or drag was refused, in plain words (`kind`: play, attack or block). */
  explain(seat: number, kind: 'play' | 'attack' | 'block' | 'tap', card: number | null, other: number | null): string {
    return this.g.explain(seat, kind, card, other);
  }
  /** Pass priority for a seat until the given turn and step (index into the step list, Untap = 0). */
  passUntil(seat: number, turn: number, step: number): void {
    this.g.passUntil(seat, turn, step);
  }
  /** Pass priority for a seat until the stack item (StackView.id) is on top of the stack, or has left it. */
  passUntilStack(seat: number, item: number): void {
    this.g.passUntilStack(seat, item);
  }
  /** "Break the rules": apply a direct change (throws if it can't be done). */
  edit(seat: number, op: R.EditOp): void {
    this.g.edit(seat, JSON.stringify(op));
  }
  /** Finish a rule break: the open priority question is asked again from scratch. */
  endBreak(seat: number): void {
    this.g.endBreak(seat);
  }
  /** Tap lands by hand for every payment (on), or let the engine pay automatically. */
  setManualPay(seat: number, on: boolean): void {
    this.g.setManualPay(seat, on);
  }
  /** Tap lands by hand for the next payment only. */
  manualOnce(seat: number): void {
    this.g.manualOnce(seat);
  }
  /** What proposed blocks ([blocker, attacker]) would do: kept and dropped blocks, and where each attacker's damage goes. */
  previewBlocks(blocks: [number, number][]): R.BlockPreview[] {
    return JSON.parse(this.g.previewBlocks(JSON.stringify(blocks))) as R.BlockPreview[];
  }
  /** Pass priority for a seat until `player`'s next turn starts: one after turn `after` (the turn the player saw), else after the current one. */
  passUntilPlayer(seat: number, player: number, after?: number): void {
    this.g.passUntilPlayer(seat, player, after ?? null);
  }
  /** Stop passing priority automatically. */
  cancelPass(seat: number): void {
    this.g.cancelPass(seat);
  }
  /** Where the seat's automatic passing ends, or null when it isn't passing. */
  passing(seat: number): PassUntil | null {
    return JSON.parse(this.g.passing(seat)) as PassUntil | null;
  }
  concede(seat: number): void {
    this.g.concede(seat);
  }
  undo(n: number): void {
    this.g.undo(n);
  }
  /** The current turn number (a synchronous engine call: see the class comment). */
  get turn(): number {
    return this.g.turn();
  }
}
