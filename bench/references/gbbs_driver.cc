// SPDX-License-Identifier: Apache-2.0 OR MIT
//
// GBBS's kernels, and ConnectIt's union-find, as bench/specs/references.toml runs them:
// the library calls of GBBS's own mains (benchmarks/*/*.cc), with one loop and one
// output for every kernel. Each round prints "### Round Time: s"; BFS runs once from
// each source of -sources and prints one "### Source Time: s" per source. -dump FILE
// writes the last result as little-endian words: BFS parents of the first source and
// component labels as uint32 (uintE), core numbers as uint32, PageRank as float64. The
// harness compares it with Everygraph's (bench/runners/run.py validate).
//
// Built inside the GBBS tree with its makefile rule, next to WorkEfficientSDB14's main
// (which links the connectivity objects ConnectIt needs).
// Usage: gbbs_driver -s -b -kernel bfs|cc|connectit|kcore|pr|tc [-rounds R]
//            [-sources FILE] [-dump FILE] GRAPH.adj

#include <fstream>
#include <string>
#include <vector>

#include "benchmarks/BFS/NonDeterministicBFS/BFS.h"
#include "benchmarks/Connectivity/ConnectIt/framework.h"
#include "benchmarks/Connectivity/WorkEfficientSDB14/Connectivity.h"
#include "benchmarks/KCore/JulienneDBS17/KCore.h"
#include "benchmarks/PageRank/PageRank.h"
#include "benchmarks/TriangleCounting/ShunTangwongsan15/Triangle.h"

namespace gbbs {

template <class Seq>
void dump(const std::string& path, const Seq& s) {
  if (path.empty()) return;
  std::ofstream out(path, std::ios::binary);
  out.write(reinterpret_cast<const char*>(s.begin()), s.size() * sizeof(s[0]));
}

template <class F>
double timed(F f) {
  timer t;
  t.start();
  f();
  return t.stop();
}

template <class Graph>
double Driver_runner(Graph& G, commandLine P) {
  std::string kernel = P.getOptionValue("-kernel", "");
  std::string out = P.getOptionValue("-dump", "");
  size_t rounds = P.getOptionLongValue("-rounds", 5);
  std::cout << "### Kernel: " << kernel << " Threads: " << num_workers()
            << " n: " << G.n << " m: " << G.m << std::endl;
  if (kernel == "bfs") {
    std::ifstream in(P.getOptionValue("-sources", ""));
    std::vector<uintE> sources;
    for (uintE s; in >> s;) sources.push_back(s);
    if (sources.empty()) {
      std::cerr << "bfs needs -sources FILE" << std::endl;
      exit(2);
    }
    for (size_t i = 0; i < sources.size(); i++) {
      sequence<uintE> parents;
      double s = timed([&] { parents = BFS(G, sources[i]); });
      std::cout << "### Source Time: " << s << std::endl;
      if (i == 0) dump(out, parents);
    }
    return 0;
  }
  for (size_t r = 0; r < rounds; r++) {
    bool last = r + 1 == rounds;
    double s;
    if (kernel == "cc") {
      sequence<parent> labels;
      s = timed([&] { labels = workefficient_cc::CC(G, 0.2, false, false); });
      if (last) dump(out, labels);
    } else if (kernel == "connectit") {
      // The variant ConnectIt's paper reports fastest: UF-Rem-CAS with k-out sampling,
      // FindAtomicSplit and SplitAtomicOne.
      sequence<parent> labels;
      s = timed([&] {
        labels = connectit::run_uf_alg<Graph, sample_kout, find_atomic_split,
                                       unite_rem_cas, split_atomic_one>(G, P);
      });
      if (last) dump(out, labels);
    } else if (kernel == "kcore") {
      sequence<uintE> cores;
      s = timed([&] { cores = KCore(G, 16); });
      if (last) dump(out, cores);
    } else if (kernel == "pr") {
      sequence<double> ranks;
      s = timed([&] { ranks = PageRank(G, 0.0, 20); });
      if (last) dump(out, ranks);
    } else if (kernel == "tc") {
      size_t count = 0;
      auto f = [&](uintE, uintE, uintE) {};
      s = timed([&] { count = Triangle(G, f, "degree", P); });
      if (last) std::cout << "### Triangles: " << count << std::endl;
    } else {
      std::cerr << "unknown -kernel " << kernel << std::endl;
      exit(2);
    }
    std::cout << "### Round Time: " << s << std::endl;
  }
  return 0;
}

}  // namespace gbbs

generate_symmetric_once_main(gbbs::Driver_runner, false);
