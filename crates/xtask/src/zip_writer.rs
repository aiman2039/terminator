//! Minimal deflated ZIP writer for the Windows package.
//!
//! The `zip` crate is not a dependency, so this writes the one layout the
//! release needs: deflated files with a central directory (no archives,
//! encryption, or ZIP64). The output opens in Explorer and `Expand-Archive`.
//! Timestamps stay zero: reproducible builds beat Finder dates.

use anyhow::{Context, Result};
use std::{fs, io::Write, path::Path};

/// IEEE CRC-32, table built once per process.
fn crc32(data: &[u8]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut table = [0u32; 256];
        for (i, slot) in table.iter_mut().enumerate() {
            let mut crc = u32::try_from(i).unwrap_or(u32::MAX);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    0xEDB8_8320 ^ (crc >> 1)
                } else {
                    crc >> 1
                };
            }
            *slot = crc;
        }
        table
    });
    let mut crc = 0xFFFF_FFFFu32;
    for byte in data {
        // Masked to one byte: widening on every 64-bit target we ship.
        let index = ((crc ^ u32::from(*byte)) & 0xFF) as usize;
        crc = table.get(index).copied().unwrap_or(0) ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

pub struct Entry {
    name: String,
    crc: u32,
    compressed: Vec<u8>,
    uncompressed: u32,
    offset: u32,
}

fn put_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn entry_header(out: &mut Vec<u8>, entry: &Entry, central: bool) -> Result<()> {
    put_u32(out, if central { 0x0201_4B50 } else { 0x0403_4B50 });
    if central {
        put_u16(out, 20); // version made by (2.0, FAT)
    }
    put_u16(out, 20); // version needed (2.0)
    put_u16(out, 0x0800); // UTF-8 names
    let empty = entry.compressed.is_empty() && entry.uncompressed == 0;
    put_u16(out, if empty { 0 } else { 8 }); // stored or deflated
    put_u16(out, 0); // time (midnight)
    put_u16(out, 0x0021); // date (1980-01-01: reproducible, always valid)
    put_u32(out, entry.crc);
    put_u32(
        out,
        u32::try_from(entry.compressed.len()).context("ZIP entry too large")?,
    );
    put_u32(out, entry.uncompressed);
    put_u16(
        out,
        u16::try_from(entry.name.len()).context("ZIP name too long")?,
    );
    put_u16(out, 0); // extra
    if central {
        put_u16(out, 0); // comment
        put_u16(out, 0); // disk
        put_u16(out, 0); // internal attributes
        // Directories read back as directories on every platform.
        put_u32(
            out,
            if entry.name.ends_with('/') {
                0x10
            } else {
                0x20
            },
        );
        put_u32(out, entry.offset);
    }
    out.extend_from_slice(entry.name.as_bytes());
    Ok(())
}

/// Append one file (stored under `name` with forward slashes) to the archive.
pub fn append_file(entries: &mut Vec<Entry>, name: &str, path: &Path) -> Result<()> {
    let data = fs::read(path).with_context(|| format!("Read {}", path.display()))?;
    let mut encoder =
        flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&data)?;
    entries.push(Entry {
        name: name.replace('\\', "/"),
        crc: crc32(&data),
        compressed: encoder.finish()?,
        uncompressed: u32::try_from(data.len()).context("ZIP entry too large")?,
        offset: 0,
    });
    Ok(())
}

/// Append an empty directory entry (forward slashes, trailing slash).
pub fn append_dir(entries: &mut Vec<Entry>, name: &str) {
    let mut name = name.replace('\\', "/");
    if !name.ends_with('/') {
        name.push('/');
    }
    entries.push(Entry {
        name,
        crc: 0,
        compressed: Vec::new(),
        uncompressed: 0,
        offset: 0,
    });
}

/// Serialize collected entries as a `.zip` file.
pub fn finish(path: &Path, entries: &mut [Entry]) -> Result<()> {
    let mut out = Vec::new();
    for entry in entries.iter_mut() {
        entry.offset = u32::try_from(out.len()).context("ZIP archive too large")?;
        entry_header(&mut out, entry, false)?;
        out.extend_from_slice(&entry.compressed);
    }
    let mut central = Vec::new();
    for entry in entries.iter() {
        entry_header(&mut central, entry, true)?;
    }
    let central_size = u32::try_from(central.len()).context("ZIP archive too large")?;
    let central_offset = u32::try_from(out.len()).context("ZIP archive too large")?;
    out.extend_from_slice(&central);
    let count = u16::try_from(entries.len()).context("ZIP has too many entries")?;
    put_u32(&mut out, 0x0605_4B50);
    put_u16(&mut out, 0);
    put_u16(&mut out, 0);
    put_u16(&mut out, count);
    put_u16(&mut out, count);
    put_u32(&mut out, central_size);
    put_u32(&mut out, central_offset);
    put_u16(&mut out, 0);
    fs::write(path, out)?;
    Ok(())
}

/// Collect a directory tree: `prefix/name` files plus directory entries, so
/// extraction recreates empty folders too.
pub fn append_tree(entries: &mut Vec<Entry>, prefix: &str, dir: &Path) -> Result<()> {
    let mut stack = vec![(prefix.to_owned(), dir.to_path_buf())];
    while let Some((name, path)) = stack.pop() {
        append_dir(entries, &name);
        let mut children: Vec<_> = fs::read_dir(&path)?
            .map(|entry| entry.map(|e| e.path()))
            .collect::<std::io::Result<_>>()?;
        children.sort();
        children.reverse();
        for child in children {
            let child_name = format!("{name}/{}", child.file_name().unwrap().to_string_lossy());
            if child.is_dir() {
                stack.push((child_name, child));
            } else if child.is_file() {
                append_file(entries, &child_name, &child)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_the_standard_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn round_trip_through_an_independent_decoder() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("nested");
        fs::create_dir(&nested).unwrap();
        fs::write(dir.path().join("hello.txt"), b"hello windows").unwrap();
        fs::write(nested.join("data.bin"), vec![7u8; 10_000]).unwrap();
        let mut entries = Vec::new();
        append_tree(&mut entries, "terminator", dir.path()).unwrap();
        let archive = dir.path().join("out.zip");
        finish(&archive, &mut entries).unwrap();
        // Verify with an independent reader, not the writer above.
        let found = read_archive(&archive);
        assert_eq!(found.get("terminator/hello.txt").unwrap(), b"hello windows");
        assert_eq!(
            found.get("terminator/nested/data.bin").unwrap(),
            &vec![7u8; 10_000]
        );
        assert_eq!(found.len(), 2);
    }

    /// Minimal independent verifier: parses headers and inflates deflated
    /// entries without touching the writer above.
    fn read_archive(path: &Path) -> std::collections::HashMap<String, Vec<u8>> {
        use std::io::{Read, Seek, SeekFrom};
        let mut file = fs::File::open(path).unwrap();
        // EOCD with an empty comment sits in the last 22 bytes.
        file.seek(SeekFrom::End(-22)).unwrap();
        let mut eocd = [0u8; 22];
        file.read_exact(&mut eocd).unwrap();
        assert_eq!(&eocd[0..4], b"PK\x05\x06");
        let count = u16::from_le_bytes([eocd[10], eocd[11]]);
        let central = u32::from_le_bytes([eocd[16], eocd[17], eocd[18], eocd[19]]);
        file.seek(SeekFrom::Start(u64::from(central))).unwrap();
        let mut offsets = Vec::new();
        for _ in 0..count {
            let mut head = [0u8; 46];
            file.read_exact(&mut head).unwrap();
            assert_eq!(&head[0..4], b"PK\x01\x02");
            let name_len = u16::from_le_bytes([head[28], head[29]]);
            let extra = u16::from_le_bytes([head[30], head[31]]);
            let comment = u16::from_le_bytes([head[32], head[33]]);
            let offset = u32::from_le_bytes([head[42], head[43], head[44], head[45]]);
            let mut name = vec![0u8; usize::from(name_len)];
            file.read_exact(&mut name).unwrap();
            let mut skip = vec![0u8; usize::from(extra)];
            file.read_exact(&mut skip).unwrap();
            let mut skip = vec![0u8; usize::from(comment)];
            file.read_exact(&mut skip).unwrap();
            offsets.push((String::from_utf8(name).unwrap(), offset));
        }
        let mut out = std::collections::HashMap::new();
        for (name, offset) in offsets {
            if name.ends_with('/') {
                continue;
            }
            file.seek(SeekFrom::Start(u64::from(offset))).unwrap();
            let mut head = [0u8; 30];
            file.read_exact(&mut head).unwrap();
            assert_eq!(&head[0..4], b"PK\x03\x04");
            let method = u16::from_le_bytes([head[8], head[9]]);
            let size = u32::from_le_bytes([head[18], head[19], head[20], head[21]]);
            let name_len = u16::from_le_bytes([head[26], head[27]]);
            let extra = u16::from_le_bytes([head[28], head[29]]);
            let mut skip = vec![0u8; usize::from(name_len)];
            file.read_exact(&mut skip).unwrap();
            let mut skip = vec![0u8; usize::from(extra)];
            file.read_exact(&mut skip).unwrap();
            let mut data = vec![0u8; size.try_into().unwrap()];
            file.read_exact(&mut data).unwrap();
            let bytes = match method {
                0 => data,
                8 => {
                    let mut decoder = flate2::read::DeflateDecoder::new(&data[..]);
                    let mut raw = Vec::new();
                    decoder.read_to_end(&mut raw).unwrap();
                    raw
                }
                other => panic!("unexpected method {other}"),
            };
            out.insert(name, bytes);
        }
        out
    }
}
