//! Modified UTF-7 codec for IMAP mailbox names (RFC 3501 Section 5.1.3).

const BASE64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+,";

/// Decodes a mailbox name from modified UTF-7. Invalid input is returned
/// unchanged (servers do send broken names; we must still show something).
pub fn decode(name: &str) -> String {
    match decode_strict(name) {
        Some(decoded) => decoded,
        None => name.to_string(),
    }
}

/// Encodes a mailbox name for use in IMAP commands (`CREATE`, `RENAME`, `SELECT`).
pub fn encode(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut non_ascii_u16 = Vec::new();

    for ch in name.chars() {
        if is_printable_ascii(ch) {
            flush_u16_block(&mut out, &mut non_ascii_u16);
            if ch == '&' {
                out.push_str("&-");
            } else {
                out.push(ch);
            }
        } else {
            let mut buf = [0u16; 2];
            let units = ch.encode_utf16(&mut buf);
            non_ascii_u16.extend_from_slice(units);
        }
    }

    flush_u16_block(&mut out, &mut non_ascii_u16);
    out
}

fn is_printable_ascii(ch: char) -> bool {
    ('\x20'..='\x7e').contains(&ch)
}

fn flush_u16_block(out: &mut String, units: &mut Vec<u16>) {
    if units.is_empty() {
        return;
    }

    let mut bytes = Vec::with_capacity(units.len() * 2);
    for &unit in units.iter() {
        bytes.extend_from_slice(&unit.to_be_bytes());
    }
    units.clear();

    out.push('&');
    encode_modified_base64(&bytes, out);
    out.push('-');
}

fn encode_modified_base64(bytes: &[u8], out: &mut String) {
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as usize;
        let b1 = chunk.get(1).copied().unwrap_or(0) as usize;
        let b2 = chunk.get(2).copied().unwrap_or(0) as usize;

        out.push(BASE64_ALPHABET[b0 >> 2] as char);
        out.push(BASE64_ALPHABET[((b0 & 0x03) << 4) | (b1 >> 4)] as char);

        if chunk.len() > 1 {
            out.push(BASE64_ALPHABET[((b1 & 0x0f) << 2) | (b2 >> 6)] as char);
        }
        if chunk.len() > 2 {
            out.push(BASE64_ALPHABET[b2 & 0x3f] as char);
        }
    }
}

fn decode_strict(name: &str) -> Option<String> {
    let mut out = String::with_capacity(name.len());
    let mut rest = name;

    while let Some(amp_idx) = rest.find('&') {
        out.push_str(&rest[..amp_idx]);
        let after_amp = &rest[amp_idx + 1..];
        let dash_idx = after_amp.find('-')?;
        let block = &after_amp[..dash_idx];

        if block.is_empty() {
            out.push('&');
        } else {
            let decoded_block = decode_block(block)?;
            out.push_str(&decoded_block);
        }

        rest = &after_amp[dash_idx + 1..];
    }

    out.push_str(rest);
    Some(out)
}

fn decode_block(block: &str) -> Option<String> {
    let bytes = decode_modified_base64(block)?;
    if bytes.len() % 2 != 0 {
        return None;
    }

    let (pairs, _) = bytes.as_chunks::<2>();
    let units = pairs.iter().map(|&[hi, lo]| u16::from_be_bytes([hi, lo]));

    let mut decoded = String::with_capacity(bytes.len() / 2);
    for result in char::decode_utf16(units) {
        let ch = result.ok()?;
        decoded.push(ch);
    }

    // RFC 3501 5.1.3 forbids base64-encoding printable US-ASCII characters.
    if decoded.chars().all(is_printable_ascii) {
        return None;
    }

    Some(decoded)
}

fn decode_modified_base64(input: &str) -> Option<Vec<u8>> {
    let raw = input.as_bytes();
    if raw.len() % 4 == 1 {
        return None;
    }

    let mut out = Vec::with_capacity((raw.len() * 3) / 4);
    for chunk in raw.chunks(4) {
        let mut vals = [0u8; 4];
        for (i, &b) in chunk.iter().enumerate() {
            vals[i] = decode_b64_byte(b)?;
        }

        out.push((vals[0] << 2) | (vals[1] >> 4));
        if chunk.len() > 2 {
            out.push((vals[1] << 4) | (vals[2] >> 2));
        } else if (vals[1] & 0x0f) != 0 {
            return None;
        }

        if chunk.len() > 3 {
            out.push((vals[2] << 6) | vals[3]);
        } else if chunk.len() == 3 && (vals[2] & 0x03) != 0 {
            return None;
        }
    }

    Some(out)
}

fn decode_b64_byte(b: u8) -> Option<u8> {
    match b {
        b'A'..=b'Z' => Some(b - b'A'),
        b'a'..=b'z' => Some(b - b'a' + 26),
        b'0'..=b'9' => Some(b - b'0' + 52),
        b'+' => Some(62),
        b',' => Some(63),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{decode, encode};

    const CASES: &[(&str, &str)] = &[
        ("INBOX", "INBOX"),
        ("Entw&APw-rfe", "Entwürfe"),
        ("Gel&APY-scht", "Gelöscht"),
        ("Tom &- Jerry", "Tom & Jerry"),
        ("&ZeVnLIqe-", "日本語"),
        ("&2D3eAA-", "😀"),
        ("Ordner/&AMQA1gDc-", "Ordner/ÄÖÜ"),
    ];

    #[test]
    fn decodes_known_mailbox_names() {
        for &(encoded, expected) in CASES {
            assert_eq!(decode(encoded), expected, "failed to decode {encoded}");
        }
    }

    #[test]
    fn encodes_known_mailbox_names() {
        for &(expected, plain) in CASES {
            assert_eq!(encode(plain), expected, "failed to encode {plain}");
        }
    }

    #[test]
    fn roundtrip_preserves_all_examples() {
        for &(_, plain) in CASES {
            assert_eq!(decode(&encode(plain)), plain);
        }

        let extra = [
            "",
            "&",
            "&&",
            "Archive/2026/Rechnungen & Belege",
            "Büro/€-Umsätze/😀🎉",
            "Sent Messages",
        ];
        for plain in extra {
            assert_eq!(decode(&encode(plain)), plain);
        }
    }

    #[test]
    fn invalid_inputs_are_returned_unchanged() {
        let invalid = [
            "&",
            "&AP",
            "&APw",
            "&AGE-",       // printable ASCII only
            "&2D0-",       // unpaired high surrogate
            "&3AA-",       // unpaired low surrogate
            "&AP/w-",      // standard base64 '/' instead of ','
            "&APw=-",      // padding '=' is invalid in modified UTF-7
            "&AA-",        // odd byte count (1 byte)
            "&A-",         // 1 base64 char cannot form a byte
            "Entw&APw",    // unclosed block after valid prefix
            "Ok&-But&Bad", // valid &- followed by unclosed block
        ];

        for input in invalid {
            assert_eq!(
                decode(input),
                input,
                "invalid input {input:?} should be returned unchanged"
            );
        }
    }
}
