import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useState } from 'react';
import { Link } from 'react-router-dom';
import { formatName } from '@aetherhub/shared';
import { api } from '../lib/api';
import { errMsg, useApp } from '../lib/store';
import { Confirm, timeAgo } from '../ui/ui';

export interface SoloSaveSummary {
  id: number;
  name: string;
  format: string;
  turn: number;
  active: number;
  seats: { name: string; deckName: string; color: string; life: number }[];
  createdAt: number;
  updatedAt: number;
}

export function useSoloSaves() {
  return useQuery({ queryKey: ['solo-saves'], queryFn: () => api.get<SoloSaveSummary[]>('api/solo/saves') });
}

/** Saved solo games with Continue and Delete. Continue opens /solo?save=<id>. */
export function SoloSavesList({ empty }: { empty?: React.ReactNode }) {
  const saves = useSoloSaves();
  const qc = useQueryClient();
  const toast = useApp((s) => s.toast);
  const [del, setDel] = useState<SoloSaveSummary | null>(null);
  if (!saves.data) return null;
  if (!saves.data.length) return <>{empty ?? null}</>;
  return (
    <div className="col" style={{ gap: 10 }}>
      {saves.data.map((s) => (
        <div key={s.id} className="row" style={{ gap: 16, padding: '14px 18px', borderRadius: 12, background: 'var(--surface)', border: '1px solid var(--line2)', flexWrap: 'wrap' }}>
          <div className="grow" style={{ minWidth: 220 }}>
            <div className="display" style={{ fontSize: 17 }}>
              {s.name}
            </div>
            <div style={{ fontSize: 12.5, color: 'var(--mu)', marginTop: 3 }}>
              Solo · {formatName(s.format)} · turn {s.turn} · {s.seats.length} seats · saved {timeAgo(s.updatedAt)}
            </div>
            <div className="row" style={{ gap: 6, marginTop: 8, flexWrap: 'wrap' }}>
              {s.seats.map((p, i) => (
                <span key={i} className="row" style={{ gap: 5, fontSize: 12, padding: '2px 8px', borderRadius: 20, background: 'var(--surface2)', border: `1px solid ${i === s.active ? p.color : 'var(--line)'}` }}>
                  <span style={{ width: 8, height: 8, borderRadius: 4, background: p.color }} />
                  {p.deckName || p.name} · ♥ {p.life}
                </span>
              ))}
            </div>
          </div>
          <button className="tb danger" onClick={() => setDel(s)}>
            Delete
          </button>
          <Link to={`/solo?save=${s.id}`} className="tb pri">
            Continue
          </Link>
        </div>
      ))}
      {del && (
        <Confirm
          title="Delete this saved game?"
          text={
            <>
              <strong style={{ color: 'var(--tx)' }}>{del.name}</strong> will be permanently deleted. This can't be undone.
            </>
          }
          confirm="Delete"
          danger
          onNo={() => setDel(null)}
          onYes={async () => {
            try {
              await api.del(`api/solo/saves/${del.id}`);
              qc.invalidateQueries({ queryKey: ['solo-saves'] });
            } catch (e) {
              toast(errMsg(e));
            }
            setDel(null);
          }}
        />
      )}
    </div>
  );
}
