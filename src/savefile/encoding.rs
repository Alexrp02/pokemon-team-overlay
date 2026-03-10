/// String terminator used in Gen IV save data.
const GEN4_TERMINATOR: u16 = 0xFFFF;

/// Maximum nickname length in Gen IV.
pub const NICKNAME_MAX_CHARS: usize = 11;

/// Decode a single Gen IV character code to a Unicode [`char`].
///
/// Character encoding reference: Bulbapedia – *Character encoding (Generation IV)*.
///
/// Contiguous runs in the encoding table:
/// - Digits  `0`–`9`:  `0x0121`–`0x012A`
/// - Upper   `A`–`Z`:  `0x012B`–`0x0144`
/// - Lower   `a`–`z`:  `0x0145`–`0x015E`
/// - Extended Latin:   `0x015F`–`0x019E`  (À–ÿ, Œœ, Şş, ªº, …)
///
/// Returns `None` for unmapped or terminator codes.
pub fn decode_char(code: u16) -> Option<char> {
    match code {
        0x0121..=0x012A => Some((b'0' + (code - 0x0121) as u8) as char),
        0x012B..=0x0144 => Some((b'A' + (code - 0x012B) as u8) as char),
        0x0145..=0x015E => Some((b'a' + (code - 0x0145) as u8) as char),
        // Extended Latin block: 0x015F = À (U+00C0), sequential through 0x019E
        0x015F..=0x019E => char::from_u32(0x00C0u32 + (code - 0x015F) as u32),
        0x01AB => Some('!'),
        0x01AC => Some('?'),
        0x01AD => Some(','),
        0x01AE => Some('.'),
        0x01AF => Some('…'),
        0x01B1 => Some('/'),
        0x01B2 => Some('\''),
        0x01B9 => Some('('),
        0x01BA => Some(')'),
        0x01BB => Some('♂'),
        0x01BC => Some('♀'),
        0x01BD => Some('+'),
        0x01BE => Some('-'),
        0x01C1 => Some('='),
        0x01C3 => Some('~'),
        0x01C4 => Some(':'),
        0x01C5 => Some(';'),
        0x01D0 => Some('@'),
        0x01D1 => Some('♪'),
        0x01D2 => Some('%'),
        0x01DE => Some(' '),
        _ => None,
    }
}

/// Decode a Gen IV encoded string from a byte slice.
///
/// Each character is a little-endian `u16`. Decoding stops at `0xFFFF` or `0x0000`.
/// Unmapped character codes are silently skipped.
pub fn decode_string(data: &[u8]) -> String {
    let mut result = String::new();
    let mut i = 0;
    while i + 1 < data.len() {
        let code = u16::from_le_bytes([data[i], data[i + 1]]);
        if code == GEN4_TERMINATOR || code == 0x0000 {
            break;
        }
        if let Some(c) = decode_char(code) {
            result.push(c);
        }
        // Unknown/unmapped characters are silently skipped.
        i += 2;
    }
    result
}
