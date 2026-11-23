import type { AppDb } from './db/app';
import type { CardDb } from './db/cards';
import type { RulingsDb } from './db/rulings';
import type { CardDataService } from './carddata';
import type { DeckService } from './decks';
import type { GameHub } from './games/hub';
import type { ImageCache } from './images';

export interface Ctx {
  app: AppDb;
  cards: CardDb;
  rulings: RulingsDb;
  images: ImageCache;
  decks: DeckService;
  carddata: CardDataService;
  hub: GameHub;
  log: (m: string) => void;
}

export class HttpError extends Error {
  constructor(
    public status: number,
    message: string,
  ) {
    super(message);
  }
}

export function bad(msg: string): never {
  throw new HttpError(400, msg);
}
export function forbidden(msg = 'Not allowed'): never {
  throw new HttpError(403, msg);
}
export function notFound(msg = 'Not found'): never {
  throw new HttpError(404, msg);
}
