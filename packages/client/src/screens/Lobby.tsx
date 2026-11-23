import { useQuery } from '@tanstack/react-query';
import { useEffect, useRef, useState } from 'react';
import { Link, useNavigate, useParams } from 'react-router-dom';
import type { AiLevel, GameSettings, GameType, LobbySlot, LobbyView } from '@aetherhub/shared';
import { FORMATS, formatInfo, formatName, GAME_KINDS, gameKind, gameTypeLabel, MULLIGAN_RULES, RULES_MODES, rulesMode, typePatch, type GameKind, type RulesMode } from '@aetherhub/shared';
import { api } from '../lib/api';
import { errMsg, useApp } from '../lib/store';
import { askConfirm } from '../ui/dialogs';
import { Select } from '../ui/Select';
import { Avatar, Chevron, Modal, Switch } from '../ui/ui';
import { DeckPicker } from './DeckPicker';
import { EventPanel, SetPicker } from './draft/EventPanel';

const LEVELS: AiLevel[] = ['Easy', 'Normal', 'Hard'];

export default function Lobby() {
  const me = useApp((s) => s.me)!;
  const stored = useApp((s) => s.lobby);
  const toast = useApp((s) => s.toast);
  const nav = useNavigate();
  const { id } = useParams();
  const [pwPrompt, setPwPrompt] = useState(false);
  const [pw, setPw] = useState('');
  const [error, setError] = useState('');
  const [picking, setPicking] = useState<number | null>(null);
  const tried = useRef<string | null>(null);
  const lobby = stored && stored.id === id ? stored : null;

  // /lobby (old links): go to the lobby you are in, or open a new one
  useEffect(() => {
    if (id) return;
    if (stored) nav(`/lobby/${stored.id}`, { replace: true });
    else
      api
        .post<{ lobby: LobbyView }>('api/lobbies', { settings: {} })
        .then((r) => {
          useApp.setState({ lobby: r.lobby });
          nav(`/lobby/${r.lobby.id}`, { replace: true });
        })
        .catch((e) => setError(errMsg(e)));
  }, [id, stored, nav]);

  // /lobby/:id: join it if you are not in it yet
  const join = (p: string | null) =>
    api
      .post<{ lobby: LobbyView }>(`api/lobbies/${id}/join`, { password: p })
      .then((r) => {
        useApp.setState({ lobby: r.lobby });
        setPwPrompt(false);
      })
      .catch((e) => {
        if (/password/i.test(errMsg(e))) setPwPrompt(true);
        else setError(errMsg(e));
      });
  // once the lobby is on screen, losing it (the game started, you were removed) must not fetch it again
  const had = useRef(false);
  if (lobby && id) {
    tried.current = id;
    had.current = true;
  }
  useEffect(() => {
    if (!id || lobby || tried.current === id) return;
    tried.current = id;
    api
      .get<LobbyView>(`api/lobbies/${id}`)
      .then((l) => {
        if (l.members.some((m) => m.userId === me.id)) useApp.setState({ lobby: l });
        else if (l.hasPassword && !me.isAdmin) setPwPrompt(true);
        else void join(null);
      })
      .catch((e) => setError(errMsg(e)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [id, lobby]);

  if (!lobby)
    return (
      <div className="screen" style={{ alignItems: 'center', justifyContent: 'center', gap: 14 }}>
        <div style={{ color: 'var(--mu)' }}>{error || (had.current ? 'You are no longer in this lobby.' : 'Opening the lobby…')}</div>
        <Link to="/" className="tb">
          Back to menu
        </Link>
        {pwPrompt && (
          <Modal
            title="Lobby password"
            onClose={() => nav('/')}
            width={400}
            foot={
              <button className="tb pri" onClick={() => join(pw)}>
                Join
              </button>
            }
          >
            <div style={{ padding: 20 }}>
              <input className="inp" type="password" style={{ width: '100%' }} autoFocus value={pw} onChange={(e) => setPw(e.target.value)} onKeyDown={(e) => e.key === 'Enter' && join(pw)} />
            </div>
          </Modal>
        )}
      </div>
    );
  return <LobbyRoom lobby={lobby} picking={picking} setPicking={setPicking} onError={toast} />;
}

function LobbyRoom({ lobby, picking, setPicking, onError }: { lobby: LobbyView; picking: number | null; setPicking: (n: number | null) => void; onError: (m: string) => void }) {
  const me = useApp((s) => s.me)!;
  const nav = useNavigate();
  const [draft, setDraft] = useState('');
  const [name, setName] = useState<string | null>(null);
  const [password, setPassword] = useState<string | null>(null);
  const chatEnd = useRef<HTMLDivElement>(null);
  const s = lobby.settings;
  const type: GameType = s.type ?? 'sandbox';
  const kind = gameKind(type);
  const mode = rulesMode(s);
  const L = `api/lobbies/${lobby.id}`;
  const isHost = lobby.hostId === me.id || me.isAdmin;
  const setup = lobby.phase === 'setup';
  const myMember = lobby.members.find((m) => m.userId === me.id);
  const mySeats = lobby.slots.flatMap((x, i) => (x.kind === 'human' && x.userId === me.id ? [i] : []));
  const run = (p: Promise<unknown>) => p.catch((e) => onError(errMsg(e)));
  const set = (patch: Partial<GameSettings> & { password?: string | null; removeAi?: boolean }) => run(api.put(`${L}/settings`, patch));
  const lifeDefault = formatInfo(s.format).life;
  const filled = lobby.slots.filter((x) => x.kind !== 'open').length;
  const needsDecks = type === 'sandbox' || type === 'rules';
  const hostName = lobby.members.find((m) => m.userId === lobby.hostId)?.name ?? '?';
  const rulesCheck = useQuery({
    queryKey: ['rules-check', lobby.slots.map((x) => (x.kind !== 'open' ? x.deckId : null)).join(',')],
    queryFn: () => api.post<Record<number, { unsupported: string[]; commanders: string[]; playable: number; total: number }>>('api/rules/check', { deckIds: lobby.slots.flatMap((x) => (x.kind !== 'open' && x.deckId != null ? [x.deckId] : [])) }),
    enabled: type === 'rules' && lobby.slots.some((x) => x.kind !== 'open' && x.deckId != null),
  });

  useEffect(() => {
    chatEnd.current?.scrollIntoView({ block: 'end' });
  }, [lobby.chat.length]);

  /** Game type or rules change (the other choice is kept): switching to sandbox with AIs seated asks first. */
  const changeType = async (k: GameKind, m: RulesMode) => {
    const patch = typePatch(k, m);
    if (m === 'sandbox' && lobby.slots.some((x) => x.kind === 'ai')) {
      const ok = await askConfirm({ title: 'Remove the AI players?', text: 'AI players can only play rules enforced games. Switching to sandbox removes them from their seats.', confirm: 'Switch and remove them', danger: true });
      if (!ok) return;
      return set({ ...patch, removeAi: true });
    }
    return set(patch);
  };

  /** Add an AI: a sandbox lobby asks to switch to rules enforced first (and adds nothing if the answer is no). */
  const addAi = async (seat: number, level: AiLevel = 'Normal') => {
    if (mode === 'sandbox') {
      if (!isHost) return onError('AI players need a rules enforced game: ask the host to switch the rules.');
      const ok = await askConfirm({ title: 'AI players need a rules enforced game', text: 'A sandbox game enforces no rules, so an AI cannot play it. Switch this lobby to rules enforced and add the AI?', confirm: 'Switch and add the AI' });
      if (!ok) return;
      try {
        await api.put(`${L}/settings`, typePatch(kind, 'rules'));
      } catch (e) {
        return onError(errMsg(e));
      }
    }
    await run(api.post(`${L}/seats/${seat}`, { ai: level }));
  };

  const leave = async () => {
    await api.post(`${L}/leave`).catch(() => {});
    useApp.setState({ lobby: null });
    nav('/');
  };
  const blockers = blockersOf(lobby);

  const toggle = (label: string, hint: string, on: boolean, key: keyof GameSettings) => (
    <div className="row" style={{ gap: 12, padding: '8px 0' }}>
      <div className="grow">
        <div style={{ fontSize: 13 }}>{label}</div>
        <div style={{ fontSize: 11.5, color: 'var(--mu2)', marginTop: 2 }}>{hint}</div>
      </div>
      {isHost ? <Switch on={on} onChange={(v) => set({ [key]: v } as Partial<GameSettings>)} label={label} /> : <span style={{ fontSize: 12, color: on ? 'var(--ok)' : 'var(--mu2)' }}>{on ? 'On' : 'Off'}</span>}
    </div>
  );

  return (
    <div className="screen">
      <header className="page-header" style={{ flexWrap: 'wrap' }}>
        <button className="tb" aria-label="Leave lobby" onClick={leave}>
          <Chevron />
          Leave
        </button>
        <h1>{s.name}</h1>
        <span style={{ fontSize: 12, color: 'var(--mu)', padding: '3px 8px', border: '1px solid var(--line3)', borderRadius: 6 }}>
          {gameTypeLabel(s)} · {lobby.hostId === me.id ? 'you are the host' : `${hostName} is hosting`}
        </span>
        <div className="grow" />
        <span className="hide-sm" style={{ fontSize: 12.5, color: 'var(--mu)' }}>
          {filled} of {s.maxPlayers} seats{needsDecks ? ` · ${formatName(s.format)} · ${s.bo === 3 ? 'Best of 3' : 'Best of 1'}` : ''}
        </span>
        {isHost && setup && (
          <button className={'tb pri' + (blockers.length ? ' dis' : '')} title={blockers.length ? `Waiting for ${blockers.join(', ')}` : undefined} onClick={() => run(api.post(`${L}/start`))}>
            {blockers.length ? `Waiting for ${blockers[0]}${blockers.length > 1 ? '…' : ''}` : type === 'draft' ? 'Start the draft' : type === 'sealed' ? 'Open the packs' : 'Start game'}
          </button>
        )}
      </header>
      <div style={{ flexGrow: 1, display: 'flex', minHeight: 0, flexWrap: 'wrap', overflowY: 'auto' }}>
        <aside className="lg col" style={{ width: 430, maxWidth: '100%', flexShrink: 0, borderRight: '1px solid var(--line)', padding: '20px 24px', overflowY: 'auto', gap: 18 }}>
          <h2 style={{ fontSize: 18 }}>Game settings</h2>
          <fieldset disabled={!isHost} style={{ border: 0, padding: 0, margin: 0, display: 'flex', flexDirection: 'column', gap: 18 }}>
            <label className="field">
              Name
              <input className="inp" value={name ?? s.name} onChange={(e) => setName(e.target.value)} onBlur={() => name && name !== s.name && set({ name }).then(() => setName(null))} />
            </label>
            <div className="row" style={{ gap: 12, alignItems: 'flex-start' }}>
              <div className="field grow" style={{ flexBasis: 0 }}>
                Game type
                <Select aria-label="Game type" value={kind} disabled={!isHost || !setup} onChange={(k) => changeType(k, mode)} options={GAME_KINDS.map((t) => ({ value: t.id, label: t.label, hint: t.hint }))} menuWidth={340} />
              </div>
              <div className="field grow" style={{ flexBasis: 0 }}>
                Rules
                <Select aria-label="Rules" value={mode} disabled={!isHost || !setup} onChange={(m) => changeType(kind, m)} options={RULES_MODES.map((t) => ({ value: t.id, label: t.label, hint: t.hint }))} menuWidth={340} />
              </div>
            </div>
            <span style={{ fontSize: 11.5, color: 'var(--mu2)', lineHeight: 1.45, marginTop: -10 }}>
              {GAME_KINDS.find((t) => t.id === kind)?.hint} {RULES_MODES.find((t) => t.id === mode)?.hint}
            </span>
            {needsDecks ? (
              <>
                <div className="row" style={{ gap: 12 }}>
                  <div className="field grow">
                    Format
                    <Select aria-label="Format" value={s.format} disabled={!isHost} onChange={(v) => set({ format: v })} options={FORMATS.map((f) => ({ value: f.id as string, label: f.name }))} />
                  </div>
                  <SeatCount lobby={lobby} isHost={isHost} set={set} />
                </div>
                {type === 'sandbox' ? (
                  <div className="field">
                    Starting life
                    <div className="row" style={{ gap: 8, flexWrap: 'wrap' }}>
                      <button className="sq" aria-label="Less starting life" onClick={() => set({ life: Math.max(1, s.life - 1) })}>
                        −
                      </button>
                      <input
                        className="inp mono"
                        type="number"
                        min={1}
                        max={999}
                        aria-label="Starting life total"
                        value={s.life}
                        onChange={(e) => {
                          const v = parseInt(e.target.value, 10);
                          if (!isNaN(v)) set({ life: Math.max(1, Math.min(999, v)) });
                        }}
                        style={{ width: 72, textAlign: 'center', fontSize: 14, padding: '0 6px' }}
                      />
                      <button className="sq" aria-label="More starting life" onClick={() => set({ life: Math.min(999, s.life + 1) })}>
                        +
                      </button>
                      <div className="segs" style={{ marginLeft: 4 }}>
                        {[20, 25, 30, 40].map((n) => (
                          <button key={n} className={'seg' + (s.life === n ? ' on' : '')} onClick={() => set({ life: n })}>
                            {n}
                          </button>
                        ))}
                      </div>
                    </div>
                    <div className="row" style={{ fontSize: 11.5, color: 'var(--mu2)', gap: 8 }}>
                      {s.life !== lifeDefault ? `Custom · ${formatName(s.format)} default is ${lifeDefault}.` : `Default for ${formatName(s.format)}. Applies to every player; life can still be changed in game.`}
                      {s.life !== lifeDefault && isHost && (
                        <button className="lnk" style={{ fontSize: 11.5 }} onClick={() => set({ life: lifeDefault })}>
                          Reset to {lifeDefault}
                        </button>
                      )}
                    </div>
                  </div>
                ) : (
                  <div style={{ fontSize: 11.5, color: 'var(--mu2)' }}>
                    {formatInfo(s.format).commandZone ? 'Commander rules: 40 life, command zone and commander damage.' : 'Standard rules: 20 life.'} The rules engine sets the starting life.
                  </div>
                )}
              </>
            ) : (
              <>
                <SeatCount lobby={lobby} isHost={isHost && setup} set={set} label={type === 'draft' ? 'Drafters' : 'Players'} />
                <SetPicker type={type} sets={s.sets ?? []} disabled={!isHost || !setup} onChange={(sets) => set({ sets })} />
                {type === 'draft' && toggle('Pick timer', 'Official paper times: 40 seconds for the first pick, fewer as the pack shrinks; time out and the best card is picked for you', s.pickTimer === true, 'pickTimer')}
                <div style={{ fontSize: 11.5, color: 'var(--mu2)', lineHeight: 1.45 }}>
                  Seats are the pod{mode === 'rules' ? ': add AI drafters now (each makes one deck the AIs play with)' : ''}. After the {type === 'draft' ? 'draft' : 'packs are opened'} everyone builds a 40-card deck, then you start the {mode === 'rules' ? 'rules enforced' : 'sandbox'} games you want from this lobby.
                </div>
              </>
            )}
            <label className="field">
              Password (optional)
              <input
                className="inp"
                type="password"
                placeholder={lobby.hasPassword ? '•••••• (set)' : 'No password'}
                value={password ?? ''}
                onChange={(e) => setPassword(e.target.value)}
                onBlur={() => password != null && set({ password: password || null }).then(() => setPassword(null))}
              />
            </label>
            {needsDecks && (
              <div className="col" style={{ gap: 8 }}>
                <div className="row">
                  <span style={{ width: 110, fontSize: 12.5, color: 'var(--tx3)' }}>Match</span>
                  <div className="segs">
                    {([1, 3] as const).map((b) => (
                      <button key={b} className={'seg' + (s.bo === b ? ' on' : '')} onClick={() => set({ bo: b })}>
                        Best of {b}
                      </button>
                    ))}
                  </div>
                </div>
                <div className="row" style={{ alignItems: 'flex-start' }}>
                  <span style={{ width: 110, fontSize: 12.5, color: 'var(--tx3)', paddingTop: 8 }}>Mulligan</span>
                  <div className="col grow" style={{ gap: 5 }}>
                    <Select aria-label="Mulligan rule" value={s.mulligan} disabled={!isHost} onChange={(v) => set({ mulligan: v })} options={MULLIGAN_RULES.map((m) => ({ value: m.id, label: m.label }))} />
                    <span style={{ fontSize: 11.5, color: 'var(--mu2)', lineHeight: 1.45 }}>{MULLIGAN_RULES.find((m) => m.id === s.mulligan)?.hint}</span>
                  </div>
                </div>
              </div>
            )}
            <div className="col" style={{ borderTop: '1px solid var(--line)', paddingTop: 12, gap: 2 }}>
              {toggle('Allow spectators', 'Spectators see everything, including every hand', s.spectators, 'spectators')}
              {toggle('Share spectator chat with players', 'Off keeps spectators in their own channel', s.specChatShared, 'specChatShared')}
              {type === 'sandbox' && toggle('Only owners touch their stuff', 'Off (default): anyone can act on anything', s.ownersOnly, 'ownersOnly')}
            </div>
            {needsDecks && (
              <div style={{ borderTop: '1px solid var(--line)', paddingTop: 12 }}>
                <div style={{ fontSize: 12, textTransform: 'uppercase', letterSpacing: 0.7, color: 'var(--mu3)', marginBottom: 4 }}>Special game objects</div>
                {toggle('Planechase', s.planechase ? `Shared planar deck: ${s.planarPool === 'all' ? 'pooled from all players' : 'the host’s'} · planar die in the top bar` : 'Shared planar deck and planar die (from the decks’ Planar sections)', s.planechase, 'planechase')}
                {s.planechase && (
                  <div className="row" style={{ paddingLeft: 12, marginBottom: 6 }}>
                    <span style={{ fontSize: 12, color: 'var(--mu)' }}>Planar deck</span>
                    <div className="segs">
                      <button className={'seg' + (s.planarPool === 'all' ? ' on' : '')} onClick={() => set({ planarPool: 'all' })}>
                        All players pooled
                      </button>
                      <button className={'seg' + (s.planarPool === 'host' ? ' on' : '')} onClick={() => set({ planarPool: 'host' })}>
                        Host’s deck
                      </button>
                    </div>
                  </div>
                )}
                {toggle('Archenemy', 'Players whose deck has a Schemes section are archenemies with a scheme deck; everyone else is their team of opponents', s.archenemy, 'archenemy')}
                <div style={{ fontSize: 11.5, color: 'var(--mu2)', marginTop: 6 }}>Dungeons and the Initiative switch on by themselves when a card that uses them is played.</div>
              </div>
            )}
          </fieldset>
        </aside>
        <main className="lg col" style={{ flex: '1 1 420px', padding: '20px 26px', overflowY: 'auto', gap: 16, minWidth: 0 }}>
          {!setup ? (
            <EventPanel lobby={lobby} isHost={isHost} onError={onError} />
          ) : (
            <>
              <div className="row">
                <h2 className="grow" style={{ fontSize: 18 }}>
                  Seats <span style={{ fontSize: 13, color: 'var(--mu2)', fontFamily: 'var(--font)', fontWeight: 400 }}>· seat 1 goes first</span>
                </h2>
                {isHost && (
                  <button className="tb" onClick={() => run(api.post(`${L}/reroll`))} title="Put the filled seats in a random order">
                    Re-roll order
                  </button>
                )}
              </div>
              <div className="col" style={{ gap: 8 }}>
                {lobby.slots.map((slot, i) => (
                  <SeatRow
                    key={i}
                    i={i}
                    slot={slot}
                    lobby={lobby}
                    isHost={isHost}
                    needsDecks={needsDecks}
                    check={type === 'rules' && slot.kind !== 'open' && slot.deckId != null ? rulesCheck.data?.[slot.deckId] : undefined}
                    commander={formatInfo(s.format).commandZone}
                    onPickDeck={() => setPicking(i)}
                    onAddAi={(lvl) => addAi(i, lvl)}
                    run={run}
                  />
                ))}
              </div>
              <Spectators lobby={lobby} isHost={isHost} run={run} />
              {myMember && (
                <div className="row" style={{ padding: '14px 16px', borderRadius: 12, background: 'var(--surface)', border: '1px solid var(--line2)', flexWrap: 'wrap' }}>
                  <div className="grow" style={{ fontSize: 13, color: 'var(--tx3)' }}>
                    {!mySeats.length
                      ? 'You are watching. Take an open seat to play — you can take several to play a solo game.'
                      : myMember.ready
                        ? 'You are ready. Changing a seat or deck un-readies you.'
                        : needsDecks
                          ? `Select a deck for ${mySeats.length > 1 ? 'each of your seats' : 'your seat'}, then ready up. The host starts once everyone is ready.`
                          : 'Ready up when you are set. The host starts the event once everyone is ready.'}
                  </div>
                  {mySeats.length > 0 && (
                    <button className={'tb ' + (myMember.ready ? '' : 'pri')} style={{ height: 38, padding: '0 22px', fontSize: 13.5 }} onClick={() => run(api.put(`${L}/ready`, { ready: !myMember.ready }))}>
                      {myMember.ready ? 'Ready ✓' : 'Ready up'}
                    </button>
                  )}
                </div>
              )}
            </>
          )}
        </main>
        <aside className="col" style={{ width: 330, maxWidth: '100%', flexShrink: 0, borderLeft: '1px solid var(--line)', background: 'var(--panel)', minHeight: 320 }}>
          <div style={{ padding: '16px 18px 10px', fontSize: 13, fontWeight: 600 }}>Lobby chat</div>
          <div className="lg col" style={{ flexGrow: 1, overflowY: 'auto', padding: '0 18px', gap: 10, minHeight: 0 }}>
            {lobby.chat.map((m, i) => (
              <div key={i} style={{ fontSize: 13, lineHeight: 1.45 }}>
                <span style={{ fontWeight: 600, color: m.color }}>{m.who}</span> <span style={{ color: m.system ? 'var(--mu2)' : 'var(--tx2)' }}>{m.text}</span>
              </div>
            ))}
            <div ref={chatEnd} />
          </div>
          <div style={{ padding: '12px 14px', borderTop: '1px solid var(--line)' }}>
            <input
              className="inp"
              aria-label="Chat message"
              placeholder="Say something… (Enter)"
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter' && draft.trim()) {
                  void run(api.post(`${L}/chat`, { text: draft.trim() }));
                  setDraft('');
                }
              }}
              style={{ width: '100%' }}
            />
          </div>
        </aside>
      </div>
      {picking != null && lobby.slots[picking] && lobby.slots[picking].kind !== 'open' && (
        <DeckPicker
          title={`Select a deck for ${(lobby.slots[picking] as { name: string }).name} (seat ${picking + 1})`}
          format={s.format}
          onClose={() => setPicking(null)}
          onSelect={(deckId) => {
            void run(api.put(`${L}/seats/${picking}/deck`, { deckId }));
            setPicking(null);
          }}
        />
      )}
    </div>
  );
}

/** What keeps the host from starting (mirrors the server's check, for the button label). */
function blockersOf(l: LobbyView): string[] {
  const type = l.settings.type ?? 'sandbox';
  const out: string[] = [];
  const filled = l.slots.filter((x) => x.kind !== 'open');
  if (filled.length < 2 && type !== 'sealed') out.push('a second player');
  if (!filled.some((x) => x.kind === 'human')) out.push('a player');
  if (rulesMode(l.settings) === 'sandbox' && filled.some((x) => x.kind === 'ai')) out.push('a rules enforced game');
  if (type === 'sandbox' || type === 'rules') {
    const noDeck = l.slots.flatMap((x, i) => (x.kind === 'human' && x.deckId == null ? [i + 1] : []));
    if (noDeck.length) out.push(`a deck for seat ${noDeck.join(', ')}`);
  }
  if ((type === 'draft' || type === 'sealed') && !(l.settings.sets ?? []).length) out.push('a set to be chosen');
  const seated = new Set(filled.flatMap((x) => (x.kind === 'human' ? [x.userId] : [])));
  const waiting = l.members.filter((m) => seated.has(m.userId) && !m.ready).map((m) => m.name);
  if (waiting.length) out.push(`${waiting.join(', ')} to be ready`);
  return out;
}

function SeatCount({ lobby, isHost, set, label = 'Max players' }: { lobby: LobbyView; isHost: boolean; set: (p: Partial<GameSettings>) => unknown; label?: string }) {
  const n = lobby.settings.maxPlayers;
  const lastFilled = lobby.slots.reduce((m, x, i) => (x.kind !== 'open' ? i : m), -1);
  return (
    <div className="field">
      {label}
      <div className="row" style={{ height: 34, gap: 8 }}>
        <button className="sq" aria-label="Fewer seats" disabled={!isHost || n <= Math.max(2, lastFilled + 1)} onClick={() => set({ maxPlayers: n - 1 })}>
          −
        </button>
        <span className="mono" style={{ fontSize: 14, color: 'var(--tx)', width: 16, textAlign: 'center' }}>
          {n}
        </span>
        <button className="sq" aria-label="More seats" disabled={!isHost || n >= 8} onClick={() => set({ maxPlayers: n + 1 })}>
          +
        </button>
      </div>
    </div>
  );
}

function SeatRow(p: {
  i: number;
  slot: LobbySlot;
  lobby: LobbyView;
  isHost: boolean;
  needsDecks: boolean;
  check?: { unsupported: string[]; commanders: string[]; playable: number; total: number };
  commander: boolean;
  onPickDeck: () => void;
  onAddAi: (lvl: AiLevel) => void;
  run: (x: Promise<unknown>) => unknown;
}) {
  const me = useApp((s) => s.me)!;
  const { i, slot, lobby, isHost, run } = p;
  const L = `api/lobbies/${lobby.id}`;
  const amMember = lobby.members.some((m) => m.userId === me.id);
  const move = (d: number) => run(api.post(`${L}/move`, { seat: i, d }));
  const num = (
    <span className="mono" style={{ fontSize: 13, color: 'var(--mu2)', width: 16 }}>
      {i + 1}
    </span>
  );
  const arrowBtn: React.CSSProperties = { width: 22, height: 16, padding: 0, border: '1px solid var(--line3)', borderRadius: 4, background: 'var(--surface2)', color: 'var(--tx3)', display: 'inline-flex', alignItems: 'center', justifyContent: 'center', cursor: 'pointer' };
  const arrows = isHost && (
    <div className="col" style={{ gap: 2, flexShrink: 0 }}>
      <button aria-label="Earlier in turn order" title="Earlier in turn order" disabled={i === 0} onClick={() => move(-1)} style={{ ...arrowBtn, opacity: i === 0 ? 0.35 : 1 }}>
        <Chevron dir="up" size={11} />
      </button>
      <button aria-label="Later in turn order" title="Later in turn order" disabled={i === lobby.slots.length - 1} onClick={() => move(1)} style={{ ...arrowBtn, opacity: i === lobby.slots.length - 1 ? 0.35 : 1 }}>
        <Chevron dir="down" size={11} />
      </button>
    </div>
  );
  if (slot.kind === 'open') {
    const others = lobby.members.filter((m) => m.userId !== me.id);
    return (
      <div className="row" style={{ gap: 14, padding: '12px 14px', borderRadius: 11, border: '1px dashed var(--line4)', flexWrap: 'wrap' }}>
        {num}
        {arrows}
        <span style={{ width: 34, height: 34, borderRadius: 17, border: '1.5px dashed var(--line5)' }} />
        <div className="grow" style={{ fontSize: 13, color: 'var(--mu3)' }}>
          Open seat
        </div>
        {amMember && (
          <button className="tb sm" onClick={() => run(api.post(`${L}/seats/${i}`, { userId: me.id }))}>
            Take seat
          </button>
        )}
        {amMember && (
          <button className="tb sm" onClick={() => p.onAddAi('Normal')} title="An AI player (rules enforced games only)">
            Add AI
          </button>
        )}
        {isHost && others.length > 0 && (
          <Select<number | ''>
            aria-label={`Seat a player in seat ${i + 1}`}
            value=""
            placeholder="Seat a player…"
            onChange={(v) => v !== '' && run(api.post(`${L}/seats/${i}`, { userId: v }))}
            style={{ height: 30, fontSize: 12 }}
            options={[{ value: '' as number | '', label: 'Seat a player…', disabled: true }, ...others.map((m) => ({ value: m.userId as number | '', label: m.name }))]}
          />
        )}
      </div>
    );
  }
  const mine = (slot.kind === 'human' && slot.userId === me.id) || (slot.kind === 'ai' && slot.addedBy === me.id);
  const canEdit = mine || isHost;
  const member = slot.kind === 'human' ? lobby.members.find((m) => m.userId === slot.userId) : undefined;
  const you = slot.kind === 'human' && slot.userId === me.id;
  return (
    <div className="row" style={{ gap: 14, padding: '12px 14px', borderRadius: 11, background: you ? 'var(--acc-bg)' : 'var(--surface)', border: `1px solid ${you ? 'var(--acc-bd)' : 'var(--line2)'}`, flexWrap: 'wrap' }}>
      {num}
      {arrows}
      {slot.kind === 'human' ? (
        <Avatar name={slot.name} color={slot.color} size={34} />
      ) : (
        <span title="AI player" style={{ width: 34, height: 34, borderRadius: 17, background: 'var(--surface3)', border: '1px solid var(--line4)', display: 'inline-flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0 }}>
          <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="var(--tx3)" strokeWidth="1.7" aria-hidden="true">
            <rect x="5" y="8" width="14" height="11" rx="2" />
            <path d="M12 4v4M9 13h.01M15 13h.01M9 16h6" />
          </svg>
        </span>
      )}
      <div className="grow" style={{ minWidth: 160 }}>
        <div className="row" style={{ fontSize: 14, fontWeight: 600, gap: 8 }}>
          {slot.name}
          {you ? ' (you)' : ''}
          {slot.kind === 'human' && slot.userId === lobby.hostId && (
            <span className="badge" style={{ color: 'var(--acc-ink)', background: 'var(--acc)' }}>
              HOST
            </span>
          )}
          {member && !member.connected && <span style={{ fontSize: 11, color: 'var(--mu2)', fontWeight: 400 }}>away</span>}
        </div>
        {p.needsDecks ? (
          <div style={{ fontSize: 12.5, color: slot.deckId == null ? '#e0a84f' : 'var(--mu)', marginTop: 2 }}>
            {slot.deckName ? `${slot.deckName}${slot.deckOwner && slot.deckOwner !== slot.name ? ` · ${slot.deckOwner}’s deck` : ''}` : slot.kind === 'ai' && !p.commander ? 'Preset deck (or select one)' : 'No deck selected yet'}
          </div>
        ) : (
          <div style={{ fontSize: 12.5, color: 'var(--mu)', marginTop: 2 }}>{slot.kind === 'ai' ? 'AI drafter' : 'Drafter'}</div>
        )}
        {p.check && (p.check.unsupported.length > 0 || (p.commander && !p.check.commanders.length)) && (
          <div style={{ fontSize: 11.5, color: 'var(--warn, #d9a441)', marginTop: 2 }}>
            {p.commander && !p.check.commanders.length ? 'No commander the rules engine supports. ' : ''}
            {p.check.unsupported.length ? `${p.check.playable} of ${p.check.total} cards supported; the rest are left out.` : ''}
          </div>
        )}
      </div>
      {slot.kind === 'human' && <span style={{ fontSize: 12.5, fontWeight: 600, color: member?.ready ? 'var(--ok)' : 'var(--mu)' }}>{member?.ready ? 'Ready' : 'Not ready'}</span>}
      {slot.kind === 'ai' && canEdit && <Select aria-label="AI difficulty" value={slot.difficulty} onChange={(v) => run(api.put(`${L}/seats/${i}/difficulty`, { difficulty: v }))} style={{ height: 30, fontSize: 12 }} options={LEVELS.map((l) => ({ value: l, label: l }))} />}
      {slot.kind === 'ai' && !canEdit && <span style={{ fontSize: 12, color: 'var(--mu)' }}>{slot.difficulty}</span>}
      {p.needsDecks && canEdit && (
        <button className={'tb sm' + (slot.deckId == null && slot.kind === 'human' ? ' pri' : '')} onClick={p.onPickDeck}>
          {slot.deckId == null ? 'Select deck' : 'Change deck'}
        </button>
      )}
      {canEdit && (
        <button className="tb sm" onClick={() => run(api.del(`${L}/seats/${i}`))} title={you ? 'Leave this seat (you stay in the lobby)' : 'Empty this seat'}>
          {you ? 'Stand up' : 'Remove'}
        </button>
      )}
    </div>
  );
}

/** Members who are not seated (watching), with kick for the host. */
function Spectators({ lobby, isHost, run }: { lobby: LobbyView; isHost: boolean; run: (x: Promise<unknown>) => unknown }) {
  const me = useApp((s) => s.me)!;
  const seated = new Set(lobby.slots.flatMap((x) => (x.kind === 'human' ? [x.userId] : [])));
  const watching = lobby.members.filter((m) => !seated.has(m.userId));
  const others = lobby.members.filter((m) => m.userId !== me.id && m.userId !== lobby.hostId);
  if (!watching.length && !(isHost && others.length)) return null;
  return (
    <div className="col" style={{ gap: 6 }}>
      {watching.length > 0 && (
        <div style={{ fontSize: 12.5, color: 'var(--mu)' }}>
          In the lobby without a seat: {watching.map((m) => m.name + (m.userId === me.id ? ' (you)' : '')).join(', ')}
        </div>
      )}
      {isHost && others.length > 0 && (
        <div className="row" style={{ gap: 6, flexWrap: 'wrap', fontSize: 12, color: 'var(--mu2)' }}>
          Remove from the lobby:
          {others.map((m) => (
            <button key={m.userId} className="tb sm" onClick={() => run(api.post(`api/lobbies/${lobby.id}/kick`, { userId: m.userId }))}>
              {m.name}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
