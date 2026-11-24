//! Human-readable text for abilities and costs, generated from the DSL (used in action menus).

use crate::dsl::*;
use crate::types::*;

fn amount(a: &Amount) -> String {
    match a {
        Amount::Fixed { n } => n.to_string(),
        Amount::X => "X".into(),
        Amount::Count { .. } => "the number of matching permanents".into(),
        Amount::PowerOf { .. } => "its power".into(),
        Amount::PowerMinusBasePower { .. } => "its power minus its base power".into(),
        Amount::ToughnessOf { .. } => "its toughness".into(),
        Amount::LifeOf { .. } => "life total".into(),
        Amount::EventAmount => "that much".into(),
        _ => "some".into(),
    }
}

fn sel(s: &Sel) -> String {
    match s {
        Sel::Target { .. } | Sel::Targets => "target".into(),
        Sel::This => "this permanent".into(),
        Sel::You => "you".into(),
        Sel::EachOpponent => "each opponent".into(),
        Sel::EachPlayer => "each player".into(),
        Sel::All { .. } => "each matching permanent".into(),
        Sel::AttachedTo | Sel::Enchanted => "the enchanted permanent".into(),
        _ => "it".into(),
    }
}

pub fn cost_text(c: &Cost) -> String {
    match c {
        Cost::Mana { cost } => cost.clone(),
        Cost::Tap => "{T}".into(),
        Cost::Untap => "{Q}".into(),
        Cost::Crew { power } => format!("Crew {power}"),
        Cost::RemoveLoyaltyX => "Remove X loyalty counters".into(),
        Cost::RemoveCountersX { .. } => "Remove X counters".into(),
        Cost::RemoveXCountersFromOne { .. } => "Remove X counters from a permanent you control".into(),
        Cost::RollDie { sides } => format!("Roll a d{sides}"),
        Cost::PayLife { n } => format!("Pay {n} life"),
        Cost::PayHalfLife => "Pay half your life, rounded up".to_string(),
        Cost::PayLifeX { .. } => "Pay life".to_string(),
        Cost::TakeDamage { n } => format!("Take {n} damage"),
        Cost::TakeDamageX { .. } => "Take damage".to_string(),
        Cost::SacrificeSelf => "Sacrifice this".into(),
        Cost::RemoveAllCountersGrantSource { .. } => "Remove all counters from the granting permanent".into(),
        Cost::SacrificeGrantSource => "Sacrifice the granting permanent".into(),
        Cost::ExileGrantSource => "Exile the granting permanent".into(),
        Cost::SacrificeDistinctNames { n, .. } => format!("Sacrifice {n} permanents with different names"),
        Cost::Sacrifice { n, .. } => format!("Sacrifice {n} permanent(s)"),
        Cost::ExileSpellFromStack { .. } => "Exile a spell you control from the stack".into(),
        Cost::ExileGraveyard => "Exile your graveyard".to_string(),
        Cost::Waterbend { n } => format!("Waterbend {{{n}}}"),
        Cost::TapCreaturesOrMana { n } => format!("{{{n}}} (creatures may be tapped for mana)"),
        Cost::WaterbendMV => "Waterbend {X}, X = its mana value".to_string(),
        Cost::SacrificeReduceByManaValue { .. } => "Sacrifice a permanent (the cost is reduced by its mana value)".into(),
        Cost::SacrificeReduceByManaCost { .. } => "Sacrifice a permanent (offering: the cost is reduced by its mana cost)".into(),
        Cost::TapReduceByPower { .. } => "Tap an untapped permanent (the cost is reduced by its power)".into(),
        Cost::XTimes { cost } => format!("{} X times", cost_text(cost)),
        Cost::RevealShareColor { n } => format!("Reveal {n} cards that share a color"),
        Cost::DiscardDistinctNames { n, .. } => format!("Discard {n} cards with different names"),
        Cost::DiscardSameName { n, .. } => format!("Discard {n} cards with the same name"),
        Cost::TapShareType { n, .. } => format!("Tap {n} untapped creatures that share a creature type"),
        Cost::SacrificeTotalPower { power, .. } => format!("Sacrifice creatures with total power {power} or greater"),
        Cost::TapTotalPower { power, .. } => format!("Tap untapped creatures with total power {power} or greater"),
        Cost::GainControlOfChosen { .. } | Cost::GainControlOfCard { .. } => "Gain control of a permanent you don't control".into(),
        Cost::PutCountersOnCard { n, .. } => format!("Put {n} counter(s) on a permanent"),
        Cost::SourceManaCost => "Pay its mana cost".into(),
        Cost::ManaCostOf { .. } => "Pay that card's mana cost".into(),
        Cost::AttachedManaCost => "Pay the enchanted permanent's mana cost".into(),
        Cost::ManaAmount { .. } => "Pay a computed amount of mana".into(),
        Cost::ColoredManaAmount { .. } => "Pay a computed amount of coloured mana".into(),
        Cost::PutCountersOnChosen { n, .. } => format!("Put {n} counter(s) on a permanent you control"),
        Cost::RemoveAllCounters { .. } => "Remove all counters".into(),
        Cost::XMinimum { n } => format!("X is at least {n}"),
        Cost::Warp => "Warp".into(),
        Cost::Blitz => "Blitz".into(),
        Cost::Evoke => "Evoke".into(),
        Cost::AltFlash => "".into(),
        Cost::SneakEntry => "".into(),
        Cost::CollectEvidence { n } => format!("Collect evidence {n}"),
        Cost::CollectEvidenceTargets => "Collect evidence X".into(),
        Cost::Discard { n, .. } => format!("Discard {n} card(s)"),
        Cost::DiscardSelf => "Discard this".into(),
        Cost::ExileSymbolsFromGraveyard { n, .. } => format!("Exile cards from your graveyard with {n} or more mana symbols"),
        Cost::DiscardHand => "Discard your hand".into(),
        Cost::SacrificeAmount { .. } => "Sacrifice permanents".into(),
        Cost::SacrificeAll { .. } => "Sacrifice all".into(),
        Cost::ExileSelf => "Exile this".into(),
        Cost::PutSelfOnLibraryBottom => "Put this on the bottom of its owner's library".into(),
        Cost::ExileFromGraveyardPayCost { .. } => "Exile a card from your graveyard and pay its mana cost".into(),
        Cost::ExileFromGraveyardAmount { .. } => "Exile cards from your graveyard".into(),
        Cost::ExileFromGraveyard { n, bottom, .. } => if *bottom { format!("Put {n} card(s) from your graveyard on the bottom of your library") } else { format!("Exile {n} card(s) from your graveyard") },
        Cost::ExileTotalMvX { .. } => "Exile permanents you control with total mana value X".into(),
        Cost::ExileGraveyardMv { n, .. } => format!("Exile cards from your graveyard with total mana value {n} or greater"),
        Cost::ExileCraftMaterials { .. } => "Exile one or more permanents you control and/or cards from your graveyard".into(),
        Cost::ExileGraveyardTypes { n } => format!("Exile cards from your graveyard with {n} or more card types among them"),
        Cost::ExileFromSingleGraveyard { n, .. } => format!("Exile {n} card(s) from a single graveyard"),
        Cost::PutExiledIntoGraveyard { .. } => "Put a card exiled with this into its owner's graveyard".into(),
        Cost::ExileFromHand { n, .. } => format!("Exile {n} card(s) from your hand"),
        Cost::TapPermanents { n, .. } => format!("Tap {n} untapped permanent(s) you control"),
        Cost::UntapPermanents { n, opponent, .. } => format!("Untap {n} tapped permanent(s) {} control", if *opponent { "an opponent" } else { "you" }),
        Cost::ReturnToHand { n, .. } => format!("Return {n} permanent(s) to hand"),
        Cost::ExileFromBattlefield { n, .. } => format!("Exile {n} permanent(s) you control"),
        Cost::UnattachAttached { .. } => "Unattach an attached permanent".to_string(),
        Cost::UnattachSelf => "Unattach this".to_string(),
        Cost::ExertSelf => "Exert this creature".to_string(),
        Cost::Behold { .. } => "Behold a card".to_string(),
        Cost::ChooseCreatureOrWarped => "Choose a creature you control or a warped creature card you own in exile".to_string(),
        Cost::RemoveCountersFrom { n, .. } => format!("Remove {n} counter(s) from a permanent you control"),
        Cost::RemoveAnyCounters { n } => format!("Remove {n} counter(s)"),
        Cost::RemoveAnyCountersFrom { n, .. } => format!("Remove {n} counter(s) from permanents you control"),
        Cost::RemoveCounters { n, kind } => format!("Remove {n} {kind:?} counter(s)"),
        Cost::PutCounters { n, kind } => format!("Put {n} {kind:?} counter(s)"),
        Cost::Mill { n } => format!("Mill {n}"),
        Cost::DrawCards { n } => format!("Draw {n} card(s)"),
        Cost::ControllerDraws { n } => format!("Have the controller draw {n} card(s)"),
        Cost::OpponentGainsLife { n } => format!("An opponent gains {n} life"),
        Cost::EachOtherPlayerGainsLife { n } => format!("Each other player gains {n} life"),
        Cost::OpponentCreatesToken { token, n } => format!("An opponent creates {n} {token}"),
        Cost::PlayerGainsLife { player, n } => format!("Player {} gains {n} life", player + 1),
        Cost::PlayerCreatesToken { player, token, n } => format!("Player {} creates {n} {token}", player + 1),
        Cost::ExileTop { n } => format!("Exile the top {n} cards of your library"),
        Cost::PayEnergy { n } => format!("Pay {n} energy"),
        Cost::GetPoison { n } => format!("Get {n} poison counters"),
        Cost::FlipCoins { n } => format!("Flip {n} coins"),
        Cost::AddMana { color, n } => format!("Add {n} {color:?} mana"),
        Cost::PayEnergyX { .. } => "Pay energy".to_string(),
        Cost::Reveal { .. } => "Reveal a card".into(),
        Cost::Or { .. } => "choose a cost".into(),
        Cost::SacrificeCards { .. } => "Sacrifice the chosen permanents".into(),
        Cost::DiscardCards { .. } => "Discard the chosen cards".into(),
    }
}

pub fn effect_text(e: &Effect) -> String {
    match e {
        Effect::DealDamage { to, amount: a, .. } => format!("Deal {} damage to {}", amount(a), sel(to)),
        Effect::Draw { who, n, .. } => format!("{} draw {} card(s)", sel(who), amount(n)),
        Effect::GainLife { who, n } => format!("{} gain {} life", sel(who), amount(n)),
        Effect::LoseLife { who, n } => format!("{} lose {} life", sel(who), amount(n)),
        Effect::Destroy { what, .. } => format!("Destroy {}", sel(what)),
        Effect::Exile { what, .. } => format!("Exile {}", sel(what)),
        Effect::Bounce { what } => format!("Return {} to hand", sel(what)),
        Effect::CounterSpell { .. } => "Counter target spell".into(),
        Effect::Pump { what, power, toughness, .. } => format!("{} gets {}/{}", sel(what), signed(power), signed(toughness)),
        Effect::AddCounters { kind, n, what } => format!("Put {} {kind:?} counter(s) on {}", amount(n), sel(what)),
        Effect::CreateToken { token, n, .. } => format!("Create {} {token} token(s)", amount(n)),
        Effect::AddMana { mana, .. } => {
            let mut s = String::from("Add ");
            for m in mana {
                match m {
                    ManaProduce::Fixed { m, n } | ManaProduce::Restricted { m, n, .. } => {
                        for _ in 0..*n {
                            s.push_str(&format!("{{{m:?}}}"));
                        }
                    }
                    _ => s.push_str("one mana of any color"),
                }
            }
            s
        }
        Effect::Discard { who, n, .. } => format!("{} discard {} card(s)", sel(who), amount(n)),
        Effect::Tap { what } => format!("Tap {}", sel(what)),
        Effect::Untap { what } => format!("Untap {}", sel(what)),
        Effect::Scry { n, .. } => format!("Scry {}", amount(n)),
        Effect::Surveil { n, .. } => format!("Surveil {}", amount(n)),
        Effect::SearchLibrary { .. } => "Search your library".into(),
        Effect::Attach { to, .. } => format!("Attach to {}", sel(to)),
        Effect::Investigate { .. } => "Investigate".into(),
        Effect::Proliferate { .. } => "Proliferate".into(),
        Effect::Mill { n, .. } => format!("Mill {}", amount(n)),
        Effect::ExileTopOfLibrary { n, .. } => format!("Exile the top {} cards of a library", amount(n)),
        _ => "…".into(),
    }
}

/// "+2", "-1", or "+X" for an amount that is counted when the effect happens.
fn signed(a: &Amount) -> String {
    if let Amount::Fixed { n } = a {
        format!("{n:+}")
    } else {
        "+X".into()
    }
}

/// "{2}, {T}: Draw a card."
pub fn ability_text(a: &Ability) -> String {
    match a {
        Ability::Activated { cost, effects, loyalty, .. } => {
            let mut head: Vec<String> = cost.iter().map(cost_text).collect();
            if let Some(l) = loyalty {
                head.insert(0, format!("{l:+}"));
            }
            let body: Vec<String> = effects.iter().map(effect_text).collect();
            format!("{}: {}", head.join(", "), body.join(". "))
        }
        Ability::Triggered { effects, .. } => effects.iter().map(effect_text).collect::<Vec<_>>().join(". "),
        Ability::Spell { effects, .. } => effects.iter().map(effect_text).collect::<Vec<_>>().join(". "),
        _ => String::new(),
    }
}

/// Type line like "Legendary Creature — Elf Druid".
pub fn type_line(supertypes: &[Supertype], types: TypeSet, subtypes: &[Sym]) -> String {
    let mut parts: Vec<String> = vec![];
    for s in supertypes {
        parts.push(format!("{s:?}"));
    }
    for t in [
        CardType::Artifact,
        CardType::Battle,
        CardType::Creature,
        CardType::Enchantment,
        CardType::Instant,
        CardType::Kindred,
        CardType::Land,
        CardType::Planeswalker,
        CardType::Sorcery,
        CardType::Dungeon,
        CardType::Plane,
        CardType::Phenomenon,
        CardType::Scheme,
        CardType::Vanguard,
        CardType::Conspiracy,
        CardType::Attraction,
        CardType::Emblem,
    ] {
        if types.has(t) {
            parts.push(t.name().to_string());
        }
    }
    let mut s = parts.join(" ");
    if !subtypes.is_empty() {
        s.push_str(" — ");
        s.push_str(&subtypes.iter().map(|x| x.name()).collect::<Vec<_>>().join(" "));
    }
    s
}
