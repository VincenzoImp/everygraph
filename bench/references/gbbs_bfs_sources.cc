// SPDX-License-Identifier: Apache-2.0 OR MIT
//
// GBBS's BFS (benchmarks/BFS/NonDeterministicBFS) from each source of a file, in one
// process, so that the graph is read once for the 64 sources of
// bench/specs/references.toml. The searches are GBBS's own `BFS(G, src)`; only the loop
// is ours. Prints one "### Source Time: s" line per source and their mean as GBBS's
// "### Running Time".
//
// Built inside the GBBS tree with its makefile rule (copied next to BFS.cc).
// Usage: BFS_sources -s -b -sources FILE -rounds 1 GRAPH.adj

#include <fstream>
#include <vector>

#include "BFS.h"

namespace gbbs {

template <class Graph>
double BFS_sources_runner(Graph& G, commandLine P) {
  std::ifstream in(P.getOptionValue("-sources", ""));
  std::vector<uintE> sources;
  for (uintE s; in >> s;)
    sources.push_back(s);
  if (sources.empty()) {
    std::cerr << "no sources: pass -sources FILE" << std::endl;
    exit(2);
  }
  std::cout << "### Application: BFS from " << sources.size() << " sources" << std::endl;
  std::cout << "### Threads: " << num_workers() << std::endl;
  std::cout << "### n: " << G.n << " m: " << G.m << std::endl;
  double total = 0;
  for (uintE src : sources) {
    timer t;
    t.start();
    auto parents = BFS(G, src);
    double s = t.stop();
    std::cout << "### Source Time: " << s << std::endl;
    total += s;
  }
  double mean = total / sources.size();
  std::cout << "### Running Time: " << mean << std::endl;
  return mean;
}

}  // namespace gbbs

generate_symmetric_main(gbbs::BFS_sources_runner, false);
