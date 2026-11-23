use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Copy, Clone, Eq, PartialEq, Hash, Ord, PartialOrd, Debug, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export)]
pub enum Color {
    W,
    U,
    B,
    R,
    G,
}

impl Color {
    pub const ALL: [Color; 5] = [Color::W, Color::U, Color::B, Color::R, Color::G];
    pub fn bit(self) -> u8 {
        1 << (self as u8)
    }
    pub fn from_char(c: char) -> Option<Color> {
        Some(match c {
            'W' => Color::W,
            'U' => Color::U,
            'B' => Color::B,
            'R' => Color::R,
            'G' => Color::G,
            _ => return None,
        })
    }
    pub fn name(self) -> &'static str {
        ["white", "blue", "black", "red", "green"][self as usize]
    }
}

/// A set of colors as a bit mask (W=1 U=2 B=4 R=8 G=16).
#[derive(Copy, Clone, Eq, PartialEq, Hash, Default, Debug, Serialize, Deserialize)]
pub struct ColorSet(pub u8);

impl ColorSet {
    pub const NONE: ColorSet = ColorSet(0);
    pub fn of(cs: &[Color]) -> ColorSet {
        ColorSet(cs.iter().fold(0, |a, c| a | c.bit()))
    }
    pub fn has(self, c: Color) -> bool {
        self.0 & c.bit() != 0
    }
    pub fn add(&mut self, c: Color) {
        self.0 |= c.bit();
    }
    pub fn count(self) -> u32 {
        self.0.count_ones()
    }
    pub fn is_colorless(self) -> bool {
        self.0 == 0
    }
    pub fn intersects(self, o: ColorSet) -> bool {
        self.0 & o.0 != 0
    }
}

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export)]
pub enum CardType {
    Artifact,
    Battle,
    Creature,
    Enchantment,
    Instant,
    Kindred,
    Land,
    Planeswalker,
    Sorcery,
    Dungeon,
    Plane,
    Phenomenon,
    Scheme,
    Vanguard,
    Conspiracy,
    Attraction,
    Emblem,
}

impl CardType {
    pub fn bit(self) -> u32 {
        1 << (self as u32)
    }
    pub fn name(self) -> &'static str {
        match self {
            CardType::Artifact => "Artifact",
            CardType::Battle => "Battle",
            CardType::Creature => "Creature",
            CardType::Enchantment => "Enchantment",
            CardType::Instant => "Instant",
            CardType::Kindred => "Kindred",
            CardType::Land => "Land",
            CardType::Planeswalker => "Planeswalker",
            CardType::Sorcery => "Sorcery",
            CardType::Dungeon => "Dungeon",
            CardType::Plane => "Plane",
            CardType::Phenomenon => "Phenomenon",
            CardType::Scheme => "Scheme",
            CardType::Vanguard => "Vanguard",
            CardType::Conspiracy => "Conspiracy",
            CardType::Attraction => "Attraction",
            CardType::Emblem => "Emblem",
        }
    }
}

#[derive(Copy, Clone, Eq, PartialEq, Hash, Default, Debug, Serialize, Deserialize)]
pub struct TypeSet(pub u32);

impl TypeSet {
    pub fn of(ts: &[CardType]) -> TypeSet {
        TypeSet(ts.iter().fold(0, |a, t| a | t.bit()))
    }
    pub fn has(self, t: CardType) -> bool {
        self.0 & t.bit() != 0
    }
    pub fn add(&mut self, t: CardType) {
        self.0 |= t.bit();
    }
    pub fn remove(&mut self, t: CardType) {
        self.0 &= !t.bit();
    }
    pub fn intersects(self, o: TypeSet) -> bool {
        self.0 & o.0 != 0
    }
    /// Permanent types (a card with any of these can be on the battlefield).
    pub fn is_permanent(self) -> bool {
        self.intersects(TypeSet::of(&[
            CardType::Artifact,
            CardType::Battle,
            CardType::Creature,
            CardType::Enchantment,
            CardType::Land,
            CardType::Planeswalker,
        ]))
    }
}

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export)]
pub enum Supertype {
    Basic,
    Legendary,
    Snow,
    World,
    Ongoing,
}

/// Interned subtype / name-like string (equality only; ids are process-local).
#[derive(Copy, Clone, Eq, PartialEq, Hash, Ord, PartialOrd, Debug, Serialize, Deserialize)]
pub struct Sym(pub u16);

mod interner {
    use super::Sym;
    use std::collections::HashMap;
    use std::sync::{OnceLock, RwLock};

    #[derive(Default)]
    struct Table {
        map: HashMap<String, u16>,
        names: Vec<String>,
    }
    fn table() -> &'static RwLock<Table> {
        static T: OnceLock<RwLock<Table>> = OnceLock::new();
        T.get_or_init(|| RwLock::new(Table::default()))
    }
    pub fn intern(s: &str) -> Sym {
        if let Some(&i) = table().read().unwrap().map.get(s) {
            return Sym(i);
        }
        let mut t = table().write().unwrap();
        if let Some(&i) = t.map.get(s) {
            return Sym(i);
        }
        let i = t.names.len() as u16;
        t.names.push(s.to_string());
        t.map.insert(s.to_string(), i);
        Sym(i)
    }
    pub fn name(s: Sym) -> String {
        table().read().unwrap().names[s.0 as usize].clone()
    }
}

impl Sym {
    pub fn new(s: &str) -> Sym {
        interner::intern(s)
    }
    pub fn name(self) -> String {
        interner::name(self)
    }
}

/// Keywords with no parameters (a bit each in `KeywordSet`).
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export)]
pub enum Keyword {
    Flying,
    FirstStrike,
    DoubleStrike,
    Deathtouch,
    Haste,
    Hexproof,
    Indestructible,
    Lifelink,
    Menace,
    Reach,
    Trample,
    Vigilance,
    Defender,
    Flash,
    Shroud,
    Fear,
    Intimidate,
    Skulk,
    Infect,
    Wither,
    Prowess,
    Undying,
    Persist,
    Cascade,
    Changeling,
    Horsemanship,
    Shadow,
    Flanking,
    Provoke,
    Annihilator1,
    Storm,
    Convoke,
    Delve,
    Improvise,
    /// Assist: another player may pay up to the generic part of this spell's cost.
    Assist,
    Cipher,
    SplitSecond,
    Unblockable,
    CantBlock,
    CantAttack,
    CantBeCountered,
    MustAttack,
    Phasing,
    Banding,
    /// "Bands with other legendary creatures": legendary creatures may attack in a band as long as one of them has this (Seafarer's Quay).
    BandsWithLegendary,
    /// "Bands with other creatures named <its own name>": creatures with the same name may attack in a band as long as one has this (Master of the Hunt's Wolves of the Hunt).
    BandsWithSameName,
    Rebound,
    Exalted,
    Soulbond,
    LivingWeapon,
    Fuse,
    Undaunted,
    Myriad,
    Battlecry,
    Ravenous,
    Cycling,
    Training,
    Evolve,
    Exploit,
    Extort,
    Haunt,
    Dethrone,
    Afterlife1,
    Mentor,
    Riot,
    Decayed,
    Toxic1,
    Toxic2,
    Toxic3,
    Toxic4,
    Toxic5,
    Toxic9,
    Backup,
    Reconfigure,
    Blitz,
    Disturb,
    Enlist,
    Squad,
    Offspring,
    Gift,
    Plainswalk,
    Islandwalk,
    Swampwalk,
    Mountainwalk,
    Forestwalk,
    Daybound,
    Nightbound,
}

impl Keyword {
    /// Every keyword, in declaration order.
    pub const ALL: &'static [Keyword] = &[
        Keyword::Flying,
        Keyword::FirstStrike,
        Keyword::DoubleStrike,
        Keyword::Deathtouch,
        Keyword::Haste,
        Keyword::Hexproof,
        Keyword::Indestructible,
        Keyword::Lifelink,
        Keyword::Menace,
        Keyword::Reach,
        Keyword::Trample,
        Keyword::Vigilance,
        Keyword::Defender,
        Keyword::Flash,
        Keyword::Shroud,
        Keyword::Fear,
        Keyword::Intimidate,
        Keyword::Skulk,
        Keyword::Infect,
        Keyword::Wither,
        Keyword::Prowess,
        Keyword::Undying,
        Keyword::Persist,
        Keyword::Cascade,
        Keyword::Changeling,
        Keyword::Horsemanship,
        Keyword::Shadow,
        Keyword::Flanking,
        Keyword::Provoke,
        Keyword::Annihilator1,
        Keyword::Storm,
        Keyword::Convoke,
        Keyword::Delve,
        Keyword::Improvise,
        Keyword::Assist,
        Keyword::Cipher,
        Keyword::SplitSecond,
        Keyword::Unblockable,
        Keyword::CantBlock,
        Keyword::CantAttack,
        Keyword::CantBeCountered,
        Keyword::MustAttack,
        Keyword::Phasing,
        Keyword::Banding,
        Keyword::BandsWithLegendary,
        Keyword::BandsWithSameName,
        Keyword::Rebound,
        Keyword::Exalted,
        Keyword::Soulbond,
        Keyword::LivingWeapon,
        Keyword::Fuse,
        Keyword::Undaunted,
        Keyword::Myriad,
        Keyword::Battlecry,
        Keyword::Cycling,
        Keyword::Training,
        Keyword::Evolve,
        Keyword::Exploit,
        Keyword::Extort,
        Keyword::Haunt,
        Keyword::Dethrone,
        Keyword::Afterlife1,
        Keyword::Mentor,
        Keyword::Riot,
        Keyword::Toxic1,
        Keyword::Toxic2,
        Keyword::Toxic3,
        Keyword::Toxic4,
        Keyword::Toxic5,
        Keyword::Toxic9,
        Keyword::Backup,
        Keyword::Reconfigure,
        Keyword::Blitz,
        Keyword::Disturb,
        Keyword::Enlist,
        Keyword::Squad,
        Keyword::Offspring,
        Keyword::Gift,
        Keyword::Plainswalk,
        Keyword::Islandwalk,
        Keyword::Swampwalk,
        Keyword::Mountainwalk,
        Keyword::Forestwalk,
        Keyword::Daybound,
        Keyword::Nightbound,
    ];

    pub fn bit(self) -> u128 {
        1u128 << (self as u32)
    }
}

#[derive(Copy, Clone, Eq, PartialEq, Hash, Default, Debug, Serialize, Deserialize)]
pub struct KeywordSet(pub u128);

impl KeywordSet {
    pub fn has(self, k: Keyword) -> bool {
        self.0 & k.bit() != 0
    }
    pub fn add(&mut self, k: Keyword) {
        self.0 |= k.bit();
    }
    pub fn remove(&mut self, k: Keyword) {
        self.0 &= !k.bit();
    }
}

#[derive(Copy, Clone, Eq, PartialEq, Hash, Ord, PartialOrd, Debug, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export)]
pub enum Zone {
    Library,
    Hand,
    Battlefield,
    Graveyard,
    Stack,
    Exile,
    Command,
    /// Outside the game: cards the owner may fetch with Wish-style effects. Never part of any game rule otherwise.
    Sideboard,
}

#[derive(Copy, Clone, Eq, PartialEq, Hash, Ord, PartialOrd, Debug, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export)]
pub enum Step {
    Untap,
    Upkeep,
    Draw,
    Main1,
    BeginCombat,
    DeclareAttackers,
    DeclareBlockers,
    FirstStrikeDamage,
    CombatDamage,
    EndCombat,
    Main2,
    End,
    Cleanup,
}

impl Step {
    /// Bit used in a player's stop masks.
    pub fn stop_bit(s: Step) -> u16 {
        1 << (s as u16)
    }
    pub fn is_main(self) -> bool {
        matches!(self, Step::Main1 | Step::Main2)
    }
    pub fn next(self) -> Option<Step> {
        use Step::*;
        Some(match self {
            Untap => Upkeep,
            Upkeep => Draw,
            Draw => Main1,
            Main1 => BeginCombat,
            BeginCombat => DeclareAttackers,
            DeclareAttackers => DeclareBlockers,
            DeclareBlockers => FirstStrikeDamage,
            FirstStrikeDamage => CombatDamage,
            CombatDamage => EndCombat,
            EndCombat => Main2,
            Main2 => End,
            End => Cleanup,
            Cleanup => return None,
        })
    }
}

#[derive(Copy, Clone, Eq, PartialEq, Hash, Ord, PartialOrd, Debug, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export)]
pub enum CounterKind {
    PlusOnePlusOne,
    MinusOneMinusOne,
    Loyalty,
    Defense,
    Charge,
    Poison,
    Energy,
    Experience,
    Time,
    Lore,
    Shield,
    Stun,
    Flying,
    Level,
    Age,
    Fade,
    Quest,
    Verse,
    // keyword counters: the permanent has the keyword while it has one
    FirstStrike,
    DoubleStrike,
    Vigilance,
    Deathtouch,
    Lifelink,
    Reach,
    Trample,
    Menace,
    Hexproof,
    Indestructible,
    Haste,
    /// A permanent with a finality counter is exiled instead of dying.
    Finality,
    Ice,
    Corpse,
    Isolation,
    Crystal,
    Study,
    Hit,
    Oil,
    Rad,
    Storage,
    Depletion,
    Ki,
    Bounty,
    Flood,
    Spore,
    Soul,
    Infection,
    Page,
    Blood,
    Wind,
    Plan,
    Memory,
    Doom,
    Net,
    Coin,
    Dream,
    Egg,
    Wage,
    Fuse,
    Brick,
    Pin,
    Rope,
    Fetch,
    Mine,
    Aim,
    Corruption,
    Vortex,
    Manifestation,
    Judgment,
    Collection,
    Ingredient,
    Echo,
    // power / toughness counters other than +1/+1 and -1/-1
    PlusOnePlusZero,
    PlusZeroPlusOne,
    PlusTwoPlusTwo,
    PlusZeroPlusTwo,
    MinusZeroMinusOne,
    MinusZeroMinusTwo,
    MinusOneMinusZero,
    MinusTwoMinusTwo,
    // named counters with no rules of their own
    Fate,
    Takeover,
    Hone,
    Promotion,
    Influence,
    Stash,
    Refine,
    Hunger,
    Glyph,
    Pain,
    Petrification,
    Spite,
    Awakening,
    Peace,
    Saurian,
    Gold,
    Sleep,
    Blight,
    Phylactery,
    Enlightened,
    Void,
    Vow,
    Tide,
    Hatching,
    Duty,
    Luck,
    Polyp,
    Intel,
    Aegis,
    Brain,
    Marker,
    Feather,
    Sleight,
    Rust,
    Reprieve,
    Revival,
    Omen,
    Shred,
    Bribery,
    Decayed,
    Slime,
    Trap,
    Eyeball,
    Delay,
    Paralyzation,
    Death,
    Point,
    PlusTwoPlusZero,
    Impostor,
    Suspect,
    Divinity,
    Matrix,
    Strife,
    Rejection,
    Despair,
    Hour,
    Music,
    Prey,
    Velocity,
    Croak,
    Bore,
    Blaze,
    ShadowCounter,
    Fungus,
    MinusTwoMinusOne,
    Necrodermis,
    PlusOnePlusTwo,
    Wish,
    Voyage,
    Mire,
    /// Magnet counter (Magnetic Web).
    Magnet,
    /// Exalted counter (Emissary of Soulfire): the permanent has an instance of exalted.
    Exalted,
    /// Everything counter (Omo, Queen of Vesuva).
    Everything,
    /// Cage counter (Mairsil, the Pretender).
    Cage,
    Acorn,
    Arrow,
    Arrowhead,
    Bait,
    Blessing,
    Bloodline,
    Bloodstain,
    Book,
    Burden,
    Carrion,
    Cell,
    Chorus,
    Component,
    Conqueror,
    Contested,
    Credit,
    Cube,
    Currency,
    Descent,
    Devotion,
    Dread,
    Elixir,
    Ember,
    Eon,
    Feeding,
    Fellowship,
    Filibuster,
    Film,
    Fire,
    Flame,
    Foreshadow,
    Fury,
    Gem,
    Ghostform,
    Growth,
    Harmony,
    Hatchling,
    Healing,
    Hoofprint,
    Hope,
    Hourglass,
    Incarnation,
    Incubation,
    Ingenuity,
    Intervention,
    Invasion,
    Invitation,
    Javelin,
    Knowledge,
    Landmark,
    Loot,
    Mining,
    Muster,
    Nest,
    Night,
    Ore,
    Palliation,
    Petal,
    Phyresis,
    Plague,
    Plot,
    Possession,
    Pressure,
    Eruption,
    Pupa,
    Rally,
    Rev,
    Ribbon,
    Ritual,
    Scream,
    Shell,
    Silver,
    Skewer,
    Slumber,
    Soot,
    Story,
    Supply,
    Task,
    Theft,
    Tower,
    Training,
    Treasure,
    Unity,
    Unlock,
    Valor,
    Vitality,
    Winch,
    /// Scroll counter (Aretopolis).
    Scroll,
    /// Exposure counter (Aplan Mortarium).
    Exposure,
    Other,
}
