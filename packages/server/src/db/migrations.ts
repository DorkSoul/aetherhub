export interface Migration {
  id: number;
  name: string;
  sql: string;
}

// Append-only. Each migration runs in its own transaction after an automatic backup.
export const MIGRATIONS: Migration[] = [
  {
    id: 1,
    name: 'init',
    sql: `
CREATE TABLE users (
  id INTEGER PRIMARY KEY,
  username TEXT NOT NULL UNIQUE COLLATE NOCASE,
  pw_hash TEXT NOT NULL,
  is_admin INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  last_seen_at INTEGER
);
CREATE TABLE sessions (
  token_hash TEXT PRIMARY KEY,
  user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  expires_at INTEGER NOT NULL,
  created_ip TEXT,
  last_ip TEXT,
  user_agent TEXT
);
CREATE INDEX sessions_user ON sessions(user_id);
CREATE TABLE user_prefs (user_id INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE, json TEXT NOT NULL);
CREATE TABLE http_warnings (
  id INTEGER PRIMARY KEY,
  user_id INTEGER REFERENCES users(id) ON DELETE SET NULL,
  username TEXT,
  ip TEXT NOT NULL,
  at INTEGER NOT NULL,
  dismissed INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE server_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE assets (
  id INTEGER PRIMARY KEY,
  sha256 TEXT NOT NULL UNIQUE,
  kind TEXT NOT NULL,
  bytes INTEGER NOT NULL,
  width INTEGER,
  height INTEGER,
  owner_id INTEGER,
  created_at INTEGER NOT NULL
);
CREATE TABLE folders (
  id INTEGER PRIMARY KEY,
  user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  sort INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE decks (
  id INTEGER PRIMARY KEY,
  owner_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  format TEXT NOT NULL DEFAULT 'commander',
  folder_id INTEGER REFERENCES folders(id) ON DELETE SET NULL,
  notes TEXT NOT NULL DEFAULT '',
  cover_key TEXT,
  sleeve_asset INTEGER,
  colors TEXT NOT NULL DEFAULT '',
  card_count INTEGER NOT NULL DEFAULT 0,
  warnings INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE INDEX decks_owner ON decks(owner_id);
CREATE TABLE deck_pins (
  user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  deck_id INTEGER NOT NULL REFERENCES decks(id) ON DELETE CASCADE,
  PRIMARY KEY (user_id, deck_id)
);
CREATE TABLE deck_cards (
  id INTEGER PRIMARY KEY,
  deck_id INTEGER NOT NULL REFERENCES decks(id) ON DELETE CASCADE,
  section TEXT NOT NULL,
  card_key TEXT NOT NULL,
  qty INTEGER NOT NULL DEFAULT 1,
  foil INTEGER NOT NULL DEFAULT 0,
  tags TEXT NOT NULL DEFAULT '[]',
  added_at INTEGER NOT NULL,
  front_asset INTEGER,
  back_asset INTEGER
);
CREATE INDEX deck_cards_deck ON deck_cards(deck_id);
CREATE TABLE user_overrides (
  user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  oracle_id TEXT NOT NULL,
  front_asset INTEGER,
  back_asset INTEGER,
  PRIMARY KEY (user_id, oracle_id)
);
CREATE TABLE custom_cards (
  id INTEGER PRIMARY KEY,
  creator_id INTEGER REFERENCES users(id) ON DELETE SET NULL,
  name TEXT NOT NULL,
  mana_cost TEXT NOT NULL DEFAULT '',
  type_line TEXT NOT NULL DEFAULT '',
  power TEXT,
  toughness TEXT,
  loyalty TEXT,
  oracle_text TEXT NOT NULL DEFAULT '',
  colors TEXT NOT NULL DEFAULT '',
  rarity TEXT NOT NULL DEFAULT 'rare',
  image_asset INTEGER,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE TABLE games (
  id TEXT PRIMARY KEY,
  status TEXT NOT NULL,
  name TEXT NOT NULL,
  host_id INTEGER,
  settings TEXT NOT NULL,
  meta TEXT NOT NULL DEFAULT '{}',
  seq INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  suspended_by TEXT,
  suspended_at INTEGER
);
CREATE TABLE game_events (
  game_id TEXT NOT NULL REFERENCES games(id) ON DELETE CASCADE,
  seq INTEGER NOT NULL,
  actor INTEGER NOT NULL,
  actor_user INTEGER,
  intent TEXT NOT NULL,
  resolved TEXT NOT NULL,
  label TEXT NOT NULL DEFAULT '',
  at INTEGER NOT NULL,
  PRIMARY KEY (game_id, seq)
);
CREATE TABLE game_snapshots (
  game_id TEXT NOT NULL REFERENCES games(id) ON DELETE CASCADE,
  seq INTEGER NOT NULL,
  state TEXT NOT NULL,
  PRIMARY KEY (game_id, seq)
);
CREATE TABLE game_log (
  game_id TEXT NOT NULL REFERENCES games(id) ON DELETE CASCADE,
  id INTEGER NOT NULL,
  entry TEXT NOT NULL,
  PRIMARY KEY (game_id, id)
);
CREATE TABLE matches (
  id INTEGER PRIMARY KEY,
  game_id TEXT,
  name TEXT NOT NULL,
  format TEXT NOT NULL,
  bo INTEGER NOT NULL DEFAULT 1,
  started_at INTEGER NOT NULL,
  ended_at INTEGER NOT NULL,
  turns INTEGER NOT NULL DEFAULT 0,
  meta TEXT NOT NULL DEFAULT '{}',
  winner_name TEXT
);
CREATE TABLE match_players (
  match_id INTEGER NOT NULL REFERENCES matches(id) ON DELETE CASCADE,
  seat INTEGER NOT NULL,
  user_id INTEGER REFERENCES users(id) ON DELETE SET NULL,
  name TEXT NOT NULL,
  color TEXT NOT NULL,
  deck_name TEXT NOT NULL,
  result TEXT NOT NULL DEFAULT '',
  PRIMARY KEY (match_id, seat)
);
CREATE TABLE match_log (
  match_id INTEGER NOT NULL REFERENCES matches(id) ON DELETE CASCADE,
  idx INTEGER NOT NULL,
  t_offset INTEGER NOT NULL,
  who TEXT NOT NULL,
  color TEXT NOT NULL,
  kind TEXT NOT NULL,
  text TEXT NOT NULL,
  PRIMARY KEY (match_id, idx)
);
`,
  },
  {
    id: 2,
    name: 'solo_saves',
    sql: `
CREATE TABLE solo_saves (
  id INTEGER PRIMARY KEY,
  user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  summary TEXT NOT NULL,
  data TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE INDEX solo_saves_user ON solo_saves(user_id, updated_at);
`,
  },
  {
    id: 3,
    name: 'deck_tags',
    sql: `
ALTER TABLE decks ADD COLUMN tags TEXT NOT NULL DEFAULT '[]';
`,
  },
  {
    id: 4,
    name: 'global_tags',
    sql: `
CREATE TABLE user_tags (
  user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  color TEXT,
  sort INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (user_id, name)
);
CREATE TABLE user_card_tags (
  user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  card TEXT NOT NULL,
  tag TEXT NOT NULL,
  PRIMARY KEY (user_id, card, tag)
);
CREATE TABLE user_recent_tags (
  user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  scope TEXT NOT NULL,
  used_at INTEGER NOT NULL,
  PRIMARY KEY (user_id, name)
);
`,
  },
  {
    id: 5,
    name: 'rules_saves',
    sql: `
CREATE TABLE rules_saves (
  id INTEGER PRIMARY KEY,
  user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  cfg TEXT NOT NULL,
  answers TEXT NOT NULL,
  summary TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE INDEX rules_saves_user ON rules_saves(user_id, updated_at);
`,
  },
  {
    id: 6,
    name: 'attractions_to_maybe',
    // joke-set Attraction and sticker deck sections move to the maybeboard
    sql: `UPDATE deck_cards SET section = 'maybe' WHERE section IN ('attractions', 'stickers');`,
  },
  {
    id: 7,
    name: 'draft_events',
    // draft / sealed events: the event lobby and its draft, so a restart does not lose them
    sql: `CREATE TABLE draft_events (id TEXT PRIMARY KEY, lobby TEXT NOT NULL, draft TEXT NOT NULL, updated_at INTEGER NOT NULL);`,
  },
  {
    id: 8,
    name: 'rules_games',
    // rules enforced games in progress (seats and players, plus the engine's record), so a restart does not lose them
    sql: `CREATE TABLE rules_games (id TEXT PRIMARY KEY, state TEXT NOT NULL, record TEXT, updated_at INTEGER NOT NULL);`,
  },
];
