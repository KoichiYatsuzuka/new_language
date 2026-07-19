// str_methods.rs — 文字列メソッド補助関数
//
// Python 互換の str.format() と % 書式演算子の実装、および
// 正規表現操作（regex クレート使用）を提供する。

use regex::{Captures, Regex};

use super::Value;

// ────────────────────────────────────────────────────────────────────────────
// str.format(*args, **kwargs)
// ────────────────────────────────────────────────────────────────────────────

/// `"{0} {name:.2f}"` のようなテンプレート文字列に引数を埋め込む。
///
/// - `{}` → 自動インデックス（呼び出しごとにカウントアップ）
/// - `{0}`, `{1}` → 位置引数インデックス
/// - `{name}` → キーワード引数
/// - `{0:.2f}`, `{name:>10}` → 簡易書式指定（`:` 以降）
/// - `{{` / `}}` → エスケープされた `{` / `}`
pub fn str_format(
    template: &str,
    pos_args: &[Value],
    kw_args: &[(String, Value)],
    display: &dyn Fn(&Value) -> String,
) -> Result<String, String> {
    let mut result = String::new();
    let chars: Vec<char> = template.chars().collect();
    let mut i = 0;
    let mut auto_idx = 0usize;

    while i < chars.len() {
        if chars[i] == '{' {
            if i + 1 < chars.len() && chars[i + 1] == '{' {
                result.push('{');
                i += 2;
                continue;
            }
            i += 1; // consume {
            let mut field = String::new();
            while i < chars.len() && chars[i] != '}' {
                field.push(chars[i]);
                i += 1;
            }
            if i < chars.len() {
                i += 1;
            } // consume }

            // Split field into name and format_spec
            let (field_name, fmt_spec) = if let Some(colon) = field.find(':') {
                (&field[..colon], &field[colon + 1..])
            } else {
                (field.as_str(), "")
            };

            let val = if field_name.is_empty() {
                // Auto index
                let v = pos_args.get(auto_idx).ok_or_else(|| {
                    format!(
                        "IndexError: not enough positional arguments (needed {})",
                        auto_idx + 1
                    )
                })?;
                auto_idx += 1;
                v
            } else if let Ok(idx) = field_name.parse::<usize>() {
                pos_args.get(idx).ok_or_else(|| {
                    format!("IndexError: positional argument index {idx} out of range")
                })?
            } else {
                kw_args
                    .iter()
                    .find(|(k, _)| k == field_name)
                    .map(|(_, v)| v)
                    .ok_or_else(|| format!("KeyError: keyword argument '{field_name}' not found"))?
            };

            result.push_str(&apply_format_spec(val, fmt_spec, display)?);
        } else if chars[i] == '}' {
            if i + 1 < chars.len() && chars[i + 1] == '}' {
                result.push('}');
                i += 2;
                continue;
            }
            result.push('}');
            i += 1;
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }
    Ok(result)
}

/// `{:>10.2f}` のような書式指定子を値に適用する。
///
/// `fill`・`align`・`width`・`precision`・`type` を解析し、
/// 整形された文字列を返す。未知の型文字の場合は `ValueError` を返す。
fn apply_format_spec(
    val: &Value,
    spec: &str,
    display: &dyn Fn(&Value) -> String,
) -> Result<String, String> {
    if spec.is_empty() {
        return Ok(display(val));
    }

    // Parse spec: [[fill]align][sign][#][0][width][grouping_option][.precision][type]
    let chars: Vec<char> = spec.chars().collect();
    let mut i = 0;

    // fill + align: optional 1-char fill followed by < > ^ =
    let (fill, align) = if chars.len() >= 2 && matches!(chars[1], '<' | '>' | '^' | '=') {
        (chars[0], Some(chars[1]))
    } else if matches!(chars.first(), Some('<') | Some('>') | Some('^') | Some('=')) {
        (' ', Some(chars[0]))
    } else {
        (' ', None)
    };
    if align.is_some() {
        i += if fill == ' ' && matches!(chars[0], '<' | '>' | '^' | '=') {
            1
        } else {
            2
        };
    }

    // sign
    let _sign = if i < chars.len() && matches!(chars[i], '+' | '-' | ' ') {
        let s = chars[i];
        i += 1;
        Some(s)
    } else {
        None
    };

    // # flag (alternate form)
    if i < chars.len() && chars[i] == '#' {
        i += 1;
    }

    // zero flag: when '0' precedes width and no explicit align, use '0' fill with right-align
    let zero_flag = if i < chars.len() && chars[i] == '0' && align.is_none() {
        i += 1;
        true
    } else {
        false
    };
    let fill = if zero_flag { '0' } else { fill };
    let align = if zero_flag { Some('>') } else { align };

    // width
    let width_start = i;
    while i < chars.len() && chars[i].is_ascii_digit() {
        i += 1;
    }
    let width: usize = if width_start < i {
        chars[width_start..i]
            .iter()
            .collect::<String>()
            .parse()
            .unwrap_or(0)
    } else {
        0
    };

    // grouping (_  or ,)
    if i < chars.len() && (chars[i] == '_' || chars[i] == ',') {
        i += 1;
    }

    // .precision
    let precision: Option<usize> = if i < chars.len() && chars[i] == '.' {
        i += 1;
        let p_start = i;
        while i < chars.len() && chars[i].is_ascii_digit() {
            i += 1;
        }
        Some(
            chars[p_start..i]
                .iter()
                .collect::<String>()
                .parse()
                .unwrap_or(0),
        )
    } else {
        None
    };

    // type
    let type_char: Option<char> = if i < chars.len() {
        Some(chars[i])
    } else {
        None
    };

    let s = match type_char {
        Some('d') | Some('i') => match val {
            Value::Int(n) => format!("{n}"),
            Value::Float(f) => format!("{}", *f as i64),
            _ => display(val),
        },
        Some('f') | Some('F') => {
            let n = to_f64(val)?;
            let prec = precision.unwrap_or(6);
            format!("{:.prec$}", n)
        }
        Some('e') => {
            let n = to_f64(val)?;
            let prec = precision.unwrap_or(6);
            format!("{:.prec$e}", n)
        }
        Some('E') => {
            let n = to_f64(val)?;
            let prec = precision.unwrap_or(6);
            format!("{:.prec$E}", n)
        }
        Some('g') | Some('G') => {
            let n = to_f64(val)?;
            let prec = precision.unwrap_or(6);
            if n.abs() < 1e-4 || n.abs() >= 10f64.powi(prec as i32) {
                format!("{:.prec$e}", n)
            } else {
                format!("{:.prec$}", n)
            }
        }
        Some('x') => match val {
            Value::Int(n) => format!("{:x}", n),
            _ => display(val),
        },
        Some('X') => match val {
            Value::Int(n) => format!("{:X}", n),
            _ => display(val),
        },
        Some('o') => match val {
            Value::Int(n) => format!("{:o}", n),
            _ => display(val),
        },
        Some('b') => match val {
            Value::Int(n) => format!("{:b}", n),
            _ => display(val),
        },
        Some('s') | None => {
            let mut s = display(val);
            if let Some(p) = precision {
                s.truncate(p);
            }
            s
        }
        Some('%') => {
            let n = to_f64(val)?;
            let prec = precision.unwrap_or(6);
            format!("{:.prec$}%", n * 100.0)
        }
        Some(c) => return Err(format!("ValueError: unknown format type '{c}'")),
    };

    // Apply width / alignment
    if width > 0 && s.len() < width {
        let pad = width - s.len();
        let aligned = match align.unwrap_or(if matches!(val, Value::Int(_) | Value::Float(_)) {
            '>'
        } else {
            '<'
        }) {
            '<' => format!("{}{}", s, fill.to_string().repeat(pad)),
            '>' => format!("{}{}", fill.to_string().repeat(pad), s),
            '^' => {
                let left = pad / 2;
                let right = pad - left;
                format!(
                    "{}{}{}",
                    fill.to_string().repeat(left),
                    s,
                    fill.to_string().repeat(right)
                )
            }
            _ => s,
        };
        Ok(aligned)
    } else {
        Ok(s)
    }
}

/// `Value` を `f64` に変換する補助関数。`Int` と `Float` のみ許容し、それ以外は `TypeError` を返す。
fn to_f64(val: &Value) -> Result<f64, String> {
    match val {
        Value::Float(f) => Ok(*f),
        Value::Int(n) => Ok(*n as f64),
        other => Err(format!(
            "TypeError: cannot format {} as float",
            value_type_name(other)
        )),
    }
}

/// `Value` のランタイム型名を静的文字列で返す（エラーメッセージ用の簡易版）。
fn value_type_name(v: &Value) -> &'static str {
    match v {
        Value::Int(_) => "int",
        Value::Float(_) => "float",
        Value::Str(_) => "str",
        Value::Bool(_) => "bool",
        Value::None => "NoneType",
        Value::List(_) => "list",
        Value::Tuple(_) => "tuple",
        Value::Dict(_) => "dict",
        Value::Set(_) => "set",
        _ => "object",
    }
}

// ────────────────────────────────────────────────────────────────────────────
// str % args  (%-format operator)
// ────────────────────────────────────────────────────────────────────────────

/// `"Hello %s, you are %d years old" % ("Alice", 30)` を処理する。
///
/// 対応する書式指定子: `%d %i %u %f %e %E %g %G %s %r %x %X %o %b %%`
/// 幅・精度の数値指定（例: `%10d`, `%.2f`, `%-8s`）に対応する。
pub fn percent_format(
    fmt: &str,
    args: &[Value],
    display: &dyn Fn(&Value) -> String,
) -> Result<String, String> {
    let mut result = String::new();
    let chars: Vec<char> = fmt.chars().collect();
    let mut i = 0;
    let mut arg_idx = 0usize;

    while i < chars.len() {
        if chars[i] != '%' {
            result.push(chars[i]);
            i += 1;
            continue;
        }
        i += 1;
        if i >= chars.len() {
            break;
        }

        if chars[i] == '%' {
            result.push('%');
            i += 1;
            continue;
        }

        // Flags: -, +, space, #, 0
        let mut flags = String::new();
        while i < chars.len() && matches!(chars[i], '-' | '+' | ' ' | '#' | '0') {
            flags.push(chars[i]);
            i += 1;
        }
        let left_align = flags.contains('-');
        let zero_pad = flags.contains('0') && !left_align;
        let plus_sign = flags.contains('+');

        // Width
        let width_start = i;
        while i < chars.len() && chars[i].is_ascii_digit() {
            i += 1;
        }
        let width: usize = if width_start < i {
            chars[width_start..i]
                .iter()
                .collect::<String>()
                .parse()
                .unwrap_or(0)
        } else {
            0
        };

        // .precision
        let precision: Option<usize> = if i < chars.len() && chars[i] == '.' {
            i += 1;
            let p_start = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            Some(
                chars[p_start..i]
                    .iter()
                    .collect::<String>()
                    .parse()
                    .unwrap_or(0),
            )
        } else {
            None
        };

        if i >= chars.len() {
            break;
        }
        let spec = chars[i];
        i += 1;

        let arg = args.get(arg_idx).ok_or_else(|| {
            format!(
                "TypeError: not enough arguments for format string (needed index {})",
                arg_idx
            )
        })?;
        arg_idx += 1;

        let s = match spec {
            'd' | 'i' | 'u' => {
                let n = to_i64(arg)?;
                let s = if plus_sign && n >= 0 {
                    format!("+{n}")
                } else {
                    format!("{n}")
                };
                pad_str(&s, width, left_align, if zero_pad { '0' } else { ' ' })
            }
            'f' | 'F' => {
                let n = to_f64(arg)?;
                let prec = precision.unwrap_or(6);
                let s = if plus_sign && n >= 0.0 {
                    format!("+{:.prec$}", n)
                } else {
                    format!("{:.prec$}", n)
                };
                pad_str(&s, width, left_align, if zero_pad { '0' } else { ' ' })
            }
            'e' => {
                let n = to_f64(arg)?;
                let prec = precision.unwrap_or(6);
                let s = format!("{:.prec$e}", n);
                pad_str(&s, width, left_align, ' ')
            }
            'E' => {
                let n = to_f64(arg)?;
                let prec = precision.unwrap_or(6);
                let s = format!("{:.prec$E}", n);
                pad_str(&s, width, left_align, ' ')
            }
            'g' | 'G' => {
                let n = to_f64(arg)?;
                let prec = precision.unwrap_or(6).max(1);
                let s = if n.abs() < 1e-4 || n.abs() >= 10f64.powi(prec as i32) {
                    format!("{:.prec$e}", n)
                } else {
                    format!("{:.prec$}", n)
                };
                pad_str(&s, width, left_align, ' ')
            }
            's' => {
                let mut s = display(arg);
                if let Some(p) = precision {
                    s.truncate(p);
                }
                pad_str(&s, width, left_align, ' ')
            }
            'r' => {
                let s = format!("{:?}", display(arg));
                pad_str(&s, width, left_align, ' ')
            }
            'x' => {
                let n = to_i64(arg)?;
                let s = format!("{:x}", n);
                pad_str(&s, width, left_align, if zero_pad { '0' } else { ' ' })
            }
            'X' => {
                let n = to_i64(arg)?;
                let s = format!("{:X}", n);
                pad_str(&s, width, left_align, if zero_pad { '0' } else { ' ' })
            }
            'o' => {
                let n = to_i64(arg)?;
                let s = format!("{:o}", n);
                pad_str(&s, width, left_align, if zero_pad { '0' } else { ' ' })
            }
            'b' => {
                let n = to_i64(arg)?;
                let s = format!("{:b}", n);
                pad_str(&s, width, left_align, if zero_pad { '0' } else { ' ' })
            }
            'c' => {
                let n = to_i64(arg)? as u32;
                let ch = char::from_u32(n)
                    .ok_or_else(|| format!("ValueError: %c: invalid Unicode code point {n}"))?;
                pad_str(&ch.to_string(), width, left_align, ' ')
            }
            c => return Err(format!("ValueError: unsupported format character '{c}'")),
        };
        result.push_str(&s);
    }

    Ok(result)
}

/// `Value` を `i64` に変換する補助関数。`Int`・`Float`・`Bool` を受け入れ、それ以外は `TypeError` を返す。
fn to_i64(val: &Value) -> Result<i64, String> {
    match val {
        Value::Int(n) => Ok(*n),
        Value::Float(f) => Ok(*f as i64),
        Value::Bool(b) => Ok(if *b { 1 } else { 0 }),
        other => Err(format!(
            "TypeError: %d format: cannot convert {} to int",
            value_type_name(other)
        )),
    }
}

/// 文字列 `s` を指定した `width` に合わせてパディングする。
/// `left_align` が `true` の場合は左寄せ、`false` の場合は右寄せ。`fill` はパディング文字。
fn pad_str(s: &str, width: usize, left_align: bool, fill: char) -> String {
    if s.len() >= width {
        return s.to_string();
    }
    let pad = width - s.len();
    if left_align {
        format!("{}{}", s, fill.to_string().repeat(pad))
    } else {
        format!("{}{}", fill.to_string().repeat(pad), s)
    }
}

// ────────────────────────────────────────────────────────────────────────────
// 正規表現メソッド
// ────────────────────────────────────────────────────────────────────────────

/// パターン文字列とフラグ文字列（`"i"`, `"m"`, `"s"`, `"x"` の組み合わせ）から `Regex` を構築する。
/// フラグは `(?i)` などのインラインフラグとしてパターン先頭に付与する。
fn build_regex(pattern: &str, flags: &str) -> Result<Regex, String> {
    let prefix: String = flags
        .chars()
        .filter_map(|c| match c {
            'i' | 'I' => Some("(?i)"),
            'm' | 'M' => Some("(?m)"),
            's' | 'S' => Some("(?s)"),
            'x' | 'X' => Some("(?x)"),
            _ => None,
        })
        .collect();
    let full = format!("{}{}", prefix, pattern);
    Regex::new(&full).map_err(|e| format!("RegexError: {e}"))
}

/// `text.match(pattern[, flags])` — 文字列の先頭でマッチを試みる。
/// マッチした文字列を返す、見つからなければ `None`。
pub fn regex_match(text: &str, pattern: &str, flags: &str) -> Result<Option<String>, String> {
    let re = build_regex(&format!("^(?:{pattern})"), flags)?;
    Ok(re.find(text).map(|m| m.as_str().to_string()))
}

/// `text.search(pattern[, flags])` — 文字列のどこかでマッチを試みる。
/// 最初にマッチした文字列を返す、見つからなければ `None`。
pub fn regex_search(text: &str, pattern: &str, flags: &str) -> Result<Option<String>, String> {
    let re = build_regex(pattern, flags)?;
    Ok(re.find(text).map(|m| m.as_str().to_string()))
}

/// `text.findall(pattern[, flags])` — 全マッチを list[str] で返す。
/// キャプチャグループがある場合は最初のグループの値のリストを返す。
pub fn regex_findall(text: &str, pattern: &str, flags: &str) -> Result<Vec<String>, String> {
    let re = build_regex(pattern, flags)?;
    let captures_count = re.captures_len();
    if captures_count > 1 {
        // Return first capture group
        let mut out = Vec::new();
        for cap in re.captures_iter(text) {
            if let Some(g) = cap.get(1) {
                out.push(g.as_str().to_string());
            }
        }
        Ok(out)
    } else {
        Ok(re.find_iter(text).map(|m| m.as_str().to_string()).collect())
    }
}

/// `text.sub(pattern, repl[, count[, flags]])` — マッチを `repl` で置換する。
/// `count=0` は全置換（Python の `re.sub` と同じ）。
pub fn regex_sub(
    text: &str,
    pattern: &str,
    repl: &str,
    count: usize,
    flags: &str,
) -> Result<String, String> {
    let re = build_regex(pattern, flags)?;
    if count == 0 {
        // Replace all
        let result = re.replace_all(text, |caps: &Captures| expand_replacement(repl, caps));
        Ok(result.into_owned())
    } else {
        let mut result = String::new();
        let mut last = 0;
        let mut n = 0;
        for mat in re.find_iter(text) {
            if n >= count {
                break;
            }
            result.push_str(&text[last..mat.start()]);
            // For simple replacements without group references
            let caps = re.captures(&text[mat.start()..mat.end()]);
            if let Some(c) = caps {
                result.push_str(&expand_replacement(repl, &c));
            } else {
                result.push_str(repl);
            }
            last = mat.end();
            n += 1;
        }
        result.push_str(&text[last..]);
        Ok(result)
    }
}

/// 置換文字列 `repl` 内の `\1`, `\2` 等のバックリファレンスをキャプチャグループの内容で展開する。
fn expand_replacement(repl: &str, caps: &Captures) -> String {
    let mut out = String::new();
    let chars: Vec<char> = repl.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() && chars[i + 1].is_ascii_digit() {
            i += 1;
            let n = chars[i].to_digit(10).unwrap_or(0) as usize;
            if let Some(g) = caps.get(n) {
                out.push_str(g.as_str());
            }
            i += 1;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// `text.regex_split(pattern[, maxsplit[, flags]])` — 正規表現で分割する。
pub fn regex_split(
    text: &str,
    pattern: &str,
    maxsplit: usize,
    flags: &str,
) -> Result<Vec<String>, String> {
    let re = build_regex(pattern, flags)?;
    let parts: Vec<String> = if maxsplit == 0 {
        re.split(text).map(|s| s.to_string()).collect()
    } else {
        re.splitn(text, maxsplit + 1)
            .map(|s| s.to_string())
            .collect()
    };
    Ok(parts)
}
