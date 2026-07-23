//! Minimal little-endian NBT reader for Bedrock Edition data (`level.dat`,
//! subchunk palettes, player records). Java NBT is big-endian and handled
//! by `fastnbt`; Bedrock stores the same format with little-endian numbers
//! and may concatenate several root tags in one payload.
//!
//! Only what the world pipeline needs: tag values as a small owned tree,
//! plus a cursor that can read consecutive roots.

use std::collections::HashMap;
use std::fmt;

/// A parsed NBT value (little-endian variant).
#[derive(Debug, Clone, PartialEq)]
pub enum NbtValue {
    /// `TAG_Byte`.
    Byte(i8),
    /// `TAG_Short`.
    Short(i16),
    /// `TAG_Int`.
    Int(i32),
    /// `TAG_Long`.
    Long(i64),
    /// `TAG_Float`.
    Float(f32),
    /// `TAG_Double`.
    Double(f64),
    /// `TAG_Byte_Array`.
    ByteArray(Vec<i8>),
    /// `TAG_String`.
    String(String),
    /// `TAG_List`.
    List(Vec<NbtValue>),
    /// `TAG_Compound`.
    Compound(HashMap<String, NbtValue>),
    /// `TAG_Int_Array`.
    IntArray(Vec<i32>),
    /// `TAG_Long_Array`.
    LongArray(Vec<i64>),
}

impl NbtValue {
    /// The compound map, when this value is a compound.
    pub fn as_compound(&self) -> Option<&HashMap<String, NbtValue>> {
        match self {
            NbtValue::Compound(map) => Some(map),
            _ => None,
        }
    }

    /// The string, when this value is a string.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            NbtValue::String(s) => Some(s),
            _ => None,
        }
    }
}

/// Why an LE NBT payload could not be parsed.
#[derive(Debug)]
pub struct NbtLeError(pub String);

impl fmt::Display for NbtLeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid little-endian NBT: {}", self.0)
    }
}

impl std::error::Error for NbtLeError {}

/// A byte cursor over an LE NBT payload. Create one per payload and call
/// [`NbtCursor::read_root`] once per root tag.
pub struct NbtCursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> NbtCursor<'a> {
    /// Start reading at the beginning of `bytes`.
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    /// Bytes consumed so far.
    pub fn position(&self) -> usize {
        self.pos
    }

    /// True when no bytes remain.
    pub fn is_empty(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    /// Read one root tag (`type`, `name`, payload) and return its value.
    pub fn read_root(&mut self) -> Result<NbtValue, NbtLeError> {
        let tag = self.u8()?;
        if tag == 0 {
            return Err(NbtLeError("unexpected TAG_End at root".into()));
        }
        let name_len = self.u16()? as usize;
        let _name = self.take(name_len)?;
        self.payload(tag)
    }

    fn payload(&mut self, tag: u8) -> Result<NbtValue, NbtLeError> {
        Ok(match tag {
            1 => NbtValue::Byte(self.u8()? as i8),
            2 => NbtValue::Short(self.i16()?),
            3 => NbtValue::Int(self.i32()?),
            4 => NbtValue::Long(self.i64()?),
            5 => NbtValue::Float(self.f32()?),
            6 => NbtValue::Double(self.f64()?),
            7 => {
                let len = self.i32()? as usize;
                NbtValue::ByteArray(self.take(len)?.iter().map(|b| *b as i8).collect())
            }
            8 => {
                let len = self.u16()? as usize;
                let raw = self.take(len)?;
                NbtValue::String(String::from_utf8_lossy(raw).into_owned())
            }
            9 => {
                let item_tag = self.u8()?;
                let len = self.i32()?;
                if len < 0 {
                    return Err(NbtLeError("negative list length".into()));
                }
                let mut items = Vec::with_capacity(len as usize);
                for _ in 0..len {
                    items.push(self.payload(item_tag)?);
                }
                NbtValue::List(items)
            }
            10 => {
                let mut map = HashMap::new();
                loop {
                    let item_tag = self.u8()?;
                    if item_tag == 0 {
                        break;
                    }
                    let name_len = self.u16()? as usize;
                    let name = String::from_utf8_lossy(self.take(name_len)?).into_owned();
                    map.insert(name, self.payload(item_tag)?);
                }
                NbtValue::Compound(map)
            }
            11 => {
                let len = self.i32()? as usize;
                let mut items = Vec::with_capacity(len);
                for _ in 0..len {
                    items.push(self.i32()?);
                }
                NbtValue::IntArray(items)
            }
            12 => {
                let len = self.i32()? as usize;
                let mut items = Vec::with_capacity(len);
                for _ in 0..len {
                    items.push(self.i64()?);
                }
                NbtValue::LongArray(items)
            }
            other => return Err(NbtLeError(format!("unknown tag id {other}"))),
        })
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], NbtLeError> {
        let end = self
            .pos
            .checked_add(len)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| NbtLeError("payload truncated".into()))?;
        let slice = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, NbtLeError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, NbtLeError> {
        Ok(u16::from_le_bytes(
            self.take(2)?.try_into().expect("2 bytes"),
        ))
    }

    fn i16(&mut self) -> Result<i16, NbtLeError> {
        Ok(i16::from_le_bytes(
            self.take(2)?.try_into().expect("2 bytes"),
        ))
    }

    fn i32(&mut self) -> Result<i32, NbtLeError> {
        Ok(i32::from_le_bytes(
            self.take(4)?.try_into().expect("4 bytes"),
        ))
    }

    fn i64(&mut self) -> Result<i64, NbtLeError> {
        Ok(i64::from_le_bytes(
            self.take(8)?.try_into().expect("8 bytes"),
        ))
    }

    fn f32(&mut self) -> Result<f32, NbtLeError> {
        Ok(f32::from_le_bytes(
            self.take(4)?.try_into().expect("4 bytes"),
        ))
    }

    fn f64(&mut self) -> Result<f64, NbtLeError> {
        Ok(f64::from_le_bytes(
            self.take(8)?.try_into().expect("8 bytes"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a compound `{name: "minecraft:stone", version: 4}` by hand.
    fn sample_compound() -> Vec<u8> {
        let mut bytes = vec![10, 0, 0]; // TAG_Compound, empty root name
        bytes.push(8); // TAG_String "name"
        bytes.extend_from_slice(&4u16.to_le_bytes());
        bytes.extend_from_slice(b"name");
        bytes.extend_from_slice(&15u16.to_le_bytes());
        bytes.extend_from_slice(b"minecraft:stone");
        bytes.push(3); // TAG_Int "version"
        bytes.extend_from_slice(&7u16.to_le_bytes());
        bytes.extend_from_slice(b"version");
        bytes.extend_from_slice(&4i32.to_le_bytes());
        bytes.push(0); // TAG_End
        bytes
    }

    #[test]
    fn reads_a_compound() {
        let bytes = sample_compound();
        let mut cursor = NbtCursor::new(&bytes);
        let value = cursor.read_root().unwrap();
        let map = value.as_compound().unwrap();
        assert_eq!(map["name"].as_str(), Some("minecraft:stone"));
        assert_eq!(map["version"], NbtValue::Int(4));
        assert!(cursor.is_empty());
    }

    #[test]
    fn reads_consecutive_roots() {
        let mut bytes = sample_compound();
        bytes.extend_from_slice(&sample_compound());
        let mut cursor = NbtCursor::new(&bytes);
        assert!(cursor.read_root().is_ok());
        assert!(cursor.read_root().is_ok());
        assert!(cursor.is_empty());
    }

    #[test]
    fn truncated_payload_is_an_error() {
        let mut cursor = NbtCursor::new(&[10, 0, 0, 8, 5, 0, b'n']);
        assert!(cursor.read_root().is_err());
    }
}
