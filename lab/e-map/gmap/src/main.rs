mod cells;
mod common;
mod embed;
mod hier;
mod layout;
mod render;
mod stress;
mod tiles;
fn main() {
    let cmd = std::env::args().nth(1).unwrap_or_default();
    if let Ok(t) = std::env::var("GMAP_THREADS") {
        rayon::ThreadPoolBuilder::new().num_threads(t.parse().unwrap()).build_global().unwrap();
    }
    match cmd.as_str() {
        "layout" => layout::run(),
        "render" => render::run(),
        "hier" => hier::run(),
        "tiles" => tiles::run(),
        "embed" => embed::run_embed(),
        "eval" => embed::run_eval(),
        _ => eprintln!("usage: gmap <layout|render|embed|eval|tiles> --key value ..."),
    }
}
