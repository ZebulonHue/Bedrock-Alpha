//! Anvil region file (.mca) reader — port of `src/lib/mc/region.ts`.
//!
//! Layout: 8 KiB header = 1024-entry offset table (3-byte BE sector offset +
//! 1-byte sector count) + 1024-entry timestamp table. Each chunk record:
//! 4-byte BE length (includes the compression byte), 1-byte compression type
//! (1=gzip, 2=zlib, 3=uncompressed, 4=LZ4), then the (compressed) NBT payload.
//!
//! The header is read without inflating; callers inflate chunks one at a time.
//! LZ4 (type 4, seen in some 1.20.5+ worlds) is skipped + counted by the
//! caller, exactly like the TS parser.

use crate::error::ParseError;
use byteorder::{BigEndian, ReadBytesExt};
use flate2::read::{GzDecoder, ZlibDecoder};
use std::io::Read;

pub const SECTOR_BYTES: usize = 4096;
pub const HEADER_BYTES: usize = 8192;

/// Compression type 4 = LZ4 (unsupported — skipped and counted).
pub const COMPRESSION_LZ4: u8 = 4;

/// A located-but-not-yet-inflated chunk record inside a region buffer.
#[derive(Debug, Clone)]
pub struct RegionChunkRef {
    /// chunk x within the region (0..31)
    pub cx: u32,
    /// chunk z within the region (0..31)
    pub cz: u32,
    /// last-modified timestamp from the header (kept for format completeness;
    /// the parser itself never uses it)
    #[allow(dead_code)]
    pub timestamp: u32,
    /// compression type from the chunk record (1, 2, 3 or 4)
    pub compression: u8,
    /// byte offset of the chunk record (its 4-byte length field)
    pub pos: usize,
    /// payload length in bytes, including the compression byte
    pub length: usize,
}

/// Result of scanning a region header.
pub struct RegionHeader {
    pub refs: Vec<RegionChunkRef>,
    /// header entries whose sector pointer or length falls outside the file
    pub corrupt: u64,
}

/// Read the region header and locate every present chunk WITHOUT inflating.
/// Out-of-file chunk records are counted as corrupt and skipped (a single
/// corrupt chunk must not stall the whole file). Records with `length < 1`
/// are skipped WITHOUT counting corrupt — a deliberate TS quirk we keep.
pub fn read_region_header(buf: &[u8]) -> Result<RegionHeader, ParseError> {
    if buf.len() < HEADER_BYTES {
        return Err(ParseError::Truncated);
    }
    let mut refs = Vec::new();
    let mut corrupt = 0u64;

    for i in 0..1024usize {
        let entry = (&buf[i * 4..]).read_u32::<BigEndian>().unwrap();
        let sector_offset = (entry >> 8) as usize;
        let sector_count = (entry & 0xff) as usize;
        if sector_offset == 0 || sector_count == 0 {
            continue;
        }
        let pos = sector_offset * SECTOR_BYTES;
        if pos + 5 > buf.len() {
            corrupt += 1; // chunk points outside the file — skip it, keep going
            continue;
        }
        let length = (&buf[pos..]).read_u32::<BigEndian>().unwrap() as usize; // includes the compression byte
        if length < 1 {
            continue; // TS quirk: skipped without counting corrupt
        }
        if pos + 4 + length > buf.len() {
            corrupt += 1; // chunk data overruns the file — skip it, keep going
            continue;
        }
        refs.push(RegionChunkRef {
            cx: (i % 32) as u32,
            cz: (i / 32) as u32,
            timestamp: (&buf[SECTOR_BYTES + i * 4..])
                .read_u32::<BigEndian>()
                .unwrap(),
            compression: buf[pos + 4],
            pos,
            length,
        });
    }
    Ok(RegionHeader { refs, corrupt })
}

/// Inflate one chunk located by a header ref.
///
/// # Errors
/// `ParseError::CorruptNbt` on bad compression type or inflate failure —
/// the per-chunk catch turns it into a skip-and-count.
pub fn inflate_chunk(buf: &[u8], r: &RegionChunkRef) -> Result<Vec<u8>, ParseError> {
    let body = &buf[r.pos + 5..r.pos + 4 + r.length];
    match r.compression {
        1 => inflate_with(GzDecoder::new(body), "gzip"),
        2 => inflate_with(ZlibDecoder::new(body), "zlib"),
        3 => Ok(body.to_vec()),
        COMPRESSION_LZ4 => Err(ParseError::CorruptNbt(
            "Region: LZ4-compressed chunk (compression type 4) is not supported".into(),
        )),
        other => Err(ParseError::CorruptNbt(format!(
            "Region: unknown chunk compression type {other}"
        ))),
    }
}

fn inflate_with<R: Read>(mut dec: R, kind: &str) -> Result<Vec<u8>, ParseError> {
    let mut out = Vec::new();
    dec.read_to_end(&mut out)
        .map_err(|e| ParseError::CorruptNbt(format!("Region: {kind} inflate failed: {e}")))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use byteorder::{BigEndian, WriteBytesExt};
    use flate2::write::ZlibEncoder;
    use flate2::Compression;
    use std::io::Write;

    /// Build a region buffer: header + records written at sector 2,3,...
    fn build_region(records: &[(u8, Vec<u8>)]) -> Vec<u8> {
        let mut buf = vec![0u8; HEADER_BYTES];
        for (i, (compression, payload)) in records.iter().enumerate() {
            let sector = 2 + i;
            let pos = sector * SECTOR_BYTES;
            if buf.len() < pos + SECTOR_BYTES {
                buf.resize(pos + SECTOR_BYTES, 0);
            }
            let length = (1 + payload.len()) as u32;
            // record: 4-byte length, compression byte, then payload
            let mut rec = Vec::new();
            rec.write_u32::<BigEndian>(length).unwrap();
            rec.push(*compression);
            rec.extend_from_slice(payload);
            buf[pos..pos + rec.len()].copy_from_slice(&rec);
            // header entry: sector offset (3 bytes) + count
            let entry = ((sector as u32) << 8) | 1;
            (&mut buf[i * 4..]).write_u32::<BigEndian>(entry).unwrap();
            (&mut buf[SECTOR_BYTES + i * 4..])
                .write_u32::<BigEndian>(12345)
                .unwrap();
        }
        buf
    }

    #[test]
    fn header_locates_chunks() {
        let payload = b"nbt-bytes-here".to_vec();
        let buf = build_region(&[(2, payload.clone()), (1, b"other".to_vec())]);
        let h = read_region_header(&buf).unwrap();
        assert_eq!(h.corrupt, 0);
        assert_eq!(h.refs.len(), 2);
        assert_eq!(h.refs[0].cx, 0);
        assert_eq!(h.refs[0].cz, 0);
        assert_eq!(h.refs[1].cx, 1);
        assert_eq!(h.refs[1].cz, 0);
        assert_eq!(h.refs[0].timestamp, 12345);
        assert_eq!(h.refs[0].compression, 2);
        // zlib inflate roundtrip
        let mut enc = ZlibEncoder::new(Vec::new(), Compression::fast());
        enc.write_all(&payload).unwrap();
        let buf = build_region(&[(2, enc.finish().unwrap())]);
        let h = read_region_header(&buf).unwrap();
        assert_eq!(inflate_chunk(&buf, &h.refs[0]).unwrap(), payload);
    }

    #[test]
    fn header_counts_corrupt_and_keeps_quirks() {
        // entry pointing outside the file -> corrupt
        let mut buf = vec![0u8; HEADER_BYTES];
        (&mut buf[0..])
            .write_u32::<BigEndian>((200u32 << 8) | 1)
            .unwrap();
        // entry at sector 2 with length 0 -> skipped, NOT corrupt (TS quirk)
        (&mut buf[4..])
            .write_u32::<BigEndian>((2u32 << 8) | 1)
            .unwrap();
        buf.resize(HEADER_BYTES + 3 * SECTOR_BYTES, 0);
        // length stays 0 at sector 2
        let h = read_region_header(&buf).unwrap();
        assert_eq!(h.corrupt, 1, "out-of-file pointer counted corrupt");
        assert_eq!(h.refs.len(), 0, "length<1 record skipped without a ref");
    }

    #[test]
    fn truncated_file_rejected() {
        assert!(matches!(
            read_region_header(&[0u8; 100]),
            Err(ParseError::Truncated)
        ));
    }

    #[test]
    fn overrun_length_counted_corrupt() {
        let mut buf = vec![0u8; HEADER_BYTES + 2 * SECTOR_BYTES];
        (&mut buf[0..])
            .write_u32::<BigEndian>((2u32 << 8) | 1)
            .unwrap();
        let pos = 2 * SECTOR_BYTES;
        (&mut buf[pos..])
            .write_u32::<BigEndian>(999_999_999)
            .unwrap(); // length overruns file
        buf[pos + 4] = 2;
        let h = read_region_header(&buf).unwrap();
        assert_eq!(h.corrupt, 1);
        assert!(h.refs.is_empty());
    }
}
