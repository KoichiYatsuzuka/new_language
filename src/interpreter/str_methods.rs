// str_methods.rs — 文字列メソッド補助関数
//
// Python 互換の str.format() と % 書式演算子の実装、および
// 正規表現操作（regex クレート使用）を提供する。

use regex::{Captures, Regex};

use super::{Interpreter, Value};

// ────────────────────────────────────────────────────────────────────────────
// str.format(*args, **kwargs)
// ────────────────────────────────────────────────────────────────────────────

/// `str.format` の置換フィールドの番号の付け方（自動と手動は混ぜられない・CPython と同じ）。
enum FieldNumbering {
    Unknown,
    Auto(usize),
    Manual,
}

impl Interpreter {
    /// `"{0} {name!r:>{w}}".format(..)`（python_builtins_plan.md のタスク 3-4）。CPython の `str.format` と同じ規則・文言:
    ///
    /// - `{}`（自動の番号）/ `{0}`（位置）/ `{name}`（キーワード）。自動と手動は混ぜられない
    /// - 属性・添字（`{0.name}` / `{0[1]}` / `{p[key]}`。添字は数字なら int、それ以外は文字列）
    /// - 変換（`!r` / `!s` / `!a`）と書式指定（`:spec`。中に置換フィールドを 1 段入れられる `{:{w}.{p}f}`）
    /// - 書式は `format_value`（`format()` / f-string と同じ・`__format__` を呼ぶ）
    /// - `{{` / `}}` は `{` / `}`
    ///
    /// ⚠ 以前は書式指定の解釈が粗く（`,` や `#` を黙って捨てた・`e` / `g` の綴りが違った）、変換・属性・添字が無かった。
    pub(crate) fn py_str_format(
        &mut self,
        template: &str,
        pos_args: &[Value],
        kw_args: &[(String, Value)],
    ) -> Result<String, String> {
        let mut numbering = FieldNumbering::Unknown;
        self.str_format_level(template, pos_args, kw_args, &mut numbering, 2)
    }

    fn str_format_level(
        &mut self,
        template: &str,
        pos_args: &[Value],
        kw_args: &[(String, Value)],
        numbering: &mut FieldNumbering,
        depth: usize,
    ) -> Result<String, String> {
        if depth == 0 {
            return Err("ValueError: Max string recursion exceeded".to_string());
        }
        let chars: Vec<char> = template.chars().collect();
        let mut out = String::new();
        let mut i = 0;
        while i < chars.len() {
            match chars[i] {
                '{' if chars.get(i + 1) == Some(&'{') => {
                    out.push('{');
                    i += 2;
                }
                '}' if chars.get(i + 1) == Some(&'}') => {
                    out.push('}');
                    i += 2;
                }
                '}' => return Err("ValueError: Single '}' encountered in format string".to_string()),
                '{' => {
                    // 対応する `}` まで（書式指定の中の `{w}` の入れ子を数える）。
                    let mut level = 1;
                    let mut j = i + 1;
                    while j < chars.len() {
                        match chars[j] {
                            '{' => level += 1,
                            '}' => {
                                level -= 1;
                                if level == 0 {
                                    break;
                                }
                            }
                            _ => {}
                        }
                        j += 1;
                    }
                    if j >= chars.len() {
                        return Err(if i + 1 == chars.len() {
                            "ValueError: Single '{' encountered in format string".to_string()
                        } else {
                            "ValueError: expected '}' before end of string".to_string()
                        });
                    }
                    let field: String = chars[i + 1..j].iter().collect();
                    out.push_str(&self.str_format_field(&field, pos_args, kw_args, numbering, depth)?);
                    i = j + 1;
                }
                c => {
                    out.push(c);
                    i += 1;
                }
            }
        }
        Ok(out)
    }

    /// 置換フィールド 1 つ（`{` と `}` の間）を書く。
    fn str_format_field(
        &mut self,
        field: &str,
        pos_args: &[Value],
        kw_args: &[(String, Value)],
        numbering: &mut FieldNumbering,
        depth: usize,
    ) -> Result<String, String> {
        // フィールド名は `[..]` の外の `!` / `:` まで（添字の中の `:` は名前の一部・CPython の `parse_field`）。
        let fc: Vec<char> = field.chars().collect();
        let mut bracket = 0usize;
        let mut k = 0;
        while k < fc.len() {
            match fc[k] {
                '[' => bracket += 1,
                ']' => bracket = bracket.saturating_sub(1),
                '!' | ':' if bracket == 0 => break,
                _ => {}
            }
            k += 1;
        }
        let name: String = fc[..k].iter().collect();
        let mut conversion = None;
        let mut spec = String::new();
        if k < fc.len() && fc[k] == '!' {
            let Some(&c) = fc.get(k + 1) else {
                return Err("ValueError: end of string while looking for conversion specifier".to_string());
            };
            conversion = Some(c);
            k += 2;
            if k < fc.len() && fc[k] != ':' {
                return Err("ValueError: expected ':' after conversion specifier".to_string());
            }
        }
        if k < fc.len() && fc[k] == ':' {
            spec = fc[k + 1..].iter().collect();
        }
        let value = self.str_format_lookup(&name, pos_args, kw_args, numbering)?;
        let value = match conversion {
            None => value,
            Some('r') => Value::str(self.repr_val(&value)?),
            Some('s') => Value::str(self.display_str(&value)?),
            Some('a') => self.py_ascii(vec![value])?,
            Some(c) => return Err(format!("ValueError: Unknown conversion specifier {c}")),
        };
        // 書式指定の中の置換フィールド（`{:{w}}`）は同じ引数で先に展開する。
        let spec = if spec.contains('{') {
            self.str_format_level(&spec, pos_args, kw_args, numbering, depth - 1)?
        } else {
            spec
        };
        self.format_value(&value, &spec)
    }

    /// フィールド名（`0` / `name` / 空・続く `.attr` / `[key]`）を値にする。
    fn str_format_lookup(
        &mut self,
        name: &str,
        pos_args: &[Value],
        kw_args: &[(String, Value)],
        numbering: &mut FieldNumbering,
    ) -> Result<Value, String> {
        let first_end = name.find(['.', '[']).unwrap_or(name.len());
        let (first, mut rest) = name.split_at(first_end);
        let mut value = if first.is_empty() {
            let idx = match numbering {
                FieldNumbering::Manual => {
                    return Err(
                        "ValueError: cannot switch from manual field specification to automatic field numbering"
                            .to_string(),
                    )
                }
                FieldNumbering::Unknown => 0,
                FieldNumbering::Auto(n) => *n,
            };
            *numbering = FieldNumbering::Auto(idx + 1);
            Self::str_format_positional(pos_args, idx)?
        } else if let Ok(idx) = first.parse::<usize>() {
            if matches!(numbering, FieldNumbering::Auto(_)) {
                return Err(
                    "ValueError: cannot switch from automatic field numbering to manual field specification".to_string(),
                );
            }
            *numbering = FieldNumbering::Manual;
            Self::str_format_positional(pos_args, idx)?
        } else {
            kw_args
                .iter()
                .find(|(k, _)| k == first)
                .map(|(_, v)| v.clone())
                .ok_or_else(|| format!("KeyError: '{first}'"))?
        };
        while !rest.is_empty() {
            if let Some(r) = rest.strip_prefix('.') {
                let end = r.find(['.', '[']).unwrap_or(r.len());
                if end == 0 {
                    return Err("ValueError: Empty attribute in format string".to_string());
                }
                value = self.get_attr_val(value, &r[..end], None)?;
                rest = &r[end..];
            } else if let Some(r) = rest.strip_prefix('[') {
                let end = r.find(']').ok_or_else(|| "ValueError: Missing ']' in format string".to_string())?;
                let key = &r[..end];
                if key.is_empty() {
                    return Err("ValueError: Empty attribute in format string".to_string());
                }
                let key = match key.parse::<i64>() {
                    Ok(n) if !key.starts_with(['-', '+']) => Value::Int(n),
                    _ => Value::str(key),
                };
                value = self.eval_subscript(value, key)?;
                rest = &r[end + 1..];
            } else {
                return Err("ValueError: Only '.' or '[' may follow ']' in format field specifier".to_string());
            }
        }
        Ok(value)
    }

    fn str_format_positional(pos_args: &[Value], idx: usize) -> Result<Value, String> {
        pos_args
            .get(idx)
            .cloned()
            .ok_or_else(|| format!("IndexError: Replacement index {idx} out of range for positional args tuple"))
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
        Value::FrozenSet(_) => "frozenset",
        Value::Bytes(_) => "bytes",
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
            // `e` / `g` は `format()` と同じ綴り（指数は符号つき 2 桁以上・`g` は末尾の 0 を消す・タスク 3-4）。
            // ⚠ 以前は Rust の書式のまま（`%e` が `1.234568e4`、`%g` が `12345.678000`）だった。
            'e' | 'E' | 'g' | 'G' => {
                use crate::interpreter::ops::format_spec as fs;
                let n = to_f64(arg)?;
                let upper = spec.is_ascii_uppercase();
                let alt = flags.contains('#');
                let body = if n.is_nan() || n.is_infinite() {
                    let b = if n.is_nan() { "nan" } else { "inf" };
                    if upper { b.to_uppercase() } else { b.to_string() }
                } else if matches!(spec, 'e' | 'E') {
                    fs::float_exp(n.abs(), precision.unwrap_or(6), upper, alt)
                } else {
                    fs::float_general(n.abs(), precision.unwrap_or(6), upper, alt, false)
                };
                let sign = if n.is_sign_negative() && !n.is_nan() {
                    "-"
                } else if plus_sign {
                    "+"
                } else if flags.contains(' ') {
                    " "
                } else {
                    ""
                };
                pad_str(&format!("{sign}{body}"), width, left_align, ' ')
            }
            's' => {
                let mut s = display(arg);
                // 精度は文字の数（⚠ 以前はバイトで切り、文字の途中なら Rust のパニックで落ちた・タスク 1-17）。
                if let Some(p) = precision {
                    s = s.chars().take(p).collect();
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
/// 幅は文字の数（⚠ 以前はバイトの数で、非 ASCII を詰め足りなかった・タスク 1-17）。
fn pad_str(s: &str, width: usize, left_align: bool, fill: char) -> String {
    let n = s.chars().count();
    if n >= width {
        return s.to_string();
    }
    let pad = width - n;
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
