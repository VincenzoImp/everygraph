---
title: Optional GPU acceleration for layout
milestone: M9 · After v0.1
labels: type:experiment, area:map, P2, risk:medium, size:L
---
**Why.** Layout is already cheap on CPU (lj in 3.3 s), so this is not on the critical path. It becomes interesting for interactive re-layout of a view, and recent GPU layouts report millions of nodes in seconds (vendor claims; one had no code available at the time of the research).

**Scope**
- [ ] Prototype sibling force solves and leaf placement with `wgpu` (Vulkan/Metal/DX12, and WebGPU in the browser)
- [ ] Must produce the same deterministic result class or be clearly marked interactive-only
- [ ] The engine never requires a GPU

**Still deferred (2026-11-26, `docs/research/sota-review-2026-11.md` § 5; the review's arithmetic, nothing measured):**
- A graph larger than RAM is bound by the NVMe: a sequential `O_DIRECT` read of the 14.45 GB Friendster arrays runs at about 7.07 GB/s with 4 threads here (official) (`bench/results/io/20261125T205022Z-21498df39590.jsonl`). A GPU adds nothing.
- A graph in RAM but larger than VRAM crosses PCIe 4 x16, 32 GB/s in theory (EMOGI reached about 12 GB/s on PCIe 3, their machine). That is below this desktop's DRAM bandwidth, which the CPU already uses. Little gain is likely for scans. Integrated laptop GPUs share the CPU's DRAM.
- One case could win clearly: a compressed graph that fits in VRAM. Friendster's compressed store is 10.4 GB (ADR 0029) and an RTX 4090 has 24 GB. It needs a GPU Stream VByte decoder and is an untested hypothesis. It concerns a workstation with a large discrete GPU, not the laptop promise.
- Routes for kernel bodies in Rust: WGSL through wgpu has no f64, so a browser GPU path cannot give our f64 PageRank. CubeCL (kernels written once in Rust, compiled to CUDA, HIP, Metal, Vulkan or WebGPU) is the only route without a second kernel language, and it is pre-1.0 (0.11 pre-releases). Its CPU backend never replaces the product's CPU path.

**Done when.** Speed-up measured on lj/orkut and Friendster-size layouts on integrated and discrete GPUs, with a go/no-go.

**Depends on** {{#m5-01-layout}}
