import { useEffect, useState } from 'react';
import type { CardDef, CardView, GameView } from '@aetherhub/shared';
import { formatName, turnKind } from '@aetherhub/shared';
import { RulingsList } from '../../ui/rulings';
import { CardImage, Modal, TurnButton } from '../../ui/ui';
import { ManaCostText, RichLabel } from '../rules/mana';
import { cardLabel, detailsCard, locateView } from './menus';
import { useTable } from './store';

const close = () => useTable.setState({ details: null });

/** The full card for the player who opened it: every face's text, its state on the table, and its rulings. Local only. */
export function CardDetailsModal() {
  const t = useTable((s) => s.details);
  const v = useTable((s) => s.view);
  const shown = t && v ? detailsCard(v, t) : null;
  // a card that is turned face down or moved out of sight closes the window
  useEffect(() => {
    if (t && !shown) close();
  }, [t, shown]);
  if (!t || !v || !shown) return null;
  return <Details key={t.iid ?? t.def?.key} v={v} def={shown.def} card={shown.card} />;
}

function Details({ v, def, card }: { v: GameView; def: CardDef; card?: CardView }) {
  const [face, setFace] = useState(card?.face ? 1 : 0);
  const kind = turnKind(def);
  const faces = def.faces && def.faces.length > 1 ? def.faces : [def];
  const fmt = v.settings.format;
  const legal = def.legalities?.[fmt];
  const print = [def.setName, def.collectorNumber && '#' + def.collectorNumber, def.rarity].filter(Boolean).join(' · ');
  return (
    <Modal title={card ? cardLabel(v, card) : def.name} onClose={close} width={860}>
      <div className="row" style={{ alignItems: 'stretch', flexWrap: 'wrap' }}>
        <div style={{ width: 310, background: 'var(--panel)', padding: 22, display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 12 }}>
          <div style={{ position: 'relative' }}>
            <CardImage def={def} face={face} size="large" style={{ width: 260, height: 363, borderRadius: 12, boxShadow: '0 12px 30px rgba(0,0,0,.5)' }} />
            <TurnButton def={def} onTurn={() => setFace(face ? 0 : 1)} size={34} />
          </div>
          {kind && (
            <button className="tb sm" onClick={() => setFace(face ? 0 : 1)}>
              ⟲ {kind === 'flip' ? (face ? 'Rotate back' : 'Rotate 180°') : kind === 'meld' ? (face ? 'Show front' : 'Show melded card') : `Turn to ${face ? 'front' : 'back'}`}
            </button>
          )}
          {print && <div style={{ fontSize: 11.5, color: 'var(--mu2)', textAlign: 'center' }}>{print}</div>}
        </div>
        <div className="col" style={{ flex: '1 1 380px', padding: '20px 22px', gap: 16, minWidth: 0 }}>
          {faces.map((f, i) => (
            <div key={i} className="col" style={{ gap: 6 }}>
              {faces.length > 1 && (
                <div className="row" style={{ gap: 10, alignItems: 'baseline' }}>
                  <span className="display grow" style={{ fontSize: 17 }}>
                    {f.name}
                  </span>
                  {f.manaCost && <ManaCostText cost={f.manaCost} size={17} />}
                </div>
              )}
              {faces.length === 1 && f.manaCost && (
                <div>
                  <ManaCostText cost={f.manaCost} size={18} />
                </div>
              )}
              <div style={{ fontSize: 12.5, color: 'var(--mu)' }}>{f.typeLine}</div>
              {f.oracleText && (
                <div className="col" style={{ gap: 6, fontSize: 13.5, lineHeight: 1.5, color: 'var(--tx2)' }}>
                  {f.oracleText.split('\n').map((line, j) => (
                    <div key={j}>
                      <RichLabel text={line} size={14} />
                    </div>
                  ))}
                </div>
              )}
              <Stats p={f.power} t={f.toughness} loyalty={f.loyalty} defense={f.defense} />
            </div>
          ))}
          {card && <OnTable v={v} c={card} />}
          {legal && fmt && (
            <div style={{ fontSize: 12, color: legal === 'legal' ? 'var(--mu)' : '#e0a84f' }}>
              {formatName(fmt)}: {legal === 'legal' ? 'legal' : legal.replace(/_/g, ' ')}
            </div>
          )}
          {!def.custom && def.oracleId && <RulingsList oracleId={def.oracleId} maxHeight={360} link={false} />}
        </div>
      </div>
    </Modal>
  );
}

function Stats({ p, t, loyalty, defense }: { p?: string; t?: string; loyalty?: string; defense?: string }) {
  const parts = [p != null && t != null ? `${p}/${t}` : null, loyalty != null ? `Loyalty ${loyalty}` : null, defense != null ? `Defense ${defense}` : null].filter(Boolean);
  if (!parts.length) return null;
  return <div style={{ fontSize: 13, fontWeight: 600, color: 'var(--tx3)' }}>{parts.join(' · ')}</div>;
}

/** What is on the card right now: who has it, counters, effects, modifiers. */
function OnTable({ v, c }: { v: GameView; c: CardView }) {
  const loc = locateView(v, c.iid);
  const rows: string[] = [];
  if (loc && loc.seat !== c.owner && loc.zone === 'battlefield') rows.push(`Controlled by ${v.seats[loc.seat]?.name} · owned by ${v.seats[c.owner]?.name}`);
  const state = [c.tapped && 'tapped', c.faceDown && 'face down', c.noUntap && "doesn't untap", c.token && 'token'].filter(Boolean);
  if (loc?.zone === 'battlefield' && state.length) rows.push(state.join(' · '));
  const ctrs = Object.entries(c.counters).filter(([, n]) => n);
  if (ctrs.length) rows.push('Counters: ' + ctrs.map(([k, n]) => `${k} ×${n}`).join(', '));
  if (c.effects.length) rows.push('Effects: ' + c.effects.map((e) => e.k + (e.eot ? ' (until end of turn)' : '')).join(', '));
  const mod = (x: { p: number; t: number }) => `${x.p >= 0 ? '+' : ''}${x.p}/${x.t >= 0 ? '+' : ''}${x.t}`;
  if (c.pt.p || c.pt.t) rows.push('P/T modifier: ' + mod(c.pt));
  if (c.ptEot.p || c.ptEot.t) rows.push('Until end of turn: ' + mod(c.ptEot));
  if (c.attachedTo && v.cards[c.attachedTo]) rows.push('Attached to ' + cardLabel(v, v.cards[c.attachedTo]));
  if (!rows.length) return null;
  return (
    <div style={{ fontSize: 12.5, color: 'var(--acc-tx)', lineHeight: 1.6, borderTop: '1px solid var(--line)', paddingTop: 10 }}>
      {rows.map((r, i) => (
        <div key={i}>{r}</div>
      ))}
    </div>
  );
}
