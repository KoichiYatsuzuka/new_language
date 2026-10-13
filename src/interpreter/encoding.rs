// encoding.rs — `str.encode(encoding)` / `bytes.decode(encoding)` の文字コード（ffi_value_exchange_plan.md 3-1・D-9・D-13）。
//
// 名前は Python と同じ綴りを受け付ける（大文字小文字と `-` / `_` の違いは無視）:
//   - `utf-8`（`utf8`）・`utf-16-le`（`utf-16le`）・`utf-16-be`・`ascii`・`latin-1`（`latin1` / `iso-8859-1`）は自前で、
//     Python と同じ意味（⚠ WHATWG の `latin1` は windows-1252 なので encoding_rs には渡さない）
//   - `shift_jis` / `cp932` / `sjis` / `windows-31j` は encoding_rs の Shift_JIS（Windows の CP932 と同じ表）
//   - それ以外は WHATWG Encoding Standard の名前（`euc-jp` / `gbk` / `euc-kr` / `windows-1252` …）
// ⚠ 変換できないとき（不正なバイト列・その文字コードに無い文字）は ValueError。置き換え文字（U+FFFD）や `&#…;` に
//   黙って置き換えない（`.claude/rules/regulations.md`）。知らない名前も ValueError（Python は LookupError）。

/// 文字コードの種類。
enum Codec {
    Utf8,
    Utf16Le,
    Utf16Be,
    Ascii,
    Latin1,
    Whatwg(&'static encoding_rs::Encoding),
}

fn codec_of(name: &str) -> Result<Codec, String> {
    let norm = name.trim().to_ascii_lowercase().replace('_', "-");
    Ok(match norm.as_str() {
        "utf-8" | "utf8" | "u8" => Codec::Utf8,
        "utf-16-le" | "utf-16le" | "utf16le" => Codec::Utf16Le,
        "utf-16-be" | "utf-16be" | "utf16be" => Codec::Utf16Be,
        "utf-16" | "utf16" => {
            return Err(format!(
                "ValueError: encoding '{name}' is ambiguous: write 'utf-16-le' or 'utf-16-be' (no byte order mark is added or read)"
            ))
        }
        "ascii" | "us-ascii" => Codec::Ascii,
        "latin-1" | "latin1" | "iso-8859-1" | "iso8859-1" | "l1" => Codec::Latin1,
        "shift-jis" | "sjis" | "cp932" | "ms932" | "windows-31j" | "ms-kanji" => Codec::Whatwg(encoding_rs::SHIFT_JIS),
        other => match encoding_rs::Encoding::for_label(other.as_bytes()) {
            // UTF-16 は上で自前で扱う（encoding_rs は UTF-16 へ符号化しない）
            Some(e) if e != encoding_rs::UTF_16LE && e != encoding_rs::UTF_16BE && e != encoding_rs::REPLACEMENT => {
                Codec::Whatwg(e)
            }
            _ => return Err(format!("ValueError: unknown encoding '{name}'")),
        },
    })
}

/// `s.encode(encoding)`。
pub(crate) fn encode(s: &str, encoding: &str) -> Result<Vec<u8>, String> {
    let unencodable = |c: char, i: usize| {
        format!("ValueError: '{encoding}' cannot encode character {c:?} (U+{:04X}) at position {i}", c as u32)
    };
    match codec_of(encoding)? {
        Codec::Utf8 => Ok(s.as_bytes().to_vec()),
        Codec::Utf16Le => Ok(s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()),
        Codec::Utf16Be => Ok(s.encode_utf16().flat_map(|u| u.to_be_bytes()).collect()),
        Codec::Ascii => s
            .chars()
            .enumerate()
            .map(|(i, c)| if c.is_ascii() { Ok(c as u8) } else { Err(unencodable(c, i)) })
            .collect(),
        Codec::Latin1 => s
            .chars()
            .enumerate()
            .map(|(i, c)| u8::try_from(c as u32).map_err(|_| unencodable(c, i)))
            .collect(),
        Codec::Whatwg(enc) => {
            let (out, _, had_unmappable) = enc.encode(s);
            if had_unmappable {
                // どの文字かを示す（失敗したときだけ 1 文字ずつ確かめる）
                let mut buf = [0u8; 4];
                for (i, c) in s.chars().enumerate() {
                    if enc.encode(c.encode_utf8(&mut buf)).2 {
                        return Err(unencodable(c, i));
                    }
                }
                return Err(format!("ValueError: '{encoding}' cannot encode the string"));
            }
            Ok(out.into_owned())
        }
    }
}

/// `b.decode(encoding)`。
pub(crate) fn decode(b: &[u8], encoding: &str) -> Result<String, String> {
    let invalid = |at: usize| format!("ValueError: cannot decode bytes as '{encoding}': invalid data at position {at}");
    match codec_of(encoding)? {
        Codec::Utf8 => std::str::from_utf8(b).map(str::to_string).map_err(|e| invalid(e.valid_up_to())),
        Codec::Utf16Le | Codec::Utf16Be => {
            if b.len() % 2 != 0 {
                return Err(format!(
                    "ValueError: cannot decode bytes as '{encoding}': odd length ({} bytes)",
                    b.len()
                ));
            }
            let le = matches!(codec_of(encoding)?, Codec::Utf16Le);
            let units = b.chunks_exact(2).map(|p| if le { u16::from_le_bytes([p[0], p[1]]) } else { u16::from_be_bytes([p[0], p[1]]) });
            let mut out = String::with_capacity(b.len() / 2);
            for (i, r) in char::decode_utf16(units).enumerate() {
                out.push(r.map_err(|_| invalid(i * 2))?);
            }
            Ok(out)
        }
        Codec::Ascii => b
            .iter()
            .enumerate()
            .map(|(i, &c)| if c.is_ascii() { Ok(c as char) } else { Err(invalid(i)) })
            .collect(),
        Codec::Latin1 => Ok(b.iter().map(|&c| c as char).collect()),
        Codec::Whatwg(enc) => enc
            .decode_without_bom_handling_and_without_replacement(b)
            .map(|s| s.into_owned())
            .ok_or_else(|| format!("ValueError: cannot decode bytes as '{encoding}': invalid byte sequence")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_round_trip_and_errors() {
        assert_eq!(encode("日本", "utf-8").unwrap(), "日本".as_bytes());
        assert_eq!(decode("日本".as_bytes(), "UTF_8").unwrap(), "日本");
        let e = decode(&[0x61, 0xff], "utf-8").unwrap_err();
        assert!(e.starts_with("ValueError: cannot decode bytes as 'utf-8': invalid data at position 1"), "{e}");
    }

    #[test]
    fn utf16_matches_wchar_t() {
        assert_eq!(encode("aé", "utf-16-le").unwrap(), vec![0x61, 0x00, 0xe9, 0x00]);
        assert_eq!(decode(&[0x61, 0x00, 0xe9, 0x00], "utf-16le").unwrap(), "aé");
        assert_eq!(encode("a", "utf-16-be").unwrap(), vec![0x00, 0x61]);
        assert!(decode(&[0x61], "utf-16-le").is_err());
        assert!(decode(&[0x00, 0xd8], "utf-16-le").is_err()); // lone surrogate
        assert!(encode("a", "utf-16").is_err());
    }

    #[test]
    fn ascii_and_latin1_follow_python() {
        assert!(encode("é", "ascii").unwrap_err().contains("U+00E9"));
        assert_eq!(encode("é", "latin-1").unwrap(), vec![0xe9]);
        // Python の latin-1 は 0x80 を U+0080 にする（WHATWG の latin1 = windows-1252 なら '€'）
        assert_eq!(decode(&[0x80], "latin-1").unwrap(), "\u{80}");
        assert!(encode("€", "latin1").is_err());
    }

    #[test]
    fn shift_jis_is_strict() {
        let sjis = encode("日本語", "shift_jis").unwrap();
        assert_eq!(sjis, vec![0x93, 0xfa, 0x96, 0x7b, 0x8c, 0xea]);
        assert_eq!(decode(&sjis, "cp932").unwrap(), "日本語");
        // Shift_JIS に無い文字は `&#…;` にせず誤り
        assert!(encode("😀", "shift_jis").unwrap_err().contains("U+1F600"));
        assert!(decode(&[0x82], "shift_jis").is_err());
    }

    #[test]
    fn unknown_encoding_is_an_error() {
        assert!(encode("a", "no-such-encoding").unwrap_err().starts_with("ValueError: unknown encoding"));
    }
}
