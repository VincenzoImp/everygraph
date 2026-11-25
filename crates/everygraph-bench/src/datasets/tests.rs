// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Fetcher tests against the local server of `test_support`: ranged download, resume,
//! fallback without ranges, checksum and size failures, atomic placement, `--from-dir`,
//! and the synthetic datasets of the real registry.

use std::time::Duration;

use super::test_support::{ChunkLie, Server, ServerConfig, TempDir, bytes};
use super::*;

fn sha256_of(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex(&Sha256::digest(data))
}

fn options(connections: usize) -> FetchOptions {
    FetchOptions {
        download: DownloadOptions {
            connections,
            retries: 3,
            backoff: Duration::from_millis(5),
            read_timeout: Duration::from_secs(10),
            save_every: Duration::from_millis(1),
            log_every: None,
        },
        from_dir: None,
        allow_unpinned: false,
        replace: false,
    }
}

fn registry_for(url: &str, size: usize, sha256: Option<&str>) -> Registry {
    let sha_line = sha256.map_or(String::new(), |h| format!("sha256 = \"{h}\""));
    let tier = if sha256.is_some() { 1 } else { 3 };
    let unverified = if sha256.is_some() {
        ""
    } else {
        "unverified = [\"sha256\"]"
    };
    Registry::parse(&format!(
        r#"
schema = 1
[[dataset]]
name = "toy"
title = "Toy"
family = "social"
tier = {tier}
directed = false
weighted = false
temporal = false
nodes = 1
edges = 1
format = "bytes"
to_edge_list = "n/a"
licence = "test data"
{unverified}
[[dataset.file]]
name = "toy.bin"
url = "{url}"
size = {size}
{sha_line}
"#
    ))
    .expect("test registry")
}

fn run(root: &DataRoot, registry: &Registry, opts: &FetchOptions) -> Result<Vec<Outcome>> {
    let agent = agent(&opts.download)?;
    let ds = registry.get("toy").expect("toy");
    fetch(root, &agent, ds, opts, &mut std::io::sink())
}

/// The ranges `(first, last)` the server was asked for, probe excluded.
fn range_spans(server: &Server) -> Vec<(u64, u64)> {
    server
        .seen()
        .into_iter()
        .flatten()
        .filter(|r| r != "bytes=0-0")
        .filter_map(|r| {
            let (a, b) = r.strip_prefix("bytes=")?.split_once('-')?;
            Some((a.parse().ok()?, b.parse().ok()?))
        })
        .collect()
}

/// Starts of the ranges the server was asked for, probe excluded, in arrival order.
fn range_starts(server: &Server) -> Vec<u64> {
    range_spans(server).into_iter().map(|(a, _)| a).collect()
}

#[test]
fn a_ranged_parallel_download_is_verified_and_placed_atomically() {
    let data = bytes(4 << 20, 1);
    let server = Server::start(data.clone(), ServerConfig::default());
    let dir = TempDir::new("ranged");
    let root = DataRoot::new(dir.path());
    let registry = registry_for(&server.url(), data.len(), Some(&sha256_of(&data)));
    let out = run(&root, &registry, &options(4)).expect("fetch");
    assert_eq!(out, vec![Outcome::Placed { from: server.url() }]);
    let ds = registry.get("toy").expect("toy");
    let f = &ds.files[0];
    assert_eq!(std::fs::read(root.file_path(ds, f)).expect("read"), data);
    assert_eq!(root.status(ds, f), FileStatus::Verified);
    // Four 1 MiB chunks, each its own request; nothing partial is left behind.
    let mut starts = range_starts(&server);
    starts.sort_unstable();
    assert_eq!(starts, vec![0, 1 << 20, 2 << 20, 3 << 20]);
    assert!(!root.dataset_dir(ds).join(".partial").exists());
}

#[test]
fn a_second_fetch_of_a_verified_file_makes_no_request() {
    let data = bytes(100_000, 2);
    let server = Server::start(data.clone(), ServerConfig::default());
    let dir = TempDir::new("again");
    let root = DataRoot::new(dir.path());
    let registry = registry_for(&server.url(), data.len(), Some(&sha256_of(&data)));
    run(&root, &registry, &options(2)).expect("first fetch");
    let requests = server.seen().len();
    let out = run(&root, &registry, &options(2)).expect("second fetch");
    assert_eq!(out, vec![Outcome::AlreadyVerified]);
    assert_eq!(server.seen().len(), requests);
}

#[test]
fn a_dropped_connection_is_retried_from_where_it_stopped() {
    let data = bytes(3 << 20, 3);
    let server = Server::start(
        data.clone(),
        ServerConfig {
            cut_responses: 2,
            cut_after: 300_000,
            ..ServerConfig::default()
        },
    );
    let dir = TempDir::new("retry");
    let root = DataRoot::new(dir.path());
    let registry = registry_for(&server.url(), data.len(), Some(&sha256_of(&data)));
    run(&root, &registry, &options(1)).expect("fetch survives two drops");
    let ds = registry.get("toy").expect("toy");
    assert_eq!(
        std::fs::read(root.file_path(ds, &ds.files[0])).expect("read"),
        data
    );
    // One connection, one 3 MiB chunk: cut at 300 000, resumed there and cut again at
    // 600 000, resumed there and completed. Resumed, never restarted.
    assert_eq!(range_starts(&server), vec![0, 300_000, 600_000]);
}

#[test]
fn an_interrupted_download_resumes_in_a_later_run() {
    let data = bytes(4 << 20, 4);
    let sha = sha256_of(&data);
    let dir = TempDir::new("resume");
    let root = DataRoot::new(dir.path());
    let mut opts = options(2);
    opts.download.retries = 0;
    let done_before = {
        // Every body response is cut after 100 000 bytes and retries are off: the first
        // run fails with partial progress on disk.
        let flaky = Server::start(
            data.clone(),
            ServerConfig {
                cut_responses: usize::MAX,
                cut_after: 100_000,
                ..ServerConfig::default()
            },
        );
        let registry = registry_for(&flaky.url(), data.len(), Some(&sha));
        let err = run(&root, &registry, &opts).expect_err("first run fails");
        assert!(err.to_string().contains("failed"), "{err}");
        let ds = registry.get("toy").expect("toy");
        assert!(!root.file_path(ds, &ds.files[0]).exists(), "nothing placed");
        match root.status(ds, &ds.files[0]) {
            FileStatus::Partial { done } => done,
            other => panic!("expected a partial download, got {other:?}"),
        }
    };
    assert!(done_before >= 100_000, "{done_before}");
    let good = Server::start(data.clone(), ServerConfig::default());
    let registry = registry_for(&good.url(), data.len(), Some(&sha));
    run(&root, &registry, &options(2)).expect("second run completes");
    let ds = registry.get("toy").expect("toy");
    assert_eq!(
        std::fs::read(root.file_path(ds, &ds.files[0])).expect("read"),
        data
    );
    // The second run asked only for the bytes the first one had not written.
    let requested: u64 = range_spans(&good).iter().map(|(a, b)| b - a + 1).sum();
    assert_eq!(
        requested,
        u64::try_from(data.len()).expect("fits") - done_before
    );
}

#[test]
fn a_server_without_range_support_still_gives_a_verified_file() {
    let data = bytes(2 << 20, 5);
    let server = Server::start(
        data.clone(),
        ServerConfig {
            ignore_ranges: true,
            ..ServerConfig::default()
        },
    );
    let dir = TempDir::new("noranges");
    let root = DataRoot::new(dir.path());
    let registry = registry_for(&server.url(), data.len(), Some(&sha256_of(&data)));
    run(&root, &registry, &options(8)).expect("fetch");
    assert_eq!(server.seen().len(), 1, "one plain request");
    let ds = registry.get("toy").expect("toy");
    assert_eq!(root.status(ds, &ds.files[0]), FileStatus::Verified);
}

#[test]
fn a_checksum_mismatch_places_nothing_and_discards_the_bytes() {
    let data = bytes(1 << 20, 6);
    let server = Server::start(data.clone(), ServerConfig::default());
    let dir = TempDir::new("badsum");
    let root = DataRoot::new(dir.path());
    let wrong = sha256_of(b"something else");
    let registry = registry_for(&server.url(), data.len(), Some(&wrong));
    let err = run(&root, &registry, &options(2)).expect_err("mismatch");
    assert!(err.to_string().contains("SHA-256"), "{err}");
    let ds = registry.get("toy").expect("toy");
    assert!(!root.file_path(ds, &ds.files[0]).exists());
    assert_eq!(root.status(ds, &ds.files[0]), FileStatus::Missing);
}

#[test]
fn a_size_different_from_the_registry_is_refused_before_downloading() {
    let data = bytes(1 << 20, 7);
    let server = Server::start(data.clone(), ServerConfig::default());
    let dir = TempDir::new("size");
    let root = DataRoot::new(dir.path());
    let registry = registry_for(&server.url(), data.len() + 1, Some(&sha256_of(&data)));
    let err = run(&root, &registry, &options(2)).expect_err("size differs");
    assert!(err.to_string().contains("source changed"), "{err}");
    assert_eq!(range_starts(&server), Vec::<u64>::new(), "only the probe");
}

/// Fetches 4 MiB from a server that answers the probe honestly and then misbehaves on
/// every chunk request; returns the error.
fn fetch_from_lying_server(lie: ChunkLie) -> Error {
    let data = bytes(4 << 20, 8);
    let server = Server::start(
        data.clone(),
        ServerConfig {
            chunk_lie: Some(lie),
            ..ServerConfig::default()
        },
    );
    let dir = TempDir::new("lie");
    let root = DataRoot::new(dir.path());
    let registry = registry_for(&server.url(), data.len(), Some(&sha256_of(&data)));
    let mut opts = options(4);
    opts.download.retries = 0;
    let err = run(&root, &registry, &opts).expect_err("a lying server must fail the fetch");
    assert!(
        !range_starts(&server).is_empty(),
        "the probe passed and chunks were asked for: {lie:?}"
    );
    err
}

#[test]
fn a_wrong_content_range_on_a_chunk_is_an_error() {
    // The bytes sent are the right ones in all three cases: only the header check can
    // fail the fetch.
    for lie in [
        ChunkLie::WrongStart,
        ChunkLie::WrongEnd,
        ChunkLie::WrongTotal,
    ] {
        let err = fetch_from_lying_server(lie);
        assert!(
            err.to_string().contains("got Content-Range"),
            "{lie:?}: {err}"
        );
    }
}

#[test]
fn a_200_answer_to_a_chunk_request_is_an_error() {
    // A plausible Content-Range with status 200 and the whole file: without the status
    // check the wrong bytes would be written and only the checksum would notice.
    let err = fetch_from_lying_server(ChunkLie::Status200);
    assert!(err.to_string().contains("got status 200"), "{err}");
}

#[test]
fn a_server_without_ranges_declaring_the_wrong_length_is_refused_before_reading() {
    let data = bytes(1 << 20, 13);
    let server = Server::start(
        data.clone(),
        ServerConfig {
            ignore_ranges: true,
            ..ServerConfig::default()
        },
    );
    let dir = TempDir::new("whole-length");
    let root = DataRoot::new(dir.path());
    let registry = registry_for(&server.url(), data.len() - 1, Some(&sha256_of(&data)));
    let err = run(&root, &registry, &options(2)).expect_err("length differs");
    assert!(err.to_string().contains("declares"), "{err}");
    let ds = registry.get("toy").expect("toy");
    assert!(
        !root.part_path(ds, &ds.files[0]).exists(),
        "nothing written"
    );
}

#[test]
fn a_server_without_ranges_sending_too_much_is_cut_off() {
    // No Content-Length: the body runs until the connection closes, and it is twice
    // the size the registry expects.
    let data = bytes(2 << 20, 14);
    let expected = &data[..1 << 20];
    let server = Server::start(
        data.clone(),
        ServerConfig {
            ignore_ranges: true,
            no_content_length: true,
            ..ServerConfig::default()
        },
    );
    let dir = TempDir::new("whole-long");
    let root = DataRoot::new(dir.path());
    let registry = registry_for(&server.url(), expected.len(), Some(&sha256_of(expected)));
    let err = run(&root, &registry, &options(2)).expect_err("too long");
    assert!(err.to_string().contains("more than 1048576 bytes"), "{err}");
    let ds = registry.get("toy").expect("toy");
    assert!(
        !root.part_path(ds, &ds.files[0]).exists(),
        "the partial stream is deleted"
    );
    assert_eq!(root.status(ds, &ds.files[0]), FileStatus::Missing);
}

#[test]
fn a_leftover_part_without_state_is_listed_as_partial_and_replaced() {
    let data = bytes(100_000, 15);
    let server = Server::start(data.clone(), ServerConfig::default());
    let dir = TempDir::new("leftover");
    let root = DataRoot::new(dir.path());
    let registry = registry_for(&server.url(), data.len(), Some(&sha256_of(&data)));
    let ds = registry.get("toy").expect("toy");
    let f = &ds.files[0];
    std::fs::create_dir_all(root.dataset_dir(ds).join(".partial")).expect("mkdir");
    std::fs::write(root.part_path(ds, f), b"left by a killed plain stream").expect("write");
    assert_eq!(root.status(ds, f), FileStatus::Partial { done: 29 });
    run(&root, &registry, &options(2)).expect("fetch");
    assert_eq!(root.status(ds, f), FileStatus::Verified);
}

#[test]
fn retries_are_not_used_up_while_a_chunk_still_advances() {
    // One 3 MiB chunk, every one of the first eight responses cut after 300 000 bytes,
    // one retry allowed: each try makes progress, so the download completes.
    let data = bytes(3 << 20, 16);
    let server = Server::start(
        data.clone(),
        ServerConfig {
            cut_responses: 8,
            cut_after: 300_000,
            ..ServerConfig::default()
        },
    );
    let dir = TempDir::new("progress");
    let root = DataRoot::new(dir.path());
    let registry = registry_for(&server.url(), data.len(), Some(&sha256_of(&data)));
    let mut opts = options(1);
    opts.download.retries = 1;
    run(&root, &registry, &opts).expect("progressing tries do not count as failures");
    assert_eq!(range_starts(&server).len(), 9);
}

#[test]
fn a_mirror_is_used_when_the_first_url_fails() {
    let data = bytes(200_000, 9);
    let server = Server::start(data.clone(), ServerConfig::default());
    let dir = TempDir::new("mirror");
    let root = DataRoot::new(dir.path());
    let missing = format!("{}/missing", server.base());
    let text = registry_for(&missing, data.len(), Some(&sha256_of(&data)));
    let mut registry = text;
    if let Some(f) = registry
        .datasets
        .get_mut(0)
        .and_then(|d| d.files.get_mut(0))
    {
        f.mirrors.push(server.url());
    }
    let out = run(&root, &registry, &options(2)).expect("mirror works");
    assert_eq!(out, vec![Outcome::Placed { from: server.url() }]);
}

#[test]
fn unpinned_files_need_permission_and_report_their_checksum() {
    let data = bytes(10_000, 10);
    let server = Server::start(data.clone(), ServerConfig::default());
    let dir = TempDir::new("unpinned");
    let root = DataRoot::new(dir.path());
    let registry = registry_for(&server.url(), data.len(), None);
    let err = run(&root, &registry, &options(2)).expect_err("refused");
    assert!(err.to_string().contains("--allow-unpinned"), "{err}");
    let mut opts = options(2);
    opts.allow_unpinned = true;
    let out = run(&root, &registry, &opts).expect("allowed");
    assert_eq!(
        out,
        vec![Outcome::PlacedUnpinned {
            from: server.url(),
            sha256: sha256_of(&data)
        }]
    );
    let ds = registry.get("toy").expect("toy");
    assert!(matches!(
        root.status(ds, &ds.files[0]),
        FileStatus::Unpinned { .. }
    ));
}

#[test]
fn from_dir_takes_a_local_copy_after_checking_it() {
    let data = bytes(50_000, 11);
    let local = TempDir::new("local");
    std::fs::write(local.path().join("toy.bin"), &data).expect("write");
    let dir = TempDir::new("fromdir");
    let root = DataRoot::new(dir.path());
    // The URL points nowhere: the file must come from the directory.
    let registry = registry_for(
        "http://127.0.0.1:9/never",
        data.len(),
        Some(&sha256_of(&data)),
    );
    let mut opts = options(2);
    opts.from_dir = Some(local.path().to_path_buf());
    let out = run(&root, &registry, &opts).expect("from dir");
    assert!(matches!(&out[..], [Outcome::Placed { from }] if from.ends_with("toy.bin")));
    // A copy, not a link: the placed file is read-only, and writing to it anyway
    // leaves the source as it was.
    let ds = registry.get("toy").expect("toy");
    let placed = root.file_path(ds, &ds.files[0]);
    assert!(
        std::fs::metadata(&placed)
            .expect("meta")
            .permissions()
            .readonly()
    );
    make_writable(&placed);
    std::fs::write(&placed, b"edited in place").expect("write");
    assert_eq!(
        std::fs::read(local.path().join("toy.bin")).expect("read"),
        data
    );
    // A corrupt local copy is refused.
    let dir2 = TempDir::new("fromdir-bad");
    std::fs::write(local.path().join("toy.bin"), b"corrupt").expect("write");
    let err = run(&DataRoot::new(dir2.path()), &registry, &opts).expect_err("corrupt");
    assert!(err.to_string().contains("does not match"), "{err}");
    assert_eq!(
        std::fs::read(local.path().join("toy.bin")).expect("read"),
        b"corrupt"
    );
}

#[test]
fn verify_detects_a_changed_file() {
    let data = bytes(10_000, 12);
    let server = Server::start(data.clone(), ServerConfig::default());
    let dir = TempDir::new("verify");
    let root = DataRoot::new(dir.path());
    let registry = registry_for(&server.url(), data.len(), Some(&sha256_of(&data)));
    run(&root, &registry, &options(1)).expect("fetch");
    let ds = registry.get("toy").expect("toy");
    let f = &ds.files[0];
    assert_eq!(verify_file(&root, ds, f).expect("ok"), FileStatus::Verified);
    let mut changed = data.clone();
    changed[0] ^= 1;
    make_writable(&root.file_path(ds, f));
    std::fs::write(root.file_path(ds, f), &changed).expect("write");
    assert!(verify_file(&root, ds, f).is_err());
    assert_eq!(root.status(ds, f), FileStatus::Unverified);
}

#[test]
fn list_status_notices_a_same_size_change_through_the_modification_time() {
    // The cheap path: no re-hash, only the stamp's size and mtime.
    let data = bytes(10_000, 17);
    let server = Server::start(data.clone(), ServerConfig::default());
    let dir = TempDir::new("mtime");
    let root = DataRoot::new(dir.path());
    let registry = registry_for(&server.url(), data.len(), Some(&sha256_of(&data)));
    run(&root, &registry, &options(1)).expect("fetch");
    let ds = registry.get("toy").expect("toy");
    let f = &ds.files[0];
    let path = root.file_path(ds, f);
    assert_eq!(root.status(ds, f), FileStatus::Verified);
    let mtime = std::fs::metadata(&path)
        .expect("meta")
        .modified()
        .expect("mtime");
    make_writable(&path);
    let mut changed = data.clone();
    changed[0] ^= 1;
    std::fs::write(&path, &changed).expect("same size, other bytes");
    // Make sure the clock moved even on a coarse file system.
    std::fs::File::options()
        .write(true)
        .open(&path)
        .and_then(|file| file.set_modified(mtime + Duration::from_secs(2)))
        .expect("set mtime");
    assert_eq!(root.status(ds, f), FileStatus::Unverified);
}

#[test]
fn a_placed_file_that_no_longer_matches_says_what_to_do_and_replace_fixes_it() {
    let old = bytes(20_000, 18);
    let new = bytes(20_000, 19);
    let dir = TempDir::new("replace");
    let root = DataRoot::new(dir.path());
    let server = Server::start(old.clone(), ServerConfig::default());
    run(
        &root,
        &registry_for(&server.url(), old.len(), Some(&sha256_of(&old))),
        &options(1),
    )
    .expect("old version placed");
    // The registry now pins a new version of the file.
    let server = Server::start(new.clone(), ServerConfig::default());
    let registry = registry_for(&server.url(), new.len(), Some(&sha256_of(&new)));
    let err = run(&root, &registry, &options(1)).expect_err("old file in place");
    let ds = registry.get("toy").expect("toy");
    let path = root.file_path(ds, &ds.files[0]);
    assert!(
        err.to_string()
            .contains(&format!("delete {} and run fetch again", path.display())),
        "{err}"
    );
    let mut opts = options(1);
    opts.replace = true;
    let out = run(&root, &registry, &opts).expect("replaced");
    assert_eq!(out, vec![Outcome::Placed { from: server.url() }]);
    assert_eq!(std::fs::read(&path).expect("read"), new);
}

fn make_writable(path: &std::path::Path) {
    let mut p = std::fs::metadata(path).expect("meta").permissions();
    #[allow(
        clippy::permissions_set_readonly_false,
        reason = "a test undoing read-only"
    )]
    p.set_readonly(false);
    std::fs::set_permissions(path, p).expect("chmod");
}

#[test]
fn every_synthetic_dataset_of_the_registry_generates_its_pinned_bytes() {
    let registry = Registry::embedded().expect("valid");
    let dir = TempDir::new("synth");
    let root = DataRoot::new(dir.path());
    let opts = options(1);
    let agent = agent(&opts.download).expect("agent");
    let synthetic: Vec<_> = registry
        .datasets
        .iter()
        .filter(|d| d.is_generated())
        .collect();
    assert!(synthetic.len() >= 5);
    for ds in synthetic {
        let out = fetch(&root, &agent, ds, &opts, &mut std::io::sink())
            .unwrap_or_else(|e| panic!("{}: {e}", ds.name));
        assert_eq!(out.len(), 1);
        let gen_ = ds.generator.expect("generator");
        assert_eq!(ds.nodes, Some(gen_.nodes()), "{}", ds.name);
        assert_eq!(ds.edges, Some(gen_.edges()), "{}", ds.name);
        assert_eq!(root.status(ds, &ds.files[0]), FileStatus::Verified);
    }
}
