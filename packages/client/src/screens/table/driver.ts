import type { CardDef, CardInst, LogView, Notice } from '@aetherhub/shared';
import { flashTitle, play } from '../../lib/sound';
import { useApp } from '../../lib/store';
import { useMenu } from '../../ui/ui';
import { mySeat, useTable } from './store';

function alerts() {
  return useApp.getState().me?.prefs.alerts;
}

/** Reacts to a server (or local engine) notice. */
export function handleNotice(n: Notice, cards: Record<string, CardInst> = {}, defs: Record<string, CardDef> = {}) {
  const st = useTable.getState();
  const me = mySeat(st.view);
  const toast = useApp.getState().toast;
  switch (n.kind) {
    case 'toast':
      toast(n.text, n.color);
      break;
    case 'sound':
      if (alerts()?.actions) play(n.sound);
      break;
    case 'commanderPrompt':
      if (n.to === me && (!st.modal || st.modal.kind === 'zone')) useTable.setState({ modal: { kind: 'commander', iid: n.iid } });
      break;
    case 'cards': {
      const data = { notice: n, cards, defs };
      if (n.mode === 'reveal' && n.to === 'all') useTable.setState({ reveal: data });
      else useTable.setState({ modal: { kind: 'cards', data } });
      break;
    }
    case 'handRequest':
      if (alerts()?.requests) play('request');
      toast(`${st.view?.seats[n.from]?.name} asks to see your hand`);
      break;
    case 'vancouverScry':
      if (me != null) st.send({ type: 'look', seat: me, n: n.n ?? 1, mode: 'scry' });
      break;
    case 'yourTurn':
      if (n.seat === me) {
        if (alerts()?.turn) {
          play('turn');
          flashTitle('Your turn!');
        }
        toast('Your turn');
      }
      break;
  }
}

export function onNewLog(entries: LogView[]) {
  const st = useTable.getState();
  const me = mySeat(st.view);
  if (entries.some((e) => e.kind === 'chat' && e.actor !== me) && alerts()?.chat) play('chat');
  useTable.setState({ log: [...st.log, ...entries].slice(-1500) });
}

export function refreshMenus() {
  if (useMenu.getState().build) useMenu.getState().refresh();
}
