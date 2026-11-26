// SPDX-License-Identifier: Apache-2.0 OR MIT
//
// The UCR parlay codes as bench/specs/references.toml runs them: Parallel-KCore (Liu,
// Dong, Gu and Sun, SIGMOD'25; github.com/ucrparlay/Parallel-KCore) and PASGAL's BFS and
// SSSP (github.com/ucrparlay/PASGAL, MIT). The kernels are the projects' own classes;
// this file is only the loop and the output. Parallel-KCore has no licence: it is
// downloaded and built on this machine only, never committed.
//
// - kcore: `KCore(G).kcore()`, -rounds times; one "### Round Time: s" each.
// - bfs, sssp: one search from each source of -sources; one "### Source Time: s" each.
//   SSSP is PASGAL's rho-stepping (its default) on uint32 costs set to those every tool
//   gets: an integer from 1 to 255 drawn from the undirected pair {min(u, v), max(u, v)}
//   (`pair_costs` of bench/runners/run.py), so the graph stays symmetric and PASGAL keeps
//   its pull step.
// -dump FILE writes the last core numbers, or the distances from the first source, as
// little-endian uint32.
//
// Built with the project's own flags, one binary per project (-DUCR_KCORE or -DUCR_PASGAL),
// against its tree: Parallel-KCore's KCore/ or PASGAL's src/ on the include path.
// Usage: ucr_driver -i GRAPH.bin -k kcore|bfs|sssp [-r ROUNDS] [-q SOURCES] [-o FILE]

#include <unistd.h>

#include <algorithm>
#include <cstdint>
#include <cstdio>
#include <fstream>
#include <string>
#include <vector>

#include "graph.h"
#include "parlay/internal/get_time.h"
#if defined(UCR_KCORE)
#include "kcore.h"
#else
#include "BFS/bfs.h"
#include "SSSP/sssp.h"
#endif

template <class Seq>
void dump(const std::string& path, const Seq& s) {
  if (path.empty()) return;
  std::ofstream out(path, std::ios::binary);
  out.write(reinterpret_cast<const char*>(s.data()), s.size() * sizeof(s[0]));
}

// The cost of the pair {u, v}: `pair_costs` of bench/runners/run.py.
uint32_t cost(uint32_t u, uint32_t v) {
  uint64_t z = ((uint64_t(std::min(u, v)) << 32) | std::max(u, v)) + 0x9E3779B97F4A7C15ULL;
  z = (z ^ (z >> 30)) * 0xBF58476D1CE4E5B9ULL;
  z = (z ^ (z >> 27)) * 0x94D049BB133111EBULL;
  z ^= z >> 31;
  return static_cast<uint32_t>(1 + z % 255);
}

// The graph in the project's own type: unweighted (4 bytes per entry), or with uint32
// costs for SSSP.
template <class G>
G read(const std::string& input, const std::string& kernel) {
  G g;
  g.read_graph(input.c_str());
  g.symmetrized = true;
  printf("### Kernel: %s n: %zu m: %zu\n", kernel.c_str(), g.n, g.m);
  return g;
}

template <class Solver, class Search>
void each(Solver& solver, Search search, const std::vector<uint32_t>& sources,
          const std::string& out) {
  for (size_t i = 0; i < sources.size(); i++) {
    parlay::internal::timer t;
    auto dist = search(solver, sources[i]);
    t.stop();
    printf("### Source Time: %f\n", t.total_time());
    if (i == 0) dump(out, dist);
  }
}

int main(int argc, char* argv[]) {
  std::string input, kernel, sources_path, out;
  int rounds = 5;
  for (int c; (c = getopt(argc, argv, "i:k:r:q:o:")) != -1;) {
    switch (c) {
      case 'i': input = optarg; break;
      case 'k': kernel = optarg; break;
      case 'r': rounds = atoi(optarg); break;
      case 'q': sources_path = optarg; break;
      case 'o': out = optarg; break;
      default: return 2;
    }
  }
#if defined(UCR_KCORE)
  if (kernel != "kcore") return 2;
  auto G = read<Graph<uint32_t, uint64_t>>(input, kernel);
  KCore solver(G);
  parlay::sequence<uint32_t> cores;
  for (int r = 0; r < rounds; r++) {
    parlay::internal::timer t;
    cores = solver.kcore();
    t.stop();
    printf("### Round Time: %f\n", t.total_time());
  }
  dump(out, cores);
#else
  std::vector<uint32_t> sources;
  std::ifstream in(sources_path);
  for (uint32_t s; in >> s;) sources.push_back(s);
  if (sources.empty()) return 2;
  if (kernel == "bfs") {
    auto G = read<Graph<uint32_t, uint64_t>>(input, kernel);
    BFS solver(G);
    each(solver, [](auto& s, uint32_t v) { return s.bfs(v); }, sources, out);
  } else if (kernel == "sssp") {
    auto G = read<Graph<uint32_t, uint64_t, uint32_t>>(input, kernel);
    parlay::parallel_for(0, G.n, [&](size_t u) {
      for (size_t e = G.offsets[u]; e < G.offsets[u + 1]; e++) {
        G.edges[e].w = cost(u, G.edges[e].v);
      }
    });
    G.weighted = true;
    Rho_Stepping solver(G);
    each(solver, [](auto& s, uint32_t v) { return s.sssp(v); }, sources, out);
  } else {
    return 2;
  }
#endif
  return 0;
}
