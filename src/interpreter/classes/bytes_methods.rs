// classes/bytes_methods.rs — バイト列(bytes)メソッドのディスパッチ: eval_bytes_method（python_builtins_plan.md のタスク 3-1）。
//
// `string_methods.rs` / `set_methods.rs` と同じ「レシーバ 1 種類 = 1 ファイル」の形。
// 結果・誤りの文言は CPython 3.12 と同じ。⚠ 書き換えるメソッドは無い（bytes は不変）。

use {
    std::cell::RefCell,
    std::rc::Rc,
    crate::interpreter::{GeneratorState, Interpreter, Value},
};

/// ASCII の空白（`bytes.split()` / `strip()` の既定・CPython の `Py_ISSPACE`）。
const ASCII_WHITESPACE: &[u8] = b" \t\n\r\x0b\x0c";

/// 評価済み引数を、仮引数の名前の並び `names` に沿って位置・キーワードから取り出す。
fn bind_args(
    evaled: Vec<(Option<String>, Value, bool)>,
    names: &[&str],
    method: &str,
) -> Result<Vec<Option<Value>>, String> {
    let mut out: Vec<Option<Value>> = vec![None; names.len()];
    let mut pos = 0;
    for (name, v, _) in evaled {
        let idx = match name {
            None => {
                if pos >= names.len() {
                    return Err(format!(
                        "TypeError: {method}() takes at most {} argument{} ({} given)",
                        names.len(),
                        if names.len() == 1 { "" } else { "s" },
                        pos + 1
                    ));
                }
                pos += 1;
                pos - 1
            }
            Some(n) => names.iter().position(|x| *x == n).ok_or_else(|| {
                format!("TypeError: {method}() got an unexpected keyword argument '{n}'")
            })?,
        };
        out[idx] = Some(v);
    }
    Ok(out)
}

/// bytes を求める位置の引数（`replace` / `split` / `strip` / `startswith` …）。
fn bytes_arg(v: &Value, interp: &Interpreter) -> Result<Rc<[u8]>, String> {
    match v {
        Value::Bytes(b) => Ok(b.clone()),
        other => Err(format!("TypeError: a bytes-like object is required, not '{}'", interp.type_name(other))),
    }
}

/// `find` / `count` / `index` の探す値（bytes か 0〜255 の int）。
fn needle_arg(v: &Value, interp: &Interpreter) -> Result<Vec<u8>, String> {
    match v {
        Value::Int(n) if (0..=255).contains(n) => Ok(vec![*n as u8]),
        Value::Int(_) => Err("ValueError: byte must be in range(0, 256)".to_string()),
        other => Ok(bytes_arg(other, interp)?.to_vec()),
    }
}

/// `start` / `end`（`None` は端）を CPython のスライスと同じく `[0, len]` に収める。
fn norm_range(len: usize, start: Option<&Value>, end: Option<&Value>) -> Result<(usize, usize), String> {
    let n = len as i64;
    let clamp = |v: Option<&Value>, dflt: i64| -> Result<i64, String> {
        let raw = match v {
            None | Some(Value::None) => return Ok(dflt),
            Some(Value::Int(i)) => *i,
            Some(_) => return Err("TypeError: slice indices must be integers or None".to_string()),
        };
        let x = if raw < 0 { raw + n } else { raw };
        Ok(x.clamp(0, n))
    };
    Ok((clamp(start, 0)? as usize, clamp(end, n)? as usize))
}

/// `hay` の中の `needle` の位置（重ならないもの・左から）。
fn find_all(hay: &[u8], needle: &[u8]) -> Vec<usize> {
    let mut out = Vec::new();
    if needle.is_empty() {
        return (0..=hay.len()).collect();
    }
    let mut i = 0;
    while i + needle.len() <= hay.len() {
        if &hay[i..i + needle.len()] == needle {
            out.push(i);
            i += needle.len();
        } else {
            i += 1;
        }
    }
    out
}

fn bytes_val(v: Vec<u8>) -> Value {
    Value::Bytes(v.into())
}

impl Interpreter {
    /// `bytes` のメソッドを評価済み引数で呼ぶ（タスク 3-1）。呼び出し元は `eval_method_call_full` のみ。
    /// 読める名前は `type_check::names::BUILTIN_VALUE_METHODS` の `bytes` の行（片方だけ足さないこと）。
    pub(crate) fn eval_bytes_method(
        &mut self,
        b: Rc<[u8]>,
        method_name: &str,
        evaled: Vec<(Option<String>, Value, bool)>,
    ) -> Result<Value, String> {
        match method_name {
            "__iter__" => {
                Self::expect_no_args_evaled(&evaled, "bytes", "__iter__")?;
                let items = b.iter().map(|&x| Value::Int(x as i64)).collect();
                Ok(Value::Generator(Rc::new(RefCell::new(GeneratorState::materialized(items)))))
            }
            // `decode(encoding="utf-8", errors="strict")`（`ops::codecs`）。
            "decode" => {
                let a = bind_args(evaled, &["encoding", "errors"], "decode")?;
                let encoding = self.str_arg(a[0].as_ref(), "utf-8", "decode", "encoding")?;
                let errors = self.str_arg(a[1].as_ref(), "strict", "decode", "errors")?;
                let codec = crate::interpreter::ops::codecs::Codec::lookup(&encoding)?;
                Ok(Value::str(crate::interpreter::ops::codecs::decode(&b, codec, &errors)?))
            }
            // `hex(sep=None, bytes_per_sep=1)`。`sep` は 1 文字。区切りは `bytes_per_sep` バイトごと（正なら右から）。
            "hex" => {
                let a = bind_args(evaled, &["sep", "bytes_per_sep"], "hex")?;
                let digits: Vec<String> = b.iter().map(|x| format!("{x:02x}")).collect();
                let sep = match &a[0] {
                    None | Some(Value::None) => return Ok(Value::str(digits.concat())),
                    Some(Value::Str(s)) => s.to_string(),
                    Some(Value::Bytes(s)) => String::from_utf8_lossy(s).into_owned(),
                    Some(other) => {
                        return Err(format!("TypeError: sep must be str or bytes, not {}", self.type_name(other)))
                    }
                };
                if sep.chars().count() != 1 {
                    return Err("ValueError: sep must be length 1.".to_string());
                }
                let per = match &a[1] {
                    None => 1i64,
                    Some(Value::Int(n)) => *n,
                    Some(other) => {
                        return Err(format!("TypeError: '{}' object cannot be interpreted as an integer", self.type_name(other)))
                    }
                };
                if per == 0 || digits.is_empty() {
                    return Ok(Value::str(digits.concat()));
                }
                let k = per.unsigned_abs() as usize;
                let mut groups: Vec<String> = Vec::new();
                if per > 0 {
                    // 右から k バイトずつ（先頭の群が短くなる）。
                    let first = digits.len() % k;
                    if first > 0 {
                        groups.push(digits[..first].concat());
                    }
                    for chunk in digits[first..].chunks(k) {
                        groups.push(chunk.concat());
                    }
                } else {
                    for chunk in digits.chunks(k) {
                        groups.push(chunk.concat());
                    }
                }
                Ok(Value::str(groups.join(&sep)))
            }
            "count" | "find" | "rfind" | "index" | "rindex" => {
                let a = bind_args(evaled, &["sub", "start", "end"], method_name)?;
                let sub = a[0].as_ref().ok_or_else(|| {
                    format!("TypeError: {method_name}() takes at least 1 argument (0 given)")
                })?;
                let needle = needle_arg(sub, self)?;
                let (s, e) = norm_range(b.len(), a[1].as_ref(), a[2].as_ref())?;
                // `start > end` は「見つからない」（CPython と同じ）。
                let hits: Vec<usize> = if s > e { Vec::new() } else { find_all(&b[s..e], &needle) };
                match method_name {
                    "count" => Ok(Value::Int(hits.len() as i64)),
                    _ => {
                        let pos = if method_name.starts_with('r') {
                            // 右から: 重なりも含めて最後の位置を探す。
                            if s > e || needle.len() > e - s {
                                None
                            } else {
                                (0..=(e - s - needle.len())).rev().find(|&i| b[s + i..s + i + needle.len()] == needle[..])
                            }
                        } else {
                            hits.first().copied()
                        };
                        match pos {
                            Some(p) => Ok(Value::Int((s + p) as i64)),
                            None if method_name.ends_with("index") => {
                                Err("ValueError: subsection not found".to_string())
                            }
                            None => Ok(Value::Int(-1)),
                        }
                    }
                }
            }
            // `startswith(prefix, start, end)` / `endswith(..)`。`prefix` は bytes か bytes の組。
            "startswith" | "endswith" => {
                let a = bind_args(evaled, &["prefix", "start", "end"], method_name)?;
                let cand = a[0].as_ref().ok_or_else(|| {
                    format!("TypeError: {method_name}() takes at least 1 argument (0 given)")
                })?;
                let (s, e) = norm_range(b.len(), a[1].as_ref(), a[2].as_ref())?;
                let part: &[u8] = if s > e { &[] } else { &b[s..e] };
                let options: Vec<Rc<[u8]>> = match cand {
                    Value::Tuple(t) => t.all_values().iter().map(|v| bytes_arg(v, self)).collect::<Result<_, _>>()?,
                    other => vec![bytes_arg(other, self).map_err(|_| {
                        format!(
                            "TypeError: {method_name} first arg must be bytes or a tuple of bytes, not {}",
                            self.type_name(other)
                        )
                    })?],
                };
                let hit = s <= e
                    && options.iter().any(|o| {
                        if method_name == "startswith" { part.starts_with(o) } else { part.ends_with(o) }
                    });
                Ok(Value::Bool(hit))
            }
            // `replace(old, new, count=-1)`。
            "replace" => {
                let a = bind_args(evaled, &["old", "new", "count"], "replace")?;
                let (Some(old), Some(new)) = (a[0].as_ref(), a[1].as_ref()) else {
                    return Err("TypeError: replace() takes at least 2 arguments".to_string());
                };
                let (old, new) = (bytes_arg(old, self)?, bytes_arg(new, self)?);
                let limit = match &a[2] {
                    None => -1,
                    Some(Value::Int(n)) => *n,
                    Some(other) => {
                        return Err(format!("TypeError: '{}' object cannot be interpreted as an integer", self.type_name(other)))
                    }
                };
                let mut out = Vec::with_capacity(b.len());
                let mut i = 0;
                let mut done = 0i64;
                while i <= b.len() {
                    let can = limit < 0 || done < limit;
                    if can && b[i..].starts_with(&old) {
                        out.extend_from_slice(&new);
                        done += 1;
                        if old.is_empty() {
                            // 空の old は各バイトの前に入れる（CPython と同じ）。
                            if i < b.len() {
                                out.push(b[i]);
                            }
                            i += 1;
                        } else {
                            i += old.len();
                        }
                    } else {
                        if i < b.len() {
                            out.push(b[i]);
                        }
                        i += 1;
                    }
                }
                Ok(bytes_val(out))
            }
            // `split(sep=None, maxsplit=-1)`。`sep` 無しは ASCII の空白の連なりで分け、空の部分を捨てる。
            "split" => {
                let a = bind_args(evaled, &["sep", "maxsplit"], "split")?;
                let maxsplit = match &a[1] {
                    None => -1,
                    Some(Value::Int(n)) => *n,
                    Some(other) => {
                        return Err(format!("TypeError: '{}' object cannot be interpreted as an integer", self.type_name(other)))
                    }
                };
                let parts: Vec<Vec<u8>> = match &a[0] {
                    None | Some(Value::None) => {
                        let mut parts = Vec::new();
                        let mut i = 0;
                        let n = b.len();
                        while i < n {
                            while i < n && ASCII_WHITESPACE.contains(&b[i]) {
                                i += 1;
                            }
                            if i >= n {
                                break;
                            }
                            if maxsplit >= 0 && parts.len() as i64 == maxsplit {
                                // 残りは末尾の空白だけを除いて 1 つに（CPython と同じ）。
                                let mut end = n;
                                while end > i && ASCII_WHITESPACE.contains(&b[end - 1]) {
                                    end -= 1;
                                }
                                parts.push(b[i..end].to_vec());
                                break;
                            }
                            let start = i;
                            while i < n && !ASCII_WHITESPACE.contains(&b[i]) {
                                i += 1;
                            }
                            parts.push(b[start..i].to_vec());
                        }
                        parts
                    }
                    Some(sep) => {
                        let sep = bytes_arg(sep, self)?;
                        if sep.is_empty() {
                            return Err("ValueError: empty separator".to_string());
                        }
                        let mut parts = Vec::new();
                        let mut start = 0;
                        let mut i = 0;
                        while i + sep.len() <= b.len() {
                            if (maxsplit < 0 || (parts.len() as i64) < maxsplit) && b[i..].starts_with(&sep) {
                                parts.push(b[start..i].to_vec());
                                i += sep.len();
                                start = i;
                            } else {
                                i += 1;
                            }
                        }
                        parts.push(b[start..].to_vec());
                        parts
                    }
                };
                Ok(Value::List(Rc::new(RefCell::new(parts.into_iter().map(bytes_val).collect()))))
            }
            // `sep.join(iterable)`。要素は bytes だけ（文言は CPython と同じ）。
            "join" => {
                let item = Self::one_arg_evaled(evaled, "bytes", "join")?;
                let items = self.drain_iterable(item)?;
                let mut out = Vec::new();
                for (i, v) in items.iter().enumerate() {
                    let Value::Bytes(x) = v else {
                        return Err(format!(
                            "TypeError: sequence item {i}: expected a bytes-like object, {} found",
                            self.type_name(v)
                        ));
                    };
                    if i > 0 {
                        out.extend_from_slice(&b);
                    }
                    out.extend_from_slice(x);
                }
                Ok(bytes_val(out))
            }
            // `strip(chars=None)` / `lstrip` / `rstrip`。既定は ASCII の空白。
            "strip" | "lstrip" | "rstrip" => {
                let a = bind_args(evaled, &["bytes"], method_name)?;
                let chars: Vec<u8> = match &a[0] {
                    None | Some(Value::None) => ASCII_WHITESPACE.to_vec(),
                    Some(v) => bytes_arg(v, self)?.to_vec(),
                };
                let mut s = 0;
                let mut e = b.len();
                if method_name != "rstrip" {
                    while s < e && chars.contains(&b[s]) {
                        s += 1;
                    }
                }
                if method_name != "lstrip" {
                    while e > s && chars.contains(&b[e - 1]) {
                        e -= 1;
                    }
                }
                Ok(bytes_val(b[s..e].to_vec()))
            }
            "upper" | "lower" => {
                Self::expect_no_args_evaled(&evaled, "bytes", method_name)?;
                Ok(bytes_val(if method_name == "upper" { b.to_ascii_uppercase() } else { b.to_ascii_lowercase() }))
            }
            // 判定（ASCII の範囲だけ・空なら偽。`isascii` だけは空でも真・CPython と同じ）。
            "isdigit" | "isalpha" | "isalnum" | "isspace" | "isascii" | "isupper" | "islower" => {
                Self::expect_no_args_evaled(&evaled, "bytes", method_name)?;
                let r = match method_name {
                    "isascii" => b.is_ascii(),
                    "isdigit" => !b.is_empty() && b.iter().all(u8::is_ascii_digit),
                    "isalpha" => !b.is_empty() && b.iter().all(u8::is_ascii_alphabetic),
                    "isalnum" => !b.is_empty() && b.iter().all(u8::is_ascii_alphanumeric),
                    "isspace" => !b.is_empty() && b.iter().all(|x| ASCII_WHITESPACE.contains(x)),
                    "isupper" => b.iter().any(u8::is_ascii_uppercase) && !b.iter().any(u8::is_ascii_lowercase),
                    _ => b.iter().any(u8::is_ascii_lowercase) && !b.iter().any(u8::is_ascii_uppercase),
                };
                Ok(Value::Bool(r))
            }
            _ => Err(format!("AttributeError: 'bytes' object has no attribute '{method_name}'")),
        }
    }

    /// 文字列を求める引数（`encoding` / `errors`）。`None`（省略）は既定値。
    pub(crate) fn str_arg(&self, v: Option<&Value>, dflt: &str, method: &str, name: &str) -> Result<String, String> {
        match v {
            None => Ok(dflt.to_string()),
            Some(Value::Str(s)) => Ok(s.to_string()),
            Some(other) => Err(format!(
                "TypeError: {method}() argument '{name}' must be str, not {}",
                self.type_name(other)
            )),
        }
    }
}
