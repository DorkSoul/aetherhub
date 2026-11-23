import { useNavigate } from 'react-router-dom';
import { api } from '../lib/api';
import { useApp } from '../lib/store';

/** End-of-game buttons: back to the game's lobby (re-opened as it was if it had closed), or the main menu (leaving that
 *  lobby, which closes when nobody else is in it). */
export function AfterGameButtons({ gameId, small }: { gameId: string; small?: boolean }) {
  const nav = useNavigate();
  const cls = small ? 'tb sm' : 'tb';
  const back = async () => {
    try {
      await api.post<{ lobbyId: string }>(`api/games/${gameId}/back-to-lobby`, {});
      nav('/lobby');
    } catch (e) {
      useApp.getState().toast((e as Error).message);
    }
  };
  const menu = async () => {
    await api.post(`api/games/${gameId}/leave-lobby`, {}).catch(() => {});
    nav('/');
  };
  return (
    <>
      <button className={cls + ' pri'} onClick={() => void back()}>
        Back to lobby
      </button>
      <button className={cls} onClick={() => void menu()}>
        Main menu
      </button>
    </>
  );
}
