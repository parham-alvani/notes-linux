//! MessagePack, the wire format of Neovim's RPC. Only what that protocol
//! sends: nil, booleans, integers, floats, strings, binary, arrays, maps, and
//! the ext type Neovim uses for buffer, window and tab handles.

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum MsgValue {
    Nil,
    Bool(bool),
    Int(i64),
    Uint(u64),
    Double(f64),
    String(String),
    Binary(Vec<u8>),
    Array(Vec<MsgValue>),
    /// In the order the bytes held them.
    Map(Vec<(MsgValue, MsgValue)>),
    Ext(i8, Vec<u8>),
}

impl MsgValue {
    pub fn as_int(&self) -> Option<i64> {
        match self {
            MsgValue::Int(value) => Some(*value),
            MsgValue::Uint(value) => i64::try_from(*value).ok(),
            // A handle (buffer, window) is an ext holding a packed integer.
            MsgValue::Ext(_, data) => MessagePack::decode(data).ok()?.as_int(),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            MsgValue::String(text) => Some(text),
            MsgValue::Binary(data) => std::str::from_utf8(data).ok(),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[MsgValue]> {
        match self {
            MsgValue::Array(items) => Some(items),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            MsgValue::Bool(flag) => Some(*flag),
            _ => None,
        }
    }

    pub fn get(&self, key: &str) -> Option<&MsgValue> {
        match self {
            MsgValue::Map(entries) => entries.iter().find(|(k, _)| k.as_str() == Some(key)).map(|(_, v)| v),
            _ => None,
        }
    }
}

impl From<&str> for MsgValue {
    fn from(text: &str) -> MsgValue {
        MsgValue::String(text.into())
    }
}

impl From<String> for MsgValue {
    fn from(text: String) -> MsgValue {
        MsgValue::String(text)
    }
}

impl From<i64> for MsgValue {
    fn from(value: i64) -> MsgValue {
        MsgValue::Int(value)
    }
}

impl From<bool> for MsgValue {
    fn from(flag: bool) -> MsgValue {
        MsgValue::Bool(flag)
    }
}

impl<T: Into<MsgValue>> From<Vec<T>> for MsgValue {
    fn from(items: Vec<T>) -> MsgValue {
        MsgValue::Array(items.into_iter().map(Into::into).collect())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MsgFailure {
    Truncated,
    UnknownType(u8),
}

impl fmt::Display for MsgFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MsgFailure::Truncated => write!(f, "a MessagePack value stops early"),
            MsgFailure::UnknownType(byte) => write!(f, "MessagePack type {byte:#04x} is not known"),
        }
    }
}

impl std::error::Error for MsgFailure {}

pub struct MessagePack;

impl MessagePack {
    pub fn encode(value: &MsgValue) -> Vec<u8> {
        let mut out = Vec::new();
        write(value, &mut out);
        out
    }

    /// One whole value from [data].
    pub fn decode(data: &[u8]) -> Result<MsgValue, MsgFailure> {
        Reader { bytes: data, at: 0 }.value()
    }
}

fn write(value: &MsgValue, out: &mut Vec<u8>) {
    match value {
        MsgValue::Nil => out.push(0xC0),
        MsgValue::Bool(flag) => out.push(if *flag { 0xC3 } else { 0xC2 }),
        MsgValue::Int(number) => {
            let n = *number;
            if n >= 0 {
                write(&MsgValue::Uint(n as u64), out);
            } else if n >= -32 {
                out.push(n as i8 as u8);
            } else if n >= i64::from(i8::MIN) {
                out.extend([0xD0, n as i8 as u8]);
            } else if n >= i64::from(i16::MIN) {
                out.push(0xD1);
                out.extend((n as i16).to_be_bytes());
            } else if n >= i64::from(i32::MIN) {
                out.push(0xD2);
                out.extend((n as i32).to_be_bytes());
            } else {
                out.push(0xD3);
                out.extend(n.to_be_bytes());
            }
        }
        MsgValue::Uint(number) => {
            let n = *number;
            if n < 0x80 {
                out.push(n as u8);
            } else if u8::try_from(n).is_ok() {
                out.extend([0xCC, n as u8]);
            } else if u16::try_from(n).is_ok() {
                out.push(0xCD);
                out.extend((n as u16).to_be_bytes());
            } else if u32::try_from(n).is_ok() {
                out.push(0xCE);
                out.extend((n as u32).to_be_bytes());
            } else {
                out.push(0xCF);
                out.extend(n.to_be_bytes());
            }
        }
        MsgValue::Double(number) => {
            out.push(0xCB);
            out.extend(number.to_bits().to_be_bytes());
        }
        MsgValue::String(text) => {
            let n = text.len();
            if n < 32 {
                out.push(0xA0 | n as u8);
            } else if u8::try_from(n).is_ok() {
                out.extend([0xD9, n as u8]);
            } else if u16::try_from(n).is_ok() {
                out.push(0xDA);
                out.extend((n as u16).to_be_bytes());
            } else {
                out.push(0xDB);
                out.extend((n as u32).to_be_bytes());
            }
            out.extend(text.as_bytes());
        }
        MsgValue::Binary(data) => {
            let n = data.len();
            if u8::try_from(n).is_ok() {
                out.extend([0xC4, n as u8]);
            } else if u16::try_from(n).is_ok() {
                out.push(0xC5);
                out.extend((n as u16).to_be_bytes());
            } else {
                out.push(0xC6);
                out.extend((n as u32).to_be_bytes());
            }
            out.extend(data);
        }
        MsgValue::Array(items) => {
            let n = items.len();
            if n < 16 {
                out.push(0x90 | n as u8);
            } else if u16::try_from(n).is_ok() {
                out.push(0xDC);
                out.extend((n as u16).to_be_bytes());
            } else {
                out.push(0xDD);
                out.extend((n as u32).to_be_bytes());
            }
            for item in items {
                write(item, out);
            }
        }
        MsgValue::Map(entries) => {
            let n = entries.len();
            if n < 16 {
                out.push(0x80 | n as u8);
            } else if u16::try_from(n).is_ok() {
                out.push(0xDE);
                out.extend((n as u16).to_be_bytes());
            } else {
                out.push(0xDF);
                out.extend((n as u32).to_be_bytes());
            }
            for (key, item) in entries {
                write(key, out);
                write(item, out);
            }
        }
        MsgValue::Ext(kind, data) => {
            match data.len() {
                1 => out.push(0xD4),
                2 => out.push(0xD5),
                4 => out.push(0xD6),
                8 => out.push(0xD7),
                16 => out.push(0xD8),
                n if u8::try_from(n).is_ok() => out.extend([0xC7, n as u8]),
                n if u16::try_from(n).is_ok() => {
                    out.push(0xC8);
                    out.extend((n as u16).to_be_bytes());
                }
                n => {
                    out.push(0xC9);
                    out.extend((n as u32).to_be_bytes());
                }
            }
            out.push(*kind as u8);
            out.extend(data);
        }
    }
}

/// Values from a stream that may stop in the middle of one. Feed it what
/// arrives; it hands back every value that is now complete.
#[derive(Debug, Default)]
pub struct MsgStream {
    buffer: Vec<u8>,
}

impl MsgStream {
    pub fn new() -> MsgStream {
        MsgStream::default()
    }

    pub fn feed(&mut self, data: &[u8]) -> Vec<MsgValue> {
        self.buffer.extend_from_slice(data);
        let mut out = Vec::new();
        let mut used = 0;
        while used < self.buffer.len() {
            let mut reader = Reader { bytes: &self.buffer[used..], at: 0 };
            let Ok(value) = reader.value() else { break };
            out.push(value);
            used += reader.at;
        }
        self.buffer.drain(..used);
        out
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, count: usize) -> Result<&[u8], MsgFailure> {
        let end = self.at.checked_add(count).filter(|e| *e <= self.bytes.len()).ok_or(MsgFailure::Truncated)?;
        let out = &self.bytes[self.at..end];
        self.at = end;
        Ok(out)
    }

    fn byte(&mut self) -> Result<u8, MsgFailure> {
        Ok(self.take(1)?[0])
    }

    fn number(&mut self, size: usize) -> Result<u64, MsgFailure> {
        Ok(self.take(size)?.iter().fold(0u64, |n, b| (n << 8) | u64::from(*b)))
    }

    fn string(&mut self, count: usize) -> Result<MsgValue, MsgFailure> {
        Ok(MsgValue::String(String::from_utf8_lossy(self.take(count)?).into_owned()))
    }

    fn array(&mut self, count: usize) -> Result<MsgValue, MsgFailure> {
        let mut items = Vec::with_capacity(count.min(1024));
        for _ in 0..count {
            items.push(self.value()?);
        }
        Ok(MsgValue::Array(items))
    }

    fn map(&mut self, count: usize) -> Result<MsgValue, MsgFailure> {
        let mut entries = Vec::with_capacity(count.min(1024));
        for _ in 0..count {
            let key = self.value()?;
            entries.push((key, self.value()?));
        }
        Ok(MsgValue::Map(entries))
    }

    fn ext(&mut self, count: usize) -> Result<MsgValue, MsgFailure> {
        let kind = self.byte()? as i8;
        Ok(MsgValue::Ext(kind, self.take(count)?.to_vec()))
    }

    fn value(&mut self) -> Result<MsgValue, MsgFailure> {
        let head = self.byte()?;
        Ok(match head {
            0x00..=0x7F => MsgValue::Uint(u64::from(head)),
            0x80..=0x8F => return self.map((head & 0x0F) as usize),
            0x90..=0x9F => return self.array((head & 0x0F) as usize),
            0xA0..=0xBF => return self.string((head & 0x1F) as usize),
            0xC0 => MsgValue::Nil,
            0xC2 => MsgValue::Bool(false),
            0xC3 => MsgValue::Bool(true),
            0xC4..=0xC6 => {
                let size = self.number(1 << (head - 0xC4))? as usize;
                MsgValue::Binary(self.take(size)?.to_vec())
            }
            0xC7..=0xC9 => {
                let size = self.number(1 << (head - 0xC7))? as usize;
                return self.ext(size);
            }
            0xCA => MsgValue::Double(f64::from(f32::from_bits(self.number(4)? as u32))),
            0xCB => MsgValue::Double(f64::from_bits(self.number(8)?)),
            0xCC..=0xCF => MsgValue::Uint(self.number(1 << (head - 0xCC))?),
            0xD0 => MsgValue::Int(i64::from(self.number(1)? as u8 as i8)),
            0xD1 => MsgValue::Int(i64::from(self.number(2)? as u16 as i16)),
            0xD2 => MsgValue::Int(i64::from(self.number(4)? as u32 as i32)),
            0xD3 => MsgValue::Int(self.number(8)? as i64),
            0xD4..=0xD8 => return self.ext(1 << (head - 0xD4)),
            0xD9..=0xDB => {
                let size = self.number(1 << (head - 0xD9))? as usize;
                return self.string(size);
            }
            0xDC | 0xDD => {
                let size = self.number(if head == 0xDC { 2 } else { 4 })? as usize;
                return self.array(size);
            }
            0xDE | 0xDF => {
                let size = self.number(if head == 0xDE { 2 } else { 4 })? as usize;
                return self.map(size);
            }
            0xE0..=0xFF => MsgValue::Int(i64::from(head as i8)),
            _ => return Err(MsgFailure::UnknownType(head)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let values = vec![
            MsgValue::Nil,
            true.into(),
            false.into(),
            0i64.into(),
            127i64.into(),
            128i64.into(),
            65_535i64.into(),
            70_000i64.into(),
            (-1i64).into(),
            (-33i64).into(),
            (-200i64).into(),
            (-40_000i64).into(),
            MsgValue::Int(-3_000_000_000),
            MsgValue::Uint(u64::from(u32::MAX) + 5),
            MsgValue::Double(1.5),
            "".into(),
            "short".into(),
            "x".repeat(300).into(),
            "فارسی".into(),
            MsgValue::Binary(vec![1, 2, 3]),
            MsgValue::Array(vec![1i64.into(), "two".into(), vec![3i64].into()]),
            MsgValue::Map(vec![("key".into(), "value".into()), ("n".into(), 2i64.into())]),
            MsgValue::Ext(0, vec![5]),
            vec![1i64; 20].into(),
        ];
        // A positive number is sent in the unsigned form: the same value.
        fn unsigned(value: &MsgValue) -> MsgValue {
            match value {
                MsgValue::Int(n) if *n >= 0 => MsgValue::Uint(*n as u64),
                MsgValue::Array(items) => MsgValue::Array(items.iter().map(unsigned).collect()),
                MsgValue::Map(entries) => MsgValue::Map(entries.iter().map(|(k, v)| (unsigned(k), unsigned(v))).collect()),
                other => other.clone(),
            }
        }
        for value in &values {
            assert_eq!(MessagePack::decode(&MessagePack::encode(value)).unwrap(), unsigned(value), "{value:?}");
        }
    }

    #[test]
    fn stream_waits_for_whole_values() {
        let mut stream = MsgStream::new();
        let mut data = MessagePack::encode(&MsgValue::Array(vec!["a".into(), 1i64.into()]));
        data.extend(MessagePack::encode(&"b".into()));
        assert!(stream.feed(&data[..3]).is_empty());
        let rest = stream.feed(&data[3..]);
        assert_eq!(rest.len(), 2);
        assert_eq!(rest.last(), Some(&"b".into()));
    }

    #[test]
    fn handles_are_integers() {
        assert_eq!(MsgValue::Ext(0, MessagePack::encode(&3i64.into())).as_int(), Some(3));
    }
}
