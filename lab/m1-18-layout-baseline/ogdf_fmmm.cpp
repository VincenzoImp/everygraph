// ogdf_fmmm EDGES.txt OUT.f32 [QUALITY]
// Reads "n m" then "u v w" per undirected edge (0-based), runs OGDF's FMMMLayout (FM^3,
// Hachul and Juenger) with its high-level options, writes n x 2 f32 in input node order.
// QUALITY: 0 GorgeousAndEfficient, 1 BeautifulAndFast (OGDF default), 2 NiceAndIncredibleSpeed.
#include <ogdf/basic/GraphAttributes.h>
#include <ogdf/energybased/FMMMLayout.h>

#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <vector>

using namespace ogdf;

int main(int argc, char** argv) {
	if (argc < 3) {
		std::fprintf(stderr, "usage: ogdf_fmmm EDGES.txt OUT.f32 [QUALITY]\n");
		return 2;
	}
	int quality = argc > 3 ? std::atoi(argv[3]) : 1;
	FILE* f = std::fopen(argv[1], "r");
	long n, m;
	if (!f || std::fscanf(f, "%ld %ld", &n, &m) != 2) return 1;
	auto t0 = std::chrono::steady_clock::now();
	Graph G;
	std::vector<node> nodes(n);
	for (long i = 0; i < n; i++) nodes[i] = G.newNode();
	for (long i = 0; i < m; i++) {
		long u, v;
		float w;
		if (std::fscanf(f, "%ld %ld %f", &u, &v, &w) != 3) return 1;
		G.newEdge(nodes[u], nodes[v]);
	}
	std::fclose(f);
	GraphAttributes GA(G, GraphAttributes::nodeGraphics);
	for (node v : G.nodes) GA.width(v) = GA.height(v) = 1.0;
	auto t1 = std::chrono::steady_clock::now();
	std::fprintf(stderr, "loaded n=%ld m=%ld in %.2f s\n", n, m, std::chrono::duration<double>(t1 - t0).count());
	FMMMLayout fmmm;
	fmmm.useHighLevelOptions(true);
	fmmm.qualityVersusSpeed(quality == 0 ? FMMMOptions::QualityVsSpeed::GorgeousAndEfficient
					: quality == 2 ? FMMMOptions::QualityVsSpeed::NiceAndIncredibleSpeed
								  : FMMMOptions::QualityVsSpeed::BeautifulAndFast);
	fmmm.call(GA);
	auto t2 = std::chrono::steady_clock::now();
	std::vector<float> out(2 * n);
	for (long i = 0; i < n; i++) {
		out[2 * i] = float(GA.x(nodes[i]));
		out[2 * i + 1] = float(GA.y(nodes[i]));
	}
	FILE* o = std::fopen(argv[2], "wb");
	std::fwrite(out.data(), sizeof(float), out.size(), o);
	std::fclose(o);
	std::printf("{\"tool\":\"OGDF FMMMLayout\",\"quality\":%d,\"load_s\":%.2f,\"layout_s\":%.2f}\n", quality,
			std::chrono::duration<double>(t1 - t0).count(), std::chrono::duration<double>(t2 - t1).count());
	return 0;
}
