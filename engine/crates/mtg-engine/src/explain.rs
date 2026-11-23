//! Plain-language reasons why something can't be done right now: shown to a player whose click or drag the rules refused.
//! These mirror the checks of `legal_actions`, `can_attack` and `can_block_attacker`; the last line of each answer is a safe fallback.

use crate::decision::{AttackerOpt, BlockerOpt};
use crate::dsl::*;
use crate::ids::*;
use crate::state::*;
use crate::types::*;

impl Game {
    fn cname(&self, id: CardId) -> String {
        self.def_of(id).display_name.clone()
    }

    fn pname(&self, p: PlayerId) -> String {
        self.players.get(p as usize).map_or_else(|| "a player".to_string(), |pl| pl.name.clone())
    }

    /// Why the sorcery-speed timing rule isn't met right now (None when it is).
    fn timing_reason(&self, p: PlayerId, what: &str) -> Option<String> {
        if self.active != p {
            return Some(format!("{what} only on your own turn (it is {}'s turn).", self.pname(self.active)));
        }
        if !self.step.is_main() {
            return Some(format!("{what} only in a main phase (it is the {} step).", format!("{:?}", self.step).to_lowercase()));
        }
        if !self.stack.is_empty() {
            return Some(format!("{what} only when the stack is empty ({} thing(s) are on it; let them resolve first).", self.stack.len()));
        }
        None
    }

    /// Names of permanents with a static "can't cast" / "can't play lands" style effect that applies to `p`.
    fn restrictors(&self, p: PlayerId, lands: bool) -> Vec<String> {
        let mut out = vec![];
        for &src in &self.battlefield {
            let sc = &self.cards[src.0 as usize];
            if sc.cur.no_abilities {
                continue;
            }
            let sd = self.db.def(sc.def);
            let sctrl = sc.cur.controller;
            for &i in &sd.statics {
                if let Ability::Static { effect, condition, .. } = &sd.abilities[i as usize] {
                    if condition.as_ref().map_or(false, |cd| !self.eval_cond(cd, &crate::eval::EvalCtx::with_controller(src, sctrl))) {
                        continue;
                    }
                    let hit = match effect {
                        StaticEffect::CantCast { who } | StaticEffect::CantCastCards { who, .. } if !lands => self.player_filter_src(src, *who, p, sctrl),
                        StaticEffect::CastOnlyAtSorcerySpeed { who } if !lands => self.player_filter_src(src, *who, p, sctrl),
                        StaticEffect::OpponentsCantCastDuringYourTurn if !lands => p != sctrl && self.active == sctrl,
                        StaticEffect::NoSpellsOrAbilitiesInCombat | StaticEffect::NoSpellsInCombat if !lands => true,
                        StaticEffect::CantPlayLands { who, .. } if lands => self.player_filter_src(src, *who, p, sctrl),
                        _ => false,
                    };
                    if hit {
                        out.push(self.cname(src));
                    }
                }
            }
        }
        out.sort();
        out.dedup();
        out
    }

    /// Why `p` can't play / cast / use `card` right now.
    pub fn explain_play(&self, p: PlayerId, card: CardId) -> String {
        if self.over.is_some() {
            return "The game is over.".into();
        }
        let Some(c) = self.cards.get(card.0 as usize) else { return "That card doesn't exist.".into() };
        let name = self.cname(card);
        if let Some(d) = &self.pending {
            if d.player != p {
                return format!("It isn't your decision: the game is waiting for {}.", self.pname(d.player));
            }
            if !matches!(d.kind, crate::decision::DecisionKind::Priority { .. }) {
                return "Answer the open question first (see the prompt on the table).".into();
            }
        }
        let d = self.def_of(card);
        // a card in another zone that a permission lets you play (the revealed top of the library, a flashback card ...) is judged like one in hand
        let permitted = matches!(c.zone, Zone::Library | Zone::Graveyard | Zone::Exile | Zone::Command) && self.play_permission(p, card);
        match c.zone {
            _ if permitted || (c.zone == Zone::Hand && c.owner == p) => {
                if d.types.has(CardType::Land) {
                    let pl = &self.players[p as usize];
                    let limit = pl.land_limit as i32 + self.extra_land_drops(p);
                    if self.cant_play_lands(p) {
                        let who = self.restrictors(p, true);
                        return if who.is_empty() { "An effect stops you from playing lands.".into() } else { format!("{} stops you from playing lands.", who.join(", ")) };
                    }
                    if pl.lands_played as i32 >= limit {
                        return format!("You've already played {} land{} this turn (you may play {}). Cards like Azusa, Exploration or Oracle of Mul Daya allow more.", pl.lands_played, if pl.lands_played == 1 { "" } else { "s" }, limit);
                    }
                    if let Some(r) = self.timing_reason(p, "Lands can be played") {
                        return r;
                    }
                    if self.cant_play_land_card(p, card) {
                        return format!("An effect stops you from playing {name}.");
                    }
                    return format!("{name} can't be played as a land right now.");
                }
                // a spell
                if !self.can_cast_timing(p, card) {
                    let what = format!("{name} has no flash, so it can be cast");
                    if let Some(r) = self.timing_reason(p, &what) {
                        return r;
                    }
                }
                let who = self.restrictors(p, false);
                if !who.is_empty() {
                    return format!("{} stops you from casting {name} right now.", who.join(", "));
                }
                if let Some(cap) = self.effects.iter().find_map(|e| match e.modif {
                    Modif::SpellCap(cap) if e.players.contains(&p) && self.players[p as usize].spells_cast >= cap => Some(cap),
                    _ => None,
                }) {
                    return format!("You've already cast {} spell(s) this turn and an effect limits you to {cap}.", self.players[p as usize].spells_cast);
                }
                let cost = self.effective_cost(p, card, None);
                if !self.can_afford(p, &cost, 0) {
                    let pool = &self.players[p as usize].pool;
                    let floating = pool.total();
                    let sources = self.mana_sources(p).len();
                    let tax = match self.players[p as usize].commander_cast.iter().find(|(id, _)| *id == card) {
                        Some((_, n)) if *n > 0 => format!(" (including {{{}}} commander tax)", *n * 2),
                        _ => String::new(),
                    };
                    return format!(
                        "{name} costs {}{tax}, which you can't pay: you have {floating} mana floating and {sources} untapped source{} you could tap{}.",
                        cost.display(),
                        if sources == 1 { "" } else { "s" },
                        self.missing_colors(p, &cost)
                    );
                }
                let (specs, modal) = self.spell_specs(card);
                if modal.is_none() && !specs.is_empty() && !self.specs_satisfiable(&specs, card, p) {
                    return format!("{name} needs a target and there is no legal one.");
                }
                format!("{name} can't be cast right now (no way to meet its requirements).")
            }
            Zone::Hand => format!("{name} is in {}'s hand.", self.pname(c.owner)),
            Zone::Battlefield if c.cur.controller == p => self.explain_use(p, card),
            Zone::Battlefield => format!("{name} is controlled by {}: you can't use it.", self.pname(c.cur.controller)),
            Zone::Graveyard | Zone::Exile | Zone::Library | Zone::Command | Zone::Sideboard => {
                format!("{name} is in {} {}: nothing lets you play it from there right now.", if c.owner == p { "your" } else { "another player's" }, format!("{:?}", c.zone).to_lowercase())
            }
            Zone::Stack => format!("{name} is on the stack: it resolves when everyone passes."),
        }
    }

    /// Colors the cost needs that nothing you control can produce (or empty).
    fn missing_colors(&self, p: PlayerId, cost: &crate::mana::ManaCost) -> String {
        let mut have = std::collections::BTreeSet::new();
        for s in self.mana_sources(p) {
            for m in &s.produces {
                have.insert(format!("{m:?}"));
            }
        }
        for m in [crate::mana::ManaType::W, crate::mana::ManaType::U, crate::mana::ManaType::B, crate::mana::ManaType::R, crate::mana::ManaType::G, crate::mana::ManaType::C] {
            if self.players[p as usize].pool.get(m) > 0 {
                have.insert(format!("{m:?}"));
            }
        }
        let need: Vec<String> = ["W", "U", "B", "R", "G"].iter().filter(|c| cost.display().contains(&format!("{{{c}}}")) && !have.contains(**c)).map(|c| format!("{{{c}}}")).collect();
        if need.is_empty() {
            String::new()
        } else {
            format!(" (nothing you control can make {})", need.join(" or "))
        }
    }

    /// Why a permanent you control has nothing to activate right now.
    fn explain_use(&self, p: PlayerId, card: CardId) -> String {
        let c = &self.cards[card.0 as usize];
        let d = self.def_of(card);
        let name = self.cname(card);
        let acts: Vec<u16> = d.activated.iter().copied().filter(|i| !d.mana_abilities.contains(i)).collect();
        let mana = !d.mana_abilities.is_empty();
        if acts.is_empty() && !mana {
            return format!("{name} has no abilities you can activate.");
        }
        if c.detained_by.is_some() {
            return format!("{name} is detained: its abilities can't be activated.");
        }
        let taps = d.abilities.iter().any(|a| matches!(a, Ability::Activated { cost, .. } if cost.iter().any(|x| matches!(x, Cost::Tap))));
        if taps && c.tapped {
            return format!("{name} is tapped, so its {{T}} abilities can't be used.");
        }
        if taps && d.types.has(CardType::Creature) && !self.can_use_tap_symbol(card) {
            return format!("{name} has summoning sickness (it came under your control this turn), so its {{T}} abilities can't be used until your next turn starts.");
        }
        if acts.iter().any(|i| matches!(d.abilities.get(*i as usize), Some(Ability::Activated { loyalty: Some(_), .. }))) {
            if let Some(r) = self.timing_reason(p, "Planeswalker abilities can be used") {
                return r;
            }
            return format!("You've already activated a loyalty ability of {name} this turn.");
        }
        if let Some(r) = acts
            .iter()
            .find_map(|i| match d.abilities.get(*i as usize) {
                Some(Ability::Activated { timing: Timing::Sorcery, .. }) => self.timing_reason(p, &format!("{name}'s ability can be activated")),
                _ => None,
            })
        {
            return r;
        }
        if mana && acts.is_empty() {
            return format!("{name}'s mana ability can't be used right now (it is tapped, or you can't pay its cost).");
        }
        format!("You can't pay the cost of {name}'s abilities right now, or an effect stops you from activating it.")
    }

    /// Why `p`'s creature can't attack.
    pub fn explain_attack(&self, p: PlayerId, card: CardId) -> String {
        let c = &self.cards[card.0 as usize];
        let name = self.cname(card);
        if c.cur.controller != p {
            return format!("{name} isn't yours.");
        }
        if self.active != p || self.step != Step::DeclareAttackers {
            return format!("{name} can attack only when you declare attackers on your own turn.");
        }
        if !c.cur.types.has(CardType::Creature) {
            return format!("{name} isn't a creature.");
        }
        if c.tapped {
            return format!("{name} is tapped.");
        }
        if c.cur.kw.has(Keyword::Defender) && !c.cur.attack_no_defender {
            return format!("{name} has defender: it can't attack.");
        }
        if c.cur.cant_attack || c.cur.kw.has(Keyword::CantAttack) {
            return format!("An effect says {name} can't attack.");
        }
        if !self.can_attack_unsick(card) && !c.cur.attack_haste {
            return format!("{name} has summoning sickness: it hasn't been under your control since your turn began, and it has no haste.");
        }
        if self.no_attack_until >= self.turn && self.no_attack_until != 0 {
            return "An effect stops all attacks this turn.".into();
        }
        format!("{name} is restricted from attacking right now (an effect limits who or what it may attack).")
    }

    /// Why `blocker` can't block `attacker` (None when it can).
    pub fn why_cant_block(&self, blocker: CardId, attacker: CardId) -> Option<String> {
        let b = &self.cards[blocker.0 as usize];
        let a = &self.cards[attacker.0 as usize];
        let (bn, an) = (self.cname(blocker), self.cname(attacker));
        if b.tapped && !self.can_block_while_tapped(blocker) {
            return Some(format!("{bn} is tapped, and tapped creatures can't block."));
        }
        if !b.cur.types.has(CardType::Creature) {
            return Some(format!("{bn} isn't a creature."));
        }
        if b.cur.cant_block || b.cur.kw.has(Keyword::CantBlock) {
            return Some(format!("An effect says {bn} can't block."));
        }
        if b.cur.kw.has(Keyword::Decayed) {
            return Some(format!("{bn} is decayed: it can't block."));
        }
        if b.cur.attacker_conds.iter().any(|(cd, csrc)| self.eval_cond(cd, &crate::eval::EvalCtx::with_controller(*csrc, a.cur.controller)))
            || b.cur.cant_block_attackers.iter().any(|(flt, src, ctl)| self.matches_card(flt, attacker, *src, *ctl))
        {
            return Some(format!("An effect says {bn} can't block {an}."));
        }
        let ak = a.cur.kw;
        if ak.has(Keyword::Unblockable) {
            return Some(format!("{an} can't be blocked."));
        }
        if ak.has(Keyword::Flying) && !(b.cur.kw.has(Keyword::Flying) || b.cur.kw.has(Keyword::Reach) || self.blocks_as_reach_vs(blocker, attacker)) {
            return Some(format!("{an} has flying: only creatures with flying or reach can block it, and {bn} has neither."));
        }
        if ak.has(Keyword::Shadow) != b.cur.kw.has(Keyword::Shadow) && !(b.cur.can_block_shadow && ak.has(Keyword::Shadow)) {
            return Some(format!("Shadow: only creatures with shadow can block creatures with shadow (and the reverse)."));
        }
        if ak.has(Keyword::Horsemanship) && !b.cur.kw.has(Keyword::Horsemanship) {
            return Some(format!("{an} has horsemanship: only creatures with horsemanship can block it."));
        }
        if ak.has(Keyword::Fear) && !(b.cur.colors.has(Color::B) || b.cur.types.has(CardType::Artifact)) {
            return Some(format!("{an} has fear: only artifact and black creatures can block it, and {bn} is neither."));
        }
        if ak.has(Keyword::Intimidate) && !(b.cur.types.has(CardType::Artifact) || b.cur.colors.0 & a.cur.colors.0 != 0) {
            return Some(format!("{an} has intimidate: only artifact creatures and creatures sharing a color with it can block it."));
        }
        if ak.has(Keyword::Skulk) && b.cur.power > a.cur.power {
            return Some(format!("{an} has skulk: it can't be blocked by creatures with greater power ({bn} has {}, {an} has {}).", b.cur.power, a.cur.power));
        }
        if a.cur.cant_be_blocked_greater && b.cur.power > a.cur.power {
            return Some(format!("{an} can't be blocked by creatures with greater power ({bn} has {}, {an} has {}).", b.cur.power, a.cur.power));
        }
        for (kw, land) in [(Keyword::Plainswalk, "Plains"), (Keyword::Islandwalk, "Island"), (Keyword::Swampwalk, "Swamp"), (Keyword::Mountainwalk, "Mountain"), (Keyword::Forestwalk, "Forest")] {
            if ak.has(kw) && !b.cur.can_block_landwalk && a.cur.ignore_landwalk & crate::layers::landwalk_bit(Some(kw)) == 0 {
                let sym = Sym::new(land);
                if self.battlefield.iter().any(|id| {
                    let l = &self.cards[id.0 as usize];
                    l.cur.controller == b.cur.controller && l.cur.subtypes.contains(&sym)
                }) {
                    return Some(format!("{an} has {land}walk and you control a {land}, so it can't be blocked."));
                }
            }
        }
        for pr in &a.cur.protections {
            let hit = match pr {
                ProtectionFrom::Color { c } => b.cur.colors.has(*c),
                ProtectionFrom::Type { t } => b.cur.types.has(*t),
                ProtectionFrom::Everything => true,
                _ => false,
            };
            if hit {
                return Some(format!("{an} has protection from something {bn} is, so {bn} can't block it."));
            }
        }
        if a.cur.cant_be_blocked_by.iter().any(|(f, fsrc, fctrl)| self.matches_card(f, blocker, *fsrc, *fctrl))
            || a.cur.defender_conds.iter().any(|(cd, unb, csrc, _)| *unb && self.eval_cond(cd, &crate::eval::EvalCtx::with_controller(*csrc, b.cur.controller)))
        {
            return Some(format!("An effect says {an} can't be blocked by {bn}."));
        }
        if !self.can_block_attacker(blocker, attacker) {
            return Some(format!("An effect on the battlefield stops {bn} from blocking {an}."));
        }
        None
    }

    /// Why `blocker` can't block (`attacker` given: that attacker; otherwise any of the attackers).
    pub fn explain_block(&self, p: PlayerId, blocker: Option<CardId>, attacker: Option<CardId>) -> String {
        let attackers: Vec<CardId> = self.combat.as_ref().map(|c| c.attackers.iter().map(|a| a.card).collect()).unwrap_or_default();
        match (blocker, attacker) {
            (Some(b), Some(a)) => self.why_cant_block(b, a).unwrap_or_else(|| format!("{} can block {}.", self.cname(b), self.cname(a))),
            (Some(b), None) => {
                if self.cards[b.0 as usize].cur.controller != p {
                    return format!("{} isn't yours.", self.cname(b));
                }
                let reasons: Vec<String> = attackers.iter().filter_map(|a| self.why_cant_block(b, *a)).collect();
                match reasons.len() {
                    0 => format!("{} can block.", self.cname(b)),
                    1 => reasons[0].clone(),
                    _ => {
                        let mut u = reasons.clone();
                        u.dedup();
                        // the same reason for every attacker (tapped, can't block): say it once
                        if u.len() == 1 { u[0].clone() } else { format!("{} can't block any of the attackers: {}", self.cname(b), reasons.join(" "))}
                    }
                }
            }
            (None, Some(a)) => {
                let mine: Vec<CardId> = self.battlefield.iter().copied().filter(|id| self.cards[id.0 as usize].cur.controller == p && self.cards[id.0 as usize].cur.types.has(CardType::Creature)).collect();
                if mine.is_empty() {
                    return "You control no creatures that could block.".into();
                }
                let reasons: Vec<String> = mine.iter().filter_map(|b| self.why_cant_block(*b, a)).collect();
                if reasons.len() == mine.len() { reasons.join(" ") } else { format!("{} can be blocked.", self.cname(a)) }
            }
            (None, None) => "Click one of your creatures, then the attacker it should block.".into(),
        }
    }
}

impl Game {
    fn notice(&mut self, player: PlayerId, text: String) {
        self.emit(Event::Notice { player, text });
    }

    /// After attackers were declared: tell the player about attackers the rules dropped or added (a requirement, a restriction, a cost).
    pub(crate) fn note_attack_changes(&mut self, p: PlayerId, requested: &[(CardId, Obj)], options: &[AttackerOpt], valid: &[(CardId, Obj)]) {
        let mut notes: Vec<String> = vec![];
        for (card, _) in requested {
            if valid.iter().any(|(c, _)| c == card) {
                continue;
            }
            if !options.iter().any(|o| o.card == *card) {
                notes.push(self.explain_attack(p, *card));
            } else {
                notes.push(format!("{} didn't attack: an effect (an attack limit, \"can't attack alone\" / \"unless another creature attacks\", or a cost) stopped it.", self.cname(*card)));
            }
        }
        for (card, _) in valid {
            if !requested.iter().any(|(c, _)| c == card) {
                notes.push(format!("{} attacks because an effect says it must attack (if able).", self.cname(*card)));
            }
        }
        for t in notes {
            self.notice(p, t);
        }
    }

    /// After blockers were declared: tell the player about blocks that were ignored or added and why.
    pub(crate) fn note_block_changes(&mut self, p: PlayerId, requested: &[(CardId, CardId)], options: &[BlockerOpt], pairs: &[(CardId, CardId)]) {
        let mut notes: Vec<String> = vec![];
        for (b, a) in requested {
            if pairs.contains(&(*b, *a)) || notes.len() > 6 {
                continue;
            }
            let (bn, an) = (self.cname(*b), self.cname(*a));
            if !options.iter().any(|o| o.card == *b && o.can_block.contains(a)) {
                notes.push(self.why_cant_block(*b, *a).unwrap_or_else(|| format!("{bn} can't block {an}.")));
                continue;
            }
            let ak = &self.cards[a.0 as usize].cur;
            let mine_on_a = requested.iter().filter(|(_, x)| x == a).count();
            if requested.iter().filter(|(x, y)| x == b && options.iter().any(|o| o.card == *b && o.can_block.contains(y))).count() > 1 + self.cards[b.0 as usize].cur.extra_blocks as usize {
                notes.push(format!("{bn} can block only one attacker."));
            } else if ak.kw.has(Keyword::Menace) && mine_on_a < 2 {
                notes.push(format!("{an} has menace: it can't be blocked except by two or more creatures, so {bn}'s block alone was ignored."));
            } else if ak.min_blockers > 1 && (mine_on_a as u8) < ak.min_blockers {
                notes.push(format!("{an} can't be blocked except by {} or more creatures, so the block by {bn} was ignored.", ak.min_blockers));
            } else if ak.max_blockers > 0 && mine_on_a > ak.max_blockers as usize {
                notes.push(format!("{an} can't be blocked by more than {} creature(s): the extra block by {bn} was ignored.", ak.max_blockers));
            } else if ak.block_all {
                notes.push(format!("{an} can't be blocked unless all your creatures block it: the block by {bn} was ignored."));
            } else if self.cards[b.0 as usize].cur.cant_block_alone {
                notes.push(format!("{bn} can't block alone, so its block was ignored."));
            } else if self.cards[b.0 as usize].cur.cant_block_unless_greater || self.cards[b.0 as usize].cur.block_needs_others > 0 {
                notes.push(format!("{bn} can't block unless other (or bigger) creatures also block, so its block was ignored."));
            } else {
                notes.push(format!("The block of {an} by {bn} was ignored: a restriction (a cost to block, or a limit on how many creatures can block) stopped it."));
            }
        }
        for (b, a) in pairs {
            if !requested.contains(&(*b, *a)) {
                notes.push(format!("{} blocks {} because an effect says it must block (if able).", self.cname(*b), self.cname(*a)));
            }
        }
        for t in notes {
            self.notice(p, t);
        }
    }

    /// Why a permanent can't be tapped for mana right now.
    pub fn explain_tap(&self, p: PlayerId, card: CardId) -> String {
        let c = &self.cards[card.0 as usize];
        let name = self.cname(card);
        if c.cur.controller != p {
            return format!("{name} is controlled by {}: you can't tap it.", self.pname(c.cur.controller));
        }
        match &self.pending {
            Some(d) if d.player != p => return format!("It isn't your decision: the game is waiting for {}.", self.pname(d.player)),
            None => return "The game is between decisions: wait until it asks you for priority.".into(),
            Some(d) if !matches!(d.kind, crate::decision::DecisionKind::Priority { .. } | crate::decision::DecisionKind::PayMana { .. }) => return "Answer the open question first (see the prompt on the table).".into(),
            _ => {}
        }
        let d = self.def_of(card);
        if d.mana_abilities.is_empty() && c.cur.granted.iter().all(|a| !crate::carddb::is_mana_ability(a)) {
            return format!("{name} has no mana ability.");
        }
        if c.tapped {
            return format!("{name} is already tapped.");
        }
        if d.types.has(CardType::Creature) && !self.can_use_tap_symbol(card) {
            return format!("{name} has summoning sickness (it came under your control this turn), so it can't tap for mana yet.");
        }
        format!("{name} can't make mana right now (an effect stops it, or the cost of its ability can't be paid).")
    }
}
