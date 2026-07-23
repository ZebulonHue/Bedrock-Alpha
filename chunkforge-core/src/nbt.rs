//! Hand-rolled NBT (Named Binary Tag) reader — port of `src/lib/mc/nbt.ts`.
//!
//! Big-endian, all 12 tag types. Compounds keep insertion order
//! (`Vec<(String, Nbt)>`), lists become `Vec<Nbt>`, long arrays `Vec<i64>`.
//! Every read is bounds-checked; any truncation or type violation becomes
//! [`ParseError::CorruptNbt`] (the TS reader throws `RangeError` mid-parse and
//! the per-chunk catch normalizes it the same way).

use crate::error::ParseError;
use byteorder::{BigEndian, ReadBytesExt};
use std::io::Cursor;

pub const NBT_END: u8 = 0;
pub const NBT_BYTE: u8 = 1;
pub const NBT_SHORT: u8 = 2;
pub const NBT_INT: u8 = 3;
pub const NBT_LONG: u8 = 4;
pub const NBT_FLOAT: u8 = 5;
pub const NBT_DOUBLE: u8 = 6;
pub const NBT_BYTE_ARRAY: u8 = 7;
pub const NBT_STRING: u8 = 8;
pub const NBT_LIST: u8 = 9;
pub const NBT_COMPOUND: u8 = 10;
pub const NBT_INT_ARRAY: u8 = 11;
pub const NBT_LONG_ARRAY: u8 = 12;

/// Defensive nesting cap. Vanilla chunk NBT is ~10 levels deep; a hostile file
/// could otherwise overflow the stack (the TS reader would throw a catchable
/// stack overflow — Rust aborts, so we stop first).
const MAX_DEPTH: usize = 512;

/// One NBT payload.
#[derive(Debug, Clone, PartialEq)]
pub enum Nbt {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    ByteArray(Vec<u8>),
    String(String),
    List(Vec<Nbt>),
    Compound(Vec<(String, Nbt)>),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}

impl Nbt {
    /// Look up `key` in a compound payload (`None` for non-compounds/misses).
    pub fn get(&self, key: &str) -> Option<&Nbt> {
        match self {
            Nbt::Compound(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// Integer value of any integer-width tag (mirrors TS `typeof x === 'number'`).
    pub fn as_i32(&self) -> Option<i32> {
        match self {
            Nbt::Byte(v) => Some(*v as i32),
            Nbt::Short(v) => Some(*v as i32),
            Nbt::Int(v) => Some(*v),
            Nbt::Long(v) => Some(*v as i32),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Nbt::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_list(&self) -> Option<&[Nbt]> {
        match self {
            Nbt::List(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_long_array(&self) -> Option<&[i64]> {
        match self {
            Nbt::LongArray(v) => Some(v),
            _ => None,
        }
    }
}

struct Reader<'a> {
    cur: Cursor<&'a [u8]>,
}

impl<'a> Reader<'a> {
    fn offset(&self) -> u64 {
        self.cur.position()
    }

    fn remaining(&self) -> usize {
        self.cur.get_ref().len() - self.cur.position() as usize
    }

    fn err(&self, msg: impl Into<String>) -> ParseError {
        ParseError::CorruptNbt(format!("{} at offset {}", msg.into(), self.offset()))
    }

    fn read_exact(&mut self, n: usize, what: &str) -> Result<Vec<u8>, ParseError> {
        if n > self.remaining() {
            return Err(self.err(format!("NBT: truncated {what}")));
        }
        let pos = self.cur.position() as usize;
        let out = self.cur.get_ref()[pos..pos + n].to_vec();
        self.cur.set_position((pos + n) as u64);
        Ok(out)
    }

    fn u8(&mut self) -> Result<u8, ParseError> {
        self.cur
            .read_u8()
            .map_err(|_| self.err("NBT: truncated tag"))
    }

    fn i32(&mut self) -> Result<i32, ParseError> {
        self.cur
            .read_i32::<BigEndian>()
            .map_err(|_| self.err("NBT: truncated int"))
    }

    fn str(&mut self) -> Result<String, ParseError> {
        let n = self
            .cur
            .read_u16::<BigEndian>()
            .map_err(|_| self.err("NBT: truncated string length"))? as usize;
        let bytes = self.read_exact(n, "string")?;
        // TS uses TextDecoder (lossy UTF-8 with replacement) — match it.
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    /// Length prefix for a variable-size payload: rejects negatives and sizes
    /// that cannot possibly fit (every element costs at least `elem` bytes),
    /// so a hostile length can never trigger a giant allocation.
    fn len_prefix(&mut self, elem: usize, what: &str) -> Result<usize, ParseError> {
        let n = self.i32()?;
        if n < 0 {
            return Err(self.err(format!("NBT: negative {what} length {n}")));
        }
        let n = n as usize;
        if n.checked_mul(elem)
            .map_or(true, |need| need > self.remaining())
        {
            return Err(self.err(format!("NBT: {what} length {n} overruns the data")));
        }
        Ok(n)
    }

    fn payload(&mut self, tag: u8, depth: usize) -> Result<Nbt, ParseError> {
        if depth > MAX_DEPTH {
            return Err(self.err("NBT: nesting too deep"));
        }
        match tag {
            NBT_BYTE => Ok(Nbt::Byte(
                self.cur
                    .read_i8()
                    .map_err(|_| self.err("NBT: truncated byte"))?,
            )),
            NBT_SHORT => Ok(Nbt::Short(
                self.cur
                    .read_i16::<BigEndian>()
                    .map_err(|_| self.err("NBT: truncated short"))?,
            )),
            NBT_INT => Ok(Nbt::Int(self.i32()?)),
            NBT_LONG => Ok(Nbt::Long(
                self.cur
                    .read_i64::<BigEndian>()
                    .map_err(|_| self.err("NBT: truncated long"))?,
            )),
            NBT_FLOAT => Ok(Nbt::Float(
                self.cur
                    .read_f32::<BigEndian>()
                    .map_err(|_| self.err("NBT: truncated float"))?,
            )),
            NBT_DOUBLE => Ok(Nbt::Double(
                self.cur
                    .read_f64::<BigEndian>()
                    .map_err(|_| self.err("NBT: truncated double"))?,
            )),
            NBT_BYTE_ARRAY => {
                let n = self.len_prefix(1, "byte array")?;
                Ok(Nbt::ByteArray(self.read_exact(n, "byte array")?))
            }
            NBT_STRING => Ok(Nbt::String(self.str()?)),
            NBT_LIST => {
                let subtype = self.u8()?;
                let n = self.len_prefix(1, "list")?;
                if n == 0 {
                    // TS never touches the subtype when the list is empty.
                    return Ok(Nbt::List(Vec::new()));
                }
                if !(NBT_BYTE..=NBT_LONG_ARRAY).contains(&subtype) {
                    return Err(self.err(format!("NBT: unknown tag type {subtype}")));
                }
                let mut out = Vec::with_capacity(n);
                for _ in 0..n {
                    out.push(self.payload(subtype, depth + 1)?);
                }
                Ok(Nbt::List(out))
            }
            NBT_COMPOUND => {
                let mut out = Vec::new();
                loop {
                    let t = self.u8()?;
                    if t == NBT_END {
                        break;
                    }
                    let name = self.str()?;
                    out.push((name, self.payload(t, depth + 1)?));
                }
                Ok(Nbt::Compound(out))
            }
            NBT_INT_ARRAY => {
                let n = self.len_prefix(4, "int array")?;
                let mut out = Vec::with_capacity(n);
                for _ in 0..n {
                    out.push(self.i32()?);
                }
                Ok(Nbt::IntArray(out))
            }
            NBT_LONG_ARRAY => {
                let n = self.len_prefix(8, "long array")?;
                let mut out = Vec::with_capacity(n);
                for _ in 0..n {
                    out.push(
                        self.cur
                            .read_i64::<BigEndian>()
                            .map_err(|_| self.err("NBT: truncated long"))?,
                    );
                }
                Ok(Nbt::LongArray(out))
            }
            other => Err(self.err(format!("NBT: unknown tag type {other}"))),
        }
    }
}

/// Parse a full NBT document. The root must be a (named) compound, per the
/// format; the root name is discarded, like the TS reader.
pub fn parse_nbt(bytes: &[u8]) -> Result<Nbt, ParseError> {
    let mut r = Reader {
        cur: Cursor::new(bytes),
    };
    let root_type = r.u8()?;
    if root_type != NBT_COMPOUND {
        return Err(ParseError::CorruptNbt(format!(
            "NBT: expected root compound (tag 10), got {root_type}"
        )));
    }
    r.str()?; // root name (usually empty) — discarded
    r.payload(NBT_COMPOUND, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use byteorder::{BigEndian, WriteBytesExt};

    /// Build a named-compound document by hand.
    fn root(entries: &[u8]) -> Vec<u8> {
        let mut v = vec![NBT_COMPOUND, 0, 0]; // tag 10, empty root name
        v.extend_from_slice(entries);
        v.push(NBT_END);
        v
    }

    fn named(tag: u8, name: &str, payload: &[u8]) -> Vec<u8> {
        let mut v = vec![tag];
        v.write_u16::<BigEndian>(name.len() as u16).unwrap();
        v.extend_from_slice(name.as_bytes());
        v.extend_from_slice(payload);
        v
    }

    #[test]
    fn roundtrip_all_tags() {
        let mut body = Vec::new();
        body.extend(named(NBT_BYTE, "b", &[-5i8 as u8]));
        body.extend(named(NBT_SHORT, "s", &(-1234i16).to_be_bytes()));
        body.extend(named(NBT_INT, "i", &(-123456i32).to_be_bytes()));
        body.extend(named(NBT_LONG, "l", &(-1234567890123i64).to_be_bytes()));
        body.extend(named(NBT_FLOAT, "f", &1.5f32.to_be_bytes()));
        body.extend(named(NBT_DOUBLE, "d", &(-2.25f64).to_be_bytes()));

        let mut ba = Vec::new();
        ba.write_i32::<BigEndian>(3).unwrap();
        ba.extend_from_slice(&[1, 2, 3]);
        body.extend(named(NBT_BYTE_ARRAY, "ba", &ba));

        let mut st = Vec::new();
        st.write_u16::<BigEndian>(5).unwrap();
        st.extend_from_slice(b"hello");
        body.extend(named(NBT_STRING, "str", &st));

        // list of 2 shorts
        let mut li = Vec::new();
        li.push(NBT_SHORT);
        li.write_i32::<BigEndian>(2).unwrap();
        li.extend_from_slice(&10i16.to_be_bytes());
        li.extend_from_slice(&(-20i16).to_be_bytes());
        body.extend(named(NBT_LIST, "li", &li));

        // nested compound { x: int 7 }
        let mut co = named(NBT_INT, "x", &7i32.to_be_bytes());
        co.push(NBT_END);
        body.extend(named(NBT_COMPOUND, "co", &co));

        let mut ia = Vec::new();
        ia.write_i32::<BigEndian>(2).unwrap();
        ia.extend_from_slice(&100i32.to_be_bytes());
        ia.extend_from_slice(&200i32.to_be_bytes());
        body.extend(named(NBT_INT_ARRAY, "ia", &ia));

        let mut la = Vec::new();
        la.write_i32::<BigEndian>(2).unwrap();
        la.extend_from_slice(&(-1i64).to_be_bytes());
        la.extend_from_slice(&(1i64 << 62).to_be_bytes());
        body.extend(named(NBT_LONG_ARRAY, "la", &la));

        let nbt = parse_nbt(&root(&body)).expect("parse");
        assert_eq!(nbt.get("b"), Some(&Nbt::Byte(-5)));
        assert_eq!(nbt.get("s"), Some(&Nbt::Short(-1234)));
        assert_eq!(nbt.get("i"), Some(&Nbt::Int(-123456)));
        assert_eq!(nbt.get("l"), Some(&Nbt::Long(-1234567890123)));
        assert_eq!(nbt.get("f"), Some(&Nbt::Float(1.5)));
        assert_eq!(nbt.get("d"), Some(&Nbt::Double(-2.25)));
        assert_eq!(nbt.get("ba"), Some(&Nbt::ByteArray(vec![1, 2, 3])));
        assert_eq!(nbt.get("str"), Some(&Nbt::String("hello".into())));
        assert_eq!(
            nbt.get("li"),
            Some(&Nbt::List(vec![Nbt::Short(10), Nbt::Short(-20)]))
        );
        assert_eq!(
            nbt.get("co"),
            Some(&Nbt::Compound(vec![("x".into(), Nbt::Int(7))]))
        );
        assert_eq!(nbt.get("ia"), Some(&Nbt::IntArray(vec![100, 200])));
        assert_eq!(nbt.get("la"), Some(&Nbt::LongArray(vec![-1, 1i64 << 62])));
        assert_eq!(nbt.get("i").and_then(Nbt::as_i32), Some(-123456));
        assert_eq!(nbt.get("b").and_then(Nbt::as_i32), Some(-5));
    }

    #[test]
    fn rejects_bad_root() {
        let err = parse_nbt(&[NBT_BYTE, 0, 0]).unwrap_err();
        assert!(matches!(err, ParseError::CorruptNbt(_)));
        assert!(err.to_string().contains("expected root compound"));
    }

    #[test]
    fn rejects_truncation_and_hostile_lengths() {
        // truncated string payload
        assert!(parse_nbt(&root(&named(NBT_STRING, "s", &[0, 10, b'a']))).is_err());
        // negative list length
        let mut li = vec![NBT_BYTE];
        li.extend_from_slice(&(-1i32).to_be_bytes());
        assert!(parse_nbt(&root(&named(NBT_LIST, "l", &li))).is_err());
        // absurd byte-array length (overruns data)
        let mut ba = Vec::new();
        ba.extend_from_slice(&(1_000_000i32).to_be_bytes());
        assert!(parse_nbt(&root(&named(NBT_BYTE_ARRAY, "b", &ba))).is_err());
        // empty buffer
        assert!(parse_nbt(&[]).is_err());
        // unknown tag inside compound
        let mut body = vec![77u8, 0, 1, b'x'];
        body.push(NBT_END);
        assert!(parse_nbt(&root(&body)).is_err());
    }

    #[test]
    fn empty_list_ignores_subtype() {
        // TS: n == 0 lists never validate the subtype (tag 0 = END here).
        let mut li = vec![NBT_END];
        li.extend_from_slice(&0i32.to_be_bytes());
        let nbt = parse_nbt(&root(&named(NBT_LIST, "l", &li))).expect("parse");
        assert_eq!(nbt.get("l"), Some(&Nbt::List(vec![])));
    }
}
