#![cfg_attr(target_arch = "wasm64", feature(simd_wasm64))]
//! WebAssembly front-end for graphcore. Raw C-ABI exports (usable with or
//! without wasm-bindgen, on wasm32 and wasm64); wasm-bindgen is only pulled in
//! for the threaded build to provide `initThreadPool` (wasm-bindgen-rayon).
//! All state lives in a thread-local of the calling (worker) thread.
use graphcore::*;
use std::cell::RefCell;

#[cfg(feature = "threads")]
pub use wasm_bindgen_rayon::init_thread_pool;

#[derive(Default)]
struct State {
    stream: Option<StreamParser>,
    two: Option<TextTwoPass>,
    edges: Option<EdgeList>,
    csr: Option<Csr>,
    pr: Option<Vec<f64>>,
    cc: Option<Vec<u32>>,
}
thread_local! { static ST: RefCell<State> = RefCell::new(State::default()); }

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    ST.with(|s| f(&mut s.borrow_mut()))
}

// ---- memory helpers: JS allocates buffers inside linear memory, fills them, hands them back.
#[no_mangle]
pub extern "C" fn ga_alloc(bytes: usize) -> *mut u8 {
    let mut v: Vec<u8> = Vec::new();
    if v.try_reserve_exact(bytes).is_err() {
        return std::ptr::null_mut();
    }
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}
#[no_mangle]
pub unsafe extern "C" fn ga_free(p: *mut u8, bytes: usize) {
    drop(Vec::from_raw_parts(p, 0, bytes));
}
/// Allocate a typed buffer of `count` elements of `width` bytes (4 or 8) with
/// the right alignment.
#[no_mangle]
pub extern "C" fn ga_alloc_typed(count: usize, width: u32) -> *mut u8 {
    unsafe fn mk<T>(c: usize) -> *mut u8 {
        let mut v: Vec<T> = Vec::new();
        if v.try_reserve_exact(c).is_err() {
            return std::ptr::null_mut();
        }
        let p = v.as_mut_ptr() as *mut u8;
        std::mem::forget(v);
        p
    }
    unsafe {
        match width {
            8 => mk::<u64>(count),
            _ => mk::<u32>(count),
        }
    }
}

#[no_mangle]
pub extern "C" fn ga_threads() -> u32 {
    par::threads() as u32
}
/// Current linear memory size in bytes (as f64 so it is a JS number on wasm64 too).
#[no_mangle]
pub extern "C" fn ga_mem_bytes() -> f64 {
    #[cfg(target_arch = "wasm32")]
    {
        core::arch::wasm32::memory_size(0) as f64 * 65536.0
    }
    #[cfg(target_arch = "wasm64")]
    {
        core::arch::wasm64::memory_size(0) as f64 * 65536.0
    }
    #[cfg(not(any(target_arch = "wasm32", target_arch = "wasm64")))]
    {
        0.0
    }
}

// ---- ingest
/// Parse a whole text buffer previously obtained from ga_alloc (freed here).
#[no_mangle]
pub unsafe extern "C" fn ga_parse_owned(p: *mut u8, len: usize) -> f64 {
    let buf = Vec::from_raw_parts(p, len, len);
    let el = parse_edges(&buf);
    drop(buf);
    let m = el.len();
    with(|s| s.edges = Some(el));
    m as f64
}
#[no_mangle]
pub extern "C" fn ga_stream_begin(edge_hint: f64) {
    with(|s| s.stream = Some(StreamParser::with_capacity(edge_hint as usize)));
}
/// Push a chunk (borrowed; JS reuses the buffer).
#[no_mangle]
pub unsafe extern "C" fn ga_stream_push(p: *const u8, len: usize) {
    let c = std::slice::from_raw_parts(p, len);
    with(|s| s.stream.as_mut().unwrap().push(c));
}
#[no_mangle]
pub extern "C" fn ga_stream_end() -> f64 {
    with(|s| {
        let el = s.stream.take().unwrap().finish();
        let m = el.len();
        s.edges = Some(el);
        m as f64
    })
}
// two-pass streaming CSR build (input is streamed twice; never resident)
#[no_mangle]
pub extern "C" fn ga_tp_begin() {
    with(|s| s.two = Some(TextTwoPass::new()));
}
#[no_mangle]
pub unsafe extern "C" fn ga_tp_push(p: *const u8, len: usize) {
    let c = std::slice::from_raw_parts(p, len);
    with(|s| s.two.as_mut().unwrap().push(c));
}
#[no_mangle]
pub extern "C" fn ga_tp_next_pass() -> f64 {
    with(|s| {
        let t = s.two.as_mut().unwrap();
        t.next_pass();
        t.edges_seen as f64
    })
}
#[no_mangle]
pub extern "C" fn ga_tp_finish() -> f64 {
    with(|s| {
        let (g, _ids) = s.two.take().unwrap().finish();
        let n = g.n();
        s.csr = Some(g);
        n as f64
    })
}
#[no_mangle]
pub extern "C" fn ga_gen2(n: u32, m: f64, seed: u32) -> f64 {
    let (g, _) = gen_csr_two_pass(n, m as usize, seed as u64);
    let nn = g.n();
    with(|s| s.csr = Some(g));
    nn as f64
}
#[no_mangle]
pub extern "C" fn ga_gen(n: u32, m: f64, seed: u32) -> f64 {
    let el = gen_edges(n, m as usize, seed as u64);
    with(|s| s.edges = Some(el));
    m
}
/// Build CSR from the parsed edge list. Returns n.
#[no_mangle]
pub extern "C" fn ga_build_csr() -> f64 {
    with(|s| {
        let el = s.edges.take().unwrap();
        let (g, _ids) = build_csr_undirected(el);
        let n = g.n();
        s.csr = Some(g);
        n as f64
    })
}
/// Adopt a CSR whose arrays were allocated with ga_alloc_typed and filled by JS.
#[no_mangle]
pub unsafe extern "C" fn ga_load_csr(off: *mut u8, n1: usize, tgt: *mut u8, m: usize) {
    let off64 = Vec::from_raw_parts(off as *mut u64, n1, n1);
    let targets = Vec::from_raw_parts(tgt as *mut u32, m, m);
    let g = Csr::from_u64_offsets(&off64, targets); // u64 -> usize (4 B on wasm32)
    drop(off64);
    with(|s| s.csr = Some(g));
}
#[no_mangle]
pub extern "C" fn ga_entries() -> f64 {
    with(|s| s.csr.as_ref().map(|g| g.entries() as f64).unwrap_or(0.0))
}

// ---- analytics
#[no_mangle]
pub extern "C" fn ga_pagerank(iters: u32) -> f64 {
    with(|s| {
        let g = s.csr.as_ref().unwrap();
        let pr = pagerank(g, iters as usize, 0.85);
        let mx = pr_checksum(&pr).1;
        s.pr = Some(pr);
        mx
    })
}
#[no_mangle]
pub extern "C" fn ga_pr_sum() -> f64 {
    with(|s| pr_checksum(s.pr.as_ref().unwrap()).0)
}
#[no_mangle]
pub extern "C" fn ga_cc() -> f64 {
    with(|s| {
        s.pr = None;
        let g = s.csr.as_ref().unwrap();
        let cc = connected_components(g);
        let (nc, _) = cc_stats(&cc);
        s.cc = Some(cc);
        nc as f64
    })
}
#[no_mangle]
pub extern "C" fn ga_cc_largest() -> f64 {
    with(|s| cc_stats(s.cc.as_ref().unwrap()).1 as f64)
}
#[no_mangle]
pub extern "C" fn ga_reset() {
    with(|s| *s = State::default());
}
