//! Moving cards between zones, drawing, shuffling.

use crate::eval::EvalCtx;
use crate::ids::*;
use crate::state::*;
use crate::types::*;
use rand::seq::SliceRandom;
use smallvec::SmallVec;

impl Game {
    /// Create a new card instance (a deck card, token or copy) in a zone.
    pub fn create_card(&mut self, def: DefId, owner: PlayerId, zone: Zone, token: bool) -> CardId {
        let id = CardId(self.cards.len() as u32);
        let ts = self.next_ts();
        let d = self.db.def(def);
        let front = if d.is_front { def } else { d.other_face.unwrap_or(def) };
        let card = Card {
            id,
            def,
            front,
            owner,
            controller: owner,
            zone,
            zcc: 0,
            entered_turn: 0,
            noted_from: 0,
            had_to_attack_turn: 0,
            counter_put_turn: 0,
            counter_put_mask: 0,
            entered_as_creature: false,
            entered_from: None,
            discarded_turn: None,
            mut_orig: None,
            mut_parts: vec![],
            merged: vec![],
            haunting: None,
            attacked_turn: 0,
            attack_count: 0,
            attack_count_turn: 0,
            regen_count: 0,
            regen_count_turn: 0,
            surveilled_turn: 0,
            surveilled_zcc: 0,
            milled_turn: 0,
            milled_zcc: 0,
            drawn_turn: 0,
            drawn_zcc: 0,
            attacked_players: 0,
            attacked_players_turn: 0,
            fought_turn: 0,
            dealt_damage_ever: false,
            loyalty_double_turn: 0,
            loyalty_extra_turn: 0,
            loyalty_extra: 0,
            blocked_turn: 0,
            was_blocked_turn: 0,
            block_pairs: Vec::new(),
            may_play_used_turn: 0,
            token_repl_turn: 0,
            may_play_types: 0,
            may_play_types_turn: 0,
            modes_used: vec![],
            face_up_turn: 0,
            enlisted_turn: 0,
            tapped: false,
            face_down: false,
            fd_extra: None,
            fd_copy: false,
            damage: 0,
            deathtouch_hit: false,
            damaged_by: SmallVec::new(),
            dmg_parts: SmallVec::new(),
            regen: 0,
            heal_shield: false,
            regen_steal: None,
            no_regen_turn: 0,
            dmg_lock_turn: 0,
            chosen_type: None,
            chosen_type2: None,
            chosen_types: (0, vec![]),
            player_colors: vec![],
            player_pick: vec![],
            left_chosen_type: None,
            left_chosen_color: None,
            chosen_name: None,
            chosen_name2: None,
            names_used: vec![],
            chosen_color: None,
            chosen_color2: None,
            zcc_mark: 0,
            created_by: None,
            played_via: None,
            gained_rebound: false,
            chosen_player: None,
            bound_card: None,
            chosen_card: None,
            emblem_eot: false,
            chosen_player2: None,
            chosen_number: None,
            noted_types: Vec::new(),
            counters: SmallVec::new(),
            attached_to: None,
            enchanted_player: None,
            protector: None,
            left_attached_to: None,
            left_counters: SmallVec::new(),
            oil_removed_by: None,
            since: 0,
            attacked_player: None,
            attacked_combat: 0,
            attacked_battle_turn: 0,
            control_turn: 0,
            tapped_at_turn_start: false,
            ts,
            token,
            cur: Derived::default(),
            activations: SmallVec::new(),
            trig_ever: SmallVec::new(),
            assist_by: None,
            cast_from: None,
            cast_own_flash: false,
            echo_done_zcc: u32::MAX,
            left_face_down: false,
            cast_subtypes: vec![],
            cast_power: 0,
            phyrexian_life: 0,
            kicked: false,
            granted_offspring: false,
            kicked_mask: 0,
            kick_by: [0; 4],
            teamwork: false,
            kicks: 0,
            monstrous: false,
            renowned: false,
            harnessed: false,
            solved: false,
            taps_turn: (0, 0),
            marks: vec![],
            suspected: false,
            made_by: None,
            detained_by: None,
            bestowed: false,
            non_aura: false,
            as_aura: false,
            extra_sub: None,
            sacrificed_any: 0,
            sacrificed_any_power: 0,
            sacrificed_any_toughness: 0,
            mana_spent: [0; 6],
            act_spent: [0; 6],
            unmask: false,
            noted_mana: 0,
            noted_mana_all: [0; 6],
            last_exiled_with: None,
            pregame_for: None,
            chosen_mask: 0,
            text_changes: vec![],
            text_base: None,
            text_global: None,
            snow_spent: 0,
            snow_colors: 0,
            src_spent: [0; 5],
            paid_src: vec![],
            mana_from_treasure: false,
            convoked: SmallVec::new(),
            rider_counters: 0,
            rider_charge: 0,
            rider_loyalty: 0,
            finality_rider: false,
            enter_counter_rider: None,
            haste_sac_rider: false,
            blitz_rider: false,
            evoke_rider: false,
            sneak_rider: false,
            cast_haste: false,
            riot_rider: false,
            warp_rider: false,
            play_leave_gain: 0,
            prepared_spell: None,
            prepare_of: None,
            cast_prepared: false,
            exiled_by: None,
            linked_card: None,
            exile_on_leave: false,
            granted_suspend: false,
            cloaked: false,
            exile_if_dies: false,
            cast_alt: None,
            cast_exile_after: false,
            cast_bottom_after: false,
            exile_link: None,
            extra_mode: false,
            exile_cast_cost: None,
            on_adventure: false,
            adventure_by: None,
            play_until: 0,
            play_one: false,
            play_limit: 0,
            play_used: 0,
            play_per_type: false,
            play_types_used: 0,
            free_turn: 0,
            free_via: None,
            enter_tapped_once: false,
            fd_revert: None,
            res_note: (0, 0),
            turnup_choice: false,
            play_src: None,
            play_cond: None,
            exile_mark: None,
            exile_mark_turn: 0,
            free_haste: false,
            skip_untap: false,
            skip_untap_by: None,
            skip_untap_more: 0,
            play_by: 0,
            spend_any: false,
            cast_copies: 0,
            beheld_power: None,
            play_free: false,
            play_life_mv: false,
            play_affinity: None,
            token_set_pt: None,
            play_extra: 0,
            play_land_tapped: false,
            play_exile_after: false,
            play_stop_end: false,
            play_stop_combat: false,
            adv_gy_until: 0,
            cast_extra_mana: None,
            phased_out: false,
            phased_indirect: false,
            paired: None,
            sector: 0,
            doors: 0,
            phase_hold: None,
            phase_lock: None,
            emerge_toughness: 0,
            enter_done: 0,
            enter_exiled_pt: (0, 0),
            exiled_with: None,
            linked: None,
            linked_extra: vec![],
            land_played: false,
            saddled: false,
            exhausted: vec![],
            cast_remembered: vec![],
            x: 0,
            is_commander: false,
            cmd_offer: false,
            is_companion: false,
            ceased: false,
            cast_count: 0,
            revealed: false,
            known_to: 0,
        };
        self.cards.push(card);
        self.reset_derived(id);
        self.zone_vec_push(id, zone, owner, false);
        self.layers_dirty = true;
        id
    }

    /// A card's types outside the battlefield with `Biotransference` in force: creature spells and creature cards are also artifacts.
    pub fn types_outside_bf(&self, id: CardId, mut types: TypeSet) -> TypeSet {
        let c = &self.cards[id.0 as usize];
        let who = if c.zone == Zone::Stack { c.controller } else { c.owner };
        if types.has(CardType::Creature) && self.artifact_creatures & (1u8 << (who & 7)) != 0 {
            types = TypeSet(types.0 | CardType::Artifact.bit());
        }
        types
    }

    /// A card's colors outside the battlefield (or as a spell): printed, then Celestial Dawn / Mycosynth Lattice / Painter's Servant.
    pub fn colors_outside_bf(&self, id: CardId) -> ColorSet {
        let c = &self.cards[id.0 as usize];
        let d = self.db.def(c.def);
        let who = if c.zone == Zone::Stack { c.controller } else { c.owner };
        let mut base = if self.colorless_all { ColorSet::default() } else { d.colors };
        // Devoid and similar characteristic-defining color statics on the card itself work everywhere, including as a spell
        for &i in &d.statics {
            if let crate::dsl::Ability::Static { effect: crate::dsl::StaticEffect::SetColors { affects, colors }, condition: None, from: None } = &d.abilities[i as usize] {
                if affects.this {
                    base = ColorSet::of(colors);
                }
            }
        }
        if self.white_nonland & (1u8 << (who & 7)) != 0 && !d.types.has(CardType::Land) {
            base = ColorSet::of(&[Color::W]);
        }
        ColorSet(base.0 | self.color_all)
    }

    /// `StaticEffect::NonBattlefieldCreature` on the card itself: it is a creature (with the subtype and P/T) while outside the battlefield.
    pub(crate) fn apply_nonbf_creature(&self, id: CardId, cur: &mut Derived) {
        let d = self.db.def(self.cards[id.0 as usize].def);
        for &i in &d.statics {
            if let crate::dsl::Ability::Static { effect: crate::dsl::StaticEffect::NonBattlefieldCreature { subtype, power, toughness }, .. } = &d.abilities[i as usize] {
                cur.types = TypeSet(cur.types.0 | CardType::Creature.bit());
                cur.subtypes.push(Sym::new(subtype));
                cur.power = *power;
                cur.toughness = *toughness;
            }
        }
    }

    /// Characteristics straight from the definition (used outside the battlefield).
    pub fn reset_derived(&mut self, id: CardId) {
        let c = &self.cards[id.0 as usize];
        let d = self.db.def(c.def);
        let cur = Derived {
            types: d.types,
            supertypes: d.supertypes.clone(),
            subtypes: d.subtypes.clone(),
            colors: d.colors,
            power: d.power.unwrap_or(0),
            toughness: d.toughness.unwrap_or(0),
            kw: d.keywords,
            controller: c.controller,
            ..Default::default()
        };
        let mut cur = cur;
        cur.colors = self.colors_outside_bf(id);
        if self.artifact_creatures != 0 {
            cur.types = self.types_outside_bf(id, cur.types);
        }
        self.apply_nonbf_creature(id, &mut cur);
        // mutate: the cards merged under / beneath the top one contribute their keywords and abilities
        for &pd in &c.mut_parts {
            let pdef = self.db.def(pd);
            cur.kw.0 |= pdef.keywords.0;
            for a in &pdef.abilities {
                if matches!(a, crate::dsl::Ability::Triggered { .. } | crate::dsl::Ability::Activated { .. }) {
                    cur.granted.push(std::sync::Arc::new(a.clone()));
                    cur.granted_by.push(id);
                }
            }
        }
        self.cards[id.0 as usize].cur = cur;
    }

    fn zone_vec_push(&mut self, id: CardId, zone: Zone, owner: PlayerId, top_or_bottom_front: bool) {
        match zone {
            Zone::Battlefield => self.battlefield.push(id),
            Zone::Library => {
                let lib = &mut self.players[owner as usize].library;
                if top_or_bottom_front {
                    lib.insert(0, id)
                } else {
                    lib.push(id)
                }
            }
            Zone::Hand => self.players[owner as usize].hand.push(id),
            Zone::Graveyard => self.players[owner as usize].graveyard.push(id),
            Zone::Exile => self.players[owner as usize].exile.push(id),
            Zone::Command => self.players[owner as usize].command.push(id),
            Zone::Sideboard => self.players[owner as usize].sideboard.push(id),
            Zone::Stack => {}
        }
    }

    fn zone_vec_remove(&mut self, id: CardId) {
        let (zone, owner) = {
            let c = &self.cards[id.0 as usize];
            (c.zone, c.owner)
        };
        let rm = |v: &mut Vec<CardId>| {
            if let Some(i) = v.iter().position(|x| *x == id) {
                v.remove(i);
            }
        };
        match zone {
            Zone::Battlefield => rm(&mut self.battlefield),
            Zone::Library => rm(&mut self.players[owner as usize].library),
            Zone::Hand => rm(&mut self.players[owner as usize].hand),
            Zone::Graveyard => rm(&mut self.players[owner as usize].graveyard),
            Zone::Exile => rm(&mut self.players[owner as usize].exile),
            Zone::Command => rm(&mut self.players[owner as usize].command),
            Zone::Sideboard => {
                rm(&mut self.players[owner as usize].sideboard);
                self.cards[id.0 as usize].is_companion = false;
            }
            Zone::Stack => {}
        }
    }

    /// The nonland card types of a card definition, in a fixed order (`GrantPlayFromExile.per_type`).
    fn per_type_types(&self, id: CardId) -> Vec<CardType> {
        let d = self.def_of(id);
        [CardType::Artifact, CardType::Battle, CardType::Creature, CardType::Enchantment, CardType::Instant, CardType::Kindred, CardType::Planeswalker, CardType::Sorcery]
            .into_iter()
            .filter(|t| d.types.has(*t))
            .collect()
    }

    /// Does a `per_type` exile-play permission still have an unused card type for this card?
    pub(crate) fn per_type_available(&self, id: CardId) -> bool {
        let used = self.cards[id.0 as usize].play_types_used;
        self.per_type_types(id).into_iter().any(|t| used & (1 << (t as u32)) == 0)
    }

    /// A card was cast under a `per_type` permission: use up one of its unused card types for the whole group (the type the fewest other granted cards share).
    pub(crate) fn use_play_per_type(&mut self, id: CardId) {
        if !self.cards[id.0 as usize].play_per_type {
            return;
        }
        let via = self.cards[id.0 as usize].played_via;
        let used = self.cards[id.0 as usize].play_types_used;
        let group: Vec<CardId> = self.cards.iter().filter(|c| c.id != id && c.zone == Zone::Exile && c.play_per_type && c.played_via == via && c.play_until != 0).map(|c| c.id).collect();
        let pick = self
            .per_type_types(id)
            .into_iter()
            .filter(|t| used & (1 << (*t as u32)) == 0)
            .min_by_key(|t| group.iter().filter(|g| self.def_of(**g).types.has(*t) && self.cards[g.0 as usize].play_types_used & (1 << (*t as u32)) == 0).count());
        let Some(t) = pick else { return };
        for g in group {
            self.cards[g.0 as usize].play_types_used |= 1 << (t as u32);
        }
    }

    /// A card played from exile under a `GrantPlayFromExile { one: true }` permission was just cast or played: the others granted by the same effect can no longer be played.
    pub(crate) fn revoke_play_one(&mut self, id: CardId) {
        // "you may play up to N of those cards": count this one; at N the rest of the group lose the permission
        let lim = self.cards[id.0 as usize].play_limit;
        if lim > 0 {
            if let Some(via) = self.cards[id.0 as usize].played_via {
                let used = self.cards[id.0 as usize].play_used.saturating_add(1);
                for c in self.cards.iter_mut() {
                    if c.zone == Zone::Exile && c.play_limit > 0 && c.played_via == Some(via) {
                        c.play_used = used;
                        if c.id != id && used >= lim {
                            c.play_until = 0;
                        }
                    }
                }
            }
        }
        if !self.cards[id.0 as usize].play_one {
            return;
        }
        if let Some(via) = self.cards[id.0 as usize].played_via {
            for c in self.cards.iter_mut() {
                if c.id != id && c.zone == Zone::Exile && c.play_one && c.played_via == Some(via) {
                    c.play_until = 0;
                }
            }
        }
    }

    /// A card cast through a `FreeCastOneThisTurn` offer was just cast: the others of that offer can't be cast for free any more.
    pub(crate) fn revoke_free_one(&mut self, id: CardId) {
        let c = &self.cards[id.0 as usize];
        if c.free_turn != self.turn {
            return;
        }
        let Some(via) = c.free_via else { return };
        let turn = self.turn;
        for x in self.cards.iter_mut() {
            if x.id != id && x.free_turn == turn && x.free_via == Some(via) {
                x.free_turn = 0;
                x.free_via = None;
            }
        }
    }

    /// Start a group of simultaneous moves (state-based actions, destroy-all effects): abilities of permanents that leave in the
    /// group still see the others leave.
    pub fn begin_batch(&mut self) {
        self.batch_active = true;
        self.batch_gy_events.clear();
        self.batch_left.clear();
        self.batch_noabil.clear();
        self.batch_granted.clear();
    }

    pub fn end_batch(&mut self) {
        self.batch_active = false;
        self.batch_gy_events.clear();
        self.batch_left.clear();
        self.batch_noabil.clear();
        self.batch_granted.clear();
    }

    /// A prepared creature stops being prepared: its exiled spell copy ceases to exist.
    pub fn unprepare(&mut self, id: CardId) {
        if let Some(t) = self.cards[id.0 as usize].prepared_spell.take() {
            self.cards[t.0 as usize].prepare_of = None;
            if self.cards[t.0 as usize].zone == Zone::Exile && !self.cards[t.0 as usize].ceased {
                self.move_card(t, Zone::Graveyard, MoveOpts::default());
            }
        }
    }

    /// Move a card to `to` (owner's zone). Returns false when nothing moved (tokens that already ceased to exist).
    /// Phase a permanent out (and what's attached to it, indirectly): it leaves `battlefield` without leaving the zone.
    pub fn phase_out(&mut self, id: CardId, indirect: bool) {
        if self.cards[id.0 as usize].zone != Zone::Battlefield || self.cards[id.0 as usize].phased_out || self.cards[id.0 as usize].cur.cant_phase_out {
            return;
        }
        let attached: Vec<CardId> = self.battlefield.iter().copied().filter(|a| self.cards[a.0 as usize].attached_to == Some(id)).collect();
        self.battlefield.retain(|x| *x != id);
        self.phased.push(id);
        let c = &mut self.cards[id.0 as usize];
        c.phased_out = true;
        c.phased_indirect = indirect;
        self.emit(Event::PhasedOut { card: id });
        if let Some(cb) = self.combat.as_mut() {
            cb.attackers.retain(|a| a.card != id);
            for a in cb.attackers.iter_mut() {
                a.blockers.retain(|b| *b != id);
            }
        }
        for a in attached {
            self.phase_out(a, true);
        }
        self.layers_dirty = true;
    }

    /// Phase a permanent in (with what phased out along with it).
    pub fn phase_in(&mut self, id: CardId) {
        if !self.cards[id.0 as usize].phased_out || self.cards[id.0 as usize].phase_lock.is_some() || self.phase_in_barred() {
            return;
        }
        self.phased.retain(|x| *x != id);
        self.battlefield.push(id);
        let c = &mut self.cards[id.0 as usize];
        c.phased_out = false;
        c.phased_indirect = false;
        self.emit(Event::PhasedIn { card: id });
        let back: Vec<CardId> = self.phased.iter().copied().filter(|a| self.cards[a.0 as usize].attached_to == Some(id) && self.cards[a.0 as usize].phased_indirect).collect();
        for a in back {
            self.phase_in(a);
        }
        self.layers_dirty = true;
    }

    /// Does a permanent on the battlefield read the top card of its controller's graveyard for its characteristics (`TextOfTopGraveyardCreature`)?
    pub(crate) fn graveyard_text_on_battlefield(&self) -> bool {
        self.battlefield.iter().any(|&b| {
            let d = self.db.def(self.cards[b.0 as usize].def);
            d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], crate::dsl::Ability::Static { effect: crate::dsl::StaticEffect::TextOfTopGraveyardCreature, .. }))
        })
    }

    /// Was this permanent modified when it left (a counter on it, or Equipment / an Aura its controller controls attached)?
    pub(crate) fn lki_modified(&self, l: &Lki) -> bool {
        !l.counters.is_empty()
            || l.attachments.iter().any(|a| {
                let x = &self.cards[a.0 as usize];
                x.cur.subtypes.contains(&Sym::new("Equipment")) || (x.cur.subtypes.contains(&Sym::new("Aura")) && x.cur.controller == l.controller)
            })
    }

    fn push_left_lki(&mut self) {
        if let Some(l) = self.last_lki.clone() {
            if self.left_lki.len() >= 64 {
                self.left_lki.remove(0);
            }
            self.left_lki.push(l);
        }
    }

    pub fn move_card(&mut self, id: CardId, to: Zone, opts: MoveOpts) -> bool {
        // a melded permanent that leaves the battlefield takes both of its halves with it
        let halves: Option<(CardId, CardId)> = if self.cards[id.0 as usize].zone == Zone::Battlefield && to != Zone::Battlefield {
            self.meld_parts.iter().position(|(r, _, _)| *r == id).map(|i| {
                let (_, a, b) = self.meld_parts.remove(i);
                (a, b)
            })
        } else {
            None
        };
        let from = self.cards[id.0 as usize].zone;
        let r = self.move_card_inner(id, to, opts);
        // (a commander taken from the command zone, as by Command Beacon, isn't offered a way back)
        if r && self.cfg.commander && self.cards[id.0 as usize].is_commander && from != Zone::Command {
            let now = self.cards[id.0 as usize].zone;
            self.cards[id.0 as usize].cmd_offer = false;
            if now != from {
                match now {
                    // CR 903.9a: its owner may put it into the command zone when state-based actions are next checked
                    Zone::Graveyard | Zone::Exile => self.cards[id.0 as usize].cmd_offer = true,
                    _ => {}
                }
            }
        }
        if let Some((a, b)) = halves {
            for h in [a, b] {
                if self.cards[h.0 as usize].zone == Zone::Exile {
                    self.move_card(h, to, Default::default());
                }
            }
        }
        if !self.zone_choice_tmp.borrow().is_empty() {
            let found: Vec<_> = std::mem::take(&mut *self.zone_choice_tmp.borrow_mut());
            for (cid, cands) in found {
                if cid == id && !self.zone_choice_q.iter().any(|q| q.0 == cid) {
                    self.zone_choice_q.push((cid, self.cards[cid.0 as usize].zcc, cands));
                }
            }
        }
        if !self.opt_zone_tmp.borrow().is_empty() {
            let found: Vec<_> = std::mem::take(&mut *self.opt_zone_tmp.borrow_mut());
            for (cid, instead, bottom) in found {
                if r && !self.opt_zone_q.iter().any(|q| q.0 == cid) {
                    self.opt_zone_q.push((cid, self.cards[cid.0 as usize].zcc, instead, bottom));
                }
            }
        }
        if !self.dmg_followups.borrow().is_empty() {
            self.flush_followups();
        }
        r
    }

    fn move_card_inner(&mut self, id: CardId, to: Zone, opts: MoveOpts) -> bool {
        if self.cards[id.0 as usize].ceased {
            return false; // a token that left the battlefield no longer exists
        }
        let from = self.cards[id.0 as usize].zone;
        let mut to = to;
        // a spell cast with flashback is exiled instead of going anywhere else when it leaves the stack (countered, bounced to the library ...)
        if from == Zone::Stack && matches!(to, Zone::Graveyard | Zone::Hand | Zone::Library) {
            let c = &self.cards[id.0 as usize];
            let fb = c.cast_exile_after || c.cast_alt.map_or(false, |i| {
                self.db.def(c.def).abilities.iter().filter_map(|a| if let crate::dsl::Ability::AlternativeCost { exile_after, .. } = a { Some(*exile_after) } else { None }).nth(i as usize).unwrap_or(false)
            });
            if fb {
                to = Zone::Exile;
            }
        }
        let owner = self.cards[id.0 as usize].owner;
        if to == Zone::Battlefield && from != Zone::Battlefield && self.enter_battlefield_barred(id, from) {
            return false;
        }
        // CR 903.9b: a commander going to its owner's hand or library may go to the command zone instead. It goes there now
        // (so "put into the command zone from the battlefield" triggers see it), and its owner is asked whether to keep it
        // there; declining sends it on where it was going.
        if self.cfg.commander && self.cards[id.0 as usize].is_commander && from != Zone::Command && matches!(to, Zone::Hand | Zone::Library) {
            self.opt_zone_tmp.borrow_mut().push((id, to, opts.bottom));
            to = Zone::Command;
        }
        if from == Zone::Battlefield && to == Zone::Exile && self.trigger_token_barred(id) {
            return false; // The Master, Multiplied
        }
        // replacement effects (Rest in Peace and friends) change where the card really goes
        let (to2, to_bottom) = self.replace_zone(id, from, to);
        // a one-shot zone-change replacement that just applied is spent now (Rod of Absorption's exile must not follow the card to its next spell cast)
        self.drain_consumed_replaces();
        to = to2;
        if from == Zone::Battlefield && self.cards[id.0 as usize].exile_on_leave && matches!(to, Zone::Graveyard | Zone::Hand | Zone::Library) {
            to = Zone::Exile;
        }
        if from == Zone::Battlefield && self.cards[id.0 as usize].exile_if_dies && to == Zone::Graveyard {
            to = Zone::Exile;
        }
        if from == Zone::Battlefield && to == Zone::Graveyard && self.cards[id.0 as usize].counter(CounterKind::Finality) > 0 {
            to = Zone::Exile;
        }
        let mut spell_to_bottom = false;
        if from == Zone::Stack {
            if to == Zone::Graveyard && self.cards[id.0 as usize].cast_bottom_after {
                to = Zone::Library;
                spell_to_bottom = true;
            }
            self.cards[id.0 as usize].cast_bottom_after = false;
        }
        if from == Zone::Battlefield {
            self.cards[id.0 as usize].exile_if_dies = false;
            self.cards[id.0 as usize].exile_on_leave = false;
            self.cards[id.0 as usize].monstrous = false;
            self.cards[id.0 as usize].cloaked = false;
            self.cards[id.0 as usize].renowned = false;
            self.cards[id.0 as usize].harnessed = false;
            self.cards[id.0 as usize].solved = false;
            self.cards[id.0 as usize].taps_turn = (0, 0);
        }
        self.cards[id.0 as usize].marks.clear();
        // a permanent that returns is a new object: it isn't attacking or blocking any more (Gossip's Talent, flicker)
        if to == Zone::Battlefield && from != Zone::Battlefield {
            if let Some(cb) = self.combat.as_mut() {
                cb.attackers.retain(|a| a.card != id);
                for a in cb.attackers.iter_mut() {
                    a.blockers.retain(|b| *b != id);
                }
            }
        }
        // a card that goes to a public zone is known to everybody from then on, even if it later returns to a hand or library
        if matches!(to, Zone::Battlefield | Zone::Graveyard | Zone::Stack | Zone::Command) || (to == Zone::Exile && !self.cards[id.0 as usize].face_down) {
            self.cards[id.0 as usize].known_to = 0xFF;
        }
        let mut opts = opts;
        if to_bottom || spell_to_bottom {
            opts.bottom = true;
        }
        // "you may play one of those cards": using one revokes the permission on the others
        if from == Zone::Exile && matches!(to, Zone::Battlefield) {
            self.revoke_play_one(id);
        }
        // leaving the battlefield: record last known information and unattach
        let mut lki = None;
        if from == Zone::Battlefield {
            self.recompute_layers();
            let c = &self.cards[id.0 as usize];
            lki = Some(Lki {
                card: id,
                derived: c.cur.clone(),
                controller: c.cur.controller,
                counters: c.counters.clone(),
                def: c.def,
                token: c.token,
                face_down: c.face_down,
                attached_to: c.attached_to,
                attachments: self
                    .battlefield
                    .iter()
                    .copied()
                    .filter(|a| self.cards[a.0 as usize].attached_to == Some(id))
                    .collect(),
                snapshot: if self.db.def(c.def).lki_snapshot {
                    self.batch_left.iter().copied().chain(self.battlefield.iter().map(|b| (*b, self.cards[b.0 as usize].cur.controller))).collect()
                } else {
                    Vec::new()
                },
            });
        }
        // descended: a permanent card was put into a graveyard this turn
        if to == Zone::Graveyard && !self.cards[id.0 as usize].token && self.def_of(id).is_permanent() {
            let o = self.cards[id.0 as usize].owner as usize;
            if let Some(d) = self.descended.get_mut(o) {
                *d = true;
            }
        }
        self.zone_vec_remove(id);
        // permanents held phased out by this source phase in when it leaves the battlefield
        if from == Zone::Battlefield {
            let zc = self.cards[id.0 as usize].zcc;
            // (a permanent held by itself just stays phased out until the code below clears its phased state)
            let held: Vec<CardId> = self.phased.iter().copied().filter(|h| *h != id && self.cards[h.0 as usize].phase_hold == Some((id, zc))).collect();
            for h in held {
                self.cards[h.0 as usize].phase_hold = None;
                self.phase_in(h);
            }
        }
        if self.cards[id.0 as usize].phased_out {
            self.phased.retain(|x| *x != id);
            self.cards[id.0 as usize].phased_out = false;
            self.cards[id.0 as usize].phased_indirect = false;
        }
        // things attached to a leaving permanent fall off
        if from == Zone::Battlefield {
            let attached: Vec<CardId> =
                self.battlefield.iter().copied().filter(|a| self.cards[a.0 as usize].attached_to == Some(id)).collect();
            for a in attached {
                self.cards[a.0 as usize].attached_to = None;
            }
            // soulbond: the pair ends the moment either creature leaves, so a flickered creature returns unpaired
            if let Some(p) = self.cards[id.0 as usize].paired.take() {
                if self.cards[p.0 as usize].paired == Some(id) {
                    self.cards[p.0 as usize].paired = None;
                }
                self.layers_dirty = true;
            }
            self.ignored_until_eot.retain(|&x| x != id);
            self.cards[id.0 as usize].sector = 0;
            // an exchange of text boxes that lasts only while this permanent is on the battlefield (Exchange of Words) ends
            if self.text_swaps.iter().any(|s| s.0 == id) {
                let swaps: Vec<_> = self.text_swaps.iter().copied().filter(|s| s.0 == id).collect();
                self.text_swaps.retain(|s| s.0 != id);
                for (_, x, y, dx, dy) in swaps {
                    for (card, donor) in [(x, dy), (y, dx)] {
                        let c = &mut self.cards[card.0 as usize];
                        if let Some(i) = c.text_changes.iter().rposition(|(ch, _)| matches!(ch, crate::textchange::TextChange::SetAbilitiesFrom { donor: d } if *d == donor)) {
                            c.text_changes.remove(i);
                            if c.text_changes.is_empty() && c.text_global.is_none() {
                                if let Some(b) = c.text_base.take() {
                                    c.def = b;
                                }
                            } else {
                                self.refresh_text(card);
                            }
                        }
                    }
                }
                self.layers_dirty = true;
            }
            // effects tied to the permanent's duration
            self.effects.retain(|e| !(e.source == id && matches!(e.duration, crate::dsl::Duration::WhileSourceOnBattlefield | crate::dsl::Duration::WhileSourceControlled | crate::dsl::Duration::WhileSourceTapped | crate::dsl::Duration::WhileSourceTappedAffectedPowerLe | crate::dsl::Duration::WhileSourceControlledTapped | crate::dsl::Duration::WhileSourceAttached | crate::dsl::Duration::WhileSourceAttachedToAffected)));
        }
        let ts = self.next_ts();
        // a spell that resolves into a permanent keeps its "this spell gains haste"-style until-end-of-turn effects
        let carry = if from == Zone::Stack && to == Zone::Battlefield { Some(self.cards[id.0 as usize].zcc) } else { None };
        // a spell that resolves into a permanent keeps the cards it exiled as costs (champion, delve, behold)
        if from == Zone::Stack && to == Zone::Battlefield {
            let old_z = self.cards[id.0 as usize].zcc;
            for x in self.cards.iter_mut() {
                if x.exiled_with == Some((id, old_z)) {
                    x.exiled_with = Some((id, old_z + 1));
                }
            }
        }
        let repl_src = self.repl_exile_src.take();
        let exile_tag = if to == Zone::Exile { repl_src.or(self.cur_src).map(|s| (s, self.cards[s.0 as usize].zcc)) } else { None };
        if let Some((s, _)) = exile_tag {
            self.cards[s.0 as usize].last_exiled_with = Some(id);
        }
        {
            let c = &mut self.cards[id.0 as usize];
            c.zone = to;
            if to == Zone::Battlefield {
                // a permanent that re-enters is a new object: its once-each-turn permission is fresh again (Lurrus of the Dream-Den bounced and recast)
                c.may_play_used_turn = 0;
            }
            c.exiled_with = exile_tag;
            c.exiled_by = None;
            if to == Zone::Battlefield {
                c.linked = None;
                c.linked_extra.clear();
                c.land_played = false;
            }
            c.zcc += 1;
            c.dealt_damage_ever = false;
            c.dmg_lock_turn = 0;
            c.ts = ts;
            c.tapped = false;
            c.damage = 0;
            c.deathtouch_hit = false;
            c.regen = 0;
            c.heal_shield = false;
            c.regen_steal = None;
            c.regen_count = 0;
            if from == Zone::Battlefield {
                c.left_attached_to = c.attached_to;
                if to != Zone::Battlefield {
                    c.left_counters = c.counters.clone();
                }
            }
            c.attached_to = None;
            if to == Zone::Battlefield && from != Zone::Stack {
                c.cast_alt = None;
            }
            if to != Zone::Battlefield {
                if from == Zone::Battlefield {
                    c.left_chosen_type = c.chosen_type;
                    c.left_chosen_color = c.chosen_color;
                }
                c.chosen_type = None;
                c.chosen_type2 = None;
                c.player_pick.clear();
                c.chosen_card = None;
                c.chosen_color = None;
                c.chosen_color2 = None;
                c.noted_types.clear();
                c.chosen_mask = 0;
                c.names_used.clear();
                c.noted_mana = 0;
                c.noted_mana_all = [0; 6];
                c.last_exiled_with = None;
            }
            c.left_face_down = c.face_down && from == Zone::Battlefield;
            c.face_down = opts.face_down;
            c.unmask = c.unmask && opts.face_down && to == Zone::Battlefield;
            c.fd_extra = None;
            c.fd_copy = false;
            if to == Zone::Battlefield && opts.face_down {
                let ctl = opts.controller.unwrap_or(c.owner) as usize;
                self.players[ctl].face_down_act_turn = self.turn;
            }
            if to == Zone::Stack && from != Zone::Stack {
                c.convoked.clear();
                c.mana_spent = [0; 6];
                c.snow_spent = 0;
                c.snow_colors = 0;
                c.src_spent = [0; 5];
                c.paid_src.clear();
            }
            c.activations.clear();
            c.trig_ever.clear();
            c.modes_used.clear();
            if to != Zone::Stack {
                c.assist_by = None;
                c.spend_any = false;
                c.beheld_power = None;
            }
            if to != Zone::Battlefield || from != Zone::Battlefield {
                c.exhausted.clear();
            }
            let keep_counters = !matches!(to, Zone::Hand | Zone::Library) && !c.token && {
                let d = self.db.def(c.def);
                d.statics.iter().any(|&i| matches!(&d.abilities[i as usize], crate::dsl::Ability::Static { effect: crate::dsl::StaticEffect::CountersRemain, .. }))
            };
            // a card leaving a graveyard / exile / hand / library was not cast: it was never kicked
            if !matches!(from, Zone::Battlefield | Zone::Stack) {
                c.kicks = 0;
                c.kicked_mask = 0;
                c.kick_by = [0; 4];
            }
            if to != Zone::Battlefield && !keep_counters {
                c.counters.clear();
                c.controller = c.owner;
                c.kicked = false;
                c.granted_offspring = false;
                // a permanent that just left keeps its kick count for its leave-the-battlefield triggers and the squad / offspring copies (last known information)
                // (a countered spell too: its cast trigger, such as replicate, still counts the kicks)
                if !(matches!(from, Zone::Battlefield | Zone::Stack) && matches!(to, Zone::Graveyard | Zone::Exile)) {
                    c.kicks = 0;
                    c.kicked_mask = 0;
                    c.kick_by = [0; 4];
                }
                c.teamwork = false;
                // a permanent that just died keeps how it was cast, so its "when this dies" trigger can ask (blitz: if it was blitzed, draw)
                if !(from == Zone::Battlefield && matches!(to, Zone::Graveyard | Zone::Exile)) {
                    c.cast_alt = None;
                }
                c.on_adventure = false;
                c.adventure_by = None;
                c.bestowed = false;
                c.non_aura = false;
                c.as_aura = false;
                if !matches!(to, Zone::Stack | Zone::Battlefield) {
                    c.extra_sub = None;
                }
                // a spell that left the stack (countered ...) keeps the mana spent on it for abilities that look back (Unravel); casting resets it
                if to != Zone::Stack && from != Zone::Stack {
                    c.mana_spent = [0; 6];
                c.snow_spent = 0;
                c.snow_colors = 0;
                c.src_spent = [0; 5];
                c.paid_src.clear();
                    c.convoked.clear();
                }
                c.exile_cast_cost = None;
                c.play_until = 0;
                c.play_src = None;
                c.play_cond = None;
                c.play_free = false;
                c.play_life_mv = false;
                c.play_affinity = None;
                c.play_extra = 0;
                c.play_land_tapped = false;
                c.play_exile_after = false;
                c.play_stop_end = false;
                c.play_stop_combat = false;
                c.adv_gy_until = 0;
                c.play_one = false;
                c.play_limit = 0;
                c.play_used = 0;
                c.play_per_type = false;
                c.play_types_used = 0;
                c.free_turn = 0;
                c.free_via = None;
                c.enter_tapped_once = false;
                if !(from == Zone::Exile && matches!(to, Zone::Stack | Zone::Battlefield)) {
                    c.played_via = None;
                }
                if from == Zone::Stack || to == Zone::Stack {
                    c.gained_rebound = false;
                }
                c.cast_extra_mana = None;
                if to != Zone::Exile {
                    c.exile_mark = None;
                }
                // a transformed / copied card returns to its front face
                c.def = c.front;
                c.text_changes.clear();
                c.text_base = None;
                c.text_global = None;
                if from == Zone::Battlefield {
                    c.free_haste = false;
                }
                if from != Zone::Stack {
                    c.cast_from = None;
                }
                if to != Zone::Stack {
                    c.haste_sac_rider = false;
                    c.blitz_rider = false;
                    c.evoke_rider = false;
                    c.sneak_rider = false;
                    c.warp_rider = false;
                }
                if from == Zone::Stack && to != Zone::Battlefield {
                    c.finality_rider = false;
                    c.play_leave_gain = 0;
                }
            }
        }
        // any zone change can change what a count-based static ability sees (hand size, graveyard size ...)
        self.layers_dirty = true;
        // mutate: the merged cards leave with the permanent (they were held off every zone list)
        let mut merged_left: Vec<CardId> = vec![];
        if from == Zone::Battlefield && to != Zone::Battlefield && !self.cards[id.0 as usize].merged.is_empty() {
            let comps = std::mem::take(&mut self.cards[id.0 as usize].merged);
            merged_left = comps.clone();
            self.cards[id.0 as usize].mut_parts.clear();
            for comp in comps {
                let cowner = self.cards[comp.0 as usize].owner;
                {
                    let c = &mut self.cards[comp.0 as usize];
                    c.zone = to;
                    c.def = c.front;
                    c.zcc += 1;
                }
                if to != Zone::Stack {
                    self.zone_vec_push(comp, to, cowner, false);
                }
            }
        }
        // tokens cease to exist outside the battlefield
        if from == Zone::Battlefield && self.cards[id.0 as usize].prepared_spell.is_some() {
            self.unprepare(id);
        }
        if self.cards[id.0 as usize].token && to != Zone::Battlefield && to != Zone::Command && to != Zone::Stack {
            // a token that dies still counts as a creature that died (morbid) and as a permanent that left (revolt)
            if from == Zone::Battlefield {
                // a token returned to hand reaches the hand for an instant before it ceases to exist (Barrin, Tolarian Archmage)
                if to == Zone::Hand {
                    let o = self.cards[id.0 as usize].owner;
                    self.bounced_turn.push((id, o));
                }
                if let (Some(l), Zone::Graveyard) = (&lki, to) {
                    self.token_gy_turn.push((l.controller, self.cards[id.0 as usize].owner, l.derived.types));
                }
                if let Some(l) = &lki {
                    if to == Zone::Graveyard && l.derived.types.has(CardType::Creature) {
                        self.creatures_died_turn += 1;
                        let c = l.controller as usize;
                        if self.creatures_died_by.len() <= c {
                            self.creatures_died_by.resize(c + 1, 0);
                        }
                        self.creatures_died_by[c] += 1;
                        self.died_turn.push((id, l.controller));
                        self.died_power.push(l.derived.power);
                        let m = self.lki_modified(l);
                        self.died_modified.push(m);
                    }
                    if let Some(r) = self.revolt.get_mut(l.controller as usize) {
                        *r = true;
                    }
                    if !l.derived.types.has(CardType::Land) {
                        self.nonland_left_turn = true;
                    }
                }
            }
            self.cards[id.0 as usize].zone = to;
            if lki.is_some() {
                self.last_lki = lki.clone();
                self.push_left_lki();
            } else if self.last_lki.as_ref().map_or(false, |l| l.card == id) {
                self.last_lki = None;
            }
            self.lki_slot = lki.take();
            self.emit(Event::Moved { card: id, from, to });
            self.lki_slot = None;
            // a token exiled from the battlefield still counts as having left it (Vren, the Relentless counts exiled tokens)
            if from == Zone::Battlefield && to == Zone::Exile {
                if let Some(l) = &self.last_lki {
                    if l.card == id {
                        let ctl = l.controller;
                        self.left_bf_turn.push((id, ctl));
                    }
                }
            }
            self.cards[id.0 as usize].zone = Zone::Exile; // gone; not in any list
            self.cards[id.0 as usize].zcc += 1;
            self.cards[id.0 as usize].ceased = true;
            return true;
        }
        match to {
            Zone::Battlefield => {
                self.battlefield.push(id);
                let turn = self.turn;
                let c = &mut self.cards[id.0 as usize];
                c.since = turn;
                c.control_turn = turn;
                // a new object: it is not suspected (the flag stays on a card that left, for last-known information)
                c.suspected = false;
                let intended = opts.controller.unwrap_or(c.owner);
                let ctl = self.enters_controller(id, intended);
                if self.db.def(self.cards[id.0 as usize].def).abilities.iter().any(|a| matches!(a, crate::dsl::Ability::EntersUnderOpponent)) {
                    self.under_opponent_chooser.retain(|(x, _)| *x != id);
                    self.under_opponent_chooser.push((id, intended));
                }
                let c = &mut self.cards[id.0 as usize];
                c.controller = ctl;
                if opts.tapped {
                    c.tapped = true;
                }
                self.reset_derived(id);
                self.cards[id.0 as usize].cur.controller = self.cards[id.0 as usize].controller;
                // Room: the door of the half it was cast as is unlocked as it enters (a Room put onto the battlefield has both doors locked)
                let mut room_door = 0u8;
                if self.def_of(id).room {
                    self.cards[id.0 as usize].doors = 0;
                    if from == Zone::Stack {
                        room_door = if self.cards[id.0 as usize].cast_alt.is_some() { 2 } else { 1 };
                        self.cards[id.0 as usize].doors = 1 << (room_door - 1);
                    }
                }
                self.cards[id.0 as usize].entered_from = Some(from);
                self.apply_enters_replacements(id);
                // an effect that puts a shock land onto the battlefield tapped: paying the life would not untap it, so the offer is skipped
                if opts.tapped && self.def_of(id).abilities.iter().any(|x| matches!(x, crate::dsl::Ability::EntersPayLifeOrTapped { .. } | crate::dsl::Ability::EntersRevealOrTapped { .. })) {
                    let bits = if self.def_of(id).abilities.iter().any(|x| matches!(x, crate::dsl::Ability::EntersPayLifeOrTapped { .. })) { 1 } else { 0 } | if self.def_of(id).abilities.iter().any(|x| matches!(x, crate::dsl::Ability::EntersRevealOrTapped { .. })) { 2 } else { 0 };
                    self.cards[id.0 as usize].enter_done |= bits;
                    if !self.enter_pending(id) {
                        if let Some(p) = self.enter_choices.iter().position(|c| *c == id) {
                            self.enter_choices.remove(p);
                        }
                    }
                }
                self.recompute_layers();
                if room_door > 0 {
                    self.emit(Event::Unlocked { card: id, door: room_door });
                }
                // day / night: the first daybound permanent makes it day; a daybound permanent entering at night enters transformed
                if self.def_of(id).keywords.has(Keyword::Daybound) {
                    match self.day_night {
                        None => self.day_night = Some(true),
                        Some(false) => self.transform(id),
                        Some(true) => {}
                    }
                }
            }
            Zone::Library => {
                let lib = &mut self.players[owner as usize].library;
                if opts.bottom {
                    lib.insert(0, id)
                } else {
                    lib.push(id)
                }
                self.reset_derived(id);
            }
            _ => {
                self.zone_vec_push(id, to, owner, false);
                self.reset_derived(id);
            }
        }
        if from == Zone::Battlefield && to != Zone::Battlefield {
            if let Some(l) = &lki {
                self.left_bf_turn.push((id, l.controller));
                self.left_bf_power.push((id, l.derived.power));
                self.left_bf_kw.push((id, l.derived.kw));
            }
        }
        if from == Zone::Battlefield && to == Zone::Graveyard && self.cards[id.0 as usize].left_counters.iter().any(|(k, _)| *k == CounterKind::Oil) {
            self.oil_died_turn = true;
        }
        if from == Zone::Battlefield && to == Zone::Hand {
            let o = self.cards[id.0 as usize].owner;
            self.bounced_turn.push((id, o));
        }
        if to == Zone::Graveyard {
            let c = &self.cards[id.0 as usize];
            let entry = (id, c.zcc, c.owner, from);
            self.gy_entries.push(entry);
        }
        if to == Zone::Exile && !self.cards[id.0 as usize].token {
            let o = self.cards[id.0 as usize].owner;
            self.exile_entries.push((id, o));
            let mv = self.def_of(id).mana_value as i32;
            self.exile_mv_log.push(mv);
        }
        if from == Zone::Graveyard && to != Zone::Graveyard {
            let o = self.cards[id.0 as usize].owner as usize;
            if self.left_graveyard_by.len() <= o {
                self.left_graveyard_by.resize(o + 1, 0);
            }
            self.left_graveyard_by[o] += 1;
            if self.def_of(id).types.has(CardType::Creature) {
                if self.left_graveyard_creature_by.len() <= o {
                    self.left_graveyard_creature_by.resize(o + 1, 0);
                }
                self.left_graveyard_creature_by[o] += 1;
            }
        }
        if to == Zone::Battlefield {
            self.cards[id.0 as usize].entered_as_creature = false;
        }
        if to == Zone::Battlefield && !self.cards[id.0 as usize].token {
            let ctrl = self.cards[id.0 as usize].cur.controller;
            if ctrl == self.active {
                self.players[ctrl as usize].act_turn = self.turn;
            }
        }
        if to == Zone::Battlefield && self.cards[id.0 as usize].cur.types.has(CardType::Creature) {
            let ctrl = self.cards[id.0 as usize].cur.controller;
            self.players[ctrl as usize].creatures_entered += 1;
            self.cards[id.0 as usize].entered_as_creature = true;
        }
        if to == Zone::Battlefield && self.cards[id.0 as usize].cur.types.has(CardType::Land) {
            self.lands_entered_turn += 1;
            let ctrl = self.cards[id.0 as usize].cur.controller;
            self.players[ctrl as usize].lands_entered += 1;
        }
        if from == Zone::Battlefield && to == Zone::Graveyard && lki.as_ref().map_or(false, |l| l.derived.types.has(CardType::Creature)) {
            self.creatures_died_turn += 1;
            if let Some(l) = &lki {
                let c = l.controller as usize;
                if self.creatures_died_by.len() <= c {
                    self.creatures_died_by.resize(c + 1, 0);
                }
                self.creatures_died_by[c] += 1;
                self.died_turn.push((id, l.controller));
                self.died_power.push(l.derived.power);
                let m = self.lki_modified(l);
                self.died_modified.push(m);
            }
        }
        // abilities granted to it stay available while its leaves-the-battlefield triggers go on the stack (undying granted by Mikaeus)
        if from == Zone::Battlefield && to != Zone::Battlefield {
            if let Some(l) = &lki {
                self.cards[id.0 as usize].cur.granted = l.derived.granted.clone();
            }
        }
        if lki.is_some() {
            self.last_lki = lki.clone();
            self.push_left_lki();
        } else if self.last_lki.as_ref().map_or(false, |l| l.card == id) {
            // the card itself moved again: its old last-known information is stale
            self.last_lki = None;
        }
        if from == Zone::Battlefield {
            if let Some(l) = &lki {
                if let Some(r) = self.revolt.get_mut(l.controller as usize) {
                    *r = true;
                }
                if !l.derived.types.has(CardType::Land) {
                    self.nonland_left_turn = true;
                }
            }
        }
        if from == Zone::Battlefield && self.batch_active {
            if let Some(l) = &lki {
                self.batch_left.push((id, l.controller));
                if l.derived.no_abilities {
                    self.batch_noabil.push(id);
                }
                for g in l.derived.granted.iter().filter(|g| matches!(&***g, crate::dsl::Ability::Replace { what: crate::dsl::Replace::TriggerExtra { .. }, .. })) {
                    self.batch_granted.push((id, g.clone()));
                }
            }
        }
        if (to == Zone::Graveyard || from == Zone::Graveyard) && self.graveyard_text_on_battlefield() {
            self.layers_dirty = true;
        }
        if to == Zone::Battlefield {
            self.cards[id.0 as usize].entered_turn = self.turn;
            self.cards[id.0 as usize].noted_from = self.exile_mv_log.len();
            self.cards[id.0 as usize].entered_from = Some(from);
            {
                let ctl = self.cards[id.0 as usize].controller;
                let d = self.def_of(id);
                let (ty, subs, ch) = (d.types, d.subtypes.to_vec(), d.keywords.has(Keyword::Changeling) && d.types.has(CardType::Creature));
                let tk = self.cards[id.0 as usize].token;
                self.entered_log.push((ctl, ty, subs, ch, id, tk));
            }
            self.layers_dirty = true;
            self.cards[id.0 as usize].damaged_by.clear();
            if let Some(h) = opts.attach_to {
                if self.cards[h.0 as usize].zone == Zone::Battlefield && h != id {
                    self.cards[id.0 as usize].attached_to = Some(h);
                    self.layers_dirty = true;
                }
            }
        }
        if let Some(old) = carry {
            let new = self.cards[id.0 as usize].zcc;
            for e in self.effects.iter_mut() {
                if matches!(e.duration, crate::dsl::Duration::UntilEndOfTurn | crate::dsl::Duration::UntilYourNextTurn | crate::dsl::Duration::Forever) {
                    for a in e.affects.iter_mut() {
                        if a.0 == id && a.1 == old {
                            a.1 = new;
                        }
                    }
                }
            }
        }
        let lki_comps = if merged_left.is_empty() { None } else { lki.clone() };
        self.lki_slot = lki;
        if to == Zone::Battlefield && self.entering_attacking.get() {
            let ctl = self.cards[id.0 as usize].cur.controller;
            self.add_attacker_late(id, ctl);
        }
        if to == Zone::Battlefield && self.entering_bare.get() {
            let z = self.cards[id.0 as usize].zcc;
            self.bare_entered.borrow_mut().push((id, z));
        }
        self.emit(Event::Moved { card: id, from, to });
        // each card of a merged permanent changes zones too: "whenever a creature dies" triggers once per card (Luminous Broodmoth)
        for comp in merged_left {
            self.lki_slot = lki_comps.clone().map(|mut l| {
                l.card = comp;
                l
            });
            self.emit(Event::Moved { card: comp, from, to });
        }
        // an Equipment that leaves the battlefield becomes unattached from its host (Grafted Wargear: sacrifice that permanent)
        if from == Zone::Battlefield && to != Zone::Battlefield {
            if let Some(h) = self.cards[id.0 as usize].left_attached_to {
                if self.cards[h.0 as usize].zone == Zone::Battlefield {
                    self.emit(Event::Unattached { card: id, from: h });
                }
            }
        }
        self.lki_slot = None;
        if from == Zone::Battlefield && !self.linked_exiles.is_empty() {
            // cards exiled "until ~ leaves the battlefield" come back
            let old_zcc = self.cards[id.0 as usize].zcc.saturating_sub(1);
            let back: Vec<(CardId, u32, Zone)> = self.linked_exiles.iter().filter(|(s, z, _, _, _)| *s == id && *z == old_zcc).map(|(_, _, c, cz, to)| (*c, *cz, *to)).collect();
            self.linked_exiles.retain(|(s, z, _, _, _)| !(*s == id && *z == old_zcc));
            for (c, cz, to) in back {
                if self.cards[c.0 as usize].zone == Zone::Exile && self.cards[c.0 as usize].zcc == cz {
                    // an Aura returning without being cast: its controller chooses what it enchants; with no legal permanent it stays exiled
                    let mut opts = MoveOpts::default();
                    let mut ask_host = false;
                    if to == Zone::Battlefield {
                        match self.aura_host_plan(c) {
                            crate::resolve::AuraHostPlan::NoHost => continue,
                            crate::resolve::AuraHostPlan::One(h) => opts.attach_to = Some(h),
                            crate::resolve::AuraHostPlan::Choose => ask_host = true,
                            crate::resolve::AuraHostPlan::NotAura => {}
                        }
                    }
                    self.move_card(c, to, opts);
                    if ask_host && self.cards[c.0 as usize].zone == Zone::Battlefield {
                        self.attach_pending.push(c);
                    }
                    if let Some(pos) = self.linked_counters.iter().position(|(lc, lz, _, _)| *lc == c && *lz == cz) {
                        let (_, _, n, ctl) = self.linked_counters.remove(pos);
                        if self.cards[c.0 as usize].zone == Zone::Battlefield && self.cards[c.0 as usize].cur.controller == ctl {
                            self.add_counters_to(Obj::card(c), CounterKind::PlusOnePlusOne, n);
                            self.layers_dirty = true;
                        }
                    }
                }
            }
        }
        true
    }

    pub fn draw_card(&mut self, p: PlayerId) -> Option<CardId> {
        if self.players[p as usize].library.is_empty() {
            self.players[p as usize].drew_from_empty = true;
            return None;
        }
        let id = self.players[p as usize].library.pop().unwrap();
        self.cards[id.0 as usize].zone = Zone::Hand;
        self.cards[id.0 as usize].zcc += 1;
        self.players[p as usize].hand.push(id);
        self.players[p as usize].cards_drawn_turn += 1;
        self.players[p as usize].drew_turn = self.turn;
        self.cards[id.0 as usize].drawn_turn = self.turn;
        self.players[p as usize].last_drawn = Some(id.0);
        self.cards[id.0 as usize].drawn_zcc = self.cards[id.0 as usize].zcc;
        self.layers_dirty = true;
        self.reset_derived(id);
        self.draw_log.borrow_mut().push(id);
        self.emit(Event::Drew { player: p, card: id });
        Some(id)
    }

    /// Draw "a card" with replacement effects applied (draw two instead, ...). `in_draw_step`: the draw step's own draw.
    /// Queue the follow-up of a draw replacement; it remembers the replacements already applied to this draw (and `src` itself when `applied`).
    pub fn push_draw_followup(&self, src: CardId, ctrl: PlayerId, effects: Vec<crate::dsl::Effect>, cyc: Option<CardId>, p: PlayerId, tgts: Vec<crate::state::Target>, applied: bool) {
        let mut chain = self.draw_chain.borrow().clone();
        if applied {
            chain.push(src);
        }
        let mut f = self.dmg_followups.borrow_mut();
        if !chain.is_empty() {
            self.followup_chains.borrow_mut().push((f.len(), chain));
        }
        f.push((src, ctrl, effects, 1, cyc, None, Some(p), tgts));
    }

    /// Where a discarded card goes: a card with madness is discarded into exile (its madness trigger then casts it or puts it into the graveyard).
    pub fn discard_zone(&self, id: CardId) -> Zone {
        if self.def_of(id).abilities.iter().any(|a| matches!(a, crate::dsl::Ability::Madness { .. })) {
            Zone::Exile
        } else {
            Zone::Graveyard
        }
    }

    pub fn draw_with_replacement(&mut self, p: PlayerId, in_draw_step: bool, plain: bool) {
        let first = in_draw_step && self.players[p as usize].cards_drawn_turn == 0;
        if let Some((src, ctrl, effects, tgts, cyc)) = if plain && self.draw_chain.borrow().is_empty() { None } else { self.draw_instead(p, first, 1) } {
            self.push_draw_followup(src, ctrl, effects, cyc, p, tgts, true);
            self.drain_consumed_replaces();
            return;
        }
        let n = self.draw_count(p, first);
        self.drain_consumed_replaces();
        for _ in 0..n {
            self.draw_card(p);
        }
    }

    pub fn shuffle_library(&mut self, p: PlayerId) {
        let mut lib = std::mem::take(&mut self.players[p as usize].library);
        lib.shuffle(&mut self.rng);
        self.players[p as usize].library = lib;
        self.emit(Event::Shuffled { player: p });
    }

    /// "Enters with N counters" abilities (evaluated once any as-enters choice has been made).
    /// The additional +1/+1 counters a permanent spell enters with because of mana-rider mana spent on it.
    fn apply_rider_counters(&mut self, id: CardId) {
        if let Some(kind) = self.cards[id.0 as usize].enter_counter_rider.take() {
            self.cards[id.0 as usize].add_counters(kind, 1);
            self.layers_dirty = true;
            self.emit(Event::Counters { card: id, kind, delta: 1, by: self.cause_ctl.get() });
        }
        if std::mem::take(&mut self.cards[id.0 as usize].finality_rider) {
            self.cards[id.0 as usize].add_counters(CounterKind::Finality, 1);
            self.layers_dirty = true;
            self.emit(Event::Counters { card: id, kind: CounterKind::Finality, delta: 1, by: self.cause_ctl.get() });
        }
        if std::mem::take(&mut self.cards[id.0 as usize].riot_rider) && self.cards[id.0 as usize].zone == Zone::Battlefield {
            let ctl = self.cards[id.0 as usize].cur.controller;
            let riot: crate::dsl::Effect = serde_json::from_str(r#"{"type":"Unless","payer":{"type":"You"},"cost":[{"type":"PutCounters","kind":"PlusOnePlusOne","n":1}],"effects":[{"type":"GrantKeyword","what":{"type":"This"},"keyword":"Haste","duration":"Forever"}]}"#).expect("riot effect");
            self.dmg_followups.borrow_mut().push((id, ctl, vec![riot], 0, Some(id), None, Some(ctl), vec![]));
        }
        if std::mem::take(&mut self.cards[id.0 as usize].cast_haste) && self.cards[id.0 as usize].zone == Zone::Battlefield {
            let ctl = self.cards[id.0 as usize].cur.controller;
            self.add_effect(id, ctl, &[id], Modif::Keyword(Keyword::Haste), crate::dsl::Duration::UntilEndOfTurn);
        }
        if std::mem::take(&mut self.cards[id.0 as usize].evoke_rider) && self.cards[id.0 as usize].zone == Zone::Battlefield {
            let ctl = self.cards[id.0 as usize].cur.controller;
            let sac: crate::dsl::Effect = serde_json::from_str(r#"{"type":"Sacrifice","who":{"type":"You"},"filter":{"this":true},"n":{"type":"Fixed","n":1}}"#).expect("evoke sacrifice");
            self.dmg_followups.borrow_mut().push((id, ctl, vec![sac], 0, Some(id), None, Some(ctl), vec![]));
        }
        if std::mem::take(&mut self.cards[id.0 as usize].sneak_rider) && self.cards[id.0 as usize].zone == Zone::Battlefield {
            let ctl = self.cards[id.0 as usize].cur.controller;
            let atk: crate::dsl::Effect = serde_json::from_str(r#"{"type":"BecomeAttacking","what":{"type":"This"}}"#).expect("sneak attack");
            self.dmg_followups.borrow_mut().push((id, ctl, vec![atk], 0, Some(id), None, Some(ctl), vec![]));
        }
        if std::mem::take(&mut self.cards[id.0 as usize].blitz_rider) && self.cards[id.0 as usize].zone == Zone::Battlefield {
            let ctl = self.cards[id.0 as usize].cur.controller;
            let draw: crate::dsl::Ability = serde_json::from_str(r#"{"type":"Triggered","trigger":{"type":"Dies","filter":{"this":true}},"effects":[{"type":"Draw","who":{"type":"You"},"n":{"type":"Fixed","n":1}}]}"#).expect("blitz draw ability");
            let eff = crate::dsl::StaticEffect::GrantAbility { affects: (crate::dsl::Filter { this: true, ..Default::default() }).into(), ability: Box::new(draw) };
            self.add_effect(id, ctl, &[id], Modif::Static(Box::new(eff)), crate::dsl::Duration::Forever);
            self.layers_dirty = true;
        }
        if std::mem::take(&mut self.cards[id.0 as usize].haste_sac_rider) && self.cards[id.0 as usize].zone == Zone::Battlefield {
            self.cards[id.0 as usize].free_haste = true;
            self.layers_dirty = true;
            let ctl = self.cards[id.0 as usize].controller;
            let did = self.next_effect_id;
            self.next_effect_id += 1;
            self.delayed.push(DelayedTrigger {
                id: did,
                controller: ctl,
                source: id,
                when: crate::dsl::DelayedWhen::NextEndStep,
                effects: vec![crate::dsl::Effect::Sacrifice { who: crate::dsl::Sel::Remembered, filter: Default::default(), n: crate::dsl::Amount::Fixed { n: 1 } }],
                created_turn: self.turn,
                targets: vec![],
                ctx: EventCtx::default(),
                remembered: vec![Obj::card(id)],
            });
        }
        if std::mem::take(&mut self.cards[id.0 as usize].warp_rider) && self.cards[id.0 as usize].zone == Zone::Battlefield {
            let ctl = self.cards[id.0 as usize].cur.controller;
            let did = self.next_effect_id;
            self.next_effect_id += 1;
            self.delayed.push(DelayedTrigger {
                id: did,
                controller: ctl,
                source: id,
                when: crate::dsl::DelayedWhen::NextEndStep,
                effects: vec![
                    crate::dsl::Effect::Move { what: crate::dsl::Sel::This, to: Zone::Exile, top_of_library: false, tapped: false, under_your_control: false, remember: false, transformed: false, from: Some(Zone::Battlefield), discard: false, position: None, position_x: false, ordered: false, bottom_ordered: false, order_chooser: None, attacking: false, choose_host: false, host_creature: false, host_only: None, host_chooser: None, non_aura_if_no_host: false, face_down: false, under_control_of: None, random_order: false, enters_bare: false },
                    crate::dsl::Effect::MarkExiled { mark: crate::dsl::ExileMark::Warped },
                ],
                created_turn: self.turn,
                targets: vec![],
                ctx: EventCtx::default(),
                remembered: vec![],
            });
        }
        let gain = std::mem::take(&mut self.cards[id.0 as usize].play_leave_gain);
        if gain == u8::MAX {
            // MayPlayFrom{leave_exile}: exiled instead of leaving the battlefield
            self.cards[id.0 as usize].exile_on_leave = true;
        } else if gain > 0 {
            let ab: crate::dsl::Ability = serde_json::from_value(serde_json::json!({
                "type": "Triggered",
                "trigger": { "type": "ChangesZone", "filter": { "this": true }, "from": "Battlefield", "to": "Graveyard" },
                "effects": [
                    { "type": "Move", "what": { "type": "This" }, "to": "Exile" },
                    { "type": "GainLife", "who": { "type": "You" }, "n": { "type": "Fixed", "n": gain } }
                ]
            })).expect("leave_exile_gain trigger");
            let ctl = self.cards[id.0 as usize].cur.controller;
            self.add_effect(id, ctl, &[id], Modif::Static(Box::new(crate::dsl::StaticEffect::GrantAbility { affects: (crate::dsl::Filter::default()).into(), ability: Box::new(ab) })), crate::dsl::Duration::Forever);
        }
        let charge = std::mem::take(&mut self.cards[id.0 as usize].rider_charge);
        if charge > 0 {
            self.cards[id.0 as usize].add_counters(CounterKind::Charge, charge as i32);
            self.layers_dirty = true;
            self.emit(Event::Counters { card: id, kind: CounterKind::Charge, delta: charge as i32, by: self.cause_ctl.get() });
        }
        let loy = std::mem::take(&mut self.cards[id.0 as usize].rider_loyalty);
        if loy > 0 {
            self.cards[id.0 as usize].add_counters(CounterKind::Loyalty, loy as i32);
            self.layers_dirty = true;
            self.emit(Event::Counters { card: id, kind: CounterKind::Loyalty, delta: loy as i32, by: self.cause_ctl.get() });
        }
        let extra = std::mem::take(&mut self.cards[id.0 as usize].rider_counters);
        if extra > 0 {
            self.cards[id.0 as usize].add_counters(CounterKind::PlusOnePlusOne, extra as i32);
            self.layers_dirty = true;
            self.emit(Event::Counters { card: id, kind: CounterKind::PlusOnePlusOne, delta: extra as i32, by: self.cause_ctl.get() });
        }
    }

    pub(crate) fn apply_enter_counters(&mut self, id: CardId) {
        self.apply_rider_counters(id);
        let db = self.db.clone();
        let d = db.def(self.cards[id.0 as usize].def);
        if d.abilities.iter().any(|a| matches!(a, crate::dsl::Ability::EntersRemoveAllCounters)) {
            self.counters_removed.set(0);
            let ids: Vec<CardId> = self.battlefield.iter().copied().filter(|x| *x != id).collect();
            for x in ids {
                let have: Vec<(CounterKind, u16)> = self.cards[x.0 as usize].counters.iter().copied().collect();
                for (k, n) in have {
                    self.remove_counters_from(Obj::card(x), k, n as i32);
                }
            }
        }
        for a in &d.abilities {
            if let crate::dsl::Ability::EntersWithCounters { kind, n, condition } = a {
                if let Some(cond) = condition {
                    if !self.eval_cond(cond, &EvalCtx::of(self, id)) {
                        continue;
                    }
                }
                let mut ctx = EvalCtx::of(self, id);
                ctx.x = self.cards[id.0 as usize].x;
                let cast_rem = std::mem::take(&mut self.cards[id.0 as usize].cast_remembered);
                ctx.remembered = &cast_rem;
                let mut k = self.eval_amount(n, &ctx);
                // entering from a graveyard, "the number of creature cards in your graveyard" still counts the card itself (Rhizome Lurcher ruling)
                if self.cards[id.0 as usize].entered_from == Some(Zone::Graveyard) {
                    if let crate::dsl::Amount::Count { filter, zone: Some(Zone::Graveyard), .. } = n {
                        self.recompute_layers();
                        if self.matches_card(filter, id, id, self.cards[id.0 as usize].cur.controller) {
                            k += 1;
                        }
                    }
                }
                let n = k;
                self.cards[id.0 as usize].cast_remembered = cast_rem.clone();
                let n = self.replace_counters_entering(id, *kind, n);
                self.cards[id.0 as usize].add_counters(*kind, n);
                self.layers_dirty = true;
                if n > 0 {
                    let by = self.cause_ctl.get().or(Some(self.cards[id.0 as usize].cur.controller));
                    self.emit(Event::Counters { card: id, kind: *kind, delta: n, by });
                }
            }
        }
    }

    /// "As this enters or is turned face up, choose ...": a permanent just turned face up makes its as-enters choices now (Aquamorph Entity).
    pub(crate) fn queue_face_up_choice(&mut self, id: CardId) {
        if self.db.def(self.cards[id.0 as usize].def).abilities.iter().any(|a| matches!(a, crate::dsl::Ability::EntersChoose { .. })) && !self.enter_choices.contains(&id) {
            self.cards[id.0 as usize].chosen_type = None;
            self.cards[id.0 as usize].chosen_type2 = None;
            self.cards[id.0 as usize].player_pick.clear();
            self.cards[id.0 as usize].enter_done = 0;
            self.enter_choices.push(id);
        }
        let ab = &self.db.def(self.cards[id.0 as usize].def).abilities;
        let copy = ab.iter().any(|a| matches!(a, crate::dsl::Ability::EntersAsCopy { until_turned_face_down: true, .. }));
        let attach = ab.iter().any(|a| matches!(a, crate::dsl::Ability::TurnedFaceUpMayAttach { .. }));
        if (copy || attach) && !self.enter_choices.contains(&id) {
            let c = &mut self.cards[id.0 as usize];
            c.enter_done &= !(4 | 1024);
            c.turnup_choice = true;
            self.enter_choices.push(id);
        }
    }

    /// Would a battlefield `RemoveAbilities` static (Blood Sun) strip all abilities from `id` as it enters?
    fn enters_stripped_of_abilities(&self, id: CardId) -> bool {
        self.battlefield.iter().any(|&src| {
            let sc = &self.cards[src.0 as usize];
            if sc.cur.no_abilities || src == id {
                return false;
            }
            let sd = self.db.def(sc.def);
            sd.statics.iter().any(|&i| match &sd.abilities[i as usize] {
                crate::dsl::Ability::Static { effect: crate::dsl::StaticEffect::RemoveAbilities { affects, .. }, condition: None, .. } => self.matches_card(&affects.clone().into_filter(), id, src, sc.cur.controller),
                _ => false,
            })
        })
    }

    fn apply_enters_replacements(&mut self, id: CardId) {
        self.enters_as_copy_by_replacement(id);
        let def_id = self.cards[id.0 as usize].def;
        let db = self.db.clone();
        let d = db.def(def_id);
        if self.enters_tapped_by_replacement(id) || std::mem::take(&mut self.cards[id.0 as usize].enter_tapped_once) {
            self.cards[id.0 as usize].tapped = true;
        }
        self.enters_counters_by_replacement(id);
        self.enters_as_creature_by_replacement(id);
        let mut counters_done = false;
        // Blood Sun: a permanent that would lose all its abilities as it enters doesn't have its own "enters tapped" abilities (614.12)
        let stripped = (self.cards[id.0 as usize].cur.types.has(CardType::Land) && self.enters_stripped_of_abilities(id)) || self.cards[id.0 as usize].face_down;
        // static abilities in effect (Barbara Wright grants EntersChoose to Sagas) are part of what this permanent does as it enters.
        // Recomputing layers puts the entrant into the battlefield state, so a permanent whose own counters are counted on entry
        // (Ambitious Dragonborn: X = greatest power) keeps the pre-entry state and is left alone.
        if !d.abilities.iter().any(|a| matches!(a, crate::dsl::Ability::EntersWithCounters { .. })) {
            self.recompute_layers();
        }
        let abils = self.enter_abilities(id, d);
        for a in abils.iter() {
            if stripped && matches!(a, crate::dsl::Ability::EntersTapped { .. } | crate::dsl::Ability::EntersRevealOrTapped { .. } | crate::dsl::Ability::EntersPayLifeOrTapped { .. } | crate::dsl::Ability::EntersMayTapped { .. }) {
                continue;
            }
            match a {
                crate::dsl::Ability::EntersTapped { unless } => {
                    // permanents entering together don't count each other ("unless you control two or more other lands": Ghostly Flicker on two lands)
                    let hidden: Vec<CardId> = self.entering_batch.iter().copied().filter(|b| *b != id && self.battlefield.contains(b)).collect();
                    if !hidden.is_empty() {
                        self.battlefield.retain(|b| !hidden.contains(b));
                    }
                    let skip = unless.as_ref().map_or(false, |c| self.eval_cond(c, &EvalCtx::of(self, id)));
                    self.battlefield.extend(hidden);
                    if !skip {
                        self.cards[id.0 as usize].tapped = true;
                    }
                }
                // a face-down permanent has no abilities: the choice is made when it is turned face up (`queue_face_up_choice`)
                crate::dsl::Ability::EntersChoose { .. } | crate::dsl::Ability::EntersAsCopy { .. } if self.cards[id.0 as usize].face_down => {}
                crate::dsl::Ability::EntersChoose { .. } | crate::dsl::Ability::EntersAsCopy { .. } | crate::dsl::Ability::EntersUnlessCost { .. } | crate::dsl::Ability::EntersExileXCounters { .. } | crate::dsl::Ability::EntersExileAny { .. } | crate::dsl::Ability::EntersSacrificeAll { .. } => {
                    if matches!(a, crate::dsl::Ability::EntersExileAny { .. }) && !self.enter_choices.contains(&id) {
                        self.cards[id.0 as usize].enter_exiled_pt = (0, 0);
                    }
                    self.cards[id.0 as usize].enter_done = if self.enter_choices.contains(&id) { self.cards[id.0 as usize].enter_done } else { 0 };
                    if !self.enter_choices.contains(&id) {
                        self.enter_choices.push(id);
                    }
                }
                crate::dsl::Ability::EntersRevealOrTapped { .. } | crate::dsl::Ability::EntersPayLifeOrTapped { .. } => {
                    self.cards[id.0 as usize].tapped = true;
                    self.cards[id.0 as usize].enter_done = if self.enter_choices.contains(&id) { self.cards[id.0 as usize].enter_done } else { 0 };
                    if !self.enter_choices.contains(&id) {
                        self.enter_choices.push(id);
                    }
                }
                crate::dsl::Ability::EntersMayTapped { .. } | crate::dsl::Ability::EntersCounterOnChosen { .. } | crate::dsl::Ability::EntersLinkChosen { .. } => {
                    self.cards[id.0 as usize].enter_done = if self.enter_choices.contains(&id) { self.cards[id.0 as usize].enter_done } else { 0 };
                    if !self.enter_choices.contains(&id) {
                        self.enter_choices.push(id);
                    }
                }
                crate::dsl::Ability::EntersWithCounters { .. } => {
                    // counters that depend on an as-enters choice (Banner of Kinship) wait for the choice
                    if !counters_done && !abils.iter().any(|x| matches!(x, crate::dsl::Ability::EntersChoose { .. })) {
                        counters_done = true;
                        self.apply_enter_counters(id);
                    }
                }
                _ => {}
            }
        }
        if !abils.iter().any(|a| matches!(a, crate::dsl::Ability::EntersWithCounters { .. })) {
            self.apply_rider_counters(id);
        }
        if self.enters_untapped_by_replacement(id) {
            self.cards[id.0 as usize].tapped = false;
        }
        let read_ahead = abils.iter().any(|a| matches!(a, crate::dsl::Ability::EntersChoose { kind: crate::dsl::ChooseKind::SagaChapter, .. }));
        if d.saga_max > 0 && !read_ahead {
            self.cards[id.0 as usize].add_counters(CounterKind::Lore, 1);
            self.emit(Event::Counters { card: id, kind: CounterKind::Lore, delta: 1, by: self.cause_ctl.get() });
        }
        // planeswalkers and battles enter with loyalty / defense counters
        if let Some(l) = d.loyalty {
            if d.types.has(CardType::Planeswalker) {
                let compleated = abils.iter().any(|a| matches!(a, crate::dsl::Ability::Compleated));
                let l = if compleated { (l - 2 * self.cards[id.0 as usize].phyrexian_life as i32).max(0) } else { l };
                let l = self.replace_counters_entering(id, CounterKind::Loyalty, l);
                self.cards[id.0 as usize].add_counters(CounterKind::Loyalty, l);
                if l > 0 {
                    self.emit(Event::Counters { card: id, kind: CounterKind::Loyalty, delta: l, by: self.cause_ctl.get() });
                }
                self.cards[id.0 as usize].phyrexian_life = 0;
            }
        }
        if let Some(df) = d.defense {
            if d.types.has(CardType::Battle) {
                let df = self.replace_counters_entering(id, CounterKind::Defense, df);
                self.cards[id.0 as usize].add_counters(CounterKind::Defense, df);
                // the protector is an opponent of the controller (the next one in turn order)
                let ctl = self.cards[id.0 as usize].cur.controller;
                let prot = self.opponents(ctl).into_iter().find(|q| self.alive(*q));
                self.cards[id.0 as usize].protector = prot;
            }
        }
    }
}

#[derive(Clone, Copy, Default, Debug)]
pub struct MoveOpts {
    pub tapped: bool,
    pub face_down: bool,
    pub bottom: bool,
    pub controller: Option<PlayerId>,
    /// An Aura / Equipment entering already attached to this permanent (so its enters triggers see the attachment).
    pub attach_to: Option<CardId>,
}


impl Game {
    /// Counters a permanent enters with: the player who puts them is its controller (Vorinclex's "if you would put" applies).
    pub(crate) fn replace_counters_entering(&self, id: CardId, kind: CounterKind, n: i32) -> i32 {
        let ctl = self.cards[id.0 as usize].cur.controller;
        let prev = self.cause_ctl.replace(self.cause_ctl.get().or(Some(ctl)));
        let r = self.replace_counters(id, kind, n);
        self.cause_ctl.set(prev);
        r
    }
}
