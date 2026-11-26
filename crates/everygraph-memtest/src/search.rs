// SPDX-License-Identifier: Apache-2.0 OR MIT

//! A search for graphs on which an operation holds more untracked heap per node: a hill
//! climb over [`Recipe`]s with a fixed seed (targeted property-based testing). A recipe
//! is kept when it raises, for some operation, the fitted slope or the maximum of any
//! phase, even if nothing failed (`MemLock`, `PerfFuzz`;
//! `docs/research/sota-testing.md` challenge 3, items 2 and 3). Fitness is counted in
//! bytes, never in time or resident memory.

use std::collections::BTreeMap;
use std::path::Path;

use crate::graphs::{Recipe, splitmix};
use crate::ops::{Input, Op};
use crate::probe::{Outcome, measure, slope};

/// What an operation holds untracked at each size of one recipe.
#[derive(Clone, Debug, Default)]
pub struct Growth {
    /// (nodes, untracked bytes), for the fit.
    pub points: Vec<(f64, f64)>,
    /// The most untracked bytes by phase, at the largest size.
    pub phases: Vec<(&'static str, u64)>,
}

/// Runs each of `ops` on `recipe` at each of `sizes`, with `dir` for the inputs, and
/// records its [`Growth`].
///
/// # Panics
///
/// If a run does not complete without a limit.
pub fn growth(ops: &[Op], recipe: &Recipe, sizes: &[u32], dir: &Path) -> Vec<Growth> {
    let mut out = vec![Growth::default(); ops.len()];
    for &n in sizes {
        let at = dir.join(format!("input-{n}"));
        let input = Input::prepare(&recipe.at(n), &at);
        for (op, g) in ops.iter().zip(&mut out) {
            let run = measure(None, None, &dir.join("scratch"), |memory, scratch| {
                op.run(&input, memory, scratch)
            });
            let name = (op.name(), recipe.name);
            assert_eq!(run.outcome, Outcome::Done, "{name:?} at n = {n}");
            g.points
                .push((f64::from(input.nodes), run.untracked() as f64));
            g.phases = run.phases;
        }
        drop(input);
        let _ = std::fs::remove_dir_all(at);
    }
    out
}

/// The worst seen for one operation.
#[derive(Clone, Debug)]
pub struct Worst {
    /// The steepest slope of untracked heap against nodes, in bytes per node.
    pub slope: f64,
    /// The recipe that gave it.
    pub recipe: Recipe,
    /// Per phase, the most untracked bytes per node at the largest size, and the
    /// recipe that gave it. Findings are told apart by phase, never by input.
    pub phases: BTreeMap<&'static str, (f64, Recipe)>,
}

/// Climbs from `start` for `steps` steps, at `sizes`, for each of `ops`, with `dir` for
/// the inputs; returns the worst found per operation.
pub fn climb(
    ops: &[Op],
    start: &[Recipe],
    sizes: &[u32],
    steps: usize,
    seed: u64,
    dir: &Path,
) -> BTreeMap<String, Worst> {
    let largest = f64::from(sizes.iter().copied().max().unwrap_or(1));
    let mut worst: BTreeMap<String, Worst> = BTreeMap::new();
    let raises = |recipe: Recipe, worst: &mut BTreeMap<String, Worst>| {
        let mut raised = false;
        for (&op, g) in ops.iter().zip(growth(ops, &recipe, sizes, dir)) {
            let s = slope(&g.points);
            let w = worst.entry(op.name()).or_insert_with(|| Worst {
                slope: f64::NEG_INFINITY,
                recipe,
                phases: BTreeMap::new(),
            });
            if s > w.slope {
                (w.slope, w.recipe, raised) = (s, recipe, true);
            }
            for (phase, bytes) in g.phases {
                let per_node = bytes as f64 / largest;
                let best = w.phases.entry(phase).or_insert((f64::NEG_INFINITY, recipe));
                if per_node > best.0 {
                    *best = (per_node, recipe);
                    raised = true;
                }
            }
        }
        raised
    };
    let mut kept: Vec<Recipe> = start.to_vec();
    for &recipe in start {
        raises(recipe, &mut worst);
    }
    let mut rng = seed;
    for _ in 0..steps {
        let parent = kept[pick(&mut rng, kept.len())];
        let child = mutate(parent, &mut rng);
        if raises(child, &mut worst) {
            kept.push(child);
        }
    }
    worst
}

fn pick(rng: &mut u64, len: usize) -> usize {
    usize::try_from(splitmix(rng) % len as u64).unwrap_or(0)
}

/// A recipe like `r` with one part changed (Singularity's components: more or fewer
/// edges, hubs, a longer chain, other components, repeats, isolated nodes, other ids).
fn mutate(r: Recipe, rng: &mut u64) -> Recipe {
    let one = |rng: &mut u64, of: &[u32]| of[pick(rng, of.len())];
    let mut c = Recipe { name: "found", ..r };
    match splitmix(rng) % 9 {
        0 => c.degree16 = one(rng, &[4, 16, 32, 64, 128, 256]),
        1 => c.hubs = one(rng, &[0, 1, 4, 16]),
        2 => c.hub_share = one(rng, &[16, 128, 512, 1024]),
        3 => c.chain_share = one(rng, &[0, 64, 512, 1024]),
        4 => c.component = one(rng, &[0, 2, 3, 8, 64]),
        5 => c.repeats = one(rng, &[1, 2, 4, 8]),
        6 => c.isolated_share = one(rng, &[0, 128, 512, 896]),
        7 => {
            c.stride = [1, 1 << 20, 1 << 40][pick(rng, 3)];
            c.text = splitmix(rng).is_multiple_of(2);
        }
        _ => c.seed = splitmix(rng),
    }
    c
}
