//! Hidden information. Before simulating, the AI replaces everything its seat cannot know with a random guess:
//!
//! * the order of its own library (except cards it has seen: scried, revealed, bounced back),
//! * the identity of every unknown card in an opponent's hand and library.
//!
//! The guess for an opponent's unknown cards is NOT their real remaining cards (that would leak their decklist).
//! It is drawn from a belief: the cards of theirs we have seen (public zones, revealed, looked at, scried),
//! weighted up because decks run several copies, plus our own deck as a stand-in for "a typical deck".
//! The number of lands is taken from the usual ratio minus the lands already seen.
//! Cards whose identity the seat knows (`Card::known_to`) stay exactly as they are.

use mtg_engine::types::{CardType, Zone};
use mtg_engine::*;
use rand::seq::SliceRandom;
use rand::Rng;

/// What one seat knows and can't know about the cards in hidden zones. Built once per decision, sampled many times.
pub struct Belief {
    me: PlayerId,
    /// Positions in my library whose card I have not seen.
    own_lib_unknown: Vec<usize>,
    opps: Vec<OppBelief>,
}

struct OppBelief {
    player: PlayerId,
    /// Cards in the hand / library whose identity I do not know (their ids stay, their definitions are sampled).
    unknown: Vec<CardId>,
    n_lands: usize,
    lands: Vec<(DefId, f32, u32)>,
    spells: Vec<(DefId, f32, u32)>,
}

fn known_to_seat(g: &Game, id: CardId, me: PlayerId) -> bool {
    let c = g.card(id);
    c.owner == me || c.known_to & (1u8 << (me as u32 & 7)) != 0
}

impl Belief {
    pub fn new(g: &Game, me: PlayerId) -> Belief {
        let own_lib_unknown: Vec<usize> =
            g.players[me as usize].library.iter().enumerate().filter(|(_, id)| g.card(**id).known_to & (1u8 << (me as u32 & 7)) == 0).map(|(i, _)| i).collect();
        let max_copies: u32 = if g.cfg.commander { 1 } else { 4 };
        // my own deck stands in for "a deck like this"
        let mut mine_lands: Vec<(DefId, f32, u32)> = vec![];
        let mut mine_spells: Vec<(DefId, f32, u32)> = vec![];
        for c in &g.cards {
            if c.owner != me || c.token || matches!(c.zone, Zone::Sideboard | Zone::Command) {
                continue;
            }
            let d = g.db.def(c.front);
            let list = if d.types.has(CardType::Land) { &mut mine_lands } else { &mut mine_spells };
            match list.iter_mut().find(|e| e.0 == c.front) {
                Some(e) => e.1 += 1.0,
                None => list.push((c.front, 1.0, 0)),
            }
        }
        let mut opps = vec![];
        for p in g.opponents(me) {
            let pl = &g.players[p as usize];
            let mut unknown: Vec<CardId> = vec![];
            for &id in pl.hand.iter().chain(pl.library.iter()) {
                if !known_to_seat(g, id, me) {
                    unknown.push(id);
                }
            }
            if unknown.is_empty() {
                continue;
            }
            // what we have seen of their deck
            let mut seen_lands: Vec<(DefId, f32, u32)> = vec![];
            let mut seen_spells: Vec<(DefId, f32, u32)> = vec![];
            let (mut deck_size, mut lands_seen) = (0usize, 0usize);
            for c in &g.cards {
                if c.owner != p || c.token || matches!(c.zone, Zone::Sideboard | Zone::Command) {
                    continue;
                }
                deck_size += 1;
                let hidden = matches!(c.zone, Zone::Hand | Zone::Library) && !known_to_seat(g, c.id, me);
                if hidden || (c.face_down && c.zone != Zone::Battlefield) {
                    continue;
                }
                let d = g.db.def(c.front);
                let is_land = d.types.has(CardType::Land);
                if is_land {
                    lands_seen += 1;
                }
                let list = if is_land { &mut seen_lands } else { &mut seen_spells };
                match list.iter_mut().find(|e| e.0 == c.front) {
                    Some(e) => e.2 += 1,
                    None => list.push((c.front, 0.0, 1)),
                }
            }
            // candidate weights: a seen card is likely to have more copies; our own deck fills in the rest
            let build = |seen: &Vec<(DefId, f32, u32)>, mine: &Vec<(DefId, f32, u32)>| -> Vec<(DefId, f32, u32)> {
                let mut out: Vec<(DefId, f32, u32)> = seen.iter().map(|e| (e.0, 3.0, e.2)).collect();
                for m in mine {
                    match out.iter_mut().find(|e| e.0 == m.0) {
                        Some(e) => e.1 += m.1,
                        None => out.push((m.0, m.1, 0)),
                    }
                }
                out
            };
            let lands = build(&seen_lands, &mine_lands);
            let spells = build(&seen_spells, &mine_spells);
            let ratio = if g.cfg.commander { 0.37 } else { 0.42 };
            let expected = (deck_size as f32 * ratio).round() as i64 - lands_seen as i64;
            let n_lands = expected.clamp(0, unknown.len() as i64) as usize;
            let _ = max_copies;
            opps.push(OppBelief { player: p, unknown, n_lands, lands, spells });
        }
        Belief { me, own_lib_unknown, opps }
    }

    /// One plausible world: same public state, hidden cards guessed.
    pub fn sample(&self, g: &Game, rng: &mut impl Rng) -> Game {
        let mut c = g.clone();
        let me = self.me;
        // my own library: the unseen cards in unseen order
        if self.own_lib_unknown.len() > 1 {
            let lib = &mut c.players[me as usize].library;
            let mut ids: Vec<CardId> = self.own_lib_unknown.iter().map(|&i| lib[i]).collect();
            ids.shuffle(rng);
            for (&pos, id) in self.own_lib_unknown.iter().zip(ids) {
                lib[pos] = id;
            }
        }
        let max_copies: u32 = if g.cfg.commander { 1 } else { 4 };
        for ob in &self.opps {
            let n = ob.unknown.len();
            // how many lands among the unknown cards: the expectation, give or take one
            let jitter: i64 = rng.gen_range(-1..=1);
            let nl = ((ob.n_lands as i64 + jitter).clamp(0, n as i64)) as usize;
            let mut defs: Vec<DefId> = Vec::with_capacity(n);
            draw(&ob.lands, nl, if g.cfg.commander { 1 } else { u32::MAX }, rng, &mut defs);
            // basic lands may repeat freely; if none of the candidates are basic the cap above would stall, so top up from the same list uncapped
            while defs.len() < nl && !ob.lands.is_empty() {
                defs.push(ob.lands[rng.gen_range(0..ob.lands.len())].0);
            }
            draw(&ob.spells, n - defs.len().min(n), max_copies, rng, &mut defs);
            while defs.len() < n {
                // no candidates at all: a copy of any known card of theirs, or of ours
                let pool = if !ob.spells.is_empty() { &ob.spells } else { &ob.lands };
                if pool.is_empty() {
                    break;
                }
                defs.push(pool[rng.gen_range(0..pool.len())].0);
            }
            defs.shuffle(rng);
            for (&id, def) in ob.unknown.iter().zip(defs) {
                let card = &mut c.cards[id.0 as usize];
                card.def = def;
                card.front = def;
                card.known_to = 0;
                c.reset_derived(id);
            }
            let _ = ob.player;
        }
        c
    }
}

/// Draw `k` definitions by weight, never more than `cap` copies of one (counting copies already seen).
fn draw(cands: &[(DefId, f32, u32)], k: usize, cap: u32, rng: &mut impl Rng, out: &mut Vec<DefId>) {
    let mut taken: Vec<u32> = cands.iter().map(|e| e.2).collect();
    for _ in 0..k {
        let total: f32 = cands.iter().enumerate().filter(|(i, _)| taken[*i] < cap).map(|(_, e)| e.1).sum();
        if total <= 0.0 {
            return;
        }
        let mut r = rng.gen::<f32>() * total;
        let mut pick = None;
        for (i, e) in cands.iter().enumerate() {
            if taken[i] >= cap {
                continue;
            }
            pick = Some(i);
            r -= e.1;
            if r <= 0.0 {
                break;
            }
        }
        match pick {
            Some(i) => {
                taken[i] += 1;
                out.push(cands[i].0);
            }
            None => return,
        }
    }
}

/// A plausible world for `me` (a belief built and sampled once). Prefer `Belief` when sampling repeatedly.
pub fn determinize(g: &Game, me: PlayerId, rng: &mut impl Rng) -> Game {
    let mut c = Belief::new(g, me).sample(g, rng);
    // other players' picks in a hidden "each player chooses" are not known yet
    c.forget_hidden_picks(me);
    c
}
