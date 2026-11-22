// diag_out.rs — 機械向けの診断の出力（`AR_DIAG_OUT=<ファイル>`・gate_speed_plan.md 3-1）。
//
// ゲートが `gate/` の禁止の形のテストを「期待した場所で、期待した段階・種類の誤りが出たか」で照らすために、
// 誤りを 1 行に 1 件の JSON で**別のファイルへ**書く。stdout / stderr は変えない（今のゲート・A/B の出力が変わらない）。
//
// ⚠ 今の stderr では照らせない理由（gate_speed_plan.md 0.3）: 静的な誤りは表で、種類の欄が常に `StaticTypeError`
//   （`TypeErrorKind` が出ない）・文面が端末の幅で折り返す・ファイルに落としても色の符号が付く。実行時の誤りは
//   位置が無いものがある。
//
// 1 行の形（欄が無いものは `null`。⚠ 位置が無いときに 0 や空文字列で埋めない）:
//   {"stage":"static","kind":"VarTypeMismatch","file":"a.ar","line":3,"col":5,"message":"…","frames":[]}
//   - stage: "parse" / "expand" / "static" / "warning" / "runtime"
//   - kind : static / warning は `TypeErrorKind` / `TypeWarningKind` の名前、runtime は例外のクラスの名前、
//            expand は `MetaError` などの接頭辞。parse は種類を持たないので null。
//   - frames: runtime だけ。外側から内側の順（トレースバックの表示と同じ）。各 {"file","line","col","fn"}。
//
// ⚠ 環境変数が立っていれば、起動時にファイルを**空で作る**（`init`）。誤りが無ければ空のまま。ファイルが無い・
//   exit が 0 でないのに 1 件も無い、は「arrow.exe が診断を書く前に止まった」と判定側が区別できる。
// ⚠ 書けなければ止める（黙って書かずに続けると、判定側は「誤りが無かった」と読む）。

use std::io::Write;
use std::sync::{Mutex, OnceLock};

use serde_json::{json, Value as Json};

static OUT: OnceLock<Option<Mutex<std::fs::File>>> = OnceLock::new();

/// `AR_DIAG_OUT` が立っていれば、そのファイルを空で作る（起動時に 1 回呼ぶ）。
pub fn init() {
    OUT.get_or_init(|| {
        let path = std::env::var_os("AR_DIAG_OUT").filter(|p| !p.is_empty())?;
        match std::fs::File::create(&path) {
            Ok(f) => Some(Mutex::new(f)),
            Err(e) => {
                eprintln!("AR_DIAG_OUT: cannot create '{}': {e}", std::path::Path::new(&path).display());
                std::process::exit(2);
            }
        }
    });
}

fn enabled() -> bool {
    matches!(OUT.get(), Some(Some(_)))
}

fn write_line(v: Json) {
    let Some(Some(m)) = OUT.get() else { return };
    let mut f = m.lock().unwrap_or_else(|p| p.into_inner());
    if let Err(e) = writeln!(f, "{v}").and_then(|()| f.flush()) {
        eprintln!("AR_DIAG_OUT: cannot write: {e}");
        std::process::exit(2);
    }
}

/// 色の符号（`ESC [ … m`）を取り除く。`detail_str` などは端末向けに色を付けている。
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            chars.next();
            // 終わりの文字（英字）まで読み飛ばす
            for d in chars.by_ref() {
                if d.is_ascii_alphabetic() {
                    break;
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// 文面の末尾の ` at <file>:<line>:<col>` を分ける（構文の誤り・展開の誤りの形・`Parser::error_location`）。
/// 位置が無ければ `(文面, None)`。
pub fn split_location(text: &str) -> (String, Option<(String, usize, usize)>) {
    if let Some((head, tail)) = text.rsplit_once(" at ") {
        let mut parts = tail.rsplitn(3, ':');
        let col = parts.next().and_then(|s| s.trim().parse::<usize>().ok());
        let line = parts.next().and_then(|s| s.parse::<usize>().ok());
        let file = parts.next().filter(|f| !f.is_empty());
        if let (Some(col), Some(line), Some(file)) = (col, line, file) {
            return (head.to_string(), Some((file.to_string(), line, col)));
        }
    }
    (text.to_string(), None)
}

fn loc_json(loc: &Option<(String, usize, usize)>) -> (Json, Json, Json) {
    match loc {
        Some((f, l, c)) => (json!(f), json!(l), json!(c)),
        None => (Json::Null, Json::Null, Json::Null),
    }
}

/// 構文の誤り（`ParseError: ` を除いた文面。位置は末尾の ` at …`）。
pub fn parse_error(full: &str) {
    if !enabled() {
        return;
    }
    let msg = full.strip_prefix("ParseError: ").unwrap_or(full);
    let (msg, loc) = split_location(msg);
    let (file, line, col) = loc_json(&loc);
    write_line(json!({"stage": "parse", "kind": Json::Null, "file": file, "line": line, "col": col,
        "message": strip_ansi(&msg), "frames": []}));
}

/// 展開の誤り（`MetaError: …` など。種類は先頭の名前、位置は 1 行目の末尾の ` at …`）。
pub fn expand_error(full: &str) {
    if !enabled() {
        return;
    }
    let plain = strip_ansi(full);
    let first = plain.lines().next().unwrap_or("");
    let kind = leading_name(first);
    let (_, loc) = split_location(first);
    let (file, line, col) = loc_json(&loc);
    write_line(json!({"stage": "expand", "kind": kind, "file": file, "line": line, "col": col,
        "message": plain, "frames": []}));
}

/// 静的な誤り・警告。
pub fn static_diag(stage: &str, kind: &str, span: Option<&crate::token::Span>, message: &str) {
    if !enabled() {
        return;
    }
    let loc = span.filter(|s| s.line != 0).map(|s| (s.file.to_string(), s.line, s.col));
    let (file, line, col) = loc_json(&loc);
    write_line(json!({"stage": stage, "kind": kind, "file": file, "line": line, "col": col,
        "message": strip_ansi(message), "frames": []}));
}

/// 実行時の誤り（例外を持っているとき）。
pub fn runtime_raised(raised: &crate::interpreter::RaisedError) {
    if !enabled() {
        return;
    }
    let line_text = strip_ansi(&crate::interpreter::Interpreter::exception_line(raised));
    let kind = leading_name(&line_text);
    // frames[0] が最も内側。表示と同じく外側から並べる。位置の無いフレーム（line 0）は null。
    let frames: Vec<Json> = raised
        .frames
        .iter()
        .rev()
        .map(|f| {
            if f.line == 0 {
                json!({"file": nonempty(&f.file), "line": Json::Null, "col": Json::Null, "fn": f.fn_name})
            } else {
                json!({"file": nonempty(&f.file), "line": f.line, "col": f.col, "fn": f.fn_name})
            }
        })
        .collect();
    // 誤りの位置はいちばん内側の位置のあるフレーム
    let inner = raised.frames.iter().find(|f| f.line != 0 && !f.file.is_empty());
    let (file, line, col) = match inner {
        Some(f) => (json!(f.file), json!(f.line), json!(f.col)),
        None => (Json::Null, Json::Null, Json::Null),
    };
    write_line(json!({"stage": "runtime", "kind": kind, "file": file, "line": line, "col": col,
        "message": message_after_name(&line_text), "frames": frames}));
}

/// 実行時の誤り（文字列だけのとき。`<file>:<line>:<col>: Name: msg` か `Name: msg`、またはトレースバックの文字列）。
pub fn runtime_text(report: &str) {
    if !enabled() {
        return;
    }
    let plain = strip_ansi(report);
    let mut frames: Vec<Json> = Vec::new();
    let mut last = "";
    for l in plain.lines() {
        if let Some(fr) = parse_frame_line(l) {
            frames.push(fr);
        } else if !l.trim().is_empty() && !l.starts_with("    ") && l != "Traceback (most recent call last):" {
            last = l;
        }
    }
    // 先頭の `<file>:<line>:<col>: ` を分ける
    let (loc, rest) = split_leading_location(last);
    let loc = loc.or_else(|| {
        frames.iter().rev().find_map(|f| {
            Some((f.get("file")?.as_str()?.to_string(), f.get("line")?.as_u64()? as usize, f.get("col")?.as_u64()? as usize))
        })
    });
    let (file, line, col) = loc_json(&loc);
    write_line(json!({"stage": "runtime", "kind": leading_name(rest), "file": file, "line": line, "col": col,
        "message": message_after_name(rest), "frames": frames}));
}

fn nonempty(s: &str) -> Json {
    if s.is_empty() { Json::Null } else { json!(s) }
}

/// `Name: msg` / `pkg.Name: msg` / `Name` の名前の部分。名前の形でなければ null（黙って別の名前を作らない）。
fn leading_name(line: &str) -> Json {
    let head = line.split_once(':').map(|(h, _)| h).unwrap_or(line).trim();
    let ok = !head.is_empty()
        && head.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_')
        && head.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.');
    if ok { json!(head) } else { Json::Null }
}

fn message_after_name(line: &str) -> String {
    match line.split_once(": ") {
        Some((h, m)) if leading_name(h) != Json::Null => m.to_string(),
        _ => line.to_string(),
    }
}

/// `  File "<file>", line <n>, col <c>, in <fn>` / `  File "<file>", in <fn>`
fn parse_frame_line(l: &str) -> Option<Json> {
    let rest = l.strip_prefix("  File \"")?;
    let (file, rest) = rest.split_once('"')?;
    let rest = rest.strip_prefix(", ")?;
    if let Some(fn_name) = rest.strip_prefix("in ") {
        return Some(json!({"file": nonempty(file), "line": Json::Null, "col": Json::Null, "fn": fn_name}));
    }
    let rest = rest.strip_prefix("line ")?;
    let (line, rest) = rest.split_once(", col ")?;
    let (col, fn_name) = rest.split_once(", in ")?;
    Some(json!({"file": nonempty(file), "line": line.parse::<usize>().ok()?, "col": col.parse::<usize>().ok()?, "fn": fn_name}))
}

/// `<file>:<line>:<col>: rest` の位置を分ける（Windows のドライブ名 `D:\…` も含めて、右から数字 2 つを探す）。
fn split_leading_location(s: &str) -> (Option<(String, usize, usize)>, &str) {
    // `: ` で区切った最初の区切りより前が `<file>:<line>:<col>` の形か
    if let Some(idx) = s.find(": ") {
        let head = &s[..idx];
        let mut parts = head.rsplitn(3, ':');
        let col = parts.next().and_then(|x| x.parse::<usize>().ok());
        let line = parts.next().and_then(|x| x.parse::<usize>().ok());
        let file = parts.next().filter(|f| !f.is_empty());
        if let (Some(col), Some(line), Some(file)) = (col, line, file) {
            return (Some((file.to_string(), line, col)), &s[idx + 2..]);
        }
    }
    (None, s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_colour_codes() {
        assert_eq!(strip_ansi("'\x1b[1;35ma\x1b[0m' is"), "'a' is");
    }

    #[test]
    fn splits_trailing_location() {
        let (m, loc) = split_location("expected identifier, got `=` at D:\\x\\a.ar:2:5");
        assert_eq!(m, "expected identifier, got `=`");
        assert_eq!(loc, Some(("D:\\x\\a.ar".to_string(), 2, 5)));
        assert_eq!(split_location("no location here").1, None);
    }

    #[test]
    fn splits_leading_location_with_drive_letter() {
        let (loc, rest) = split_leading_location("D:\\x\\a.ar:16:5: TypeError: cannot freeze");
        assert_eq!(loc, Some(("D:\\x\\a.ar".to_string(), 16, 5)));
        assert_eq!(rest, "TypeError: cannot freeze");
        let (loc, rest) = split_leading_location("TypeError: x");
        assert_eq!(loc, None);
        assert_eq!(rest, "TypeError: x");
    }

    #[test]
    fn leading_name_rejects_non_names() {
        assert_eq!(leading_name("ValueError: bad"), json!("ValueError"));
        assert_eq!(leading_name("pkg.MyErr: bad"), json!("pkg.MyErr"));
        assert_eq!(leading_name("GateCheckFailed"), json!("GateCheckFailed"));
        assert_eq!(leading_name("cannot do this: because"), Json::Null);
    }

    #[test]
    fn parses_frame_lines() {
        let f = parse_frame_line("  File \"a.ar\", line 3, col 9, in check_eq").unwrap();
        assert_eq!(f["line"], json!(3));
        assert_eq!(f["fn"], json!("check_eq"));
        let g = parse_frame_line("  File \"\", in <module>").unwrap();
        assert_eq!(g["file"], Json::Null);
        assert_eq!(g["line"], Json::Null);
    }
}
