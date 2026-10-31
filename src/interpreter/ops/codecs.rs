// ops/codecs.rs — `str.encode` / `bytes.decode` / `bytes(s, encoding)` の符号化（**唯一の実装**・
// python_builtins_plan.md のタスク 3-1・ffi_value_exchange_plan.md 3-1・D-9・D-13）。
//
// 名前は CPython と同じ綴りを受け付ける（大文字小文字と `-` / ` ` / `_` を区別しない・別名も受ける）:
//   - `utf-8` / `ascii` / `latin-1` は自前で、誤りの文言・位置（文字の番号・バイトの番号）まで CPython 3.12 と同じ
//     （⚠ WHATWG の `latin1` / `ascii` は windows-1252 なので encoding_rs には渡さない）
//   - `utf-16-le` / `utf-16-be` は自前（BOM は付けも読みもしない・Windows の `wchar_t*` と同じ並び）。
//     `utf-16` はバイト順が決まらないので誤り
//   - `shift_jis` / `cp932` / `sjis` / `windows-31j` は encoding_rs の Shift_JIS（Windows の CP932 と同じ表）
//   - それ以外は WHATWG Encoding Standard の名前（`euc-jp` / `gbk` / `euc-kr` / `windows-1252` …）
// 誤りの扱い（`errors=`）は `strict` / `ignore` / `replace`（どの符号化でも）。`strict` の誤りは
// `UnicodeEncodeError` / `UnicodeDecodeError`（どちらも `ValueError` の派生）。
// ⚠ U+FFFD・`?` に置き換えるのは `errors="replace"` と書いたときだけ。`&#…;`（encoding_rs の既定）にはしない
//   （黙って置き換えない・`.claude/rules/regulations.md`）。
// ⚠ 知らない符号化は `LookupError: unknown encoding: ..`、知らない誤りの扱いは誤りが起きたときだけ
//   `LookupError: unknown error handler name '..'`（CPython も誤りが起きるまで名前を見ない）。
// ⚠ 以前は `interpreter/encoding.rs` にも別の実装があり（FFI のための文字コード）、`utf-8` などの
//   誤りの種類（`ValueError`）と文言がこちらと食い違っていた。

/// 対応する符号化。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Codec {
    Utf8,
    Ascii,
    Latin1,
    /// UTF-16（バイト順は名前で決める・BOM は付けも読みもしない）。
    Utf16 { little_endian: bool },
    /// encoding_rs の符号化（Shift_JIS と WHATWG の名前）。
    Whatwg(&'static encoding_rs::Encoding),
}

impl Codec {
    /// 符号化の名前を引く（CPython と同じく大文字小文字と `-` / ` ` / `_` を区別しない・別名も受ける）。
    pub(crate) fn lookup(name: &str) -> Result<Codec, String> {
        let n: String = name
            .trim()
            .to_ascii_lowercase()
            .chars()
            .map(|c| if c == '-' || c == ' ' { '_' } else { c })
            .collect();
        Ok(match n.as_str() {
            "utf_8" | "utf8" | "u8" | "utf" | "cp65001" => Codec::Utf8,
            "ascii" | "us_ascii" | "646" | "us" => Codec::Ascii,
            "latin_1" | "latin1" | "latin" | "l1" | "iso_8859_1" | "iso8859_1" | "8859" | "cp819" => Codec::Latin1,
            "utf_16_le" | "utf_16le" | "utf16le" => Codec::Utf16 { little_endian: true },
            "utf_16_be" | "utf_16be" | "utf16be" => Codec::Utf16 { little_endian: false },
            "utf_16" | "utf16" => {
                return Err(format!(
                    "ValueError: encoding '{name}' is ambiguous: write 'utf-16-le' or 'utf-16-be' (no byte order mark is added or read)"
                ))
            }
            "shift_jis" | "shiftjis" | "sjis" | "s_jis" | "cp932" | "ms932" | "mskanji" | "ms_kanji" | "windows_31j" => {
                Codec::Whatwg(encoding_rs::SHIFT_JIS)
            }
            other => match encoding_rs::Encoding::for_label(other.replace('_', "-").as_bytes()) {
                Some(e) if e == encoding_rs::UTF_8 => Codec::Utf8,
                // UTF-16 は上で自前で扱う（encoding_rs は UTF-16 へ符号化しない）
                Some(e) if e != encoding_rs::UTF_16LE && e != encoding_rs::UTF_16BE && e != encoding_rs::REPLACEMENT => {
                    Codec::Whatwg(e)
                }
                _ => return Err(format!("LookupError: unknown encoding: {name}")),
            },
        })
    }

    /// 誤りの文言に出す名前（CPython の正式名）。
    fn name(self) -> String {
        match self {
            Codec::Utf8 => "utf-8".to_string(),
            Codec::Ascii => "ascii".to_string(),
            Codec::Latin1 => "latin-1".to_string(),
            Codec::Utf16 { little_endian: true } => "utf-16-le".to_string(),
            Codec::Utf16 { little_endian: false } => "utf-16-be".to_string(),
            Codec::Whatwg(e) => e.name().to_ascii_lowercase(),
        }
    }
}

/// 誤りの扱い（`errors=`）。
enum Handler {
    Strict,
    Ignore,
    Replace,
}

/// 誤りの扱いの名前を引く。⚠ 呼ぶのは誤りが起きたときだけ（上の doc）。
fn handler(errors: &str) -> Result<Handler, String> {
    match errors {
        "strict" => Ok(Handler::Strict),
        "ignore" => Ok(Handler::Ignore),
        "replace" => Ok(Handler::Replace),
        other => Err(format!("LookupError: unknown error handler name '{other}'")),
    }
}

/// 誤りの文言の中の 1 文字（CPython は表示できる文字でも `\x..` / `\u....` / `\U........` で出す）。
fn escaped_char(c: char) -> String {
    let cp = c as u32;
    if cp <= 0xFF {
        format!("\\x{cp:02x}")
    } else if cp <= 0xFFFF {
        format!("\\u{cp:04x}")
    } else {
        format!("\\U{cp:08x}")
    }
}

/// `s.encode(encoding, errors)`。
pub(crate) fn encode(s: &str, codec: Codec, errors: &str) -> Result<Vec<u8>, String> {
    let limit: u32 = match codec {
        // UTF-8 / UTF-16 で符号化できない文字は Rust の文字列に無い（サロゲートを持てない）。
        Codec::Utf8 => return Ok(s.as_bytes().to_vec()),
        Codec::Utf16 { little_endian } => {
            return Ok(s
                .encode_utf16()
                .flat_map(|u| if little_endian { u.to_le_bytes() } else { u.to_be_bytes() })
                .collect())
        }
        Codec::Whatwg(enc) => return encode_whatwg(s, enc, errors),
        Codec::Ascii => 128,
        Codec::Latin1 => 256,
    };
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        let cp = chars[i] as u32;
        if cp < limit {
            out.push(cp as u8);
            i += 1;
            continue;
        }
        // 符号化できない文字の連なりを 1 つの誤りにまとめる（CPython の strict と同じ範囲）。
        let start = i;
        while i < chars.len() && chars[i] as u32 >= limit {
            i += 1;
        }
        match handler(errors)? {
            Handler::Strict => {
                let what = if i - start == 1 {
                    format!("character '{}' in position {start}", escaped_char(chars[start]))
                } else {
                    format!("characters in position {start}-{}", i - 1)
                };
                return Err(format!(
                    "UnicodeEncodeError: '{}' codec can't encode {what}: ordinal not in range({limit})",
                    codec.name()
                ));
            }
            Handler::Ignore => {}
            Handler::Replace => out.extend(std::iter::repeat(b'?').take(i - start)),
        }
    }
    Ok(out)
}

/// `b.decode(encoding, errors)`。
pub(crate) fn decode(b: &[u8], codec: Codec, errors: &str) -> Result<String, String> {
    match codec {
        Codec::Latin1 => Ok(b.iter().map(|&x| x as char).collect()),
        Codec::Ascii => {
            let mut out = String::with_capacity(b.len());
            for (pos, &x) in b.iter().enumerate() {
                if x < 0x80 {
                    out.push(x as char);
                    continue;
                }
                match handler(errors)? {
                    Handler::Strict => {
                        return Err(format!(
                            "UnicodeDecodeError: 'ascii' codec can't decode byte 0x{x:02x} in position {pos}: \
                             ordinal not in range(128)"
                        ))
                    }
                    Handler::Ignore => {}
                    Handler::Replace => out.push('\u{FFFD}'),
                }
            }
            Ok(out)
        }
        Codec::Utf8 => decode_utf8(b, errors),
        Codec::Utf16 { little_endian } => decode_utf16(b, little_endian, errors),
        Codec::Whatwg(enc) => decode_whatwg(b, enc, errors),
    }
}

/// encoding_rs の符号化。⚠ encoding_rs は表に無い文字を `&#…;` にする（WHATWG の規則）ので、そうなったときは
/// 1 文字ずつ確かめて CPython と同じ扱いにする（`strict` は誤り・`ignore` は落とす・`replace` は `?`）。
fn encode_whatwg(s: &str, enc: &'static encoding_rs::Encoding, errors: &str) -> Result<Vec<u8>, String> {
    let (whole, _, unmappable) = enc.encode(s);
    if !unmappable {
        return Ok(whole.into_owned());
    }
    let maps = |c: char| -> Option<Vec<u8>> {
        let mut buf = [0u8; 4];
        let (bytes, _, bad) = enc.encode(c.encode_utf8(&mut buf));
        (!bad).then(|| bytes.into_owned())
    };
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        if let Some(bytes) = maps(chars[i]) {
            out.extend_from_slice(&bytes);
            i += 1;
            continue;
        }
        // 符号化できない文字の連なりを 1 つの誤りにまとめる（`encode` と同じ）。
        let start = i;
        while i < chars.len() && maps(chars[i]).is_none() {
            i += 1;
        }
        match handler(errors)? {
            Handler::Strict => {
                let what = if i - start == 1 {
                    format!("character '{}' in position {start}", escaped_char(chars[start]))
                } else {
                    format!("characters in position {start}-{}", i - 1)
                };
                return Err(format!(
                    "UnicodeEncodeError: '{}' codec can't encode {what}: illegal multibyte sequence",
                    Codec::Whatwg(enc).name()
                ));
            }
            Handler::Ignore => {}
            Handler::Replace => out.extend(std::iter::repeat(b'?').take(i - start)),
        }
    }
    Ok(out)
}

/// UTF-16 の復号。対になっていないサロゲートと、奇数長の末尾の 1 バイトが誤り（CPython と同じ文言）。
fn decode_utf16(b: &[u8], little_endian: bool, errors: &str) -> Result<String, String> {
    let name = Codec::Utf16 { little_endian }.name();
    let units = b.chunks_exact(2).map(|p| {
        if little_endian { u16::from_le_bytes([p[0], p[1]]) } else { u16::from_be_bytes([p[0], p[1]]) }
    });
    let mut out = String::with_capacity(b.len() / 2);
    let mut unit = 0usize; // 読んだ 16 ビット単位の数
    for r in char::decode_utf16(units) {
        match r {
            Ok(c) => {
                out.push(c);
                unit += c.len_utf16();
            }
            Err(_) => {
                let pos = unit * 2;
                unit += 1;
                match handler(errors)? {
                    Handler::Strict => {
                        return Err(format!(
                            "UnicodeDecodeError: '{name}' codec can't decode bytes in position {pos}-{}: illegal UTF-16 surrogate",
                            pos + 1
                        ))
                    }
                    Handler::Ignore => {}
                    Handler::Replace => out.push('\u{FFFD}'),
                }
            }
        }
    }
    if b.len() % 2 != 0 {
        let pos = b.len() - 1;
        match handler(errors)? {
            Handler::Strict => {
                return Err(format!(
                    "UnicodeDecodeError: '{name}' codec can't decode byte 0x{:02x} in position {pos}: truncated data",
                    b[pos]
                ))
            }
            Handler::Ignore => {}
            Handler::Replace => out.push('\u{FFFD}'),
        }
    }
    Ok(out)
}

/// encoding_rs の符号化の復号。不正な並びの位置は encoding_rs の復号器が返す範囲（`DecoderResult::Malformed`）。
fn decode_whatwg(b: &[u8], enc: &'static encoding_rs::Encoding, errors: &str) -> Result<String, String> {
    use encoding_rs::DecoderResult;
    let mut decoder = enc.new_decoder_without_bom_handling();
    let mut out = String::new();
    let mut pos = 0;
    loop {
        let rest = b.len() - pos;
        out.reserve(decoder.max_utf8_buffer_length_without_replacement(rest).unwrap_or(4 * rest + 16));
        let (result, read) = decoder.decode_to_string_without_replacement(&b[pos..], &mut out, true);
        pos += read;
        match result {
            DecoderResult::InputEmpty => return Ok(out),
            DecoderResult::OutputFull => continue,
            DecoderResult::Malformed(bad, after) => {
                let end = pos - after as usize;
                let start = end - bad as usize;
                match handler(errors)? {
                    Handler::Strict => {
                        let what = if end <= start + 1 {
                            format!("byte 0x{:02x} in position {start}", b.get(start).copied().unwrap_or(0))
                        } else {
                            format!("bytes in position {start}-{}", end - 1)
                        };
                        return Err(format!(
                            "UnicodeDecodeError: '{}' codec can't decode {what}: illegal multibyte sequence",
                            Codec::Whatwg(enc).name()
                        ));
                    }
                    Handler::Ignore => {}
                    Handler::Replace => out.push('\u{FFFD}'),
                }
            }
        }
    }
}

/// UTF-8 の復号。誤りの範囲は Rust の `Utf8Error`（`valid_up_to` / `error_len`）が CPython と同じ切り方をする
/// （不正な並びの最長の正しい接頭辞を 1 つの誤りにする。`replace` もその単位で U+FFFD 1 つ）。
fn decode_utf8(b: &[u8], errors: &str) -> Result<String, String> {
    let mut out = String::with_capacity(b.len());
    let mut pos = 0;
    while pos < b.len() {
        match std::str::from_utf8(&b[pos..]) {
            Ok(rest) => {
                out.push_str(rest);
                break;
            }
            Err(e) => {
                let good = e.valid_up_to();
                out.push_str(std::str::from_utf8(&b[pos..pos + good]).expect("valid prefix"));
                let start = pos + good;
                let (end, reason) = match e.error_len() {
                    // 並びの途中で尽きた。
                    None => (b.len(), "unexpected end of data"),
                    Some(n) => {
                        let lead = b[start];
                        // 先頭に来られないバイト（継続のバイト・C0 / C1・F5 以上）。
                        let bad_start = n == 1 && (matches!(lead, 0x80..=0xC1) || lead >= 0xF5);
                        (start + n, if bad_start { "invalid start byte" } else { "invalid continuation byte" })
                    }
                };
                match handler(errors)? {
                    Handler::Strict => {
                        let what = if end - start == 1 {
                            format!("byte 0x{:02x} in position {start}", b[start])
                        } else {
                            format!("bytes in position {start}-{}", end - 1)
                        };
                        return Err(format!("UnicodeDecodeError: 'utf-8' codec can't decode {what}: {reason}"));
                    }
                    Handler::Ignore => {}
                    Handler::Replace => out.push('\u{FFFD}'),
                }
                pos = end;
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enc(s: &str, name: &str) -> Result<Vec<u8>, String> {
        encode(s, Codec::lookup(name)?, "strict")
    }
    fn dec(b: &[u8], name: &str) -> Result<String, String> {
        decode(b, Codec::lookup(name)?, "strict")
    }

    #[test]
    fn utf8_round_trip_and_errors() {
        assert_eq!(enc("日本", "utf-8").unwrap(), "日本".as_bytes());
        assert_eq!(dec("日本".as_bytes(), "UTF_8").unwrap(), "日本");
        assert_eq!(
            dec(&[0x61, 0xff], "utf-8").unwrap_err(),
            "UnicodeDecodeError: 'utf-8' codec can't decode byte 0xff in position 1: invalid start byte"
        );
    }

    /// UTF-16 は C の `wchar_t*`（Windows）と同じ並び。BOM は付けも読みもしない（ffi_value_exchange_plan.md D-13）。
    #[test]
    fn utf16_matches_wchar_t() {
        assert_eq!(enc("aé", "utf-16-le").unwrap(), vec![0x61, 0x00, 0xe9, 0x00]);
        assert_eq!(dec(&[0x61, 0x00, 0xe9, 0x00], "utf-16le").unwrap(), "aé");
        assert_eq!(enc("a", "utf-16-be").unwrap(), vec![0x00, 0x61]);
        assert_eq!(
            dec(&[0x61], "utf-16-le").unwrap_err(),
            "UnicodeDecodeError: 'utf-16-le' codec can't decode byte 0x61 in position 0: truncated data"
        );
        assert!(dec(&[0x00, 0xd8], "utf-16-le").unwrap_err().contains("illegal UTF-16 surrogate")); // lone surrogate
        assert!(enc("a", "utf-16").unwrap_err().contains("ambiguous"));
        assert_eq!(decode(&[0x61, 0x00, 0x62], Codec::lookup("utf-16-le").unwrap(), "replace").unwrap(), "a\u{FFFD}");
    }

    #[test]
    fn ascii_and_latin1_follow_python() {
        assert!(enc("é", "ascii").unwrap_err().starts_with("UnicodeEncodeError: 'ascii' codec can't encode character '\\xe9'"));
        assert_eq!(enc("é", "latin-1").unwrap(), vec![0xe9]);
        // Python の latin-1 は 0x80 を U+0080 にする（WHATWG の latin1 = windows-1252 なら '€'）
        assert_eq!(dec(&[0x80], "latin-1").unwrap(), "\u{80}");
        assert!(enc("€", "latin1").is_err());
    }

    /// Shift_JIS に無い文字・不正な並びは `&#…;` や U+FFFD にせず誤り（`errors=` を書いたときだけ落とす・置き換える）。
    #[test]
    fn shift_jis_is_strict_unless_told() {
        let sjis = enc("日本語", "shift_jis").unwrap();
        assert_eq!(sjis, vec![0x93, 0xfa, 0x96, 0x7b, 0x8c, 0xea]);
        assert_eq!(dec(&sjis, "cp932").unwrap(), "日本語");
        assert_eq!(
            enc("a😀", "shift_jis").unwrap_err(),
            "UnicodeEncodeError: 'shift_jis' codec can't encode character '\\U0001f600' in position 1: illegal multibyte sequence"
        );
        let sj = Codec::lookup("sjis").unwrap();
        assert_eq!(encode("a😀b", sj, "replace").unwrap(), b"a?b");
        assert_eq!(encode("a😀b", sj, "ignore").unwrap(), b"ab");
        assert!(dec(&[0x82], "shift_jis").unwrap_err().starts_with("UnicodeDecodeError: 'shift_jis' codec can't decode"));
        assert_eq!(decode(&[0x61, 0x82], sj, "replace").unwrap(), "a\u{FFFD}");
    }

    #[test]
    fn unknown_encoding_is_a_lookup_error() {
        assert_eq!(enc("a", "no-such-encoding").unwrap_err(), "LookupError: unknown encoding: no-such-encoding");
    }
}
