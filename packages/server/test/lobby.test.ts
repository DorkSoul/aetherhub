import { describe, expect, it } from 'vitest';
import { gameTypeLabel, rulesMode, typePatch } from '@aetherhub/shared';
import { Lobby } from '../src/games/lobby';

const host = { id: 1, username: 'host', isAdmin: false };
const make = (settings: Record<string, unknown>) => new Lobby(host.id, { name: 'host', color: '#fff' }, settings, null);

describe('lobby game type and rules', () => {
  it('splits the game type into kind and rules mode', () => {
    expect(rulesMode({ type: 'sandbox' })).toBe('sandbox');
    expect(rulesMode({ type: 'rules' })).toBe('rules');
    // events from before the choice existed were rules enforced
    expect(rulesMode({ type: 'draft' })).toBe('rules');
    expect(rulesMode({ type: 'sealed', enforced: false })).toBe('sandbox');
    expect(typePatch('constructed', 'rules')).toEqual({ type: 'rules' });
    expect(typePatch('draft', 'sandbox')).toEqual({ type: 'draft', enforced: false });
    expect(gameTypeLabel({ type: 'draft', enforced: false })).toBe('Draft · sandbox');
  });

  it('a sandbox draft takes no AI drafters; a rules enforced one does', () => {
    const l = make({ type: 'draft', enforced: false, maxPlayers: 3 });
    expect(() => l.sit(host, 1, { ai: 'Normal' })).toThrow(/rules enforced/);
    l.updateSettings(host, { enforced: true });
    l.sit(host, 1, { ai: 'Normal' });
    expect(l.slots[1].kind).toBe('ai');
  });

  it('switching an event to sandbox with AIs seated needs removeAi, which empties their seats', () => {
    const l = make({ type: 'sealed', maxPlayers: 3 });
    l.sit(host, 1, { ai: 'Normal' });
    expect(() => l.updateSettings(host, { enforced: false })).toThrow(/rules enforced/);
    l.updateSettings(host, { enforced: false, removeAi: true });
    expect(l.slots[1].kind).toBe('open');
    expect(rulesMode(l.settings)).toBe('sandbox');
  });

  it('changing the kind keeps the rules mode the client sends', () => {
    const l = make({ type: 'rules', maxPlayers: 3 });
    l.sit(host, 1, { ai: 'Normal' });
    l.updateSettings(host, typePatch('draft', 'rules'));
    expect(l.slots[1].kind).toBe('ai');
    expect(rulesMode(l.settings)).toBe('rules');
  });

  it('the draft pick timer is off unless the host turns it on', () => {
    expect(make({ type: 'draft' }).settings.pickTimer).toBeFalsy();
  });
});
