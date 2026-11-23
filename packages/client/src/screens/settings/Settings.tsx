import { useQuery, useQueryClient } from '@tanstack/react-query';
import { Select } from '../../ui/Select';
import { useEffect, useRef, useState } from 'react';
import { Link, useNavigate, useParams } from 'react-router-dom';
import type { HotkeyAction, PanelSlot, UserPrefs } from '@aetherhub/shared';
import { ACCENT_DEFAULT, ACCENTS, DEFAULT_HOTKEYS, defaultSlots, FORMATS, hexA, HOTKEY_LABELS, PLAYER_COLORS } from '@aetherhub/shared';
import { api, download } from '../../lib/api';
import { imgUrl } from '../../lib/base';
import { play } from '../../lib/sound';
import { errMsg, useApp } from '../../lib/store';
import { Chevron, Avatar, Confirm, Modal, Seg, Switch, timeAgo } from '../../ui/ui';

const H2 = ({ children }: { children: React.ReactNode }) => <h2 style={{ fontSize: 20, marginBottom: 4 }}>{children}</h2>;
const Hint = ({ children }: { children: React.ReactNode }) => <div style={{ fontSize: 12.5, color: 'var(--mu)', marginBottom: 14 }}>{children}</div>;

function Toggle({ label, hint, on, set }: { label: string; hint?: string; on: boolean; set: (v: boolean) => void }) {
  return (
    <div className="row" style={{ gap: 12, padding: '8px 0' }}>
      <div className="grow">
        <div style={{ fontSize: 13.5 }}>{label}</div>
        {hint && <div style={{ fontSize: 12, color: 'var(--mu2)', marginTop: 2 }}>{hint}</div>}
      </div>
      <Switch on={on} onChange={set} label={label} />
    </div>
  );
}

function Profile() {
  const me = useApp((s) => s.me)!;
  const p = me.prefs;
  const setPrefs = useApp((s) => s.setPrefs);
  const toast = useApp((s) => s.toast);
  const [cur, setCur] = useState('');
  const [next, setNext] = useState('');
  const [pwNote, setPwNote] = useState('');
  const file = useRef<HTMLInputElement>(null);
  const mat = imgUrl(p.playmat ? 'a/' + p.playmat : null);
  return (
    <div style={{ maxWidth: 760, display: 'flex', flexDirection: 'column', gap: 26 }}>
      <section>
        <H2>App colour</H2>
        <Hint>The highlight colour for buttons, links, selections and active tabs across the app. Only you see it.</Hint>
        {(['mtg', 'more'] as const).map((g) => (
          <div key={g} className="row" style={{ gap: 14, marginBottom: 12, flexWrap: 'wrap' }}>
            <span style={{ width: 110, fontSize: 12, color: 'var(--mu2)' }}>{g === 'mtg' ? 'The five colours' : 'More'}</span>
            <div className="row" style={{ gap: 10, flexWrap: 'wrap' }}>
              {ACCENTS.filter((a) => a.group === g).map((a) => {
                const on = p.accent === a.id;
                return (
                  <button
                    key={a.id}
                    onClick={() => setPrefs({ accent: a.id })}
                    aria-pressed={on}
                    title={a.name}
                    style={{ width: 76, display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 6, padding: '8px 0 7px', borderRadius: 10, border: `1px solid ${on ? a.hex : 'var(--line)'}`, background: on ? 'var(--surface2)' : 'var(--surface)', color: 'var(--tx)', cursor: 'pointer' }}
                  >
                    <span style={{ width: 30, height: 30, borderRadius: 15, background: a.hex, boxShadow: `0 0 0 2px ${on ? 'var(--tx)' : 'transparent'}` }} />
                    <span style={{ fontSize: 12 }}>
                      {a.name}
                      {a.id === ACCENT_DEFAULT ? ' (default)' : ''}
                    </span>
                  </button>
                );
              })}
            </div>
          </div>
        ))}
      </section>
      <section>
        <H2>Player color</H2>
        <Hint>Used for your border, arrows and your name in the log.</Hint>
        <div className="row" style={{ flexWrap: 'wrap' }}>
          {PLAYER_COLORS.map((c, i) => (
            <button key={c} onClick={() => setPrefs({ playerColor: c })} aria-label={`Player color ${i + 1}`} style={{ width: 36, height: 36, borderRadius: 18, background: c, border: `3px solid ${p.playerColor === c ? 'var(--tx)' : 'transparent'}`, cursor: 'pointer' }} />
          ))}
        </div>
      </section>
      <section>
        <H2>Playmat</H2>
        <Hint>Everyone sees it under your battlefield. Images up to 10 MB, stored as WebP.</Hint>
        <div className="row" style={{ gap: 16 }}>
          <div
            style={{
              width: 300,
              height: 120,
              borderRadius: 10,
              border: '1px solid var(--line)',
              backgroundColor: hexA(p.playerColor, 0.12),
              backgroundImage: mat ? `url("${mat}")` : 'radial-gradient(rgba(255,255,255,.06) 1px, transparent 1.3px)',
              backgroundSize: mat ? 'cover' : '18px 18px',
              backgroundPosition: 'center',
            }}
          />
          <div className="col" style={{ gap: 8 }}>
            <input
              ref={file}
              type="file"
              accept="image/*"
              hidden
              onChange={async (e) => {
                const f = e.target.files?.[0];
                if (!f) return;
                try {
                  setPrefs({ playmat: await api.upload(f, 'playmat') });
                  toast('Playmat uploaded');
                } catch (e2) {
                  toast(errMsg(e2));
                }
                e.target.value = '';
              }}
            />
            <button className="tb" onClick={() => file.current?.click()}>
              Upload image…
            </button>
            <button className="tb" onClick={() => setPrefs({ playmat: null })} disabled={!p.playmat}>
              Remove
            </button>
          </div>
        </div>
      </section>
      <section className="col" style={{ gap: 4 }}>
        <H2>Appearance &amp; table</H2>
        <div className="row" style={{ padding: '8px 0' }}>
          <div className="grow" style={{ fontSize: 13.5 }}>
            Theme
          </div>
          <Seg value={p.theme} options={[['dark', 'Dark'], ['light', 'Light']]} onChange={(v) => setPrefs({ theme: v })} label="Theme" />
        </div>
        <div className="row" style={{ padding: '8px 0' }}>
          <div className="grow col" style={{ gap: 2 }}>
            <span style={{ fontSize: 13.5 }}>Table</span>
            <span className="muted" style={{ fontSize: 12 }}>Classic lights each battlefield in its player’s colour · Felt is green baize · Playmats gives each player a mat of their colour</span>
          </div>
          <Seg value={p.tableTheme} options={[['classic', 'Classic'], ['felt', 'Felt'], ['playmats', 'Playmats']]} onChange={(v) => setPrefs({ tableTheme: v })} label="Table" />
        </div>
        <Toggle label="Tint player panels in player colour" hint="Each player’s info column and hand strip take on their player colour" on={p.tintPanels} set={(v) => setPrefs({ tintPanels: v })} />
        <Toggle label="Auto untap on my turn" hint="Cards marked “doesn’t untap” are skipped" on={p.autoUntap} set={(v) => setPrefs({ autoUntap: v })} />
        <Toggle label="Auto draw on my turn" hint="Off: your turn starts in the draw step and you draw yourself (D, or drag from your library)" on={p.autoDraw} set={(v) => setPrefs({ autoDraw: v })} />
        <Toggle label="Hover preview popup (Minimal view)" hint="Hold Shift to freeze it" on={p.preview} set={(v) => setPrefs({ preview: v })} />
        <Toggle label="Reduce motion" hint="Turns off card, ping and arrow animations" on={p.reduceMotion} set={(v) => setPrefs({ reduceMotion: v })} />
      </section>
      <UsernameSection />
      <section>
        <H2>Password</H2>
        <div className="row" style={{ flexWrap: 'wrap', marginTop: 8 }}>
          <input className="inp" type="password" placeholder="Current password" aria-label="Current password" value={cur} onChange={(e) => setCur(e.target.value)} style={{ width: 220 }} />
          <input className="inp" type="password" placeholder="New password" aria-label="New password" value={next} onChange={(e) => setNext(e.target.value)} style={{ width: 220 }} />
          <button
            className="tb"
            onClick={async () => {
              try {
                await api.put('api/me/password', { current: cur, next });
                setCur('');
                setNext('');
                setPwNote('Password changed. Other sessions stay signed in until they expire.');
              } catch (e) {
                setPwNote(errMsg(e));
              }
            }}
          >
            Change password
          </button>
        </div>
        <div style={{ fontSize: 12, color: 'var(--mu2)', marginTop: 8 }}>Sessions last 30 days. {pwNote}</div>
      </section>
    </div>
  );
}

/** Change your own username; asks for your password to confirm. */
function UsernameSection() {
  const me = useApp((s) => s.me)!;
  const [name, setName] = useState(me.username);
  const [pw, setPw] = useState('');
  const [note, setNote] = useState('');
  const changed = name.trim() !== me.username;
  return (
    <section>
      <H2>Username</H2>
      <form
        className="row"
        style={{ flexWrap: 'wrap', marginTop: 8 }}
        onSubmit={async (e) => {
          e.preventDefault();
          try {
            const r = await api.put<{ username: string }>('api/me/username', { username: name.trim(), password: pw });
            useApp.setState({ me: { ...useApp.getState().me!, username: r.username } });
            setPw('');
            setNote(`You're now ${r.username}. Sign in with the new name from now on.`);
          } catch (err) {
            setNote(errMsg(err));
          }
        }}
      >
        <input className="inp" aria-label="Username" autoComplete="username" value={name} onChange={(e) => setName(e.target.value)} style={{ width: 220 }} />
        <input className="inp" type="password" aria-label="Password to confirm" placeholder="Your password" autoComplete="current-password" value={pw} onChange={(e) => setPw(e.target.value)} style={{ width: 220 }} />
        <button className="tb" type="submit" disabled={!changed || !pw}>
          Change username
        </button>
      </form>
      <div style={{ fontSize: 12, color: 'var(--mu2)', marginTop: 8 }}>{note || '2–32 characters: letters, numbers, spaces, dots, dashes and underscores. Your decks and history move with you.'}</div>
    </section>
  );
}

interface EditableUser {
  id: number;
  username: string;
}

/** Admin: rename a user and/or set their password. */
function EditUserModal({ user, onClose, onSaved }: { user: EditableUser; onClose: () => void; onSaved: (username: string) => void }) {
  const me = useApp((s) => s.me)!;
  const [name, setName] = useState(user.username);
  const [pw, setPw] = useState('');
  const [pw2, setPw2] = useState('');
  const [err, setErr] = useState('');
  const [busy, setBusy] = useState(false);
  const self = user.id === me.id;
  const nameChanged = name.trim() !== user.username;
  const mismatch = pw !== pw2;
  const save = async () => {
    if (mismatch) return;
    setBusy(true);
    setErr('');
    try {
      await api.put(`api/admin/users/${user.id}`, { ...(nameChanged ? { username: name.trim() } : {}), ...(pw ? { password: pw } : {}) });
      if (self && nameChanged) useApp.setState({ me: { ...useApp.getState().me!, username: name.trim() } });
      onSaved(name.trim());
    } catch (e) {
      setErr(errMsg(e));
      setBusy(false);
    }
  };
  return (
    <Modal
      title={`Edit ${user.username}`}
      onClose={onClose}
      width={480}
      foot={
        <>
          <button className="tb" onClick={onClose}>
            Cancel
          </button>
          <button className="tb pri" disabled={busy || mismatch || (!nameChanged && !pw)} onClick={save}>
            Save
          </button>
        </>
      }
    >
      <form
        className="col"
        style={{ padding: '16px 20px', gap: 12 }}
        onSubmit={(e) => {
          e.preventDefault();
          void save();
        }}
      >
        <label className="field">
          Username
          <input className="inp" aria-label="Username" value={name} onChange={(e) => setName(e.target.value)} autoFocus />
        </label>
        <label className="field">
          New password <span style={{ color: 'var(--mu2)', fontWeight: 400 }}>· leave blank to keep the current one</span>
          <input className="inp" type="password" aria-label="New password" autoComplete="new-password" value={pw} onChange={(e) => setPw(e.target.value)} />
        </label>
        {pw && (
          <label className="field">
            Repeat new password
            <input className="inp" type="password" aria-label="Repeat new password" autoComplete="new-password" value={pw2} onChange={(e) => setPw2(e.target.value)} />
          </label>
        )}
        <div style={{ fontSize: 12.5, color: mismatch && pw2 ? 'var(--danger-tx)' : 'var(--mu2)', lineHeight: 1.5 }}>
          {mismatch && pw2 ? "The passwords don't match." : pw ? (self ? 'You stay signed in here.' : `${user.username} will be signed out everywhere and must use the new password.`) : 'Decks, games and history stay with the account when it is renamed.'}
        </div>
        {err && <div className="warnbox">{err}</div>}
        <button type="submit" hidden />
      </form>
    </Modal>
  );
}

const SLOT_LABEL: Record<PanelSlot, string> = { log: 'Log & chat', players: 'Players', rules: 'Rules text', tabs: 'Tabs: Log · Players · Rules', spoiler: 'Spoiler' };

function Views() {
  const p = useApp((s) => s.me!.prefs);
  const setPrefs = useApp((s) => s.setPrefs);
  const segRow = (label: string, hint: string, el: React.ReactNode) => (
    <div className="row" style={{ gap: 12, flexWrap: 'wrap' }}>
      <div style={{ width: 200 }}>
        <div style={{ fontSize: 13.5 }}>{label}</div>
        <div style={{ fontSize: 12, color: 'var(--mu2)', marginTop: 2 }}>{hint}</div>
      </div>
      {el}
    </div>
  );
  return (
    <div style={{ maxWidth: 900, display: 'flex', flexDirection: 'column', gap: 22 }}>
      <h2 style={{ fontSize: 20 }}>Default game views</h2>
      {segRow('Layout', 'Where opponents sit', <Seg value={p.layout} options={[['grid', 'Grid'], ['tab', 'Tab'], ['free', 'Free (pan & zoom)']]} onChange={(v) => setPrefs({ layout: v })} />)}
      {segRow('Panel', 'How card details appear', <Seg value={p.panel} options={[['minimal', 'Minimal'], ['detailed', 'Detailed']]} onChange={(v) => setPrefs({ panel: v })} />)}
      {segRow('Panel sections', 'Detailed view', <Seg value={p.sections} options={[[2, '2'], [3, '3'], [4, '4']]} onChange={(v) => setPrefs({ sections: v, slots: defaultSlots(v) })} />)}
      {segRow('Hover preview size', 'Minimal view popup', <Seg value={p.previewSize} options={[[80, '80%'], [100, '100%'], [120, '120%'], [150, '150%']]} onChange={(v) => setPrefs({ previewSize: v })} />)}
      {segRow(
        'Side panel width',
        'Detailed view',
        <div className="row" style={{ fontSize: 12.5, color: 'var(--tx3)' }}>
          <span style={{ width: 4, height: 26, borderRadius: 2, background: 'var(--acc)' }} />
          Drag the left edge of the side panel on the table. Your width ({p.panelW}px) is remembered.
        </div>,
      )}
      <div className="row" style={{ gap: 24, alignItems: 'flex-start', flexWrap: 'wrap' }}>
        <div className="col" style={{ gap: 10, width: 360 }}>
          <div style={{ fontSize: 13.5, fontWeight: 600 }}>Detailed panel sections</div>
          {p.slots.map((v, i) => (
            <label key={i} className="row" style={{ fontSize: 12.5, color: 'var(--mu)' }}>
              <span style={{ width: 90 }}>Section {i + 1}</span>
              {v === 'spoiler' ? (
                <span style={{ color: 'var(--tx)' }}>Spoiler (always last)</span>
              ) : (
                <Select
                  aria-label={`Panel section ${i + 1}`}
                  value={v as PanelSlot}
                  onChange={(nv) => {
                    const a = p.slots.slice();
                    a[i] = nv;
                    setPrefs({ slots: a });
                  }}
                  options={([['log', 'Log & chat'], ['players', 'Players'], ['rules', 'Rules text'], ['tabs', 'Tabs: all three']] as [PanelSlot, string][]).map(([value, label]) => ({ value, label }))}
                />
              )}
            </label>
          ))}
        </div>
        <div style={{ width: 220, height: 300, borderRadius: 10, border: '1px solid var(--line)', background: 'var(--surface)', display: 'flex', flexDirection: 'column', overflow: 'hidden' }} aria-label="Panel preview">
          {p.slots.map((v, i) => (
            <div key={i} style={{ flexGrow: v === 'spoiler' ? 3 : 2, flexBasis: 0, borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'center', justifyContent: 'center', fontSize: 12, textAlign: 'center', color: v === 'spoiler' ? 'var(--acc-tx)' : 'var(--mu)', background: v === 'spoiler' ? 'var(--acc-bg)' : 'transparent' }}>
              {SLOT_LABEL[v]}
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

export function keyName(e: KeyboardEvent | React.KeyboardEvent): string | null {
  if (['Control', 'Shift', 'Alt', 'Meta'].includes(e.key)) return null;
  return (e.ctrlKey || e.metaKey ? 'Ctrl+' : '') + (e.altKey ? 'Alt+' : '') + (e.key === ' ' ? 'Space' : e.key.length === 1 ? e.key.toUpperCase() : e.key);
}

function Hotkeys() {
  const p = useApp((s) => s.me!.prefs);
  const setPrefs = useApp((s) => s.setPrefs);
  const [listening, setListening] = useState<HotkeyAction | null>(null);
  return (
    <div style={{ maxWidth: 640 }}>
      <div className="row" style={{ marginBottom: 14 }}>
        <h2 className="grow" style={{ fontSize: 20 }}>
          Hotkeys
        </h2>
        <button className="tb" onClick={() => setPrefs({ hotkeys: DEFAULT_HOTKEYS })}>
          Reset defaults
        </button>
      </div>
      <Hint>Click a key, then press the new one. Esc cancels.</Hint>
      <div style={{ border: '1px solid var(--line2)', borderRadius: 12, overflow: 'hidden' }}>
        {(Object.keys(HOTKEY_LABELS) as HotkeyAction[]).map((k) => (
          <div key={k} className="row" style={{ padding: '11px 16px', borderBottom: '1px solid var(--line)', background: 'var(--surface)' }}>
            <span className="grow" style={{ fontSize: 13.5 }}>
              {HOTKEY_LABELS[k]}
            </span>
            <button
              className="mono"
              aria-label={`${HOTKEY_LABELS[k]}: ${p.hotkeys[k]}. Click to change`}
              onClick={() => setListening(listening === k ? null : k)}
              onKeyDown={(e) => {
                if (listening !== k) return;
                e.preventDefault();
                if (e.key === 'Escape') return setListening(null);
                const n = keyName(e);
                if (!n) return;
                setPrefs({ hotkeys: { ...p.hotkeys, [k]: n } });
                setListening(null);
              }}
              style={{ minWidth: 72, height: 30, padding: '0 10px', borderRadius: 7, border: `1px solid ${listening === k ? 'var(--acc)' : 'var(--line5)'}`, borderBottomWidth: 3, background: 'var(--surface3)', color: listening === k ? 'var(--acc-tx)' : 'var(--tx)', fontSize: 12.5, cursor: 'pointer' }}
            >
              {listening === k ? 'Press a key…' : p.hotkeys[k]}
            </button>
          </div>
        ))}
      </div>
    </div>
  );
}

function Sounds() {
  const p = useApp((s) => s.me!.prefs);
  const setPrefs = useApp((s) => s.setPrefs);
  const defs: [keyof UserPrefs['alerts'], string, string][] = [
    ['turn', 'Your turn', 'Sound plus a flashing tab title'],
    ['requests', 'Requests aimed at you', 'Hand-view requests, undo approvals'],
    ['pings', 'Pings and arrows at you', 'Middle-click pings and Alt-drag arrows'],
    ['chat', 'Chat messages', ''],
    ['actions', 'Action sounds', 'Shuffle, draw, tap, dice'],
  ];
  return (
    <div style={{ maxWidth: 640, display: 'flex', flexDirection: 'column', gap: 4 }}>
      <h2 style={{ fontSize: 20, marginBottom: 8 }}>Sounds &amp; alerts</h2>
      <div className="row" style={{ padding: '8px 0 14px' }}>
        <span className="grow" style={{ fontSize: 13.5 }}>
          Volume
        </span>
        <input type="range" min={0} max={100} value={p.volume} onChange={(e) => setPrefs({ volume: parseInt(e.target.value, 10) })} onMouseUp={() => play('turn')} aria-label="Volume" style={{ width: 220, accentColor: 'var(--acc)' }} />
        <span className="mono" style={{ fontSize: 12.5, width: 38, textAlign: 'right' }}>
          {p.volume}%
        </span>
      </div>
      {defs.map(([k, l, h]) => (
        <div key={k} style={{ borderTop: '1px solid var(--line)' }}>
          <Toggle label={l} hint={h} on={p.alerts[k]} set={(v) => setPrefs({ alerts: { ...p.alerts, [k]: v } })} />
        </div>
      ))}
    </div>
  );
}

interface AdminUser {
  id: number;
  username: string;
  isAdmin: boolean;
  createdAt: number;
  lastSeenAt: number | null;
  decks: number;
  customCards: number;
  online: boolean;
}

function Users() {
  const me = useApp((s) => s.me)!;
  const toast = useApp((s) => s.toast);
  const qc = useQueryClient();
  const users = useQuery({ queryKey: ['admin-users'], queryFn: () => api.get<AdminUser[]>('api/admin/users') });
  const warnings = useQuery({ queryKey: ['admin-warnings'], queryFn: () => api.get<{ id: number; username: string; ip: string; at: number }[]>('api/admin/warnings') });
  const [temp, setTemp] = useState<Record<string, string>>({});
  const [name, setName] = useState('');
  const [del, setDel] = useState<AdminUser | null>(null);
  const [mode, setMode] = useState<'transfer' | 'delete'>('transfer');
  const [edit, setEdit] = useState<AdminUser | null>(null);
  const refresh = () => qc.invalidateQueries({ queryKey: ['admin-users'] });
  const colors = PLAYER_COLORS;
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 18, maxWidth: 1020 }}>
      {!!warnings.data?.length && (
        <div className="warnbox">
          <span className="dot" style={{ background: 'var(--danger)' }} />
          <div className="grow" style={{ color: 'var(--tx3)' }}>
            {warnings.data.slice(0, 3).map((w) => (
              <div key={w.id}>
                {w.username} signed in from outside your LAN ({w.ip}) over plain HTTP {timeAgo(w.at)}.
              </div>
            ))}
            Use your HTTPS tunnel for remote players — cookies get the Secure flag automatically there.
          </div>
          <button className="tb" onClick={() => api.post('api/admin/warnings/dismiss').then(() => qc.invalidateQueries({ queryKey: ['admin-warnings'] }))}>
            Dismiss
          </button>
        </div>
      )}
      <div className="row" style={{ flexWrap: 'wrap' }}>
        <h2 className="grow" style={{ fontSize: 20 }}>
          Users
        </h2>
        <input className="inp" aria-label="New username" placeholder="New username" value={name} onChange={(e) => setName(e.target.value)} style={{ width: 200 }} />
        <button
          className="tb pri"
          onClick={async () => {
            try {
              const r = await api.post<{ username: string; tempPassword: string }>('api/admin/users', { username: name });
              setTemp((t) => ({ ...t, [r.username]: r.tempPassword }));
              setName('');
              refresh();
            } catch (e) {
              toast(errMsg(e));
            }
          }}
        >
          Create user
        </button>
      </div>
      <div style={{ border: '1px solid var(--line2)', borderRadius: 12, overflow: 'hidden' }}>
        {(users.data || []).map((u, i) => (
          <div key={u.id} className="row" style={{ gap: 14, padding: '12px 16px', borderBottom: '1px solid var(--line)', background: 'var(--surface)', flexWrap: 'wrap' }}>
            <Avatar name={u.username} color={colors[i % colors.length]} />
            <div style={{ width: 180 }}>
              <div style={{ fontSize: 14, fontWeight: 600 }}>
                {u.username}
                {u.id === me.id ? ' (you)' : ''}
              </div>
              <div style={{ fontSize: 12, color: 'var(--mu2)' }}>{u.online ? 'online now' : u.lastSeenAt ? timeAgo(u.lastSeenAt) : 'never signed in'}</div>
            </div>
            <span className="badge" style={{ background: u.isAdmin ? 'var(--acc-bg)' : 'var(--surface4)', color: u.isAdmin ? 'var(--acc-tx)' : 'var(--tx3)' }}>
              {u.isAdmin ? 'Admin' : 'Player'}
            </span>
            <div className="grow mono" style={{ fontSize: 12.5, color: 'var(--ok)' }}>
              {temp[u.username] ? `Temporary password: ${temp[u.username]}` : ''}
            </div>
            <button className="tb" onClick={() => setEdit(u)}>
              Edit
            </button>
            <button
              className="tb"
              onClick={async () => {
                const r = await api.post<{ tempPassword: string }>(`api/admin/users/${u.id}/reset`);
                setTemp((t) => ({ ...t, [u.username]: r.tempPassword }));
              }}
            >
              Reset password
            </button>
            <button
              className="tb"
              onClick={() =>
                api
                  .post(`api/admin/users/${u.id}/role`, { admin: !u.isAdmin })
                  .then(refresh)
                  .catch((e) => toast(errMsg(e)))
              }
            >
              {u.isAdmin ? 'Make player' : 'Make admin'}
            </button>
            {u.id !== me.id && (
              <button className="tb danger" onClick={() => (setDel(u), setMode('transfer'))}>
                Delete
              </button>
            )}
          </div>
        ))}
      </div>
      <div style={{ fontSize: 12.5, color: 'var(--mu2)' }}>
        <b>Edit</b> renames a user or sets a password you choose; <b>Reset password</b> makes a temporary one. Locked out? Run{' '}
        <span className="mono" style={{ color: 'var(--tx3)' }}>docker exec &lt;container&gt; npm run reset-password -- {me.username}</span> (your container's name, e.g. aetherhub) to print a temporary password.
      </div>
      {edit && (
        <EditUserModal
          user={edit}
          onClose={() => setEdit(null)}
          onSaved={(n) => {
            setEdit(null);
            refresh();
            toast(`Saved ${n}`);
          }}
        />
      )}
      {del && (
        <Confirm
          title={`Delete ${del.username}?`}
          danger
          confirm="Delete user"
          onNo={() => setDel(null)}
          onYes={async () => {
            try {
              await api.del(`api/admin/users/${del.id}?mode=${mode}`);
              setDel(null);
              refresh();
            } catch (e) {
              toast(errMsg(e));
            }
          }}
          text={
            <div className="col" style={{ gap: 10 }}>
              <div>
                They own {del.decks} deck{del.decks === 1 ? '' : 's'} and {del.customCards} custom card{del.customCards === 1 ? '' : 's'}. Match history keeps their name as “(deleted) {del.username}”.
              </div>
              {(
                [
                  ['transfer', `Transfer their decks and custom cards to ${me.username}`],
                  ['delete', 'Delete their decks and custom cards'],
                ] as const
              ).map(([k, l]) => (
                <button key={k} onClick={() => setMode(k)} className="row" style={{ padding: '11px 12px', borderRadius: 9, border: `1px solid ${mode === k ? 'var(--acc)' : 'var(--line5)'}`, background: mode === k ? 'var(--acc-bg)' : 'var(--surface2)', color: 'var(--tx)', fontSize: 13, cursor: 'pointer', textAlign: 'left' }}>
                  <span style={{ width: 14, height: 14, borderRadius: 7, border: `2px solid ${mode === k ? 'var(--acc)' : 'var(--line5)'}`, background: mode === k ? 'var(--acc)' : 'transparent' }} />
                  {l}
                </button>
              ))}
            </div>
          }
        />
      )}
    </div>
  );
}

interface CardDataStatus {
  printings: number;
  oracles: number;
  importedAt: number | null;
  importing: boolean;
  progress: { stage: string; rows: number; bytes: number; totalBytes: number; message?: string } | null;
  lastCheck: number | null;
  nextCheck: number;
  bulkName: string | null;
  lastError: string | null;
  cache: { files: number; bytes: number };
  prefetch: { running: boolean; done: number; total: number };
  queued: number;
  rulings?: number;
  rulingsUpdatedAt?: number | null;
}

function fmtBytes(b: number): string {
  if (b > 1e9) return (b / 1e9).toFixed(1) + ' GB';
  if (b > 1e6) return (b / 1e6).toFixed(1) + ' MB';
  return Math.round(b / 1e3) + ' KB';
}

function CardData() {
  const toast = useApp((s) => s.toast);
  const q = useQuery({ queryKey: ['carddata'], queryFn: () => api.get<CardDataStatus>('api/admin/carddata'), refetchInterval: (d) => (d.state.data?.importing || d.state.data?.prefetch.running ? 700 : 5000) });
  const s = q.data;
  const pr = s?.progress;
  const pct = !pr ? 0 : pr.stage === 'done' ? 100 : pr.stage === 'indexing' ? 90 : pr.totalBytes ? Math.min(88, (pr.bytes / pr.totalBytes) * 88) : pr.stage === 'importing' ? 50 : 5;
  const stageTxt = !pr ? '' : pr.stage === 'error' ? `Failed: ${pr.message}` : pr.stage === 'done' ? `Imported ${pr.rows.toLocaleString()} printings. Search index rebuilt.` : pr.stage === 'indexing' ? 'Rebuilding the search index…' : pr.rows ? `Streaming cards into SQLite… ${pr.rows.toLocaleString()}` : 'Downloading bulk file…';
  const tiles = s
    ? [
        { l: 'Last update', v: s.importedAt ? new Date(s.importedAt).toLocaleDateString(undefined, { month: 'short', day: 'numeric' }) : '—', sub: s.bulkName || 'no data yet' },
        { l: 'Printings', v: s.printings.toLocaleString(), sub: `${s.oracles.toLocaleString()} cards · SQLite + FTS5` },
        { l: 'Rulings', v: (s.rulings ?? 0).toLocaleString(), sub: s.rulingsUpdatedAt ? `updated ${timeAgo(s.rulingsUpdatedAt)} · with the card data` : 'not downloaded yet' },
        { l: 'Image cache', v: fmtBytes(s.cache.bytes), sub: `${s.cache.files.toLocaleString()} images${s.queued ? ` · ${s.queued} queued` : ''}` },
        { l: 'Next check', v: new Date(s.nextCheck).toLocaleDateString(undefined, { month: 'short', day: 'numeric' }), sub: 'weekly' },
      ]
    : [];
  return (
    <div style={{ maxWidth: 900, display: 'flex', flexDirection: 'column', gap: 18 }}>
      <h2 style={{ fontSize: 20 }}>Card data</h2>
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(180px, 1fr))', gap: 10 }}>
        {tiles.map((t) => (
          <div key={t.l} className="card-box" style={{ borderRadius: 10, padding: '12px 14px' }}>
            <div style={{ fontSize: 11.5, color: 'var(--mu2)' }}>{t.l}</div>
            <div className="display" style={{ fontSize: 22, marginTop: 3 }}>
              {t.v}
            </div>
            <div style={{ fontSize: 11.5, color: 'var(--mu2)', marginTop: 2 }}>{t.sub}</div>
          </div>
        ))}
      </div>
      <div className="card-box" style={{ padding: '16px 18px', display: 'flex', flexDirection: 'column', gap: 12 }}>
        <div className="row">
          <div className="grow">
            <div style={{ fontSize: 14, fontWeight: 600 }}>Scryfall Default Cards</div>
            <div style={{ fontSize: 12.5, color: 'var(--mu)', marginTop: 2 }}>Checked weekly via the bulk-data API. The download is streamed straight into the database, so nothing large is left on disk.</div>
          </div>
          <button
            className={'tb ' + (s?.importing ? '' : 'pri')}
            disabled={s?.importing}
            onClick={() =>
              api
                .post('api/admin/carddata/refresh')
                .then(() => q.refetch())
                .catch((e) => toast(errMsg(e)))
            }
          >
            {s?.importing ? 'Refreshing…' : 'Refresh now'}
          </button>
        </div>
        {pr && (
          <div>
            <div style={{ height: 8, borderRadius: 4, background: 'var(--surface4)', overflow: 'hidden' }}>
              <div style={{ width: `${pct}%`, height: '100%', background: pr.stage === 'error' ? 'var(--danger)' : 'var(--acc)', transition: 'width .25s' }} />
            </div>
            <div style={{ fontSize: 12, color: 'var(--mu)', marginTop: 6 }}>{stageTxt}</div>
          </div>
        )}
      </div>
      <div className="row">
        <button
          className="tb"
          onClick={() =>
            api
              .post<{ total: number }>('api/admin/images/prefetch')
              .then((r) => toast(`Prefetching ${r.total.toLocaleString()} images in the background`))
              .catch((e) => toast(errMsg(e)))
          }
        >
          Prefetch every image
        </button>
        <button className="tb danger" onClick={() => api.post('api/admin/images/clear').then(() => q.refetch())}>
          Clear image cache
        </button>
        {s?.prefetch.running && (
          <span style={{ fontSize: 12.5, color: 'var(--mu)' }}>
            Prefetching {s.prefetch.done.toLocaleString()} / {s.prefetch.total.toLocaleString()}
          </span>
        )}
      </div>
      <div style={{ fontSize: 12.5, color: 'var(--mu2)' }}>Images load through the server and are cached at normal (table) and large (preview) sizes on first request.</div>
    </div>
  );
}

function Server() {
  const toast = useApp((s) => s.toast);
  const q = useQuery({ queryKey: ['admin-settings'], queryFn: () => api.get<{ serverName: string; defaultFormat: string; defaultMaxPlayers: number; env: Record<string, string | number> }>('api/admin/settings') });
  const [f, setF] = useState<{ serverName: string; defaultFormat: string; defaultMaxPlayers: number } | null>(null);
  useEffect(() => {
    if (q.data && !f) setF({ serverName: q.data.serverName, defaultFormat: q.data.defaultFormat, defaultMaxPlayers: q.data.defaultMaxPlayers });
  }, [q.data, f]);
  if (!f || !q.data) return null;
  const e = q.data.env;
  return (
    <div style={{ maxWidth: 640, display: 'flex', flexDirection: 'column', gap: 16 }}>
      <h2 style={{ fontSize: 20 }}>Server</h2>
      <label className="field">
        Server name
        <input className="inp" value={f.serverName} onChange={(x) => setF({ ...f, serverName: x.target.value })} />
      </label>
      <div className="row">
        <label className="field grow">
          Default format
          <Select aria-label="Default format" value={f.defaultFormat} onChange={(v) => setF({ ...f, defaultFormat: v })} options={FORMATS.map((x) => ({ value: x.id as string, label: x.name }))} />
        </label>
        <label className="field grow">
          Default max players
          <Select aria-label="Default max players" value={f.defaultMaxPlayers} onChange={(v) => setF({ ...f, defaultMaxPlayers: v })} options={[2, 3, 4, 5, 6, 7, 8].map((n) => ({ value: n, label: String(n) }))} />
        </label>
      </div>
      <div className="card-box mono" style={{ borderRadius: 10, padding: '14px 16px', fontSize: 12.5, color: 'var(--mu)', lineHeight: 1.8 }}>
        PORT={e.port} · BASE_PATH={e.basePath} · data volume {e.dataDir}
        <br />
        UI, REST API and WebSocket on one port · heartbeat every {e.heartbeatSeconds} s<br />
        Uploads capped at {e.uploadLimitMb} MB, re-encoded to WebP · v{e.version} on Node {e.node}
      </div>
      <div>
        <button
          className="tb pri"
          onClick={() =>
            api
              .put('api/admin/settings', f)
              .then(() => toast('Saved'))
              .catch((x) => toast(errMsg(x)))
          }
        >
          Save
        </button>
      </div>
    </div>
  );
}

function Backups() {
  const toast = useApp((s) => s.toast);
  const q = useQuery({ queryKey: ['backups'], queryFn: () => api.get<{ file: string; reason: string; bytes: number; at: number }[]>('api/admin/backups') });
  const [restore, setRestore] = useState<string | null>(null);
  return (
    <div style={{ maxWidth: 820, display: 'flex', flexDirection: 'column', gap: 16 }}>
      <div className="row">
        <h2 className="grow" style={{ fontSize: 20 }}>
          Backups
        </h2>
        <button
          className="tb"
          onClick={() =>
            api
              .post('api/admin/backups')
              .then(() => q.refetch())
              .catch((e) => toast(errMsg(e)))
          }
        >
          Back up now
        </button>
        <button className="tb" onClick={() => download('api/admin/export', 'aetherhub.sqlite').catch((e) => toast(errMsg(e)))}>
          Export database
        </button>
      </div>
      <div style={{ fontSize: 12.5, color: 'var(--mu)' }}>A backup is taken automatically before every schema migration. The last 5 are kept in /data/backups. Card data is not included — it is re-downloaded from Scryfall.</div>
      <div style={{ border: '1px solid var(--line2)', borderRadius: 12, overflow: 'hidden' }}>
        {(q.data || []).map((b) => (
          <div key={b.file} className="row" style={{ gap: 14, padding: '12px 16px', borderBottom: '1px solid var(--line)', background: 'var(--surface)', flexWrap: 'wrap' }}>
            <span className="mono" style={{ fontSize: 12.5, width: 270 }}>
              {b.file}
            </span>
            <span className="grow" style={{ fontSize: 12.5, color: 'var(--mu)' }}>
              {b.reason}
            </span>
            <span className="mono" style={{ fontSize: 12, color: 'var(--mu2)' }}>
              {fmtBytes(b.bytes)}
            </span>
            <button className="tb" onClick={() => setRestore(b.file)}>
              Restore
            </button>
          </div>
        ))}
        {!q.data?.length && <div style={{ padding: 18, fontSize: 13, color: 'var(--mu)' }}>No backups yet.</div>}
      </div>
      {restore && (
        <Confirm
          title="Restore this backup?"
          text={`The current database is backed up first, then replaced by ${restore}. Everyone may need to sign in again.`}
          confirm="Restore"
          danger
          onNo={() => setRestore(null)}
          onYes={() =>
            api
              .post(`api/admin/backups/${encodeURIComponent(restore)}/restore`)
              .then(() => {
                toast('Restored. Reloading…');
                setTimeout(() => location.reload(), 800);
              })
              .catch((e) => toast(errMsg(e)))
              .finally(() => setRestore(null))
          }
        />
      )}
    </div>
  );
}

const PANES: [string, string, boolean][] = [
  ['profile', 'Profile & table', false],
  ['views', 'Game views', false],
  ['hotkeys', 'Hotkeys', false],
  ['sounds', 'Sounds & alerts', false],
  ['users', 'Users', true],
  ['cards', 'Card data', true],
  ['server', 'Server', true],
  ['backups', 'Backups', true],
];

export default function Settings() {
  const { pane = 'profile' } = useParams();
  const me = useApp((s) => s.me)!;
  const nav = useNavigate();
  const warnings = useQuery({ queryKey: ['admin-warnings'], queryFn: () => api.get<unknown[]>('api/admin/warnings'), enabled: me.isAdmin });
  const panes = PANES.filter((p) => !p[2] || me.isAdmin);
  const Body = { profile: Profile, views: Views, hotkeys: Hotkeys, sounds: Sounds, users: Users, cards: CardData, server: Server, backups: Backups }[pane] || Profile;
  return (
    <div className="screen">
      <header className="page-header">
        <Link to="/" className="tb" aria-label="Back to main menu">
          <Chevron />Menu
        </Link>
        <h1>Settings</h1>
      </header>
      <div style={{ flexGrow: 1, display: 'flex', minHeight: 0 }}>
        <aside style={{ width: 250, flexShrink: 0, borderRight: '1px solid var(--line)', padding: '16px 14px', display: 'flex', flexDirection: 'column', gap: 3 }} className="hide-sm">
          <div className="nav-head">You</div>
          {panes
            .filter((p) => !p[2])
            .map((p) => (
              <button key={p[0]} className={'nav' + (pane === p[0] ? ' on' : '')} onClick={() => nav(`/settings/${p[0]}`)}>
                {p[1]}
              </button>
            ))}
          {me.isAdmin && <div className="nav-head">Admin</div>}
          {panes
            .filter((p) => p[2])
            .map((p) => (
              <button key={p[0]} className={'nav' + (pane === p[0] ? ' on' : '')} onClick={() => nav(`/settings/${p[0]}`)}>
                {p[1]}
                {p[0] === 'users' && !!warnings.data?.length && <span className="dot" style={{ marginLeft: 'auto', background: 'var(--danger)' }} />}
              </button>
            ))}
        </aside>
        <main className="lg" style={{ flexGrow: 1, overflowY: 'auto', padding: '26px 40px 60px', minWidth: 0 }}>
          <div className="row" style={{ marginBottom: 16, display: 'none' }} />
          <Select aria-label="Settings page" style={{ marginBottom: 18, width: '100%', display: window.innerWidth < 760 ? 'flex' : 'none' }} value={pane} onChange={(v) => nav(`/settings/${v}`)} options={panes.map((p) => ({ value: p[0] as string, label: p[1] as string }))} />
          <Body />
        </main>
      </div>
    </div>
  );
}
