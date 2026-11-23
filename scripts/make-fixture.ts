// Extracts a small card set from a Scryfall bulk file for tests and e2e runs.
// Usage: npx tsx scripts/make-fixture.ts <bulk.jsonl> [out]
import fs from 'node:fs';
import readline from 'node:readline';

const NAMES = `Atraxa, Praetors' Voice|Sol Ring|Arcane Signet|Mana Crypt|Chromatic Lantern|Contagion Engine|Tamiyo's Journal|Karn, the Great Creator|Command Tower|Karn's Bastion|Forest|Island|Plains|Swamp|Mountain|Swords to Plowshares|Teferi's Protection|Wrath of God|Grateful Apparition|Counterspell|Rhystic Study|Deepglow Skate|Flux Channeler|Tezzeret's Gambit|Cyclonic Rift|Swan Song|Inexorable Tide|Viral Drake|Demonic Tutor|Cultivate|Kodama's Reach|Farseek|Doubling Season|Hardened Scales|Evolution Sage|Pir, Imaginative Rascal|Vorinclex, Monstrous Raider|Corpsejack Menace|Lightning Bolt|Winding Constrictor|Branching Evolution|Krenko, Mob Boss|Skullclamp|Meren of Clan Nel Toth|Sakura-Tribe Elder|Eternal Witness|Sylvan Library|Talrand, Sky Summoner|Llanowar Elves|Serra Angel|Shivan Dragon|Delver of Secrets|Fire // Ice|Bushi Tenderfoot|Invasion of Zendikar|Bruna, the Fading Light|Gisela, the Broken Blade|Lurrus of the Dream-Den|Lightning Greaves|Lightning Helix|Evolving Wilds|Exotic Orchard|Relentless Rats|Seven Dwarves|Black Lotus|Brainstorm|Ponder|Dark Ritual|Thalia, Guardian of Thraben|Tymna the Weaver|Thrasios, Triton Hero|Grixis Panorama|Omnath, Locus of Creation|Llanowar Wastes|Goblin Guide|Monastery Swiftspear|Snapcaster Mage|Tarmogoyf|Ragavan, Nimble Pilferer|Emry, Lurker of the Loch|Urza's Saga|Gideon's Intervention|Opt|Consider|Thought Scour|Esper Sentinel|Dockside Extortionist|Jeweled Lotus|Nadu, Winged Wisdom|Wrenn and Six|Liliana of the Veil|Jace, the Mind Sculptor|Bitterblossom|Spellstutter Sprite|Pestermite|Ninja of the Deep Hours|Augur of Bolas|Faerie Seer|Mulldrifter|Ghostly Flicker|Rancor|Hunger of the Howlpack|Nettle Sentinel|Quirion Ranger|Skarrgan Pit-Skulk|Vines of Vastwood|Savage Swipe|Elephant Guide|Gladecover Scout|Aura Gnarlid|Chaos Orb|Grovetender Druids|Planar Bridge|Tomb of Annihilation|Undercity|Lost Mine of Phandelver|Dungeon of the Mad Mage|Tazeem|The Aether Flues|Naya|Stairs to Infinity|Chaotic Aether|Interplanar Tunnel|Mutual Destruction|Behold the Power of Destruction|Realms Befitting My Majesty|Feed the Machine|Bounce Chamber|Balloon Stand|Clown Car|Trash Bin|Pin Collection|Ticket Booth|Candy Grapple|Squirrel Stack|Space Beleren`.split('|');
const TOKENS = new Set(['Treasure', 'Clue', 'Bird', 'Goblin', 'Drake', 'Soldier', 'Spirit', 'Food', 'Insect', 'Faerie Rogue', 'Zombie', 'Elf Warrior', 'Human Soldier', 'Monarch', 'The Initiative', 'Energy Reserve']);

const [src, out = 'fixtures/cards-fixture.jsonl'] = process.argv.slice(2);
if (!src) throw new Error('usage: make-fixture <bulk.jsonl> [out]');
const want = new Set(NAMES.map((n) => n.toLowerCase()));
const per: Record<string, number> = {};
const lines: string[] = [];
const rl = readline.createInterface({ input: fs.createReadStream(src) });
for await (let l of rl) {
  l = l.trim().replace(/,$/, '');
  if (!l.startsWith('{')) continue;
  const c = JSON.parse(l);
  const name: string = c.name;
  const front = name.split(' // ')[0].toLowerCase();
  const isTok = c.layout === 'token' || c.layout === 'emblem';
  const hit = want.has(name.toLowerCase()) || want.has(front) || (isTok && TOKENS.has(name) && c.lang === 'en');
  if (!hit || c.lang !== 'en') continue;
  const k = name + (isTok ? c.type_line : '');
  per[k] = (per[k] || 0) + 1;
  if (per[k] > (isTok ? 2 : 3)) continue;
  lines.push(l);
}
fs.writeFileSync(out, lines.join('\n') + '\n');
console.log(`wrote ${lines.length} printings (${Object.keys(per).length} distinct) to ${out}`);
