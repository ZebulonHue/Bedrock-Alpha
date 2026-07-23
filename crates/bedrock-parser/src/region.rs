use std::fmt;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// Size of one sector in a region file.
const SECTOR_BYTES: usize = 4096;
/// Chunks per region edge (regions are 32×32 chunks).
pub const REGION_CHUNKS: u8 = 32;

/// Compression scheme byte preceding chunk NBT data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Compression {
    Gzip,
    Zlib,
    Uncompressed,
}

/// Why reading a region file or one of its chunks failed.
#[derive(Debug)]
pub enum RegionError {
    /// File could not be read.
    Io(std::io::Error),
    /// The file is smaller than the mandatory 8 KiB header.
    HeaderTooShort,
    /// A chunk location points outside the file.
    ChunkOutOfBounds,
    /// Unsupported compression byte (LZ4 and external chunks are not yet supported).
    UnsupportedCompression(u8),
    /// Chunk data ends before the declared length.
    TruncatedChunk,
}

impl fmt::Display for RegionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegionError::Io(err) => write!(f, "I/O error: {err}"),
            RegionError::HeaderTooShort => write!(f, "region file smaller than 8 KiB header"),
            RegionError::ChunkOutOfBounds => write!(f, "chunk location points outside the file"),
            RegionError::UnsupportedCompression(kind) => {
                write!(
                    f,
                    "unsupported chunk compression {kind} (LZ4/external not yet supported)"
                )
            }
            RegionError::TruncatedChunk => write!(f, "chunk data ends before its declared length"),
        }
    }
}

impl std::error::Error for RegionError {}

impl From<std::io::Error> for RegionError {
    fn from(err: std::io::Error) -> Self {
        RegionError::Io(err)
    }
}

/// A parsed region file. Holds an open file handle and reads chunks on demand,
/// mirroring the robust disk-streaming approaches of reference tools.
pub struct RegionFile {
    file: File,
    locations: Vec<u32>, // 1024 chunk offsets + sector counts
}

impl RegionFile {
    /// Read a region file from disk.
    pub fn open(path: &Path) -> Result<Self, RegionError> {
        let mut file = File::open(path)?;
        let metadata = file.metadata()?;
        if metadata.len() < 2 * SECTOR_BYTES as u64 {
            return Err(RegionError::HeaderTooShort);
        }

        let mut header = vec![0u8; SECTOR_BYTES];
        file.read_exact(&mut header)?;

        // Pre-parse the location table (1024 entries, 4 bytes each)
        let mut locations = Vec::with_capacity(1024);
        for i in 0..1024 {
            let offset = i * 4;
            let val = u32::from_be_bytes([
                header[offset],
                header[offset + 1],
                header[offset + 2],
                header[offset + 3],
            ]);
            locations.push(val);
        }

        Ok(Self { file, locations })
    }

    /// Parse a region file from memory (kept for tests).
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, RegionError> {
        use std::io::Write;
        let path = std::env::temp_dir().join(format!(
            "region_test_{}.mca",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut temp = std::fs::File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)?;
        temp.write_all(&bytes)?;
        temp.seek(SeekFrom::Start(0))?;

        if bytes.len() < 2 * SECTOR_BYTES {
            return Err(RegionError::HeaderTooShort);
        }

        let mut locations = Vec::with_capacity(1024);
        for i in 0..1024 {
            let offset = i * 4;
            let val = u32::from_be_bytes([
                bytes[offset],
                bytes[offset + 1],
                bytes[offset + 2],
                bytes[offset + 3],
            ]);
            locations.push(val);
        }

        Ok(Self {
            file: temp,
            locations,
        })
    }

    /// The (sector offset, sector count) location of chunk `(x, z)` —
    /// local coordinates 0..32 — or `None` if the chunk was never generated.
    fn location(&self, x: u8, z: u8) -> Option<(usize, usize)> {
        debug_assert!(x < REGION_CHUNKS && z < REGION_CHUNKS);
        let index = x as usize + z as usize * REGION_CHUNKS as usize;
        let entry = self.locations[index];
        let offset = (entry >> 8) as usize;
        let sectors = (entry & 0xFF) as usize;
        (offset > 0 && sectors > 0).then_some((offset, sectors))
    }

    /// Coordinates of every chunk present in this region (local 0..32).
    pub fn present_chunks(&self) -> Vec<(u8, u8)> {
        let mut chunks = Vec::new();
        for z in 0..REGION_CHUNKS {
            for x in 0..REGION_CHUNKS {
                if self.location(x, z).is_some() {
                    chunks.push((x, z));
                }
            }
        }
        chunks
    }

    /// Decompressed NBT bytes of chunk `(x, z)`, or `None` if absent.
    pub fn chunk_nbt(&mut self, x: u8, z: u8) -> Option<Result<Vec<u8>, RegionError>> {
        let (offset, _sectors) = self.location(x, z)?;
        let start = (offset * SECTOR_BYTES) as u64;

        if let Err(e) = self.file.seek(SeekFrom::Start(start)) {
            return Some(Err(RegionError::Io(e)));
        }

        let mut header = [0u8; 5];
        if let Err(e) = self.file.read_exact(&mut header) {
            // EOF or unreadable
            return Some(Err(RegionError::Io(e)));
        }

        let length = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
        if length == 0 {
            return Some(Err(RegionError::TruncatedChunk));
        }

        let compression = match header[4] {
            1 => Compression::Gzip,
            2 => Compression::Zlib,
            3 => Compression::Uncompressed,
            other => return Some(Err(RegionError::UnsupportedCompression(other))),
        };

        let payload_len = length.saturating_sub(1);
        let mut payload = vec![0u8; payload_len];
        if let Err(e) = self.file.read_exact(&mut payload) {
            return Some(Err(RegionError::Io(e)));
        }

        Some(decompress(compression, &payload))
    }
}

/// Decompress one chunk payload.
fn decompress(compression: Compression, payload: &[u8]) -> Result<Vec<u8>, RegionError> {
    let mut out = Vec::new();
    match compression {
        Compression::Gzip => {
            flate2::read::GzDecoder::new(payload).read_to_end(&mut out)?;
        }
        Compression::Zlib => {
            flate2::read::ZlibDecoder::new(payload).read_to_end(&mut out)?;
        }
        Compression::Uncompressed => out.extend_from_slice(payload),
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Build a minimal region file holding `chunks` as zlib NBT payloads.
    fn build_region(chunks: &[(u8, u8, Vec<u8>)]) -> Vec<u8> {
        let mut file = vec![0u8; 2 * SECTOR_BYTES];
        let mut next_sector = 2;
        for (x, z, nbt) in chunks {
            let mut encoder =
                flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
            encoder.write_all(nbt).unwrap();
            let payload = encoder.finish().unwrap();
            let length = (payload.len() + 1) as u32;
            let sectors = (4 + length as usize).div_ceil(SECTOR_BYTES);

            let index = 4 * (*x as usize + *z as usize * 32);
            file[index] = ((next_sector >> 16) & 0xFF) as u8;
            file[index + 1] = ((next_sector >> 8) & 0xFF) as u8;
            file[index + 2] = (next_sector & 0xFF) as u8;
            file[index + 3] = sectors as u8;

            let start = next_sector * SECTOR_BYTES;
            file.resize(start + sectors * SECTOR_BYTES, 0);
            file[start..start + 4].copy_from_slice(&length.to_be_bytes());
            file[start + 4] = 2; // zlib
            file[start + 5..start + 5 + payload.len()].copy_from_slice(&payload);
            next_sector += sectors;
        }
        file
    }

    #[test]
    fn reads_back_a_zlib_chunk() {
        let nbt = b"fake-nbt-payload".to_vec();
        let bytes = build_region(&[(3, 7, nbt.clone())]);
        let mut region = RegionFile::from_bytes(bytes).unwrap();

        assert_eq!(region.present_chunks(), vec![(3, 7)]);
        assert_eq!(region.chunk_nbt(3, 7).unwrap().unwrap(), nbt);
        assert!(region.chunk_nbt(0, 0).is_none());
    }

    #[test]
    fn rejects_truncated_header() {
        assert!(matches!(
            RegionFile::from_bytes(vec![0u8; 100]),
            Err(RegionError::HeaderTooShort)
        ));
    }

    #[test]
    fn rejects_unknown_compression() {
        let mut bytes = build_region(&[(1, 1, b"data".to_vec())]);
        // Corrupt the compression byte (first chunk is at sector 2).
        bytes[2 * SECTOR_BYTES + 4] = 99;
        let mut region = RegionFile::from_bytes(bytes).unwrap();
        assert!(matches!(
            region.chunk_nbt(1, 1),
            Some(Err(RegionError::UnsupportedCompression(99)))
        ));
    }
}
