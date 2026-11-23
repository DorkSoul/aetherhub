import { useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';

export interface SelectOption<T extends string | number> {
  value: T;
  label: string;
  /** a heading the option is listed under (options of one group should be next to each other) */
  group?: string;
  disabled?: boolean;
  /** a second, dimmer line under the label */
  hint?: string;
  /** tooltip */
  title?: string;
  /** drawn before the label (a colour dot, a mana symbol) */
  icon?: React.ReactNode;
}

export interface SelectProps<T extends string | number> {
  value: T;
  options: SelectOption<T>[];
  onChange: (v: T) => void;
  'aria-label'?: string;
  /** shown when no option matches `value` */
  placeholder?: string;
  disabled?: boolean;
  className?: string;
  style?: React.CSSProperties;
  /** a search box at the top of the list; on by default for lists longer than 12 */
  searchable?: boolean;
  /** min width of the open list (defaults to the button's width) */
  menuWidth?: number;
  title?: string;
}

/**
 * The app's own dropdown, used instead of the browser's <select>: a button styled like the inputs and a list in the
 * app's panel colours. Keyboard: arrows / Home / End move, Enter or Space picks, Esc closes, typing jumps to the
 * first option starting with what was typed (or filters, when the list has a search box).
 */
export function Select<T extends string | number>(p: SelectProps<T>) {
  const [open, setOpen] = useState(false);
  const btn = useRef<HTMLButtonElement>(null);
  const current = p.options.find((o) => o.value === p.value);
  const id = useId();
  const close = (refocus = true) => {
    setOpen(false);
    if (refocus) btn.current?.focus();
  };
  const step = (d: number) => {
    const enabled = p.options.filter((o) => !o.disabled);
    if (!enabled.length) return;
    const i = enabled.findIndex((o) => o.value === p.value);
    const n = enabled[Math.max(0, Math.min(enabled.length - 1, i < 0 ? 0 : i + d))];
    if (n && n.value !== p.value) p.onChange(n.value);
  };
  return (
    <>
      <button
        ref={btn}
        type="button"
        className={'sel csel' + (p.className ? ' ' + p.className : '')}
        style={p.style}
        disabled={p.disabled}
        title={p.title ?? current?.title}
        aria-label={p['aria-label']}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={open ? id : undefined}
        onClick={() => setOpen((o) => !o)}
        onKeyDown={(e) => {
          if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
            e.preventDefault();
            if (e.altKey || open) setOpen(true);
            else step(e.key === 'ArrowDown' ? 1 : -1);
          } else if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault();
            setOpen(true);
          }
        }}
      >
        {current?.icon}
        <span className="csel-label">{current ? current.label : (p.placeholder ?? '')}</span>
        <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.4" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" style={{ flexShrink: 0, opacity: 0.7, transform: open ? 'rotate(180deg)' : undefined, transition: 'transform .12s' }}>
          <path d="M6 9l6 6 6-6" />
        </svg>
      </button>
      {open && btn.current && (
        <SelectMenu
          id={id}
          anchor={btn.current}
          options={p.options}
          value={p.value}
          searchable={p.searchable ?? p.options.length > 12}
          menuWidth={p.menuWidth}
          onPick={(v) => {
            if (v !== p.value) p.onChange(v);
            close();
          }}
          onClose={close}
        />
      )}
    </>
  );
}

function SelectMenu<T extends string | number>({ id, anchor, options, value, searchable, menuWidth, onPick, onClose }: { id: string; anchor: HTMLElement; options: SelectOption<T>[]; value: T; searchable: boolean; menuWidth?: number; onPick: (v: T) => void; onClose: (refocus?: boolean) => void }) {
  const panel = useRef<HTMLDivElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const search = useRef<HTMLInputElement>(null);
  const [q, setQ] = useState('');
  const shown = useMemo(() => {
    const t = q.trim().toLowerCase();
    return t ? options.filter((o) => o.label.toLowerCase().includes(t) || (o.group ?? '').toLowerCase().includes(t)) : options;
  }, [options, q]);
  const [hi, setHi] = useState(() => Math.max(0, options.findIndex((o) => o.value === value)));
  const [pos, setPos] = useState<{ left: number; top: number; width: number; maxH: number; up: boolean } | null>(null);
  const typed = useRef({ text: '', at: 0 });

  // Place under the button, or above it when there is more room there; follow it while the page scrolls.
  useLayoutEffect(() => {
    const place = () => {
      const r = anchor.getBoundingClientRect();
      const vh = window.innerHeight;
      const vw = window.innerWidth;
      const below = vh - r.bottom - 10;
      const above = r.top - 10;
      const up = below < 220 && above > below;
      // at least as wide as the button, wider when the labels need it; kept inside the window
      const minW = Math.min(vw - 16, Math.max(r.width, menuWidth ?? 0));
      const w = Math.max(minW, panel.current?.offsetWidth ?? 0);
      setPos({ left: Math.max(8, Math.min(r.left, vw - w - 8)), top: up ? r.top - 4 : r.bottom + 4, width: minW, maxH: Math.min(360, up ? above : below), up });
    };
    place();
    window.addEventListener('resize', place);
    window.addEventListener('scroll', place, true);
    return () => {
      window.removeEventListener('resize', place);
      window.removeEventListener('scroll', place, true);
    };
  }, [anchor, menuWidth]);

  useEffect(() => {
    const down = (e: MouseEvent) => {
      if (!panel.current?.contains(e.target as Node) && !anchor.contains(e.target as Node)) onClose(false);
    };
    window.addEventListener('mousedown', down);
    return () => window.removeEventListener('mousedown', down);
  }, [anchor, onClose]);

  useEffect(() => {
    (searchable ? search.current : list.current)?.focus({ preventScroll: true });
  }, [searchable]);

  useEffect(() => {
    setHi((h) => (q ? shown.findIndex((o) => !o.disabled) : h));
  }, [q, shown]);

  // keep the highlighted option in view
  useEffect(() => {
    const el = list.current?.querySelector<HTMLElement>(`[data-i="${hi}"]`);
    el?.scrollIntoView({ block: 'nearest' });
  }, [hi, pos]);

  const move = (d: number) => {
    if (!shown.length) return;
    let i = hi;
    for (let k = 0; k < shown.length; k++) {
      i = Math.max(0, Math.min(shown.length - 1, i + d));
      if (!shown[i].disabled) break;
    }
    setHi(i);
  };
  const onKey = (e: React.KeyboardEvent) => {
    if (e.key === 'ArrowDown') (e.preventDefault(), move(1));
    else if (e.key === 'ArrowUp') (e.preventDefault(), move(-1));
    else if (e.key === 'Home') (e.preventDefault(), setHi(shown.findIndex((o) => !o.disabled)));
    else if (e.key === 'End') (e.preventDefault(), setHi(shown.length - 1 - [...shown].reverse().findIndex((o) => !o.disabled)));
    else if (e.key === 'PageDown') (e.preventDefault(), move(8));
    else if (e.key === 'PageUp') (e.preventDefault(), move(-8));
    else if (e.key === 'Enter' || (e.key === ' ' && !searchable)) {
      e.preventDefault();
      const o = shown[hi];
      if (o && !o.disabled) onPick(o.value);
    } else if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      onClose();
    } else if (e.key === 'Tab') onClose(false);
    else if (!searchable && e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey) {
      // type-ahead
      const now = Date.now();
      typed.current = { text: (now - typed.current.at < 700 ? typed.current.text : '') + e.key.toLowerCase(), at: now };
      const i = shown.findIndex((o) => !o.disabled && o.label.toLowerCase().startsWith(typed.current.text));
      if (i >= 0) setHi(i);
    }
  };

  let lastGroup: string | undefined;
  return createPortal(
    <div
      ref={panel}
      className="csel-menu"
      style={pos ? { left: pos.left, minWidth: pos.width, ...(pos.up ? { bottom: window.innerHeight - pos.top } : { top: pos.top }), maxHeight: pos.maxH } : { left: 0, top: 0, visibility: 'hidden' }}
      onKeyDown={onKey}
    >
      {searchable && <input ref={search} className="inp csel-search" placeholder="Search…" aria-label="Search options" value={q} onChange={(e) => setQ(e.target.value)} />}
      <div ref={list} id={id} role="listbox" tabIndex={-1} className="csel-list" aria-activedescendant={shown[hi] ? `${id}-${hi}` : undefined}>
        {shown.map((o, i) => {
          const head = o.group !== lastGroup && o.group ? o.group : null;
          lastGroup = o.group;
          return (
            <div key={String(o.value)}>
              {head && <div className="csel-group">{head}</div>}
              <div
                id={`${id}-${i}`}
                data-i={i}
                role="option"
                aria-selected={o.value === value}
                aria-disabled={o.disabled || undefined}
                title={o.title}
                className={'csel-opt' + (i === hi ? ' hi' : '') + (o.value === value ? ' on' : '') + (o.disabled ? ' dis' : '') + (o.group ? ' ind' : '')}
                onMouseMove={() => !o.disabled && i !== hi && setHi(i)}
                onClick={() => !o.disabled && onPick(o.value)}
              >
                {o.icon}
                <span className="grow" style={{ minWidth: 0 }}>
                  <span style={{ display: 'block', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{o.label}</span>
                  {o.hint && <span className="csel-hint">{o.hint}</span>}
                </span>
                {o.value === value && (
                  <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.6" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" style={{ flexShrink: 0 }}>
                    <path d="M5 12l5 5 9-10" />
                  </svg>
                )}
              </div>
            </div>
          );
        })}
        {!shown.length && <div className="csel-empty">Nothing matches “{q}”</div>}
      </div>
    </div>,
    document.body,
  );
}
