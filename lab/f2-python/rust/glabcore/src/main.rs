use std::time::Instant;
fn main() {
    let dir = std::env::args().nth(1).expect("csr dir");
    let reps: usize = std::env::args().nth(2).map(|s| s.parse().unwrap()).unwrap_or(3);
    let g = glabcore::Csr::open(&dir).unwrap();
    // warm page cache / touch
    let s: u64 = g.degree().iter().map(|&d| d as u64).sum();
    let mut best_pr = f64::MAX; let mut iters = 0; let mut best_cc = f64::MAX; let mut ncc = 0;
    for _ in 0..reps {
        let t = Instant::now(); let (x, it) = g.pagerank(0.85, 20, 0.0); let dt = t.elapsed().as_secs_f64();
        best_pr = best_pr.min(dt); iters = it; std::hint::black_box(x);
        let t = Instant::now(); let l = g.wcc(); let dt = t.elapsed().as_secs_f64();
        best_cc = best_cc.min(dt);
        ncc = l.iter().enumerate().filter(|(i, &x)| *i as u32 == x).count();
    }
    println!("{{\"n\":{},\"entries\":{},\"threads\":{},\"pagerank_s\":{:.4},\"pagerank_iters\":{},\"wcc_s\":{:.4},\"n_components\":{}}}",
        g.n(), s, rayon::current_num_threads(), best_pr, iters, best_cc, ncc);
}
