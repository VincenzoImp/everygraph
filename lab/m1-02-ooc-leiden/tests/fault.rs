//! Review of PR #143, S1: a broken invariant during the sequential aggregation must end the run
//! at once (exit 4, one JSON status line), not leave threads waiting for a bucket that will
//! never be committed. The fault is injected with M102_INJECT_ROW_FAULT; the run gets a small
//! budget so that buckets go to disk and threads wait for memory when the fault fires.
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

fn write_csr(dir: &Path, n: usize, seed: u64) {
    let mut x = seed;
    let mut e: Vec<(u32, u32)> = Vec::new();
    for u in 0..n as u32 {
        for _ in 0..8 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let v = (x % n as u64) as u32;
            if v != u {
                e.push((u, v));
                e.push((v, u));
            }
        }
    }
    e.sort_unstable();
    e.dedup();
    let mut off = vec![0u64; n + 1];
    for &(a, _) in &e {
        off[a as usize + 1] += 1;
    }
    for i in 0..n {
        off[i + 1] += off[i];
    }
    let ob: Vec<u8> = off.iter().flat_map(|x| x.to_le_bytes()).collect();
    let tb: Vec<u8> = e.iter().flat_map(|&(_, b)| b.to_le_bytes()).collect();
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("offsets.u64"), ob).unwrap();
    std::fs::write(dir.join("targets.u32"), tb).unwrap();
}

fn run_with_timeout(mut cmd: Command, limit: Duration) -> (Option<i32>, String, Duration) {
    let t = Instant::now();
    let mut child = cmd.stdout(std::process::Stdio::piped()).spawn().unwrap();
    loop {
        if let Some(st) = child.try_wait().unwrap() {
            let mut out = String::new();
            use std::io::Read;
            child.stdout.take().unwrap().read_to_string(&mut out).unwrap();
            return (st.code(), out, t.elapsed());
        }
        if t.elapsed() > limit {
            let _ = child.kill();
            let _ = child.wait();
            return (None, String::new(), t.elapsed());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn injected_invariant_violation_ends_the_run_at_once() {
    let bin = env!("CARGO_BIN_EXE_m102");
    let root = std::env::temp_dir().join(format!("m102-fault-test-{}", std::process::id()));
    let csr = root.join("csr");
    let store = root.join("store");
    write_csr(&csr, 60_000, 0x2545_F491_4F6C_DD1D);
    let st = Command::new(bin).args(["encode", "--graph", csr.to_str().unwrap(), "--out", store.to_str().unwrap(), "--threads", "4"]).output().unwrap();
    assert!(st.status.success(), "encode failed: {}", String::from_utf8_lossy(&st.stderr));
    let args = |fault: Option<&str>| {
        let mut c = Command::new(bin);
        c.args(["leiden", "run", "--store", store.to_str().unwrap(), "--config", "comp", "--mem", "mmap-lazy", "--threads", "4", "--ooc", "--budget-gb", "0.012", "--spill", root.join("spill").to_str().unwrap()]);
        if let Some(f) = fault {
            c.env("M102_INJECT_ROW_FAULT", f);
        }
        c
    };
    // without the fault the run completes
    let (code, out, _) = run_with_timeout(args(None), Duration::from_secs(120));
    assert_eq!(code, Some(0), "clean run failed: {out}");
    assert!(out.contains("\"status\":\"ok\""));
    // with it, exit 4 at once, with the reason
    let (code, out, took) = run_with_timeout(args(Some("0")), Duration::from_secs(60));
    assert_eq!(code, Some(4), "expected exit 4 within 60 s, got {code:?} after {took:?}: {out}");
    assert!(out.contains("k_equals_row_sum"), "{out}");
    assert!(took < Duration::from_secs(30), "took {took:?}");
    let _ = std::fs::remove_dir_all(&root);
}
