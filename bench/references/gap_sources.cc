// SPDX-License-Identifier: Apache-2.0 OR MIT
//
// Prints the first K sources that the GAP Benchmark Suite's own BFS and SSSP pick on a
// graph, one per line: SourcePicker (src/benchmark.h) with GAP's fixed seed, which
// draws node ids uniformly and skips nodes of degree 0. Every tool of
// bench/specs/references.toml starts from this list, so all run the same 64 searches
// that `bfs -n 64` and `sssp -n 64` run (comparison protocol rule 3,
// plan/issues/m8-03-public-benchmarks.md).
//
// Built against GAP's sources: g++ -O3 -fopenmp -I gapbs/src gap_sources.cc
// Usage: gap_sources -f GRAPH.sg -n K

#include <iostream>

#include "benchmark.h"
#include "builder.h"
#include "command_line.h"
#include "graph.h"

int main(int argc, char* argv[]) {
  CLApp cli(argc, argv, "GAP's source list");
  if (!cli.ParseArgs())
    return -1;
  Builder b(cli);
  Graph g = b.MakeGraph();
  SourcePicker<Graph> sp(g, cli.start_vertex());
  for (int i = 0; i < cli.num_trials(); i++)
    std::cout << sp.PickNext() << '\n';
  return 0;
}
