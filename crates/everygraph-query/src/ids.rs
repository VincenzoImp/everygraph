// SPDX-License-Identifier: Apache-2.0 OR MIT

//! External identifiers to node ids, in bulk, through the graph's dictionary
//! (`docs/api-sketch.md` § 2.2): what a query's `nodes` column is turned into before it
//! runs.

use everygraph_core::{ByteSource, Memory, NodeId, SegVec};
use everygraph_ingest::ids::{Dictionary, Key};

use crate::QueryError;

/// The node id of each key of `keys`, in that order; `None` for a key the graph does
/// not have. Hexadecimal and UUID keys match in any case, and integer keys may be given
/// as integers or as decimal text, as [`Dictionary::lookup`] matches them.
///
/// # Errors
///
/// [`QueryError::Ids`] when the dictionary cannot be read; [`QueryError::Lists`] when
/// the ids do not fit in `memory`.
pub fn resolve<'k, S: ByteSource>(
    dictionary: &Dictionary<'_, S>,
    keys: impl ExactSizeIterator<Item = Key<'k>>,
    memory: &Memory,
) -> Result<SegVec<Option<NodeId>>, QueryError> {
    let mut out = SegVec::new(memory, "point query ids");
    out.try_reserve(keys.len())?;
    for key in keys {
        out.try_push(dictionary.lookup(key)?)?;
    }
    Ok(out)
}
