// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The probe of the machine besides memory: cores and the storage of a directory.

#![forbid(unsafe_code)]

use everygraph_io::physical_cores;

#[test]
fn physical_cores_are_at_most_the_threads_the_process_may_use() {
    let threads = std::thread::available_parallelism().unwrap();
    assert!(physical_cores() <= threads);
}

/// A thread whose affinity mask holds one CPU sees one core, as under `taskset -c 0`
/// or a cpuset of one CPU.
#[test]
#[cfg(target_os = "linux")]
fn physical_cores_follow_the_affinity_mask() {
    use rustix::thread::{CpuSet, sched_getaffinity, sched_setaffinity};
    std::thread::spawn(|| {
        let mine = sched_getaffinity(None).unwrap();
        let first = (0..CpuSet::MAX_CPU).find(|&c| mine.is_set(c)).unwrap();
        let mut one = CpuSet::new();
        one.set(first);
        sched_setaffinity(None, &one).unwrap();
        assert_eq!(physical_cores().get(), 1);
    })
    .join()
    .unwrap();
}

#[test]
#[cfg(target_os = "linux")]
fn the_storage_of_a_directory_has_a_kind_and_free_bytes() {
    use everygraph_core::StorageKind;
    use everygraph_io::{memory_backed, storage};
    let dir = std::env::temp_dir();
    let s = storage(&dir).expect("statvfs of the temporary directory");
    // Memory-backed exactly when `memory_backed` says so, as the plan's scratch needs.
    assert_eq!(s.kind == StorageKind::Memory, memory_backed(&dir), "{s:?}");
    assert!(s.free_bytes > 0, "{s:?}");
    assert!(storage(std::path::Path::new("/no/such/directory")).is_none());
}
