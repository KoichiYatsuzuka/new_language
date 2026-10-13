// classes/string_methods.rs — 文字列(str)メソッドのディスパッチ: eval_str_method。

use {
    std::cell::RefCell, std::rc::Rc,
    crate::interpreter::str_methods::{
        regex_findall, regex_match, regex_search, regex_split, regex_sub,
    },
    crate::interpreter::{
        GeneratorState,
        Interpreter, Value,
    },
};

impl Interpreter {
    /// 文字列値のメソッド（`split` / `strip` / `replace` / `startswith` 等）を評価して結果を返す。
    ///
    /// `str` のメソッドを評価済み引数で呼ぶ（#27-b で CallArg 版から変換）。
    /// 呼び出し元は `eval_method_call_full` のみ。
    ///
    /// ⚠ 48 アームの**平坦な組み込みメソッド表**（平均 12 行）。カテゴリ別への分割は #65
    /// だが優先度は低い（得るのは行数だけで、`exec_op` を対象外にしたのと同じ理由が半分当たる）。
    // ⚠ 属性は doc コメントの**後**に置くこと（#51 が潰した orphan doc と同型で、
    // #58〜#68 の診断が「src 唯一の `too_many_lines` がこの形」と指摘していた・#63 で修正）。
    #[allow(clippy::too_many_lines)]
    pub(crate) fn eval_str_method(
        &mut self,
        s: Rc<str>,
        method_name: &str,
        evaled: Vec<(Option<String>, Value, bool)>,
    ) -> Result<Value, String> {
        // ⚠ `format` だけがキーワード引数を使う（#27-b）。値だけに落とす前に処理すること。
        // ここで `vals` を先に作ると `k=v` の名前が失われ、他メソッドの arity 検査の
        // 見え方も変わってしまう（従来は名前つきの値も `vals` に入っていた）。
        if method_name == "format" {
            let mut pos_args: Vec<Value> = Vec::new();
            let mut kw_args: Vec<(String, Value)> = Vec::new();
            for (kw, v, _) in evaled {
                match kw {
                    Some(k) => kw_args.push((k, v)),
                    None => pos_args.push(v),
                }
            }
            // 書式は `format()` / f-string と同じ `format_value`（タスク 3-4）。
            let result = self.py_str_format(&s, &pos_args, &kw_args)?;
            return Ok(Value::str(result));
        }
        // `encode(encoding="utf-8", errors="strict")` は bytes を返す（python_builtins_plan.md のタスク 3-1・`ops::codecs`）。
        // ⚠ 以前は UTF-8 のバイトを int の list で返していた（`"Hi".encode()` が `[72, 105]`。CPython は `b'Hi'`）。
        if method_name == "encode" {
            let (mut encoding, mut errors) = (None, None);
            for (i, (kw, v, _)) in evaled.into_iter().enumerate() {
                match (kw.as_deref(), i) {
                    (Some("encoding"), _) | (None, 0) => encoding = Some(v),
                    (Some("errors"), _) | (None, 1) => errors = Some(v),
                    (Some(other), _) => {
                        return Err(format!("TypeError: encode() got an unexpected keyword argument '{other}'"))
                    }
                    (None, n) => return Err(format!("TypeError: encode() takes at most 2 arguments ({} given)", n + 1)),
                }
            }
            let encoding = self.str_arg(encoding.as_ref(), "utf-8", "encode", "encoding")?;
            let errors = self.str_arg(errors.as_ref(), "strict", "encode", "errors")?;
            let codec = crate::interpreter::ops::codecs::Codec::lookup(&encoding)?;
            return Ok(Value::Bytes(crate::interpreter::ops::codecs::encode(&s, codec, &errors)?.into()));
        }
        let vals: Vec<Value> = evaled.into_iter().map(|(_, v, _)| v).collect();

        // Helper: extract str from first positional arg
        macro_rules! arg_str {
            ($idx:expr, $name:literal) => {
                match vals.get($idx) {
                    Some(Value::Str(s)) => s.to_string(),
                    Some(other) => {
                        return Err(format!(
                            "TypeError: {}.{}() argument {} must be str, not '{}'",
                            method_name,
                            $name,
                            $idx + 1,
                            self.type_name(other)
                        ))
                    }
                    None => {
                        return Err(format!(
                            "TypeError: {}() missing argument '{}'",
                            method_name, $name
                        ))
                    }
                }
            };
        }
        // ⚠⚠ `ljust` / `rjust` / `center` の**幅と埋め文字**（#65）。
        // 3 アームに**逐語で同じ 10 行**が書かれており、その結果 `ljust` だけが
        // 別実装になって**実バグ**を持っていた（元の文字列内の空白まで埋め文字に置換していた）。
        // ⇒ 解析も詰め方も 1 箇所に畳んで、3 つが同じ規則で動くことを構造で保証する。
        macro_rules! pad_args {
            () => {{
                let width = arg_int!(0, 0).max(0) as usize;
                let fill = match vals.get(1) {
                    Some(Value::Str(f)) if f.chars().count() == 1 => f.chars().next().unwrap(),
                    None => ' ',
                    _ => {
                        return Err(format!(
                            "TypeError: {}() fillchar must be single char str",
                            method_name
                        ))
                    }
                };
                (width, fill)
            }};
        }
        macro_rules! arg_opt_str {
            ($idx:expr) => {
                match vals.get($idx) {
                    Some(Value::Str(s)) => Some(s.to_string()),
                    Some(Value::None) | None => None,
                    Some(other) => {
                        return Err(format!(
                            "TypeError: {}() argument must be str or None, not '{}'",
                            method_name,
                            self.type_name(other)
                        ))
                    }
                }
            };
        }
        macro_rules! arg_int {
            ($idx:expr, $default:expr) => {
                match vals.get($idx) {
                    Some(Value::Int(n)) => *n,
                    None => $default,
                    Some(other) => {
                        return Err(format!(
                            "TypeError: {}() argument must be int, not '{}'",
                            method_name,
                            self.type_name(other)
                        ))
                    }
                }
            };
        }

        match method_name {
            "__iter__" => {
                if !vals.is_empty() {
                    return Err("TypeError: str.__iter__() takes no arguments".to_string());
                }
                let chars: Vec<Value> = s.chars().map(|c| Value::str(c.to_string())).collect();
                Ok(Value::Generator(Rc::new(RefCell::new(GeneratorState::materialized(chars)))))
            }

            // ── 大文字・小文字変換 ──────────────────────────────────────────
            // ── 文字コード（3-1）────────────────────────────────────────────
            // `s.encode(encoding="utf-8")` → bytes。変換できない文字は ValueError（黙って置き換えない）。
            "upper" => Ok(Value::str(s.to_uppercase())),
            "lower" => Ok(Value::str(s.to_lowercase())),
            "swapcase" => Ok(Value::str(
                s.chars()
                    .map(|c| {
                        if c.is_uppercase() {
                            c.to_lowercase().next().unwrap_or(c)
                        } else {
                            c.to_uppercase().next().unwrap_or(c)
                        }
                    })
                    .collect::<String>(),
            )),
            "capitalize" => Ok(Value::str({
                let mut cs = s.chars();
                match cs.next() {
                    None => String::new(),
                    Some(c) => c.to_uppercase().collect::<String>() + &cs.as_str().to_lowercase(),
                }
            })),
            "title" => Ok(Value::str({
                let mut result = String::new();
                let mut capitalize_next = true;
                for c in s.chars() {
                    if c.is_whitespace() || !c.is_alphanumeric() {
                        capitalize_next = true;
                        result.push(c);
                    } else if capitalize_next {
                        result.extend(c.to_uppercase());
                        capitalize_next = false;
                    } else {
                        result.extend(c.to_lowercase());
                    }
                }
                result
            })),

            // ── 空白除去 ────────────────────────────────────────────────────
            "strip" => {
                let chars_arg = arg_opt_str!(0);
                Ok(Value::str(match chars_arg {
                    None => s.trim().to_string(),
                    Some(ref ch) => s.trim_matches(|c: char| ch.contains(c)).to_string(),
                }))
            }
            "lstrip" => {
                let chars_arg = arg_opt_str!(0);
                Ok(Value::str(match chars_arg {
                    None => s.trim_start().to_string(),
                    Some(ref ch) => s.trim_start_matches(|c: char| ch.contains(c)).to_string(),
                }))
            }
            "rstrip" => {
                let chars_arg = arg_opt_str!(0);
                Ok(Value::str(match chars_arg {
                    None => s.trim_end().to_string(),
                    Some(ref ch) => s.trim_end_matches(|c: char| ch.contains(c)).to_string(),
                }))
            }

            // ── 分割 ────────────────────────────────────────────────────────
            //
            // ⚠⚠ `split` と `rsplit` は 9 割同じだが**畳んでいない**（#65 で判断）。
            // 違うのは 3 点で、**どれも CPython に合わせるために必要**（実際に突き合わせて確認した）:
            //   ① `splitn` ↔ `rsplitn`（＋ `rsplit` は結果を `reverse()` する）
            //   ② 空白区切り＋maxsplit のとき `split` だけ `retain(非空)` する
            //   ③ 区切り指定＋maxsplit のとき `rsplit` だけ `reverse()` する
            // ⇒ 「reverse するか」1 つのフラグでは表せず、**畳むと非対称の理由が読めなくなる**。
            // 検査: `"a  b  c".split(" ",1)` → `['a', ' b  c']` ／ `.rsplit(" ",1)` → `['a  b ', 'c']`
            //       `"x,,y".split(",",1)` → `['x', ',y']` ／ `.rsplit(",",1)` → `['x,', 'y']`（CPython 一致）
            "split" => {
                let sep = arg_opt_str!(0);
                let maxsplit = arg_int!(1, -1);
                let parts: Vec<Value> = match sep {
                    None => {
                        if maxsplit < 0 {
                            s.split_whitespace()
                                .map(|p| Value::str(p.to_string()))
                                .collect()
                        } else {
                            let mut result: Vec<&str> = s
                                .splitn(maxsplit as usize + 1, |c: char| c.is_whitespace())
                                .collect();
                            result.retain(|p| !p.is_empty());
                            result.iter().map(|p| Value::str(p.to_string())).collect()
                        }
                    }
                    Some(ref sep) => {
                        if maxsplit < 0 {
                            s.split(sep.as_str())
                                .map(|p| Value::str(p.to_string()))
                                .collect()
                        } else {
                            s.splitn(maxsplit as usize + 1, sep.as_str())
                                .map(|p| Value::str(p.to_string()))
                                .collect()
                        }
                    }
                };
                Ok(Value::List(Rc::new(RefCell::new(parts))))
            }
            "rsplit" => {
                let sep = arg_opt_str!(0);
                let maxsplit = arg_int!(1, -1);
                let parts: Vec<Value> = match sep {
                    None => {
                        if maxsplit < 0 {
                            s.split_whitespace()
                                .map(|p| Value::str(p.to_string()))
                                .collect()
                        } else {
                            let mut result: Vec<&str> = s
                                .rsplitn(maxsplit as usize + 1, |c: char| c.is_whitespace())
                                .collect();
                            result.reverse();
                            result.iter().map(|p| Value::str(p.to_string())).collect()
                        }
                    }
                    Some(ref sep) => {
                        if maxsplit < 0 {
                            s.split(sep.as_str())
                                .map(|p| Value::str(p.to_string()))
                                .collect()
                        } else {
                            let mut v: Vec<&str> =
                                s.rsplitn(maxsplit as usize + 1, sep.as_str()).collect();
                            v.reverse();
                            v.iter().map(|p| Value::str(p.to_string())).collect()
                        }
                    }
                };
                Ok(Value::List(Rc::new(RefCell::new(parts))))
            }
            "splitlines" => {
                let parts: Vec<Value> = s.lines().map(|p| Value::str(p.to_string())).collect();
                Ok(Value::List(Rc::new(RefCell::new(parts))))
            }

            // ── 結合 ────────────────────────────────────────────────────────
            "join" => {
                let iterable = vals
                    .first()
                    .ok_or_else(|| "TypeError: join() missing argument 'iterable'".to_string())?;
                let items = match iterable {
                    Value::List(lst) => lst.borrow().clone(),
                    Value::Tuple(t) => t.all_values().to_vec(),
                    Value::Generator(g) => g.borrow().values[g.borrow().index..].to_vec(),
                    other => {
                        return Err(format!(
                            "TypeError: join() argument must be iterable, not '{}'",
                            self.type_name(other)
                        ))
                    }
                };
                let parts: Vec<String> = items.iter().map(|v| self.display(v)).collect();
                Ok(Value::str(parts.join(&s)))
            }

            // ── 検索 ────────────────────────────────────────────────────────
            // 位置と範囲（`start` / `end`）は**文字の番号**（添字・切り出し・`len` と同じ単位・CPython と同じ。
            // 負は後ろから・範囲外は詰める・`None` は省略と同じ・python_builtins_plan.md のタスク 1-17・ffi_value_exchange_plan.md 7-2）。
            // ⚠ 以前はバイトの番号で、`"γάμμα".find("μ")` が 4（CPython は 2）、文字の途中の `start` を渡すと
            //   Rust のパニックで**プロセスごと落ちていた**。負の `start` / `end` も 0 に詰めていた。
            "find" | "rfind" | "index" | "rindex" => {
                let sub = arg_str!(0, "sub");
                let start = str_index_arg(vals.get(1))?;
                let end = str_index_arg(vals.get(2))?;
                let found = str_char_range(&s, start, end).and_then(|(bs, be, first)| {
                    let slice = &s[bs..be];
                    let hit = if method_name.starts_with('r') { slice.rfind(sub.as_str()) } else { slice.find(sub.as_str()) };
                    hit.map(|i| first + slice[..i].chars().count())
                });
                match (found, method_name) {
                    (Some(i), _) => Ok(Value::Int(i as i64)),
                    (None, "find" | "rfind") => Ok(Value::Int(-1)),
                    (None, _) => Err("ValueError: substring not found".to_string()),
                }
            }
            "count" => {
                let sub = arg_str!(0, "sub");
                let start = str_index_arg(vals.get(1))?;
                let end = str_index_arg(vals.get(2))?;
                let n = match str_char_range(&s, start, end) {
                    None => 0,
                    Some((bs, be, _)) if sub.is_empty() => s[bs..be].chars().count() + 1,
                    Some((bs, be, _)) => s[bs..be].matches(sub.as_str()).count(),
                };
                Ok(Value::Int(n as i64))
            }
            "contains" => {
                let sub = arg_str!(0, "sub");
                Ok(Value::Bool(s.contains(sub.as_str())))
            }
            // `startswith(prefix, start, end)` / `endswith(..)`。`prefix` は文字列か文字列の組（どれかに当たれば真）。
            // ⚠ 以前は `start` / `end` を黙って捨て、組を受けなかった（タスク 1-17）。
            "startswith" | "endswith" => {
                let candidates: Vec<String> = match vals.first() {
                    Some(Value::Str(p)) => vec![p.to_string()],
                    Some(Value::Tuple(t)) => t
                        .all_values()
                        .iter()
                        .map(|v| match v {
                            Value::Str(p) => Ok(p.to_string()),
                            other => Err(format!(
                                "TypeError: tuple for {method_name} must only contain str, not {}",
                                self.type_name(other)
                            )),
                        })
                        .collect::<Result<_, _>>()?,
                    Some(other) => {
                        return Err(format!(
                            "TypeError: {method_name} first arg must be str or a tuple of str, not {}",
                            self.type_name(other)
                        ))
                    }
                    None => return Err(format!("TypeError: {method_name}() takes at least 1 argument (0 given)")),
                };
                let start = str_index_arg(vals.get(1))?;
                let end = str_index_arg(vals.get(2))?;
                Ok(Value::Bool(match str_char_range(&s, start, end) {
                    None => false,
                    Some((bs, be, _)) => {
                        let slice = &s[bs..be];
                        candidates.iter().any(|c| {
                            if method_name == "startswith" { slice.starts_with(c.as_str()) } else { slice.ends_with(c.as_str()) }
                        })
                    }
                }))
            }

            // ── 置換 ────────────────────────────────────────────────────────
            "replace" => {
                let old = arg_str!(0, "old");
                let new = arg_str!(1, "new");
                let count = arg_int!(2, -1);
                Ok(Value::str(if count < 0 {
                    s.replace(old.as_str(), new.as_str())
                } else {
                    s.replacen(old.as_str(), new.as_str(), count as usize)
                }))
            }
            "removeprefix" => {
                let prefix = arg_str!(0, "prefix");
                Ok(Value::str(
                    s.strip_prefix(prefix.as_str()).unwrap_or(&s).to_string(),
                ))
            }
            "removesuffix" => {
                let suffix = arg_str!(0, "suffix");
                Ok(Value::str(
                    s.strip_suffix(suffix.as_str()).unwrap_or(&s).to_string(),
                ))
            }


            // ── 文字判定 ────────────────────────────────────────────────────
            "isdigit" => Ok(Value::Bool(
                !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()),
            )),
            "isnumeric" => Ok(Value::Bool(
                !s.is_empty() && s.chars().all(|c| c.is_numeric()),
            )),
            "isalpha" => Ok(Value::Bool(
                !s.is_empty() && s.chars().all(|c| c.is_alphabetic()),
            )),
            "isalnum" => Ok(Value::Bool(
                !s.is_empty() && s.chars().all(|c| c.is_alphanumeric()),
            )),
            "isspace" => Ok(Value::Bool(
                !s.is_empty() && s.chars().all(|c| c.is_whitespace()),
            )),
            "isupper" => Ok(Value::Bool(
                !s.is_empty()
                    && s.chars().any(|c| c.is_alphabetic())
                    && s.chars().all(|c| !c.is_alphabetic() || c.is_uppercase()),
            )),
            "islower" => Ok(Value::Bool(
                !s.is_empty()
                    && s.chars().any(|c| c.is_alphabetic())
                    && s.chars().all(|c| !c.is_alphabetic() || c.is_lowercase()),
            )),
            "isascii" => Ok(Value::Bool(s.is_ascii())),
            // CPython と同じ表（`repr` が `\x..` に変えるかと同じ判定・タスク 1-13）。以前は `!is_control()` で、
            // 書式文字（U+200B）・区切り（U+00A0 / U+2028）・未割り当ての符号位置を「表示できる」としていた。
            "isprintable" => Ok(Value::Bool(
                s.chars().all(crate::interpreter::ops::unicode_printable::is_printable),
            )),

            // ── 幅揃え・ゼロ埋め ────────────────────────────────────────────
            // 幅は文字の数。符号（`+` / `-`）は先頭に残して後ろへ 0 を詰める（CPython と同じ・`"-42".zfill(5)` は `-0042`）。
            // ⚠ 以前はバイトの数で比べ（非 ASCII は詰めなかった）、符号の前に 0 を置いていた（タスク 1-17）。
            "zfill" => {
                let width = arg_int!(0, 0).max(0) as usize;
                let n = s.chars().count();
                // 幅を満たしていればレシーバの Rc を使い回す（確保なし）
                if n >= width {
                    return Ok(Value::Str(s.clone()));
                }
                let zeros = "0".repeat(width - n);
                Ok(Value::str(match s.strip_prefix(['+', '-']) {
                    Some(rest) => format!("{}{zeros}{rest}", &s[..1]),
                    None => format!("{zeros}{s}"),
                }))
            }
            // ⚠⚠ #65 で**実バグを修正**した。以前は「空白で幅詰めしてから空白を埋め文字へ
            // `replace`」していたので、**元の文字列に含まれる空白まで置換**していた
            // （`"a b".ljust(6, "*")` が CPython の `a b***` に対し `a*b***`）。
            // `rjust` / `center` は最初から `repeat(pad)` で正しく、**ljust だけがずれていた**
            // ＝ 同じ処理を 3 回書いた結果。⇒ 3 つとも同じ形に揃えてある。
            "ljust" => {
                let (width, fill) = pad_args!();
                if s.chars().count() >= width {
                    return Ok(Value::str(s.to_string()));
                }
                let pad = width - s.chars().count();
                Ok(Value::str(format!("{}{}", s, fill.to_string().repeat(pad))))
            }
            "rjust" => {
                let (width, fill) = pad_args!();
                if s.chars().count() >= width {
                    return Ok(Value::str(s.to_string()));
                }
                let pad = width - s.chars().count();
                Ok(Value::str(format!("{}{}", fill.to_string().repeat(pad), s)))
            }
            "center" => {
                let (width, fill) = pad_args!();
                if s.chars().count() >= width {
                    return Ok(Value::str(s.to_string()));
                }
                let pad = width - s.chars().count();
                let left = pad / 2;
                let right = pad - left;
                Ok(Value::str(format!(
                    "{}{}{}",
                    fill.to_string().repeat(left),
                    s,
                    fill.to_string().repeat(right)
                )))
            }

            // ── 分割（区切りを含む） ─────────────────────────────────────────
            "partition" => {
                let sep = arg_str!(0, "sep");
                let (a, b, c) = match s.find(sep.as_str()) {
                    Some(i) => (&s[..i], sep.as_str(), &s[i + sep.len()..]),
                    None => (&*s, "", ""),
                };
                Ok(Value::Tuple(Rc::new(crate::interpreter::TupleData::new(
                    vec![
                        Value::str(a.to_string()),
                        Value::str(b.to_string()),
                        Value::str(c.to_string()),
                    ],
                    vec!["str".to_string(), "str".to_string(), "str".to_string()],
                ))))
            }
            "rpartition" => {
                let sep = arg_str!(0, "sep");
                let (a, b, c) = match s.rfind(sep.as_str()) {
                    Some(i) => (&s[..i], sep.as_str(), &s[i + sep.len()..]),
                    None => ("", "", &*s),
                };
                Ok(Value::Tuple(Rc::new(crate::interpreter::TupleData::new(
                    vec![
                        Value::str(a.to_string()),
                        Value::str(b.to_string()),
                        Value::str(c.to_string()),
                    ],
                    vec!["str".to_string(), "str".to_string(), "str".to_string()],
                ))))
            }

            // ── その他変換 ───────────────────────────────────────────────────
            // タブを次のタブ位置まで空白で埋める（桁は文字の数・改行で 0 に戻る・`tabsize` が 0 以下ならタブを消す・CPython と同じ）。
            // ⚠ 以前はタブを `tabsize` 個の空白に置き換えるだけだった（`"ab\tc".expandtabs(4)` が `ab    c`。CPython は `ab  c`・タスク 1-17）。
            "expandtabs" => {
                let tabsize = arg_int!(0, 8).max(0) as usize;
                let mut out = String::with_capacity(s.len());
                let mut col = 0usize;
                for c in s.chars() {
                    match c {
                        '\t' => {
                            if tabsize > 0 {
                                let n = tabsize - col % tabsize;
                                out.extend(std::iter::repeat(' ').take(n));
                                col += n;
                            }
                        }
                        '\n' | '\r' => {
                            out.push(c);
                            col = 0;
                        }
                        _ => {
                            out.push(c);
                            col += 1;
                        }
                    }
                }
                Ok(Value::str(out))
            }
            "chars" => {
                // 文字リストを返す
                let chars: Vec<Value> = s.chars().map(|c| Value::str(c.to_string())).collect();
                Ok(Value::List(Rc::new(RefCell::new(chars))))
            }
            "ord" => {
                // 1 文字の文字列を ord 値 (int) に変換
                let mut cs = s.chars();
                match (cs.next(), cs.next()) {
                    (Some(c), None) => Ok(Value::Int(c as i64)),
                    _ => Err(
                        "TypeError: ord() expected a character, but found a string of length != 1"
                            .to_string(),
                    ),
                }
            }

            // ── 正規表現メソッド ─────────────────────────────────────────────
            "match" => {
                let pattern = arg_str!(0, "pattern");
                let flags = match vals.get(1) {
                    Some(Value::Str(f)) => f.to_string(),
                    None => String::new(),
                    Some(other) => {
                        return Err(format!(
                            "TypeError: match() flags must be str, not '{}'",
                            self.type_name(other)
                        ))
                    }
                };
                match regex_match(&s, &pattern, &flags)? {
                    Some(m) => Ok(Value::str(m)),
                    None => Ok(Value::None),
                }
            }
            "search" => {
                let pattern = arg_str!(0, "pattern");
                let flags = match vals.get(1) {
                    Some(Value::Str(f)) => f.to_string(),
                    None => String::new(),
                    Some(other) => {
                        return Err(format!(
                            "TypeError: search() flags must be str, not '{}'",
                            self.type_name(other)
                        ))
                    }
                };
                match regex_search(&s, &pattern, &flags)? {
                    Some(m) => Ok(Value::str(m)),
                    None => Ok(Value::None),
                }
            }
            "findall" => {
                let pattern = arg_str!(0, "pattern");
                let flags = match vals.get(1) {
                    Some(Value::Str(f)) => f.to_string(),
                    None => String::new(),
                    Some(other) => {
                        return Err(format!(
                            "TypeError: findall() flags must be str, not '{}'",
                            self.type_name(other)
                        ))
                    }
                };
                let matches = regex_findall(&s, &pattern, &flags)?;
                Ok(Value::List(Rc::new(RefCell::new(
                    matches.into_iter().map(Value::str).collect(),
                ))))
            }
            "sub" => {
                let pattern = arg_str!(0, "pattern");
                let repl = arg_str!(1, "repl");
                let count = arg_int!(2, 0).max(0) as usize;
                let flags = match vals.get(3) {
                    Some(Value::Str(f)) => f.to_string(),
                    None => String::new(),
                    Some(other) => {
                        return Err(format!(
                            "TypeError: sub() flags must be str, not '{}'",
                            self.type_name(other)
                        ))
                    }
                };
                Ok(Value::str(regex_sub(&s, &pattern, &repl, count, &flags)?))
            }
            "regex_split" => {
                let pattern = arg_str!(0, "pattern");
                let maxsplit = arg_int!(1, 0).max(0) as usize;
                let flags = match vals.get(2) {
                    Some(Value::Str(f)) => f.to_string(),
                    None => String::new(),
                    Some(other) => {
                        return Err(format!(
                            "TypeError: regex_split() flags must be str, not '{}'",
                            self.type_name(other)
                        ))
                    }
                };
                let parts = regex_split(&s, &pattern, maxsplit, &flags)?;
                Ok(Value::List(Rc::new(RefCell::new(
                    parts.into_iter().map(Value::str).collect(),
                ))))
            }

            _ => Err(format!(
                "AttributeError: 'str' object has no method '{method_name}'"
            )),
        }
    }

}


/// `find` / `count` / `startswith` などの `start` / `end`（整数か `None`・省略は `None`）。
fn str_index_arg(v: Option<&Value>) -> Result<Option<i64>, String> {
    match v {
        None | Some(Value::None) => Ok(None),
        Some(Value::Int(n)) => Ok(Some(*n)),
        Some(Value::Bool(b)) => Ok(Some(i64::from(*b))),
        Some(other) => Err(format!(
            "TypeError: slice indices must be integers or None or have an __index__ method, not '{}'",
            crate::interpreter::ops::typecheck::runtime_type_name(other)
        )),
    }
}

/// `s[start:end]` の範囲を、**文字の番号**で解釈してバイトの範囲にする（CPython の `ADJUST_INDICES`・タスク 1-17）。
/// 負は後ろから数え、長さを超えた `end` は長さに詰める。返すのは（開始のバイト, 終わりのバイト, 開始の文字の番号）。
/// 開始が長さを超える・終わりが開始より前なら `None`（`find` は -1、`count` は 0、`startswith` は偽・CPython と同じ）。
fn str_char_range(s: &str, start: Option<i64>, end: Option<i64>) -> Option<(usize, usize, usize)> {
    let n = s.chars().count() as i64;
    let adjust = |v: i64| if v < 0 { (v + n).max(0) } else { v };
    let st = start.map_or(0, adjust);
    let en = end.map_or(n, |e| adjust(e).min(n));
    if st > n || en < st {
        return None;
    }
    let byte_of = |ci: i64| s.char_indices().nth(ci as usize).map_or(s.len(), |(b, _)| b);
    Some((byte_of(st), byte_of(en), st as usize))
}
