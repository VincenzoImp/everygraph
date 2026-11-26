// SPDX-License-Identifier: Apache-2.0 OR MIT

//! What the dictionary's memory tests share: scratch files and synthetic keys.

#![allow(dead_code, reason = "each test crate uses a part of it")]

use std::fs::File;
use std::io;
use std::os::unix::fs::FileExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use everygraph_core::{ByteSource, ReadError};
use everygraph_ingest::ids::{ColumnType, DictionaryError, Key, KeySource, Scratch};

/// Scratch files with no buffer of their own (the library's write buffers are its
/// own, and reserved).
pub struct Files {
    dir: PathBuf,
    next: AtomicU64,
}

impl Files {
    /// Scratch files in a new directory `dir`, removed when dropped (also when a test
    /// panics).
    pub fn new(dir: PathBuf) -> Self {
        std::fs::create_dir_all(&dir).unwrap();
        Self {
            dir,
            next: AtomicU64::new(0),
        }
    }
}

impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

pub struct Sealed {
    file: File,
    len: u64,
    path: PathBuf,
}

impl ByteSource for Sealed {
    fn len(&self) -> u64 {
        self.len
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), ReadError> {
        self.file
            .read_exact_at(buf, offset)
            .map_err(|e| ReadError::Backend(Box::new(e)))
    }
}

impl Drop for Sealed {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub struct Writer {
    file: File,
    path: PathBuf,
}

impl io::Write for Writer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.file.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Scratch for Files {
    type Source = Sealed;
    type Writer = Writer;

    fn create(&self, _: &'static str) -> io::Result<Writer> {
        let path = self
            .dir
            .join(format!("{}", self.next.fetch_add(1, Ordering::Relaxed)));
        Ok(Writer {
            file: File::create(&path)?,
            path,
        })
    }

    fn seal(&self, w: Writer) -> io::Result<Sealed> {
        drop(w.file);
        let file = File::open(&w.path)?;
        let len = file.metadata()?.len();
        Ok(Sealed {
            file,
            len,
            path: w.path,
        })
    }
}

/// Keys made on demand (so that the input is not on the heap).
pub struct Synthetic {
    pub column: ColumnType,
    pub n: u64,
    pub stream: u64,
    pub make: fn(u64, &mut Vec<u8>) -> Option<u64>,
}

impl KeySource for Synthetic {
    fn column(&self) -> ColumnType {
        self.column
    }
    fn chunks(&self) -> usize {
        usize::try_from(self.stream.div_ceil(1 << 14)).unwrap()
    }
    fn read_chunk(
        &self,
        chunk: usize,
        f: &mut dyn FnMut(u64, Key<'_>) -> Result<(), DictionaryError>,
    ) -> Result<(), DictionaryError> {
        let start = u64::try_from(chunk).unwrap() << 14;
        let mut buf = Vec::with_capacity(128);
        for j in start..(start + (1 << 14)).min(self.stream) {
            let i = splitmix(j) % self.n;
            buf.clear();
            match (self.make)(i, &mut buf) {
                Some(v) => f(j, Key::UInt(v))?,
                None => f(j, Key::Bytes(&buf))?,
            }
        }
        Ok(())
    }
}

fn splitmix(z: u64) -> u64 {
    everygraph_oracle::rng::SplitMix64::new(z).next_u64()
}

pub fn hex(i: u64, out: &mut Vec<u8>) -> Option<u64> {
    out.extend_from_slice(
        format!(
            "0x{:016x}{:016x}{:08x}",
            splitmix(i),
            splitmix(i + 1),
            splitmix(i + 2) >> 32
        )
        .as_bytes(),
    );
    None
}

pub fn short(i: u64, out: &mut Vec<u8>) -> Option<u64> {
    out.extend_from_slice(format!("u{i}").as_bytes());
    None
}

pub fn url(i: u64, out: &mut Vec<u8>) -> Option<u64> {
    out.extend_from_slice(
        format!("https://www.example-social-network.com/profiles/user-{i}/about").as_bytes(),
    );
    None
}

pub fn binary20(i: u64, out: &mut Vec<u8>) -> Option<u64> {
    for j in 0..3 {
        out.extend_from_slice(&splitmix(i * 3 + j).to_le_bytes());
    }
    out.truncate(20);
    None
}

pub fn sparse_int(i: u64, _: &mut Vec<u8>) -> Option<u64> {
    Some(splitmix(i))
}
