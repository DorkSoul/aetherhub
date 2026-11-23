//! Information-set Monte-Carlo tree search (single observer, root-parallel).
//!
//! Every iteration samples a plausible world from the seat's `Belief` (hidden cards guessed, nothing leaked), walks the shared
//! tree choosing among the moves legal in that world (UCB1 with availability counts, so moves that only exist in some worlds are
//! not penalised), expands one new move, plays the rest of the turn out with a cheap policy for everybody and scores the position
//! for *every* seat. A seat choosing in the tree maximises its own score, which gives sensible multiplayer play without opponent
//! modelling. Threads search independent trees (different worlds) and the root statistics are merged.
//!
//! The tree branches on priority decisions, attack and block declarations and single-target / mode picks; every other question is
//! answered by the cheap policy inside the search.

use crate::agent::Agent;
use crate::determinize::Belief;
use crate::eval::evaluate;
use crate::heuristic::*;
use mtg_engine::types::*;
use mtg_engine::*;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};

const MAXP: usize = 8;

/// How the search is tuned. Difficulty tiers are different configs of the same agent.
#[derive(Clone, Debug)]
pub struct MctsConfig {
    /// Time allowed for one decision.
    pub budget: Duration,
    /// UCB exploration constant (rewards are in -1..1).
    pub exploration: f32,
    /// Search threads (root parallelism).
    pub threads: usize,
    /// Standard deviation of noise added to every leaf score: a sloppy player cannot tell close positions apart.
    pub noise: f32,
    /// Chance per decision of taking a random one of the three best moves instead of the best.
    pub blunder: f32,
    /// Hard cap on engine decisions per playout.
    pub max_playout: u32,
    /// How many own turns the playout runs through (1 = until the turn changes).
    pub horizon_turns: u32,
    /// Weight of the counter-attack danger term in the leaf score (0 = plain evaluation).
    pub risk: f32,
    /// Playouts also use planeswalker abilities.
    pub activate: bool,
    /// Search a fixed number of iterations per thread instead of for a time budget (reproducible benchmarks; the hard deadline still applies).
    pub iters: Option<u32>,
    /// Score leaves with the fitted model (`features.rs`) instead of the hand-written evaluation.
    pub learned: bool,
    /// Score leaves with the ONNX network (`eval_mlp.onnx`, trained by scripts/ai/train_eval.py).
    pub onnx: bool,
    /// Weight of the strongest opponent in the leaf score (0.5 = the default; higher plays against the leader in multiplayer).
    pub threat: f32,
    /// Attack candidates aim at every opponent, not only the weakest and the strongest.
    pub all_targets: bool,
    /// Extra attack declarations: the k strongest attackers for every k.
    pub more_attacks: bool,
    /// How much a move must beat passing by (mean reward) to be played.
    pub pass_margin: f32,
}

impl MctsConfig {
    pub fn new(budget: Duration) -> Self {
        MctsConfig { budget, exploration: 0.9, threads: if budget >= Duration::from_millis(100) { search_threads() } else { 1 }, noise: 0.0, blunder: 0.0, max_playout: 500, horizon_turns: 1, risk: 0.0, activate: false, iters: None, learned: false, onnx: false, threat: 0.5, all_targets: false, more_attacks: false, pass_margin: 0.002 }
    }
}

struct Edge {
    key: u64,
    n: u32,
    avail: u32,
    r: [f32; MAXP],
    child: usize,
}

struct Node {
    player: PlayerId,
    edges: Vec<Edge>,
}

fn key_of(c: &Choice) -> u64 {
    let mut h = DefaultHasher::new();
    format!("{c:?}").hash(&mut h);
    h.finish()
}

/// Root statistics of one search: (choice index, visits, summed reward for the deciding seat).
type RootStats = Vec<(f32, f32)>;

/// A move with a key that means the same thing in every sampled world (what is being done, not an index into a list).
type Cand = (u64, Choice);

fn hash_str(s: &str) -> u64 {
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

pub struct MctsAgent {
    pub cfg: MctsConfig,
    inner: HeuristicAgent,
    rng: ChaCha8Rng,
    pub sims: u64,
    pub last_think: Duration,
    pub max_think: Duration,
    stuck: ((u32, String, u32, usize), u32),
    activations: ((u32, CardId, u16), u32),
}

impl MctsAgent {
    pub fn new(seed: u64, cfg: MctsConfig) -> Self {
        MctsAgent { inner: HeuristicAgent::new(seed, cfg.budget), cfg, rng: ChaCha8Rng::seed_from_u64(seed ^ 0x5eed), sims: 0, last_think: Duration::ZERO, max_think: Duration::ZERO, stuck: ((0, String::new(), 0, 0), 0), activations: ((0, CardId(0), 0), 0) }
    }

    pub fn reseed(&mut self, seed: u64) {
        self.rng = ChaCha8Rng::seed_from_u64(seed ^ 0x5eed);
        self.inner.reseed(seed);
    }
}

impl Agent for MctsAgent {
    fn decide(&mut self, g: &Game, d: &Decision) -> Choice {
        let t = Instant::now();
        let c = self.decide_inner(g, d);
        self.last_think = t.elapsed();
        self.max_think = self.max_think.max(self.last_think);
        c
    }
}

impl MctsAgent {
    fn decide_inner(&mut self, g: &Game, d: &Decision) -> Choice {
        let me = d.player;
        match &d.kind {
            DecisionKind::Priority { actions, stack_len } => {
                if actions.is_empty() {
                    return pass();
                }
                if actions.iter().any(|a| matches!(a, Action::PlayLand { .. })) {
                    return Choice::Priority { action: Some(best_land(g, me, actions)) };
                }
                let key = (g.turn, format!("{:?}", g.step), *stack_len, actions.len());
                if self.stuck.0 == key {
                    self.stuck.1 += 1;
                } else {
                    self.stuck = (key, 1);
                }
                if self.stuck.1 > 12 {
                    self.stuck.1 = 0;
                    return pass();
                }
                let cands = priority_candidates(g, actions);
                let mut c = self.search(g, d, cands);
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
            DecisionKind::Attackers { options, must_attack } => {
                if options.is_empty() {
                    return Choice::Attackers { attacks: vec![] };
                }
                let cands = attack_choices(g, me, options, must_attack, self.cfg.all_targets, self.cfg.more_attacks);
                self.search(g, d, cands)
            }
            DecisionKind::Blockers { options } => {
                let cands = block_choices(g, me, options);
                self.search(g, d, cands)
            }
            DecisionKind::Pick { .. } => match pick_candidates(g, d) {
                Some(c) => self.search(g, d, c),
                None => self.inner.decide(g, d),
            },
            _ => self.inner.decide(g, d),
        }
    }

    /// Search among `cands` (the root moves) and return the best.
    fn search(&mut self, g: &Game, d: &Decision, cands: Vec<Cand>) -> Choice {
        if cands.len() <= 1 {
            return cands.into_iter().next().map(|c| c.1).unwrap_or_else(|| policy(g, d));
        }
        let me = d.player;
        let belief = Belief::new(g, me);
        let deadline = Instant::now() + self.cfg.budget;
        let threads = self.cfg.threads.max(1);
        let seeds: Vec<u64> = (0..threads).map(|_| self.rng.gen::<u64>()).collect();
        let cfg = &self.cfg;
        let results: Vec<(RootStats, u64)> = if threads == 1 {
            vec![run_search(g, d, &belief, &cands, cfg, seeds[0], deadline)]
        } else {
            // Game is Send but not Sync: every thread searches its own copy
            let roots: Vec<Game> = seeds.iter().map(|_| g.clone()).collect();
            std::thread::scope(|sc| {
                let hs: Vec<_> = seeds
                    .iter()
                    .zip(roots)
                    .map(|(&seed, root)| {
                        let (belief, cands) = (&belief, &cands);
                        std::thread::Builder::new()
                            .stack_size(64 << 20)
                            .spawn_scoped(sc, move || run_search(&root, d, belief, cands, cfg, seed, deadline))
                            .ok()
                    })
                    .collect();
                hs.into_iter().filter_map(|h| h?.join().ok()).collect()
            })
        };
        let mut visits = vec![0u32; cands.len()];
        let mut sum = vec![0f32; cands.len()];
        for (r, sims) in &results {
            self.sims += sims;
            for (i, (n, s)) in r.iter().enumerate() {
                visits[i] += *n as u32;
                sum[i] += s;
            }
        }
        let mean = |i: usize| if visits[i] == 0 { f32::MIN } else { sum[i] / visits[i] as f32 };
        let pass_idx = cands.iter().position(|c| matches!(c.1, Choice::Priority { action: None }));
        // order by visits (robust child), ties by mean
        let mut order: Vec<usize> = (0..cands.len()).filter(|i| visits[*i] > 0).collect();
        if order.is_empty() {
            return policy(g, d);
        }
        order.sort_by(|a, b| visits[*b].cmp(&visits[*a]).then(mean(*b).partial_cmp(&mean(*a)).unwrap_or(std::cmp::Ordering::Equal)));
        // a deliberate slip: sometimes one of the best three
        let mut pick = order[0];
        if self.cfg.blunder > 0.0 && order.len() > 1 && self.rng.gen::<f32>() < self.cfg.blunder {
            pick = order[self.rng.gen_range(0..order.len().min(3))];
        }
        // doing something must beat passing by a margin: no useless casts
        if let Some(pi) = pass_idx {
            if pick != pi && visits[pi] > 0 && mean(pick) <= mean(pi) + self.cfg.pass_margin && self.cfg.blunder == 0.0 {
                pick = pi;
            }
        }
        cands[pick].1.clone()
    }
}

// ------------------------------------------------------------------ candidate moves

fn priority_candidates(g: &Game, actions: &[Action]) -> Vec<Cand> {
    let mut idx: Vec<usize> = (0..actions.len()).filter(|i| !matches!(actions[*i], Action::ActivateMana { .. })).collect();
    idx.sort_by_key(|i| action_rank(g, &actions[*i]));
    idx.truncate(10);
    let mut out: Vec<Cand> = idx.into_iter().map(|i| (hash_str(&format!("{:?}", actions[i])), Choice::Priority { action: Some(i) })).collect();
    out.push((hash_str("pass"), pass()));
    out
}

fn attack_choices(g: &Game, me: PlayerId, options: &[AttackerOpt], must: &[CardId], all: bool, more: bool) -> Vec<Cand> {
    let mut sets: Vec<Vec<(CardId, Obj)>> = vec![];
    let mut targets: Vec<PlayerId> = vec![weakest_opponent(g, me)];
    if let Some(t) = strongest_opponent(g, me) {
        if !targets.contains(&t) {
            targets.push(t);
        }
    }
    if all {
        for o in g.opponents(me) {
            if !targets.contains(&o) {
                targets.push(o);
            }
        }
    }
    for t in targets {
        for s in attack_candidates(g, me, options, must, t) {
            if !sets.contains(&s) {
                sets.push(s);
            }
        }
    }
    if more && sets.len() >= 2 {
        // the k strongest attackers for every k, aimed like the "all" set
        let all_set = sets.last().cloned().unwrap_or_default();
        let mut ranked = all_set.clone();
        ranked.sort_by(|a, b| crate::eval::permanent_value(g, b.0).partial_cmp(&crate::eval::permanent_value(g, a.0)).unwrap_or(std::cmp::Ordering::Equal));
        for k in 1..ranked.len() {
            let mut sub: Vec<(CardId, Obj)> = ranked[..k].to_vec();
            for m in must {
                if !sub.iter().any(|x| x.0 == *m) {
                    if let Some(x) = all_set.iter().find(|x| x.0 == *m) {
                        sub.push(*x);
                    }
                }
            }
            if !sets.contains(&sub) {
                sets.push(sub);
            }
        }
    }
    sets.into_iter().map(|attacks| Choice::Attackers { attacks }).map(|c| (key_of(&c), c)).collect()
}

fn block_choices(g: &Game, me: PlayerId, options: &[BlockerOpt]) -> Vec<Cand> {
    block_candidates(g, me, options).into_iter().map(|blocks| Choice::Blockers { blocks }).map(|c| (key_of(&c), c)).collect()
}

/// Branching picks: one target / mode / cost item among a few ranked options.
fn pick_candidates(g: &Game, d: &Decision) -> Option<Vec<Cand>> {
    let DecisionKind::Pick { options, min, max, purpose, .. } = &d.kind else { return None };
    if options.len() < 2 || *max != 1 || !matches!(purpose, PickPurpose::Targets { .. } | PickPurpose::Mode | PickPurpose::Cost | PickPurpose::Search | PickPurpose::Sacrifice | PickPurpose::Discard) {
        return None;
    }
    let ranked = rank_options(g, d.player, d);
    let mut out: Vec<Cand> = ranked.iter().take(5).map(|i| (hash_str(&format!("pick {:?}", options[*i])), Choice::Pick { indices: vec![*i] })).collect();
    if *min == 0 {
        out.push((hash_str("pick none"), Choice::Pick { indices: vec![] }));
    }
    Some(out)
}

/// The opponent with the most dangerous board (a threat to everyone), if different from the weakest.
fn strongest_opponent(g: &Game, me: PlayerId) -> Option<PlayerId> {
    g.opponents(me).into_iter().max_by(|a, b| board_power(g, *a).partial_cmp(&board_power(g, *b)).unwrap_or(std::cmp::Ordering::Equal))
}

fn board_power(g: &Game, p: PlayerId) -> f32 {
    g.battlefield.iter().filter(|id| g.card(**id).cur.controller == p).map(|id| crate::eval::permanent_value(g, *id)).sum::<f32>() + g.player(p).life as f32 * 0.2
}

/// Candidates at a deeper tree node (no pass bias, `None` = not a branching decision).
fn node_candidates(g: &Game, d: &Decision, all_targets: bool, more_attacks: bool) -> Option<Vec<Cand>> {
    match &d.kind {
        DecisionKind::Priority { actions, .. } if !actions.is_empty() => {
            if actions.iter().any(|a| matches!(a, Action::PlayLand { .. })) {
                let c = Choice::Priority { action: Some(best_land(g, d.player, actions)) };
                return Some(vec![(key_of(&c), c)]);
            }
            Some(priority_candidates(g, actions))
        }
        DecisionKind::Attackers { options, must_attack } if !options.is_empty() => Some(attack_choices(g, d.player, options, must_attack, all_targets, more_attacks)),
        DecisionKind::Blockers { options } => Some(block_choices(g, d.player, options)),
        DecisionKind::Pick { .. } => pick_candidates(g, d),
        _ => None,
    }
}

/// The playout's answer: greedy development for priority (land, then the most expensive spell), the cheap policy otherwise.
fn rollout_choice(g: &Game, d: &Decision, rng: &mut ChaCha8Rng, activate: bool) -> Choice {
    if let DecisionKind::Priority { actions, .. } = &d.kind {
        if let Some(i) = actions.iter().position(|a| matches!(a, Action::PlayLand { .. })) {
            let _ = i;
            return Choice::Priority { action: Some(best_land(g, d.player, actions)) };
        }
        if rng.gen::<f32>() < 0.9 {
            let best = actions
                .iter()
                .enumerate()
                .filter_map(|(i, a)| if let Action::Cast { card, .. } = a { Some((i, g.def_of(*card).mana_value)) } else { None })
                .max_by_key(|x| x.1);
            if let Some((i, _)) = best {
                return Choice::Priority { action: Some(i) };
            }
        }
        if activate && rng.gen::<f32>() < 0.8 {
            // a planeswalker's loyalty ability (the engine allows one per turn each)
            let pw = actions.iter().enumerate().find(|(_, a)| matches!(a, Action::Activate { card, .. } if g.card(*card).cur.types.has(CardType::Planeswalker)));
            if let Some((i, _)) = pw {
                return Choice::Priority { action: Some(i) };
            }
        }
        return pass();
    }
    policy(g, d)
}

// ------------------------------------------------------------------ the search itself

fn run_search(root: &Game, rd: &Decision, belief: &Belief, cands: &[Cand], cfg: &MctsConfig, seed: u64, deadline: Instant) -> (RootStats, u64) {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let me = rd.player;
    let players = root.players.len().min(MAXP);
    let mut tree: Vec<Node> = vec![Node { player: me, edges: vec![] }];
    let hard = if cfg.iters.is_some() { Instant::now() + Duration::from_secs(30) } else { deadline + cfg.budget / 2 + Duration::from_millis(20) };
    let root_turn = root.turn;
    let target_turn = root_turn + cfg.horizon_turns.max(1);
    let mut sims = 0u64;
    let mut iter = 0u32;
    loop {
        // always finish at least one pass over the root moves, then respect the clock
        match cfg.iters {
            Some(n) => {
                if iter >= n.max(cands.len() as u32) {
                    break;
                }
            }
            None => {
                if iter as usize >= cands.len() && Instant::now() >= deadline {
                    break;
                }
            }
        }
        if Instant::now() >= hard {
            break;
        }
        iter += 1;
        sims += 1;
        let mut g = belief.sample(root, &mut rng);
        g.abort_at = Some(hard);
        let mut path: Vec<(usize, usize)> = vec![];
        let mut cur: Option<usize> = Some(0);
        let mut first = true;
        let mut steps = 0u32;
        let mut rewards = [0f32; MAXP];
        loop {
            if g.run() == Status::Over {
                break;
            }
            if Instant::now() >= hard {
                break;
            }
            let d = match g.decision().cloned() {
                Some(d) => d,
                None => break,
            };
            steps += 1;
            if steps > cfg.max_playout || (!first && g.turn >= target_turn) {
                break;
            }
            let ch = if let Some(nid) = cur {
                let local;
                let cs: &[Cand] = if first {
                    cands
                } else {
                    match node_candidates(&g, &d, cfg.all_targets, cfg.more_attacks) {
                        Some(v) => {
                            local = v;
                            &local[..]
                        }
                        None => {
                            // not a branching decision: answered by the policy, the tree position is unchanged
                            let c = rollout_choice(&g, &d, &mut rng, cfg.activate);
                            if g.apply(c).is_err() {
                                break;
                            }
                            continue;
                        }
                    }
                };
                first = false;
                if cs.len() == 1 {
                    cs[0].1.clone()
                } else {
                    let ks: Vec<u64> = cs.iter().map(|c| c.0).collect();
                    tree[nid].player = d.player;
                    // moves not yet tried here, in order of the heuristic ranking
                    for k in &ks {
                        if !tree[nid].edges.iter().any(|e| e.key == *k) {
                            tree[nid].edges.push(Edge { key: *k, n: 0, avail: 0, r: [0.0; MAXP], child: usize::MAX });
                        }
                    }
                    let mut avail: Vec<usize> = vec![];
                    for k in &ks {
                        let ei = tree[nid].edges.iter().position(|e| e.key == *k).unwrap();
                        if !avail.contains(&ei) {
                            avail.push(ei);
                        }
                    }
                    for &ei in &avail {
                        tree[nid].edges[ei].avail += 1;
                    }
                    let pl = (d.player as usize).min(MAXP - 1);
                    let untried = avail.iter().position(|&ei| tree[nid].edges[ei].n == 0);
                    let chosen = match untried {
                        Some(p) => p,
                        None => {
                            let mut best = (f32::MIN, 0usize);
                            for (p, &ei) in avail.iter().enumerate() {
                                let e = &tree[nid].edges[ei];
                                let n = e.n as f32;
                                let u = e.r[pl] / n + cfg.exploration * ((e.avail as f32).ln().max(0.0) / n).sqrt();
                                if u > best.0 {
                                    best = (u, p);
                                }
                            }
                            best.1
                        }
                    };
                    let ei = avail[chosen];
                    // which candidate carries this key
                    let ci = ks.iter().position(|k| *k == tree[nid].edges[ei].key).unwrap_or(0);
                    path.push((nid, ei));
                    if tree[nid].edges[ei].child == usize::MAX {
                        tree.push(Node { player: me, edges: vec![] });
                        let id = tree.len() - 1;
                        tree[nid].edges[ei].child = id;
                        cur = None; // expansion: the rest is a playout
                    } else {
                        cur = Some(tree[nid].edges[ei].child);
                    }
                    cs[ci].1.clone()
                }
            } else {
                rollout_choice(&g, &d, &mut rng, cfg.activate)
            };
            if g.apply(ch).is_err() {
                // the move was not legal in this world; leave it unscored
                path.clear();
                break;
            }
        }
        if path.is_empty() {
            continue;
        }
        for (p, r) in rewards.iter_mut().enumerate().take(players) {
            *r = if cfg.onnx { crate::features::onnx_eval(&g, p as PlayerId) } else if cfg.learned { crate::features::learned_eval(&g, p as PlayerId) } else if cfg.risk > 0.0 { risk_score(&g, p as PlayerId, cfg.risk) } else { crate::eval::evaluate_w(&g, p as PlayerId, cfg.threat) };
            if cfg.noise > 0.0 {
                // roughly gaussian: sum of uniforms
                let z: f32 = (0..4).map(|_| rng.gen::<f32>() - 0.5).sum::<f32>() * 1.7;
                *r = (*r + z * cfg.noise).clamp(-1.0, 1.0);
            }
        }
        for (nid, ei) in path {
            let e = &mut tree[nid].edges[ei];
            e.n += 1;
            for p in 0..MAXP {
                e.r[p] += rewards[p];
            }
        }
    }
    let pl = (me as usize).min(MAXP - 1);
    let stats: RootStats = cands
        .iter()
        .map(|c| match tree[0].edges.iter().find(|e| e.key == c.0) {
            Some(e) => (e.n as f32, e.r[pl]),
            None => (0.0, 0.0),
        })
        .collect();
    (stats, sims)
}

/// Evaluation minus the danger of being open to a counter-attack: what the opponents' creatures could push through
/// the blockers `p` has left untapped, against `p`'s life.
fn risk_score(g: &Game, p: PlayerId, w: f32) -> f32 {
    let base = evaluate(g, p);
    if g.over.is_some() {
        return base;
    }
    let life = g.player(p).life as f32;
    let blockers = g.battlefield.iter().filter(|id| {
        let c = g.card(**id);
        c.cur.controller == p && c.cur.types.has(CardType::Creature) && !c.tapped
    }).count();
    let mut worst = 0f32;
    for o in g.opponents(p) {
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
        worst = worst.max(powers.iter().skip(blockers).sum::<i32>() as f32);
    }
    let danger = if worst >= life { 0.6 } else if worst >= life * 0.6 { 0.15 } else { 0.0 };
    base - w * danger
}
