import { useState } from 'react';
import type { Me, SetupState } from '@aetherhub/shared';
import { api } from '../lib/api';
import { errMsg } from '../lib/store';
import { Logo } from '../ui/ui';

export function Login({ setup, onDone }: { setup: SetupState; onDone: (m: Me) => void }) {
  const firstRun = setup.needsAdmin;
  const [user, setUser] = useState('');
  const [pw, setPw] = useState('');
  const [pw2, setPw2] = useState('');
  const [err, setErr] = useState('');
  const [busy, setBusy] = useState(false);

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    setErr('');
    if (firstRun && pw !== pw2) return setErr('The passwords do not match.');
    setBusy(true);
    try {
      const me = await api.post<Me>(firstRun ? 'api/setup/admin' : 'api/auth/login', { username: user, password: pw });
      onDone(me);
    } catch (e2) {
      setErr(errMsg(e2));
    } finally {
      setBusy(false);
    }
  };

  const conn = setup.insecureRemote
    ? { color: '#e2604f', text: 'Plain HTTP from outside your LAN — your password is sent unencrypted. Use the HTTPS tunnel address.' }
    : setup.secure
      ? { color: '#6cc28a', text: 'Secure connection via your tunnel (HTTPS)' }
      : { color: '#6cc28a', text: 'Local network connection' };

  return (
    <div style={{ minHeight: '100%', display: 'flex', background: 'var(--bg)' }}>
      <div
        className="hide-sm"
        style={{
          width: 640,
          flexShrink: 0,
          backgroundColor: 'var(--acc-bg)',
          backgroundImage: 'repeating-linear-gradient(135deg, rgba(var(--acc-rgb),.05) 0 2px, transparent 2px 14px)',
          borderRight: '1px solid var(--acc-bd)',
          display: 'flex',
          flexDirection: 'column',
          justifyContent: 'space-between',
          padding: 56,
        }}
      >
        <div className="row" style={{ gap: 12 }}>
          <Logo size={30} />
          <span className="display" style={{ fontSize: 22 }}>
            Aetherhub
          </span>
        </div>
        <div>
          <div className="display" style={{ fontSize: 54, lineHeight: 1.05, letterSpacing: -0.5, color: 'var(--acc-tx)' }}>
            {setup.serverName}
          </div>
          <div style={{ fontSize: 16, color: 'var(--acc-mu)', marginTop: 16, lineHeight: 1.6, maxWidth: 440 }}>
            A private deckbuilder and sandbox tabletop. Build decks, sit down, shuffle up. No rules engine: play it like paper.
          </div>
        </div>
        <div className="row" style={{ gap: 22, fontSize: 12.5, color: 'var(--acc-mu)' }}>
          <span>2–8 players</span>
          <span>·</span>
          <span>Self-hosted</span>
          <span>·</span>
          <span>Card data from Scryfall</span>
        </div>
      </div>
      <div style={{ flexGrow: 1, display: 'flex', alignItems: 'center', justifyContent: 'center', padding: 16 }}>
        <form onSubmit={submit} style={{ width: 380, maxWidth: '100%', display: 'flex', flexDirection: 'column', gap: 18 }}>
          {firstRun ? (
            <>
              <div style={{ fontSize: 12, fontWeight: 600, letterSpacing: 1, textTransform: 'uppercase', color: 'var(--acc)' }}>First run</div>
              <h1 style={{ fontSize: 32 }}>Create the admin account</h1>
              <div style={{ fontSize: 13.5, color: 'var(--mu)', lineHeight: 1.55 }}>
                No admin exists yet on this server. This account can create users, refresh card data and manage backups. You can add more admins later.
              </div>
            </>
          ) : (
            <>
              <h1 style={{ fontSize: 34 }}>Sign in</h1>
              <div style={{ fontSize: 13.5, color: 'var(--mu)' }}>Accounts are created by an admin. There are no guest logins.</div>
            </>
          )}
          <label className="field" style={{ color: 'var(--tx3)', fontSize: 12.5 }}>
            Username
            <input className="inp lg" autoComplete="username" value={user} onChange={(e) => setUser(e.target.value)} autoFocus required />
          </label>
          <label className="field" style={{ color: 'var(--tx3)', fontSize: 12.5 }}>
            Password
            <input className="inp lg" type="password" autoComplete={firstRun ? 'new-password' : 'current-password'} value={pw} onChange={(e) => setPw(e.target.value)} required />
          </label>
          {firstRun && (
            <label className="field" style={{ color: 'var(--tx3)', fontSize: 12.5 }}>
              Confirm password
              <input className="inp lg" type="password" autoComplete="new-password" value={pw2} onChange={(e) => setPw2(e.target.value)} required />
            </label>
          )}
          {err && <div className="warnbox">{err}</div>}
          <button className="tb pri" style={{ height: 44, fontSize: 14 }} disabled={busy}>
            {firstRun ? 'Create admin and continue' : 'Sign in'}
          </button>
          {!firstRun && <div style={{ fontSize: 12.5, color: 'var(--mu2)', lineHeight: 1.5 }}>Stays signed in for 30 days on this device. Forgot your password? Ask an admin to reset it.</div>}
          <div className="row" style={{ padding: '10px 12px', borderRadius: 9, background: 'var(--surface)', border: '1px solid var(--line2)', fontSize: 12, color: 'var(--mu)' }}>
            <span className="dot" style={{ background: conn.color }} />
            {conn.text}
          </div>
        </form>
      </div>
    </div>
  );
}
