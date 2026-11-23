use crate::types::Color;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use ts_rs::TS;

/// One symbol of a mana cost.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "t")]
#[ts(export)]
pub enum Shard {
    Generic { n: u8 },
    Color { c: Color },
    Colorless,
    Snow,
    X,
    Hybrid { a: Color, b: Color },
    /// {2/W}
    HybridGeneric { c: Color },
    /// {C/W}
    HybridColorless { c: Color },
    /// {W/P}
    Phyrexian { c: Color },
    /// {W/U/P}
    PhyrexianHybrid { a: Color, b: Color },
}

#[derive(Clone, Eq, PartialEq, Hash, Default, Debug, Serialize, Deserialize)]
pub struct ManaCost {
    pub shards: SmallVec<[Shard; 6]>,
    /// Payment-only flag: mana from snow sources may be spent on the coloured symbols as though it were mana of any color (`StaticEffect::SpendSnowManaAsAnyColor`).
    #[serde(default, skip_serializing)]
    pub snow_any: bool,
    /// Payment-only flag (`StaticEffect::SpendWhiteAsAnyOtherAsColorless`, Celestial Dawn): white mana pays any coloured symbol; other coloured mana pays only generic and {C} symbols.
    #[serde(default, skip_serializing)]
    pub white_any: bool,
}

impl ManaCost {
    /// Parse "{2}{W}{U/B}{G/P}{X}".
    pub fn parse(s: &str) -> Result<ManaCost, String> {
        let mut shards = SmallVec::new();
        let mut rest = s.trim();
        while !rest.is_empty() {
            let r = rest.strip_prefix('{').ok_or_else(|| format!("bad mana cost '{s}'"))?;
            let end = r.find('}').ok_or_else(|| format!("bad mana cost '{s}'"))?;
            let sym = &r[..end];
            rest = &r[end + 1..];
            shards.push(parse_shard(sym).ok_or_else(|| format!("bad mana symbol '{{{sym}}}' in '{s}'"))?);
        }
        Ok(ManaCost { shards, snow_any: false, white_any: false })
    }

    /// The cost as text again: "{2}{W}{U/B}" (inverse of `parse`).
    pub fn to_symbols(&self) -> String {
        let l = |c: &Color| ["W", "U", "B", "R", "G"][*c as usize];
        self.shards
            .iter()
            .map(|s| match s {
                Shard::Generic { n } => format!("{{{n}}}"),
                Shard::Color { c } => format!("{{{}}}", l(c)),
                Shard::Colorless => "{C}".into(),
                Shard::Snow => "{S}".into(),
                Shard::X => "{X}".into(),
                Shard::Hybrid { a, b } => format!("{{{}/{}}}", l(a), l(b)),
                Shard::HybridGeneric { c } => format!("{{2/{}}}", l(c)),
                Shard::HybridColorless { c } => format!("{{C/{}}}", l(c)),
                Shard::Phyrexian { c } => format!("{{{}/P}}", l(c)),
                Shard::PhyrexianHybrid { a, b } => format!("{{{}/{}/P}}", l(a), l(b)),
            })
            .collect()
    }

    pub fn mana_value(&self, x: u32) -> u32 {
        self.shards
            .iter()
            .map(|s| match s {
                Shard::Generic { n } => *n as u32,
                Shard::X => x,
                Shard::HybridGeneric { .. } => 2,
                _ => 1,
            })
            .sum()
    }

    pub fn has_phyrexian(&self) -> bool {
        self.shards.iter().any(|s| matches!(s, Shard::Phyrexian { .. } | Shard::PhyrexianHybrid { .. }))
    }

    pub fn has_x(&self) -> bool {
        self.shards.iter().any(|s| matches!(s, Shard::X))
    }

    pub fn colors(&self) -> crate::types::ColorSet {
        let mut cs = crate::types::ColorSet::NONE;
        for s in &self.shards {
            match s {
                Shard::Color { c }
                | Shard::HybridGeneric { c }
                | Shard::HybridColorless { c }
                | Shard::Phyrexian { c } => cs.add(*c),
                Shard::Hybrid { a, b } | Shard::PhyrexianHybrid { a, b } => {
                    cs.add(*a);
                    cs.add(*b)
                }
                _ => {}
            }
        }
        cs
    }

    pub fn is_empty(&self) -> bool {
        self.shards.is_empty()
    }

    /// Add a generic amount (cost increases such as commander tax).
    pub fn add_generic(&mut self, n: u8) {
        if n == 0 {
            return;
        }
        for s in self.shards.iter_mut() {
            if let Shard::Generic { n: g } = s {
                *g = g.saturating_add(n);
                return;
            }
        }
        self.shards.insert(0, Shard::Generic { n });
    }

    pub fn display(&self) -> String {
        let mut out = String::new();
        for s in &self.shards {
            out.push_str(&match s {
                Shard::Generic { n } => format!("{{{n}}}"),
                Shard::Color { c } => format!("{{{c:?}}}"),
                Shard::Colorless => "{C}".into(),
                Shard::Snow => "{S}".into(),
                Shard::X => "{X}".into(),
                Shard::Hybrid { a, b } => format!("{{{a:?}/{b:?}}}"),
                Shard::HybridGeneric { c } => format!("{{2/{c:?}}}"),
                Shard::HybridColorless { c } => format!("{{C/{c:?}}}"),
                Shard::Phyrexian { c } => format!("{{{c:?}/P}}"),
                Shard::PhyrexianHybrid { a, b } => format!("{{{a:?}/{b:?}/P}}"),
            });
        }
        out
    }
}

fn parse_shard(sym: &str) -> Option<Shard> {
    if let Ok(n) = sym.parse::<u8>() {
        return Some(Shard::Generic { n });
    }
    let parts: Vec<&str> = sym.split('/').collect();
    let col = |p: &str| p.chars().next().filter(|_| p.len() == 1).and_then(Color::from_char);
    match parts.as_slice() {
        ["X"] => Some(Shard::X),
        ["C"] => Some(Shard::Colorless),
        ["S"] => Some(Shard::Snow),
        [p] => col(p).map(|c| Shard::Color { c }),
        [a, "P"] => col(a).map(|c| Shard::Phyrexian { c }),
        ["2", b] => col(b).map(|c| Shard::HybridGeneric { c }),
        ["C", b] => col(b).map(|c| Shard::HybridColorless { c }),
        [a, b, "P"] => Some(Shard::PhyrexianHybrid { a: col(a)?, b: col(b)? }),
        [a, b] => Some(Shard::Hybrid { a: col(a)?, b: col(b)? }),
        _ => None,
    }
}

/// Mana in a pool: W U B R G C.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Default, Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ManaPool(pub [u16; 6]);

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export)]
pub enum ManaType {
    W,
    U,
    B,
    R,
    G,
    C,
}

impl ManaType {
    pub const ALL: [ManaType; 6] = [ManaType::W, ManaType::U, ManaType::B, ManaType::R, ManaType::G, ManaType::C];
    pub fn color(self) -> Option<Color> {
        match self {
            ManaType::W => Some(Color::W),
            ManaType::U => Some(Color::U),
            ManaType::B => Some(Color::B),
            ManaType::R => Some(Color::R),
            ManaType::G => Some(Color::G),
            ManaType::C => None,
        }
    }
    pub fn of(c: Color) -> ManaType {
        match c {
            Color::W => ManaType::W,
            Color::U => ManaType::U,
            Color::B => ManaType::B,
            Color::R => ManaType::R,
            Color::G => ManaType::G,
        }
    }
}

impl ManaPool {
    pub fn get(&self, t: ManaType) -> u16 {
        self.0[t as usize]
    }
    pub fn add(&mut self, t: ManaType, n: u16) {
        self.0[t as usize] += n;
    }
    pub fn take(&mut self, t: ManaType) -> bool {
        if self.0[t as usize] > 0 {
            self.0[t as usize] -= 1;
            true
        } else {
            false
        }
    }
    pub fn total(&self) -> u32 {
        self.0.iter().map(|&n| n as u32).sum()
    }
    pub fn clear(&mut self) {
        self.0 = [0; 6];
    }
}
