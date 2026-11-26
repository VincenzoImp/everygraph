// SPDX-License-Identifier: Apache-2.0 OR MIT
//
// One run of a GAP Benchmark Suite kernel, its result written to the file named by
// $GAP_DUMP as little-endian words, for bench/runners/run.py validate: BFS parents and
// component labels as int32 (NodeID), SSSP distances as WeightT, PageRank as ScoreT.
// BFS and SSSP start from the first source of GAP's SourcePicker, the first of the 64
// that every tool runs. The kernel is GAP's own function from its source file, which
// this file includes with its main() renamed; GAP's timed runs use GAP's own binaries.
//
// Built against GAP's sources, one binary per kernel:
//   g++ -std=c++11 -O3 -fopenmp -I gapbs/src -DKERNEL='"cc.cc"' -DGAP_CC gap_dump.cc
// (GAP_BFS, GAP_CC, GAP_SSSP, GAP_PR).
// Usage: GAP_DUMP=FILE gap_dump_cc -f GRAPH.sg

#define main gap_own_main
#include KERNEL
#undef main

#include <cstdlib>
#include <fstream>
#include <iostream>

template <class V>
void dump(const V& v) {
  const char* path = std::getenv("GAP_DUMP");
  if (path == nullptr) {
    std::cerr << "set GAP_DUMP to the output file" << std::endl;
    std::exit(2);
  }
  std::ofstream out(path, std::ios::binary);
  out.write(reinterpret_cast<const char*>(v.begin()), v.size() * sizeof(v[0]));
}

int main(int argc, char* argv[]) {
#if defined(GAP_SSSP)
  CLDelta<WeightT> cli(argc, argv, "dump");
  if (!cli.ParseArgs()) return -1;
  WeightedBuilder b(cli);
  WGraph g = b.MakeGraph();
  SourcePicker<WGraph> sp(g, cli.start_vertex());
  dump(DeltaStep(g, sp.PickNext(), cli.delta()));
#elif defined(GAP_PR)
  CLPageRank cli(argc, argv, "dump", 0, 20);
  if (!cli.ParseArgs()) return -1;
  Builder b(cli);
  Graph g = b.MakeGraph();
  dump(PageRankPull(g, cli.max_iters(), cli.tolerance()));
#else
  CLApp cli(argc, argv, "dump");
  if (!cli.ParseArgs()) return -1;
  Builder b(cli);
  Graph g = b.MakeGraph();
#if defined(GAP_BFS)
  SourcePicker<Graph> sp(g, cli.start_vertex());
  dump(DOBFS(g, sp.PickNext()));
#elif defined(GAP_CC)
  dump(Afforest(g));
#endif
#endif
  return 0;
}
