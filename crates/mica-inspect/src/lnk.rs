//! Windows shortcuts (`.lnk`): where does a Start-menu entry point?
//!
//! Layout: Microsoft's [MS-SHLLINK] Shell Link Binary File Format. MICA reads
//! the header, skips the optional item-ID list, and takes the target's local
//! path from the LinkInfo structure (`LocalBasePath`, plus
//! `CommonPathSuffix`). Arguments and working directory live in the string
//! data that follows; MICA reads the arguments too.

use crate::bytes::{c_string, u16_le, u32_le};

const HAS_LINK_TARGET_ID_LIST: u32 = 0x01;
const HAS_LINK_INFO: u32 = 0x02;
const HAS_NAME: u32 = 0x04;
const HAS_RELATIVE_PATH: u32 = 0x08;
const HAS_WORKING_DIR: u32 = 0x10;
const HAS_ARGUMENTS: u32 = 0x20;
const IS_UNICODE: u32 = 0x80;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shortcut {
    /// The target as a Windows path, e.g. `C:\Program Files\App\app.exe`.
    pub target: String,
    pub arguments: Option<String>,
}

/// Read a shortcut's target, if it has a local path.
pub fn parse(data: &[u8]) -> Option<Shortcut> {
    if u32_le(data, 0)? != 0x4C {
        return None;
    }
    let flags = u32_le(data, 0x14)?;
    let mut at = 0x4C;
    if flags & HAS_LINK_TARGET_ID_LIST != 0 {
        at += 2 + u16_le(data, at)? as usize;
    }
    if flags & HAS_LINK_INFO == 0 {
        return None;
    }
    let info = at;
    let info_size = u32_le(data, info)? as usize;
    let local_base = u32_le(data, info + 16)? as usize;
    let suffix = u32_le(data, info + 24)? as usize;
    let mut target = c_string(data, info.checked_add(local_base)?, 1024)?;
    if let Some(rest) = c_string(data, info + suffix, 1024).filter(|rest| !rest.is_empty()) {
        if !target.ends_with('\\') {
            target.push('\\');
        }
        target.push_str(&rest);
    }
    at = info.checked_add(info_size)?;

    // StringData: counted strings, UTF-16 when IS_UNICODE.
    let unicode = flags & IS_UNICODE != 0;
    let read_string = |at: &mut usize| -> Option<String> {
        let count = u16_le(data, *at)? as usize;
        *at += 2;
        let bytes = if unicode { count * 2 } else { count };
        let raw = data.get(*at..*at + bytes)?;
        *at += bytes;
        Some(if unicode {
            let units: Vec<u16> = raw
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect();
            String::from_utf16_lossy(&units)
        } else {
            String::from_utf8_lossy(raw).into_owned()
        })
    };
    let mut arguments = None;
    for (flag, is_arguments) in [
        (HAS_NAME, false),
        (HAS_RELATIVE_PATH, false),
        (HAS_WORKING_DIR, false),
        (HAS_ARGUMENTS, true),
    ] {
        if flags & flag != 0 {
            let value = read_string(&mut at);
            if is_arguments {
                arguments = value.filter(|text| !text.is_empty());
            }
        }
    }
    Some(Shortcut { target, arguments })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A shortcut with LinkInfo and Unicode arguments.
    pub(crate) fn build(target: &str, arguments: &str) -> Vec<u8> {
        let mut data = vec![0u8; 0x4C];
        data[0..4].copy_from_slice(&0x4Cu32.to_le_bytes());
        let flags = HAS_LINK_INFO | HAS_ARGUMENTS | IS_UNICODE;
        data[0x14..0x18].copy_from_slice(&flags.to_le_bytes());
        let header = 28u32;
        let base_offset = header;
        let suffix_offset = header + target.len() as u32 + 1;
        let info_size = suffix_offset + 1;
        data.extend_from_slice(&info_size.to_le_bytes());
        data.extend_from_slice(&header.to_le_bytes());
        data.extend_from_slice(&1u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&base_offset.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&suffix_offset.to_le_bytes());
        data.extend_from_slice(target.as_bytes());
        data.push(0);
        data.push(0);
        let units: Vec<u16> = arguments.encode_utf16().collect();
        data.extend_from_slice(&(units.len() as u16).to_le_bytes());
        for unit in units {
            data.extend_from_slice(&unit.to_le_bytes());
        }
        data
    }

    #[test]
    fn reads_target_and_arguments() {
        let shortcut = parse(&build(
            r"C:\Program Files\Notepad++\notepad++.exe",
            "-multiInst",
        ))
        .unwrap();
        assert_eq!(shortcut.target, r"C:\Program Files\Notepad++\notepad++.exe");
        assert_eq!(shortcut.arguments.as_deref(), Some("-multiInst"));
        let data = build(r"C:\a.exe", "");
        for cut in 0..data.len() {
            let _ = parse(&data[..cut]);
        }
        assert!(parse(b"not a shortcut at all, definitely").is_none());
    }
}
