// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Datasets by name: the registry (`datasets/datasets.toml`), the data root
//! (`$EVERYGRAPH_DATA`, default `./data`), the fetcher and the synthetic generators.
//!
//! Other tools find a dataset's files like this:
//!
//! ```no_run
//! use everygraph_bench::datasets::{DataRoot, FileStatus, Registry};
//!
//! let registry = Registry::embedded()?;
//! let root = DataRoot::from_env();
//! let lj = registry.get("com-livejournal").expect("registered");
//! let file = &lj.files[0];
//! assert_eq!(root.status(lj, file), FileStatus::Verified, "run: everygraph-bench fetch com-livejournal");
//! let path = root.file_path(lj, file);
//! # Ok::<(), everygraph_bench::datasets::Error>(())
//! ```

mod download;
mod error;
mod fetch;
pub mod markdown;
mod registry;
mod store;
mod synth;

#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;

pub use download::{DownloadOptions, agent, download};
pub use error::{Error, Result};
pub use fetch::{FetchOptions, Outcome, fetch, verify_file};
pub use registry::{Dataset, EMBEDDED, Family, FileEntry, Registry, SCHEMA, UNVERIFIABLE_FIELDS};
pub use store::{
    DATA_ENV, DEFAULT_ROOT, DataRoot, FileStatus, Hashes, check_hashes, hash_file, hex, human_bytes,
};
pub use synth::Generator;
