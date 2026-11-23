// Room lists for the dungeon cards. `next` holds the indices a room leads to.
export interface Room {
  name: string;
  next: number[];
}

export const DUNGEONS: Record<string, Room[]> = {
  'Lost Mine of Phandelver': [
    { name: 'Cave Entrance', next: [1, 2] },
    { name: 'Goblin Lair', next: [3, 4] },
    { name: 'Mine Tunnels', next: [4, 5] },
    { name: 'Storeroom', next: [6] },
    { name: 'Dark Pool', next: [6] },
    { name: 'Fungi Cavern', next: [6] },
    { name: 'Temple of Dumathoin', next: [] },
  ],
  'Dungeon of the Mad Mage': [
    { name: 'Yawning Portal', next: [1] },
    { name: 'Dungeon Level', next: [2, 3] },
    { name: 'Goblin Bazaar', next: [4] },
    { name: 'Twisted Caverns', next: [4] },
    { name: 'Lost Level', next: [5, 6] },
    { name: 'Runestone Caverns', next: [7] },
    { name: "Muiral's Graveyard", next: [7] },
    { name: 'Deep Mines', next: [8] },
    { name: "Mad Wizard's Lair", next: [] },
  ],
  'Tomb of Annihilation': [
    { name: 'Trapped Entry', next: [1, 2] },
    { name: 'Veils of Fear', next: [3] },
    { name: 'Oubliette', next: [4] },
    { name: 'Sandfall Cell', next: [4] },
    { name: 'Cradle of the Death God', next: [] },
  ],
  Undercity: [
    { name: 'Secret Entrance', next: [1, 2] },
    { name: 'Forge', next: [3, 4] },
    { name: 'Lost Well', next: [4, 5] },
    { name: 'Trap!', next: [6] },
    { name: 'Arena', next: [6, 7] },
    { name: 'Stash', next: [7] },
    { name: 'Archives', next: [8] },
    { name: 'Catacombs', next: [8] },
    { name: 'Throne of the Dead Three', next: [] },
  ],
};

export const DUNGEON_NAMES = Object.keys(DUNGEONS);
