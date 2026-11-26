// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The check of a per-entry weight or cost column before a kernel uses it.

use std::num::NonZeroUsize;

use everygraph_core::par;

use crate::edge_map::Error;

const GRAIN: NonZeroUsize = NonZeroUsize::new(1 << 16).expect("non-zero");

/// `Ok` if `column` has one value per entry of a source with `entries` entries, and
/// every value is finite and not negative (zero is allowed). Otherwise
/// [`Error::Invalid`], or [`Error::Weight`] with the first bad value
/// (`docs/spec/graph-semantics.md` § 4.4).
pub(crate) fn check(column: &[f64], entries: u64) -> Result<(), Error> {
    if column.len() as u64 != entries {
        return Err(Error::Invalid(
            "a per-entry column whose length is not the number of entries",
        ));
    }
    let first_bad = par::map_ranges(column.len(), GRAIN, |r| {
        let start = r.start;
        column[r]
            .iter()
            .position(|w| !(w.is_finite() && *w >= 0.0))
            .map(|i| start + i)
    });
    match first_bad.into_iter().flatten().next() {
        Some(i) => Err(Error::Weight {
            entry: i as u64,
            value: column[i],
        }),
        None => Ok(()),
    }
}
