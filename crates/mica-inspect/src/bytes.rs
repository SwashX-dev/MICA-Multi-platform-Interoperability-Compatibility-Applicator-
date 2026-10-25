//! Bounds-checked reads from untrusted bytes.
//!
//! Every parser in this crate reads through these functions, so a truncated or
//! hostile file yields `None` (and a less detailed report), never a panic or
//! an out-of-bounds read.

pub(crate) fn slice(data: &[u8], offset: usize, len: usize) -> Option<&[u8]> {
    data.get(offset..offset.checked_add(len)?)
}

pub(crate) fn u16_le(data: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(slice(data, offset, 2)?.try_into().ok()?))
}

pub(crate) fn u32_le(data: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(slice(data, offset, 4)?.try_into().ok()?))
}

pub(crate) fn u64_le(data: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes(slice(data, offset, 8)?.try_into().ok()?))
}

pub(crate) fn u16_be(data: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_be_bytes(slice(data, offset, 2)?.try_into().ok()?))
}

pub(crate) fn u32_be(data: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_be_bytes(slice(data, offset, 4)?.try_into().ok()?))
}

pub(crate) fn u64_be(data: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_be_bytes(slice(data, offset, 8)?.try_into().ok()?))
}

/// Read with the byte order a format declared.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Endian {
    Little,
    Big,
}

impl Endian {
    pub(crate) fn u16(self, data: &[u8], offset: usize) -> Option<u16> {
        match self {
            Endian::Little => u16_le(data, offset),
            Endian::Big => u16_be(data, offset),
        }
    }

    pub(crate) fn u32(self, data: &[u8], offset: usize) -> Option<u32> {
        match self {
            Endian::Little => u32_le(data, offset),
            Endian::Big => u32_be(data, offset),
        }
    }

    pub(crate) fn u64(self, data: &[u8], offset: usize) -> Option<u64> {
        match self {
            Endian::Little => u64_le(data, offset),
            Endian::Big => u64_be(data, offset),
        }
    }
}

/// A NUL-terminated string starting at `offset`, at most `max` bytes long,
/// lossily decoded. `None` if it runs off the end or past `max`.
pub(crate) fn c_string(data: &[u8], offset: usize, max: usize) -> Option<String> {
    let rest = data.get(offset..)?;
    let window = &rest[..rest.len().min(max)];
    let end = window.iter().position(|&byte| byte == 0)?;
    Some(String::from_utf8_lossy(&window[..end]).into_owned())
}

/// Whether `needle` occurs anywhere in `haystack`.
pub(crate) fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

/// Like [`contains`], ignoring ASCII case.
pub(crate) fn contains_ignore_case(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window.eq_ignore_ascii_case(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_stop_at_the_end() {
        let data = [1, 2, 3, 4];
        assert_eq!(u16_le(&data, 2), Some(0x0403));
        assert_eq!(u16_le(&data, 3), None);
        assert_eq!(u32_be(&data, 0), Some(0x01020304));
        assert_eq!(u32_le(&data, 1), None);
        assert_eq!(u64_le(&data, 0), None);
        assert_eq!(slice(&data, usize::MAX, 2), None);
    }

    #[test]
    fn c_strings_need_a_terminator() {
        let data = b"abc\0def";
        assert_eq!(c_string(data, 0, 16).as_deref(), Some("abc"));
        assert_eq!(c_string(data, 4, 16), None);
        assert_eq!(c_string(data, 0, 2), None);
        assert_eq!(c_string(data, 99, 2), None);
    }

    #[test]
    fn searches() {
        assert!(contains(b"hello world", b"o w"));
        assert!(!contains(b"hello", b""));
        assert!(contains_ignore_case(b"Nullsoft Install", b"NULLSOFT"));
    }
}
