//! Evaluating filters, amounts, selectors and conditions against the current game state.

use crate::carddb::Def;
use crate::dsl::*;
use crate::mana::{ManaType, Shard};
use crate::ids::*;
use crate::state::*;
use crate::types::*;

/// What an ability evaluation can refer to.
pub struct EvalCtx<'a> {
    pub source: CardId,
    pub controller: PlayerId,
    pub x: i32,
    pub targets: &'a [Target],
    /// Position in `targets` of the first target chosen for each target spec (255 = none chosen).
    pub tidx: &'a [u8],
    pub event: &'a EventCtx,
    pub remembered: &'a [Obj],
}

static EMPTY_EVENT: EventCtx = EventCtx { obj: None, source: None, player: None, amount: 0, dmg_combat: false, lki: None, batch_power: 0, card_powers: Vec::new(), damaged: None, granted_by: None, counter_kind: None, counter_kinds: Vec::new(), cards: Vec::new(), entering: false, attacking: false, stack: None, mana_colors: 0, ts: 0, this_zcc: None };

impl<'a> EvalCtx<'a> {
    pub fn of(g: &Game, source: CardId) -> EvalCtx<'static> {
        EvalCtx {
            source,
            // a permanent whose control changed (Act of Aggression) is controlled by whoever controls it now
            controller: if g.cards[source.0 as usize].zone == Zone::Battlefield { g.cards[source.0 as usize].cur.controller } else { g.cards[source.0 as usize].controller },
            x: g.cards[source.0 as usize].x,
            targets: &[],
            tidx: &[],
            event: &EMPTY_EVENT,
            remembered: &[],
        }
    }
    pub fn target(&self, idx: u8) -> Option<&'a Target> {
        let p = *self.tidx.get(idx as usize)?;
        if p == 255 {
            None
        } else {
            self.targets.get(p as usize)
        }
    }
    pub fn with_controller(source: CardId, controller: PlayerId) -> EvalCtx<'static> {
        EvalCtx { source, controller, x: 0, targets: &[], tidx: &[], event: &EMPTY_EVENT, remembered: &[] }
    }
}

/// A view of an object for filter matching: either a live card or last-known information.
pub struct ObjView<'a> {
    pub id: CardId,
    pub d: &'a Derived,
    pub def: &'a Def,
    pub owner: PlayerId,
    pub controller: PlayerId,
    pub token: bool,
    pub tapped: bool,
    pub damage: i32,
    pub counters: &'a [(CounterKind, u16)],
    pub live: bool,
    pub face_down: bool,
    /// Whether something was attached to it (for last-known information: when it left).
    pub had_attachment: Option<bool>,
    /// Permanents attached to it when it left (last-known information).
    pub lki_attachments: &'a [CardId],
}

thread_local! {
    /// Re-entrancy guard for evaluating a characteristic-defining */* of a spell inside a filter.
    static CDA_GUARD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Set while checking whether a card may be cast: a split card is then a spell of one half, not the total of both.
    pub(crate) static AS_SPELL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

impl Game {
    /// The value of `card`'s `mark` (`Effect::SetMark`): 0 when unset, or when it was set for a turn that is over.
    pub fn mark_of(&self, card: CardId, mark: crate::dsl::Mark) -> i32 {
        self.cards[card.0 as usize].marks.iter().find(|(m, _, t)| *m == mark && (*t == 0 || *t == self.turn)).map_or(0, |(_, n, _)| *n)
    }

    /// Mana value of a card as the rules see it: a split card off the stack has the total of both halves.
    pub(crate) fn mv_of(&self, id: CardId, def: &crate::carddb::Def) -> u32 {
        // a face-down spell or permanent has no mana cost: mana value 0 (CR 708.2)
        if (self.cards[id.0 as usize].face_down || self.cards[id.0 as usize].cast_alt == Some(crate::decision::FACE_DOWN_ALT)) && matches!(self.cards[id.0 as usize].zone, Zone::Stack | Zone::Battlefield) {
            return 0;
        }
        // a prototyped spell or permanent has the mana value of its prototype cost (CR 718.3b)
        if let Some(alt) = self.cards[id.0 as usize].cast_alt {
            if matches!(self.cards[id.0 as usize].zone, Zone::Stack | Zone::Battlefield) {
                let mut n = 0u8;
                for a in &def.abilities {
                    match a {
                        crate::dsl::Ability::AlternativeCost { .. } => n += 1,
                        crate::dsl::Ability::Prototype { cost, .. } if n > 0 && alt == n - 1 => {
                            if let Ok(mc) = crate::mana::ManaCost::parse(cost) {
                                return mc.mana_value(0);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        // the back face of a transforming double-faced card has no mana cost: on the battlefield its mana value is the front face's (CR 712.8e)
        if def.transforming && def.cost.shards.is_empty() && matches!(self.cards[id.0 as usize].zone, Zone::Battlefield | Zone::Stack) {
            if let Some(of) = def.other_face {
                let front = self.db.def(of);
                if !front.cost.shards.is_empty() {
                    return front.mana_value;
                }
            }
        }
        if def.split_mv > 0 && self.cards[id.0 as usize].zone != Zone::Stack && !AS_SPELL.with(|c| c.get()) { def.split_mv } else { def.mana_value }
    }

    /// The first card a selector names; a spell on the stack counts as its card ("the mana spent to cast target spell").
    pub(crate) fn first_card_or_spell(&self, of: &Sel, c: &EvalCtx) -> Option<CardId> {
        if let Some(id) = self.sel_cards(of, c).first().copied() {
            return Some(id);
        }
        self.sel_objs(of, c).into_iter().find_map(|o| match o {
            Obj::Stack { id } => self.stack.iter().find(|s| s.id == id).map(|s| s.source).or_else(|| self.stack_lki_src.iter().find(|(i, _)| *i == id).map(|(_, c)| *c)),
            _ => None,
        })
    }
    pub fn view(&self, id: CardId) -> ObjView<'_> {
        let c = &self.cards[id.0 as usize];
        ObjView {
            id,
            d: &c.cur,
            def: self.db.def(c.def),
            owner: c.owner,
            controller: if c.zone == Zone::Battlefield { c.cur.controller } else { c.owner },
            token: c.token,
            tapped: c.tapped,
            damage: c.damage,
            counters: &c.counters,
            live: true,
            face_down: c.face_down,
            had_attachment: None,
            lki_attachments: &[],
        }
    }

    pub fn view_lki<'a>(&'a self, l: &'a Lki) -> ObjView<'a> {
        ObjView {
            id: l.card,
            d: &l.derived,
            def: self.db.def(l.def),
            owner: self.cards[l.card.0 as usize].owner,
            controller: l.controller,
            token: l.token,
            tapped: false,
            damage: 0,
            counters: &l.counters,
            live: false,
            face_down: l.face_down,
            had_attachment: Some(!l.attachments.is_empty()),
            lki_attachments: &l.attachments,
        }
    }

    pub fn is_attacking(&self, id: CardId) -> bool {
        self.combat.as_ref().map_or(false, |c| c.attackers.iter().any(|a| a.card == id))
    }

    pub fn is_blocking(&self, id: CardId) -> bool {
        self.combat.as_ref().map_or(false, |c| c.attackers.iter().any(|a| a.blockers.contains(&id)))
    }

    /// Like `player_filter` but knows the source card, so `Enchanted` means "the player this card enchants".
    pub fn player_filter_src(&self, src: CardId, f: PlayerFilter, who: PlayerId, controller: PlayerId) -> bool {
        match f {
            PlayerFilter::Enchanted => self.cards[src.0 as usize].enchanted_player == Some(who),
            PlayerFilter::DealtDamageByThisThisTurn => self.dmg_log.iter().any(|r| r.source == Some(src) && r.target == Obj::player(who)),
            PlayerFilter::DealtCombatDamageByThisThisTurn => self.dmg_log.iter().any(|r| r.combat && r.source == Some(src) && r.target == Obj::player(who)),
            PlayerFilter::AttackingEnchanted => {
                let Some(ep) = self.cards[src.0 as usize].enchanted_player else { return false };
                who != controller
                    && self.combat.as_ref().map_or(false, |cb| {
                        cb.attackers.iter().any(|a| {
                            self.cards[a.card.0 as usize].cur.controller == who
                                && self.cards[a.card.0 as usize].zone == Zone::Battlefield
                                && match a.defender {
                                    Obj::Player { id } => id == ep,
                                    Obj::Card { id } => self.cards[id.0 as usize].cur.controller == ep,
                                    _ => false,
                                }
                        })
                    })
            }
            PlayerFilter::OtherThanEnchanted => self.cards[src.0 as usize].enchanted_player.map_or(false, |e| e != who),
            PlayerFilter::NotOwner => self.cards[src.0 as usize].owner != who,
            PlayerFilter::SourceOwner => self.cards[src.0 as usize].owner == who,
            PlayerFilter::TriggerPlayer => self.filter_event.borrow().as_ref().and_then(|e| e.player).map_or(false, |p| p == who),
            PlayerFilter::OtherThanTriggerPlayer => self.filter_event.borrow().as_ref().and_then(|e| e.player).map_or(false, |p| p != who),
            PlayerFilter::ControllerOfAttached => self.cards[src.0 as usize].attached_to.map_or(false, |h| self.cards[h.0 as usize].cur.controller == who),
            PlayerFilter::ChosenPlayer => self.cards[src.0 as usize].chosen_player == Some(who),
            PlayerFilter::DefendingPlayerOfThis => self.combat.as_ref().map_or(false, |cb| {
                cb.attackers.iter().any(|a| {
                    a.card == src
                        && match a.defender {
                            Obj::Player { id } => id == who,
                            Obj::Card { id } => self.cards[id.0 as usize].cur.controller == who,
                            _ => false,
                        }
                })
            }),
            PlayerFilter::DefendingPlayerOfAttached => self.combat.as_ref().map_or(false, |cb| {
                let Some(host) = self.cards[src.0 as usize].attached_to else { return false };
                cb.attackers.iter().any(|a| {
                    a.card == host
                        && match a.defender {
                            Obj::Player { id } => id == who,
                            Obj::Card { id } => self.cards[id.0 as usize].cur.controller == who,
                            _ => false,
                        }
                })
            }),
            PlayerFilter::MorePermanentsThanEachOther => !self.effect_ignored.iter().any(|(s, p, t)| *s == src && *p == who && *t == self.turn) && self.player_filter(f, who, controller),
            PlayerFilter::EverDamagedByThis => self.dmg_game.iter().any(|(s, t, _)| *s == src && *t == Obj::player(who)),
            PlayerFilter::OpponentEverDamagedByThis => self.is_opponent(who, controller) && self.dmg_game.iter().any(|(s, t, _)| *s == src && *t == Obj::player(who)),
            PlayerFilter::OpponentEverCombatDamagedBySameName => {
                let nm = &self.db.def(self.cards[src.0 as usize].def).display_name;
                self.is_opponent(who, controller) && self.dmg_game.iter().any(|(s, t, cb)| *cb && *t == Obj::player(who) && &self.db.def(self.cards[s.0 as usize].def).display_name == nm)
            }
            PlayerFilter::EverCombatDamagedByThis => self.dmg_game.iter().any(|(s, t, cb)| *cb && *s == src && *t == Obj::player(who)),
            PlayerFilter::EverCombatDamagedBySameName => {
                let nm = &self.db.def(self.cards[src.0 as usize].def).display_name;
                self.dmg_game.iter().any(|(s, t, cb)| *cb && *t == Obj::player(who) && &self.db.def(self.cards[s.0 as usize].def).display_name == nm)
            }
            PlayerFilter::ChoseFirstOption => self.last_named_choice(src, who) == Some(0),
            PlayerFilter::ChoseSecondOption => self.last_named_choice(src, who) == Some(1),
            _ => self.player_filter(f, who, controller),
        }
    }

    /// The option index `who` last picked with `Effect::EachPlayerChooses { remember: true }` for a card named like `src`.
    pub fn last_named_choice(&self, src: CardId, who: PlayerId) -> Option<u8> {
        let name = &self.db.def(self.cards[src.0 as usize].def).name;
        self.player_choices.iter().find(|(n, p, _)| *p == who && n == name).map(|(_, _, o)| *o)
    }

    pub fn player_filter(&self, f: PlayerFilter, who: PlayerId, controller: PlayerId) -> bool {
        match f {
            PlayerFilter::DealtDamageThisTurn => self.dmg_log.iter().any(|r| r.target == Obj::player(who)),
            PlayerFilter::DealtCombatDamageThisTurn => self.dmg_log.iter().any(|r| r.combat && r.target == Obj::player(who)),
            PlayerFilter::DealtNoncombatDamageThisTurn => self.dmg_log.iter().any(|r| !r.combat && r.target == Obj::player(who)),
            // these need the source card: see `player_filter_src`
            PlayerFilter::LeftOfYou => who != controller && who == self.next_player(controller),
            PlayerFilter::ChoseFirstOption | PlayerFilter::ChoseSecondOption => false,
            PlayerFilter::SourceOwner | PlayerFilter::OpponentEverDamagedByThis | PlayerFilter::OpponentEverCombatDamagedBySameName | PlayerFilter::AttackingEnchanted | PlayerFilter::OtherThanEnchanted | PlayerFilter::TriggerPlayer | PlayerFilter::OtherThanTriggerPlayer | PlayerFilter::NotOwner | PlayerFilter::DefendingPlayerOfThis | PlayerFilter::DefendingPlayerOfAttached | PlayerFilter::ControllerOfAttached | PlayerFilter::ChosenPlayer | PlayerFilter::DealtDamageByThisThisTurn | PlayerFilter::DealtCombatDamageByThisThisTurn | PlayerFilter::EverDamagedByThis | PlayerFilter::EverCombatDamagedByThis | PlayerFilter::EverCombatDamagedBySameName => false,
            PlayerFilter::Any => true,
            PlayerFilter::NotActive => who != self.active,
            PlayerFilter::You => who == controller,
            PlayerFilter::Opponent => self.is_opponent(who, controller),
            PlayerFilter::Team => !self.is_opponent(who, controller),
            PlayerFilter::Enchanted => false,
            PlayerFilter::MostLife => self.players.iter().all(|p| p.life <= self.players[who as usize].life),
            PlayerFilter::AttackedThisTurn => self.players[who as usize].attacked_turn,
            PlayerFilter::OpponentSearchedLibraryThisTurn => self.is_opponent(who, controller) && self.players[who as usize].searched_turn,
            PlayerFilter::OpponentAttackedThisTurn => self.is_opponent(who, controller) && self.players[who as usize].attacked_turn,
            PlayerFilter::AttackedYouThisTurn => who != controller && self.attacked_players_turn.contains(&(who, controller)),
            PlayerFilter::OpponentCastSpellThisTurn => self.is_opponent(who, controller) && self.players[who as usize].spells_cast > 0,
            PlayerFilter::CastSpellThisTurn => self.players[who as usize].spells_cast > 0,
            PlayerFilter::CastSorceryThisTurn => self.spells_log.iter().any(|(pl, card)| *pl == who && self.cards[card.0 as usize].cur.types.has(CardType::Sorcery)),
            PlayerFilter::CastNonartifactSpellThisTurn => self.spells_log.iter().any(|(pl, card)| *pl == who && !self.cards[card.0 as usize].cur.types.has(CardType::Artifact)),
            PlayerFilter::CastNonPhyrexianSpellThisTurn => self.spells_log.iter().any(|(pl, card)| {
                let d = &self.cards[card.0 as usize].cur;
                *pl == who && !d.subtypes.contains(&Sym::new("Phyrexian")) && !(d.kw.has(Keyword::Changeling) && d.types.has(CardType::Creature))
            }),
            PlayerFilter::LeastLife => self.players.iter().all(|p| p.life >= self.players[who as usize].life),
            PlayerFilter::MostLands | PlayerFilter::FewestCreatures | PlayerFilter::MostCreatures => {
                let ty = if matches!(f, PlayerFilter::MostLands) { CardType::Land } else { CardType::Creature };
                let n = |pl: PlayerId| self.battlefield.iter().filter(|id| self.cards[id.0 as usize].cur.controller == pl && self.cards[id.0 as usize].cur.types.has(ty)).count();
                let me = n(who);
                if matches!(f, PlayerFilter::FewestCreatures) {
                    self.players.iter().all(|p| n(p.id) >= me)
                } else {
                    self.players.iter().all(|p| n(p.id) <= me)
                }
            }
            PlayerFilter::MostWizards => {
                let w = Sym::new("Wizard");
                let n = |pl: PlayerId| self.battlefield.iter().filter(|id| self.cards[id.0 as usize].cur.controller == pl && self.cards[id.0 as usize].cur.subtypes.contains(&w)).count();
                let me = n(who);
                self.players.iter().all(|p| n(p.id) <= me)
            }
            PlayerFilter::TwoFewerGraveyardCreaturesThanYou => {
                let n = |pl: PlayerId| self.players[pl as usize].graveyard.iter().filter(|id| self.def_of(**id).types.has(CardType::Creature)).count();
                who != controller && n(who) + 2 <= n(controller)
            }
            PlayerFilter::Active => who == self.active,
            PlayerFilter::MostCardsInHand => self.players.iter().all(|p| p.hand.len() <= self.players[who as usize].hand.len()),
            PlayerFilter::OpponentMostLife => {
                who != controller
                    && self.players.iter().enumerate().all(|(i, p)| i as PlayerId == controller || p.life <= self.players[who as usize].life)
            }
            PlayerFilter::LostLifeThisTurn => self.players[who as usize].life_lost > 0,
            PlayerFilter::GraveyardSevenOrMore => self.players[who as usize].graveyard.len() >= 7,
            PlayerFilter::LifeBelowHalfStarting => self.players[who as usize].life * 2 < self.cfg.starting_life,
            PlayerFilter::OpponentWithMoreLifeThanActive => who != self.active && self.players[who as usize].life > self.players[self.active as usize].life,
            PlayerFilter::OpponentWithMoreLandsThanActive | PlayerFilter::OpponentWithMoreCreaturesThanActive => {
                let ty = if matches!(f, PlayerFilter::OpponentWithMoreLandsThanActive) { CardType::Land } else { CardType::Creature };
                let n = |pl: PlayerId| self.battlefield.iter().filter(|id| self.cards[id.0 as usize].cur.controller == pl && self.cards[id.0 as usize].cur.types.has(ty)).count();
                who != self.active && n(who) > n(self.active)
            }
            PlayerFilter::OpponentWithFewerGraveyardCreaturesThanActive => {
                let n = |pl: PlayerId| self.players[pl as usize].graveyard.iter().filter(|id| self.def_of(**id).types.has(CardType::Creature)).count();
                who != self.active && n(who) < n(self.active)
            }
            PlayerFilter::OpponentWithMoreCardsThanActive => who != self.active && self.players[who as usize].hand.len() > self.players[self.active as usize].hand.len(),
            PlayerFilter::MorePermanentsThanEachOther => {
                let n = |pl: PlayerId| self.battlefield.iter().filter(|id| self.cards[id.0 as usize].cur.controller == pl).count();
                let me = n(who);
                self.players.iter().all(|p| p.id == who || n(p.id) < me)
            }
            PlayerFilter::MoreLifeThanYou => self.players[who as usize].life > self.players[controller as usize].life,
            PlayerFilter::MoreCreaturesThanYou | PlayerFilter::MoreArtifactsThanYou | PlayerFilter::MoreEnchantmentsThanYou | PlayerFilter::MoreLandsThanYou => {
                let ty = match f {
                    PlayerFilter::MoreArtifactsThanYou => CardType::Artifact,
                    PlayerFilter::MoreEnchantmentsThanYou => CardType::Enchantment,
                    PlayerFilter::MoreLandsThanYou => CardType::Land,
                    _ => CardType::Creature,
                };
                let n = |pl: PlayerId| self.battlefield.iter().filter(|id| self.cards[id.0 as usize].cur.controller == pl && self.cards[id.0 as usize].cur.types.has(ty)).count();
                n(who) > n(controller)
            }
            PlayerFilter::TwoMoreCardsInHandThanYou => self.players[who as usize].hand.len() >= self.players[controller as usize].hand.len() + 2,
            PlayerFilter::MoreCardsInHandThanYou => self.players[who as usize].hand.len() > self.players[controller as usize].hand.len(),
        }
    }

    pub fn matches_card(&self, f: &Filter, id: CardId, source: CardId, src_controller: PlayerId) -> bool {
        self.matches_view(f, &self.view(id), source, src_controller)
    }

    /// A spell on the stack judged against a filter; a copy made "except it's red" (Fork) has that colour instead of its card's.
    pub fn matches_stack_spell(&self, f: &Filter, s: &StackItem, source: CardId, src_controller: PlayerId) -> bool {
        match s.copy_except.iter().find_map(|x| if let CopyExcept::SetColors { colors } = x { Some(colors) } else { None }) {
            Some(colors) if s.copy => {
                let mut d = self.cards[s.source.0 as usize].cur.clone();
                d.colors = ColorSet::of(colors);
                let mut v = self.view(s.source);
                v.d = &d;
                self.matches_view(f, &v, source, src_controller)
            }
            _ => self.matches_card(f, s.source, source, src_controller),
        }
    }

    /// A card type by its name (used when the chosen "type" is a card type such as Artifact).
    pub fn card_type_named(n: &str) -> Option<CardType> {
        [CardType::Artifact, CardType::Battle, CardType::Creature, CardType::Enchantment, CardType::Instant, CardType::Kindred, CardType::Land, CardType::Planeswalker, CardType::Sorcery].into_iter().find(|t| t.name() == n)
    }

    /// The object's current name (a copy's "except its name is ..." overrides the printed one).
    pub fn view_name(&self, v: &ObjView) -> String {
        // a face-down permanent or exiled face-down card has no name
        if v.face_down {
            return String::new();
        }
        v.d.name.map_or_else(|| v.def.name.clone(), Sym::name)
    }

    /// The names a Room permanent currently has: one per unlocked door (bit 0 = door 1, bit 1 = door 2). None for non-Room objects.
    pub fn room_door_names(&self, id: CardId) -> Option<Vec<String>> {
        let def = self.def_of(id);
        if !def.room {
            return None;
        }
        let doors = self.cards[id.0 as usize].doors;
        Some(def.name.split(" // ").enumerate().take(2).filter(|(bit, _)| doors & (1 << bit) != 0).map(|(_, part)| part.to_string()).collect())
    }

    /// Does the object have this name (its own, or any nonlegendary creature card's if it has all creature names)?
    pub fn view_has_name(&self, v: &ObjView, n: &str) -> bool {
        if self.view_name(v) == n {
            return true;
        }
        self.cards[v.id.0 as usize].cur.all_creature_names
            && self.db.find(n).map_or(false, |d| {
                let def = self.db.def(d);
                def.name == n && def.has_type(CardType::Creature) && !def.is_legendary()
            })
    }

    /// The colour `id` chose, or the one it had chosen when it left the battlefield (last known information: a source
    /// sacrificed as a cost, Floating Shield).
    pub(crate) fn chosen_color_lki(&self, id: CardId) -> Option<Color> {
        let c = &self.cards[id.0 as usize];
        c.chosen_color.or(c.left_chosen_color.filter(|_| c.zone != Zone::Battlefield))
    }

    /// The creature / card type `id` chose, or the one it had chosen when it left the battlefield.
    pub(crate) fn chosen_type_lki(&self, id: CardId) -> Option<Sym> {
        let c = &self.cards[id.0 as usize];
        c.chosen_type.or(c.left_chosen_type.filter(|_| c.zone != Zone::Battlefield))
    }

    /// A "chosen" protection / hexproof quality granted by `source` to another object, settled to what the SOURCE
    /// chose (the holder's own choices do not count: Skrelv's "hexproof from that colour", Guardian Archon). `None`
    /// when the source made no such choice; other qualities are returned unchanged.
    pub(crate) fn settle_chosen_quality(&self, source: CardId, p: &ProtectionFrom) -> Option<ProtectionFrom> {
        Some(match p {
            ProtectionFrom::ChosenColor => ProtectionFrom::Color { c: self.chosen_color_lki(source)? },
            ProtectionFrom::ChosenType => {
                let t = self.chosen_type_lki(source)?;
                match Self::card_type_named(&Sym::name(t)) {
                    Some(ct) => ProtectionFrom::Type { t: ct },
                    None => ProtectionFrom::Subtype { name: Sym::name(t) },
                }
            }
            ProtectionFrom::ChosenPlayer => ProtectionFrom::ControlledBy { p: Some(self.cards[source.0 as usize].chosen_player?), who: None },
            other => other.clone(),
        })
    }

    /// Protection from a chosen colour / type: does `src` match what `holder` chose?
    pub(crate) fn chosen_prot_hit(&self, holder: &Card, src: &Card, pr: &ProtectionFrom) -> bool {
        match pr {
            ProtectionFrom::ChosenColor => holder.chosen_color.map_or(false, |c| src.cur.colors.has(c)),
            ProtectionFrom::ChosenType => holder.chosen_type.map_or(false, |t| src.cur.subtypes.contains(&t) || Self::card_type_named(&Sym::name(t)).map_or(false, |ct| src.cur.types.has(ct))),
            ProtectionFrom::Matching { filter } => self.matches_card(filter, src.id, holder.id, holder.cur.controller),
            ProtectionFrom::ArtifactManaValues => {
                let mv = if src.cur.no_mana_cost { 0 } else { self.def_of(src.id).mana_value };
                self.battlefield.iter().any(|id| {
                    let a = &self.cards[id.0 as usize];
                    a.cur.controller == holder.cur.controller && a.cur.types.has(CardType::Artifact) && (if a.cur.no_mana_cost { 0 } else { self.def_of(*id).mana_value }) == mv
                })
            }
            _ => false,
        }
    }

    pub fn matches_view(&self, f: &Filter, v: &ObjView, source: CardId, src_controller: PlayerId) -> bool {
        if f.chosen_name {
            let wants = [self.cards[source.0 as usize].chosen_name.as_deref(), self.cards[source.0 as usize].chosen_name2.as_deref()];
            let c = &self.cards[v.id.0 as usize];
            // a double-faced card outside the battlefield has only its front face's name
            let half = |s: &str| -> String { s.split_once(" // ").map_or_else(|| s.to_string(), |(a, _)| a.to_string()) };
            let fd = self.db.def(c.front);
            let dfc = fd.other_face.is_some();
            let n = &if dfc { half(&self.view_name(v)) } else { self.view_name(v) };
            let nf = &if dfc { half(&fd.name) } else { fd.name.clone() };
            if !wants.iter().flatten().any(|w| *w == n || *w == nf || (c.zone != Zone::Hand && self.view_has_name(v, w))) {
                return false;
            }
        }
        if f.graveyard_top {
            // the topmost matching card of its owner's graveyard (the last one put there)
            let owner = self.cards[v.id.0 as usize].owner;
            let gy = &self.players[owner as usize].graveyard;
            let Some(pos) = gy.iter().position(|x| *x == v.id) else { return false };
            let rest = Filter { graveyard_top: false, ..f.clone() };
            if gy[pos + 1..].iter().any(|x| self.matches_card(&rest, *x, source, src_controller)) {
                return false;
            }
        }
        if f.suspected && !self.cards[v.id.0 as usize].suspected {
            return false;
        }
        if f.created_by_source {
            let ok = self.cards[v.id.0 as usize].made_by.map_or(false, |(s, z)| {
                let sc = &self.cards[s.0 as usize];
                s == source && (sc.zcc == z || (sc.zone != Zone::Battlefield && sc.zcc == z + 1))
            });
            if !ok {
                return false;
            }
        }
        if f.ring_bearer && !self.is_ring_bearer(v.id) {
            return false;
        }
        if f.commander && !self.cards[v.id.0 as usize].is_commander {
            return false;
        }
        if f.single_target {
            let one = self.stack.iter().any(|s| s.source == v.id && matches!(s.kind, ItemKind::Spell { .. }) && !s.targets.is_empty() && s.targets.iter().all(|t| t.obj == s.targets[0].obj))
                || self.exec_spell.borrow().as_ref().map_or(false, |(c, ts)| *c == v.id && !ts.is_empty() && ts.iter().all(|t| t.obj == ts[0].obj));
            if !one {
                return false;
            }
        }
        if f.not_targeted_by_same_name_ability {
            // Goblin Artisans: no ability from another source with the same name targets this spell
            let sid = self.stack.iter().find(|s| s.source == v.id && matches!(s.kind, ItemKind::Spell { .. })).map(|s| s.id);
            if let Some(sid) = sid {
                let name = &self.def_of(source).name;
                if self.stack.iter().any(|s| !matches!(s.kind, ItemKind::Spell { .. }) && s.source != source && self.def_of(s.source).name == *name && s.targets.iter().any(|t| t.obj == Obj::Stack { id: sid })) {
                    return false;
                }
            }
        }
        if f.spell_has_targets || f.spell_targets_permanent_or_player || f.spell_targets_single_opp_object || f.spell_targets.is_some() || f.spell_targets_stack.is_some() || f.spell_targets_you || f.spell_targets_opponent || f.spell_targets_enchanted_player || f.spell_targets_only_players {
            let item = self.stack.iter().find(|s| s.source == v.id && matches!(s.kind, ItemKind::Spell { .. }));
            let resolving = self.exec_spell.borrow().as_ref().filter(|(c, _)| *c == v.id).map(|(_, ts)| ts.clone());
            let targets: Vec<Target> = match (item, resolving) {
                (Some(item), _) => item.targets.clone(),
                (None, Some(ts)) => ts,
                (None, None) => return false,
            };
            if !self.spell_targets_match(f, &targets, source, src_controller) {
                return false;
            }
        }
        if f.spell_would_destroy_your_land {
            let item = self.stack.iter().find(|s| s.source == v.id && matches!(s.kind, ItemKind::Spell { .. }));
            let Some(item) = item else { return false };
            if !self.spell_would_destroy_land(item, src_controller) {
                return false;
            }
        }
        if f.not_commander && self.cards[v.id.0 as usize].is_commander {
            return false;
        }
        if f.has_x {
            // restricted mana paying for an activated ability: the ability's own cost is what must contain {X}
            let hx = if self.pay_is_ability.get() && self.cards[v.id.0 as usize].zone != Zone::Hand { self.pay_ability_has_x.get() } else { self.db.def(self.cards[v.id.0 as usize].def).cost.has_x() };
            if !hx {
                return false;
            }
        }
        if f.modal_spell && !self.db.def(self.cards[v.id.0 as usize].def).abilities.iter().any(|a| matches!(a, Ability::Modal { .. })) {
            return false;
        }
        if f.renowned && !self.cards[v.id.0 as usize].renowned {
            return false;
        }
        if f.transformed {
            let c = &self.cards[v.id.0 as usize];
            let fd = self.db.def(c.front);
            if !(c.def != c.front && fd.other_face == Some(c.def) && !fd.face_castable) {
                return false;
            }
        }
        if f.has_phyrexian_mana && !self.db.def(self.cards[v.id.0 as usize].def).cost.has_phyrexian() {
            return false;
        }
        if f.has_protection && self.cards[v.id.0 as usize].cur.protections.is_empty() {
            return false;
        }
        if f.protection_any_color && {
            let ps = &self.cards[v.id.0 as usize].cur.protections;
            !(ps.iter().any(|p| matches!(p, ProtectionFrom::Everything))
                || (ps.iter().any(|p| matches!(p, ProtectionFrom::Monocolored)) && ps.iter().any(|p| matches!(p, ProtectionFrom::Multicolored)))
                || [Color::W, Color::U, Color::B, Color::R, Color::G].iter().all(|c| ps.iter().any(|p| matches!(p, ProtectionFrom::Color { c: x } if x == c))))
        } {
            return false;
        }
        if f.in_chosen_sector {
            let want = match self.cards[source.0 as usize].chosen_type.map(|t| t.name()).as_deref() {
                Some("Alpha") => 1,
                Some("Beta") => 2,
                Some("Gamma") => 3,
                _ => 0,
            };
            if want == 0 || self.cards[v.id.0 as usize].sector != want {
                return false;
            }
        }
        if f.aura_could_enchant_source && !self.aura_hosts(v.id).map_or(false, |h| h.contains(&source)) {
            return false;
        }
        if f.can_be_enchanted_by_source && self.protected_from(v.id, source) {
            return false;
        }
        if let Some(w) = &f.has_keyword_word {
            let w = w.to_lowercase();
            let text = self.db.def(self.cards[v.id.0 as usize].def).oracle_text.to_lowercase();
            // "cycling" also covers basic landcycling and typecycling ("Plainscycling {1}", "Basic landcycling {1}{G}")
            let any_cycling = w == "cycling";
            let cycles = |part: &str| any_cycling && (part.starts_with("basic landcycling") || part.split(|c: char| c.is_whitespace() || c == '{').next().map_or(false, |first| first.ends_with("cycling")));
            if !text.lines().any(|l| l.split(',').any(|part| part.trim().starts_with(w.as_str()) || cycles(part.trim()))) {
                return false;
            }
        }
        if let Some(w) = &f.lacks_keyword_word {
            let w = w.to_lowercase();
            let text = self.db.def(self.cards[v.id.0 as usize].def).oracle_text.to_lowercase();
            if text.lines().any(|l| l.split(',').any(|part| part.trim().starts_with(w.as_str()))) {
                return false;
            }
            // rampage granted until end of turn by an `AddTrigger` bound to the creature (Rapid Fire)
            if w == "rampage" && self.temp_triggers.iter().any(|t| t.bound.contains(&v.id) && matches!(t.trigger, Trigger::BecomesBlocked { .. }) && format!("{:?}", t.d.effects).contains("BlockersOfTrigger")) {
                return false;
            }
        }
        if let Some(oc) = f.only_card {
            if v.id.0 != oc {
                return false;
            }
        }
        if let Some(n) = f.nth_spell_this_turn {
            if self.spells_log.get(n as usize - 1).map(|(_, c)| *c) != Some(v.id) {
                return false;
            }
        }
        if f.foretold && !(self.cards[v.id.0 as usize].zone == Zone::Exile && self.cards[v.id.0 as usize].exile_mark == Some(crate::dsl::ExileMark::Foretold)) {
            return false;
        }
        if f.same_name_as_cast_spell {
            if !self.spells_log.iter().any(|(_, c)| self.def_of(*c).name == self.def_of(v.id).name || self.view_has_name(v, &self.def_of(*c).name)) {
                return false;
            }
        }
        if f.granted_offspring && !(v.live && self.cards[v.id.0 as usize].granted_offspring) {
            return false;
        }
        if f.same_name_as_nontoken_permanent {
            let nm = self.view_name(v);
            if !self.battlefield.iter().any(|&o| o != v.id && !self.cards[o.0 as usize].token && (self.def_of(o).name == nm || self.view_has_name(v, &self.def_of(o).name))) {
                return false;
            }
        }
        if f.only_controllers_chosen_color || f.not_only_controllers_chosen_color {
            let chosen = self.cards[source.0 as usize].player_colors.iter().find(|(p, _)| *p == v.controller).map(|(_, c)| *c);
            let only = v.d.colors.count() == 1 && chosen.map_or(false, |c| v.d.colors.has(c));
            if (f.only_controllers_chosen_color && !only) || (f.not_only_controllers_chosen_color && only) {
                return false;
            }
        }
        if f.first_by_caster_this_turn {
            let Some(pos) = self.spells_log.iter().position(|(_, c)| *c == v.id) else { return false };
            let caster = self.spells_log[pos].0;
            let rest = Filter { first_by_caster_this_turn: false, controller: PlayerFilter::Any, ..f.clone() };
            if self.spells_log[..pos].iter().any(|(pl, card)| *pl == caster && self.matches_card(&rest, *card, source, src_controller)) {
                return false;
            }
        }
        if f.cast_for_warp {
            let c = &self.cards[v.id.0 as usize];
            let d = self.db.def(c.def);
            let warped = d.abilities.iter().position(|a| matches!(a, Ability::Warp { .. })).map_or(false, |p| {
                let idx = d.abilities[..p.saturating_sub(1)].iter().filter(|x| matches!(x, Ability::AlternativeCost { .. })).count() as u8;
                c.cast_alt == Some(idx)
            });
            if !warped {
                return false;
            }
        }
        if let Some(w) = f.was_cast {
            if self.cards[v.id.0 as usize].cast_from.is_some() != w {
                return false;
            }
        }
        if f.graveyard_top_card {
            let c = &self.cards[v.id.0 as usize];
            if c.zone != Zone::Graveyard || self.players[c.owner as usize].graveyard.last() != Some(&v.id) {
                return false;
            }
        }
        if f.cast_from.is_some() || f.not_cast_from.is_some() {
            let c = &self.cards[v.id.0 as usize];
            // a spell that was cast: the zone it was cast from (kept after it resolves); otherwise where the card is
            let z = if self.cost_probe.get() == Some(v.id) { c.zone } else { c.cast_from.unwrap_or(c.zone) };
            if f.cast_from.map_or(false, |w| w != z) || f.not_cast_from.map_or(false, |w| w == z) {
                return false;
            }
        }
        // a card exiled face down has no card types while it is in exile (Oblivion Sower can't take a land exiled that way)
        // (only land filters: a face-down exiled card is still castable through its own permission, e.g. Vivien's creature)
        if !f.types.is_empty() && f.types.iter().all(|t| *t == CardType::Land) && v.face_down && v.live && self.cards[v.id.0 as usize].zone == Zone::Exile {
            return false;
        }
        if !f.types.is_empty() && !f.types.iter().any(|t| v.d.types.has(*t)) {
            return false;
        }
        if f.not_types.iter().any(|t| v.d.types.has(*t)) {
            return false;
        }
        if f.all_types.iter().any(|t| !v.d.types.has(*t)) {
            return false;
        }
        if !f.any_of.is_empty() && !f.any_of.iter().any(|g| self.matches_view(g, v, source, src_controller)) {
            return false;
        }
        if f.chosen_type {
            let ok = self.cards[source.0 as usize].chosen_type.or(self.cards[source.0 as usize].left_chosen_type.filter(|_| self.cards[source.0 as usize].zone != Zone::Battlefield)).map_or(false, |t| {
                v.d.subtypes.contains(&t) || (v.d.kw.has(Keyword::Changeling) && v.d.types.has(CardType::Creature)) || Self::card_type_named(&t.name()).map_or(false, |ct| v.d.types.has(ct))
            });
            if !ok {
                return false;
            }
        }
        if f.chosen_type_any {
            let sc = &self.cards[source.0 as usize];
            if !sc.chosen_types.1.iter().any(|t| v.d.subtypes.contains(t) || (v.d.kw.has(Keyword::Changeling) && v.d.types.has(CardType::Creature))) {
                return false;
            }
        }
        if f.not_chosen_type_any {
            let sc = &self.cards[source.0 as usize];
            if sc.chosen_types.1.iter().any(|t| v.d.subtypes.contains(t) || (v.d.kw.has(Keyword::Changeling) && v.d.types.has(CardType::Creature))) {
                return false;
            }
        }
        if f.not_chosen_type {
            let has = self.cards[source.0 as usize].chosen_type.or(self.cards[source.0 as usize].left_chosen_type.filter(|_| self.cards[source.0 as usize].zone != Zone::Battlefield)).map_or(false, |t| {
                v.d.subtypes.contains(&t) || (v.d.kw.has(Keyword::Changeling) && v.d.types.has(CardType::Creature)) || Self::card_type_named(&t.name()).map_or(false, |ct| v.d.types.has(ct))
            });
            if has {
                return false;
            }
        }
        if f.not_chosen_color && self.cards[source.0 as usize].chosen_color.or(self.cards[source.0 as usize].left_chosen_color.filter(|_| self.cards[source.0 as usize].zone != Zone::Battlefield)).map_or(false, |c| v.d.colors.has(c)) {
            return false;
        }
        if f.chosen_color && !self.cards[source.0 as usize].chosen_color.or(self.cards[source.0 as usize].left_chosen_color.filter(|_| self.cards[source.0 as usize].zone != Zone::Battlefield)).map_or(false, |c| v.d.colors.has(c)) {
            return false;
        }
        if let Some(n) = &f.not_name {
            if self.view_name(v) == *n || v.def.display_name == *n {
                return false;
            }
        }
        if f.name_not_curse_on_enchanted {
            if let Some(ep) = self.cards[source.0 as usize].enchanted_player {
                let curse = Sym::new("Curse");
                if self.battlefield.iter().any(|b| {
                    let c = &self.cards[b.0 as usize];
                    c.enchanted_player == Some(ep) && c.cur.subtypes.contains(&curse) && self.def_of(*b).name == v.def.name
                }) {
                    return false;
                }
            }
        }
        if let Some(k) = f.top_of_graveyard {
            let gy = &self.players[self.cards[v.id.0 as usize].owner as usize].graveyard;
            let Some(pos) = gy.iter().position(|x| *x == v.id) else { return false };
            let g = Filter { top_of_graveyard: None, controller: PlayerFilter::Any, ..f.clone() };
            if gy[pos + 1..].iter().filter(|o| self.matches_card(&g, **o, source, src_controller)).count() >= k as usize {
                return false;
            }
        }
        if f.damaged_by_source_ever && !self.dmg_game.iter().any(|(s, t, _)| *s == source && *t == Obj::card(v.id)) {
            return false;
        }
        if f.toughness_gt_power && v.d.toughness <= v.d.power {
            return false;
        }
        if f.pt_unequal && v.d.power == v.d.toughness {
            return false;
        }
        if let Some(k) = &f.counter_put_by_you {
            if !self.counters_log.iter().any(|(id, _, ck, by, _, _)| *id == v.id && ck == k && *by == Some(src_controller)) {
                return false;
            }
        }
        if f.power_gt_base || f.power_ne_base {
            // a characteristic-defining power ("*") is part of the base power (Tarmogoyf)
            let cda = v.def.abilities.iter().find_map(|a| if let Ability::Static { effect: StaticEffect::CdaPT { power, .. }, .. } = a { Some(power.clone()) } else { None });
            let base = match cda {
                Some(p) => self.eval_amount(&p, &EvalCtx::with_controller(v.id, v.controller)),
                None => v.def.power.unwrap_or(0),
            };
            if f.power_gt_base && v.d.power <= base {
                return false;
            }
            if f.power_ne_base && v.d.power == base {
                return false;
            }
        }
        if let Some(ac) = &f.total_pt_vs {
            let fe = self.filter_event.borrow();
            let fr = self.filter_rem.borrow();
            let ft = self.filter_targets.borrow();
            let ec = EvalCtx { source, controller: src_controller, x: self.cards[source.0 as usize].x, targets: &ft.0, tidx: &ft.1, event: fe.as_ref().unwrap_or(&EMPTY_EVENT), remembered: &fr };
            let total = if self.cards[v.id.0 as usize].zone == Zone::Battlefield { v.d.power + v.d.toughness } else { v.def.power.unwrap_or(0) + v.def.toughness.unwrap_or(0) };
            if !ac.op.test(total, self.eval_amount(&ac.amount, &ec)) {
                return false;
            }
        }
        if let Some(m) = f.total_pt_max {
            if v.d.power + v.d.toughness > m {
                return false;
            }
        }
        // a card outside the battlefield with a characteristic-defining power/toughness compares it (Burrowguard Mentor in a graveyard)
        let (cda_p, cda_t) = if (f.power_vs.is_some() || f.toughness_vs.is_some()) && v.live && self.cards[v.id.0 as usize].zone != Zone::Battlefield && !CDA_GUARD.with(|g| g.get()) {
            CDA_GUARD.with(|g| g.set(true));
            let r = self.cda_pt_off_battlefield(v.id).unwrap_or((v.d.power, v.d.toughness));
            CDA_GUARD.with(|g| g.set(false));
            r
        } else {
            (v.d.power, v.d.toughness)
        };
        for (cmp, val) in [(&f.power_vs, cda_p), (&f.toughness_vs, cda_t)] {
            if let Some(ac) = cmp {
                let fe = self.filter_event.borrow();
                let fr = self.filter_rem.borrow();
                let ft = self.filter_targets.borrow();
                let x = self.cards[source.0 as usize].x;
                let ec = EvalCtx { source, controller: src_controller, x, targets: &ft.0, tidx: &ft.1, event: fe.as_ref().unwrap_or(&EMPTY_EVENT), remembered: &fr };
                if !ac.op.test(val, self.eval_amount(&ac.amount, &ec)) {
                    return false;
                }
            }
        }
        if let Some(sel) = &f.not_controlled_by {
            let fe = self.filter_event.borrow();
            let fr = self.filter_rem.borrow();
            let ft = self.filter_targets.borrow();
            let ec = EvalCtx { source, controller: src_controller, x: self.cards[source.0 as usize].x, targets: &ft.0, tidx: &ft.1, event: fe.as_ref().unwrap_or(&EMPTY_EVENT), remembered: &fr };
            if self.sel_players(sel, &ec).contains(&v.controller) {
                return false;
            }
        }
        if let Some(sel) = &f.owned_by {
            let fe = self.filter_event.borrow();
            let fr = self.filter_rem.borrow();
            let ft = self.filter_targets.borrow();
            let ec = EvalCtx { source, controller: src_controller, x: self.cards[source.0 as usize].x, targets: &ft.0, tidx: &ft.1, event: fe.as_ref().unwrap_or(&EMPTY_EVENT), remembered: &fr };
            if !self.sel_players(sel, &ec).contains(&self.cards[v.id.0 as usize].owner) {
                return false;
            }
        }
        if let Some(sel) = &f.defended_by {
            let fe = self.filter_event.borrow();
            let fr = self.filter_rem.borrow();
            let ft = self.filter_targets.borrow();
            let ec = EvalCtx { source, controller: src_controller, x: self.cards[source.0 as usize].x, targets: &ft.0, tidx: &ft.1, event: fe.as_ref().unwrap_or(&EMPTY_EVENT), remembered: &fr };
            if !self.sel_players(sel, &ec).contains(&self.defender_side(v.id)) {
                return false;
            }
        }
        if let Some(sel) = &f.controlled_by {
            let fe = self.filter_event.borrow();
            let fr = self.filter_rem.borrow();
            let ft = self.filter_targets.borrow();
            let ec = EvalCtx { source, controller: src_controller, x: self.cards[source.0 as usize].x, targets: &ft.0, tidx: &ft.1, event: fe.as_ref().unwrap_or(&EMPTY_EVENT), remembered: &fr };
            if !self.sel_players(sel, &ec).contains(&v.controller) {
                return false;
            }
        }
        if f.among_remembered {
            let fr = self.filter_rem.borrow();
            if !fr.iter().any(|o| matches!(o, Obj::Card { id } if *id == v.id)) {
                return false;
            }
        }
        if let Some(slot) = f.among_stored {
            let st = self.stored_view();
            if !st.get(slot as usize).map_or(false, |s| s.iter().any(|o| matches!(o, Obj::Card { id } if *id == v.id))) {
                return false;
            }
        }
        if let Some(slot) = f.not_among_stored {
            let st = self.stored_view();
            if st.get(slot as usize).map_or(false, |s| s.iter().any(|o| matches!(o, Obj::Card { id } if *id == v.id))) {
                return false;
            }
        }
        if f.not_trigger_object {
            let fe = self.filter_event.borrow();
            if fe.as_ref().map_or(false, |e| matches!(e.obj, Some(Obj::Card { id }) if id == v.id)) {
                return false;
            }
        }
        if f.not_among_remembered {
            let fr = self.filter_rem.borrow();
            if fr.iter().any(|o| matches!(o, Obj::Card { id } if *id == v.id)) {
                return false;
            }
        }
        if let Some(sel) = &f.same_name_as {
            let fe = self.filter_event.borrow();
            let fr = self.filter_rem.borrow();
            let ft = self.filter_targets.borrow();
            let ec = EvalCtx { source, controller: src_controller, x: self.cards[source.0 as usize].x, targets: &ft.0, tidx: &ft.1, event: fe.as_ref().unwrap_or(&EMPTY_EVENT), remembered: &fr };
            // a permanent that left the battlefield is looked at as it last was (a Clone that died as a Grizzly Bears: Remembrance)
            let mut names: Vec<_> = self
                .sel_cards(sel, &ec)
                .into_iter()
                .map(|id| match ec.event.lki.as_deref().filter(|l| l.card == id).or(self.last_lki.as_ref().filter(|l| l.card == id && self.cards[id.0 as usize].zone != Zone::Battlefield)) {
                    Some(l) if l.card == id && !l.face_down => l.derived.name.map_or_else(|| self.db.def(l.def).name.clone(), Sym::name),
                    _ => self.def_of(id).name.clone(),
                })
                .collect();
            // a targeted spell (Counterbore, Quash): its name survives even after it was countered
            for o in self.sel_objs(sel, &ec) {
                if let Obj::Stack { id } = o {
                    let def = self.stack.iter().find(|s| s.id == id).map(|s| s.def).or_else(|| self.stack_lki.iter().find(|(i, _)| *i == id).map(|(_, d)| *d));
                    if let Some(d) = def {
                        names.push(self.db.def(d).name.clone());
                    }
                }
            }
            if names.is_empty() || !names.iter().any(|n| self.view_has_name(v, n)) {
                return false;
            }
        }
        if let Some(cf) = &f.controller_controls {
            let ctrl = v.controller;
            if !self.battlefield.clone().iter().any(|&o| self.cards[o.0 as usize].cur.controller == ctrl && self.matches_card(cf, o, source, src_controller)) {
                return false;
            }
        }
        if let Some(w) = &f.controller_chose {
            let ctrl = v.controller;
            let ok = self.cards[source.0 as usize].player_pick.iter().any(|(p, peace)| *p == ctrl && (w.as_str() == "Peace") == *peace);
            if !ok {
                return false;
            }
        }
        if let Some(max) = &f.controller_lands_max {
            let ctrl = v.controller;
            let n = self.battlefield.iter().filter(|&&o| { let c = &self.cards[o.0 as usize].cur; c.controller == ctrl && c.types.has(CardType::Land) }).count() as u32;
            if n > **max {
                return false;
            }
        }
        if let Some(h) = &f.controller_hit_by {
            let ctrl = v.controller;
            let mut seen: Vec<CardId> = vec![];
            for r in self.dmg_log.iter().filter(|r| r.combat && r.target == Obj::Player { id: ctrl }) {
                if let Some(sid) = r.source {
                    if !seen.contains(&sid) && self.matches_card(&h.filter, sid, source, src_controller) {
                        seen.push(sid);
                    }
                }
            }
            if (seen.len() as u32) < h.at_least {
                return false;
            }
        }
        if let Some(cf) = &f.no_permanent_same_name {
            let nm = self.view_name(v);
            let mine: Vec<&str> = nm.split(" // ").collect();
            if self.battlefield.clone().iter().any(|&o| {
                if o == v.id || !self.matches_card(cf, o, source, src_controller) {
                    return false;
                }
                match self.room_door_names(o) {
                    // a Room's names are its unlocked doors' names: a Room with both doors locked has none
                    Some(ns) => ns.iter().any(|n| mine.contains(&n.as_str())),
                    None => self.def_of(o).name == nm || self.view_has_name(v, &self.def_of(o).name),
                }
            }) {
                return false;
            }
        }
        if let Some(sel) = &f.not_same_name_as {
            let fe = self.filter_event.borrow();
            let fr = self.filter_rem.borrow();
            let ft = self.filter_targets.borrow();
            let ec = EvalCtx { source, controller: src_controller, x: self.cards[source.0 as usize].x, targets: &ft.0, tidx: &ft.1, event: fe.as_ref().unwrap_or(&EMPTY_EVENT), remembered: &fr };
            let nm = self.view_name(v);
            if self.sel_cards(sel, &ec).into_iter().any(|id| self.def_of(id).name == nm || self.view_has_name(v, &self.def_of(id).name)) {
                return false;
            }
        }
        if let Some(sel) = &f.skip_first_same_name_as {
            let fe = self.filter_event.borrow();
            let fr = self.filter_rem.borrow();
            let ft = self.filter_targets.borrow();
            let ec = EvalCtx { source, controller: src_controller, x: self.cards[source.0 as usize].x, targets: &ft.0, tidx: &ft.1, event: fe.as_ref().unwrap_or(&EMPTY_EVENT), remembered: &fr };
            let nm = self.view_name(v);
            if self.sel_cards(sel, &ec).into_iter().any(|id| self.def_of(id).name == nm) {
                let first = self.cards.iter().enumerate().find(|(_, c)| c.zone == Zone::Exile && c.exiled_with.map_or(false, |(s, _)| s == source) && self.db.def(c.def).name == nm).map(|(i, _)| i);
                if first == Some(v.id.0 as usize) {
                    return false;
                }
            }
        }
        if let Some(sel) = &f.except {
            let fe = self.filter_event.borrow();
            let fr = self.filter_rem.borrow();
            let ft = self.filter_targets.borrow();
            let ec = EvalCtx { source, controller: src_controller, x: self.cards[source.0 as usize].x, targets: &ft.0, tidx: &ft.1, event: fe.as_ref().unwrap_or(&EMPTY_EVENT), remembered: &fr };
            if self.sel_cards(sel, &ec).contains(&v.id) {
                return false;
            }
        }
        if f.attacking_same_as.is_some() || f.attacking_player.is_some() {
            let def = self.combat.as_ref().and_then(|cb| cb.attackers.iter().find(|a| a.card == v.id)).map(|a| a.defender);
            let Some(def) = def else { return false };
            let fe = self.filter_event.borrow();
            let fr = self.filter_rem.borrow();
            let ft = self.filter_targets.borrow();
            let ec = EvalCtx { source, controller: src_controller, x: self.cards[source.0 as usize].x, targets: &ft.0, tidx: &ft.1, event: fe.as_ref().unwrap_or(&EMPTY_EVENT), remembered: &fr };
            if let Some(sel) = &f.attacking_same_as {
                let same = self.sel_cards(sel, &ec).into_iter().any(|o| self.combat.as_ref().map_or(false, |cb| cb.attackers.iter().any(|a| a.card == o && a.defender == def)));
                if !same {
                    return false;
                }
            }
            if let Some(sel) = &f.attacking_player {
                let who = match def {
                    Obj::Player { id } => id,
                    Obj::Card { .. } if f.attacking_player_direct => return false,
                    Obj::Card { id } => self.cards[id.0 as usize].cur.controller,
                    _ => return false,
                };
                if !self.sel_players(sel, &ec).contains(&who) {
                    return false;
                }
            }
        }
        if let Some(sel) = &f.only {
            let fe = self.filter_event.borrow();
            let fr = self.filter_rem.borrow();
            let ft = self.filter_targets.borrow();
            let ec = EvalCtx { source, controller: src_controller, x: self.cards[source.0 as usize].x, targets: &ft.0, tidx: &ft.1, event: fe.as_ref().unwrap_or(&EMPTY_EVENT), remembered: &fr };
            if !self.sel_cards(sel, &ec).contains(&v.id) {
                return false;
            }
        }
        if let Some(sel) = &f.damaged_by {
            let fe = self.filter_event.borrow();
            let fr = self.filter_rem.borrow();
            let ft = self.filter_targets.borrow();
            let ec = EvalCtx { source, controller: src_controller, x: self.cards[source.0 as usize].x, targets: &ft.0, tidx: &ft.1, event: fe.as_ref().unwrap_or(&EMPTY_EVENT), remembered: &fr };
            let by = &self.cards[v.id.0 as usize].damaged_by;
            if !self.sel_cards(sel, &ec).iter().any(|c| by.contains(c)) {
                return false;
            }
        }
        if let Some(hf) = &f.attached_host {
            match self.cards[v.id.0 as usize].attached_to {
                Some(h) if self.matches_card(hf, h, source, src_controller) => {}
                _ => return false,
            }
        }
        if let Some(sel) = &f.targeted_by {
            let fe = self.filter_event.borrow();
            let fr = self.filter_rem.borrow();
            let ft = self.filter_targets.borrow();
            let ec = EvalCtx { source, controller: src_controller, x: self.cards[source.0 as usize].x, targets: &ft.0, tidx: &ft.1, event: fe.as_ref().unwrap_or(&EMPTY_EVENT), remembered: &fr };
            let probe = Sel::SpellTargets { spell: sel.clone(), filter: (Filter::default()).into() };
            if !self.sel_cards(&probe, &ec).contains(&v.id) {
                return false;
            }
        }
        if let Some(sel) = &f.legal_target_of {
            let fe = self.filter_event.borrow();
            let fr = self.filter_rem.borrow();
            let ft = self.filter_targets.borrow();
            let ec = EvalCtx { source, controller: src_controller, x: self.cards[source.0 as usize].x, targets: &ft.0, tidx: &ft.1, event: fe.as_ref().unwrap_or(&EMPTY_EVENT), remembered: &fr };
            let probe = Sel::LegalTargetsOf { spell: sel.clone(), filter: (Filter::default()).into(), players: false, except_current: true };
            if !self.sel_objs(&probe, &ec).contains(&Obj::card(v.id)) {
                return false;
            }
        }
        if let Some(sel) = &f.shares_type_with {
            let fe = self.filter_event.borrow();
            let fr = self.filter_rem.borrow();
            let ft = self.filter_targets.borrow();
            let ec = EvalCtx { source, controller: src_controller, x: self.cards[source.0 as usize].x, targets: &ft.0, tidx: &ft.1, event: fe.as_ref().unwrap_or(&EMPTY_EVENT), remembered: &fr };
            let others = self.sel_cards(sel, &ec);
            let share_one = |o: &CardId| {
                let od = &self.cards[o.0 as usize];
                let osubs: Vec<Sym> = if od.left_face_down && od.zone != Zone::Battlefield { vec![] } else if od.zone == Zone::Battlefield { od.cur.subtypes.iter().copied().collect() } else { self.db.def(od.def).subtypes.iter().copied().collect() };
                v.d.subtypes.iter().any(|s| osubs.contains(s)) || v.def.subtypes.iter().any(|s| osubs.contains(s)) || self.changeling_shares(v.id, *o, !osubs.is_empty())
            };
            let shares = if f.shares_type_with_each { !others.is_empty() && others.iter().all(share_one) } else { others.iter().any(share_one) };
            if !shares {
                return false;
            }
        }
        if let Some(sel) = &f.not_shares_type_with {
            let fe = self.filter_event.borrow();
            let fr = self.filter_rem.borrow();
            let ft = self.filter_targets.borrow();
            let ec = EvalCtx { source, controller: src_controller, x: self.cards[source.0 as usize].x, targets: &ft.0, tidx: &ft.1, event: fe.as_ref().unwrap_or(&EMPTY_EVENT), remembered: &fr };
            let others = self.sel_cards(sel, &ec);
            let shares = others.iter().any(|o| {
                let od = &self.cards[o.0 as usize];
                let osubs: Vec<Sym> = if od.zone == Zone::Battlefield { od.cur.subtypes.iter().copied().collect() } else { self.db.def(od.def).subtypes.iter().copied().collect() };
                v.d.subtypes.iter().any(|s| osubs.contains(s)) || v.def.subtypes.iter().any(|s| osubs.contains(s)) || self.changeling_shares(v.id, *o, !osubs.is_empty())
            });
            if shares {
                return false;
            }
        }
        if f.least_toughness && self.battlefield.iter().any(|b| {
            let bc = &self.cards[b.0 as usize];
            bc.cur.types.has(CardType::Creature) && bc.cur.toughness < v.d.toughness
        }) {
            return false;
        }
        if let Some(sel) = &f.shares_color_with {
            let fe = self.filter_event.borrow();
            let fr = self.filter_rem.borrow();
            let ft = self.filter_targets.borrow();
            let ec = EvalCtx { source, controller: src_controller, x: self.cards[source.0 as usize].x, targets: &ft.0, tidx: &ft.1, event: fe.as_ref().unwrap_or(&EMPTY_EVENT), remembered: &fr };
            let others = self.sel_cards(sel, &ec);
            let shares = others.iter().any(|o| {
                let od = &self.cards[o.0 as usize];
                let oc = if matches!(od.zone, Zone::Battlefield | Zone::Stack) { od.cur.colors } else { self.db.def(od.def).colors };
                v.d.colors.intersects(oc) || v.def.colors.intersects(oc)
            });
            if !shares {
                return false;
            }
        }
        if f.shares_color_last_spell {
            let shares = self.spells_log.last().map_or(false, |(_, o)| {
                let od = &self.cards[o.0 as usize];
                let oc = if matches!(od.zone, Zone::Battlefield | Zone::Stack) { od.cur.colors } else { self.db.def(od.def).colors };
                v.d.colors.intersects(oc) || v.def.colors.intersects(oc)
            });
            if !shares {
                return false;
            }
        }
        if let Some(sel) = &f.could_produce_mana_of {
            let fe = self.filter_event.borrow();
            let fr = self.filter_rem.borrow();
            let ft = self.filter_targets.borrow();
            let ec = EvalCtx { source, controller: src_controller, x: self.cards[source.0 as usize].x, targets: &ft.0, tidx: &ft.1, event: fe.as_ref().unwrap_or(&EMPTY_EVENT), remembered: &fr };
            let others = self.sel_cards(sel, &ec);
            drop((fe, fr, ft));
            let mine = self.card_mana_types(v.id);
            if !others.iter().any(|o| self.card_mana_types(*o).iter().any(|t| mine.contains(t))) {
                return false;
            }
        }
        if f.could_produce_colorless && !self.card_mana_types(v.id).contains(&crate::mana::ManaType::C) {
            return false;
        }
        if f.has_nonmana_activated && !v.def.abilities.iter().any(|a| matches!(a, Ability::Activated { .. }) && !crate::carddb::is_mana_ability(a)) {
            return false;
        }
        if f.has_mana_ability && !v.def.abilities.iter().any(|a| crate::carddb::is_mana_ability(a) || matches!(a, Ability::Triggered { trigger: Trigger::TapsForMana { .. }, effects, .. } if effects.iter().any(|e| matches!(e, Effect::AddMana { .. })))) {
            return false;
        }
        if f.no_mana_ability && v.def.abilities.iter().any(crate::carddb::is_mana_ability) {
            return false;
        }
        if f.has_tap_ability && !v.def.abilities.iter().any(|a| matches!(a, Ability::Activated { cost, loyalty: None, .. } if cost.iter().any(|c| matches!(c, Cost::Tap)))) && !self.cards[v.id.0 as usize].cur.granted.iter().any(|a| matches!(&**a, Ability::Activated { cost, loyalty: None, .. } if cost.iter().any(|c| matches!(c, Cost::Tap)))) {
            return false;
        }
        if f.prepared_spell && !self.cards[v.id.0 as usize].cast_prepared {
            return false;
        }
        if let Some(sel) = &f.shares_card_type_with {
            let fe = self.filter_event.borrow();
            let fr = self.filter_rem.borrow();
            let ft = self.filter_targets.borrow();
            let ec = EvalCtx { source, controller: src_controller, x: self.cards[source.0 as usize].x, targets: &ft.0, tidx: &ft.1, event: fe.as_ref().unwrap_or(&EMPTY_EVENT), remembered: &fr };
            let others = self.sel_cards(sel, &ec);
            let shares = others.iter().any(|o| {
                let od = &self.cards[o.0 as usize];
                let ot = if matches!(od.zone, Zone::Battlefield | Zone::Stack) { od.cur.types } else { self.db.def(od.def).types };
                v.d.types.intersects(ot) || v.def.types.intersects(ot)
            });
            if !shares {
                return false;
            }
        }
        if !f.printed_subtypes.is_empty() && !f.printed_subtypes.iter().any(|t| v.def.subtypes.contains(&Sym::new(t))) {
            return false;
        }
        if f.not_printed_subtypes.iter().any(|t| v.def.subtypes.contains(&Sym::new(t))) {
            return false;
        }
        if !f.printed_types.is_empty() && !f.printed_types.iter().any(|t| v.def.types.has(*t)) {
            return false;
        }
        if f.not_printed_types.iter().any(|t| v.def.types.has(*t)) {
            return false;
        }
        if f.artifact_mana_spent && self.cards[v.id.0 as usize].src_spent[ManaSourceKind::Artifact as usize] == 0 {
            return false;
        }
        if f.had_to_attack && self.cards[v.id.0 as usize].had_to_attack_turn != self.turn {
            return false;
        }
        if f.counter_put_by_you_this_turn {
            let c = &self.cards[v.id.0 as usize];
            if c.counter_put_turn != self.turn || c.counter_put_mask & (1 << self.cards[source.0 as usize].controller) == 0 {
                return false;
            }
        }
        if f.shares_color_with_activation_mana {
            let spent = self.cards[source.0 as usize].act_spent;
            let colors = ColorSet(spent[..5].iter().enumerate().fold(0u8, |a, (i, &n)| if n > 0 { a | (1 << i) } else { a }));
            if !v.d.colors.intersects(colors) {
                return false;
            }
        }
        if f.payable_by_activation_mana {
            let mut pool = self.cards[source.0 as usize].act_spent;
            let mut generic: i32 = 0;
            let mut ok = true;
            let mut take = |pool: &mut [u8; 6], i: usize| {
                if pool[i] > 0 {
                    pool[i] -= 1;
                    true
                } else {
                    false
                }
            };
            for sh in v.def.cost.shards.iter() {
                match sh {
                    Shard::Generic { n } => generic += *n as i32,
                    Shard::Snow => generic += 1,
                    Shard::X => {}
                    Shard::Color { c } => ok &= take(&mut pool, ManaType::of(*c) as usize),
                    Shard::Colorless => ok &= take(&mut pool, 5),
                    Shard::Hybrid { a, b } => ok &= take(&mut pool, ManaType::of(*a) as usize) || take(&mut pool, ManaType::of(*b) as usize),
                    Shard::HybridGeneric { c } => {
                        if !take(&mut pool, ManaType::of(*c) as usize) {
                            generic += 2;
                        }
                    }
                    Shard::HybridColorless { c } => ok &= take(&mut pool, ManaType::of(*c) as usize) || take(&mut pool, 5),
                    Shard::Phyrexian { c } => {
                        take(&mut pool, ManaType::of(*c) as usize);
                    }
                    Shard::PhyrexianHybrid { a, b } => {
                        let _ = take(&mut pool, ManaType::of(*a) as usize) || take(&mut pool, ManaType::of(*b) as usize);
                    }
                }
            }
            if !ok || pool.iter().map(|&n| n as i32).sum::<i32>() < generic {
                return false;
            }
        }
        if f.fading && !v.def.abilities.iter().any(|a| matches!(a, Ability::EntersWithCounters { kind: CounterKind::Fade, .. })) {
            return false;
        }
        if f.enchants_creature {
            let ok = v.def.abilities.iter().any(|a| {
                matches!(a, Ability::Attach { to } if to.types == [CardType::Creature] && to.subtypes.is_empty() && to.not_types.is_empty() && to.colors.is_empty() && to.not_colors.is_empty() && to.keywords.is_empty() && to.not_keywords.is_empty() && to.supertypes.is_empty() && to.not_supertypes.is_empty() && matches!(to.controller, PlayerFilter::Any))
            });
            if !ok {
                return false;
            }
        }
        if let Some(sel) = &f.can_enchant {
            let to = v.def.abilities.iter().find_map(|a| if let Ability::Attach { to } = a { Some(to.clone()) } else { None });
            let Some(to) = to else { return false };
            let fe = self.filter_event.borrow();
            let fr = self.filter_rem.borrow();
            let ft = self.filter_targets.borrow();
            let ec = EvalCtx { source, controller: src_controller, x: self.cards[source.0 as usize].x, targets: &ft.0, tidx: &ft.1, event: fe.as_ref().unwrap_or(&EMPTY_EVENT), remembered: &fr };
            let hosts = self.sel_cards(sel, &ec);
            drop((fe, fr, ft));
            if !hosts.iter().any(|h| self.matches_card(&to, *h, v.id, src_controller)) {
                return false;
            }
        }
        if let Some(sel) = &f.enchantable_by {
            let fe = self.filter_event.borrow();
            let fr = self.filter_rem.borrow();
            let ft = self.filter_targets.borrow();
            let ec = EvalCtx { source, controller: src_controller, x: self.cards[source.0 as usize].x, targets: &ft.0, tidx: &ft.1, event: fe.as_ref().unwrap_or(&EMPTY_EVENT), remembered: &fr };
            let auras = self.sel_cards(sel, &ec);
            drop((fe, fr, ft));
            let ok = auras.iter().any(|a| {
                self.def_of(*a).abilities.iter().find_map(|ab| if let Ability::Attach { to } = ab { Some(to.clone()) } else { None }).map_or(false, |to| self.matches_card(&to, v.id, *a, src_controller))
            });
            if !ok {
                return false;
            }
        }
        if let Some(ac) = &f.mana_value_vs {
            let fe = self.filter_event.borrow();
            let mut ec = EvalCtx::with_controller(source, src_controller);
            ec.x = self.cards[source.0 as usize].x;
            let fr = self.filter_rem.borrow();
            let ft = self.filter_targets.borrow();
            let mut ec2 = EvalCtx { source, controller: src_controller, x: ec.x, targets: &ft.0, tidx: &ft.1, event: fe.as_ref().unwrap_or(&EMPTY_EVENT), remembered: &fr };
            std::mem::swap(&mut ec, &mut ec2);
            let w = self.eval_amount(&ac.amount, &ec);
            let xs = if self.cards[v.id.0 as usize].zone == Zone::Stack { self.stack.iter().find(|s| s.source == v.id).map_or(0, |s| s.x as i32) * v.def.cost.shards.iter().filter(|s| matches!(s, Shard::X)).count() as i32 } else { 0 };
            if !ac.op.test(if v.d.no_mana_cost { 0 } else { self.mv_of(v.id, v.def) as i32 + xs }, w) {
                return false;
            }
        }
        if f.mv_odd || f.mv_even {
            let mv = if v.d.no_mana_cost { 0 } else { self.mv_of(v.id, v.def) as i32 };
            if (mv % 2 == 1) != f.mv_odd {
                return false;
            }
        }
        if f.mv_le_controller_graveyard {
            let xs = if self.cards[v.id.0 as usize].zone == Zone::Stack { self.stack.iter().find(|s| s.source == v.id).map_or(0, |s| s.x as i32) * v.def.cost.shards.iter().filter(|s| matches!(s, Shard::X)).count() as i32 } else { 0 };
            let mv = if v.d.no_mana_cost { 0 } else { self.mv_of(v.id, v.def) as i32 + xs };
            if mv > self.players[v.controller as usize].graveyard.len() as i32 {
                return false;
            }
        }
        if f.supertypes.iter().any(|s| !v.d.supertypes.contains(s)) || f.not_supertypes.iter().any(|s| v.d.supertypes.contains(s)) {
            return false;
        }
        if let Some(of) = f.owner {
            if !self.player_filter_src(source, of, v.owner, src_controller) {
                return false;
            }
        }
        if f.attacking_you {
            let hit = self.combat.as_ref().map_or(false, |c| {
                c.attackers.iter().any(|a| {
                    a.card == v.id
                        && match a.defender {
                            Obj::Player { id } => id == src_controller,
                            Obj::Card { id } => self.cards[id.0 as usize].zone == Zone::Battlefield && self.cards[id.0 as usize].cur.controller == src_controller,
                            _ => false,
                        }
                })
            });
            if !hit {
                return false;
            }
        }
        if f.paid_with_mana_from_source && !self.cards[v.id.0 as usize].paid_src.contains(&source) {
            return false;
        }
        if f.attacking_chosen_player {
            let chosen = self.cards[source.0 as usize].chosen_player;
            let hit = chosen.is_some() && self.combat.as_ref().map_or(false, |c| {
                c.attackers.iter().any(|a| {
                    a.card == v.id
                        && match a.defender {
                            Obj::Player { id } => Some(id) == chosen,
                            Obj::Card { id } => Some(self.cards[id.0 as usize].cur.controller) == chosen,
                            _ => false,
                        }
                })
            });
            if !hit {
                return false;
            }
        }
        if f.banded_with_attacking_source {
            let hit = self.combat.as_ref().map_or(false, |c| {
                c.attackers.iter().find(|a| a.card == source).map_or(false, |sa| v.id == source || (sa.band != 0 && c.attackers.iter().any(|a| a.card == v.id && a.band == sa.band)))
            });
            if !hit {
                return false;
            }
        }
        if f.attacking_enchanted_player {
            let ep = self.cards[source.0 as usize].enchanted_player;
            let hit = ep.is_some() && self.combat.as_ref().map_or(false, |c| c.attackers.iter().any(|a| a.card == v.id && matches!(a.defender, Obj::Player { id } if Some(id) == ep)));
            if !hit {
                return false;
            }
        }
        if f.attacking_you_player {
            let hit = self.combat.as_ref().map_or(false, |c| c.attackers.iter().any(|a| a.card == v.id && matches!(a.defender, Obj::Player { id } if id == src_controller)));
            if !hit {
                return false;
            }
        }
        if f.attacking_opponent_player {
            let hit = self.combat.as_ref().map_or(false, |c| c.attackers.iter().any(|a| a.card == v.id && matches!(a.defender, Obj::Player { id } if id != src_controller)));
            if !hit {
                return false;
            }
        }
        if f.not_attacking_you {
            let other = self.combat.as_ref().map_or(false, |c| {
                c.attackers.iter().any(|a| {
                    a.card == v.id
                        && match a.defender {
                            Obj::Player { id } => id != src_controller,
                            Obj::Card { id } => self.cards[id.0 as usize].cur.controller != src_controller,
                            _ => false,
                        }
                })
            });
            if !other {
                return false;
            }
        }
        let changeling = v.d.kw.has(Keyword::Changeling);
        for s in &f.subtypes {
            let sym = Sym::new(s);
            if !v.d.subtypes.contains(&sym) && !(changeling && v.d.types.has(CardType::Creature)) {
                return false;
            }
        }
        for s in &f.not_subtypes {
            if v.d.subtypes.contains(&Sym::new(s)) || (changeling && v.d.types.has(CardType::Creature)) {
                return false;
            }
        }
        // a card turned (or put onto the battlefield) face down is colorless even before the layers are recomputed
        let fd_colorless = v.face_down && v.live;
        if !f.colors.is_empty() && (fd_colorless || !f.colors.iter().any(|c| v.d.colors.has(*c))) {
            return false;
        }
        if !f.all_colors.is_empty() && (fd_colorless || !f.all_colors.iter().all(|c| v.d.colors.has(*c))) {
            return false;
        }
        if !fd_colorless && f.not_colors.iter().any(|c| v.d.colors.has(*c)) {
            return false;
        }
        if f.colorless && !fd_colorless && !v.d.colors.is_colorless() {
            return false;
        }
        if f.has_activated_ability && (v.d.no_abilities || (v.def.activated.is_empty() && !v.d.granted.iter().any(|a| matches!(**a, Ability::Activated { .. })))) {
            return false;
        }
        if f.multicolored && v.d.colors.count() < 2 {
            return false;
        }
        if let Some(n) = f.color_count {
            if v.d.colors.count() != n as u32 {
                return false;
            }
        }
        if let Some(ac) = &f.color_count_vs {
            let fe = self.filter_event.borrow();
            let fr = self.filter_rem.borrow();
            let ft = self.filter_targets.borrow();
            let ec = EvalCtx { source, controller: src_controller, x: self.cards[source.0 as usize].x, targets: &ft.0, tidx: &ft.1, event: fe.as_ref().unwrap_or(&EMPTY_EVENT), remembered: &fr };
            if !ac.op.test(v.d.colors.count() as i32, self.eval_amount(&ac.amount, &ec)) {
                return false;
            }
        }
        if f.controlled_since_turn_start && self.cards[v.id.0 as usize].control_turn >= self.turn {
            return false;
        }
        if let Some(n) = f.not_color_count {
            if v.d.colors.count() == n as u32 {
                return false;
            }
        }
        if f.monocolored && v.d.colors.count() != 1 {
            return false;
        }
        if f.keywords.iter().any(|k| !v.d.kw.has(*k)) {
            return false;
        }
        if f.not_keywords.iter().any(|k| v.d.kw.has(*k)) {
            return false;
        }
        if !self.player_filter_src(source, f.controller, v.controller, src_controller) {
            return false;
        }
        if let Some(t) = f.tapped {
            if v.tapped != t {
                return false;
            }
        }
        if let Some(t) = f.face_down {
            // a spell cast face down (morph / disguise cast for {3}) is face down on the stack too
            let fd_spell = v.live && self.cards[v.id.0 as usize].zone == Zone::Stack && self.cards[v.id.0 as usize].cast_alt == Some(crate::decision::FACE_DOWN_ALT);
            if (v.face_down || fd_spell || (v.live && self.face_down_cast.get() == Some(v.id))) != t {
                return false;
            }
        }
        if let Some(t) = f.token {
            if v.token != t {
                return false;
            }
        }
        if let Some(t) = f.has_morph {
            if v.def.abilities.iter().any(|a| matches!(a, Ability::Morph { .. })) != t {
                return false;
            }
        }
        if let Some(t) = f.has_disguise {
            if v.def.abilities.iter().any(|a| matches!(a, Ability::Morph { disguise: true, .. })) != t {
                return false;
            }
        }
        if let Some(t) = f.kicked {
            if (v.live && self.cards[v.id.0 as usize].kicked) != t {
                return false;
            }
        }
        if let Some(t) = f.has_adventure {
            // a card cast as its Adventure half still "has an Adventure" (its front face carries the flag)
            let has = v.def.adventure || (v.live && self.db.def(self.cards[v.id.0 as usize].front).adventure);
            if has != t {
                return false;
            }
        }
        if let Some(t) = f.double_faced {
            if v.def.other_face.is_some() != t {
                return false;
            }
        }
        if let Some(t) = f.goaded {
            let goaded = if v.live {
                !self.goaded_by(v.id).is_empty()
            } else {
                // last-known information: it was goaded when it left (a dies trigger such as Baeloth Barrityl's); the record belongs to its latest life
                let zcc = self.cards[v.id.0 as usize].zcc;
                !v.d.static_goaders.is_empty() || self.goaded.iter().any(|(c, z, p, t)| *c == v.id && *z + 1 >= zcc && !(self.turn > *t && self.active == *p))
            };
            if goaded != t {
                return false;
            }
        }
        if f.modified {
            let ctl = v.controller;
            // a creature that already left is judged by what was attached when it left (an unattached Equipment doesn't remember)
            let attached: Vec<CardId> = if v.live { self.battlefield.iter().copied().filter(|a| self.cards[a.0 as usize].attached_to == Some(v.id)).collect() } else { v.lki_attachments.to_vec() };
            let has = !v.counters.is_empty()
                || attached.iter().any(|a| {
                    let x = &self.cards[a.0 as usize];
                    (x.cur.subtypes.contains(&Sym::new("Equipment"))) || (x.cur.subtypes.contains(&Sym::new("Aura")) && x.cur.controller == ctl)
                });
            if !has {
                return false;
            }
        }
        if let Some(p) = f.paired {
            if self.cards[v.id.0 as usize].paired.is_some() != p {
                return false;
            }
        }
        if f.no_land_type_shared_with_yours {
            let shared = self.battlefield.iter().any(|id| {
                let ca = &self.cards[id.0 as usize];
                ca.cur.types.has(CardType::Land) && ca.cur.controller == src_controller && ca.cur.subtypes.iter().any(|t| v.d.subtypes.contains(t))
            });
            if shared {
                return false;
            }
        }
        if f.paired_with_this {
            let sp = self.cards[source.0 as usize].paired;
            let ok = (v.id == source && sp.is_some()) || sp == Some(v.id);
            if !ok {
                return false;
            }
        }
        if f.exiled_this_turn && !self.exile_entries.iter().any(|(id, _)| *id == v.id) {
            return false;
        }
        if f.exiled_with_this {
            let on_bf = matches!(self.cards[source.0 as usize].zone, Zone::Battlefield | Zone::Command);
            let z = self.cards[source.0 as usize].zcc;
            let ok = self.cards[v.id.0 as usize].exiled_with.map_or(false, |(s, zz)| s == source && (!on_bf || zz == z))
                || (self.cards[v.id.0 as usize].pregame_for.is_some() && self.cards[v.id.0 as usize].pregame_for == Some(Sym::new(&self.def_of(source).name)));
            if !ok {
                return false;
            }
        }
        if f.enchanting_you && self.cards[v.id.0 as usize].enchanted_player != Some(src_controller) {
            return false;
        }
        if f.last_exiled_with_this && self.cards[source.0 as usize].last_exiled_with != Some(v.id) {
            return false;
        }
        if f.created_by_this {
            let on_bf = self.cards[source.0 as usize].zone == Zone::Battlefield;
            let z = self.cards[source.0 as usize].zcc;
            if !self.cards[v.id.0 as usize].created_by.map_or(false, |(s, zz)| s == source && (!on_bf || zz == z)) {
                return false;
            }
        }
        if f.played_via_this && !self.cards[v.id.0 as usize].played_via.map_or(false, |(s, zz)| s == source && zz == self.cards[source.0 as usize].zcc) {
            return false;
        }
        if let Some(sel) = &f.attached_to {
            let fe = self.filter_event.borrow();
            let fr = self.filter_rem.borrow();
            let ft = self.filter_targets.borrow();
            let ec = EvalCtx { source, controller: src_controller, x: self.cards[source.0 as usize].x, targets: &ft.0, tidx: &ft.1, event: fe.as_ref().unwrap_or(&EMPTY_EVENT), remembered: &fr };
            if !self.cards[v.id.0 as usize].attached_to.map_or(false, |h| self.sel_cards(sel, &ec).contains(&h)) {
                return false;
            }
        }
        if f.linked_with_this || f.not_linked_with_this {
            let sc = &self.cards[source.0 as usize];
            let ok = sc.linked.iter().chain(sc.linked_extra.iter()).any(|&(lid, z)| {
                let c = &self.cards[v.id.0 as usize];
                lid == v.id && (c.zcc == z || (c.zcc == z + 1 && c.zone != Zone::Battlefield))
            });
            if ok != f.linked_with_this {
                return false;
            }
        }
        if f.not_put_by_this && self.cause_src.get() == Some(source) {
            return false;
        }
        if f.not_played && self.cards[v.id.0 as usize].land_played {
            return false;
        }
        if let Some(na) = f.no_abilities {
            let none = v.d.no_abilities || (v.def.abilities.is_empty() && v.d.kw.0 == 0 && v.d.granted.is_empty());
            if none != na {
                return false;
            }
        }
        if f.other && v.id == source {
            return false;
        }
        if f.this && v.id != source {
            return false;
        }
        if let Some(c) = &f.base_power {
            if !v.d.base_pt.map(|(p, _)| p).or(v.def.power).map_or(false, |x| c.test(x)) {
                return false;
            }
        }
        if let Some(c) = &f.base_toughness {
            if !v.d.base_pt.map(|(_, t)| t).or(v.def.toughness).map_or(false, |x| c.test(x)) {
                return false;
            }
        }
        // a characteristic-defining */* works in every zone (a Tarmogoyf spell on the stack has its power)
        let (vp, vt) = if v.live && self.cards[v.id.0 as usize].zone == Zone::Stack && !CDA_GUARD.with(|g| g.get()) && (f.power.is_some() || f.toughness.is_some()) {
            CDA_GUARD.with(|g| g.set(true));
            let r = self.cda_pt_off_battlefield(v.id).unwrap_or((v.d.power, v.d.toughness));
            CDA_GUARD.with(|g| g.set(false));
            r
        } else {
            (v.d.power, v.d.toughness)
        };
        if let Some(c) = &f.power {
            if !c.test(vp) {
                return false;
            }
        }
        if let Some(c) = &f.toughness {
            if !c.test(vt) {
                return false;
            }
        }
        if let Some(c) = &f.mana_value {
            // a spell on the stack counts the X it was cast with (every {X} in its cost counts X)
            let on_stack_x = if self.cards[v.id.0 as usize].zone == Zone::Stack { self.cards[v.id.0 as usize].x.max(0) * v.def.cost.shards.iter().filter(|s| matches!(s, Shard::X)).count() as i32 } else { 0 };
            if !c.test(if v.d.no_mana_cost { 0 } else { self.mv_of(v.id, v.def) as i32 + on_stack_x }) {
                return false;
            }
        }
        let osx = if self.cards[v.id.0 as usize].zone == Zone::Stack { self.cards[v.id.0 as usize].x.max(0) * v.def.cost.shards.iter().filter(|s| matches!(s, Shard::X)).count() as i32 } else { 0 };
        if let Some(ev) = f.mana_value_even {
            if (if v.d.no_mana_cost { 0 } else { self.mv_of(v.id, v.def) as i32 + osx } % 2 == 0) != ev {
                return false;
            }
        }
        if f.mana_value_not_chosen_number {
            let mv = if v.d.no_mana_cost { 0 } else { self.mv_of(v.id, v.def) };
            if self.cards[source.0 as usize].chosen_number == Some(mv as i32) {
                return false;
            }
        }
        if f.mana_value_chosen_parity {
            let even = self.cards[source.0 as usize].chosen_type.map(|t| t.name() == "Even");
            let mv = if v.d.no_mana_cost { 0 } else { self.mv_of(v.id, v.def) as i32 + osx };
            if even.map_or(true, |e| (mv % 2 == 0) != e) {
                return false;
            }
        }
        if f.multi_type && v.d.types.0.count_ones() < 2 {
            return false;
        }
        if f.drawn_this_turn {
            let cd = &self.cards[v.id.0 as usize];
            if !(cd.zone == Zone::Hand && cd.drawn_turn == self.turn && cd.drawn_zcc == cd.zcc) {
                return false;
            }
        }
        if let Some(z) = f.in_zone {
            if self.cards[v.id.0 as usize].zone != z {
                return false;
            }
        }
        if f.loyalty_activated_this_turn && !self.cards[v.id.0 as usize].activations.iter().any(|(i, _)| *i == u16::MAX) {
            return false;
        }
        if f.last_drawn {
            let cd = &self.cards[v.id.0 as usize];
            if !(cd.zone == Zone::Hand && cd.drawn_turn == self.turn && cd.drawn_zcc == cd.zcc && self.players[cd.owner as usize].last_drawn == Some(v.id.0)) {
                return false;
            }
        }
        if let Some(ev) = f.power_even {
            if (v.d.power.rem_euclid(2) == 0) != ev {
                return false;
            }
        }
        if let Some(n) = &f.name {
            if !self.view_has_name(v, n) && !(self.cards[v.id.0 as usize].zone == Zone::Graveyard && v.def.abilities.iter().any(|a| matches!(a, Ability::GraveyardAlias { name } if name == n))) {
                return false;
            }
        }
        if f.not_attacking && self.is_attacking(v.id) {
            return false;
        }
        if f.not_blocking && self.is_blocking(v.id) {
            return false;
        }
        if f.combat_alone {
            let alone = self.combat.as_ref().map_or(false, |cb| {
                let blockers: Vec<CardId> = cb.attackers.iter().flat_map(|a| a.blockers.iter().copied()).collect();
                (cb.attackers.len() == 1 && cb.attackers[0].card == v.id) || (blockers.len() == 1 && blockers[0] == v.id)
            });
            if !alone {
                return false;
            }
        }
        if f.attacking && !self.is_attacking(v.id) {
            return false;
        }
        if f.blocking_source && !self.combat.as_ref().map_or(false, |cb| cb.attackers.iter().any(|a| a.card == source && a.blockers.contains(&v.id))) {
            return false;
        }
        if f.crewed_source_this_turn && !self.crew_log.iter().any(|(c, veh, t)| *c == v.id && *veh == source && *t == self.turn) {
            return false;
        }
        if f.tapped_for_source_cost_this_turn && !self.tap_cost_log.iter().any(|(c, src, t)| *c == v.id && *src == source && *t == self.turn) {
            return false;
        }
        if let Some(slot) = f.blocking_stored {
            let st = self.stored_view();
            let held = |a: CardId| st.get(slot as usize).map_or(false, |s| s.iter().any(|o| matches!(o, Obj::Card { id } if *id == a)));
            if !self.combat.as_ref().map_or(false, |cb| cb.attackers.iter().any(|a| held(a.card) && a.blockers.contains(&v.id))) {
                return false;
            }
        }
        if let Some(slot) = f.blocked_by_stored {
            let st = self.stored_view();
            let held = |b: CardId| st.get(slot as usize).map_or(false, |s| s.iter().any(|o| matches!(o, Obj::Card { id } if *id == b)));
            if !self.combat.as_ref().map_or(false, |cb| cb.attackers.iter().any(|a| a.card == v.id && a.blockers.iter().any(|b| held(*b)))) {
                return false;
            }
        }
        if f.blocking_attached {
            let eq = self.cards[source.0 as usize].attached_to.or_else(|| self.last_lki.as_ref().filter(|l| l.card == source && self.cards[source.0 as usize].zone != Zone::Battlefield).and_then(|l| l.attached_to));
            if !eq.map_or(false, |e| self.combat.as_ref().map_or(false, |cb| cb.attackers.iter().any(|a| a.card == e && a.blockers.contains(&v.id)))) {
                return false;
            }
        }
        if f.banded_with_source {
            let ok = v.id != source
                && self.combat.as_ref().map_or(false, |cb| {
                    let sb = cb.attackers.iter().find(|a| a.card == source).map_or(0, |a| a.band);
                    sb != 0 && cb.attackers.iter().any(|a| a.card == v.id && a.band == sb)
                });
            if !ok {
                return false;
            }
        }
        if f.blocked_by_attached {
            let eq = self.cards[source.0 as usize].attached_to.or_else(|| self.last_lki.as_ref().filter(|l| l.card == source && self.cards[source.0 as usize].zone != Zone::Battlefield).and_then(|l| l.attached_to));
            if !eq.map_or(false, |e| self.combat.as_ref().map_or(false, |cb| cb.attackers.iter().any(|a| a.card == v.id && a.blockers.contains(&e)))) {
                return false;
            }
        }
        if f.blocked_by_source && !self.combat.as_ref().map_or(false, |cb| cb.attackers.iter().any(|a| a.card == v.id && a.blockers.contains(&source))) {
            return false;
        }
        if f.beamable && self.cards[v.id.0 as usize].cur.cant_be_beamed {
            return false;
        }
        if f.sacrificable && (self.cards[v.id.0 as usize].cur.cant_be_sacrificed || self.trigger_token_barred(v.id) || (!v.d.types.has(CardType::Land) && self.nonland_sac_costs_barred())) {
            return false;
        }
        if f.no_counters && !self.cards[v.id.0 as usize].counters.is_empty() {
            return false;
        }
        if f.blocking && !self.is_blocking(v.id) {
            return false;
        }
        if f.blocked_attacker && !self.combat.as_ref().map_or(false, |c| c.attackers.iter().any(|a| a.card == v.id && a.blocked)) {
            return false;
        }
        if let Some(z) = f.entered_from {
            if self.cards[v.id.0 as usize].entered_from != Some(z) {
                return false;
            }
        }
        if f.fought_this_turn && self.cards[v.id.0 as usize].fought_turn != self.turn {
            return false;
        }
        if f.unblocked && (!matches!(self.step, Step::DeclareBlockers | Step::FirstStrikeDamage | Step::CombatDamage | Step::EndCombat) || !self.combat.as_ref().map_or(false, |c| c.attackers.iter().any(|a| a.card == v.id && !a.blocked))) {
            return false;
        }
        if f.is_spell && !(v.live && matches!(self.cards[v.id.0 as usize].zone, Zone::Stack | Zone::Hand)) {
            return false;
        }
        if f.cast_by_non_owner {
            let owner = self.cards[v.id.0 as usize].owner;
            if !(v.live && self.stack.iter().any(|s| s.source == v.id && !s.copy && matches!(s.kind, ItemKind::Spell { .. }) && s.controller != owner)) {
                return false;
            }
        }
        if let Some(sel) = &f.attached_to_sel {
            let host = if v.live { self.cards[v.id.0 as usize].attached_to } else { None };
            let fe = self.filter_event.borrow();
            let fr = self.filter_rem.borrow();
            let ft = self.filter_targets.borrow();
            let ec = EvalCtx { source, controller: src_controller, x: self.cards[source.0 as usize].x, targets: &ft.0, tidx: &ft.1, event: fe.as_ref().unwrap_or(&EMPTY_EVENT), remembered: &fr };
            match host {
                Some(h) if self.sel_cards(sel, &ec).contains(&h) => {}
                _ => return false,
            }
        }
        if let Some(hf) = &f.attached_to_matching {
            let host = if v.live { self.cards[v.id.0 as usize].attached_to } else { None };
            match host {
                Some(h) if self.cards[h.0 as usize].zone == Zone::Battlefield && self.matches_card(hf, h, source, src_controller) => {}
                _ => return false,
            }
        }
        if f.attached_by_source && (if v.live { self.cards[source.0 as usize].attached_to != Some(v.id) } else { !v.lki_attachments.contains(&source) }) {
            return false;
        }
        if f.controlled_by_chosen_player && self.cards[source.0 as usize].chosen_player != Some(v.controller) {
            return false;
        }
        if f.not_attached_by_source && (if v.live { self.cards[source.0 as usize].attached_to == Some(v.id) } else { v.lki_attachments.contains(&source) }) {
            return false;
        }
        if f.attached_to_source && !(v.live && self.cards[v.id.0 as usize].attached_to == Some(source)) {
            return false;
        }
        if f.haunted_by_this && self.cards[source.0 as usize].haunting != Some(v.id) {
            return false;
        }
        if f.not_summoning_sick && !self.can_use_tap_symbol(v.id) {
            return false;
        }
        if f.discarded_this_turn && self.cards[v.id.0 as usize].discarded_turn != Some(self.turn) {
            return false;
        }
        if f.not_put_in_graveyard_this_combat {
            let zcc = self.cards[v.id.0 as usize].zcc;
            if self.gy_entries.iter().skip(self.combat_gy_start).any(|(id, z, _, _)| *id == v.id && *z == zcc) {
                return false;
            }
        }
        if f.milled_this_turn {
            let cd = &self.cards[v.id.0 as usize];
            if !(cd.milled_turn == self.turn && cd.milled_zcc == cd.zcc && cd.zone == Zone::Graveyard) {
                return false;
            }
        }
        if f.put_in_graveyard_from_library_this_turn {
            let zcc = self.cards[v.id.0 as usize].zcc;
            if !self.gy_entries.iter().any(|(id, z, _, from)| *id == v.id && *z == zcc && *from == Zone::Library) {
                return false;
            }
        }
        if f.put_in_graveyard_this_turn || f.put_in_graveyard_from_battlefield_this_turn || f.put_in_graveyard_not_from_battlefield_this_turn {
            let zcc = self.cards[v.id.0 as usize].zcc;
            let hit = self.gy_entries.iter().any(|(id, z, _, from)| *id == v.id && *z == zcc && (!f.put_in_graveyard_from_battlefield_this_turn || *from == Zone::Battlefield) && (!f.put_in_graveyard_not_from_battlefield_this_turn || *from != Zone::Battlefield));
            if !hit {
                return false;
            }
        }
        if f.surveilled_this_turn {
            let cd = &self.cards[v.id.0 as usize];
            if !(cd.surveilled_turn == self.turn && cd.surveilled_zcc == cd.zcc && cd.zone == Zone::Graveyard) {
                return false;
            }
        }
        if f.entered_this_turn && !(self.cards[v.id.0 as usize].zone == Zone::Battlefield && self.cards[v.id.0 as usize].entered_turn == self.turn) {
            return false;
        }
        if f.entered_since_your_last_turn_ended {
            let cd = &self.cards[v.id.0 as usize];
            let me = &self.players[src_controller as usize];
            // during your own turn the previous one ended before this turn began
            let since = if self.active == src_controller { me.prev_turn_start } else { me.last_turn_start };
            if !(cd.zone == Zone::Battlefield && cd.entered_turn > since) {
                return false;
            }
        }
        if f.attacked_or_blocked_since_upkeep {
            let cd = &self.cards[v.id.0 as usize];
            let me = &self.players[src_controller as usize];
            // during the controller's own turn (their upkeep) the previous upkeep began on `prev_turn_start`
            let since = if self.active == src_controller { me.prev_turn_start } else { me.last_turn_start };
            let t = cd.attacked_turn.max(cd.blocked_turn);
            if t == 0 || t < since {
                return false;
            }
        }
        if f.attacked_last_own_turn {
            let rounds = self.players.len() as u32;
            let at = self.cards[v.id.0 as usize].attacked_turn;
            if at == 0 || self.turn < rounds || at != self.turn - rounds {
                return false;
            }
        }
        if f.attacking_battle {
            let hit = self.combat.as_ref().map_or(false, |cb| {
                cb.attackers.iter().any(|a| a.card == v.id && matches!(a.defender, Obj::Card { id } if self.cards[id.0 as usize].cur.types.has(CardType::Battle)))
            });
            if !hit {
                return false;
            }
        }
        if f.attacked_battle_this_turn && self.cards[v.id.0 as usize].attacked_battle_turn != self.turn {
            return false;
        }
        if f.attacked_you_this_turn && !(self.cards[v.id.0 as usize].attacked_turn == self.turn && self.cards[v.id.0 as usize].attacked_player == Some(src_controller)) {
            return false;
        }
        if f.attacked_this_turn && self.cards[v.id.0 as usize].attacked_turn != self.turn {
            return false;
        }
        if f.not_attacked_this_turn && self.cards[v.id.0 as usize].attacked_turn == self.turn {
            return false;
        }
        if let Some(want) = f.couldnt_attack {
            let could = v.live && v.controller == self.active && self.attacker_options(v.controller).iter().any(|o| o.card == v.id);
            if could == want {
                return false;
            }
        }
        if f.was_blocked_this_turn && self.cards[v.id.0 as usize].was_blocked_turn != self.turn {
            return false;
        }
        if let Some(slot) = f.blocked_pair_stored {
            let st = self.stored_view();
            let held = |b: CardId| st.get(slot as usize).map_or(false, |s| s.iter().any(|o| matches!(o, Obj::Card { id } if *id == b)));
            if !self.cards[v.id.0 as usize].block_pairs.iter().any(|p| p.0 == self.turn && held(p.1)) {
                return false;
            }
        }
        if let Some(pf) = &f.blocked_pair_this_turn {
            let others: Vec<CardId> = self.cards[v.id.0 as usize].block_pairs.iter().filter(|p| p.0 == self.turn).map(|p| p.1).collect();
            if !others.iter().any(|o| self.matches_card(pf, *o, source, src_controller)) {
                return false;
            }
        }
        if let Some(pf) = &f.blocking_matching {
            let ok = self.combat.as_ref().map_or(false, |cb| cb.attackers.iter().any(|a| a.blockers.contains(&v.id) && self.matches_card(pf, a.card, source, src_controller)));
            if !ok {
                return false;
            }
        }
        if let Some(pf) = &f.blocked_by_matching {
            let ok = self.combat.as_ref().map_or(false, |cb| cb.attackers.iter().any(|a| a.card == v.id && a.blockers.iter().any(|b| self.matches_card(pf, *b, source, src_controller))));
            if !ok {
                return false;
            }
        }
        if let Some(ab) = &f.has_ability {
            if !v.d.granted.iter().any(|g| **g == **ab) && (v.d.no_abilities || !v.def.abilities.iter().any(|a| a == &**ab)) {
                return false;
            }
        }
        if f.not_blocked_this_turn && self.cards[v.id.0 as usize].blocked_turn == self.turn {
            return false;
        }
        if f.blocked_this_turn && self.cards[v.id.0 as usize].blocked_turn != self.turn {
            return false;
        }
        if f.enlisted_this_turn && self.cards[v.id.0 as usize].enlisted_turn != self.turn {
            return false;
        }
        if f.turned_face_up_this_turn && self.cards[v.id.0 as usize].face_up_turn != self.turn {
            return false;
        }
        if f.dealt_combat_damage_this_turn && !self.combat_damagers.contains(&v.id) {
            return false;
        }
        if f.dealt_damage_this_turn && !self.dmg_log.iter().any(|r| r.source == Some(v.id)) {
            return false;
        }
        if f.was_dealt_noncombat_damage_this_turn && !self.dmg_log.iter().any(|r| !r.combat && r.target == Obj::card(v.id)) {
            return false;
        }
        if f.was_dealt_damage_this_turn && !self.dmg_log.iter().any(|r| r.target == Obj::card(v.id)) {
            return false;
        }
        if f.was_dealt_excess_damage_this_turn && !self.dmg_log.iter().any(|r| r.target == Obj::card(v.id) && r.excess > 0) {
            return false;
        }
        if f.dealt_damage_to_opponent_this_turn {
            let me = self.cards[source.0 as usize].cur.controller;
            if !self.dmg_log.iter().any(|r| r.source == Some(v.id) && matches!(r.target, Obj::Player { id } if id != me)) {
                return false;
            }
        }
        if f.dealt_damage_to_you_this_turn && !self.dmg_log.iter().any(|r| r.source == Some(v.id) && r.target == Obj::player(self.cards[source.0 as usize].controller)) {
            return false;
        }
        if f.dealt_combat_damage_to_player_this_turn && !self.dmg_log.iter().any(|r| r.combat && r.source == Some(v.id) && matches!(r.target, Obj::Player { .. })) {
            return false;
        }
        if f.dealt_combat_damage_to_player_this_combat && !self.dmg_log.iter().any(|r| r.combat && r.combat_no == self.combats_this_turn && r.source == Some(v.id) && matches!(r.target, Obj::Player { .. })) {
            return false;
        }
        if f.dealt_combat_damage_to_you_this_turn && !self.dmg_log.iter().any(|r| r.combat && r.source == Some(v.id) && r.target == Obj::player(self.cards[source.0 as usize].controller)) {
            return false;
        }
        if f.damaged_by_your_source {
            if !self.dmg_log.iter().any(|r| r.target == Obj::card(v.id) && r.source_ctrl == Some(src_controller)) {
                return false;
            }
        }
        if let Some(df) = &f.damaged_by_your_matching {
            let hit = self.dmg_log.iter().any(|r| r.target == Obj::card(v.id) && r.source_ctrl == Some(src_controller) && r.source.map_or(false, |s| self.matches_card_ignore_controller_zone(df, s, source, src_controller)));
            if !hit {
                return false;
            }
        }
        if f.damaged_by_source && !self.cards[v.id.0 as usize].damaged_by.contains(&source) {
            return false;
        }
        if f.dealt_damage_to_source && !self.cards[source.0 as usize].damaged_by.contains(&v.id) {
            return false;
        }
        if f.damaged_by_attached && !self.cards[source.0 as usize].attached_to.map_or(false, |h| self.cards[v.id.0 as usize].damaged_by.contains(&h)) {
            return false;
        }
        if f.damaged && v.damage == 0 {
            return false;
        }
        if f.enchanted_or_equipped {
            let has = v.had_attachment.unwrap_or_else(|| self.battlefield.iter().any(|a| self.cards[a.0 as usize].attached_to == Some(v.id)));
            if !has {
                return false;
            }
        }
        if let Some(af) = &f.has_attached {
            let ok = if v.live {
                self.battlefield.iter().any(|a| self.cards[a.0 as usize].attached_to == Some(v.id) && self.matches_card(af, *a, source, src_controller))
            } else {
                v.lki_attachments.iter().any(|a| self.matches_card(af, *a, source, src_controller))
            };
            if !ok {
                return false;
            }
        }
        if f.not_enchanted {
            let atts: Vec<CardId> = if v.live { self.battlefield.iter().copied().filter(|a| self.cards[a.0 as usize].attached_to == Some(v.id)).collect() } else { v.lki_attachments.to_vec() };
            if atts.iter().any(|a| self.cards[a.0 as usize].cur.types.has(CardType::Enchantment)) {
                return false;
            }
        }
        if f.enchanted || f.equipped {
            let atts: Vec<CardId> = if v.live { self.battlefield.iter().copied().filter(|a| self.cards[a.0 as usize].attached_to == Some(v.id)).collect() } else { v.lki_attachments.to_vec() };
            if f.enchanted && !atts.iter().any(|a| self.cards[a.0 as usize].cur.types.has(CardType::Enchantment)) {
                return false;
            }
            if f.equipped && !atts.iter().any(|a| self.cards[a.0 as usize].cur.subtypes.contains(&Sym::new("Equipment"))) {
                return false;
            }
        }
        if let Some(k) = f.has_counter {
            if !v.counters.iter().any(|(c, n)| *c == k && *n as i32 >= f.counter_min.unwrap_or(1).max(1)) {
                return false;
            }
        }
        if f.has_any_counter && !v.counters.iter().any(|(_, n)| *n > 0) {
            return false;
        }
        if let Some(k) = f.not_has_counter {
            if v.counters.iter().any(|(c, n)| *c == k && *n > 0) {
                return false;
            }
        }
        true
    }

    /// All cards in `zone` (for `All` selectors; battlefield by default) matching the filter.
    pub fn cards_matching(&self, f: &Filter, zone: Option<Zone>, source: CardId, controller: PlayerId) -> Vec<CardId> {
        let zone = zone.unwrap_or(Zone::Battlefield);
        let mut out = vec![];
        let check = |id: CardId, out: &mut Vec<CardId>| {
            if self.matches_card(f, id, source, controller) {
                out.push(id)
            }
        };
        match zone {
            Zone::Battlefield => {
                for &id in &self.battlefield {
                    check(id, &mut out)
                }
            }
            Zone::Graveyard => {
                for p in &self.players {
                    for &id in &p.graveyard {
                        check(id, &mut out)
                    }
                }
            }
            Zone::Hand => {
                for p in &self.players {
                    for &id in &p.hand {
                        check(id, &mut out)
                    }
                }
            }
            Zone::Library => {
                for p in &self.players {
                    for &id in &p.library {
                        check(id, &mut out)
                    }
                }
            }
            Zone::Exile => {
                for p in &self.players {
                    for &id in &p.exile {
                        check(id, &mut out)
                    }
                }
            }
            Zone::Command => {
                for p in &self.players {
                    for &id in &p.command {
                        check(id, &mut out)
                    }
                }
            }
            Zone::Sideboard => {
                for p in &self.players {
                    for &id in &p.sideboard {
                        check(id, &mut out)
                    }
                }
            }
            Zone::Stack => {
                for s in &self.stack {
                    if matches!(s.kind, ItemKind::Spell { .. }) {
                        check(s.source, &mut out)
                    }
                }
            }
        }
        out
    }

    fn player_counter(&self, p: PlayerId, kind: PlayerCounterKind) -> i32 {
        let pl = &self.players[p as usize];
        match kind {
            PlayerCounterKind::Energy => pl.energy,
            PlayerCounterKind::Poison => pl.poison,
            PlayerCounterKind::Experience => pl.experience,
            PlayerCounterKind::Rad => pl.rad,
        }
    }

    pub fn eval_amount(&self, a: &Amount, c: &EvalCtx) -> i32 {
        match a {
            Amount::Fixed { n } => *n,
            Amount::X => c.x,
            Amount::TriggerSpellModes => {
                let spell = match c.event.obj {
                    Some(Obj::Card { id }) => Some(id),
                    _ => c.event.source,
                };
                spell.and_then(|sp| self.stack.iter().find(|s| s.source == sp && !s.copy).map(|s| s.modes.len() as i32).or_else(|| self.stack_lki_items.iter().rev().find(|s| s.source == sp && !s.copy).map(|s| s.modes.len() as i32))).unwrap_or(0)
            }
            Amount::TriggerSpellX => {
                let spell = match c.event.obj {
                    Some(Obj::Card { id }) => Some(id),
                    _ => c.event.source,
                };
                spell.and_then(|sp| self.stack.iter().find(|s| s.source == sp && !s.copy).map(|s| s.x).or_else(|| self.stack_lki_items.iter().rev().find(|s| s.source == sp && !s.copy).map(|s| s.x))).unwrap_or(0)
            }
            // several players (e.g. each opponent): the highest count, so "an opponent has N or more" works
            Amount::PlayerCounters { who, kind } => self
                .sel_players(who, c)
                .iter()
                .map(|p| {
                    let pl = &self.players[*p as usize];
                    match kind {
                        PlayerCounterKind::Energy => pl.energy,
                        PlayerCounterKind::Poison => pl.poison,
                        PlayerCounterKind::Experience => pl.experience,
                        PlayerCounterKind::Rad => pl.rad,
                    }
                })
                .max()
                .unwrap_or(0),
            Amount::PlayerCountersTotal { who, kind } => self.sel_players(who, c).iter().map(|p| self.player_counter(*p, *kind)).sum(),
            Amount::PlayersWithCounters { who, kind, at_least } => self.sel_players(who, c).iter().filter(|p| self.player_counter(**p, *kind) >= *at_least).count() as i32,
            Amount::Votes { option } => self.last_votes.get(*option as usize).copied().unwrap_or(0),
            Amount::PlayerNumber { who } => self.sel_players(who, c).first().and_then(|&p| self.player_numbers.get(p as usize).copied().flatten()).unwrap_or(0),
            Amount::HighestPlayerNumber => self.player_numbers.iter().flatten().copied().max().unwrap_or(0),
            Amount::VotesReceived { who } => self.sel_players(who, c).iter().map(|&p| self.last_votes.get(p as usize).copied().unwrap_or(0)).sum(),
            Amount::CardVotes { of } => self.sel_cards(of, c).iter().map(|id| self.last_card_votes.iter().filter(|(x, _)| x == id).map(|(_, n)| *n).sum::<i32>()).sum(),
            Amount::ManaSpentOf { of, mana } if self.copy_resolving.get() && matches!(of, Sel::This) => 0,
            Amount::ManaSpentOf { of, mana } => self.first_card_or_spell(of, c).map_or(0, |id| self.cards[id.0 as usize].mana_spent[*mana as usize] as i32),
            Amount::DistinctDungeonsCompleted { who } => self
                .sel_players(who, c)
                .iter()
                .map(|p| {
                    let mut names = self.players[*p as usize].completed_dungeon_names.clone();
                    names.sort();
                    names.dedup();
                    names.len() as i32
                })
                .sum(),
            Amount::DungeonsCompleted { who } => self.sel_players(who, c).iter().map(|p| self.players[*p as usize].dungeons_completed as i32).sum(),
            Amount::FlipsWon => self.flip_wins as i32,
            Amount::SacrificedAnyTotalPower { of } => self.sel_cards(of, c).first().map_or(0, |id| self.cards[id.0 as usize].sacrificed_any_power),
            Amount::SacrificedAnyTotalToughness { of } => self.sel_cards(of, c).first().map_or(0, |id| self.cards[id.0 as usize].sacrificed_any_toughness),
            Amount::SacrificedAnyCount { of } => self.sel_cards(of, c).first().map_or(0, |id| self.cards[id.0 as usize].sacrificed_any as i32),
            Amount::RememberedCount => c.remembered.len() as i32,
            Amount::DrawnSinceStart => self.draw_log.borrow().len() as i32,
            Amount::RememberedSharedManaValue => {
                let mvs: Vec<i32> = c.remembered.iter().filter_map(|o| if let Obj::Card { id } = o { Some(if self.cards[id.0 as usize].cur.no_mana_cost { 0 } else { self.def_of(*id).mana_value as i32 }) } else { None }).collect();
                mvs.iter().enumerate().filter(|(i, m)| mvs.iter().enumerate().any(|(j, n)| j != *i && n == *m)).count() as i32
            }
            Amount::LeftBattlefieldControlledBy { of, who, filter } => {
                let ps = self.sel_players(who, c);
                self.sel_cards(of, c).iter().filter(|id| self.left_bf_turn.iter().any(|(lid, p)| lid == *id && ps.contains(p)) && filter.as_ref().map_or(true, |f| self.matches_card(&Filter { controller: PlayerFilter::Any, ..(**f).clone() }, **id, c.source, c.controller))).count() as i32
            }
            Amount::LkiCount { filter } => match c.event.lki.as_deref().or(self.last_lki.as_ref()) {
                Some(l) => {
                    let ctl = l.controller;
                    let f = Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() };
                    let mut ids: Vec<CardId> = vec![];
                    // controller test relative to the leaving permanent's controller, judged as it was when everything left
                    l.snapshot
                        .iter()
                        .filter(|(id, p)| {
                            let keep = match filter.controller {
                                PlayerFilter::You => *p == ctl,
                                PlayerFilter::Opponent => self.is_opponent(*p, ctl),
                                _ => true,
                            };
                            keep && !ids.contains(id) && { ids.push(*id); true } && self.matches_card(&f, *id, c.source, ctl)
                        })
                        .count() as i32
                }
                None => 0,
            },
            Amount::PartySize { who } => self
                .sel_players(who, c)
                .iter()
                .map(|p| {
                    // each creature fills at most one of the four roles: the biggest party (a maximum matching, found by search)
                    let roles = ["Cleric", "Rogue", "Warrior", "Wizard"];
                    let masks: Vec<u8> = self
                        .battlefield
                        .iter()
                        .filter_map(|id| {
                            let k = &self.cards[id.0 as usize];
                            if k.cur.controller != *p || !k.cur.types.has(CardType::Creature) {
                                return None;
                            }
                            let m = roles.iter().enumerate().fold(0u8, |a, (i, r)| if k.cur.subtypes.contains(&Sym::new(r)) || k.cur.kw.has(Keyword::Changeling) { a | (1 << i) } else { a });
                            (m != 0).then_some(m)
                        })
                        .collect();
                    fn best(masks: &[u8], used: u32, role: usize) -> i32 {
                        if role == 4 {
                            return 0;
                        }
                        let mut b = best(masks, used, role + 1);
                        for (i, m) in masks.iter().enumerate() {
                            if used & (1 << i) == 0 && m & (1 << role) != 0 {
                                b = b.max(1 + best(masks, used | (1 << i), role + 1));
                            }
                        }
                        b
                    }
                    best(&masks[..masks.len().min(20)], 0, 0)
                })
                .sum(),
            Amount::Speed { who } => self.sel_players(who, c).iter().map(|p| self.players[*p as usize].speed as i32).sum(),
            Amount::LastPaymentManaSpent { mana } => mana.iter().map(|m| self.unless_spent[*m as usize] as i32).sum(),
            Amount::ManaSpentFromSource { of, source } if self.copy_resolving.get() && matches!(of, Sel::This) => 0,
            Amount::ManaSpentFromSource { of, source } => self.first_card_or_spell(of, c).map_or(0, |id| self.cards[id.0 as usize].src_spent[*source as usize] as i32),
            Amount::StationValue { of } => self.sel_cards(of, c).first().map_or(0, |id| {
                let cur = &self.cards[id.0 as usize].cur;
                (if cur.station_by_toughness && cur.toughness > cur.power { cur.toughness } else { cur.power }) * if self.crew_station_triples(*id) { 3 } else { 1 }
            }),
            Amount::SnowColorsMatching { of } => self.first_card_or_spell(of, c).map_or(0, |id| (self.cards[id.0 as usize].snow_colors & self.cards[id.0 as usize].cur.colors.0).count_ones() as i32),
            Amount::SnowManaSpent { of } if self.copy_resolving.get() && matches!(of, Sel::This) => 0,
            Amount::SnowManaSpent { of } => self.first_card_or_spell(of, c).map_or(0, |id| self.cards[id.0 as usize].snow_spent as i32),
            Amount::ManaSpent { of } if self.copy_resolving.get() && matches!(of, Sel::This) => 0,
            Amount::ManaSpent { of } => self.first_card_or_spell(of, c).map_or(0, |id| self.cards[id.0 as usize].mana_spent.iter().map(|&n| n as i32).sum()),
            Amount::ConvokedCount { of } => self.first_card_or_spell(of, c).map_or(0, |id| self.cards[id.0 as usize].convoked.len() as i32),
            Amount::ColorsOf { of } => self.first_card_or_spell(of, c).map_or(0, |id| match c.event.lki.as_deref().or(self.last_lki.as_ref()) {
                Some(l) if l.card == id && self.cards[id.0 as usize].zone != Zone::Battlefield && self.cards[id.0 as usize].zone != Zone::Stack => l.derived.colors.count() as i32,
                _ if matches!(self.cards[id.0 as usize].zone, Zone::Battlefield | Zone::Stack) => self.cards[id.0 as usize].cur.colors.count() as i32,
                _ => self.def_of(id).colors.count() as i32,
            }),
            Amount::ActivationColorsSpent => self.cards[c.source.0 as usize].act_spent[..5].iter().filter(|&&n| n > 0).count() as i32,
            Amount::ColorsSpent { of } if self.copy_resolving.get() && matches!(of, Sel::This) => 0,
            Amount::ColorsSpent { of } => self.first_card_or_spell(of, c).map_or(0, |id| self.cards[id.0 as usize].mana_spent[..5].iter().filter(|&&n| n > 0).count() as i32),
            Amount::KickCount => {
                // inside a "whenever you cast a spell" trigger: the spell that was cast
                let src = c.event.source.filter(|s| *s != c.source && self.cards[s.0 as usize].zone == Zone::Stack && self.cards[s.0 as usize].cast_from.is_some()).unwrap_or(c.source);
                self.cards[src.0 as usize].kicks as i32
            }
            Amount::KickCountAt { index } => {
                let src = c.event.source.filter(|s| *s != c.source && self.cards[s.0 as usize].zone == Zone::Stack && self.cards[s.0 as usize].cast_from.is_some()).unwrap_or(c.source);
                self.cards[src.0 as usize].kick_by[(*index as usize).min(3)] as i32
            }
            Amount::CountMaxBy { filter, zone, who } => {
                let z = zone.unwrap_or(Zone::Battlefield);
                let mut f2 = filter.clone();
                f2.controller = PlayerFilter::Any;
                let ps = self.sel_players(who, c);
                if z == Zone::Battlefield {
                    let ids = self.cards_matching(&f2, None, c.source, c.controller);
                    return ps.iter().map(|p| ids.iter().filter(|id| self.cards[id.0 as usize].cur.controller == *p).count() as i32).max().unwrap_or(0);
                }
                ps.iter()
                    .map(|p| {
                        let pl = &self.players[*p as usize];
                        let list = match z {
                            Zone::Graveyard => &pl.graveyard,
                            Zone::Hand => &pl.hand,
                            Zone::Library => &pl.library,
                            Zone::Exile => &pl.exile,
                            Zone::Command => &pl.command,
                            Zone::Sideboard => &pl.sideboard,
                            _ => return 0,
                        };
                        list.iter().filter(|id| self.matches_card(&f2, **id, c.source, c.controller)).count() as i32
                    })
                    .max()
                    .unwrap_or(0)
            }
            Amount::Count { filter, zone, player } => {
                // `zone: None` counts permanents; for graveyards etc. the filter's controller restricts whose
                let z = zone.unwrap_or(Zone::Battlefield);
                if let Some(ps) = player {
                    let who = self.sel_players(ps, c);
                    let mut f2 = filter.clone();
                    f2.controller = PlayerFilter::Any;
                    return if z == Zone::Battlefield {
                        self.cards_matching(&f2, None, c.source, c.controller).into_iter().filter(|id| who.contains(&self.cards[id.0 as usize].cur.controller)).count() as i32
                    } else {
                        let mut n = 0;
                        for p in self.players.iter().filter(|p| who.contains(&p.id)) {
                            let list = match z {
                                Zone::Graveyard => &p.graveyard,
                                Zone::Hand => &p.hand,
                                Zone::Library => &p.library,
                                Zone::Exile => &p.exile,
                                Zone::Command => &p.command,
                                Zone::Sideboard => &p.sideboard,
                                _ => continue,
                            };
                            n += list.iter().filter(|id| self.matches_card(&f2, **id, c.source, c.controller)).count() as i32;
                        }
                        n
                    };
                }
                if z == Zone::Battlefield {
                    self.cards_matching(filter, None, c.source, c.controller).len() as i32
                } else {
                    let mut n = 0;
                    for p in &self.players {
                        if !self.player_filter(filter.controller, p.id, c.controller) {
                            continue;
                        }
                        let list = match z {
                            Zone::Graveyard => &p.graveyard,
                            Zone::Hand => &p.hand,
                            Zone::Library => &p.library,
                            Zone::Exile => &p.exile,
                            Zone::Command => &p.command,
                            Zone::Sideboard => &p.sideboard,
                            _ => continue,
                        };
                        let mut f2 = filter.clone();
                        f2.controller = PlayerFilter::Any;
                        n += list.iter().filter(|id| self.matches_card(&f2, **id, c.source, c.controller)).count() as i32;
                    }
                    n
                }
            }
            Amount::PowerOf { of } => self.sel_cards_or_spell(of, c).first().map_or_else(|| self.countered_spell_def(of, c).and_then(|d| self.db.def(d).power).unwrap_or(0), |id| match c.event.lki.as_deref().filter(|e| e.card == *id).or(self.last_lki.as_ref()).or(c.event.lki.as_deref()) {
                Some(l) if l.card == *id && (self.cards[id.0 as usize].zone != Zone::Battlefield || c.event.lki.as_deref().map_or(false, |e| e.card == *id)) => l.derived.power,
                // a card in hand / graveyard / exile has its printed power
                _ if self.cards[id.0 as usize].zone != Zone::Battlefield => self.cda_pt_off_battlefield(*id).map_or_else(|| self.def_of(*id).power.unwrap_or_else(|| self.power_of(*id)), |p| p.0),
                _ => self.power_of(*id),
            }),
            Amount::BasePowerOf { of } => self.sel_cards(of, c).first().map_or(0, |id| self.def_of(*id).power.unwrap_or(0)),
            Amount::ToxicValue { of } => self.sel_cards(of, c).first().map_or(0, |id| {
                let kw = self.cards[id.0 as usize].cur.kw;
                [(Keyword::Toxic1, 1), (Keyword::Toxic2, 2), (Keyword::Toxic3, 3), (Keyword::Toxic4, 4), (Keyword::Toxic5, 5), (Keyword::Toxic9, 9)].iter().filter(|(k, _)| kw.has(*k)).map(|(_, n)| *n).sum()
            }),
            Amount::PowerMinusBasePower { of } => self.sel_cards(of, c).first().map_or(0, |id| self.power_of(*id) - self.cards[id.0 as usize].cur.base_pt.map(|b| b.0).unwrap_or_else(|| self.def_of(*id).power.unwrap_or(0))),
            Amount::ToughnessOf { of } => self.sel_cards_or_spell(of, c).first().map_or(0, |id| match c.event.lki.as_deref().or(self.last_lki.as_ref()) {
                Some(l) if l.card == *id && (self.cards[id.0 as usize].zone != Zone::Battlefield || c.event.lki.as_deref().map_or(false, |e| e.card == *id)) => l.derived.toughness,
                _ if self.cards[id.0 as usize].zone != Zone::Battlefield => self.cda_pt_off_battlefield(*id).map_or_else(|| self.def_of(*id).toughness.unwrap_or_else(|| self.cards[id.0 as usize].cur.toughness), |p| p.1),
                _ => self.cards[id.0 as usize].cur.toughness,
            }),
            Amount::CreatureTypeCount { of } => self.sel_cards(of, c).first().map_or(0, |id| self.cards[id.0 as usize].cur.subtypes.iter().filter(|s| crate::dsl::SubtypeKind::Creature.contains(&s.name())).count() as i32),
            Amount::SourceMark { mark } => self.mark_of(c.source, *mark),
            Amount::ManaValueOf { of } => {
                // a targeted spell on the stack has its card's mana value
                if let Sel::Target { idx } = of {
                    if let Some(Target { obj: Obj::Stack { id }, .. }) = c.target(*idx) {
                        let def = self.stack.iter().find(|s| s.id == *id).map(|s| s.def).or_else(|| self.stack_lki.iter().find(|(i, _)| i == id).map(|(_, d)| *d));
                        // a spell on the stack counts its announced X
                        let x = self.stack.iter().find(|s| s.id == *id).map(|s| s.x as i32).or_else(|| self.stack_lki_x.iter().find(|(i, _)| i == id).map(|(_, x)| *x)).unwrap_or(0);
                        return def.map_or(0, |d| { let d = self.db.def(d); d.mana_value as i32 + x * d.cost.shards.iter().filter(|s| matches!(s, Shard::X)).count() as i32 });
                    }
                }
                self.sel_cards(of, c).first().map_or(0, |id| {
                    if self.cards[id.0 as usize].cur.no_mana_cost {
                        return 0;
                    }
                    // a permanent that left the battlefield (sacrificed, destroyed) has the mana value it had there: a Clone copy of a Grizzly Bears is 2, not the Clone card's 4
                    let off_bf = !matches!(self.cards[id.0 as usize].zone, Zone::Battlefield | Zone::Stack);
                    let lki = if off_bf { c.event.lki.as_deref().filter(|e| e.card == *id).or(self.last_lki.as_ref().filter(|l| l.card == *id)) } else { None };
                    if let Some(l) = lki {
                        if l.face_down {
                            return 0;
                        }
                    }
                    let d = match lki {
                        Some(l) if !l.token && l.def != self.cards[id.0 as usize].def && self.def_of(*id).other_face != Some(l.def) && self.db.def(l.def).other_face != Some(self.cards[id.0 as usize].def) => self.db.def(l.def),
                        _ => self.def_of(*id),
                    };
                    // a spell on the stack counts its announced X
                    let x = if self.cards[id.0 as usize].zone == Zone::Stack {
                        self.stack.iter().find(|s| s.source == *id).map_or(0, |s| s.x as i32)
                    } else if matches!(of, Sel::TriggerObject | Sel::Remembered) {
                        // a spell that was just countered or has resolved: its mana value as it last existed on the stack (Plasm Capture, Metallurgic Summonings)
                        self.stack_lki_src.iter().rev().find(|(_, cid)| cid == id).and_then(|(sid, _)| self.stack_lki_x.iter().find(|(i, _)| i == sid).map(|(_, x)| *x)).unwrap_or(0)
                    } else {
                        0
                    };
                    self.mv_of(*id, d) as i32 + x * d.cost.shards.iter().filter(|s| matches!(s, Shard::X)).count() as i32
                })
            }
            Amount::LifeOf { who } => self.sel_players(who, c).first().map_or(0, |p| self.players[*p as usize].life),
            Amount::StartingLife => self.cfg.starting_life,
            Amount::StartingDeckSize { who } => self.sel_players(who, c).first().map_or(0, |p| self.players[*p as usize].starting_deck),
            Amount::UntappedLandsAtTurnStart { who } => self.sel_players(who, c).first().map_or(0, |p| self.players[*p as usize].lands_untapped_turn_start),
            Amount::HandSizeAtTurnStart { who } => self.sel_players(who, c).first().map_or(0, |p| self.players[*p as usize].hand_turn_start),
            Amount::HandSize { who } => self.sel_players(who, c).first().map_or(0, |p| self.players[*p as usize].hand.len() as i32),
            Amount::CreaturesEnteredThisTurn { who } => self.sel_players(who, c).iter().map(|p| self.players[*p as usize].creatures_entered as i32).sum(),
            Amount::CreaturesEnteredLastTurn { who, other } => self
                .sel_players(who, c)
                .iter()
                .map(|p| {
                    let me = &self.cards[c.source.0 as usize];
                    let itself = *other && me.entered_as_creature && me.cur.controller == *p && me.entered_turn + 1 == self.turn;
                    self.players[*p as usize].creatures_entered_last as i32 - itself as i32
                })
                .sum(),
            Amount::HandSizeMin { who } => self.sel_players(who, c).iter().map(|p| self.players[*p as usize].hand.len() as i32).min().unwrap_or(0),
            Amount::HandSizeTotal { who } => self.sel_players(who, c).iter().map(|p| self.players[*p as usize].hand.len() as i32).sum(),
            Amount::LifeOfMax { who } => self.sel_players(who, c).iter().map(|p| self.players[*p as usize].life).max().unwrap_or(0),
            Amount::LifeOfMin { who } => self.sel_players(who, c).iter().map(|p| self.players[*p as usize].life).min().unwrap_or(0),
            Amount::HandSizeMax { who } => self.sel_players(who, c).iter().map(|p| self.players[*p as usize].hand.len() as i32).max().unwrap_or(0),
            Amount::PayTally { who } => self.sel_players(who, c).iter().map(|p| self.pay_tally.get(*p as usize).copied().unwrap_or(0)).sum(),
            Amount::CoinFlipsInSeries => self.flip_count,
            Amount::BidOf { who } => self.sel_players(who, c).iter().map(|p| self.life_rounds.get(*p as usize).copied().unwrap_or(0)).sum(),
            Amount::LifePaidRounds { who } => self.sel_players(who, c).iter().map(|p| self.life_rounds.get(*p as usize).copied().unwrap_or(0)).sum(),
            Amount::AllPlayerCounters { who } => self.sel_players(who, c).iter().map(|p| [PlayerCounterKind::Energy, PlayerCounterKind::Poison, PlayerCounterKind::Experience, PlayerCounterKind::Rad].into_iter().map(|k| self.player_counter(*p, k)).sum::<i32>()).sum(),
            Amount::GraveyardSizeMax { who } => self.sel_players(who, c).iter().map(|p| self.players[*p as usize].graveyard.len() as i32).max().unwrap_or(0),
            Amount::GraveyardSize { who } => {
                self.sel_players(who, c).first().map_or(0, |p| self.players[*p as usize].graveyard.len() as i32)
            }
            Amount::BushidoValue { of } => self
                .sel_cards(of, c)
                .iter()
                .map(|id| {
                    self.db
                        .def(self.cards[id.0 as usize].def)
                        .abilities
                        .iter()
                        .map(|a| match a {
                            Ability::Triggered { trigger: Trigger::Blocks { filter, .. }, effects, .. } if filter.this => effects
                                .iter()
                                .map(|e| match e {
                                    Effect::Pump { what: Sel::This, power: Amount::Fixed { n }, toughness: Amount::Fixed { n: t }, .. } if n == t => *n,
                                    _ => 0,
                                })
                                .sum::<i32>(),
                            _ => 0,
                        })
                        .sum::<i32>()
                })
                .sum(),
            Amount::AllCountersOn { of } => self
                .sel_cards(of, c)
                .iter()
                .map(|id| {
                    let card = &self.cards[id.0 as usize];
                    if card.zone != Zone::Battlefield {
                        // a permanent that left the battlefield: its last known counters
                        if let Some(l) = c.event.lki.as_deref().or(self.last_lki.as_ref()).filter(|l| l.card == *id) {
                            return l.counters.iter().map(|(_, n)| *n as i32).sum::<i32>();
                        }
                        if card.counters.iter().all(|(_, n)| *n == 0) {
                            return card.left_counters.iter().map(|(_, n)| *n as i32).sum::<i32>();
                        }
                    }
                    card.counters.iter().map(|(_, n)| *n as i32).sum::<i32>()
                })
                .sum(),
            Amount::BeheldPower => self.cards[c.source.0 as usize].beheld_power.unwrap_or(0),
            Amount::PowerOfTwo { of } => 1i32 << self.eval_amount(of, c).clamp(0, 30),
            Amount::CountSel { of } => self.sel_cards(of, c).len() as i32,
            Amount::CountersOn { of, kind } => self.sel_cards(of, c).first().map_or(0, |id| match c.event.lki.as_deref().or(self.last_lki.as_ref()) {
                Some(l) if l.card == *id && !matches!(self.cards[id.0 as usize].zone, Zone::Battlefield | Zone::Stack) => l.counters.iter().filter(|(k, _)| k == kind).map(|(_, n)| *n as i32).sum(),
                _ if self.cards[id.0 as usize].zone != Zone::Battlefield && self.cards[id.0 as usize].counter(*kind) == 0 => {
                    self.cards[id.0 as usize].left_counters.iter().filter(|(k, _)| k == kind).map(|(_, n)| *n as i32).sum()
                }
                _ => self.cards[id.0 as usize].counter(*kind),
            }),
            Amount::DamageDealtByThisThisTurn => self.dmg_by_source.borrow().get(&c.source).copied().unwrap_or(0),
            Amount::GreatestSorceryDamage { who } => {
                let Some(&p) = self.sel_players(who, c).first() else { return 0 };
                let by = self.dmg_by_source.borrow();
                self.spells_log.iter().filter(|(pl, card)| *pl == p && self.cards[card.0 as usize].cur.types.has(CardType::Sorcery)).map(|(_, card)| by.get(card).copied().unwrap_or(0)).max().unwrap_or(0)
            }
            Amount::CountersRemoved => self.counters_removed.get(),
            Amount::EventAmount => c.event.amount,
            Amount::DieRoll => self.last_die,
            Amount::RollChosen => self.roll_chosen,
            Amount::RollOther => self.roll_other,
            Amount::Plus { a, b } => self.eval_amount(a, c) + self.eval_amount(b, c),
            Amount::Minus { a, b } => self.eval_amount(a, c) - self.eval_amount(b, c),
            Amount::Times { a, b } => self.eval_amount(a, c) * self.eval_amount(b, c),
            Amount::Modulo { a, n } => {
                let v = self.eval_amount(a, c);
                if *n == 0 { 0 } else { v.rem_euclid(*n) }
            }
            Amount::DivideBy { a, n } => {
                let v = self.eval_amount(a, c);
                if *n == 0 { 0 } else { v.div_euclid(*n) }
            }
            Amount::NotedExileMv => {
                let sc = &self.cards[c.source.0 as usize];
                let from = if sc.entered_turn == self.turn { sc.noted_from } else { 0 };
                self.exile_mv_log.iter().skip(from).copied().max().unwrap_or(0)
            }
            Amount::LowestManaValueOf { of } => self.sel_cards(of, c).iter().map(|id| self.def_of(*id).mana_value as i32).min().unwrap_or(0),
            Amount::HighestManaValueOf { of } => self.sel_cards(of, c).iter().map(|id| self.def_of(*id).mana_value as i32).max().unwrap_or(0),
            Amount::EventBatchPower => c.event.batch_power,
            Amount::EventAttackersPower => c.event.card_powers.iter().map(|(id, p)| if self.cards[id.0 as usize].zone == Zone::Battlefield { self.power_of(*id) } else { *p }).sum(),
            Amount::SacrificedCostManaValue => {
                let mv = |id: CardId| if self.cards[id.0 as usize].cur.no_mana_cost { 0 } else { self.def_of(id).mana_value as i32 };
                if let Some(Obj::Card { id }) = c.event.obj {
                    mv(id)
                } else {
                    // not paying right now (planning which mana a source could make): the most valuable permanent its sacrifice cost could take
                    let d = self.def_of(c.source);
                    let mut best = 0;
                    for a in d.abilities.iter() {
                        if let Ability::Activated { cost, .. } = a {
                            for part in cost {
                                if let Cost::Sacrifice { filter, .. } = part {
                                    let mut f = self.sac_cost_filter(filter);
                                    f.controller = PlayerFilter::Any;
                                    for &id in self.battlefield.iter() {
                                        if self.cards[id.0 as usize].cur.controller == c.controller && self.matches_card(&f, id, c.source, c.controller) {
                                            best = best.max(mv(id));
                                        }
                                    }
                                }
                            }
                        }
                    }
                    best
                }
            }
            Amount::RememberedTotalPower => c.remembered.iter().filter_map(|o| if let Obj::Card { id } = o { Some(self.left_bf_power.iter().rev().find(|(i, _)| i == id).filter(|_| self.cards[id.0 as usize].zone != Zone::Battlefield).map_or_else(|| self.power_of(*id), |(_, p)| *p).max(0)) } else { None }).sum(),
            Amount::RememberedTotalManaValue => c.remembered.iter().filter_map(|o| if let Obj::Card { id } = o { Some(if self.cards[id.0 as usize].cur.no_mana_cost { 0 } else { self.def_of(*id).mana_value as i32 }) } else { None }).sum(),
            Amount::DistinctTokenNames { filter } => {
                let mut names: Vec<String> = vec![];
                for id in self.battlefield.iter().copied().filter(|id| self.cards[id.0 as usize].token && self.matches_card(filter, *id, c.source, c.controller)) {
                    let cur = &self.cards[id.0 as usize].cur;
                    let n = if cur.types.has(CardType::Creature) && !cur.subtypes.is_empty() { cur.subtypes.iter().map(|s| s.name()).collect::<Vec<_>>().join(" ") } else { self.db.def(self.cards[id.0 as usize].def).name.clone() };
                    if !names.contains(&n) {
                        names.push(n);
                    }
                }
                names.len() as i32
            }
            Amount::EnterExiledPower => self.cards[c.source.0 as usize].enter_exiled_pt.0,
            Amount::EnterExiledToughness => self.cards[c.source.0 as usize].enter_exiled_pt.1,
            Amount::RememberedTotalToughness => c.remembered.iter().filter_map(|o| if let Obj::Card { id } = o { Some(self.cards[id.0 as usize].cur.toughness.max(0)) } else { None }).sum(),
            Amount::Half { a, round_up } => {
                let v = self.eval_amount(a, c);
                if *round_up {
                    (v + 1).div_euclid(2)
                } else {
                    v.div_euclid(2)
                }
            }
            Amount::Max { a, b } => self.eval_amount(a, c).max(self.eval_amount(b, c)),
            Amount::Min { a, b } => self.eval_amount(a, c).min(self.eval_amount(b, c)),
            Amount::GreatestPowerAtCast => self.cards[c.source.0 as usize].cast_power,
            Amount::HighestPower { filter, zone } => {
                // a card outside the battlefield with a characteristic-defining power reads it from its definition (Burrowguard Mentor in a graveyard)
                self.cards_matching(filter, *zone, c.source, c.controller).iter().map(|id| {
                    if self.cards[id.0 as usize].zone != Zone::Battlefield {
                        self.cda_pt_off_battlefield(*id).map_or_else(|| self.power_of(*id), |p| p.0)
                    } else {
                        self.power_of(*id)
                    }
                }).max().unwrap_or(0)
            }
            Amount::LowestPower { filter, zone } => {
                self.cards_matching(filter, *zone, c.source, c.controller).iter().map(|id| self.power_of(*id)).min().unwrap_or(0)
            }
            Amount::LowestToughness { filter } => self.cards_matching(filter, None, c.source, c.controller).iter().map(|id| self.cards[id.0 as usize].cur.toughness).min().unwrap_or(0),
            Amount::LowestManaValue { filter } => self.cards_matching(filter, None, c.source, c.controller).iter().map(|id| self.def_of(*id).mana_value as i32).min().unwrap_or(0),
            Amount::DistinctColors { filter, spells_cast } => {
                let mut set = 0u32;
                if *spells_cast {
                    for (pl, id) in &self.spells_log {
                        if *pl == c.controller {
                            for col in Color::ALL {
                                if self.cards[id.0 as usize].cur.colors.has(col) {
                                    set |= 1 << (col as u32);
                                }
                            }
                        }
                    }
                }
                for id in self.cards_matching(filter, if filter.exiled_with_this { Some(Zone::Exile) } else { None }, c.source, c.controller) {
                    for col in Color::ALL {
                        if self.cards[id.0 as usize].cur.colors.has(col) {
                            set |= 1 << (col as u32);
                        }
                    }
                }
                set.count_ones() as i32
            }
            Amount::HighestToughness { filter, zone } => {
                self.cards_matching(filter, zone.or(if filter.exiled_with_this { Some(Zone::Exile) } else { None }), c.source, c.controller).iter().map(|id| self.cards[id.0 as usize].cur.toughness).max().unwrap_or(0)
            }
            Amount::HighestManaValue { filter, .. } if filter.among_remembered => {
                // "among cards milled/exiled this way": the remembered cards themselves, wherever they are now
                let f = Filter { among_remembered: false, ..filter.clone().into_filter() };
                c.remembered
                    .iter()
                    .filter_map(|o| if let Obj::Card { id } = o { Some(*id) } else { None })
                    .filter(|id| self.matches_card(&f, *id, c.source, c.controller))
                    .map(|id| self.def_of(id).mana_value as i32)
                    .max()
                    .unwrap_or(0)
            }
            Amount::HighestManaValue { filter, zone } => {
                let mut best = self.cards_matching(filter, zone.or(if filter.exiled_with_this { Some(Zone::Exile) } else { None }), c.source, c.controller).iter().map(|id| self.def_of(*id).mana_value as i32).max().unwrap_or(0);
                // "your commander on the battlefield or in the command zone"
                if filter.commander {
                    best = best.max(self.cards_matching(filter, Some(Zone::Command), c.source, c.controller).iter().map(|id| self.def_of(*id).mana_value as i32).max().unwrap_or(0));
                }
                best
            }
            Amount::TotalToughness { filter } => self.cards_matching(filter, if filter.exiled_with_this { Some(Zone::Exile) } else { None }, c.source, c.controller).iter().map(|id| self.cards[id.0 as usize].cur.toughness.max(0)).sum(),
            Amount::TotalPower { filter } => self.cards_matching(filter, if filter.exiled_with_this { Some(Zone::Exile) } else { None }, c.source, c.controller).iter().map(|id| self.power_of(*id)).sum(),
            Amount::LandsPlayedThisTurn { who } => self.sel_players(who, c).iter().map(|p| self.players[*p as usize].lands_played as i32).sum(),
            Amount::CardsDrawnThisTurn { who } => self.sel_players(who, c).iter().map(|p| self.players[*p as usize].cards_drawn_turn as i32).sum(),
            Amount::LandsEnteredThisTurn { who } => self.sel_players(who, c).iter().map(|p| self.players[*p as usize].lands_entered as i32).sum(),
            Amount::CountersPutThisTurn { kind } => self.counters_log.iter().filter(|(_, p, k, by, cr, _)| *cr && k == kind && *p == c.controller && *by == Some(c.controller)).map(|t| t.5).sum(),
            Amount::SameNameInGraveyardAsSpell => match self.costing_card.get() {
                Some(cc) => {
                    let name = self.card_name(cc);
                    self.players[c.controller as usize].graveyard.iter().filter(|&&g| g != cc && self.card_name(g) == name).count() as i32
                }
                None => 0,
            },
            Amount::RevealedCount => self.last_reveal_count.get(),
            Amount::EnteredThisTurn { who, types, not_types, subtypes, other, nontoken, colors, keywords } => {
                let ps = self.sel_players(who, c);
                let subs: Vec<Sym> = subtypes.iter().map(|s| Sym::new(s)).collect();
                self.entered_log
                    .iter()
                    .filter(|(p, ty, st, ch, eid, tk)| ps.contains(p) && !(*other && *eid == c.source) && !(*nontoken && *tk) && (types.is_empty() || types.iter().any(|t| ty.has(*t))) && !not_types.iter().any(|t| ty.has(*t))
                        && (colors.is_empty() || colors.iter().any(|col| self.def_of(*eid).colors.has(*col)))
                        && (keywords.is_empty() || keywords.iter().any(|k| self.def_of(*eid).keywords.has(*k)))
                        && (subs.is_empty() || *ch || subs.iter().any(|s| st.contains(s))))
                    .count() as i32
            }
            Amount::AttackedThisTurn { who, subtypes, token, commander } => {
                let ps = self.sel_players(who, c);
                if *commander {
                    return self.commander_attackers_log.iter().filter(|p| ps.contains(p)).count() as i32;
                }
                if *token {
                    return self.token_attackers_log.iter().filter(|p| ps.contains(p)).count() as i32;
                }
                let subs: Vec<Sym> = subtypes.iter().map(|s| Sym::new(s)).collect();
                self.attackers_log.iter().filter(|(p, st, ch)| ps.contains(p) && (subs.is_empty() || *ch || subs.iter().any(|s| st.contains(s)))).count() as i32
            }
            Amount::HighestManaValueSpellsCast { filter } => {
                let by_you_by_default = filter.controller == PlayerFilter::default();
                let who = filter.controller;
                let f = Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() };
                self.spells_log
                    .iter()
                    .filter(|(pl, card)| {
                        let ok_player = if by_you_by_default { *pl == c.controller } else { self.player_filter(who, *pl, c.controller) };
                        ok_player && self.matches_card(&f, *card, c.source, c.controller)
                    })
                    .map(|(_, card)| self.def_of(*card).mana_value as i32)
                    .max()
                    .unwrap_or(0)
            }
            Amount::TotalManaValueSpellsCast { filter } => {
                let by_you_by_default = filter.controller == PlayerFilter::default();
                let who = filter.controller;
                let f = Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() };
                self.spells_log
                    .iter()
                    .filter(|(pl, card)| {
                        let ok_player = if by_you_by_default { *pl == c.controller } else { self.player_filter(who, *pl, c.controller) };
                        ok_player && self.matches_card(&f, *card, c.source, c.controller)
                    })
                    .map(|(_, card)| self.def_of(*card).mana_value as i32)
                    .sum()
            }
            Amount::SpellsCastByWhere { who, filter } => {
                let players = self.sel_players(who, c);
                let f = Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() };
                self.spells_log.iter().filter(|(pl, card)| players.contains(pl) && self.matches_card(&f, *card, c.source, c.controller)).count() as i32
            }
            Amount::DamageToPermanentByPlayer { of, by } => {
                let (Some(id), Some(p)) = (self.sel_cards(of, c).first().copied(), self.sel_players(by, c).first().copied()) else { return 0 };
                self.dmg_log.iter().filter(|r| r.target == Obj::card(id) && r.source_ctrl == Some(p)).map(|r| r.amount).sum()
            }
            Amount::DamageToPlayersThisTurn { who, combat, from } => {
                let players = self.sel_players(who, c);
                let ff = from.as_ref().map(|f| Filter { controller: PlayerFilter::Any, ..f.clone().into_filter() });
                self.dmg_log
                    .iter()
                    .filter(|r| matches!(r.target, Obj::Player { id } if players.contains(&id)) && combat.map_or(true, |cb| r.combat == cb))
                    .filter(|r| ff.as_ref().map_or(true, |f| r.source.map_or(false, |s| self.matches_card(f, s, c.source, c.controller))))
                    .map(|r| r.amount)
                    .sum()
            }
            Amount::XOf { of } => match self.first_card_or_spell(of, c) {
                Some(id) => self.stack.iter().find(|it| it.source == id && matches!(it.kind, ItemKind::Spell { .. })).map_or(self.cards[id.0 as usize].x, |it| it.x),
                None => 0,
            },
            Amount::PlayersWhere { who, filter } => self.sel_players(who, c).iter().filter(|p| self.player_filter_src(c.source, *filter, **p, c.controller)).count() as i32,
            Amount::DistinctDamageSources { from, to, combat } => {
                let tos = to.as_ref().map(|s| self.sel_players(s, c));
                let f = Filter { controller: PlayerFilter::Any, ..from.clone().into_filter() };
                let mut seen: Vec<CardId> = vec![];
                for r in &self.dmg_log {
                    let Some(src) = r.source else { continue };
                    if seen.contains(&src) || combat.map_or(false, |cb| r.combat != cb) {
                        continue;
                    }
                    if let Some(ps) = &tos {
                        if !matches!(r.target, Obj::Player { id } if ps.contains(&id)) {
                            continue;
                        }
                    }
                    let ctrl_ok = r.source_ctrl.map_or(true, |sc| self.player_filter(from.controller, sc, c.controller));
                    if ctrl_ok && self.matches_card(&f, src, c.source, c.controller) {
                        seen.push(src);
                    }
                }
                seen.len() as i32
            }
            Amount::MostDamageBySingleSource { from } => {
                let f = Filter { controller: PlayerFilter::Any, ..from.clone().into_filter() };
                let mut totals: Vec<(CardId, i32)> = vec![];
                for r in &self.dmg_log {
                    let Some(src) = r.source else { continue };
                    let ctrl_ok = r.source_ctrl.map_or(true, |sc| self.player_filter(from.controller, sc, c.controller));
                    if !(ctrl_ok && self.matches_card(&f, src, c.source, c.controller)) {
                        continue;
                    }
                    match totals.iter_mut().find(|(id, _)| *id == src) {
                        Some(t) => t.1 += r.amount,
                        None => totals.push((src, r.amount)),
                    }
                }
                totals.into_iter().map(|(_, n)| n).max().unwrap_or(0)
            }
            Amount::OverPlayers { who, amount, how } => {
                let vals: Vec<i32> = self
                    .sel_players(who, c)
                    .iter()
                    .map(|p| {
                        let e = EvalCtx { source: c.source, controller: *p, x: c.x, targets: c.targets, tidx: c.tidx, event: c.event, remembered: c.remembered };
                        self.eval_amount(amount, &e)
                    })
                    .collect();
                match how {
                    PlayerAgg::Max => vals.into_iter().max().unwrap_or(0),
                    PlayerAgg::Min => vals.into_iter().min().unwrap_or(0),
                    PlayerAgg::Sum => vals.into_iter().sum(),
                }
            }
            Amount::ColorCountOf { of } => self.first_card_or_spell(of, c).map_or(0, |id| self.cards[id.0 as usize].cur.colors.count() as i32),
            Amount::AbilityResolutionsThisTurn => self.cur_ability_res as i32,
            Amount::SpellTargetCount { filter } => match filter {
                None => c.targets.len() as i32,
                Some(f) => {
                    // distinct objects only (one object targeted twice counts once)
                    let mut seen: Vec<CardId> = vec![];
                    for t in c.targets.iter() {
                        if let Obj::Card { id } = t.obj {
                            if !seen.contains(&id) && self.matches_card(f, id, c.source, c.controller) {
                                seen.push(id);
                            }
                        }
                    }
                    seen.len() as i32
                }
            },
            Amount::TargetCountOf { of, filter } => match self.first_card_or_spell(of, c) {
                Some(id) => self.stack.iter().find(|it| it.source == id && matches!(it.kind, ItemKind::Spell { .. })).map_or(0, |it| match filter {
                    None => it.targets.len() as i32,
                    Some(f) => {
                        let mut seen: Vec<CardId> = vec![];
                        for t in it.targets.iter() {
                            if let Obj::Card { id: tid } = t.obj {
                                if !seen.contains(&tid) && self.cards[tid.0 as usize].zone == Zone::Battlefield && self.matches_card(f, tid, c.source, c.controller) {
                                    seen.push(tid);
                                }
                            }
                        }
                        seen.len() as i32
                    }
                }),
                None => 0,
            },
            Amount::DamageDealtBySources { from, combat } => {
                let f = Filter { controller: PlayerFilter::Any, ..from.clone().into_filter() };
                self.dmg_log
                    .iter()
                    .filter(|r| combat.map_or(true, |cb| r.combat == cb))
                    .filter(|r| r.source.map_or(false, |s| r.source_ctrl.map_or(true, |sc| self.player_filter(from.controller, sc, c.controller)) && self.matches_card(&f, s, c.source, c.controller)))
                    .map(|r| r.amount)
                    .sum()
            }
            Amount::EnergySpentThisTurn { who } => self.sel_players(who, c).iter().map(|p| self.players[*p as usize].energy_spent).sum(),
            Amount::ManaInPool { who, color } => self
                .sel_players(who, c)
                .iter()
                .map(|p| {
                    let pool = &self.players[*p as usize].pool;
                    match color {
                        Some(t) => pool.get(*t) as i32,
                        None => crate::mana::ManaType::ALL.iter().map(|t| pool.get(*t) as i32).sum(),
                    }
                })
                .sum(),
            Amount::ExcessDamageDealtToThisTurn { of } => {
                let cs = self.sel_cards(of, c);
                self.dmg_log.iter().filter(|r| matches!(r.target, Obj::Card { id } if cs.contains(&id))).map(|r| r.excess).sum()
            }
            Amount::DamageDealtToThisTurn { of, from } => {
                let cs = self.sel_cards(of, c);
                self.dmg_log
                    .iter()
                    .filter(|r| matches!(r.target, Obj::Card { id } if cs.contains(&id)))
                    .filter(|r| from.as_ref().map_or(true, |f| r.source.map_or(false, |s| self.matches_card(&Filter { controller: PlayerFilter::Any, ..f.clone().into_filter() }, s, c.source, c.controller))))
                    .map(|r| r.amount)
                    .sum()
            }
            Amount::DamageTakenThisTurn { who } => self.sel_players(who, c).iter().map(|p| self.players[*p as usize].damage_taken_turn).sum(),
            Amount::CardsCycledThisTurn { who } => self.sel_players(who, c).iter().map(|p| self.players[*p as usize].cycled_turn).sum(),
            Amount::CardsDiscardedThisTurn { who } => self.sel_players(who, c).iter().map(|p| self.players[*p as usize].discarded_turn).sum(),
            Amount::TimesMutated => self.cards[c.source.0 as usize].merged.len() as i32,
            Amount::TimesRegeneratedThisTurn { of } => self.sel_cards(of, c).first().map_or(0, |id| { let cd = &self.cards[id.0 as usize]; if cd.regen_count_turn == self.turn { cd.regen_count as i32 } else { 0 } }),
            Amount::TimesAttackedThisTurn { of } => self.sel_cards(of, c).first().map_or(0, |id| { let cd = &self.cards[id.0 as usize]; if cd.attack_count_turn == self.turn { cd.attack_count as i32 } else { 0 } }),
            Amount::OpponentsDealtCombatDamageBy { filter } => {
                let f = Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() };
                (0..self.players.len() as PlayerId)
                    .filter(|&o| o != c.controller && self.dmg_log.iter().any(|r| r.combat && r.target == Obj::player(o) && r.source.map_or(false, |s| self.matches_card(&f, s, c.source, c.controller))))
                    .count() as i32
            }
            Amount::GreatestDamageThisTurn => self.dmg_log.iter().map(|r| r.amount).max().unwrap_or(0),
            Amount::LastDamageDealt => self.last_damage.get().0,
            Amount::LifeLostThisWay => self.last_life_lost.get(),
            Amount::EmergeSacrificedToughness => self.cards[c.source.0 as usize].emerge_toughness,
            Amount::CrewedItThisTurn => self.crew_log.iter().filter(|(_, veh, t)| *veh == c.source && *t == self.turn).count() as i32,
            Amount::SurveilKeptOnTop => self.last_surveil_kept.get(),
            Amount::LastExcessDamage => self.last_damage.get().1,
            Amount::AttachedToEnchantedPlayer { filter } => {
                let Some(pl) = self.cards[c.source.0 as usize].enchanted_player else { return 0 };
                let f = Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() };
                self.battlefield.iter().filter(|id| self.cards[id.0 as usize].enchanted_player == Some(pl) && self.matches_card(&f, **id, c.source, c.controller)).count() as i32
            }
            Amount::OpponentCount => self.players.iter().enumerate().filter(|(i, p)| *i as u8 != c.controller && !p.lost && !p.conceded).count() as i32,
            Amount::LifeLostThisTurn { who } => self.sel_players(who, c).iter().map(|p| self.players[*p as usize].life_lost).sum(),
            Amount::LifeGainEventsThisTurn { who } => self.sel_players(who, c).iter().map(|p| self.players[*p as usize].life_gain_events as i32).sum(),
            Amount::LifeGainedThisTurn { who } => self.sel_players(who, c).iter().map(|p| self.players[*p as usize].life_gained).sum(),
            Amount::Domain { who } => {
                let ps = self.sel_players(who, c);
                let mut n = 0;
                for name in ["Plains", "Island", "Swamp", "Mountain", "Forest"] {
                    let sym = Sym::new(name);
                    let has = self.battlefield.iter().any(|id| {
                        let ca = &self.cards[id.0 as usize];
                        ca.cur.types.has(CardType::Land) && ps.contains(&ca.cur.controller) && ca.cur.subtypes.contains(&sym)
                    });
                    if has {
                        n += 1;
                    }
                }
                n
            }
            Amount::DistinctPowers { filter, zone } => {
                let mut v: Vec<i32> = vec![];
                for id in self.cards_matching(filter, *zone, c.source, c.controller) {
                    let p = self.cards[id.0 as usize].cur.power;
                    if !v.contains(&p) {
                        v.push(p);
                    }
                }
                v.len() as i32
            }
            Amount::DistinctManaValues { filter, zone } => {
                let mut v: Vec<u32> = vec![];
                for id in self.cards_matching(filter, *zone, c.source, c.controller) {
                    let mv = self.mv_of(id, self.def_of(id));
                    if !v.contains(&mv) {
                        v.push(mv);
                    }
                }
                v.len() as i32
            }
            Amount::DistinctManaCosts { filter, zone } => {
                let mut v: Vec<String> = vec![];
                for id in self.cards_matching(filter, *zone, c.source, c.controller) {
                    let k = format!("{:?}", self.def_of(id).cost);
                    if !v.contains(&k) {
                        v.push(k);
                    }
                }
                v.len() as i32
            }
            Amount::DistinctSubtypes { filter, zone } => {
                let mut v: Vec<Sym> = vec![];
                for id in self.cards_matching(filter, *zone, c.source, c.controller) {
                    for s in self.cards[id.0 as usize].cur.subtypes.iter() {
                        if !v.contains(s) {
                            v.push(*s);
                        }
                    }
                }
                v.len() as i32
            }
            Amount::AllCounters { filter } => self
                .cards_matching(filter, None, c.source, c.controller)
                .into_iter()
                .map(|id| self.cards[id.0 as usize].counters.iter().map(|(_, n)| *n as i32).sum::<i32>())
                .sum(),
            Amount::TurnsTaken { who } => self.sel_players(who, c).first().map_or(0, |p| self.players[*p as usize].turns_taken as i32),
            Amount::PlayersIn { who } => self.sel_players(who, c).len() as i32,
            Amount::ExtremeOverPlayers { who, amount, least } => {
                let ps = self.sel_players(who, c);
                let vals = ps.iter().map(|p| {
                    let cc = EvalCtx { controller: *p, source: c.source, x: c.x, targets: c.targets, tidx: c.tidx, event: c.event, remembered: c.remembered };
                    self.eval_amount(amount, &cc)
                });
                if *least { vals.min().unwrap_or(0) } else { vals.max().unwrap_or(0) }
            }
            Amount::GreatestSharedName { filter, zone } => {
                let mut tally: Vec<(String, i32)> = vec![];
                for id in self.cards_matching(filter, *zone, c.source, c.controller) {
                    let n = self.def_of(id).name.to_string();
                    match tally.iter_mut().find(|(t, _)| *t == n) {
                        Some(e) => e.1 += 1,
                        None => tally.push((n, 1)),
                    }
                }
                tally.iter().map(|(_, n)| *n).max().unwrap_or(0)
            }
            Amount::GreatestSharedSubtype { filter, zone } => {
                let mut tally: Vec<(Sym, i32)> = vec![];
                let mut changelings = 0;
                for id in self.cards_matching(filter, *zone, c.source, c.controller) {
                    let k = &self.cards[id.0 as usize].cur;
                    if k.kw.has(Keyword::Changeling) {
                        changelings += 1;
                        continue;
                    }
                    for s in k.subtypes.iter() {
                        match tally.iter_mut().find(|(t, _)| t == s) {
                            Some(e) => e.1 += 1,
                            None => tally.push((*s, 1)),
                        }
                    }
                }
                changelings + tally.iter().map(|(_, n)| *n).max().unwrap_or(0)
            }
            Amount::DistinctNames { filter, zone } => {
                let mut v: Vec<String> = vec![];
                for id in self.cards_matching(filter, *zone, c.source, c.controller) {
                    let d = self.def_of(id);
                    // a token without a name of its own is named after its creature types ("Gnome Soldier"), whichever token card it is
                    let n = if d.token && !d.subtypes.is_empty() && !d.supertypes.contains(&Supertype::Legendary) {
                        d.subtypes.iter().map(|t| t.name()).collect::<Vec<_>>().join(" ")
                    } else {
                        d.name.to_string()
                    };
                    if !v.contains(&n) {
                        v.push(n);
                    }
                }
                v.len() as i32
            }
            Amount::TotalManaValue { filter, zone } => self
                .cards_matching(filter, *zone, c.source, c.controller)
                .into_iter()
                .map(|id| if self.cards[id.0 as usize].cur.no_mana_cost { 0 } else { self.def_of(id).mana_value as i32 })
                .sum(),
            Amount::ColorSymbols { of, colors } => {
                let mut n = 0;
                for id in self.sel_cards(of, c) {
                    for sh in &self.def_of(id).cost.shards {
                        let hit = |x: &Color| colors.is_empty() || colors.contains(x);
                        let ok = match sh {
                            crate::mana::Shard::Color { c: x } | crate::mana::Shard::Phyrexian { c: x } | crate::mana::Shard::HybridGeneric { c: x } | crate::mana::Shard::HybridColorless { c: x } => hit(x),
                            crate::mana::Shard::Hybrid { a, b } | crate::mana::Shard::PhyrexianHybrid { a, b } => hit(a) || hit(b),
                            _ => false,
                        };
                        if ok {
                            n += 1;
                        }
                    }
                }
                n
            }
            Amount::BlockersOfTrigger => self.combat.as_ref().and_then(|cb| cb.attackers.iter().find(|a| Some(a.card) == c.event.source)).map_or(0, |a| a.blockers.len() as i32),
            Amount::ChosenColorsOfSpell => match self.costing_card.get().or(match c.event.obj {
                Some(Obj::Card { id }) => Some(id),
                _ => None,
            }) {
                Some(cc) => {
                    let s = &self.cards[c.source.0 as usize];
                    [s.chosen_color, s.chosen_color2].into_iter().flatten().filter(|col| self.cards[cc.0 as usize].cur.colors.has(*col)).count() as i32
                }
                None => 0,
            },
            Amount::BlockersOfThis => self.combat.as_ref().and_then(|cb| cb.attackers.iter().find(|a| a.card == c.source)).map_or(0, |a| a.blockers.iter().filter(|b| self.cards[b.0 as usize].zone == Zone::Battlefield).count() as i32),
            Amount::GreatestCommanderManaValue { who } => {
                let ps = self.sel_players(who, c);
                self.cards
                    .iter()
                    .filter(|k| k.is_commander && ps.contains(&k.owner) && matches!(k.zone, Zone::Battlefield | Zone::Command))
                    .map(|k| self.db.def(k.def).mana_value as i32)
                    .max()
                    .unwrap_or(0)
            }
            Amount::GreatestPowerSacrificed => self.sac_max_power.get(),
            Amount::CommanderCasts => self.players[c.controller as usize].commander_cast.iter().map(|(_, n)| *n as i32).sum(),
            Amount::OpponentsAttacked => {
                let mut v: Vec<PlayerId> = vec![];
                if let Some(cb) = &self.combat {
                    for a in &cb.attackers {
                        let d = match a.defender {
                            Obj::Player { id } => id,
                            // attacking a planeswalker or battle isn't attacking its controller (melee, Adriana)
                            _ => continue,
                        };
                        if !v.contains(&d) {
                            v.push(d);
                        }
                    }
                }
                v.len() as i32
            }
            Amount::DistinctTypesOf { of } => {
                let mut t = TypeSet::default();
                for id in self.sel_cards(of, c) {
                    t.0 |= self.cards[id.0 as usize].cur.types.0;
                }
                t.0.count_ones() as i32
            }
            Amount::DistinctTypes { filter, zone, permanent_only } => {
                let mut t = TypeSet::default();
                for id in self.cards_matching(filter, *zone, c.source, c.controller) {
                    t.0 |= self.cards[id.0 as usize].cur.types.0;
                }
                if *permanent_only {
                    t.0 &= TypeSet::of(&[CardType::Artifact, CardType::Battle, CardType::Creature, CardType::Enchantment, CardType::Land, CardType::Planeswalker]).0;
                }
                t.0.count_ones() as i32
            }
            Amount::DistinctTypesUnion { parts } => {
                let mut t = TypeSet::default();
                for (filter, zone) in parts {
                    for id in self.cards_matching(filter, *zone, c.source, c.controller) {
                        t.0 |= self.cards[id.0 as usize].cur.types.0;
                    }
                }
                t.0.count_ones() as i32
            }
            Amount::NotedTypes { of } => self.sel_cards(of, c).first().map_or(0, |id| self.cards[id.0 as usize].noted_types.len() as i32),
            Amount::ChosenNumber => self.cards[c.source.0 as usize].chosen_number.unwrap_or(0),
            Amount::CreaturesDiedThisTurn => self.creatures_died_turn as i32,
            Amount::DilemmasFacedThisTurn => self.players[c.controller as usize].dilemmas_turn as i32,
            Amount::PileGuessAbilitiesThisTurn => self.pile_guess_turn as i32,
            Amount::Devotion { .. } | Amount::DevotionToChosenColor => {
                let colors: Vec<Color> = match a {
                    Amount::Devotion { colors } => colors.clone(),
                    _ => self.cards[c.source.0 as usize].chosen_color.into_iter().collect(),
                };
                let mut n = 0;
                for &id in &self.battlefield {
                    let ca = &self.cards[id.0 as usize];
                    if ca.cur.controller != c.controller || ca.face_down {
                        continue;
                    }
                    for sh in &self.def_of(id).cost.shards {
                        let hit = match sh {
                            crate::mana::Shard::Color { c: x } | crate::mana::Shard::Phyrexian { c: x } | crate::mana::Shard::HybridGeneric { c: x } | crate::mana::Shard::HybridColorless { c: x } => colors.contains(x),
                            crate::mana::Shard::Hybrid { a, b } | crate::mana::Shard::PhyrexianHybrid { a, b } => colors.contains(a) || colors.contains(b),
                            _ => false,
                        };
                        if hit {
                            n += 1;
                        }
                    }
                }
                if !colors.is_empty() {
                    for &id in &self.battlefield {
                        let ca = &self.cards[id.0 as usize];
                        if ca.cur.controller != c.controller || ca.cur.no_abilities {
                            continue;
                        }
                        let d = self.def_of(id);
                        for &i in &d.statics {
                            if let Ability::Static { effect: StaticEffect::DevotionBonus { n: b }, .. } = &d.abilities[i as usize] {
                                n += *b;
                            }
                        }
                    }
                }
                n
            }
            Amount::DistinctKeywords { filter, keywords } => {
                let ids: Vec<CardId> = self.battlefield.iter().copied().filter(|id| self.matches_card(filter, *id, c.source, c.controller)).collect();
                let views: Vec<_> = ids.iter().map(|id| self.view(*id)).collect();
                keywords.iter().filter(|k| views.iter().any(|v| v.d.kw.has(**k))).count() as i32
            }
            Amount::DistinctCounterKinds { filter } => {
                let mut kinds: Vec<CounterKind> = vec![];
                for id in self.battlefield.clone() {
                    if self.matches_card(filter, id, c.source, c.controller) {
                        for (k, n) in self.cards[id.0 as usize].counters.iter() {
                            if *n > 0 && !kinds.contains(k) {
                                kinds.push(*k);
                            }
                        }
                    }
                }
                kinds.len() as i32
            }
            Amount::TotalCounters { filter, kind } => {
                let ids: Vec<CardId> = self.battlefield.clone();
                ids.into_iter().filter(|id| self.matches_card(filter, *id, c.source, c.controller)).map(|id| self.cards[id.0 as usize].counter(*kind) as i32).sum()
            }
            Amount::CommanderColorCount => self.commander_identity(c.controller).count() as i32,
            Amount::TotalCountersAllKinds { filter } => {
                let ids: Vec<CardId> = self.battlefield.clone();
                ids.into_iter().filter(|id| self.matches_card(filter, *id, c.source, c.controller)).map(|id| self.cards[id.0 as usize].counters.iter().map(|(_, n)| *n as i32).sum::<i32>()).sum()
            }
            Amount::CardsAboveInGraveyard { filter } => {
                let owner = self.cards[c.source.0 as usize].owner;
                let gy = &self.players[owner as usize].graveyard;
                match gy.iter().position(|x| *x == c.source) {
                    Some(i) => {
                        let ids: Vec<CardId> = gy[i + 1..].to_vec();
                        ids.into_iter().filter(|id| self.matches_card(&Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() }, *id, c.source, c.controller)).count() as i32
                    }
                    None => 0,
                }
            }
            Amount::CardDirectlyAboveInGraveyard { filter } => {
                let owner = self.cards[c.source.0 as usize].owner;
                let gy = &self.players[owner as usize].graveyard;
                match gy.iter().position(|x| *x == c.source).and_then(|i| gy.get(i + 1).copied()) {
                    Some(id) => self.matches_card(&Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() }, id, c.source, c.controller) as i32,
                    None => 0,
                }
            }
            Amount::DistinctDoorNames { filter } => {
                let ids: Vec<CardId> = self.battlefield.clone();
                let mut names: Vec<String> = vec![];
                for id in ids.into_iter().filter(|id| self.matches_card(filter, *id, c.source, c.controller)) {
                    let card = &self.cards[id.0 as usize];
                    let full = self.db.def(card.def).name.clone();
                    let parts: Vec<&str> = full.split(" // ").collect();
                    for (bit, part) in parts.iter().enumerate().take(2) {
                        if card.doors & (1 << bit) != 0 && !names.iter().any(|n| n == part) {
                            names.push(part.to_string());
                        }
                    }
                }
                names.len() as i32
            }
            Amount::UnlockedDoors { filter } => {
                let ids: Vec<CardId> = self.battlefield.clone();
                ids.into_iter().filter(|id| self.matches_card(filter, *id, c.source, c.controller)).map(|id| self.cards[id.0 as usize].doors.count_ones() as i32).sum()
            }
            Amount::CardsPutIntoGraveyardThisTurn { whose, of, from_battlefield, by_controller, tokens, filter } => {
                let f = Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() };
                let of_players: Option<Vec<PlayerId>> = of.as_ref().map(|s| self.sel_players(s, c));
                let tok = if *tokens && *from_battlefield {
                    self.token_gy_turn.iter().filter(|(ctl, owner, ty)| (f.types.is_empty() || f.types.iter().any(|t| ty.has(*t))) && !f.not_types.iter().any(|t| ty.has(*t)) && match &of_players { Some(v) => v.contains(owner), None => self.player_filter_src(c.source, *whose, if *by_controller { *ctl } else { *owner }, c.controller) }).count() as i32
                } else { 0 };
                tok + self.gy_entries
                    .iter()
                    .filter(|(id, _, owner, from)| {
                        (!*from_battlefield || *from == Zone::Battlefield)
                            && (*tokens || !self.cards[id.0 as usize].token)
                            && of_players.as_ref().map_or_else(
                                || {
                                    if *by_controller {
                                        self.left_bf_turn.iter().any(|(lid, p)| lid == id && self.player_filter_src(c.source, *whose, *p, c.controller))
                                    } else {
                                        self.player_filter_src(c.source, *whose, *owner, c.controller)
                                    }
                                },
                                |v| v.contains(owner),
                            )
                            && self.matches_card(&f, *id, c.source, c.controller)
                    })
                    .count() as i32
            }
            Amount::CardsPutIntoExileThisTurn { whose, filter } => {
                let f = Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() };
                self.exile_entries.iter().filter(|(id, o)| self.player_filter_src(c.source, *whose, *o, c.controller) && self.matches_card(&f, *id, c.source, c.controller)).count() as i32
            }
            Amount::SacrificedTypesThisTurn { whose } => {
                let mut t = TypeSet::default();
                for (p, bits) in self.sacrificed_types.iter() {
                    if self.player_filter_src(c.source, *whose, *p, c.controller) {
                        t.0 |= *bits;
                    }
                }
                t.0.count_ones() as i32
            }
            Amount::SacrificedThisTurn { whose, filter } => {
                let f = Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() };
                self.sacrificed_turn.iter().filter(|(id, p)| self.player_filter_src(c.source, *whose, *p, c.controller) && self.matches_card(&f, *id, c.source, c.controller)).count() as i32
            }
            Amount::TreasuresCreatedThisTurn { who } => self.sel_players(who, c).iter().map(|p| self.treasures_made_by.get(*p as usize).copied().unwrap_or(0) as i32).sum(),
            Amount::TokensCreatedThisTurn { who } => self.sel_players(who, c).iter().map(|p| self.tokens_made_by.get(*p as usize).copied().unwrap_or(0) as i32).sum(),
            Amount::PermanentsReturnedToHandThisTurn { whose, filter } => {
                let f = Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() };
                self.bounced_turn.iter().filter(|(id, o)| self.player_filter_src(c.source, *whose, *o, c.controller) && (self.matches_card(&f, *id, c.source, c.controller) || (self.cards[id.0 as usize].token && f == Filter { controller: PlayerFilter::Any, ..Filter::default() }))).count() as i32
            }
            Amount::PermanentsLeftBattlefieldThisTurn { whose, filter, now_in } => {
                let f = Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() };
                self.left_bf_turn.iter().filter(|(id, p)| now_in.map_or(true, |z| self.cards[id.0 as usize].zone == z) && self.player_filter_src(c.source, *whose, *p, c.controller) && self.matches_card(&f, *id, c.source, c.controller)).count() as i32
            }
            Amount::CreaturesDealtCombatDamageThisTurn { whose } => self.combat_damagers_ctl.iter().filter(|(_, p)| self.player_filter_src(c.source, *whose, *p, c.controller)).count() as i32,
            Amount::DiedThisTurnTotalPower { whose, filter } => {
                let f = Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() };
                self.died_turn.iter().zip(self.died_power.iter()).filter(|((id, p), _)| self.player_filter_src(c.source, *whose, *p, c.controller) && self.matches_card(&f, *id, c.source, c.controller)).map(|(_, pw)| *pw).sum()
            }
            Amount::CreaturesDiedThisTurnWhere { whose, filter } => {
                let mut f = Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() };
                // "modified" is judged from what the creature had when it died (its counters are gone by now)
                let want_modified = std::mem::take(&mut f.modified);
                self.died_turn.iter().enumerate().filter(|(i, (id, p))| self.player_filter_src(c.source, *whose, *p, c.controller) && (!want_modified || self.died_modified.get(*i).copied().unwrap_or(false)) && self.matches_card(&f, *id, c.source, c.controller)).count() as i32
            }
            Amount::LifeAtTurnStart { who } => self.sel_players(who, c).iter().map(|p| self.players[*p as usize].life_turn_start).sum(),
            Amount::LifeLostLastTurn { who } => self.sel_players(who, c).iter().map(|p| self.players[*p as usize].life_lost_last).sum(),
            Amount::OpponentsYouAttackedThisTurn => self.attacked_players_turn.iter().filter(|(a, d)| *a == c.controller && *d != c.controller).count() as i32,
            Amount::CardsLeftYourGraveyardThisTurn => self.left_graveyard_by.get(c.controller as usize).copied().unwrap_or(0) as i32,
            Amount::YourCreaturesDiedThisTurn => self.creatures_died_by.get(c.controller as usize).copied().unwrap_or(0) as i32,
            Amount::SpellsCounteredThisTurn { filter, whose, by } => {
                let f = Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() };
                self.spells_countered.iter().filter(|(card, ctl, b)| self.player_filter(*whose, *ctl, c.controller) && self.player_filter(*by, *b, c.controller) && self.matches_card(&f, *card, c.source, c.controller)).count() as i32
            }
            Amount::PlayersLost => self.players.iter().filter(|p| p.lost || p.conceded).count() as i32,
            Amount::SpellsCastThisGame { who } => self.sel_players(who, c).iter().map(|p| self.players[*p as usize].spells_cast_game as i32).sum(),
            Amount::ManaDrained => self.mana_drained_by.iter().find(|(s, _)| *s == c.source).map_or(0, |e| e.1),
            Amount::SpellsCastThisTurn { who } => {
                self.sel_players(who, c).iter().map(|p| self.players[*p as usize].spells_cast as i32).sum()
            }
            Amount::SpellsCastWhere { filter, before_trigger } => {
                let limit = if *before_trigger {
                    let trig = match c.event.obj {
                        Some(Obj::Card { id }) => Some(id),
                        Some(Obj::Stack { .. }) | None => c.event.source,
                        _ => c.event.source,
                    };
                    trig.and_then(|t| self.spells_log.iter().position(|(_, card)| *card == t)).unwrap_or(self.spells_log.len())
                } else {
                    self.spells_log.len()
                };
                let by_you_by_default = filter.controller == PlayerFilter::default();
                let who = filter.controller;
                let mut f = filter.clone();
                f.controller = PlayerFilter::Any;
                self.spells_log
                    .iter()
                    .take(limit)
                    .filter(|(pl, card)| {
                        let ok_player = if by_you_by_default { *pl == c.controller } else { self.player_filter(who, *pl, c.controller) };
                        ok_player && self.matches_card(&f, *card, c.source, c.controller)
                    })
                    .count() as i32
            }
            Amount::SpellsCastLastTurn { who } => {
                self.sel_players(who, c).iter().map(|p| self.players[*p as usize].spells_cast_last as i32).sum()
            }
            Amount::MostSpellsCastLastTurn => self.players.iter().map(|p| p.spells_cast_last as i32).max().unwrap_or(0),
        }
    }

    /// Would this spell destroy a land `ctrl` controls if it resolved right now (`Filter.spell_would_destroy_your_land`)?
    pub(crate) fn spell_would_destroy_land(&self, item: &StackItem, ctrl: PlayerId) -> bool {
        let Some(prog) = self.program_of_base(item, &self.db) else { return false };
        let is_my_land = |id: CardId| {
            let k = &self.cards[id.0 as usize];
            k.zone == Zone::Battlefield && k.cur.controller == ctrl && k.cur.types.has(CardType::Land)
        };
        prog.iter().any(|e| match e {
            Effect::Destroy { what, .. } => match what {
                Sel::Target { idx } => item.targets.get(*idx as usize).map_or(false, |t| matches!(t.obj, Obj::Card { id } if is_my_land(id))),
                Sel::Targets | Sel::SlotTargets { .. } => item.targets.iter().any(|t| matches!(t.obj, Obj::Card { id } if is_my_land(id))),
                Sel::All { filter, .. } => self.battlefield.iter().any(|id| is_my_land(*id) && self.matches_card(filter, *id, item.source, item.controller)),
                _ => false,
            },
            _ => false,
        })
    }

    /// The spell-target predicates of a filter (`spell_has_targets`, `spell_targets`, `spell_targets_you`) against a spell's chosen targets.
    pub(crate) fn spell_targets_match(&self, f: &Filter, targets: &[Target], source: CardId, ctrl: PlayerId) -> bool {
        if f.spell_has_targets && targets.is_empty() {
            return false;
        }
        if f.spell_targets_permanent_or_player && !targets.iter().any(|t| match t.obj {
            Obj::Player { .. } => true,
            Obj::Card { id } => self.cards[id.0 as usize].zone == Zone::Battlefield,
            _ => false,
        }) {
            return false;
        }
        if f.spell_targets_single_opp_object {
            let ok = targets.len() == 1
                && match targets[0].obj {
                    Obj::Player { id } => id != ctrl,
                    Obj::Card { id } => self.cards[id.0 as usize].zone == Zone::Battlefield && self.cards[id.0 as usize].cur.controller != ctrl,
                    _ => false,
                };
            if !ok {
                return false;
            }
        }
        if f.spell_targets_you && !targets.iter().any(|t| matches!(t.obj, Obj::Player { id } if id == ctrl)) {
            return false;
        }
        if f.spell_targets_only_players && (targets.is_empty() || !targets.iter().all(|t| matches!(t.obj, Obj::Player { .. }))) {
            return false;
        }
        if f.spell_targets_enchanted_player {
            let ep = self.cards[source.0 as usize].enchanted_player;
            if ep.is_none() || !targets.iter().any(|t| matches!(t.obj, Obj::Player { id } if Some(id) == ep)) {
                return false;
            }
        }
        if f.spell_targets_opponent && !targets.iter().any(|t| matches!(t.obj, Obj::Player { id } if id != ctrl)) {
            return false;
        }
        if let Some(tf) = &f.spell_targets {
            // a spell on the stack is targeted as a stack object (Counterspell on Kaervek's Torch)
            let hit = targets.iter().any(|t| match t.obj {
                // a target that has left (changed zones since the spell was cast) no longer counts (Keep Safe)
                Obj::Card { id } => (t.zcc == 0 || self.cards[id.0 as usize].zcc == t.zcc) && self.matches_card(tf, id, source, ctrl),
                Obj::Stack { id } => self.stack.iter().find(|s| s.id == id).map_or(false, |s| matches!(s.kind, ItemKind::Spell { .. }) && self.matches_card(tf, s.source, source, ctrl)),
                _ => false,
            });
            if !hit {
                return false;
            }
        }
        if let Some(tf) = &f.spell_targets_stack {
            let hit = targets.iter().any(|t| match t.obj {
                Obj::Stack { id } => self.stack.iter().find(|s| s.id == id).map_or(false, |s| s.targets.iter().any(|x| matches!(x.obj, Obj::Card { id } if self.matches_card(tf, id, source, ctrl)))),
                _ => false,
            });
            if !hit {
                return false;
            }
        }
        true
    }

    pub fn power_of(&self, id: CardId) -> i32 {
        self.cards[id.0 as usize].cur.power
    }

    /// Resolve a selector to players.
    pub fn sel_players(&self, s: &Sel, c: &EvalCtx) -> Vec<PlayerId> {
        match s {
            Sel::You => vec![c.controller],
            Sel::ActivePlayer => vec![self.active],
            Sel::Monarch => self.monarch.into_iter().collect(),
            Sel::EnchantedPlayer => self.cards[c.source.0 as usize].enchanted_player.into_iter().collect(),
            Sel::Owner => vec![self.cards[c.source.0 as usize].owner],
            Sel::EachOpponent => self.opponents(c.controller),
            Sel::PlayersPicked { foe } => {
                let picks = &self.cards[c.source.0 as usize].player_pick;
                let mut out = vec![];
                let mut p = c.controller;
                for _ in 0..self.players.len() {
                    if picks.iter().any(|(q, f)| *q == p && *f == *foe) && self.alive(p) {
                        out.push(p);
                    }
                    p = self.next_player(p);
                }
                out
            }
            // the players among the targets chosen for one target slot (Fiery Justice: "target opponent gains 5 life" after a variable-size damage slot)
            Sel::SlotTargets { idx } => {
                let start = c.tidx.get(*idx as usize).copied().unwrap_or(255);
                if start == 255 {
                    vec![]
                } else {
                    let end = c.tidx.iter().copied().filter(|&p| p != 255 && p > start).min().map_or(c.targets.len(), |p| p as usize);
                    c.targets[start as usize..end.min(c.targets.len())].iter().filter_map(|t| if let Obj::Player { id } = t.obj { Some(id) } else { None }).collect()
                }
            }
            Sel::EachOpponentExceptTriggerPlayer => self.opponents(c.controller).into_iter().filter(|p| Some(*p) != c.event.player).collect(),
            Sel::EachPlayerExcept { of } => {
                let skip = self.sel_players(&Sel::ControllerOf { of: of.clone() }, c);
                self.apnap().into_iter().filter(|p| !skip.contains(p)).collect()
            }
            Sel::EachPlayerExceptBoth { a, b } => {
                let mut skip = self.sel_players(a, c);
                skip.extend(self.sel_players(b, c));
                self.apnap().into_iter().filter(|p| !skip.contains(p)).collect()
            }
            Sel::EachOpponentExcept { of } => {
                let skip = self.sel_players(of, c);
                self.opponents(c.controller).into_iter().filter(|p| !skip.contains(p)).collect()
            }
            Sel::EachPlayerExceptTriggerPlayer => self.apnap().into_iter().filter(|p| Some(*p) != c.event.player).collect(),
            Sel::EachOpponentOfTriggerPlayer => c.event.player.map_or(vec![], |p| self.opponents(p)),
            Sel::VotersFor { option } => {
                let mut v: Vec<PlayerId> = self.last_ballots.iter().filter(|(_, o)| *o == *option as usize).map(|(p, _)| *p).collect();
                v.sort();
                v.dedup();
                v
            }
            Sel::VotersLikeController => {
                let mine: Vec<usize> = self.last_ballots.iter().filter(|(p, _)| *p == c.controller).map(|(_, o)| *o).collect();
                let mut v: Vec<PlayerId> = self.last_ballots.iter().filter(|(_, o)| mine.contains(o)).map(|(p, _)| *p).collect();
                v.push(c.controller);
                v.sort();
                v.dedup();
                v
            }
            Sel::HighestNumberPlayers => {
                let best = self.player_numbers.iter().flatten().copied().max();
                self.player_numbers.iter().enumerate().filter(|(_, n)| best.is_some() && **n == best).map(|(i, _)| i as PlayerId).collect()
            }
            Sel::OpponentVotersLikeController => {
                let mine: Vec<usize> = self.last_ballots.iter().filter(|(p, _)| *p == c.controller).map(|(_, o)| *o).collect();
                let mut v: Vec<PlayerId> = self.last_ballots.iter().filter(|(p, o)| *p != c.controller && mine.contains(o)).map(|(p, _)| *p).collect();
                v.sort();
                v.dedup();
                v
            }
            Sel::VotersUnlikeController => {
                let mine: Vec<usize> = self.last_ballots.iter().filter(|(p, _)| *p == c.controller).map(|(_, o)| *o).collect();
                let mut v: Vec<PlayerId> = self.last_ballots.iter().filter(|(p, o)| *p != c.controller && !mine.contains(o)).map(|(p, _)| *p).collect();
                v.sort();
                v.dedup();
                v
            }
            Sel::PlayersWhere { filter } => self.apnap().into_iter().filter(|p| self.player_filter_src(c.source, *filter, *p, c.controller)).collect(),
            Sel::EachPlayer => self.apnap(),
            Sel::PlayerMostLife => {
                let alive: Vec<PlayerId> = self.apnap();
                let best = alive.iter().map(|p| self.players[*p as usize].life).max();
                alive.into_iter().filter(|p| Some(self.players[*p as usize].life) == best).collect()
            }
            Sel::PlayerLeastLife => {
                let alive: Vec<PlayerId> = self.apnap();
                let best = alive.iter().map(|p| self.players[*p as usize].life).min();
                alive.into_iter().filter(|p| Some(self.players[*p as usize].life) == best).collect()
            }
            Sel::RandomOpponent => {
                // "an opponent chosen at random": a deterministic pick from the game state (every call within one resolution agrees)
                let opps = self.opponents(c.controller);
                if opps.len() <= 1 {
                    opps
                } else {
                    vec![opps[self.rand_index(c, opps.len())]]
                }
            }
            Sel::RandomOpponentNotEnchanted => {
                let enchanted = self.cards[c.source.0 as usize].enchanted_player;
                let opps: Vec<PlayerId> = self.opponents(c.controller).into_iter().filter(|p| Some(*p) != enchanted).collect();
                if opps.len() <= 1 {
                    opps
                } else {
                    vec![opps[self.rand_index(c, opps.len())]]
                }
            }
            Sel::RandomPlayer => {
                let ps = self.apnap();
                if ps.len() <= 1 {
                    ps
                } else {
                    vec![ps[self.rand_index(c, ps.len())]]
                }
            }
            Sel::RandomAnyTarget => match self.random_any_target(c) {
                Some(Obj::Player { id }) => vec![id],
                _ => vec![],
            },
            Sel::OtherChosenPlayer => {
                let s = &self.cards[c.source.0 as usize];
                match (s.chosen_player, s.chosen_player2, c.event.player) {
                    (Some(a), Some(b), Some(p)) => vec![if p == a { b } else { a }],
                    _ => vec![],
                }
            }
            Sel::LastDamagerController { to, filter } => {
                let ps = self.sel_players(to, c);
                self.dmg_log
                    .iter()
                    .rev()
                    .find(|r| matches!(r.target, Obj::Player { id } if ps.contains(&id)) && r.source.map_or(false, |sid| self.matches_card(filter, sid, c.source, c.controller)))
                    .and_then(|r| r.source_ctrl)
                    .into_iter()
                    .collect()
            }
            Sel::OwnerOfGreatestMv { of } => {
                let cards = self.sel_cards(of, c);
                let mv = |id: &CardId| self.mv_of(*id, self.def_of(*id));
                let best = cards.iter().map(|id| mv(id)).max();
                let top: Vec<CardId> = cards.iter().copied().filter(|id| Some(mv(id)) == best).collect();
                if top.len() == 1 { vec![self.cards[top[0].0 as usize].owner] } else { vec![] }
            }
            Sel::NextPlayer => vec![self.next_player(c.controller)],
            Sel::PreviousPlayer => vec![self.previous_player(c.controller)],
            Sel::DirectionNeighbor { of, reverse } => {
                let Some(&p) = self.sel_players(of, c).first() else { return vec![] };
                let right = self.cards[c.source.0 as usize].chosen_type.map_or(false, |t| t.name() == "Right") != *reverse;
                vec![if right { self.previous_player(p) } else { self.next_player(p) }]
            }
            Sel::Target { idx } => match c.target(*idx) {
                Some(Target { obj: Obj::Player { id }, .. }) => vec![*id],
                _ => vec![],
            },
            Sel::Targets => c
                .targets
                .iter()
                .filter_map(|t| if let Obj::Player { id } = t.obj { Some(id) } else { None })
                .collect(),
            Sel::TriggerPlayer => c.event.player.into_iter().collect(),
            Sel::AssistingPlayer => c.event.source.and_then(|s| self.cards[s.0 as usize].assist_by).into_iter().collect(),
            Sel::ChosenPlayer => self.cards[c.source.0 as usize].chosen_player.into_iter().collect(),
            Sel::TriggerSource | Sel::TriggerStackItem | Sel::StackSource { .. } | Sel::StackAbilities | Sel::DamagedThisBy { .. } | Sel::TriggerDamaged | Sel::GrantSource | Sel::EventCards | Sel::RandomOf { .. } | Sel::OwnedAndLesserMv { .. } => vec![],
            Sel::TriggerObject => match c.event.obj {
                Some(Obj::Player { id }) => vec![id],
                _ => vec![],
            },
            Sel::ControllerOf { of } => {
                // a spell or ability on the stack has its own controller
                let on_stack: Vec<PlayerId> = self
                    .sel_objs(of, c)
                    .into_iter()
                    .filter_map(|o| {
                        if let Obj::Stack { id } = o {
                            self.stack.iter().find(|s| s.id == id).map(|s| s.controller).or_else(|| self.countered_by.iter().rev().find(|(i, _)| *i == id).map(|(_, p)| *p))
                        } else {
                            None
                        }
                    })
                    .collect();
                self.sel_cards(of, c)
                    .into_iter()
                    .map(|id| {
                        // a permanent that already left the battlefield: its last known controller
                        if self.cards[id.0 as usize].zone != Zone::Battlefield {
                            if let Some(l) = c.event.lki.as_deref().or(self.last_lki.as_ref()).filter(|l| l.card == id) {
                                return l.controller;
                            }
                            // several permanents left together (exiled by one spell): the controller each had on the battlefield
                            if let Some((_, ctl)) = self.left_bf_turn.iter().rev().find(|(i, _)| *i == id) {
                                return *ctl;
                            }
                        }
                        self.cards[id.0 as usize].cur.controller
                    })
                    .chain(on_stack)
                    .chain(self.sel_players(of, c))
                    .take(1)
                    .collect()
            }
            Sel::OwnerOf { of } => {
                let mut owners: Vec<PlayerId> = self.sel_cards(of, c).into_iter().map(|id| self.cards[id.0 as usize].owner).take(1).collect();
                if owners.is_empty() {
                    // a spell on the stack
                    owners = self.sel_objs(of, c).into_iter().filter_map(|o| if let Obj::Stack { id } = o { self.stack.iter().find(|s| s.id == id).map(|s| self.cards[s.source.0 as usize].owner) } else { None }).take(1).collect();
                }
                owners
            }
            Sel::OwnersOf { of, filter } => {
                let mut owners: Vec<PlayerId> = vec![];
                for id in self.sel_cards(of, c) {
                    if !self.matches_card(filter, id, c.source, c.controller) {
                        continue;
                    }
                    let o = self.cards[id.0 as usize].owner;
                    if !owners.contains(&o) {
                        owners.push(o);
                    }
                }
                owners
            }
            Sel::PlayersDealtCombatDamageBy { of } => {
                let srcs = self.sel_cards(of, c);
                let mut out: Vec<PlayerId> = vec![];
                for r in self.dmg_log.iter() {
                    if let (true, Some(s), Obj::Player { id }) = (r.combat, r.source, r.target) {
                        if srcs.contains(&s) && !out.contains(&id) {
                            out.push(id);
                        }
                    }
                }
                out
            }
            Sel::PlayersAttackedBy { of } => {
                let mut out: Vec<PlayerId> = vec![];
                for a in self.sel_cards(of, c) {
                    let cd = &self.cards[a.0 as usize];
                    if cd.attacked_players_turn == self.turn {
                        for q in 0..self.players.len() as u8 {
                            if cd.attacked_players & (1u8 << (q & 7)) != 0 && !out.contains(&q) {
                                out.push(q);
                            }
                        }
                    }
                }
                out
            }
            Sel::AttackedBy { of } => self
                .sel_cards(of, c)
                .into_iter()
                .filter_map(|a| self.combat.as_ref().and_then(|cb| cb.attackers.iter().find(|x| x.card == a)).and_then(|x| if let Obj::Player { id } = x.defender { Some(id) } else { None }))
                .collect(),
            Sel::YouAndDefendingPlayer => {
                let mut v = vec![c.controller];
                for p in self.sel_players(&Sel::DefendingPlayer, c) {
                    if !v.contains(&p) {
                        v.push(p);
                    }
                }
                v
            }
            Sel::DefendingPlayer => {
                // "whenever a creature you control attacks, defending player ...": the attacker named by the event, else the source itself
                let src = match c.event.obj {
                    Some(Obj::Card { id }) if self.combat.as_ref().map_or(false, |cb| cb.attackers.iter().any(|a| a.card == id)) => id,
                    _ => c.source,
                };
                let declared: Vec<PlayerId> = self
                    .combat
                    .as_ref()
                    .and_then(|cb| cb.attackers.iter().find(|a| a.card == src))
                    .map(|a| match a.defender {
                        Obj::Player { id } => id,
                        Obj::Card { id } => self.cards[id.0 as usize].cur.controller,
                        _ => c.controller,
                    })
                    .into_iter()
                    .collect();
                // an attack trigger on a permanent that isn't the attacker: the defender named by the attack event
                if declared.is_empty() && self.combat.is_some() {
                    if let Some(p) = c.event.player {
                        return vec![p];
                    }
                }
                // a permanent that isn't attacking, during combat: the players being attacked
                if declared.is_empty() && self.combat.is_some() {
                    let mut ds: Vec<PlayerId> = vec![];
                    for a in self.combat.as_ref().map(|cb| cb.attackers.iter()).into_iter().flatten() {
                        let p = match a.defender {
                            Obj::Player { id } => id,
                            Obj::Card { id } => self.cards[id.0 as usize].cur.controller,
                            _ => continue,
                        };
                        if !ds.contains(&p) {
                            ds.push(p);
                        }
                    }
                    if !ds.is_empty() {
                        return ds;
                    }
                }
                // before attackers are declared ("can't attack unless defending player ..."): the opponents it could attack
                if declared.is_empty() && self.combat.is_none() {
                    self.opponents(c.controller)
                } else {
                    declared
                }
            }
            Sel::Remembered => c.remembered.iter().filter_map(|o| if let Obj::Player { id } = o { Some(*id) } else { None }).collect(),
            Sel::Stored { slot } => self.stored_view().get(*slot as usize).map_or(vec![], |s| s.iter().filter_map(|o| if let Obj::Player { id } = o { Some(*id) } else { None }).collect()),
            Sel::EachObject => self.each_current.iter().filter_map(|o| if let Obj::Player { id } = o { Some(*id) } else { None }).collect(),
            Sel::ChosenByEach { .. } | Sel::BoundCard | Sel::ChosenCard | Sel::LinkedCard | Sel::OwnersOf { .. } | Sel::RememberedSlice { .. } | Sel::RememberedWhere { .. } | Sel::RingBearer | Sel::SlotTargets { .. } | Sel::SpellTargets { .. } | Sel::YourCommanders | Sel::YourCommandersInPlay | Sel::LegalTargetsOf { .. } | Sel::ExiledWithThis | Sel::ConvokedCreatures { .. } | Sel::TopOfLibrary { .. } | Sel::TopOfGraveyard { .. } | Sel::BottomOfLibrary { .. } | Sel::BottomOfGraveyard { .. } => vec![],
            Sel::AttachmentsOf { .. } | Sel::HostOf { .. } | Sel::PairedWith { .. } | Sel::AttachedTo | Sel::Enchanted | Sel::This | Sel::All { .. } | Sel::PhasedOut { .. } | Sel::PhaseHeldBySource | Sel::AllOfPlayer { .. } => vec![],
        }
    }

    /// Resolve a selector to cards.
    /// A pseudo-random index below `n`, fixed by the game state, so every evaluation within one resolution agrees; `Remembered` objects salt it ("for each permanent, choose a player at random").
    pub(crate) fn rand_index(&self, c: &EvalCtx, n: usize) -> usize {
        let mut h = (self.turn as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (self.next_stack_id as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F) ^ (c.source.0 as u64).wrapping_mul(0x1656_67B1_9E37_79F9);
        for o in c.remembered {
            if let Obj::Card { id } = o {
                h = h.rotate_left(13) ^ (id.0 as u64).wrapping_mul(0x2545_F491_4F6C_DD1D);
            }
        }
        h ^= h >> 29;
        h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        h ^= h >> 32;
        (h % n as u64) as usize
    }

    /// The object chosen by `Sel::RandomAnyTarget`: every creature and planeswalker on the battlefield and every player has the same chance.
    fn random_any_target(&self, c: &EvalCtx) -> Option<Obj> {
        let mut pool: Vec<Obj> = self
            .battlefield
            .iter()
            .filter(|id| {
                let t = &self.cards[id.0 as usize].cur.types;
                t.has(CardType::Creature) || t.has(CardType::Planeswalker)
            })
            .map(|id| Obj::card(*id))
            .collect();
        pool.extend(self.apnap().into_iter().map(Obj::player));
        if pool.is_empty() {
            None
        } else {
            Some(pool[self.rand_index(c, pool.len())])
        }
    }

    pub fn sel_cards(&self, s: &Sel, c: &EvalCtx) -> Vec<CardId> {
        match s {
            Sel::RandomAnyTarget => match self.random_any_target(c) {
                Some(Obj::Card { id }) => vec![id],
                _ => vec![],
            },
            Sel::This => vec![c.source],
            Sel::SpellTargets { spell, filter } => {
                let item = self.sel_objs(spell, c).into_iter().find_map(|o| match o {
                    Obj::Stack { id } => self.stack.iter().find(|s| s.id == id).or_else(|| self.stack_lki_items.iter().rev().find(|s| s.id == id && matches!(s.kind, ItemKind::Spell { .. }))),
                    Obj::Card { id } => self.stack.iter().rev().find(|s| s.source == id).or_else(|| self.stack_lki_items.iter().rev().find(|s| s.source == id && !s.copy && matches!(s.kind, ItemKind::Spell { .. }))),
                    _ => None,
                });
                item.map_or(vec![], |it| it.targets.iter().filter_map(|t| if let Obj::Card { id } = t.obj { Some(id) } else { None }).filter(|id| self.matches_card(filter, *id, c.source, c.controller)).collect())
            }
            Sel::YourCommandersInPlay => self.cards.iter().filter(|cd| cd.is_commander && cd.owner == c.controller && matches!(cd.zone, Zone::Battlefield | Zone::Command)).map(|cd| cd.id).collect(),
            Sel::YourCommanders => self.cards.iter().filter(|cd| cd.is_commander && cd.owner == c.controller).map(|cd| cd.id).collect(),
            Sel::Target { idx } => match c.target(*idx) {
                Some(Target { obj: Obj::Card { id }, .. }) => vec![*id],
                _ => vec![],
            },
            Sel::Targets => c.targets.iter().filter_map(|t| if let Obj::Card { id } = t.obj { Some(id) } else { None }).collect(),
            Sel::OwnedAndLesserMv { of } => {
                let cards = self.sel_cards(of, c);
                let mine: Vec<CardId> = cards.iter().copied().filter(|id| self.cards[id.0 as usize].owner == c.controller).collect();
                let Some(best) = mine.iter().map(|id| self.def_of(*id).mana_value).max() else { return vec![] };
                cards.into_iter().filter(|id| self.cards[id.0 as usize].owner == c.controller || self.def_of(*id).mana_value < best).collect()
            }
            Sel::RandomOf { of } => {
                // one of the selected cards, chosen with the seeded RNG ("destroy one of them at random")
                let cs = self.sel_cards(of, c);
                if cs.len() <= 1 {
                    cs
                } else {
                    vec![cs[self.rand_index(c, cs.len())]]
                }
            }
            Sel::SlotTargets { idx } => {
                let start = c.tidx.get(*idx as usize).copied().unwrap_or(255);
                if start == 255 {
                    vec![]
                } else {
                    let end = c.tidx.iter().copied().filter(|&p| p != 255 && p > start).min().map_or(c.targets.len(), |p| p as usize);
                    c.targets[start as usize..end.min(c.targets.len())].iter().filter_map(|t| if let Obj::Card { id } = t.obj { Some(id) } else { None }).collect()
                }
            }
            Sel::PhaseHeldBySource => { let zc = self.cards[c.source.0 as usize].zcc; self.phased.iter().copied().filter(|id| self.cards[id.0 as usize].phase_hold == Some((c.source, zc))).collect() }
            Sel::PhasedOut { filter } => self.phased.iter().copied().filter(|id| self.matches_card(filter, *id, c.source, c.controller)).collect(),
            Sel::All { filter, zone } => self.cards_matching(filter, *zone, c.source, c.controller),
            Sel::AllOfPlayer { player, zone, filter, owned } => {
                let mut out = vec![];
                for p in self.sel_players(player, c) {
                    let pool: Vec<CardId> = match zone {
                        Zone::Hand => self.players[p as usize].hand.clone(),
                        Zone::Graveyard => self.players[p as usize].graveyard.clone(),
                        Zone::Exile => self.players[p as usize].exile.clone(),
                        Zone::Library => self.players[p as usize].library.clone(),
                        Zone::Command => self.players[p as usize].command.clone(),
                        Zone::Sideboard => self.players[p as usize].sideboard.clone(),
                        Zone::Battlefield => self.battlefield.iter().copied().filter(|id| if *owned { self.cards[id.0 as usize].owner == p } else { self.cards[id.0 as usize].cur.controller == p }).collect(),
                        Zone::Stack => vec![],
                    };
                    let mut f = filter.clone();
                    f.controller = PlayerFilter::Any;
                    out.extend(pool.into_iter().filter(|id| self.matches_card(&f, *id, c.source, c.controller)));
                }
                out
            }
            Sel::TriggerObject => match c.event.obj {
                Some(Obj::Card { id }) => vec![id],
                _ => vec![],
            },
            Sel::EventCards => c.event.cards.clone(),
            Sel::TriggerSource => c.event.source.into_iter().collect(),
            Sel::DamagedThisBy { of } => {
                let mut out: Vec<CardId> = vec![];
                for id in self.sel_cards(of, c) {
                    for s in &self.cards[id.0 as usize].damaged_by {
                        if !out.contains(s) {
                            out.push(*s);
                        }
                    }
                }
                out
            }
            Sel::StackSource { of } => self
                .sel_objs(of, c)
                .into_iter()
                .filter_map(|o| match o {
                    Obj::Stack { id } => self.stack.iter().find(|s| s.id == id).map(|s| s.source).or_else(|| self.stack_lki_src.iter().find(|(i, _)| *i == id).map(|(_, c)| *c)),
                    Obj::Card { id } => Some(id),
                    _ => None,
                })
                .collect(),
            Sel::TriggerDamaged => c.event.damaged.into_iter().collect(),
            Sel::GrantSource => c.event.granted_by.into_iter().collect(),
            Sel::HostOf { of } => self.sel_cards(of, c).into_iter().filter_map(|a| self.cards[a.0 as usize].attached_to).collect(),
            Sel::PairedWith { of } => self.sel_cards(of, c).into_iter().filter_map(|a| self.cards[a.0 as usize].paired).filter(|p| self.cards[p.0 as usize].zone == Zone::Battlefield).collect(),
            Sel::AttachedTo | Sel::Enchanted => {
                let src = &self.cards[c.source.0 as usize];
                match src.attached_to {
                    Some(a) => vec![a],
                    // the Aura/Equipment is gone (sacrificed as a cost): use what it was attached to
                    None if src.zone != Zone::Battlefield => src.left_attached_to.filter(|a| self.cards[a.0 as usize].zone == Zone::Battlefield).into_iter().collect(),
                    None => vec![],
                }
            }
            Sel::Remembered => c.remembered.iter().filter_map(|o| if let Obj::Card { id } = o { Some(*id) } else { None }).collect(),
            Sel::RememberedWhere { filter } => {
                let f = Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() };
                c.remembered.iter().filter_map(|o| if let Obj::Card { id } = o { Some(*id) } else { None }).filter(|id| self.matches_card(&f, *id, c.source, c.controller)).collect()
            }
            Sel::RememberedSlice { from, count } => c.remembered.iter().skip(*from as usize).take(count.map_or(usize::MAX, |n| n as usize)).filter_map(|o| if let Obj::Card { id } = o { Some(*id) } else { None }).collect(),
            Sel::Stored { slot } => self.stored_view().get(*slot as usize).map_or(vec![], |s| s.iter().filter_map(|o| if let Obj::Card { id } = o { Some(*id) } else { None }).collect()),
            Sel::TopOfLibrary { who, n, filter } => self.sel_players(who, c).into_iter().flat_map(|p| self.players[p as usize].library.iter().rev().filter(|id| self.matches_card(&Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() }, **id, c.source, c.controller)).take(*n as usize).copied().collect::<Vec<_>>()).collect(),
            Sel::BottomOfLibrary { who, n, filter, amount } => { let n = amount.as_ref().map_or(*n as usize, |a| self.eval_amount(a, c).max(0) as usize); self.sel_players(who, c).into_iter().flat_map(|p| self.players[p as usize].library.iter().filter(|id| self.matches_card(&Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() }, **id, c.source, c.controller)).take(n).copied().collect::<Vec<_>>()).collect() },
            Sel::BottomOfGraveyard { who, n, filter } => self.sel_players(who, c).into_iter().flat_map(|p| self.players[p as usize].graveyard.iter().filter(|id| self.matches_card(&Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() }, **id, c.source, c.controller)).take(*n as usize).copied().collect::<Vec<_>>()).collect(),
            Sel::TopOfGraveyard { who, n, filter } => self.sel_players(who, c).into_iter().flat_map(|p| self.players[p as usize].graveyard.iter().rev().filter(|id| self.matches_card(&Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() }, **id, c.source, c.controller)).take(*n as usize).copied().collect::<Vec<_>>()).collect(),
            Sel::ConvokedCreatures { of } => self.first_card_or_spell(of, c).map_or(vec![], |id| self.cards[id.0 as usize].convoked.iter().copied().filter(|x| self.cards[x.0 as usize].zone == Zone::Battlefield).collect()),
            Sel::BoundCard => self.cards[c.source.0 as usize].bound_card.into_iter().collect(),
            Sel::ChosenCard => self.cards[c.source.0 as usize].chosen_card.filter(|l| self.cards[l.0 as usize].zone == Zone::Exile).into_iter().collect(),
            Sel::LinkedCard => self.cards[c.source.0 as usize].linked_card.filter(|l| self.cards[l.0 as usize].zone == Zone::Exile).into_iter().collect(),
            Sel::ExiledWithThis => {
                // while the source is on the battlefield only cards exiled by this incarnation count; after it left (dies / leaves triggers, sacrifice costs) any it exiled
                let on_bf = self.cards[c.source.0 as usize].zone == Zone::Battlefield;
                let z = self.cards[c.source.0 as usize].zcc;
                self.cards.iter().enumerate().filter(|(_, x)| x.zone == Zone::Exile && x.exiled_with.map_or(false, |(s, zz)| s == c.source && (!on_bf || zz == z))).map(|(i, _)| CardId(i as u32)).collect()
            }
            Sel::AttackedBy { of } => self
                .sel_cards(of, c)
                .into_iter()
                .filter_map(|a| self.combat.as_ref().and_then(|cb| cb.attackers.iter().find(|x| x.card == a)).and_then(|x| if let Obj::Card { id } = x.defender { Some(id) } else { None }))
                .collect(),
            Sel::AttachmentsOf { of, filter } => {
                let hosts = self.sel_cards(of, c);
                let f = Filter { controller: PlayerFilter::Any, ..filter.clone().into_filter() };
                let mut out: Vec<CardId> = self.battlefield.iter().copied().filter(|b| self.cards[b.0 as usize].attached_to.map_or(false, |h| hosts.contains(&h)) && self.matches_card(&f, *b, c.source, c.controller)).collect();
                // a host that already left the battlefield (dies trigger): what was attached to it per last-known information
                if let Some(l) = c.event.lki.as_deref() {
                    if hosts.contains(&l.card) && self.cards[l.card.0 as usize].zone != Zone::Battlefield {
                        out.extend(l.attachments.iter().copied().filter(|a| self.matches_card(filter, *a, c.source, c.controller)));
                    }
                }
                out
            }
            Sel::EachObject => self.each_current.iter().filter_map(|o| if let Obj::Card { id } = o { Some(*id) } else { None }).collect(),
            Sel::ChosenByEach { index } => {
                let Some(Obj::Player { id: who }) = self.each_current.first().copied() else { return vec![] };
                let picks = self.hidden_picks.iter().find(|(p, _)| *p == who).map(|(_, v)| v.as_slice()).unwrap_or(&[]);
                match index {
                    Some(i) => picks.get(*i as usize).copied().into_iter().collect(),
                    None => picks.to_vec(),
                }
            }
            Sel::RingBearer => self.players[c.controller as usize].ring_bearer.filter(|b| self.is_ring_bearer(*b)).into_iter().collect(),
            _ => vec![],
        }
    }

    /// Resolve a selector to anything damage / counters etc. could apply to.
    /// The definition of a targeted spell that has already left the stack (countered): printed values stand in for last-known information.
    fn countered_spell_def(&self, s: &Sel, c: &EvalCtx) -> Option<DefId> {
        for o in self.sel_objs(s, c) {
            if let Obj::Stack { id } = o {
                return self.stack.iter().find(|i| i.id == id).map(|i| i.def).or_else(|| self.stack_lki.iter().find(|(i, _)| *i == id).map(|(_, d)| *d));
            }
        }
        None
    }

    /// Like `sel_cards`, but a selected spell on the stack (a targeted spell) yields its card too.
    pub fn sel_cards_or_spell(&self, s: &Sel, c: &EvalCtx) -> Vec<CardId> {
        let mut v = self.sel_cards(s, c);
        if v.is_empty() {
            for o in self.sel_objs(s, c) {
                if let Obj::Stack { id } = o {
                    if let Some(it) = self.stack.iter().find(|i| i.id == id) {
                        v.push(it.source);
                    }
                }
            }
        }
        v
    }

    pub fn sel_objs(&self, s: &Sel, c: &EvalCtx) -> Vec<Obj> {
        let mut out: Vec<Obj> = self.sel_cards(s, c).into_iter().map(Obj::card).collect();
        match s {
            Sel::TriggerObject => {
                if let Some(Obj::Stack { id }) = c.event.obj {
                    out.push(Obj::Stack { id });
                }
            }
            Sel::Target { idx } => {
                if let Some(Target { obj: Obj::Stack { id }, .. }) = c.target(*idx) {
                    out.push(Obj::Stack { id: *id });
                }
            }
            Sel::Targets => {
                for t in c.targets {
                    if let Obj::Stack { id } = t.obj {
                        out.push(Obj::Stack { id })
                    }
                }
            }
            Sel::SpellTargets { spell, .. } => {
                let item = self.sel_objs(spell, c).into_iter().find_map(|o| match o {
                    Obj::Stack { id } => self.stack.iter().find(|s| s.id == id).or_else(|| self.stack_lki_items.iter().rev().find(|s| s.id == id && matches!(s.kind, ItemKind::Spell { .. }))),
                    Obj::Card { id } => self.stack.iter().rev().find(|s| s.source == id).or_else(|| self.stack_lki_items.iter().rev().find(|s| s.source == id && !s.copy && matches!(s.kind, ItemKind::Spell { .. }))),
                    _ => None,
                });
                if let Some(item) = item {
                    out.extend(item.targets.iter().map(|t| t.obj).filter(|o| !matches!(o, Obj::Card { .. })));
                }
            }
            Sel::LegalTargetsOf { spell, filter, players, except_current } => {
                let item = self.sel_objs(spell, c).into_iter().find_map(|o| match o {
                    Obj::Stack { id } => self.stack.iter().find(|s| s.id == id).or_else(|| self.stack_lki_items.iter().rev().find(|s| s.id == id && matches!(s.kind, ItemKind::Spell { .. }))),
                    Obj::Card { id } => self.stack.iter().rev().find(|s| s.source == id).or_else(|| self.stack_lki_items.iter().rev().find(|s| s.source == id && !s.copy && matches!(s.kind, ItemKind::Spell { .. }))),
                    _ => None,
                });
                if let Some(item) = item {
                    let d = self.db.def(self.cards[item.source.0 as usize].def);
                    let spec = d.abilities.iter().find_map(|a| if let Ability::Spell { targets, .. } = a { targets.first() } else { None });
                    // a modal spell (spree): the first target of each chosen mode; an object must be legal for all of them
                    let mode_specs: Vec<&TargetSpec> = if spec.is_some() {
                        vec![]
                    } else {
                        d.abilities.iter().find_map(|a| if let Ability::Modal { modes, .. } = a { Some(item.modes.iter().filter_map(|m| modes.get(*m as usize).and_then(|md| md.targets.first())).collect()) } else { None }).unwrap_or_default()
                    };
                    let spec = spec.or(mode_specs.first().copied());
                    if let Some(spec) = spec {
                        let others: Vec<Vec<Obj>> = mode_specs.iter().skip(1).map(|sp| self.legal_targets(sp, item.source, item.controller)).collect();
                        for o in self.legal_targets(spec, item.source, item.controller).into_iter().filter(|o| others.iter().all(|l| l.contains(o))) {
                            let keep = match o {
                                Obj::Card { id } => self.matches_card(filter, id, c.source, c.controller),
                                Obj::Player { .. } => *players,
                                _ => false,
                            };
                            if keep && !(*except_current && item.targets.iter().any(|t| t.obj == o)) {
                                out.push(o);
                            }
                        }
                    }
                }
            }
            Sel::StackAbilities => {
                for s in &self.stack {
                    if !matches!(s.kind, ItemKind::Spell { .. }) && s.controller == c.controller {
                        out.push(Obj::Stack { id: s.id });
                    }
                }
            }
            // every matching spell on the stack ("counter each spell you don't control")
            Sel::All { filter, zone: Some(Zone::Stack) } => {
                out.retain(|o| !matches!(o, Obj::Card { id } if self.cards[id.0 as usize].zone == Zone::Stack));
                for s in &self.stack {
                    if matches!(s.kind, ItemKind::Spell { .. }) && self.matches_card(filter, s.source, c.source, c.controller) {
                        out.push(Obj::Stack { id: s.id });
                    }
                }
            }
            // "copy this spell": the spell on the stack, not the card
            Sel::This => {
                if self.cards[c.source.0 as usize].zone == Zone::Stack {
                    if let Some(s) = self.stack.iter().rev().find(|s| s.source == c.source && matches!(s.kind, ItemKind::Spell { .. })) {
                        out.retain(|o| *o != Obj::card(c.source));
                        out.push(Obj::Stack { id: s.id });
                    }
                }
            }
            // the spell that caused the trigger ("when you cast this spell"): the stack object, not the card
            Sel::TriggerStackItem => {
                out.extend(c.event.stack.filter(|id| self.stack.iter().any(|s| s.id == *id)).map(|id| Obj::Stack { id }));
            }
            Sel::TriggerSource => {
                for card in c.event.source {
                    if self.cards[card.0 as usize].zone == Zone::Stack {
                        // the spell itself, not a copy of it made earlier in the same trigger (Taigam: copy it, then exile the spell you cast)
                        let spell = self.stack.iter().rev().find(|s| s.source == card && !s.copy && matches!(s.kind, ItemKind::Spell { .. })).or_else(|| self.stack.iter().rev().find(|s| s.source == card && matches!(s.kind, ItemKind::Spell { .. })));
                        if let Some(s) = spell {
                            out.retain(|o| *o != Obj::card(card));
                            out.push(Obj::Stack { id: s.id });
                        }
                    }
                }
            }
            _ => {}
        }
        out.extend(self.sel_players(s, c).into_iter().map(Obj::player));
        out
    }

    pub fn eval_cond(&self, cond: &Condition, c: &EvalCtx) -> bool {
        match cond {
            Condition::Not { c: inner } => !self.eval_cond(inner, c),
            Condition::All { cs } => cs.iter().all(|x| self.eval_cond(x, c)),
            Condition::AnyOf { cs } => cs.iter().any(|x| self.eval_cond(x, c)),
            Condition::LifeAtLeast { who, n } => self.sel_players(who, c).iter().all(|p| self.players[*p as usize].life >= *n),
            Condition::LifeAtMost { who, n } => self.sel_players(who, c).iter().all(|p| self.players[*p as usize].life <= *n),
            Condition::Control { who, filter, at_least } => {
                let ps = self.sel_players(who, c);
                let mut f = filter.clone();
                f.controller = PlayerFilter::Any;
                // "this" in a delayed trigger's condition: a source that changed zones since the trigger was created is a new object
                let stale_this = f.this && c.event.this_zcc.map_or(false, |z| self.cards[c.source.0 as usize].zcc != z);
                let n = if stale_this {
                    0
                } else {
                    self.battlefield
                        .iter()
                        .filter(|id| ps.contains(&self.cards[id.0 as usize].cur.controller) && self.matches_card(&f, **id, c.source, c.controller))
                        .count() as i32
                };
                n >= *at_least
            }
            Condition::ControlSharingType { who, filter, at_least } => {
                let ps = self.sel_players(who, c);
                let mut f = filter.clone();
                f.controller = PlayerFilter::Any;
                let ids: Vec<CardId> = self.battlefield.iter().copied().filter(|id| ps.contains(&self.cards[id.0 as usize].cur.controller) && self.matches_card(&f, *id, c.source, c.controller)).collect();
                self.max_shared_creature_type(&ids) as i32 >= *at_least
            }
            Condition::ExcessDamageDealtThisTurn { controlled_by } => self.dmg_log.iter().any(|r| r.excess > 0 && r.target_ctrl.map_or(false, |p| self.player_filter(*controlled_by, p, c.controller))),
            Condition::TappedLandForManaThisTurn { who } => self.sel_players(who, c).iter().all(|p| self.land_mana_turn & (1u32 << (*p as u32 & 31)) != 0),
            Condition::PlayerDealtCombatDamage { n } => (0..self.players.len() as PlayerId).any(|p| self.dmg_log.iter().filter(|r| r.combat && r.target == Obj::player(p)).map(|r| r.amount).sum::<i32>() >= *n),
            Condition::FaceDownActivityThisTurn => self.players[c.controller as usize].face_down_act_turn == self.turn,
            Condition::NoFaceDownSpellCastThisTurn => self.players[c.controller as usize].face_down_cast_turn != self.turn,
            Condition::YourTurn => self.active == c.controller,
            Condition::OilCounterEventThisTurn => self.oil_died_turn || self.cards.iter().any(|k| k.oil_removed_by == Some(c.controller)),
            Condition::OpponentsTurn => self.active != c.controller,
            Condition::InStep { step } => self.step == *step,
            Condition::ResolvedTimesThisTurn { n } => {
                let r = self.cards[c.source.0 as usize].res_note;
                r.0 == self.turn && r.1 >= *n
            }
            Condition::InCombatPhase => matches!(self.step, Step::BeginCombat | Step::DeclareAttackers | Step::DeclareBlockers | Step::FirstStrikeDamage | Step::CombatDamage | Step::EndCombat),
            Condition::IgnoredUntilEndOfTurn => self.ignored_until_eot.contains(&c.source),
            Condition::SourceTapped => self.cards[c.source.0 as usize].tapped,
            Condition::SourceHasOtherActivatedAbility => {
                let s = &self.cards[c.source.0 as usize];
                !s.cur.no_abilities
                    && (self.def_of(c.source).abilities.iter().any(|a| matches!(a, Ability::Activated { .. }))
                        || s.cur.granted.iter().any(|a| matches!(&**a, Ability::Activated { condition, .. } if !matches!(condition, Some(Condition::SourceHasOtherActivatedAbility)))))
            }
            Condition::GraveyardCardAboveSource { filter } => {
                let s = &self.cards[c.source.0 as usize];
                s.zone == Zone::Graveyard
                    && self.players[s.owner as usize].graveyard.iter().position(|x| *x == c.source).and_then(|i| self.players[s.owner as usize].graveyard.get(i + 1)).map_or(false, |&a| self.matches_card(filter, a, c.source, c.controller))
            }
            Condition::SourceStartedTurnUntapped => {
                let s = &self.cards[c.source.0 as usize];
                s.zone == Zone::Battlefield && !s.tapped_at_turn_start && s.control_turn < self.turn
            }
            Condition::OpponentDealtNoncombatDamageLastTurn => self.opponents(c.controller).iter().any(|o| self.noncombat_dmg_last.contains(o)),
            Condition::SourceControlledSinceTurnStart => {
                let sc = &self.cards[c.source.0 as usize];
                sc.since.max(sc.control_turn) < self.players[sc.cur.controller as usize].last_turn_start
            }
            Condition::SourceDamagedOpponentThisTurn => self.dmg_log.iter().any(|r| r.source == Some(c.source) && r.amount > 0 && matches!(r.target, Obj::Player { id } if id != c.controller)),
            Condition::SourceHasDealtDamage => self.cards[c.source.0 as usize].dealt_damage_ever,
            Condition::Monstrous => self.cards[c.source.0 as usize].monstrous,
            Condition::MostVotes { option, strict } => {
                let mine = self.last_votes.get(*option as usize).copied().unwrap_or(0);
                self.last_votes.iter().enumerate().all(|(i, v)| i == *option as usize || if *strict { mine > *v } else { mine >= *v })
            }
            Condition::CompletedNamedDungeon { name } => self.players[c.controller as usize].completed_dungeon_names.iter().any(|n| n == name),
            Condition::CompletedDungeon { at_least } => self.players[c.controller as usize].dungeons_completed >= *at_least,
            Condition::EnduringStory => {
                // the latch is set by the state-based check; a trigger that fires as the third qualifying permanent enters must already see it
                self.players[c.controller as usize].enduring_story
                    || self
                        .battlefield
                        .iter()
                        .filter(|id| {
                            let k = &self.cards[id.0 as usize];
                            k.cur.controller == c.controller
                                && (k.cur.types.has(CardType::Artifact) || k.cur.supertypes.contains(&Supertype::Legendary) || k.cur.subtypes.iter().any(|s| Sym::name(*s) == "Saga"))
                        })
                        .count()
                        >= 3
            }
            Condition::CityBlessing => {
                // ten permanents give the blessing even before state-based actions run (ascend is checked continuously)
                self.players[c.controller as usize].city_blessing || self.battlefield.iter().filter(|id| self.cards[id.0 as usize].cur.controller == c.controller).count() >= 10
            }
            Condition::IsSuspected => self.cards[c.source.0 as usize].suspected,
            Condition::HaveInitiative => self.initiative == Some(c.controller),
            Condition::PlayerHasInitiative { who } => self.initiative.map_or(false, |i| self.sel_players(who, c).contains(&i)),
            Condition::NoCombatDamageSinceYourLastTurn => {
                let p = &self.players[c.controller as usize];
                p.combat_dmg_turn <= p.prev_turn
            }
            Condition::NoOpponentSpellSinceYourLastTurn => {
                let prev = self.players[c.controller as usize].prev_turn;
                self.opponents(c.controller).into_iter().all(|o| self.players[o as usize].spell_cast_turn <= prev)
            }
            Condition::TriggerPlayerIsActive => c.event.player == Some(self.active),
            Condition::TriggerPlayerAttackedYouThisTurn => c.event.player.map_or(false, |p| self.attacked_players_turn.contains(&(p, c.controller))),
            Condition::TriggerPlayerAttackedYouLastTurn => c.event.player.map_or(false, |p| {
                let pl = &self.players[p as usize];
                let t = if self.active == p { pl.prev_turn_start } else { pl.last_turn_start };
                t > 0 && self.attacked_player_log.iter().any(|&(lt, a, d)| lt == t && a == p && d == c.controller)
            }),
            Condition::TriggerPlayerHasOtherOpponentNotAttacked => c.event.player.map_or(false, |p| {
                let attacked: Vec<PlayerId> = self.combat.as_ref().map_or(vec![], |cb| {
                    cb.attackers.iter().filter_map(|a| match a.defender {
                        // a player is "being attacked" only when attacked directly: a creature attacking their planeswalker doesn't count
                        Obj::Player { id } => Some(id),
                        _ => None,
                    }).collect()
                });
                self.alive_players().into_iter().any(|q| q != p && q != c.controller && !attacked.contains(&q))
            }),
            Condition::ZccMarked => {
                let s = &self.cards[c.source.0 as usize];
                s.zcc_mark == s.zcc + 1
            }
            Condition::AnyPlayerAttackedYouLastTurn => self.alive_players().into_iter().any(|p| {
                let pl = &self.players[p as usize];
                let t = if self.active == p { pl.prev_turn_start } else { pl.last_turn_start };
                p != c.controller && t > 0 && self.attacked_player_log.iter().any(|&(lt, a, d)| lt == t && a == p && d == c.controller)
            }),
            Condition::TriggerSpellAssisted => c.event.source.map_or(false, |s| self.cards[s.0 as usize].assist_by.is_some()),
            Condition::TriggerPlayerIsYou => c.event.player == Some(c.controller),
            Condition::TriggerPlayerAttackedPoisoned => match c.event.player {
                Some(tp) => self.defended_log.iter().any(|(a, d, _, _)| *a == tp && self.players[*d as usize].poison > 0),
                None => false,
            },
            Condition::TriggerPlayerAttackedBy { subtypes } => {
                let subs: Vec<Sym> = subtypes.iter().map(|s| Sym::new(s)).collect();
                match c.event.player {
                    Some(tp) => self.defended_log.iter().any(|(a, d, st, ch)| *a == c.controller && *d == tp && (subs.is_empty() || *ch || subs.iter().any(|s| st.contains(s)))),
                    None => false,
                }
            }
            Condition::ActedLastTurn => {
                let pl = &self.players[c.controller as usize];
                let t = if self.active == c.controller { pl.prev_turn_start } else { pl.last_turn_start };
                t > 0 && pl.act_turn == t
            }
            Condition::DrewLastTurn => {
                let pl = &self.players[c.controller as usize];
                pl.drew_turn > 0 && pl.drew_turn + 1 == self.turn
            }
            Condition::PlayerAttackOnly { c: inner } => self.eval_cond(inner, c),
            Condition::IsRingBearer => self.is_ring_bearer(c.source),
            Condition::RingTempted { at_least } => self.players[c.controller as usize].ring_level >= *at_least,
            Condition::MaxSpeed => self.players[c.controller as usize].speed >= 4,
            Condition::SourcePrepared => self.cards[c.source.0 as usize].prepared_spell.is_some(),
            Condition::Renowned => self.cards[c.source.0 as usize].renowned,
            Condition::Harnessed => self.cards[c.source.0 as usize].harnessed,
            Condition::Solved => self.cards[c.source.0 as usize].solved,
            Condition::SourceMark { mark, at_least } => self.mark_of(c.source, *mark) >= *at_least,
            Condition::SourceHasCounter { kind, at_least } => {
                // after it left the battlefield: what it had when it left (undying / persist)
                match c.event.lki.as_deref() {
                    Some(l) if l.card == c.source && self.cards[c.source.0 as usize].zone != Zone::Battlefield => {
                        l.counters.iter().filter(|(k, _)| k == kind).map(|(_, n)| *n as i32).sum::<i32>() >= *at_least
                    }
                    _ => self.cards[c.source.0 as usize].counter(*kind) >= *at_least,
                }
            }
            Condition::Compare { a, cmp } => cmp.test(self.eval_amount(a, c)),
            Condition::CompareAmounts { a, op, b } => op.test(self.eval_amount(a, c), self.eval_amount(b, c)),
            Condition::SourceActivationsThisTurn { at_least } => self.cards[c.source.0 as usize].activations.iter().map(|(_, n)| *n as u32).sum::<u32>() >= *at_least as u32,
            Condition::SourceActivationsBelow { limit } => (self.cards[c.source.0 as usize].activations.iter().map(|(_, n)| *n as i32).sum::<i32>()) < self.eval_amount(limit, c),
            Condition::PlayedLandThisTurn => self.players[c.controller as usize].lands_played > 0,
            Condition::Threshold => self.players[c.controller as usize].graveyard.len() >= 7,
            Condition::Delirium => {
                let mut t = TypeSet::default();
                let costing = self.costing_card.get();
                for id in &self.players[c.controller as usize].graveyard {
                    if Some(*id) == costing {
                        continue;
                    }
                    t.0 |= self.cards[id.0 as usize].cur.types.0;
                }
                t.0.count_ones() >= 4
            }
            Condition::GraveyardCardTypes { at_least } => {
                let mut t = TypeSet::default();
                for id in &self.players[c.controller as usize].graveyard {
                    t.0 |= self.cards[id.0 as usize].cur.types.0;
                }
                t.0.count_ones() >= *at_least as u32
            }
            Condition::Metalcraft => {
                self.battlefield
                    .iter()
                    .filter(|id| {
                        let k = &self.cards[id.0 as usize];
                        k.cur.controller == c.controller && k.cur.types.has(CardType::Artifact)
                    })
                    .count()
                    >= 3
            }
            Condition::ChosenTypeIs { value } => self.cards[c.source.0 as usize].chosen_type.map_or(false, |t| Sym::name(t) == *value),
            Condition::SourceFaceDown => self.cards[c.source.0 as usize].face_down,
            Condition::SourceCardIsCreature => self.db.def(self.cards[c.source.0 as usize].def).types.has(CardType::Creature),
            Condition::Monarch { who } => self.sel_players(who, c).iter().any(|p| self.monarch == Some(*p)),
            Condition::SourceEverDealtCombatDamage => self.dmg_game.iter().any(|(s, _, cb)| *cb && *s == c.source),
            Condition::MayForetellNow => self.active == c.controller || self.cards.iter().enumerate().any(|(i, k)| k.zone == Zone::Battlefield && k.controller == c.controller && self.def_of(CardId(i as _)).abilities.iter().any(|a| matches!(a, Ability::Static { effect: StaticEffect::ForetellAnyTime, .. }))),
            Condition::NoForetoldThisTurn => !self.foretold_log.contains(&c.controller),
            Condition::PlayedOrCastFromNonHandThisTurn => self.players[c.controller as usize].nonhand_turn == self.turn,
            Condition::UsedGraveyardThisTurn => self.graveyard_use_log.contains(&c.controller),
            Condition::FirstInstantSorceryFromGraveyardThisTurn => self.gy_instant_sorcery_log.iter().filter(|p| **p == c.controller).count() == 1,
            Condition::CastThisTurn { filter, at_least } => {
                let plain = *filter == (Filter::default()).into();
                if plain {
                    self.players[c.controller as usize].spells_cast as i32 >= *at_least
                } else {
                    // `controller` in the filter says whose spells (relative to the ability's controller; default: yours)
                    let who = if filter.controller == PlayerFilter::Any && filter.controller != PlayerFilter::default() { PlayerFilter::Any } else { filter.controller };
                    let mut f = filter.clone();
                    f.controller = PlayerFilter::Any;
                    let by_you_by_default = filter.controller == PlayerFilter::default();
                    self.spells_log
                        .iter()
                        .filter(|(pl, card)| {
                            let ok_player = if by_you_by_default { *pl == c.controller } else { self.player_filter(who, *pl, c.controller) };
                            ok_player && self.matches_card(&f, *card, c.source, c.controller)
                        })
                        .count() as i32
                        >= *at_least
                }
            }
            Condition::ForSomePlayer { who, cond } => self.sel_players(who, c).into_iter().any(|p| {
                let c2 = EvalCtx { source: c.source, controller: p, x: c.x, targets: c.targets, tidx: c.tidx, event: c.event, remembered: c.remembered };
                self.eval_cond(cond, &c2)
            }),
            Condition::Descended => self.descended.get(c.controller as usize).copied().unwrap_or(false),
            Condition::Revolt => self.revolt.get(c.controller as usize).copied().unwrap_or(false),
            Condition::Morbid => self.creatures_died_turn > 0,
            Condition::FirstCombatThisTurn => self.combats_this_turn <= 1,
            Condition::UniqueMostLife => {
                let alive: Vec<i32> = self.alive_players().iter().map(|p| self.players[*p as usize].life).collect();
                let m = alive.iter().copied().max();
                m.map_or(false, |m| alive.iter().filter(|l| **l == m).count() == 1)
            }
            Condition::UniqueLeastLife => {
                let alive: Vec<i32> = self.alive_players().iter().map(|p| self.players[*p as usize].life).collect();
                let m = alive.iter().copied().min();
                m.map_or(false, |m| alive.iter().filter(|l| **l == m).count() == 1)
            }
            Condition::CounterPutOnYourPermanentThisTurn { kind } => self.counters_log.iter().any(|(_, p, k, _, _, _)| *p == c.controller && k == kind),
            Condition::CounterPutByYouOnCreatureThisTurn { kind } => self.counters_log.iter().any(|(_, _, k, by, cr, _)| *cr && k == kind && *by == Some(c.controller)),
            Condition::FirstCountersOnTriggerObjectThisTurn => match c.event.obj {
                Some(Obj::Card { id }) => self.counters_log.iter().filter(|t| t.0 == id).count() <= 1,
                _ => false,
            },
            Condition::PlaneswalkedToThisTurn => self.planeswalked_to_this_turn.contains(&c.source),
            Condition::FirstUpkeepThisTurn => self.upkeeps_this_turn <= 1,
            Condition::WasMonarchAtTurnStart { who } => self.sel_players(who, c).iter().any(|p| self.monarch_turn_start == Some(*p)),
            Condition::FirstEndStepThisTurn => self.end_steps_this_turn <= 1,
            Condition::AnyCounterPutByYouOnCreatureThisTurn => self.counters_log.iter().any(|(_, _, _, by, cr, _)| *cr && *by == Some(c.controller)),
            Condition::CounterPutOnThisThisTurn => self.counters_log.iter().any(|(id, ..)| *id == c.source),
            Condition::CreatureCardLeftYourGraveyardThisTurn => self.left_graveyard_creature_by.get(c.controller as usize).copied().unwrap_or(0) > 0,
            Condition::CardLeftYourGraveyardThisTurn => self.left_graveyard_by.get(c.controller as usize).copied().unwrap_or(0) > 0,
            Condition::YourCreatureDied => self.creatures_died_by.get(c.controller as usize).copied().unwrap_or(0) > 0,
            Condition::ControlledSubtypeAtCast { subtype } => {
                let s = Sym::new(subtype);
                let w = Sym::new("*");
                self.cards[c.source.0 as usize].cast_subtypes.iter().any(|x| *x == s || *x == w)
            }
            Condition::LoyaltyActivatedThisTurn => self.players[c.controller as usize].loyalty_turn,
            Condition::SurveilledThisTurn => self.players[c.controller as usize].surveilled_turn,
            Condition::ScriedOrSurveilledThisTurn => self.players[c.controller as usize].scried_turn,
            Condition::Raid => self.players[c.controller as usize].attacked_turn,
            Condition::CastAnotherSameNameThisGame => {
                let nm = &self.def_of(c.source).name;
                self.spells_cast_names.iter().filter(|(p, n)| *p == c.controller && n == nm).count() >= 2
            }
            Condition::CastFrom { zone } => {
                // inside a "whenever a spell is cast" trigger the question is about the spell that was cast
                if let Some(sp) = c.event.source.filter(|s| *s != c.source && self.cards[s.0 as usize].zone == Zone::Stack && self.cards[s.0 as usize].cast_from.is_some()) {
                    return self.cards[sp.0 as usize].cast_from == Some(*zone);
                }
                (self.cards[c.source.0 as usize].cast_from == Some(*zone) && !self.copy_resolving.get()) || self.cast_hint.get() == Some(*zone)
            }
            Condition::EventObjectWasCast => matches!(c.event.obj, Some(Obj::Card { id }) if self.cards[id.0 as usize].cast_from.is_some()),
            Condition::ResolvingAsCopy => self.copy_resolving.get(),
            Condition::WasCast => self.cards[c.source.0 as usize].cast_from.is_some() && !self.copy_resolving.get(),
            Condition::CastWithAlt { index } => match (match self.pay_alt_probe.get() { Some(a) if self.pay_for.get() == Some(c.source) => a, _ => self.cards[c.source.0 as usize].cast_alt }, index) {
                (Some(a), Some(i)) => a == *i,
                (Some(_), None) => true,
                _ => false,
            },
            Condition::ExileMarked { mark } => {
                let cd = &self.cards[c.source.0 as usize];
                cd.zone == Zone::Exile && cd.exile_mark == Some(*mark) && cd.exile_mark_turn < self.turn
            }
            Condition::SorcerySpeed => self.sorcery_timing(c.controller),
            Condition::CastWithOwnFlash => self.cards[c.source.0 as usize].cast_own_flash,
            Condition::LibraryAtMost { n } => self.players.iter().any(|p| p.library.len() <= *n as usize),
            Condition::CommittedCrime => self.players[c.controller as usize].crime,
            Condition::DoorUnlocked { door } => self.cards[c.source.0 as usize].doors & (1 << (door - 1)) != 0,
            Condition::FullyUnlocked => self.cards[c.source.0 as usize].doors == 3,
            Condition::Paired => self.cards[c.source.0 as usize].paired.is_some(),
            Condition::Saddled => self.cards[c.source.0 as usize].saddled,
            Condition::NoncreatureDestroyedByOpponentThisTurn => self.nc_destroyed_by_opp & (1 << c.controller) != 0,
            Condition::ThisDiscardedThisTurn => self.cards[c.source.0 as usize].discarded_turn == Some(self.turn),
            Condition::DealtCombatDamageThisTurn { filter } => self.combat_damagers.iter().any(|id| self.matches_card(filter, *id, c.source, c.controller)),
            Condition::DealtCombatDamageToCreature { what } => {
                let cs = self.sel_cards(what, c);
                self.dmg_log.iter().any(|r| r.combat && r.source.map_or(false, |s| cs.contains(&s)) && matches!(r.target, Obj::Card { id } if self.cards[id.0 as usize].cur.types.has(CardType::Creature)))
            }
            Condition::DamagedThisTurn { who } => self.sel_players(who, c).iter().any(|p| self.players[*p as usize].damaged),
            Condition::DamagedThisTurnBy { who, filter } => {
                let ps = self.sel_players(who, c);
                self.dmg_log.iter().any(|r| matches!(r.target, Obj::Player { id } if ps.contains(&id)) && r.source.map_or(false, |sid| self.matches_card(filter, sid, c.source, c.controller)))
            }
            Condition::KickedWith { index } => {
                if let Some(sp) = c.event.source.filter(|s| *s != c.source && self.cards[s.0 as usize].zone == Zone::Stack && self.cards[s.0 as usize].cast_from.is_some()) {
                    return self.cards[sp.0 as usize].kicked_mask & (1 << (*index).min(7)) != 0;
                }
                self.cards[c.source.0 as usize].kicked_mask & (1 << (*index).min(7)) != 0
            }
            Condition::Kicked => {
                // inside a "whenever you cast a spell" trigger the question is about the spell that was cast
                if let Some(sp) = c.event.source.filter(|s| *s != c.source && self.cards[s.0 as usize].zone == Zone::Stack && self.cards[s.0 as usize].cast_from.is_some()) {
                    return self.cards[sp.0 as usize].kicked;
                }
                self.cards[c.source.0 as usize].kicked
            }
            Condition::NonAura => self.cards[c.source.0 as usize].non_aura,
            Condition::AsAura => self.cards[c.source.0 as usize].as_aura,
            Condition::ControllerMatches { filter } => self.player_filter_src(c.source, *filter, c.controller, c.controller),
            Condition::TargetPlayerMatches { idx, filter } => matches!(c.target(*idx), Some(Target { obj: Obj::Player { id }, .. }) if self.player_filter_src(c.source, *filter, *id, c.controller)),
            Condition::TargetIsPlayer { idx } => matches!(c.target(*idx), Some(Target { obj: Obj::Player { .. }, .. })),
            Condition::TargetIs { idx, filter } => match c.target(*idx) {
                Some(Target { obj: Obj::Card { id }, .. }) => self.matches_card(filter, *id, c.source, c.controller),
                // a spell on the stack: judge its card
                Some(Target { obj: Obj::Stack { id }, .. }) => match self.stack.iter().find(|s| s.id == *id) {
                    Some(s) => self.matches_stack_spell(filter, s, c.source, c.controller),
                    None => false,
                },
                _ => false,
            },
            Condition::BidIsLowest { who } => {
                let low = self.alive_players().iter().map(|p| self.life_rounds.get(*p as usize).copied().unwrap_or(0)).min().unwrap_or(0);
                self.sel_players(who, c).iter().all(|p| self.life_rounds.get(*p as usize).copied().unwrap_or(0) <= low)
            }
            Condition::BidIsHighest { who } => {
                let hi = self.alive_players().iter().map(|p| self.life_rounds.get(*p as usize).copied().unwrap_or(0)).max().unwrap_or(0);
                self.sel_players(who, c).iter().all(|p| self.life_rounds.get(*p as usize).copied().unwrap_or(0) >= hi)
            }
            Condition::RandomPick { of, pick } => self.rand_index(c, (*of).max(1) as usize) == *pick as usize,
            Condition::BentAllFour { who } => self.sel_players(who, c).iter().all(|p| {
                let pl = &self.players[*p as usize];
                pl.bend_turn == self.turn && pl.bend_mask == 0b1111
            }),
            Condition::LastFlipWon => self.last_flip_won,
            Condition::FlipLost => self.flip_lost,
            Condition::LastCostPaid => self.last_cost_paid,
            Condition::SharesColorWithEventMana { with } => {
                let mc = c.event.mana_colors;
                self.sel_cards(with, c).iter().any(|id| self.cards[id.0 as usize].cur.colors.0 & mc != 0)
            }
            Condition::ExtraTurn => self.in_extra_turn,
            Condition::BegunExtraTurn => self.players[c.controller as usize].began_extra_turn,
            Condition::ManaFromTreasure { of } => self.first_card_or_spell(of, c).map_or(false, |id| self.cards[id.0 as usize].mana_from_treasure),
            Condition::SelIs { what, filter } => {
                let cards = self.sel_cards_or_spell(what, c);
                !cards.is_empty() && cards.iter().all(|id| self.matches_card(filter, *id, c.source, c.controller))
            }
            Condition::StoredHasPlayer { slot, who } => {
                let p = self.sel_players(who, c).first().copied();
                let st = self.stored_view();
                p.map_or(false, |p| st.get(*slot as usize).map_or(false, |s| s.iter().any(|o| matches!(o, Obj::Player { id } if *id == p))))
            }
            Condition::RememberedIs { filter } => {
                let cards: Vec<CardId> = c.remembered.iter().filter_map(|o| if let Obj::Card { id } = o { Some(*id) } else { None }).collect();
                !cards.is_empty() && cards.iter().all(|id| self.matches_card(filter, *id, c.source, c.controller))
            }
            Condition::RememberedGreatestManaValueBy { who } => {
                let mv = |id: CardId| if self.cards[id.0 as usize].cur.no_mana_cost { 0 } else { self.def_of(id).mana_value as i32 };
                let ids: Vec<CardId> = c.remembered.iter().filter_map(|o| if let Obj::Card { id } = o { Some(*id) } else { None }).collect();
                let top = ids.iter().map(|id| mv(*id)).max();
                top.map_or(false, |t| ids.iter().any(|id| mv(*id) == t && self.player_filter(*who, self.cards[id.0 as usize].owner, c.controller)))
            }
            Condition::DamagedThisTurnByYour { what, filter } => {
                let ids = self.sel_cards(what, c);
                self.dmg_log.iter().any(|r| r.source_ctrl == Some(c.controller) && matches!(r.target, Obj::Card { id } if ids.contains(&id)) && r.source.map_or(false, |sid| self.matches_card(filter, sid, c.source, c.controller)))
            }
            Condition::AnyRememberedIs { filter } => c.remembered.iter().any(|o| matches!(o, Obj::Card { id } if self.matches_card(filter, *id, c.source, c.controller))),
            Condition::AttackedPlayerHasMostLife => {
                let atk = c.event.source.unwrap_or(c.source);
                let def = self.combat.as_ref().and_then(|cb| cb.attackers.iter().find(|a| a.card == atk)).map(|a| a.defender);
                match def {
                    Some(Obj::Player { id }) => {
                        let life = self.players[id as usize].life;
                        self.alive_players().iter().all(|q| self.players[*q as usize].life <= life)
                    }
                    _ => false,
                }
            }
            Condition::AttackedPlayerHasMostLifeAmongOpponents => {
                let atk = c.event.source.unwrap_or(c.source);
                let def = self.combat.as_ref().and_then(|cb| cb.attackers.iter().find(|a| a.card == atk)).map(|a| a.defender);
                match def {
                    Some(Obj::Player { id }) => {
                        let life = self.players[id as usize].life;
                        self.alive_players().iter().all(|q| *q == c.controller || self.players[*q as usize].life <= life)
                    }
                    _ => false,
                }
            }
            Condition::TriggerAttackerAttacksPlayer => {
                let atk = c.event.source.unwrap_or(c.source);
                // an attacker that has left the battlefield is out of combat
                self.cards[atk.0 as usize].zone == Zone::Battlefield && matches!(self.combat.as_ref().and_then(|cb| cb.attackers.iter().find(|a| a.card == atk)).map(|a| a.defender), Some(Obj::Player { .. }))
            }
            Condition::EventCardsNoManaSpent => {
                let ids: Vec<CardId> = if c.event.cards.is_empty() { c.event.obj.iter().filter_map(|o| if let Obj::Card { id } = o { Some(*id) } else { None }).collect() } else { c.event.cards.clone() };
                ids.iter().all(|id| self.cards[id.0 as usize].mana_spent.iter().all(|&n| n == 0))
            }
            Condition::Void => self.nonland_left_turn || self.warped_this_turn,
            Condition::IsDay => self.day_night == Some(true),
            Condition::IsNight => self.day_night == Some(false),
        }
    }
}

impl Game {
    /// A changeling is every creature type: it shares a type with any other creature that has a creature type (or is itself a changeling).
    fn changeling_shares(&self, a: CardId, b: CardId, b_has_type: bool) -> bool {
        let is_changeling = |id: CardId| {
            let c = &self.cards[id.0 as usize];
            if c.zone == Zone::Battlefield { c.cur.all_creature_names || c.cur.kw.has(Keyword::Changeling) } else { self.db.def(c.def).keywords.has(Keyword::Changeling) }
        };
        let (ca, cb) = (is_changeling(a), is_changeling(b));
        (ca && (b_has_type || cb)) || (cb && (self.cards[a.0 as usize].cur.subtypes.len() > 0 || ca))
    }
}

impl Game {
    /// Power and toughness a card's characteristic-defining ability (`CdaPT`) gives it outside the battlefield (CR 604.3: it works in all zones; Tarmogoyf exiled by Duplicant).
    pub(crate) fn cda_pt_off_battlefield(&self, id: CardId) -> Option<(i32, i32)> {
        let d = self.def_of(id);
        for a in &d.abilities {
            if let Ability::Static { effect: StaticEffect::CdaPT { power, toughness }, condition: None, .. } = a {
                let ec = EvalCtx::with_controller(id, self.cards[id.0 as usize].owner);
                return Some((self.eval_amount(power, &ec), self.eval_amount(toughness, &ec)));
            }
        }
        None
    }
}

/// The stored slots as seen by a filter or selector: 0-3 are the live `StoreRemembered` slots, 4 and up the snapshots
/// pinned into lasting objects (`Game::pin_stored`).
pub struct StoredView<'a> {
    live: std::cell::Ref<'a, [Vec<Obj>; 4]>,
    pinned: &'a [Vec<Obj>],
}

impl StoredView<'_> {
    pub fn get(&self, slot: usize) -> Option<&Vec<Obj>> {
        if slot < 4 {
            self.live.get(slot)
        } else {
            self.pinned.get(slot - 4)
        }
    }
}

/// Most snapshots `pinned_sets` keeps (slot ids are a `u8`: 4..=255).
pub const PIN_MAX: usize = 252;

impl Game {
    pub fn stored_view(&self) -> StoredView<'_> {
        StoredView { live: self.stored_sets.borrow(), pinned: &self.pinned_sets }
    }

    /// A lasting object made by a resolving effect (a granted static, a delayed or added trigger, a replacement) keeps the
    /// stored slots it reads as they are now: every read of slot 0-3 (`Sel::Stored`, `Condition::StoredHasPlayer`, the
    /// `*_stored` filter fields) that the object does not write itself first is rewritten to a pinned snapshot. `None` when
    /// nothing reads a slot (use the original then).
    pub fn pin_stored<T: serde::Serialize + serde::de::DeserializeOwned>(&mut self, v: &T) -> Option<T> {
        self.pin_stored_as(v)
    }

    /// `pin_stored` for a value serialized from borrowed parts (a tuple of references) and read back owned.
    pub fn pin_stored_as<S: serde::Serialize, T: serde::de::DeserializeOwned>(&mut self, v: &S) -> Option<T> {
        let mut val = serde_json::to_value(v).ok()?;
        let mut pins: [Option<u8>; 4] = [None; 4];
        let mut written = [false; 4];
        let mut changed = false;
        self.pin_walk(&mut val, &mut pins, &mut written, &mut changed);
        if !changed {
            return None;
        }
        serde_json::from_value(val).ok()
    }

    fn pin_slot(&mut self, slot: usize, pins: &mut [Option<u8>; 4]) -> u8 {
        if let Some(p) = pins[slot] {
            return p;
        }
        let snap = self.stored_sets.borrow()[slot].clone();
        let i = match self.pinned_sets.iter().position(|s| *s == snap) {
            Some(i) => i,
            None if self.pinned_sets.len() < PIN_MAX => {
                self.pinned_sets.push(snap);
                self.pinned_sets.len() - 1
            }
            None => {
                let i = self.pinned_next as usize % PIN_MAX;
                self.pinned_next = ((i + 1) % PIN_MAX) as u16;
                self.pinned_sets[i] = snap;
                i
            }
        };
        let p = (4 + i) as u8;
        pins[slot] = Some(p);
        p
    }

    fn pin_walk(&mut self, v: &mut serde_json::Value, pins: &mut [Option<u8>; 4], written: &mut [bool; 4], changed: &mut bool) {
        use serde_json::Value;
        match v {
            Value::Array(a) => {
                for x in a.iter_mut() {
                    self.pin_walk(x, pins, written, changed);
                }
            }
            Value::Object(m) => {
                let ty = m.get("type").and_then(|t| t.as_str()).unwrap_or("").to_string();
                if ty == "StoreRemembered" {
                    // later reads of a slot the object fills itself stay live
                    if let Some(s) = m.get("slot").and_then(|s| s.as_u64()).filter(|s| *s < 4) {
                        written[s as usize] = true;
                    }
                    return;
                }
                let slot_holder = ty == "Stored" || ty == "StoredHasPlayer";
                for (k, x) in m.iter_mut() {
                    if k.ends_with("_stored") || (slot_holder && k == "slot") {
                        if let Some(s) = x.as_u64().filter(|s| *s < 4 && !written[*s as usize]) {
                            *x = Value::from(self.pin_slot(s as usize, pins));
                            *changed = true;
                        }
                        continue;
                    }
                    self.pin_walk(x, pins, written, changed);
                }
            }
            _ => {}
        }
    }
}
