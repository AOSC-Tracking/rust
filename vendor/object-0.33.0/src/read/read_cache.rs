use core::ops::Range;
use std::boxed::Box;
use std::cell::RefCell;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::convert::TryInto;
use std::io::{Read, Seek, SeekFrom};
use std::mem;
use std::vec::Vec;

use crate::read::ReadRef;

/// An implementation of [`ReadRef`] for data in a stream that implements
/// `Read + Seek`.
///
/// Contains a cache of read-only blocks of data, allowing references to
/// them to be returned. Entries in the cache are never removed.
/// Entries are keyed on the offset and size of the read.
/// Currently overlapping reads are considered separate reads.
///
/// This is primarily intended for environments where memory mapped files
/// are not available or not suitable, such as WebAssembly.
///
/// Note that malformed files can cause the cache to grow much larger than
/// the file size.
#[derive(Debug)]
pub struct ReadCache<R: Read + Seek> {
    cache: RefCell<ReadCacheInternal<R>>,
}

#[derive(Debug)]
struct ReadCacheInternal<R: Read + Seek> {
    read: R,
    bufs: HashMap<(u64, u64), Box<[u8]>>,
    strings: HashMap<(u64, u8), Box<[u8]>>,
    len: Option<u64>,
}

impl<R: Read + Seek> ReadCacheInternal<R> {
    /// Ensures this range is contained in the len of the file
    fn range_in_bounds(&mut self, range: &Range<u64>) -> Result<(), ()> {
        if range.start <= range.end && range.end <= self.len()? {
            Ok(())
        } else {
            Err(())
        }
    }

    /// The length of the underlying read, memoized
    fn len(&mut self) -> Result<u64, ()> {
        match self.len {
            Some(len) => Ok(len),
            None => {
                let len = self.read.seek(SeekFrom::End(0)).map_err(|_| ())?;
                self.len = Some(len);
                Ok(len)
            }
        }
    }
}

impl<R: Read + Seek> ReadCache<R> {
    /// Create an empty `ReadCache` for the given stream.
    pub fn new(read: R) -> Self {
        ReadCache {
            cache: RefCell::new(ReadCacheInternal {
                read,
                bufs: HashMap::new(),
                strings: HashMap::new(),
                len: None,
            }),
        }
    }

    /// Return an implementation of `ReadRef` that restricts reads
    /// to the given range of the stream.
    pub fn range(&self, offset: u64, size: u64) -> ReadCacheRange<'_, R> {
        ReadCacheRange {
            r: self,
            offset,
            size,
        }
    }

    /// Free buffers used by the cache.
    pub fn clear(&mut self) {
        self.cache.borrow_mut().bufs.clear();
    }

    /// Unwrap this `ReadCache<R>`, returning the underlying reader.
    pub fn into_inner(self) -> R {
        self.cache.into_inner().read
    }
}

impl<'a, R: Read + Seek> ReadRef<'a> for &'a ReadCache<R> {
    fn len(self) -> Result<u64, ()> {
        self.cache.borrow_mut().len()
    }

    fn read_bytes_at(self, offset: u64, size: u64) -> Result<&'a [u8], ()> {
        if size == 0 {
            return Ok(&[]);
        }
        let cache = &mut *self.cache.borrow_mut();
        cache.range_in_bounds(&(offset..(offset.saturating_add(size))))?;
        let buf = match cache.bufs.entry((offset, size)) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let size = size.try_into().map_err(|_| ())?;
                cache.read.seek(SeekFrom::Start(offset)).map_err(|_| ())?;
                let mut bytes = Vec::new();
                bytes.try_reserve_exact(size).map_err(|_| ())?;
                bytes.resize(size, 0);
                let mut bytes = bytes.into_boxed_slice();
                cache.read.read_exact(&mut bytes).map_err(|_| ())?;
                entry.insert(bytes)
            }
        };
        // Extend the lifetime to that of self.
        // This is OK because we never mutate or remove entries.
        Ok(unsafe { mem::transmute::<&[u8], &[u8]>(buf) })
    }

    fn read_bytes_at_until(self, range: Range<u64>, delimiter: u8) -> Result<&'a [u8], ()> {
        let cache = &mut *self.cache.borrow_mut();
        cache.range_in_bounds(&range)?;
        let buf = match cache.strings.entry((range.start, delimiter)) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                cache
                    .read
                    .seek(SeekFrom::Start(range.start))
                    .map_err(|_| ())?;

                let max_check: usize = (range.end - range.start).try_into().map_err(|_| ())?;
                // Strings should be relatively small.
                // TODO: make this configurable?
                let max_check = max_check.min(4096);

                let mut bytes = Vec::new();
                let mut checked = 0;
                loop {
                    bytes.resize((checked + 256).min(max_check), 0);
                    let read = cache.read.read(&mut bytes[checked..]).map_err(|_| ())?;
                    if read == 0 {
                        return Err(());
                    }
                    if let Some(len) = memchr::memchr(delimiter, &bytes[checked..][..read]) {
                        bytes.truncate(checked + len);
                        break entry.insert(bytes.into_boxed_slice());
                    }
                    checked += read;
                    if checked >= max_check {
                        return Err(());
                    }
                }
            }
        };
        // Extend the lifetime to that of self.
        // This is OK because we never mutate or remove entries.
        Ok(unsafe { mem::transmute::<&[u8], &[u8]>(buf) })
    }
}

/// An implementation of [`ReadRef`] for a range of data in a stream that
/// implements `Read + Seek`.
///
/// Shares an underlying [`ReadCache`] with a lifetime of `'a`.
#[derive(Debug)]
pub struct ReadCacheRange<'a, R: Read + Seek> {
    r: &'a ReadCache<R>,
    offset: u64,
    size: u64,
}

impl<'a, R: Read + Seek> Clone for ReadCacheRange<'a, R> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'a, R: Read + Seek> Copy for ReadCacheRange<'a, R> {}

impl<'a, R: Read + Seek> ReadRef<'a> for ReadCacheRange<'a, R> {
    fn len(self) -> Result<u64, ()> {
        Ok(self.size)
    }

    fn read_bytes_at(self, offset: u64, size: u64) -> Result<&'a [u8], ()> {
        if size == 0 {
            return Ok(&[]);
        }
        let end = offset.checked_add(size).ok_or(())?;
        if end > self.size {
            return Err(());
        }
        let r_offset = self.offset.checked_add(offset).ok_or(())?;
        self.r.read_bytes_at(r_offset, size)
    }

    fn read_bytes_at_until(self, range: Range<u64>, delimiter: u8) -> Result<&'a [u8], ()> {
        let r_start = self.offset.checked_add(range.start).ok_or(())?;
        let r_end = self.offset.checked_add(range.end).ok_or(())?;
        let bytes = self.r.read_bytes_at_until(r_start..r_end, delimiter)?;
        let size = bytes.len().try_into().map_err(|_| ())?;
        let end = range.start.checked_add(size).ok_or(())?;
        if end > self.size {
            return Err(());
        }
        Ok(bytes)
    }
}
