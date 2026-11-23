//! Self-checks run by the fuzzer and in debug builds: the game state must always be consistent.

use crate::ids::*;
use crate::state::*;
use crate::types::*;

impl Game {
    /// Verify structural consistency; returns a description of the first problem found.
    pub fn check_invariants(&self) -> Result<(), String> {
        let mut seen = vec![0u8; self.cards.len()];
        let mut mark = |id: CardId, what: &str| -> Result<(), String> {
            let i = id.0 as usize;
            if i >= seen.len() {
                return Err(format!("{what}: card id {} out of range", id.0));
            }
            seen[i] += 1;
            Ok(())
        };
        for &id in &self.battlefield {
            mark(id, "battlefield")?;
            let c = &self.cards[id.0 as usize];
            if c.zone != Zone::Battlefield {
                return Err(format!("{} listed on the battlefield but zone is {:?}", self.def_of(id).name, c.zone));
            }
        }
        for &id in &self.phased {
            mark(id, "phased")?;
            let c = &self.cards[id.0 as usize];
            if c.zone != Zone::Battlefield || !c.phased_out {
                return Err(format!("{} listed as phased out but zone is {:?}", self.def_of(id).name, c.zone));
            }
        }
        for p in &self.players {
            for (list, zone, name) in [
                (&p.library, Zone::Library, "library"),
                (&p.hand, Zone::Hand, "hand"),
                (&p.graveyard, Zone::Graveyard, "graveyard"),
                (&p.exile, Zone::Exile, "exile"),
                (&p.command, Zone::Command, "command"),
                (&p.sideboard, Zone::Sideboard, "sideboard"),
            ] {
                for &id in list {
                    mark(id, name)?;
                    let c = &self.cards[id.0 as usize];
                    if c.zone != zone {
                        return Err(format!("{} in player {}'s {name} list but zone is {:?}", self.def_of(id).name, p.id, c.zone));
                    }
                    if c.owner != p.id {
                        return Err(format!("{} in {name} of player {} but owned by {}", self.def_of(id).name, p.id, c.owner));
                    }
                }
            }
            if p.pool.0.iter().any(|n| *n > 60_000) {
                return Err(format!("absurd mana pool: p{} {:?}", p.id, p.pool.0));
            }
        }
        for s in &self.stack {
            if matches!(s.kind, ItemKind::Spell { .. }) && !s.copy {
                mark(s.source, "stack")?;
                let c = &self.cards[s.source.0 as usize];
                if c.zone != Zone::Stack {
                    return Err(format!("spell {} on the stack but zone is {:?}", self.def_of(s.source).name, c.zone));
                }
            }
        }
        // mutate: merged cards are held off every zone list
        let merged_ids: Vec<CardId> = self.cards.iter().flat_map(|c| c.merged.iter().copied()).collect();
        for c in &self.cards {
            let n = seen[c.id.0 as usize];
            let listed = matches!(c.zone, Zone::Battlefield | Zone::Library | Zone::Hand | Zone::Graveyard | Zone::Command | Zone::Stack) || !c.token;
            // tokens that ceased to exist and copies of spells are parked in Exile without being listed
            if n > 1 {
                return Err(format!("{} appears in {n} zones", self.def_of(c.id).name));
            }
            // Planechase / Archenemy: the planar deck and the scheme decks are held off every zone list
            let parked = self.planar_deck.contains(&c.id) || self.players.iter().any(|p| p.scheme_deck.contains(&c.id));
            if n == 0 && !parked && !c.token && listed && !self.is_copy_card(c.id) && !merged_ids.contains(&c.id) && !self.in_flight(c.id) && !self.players.iter().any(|p| p.lost) {
                return Err(format!("{} (zone {:?}) is in no zone list", self.def_of(c.id).name, c.zone));
            }
            if c.zone != Zone::Battlefield && (c.attached_to.is_some()) {
                return Err(format!("{} attached while not on the battlefield", self.def_of(c.id).name));
            }
            if c.counters.iter().any(|(_, n)| *n == 0) {
                return Err("zero-valued counter entry".into());
            }
            if c.zone == Zone::Battlefield && c.tapped && false {
                return Err("unreachable".into());
            }
        }
        for p in &self.players {
            if p.library.len() + p.hand.len() + p.graveyard.len() > self.cards.len() {
                return Err("more cards than exist".into());
            }
        }
        // nothing is stuck on the stack: a stack that keeps growing without bound is a runaway loop
        if self.stack.len() > 5000 {
            return Err(format!("runaway stack: {} items", self.stack.len()));
        }
        if let Some(cb) = &self.combat {
            for a in &cb.attackers {
                let c = &self.cards[a.card.0 as usize];
                if c.zone != Zone::Battlefield && self.step.is_main() {
                    return Err("attacker left combat state inconsistent".into());
                }
            }
        }
        Ok(())
    }


    /// State-based actions are settled whenever a player holds priority: nobody sits at 0 life, no creature has 0 toughness,
    /// no planeswalker has no loyalty. Only meaningful at a priority decision (mid-resolution the actions are still pending).
    pub fn check_sbas_settled(&self) -> Result<(), String> {
        let at_priority = matches!(self.decision().map(|d| &d.kind), Some(crate::decision::DecisionKind::Priority { .. }));
        if !at_priority || self.over.is_some() || self.layers_dirty {
            return Ok(());
        }
        for p in &self.players {
            if !p.lost && p.life <= 0 && !self.dont_lose_at_zero(p.id) {
                return Err(format!("player {} holds priority at {} life without having lost", p.id, p.life));
            }
        }
        for &id in &self.battlefield {
            let c = &self.cards[id.0 as usize];
            if c.phased_out {
                continue;
            }
            if c.cur.types.has(CardType::Creature) && c.cur.toughness <= 0 && !self.as_enters_waiting(id) {
                return Err(format!("{} has toughness {} on the battlefield at priority", self.def_of(id).name, c.cur.toughness));
            }
            if c.cur.types.has(CardType::Planeswalker) && c.counter(CounterKind::Loyalty) <= 0 && !c.cur.zero_loyalty_stays {
                return Err(format!("{} has no loyalty on the battlefield at priority", self.def_of(id).name));
            }
        }
        Ok(())
    }

    fn is_copy_card(&self, id: CardId) -> bool {
        self.stack.iter().any(|s| s.copy && s.source == id)
    }

    /// The spell currently being cast (on the stack zone but not yet on the stack list), or resolving.
    fn in_flight(&self, id: CardId) -> bool {
        match &self.run {
            Run::Cast(ctx) => ctx.item.source == id,
            Run::Resolve(ctx) => ctx.item.source == id,
            _ => false,
        }
    }
}
