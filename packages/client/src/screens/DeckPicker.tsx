import { useEffect, useState } from 'react';
import { createPortal } from 'react-dom';
import { DeckView } from './deck/DeckEditor';
import { DeckLibrary } from './deck/DeckList';

/**
 * Full-screen deck picker for a lobby seat: the deck library (all decks, pinned, folders, others' decks, search and
 * filters) with Select deck on each deck; opening a deck shows it read-only with Select deck and Sample hand.
 */
export function DeckPicker({ title, format, onSelect, onClose }: { title: string; format?: string; onSelect: (deckId: number) => void; onClose: () => void }) {
  const [open, setOpen] = useState<number | null>(null);
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (e.key !== 'Escape' || document.querySelector('.csel-menu, .scrim:not(.deck-picker)')) return;
      if (open != null) setOpen(null);
      else onClose();
    };
    window.addEventListener('keydown', key);
    return () => window.removeEventListener('keydown', key);
  }, [open, onClose]);
  return createPortal(
    <div className="deck-picker" role="dialog" aria-label={title} style={{ position: 'fixed', inset: 0, zIndex: 1000, background: 'var(--bg)', display: 'flex', flexDirection: 'column' }}>
      {open != null ? (
        <DeckView deckId={open} pick={{ onSelect: (d) => onSelect(d.id), onBack: () => setOpen(null) }} />
      ) : (
        <DeckLibrary pick={{ title, format, onSelect: (d) => onSelect(d.id), onOpen: setOpen, onClose }} />
      )}
    </div>,
    document.body,
  );
}
