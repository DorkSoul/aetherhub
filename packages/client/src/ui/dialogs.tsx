// The app's own confirm and text dialogs, used instead of the browser's confirm() and prompt().
// `askConfirm` and `askText` return promises, so a call site reads like the built-in it replaces.
import { useEffect, useRef, useState, type ReactNode } from 'react';
import { create } from 'zustand';
import { Confirm, Modal } from './ui';

interface ConfirmReq {
  kind: 'confirm';
  title: string;
  text: ReactNode;
  confirm: string;
  danger?: boolean;
  done: (v: boolean) => void;
}
interface TextReq {
  kind: 'text';
  title: string;
  label: string;
  initial: string;
  confirm: string;
  done: (v: string | null) => void;
}
type Req = (ConfirmReq | TextReq) & { id: number };

let seq = 0;
const useDialogs = create<{ queue: Req[] }>(() => ({ queue: [] }));
const push = (r: ConfirmReq | TextReq) => useDialogs.setState((s) => ({ queue: [...s.queue, { ...r, id: ++seq }] }));
const pop = (id: number) => useDialogs.setState((s) => ({ queue: s.queue.filter((x) => x.id !== id) }));

/** Ask a yes/no question. Resolves true when confirmed, false when cancelled or closed. */
export function askConfirm(o: { title: string; text?: ReactNode; confirm?: string; danger?: boolean }): Promise<boolean> {
  return new Promise((done) => push({ kind: 'confirm', title: o.title, text: o.text ?? '', confirm: o.confirm ?? 'OK', danger: o.danger, done }));
}

/** Ask for a line of text. Resolves the trimmed text, or null when cancelled, closed or left empty. */
export function askText(o: { title: string; label: string; initial?: string; confirm?: string }): Promise<string | null> {
  return new Promise((done) => push({ kind: 'text', title: o.title, label: o.label, initial: o.initial ?? '', confirm: o.confirm ?? 'Save', done }));
}

function TextDialog({ r, close }: { r: TextReq; close: (v: string | null) => void }) {
  const [v, setV] = useState(r.initial);
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => {
    input.current?.focus();
    input.current?.select();
  }, []);
  const ok = () => close(v.trim() || null);
  return (
    <Modal
      title={r.title}
      onClose={() => close(null)}
      width={460}
      foot={
        <>
          <button className="tb" onClick={() => close(null)}>
            Cancel
          </button>
          <button className="tb pri" onClick={ok} disabled={!v.trim()}>
            {r.confirm}
          </button>
        </>
      }
    >
      <label className="field" style={{ padding: '16px 20px' }}>
        {r.label}
        <input
          ref={input}
          className="inp"
          value={v}
          maxLength={120}
          onChange={(e) => setV(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && v.trim()) ok();
          }}
        />
      </label>
    </Modal>
  );
}

/** Mounted once at the app root: shows the oldest open dialog. */
export function Dialogs() {
  const r = useDialogs((s) => s.queue[0]);
  if (!r) return null;
  const close = (v: boolean | string | null) => {
    pop(r.id);
    (r.done as (x: boolean | string | null) => void)(v);
  };
  if (r.kind === 'confirm') return <Confirm key={r.id} title={r.title} text={r.text} confirm={r.confirm} danger={r.danger} onYes={() => close(true)} onNo={() => close(false)} />;
  return <TextDialog key={r.id} r={r} close={close} />;
}
