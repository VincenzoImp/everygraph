// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The probe of the machine besides memory: cores and the storage of a directory.

#![forbid(unsafe_code)]

use everygraph_io::physical_cores;

#[test]
fn physical_cores_are_at_most_the_hardware_threads() {
    let threads = std::thread::available_parallelism().map_or(1, usize::from);
    if let Some(cores) = physical_cores() {
        assert!(
            (1..=threads).contains(&cores),
            "{cores} cores, {threads} threads"
        );
    }
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
