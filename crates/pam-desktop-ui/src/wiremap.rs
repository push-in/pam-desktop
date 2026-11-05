//! WireMap: the bounded key/value envelope used by event payloads and native
//! module calls. Keys are portable ASCII identifiers in ascending byte order.
//!
//! ```text
//! u16 count · ( u16 key length · key · u8 tag · value )*
//! tag 1 = string (u32 length + UTF-8)   tag 2 = i64   tag 3 = f64
//! tag 4 = bool (u8)                     tag 5 = opaque bytes (u32 length)
//! ```

use std::collections::BTreeMap;

pub const MAX_WIRE_MAP_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub enum WireValue {
    Text(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Bytes(Vec<u8>),
}

impl From<&str> for WireValue {
    fn from(value: &str) -> Self {
        Self::Text(value.to_owned())
    }
}

impl From<String> for WireValue {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<i64> for WireValue {
    fn from(value: i64) -> Self {
        Self::Integer(value)
    }
}

impl From<i32> for WireValue {
    fn from(value: i32) -> Self {
        Self::Integer(i64::from(value))
    }
}

impl From<u32> for WireValue {
    fn from(value: u32) -> Self {
        Self::Integer(i64::from(value))
    }
}

impl From<usize> for WireValue {
    fn from(value: usize) -> Self {
        Self::Integer(i64::try_from(value).unwrap_or(i64::MAX))
    }
}

impl From<f64> for WireValue {
    fn from(value: f64) -> Self {
        Self::Float(value)
    }
}

impl From<f32> for WireValue {
    fn from(value: f32) -> Self {
        Self::Float(f64::from(value))
    }
}

impl From<bool> for WireValue {
    fn from(value: bool) -> Self {
        Self::Boolean(value)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WireMap {
    values: BTreeMap<String, WireValue>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WireMapError {
    Truncated,
    InvalidKey,
    DuplicateKey,
    InvalidValue,
    TrailingBytes,
    TooLarge,
}

impl std::fmt::Display for WireMapError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Truncated => "wire map is truncated",
            Self::InvalidKey => "wire map contains an invalid key",
            Self::DuplicateKey => "wire map contains a duplicate key",
            Self::InvalidValue => "wire map contains an invalid value",
            Self::TrailingBytes => "wire map contains trailing bytes",
            Self::TooLarge => "wire map exceeds one mebibyte",
        })
    }
}

impl std::error::Error for WireMapError {}

fn valid_key(key: &str) -> bool {
    let bytes = key.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 255
        && bytes[0].is_ascii_alphabetic()
        && bytes.iter().all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
}

impl WireMap {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Builder-style insert. Keys must be portable identifiers; invalid keys
    /// and non-finite decimals are programmer errors and are dropped.
    #[must_use]
    pub fn with(mut self, key: &str, value: impl Into<WireValue>) -> Self {
        self.insert(key, value);
        self
    }

    pub fn insert(&mut self, key: &str, value: impl Into<WireValue>) {
        let value = value.into();
        debug_assert!(valid_key(key), "invalid wire map key {key}");
        if !valid_key(key) {
            return;
        }
        if let WireValue::Float(number) = value
            && !number.is_finite() {
                self.values.insert(key.to_owned(), WireValue::Float(0.0));
                return;
            }
        self.values.insert(key.to_owned(), value);
    }

    #[must_use]
    pub fn get(&self, key: &str) -> Option<&WireValue> {
        self.values.get(key)
    }

    #[must_use]
    pub fn text(&self, key: &str) -> Option<&str> {
        match self.values.get(key) {
            Some(WireValue::Text(value)) => Some(value),
            _ => None,
        }
    }

    #[must_use]
    pub fn integer(&self, key: &str) -> Option<i64> {
        match self.values.get(key) {
            Some(WireValue::Integer(value)) => Some(*value),
            Some(WireValue::Float(value)) => Some(*value as i64),
            Some(WireValue::Boolean(value)) => Some(i64::from(*value)),
            _ => None,
        }
    }

    #[must_use]
    pub fn number(&self, key: &str) -> Option<f64> {
        match self.values.get(key) {
            Some(WireValue::Integer(value)) => Some(*value as f64),
            Some(WireValue::Float(value)) => Some(*value),
            _ => None,
        }
    }

    #[must_use]
    pub fn boolean(&self, key: &str) -> Option<bool> {
        match self.values.get(key) {
            Some(WireValue::Boolean(value)) => Some(*value),
            Some(WireValue::Integer(value)) => Some(*value != 0),
            _ => None,
        }
    }

    #[must_use]
    pub fn bytes(&self, key: &str) -> Option<&[u8]> {
        match self.values.get(key) {
            Some(WireValue::Bytes(value)) => Some(value),
            Some(WireValue::Text(value)) => Some(value.as_bytes()),
            _ => None,
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &WireValue)> {
        self.values.iter().map(|(key, value)| (key.as_str(), value))
    }

    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut output = Vec::with_capacity(2 + self.values.len() * 24);
        let count = u16::try_from(self.values.len()).unwrap_or(u16::MAX);
        output.extend_from_slice(&count.to_le_bytes());
        for (key, value) in self.values.iter().take(usize::from(count)) {
            output.extend_from_slice(&(key.len() as u16).to_le_bytes());
            output.extend_from_slice(key.as_bytes());
            match value {
                WireValue::Text(text) => {
                    output.push(1);
                    output.extend_from_slice(&(text.len() as u32).to_le_bytes());
                    output.extend_from_slice(text.as_bytes());
                }
                WireValue::Integer(number) => {
                    output.push(2);
                    output.extend_from_slice(&number.to_le_bytes());
                }
                WireValue::Float(number) => {
                    output.push(3);
                    output.extend_from_slice(&number.to_le_bytes());
                }
                WireValue::Boolean(flag) => {
                    output.push(4);
                    output.push(u8::from(*flag));
                }
                WireValue::Bytes(bytes) => {
                    output.push(5);
                    output.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
                    output.extend_from_slice(bytes);
                }
            }
        }
        output
    }

    pub fn decode(payload: &[u8]) -> Result<Self, WireMapError> {
        if payload.len() > MAX_WIRE_MAP_BYTES {
            return Err(WireMapError::TooLarge);
        }
        let mut reader = Reader { bytes: payload, offset: 0 };
        let count = reader.u16()?;
        let mut values = BTreeMap::new();
        for _ in 0..count {
            let key_length = usize::from(reader.u16()?);
            let key = std::str::from_utf8(reader.take(key_length)?).map_err(|_| WireMapError::InvalidKey)?;
            if !valid_key(key) {
                return Err(WireMapError::InvalidKey);
            }
            let value = match reader.u8()? {
                1 => {
                    let length = reader.u32()? as usize;
                    let text = std::str::from_utf8(reader.take(length)?).map_err(|_| WireMapError::InvalidValue)?;
                    WireValue::Text(text.to_owned())
                }
                2 => WireValue::Integer(i64::from_le_bytes(reader.array()?)),
                3 => {
                    let number = f64::from_le_bytes(reader.array()?);
                    if !number.is_finite() {
                        return Err(WireMapError::InvalidValue);
                    }
                    WireValue::Float(number)
                }
                4 => match reader.u8()? {
                    0 => WireValue::Boolean(false),
                    1 => WireValue::Boolean(true),
                    _ => return Err(WireMapError::InvalidValue),
                },
                5 => {
                    let length = reader.u32()? as usize;
                    WireValue::Bytes(reader.take(length)?.to_vec())
                }
                _ => return Err(WireMapError::InvalidValue),
            };
            if values.insert(key.to_owned(), value).is_some() {
                return Err(WireMapError::DuplicateKey);
            }
        }
        if reader.offset != payload.len() {
            return Err(WireMapError::TrailingBytes);
        }
        Ok(Self { values })
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, length: usize) -> Result<&'a [u8], WireMapError> {
        let end = self.offset.checked_add(length).ok_or(WireMapError::Truncated)?;
        let slice = self.bytes.get(self.offset..end).ok_or(WireMapError::Truncated)?;
        self.offset = end;
        Ok(slice)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], WireMapError> {
        let mut output = [0_u8; N];
        output.copy_from_slice(self.take(N)?);
        Ok(output)
    }

    fn u8(&mut self) -> Result<u8, WireMapError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, WireMapError> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    fn u32(&mut self) -> Result<u32, WireMapError> {
        Ok(u32::from_le_bytes(self.array()?))
    }
}

/// Packed string list: `u32 count · (u32 length · UTF-8)*`.
#[must_use]
pub fn decode_string_list(payload: &[u8]) -> Option<Vec<String>> {
    let mut reader = Reader { bytes: payload, offset: 0 };
    let count = reader.u32().ok()? as usize;
    if count > 100_000 {
        return None;
    }
    let mut items = Vec::with_capacity(count.min(4096));
    for _ in 0..count {
        let length = reader.u32().ok()? as usize;
        items.push(std::str::from_utf8(reader.take(length).ok()?).ok()?.to_owned());
    }
    (reader.offset == payload.len()).then_some(items)
}

#[must_use]
pub fn encode_string_list(items: &[impl AsRef<str>]) -> Vec<u8> {
    let mut output = Vec::new();
    output.extend_from_slice(&(items.len() as u32).to_le_bytes());
    for item in items {
        let item = item.as_ref();
        output.extend_from_slice(&(item.len() as u32).to_le_bytes());
        output.extend_from_slice(item.as_bytes());
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_like_the_php_sdk() {
        // php -r 'echo bin2hex(Pam\Desktop\Internal\Wire::map(["b"=>true,"a"=>"x","n"=>7,"f"=>1.5]));'
        let map = WireMap::new().with("b", true).with("a", "x").with("n", 7_i64).with("f", 1.5_f64);
        let expected = concat!(
            "0400",
            "0100", "61", "01", "01000000", "78",
            "0100", "62", "04", "01",
            "0100", "66", "03", "000000000000f83f",
            "0100", "6e", "02", "0700000000000000",
        );
        let hex: String = map.encode().iter().map(|byte| format!("{byte:02x}")).collect();
        assert_eq!(hex, expected);
        assert_eq!(WireMap::decode(&map.encode()), Ok(map));
    }

    #[test]
    fn rejects_malformed_payloads() {
        assert_eq!(WireMap::decode(&[1, 0]), Err(WireMapError::Truncated));
        assert_eq!(WireMap::decode(&[0, 0, 9]), Err(WireMapError::TrailingBytes));
        let mut duplicate = vec![2, 0];
        for _ in 0..2 {
            duplicate.extend_from_slice(&[1, 0, b'a', 4, 1]);
        }
        assert_eq!(WireMap::decode(&duplicate), Err(WireMapError::DuplicateKey));
        assert_eq!(WireMap::decode(&[1, 0, 1, 0, b'1', 4, 1]), Err(WireMapError::InvalidKey));
    }

    #[test]
    fn string_lists_round_trip() {
        let items = ["um", "dois", "três"];
        assert_eq!(decode_string_list(&encode_string_list(&items)).as_deref(), Some(&items.map(str::to_owned)[..]));
        assert!(decode_string_list(&[1, 0, 0, 0]).is_none());
    }
}
