//! Creating a game.

use crate::carddb::CardDb;
use crate::decision::*;
use crate::ids::*;
use crate::state::*;
use crate::types::*;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use smallvec::SmallVec;
use std::sync::Arc;

impl Game {
    /// Set up a game: shuffled libraries, commanders in the command zone, opening hands drawn. Call `run()` to start.
    pub fn new(db: Arc<CardDb>, cfg: Config, decks: Vec<DeckSpec>, seed: u64) -> Game {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let first = rng.gen_range(0..decks.len().max(1)) as PlayerId;
        let players: Vec<Player> = decks
            .iter()
            .enumerate()
            .map(|(i, d)| Player {
                id: i as PlayerId,
                name: d.name.clone(),
                life: cfg.starting_life,
                library: vec![],
                hand: vec![],
                graveyard: vec![],
                exile: vec![],
                command: vec![],
                scheme_deck: vec![],
                team: i as u8,
                planar_rolls: 0,
                sideboard: vec![],
                pool: Default::default(),
                pool_keep: Default::default(),
                pool_turn: Default::default(),
                poison: 0,
                energy: 0,
                energy_spent: 0,
                experience: 0,
                rad: 0,
                lands_played: 0,
                land_limit: 1,
                spells_cast: 0,
                spells_cast_last: 0,
                spells_cast_game: 0,
                face_down_cast_turn: 0,
                face_down_act_turn: 0,
                nonhand_turn: 0,
                life_gained: 0,
                life_gain_events: 0,
                life_lost: 0,
                life_lost_last: 0,
                life_turn_start: cfg.starting_life,
                starting_deck: 0,
                any_type_turn: 0,
                cascade_turn: 0,
                hand_cascade_turn: 0,
                damaged: false,
                bend_mask: 0,
                bend_turn: 0,
            damage_taken_turn: 0,
            discarded_turn: 0,
            cycled_turn: 0,
            combat_dmg_turn: 0,
            spell_cast_turn: 0,
            cur_turn: 0,
            prev_turn: 0,
                crime: false,
                speed: 0,
                city_blessing: false,
                enduring_story: false,
                dungeon: None,
                dungeons_completed: 0,
                foreign_lands_turn: 0,
                life_mana: None,
                pregame_color: None,
                completed_dungeon_names: Vec::new(),
                ring_level: 0,
                ring_bearer: None,
                ring_emblem: None,
                mana_spent_turn: 0,
                restricted: vec![],
                pool_treasure: false,
                shield_bar: false,
                shield_tax: 0,
                restricted_turn: vec![],
                speed_up: false,
                drew_from_empty: false,
                lost: false,
                conceded: false,
                mulligans: 0,
                kept: false,
                scry_bottomed: 0,
                powder_declined: false,
                luck_declined: false,
                max_hand: 7,
                commander_damage: SmallVec::new(),
                commander_cast: SmallVec::new(),
                auto_skip: true,
                allow_cancel: false,
                manual_pay: false,
                manual_once: false,
                stops_mine: Step::stop_bit(Step::Main1) | Step::stop_bit(Step::Main2),
                stops_opp: 0,
                full_control: false,
                offer_mana: false,
                pass_until: None,
                skip_turns: 0,
                skip_draw: 0,
                skip_untap: 0,
                skip_combat: 0,
                skip_combat_after: 0,
                skip_phases: (0, 0),
                cards_drawn_turn: 0,
                venture_turn: (0, 0),
                last_drawn: None,
                act_turn: 0,
                drew_turn: 0,
                attacked_turn: false,
                searched_turn: false,
                scried_turn: false,
                dilemmas_turn: 0,
            began_extra_turn: false,
                loyalty_turn: false,
                exhaust_turn: false,
                equip_turn: false,
                investigated_turn: 0,
                loyalty_all_double_turn: 0,
                loyalty_instant_turn: 0,
                coin_flip_turn: 0,
                coin_flip_serial: 0,
                loyalty_instant_sub: None,
                lands_no_untap: false,
                creatures_no_untap: false,
                damage_shield: false,
                surveilled_turn: false,
                surveil_count_turn: 0,
                last_turn_start: 0,
                prev_turn_start: 0,
                hand_turn_start: 0,
                lands_untapped_turn_start: 0,
                turns_taken: 0,
                lands_entered: 0,
                creatures_entered: 0,
                creatures_entered_last: 0,
                attacked_last_turn: 0,
                combat_serial: 0,
                contraptions: 0,
            })
            .collect();
        let mut g = Game {
            db,
            cfg: cfg.clone(),
            players,
            cards: vec![],
            battlefield: vec![],
            stack: vec![],
            stack_lki: vec![],
            stack_lki_x: vec![],
            stack_lki_items: vec![],
            stack_lki_src: vec![],
            retarget_must_change: std::cell::Cell::new(false),
            retarget_random: std::cell::Cell::new(false),
            retarget_exclude: std::cell::Cell::new(false),
            retarget_only: std::cell::Cell::new(None),
            retarget_forced: std::cell::Cell::new(false),
            retarget_queue: std::cell::RefCell::new(vec![]),
            last_reveal_count: std::cell::Cell::new(0),
            costing_card: std::cell::Cell::new(None),
            retarget_players_only: std::cell::Cell::new(false),
            retarget_filter: std::cell::RefCell::new(None),
            turn: 0,
            active: 0,
            step: Step::Untap,
            extra_turns: vec![],
            extra_lose_marks: vec![],
            lose_end_turn: None,
            regular_active: 0,
            in_extra_turn: false,
            rng,
            ts: 0,
            next_stack_id: 0,
            next_effect_id: 0,
            effects: vec![],
            delayed: vec![],
            pending_triggers: vec![],
            zone_sub_dirty: false,
            temp_triggers: vec![],
            combat_damagers: vec![],
            combat_damagers_ctl: vec![],
            adapt_ignore: vec![],
            no_attack_pairs: vec![],
            no_attack_pw_pairs: vec![],
            no_attack_perms: vec![],
            no_attack_next: vec![],
            attack_nearest: vec![],
            no_attack_pending: vec![],
            sanctuary: vec![],
            draw_skip_declined: false,
            untap_pick_done: false,
            untap_limit_done: false,
            untap_prefer: None,
            untap_only: None,
            taunts: vec![],
            attack_all: vec![],
            next_turn_ctl: vec![],
            turn_ctl: None,
            turn_ctl_combat: false,
            resolve_ctl: None,
            land_mana_only: None,
            text_ask: None,
            spell_ctl: vec![],
            resolve_ctl_item: None,
            wok_pending: None,
            remapped_at: None,
            effect_ignored: vec![],
            no_attack_until: 0,
            no_target_until: 0,
            target_trig: std::cell::Cell::new(false),
            ignored_until_eot: vec![],
            block_chooser: vec![],
            attack_chooser: None,
            dmg_log: vec![],
            noncombat_dmg_last: vec![],
            dmg_game: vec![],
            ability_res: Default::default(),
            cur_ability_res: 0,
            combat: None,
            run: Run::Over,
            pending: None,
            answer: None,
            events: vec![],
            over: None,
            layers_dirty: true,
            layer_started: std::cell::RefCell::new(vec![]),
            monarch: None,
            initiative: None,
            day_night: None,
            no_prevent_turn: 0,
            attack_bands: vec![],
            nonland_left_turn: false,
            warped_this_turn: false,
            alt_grants_used: vec![],
            last_die: 0,
            planar_deck: vec![],
            planes: vec![],
            chaotic_aether: false,
            no_enter_from_exile_turn: 0,
            fixed_point_until: vec![],
            no_attack_until_planeswalk: vec![],
            safe_until_planeswalk: vec![],
            scheme_this_turn: false,
            scheme_ban_next: false,
            scheme_triple: vec![],
            planeswalked_to_this_turn: vec![],
            archenemy: false,
            roll_chosen: 0,
            roll_other: 0,
            face_down_cast: std::cell::Cell::new(None),
            cost_probe: std::cell::Cell::new(None),
            entering_attacking: std::cell::Cell::new(false),
            entering_bare: std::cell::Cell::new(false),
            bare_entered: std::cell::RefCell::new(Vec::new()),
            repl_exile_src: std::cell::Cell::new(None),
            crew_log: vec![],
            tap_cost_log: vec![],
            resolving_spell: std::cell::Cell::new(None),
            exec_spell: std::cell::RefCell::new(None),
            resolved_ok: std::cell::Cell::new(false),
            last_tapped_mana: vec![],
            last_tapped_mana_n: 0,
            last_flip_won: false,
            flip_wins: 0,
            flip_lost: false,
            last_cost_paid: false,
            no_regen_dealers: vec![],
            life_rounds: vec![],
            flip_count: 0,
            pay_tally: vec![],
            colorless_all: false,
            color_all: 0,
            artifact_creatures: 0,
            white_nonland: 0,
            cause_trigger: std::cell::Cell::new(false),
            first_player: first,
            restart_pending: None,
            restart_return: vec![],
            lands_entered_turn: 0,
            entered_log: vec![],
            attackers_log: vec![],
            token_attackers_log: vec![],
            commander_attackers_log: vec![],
            defended_log: vec![],
            counters_log: vec![],
            creatures_died_turn: 0,
            pile_guess_turn: 0,
            each_current: vec![],
            hidden_picks: vec![],
            player_choices: vec![],
            initiative_emblem: None,
            last_votes: vec![],
            last_card_votes: vec![],
            player_numbers: vec![],
            vote_control: vec![],
            last_ballots: vec![],
            creatures_died_by: vec![],
            left_graveyard_by: vec![],
            left_graveyard_creature_by: vec![],
            delayed_rem_zcc: vec![],
            gy_entries: vec![],
            exile_entries: vec![],
            exile_mv_log: vec![],
            combat_gy_start: 0,
            sacrificed_turn: vec![],
            land_mana_turn: 0,
            nc_destroyed_by_opp: 0,
            sacrificed_types: vec![],
            tokens_made_by: vec![],
            treasures_made_by: vec![],
            bounced_turn: vec![],
            left_bf_turn: vec![],
            left_bf_power: vec![],
            left_bf_kw: vec![],
            token_gy_turn: vec![],
            oil_died_turn: false,
            died_turn: vec![],
            died_power: vec![],
            died_modified: vec![],
            combats_this_turn: 0,
            attacked_players_turn: vec![],
            block_log: vec![],
            attacked_player_log: vec![],
            spells_this_turn: 0,
            spells_log: vec![],
            poison_capped: std::cell::Cell::new(0),
            graveyard_use_log: vec![],
            gy_instant_sorcery_log: vec![],
            foretold_log: vec![],
            artifact_act_log: vec![],
            creature_tgt_act_log: vec![],
            power_up_act_log: vec![],
            spells_cast_names: vec![],
            decision_log: vec![],
            lki_slot: None,
            craft_exile: std::cell::Cell::new(false),
            last_lki: None,
            left_lki: Vec::new(),
            batch_seen: vec![],
            batch_seen_pw: vec![],
            cost_sacrificed: vec![],
            mana_gift: None,
            mana_picks: Vec::new(),
            cost_milled: vec![],
            unless_spent: [0; 6],
            track_pay_spent: false,
            zone_batch: vec![],
            linked_exiles: vec![],
            linked_counters: vec![],
            manual_linked: vec![],
            monarch_exiles: vec![],
            batch_active: false,
            copy_resolving: std::cell::Cell::new(false),
            pay_for: std::cell::Cell::new(None),
            pay_alt_probe: std::cell::Cell::new(None),
            pay_cost_has_c: std::cell::Cell::new(false),
            sac_reserve: std::cell::RefCell::new(vec![]),
            spent_track: std::cell::Cell::new([0; 6]),
            snow_track: std::cell::Cell::new(0),
            snow_colors_track: std::cell::Cell::new(0),
            src_track: std::cell::Cell::new([0; 5]),
            src_cards_track: std::cell::RefCell::new(vec![]),
            force_unprevent: std::cell::Cell::new(false),
            skip_opt_redirect: std::cell::Cell::new(false),
            treasure_paid: std::cell::Cell::new(false),
            pay_is_ability: std::cell::Cell::new(false),
            pay_upkeep: std::cell::Cell::new(false),
            pay_unlock: std::cell::Cell::new(false),
            pay_is_equip: std::cell::Cell::new(false),
            pay_is_power_up: std::cell::Cell::new(false),
            pay_is_foretell: std::cell::Cell::new(false),
            pay_ability_has_x: std::cell::Cell::new(false),
            paying_activation: std::cell::Cell::new(false),
            pay_riders: Default::default(),
            pay_rider_srcs: Default::default(),
            pay_rider_units: Default::default(),
            copy_rider_always: std::cell::Cell::new(false),
            assume_target_tax: std::cell::Cell::new(false),
            countered_by: vec![],
            spells_countered: vec![],
            cast_hint: std::cell::Cell::new(None),
            last_transform: Default::default(),
            last_transform_stack: Default::default(),
            goaded: vec![],
            goad_while_src: vec![],
            no_counters_src: vec![],
            extra_combat_now: 0,
            extra_upkeep: 0,
            upkeep_return: None,
            extra_beginning: 0,
            in_extra_beginning: false,
            extra_beginning_combat: 0,
            extra_beginning_return: None,
            upkeeps_this_turn: 0,
            extra_end: 0,
            end_steps_this_turn: 0,
            monarch_turn_start: None,
            extra_combat_after_main: 0,
            extra_main_pending: 0,
            extra_main_repeat: 0,
            copy_reverts: vec![],
            copy_reverts_turn: vec![],
            copy_while_source: vec![],
            free_casts: vec![],
            cast_after: vec![],
            counter_offer: None,
            encoded: vec![],
            meld_parts: vec![],
            exile_marked: vec![],
            free_cast_now: std::cell::Cell::new(false),
            x_red: std::cell::Cell::new(0),
            red_overflow: std::cell::Cell::new(0),
            color_left: Default::default(),
            filter_rem: std::cell::RefCell::new(vec![]),
            reflex_used: std::cell::RefCell::new(vec![]),
            filter_targets: std::cell::RefCell::new((vec![], vec![])),
            filter_event: std::cell::RefCell::new(None),
            divide_any: std::cell::Cell::new(false),
            prevent_batch: std::cell::RefCell::new(None),
            dmg_followups: std::cell::RefCell::new(vec![]),
            dmg_combat_now: std::cell::Cell::new(false),
            mana_x_hint: std::cell::Cell::new(None),
            mana_color_hints: Default::default(),
            mana_produce_hint: std::cell::Cell::new(None),
            redirect_pick: std::cell::Cell::new(None),
            draw_chain: std::cell::RefCell::new(vec![]),
            followup_chains: std::cell::RefCell::new(vec![]),
            draw_log: std::cell::RefCell::new(vec![]),
            cause_destroy: std::cell::Cell::new(false),
            last_sacrificed: std::cell::Cell::new(None),
            emerge_sac: std::cell::Cell::new(false),
            attach_pending: vec![],
            fs_dealers: vec![],
            sac_max_power: std::cell::Cell::new(0),
            divide_prevent: std::cell::Cell::new(false),
            divide_cleanup: std::cell::Cell::new(false),
            divide_end_step: std::cell::Cell::new(false),
            cleanup_counter_removals: vec![],
            counters_removed: std::cell::Cell::new(0),
            kind_pick_min1: std::cell::Cell::new(false),
            dmg_by_source: std::cell::RefCell::new(std::collections::HashMap::new()),
            revolt: vec![false; 8],
            descended: vec![false; 8],
            shields: vec![],
            granted_only: false,
            phased: vec![],
            cur_src: None,
            end_turn_pending: false,
            end_combat_pending: false,
            cause_ctl: std::cell::Cell::new(None),
            cycling_move: std::cell::Cell::new(false),
            draw_cycled: std::cell::Cell::new(None),
            cause_src: std::cell::Cell::new(None),
            cause_targets: std::cell::RefCell::new(Vec::new()),
            declaring_attacker: std::cell::Cell::new(false),
            choose_accumulate: std::cell::Cell::new(false),
            forced_rolls: std::cell::RefCell::new(vec![]),
            stored_sets: std::cell::RefCell::new(Default::default()),
            pinned_sets: vec![],
            pinned_next: 0,
            text_swaps: vec![],
            sector_block_turn: 0,
            abort_at: None,
            mana_pick: None,
            temp_replaces: vec![],
            long_replaces: vec![],
            zone_granted: vec![],
            turn_play_grants: vec![],
            block_limits: vec![],
            pile_blocks_turn: 0,
            pile_state: None,
            divide_from: std::cell::Cell::new(None),
            forage_grants: vec![],
            gy_play_grants: vec![],
            lib_play_grants: vec![],
            lib_free_grants: vec![],
            mana_drained_by: vec![],
            hand_play_grants: vec![],
            outside_grants: vec![],
            outside_card_grants: vec![],
            gy_exile_grants: vec![],
            gy_free_grants: vec![],
            gy_card_grants: vec![],
            gy_card_riders: vec![],
            gy_once_grants: vec![],
            coffin_notes: vec![],
            meddled: vec![],
            turn_reversed: false,
            modular_cause: std::cell::Cell::new(false),
            counters_as_cost: std::cell::Cell::new(false),
            teamwork_tap: std::cell::Cell::new(false),
            consumed_temp: std::cell::RefCell::new(vec![]),
            last_damage: std::cell::Cell::new((0, 0)),
            last_life_lost: std::cell::Cell::new(0),
            last_surveil_kept: std::cell::Cell::new(0),
            reduced_temp: std::cell::RefCell::new(vec![]),
            redirecting: 0,
            flip_serial: 0,
            combat_batch: false,
            life_batch: vec![],
            batch_once_used: vec![],
            player_prot: vec![],
            batch_left: vec![],
            batch_gy_events: vec![],
            gy_replay: false,
            batch_noabil: vec![],
            entering_batch: vec![],
            defer_events: None,
            batch_granted: vec![],
            sba_waiting: false,
            enter_choices: vec![],
            copy_deferred: vec![],
            opt_zone_tmp: std::cell::RefCell::new(vec![]),
            zone_choice_tmp: std::cell::RefCell::new(vec![]),
            opt_zone_q: vec![],
            zone_choice_q: vec![],
            last_actions: vec![],
            steps_taken: 0,
            max_steps: 2_000_000,
        };
        for (i, d) in decks.iter().enumerate() {
            let p = i as PlayerId;
            for &def in &d.main {
                g.create_card(def, p, Zone::Library, false);
            }
            for &def in &d.commanders {
                let id = g.create_card(def, p, Zone::Command, false);
                g.cards[id.0 as usize].is_commander = true;
            }
            let mut companion = d.companion;
            for &def in &d.sideboard {
                let id = g.create_card(def, p, Zone::Sideboard, false);
                if companion == Some(def) {
                    g.cards[id.0 as usize].is_companion = true;
                    companion = None;
                }
            }
        }
        for (i, d) in decks.iter().enumerate() {
            g.players[i].starting_deck = (d.main.len() + d.commanders.len()) as i32;
        }
        for p in 0..g.players.len() as PlayerId {
            g.shuffle_library(p);
        }
        g.events.clear();
        for p in 0..g.players.len() as PlayerId {
            for _ in 0..cfg.hand_size {
                g.draw_card(p);
            }
            g.players[p as usize].cards_drawn_turn = 0;
            g.players[p as usize].drew_from_empty = false;
        }
        g.events.clear();
        g.run = Run::PreGame { stage: PreGame::Sideboard { idx: 0, k: 0 } };
        g
    }

    /// `Effect::RestartGame` (CR 727): rebuild the game from the cards both players started with. Cards left in exile by the effect (`restart_pending.1`) stay in exile and enter the battlefield under the restarting player's control as the new game begins.
    pub(crate) fn do_restart(&mut self) {
        let Some((ctl, keep)) = self.restart_pending.take() else { return };
        let mut decks: Vec<DeckSpec> = self.players.iter().map(|p| DeckSpec { name: p.name.clone(), main: vec![], commanders: vec![], sideboard: vec![], companion: None }).collect();
        let mut kept: Vec<(PlayerId, DefId)> = vec![];
        for c in &self.cards {
            if c.token {
                continue;
            }
            if keep.contains(&c.id) {
                kept.push((c.owner, c.front));
            } else if c.zone == Zone::Sideboard {
                decks[c.owner as usize].sideboard.push(c.front);
                if c.is_companion {
                    decks[c.owner as usize].companion = Some(c.front);
                }
            } else if c.is_commander {
                decks[c.owner as usize].commanders.push(c.front);
            } else {
                decks[c.owner as usize].main.push(c.front);
            }
        }
        let seed: u64 = self.rng.gen();
        let mut g = Game::new(self.db.clone(), self.cfg.clone(), decks, seed);
        for (new, old) in g.players.iter_mut().zip(self.players.iter()) {
            new.auto_skip = old.auto_skip;
            new.offer_mana = old.offer_mana;
            new.allow_cancel = old.allow_cancel;
            new.manual_pay = old.manual_pay;
            new.manual_once = old.manual_once;
            new.stops_mine = old.stops_mine;
            new.stops_opp = old.stops_opp;
            new.full_control = old.full_control;
            new.conceded = old.conceded;
            new.lost = old.lost;
        }
        for (owner, def) in kept {
            let id = g.create_card(def, owner, Zone::Exile, false);
            g.restart_return.push((ctl, id));
        }
        g.first_player = ctl;
        g.steps_taken = self.steps_taken;
        g.max_steps = self.max_steps;
        g.events.clear();
        *self = g;
    }

    /// Mark a player as conceding.
    pub fn concede(&mut self, p: PlayerId) {
        self.players[p as usize].conceded = true;
        self.player_loses(p, "conceded");
        self.check_game_over();
        if self.pending.as_ref().map_or(false, |d| d.player == p) {
            self.pending = None;
            if let Some(d) = self.pending.take() {
                let _ = d;
            }
            // the conceding player's pending question is dropped; the run loop continues past them
            self.answer = None;
            self.pending = None;
            if let Run::Priority { .. } = self.run {
                self.run = Run::Priority { player: self.next_player(p), passes: 0 };
            }
        }
    }
}
