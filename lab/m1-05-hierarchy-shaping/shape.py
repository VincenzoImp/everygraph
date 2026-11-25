#!/usr/bin/env python3
"""Experiment #19 (m1-05): shape a community hierarchy for the map.

    shape.py solve   --store S --work W [--ooc --budget-gb B]       plain Leiden: partition T + natural levels
    shape.py build   --method M --store S --work W [...]            -> W/tree_<M>.npz
    shape.py metrics --tree W/tree_<M>.npz --store S --out R.json   fan-out, modularity, conductance, derived share
    shape.py export  --tree W/tree_<M>.npz --out HIERDIR            gmap hierarchy (levels by height, kinds)
    shape.py compare --tree A.npz --tree B.npz [--mask DEG.u32]     agreement level by level (roles), ARI and NMI

Methods (issue #19 candidates; F = target fan-out, default 64):
    capin     the cap during coarsening (lab baseline): m102 --cap F --max-passes 20 --min-ratio 3,
              hierarchy as produced (`--iterations 2` uses the fixed capped refinement)
    capin+agg the same, then the fan-out post-pass below
    sat       the plain (uncapped) hierarchy, then the post-pass: satellite aggregates only
    capafter  T first, then a capped coarsening inside every community of T (cut graph with
              degree-restoring self-loops: the full graph's modularity, moves only inside T), then T
              as a level, then the post-pass
    rec       T; every region with more than 64 nodes is split again by Leiden on its own subgraph
              with its own null model (local modularity, resolution 1). One m102 run per round, on
              the cut graph of that round's active regions only (renumbered); per-node resolution
              multipliers give each region its own null model. Super-communities above T when T
              needs more than F-1 communities to cover 90% of the nodes (--up auto). Then the post-pass.
    rec-up    rec with the super level always built (--up on)
    nest      nested resolution levels: as rec, with full degrees (degree-restoring self-loops) and
              the null model of the region's own volume, at gamma = beta^(k+1) for the k-th split
              below T; an indivisible region above 384 nodes tries again one step finer
    nest-up   nest with the super level always built

The post-pass (satellite aggregates): every region with more than F children keeps its F-1
largest children and puts the others into one aggregate. Inside an aggregate the largest members
(F/2) are direct children and the rest is split by size rank into at most F/2 smaller
aggregates, recursively: at most F children everywhere, depth logarithmic. The drawing shows
one hatched area per parent ("+N smaller communities", design/preview/style/viewer.html): the
nested aggregates are not drawn (gmap flags them, the viewer and svgmap.py skip them).

Roles (Tree.role): every region records the level of meaning it belongs to (super, T, T+1, ...),
so that T is found as an antichain of the tree whatever the depths, and two trees are compared
level by level (compare, metrics' by_role).

Everything is a fixed function of the graph, T and the seed: m102 runs in deterministic mode, and
every tie is broken by id. Kinds: 0 root, 1 found community, 2 aggregate, 3 subdivision (a split
that needed a resolution above the method's own: rec --retries only).
"""
import argparse
import json
import os
import resource
import subprocess
import sys
import time

import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
M102 = os.path.join(HERE, "..", "m1-02-ooc-leiden", "target", "release", "m102")
NONE = np.uint32(0xFFFFFFFF)
ROOT, COMM, AGG, SUBDIV = 0, 1, 2, 3
KIND_NAMES = {ROOT: "root", COMM: "community", AGG: "aggregate", SUBDIV: "subdivision"}


def log(*a):
    print(f"[shape {time.strftime('%H:%M:%S')}]", *a, file=sys.stderr, flush=True)


def m102(args, out_json=True):
    t = time.time()
    p = subprocess.run([M102] + [str(x) for x in args], capture_output=True, text=True)
    if p.returncode != 0:
        sys.stderr.write(p.stderr[-3000:])
        raise SystemExit(f"m102 {args[0]} failed with exit {p.returncode}")
    lines = [l for l in p.stdout.splitlines() if l.startswith("{")]
    d = json.loads(lines[-1]) if (out_json and lines) else {}
    d["_wall_s"] = time.time() - t
    return d


def leiden_args(a):
    """Common flags of every Leiden run: deterministic, one iteration, compressed store."""
    x = ["--config", "comp", "--threads", a.threads, "--det"]
    if a.ooc:
        x += ["--mem", "mmap-lazy", "--ooc", "--spill", os.path.join(a.work, "spill")]
        if a.budget_gb:
            x += ["--budget-gb", a.budget_gb]
    else:
        x += ["--mem", "mmap"]
    return x


def store_n(store):
    return json.load(open(os.path.join(store, "meta.json")))["n"]


# ------------------------------------------------------------------------------------- tree
class Tree:
    """par[i], kind[i], nn[i] (graph nodes inside), ncomm[i] (communities inside an aggregate,
    for its label), role[i]. leaf[u] = the tree leaf holding graph node u.

    role: what level of meaning a region belongs to, so that trees can be compared level by
    level whatever their depths: 0 root, 1 super-community (above T), 2 the partition T (for
    capin: its own top partition), 2+k the k-th split below T (rec/nest round k; the k-th
    natural or capped level below the top for the level-built trees), -1 aggregate (a drawing
    aid: skipped). Roles only grow along a path from the root; when a one-child region is
    removed its child keeps the smaller role (they hold the same nodes), so T is always found
    as the deepest region of role <= 2 on a node's path (T is the antichain of role-2 regions)."""

    def __init__(self, n):
        self.par = np.array([-1], np.int64)
        self.kind = np.array([ROOT], np.uint8)
        self.nn = np.array([n], np.int64)
        self.ncomm = np.array([0], np.int64)
        self.role = np.array([0], np.int16)
        self.leaf = np.zeros(n, np.int64)

    @property
    def size(self):
        return len(self.par)

    def add(self, par, kind, nn, role, ncomm=None):
        k = len(par)
        ids = np.arange(self.size, self.size + k, dtype=np.int64)
        self.par = np.concatenate([self.par, np.asarray(par, np.int64)])
        self.kind = np.concatenate([self.kind, np.broadcast_to(np.asarray(kind, np.uint8), (k,))])
        self.nn = np.concatenate([self.nn, np.asarray(nn, np.int64)])
        self.ncomm = np.concatenate([self.ncomm, np.zeros(k, np.int64) if ncomm is None else np.asarray(ncomm, np.int64)])
        self.role = np.concatenate([self.role, np.broadcast_to(np.asarray(role, np.int16), (k,))])
        return ids

    def save(self, path, meta):
        np.savez(path, par=self.par, kind=self.kind, nn=self.nn, ncomm=self.ncomm, role=self.role, leaf=self.leaf, meta=json.dumps(meta))

    @staticmethod
    def load(path):
        z = np.load(path)
        t = Tree.__new__(Tree)
        for k in ["par", "kind", "nn", "ncomm", "role", "leaf"]:
            setattr(t, k, z[k])
        return t, json.loads(str(z["meta"]))

    def children_index(self):
        """(order, start): children of i are order[start[i]:start[i+1]], sorted by id."""
        p = self.par.copy()
        p[0] = self.size  # root last, has no parent
        order = np.argsort(p, kind="stable")
        cnt = np.bincount(p, minlength=self.size + 1)[: self.size]
        start = np.zeros(self.size + 1, np.int64)
        np.cumsum(cnt, out=start[1:])
        return order[: start[-1]], start

    def depth(self):
        d = np.zeros(self.size, np.int64)
        d[1:] = -1
        while (d < 0).any():
            ok = (d < 0) & (d[np.maximum(self.par, 0)] >= 0)
            ok[0] = False
            d[ok] = d[self.par[ok]] + 1
        return d

    def height(self):
        h = np.zeros(self.size, np.int64)
        while True:
            h2 = h.copy()
            np.maximum.at(h2, self.par[1:], h[1:] + 1)
            if (h2 == h).all():
                return h
            h = h2

    def compact(self, keep):
        """Keep the nodes in `keep` (bool), re-parenting each kept node to its nearest kept
        ancestor; ids renumbered in the old order (the root stays 0)."""
        keep = keep.copy()
        keep[0] = True
        anc = self.par.copy()
        anc[0] = 0
        while True:
            bad = ~keep[anc]
            if not bad.any():
                break
            anc[bad] = self.par[anc[bad]]
        new = np.cumsum(keep) - 1
        self.par = np.where(np.arange(self.size) == 0, -1, new[anc])[keep]
        self.kind, self.nn, self.ncomm, self.role = self.kind[keep], self.nn[keep], self.ncomm[keep], self.role[keep]
        # only empty nodes and one-child internal nodes are ever removed, never a leaf
        assert keep[self.leaf].all()
        self.leaf = new[self.leaf]

    def collapse_single(self):
        """Remove every non-root node that has exactly one child (the child takes its place and
        the smaller of the two roles: both regions hold the same nodes)."""
        order, start = self.children_index()
        nch = np.diff(start)
        drop = (nch == 1)
        drop[0] = False
        if drop.any():
            d = np.nonzero(drop)[0]
            only = order[start[d]]
            while True:  # down chains of one-child regions
                r = np.minimum(self.role[only], self.role[d])
                if (r == self.role[only]).all():
                    break
                self.role[only] = r
            self.compact(~drop)


def levels_to_tree(n, levels):
    """levels[0]: graph node -> level-0 unit; levels[k]: level-(k-1) unit -> level-k unit.
    The top level's units are the root's children."""
    t = Tree(n)
    base = []
    # create top-down so that ids grow downwards (parents before children)
    sizes = [int(l.max()) + 1 for l in levels]
    nodes = [np.bincount(levels[0], minlength=sizes[0]).astype(np.int64)]
    for k in range(1, len(levels)):
        nodes.append(np.bincount(levels[k], weights=nodes[k - 1], minlength=sizes[k]).astype(np.int64))
    ids = [None] * len(levels)
    top = len(levels) - 1
    ids[top] = t.add(np.zeros(sizes[top], np.int64), COMM, nodes[top], 2)
    for k in range(top - 1, -1, -1):
        ids[k] = t.add(ids[k + 1][levels[k + 1]], COMM, nodes[k], 2 + top - k)
    t.leaf = ids[0][levels[0]]
    alive = np.ones(t.size, bool)
    alive[1:] = t.nn[1:] > 0
    if not alive.all():
        t.compact(alive)
    t.collapse_single()
    return t


def bound_fanout(t, F, kmax):
    """The satellite-aggregate post-pass. Children are ranked by (node count desc, id)."""
    order, start = t.children_index()
    nch = np.diff(start)
    over = np.nonzero(nch > F)[0]
    new_par, new_nn, new_nc = [], [], []
    reparent = {}
    nxt = t.size

    def build(members, parent):
        # members: child ids (sorted by rank); returns nothing, creates aggregate nodes under parent
        nonlocal nxt
        me = nxt
        nxt += 1
        new_par.append(parent)
        new_nn.append(int(t.nn[members].sum()))
        new_nc.append(len(members))
        if len(members) <= F:
            for c in members:
                reparent[int(c)] = me
            return
        half = F // 2
        for c in members[:half]:
            reparent[int(c)] = me
        rest = members[half:]
        chunks = np.array_split(rest, min(half, -(-len(rest) // half)))
        for ch in chunks:
            if len(ch) == 1:
                reparent[int(ch[0])] = me
            else:
                build(ch, me)

    for p in over:
        ch = order[start[p]: start[p + 1]]
        rank = np.lexsort((ch, -t.nn[ch]))
        ch = ch[rank]
        build(ch[F - 1:], int(p))
    if new_par:
        t.add(np.array(new_par), AGG, np.array(new_nn), -1, np.array(new_nc))
        keys = np.fromiter(reparent.keys(), np.int64)
        vals = np.fromiter(reparent.values(), np.int64)
        t.par[keys] = vals
    return {"parents_over_F": int(len(over)), "aggregates": len(new_par)}


# ------------------------------------------------------------------------------------- solve
def cmd_solve(a):
    os.makedirs(a.work, exist_ok=True)
    memb = os.path.join(a.work, "plain.memb.u32")
    hier = os.path.join(a.work, "plain.hier")
    d = m102(["leiden", "run", "--store", a.store] + leiden_args(a) + ["--out-memb", memb] + ([] if a.no_hier else ["--out-hier", hier]))
    json.dump(d, open(os.path.join(a.work, "plain.json"), "w"))
    log("plain", d["time_s"], "Q", d["modularity"], "comms", d["communities"], "levels", d["level_sizes"])


def read_levels(hdir):
    h = json.load(open(os.path.join(hdir, "hier.json")))
    return [np.fromfile(os.path.join(hdir, f"level_{k}.u32"), np.uint32).astype(np.int64) for k in range(len(h["levels"]))]


# ------------------------------------------------------------------------------------- rounds
def cover(sizes, share=0.9):
    s = np.sort(np.asarray(sizes))[::-1]
    return int(np.searchsorted(np.cumsum(s), share * s.sum()) + 1)


def upward(a, n, T):
    """Super-communities above T when T needs more than F-1 communities to cover 90% of the
    nodes: Leiden on the whole graph at resolution 1/2, 1/4, ..., 2^-16 until the groups need at most
    F-1; each community of T goes to the group holding most of its nodes (ties: smaller label),
    so the levels nest by construction. These runs continue while a pass shrinks the graph at
    all (--agg-tol 0.99): at low resolution the default (stop below 20% shrinkage) stops early
    on road networks (roadnet-ca, 1/128: 122 groups to cover 90% against 16). Returns (per-node group label or None, record)."""
    _, Tinv, Tcnt = np.unique(T, return_inverse=True, return_counts=True)
    rec = {"cover90_T": cover(Tcnt), "tries": [], "mode": a.up}
    rec["built"] = not (a.up == "off" or (a.up == "auto" and rec["cover90_T"] <= a.F - 1))
    if not rec["built"]:
        return None, rec
    best = None
    for j in range(1, 17):
        gamma = 0.5 ** j
        f = os.path.join(a.work, "up.memb.u32")
        d = m102(["leiden", "run", "--store", a.store] + leiden_args(a) + ["--gamma", gamma, "--agg-tol", 0.99, "--max-passes", 60, "--out-memb", f, "--no-eval"])
        P = np.fromfile(f, np.uint32).astype(np.int64)
        key = Tinv.astype(np.int64) * (int(P.max()) + 1) + P
        uk, ck = np.unique(key, return_counts=True)
        kt, kp = uk // (int(P.max()) + 1), uk % (int(P.max()) + 1)
        o = np.lexsort((kp, -ck, kt))
        first = np.ones(len(o), bool)
        first[1:] = kt[o][1:] != kt[o][:-1]
        g_of_T = np.empty(len(Tcnt), np.int64)
        g_of_T[kt[o][first]] = kp[o][first]
        gsize = np.bincount(g_of_T, weights=Tcnt)
        c = cover(gsize[gsize > 0])
        rec["tries"].append({"gamma": gamma, "groups": int((gsize > 0).sum()), "cover90": c, "leiden_s": d["time_s"], "wall_s": d["_wall_s"],
                             "anon_peak_kb": d.get("rss_anon_peak_kb")})
        log("upward", rec["tries"][-1])
        del P, key, uk, ck, kt, kp, o, first
        best = None
        best = (g_of_T[Tinv], gamma)
        if c <= a.F - 1:
            break
    rec["gamma"] = best[1]
    return best[0], rec


def recursive(a, n):
    """rec / nest: rounds of Leiden on the cut graph of the regions that are still too large.

    Each round's cut graph holds the active regions only, renumbered (m102 cutgraph --labels),
    so a region's split does not depend on how many other nodes are already finished: no
    inactive vertex enters Leiden's stopping test or its scans. What still couples the regions
    of one round is Leiden's own processing order and its global stopping rules (the total gain
    of an iteration, the share of vertices merged in a pass); dev_independence.sh measures the
    effect (a region alone against the same region in the union).

    rec: each region with its own null model (local modularity of its induced subgraph, gamma 1:
    multiplier m_active / e_region). nest: degree-restoring self-loops (full degrees) and the
    null model of the region's own volume in the full graph (multiplier vol_active / vol_region),
    at gamma = beta^k for the k-th split below T; an indivisible region above kmax stays active
    at the next, higher gamma (at most --nest-stuck rounds).
    Memory: during an m102 call the driver holds cur (8 B per node) and the active mask."""
    t = Tree(n)
    T = np.fromfile(os.path.join(a.work, "plain.memb.u32"), np.uint32).astype(np.int64)
    tu = time.time()
    U, uprec = upward(a, n, T)
    uprec["seconds"] = time.time() - tu
    # round 0: the partition T under the root (under its super-community, if any)
    us, inv, cnt = np.unique(T, return_inverse=True, return_counts=True)
    rank = np.lexsort((us, -cnt))
    pos = np.empty_like(rank)
    pos[rank] = np.arange(len(rank))
    if U is None:
        ids = t.add(np.zeros(len(us), np.int64), COMM, cnt[rank], 2)
    else:
        gu, ginv, gcnt = np.unique(U, return_inverse=True, return_counts=True)
        grank = np.lexsort((gu, -gcnt))
        gpos = np.empty_like(grank)
        gpos[grank] = np.arange(len(grank))
        gids = t.add(np.zeros(len(gu), np.int64), COMM, gcnt[grank], 1)
        g_of_t = np.empty(len(us), np.int64)
        g_of_t[inv] = gids[gpos[ginv]]
        ids = t.add(g_of_t[rank], COMM, cnt[rank], 2)
        del U, gu, ginv, gcnt, grank, gpos, g_of_t
    cur = ids[pos[inv]]
    del T, us, inv, cnt, rank, pos, ids
    done = np.zeros(t.size, bool)  # finished leaf (indivisible)
    stuck = np.zeros(t.size, np.int64)  # nest: consecutive rounds without a split
    mult = np.ones(t.size, np.float64)  # rec --retries: resolution multiplier of a retried region
    rounds = []
    for r in range(1, a.max_rounds + 1):
        grow = t.size - len(done)
        if grow > 0:
            done = np.concatenate([done, np.zeros(grow, bool)])
            stuck = np.concatenate([stuck, np.zeros(grow, np.int64)])
            mult = np.concatenate([mult, np.ones(grow)])
        isleaf_now = np.ones(t.size, bool)
        isleaf_now[np.unique(t.par[1:])] = False
        active = isleaf_now & (t.nn > a.leaf_max) & ~done
        active[0] = False
        na = int(active.sum())
        if na == 0:
            break
        act_u = active[cur]
        idx = np.nonzero(act_u)[0]  # the cut graph's node order (increasing original id)
        lab = np.where(act_u, cur, NONE).astype(np.uint32)
        lab_f = os.path.join(a.work, "round.lab.u32")
        lab.tofile(lab_f)
        del lab
        cut = os.path.join(a.work, "round.cut")
        stats = os.path.join(a.work, "round.stats")
        deg_f = os.path.join(a.work, "round.deg.u32")
        cg = ["cutgraph", "--store", a.store, "--out", cut, "--labels", lab_f, "--threads", a.threads, "--out-stats", stats]
        if a.method == "nest":
            cg += ["--selfloop", "--out-deg", deg_f]
        dc = m102(cg)
        reg = cur[idx]
        if a.method == "rec":
            e = np.fromfile(stats + ".internal.u64", np.uint64).astype(np.float64)
            e = np.concatenate([e, np.zeros(max(0, t.size - len(e)))])
            ea = e[reg]
            if (ea <= 0).any():
                bad = np.unique(reg[ea <= 0])[:5]
                raise SystemExit(f"rec: region(s) {bad.tolist()} have no internal edge (a community of the input partition is not connected; "
                                 "Leiden's are, Louvain's may not be). Split the input partition into connected components first.")
            gm = (e[active].sum() / ea) * mult[reg]
            gamma = 1.0
            del e, ea
        else:
            deg = np.fromfile(deg_f, np.uint32)[idx].astype(np.float64)
            vol = np.bincount(reg, weights=deg, minlength=t.size)
            # gamma rises with the split depth k below T (role 2 + k): gamma = beta^(k+1), plus one
            # step per round a region stayed indivisible; one run holds regions of several depths,
            # so gamma goes into the multiplier
            gm = (deg.sum() / vol[reg]) * a.beta ** (t.role[reg].astype(np.float64) - 1 + stuck[reg])
            gamma = 1.0
            del deg, vol
        gm.tofile(os.path.join(a.work, "round.gmul.f64"))
        del gm
        args = ["leiden", "run", "--store", cut] + leiden_args(a) + ["--gmul", os.path.join(a.work, "round.gmul.f64"),
                                                                    "--out-memb", os.path.join(a.work, "round.memb.u32"), "--no-eval"] + a.round_flags.split()
        dl = m102(args)
        s = np.fromfile(os.path.join(a.work, "round.memb.u32"), np.uint32).astype(np.int64)
        assert len(s) == len(idx)
        us, inv, cnt = np.unique(s, return_inverse=True, return_counts=True)
        sreg = np.zeros(len(us), np.int64)
        sreg[inv] = reg
        if not (sreg[inv] == reg).all():
            raise SystemExit("a sub-community crosses two regions (the cut graph has an edge between regions)")
        nsub = np.bincount(sreg, minlength=t.size)
        split = nsub[sreg] > 1
        # children, ranked inside each region by (size desc, label)
        sel = np.nonzero(split)[0]
        o = np.lexsort((us[sel], -cnt[sel], sreg[sel]))
        sel = sel[o]
        # rec: a split that needed a resolution above the region's own is a subdivision
        kind = np.where(mult[sreg[sel]] > 1.0, SUBDIV, COMM).astype(np.uint8)
        newid = np.full(len(us), -1, np.int64)
        if len(sel):
            before = t.size
            ids = t.add(sreg[sel], COMM, cnt[sel], t.role[sreg[sel]] + 1)
            t.kind[before:] = kind
            newid[sel] = ids
        moved = newid[inv] >= 0
        cur[idx[moved]] = newid[inv][moved]
        # indivisible regions: leaves; above kmax, rec may retry at 4x (--retries, default 0:
        # these are star-like regions where a higher resolution only peels off single nodes),
        # nest keeps them at its next, higher gamma
        indiv = np.nonzero((nsub == 1) & active)[0]
        big = indiv[t.nn[indiv] > a.kmax]
        done[indiv[t.nn[indiv] <= a.kmax]] = True
        if a.method == "rec":
            stuck[big] += 1
            mult[big] *= 4.0
            done[big[stuck[big] > a.retries]] = True
        else:
            stuck[big] += 1  # the next attempt is one resolution step finer
            done[big[stuck[big] > a.nest_stuck]] = True
        rounds.append({"round": r, "active_regions": na, "active_nodes": int(len(idx)), "cut_entries": dc["entries"],
                       "cutgraph_s": dc["seconds"], "cutgraph_wall_s": dc["_wall_s"], "leiden_s": dl["time_s"], "leiden_wall_s": dl["_wall_s"],
                       "passes": dl.get("n_passes"), "subcommunities": int(len(us)), "regions_split": int((nsub > 1).sum()), "indivisible": int(len(indiv)),
                       "indivisible_above_kmax": int(len(big)), "leiden_anon_peak_kb": dl.get("rss_anon_peak_kb"),
                       "cutgraph_peak_rss_bytes": dc.get("peak_rss_bytes")})
        log(rounds[-1])
        del s, reg, us, inv, cnt, sreg, nsub, split, sel, o, kind, newid, moved, idx, isleaf_now, act_u
    t.leaf = cur
    if uprec["built"]:
        t.collapse_single()  # a super-community holding one community of T (roles keep T)
    return t, rounds, uprec


# ------------------------------------------------------------------------------------- build
def cmd_build(a):
    os.makedirs(a.work, exist_ok=True)
    t0 = time.time()
    n = store_n(a.store)
    meta = {"method": a.method, "F": a.F, "leaf_max": a.leaf_max, "kmax": a.kmax, "store": a.store}
    T = np.fromfile(os.path.join(a.work, "plain.memb.u32"), np.uint32).astype(np.int64) if a.method == "capafter" else None
    post = True
    if a.method in ("capin", "capin+agg"):
        hdir = os.path.join(a.work, f"capin_it{a.iterations}.hier")
        d = m102(["leiden", "run", "--store", a.store] + leiden_args(a) + ["--cap", a.F, "--max-passes", 20, "--min-ratio", 3, "--iterations", a.iterations, "--out-hier", hdir])
        meta["solve"] = {k: d.get(k) for k in ["time_s", "modularity", "communities", "level_sizes", "rss_anon_peak_kb", "hash_levels"]}
        t = levels_to_tree(n, read_levels(hdir))
        post = a.method == "capin+agg"
    elif a.method == "sat":
        t = levels_to_tree(n, read_levels(os.path.join(a.work, "plain.hier")))
    elif a.method == "capafter":
        lab = os.path.join(a.work, "capafter.lab.u32")
        T.astype(np.uint32).tofile(lab)
        cut = os.path.join(a.work, "capafter.cut")
        dc = m102(["cutgraph", "--store", a.store, "--out", cut, "--labels", lab, "--selfloop", "--threads", a.threads])
        hdir = os.path.join(a.work, "capafter.hier")
        d = m102(["leiden", "run", "--store", cut] + leiden_args(a) + ["--cap", a.F, "--max-passes", 20, "--min-ratio", 3, "--out-hier", hdir, "--no-eval"])
        meta["solve"] = {"cutgraph_s": dc["seconds"], "time_s": d["time_s"], "level_sizes": d["level_sizes"], "rss_anon_peak_kb": d.get("rss_anon_peak_kb")}
        lv = read_levels(hdir)
        flat = lv[0].copy()
        for l in lv[1:]:
            flat = l[flat]
        top = np.zeros(int(flat.max()) + 1, np.int64)
        top[flat] = T
        assert (top[flat] == T).all(), "a capped unit crosses two communities of T"
        _, Td = np.unique(top, return_inverse=True)
        t = levels_to_tree(n, lv + [Td])
    else:
        t, rounds, uprec = recursive(a, n)
        meta["rounds"] = rounds
        meta["upward"] = uprec
        # the time split of the shaping (seconds; the rest of build_s is the driver's own work)
        meta["split"] = {"rounds": len(rounds), "cutgraph_s": sum(r["cutgraph_wall_s"] for r in rounds),
                         "leiden_s": sum(r["leiden_s"] for r in rounds), "leiden_wall_s": sum(r["leiden_wall_s"] for r in rounds),
                         "upward_s": uprec["seconds"], "upward_built": uprec["built"], "cover90_T": uprec["cover90_T"],
                         "max_round_leiden_anon_peak_kb": max([r["leiden_anon_peak_kb"] or 0 for r in rounds] or [0]),
                         "max_upward_leiden_anon_peak_kb": max([x["anon_peak_kb"] or 0 for x in uprec["tries"]] or [0])}
    meta["tree_nodes_before_post"] = int(t.size)
    tp = time.time()
    if post:
        meta["post"] = bound_fanout(t, a.F, a.kmax)
    if "split" in meta:
        meta["split"]["post_s"] = time.time() - tp
    meta["build_s"] = time.time() - t0
    # peak resident sizes (KiB): the driver itself, and the largest m102 it ran; the scope's
    # anonymous peak (both together) is what the harness records
    meta["driver_maxrss_kb"] = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    meta["children_maxrss_kb"] = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
    meta["variant"] = a.method + a.tag
    out = os.path.join(a.work, f"tree_{a.method}{a.tag}.npz")
    t.save(out, meta)
    log("built", out, "nodes", t.size, "in", round(meta["build_s"], 1), "s")
    print(json.dumps({"tree": out, **{k: v for k, v in meta.items() if k != "rounds"}}))


# ------------------------------------------------------------------------------------- metrics
def iter_depths(t, dep=None):
    """Deepest first: (d, node -> its region at depth d, or its leaf if the leaf is shallower)."""
    dep = t.depth() if dep is None else dep
    cur = t.leaf.copy()
    for d in range(int(dep.max()), 0, -1):
        deep = dep[cur] > d
        while deep.any():
            cur[deep] = t.par[cur[deep]]
            deep = dep[cur] > d
        yield d, cur.copy()


def depth_memberships(t, max_depth=None):
    dep = t.depth()
    return {d: m for d, m in iter_depths(t, dep) if max_depth is None or d <= max_depth}, dep


def role_membership(t, R):
    """node -> the deepest region of its path whose role is in 0..R (aggregates, role -1, are
    skipped: they are resolved to the communities they hold). R = 2 gives T; R = 1 the super
    level (the root where there is none); R = 2 + k the k-th split below T (a shallower leaf
    where the path ends before)."""
    cur = t.leaf.copy()
    while True:
        bad = (t.role[cur] > R) | (t.role[cur] < 0)
        if not bad.any():
            return cur
        cur[bad] = t.par[cur[bad]]


def dist(x):
    x = np.asarray(x)
    if len(x) == 0:
        return None
    return {"count": int(len(x)), "median": float(np.median(x)), "p95": float(np.percentile(x, 95)), "max": int(x.max()),
            "share_parents_4_64": float(((x >= 4) & (x <= 64)).mean()), "share_children_in_parents_4_64": float(x[(x >= 4) & (x <= 64)].sum() / x.sum()),
            "share_parents_2_3": float(((x >= 2) & (x <= 3)).mean())}


def cmd_metrics(a):
    t, meta = Tree.load(a.tree)
    n = len(t.leaf)
    work = os.path.dirname(a.tree)
    order, start = t.children_index()
    nch = np.diff(start)
    dep = t.depth()
    internal = nch > 0
    isleaf = ~internal
    res = {"tree": a.tree, "method": meta.get("variant", meta["method"]), "graph": a.graph, "n": n, "tree_nodes": int(t.size),
           "depth_max": int(dep.max()), "meta": meta}
    # fan-out of internal nodes per depth; node count of leaf communities
    res["fanout_by_depth"] = {int(d): dist(nch[internal & (dep == d)]) for d in range(int(dep.max())) if (internal & (dep == d)).any()}
    res["fanout_all"] = dist(nch[internal])
    res["leaf_nodes"] = dist(t.nn[isleaf])
    res["leaf_over_kmax"] = {"leaves": int((isleaf & (t.nn > a.kmax)).sum()), "nodes": int(t.nn[isleaf & (t.nn > a.kmax)].sum())}
    res["kinds"] = {KIND_NAMES[k]: int((t.kind == k).sum()) for k in KIND_NAMES}
    # derived share: graph nodes whose path to the root passes an aggregate / a subdivision
    derived = np.zeros(t.size, bool)
    derived[(t.kind == AGG) | (t.kind == SUBDIV)] = True
    anyd = derived.copy()
    for _ in range(int(dep.max())):
        anyd = anyd | anyd[np.maximum(t.par, 0)] & (t.par >= 0)
    aggpath = np.zeros(t.size, bool)
    aggpath[t.kind == AGG] = True
    for _ in range(int(dep.max())):
        aggpath = aggpath | aggpath[np.maximum(t.par, 0)] & (t.par >= 0)
    res["share_nodes_under_derived"] = float(anyd[t.leaf].mean())
    res["share_nodes_under_aggregate"] = float(aggpath[t.leaf].mean())
    # Per depth (what is drawn at that depth) and per role (super level, T, T+1, T+2, whatever
    # their depths): modularity and conductance through m102 regionstats. A depth is measured as
    # drawn (an aggregate counts as one region) and, where it has aggregates, with each aggregate
    # resolved to the communities it holds; a role level never contains aggregates. One
    # membership in memory at a time (deepest first: the resolved one needs the one below).
    isagg = t.kind == AGG
    dense, triples, below = {}, [], None

    def emit(key, mm, name):
        u, inv = np.unique(mm, return_inverse=True)
        dense[key] = u
        f = os.path.join(work, f"metrics.{name}.u32")
        inv.astype(np.uint32).tofile(f)
        triples.append((key, f, os.path.join(work, f"metrics.{name}")))

    for d, m in iter_depths(t, dep):
        r = m if below is None else np.where(isagg[m], below, m)
        below = r
        if d > a.max_depth:
            continue
        emit(("d", d, 0), m, f"d{d}")
        if isagg[m].any():
            emit(("d", d, 1), r, f"d{d}r")
        del m
    del below
    for R in (1, 2, 3, 4):
        if (t.role == R).any():
            emit(("r", R, 0), role_membership(t, R), f"role{R}")
    triples.sort()
    jobs, files, outs = [x[0] for x in triples], [x[1] for x in triples], [x[2] for x in triples]
    p = subprocess.run([M102, "regionstats", "--store", a.store, "--memb", ",".join(files), "--out", ",".join(outs), "--threads", str(a.threads)], capture_output=True, text=True, check=True)
    lines = [json.loads(l) for l in p.stdout.splitlines() if l.startswith("{")]
    per, per_role = {}, {}
    for f, o, key, js in zip(files, outs, jobs, lines):
        typ, d, rv = key
        it = np.fromfile(o + ".internal.u64", np.uint64).astype(np.float64)
        vo = np.fromfile(o + ".volume.u64", np.uint64).astype(np.float64)
        for x in (f, o + ".internal.u64", o + ".volume.u64"):
            os.remove(x)
        if typ == "d" and rv:
            per[d]["modularity_aggregates_resolved"] = js["modularity"]
            per[d]["regions_aggregates_resolved"] = js["regions"]
            continue
        m2 = vo.sum()
        cut = vo - 2 * it
        den = np.minimum(vo, m2 - vo)
        ok = den > 0
        phi = np.where(ok, cut / np.where(ok, den, 1), 0.0)
        ids = dense[key]
        kinds = t.kind[ids]
        nn = t.nn[ids].astype(np.float64)
        comm = (kinds == COMM) | (kinds == SUBDIV)
        row = {"regions": int(len(ids)), "modularity": js["modularity"],
               "conductance_median": float(np.median(phi[ok])) if ok.any() else None,
               "conductance_node_weighted": float((phi * nn).sum() / nn.sum()),
               "conductance_node_weighted_communities": float((phi[comm] * nn[comm]).sum() / max(nn[comm].sum(), 1)),
               "share_nodes_in_aggregates": float(nn[kinds == AGG].sum() / n), "share_nodes_in_subdivisions": float(nn[kinds == SUBDIV].sum() / n),
               "share_nodes_in_leaves": float(nn[isleaf[ids]].sum() / n)}
        if typ == "d":
            per[d] = row
        else:
            per_role[d] = row
    res["by_depth"] = per
    res["by_role"] = {{1: "super", 2: "T", 3: "T+1", 4: "T+2"}[k]: v for k, v in per_role.items()}
    res["modularity_max_over_depths"] = max(v["modularity"] for v in per.values())
    res["modularity_max_resolved"] = max(v.get("modularity_aggregates_resolved", v["modularity"]) for v in per.values())
    if a.plain_json:
        res["plain_modularity"] = json.load(open(a.plain_json))["modularity"]
    json.dump(res, open(a.out, "w"), indent=1)
    # one flat line for the benchmark harness (JSON pointers)
    qr = lambda d: per[d].get("modularity_aggregates_resolved", per[d]["modularity"]) if d in per else None
    qd = lambda d: per[d]["modularity"] if d in per else None
    rl = lambda R, k: per_role[R][k] if R in per_role else None
    fa, lf = res["fanout_all"], res["leaf_nodes"]
    print(json.dumps({"method": res["method"], "graph": a.graph, "tree_nodes": res["tree_nodes"], "depth_max": res["depth_max"],
                      "fanout_median": fa["median"], "fanout_p95": fa["p95"], "fanout_max": fa["max"], "share_parents_4_64": fa["share_parents_4_64"],
                      "share_parents_2_3": fa["share_parents_2_3"], "leaf_nodes_median": lf["median"], "leaf_nodes_max": lf["max"],
                      "nodes_in_leaves_over_kmax": res["leaf_over_kmax"]["nodes"], "share_nodes_under_aggregate": res["share_nodes_under_aggregate"],
                      "share_nodes_under_derived": res["share_nodes_under_derived"], "modularity_plain": res.get("plain_modularity"),
                      "modularity_d1_drawn": qd(1), "modularity_d1": qr(1), "modularity_d2_drawn": qd(2), "modularity_d2": qr(2),
                      "modularity_super": rl(1, "modularity"), "modularity_T": rl(2, "modularity"), "modularity_T1": rl(3, "modularity"), "modularity_T2": rl(4, "modularity"),
                      "conductance_T": rl(2, "conductance_node_weighted"), "conductance_T1": rl(3, "conductance_node_weighted"), "conductance_T2": rl(4, "conductance_node_weighted"),
                      "regions_super": rl(1, "regions"), "regions_T": rl(2, "regions"), "regions_T1": rl(3, "regions"),
                      "modularity_best": res["modularity_max_resolved"],
                      "conductance_d1": per[1]["conductance_node_weighted_communities"], "conductance_d2": per[2]["conductance_node_weighted_communities"] if 2 in per else None,
                      "regions_d1": per[1]["regions"]}))


# ------------------------------------------------------------------------------------- export
def cmd_export(a):
    """gmap hierarchy: level k = the tree nodes of height k, padded with one-child chains where a
    child is more than one level below its parent (gmap collapses those chains). Per level, kinds
    in level_k.kind.u8 and, for aggregates, the number of communities in level_k.count.u32."""
    t, meta = Tree.load(a.tree)
    n = len(t.leaf)
    h = t.height()
    H = int(h[0])
    os.makedirs(a.out, exist_ok=True)
    v = np.arange(1, t.size)
    lo = h[v]
    hi = h[t.par[v]] - 1
    # unit (v, k) for k in lo..hi; unit ids per level ordered by v
    unit_index = {}
    sizes = []
    for k in range(H):
        sel = v[(lo <= k) & (hi >= k)]
        unit_index[k] = sel
        sizes.append(len(sel))
    pos = {}
    for k in range(H):
        p = np.full(t.size, -1, np.int64)
        p[unit_index[k]] = np.arange(len(unit_index[k]))
        pos[k] = p
    # level 0: graph node -> unit (leaf, 0)
    l0 = pos[0][t.leaf]
    assert (l0 >= 0).all()
    l0.astype(np.uint32).tofile(os.path.join(a.out, "level_0.u32"))
    for k in range(H):
        sel = unit_index[k]
        t.kind[sel].astype(np.uint8).tofile(os.path.join(a.out, f"level_{k}.kind.u8"))
        t.ncomm[sel].astype(np.uint32).tofile(os.path.join(a.out, f"level_{k}.count.u32"))
        if k + 1 < H:
            # parent unit at level k+1: (v, k+1) if v continues, else (par[v], k+1)
            cont = hi[sel - 1] >= k + 1
            tgt = np.where(cont, sel, t.par[sel])
            pm = pos[k + 1][tgt]
            assert (pm >= 0).all()
            pm.astype(np.uint32).tofile(os.path.join(a.out, f"level_{k + 1}.u32"))
    json.dump({"graph": a.graph, "n": n, "levels": sizes, "method": meta["method"], "files": "level_k.u32 parent maps (gmap); level_k.kind.u8 0 root 1 community 2 aggregate 3 subdivision; level_k.count.u32 communities inside an aggregate"},
              open(os.path.join(a.out, "hier.json"), "w"))
    print(json.dumps({"out": a.out, "levels": sizes}))


# ------------------------------------------------------------------------------------- compare
def ari_nmi(x, y):
    """Adjusted Rand index and NMI (arithmetic) between two labelings of the same nodes."""
    _, xi = np.unique(x, return_inverse=True)
    _, yi = np.unique(y, return_inverse=True)
    n = len(x)
    pair = xi.astype(np.int64) * (int(yi.max()) + 1) + yi
    _, c = np.unique(pair, return_counts=True)
    a = np.bincount(xi).astype(np.float64)
    b = np.bincount(yi).astype(np.float64)
    c = c.astype(np.float64)
    comb = lambda z: (z * (z - 1) / 2).sum()
    sc, sa, sb, tot = comb(c), comb(a), comb(b), n * (n - 1) / 2
    exp = sa * sb / tot
    ari = (sc - exp) / (0.5 * (sa + sb) - exp) if (0.5 * (sa + sb) - exp) != 0 else 1.0
    h = lambda p: -(p[p > 0] * np.log(p[p > 0])).sum()
    hx, hy = h(a / n), h(b / n)
    # mutual information from the contingency counts
    _, cu = np.unique(pair, return_inverse=True)
    ux, uy = np.unique(pair) // (int(yi.max()) + 1), np.unique(pair) % (int(yi.max()) + 1)
    mi = (c / n * np.log(c * n / (a[ux] * b[uy]))).sum()
    nmi = 2 * mi / (hx + hy) if hx + hy > 0 else 1.0
    return float(ari), float(nmi)


def cmd_compare(a):
    """Agreement of two trees level by level, levels matched by role (super, T, T+1, T+2), with
    aggregates resolved (role levels never contain them). --mask: a u32 degree file (cutgraph
    --out-deg of the perturbed graph); nodes of degree 0 there are left out."""
    (ta, ma), (tb, mb) = Tree.load(a.tree[0]), Tree.load(a.tree[1])
    keep = np.fromfile(a.mask, np.uint32) > 0 if a.mask else None
    sup = lambda m: m.get("upward", {}).get("built")
    out = {"a": a.tree[0], "b": a.tree[1], "nodes_compared": int(keep.sum()) if keep is not None else len(ta.leaf),
           "nodes_left_out": int((~keep).sum()) if keep is not None else 0,
           "super_built_a": sup(ma), "super_built_b": sup(mb),
           "cover90_T_a": ma.get("upward", {}).get("cover90_T"), "cover90_T_b": mb.get("upward", {}).get("cover90_T"), "by_role": {}}
    for R, name in ((1, "super"), (2, "T"), (3, "T+1"), (4, "T+2")):
        if not ((ta.role == R).any() and (tb.role == R).any()):
            continue
        x, y = role_membership(ta, R), role_membership(tb, R)
        if keep is not None:
            x, y = x[keep], y[keep]
        ari, nmi = ari_nmi(x, y)
        out["by_role"][name] = {"ari": ari, "nmi": nmi, "regions_a": int(len(np.unique(x))), "regions_b": int(len(np.unique(y)))}
    if a.out:
        json.dump(out, open(a.out, "w"), indent=1)
    print(json.dumps(out))


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("cmd", choices=["solve", "build", "metrics", "export", "compare"])
    ap.add_argument("--store")
    ap.add_argument("--work")
    ap.add_argument("--graph", default="")
    ap.add_argument("--method", choices=["capin", "capin+agg", "sat", "capafter", "rec", "nest", "rec-up", "nest-up"],
                    help="rec-up / nest-up: rec / nest with --up on (tree file tree_rec-up.npz ...)")
    ap.add_argument("--threads", type=int, default=16)
    ap.add_argument("--ooc", action="store_true", help="m102 out of core (sequential aggregation, spilled levels)")
    ap.add_argument("--budget-gb", type=float, help="m102 --budget-gb (the driver's own arrays are outside it)")
    ap.add_argument("--F", type=int, default=64, help="target maximum fan-out")
    ap.add_argument("--leaf-max", type=int, default=64, help="rec/nest: regions above this many nodes are split again")
    ap.add_argument("--kmax", type=int, default=384, help="indivisible regions up to this size stay leaves (gmap's kmax)")
    ap.add_argument("--retries", type=int, default=0, help="rec: retries (resolution x4) for indivisible regions above kmax")
    ap.add_argument("--nest-stuck", type=int, default=8, help="nest: rounds an indivisible region above kmax stays active (gamma x beta each)")
    ap.add_argument("--round-flags", default="", help="rec/nest: extra m102 flags for every round's Leiden run")
    ap.add_argument("--mask", help="compare: degree file of the perturbed graph (degree-0 nodes are left out)")
    ap.add_argument("--max-rounds", type=int, default=40)
    ap.add_argument("--beta", type=float, default=2.0, help="nest: gamma = beta^round")
    ap.add_argument("--iterations", type=int, default=1, help="capin: Leiden iterations")
    ap.add_argument("--no-hier", action="store_true", help="solve: no natural hierarchy (only the method sat reads it)")
    ap.add_argument("--tag", default="", help="suffix of the tree file name (variants of one method)")
    ap.add_argument("--up", choices=["auto", "on", "off"], default="auto", help="rec/nest: super-communities above T (auto: when T needs more than F-1 communities to cover 90%% of the nodes)")
    ap.add_argument("--tree", action="append")
    ap.add_argument("--out")
    ap.add_argument("--plain-json")
    ap.add_argument("--max-depth", type=int, default=12)
    a = ap.parse_args()
    if a.method in ("rec-up", "nest-up"):
        a.method, a.up, a.tag = a.method[:-3], "on", "-up"
    if a.tree and len(a.tree) == 1:
        a.tree = a.tree[0]
    {"solve": cmd_solve, "build": cmd_build, "metrics": cmd_metrics, "export": cmd_export, "compare": cmd_compare}[a.cmd](a)


if __name__ == "__main__":
    main()
