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

**Done when.** Speed-up measured on lj/orkut and Friendster-size layouts on integrated and discrete GPUs, with a go/no-go.

**Depends on** {{#m5-01-layout}}
