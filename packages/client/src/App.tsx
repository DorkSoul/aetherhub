import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { lazy, Suspense, useEffect, useState } from 'react';
import { BrowserRouter, Navigate, Route, Routes, useNavigate } from 'react-router-dom';
import type { Me, SetupState } from '@aetherhub/shared';
import { api, ApiError } from './lib/api';
import { BASE } from './lib/base';
import { useApp } from './lib/store';
import { socket } from './lib/ws';
import { Login } from './screens/Login';
import { MainMenu } from './screens/MainMenu';
import { Dialogs } from './ui/dialogs';
import { ContextMenu, HoverPreview, Toasts } from './ui/ui';

const Settings = lazy(() => import('./screens/settings/Settings'));
const DeckList = lazy(() => import('./screens/deck/DeckList'));
const DeckEditor = lazy(() => import('./screens/deck/DeckEditor'));
const Import = lazy(() => import('./screens/deck/Import'));
const Lobby = lazy(() => import('./screens/Lobby'));
const Games = lazy(() => import('./screens/Games'));
const Draft = lazy(() => import('./screens/Draft'));
const RulesGame = lazy(() => import('./screens/rules/RulesGame'));
const Rulings = lazy(() => import('./screens/Rulings'));
const Table = lazy(() => import('./screens/table/Table'));
const Playtest = lazy(() => import('./screens/table/Playtest'));
const SoloGame = lazy(() => import('./screens/table/Playtest').then((m) => ({ default: m.SoloGame })));

export const queryClient = new QueryClient({ defaultOptions: { queries: { staleTime: 10_000, retry: 1, refetchOnWindowFocus: false } } });

function Loading() {
  return <div style={{ padding: 40, color: 'var(--mu)' }}>Loading…</div>;
}

/** Keeps global WebSocket state (status banner, lobby) in the store and follows game starts. */
function SocketBridge() {
  const nav = useNavigate();
  useEffect(() => {
    socket.onState = (c) => useApp.setState({ wsConnected: c });
    socket.connect();
    const off = socket.on((m) => {
      if (m.t === 'status') useApp.setState({ status: m.status });
      if (m.t === 'lobby') useApp.setState({ lobby: m.lobby });
      // The lobby we are in started its game: go to it.
      if (m.t === 'started') nav(m.kind === 'sandbox' ? `/table/${m.id}` : m.kind === 'rules' ? `/rules?id=${m.id}` : `/draft?id=${m.id}`);
    });
    return () => {
      off();
      socket.close();
    };
  }, [nav]);
  return null;
}

export function App() {
  const me = useApp((s) => s.me);
  const [setup, setSetup] = useState<SetupState | null>(null);
  const [checked, setChecked] = useState(false);

  useEffect(() => {
    api
      .get<SetupState>('api/setup/state')
      .then(async (s) => {
        setSetup(s);
        if (!s.needsAdmin) {
          try {
            useApp.getState().setMe(await api.get<Me>('api/me'));
          } catch (e) {
            if (!(e instanceof ApiError && e.status === 401)) throw e;
          }
        }
      })
      .catch(() => setSetup({ needsAdmin: false, serverName: 'Aetherhub', secure: location.protocol === 'https:', lan: true, insecureRemote: false }))
      .finally(() => setChecked(true));
  }, []);

  if (!checked || !setup) return <Loading />;
  if (!me)
    return (
      <>
        <Login setup={setup} onDone={(m) => useApp.getState().setMe(m)} />
        <Dialogs />
        <Toasts />
      </>
    );

  return (
    <QueryClientProvider client={queryClient}>
      <BrowserRouter basename={BASE.replace(/\/$/, '') || '/'}>
        <SocketBridge />
        <Suspense fallback={<Loading />}>
          <Routes>
            <Route path="/" element={<MainMenu />} />
            <Route path="/decks" element={<DeckList />} />
            <Route path="/decks/:id" element={<DeckEditor />} />
            <Route path="/decks/:id/playtest" element={<Playtest />} />
            <Route path="/import" element={<Import />} />
            <Route path="/lobby" element={<Lobby />} />
            <Route path="/lobby/:id" element={<Lobby />} />
            <Route path="/table" element={<Table />} />
            <Route path="/table/:id" element={<Table />} />
            <Route path="/solo" element={<SoloGame />} />
            <Route path="/rules" element={<RulesGame />} />
            <Route path="/draft" element={<Draft />} />
            <Route path="/games" element={<Games />} />
            <Route path="/rulings" element={<Rulings />} />
            <Route path="/settings" element={<Navigate to="/settings/profile" replace />} />
            <Route path="/settings/:pane" element={<Settings />} />
            <Route path="*" element={<Navigate to="/" replace />} />
          </Routes>
        </Suspense>
        <ContextMenu />
        <HoverPreview />
        <Dialogs />
        <Toasts />
      </BrowserRouter>
    </QueryClientProvider>
  );
}
