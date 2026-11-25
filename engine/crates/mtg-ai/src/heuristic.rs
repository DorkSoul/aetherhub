//! A budgeted, simulation-backed agent: it generates a few candidate plays, simulates each on a determinized
//! clone with cheap policies for everyone else, and picks the best-scoring one. Always has an answer ready.

use crate::agent::Agent;
use crate::determinize::determinize;
use crate::eval::*;
use mtg_engine::polarity::Polarity;
use mtg_engine::types::*;
use mtg_engine::*;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

pub struct HeuristicAgent {
    /// Time allowed for one decision (difficulty).
    pub budget: Duration,
    rng: ChaCha8Rng,
    plan: VecDeque<Choice>,
    /// Decision-log position at which the next plan step is expected (a plan only applies to the questions that directly follow it).
    plan_at: usize,
    pub sims: u64,
    pub last_think: Duration,
    pub max_think: Duration,
    /// (turn, step, stack size, actions) of the last priority decision and how many times in a row it came up: a cast the engine keeps abandoning would otherwise be retried forever.
    stuck: ((u32, String, u32, usize), u32),
    /// Activations this turn of the same ability by this agent: a free ability that never pays off (Varchild's Crusader) is not worth more than a few tries.
    activations: ((u32, CardId, u16), u32),
}

impl HeuristicAgent {
    pub fn new(seed: u64, budget: Duration) -> Self {
        HeuristicAgent { budget, rng: ChaCha8Rng::seed_from_u64(seed), plan: VecDeque::new(), plan_at: 0, sims: 0, last_think: Duration::ZERO, max_think: Duration::ZERO, stuck: ((0, String::new(), 0, 0), 0), activations: ((0, CardId(0), 0), 0) }
    }
}

impl Agent for HeuristicAgent {
    fn decide(&mut self, g: &Game, d: &Decision) -> Choice {
        let t = Instant::now();
        let c = self.decide_inner(g, d);
        self.last_think = t.elapsed();
        self.max_think = self.max_think.max(self.last_think);
        c
    }
}

pub(crate) fn pass() -> Choice {
    Choice::Priority { action: None }
}

impl HeuristicAgent {
    /// Restart the agent's randomness from a seed (the session derives it from the game seed, the seat and the decision count,
    /// so a restored game continues exactly as the original would).
    pub fn reseed(&mut self, seed: u64) {
        self.rng = ChaCha8Rng::seed_from_u64(seed);
    }

    fn decide_inner(&mut self, g: &Game, d: &Decision) -> Choice {
        let me = d.player;
        if matches!(d.kind, DecisionKind::Priority { .. }) {
            self.plan.clear();
        } else if !self.plan.is_empty() {
            if g.decision_log.len() == self.plan_at {
                let c = self.plan.pop_front().unwrap();
                if g.check_choice(d, &c) {
                    self.plan_at += 1;
                    return c;
                }
            }
            self.plan.clear();
        }
        match &d.kind {
            DecisionKind::Priority { actions, stack_len } => {
                let key = (g.turn, format!("{:?}", g.step), *stack_len, actions.len());
                if self.stuck.0 == key {
                    self.stuck.1 += 1;
                } else {
                    self.stuck = (key, 1);
                }
                if self.stuck.1 > 12 {
                    // the same choices again and again with nothing changing: give up on them and pass
                    self.stuck.1 = 0;
                    return pass();
                }
                let mut c = self.priority(g, me, actions);
                if let Choice::Priority { action: Some(i) } = &c {
                    if let Some(Action::Activate { card, ability }) = actions.get(*i) {
                        let k = (g.turn, *card, *ability);
                        self.activations = if self.activations.0 == k { (k, self.activations.1 + 1) } else { (k, 1) };
                        if self.activations.1 > 12 {
                            c = pass();
                        }
                    }
                }
                if matches!(c, Choice::Priority { action: None }) {
                    self.stuck.1 = 0;
                }
                c
            }
            DecisionKind::Attackers { options, must_attack } => self.attackers(g, me, options, must_attack),
            DecisionKind::Blockers { options } => self.blockers(g, me, options),
            DecisionKind::Number { prompt, min, max, .. } if max > min => self.number(g, me, prompt, *min, *max),
            _ => policy(g, d),
        }
    }

    // ------------------------------------------------------------------ priority

    fn priority(&mut self, g: &Game, me: PlayerId, actions: &[Action]) -> Choice {
        if actions.is_empty() {
            return pass();
        }
        let start = Instant::now();
        let deadline = start + self.budget;
        // lands: always play one, choosing the colors we are missing
        if actions.iter().any(|a| matches!(a, Action::PlayLand { .. })) {
            return Choice::Priority { action: Some(best_land(g, me, actions)) };
        }
        let mut order: Vec<usize> = (0..actions.len()).collect();
        order.sort_by_key(|i| action_rank(g, &actions[*i]));
        // several determinizations searched in parallel (one thread each); a candidate is judged by its mean score
        let workers = if self.budget >= Duration::from_millis(150) && actions.len() > 1 { search_threads() } else { 1 };
        let dets: Vec<Game> = (0..workers).map(|_| determinize(g, me, &mut self.rng)).collect();
        let seeds: Vec<u64> = (0..workers).map(|_| rand::Rng::gen::<u64>(&mut self.rng)).collect();
        let budget = self.budget;
        let results: Vec<(f32, Vec<Option<(f32, Vec<Choice>)>>)> = if workers == 1 {
            vec![self.score_actions(&dets[0], me, &order, actions.len(), deadline)]
        } else {
            let order_ref = &order;
            let n = actions.len();
            std::thread::scope(|sc| {
                let hs: Vec<_> = dets
                    .into_iter()
                    .zip(seeds.into_iter())
                    .map(|(det, seed)| {
                        sc.spawn(move || {
                            let mut w = HeuristicAgent::new(seed, budget);
                            w.score_actions(&det, me, order_ref, n, deadline)
                        })
                    })
                    .collect();
                hs.into_iter().filter_map(|h| h.join().ok()).collect()
            })
        };
        if results.is_empty() {
            return pass();
        }
        let base = results.iter().map(|r| r.0).sum::<f32>() / results.len() as f32;
        let mut best: Option<(f32, usize, Vec<Choice>)> = None;
        for &i in &order {
            let got: Vec<&(f32, Vec<Choice>)> = results.iter().filter_map(|r| r.1[i].as_ref()).collect();
            if got.is_empty() {
                continue;
            }
            let mean = got.iter().map(|x| x.0).sum::<f32>() / got.len() as f32;
            if best.as_ref().map_or(true, |b| mean > b.0) {
                // the follow-up plan comes from the determinization that liked this play most
                let top = got.iter().max_by(|x, y| x.0.partial_cmp(&y.0).unwrap_or(std::cmp::Ordering::Equal)).unwrap();
                best = Some((mean, i, top.1.clone()));
            }
        }
        match best {
            Some((score, i, path)) if score > base + 0.002 => {
                self.plan = path.into();
                self.plan_at = g.decision_log.len() + 1;
                Choice::Priority { action: Some(i) }
            }
            _ => pass(),
        }
    }

    /// Score the position if everyone passes (`base`) and each candidate first action on one determinization.
    fn score_actions(&mut self, det: &Game, me: PlayerId, order: &[usize], n: usize, deadline: Instant) -> (f32, Vec<Option<(f32, Vec<Choice>)>>) {
        let base = self.settle_score(det, me);
        let mut out: Vec<Option<(f32, Vec<Choice>)>> = vec![None; n];
        let mut have = false;
        for &i in order {
            if Instant::now() > deadline && have {
                break;
            }
            let mut sim = det.clone();
            if sim.apply(Choice::Priority { action: Some(i) }).is_err() {
                continue;
            }
            let mut branches = 12usize;
            let (score, path) = self.explore(sim, me, vec![], &mut branches, deadline);
            out[i] = Some((score, path));
            have = true;
        }
        (base, out)
    }

    /// Score of the position once the stack has resolved if everyone passes from here.
    fn settle_score(&mut self, g: &Game, me: PlayerId) -> f32 {
        let sim = self.settle_game(g.clone(), me);
        evaluate(&sim, me)
    }

    /// Play the position forward until the stack is empty and `me` has priority again (everyone else passes or follows the cheap policy).
    fn settle_game(&mut self, mut sim: Game, me: PlayerId) -> Game {
        let hard = Instant::now() + self.budget + Duration::from_millis(20);
        sim.abort_at = Some(hard);
        for _ in 0..60 {
            if Instant::now() > hard {
                break;
            }
            if sim.run() == Status::Over {
                break;
            }
            let d = sim.decision().cloned().unwrap();
            if matches!(d.kind, DecisionKind::Priority { .. }) {
                if sim.stack.is_empty() && d.player == me {
                    // the stack is empty and it is my priority again
                    break;
                }
                if sim.apply(pass()).is_err() {
                    break;
                }
            } else {
                let c = policy(&sim, &d);
                if sim.apply(c).is_err() {
                    break;
                }
            }
        }
        sim
    }

    /// Run a clone forward until the stack is empty, branching over my own target/mode choices.
    fn explore(&mut self, mut g: Game, me: PlayerId, mut path: Vec<Choice>, branches: &mut usize, deadline: Instant) -> (f32, Vec<Choice>) {
        self.sims += 1;
        // a playout never runs far past the thinking budget, whatever the board looks like
        let hard = deadline + self.budget / 2 + Duration::from_millis(20);
        g.abort_at = Some(hard);
        for _ in 0..80 {
            if Instant::now() > hard {
                return (evaluate(&g, me), path);
            }
            if g.run() == Status::Over {
                return (evaluate(&g, me), path);
            }
            let d = g.decision().cloned().unwrap();
            match &d.kind {
                DecisionKind::Priority { .. } => {
                    if g.stack.is_empty() {
                        return (self.leaf_score(&g, me), path);
                    }
                    if g.apply(pass()).is_err() {
                        return (evaluate(&g, me), path);
                    }
                }
                DecisionKind::Pick { options, min, max, purpose, .. }
                    if d.player == me
                        && options.len() > 1
                        && *branches > 0
                        && Instant::now() < deadline
                        && *max == 1
                        && matches!(purpose, PickPurpose::Targets { .. } | PickPurpose::Mode | PickPurpose::Cost | PickPurpose::Search | PickPurpose::Sacrifice | PickPurpose::Discard) =>
                {
                    let ranked = rank_options(&g, me, &d);
                    let take = ranked.len().min(*branches).min(8);
                    *branches = branches.saturating_sub(take);
                    let mut best: Option<(f32, Vec<Choice>)> = None;
                    let mut cands: Vec<Vec<usize>> = ranked.iter().take(take).map(|i| vec![*i]).collect();
                    if *min == 0 {
                        cands.push(vec![]);
                    }
                    for idx in cands {
                        let mut g2 = g.clone();
                        let ch = Choice::Pick { indices: idx };
                        if g2.apply(ch.clone()).is_err() {
                            continue;
                        }
                        let mut p2 = path.clone();
                        p2.push(ch);
                        let (s, p) = self.explore(g2, me, p2, branches, deadline);
                        if best.as_ref().map_or(true, |b| s > b.0) {
                            best = Some((s, p));
                        }
                    }
                    return best.unwrap_or((evaluate(&g, me), path));
                }
                _ => {
                    let c = policy(&g, &d);
                    if d.player == me {
                        path.push(c.clone());
                    }
                    if g.apply(c).is_err() {
                        return (evaluate(&g, me), path);
                    }
                }
            }
        }
        (evaluate(&g, me), path)
    }

    fn leaf_score(&self, g: &Game, me: PlayerId) -> f32 {
        evaluate(g, me)
    }

    /// A number question: try the sensible values on determinized clones (X, life paid, a secret number...) and keep the best-scoring one.
    fn number(&mut self, g: &Game, me: PlayerId, prompt: &str, min: i32, max: i32) -> Choice {
        // paying life for a bonus: the evaluation would pay all but one point, so never offer more than half of what is left
        let max = if crate::textchoice::pays_life(prompt) { max.min((g.player(me).life / 2).max(min)) } else { max };
        let default = crate::textchoice::number_default(prompt, min, max);
        let cands = crate::textchoice::number_candidates(min, max, default);
        let deadline = Instant::now() + self.budget;
        let det = determinize(g, me, &mut self.rng);
        let mut best: Option<(f32, i32)> = None;
        for n in cands {
            if Instant::now() > deadline && best.is_some() {
                break;
            }
            let mut sim = det.clone();
            if sim.apply(Choice::Number { n }).is_err() {
                continue;
            }
            let end = self.settle_game(sim, me);
            let mut s = evaluate(&end, me);
            // a number that costs more than two fifths of the life is never worth it (the evaluation alone would pay all but one point)
            let (before, after) = (g.player(me).life, end.player(me).life);
            let keep = before * 3 / 5;
            if after < keep {
                s -= 0.15 * (keep - after) as f32;
            }
            // the cheap answer wins ties
            let s = s + if n == default { 0.0005 } else { 0.0 };
            if best.map_or(true, |b| s > b.0) {
                best = Some((s, n));
            }
        }
        Choice::Number { n: best.map_or(default, |b| b.1) }
    }

    // ------------------------------------------------------------------ combat

    fn attackers(&mut self, g: &Game, me: PlayerId, options: &[AttackerOpt], must: &[CardId]) -> Choice {
        if options.is_empty() {
            return Choice::Attackers { attacks: vec![] };
        }
        let deadline = Instant::now() + self.budget;
        let target = weakest_opponent(g, me);
        let sets = attack_candidates(g, me, options, must, target);
        let det = determinize(g, me, &mut self.rng);
        let mut best: Option<(f32, usize)> = None;
        for (i, set) in sets.iter().enumerate() {
            if Instant::now() > deadline && best.is_some() {
                break;
            }
            let mut sim = det.clone();
            if sim.apply(Choice::Attackers { attacks: set.clone() }).is_err() {
                continue;
            }
            self.sims += 1;
            let s = self.finish_combat(sim, me);
            // prefer less risky options on ties: fewer attackers
            let s = s - 0.0005 * set.len() as f32;
            if best.map_or(true, |b| s > b.0) {
                best = Some((s, i));
            }
        }
        Choice::Attackers { attacks: sets[best.map_or(0, |b| b.1)].clone() }
    }

    /// Play combat out on a clone (others decide by policy) and score the position after damage.
    fn finish_combat(&mut self, mut g: Game, me: PlayerId) -> f32 {
        for _ in 0..200 {
            if g.run() == Status::Over {
                return evaluate(&g, me);
            }
            let d = g.decision().cloned().unwrap();
            if let DecisionKind::Priority { .. } = d.kind {
                if g.stack.is_empty() && matches!(g.step, Step::EndCombat | Step::Main2 | Step::End) {
                    return evaluate_with_risk(&g, me);
                }
                if g.apply(pass()).is_err() {
                    break;
                }
                continue;
            }
            let c = policy(&g, &d);
            if g.apply(c).is_err() {
                break;
            }
        }
        evaluate(&g, me)
    }

    fn blockers(&mut self, g: &Game, me: PlayerId, options: &[BlockerOpt]) -> Choice {
        let deadline = Instant::now() + self.budget;
        let sets = block_candidates(g, me, options);
        if sets.len() == 1 {
            return Choice::Blockers { blocks: sets[0].clone() };
        }
        let det = determinize(g, me, &mut self.rng);
        let mut best: Option<(f32, usize)> = None;
        for (i, set) in sets.iter().enumerate() {
            if Instant::now() > deadline && best.is_some() {
                break;
            }
            let mut sim = det.clone();
            if sim.apply(Choice::Blockers { blocks: set.clone() }).is_err() {
                continue;
            }
            self.sims += 1;
            let s = self.finish_combat(sim, me);
            if best.map_or(true, |b| s > b.0) {
                best = Some((s, i));
            }
        }
        Choice::Blockers { blocks: sets[best.map_or(0, |b| b.1)].clone() }
    }
}

/// Evaluation that also accounts for being open to a counter-attack (tapped creatures can't block next turn).
pub fn evaluate_with_risk(g: &Game, me: PlayerId) -> f32 {
    let base = evaluate(g, me);
    if g.over.is_some() {
        return base;
    }
    let my_life = g.player(me).life as f32;
    let my_blockers = g.battlefield.iter().filter(|id| {
        let c = g.card(**id);
        c.cur.controller == me && c.cur.types.has(CardType::Creature) && !c.tapped
    }).count();
    let mut worst = 0.0f32;
    for o in g.opponents(me) {
        let mut powers: Vec<i32> = g
            .battlefield
            .iter()
            .filter(|id| {
                let c = g.card(**id);
                c.cur.controller == o && c.cur.types.has(CardType::Creature) && !c.cur.kw.has(Keyword::Defender)
            })
            .map(|id| g.card(*id).cur.power)
            .collect();
        powers.sort_by(|a, b| b.cmp(a));
        let unblocked: i32 = powers.iter().skip(my_blockers).sum();
        worst = worst.max(unblocked as f32);
    }
    let danger = if worst >= my_life { 0.6 } else if worst >= my_life * 0.6 { 0.15 } else { 0.0 };
    base - danger
}

// ---------------------------------------------------------------------- policies (cheap, deterministic)

/// A reasonable answer for any non-priority decision without searching.
pub fn policy(g: &Game, d: &Decision) -> Choice {
    let me = d.player;
    match &d.kind {
        DecisionKind::Mulligan { hand, mulligans, peek } => {
            let is_land = |c: &CardId| g.card(*c).cur.types.has(CardType::Land);
            let lands = hand.iter().filter(|c| is_land(c)).count();
            let n = hand.len();
            let mut keep = n <= 5 || (*mulligans >= 3) || (lands >= 2 && lands <= if n >= 7 { 5 } else { 4 });
            if !peek.is_empty() {
                // free mulligan + scry 3: the scry can keep the lands it needs on top (or bottom the extra ones), so a
                // one-lander with two lands coming, or a six-lander with spells coming, is fine; mulligans are free,
                // so stop only after several
                let peek_lands = peek.iter().filter(|c| is_land(c)).count();
                keep = (2..=5).contains(&lands) || (lands == 1 && peek_lands >= 2) || (lands == 6 && peek_lands < peek.len()) || *mulligans >= 5;
                if keep {
                    // scry before keeping: lands past five and spells far above the land count go to the bottom
                    let bad: Vec<usize> = peek
                        .iter()
                        .enumerate()
                        .filter(|(_, c)| if is_land(c) { lands >= 5 } else { g.def_of(**c).mana_value as usize > lands + 3 })
                        .map(|(i, _)| i)
                        .collect();
                    if !bad.is_empty() {
                        return Choice::Pick { indices: bad };
                    }
                }
            }
            Choice::Keep { keep }
        }
        DecisionKind::Bottom { hand, n } => {
            let mut cards: Vec<CardId> = hand.clone();
            let mut out = vec![];
            for _ in 0..*n {
                let lands = cards.iter().filter(|c| g.card(**c).cur.types.has(CardType::Land)).count();
                let idx = if lands > 3 {
                    cards.iter().position(|c| g.card(*c).cur.types.has(CardType::Land))
                } else {
                    let mut best: Option<(usize, u32)> = None;
                    for (i, c) in cards.iter().enumerate() {
                        let k = g.card(*c);
                        if k.cur.types.has(CardType::Land) {
                            continue;
                        }
                        let mv = g.def_of(*c).mana_value;
                        if best.map_or(true, |b| mv > b.1) {
                            best = Some((i, mv));
                        }
                    }
                    best.map(|b| b.0)
                };
                let i = idx.unwrap_or(0);
                out.push(cards.remove(i));
            }
            Choice::Bottom { cards: out }
        }
        DecisionKind::Priority { .. } => pass(),
        DecisionKind::Pick { options, min, max, purpose, .. } => {
            let ranked = rank_options(g, me, d);
            let (min, max) = (*min as usize, (*max as usize).min(options.len()));
            let want = match purpose {
                PickPurpose::Scry | PickPurpose::Surveil => {
                    // choose cards to bottom/graveyard: keep good ones on top
                    let lands_have = g.player(me).hand.iter().filter(|c| g.card(**c).cur.types.has(CardType::Land)).count()
                        + g.battlefield.iter().filter(|c| g.card(**c).cur.controller == me && g.card(**c).cur.types.has(CardType::Land)).count();
                    let bad: Vec<usize> = options
                        .iter()
                        .enumerate()
                        .filter_map(|(i, o)| {
                            if let Opt::Obj { obj: Obj::Card { id } } = o {
                                let land = g.card(*id).cur.types.has(CardType::Land);
                                let mv = g.def_of(*id).mana_value as usize;
                                if (land && lands_have >= 5) || (!land && mv > lands_have + 3) {
                                    return Some(i);
                                }
                            }
                            None
                        })
                        .collect();
                    return Choice::Pick { indices: bad };
                }
                PickPurpose::Targets { polarity, .. } => {
                    // take as many as needed; fewer when the best options are bad
                    let n = min.max(1).min(max);
                    let good = ranked.iter().take(n).filter(|i| option_score(g, me, &options[**i], *polarity) > -50.0).count();
                    if min == 0 {
                        good
                    } else {
                        n
                    }
                }
                PickPurpose::Search => max.min(1).max(min),
                _ => min,
            };
            Choice::Pick { indices: ranked.into_iter().take(want).collect() }
        }
        DecisionKind::Number { prompt, min, max, .. } => Choice::Number { n: crate::textchoice::number_default(prompt, *min, *max) },
        DecisionKind::YesNo { .. } => Choice::YesNo { yes: true },
        DecisionKind::PayMana { .. } => Choice::Pay { answer: PayAnswer::Auto },
        DecisionKind::Attackers { options, must_attack } => {
            let target = weakest_opponent(g, me);
            let sets = attack_candidates(g, me, options, must_attack, target);
            // without searching: the "safe attackers" set
            Choice::Attackers { attacks: sets.get(2).or(sets.first()).cloned().unwrap_or_default() }
        }
        DecisionKind::Blockers { options } => {
            let sets = block_candidates(g, me, options);
            Choice::Blockers { blocks: sets.last().cloned().unwrap_or_default() }
        }
        DecisionKind::AssignDamage { total, blockers, defender, trample, .. } => {
            let mut order: Vec<&(CardId, i32)> = blockers.iter().collect();
            order.sort_by(|a, b| permanent_value(g, b.0).partial_cmp(&permanent_value(g, a.0)).unwrap());
            let mut left = *total;
            let mut assign = vec![];
            for (b, lethal) in order {
                let n = left.min(*lethal);
                if n > 0 {
                    assign.push((Obj::card(*b), n));
                    left -= n;
                }
            }
            if left > 0 {
                if let (true, Some(def)) = (*trample, defender) {
                    assign.push((*def, left));
                } else if let Some(last) = assign.last_mut() {
                    last.1 += left;
                } else if let Some(def) = defender {
                    assign.push((*def, left));
                }
            }
            Choice::Damage { assign }
        }
        DecisionKind::Order { items, .. } => Choice::Order { order: (0..items.len()).collect() },
    }
}

fn obj_value(g: &Game, o: &Obj) -> (bool, f32) {
    // (belongs to nobody-in-particular placeholder, value)
    match o {
        Obj::Card { id } => (false, permanent_value(g, *id)),
        _ => (false, 0.0),
    }
}

/// How attractive an option is given the polarity (higher is better; below -50 means "never").
fn option_score(g: &Game, me: PlayerId, o: &Opt, pol: Polarity) -> f32 {
    match o {
        Opt::Obj { obj } => match obj {
            Obj::Card { id } => {
                let c = g.card(*id);
                let mine = if c.zone == Zone::Battlefield { c.cur.controller == me } else { c.owner == me };
                let v = obj_value(g, obj).1 + if c.zone == Zone::Battlefield { 0.0 } else { g.def_of(*id).mana_value as f32 * 0.3 };
                match pol {
                    Polarity::Hostile => {
                        if c.zone == Zone::Stack || c.zone == Zone::Graveyard && !mine {
                            v
                        } else if mine {
                            -100.0 - v
                        } else {
                            v
                        }
                    }
                    Polarity::Friendly => {
                        if mine {
                            v
                        } else {
                            -100.0 - v
                        }
                    }
                    Polarity::Neutral => {
                        if mine {
                            -v * 0.5
                        } else {
                            v
                        }
                    }
                }
            }
            Obj::Player { id } => {
                let life = g.player(*id).life as f32;
                match pol {
                    Polarity::Hostile | Polarity::Neutral => {
                        if *id == me {
                            -100.0
                        } else {
                            3.5 + (25.0 - life).max(0.0) * 0.25
                        }
                    }
                    Polarity::Friendly => {
                        if *id == me {
                            6.0
                        } else {
                            -100.0
                        }
                    }
                }
            }
            Obj::Stack { id } => {
                let item = g.stack.iter().find(|s| s.id == *id);
                match item {
                    Some(s) if s.controller != me => 4.0 + g.def_of(s.source).mana_value as f32,
                    _ => -100.0,
                }
            }
        },
        Opt::Text { .. } | Opt::Color { .. } => 0.0,
    }
}

/// Option indices of a `Pick`, best first.
pub fn rank_options(g: &Game, me: PlayerId, d: &Decision) -> Vec<usize> {
    let DecisionKind::Pick { options, purpose, .. } = &d.kind else { return vec![] };
    if let Some(r) = crate::textchoice::rank_text_pick(g, me, d).or_else(|| crate::textchoice::rank_exchange(g, me, d)) {
        return r;
    }
    let mut scored: Vec<(usize, f32)> = options
        .iter()
        .enumerate()
        .map(|(i, o)| {
            let s = match purpose {
                PickPurpose::Targets { polarity, .. } => option_score(g, me, o, *polarity),
                PickPurpose::Discard | PickPurpose::Cost | PickPurpose::Sacrifice => {
                    // lowest keep-value first
                    match o {
                        Opt::Obj { obj: Obj::Card { id } } => -keep_value(g, me, *id),
                        _ => 0.0,
                    }
                }
                PickPurpose::Search => match o {
                    Opt::Obj { obj: Obj::Card { id } } => keep_value(g, me, *id),
                    _ => 0.0,
                },
                PickPurpose::Legend => match o {
                    Opt::Obj { obj: Obj::Card { id } } => keep_value(g, me, *id),
                    _ => 0.0,
                },
                _ => 0.0,
            };
            (i, s)
        })
        .collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.into_iter().map(|(i, _)| i).collect()
}

/// How much we want to keep (or fetch) this card.
fn keep_value(g: &Game, me: PlayerId, id: CardId) -> f32 {
    let c = g.card(id);
    let lands_have = g.battlefield.iter().filter(|x| g.card(**x).cur.controller == me && g.card(**x).cur.types.has(CardType::Land)).count()
        + g.player(me).hand.iter().filter(|x| g.card(**x).cur.types.has(CardType::Land)).count();
    if c.cur.types.has(CardType::Land) {
        // lands are valuable until we have plenty
        return if lands_have < 4 { 6.0 } else if lands_have < 6 { 3.0 } else { 0.5 };
    }
    let mv = g.def_of(id).mana_value as f32;
    let base = permanent_value(g, id).max(1.0) + mv * 0.4;
    if c.zone == Zone::Battlefield && c.token {
        return base * 0.5;
    }
    base
}

pub(crate) fn action_rank(g: &Game, a: &Action) -> i32 {
    match a {
        Action::PlayLand { .. } => 0,
        Action::Cast { card, .. } => 10 - g.def_of(*card).mana_value.min(9) as i32,
        Action::Activate { .. } => 12,
        _ => 20,
    }
}

pub(crate) fn best_land(g: &Game, me: PlayerId, actions: &[Action]) -> usize {
    // demand per color from spells in hand
    let mut demand = [0i32; 5];
    for &id in &g.player(me).hand {
        for sh in &g.def_of(id).cost.shards {
            match sh {
                mtg_engine::mana::Shard::Color { c } => demand[*c as usize] += 2,
                mtg_engine::mana::Shard::Hybrid { a, b } => {
                    demand[*a as usize] += 1;
                    demand[*b as usize] += 1
                }
                _ => {}
            }
        }
    }
    // supply on the battlefield
    for &id in &g.battlefield {
        let c = g.card(id);
        if c.cur.controller == me {
            for &ai in &g.def_of(id).mana_abilities {
                if let Some(mtg_engine::dsl::Ability::Activated { effects, .. }) = g.def_of(id).abilities.get(ai as usize) {
                    for e in effects {
                        if let mtg_engine::dsl::Effect::AddMana { mana, .. } = e {
                            for m in mana {
                                if let mtg_engine::dsl::ManaProduce::Fixed { m, .. } = m {
                                    if let Some(col) = m.color() {
                                        demand[col as usize] -= 1;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    let mut best = (i32::MIN, 0usize);
    for (i, a) in actions.iter().enumerate() {
        if let Action::PlayLand { card } = a {
            let d = g.def_of(*card);
            let mut s = 0;
            for &ai in &d.mana_abilities {
                if let Some(mtg_engine::dsl::Ability::Activated { effects, .. }) = d.abilities.get(ai as usize) {
                    for e in effects {
                        if let mtg_engine::dsl::Effect::AddMana { mana, .. } = e {
                            for m in mana {
                                if let mtg_engine::dsl::ManaProduce::Fixed { m, .. } = m {
                                    if let Some(col) = m.color() {
                                        s += demand[col as usize];
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if s > best.0 {
                best = (s, i);
            }
        }
    }
    best.1
}

pub(crate) fn weakest_opponent(g: &Game, me: PlayerId) -> PlayerId {
    g.opponents(me).into_iter().min_by_key(|p| g.player(*p).life).unwrap_or(me)
}

/// Would `blocker` kill `attacker` in combat (ignoring tricks)? Accounts for first strike and deathtouch.
fn deals_lethal(g: &Game, from: CardId, to: CardId) -> bool {
    let a = g.card(from);
    let b = g.card(to);
    a.cur.power > 0 && (a.cur.kw.has(Keyword::Deathtouch) || a.cur.power >= b.cur.toughness - b.damage)
}

fn has_fs(g: &Game, id: CardId) -> bool {
    let k = g.card(id).cur.kw;
    k.has(Keyword::FirstStrike) || k.has(Keyword::DoubleStrike)
}

/// Does `x` survive a fight with `y`?
fn survives(g: &Game, x: CardId, y: CardId) -> bool {
    if !deals_lethal(g, y, x) {
        return true;
    }
    // x survives if it kills y first with first strike and y has none
    has_fs(g, x) && !has_fs(g, y) && deals_lethal(g, x, y)
}

/// Candidate attack declarations, from timid to all-in.
pub(crate) fn attack_candidates(g: &Game, me: PlayerId, options: &[AttackerOpt], must: &[CardId], target: PlayerId) -> Vec<Vec<(CardId, Obj)>> {
    let opps = g.opponents(me);
    let blockers: Vec<CardId> = g
        .battlefield
        .iter()
        .copied()
        .filter(|id| {
            let c = g.card(*id);
            opps.contains(&c.cur.controller) && c.cur.types.has(CardType::Creature) && !c.tapped
        })
        .collect();
    let mk = |sel: &dyn Fn(&AttackerOpt) -> bool| -> Vec<(CardId, Obj)> {
        options
            .iter()
            .filter(|o| sel(o) || must.contains(&o.card))
            .filter_map(|o| {
                let def = if o.players.contains(&target) { Obj::player(target) } else { Obj::player(*o.players.first()?) };
                Some((o.card, def))
            })
            .collect()
    };
    let can_block = |att: CardId| -> Vec<CardId> { blockers.iter().copied().filter(|b| g.can_block_attacker(*b, att)).collect() };
    let none = mk(&|_| false);
    let all = mk(&|_| true);
    let safe = mk(&|o| {
        let bl = can_block(o.card);
        bl.iter().all(|b| survives(g, o.card, *b))
    });
    // attackers that are safe, or whose trade is favorable for me
    let favorable = mk(&|o| {
        let bl = can_block(o.card);
        let mine = permanent_value(g, o.card);
        bl.iter().all(|b| survives(g, o.card, *b) || permanent_value(g, *b) >= mine * 0.9 || !deals_lethal(g, *b, o.card))
    });
    let evasive = mk(&|o| can_block(o.card).is_empty());
    let mut sets = vec![none, evasive, safe, favorable, all];
    sets.dedup();
    // remove duplicates preserving order
    let mut uniq: Vec<Vec<(CardId, Obj)>> = vec![];
    for s in sets {
        if !uniq.contains(&s) {
            uniq.push(s);
        }
    }
    uniq
}

/// Candidate block declarations: nothing, sensible blocks, and chump-everything-lethal.
pub(crate) fn block_candidates(g: &Game, me: PlayerId, options: &[BlockerOpt]) -> Vec<Vec<(CardId, CardId)>> {
    let attackers: Vec<CardId> = g
        .combat
        .as_ref()
        .map(|c| c.attackers.iter().filter(|a| matches!(a.defender, Obj::Player { id } if id == me) || matches!(a.defender, Obj::Card { .. })).map(|a| a.card).collect())
        .unwrap_or_default();
    let my_life = g.player(me).life;
    let mut by_power: Vec<CardId> = attackers.clone();
    by_power.sort_by_key(|a| -g.card(*a).cur.power);
    let incoming: i32 = attackers
        .iter()
        .filter(|a| g.combat.as_ref().map_or(false, |c| c.attackers.iter().any(|x| x.card == **a && matches!(x.defender, Obj::Player { id } if id == me))))
        .map(|a| g.card(*a).cur.power)
        .sum();
    let mut used: Vec<CardId> = vec![];
    let mut good: Vec<(CardId, CardId)> = vec![];
    for &a in &by_power {
        let cands: Vec<&BlockerOpt> = options.iter().filter(|o| !used.contains(&o.card) && o.can_block.contains(&a)).collect();
        // safe kill, then safe wall, then favorable trade
        let safe_kill = cands.iter().filter(|o| survives(g, o.card, a) && deals_lethal(g, o.card, a)).min_by_key(|o| permanent_value(g, o.card) as i32);
        if let Some(o) = safe_kill {
            used.push(o.card);
            good.push((o.card, a));
            continue;
        }
        let wall = cands.iter().filter(|o| survives(g, o.card, a)).min_by_key(|o| permanent_value(g, o.card) as i32);
        if let Some(o) = wall {
            if !g.card(a).cur.kw.has(Keyword::Trample) {
                used.push(o.card);
                good.push((o.card, a));
                continue;
            }
        }
        let trade = cands
            .iter()
            .filter(|o| deals_lethal(g, o.card, a) && permanent_value(g, o.card) <= permanent_value(g, a) * 1.1)
            .min_by_key(|o| permanent_value(g, o.card) as i32);
        if let Some(o) = trade {
            used.push(o.card);
            good.push((o.card, a));
        }
    }
    // chump blocks if what's left is dangerous
    let mut with_chumps = good.clone();
    let mut remaining: i32 = incoming
        - with_chumps
            .iter()
            .filter(|(_, a)| g.combat.as_ref().map_or(false, |c| c.attackers.iter().any(|x| x.card == *a && matches!(x.defender, Obj::Player { id } if id == me))))
            .map(|(_, a)| g.card(*a).cur.power)
            .sum::<i32>();
    for &a in &by_power {
        if remaining < my_life - 3 {
            break;
        }
        if with_chumps.iter().any(|(_, x)| *x == a) {
            continue;
        }
        let chump = options
            .iter()
            .filter(|o| !with_chumps.iter().any(|(b, _)| *b == o.card) && o.can_block.contains(&a))
            .min_by_key(|o| permanent_value(g, o.card) as i32);
        if let Some(o) = chump {
            with_chumps.push((o.card, a));
            remaining -= g.card(a).cur.power;
        }
    }
    let mut sets: Vec<Vec<(CardId, CardId)>> = vec![vec![]];
    for s in [good, with_chumps] {
        if !sets.contains(&s) {
            sets.push(s);
        }
    }
    sets
}

/// Threads for one decision's search: leave a core for the rest of the server, at most 4.
pub(crate) fn search_threads() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).saturating_sub(1).clamp(1, 4)
}
